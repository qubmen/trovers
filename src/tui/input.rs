use super::{App, Focus, InputMode, SettingsItem, SidebarItem, SETTINGS_ITEMS};
use crate::library;
use crate::library::Track;
use crate::library_import;
use crate::playlist::{LoopMode, Playlist, PlaylistEntry, PlaylistKind};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tracing::{error, info, warn};

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

    // In FolderInput mode, Tab cycles the explicit add-target instead —
    // "Auto" (today's folder-identity matching) plus every playlist/album by
    // name, so a folder or single file can be pointed at any existing list.
    if key.code == KeyCode::Tab && app.input_mode == InputMode::FolderInput {
        app.cycle_add_target();
        return Ok(Action::Continue);
    }

    if key.code == KeyCode::Tab
        && !matches!(
            app.input_mode,
            InputMode::UrlInput
                | InputMode::NewPlaylist
                | InputMode::NewAlbum
                | InputMode::SearchInput
                | InputMode::TrackContextMenu
                | InputMode::PlaylistRename
                | InputMode::PlaylistDelete
                | InputMode::AlbumRename
                | InputMode::AlbumDelete
                | InputMode::FolderInput
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
        InputMode::NewAlbum => handle_new_album(app, key),
        InputMode::SearchInput => handle_search(app, key),
        InputMode::ConfirmDelete => handle_confirm_delete(app, key),
        InputMode::TrackContextMenu => handle_track_context_menu(app, key),
        InputMode::PlaylistRename => handle_playlist_rename(app, key).await,
        InputMode::PlaylistDelete => handle_playlist_delete(app, key).await,
        InputMode::AlbumRename => handle_album_rename(app, key),
        InputMode::AlbumDelete => handle_album_delete(app, key),
        InputMode::FolderInput => handle_folder_input(app, key),
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
                    SidebarItem::Playlist { name, path, .. } => {
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
                    SidebarItem::ImportFolder => {
                        app.input_mode = InputMode::FolderInput;
                        app.input_buf.clear();
                        app.target_list_for_add = None;
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
            if matches!(
                items.get(app.sidebar_selected),
                Some(SidebarItem::Playlist { .. })
            ) {
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
            // On an album header this folds or unfolds the group instead: a header
            // names a list, and there is nothing to play about a list.
            if let Some(album) = app.album_of(app.selected) {
                app.toggle_album(album);
            } else {
                app.play_row(app.selected);
            }
        }

        // Space: pause/resume if the row under the cursor is the one actually
        // playing right now; otherwise switch playback to it — a track row
        // starts itself, a header starts its album (resuming from
        // `current_track`/`last_position` if it has one, otherwise its first
        // track). This is what lets Space on a *different* album's header
        // switch playback to it, the same way Enter already does for a track
        // row, while Space on the header of the album that's already playing
        // pauses instead of restarting it from its last saved position.
        //
        // A dead player (mpv exited, crashed, or was killed) never counts as
        // "already playing" even if `self.playing` still names this exact
        // row — ADR-013 leaves that pointer in place so the footer can show
        // what was last playing, but there is nothing left to pause, so
        // Space (re)starts it instead, same as it always has.
        KeyCode::Char(' ') => {
            let target = match app.row_at(app.selected).copied() {
                Some(crate::tui::VisibleRow::Track { source, index }) => {
                    let start_pos = app
                        .source_playlist(source)
                        .and_then(|(pl, _)| pl.tracks.get(index))
                        .and_then(|id| app.library.get(id))
                        .and_then(resume_start_pos);
                    Some((source, index, start_pos))
                }
                Some(crate::tui::VisibleRow::AlbumHeader { album }) => app
                    .albums
                    .get(album)
                    .and_then(|loaded| crate::tui::album_resume_target(loaded, &app.library))
                    .map(|(idx, start_pos)| (crate::tui::RowSource::Album(album), idx, start_pos)),
                None => None,
            };

            if let Some((source, index, start_pos)) = target {
                let already_playing = app.player.is_some()
                    && app
                        .source_playlist(source)
                        .and_then(|(pl, path)| {
                            pl.tracks
                                .get(index)
                                .map(|id| (path.to_path_buf(), id.clone()))
                        })
                        .is_some_and(|(path, id)| app.is_playing_track(&path, &id));

                if already_playing {
                    app.is_paused = !app.is_paused;
                    let pausing = app.is_paused;
                    let res = match &app.player {
                        Some(player) => Some(if pausing {
                            player.pause().await
                        } else {
                            player.resume().await
                        }),
                        None => None,
                    };
                    note_ipc_result(app, "pause", res);
                } else {
                    app.play_from_list(source, index, start_pos);
                }
            }
        }

        // Seek
        KeyCode::Left => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) {
                -60
            } else {
                -10
            };
            let res = match &app.player {
                Some(player) => Some(player.seek(secs, "relative").await),
                None => None,
            };
            note_ipc_result(app, "seek", res);
        }
        KeyCode::Right => {
            let secs = if key.modifiers.contains(KeyModifiers::SHIFT) {
                60
            } else {
                10
            };
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

        // Loop mode: acts on the list the selected row belongs to — an
        // album's own file when the row is inside one, the displayed
        // playlist otherwise (also on a header, which belongs to no list).
        // Persisted immediately either way: relying on the save at quit meant
        // the setting was lost to any exit that did not run it.
        KeyCode::Char('l') => match row_owning_list(app) {
            Some(path) => {
                let _ = app.with_list_at(&path, |pl, _lib| {
                    pl.loop_mode = cycle_loop_mode(&pl.loop_mode);
                });
            }
            None => {
                app.playlist.loop_mode = cycle_loop_mode(&app.playlist.loop_mode);
                app.save_playlist();
            }
        },

        // On an album header, rename that album. Anywhere else, toggle
        // shuffle for the list the selected row belongs to — an album's own
        // file when the row is inside one, the displayed playlist otherwise.
        KeyCode::Char('r') => {
            if let Some(album) = app.album_of(app.selected) {
                app.input_buf = app.albums[album].name.clone();
                app.input_mode = InputMode::AlbumRename;
            } else {
                // Grabbed before the mutation, since `with_list_at` may
                // rebuild `self.rows` and there is no need to re-derive it
                // afterwards.
                let album_name = app
                    .row_group(app.selected)
                    .and_then(|(source, _)| match source {
                        crate::tui::RowSource::Album(album) => {
                            app.albums.get(album).map(|loaded| loaded.name.clone())
                        }
                        crate::tui::RowSource::Own => None,
                    });
                match row_owning_list(app) {
                    Some(path) => {
                        let toggled = app.with_list_at(&path, |pl, _lib| {
                            pl.shuffle = !pl.shuffle;
                            (pl.shuffle, pl.tracks.len())
                        });
                        if let Ok((shuffle, len)) = toggled {
                            app.rebuild_shuffle_order_for(&path, shuffle, len);
                            app.set_status(match (shuffle, album_name) {
                                (true, Some(name)) => format!("Shuffle on for {name}"),
                                (false, Some(name)) => format!("Shuffle off for {name}"),
                                (true, None) => "Shuffle on".to_string(),
                                (false, None) => "Shuffle off".to_string(),
                            });
                        }
                    }
                    None => {
                        app.playlist.shuffle = !app.playlist.shuffle;
                        app.rebuild_shuffle_order();
                        app.save_playlist();
                        app.set_status(if app.playlist.shuffle {
                            "Shuffle on"
                        } else {
                            "Shuffle off"
                        });
                    }
                }
            }
        }

        KeyCode::Char('n') => step_track(app, true),
        KeyCode::Char('b') => step_track(app, false),

        // Add URL
        KeyCode::Char('a') => {
            app.input_mode = InputMode::UrlInput;
            app.input_buf.clear();
            // Reset target to the currently active playlist
            app.target_playlist_for_url = Some(app.playlist.name.clone());
        }

        // Delete: the album on a header, otherwise the track under the cursor.
        KeyCode::Char('d') => {
            if app.album_of(app.selected).is_some() {
                app.input_mode = InputMode::AlbumDelete;
            } else if app.row_track_id(app.selected).is_some() {
                app.input_mode = InputMode::ConfirmDelete;
            }
        }

        // Recache: force a fresh download of the selected track, whatever its
        // current cache status.
        KeyCode::Char('c') => {
            if app.album_of(app.selected).is_some() {
                app.set_status("Nothing to recache for an album");
            } else if let Some(id) = app.row_track_id(app.selected) {
                app.recache_track(&id);
            }
        }

        // Search
        KeyCode::Char('/') => {
            app.input_mode = InputMode::SearchInput;
            app.input_buf.clear();
            app.clear_search();
        }

        // New playlist
        KeyCode::Char('N') => {
            app.input_mode = InputMode::NewPlaylist;
            app.input_buf.clear();
        }

        // Import a local folder as an album
        KeyCode::Char('F') => {
            app.input_mode = InputMode::FolderInput;
            app.input_buf.clear();
            app.target_list_for_add = None;
        }

        // New album under the displayed playlist, with no folder required —
        // the manual counterpart to `F`.
        KeyCode::Char('A') => {
            app.input_mode = InputMode::NewAlbum;
            app.input_buf.clear();
        }

        // Rescan a folder: the one the album under the cursor mirrors when the
        // cursor is on a header, otherwise the displayed playlist's own.
        KeyCode::Char('R') => match app.album_of(app.selected) {
            Some(album) => app.rescan_album(album),
            None => match app.playlist.source_folder.clone() {
                Some(root) => app.import_folder(root, None),
                None => app.set_status("Not linked to a folder"),
            },
        },

        // Reorder the selected row within this playlist
        KeyCode::Char('J') => app.move_selected_row(true),
        KeyCode::Char('K') => app.move_selected_row(false),

        // Move track to another playlist
        KeyCode::Char('m') => {
            if app.album_of(app.selected).is_some() {
                // A header *is* a list. Opening the menu here would move whichever
                // row happened to sit under it, which is not what was asked.
                app.set_status("Move tracks, not albums");
            } else if app.row_track_id(app.selected).is_some()
                && !app.available_playlist_names().is_empty()
            {
                app.context_menu_selected = 0;
                app.input_mode = InputMode::TrackContextMenu;
            }
        }

        _ => {}
    }

    Ok(Action::Continue)
}

/// The file backing the list the selected row belongs to — an album's own
/// path when the row is inside one, the displayed playlist's path otherwise.
/// `None` on a header or when nothing is selected, which belong to no list.
fn row_owning_list(app: &App) -> Option<std::path::PathBuf> {
    let (source, _) = app.row_group(app.selected)?;
    app.source_playlist(source)
        .map(|(_, path)| path.to_path_buf())
}

fn cycle_loop_mode(mode: &LoopMode) -> LoopMode {
    match mode {
        LoopMode::None => LoopMode::Track,
        LoopMode::Track => LoopMode::Playlist,
        LoopMode::Playlist => LoopMode::None,
    }
}

/// `n` / `b`: step to the next/previous track of the *displayed* playlist and
/// play it, moving the cursor with it — regardless of where the currently
/// playing track (if any) actually lives. This is what makes "browsing playlist
/// X and pressing n/b walks X's tracks" hold even while something else plays in
/// the background. Both directions wrap at the bounds.
///
/// Shuffle applies only when no search filter is active. The visible rows under
/// a filter are already a deliberate subset in a deliberate order, and hopping
/// around inside it at random reads as a bug rather than a feature — so a filter
/// steps sequentially through what it shows, and shuffle resumes once cleared.
fn step_track(app: &mut App, forward: bool) {
    // The rows of the list the cursor's row belongs to — the parent's own tracks,
    // or one album's. Stepping never crosses that boundary: from an album's last
    // track `n` wraps to its first (ADR-019). A header belongs to no list, so
    // there is nothing to step.
    let Some((source, group)) = app.row_group(app.selected) else {
        return;
    };
    let Some(pos) = group.iter().position(|&cursor| cursor == app.selected) else {
        return;
    };
    let count = group.len();

    let shuffle = app
        .source_playlist(source)
        .is_some_and(|(playlist, _)| playlist.shuffle);
    let next_cursor = if app.has_filter() || !shuffle {
        let next = if forward {
            (pos + 1) % count
        } else {
            pos.checked_sub(1).unwrap_or(count - 1)
        };
        group[next]
    } else {
        // Unfiltered, a list's rows are its tracks in order, so its own index and
        // its position within the group are the same number — and the shuffled
        // step is directly usable as one.
        let Some((_, path)) = app.source_playlist(source) else {
            return;
        };
        let path = path.to_path_buf();
        match app.step_index(&path, count, true, pos, forward) {
            Some(idx) => match group.get(idx) {
                Some(&cursor) => cursor,
                None => return,
            },
            None => return,
        }
    };

    app.selected = next_cursor;
    app.clamp_scroll();
    app.play_row(next_cursor);
}

/// Adjust the speed of the track actually driving playback right now (per
/// `app.playing`), not whatever the displayed playlist's cursor happens to
/// point at — the playing track may live in a different playlist entirely.
/// No-op if nothing is playing. `delta` is added to the current effective
/// speed and clamped to mpv's supported range.
///
/// Written to the individual track's document for a plain playlist, same as
/// always — but to the *album's* `default_speed` when the playing track is
/// one of an album's, so every other chapter in it that has no speed of its
/// own already picks it up too. That is the one case this is for: an
/// audiobook's chapters share one natural reading speed, and having to redo
/// `[`/`]` at every chapter boundary is the friction this removes. A track
/// that was individually sped up before this existed keeps that override —
/// `effective_speed`'s precedence (track, then playlist, then config) is
/// unchanged, only which side of it gets written is new.
///
/// The speed is persisted to TOML even if mpv never receives it, so the setting
/// survives a dead player and applies the next time the track is started.
pub(crate) async fn adjust_playing_track_speed(app: &mut App, delta: f32) {
    let Some(session) = app.playing.as_ref() else {
        return;
    };
    let is_album = session.playlist.kind == PlaylistKind::Album;
    let path = session.path.clone();
    let session_playlist = session.playlist.clone();

    let Some(track) = app.playing_track() else {
        return;
    };
    let base = super::effective_speed(track, &session_playlist, &app.config);
    let new_speed = (base + delta).clamp(0.25, 3.0);

    if is_album {
        if let Err(e) = app.with_list_at(&path, |pl, _lib| {
            pl.default_speed = Some(new_speed);
        }) {
            warn!(err = %e, path = %path.display(), "failed to save the album's speed");
        }
        // `PlayingSession.playlist` is its own snapshot, taken when playback
        // started, and `with_list_at` above only touched the real copy
        // (`self.albums[i]` or the file) — not this one. Without refreshing
        // it too, both the next call's `base` (recomputed from this same
        // stale snapshot) and the Now Playing header's speed
        // (`ui.rs`'s `playing_playlist()`) would keep reading the old value
        // forever, which is what "speed doesn't change" actually was: the
        // write landed, but nothing that reads it afterward ever saw it.
        if let Some(session) = app.playing.as_mut() {
            session.playlist.default_speed = Some(new_speed);
        }
    } else {
        if let Some(track) = app.playing_track_mut() {
            track.speed = Some(new_speed);
        }
        // Persist the new speed into the track's own document.
        app.save_playing_track();
    }

    let res = match &app.player {
        Some(player) => Some(player.set_speed(new_speed).await),
        None => None,
    };
    note_ipc_result(app, "speed", res);
}

/// Change the volume by `delta` and push it to mpv. The config value is updated
/// regardless of whether mpv is reachable, so the new level applies to the next
/// track even when the current player has already exited.
async fn set_volume(app: &mut App, delta: i16) {
    let vol = (app.config.default_volume as i16 + delta).clamp(0, 100) as u8;
    app.config.default_volume = vol;
    // Persist immediately, for the same reason as `loop_mode`: the save at quit
    // does not run on every exit path.
    if let Err(e) = app.config.save() {
        warn!(err = %e, "failed to save config after volume change");
    }
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
                let target_path = app.target_playlist_for_url.as_deref().and_then(|name| {
                    if name == app.playlist.name {
                        None // Same as active – use default path
                    } else {
                        app.available_playlists
                            .iter()
                            .find(|entry| entry.name == *name)
                            .map(|entry| entry.path.clone())
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
                        app.available_playlists
                            .push(PlaylistEntry::normal(name, path));
                        app.available_playlists.sort_by(|a, b| a.name.cmp(&b.name));
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

/// The new-album name prompt (`A`): an empty album under the displayed
/// playlist, with no folder required — the manual counterpart to `F`.
pub(crate) fn handle_new_album(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let name = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;
            if !name.is_empty() {
                if let Err(msg) = validate_playlist_name(&name, &app.available_playlists, None) {
                    warn!(msg = %msg, "invalid album name");
                    app.set_status(msg);
                    return Ok(Action::Continue);
                }
                let parent = app.default_album_parent();
                if let Err(e) = app.create_album(&name, parent) {
                    error!(err = %e, "failed to create album");
                    app.set_status("Could not create the album");
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

/// The folder path prompt. Deliberately thin: everything a folder path means
/// is `App::import_folder`'s business, and a single file's `App::import_file`'s
/// — so `F` and the sidebar item and a rescan all go through one place.
fn handle_folder_input(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let typed = app.input_buf.trim().to_string();
            app.input_buf.clear();
            app.input_mode = InputMode::Normal;
            // `Tab`-cycled explicit destination, resolved to a path once: a
            // name that no longer names anything (deleted mid-prompt) falls
            // back to "Auto" rather than silently doing nothing.
            let target_override = app.target_list_for_add.take().and_then(|name| {
                app.available_playlists
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| entry.path.clone())
            });
            if !typed.is_empty() {
                // A pasted path arrives in whichever spelling the clipboard had
                // — a `file://` URL, escaped spaces, quotes — and none of them is
                // a path until `path_from_input` says so.
                let path = library_import::path_from_input(&typed, dirs::home_dir().as_deref());
                // A path that does not exist at all falls to `import_folder`
                // too, which is what already reports "Not a folder" for it —
                // only a path that positively *is* a file takes the new
                // branch, so a typo keeps its familiar message.
                if path.is_file() {
                    // "Auto" for a single file has no folder identity to
                    // match against, so it is simply the displayed playlist.
                    let target_path = target_override.unwrap_or_else(|| app.playlist_path.clone());
                    app.import_file(path, target_path);
                } else {
                    app.import_folder(path, target_override);
                }
            }
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input_buf.clear();
            app.target_list_for_add = None;
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
                app.clear_search();
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
        // The row's own list: an album's row leaves the album's file, not the
        // playlist showing it.
        let row = app.row_at(app.selected).copied();
        if let Some(crate::tui::VisibleRow::Track { source, index: idx }) = row {
            let Some(id) = app
                .source_playlist(source)
                .and_then(|(playlist, _)| playlist.tracks.get(idx))
                .cloned()
            else {
                app.input_mode = InputMode::Normal;
                return Ok(Action::Continue);
            };
            let owner_path = app
                .source_playlist(source)
                .map(|(_, path)| path.to_path_buf())
                .unwrap_or_default();
            // Only stop playback if the track being deleted is literally the
            // one actually driving playback right now (identity is `(path,
            // id)`) — not just any track with a matching id that
            // happens to exist in a differently-playing session elsewhere.
            let is_current = app.is_playing_track(&owner_path, &id);

            if is_current {
                // Stop playback immediately when deleting current track
                app.stop_player(); // kills mpv and retires its position poller
                app.playing = None;
                app.is_paused = false;
                // Nothing is playing any more, so the elapsed time belongs to
                // no track. Left as it was, it kept counting against whatever
                // was played next — and the playback bar showed a position for
                // a track that had been deleted.
                app.position = 0.0;
                let _ = app.pos_tx.send(0.0);
            }

            // The cached file is trovers' own copy of a remote track and goes with
            // the last row referencing it. A local file is the user's music,
            // which trovers only ever reads: deleting the row means forgetting
            // it, never touching what is on disk.
            let file_to_delete = app
                .library
                .get(&id)
                .filter(|t| t.origin != crate::library::TrackOrigin::Local)
                .and_then(|t| t.file.clone());
            app.remove_row(source, idx);
            // A download still running for this row has nowhere to land now.
            app.clear_download_state(&id);
            // Clear any active search filter; it was built over rows that no
            // longer describe the list.
            app.drop_filter();
            if app.selected >= app.visible_track_count() && app.selected > 0 {
                app.selected -= 1;
            }
            app.clamp_scroll();

            // Only the row is definitely gone. The track's document and its
            // cached audio are shared by every playlist listing it, so they go
            // only once nothing does — scoped to the *platform* id, which is
            // what the audio cache is keyed by.
            let platform_id = library::platform_id_of(&id).to_string();
            if app.platform_id_referenced_elsewhere(&platform_id) {
                info!(id = %id, "kept the track document, another playlist still references it");
            } else {
                if let Err(e) = app.library.remove(&id) {
                    warn!(id = %id, err = %e, "failed to delete the track document");
                }
                if let Some(path) = file_to_delete {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), err = %e, "failed to delete cached file");
                    }
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
            if let Some(SidebarItem::Playlist {
                name: old_name,
                path: old_path,
                ..
            }) = selected_item
            {
                // Validate: no duplicate name
                if let Err(msg) =
                    validate_playlist_name(&new_name, &app.available_playlists, Some(&old_name))
                {
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
                        // Update available_playlists entry
                        for entry in &mut app.available_playlists {
                            if entry.name == old_name {
                                entry.name = new_name.clone();
                                entry.path = new_path.clone();
                                break;
                            }
                        }
                        // Albums point at their parent by name, so a rename has to
                        // follow them or every album under it orphans.
                        repoint_albums(app, &old_name, &new_name);
                        app.available_playlists.sort_by(|a, b| a.name.cmp(&b.name));

                        // Re-anchor sidebar_selected to the renamed playlist's new
                        // position. Found by looking, not by arithmetic on the
                        // listing's index: albums nest, so a playlist's row number
                        // is no longer its position in `available_playlists`.
                        let items = app.sidebar_items();
                        if let Some(new_pos) = items.iter().position(
                            |i| matches!(i, SidebarItem::Playlist { name, .. } if name == &new_name),
                        ) {
                            app.sidebar_selected = new_pos;
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

                        info!(old = %old_name, new = %new_name, "renamed playlist");
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

/// Point every album that named `old_name` as its parent at `new_name`, in the
/// listing and in each album's own file.
///
/// A failed write is logged rather than fatal: the album is still there and still
/// listed, it has merely orphaned to the top level, which the user can fix by
/// hand. Losing the rename over it would be worse.
fn repoint_albums(app: &mut App, old_name: &str, new_name: &str) {
    // The displayed playlist's own albums are held in memory, and that copy is the
    // authority on its contents — a fold, a reorder, a play may not be on disk
    // yet. Repoint it and save it from there; re-reading the file would both miss
    // the rename in memory and risk writing a stale copy back over those edits.
    let mut in_memory = Vec::new();
    for loaded in &mut app.albums {
        in_memory.push(loaded.path.clone());
        if loaded.playlist.parent.as_deref() != Some(old_name) {
            continue;
        }
        loaded.playlist.parent = Some(new_name.to_string());
        if let Err(e) = loaded.playlist.save(&loaded.path) {
            error!(err = %e, album = %loaded.name, "failed to save album's new parent");
        }
    }

    for entry in &mut app.available_playlists {
        if entry.kind != PlaylistKind::Album || entry.parent.as_deref() != Some(old_name) {
            continue;
        }
        entry.parent = Some(new_name.to_string());
        if in_memory.contains(&entry.path) {
            continue;
        }
        match Playlist::load(&entry.path) {
            Ok(mut album) => {
                album.parent = Some(new_name.to_string());
                if let Err(e) = album.save(&entry.path) {
                    error!(err = %e, album = %entry.name, "failed to save album's new parent");
                }
            }
            Err(e) => {
                error!(err = %e, album = %entry.name, "failed to load album to repoint its parent");
            }
        }
    }
}

// ── Playlist delete ───────────────────────────────────────────────────────

pub(crate) async fn handle_playlist_delete(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.input_mode = InputMode::Normal;

            let items = app.sidebar_items();
            let selected_item = items.get(app.sidebar_selected).cloned();
            if let Some(SidebarItem::Playlist { name, path, .. }) = selected_item {
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
                let deleting_playing_playlist =
                    app.playing.as_ref().is_some_and(|p| p.path == path);
                if deleting_playing_playlist {
                    app.stop_player(); // kills mpv and retires its position poller
                    app.playing = None;
                    app.is_paused = false;
                }

                match Playlist::delete(&path) {
                    Ok(()) => {
                        // The tracks it listed are untouched: they live in the
                        // library, and any in-flight download still lands in the
                        // document it was started for.
                        // Its albums are playlists in their own right: their files
                        // stay put and they orphan to the top level, which is what
                        // `sidebar_entries` does with a parent that is not there.
                        app.available_playlists.retain(|entry| entry.name != name);
                        // The sidebar can reach an album that is also drawn as a row
                        // here — one whose parent is itself an album, which trovers
                        // does not write but a hand-edited file can say. Drop the
                        // loaded copy so the header does not outlive its file.
                        if app.albums.iter().any(|loaded| loaded.path == path) {
                            app.albums.retain(|loaded| loaded.path != path);
                            app.rebuild_rows();
                            app.clamp_scroll();
                        }
                        // Move sidebar selection up if needed
                        let new_items = app.sidebar_items();
                        if app.sidebar_selected >= new_items.len() {
                            app.sidebar_selected = new_items.len().saturating_sub(1);
                        }
                        // Ensure selection lands on a selectable item.
                        // Prefer the nearest item at-or-before the cursor; if none exists,
                        // fall forward to the first selectable item after the cursor.
                        let at_or_before =
                            new_items.iter().enumerate().rev().find(|(i, item)| {
                                *i <= app.sidebar_selected && item.is_selectable()
                            });
                        if let Some((i, _)) = at_or_before {
                            app.sidebar_selected = i;
                        } else {
                            // Nothing selectable before cursor — find the first one after.
                            let after = new_items.iter().enumerate().find(|(i, item)| {
                                *i > app.sidebar_selected && item.is_selectable()
                            });
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

// ── Album rename and delete, from the header row ──────────────────────────

/// The album whose header the cursor is on, if it still is on one.
///
/// Every album edit re-reads this rather than remembering an index across the
/// prompt: the rows can be rebuilt while it is open, and an index into `albums`
/// that was right when `r` was pressed is not a promise.
fn selected_album(app: &App) -> Option<usize> {
    app.album_of(app.selected)
}

pub(crate) fn handle_album_rename(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Enter => {
            let new_name = app.input_buf.trim().to_string();
            let Some(album) = selected_album(app) else {
                app.input_mode = InputMode::Normal;
                app.input_buf.clear();
                return Ok(Action::Continue);
            };
            match app.rename_album(album, &new_name) {
                Ok(()) => {
                    app.input_mode = InputMode::Normal;
                    app.input_buf.clear();
                }
                // The prompt stays open on a rejected name, holding what was
                // typed: the fix is almost always one keystroke away.
                Err(msg) => {
                    warn!(msg = %msg, "invalid album name");
                    app.set_status(msg);
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

pub(crate) fn handle_album_delete(app: &mut App, key: KeyEvent) -> Result<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            if let Some(album) = selected_album(app) {
                app.delete_album(album);
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
    existing: &[PlaylistEntry],
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
        .any(|entry| entry.name == name && exclude.is_none_or(|ex| entry.name != ex));
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
