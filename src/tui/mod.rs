pub mod input;
pub mod ui;

#[cfg(test)]
mod ui_test;

use crate::cache;
use crate::config::{AudioQuality, Config};
use crate::library::{self, Library};
use crate::library::{CacheStatus, MediaKind, Track, TrackOrigin};
use crate::library_import;
use crate::library_scan;
use crate::player::{self, Player};
use crate::playlist::{self, LoopMode, Playlist, PlaylistEntry, PlaylistKind};
use crate::ytdlp::{self, TrackMeta};
use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

// ── Focus ──────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone)]
pub enum Focus {
    Sidebar,
    TrackList,
    Settings,
}

// ── InputMode ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone)]
pub enum InputMode {
    Normal,
    UrlInput,
    NewPlaylist,
    /// Typing the name of a new, empty album under the displayed playlist —
    /// the manual counterpart to importing a folder as one.
    NewAlbum,
    ConfirmDelete,
    SearchInput,
    TrackContextMenu,
    PlaylistRename,
    PlaylistDelete,
    /// Renaming or forgetting the album whose header the cursor is on. Separate
    /// modes from the sidebar's, because they address a different thing: the album
    /// under the cursor in the track list, not the sidebar's selected row.
    AlbumRename,
    AlbumDelete,
    /// Typing the path of a folder to import as an album.
    FolderInput,
    Help,
}

// ── SidebarItem ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarItem {
    PlaylistsHeader,
    Playlist {
        name: String,
        path: PathBuf,
        /// Whether to mark the row as an album. Only an *orphaned* album is
        /// listed here at all — one with a live parent is drawn inside that
        /// parent's track list — but it is still an album and says so.
        is_album: bool,
    },
    Separator,
    Music,
    Video,
    Plunder,
    /// Import a local folder as an album — the discoverable half of `F`.
    ImportFolder,
    Settings,
}

impl SidebarItem {
    pub fn is_selectable(&self) -> bool {
        matches!(
            self,
            SidebarItem::PlaylistsHeader
                | SidebarItem::Playlist { .. }
                | SidebarItem::Plunder
                | SidebarItem::ImportFolder
                | SidebarItem::Settings
        )
    }
}

// ── Import target ─────────────────────────────────────────────────────────

/// Which playlist file an import's rows land in.
///
/// Decided before the scan starts, so the answer cannot drift while ffprobe
/// works through a few hundred files.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportTarget {
    /// A new album, under `parent` when there is one to hang it under.
    NewAlbum { parent: Option<String> },
    /// A playlist that already exists — an album being rescanned.
    Existing(PathBuf),
}

// ── Task messages (async → event loop) ───────────────────────────────────

pub enum TaskMsg {
    MetaReady {
        url: String,
        meta: TrackMeta,
        target_path: Option<PathBuf>,
    },
    MetaError {
        url: String,
        err: String,
    },
    DownloadDone {
        id: String,
        file: PathBuf,
    },
    DownloadError {
        id: String,
        err: String,
    },
    /// A freshly spawned mpv is ready. `generation` identifies which playback
    /// request it belongs to, so a player that finished starting *after* the
    /// user already moved on is discarded instead of hijacking the new track.
    PlayerReady {
        id: String,
        player: Box<Player>,
        generation: u64,
    },
    PlayerError {
        id: String,
        err: String,
    },
    /// mpv exited on its own — it reached the end of the track, or crashed.
    /// Without this the app kept a `Player` pointing at a dead socket, showed
    /// "▶ Playing", and then died the moment any key sent an IPC command.
    PlayerGone {
        generation: u64,
    },
    /// One more file of a folder import has been probed.
    ImportProgress {
        done: usize,
        total: usize,
    },
    /// A folder import finished scanning. The merge itself happens on this side,
    /// because only the event loop owns the library and the album file.
    ImportScanned {
        root: PathBuf,
        target: ImportTarget,
        files: Vec<library_import::ImportedFile>,
    },
    /// A single local file (as opposed to a whole folder) has been probed and
    /// is ready to fold into `target_path`.
    FileProbed {
        file: library_import::ImportedFile,
        target_path: PathBuf,
    },
}

// ── Speed resolution ──────────────────────────────────────────────────────

pub fn effective_speed(track: &Track, playlist: &Playlist, config: &Config) -> f32 {
    track
        .speed
        .or(playlist.default_speed)
        .unwrap_or(config.default_speed)
}

// ── Settings items ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsItem {
    AudioQuality,
    DefaultSpeed,
    DefaultVolume,
}

pub const SETTINGS_ITEMS: &[SettingsItem] = &[
    SettingsItem::AudioQuality,
    SettingsItem::DefaultSpeed,
    SettingsItem::DefaultVolume,
];

/// How often the playing track's position is written to disk while it plays.
/// Still throttled — a flush is a file write either way — but it is now one small
/// track document rather than the whole playlist. Without any periodic flush at
/// all a hard kill discarded the entire session's progress.
const POSITION_FLUSH_INTERVAL: Duration = Duration::from_secs(15);

/// How far short of a track's duration mpv may exit and still count as having
/// reached the end. The position poller samples once a second, so the last
/// reading always lags a little behind where mpv actually got to — and a stream
/// whose reported duration is slightly optimistic lags further still.
const EOF_SLACK_SECS: f64 = 10.0;

// ── PlayingSession ────────────────────────────────────────────────────────

/// Which track is actually driving playback right now, and out of which
/// playlist — independent of whichever playlist the user happens to be browsing.
///
/// `playlist` is a copy of the playlist file the playing track was started from.
/// It carries no track data any more, only the running order and this list's own
/// `loop_mode`/`shuffle`/`default_speed`, which is what auto-advance needs to
/// keep following that playlist while the user browses elsewhere. The track
/// itself is read from `App::library` by `track_id`, so there is only ever one
/// copy of it to update.
pub struct PlayingSession {
    pub path: PathBuf,
    pub playlist: Playlist,
    pub track_id: String,
}

impl PlayingSession {
    /// Where the playing track sits in its playlist's running order, or `None`
    /// if the row has since been removed from that list.
    pub fn track_index(&self) -> Option<usize> {
        self.playlist
            .tracks
            .iter()
            .position(|id| id == &self.track_id)
    }
}

// ── Visible rows ──────────────────────────────────────────────────────────

/// Which list a visible row's track comes out of.
///
/// A row on screen is no longer an index into one vector: the displayed playlist
/// and each album under it are separate files with separate running orders, so a
/// row has to say which one it means before anything can play, delete or reorder
/// it (ADR-019).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowSource {
    /// The displayed playlist's own tracks.
    Own,
    /// An album under it, by index into `App::albums`.
    Album(usize),
}

/// One line of the track table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VisibleRow {
    /// `index` indexes the `tracks` of whichever list `source` names — never the
    /// screen, which is what `App::rows` itself is.
    Track { source: RowSource, index: usize },
    /// An album's own line: its name, its size, and whether it is open.
    AlbumHeader { album: usize },
}

/// An album under the displayed playlist, held in memory while it is on screen.
///
/// The `Playlist` is the same struct as any other, because an album *is* an
/// ordinary playlist file — which is what lets one play as its own list without
/// the rest of playback knowing that albums exist at all.
pub struct LoadedAlbum {
    /// The file stem, which is what `parent` and a rename address it by.
    pub name: String,
    pub path: PathBuf,
    pub playlist: Playlist,
}

/// The album's resume target: the index of its `current_track` within its own
/// running order, and that track's `last_position` if it has one. `None` only
/// when the album has no tracks at all — there is nothing to play or mark. A
/// `current_track` naming a track no longer in the album is treated the same
/// as having none, and resolves to track 0 from the start.
///
/// Shared by the "play this album" trigger and the track table's resume-point
/// marker, so the two can never name different rows.
pub(crate) fn album_resume_target(
    loaded: &LoadedAlbum,
    library: &Library,
) -> Option<(usize, Option<f64>)> {
    if loaded.playlist.tracks.is_empty() {
        return None;
    }
    let resolved = loaded
        .playlist
        .current_track
        .as_deref()
        .and_then(|id| loaded.playlist.tracks.iter().position(|t| t == id));
    match resolved {
        Some(idx) => {
            let start_pos = loaded
                .playlist
                .tracks
                .get(idx)
                .and_then(|id| library.get(id))
                .and_then(input::resume_start_pos);
            Some((idx, start_pos))
        }
        None => Some((0, None)),
    }
}

// ── App ───────────────────────────────────────────────────────────────────

pub struct App {
    // Playlist & config
    pub playlist: Playlist,
    pub playlist_path: PathBuf,
    /// Every track known to trovers. The displayed playlist holds ids into this;
    /// resolving a row means a library lookup.
    pub library: Library,
    pub config: Config,
    pub player: Option<Player>,
    /// Monotonic counter identifying the *current* playback request. Bumped
    /// every time a player is stopped or replaced (see `stop_player`), and
    /// shared with the async spawn task and the position poller so both can tell
    /// whether the player they are working on is still the one the app wants.
    /// Anything carrying a stale generation is discarded rather than applied.
    pub player_generation: Arc<AtomicU64>,
    /// The session (playlist + track index) actually driving playback right
    /// now — independent of whichever playlist is currently displayed.
    pub playing: Option<PlayingSession>,

    // Async channels
    pub pos_tx: watch::Sender<f64>,
    pub position_rx: watch::Receiver<f64>,
    pub download_tx: watch::Sender<(String, f32)>,
    pub download_rx: watch::Receiver<(String, f32)>,
    pub task_tx: mpsc::UnboundedSender<TaskMsg>,
    pub task_rx: mpsc::UnboundedReceiver<TaskMsg>,

    // UI state
    pub input_mode: InputMode,
    pub input_buf: String,
    pub focus: Focus,
    pub should_quit: bool,

    // Track list
    pub selected: usize,
    pub track_offset: usize,
    pub track_list_height: u16,
    /// The albums hanging under the displayed playlist, alphabetically.
    pub albums: Vec<LoadedAlbum>,
    /// Every row on screen, in display order — the cursor's coordinate system.
    /// Derived from `playlist`, `albums`, `search_query` and each album's
    /// `collapsed`; `rebuild_rows` is its only writer.
    pub rows: Vec<VisibleRow>,
    /// The active search text, lowercased on use. Held apart from `input_buf`,
    /// which is cleared the moment the prompt closes while the filter stays on.
    pub search_query: String,

    // Sidebar
    pub sidebar_selected: usize,
    pub playlists_expanded: bool,
    /// Every playlist file on disk, sorted by name — the sidebar's source of
    /// truth for what exists and what hangs under what.
    pub available_playlists: Vec<PlaylistEntry>,

    // Settings panel
    pub settings_selected: usize,

    // Playback state
    pub position: f64,
    /// Per-track download progress percentage (0.0-100.0), keyed by
    /// `id`. A `HashMap` (rather than a single global `f32`) so
    /// multiple concurrent downloads never cross-contaminate each other's
    /// displayed percentage.
    pub download_progress: HashMap<String, f32>,
    pub is_paused: bool,

    // Footer status message (toast-style)
    pub status_message: Option<(String, Instant)>,

    // Tracks being downloaded
    pub downloading: HashSet<String>,
    // In-flight metadata fetches
    pub pending_fetches: usize,
    /// When the playing track's position was last written to disk — see
    /// `maybe_flush_position`.
    pub last_position_flush: Instant,

    /// Shuffled traversal order over the tracks of `shuffle_order_path`, as
    /// indices into that playlist's `tracks`. A stored permutation rather than a
    /// random pick per step, so a shuffled walk hits every track once before
    /// repeating and `b` can step back through it. Empty when shuffle is off.
    pub shuffle_order: Vec<usize>,
    /// The playlist `shuffle_order` was built for. An order is only valid for
    /// one playlist file at one length; anything else forces a rebuild.
    pub shuffle_order_path: Option<PathBuf>,

    // Context menu
    pub context_menu_selected: usize,

    // URL input playlist target
    pub target_playlist_for_url: Option<String>,
    /// The explicit destination for a folder or single-file add, cycled by
    /// `Tab` in the folder prompt. `None` means "Auto" — a folder keeps
    /// today's `import_target_for` matching, a file lands in the displayed
    /// playlist — `Some(name)` bypasses that and merges straight into the
    /// named list regardless of any folder it already recognizes.
    pub target_list_for_add: Option<String>,
}

impl App {
    pub fn new(
        playlist: Playlist,
        config: Config,
        available_playlists: Vec<PlaylistEntry>,
        playlist_path: PathBuf,
        library: Library,
    ) -> Self {
        let (pos_tx, position_rx) = watch::channel(0.0f64);
        let (download_tx, download_rx) = watch::channel((String::new(), 0.0f32));
        let (task_tx, task_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            playlist,
            playlist_path,
            library,
            config,
            player: None,
            player_generation: Arc::new(AtomicU64::new(0)),
            playing: None,
            pos_tx,
            position_rx,
            download_tx,
            download_rx,
            task_tx,
            task_rx,
            input_mode: InputMode::Normal,
            input_buf: String::new(),
            focus: Focus::TrackList,
            should_quit: false,
            selected: 0,
            track_offset: 0,
            track_list_height: 10,
            albums: Vec::new(),
            rows: Vec::new(),
            search_query: String::new(),
            sidebar_selected: 0,
            playlists_expanded: true,
            available_playlists,
            settings_selected: 0,
            position: 0.0,
            download_progress: HashMap::new(),
            is_paused: false,
            status_message: None,
            downloading: HashSet::new(),
            pending_fetches: 0,
            last_position_flush: Instant::now(),
            shuffle_order: Vec::new(),
            shuffle_order_path: None,
            context_menu_selected: 0,
            target_playlist_for_url: None,
            target_list_for_add: None,
        };
        app.load_albums();
        app.rebuild_rows();
        // `current_track` is an index into the playlist's own list, and the cursor
        // counts rows, so it has to be translated rather than assigned.
        if let Some(cursor) = app
            .current_track_index()
            .and_then(|index| app.cursor_of_own_index(index))
        {
            app.selected = cursor;
        }
        app
    }

    /// Index (within the *displayed* playlist, `self.playlist`) of the track
    /// marked as `current_track` in the playlist file. This is now used
    /// strictly to restore the cursor position when a playlist is (re)loaded
    /// from disk — it is **not** the source of truth for "what's playing".
    /// See `self.playing` (`PlayingSession`) for that.
    pub fn current_track_index(&self) -> Option<usize> {
        let id = self.playlist.current_track.as_deref()?;
        self.playlist.tracks.iter().position(|t| t == id)
    }

    /// Returns true if the track identified by `(path, id)` is
    /// literally the one actually driving playback right now — i.e.
    /// `self.playing` points at a session whose playlist file is `path` and
    /// whose current track's `id` matches. Used to guard delete/move
    /// operations so they only stop playback when the track being removed is
    /// truly the one playing, not just any track that happens to share a
    /// `id` with an unrelated playing session in a different playlist.
    pub fn is_playing_track(&self, path: &Path, id: &str) -> bool {
        self.playing
            .as_ref()
            .is_some_and(|p| p.path == path && p.track_id == id)
    }

    /// Persist whatever mutation was just made (via `playing_track_mut()`) to the
    /// track actually driving playback. No-op if nothing is playing.
    ///
    /// One small document, whoever is browsing what: the playing track has a
    /// single home in the library, so there is no longer any displayed-vs-session
    /// copy to reconcile before writing.
    pub fn save_playing_track(&mut self) {
        let Some(id) = self.playing.as_ref().map(|p| p.track_id.clone()) else {
            return;
        };
        if let Err(e) = self.library.save(&id) {
            error!(err = %e, id = %id, "failed to save the playing track's document");
        }
    }

    /// Returns the playlist that owns the currently playing track, if any —
    /// the playing session's own private copy, or `self.playlist` when the
    /// displayed playlist happens to be the one that's playing. Used for
    /// lookups like `default_speed` fallback that need "the playlist the
    /// playing track lives in", not "the displayed playlist".
    pub fn playing_playlist(&self) -> Option<&Playlist> {
        self.playing.as_ref().map(|p| &p.playlist)
    }

    /// Returns the track that is actually driving playback right now, if any.
    ///
    /// One lookup, no reconciliation: the session records an id and the library
    /// holds the single copy of that track, so an edit made through the track
    /// list is visible here immediately.
    pub fn playing_track(&self) -> Option<&Track> {
        self.library.get(&self.playing.as_ref()?.track_id)
    }

    /// Mutable counterpart of `playing_track`. Mutating a track does **not**
    /// persist it — call `save_playing_track` once the edit is complete.
    pub fn playing_track_mut(&mut self) -> Option<&mut Track> {
        let id = self.playing.as_ref()?.track_id.clone();
        self.library.get_mut(&id)
    }

    /// Kick off a background download for the track `id`. Shared by the
    /// add-track flow and the manual recache key (`c`) — both need identical
    /// bookkeeping, just triggered differently and (for recache) regardless of
    /// the track's current `cache_status`.
    ///
    /// No playlist bookkeeping any more: the download lands in the track's own
    /// document, which every playlist listing it reads from.
    ///
    /// Retries on failure (`ytdlp::download_with_retries`), so a track only
    /// reaches `Failed` after every attempt has been exhausted.
    fn start_download(&mut self, id: String, url: String) {
        self.downloading.insert(id.clone());

        let task_tx = self.task_tx.clone();
        let dl_tx = self.download_tx.clone();
        let quality = self.config.audio_quality.clone();
        let audio_dir = cache::audio_dir();
        // The cached file is named after the *platform's* id, not the library
        // id — that is what keeps audio downloaded by earlier versions valid.
        // Progress, meanwhile, is keyed by the library id, because that is what
        // the rows on screen are keyed by.
        let platform_id = library::platform_id_of(&id).to_string();
        tokio::spawn(async move {
            match ytdlp::download_with_retries(&url, &audio_dir, &platform_id, &id, &quality, dl_tx)
                .await
            {
                Ok(file) => {
                    let _ = task_tx.send(TaskMsg::DownloadDone { id, file });
                }
                Err(e) => {
                    let _ = task_tx.send(TaskMsg::DownloadError {
                        id,
                        err: e.to_string(),
                    });
                }
            }
        });
    }

    /// Force a fresh download of the track at `idx` in the displayed playlist,
    /// regardless of its current `cache_status` — `cached` (overwrites the
    /// existing file), `streaming`, or `failed` all go through the same path.
    /// A no-op, with a status message, if a download for it is already running —
    /// or if the track is the user's own file, which has no remote to fetch it
    /// from and whose bytes are not ours to overwrite.
    pub fn recache_track(&mut self, id: &str) {
        let Some(track) = self.library.get(id) else {
            return;
        };
        let id = track.id.clone();
        if track.origin == TrackOrigin::Local {
            self.set_status("Local file, nothing to download");
            return;
        }
        if self.downloading.contains(&id) {
            self.set_status("Already downloading");
            return;
        }
        let url = track.url.clone();
        let title = track.title.clone();

        self.patch_track(&id, |t| {
            t.cache_status = CacheStatus::Downloading;
        });
        self.start_download(id, url);
        self.set_status(format!("Recaching: {title}"));
    }

    /// Forget every trace of an in-flight download for `id`.
    ///
    /// Called when the row the download was going to fill disappears (track
    /// deleted, or its whole playlist deleted). Without it the `⟳` spinner and
    /// `is_downloading()` stay stuck forever on a track that no longer exists.
    ///
    /// The yt-dlp process itself is not cancelled — its handle is not retained —
    /// so `DownloadDone` can still arrive afterwards. `patch_track` then finds no
    /// such document and logs a warning, which is the intended no-op.
    pub fn clear_download_state(&mut self, id: &str) {
        self.downloading.remove(id);
        self.download_progress.remove(id);
    }

    /// True when the cached audio for `platform_id` is still referenced by some
    /// playlist other than the displayed one — or by a duplicate row within it —
    /// and so must not be deleted.
    ///
    /// Scoped to the *platform* id, not the library id, because that is what the
    /// audio cache is keyed by: one `<platform-id>.opus` backs the track in every
    /// playlist holding it. Deleting a track used to unlink that file
    /// unconditionally, silently downgrading every other playlist's copy to
    /// `streaming`.
    ///
    /// Deliberately answers "yes" whenever a playlist cannot be read: a stray
    /// cached file costs disk, whereas another playlist's deleted audio costs a
    /// re-download.
    ///
    /// The lists held in memory — the displayed playlist and its loaded albums —
    /// are consulted from memory and their files skipped on disk. They carry edits
    /// that have not been written yet, and reading the row just removed back off
    /// disk would always answer "still referenced" and leak the document.
    pub fn platform_id_referenced_elsewhere(&self, platform_id: &str) -> bool {
        let lists = |ids: &[String]| {
            ids.iter()
                .any(|id| library::platform_id_of(id) == platform_id)
        };
        if lists(&self.playlist.tracks) {
            return true;
        }
        if self
            .albums
            .iter()
            .any(|loaded| lists(&loaded.playlist.tracks))
        {
            return true;
        }
        let in_memory = |path: &Path| {
            path == self.playlist_path || self.albums.iter().any(|loaded| loaded.path == path)
        };
        self.available_playlists
            .iter()
            .filter(|entry| !in_memory(&entry.path))
            .any(|entry| match Playlist::load(&entry.path) {
                Ok(pl) => lists(&pl.tracks),
                Err(e) => {
                    warn!(err = %e, path = %entry.path.display(), "could not check playlist for shared cache file; keeping it");
                    true
                }
            })
    }

    /// Tear down the current player and invalidate everything still working on
    /// its behalf, returning the new generation.
    ///
    /// Dropping the `Player` kills mpv. Bumping the generation additionally
    /// retires the position poller watching the old socket and any in-flight
    /// spawn task, so neither can write into `App` after this point. Callers
    /// that are about to start a *different* player just call
    /// `spawn_player_for`, which does this for them.
    pub fn stop_player(&mut self) -> u64 {
        self.player = None;
        self.player_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    // ── Traversal order (shuffle) ─────────────────────────────────────────

    /// Throw away the current shuffle order and build a fresh one for the
    /// displayed playlist — or none at all if shuffle is off.
    ///
    /// Called when shuffle is toggled. Rebuilding on *both* edges means
    /// toggling off and on again gives a new walk rather than resuming the old
    /// one, which is what "shuffle again" is expected to do.
    pub fn rebuild_shuffle_order(&mut self) {
        let path = self.playlist_path.clone();
        let shuffle = self.playlist.shuffle;
        let len = self.playlist.tracks.len();
        self.rebuild_shuffle_order_for(&path, shuffle, len);
    }

    /// `rebuild_shuffle_order`, generalized to any list rather than always the
    /// displayed playlist — what lets toggling shuffle on an album's track
    /// start a fresh walk too, instead of `ensure_shuffle_order`'s lazy check
    /// (same path, same length) quietly resuming whatever permutation this
    /// cache already held for it.
    pub fn rebuild_shuffle_order_for(&mut self, path: &Path, shuffle: bool, len: usize) {
        if !shuffle {
            self.shuffle_order.clear();
            self.shuffle_order_path = None;
            return;
        }
        self.shuffle_order = playlist::shuffled_indices(len, playlist::shuffle_seed());
        self.shuffle_order_path = Some(path.to_path_buf());
    }

    /// Make sure `shuffle_order` is a usable permutation of `0..len` for the
    /// playlist at `path`, building one if it is missing or was built for a
    /// different playlist or a different track count (a track was added or
    /// deleted, so the old order no longer covers it).
    fn ensure_shuffle_order(&mut self, path: &Path, len: usize) {
        let stale =
            self.shuffle_order_path.as_deref() != Some(path) || self.shuffle_order.len() != len;
        if !stale {
            return;
        }
        self.shuffle_order = playlist::shuffled_indices(len, playlist::shuffle_seed());
        self.shuffle_order_path = Some(path.to_path_buf());
    }

    /// The track index that comes after (or before) `from` in the playlist at
    /// `path`, following the shuffled order when `shuffle` is set and the plain
    /// index order otherwise. Wraps at both ends; `None` only for an empty
    /// playlist.
    ///
    /// Whether wrapping is *wanted* is the caller's business — `n`/`b` always
    /// wrap, auto-advance consults `loop_mode` (see `next_after_end`).
    pub fn step_index(
        &mut self,
        path: &Path,
        len: usize,
        shuffle: bool,
        from: usize,
        forward: bool,
    ) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let step = |pos: usize| {
            if forward {
                (pos + 1) % len
            } else {
                pos.checked_sub(1).unwrap_or(len - 1)
            }
        };

        if !shuffle {
            return Some(step(from.min(len - 1)));
        }

        self.ensure_shuffle_order(path, len);
        // `from` outside the order means the caller is stepping from a track
        // this order does not describe; plain index order is the safe answer.
        let Some(pos) = self.shuffle_order.iter().position(|&i| i == from) else {
            return Some(step(from.min(len - 1)));
        };
        self.shuffle_order.get(step(pos)).copied()
    }

    /// The track to play once the current one has finished, per `loop_mode`.
    /// `None` means "stop here".
    fn next_after_end(
        &mut self,
        path: &Path,
        len: usize,
        shuffle: bool,
        from: usize,
        loop_mode: &LoopMode,
    ) -> Option<usize> {
        match loop_mode {
            LoopMode::Track => Some(from),
            LoopMode::Playlist => self.step_index(path, len, shuffle, from, true),
            LoopMode::None => {
                // Play through and stop at the end — "none" turns *looping* off,
                // not advancing. The end is the end of the shuffled walk when
                // shuffle is on, which is not the last track by index.
                let next = self.step_index(path, len, shuffle, from, true)?;
                let wrapped = if shuffle {
                    self.shuffle_order.first().copied() == Some(next)
                } else {
                    next == 0
                };
                if wrapped {
                    None
                } else {
                    Some(next)
                }
            }
        }
    }

    /// Whether the playing track had effectively reached its end at the point
    /// mpv exited, and so whether that exit should be read as "track finished"
    /// rather than "player died".
    ///
    /// A track whose duration is unknown (yt-dlp reported none) counts as
    /// finished: there is nothing to compare against, and refusing to advance
    /// would make auto-advance silently not work for those tracks.
    fn reached_end_of_track(&self) -> bool {
        let Some(track) = self.playing_track() else {
            return false;
        };
        track.duration == 0 || self.position + EOF_SLACK_SECS >= track.duration as f64
    }

    /// The playing track reached its end. Rewind its resume point, then advance
    /// per the **playing** playlist's `loop_mode` and `shuffle` — never the
    /// displayed playlist's, since the two can be different files entirely.
    pub fn handle_track_ended(&mut self) {
        let Some(session) = self.playing.as_ref() else {
            self.set_status("Playback finished");
            return;
        };
        // The row can have been deleted out from under playback while it played;
        // there is then no "next after it" to speak of.
        let Some(from) = session.track_index() else {
            self.set_status("Playback finished");
            return;
        };
        let path = session.path.clone();
        let len = session.playlist.tracks.len();
        let loop_mode = session.playlist.loop_mode.clone();
        let shuffle = session.playlist.shuffle;

        // A track that ran to its end resumes from the start, not the end.
        // Leaving `last_position` at the end would make a later replay open on
        // top of EOF — and now that finishing a track advances, skip past it.
        if let Some(track) = self.playing_track_mut() {
            track.last_position = 0;
        }
        self.position = 0.0;
        let _ = self.pos_tx.send(0.0);
        self.save_playing_track();

        match self.next_after_end(&path, len, shuffle, from, &loop_mode) {
            Some(next) => self.play_session_track(next),
            None => self.set_status("Playback finished"),
        }
    }

    /// Start playback of index `idx` within the playlist that is *already*
    /// driving playback, rather than the displayed one — auto-advance has to
    /// follow the playing playlist even while the user browses another.
    fn play_session_track(&mut self, idx: usize) {
        let Some(session) = self.playing.as_ref() else {
            return;
        };

        // A file that is on screen — the displayed playlist or one of its albums
        // — goes through the normal path, so that copy, its `current_track` and
        // the cursor all stay in step.
        if let Some(source) = self.source_of_path(&session.path.clone()) {
            let start_pos = self
                .source_playlist(source)
                .and_then(|(playlist, _)| playlist.tracks.get(idx))
                .and_then(|id| self.library.get(id))
                .and_then(input::resume_start_pos);
            self.play_from_list(source, idx, start_pos);
            return;
        }

        let Some(id) = session.playlist.tracks.get(idx).cloned() else {
            return;
        };
        let Some(track) = self.library.get(&id) else {
            warn!(id = %id, "next track's document is missing, stopping here");
            self.set_status("Track document missing");
            return;
        };
        let start_pos = input::resume_start_pos(track);
        let speed = track
            .speed
            .or(session.playlist.default_speed)
            .unwrap_or(self.config.default_speed);
        let source = play_source_for(track);
        let media = track.media;

        if let Some(session) = self.playing.as_mut() {
            session.track_id = id.clone();
            session.playlist.current_track = Some(id.clone());
        }
        self.is_paused = false;
        self.position = start_pos.unwrap_or(0.0);
        let _ = self.pos_tx.send(self.position);
        // The session's playlist file records which track was last played out of
        // it, so the cursor lands there when the user next opens it.
        if let Some(session) = self.playing.as_ref() {
            let path = session.path.clone();
            if let Err(e) = session.playlist.save(&path) {
                error!(err = %e, path = %path.display(), "failed to save the playing session's playlist");
            }
        }
        self.spawn_player_for(id, source, speed, start_pos, media);
    }

    /// Play the row the cursor is on, whichever list it belongs to, resuming from
    /// the track's `last_position` when it has one. Returns whether there was
    /// anything to play: a header names a group, not a track.
    ///
    /// The one door `Enter`, `Space` and `n`/`b` go through, so all three agree
    /// about what a row means.
    pub fn play_row(&mut self, cursor: usize) -> bool {
        let Some(&VisibleRow::Track { source, index }) = self.row_at(cursor) else {
            return false;
        };
        let start_pos = self
            .source_playlist(source)
            .and_then(|(playlist, _)| playlist.tracks.get(index))
            .and_then(|id| self.library.get(id))
            .and_then(input::resume_start_pos);
        self.play_from_list(source, index, start_pos);
        true
    }

    /// Start playback of index `idx` within the list `source` names — the
    /// displayed playlist or one of the albums under it.
    ///
    /// An album plays as its own list: the session it builds carries the album's
    /// path and its running order, so `n`/`b`, `loop_mode`, `shuffle` and
    /// auto-advance all stay inside the album (ADR-019).
    pub fn play_from_list(&mut self, source: RowSource, idx: usize, start_pos: Option<f64>) {
        // Collect all track data before any mutations (borrow checker)
        let (list_path, id, speed, source_media, media) = {
            let Some((playlist, path)) = self.source_playlist(source) else {
                return;
            };
            let (path, default_speed) = (path.to_path_buf(), playlist.default_speed);
            let Some(id) = playlist.tracks.get(idx).cloned() else {
                return;
            };
            // A row whose document has gone missing cannot be played, and must
            // not silently start something else.
            let Some(track) = self.library.get(&id) else {
                warn!(id = %id, "row references a track with no document");
                self.set_status("Track document missing");
                return;
            };
            let speed = track
                .speed
                .or(default_speed)
                .unwrap_or(self.config.default_speed);
            // A remote track always has a stream to fall back on. A local one has
            // only the file, so if that has moved or its drive is unplugged there
            // is nothing to play — and handing mpv a path to nothing looks like a
            // hang rather than an answer.
            let absent_local =
                track.origin == TrackOrigin::Local && !local_media_path(track).exists();
            if absent_local {
                warn!(id = %id, "local media is not where the row says it is");
                self.patch_track(&id, |t| t.cache_status = CacheStatus::Missing);
                self.set_status("File not found");
                return;
            }
            (path, id, speed, play_source_for(track), track.media)
        };

        // Save position of the track we're leaving (not applicable when switching
        // within the same track, e.g. stream → local file). The leaving track may
        // live in the displayed playlist or in a different one entirely (the user
        // was browsing elsewhere while it played) — route the write through
        // whichever copy is the source of truth for that track's identity.
        if let Some(session) = self.playing.as_ref() {
            // Identity is `(path, id)`, not `id` alone: the same
            // track can sit in two playlists, and starting playlist B's copy
            // while playlist A's copy plays *is* leaving a track, so its
            // position still has to be written. Comparing ids alone silently
            // dropped that position.
            let leaving = (session.path.clone(), session.track_id.clone());
            if leaving != (list_path.clone(), id.clone()) {
                let pos = self.position as u64;
                if let Some(t) = self.playing_track_mut() {
                    t.last_position = pos;
                }
                // Persist the mutation above so the position update isn't
                // silently dropped when `self.playing` is replaced below.
                self.save_playing_track();
            }
        }

        // `current_track` means strictly "last track selected/played in *this*
        // playlist file" — used only to restore the cursor on load. Record it on
        // the list the row actually came out of.
        //
        // An album's file is written straight away: it lives in memory here and
        // nothing else would ever save it, so the fold-and-restart case would lose
        // the cursor. The displayed playlist is saved by the ordinary paths.
        match source {
            RowSource::Own => self.playlist.current_track = Some(id.clone()),
            RowSource::Album(album) => {
                if let Some(loaded) = self.albums.get_mut(album) {
                    loaded.playlist.current_track = Some(id.clone());
                    if let Err(e) = loaded.playlist.save(&loaded.path) {
                        error!(err = %e, path = %loaded.path.display(), "failed to save an album's current track");
                    }
                }
            }
        }
        let Some((snapshot, _)) = self.source_playlist(source) else {
            return;
        };
        self.playing = Some(PlayingSession {
            path: list_path,
            playlist: snapshot.clone(),
            track_id: id.clone(),
        });
        self.is_paused = false;
        // Set the position to wherever this player is actually about to start.
        //
        // This used to be skipped whenever `start_pos` was `Some`, which is the
        // case for every track that carries a `last_position`. `App::position`
        // then still held the *outgoing* track's timestamp — and when the new
        // track's download completed, `hot_switch_to_local_file` respawned mpv
        // with `--start=<that stale value>`, so the new track jumped to where the
        // previous one left off. Writing it into the watch channel too means a
        // position still queued from the retired poller cannot overwrite it.
        self.position = start_pos.unwrap_or(0.0);
        let _ = self.pos_tx.send(self.position);

        self.spawn_player_for(id, source_media, speed, start_pos, media);
    }

    /// If `id` is the track actually driving playback right now (per
    /// `self.playing`, independent of what's displayed) and a player is running,
    /// spawn a fresh mpv against the freshly downloaded local `file`, resuming at
    /// the current live position. This is the stream→local-file hot-switch
    /// triggered by `TaskMsg::DownloadDone`.
    ///
    /// The id alone is identity now: the download filled the one document that
    /// track has, so it makes no difference which playlist file playback is
    /// running out of.
    fn hot_switch_to_local_file(&mut self, id: &str, file: PathBuf) {
        let playing_this = self.playing.as_ref().is_some_and(|p| p.track_id == id);
        if !playing_this || self.player.is_none() {
            return;
        }

        let (speed, media) = {
            // Fall back to the *playing* playlist's default speed, not the
            // displayed one — they may differ once playback is decoupled
            // from the displayed playlist.
            let playing_playlist = self
                .playing_playlist()
                .expect("just verified a playing track exists");
            let track = self
                .playing_track()
                .expect("just verified a playing track exists");
            (
                effective_speed(track, playing_playlist, &self.config),
                track.media,
            )
        };
        let pos = self.position;
        info!(id = %id, pos = pos, "switching stream → local file");
        self.is_paused = false;
        self.spawn_player_for(
            id.to_string(),
            PlaySource::File(file),
            speed,
            Some(pos),
            media,
        );
    }

    /// Resolve the stream/local-file source and spawn mpv, wiring up position
    /// polling and reporting the result back via `TaskMsg::PlayerReady`/
    /// `PlayerError`. Pure "start a player" — callers are responsible for any
    /// `self.playing`/`current_track`/`position` bookkeeping beforehand.
    ///
    /// Always stops the previous player first (via `stop_player`), so no two mpv
    /// processes are ever audible at once and the outgoing player's poller is
    /// retired before the new one starts reporting positions.
    fn spawn_player_for(
        &mut self,
        id: String,
        source: PlaySource,
        speed: f32,
        start_pos: Option<f64>,
        media: MediaKind,
    ) {
        let generation = self.stop_player();
        let volume = self.config.default_volume;
        let quality = self.config.audio_quality.clone();
        // A window, and the user's own flags for it, only for something to show.
        let video = media == MediaKind::Video;
        let video_args = self.config.video_mpv_args.clone();
        let task_tx = self.task_tx.clone();
        let pos_tx = self.pos_tx.clone();
        let player_generation = Arc::clone(&self.player_generation);

        tokio::spawn(async move {
            let resolved_source = match source {
                PlaySource::File(path) => path.to_string_lossy().into_owned(),
                PlaySource::Stream(url) => match ytdlp::get_stream_url(&url, &quality).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = task_tx.send(TaskMsg::PlayerError {
                            id,
                            err: e.to_string(),
                        });
                        return;
                    }
                },
            };

            // Resolving the stream URL above can take seconds; bail out rather
            // than spawn an mpv nobody asked for any more.
            if player_generation.load(Ordering::SeqCst) != generation {
                info!(id = %id, "playback request superseded before spawn");
                return;
            }

            match Player::spawn(&resolved_source, start_pos, video, &video_args).await {
                Ok(player) => {
                    let _ = player.set_speed(speed).await;
                    let _ = player.set_volume(volume).await;
                    // Start position polling as independent task. It reports
                    // back when mpv exits on its own so the app can drop the
                    // dead `Player` instead of keeping a stale one around.
                    let socket_path = player.socket_path.clone();
                    let poll_task_tx = task_tx.clone();
                    let poll_generation = Arc::clone(&player_generation);
                    tokio::spawn(async move {
                        let mpv_exited = player::poll_position_loop(
                            socket_path,
                            pos_tx,
                            generation,
                            poll_generation,
                        )
                        .await;
                        if mpv_exited {
                            let _ = poll_task_tx.send(TaskMsg::PlayerGone { generation });
                        }
                    });
                    let _ = task_tx.send(TaskMsg::PlayerReady {
                        id,
                        player: Box::new(player),
                        generation,
                    });
                }
                Err(e) => {
                    let _ = task_tx.send(TaskMsg::PlayerError {
                        id,
                        err: e.to_string(),
                    });
                }
            }
        });
    }

    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();
        items.push(SidebarItem::PlaylistsHeader);
        if self.playlists_expanded {
            for entry in playlist::sidebar_entries(&self.available_playlists) {
                items.push(SidebarItem::Playlist {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    is_album: entry.kind == PlaylistKind::Album,
                });
            }
        }
        items.push(SidebarItem::Separator);
        items.push(SidebarItem::Music);
        items.push(SidebarItem::Video);
        items.push(SidebarItem::Separator);
        items.push(SidebarItem::Plunder);
        items.push(SidebarItem::ImportFolder);
        items.push(SidebarItem::Settings);
        items
    }

    /// Where playlist files live, taken from the displayed playlist's own path so
    /// an import writes its album beside the list it belongs to.
    pub fn playlists_dir(&self) -> PathBuf {
        self.playlist_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(cache::playlists_dir)
    }

    /// The name that addresses the displayed playlist — its file stem, which is
    /// what an album's `parent` names and what the sidebar shows.
    pub fn displayed_playlist_name(&self) -> String {
        self.playlist_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.playlist.name.clone())
    }

    /// Which playlist file the contents of `root` belong in.
    ///
    /// A folder some album already mirrors is rescanned rather than imported
    /// again — otherwise pointing at the same folder twice leaves an `Ultra (2)`
    /// holding a second copy of every row.
    pub fn import_target_for(&self, root: &Path) -> ImportTarget {
        let normalize = |p: &Path| -> PathBuf { p.components().collect() };
        let wanted = normalize(root);
        let linked_to_root = |folder: Option<&Path>| folder.map(normalize) == Some(wanted.clone());

        if linked_to_root(self.playlist.source_folder.as_deref()) {
            return ImportTarget::Existing(self.playlist_path.clone());
        }
        // The displayed list's own albums are loaded, so they answer from memory
        // too — and a rescan started from one of their headers is exactly this
        // case.
        for loaded in &self.albums {
            if linked_to_root(loaded.playlist.source_folder.as_deref()) {
                return ImportTarget::Existing(loaded.path.clone());
            }
        }
        for entry in &self.available_playlists {
            // Anything already in memory is answered for above: it may hold
            // unsaved edits, and re-reading it would miss them.
            if entry.path == self.playlist_path
                || entry.kind != PlaylistKind::Album
                || self.albums.iter().any(|loaded| loaded.path == entry.path)
            {
                continue;
            }
            // Only the link is wanted, so a list that will not load is simply not
            // a match — the import falls back to making a new album.
            let folder = match Playlist::load(&entry.path) {
                Ok(pl) => pl.source_folder,
                Err(e) => {
                    warn!(err = %e, path = %entry.path.display(), "could not read a playlist while looking for a linked folder");
                    continue;
                }
            };
            if linked_to_root(folder.as_deref()) {
                return ImportTarget::Existing(entry.path.clone());
            }
        }

        ImportTarget::NewAlbum {
            parent: self.default_album_parent(),
        }
    }

    /// The parent a *new* album gets when nothing else decides it: the
    /// displayed playlist, or — since nesting stays two levels deep — that
    /// playlist's own parent when an album is itself displayed, so importing
    /// or creating one while browsing an album makes a sibling under the same
    /// parent rather than an album inside an album.
    fn default_album_parent(&self) -> Option<String> {
        if self.playlist.kind == PlaylistKind::Album {
            self.playlist.parent.clone()
        } else {
            Some(self.displayed_playlist_name())
        }
    }

    /// Create a new, empty album named `name` under `parent`, save it, and —
    /// when `parent` is the displayed playlist — join it to `self.albums` so
    /// it appears in the track list immediately rather than only on disk.
    ///
    /// `parent` is taken as given rather than recomputed from
    /// `self.playlist`, because one caller (`apply_import`) must use the
    /// parent it decided when the scan *started*, not whatever happens to be
    /// displayed by the time a slow scan finishes; callers with no such gap
    /// (manual creation) just pass `self.default_album_parent()`.
    pub fn create_album(&mut self, name: &str, parent: Option<String>) -> Result<PathBuf> {
        let path = self.playlists_dir().join(format!("{name}.toml"));

        let mut album = Playlist::empty(name);
        album.kind = PlaylistKind::Album;
        album.parent = parent.clone();
        // Stored open: an album the user just asked for should be visibly
        // there, not folded away behind one row.
        album.collapsed = false;
        album.save(&path)?;

        self.available_playlists.push(PlaylistEntry {
            name: name.to_string(),
            path: path.clone(),
            kind: PlaylistKind::Album,
            parent: parent.clone(),
        });
        self.available_playlists.sort_by(|a, b| a.name.cmp(&b.name));
        // A new album under the playlist on screen is a new group in it, so
        // it has to join the loaded ones rather than only exist on disk.
        if parent.as_deref() == Some(&*self.displayed_playlist_name()) {
            self.albums.push(LoadedAlbum {
                name: name.to_string(),
                path: path.clone(),
                playlist: album,
            });
            self.albums.sort_by(|a, b| a.name.cmp(&b.name));
            self.rebuild_rows();
        }
        Ok(path)
    }

    /// Scan `root` in the background, ending in a `TaskMsg::ImportScanned`.
    ///
    /// The target is settled here rather than on arrival so the album an import
    /// lands in is the one that was on screen when the user asked for it.
    ///
    /// `target_override`, when given, bypasses `import_target_for`'s
    /// folder-identity matching entirely and merges straight into the list at
    /// that path — what lets the user Tab to an explicit destination in the
    /// folder prompt instead of only ever landing on whichever list already
    /// recognizes this folder, or a new sibling album.
    pub fn import_folder(&mut self, root: PathBuf, target_override: Option<PathBuf>) {
        if !root.is_dir() {
            self.set_status(format!("Not a folder: {}", root.display()));
            return;
        }

        let target = match target_override {
            Some(path) => ImportTarget::Existing(path),
            None => self.import_target_for(&root),
        };
        self.set_status(format!("Scanning {}", root.display()));

        let task_tx = self.task_tx.clone();
        tokio::spawn(async move {
            let progress_tx = task_tx.clone();
            let files = library_import::scan_and_probe(&root, move |done, total| {
                let _ = progress_tx.send(TaskMsg::ImportProgress { done, total });
            })
            .await;
            let _ = task_tx.send(TaskMsg::ImportScanned {
                root,
                target,
                files,
            });
        });
    }

    /// Probe a single local file in the background, ending in a
    /// `TaskMsg::FileProbed` that folds it into the list at `target_path`.
    ///
    /// Not a one-file call to `import_folder`: there is no folder to scan, no
    /// `source_folder` to stamp, and no "files that vanished" bookkeeping to
    /// do — see `library_import::add_single_file`.
    pub fn import_file(&mut self, path: PathBuf, target_path: PathBuf) {
        if !path.is_file() {
            self.set_status(format!("Not a file: {}", path.display()));
            return;
        }
        let Some(media) = library_scan::media_kind_for_path(&path) else {
            self.set_status(format!("Not a media file: {}", path.display()));
            return;
        };
        self.set_status(format!("Adding {}", path.display()));

        let task_tx = self.task_tx.clone();
        tokio::spawn(async move {
            let scanned = library_scan::ScannedFile {
                path: path.clone(),
                media,
            };
            let meta = library_scan::probe(&scanned).await;
            let _ = task_tx.send(TaskMsg::FileProbed {
                file: library_import::ImportedFile { path, meta },
                target_path,
            });
        });
    }

    /// Fold a finished scan into the library and the album it belongs to.
    ///
    /// The merge rules — nothing deleted, nothing reordered, documents reused —
    /// live in `library_import::merge_scan`; this decides only which playlist file
    /// is merged into and keeps the sidebar and the rows on screen in step.
    pub fn apply_import(
        &mut self,
        root: PathBuf,
        target: ImportTarget,
        files: Vec<library_import::ImportedFile>,
    ) {
        if files.is_empty() {
            // Deliberately before any file is written: an empty album is worse
            // than none, and this is most often a mistyped path.
            self.set_status(format!("No playable files in {}", root.display()));
            return;
        }

        match target {
            // Merging goes through `with_list_at`, whichever of the three
            // places the target list lives in: the displayed playlist, one of
            // its loaded albums, or a file nothing currently holds in memory.
            ImportTarget::Existing(path) => {
                let is_own = path == self.playlist_path;
                let result = self.with_list_at(&path, |pl, lib| {
                    let report = library_import::merge_scan(lib, pl, &root, files);
                    (pl.name.clone(), report)
                });
                let (name, report) = match result {
                    Ok(pair) => pair,
                    Err(e) => {
                        error!(err = %e, path = %path.display(), "cannot rescan a playlist that will not load or save");
                        self.set_status("Could not read or save the album");
                        return;
                    }
                };
                // New rows mean the shuffled order no longer covers the list —
                // only relevant when the merge landed in the displayed
                // playlist itself, since that is the only list whose shuffle
                // order `App` caches.
                if is_own {
                    self.rebuild_shuffle_order();
                }
                self.report_import(&name, report);
            }
            ImportTarget::NewAlbum { parent } => {
                let taken: Vec<String> = self
                    .available_playlists
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect();
                let name = library_import::unique_album_name(
                    &library_import::album_name_for_folder(&root),
                    &taken,
                );
                let path = match self.create_album(&name, parent) {
                    Ok(path) => path,
                    Err(e) => {
                        error!(err = %e, "failed to create a new album for import");
                        self.set_status("Could not create the album");
                        return;
                    }
                };
                // `create_album` already joined the empty album to
                // `self.albums` when its parent is the displayed playlist, so
                // the merge goes through `with_list_at` like any other target.
                let merge_result = self.with_list_at(&path, |pl, lib| {
                    library_import::merge_scan(lib, pl, &root, files)
                });
                match merge_result {
                    Ok(report) => self.report_import(&name, report),
                    Err(e) => {
                        error!(err = %e, path = %path.display(), "failed to save an imported album");
                        self.set_status("Could not save the album");
                    }
                }
            }
        }
    }

    /// What an import did, in the footer. `missing` is worth saying out loud: it
    /// is the only sign that a file the user still has a row for has moved.
    fn report_import(&mut self, name: &str, report: library_import::ImportReport) {
        info!(
            album = %name,
            added = report.added,
            updated = report.updated,
            missing = report.missing,
            "folder import merged"
        );
        self.set_status(format!(
            "{name} · {} added · {} updated · {} missing",
            report.added, report.updated, report.missing
        ));
    }

    pub fn sync_channels(&mut self) {
        if self.position_rx.has_changed().unwrap_or(false) {
            self.position = *self.position_rx.borrow_and_update();
        }
        if self.download_rx.has_changed().unwrap_or(false) {
            let (id, pct) = self.download_rx.borrow_and_update().clone();
            self.download_progress.insert(id, pct);
        }
        while let Ok(msg) = self.task_rx.try_recv() {
            self.handle_task_msg(msg);
        }
    }

    /// Set a transient status message for the footer.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    pub(crate) fn handle_task_msg(&mut self, msg: TaskMsg) {
        match msg {
            TaskMsg::MetaReady {
                url,
                meta,
                target_path,
            } => {
                self.pending_fetches = self.pending_fetches.saturating_sub(1);
                // This is the one place a remote track's library id is minted:
                // the source domain's slug plus the platform's own id.
                let id = library::make_id(&meta.source, &meta.video_id);
                info!(id = %id, title = %meta.title, "metadata ready, starting download");
                let status_title = meta.title.clone();

                // Which playlist the row goes into. The track's own document is
                // global; only this list membership is per-playlist.
                let owning_path = target_path.unwrap_or_else(|| self.playlist_path.clone());

                // A read-only check, so nothing is written and no download is
                // started if the row cannot be recorded anywhere: a download
                // completing against a playlist that has no such row leaves an
                // untracked file in the audio cache. Reads whichever in-memory
                // copy is loaded (the displayed playlist or one of its albums)
                // rather than the disk, so a target that already has unsaved
                // edits is checked accurately.
                //
                // Adding the same URL twice used to produce a second row
                // sharing the first one's id, and so its cached file too — two
                // rows whose download, cache status and deletion all fight
                // over one file.
                let already_present = match self.source_of_path(&owning_path) {
                    Some(source) => self
                        .source_playlist(source)
                        .is_some_and(|(pl, _)| pl.tracks.contains(&id)),
                    None => match Playlist::load(&owning_path) {
                        Ok(pl) => pl.tracks.contains(&id),
                        Err(e) => {
                            error!(err = %e, path = %owning_path.display(), "target playlist not found, track not added");
                            self.set_status("Target playlist not found");
                            return;
                        }
                    },
                };
                if already_present {
                    info!(id = %id, path = %owning_path.display(), "track already in target playlist, not adding again");
                    self.set_status(format!("Already in playlist: {status_title}"));
                    return;
                }

                // The download starts unconditionally below, so record that in the
                // document about to be written. A track already in the library
                // keeps everything else it has — its position, speed and any
                // renamed title survive being added to another playlist.
                if self.library.get(&id).is_some() {
                    self.patch_track(&id, |t| t.cache_status = CacheStatus::Downloading);
                } else {
                    let track = Track {
                        url: url.clone(),
                        source: meta.source,
                        title: meta.title,
                        artist: meta.artist,
                        channel: meta.channel,
                        duration: meta.duration,
                        id: id.clone(),
                        cache_status: CacheStatus::Downloading,
                        file: None,
                        last_position: 0,
                        speed: None,
                        user_title: None,
                        user_artist: None,
                        added_at: Utc::now(),
                        origin: TrackOrigin::Remote,
                        media: MediaKind::Audio,
                        resume: true,
                    };
                    if let Err(e) = self.library.upsert(track) {
                        error!(err = %e, id = %id, "failed to write the track document");
                        self.set_status("Could not save the track");
                        return;
                    }
                }

                // The cursor deliberately stays where the user left it. It used
                // to jump to the new row, which meant adding a track while
                // browsing moved the selection out from under `Enter` and `d`
                // — and with a search filter active it jumped to a row index
                // that the filter does not even display. `with_list_at` routes
                // to whichever list is at `owning_path` — the displayed
                // playlist, one of its loaded albums, or a file on disk — and
                // keeps that list's in-memory copy (and `self.rows`) in step.
                let displayed = owning_path == self.playlist_path;
                let added_id = id.clone();
                if let Err(e) = self.with_list_at(&owning_path, |pl, _lib| pl.add_track(added_id)) {
                    error!(err = %e, path = %owning_path.display(), "failed to save target playlist after URL add");
                    self.set_status("Could not save to target playlist");
                    return;
                }
                self.set_status(if displayed {
                    format!("Added: {status_title}")
                } else {
                    format!("Added to playlist: {status_title}")
                });
                self.start_download(id, url);
            }

            TaskMsg::MetaError { url, err } => {
                self.pending_fetches = self.pending_fetches.saturating_sub(1);
                error!(url = %url, err = %err, "metadata fetch failed");
                self.set_status("Metadata fetch failed");
            }

            TaskMsg::DownloadDone { id, file } => {
                info!(id = %id, path = %file.display(), "download complete");
                self.downloading.remove(&id);
                self.download_progress.remove(&id);
                self.set_status("Download complete");

                // One document, so one write — every playlist listing this track
                // sees the new cache status, whichever one the user is browsing.
                let file_for_patch = file.clone();
                self.patch_track(&id, move |track| {
                    track.cache_status = CacheStatus::Cached;
                    track.file = Some(file_for_patch);
                });

                // If this track is the one actually driving playback right now — per
                // `self.playing`, independent of whatever playlist is displayed — and
                // it was streaming, hot-switch mpv to the freshly downloaded local file.
                self.hot_switch_to_local_file(&id, file);
            }

            TaskMsg::DownloadError { id, err } => {
                error!(id = %id, err = %err, "download failed after all retries");
                // Roll the row back off `downloading`, otherwise it keeps
                // claiming a download is in progress until the next
                // `Playlist::load` happens to reset it.
                //
                // `Failed`, not `Streaming`: streaming still works fine, but the
                // track was tried and given up on, which is worth surfacing —
                // unlike a track nobody has ever attempted to cache. Recoverable
                // with the recache key (`c`), which does not care what state it
                // finds the row in.
                self.patch_track(&id, |track| {
                    track.cache_status = CacheStatus::Failed;
                });
                self.clear_download_state(&id);
                self.set_status(match ytdlp::blocked_by_youtube_hint(&err) {
                    Some(hint) => format!("Download failed — {hint}"),
                    None => "Download failed".to_string(),
                });
            }

            TaskMsg::PlayerReady {
                id,
                player,
                generation,
            } => {
                // Discard a player that finished starting after the user already
                // moved on. Comparing generations (rather than video ids) also
                // covers replaying the *same* track and the stream→local-file
                // hot switch, where the id alone cannot tell the two apart.
                // Dropping `player` here kills its mpv.
                if generation != self.player_generation.load(Ordering::SeqCst) {
                    info!(id = %id, generation, "player ready but superseded, discarding");
                    return;
                }
                info!(id = %id, "player started");
                self.player = Some(*player);
                self.is_paused = false;
                self.set_status("Player ready");
            }

            TaskMsg::PlayerError { id, err } => {
                error!(id = %id, err = %err, "player failed to start");
                self.set_status(match ytdlp::blocked_by_youtube_hint(&err) {
                    Some(hint) => format!("Player error — {hint}"),
                    None => "Player error".to_string(),
                });
            }

            TaskMsg::PlayerGone { generation } => {
                // mpv exits by itself at the end of a track (it runs without
                // --idle/--keep-open). Drop the dead `Player` so the UI stops
                // claiming it is playing and no keypress tries to talk to a
                // socket nobody is listening on. `self.playing` is deliberately
                // left in place: it still records which track was last playing,
                // which the footer and resume-on-replay rely on.
                if generation != self.player_generation.load(Ordering::SeqCst) {
                    return;
                }
                info!(generation, "mpv exited on its own");
                self.player = None;
                self.is_paused = false;
                if self.reached_end_of_track() {
                    self.handle_track_ended();
                } else {
                    // mpv died well short of the end: a broken stream, a codec
                    // it could not handle, an external kill. Advancing here
                    // would walk the whole playlist in seconds, respawning mpv
                    // and yt-dlp for every track on the way.
                    warn!(
                        position = self.position,
                        "mpv exited before the end of the track"
                    );
                    self.set_status("Playback stopped unexpectedly");
                }
            }

            TaskMsg::ImportProgress { done, total } => {
                self.set_status(format!("Scanning {done}/{total}"));
            }

            TaskMsg::ImportScanned {
                root,
                target,
                files,
            } => {
                self.apply_import(root, target, files);
            }

            TaskMsg::FileProbed { file, target_path } => {
                let title = file.meta.title.clone();
                let result = self.with_list_at(&target_path, |pl, lib| {
                    library_import::add_single_file(lib, pl, file)
                });
                match result {
                    Ok(library_import::SingleAddOutcome::Added) => {
                        self.set_status(format!("Added: {title}"));
                    }
                    Ok(library_import::SingleAddOutcome::AlreadyPresent) => {
                        self.set_status(format!("Already in playlist: {title}"));
                    }
                    Err(e) => {
                        error!(err = %e, path = %target_path.display(), "failed to add a file to the target playlist");
                        self.set_status("Could not add the file");
                    }
                }
            }
        }
    }

    pub fn clamp_scroll(&mut self) {
        let visible = self.track_list_height as usize;
        if visible == 0 {
            return;
        }
        if self.selected < self.track_offset {
            self.track_offset = self.selected;
        } else if self.selected >= self.track_offset + visible {
            self.track_offset = self.selected + 1 - visible;
        }
    }

    /// Load the albums that name the displayed playlist as their parent.
    ///
    /// Read from `available_playlists`, so it costs one small file per album
    /// rather than a directory scan. An album that will not parse is skipped with
    /// a warning: a broken file must not take the playlist down with it.
    pub fn load_albums(&mut self) {
        let parent = self.displayed_playlist_name();
        let mut albums = Vec::new();
        for entry in &self.available_playlists {
            if entry.kind != PlaylistKind::Album || entry.parent.as_deref() != Some(&*parent) {
                continue;
            }
            match Playlist::load(&entry.path) {
                Ok(playlist) => albums.push(LoadedAlbum {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    playlist,
                }),
                Err(e) => {
                    warn!(err = %e, path = %entry.path.display(), "skipping an album that will not load")
                }
            }
        }
        albums.sort_by(|a, b| a.name.cmp(&b.name));
        self.albums = albums;
    }

    /// Whether a search filter is narrowing the rows.
    pub fn has_filter(&self) -> bool {
        !self.search_query.is_empty()
    }

    /// Rebuild `rows` from the playlist, its albums, the filter and each album's
    /// fold state. The only writer of `rows`.
    ///
    /// The parent's own tracks come first, then the albums alphabetically — so the
    /// list the user built by hand stays where they left it and the folders they
    /// imported sit below it in a predictable order.
    pub fn rebuild_rows(&mut self) {
        let query = self.search_query.to_lowercase();
        // A row whose document is missing matches nothing: there is no title to
        // match against.
        let hit = |id: &String| {
            query.is_empty()
                || self
                    .library
                    .get(id)
                    .is_some_and(|track| track_matches(track, &query))
        };

        let mut rows = Vec::new();
        for (index, id) in self.playlist.tracks.iter().enumerate() {
            if hit(id) {
                rows.push(VisibleRow::Track {
                    source: RowSource::Own,
                    index,
                });
            }
        }

        for (album, loaded) in self.albums.iter().enumerate() {
            // A name match shows the whole album: the user asked for the album,
            // not for whichever of its tracks repeat its name in their titles.
            let by_name = !query.is_empty() && loaded.name.to_lowercase().contains(&query);
            let indices: Vec<usize> = loaded
                .playlist
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, id)| by_name || hit(id))
                .map(|(index, _)| index)
                .collect();
            // An empty album keeps its header — the album exists, and the row is
            // how the user reaches it. Under a filter it does not: nothing in it
            // was asked for.
            if self.has_filter() && indices.is_empty() {
                continue;
            }
            rows.push(VisibleRow::AlbumHeader { album });
            // A filter overrides the fold. Hits left folded away inside an album
            // would read as a search that missed them.
            if !loaded.playlist.collapsed || self.has_filter() {
                for index in indices {
                    rows.push(VisibleRow::Track {
                        source: RowSource::Album(album),
                        index,
                    });
                }
            }
        }
        self.rows = rows;
    }

    pub fn row_at(&self, cursor: usize) -> Option<&VisibleRow> {
        self.rows.get(cursor)
    }

    /// The list a row comes out of, and the file that list lives in.
    pub fn source_playlist(&self, source: RowSource) -> Option<(&Playlist, &Path)> {
        match source {
            RowSource::Own => Some((&self.playlist, self.playlist_path.as_path())),
            RowSource::Album(album) => self
                .albums
                .get(album)
                .map(|loaded| (&loaded.playlist, loaded.path.as_path())),
        }
    }

    /// The library id a row names, whichever list it comes from. `None` on a
    /// header, which names an album rather than a track.
    pub fn row_track_id(&self, cursor: usize) -> Option<String> {
        let &VisibleRow::Track { source, index } = self.row_at(cursor)? else {
            return None;
        };
        let (playlist, _) = self.source_playlist(source)?;
        playlist.tracks.get(index).cloned()
    }

    /// Which of the lists on screen lives at `path`, if either.
    ///
    /// What lets auto-advance route back through the in-memory copy when the
    /// playing list happens to be one the user is looking at.
    pub fn source_of_path(&self, path: &Path) -> Option<RowSource> {
        if path == self.playlist_path {
            return Some(RowSource::Own);
        }
        self.albums
            .iter()
            .position(|loaded| loaded.path == path)
            .map(RowSource::Album)
    }

    /// Run `f` against the `Playlist` at `path` (and, since almost every
    /// caller is folding a track into both a list and the library, the
    /// library alongside it) wherever that list currently lives —
    /// `self.playlist` if `path` is the displayed playlist, the matching
    /// `self.albums[i]` if it's one of the loaded albums, or a copy read fresh
    /// from disk otherwise — then persist whichever one it was and, for the
    /// first two cases, rebuild `self.rows` so the change is visible on screen
    /// immediately rather than only on disk.
    ///
    /// This is the one door every add/move call site should use to write into
    /// a list by path: writing straight to disk when the target happens to be
    /// the displayed playlist or one of its loaded albums leaves the in-memory
    /// copy stale until the next switch or restart (see `apply_import`, which
    /// this generalizes — it already had to solve this once).
    pub fn with_list_at<R>(
        &mut self,
        path: &Path,
        f: impl FnOnce(&mut Playlist, &mut Library) -> R,
    ) -> Result<R> {
        use anyhow::Context as _;

        match self.source_of_path(path) {
            Some(RowSource::Own) => {
                let result = f(&mut self.playlist, &mut self.library);
                self.save_playlist();
                self.rebuild_rows();
                Ok(result)
            }
            Some(RowSource::Album(album)) => {
                let result = f(&mut self.albums[album].playlist, &mut self.library);
                let loaded = &self.albums[album];
                // Best-effort, like every other in-place album mutation
                // (`toggle_album`, `remove_row`, `rename_album`): the row is
                // right in memory either way, and the next thing that saves
                // this album writes it again.
                if let Err(e) = loaded.playlist.save(&loaded.path) {
                    error!(err = %e, path = %loaded.path.display(), "failed to save an album via with_list_at");
                }
                self.rebuild_rows();
                Ok(result)
            }
            None => {
                let mut playlist = Playlist::load(path)
                    .with_context(|| format!("failed to load playlist at {}", path.display()))?;
                let result = f(&mut playlist, &mut self.library);
                playlist
                    .save(path)
                    .with_context(|| format!("failed to save playlist at {}", path.display()))?;
                Ok(result)
            }
        }
    }

    /// The list the row at `cursor` belongs to, and every cursor position showing
    /// a row of that same list. `None` on a header, which belongs to no list.
    ///
    /// This is the running order `n`/`b` steps: from an album's last track they
    /// wrap to its first rather than falling into the parent's tracks.
    pub fn row_group(&self, cursor: usize) -> Option<(RowSource, Vec<usize>)> {
        let &VisibleRow::Track { source, .. } = self.row_at(cursor)? else {
            return None;
        };
        let group = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, VisibleRow::Track { source: s, .. } if *s == source))
            .map(|(cursor, _)| cursor)
            .collect();
        Some((source, group))
    }

    /// The album a header row is for — `None` on any other row.
    pub fn album_of(&self, cursor: usize) -> Option<usize> {
        match self.row_at(cursor)? {
            VisibleRow::AlbumHeader { album } => Some(*album),
            VisibleRow::Track { .. } => None,
        }
    }

    /// Fold or unfold album `album`, remembering it in the album's own file so the
    /// state survives a restart.
    pub fn toggle_album(&mut self, album: usize) {
        let Some(loaded) = self.albums.get_mut(album) else {
            return;
        };
        loaded.playlist.collapsed = !loaded.playlist.collapsed;
        if let Err(e) = loaded.playlist.save(&loaded.path) {
            error!(err = %e, path = %loaded.path.display(), "failed to save an album's fold state");
        }
        self.rebuild_rows();
        self.clamp_scroll();
    }

    /// Rename album `album`, in its own file, in the listing, and in its row.
    ///
    /// Refused when the name is already taken or unusable as a filename — an album
    /// is a playlist file, and two of them under one name would be one file.
    pub fn rename_album(
        &mut self,
        album: usize,
        new_name: &str,
    ) -> std::result::Result<(), String> {
        let Some(loaded) = self.albums.get(album) else {
            return Err("no such album".to_string());
        };
        let (old_name, old_path) = (loaded.name.clone(), loaded.path.clone());
        crate::tui::input::validate_playlist_name(
            new_name,
            &self.available_playlists,
            Some(&old_name),
        )?;

        let Some(loaded) = self.albums.get_mut(album) else {
            return Err("no such album".to_string());
        };
        let new_path = loaded
            .playlist
            .rename(new_name, &old_path)
            .map_err(|e| e.to_string())?;
        loaded.name = new_name.to_string();
        loaded.path = new_path.clone();

        for entry in &mut self.available_playlists {
            if entry.name == old_name {
                entry.name = new_name.to_string();
                entry.path = new_path.clone();
            }
        }
        self.available_playlists.sort_by(|a, b| a.name.cmp(&b.name));

        // A rename moves the file the session writes into. Left pointing at the
        // old path, the next position flush would recreate the album under the
        // name the user just renamed away from.
        if let Some(session) = self.playing.as_mut() {
            if session.path == old_path {
                session.path = new_path.clone();
            }
        }
        // Same for the shuffled order, which is keyed by the list's path.
        if self.shuffle_order_path.as_deref() == Some(&*old_path) {
            self.shuffle_order_path = Some(new_path);
        }

        // Albums are ordered by name, so a rename can move this one.
        self.albums.sort_by(|a, b| a.name.cmp(&b.name));
        self.rebuild_rows();
        self.clamp_scroll();
        Ok(())
    }

    /// Forget album `album`: its playlist file goes, and nothing else.
    ///
    /// Not the folder it mirrored, not the files in it, not the documents of the
    /// tracks it listed — those live in the library and may well be listed
    /// elsewhere. Deleting a container has never meant deleting its contents here
    /// (ADR-018).
    pub fn delete_album(&mut self, album: usize) {
        let Some(loaded) = self.albums.get(album) else {
            return;
        };
        let (name, path) = (loaded.name.clone(), loaded.path.clone());

        // Stop first if this is the list playing: the file is about to go, and a
        // later flush against it would write the album back from a stale snapshot.
        if self.playing.as_ref().is_some_and(|p| p.path == path) {
            self.stop_player();
            self.playing = None;
            self.is_paused = false;
        }

        if let Err(e) = Playlist::delete(&path) {
            error!(err = %e, path = %path.display(), "failed to delete album");
            self.set_status(format!("Could not delete {name}"));
            return;
        }
        self.albums.remove(album);
        self.available_playlists.retain(|entry| entry.path != path);
        self.rebuild_rows();
        self.clamp_scroll();
        info!(album = %name, "deleted album");
    }

    /// Rescan the folder album `album` mirrors.
    pub fn rescan_album(&mut self, album: usize) {
        match self
            .albums
            .get(album)
            .and_then(|loaded| loaded.playlist.source_folder.clone())
        {
            Some(root) => self.import_folder(root, None),
            None => self.set_status("Not linked to a folder"),
        }
    }

    /// Drop row `index` from the list `source` names and save that file.
    ///
    /// The track's document is untouched — whether it survives is decided by
    /// whether anything else still lists it (see
    /// `platform_id_referenced_elsewhere`).
    pub fn remove_row(&mut self, source: RowSource, index: usize) {
        match source {
            RowSource::Own => {
                let Some(id) = self.playlist.tracks.get(index).cloned() else {
                    return;
                };
                self.playlist.tracks.remove(index);
                if self.playlist.current_track.as_deref() == Some(&*id) {
                    self.playlist.current_track = None;
                }
                self.save_playlist();
            }
            RowSource::Album(album) => {
                let Some(loaded) = self.albums.get_mut(album) else {
                    return;
                };
                if index >= loaded.playlist.tracks.len() {
                    return;
                }
                let id = loaded.playlist.tracks.remove(index);
                if loaded.playlist.current_track.as_deref() == Some(&*id) {
                    loaded.playlist.current_track = None;
                }
                if let Err(e) = loaded.playlist.save(&loaded.path) {
                    error!(err = %e, path = %loaded.path.display(), "failed to save an album after a row was removed");
                }
            }
        }
        self.rebuild_rows();
    }

    /// Drop the search filter without moving the cursor.
    ///
    /// For an edit that invalidated the filter rather than the user closing it:
    /// the row they were holding is still the row they were holding.
    pub fn drop_filter(&mut self) {
        self.search_query.clear();
        self.rebuild_rows();
    }

    /// Where an own-track index sits on screen, for restoring a cursor.
    pub fn cursor_of_own_index(&self, index: usize) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(row, VisibleRow::Track { source: RowSource::Own, index: i } if *i == index)
        })
    }

    /// How many rows the cursor and the scroll window count — headers included.
    pub fn visible_track_count(&self) -> usize {
        self.rows.len()
    }

    /// Every track the playlist holds, folded away or not — or, under a filter,
    /// every track row on screen, so the panel title agrees with what is there.
    ///
    /// Folding is a view, so it must not change the number. That does mean this is
    /// smaller than `visible_track_count` by the number of headers whenever an
    /// album is present; the two are separate readings, and the panel title keeps
    /// them apart.
    pub fn total_track_count(&self) -> usize {
        if self.has_filter() {
            return self
                .rows
                .iter()
                .filter(|row| matches!(row, VisibleRow::Track { .. }))
                .count();
        }
        self.playlist.tracks.len()
            + self
                .albums
                .iter()
                .map(|loaded| loaded.playlist.tracks.len())
                .sum::<usize>()
    }

    /// What `total_track_count` covers, in seconds. A row whose document is gone,
    /// or whose duration was never learned, contributes nothing.
    pub fn total_duration_secs(&self) -> u64 {
        let sum = |ids: &[String]| -> u64 {
            ids.iter()
                .filter_map(|id| self.library.get(id))
                .map(|track| track.duration)
                .sum()
        };
        if self.has_filter() {
            return (0..self.rows.len())
                .filter_map(|cursor| self.row_track_id(cursor))
                .filter_map(|id| self.library.get(&id))
                .map(|track| track.duration)
                .sum();
        }
        sum(&self.playlist.tracks)
            + self
                .albums
                .iter()
                .map(|loaded| sum(&loaded.playlist.tracks))
                .sum::<u64>()
    }

    /// Move the selected row one place down (`down`) or up within the playlist.
    ///
    /// Refused under a search filter: the cursor counts visible rows there, so
    /// ±1 would jump the row over whatever the filter hides. At either end it is
    /// simply a no-op — nothing to say about a row already where it can go.
    pub fn move_selected_row(&mut self, down: bool) {
        if self.has_filter() {
            self.set_status("Clear the search to reorder");
            return;
        }
        // Albums have no hand-made order to move a row within — they are sorted by
        // name, which is the only order they have.
        if self.album_of(self.selected).is_some() {
            self.set_status("Albums are sorted by name");
            return;
        }

        let Some((source, group)) = self.row_group(self.selected) else {
            return;
        };
        let &VisibleRow::Track { index: from, .. } = self.row_at(self.selected).expect("a row")
        else {
            return;
        };
        let to = if down { from + 1 } else { from.wrapping_sub(1) };
        let len = match self.source_playlist(source) {
            Some((playlist, _)) => playlist.tracks.len(),
            None => return,
        };
        if from >= len || to >= len {
            return;
        }

        match source {
            RowSource::Own => {
                self.playlist.tracks.swap(from, to);
                self.save_playlist();
            }
            RowSource::Album(album) => {
                let Some(loaded) = self.albums.get_mut(album) else {
                    return;
                };
                loaded.playlist.tracks.swap(from, to);
                if let Err(e) = loaded.playlist.save(&loaded.path) {
                    error!(err = %e, path = %loaded.path.display(), "failed to save a reordered album");
                }
            }
        }
        self.rebuild_rows();
        // The cursor stays on the row the user was holding, not on the position.
        // Unfiltered, a list's rows are its tracks in order, so the row that now
        // holds `to` is the group's `to`-th.
        self.selected = group.get(to).copied().unwrap_or(self.selected);
        self.clamp_scroll();
        // `shuffle_order` is deliberately left alone. It holds indices, so after a
        // swap it is still a permutation of `0..len` — no track is skipped or
        // repeated, only two of them trade places in the shuffled run. Rebuilding
        // would throw away a run the user is in the middle of to fix nothing.
    }

    /// Adopt whatever the search prompt currently holds as the filter.
    pub fn update_search(&mut self) {
        self.search_query = self.input_buf.clone();
        self.rebuild_rows();
        self.selected = 0;
        self.track_offset = 0;
    }

    /// Drop the search filter and show everything again.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.rebuild_rows();
        self.selected = 0;
        self.track_offset = 0;
    }

    pub fn sidebar_next(&mut self) {
        let items = self.sidebar_items();
        let start = self.sidebar_selected;
        let mut idx = (start + 1) % items.len();
        for _ in 0..items.len() {
            if items[idx].is_selectable() {
                self.sidebar_selected = idx;
                return;
            }
            idx = (idx + 1) % items.len();
        }
    }

    pub fn sidebar_prev(&mut self) {
        let items = self.sidebar_items();
        let len = items.len();
        let start = self.sidebar_selected;
        let mut idx = if start == 0 { len - 1 } else { start - 1 };
        for _ in 0..len {
            if items[idx].is_selectable() {
                self.sidebar_selected = idx;
                return;
            }
            idx = if idx == 0 { len - 1 } else { idx - 1 };
        }
    }

    /// The production build never checks this directly — `app.downloading` is
    /// read via `.contains`/`.len` in `ui.rs` — but the tests do.
    #[allow(dead_code)]
    pub fn is_downloading(&self) -> bool {
        !self.downloading.is_empty()
    }

    pub fn fetch_url(&mut self, url: String) {
        self.fetch_url_to(url, None);
    }

    /// Fetch metadata for `url` and, on success, add the track to the playlist at
    /// `target_path`. When `target_path` is `None` the track is added to the current
    /// active playlist (the existing behaviour).
    pub fn fetch_url_to(&mut self, url: String, target_path: Option<PathBuf>) {
        self.pending_fetches += 1;
        info!(url = %url, target = ?target_path, "fetching metadata");
        let task_tx = self.task_tx.clone();
        tokio::spawn(async move {
            match ytdlp::fetch_metadata(&url).await {
                Ok(meta) => {
                    let _ = task_tx.send(TaskMsg::MetaReady {
                        url,
                        meta,
                        target_path,
                    });
                }
                Err(e) => {
                    let _ = task_tx.send(TaskMsg::MetaError {
                        url,
                        err: e.to_string(),
                    });
                }
            }
        });
    }

    /// Cycle `target_playlist_for_url` to the next available playlist.
    /// The cycle order is: all playlists (including the active one) sorted alphabetically.
    /// If `target_playlist_for_url` is `None` or points to the last in the list, wraps around.
    pub fn cycle_url_target_playlist(&mut self) {
        let all: Vec<String> = self
            .available_playlists
            .iter()
            .map(|entry| entry.name.clone())
            .collect();

        if all.is_empty() {
            return;
        }

        let current = self
            .target_playlist_for_url
            .as_deref()
            .unwrap_or(&self.playlist.name);

        let next = if let Some(pos) = all.iter().position(|n| n == current) {
            all[(pos + 1) % all.len()].clone()
        } else {
            all[0].clone()
        };

        self.target_playlist_for_url = Some(next);
    }

    /// Cycle `target_list_for_add` to the next explicit destination, wrapping
    /// back to "Auto" (`None`) after the last one — unlike
    /// `cycle_url_target_playlist`, whose cycle has no "Auto" stop because a
    /// URL add has no folder-identity matching to default to.
    pub fn cycle_add_target(&mut self) {
        let all: Vec<String> = self
            .available_playlists
            .iter()
            .map(|entry| entry.name.clone())
            .collect();

        if all.is_empty() {
            self.target_list_for_add = None;
            return;
        }

        self.target_list_for_add = match self.target_list_for_add.as_deref() {
            Some(current) => match all.iter().position(|n| n == current) {
                Some(pos) if pos + 1 < all.len() => Some(all[pos + 1].clone()),
                _ => None,
            },
            None => Some(all[0].clone()),
        };
    }

    pub fn save_playlist(&self) {
        if let Err(e) = self.playlist.save(&self.playlist_path) {
            error!(err = %e, "failed to auto-save playlist");
        }
    }

    /// Persist the currently playing track's live position into
    /// `last_position` and save it to disk. Called right before quitting so
    /// resume-on-launch (Task 6) has an up-to-date value — previously
    /// `last_position` was only ever updated when switching *away* from a
    /// track mid-session (in `request_playback`), never on quit, so whatever
    /// track was playing when the user pressed `q` always resumed from 0:00.
    ///
    /// No-op if nothing is playing. Writes only the playing track's own document,
    /// which is what makes this cheap enough to run periodically.
    pub fn flush_playing_position(&mut self) {
        if self.playing.is_none() {
            return;
        }
        let pos = self.position as u64;
        if let Some(track) = self.playing_track_mut() {
            track.last_position = pos;
        }
        self.save_playing_track();
    }

    /// Write the playing track's position to disk, but at most once every
    /// `POSITION_FLUSH_INTERVAL`.
    ///
    /// `flush_playing_position` otherwise runs only on quit and when switching
    /// away from a track, so anything short of a clean exit — a `SIGKILL`, a
    /// closed lid, a power cut — threw away the whole session's listening
    /// progress. Skipped while paused or with no live player, since the position
    /// cannot have moved.
    pub fn maybe_flush_position(&mut self) {
        if self.player.is_none() || self.is_paused || self.playing.is_none() {
            return;
        }
        if self.last_position_flush.elapsed() < POSITION_FLUSH_INTERVAL {
            return;
        }
        self.last_position_flush = Instant::now();
        self.flush_playing_position();
    }

    /// Mutate one track in the library and persist its document.
    ///
    /// The general "change a track that may or may not be on screen" mechanism.
    /// Which playlists list it is irrelevant — they all read the same document,
    /// so this is one small write rather than one per playlist.
    ///
    /// An id with no document is a no-op (logged, not an error): a download can
    /// still finish for a track the user has since deleted.
    pub fn patch_track(&mut self, id: &str, f: impl FnOnce(&mut Track)) {
        match self.library.get_mut(id) {
            Some(track) => f(track),
            None => {
                warn!(id = %id, "patch_track: no such track in the library");
                return;
            }
        }
        if let Err(e) = self.library.save(id) {
            error!(err = %e, id = %id, "patch_track: failed to save the track document");
        }
    }

    /// Switch the displayed playlist to the one at `path` with the given `name`.
    ///
    /// - Does **not** affect playback: `self.player`, `self.playing`, and
    ///   `self.position` are left untouched, so browsing/editing another
    ///   playlist never interrupts whatever is currently playing.
    /// - Loads the playlist from disk; returns an error on failure.
    /// - Resets track selection, scroll offset, and search filter state.
    /// - Updates `playlist_path` to the new path.
    /// - Switches focus to the track list so the user can browse the new playlist.
    pub fn switch_to_playlist(&mut self, name: &str, path: &std::path::Path) -> anyhow::Result<()> {
        use anyhow::Context as _;

        let new_playlist = Playlist::load(path)
            .with_context(|| format!("failed to load playlist '{name}' from {}", path.display()))?;

        // If the track that's playing lives in the playlist we're about to stop
        // displaying, its `PlayingSession.playlist` clone has been sitting
        // untouched while `self.playlist` received the in-place edits (add,
        // delete, loop/shuffle toggles). Refresh the clone now, at the last
        // moment the two still refer to the same file: auto-advance reads its
        // running order and loop/shuffle settings from that snapshot, and a stale
        // one would step to the wrong track — or write itself back over those
        // edits when `play_session_track` saves the session's file.
        if let Some(session) = self.playing.as_mut() {
            if session.path == self.playlist_path {
                session.playlist = self.playlist.clone();
            }
        }

        // Replace playlist state
        self.playlist = new_playlist;
        self.playlist_path = path.to_path_buf();

        // Persist the newly active playlist so restarting the app reopens it.
        self.config.active_playlist = Some(name.to_string());
        let _ = self.config.save();

        // Reset track list state
        self.selected = 0;
        self.track_offset = 0;
        self.search_query.clear();
        self.input_buf.clear();

        // The albums hanging under the new playlist are a different set entirely,
        // and the rows are built from them.
        self.load_albums();
        self.rebuild_rows();

        // Restore cursor to last-played track when available. `current_track` is an
        // index into the playlist's own list and the cursor counts rows, so it has
        // to be translated rather than assigned.
        if let Some(cursor) = self
            .current_track_index()
            .and_then(|index| self.cursor_of_own_index(index))
        {
            self.selected = cursor;
        }

        // Move focus to track list so user can immediately interact
        self.focus = Focus::TrackList;

        Ok(())
    }

    /// The file stem of the list the selected row belongs to — the displayed
    /// playlist, or the album the row sits under.
    fn selected_row_list_name(&self) -> String {
        match self.row_group(self.selected).map(|(source, _)| source) {
            Some(RowSource::Album(album)) => self
                .albums
                .get(album)
                .map(|loaded| loaded.name.clone())
                .unwrap_or_else(|| self.playlist.name.clone()),
            _ => self.playlist.name.clone(),
        }
    }

    /// Returns playlist names available as move targets — everything but the list
    /// the selected row already belongs to, which for an album row means the album
    /// rather than the playlist showing it.
    pub fn available_playlist_names(&self) -> Vec<String> {
        let own = self.selected_row_list_name();
        self.available_playlists
            .iter()
            .filter(|entry| entry.name != own)
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Move the currently selected track to the named playlist.
    ///
    /// Handles:
    /// - Stopping playback if the moved track is currently playing.
    /// - Loading the target playlist from disk (or returning an error if missing).
    /// - Saving both source and target playlists atomically.
    /// - Updating `available_playlists` when the target playlist is a new entry.
    pub fn move_track_to_playlist(&mut self, target_name: &str) -> anyhow::Result<()> {
        use anyhow::Context as _;

        // The row's own list — which may be an album under the displayed playlist,
        // and then it is the album's file the row leaves.
        let &VisibleRow::Track { source, index } = self
            .row_at(self.selected)
            .with_context(|| "no track at current selection")?
        else {
            anyhow::bail!("no track at current selection");
        };
        let (from_playlist, from_path) = self
            .source_playlist(source)
            .with_context(|| "no track at current selection")?;
        let from_path = from_path.to_path_buf();
        let id = from_playlist
            .tracks
            .get(index)
            .cloned()
            .with_context(|| "no track at current selection")?;

        // Resolve the target playlist path
        let target_path = self
            .available_playlists
            .iter()
            .find(|entry| entry.name == target_name)
            .map(|entry| entry.path.clone())
            .with_context(|| {
                format!("target playlist '{target_name}' not found in available_playlists")
            })?;

        // The entry is listed but its file is gone — recreate it empty before
        // `with_list_at` tries to load it below. Can only happen for a path
        // with no in-memory copy: `self.playlist`/`self.albums` are always
        // backed by a real file.
        if !target_path.exists() {
            Playlist::create(target_name)
                .with_context(|| format!("failed to create target playlist '{target_name}'"))?;
        }

        // Stop playback only if the track being moved is literally the one
        // actually driving playback right now — identity is `(path,
        // id)`, not just a matching `id` that might coincidentally
        // also exist in an unrelated playing session elsewhere.
        let is_current = self.is_playing_track(&from_path, &id);
        if is_current {
            self.stop_player(); // kills mpv and retires its position poller
            self.playing = None;
            self.is_paused = false;
            self.position = 0.0;
        }

        // Remove from the row's own list. Only the row moves — the track's document
        // stays exactly where it is, which is why an in-flight download for it
        // needs no bookkeeping any more.
        let removed = match source {
            RowSource::Own => self.playlist.remove_track_by_id(&id),
            RowSource::Album(album) => self
                .albums
                .get_mut(album)
                .is_some_and(|loaded| loaded.playlist.remove_track_by_id(&id)),
        };
        anyhow::ensure!(removed, "track '{id}' not found in source playlist");

        // Append to the target list, wherever it lives — the displayed
        // playlist, one of its loaded albums, or a file nothing currently
        // holds in memory. Save target first, then source (both atomic).
        let moved_id = id.clone();
        self.with_list_at(&target_path, |pl, _lib| pl.add_track(moved_id))
            .with_context(|| format!("failed to save target playlist '{target_name}'"))?;
        let (from_playlist, _) = self
            .source_playlist(source)
            .with_context(|| "the row's own list went away mid-move")?;
        from_playlist
            .save(&from_path)
            .with_context(|| "failed to save source playlist after move")?;

        // Clear any active search filter: it was built over rows that no longer
        // describe the list.
        self.drop_filter();

        // Clamp the selection cursor so it stays in bounds
        let new_count = self.visible_track_count();
        if self.selected >= new_count && self.selected > 0 {
            self.selected -= 1;
        }
        self.clamp_scroll();

        Ok(())
    }

    pub fn quality_next(&mut self) {
        self.config.audio_quality = match self.config.audio_quality {
            AudioQuality::Best => AudioQuality::High,
            AudioQuality::High => AudioQuality::Medium,
            AudioQuality::Medium => AudioQuality::Low,
            AudioQuality::Low => AudioQuality::Best,
        };
        let _ = self.config.save();
    }

    pub fn quality_prev(&mut self) {
        self.config.audio_quality = match self.config.audio_quality {
            AudioQuality::Best => AudioQuality::Low,
            AudioQuality::High => AudioQuality::Best,
            AudioQuality::Medium => AudioQuality::High,
            AudioQuality::Low => AudioQuality::Medium,
        };
        let _ = self.config.save();
    }
}

// ── PlaySource (internal) ─────────────────────────────────────────────────

enum PlaySource {
    File(PathBuf),
    Stream(String),
}

/// Where a local track's media is meant to be: the recorded file, falling back
/// to `url`, which for a local track is that same path.
fn local_media_path(track: &Track) -> PathBuf {
    track
        .file
        .clone()
        .unwrap_or_else(|| PathBuf::from(&track.url))
}

/// Where mpv should read this track from: the cached file when there is one,
/// otherwise the remote stream.
fn play_source_for(track: &Track) -> PlaySource {
    match (&track.cache_status, &track.file) {
        (CacheStatus::Cached, Some(file)) => PlaySource::File(file.clone()),
        _ => PlaySource::Stream(track.url.clone()),
    }
}

/// Whether a track is a hit for the search box. `query` must already be
/// lowercased; the user-set title and artist are searched alongside the
/// yt-dlp-provided ones, since a renamed track is what the user remembers.
fn track_matches(track: &Track, query: &str) -> bool {
    let hit = |s: &str| s.to_lowercase().contains(query);
    hit(&track.title)
        || hit(&track.artist)
        || track.user_title.as_deref().is_some_and(hit)
        || track.user_artist.as_deref().is_some_and(hit)
}

// ── Shutdown plumbing ─────────────────────────────────────────────────────

/// Restores the terminal when `run` returns for *any* reason, including an early
/// `?`. `ratatui::init()` installs a panic hook that already covers panics, but
/// an error propagating out of the event loop would otherwise leave the terminal
/// in raw mode on the alternate screen — indistinguishable from a hard crash.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// Flip `flag` when the process is asked to terminate, so the event loop can run
/// its normal shutdown path (flush position → save playlist → kill mpv →
/// restore terminal) instead of dying without unwinding and leaving mpv playing
/// with no UI attached.
///
/// `SIGHUP` is the one that matters most in practice: it is what arrives when the
/// user closes the terminal window mid-playback.
fn spawn_signal_listener(flag: Arc<AtomicBool>) {
    use tokio::signal::unix::{signal, SignalKind};

    tokio::spawn(async move {
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                error!(err = %e, "failed to register SIGINT handler");
                return;
            }
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                error!(err = %e, "failed to register SIGTERM handler");
                return;
            }
        };
        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                error!(err = %e, "failed to register SIGHUP handler");
                return;
            }
        };

        tokio::select! {
            _ = sigint.recv() => info!("SIGINT received"),
            _ = sigterm.recv() => info!("SIGTERM received"),
            _ = sighup.recv() => info!("SIGHUP received"),
        }

        flag.store(true, Ordering::SeqCst);
    });
}

// ── Event loop ────────────────────────────────────────────────────────────

pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::init();
    let _terminal_guard = TerminalGuard;

    let shutdown = Arc::new(AtomicBool::new(false));
    spawn_signal_listener(Arc::clone(&shutdown));

    loop {
        app.sync_channels();
        app.maybe_flush_position();
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if input::handle_key(app, key).await? == input::Action::Quit {
                    break;
                }
            }
        }

        if shutdown.load(Ordering::SeqCst) {
            app.should_quit = true;
        }

        if app.should_quit {
            break;
        }
    }

    app.flush_playing_position();
    // Kill mpv before the terminal is restored, so a slow teardown can never
    // leave audio playing over a shell prompt.
    app.stop_player();

    Ok(())
}
