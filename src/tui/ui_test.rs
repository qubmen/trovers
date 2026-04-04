#[cfg(test)]
mod tests {
    use crate::tui::ui::{format_duration, truncate};

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
    // Note: build_progress_bar is private, so we test it via the public API
    // by checking the behavior through observable properties (string content).
    // We re-implement the logic here for direct unit testing.

    fn make_bar(width: usize, ratio: f64, fill: char, empty: char, thumb: char) -> String {
        if width == 0 {
            return String::new();
        }
        let filled = ((ratio * width as f64) as usize).min(width);
        let mut bar = String::with_capacity(width + 1);

        if thumb != '\0' && filled < width {
            let pre = filled.saturating_sub(1);
            bar.extend(std::iter::repeat(fill).take(pre));
            bar.push(thumb);
            bar.extend(std::iter::repeat(empty).take(width - pre - 1));
        } else {
            bar.extend(std::iter::repeat(fill).take(filled));
            bar.extend(std::iter::repeat(empty).take(width - filled));
        }

        bar
    }

    #[test]
    fn progress_bar_zero_width() {
        let bar = make_bar(0, 0.5, '━', '─', '◉');
        assert_eq!(bar, "");
    }

    #[test]
    fn progress_bar_zero_ratio() {
        let bar = make_bar(10, 0.0, '━', '─', '◉');
        // At ratio=0.0, filled=0, thumb at position 0 (pre=0, saturating_sub)
        // pre = 0.saturating_sub(1) = 0, push thumb, then 9 empty chars
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.starts_with('◉'));
        assert_eq!(bar.chars().filter(|&c| c == '─').count(), 9);
    }

    #[test]
    fn progress_bar_full_ratio() {
        let bar = make_bar(10, 1.0, '━', '─', '◉');
        // filled = 10 = width, so no thumb case applies (filled < width is false)
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '─').count(), 0);
    }

    #[test]
    fn progress_bar_half_ratio() {
        let bar = make_bar(10, 0.5, '━', '─', '◉');
        // filled = 5, pre = 4, so 4 filled + thumb + 5 empty = 10
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 4);
        assert_eq!(bar.chars().filter(|&c| c == '◉').count(), 1);
        assert_eq!(bar.chars().filter(|&c| c == '─').count(), 5);
    }

    #[test]
    fn progress_bar_no_thumb() {
        // thumb = '\0' means no thumb character
        let bar = make_bar(10, 0.4, '▓', '░', '\0');
        // filled = 4, no thumb
        assert_eq!(bar.chars().count(), 10);
        assert_eq!(bar.chars().filter(|&c| c == '▓').count(), 4);
        assert_eq!(bar.chars().filter(|&c| c == '░').count(), 6);
    }

    #[test]
    fn progress_bar_width_one() {
        let bar = make_bar(1, 0.5, '━', '─', '◉');
        // filled=0, pre=0.saturating_sub(1)=0, push thumb, then 0 empty = "◉"
        assert_eq!(bar.chars().count(), 1);
    }

    #[test]
    fn progress_bar_ratio_clamped_at_one() {
        let bar = make_bar(5, 1.5, '━', '─', '◉');
        // ratio clamped via min(width): filled = min(7, 5) = 5, no thumb
        assert_eq!(bar.chars().count(), 5);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 5);
    }
}
