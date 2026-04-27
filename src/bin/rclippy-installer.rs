#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("rclippy-installer is only supported on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_installer::run() {
        eprintln!("Install failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_installer {
    use std::{
        env, fs, io,
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    use anyhow::{Context, Result, bail};
    use base64::{Engine as _, engine::general_purpose};
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::UI::Shell::IsUserAnAdmin;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const APP_NAME: &str = "rclippy";
    const EXE_NAME: &str = "rclippy.exe";
    const UNINSTALLER_NAME: &str = "rclippy-uninstaller.exe";
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\rclippy";
    const PAYLOAD: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-installer-payload.exe"));
    const UNINSTALLER: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/rclippy-uninstaller-payload.exe"));
    const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rclippy.ico"));

    pub fn run() -> Result<()> {
        if PAYLOAD.is_empty() {
            bail!("installer payload is missing");
        }
        if UNINSTALLER.is_empty() {
            bail!("uninstaller payload is missing");
        }

        let scope = InstallScope::detect()?;
        let install_dir = scope.install_dir()?;
        let exe_path = install_dir.join(EXE_NAME);
        let uninstaller_path = install_dir.join(UNINSTALLER_NAME);
        let icon_path = install_dir.join("rclippy.ico");
        let shortcut_path = scope.shortcut_path()?;

        fs::create_dir_all(&install_dir).context("create install directory")?;
        stop_running_app();
        fs::write(&exe_path, PAYLOAD).context("write app executable")?;
        fs::write(&uninstaller_path, UNINSTALLER).context("write uninstaller")?;
        fs::write(&icon_path, ICON).context("write app icon")?;
        create_shortcut(&shortcut_path, &exe_path, &icon_path)
            .context("create start menu shortcut")?;
        create_uninstall_entry(
            &scope,
            &install_dir,
            &exe_path,
            &uninstaller_path,
            &icon_path,
        )
        .context("create uninstall entry")?;
        Command::new(&exe_path)
            .spawn()
            .context("launch installed app")?;

        println!("Installed rclippy to {}", exe_path.display());
        Ok(())
    }

    enum InstallScope {
        User,
        Machine,
    }

    impl InstallScope {
        fn detect() -> Result<Self> {
            if unsafe { IsUserAnAdmin() } != 0 {
                Ok(Self::Machine)
            } else {
                Ok(Self::User)
            }
        }

        fn install_dir(&self) -> Result<PathBuf> {
            Ok(match self {
                Self::User => env_path("LOCALAPPDATA")?.join("Programs").join(APP_NAME),
                Self::Machine => env_path("ProgramFiles")?.join(APP_NAME),
            })
        }

        fn shortcut_path(&self) -> Result<PathBuf> {
            Ok(match self {
                Self::User => env_path("APPDATA")?,
                Self::Machine => env_path("ProgramData")?,
            }
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("rclippy.lnk"))
        }

        fn reg_root(&self) -> &'static str {
            match self {
                Self::User => "HKCU",
                Self::Machine => "HKLM",
            }
        }
    }

    fn env_path(name: &str) -> Result<PathBuf> {
        env::var_os(name)
            .map(PathBuf::from)
            .with_context(|| format!("{name} is not set"))
    }

    fn stop_running_app() {
        let _ = hidden_command("taskkill")
            .args(["/IM", EXE_NAME, "/F"])
            .status();
    }

    fn create_shortcut(shortcut_path: &Path, exe_path: &Path, icon_path: &Path) -> Result<()> {
        if let Some(parent) = shortcut_path.parent() {
            fs::create_dir_all(parent).context("create start menu directory")?;
        }

        let script = format!(
            "$shell = New-Object -ComObject WScript.Shell\n\
             $shortcut = $shell.CreateShortcut({shortcut})\n\
             $shortcut.TargetPath = {target}\n\
             $shortcut.WorkingDirectory = {working_dir}\n\
             $shortcut.IconLocation = {icon}\n\
             $shortcut.Save()\n",
            shortcut = powershell_string(shortcut_path),
            target = powershell_string(exe_path),
            working_dir = powershell_string(
                exe_path
                    .parent()
                    .ok_or_else(|| io::Error::other("missing install directory"))?,
            ),
            icon = powershell_string(format!("{},0", icon_path.display())),
        );

        let encoded = general_purpose::STANDARD.encode(utf16le(&script));
        let status = hidden_command("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &encoded,
            ])
            .status()
            .context("run powershell")?;

        if !status.success() {
            bail!("powershell shortcut creation failed");
        }

        Ok(())
    }

    fn create_uninstall_entry(
        scope: &InstallScope,
        install_dir: &Path,
        exe_path: &Path,
        uninstaller_path: &Path,
        icon_path: &Path,
    ) -> Result<()> {
        let key = format!(r"{}\{}", scope.reg_root(), UNINSTALL_KEY);
        reg_add_sz(&key, "DisplayName", "rclippy")?;
        reg_add_sz(&key, "DisplayVersion", env!("CARGO_PKG_VERSION"))?;
        reg_add_sz(&key, "Publisher", "Invades")?;
        reg_add_sz(&key, "InstallLocation", &install_dir.display().to_string())?;
        reg_add_sz(&key, "DisplayIcon", &format!("{},0", icon_path.display()))?;
        reg_add_sz(
            &key,
            "UninstallString",
            &format!("\"{}\"", uninstaller_path.display()),
        )?;
        reg_add_sz(
            &key,
            "QuietUninstallString",
            &format!("\"{}\"", uninstaller_path.display()),
        )?;
        reg_add_dword(&key, "NoModify", 1)?;
        reg_add_dword(&key, "NoRepair", 1)?;
        reg_add_dword(
            &key,
            "EstimatedSize",
            estimated_kib([exe_path, uninstaller_path, icon_path]),
        )?;
        Ok(())
    }

    fn reg_add_sz(key: &str, name: &str, value: &str) -> Result<()> {
        reg_add(key, name, "REG_SZ", value)
    }

    fn reg_add_dword(key: &str, name: &str, value: u32) -> Result<()> {
        reg_add(key, name, "REG_DWORD", &format!("{value}"))
    }

    fn reg_add(key: &str, name: &str, value_type: &str, value: &str) -> Result<()> {
        let status = hidden_command("reg.exe")
            .args(["add", key, "/v", name, "/t", value_type, "/d", value, "/f"])
            .status()
            .context("run reg.exe")?;
        if !status.success() {
            bail!("registry update failed for {name}");
        }
        Ok(())
    }

    fn estimated_kib(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> u32 {
        paths
            .into_iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum::<u64>()
            .div_ceil(1024)
            .min(u64::from(u32::MAX)) as u32
    }

    fn powershell_string(value: impl AsRef<Path>) -> String {
        format!(
            "'{}'",
            value.as_ref().display().to_string().replace('\'', "''")
        )
    }

    fn utf16le(value: &str) -> Vec<u8> {
        value.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn hidden_command(program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}
