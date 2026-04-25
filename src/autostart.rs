use anyhow::{Context, Result};
use auto_launch::{
    AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode,
};

use crate::APP_NAME;

fn build_auto_launch() -> Result<AutoLaunch> {
    let exe = std::env::current_exe().context("resolve current executable")?;
    let exe = exe
        .to_str()
        .context("current executable path is not valid UTF-8")?;
    let auto = AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(exe)
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
        .set_args(&["--minimized"])
        .build()?;
    Ok(auto)
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let auto = build_auto_launch()?;
    if enabled {
        auto.enable()?;
    } else {
        auto.disable()?;
    }
    Ok(())
}

pub fn is_enabled() -> Result<bool> {
    Ok(build_auto_launch()?.is_enabled()?)
}
