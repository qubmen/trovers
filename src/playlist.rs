use crate::cache;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CacheStatus {
    Cached,
    Streaming,
    Downloading,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LoopMode {
    None,
    Track,
    Playlist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub url: String,
    pub source: String,
    pub title: String,
    pub artist: String,
    pub channel: String,
    pub duration: u64,
    pub video_id: String,
    pub cache_status: CacheStatus,
    pub file: Option<PathBuf>,
    pub last_position: u64,
    pub speed: Option<f32>,
    pub user_title: Option<String>,
    pub user_artist: Option<String>,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub created: DateTime<Utc>,
    pub loop_mode: LoopMode,
    pub default_speed: Option<f32>,
    pub tracks: Vec<Track>,
    pub current_track: Option<String>,
}

impl Playlist {
    /// Load a playlist from a TOML file.
    /// Resets any Cached track whose file is missing back to Streaming.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read playlist at {}", path.display()))?;
        let mut playlist: Playlist =
            toml::from_str(&raw).context("failed to parse playlist TOML")?;

        // File-existence check: if cached file was deleted or never written, treat as streaming.
        // Also reset any in-progress downloads (crash recovery) back to streaming.
        for track in &mut playlist.tracks {
            if track.cache_status == CacheStatus::Downloading {
                track.cache_status = CacheStatus::Streaming;
            } else if track.cache_status == CacheStatus::Cached {
                let exists = track.file.as_ref().map(|p| p.exists()).unwrap_or(false);
                if !exists {
                    track.cache_status = CacheStatus::Streaming;
                    track.file = None;
                }
            }
        }

        Ok(playlist)
    }

    /// Serialize playlist to TOML and write to disk atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let raw = toml::to_string(self).context("failed to serialize playlist")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &raw)
            .with_context(|| format!("failed to write tmp playlist at {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("failed to rename playlist to {}", path.display()))
    }

    /// Return paths of all .toml files in the playlists directory.
    pub fn list_all() -> Result<Vec<PathBuf>> {
        let dir = cache::playlists_dir();
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read playlists dir at {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(paths)
    }

    /// Create a new empty playlist and save it to disk.
    pub fn create(name: &str) -> Result<(Self, PathBuf)> {
        let playlist = Playlist {
            name: name.to_string(),
            created: Utc::now(),
            loop_mode: LoopMode::None,
            default_speed: None,
            tracks: Vec::new(),
            current_track: None,
        };
        let path = cache::playlists_dir().join(format!("{name}.toml"));
        playlist.save(&path)?;
        Ok((playlist, path))
    }

    /// Append a track to this playlist (does not save to disk).
    pub fn add_track(&mut self, track: Track) {
        self.tracks.push(track);
    }

    /// Rename this playlist to `new_name` by updating the name field,
    /// saving to a new path, and removing the old file.
    ///
    /// Returns the new path on success. The caller is responsible for
    /// updating any references to the old path.
    pub fn rename(&mut self, new_name: &str, old_path: &Path) -> Result<PathBuf> {
        let new_path = old_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!("{new_name}.toml"));

        self.name = new_name.to_string();
        // Save to new path first (atomic)
        self.save(&new_path)?;
        // Remove old file
        if old_path != new_path {
            std::fs::remove_file(old_path)
                .with_context(|| format!("failed to remove old playlist file at {}", old_path.display()))?;
        }
        Ok(new_path)
    }

    /// Delete this playlist by removing its file from disk.
    pub fn delete(path: &Path) -> Result<()> {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to delete playlist at {}", path.display()))
    }

    /// Remove a track by video_id and return it.
    /// Returns `None` if no track with the given video_id exists.
    pub fn remove_track_by_video_id(&mut self, video_id: &str) -> Option<Track> {
        if let Some(idx) = self.tracks.iter().position(|t| t.video_id == video_id) {
            let track = self.tracks.remove(idx);
            // Clear current_track pointer if it pointed to the removed track
            if self.current_track.as_deref() == Some(video_id) {
                self.current_track = None;
            }
            Some(track)
        } else {
            None
        }
    }
}
