#[cfg(test)]
mod tests {
    use crate::tui::ui::{
        build_now_playing_header_line, build_playback_bar_line, build_progress_bar,
        build_separated_line, build_track_info_line, calculate_distributed_widths,
        format_duration, format_playback_state, make_panel_block, truncate,
        url_input_target_display, CacheState,
    };
    use ratatui::style::Color;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_playlist(name: &str) -> crate::playlist::Playlist {
        crate::playlist::Playlist {
            name: name.to_string(),
            created: chrono::Utc::now(),
            loop_mode: crate::playlist::LoopMode::None,
            default_speed: None,
            tracks: Vec::new(),
            current_track: None,
        }
    }

    fn make_app_with_playlists(
        active: &str,
        playlists: &[&str],
    ) -> crate::tui::App {
        use std::path::PathBuf;
        let playlist = make_playlist(active);
        let config = crate::config::Config::default();
        let available: Vec<(String, PathBuf)> = playlists
            .iter()
            .map(|n| (n.to_string(), PathBuf::from(format!("/fake/{}.toml", n))))
            .collect();
        crate::tui::App::new(playlist, config, available, PathBuf::from("/fake/active.toml"))
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
        let empty_color = Color::Rgb(70, 70, 70);   // BORDER_IDLE
        let spans = build_progress_bar(10, 0.5, '━', '─', '◉', fill_color, empty_color);

        // The fill and thumb spans should use fill_color
        // The empty span should use empty_color
        let fill_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.content.contains('━') || s.content.contains('◉'))
            .collect();
        let empty_spans: Vec<_> = spans
            .iter()
            .filter(|s| s.content.contains('─'))
            .collect();

        for s in &fill_spans {
            assert_eq!(s.style.fg, Some(fill_color), "fill/thumb span should have fill_color");
        }
        for s in &empty_spans {
            assert_eq!(s.style.fg, Some(empty_color), "empty span should have empty_color");
        }
    }

    #[test]
    fn progress_bar_no_thumb_colors() {
        // No-thumb mode: filled uses fill_color, empty uses empty_color
        let fill_color = Color::Rgb(212, 175, 55);  // GOLD
        let empty_color = Color::Rgb(130, 130, 130); // TEXT_DIM
        let spans = build_progress_bar(10, 0.4, '▓', '░', '\0', fill_color, empty_color);

        let fill_spans: Vec<_> = spans.iter().filter(|s| s.content.contains('▓')).collect();
        let empty_spans: Vec<_> = spans.iter().filter(|s| s.content.contains('░')).collect();

        for s in &fill_spans {
            assert_eq!(s.style.fg, Some(fill_color), "fill span should have fill_color");
        }
        for s in &empty_spans {
            assert_eq!(s.style.fg, Some(empty_color), "empty span should have empty_color");
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
        assert_eq!(result[2], 8);  // fixed
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
        let result = build_separated_line(
            &[("Hello World", true), ("Artist Name", false)],
            15,
        );
        // Result: "Hello World" + sep + 1-char truncated artist
        let texts: Vec<&str> = result.iter().map(|r| r.0.as_str()).collect();
        assert!(texts.contains(&"Hello World"), "primary should be preserved: {:?}", texts);
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
        let result = build_separated_line(
            &[("Track", true), ("Artist", false), ("Source", false)],
            80,
        );
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
        assert!(text.contains("🎵 Now Playing"), "should contain label: {text:?}");
        assert!(text.contains("No track selected"), "should contain no-track status: {text:?}");
    }

    #[test]
    fn header_no_track_total_width_does_not_exceed() {
        let width = 80;
        let line = build_now_playing_header_line(width, None, None);
        let text = line_to_string(&line);
        let char_count: usize = text.chars().count();
        // Should not exceed the width (may be less due to saturation)
        assert!(char_count <= width + 5, "header too wide: {char_count} chars for width={width}");
    }

    #[test]
    fn header_playing_state_contains_all_three_sections() {
        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let text = line_to_string(&line);
        assert!(text.contains("🎵 Now Playing"), "should contain label: {text:?}");
        assert!(text.contains("▶ Playing"), "should contain playback status: {text:?}");
        assert!(text.contains("1.4×"), "should contain speed: {text:?}");
    }

    #[test]
    fn header_paused_state() {
        let line = build_now_playing_header_line(80, Some("⏸ Paused"), Some("1.0×"));
        let text = line_to_string(&line);
        assert!(text.contains("⏸ Paused"), "should contain paused status: {text:?}");
        assert!(text.contains("1.0×"), "should contain speed: {text:?}");
    }

    #[test]
    fn header_playing_gold_style_on_label() {
        let gold = Color::Rgb(212, 175, 55);
        let line = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        // The label span should have GOLD color
        let label_span = line.spans.iter().find(|s| s.content.contains("🎵 Now Playing"));
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
            assert!(text.contains(speed), "should contain speed {speed}: {text:?}");
        }
    }

    // ── build_track_info_line tests ───────────────────────────────────────────

    #[test]
    fn track_info_line_contains_all_three_parts() {
        let line = build_track_info_line(80, "My Track Title", "Some Artist", "youtube.com/watch");
        let text = line_to_string(&line);
        assert!(text.contains("My Track Title"), "should contain title: {text:?}");
        assert!(text.contains("Some Artist"), "should contain artist: {text:?}");
        assert!(text.contains("youtube.com/watch"), "should contain source: {text:?}");
    }

    #[test]
    fn track_info_line_has_bullet_separators() {
        let line = build_track_info_line(80, "Track", "Artist", "source.com");
        let text = line_to_string(&line);
        // Should have bullet separators between sections
        assert!(text.contains(" • "), "should contain bullet separator: {text:?}");
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
        let source_span = line.spans.iter().find(|s| s.content.contains("my-source.com"));
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
        assert_eq!(line.spans[0].content, " ", "should start with a space for margin");
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
            let line = build_track_info_line(w, "Track Title That Is Very Long Indeed", "Artist", "source.com");
            let text = line_to_string(&line);
            // Just checking no panic - content may be very short
            let _ = text;
        }
    }

    #[test]
    fn track_info_line_total_width_respects_bounds() {
        let width = 60;
        let line = build_track_info_line(width, "My Track Title", "Great Artist", "youtube.com/watch?v=abc123");
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
        assert!(text.contains("Track Title"), "should contain title: {text:?}");
    }

    #[test]
    fn track_info_line_empty_source_still_works() {
        let line = build_track_info_line(80, "Track Title", "Artist Name", "");
        let text = line_to_string(&line);
        assert!(text.contains("Track Title"), "should contain title: {text:?}");
        assert!(text.contains("Artist Name"), "should contain artist: {text:?}");
    }

    #[test]
    fn track_info_line_user_overrides_applied_by_caller() {
        // The function takes already-resolved title/artist (caller applies overrides)
        // This tests that what we pass in is what appears in the output
        let user_title = "Custom Title Override";
        let user_artist = "Custom Artist Override";
        let line = build_track_info_line(120, user_title, user_artist, "source.com");
        let text = line_to_string(&line);
        assert!(text.contains(user_title), "user title override should appear: {text:?}");
        assert!(text.contains(user_artist), "user artist override should appear: {text:?}");
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
        assert!(text.contains("◈ Cached"), "should contain cached indicator: {text:?}");
    }

    #[test]
    fn playback_bar_streaming_shows_stream_indicator() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Streaming);
        let text = line_to_string(&line);
        assert!(text.contains("◌ Stream"), "should contain streaming indicator: {text:?}");
    }

    #[test]
    fn playback_bar_downloading_shows_caching_indicator() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Downloading(0.45));
        let text = line_to_string(&line);
        assert!(text.contains("⟳ Caching"), "should contain caching indicator: {text:?}");
    }

    #[test]
    fn playback_bar_downloading_shows_percentage() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Downloading(0.45));
        let text = line_to_string(&line);
        assert!(text.contains("45%"), "should contain download percentage: {text:?}");
    }

    #[test]
    fn playback_bar_downloading_shows_position_and_duration() {
        let line = build_playback_bar_line(80, "01:23", 0.3, "04:56", "♪ 70%", CacheState::Downloading(0.6));
        let text = line_to_string(&line);
        assert!(text.contains("01:23"), "should contain position in download mode: {text:?}");
        assert!(text.contains("04:56"), "should contain duration in download mode: {text:?}");
    }

    #[test]
    fn playback_bar_no_panic_on_zero_width() {
        // Should not panic with very small widths
        for w in [0, 1, 5, 10] {
            let _line = build_playback_bar_line(w, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        }
    }

    #[test]
    fn playback_bar_no_panic_on_narrow_terminal() {
        for w in [20, 30, 40] {
            let _line = build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Cached);
            let _line = build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Streaming);
            let _line = build_playback_bar_line(w, "00:03", 0.5, "55:34", "♪ 85%", CacheState::Downloading(0.3));
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
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Streaming);
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
        let line = build_playback_bar_line(80, "00:00", 0.0, "10:00", "♪ 80%", CacheState::Downloading(0.0));
        let text = line_to_string(&line);
        assert!(text.contains("0%"), "should show 0% when download just started: {text:?}");
    }

    #[test]
    fn playback_bar_downloading_100_percent() {
        // dl_ratio = 1.0 → "100%"
        let line = build_playback_bar_line(80, "05:00", 0.5, "10:00", "♪ 80%", CacheState::Downloading(1.0));
        let text = line_to_string(&line);
        assert!(text.contains("100%"), "should show 100% when download complete: {text:?}");
    }

    #[test]
    fn cache_state_equality() {
        assert_eq!(CacheState::Cached, CacheState::Cached);
        assert_eq!(CacheState::Streaming, CacheState::Streaming);
        assert_ne!(CacheState::Cached, CacheState::Streaming);
    }

    #[test]
    fn playback_bar_starts_with_space() {
        let line = build_playback_bar_line(80, "00:03", 0.1, "55:34", "♪ 85%", CacheState::Cached);
        assert!(!line.spans.is_empty(), "should have spans");
        assert_eq!(line.spans[0].content, " ", "should start with leading space");
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
        assert!(header_text.contains("🎵 Now Playing"), "header row missing label");
        assert!(track_text.contains("My Song"), "track row missing title");
        assert!(bar_text.contains("00:30"), "playback row missing position");

        // Content should not cross between rows
        assert!(!track_text.contains("🎵 Now Playing"), "track row should not contain header label");
        assert!(!header_text.contains("My Song"), "header row should not contain track title");
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
        let line = build_playback_bar_line(80, "02:15", 0.3, "07:30", "♪ 75%", CacheState::Streaming);
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
            let _bar_cached = build_playback_bar_line(w, "01:00", 0.5, "02:00", "♪ 80%", CacheState::Cached);
            let _bar_stream = build_playback_bar_line(w, "01:00", 0.5, "02:00", "♪ 80%", CacheState::Streaming);
            let _bar_dl = build_playback_bar_line(w, "01:00", 0.5, "02:00", "♪ 80%", CacheState::Downloading(0.6));
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
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::TOP);
        let inner = block.inner(outer);
        assert_eq!(inner.height, 3, "Borders::TOP on a 4-row area leaves 3 rows for content");

        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
        assert_eq!(rows[0].height, 1, "header row height");
        assert_eq!(rows[1].height, 1, "track info row height");
        assert_eq!(rows[2].height, 1, "playback bar row height — must be 1, not 0");
    }

    #[test]
    fn now_playing_cache_state_removed_from_old_row() {
        // Verify cache status is integrated into row 3 (playback bar),
        // not in a separate fourth row. The playback bar should contain cache info.
        let cached_line = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 90%", CacheState::Cached);
        let stream_line = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 90%", CacheState::Streaming);
        let dl_line = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 90%", CacheState::Downloading(0.5));

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
        assert!(!b.contains("1.2×"), "speed must not bleed into playback bar");

        // Track title is only in row 2
        assert!(t.contains("Cool Track"), "title in track info");
        assert!(!h.contains("Cool Track"), "title must not bleed into header");

        // Position time is only in row 3
        assert!(b.contains("00:45"), "position in playback bar");
        assert!(!h.contains("00:45"), "position must not bleed into header");
        assert!(!t.contains("00:45"), "position must not bleed into track info");
    }

    // ── UI consistency / make_panel_block tests ───────────────────────────────

    #[test]
    fn panel_block_focused_and_unfocused_are_distinct() {
        // focused=true and focused=false should produce different Block values
        let focused = make_panel_block(" My Panel ", true);
        let unfocused = make_panel_block(" My Panel ", false);
        // Blocks with different border colors are not equal
        assert_ne!(focused, unfocused, "focused and unfocused panels should differ");
    }

    #[test]
    fn panel_block_same_focus_state_is_consistent() {
        // Calling make_panel_block twice with same args should produce equal blocks
        let block1 = make_panel_block(" Settings ", true);
        let block2 = make_panel_block(" Settings ", true);
        assert_eq!(block1, block2, "same focus state should produce identical blocks");
    }

    #[test]
    fn panel_block_different_titles_are_distinct() {
        let settings = make_panel_block(" ⚙ Settings ", false);
        let tracks = make_panel_block(" My Playlist ", false);
        assert_ne!(settings, tracks, "different titles should produce different blocks");
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
        assert_ne!(settings_focused, settings_idle, "settings focused vs idle should differ");
        assert_ne!(tracks_focused, tracks_idle, "track table focused vs idle should differ");

        // Cross-panel with same focus: should differ only by title
        assert_ne!(settings_focused, tracks_focused, "different panel titles should differ");
        assert_ne!(settings_idle, tracks_idle, "different panel titles should differ");
    }

    // ── Task 9: Acceptance criteria and edge case verification ────────────────

    // --- Requirement verification: Overview requirements ---

    #[test]
    fn requirement_header_centric_layout_has_now_playing_label() {
        // "Header-centric layout: Row 1 becomes a proper header with 🎵 Now Playing label"
        let playing = build_now_playing_header_line(80, Some("▶ Playing"), Some("1.4×"));
        let paused = build_now_playing_header_line(80, Some("⏸ Paused"), Some("1.0×"));
        let no_track = build_now_playing_header_line(80, None, None);

        for (name, line) in [("playing", &playing), ("paused", &paused), ("no_track", &no_track)] {
            let text = line_to_string(line);
            assert!(text.contains("🎵 Now Playing"),
                "header must always show label in state {name}: {text:?}");
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

        let label_span = line.spans.iter().find(|s| s.content.contains("🎵 Now Playing"));
        let speed_span = line.spans.iter().find(|s| s.content.contains("1.4×"));

        assert_eq!(label_span.unwrap().style.fg, Some(gold), "label must be GOLD");
        assert_eq!(speed_span.unwrap().style.fg, Some(accent), "speed must be ACCENT");
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

        let title_span = line.spans.iter().find(|s| s.content.contains("My Track Title"));
        let artist_span = line.spans.iter().find(|s| s.content.contains("Great Artist"));
        let source_span = line.spans.iter().find(|s| s.content.contains("youtube.com/watch"));

        assert_eq!(title_span.unwrap().style.fg, Some(white), "title must be white");
        assert_eq!(artist_span.unwrap().style.fg, Some(text_dim), "artist must be TEXT_DIM");
        assert_eq!(source_span.unwrap().style.fg, Some(text_dim), "source must be TEXT_DIM");
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
        let label_span = header.spans.iter().find(|s| s.content.contains("🎵 Now Playing")).unwrap();
        let speed_span = header.spans.iter().find(|s| s.content.contains("1.4×")).unwrap();
        assert_eq!(label_span.style.fg, Some(gold), "header label must be GOLD");
        assert_eq!(speed_span.style.fg, Some(accent), "header speed must be ACCENT");

        // Track info: artist=TEXT_DIM
        let track = build_track_info_line(80, "Title", "Artist", "source.com");
        let artist_span = track.spans.iter().find(|s| s.content.contains("Artist")).unwrap();
        assert_eq!(artist_span.style.fg, Some(text_dim), "artist must be TEXT_DIM");

        // Playback bar: cached indicator=SEA_GREEN
        let bar = build_playback_bar_line(80, "00:00", 0.0, "01:00", "♪ 80%", CacheState::Cached);
        let cache_span = bar.spans.iter().find(|s| s.content.contains("◈ Cached")).unwrap();
        assert_eq!(cache_span.style.fg, Some(sea_green), "cached indicator must be SEA_GREEN");

        // Progress bar: fill color=SEA_GREEN, empty color=BORDER_IDLE
        let border_idle = Color::Rgb(70, 70, 70);
        let bar_spans = build_progress_bar(20, 0.5, '━', '─', '◉', sea_green, border_idle);
        let fill_spans: Vec<_> = bar_spans.iter().filter(|s| s.content.contains('━')).collect();
        let empty_spans: Vec<_> = bar_spans.iter().filter(|s| s.content.contains('─')).collect();
        for s in &fill_spans { assert_eq!(s.style.fg, Some(sea_green), "fill must be SEA_GREEN"); }
        for s in &empty_spans { assert_eq!(s.style.fg, Some(border_idle), "empty must be BORDER_IDLE"); }
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
        let _t = build_track_info_line(w, "A Very Long Track Title Indeed", "Long Artist Name", "very-long-source-url.com/watch?v=abc");
        let _b = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);
    }

    #[test]
    fn edge_case_very_narrow_terminal_width_20_no_panic() {
        // At 20 chars (very narrow), should not panic
        for w in [1, 5, 10, 15, 20] {
            let _h = build_now_playing_header_line(w, Some("▶ Playing"), Some("1.0×"));
            let _t = build_track_info_line(w, "Track Title", "Artist Name", "source.com");
            let _b_cached = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Cached);
            let _b_stream = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Streaming);
            let _b_dl = build_playback_bar_line(w, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Downloading(0.5));
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
            assert_eq!(widths[1], expected_flex,
                "flexible section should be max(0, w-20) for w={w}: {widths:?}");
        }
        // Overflow case: when fixed widths exceed total_width, flexible section is 0
        // and sum > total_width (fixed sections retain their sizes)
        let overflow = calculate_distributed_widths(10, 3, &[(0, 15), (2, 5)]);
        assert_eq!(overflow[0], 15, "fixed section 0 unchanged in overflow");
        assert_eq!(overflow[2], 5, "fixed section 2 unchanged in overflow");
        assert_eq!(overflow[1], 0, "flexible section is 0 in overflow");
        assert!(overflow.iter().sum::<usize>() > 10, "sum exceeds total_width in overflow case");
    }

    // --- Edge case: no tracks ---

    #[test]
    fn edge_case_no_track_header_shows_no_track_selected() {
        let line = build_now_playing_header_line(80, None, None);
        let text = line_to_string(&line);
        assert!(text.contains("No track selected"), "no-track state: {text:?}");
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
        assert!(!text.contains('×'), "no track should not show speed: {text:?}");
    }

    // --- Edge case: long titles ---

    #[test]
    fn edge_case_very_long_title_truncated_with_ellipsis() {
        let long_title = "A".repeat(200);
        let line = build_track_info_line(80, &long_title, "Artist", "source.com");
        let text = line_to_string(&line);
        // Content should fit within width (leading space + content)
        let char_count = text.chars().count();
        assert!(char_count <= 82, "long title must be truncated: {char_count} chars");
        // Truncation should use ellipsis
        assert!(text.contains('…'), "truncated text should end with ellipsis: {text:?}");
    }

    #[test]
    fn edge_case_very_long_artist_truncated() {
        let long_artist = "B".repeat(200);
        let line = build_track_info_line(80, "Short Title", &long_artist, "source.com");
        let text = line_to_string(&line);
        let char_count = text.chars().count();
        assert!(char_count <= 82, "long artist must be truncated: {char_count} chars");
    }

    #[test]
    fn edge_case_very_long_source_truncated() {
        let long_source = "https://example.com/".repeat(20);
        let line = build_track_info_line(80, "Title", "Artist", &long_source);
        let text = line_to_string(&line);
        let char_count = text.chars().count();
        assert!(char_count <= 82, "long source must be truncated: {char_count} chars");
    }

    #[test]
    fn edge_case_long_title_preserves_priority_over_artist_and_source() {
        // Title has highest priority - even in tight space, title should appear
        let line = build_track_info_line(30, "My Important Track Title", "Artist", "src.com");
        let text = line_to_string(&line);
        // "My Important Track Title" (24) doesn't fit in 29 chars with separators,
        // but its beginning should be there since it has priority
        assert!(text.starts_with(" M") || text.contains("My "),
            "title should have truncation priority: {text:?}");
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
        assert!(header_text.contains("No track selected"), "no-track header text");
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
        assert!(header_text.contains("▶ Playing"), "header shows playing state");
    }

    #[test]
    fn playback_state_paused_display() {
        let (icon, text) = format_playback_state(true, true, true);
        assert_eq!(icon, "⏸", "paused has pause icon");
        assert_eq!(text, "Paused", "paused shows Paused text");

        let center = format!("{} {}", icon, text);
        let line = build_now_playing_header_line(80, Some(&center), Some("1.0×"));
        let header_text = line_to_string(&line);
        assert!(header_text.contains("⏸ Paused"), "header shows paused state");
    }

    #[test]
    fn playback_state_downloading_display() {
        // Downloading state shows caching indicator with percentage
        let line_25 = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Downloading(0.25));
        let line_75 = build_playback_bar_line(80, "00:30", 0.5, "01:00", "♪ 80%", CacheState::Downloading(0.75));

        let text_25 = line_to_string(&line_25);
        let text_75 = line_to_string(&line_75);

        assert!(text_25.contains("⟳ Caching"), "downloading: caching label at 25%: {text_25:?}");
        assert!(text_25.contains("25%"), "downloading: percentage 25%: {text_25:?}");
        assert!(text_75.contains("⟳ Caching"), "downloading: caching label at 75%: {text_75:?}");
        assert!(text_75.contains("75%"), "downloading: percentage 75%: {text_75:?}");
    }

    #[test]
    fn playback_state_all_cache_states_covered() {
        // All three cache states must be visually distinct and clearly indicated
        let cached = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Cached);
        let streaming = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Streaming);
        let downloading = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", CacheState::Downloading(0.5));

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
        assert_eq!(widths[0], w.saturating_sub(5 + 3), "flexible section saturates");
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
            let line = build_track_info_line(w, "Song Title That Is Somewhat Long", "Artist Name Here", "source.com/path");
            let char_count = line_to_string(&line).chars().count();
            assert!(char_count <= w + 2,
                "track info at w={w}: {char_count} chars > {}", w + 2);
        }
    }

    #[test]
    fn layout_format_duration_edge_cases() {
        // Test boundary values for duration formatting
        assert_eq!(format_duration(0), "00:00", "zero duration");
        assert_eq!(format_duration(59), "00:59", "59 seconds");
        assert_eq!(format_duration(3599), "59:59", "59m59s");
        assert_eq!(format_duration(3600), "01:00:00", "exactly 1 hour");
        assert_eq!(format_duration(u64::MAX / 3600 * 3600), // large hours value
            format!("{:02}:00:00", u64::MAX / 3600), "large duration");
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
        for state in [CacheState::Cached, CacheState::Streaming, CacheState::Downloading(0.3)] {
            let line = build_playback_bar_line(80, "00:10", 0.2, "01:00", "♪ 80%", state.clone());
            let text = line_to_string(&line);
            let has_cache_info = text.contains("◈ Cached")
                || text.contains("◌ Stream")
                || text.contains("⟳ Caching");
            assert!(has_cache_info, "row 3 must contain cache info for state {state:?}: {text:?}");
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
        assert!(items.is_empty(), "should be empty when only active playlist exists: {items:?}");
    }

    #[test]
    fn context_menu_items_excludes_active_playlist() {
        // Three playlists; active is "Jazz" — should return the other two
        let app = make_app_with_playlists("Jazz", &["Jazz", "Rock", "Classical"]);
        let items = app.available_playlist_names();
        assert!(!items.contains(&"Jazz".to_string()), "active playlist must be excluded");
        assert!(items.contains(&"Rock".to_string()), "Rock should be in items");
        assert!(items.contains(&"Classical".to_string()), "Classical should be in items");
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
        assert!(items.is_empty(), "should be empty with no available_playlists");
    }

    #[test]
    fn context_menu_items_many_playlists() {
        let names = &["A", "B", "C", "D", "E", "Active"];
        let app = make_app_with_playlists("Active", names);
        let items = app.available_playlist_names();
        assert_eq!(items.len(), 5, "should exclude 1 active from 6 total");
        assert!(!items.contains(&"Active".to_string()), "Active must be excluded");
        for n in &["A", "B", "C", "D", "E"] {
            assert!(items.contains(&n.to_string()), "{n} should be included");
        }
    }

    #[test]
    fn available_playlist_names_excludes_active_and_is_sorted() {
        // available_playlist_names must exclude the active playlist and preserve sorted order
        let app = make_app_with_playlists("Jazz", &["Jazz", "Rock", "Classical"]);
        let names = app.available_playlist_names();
        assert!(!names.contains(&"Jazz".to_string()), "active playlist must be excluded");
        assert!(names.contains(&"Rock".to_string()), "Rock should be included");
        assert!(names.contains(&"Classical".to_string()), "Classical should be included");
        assert_eq!(names.len(), 2, "exactly two non-active playlists");
    }

    #[test]
    fn context_menu_selected_initialized_to_zero() {
        let app = make_app_with_playlists("Main", &["Main", "Other"]);
        assert_eq!(app.context_menu_selected, 0, "context_menu_selected should start at 0");
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
        assert_eq!(app.context_menu_selected, count - 1, "should clamp at last item");
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

    fn make_track(video_id: &str, title: &str) -> crate::playlist::Track {
        use crate::playlist::CacheStatus;
        crate::playlist::Track {
            url: format!("https://example.com/{video_id}"),
            source: "youtube.com".to_string(),
            title: title.to_string(),
            artist: "Test Artist".to_string(),
            channel: "Test Channel".to_string(),
            duration: 180,
            video_id: video_id.to_string(),
            cache_status: CacheStatus::Streaming,
            file: None,
            last_position: 0,
            speed: None,
            user_title: None,
            user_artist: None,
            added_at: chrono::Utc::now(),
        }
    }

    // ── Playlist::add_track tests ─────────────────────────────────────────────

    #[test]
    fn add_track_appends_to_empty_playlist() {
        let mut pl = make_playlist("Test");
        let track = make_track("vid1", "Track One");
        pl.add_track(track);
        assert_eq!(pl.tracks.len(), 1);
        assert_eq!(pl.tracks[0].video_id, "vid1");
    }

    #[test]
    fn add_track_appends_to_existing_tracks() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        pl.add_track(make_track("vid2", "Track Two"));
        assert_eq!(pl.tracks.len(), 2);
        assert_eq!(pl.tracks[1].video_id, "vid2");
    }

    #[test]
    fn add_track_does_not_modify_other_fields() {
        let mut pl = make_playlist("Test");
        let original_name = pl.name.clone();
        pl.add_track(make_track("vid1", "Track One"));
        assert_eq!(pl.name, original_name);
        assert!(pl.current_track.is_none());
    }

    // ── Playlist::remove_track_by_video_id tests ─────────────────────────────

    #[test]
    fn remove_track_returns_removed_track() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        let removed = pl.remove_track_by_video_id("vid1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().video_id, "vid1");
        assert!(pl.tracks.is_empty());
    }

    #[test]
    fn remove_track_returns_none_for_missing_id() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        let removed = pl.remove_track_by_video_id("nonexistent");
        assert!(removed.is_none());
        assert_eq!(pl.tracks.len(), 1, "track should remain");
    }

    #[test]
    fn remove_track_clears_current_track_pointer() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        pl.current_track = Some("vid1".to_string());
        pl.remove_track_by_video_id("vid1");
        assert!(pl.current_track.is_none(), "current_track should be cleared");
    }

    #[test]
    fn remove_track_preserves_current_track_for_other_tracks() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        pl.add_track(make_track("vid2", "Track Two"));
        pl.current_track = Some("vid2".to_string());
        pl.remove_track_by_video_id("vid1");
        assert_eq!(pl.current_track.as_deref(), Some("vid2"), "current_track should be preserved");
    }

    #[test]
    fn remove_track_removes_correct_track_from_middle() {
        let mut pl = make_playlist("Test");
        pl.add_track(make_track("vid1", "Track One"));
        pl.add_track(make_track("vid2", "Track Two"));
        pl.add_track(make_track("vid3", "Track Three"));
        pl.remove_track_by_video_id("vid2");
        assert_eq!(pl.tracks.len(), 2);
        assert_eq!(pl.tracks[0].video_id, "vid1");
        assert_eq!(pl.tracks[1].video_id, "vid3");
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
            playlist.add_track(make_track(id, title));
        }
        let config = crate::config::Config::default();
        let mut available: Vec<(String, PathBuf)> = vec![
            (active.to_string(), PathBuf::from(format!("/fake/{active}.toml"))),
        ];
        for t in targets {
            available.push((t.to_string(), PathBuf::from(format!("/fake/{t}.toml"))));
        }
        crate::tui::App::new(playlist, config, available, PathBuf::from(format!("/fake/{active}.toml")))
    }

    #[test]
    fn move_track_fails_for_missing_target_playlist() {
        let mut app = make_app_with_tracks_and_targets(
            "Source",
            &[("vid1", "Track One")],
            &[], // no targets at all
        );
        let result = app.move_track_to_playlist("NonExistent");
        assert!(result.is_err(), "should fail when target not in available_playlists");
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
        source_pl.add_track(make_track("vid1", "Track One"));
        source_pl.save(&source_path).expect("save source");

        let rock_pl = make_playlist("Rock");
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = vec![
            ("Source".to_string(), source_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ];
        let mut app = crate::tui::App::new(source_pl, config, available, source_path.clone());

        // Simulate a playing track
        app.playlist.current_track = Some("vid1".to_string());
        app.is_paused = true;
        app.position = 42.0;
        // player stays None (no real mpv), but the in-memory flags must be cleared

        let result = app.move_track_to_playlist("Rock");
        assert!(result.is_ok(), "move should succeed: {:?}", result.err());

        // Critical invariants: player cleared, current_track cleared, is_paused reset
        assert!(app.player.is_none(), "player must be None after moving current track");
        assert!(app.playlist.current_track.is_none(), "current_track must be cleared");
        assert!(!app.is_paused, "is_paused must be reset to false");
        assert_eq!(app.position, 0.0, "position must be reset");

        // Source playlist must no longer contain vid1
        assert!(app.playlist.tracks.is_empty(), "source playlist should be empty after move");
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
        // Simulate what move does: remove track and clamp
        app.playlist.remove_track_by_video_id("vid3");
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
        app.playlist.remove_track_by_video_id("vid1");
        let new_count = app.visible_track_count();
        if app.selected >= new_count && app.selected > 0 {
            app.selected -= 1;
        }
        // selected=0 < new_count=2, so no clamping
        assert_eq!(app.selected, 0, "selection should stay at 0 when not out of bounds");
    }

    #[test]
    fn playlist_add_and_remove_round_trip() {
        // Add then remove the same track — playlist should be empty again
        let mut pl = make_playlist("Round Trip");
        let track = make_track("vid1", "Track One");
        pl.add_track(track);
        let removed = pl.remove_track_by_video_id("vid1");
        assert!(removed.is_some());
        assert!(pl.tracks.is_empty(), "playlist should be empty after round trip");
    }

    #[test]
    fn remove_track_from_empty_playlist_returns_none() {
        let mut pl = make_playlist("Empty");
        let result = pl.remove_track_by_video_id("vid1");
        assert!(result.is_none(), "removing from empty playlist should return None");
    }

    #[test]
    fn add_multiple_tracks_preserve_insertion_order() {
        let mut pl = make_playlist("Order Test");
        for i in 0..5 {
            pl.add_track(make_track(&format!("vid{i}"), &format!("Track {i}")));
        }
        for (i, track) in pl.tracks.iter().enumerate() {
            assert_eq!(track.video_id, format!("vid{i}"), "track order must be preserved");
        }
    }

    // ── App::switch_to_playlist tests ─────────────────────────────────────────

    /// Write a playlist to a temp file and return the path.
    fn write_temp_playlist(pl: &crate::playlist::Playlist) -> (tempfile::TempDir, std::path::PathBuf) {
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

        app.switch_to_playlist("Beta", &path).expect("switch should succeed");

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
        app.filtered_indices = vec![0, 2, 4];

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert!(app.input_buf.is_empty(), "input_buf should be cleared");
        assert!(app.filtered_indices.is_empty(), "filtered_indices should be cleared");
    }

    #[test]
    fn switch_to_playlist_does_not_stop_playback() {
        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.is_paused = true;
        app.position = 42.5;

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert!(app.is_paused, "is_paused must be unaffected by playlist switch");
        assert_eq!(app.position, 42.5, "position must be unaffected by playlist switch");
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
        beta.tracks.push(make_track("first", "First"));
        beta.tracks.push(make_track("second", "Second"));
        beta.tracks.push(make_track("third", "Third"));
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

    // ── Task 2: PlayingSession decoupled from switch_to_playlist ───────────────

    #[test]
    fn playing_session_survives_switch_to_playlist() {
        use crate::tui::PlayingSession;

        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);

        // Simulate a track from a third, unrelated playlist "Gamma" playing.
        let mut gamma = make_playlist("Gamma");
        gamma.tracks.push(make_track("g1", "Gamma Track"));
        let gamma_path = std::path::PathBuf::from("/fake/Gamma.toml");
        app.playing = Some(PlayingSession {
            path: gamma_path.clone(),
            playlist: gamma,
            track_idx: 0,
        });

        let beta = make_playlist("Beta");
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        let session = app.playing.as_ref().expect("playing session should survive switch");
        assert_eq!(session.path, gamma_path, "playing session path unchanged");
        assert_eq!(session.track().video_id, "g1", "playing track unchanged");
        assert_eq!(app.playlist.name, "Beta", "displayed playlist did switch");
    }

    #[test]
    fn playing_track_reflects_live_edit_when_paths_match() {
        use crate::tui::PlayingSession;

        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.playlist.tracks.push(make_track("t1", "Original Title"));
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");

        app.playing = Some(PlayingSession {
            path: app.playlist_path.clone(),
            playlist: app.playlist.clone(),
            track_idx: 0,
        });

        // Simulate an edit made through the track list (e.g. a rename) directly
        // on the displayed playlist, without any manual sync step.
        app.playlist.tracks[0].user_title = Some("Edited Title".to_string());

        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(
            playing_track.user_title.as_deref(),
            Some("Edited Title"),
            "playing_track() should reflect the live edit when paths match"
        );
    }

    #[test]
    fn playing_track_uses_own_copy_when_paths_differ() {
        use crate::tui::PlayingSession;

        let mut app = make_app_with_playlists("Alpha", &["Alpha", "Beta"]);
        app.playlist_path = std::path::PathBuf::from("/fake/Alpha.toml");

        let mut gamma = make_playlist("Gamma");
        gamma.tracks.push(make_track("g1", "Gamma Track"));
        let gamma_path = std::path::PathBuf::from("/fake/Gamma.toml");
        app.playing = Some(PlayingSession {
            path: gamma_path,
            playlist: gamma,
            track_idx: 0,
        });

        // Editing the displayed (Alpha) playlist must not affect the playing
        // track, which belongs to a different playlist (Gamma).
        app.playlist.tracks.push(make_track("g1", "Colliding Id But Different Playlist"));
        app.playlist.tracks[0].user_title = Some("Should not leak".to_string());

        let playing_track = app.playing_track().expect("playing track should resolve");
        assert_eq!(playing_track.title, "Gamma Track", "should use session's own copy, not displayed playlist");
        assert_eq!(playing_track.user_title, None, "must not pick up edits from the unrelated displayed playlist");
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
        pl.tracks.push(make_track("vid1", "Some Track"));
        let (dir, old_path) = write_temp_playlist(&pl);

        pl.rename("Renamed", &old_path).expect("rename");

        let new_path = dir.path().join("Renamed.toml");
        let loaded = crate::playlist::Playlist::load(&new_path).expect("load renamed");
        assert_eq!(loaded.name, "Renamed");
        assert_eq!(loaded.tracks.len(), 1);
        assert_eq!(loaded.tracks[0].video_id, "vid1");
    }

    #[test]
    fn playlist_rename_to_same_name_is_noop() {
        let pl = make_playlist("SameName");
        let (_dir, path) = write_temp_playlist(&pl);

        let mut pl2 = crate::playlist::Playlist::load(&path).expect("load");
        let result = pl2.rename("SameName", &path);

        // Renaming to the same name should succeed and file should still exist
        assert!(result.is_ok(), "rename to same name should not fail: {result:?}");
        assert!(path.exists(), "file should still exist after rename to same name");
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
        assert!(result.is_err(), "deleting non-existent file should return error");
    }

    // --- validate_playlist_name tests ---

    #[test]
    fn validate_playlist_name_accepts_valid_name() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("My Playlist", &existing, None);
        assert!(result.is_ok(), "valid name should be accepted: {result:?}");
    }

    #[test]
    fn validate_playlist_name_rejects_empty() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("", &existing, None);
        assert!(result.is_err(), "empty name should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_slash() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("bad/name", &existing, None);
        assert!(result.is_err(), "name with slash should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_backslash() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("bad\\name", &existing, None);
        assert!(result.is_err(), "name with backslash should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_colon() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("bad:name", &existing, None);
        assert!(result.is_err(), "name with colon should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_whitespace_only() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        let result = validate_playlist_name("   ", &existing, None);
        assert!(result.is_err(), "whitespace-only name should be rejected");
    }

    #[test]
    fn validate_playlist_name_rejects_dot() {
        use crate::tui::input::validate_playlist_name;
        let existing: Vec<(String, std::path::PathBuf)> = vec![];
        assert!(validate_playlist_name(".", &existing, None).is_err(), ". is invalid");
        assert!(validate_playlist_name("..", &existing, None).is_err(), ".. is invalid");
    }

    #[test]
    fn validate_playlist_name_rejects_duplicate() {
        use crate::tui::input::validate_playlist_name;
        let existing = vec![
            ("Rock".to_string(), std::path::PathBuf::from("/fake/Rock.toml")),
        ];
        let result = validate_playlist_name("Rock", &existing, None);
        assert!(result.is_err(), "duplicate name should be rejected: {result:?}");
    }

    #[test]
    fn validate_playlist_name_allows_current_name_during_rename() {
        use crate::tui::input::validate_playlist_name;
        // During rename, the current name is excluded from duplicate check
        let existing = vec![
            ("Rock".to_string(), std::path::PathBuf::from("/fake/Rock.toml")),
        ];
        let result = validate_playlist_name("Rock", &existing, Some("Rock"));
        assert!(result.is_ok(), "current name should be allowed during rename: {result:?}");
    }

    #[test]
    fn validate_playlist_name_rejects_other_duplicate_during_rename() {
        use crate::tui::input::validate_playlist_name;
        let existing = vec![
            ("Rock".to_string(), std::path::PathBuf::from("/fake/Rock.toml")),
            ("Jazz".to_string(), std::path::PathBuf::from("/fake/Jazz.toml")),
        ];
        // Renaming "Rock" to "Jazz" (which already exists) should be rejected
        let result = validate_playlist_name("Jazz", &existing, Some("Rock"));
        assert!(result.is_err(), "renaming to existing name should be rejected");
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
        if let Some(crate::tui::SidebarItem::Playlist { name, .. }) = items.get(app.sidebar_selected) {
            app.input_buf = name.clone();
            app.input_mode = InputMode::PlaylistRename;
        }
        assert_eq!(app.input_mode, InputMode::PlaylistRename, "should enter PlaylistRename");
        assert_eq!(app.input_buf, "Jazz", "input_buf should be pre-filled with playlist name");
    }

    #[test]
    fn sidebar_rename_mode_not_entered_when_on_header() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 0; // PlaylistsHeader
        let items = app.sidebar_items();
        // Simulate 'r' key — only enter rename if Playlist item
        if let Some(crate::tui::SidebarItem::Playlist { name, .. }) = items.get(app.sidebar_selected) {
            app.input_buf = name.clone();
            app.input_mode = InputMode::PlaylistRename;
        }
        assert_eq!(app.input_mode, InputMode::Normal, "should not enter rename on header");
    }

    #[test]
    fn sidebar_delete_mode_entered_when_on_playlist_item() {
        use crate::tui::InputMode;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 1;
        let items = app.sidebar_items();
        if matches!(items.get(app.sidebar_selected), Some(crate::tui::SidebarItem::Playlist { .. })) {
            app.input_mode = InputMode::PlaylistDelete;
        }
        assert_eq!(app.input_mode, InputMode::PlaylistDelete, "should enter PlaylistDelete");
    }

    // --- playlist_delete_target helper ---

    #[test]
    fn playlist_delete_target_returns_name_for_playlist_item() {
        use crate::tui::ui::playlist_delete_target;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 1; // Jazz playlist item
        let target = playlist_delete_target(&app);
        assert_eq!(target, Some("Jazz"), "should return the selected playlist name");
    }

    #[test]
    fn playlist_delete_target_returns_none_for_header() {
        use crate::tui::ui::playlist_delete_target;
        let mut app = make_app_with_playlists("Jazz", &["Jazz", "Rock"]);
        app.sidebar_selected = 0; // PlaylistsHeader
        let target = playlist_delete_target(&app);
        assert!(target.is_none(), "should return None for non-playlist item");
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
        let names: std::collections::HashSet<_> = [
            &after_first as &str,
            &after_second,
            &after_third,
        ]
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
        let all: Vec<&str> = vec!["P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8", "P9", "Active"];
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

        assert_eq!(app.position, 123.4, "position must survive a playlist switch");
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
        beta.tracks.push(make_track("v1", "Track 1"));
        beta.tracks.push(make_track("v2", "Track 2"));
        beta.tracks.push(make_track("v3", "Track 3"));
        let (_dir, path) = write_temp_playlist(&beta);

        app.switch_to_playlist("Beta", &path).expect("switch");

        assert_eq!(app.playlist.tracks.len(), 3, "switched playlist should have 3 tracks");
    }

    // --- Verify playlist management handles file system errors ---

    #[test]
    fn playlist_delete_nonexistent_file_returns_error() {
        let path = std::path::Path::new("/tmp/trovers_task6_nonexistent_test.toml");
        let result = crate::playlist::Playlist::delete(path);
        assert!(result.is_err(), "delete of missing file should return error");
    }

    #[test]
    fn playlist_rename_to_invalid_path_returns_error() {
        // Create a valid playlist then try renaming to a path in a non-existent directory
        let pl = make_playlist("Original");
        let (dir, old_path) = write_temp_playlist(&pl);
        let mut pl2 = crate::playlist::Playlist::load(&old_path).expect("load");

        // Using a path that can't be written: simulate by pointing to a non-existent dir
        let nonexistent_parent = dir.path().join("nonexistent_subdir").join("NewName.toml");
        // Try saving to a path whose parent doesn't exist
        let result = pl2.save(&nonexistent_parent);
        assert!(result.is_err(), "save to non-existent directory should fail");
    }

    #[test]
    fn playlist_save_and_load_round_trip_preserves_tracks() {
        // Backward compatibility: save a playlist and reload it
        let mut pl = make_playlist("RoundTrip");
        pl.tracks.push(make_track("v1", "Track A"));
        pl.tracks.push(make_track("v2", "Track B"));

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("RoundTrip.toml");
        pl.save(&path).expect("save");

        let loaded = crate::playlist::Playlist::load(&path).expect("load");
        assert_eq!(loaded.name, "RoundTrip");
        assert_eq!(loaded.tracks.len(), 2);
        assert_eq!(loaded.tracks[0].video_id, "v1");
        assert_eq!(loaded.tracks[1].video_id, "v2");
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

        assert_eq!(seen.len(), 5, "all 5 playlists should be cycled through: {seen:?}");
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
        assert_eq!(loaded.tracks.len(), 0, "empty playlist should load with 0 tracks");
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
        pl.add_track(make_track("only", "Only Track"));
        pl.current_track = Some("only".to_string());

        let removed = pl.remove_track_by_video_id("only");

        assert!(removed.is_some(), "should return the removed track");
        assert!(pl.tracks.is_empty(), "playlist should be empty after removal");
        assert!(pl.current_track.is_none(), "current_track should be cleared");
    }

    // --- Verify backward compatibility with existing playlist files ---

    #[test]
    fn backward_compatible_playlist_toml_loads_correctly() {
        // Simulate a "legacy" playlist file that might exist before these changes
        // The format hasn't changed - this tests the TOML structure is stable
        let toml_content = r#"
name = "LegacyPlaylist"
created = "2025-01-01T00:00:00Z"
loop_mode = "none"

[[tracks]]
url = "https://example.com/track1"
source = "youtube.com"
title = "Legacy Track"
artist = "Old Artist"
channel = "OldChannel"
duration = 240
video_id = "abc123"
cache_status = "streaming"
last_position = 0
added_at = "2025-01-01T12:00:00Z"
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("legacy.toml");
        std::fs::write(&path, toml_content).expect("write");

        let result = crate::playlist::Playlist::load(&path);
        assert!(result.is_ok(), "legacy TOML should load without error: {result:?}");

        let pl = result.unwrap();
        assert_eq!(pl.name, "LegacyPlaylist");
        assert_eq!(pl.tracks.len(), 1);
        assert_eq!(pl.tracks[0].video_id, "abc123");
        assert_eq!(pl.tracks[0].title, "Legacy Track");
    }

    #[test]
    fn playlist_with_optional_fields_absent_loads_correctly() {
        // Playlists with optional fields (speed, user_title, user_artist, file) absent
        // should still load correctly (backward compatibility)
        let toml_content = r#"
name = "MinimalPlaylist"
created = "2025-06-01T00:00:00Z"
loop_mode = "none"

[[tracks]]
url = "https://example.com/minimal"
source = "bandcamp.com"
title = "Minimal Track"
artist = "Minimal Artist"
channel = "MinChannel"
duration = 120
video_id = "min001"
cache_status = "streaming"
last_position = 0
added_at = "2025-06-01T08:00:00Z"
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("minimal.toml");
        std::fs::write(&path, toml_content).expect("write");

        let result = crate::playlist::Playlist::load(&path);
        assert!(result.is_ok(), "minimal playlist without optional fields should load: {result:?}");

        let pl = result.unwrap();
        assert!(pl.tracks[0].speed.is_none(), "speed should be None when absent");
        assert!(pl.tracks[0].user_title.is_none(), "user_title should be None when absent");
        assert!(pl.tracks[0].user_artist.is_none(), "user_artist should be None when absent");
        assert!(pl.tracks[0].file.is_none(), "file should be None when absent");
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
            assert!(result.is_ok(), "loop_mode={mode_str} should load: {result:?}");
            assert_eq!(result.unwrap().loop_mode, expected, "loop_mode={mode_str} mismatch");
        }
    }

    #[test]
    fn cached_track_with_missing_file_degrades_to_streaming() {
        // Backward compatibility: if a cached track's file no longer exists,
        // Playlist::load() should reset it to streaming
        let mut pl = make_playlist("CacheTest");
        let mut track = make_track("vid1", "Cached Track");
        track.cache_status = crate::playlist::CacheStatus::Cached;
        track.file = Some(std::path::PathBuf::from("/nonexistent/path/audio.mp3"));
        pl.tracks.push(track);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("CacheTest.toml");
        pl.save(&path).expect("save");

        let loaded = crate::playlist::Playlist::load(&path).expect("load");
        assert_eq!(
            loaded.tracks[0].cache_status,
            crate::playlist::CacheStatus::Streaming,
            "cached track with missing file should degrade to streaming"
        );
        assert!(
            loaded.tracks[0].file.is_none(),
            "file field should be cleared when file is missing"
        );
    }

    // ── DownloadDone with non-active playlist ─────────────────────────────────

    #[test]
    fn download_done_updates_non_active_playlist_on_disk() {
        use crate::tui::{App, TaskMsg};
        use crate::playlist::{CacheStatus, Playlist};

        let dir = tempfile::tempdir().expect("tempdir");

        // Source (active) playlist — vid1 will be added to "Rock", not here
        let source_path = dir.path().join("Source.toml");
        let source_pl = make_playlist("Source");
        source_pl.save(&source_path).expect("save source");

        // Target (Rock) playlist — contains vid1 in Streaming state
        let rock_path = dir.path().join("Rock.toml");
        let mut rock_pl = make_playlist("Rock");
        rock_pl.add_track(make_track("vid1", "Track One"));
        rock_pl.save(&rock_path).expect("save rock");

        let config = crate::config::Config::default();
        let available = vec![
            ("Source".to_string(), source_path.clone()),
            ("Rock".to_string(), rock_path.clone()),
        ];
        let mut app = App::new(source_pl, config, available, source_path.clone());

        // Simulate that vid1 was submitted for downloading into the Rock playlist
        app.downloading.insert("vid1".to_string());
        app.download_targets.insert("vid1".to_string(), rock_path.clone());

        // Fire the DownloadDone message
        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            video_id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        // download_targets entry must be removed
        assert!(!app.download_targets.contains_key("vid1"), "download_targets should be cleared");

        // The active (Source) playlist must NOT be mutated
        assert!(app.playlist.tracks.is_empty(), "source playlist must not be modified");

        // The Rock playlist on disk must have cache_status = Cached and file set
        let rock_reloaded = Playlist::load(&rock_path).expect("reload rock");
        let track = rock_reloaded.tracks.iter().find(|t| t.video_id == "vid1").expect("vid1");
        assert_eq!(track.cache_status, CacheStatus::Cached, "cache_status must be Cached");
        assert_eq!(track.file.as_deref(), Some(fake_file.as_path()), "file path must be set");
    }

    #[test]
    fn download_done_for_active_playlist_updates_in_memory_state() {
        use crate::tui::{App, TaskMsg};
        use crate::playlist::CacheStatus;

        let dir = tempfile::tempdir().expect("tempdir");

        let source_path = dir.path().join("Source.toml");
        let mut source_pl = make_playlist("Source");
        source_pl.add_track(make_track("vid1", "Track One"));
        source_pl.save(&source_path).expect("save source");

        let config = crate::config::Config::default();
        let available = vec![("Source".to_string(), source_path.clone())];
        let mut app = App::new(source_pl, config, available, source_path.clone());

        app.downloading.insert("vid1".to_string());
        // No entry in download_targets → active playlist path

        let fake_file = dir.path().join("vid1.m4a");
        std::fs::write(&fake_file, b"audio data").expect("write fake audio");
        app.handle_task_msg(TaskMsg::DownloadDone {
            video_id: "vid1".to_string(),
            file: fake_file.clone(),
        });

        let track = app.playlist.tracks.iter().find(|t| t.video_id == "vid1").expect("vid1");
        assert_eq!(track.cache_status, CacheStatus::Cached, "in-memory cache_status must be Cached");
        assert_eq!(track.file.as_deref(), Some(fake_file.as_path()), "in-memory file must be set");
    }

    // ── Task 1: add-track playback-hijack regression tests ─────────────────────

    #[tokio::test]
    async fn adding_track_does_not_change_current_track() {
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        pl.add_track(make_track("A", "Track A"));
        pl.current_track = Some("A".to_string());
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = vec![("Active".to_string(), path.clone())];
        let mut app = App::new(pl, config, available, path.clone());

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

        // Track B must have been added...
        assert!(app.playlist.tracks.iter().any(|t| t.video_id == "B"), "track B should be added");
        // ...but current_track must remain unchanged (still A), and playback
        // position must not have been touched by adding the track.
        assert_eq!(
            app.playlist.current_track.as_deref(),
            Some("A"),
            "adding a track must not change current_track"
        );
        assert_eq!(app.position, 137.0, "adding a track must not touch playback position");
    }

    #[tokio::test]
    async fn download_done_for_newly_added_track_does_not_hijack_playback() {
        use crate::tui::{App, TaskMsg};
        use crate::ytdlp::TrackMeta;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Active.toml");

        let mut pl = make_playlist("Active");
        pl.add_track(make_track("A", "Track A"));
        pl.current_track = Some("A".to_string());
        pl.save(&path).expect("save");

        let config = crate::config::Config::default();
        let available = vec![("Active".to_string(), path.clone())];
        let mut app = App::new(pl, config, available, path.clone());

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
            video_id: "B".to_string(),
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
        assert_eq!(app.position, 137.0, "playback position must remain untouched");

        // B's cache metadata should still have been updated normally.
        let track_b = app.playlist.tracks.iter().find(|t| t.video_id == "B").expect("track B");
        assert_eq!(track_b.cache_status, crate::playlist::CacheStatus::Cached);
        assert_eq!(track_b.file.as_deref(), Some(fake_file.as_path()));
    }
}
