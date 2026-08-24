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

/// Returns ~/.local/share/trovers/tracks/ — one TOML document per track, which
/// is what playlists reference by id. See `crate::library`.
pub fn tracks_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("trovers")
        .join("tracks")
}

/// Creates the audio, playlists and tracks directories if they do not exist.
pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(audio_dir()).context("failed to create audio directory")?;
    std::fs::create_dir_all(playlists_dir()).context("failed to create playlists directory")?;
    std::fs::create_dir_all(tracks_dir()).context("failed to create tracks directory")?;
    Ok(())
}
