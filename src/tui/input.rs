use super::{App, Focus, InputMode, SettingsItem, SidebarItem, SETTINGS_ITEMS};
use crate::playlist::{LoopMode, Playlist, Track};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;
use tracing::{error, warn};

// #region agent log
fn agent_log(
    run_id: &str,
    hypothesis_id: &str,
    location: &str,
    message: &str,
    data: serde_json::Value,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let payload = serde_json::json!({
        "sessionId": "d28f88",
        "runId": run_id,
        "hypothesisId": hypothesis_id,
        "location": location,
        "message": message,
        "data": data,
        "timestamp": ts
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/Users/den/Projects/trovers/.cursor/debug-d28f88.log")
    {
        let _ = writeln!(f, "{}", payload);
    }
}
// #endregion agent log

#[derive(Debug, PartialEq)]
pub enum Action {
    Continue,
    Quit,
}

/// Top-level key dispatcher. Tab is handled first, always.
pub async fn handle_key(app: &mut App, key: KeyEvent) -> Result<Action> {
    // Ctrl+C, from any mode. Raw mode suppresses the terminal's own SIGINT
    // translation, so without this the keystroke is silently swallowed and the
    // user's only way out is to close the window — which used to strand mpv.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return Ok(Action::Quit);
    }

    // Help modal: only allow closing toggles while open.
    if app.input_mode == InputMode::Help {
        return handle_help(app, key);
    }

    // Global help toggle (only in non-text-input modes).
    if key.code == KeyCode::Char('?')
        && matches!(app.input_mode, InputMode::Normal | InputMode::ConfirmDelete)
    {
        app.input_mode = InputMode::Help;
        return Ok(Action::Continue);
    }

    // Tab switches focus regardless of mode (except when typing or in context menu).
    // In UrlInput mode, Tab cycles through available playlists instead.
    if key.code == KeyCode::Tab && app.input_mode == InputMode::UrlInput {
        app.cycle_url_target_playlist();
        return Ok(Action::Continue);
    }

    if key.code == KeyCode::Tab
        && !matches!(
            app.input_mode,
            InputMode::UrlInput
                | InputMode::NewPlaylist
                | InputMode::SearchInput
                | InputMode::TrackContextMenu
                | InputMode::PlaylistRename
                | InputMode::PlaylistDelete
        )
    {
        app.focus = match app.focus {
            Focus::Sidebar => Focus::TrackList,
            Focus::TrackList | Focus::Settings => Focus::Sidebar,
        };
        return Ok(Action::Continue);
    }

    match app.input_mode {
        InputMode::Normal => match app.focus {
            Focus::Sidebar => handle_sidebar(app, key).await,
            Focus::TrackList => handle_tracklist(app, key).await,
            Focus::Settings => handle_settings(app, key),
        },
        InputMode::UrlInput => handle_url_input(app, key).await,
        InputMode::NewPlaylist => handle_new_playlist(app, key).await,
        InputMode::SearchInput => handle_search(app, key),
        InputMode::ConfirmDelete => handle_confirm_delete(app, key),
        InputMode::TrackContextMenu => handle_track_context_menu(app, key),
        InputMode::PlaylistRename => handle_playlist_rename(app, key).await,
        InputMode::PlaylistDelete => handle_playlist_delete(app, key).await,
        InputMode::Help => Ok(Action::Continue),
    }
}

fn handle_help(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') => {
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Sidebar ───────────────────────────────────────────────────────────────

async fn handle_sidebar(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.sidebar_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.sidebar_next(),

        KeyCode::Enter => {
            let items = app.sidebar_items();
            if let Some(item) = items.get(app.sidebar_selected) {
                match item {
                    SidebarItem::PlaylistsHeader => {
                        app.playlists_expanded = !app.playlists_expanded;
                        let items = app.sidebar_items();
                        if !items
                            .get(app.sidebar_selected)
                            .map(|i| i.is_selectable())
                            .unwrap_or(false)
                        {
                            app.sidebar_selected = 0;
                        }
                    }
                    SidebarItem::Playlist { name, path } => {
                        let name = name.clone();
                        let path = path.clone();
                        if let Err(e) = app.switch_to_playlist(&name, &path) {
                            error!(err = %e, "failed to switch playlist");
                            // Still move focus to track list even on error so UX doesn't
                            // get stuck in the sidebar.
                            app.focus = Focus::TrackList;
                        }
                    }
                    SidebarItem::Plunder => {
                        app.input_mode = InputMode::UrlInput;
                        app.input_buf.clear();
                        app.target_playlist_for_url = Some(app.playlist.name.clone());
                        app.focus = Focus::TrackList;
                    }
                    SidebarItem::Settings => {
                        app.focus = Focus::Settings;
                        app.settings_selected = 0;
                    }
                    _ => {}
                }
            }
        }

        // Rename selected playlist (sidebar must be on a Playlist item)
        KeyCode::Char('r') => {
            let items = app.sidebar_items();
            if let Some(SidebarItem::Playlist { name, .. }) = items.get(app.sidebar_selected) {
                app.input_buf = name.clone();
                app.input_mode = InputMode::PlaylistRename;
            }
        }

        // Delete selected playlist (sidebar must be on a Playlist item)
        KeyCode::Char('d') => {
            let items = app.sidebar_items();
            if matches!(items.get(app.sidebar_selected), Some(SidebarItem::Playlist { .. })) {
                app.input_mode = InputMode::PlaylistDelete;
            }
        }

        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(Action::Quit);
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Track list ────────────────────────────────────────────────────────────

pub(crate) async fn handle_tracklist(app: &mut App, key: KeyEvent) -> Result<Action> {
    let count = app.visible_track_count();
    let visible = app.track_list_height as usize;

    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(Action::Quit);
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => {
            if app.selected > 0 {
                app.selected -= 1;
                app.clamp_scroll();
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if count > 0 && app.selected + 1 < count {
                app.selected += 1;
                app.clamp_scroll();
            }
        }
        KeyCode::Char('g') => {
            app.selected = 0;
            app.clamp_scroll();
        }
        KeyCode::Char('G') => {
            if count > 0 {
                app.selected = count - 1;
                app.clamp_scroll();
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let step = (visible / 2).max(1);
            app.selected = (app.selected + step).min(count.saturating_sub(1));
            app.clamp_scroll();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let step = (visible / 2).max(1);
            app.selected = app.selected.saturating_sub(step);
            app.clamp_scroll();
        }

        // Enter: select track and start playback (resuming from
        // `last_position` if the track has one).
        KeyCode::Enter => {
            if let Some(idx) = app.track_index_at(app.selected) {
                let start_pos = app.playlist.tracks.get(idx).and_then(resume_start_pos);
                app.request_playback(idx, start_pos);
            }
        }

        // Space: toggle pause if playing, otherwise start (resuming from
        // `last_position` if the track has one).
        KeyCode::Char(' ') => {
            if app.player.is_some() {
                app.is_paused = !app.is_paused;
                let pausing = app.is_paused;
                let res = match &app.player {
                    Some(player) => {
                        Some(if pausing { player.pause().await } else { player.resume().await })
                    }
                    None => None,
                };
                note_ipc_result(app, "pause", res);
            } else if let Some(idx) = app.track_index_at(app.selected) {
                let start_pos = app.playlist.tracks.get(idx).and_then(resume_start_pos);
                app.request_playback(idx, start_pos);
            }
        }

        // Seek
        KeyCode::Left => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) { -60 } else { -10 };
            let res = match &app.player {
                Some(player) => Some(player.seek(secs, "relative").await),
                None => None,
            };
            note_ipc_result(app, "seek", res);
        }
        KeyCode::Right => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) { 60 } else { 10 };
            let res = match &app.player {
                Some(player) => Some(player.seek(secs, "relative").await),
                None => None,
            };
            note_ipc_result(app, "seek", res);
        }

        // Speed: [ = slower, ] = faster. Adjusts the speed of the *playing*
        // track (which may live in a playlist other than the one displayed),
        // not whatever track happens to sit at `current_track_index()` in
        // the displayed playlist.
        KeyCode::Char(']') => {
            adjust_playing_track_speed(app, 0.1).await;
        }
        KeyCode::Char('[') => {
            adjust_playing_track_speed(app, -0.1).await;
        }

        // Volume
        KeyCode::Char('v') => {
            set_volume(app, 5).await;
        }
        KeyCode::Char('V') => {
            set_volume(app, -5).await;
        }

        // Loop mode
        KeyCode::Char('l') => {
            app.playlist.loop_mode = match app.playlist.loop_mode {
                LoopMode::None => LoopMode::Track,
                LoopMode::Track => LoopMode::Playlist,
                LoopMode::Playlist => LoopMode::None,
            };
        }

        // Next / Previous: always step relative to the cursor position in
        // the *displayed* playlist (`app.selected`), wrapping at its bounds,
        // and play the resulting displayed-playlist track — regardless of
        // where the currently-playing track (if any) actually lives. This
        // matches "browsing playlist X and pressing n/b walks X's tracks"
        // even while something else plays in the background.
        KeyCode::Char('n') => {
            let count = app.visible_track_count();
            if count > 0 {
                let next_cursor = (app.selected + 1) % count;
                app.selected = next_cursor;
                app.clamp_scroll();
                if let Some(idx) = app.track_index_at(next_cursor) {
                    let start_pos = app.playlist.tracks.get(idx).and_then(resume_start_pos);
                    app.request_playback(idx, start_pos);
                }
            }
        }
        KeyCode::Char('b') => {
            let count = app.visible_track_count();
            if count > 0 {
                let prev_cursor = app.selected.checked_sub(1).unwrap_or(count - 1);
                app.selected = prev_cursor;
                app.clamp_scroll();
                if let Some(idx) = app.track_index_at(prev_cursor) {
                    let start_pos = app.playlist.tracks.get(idx).and_then(resume_start_pos);
                    app.request_playback(idx, start_pos);
                }
            }
        }

        // Add URL
        KeyCode::Char('a') => {
            app.input_mode = InputMode::UrlInput;
            app.input_buf.clear();
            // Reset target to the currently active playlist
            app.target_playlist_for_url = Some(app.playlist.name.clone());
        }

        // Delete track
        KeyCode::Char('d') => {
            if !app.playlist.tracks.is_empty() {
                app.input_mode = InputMode::ConfirmDelete;
            }
        }

        // Search
        KeyCode::Char('/') => {
            app.input_mode = InputMode::SearchInput;
            app.input_buf.clear();
            app.filtered_indices.clear();
        }

        // New playlist
        KeyCode::Char('N') => {
            app.input_mode = InputMode::NewPlaylist;
            app.input_buf.clear();
        }

        // Move track to another playlist
        KeyCode::Char('m') => {
            if !app.playlist.tracks.is_empty() && !app.available_playlist_names().is_empty() {
                app.context_menu_selected = 0;
                app.input_mode = InputMode::TrackContextMenu;
            }
        }

        _ => {}
    }

    Ok(Action::Continue)
}

/// Adjust the speed of the track actually driving playback right now (per
/// `app.playing`), not whatever the displayed playlist's cursor happens to
/// point at — the playing track may live in a different playlist entirely.
/// No-op if nothing is playing. `delta` is added to the track's current
/// effective speed and clamped to mpv's supported range.
///
/// The speed is persisted to TOML even if mpv never receives it, so the setting
/// survives a dead player and applies the next time the track is started.
pub(crate) async fn adjust_playing_track_speed(app: &mut App, delta: f32) {
    if app.playing.is_none() {
        return;
    }
    let default_speed = app.config.default_speed;
    let playlist_default_speed = app.playing.as_ref().and_then(|p| p.playlist.default_speed);

    let speed = {
        let Some(track) = app.playing_track_mut() else {
            return;
        };
        let base = track.speed.or(playlist_default_speed).unwrap_or(default_speed);
        let new_speed = (base + delta).clamp(0.25, 3.0);
        track.speed = Some(new_speed);
        new_speed
    };

    let res = match &app.player {
        Some(player) => Some(player.set_speed(speed).await),
        None => None,
    };
    note_ipc_result(app, "speed", res);

    // Persist through whichever copy is the source of truth for the playing
    // track's identity: the displayed playlist (already the case when paths
    // match, since `playing_track_mut` mutated it directly) or the playing
    // session's own playlist file.
    app.save_playing_session_playlist();
}

/// Change the volume by `delta` and push it to mpv. The config value is updated
/// regardless of whether mpv is reachable, so the new level applies to the next
/// track even when the current player has already exited.
async fn set_volume(app: &mut App, delta: i16) {
    let vol = (app.config.default_volume as i16 + delta).clamp(0, 100) as u8;
    app.config.default_volume = vol;
    let res = match &app.player {
        Some(player) => Some(player.set_volume(vol).await),
        None => None,
    };
    note_ipc_result(app, "volume", res);
}

/// Absorb a failed mpv IPC call into a footer message instead of letting it
/// escape `handle_key`.
///
/// mpv runs without `--idle`, so it exits by itself at the end of a track and
/// `app.player` can briefly hold a socket nobody is listening on. Propagating
/// that error used to unwind the whole event loop and abort `main` — which is
/// what made the app "randomly crash", and discarded the session's unsaved
/// playlist edits along with it. Clearing the stale player is the job of the
/// `TaskMsg::PlayerGone` handler; there is nothing to do here but say so.
///
/// `None` means there was no player to talk to, which is not a failure.
fn note_ipc_result(app: &mut App, action: &str, res: Option<Result<()>>) {
    if let Some(Err(e)) = res {
        warn!(action, err = %e, "mpv IPC call failed");
        app.set_status(format!("Player not responding ({action})"));
    }
}

// ── Settings panel ────────────────────────────────────────────────────────

fn handle_settings(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(Action::Quit);
        }
        KeyCode::Tab | KeyCode::Esc => {
            app.focus = Focus::Sidebar;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.settings_selected > 0 {
                app.settings_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.settings_selected + 1 < SETTINGS_ITEMS.len() {
                app.settings_selected += 1;
            }
        }
        KeyCode::Left | KeyCode::Char('[') => {
            settings_change(app, -1);
        }
        KeyCode::Right | KeyCode::Char(']') => {
            settings_change(app, 1);
        }
        _ => {}
    }
    Ok(Action::Continue)
}

fn settings_change(app: &mut App, dir: i8) {
    match SETTINGS_ITEMS.get(app.settings_selected) {
        Some(SettingsItem::AudioQuality) => {
            if dir > 0 {
                app.quality_next();
            } else {
                app.quality_prev();
            }
        }
        Some(SettingsItem::DefaultSpeed) => {
            let step = 0.1 * dir as f32;
            app.config.default_speed = (app.config.default_speed + step).clamp(0.25, 3.0);
            let _ = app.config.save();
        }
        Some(SettingsItem::DefaultVolume) => {
            if dir > 0 {
                app.config.default_volume = app.config.default_volume.saturating_add(5).min(100);
            } else {
                app.config.default_volume = app.config.default_volume.saturating_sub(5);
            }
            let _ = app.config.save();
        }
        None => {}
    }
}

// ── Text input modes ──────────────────────────────────────────────────────

async fn handle_url_input(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let url = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;
            if !url.is_empty() {
                // Determine the target playlist path from target_playlist_for_url.
                // If it matches the active playlist (or is not set), use the default flow.
                let target_path = app
                    .target_playlist_for_url
                    .as_deref()
                    .and_then(|name| {
                        if name == app.playlist.name {
                            None // Same as active – use default path
                        } else {
                            app.available_playlists
                                .iter()
                                .find(|(n, _)| n == name)
                                .map(|(_, p)| p.clone())
                        }
                    });
                app.fetch_url_to(url, target_path);
            }
            // Reset target to current playlist for next invocation
            app.target_playlist_for_url = Some(app.playlist.name.clone());
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buf.clear();
            // Reset target on cancel too
            app.target_playlist_for_url = Some(app.playlist.name.clone());
        }
        _ => type_char(app, key),
    }
    Ok(Action::Continue)
}

async fn handle_new_playlist(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let name = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;
            if !name.is_empty() {
                if let Err(msg) = validate_playlist_name(&name, &app.available_playlists, None) {
                    warn!(msg = %msg, "invalid playlist name");
                    return Ok(Action::Continue);
                }
                match Playlist::create(&name) {
                    Ok((_, path)) => {
                        app.available_playlists.push((name, path));
                        app.available_playlists.sort_by(|a, b| a.0.cmp(&b.0));
                    }
                    Err(e) => {
                        error!(err = %e, "failed to create playlist");
                    }
                }
            }
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buf.clear();
        }
        _ => type_char(app, key),
    }
    Ok(Action::Continue)
}

fn handle_search(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            if key.code == KeyCode::Esc {
                app.input_buf.clear();
                app.filtered_indices.clear();
                app.selected = 0;
                app.track_offset = 0;
            }
            app.input_mode = InputMode::Normal;
        }
        _ => {
            type_char(app, key);
            app.update_search();
        }
    }
    Ok(Action::Continue)
}

pub(crate) fn handle_confirm_delete(app: &mut App, key: KeyEvent) -> Result<Action> {
    if key.code == KeyCode::Char('y') {
        if let Some(idx) = app.track_index_at(app.selected) {
            let video_id = app.playlist.tracks[idx].video_id.clone();
            // Only stop playback if the track being deleted is literally the
            // one actually driving playback right now (identity is `(path,
            // video_id)`) — not just any track with a matching video_id that
            // happens to exist in a differently-playing session elsewhere.
            let is_current = app.is_playing_track(&app.playlist_path, &video_id);

            if is_current {
                // Stop playback immediately when deleting current track
                app.stop_player(); // kills mpv and retires its position poller
                app.playing = None;
                app.playlist.current_track = None;
                app.is_paused = false;
            }

            let file_to_delete = app.playlist.tracks[idx].file.clone();
            app.playlist.tracks.remove(idx);
            // Clear any active search filter; stale indices would point to wrong tracks
            app.filtered_indices.clear();
            if app.selected >= app.visible_track_count() && app.selected > 0 {
                app.selected -= 1;
            }
            app.clamp_scroll();
            app.save_playlist();
            if let Some(path) = file_to_delete {
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!(path = %path.display(), err = %e, "failed to delete cached file");
                }
            }
        }
    }
    app.input_mode = InputMode::Normal;
    Ok(Action::Continue)
}

// ── Track context menu ────────────────────────────────────────────────────

fn handle_track_context_menu(app: &mut App, key: KeyEvent) -> Result<Action> {
    let names = app.available_playlist_names();
    let count = names.len();

    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if count > 0 && app.context_menu_selected > 0 {
                app.context_menu_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if count > 0 && app.context_menu_selected + 1 < count {
                app.context_menu_selected += 1;
            }
        }
        KeyCode::Enter => {
            // Perform the actual move
            let names = app.available_playlist_names();
            if let Some(target_name) = names.get(app.context_menu_selected).cloned() {
                if let Err(e) = app.move_track_to_playlist(&target_name) {
                    error!(err = %e, "failed to move track");
                }
            }
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Playlist rename ───────────────────────────────────────────────────────

pub(crate) async fn handle_playlist_rename(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let new_name = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;

            if new_name.is_empty() {
                return Ok(Action::Continue);
            }

            // Find which playlist is selected in the sidebar
            let items = app.sidebar_items();
            let selected_item = items.get(app.sidebar_selected).cloned();
            if let Some(SidebarItem::Playlist { name: old_name, path: old_path }) = selected_item {
                // #region agent log
                agent_log(
                    "pre",
                    "C",
                    "src/tui/input.rs:playlist_rename_enter",
                    "rename playlist requested",
                    serde_json::json!({
                        "old_name": old_name,
                        "old_path": old_path.display().to_string(),
                        "new_name": new_name,
                        "app_active_playlist_name": app.playlist.name,
                        "app_playlist_path": app.playlist_path.display().to_string(),
                        "config_active_playlist": app.config.active_playlist,
                    }),
                );
                // #endregion agent log

                // Validate: no duplicate name
                if let Err(msg) = validate_playlist_name(&new_name, &app.available_playlists, Some(&old_name)) {
                    warn!(msg = %msg, "invalid playlist name");
                    return Ok(Action::Continue);
                }

                let mut playlist = match Playlist::load(&old_path) {
                    Ok(p) => p,
                    Err(e) => {
                        error!(err = %e, "failed to load playlist for rename");
                        return Ok(Action::Continue);
                    }
                };

                match playlist.rename(&new_name, &old_path) {
                    Ok(new_path) => {
                        let new_path_for_log = new_path.display().to_string();
                        // Update available_playlists entry
                        for entry in &mut app.available_playlists {
                            if entry.0 == old_name {
                                entry.0 = new_name.clone();
                                entry.1 = new_path.clone();
                                break;
                            }
                        }
                        app.available_playlists.sort_by(|a, b| a.0.cmp(&b.0));

                        // Re-anchor sidebar_selected to the renamed playlist's new position.
                        // sidebar_items() starts with PlaylistsHeader at index 0, so playlist
                        // entries begin at index 1 when expanded.
                        if let Some(new_pos) = app.available_playlists.iter().position(|(n, _)| n == &new_name) {
                            app.sidebar_selected = 1 + new_pos; // +1 for PlaylistsHeader
                        }

                        // If the playing session belongs to the renamed playlist file,
                        // re-point it at the new path so future saves (flush_playing_position,
                        // adjust_playing_track_speed, request_playback's leaving-track save)
                        // target the file that now actually exists on disk, instead of
                        // resurrecting the just-deleted `old_path`.
                        if let Some(session) = app.playing.as_mut() {
                            if session.path == old_path {
                                session.path = new_path.clone();
                            }
                        }

                        // If we just renamed the active playlist, update playlist_path too
                        if app.playlist.name == old_name {
                            app.playlist.name = new_name.clone();
                            app.playlist_path = new_path;
                        }

                        // #region agent log
                        agent_log(
                            "pre",
                            "C",
                            "src/tui/input.rs:playlist_rename_done",
                            "rename playlist completed",
                            serde_json::json!({
                                "old_name": old_name,
                                "new_name": new_name,
                                "new_path": new_path_for_log,
                                "app_active_playlist_name_now": app.playlist.name,
                                "app_playlist_path_now": app.playlist_path.display().to_string(),
                                "config_active_playlist_now": app.config.active_playlist,
                            }),
                        );
                        // #endregion agent log
                    }
                    Err(e) => {
                        error!(err = %e, "failed to rename playlist");
                    }
                }
            }
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buf.clear();
        }
        _ => type_char(app, key),
    }
    Ok(Action::Continue)
}

// ── Playlist delete ───────────────────────────────────────────────────────

pub(crate) async fn handle_playlist_delete(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.input_mode = InputMode::Normal;

            let items = app.sidebar_items();
            let selected_item = items.get(app.sidebar_selected).cloned();
            if let Some(SidebarItem::Playlist { name, path }) = selected_item {
                // Don't allow deleting the active playlist
                if app.playlist.name == name {
                    warn!("cannot delete the currently active playlist");
                    return Ok(Action::Continue);
                }

                // If the playlist being deleted is the one `app.playing` points at
                // (even though it's not the *displayed* playlist), stop playback
                // first — otherwise the file gets removed out from under a live
                // session, and a later save (flush_playing_position, etc.) would
                // resurrect the just-deleted file with a stale snapshot.
                let deleting_playing_playlist = app.playing.as_ref().is_some_and(|p| p.path == path);
                if deleting_playing_playlist {
                    app.stop_player(); // kills mpv and retires its position poller
                    app.playing = None;
                    app.is_paused = false;
                }

                match Playlist::delete(&path) {
                    Ok(()) => {
                        app.available_playlists.retain(|(n, _)| n != &name);
                        // Move sidebar selection up if needed
                        let new_items = app.sidebar_items();
                        if app.sidebar_selected >= new_items.len() {
                            app.sidebar_selected = new_items.len().saturating_sub(1);
                        }
                        // Ensure selection lands on a selectable item.
                        // Prefer the nearest item at-or-before the cursor; if none exists,
                        // fall forward to the first selectable item after the cursor.
                        let at_or_before = new_items
                            .iter()
                            .enumerate()
                            .rev()
                            .find(|(i, item)| *i <= app.sidebar_selected && item.is_selectable());
                        if let Some((i, _)) = at_or_before {
                            app.sidebar_selected = i;
                        } else {
                            // Nothing selectable before cursor — find the first one after.
                            let after = new_items
                                .iter()
                                .enumerate()
                                .find(|(i, item)| *i > app.sidebar_selected && item.is_selectable());
                            app.sidebar_selected = after.map(|(i, _)| i).unwrap_or(0);
                        }
                    }
                    Err(e) => {
                        error!(err = %e, "failed to delete playlist");
                    }
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Playlist name validation ──────────────────────────────────────────────

/// Validate a playlist name.
/// Returns `Ok(())` if valid, `Err(message)` describing the problem.
/// `existing` is the list of current playlist (name, path) pairs.
/// `exclude` is an optional name to skip during duplicate check (used for rename).
pub(crate) fn validate_playlist_name(
    name: &str,
    existing: &[(String, std::path::PathBuf)],
    exclude: Option<&str>,
) -> std::result::Result<(), String> {
    if name.is_empty() {
        return Err("playlist name cannot be empty".to_string());
    }
    // Reject names with filesystem-unfriendly characters
    let invalid_chars = ['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];
    if name.chars().any(|c| invalid_chars.contains(&c)) {
        return Err("playlist name contains invalid character".to_string());
    }
    // Reject names that are purely whitespace or dots
    if name.trim().is_empty() || name == "." || name == ".." {
        return Err("playlist name is not valid".to_string());
    }
    // Check for duplicate
    let is_duplicate = existing
        .iter()
        .any(|(n, _)| n == name && exclude.map_or(true, |ex| n != ex));
    if is_duplicate {
        return Err(format!("a playlist named '{name}' already exists"));
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Decide the resume position to pass to `request_playback` for a
/// user-initiated play of `track`: resume from `Track.last_position` if it's
/// non-zero (meaning we've previously left off somewhere mid-track), else
/// start fresh from the beginning.
pub(crate) fn resume_start_pos(track: &Track) -> Option<f64> {
    if track.last_position > 0 {
        Some(track.last_position as f64)
    } else {
        None
    }
}

fn type_char(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) => app.input_buf.push(c),
        KeyCode::Backspace => {
            app.input_buf.pop();
        }
        _ => {}
    }
}
