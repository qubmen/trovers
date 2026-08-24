#[cfg(test)]
mod tests {
    use crate::ytdlp::{
        blocked_by_youtube_hint, clean_partial_downloads, extract_domain, find_downloaded_file,
        first_url_line, is_partial_artifact, parse_destination_line, parse_progress_line,
        retry_with_backoff,
    };
    use anyhow::{anyhow, Result};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    // ── first_url_line ──────────────────────────────────────────────────────

    #[test]
    fn first_url_line_takes_the_first_non_blank_line() {
        assert_eq!(
            first_url_line("https://a.example/audio\nhttps://a.example/video\n"),
            Some("https://a.example/audio".to_string())
        );
    }

    #[test]
    fn first_url_line_skips_leading_blank_lines() {
        assert_eq!(
            first_url_line("\n\n  \nhttps://a.example/audio"),
            Some("https://a.example/audio".to_string())
        );
    }

    #[test]
    fn first_url_line_trims_whitespace() {
        assert_eq!(
            first_url_line("  https://a.example/audio  \n"),
            Some("https://a.example/audio".to_string())
        );
    }

    #[test]
    fn first_url_line_is_none_for_empty_output() {
        assert_eq!(first_url_line(""), None);
        assert_eq!(first_url_line("\n\n  \n"), None);
    }

    // ── parse_progress_line ─────────────────────────────────────────────────

    #[test]
    fn parse_progress_line_reads_the_percentage() {
        assert_eq!(
            parse_progress_line("[download]  42.5% of 3.20MiB at 1.00MiB/s"),
            Some(42.5)
        );
    }

    #[test]
    fn parse_progress_line_reads_a_whole_number_percentage() {
        assert_eq!(
            parse_progress_line("[download] 100% of 3.20MiB"),
            Some(100.0)
        );
    }

    #[test]
    fn parse_progress_line_ignores_unrelated_lines() {
        assert_eq!(
            parse_progress_line("[ExtractAudio] Destination: foo.opus"),
            None
        );
        assert_eq!(parse_progress_line("some random log line"), None);
    }

    // ── parse_destination_line ──────────────────────────────────────────────

    #[test]
    fn parse_destination_line_reads_a_download_destination() {
        assert_eq!(
            parse_destination_line("[download] Destination: /tmp/audio/abc123.webm"),
            Some(PathBuf::from("/tmp/audio/abc123.webm"))
        );
    }

    #[test]
    fn parse_destination_line_reads_an_extract_audio_destination() {
        assert_eq!(
            parse_destination_line("[ExtractAudio] Destination: /tmp/audio/abc123.opus"),
            Some(PathBuf::from("/tmp/audio/abc123.opus"))
        );
    }

    #[test]
    fn parse_destination_line_trims_trailing_whitespace() {
        assert_eq!(
            parse_destination_line("[download] Destination: /tmp/audio/abc123.webm  "),
            Some(PathBuf::from("/tmp/audio/abc123.webm"))
        );
    }

    #[test]
    fn parse_destination_line_ignores_unrelated_lines() {
        assert_eq!(parse_destination_line("[download]  42.5% of 3.20MiB"), None);
    }

    // ── is_partial_artifact ─────────────────────────────────────────────────

    #[test]
    fn is_partial_artifact_matches_part_files() {
        assert!(is_partial_artifact("abc123.opus.part", "abc123"));
    }

    #[test]
    fn is_partial_artifact_matches_fragment_part_files() {
        assert!(is_partial_artifact("abc123.opus.part-Frag3", "abc123"));
    }

    #[test]
    fn is_partial_artifact_matches_ytdl_resume_state() {
        assert!(is_partial_artifact("abc123.webm.ytdl", "abc123"));
    }

    #[test]
    fn is_partial_artifact_matches_temp_intermediates() {
        assert!(is_partial_artifact("abc123.webm.temp", "abc123"));
    }

    #[test]
    fn is_partial_artifact_does_not_match_a_finished_file() {
        assert!(!is_partial_artifact("abc123.opus", "abc123"));
    }

    #[test]
    fn is_partial_artifact_does_not_match_an_unrelated_id_with_the_same_prefix() {
        // "abc1234" merely starts with "abc123" — must not be treated as ours.
        assert!(!is_partial_artifact("abc1234.opus.part", "abc123"));
    }

    #[test]
    fn is_partial_artifact_does_not_match_a_different_id() {
        assert!(!is_partial_artifact("xyz789.opus.part", "abc123"));
    }

    // ── clean_partial_downloads ──────────────────────────────────────────────

    #[test]
    fn clean_partial_downloads_removes_only_this_ids_scratch_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let finished = dir.path().join("abc123.opus");
        let partial = dir.path().join("abc123.webm.part");
        let other_finished = dir.path().join("xyz789.opus");
        std::fs::write(&finished, b"").unwrap();
        std::fs::write(&partial, b"").unwrap();
        std::fs::write(&other_finished, b"").unwrap();

        clean_partial_downloads(dir.path(), "abc123");

        assert!(finished.exists(), "a finished file must survive cleanup");
        assert!(!partial.exists(), "the partial artifact must be removed");
        assert!(
            other_finished.exists(),
            "another track's cached file must not be touched"
        );
    }

    #[test]
    fn clean_partial_downloads_on_a_missing_dir_does_not_panic() {
        clean_partial_downloads(Path::new("/no/such/dir"), "abc123");
    }

    // ── find_downloaded_file ─────────────────────────────────────────────────

    #[test]
    fn find_downloaded_file_prefers_opus_over_other_extensions() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("abc123.webm"), b"").unwrap();
        std::fs::write(dir.path().join("abc123.opus"), b"").unwrap();

        let found = find_downloaded_file(dir.path(), "abc123");

        assert_eq!(found, Some(dir.path().join("abc123.opus")));
    }

    #[test]
    fn find_downloaded_file_falls_back_to_any_matching_stem() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("abc123.webm"), b"").unwrap();

        let found = find_downloaded_file(dir.path(), "abc123");

        assert_eq!(found, Some(dir.path().join("abc123.webm")));
    }

    #[test]
    fn find_downloaded_file_is_none_when_nothing_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("xyz789.opus"), b"").unwrap();

        assert_eq!(find_downloaded_file(dir.path(), "abc123"), None);
    }

    // ── blocked_by_youtube_hint ──────────────────────────────────────────────

    #[test]
    fn blocked_by_youtube_hint_recognizes_a_403() {
        assert!(blocked_by_youtube_hint("ERROR: HTTP Error 403: Forbidden").is_some());
    }

    #[test]
    fn blocked_by_youtube_hint_recognizes_a_bot_check() {
        assert!(blocked_by_youtube_hint("Sign in to confirm you're not a bot").is_some());
    }

    #[test]
    fn blocked_by_youtube_hint_is_none_for_an_unrelated_error() {
        assert_eq!(blocked_by_youtube_hint("network is unreachable"), None);
    }

    // ── extract_domain ───────────────────────────────────────────────────────

    #[test]
    fn extract_domain_strips_scheme_and_www() {
        assert_eq!(
            extract_domain("https://www.youtube.com/watch?v=abc"),
            "youtube.com"
        );
    }

    #[test]
    fn extract_domain_handles_plain_http() {
        assert_eq!(extract_domain("http://example.com/path"), "example.com");
    }

    #[test]
    fn extract_domain_handles_a_bare_host_with_no_path() {
        assert_eq!(extract_domain("https://youtube.com"), "youtube.com");
    }

    #[test]
    fn extract_domain_handles_a_url_with_no_scheme() {
        assert_eq!(extract_domain("youtube.com/watch?v=abc"), "youtube.com");
    }

    // ── retry_with_backoff ───────────────────────────────────────────────────

    #[tokio::test]
    async fn retry_with_backoff_returns_ok_on_the_first_success() {
        let calls = AtomicU32::new(0);
        let result = retry_with_backoff(&[], || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, anyhow::Error>(42) }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_with_backoff_retries_up_to_delays_len_plus_one_times() {
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let calls = AtomicU32::new(0);

        let result: Result<()> = retry_with_backoff(&delays, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(anyhow!("still failing")) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "one initial attempt plus one retry per delay"
        );
    }

    #[tokio::test]
    async fn retry_with_backoff_succeeds_after_a_transient_failure() {
        let delays = [Duration::from_millis(1)];
        let calls = AtomicU32::new(0);

        let result = retry_with_backoff(&delays, || {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if attempt == 0 {
                    Err(anyhow!("transient"))
                } else {
                    Ok(99)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
