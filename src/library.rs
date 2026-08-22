//! The track library: one TOML document per track, indexed by its own `id`.
//!
//! Playlists hold ordered lists of these ids rather than embedding track data,
//! so a track's title, cache status, position and speed have exactly one home
//! however many playlists reference it.

use crate::playlist::{CacheStatus, LoopMode, Playlist, Track};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

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

// ── Migration from embedded playlists ─────────────────────────────────────

/// What a migration did, for the log and for the status line on the launch that
/// ran it.
pub struct MigrationReport {
    /// Playlists rewritten as id lists.
    pub playlists: usize,
    /// Documents newly written. Lower than the number of rows migrated whenever
    /// playlists shared a track — which is the point.
    pub tracks: usize,
    pub backup: PathBuf,
}

/// A track row as playlists used to embed it: its own copy of every field, keyed
/// by the platform's `video_id` rather than a library id.
#[derive(Deserialize)]
struct LegacyTrack {
    url: String,
    source: String,
    title: String,
    artist: String,
    channel: String,
    duration: u64,
    video_id: String,
    cache_status: CacheStatus,
    file: Option<PathBuf>,
    last_position: u64,
    speed: Option<f32>,
    user_title: Option<String>,
    user_artist: Option<String>,
    added_at: DateTime<Utc>,
}

impl LegacyTrack {
    fn into_track(self, id: String) -> Track {
        Track {
            url: self.url,
            source: self.source,
            title: self.title,
            artist: self.artist,
            channel: self.channel,
            duration: self.duration,
            id,
            cache_status: self.cache_status,
            file: self.file,
            last_position: self.last_position,
            speed: self.speed,
            user_title: self.user_title,
            user_artist: self.user_artist,
            added_at: self.added_at,
        }
    }
}

/// A playlist's `tracks` in either format.
///
/// Untagged, so the shape on disk decides and nothing has to be versioned: a
/// list of strings is already migrated, a list of tables is not. `tracks = []`
/// matches `Ids` first, which is the right answer — an empty playlist has
/// nothing to migrate whichever format wrote it.
#[derive(Deserialize)]
#[serde(untagged)]
enum TrackList {
    /// Already migrated. The ids themselves are never read — matching the shape
    /// is the whole job, and the playlist is then left alone.
    Ids(#[allow(dead_code)] Vec<String>),
    Embedded(Vec<LegacyTrack>),
}

#[derive(Deserialize)]
struct MaybeLegacyPlaylist {
    name: String,
    created: DateTime<Utc>,
    loop_mode: LoopMode,
    #[serde(default)]
    shuffle: bool,
    default_speed: Option<f32>,
    tracks: TrackList,
    current_track: Option<String>,
}

/// A playlist that still embeds its tracks, with everything needed to write the
/// migrated version alongside the rows to turn into documents.
struct LegacyPlaylist {
    path: PathBuf,
    name: String,
    created: DateTime<Utc>,
    loop_mode: LoopMode,
    shuffle: bool,
    default_speed: Option<f32>,
    current_track: Option<String>,
    rows: Vec<LegacyTrack>,
}

/// Move any playlist that still embeds its tracks over to the library model:
/// one document per track, the playlist left holding an ordered list of ids.
///
/// Safe to run on every launch. Detection is by shape, so a playlist already
/// holding ids is recognised and left byte-for-byte alone, and `Ok(None)` means
/// there was nothing to do. `playlists/` is copied to
/// `playlists.backup-<utc>/` before anything is written, so a migration that
/// goes wrong is recoverable by hand.
pub fn migrate(playlists_dir: &Path, tracks_dir: &Path) -> Result<Option<MigrationReport>> {
    let legacy = collect_legacy_playlists(playlists_dir)?;
    if legacy.is_empty() {
        return Ok(None);
    }

    // Before any mutation, including the first document write.
    let backup = back_up_playlists(playlists_dir)?;
    info!(backup = %backup.display(), playlists = legacy.len(), "migrating playlists to the track library");

    let mut library = Library::load(tracks_dir)?;
    let mut new_documents = 0usize;
    let playlists = legacy.len();

    for pl in legacy {
        // Mapped from the rows, so a stale pointer at a video the playlist does
        // not list resolves to nothing rather than to a row that isn't there.
        let current_track = pl.current_track.and_then(|video_id| {
            pl.rows
                .iter()
                .find(|row| row.video_id == video_id)
                .map(|row| make_id(&row.source, &row.video_id))
        });

        let mut ids = Vec::with_capacity(pl.rows.len());
        for row in pl.rows {
            let id = make_id(&row.source, &row.video_id);
            // First writer wins: the same video in two playlists had two
            // independent copies of its state, and there is no way to tell which
            // one the user would have picked.
            if library.get(&id).is_some() {
                info!(id = %id, "track already has a document, keeping it");
            } else {
                library.upsert(row.into_track(id.clone()))?;
                new_documents += 1;
            }
            ids.push(id);
        }

        let migrated = Playlist {
            name: pl.name,
            created: pl.created,
            loop_mode: pl.loop_mode,
            shuffle: pl.shuffle,
            default_speed: pl.default_speed,
            tracks: ids,
            current_track,
        };
        migrated.save(&pl.path)?;
    }

    info!(playlists, tracks = new_documents, "migration complete");
    Ok(Some(MigrationReport {
        playlists,
        tracks: new_documents,
        backup,
    }))
}

/// Every playlist under `playlists_dir` that still embeds its tracks, sorted by
/// path so a run is reproducible (and so "first writer wins" is deterministic).
/// A playlist that cannot be read or parsed is logged and skipped: migration
/// cannot know what it meant, and refusing to launch over it would be worse.
fn collect_legacy_playlists(playlists_dir: &Path) -> Result<Vec<LegacyPlaylist>> {
    let entries = match std::fs::read_dir(playlists_dir) {
        Ok(entries) => entries,
        // No playlists directory yet is first launch, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "failed to read playlists dir at {}",
                    playlists_dir.display()
                )
            })
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    paths.sort();

    let mut legacy = Vec::new();
    for path in paths {
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                warn!(err = %e, path = %path.display(), "unreadable playlist, leaving it alone");
                continue;
            }
        };
        let parsed: MaybeLegacyPlaylist = match toml::from_str(&raw) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!(err = %e, path = %path.display(), "unparseable playlist, leaving it alone");
                continue;
            }
        };
        let TrackList::Embedded(rows) = parsed.tracks else {
            continue;
        };
        legacy.push(LegacyPlaylist {
            path,
            name: parsed.name,
            created: parsed.created,
            loop_mode: parsed.loop_mode,
            shuffle: parsed.shuffle,
            default_speed: parsed.default_speed,
            current_track: parsed.current_track,
            rows,
        });
    }
    Ok(legacy)
}

/// Copy `playlists_dir` to a timestamped sibling and return where it went.
///
/// A hard failure on purpose: the backup is the only way back, so nothing may be
/// rewritten if it could not be taken.
fn back_up_playlists(playlists_dir: &Path) -> Result<PathBuf> {
    let parent = playlists_dir.parent().unwrap_or_else(|| Path::new("."));
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");

    let mut backup = parent.join(format!("playlists.backup-{stamp}"));
    let mut n = 2;
    while backup.exists() && n < MAX_FILENAME_ATTEMPTS {
        backup = parent.join(format!("playlists.backup-{stamp}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&backup)
        .with_context(|| format!("failed to create backup dir at {}", backup.display()))?;

    for entry in std::fs::read_dir(playlists_dir)
        .with_context(|| format!("failed to read {}", playlists_dir.display()))?
        .flatten()
    {
        let from = entry.path();
        if !from.is_file() {
            continue;
        }
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = backup.join(name);
        std::fs::copy(&from, &to)
            .with_context(|| format!("failed to back up {} to {}", from.display(), to.display()))?;
    }
    Ok(backup)
}
