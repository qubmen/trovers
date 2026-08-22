use crate::cache;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LoopMode {
    None,
    Track,
    Playlist,
}

/// Whether a playlist stands on its own or hangs under another one.
///
/// Two levels, deliberately: a playlist and the albums inside it. Deeper nesting
/// buys a tree widget and a lot of cursor arithmetic for very little.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistKind {
    #[default]
    Normal,
    Album,
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
    /// Library ids, in playing order — see `crate::library`. A playlist is a
    /// running order over the library, not a container of track data, so the
    /// same track can sit in several playlists with one position between them.
    pub tracks: Vec<String>,
    pub current_track: Option<String>,
    /// The three below are `default`ed so every playlist written before albums
    /// existed loads as what it is: a top-level list belonging to nobody.
    #[serde(default)]
    pub kind: PlaylistKind,
    /// The playlist this album sits under, by name. A name rather than a path
    /// because that is what the sidebar and the rename path both address a
    /// playlist by; a dangling name simply orphans the album to the top level.
    #[serde(default)]
    pub parent: Option<String>,
    /// The folder this album mirrors, when it was imported from one. Its presence
    /// is what makes an album rescannable.
    #[serde(default)]
    pub source_folder: Option<PathBuf>,
}

/// A playlist as the sidebar needs it: enough to draw and address it, without
/// loading every track list on every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    /// The file's stem, which is the name everything else addresses it by.
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaylistKind,
    pub parent: Option<String>,
}

impl PlaylistEntry {
    /// A top-level playlist belonging to nobody — what creating one gives you.
    pub fn normal(name: String, path: PathBuf) -> Self {
        PlaylistEntry {
            name,
            path,
            kind: PlaylistKind::Normal,
            parent: None,
        }
    }
}

impl Playlist {
    /// Load a playlist from a TOML file.
    ///
    /// Nothing to repair here any more: a playlist holds only ids, and reconciling
    /// a track's recorded `cache_status` with what is actually on disk is
    /// `Library::load`'s job — one place rather than once per playlist file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read playlist at {}", path.display()))?;
        toml::from_str(&raw).context("failed to parse playlist TOML")
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

    /// Every playlist file in `dir`, sorted by name.
    ///
    /// Each file is parsed for its `kind` and `parent` — the sidebar has to know
    /// what hangs under what before it can draw a single row. One that will not
    /// parse is still listed, as a normal playlist: a broken file the user can see
    /// and delete beats a row that silently vanished.
    pub fn list_entries(dir: &Path) -> Result<Vec<PlaylistEntry>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)
            .with_context(|| format!("failed to read playlists dir at {}", dir.display()))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let (kind, parent) = match Playlist::load(&path) {
                Ok(pl) => (pl.kind, pl.parent),
                Err(e) => {
                    warn!(err = %e, path = %path.display(), "unreadable playlist, listing it as a plain one");
                    (PlaylistKind::Normal, None)
                }
            };
            entries.push(PlaylistEntry {
                name: name.to_string(),
                path,
                kind,
                parent,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
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
            kind: PlaylistKind::Normal,
            parent: None,
            source_folder: None,
        };
        let path = cache::playlists_dir().join(format!("{name}.toml"));
        playlist.save(&path)?;
        Ok((playlist, path))
    }

    /// Append a library id to this playlist (does not save to disk). Writing the
    /// track's own document is the library's business — see `Library::upsert`.
    pub fn add_track(&mut self, id: String) {
        self.tracks.push(id);
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

    /// Drop the first row referencing `id`, returning whether one was there.
    ///
    /// The track itself is untouched — it lives in the library and may well be
    /// listed by other playlists.
    pub fn remove_track_by_id(&mut self, id: &str) -> bool {
        let Some(idx) = self.tracks.iter().position(|t| t == id) else {
            return false;
        };
        self.tracks.remove(idx);
        // Clear current_track pointer if it pointed to the removed row.
        if self.current_track.as_deref() == Some(id) {
            self.current_track = None;
        }
        true
    }
}

/// Playlist entries in display order, each paired with its indent level.
///
/// Top-level playlists come alphabetically, each immediately followed by its own
/// albums, also alphabetically. Only an album nests, and only under a *normal*
/// playlist that is actually present — which keeps the tree two deep and gives
/// every broken link the same harmless answer: an album whose parent was deleted,
/// or which names another album (or itself), comes back as a top-level row.
///
/// Reorders rows; never adds or drops one.
pub fn nested_order(entries: &[PlaylistEntry]) -> Vec<(&PlaylistEntry, usize)> {
    let can_parent = |name: &str| {
        entries
            .iter()
            .any(|e| e.name == name && e.kind == PlaylistKind::Normal)
    };
    let parent_of = |entry: &PlaylistEntry| -> Option<String> {
        if entry.kind != PlaylistKind::Album {
            return None;
        }
        entry.parent.clone().filter(|p| can_parent(p))
    };

    let mut sorted: Vec<&PlaylistEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut rows = Vec::with_capacity(sorted.len());
    for entry in &sorted {
        // Anything with a real parent is emitted below that parent instead.
        if parent_of(entry).is_some() {
            continue;
        }
        rows.push((*entry, 0));
        for child in &sorted {
            if parent_of(child).as_deref() == Some(entry.name.as_str()) {
                rows.push((*child, 1));
            }
        }
    }
    rows
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
