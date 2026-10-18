//! Handing a Library item's file to the OS default player.

use std::path::Path;
use std::process::{Command, Stdio};

/// Opens a file with the OS default handler, detached from pahe so playback
/// outlives the window that launched it.
pub fn open_externally(file: &Path) -> Result<(), String> {
    let mut command = opener(file);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("The system could not open this Episode: {error}"))
}

#[cfg(target_os = "macos")]
fn opener(file: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg(file);
    command
}

#[cfg(target_os = "windows")]
fn opener(file: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = Command::new("cmd");
    command
        .args(["/C", "start", ""])
        .arg(file)
        .creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn opener(file: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(file);
    command
}
