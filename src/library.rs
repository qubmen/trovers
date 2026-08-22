//! The track library: one TOML document per track, indexed by its own `id`.
//!
//! Playlists hold ordered lists of these ids rather than embedding track data,
//! so a track's title, cache status, position and speed have exactly one home
//! however many playlists reference it.

use crate::playlist::{CacheStatus, Track};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

/// How many `-N` suffixes to try before giving up on finding a free filename for
/// a document. A bound rather than an unbounded loop: reaching it means something
/// is wrong with the directory, not that the 1000th spelling would have worked.
const MAX_FILENAME_ATTEMPTS: u32 = 1000;

/// Every track document under one directory, indexed by the `id` written inside
/// each one.
///
/// `root` is injected rather than looked up, so the library is testable against
/// a tempdir — the same reason `Playlist::load`/`save` take a `&Path`.
pub struct Library {
    root: PathBuf,
    tracks: HashMap<String, Track>,
    /// Which file each id was read from (or written to). The filename is derived
    /// from the id but is not authoritative, so the mapping has to be recorded
    /// rather than recomputed.
    paths: HashMap<String, PathBuf>,
}

impl Library {
    /// Read every `*.toml` under `root`, indexing by each document's inner `id`.
    /// A missing directory is an empty library; an unreadable or unparseable
    /// document is logged and skipped rather than failing the whole load.
    pub fn load(root: &Path) -> Result<Self> {
        let mut lib = Self {
            root: root.to_path_buf(),
            tracks: HashMap::new(),
            paths: HashMap::new(),
        };

        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            // No directory yet is the first-launch case, not an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(lib),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("failed to read track library at {}", root.display()))
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(e) => {
                    warn!(err = %e, path = %path.display(), "unreadable track document, skipping");
                    continue;
                }
            };
            let mut track: Track = match toml::from_str(&raw) {
                Ok(track) => track,
                Err(e) => {
                    warn!(err = %e, path = %path.display(), "unparseable track document, skipping");
                    continue;
                }
            };
            repair_cache_status(&mut track);
            // The `id` inside the document wins over the filename, which is only
            // a hint — the two can disagree after a `-N` collision suffix, a
            // case-folding filesystem, or a hand-moved file.
            lib.paths.insert(track.id.clone(), path);
            lib.tracks.insert(track.id.clone(), track);
        }

        Ok(lib)
    }

    pub fn get(&self, id: &str) -> Option<&Track> {
        self.tracks.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.get_mut(id)
    }

    /// Write one track's document to disk, atomically (tmp + rename), so an
    /// interrupted write can never leave a half-written document behind.
    pub fn save(&self, id: &str) -> Result<()> {
        let track = self
            .tracks
            .get(id)
            .with_context(|| format!("no track '{id}' in the library"))?;
        let path = self
            .paths
            .get(id)
            .with_context(|| format!("no document path recorded for track '{id}'"))?;

        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let raw = toml::to_string(track).context("failed to serialize track")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &raw)
            .with_context(|| format!("failed to write tmp document at {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("failed to rename document to {}", path.display()))
    }

    /// Insert or replace `track` and write its document. An id already in the
    /// library keeps the file it was read from; a new one gets a fresh name.
    pub fn upsert(&mut self, track: Track) -> Result<()> {
        let id = track.id.clone();
        if !self.paths.contains_key(&id) {
            let path = self.free_document_path(&track);
            self.paths.insert(id.clone(), path);
        }
        self.tracks.insert(id.clone(), track);
        self.save(&id)
    }

    /// Drop a track from the library and delete its document. An id that is not
    /// in the library is `Ok(None)`, not an error — deleting twice is harmless.
    pub fn remove(&mut self, id: &str) -> Result<Option<Track>> {
        let Some(track) = self.tracks.remove(id) else {
            return Ok(None);
        };
        if let Some(path) = self.paths.remove(id) {
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(e).with_context(|| {
                        format!("failed to delete track document at {}", path.display())
                    });
                }
            }
        }
        Ok(Some(track))
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Pairs with `len` — clippy asks for it, and the tests read it.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// A document path for a track new to this library: `<slug>-<platform-id>`,
    /// with `-2`, `-3`, ... appended until nothing else claims it.
    ///
    /// Collisions are real, not theoretical: YouTube ids are case-sensitive and
    /// macOS filenames are not, so `abc` and `ABC` want the same file.
    fn free_document_path(&self, track: &Track) -> PathBuf {
        let stem = format!(
            "{}-{}",
            sanitize(&source_slug(&track.source)),
            sanitize(track.platform_id())
        );
        let taken =
            |candidate: &PathBuf| self.paths.values().any(|p| p == candidate) || candidate.exists();

        let first = self.root.join(format!("{stem}.toml"));
        if !taken(&first) {
            return first;
        }
        for n in 2..MAX_FILENAME_ATTEMPTS {
            let candidate = self.root.join(format!("{stem}-{n}.toml"));
            if !taken(&candidate) {
                return candidate;
            }
        }
        // Every spelling taken means the directory is in a state no naming
        // scheme can resolve; overwriting the base name is the least surprising
        // way to keep going, and the id inside stays authoritative either way.
        warn!(stem = %stem, "no free document filename, reusing the base name");
        first
    }
}

/// Everything a filename can safely hold. Platform ids are opaque and some carry
/// `:` or `/`, neither of which belongs in a path component.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Bring a freshly read document's `cache_status` back in line with reality.
///
/// `Downloading` is crash recovery: no download survives a restart, so a document
/// still claiming one would spin forever. A `Cached` track whose file has gone
/// falls back to streaming rather than failing to play. `Failed` is deliberately
/// left alone — it is a real terminal state, not a crash artifact.
fn repair_cache_status(track: &mut Track) {
    match track.cache_status {
        CacheStatus::Downloading => track.cache_status = CacheStatus::Streaming,
        CacheStatus::Cached => {
            let exists = track.file.as_ref().is_some_and(|p| p.exists());
            if !exists {
                track.cache_status = CacheStatus::Streaming;
                track.file = None;
            }
        }
        CacheStatus::Streaming | CacheStatus::Failed => {}
    }
}

/// The short, stable name for a track's origin platform, derived from a
/// `Track::source` domain.
///
/// The *registrable* label, not the whole domain: `youtube.com` and
/// `music.youtube.com` both give `youtube`, so one video reached via two host
/// spellings stays one document with one playback position. Case is folded for
/// the same reason — host names are case-insensitive.
pub fn source_slug(source: &str) -> String {
    let labels: Vec<&str> = source.split('.').filter(|l| !l.is_empty()).collect();
    let slug = match labels.len() {
        0 => "unknown",
        1 => labels[0],
        n => labels[n - 2],
    };
    slug.to_lowercase()
}

/// Build a library id from a track's `source` domain and its platform-native id.
pub fn make_id(source: &str, platform_id: &str) -> String {
    format!("{}:{}", source_slug(source), platform_id)
}

/// The platform-native id encoded in a library `id` — what the audio cache and
/// yt-dlp are keyed by.
///
/// Splits on the *first* colon only: platform ids are opaque strings minted by
/// yt-dlp per site and some contain colons of their own.
pub fn platform_id_of(id: &str) -> &str {
    match id.split_once(':') {
        Some((_, platform_id)) => platform_id,
        None => id,
    }
}
