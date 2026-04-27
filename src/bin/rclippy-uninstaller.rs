#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("rclippy-uninstaller is only supported on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_uninstaller::run() {
        eprintln!("Uninstall failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_uninstaller {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    use anyhow::{Context, Result};
    use base64::{Engine as _, engine::general_purpose};
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const EXE_NAME: &str = "rclippy.exe";
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\rclippy";

    pub fn run() -> Result<()> {
        let current_exe = env::current_exe().context("resolve uninstaller path")?;
        let install_dir = current_exe
            .parent()
            .context("resolve install directory")?
            .to_path_buf();
        let scope = InstallScope::from_install_dir(&install_dir);

        stop_running_app();
        remove_shortcut(&scope);
        remove_uninstall_entry(&scope);

        spawn_cleanup(&install_dir)?;

        Ok(())
    }

    fn spawn_cleanup(install_dir: &Path) -> Result<()> {
        let script = format!(
            "Start-Sleep -Milliseconds 750; Remove-Item -LiteralPath {} -Recurse -Force",
            powershell_string(install_dir)
        );
        let encoded = general_purpose::STANDARD.encode(utf16le(&script));
        hidden_command("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-EncodedCommand",
                &encoded,
            ])
            .spawn()
            .context("start cleanup")?;
        Ok(())
    }

    enum InstallScope {
        User,
        Machine,
    }

    impl InstallScope {
        fn from_install_dir(path: &Path) -> Self {
            let program_files = env::var_os("ProgramFiles").map(PathBuf::from);
            if program_files
                .as_ref()
                .is_some_and(|root| path.starts_with(root))
            {
                Self::Machine
            } else {
                Self::User
            }
        }

        fn reg_root(&self) -> &'static str {
            match self {
                Self::User => "HKCU",
                Self::Machine => "HKLM",
            }
        }

        fn shortcut_path(&self) -> Option<PathBuf> {
            let root = match self {
                Self::User => env::var_os("APPDATA"),
                Self::Machine => env::var_os("ProgramData"),
            }?;

            Some(
                PathBuf::from(root)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs")
                    .join("rclippy.lnk"),
            )
        }
    }

    fn stop_running_app() {
        let _ = hidden_command("taskkill")
            .args(["/IM", EXE_NAME, "/F"])
            .status();
    }

    fn remove_shortcut(scope: &InstallScope) {
        if let Some(path) = scope.shortcut_path() {
            let _ = fs::remove_file(path);
        }
    }

    fn remove_uninstall_entry(scope: &InstallScope) {
        let key = format!(r"{}\{}", scope.reg_root(), UNINSTALL_KEY);
        let _ = hidden_command("reg.exe")
            .args(["delete", &key, "/f"])
            .status();
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
