use super::{effective_speed, App, Focus, InputMode, SidebarItem, SettingsItem, SETTINGS_ITEMS};
use crate::tui::input::validate_playlist_name;
use crate::config::AudioQuality;
use crate::playlist::{CacheStatus, LoopMode};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row,
        Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
    },
    Frame,
};
use std::time::Duration;

// ── Color palette ─────────────────────────────────────────────────────────

const ACCENT: Color = Color::Rgb(206, 65, 43);
const ACCENT_DIM: Color = Color::Rgb(100, 32, 21);
const SEA_GREEN: Color = Color::Rgb(32, 178, 136);
const GOLD: Color = Color::Rgb(212, 175, 55);
const TEXT_DIM: Color = Color::Rgb(130, 130, 130);
const BORDER_IDLE: Color = Color::Rgb(70, 70, 70);
/// Color for non-interactive / disabled sidebar items.
const ITEM_DISABLED: Color = Color::Rgb(90, 90, 90);
/// Background color for the selected (but not playing) row in the track table.
const ROW_SELECTED_BG: Color = Color::Rgb(60, 60, 60);
/// Background color for the footer status line.
const FOOTER_BG: Color = Color::Rgb(55, 55, 55);

// ── Panel block builder ───────────────────────────────────────────────────

/// Build a rounded-border panel block with consistent title and focus-aware border color.
/// `title` should include surrounding spaces (e.g. `" My Panel "`).
/// `is_focused` controls whether the border uses ACCENT or BORDER_IDLE.
pub(crate) fn make_panel_block(title: &str, is_focused: bool) -> Block<'static> {
    let border_color = if is_focused { ACCENT } else { BORDER_IDLE };
    Block::default()
        .title(title.to_string())
        .title_style(Style::new().fg(Color::White).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border_color))
}

// ── Entry point ───────────────────────────────────────────────────────────

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(0),    // main (sidebar + tracks/settings)
        Constraint::Length(4), // now playing
        Constraint::Length(1), // footer
    ])
    .split(area);

    render_header(frame, app, rows[0]);
    render_main(frame, app, rows[1]);
    render_now_playing(frame, app, rows[2]);
    render_footer(frame, app, rows[3]);

    match app.input_mode {
        InputMode::UrlInput | InputMode::NewPlaylist | InputMode::SearchInput => {
            render_input_overlay(frame, app, area);
        }
        InputMode::TrackContextMenu => {
            render_track_context_menu(frame, app, area);
        }
        InputMode::PlaylistRename => {
            render_playlist_rename_overlay(frame, app, area);
        }
        InputMode::PlaylistDelete => {
            render_playlist_delete_overlay(frame, app, area);
        }
        InputMode::Help => {
            render_help_overlay(frame, app, area);
        }
        _ => {}
    }
}

// ── Header ────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, _app: &App, area: Rect) {
    let clock = chrono::Local::now().format("%H:%M:%S").to_string();
    let width = area.width as usize;
    let left = " ☠ trovers v0.1";
    let padding = width.saturating_sub(left.chars().count() + clock.chars().count() + 1);

    let line = Line::from(vec![
        Span::styled(left, Style::new().fg(ACCENT).bold()),
        Span::raw(" ".repeat(padding)),
        Span::styled(clock, Style::new().fg(TEXT_DIM)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ── Main area ─────────────────────────────────────────────────────────────

fn render_main(frame: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::horizontal([
        Constraint::Length(22),
        Constraint::Min(0),
    ])
    .split(area);

    render_sidebar(frame, app, cols[0]);

    match app.focus {
        Focus::Settings => render_settings_panel(frame, app, cols[1]),
        _ => render_track_table(frame, app, cols[1]),
    }
}

// ── Sidebar ───────────────────────────────────────────────────────────────

fn render_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let block = make_panel_block("", app.focus == Focus::Sidebar);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = app.sidebar_items();
    let active_name = &app.playlist.name;

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == app.sidebar_selected && item.is_selectable();

            let line = match item {
                SidebarItem::PlaylistsHeader => {
                    let arrow = if app.playlists_expanded { "▼" } else { "▶" };
                    let style = if is_selected {
                        Style::new().fg(Color::White).bg(ACCENT_DIM).bold()
                    } else {
                        Style::new().fg(Color::White).bold()
                    };
                    Line::styled(format!(" {arrow} ≡ Playlists"), style)
                }
                SidebarItem::Playlist { name, .. } => {
                    let marker = if name == active_name { " ◄" } else { "" };
                    let (fg, bg) = if is_selected {
                        (Color::White, ACCENT_DIM)
                    } else if name == active_name {
                        (ACCENT, Color::Reset)
                    } else {
                        (Color::White, Color::Reset)
                    };
                    Line::styled(
                        format!("   {}{}", truncate(name, 14), marker),
                        Style::new().fg(fg).bg(bg),
                    )
                }
                SidebarItem::Separator => Line::raw(""),
                SidebarItem::Music => Line::styled(" ♪ Music", Style::new().fg(ITEM_DISABLED)),
                SidebarItem::Video => Line::styled(" ▶ Video", Style::new().fg(ITEM_DISABLED)),
                SidebarItem::Plunder => {
                    let style = if is_selected {
                        Style::new().fg(Color::White).bg(ACCENT_DIM)
                    } else {
                        Style::new().fg(Color::White)
                    };
                    Line::styled(" ↓ Plunder", style)
                }
                SidebarItem::Settings => {
                    let active = app.focus == Focus::Settings;
                    let style = if is_selected || active {
                        Style::new().fg(Color::White).bg(ACCENT_DIM)
                    } else {
                        Style::new().fg(Color::White)
                    };
                    Line::styled(" ⚙ Settings", style)
                }
            };

            ListItem::new(line)
        })
        .collect();

    frame.render_widget(List::new(list_items), inner);
}

// ── Track table ───────────────────────────────────────────────────────────

/// Whether the track table row for `video_id` should show the `▶`
/// highlight — true only when the playing session's track actually belongs
/// to the currently *displayed* playlist file, not just when the id happens
/// to match (ids can collide across playlist files).
pub(crate) fn row_is_playing(app: &App, video_id: &str) -> bool {
    app.playing
        .as_ref()
        .is_some_and(|p| p.path == app.playlist_path && p.track().video_id == video_id)
}

fn render_track_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let total = app.visible_track_count();
    let first = app.track_offset + 1;
    let last = (app.track_offset + app.track_list_height as usize).min(total);

    let title = if total == 0 {
        format!(" {} ", app.playlist.name)
    } else {
        format!(" {}  [ {}–{} / {} ] ", app.playlist.name, first, last, total)
    };

    let block = make_panel_block(&title, app.focus == Focus::TrackList);

    let inner = block.inner(area);
    let table_area = Rect { width: inner.width.saturating_sub(1), ..inner };
    let scrollbar_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        width: 1,
        ..inner
    };

    app.track_list_height = table_area.height;

    // icons(2) + num(4) + sep(1) + artist(16) + sep(1) + dur(7) + padding(3)
    let title_width =
        table_area.width.saturating_sub(2 + 4 + 1 + 16 + 1 + 7 + 3) as usize;

    let rows: Vec<Row> = (app.track_offset..app.track_offset + app.track_list_height as usize)
        .filter_map(|cursor| {
            let track_idx = app.track_index_at(cursor)?;
            let track = app.playlist.tracks.get(track_idx)?;
            let is_playing = row_is_playing(app, &track.video_id);
            let is_selected = cursor == app.selected;

            let play_icon = if is_playing { "▶" } else { " " };
            let status_icon = if app.downloading.contains(&track.video_id) {
                "⟳"
            } else {
                match track.cache_status {
                    CacheStatus::Cached => "◈",
                    CacheStatus::Streaming => "◌",
                    CacheStatus::Downloading => "⟳",
                }
            };

            let row_style = if is_playing && is_selected {
                Style::new().fg(Color::White).bg(ACCENT).bold()
            } else if is_playing {
                Style::new().fg(SEA_GREEN).bold()
            } else if is_selected {
                Style::new().fg(Color::White).bg(ROW_SELECTED_BG)
            } else {
                Style::default()
            };

            let num_str = format!("{:>3} ", track_idx + 1);
            let title_str = truncate(
                track.user_title.as_deref().unwrap_or(&track.title),
                title_width,
            );
            let artist_str = truncate(
                track.user_artist.as_deref().unwrap_or(&track.artist),
                15,
            );
            let dur_str = format_duration(track.duration);

            Some(
                Row::new(vec![
                    Cell::from(format!("{play_icon} {status_icon}")),
                    Cell::from(Span::styled(num_str, Style::new().fg(TEXT_DIM))),
                    Cell::from(title_str),
                    Cell::from(Span::styled(artist_str, Style::new().fg(TEXT_DIM))),
                    Cell::from(Span::styled(dur_str, Style::new().fg(TEXT_DIM))),
                ])
                .style(row_style),
            )
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Length(16),
        Constraint::Length(7),
    ];

    let mut table_state = TableState::default();
    frame.render_widget(block, area);
    frame.render_stateful_widget(Table::new(rows, widths), table_area, &mut table_state);

    if total > 0 {
        let mut scrollbar_state = ScrollbarState::new(total).position(app.track_offset);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .track_symbol(Some("┃"))
                .thumb_symbol("█"),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}

// ── Settings panel ────────────────────────────────────────────────────────

fn render_settings_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = make_panel_block(" ⚙ Settings ", app.focus == Focus::Settings);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical(
        std::iter::repeat(Constraint::Length(2))
            .take(SETTINGS_ITEMS.len())
            .chain(std::iter::once(Constraint::Min(0)))
            .collect::<Vec<_>>(),
    )
    .split(inner);

    for (i, item) in SETTINGS_ITEMS.iter().enumerate() {
        let is_selected = i == app.settings_selected;
        let (label, value) = settings_item_display(item, app);

        let key_style = if is_selected {
            Style::new().fg(Color::White).bold()
        } else {
            Style::new().fg(TEXT_DIM)
        };
        let val_style = if is_selected {
            Style::new().fg(ACCENT).bold()
        } else {
            Style::new().fg(Color::White)
        };

        let hint = if is_selected { " ←/→" } else { "" };
        let line = Line::from(vec![
            Span::raw(if is_selected { " ▶ " } else { "   " }),
            Span::styled(format!("{label:<18}"), key_style),
            Span::styled(value, val_style),
            Span::styled(hint, Style::new().fg(TEXT_DIM)),
        ]);
        frame.render_widget(Paragraph::new(line), rows[i]);
    }
}

fn settings_item_display(item: &SettingsItem, app: &App) -> (&'static str, String) {
    match item {
        SettingsItem::AudioQuality => {
            let val = match app.config.audio_quality {
                AudioQuality::Best => "best",
                AudioQuality::High => "high  (≥192 kbps)",
                AudioQuality::Medium => "medium (96–192 kbps)",
                AudioQuality::Low => "low   (<96 kbps)",
            };
            ("Audio quality", val.to_string())
        }
        SettingsItem::DefaultSpeed => (
            "Default speed",
            format!("{:.1}×", app.config.default_speed),
        ),
        SettingsItem::DefaultVolume => (
            "Default volume",
            format!("{}%", app.config.default_volume),
        ),
    }
}

// ── Now Playing ───────────────────────────────────────────────────────────

fn render_now_playing(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(BORDER_IDLE));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    render_now_playing_header(frame, app, rows[0]);
    render_track_info_row(frame, app, rows[1]);
    render_playback_bar(frame, app, rows[2]);
}

fn render_now_playing_header(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;

    let Some(track) = app.playing_track() else {
        let line = build_now_playing_header_line(width, None, None);
        frame.render_widget(Paragraph::new(line), area);
        return;
    };

    // Fall back to the *playing* playlist's `default_speed`, not the
    // displayed one — they may differ once playback is decoupled from the
    // displayed playlist.
    let fallback_playlist = app.playing_playlist().unwrap_or(&app.playlist);
    let speed = effective_speed(track, fallback_playlist, &app.config);
    let speed_str = format!("{:.1}×", speed);
    let (status_icon, status_text) =
        format_playback_state(app.player.is_some(), app.is_paused, true);

    let center_text = if status_icon.is_empty() {
        status_text.to_string()
    } else {
        format!("{} {}", status_icon, status_text)
    };

    let line = build_now_playing_header_line(width, Some(&center_text), Some(&speed_str));
    frame.render_widget(Paragraph::new(line), area);
}

/// Build the header line spans for the now-playing area.
/// `width` is the available display width.
/// `center` is the playback status text (e.g. "▶ Playing"). None = no track / fetching.
/// `speed` is the speed string (e.g. "1.4×"). None = no track / fetching.
pub(crate) fn build_now_playing_header_line<'a>(
    width: usize,
    center: Option<&str>,
    speed: Option<&str>,
) -> Line<'a> {
    let label = "🎵 Now Playing";

    let label_len = label.chars().count();

    match (center, speed) {
        (Some(status_text), Some(speed_str)) => {
            // Two-section compact layout: label (left) + right-aligned "status · speed"
            let sep = " · ";
            let right_text = format!("{status_text}{sep}{speed_str}");
            let right_len = right_text.chars().count();

            // Fixed: leading space + label + at least 1 space + right + trailing space
            let fixed = 1 + label_len + 1 + right_len + 1;
            let pad = width.saturating_sub(fixed);

            // Style right side with separate spans so speed can be ACCENT.
            Line::from(vec![
                Span::raw(" "),
                Span::styled(label.to_string(), Style::new().fg(GOLD).bold()),
                Span::raw(" ".repeat(1 + pad)),
                Span::styled(status_text.to_string(), Style::new().fg(Color::White)),
                Span::styled(sep.to_string(), Style::new().fg(TEXT_DIM)),
                Span::styled(speed_str.to_string(), Style::new().fg(ACCENT).bold()),
                Span::raw(" "),
            ])
        }
        _ => {
            let status = "No track selected".to_string();
            let status_len = status.chars().count();
            let fixed = 1 + label_len + 1 + status_len;
            let pad = width.saturating_sub(fixed);
            Line::from(vec![
                Span::raw(" "),
                Span::styled(label.to_string(), Style::new().fg(GOLD).bold()),
                Span::raw(" "),
                Span::styled(status, Style::new().fg(TEXT_DIM)),
                Span::raw(" ".repeat(pad)),
            ])
        }
    }
}

fn render_track_info_row(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;

    let Some(track) = app.playing_track() else {
        // No track: render empty row
        frame.render_widget(Paragraph::new(Line::raw("")), area);
        return;
    };

    let title = track.user_title.as_deref().unwrap_or(&track.title);
    let artist = track.user_artist.as_deref().unwrap_or(&track.artist);
    let source = &track.source;

    let line = build_track_info_line(width, title, artist, source);
    frame.render_widget(Paragraph::new(line), area);
}

/// Build the track info line for row 2 of the now-playing area.
/// Displays: TITLE (bold white) • Artist (dim) • source (dim, truncated)
/// Uses `build_separated_line` with truncation priority: title > artist > source.
pub(crate) fn build_track_info_line<'a>(
    width: usize,
    title: &str,
    artist: &str,
    source: &str,
) -> Line<'a> {
    // Use 1-char left margin, so effective text width is width - 1 (for leading space)
    let text_width = width.saturating_sub(1);

    let segments = [
        (title, true),
        (artist, false),
        (source, false),
    ];

    let parts = build_separated_line(&segments, text_width);

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for (text, is_primary) in parts {
        let style = if is_primary {
            Style::new().fg(Color::White).bold()
        } else {
            Style::new().fg(TEXT_DIM)
        };
        spans.push(Span::styled(text, style));
    }

    Line::from(spans)
}

fn render_playback_bar(frame: &mut Frame, app: &App, area: Rect) {
    let Some(track) = app.playing_track() else {
        return;
    };

    let pos_str = format_duration(app.position as u64);
    let dur_str = format_duration(track.duration);
    let vol_str = format!("♪ {}%", app.config.default_volume);

    let track_progress = || {
        app.download_progress
            .get(&track.video_id)
            .copied()
            .unwrap_or(0.0) as f64
            / 100.0
    };

    let cache_state = if app.downloading.contains(&track.video_id) {
        CacheState::Downloading(track_progress())
    } else {
        match track.cache_status {
            CacheStatus::Cached => CacheState::Cached,
            CacheStatus::Streaming => CacheState::Streaming,
            CacheStatus::Downloading => CacheState::Downloading(track_progress()),
        }
    };

    let ratio = if track.duration > 0 {
        (app.position / track.duration as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let line = build_playback_bar_line(
        area.width as usize,
        &pos_str,
        ratio,
        &dur_str,
        &vol_str,
        cache_state,
    );
    frame.render_widget(Paragraph::new(line), area);
}

/// Cache status for playback bar display.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CacheState {
    /// Track is fully cached locally.
    Cached,
    /// Track is streaming (not cached).
    Streaming,
    /// Track is currently downloading; ratio is 0.0–1.0.
    Downloading(f64),
}

/// Build the integrated playback bar line for row 3 of the now-playing area.
/// Layout: ` pos_str [progress bar] dur_str  |  vol_str  |  cache_status `
pub(crate) fn build_playback_bar_line<'a>(
    width: usize,
    pos_str: &str,
    ratio: f64,
    dur_str: &str,
    vol_str: &str,
    cache_state: CacheState,
) -> Line<'a> {
    // In downloading state, delegate entirely to the downloading-specific layout
    if let CacheState::Downloading(dl_ratio) = cache_state {
        return build_downloading_bar_line(width, pos_str, ratio, dur_str, dl_ratio);
    }

    // At this point cache_state is Cached or Streaming
    let (cache_str, cache_color) = match &cache_state {
        CacheState::Cached => ("◈ Cached", SEA_GREEN),
        CacheState::Streaming => ("◌ Stream", TEXT_DIM),
        CacheState::Downloading(_) => unreachable!("Downloading handled above"),
    };

    // Fixed label widths: " pos " + " dur  |  vol  |  cache "
    // " " (1) + pos + " " (1) = pos section prefix/suffix
    // " " (1) + dur + "  " (2) = dur section
    // vol_str + "  " (2) = vol section
    // "│ " (2) + cache_str + " " (1) = cache section
    let sep = " │ ";
    let sep_len = sep.chars().count();
    let pos_len = pos_str.chars().count();
    let dur_len = dur_str.chars().count();
    let vol_len = vol_str.chars().count();
    let cache_len = cache_str.chars().count();

    // Right side: "  " + vol + sep + cache + " "
    let right_fixed = 2 + vol_len + sep_len + cache_len + 1;
    // Left fixed: " " + pos + " " + bar + " " + dur
    let left_fixed = 1 + pos_len + 1 + 1 + dur_len;
    let bar_width = width.saturating_sub(left_fixed + right_fixed).max(1);

    let bar = build_progress_bar(bar_width, ratio, '━', '─', '◉', SEA_GREEN, BORDER_IDLE);

    let mut spans: Vec<Span<'static>> = vec![
        Span::raw(" "),
        Span::styled(pos_str.to_string(), Style::new().fg(TEXT_DIM)),
        Span::raw(" "),
    ];
    spans.extend(bar);
    spans.extend([
        Span::raw(" "),
        Span::styled(dur_str.to_string(), Style::new().fg(TEXT_DIM)),
        Span::raw("  "),
        Span::styled(vol_str.to_string(), Style::new().fg(TEXT_DIM)),
        Span::styled(sep.to_string(), Style::new().fg(BORDER_IDLE)),
        Span::styled(cache_str.to_string(), Style::new().fg(cache_color)),
        Span::raw(" "),
    ]);

    Line::from(spans)
}

/// Build the playback bar line when track is downloading.
/// Replaces the volume section with a download progress bar.
fn build_downloading_bar_line<'a>(
    width: usize,
    pos_str: &str,
    play_ratio: f64,
    dur_str: &str,
    dl_ratio: f64,
) -> Line<'a> {
    let pct_str = format!("{:.0}%", (dl_ratio * 100.0).clamp(0.0, 100.0));
    let dl_label = "⟳ Caching ";
    let dl_label_len = dl_label.chars().count();
    let pct_len = pct_str.chars().count();

    let pos_len = pos_str.chars().count();
    let dur_len = dur_str.chars().count();

    // Fixed: " " + pos + " " + playbar + " " + dur + "  " + dl_label + dlbar + " " + pct + " "
    let right_fixed = 2 + dl_label_len + 1 + pct_len + 1;
    let left_fixed = 1 + pos_len + 1 + 1 + dur_len;
    let total_bar_budget = width.saturating_sub(left_fixed + right_fixed).max(2);

    // Split bar budget: 2/3 for playback, 1/3 for download
    let play_bar_width = (total_bar_budget * 2 / 3).max(1);
    let dl_bar_width = total_bar_budget.saturating_sub(play_bar_width).max(1);

    let play_bar = build_progress_bar(play_bar_width, play_ratio, '━', '─', '◉', SEA_GREEN, BORDER_IDLE);
    let dl_bar = build_progress_bar(dl_bar_width, dl_ratio, '▓', '░', '\0', GOLD, TEXT_DIM);

    let mut spans: Vec<Span<'static>> = vec![
        Span::raw(" "),
        Span::styled(pos_str.to_string(), Style::new().fg(TEXT_DIM)),
        Span::raw(" "),
    ];
    spans.extend(play_bar);
    spans.extend([
        Span::raw(" "),
        Span::styled(dur_str.to_string(), Style::new().fg(TEXT_DIM)),
        Span::raw("  "),
        Span::styled(dl_label.to_string(), Style::new().fg(GOLD)),
    ]);
    spans.extend(dl_bar);
    spans.extend([
        Span::raw(" "),
        Span::styled(pct_str, Style::new().fg(GOLD)),
        Span::raw(" "),
    ]);

    Line::from(spans)
}

// ── Footer ────────────────────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;

    let left = footer_left_message(app);
    let center = footer_center_context(app);
    let right = footer_right_counters(app);

    let fixed = [(0, left.chars().count() + 2), (2, right.chars().count() + 2)];
    let widths = calculate_distributed_widths(width, 3, &fixed);
    let center_width = widths[1];

    let center_trunc = truncate(&center, center_width.saturating_sub(2));
    let center_len = center_trunc.chars().count();
    let pad_total = center_width.saturating_sub(center_len);
    let left_pad = pad_total / 2;
    let right_pad = pad_total - left_pad;

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(truncate(&left, widths[0].saturating_sub(2)), Style::new().fg(Color::White)),
        Span::raw(" ".repeat(left_pad)),
        Span::styled(center_trunc, Style::new().fg(TEXT_DIM)),
        Span::raw(" ".repeat(right_pad)),
        Span::styled(truncate(&right, widths[2].saturating_sub(2)), Style::new().fg(Color::White)),
        Span::raw(" "),
    ]);

    frame.render_widget(
        Paragraph::new(line).style(Style::new().fg(Color::White).bg(FOOTER_BG)),
        area,
    );
}

fn footer_left_message(app: &App) -> String {
    // Always show explicit modal prompts when in an input/confirm mode.
    match (&app.input_mode, &app.focus) {
        (InputMode::UrlInput, _) => return "Add track: Enter URL · [enter] confirm · [esc] cancel".to_string(),
        (InputMode::NewPlaylist, _) => {
            return "New playlist: Enter name · [enter] confirm · [esc] cancel".to_string();
        }
        (InputMode::SearchInput, _) => return "Search: type to filter · [enter] done · [esc] clear".to_string(),
        (InputMode::ConfirmDelete, _) => return "Delete track? · [y] confirm · [n/esc] cancel".to_string(),
        (InputMode::TrackContextMenu, _) => {
            return "Move track: [↑↓] select · [enter] move · [esc] cancel".to_string();
        }
        (InputMode::PlaylistRename, _) => {
            return "Rename playlist: Enter name · [enter] confirm · [esc] cancel".to_string();
        }
        (InputMode::PlaylistDelete, _) => {
            return "Delete playlist? · [y/enter] confirm · [n/esc] cancel".to_string();
        }
        (InputMode::Help, _) => return "Help open · [?]/[esc] close".to_string(),
        _ => {}
    }

    // Otherwise, show a transient status message if present and fresh.
    if let Some((msg, at)) = &app.status_message {
        if at.elapsed() <= Duration::from_secs(4) {
            return msg.clone();
        }
    }

    // Default: context hint.
    match app.focus {
        Focus::Sidebar => "Sidebar".to_string(),
        Focus::TrackList => "Tracks".to_string(),
        Focus::Settings => "Settings".to_string(),
    }
}

fn footer_center_context(app: &App) -> String {
    let focus = match app.focus {
        Focus::Sidebar => "Sidebar",
        Focus::TrackList => "Tracks",
        Focus::Settings => "Settings",
    };

    let mode: String = match app.input_mode {
        InputMode::Normal => "Normal".to_string(),
        InputMode::UrlInput => "Add URL".to_string(),
        InputMode::NewPlaylist => "New playlist".to_string(),
        InputMode::ConfirmDelete => "Confirm delete".to_string(),
        InputMode::SearchInput => {
            if app.input_buf.trim().is_empty() {
                "Search".to_string()
            } else {
                format!("Search: {}", truncate(app.input_buf.trim(), 24))
            }
        }
        InputMode::TrackContextMenu => "Move track".to_string(),
        InputMode::PlaylistRename => "Rename playlist".to_string(),
        InputMode::PlaylistDelete => "Delete playlist".to_string(),
        InputMode::Help => "Help".to_string(),
    };

    if matches!(app.input_mode, InputMode::SearchInput) {
        // In SearchInput, show match counts if possible.
        let visible = app.visible_track_count();
        let total = app.playlist.tracks.len();
        return format!("{focus} · {mode} · {visible}/{total}");
    }

    format!("{focus} · {mode}")
}

pub(crate) fn footer_right_counters(app: &App) -> String {
    let mut parts: Vec<String> = Vec::new();

    if app.pending_fetches > 0 {
        parts.push(format!("⏳{}", app.pending_fetches));
    }

    let dl = app.downloading.len();
    if dl > 0 {
        parts.push(format!("↓{}", dl));
    }

    // Loop mode and shuffle both change what happens when a track ends, which
    // is otherwise invisible until it happens — and `l`/`r` gave no feedback at
    // all that they had done anything.
    match app.playlist.loop_mode {
        LoopMode::None => {}
        LoopMode::Track => parts.push("↻ Track".to_string()),
        LoopMode::Playlist => parts.push("↻ All".to_string()),
    }

    if app.playlist.shuffle {
        parts.push("⇄ Shuffle".to_string());
    }

    if !app.filtered_indices.is_empty() {
        parts.push("Filter".to_string());
    }

    if parts.is_empty() {
        "? Help".to_string()
    } else {
        parts.join("  ")
    }
}

// ── Input overlay ─────────────────────────────────────────────────────────

fn render_input_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let (title, prompt) = match app.input_mode {
        InputMode::UrlInput => ("Add Track", "URL: "),
        InputMode::NewPlaylist => ("New Playlist", "Name: "),
        InputMode::SearchInput => ("Search", "/"),
        _ => return,
    };

    // For URL input mode, show the target playlist hint as a second line
    let show_playlist_hint = app.input_mode == InputMode::UrlInput;
    let height = if show_playlist_hint { 5u16 } else { 3u16 };
    let width = area.width.min(64).max(30);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(Style::new().fg(ACCENT).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if show_playlist_hint {
        let rows = Layout::vertical([
            Constraint::Length(1), // URL input line
            Constraint::Length(1), // empty separator
            Constraint::Length(1), // playlist target line
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(format!("{}{}_", prompt, app.input_buf))
                .style(Style::new().fg(Color::White)),
            rows[0],
        );

        let target_name = url_input_target_display(app);
        let has_multiple = app.available_playlists.len() > 1;
        let tab_hint = if has_multiple { "  [tab] cycle" } else { "" };

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Playlist: ", Style::new().fg(TEXT_DIM)),
                Span::styled(target_name, Style::new().fg(ACCENT).bold()),
                Span::styled(tab_hint, Style::new().fg(TEXT_DIM)),
            ])),
            rows[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(format!("{}{}_", prompt, app.input_buf))
                .style(Style::new().fg(Color::White)),
            inner,
        );
    }
}

/// Returns the display name for the current URL input target playlist.
/// Falls back to the active playlist name when no target is set.
pub(crate) fn url_input_target_display(app: &App) -> String {
    app.target_playlist_for_url
        .as_deref()
        .unwrap_or(&app.playlist.name)
        .to_string()
}

// ── Track context menu ────────────────────────────────────────────────────

fn render_track_context_menu(frame: &mut Frame, app: &App, area: Rect) {
    let items = app.available_playlist_names();

    let item_count = items.len();
    // Height: 2 (border) + 1 (header line) + max(1, item_count) rows, clamped to terminal area
    let content_rows = if item_count == 0 { 1 } else { item_count };
    let height = ((2 + 1 + content_rows) as u16).min(area.height);
    let width = area.width.min(40).max(24);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Move Track To ")
        .title_style(Style::new().fg(ACCENT).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if item_count == 0 {
        frame.render_widget(
            Paragraph::new("  No other playlists").style(Style::new().fg(TEXT_DIM)),
            inner,
        );
        return;
    }

    // Layout: 1-row hint + item rows
    let rows = Layout::vertical(
        std::iter::once(Constraint::Length(1))
            .chain(std::iter::repeat(Constraint::Length(1)).take(item_count))
            .chain(std::iter::once(Constraint::Min(0)))
            .collect::<Vec<_>>(),
    )
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓", Style::new().fg(TEXT_DIM)),
            Span::raw(" navigate  "),
            Span::styled("[enter]", Style::new().fg(ACCENT)),
            Span::raw(" move  "),
            Span::styled("[esc]", Style::new().fg(TEXT_DIM)),
            Span::raw(" cancel"),
        ])),
        rows[0],
    );

    for (i, name) in items.iter().enumerate() {
        let is_selected = i == app.context_menu_selected;
        let (fg, bg) = if is_selected {
            (Color::White, ACCENT_DIM)
        } else {
            (Color::White, Color::Reset)
        };
        let prefix = if is_selected { " ▶ " } else { "   " };
        let label = format!("{}{}", prefix, truncate(name, width as usize - 5));
        frame.render_widget(
            Paragraph::new(label).style(Style::new().fg(fg).bg(bg)),
            rows[i + 1],
        );
    }
}

// ── Help overlay ──────────────────────────────────────────────────────────

fn render_help_overlay(frame: &mut Frame, _app: &App, area: Rect) {
    let width = area.width.min(84).max(44);
    let height = area.height.min(22).max(12);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Help ")
        .title_style(Style::new().fg(ACCENT).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Render as simple lines for stability across terminals.
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(" Global", Style::new().fg(GOLD).bold()),
        Span::raw("  "),
        Span::styled("[?]", Style::new().fg(ACCENT)),
        Span::raw(" help  "),
        Span::styled("[tab]", Style::new().fg(ACCENT)),
        Span::raw(" focus  "),
        Span::styled("[q]", Style::new().fg(ACCENT)),
        Span::raw(" quit"),
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled(
        " Track list",
        Style::new().fg(GOLD).bold(),
    )]));
    lines.push(Line::raw("  [↑↓/jk] navigate   [enter] play   [spc] play/pause"));
    lines.push(Line::raw("  [←→] seek ±10s     [shift+←→] seek ±60s"));
    lines.push(Line::raw("  Speed: '[' slower   ']' faster"));
    lines.push(Line::raw("  [a] add URL        [m] move track   [d] delete   [/] search"));
    lines.push(Line::raw("  [n] next           [b] previous    [N] new playlist"));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled(
        " Sidebar",
        Style::new().fg(GOLD).bold(),
    )]));
    lines.push(Line::raw("  [↑↓] navigate   [enter] select/toggle   [r] rename   [d] delete"));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" Close", Style::new().fg(GOLD).bold()),
        Span::raw("  "),
        Span::styled("[esc]", Style::new().fg(ACCENT)),
        Span::raw(" or "),
        Span::styled("[?]", Style::new().fg(ACCENT)),
    ]));

    frame.render_widget(
        Paragraph::new(lines).style(Style::new().fg(Color::White)),
        inner,
    );
}

// ── Playlist rename overlay ───────────────────────────────────────────────

fn render_playlist_rename_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width.min(52).max(30);
    let height = 3u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    // Determine current name from sidebar selection so we allow keeping it during rename
    let current_name: Option<String> = {
        let items = app.sidebar_items();
        match items.get(app.sidebar_selected) {
            Some(SidebarItem::Playlist { name, .. }) => Some(name.clone()),
            _ => None,
        }
    };

    // Show validation hint when name is invalid
    let is_valid = validate_playlist_name(
        app.input_buf.trim(),
        &app.available_playlists,
        current_name.as_deref(),
    )
    .is_ok();

    let border_color = if is_valid { ACCENT } else { GOLD };

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!("Name: {}_", app.input_buf))
            .block(
                Block::default()
                    .title(" Rename Playlist ")
                    .title_style(Style::new().fg(ACCENT).bold())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(border_color)),
            )
            .style(Style::new().fg(Color::White)),
        popup,
    );
}

// ── Playlist delete overlay ───────────────────────────────────────────────

/// Returns the name of the playlist currently targeted for delete, if any.
pub(crate) fn playlist_delete_target<'a>(app: &'a App) -> Option<&'a str> {
    let items = app.sidebar_items();
    // We need a stable reference – match on the sidebar items vec
    // Note: sidebar_items() returns owned Strings so we need to look up in available_playlists
    match items.get(app.sidebar_selected) {
        Some(SidebarItem::Playlist { name, .. }) => {
            // Return ref to the name stored in available_playlists for lifetime safety
            app.available_playlists
                .iter()
                .find(|(n, _)| n == name)
                .map(|(n, _)| n.as_str())
        }
        _ => None,
    }
}

fn render_playlist_delete_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let Some(target) = playlist_delete_target(app) else {
        return;
    };
    let target = target.to_string();

    let is_active = app.playlist.name == target;

    let msg = if is_active {
        format!("Cannot delete active playlist '{}'", truncate(&target, 20))
    } else {
        format!("Delete '{}'?  [y] yes  [n] no", truncate(&target, 24))
    };

    let width = area.width.min(56).max(34);
    let height = 3u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!(" {msg}"))
            .block(
                Block::default()
                    .title(" Delete Playlist ")
                    .title_style(Style::new().fg(ACCENT).bold())
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::new().fg(ACCENT)),
            )
            .style(Style::new().fg(Color::White)),
        popup,
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────

pub(crate) fn build_progress_bar(
    width: usize,
    ratio: f64,
    fill: char,
    empty: char,
    thumb: char,
    fill_color: Color,
    empty_color: Color,
) -> Vec<Span<'static>> {
    if width == 0 {
        return vec![];
    }
    let filled = ((ratio * width as f64) as usize).min(width);

    if thumb != '\0' && filled < width {
        // thumb mode: filled section | thumb | empty section
        let pre = filled.saturating_sub(1);
        let empty_count = width.saturating_sub(pre + 1);

        let mut spans = Vec::with_capacity(3);
        if pre > 0 {
            spans.push(Span::styled(
                fill.to_string().repeat(pre),
                Style::new().fg(fill_color),
            ));
        }
        spans.push(Span::styled(thumb.to_string(), Style::new().fg(fill_color)));
        if empty_count > 0 {
            spans.push(Span::styled(
                empty.to_string().repeat(empty_count),
                Style::new().fg(empty_color),
            ));
        }
        spans
    } else {
        // no-thumb mode: filled section | empty section
        let mut spans = Vec::with_capacity(2);
        if filled > 0 {
            spans.push(Span::styled(
                fill.to_string().repeat(filled),
                Style::new().fg(fill_color),
            ));
        }
        let remaining = width.saturating_sub(filled);
        if remaining > 0 {
            spans.push(Span::styled(
                empty.to_string().repeat(remaining),
                Style::new().fg(empty_color),
            ));
        }
        spans
    }
}

/// Distribute `total_width` across N sections with optional fixed-width items.
/// Returns a Vec of widths for each section.
/// `fixed_widths` contains (index, width) pairs for sections with known widths.
/// Remaining width is distributed to the first flexible section (index not in fixed_widths).
pub(crate) fn calculate_distributed_widths(
    total_width: usize,
    section_count: usize,
    fixed_widths: &[(usize, usize)],
) -> Vec<usize> {
    if section_count == 0 {
        return vec![];
    }
    let mut widths = vec![0usize; section_count];
    let mut used: usize = 0;

    for &(idx, w) in fixed_widths {
        if idx < section_count {
            widths[idx] = w;
            used = used.saturating_add(w);
        }
    }

    let remaining = total_width.saturating_sub(used);
    // Give remaining width to the first flexible (not fixed) section
    if let Some(flex_idx) = (0..section_count).find(|i| !fixed_widths.iter().any(|&(fi, _)| fi == *i)) {
        widths[flex_idx] = remaining;
    }

    widths
}

/// Build a line of bullet-separated text segments, applying truncation priority.
/// `segments` is a list of (text, is_bold) pairs.
/// `max_width` is the total character budget.
/// Segments with lower index have higher truncation priority (kept longer).
/// Returns a Vec of (text, is_primary) pairs where is_primary = true means bold/primary style.
pub(crate) fn build_separated_line(
    segments: &[(&str, bool)],
    max_width: usize,
) -> Vec<(String, bool)> {
    if segments.is_empty() || max_width == 0 {
        return vec![];
    }

    let sep = " • ";
    let sep_len = sep.chars().count();

    // Calculate total separators width
    let total_sep = if segments.len() > 1 {
        (segments.len() - 1) * sep_len
    } else {
        0
    };

    let text_budget = max_width.saturating_sub(total_sep);

    // Give primary (first) segment priority: it gets up to its full length,
    // then distribute remaining to subsequent segments proportionally
    let mut result: Vec<(String, bool)> = Vec::with_capacity(segments.len() * 2 - 1);

    // First pass: calculate natural lengths
    let natural_lens: Vec<usize> = segments.iter().map(|(t, _)| t.chars().count()).collect();
    let total_natural: usize = natural_lens.iter().sum();

    if total_natural <= text_budget {
        // Everything fits
        for (i, (text, is_bold)) in segments.iter().enumerate() {
            if i > 0 && !text.is_empty() && !result.is_empty() {
                result.push((sep.to_string(), false));
            }
            if !text.is_empty() {
                result.push((text.to_string(), *is_bold));
            }
        }
    } else {
        // Need to truncate: primary segment (index 0) gets priority
        // It keeps its full text if possible, others get remaining
        let primary_len = natural_lens[0].min(text_budget);
        let secondary_budget = text_budget.saturating_sub(primary_len);

        // Distribute secondary budget evenly among remaining segments
        let secondary_count = segments.len().saturating_sub(1);
        let per_secondary = if secondary_count > 0 {
            secondary_budget / secondary_count
        } else {
            0
        };

        for (i, (text, is_bold)) in segments.iter().enumerate() {
            let budget = if i == 0 { primary_len } else { per_secondary };
            let truncated = truncate(text, budget);
            if !truncated.is_empty() {
                if !result.is_empty() {
                    result.push((sep.to_string(), false));
                }
                result.push((truncated, *is_bold));
            }
        }
    }

    result
}

/// Format playback state into a display string.
/// Returns (status_icon, status_text) based on player/paused state.
pub(crate) fn format_playback_state(
    has_player: bool,
    is_paused: bool,
    has_track: bool,
) -> (&'static str, &'static str) {
    if !has_track {
        return ("", "No track");
    }
    if !has_player {
        return ("⏳", "Loading…");
    }
    if is_paused {
        ("⏸", "Paused")
    } else {
        ("▶", "Playing")
    }
}

pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let end = max.saturating_sub(1);
        chars[..end].iter().collect::<String>() + "…"
    }
}
