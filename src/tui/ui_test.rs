#[cfg(test)]
mod tests {
    use crate::tui::ui::{
        build_progress_bar, build_separated_line, calculate_distributed_widths,
        format_duration, format_playback_state, truncate,
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
}
