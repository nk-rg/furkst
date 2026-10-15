//! Integration with the desktop: file manager, terminal and default applications.

use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
const OPENER: &str = "explorer";
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const OPENER: &str = "xdg-open";

/// Opens `path` with the system's default handler (folders open in the file manager).
pub fn open(path: &Path) {
    let _ = Command::new(OPENER).arg(path).spawn();
}

/// Shows `path` selected in the file manager.
pub fn reveal(path: &Path) {
    if cfg!(target_os = "windows") {
        let _ = Command::new(OPENER).arg(format!("/select,{}", path.display())).spawn();
    } else if cfg!(target_os = "macos") {
        let _ = Command::new(OPENER).arg("-R").arg(path).spawn();
    } else {
        open(path.parent().unwrap_or(path));
    }
}

/// Opens a terminal window in `dir`.
pub fn terminal(dir: &Path) {
    let spawned = if cfg!(target_os = "windows") {
        // Prefer Windows Terminal, fall back to the classic console.
        Command::new("wt").arg("-d").arg(dir).spawn().or_else(|_| classic_console(dir))
    } else if cfg!(target_os = "macos") {
        Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn()
    } else {
        Command::new("x-terminal-emulator").current_dir(dir).spawn()
    };
    drop(spawned);
}

/// `cmd` in `dir`, in a console window of its own. Started directly: through `cmd /C start cmd`, the outer `cmd`
/// would flash a console window of its own first.
fn classic_console(dir: &Path) -> std::io::Result<std::process::Child> {
    let mut console = Command::new("cmd");
    console.current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        console.creation_flags(CREATE_NEW_CONSOLE);
    }
    console.spawn()
}
