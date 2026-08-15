use anyhow::{Context, Result};
use std::path::PathBuf;

/// Returns ~/.local/share/trovers/audio/
pub fn audio_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("trovers")
        .join("audio")
}

/// Returns ~/.local/share/trovers/playlists/
pub fn playlists_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("trovers")
        .join("playlists")
}

/// Returns the full path for a cached audio file: audio_dir/<video_id>.<ext>
pub fn audio_path(video_id: &str, ext: &str) -> PathBuf {
    audio_dir().join(format!("{video_id}.{ext}"))
}

/// Creates audio and playlists directories if they do not exist.
pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(audio_dir()).context("failed to create audio directory")?;
    std::fs::create_dir_all(playlists_dir()).context("failed to create playlists directory")?;
    Ok(())
}
