pub mod input;
pub mod ui;

#[cfg(test)]
mod ui_test;

use crate::cache;
use crate::config::{AudioQuality, Config};
use crate::player::{self, Player};
use crate::playlist::{CacheStatus, Playlist, Track};
use crate::ytdlp::{self, TrackMeta};
use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    MetaReady { url: String, meta: TrackMeta, target_path: Option<PathBuf> },
    MetaError { url: String, err: String },
    DownloadDone { video_id: String, file: PathBuf },
    DownloadError { video_id: String, err: String },
    PlayerReady { video_id: String, player: Box<Player> },
    PlayerError { video_id: String, err: String },
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

// ── PlayingSession ────────────────────────────────────────────────────────

/// Snapshot of the playlist/track that is actually driving playback right
/// now, independent of whichever playlist the user happens to be browsing.
///
/// `playlist` is a full loaded copy of the playlist the playing track
/// belongs to. When `path` matches `App::playlist_path` (the user is
/// browsing the same playlist that's playing), display/mutation code should
/// prefer `App::playlist` instead — see `App::playing_track`/
/// `App::playing_track_mut` — so the two views of "the same playlist" never
/// diverge.
pub struct PlayingSession {
    pub path: PathBuf,
    pub playlist: Playlist,
    pub track_idx: usize,
}

impl PlayingSession {
    pub fn track(&self) -> &Track {
        &self.playlist.tracks[self.track_idx]
    }

    pub fn track_mut(&mut self) -> &mut Track {
        &mut self.playlist.tracks[self.track_idx]
    }
}

// ── App ───────────────────────────────────────────────────────────────────

pub struct App {
    // Playlist & config
    pub playlist: Playlist,
    pub playlist_path: PathBuf,
    pub config: Config,
    pub player: Option<Player>,
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
    /// `video_id`. A `HashMap` (rather than a single global `f32`) so
    /// multiple concurrent downloads never cross-contaminate each other's
    /// displayed percentage.
    pub download_progress: HashMap<String, f32>,
    pub is_paused: bool,

    // Footer status message (toast-style)
    pub status_message: Option<(String, Instant)>,

    // Tracks being downloaded
    pub downloading: HashSet<String>,
    // Maps video_id → target playlist path for tracks downloading into a non-active playlist
    pub download_targets: HashMap<String, PathBuf>,
    // In-flight metadata fetches
    pub pending_fetches: usize,

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
    ) -> Self {
        let (pos_tx, position_rx) = watch::channel(0.0f64);
        let (download_tx, download_rx) = watch::channel((String::new(), 0.0f32));
        let (task_tx, task_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            playlist,
            playlist_path,
            config,
            player: None,
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
            download_targets: HashMap::new(),
            pending_fetches: 0,
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
        self.playlist.tracks.iter().position(|t| t.video_id == id)
    }

    /// Returns true if the track identified by `(path, video_id)` is
    /// literally the one actually driving playback right now — i.e.
    /// `self.playing` points at a session whose playlist file is `path` and
    /// whose current track's `video_id` matches. Used to guard delete/move
    /// operations so they only stop playback when the track being removed is
    /// truly the one playing, not just any track that happens to share a
    /// `video_id` with an unrelated playing session in a different playlist.
    pub fn is_playing_track(&self, path: &Path, video_id: &str) -> bool {
        self.playing
            .as_ref()
            .is_some_and(|p| p.path == path && p.track().video_id == video_id)
    }

    /// Persist whatever mutation was just made (via `playing_track_mut()`)
    /// to the track actually driving playback, routed through the same
    /// in-memory-vs-disk identity rule used throughout this module: if the
    /// playing session's playlist file is the one currently displayed, save
    /// via `self.save_playlist()` so in-memory and on-disk state stay in
    /// sync; otherwise save the playing session's own private playlist copy
    /// directly to its file. No-op if nothing is playing. This is the single
    /// "resolve by path identity, then persist" implementation shared by
    /// `request_playback` (leaving-track position save), `flush_playing_position`,
    /// and `adjust_playing_track_speed`.
    pub fn save_playing_session_playlist(&mut self) {
        let Some(session) = self.playing.as_ref() else {
            return;
        };
        if session.path == self.playlist_path {
            self.save_playlist();
        } else {
            let path = session.path.clone();
            if let Err(e) = session.playlist.save(&path) {
                error!(err = %e, path = %path.display(), "failed to save playing session's playlist");
            }
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
    /// When the playing session's playlist is the same file currently
    /// displayed (`playing.path == self.playlist_path`), this resolves the
    /// track from `self.playlist` instead of the session's private copy, so
    /// edits made through the track list (rename, speed change, etc.) are
    /// reflected immediately without any extra sync step.
    pub fn playing_track(&self) -> Option<&Track> {
        let session = self.playing.as_ref()?;
        if session.path == self.playlist_path {
            let video_id = &session.track().video_id;
            self.playlist.tracks.iter().find(|t| t.video_id == *video_id)
        } else {
            Some(session.track())
        }
    }

    /// Mutable counterpart of `playing_track` — see its docs for the
    /// same-path/borrow-from-displayed-playlist behavior.
    pub fn playing_track_mut(&mut self) -> Option<&mut Track> {
        let path_matches = self.playing.as_ref()?.path == self.playlist_path;
        if path_matches {
            let video_id = self.playing.as_ref().unwrap().track().video_id.clone();
            self.playlist.tracks.iter_mut().find(|t| t.video_id == video_id)
        } else {
            self.playing.as_mut().map(|p| p.track_mut())
        }
    }

    /// Start playback of the track at Vec index `idx` within the displayed
    /// playlist (`self.playlist`).
    /// `start_pos`: resume at this position in seconds (used when switching
    /// from stream to local file mid-play; pass `None` for a fresh start).
    pub fn request_playback(&mut self, idx: usize, start_pos: Option<f64>) {
        // Collect all track data before any mutations (borrow checker)
        let (video_id, speed, source) = {
            let Some(track) = self.playlist.tracks.get(idx) else {
                return;
            };
            let video_id = track.video_id.clone();
            let speed = track.speed
                .or(self.playlist.default_speed)
                .unwrap_or(self.config.default_speed);
            let source = match (&track.cache_status, &track.file) {
                (CacheStatus::Cached, Some(file)) => PlaySource::File(file.clone()),
                _ => PlaySource::Stream(track.url.clone()),
            };
            (video_id, speed, source)
        };

        // Save position of the track we're leaving (not applicable when switching
        // within the same track, e.g. stream → local file). The leaving track may
        // live in the displayed playlist or in a different one entirely (the user
        // was browsing elsewhere while it played) — route the write through
        // whichever copy is the source of truth for that track's identity.
        if let Some(session) = self.playing.as_ref() {
            let leaving_id = session.track().video_id.clone();
            if leaving_id != video_id {
                let pos = self.position as u64;
                if let Some(t) = self.playing_track_mut() {
                    t.last_position = pos;
                }
                // Persist the mutation above — whichever copy is the source of
                // truth for the leaving track (displayed playlist or the
                // playing session's own file) — so the position update isn't
                // silently dropped when `self.playing` is replaced below.
                self.save_playing_session_playlist();
            }
        }

        self.playing = Some(PlayingSession {
            path: self.playlist_path.clone(),
            playlist: self.playlist.clone(),
            track_idx: idx,
        });
        // `current_track` on the displayed playlist means strictly "last
        // track selected/played in *this* playlist file" — used only to
        // restore the cursor on load. Since `idx` always indexes into
        // `self.playlist` here, the playing track does live in the
        // displayed playlist, so record it.
        self.playlist.current_track = Some(video_id.clone());
        self.is_paused = false;
        // Only reset the position display on a fresh start, not on stream→file switch
        if start_pos.is_none() {
            self.position = 0.0;
            let _ = self.pos_tx.send(0.0);
        }

        self.spawn_player_for(video_id, source, speed, start_pos);
    }

    /// If `(owning_path, video_id)` identifies the track actually driving
    /// playback right now (per `self.playing`, independent of what's
    /// displayed), sync its cache status/file into the playing session's own
    /// view, and — if a player is actually running — spawn a fresh mpv
    /// process against the freshly downloaded local `file`, resuming at the
    /// current live position. This is the stream→local-file hot-switch
    /// triggered by `TaskMsg::DownloadDone`.
    ///
    /// Identity is checked as `(path, video_id)`, not `video_id` alone —
    /// matching `is_playing_track` — so a download for a track that merely
    /// shares a `video_id` with the actually-playing track in a *different*
    /// playlist file never hijacks playback.
    fn hot_switch_to_local_file(&mut self, owning_path: &Path, video_id: &str, file: PathBuf) {
        if !self.is_playing_track(owning_path, video_id) {
            return;
        }

        // `patch_and_save_playlist` above already updated the on-disk copy and, if
        // `playing.path == self.playlist_path`, the in-memory displayed playlist too
        // (which `playing_track`/`playing_track_mut` borrow from in that case). When
        // the playing session belongs to a *different* playlist than the one
        // displayed, that patch never touched the session's own private `Playlist`
        // copy — sync it here so `playing_track()` reflects the new cache state
        // regardless of whether a player is actually running.
        if let Some(track) = self.playing_track_mut() {
            track.cache_status = CacheStatus::Cached;
            track.file = Some(file.clone());
        }

        if self.player.is_none() {
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
        info!(video_id = %video_id, pos = pos, "switching stream → local file");
        self.is_paused = false;
        self.spawn_player_for(video_id.to_string(), PlaySource::File(file), speed, Some(pos));
    }

    /// Resolve the stream/local-file source and spawn mpv, wiring up position
    /// polling and reporting the result back via `TaskMsg::PlayerReady`/
    /// `PlayerError`. Pure "start a player" — callers are responsible for any
    /// `self.playing`/`current_track`/`position` bookkeeping beforehand.
    fn spawn_player_for(&self, video_id: String, source: PlaySource, speed: f32, start_pos: Option<f64>) {
        let volume = self.config.default_volume;
        let quality = self.config.audio_quality.clone();
        let task_tx = self.task_tx.clone();
        let pos_tx = self.pos_tx.clone();

        tokio::spawn(async move {
            let resolved_source = match source {
                PlaySource::File(path) => path.to_string_lossy().into_owned(),
                PlaySource::Stream(url) => {
                    match ytdlp::get_stream_url(&url, &quality).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = task_tx.send(TaskMsg::PlayerError {
                                video_id,
                                err: e.to_string(),
                            });
                            return;
                        }
                    }
                }
            };

            match Player::spawn(&resolved_source, start_pos).await {
                Ok(player) => {
                    let _ = player.set_speed(speed).await;
                    let _ = player.set_volume(volume).await;
                    // Start position polling as independent task
                    let socket_path = player.socket_path.clone();
                    tokio::spawn(player::poll_position_loop(socket_path, pos_tx));
                    let _ = task_tx.send(TaskMsg::PlayerReady {
                        video_id,
                        player: Box::new(player),
                    });
                }
                Err(e) => {
                    let _ = task_tx.send(TaskMsg::PlayerError {
                        video_id,
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
            let (video_id, pct) = self.download_rx.borrow_and_update().clone();
            self.download_progress.insert(video_id, pct);
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
            TaskMsg::MetaReady { url, meta, target_path } => {
                self.pending_fetches = self.pending_fetches.saturating_sub(1);
                let video_id = meta.video_id.clone();
                info!(video_id = %video_id, title = %meta.title, "metadata ready, starting download");
                let status_title = meta.title.clone();

                let track = Track {
                    url: url.clone(),
                    source: meta.source,
                    title: meta.title,
                    artist: meta.artist,
                    channel: meta.channel,
                    duration: meta.duration,
                    video_id: meta.video_id,
                    cache_status: CacheStatus::Streaming,
                    file: None,
                    last_position: 0,
                    speed: None,
                    user_title: None,
                    user_artist: None,
                    added_at: Utc::now(),
                };

                // Resolve which playlist file this track actually belongs to. Always
                // recorded here, at add-time, regardless of whether it's the active
                // playlist — the user may switch playlists before the download
                // finishes, so `DownloadDone` must not have to guess the path from
                // whatever happens to be displayed at completion time.
                let owning_path = target_path.unwrap_or_else(|| self.playlist_path.clone());

                // When the target playlist path is not the currently displayed one, add
                // the track there instead of the currently displayed playlist.
                if owning_path != self.playlist_path {
                    match Playlist::load(&owning_path) {
                        Ok(mut target_pl) => {
                            target_pl.add_track(track);
                            if let Err(e) = target_pl.save(&owning_path) {
                                error!(err = %e, "failed to save target playlist after URL add");
                            }
                        }
                        Err(e) => {
                            error!(err = %e, path = %owning_path.display(), "target playlist not found, track not added");
                        }
                    }
                    self.set_status(format!("Added to playlist: {status_title}"));
                } else {
                    // Default: add to the active (displayed) playlist
                    self.playlist.tracks.push(track);
                    self.selected = self.playlist.tracks.len() - 1;
                    self.save_playlist();
                    self.set_status(format!("Added: {status_title}"));
                }
                self.downloading.insert(video_id.clone());
                // Remember which playlist file this download belongs to so
                // `DownloadDone` can patch the correct file on disk, even if the
                // user has since switched to browsing a different playlist.
                self.download_targets.insert(video_id.clone(), owning_path.clone());

                let task_tx = self.task_tx.clone();
                let dl_tx = self.download_tx.clone();
                let quality = self.config.audio_quality.clone();
                let audio_dir = cache::audio_dir();
                let vid = video_id.clone();
                tokio::spawn(async move {
                    match ytdlp::spawn_download(&url, &audio_dir, &vid, &quality, dl_tx).await {
                        Ok(file) => {
                            let _ = task_tx.send(TaskMsg::DownloadDone { video_id: vid, file });
                        }
                        Err(e) => {
                            let _ = task_tx.send(TaskMsg::DownloadError {
                                video_id: vid,
                                err: e.to_string(),
                            });
                        }
                    }
                });
            }

            TaskMsg::MetaError { url, err } => {
                self.pending_fetches = self.pending_fetches.saturating_sub(1);
                error!(url = %url, err = %err, "metadata fetch failed");
                self.set_status("Metadata fetch failed");
            }

            TaskMsg::DownloadDone { video_id, file } => {
                info!(video_id = %video_id, path = %file.display(), "download complete");
                self.downloading.remove(&video_id);
                self.download_progress.remove(&video_id);
                self.set_status("Download complete");

                // `download_targets` is always populated at add-time (see
                // `TaskMsg::MetaReady`), regardless of whether the track was added to
                // the active playlist or a different one — so this single call always
                // patches the file the track actually lives in, even if the user has
                // since switched to browsing something else. Fall back to the
                // displayed playlist's path defensively in case an entry is ever
                // missing (should not normally happen).
                let owning_path = self
                    .download_targets
                    .remove(&video_id)
                    .unwrap_or_else(|| self.playlist_path.clone());

                let file_for_patch = file.clone();
                self.patch_and_save_playlist(&owning_path, &video_id, move |track| {
                    track.cache_status = CacheStatus::Cached;
                    track.file = Some(file_for_patch);
                });

                // If this track is the one actually driving playback right now — per
                // `self.playing`, independent of whatever playlist is displayed — and
                // it was streaming, hot-switch mpv to the freshly downloaded local file.
                self.hot_switch_to_local_file(&owning_path, &video_id, file);
            }

            TaskMsg::DownloadError { video_id, err } => {
                error!(video_id = %video_id, err = %err, "download failed");
                self.downloading.remove(&video_id);
                self.download_progress.remove(&video_id);
                self.download_targets.remove(&video_id);
                self.set_status("Download failed");
            }

            TaskMsg::PlayerReady { video_id, player } => {
                // Ignore if user already switched to a different track
                let matches_playing = self
                    .playing
                    .as_ref()
                    .is_some_and(|p| p.track().video_id == video_id);
                if !matches_playing {
                    info!(video_id = %video_id, "player ready but track changed, discarding");
                    return;
                }
                info!(video_id = %video_id, "player started");
                self.player = Some(*player);
                self.is_paused = false;
                self.set_status("Player ready");
            }

            TaskMsg::PlayerError { video_id, err } => {
                error!(video_id = %video_id, err = %err, "player failed to start");
                self.set_status("Player error");
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
            self.filtered_indices = self
                .playlist
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.title.to_lowercase().contains(&query)
                        || t.artist.to_lowercase().contains(&query)
                        || t.user_title
                            .as_deref()
                            .map(|s| s.to_lowercase().contains(&query))
                            .unwrap_or(false)
                        || t.user_artist
                            .as_deref()
                            .map(|s| s.to_lowercase().contains(&query))
                            .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
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
                    let _ = task_tx.send(TaskMsg::MetaReady { url, meta, target_path });
                }
                Err(e) => {
                    let _ = task_tx.send(TaskMsg::MetaError { url, err: e.to_string() });
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
    /// No-op if nothing is playing. Routes the write through the same
    /// in-memory-vs-disk identity rule as the rest of this module: if the
    /// playing session's playlist file is the one currently displayed, the
    /// mutation goes through `self.playlist` (and `save_playlist`) so an
    /// in-progress edit to the displayed playlist isn't clobbered by a stale
    /// on-disk write; otherwise it's written directly to the playing
    /// session's own playlist file.
    pub fn flush_playing_position(&mut self) {
        if self.playing.is_none() {
            return;
        }
        let pos = self.position as u64;
        if let Some(track) = self.playing_track_mut() {
            track.last_position = pos;
        }
        self.save_playing_session_playlist();
    }

    /// Patch a single track (found by `video_id`) in the playlist at `path`
    /// and persist the change to disk — the general "mutate a track that
    /// might not be in the currently displayed playlist" mechanism.
    ///
    /// - If `path == self.playlist_path`, the track lives in the
    ///   already-loaded `self.playlist`: mutate it in place and save via
    ///   `self.save_playlist()` so in-memory and on-disk state stay in sync.
    /// - Otherwise, load the playlist at `path` from disk, mutate the track
    ///   there, and save it back to `path`. `self.playlist` (the displayed
    ///   playlist) is left untouched.
    /// - If no track with `video_id` exists in the target playlist, this is
    ///   a no-op (logged, not an error) — matches the existing style used by
    ///   the target-playlist branch this replaces.
    /// - Load/save errors are logged and cause an early return.
    pub fn patch_and_save_playlist(&mut self, path: &Path, video_id: &str, f: impl FnOnce(&mut Track)) {
        if path == self.playlist_path.as_path() {
            match self.playlist.tracks.iter_mut().find(|t| t.video_id == video_id) {
                Some(track) => f(track),
                None => {
                    warn!(video_id = %video_id, path = %path.display(), "patch_and_save_playlist: track not found in displayed playlist");
                    return;
                }
            }
            self.save_playlist();
            return;
        }

        let mut target_pl = match Playlist::load(path) {
            Ok(pl) => pl,
            Err(e) => {
                error!(err = %e, path = %path.display(), "patch_and_save_playlist: failed to load playlist");
                return;
            }
        };

        match target_pl.tracks.iter_mut().find(|t| t.video_id == video_id) {
            Some(track) => f(track),
            None => {
                warn!(video_id = %video_id, path = %path.display(), "patch_and_save_playlist: track not found");
                return;
            }
        }

        if let Err(e) = target_pl.save(path) {
            error!(err = %e, path = %path.display(), "patch_and_save_playlist: failed to save playlist");
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

        let video_id = self.playlist.tracks[track_idx].video_id.clone();

        // Resolve the target playlist path
        let target_path = self
            .available_playlists
            .iter()
            .find(|(n, _)| n == target_name)
            .map(|(_, p)| p.clone())
            .with_context(|| format!("target playlist '{target_name}' not found in available_playlists"))?;

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
        // video_id)`, not just a matching `video_id` that might coincidentally
        // also exist in an unrelated playing session elsewhere.
        let is_current = self.is_playing_track(&self.playlist_path, &video_id);
        if is_current {
            self.player = None; // Drop kills mpv process
            self.playing = None;
            self.playlist.current_track = None;
            self.is_paused = false;
            self.position = 0.0;
        }

        // Remove from source playlist
        let track = self
            .playlist
            .remove_track_by_video_id(&video_id)
            .with_context(|| format!("track '{video_id}' not found in source playlist"))?;

        // Append to target playlist
        target_playlist.add_track(track);

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

// ── Event loop ────────────────────────────────────────────────────────────

pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::init();

    loop {
        app.sync_channels();
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if input::handle_key(app, key).await? == input::Action::Quit {
                    break;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    app.flush_playing_position();

    ratatui::restore();
    Ok(())
}
