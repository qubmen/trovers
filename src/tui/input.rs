use super::{App, Focus, InputMode, SidebarItem, SETTINGS_ITEMS};
use crate::playlist::Playlist;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, PartialEq)]
pub enum Action {
    Continue,
    Quit,
}

/// Top-level key dispatcher. Tab is handled first, always.
pub async fn handle_key(app: &mut App, key: KeyEvent) -> Result<Action> {
    // Tab switches focus regardless of mode (except when typing)
    if key.code == KeyCode::Tab
        && !matches!(
            app.input_mode,
            InputMode::UrlInput | InputMode::NewPlaylist | InputMode::SearchInput | InputMode::TrackContextMenu
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
    }
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
                        let items2 = app.sidebar_items();
                        if !items2
                            .get(app.sidebar_selected)
                            .map(|i| i.is_selectable())
                            .unwrap_or(false)
                        {
                            app.sidebar_selected = 0;
                        }
                    }
                    SidebarItem::Playlist { name, path } => {
                        // TODO: load playlist from path, replace app.playlist
                        let _name = name.clone();
                        let _path = path.clone();
                        app.focus = Focus::TrackList;
                    }
                    SidebarItem::Plunder => {
                        app.input_mode = InputMode::UrlInput;
                        app.input_buf.clear();
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

        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(Action::Quit);
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Track list ────────────────────────────────────────────────────────────

async fn handle_tracklist(app: &mut App, key: KeyEvent) -> Result<Action> {
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

        // Enter: select track and start playback
        KeyCode::Enter => {
            if let Some(idx) = app.track_index_at(app.selected) {
                app.request_playback(idx, None);
            }
        }

        // Space: toggle pause if playing, otherwise start
        KeyCode::Char(' ') => {
            if app.player.is_some() {
                app.is_paused = !app.is_paused;
                if app.is_paused {
                    app.player.as_ref().unwrap().pause().await?;
                } else {
                    app.player.as_ref().unwrap().resume().await?;
                }
            } else {
                let idx = app.current_track_index()
                    .or_else(|| app.track_index_at(app.selected));
                if let Some(idx) = idx {
                    app.request_playback(idx, None);
                }
            }
        }

        // Seek
        KeyCode::Left => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) { -60 } else { -10 };
            if let Some(player) = &app.player {
                player.seek(secs, "relative").await?;
            }
        }
        KeyCode::Right => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) { 60 } else { 10 };
            if let Some(player) = &app.player {
                player.seek(secs, "relative").await?;
            }
        }

        // Speed: [ = slower, ] = faster
        KeyCode::Char(']') => {
            if let Some(idx) = app.current_track_index() {
                let base = app.playlist.tracks[idx].speed
                    .or(app.playlist.default_speed)
                    .unwrap_or(app.config.default_speed);
                let speed = (base + 0.1).min(3.0);
                app.playlist.tracks[idx].speed = Some(speed);
                if let Some(player) = &app.player {
                    player.set_speed(speed).await?;
                }
                app.save_playlist();
            }
        }
        KeyCode::Char('[') => {
            if let Some(idx) = app.current_track_index() {
                let base = app.playlist.tracks[idx].speed
                    .or(app.playlist.default_speed)
                    .unwrap_or(app.config.default_speed);
                let speed = (base - 0.1).max(0.25);
                app.playlist.tracks[idx].speed = Some(speed);
                if let Some(player) = &app.player {
                    player.set_speed(speed).await?;
                }
                app.save_playlist();
            }
        }

        // Volume
        KeyCode::Char('v') => {
            let vol = app.config.default_volume.saturating_add(5).min(100);
            app.config.default_volume = vol;
            if let Some(player) = &app.player {
                player.set_volume(vol).await?;
            }
        }
        KeyCode::Char('V') => {
            let vol = app.config.default_volume.saturating_sub(5);
            app.config.default_volume = vol;
            if let Some(player) = &app.player {
                player.set_volume(vol).await?;
            }
        }

        // Loop mode
        KeyCode::Char('l') => {
            use crate::playlist::LoopMode;
            app.playlist.loop_mode = match app.playlist.loop_mode {
                LoopMode::None => LoopMode::Track,
                LoopMode::Track => LoopMode::Playlist,
                LoopMode::Playlist => LoopMode::None,
            };
        }

        // Next / Previous
        KeyCode::Char('n') => {
            let len = app.playlist.tracks.len();
            if len > 0 {
                let cur = app.current_track_index().unwrap_or(0);
                let next = (cur + 1) % len;
                app.selected = next;
                app.clamp_scroll();
                app.request_playback(next, None);
            }
        }
        KeyCode::Char('b') => {
            let len = app.playlist.tracks.len();
            if len > 0 {
                let cur = app.current_track_index().unwrap_or(0);
                let prev = cur.checked_sub(1).unwrap_or(len - 1);
                app.selected = prev;
                app.clamp_scroll();
                app.request_playback(prev, None);
            }
        }

        // Add URL
        KeyCode::Char('a') => {
            app.input_mode = InputMode::UrlInput;
            app.input_buf.clear();
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
    use super::SettingsItem;
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
                // TODO: playlist selection — for now always adds to current playlist
                app.fetch_url(url);
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

async fn handle_new_playlist(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let name = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;
            if !name.is_empty() {
                match Playlist::create(&name) {
                    Ok((_, path)) => {
                        app.available_playlists.push((name, path));
                        app.available_playlists.sort_by(|a, b| a.0.cmp(&b.0));
                    }
                    Err(e) => {
                        tracing::error!(err = %e, "failed to create playlist");
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

fn handle_confirm_delete(app: &mut App, key: KeyEvent) -> Result<Action> {
    if key.code == KeyCode::Char('y') {
        if let Some(idx) = app.track_index_at(app.selected) {
            let is_current = app.playlist.current_track.as_deref()
                == Some(app.playlist.tracks[idx].video_id.as_str());

            if is_current {
                // Stop playback immediately when deleting current track
                app.player = None;  // Drop implementation kills mpv process
                app.playlist.current_track = None;
                app.is_paused = false;
            }

            let file_to_delete = app.playlist.tracks[idx].file.clone();
            app.playlist.tracks.remove(idx);
            if app.selected >= app.visible_track_count() && app.selected > 0 {
                app.selected -= 1;
            }
            app.clamp_scroll();
            app.save_playlist();
            if let Some(path) = file_to_delete {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::warn!(path = %path.display(), err = %e, "failed to delete cached file");
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
            // Selection confirmed: close menu (actual move will be implemented in Task 2)
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
    Ok(Action::Continue)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn type_char(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(c) => app.input_buf.push(c),
        KeyCode::Backspace => {
            app.input_buf.pop();
        }
        _ => {}
    }
}
