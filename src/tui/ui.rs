use super::{effective_speed, App, Focus, InputMode, SidebarItem, SettingsItem, SETTINGS_ITEMS};
use crate::config::AudioQuality;
use crate::playlist::CacheStatus;
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

// ── Color palette ─────────────────────────────────────────────────────────

const ACCENT: Color = Color::Rgb(206, 65, 43);
const ACCENT_DIM: Color = Color::Rgb(100, 32, 21);
const SEA_GREEN: Color = Color::Rgb(32, 178, 136);
const GOLD: Color = Color::Rgb(212, 175, 55);
const TEXT_DIM: Color = Color::Rgb(130, 130, 130);
const BORDER_IDLE: Color = Color::Rgb(70, 70, 70);

// ── Entry point ───────────────────────────────────────────────────────────

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(0),    // main (sidebar + tracks/settings)
        Constraint::Length(3), // now playing
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
        _ => {}
    }
}

// ── Header ────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, _app: &App, area: Rect) {
    let clock = chrono::Local::now().format("%H:%M:%S").to_string();
    let width = area.width as usize;
    let left = " ☠ trovers v0.1";
    let padding = width.saturating_sub(left.len() + clock.len() + 1);

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
    let border_color = if app.focus == Focus::Sidebar { ACCENT } else { BORDER_IDLE };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border_color));

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
                SidebarItem::PlaylistsOverflow { count } => {
                    Line::styled(format!("   ▼ {count} more…"), Style::new().fg(TEXT_DIM))
                }
                SidebarItem::Separator => Line::raw(""),
                SidebarItem::Music => Line::styled(" ♪ Music", Style::new().fg(TEXT_DIM)),
                SidebarItem::Video => Line::styled(" ▶ Video", Style::new().fg(TEXT_DIM)),
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

fn render_track_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let total = app.visible_track_count();
    let first = app.track_offset + 1;
    let last = (app.track_offset + app.track_list_height as usize).min(total);

    let title = if total == 0 {
        format!(" {} ", app.playlist.name)
    } else {
        format!(" {}  [ {}–{} / {} ] ", app.playlist.name, first, last, total)
    };

    let border_color = if app.focus == Focus::TrackList { ACCENT } else { BORDER_IDLE };

    let block = Block::default()
        .title(title)
        .title_style(Style::new().fg(Color::White).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border_color));

    let inner = block.inner(area);
    let table_area = Rect { width: inner.width.saturating_sub(1), ..inner };
    let scrollbar_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        width: 1,
        ..inner
    };

    app.track_list_height = table_area.height;

    let title_width =
        table_area.width.saturating_sub(2 + 4 + 1 + 16 + 1 + 7 + 3) as usize;

    let rows: Vec<Row> = (app.track_offset..app.track_offset + app.track_list_height as usize)
        .filter_map(|cursor| {
            let track_idx = app.track_index_at(cursor)?;
            let track = app.playlist.tracks.get(track_idx)?;
            let is_playing = app.playlist.current_track.as_deref()
                == Some(track.video_id.as_str());
            let is_selected = cursor == app.selected;

            let play_icon = if is_playing { "▶" } else { " " };
            let status_icon = if app.downloading.contains(&track.video_id) {
                "⟳"
            } else {
                match track.cache_status {
                    CacheStatus::Cached => "◈",
                    CacheStatus::Streaming => "◌",
                }
            };

            let row_style = if is_playing && is_selected {
                Style::new().fg(Color::White).bg(ACCENT).bold()
            } else if is_playing {
                Style::new().fg(SEA_GREEN).bold()
            } else if is_selected {
                Style::new().fg(Color::White).bg(Color::Rgb(60, 60, 60))
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
    let block = Block::default()
        .title(" ⚙ Settings ")
        .title_style(Style::new().fg(Color::White).bold())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));

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

    render_now_playing_title(frame, app, rows[0]);
    render_playback_bar(frame, app, rows[1]);
    render_cache_and_eq(frame, app, rows[2]);
}

fn render_now_playing_title(frame: &mut Frame, app: &App, area: Rect) {
    // Show "fetching…" while metadata is in flight
    if app.pending_fetches > 0 && app.playlist.current_track.is_none() {
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled("⏳ fetching metadata…", Style::new().fg(GOLD)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let current_idx = app
        .playlist
        .current_track
        .as_deref()
        .and_then(|id| app.playlist.tracks.iter().position(|t| t.video_id == id));

    let Some(track) = current_idx.and_then(|i| app.playlist.tracks.get(i)) else {
        frame.render_widget(
            Paragraph::new(Span::styled(" No track selected", Style::new().fg(TEXT_DIM))),
            area,
        );
        return;
    };

    let play_icon = if app.is_paused { "⏸" } else { "▶" };
    let speed = effective_speed(track, &app.playlist, &app.config);
    let speed_str = format!("{:.1}×", speed);

    let title = track.user_title.as_deref().unwrap_or(&track.title);
    let artist = track.user_artist.as_deref().unwrap_or(&track.artist);

    let meta_max = area.width.saturating_sub(
        2 + play_icon.len() as u16 + 2 + speed_str.len() as u16 + 1,
    ) as usize;
    let meta = format!("{} · {} · {}", title, artist, track.source);
    let meta_truncated = truncate(&meta, meta_max);
    let pad = (area.width as usize)
        .saturating_sub(2 + play_icon.len() + 1 + meta_truncated.len() + 1 + speed_str.len());

    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(play_icon, Style::new().fg(SEA_GREEN)),
        Span::raw("  "),
        Span::styled(title.to_string(), Style::new().bold()),
        Span::styled(
            format!(" · {} · {}", artist, track.source),
            Style::new().fg(TEXT_DIM),
        ),
        Span::raw(" ".repeat(pad)),
        Span::styled(speed_str, Style::new().fg(ACCENT).bold()),
        Span::raw(" "),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_playback_bar(frame: &mut Frame, app: &App, area: Rect) {
    let current_idx = app
        .playlist
        .current_track
        .as_deref()
        .and_then(|id| app.playlist.tracks.iter().position(|t| t.video_id == id));
    let Some(track) = current_idx.and_then(|i| app.playlist.tracks.get(i)) else {
        return;
    };

    let pos_str = format_duration(app.position as u64);
    let dur_str = format_duration(track.duration);
    let vol_str = format!("♪ {}%", app.config.default_volume);

    let labels_width = 1 + pos_str.len() + 1 + 1 + dur_str.len() + 3 + vol_str.len() + 1;
    let bar_width = (area.width as usize).saturating_sub(labels_width);

    let ratio = if track.duration > 0 {
        (app.position / track.duration as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let bar = build_progress_bar(bar_width, ratio, '━', '─', '◉', SEA_GREEN, BORDER_IDLE);

    let mut spans = vec![
        Span::raw(" "),
        Span::styled(pos_str, Style::new().fg(TEXT_DIM)),
        Span::raw(" "),
    ];
    spans.extend(bar);
    spans.extend([
        Span::raw(" "),
        Span::styled(dur_str, Style::new().fg(TEXT_DIM)),
        Span::raw("   "),
        Span::styled(vol_str, Style::new().fg(TEXT_DIM)),
        Span::raw(" "),
    ]);
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_cache_and_eq(frame: &mut Frame, app: &App, area: Rect) {
    let status_str = if app.player.is_some() {
        if app.is_paused { "⏸" } else { "♪" }
    } else {
        ""
    };

    let cache_part = if app.is_downloading() {
        let labels_width = 9 + 4 + 1;
        let bar_width = (area.width as usize)
            .saturating_sub(labels_width + status_str.len() + 2)
            .max(1);
        let ratio = (app.download_progress / 100.0).clamp(0.0, 1.0) as f64;
        let bar = build_progress_bar(bar_width, ratio, '▓', '░', '\0', GOLD, TEXT_DIM);

        let pct_str = if app.download_progress > 0.0 {
            format!(" {:.0}%", app.download_progress)
        } else {
            " …".to_string()
        };
        let mut cache_spans = vec![Span::styled(" caching ", Style::new().fg(GOLD))];
        cache_spans.extend(bar);
        cache_spans.push(Span::styled(pct_str, Style::new().fg(GOLD)));
        cache_spans
    } else {
        vec![Span::raw("")]
    };

    let cache_len: usize = cache_part.iter().map(|s| s.content.len()).sum();
    let pad = (area.width as usize)
        .saturating_sub(cache_len + status_str.len() + 1)
        .max(0);

    let mut spans = cache_part;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(status_str, Style::new().fg(SEA_GREEN)));
    spans.push(Span::raw(" "));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Footer ────────────────────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let text = match (&app.input_mode, &app.focus) {
        (InputMode::UrlInput, _) => {
            " Enter URL and press [enter]  ·  [esc] cancel"
        }
        (InputMode::NewPlaylist, _) => {
            " Enter playlist name and press [enter]  ·  [esc] cancel"
        }
        (InputMode::SearchInput, _) => {
            " Type to filter  ·  [enter] confirm  ·  [esc] clear"
        }
        (InputMode::ConfirmDelete, _) => {
            " Delete track?  ·  [y] confirm  ·  [n/esc] cancel"
        }
        (InputMode::Normal, Focus::Sidebar) => {
            " [↑↓] nav  ·  [enter] select  ·  [tab] → tracks  ·  [q] quit"
        }
        (InputMode::Normal, Focus::TrackList) => {
            " [↑↓/jk] nav  ·  [enter] play  ·  [spc] play/pause  ·  [←→] seek  ·  [[]]] speed  ·  [a] add  ·  [/] search  ·  [N] playlist  ·  [q] quit"
        }
        (InputMode::Normal, Focus::Settings) => {
            " [↑↓] select  ·  [←→] change  ·  [esc/tab] back"
        }
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(TEXT_DIM)),
        area,
    );
}

// ── Input overlay ─────────────────────────────────────────────────────────

fn render_input_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let (title, prompt) = match app.input_mode {
        InputMode::UrlInput => ("Add Track", "URL: "),
        InputMode::NewPlaylist => ("New Playlist", "Name: "),
        InputMode::SearchInput => ("Search", "/"),
        _ => return,
    };

    let width = area.width.min(64).max(30);
    let height = 3u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!("{}{}_", prompt, app.input_buf))
            .block(
                Block::default()
                    .title(format!(" {title} "))
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
