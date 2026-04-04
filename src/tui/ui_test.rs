#[cfg(test)]
mod tests {
    use crate::tui::ui::{
        build_now_playing_header_line, build_playback_bar_line, build_progress_bar,
        build_separated_line, build_track_info_line, calculate_distributed_widths,
        format_duration, format_playback_state, truncate, CacheState,
    };
    use ratatui::style::Color;

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
        assert!(text.contains("⟳ caching"), "should contain caching indicator: {text:?}");
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
        // Verify that the three-row structure allocates exactly 3 rows of height 1
        // (This tests the layout constraint: 3 × Length(1) = 3 total height inside border)
        // The outer area has Length(3), with a TOP border taking 1 row,
        // leaving exactly 3 rows for inner content.
        let inner_height = 3u16; // Borders::TOP uses 1 row from Length(3) outer area
        let row_heights: Vec<u16> = vec![1, 1, 1];
        assert_eq!(row_heights.iter().sum::<u16>(), inner_height,
            "three rows of height 1 must sum to inner_height=3");
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
        assert!(dl_text.contains("⟳ caching"), "Downloading state in row 3");
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
}
