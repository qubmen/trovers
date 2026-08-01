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
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

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

// ── App ───────────────────────────────────────────────────────────────────

pub struct App {
    // Playlist & config
    pub playlist: Playlist,
    pub playlist_path: PathBuf,
    pub config: Config,
    pub player: Option<Player>,

    // Async channels
    pub pos_tx: watch::Sender<f64>,
    pub position_rx: watch::Receiver<f64>,
    pub download_tx: watch::Sender<f32>,
    pub download_rx: watch::Receiver<f32>,
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
    pub download_progress: f32,
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
        let (download_tx, download_rx) = watch::channel(0.0f32);
        let (task_tx, task_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            playlist,
            playlist_path,
            config,
            player: None,
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
            download_progress: 0.0,
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

    pub fn current_track_index(&self) -> Option<usize> {
        let id = self.playlist.current_track.as_deref()?;
        self.playlist.tracks.iter().position(|t| t.video_id == id)
    }

    /// Start playback of the track at Vec index `idx`.
    /// `start_pos`: resume at this position in seconds (used when switching
    /// from stream to local file mid-play; pass `None` for a fresh start).
    pub fn request_playback(&mut self, idx: usize, start_pos: Option<f64>) {
        // Collect all track data before any mutations (borrow checker)
        let (video_id, url, speed, source) = {
            let Some(track) = self.playlist.tracks.get(idx) else {
                return;
            };
            let video_id = track.video_id.clone();
            let url = track.url.clone();
            let speed = track.speed
                .or(self.playlist.default_speed)
                .unwrap_or(self.config.default_speed);
            let source = match (&track.cache_status, &track.file) {
                (CacheStatus::Cached, Some(file)) => PlaySource::File(file.clone()),
                _ => PlaySource::Stream(url.clone()),
            };
            (video_id, url, speed, source)
        };

        // Save position of the track we're leaving (not applicable when switching
        // within the same track, e.g. stream → local file)
        if let Some(cur_idx) = self.current_track_index() {
            if cur_idx != idx {
                self.playlist.tracks[cur_idx].last_position = self.position as u64;
            }
        }

        let volume = self.config.default_volume;
        let quality = self.config.audio_quality.clone();
        let _ = url; // consumed via source

        self.playlist.current_track = Some(video_id.clone());
        self.is_paused = false;
        // Only reset the position display on a fresh start, not on stream→file switch
        if start_pos.is_none() {
            self.position = 0.0;
            let _ = self.pos_tx.send(0.0);
        }

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
            self.download_progress = *self.download_rx.borrow_and_update();
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

                // When a non-active target playlist path is set, add the track there
                // instead of the currently displayed playlist.
                if let Some(p) = target_path.as_deref().filter(|p| *p != self.playlist_path.as_path()) {
                    match Playlist::load(p) {
                        Ok(mut target_pl) => {
                            target_pl.add_track(track);
                            if let Err(e) = target_pl.save(p) {
                                error!(err = %e, "failed to save target playlist after URL add");
                            }
                        }
                        Err(e) => {
                            error!(err = %e, path = %p.display(), "target playlist not found, track not added");
                        }
                    }
                    self.downloading.insert(video_id.clone());
                    self.set_status(format!("Added to playlist: {status_title}"));
                    // Remember that this download belongs to the non-active playlist so
                    // DownloadDone can update the correct file on disk.
                    self.download_targets.insert(video_id.clone(), p.to_path_buf());
                } else {
                    // Default: add to the active playlist
                    self.playlist.tracks.push(track);
                    self.selected = self.playlist.tracks.len() - 1;
                    self.downloading.insert(video_id.clone());
                    self.save_playlist();
                    self.set_status(format!("Added: {status_title}"));
                }

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
                let _ = self.download_tx.send(0.0);
                self.set_status("Download complete");

                // Check whether this download was for a non-active playlist.
                if let Some(target_path) = self.download_targets.remove(&video_id) {
                    // Update the track in the target playlist file on disk.
                    match Playlist::load(&target_path) {
                        Ok(mut target_pl) => {
                            if let Some(track) =
                                target_pl.tracks.iter_mut().find(|t| t.video_id == video_id)
                            {
                                track.cache_status = CacheStatus::Cached;
                                track.file = Some(file);
                            }
                            if let Err(e) = target_pl.save(&target_path) {
                                error!(err = %e, "failed to save target playlist after download done");
                            }
                        }
                        Err(e) => {
                            error!(err = %e, path = %target_path.display(),
                                "failed to load target playlist after download done");
                        }
                    }
                    return;
                }

                // Active-playlist path: update in-memory state and save.
                let idx = self.playlist.tracks.iter().position(|t| t.video_id == video_id);

                if let Some(track) =
                    self.playlist.tracks.iter_mut().find(|t| t.video_id == video_id)
                {
                    track.cache_status = CacheStatus::Cached;
                    track.file = Some(file);
                }
                self.save_playlist();

                // If this track is currently streaming, switch mpv to the local file
                let is_current = self.playlist.current_track.as_deref() == Some(&video_id);
                if is_current && self.player.is_some() {
                    if let Some(idx) = idx {
                        let pos = self.position;
                        info!(video_id = %video_id, pos = pos, "switching stream → local file");
                        self.request_playback(idx, Some(pos));
                    }
                }
            }

            TaskMsg::DownloadError { video_id, err } => {
                error!(video_id = %video_id, err = %err, "download failed");
                self.downloading.remove(&video_id);
                self.set_status("Download failed");
            }

            TaskMsg::PlayerReady { video_id, player } => {
                // Ignore if user already switched to a different track
                if self.playlist.current_track.as_deref() != Some(&video_id) {
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

    /// Switch the active playlist to the one at `path` with the given `name`.
    ///
    /// - Stops any active playback (drops the player).
    /// - Loads the playlist from disk; returns an error on failure.
    /// - Resets track selection, scroll offset, and search filter state.
    /// - Updates `playlist_path` to the new path.
    /// - Switches focus to the track list so the user can browse the new playlist.
    pub fn switch_to_playlist(&mut self, name: &str, path: &std::path::Path) -> anyhow::Result<()> {
        use anyhow::Context as _;

        let new_playlist = Playlist::load(path)
            .with_context(|| format!("failed to load playlist '{name}' from {}", path.display()))?;

        // Stop any active playback
        self.player = None;
        self.is_paused = false;
        self.position = 0.0;
        let _ = self.pos_tx.send(0.0);

        // Replace playlist state
        self.playlist = new_playlist;
        self.playlist_path = path.to_path_buf();

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

        // Stop playback if the track being moved is the current one
        let is_current = self.playlist.current_track.as_deref() == Some(&video_id);
        if is_current {
            self.player = None; // Drop kills mpv process
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

    ratatui::restore();
    Ok(())
}
