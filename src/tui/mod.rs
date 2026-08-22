pub mod input;
pub mod ui;

#[cfg(test)]
mod ui_test;

use crate::cache;
use crate::config::{AudioQuality, Config};
use crate::library::{self, Library};
use crate::library::{CacheStatus, MediaKind, Track, TrackOrigin};
use crate::player::{self, Player};
use crate::playlist::{self, LoopMode, Playlist};
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
    ConfirmDelete,
    SearchInput,
    TrackContextMenu,
    PlaylistRename,
    PlaylistDelete,
    Help,
}

// ── SidebarItem ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SidebarItem {
    PlaylistsHeader,
    Playlist { name: String, path: PathBuf },
    Separator,
    Music,
    Video,
    Plunder,
    Settings,
}

impl SidebarItem {
    pub fn is_selectable(&self) -> bool {
        matches!(
            self,
            SidebarItem::PlaylistsHeader
                | SidebarItem::Playlist { .. }
                | SidebarItem::Plunder
                | SidebarItem::Settings
        )
    }
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
    pub filtered_indices: Vec<usize>,

    // Sidebar
    pub sidebar_selected: usize,
    pub playlists_expanded: bool,
    pub available_playlists: Vec<(String, PathBuf)>,

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
}

impl App {
    pub fn new(
        playlist: Playlist,
        config: Config,
        available_playlists: Vec<(String, PathBuf)>,
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
            filtered_indices: Vec::new(),
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
        };
        if let Some(idx) = app.current_track_index() {
            app.selected = idx;
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

    /// The track a displayed row resolves to, or `None` when its document has
    /// gone missing from the library — a row is an id, and the document it names
    /// can be deleted from under it (by another instance, or by hand).
    pub fn track_at(&self, idx: usize) -> Option<&Track> {
        self.library.get(self.playlist.tracks.get(idx)?)
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
    /// A no-op, with a status message, if a download for it is already running.
    pub fn recache_track(&mut self, idx: usize) {
        let Some(track) = self.track_at(idx) else {
            return;
        };
        let id = track.id.clone();
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
    pub fn platform_id_referenced_elsewhere(&self, platform_id: &str) -> bool {
        let lists = |ids: &[String]| {
            ids.iter()
                .any(|id| library::platform_id_of(id) == platform_id)
        };
        if lists(&self.playlist.tracks) {
            return true;
        }
        self.available_playlists
            .iter()
            .filter(|(_, path)| path != &self.playlist_path)
            .any(|(_, path)| match Playlist::load(path) {
                Ok(pl) => lists(&pl.tracks),
                Err(e) => {
                    warn!(err = %e, path = %path.display(), "could not check playlist for shared cache file; keeping it");
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
        if !self.playlist.shuffle {
            self.shuffle_order.clear();
            self.shuffle_order_path = None;
            return;
        }
        self.shuffle_order =
            playlist::shuffled_indices(self.playlist.tracks.len(), playlist::shuffle_seed());
        self.shuffle_order_path = Some(self.playlist_path.clone());
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

        // Same file as the one on screen: go through the normal path so the
        // displayed copy, its `current_track` and the cursor stay in step.
        if session.path == self.playlist_path {
            let start_pos = self.track_at(idx).and_then(input::resume_start_pos);
            self.request_playback(idx, start_pos);
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
        self.spawn_player_for(id, source, speed, start_pos);
    }

    /// Start playback of the track at Vec index `idx` within the displayed
    /// playlist (`self.playlist`).
    /// `start_pos`: resume at this position in seconds (used when switching
    /// from stream to local file mid-play; pass `None` for a fresh start).
    pub fn request_playback(&mut self, idx: usize, start_pos: Option<f64>) {
        // Collect all track data before any mutations (borrow checker)
        let (id, speed, source) = {
            let Some(id) = self.playlist.tracks.get(idx).cloned() else {
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
                .or(self.playlist.default_speed)
                .unwrap_or(self.config.default_speed);
            (id, speed, play_source_for(track))
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
            if leaving != (self.playlist_path.clone(), id.clone()) {
                let pos = self.position as u64;
                if let Some(t) = self.playing_track_mut() {
                    t.last_position = pos;
                }
                // Persist the mutation above so the position update isn't
                // silently dropped when `self.playing` is replaced below.
                self.save_playing_track();
            }
        }

        self.playing = Some(PlayingSession {
            path: self.playlist_path.clone(),
            playlist: self.playlist.clone(),
            track_id: id.clone(),
        });
        // `current_track` on the displayed playlist means strictly "last
        // track selected/played in *this* playlist file" — used only to
        // restore the cursor on load. Since `idx` always indexes into
        // `self.playlist` here, the playing track does live in the
        // displayed playlist, so record it.
        self.playlist.current_track = Some(id.clone());
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

        self.spawn_player_for(id, source, speed, start_pos);
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

        let speed = {
            // Fall back to the *playing* playlist's default speed, not the
            // displayed one — they may differ once playback is decoupled
            // from the displayed playlist.
            let playing_playlist = self
                .playing_playlist()
                .expect("just verified a playing track exists");
            let track = self
                .playing_track()
                .expect("just verified a playing track exists");
            effective_speed(track, playing_playlist, &self.config)
        };
        let pos = self.position;
        info!(id = %id, pos = pos, "switching stream → local file");
        self.is_paused = false;
        self.spawn_player_for(id.to_string(), PlaySource::File(file), speed, Some(pos));
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
    ) {
        let generation = self.stop_player();
        let volume = self.config.default_volume;
        let quality = self.config.audio_quality.clone();
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

            match Player::spawn(&resolved_source, start_pos).await {
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
            for (name, path) in &self.available_playlists {
                items.push(SidebarItem::Playlist {
                    name: name.clone(),
                    path: path.clone(),
                });
            }
        }
        items.push(SidebarItem::Separator);
        items.push(SidebarItem::Music);
        items.push(SidebarItem::Video);
        items.push(SidebarItem::Separator);
        items.push(SidebarItem::Plunder);
        items.push(SidebarItem::Settings);
        items
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
                let displayed = owning_path == self.playlist_path;

                // Load the target list up front, so nothing is written and no
                // download is started if the row cannot be recorded anywhere: a
                // download completing against a playlist that has no such row
                // leaves an untracked file in the audio cache.
                let mut target_pl = if displayed {
                    None
                } else {
                    match Playlist::load(&owning_path) {
                        Ok(pl) => Some(pl),
                        Err(e) => {
                            error!(err = %e, path = %owning_path.display(), "target playlist not found, track not added");
                            self.set_status("Target playlist not found");
                            return;
                        }
                    }
                };

                // Adding the same URL twice used to produce a second row sharing
                // the first one's id, and so its cached file too — two rows whose
                // download, cache status and deletion all fight over one file.
                let ids = target_pl
                    .as_ref()
                    .map_or(&self.playlist.tracks, |pl| &pl.tracks);
                if ids.contains(&id) {
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

                match target_pl.as_mut() {
                    Some(pl) => {
                        pl.add_track(id.clone());
                        if let Err(e) = pl.save(&owning_path) {
                            error!(err = %e, "failed to save target playlist after URL add");
                            self.set_status("Could not save to target playlist");
                            return;
                        }
                        self.set_status(format!("Added to playlist: {status_title}"));
                    }
                    None => {
                        // The cursor deliberately stays where the user left it. It
                        // used to jump to the new row, which meant adding a track
                        // while browsing moved the selection out from under `Enter`
                        // and `d` — and with a search filter active it jumped to a
                        // row index that the filter does not even display.
                        self.playlist.add_track(id.clone());
                        self.save_playlist();
                        self.set_status(format!("Added: {status_title}"));
                    }
                }
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

    pub fn visible_track_count(&self) -> usize {
        if self.filtered_indices.is_empty() {
            self.playlist.tracks.len()
        } else {
            self.filtered_indices.len()
        }
    }

    pub fn track_index_at(&self, cursor: usize) -> Option<usize> {
        if self.filtered_indices.is_empty() {
            if cursor < self.playlist.tracks.len() {
                Some(cursor)
            } else {
                None
            }
        } else {
            self.filtered_indices.get(cursor).copied()
        }
    }

    pub fn update_search(&mut self) {
        let query = self.input_buf.to_lowercase();
        if query.is_empty() {
            self.filtered_indices.clear();
        } else {
            // Collected into a local first: the filter reads `self.library` while
            // `self.filtered_indices` is being assigned.
            let matches: Vec<usize> = self
                .playlist
                .tracks
                .iter()
                .enumerate()
                // A row whose document is missing matches nothing — there is no
                // title to match against.
                .filter(|(_, id)| {
                    self.library
                        .get(id)
                        .is_some_and(|t| track_matches(t, &query))
                })
                .map(|(i, _)| i)
                .collect();
            self.filtered_indices = matches;
        }
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
            .map(|(n, _)| n.clone())
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
        self.filtered_indices.clear();
        self.input_buf.clear();

        // Restore cursor to last-played track when available
        if let Some(idx) = self.current_track_index() {
            self.selected = idx;
        }

        // Move focus to track list so user can immediately interact
        self.focus = Focus::TrackList;

        Ok(())
    }

    /// Returns playlist names available as move targets (excludes the currently active playlist).
    pub fn available_playlist_names(&self) -> Vec<String> {
        self.available_playlists
            .iter()
            .filter(|(name, _)| name != &self.playlist.name)
            .map(|(name, _)| name.clone())
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

        // Determine the real track index for the cursor position
        let track_idx = self
            .track_index_at(self.selected)
            .with_context(|| "no track at current selection")?;

        let id = self.playlist.tracks[track_idx].clone();

        // Resolve the target playlist path
        let target_path = self
            .available_playlists
            .iter()
            .find(|(n, _)| n == target_name)
            .map(|(_, p)| p.clone())
            .with_context(|| {
                format!("target playlist '{target_name}' not found in available_playlists")
            })?;

        // Load or create the target playlist from disk
        let mut target_playlist = if target_path.exists() {
            Playlist::load(&target_path)
                .with_context(|| format!("failed to load target playlist '{target_name}'"))?
        } else {
            // Path listed but file missing – create a fresh empty playlist
            let (pl, _) = Playlist::create(target_name)
                .with_context(|| format!("failed to create target playlist '{target_name}'"))?;
            pl
        };

        // Stop playback only if the track being moved is literally the one
        // actually driving playback right now — identity is `(path,
        // id)`, not just a matching `id` that might coincidentally
        // also exist in an unrelated playing session elsewhere.
        let is_current = self.is_playing_track(&self.playlist_path, &id);
        if is_current {
            self.stop_player(); // kills mpv and retires its position poller
            self.playing = None;
            self.playlist.current_track = None;
            self.is_paused = false;
            self.position = 0.0;
        }

        // Remove from source playlist. Only the row moves — the track's document
        // stays exactly where it is, which is why an in-flight download for it
        // needs no bookkeeping any more.
        anyhow::ensure!(
            self.playlist.remove_track_by_id(&id),
            "track '{id}' not found in source playlist"
        );

        // Append to target playlist
        target_playlist.add_track(id);

        // Save target first, then source (both atomic)
        target_playlist
            .save(&target_path)
            .with_context(|| format!("failed to save target playlist '{target_name}'"))?;
        self.playlist
            .save(&self.playlist_path)
            .with_context(|| "failed to save source playlist after move")?;

        // Clear any active search filter: the index set is now stale after the removal.
        self.filtered_indices.clear();

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
