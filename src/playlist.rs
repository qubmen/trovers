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
    /// All download attempts (see `ytdlp::download_with_retries`) were
    /// exhausted. Unlike `Downloading`, this is a real terminal state and
    /// survives restarts — `Playlist::load`'s crash recovery only resets
    /// `Downloading`, never `Failed`. Cleared only by a fresh download,
    /// automatic (re-adding the track) or manual (the recache hotkey).
    Failed,
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
    /// Library id — `<source-slug>:<platform-id>`, the name of this track's
    /// document and the only thing playlists store. See `crate::library`.
    pub id: String,
    pub cache_status: CacheStatus,
    pub file: Option<PathBuf>,
    pub last_position: u64,
    pub speed: Option<f32>,
    pub user_title: Option<String>,
    pub user_artist: Option<String>,
    pub added_at: DateTime<Utc>,
}

impl Track {
    /// The platform's own id for this track — what the audio cache filename and
    /// yt-dlp are keyed by. Derived from `id` so there is no second field to
    /// fall out of step with it.
    pub fn platform_id(&self) -> &str {
        crate::library::platform_id_of(&self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub created: DateTime<Utc>,
    pub loop_mode: LoopMode,
    /// Whether next/previous and auto-advance walk this playlist in a shuffled
    /// order. `default` so playlists written before shuffle existed still load.
    #[serde(default)]
    pub shuffle: bool,
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
            shuffle: false,
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
            std::fs::remove_file(old_path).with_context(|| {
                format!(
                    "failed to remove old playlist file at {}",
                    old_path.display()
                )
            })?;
        }
        Ok(new_path)
    }

    /// Delete this playlist by removing its file from disk.
    pub fn delete(path: &Path) -> Result<()> {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to delete playlist at {}", path.display()))
    }

    /// Remove a track by library id and return it.
    /// Returns `None` if no track with the given id exists.
    pub fn remove_track_by_id(&mut self, id: &str) -> Option<Track> {
        if let Some(idx) = self.tracks.iter().position(|t| t.id == id) {
            let track = self.tracks.remove(idx);
            // Clear current_track pointer if it pointed to the removed track
            if self.current_track.as_deref() == Some(id) {
                self.current_track = None;
            }
            Some(track)
        } else {
            None
        }
    }
}

/// A shuffled traversal order over `0..len`.
///
/// A permutation rather than a random pick per step: it is what makes a
/// shuffled walk visit every track exactly once before repeating any, and what
/// gives "previous" a meaningful answer. Reproducible from `seed`, which is
/// what makes it testable — callers pass `shuffle_seed()` for a real shuffle.
pub fn shuffled_indices(len: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    // Fisher-Yates. `xorshift64*` is plenty for deciding play order and saves a
    // dependency; nothing here is security-sensitive.
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for i in (1..len).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

/// A seed for `shuffled_indices` taken from the clock.
pub fn shuffle_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}
