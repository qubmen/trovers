use anyhow::{bail, Result};
use std::process::Command;

/// Verify that yt-dlp and mpv are available in PATH.
/// Prints a clear error with install instructions and returns Err if either is missing.
pub fn check() -> Result<()> {
    check_binary(
        "yt-dlp",
        "Install with: pip install yt-dlp  (or your system package manager)",
    )?;
    check_binary(
        "mpv",
        "Install with: brew install mpv  (macOS) / apt install mpv  (Debian/Ubuntu)",
    )?;
    Ok(())
}

fn check_binary(name: &str, install_hint: &str) -> Result<()> {
    let ok = Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok {
        bail!(
            "Required binary '{name}' not found in PATH.\n  {install_hint}"
        );
    }
    Ok(())
}
