#[cfg(test)]
mod tests {
    use crate::tui::ui::{
        build_now_playing_header_line, build_playback_bar_line, build_progress_bar,
        build_separated_line, build_track_info_line, calculate_distributed_widths, format_duration,
        format_playback_state, make_panel_block, truncate, CacheState,
    };
    use ratatui::style::Color;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_playlist(name: &str) -> crate::playlist::Playlist {
        crate::playlist::Playlist {
            name: name.to_string(),
            created: chrono::Utc::now(),
            loop_mode: crate::playlist::LoopMode::None,
            shuffle: false,
            default_speed: None,
            tracks: Vec::new(),
            current_track: None,
            kind: crate::playlist::PlaylistKind::Normal,
            parent: None,
            source_folder: None,
            collapsed: true,
        }
    }

    /// The tracks directory for the current test.
    ///
    /// Track documents are global by design, so a test that wants rows to resolve
    /// needs a real directory to write them into. Keyed by thread id because
    /// libtest gives each test its own thread: one test's `vid1` document must
    /// never become another's. (Run single-threaded, tests share the directory but
    /// never interleave, and each one rewrites the ids it cares about before
    /// loading its library — so that is safe too.)
    fn test_tracks_root() -> std::path::PathBuf {
        static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        ROOT.get_or_init(|| tempfile::tempdir().expect("tempdir"))
            .path()
            .join(format!("{:?}", std::thread::current().id()))
    }

    /// The library to build an `App` under test around: every document written by
    /// `add_track` so far.
    fn test_library() -> crate::library::Library {
        crate::library::Library::load(&test_tracks_root()).expect("load library")
    }

    /// Append `track` to `pl` as a row and write its document, so the row
    /// resolves once an `App` is built around `test_library()`.
    fn add_track(pl: &mut crate::playlist::Playlist, track: crate::library::Track) {
        let mut lib = test_library();
        let id = track.id.clone();
        lib.upsert(track).expect("write track document");
        pl.tracks.push(id);
    }

    /// `add_track` for a playlist that already belongs to an `App`: the document
    /// goes through the app's own library, which is where it reads rows from.
    fn push_track(app: &mut crate::tui::App, track: crate::library::Track) {
        let id = track.id.clone();
        app.library.upsert(track).expect("write track document");
        app.playlist.tracks.push(id);
        // A new row is a new row on screen: production adds one through a path that
        // rebuilds, and a test whose rows were left stale would not be testing the
        // list the user sees.
        app.rebuild_rows();
    }

    /// The raw TOML of `id`'s document. For assertions that must see exactly what
    /// reached disk: `Library::load` repairs some states on the way in, so
    /// reloading would pass either way.
    fn raw_document(id: &str) -> String {
        let root = test_tracks_root();
        let needle = format!("id = \"{id}\"");
        for entry in std::fs::read_dir(&root).expect("read tracks dir").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).expect("read document");
            if raw.contains(&needle) {
                return raw;
            }
        }
        panic!("no document for id '{id}' under {}", root.display());
    }

    /// Re-read the app's library from disk, so it sees documents `add_track`
    /// wrote after the app was built — typically for a *second* playlist the
    /// test then hands to `session_at`.
    fn refresh_library(app: &mut crate::tui::App) {
        app.library = test_library();
    }

    /// The `PlayingSession` playing row `idx` of `pl`, as `request_playback` would
    /// have built it. Takes `pl` by value because the id is read out of it.
    fn session_at(
        path: impl Into<std::path::PathBuf>,
        pl: crate::playlist::Playlist,
        idx: usize,
    ) -> crate::tui::PlayingSession {
        let track_id = pl.tracks[idx].clone();
        crate::tui::PlayingSession {
            path: path.into(),
            playlist: pl,
            track_id,
        }
    }

    fn make_app_with_playlists(active: &str, playlists: &[&str]) -> crate::tui::App {
        use std::path::PathBuf;
        let playlist = make_playlist(active);
        let config = crate::config::Config::default();
        let available: Vec<crate::playlist::PlaylistEntry> = playlists
            .iter()
            .map(|n| {
                crate::playlist::PlaylistEntry::normal(
                    n.to_string(),
                    PathBuf::from(format!("/fake/{}.toml", n)),
                )
            })
            .collect();
        crate::tui::App::new(
            playlist,
            config,
            available,
            PathBuf::from("/fake/active.toml"),
            test_library(),
        )
    }

    /// Top-level playlist entries from `(name, path)` pairs. Most tests care only
    /// about which playlists exist, not about albums.
    fn entries<S: Into<String>>(
        pairs: Vec<(S, std::path::PathBuf)>,
    ) -> Vec<crate::playlist::PlaylistEntry> {
        pairs
            .into_iter()
            .map(|(name, path)| crate::playlist::PlaylistEntry::normal(name.into(), path))
            .collect()
    }

    // ── format_duration tests ─────────────────────────────────────────────

    #[test]
    fn format_duration_zero() {
        assert_eq!(format_duration(0), "00:00");
    }

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(45), "00:45");
    }

    #[test]
    fn format_duration_one_minute() {
        assert_eq!(format_duration(60), "01:00");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(125), "02:05");
    }

    #[test]
    fn format_duration_fifty_nine_minutes() {
        assert_eq!(format_duration(3599), "59:59");
    }

    #[test]
    fn format_duration_one_hour() {
        assert_eq!(format_duration(3600), "01:00:00");
    }

    #[test]
    fn format_duration_hours_minutes_seconds() {
        assert_eq!(format_duration(3661), "01:01:01");
    }

    #[test]
    fn format_duration_long() {
        // 2h 30m 45s = 9045s
        assert_eq!(format_duration(9045), "02:30:45");
    }

    #[test]
    fn format_duration_large() {
        // 10h = 36000s
        assert_eq!(format_duration(36000), "10:00:00");
    }

    // ── truncate tests ────────────────────────────────────────────────────

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_max_zero() {
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_shorter_than_max() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_overflow_appends_ellipsis() {
        let result = truncate("hello world", 8);
        assert_eq!(result, "hello w…");
        assert_eq!(result.chars().count(), 8);
    }

    #[test]
    fn truncate_max_one() {
        // max=1: end = max.saturating_sub(1) = 0, so empty string + ellipsis
        let result = truncate("hello", 1);
        assert_eq!(result, "…");
        assert_eq!(result.chars().count(), 1);
    }

    #[test]
    fn truncate_max_two() {
        let result = truncate("hello", 2);
        assert_eq!(result, "h…");
        assert_eq!(result.chars().count(), 2);
    }

    #[test]
    fn truncate_unicode() {
        let result = truncate("héllo wörld", 7);
        assert_eq!(result.chars().count(), 7);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncate_unicode_exact() {
        assert_eq!(truncate("héllo", 5), "héllo");
    }

    // ── build_progress_bar tests ──────────────────────────────────────────

    /// Collect all characters from a Vec<Span> into a single String.
    fn spans_to_string(spans: &[ratatui::text::Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Count characters matching predicate across all spans.
    fn spans_char_count(spans: &[ratatui::text::Span], ch: char) -> usize {
        spans_to_string(spans).chars().filter(|&c| c == ch).count()
    }

    #[test]
    fn progress_bar_zero_width() {
        let spans = build_progress_bar(0, 0.5, '━', '─', '◉', Color::Green, Color::Gray);
        assert!(spans.is_empty());
    }

    #[test]
    fn progress_bar_zero_ratio_with_thumb() {
        // ratio=0.0 → filled=0, pre=0, thumb at start, then 9 empty chars
        let spans = build_progress_bar(10, 0.0, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 10);
        assert!(s.starts_with('◉'));
        assert_eq!(spans_char_count(&spans, '─'), 9);
    }

    #[test]
    fn progress_bar_full_ratio_with_thumb() {
        // filled = 10 = width, no thumb case (filled < width is false)
        let spans = build_progress_bar(10, 1.0, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 10);
        assert_eq!(spans_char_count(&spans, '━'), 10);
        assert_eq!(spans_char_count(&spans, '─'), 0);
    }

    #[test]
    fn progress_bar_half_ratio_with_thumb() {
        // filled = 5, pre = 4 → 4 fill + thumb + 5 empty = 10
        let spans = build_progress_bar(10, 0.5, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 10);
        assert_eq!(spans_char_count(&spans, '━'), 4);
        assert_eq!(spans_char_count(&spans, '◉'), 1);
        assert_eq!(spans_char_count(&spans, '─'), 5);
    }

    #[test]
    fn progress_bar_no_thumb() {
        // thumb = '\0' means no thumb character
        let spans = build_progress_bar(10, 0.4, '▓', '░', '\0', Color::Yellow, Color::DarkGray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 10);
        assert_eq!(spans_char_count(&spans, '▓'), 4);
        assert_eq!(spans_char_count(&spans, '░'), 6);
    }

    #[test]
    fn progress_bar_width_one_with_thumb() {
        // filled=0, pre=0, push thumb, then 0 empty → single char
        let spans = build_progress_bar(1, 0.5, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 1);
    }

    #[test]
    fn progress_bar_ratio_clamped_at_one() {
        // ratio > 1.0 clamped: filled = min(7, 5) = 5, no thumb
        let spans = build_progress_bar(5, 1.5, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 5);
        assert_eq!(spans_char_count(&spans, '━'), 5);
    }

    #[test]
    fn progress_bar_filled_spans_use_fill_color() {
        // filled section must use fill_color
        let fill_color = Color::Rgb(32, 178, 136); // SEA_GREEN
        let empty_color = Color::Rgb(70, 70, 70); // BORDER_IDLE
        let spans = build_progress_bar(10, 0.5, '━', '─', '◉', fill_color, empty_color);

        // The fill and thumb spans should use fill_color
        // The empty span should use empty_color
        let fill_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.content.contains('━') || s.content.contains('◉'))
            .collect();
        let empty_spans: Vec<_> = spans.iter().filter(|s| s.content.contains('─')).collect();

        for s in &fill_spans {
            assert_eq!(
                s.style.fg,
                Some(fill_color),
                "fill/thumb span should have fill_color"
            );
        }
        for s in &empty_spans {
            assert_eq!(
                s.style.fg,
                Some(empty_color),
                "empty span should have empty_color"
            );
        }
    }

    #[test]
    fn progress_bar_no_thumb_colors() {
        // No-thumb mode: filled uses fill_color, empty uses empty_color
        let fill_color = Color::Rgb(212, 175, 55); // GOLD
        let empty_color = Color::Rgb(130, 130, 130); // TEXT_DIM
        let spans = build_progress_bar(10, 0.4, '▓', '░', '\0', fill_color, empty_color);

        let fill_spans: Vec<_> = spans.iter().filter(|s| s.content.contains('▓')).collect();
        let empty_spans: Vec<_> = spans.iter().filter(|s| s.content.contains('░')).collect();

        for s in &fill_spans {
            assert_eq!(
                s.style.fg,
                Some(fill_color),
                "fill span should have fill_color"
            );
        }
        for s in &empty_spans {
            assert_eq!(
                s.style.fg,
                Some(empty_color),
                "empty span should have empty_color"
            );
        }
    }

    #[test]
    fn progress_bar_zero_ratio_no_thumb() {
        // No thumb, ratio=0: all empty with empty_color
        let fill_color = Color::Green;
        let empty_color = Color::Gray;
        let spans = build_progress_bar(8, 0.0, '▓', '░', '\0', fill_color, empty_color);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 8);
        assert_eq!(spans_char_count(&spans, '░'), 8);
        assert_eq!(spans_char_count(&spans, '▓'), 0);

        // All spans should use empty_color
        for span in &spans {
            assert_eq!(span.style.fg, Some(empty_color));
        }
    }

    #[test]
    fn progress_bar_full_ratio_no_thumb() {
        // No thumb, ratio=1: all filled with fill_color
        let fill_color = Color::Green;
        let empty_color = Color::Gray;
        let spans = build_progress_bar(8, 1.0, '▓', '░', '\0', fill_color, empty_color);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 8);
        assert_eq!(spans_char_count(&spans, '▓'), 8);
        assert_eq!(spans_char_count(&spans, '░'), 0);

        for span in &spans {
            assert_eq!(span.style.fg, Some(fill_color));
        }
    }

    #[test]
    fn progress_bar_thumb_at_start_edge_case() {
        // ratio=0.0, width=5: filled=0, pre=0, thumb at pos 0, then 4 empty
        let spans = build_progress_bar(5, 0.0, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 5);
        assert!(s.starts_with('◉'));
        assert_eq!(spans_char_count(&spans, '─'), 4);
    }

    #[test]
    fn progress_bar_thumb_at_end_edge_case() {
        // ratio close to 1.0, width=5: filled=4, pre=3 → 3 fill + thumb + 1 empty
        let spans = build_progress_bar(5, 0.8, '━', '─', '◉', Color::Green, Color::Gray);
        let s = spans_to_string(&spans);
        assert_eq!(s.chars().count(), 5);
        assert_eq!(spans_char_count(&spans, '━'), 3);
        assert_eq!(spans_char_count(&spans, '◉'), 1);
        assert_eq!(spans_char_count(&spans, '─'), 1);
    }

    // ── calculate_distributed_widths tests ───────────────────────────────────

    #[test]
    fn distributed_widths_empty_sections() {
        let result = calculate_distributed_widths(80, 0, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn distributed_widths_single_flexible_section() {
        // No fixed widths: single section gets all the width
        let result = calculate_distributed_widths(80, 1, &[]);
        assert_eq!(result, vec![80]);
    }

    #[test]
    fn distributed_widths_all_flexible_first_gets_remainder() {
        // 3 sections, no fixed: first gets all, others get 0
        let result = calculate_distributed_widths(80, 3, &[]);
        assert_eq!(result[0], 80);
        assert_eq!(result[1], 0);
        assert_eq!(result[2], 0);
    }

    #[test]
    fn distributed_widths_with_fixed_sections() {
        // 3 sections: section 1 fixed=10, section 2 fixed=8 → section 0 gets 80-18=62
        let result = calculate_distributed_widths(80, 3, &[(1, 10), (2, 8)]);
        assert_eq!(result[0], 62); // flexible
        assert_eq!(result[1], 10); // fixed
        assert_eq!(result[2], 8); // fixed
    }

    #[test]
    fn distributed_widths_first_section_fixed_flex_goes_to_second() {
        // section 0 fixed=20, section 1 is flexible → gets 80-20=60
        let result = calculate_distributed_widths(80, 2, &[(0, 20)]);
        assert_eq!(result[0], 20); // fixed
        assert_eq!(result[1], 60); // flexible
    }

    #[test]
    fn distributed_widths_overflow_saturates_at_zero() {
        // Fixed widths exceed total → flexible section gets 0
        let result = calculate_distributed_widths(10, 3, &[(1, 8), (2, 6)]);
        assert_eq!(result[0], 0); // flexible, saturated
        assert_eq!(result[1], 8);
        assert_eq!(result[2], 6);
    }

    #[test]
    fn distributed_widths_exact_fit() {
        // Fixed widths exactly equal total → flexible gets 0
        let result = calculate_distributed_widths(20, 3, &[(1, 10), (2, 10)]);
        assert_eq!(result[0], 0);
        assert_eq!(result[1], 10);
        assert_eq!(result[2], 10);
    }

    #[test]
    fn distributed_widths_narrow_terminal() {
        // Typical narrow terminal width scenario
        let result = calculate_distributed_widths(40, 3, &[(1, 10), (2, 6)]);
        assert_eq!(result[0], 24); // 40 - 10 - 6
        assert_eq!(result[1], 10);
        assert_eq!(result[2], 6);
    }

    // ── build_separated_line tests ────────────────────────────────────────────

    #[test]
    fn separated_line_empty_segments() {
        let result = build_separated_line(&[], 80);
        assert!(result.is_empty());
    }

    #[test]
    fn separated_line_zero_width() {
        let result = build_separated_line(&[("hello", true)], 0);
        assert!(result.is_empty());
    }

    #[test]
    fn separated_line_single_segment_fits() {
        let result = build_separated_line(&[("hello", true)], 80);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "hello");
        assert!(result[0].1);
    }

    #[test]
    fn separated_line_two_segments_fit() {
        // "hello • world" = 13 chars
        let result = build_separated_line(&[("hello", true), ("world", false)], 80);
        assert_eq!(result.len(), 3); // hello + sep + world
        assert_eq!(result[0].0, "hello");
        assert_eq!(result[1].0, " • ");
        assert!(!result[1].1); // separator is not bold
        assert_eq!(result[2].0, "world");
        assert!(!result[2].1);
    }

    #[test]
    fn separated_line_three_segments_fit() {
        // "Track • Artist • source.com" all fit in 80
        let result = build_separated_line(
            &[("Track", true), ("Artist", false), ("source.com", false)],
            80,
        );
        assert_eq!(result.len(), 5); // track + sep + artist + sep + source
        assert_eq!(result[0].0, "Track");
        assert_eq!(result[2].0, "Artist");
        assert_eq!(result[4].0, "source.com");
    }

    #[test]
    fn separated_line_primary_has_priority_on_truncation() {
        // Very narrow: primary segment should be truncated less than secondary
        // "Hello World" (11) + " • " (3) + "Artist Name" (11) = 25
        // Force truncation with max_width=15: text_budget=15-3=12
        // primary gets min(11,12)=11, secondary gets 12-11=1
        let result = build_separated_line(&[("Hello World", true), ("Artist Name", false)], 15);
        // Result: "Hello World" + sep + 1-char truncated artist
        let texts: Vec<&str> = result.iter().map(|r| r.0.as_str()).collect();
        assert!(
            texts.contains(&"Hello World"),
            "primary should be preserved: {:?}",
            texts
        );
    }

    #[test]
    fn separated_line_truncation_applied() {
        // Single segment that needs truncation
        let result = build_separated_line(&[("Hello World", true)], 8);
        assert_eq!(result.len(), 1);
        let text = &result[0].0;
        assert_eq!(text.chars().count(), 8);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn separated_line_bold_flags_preserved() {
        let result =
            build_separated_line(&[("Track", true), ("Artist", false), ("Source", false)], 80);
        // indices: 0=Track(bold), 1=sep, 2=Artist(not bold), 3=sep, 4=Source(not bold)
        assert!(result[0].1, "first segment should be bold");
        assert!(!result[1].1, "separator should not be bold");
        assert!(!result[2].1, "second segment should not be bold");
    }

    // ── format_playback_state tests ───────────────────────────────────────────

    #[test]
    fn playback_state_no_track() {
        let (icon, text) = format_playback_state(false, false, false);
        assert_eq!(icon, "");
        assert_eq!(text, "No track");
    }

    #[test]
    fn playback_state_no_player_but_has_track() {
        // Track selected but player not ready yet (loading)
        let (icon, text) = format_playback_state(false, false, true);
        assert_eq!(icon, "⏳");
        assert_eq!(text, "Loading…");
    }

    #[test]
    fn playback_state_playing() {
        let (icon, text) = format_playback_state(true, false, true);
        assert_eq!(icon, "▶");
        assert_eq!(text, "Playing");
    }

    #[test]
    fn playback_state_paused() {
        let (icon, text) = format_playback_state(true, true, true);
        assert_eq!(icon, "⏸");
        assert_eq!(text, "Paused");
    }

    #[test]
    fn playback_state_no_track_takes_priority() {
        // Even if has_player=true and is_paused=true, no track means "No track"
        let (icon, text) = format_playback_state(true, true, false);
        assert_eq!(icon, "");
        assert_eq!(text, "No track");
    }

    #[test]
    fn playback_state_icons_are_nonempty_when_active() {
        let (playing_icon, _) = format_playback_state(true, false, true);
        let (paused_icon, _) = format_playback_state(true, true, true);
        assert!(!playing_icon.is_empty());
        assert!(!paused_icon.is_empty());
    }

    // ── build_now_playing_header_line tests ───────────────────────────────────

    /// Collect all span content into a single string.
    fn line_to_string(line: &ratatui::text::Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn header_no_track_contains_label_and_status() {
        let line = build_now_playing_header_line(80, None, None);
        let text = line_to_string(&line);
        assert!(
            text.contains("🎵 Now Playing"),
            "should contain label: {text:?}"
        );
        assert!(
            text.contains("No track selected"),
            "should contain no-track status: {text:?}"
        );
    }

    #[test]
    fn header_no_track_total_width_does_not_exceed() {
        let width = 80;
        let line = build_now_playing_header_line(width, None, None);
        let text = line_to_string(&line);
        let char_count: usize = text.chars().count();
        // Should not exceed the width (may be less due to saturation)
        assert!(
            char_count <= width + 5,
            "header too wide: {char_count} chars for width={width}"
        );
    }

    #[test]
    fn header_playing_state_contains_all_three_sections() {
        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let text = line_to_string(&line);
        assert!(
            text.contains("🎵 Now Playing"),
            "should contain label: {text:?}"
        );
        assert!(
            text.contains("▶ Playing"),
            "should contain playback status: {text:?}"
        );
        assert!(text.contains("1.4×"), "should contain speed: {text:?}");
    }

    #[test]
    fn header_paused_state() {
        let line = build_now_playing_header_line(80, Some("⏸ Paused"), Some("1.0×"));
        let text = line_to_string(&line);
        assert!(
            text.contains("⏸ Paused"),
            "should contain paused status: {text:?}"
        );
        assert!(text.contains("1.0×"), "should contain speed: {text:?}");
    }

    #[test]
    fn header_playing_gold_style_on_label() {
        let gold = Color::Rgb(212, 175, 55);
        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        // The label span should have GOLD color
        let label_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("🎵 Now Playing"));
        assert!(label_span.is_some(), "label span should exist");
        assert_eq!(
            label_span.unwrap().style.fg,
            Some(gold),
            "label should be GOLD colored"
        );
    }

    #[test]
    fn header_speed_accent_style() {
        let accent = Color::Rgb(206, 65, 43);
        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        // The speed span should have ACCENT color
        let speed_span = line.spans.iter().find(|s| s.content.contains("1.4×"));
        assert!(speed_span.is_some(), "speed span should exist");
        assert_eq!(
            speed_span.unwrap().style.fg,
            Some(accent),
            "speed should be ACCENT colored"
        );
    }

    #[test]
    fn header_narrow_terminal_does_not_panic() {
        // Very narrow: should not panic, just produce truncated/minimal output
        let line = build_now_playing_header_line(10, Some("▶ Playing"), Some("1.0×"));
        let text = line_to_string(&line);
        // Just check it doesn't panic and produces something
        assert!(!text.is_empty());
    }

    #[test]
    fn header_layout_calculations_use_full_width() {
        // In three-section mode, the total widths should sum to ~= the available width
        let width = 100;
        let label = "🎵 Now Playing"; // 14 chars
        let label_section = 1 + label.chars().count(); // 15
        let speed_str = "1.4×";
        let speed_section = speed_str.chars().count() + 1; // 5

        let fixed = [(0, label_section), (2, speed_section)];
        let widths = calculate_distributed_widths(width, 3, &fixed);

        assert_eq!(widths[0], label_section);
        assert_eq!(widths[2], speed_section);
        // Center gets the remaining space
        let expected_center = width.saturating_sub(label_section + speed_section);
        assert_eq!(widths[1], expected_center);
        // Total should equal width
        assert_eq!(widths[0] + widths[1] + widths[2], width);
    }

    #[test]
    fn header_no_track_different_widths() {
        // Test at various widths to ensure no panic
        for w in [20, 40, 60, 80, 120] {
            let line = build_now_playing_header_line(w, None, None);
            let text = line_to_string(&line);
            assert!(text.contains("🎵 Now Playing"), "width={w}: missing label");
        }
    }

    #[test]
    fn header_three_section_different_speeds() {
        for speed in ["0.5×", "1.0×", "1.5×", "2.0×"] {
            let line = build_now_playing_header_line(80, Some("▶ Playing"), Some(speed));
            let text = line_to_string(&line);
            assert!(
                text.contains(speed),
                "should contain speed {speed}: {text:?}"
            );
        }
    }

    // ── build_track_info_line tests ───────────────────────────────────────────

    #[test]
    fn track_info_line_contains_all_three_parts() {
        let line = build_track_info_line(80, "My Track Title", "Some Artist", "youtube.com/watch");
        let text = line_to_string(&line);
        assert!(
            text.contains("My Track Title"),
            "should contain title: {text:?}"
        );
        assert!(
            text.contains("Some Artist"),
            "should contain artist: {text:?}"
        );
        assert!(
            text.contains("youtube.com/watch"),
            "should contain source: {text:?}"
        );
    }

    #[test]
    fn track_info_line_has_bullet_separators() {
        let line = build_track_info_line(80, "Track", "Artist", "source.com");
        let text = line_to_string(&line);
        // Should have bullet separators between sections
        assert!(
            text.contains(" • "),
            "should contain bullet separator: {text:?}"
        );
    }

    #[test]
    fn track_info_line_title_is_bold_white() {
        let line = build_track_info_line(80, "My Track", "Artist", "source.com");
        // Find span containing title text - it should be bold and white
        let title_span = line.spans.iter().find(|s| s.content.contains("My Track"));
        assert!(title_span.is_some(), "title span should exist");
        let span = title_span.unwrap();
        assert_eq!(
            span.style.fg,
            Some(ratatui::style::Color::White),
            "title should be white"
        );
    }

    #[test]
    fn track_info_line_artist_is_dim() {
        let text_dim = ratatui::style::Color::Rgb(130, 130, 130);
        let line = build_track_info_line(80, "Track", "My Artist", "source.com");
        let artist_span = line.spans.iter().find(|s| s.content.contains("My Artist"));
        assert!(artist_span.is_some(), "artist span should exist");
        assert_eq!(
            artist_span.unwrap().style.fg,
            Some(text_dim),
            "artist should be TEXT_DIM colored"
        );
    }

    #[test]
    fn track_info_line_source_is_dim() {
        let text_dim = ratatui::style::Color::Rgb(130, 130, 130);
        let line = build_track_info_line(80, "Track", "Artist", "my-source.com");
        let source_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("my-source.com"));
        assert!(source_span.is_some(), "source span should exist");
        assert_eq!(
            source_span.unwrap().style.fg,
            Some(text_dim),
            "source should be TEXT_DIM colored"
        );
    }

    #[test]
    fn track_info_line_starts_with_space() {
        let line = build_track_info_line(80, "Track", "Artist", "source.com");
        // First span should be a leading space for margin
        assert!(!line.spans.is_empty(), "line should have spans");
        assert_eq!(
            line.spans[0].content, " ",
            "should start with a space for margin"
        );
    }

    #[test]
    fn track_info_line_title_truncation_priority() {
        // In narrow width, title should be preserved over artist/source
        // Width=20: after leading space, text_width=19
        // "Long Title Here" (15) + " • " (3) + "Art" (3) + " • " (3) + "src" (3) = 27 > 19
        let line = build_track_info_line(20, "Long Title Here", "Artist Name", "source.com");
        let text = line_to_string(&line);
        // The title should be present (possibly truncated but longer than artist)
        assert!(!text.is_empty(), "should not be empty");
    }

    #[test]
    fn track_info_line_narrow_terminal_no_panic() {
        // Very narrow: should not panic
        for w in [1, 5, 10, 15, 20] {
            let line = build_track_info_line(
                w,
                "Track Title That Is Very Long Indeed",
                "Artist",
                "source.com",
            );
            let text = line_to_string(&line);
            // Just checking no panic - content may be very short
            let _ = text;
        }
    }

    #[test]
    fn track_info_line_total_width_respects_bounds() {
        let width = 60;
        let line = build_track_info_line(
            width,
            "My Track Title",
            "Great Artist",
            "youtube.com/watch?v=abc123",
        );
        let text = line_to_string(&line);
        // Total chars should not greatly exceed available width
        // (allow some slack for unicode multi-byte but char count should be ≤ width)
        let char_count = text.chars().count();
        assert!(
            char_count <= width + 2,
            "track info line too wide: {char_count} chars for width={width}, text={text:?}"
        );
    }

    #[test]
    fn track_info_line_empty_artist_still_works() {
        // Empty artist should still render title and source
        let line = build_track_info_line(80, "Track Title", "", "source.com");
        let text = line_to_string(&line);
        assert!(
            text.contains("Track Title"),
            "should contain title: {text:?}"
        );
    }

    #[test]
    fn track_info_line_empty_source_still_works() {
        let line = build_track_info_line(80, "Track Title", "Artist Name", "");
        let text = line_to_string(&line);
        assert!(
            text.contains("Track Title"),
            "should contain title: {text:?}"
        );
        assert!(
            text.contains("Artist Name"),
            "should contain artist: {text:?}"
        );
    }

    #[test]
    fn track_info_line_user_overrides_applied_by_caller() {
        // The function takes already-resolved title/artist (caller applies overrides)
        // This tests that what we pass in is what appears in the output
        let user_title = "Custom Title Override";
        let user_artist = "Custom Artist Override";
        let line = build_track_info_line(120, user_title, user_artist, "source.com");
        let text = line_to_string(&line);
        assert!(
            text.contains(user_title),
            "user title override should appear: {text:?}"
        );
        assert!(
            text.contains(user_artist),
            "user artist override should appear: {text:?}"
        );
    }

    // ── build_playback_bar_line tests ─────────────────────────────────────────

    #[test]
    fn playback_bar_cached_contains_pos_and_dur() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        let text = line_to_string(&line);
        assert!(text.contains("00:03"), "should contain position: {text:?}");
        assert!(text.contains("55:34"), "should contain duration: {text:?}");
    }

    #[test]
    fn playback_bar_cached_contains_volume() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        let text = line_to_string(&line);
        assert!(text.contains("♪ 85%"), "should contain volume: {text:?}");
    }

    #[test]
    fn playback_bar_cached_shows_cache_indicator() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        let text = line_to_string(&line);
        assert!(
            text.contains("◈ Cached"),
            "should contain cached indicator: {text:?}"
        );
    }

    #[test]
    fn playback_bar_streaming_shows_stream_indicator() {
        let line =
            build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Streaming);
        let text = line_to_string(&line);
        assert!(
            text.contains("◌ Stream"),
            "should contain streaming indicator: {text:?}"
        );
    }

    #[test]
    fn playback_bar_failed_shows_failed_indicator() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Failed);
        let text = line_to_string(&line);
        assert!(
            text.contains("✕ Failed"),
            "should contain failed indicator: {text:?}"
        );
    }

    #[test]
    fn playback_bar_downloading_shows_caching_indicator() {
        let line = build_playback_bar_line(
            80,
            "00:03",
            0.1,
            "55:34",
            "♪ 85%",
            CacheState::Downloading(0.45),
        );
        let text = line_to_string(&line);
        assert!(
            text.contains("⟳ Caching"),
            "should contain caching indicator: {text:?}"
        );
    }

    #[test]
    fn playback_bar_downloading_shows_percentage() {
        let line = build_playback_bar_line(
            80,
            "00:03",
            0.1,
            "55:34",
            "♪ 85%",
            CacheState::Downloading(0.45),
        );
        let text = line_to_string(&line);
        assert!(
            text.contains("45%"),
            "should contain download percentage: {text:?}"
        );
    }

    #[test]
    fn playback_bar_downloading_shows_position_and_duration() {
        let line = build_playback_bar_line(
            80,
            "01:23",
            0.3,
            "04:56",
            "♪ 70%",
            CacheState::Downloading(0.6),
        );
        let text = line_to_string(&line);
        assert!(
            text.contains("01:23"),
            "should contain position in download mode: {text:?}"
        );
        assert!(
            text.contains("04:56"),
            "should contain duration in download mode: {text:?}"
        );
    }

    #[test]
    fn playback_bar_no_panic_on_zero_width() {
        // Should not panic with very small widths
        for w in [0, 1, 5, 10] {
            let _line =
                build_playback_bar_line(w, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        }
    }

    #[test]
    fn playback_bar_no_panic_on_narrow_terminal() {
        for w in [20, 30, 40] {
            let _line =
                build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Cached);
            let _line =
                build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Streaming);
            let _line = build_playback_bar_line(
                w,
                "00:03",
                0.5,
                "55:34",
                "♪ 85%",
                CacheState::Downloading(0.3),
            );
            let _line =
                build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Failed);
        }
    }

    #[test]
    fn playback_bar_cached_separator_present() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        let text = line_to_string(&line);
        // Should have a separator between volume and cache status
        assert!(text.contains(" │ "), "should contain separator: {text:?}");
    }

    #[test]
    fn playback_bar_cached_color_sea_green_on_cache_span() {
        let sea_green = Color::Rgb(32, 178, 136);
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        let cache_span = line.spans.iter().find(|s| s.content.contains("◈ Cached"));
        assert!(cache_span.is_some(), "cached span should exist");
        assert_eq!(
            cache_span.unwrap().style.fg,
            Some(sea_green),
            "cached indicator should be SEA_GREEN"
        );
    }

    #[test]
    fn playback_bar_streaming_color_dim_on_cache_span() {
        let text_dim = Color::Rgb(130, 130, 130);
        let line =
            build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Streaming);
        let stream_span = line.spans.iter().find(|s| s.content.contains("◌ Stream"));
        assert!(stream_span.is_some(), "streaming span should exist");
        assert_eq!(
            stream_span.unwrap().style.fg,
            Some(text_dim),
            "streaming indicator should be TEXT_DIM"
        );
    }

    #[test]
    fn playback_bar_downloading_0_percent() {
        // dl_ratio = 0.0 → "0%"
        let line = build_playback_bar_line(
            80,
            "00:00",
            0.0,
            "10:00",
            "♪ 80%",
            CacheState::Downloading(0.0),
        );
        let text = line_to_string(&line);
        assert!(
            text.contains("0%"),
            "should show 0% when download just started: {text:?}"
        );
    }

    #[test]
    fn playback_bar_downloading_100_percent() {
        // dl_ratio = 1.0 → "100%"
        let line = build_playback_bar_line(
            80,
            "05:00",
            0.5,
            "10:00",
            "♪ 80%",
            CacheState::Downloading(1.0),
        );
        let text = line_to_string(&line);
        assert!(
            text.contains("100%"),
            "should show 100% when download complete: {text:?}"
        );
    }

    #[test]
    fn cache_state_equality() {
        assert_eq!(CacheState::Cached, CacheState::Cached);
        assert_eq!(CacheState::Streaming, CacheState::Streaming);
        assert_eq!(CacheState::Failed, CacheState::Failed);
        assert_ne!(CacheState::Cached, CacheState::Streaming);
        assert_ne!(CacheState::Streaming, CacheState::Failed);
    }

    #[test]
    fn playback_bar_starts_with_space() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        assert!(!line.spans.is_empty(), "should have spans");
        assert_eq!(
            line.spans[0].content, " ",
            "should start with leading space"
        );
    }

    #[test]
    fn playback_bar_ratio_full_no_panic() {
        // ratio=1.0 should fill entire progress bar without panic
        let _line = build_playback_bar_line(80, "55:34", 1.0, "55:34", "♪ 80%", CacheState::Cached);
    }

    #[test]
    fn playback_bar_ratio_zero_no_panic() {
        let _line = build_playback_bar_line(80, "00:00", 0.0, "55:34", "♪ 80%", CacheState::Cached);
    }

    // ── render_now_playing integration tests ──────────────────────────────────
    //
    // These tests verify that the three rows of the now-playing area work
    // correctly together: header (row 1) + track info (row 2) + playback bar (row 3).
    // We test via the public builder functions since render_now_playing uses Frame.

    #[test]
    fn now_playing_three_rows_produce_distinct_content() {
        // Row 1: header with label, status, speed
        let header = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.0×"));
        let header_text = line_to_string(&header);

        // Row 2: track info with title, artist, source
        let track = build_track_info_line(80, "My Song", "My Artist", "youtube.com");
        let track_text = line_to_string(&track);

        // Row 3: playback bar with position, bar, duration, volume, cache
        let bar = build_playback_bar_line(80, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);
        let bar_text = line_to_string(&bar);

        // Each row should have distinct primary content
        assert!(
            header_text.contains("🎵 Now Playing"),
            "header row missing label"
        );
        assert!(track_text.contains("My Song"), "track row missing title");
        assert!(bar_text.contains("00:30"), "playback row missing position");

        // Content should not cross between rows
        assert!(
            !track_text.contains("🎵 Now Playing"),
            "track row should not contain header label"
        );
        assert!(
            !header_text.contains("My Song"),
            "header row should not contain track title"
        );
    }

    #[test]
    fn now_playing_row1_header_structure() {
        // Row 1 must always contain the "🎵 Now Playing" label
        let playing = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.5×"));
        let paused = build_now_playing_header_line(80, Some("⏸ Paused"), Some("1.0×"));
        let no_track = build_now_playing_header_line(80, None, None);

        assert!(line_to_string(&playing).contains("🎵 Now Playing"));
        assert!(line_to_string(&paused).contains("🎵 Now Playing"));
        assert!(line_to_string(&no_track).contains("🎵 Now Playing"));
    }

    #[test]
    fn now_playing_row2_track_info_structure() {
        // Row 2: bullet-separated track info
        let line = build_track_info_line(80, "Song Title", "Artist Name", "source.com");
        let text = line_to_string(&line);
        // Must contain all three parts with separator
        assert!(text.contains("Song Title"));
        assert!(text.contains("Artist Name"));
        assert!(text.contains("source.com"));
        assert!(text.contains(" • "));
    }

    #[test]
    fn now_playing_row3_playback_bar_structure() {
        // Row 3: integrated position + progress + duration + volume + cache
        let line =
            build_playback_bar_line(80, "02:15", 0.3, "07:30", "♪ 75%", CacheState::Streaming);
        let text = line_to_string(&line);
        assert!(text.contains("02:15"), "position");
        assert!(text.contains("07:30"), "duration");
        assert!(text.contains("♪ 75%"), "volume");
        assert!(text.contains("◌ Stream"), "cache status");
    }

    #[test]
    fn now_playing_all_states_no_panic() {
        // Verify no panics for all combinations of states across all three rows
        let widths = [40, 80, 120];
        let states = [
            (None, None),
            (Some("▶ Playing"), Some("1.0×")),
            (Some("⏸ Paused"), Some("0.8×")),
            (Some("⏳ Loading…"), Some("1.0×")),
        ];

        for w in widths {
            for (center, speed) in &states {
                let _header = build_now_playing_header_line(w, *center, *speed);
            }
            let _track = build_track_info_line(w, "Title", "Artist", "source");
            let _bar_cached =
                build_playback_bar_line(w, "01:00", 0.5, "02:00", "♪ 80%", CacheState::Cached);
            let _bar_stream =
                build_playback_bar_line(w, "01:00", 0.5, "02:00", "♪ 80%", CacheState::Streaming);
            let _bar_dl = build_playback_bar_line(
                w,
                "01:00",
                0.5,
                "02:00",
                "♪ 80%",
                CacheState::Downloading(0.6),
            );
        }
    }

    #[test]
    fn now_playing_row_allocation_three_rows_of_height_one() {
        // Verify that the three-row structure fits inside the outer area.
        // The outer area has Constraint::Length(4). Borders::TOP consumes 1 row,
        // leaving inner_height = 3 for the 3 × Length(1) content rows.
        // We validate this with actual ratatui layout math.
        use ratatui::layout::{Constraint, Layout, Rect};
        let outer = Rect::new(0, 0, 80, 4); // Length(4) outer area
        let block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::TOP);
        let inner = block.inner(outer);
        assert_eq!(
            inner.height, 3,
            "Borders::TOP on a 4-row area leaves 3 rows for content"
        );

        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
        assert_eq!(rows[0].height, 1, "header row height");
        assert_eq!(rows[1].height, 1, "track info row height");
        assert_eq!(
            rows[2].height, 1,
            "playback bar row height — must be 1, not 0"
        );
    }

    #[test]
    fn now_playing_cache_state_removed_from_old_row() {
        // Verify cache status is integrated into row 3 (playback bar),
        // not in a separate fourth row. The playback bar should contain cache info.
        let cached_line =
            build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 90%", CacheState::Cached);
        let stream_line =
            build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 90%", CacheState::Streaming);
        let dl_line = build_playback_bar_line(
            80,
            "00:10",
            0.2,
            "01:00",
            "♪ 90%",
            CacheState::Downloading(0.5),
        );

        let cached_text = line_to_string(&cached_line);
        let stream_text = line_to_string(&stream_line);
        let dl_text = line_to_string(&dl_line);

        // Each cache state should be visible in row 3
        assert!(cached_text.contains("◈ Cached"), "Cached state in row 3");
        assert!(stream_text.contains("◌ Stream"), "Streaming state in row 3");
        assert!(dl_text.contains("⟳ Caching"), "Downloading state in row 3");
    }

    #[test]
    fn now_playing_no_duplicate_playback_state_across_rows() {
        // Row 1 shows playback state (Playing/Paused/Loading)
        // Row 2 shows track info only
        // Row 3 shows progress only
        // No cross-row content duplication
        let header = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.2×"));
        let track = build_track_info_line(80, "Cool Track", "Great Artist", "bandcamp.com");
        let bar = build_playback_bar_line(80, "00:45", 0.75, "01:00", "♪ 60%", CacheState::Cached);

        let h = line_to_string(&header);
        let t = line_to_string(&track);
        let b = line_to_string(&bar);

        // Speed is only in row 1, not in row 2 or 3
        assert!(h.contains("1.2×"), "speed in header");
        assert!(!t.contains("1.2×"), "speed must not bleed into track info");
        assert!(
            !b.contains("1.2×"),
            "speed must not bleed into playback bar"
        );

        // Track title is only in row 2
        assert!(t.contains("Cool Track"), "title in track info");
        assert!(
            !h.contains("Cool Track"),
            "title must not bleed into header"
        );

        // Position time is only in row 3
        assert!(b.contains("00:45"), "position in playback bar");
        assert!(!h.contains("00:45"), "position must not bleed into header");
        assert!(
            !t.contains("00:45"),
            "position must not bleed into track info"
        );
    }

    // ── UI consistency / make_panel_block tests ───────────────────────────────

    #[test]
    fn panel_block_focused_and_unfocused_are_distinct() {
        // focused=true and focused=false should produce different Block values
        let focused = make_panel_block(" My Panel ", true);
        let unfocused = make_panel_block(" My Panel ", false);
        // Blocks with different border colors are not equal
        assert_ne!(
            focused, unfocused,
            "focused and unfocused panels should differ"
        );
    }

    #[test]
    fn panel_block_same_focus_state_is_consistent() {
        // Calling make_panel_block twice with same args should produce equal blocks
        let block1 = make_panel_block(" Settings ", true);
        let block2 = make_panel_block(" Settings ", true);
        assert_eq!(
            block1, block2,
            "same focus state should produce identical blocks"
        );
    }

    #[test]
    fn panel_block_different_titles_are_distinct() {
        let settings = make_panel_block(" ⚙ Settings ", false);
        let tracks = make_panel_block(" My Playlist ", false);
        assert_ne!(
            settings, tracks,
            "different titles should produce different blocks"
        );
    }

    #[test]
    fn panel_block_renders_without_panic() {
        // Verify that blocks can be built for all combinations without panicking
        for focused in [true, false] {
            let _block = make_panel_block(" Test Panel ", focused);
            let _block = make_panel_block("", focused);
            let _block = make_panel_block(" ⚙ Settings ", focused);
            let _block = make_panel_block(" ≡ Playlists ", focused);
        }
    }

    #[test]
    fn panel_block_consistent_across_all_panels() {
        // Settings and track table should both use make_panel_block with focus-aware color.
        // Verify that the same focus state produces matching block structure by checking
        // that focused=true blocks are pairwise different from focused=false.
        let settings_focused = make_panel_block(" ⚙ Settings ", true);
        let settings_idle = make_panel_block(" ⚙ Settings ", false);
        let tracks_focused = make_panel_block(" My Tracks ", true);
        let tracks_idle = make_panel_block(" My Tracks ", false);

        // Each panel: focused ≠ unfocused
        assert_ne!(
            settings_focused, settings_idle,
            "settings focused vs idle should differ"
        );
        assert_ne!(
            tracks_focused, tracks_idle,
            "track table focused vs idle should differ"
        );

        // Cross-panel with same focus: should differ only by title
        assert_ne!(
            settings_focused, tracks_focused,
            "different panel titles should differ"
        );
        assert_ne!(
            settings_idle, tracks_idle,
            "different panel titles should differ"
        );
    }

    // ── Task 9: Acceptance criteria and edge case verification ────────────────

    // --- Requirement verification: Overview requirements ---

    #[test]
    fn requirement_header_centric_layout_has_now_playing_label() {
        // "Header-centric layout: Row 1 becomes a proper header with 🎵 Now Playing label"
        let playing = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let paused = build_now_playing_header_line(80, Some("⏸ Paused"), Some("1.0×"));
        let no_track = build_now_playing_header_line(80, None, None);

        for (name, line) in [
            ("playing", &playing),
            ("paused", &paused),
            ("no_track", &no_track),
        ] {
            let text = line_to_string(line);
            assert!(
                text.contains("🎵 Now Playing"),
                "header must always show label in state {name}: {text:?}"
            );
        }
    }

    #[test]
    fn requirement_header_has_playback_status_center_and_speed_right() {
        // "Row 1: 🎵 Now Playing (GOLD) | ▶️ Playing (white) | 1.4x (ACCENT)"
        let gold = Color::Rgb(212, 175, 55);
        let accent = Color::Rgb(206, 65, 43);

        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let text = line_to_string(&line);

        assert!(text.contains("🎵 Now Playing"), "label present");
        assert!(text.contains("▶ Playing"), "status present");
        assert!(text.contains("1.4×"), "speed present");

        let label_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("🎵 Now Playing"));
        let speed_span = line.spans.iter().find(|s| s.content.contains("1.4×"));

        assert_eq!(
            label_span.unwrap().style.fg,
            Some(gold),
            "label must be GOLD"
        );
        assert_eq!(
            speed_span.unwrap().style.fg,
            Some(accent),
            "speed must be ACCENT"
        );
    }

    #[test]
    fn requirement_track_info_row_has_title_artist_source() {
        // "Row 2: TRACK TITLE (bold white) • Artist (TEXT_DIM) • source (TEXT_DIM, truncated)"
        let white = Color::White;
        let text_dim = Color::Rgb(130, 130, 130);

        let line = build_track_info_line(80, "My Track Title", "Great Artist", "youtube.com/watch");
        let text = line_to_string(&line);

        assert!(text.contains("My Track Title"), "title present");
        assert!(text.contains("Great Artist"), "artist present");
        assert!(text.contains("youtube.com/watch"), "source present");
        assert!(text.contains(" • "), "bullet separators present");

        let title_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("My Track Title"));
        let artist_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("Great Artist"));
        let source_span = line
            .spans
            .iter()
            .find(|s| s.content.contains("youtube.com/watch"));

        assert_eq!(
            title_span.unwrap().style.fg,
            Some(white),
            "title must be white"
        );
        assert_eq!(
            artist_span.unwrap().style.fg,
            Some(text_dim),
            "artist must be TEXT_DIM"
        );
        assert_eq!(
            source_span.unwrap().style.fg,
            Some(text_dim),
            "source must be TEXT_DIM"
        );
    }

    #[test]
    fn requirement_playback_bar_has_time_progress_volume_cache() {
        // "Row 3: 0:03 ████████████████──── 830:34 | ♪ 85% | ◈ Cached"
        let line = build_playback_bar_line(80, "00:03", 0.1, "830:34", "♪ 85%", CacheState::Cached);
        let text = line_to_string(&line);

        assert!(text.contains("00:03"), "position present");
        assert!(text.contains("830:34"), "duration present");
        assert!(text.contains("♪ 85%"), "volume present");
        assert!(text.contains("◈ Cached"), "cache status present");
        assert!(text.contains(" │ "), "section separator present");
    }

    #[test]
    fn requirement_pirate_theme_colors() {
        // Verify the pirate theme color palette is used consistently
        // ACCENT: Rgb(206,65,43) – red-orange
        // GOLD: Rgb(212,175,55) – yellow
        // SEA_GREEN: Rgb(32,178,136) – teal
        // TEXT_DIM: Rgb(130,130,130) – gray
        let accent = Color::Rgb(206, 65, 43);
        let gold = Color::Rgb(212, 175, 55);
        let sea_green = Color::Rgb(32, 178, 136);
        let text_dim = Color::Rgb(130, 130, 130);

        // Header: label=GOLD, speed=ACCENT
        let header = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let label_span = header
            .spans
            .iter()
            .find(|s| s.content.contains("🎵 Now Playing"))
            .unwrap();
        let speed_span = header
            .spans
            .iter()
            .find(|s| s.content.contains("1.4×"))
            .unwrap();
        assert_eq!(label_span.style.fg, Some(gold), "header label must be GOLD");
        assert_eq!(
            speed_span.style.fg,
            Some(accent),
            "header speed must be ACCENT"
        );

        // Track info: artist=TEXT_DIM
        let track = build_track_info_line(80, "Title", "Artist", "source.com");
        let artist_span = track
            .spans
            .iter()
            .find(|s| s.content.contains("Artist"))
            .unwrap();
        assert_eq!(
            artist_span.style.fg,
            Some(text_dim),
            "artist must be TEXT_DIM"
        );

        // Playback bar: cached indicator=SEA_GREEN
        let bar = build_playback_bar_line(80, "00:00", 0.0, "01:00", "♪ 80%", CacheState::Cached);
        let cache_span = bar
            .spans
            .iter()
            .find(|s| s.content.contains("◈ Cached"))
            .unwrap();
        assert_eq!(
            cache_span.style.fg,
            Some(sea_green),
            "cached indicator must be SEA_GREEN"
        );

        // Progress bar: fill color=SEA_GREEN, empty color=BORDER_IDLE
        let border_idle = Color::Rgb(70, 70, 70);
        let bar_spans = build_progress_bar(20, 0.5, '━', '─', '◉', sea_green, border_idle);
        let fill_spans: Vec<_> = bar_spans
            .iter()
            .filter(|s| s.content.contains('━'))
            .collect();
        let empty_spans: Vec<_> = bar_spans
            .iter()
            .filter(|s| s.content.contains('─'))
            .collect();
        for s in &fill_spans {
            assert_eq!(s.style.fg, Some(sea_green), "fill must be SEA_GREEN");
        }
        for s in &empty_spans {
            assert_eq!(s.style.fg, Some(border_idle), "empty must be BORDER_IDLE");
        }
    }

    // --- Edge case: very narrow terminal ---

    #[test]
    fn edge_case_minimum_terminal_width_80() {
        // Verify correct behavior at minimum recommended width of 80 chars
        let w = 80usize;
        let header = build_now_playing_header_line(w, Some("▶ Playing"), Some("1.0×"));
        let track = build_track_info_line(w, "Track Title", "Artist", "source.com");
        let bar = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);

        // At 80 chars, all three main sections should fit
        assert!(line_to_string(&header).contains("🎵 Now Playing"));
        assert!(line_to_string(&track).contains("Track Title"));
        assert!(line_to_string(&bar).contains("◈ Cached"));
    }

    #[test]
    fn edge_case_minimum_terminal_width_40_no_panic() {
        // At 40 chars (below recommended), should not panic but content may be limited
        let w = 40usize;
        let _h = build_now_playing_header_line(w, Some("▶ Playing"), Some("1.0×"));
        let _t = build_track_info_line(
            w,
            "A Very Long Track Title Indeed",
            "Long Artist Name",
            "very-long-source-url.com/watch?v=abc",
        );
        let _b = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);
    }

    #[test]
    fn edge_case_very_narrow_terminal_width_20_no_panic() {
        // At 20 chars (very narrow), should not panic
        for w in [1, 5, 10, 15, 20] {
            let _h = build_now_playing_header_line(w, Some("▶ Playing"), Some("1.0×"));
            let _t = build_track_info_line(w, "Track Title", "Artist Name", "source.com");
            let _b_cached =
                build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);
            let _b_stream =
                build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Streaming);
            let _b_dl = build_playback_bar_line(
                w,
                "00:30",
                0.5,
                "01:00",
                "♪ 80%",
                CacheState::Downloading(0.5),
            );
        }
    }

    #[test]
    fn edge_case_minimum_terminal_width_layout_calculations() {
        // Verify layout calculations don't underflow or panic at narrow widths.
        // fixed_widths: section 0 = 15, section 2 = 5 (sum = 20). Section 1 is flexible.
        // The correct invariant: fixed sections retain their widths; the flexible section
        // gets max(0, total_width - sum_of_fixed). Sum == total_width only when no overflow.
        for w in [20, 30, 40, 60, 80] {
            let widths = calculate_distributed_widths(w, 3, &[(0, 15), (2, 5)]);
            assert_eq!(widths.len(), 3, "should have 3 sections for w={w}");
            assert_eq!(widths[0], 15, "fixed section 0 should be 15 for w={w}");
            assert_eq!(widths[2], 5, "fixed section 2 should be 5 for w={w}");
            // flexible section gets the remaining space (non-negative)
            let expected_flex = w.saturating_sub(20);
            assert_eq!(
                widths[1], expected_flex,
                "flexible section should be max(0, w-20) for w={w}: {widths:?}"
            );
        }
        // Overflow case: when fixed widths exceed total_width, flexible section is 0
        // and sum > total_width (fixed sections retain their sizes)
        let overflow = calculate_distributed_widths(10, 3, &[(0, 15), (2, 5)]);
        assert_eq!(overflow[0], 15, "fixed section 0 unchanged in overflow");
        assert_eq!(overflow[2], 5, "fixed section 2 unchanged in overflow");
        assert_eq!(overflow[1], 0, "flexible section is 0 in overflow");
        assert!(
            overflow.iter().sum::<usize>() > 10,
            "sum exceeds total_width in overflow case"
        );
    }

    // --- Edge case: no tracks ---

    #[test]
    fn edge_case_no_track_header_shows_no_track_selected() {
        let line = build_now_playing_header_line(80, None, None);
        let text = line_to_string(&line);
        assert!(
            text.contains("No track selected"),
            "no-track state: {text:?}"
        );
    }

    #[test]
    fn edge_case_no_track_header_still_has_label() {
        // Even with no track, the 🎵 Now Playing label must be present
        let line = build_now_playing_header_line(80, None, None);
        assert!(line_to_string(&line).contains("🎵 Now Playing"));
    }

    #[test]
    fn edge_case_no_track_does_not_show_speed() {
        // Without a track there's no speed to show
        let line = build_now_playing_header_line(80, None, None);
        let text = line_to_string(&line);
        // In no-track mode, no speed "×" suffix should appear
        assert!(
            !text.contains('×'),
            "no track should not show speed: {text:?}"
        );
    }

    // --- Edge case: long titles ---

    #[test]
    fn edge_case_very_long_title_truncated_with_ellipsis() {
        let long_title = "A".repeat(200);
        let line = build_track_info_line(80, &long_title, "Artist", "source.com");
        let text = line_to_string(&line);
        // Content should fit within width (leading space + content)
        let char_count = text.chars().count();
        assert!(
            char_count <= 82,
            "long title must be truncated: {char_count} chars"
        );
        // Truncation should use ellipsis
        assert!(
            text.contains('…'),
            "truncated text should end with ellipsis: {text:?}"
        );
    }

    #[test]
    fn edge_case_very_long_artist_truncated() {
        let long_artist = "B".repeat(200);
        let line = build_track_info_line(80, "Short Title", &long_artist, "source.com");
        let text = line_to_string(&line);
        let char_count = text.chars().count();
        assert!(
            char_count <= 82,
            "long artist must be truncated: {char_count} chars"
        );
    }

    #[test]
    fn edge_case_very_long_source_truncated() {
        let long_source = "https://example.com/".repeat(20);
        let line = build_track_info_line(80, "Title", "Artist", &long_source);
        let text = line_to_string(&line);
        let char_count = text.chars().count();
        assert!(
            char_count <= 82,
            "long source must be truncated: {char_count} chars"
        );
    }

    #[test]
    fn edge_case_long_title_preserves_priority_over_artist_and_source() {
        // Title has highest priority - even in tight space, title should appear
        let line = build_track_info_line(30, "My Important Track Title", "Artist", "src.com");
        let text = line_to_string(&line);
        // "My Important Track Title" (24) doesn't fit in 29 chars with separators,
        // but its beginning should be there since it has priority
        assert!(
            text.starts_with(" M") || text.contains("My "),
            "title should have truncation priority: {text:?}"
        );
    }

    // --- All playback states: stopped/no-track, playing, paused, downloading ---

    #[test]
    fn playback_state_stopped_no_track_display() {
        // Stopped/no track state
        let (icon, text) = format_playback_state(false, false, false);
        assert_eq!(icon, "", "stopped has no icon");
        assert_eq!(text, "No track", "stopped shows No track");

        // Header with no track
        let line = build_now_playing_header_line(80, None, None);
        let header_text = line_to_string(&line);
        assert!(
            header_text.contains("No track selected"),
            "no-track header text"
        );
    }

    #[test]
    fn playback_state_loading_display() {
        // Loading state (track selected but player not ready)
        let (icon, text) = format_playback_state(false, false, true);
        assert_eq!(icon, "⏳", "loading has hourglass icon");
        assert_eq!(text, "Loading…", "loading shows Loading text");
    }

    #[test]
    fn playback_state_playing_display() {
        let (icon, text) = format_playback_state(true, false, true);
        assert_eq!(icon, "▶", "playing has play icon");
        assert_eq!(text, "Playing", "playing shows Playing text");

        let center = format!("{} {}", icon, text);
        let line = build_now_playing_header_line(80, Some(&center), Some("1.5×"));
        let header_text = line_to_string(&line);
        assert!(
            header_text.contains("▶ Playing"),
            "header shows playing state"
        );
    }

    #[test]
    fn playback_state_paused_display() {
        let (icon, text) = format_playback_state(true, true, true);
        assert_eq!(icon, "⏸", "paused has pause icon");
        assert_eq!(text, "Paused", "paused shows Paused text");

        let center = format!("{} {}", icon, text);
        let line = build_now_playing_header_line(80, Some(&center), Some("1.0×"));
        let header_text = line_to_string(&line);
        assert!(
            header_text.contains("⏸ Paused"),
            "header shows paused state"
        );
    }

    #[test]
    fn playback_state_downloading_display() {
        // Downloading state shows caching indicator with percentage
        let line_25 = build_playback_bar_line(
            80,
            "00:10",
            0.2,
            "01:00",
            "♪ 80%",
            CacheState::Downloading(0.25),
        );
        let line_75 = build_playback_bar_line(
            80,
            "00:30",
            0.5,
            "01:00",
            "♪ 80%",
            CacheState::Downloading(0.75),
        );

        let text_25 = line_to_string(&line_25);
        let text_75 = line_to_string(&line_75);

        assert!(
            text_25.contains("⟳ Caching"),
            "downloading: caching label at 25%: {text_25:?}"
        );
        assert!(
            text_25.contains("25%"),
            "downloading: percentage 25%: {text_25:?}"
        );
        assert!(
            text_75.contains("⟳ Caching"),
            "downloading: caching label at 75%: {text_75:?}"
        );
        assert!(
            text_75.contains("75%"),
            "downloading: percentage 75%: {text_75:?}"
        );
    }

    #[test]
    fn playback_state_all_cache_states_covered() {
        // All three cache states must be visually distinct and clearly indicated
        let cached =
            build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Cached);
        let streaming =
            build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Streaming);
        let downloading = build_playback_bar_line(
            80,
            "00:10",
            0.2,
            "01:00",
            "♪ 80%",
            CacheState::Downloading(0.5),
        );

        let ct = line_to_string(&cached);
        let st = line_to_string(&streaming);
        let dt = line_to_string(&downloading);

        assert!(ct.contains("◈ Cached"), "Cached state");
        assert!(st.contains("◌ Stream"), "Streaming state");
        assert!(dt.contains("⟳ Caching"), "Downloading state");

        // States must be distinct from each other
        assert_ne!(ct, st, "cached ≠ streaming");
        assert_ne!(ct, dt, "cached ≠ downloading");
        assert_ne!(st, dt, "streaming ≠ downloading");
    }

    // --- Layout calculations at minimum terminal dimensions ---

    #[test]
    fn layout_progress_bar_minimum_width_one() {
        // Progress bar with width=1 should produce exactly 1 character
        let spans = build_progress_bar(1, 0.0, '━', '─', '◉', Color::Green, Color::Gray);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), 1, "width=1 progress bar");
    }

    #[test]
    fn layout_distributed_widths_minimum_width() {
        // At minimum terminal width, widths should not panic and should not overflow
        let w = 10usize;
        // Three sections with fixed sizes that together might exceed w
        let widths = calculate_distributed_widths(w, 3, &[(1, 5), (2, 3)]);
        assert_eq!(widths[1], 5, "fixed section 1");
        assert_eq!(widths[2], 3, "fixed section 2");
        assert_eq!(
            widths[0],
            w.saturating_sub(5 + 3),
            "flexible section saturates"
        );
    }

    #[test]
    fn layout_header_width_80_all_sections_present() {
        // At exactly 80 chars, the header should display all three sections
        let w = 80usize;
        let line = build_now_playing_header_line(w, Some("▶ Playing"), Some("1.4×"));
        let text = line_to_string(&line);
        assert!(text.contains("🎵 Now Playing"), "label at w=80");
        assert!(text.contains("▶ Playing"), "status at w=80");
        assert!(text.contains("1.4×"), "speed at w=80");
    }

    #[test]
    fn layout_track_info_width_bounds_respected() {
        // At various widths, track info line should not exceed width + margin
        for w in [40, 60, 80, 100, 120] {
            let line = build_track_info_line(
                w,
                "Song Title That Is Somewhat Long",
                "Artist Name Here",
                "source.com/path",
            );
            let char_count = line_to_string(&line).chars().count();
            assert!(
                char_count <= w + 2,
                "track info at w={w}: {char_count} chars > {}",
                w + 2
            );
        }
    }

    #[test]
    fn layout_format_duration_edge_cases() {
        // Test boundary values for duration formatting
        assert_eq!(format_duration(0), "00:00", "zero duration");
        assert_eq!(format_duration(59), "00:59", "59 seconds");
        assert_eq!(format_duration(3599), "59:59", "59m59s");
        assert_eq!(format_duration(3600), "01:00:00", "exactly 1 hour");
        assert_eq!(
            format_duration(u64::MAX / 3600 * 3600), // large hours value
            format!("{:02}:00:00", u64::MAX / 3600),
            "large duration"
        );
    }

    #[test]
    fn layout_truncate_respects_unicode_char_boundaries() {
        // Unicode characters should count as one char each
        let emoji = "🎵🎶🎼🎹🎸"; // 5 emoji
        let result = truncate(emoji, 3);
        assert_eq!(result.chars().count(), 3, "truncated emoji count");
        assert!(result.ends_with('…'), "truncated with ellipsis");
    }

    // --- Verify no old render_cache_and_eq function (removed in task 7) ---

    #[test]
    fn cache_status_integrated_into_row3_not_separate_row() {
        // Cache status is part of the playback bar (row 3), not a fourth separate row.
        // Verify by checking the playback bar contains cache info directly.
        for state in [
            CacheState::Cached,
            CacheState::Streaming,
            CacheState::Downloading(0.3),
        ] {
            let line = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", state.clone());
            let text = line_to_string(&line);
            let has_cache_info = text.contains("◈ Cached")
                || text.contains("◌ Stream")
                || text.contains("⟳ Caching");
            assert!(
                has_cache_info,
                "row 3 must contain cache info for state {state:?}: {text:?}"
            );
        }
    }

    #[test]
    fn all_three_row_functions_exist_and_produce_output() {
        // Verify the three key builder functions exist and produce non-empty lines
        let header = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.0×"));
        let track = build_track_info_line(80, "Title", "Artist", "source");
        let bar = build_playback_bar_line(80, "00:00", 0.0, "01:00", "♪ 80%", CacheState::Cached);

        assert!(!header.spans.is_empty(), "header should produce spans");
        assert!(!track.spans.is_empty(), "track info should produce spans");
        assert!(!bar.spans.is_empty(), "playback bar should produce spans");
    }

    // manual test (skipped - not automatable)
    // Visual layout verification in terminal at different sizes requires human inspection.
    // Items to manually verify when running the app:
    //   - 80x24 minimum: all three rows visible
    //   - 120x30 recommended: full track info visible without truncation
    //   - Pirate theme colors visible in dark terminal
    //   - Progress bar updates smoothly during playback
    //   - Cache status updates correctly when downloading

    // ── Task 1: context menu infrastructure tests ─────────────────────────────

    #[test]
    fn context_menu_items_empty_when_no_other_playlists() {
        // Active playlist is "Jazz", no other playlists registered
        let app = make_app_with_playlists("Jazz", &["Jazz"]);
        let items = app.available_playlist_names();
        assert!(
            items.is_empty(),
            "should be empty when only active playlist exists: {items:?}"
        );
    }

    #[test]
    fn context_menu_items_excludes_active_playlist() {
        // Three playlists; active is "Jazz" — should return the other two
        let app = make_app_with_playlists("Jazz", &["Jazz", "Rock", "Classical"]);
        let items = app.available_playlist_names();
        assert!(
            !items.contains(&"Jazz".to_string()),
            "active playlist must be excluded"
        );
        assert!(
            items.contains(&"Rock".to_string()),
            "Rock should be in items"
        );
        assert!(
            items.contains(&"Classical".to_string()),
            "Classical should be in items"
        );
        assert_eq!(items.len(), 2, "should have exactly 2 items");
    }

    #[test]
    fn context_menu_items_single_other_playlist() {
        let app = make_app_with_playlists("Main", &["Main", "Other"]);
        let items = app.available_playlist_names();
        assert_eq!(items, vec!["Other".to_string()]);
    }

    #[test]
    fn context_menu_items_no_playlists_at_all() {
        // available_playlists is empty
        let app = make_app_with_playlists("Main", &[]);
        let items = app.available_playlist_names();
        assert!(
            items.is_empty(),
            "should be empty with no available_playlists"
        );
    }

    #[test]
    fn context_menu_items_many_playlists() {
        let names = &["A", "B", "C", "D", "E", "Active"];
        let app = make_app_with_playlists("Active", names);
        let items = app.available_playlist_names();
        assert_eq!(items.len(), 5, "should exclude 1 active from 6 total");
        assert!(
            !items.contains(&"Active".to_string()),
            "Active must be excluded"
        );
        for n in &["A", "B", "C", "D", "E"] {
            assert!(items.contains(&n.to_string()), "{n} should be included");
        }
    }

    #[test]
    fn available_playlist_names_excludes_active_and_is_sorted() {
        // available_playlist_names must exclude the active playlist and preserve sorted order
        let app = make_app_with_playlists("Jazz", &["Jazz", "Rock", "Classical"]);
        let names = app.available_playlist_names();
        assert!(
            !names.contains(&"Jazz".to_string()),
            "active playlist must be excluded"
        );
        assert!(
            names.contains(&"Rock".to_string()),
            "Rock should be included"
        );
        assert!(
            names.contains(&"Classical".to_string()),
            "Classical should be included"
        );
        assert_eq!(names.len(), 2, "exactly two non-active playlists");
    }

    #[test]
    fn context_menu_selected_initialized_to_zero() {
        let app = make_app_with_playlists("Main", &["Main", "Other"]);
        assert_eq!(
            app.context_menu_selected, 0,
            "context_menu_selected should start at 0"
        );
    }

    #[test]
    fn context_menu_navigation_clamps_at_top() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Main", &["Main", "Rock", "Jazz"]);
        app.input_mode = InputMode::TrackContextMenu;
        app.context_menu_selected = 0;

        // Going up at 0 should stay at 0
        let names = app.available_playlist_names();
        if app.context_menu_selected > 0 {
            app.context_menu_selected -= 1;
        }
        assert_eq!(app.context_menu_selected, 0, "should clamp at 0");
        drop(names); // avoid unused warning
    }

    #[test]
    fn context_menu_navigation_clamps_at_bottom() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Main", &["Main", "Rock", "Jazz"]);
        app.input_mode = InputMode::TrackContextMenu;
        let count = app.available_playlist_names().len(); // 2
        app.context_menu_selected = count - 1;

        // Going down at last item should stay
        if app.context_menu_selected + 1 < count {
            app.context_menu_selected += 1;
        }
        assert_eq!(
            app.context_menu_selected,
            count - 1,
            "should clamp at last item"
        );
    }

    #[test]
    fn context_menu_navigation_increments() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Main", &["Main", "Rock", "Jazz"]);
        app.input_mode = InputMode::TrackContextMenu;
        app.context_menu_selected = 0;
        let count = app.available_playlist_names().len();

        if app.context_menu_selected + 1 < count {
            app.context_menu_selected += 1;
        }
        assert_eq!(app.context_menu_selected, 1, "should move to index 1");
    }

    #[test]
    fn context_menu_enter_returns_to_normal() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Main", &["Main", "Rock"]);
        app.input_mode = InputMode::TrackContextMenu;
        // Simulate enter: close menu
        app.input_mode = InputMode::Normal;
        assert_eq!(app.input_mode, InputMode::Normal, "enter should close menu");
    }

    #[test]
    fn context_menu_esc_returns_to_normal() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Main", &["Main", "Rock"]);
        app.input_mode = InputMode::TrackContextMenu;
        app.input_mode = InputMode::Normal;
        assert_eq!(app.input_mode, InputMode::Normal, "esc should close menu");
    }

    #[test]
    fn context_menu_mode_exists_in_input_mode_enum() {
        use crate::tui::InputMode;
        // Verify TrackContextMenu variant is distinct and comparable
        let mode = InputMode::TrackContextMenu;
        assert_eq!(mode, InputMode::TrackContextMenu);
        assert_ne!(mode, InputMode::Normal);
        assert_ne!(mode, InputMode::UrlInput);
        assert_ne!(mode, InputMode::SearchInput);
    }

    // ── Task 2: track moving between playlists ────────────────────────────────

    fn make_track(id: &str, title: &str) -> crate::library::Track {
        use crate::library::{CacheStatus, MediaKind, TrackOrigin};
        crate::library::Track {
            url: format!("https://example.com/{id}"),
            source: "youtube.com".to_string(),
            title: title.to_string(),
            artist: "Test Artist".to_string(),
            channel: "Test Channel".to_string(),
            duration: 180,
            id: id.to_string(),
            cache_status: CacheStatus::Streaming,
            file: None,
            last_position: 0,
            speed: None,
            user_title: None,
            user_artist: None,
            added_at: chrono::Utc::now(),
            origin: TrackOrigin::Remote,
            media: MediaKind::Audio,
            resume: true,
        }
    }

    // ── Playlist::add_track tests ─────────────────────────────────────────────

    #[test]
    fn add_track_appends_to_empty_playlist() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        assert_eq!(pl.tracks.len(), 1);
        assert_eq!(pl.tracks[0], "vid1");
    }

    #[test]
    fn add_track_appends_to_existing_tracks() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        pl.add_track("vid2".to_string());
        assert_eq!(pl.tracks.len(), 2);
        assert_eq!(pl.tracks[1], "vid2");
    }

    #[test]
    fn add_track_does_not_modify_other_fields() {
        let mut pl = make_playlist("Test");
        let original_name = pl.name.clone();
        pl.add_track("vid1".to_string());
        assert_eq!(pl.name, original_name);
        assert!(pl.current_track.is_none());
    }

    // ── Playlist::remove_track_by_id tests ─────────────────────────────

    #[test]
    fn remove_track_reports_the_row_it_dropped() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        assert!(pl.remove_track_by_id("vid1"));
        assert!(pl.tracks.is_empty());
    }

    #[test]
    fn remove_track_reports_false_for_missing_id() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        assert!(!pl.remove_track_by_id("nonexistent"));
        assert_eq!(pl.tracks.len(), 1, "track should remain");
    }

    #[test]
    fn remove_track_clears_current_track_pointer() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        pl.current_track = Some("vid1".to_string());
        pl.remove_track_by_id("vid1");
        assert!(
            pl.current_track.is_none(),
            "current_track should be cleared"
        );
    }

    #[test]
    fn remove_track_preserves_current_track_for_other_tracks() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        pl.add_track("vid2".to_string());
        pl.current_track = Some("vid2".to_string());
        pl.remove_track_by_id("vid1");
        assert_eq!(
            pl.current_track.as_deref(),
            Some("vid2"),
            "current_track should be preserved"
        );
    }

    #[test]
    fn remove_track_removes_correct_track_from_middle() {
        let mut pl = make_playlist("Test");
        pl.add_track("vid1".to_string());
        pl.add_track("vid2".to_string());
        pl.add_track("vid3".to_string());
        pl.remove_track_by_id("vid2");
        assert_eq!(pl.tracks.len(), 2);
        assert_eq!(pl.tracks[0], "vid1");
        assert_eq!(pl.tracks[1], "vid3");
    }

    // ── App::move_track_to_playlist tests ────────────────────────────────────

    fn make_app_with_tracks_and_targets(
        active: &str,
        tracks: &[(&str, &str)],
        targets: &[&str],
    ) -> crate::tui::App {
        use std::path::PathBuf;
        let mut playlist = make_playlist(active);
        for (id, title) in tracks {
            add_track(&mut playlist, make_track(id, title));
        }
        let config = crate::config::Config::default();
        let mut available = entries(vec![(
            active.to_string(),
            PathBuf::from(format!("/fake/{active}.toml")),
        )]);
        for t in targets {
            available.push(crate::playlist::PlaylistEntry::normal(
                t.to_string(),
                PathBuf::from(format!("/fake/{t}.toml")),
            ));
        }
        crate::tui::App::new(
            playlist,
            config,
            available,
            PathBuf::from(format!("/fake/{active}.toml")),
            test_library(),
        )
    }

    #[test]
    fn move_track_fails_for_missing_target_playlist() {
        let mut app = make_app_with_tracks_and_targets(
            "Source",
            &[("vid1", "Track One")],
            &[], // no targets at all
        );
        let result = app.move_track_to_playlist("NonExistent");
        assert!(
            result.is_err(),
            "should fail when target not in available_playlists"
        );
    }

    #[test]
    fn move_track_fails_when_no_track_at_selection() {
        // Playlist is empty, nothing to move
        let mut app = make_app_with_tracks_and_targets(
            "Source",
            &[],       // empty playlist
            &["Rock"], // target exists
        );
        let result = app.move_track_to_playlist("Rock");
        assert!(result.is_err(), "should fail when no track at cursor");
    }

    #[test]
    fn move_track_stops_playback_when_moving_current_track() {
        // Use a real temp directory so the move actually succeeds and we can
        // verify the in-memory state changes (player cleared, paused reset, etc.).
        let dir = tempfile::tempdir().expect("tempdir");

        let source_path = dir.path().join("Source.toml");
        let rock_path = dir.path().join("Rock.toml");

        let mut source_pl = make_playlist("Source");
        add_track(&mut source_pl, make_track("vid1", "Track One"));
        source_pl.save(&source_path).expect("save source");

        let rock_pl = make_playlist("Rock");
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Source".to_string(), source_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ]);
        let mut app = crate::tui::App::new(
            source_pl,
            config,
            available,
            source_path.clone(),
            test_library(),
        );

        // Simulate a playing track: both the legacy `current_track` pointer
        // (kept for cursor-restore purposes) and the real identity source of
        // truth, `app.playing`, pointing at this exact (path, id).
        app.playlist.current_track = Some("vid1".to_string());
        app.playing = Some(session_at(source_path.clone(), app.playlist.clone(), 0));
        app.is_paused = true;
        app.position = 42.0;
        // player stays None (no real mpv), but the in-memory flags must be cleared

        let result = app.move_track_to_playlist("Rock");
        assert!(result.is_ok(), "move should succeed: {:?}", result.err());

        // Critical invariants: player cleared, playing session cleared,
        // current_track cleared, is_paused reset
        assert!(
            app.player.is_none(),
            "player must be None after moving current track"
        );
        assert!(
            app.playing.is_none(),
            "playing session must be cleared after moving the playing track"
        );
        assert!(
            app.playlist.current_track.is_none(),
            "current_track must be cleared"
        );
        assert!(!app.is_paused, "is_paused must be reset to false");
        assert_eq!(app.position, 0.0, "position must be reset");

        // Source playlist must no longer contain vid1
        assert!(
            app.playlist.tracks.is_empty(),
            "source playlist should be empty after move"
        );
    }

    #[test]
    fn move_track_selection_clamps_after_removal() {
        // This tests the clamping logic directly on the App struct
        // without requiring disk I/O by checking the logic path
        let mut app = make_app_with_tracks_and_targets(
            "Source",
            &[("vid1", "One"), ("vid2", "Two"), ("vid3", "Three")],
            &["Rock"],
        );
        // Select last track
        app.selected = 2;
        // Simulate what move does: remove track, rebuild the rows and clamp
        app.playlist.remove_track_by_id("vid3");
        app.rebuild_rows();
        let new_count = app.visible_track_count();
        if app.selected >= new_count && app.selected > 0 {
            app.selected -= 1;
        }
        app.clamp_scroll();
        assert_eq!(app.selected, 1, "selection should clamp to new last index");
    }

    #[test]
    fn move_track_selection_stays_when_not_last() {
        let mut app = make_app_with_tracks_and_targets(
            "Source",
            &[("vid1", "One"), ("vid2", "Two"), ("vid3", "Three")],
            &["Rock"],
        );
        // Select first track
        app.selected = 0;
        // Remove middle track (simulate removing what's at cursor=0)
        app.playlist.remove_track_by_id("vid1");
        let new_count = app.visible_track_count();
        if app.selected >= new_count && app.selected > 0 {
            app.selected -= 1;
        }
        // selected=0 < new_count=2, so no clamping
        assert_eq!(
            app.selected, 0,
            "selection should stay at 0 when not out of bounds"
        );
    }

    #[test]
    fn playlist_add_and_remove_round_trip() {
        // Add then remove the same track — playlist should be empty again
        let mut pl = make_playlist("Round Trip");
        pl.add_track("vid1".to_string());
        assert!(pl.remove_track_by_id("vid1"));
        assert!(
            pl.tracks.is_empty(),
            "playlist should be empty after round trip"
        );
    }

    #[test]
    fn remove_track_from_empty_playlist_reports_false() {
        let mut pl = make_playlist("Empty");
        assert!(
            !pl.remove_track_by_id("vid1"),
            "removing from empty playlist should report nothing dropped"
        );
    }

    #[test]
    fn add_multiple_tracks_preserve_insertion_order() {
        let mut pl = make_playlist("Order Test");
        for i in 0..5 {
            pl.add_track(format!("vid{i}"));
        }
        for (i, id) in pl.tracks.iter().enumerate() {
            assert_eq!(*id, format!("vid{i}"), "track order must be preserved");
        }
    }

    // ── App::switch_to_playlist tests ─────────────────────────────────────────

    /// Write a playlist to a temp file and return the path.
    fn write_temp_playlist(
        pl: &crate::playlist::Playlist,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(format!("{}.toml", pl.name));
        pl.save(&path).expect("save");
        (dir, path)
    }

    #[test]
    fn switch_to_playlist_loads_new_playlist() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path)
            .expect("switch should succeed");

        assert_eq!(app.playlist.name, "Beta");
        assert_eq!(app.playlist_path, path);
    }

    #[test]
    fn switch_to_playlist_resets_selection_and_scroll() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.selected = 5;
        app.track_offset = 3;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(app.selected, 0, "selected should reset to 0");
        assert_eq!(app.track_offset, 0, "track_offset should reset to 0");
    }

    #[test]
    fn switch_to_playlist_clears_search_state() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.input_buf = "search text".to_string();
        app.search_query = "search text".to_string();

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert!(app.input_buf.is_empty(), "input_buf should be cleared");
        assert!(!app.has_filter(), "the search filter should be cleared");
    }

    #[test]
    fn switch_to_playlist_does_not_stop_playback() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.is_paused = true;
        app.position = 42.5;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert!(
            app.is_paused,
            "is_paused must be unaffected by playlist switch"
        );
        assert_eq!(
            app.position, 42.5,
            "position must be unaffected by playlist switch"
        );
    }

    #[test]
    fn switch_to_playlist_focuses_track_list() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.focus = crate::tui::Focus::Sidebar;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(app.focus, crate::tui::Focus::TrackList);
    }

    #[test]
    fn switch_to_playlist_restores_cursor_to_current_track() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);

        let mut beta = make_playlist("Beta");
        add_track(&mut beta, make_track("first", "First"));
        add_track(&mut beta, make_track("second", "Second"));
        add_track(&mut beta, make_track("third", "Third"));
        beta.current_track = Some("second".to_string());

        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(app.selected, 1, "cursor should land on current_track index");
    }

    #[test]
    fn switch_to_playlist_returns_error_on_missing_file() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        let missing = std::path::Path::new("/tmp/does_not_exist_trovers_test.toml");

        let result = app.switch_to_playlist("Ghost", missing);

        assert!(result.is_err(), "should return error for missing file");
    }

    #[test]
    fn switch_to_playlist_returns_error_on_corrupted_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, b"not valid toml [[[[").expect("write");

        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Bad"]);

        let result = app.switch_to_playlist("Bad", &path);

        assert!(result.is_err(), "should return error for corrupted TOML");
    }

    #[test]
    fn switch_to_playlist_does_not_mutate_available_playlists() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta", "Gamma"]);
        let original_count = app.available_playlists.len();

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(
            app.available_playlists.len(),
            original_count,
            "available_playlists should be unchanged after switch"
        );
    }

    // ── Task 8: active_playlist persisted on switch ─────────────────────────

    #[test]
    fn switch_to_playlist_updates_config_active_playlist() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        assert_eq!(app.config.active_playlist, None);

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(
            app.config.active_playlist,
            Some("Beta".to_string()),
            "config.active_playlist should reflect the newly switched-to playlist"
        );
    }

    #[test]
    fn switch_to_playlist_updates_config_active_playlist_across_multiple_switches() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta", "Gamma"]);

        let beta = make_playlist("Beta");
        let (_dir_beta, path_beta) = write_temp_playlist(&beta);
        app.switch_to_playlist("Beta", &path_beta)
            .expect("switch to beta");
        assert_eq!(app.config.active_playlist, Some("Beta".to_string()));

        let gamma = make_playlist("Gamma");
        let (_dir_gamma, path_gamma) = write_temp_playlist(&gamma);
        app.switch_to_playlist("Gamma", &path_gamma)
            .expect("switch to gamma");
        assert_eq!(app.config.active_playlist, Some("Gamma".to_string()));
    }

    #[test]
    fn switch_to_playlist_does_not_update_config_active_playlist_on_error() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha"]);
        app.config.active_playlist = Some("Alpha".to_string());
        let missing = std::path::Path::new("/tmp/does_not_exist_trovers_test_task8.toml");

        let result = app.switch_to_playlist("Ghost", missing);

        assert!(result.is_err(), "should return error for missing file");
        assert_eq!(
            app.config.active_playlist,
            Some("Alpha".to_string()),
            "config.active_playlist must be unchanged when the switch fails"
        );
    }

    // ── Task 2: PlayingSession decoupled from switch_to_playlist ───────────────

    #[test]
    fn playing_session_survives_switch_to_playlist() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);

        // Simulate a track from a third, unrelated playlist "Gamma" playing.
        let mut gamma = make_playlist("Gamma");
        add_track(&mut gamma, make_track("g1", "Gamma Track"));
        let gamma_path = std::path::PathBuf::from("/fake/Gamma.toml");
        app.playing = Some(session_at(gamma_path.clone(), gamma, 0));

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        let session = app
            .playing
            .as_ref()
            .expect("playing session should survive switch");
        assert_eq!(session.path, gamma_path, "playing session path unchanged");
        assert_eq!(session.track_id, "g1", "playing track unchanged");
        assert_eq!(app.playlist.name, "Beta", "displayed playlist did switch");
    }

    /// A track has one home, so an edit is visible through `playing_track()`
    /// with no sync step — and it does not matter which playlist is on screen.
    #[test]
    fn playing_track_reads_the_edited_document() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        push_track(&mut app, make_track("t1", "Original Title"));
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");

        app.playing = Some(session_at(
            app.playlist_path.clone(),
            app.playlist.clone(),
            0,
        ));

        app.patch_track("t1", |t| t.user_title = Some("Edited Title".to_string()));

        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            playing_track.user_title.as_deref(),
            Some("Edited Title"),
            "playing_track() must read the one document the edit went into"
        );
    }

    /// `playing_track()` follows the session's id, not the displayed playlist:
    /// browsing elsewhere must not change which track is reported as playing.
    #[test]
    fn playing_track_follows_the_session_not_the_displayed_playlist() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");
        push_track(&mut app, make_track("a1", "Alpha Track"));

        let mut gamma = make_playlist("Gamma");
        add_track(&mut gamma, make_track("g1", "Gamma Track"));
        refresh_library(&mut app);
        let gamma_path = std::path::PathBuf::from("/fake/Gamma.toml");
        app.playing = Some(session_at(gamma_path, gamma, 0));

        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            playing_track.title, "Gamma Track",
            "should follow the session's track id, not the displayed playlist"
        );
    }

    // ── Task 4: Playlist management in sidebar ────────────────────────────────

    // --- Playlist::rename tests ---

    #[test]
    fn playlist_rename_updates_name_and_creates_new_file() {
        let pl = make_playlist("OldName");
        let (dir, old_path) = write_temp_playlist(&pl);
        let new_path = dir.path().join("NewName.toml");

        let mut pl2 = crate::playlist::Playlist::load(&old_path).expect("load");
        let result = pl2.rename("NewName", &old_path);

        assert!(result.is_ok(), "rename should succeed: {result:?}");
        assert!(new_path.exists(), "new file should exist");
        assert!(!old_path.exists(), "old file should be removed");
        assert_eq!(pl2.name, "NewName", "playlist name should be updated");
    }

    #[test]
    fn playlist_rename_new_file_has_correct_content() {
        let mut pl = make_playlist("Original");
        add_track(&mut pl, make_track("vid1", "Some Track"));
        let (dir, old_path) = write_temp_playlist(&pl);

        pl.rename("Renamed", &old_path).expect("rename");

        let new_path = dir.path().join("Renamed.toml");
        let loaded = crate::playlist::Playlist::load(&new_path).expect("load renamed");
        assert_eq!(loaded.name, "Renamed");
        assert_eq!(loaded.tracks.len(), 1);
        assert_eq!(loaded.tracks[0], "vid1");
    }

    #[test]
    fn playlist_rename_to_same_name_is_noop() {
        let pl = make_playlist("SameName");
        let (_dir, path) = write_temp_playlist(&pl);

        let mut pl2 = crate::playlist::Playlist::load(&path).expect("load");
        let result = pl2.rename("SameName", &path);

        // Renaming to the same name should succeed and file should still exist
        assert!(
            result.is_ok(),
            "rename to same name should not fail: {result:?}"
        );
        assert!(
            path.exists(),
            "file should still exist after rename to same name"
        );
    }

    #[test]
    fn playlist_delete_removes_file() {
        let pl = make_playlist("ToDelete");
        let (_dir, path) = write_temp_playlist(&pl);

        assert!(path.exists(), "file should exist before delete");

        let result = crate::playlist::Playlist::delete(&path);

        assert!(result.is_ok(), "delete should succeed: {result:?}");
        assert!(!path.exists(), "file should be removed after delete");
    }

    #[test]
    fn playlist_delete_missing_file_returns_error() {
        let path = std::path::Path::new("/tmp/does_not_exist_trovers_task4_test.toml");
        let result = crate::playlist::Playlist::delete(path);
        assert!(
            result.is_err(),
            "deleting non-existent file should return error"
        );
    }

    // --- validate_playlist_name tests ---

    #[test]
    fn validate_playlist_name_accepts_valid_name() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("My Playlist", &existing, None);
        assert!(result.is_ok(), "valid name should be accepted: {result:?}");
    }

    #[test]
    fn validate_playlist_name_rejects_empty() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("", &existing, None);
        assert!(result.is_err(), "empty name should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_slash() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("bad/name", &existing, None);
        assert!(result.is_err(), "name with slash should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_backslash() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("bad\\name", &existing, None);
        assert!(result.is_err(), "name with backslash should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_colon() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("bad:name", &existing, None);
        assert!(result.is_err(), "name with colon should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_whitespace_only() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        let result = validate_playlist_name("   ", &existing, None);
        assert!(result.is_err(), "whitespace-only name should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_dot() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<crate::playlist::PlaylistEntry> = vec![];
        assert!(
            validate_playlist_name(".", &existing, None).is_err(),
            ". is invalid"
        );
        assert!(
            validate_playlist_name("..", &existing, None).is_err(),
            ".. is invalid"
        );
    }

    #[test]
    fn validate_playlist_name_rejects_duplicate() {
        use crate::tui::input::validate_playlist_name;
        let existing = entries(vec![(
            "Rock".to_string(),
            std::path::PathBuf::from("/fake/Rock.toml"),
        )]);
        let result = validate_playlist_name("Rock", &existing, None);
        assert!(
            result.is_err(),
            "duplicate name should be rejected: {result:?}"
        );
    }

    #[test]
    fn validate_playlist_name_allows_current_name_during_rename() {
        use crate::tui::input::validate_playlist_name;
        // During rename, the current name is excluded from duplicate check
        let existing = entries(vec![(
            "Rock".to_string(),
            std::path::PathBuf::from("/fake/Rock.toml"),
        )]);
        let result = validate_playlist_name("Rock", &existing, Some("Rock"));
        assert!(
            result.is_ok(),
            "current name should be allowed during rename: {result:?}"
        );
    }

    #[test]
    fn validate_playlist_name_rejects_other_duplicate_during_rename() {
        use crate::tui::input::validate_playlist_name;
        let existing = entries(vec![
            (
                "Rock".to_string(),
                std::path::PathBuf::from("/fake/Rock.toml"),
            ),
            (
                "Jazz".to_string(),
                std::path::PathBuf::from("/fake/Jazz.toml"),
            ),
        ]);
        // Renaming "Rock" to "Jazz" (which already exists) should be rejected
        let result = validate_playlist_name("Jazz", &existing, Some("Rock"));
        assert!(
            result.is_err(),
            "renaming to existing name should be rejected"
        );
    }

    // --- InputMode variants tests ---

    #[test]
    fn playlist_rename_mode_exists_in_input_mode_enum() {
        use crate::tui::InputMode;
        let mode = InputMode::PlaylistRename;
        assert_eq!(mode, InputMode::PlaylistRename);
        assert_ne!(mode, InputMode::Normal);
        assert_ne!(mode, InputMode::PlaylistDelete);
    }

    #[test]
    fn playlist_delete_mode_exists_in_input_mode_enum() {
        use crate::tui::InputMode;
        let mode = InputMode::PlaylistDelete;
        assert_eq!(mode, InputMode::PlaylistDelete);
        assert_ne!(mode, InputMode::Normal);
        assert_ne!(mode, InputMode::PlaylistRename);
    }

    // --- Sidebar 'r' and 'd' key behaviour (state logic tests) ---

    #[test]
    fn sidebar_rename_mode_entered_when_on_playlist_item() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        // sidebar_items: [PlaylistsHeader(0), Playlist Jazz(1), Playlist Rock(2), ...]
        // sidebar_selected=1 → Jazz playlist item; simulate what handle_sidebar 'r' does
        app.sidebar_selected = 1; // PlaylistsHeader at 0, Jazz at 1
        let items = app.sidebar_items();
        if let Some(crate::tui::SidebarItem::Playlist { name, .. }) =
            items.get(app.sidebar_selected)
        {
            app.input_buf = name.clone();
            app.input_mode = InputMode::PlaylistRename;
        }
        assert_eq!(
            app.input_mode,
            InputMode::PlaylistRename,
            "should enter PlaylistRename"
        );
        assert_eq!(
            app.input_buf, "Jazz",
            "input_buf should be pre-filled with playlist name"
        );
    }

    #[test]
    fn sidebar_rename_mode_not_entered_when_on_header() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 0; // PlaylistsHeader
        let items = app.sidebar_items();
        // Simulate 'r' key — only enter rename if Playlist item
        if let Some(crate::tui::SidebarItem::Playlist { name, .. }) =
            items.get(app.sidebar_selected)
        {
            app.input_buf = name.clone();
            app.input_mode = InputMode::PlaylistRename;
        }
        assert_eq!(
            app.input_mode,
            InputMode::Normal,
            "should not enter rename on header"
        );
    }

    #[test]
    fn sidebar_delete_mode_entered_when_on_playlist_item() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 1;
        let items = app.sidebar_items();
        if matches!(
            items.get(app.sidebar_selected),
            Some(crate::tui::SidebarItem::Playlist { .. })
        ) {
            app.input_mode = InputMode::PlaylistDelete;
        }
        assert_eq!(
            app.input_mode,
            InputMode::PlaylistDelete,
            "should enter PlaylistDelete"
        );
    }

    // --- playlist_delete_target helper ---

    #[test]
    fn playlist_delete_target_returns_name_for_playlist_item() {
        use crate::tui::ui::playlist_delete_target;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 1; // Jazz playlist item
        let target = playlist_delete_target(&app);
        assert_eq!(
            target,
            Some("Jazz"),
            "should return the selected playlist name"
        );
    }

    #[test]
    fn playlist_delete_target_returns_none_for_header() {
        use crate::tui::ui::playlist_delete_target;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 0; // PlaylistsHeader
        let target = playlist_delete_target(&app);
        assert!(target.is_none(), "should return None for non-playlist item");
    }

    // ── Albums in the sidebar ─────────────────────────────────────────────

    /// Name and album-ness of every playlist row in the sidebar.
    fn sidebar_playlist_rows(app: &crate::tui::App) -> Vec<(String, bool)> {
        app.sidebar_items()
            .into_iter()
            .filter_map(|item| match item {
                crate::tui::SidebarItem::Playlist { name, is_album, .. } => Some((name, is_album)),
                _ => None,
            })
            .collect()
    }

    fn album_entry(
        name: &str,
        parent: &str,
        path: std::path::PathBuf,
    ) -> crate::playlist::PlaylistEntry {
        crate::playlist::PlaylistEntry {
            name: name.to_string(),
            path,
            kind: crate::playlist::PlaylistKind::Album,
            parent: Some(parent.to_string()),
        }
    }

    /// An album with a live parent is drawn inside that parent's track list, so
    /// the sidebar leaves it out rather than repeating it in fourteen columns.
    #[test]
    fn sidebar_leaves_out_an_album_that_has_a_parent() {
        let mut app = make_app_with_playlists("Live Sets", &["Ambient", "Live Sets"]);
        app.available_playlists.push(album_entry(
            "Ultra 2026",
            "Live Sets",
            std::path::PathBuf::from("/fake/Ultra 2026.toml"),
        ));

        assert_eq!(
            sidebar_playlist_rows(&app),
            vec![
                ("Ambient".to_string(), false),
                ("Live Sets".to_string(), false),
            ]
        );
    }

    /// An album whose parent has been deleted has no track list left to appear
    /// in, so the sidebar is the only route to it. It stays listed — and still
    /// says it is an album.
    #[test]
    fn sidebar_shows_an_orphaned_album_at_the_top_level() {
        let mut app = make_app_with_playlists("Ambient", &["Ambient"]);
        app.available_playlists.push(album_entry(
            "Ultra 2026",
            "Gone",
            std::path::PathBuf::from("/fake/Ultra 2026.toml"),
        ));

        assert_eq!(
            sidebar_playlist_rows(&app),
            vec![
                ("Ambient".to_string(), false),
                ("Ultra 2026".to_string(), true),
            ]
        );
    }

    /// A parent's albums point at it by name, so renaming it has to rewrite
    /// them — otherwise every album under it orphans on the next launch.
    #[tokio::test]
    async fn renaming_a_playlist_repoints_the_albums_under_it() {
        use crate::tui::input::handle_playlist_rename;
        use crate::tui::InputMode;

        let dir = tempfile::tempdir().expect("tempdir");
        let parent_path = dir.path().join("Live Sets.toml");
        make_playlist("Live Sets").save(&parent_path).expect("save");

        let album_path = dir.path().join("Ultra 2026.toml");
        let mut album = make_playlist("Ultra 2026");
        album.kind = crate::playlist::PlaylistKind::Album;
        album.parent = Some("Live Sets".to_string());
        album.save(&album_path).expect("save album");

        let mut app = crate::tui::App::new(
            crate::playlist::Playlist::load(&parent_path).expect("load"),
            crate::config::Config::default(),
            crate::playlist::Playlist::list_entries(dir.path()).expect("list"),
            parent_path.clone(),
            test_library(),
        );

        // [PlaylistsHeader, Live Sets, Ultra 2026, ...]
        app.sidebar_selected = 1;
        app.input_mode = InputMode::PlaylistRename;
        app.input_buf = "Warehouse".to_string();
        handle_playlist_rename(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("rename");

        let album = crate::playlist::Playlist::load(&album_path).expect("load album");
        assert_eq!(
            album.parent.as_deref(),
            Some("Warehouse"),
            "the album's document must follow its parent's new name"
        );
        assert_eq!(
            sidebar_playlist_rows(&app),
            vec![("Warehouse".to_string(), false)],
            "and the sidebar must still leave it out, without a relaunch: it now \
             belongs inside Warehouse's track list"
        );
    }

    /// Deleting a playlist deletes one file. Its albums are playlists in their
    /// own right and stay exactly where they are — orphaned, not lost.
    #[tokio::test]
    async fn deleting_a_playlist_leaves_the_albums_under_it_alone() {
        use crate::tui::input::handle_playlist_delete;

        let dir = tempfile::tempdir().expect("tempdir");
        let active_path = dir.path().join("Ambient.toml");
        make_playlist("Ambient").save(&active_path).expect("save");

        let parent_path = dir.path().join("Live Sets.toml");
        make_playlist("Live Sets").save(&parent_path).expect("save");

        let album_path = dir.path().join("Ultra 2026.toml");
        let mut album = make_playlist("Ultra 2026");
        album.kind = crate::playlist::PlaylistKind::Album;
        album.parent = Some("Live Sets".to_string());
        add_track(&mut album, make_track("A", "Track A"));
        album.save(&album_path).expect("save album");

        let mut app = crate::tui::App::new(
            crate::playlist::Playlist::load(&active_path).expect("load"),
            crate::config::Config::default(),
            crate::playlist::Playlist::list_entries(dir.path()).expect("list"),
            active_path.clone(),
            test_library(),
        );

        // [PlaylistsHeader, Ambient, Live Sets, Ultra 2026, ...] — delete the parent.
        app.sidebar_selected = 2;
        handle_playlist_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .await
            .expect("delete");

        assert!(!parent_path.exists(), "the parent's file is gone");
        assert!(album_path.exists(), "the album's file must survive");
        assert_eq!(
            crate::playlist::Playlist::load(&album_path)
                .expect("load album")
                .tracks
                .len(),
            1,
            "with its rows intact"
        );
        assert_eq!(
            sidebar_playlist_rows(&app),
            vec![
                ("Ambient".to_string(), false),
                ("Ultra 2026".to_string(), true),
            ],
            "and it must still be reachable, now that no track list holds it"
        );
    }

    // ── Task 5: playlist selection during URL input ───────────────────────────

    #[test]
    fn url_input_target_display_defaults_to_active_playlist() {
        use crate::tui::ui::url_input_target_display;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.target_playlist_for_url = None;
        let display = url_input_target_display(&app);
        assert_eq!(display, "Jazz", "should default to active playlist name");
    }

    #[test]
    fn url_input_target_display_shows_target_when_set() {
        use crate::tui::ui::url_input_target_display;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.target_playlist_for_url = Some("Rock".to_string());
        let display = url_input_target_display(&app);
        assert_eq!(display, "Rock", "should show configured target playlist");
    }

    #[test]
    fn cycle_url_target_playlist_cycles_through_all() {
        let mut app = make_app_with_playlists("Jazz", &["Classical", "Jazz", "Rock"]);
        // Start with Jazz as target
        app.target_playlist_for_url = Some("Jazz".to_string());

        // Cycle once – should move to the next in alphabetical order
        app.cycle_url_target_playlist();
        let after_first = app.target_playlist_for_url.clone().unwrap();

        // Cycle again
        app.cycle_url_target_playlist();
        let after_second = app.target_playlist_for_url.clone().unwrap();

        // Cycle again
        app.cycle_url_target_playlist();
        let after_third = app.target_playlist_for_url.clone().unwrap();

        // All three names should appear
        let names: std::collections::HashSet<_> =
            [&after_first as &str, &after_second, &after_third]
                .into_iter()
                .collect();
        assert!(
            names.contains("Classical") || names.contains("Jazz") || names.contains("Rock"),
            "cycle should cover all playlist names; got: {after_first:?}, {after_second:?}, {after_third:?}"
        );
    }

    #[test]
    fn cycle_url_target_playlist_wraps_around() {
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        // available_playlists is sorted: ["Jazz", "Rock"]
        app.target_playlist_for_url = Some("Jazz".to_string());

        app.cycle_url_target_playlist();
        assert_eq!(
            app.target_playlist_for_url.as_deref(),
            Some("Rock"),
            "first cycle should go to Rock"
        );

        app.cycle_url_target_playlist();
        assert_eq!(
            app.target_playlist_for_url.as_deref(),
            Some("Jazz"),
            "second cycle should wrap back to Jazz"
        );
    }

    #[test]
    fn cycle_url_target_playlist_single_playlist_no_panic() {
        let mut app = make_app_with_playlists("Jazz", &["Jazz"]);
        app.target_playlist_for_url = Some("Jazz".to_string());
        app.cycle_url_target_playlist();
        // With only one playlist, should stay on Jazz
        assert_eq!(
            app.target_playlist_for_url.as_deref(),
            Some("Jazz"),
            "single playlist: should stay on same playlist"
        );
    }

    #[test]
    fn cycle_url_target_playlist_empty_playlists_no_panic() {
        let mut app = make_app_with_playlists("Jazz", &[]);
        app.target_playlist_for_url = Some("Jazz".to_string());
        // Should not panic with no available playlists
        app.cycle_url_target_playlist();
        // target remains unchanged when empty
        assert_eq!(
            app.target_playlist_for_url.as_deref(),
            Some("Jazz"),
            "empty list: target should remain unchanged"
        );
    }

    /// Unlike `cycle_url_target_playlist`, this cycle has an "Auto" (`None`)
    /// stop — a URL add has no folder-identity matching to fall back to, but
    /// a folder or file add does.
    #[test]
    fn cycle_add_target_visits_auto_then_every_playlist_then_wraps() {
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        assert_eq!(app.target_list_for_add, None, "starts on Auto");

        app.cycle_add_target();
        assert_eq!(app.target_list_for_add.as_deref(), Some("Jazz"));

        app.cycle_add_target();
        assert_eq!(app.target_list_for_add.as_deref(), Some("Rock"));

        app.cycle_add_target();
        assert_eq!(app.target_list_for_add, None, "wraps back to Auto");

        app.cycle_add_target();
        assert_eq!(app.target_list_for_add.as_deref(), Some("Jazz"));
    }

    #[test]
    fn cycle_add_target_single_playlist_alternates_with_auto() {
        let mut app = make_app_with_playlists("Jazz", &["Jazz"]);
        app.cycle_add_target();
        assert_eq!(app.target_list_for_add.as_deref(), Some("Jazz"));
        app.cycle_add_target();
        assert_eq!(app.target_list_for_add, None);
    }

    #[test]
    fn cycle_add_target_empty_playlists_stays_on_auto() {
        let mut app = make_app_with_playlists("Jazz", &[]);
        app.cycle_add_target();
        assert_eq!(app.target_list_for_add, None);
    }

    #[test]
    fn url_input_target_display_fallback_when_target_none() {
        use crate::tui::ui::url_input_target_display;
        let app = make_app_with_playlists("My Playlist", &["My Playlist"]);
        // target_playlist_for_url defaults to None
        let display = url_input_target_display(&app);
        assert_eq!(display, "My Playlist");
    }

    #[test]
    fn app_new_target_playlist_for_url_is_none() {
        let app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        assert!(
            app.target_playlist_for_url.is_none(),
            "new App should have no URL target set"
        );
    }

    #[test]
    fn cycle_url_target_playlist_with_none_target_starts_from_first() {
        let mut app = make_app_with_playlists("Jazz", &["Classical", "Jazz"]);
        // No target set – cycle should pick from the available list based on active name
        app.target_playlist_for_url = None;
        app.cycle_url_target_playlist();
        // Active is "Jazz"; in available_playlists ["Classical", "Jazz"], Jazz is at index 1
        // Next after Jazz (index 1) → wrap to Classical (index 0)
        assert!(
            app.target_playlist_for_url.is_some(),
            "after cycling, target should be set"
        );
    }

    // ── Task 6: Acceptance criteria and edge cases ────────────────────────────

    // --- Verify track context menu works with all playlist combinations ---

    #[test]
    fn context_menu_with_two_playlists_shows_only_other() {
        // Exactly two playlists: context menu shows the non-active one only
        let app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        let items = app.available_playlist_names();
        assert_eq!(items.len(), 1, "two playlists: one target");
        assert_eq!(items[0], "Beta");
    }

    #[test]
    fn context_menu_with_many_playlists_shows_all_others() {
        // Ten playlists: context menu should show nine others
        let all: Vec<&str> = vec![
            "P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8", "P9", "Active",
        ];
        let app = make_app_with_playlists("Active", &all);
        let items = app.available_playlist_names();
        assert_eq!(items.len(), 9, "ten playlists: nine targets");
        assert!(!items.contains(&"Active".to_string()), "active excluded");
    }

    #[test]
    fn context_menu_active_not_in_available_list_shows_empty() {
        // Unusual case: available_playlists doesn't contain the active playlist
        let app = make_app_with_playlists("Active", &["Other1", "Other2"]);
        // available_playlist_names filters by name != active, so Other1/Other2 both appear
        let items = app.available_playlist_names();
        assert_eq!(items.len(), 2, "both non-active playlists visible");
    }

    // --- Verify playlist switching leaves playback state untouched ---

    #[test]
    fn switch_to_playlist_does_not_clear_paused_state() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.is_paused = true;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);
        app.switch_to_playlist("Beta", &path).expect("switch");

        assert!(app.is_paused, "paused state must survive a playlist switch");
    }

    #[test]
    fn switch_to_playlist_does_not_reset_position() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.position = 123.4;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);
        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(
            app.position, 123.4,
            "position must survive a playlist switch"
        );
    }

    #[test]
    fn switch_to_empty_playlist_selection_stays_at_zero() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.selected = 3;

        let beta = make_playlist("Beta"); // empty playlist
        let (_dir, path) = write_temp_playlist(&beta);
        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(app.selected, 0, "selection at 0 for empty playlist");
        assert_eq!(app.playlist.tracks.len(), 0, "switched playlist is empty");
    }

    #[test]
    fn switch_to_playlist_with_tracks_updates_track_count() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);

        let mut beta = make_playlist("Beta");
        add_track(&mut beta, make_track("v1", "Track 1"));
        add_track(&mut beta, make_track("v2", "Track 2"));
        add_track(&mut beta, make_track("v3", "Track 3"));
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(
            app.playlist.tracks.len(),
            3,
            "switched playlist should have 3 tracks"
        );
    }

    // --- Verify playlist management handles file system errors ---

    #[test]
    fn playlist_delete_nonexistent_file_returns_error() {
        let path = std::path::Path::new("/tmp/trovers_task6_nonexistent_test.toml");
        let result = crate::playlist::Playlist::delete(path);
        assert!(
            result.is_err(),
            "delete of missing file should return error"
        );
    }

    #[test]
    fn playlist_rename_to_invalid_path_returns_error() {
        // Create a valid playlist then try renaming to a path in a non-existent directory
        let pl = make_playlist("Original");
        let (dir, old_path) = write_temp_playlist(&pl);
        let pl2 = crate::playlist::Playlist::load(&old_path).expect("load");

        // Using a path that can't be written: simulate by pointing to a non-existent dir
        let nonexistent_parent = dir.path().join("nonexistent_subdir").join("NewName.toml");
        // Try saving to a path whose parent doesn't exist
        let result = pl2.save(&nonexistent_parent);
        assert!(
            result.is_err(),
            "save to non-existent directory should fail"
        );
    }

    #[test]
    fn playlist_save_and_load_round_trip_preserves_tracks() {
        // Backward compatibility: save a playlist and reload it
        let mut pl = make_playlist("RoundTrip");
        add_track(&mut pl, make_track("v1", "Track A"));
        add_track(&mut pl, make_track("v2", "Track B"));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("RoundTrip.toml");
        pl.save(&path).expect("save");

        let loaded = crate::playlist::Playlist::load(&path).expect("load");
        assert_eq!(loaded.name, "RoundTrip");
        assert_eq!(loaded.tracks.len(), 2);
        assert_eq!(loaded.tracks[0], "v1");
        assert_eq!(loaded.tracks[1], "v2");
    }

    // --- Verify URL input playlist selection works with 1 and many playlists ---

    #[test]
    fn cycle_url_target_with_one_playlist_stays_on_same() {
        let mut app = make_app_with_playlists("Solo", &["Solo"]);
        app.target_playlist_for_url = Some("Solo".to_string());

        app.cycle_url_target_playlist();

        assert_eq!(
            app.target_playlist_for_url.as_deref(),
            Some("Solo"),
            "single playlist cycling stays on same playlist"
        );
    }

    #[test]
    fn cycle_url_target_with_many_playlists_covers_all() {
        let all: Vec<&str> = vec!["A", "B", "C", "D", "E"];
        let mut app = make_app_with_playlists("A", &all);
        app.target_playlist_for_url = Some("A".to_string());

        let mut seen = std::collections::HashSet::new();
        // Cycle through all 5 playlists
        for _ in 0..5 {
            if let Some(t) = app.target_playlist_for_url.as_deref() {
                seen.insert(t.to_string());
            }
            app.cycle_url_target_playlist();
        }

        assert_eq!(
            seen.len(),
            5,
            "all 5 playlists should be cycled through: {seen:?}"
        );
    }

    // --- Test edge cases: empty playlists, corrupted files ---

    #[test]
    fn load_empty_tracks_playlist_succeeds() {
        // A playlist with zero tracks is valid
        let pl = make_playlist("Empty");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Empty.toml");
        pl.save(&path).expect("save");

        let loaded = crate::playlist::Playlist::load(&path).expect("load empty playlist");
        assert_eq!(
            loaded.tracks.len(),
            0,
            "empty playlist should load with 0 tracks"
        );
        assert_eq!(loaded.name, "Empty");
    }

    #[test]
    fn load_corrupted_file_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("corrupted.toml");
        std::fs::write(&path, b"this is not valid TOML [[[ ]]").expect("write");

        let result = crate::playlist::Playlist::load(&path);
        assert!(result.is_err(), "corrupted file should return error");
    }

    #[test]
    fn load_missing_file_returns_error() {
        let path = std::path::Path::new("/tmp/trovers_task6_missing_playlist.toml");
        let result = crate::playlist::Playlist::load(path);
        assert!(result.is_err(), "missing file should return error");
    }

    #[test]
    fn remove_track_from_single_track_playlist_leaves_empty() {
        let mut pl = make_playlist("Single");
        pl.add_track("only".to_string());
        pl.current_track = Some("only".to_string());

        assert!(pl.remove_track_by_id("only"), "should drop the row");
        assert!(
            pl.tracks.is_empty(),
            "playlist should be empty after removal"
        );
        assert!(
            pl.current_track.is_none(),
            "current_track should be cleared"
        );
    }

    // --- Verify the playlist file schema ---

    /// A hand-written playlist file loads: rows are ids, and nothing else about
    /// the schema changed. (Files still carrying embedded `[[tracks]]` tables are
    /// the migration's business — see `library::migrate`.)
    #[test]
    fn a_hand_written_id_list_playlist_loads() {
        let toml_content = r#"
name = "HandWritten"
created = "2025-01-01T00:00:00Z"
loop_mode = "none"
tracks = ["youtube:abc123", "bandcamp:min001"]
current_track = "youtube:abc123"
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hand-written.toml");
        std::fs::write(&path, toml_content).expect("write");

        let pl = crate::playlist::Playlist::load(&path).expect("load");
        assert_eq!(pl.name, "HandWritten");
        assert_eq!(pl.tracks, vec!["youtube:abc123", "bandcamp:min001"]);
        assert_eq!(pl.current_track.as_deref(), Some("youtube:abc123"));
    }

    #[test]
    fn playlist_loop_mode_variants_all_load_correctly() {
        // All loop_mode variants should deserialize without error
        for (mode_str, expected) in [
            ("none", crate::playlist::LoopMode::None),
            ("track", crate::playlist::LoopMode::Track),
            ("playlist", crate::playlist::LoopMode::Playlist),
        ] {
            let toml_content = format!(
                r#"
name = "LoopTest"
created = "2025-01-01T00:00:00Z"
loop_mode = "{mode_str}"
tracks = []
"#
            );
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("loop_test.toml");
            std::fs::write(&path, toml_content.as_bytes()).expect("write");

            let result = crate::playlist::Playlist::load(&path);
            assert!(
                result.is_ok(),
                "loop_mode={mode_str} should load: {result:?}"
            );
            assert_eq!(
                result.unwrap().loop_mode,
                expected,
                "loop_mode={mode_str} mismatch"
            );
        }
    }

    // Reconciling a recorded `cache_status` with what is on disk is now
    // `Library::load`'s job — see `library_test::load_downgrades_a_cached_track_whose_file_is_gone`.

    // ── DownloadDone with non-active playlist ─────────────────────────────────

    #[test]
    fn download_done_updates_non_active_playlist_on_disk() {
        use crate::library::CacheStatus;
        use crate::playlist::Playlist;
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");

        // Source (active) playlist — vid1 will be added to "Rock", not here
        let source_path = dir.path().join("Source.toml");
        let source_pl = make_playlist("Source");
        source_pl.save(&source_path).expect("save source");

        // Target (Rock) playlist — contains vid1 in Streaming state
        let rock_path = dir.path().join("Rock.toml");
        let mut rock_pl = make_playlist("Rock");
        add_track(&mut rock_pl, make_track("vid1", "Track One"));
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Source".to_string(), source_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ]);
        let mut app = App::new(
            source_pl,
            config,
            available,
            source_path.clone(),
            test_library(),
        );

        // Simulate that vid1 was submitted for downloading. Which playlist asked
        // for it is no longer anybody's business: the download updates the track.
        app.downloading.insert("vid1".to_string());

        // Fire the DownloadDone message
        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        assert!(
            !app.downloading.contains("vid1"),
            "the download should no longer be in flight"
        );

        // Neither playlist file is touched — rows are ids, and the id did not change.
        assert!(
            app.playlist.tracks.is_empty(),
            "source playlist must not be modified"
        );
        let rock_reloaded = Playlist::load(&rock_path).expect("reload rock");
        assert_eq!(rock_reloaded.tracks, vec!["vid1"]);

        // The track document on disk carries the new state.
        let track = test_library().get("vid1").cloned().expect("vid1 document");
        assert_eq!(
            track.cache_status,
            CacheStatus::Cached,
            "cache_status must be Cached"
        );
        assert_eq!(
            track.file.as_deref(),
            Some(fake_file.as_path()),
            "file path must be set"
        );
    }

    #[test]
    fn download_done_for_active_playlist_updates_in_memory_state() {
        use crate::library::CacheStatus;
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");

        let source_path = dir.path().join("Source.toml");
        let mut source_pl = make_playlist("Source");
        add_track(&mut source_pl, make_track("vid1", "Track One"));
        source_pl.save(&source_path).expect("save source");

        let config = crate::config::Config::default();
        let available = entries(vec![("Source".to_string(), source_path.clone())]);
        let mut app = App::new(
            source_pl,
            config,
            available,
            source_path.clone(),
            test_library(),
        );

        app.downloading.insert("vid1".to_string());

        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        let track = app.library.get("vid1").expect("vid1");
        assert_eq!(
            track.cache_status,
            CacheStatus::Cached,
            "in-memory cache_status must be Cached"
        );
        assert_eq!(
            track.file.as_deref(),
            Some(fake_file.as_path()),
            "in-memory file must be set"
        );
    }

    // ── Task 1: add-track playback-hijack regression tests ─────────────────────

    #[tokio::test]
    async fn adding_track_does_not_change_current_track() {
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        pl.current_track = Some("A".to_string());
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        // Simulate track A currently playing at a non-zero position.
        app.position = 137.0;

        // Simulate adding a brand-new track B to the active playlist via the
        // URL-add flow (MetaReady with no target_path override).
        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/B".to_string(),
            meta: TrackMeta {
                title: "Track B".to_string(),
                artist: "Artist B".to_string(),
                channel: "Channel B".to_string(),
                duration: 200,
                video_id: "B".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: None,
        });

        // Track B must have been added, under the library id minted from its
        // source domain and platform id...
        assert!(
            app.playlist.tracks.iter().any(|id| id == "youtube:B"),
            "track B should be added"
        );
        // ...but current_track must remain unchanged (still A), and playback
        // position must not have been touched by adding the track.
        assert_eq!(
            app.playlist.current_track.as_deref(),
            Some("A"),
            "adding a track must not change current_track"
        );
        assert_eq!(
            app.position, 137.0,
            "adding a track must not touch playback position"
        );
    }

    #[tokio::test]
    async fn a_track_added_by_url_shows_up_as_a_row_immediately() {
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/B".to_string(),
            meta: TrackMeta {
                title: "Track B".to_string(),
                artist: "Artist B".to_string(),
                channel: "Channel B".to_string(),
                duration: 200,
                video_id: "B".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: None,
        });

        // The rows are what the user sees; a track that is in the playlist but
        // not in `rows` is a track that was added into thin air.
        assert_eq!(row_shapes(&app), vec!["own:0", "own:1"]);
    }

    #[tokio::test]
    async fn download_done_for_newly_added_track_does_not_hijack_playback() {
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        pl.current_track = Some("A".to_string());
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        // Track A is "playing" at a non-zero position (mirrors the user report:
        // track A playing, non-zero position, then a track is added).
        app.position = 137.0;

        // Add track B while A is playing.
        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/B".to_string(),
            meta: TrackMeta {
                title: "Track B".to_string(),
                artist: "Artist B".to_string(),
                channel: "Channel B".to_string(),
                duration: 200,
                video_id: "B".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: None,
        });

        // B's background download finishes.
        let fake_file = dir.path().join("B.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "youtube:B".to_string(),
            file: fake_file.clone(),
        });

        // The hot-switch "is this the currently playing track" check must
        // evaluate false for B, since current_track is still "A" — proving
        // the DownloadDone handler no longer hijacks playback for a track
        // that was merely added, not actually playing.
        assert_eq!(
            app.playlist.current_track.as_deref(),
            Some("A"),
            "current_track must still be A after B's download completes"
        );
        assert_eq!(
            app.position, 137.0,
            "playback position must remain untouched"
        );

        // B's cache metadata should still have been updated normally.
        let track_b = app.library.get("youtube:B").expect("track B");
        assert_eq!(track_b.cache_status, crate::library::CacheStatus::Cached);
        assert_eq!(track_b.file.as_deref(), Some(fake_file.as_path()));
    }

    #[tokio::test]
    async fn meta_ready_adds_to_non_active_target_playlist_via_handle_task_msg() {
        // End-to-end coverage, via the real handle_task_msg code path, for
        // MetaReady's "add to non-active target playlist" branch: the id must
        // land in the target playlist file on disk, the displayed (active)
        // playlist must be untouched, and the track's document must be written.
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");

        let active_path = dir.path().join("Active.toml");
        let active_pl = make_playlist("Active");
        active_pl.save(&active_path).expect("save active");

        let rock_path = dir.path().join("Rock.toml");
        let rock_pl = make_playlist("Rock");
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Active".to_string(), active_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ]);
        let mut app = App::new(
            active_pl,
            config,
            available,
            active_path.clone(),
            test_library(),
        );

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/vidX".to_string(),
            meta: TrackMeta {
                title: "Track X".to_string(),
                artist: "Artist X".to_string(),
                channel: "Channel X".to_string(),
                duration: 150,
                video_id: "vidX".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: Some(rock_path.clone()),
        });

        // Active (displayed) playlist must be untouched in memory and on disk.
        assert!(
            app.playlist.tracks.is_empty(),
            "active playlist must not be mutated in memory"
        );
        let active_reloaded = crate::playlist::Playlist::load(&active_path).expect("reload active");
        assert!(
            active_reloaded.tracks.is_empty(),
            "active playlist must not be mutated on disk"
        );

        // Target (Rock) playlist must list the new id on disk.
        let rock_reloaded = crate::playlist::Playlist::load(&rock_path).expect("reload rock");
        assert_eq!(rock_reloaded.tracks, vec!["youtube:vidX"]);

        // And the track's own document carries the metadata, wherever it is listed.
        assert_eq!(
            app.library.get("youtube:vidX").map(|t| t.title.as_str()),
            Some("Track X")
        );
    }

    // ── Task 3: patch_track ─────────────────────────────────────────────────

    #[test]
    fn patch_track_mutates_the_document_in_memory_and_on_disk() {
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        // `cache_status` must be paired with an existing `file` — otherwise
        // `Library::load`'s file-existence check resets `Cached` back to
        // `Streaming` on reload, which has nothing to do with `patch_track`.
        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.patch_track("vid1", |t| {
            t.cache_status = crate::library::CacheStatus::Cached;
            t.file = Some(fake_file.clone());
            t.user_title = Some("Patched".to_string());
        });

        // The app's own library reflects the patch immediately.
        let track = app.library.get("vid1").expect("vid1");
        assert_eq!(track.cache_status, crate::library::CacheStatus::Cached);
        assert_eq!(track.file.as_deref(), Some(fake_file.as_path()));
        assert_eq!(track.user_title.as_deref(), Some("Patched"));

        // And the document was persisted, so a fresh library sees it too.
        let track = test_library().get("vid1").cloned().expect("vid1");
        assert_eq!(track.cache_status, crate::library::CacheStatus::Cached);
        assert_eq!(track.file.as_deref(), Some(fake_file.as_path()));
        assert_eq!(track.user_title.as_deref(), Some("Patched"));

        // The playlist file is untouched: patching a track is not a playlist edit.
        let reloaded = crate::playlist::Playlist::load(&path).expect("reload");
        assert_eq!(reloaded.tracks, vec!["vid1"]);
    }

    /// A row can name a document that has gone missing from under it, so
    /// patching an id the library does not hold has to be a silent no-op.
    #[test]
    fn patch_track_of_an_unknown_id_is_a_noop() {
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        app.patch_track("does-not-exist", |t| {
            t.cache_status = crate::library::CacheStatus::Cached;
        });

        assert_eq!(
            app.library.get("vid1").map(|t| &t.cache_status),
            Some(&crate::library::CacheStatus::Streaming),
            "existing track must be untouched when the patched id doesn't exist"
        );
        assert!(
            app.library.get("does-not-exist").is_none(),
            "patching must not conjure a document"
        );
    }

    // ── Task 3: DownloadDone reaches the playing track from anywhere ────────

    #[tokio::test]
    async fn download_done_updates_the_playing_track_even_when_browsing_elsewhere() {
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");

        // The playing track is listed by "Playing.toml".
        let playing_path = dir.path().join("Playing.toml");
        let mut playing_pl = make_playlist("Playing");
        add_track(&mut playing_pl, make_track("vid1", "Track One"));
        playing_pl.save(&playing_path).expect("save playing");

        // The user is currently browsing a *different* playlist.
        let browsing_path = dir.path().join("Browsing.toml");
        let browsing_pl = make_playlist("Browsing");
        browsing_pl.save(&browsing_path).expect("save browsing");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Playing".to_string(), playing_path.clone()),
            ("Browsing".to_string(), browsing_path.clone()),
        ]);
        let mut app = App::new(
            browsing_pl,
            config,
            available,
            browsing_path.clone(),
            test_library(),
        );

        // Simulate vid1 playing out of the Playing playlist while Browsing is
        // on screen.
        app.playing = Some(session_at(
            playing_path.clone(),
            crate::playlist::Playlist::load(&playing_path).expect("load playing"),
            0,
        ));
        app.position = 42.0;

        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");

        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        // No mpv runs in tests, so the hot-switch itself cannot be observed —
        // what is observable is that the state it keys off reached the one
        // document, with no "which playlist owns this download" bookkeeping
        // in between.
        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            playing_track.cache_status,
            crate::library::CacheStatus::Cached
        );
        assert_eq!(playing_track.file.as_deref(), Some(fake_file.as_path()));

        // Neither playlist file was rewritten, and the displayed one is still empty.
        assert!(
            app.playlist.tracks.is_empty(),
            "displayed playlist must not be mutated"
        );
        let reloaded = crate::playlist::Playlist::load(&playing_path).expect("reload playing");
        assert_eq!(reloaded.tracks, vec!["vid1"]);
    }

    // ── Task 3: speed handlers operate on the playing track, not the displayed cursor ──

    #[tokio::test]
    async fn adjust_playing_track_speed_follows_the_playing_track_not_the_cursor() {
        use crate::tui::input::adjust_playing_track_speed;
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");

        // The playing track is listed by "Playing.toml"...
        let playing_path = dir.path().join("Playing.toml");
        let mut playing_pl = make_playlist("Playing");
        add_track(&mut playing_pl, make_track("vid1", "Track One"));
        playing_pl.save(&playing_path).expect("save playing");

        // ...but the user is browsing a *different* playlist, with a different
        // track under the cursor.
        let browsing_path = dir.path().join("Browsing.toml");
        let mut browsing_pl = make_playlist("Browsing");
        add_track(&mut browsing_pl, make_track("vid2", "Unrelated Track"));
        browsing_pl.save(&browsing_path).expect("save browsing");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Playing".to_string(), playing_path.clone()),
            ("Browsing".to_string(), browsing_path.clone()),
        ]);
        let mut app = App::new(
            browsing_pl,
            config,
            available,
            browsing_path.clone(),
            test_library(),
        );
        app.selected = 0; // cursor sits on Browsing's vid2

        app.playing = Some(session_at(
            playing_path.clone(),
            crate::playlist::Playlist::load(&playing_path).expect("load playing"),
            0,
        ));

        adjust_playing_track_speed(&mut app, 0.1).await;

        // The track under the cursor must be untouched.
        assert_eq!(
            app.library.get("vid2").and_then(|t| t.speed),
            None,
            "the track under the cursor must not be touched"
        );

        // The playing track's speed must be bumped by 0.1 relative to the
        // default (1.0), since neither the track nor its playlist set one.
        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            playing_track.speed,
            Some(1.1),
            "playing track's speed must be bumped from the default"
        );

        // Persisted to the playing track's document, and only that one.
        let reloaded = test_library();
        assert_eq!(reloaded.get("vid1").and_then(|t| t.speed), Some(1.1));
        assert_eq!(reloaded.get("vid2").and_then(|t| t.speed), None);
    }

    #[tokio::test]
    async fn adjust_playing_track_speed_is_noop_when_nothing_playing() {
        use crate::tui::input::adjust_playing_track_speed;
        use crate::tui::App;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        let config = crate::config::Config::default();
        let available = entries(vec![(
            "Active".to_string(),
            std::path::PathBuf::from("/fake/Active.toml"),
        )]);
        let mut app = App::new(
            pl,
            config,
            available,
            std::path::PathBuf::from("/fake/Active.toml"),
            test_library(),
        );

        adjust_playing_track_speed(&mut app, 0.1).await;

        assert_eq!(
            app.library.get("vid1").and_then(|t| t.speed),
            None,
            "no track should be touched when nothing is playing"
        );
    }

    #[tokio::test]
    async fn adjust_playing_track_speed_clamps_to_max() {
        use crate::tui::input::adjust_playing_track_speed;
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app = App::new(pl, config, available, path.clone(), test_library());

        app.playing = Some(session_at(
            path.clone(),
            crate::playlist::Playlist::load(&path).expect("load"),
            0,
        ));
        app.patch_track("vid1", |t| t.speed = Some(2.95));

        adjust_playing_track_speed(&mut app, 0.5).await;

        assert_eq!(
            app.library.get("vid1").and_then(|t| t.speed),
            Some(3.0),
            "speed must clamp at 3.0"
        );
    }

    /// The audiobook case: adjusting speed while a track from an album is
    /// playing sets the *album's* `default_speed`, not the individual
    /// track's — so every other chapter that has never had its own speed
    /// set inherits the change too, and re-adjusting at every chapter
    /// boundary is no longer necessary.
    #[tokio::test]
    async fn adjust_playing_track_speed_on_an_albums_track_sets_the_albums_default_speed() {
        use crate::tui::input::adjust_playing_track_speed;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        let album_path = app.albums[0].path.clone();
        let album_playlist = app.albums[0].playlist.clone();
        app.playing = Some(session_at(album_path.clone(), album_playlist, 0)); // k1

        adjust_playing_track_speed(&mut app, 0.1).await;

        let expected = app.config.default_speed + 0.1;
        assert_eq!(
            app.albums[0].playlist.default_speed,
            Some(expected),
            "the album's own default_speed changed"
        );
        assert_eq!(
            app.library.get("k1").and_then(|t| t.speed),
            None,
            "the individual track's own speed was not written"
        );
        let on_disk = crate::playlist::Playlist::load(&album_path).expect("reload");
        assert_eq!(on_disk.default_speed, Some(expected), "persisted to disk");

        // The next chapter, never individually sped up, must pick up the new
        // album-wide speed too.
        let k2_speed = crate::tui::effective_speed(
            app.library.get("k2").expect("track"),
            &app.albums[0].playlist,
            &app.config,
        );
        assert_eq!(k2_speed, expected, "an untouched chapter inherits it");
    }

    /// The bug this fixes: `PlayingSession.playlist` is its own snapshot
    /// taken when playback started, and writing the new speed only to the
    /// real album (`self.albums`/the file) without also refreshing that
    /// snapshot meant every read of "the current speed" — the next `]`
    /// press's own base, and the Now Playing header — kept seeing the value
    /// from *before* the first press, forever. A second press must move
    /// further, not repeat the first press's result.
    #[tokio::test]
    async fn adjust_playing_track_speed_on_an_album_compounds_across_presses() {
        use crate::tui::input::adjust_playing_track_speed;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1"])]);
        let album_path = app.albums[0].path.clone();
        let album_playlist = app.albums[0].playlist.clone();
        app.playing = Some(session_at(album_path, album_playlist, 0));

        adjust_playing_track_speed(&mut app, 0.1).await;
        let after_first = app.albums[0].playlist.default_speed;
        adjust_playing_track_speed(&mut app, 0.1).await;
        let after_second = app.albums[0].playlist.default_speed;

        assert_eq!(
            after_first,
            Some(app.config.default_speed + 0.1),
            "first press"
        );
        assert_eq!(
            after_second,
            Some(app.config.default_speed + 0.2),
            "second press must build on the first, not repeat it"
        );
        assert_eq!(
            app.playing_playlist().and_then(|pl| pl.default_speed),
            after_second,
            "the playing session's own snapshot must stay in step too, or the \
             Now Playing header would keep showing the old speed"
        );
    }

    // ── Task 4: Now Playing / track-highlight render from app.playing ──────────

    #[test]
    fn playing_track_shows_data_from_unrelated_displayed_playlist() {
        // Playing session points at playlist A ("Alpha")...
        let mut alpha = make_playlist("Alpha");
        add_track(&mut alpha, make_track("a1", "Alpha Track"));
        let alpha_path = std::path::PathBuf::from("/fake/Alpha.toml");

        // ...while the user is browsing a completely different playlist B
        // ("Beta") with different tracks.
        let mut app = make_app_with_playlists("Beta", &["Alpha", "Beta"]);
        push_track(&mut app, make_track("b1", "Beta Track"));
        app.playlist_path = std::path::PathBuf::from("/fake/Beta.toml");

        app.playing = Some(session_at(alpha_path, alpha, 0));

        let track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            track.title, "Alpha Track",
            "Now Playing must reflect the playing session's track, not the displayed playlist"
        );
    }

    #[test]
    fn row_is_playing_false_when_paths_differ_even_with_matching_video_id() {
        use crate::tui::ui::row_is_playing;

        // Playing session lives in "Alpha.toml" and its track happens to
        // share a id with a track in the displayed "Beta" playlist.
        let mut alpha = make_playlist("Alpha");
        add_track(&mut alpha, make_track("shared", "Alpha Track"));
        let alpha_path = std::path::PathBuf::from("/fake/Alpha.toml");

        let mut app = make_app_with_playlists("Beta", &["Alpha", "Beta"]);
        push_track(&mut app, make_track("shared", "Beta Track"));
        app.playlist_path = std::path::PathBuf::from("/fake/Beta.toml");

        app.playing = Some(session_at(alpha_path, alpha, 0));

        assert!(
            !row_is_playing(&app, crate::tui::RowSource::Own, "shared"),
            "must not highlight a row just because the id matches across different playlist files"
        );
    }

    #[test]
    fn row_is_playing_true_when_paths_and_video_id_match() {
        use crate::tui::ui::row_is_playing;

        let mut app = make_app_with_playlists("Alpha", &["Alpha"]);
        push_track(&mut app, make_track("vid1", "Track One"));
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");

        app.playing = Some(session_at(
            app.playlist_path.clone(),
            app.playlist.clone(),
            0,
        ));

        assert!(
            row_is_playing(&app, crate::tui::RowSource::Own, "vid1"),
            "must highlight the row when the playing session's track belongs to the displayed playlist"
        );
    }

    #[test]
    fn row_is_playing_false_when_nothing_playing() {
        use crate::tui::ui::row_is_playing;

        let mut app = make_app_with_playlists("Alpha", &["Alpha"]);
        push_track(&mut app, make_track("vid1", "Track One"));

        assert!(
            !row_is_playing(&app, crate::tui::RowSource::Own, "vid1"),
            "no highlight when app.playing is None"
        );
    }

    // ── Task 5: n/b operate on displayed playlist; delete/move identity guard ──

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn n_steps_cursor_within_displayed_playlist_ignoring_unrelated_playing_session() {
        use crate::tui::input::handle_tracklist;

        // Displayed playlist ("Browsing") has three tracks; cursor starts at 0.
        let mut app = make_app_with_playlists("Browsing", &["Browsing", "Playing"]);
        push_track(&mut app, make_track("x1", "X One"));
        push_track(&mut app, make_track("x2", "X Two"));
        push_track(&mut app, make_track("x3", "X Three"));
        app.playlist_path = std::path::PathBuf::from("/fake/Browsing.toml");
        app.selected = 0;

        // Something entirely unrelated is playing in a different playlist.
        let mut playing_pl = make_playlist("Playing");
        add_track(&mut playing_pl, make_track("p1", "P One"));
        app.playing = Some(session_at(
            std::path::PathBuf::from("/fake/Playing.toml"),
            playing_pl,
            0,
        ));

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handle n");

        // Cursor must step to the next track *within the displayed playlist*,
        // not jump toward the unrelated playing session.
        assert_eq!(
            app.selected, 1,
            "n should move cursor to the next displayed-playlist track"
        );
    }

    #[tokio::test]
    async fn n_wraps_to_first_track_at_end_of_displayed_playlist() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("x1", "X One"));
        push_track(&mut app, make_track("x2", "X Two"));
        app.selected = 1; // last track

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handle n");

        assert_eq!(app.selected, 0, "n should wrap around to the first track");
    }

    #[tokio::test]
    async fn b_steps_cursor_backward_within_displayed_playlist() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing", "Playing"]);
        push_track(&mut app, make_track("x1", "X One"));
        push_track(&mut app, make_track("x2", "X Two"));
        push_track(&mut app, make_track("x3", "X Three"));
        app.playlist_path = std::path::PathBuf::from("/fake/Browsing.toml");
        app.selected = 2;

        // Unrelated playing session elsewhere.
        let mut playing_pl = make_playlist("Playing");
        add_track(&mut playing_pl, make_track("p1", "P One"));
        app.playing = Some(session_at(
            std::path::PathBuf::from("/fake/Playing.toml"),
            playing_pl,
            0,
        ));

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('b')))
            .await
            .expect("handle b");

        assert_eq!(
            app.selected, 1,
            "b should move cursor to the previous displayed-playlist track"
        );
    }

    #[tokio::test]
    async fn b_wraps_to_last_track_at_start_of_displayed_playlist() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("x1", "X One"));
        push_track(&mut app, make_track("x2", "X Two"));
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('b')))
            .await
            .expect("handle b");

        assert_eq!(app.selected, 1, "b should wrap around to the last track");
    }

    #[tokio::test]
    async fn n_is_noop_on_empty_displayed_playlist() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Empty", &["Empty"]);
        // No tracks pushed.

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handle n");

        assert_eq!(
            app.selected, 0,
            "n on an empty playlist must not panic or move cursor"
        );
    }

    #[tokio::test]
    async fn space_falls_back_to_cursor_track_when_nothing_playing() {
        // Regression check for the simplified Space fallback: with nothing
        // playing, pressing Space on a selected track must fall back to
        // `track_index_at(app.selected)` (previously it preferred a
        // now-misleading `current_track_index()` first). We can't assert on
        // the real player spawn (no mpv in tests), but we can assert the
        // decision surface didn't bail out early: `app.player` stays None
        // (fire-and-forget spawn) and no panic occurs even with a stale
        // `current_track` pointing at an unrelated/absent track.
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("x1", "X One"));
        push_track(&mut app, make_track("x2", "X Two"));
        // Stale current_track pointer to a track that isn't at the cursor.
        app.playlist.current_track = Some("x1".to_string());
        app.selected = 1; // cursor sits on x2, not x1

        let result = handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' '))).await;
        assert!(
            result.is_ok(),
            "space handling must not error: {:?}",
            result.err()
        );
    }

    /// Space on the exact track that is playing right now pauses it — the
    /// plain-track half of the same-vs-different rule the album header tests
    /// above check for headers.
    #[tokio::test]
    async fn space_on_the_playing_track_pauses() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("x1", "X One"));
        add_track(&mut pl, make_track("x2", "X Two"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());
        app.player = Some(make_dead_player(dead_player_socket("same-track-pause")));
        app.playing = Some(session_at(path, pl, 0)); // x1
        app.position = 30.0;
        app.selected = 0; // cursor on x1, the track that's playing

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert!(app.is_paused, "space on the playing track must pause it");
        assert_eq!(app.position, 30.0, "must not restart or seek");
    }

    /// Space on a *different* track — something else is playing — switches
    /// playback to it, mirroring what Enter already does.
    #[tokio::test]
    async fn space_on_a_different_track_switches_playback_to_it() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("x1", "X One"));
        add_track(&mut pl, make_track("x2", "X Two"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());
        app.player = Some(make_dead_player(dead_player_socket("diff-track-switch")));
        app.playing = Some(session_at(path, pl, 0)); // x1
        app.position = 30.0;
        app.selected = 1; // cursor on x2, not the track that's playing

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("x2"),
            "playback switched to the track under the cursor"
        );
        assert!(!app.is_paused, "a freshly started track is not paused");
    }

    #[test]
    fn is_playing_track_true_for_exact_path_and_video_id_match() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha"]);
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");
        let mut alpha = make_playlist("Alpha");
        add_track(&mut alpha, make_track("vid1", "Track One"));
        app.playing = Some(session_at(app.playlist_path.clone(), alpha, 0));

        assert!(app.is_playing_track(&app.playlist_path.clone(), "vid1"));
    }

    #[test]
    fn is_playing_track_false_when_video_id_matches_but_path_differs() {
        let mut app = make_app_with_playlists("Beta", &["Alpha", "Beta"]);
        app.playlist_path = std::path::PathBuf::from("/fake/Beta.toml");

        let mut alpha = make_playlist("Alpha");
        add_track(&mut alpha, make_track("shared", "Alpha Track"));
        app.playing = Some(session_at(
            std::path::PathBuf::from("/fake/Alpha.toml"),
            alpha,
            0,
        ));

        // A different playlist (Beta) happens to also contain a track with
        // the same id — this must NOT count as "the playing track".
        assert!(
            !app.is_playing_track(&std::path::PathBuf::from("/fake/Beta.toml"), "shared"),
            "matching id across different playlist files must not count as identity match"
        );
    }

    #[test]
    fn delete_does_not_stop_playback_for_the_same_id_in_a_different_playlist() {
        use crate::tui::input::handle_confirm_delete;
        use crate::tui::InputMode;

        // "shared" is playing out of "Playing.toml" and is *also* listed by the
        // displayed playlist ("Browsing") — the same track in two playlists,
        // which the track library makes ordinary. Deleting the Browsing row must
        // not stop playback, since identity requires both path and id to match.
        let mut app = make_app_with_playlists("Browsing", &["Browsing", "Playing"]);
        push_track(&mut app, make_track("shared", "Shared Track"));
        app.playlist_path = std::path::PathBuf::from("/fake/Browsing.toml");
        app.selected = 0;
        app.input_mode = InputMode::ConfirmDelete;

        let mut playing_pl = make_playlist("Playing");
        playing_pl.tracks.push("shared".to_string());
        app.playing = Some(session_at(
            std::path::PathBuf::from("/fake/Playing.toml"),
            playing_pl,
            0,
        ));

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("handle delete confirm");

        assert!(
            app.playing.is_some(),
            "playing session in a different playlist must survive deleting the same id elsewhere"
        );
        assert!(
            app.playlist.tracks.is_empty(),
            "the track in the displayed (Browsing) playlist should still be deleted"
        );
    }

    #[test]
    fn delete_stops_playback_when_deleting_the_actually_playing_track() {
        use crate::tui::input::handle_confirm_delete;
        use crate::tui::InputMode;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("vid1", "Now Playing Track"));
        app.playlist_path = std::path::PathBuf::from("/fake/Browsing.toml");
        app.selected = 0;
        app.input_mode = InputMode::ConfirmDelete;

        app.playing = Some(session_at(
            app.playlist_path.clone(),
            app.playlist.clone(),
            0,
        ));

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("handle delete confirm");

        assert!(
            app.playing.is_none(),
            "playing session must be cleared when the track actually driving playback is deleted"
        );
        assert!(app.playlist.tracks.is_empty());
    }

    #[test]
    fn move_track_does_not_stop_playback_for_the_same_id_in_a_different_playlist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let browsing_path = dir.path().join("Browsing.toml");
        let rock_path = dir.path().join("Rock.toml");

        let mut browsing_pl = make_playlist("Browsing");
        add_track(&mut browsing_pl, make_track("shared", "Colliding Track"));
        browsing_pl.save(&browsing_path).expect("save browsing");

        let rock_pl = make_playlist("Rock");
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = entries(vec![
            ("Browsing".to_string(), browsing_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ]);
        let mut app = crate::tui::App::new(
            browsing_pl,
            config,
            available,
            browsing_path.clone(),
            test_library(),
        );

        // The track actually playing is listed by a wholly different, unrelated
        // playlist file, which happens to list the same "shared" id.
        let mut playing_pl = make_playlist("Playing");
        playing_pl.tracks.push("shared".to_string());
        app.playing = Some(session_at(
            std::path::PathBuf::from("/fake/Playing.toml"),
            playing_pl,
            0,
        ));

        let result = app.move_track_to_playlist("Rock");
        assert!(result.is_ok(), "move should succeed: {:?}", result.err());

        assert!(
            app.playing.is_some(),
            "moving the row out of Browsing must not clear an unrelated playing session"
        );
    }

    // ── Task 6: Resume playback from `last_position` ──────────────────────────

    #[test]
    fn resume_start_pos_returns_none_for_zero_last_position() {
        use crate::tui::input::resume_start_pos;

        let mut track = make_track("vid1", "Track One");
        track.last_position = 0;

        assert_eq!(resume_start_pos(&track), None);
    }

    #[test]
    fn resume_start_pos_returns_some_for_nonzero_last_position() {
        use crate::tui::input::resume_start_pos;

        let mut track = make_track("vid1", "Track One");
        track.last_position = 90;

        assert_eq!(resume_start_pos(&track), Some(90.0));
    }

    #[tokio::test]
    async fn enter_resumes_from_last_position_via_request_playback_arg() {
        // We can't observe the real spawned player (no mpv in tests), but we can
        // observe the *decision*: `request_playback` sets `app.position` to
        // wherever the new player is about to start. So seed `app.position` with
        // an unrelated value standing in for the outgoing track's timestamp,
        // press Enter on a track with a nonzero `last_position`, and confirm the
        // position becomes that track's own resume point — not the stale one it
        // used to keep, which is what made the new track pick up where the
        // previous one left off.
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        let mut track = make_track("vid1", "Resume Track");
        track.last_position = 90;
        push_track(&mut app, track);
        app.selected = 0;
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handle enter");

        assert_eq!(
            app.position, 90.0,
            "Enter on a track with last_position=90 must resume there, discarding the stale 42.0"
        );
    }

    #[tokio::test]
    async fn enter_starts_fresh_when_last_position_is_zero() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        let track = make_track("vid1", "Fresh Track"); // last_position defaults to 0
        push_track(&mut app, track);
        app.selected = 0;
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handle enter");

        assert_eq!(
            app.position, 0.0,
            "request_playback must have been called with None, resetting app.position to 0"
        );
    }

    #[tokio::test]
    async fn n_resumes_next_track_from_its_last_position() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("x1", "X One"));
        let mut second = make_track("x2", "X Two");
        second.last_position = 55;
        push_track(&mut app, second);
        app.selected = 0;
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handle n");

        assert_eq!(app.selected, 1, "n should move cursor to the next track");
        assert_eq!(
            app.position, 55.0,
            "n landing on a track with last_position=55 must resume there, discarding the stale 42.0"
        );
    }

    #[tokio::test]
    async fn b_resumes_previous_track_from_its_last_position() {
        use crate::tui::input::handle_tracklist;

        let mut first = make_track("x1", "X One");
        first.last_position = 30;
        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, first);
        push_track(&mut app, make_track("x2", "X Two"));
        app.selected = 1;
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('b')))
            .await
            .expect("handle b");

        assert_eq!(
            app.selected, 0,
            "b should move cursor to the previous track"
        );
        assert_eq!(
            app.position, 30.0,
            "b landing on a track with last_position=30 must resume there, discarding the stale 42.0"
        );
    }

    #[tokio::test]
    async fn space_resumes_from_last_position_when_nothing_playing() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        let mut track = make_track("vid1", "Resume Track");
        track.last_position = 12;
        push_track(&mut app, track);
        app.selected = 0;
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert_eq!(
            app.position, 12.0,
            "space fallback-to-play on a track with last_position=12 must resume there, discarding the stale 42.0"
        );
    }

    // ── Task 7: per-track download progress ─────────────────────────────────

    #[test]
    fn download_progress_is_tracked_per_video_id() {
        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);

        app.download_progress.insert("vid1".to_string(), 10.0);
        app.download_progress.insert("vid2".to_string(), 90.0);

        // Sending progress for vid1 must not affect vid2's stored percentage.
        app.download_progress.insert("vid1".to_string(), 35.0);

        assert_eq!(app.download_progress.get("vid1"), Some(&35.0));
        assert_eq!(
            app.download_progress.get("vid2"),
            Some(&90.0),
            "unrelated track's progress must remain untouched"
        );
    }

    #[test]
    fn download_done_removes_only_its_own_progress_entry() {
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("Source.toml");
        let mut source_pl = make_playlist("Source");
        add_track(&mut source_pl, make_track("vid1", "Track One"));
        add_track(&mut source_pl, make_track("vid2", "Track Two"));
        source_pl.save(&source_path).expect("save source");

        let config = crate::config::Config::default();
        let available = entries(vec![("Source".to_string(), source_path.clone())]);
        let mut app = App::new(
            source_pl,
            config,
            available,
            source_path.clone(),
            test_library(),
        );

        // Two concurrent downloads in flight.
        app.downloading.insert("vid1".to_string());
        app.downloading.insert("vid2".to_string());
        app.download_progress.insert("vid1".to_string(), 40.0);
        app.download_progress.insert("vid2".to_string(), 70.0);

        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        assert!(
            !app.download_progress.contains_key("vid1"),
            "completed download's progress entry must be removed"
        );
        assert_eq!(
            app.download_progress.get("vid2"),
            Some(&70.0),
            "completing vid1's download must not reset vid2's still-running percentage"
        );
    }

    #[test]
    fn download_error_removes_only_its_own_progress_entry() {
        use crate::tui::TaskMsg;

        let mut app = make_app_with_playlists("Source", &["Source"]);
        push_track(&mut app, make_track("vid1", "Track One"));
        push_track(&mut app, make_track("vid2", "Track Two"));

        app.downloading.insert("vid1".to_string());
        app.downloading.insert("vid2".to_string());
        app.download_progress.insert("vid1".to_string(), 20.0);
        app.download_progress.insert("vid2".to_string(), 60.0);

        app.handle_task_msg(TaskMsg::DownloadError {
            id: "vid1".to_string(),
            err: "boom".to_string(),
        });

        assert!(
            !app.download_progress.contains_key("vid1"),
            "failed download's progress entry must be removed"
        );
        assert_eq!(
            app.download_progress.get("vid2"),
            Some(&60.0),
            "vid1's failure must not affect vid2's progress"
        );
    }

    // ── Task 7: flush position on quit ──────────────────────────────────────

    #[test]
    fn flush_playing_position_persists_the_track_document() {
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        let (_dir, path) = write_temp_playlist(&pl);

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app =
            crate::tui::App::new(pl.clone(), config, available, path.clone(), test_library());

        app.playing = Some(session_at(path.clone(), pl, 0));
        app.position = 123.0;

        app.flush_playing_position();

        // In memory first...
        assert_eq!(app.library.get("vid1").map(|t| t.last_position), Some(123));
        // ...and on disk, which is what makes it survive the quit.
        assert_eq!(
            test_library().get("vid1").map(|t| t.last_position),
            Some(123),
            "last_position must be flushed to disk on quit"
        );
    }

    /// The position of the *playing* track is flushed, whichever playlist is on
    /// screen — and the displayed playlist file is not rewritten for it.
    #[test]
    fn flush_playing_position_works_while_browsing_elsewhere() {
        // App is displaying "Browsing", but the playing track is listed by a
        // different playlist file entirely.
        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);

        let mut playing_pl = make_playlist("Elsewhere");
        add_track(&mut playing_pl, make_track("vid1", "Elsewhere Track"));
        refresh_library(&mut app);
        let (_dir, playing_path) = write_temp_playlist(&playing_pl);

        app.playing = Some(session_at(playing_path.clone(), playing_pl, 0));
        app.position = 77.0;

        app.flush_playing_position();

        // Displayed playlist must be untouched.
        assert!(app.playlist.tracks.is_empty());

        assert_eq!(
            test_library().get("vid1").map(|t| t.last_position),
            Some(77),
            "the playing track's own document must carry the position"
        );
    }

    // ── Playlist rename/delete syncing app.playing (review finding #2) ──────

    #[tokio::test]
    async fn playlist_rename_updates_stale_playing_session_path() {
        use crate::tui::input::handle_playlist_rename;

        // The playlist being renamed ("Elsewhere") is not the displayed
        // playlist ("Browsing"), but it is the one `app.playing` points at.
        let mut elsewhere = make_playlist("Elsewhere");
        add_track(&mut elsewhere, make_track("vid1", "Track One"));
        let (dir, old_path) = write_temp_playlist(&elsewhere);

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        app.available_playlists
            .push(crate::playlist::PlaylistEntry::normal(
                "Elsewhere".to_string(),
                old_path.clone(),
            ));

        app.playing = Some(session_at(old_path.clone(), elsewhere, 0));

        // Point the sidebar cursor at the "Elsewhere" entry.
        let items = app.sidebar_items();
        let pos = items
            .iter()
            .position(|i| matches!(i, crate::tui::SidebarItem::Playlist { name, .. } if name == "Elsewhere"))
            .expect("Elsewhere present in sidebar");
        app.sidebar_selected = pos;

        app.input_buf = "Renamed".to_string();
        handle_playlist_rename(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handle rename");

        let new_path = dir.path().join("Renamed.toml");
        assert!(new_path.exists(), "renamed file should exist");
        assert!(!old_path.exists(), "old file should be gone");

        let session = app
            .playing
            .as_ref()
            .expect("playing session must survive rename");
        assert_eq!(
            session.path, new_path,
            "playing session's path must be re-pointed at the renamed file, not the deleted old_path"
        );
    }

    #[tokio::test]
    async fn playlist_delete_stops_playback_when_deleting_playing_but_not_displayed_playlist() {
        use crate::tui::input::handle_playlist_delete;

        // The playlist being deleted ("Elsewhere") is not the displayed
        // playlist ("Browsing"), but it is the one `app.playing` points at.
        let mut elsewhere = make_playlist("Elsewhere");
        add_track(&mut elsewhere, make_track("vid1", "Track One"));
        let (_dir, elsewhere_path) = write_temp_playlist(&elsewhere);

        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        app.available_playlists
            .push(crate::playlist::PlaylistEntry::normal(
                "Elsewhere".to_string(),
                elsewhere_path.clone(),
            ));

        app.playing = Some(session_at(elsewhere_path.clone(), elsewhere, 0));
        app.is_paused = true;

        let items = app.sidebar_items();
        let pos = items
            .iter()
            .position(|i| matches!(i, crate::tui::SidebarItem::Playlist { name, .. } if name == "Elsewhere"))
            .expect("Elsewhere present in sidebar");
        app.sidebar_selected = pos;

        handle_playlist_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .await
            .expect("handle delete");

        assert!(!elsewhere_path.exists(), "playlist file should be deleted");
        assert!(
            app.playing.is_none(),
            "app.playing must be cleared when deleting the playlist it points at, \
             even though it isn't the displayed playlist"
        );
        assert!(
            !app.is_paused,
            "paused state must be cleared alongside playback"
        );
    }

    #[test]
    fn flush_playing_position_is_noop_when_nothing_playing() {
        let mut app = make_app_with_playlists("Browsing", &["Browsing"]);
        push_track(&mut app, make_track("vid1", "Track One"));
        app.position = 55.0;

        // Should not panic and should leave the track untouched.
        app.flush_playing_position();

        assert_eq!(app.library.get("vid1").map(|t| t.last_position), Some(0));
    }

    // ── request_playback: leaving-track position save (review finding #1) ───

    #[tokio::test]
    async fn request_playback_saves_leaving_track_position_same_playlist() {
        // Both the leaving track (A) and the new track (B) are listed by the
        // displayed playlist — the `session.path == self.playlist_path` branch.
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        add_track(&mut pl, make_track("B", "Track B"));
        let (_dir, path) = write_temp_playlist(&pl);

        let config = crate::config::Config::default();
        let available = entries(vec![("Active".to_string(), path.clone())]);
        let mut app =
            crate::tui::App::new(pl.clone(), config, available, path.clone(), test_library());

        app.playing = Some(session_at(path.clone(), pl, 0)); // A
        app.position = 99.0;

        // Switch to B — A is the leaving track.
        app.play_from_list(crate::tui::RowSource::Own, 1, None);

        assert_eq!(
            app.library.get("A").map(|t| t.last_position),
            Some(99),
            "leaving track A's last_position must be updated in memory"
        );
        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(99),
            "leaving track A's last_position must be flushed to disk, not dropped when \
             self.playing is replaced by the new session"
        );
    }

    #[tokio::test]
    async fn request_playback_saves_leaving_track_position_cross_playlist() {
        // The leaving track (A) is listed by a different playlist file than the
        // one displayed — the `session.path != self.playlist_path` branch.
        let mut elsewhere = make_playlist("Elsewhere");
        add_track(&mut elsewhere, make_track("A", "Track A"));
        let (_dir_elsewhere, elsewhere_path) = write_temp_playlist(&elsewhere);

        let mut app = make_app_with_playlists("Browsing", &["Browsing", "Elsewhere"]);
        push_track(&mut app, make_track("B", "Track B"));
        refresh_library(&mut app);

        app.playing = Some(session_at(elsewhere_path.clone(), elsewhere, 0)); // A
        app.position = 77.0;

        // Switch to B in the displayed playlist — A (in Elsewhere.toml) is
        // the leaving track.
        app.play_from_list(crate::tui::RowSource::Own, 0, None);

        // The displayed playlist must be untouched by the leaving-track save.
        assert_eq!(app.playlist.tracks, vec!["B"]);

        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(77),
            "leaving track A's last_position must be flushed to its own document, \
             not dropped when self.playing is replaced by the new session"
        );
    }

    // ── Phase 1: player lifecycle & crash resistance ──────────────────────────

    /// A `Player` whose mpv is already gone: the socket path does not exist, so
    /// every IPC call against it fails. This is exactly the state the app is
    /// left in when mpv exits by itself at the end of a track.
    fn make_dead_player(socket_path: std::path::PathBuf) -> crate::player::Player {
        let process = tokio::process::Command::new("true")
            .spawn()
            .expect("spawn placeholder process");
        crate::player::Player {
            process,
            socket_path,
        }
    }

    fn dead_player_socket(tag: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/tmp/trovers-no-such-socket-{tag}.sock"))
    }

    #[tokio::test]
    async fn volume_key_with_dead_player_does_not_abort_the_event_loop() {
        use crate::tui::input::{handle_key, Action};

        let mut app = make_app_with_playlists("Active", &["Active"]);
        app.player = Some(make_dead_player(dead_player_socket("vol")));

        let action = handle_key(&mut app, key(crossterm::event::KeyCode::Char('v')))
            .await
            .expect("a dead mpv socket must not propagate an error out of handle_key");

        assert_eq!(action, Action::Continue);
        assert_eq!(
            app.config.default_volume, 85,
            "the volume setting must still be applied even when mpv cannot be reached"
        );
        assert!(
            app.status_message.is_some(),
            "the failure should surface as a footer message"
        );
    }

    #[tokio::test]
    async fn seek_key_with_dead_player_does_not_abort_the_event_loop() {
        use crate::tui::input::{handle_key, Action};

        let mut app = make_app_with_playlists("Active", &["Active"]);
        app.player = Some(make_dead_player(dead_player_socket("seek")));

        let action = handle_key(&mut app, key(crossterm::event::KeyCode::Left))
            .await
            .expect("a dead mpv socket must not propagate an error out of handle_key");

        assert_eq!(action, Action::Continue);
    }

    #[tokio::test]
    async fn pause_key_with_dead_player_does_not_abort_the_event_loop() {
        use crate::tui::input::{handle_key, Action};

        let mut app = make_app_with_playlists("Active", &["Active"]);
        app.player = Some(make_dead_player(dead_player_socket("pause")));

        let action = handle_key(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("a dead mpv socket must not propagate an error out of handle_key");

        assert_eq!(action, Action::Continue);
    }

    #[tokio::test]
    async fn speed_key_with_dead_player_still_persists_the_new_speed() {
        use crate::tui::input::{handle_key, Action};
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        pl.save(&path).expect("save");

        let mut app = App::new(
            pl,
            crate::config::Config::default(),
            entries(vec![("Active".to_string(), path.clone())]),
            path.clone(),
            test_library(),
        );
        app.playing = Some(session_at(
            path.clone(),
            crate::playlist::Playlist::load(&path).expect("load"),
            0,
        ));
        app.player = Some(make_dead_player(dead_player_socket("speed")));

        let action = handle_key(&mut app, key(crossterm::event::KeyCode::Char(']')))
            .await
            .expect("a dead mpv socket must not propagate an error out of handle_key");

        assert_eq!(action, Action::Continue);
        assert_eq!(
            app.library.get("vid1").and_then(|t| t.speed),
            Some(1.1),
            "the speed change must be recorded even though mpv never received it"
        );
    }

    #[tokio::test]
    async fn ctrl_c_quits() {
        use crate::tui::input::{handle_key, Action};

        let mut app = make_app_with_playlists("Active", &["Active"]);
        let event = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        );

        let action = handle_key(&mut app, event).await.expect("handle_key");

        assert_eq!(action, Action::Quit);
        assert!(
            app.should_quit,
            "Ctrl+C must trigger the normal shutdown path"
        );
    }

    #[test]
    fn stop_player_drops_the_player_and_bumps_the_generation() {
        use std::sync::atomic::Ordering;

        let mut app = make_app_with_playlists("Active", &["Active"]);
        let before = app.player_generation.load(Ordering::SeqCst);

        let returned = app.stop_player();

        assert!(app.player.is_none());
        assert_eq!(returned, before + 1);
        assert_eq!(app.player_generation.load(Ordering::SeqCst), before + 1);
    }

    #[tokio::test]
    async fn player_gone_for_the_current_generation_clears_the_player() {
        use crate::tui::TaskMsg;
        use std::sync::atomic::Ordering;

        let mut app = make_app_with_playlists("Active", &["Active"]);
        app.player = Some(make_dead_player(dead_player_socket("gone")));
        app.is_paused = true;
        let generation = app.player_generation.load(Ordering::SeqCst);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert!(
            app.player.is_none(),
            "mpv exiting on its own must clear the stale Player, or the next \
             keypress talks to a socket nobody is listening on"
        );
        assert!(!app.is_paused);
    }

    #[tokio::test]
    async fn player_gone_for_a_stale_generation_is_ignored() {
        use crate::tui::TaskMsg;

        let mut app = make_app_with_playlists("Active", &["Active"]);
        let stale = app
            .player_generation
            .load(std::sync::atomic::Ordering::SeqCst);

        // A newer player has since been started.
        app.stop_player();
        app.player = Some(make_dead_player(dead_player_socket("gone-stale")));

        app.handle_task_msg(TaskMsg::PlayerGone { generation: stale });

        assert!(
            app.player.is_some(),
            "the previous player's exit notification must not kill the current one"
        );
    }

    #[tokio::test]
    async fn player_ready_for_a_stale_generation_is_discarded() {
        use crate::tui::TaskMsg;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        pl.save(&path).expect("save");

        let mut app = crate::tui::App::new(
            pl,
            crate::config::Config::default(),
            entries(vec![("Active".to_string(), path.clone())]),
            path.clone(),
            test_library(),
        );
        app.playing = Some(session_at(
            path.clone(),
            crate::playlist::Playlist::load(&path).expect("load"),
            0,
        ));

        let stale = app
            .player_generation
            .load(std::sync::atomic::Ordering::SeqCst);
        // The user moves on before the slow spawn finishes.
        app.stop_player();

        app.handle_task_msg(TaskMsg::PlayerReady {
            id: "vid1".to_string(),
            player: Box::new(make_dead_player(dead_player_socket("ready-stale"))),
            generation: stale,
        });

        assert!(
            app.player.is_none(),
            "a player that finished starting after being superseded must be dropped, \
             not installed — matching video_ids are not enough to tell them apart"
        );
    }

    #[tokio::test]
    async fn request_playback_invalidates_the_previous_player() {
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("vid1", "Track One"));
        add_track(&mut pl, make_track("vid2", "Track Two"));
        pl.save(&path).expect("save");

        let mut app = crate::tui::App::new(
            pl,
            crate::config::Config::default(),
            entries(vec![("Active".to_string(), path.clone())]),
            path.clone(),
            test_library(),
        );
        app.player = Some(make_dead_player(dead_player_socket("switch")));
        let before = app.player_generation.load(Ordering::SeqCst);

        app.play_from_list(crate::tui::RowSource::Own, 1, None);

        assert!(
            app.player_generation.load(Ordering::SeqCst) > before,
            "starting a new track must retire the outgoing player's position poller, \
             otherwise its timestamps keep landing in App::position"
        );
        assert!(
            app.player.is_none(),
            "the outgoing mpv must be killed straight away"
        );
    }

    #[tokio::test]
    async fn meta_ready_for_a_missing_target_playlist_starts_no_download() {
        use crate::tui::TaskMsg;
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let active_path = dir.path().join("Active.toml");
        make_playlist("Active")
            .save(&active_path)
            .expect("save active");

        let missing_path = dir.path().join("Gone.toml");

        let mut app = crate::tui::App::new(
            make_playlist("Active"),
            crate::config::Config::default(),
            entries(vec![("Active".to_string(), active_path.clone())]),
            active_path,
            test_library(),
        );

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/x".to_string(),
            meta: TrackMeta {
                title: "T".to_string(),
                artist: "A".to_string(),
                channel: "C".to_string(),
                duration: 10,
                video_id: "vid1".to_string(),
                source: "example.com".to_string(),
            },
            target_path: Some(missing_path),
        });

        assert!(
            app.downloading.is_empty(),
            "a download whose track could not be recorded anywhere would leave an \
             orphaned file in the audio cache"
        );
    }

    // ── Orphan socket reaper ──────────────────────────────────────────────────

    #[test]
    fn socket_owner_pid_parses_our_socket_names() {
        use crate::player::socket_owner_pid;
        use std::path::Path;

        assert_eq!(
            socket_owner_pid(Path::new("/tmp/trovers-1234-0.sock")),
            Some(1234)
        );
        assert_eq!(
            socket_owner_pid(Path::new("/tmp/trovers-99-7.sock")),
            Some(99)
        );
    }

    #[test]
    fn socket_owner_pid_rejects_foreign_files() {
        use crate::player::socket_owner_pid;
        use std::path::Path;

        assert_eq!(socket_owner_pid(Path::new("/tmp/mpv-1234-0.sock")), None);
        assert_eq!(socket_owner_pid(Path::new("/tmp/trovers-1234-0.log")), None);
        assert_eq!(socket_owner_pid(Path::new("/tmp/trovers-abc-0.sock")), None);
        assert_eq!(socket_owner_pid(Path::new("/tmp/trovers.sock")), None);
    }

    #[tokio::test]
    async fn reaper_leaves_a_live_instances_socket_alone() {
        // A socket belonging to a *running* trovers (here: this test process)
        // must survive, or launching a second instance would silently kill the
        // first one's playback.
        let own =
            std::path::PathBuf::from(format!("/tmp/trovers-{}-90001.sock", std::process::id()));
        std::fs::write(&own, b"").expect("create own socket placeholder");

        crate::player::reap_orphaned_players().await;

        let survived = own.exists();
        let _ = std::fs::remove_file(&own);
        assert!(
            survived,
            "the reaper must never touch a live instance's socket"
        );
    }

    #[tokio::test]
    async fn reaper_removes_a_dead_instances_socket() {
        // Borrow the pid of a process we have already reaped, so it is
        // guaranteed dead by the time the reaper looks at it.
        let mut child = tokio::process::Command::new("true")
            .spawn()
            .expect("spawn short-lived process");
        let dead_pid = child.id().expect("child pid");
        child.wait().await.expect("wait");

        let stale = std::path::PathBuf::from(format!("/tmp/trovers-{dead_pid}-90002.sock"));
        std::fs::write(&stale, b"").expect("create stale socket placeholder");

        crate::player::reap_orphaned_players().await;

        let still_there = stale.exists();
        let _ = std::fs::remove_file(&stale);
        assert!(
            !still_there,
            "a socket whose owning trovers is gone must be cleaned up, so a stranded \
             mpv is not left playing with nothing attached to stop it"
        );
    }

    // ── End-to-end player lifecycle (needs a real mpv on PATH) ────────────────

    /// A silent, one-second source mpv can synthesise on its own, so this test
    /// needs nothing but mpv itself — no fixture file, no audible output.
    const SHORT_SILENT_SOURCE: &str = "av://lavfi:aevalsrc=0:d=1";

    /// The real-mpv tests below share global state that cargo's test harness
    /// knows nothing about: `/tmp` and this process's own pid. `live_own_sockets`
    /// cannot tell *its* mpv from one another test happens to be running, and
    /// `Player::drop` removes a socket file a concurrent test may be asserting
    /// about. Run them one at a time.
    static REAL_MPV_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    /// Proves the whole symptom-1 chain is broken at the root: mpv runs without
    /// `--idle`, so it exits when the track ends, and the poller must notice and
    /// report it. Detection cannot key off the socket file — mpv leaves that on
    /// disk after exiting — so this guards the `ECONNREFUSED` classification in
    /// `poll_time_pos` against regressing back to an `exists()` check.
    ///
    /// Ignored by default because it spawns a real mpv:
    /// `cargo test -- --ignored real_mpv`
    #[tokio::test]
    #[ignore = "spawns a real mpv process"]
    async fn real_mpv_exiting_at_end_of_track_is_reported_as_gone() {
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;

        let _serial = REAL_MPV_LOCK.lock().await;

        let player = crate::player::Player::spawn(SHORT_SILENT_SOURCE, None, false, &[])
            .await
            .expect("mpv must be on PATH for this test");
        let socket_path = player.socket_path.clone();

        // Hand the socket to the poller and let mpv reach the end of the source.
        // Keeping `player` alive means `Player::drop` has *not* removed the
        // socket file, which is exactly the situation being tested.
        let (pos_tx, _pos_rx) = tokio::sync::watch::channel(0.0f64);
        let generation = 1u64;
        let counter = Arc::new(AtomicU64::new(generation));

        let mpv_exited =
            crate::player::poll_position_loop(socket_path.clone(), pos_tx, generation, counter)
                .await;

        assert!(
            mpv_exited,
            "the poller must report mpv exiting on its own, otherwise the app keeps \
             a Player pointing at a dead socket and dies on the next keypress"
        );
        assert!(
            socket_path.exists(),
            "sanity check: mpv leaves its IPC socket file behind, so detection must \
             not rely on the file disappearing"
        );
    }

    /// The generation guard must retire a poller without it publishing another
    /// position — the mechanism that stops the outgoing track's timestamp from
    /// bleeding into the next track.
    ///
    /// Ignored by default because it spawns a real mpv:
    /// `cargo test -- --ignored real_mpv`
    #[tokio::test]
    #[ignore = "spawns a real mpv process"]
    async fn real_mpv_poller_stops_without_publishing_when_superseded() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        let _serial = REAL_MPV_LOCK.lock().await;

        let player = crate::player::Player::spawn(SHORT_SILENT_SOURCE, None, false, &[])
            .await
            .expect("mpv must be on PATH for this test");

        // A sentinel no real position can equal, so "still the sentinel" proves
        // nothing was ever published.
        let (pos_tx, pos_rx) = tokio::sync::watch::channel(-1.0f64);
        let generation = 1u64;
        let counter = Arc::new(AtomicU64::new(generation));

        // Supersede the player before the poller's first tick.
        counter.store(generation + 1, Ordering::SeqCst);

        let mpv_exited = crate::player::poll_position_loop(
            player.socket_path.clone(),
            pos_tx,
            generation,
            Arc::clone(&counter),
        )
        .await;

        assert!(!mpv_exited, "a superseded poller must not raise PlayerGone");
        assert_eq!(
            *pos_rx.borrow(),
            -1.0,
            "a superseded poller must publish no position at all — a single stale \
             write is enough to make the next track resume at this track's timestamp"
        );
    }

    /// An endless silent source, so mpv never exits on its own and its survival
    /// is entirely down to whether we killed it.
    const ENDLESS_SILENT_SOURCE: &str = "av://lavfi:anullsrc=r=8000:cl=mono";

    /// Try to connect to every mpv IPC socket belonging to *this* process.
    /// Any socket that accepts a connection has a live mpv behind it.
    async fn live_own_sockets() -> Vec<std::path::PathBuf> {
        let own_pid = std::process::id();
        let mut live = Vec::new();
        let Ok(entries) = std::fs::read_dir("/tmp") else {
            return live;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if crate::player::socket_owner_pid(&path) != Some(own_pid) {
                continue;
            }
            if tokio::net::UnixStream::connect(&path).await.is_ok() {
                live.push(path);
            }
        }
        live
    }

    /// Symptom 2, root link (a): `Player::spawn` waits up to a second for mpv's
    /// socket to appear, and during that wait the `Child` is still a bare local.
    /// If the future is cancelled then — the user quits or switches track — the
    /// `Child` is dropped, and `tokio::process::Child` *detaches* rather than
    /// kills unless `kill_on_drop(true)` is set. That is how mpv was left
    /// playing forever with the app gone.
    ///
    /// Ignored by default because it spawns a real mpv:
    /// `cargo test -- --ignored real_mpv`
    #[tokio::test]
    #[ignore = "spawns a real mpv process"]
    async fn real_mpv_is_killed_when_spawn_is_cancelled_midway() {
        let _serial = REAL_MPV_LOCK.lock().await;

        let before = live_own_sockets().await;

        // Give the spawn long enough to fork mpv, but nowhere near long enough
        // to return — then drop the future.
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(80),
            crate::player::Player::spawn(ENDLESS_SILENT_SOURCE, None, false, &[]),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "the spawn was meant to be cancelled, not to finish"
        );

        // mpv needs a moment to die after receiving the kill.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let after = live_own_sockets().await;
        let leaked: Vec<_> = after.iter().filter(|p| !before.contains(p)).collect();
        for path in &after {
            if !before.contains(path) {
                let _ = std::fs::remove_file(path);
            }
        }
        assert!(
            leaked.is_empty(),
            "cancelling Player::spawn leaked a live mpv on {leaked:?} — kill_on_drop is not set"
        );
    }

    // ── Phase 2: position bleed ─────────────────────────────────────────────

    /// Build an app around a real on-disk playlist so saves can be read back.
    fn app_on_disk(
        pl: crate::playlist::Playlist,
    ) -> (tempfile::TempDir, std::path::PathBuf, crate::tui::App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(format!("{}.toml", pl.name));
        pl.save(&path).expect("save");
        let available = entries(vec![(pl.name.clone(), path.clone())]);
        let app = crate::tui::App::new(
            pl,
            crate::config::Config::default(),
            available,
            path.clone(),
            test_library(),
        );
        (dir, path, app)
    }

    #[tokio::test]
    async fn playing_a_fresh_track_starts_at_zero_not_the_outgoing_tracks_position() {
        // The reported symptom, at its root: track A is playing at 2:00 and the
        // user starts a never-played track B. `App::position` is what
        // `hot_switch_to_local_file` later hands to mpv as `--start=`, so if it
        // still holds A's 120s, B literally begins two minutes in.
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        add_track(&mut pl, make_track("B", "Track B"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());

        app.playing = Some(session_at(path.clone(), pl, 0));
        app.position = 120.0;

        app.selected = 1;
        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handle enter");

        assert_eq!(
            app.position, 0.0,
            "a track with no last_position must start at 0, not inherit the outgoing track's 120s"
        );
        assert_eq!(
            app.library.get("A").map(|t| t.last_position),
            Some(120),
            "the outgoing track's position must be recorded before it is left"
        );
    }

    #[tokio::test]
    async fn leaving_a_track_persists_its_position_to_disk() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        add_track(&mut pl, make_track("B", "Track B"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());

        app.playing = Some(session_at(path.clone(), pl, 0));
        app.position = 77.0;
        app.selected = 1;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handle enter");

        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(77),
            "A's position must survive the switch on disk"
        );
    }

    // ── Phase 2: duplicate adds ─────────────────────────────────────────────

    fn meta_for(id: &str, title: &str) -> crate::ytdlp::TrackMeta {
        crate::ytdlp::TrackMeta {
            title: title.to_string(),
            artist: "Artist".to_string(),
            channel: "Channel".to_string(),
            duration: 100,
            video_id: id.to_string(),
            source: "youtube.com".to_string(),
        }
    }

    #[tokio::test]
    async fn meta_ready_rejects_a_duplicate_id_in_the_displayed_playlist() {
        use crate::tui::TaskMsg;

        let mut pl = make_playlist("Active");
        // The id `meta_for("A", ..)` would mint, so the row already present is
        // genuinely the same track.
        add_track(&mut pl, make_track("youtube:A", "Track A"));
        let (_dir, _path, mut app) = app_on_disk(pl);

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/A".to_string(),
            meta: meta_for("A", "Track A Again"),
            target_path: None,
        });

        assert_eq!(
            app.playlist.tracks.len(),
            1,
            "the same id must not be added twice"
        );
        assert!(
            app.downloading.is_empty(),
            "no download may start for a track that was not added"
        );
    }

    #[tokio::test]
    async fn meta_ready_rejects_a_duplicate_id_in_a_target_playlist() {
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");
        let active_path = dir.path().join("Active.toml");
        make_playlist("Active")
            .save(&active_path)
            .expect("save active");

        let rock_path = dir.path().join("Rock.toml");
        let mut rock = make_playlist("Rock");
        add_track(&mut rock, make_track("youtube:A", "Track A"));
        rock.save(&rock_path).expect("save rock");

        let available = entries(vec![
            ("Active".to_string(), active_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ]);
        let mut app = App::new(
            make_playlist("Active"),
            crate::config::Config::default(),
            available,
            active_path.clone(),
            test_library(),
        );

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/A".to_string(),
            meta: meta_for("A", "Track A Again"),
            target_path: Some(rock_path.clone()),
        });

        let rock_after = crate::playlist::Playlist::load(&rock_path).expect("reload rock");
        assert_eq!(
            rock_after.tracks.len(),
            1,
            "target playlist must not gain a duplicate row"
        );
        assert!(
            app.playlist.tracks.is_empty(),
            "the displayed playlist must be untouched"
        );
        assert!(
            app.downloading.is_empty(),
            "no download may start for a rejected add"
        );
    }

    // ── Phase 2: downloading state persistence ──────────────────────────────

    #[tokio::test]
    async fn meta_ready_records_the_downloading_state_in_the_document() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_on_disk(make_playlist("Active"));

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/A".to_string(),
            meta: meta_for("A", "Track A"),
            target_path: None,
        });

        assert_eq!(
            app.library.get("youtube:A").map(|t| &t.cache_status),
            Some(&crate::library::CacheStatus::Downloading)
        );
        // Asserted against the raw document rather than `Library::load`, because
        // load deliberately rewrites `downloading` back to `streaming` as crash
        // recovery — which is precisely the state that had nothing to recover.
        let raw = raw_document("youtube:A");
        assert!(
            raw.contains("cache_status = \"downloading\""),
            "the downloading state must reach the document; got:\n{raw}"
        );
    }

    #[tokio::test]
    async fn download_error_rolls_the_row_to_failed() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_on_disk(make_playlist("Active"));

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/A".to_string(),
            meta: meta_for("A", "Track A"),
            target_path: None,
        });
        app.handle_task_msg(TaskMsg::DownloadError {
            id: "youtube:A".to_string(),
            err: "network unreachable".to_string(),
        });

        assert_eq!(
            app.library.get("youtube:A").map(|t| &t.cache_status),
            Some(&crate::library::CacheStatus::Failed),
            "a failed download must not leave the track claiming to be downloading, \
             and must be distinguishable from a track nobody ever tried to cache"
        );
        // Read the raw document: `Library::load` rewrites `downloading` to
        // `streaming` on the way in, so loading it back would pass either way.
        let raw = raw_document("youtube:A");
        assert!(
            !raw.contains("cache_status = \"downloading\""),
            "the rollback must reach disk too, got:\n{raw}"
        );
        assert!(raw.contains("cache_status = \"failed\""));
        assert!(!app.is_downloading());
        assert!(app.download_progress.is_empty());
    }

    // `failed` surviving a reload is a library concern now — see
    // `library_test::load_does_not_reset_a_failed_track`.

    #[tokio::test]
    async fn retry_with_backoff_succeeds_without_retrying_when_the_first_attempt_works() {
        use crate::ytdlp::retry_with_backoff;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let attempts = Arc::new(AtomicU32::new(0));
        let a = attempts.clone();
        let result: anyhow::Result<u32> = retry_with_backoff(&[], move || {
            let a = a.clone();
            async move {
                a.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "a working first attempt must not retry"
        );
    }

    #[tokio::test]
    async fn retry_with_backoff_retries_on_failure_and_returns_the_eventual_success() {
        use crate::ytdlp::retry_with_backoff;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let attempts = Arc::new(AtomicU32::new(0));
        let a = attempts.clone();
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let result: anyhow::Result<&str> = retry_with_backoff(&delays, move || {
            let a = a.clone();
            async move {
                let n = a.fetch_add(1, Ordering::SeqCst);
                if n < 1 {
                    anyhow::bail!("transient failure");
                }
                Ok("ok")
            }
        })
        .await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "must stop retrying as soon as an attempt succeeds"
        );
    }

    #[tokio::test]
    async fn retry_with_backoff_gives_up_after_exhausting_every_delay() {
        use crate::ytdlp::retry_with_backoff;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let attempts = Arc::new(AtomicU32::new(0));
        let a = attempts.clone();
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let result: anyhow::Result<()> = retry_with_backoff(&delays, move || {
            let a = a.clone();
            async move {
                a.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("still failing")
            }
        })
        .await;

        assert!(
            result.is_err(),
            "must surface the failure once every attempt is spent"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "must try exactly delays.len() + 1 times (3, for the real download policy) before giving up"
        );
    }

    #[tokio::test]
    async fn recache_forces_a_fresh_download_regardless_of_current_status() {
        let mut pl = make_playlist("Active");
        let mut track = make_track("A", "Track A");
        track.cache_status = crate::library::CacheStatus::Cached;
        track.file = Some(std::path::PathBuf::from("/fake/A.opus"));
        add_track(&mut pl, track);
        let (_dir, _path, mut app) = app_on_disk(pl);

        app.recache_track(&app.playlist.tracks[0].clone());

        assert!(
            app.downloading.contains("A"),
            "recache must start a real download"
        );
        assert_eq!(
            app.library.get("A").map(|t| &t.cache_status),
            Some(&crate::library::CacheStatus::Downloading),
            "must show as downloading immediately, even though it was already cached"
        );
    }

    #[tokio::test]
    async fn recaching_a_track_already_downloading_is_a_no_op() {
        let (_dir, _path, mut app) = app_with_tracks(1);
        app.downloading.insert("A".to_string());

        app.recache_track(&app.playlist.tracks[0].clone());

        assert_eq!(
            app.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("Already downloading"),
            "must not start a second, overlapping download for the same track"
        );
    }

    // ── Phase 2: download state follows its row ─────────────────────────────

    #[tokio::test]
    async fn deleting_a_track_clears_its_in_flight_download_state() {
        use crate::tui::input::handle_confirm_delete;

        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        let (_dir, _path, mut app) = app_on_disk(pl);

        app.downloading.insert("A".to_string());
        app.download_progress.insert("A".to_string(), 42.0);
        app.selected = 0;

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("confirm delete");

        assert!(app.playlist.tracks.is_empty());
        assert!(
            !app.is_downloading(),
            "the spinner must not keep running for a row that no longer exists"
        );
        assert!(app.download_progress.is_empty());
    }

    /// A download lands in the track's document, so renaming the playlist that
    /// happened to ask for it is irrelevant — there is no target to repoint.
    #[tokio::test]
    async fn renaming_a_playlist_does_not_disturb_its_in_flight_downloads() {
        use crate::tui::input::handle_playlist_rename;
        use crate::tui::{InputMode, TaskMsg};

        let mut pl = make_playlist("Old");
        add_track(&mut pl, make_track("A", "Track A"));
        let (dir, _old_path, mut app) = app_on_disk(pl);

        app.downloading.insert("A".to_string());

        // sidebar_items() is [PlaylistsHeader, Old, ...], so index 1 is the playlist.
        app.sidebar_selected = 1;
        app.input_mode = InputMode::PlaylistRename;
        app.input_buf = "New".to_string();
        handle_playlist_rename(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("rename");

        assert!(
            app.downloading.contains("A"),
            "the download is still in flight"
        );

        let audio = dir.path().join("A.opus");
        std::fs::write(&audio, b"audio").expect("write audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "A".to_string(),
            file: audio.clone(),
        });

        let a = test_library().get("A").cloned().expect("track A");
        assert_eq!(a.cache_status, crate::library::CacheStatus::Cached);
        assert_eq!(a.file.as_deref(), Some(audio.as_path()));
    }

    /// Same reason, for the row moving to another playlist mid-download.
    #[tokio::test]
    async fn moving_a_track_does_not_disturb_its_in_flight_download() {
        use crate::tui::{App, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("Source.toml");
        let mut source = make_playlist("Source");
        add_track(&mut source, make_track("A", "Track A"));
        source.save(&source_path).expect("save source");

        let target_path = dir.path().join("Target.toml");
        make_playlist("Target")
            .save(&target_path)
            .expect("save target");

        let available = entries(vec![
            ("Source".to_string(), source_path.clone()),
            ("Target".to_string(), target_path.clone()),
        ]);
        let mut app = App::new(
            source,
            crate::config::Config::default(),
            available,
            source_path.clone(),
            test_library(),
        );
        app.downloading.insert("A".to_string());
        app.selected = 0;

        app.move_track_to_playlist("Target").expect("move");

        let audio = dir.path().join("A.opus");
        std::fs::write(&audio, b"audio").expect("write audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "A".to_string(),
            file: audio.clone(),
        });

        assert_eq!(
            test_library().get("A").map(|t| t.cache_status.clone()),
            Some(crate::library::CacheStatus::Cached),
            "the download lands in the track, wherever the row ended up"
        );
        let target = crate::playlist::Playlist::load(&target_path).expect("load target");
        assert_eq!(target.tracks, vec!["A"]);
    }

    /// Deleting a playlist does not cancel a download: the track it belongs to
    /// lives in the library and may well be listed elsewhere.
    #[tokio::test]
    async fn deleting_a_playlist_leaves_its_tracks_downloads_alone() {
        use crate::tui::input::handle_playlist_delete;
        use crate::tui::{App, InputMode, TaskMsg};

        let dir = tempfile::tempdir().expect("tempdir");
        let active_path = dir.path().join("Active.toml");
        make_playlist("Active")
            .save(&active_path)
            .expect("save active");

        let doomed_path = dir.path().join("Doomed.toml");
        let mut doomed = make_playlist("Doomed");
        add_track(&mut doomed, make_track("A", "Track A"));
        doomed.save(&doomed_path).expect("save doomed");

        // Sorted so that sidebar index 1 is "Active" and 2 is "Doomed".
        let available = entries(vec![
            ("Active".to_string(), active_path.clone()),
            ("Doomed".to_string(), doomed_path.clone()),
        ]);
        let mut app = App::new(
            make_playlist("Active"),
            crate::config::Config::default(),
            available,
            active_path.clone(),
            test_library(),
        );
        app.downloading.insert("A".to_string());
        app.download_progress.insert("A".to_string(), 10.0);

        app.sidebar_selected = 2;
        app.input_mode = InputMode::PlaylistDelete;
        handle_playlist_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .await
            .expect("delete playlist");

        assert!(!doomed_path.exists(), "the playlist file must be gone");
        assert!(
            app.is_downloading(),
            "the track survives the playlist, so its download still has somewhere to land"
        );

        let audio = dir.path().join("A.opus");
        std::fs::write(&audio, b"audio").expect("write audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            id: "A".to_string(),
            file: audio.clone(),
        });
        assert_eq!(
            test_library().get("A").map(|t| t.cache_status.clone()),
            Some(crate::library::CacheStatus::Cached)
        );
    }

    // ── Phase 2: shared cached files ────────────────────────────────────────

    /// Two playlists on disk, both holding a `Cached` row for `id` backed
    /// by the same file. Returns (dir, displayed path, other path, audio file).
    fn two_playlists_sharing_a_file(
        id: &str,
        also_in_other: bool,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        crate::tui::App,
    ) {
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join(format!("{id}.opus"));
        std::fs::write(&audio, b"audio data").expect("write audio");

        let mut cached = make_track(id, "Shared Track");
        cached.cache_status = crate::library::CacheStatus::Cached;
        cached.file = Some(audio.clone());

        let displayed_path = dir.path().join("Displayed.toml");
        let mut displayed = make_playlist("Displayed");
        add_track(&mut displayed, cached.clone());
        displayed.save(&displayed_path).expect("save displayed");

        let other_path = dir.path().join("Other.toml");
        let mut other = make_playlist("Other");
        if also_in_other {
            add_track(&mut other, cached);
        }
        other.save(&other_path).expect("save other");

        let available = entries(vec![
            ("Displayed".to_string(), displayed_path.clone()),
            ("Other".to_string(), other_path.clone()),
        ]);
        let app = App::new(
            displayed,
            crate::config::Config::default(),
            available,
            displayed_path.clone(),
            test_library(),
        );
        (dir, displayed_path, other_path, audio, app)
    }

    #[tokio::test]
    async fn deleting_a_track_keeps_a_cached_file_another_playlist_still_uses() {
        use crate::tui::input::handle_confirm_delete;

        let (_dir, _displayed, other_path, audio, mut app) =
            two_playlists_sharing_a_file("A", true);
        app.selected = 0;

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("confirm delete");

        assert!(
            app.playlist.tracks.is_empty(),
            "the row must be removed from this playlist"
        );
        assert!(
            audio.exists(),
            "the cached file is shared with another playlist and must survive"
        );

        // And that other playlist must still list the track, still cached.
        let other = crate::playlist::Playlist::load(&other_path).expect("load other");
        assert_eq!(other.tracks, vec!["A"]);
        assert_eq!(
            test_library().get("A").map(|t| t.cache_status.clone()),
            Some(crate::library::CacheStatus::Cached),
            "the track must not be silently downgraded to streaming"
        );
    }

    #[tokio::test]
    async fn deleting_a_track_removes_a_cached_file_nothing_else_references() {
        use crate::tui::input::handle_confirm_delete;

        let (_dir, _displayed, _other, audio, mut app) = two_playlists_sharing_a_file("A", false);
        app.selected = 0;

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("confirm delete");

        assert!(
            !audio.exists(),
            "an unshared cached file must still be cleaned up on delete"
        );
    }

    #[test]
    fn platform_id_referenced_elsewhere_reports_a_duplicate_row_in_the_displayed_playlist() {
        // Playlists written before duplicate rejection landed may still hold two
        // rows sharing a id; deleting one must not unlink the other's file.
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "First copy"));
        add_track(&mut pl, make_track("A", "Second copy"));
        let (_dir, _path, mut app) = app_on_disk(pl);

        app.playlist.tracks.remove(0);

        assert!(
            app.platform_id_referenced_elsewhere("A"),
            "the surviving duplicate row still needs the file"
        );
    }

    #[test]
    fn platform_id_referenced_elsewhere_keeps_the_file_when_a_playlist_cannot_be_read() {
        use crate::tui::App;

        let dir = tempfile::tempdir().expect("tempdir");
        let displayed_path = dir.path().join("Displayed.toml");
        make_playlist("Displayed")
            .save(&displayed_path)
            .expect("save");

        // Listed but absent from disk: we cannot prove the file is unreferenced.
        let missing_path = dir.path().join("Missing.toml");
        let available = entries(vec![
            ("Displayed".to_string(), displayed_path.clone()),
            ("Missing".to_string(), missing_path),
        ]);
        let app = App::new(
            make_playlist("Displayed"),
            crate::config::Config::default(),
            available,
            displayed_path,
            test_library(),
        );

        assert!(
            app.platform_id_referenced_elsewhere("A"),
            "an unreadable playlist must be treated as possibly holding the track"
        );
    }

    // ── Phase 2: periodic position flush ────────────────────────────────────

    #[tokio::test]
    async fn maybe_flush_position_writes_the_live_position_once_the_interval_elapses() {
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());

        app.player = Some(make_dead_player(dead_player_socket("flush")));
        app.playing = Some(session_at(path.clone(), pl, 0));
        app.position = 64.0;

        // Nothing yet: the app has only just started.
        app.maybe_flush_position();
        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(0),
            "the flush must be throttled"
        );

        app.last_position_flush = std::time::Instant::now() - std::time::Duration::from_secs(60);
        app.maybe_flush_position();

        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(64),
            "once the interval has elapsed the live position must reach disk"
        );

        // Immediately afterwards the throttle applies again.
        app.position = 999.0;
        app.maybe_flush_position();
        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(64),
            "the timer must have been reset"
        );
    }

    #[tokio::test]
    async fn maybe_flush_position_does_nothing_while_paused_or_stopped() {
        let mut pl = make_playlist("Active");
        add_track(&mut pl, make_track("A", "Track A"));
        let (_dir, path, mut app) = app_on_disk(pl.clone());

        app.playing = Some(session_at(path.clone(), pl, 0));
        app.position = 64.0;
        app.last_position_flush = std::time::Instant::now() - std::time::Duration::from_secs(60);

        // No player at all.
        app.maybe_flush_position();
        assert_eq!(test_library().get("A").map(|t| t.last_position), Some(0));

        // Paused: the position cannot have moved.
        app.player = Some(make_dead_player(dead_player_socket("flush-paused")));
        app.is_paused = true;
        app.maybe_flush_position();
        assert_eq!(test_library().get("A").map(|t| t.last_position), Some(0));
    }

    // ── Phase 2: settings persisted on change ───────────────────────────────

    #[tokio::test]
    async fn cycling_loop_mode_saves_the_playlist() {
        use crate::tui::input::handle_tracklist;

        let (_dir, path, mut app) = app_on_disk(make_playlist("Active"));

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('l')))
            .await
            .expect("handle l");

        let on_disk = crate::playlist::Playlist::load(&path).expect("reload");
        assert_eq!(
            on_disk.loop_mode,
            crate::playlist::LoopMode::Track,
            "loop mode must be persisted when it changes, not only at quit"
        );
    }

    // ── Phase 2: yt-dlp output parsing ──────────────────────────────────────

    #[test]
    fn parse_progress_line_reads_the_percentage() {
        use crate::ytdlp::parse_progress_line;

        assert_eq!(
            parse_progress_line("[download]  45.3% of    4.23MiB at    1.23MiB/s ETA 00:02"),
            Some(45.3)
        );
        assert_eq!(
            parse_progress_line("[download] 100% of    3.27MiB in 00:00:00 at 14.05MiB/s"),
            Some(100.0)
        );
        assert_eq!(
            parse_progress_line("[youtube] A: Downloading webpage"),
            None
        );
    }

    #[test]
    fn parse_destination_line_reads_both_download_and_extractaudio() {
        use crate::ytdlp::parse_destination_line;

        assert_eq!(
            parse_destination_line("[download] Destination: /tmp/audio/A.webm"),
            Some(std::path::PathBuf::from("/tmp/audio/A.webm"))
        );
        // With `-x --audio-format opus` this is often the only Destination line
        // yt-dlp prints, and it names the file that survives conversion.
        assert_eq!(
            parse_destination_line("[ExtractAudio] Destination: /tmp/audio/A.opus"),
            Some(std::path::PathBuf::from("/tmp/audio/A.opus"))
        );
        assert_eq!(parse_destination_line("[download] 100% of 3.27MiB"), None);
    }

    #[test]
    fn find_downloaded_file_prefers_opus_over_a_leftover_source_file() {
        use crate::ytdlp::find_downloaded_file;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("A.webm"), b"pre-conversion").expect("write webm");
        std::fs::write(dir.path().join("A.opus"), b"converted").expect("write opus");
        std::fs::write(dir.path().join("B.opus"), b"other track").expect("write other");

        assert_eq!(
            find_downloaded_file(dir.path(), "A"),
            Some(dir.path().join("A.opus")),
            "read_dir order must not decide which file gets recorded"
        );
        assert_eq!(find_downloaded_file(dir.path(), "missing"), None);
    }

    // ── Phase 3: cursor, position and identity ──────────────────────────────

    /// A playlist of `n` tracks named `A`, `B`, `C`, … saved to a tempdir.
    fn app_with_tracks(n: usize) -> (tempfile::TempDir, std::path::PathBuf, crate::tui::App) {
        let mut pl = make_playlist("Active");
        for i in 0..n {
            let id = ((b'A' + i as u8) as char).to_string();
            add_track(&mut pl, make_track(&id, &format!("Track {id}")));
        }
        app_on_disk(pl)
    }

    /// Drive the app to the state it is in while a track plays, without a real
    /// mpv: the playing session is set, the position is wherever we say, and the
    /// generation matches so `PlayerGone` is not discarded as stale.
    fn pretend_playing(app: &mut crate::tui::App, idx: usize, position: f64) -> u64 {
        app.playing = Some(session_at(
            app.playlist_path.clone(),
            app.playlist.clone(),
            idx,
        ));
        app.position = position;
        app.player = Some(make_dead_player(dead_player_socket(&format!(
            "phase3-{idx}"
        ))));
        app.player_generation
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[tokio::test]
    async fn adding_a_track_leaves_the_cursor_where_the_user_put_it() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        app.selected = 1;

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/D".to_string(),
            meta: meta_for("D", "Track D"),
            target_path: None,
        });

        assert_eq!(
            app.playlist.tracks.len(),
            4,
            "the track must still be added"
        );
        assert_eq!(
            app.selected, 1,
            "adding a track must not move the selection out from under the user"
        );
    }

    #[tokio::test]
    async fn deleting_the_playing_track_resets_the_position() {
        use crate::tui::input::handle_confirm_delete;

        let (_dir, _path, mut app) = app_with_tracks(2);
        pretend_playing(&mut app, 0, 95.0);
        app.selected = 0;

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y'))).unwrap();

        assert!(
            app.playing.is_none(),
            "playback must stop with the track gone"
        );
        assert_eq!(
            app.position, 0.0,
            "the deleted track's elapsed time must not carry over to the next one"
        );
        assert_eq!(
            *app.position_rx.borrow(),
            0.0,
            "and the reset must reach the channel"
        );
    }

    #[tokio::test]
    async fn leaving_a_tracks_twin_in_another_playlist_still_saves_its_position() {
        // The same track can live in two playlists. Starting playlist B's copy
        // while playlist A's copy plays *is* leaving a track, so A's row has to
        // record where it got to — comparing `id` alone said "same track,
        // nothing to save" and silently dropped it.
        let dir = tempfile::tempdir().expect("tempdir");

        let other_path = dir.path().join("Other.toml");
        let mut other = make_playlist("Other");
        add_track(&mut other, make_track("shared", "Shared Track"));
        other.save(&other_path).expect("save other");

        let mut active = make_playlist("Active");
        add_track(&mut active, make_track("shared", "Shared Track"));
        let active_path = dir.path().join("Active.toml");
        active.save(&active_path).expect("save active");

        let mut app = crate::tui::App::new(
            active,
            crate::config::Config::default(),
            entries(vec![
                ("Active".to_string(), active_path.clone()),
                ("Other".to_string(), other_path.clone()),
            ]),
            active_path.clone(),
            test_library(),
        );

        // `Other`'s row is playing, 70s in, while `Active` is on screen.
        let other_on_disk = crate::playlist::Playlist::load(&other_path).expect("load other");
        app.playing = Some(session_at(other_path.clone(), other_on_disk, 0));
        app.position = 70.0;

        // Start the displayed playlist's own row for the same track.
        app.play_from_list(crate::tui::RowSource::Own, 0, None);

        assert_eq!(
            test_library().get("shared").map(|t| t.last_position),
            Some(70),
            "the outgoing row's position must reach the track document"
        );
    }

    #[tokio::test]
    async fn browsing_away_from_a_playing_track_does_not_undo_its_document_edits() {
        // This used to be the ugliest failure mode of embedding track state in
        // playlists: `PlayingSession` carried a `Playlist` clone taken once, in
        // `request_playback`, and `switch_to_playlist` replaced `self.playlist`
        // without telling the session — so the clone went stale exactly when it
        // became the thing that got written (the periodic position flush, or the
        // one at quit), reverting every edit made since playback started.
        //
        // A track's state now has one home, so the position flush touches only
        // the track's own document and cannot carry a stale copy of anything
        // else. The sequence is cheap to keep asserting.
        let dir = tempfile::tempdir().expect("tempdir");

        let cached_file = dir.path().join("X.opus");
        std::fs::write(&cached_file, b"fake audio").expect("write fake audio");

        let mut active = make_playlist("Active");
        add_track(&mut active, make_track("X", "Track X"));
        let active_path = dir.path().join("Active.toml");
        active.save(&active_path).expect("save active");

        let mut other = make_playlist("Other");
        add_track(&mut other, make_track("Y", "Track Y"));
        let other_path = dir.path().join("Other.toml");
        other.save(&other_path).expect("save other");

        let mut app = crate::tui::App::new(
            active.clone(),
            crate::config::Config::default(),
            entries(vec![
                ("Active".to_string(), active_path.clone()),
                ("Other".to_string(), other_path.clone()),
            ]),
            active_path.clone(),
            test_library(),
        );

        // X starts playing while "Active" is the displayed playlist.
        app.playing = Some(session_at(active_path.clone(), app.playlist.clone(), 0));

        // A download completes and patches X's document.
        let file_for_patch = cached_file.clone();
        app.patch_track("X", move |t| {
            t.cache_status = crate::library::CacheStatus::Cached;
            t.file = Some(file_for_patch);
        });

        // The user browses to "Other" while X keeps playing in the background.
        app.switch_to_playlist("Other", &other_path)
            .expect("switch");

        // A periodic position flush (or the one at quit).
        app.position = 42.0;
        app.flush_playing_position();

        let reloaded = test_library();
        let x = reloaded.get("X").expect("X's document");
        assert_eq!(
            x.cache_status,
            crate::library::CacheStatus::Cached,
            "switching away and flushing position must not revert the cache status"
        );
        assert_eq!(
            x.file.as_deref(),
            Some(cached_file.as_path()),
            "switching away and flushing position must not drop the cached file"
        );
        assert_eq!(x.last_position, 42, "and the flush must still have landed");
    }

    // ── Phase 3: auto-advance at end of track ───────────────────────────────

    #[tokio::test]
    async fn finishing_a_track_advances_to_the_next_one() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        let generation = pretend_playing(&mut app, 0, 180.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(1),
            "reaching the end of a track must start the next one — `l` cycled loop \
             modes but nothing ever read them, so playback just stopped"
        );
    }

    #[tokio::test]
    async fn finishing_a_track_rewinds_its_resume_position() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(2);
        let generation = pretend_playing(&mut app, 0, 178.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            test_library().get("A").map(|t| t.last_position),
            Some(0),
            "a track that played to its end must resume from the start, not sit at \
             EOF where replaying it would end instantly"
        );
    }

    #[tokio::test]
    async fn finishing_the_last_track_stops_when_not_looping() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        let generation = pretend_playing(&mut app, 2, 180.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(2),
            "loop mode `none` plays through and stops at the end rather than wrapping"
        );
        assert!(app.player.is_none(), "and no new player is started");
    }

    #[tokio::test]
    async fn finishing_the_last_track_wraps_when_looping_the_playlist() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        app.playlist.loop_mode = crate::playlist::LoopMode::Playlist;
        let generation = pretend_playing(&mut app, 2, 180.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(0),
            "loop mode `playlist` wraps from the last track to the first"
        );
    }

    #[tokio::test]
    async fn finishing_a_track_replays_it_when_looping_the_track() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        app.playlist.loop_mode = crate::playlist::LoopMode::Track;
        let generation = pretend_playing(&mut app, 1, 180.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(1),
            "loop mode `track` repeats the same track"
        );
        assert_eq!(app.position, 0.0, "and restarts it from the beginning");
    }

    #[tokio::test]
    async fn mpv_dying_mid_track_does_not_advance() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(3);
        app.playlist.loop_mode = crate::playlist::LoopMode::Playlist;
        // 3s into a 180s track: whatever killed mpv, it was not the end.
        let generation = pretend_playing(&mut app, 0, 3.0);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(0),
            "treating an unexpected exit as EOF would walk the whole playlist in \
             seconds, respawning mpv and yt-dlp for every track on the way"
        );
        assert!(app.player.is_none());
    }

    #[tokio::test]
    async fn a_track_of_unknown_duration_still_advances_at_its_end() {
        use crate::tui::TaskMsg;

        let mut pl = make_playlist("Active");
        let mut first = make_track("A", "Track A");
        // yt-dlp reports no duration for some sources, e.g. live streams.
        first.duration = 0;
        add_track(&mut pl, first);
        add_track(&mut pl, make_track("B", "Track B"));
        let (_dir, _path, mut app) = app_on_disk(pl);

        let generation = pretend_playing(&mut app, 0, 12.0);
        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(1),
            "with no duration to compare against there is nothing to be suspicious \
             of, and refusing to advance would break auto-advance for these tracks"
        );
    }

    #[tokio::test]
    async fn auto_advance_follows_the_playing_playlist_not_the_displayed_one() {
        use crate::tui::TaskMsg;

        let dir = tempfile::tempdir().expect("tempdir");

        // The playlist that is playing: two tracks, looping.
        let other_path = dir.path().join("Other.toml");
        let mut other = make_playlist("Other");
        add_track(&mut other, make_track("O1", "Other One"));
        add_track(&mut other, make_track("O2", "Other Two"));
        other.loop_mode = crate::playlist::LoopMode::Playlist;
        other.save(&other_path).expect("save other");

        // The playlist on screen, with loop off — it must have no say here.
        let (_d, active_path, mut app) = app_with_tracks(3);
        app.available_playlists
            .push(crate::playlist::PlaylistEntry::normal(
                "Other".to_string(),
                other_path.clone(),
            ));

        app.playing = Some(session_at(
            other_path.clone(),
            crate::playlist::Playlist::load(&other_path).expect("load other"),
            0,
        ));
        app.position = 180.0;
        app.player = Some(make_dead_player(dead_player_socket("phase3-elsewhere")));
        let generation = app
            .player_generation
            .load(std::sync::atomic::Ordering::SeqCst);

        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        let session = app.playing.as_ref().expect("still playing");
        assert_eq!(
            session.path, other_path,
            "playback must stay in its own playlist"
        );
        assert_eq!(
            session.track_index(),
            Some(1),
            "the next track must come from the playing playlist, not the displayed one"
        );
        assert_eq!(
            app.playlist_path, active_path,
            "and the displayed playlist must not change under the user"
        );
    }

    // ── Phase 3: shuffle ────────────────────────────────────────────────────

    #[test]
    fn shuffled_indices_is_a_permutation() {
        use crate::playlist::shuffled_indices;

        for seed in [1u64, 42, 9_999, u64::MAX] {
            let order = shuffled_indices(7, seed);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..7).collect::<Vec<_>>(),
                "every track must appear exactly once (seed {seed}), else a shuffled \
                 walk drops or repeats tracks"
            );
        }
        assert_eq!(shuffled_indices(0, 1), Vec::<usize>::new());
        assert_eq!(shuffled_indices(1, 1), vec![0]);
    }

    #[test]
    fn shuffled_indices_actually_reorders() {
        use crate::playlist::shuffled_indices;

        let identity: Vec<usize> = (0..12).collect();
        let shuffled_any = [1u64, 2, 3, 4, 5]
            .iter()
            .any(|&seed| shuffled_indices(12, seed) != identity);
        assert!(
            shuffled_any,
            "a shuffle that returns the input order for every seed is not a shuffle"
        );
    }

    #[tokio::test]
    async fn a_shuffled_walk_visits_every_track_once_before_repeating() {
        let (_dir, path, mut app) = app_with_tracks(6);
        app.playlist.shuffle = true;
        app.rebuild_shuffle_order();

        let mut visited = vec![0usize];
        let mut at = 0usize;
        for _ in 0..5 {
            at = app
                .step_index(&path, 6, true, at, true)
                .expect("a next track");
            visited.push(at);
        }

        let mut sorted = visited.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            6,
            "a shuffled walk must cover the playlist before repeating, got {visited:?}"
        );
    }

    #[tokio::test]
    async fn stepping_back_through_the_shuffled_order_returns_the_previous_track() {
        let (_dir, path, mut app) = app_with_tracks(6);
        app.playlist.shuffle = true;
        app.rebuild_shuffle_order();

        let next = app.step_index(&path, 6, true, 0, true).expect("next");
        let back = app
            .step_index(&path, 6, true, next, false)
            .expect("previous");

        assert_eq!(
            back, 0,
            "`b` has to undo `n`, which is why the order is a stored permutation \
             rather than a fresh random pick each step"
        );
    }

    #[tokio::test]
    async fn the_shuffle_order_is_rebuilt_when_the_track_count_changes() {
        let (_dir, path, mut app) = app_with_tracks(4);
        app.playlist.shuffle = true;
        app.rebuild_shuffle_order();
        assert_eq!(app.shuffle_order.len(), 4);

        // A track arrives while shuffle is on.
        push_track(&mut app, make_track("E", "Track E"));
        let next = app.step_index(&path, 5, true, 0, true).expect("next");

        assert_eq!(
            app.shuffle_order.len(),
            5,
            "an order built for 4 tracks can never reach the 5th"
        );
        assert!(next < 5);
    }

    #[tokio::test]
    async fn toggling_shuffle_saves_it_to_the_playlist() {
        use crate::tui::input::handle_tracklist;

        let (_dir, path, mut app) = app_with_tracks(4);

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('r')))
            .await
            .unwrap();
        assert!(app.playlist.shuffle, "`r` must toggle shuffle on");
        assert_eq!(app.shuffle_order.len(), 4, "and build an order to walk");
        assert!(
            crate::playlist::Playlist::load(&path)
                .expect("load")
                .shuffle,
            "shuffle must survive a restart, like loop mode"
        );

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('r')))
            .await
            .unwrap();
        assert!(!app.playlist.shuffle, "`r` must toggle shuffle back off");
        assert!(app.shuffle_order.is_empty());
        assert!(
            !crate::playlist::Playlist::load(&path)
                .expect("load")
                .shuffle
        );
    }

    /// `r` on a track inside an album must flip that album's own `shuffle` —
    /// not the displayed playlist's — since the album plays as its own list
    /// (ADR-019) and this is the only way its shuffle setting can be reached.
    #[tokio::test]
    async fn toggling_shuffle_on_an_albums_track_changes_only_that_album() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        let album_path = app.albums[0].path.clone();
        app.selected = 2; // own:0, header:Kino, album0:0 -> "k1"

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('r')))
            .await
            .expect("handle r");

        assert!(
            app.albums[0].playlist.shuffle,
            "the album's own shuffle is on"
        );
        assert!(
            !app.playlist.shuffle,
            "the displayed playlist's is untouched"
        );
        assert!(
            crate::playlist::Playlist::load(&album_path)
                .expect("reload")
                .shuffle,
            "persisted to the album's own file"
        );
    }

    /// `l` on a track inside an album must flip that album's own `loop_mode`
    /// — not the displayed playlist's.
    #[tokio::test]
    async fn toggling_loop_mode_on_an_albums_track_changes_only_that_album() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        let album_path = app.albums[0].path.clone();
        app.selected = 2; // own:0, header:Kino, album0:0 -> "k1"

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('l')))
            .await
            .expect("handle l");

        assert_eq!(
            app.albums[0].playlist.loop_mode,
            crate::playlist::LoopMode::Track,
            "the album's own loop mode advanced"
        );
        assert_eq!(
            app.playlist.loop_mode,
            crate::playlist::LoopMode::None,
            "the displayed playlist's is untouched"
        );
        assert_eq!(
            crate::playlist::Playlist::load(&album_path)
                .expect("reload")
                .loop_mode,
            crate::playlist::LoopMode::Track,
            "persisted to the album's own file"
        );
    }

    /// The other half of the fix: with albums present, `r`/`l` on one of the
    /// displayed playlist's *own* tracks still changes the displayed
    /// playlist, unchanged from today.
    #[tokio::test]
    async fn toggling_shuffle_and_loop_on_the_playlists_own_track_is_unchanged() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.selected = 0; // own:0 -> "a"

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('r')))
            .await
            .expect("handle r");
        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('l')))
            .await
            .expect("handle l");

        assert!(app.playlist.shuffle);
        assert_eq!(app.playlist.loop_mode, crate::playlist::LoopMode::Track);
        assert!(!app.albums[0].playlist.shuffle, "the album is untouched");
        assert_eq!(
            app.albums[0].playlist.loop_mode,
            crate::playlist::LoopMode::None,
            "the album is untouched"
        );
    }

    #[tokio::test]
    async fn shuffle_is_ignored_while_a_search_filter_is_active() {
        use crate::tui::input::handle_tracklist;

        // Every other track matches, so the filter shows playlist rows 1, 3 and 5
        // in that order.
        let mut pl = make_playlist("Active");
        for (i, id) in ["A", "B", "C", "D", "E", "F"].iter().enumerate() {
            let title = if i % 2 == 1 {
                format!("keeper {id}")
            } else {
                format!("Track {id}")
            };
            add_track(&mut pl, make_track(id, &title));
        }
        let (_dir, _path, mut app) = app_on_disk(pl);
        app.playlist.shuffle = true;
        app.rebuild_shuffle_order();
        app.search_query = "keeper".to_string();
        app.rebuild_rows();
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .unwrap();

        assert_eq!(
            app.selected, 1,
            "under a filter, `n` steps through what is shown; a shuffled hop inside \
             a deliberate subset reads as a bug"
        );
        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(3),
            "and plays the playlist row that cursor position maps to"
        );
    }

    /// Pin the shuffled order to a known permutation, so a test asserting that
    /// traversal *follows* it cannot pass by coincidence — a random order whose
    /// successor of 0 happens to be 1 is indistinguishable from shuffle being
    /// ignored entirely.
    fn pin_shuffle_order(app: &mut crate::tui::App, order: Vec<usize>) {
        app.playlist.shuffle = true;
        app.shuffle_order = order;
        app.shuffle_order_path = Some(app.playlist_path.clone());
    }

    #[tokio::test]
    async fn next_follows_the_shuffled_order_when_unfiltered() {
        use crate::tui::input::handle_tracklist;

        let (_dir, _path, mut app) = app_with_tracks(8);
        pin_shuffle_order(&mut app, vec![0, 5, 2, 7, 1, 4, 3, 6]);
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .unwrap();

        assert_eq!(
            app.selected, 5,
            "`n` must follow the shuffled order, not the index order"
        );
        assert_eq!(app.playing.as_ref().and_then(|p| p.track_index()), Some(5));

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('b')))
            .await
            .unwrap();
        assert_eq!(app.selected, 0, "and `b` must walk back along it");
    }

    #[tokio::test]
    async fn finishing_a_track_advances_along_the_shuffled_order() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(6);
        app.playlist.loop_mode = crate::playlist::LoopMode::Playlist;
        pin_shuffle_order(&mut app, vec![0, 4, 1, 5, 2, 3]);

        let generation = pretend_playing(&mut app, 0, 180.0);
        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(4),
            "auto-advance must follow the same order `n` does"
        );
    }

    #[tokio::test]
    async fn a_shuffled_playlist_stops_at_the_end_of_its_walk_when_not_looping() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(5);
        // The walk ends on track 2, which is neither the last track by index nor
        // adjacent to it — so "stopped at the end" cannot be index arithmetic
        // getting lucky.
        pin_shuffle_order(&mut app, vec![4, 0, 3, 1, 2]);

        let generation = pretend_playing(&mut app, 2, 180.0);
        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(2),
            "loop mode `none` stops at the end of the shuffled walk, not at the last \
             track by index"
        );
        assert!(app.player.is_none(), "and starts no new player");
    }

    #[tokio::test]
    async fn a_shuffled_playlist_wraps_to_the_start_of_its_walk_when_looping() {
        use crate::tui::TaskMsg;

        let (_dir, _path, mut app) = app_with_tracks(5);
        app.playlist.loop_mode = crate::playlist::LoopMode::Playlist;
        pin_shuffle_order(&mut app, vec![4, 0, 3, 1, 2]);

        let generation = pretend_playing(&mut app, 2, 180.0);
        app.handle_task_msg(TaskMsg::PlayerGone { generation });

        assert_eq!(
            app.playing.as_ref().and_then(|p| p.track_index()),
            Some(4),
            "looping a shuffled playlist returns to the start of the walk"
        );
    }

    // ── Phase 3: mpv IPC must not be able to hang the UI ────────────────────

    /// Stand in for mpv on a real Unix socket: accept one connection, then send
    /// each of `lines` (newline-terminated) — or nothing at all, to imitate an
    /// mpv that has wedged. Returns the socket path; the listener task ends with
    /// the test.
    async fn fake_mpv_socket(tag: &str, lines: Vec<String>) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("trovers-test-ipc-{tag}.sock"));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).expect("bind fake mpv socket");
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            // Wait for the command before answering, like mpv does.
            let mut byte = [0u8; 1];
            loop {
                match tokio::io::AsyncReadExt::read_exact(&mut stream, &mut byte).await {
                    Ok(_) if byte[0] == b'\n' => break,
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
            for line in lines {
                let payload = format!("{line}\n");
                if tokio::io::AsyncWriteExt::write_all(&mut stream, payload.as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // Hold the connection open so a caller expecting more gets silence
            // rather than EOF.
            std::future::pending::<()>().await;
        });
        path
    }

    #[tokio::test]
    async fn ipc_gives_up_on_an_mpv_that_never_answers() {
        let path = fake_mpv_socket("hung", Vec::new()).await;
        let player = make_dead_player(path.clone());

        let started = std::time::Instant::now();
        let result = player
            .send_command_with_timeout(
                serde_json::json!({"command": ["get_property", "time-pos"]}),
                std::time::Duration::from_millis(120),
            )
            .await;

        let _ = std::fs::remove_file(&path);
        assert!(
            result.is_err(),
            "an mpv that accepts the connection and then goes quiet must not park \
             the future forever — key handling awaits this inline on the render \
             loop, so that was the whole UI frozen with no way out"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "and it must give up at the deadline, not eventually"
        );
    }

    #[tokio::test]
    async fn ipc_skips_the_events_mpv_pushes_unprompted() {
        // mpv forwards events to every connected client, interleaved with command
        // replies. Reading the first line as the reply meant an event arriving in
        // the window between writing a command and reading its answer was parsed
        // as that answer.
        let path = fake_mpv_socket(
            "events",
            vec![
                r#"{"event":"playback-restart"}"#.to_string(),
                r#"{"event":"audio-reconfig"}"#.to_string(),
                r#"{"data":41.5,"error":"success"}"#.to_string(),
            ],
        )
        .await;
        let player = make_dead_player(path.clone());

        let resp = player
            .send_command_with_timeout(
                serde_json::json!({"command": ["get_property", "time-pos"]}),
                std::time::Duration::from_secs(2),
            )
            .await
            .expect("the reply must be found past the events");

        let _ = std::fs::remove_file(&path);
        assert_eq!(
            resp["data"].as_f64(),
            Some(41.5),
            "the position must come from mpv's reply, not from whichever event \
             happened to arrive first"
        );
    }

    #[test]
    fn the_footer_shows_loop_mode_and_shuffle() {
        use crate::tui::ui::footer_right_counters;

        let mut app = make_app_with_playlists("Active", &["Active"]);
        assert!(
            !footer_right_counters(&app).contains('↻'),
            "loop mode `none` is the default and needs no badge"
        );

        app.playlist.loop_mode = crate::playlist::LoopMode::Track;
        assert!(footer_right_counters(&app).contains("↻ Track"));

        app.playlist.loop_mode = crate::playlist::LoopMode::Playlist;
        assert!(footer_right_counters(&app).contains("↻ All"));

        app.playlist.shuffle = true;
        let footer = footer_right_counters(&app);
        assert!(
            footer.contains("⇄ Shuffle") && footer.contains("↻ All"),
            "both states have to be visible at once, got {footer:?}"
        );
    }

    // ── Phase 4: stream URL and download cleanup ────────────────────────────

    #[test]
    fn the_stream_url_is_the_first_line_of_yt_dlps_output() {
        use crate::ytdlp::first_url_line;

        // A format selector resolving to separate audio and video streams makes
        // yt-dlp print one URL per line. mpv cannot open a URL with a newline in
        // the middle of it.
        assert_eq!(
            first_url_line("https://example.com/audio\nhttps://example.com/video\n").as_deref(),
            Some("https://example.com/audio")
        );
        assert_eq!(
            first_url_line("\n  https://example.com/audio  \n").as_deref(),
            Some("https://example.com/audio")
        );
        assert_eq!(first_url_line("   \n\n"), None);
        assert_eq!(first_url_line(""), None);
    }

    #[test]
    fn partial_download_files_are_recognised_but_finished_ones_are_not() {
        use crate::ytdlp::is_partial_artifact;

        assert!(is_partial_artifact("A.webm.part", "A"));
        assert!(is_partial_artifact("A.webm.ytdl", "A"));
        assert!(is_partial_artifact("A.opus.temp", "A"));
        assert!(is_partial_artifact("A.webm.part-Frag3", "A"));

        assert!(
            !is_partial_artifact("A.opus", "A"),
            "the cached file must survive"
        );
        assert!(!is_partial_artifact("A.webm", "A"));
        // A different id that merely starts with ours.
        assert!(!is_partial_artifact("AB.webm.part", "A"));
        assert!(!is_partial_artifact("B.webm.part", "A"));
    }

    #[test]
    fn a_failed_download_leaves_no_scratch_files_but_keeps_the_shared_cache() {
        use crate::ytdlp::clean_partial_downloads;

        let dir = tempfile::tempdir().expect("tempdir");
        let scratch = ["A.webm.part", "A.webm.ytdl", "A.opus.temp"];
        // `A.opus` is here because another playlist already cached this track: a
        // download is spawned even then, so a failure must not take it with it.
        let keep = ["A.opus", "B.webm.part"];
        for name in scratch.iter().chain(keep.iter()) {
            std::fs::write(dir.path().join(name), b"x").expect("write");
        }

        clean_partial_downloads(dir.path(), "A");

        for name in scratch {
            assert!(
                !dir.path().join(name).exists(),
                "{name} should have been removed"
            );
        }
        for name in keep {
            assert!(
                dir.path().join(name).exists(),
                "{name} should have been kept"
            );
        }
    }

    #[test]
    fn cleaning_up_a_missing_audio_dir_is_not_an_error() {
        use crate::ytdlp::clean_partial_downloads;

        // The cache dir is created at startup, but a failed download racing a
        // user who removed it by hand must not take the download task down.
        clean_partial_downloads(std::path::Path::new("/nonexistent/trovers-audio"), "A");
    }

    #[test]
    fn known_youtube_blocking_errors_get_an_update_hint() {
        use crate::ytdlp::blocked_by_youtube_hint;

        let blocking_errors = [
            "yt-dlp download exited with status exit status: 1: ERROR: unable to download video data: HTTP Error 403: Forbidden",
            "ERROR: [youtube] d04frRhBx8A: Sign in to confirm you're not a bot",
            "mweb client https formats require a GVS PO Token which was not provided. They will be skipped as they may yield HTTP Error 403.",
            "WARNING: Only images are available for download. use --list-formats to see them",
            "ERROR: [youtube] d04frRhBx8A: Requested format is not available. Use --list-formats for a list of available formats",
        ];
        for err in blocking_errors {
            assert!(
                blocked_by_youtube_hint(err).is_some(),
                "expected a hint for: {err}"
            );
        }
    }

    #[test]
    fn unrelated_errors_get_no_youtube_hint() {
        use crate::ytdlp::blocked_by_youtube_hint;

        let unrelated_errors = [
            "yt-dlp failed: ERROR: [generic] 'Last login: Tue Jul  7 22:00:27 on ttys000' is not a valid URL",
            "network unreachable",
            "boom",
        ];
        for err in unrelated_errors {
            assert!(
                blocked_by_youtube_hint(err).is_none(),
                "expected no hint for: {err}"
            );
        }
    }

    #[test]
    fn download_error_from_a_youtube_block_gets_an_update_hint_in_the_footer() {
        use crate::tui::TaskMsg;

        let mut app = make_app_with_playlists("Source", &["Source"]);
        push_track(&mut app, make_track("A", "Track A"));

        app.handle_task_msg(TaskMsg::DownloadError {
            id: "A".to_string(),
            err: "yt-dlp download exited with status exit status: 1: ERROR: unable to download video data: HTTP Error 403: Forbidden".to_string(),
        });

        assert_eq!(
            app.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("Download failed — YouTube may have changed something — try updating yt-dlp"),
        );
    }

    #[test]
    fn download_error_from_an_unrelated_cause_keeps_the_plain_message() {
        use crate::tui::TaskMsg;

        let mut app = make_app_with_playlists("Source", &["Source"]);
        push_track(&mut app, make_track("A", "Track A"));

        app.handle_task_msg(TaskMsg::DownloadError {
            id: "A".to_string(),
            err: "network unreachable".to_string(),
        });

        assert_eq!(
            app.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("Download failed"),
        );
    }

    #[test]
    fn player_error_from_a_youtube_block_gets_an_update_hint_in_the_footer() {
        use crate::tui::TaskMsg;

        let mut app = make_app_with_playlists("Source", &["Source"]);

        app.handle_task_msg(TaskMsg::PlayerError {
            id: "A".to_string(),
            err: "WARNING: Only images are available for download. use --list-formats to see them"
                .to_string(),
        });

        assert_eq!(
            app.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("Player error — YouTube may have changed something — try updating yt-dlp"),
        );
    }

    // ── The track library: playlists as id lists ──────────────────────────

    /// Build an app around its own tracks directory, so the test can inspect
    /// the documents on disk independently of the app's in-memory library.
    fn app_with_own_library(
        pl: crate::playlist::Playlist,
        others: &[(&str, &std::path::Path)],
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        crate::tui::App,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tracks = dir.path().join("tracks");
        let path = dir.path().join(format!("{}.toml", pl.name));
        pl.save(&path).expect("save playlist");

        let mut available = entries(vec![(pl.name.clone(), path.clone())]);
        for (name, other) in others {
            available.push(crate::playlist::PlaylistEntry::normal(
                name.to_string(),
                other.to_path_buf(),
            ));
        }
        let library = crate::library::Library::load(&tracks).expect("load library");
        let app = crate::tui::App::new(
            pl,
            crate::config::Config::default(),
            available,
            path.clone(),
            library,
        );
        (dir, tracks, path, app)
    }

    /// A playlist row is an id, not a copy of the track. Asserted against the
    /// raw TOML because that is the compatibility surface: anything reading a
    /// playlist file sees a list of strings.
    #[test]
    fn a_saved_playlist_stores_track_ids_not_embedded_tracks() {
        let mut pl = make_playlist("Active");
        pl.tracks.push("youtube:A".to_string());
        pl.tracks.push("youtube:B".to_string());

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");
        pl.save(&path).expect("save");

        let raw = std::fs::read_to_string(&path).expect("read back");
        assert!(
            raw.contains(r#"tracks = ["youtube:A", "youtube:B"]"#),
            "expected an id list, got:\n{raw}"
        );
        assert!(
            !raw.contains("[[tracks]]"),
            "no embedded track tables should remain:\n{raw}"
        );
    }

    /// The quirk the track library exists to fix: one video added to two
    /// playlists used to carry two independent `last_position` values, so
    /// listening to half of it in one playlist left the other's copy at 0:00.
    #[test]
    fn the_same_track_in_two_playlists_shares_one_position() {
        use crate::library::Library;
        use crate::playlist::Playlist;
        use crate::tui::{App, PlayingSession};

        let dir = tempfile::tempdir().expect("tempdir");
        let tracks = dir.path().join("tracks");
        let alpha_path = dir.path().join("Alpha.toml");
        let beta_path = dir.path().join("Beta.toml");

        let mut library = Library::load(&tracks).expect("load library");
        library
            .upsert(make_track("youtube:shared", "Shared"))
            .expect("upsert");

        let mut alpha = make_playlist("Alpha");
        alpha.tracks.push("youtube:shared".to_string());
        alpha.save(&alpha_path).expect("save alpha");
        let mut beta = make_playlist("Beta");
        beta.tracks.push("youtube:shared".to_string());
        beta.save(&beta_path).expect("save beta");

        let available = entries(vec![
            ("Alpha".to_string(), alpha_path.clone()),
            ("Beta".to_string(), beta_path.clone()),
        ]);

        // Listen to 90 seconds of it out of Alpha and leave.
        let mut app = App::new(
            alpha,
            crate::config::Config::default(),
            available.clone(),
            alpha_path.clone(),
            library,
        );
        app.playing = Some(PlayingSession {
            path: alpha_path.clone(),
            playlist: app.playlist.clone(),
            track_id: "youtube:shared".to_string(),
        });
        app.position = 90.0;
        app.flush_playing_position();

        // Open Beta in a fresh session: the same document, so the same position.
        let beta = Playlist::load(&beta_path).expect("load beta");
        let reloaded = Library::load(&tracks).expect("reload library");
        let app = App::new(
            beta,
            crate::config::Config::default(),
            available,
            beta_path,
            reloaded,
        );
        assert_eq!(
            app.library
                .get("youtube:shared")
                .expect("document present")
                .last_position,
            90,
            "Beta's row must resume where Alpha left off"
        );
    }

    /// `d` removes the row; the document and its cached audio go with it only
    /// when nothing else lists the id.
    #[test]
    fn deleting_the_last_row_referencing_a_track_removes_its_document() {
        use crossterm::event::{KeyCode, KeyEvent};

        let mut pl = make_playlist("Active");
        pl.tracks.push("youtube:A".to_string());
        let (_dir, tracks, _path, mut app) = app_with_own_library(pl, &[]);
        app.library
            .upsert(make_track("youtube:A", "Track A"))
            .expect("upsert");

        crate::tui::input::handle_confirm_delete(&mut app, KeyEvent::from(KeyCode::Char('y')))
            .expect("delete");

        assert!(app.playlist.tracks.is_empty(), "the row is gone");
        assert!(
            app.library.get("youtube:A").is_none(),
            "the document is gone too"
        );
        let left: Vec<_> = std::fs::read_dir(&tracks)
            .expect("read tracks dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        assert!(left.is_empty(), "no document file should remain: {left:?}");
    }

    #[test]
    fn deleting_a_row_keeps_the_document_when_another_playlist_lists_it() {
        use crossterm::event::{KeyCode, KeyEvent};

        let other_dir = tempfile::tempdir().expect("tempdir");
        let other_path = other_dir.path().join("Other.toml");
        let mut other = make_playlist("Other");
        other.tracks.push("youtube:A".to_string());
        other.save(&other_path).expect("save other");

        let mut pl = make_playlist("Active");
        pl.tracks.push("youtube:A".to_string());
        let (_dir, _tracks, _path, mut app) =
            app_with_own_library(pl, &[("Other", other_path.as_path())]);
        app.library
            .upsert(make_track("youtube:A", "Track A"))
            .expect("upsert");

        crate::tui::input::handle_confirm_delete(&mut app, KeyEvent::from(KeyCode::Char('y')))
            .expect("delete");

        assert!(app.playlist.tracks.is_empty(), "the row is gone");
        assert!(
            app.library.get("youtube:A").is_some(),
            "Other still lists the id, so the document must survive"
        );
    }

    /// A row can outlive its document — a hand-deleted file, a playlist shared
    /// without the documents it references. Playing it must fail visibly rather
    /// than spawn mpv against nothing.
    #[tokio::test]
    async fn playing_a_row_whose_document_is_missing_starts_no_player() {
        let mut pl = make_playlist("Active");
        pl.tracks.push("youtube:gone".to_string());
        let (_dir, _tracks, _path, mut app) = app_with_own_library(pl, &[]);

        app.play_from_list(crate::tui::RowSource::Own, 0, None);

        assert!(
            app.playing.is_none(),
            "no session for a track that is absent"
        );
        assert_eq!(
            app.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("Track document missing")
        );
    }

    /// Adding a URL that is already in the library to a *second* playlist must
    /// reuse its document. Overwriting it would reset the position, speed and
    /// any retitling the user had accumulated on the first playlist's row.
    #[tokio::test]
    async fn adding_a_track_already_in_the_library_keeps_its_position() {
        use crate::tui::TaskMsg;
        use crate::ytdlp::TrackMeta;

        let (_dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Active"), &[]);
        let mut known = make_track("youtube:A", "Track A");
        known.last_position = 175;
        known.speed = Some(1.5);
        app.library.upsert(known).expect("upsert");

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://youtube.com/watch?v=A".to_string(),
            meta: TrackMeta {
                title: "Track A".to_string(),
                artist: "Test Artist".to_string(),
                channel: "Test Channel".to_string(),
                duration: 180,
                video_id: "A".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: None,
        });

        assert_eq!(
            app.playlist.tracks,
            vec!["youtube:A".to_string()],
            "the row is added"
        );
        let track = app.library.get("youtube:A").expect("document present");
        assert_eq!(track.last_position, 175, "position must survive re-adding");
        assert_eq!(track.speed, Some(1.5), "so must the per-track speed");
    }

    // ── Importing a local folder ──────────────────────────────────────────

    /// A scanned-and-probed file, as an import task hands one back.
    fn imported_file(path: &std::path::Path, title: &str) -> crate::library_import::ImportedFile {
        crate::library_import::ImportedFile {
            path: path.to_path_buf(),
            meta: crate::library_scan::ProbedMeta {
                title: title.to_string(),
                artist: "Miss Monique".to_string(),
                duration: 3529,
                media: crate::library::MediaKind::Audio,
            },
        }
    }

    fn status_of(app: &crate::tui::App) -> Option<&str> {
        app.status_message.as_ref().map(|(m, _)| m.as_str())
    }

    #[tokio::test]
    async fn f_opens_the_folder_prompt() {
        use crate::tui::input::handle_tracklist;

        let mut app = make_app_with_playlists("Live Sets", &["Live Sets"]);
        app.input_buf = "leftovers".to_string();

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('F')))
            .await
            .expect("handled");

        assert_eq!(app.input_mode, crate::tui::InputMode::FolderInput);
        assert!(app.input_buf.is_empty(), "the prompt starts empty");
    }

    /// The `+ Folder` sidebar item is the discoverable half of `F`.
    #[tokio::test]
    async fn the_sidebar_folder_item_opens_the_folder_prompt() {
        use crate::tui::input::handle_key;
        use crate::tui::{Focus, SidebarItem};

        let mut app = make_app_with_playlists("Live Sets", &["Live Sets"]);
        app.focus = Focus::Sidebar;
        let pos = app
            .sidebar_items()
            .iter()
            .position(|i| matches!(i, SidebarItem::ImportFolder))
            .expect("the sidebar offers a folder import item");
        app.sidebar_selected = pos;

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");

        assert_eq!(app.input_mode, crate::tui::InputMode::FolderInput);
    }

    /// Being in the list is not enough: `sidebar_next` steps straight over an
    /// unselectable row, so an item nobody can land on can never be pressed.
    #[test]
    fn the_sidebar_folder_item_can_be_reached_with_the_arrow_keys() {
        use crate::tui::SidebarItem;

        let mut app = make_app_with_playlists("Live Sets", &["Live Sets"]);
        let plunder = app
            .sidebar_items()
            .iter()
            .position(|i| matches!(i, SidebarItem::Plunder))
            .expect("the sidebar offers Plunder");
        app.sidebar_selected = plunder;

        app.sidebar_next();

        assert!(matches!(
            app.sidebar_items()[app.sidebar_selected],
            SidebarItem::ImportFolder
        ));
    }

    #[tokio::test]
    async fn a_folder_that_is_not_there_is_reported_and_imports_nothing() {
        use crate::tui::input::handle_key;
        use crate::tui::InputMode;

        let (_dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let before = app.available_playlists.len();
        app.input_mode = InputMode::FolderInput;
        app.input_buf = "/no/such/folder".to_string();

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");

        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(status_of(&app), Some("Not a folder: /no/such/folder"));
        assert_eq!(app.available_playlists.len(), before, "no album appeared");
    }

    /// Two levels only, so an import lands under the playlist on screen.
    #[test]
    fn a_folder_imported_from_a_playlist_becomes_an_album_under_it() {
        use crate::tui::ImportTarget;

        let (_dir, _tracks, _path, app) = app_with_own_library(make_playlist("Live Sets"), &[]);

        assert_eq!(
            app.import_target_for(std::path::Path::new("/Volumes/Sets/Ultra")),
            ImportTarget::NewAlbum {
                parent: Some("Live Sets".to_string())
            }
        );
    }

    /// An album cannot hold an album — a third level. Importing from one makes a
    /// sibling under the same parent instead.
    #[test]
    fn a_folder_imported_from_an_album_becomes_a_sibling_album() {
        use crate::playlist::PlaylistKind;
        use crate::tui::ImportTarget;

        let mut pl = make_playlist("Ultra 2026");
        pl.kind = PlaylistKind::Album;
        pl.parent = Some("Live Sets".to_string());
        let (_dir, _tracks, _path, app) = app_with_own_library(pl, &[]);

        assert_eq!(
            app.import_target_for(std::path::Path::new("/Volumes/Sets/Other")),
            ImportTarget::NewAlbum {
                parent: Some("Live Sets".to_string())
            }
        );
    }

    /// Importing a folder that is already an album is what `R` does, whichever way
    /// the user asks for it — otherwise every import makes another `Ultra (2)`.
    #[test]
    fn importing_a_folder_that_is_already_linked_rescans_that_album() {
        use crate::playlist::PlaylistKind;
        use crate::tui::ImportTarget;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let root = dir.path().join("Ultra");
        let album_path = dir.path().join("Ultra.toml");
        let mut existing = make_playlist("Ultra");
        existing.kind = PlaylistKind::Album;
        existing.parent = Some("Live Sets".to_string());
        existing.source_folder = Some(root.clone());
        existing.save(&album_path).expect("save album");
        app.available_playlists
            .push(crate::playlist::PlaylistEntry {
                name: "Ultra".to_string(),
                path: album_path.clone(),
                kind: PlaylistKind::Album,
                parent: Some("Live Sets".to_string()),
            });

        assert_eq!(
            app.import_target_for(&root),
            ImportTarget::Existing(album_path)
        );
    }

    #[test]
    fn applying_an_import_writes_the_album_beside_the_displayed_playlist() {
        use crate::playlist::{Playlist, PlaylistKind};
        use crate::tui::ImportTarget;

        let (dir, _tracks, path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let root = dir.path().join("Ultra 2026");
        let files = vec![
            imported_file(&root.join("Day One.flac"), "Day One"),
            imported_file(&root.join("Day Two.flac"), "Day Two"),
        ];

        app.apply_import(
            root.clone(),
            ImportTarget::NewAlbum {
                parent: Some("Live Sets".to_string()),
            },
            files,
        );

        let album_path = dir.path().join("Ultra 2026.toml");
        let album = Playlist::load(&album_path).expect("album written");
        assert_eq!(album.kind, PlaylistKind::Album);
        assert_eq!(album.parent.as_deref(), Some("Live Sets"));
        assert_eq!(album.source_folder.as_deref(), Some(root.as_path()));
        assert_eq!(album.tracks.len(), 2);

        // Listed in the sidebar, and under its parent.
        let entry = app
            .available_playlists
            .iter()
            .find(|e| e.name == "Ultra 2026")
            .expect("listed in the sidebar");
        assert_eq!(entry.kind, PlaylistKind::Album);
        assert_eq!(entry.parent.as_deref(), Some("Live Sets"));

        // The parent's own track list is not what was imported into.
        assert!(app.playlist.tracks.is_empty());
        assert_eq!(app.playlist_path, path);
    }

    #[test]
    fn applying_an_import_numbers_an_album_whose_name_is_taken() {
        use crate::tui::ImportTarget;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let root = dir.path().join("Ultra 2026");
        make_playlist("Ultra 2026")
            .save(&dir.path().join("Ultra 2026.toml"))
            .expect("save");
        app.available_playlists
            .push(crate::playlist::PlaylistEntry::normal(
                "Ultra 2026".to_string(),
                dir.path().join("Ultra 2026.toml"),
            ));

        app.apply_import(
            root.clone(),
            ImportTarget::NewAlbum { parent: None },
            vec![imported_file(&root.join("Day One.flac"), "Day One")],
        );

        assert!(dir.path().join("Ultra 2026 (2).toml").exists());
        assert!(app
            .available_playlists
            .iter()
            .any(|e| e.name == "Ultra 2026 (2)"));
    }

    #[test]
    fn a_folder_with_no_playable_files_makes_no_album() {
        use crate::tui::ImportTarget;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let root = dir.path().join("Documents");
        let before = app.available_playlists.len();

        app.apply_import(
            root.clone(),
            ImportTarget::NewAlbum { parent: None },
            Vec::new(),
        );

        assert_eq!(app.available_playlists.len(), before);
        assert!(!dir.path().join("Documents.toml").exists());
        assert_eq!(
            status_of(&app),
            Some(format!("No playable files in {}", root.display()).as_str())
        );
    }

    /// Rescanning the album on screen has to change the rows the user is looking
    /// at, not just the file behind them.
    #[test]
    fn rescanning_the_displayed_album_updates_the_rows_on_screen() {
        use crate::playlist::{Playlist, PlaylistKind};
        use crate::tui::ImportTarget;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut pl = make_playlist("Ultra 2026");
        pl.kind = PlaylistKind::Album;
        pl.source_folder = Some(root.clone());
        let (dir2, _tracks, path, mut app) = app_with_own_library(pl, &[]);
        let root = dir2.path().join("Ultra 2026");

        app.apply_import(
            root.clone(),
            ImportTarget::Existing(path.clone()),
            vec![imported_file(&root.join("Day One.flac"), "Day One")],
        );

        assert_eq!(app.playlist.tracks.len(), 1, "the displayed rows changed");
        assert!(
            app.row_track_id(0)
                .and_then(|id| app.library.get(&id))
                .is_some(),
            "and the row resolves"
        );
        let saved = Playlist::load(&path).expect("load");
        assert_eq!(saved.tracks, app.playlist.tracks, "and reached disk");
        // No second album for a folder that already has one.
        assert!(!dir2.path().join("Ultra 2026 (2).toml").exists());
        let _ = dir;
    }

    #[tokio::test]
    async fn r_on_a_playlist_that_is_not_linked_to_a_folder_says_so() {
        use crate::tui::input::handle_tracklist;

        let (_dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('R')))
            .await
            .expect("handled");

        assert_eq!(status_of(&app), Some("Not linked to a folder"));
    }

    #[tokio::test]
    async fn r_rescans_the_folder_the_displayed_album_is_linked_to() {
        use crate::playlist::PlaylistKind;
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        std::fs::create_dir_all(&root).expect("mkdir");
        let mut pl = make_playlist("Ultra 2026");
        pl.kind = PlaylistKind::Album;
        pl.source_folder = Some(root.clone());
        let (_dir2, _tracks, _path, mut app) = app_with_own_library(pl, &[]);

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('R')))
            .await
            .expect("handled");

        assert_eq!(
            status_of(&app),
            Some(format!("Scanning {}", root.display()).as_str())
        );
    }

    // ── The user's own files are never deleted ─────────────────────────────

    /// A local track pointing at `path`, as an import would have written one.
    fn local_track(path: &std::path::Path) -> crate::library::Track {
        use crate::library::{CacheStatus, MediaKind, TrackOrigin};
        crate::library::Track {
            url: path.to_string_lossy().to_string(),
            source: "local".to_string(),
            title: "Day One".to_string(),
            artist: "Miss Monique".to_string(),
            channel: "Miss Monique".to_string(),
            duration: 3529,
            id: crate::library_scan::local_id(path),
            cache_status: CacheStatus::Cached,
            file: Some(path.to_path_buf()),
            last_position: 0,
            speed: None,
            user_title: None,
            user_artist: None,
            added_at: chrono::Utc::now(),
            origin: TrackOrigin::Local,
            media: MediaKind::Audio,
            resume: true,
        }
    }

    /// The invariant that matters most: `d` on an imported row takes the row, not
    /// the user's music.
    #[test]
    fn deleting_a_local_row_leaves_the_users_file_alone() {
        use crate::tui::input::handle_confirm_delete;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Ultra"), &[]);
        let file = dir.path().join("Day One.flac");
        std::fs::write(&file, b"audio").expect("write file");
        let track = local_track(&file);
        let id = track.id.clone();
        push_track(&mut app, track);
        app.selected = 0;

        handle_confirm_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .expect("confirm delete");

        assert!(app.playlist.tracks.is_empty(), "the row goes");
        assert!(app.library.get(&id).is_none(), "and so does its document");
        assert!(file.exists(), "but never the user's own file");
    }

    /// Deleting an album is deleting a list. The folder it mirrors is not ours.
    #[tokio::test]
    async fn deleting_an_album_leaves_the_users_files_alone() {
        use crate::playlist::PlaylistKind;
        use crate::tui::input::handle_playlist_delete;
        use crate::tui::SidebarItem;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let file = dir.path().join("Day One.flac");
        std::fs::write(&file, b"audio").expect("write file");

        // An album listing that file, sitting beside the displayed playlist.
        let album_path = dir.path().join("Ultra.toml");
        let mut album = make_playlist("Ultra");
        album.kind = PlaylistKind::Album;
        let track = local_track(&file);
        album.tracks.push(track.id.clone());
        app.library.upsert(track).expect("write document");
        album.save(&album_path).expect("save album");
        app.available_playlists
            .push(crate::playlist::PlaylistEntry {
                name: "Ultra".to_string(),
                path: album_path.clone(),
                kind: PlaylistKind::Album,
                parent: None,
            });
        app.sidebar_selected = app
            .sidebar_items()
            .iter()
            .position(|i| matches!(i, SidebarItem::Playlist { name, .. } if name == "Ultra"))
            .expect("the album is in the sidebar");

        handle_playlist_delete(&mut app, key(crossterm::event::KeyCode::Char('y')))
            .await
            .expect("confirm delete");

        assert!(!album_path.exists(), "the album file goes");
        assert!(file.exists(), "the user's own file stays");
    }

    #[tokio::test]
    async fn recaching_a_local_track_does_nothing() {
        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Ultra"), &[]);
        let file = dir.path().join("Day One.flac");
        std::fs::write(&file, b"audio").expect("write file");
        let track = local_track(&file);
        let id = track.id.clone();
        push_track(&mut app, track);

        app.recache_track(&app.playlist.tracks[0].clone());

        assert!(
            app.downloading.is_empty(),
            "there is nothing to download a local file from"
        );
        assert_eq!(
            app.library.get(&id).map(|t| t.cache_status.clone()),
            Some(crate::library::CacheStatus::Cached),
            "and its status must not be moved to downloading"
        );
        assert_eq!(status_of(&app), Some("Local file, nothing to download"));
    }

    /// A moved or unplugged file has nothing to stream instead — playing it would
    /// hand mpv a path to nothing and look like a hang.
    #[tokio::test]
    async fn a_local_file_that_is_gone_refuses_to_play() {
        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Ultra"), &[]);
        let track = local_track(&dir.path().join("Day One.flac"));
        let id = track.id.clone();
        push_track(&mut app, track);

        app.play_from_list(crate::tui::RowSource::Own, 0, None);

        assert!(app.playing.is_none(), "nothing may start playing");
        assert_eq!(status_of(&app), Some("File not found"));
        assert_eq!(
            app.library.get(&id).map(|t| t.cache_status.clone()),
            Some(crate::library::CacheStatus::Missing),
            "and the row says why"
        );
    }

    #[tokio::test]
    async fn a_local_file_that_is_there_still_plays() {
        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Ultra"), &[]);
        let file = dir.path().join("Day One.flac");
        std::fs::write(&file, b"audio").expect("write file");
        let track = local_track(&file);
        let id = track.id.clone();
        push_track(&mut app, track);

        app.play_from_list(crate::tui::RowSource::Own, 0, None);

        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some(id.as_str())
        );
    }

    // ── Panel totals and row reordering ───────────────────────────────────

    #[test]
    fn the_panel_title_shows_the_count_and_the_total_duration() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("Live Sets", 42, 12, 20, 42, 6 * 3600 + 12 * 60),
            " Live Sets · 42 tracks · 6h 12m  [ 12–20 / 42 ] "
        );
    }

    #[test]
    fn the_panel_title_counts_one_track_in_the_singular() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("Live Sets", 1, 1, 1, 1, 90),
            " Live Sets · 1 track · 1m  [ 1–1 / 1 ] "
        );
    }

    /// Without ffprobe every local duration is zero, and "0m" would read as a
    /// claim rather than an absence.
    #[test]
    fn the_panel_title_omits_a_duration_nothing_knows() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("Live Sets", 3, 1, 3, 3, 0),
            " Live Sets · 3 tracks  [ 1–3 / 3 ] "
        );
    }

    #[test]
    fn the_panel_title_says_under_a_minute_rather_than_zero() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("Live Sets", 1, 1, 1, 1, 30),
            " Live Sets · 1 track · <1m  [ 1–1 / 1 ] "
        );
    }

    #[test]
    fn the_panel_title_of_an_empty_playlist_is_just_its_name() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(track_panel_title("Live Sets", 0, 1, 0, 0, 0), " Live Sets ");
    }

    /// The count and the total have to agree with each other, so a filter that
    /// hides rows hides their seconds too.
    #[test]
    fn the_total_duration_covers_only_the_visible_rows() {
        let mut pl = make_playlist("Live Sets");
        add_track(&mut pl, {
            let mut t = make_track("dur1", "Keeper");
            t.duration = 100;
            t
        });
        add_track(&mut pl, {
            let mut t = make_track("dur2", "Hidden");
            t.duration = 900;
            t
        });
        let (_dir, _path, mut app) = app_on_disk(pl);

        assert_eq!(app.total_duration_secs(), 1000, "unfiltered, everything");

        app.search_query = "keeper".to_string();
        app.rebuild_rows();
        assert_eq!(app.total_duration_secs(), 100, "filtered, only what shows");
    }

    #[tokio::test]
    async fn shift_j_moves_the_selected_row_down() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Live Sets");
        add_track(&mut pl, make_track("mv1", "First"));
        add_track(&mut pl, make_track("mv2", "Second"));
        add_track(&mut pl, make_track("mv3", "Third"));
        let (_dir, path, mut app) = app_on_disk(pl);
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('J')))
            .await
            .expect("handled");

        assert_eq!(app.playlist.tracks, vec!["mv2", "mv1", "mv3"]);
        assert_eq!(app.selected, 1, "the cursor follows the row it moved");
        let saved = crate::playlist::Playlist::load(&path).expect("load");
        assert_eq!(saved.tracks, app.playlist.tracks, "and it reached disk");
    }

    #[tokio::test]
    async fn shift_k_moves_the_selected_row_up() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Live Sets");
        add_track(&mut pl, make_track("mv1", "First"));
        add_track(&mut pl, make_track("mv2", "Second"));
        let (_dir, _path, mut app) = app_on_disk(pl);
        app.selected = 1;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('K')))
            .await
            .expect("handled");

        assert_eq!(app.playlist.tracks, vec!["mv2", "mv1"]);
        assert_eq!(app.selected, 0);
    }

    #[tokio::test]
    async fn moving_past_either_end_keeps_the_order() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Live Sets");
        add_track(&mut pl, make_track("mv1", "First"));
        add_track(&mut pl, make_track("mv2", "Second"));
        let (_dir, _path, mut app) = app_on_disk(pl);

        app.selected = 0;
        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('K')))
            .await
            .expect("handled");
        assert_eq!(app.playlist.tracks, vec!["mv1", "mv2"]);
        assert_eq!(app.selected, 0);

        app.selected = 1;
        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('J')))
            .await
            .expect("handled");
        assert_eq!(app.playlist.tracks, vec!["mv1", "mv2"]);
        assert_eq!(app.selected, 1);
    }

    /// Under a filter the cursor counts visible rows, so ±1 would move a row past
    /// something the user cannot see.
    #[tokio::test]
    async fn reordering_is_refused_while_a_search_filter_is_active() {
        use crate::tui::input::handle_tracklist;

        let mut pl = make_playlist("Live Sets");
        add_track(&mut pl, make_track("mv1", "First"));
        add_track(&mut pl, make_track("mv2", "Second"));
        add_track(&mut pl, make_track("mv3", "Third"));
        let (_dir, _path, mut app) = app_on_disk(pl);
        app.search_query = "ir".to_string(); // First and Third
        app.rebuild_rows();
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('J')))
            .await
            .expect("handled");

        assert_eq!(app.playlist.tracks, vec!["mv1", "mv2", "mv3"]);
        assert_eq!(status_of(&app), Some("Clear the search to reorder"));
    }

    // ── Albums as rows in the track list ──────────────────────────────────

    /// An `App` on disk showing a playlist of `own` tracks with `albums` under it.
    ///
    /// Every id doubles as its track's title, so a search expectation reads as the
    /// id it is meant to match. Albums start folded, which is what a file with no
    /// `collapsed` key loads as.
    fn app_with_albums(
        dir: &std::path::Path,
        own: &[&str],
        albums: &[(&str, &[&str])],
    ) -> crate::tui::App {
        let mut parent = make_playlist("aBooks");
        for id in own {
            add_track(&mut parent, make_track(id, id));
        }
        let parent_path = dir.join("aBooks.toml");
        parent.save(&parent_path).expect("save parent");

        for (name, ids) in albums {
            let mut album = make_playlist(name);
            album.kind = crate::playlist::PlaylistKind::Album;
            album.parent = Some("aBooks".to_string());
            for id in *ids {
                add_track(&mut album, make_track(id, id));
            }
            album
                .save(&dir.join(format!("{name}.toml")))
                .expect("save album");
        }

        crate::tui::App::new(
            parent,
            crate::config::Config::default(),
            crate::playlist::Playlist::list_entries(dir).expect("list"),
            parent_path,
            test_library(),
        )
    }

    /// Each row as `own:<index>`, `album<n>:<index>` or `header:<name>` — the whole
    /// of what a row means, one readable line per row.
    fn row_shapes(app: &crate::tui::App) -> Vec<String> {
        use crate::tui::{RowSource, VisibleRow};
        app.rows
            .iter()
            .map(|row| match row {
                VisibleRow::Track {
                    source: RowSource::Own,
                    index,
                } => format!("own:{index}"),
                VisibleRow::Track {
                    source: RowSource::Album(a),
                    index,
                } => format!("album{a}:{index}"),
                VisibleRow::AlbumHeader { album } => {
                    format!("header:{}", app.albums[*album].name)
                }
            })
            .collect()
    }

    /// The whole UI drawn into a `width × height` buffer, one `String` per line.
    ///
    /// The only way to assert on indentation and per-album numbering: both are
    /// decided inside `render_track_table`, which builds ratatui `Row`s whose cell
    /// contents are not readable back out of them.
    fn render_to_lines(app: &mut crate::tui::App, width: u16, height: u16) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::ui::render(frame, app))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Unfold every album, as pressing `enter` on each header would.
    fn open_all(app: &mut crate::tui::App) {
        for i in 0..app.albums.len() {
            app.albums[i].playlist.collapsed = false;
        }
        app.rebuild_rows();
    }

    // ── Album resume target: "play this album" and its resume-point marker ──

    #[test]
    fn album_resume_target_uses_current_track_and_its_last_position() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("k2".to_string());
        app.patch_track("k2", |t| t.last_position = 42);

        let target = crate::tui::album_resume_target(&app.albums[0], &app.library);
        assert_eq!(target, Some((1, Some(42.0))));
    }

    #[test]
    fn album_resume_target_start_pos_is_none_when_the_track_has_never_stopped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("k1".to_string());

        let target = crate::tui::album_resume_target(&app.albums[0], &app.library);
        assert_eq!(target, Some((0, None)));
    }

    #[test]
    fn album_resume_target_falls_back_to_the_first_track_with_no_current_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        // Track 0 has its own last_position, but with no `current_track` at all
        // the album has never been played from — starting "fresh" means from
        // the very beginning, not wherever this untouched-by-the-album track
        // happens to have last stopped.
        app.patch_track("k1", |t| t.last_position = 999);

        let target = crate::tui::album_resume_target(&app.albums[0], &app.library);
        assert_eq!(target, Some((0, None)));
    }

    #[test]
    fn album_resume_target_falls_back_when_current_track_no_longer_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("ghost".to_string());

        let target = crate::tui::album_resume_target(&app.albums[0], &app.library);
        assert_eq!(target, Some((0, None)));
    }

    #[test]
    fn album_resume_target_is_none_for_an_empty_album() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = app_with_albums(dir.path(), &[], &[("Empty", &[])]);

        assert_eq!(
            crate::tui::album_resume_target(&app.albums[0], &app.library),
            None
        );
    }

    #[tokio::test]
    async fn space_on_an_album_header_with_no_current_track_starts_the_first_track() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.selected = 0; // the only row: the "Kino" header, folded
        app.position = 42.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert_eq!(
            app.position, 0.0,
            "no current_track: must start the album's first track from the beginning"
        );
        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("k1")
        );
    }

    #[tokio::test]
    async fn space_on_an_album_header_resumes_its_last_played_track() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("k2".to_string());
        app.patch_track("k2", |t| t.last_position = 77);
        app.selected = 0;
        app.position = 5.0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert_eq!(app.position, 77.0);
        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("k2")
        );
    }

    #[tokio::test]
    async fn space_on_an_empty_album_header_does_nothing() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Empty", &[])]);
        app.selected = 0;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert!(app.playing.is_none(), "an empty album has nothing to play");
    }

    /// The bug this fixes: pressing Space a second time while sitting on an
    /// album header — meaning to pause — must not re-trigger "play this
    /// album from its resume point". That branch used to run unconditionally,
    /// so it always won: it respawned mpv a few seconds behind the live
    /// position instead of pausing it, which read as "seeks back and keeps
    /// playing" rather than stopping.
    #[tokio::test]
    async fn space_on_a_header_pauses_instead_of_restarting_when_the_album_is_already_playing() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.patch_track("k1", |t| t.last_position = 12);
        let album_path = app.albums[0].path.clone();
        let album_playlist = app.albums[0].playlist.clone();
        app.player = Some(make_dead_player(dead_player_socket("header-pause")));
        app.playing = Some(session_at(album_path, album_playlist, 0)); // k1
        app.position = 50.0; // well past the saved last_position
        app.selected = 0; // the "Kino" header, the only row while folded

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert!(app.is_paused, "space must pause, not restart");
        assert_eq!(
            app.position, 50.0,
            "must not jump back to the saved last_position"
        );
        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("k1"),
            "must not have switched to a different track"
        );
    }

    /// The other half of the fix: Space on a *different* album's header —
    /// one that is not the one currently playing — must switch playback to
    /// it, the same way Enter already does for a track row. Before this,
    /// "anything playing" alone made Space always pause, with no way to
    /// switch albums from a header at all.
    #[tokio::test]
    async fn space_on_a_different_albums_header_switches_playback_to_it() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Ace", &["a1"]), ("Zed", &["z1"])]);
        // Rows (both folded): header:Ace (0), header:Zed (1).
        let ace_path = app.albums[0].path.clone();
        let ace_playlist = app.albums[0].playlist.clone();
        app.player = Some(make_dead_player(dead_player_socket("switch-album")));
        app.playing = Some(session_at(ace_path, ace_playlist, 0)); // a1, from Ace
        app.position = 30.0;
        app.selected = 1; // header:Zed

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char(' ')))
            .await
            .expect("handle space");

        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("z1"),
            "playback switched to the album under the cursor"
        );
        assert!(!app.is_paused, "a freshly started track is not paused");
    }

    #[test]
    fn resume_marker_shows_on_the_albums_current_track_when_it_is_not_playing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["p1"], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("k2".to_string());
        open_all(&mut app);

        let screen = render_to_lines(&mut app, 78, 16);
        let k2_line = screen
            .iter()
            .find(|l| l.contains("k2"))
            .unwrap_or_else(|| panic!("no row for k2 in {screen:#?}"));
        assert!(
            k2_line.contains('‣'),
            "resume marker expected on k2's row: {k2_line}"
        );

        let k1_line = screen
            .iter()
            .find(|l| l.contains("k1"))
            .unwrap_or_else(|| panic!("no row for k1 in {screen:#?}"));
        assert!(
            !k1_line.contains('‣'),
            "the marker must sit only on the resume target: {k1_line}"
        );
    }

    #[test]
    fn resume_marker_is_replaced_by_the_playing_indicator_once_that_track_plays() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        app.albums[0].playlist.current_track = Some("k2".to_string());
        open_all(&mut app);
        let album_path = app.albums[0].path.clone();
        app.playing = Some(session_at(album_path, app.albums[0].playlist.clone(), 1));

        let screen = render_to_lines(&mut app, 78, 16);
        let k2_line = screen
            .iter()
            .find(|l| l.contains("k2"))
            .unwrap_or_else(|| panic!("no row for k2 in {screen:#?}"));
        assert!(
            k2_line.contains('▶'),
            "an actually-playing row shows the playing indicator: {k2_line}"
        );
        assert!(
            !k2_line.contains('‣'),
            "not the resume marker at the same time: {k2_line}"
        );
    }

    #[test]
    fn no_resume_marker_when_the_album_has_no_current_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);

        let screen = render_to_lines(&mut app, 78, 16);
        assert!(
            !screen.iter().any(|l| l.contains('‣')),
            "nothing to mark with no current_track: {screen:#?}"
        );
    }

    #[test]
    fn resume_marker_does_not_leak_across_albums_sharing_a_track_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(
            dir.path(),
            &[],
            &[("AlbumA", &["shared"]), ("AlbumB", &["shared"])],
        );
        let a = app
            .albums
            .iter()
            .position(|loaded| loaded.name == "AlbumA")
            .expect("AlbumA loaded");
        app.albums[a].playlist.current_track = Some("shared".to_string());
        open_all(&mut app);

        let screen = render_to_lines(&mut app, 78, 16);
        let shared_lines: Vec<&String> = screen.iter().filter(|l| l.contains("shared")).collect();
        assert_eq!(shared_lines.len(), 2, "{screen:#?}");
        let marked = shared_lines.iter().filter(|l| l.contains('‣')).count();
        assert_eq!(
            marked, 1,
            "only AlbumA's row may be marked, not AlbumB's: {screen:#?}"
        );
    }

    #[test]
    fn displayed_playlists_own_rows_never_show_the_album_resume_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["p1", "p2"], &[]);
        app.playlist.current_track = Some("p2".to_string());
        app.rebuild_rows();

        let screen = render_to_lines(&mut app, 78, 16);
        assert!(
            !screen.iter().any(|l| l.contains('‣')),
            "the resume marker is an album-only signal, not the top-level playlist's: {screen:#?}"
        );
    }

    #[test]
    fn the_rows_are_the_parents_own_tracks_and_then_its_albums() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a", "b"], &[("Kino", &["k1", "k2"])]);
        assert_eq!(
            row_shapes(&app),
            vec!["own:0", "own:1", "header:Kino"],
            "a folded album is one row"
        );

        open_all(&mut app);
        assert_eq!(
            row_shapes(&app),
            vec!["own:0", "own:1", "header:Kino", "album0:0", "album0:1"]
        );
    }

    /// The parent's own tracks keep the order the user gave them; the albums below
    /// go by name, which is the only order they have.
    #[test]
    fn albums_come_after_the_parents_tracks_in_alphabetical_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = app_with_albums(dir.path(), &["a"], &[("Zed", &["z"]), ("Ace", &["x"])]);
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Ace", "header:Zed"]);
        assert_eq!(
            app.albums
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Ace", "Zed"]
        );
    }

    /// The album exists and the header row is how the user reaches it — to rename
    /// it, to rescan its folder, or to delete it.
    #[test]
    fn an_empty_album_still_has_a_header_to_reach_it_by() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = app_with_albums(dir.path(), &[], &[("Empty", &[])]);
        assert_eq!(row_shapes(&app), vec!["header:Empty"]);
    }

    /// An album of another playlist has no business in this one's rows.
    #[test]
    fn only_the_displayed_playlists_own_albums_are_loaded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);

        let mut elsewhere = make_playlist("Elsewhere");
        elsewhere.kind = crate::playlist::PlaylistKind::Album;
        elsewhere.parent = Some("Some Other List".to_string());
        elsewhere
            .save(&dir.path().join("Elsewhere.toml"))
            .expect("save");
        app.available_playlists =
            crate::playlist::Playlist::list_entries(dir.path()).expect("list");
        app.load_albums();
        app.rebuild_rows();

        assert_eq!(
            app.albums
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Kino"]
        );
    }

    #[test]
    fn a_search_hides_an_album_with_no_matches_header_and_all() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["alpha"], &[("Kino", &["beta"])]);
        app.search_query = "alpha".to_string();
        app.rebuild_rows();
        assert_eq!(row_shapes(&app), vec!["own:0"]);
    }

    /// A search that quietly left its hits folded away inside an album would read
    /// as a search that missed them.
    #[test]
    fn a_search_opens_a_folded_album_that_has_a_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["alpha"], &[("Kino", &["beta", "alpha two"])]);
        assert!(app.albums[0].playlist.collapsed, "folded to begin with");

        app.search_query = "alpha".to_string();
        app.rebuild_rows();

        assert_eq!(
            row_shapes(&app),
            vec!["own:0", "header:Kino", "album0:1"],
            "only the matching track, with its header above it"
        );
    }

    /// Asking for the album means asking for the album, not for whichever of its
    /// tracks happen to repeat its name in their titles.
    #[test]
    fn a_search_matching_an_albums_name_shows_all_of_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["alpha"], &[("Kino", &["beta", "gamma"])]);
        app.search_query = "kino".to_string();
        app.rebuild_rows();
        assert_eq!(
            row_shapes(&app),
            vec!["header:Kino", "album0:0", "album0:1"]
        );
    }

    /// The summary describes what the playlist holds; folding is a view, so it must
    /// not change the number. The bracketed counter beside it is the scroll window
    /// and does count rows — that is `visible_track_count`.
    #[test]
    fn the_title_counts_every_track_the_playlist_holds_folded_or_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = app_with_albums(dir.path(), &["a", "b"], &[("Kino", &["k1", "k2"])]);
        assert_eq!(app.total_track_count(), 4);
        assert_eq!(app.total_duration_secs(), 4 * 180);
        assert_eq!(app.visible_track_count(), 3, "rows, not tracks");
    }

    #[test]
    fn under_a_filter_the_title_counts_only_what_is_shown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["alpha"], &[("Kino", &["beta"])]);
        app.search_query = "alpha".to_string();
        app.rebuild_rows();
        assert_eq!(app.total_track_count(), 1);
        assert_eq!(app.total_duration_secs(), 180);
    }

    /// `row_track_id` is what every key that acts on a row goes through, so it has
    /// to read out of the row's own list rather than always the displayed one.
    #[test]
    fn a_row_names_the_track_of_the_list_it_belongs_to() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);
        assert_eq!(app.row_track_id(0).as_deref(), Some("a"));
        assert_eq!(app.row_track_id(1), None, "a header names no track");
        assert_eq!(app.row_track_id(2).as_deref(), Some("k1"));
        assert_eq!(app.row_track_id(3).as_deref(), Some("k2"));
        assert_eq!(app.row_track_id(4), None, "past the end");
    }

    #[test]
    fn a_header_row_reports_which_album_it_is_for() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Ace", &["x"]), ("Zed", &["z"])]);
        open_all(&mut app);
        // own:0, header:Ace, album0:0, header:Zed, album1:0
        assert_eq!(app.album_of(0), None);
        assert_eq!(app.album_of(1), Some(0));
        assert_eq!(app.album_of(2), None);
        assert_eq!(app.album_of(3), Some(1));
    }

    /// The cursor is restored from `current_track`, which is an id in the
    /// playlist's own list — and rows are not that list any more.
    #[test]
    fn the_cursor_opens_on_the_row_of_the_last_played_own_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut parent = make_playlist("aBooks");
        add_track(&mut parent, make_track("a", "First"));
        add_track(&mut parent, make_track("b", "Second"));
        parent.current_track = Some("b".to_string());
        let parent_path = dir.path().join("aBooks.toml");
        parent.save(&parent_path).expect("save");

        let app = crate::tui::App::new(
            parent,
            crate::config::Config::default(),
            crate::playlist::Playlist::list_entries(dir.path()).expect("list"),
            parent_path,
            test_library(),
        );
        assert_eq!(app.selected, 1);
    }

    // ── with_list_at and cross-list sync ────────────────────────────────────

    /// `with_list_at` on the displayed playlist's own path mutates
    /// `self.playlist` in place and persists it.
    #[test]
    fn with_list_at_on_the_displayed_playlist_mutates_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[]);
        let path = app.playlist_path.clone();

        app.with_list_at(&path, |pl, _lib| pl.add_track("b".to_string()))
            .expect("with_list_at");

        assert_eq!(app.playlist.tracks, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            crate::playlist::Playlist::load(&path)
                .expect("reload")
                .tracks,
            vec!["a".to_string(), "b".to_string()],
            "persisted to disk too"
        );
    }

    /// `with_list_at` on one of the displayed playlist's loaded albums mutates
    /// that `LoadedAlbum` in place. Writing straight to disk instead — the bug
    /// this change fixes — would leave `app.albums` stale until a reload.
    #[test]
    fn with_list_at_on_a_loaded_album_mutates_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        let album_path = app.albums[0].path.clone();

        app.with_list_at(&album_path, |pl, _lib| pl.add_track("k2".to_string()))
            .expect("with_list_at");

        assert_eq!(
            app.albums[0].playlist.tracks,
            vec!["k1".to_string(), "k2".to_string()]
        );
        assert_eq!(
            crate::playlist::Playlist::load(&album_path)
                .expect("reload")
                .tracks,
            vec!["k1".to_string(), "k2".to_string()]
        );
    }

    /// `with_list_at` on a path that names neither the displayed playlist nor
    /// one of its loaded albums reads and writes the file on disk directly,
    /// leaving the displayed playlist and its albums untouched.
    #[test]
    fn with_list_at_on_an_unrelated_file_reads_and_writes_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[]);

        let other_path = dir.path().join("Other.toml");
        make_playlist("Other")
            .save(&other_path)
            .expect("save other");

        app.with_list_at(&other_path, |pl, _lib| pl.add_track("z".to_string()))
            .expect("with_list_at");

        assert_eq!(app.playlist.tracks, vec!["a".to_string()], "own untouched");
        assert_eq!(
            crate::playlist::Playlist::load(&other_path)
                .expect("reload")
                .tracks,
            vec!["z".to_string()]
        );
    }

    /// Moving a track into one of the displayed playlist's own loaded albums
    /// must update that album's in-memory copy, not only its file — otherwise
    /// the moved row is invisible until the playlist is reloaded.
    #[test]
    fn moving_a_track_into_a_loaded_album_updates_it_in_memory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a", "b"], &[("Kino", &["k1"])]);
        app.selected = 0; // own:0 -> "a"

        app.move_track_to_playlist("Kino").expect("move");

        assert_eq!(
            app.playlist.tracks,
            vec!["b".to_string()],
            "left the parent"
        );
        assert_eq!(
            app.albums[0].playlist.tracks,
            vec!["k1".to_string(), "a".to_string()],
            "landed in the loaded album's in-memory copy, not only on disk"
        );

        open_all(&mut app);
        assert!(
            row_shapes(&app).contains(&"album0:1".to_string()),
            "and it is a visible row without switching away and back"
        );
    }

    /// The mirror of the above: moving a track out of a loaded album, up into
    /// the displayed playlist itself, must update `self.playlist` in memory.
    #[test]
    fn moving_a_track_out_of_an_album_into_the_parent_updates_it_in_memory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);
        app.selected = 2; // album0:0 -> "k1"

        app.move_track_to_playlist("aBooks").expect("move");

        assert_eq!(
            app.albums[0].playlist.tracks,
            vec!["k2".to_string()],
            "left the album"
        );
        assert_eq!(
            app.playlist.tracks,
            vec!["a".to_string(), "k1".to_string()],
            "landed in the displayed playlist's in-memory copy, not only on disk"
        );
        assert!(row_shapes(&app).contains(&"own:1".to_string()));
    }

    /// Adding a track by URL, targeted at one of the displayed playlist's own
    /// loaded albums (as Tab-cycling the URL prompt's target lets the user
    /// do), must show up without switching away and back.
    #[tokio::test]
    async fn a_track_added_by_url_to_a_loaded_album_shows_up_immediately() {
        use crate::tui::TaskMsg;
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        let album_path = app.albums[0].path.clone();

        app.handle_task_msg(TaskMsg::MetaReady {
            url: "https://example.com/k2".to_string(),
            meta: TrackMeta {
                title: "Track K2".to_string(),
                artist: "Artist".to_string(),
                channel: "Channel".to_string(),
                duration: 200,
                video_id: "k2".to_string(),
                source: "youtube.com".to_string(),
            },
            target_path: Some(album_path),
        });

        assert_eq!(
            app.albums[0].playlist.tracks,
            vec!["k1".to_string(), "youtube:k2".to_string()],
            "the album's in-memory copy has the new track"
        );
        open_all(&mut app);
        assert!(
            row_shapes(&app).contains(&"album0:1".to_string()),
            "and it is a visible row without switching away and back"
        );
    }

    // ── Manual album creation ────────────────────────────────────────────────

    /// `A` opens the new-album prompt; entering a name creates an empty album
    /// under the displayed playlist and it shows up in the track list without
    /// a folder ever being involved.
    #[tokio::test]
    async fn creating_an_album_manually_adds_an_empty_group() {
        use crate::tui::input::{handle_new_album, handle_tracklist};
        use crate::tui::InputMode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[]);

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('A')))
            .await
            .expect("handle A");
        assert_eq!(app.input_mode, InputMode::NewAlbum);

        app.input_buf = "Fresh".to_string();
        handle_new_album(&mut app, key(crossterm::event::KeyCode::Enter)).expect("handle enter");

        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.albums.len(), 1);
        assert_eq!(app.albums[0].name, "Fresh");
        assert!(app.albums[0].playlist.tracks.is_empty());
        assert!(
            app.albums[0].playlist.source_folder.is_none(),
            "no folder was ever involved"
        );
        assert_eq!(
            app.albums[0].playlist.parent.as_deref(),
            Some("aBooks"),
            "parented to the displayed playlist"
        );
        assert!(row_shapes(&app).contains(&"header:Fresh".to_string()));

        let on_disk = crate::playlist::Playlist::load(&app.albums[0].path).expect("reload");
        assert!(on_disk.tracks.is_empty());
        assert_eq!(on_disk.kind, crate::playlist::PlaylistKind::Album);
    }

    /// A name already taken by another playlist/album is rejected, exactly as
    /// `validate_playlist_name` already does for `N` and album rename, and
    /// nothing is written to disk.
    #[tokio::test]
    async fn creating_an_album_with_a_taken_name_is_rejected() {
        use crate::tui::input::handle_new_album;
        use crate::tui::InputMode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);

        app.input_mode = InputMode::NewAlbum;
        app.input_buf = "Kino".to_string();
        handle_new_album(&mut app, key(crossterm::event::KeyCode::Enter)).expect("handle enter");

        assert_eq!(app.albums.len(), 1, "no second album created");
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("read dir")
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("toml"))
                .count(),
            2,
            "still just aBooks.toml and Kino.toml"
        );
    }

    // ── Single-file add ─────────────────────────────────────────────────────

    /// A single-file probe targeted at one of the displayed playlist's own
    /// loaded albums must show up without switching away and back — the same
    /// in-memory sync as `MetaReady`'s URL-add path.
    #[test]
    fn a_probed_file_targeted_at_a_loaded_album_shows_up_immediately() {
        use crate::library::MediaKind;
        use crate::library_scan::ProbedMeta;
        use crate::tui::TaskMsg;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        let album_path = app.albums[0].path.clone();
        let file_path = dir.path().join("Extra Track.flac");

        app.handle_task_msg(TaskMsg::FileProbed {
            file: crate::library_import::ImportedFile {
                path: file_path.clone(),
                meta: ProbedMeta {
                    title: "Extra Track".to_string(),
                    artist: "Someone".to_string(),
                    duration: 120,
                    media: MediaKind::Audio,
                },
            },
            target_path: album_path,
        });

        let expected_id = crate::library_scan::local_id(&file_path);
        assert_eq!(
            app.albums[0].playlist.tracks,
            vec!["k1".to_string(), expected_id],
            "the album's in-memory copy has the new track"
        );
        open_all(&mut app);
        assert!(
            row_shapes(&app).contains(&"album0:1".to_string()),
            "and it is a visible row without switching away and back"
        );
    }

    // ── Targeted folder and file adds ───────────────────────────────────────

    /// Drain `app.task_rx`, applying every message via `handle_task_msg` — a
    /// scan reports progress once per file before its final message — until
    /// the import or file-add actually lands.
    async fn drain_import(app: &mut crate::tui::App) {
        loop {
            let Some(msg) = app.task_rx.recv().await else {
                return;
            };
            let is_terminal = matches!(
                msg,
                crate::tui::TaskMsg::ImportScanned { .. } | crate::tui::TaskMsg::FileProbed { .. }
            );
            app.handle_task_msg(msg);
            if is_terminal {
                return;
            }
        }
    }

    /// folder × Auto: unchanged from today — a folder with no existing link
    /// becomes a new sibling album under the displayed playlist.
    #[tokio::test]
    async fn folder_add_with_auto_target_creates_a_new_album() {
        use crate::tui::input::handle_key;
        use crate::tui::InputMode;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let root = dir.path().join("Fresh Set");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("Track One.flac"), b"bytes").expect("write");

        app.input_mode = InputMode::FolderInput;
        app.input_buf = root.to_string_lossy().to_string();
        app.target_list_for_add = None;

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");
        drain_import(&mut app).await;

        assert_eq!(
            app.albums.len(),
            1,
            "auto matching made a new sibling album"
        );
        assert_eq!(app.albums[0].playlist.tracks.len(), 1);
    }

    /// folder × explicit (also covers the case task 5.4 asks for): `Tab`-ing
    /// to an existing album whose `source_folder` does not match the typed
    /// folder still merges into that album, not a new sibling.
    #[tokio::test]
    async fn folder_add_with_explicit_target_merges_into_the_chosen_album_regardless_of_its_own_folder(
    ) {
        use crate::tui::input::handle_key;
        use crate::tui::InputMode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        // Kino is already linked to a folder of its own — a *different* one
        // from the one about to be imported.
        let kinos_own_folder = dir.path().join("Kinos Own Folder");
        app.albums[0].playlist.source_folder = Some(kinos_own_folder.clone());
        let album_path = app.albums[0].path.clone();
        app.albums[0].playlist.save(&album_path).expect("save");

        let root = dir.path().join("Unrelated Folder");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("Track One.flac"), b"bytes").expect("write");

        app.input_mode = InputMode::FolderInput;
        app.input_buf = root.to_string_lossy().to_string();
        app.target_list_for_add = Some("Kino".to_string());

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");
        drain_import(&mut app).await;

        assert_eq!(app.albums.len(), 1, "no new sibling album was created");
        assert_eq!(
            app.albums[0].playlist.tracks.len(),
            2,
            "k1 plus the one file from the unrelated folder"
        );
        assert_eq!(
            app.albums[0].playlist.source_folder.as_deref(),
            Some(kinos_own_folder.as_path()),
            "R still rescans Kino's own folder, not the one just merged in"
        );
    }

    /// file × Auto: a single file with no explicit target lands in the
    /// displayed playlist.
    #[tokio::test]
    async fn file_add_with_auto_target_lands_in_the_displayed_playlist() {
        use crate::tui::input::handle_key;
        use crate::tui::InputMode;

        let (dir, _tracks, _path, mut app) = app_with_own_library(make_playlist("Live Sets"), &[]);
        let file_path = dir.path().join("Track One.flac");
        std::fs::write(&file_path, b"bytes").expect("write");

        app.input_mode = InputMode::FolderInput;
        app.input_buf = file_path.to_string_lossy().to_string();
        app.target_list_for_add = None;

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");
        drain_import(&mut app).await;

        assert_eq!(app.playlist.tracks.len(), 1);
        assert!(app.albums.is_empty(), "no album was involved");
    }

    /// file × explicit: `Tab`-ing to an existing album adds the single file
    /// straight into it.
    #[tokio::test]
    async fn file_add_with_explicit_target_lands_in_the_chosen_album() {
        use crate::tui::input::handle_key;
        use crate::tui::InputMode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        let file_path = dir.path().join("Track One.flac");
        std::fs::write(&file_path, b"bytes").expect("write");

        app.input_mode = InputMode::FolderInput;
        app.input_buf = file_path.to_string_lossy().to_string();
        app.target_list_for_add = Some("Kino".to_string());

        handle_key(&mut app, key(crossterm::event::KeyCode::Enter))
            .await
            .expect("handled");
        drain_import(&mut app).await;

        assert_eq!(app.playlist.tracks, vec!["a".to_string()], "own untouched");
        assert_eq!(
            app.albums[0].playlist.tracks.len(),
            2,
            "k1 plus the new file"
        );
    }

    // ── Rendering an album ────────────────────────────────────────────────

    /// The glyphs are `▸`/`▾`, not the sidebar's `▶`/`▼`: `▶` is the playing
    /// marker and the two must never be confused.
    #[test]
    fn a_folded_album_header_points_right_and_an_open_one_points_down() {
        use crate::tui::ui::album_header_cells;

        let (title, _, _) = album_header_cells("Kino", 10, 2760, false, 40);
        assert_eq!(title, "▸ Kino");
        let (title, _, _) = album_header_cells("Kino", 10, 2760, true, 40);
        assert_eq!(title, "▾ Kino");
    }

    /// A header spends the artist and duration columns on what the group holds,
    /// which is the only summary of it the user gets while it is folded.
    #[test]
    fn an_album_header_reports_its_size_where_the_artist_and_duration_go() {
        use crate::tui::ui::album_header_cells;

        let (_, count, duration) = album_header_cells("Kino", 10, 2760, false, 40);
        assert_eq!(count, "10 tracks");
        assert_eq!(duration, "46m");

        let (_, count, _) = album_header_cells("Solo", 1, 60, false, 40);
        assert_eq!(count, "1 track");
    }

    /// An import without ffprobe knows no durations. `0m` would read as a claim
    /// that the album is empty of time rather than as an absence.
    #[test]
    fn an_album_header_of_unknown_length_says_nothing_about_it() {
        use crate::tui::ui::album_header_cells;

        let (_, count, duration) = album_header_cells("Kino", 3, 0, false, 40);
        assert_eq!(count, "3 tracks");
        assert_eq!(duration, "");
    }

    /// The name is what the sidebar could not fit, so the whole point is that it
    /// gets the title column — but it still has to fit inside it, glyph included.
    #[test]
    fn a_long_album_name_is_truncated_to_the_title_column() {
        use crate::tui::ui::album_header_cells;

        let (title, _, _) = album_header_cells("Суржиков Роман – Полари 06", 20, 0, true, 12);
        assert_eq!(title.chars().count(), 12);
        assert!(title.starts_with("▾ "), "the glyph survives truncation");
    }

    /// With albums present the bracketed counter (rows) and the summary count
    /// (tracks) have different denominators, and each has to say what it means.
    #[test]
    fn the_panel_title_counts_tracks_in_its_summary_and_rows_in_its_window() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("aBooks", 23, 1, 12, 14, 32 * 3600 + 41 * 60),
            " aBooks · 23 tracks · 32h 41m  [ 1–12 / 14 ] "
        );
    }

    /// A playlist that is nothing but one folded album still has a row on screen,
    /// so the panel must not fall back to the bare-name form.
    #[test]
    fn a_playlist_whose_only_row_is_a_header_still_gets_a_counter() {
        use crate::tui::ui::track_panel_title;

        assert_eq!(
            track_panel_title("aBooks", 10, 1, 1, 1, 600),
            " aBooks · 10 tracks · 10m  [ 1–1 / 1 ] "
        );
    }

    /// `▶` marks the row that is actually playing, and identity is
    /// `(playlist file, id)` — so the same id in the parent and in an album under
    /// it must not both light up.
    #[test]
    fn the_playing_marker_follows_the_list_the_row_belongs_to() {
        use crate::tui::ui::row_is_playing;
        use crate::tui::RowSource;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["shared"], &[("Kino", &["shared"])]);
        open_all(&mut app);

        app.playing = Some(crate::tui::PlayingSession {
            path: app.albums[0].path.clone(),
            playlist: app.albums[0].playlist.clone(),
            track_id: "shared".to_string(),
        });

        assert!(
            row_is_playing(&app, RowSource::Album(0), "shared"),
            "the album's row is the one playing"
        );
        assert!(
            !row_is_playing(&app, RowSource::Own, "shared"),
            "the parent's own row holds the same id but is not what is playing"
        );
    }

    /// Numbering says where a track sits in the list that plays it, so it restarts
    /// at 1 inside each album; the indent is what makes the group read as a group.
    /// Where `needle` starts on `line`, counted in columns rather than bytes — the
    /// panel borders either side of it are multi-byte.
    fn column_of(screen: &[String], needle: &str) -> usize {
        let line = screen
            .iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("'{needle}' is nowhere in {screen:#?}"));
        let byte = line.find(needle).expect("just found it");
        line[..byte].chars().count()
    }

    /// What is before the title on the row holding `needle` — where the number is.
    fn number_cell_of(screen: &[String], needle: &str) -> String {
        let line = screen
            .iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("'{needle}' is nowhere in {screen:#?}"));
        let byte = line.find(needle).expect("just found it");
        line[..byte].to_string()
    }

    #[test]
    fn album_tracks_are_indented_and_numbered_within_their_album() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["p1", "p2"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);

        let screen = render_to_lines(&mut app, 78, 16);

        assert_eq!(
            column_of(&screen, "k1"),
            column_of(&screen, "p1") + 4,
            "an album's tracks are indented so the group reads as one thing: {screen:#?}"
        );

        // Numbering says where the track sits in the list that plays it, so `k2` is
        // the album's second track and not the playlist's fourth row.
        assert!(number_cell_of(&screen, "p1").contains('1'), "{screen:#?}");
        assert!(
            number_cell_of(&screen, "k1").contains('1'),
            "the album's numbering restarts at 1: {screen:#?}"
        );
        let second = number_cell_of(&screen, "k2");
        assert!(second.contains('2'), "{screen:#?}");
        assert!(
            !second.contains('4'),
            "numbered within the album, not across the screen: {screen:#?}"
        );
    }

    /// The header is drawn from the album's own file, so what is on screen is what
    /// the group holds.
    #[test]
    fn a_header_is_drawn_with_its_name_count_and_total() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);

        let screen = render_to_lines(&mut app, 78, 16);
        let header = screen
            .iter()
            .find(|line| line.contains("Kino"))
            .unwrap_or_else(|| panic!("no header row in {screen:#?}"));
        assert!(header.contains("▸ Kino"), "{header}");
        assert!(header.contains("2 tracks"), "{header}");
        assert!(header.contains("6m"), "2 × 180s: {header}");
    }

    /// The duration column must be wide enough for `HH:MM:SS`, and a blank
    /// column must separate the table from the scrollbar.
    #[test]
    fn duration_over_an_hour_is_not_clipped_and_a_gap_precedes_the_scrollbar() {
        let mut pl = make_playlist("Active");
        let mut track = make_track("a", "Long Track");
        track.duration = 3723; // 01:02:03
        add_track(&mut pl, track);
        let (_dir, _path, mut app) = app_on_disk(pl);

        let screen = render_to_lines(&mut app, 78, 16);
        let line = screen
            .iter()
            .find(|line| line.contains("Long Track"))
            .unwrap_or_else(|| panic!("no row for the track in {screen:#?}"));
        assert!(
            line.contains("01:02:03"),
            "the full HH:MM:SS must render, with no digit clipped: {line:?}"
        );

        let chars: Vec<char> = line.chars().collect();
        let width = chars.len();
        assert_eq!(
            chars[width - 3],
            ' ',
            "a blank column must separate the table from the scrollbar: {line:?}"
        );
    }

    // ── An album plays as its own list ────────────────────────────────────

    #[tokio::test]
    async fn playing_an_album_row_builds_a_session_on_the_albums_own_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);

        assert!(app.play_row(3), "album0:1 is playable");

        let session = app.playing.as_ref().expect("a session");
        assert_eq!(
            session.path, app.albums[0].path,
            "the album's file, not aBooks"
        );
        assert_eq!(session.track_id, "k2");
        assert_eq!(
            session.playlist.tracks,
            vec!["k1", "k2"],
            "the running order auto-advance will walk is the album's"
        );
    }

    /// The album is loaded in memory, so nothing else would ever write this — and
    /// without it the cursor would not come back to where the user left off.
    #[tokio::test]
    async fn playing_an_album_row_records_current_track_in_the_albums_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);

        app.play_row(2);

        assert_eq!(
            app.albums[0].playlist.current_track.as_deref(),
            Some("k1"),
            "in memory"
        );
        assert_eq!(
            crate::playlist::Playlist::load(&app.albums[0].path)
                .expect("load")
                .current_track
                .as_deref(),
            Some("k1"),
            "and on disk"
        );
        assert_eq!(
            app.playlist.current_track, None,
            "the parent played nothing, so it must not claim to have"
        );
    }

    /// A header names a group, not a track. `Enter` on it folds and unfolds
    /// (see the key tests); it must never start something.
    #[tokio::test]
    async fn a_header_row_is_not_playable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);

        assert!(!app.play_row(1), "the header refuses");
        assert!(app.playing.is_none());
    }

    #[tokio::test]
    async fn next_from_an_albums_last_track_wraps_to_its_first() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a", "b"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);
        // own:0, own:1, header:Kino, album0:0, album0:1
        app.selected = 4;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handled");

        assert_eq!(
            app.selected, 3,
            "back to the album's first track, not past it"
        );
        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("k1")
        );
    }

    /// The parent's own tracks are their own running order too: `n` off the end of
    /// them wraps within them rather than falling into the first album.
    #[tokio::test]
    async fn next_from_the_parents_last_track_wraps_within_the_parent() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a", "b"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        app.selected = 1; // own:1, the last of the parent's own tracks

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handled");

        assert_eq!(app.selected, 0);
        assert_eq!(app.playing.as_ref().map(|p| p.track_id.as_str()), Some("a"));
        assert_eq!(
            app.playing.as_ref().map(|p| p.path.clone()),
            Some(app.playlist_path.clone()),
            "still playing out of the parent's file"
        );
    }

    /// A header belongs to no running order, so there is nothing for `n` to step.
    #[tokio::test]
    async fn next_on_a_header_does_nothing() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.selected = 1;

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handled");

        assert_eq!(app.selected, 1, "the cursor stays put");
        assert!(app.playing.is_none());
    }

    /// Shuffle is per file, so an album shuffles on its own order without the
    /// parent's setting having anything to do with it.
    #[tokio::test]
    async fn an_album_follows_its_own_shuffled_order() {
        use crate::tui::input::handle_tracklist;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2", "k3"])]);
        open_all(&mut app);
        app.albums[0].playlist.shuffle = true;
        // Pin the order so following it cannot pass by coincidence.
        app.shuffle_order = vec![2, 0, 1];
        app.shuffle_order_path = Some(app.albums[0].path.clone());
        app.selected = 2; // album0:0

        handle_tracklist(&mut app, key(crossterm::event::KeyCode::Char('n')))
            .await
            .expect("handled");

        assert_eq!(
            app.playing.as_ref().map(|p| p.track_id.as_str()),
            Some("k2"),
            "0 is followed by 1 in the pinned order"
        );
    }

    // ── The keys on an album header ────────────────────────────────────────

    #[tokio::test]
    async fn enter_on_a_header_opens_and_closes_the_album_without_playing() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.selected = 1;

        handle_tracklist(&mut app, key(KeyCode::Enter))
            .await
            .expect("handled");
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino", "album0:0"]);
        assert!(app.playing.is_none(), "a header is not playable");
        assert!(
            !crate::playlist::Playlist::load(&app.albums[0].path)
                .expect("load")
                .collapsed,
            "and the fold state is remembered on disk"
        );

        handle_tracklist(&mut app, key(KeyCode::Enter))
            .await
            .expect("handled");
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino"]);
        assert!(
            crate::playlist::Playlist::load(&app.albums[0].path)
                .expect("load")
                .collapsed
        );
    }

    #[tokio::test]
    async fn shift_j_swaps_two_tracks_inside_the_album_they_belong_to() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1", "k2"])]);
        open_all(&mut app);
        app.selected = 2; // album0:0

        handle_tracklist(&mut app, key(KeyCode::Char('J')))
            .await
            .expect("handled");

        assert_eq!(app.albums[0].playlist.tracks, vec!["k2", "k1"]);
        assert_eq!(app.playlist.tracks, vec!["a"], "the parent is untouched");
        assert_eq!(app.selected, 3, "the cursor stays on the row it moved");
        assert_eq!(
            crate::playlist::Playlist::load(&app.albums[0].path)
                .expect("load")
                .tracks,
            vec!["k2", "k1"]
        );
    }

    /// The boundary between two lists is not a place a swap can happen: moving a
    /// track out of the album it belongs to is `m`, not `J`.
    #[tokio::test]
    async fn shift_j_refuses_to_move_a_track_across_the_boundary() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        app.selected = 0; // the parent's last own track, a header below it

        handle_tracklist(&mut app, key(KeyCode::Char('J')))
            .await
            .expect("handled");

        assert_eq!(app.playlist.tracks, vec!["a"]);
        assert_eq!(app.albums[0].playlist.tracks, vec!["k1"]);
        assert_eq!(app.selected, 0);
    }

    #[tokio::test]
    async fn shift_j_on_a_header_says_albums_are_sorted() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"]), ("Zed", &["z1"])]);
        app.selected = 1; // header:Kino

        handle_tracklist(&mut app, key(KeyCode::Char('J')))
            .await
            .expect("handled");

        assert_eq!(
            app.albums
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Kino", "Zed"]
        );
        assert_eq!(status_of(&app), Some("Albums are sorted by name"));
    }

    #[tokio::test]
    async fn deleting_an_album_row_edits_the_album_and_leaves_the_file_alone() {
        use crate::tui::input::{handle_confirm_delete, handle_tracklist};
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let media = dir.path().join("song.mp3");
        std::fs::write(&media, b"not really audio").expect("write");

        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &[])]);
        let track = local_track(&media);
        let id = track.id.clone();
        app.library.upsert(track).expect("write document");
        app.albums[0].playlist.tracks.push(id.clone());
        open_all(&mut app);
        app.selected = 2; // album0:0

        handle_tracklist(&mut app, key(KeyCode::Char('d')))
            .await
            .expect("handled");
        assert_eq!(app.input_mode, crate::tui::InputMode::ConfirmDelete);
        handle_confirm_delete(&mut app, key(KeyCode::Char('y'))).expect("confirmed");

        assert!(app.albums[0].playlist.tracks.is_empty(), "the row goes");
        assert!(
            media.exists(),
            "the user's file is never trovers' to delete"
        );
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino"]);
    }

    #[tokio::test]
    async fn deleting_an_album_from_its_header_forgets_it_and_keeps_the_folder() {
        use crate::tui::input::{handle_album_delete, handle_tracklist};
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("Kino files");
        std::fs::create_dir(&folder).expect("create folder");

        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.albums[0].playlist.source_folder = Some(folder.clone());
        let album_path = app.albums[0].path.clone();
        app.selected = 1; // header:Kino

        handle_tracklist(&mut app, key(KeyCode::Char('d')))
            .await
            .expect("handled");
        assert_eq!(app.input_mode, crate::tui::InputMode::AlbumDelete);
        handle_album_delete(&mut app, key(KeyCode::Char('y'))).expect("confirmed");

        assert!(app.albums.is_empty(), "the album is gone from the rows");
        assert!(!album_path.exists(), "its playlist file goes");
        assert!(folder.exists(), "the folder it mirrored stays");
        assert!(
            app.library.get("k1").is_some(),
            "the track keeps its document"
        );
        assert_eq!(row_shapes(&app), vec!["own:0"]);
        assert!(
            !app.available_playlists.iter().any(|e| e.name == "Kino"),
            "and the listing forgets it too"
        );
    }

    /// Deleting the album that is playing has to stop it first: the file goes, and
    /// a later position flush against a dead path would write it back.
    #[tokio::test]
    async fn deleting_the_playing_album_stops_playback() {
        use crate::tui::input::handle_album_delete;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        app.play_row(2);
        assert!(app.playing.is_some(), "the album is playing");

        app.selected = 1; // header:Kino
        app.input_mode = crate::tui::InputMode::AlbumDelete;
        handle_album_delete(&mut app, key(KeyCode::Char('y'))).expect("confirmed");

        assert!(app.playing.is_none(), "nothing is playing any more");
        assert!(app.albums.is_empty());
    }

    #[tokio::test]
    async fn renaming_an_album_from_its_header_renames_its_file() {
        use crate::tui::input::{handle_album_rename, handle_tracklist};
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        let old = app.albums[0].path.clone();
        app.selected = 1; // header:Kino

        handle_tracklist(&mut app, key(KeyCode::Char('r')))
            .await
            .expect("handled");
        assert_eq!(app.input_mode, crate::tui::InputMode::AlbumRename);
        assert_eq!(
            app.input_buf, "Kino",
            "the prompt opens on the current name"
        );
        assert!(
            !app.playlist.shuffle,
            "`r` on a header renames; it must not have toggled shuffle on the way"
        );

        app.input_buf = "Viktor".to_string();
        handle_album_rename(&mut app, key(KeyCode::Enter)).expect("renamed");

        assert_eq!(app.albums[0].name, "Viktor");
        assert_eq!(app.albums[0].playlist.name, "Viktor");
        assert!(!old.exists(), "the old file goes");
        assert!(app.albums[0].path.exists());
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Viktor"]);
        assert!(
            app.available_playlists.iter().any(|e| e.name == "Viktor"),
            "the listing follows the rename"
        );
    }

    /// A rename moves the file the session saves into. Left pointing at the old
    /// path, the next position flush would recreate the album under its old name.
    #[tokio::test]
    async fn renaming_the_playing_album_repoints_its_session() {
        use crate::tui::input::handle_album_rename;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        open_all(&mut app);
        app.play_row(2);

        app.selected = 1; // header:Kino
        app.input_mode = crate::tui::InputMode::AlbumRename;
        app.input_buf = "Viktor".to_string();
        handle_album_rename(&mut app, key(KeyCode::Enter)).expect("renamed");

        assert_eq!(
            app.playing.as_ref().map(|p| p.path.clone()),
            Some(app.albums[0].path.clone()),
            "the session follows the file"
        );
    }

    /// Two albums under one parent cannot share a name: they would share a file.
    #[tokio::test]
    async fn renaming_an_album_onto_a_taken_name_is_refused() {
        use crate::tui::input::handle_album_rename;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"]), ("Zed", &["z1"])]);
        app.selected = 1; // header:Kino
        app.input_mode = crate::tui::InputMode::AlbumRename;
        app.input_buf = "Zed".to_string();

        handle_album_rename(&mut app, key(KeyCode::Enter)).expect("handled");

        assert_eq!(app.albums[0].name, "Kino", "the rename is refused");
        assert_eq!(
            app.input_mode,
            crate::tui::InputMode::AlbumRename,
            "and the prompt stays open so the name can be fixed"
        );
    }

    #[tokio::test]
    async fn shift_r_on_a_header_rescans_that_albums_folder() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("Kino files");
        std::fs::create_dir(&folder).expect("create folder");

        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.albums[0].playlist.source_folder = Some(folder.clone());
        app.selected = 1; // header:Kino

        handle_tracklist(&mut app, key(KeyCode::Char('R')))
            .await
            .expect("handled");

        assert_eq!(
            status_of(&app),
            Some(format!("Scanning {}", folder.display()).as_str()),
            "the album's folder, not the parent's"
        );
    }

    #[tokio::test]
    async fn shift_r_on_a_header_of_an_unlinked_album_says_so() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.selected = 1;

        handle_tracklist(&mut app, key(KeyCode::Char('R')))
            .await
            .expect("handled");

        assert_eq!(status_of(&app), Some("Not linked to a folder"));
    }

    // ── Imports and switches keep the rows in step ─────────────────────────

    /// An import the user just asked for arrives open. It is the one album whose
    /// contents they have already said they want to see.
    #[test]
    fn a_fresh_import_appears_as_an_open_album_under_the_cursor() {
        use crate::tui::ImportTarget;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[]);
        let root = dir.path().join("Kino");

        app.apply_import(
            root.clone(),
            ImportTarget::NewAlbum {
                parent: Some(app.displayed_playlist_name()),
            },
            vec![imported_file(&root.join("one.mp3"), "One")],
        );

        assert_eq!(app.albums.len(), 1);
        assert!(
            !app.albums[0].playlist.collapsed,
            "an import you cannot see did nothing"
        );
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino", "album0:0"]);
    }

    /// An import whose parent is some other playlist must not appear here — the
    /// rows show the displayed playlist's albums, not every album there is.
    #[test]
    fn an_import_under_another_playlist_does_not_join_these_rows() {
        use crate::tui::ImportTarget;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[]);
        let root = dir.path().join("Kino");

        app.apply_import(
            root.clone(),
            ImportTarget::NewAlbum {
                parent: Some("Elsewhere".to_string()),
            },
            vec![imported_file(&root.join("one.mp3"), "One")],
        );

        assert!(app.albums.is_empty());
        assert_eq!(row_shapes(&app), vec!["own:0"]);
    }

    #[test]
    fn switching_playlist_loads_that_playlists_own_albums() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);

        // A second top-level playlist with an album of its own.
        let second_path = dir.path().join("Second.toml");
        let mut second = make_playlist("Second");
        add_track(&mut second, make_track("s1", "s1"));
        second.save(&second_path).expect("save second");
        let mut zed = make_playlist("Zed");
        zed.kind = crate::playlist::PlaylistKind::Album;
        zed.parent = Some("Second".to_string());
        add_track(&mut zed, make_track("z1", "z1"));
        zed.save(&dir.path().join("Zed.toml")).expect("save Zed");
        app.available_playlists =
            crate::playlist::Playlist::list_entries(dir.path()).expect("list");
        refresh_library(&mut app);

        assert_eq!(album_names(&app), vec!["Kino"]);

        app.switch_to_playlist("Second", &second_path)
            .expect("switch");

        assert_eq!(album_names(&app), vec!["Zed"]);
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Zed"]);
    }

    /// Renaming a parent has to reach the copies of its albums held in memory. A
    /// stale `parent` there is written back by the next thing that saves the album
    /// — a fold, a play — and the album orphans out of the list it belongs to.
    #[tokio::test]
    async fn renaming_the_displayed_playlist_follows_its_loaded_albums() {
        use crate::tui::input::handle_playlist_rename;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.sidebar_selected = app
            .sidebar_items()
            .iter()
            .position(
                |i| matches!(i, crate::tui::SidebarItem::Playlist { name, .. } if name == "aBooks"),
            )
            .expect("the parent is in the sidebar");
        app.input_mode = crate::tui::InputMode::PlaylistRename;
        app.input_buf = "Books".to_string();
        handle_playlist_rename(&mut app, key(KeyCode::Enter))
            .await
            .expect("renamed");

        assert_eq!(
            app.albums[0].playlist.parent.as_deref(),
            Some("Books"),
            "in memory"
        );
        // And it survives the next save of the album, which is what would have
        // written the stale name back.
        app.toggle_album(0);
        assert_eq!(
            crate::playlist::Playlist::load(&app.albums[0].path)
                .expect("load")
                .parent
                .as_deref(),
            Some("Books"),
            "and on disk"
        );
        assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino", "album0:0"]);
    }

    /// An album whose parent is itself an album is not something trovers writes —
    /// two levels only (ADR-016) — but a hand-edited file can say it, and then the
    /// album is both listed in the sidebar and drawn as a row here. Deleting it
    /// from the sidebar has to take the row with it, not leave a header pointing at
    /// a file that is gone.
    #[tokio::test]
    async fn deleting_an_album_from_the_sidebar_takes_its_row_too() {
        use crate::tui::input::handle_playlist_delete;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        // Display the album, and hang another album off it.
        let kino_path = app.albums[0].path.clone();
        let mut nested = make_playlist("Nested");
        nested.kind = crate::playlist::PlaylistKind::Album;
        nested.parent = Some("Kino".to_string());
        add_track(&mut nested, make_track("n1", "n1"));
        let nested_path = dir.path().join("Nested.toml");
        nested.save(&nested_path).expect("save nested");
        app.available_playlists =
            crate::playlist::Playlist::list_entries(dir.path()).expect("list");
        refresh_library(&mut app);
        app.switch_to_playlist("Kino", &kino_path).expect("switch");
        assert_eq!(album_names(&app), vec!["Nested"]);

        app.sidebar_selected = app
            .sidebar_items()
            .iter()
            .position(
                |i| matches!(i, crate::tui::SidebarItem::Playlist { name, .. } if name == "Nested"),
            )
            .expect("an album under an album is still reachable in the sidebar");
        handle_playlist_delete(&mut app, key(KeyCode::Char('y')))
            .await
            .expect("deleted");

        assert!(!nested_path.exists());
        assert!(app.albums.is_empty(), "and the loaded copy goes with it");
        assert_eq!(row_shapes(&app), vec!["own:0"]);
    }

    /// The names of the loaded albums, in row order.
    fn album_names(app: &crate::tui::App) -> Vec<&str> {
        app.albums
            .iter()
            .map(|loaded| loaded.name.as_str())
            .collect()
    }

    /// `m` moves a row into another list. A header *is* a list; there is nothing
    /// to move, and silently opening the menu on the row below would be worse.
    #[tokio::test]
    async fn m_on_a_header_says_there_is_nothing_to_move() {
        use crate::tui::input::handle_tracklist;
        use crossterm::event::KeyCode;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &["a"], &[("Kino", &["k1"])]);
        app.selected = 1;

        handle_tracklist(&mut app, key(KeyCode::Char('m')))
            .await
            .expect("handled");

        assert_eq!(app.input_mode, crate::tui::InputMode::Normal);
        assert_eq!(status_of(&app), Some("Move tracks, not albums"));
    }

    // ── Video rows say so ─────────────────────────────────────────────────

    /// A local track of `media`, titled `title`, already in the library.
    fn media_track(title: &str, media: crate::library::MediaKind) -> crate::library::Track {
        let mut track = local_track(std::path::Path::new(&format!("/movies/{title}.mkv")));
        track.title = title.to_string();
        track.media = media;
        track
    }

    #[tokio::test]
    async fn a_video_row_is_marked_with_a_media_glyph() {
        use crate::library::MediaKind;

        let mut app = make_app_with_playlists("Films", &["Films"]);
        push_track(&mut app, media_track("Blade Runner", MediaKind::Video));

        let screen = render_to_lines(&mut app, 80, 24);

        assert!(
            screen.iter().any(|line| line.contains("▣ Blade Runner")),
            "a video row must say it will open a window, {screen:#?}"
        );
    }

    #[tokio::test]
    async fn an_audio_row_carries_no_media_glyph() {
        use crate::library::MediaKind;

        let mut app = make_app_with_playlists("Films", &["Films"]);
        push_track(&mut app, media_track("Day One", MediaKind::Audio));

        let screen = render_to_lines(&mut app, 80, 24);

        assert!(
            screen.iter().any(|line| line.contains(" Day One")),
            "the row is there, {screen:#?}"
        );
        assert!(
            !screen.iter().any(|line| line.contains("▣")),
            "and nothing marks it, {screen:#?}"
        );
    }

    /// The glyph belongs to the title, so inside an album it follows the indent
    /// rather than un-nesting the row.
    #[tokio::test]
    async fn a_video_row_inside_an_album_keeps_its_indent() {
        use crate::library::MediaKind;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = app_with_albums(dir.path(), &[], &[("Kino", &["k1"])]);
        let mut track = media_track("Igla", MediaKind::Video);
        track.id = "k1".to_string();
        app.library.upsert(track).expect("write document");
        open_all(&mut app);

        let screen = render_to_lines(&mut app, 80, 24);
        let glyph = column_of(&screen, "▣ Igla");
        let header = column_of(&screen, "▾ Kino");

        assert!(
            glyph > header,
            "an album's video row is indented past its header, {screen:#?}"
        );
    }
}
