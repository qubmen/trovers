#[cfg(test)]
mod tests {
    use crate::player::mpv_args;
    use std::path::Path;

    fn args(video: bool) -> Vec<String> {
        mpv_args(Path::new("/tmp/trovers-1-0.sock"), None, video, &[])
    }

    // ── Audio: no window, ever ────────────────────────────────────────────

    #[test]
    fn audio_playback_passes_no_video() {
        assert!(args(false).contains(&"--no-video".to_string()));
    }

    #[test]
    fn audio_playback_does_not_force_a_window() {
        assert!(
            !args(false).iter().any(|a| a.starts_with("--force-window")),
            "an audio track must never open a window"
        );
    }

    // ── Video: a window, forced ───────────────────────────────────────────

    #[test]
    fn video_playback_drops_no_video() {
        assert!(
            !args(true).contains(&"--no-video".to_string()),
            "--no-video would leave a video file playing its audio only"
        );
    }

    #[test]
    fn video_playback_forces_a_window() {
        // Without this mpv can decide it has nothing worth a window and play the
        // file with none at all.
        assert!(args(true).contains(&"--force-window=yes".to_string()));
    }

    // ── The tty is the TUI's, whatever is playing ─────────────────────────

    #[test]
    fn both_kinds_keep_mpv_off_the_terminal() {
        for video in [false, true] {
            let got = args(video);
            assert!(
                got.contains(&"--no-terminal".to_string()),
                "video={video}: mpv would compete for keystrokes on the shared tty"
            );
            assert!(
                got.contains(&"--really-quiet".to_string()),
                "video={video}: mpv would write over the TUI"
            );
        }
    }

    #[test]
    fn both_kinds_get_an_ipc_socket() {
        for video in [false, true] {
            assert!(
                args(video).contains(&"--input-ipc-server=/tmp/trovers-1-0.sock".to_string()),
                "video={video}"
            );
        }
    }

    // ── Resume position ───────────────────────────────────────────────────

    #[test]
    fn a_resume_position_becomes_a_start_flag() {
        let got = mpv_args(Path::new("/tmp/s.sock"), Some(176.5), false, &[]);
        assert!(got.contains(&"--start=176.500".to_string()));
    }

    #[test]
    fn a_position_at_the_very_start_is_left_off() {
        // `--start=0` is not wrong, just noise — and a track that has never been
        // played carries `last_position = 0`.
        let got = mpv_args(Path::new("/tmp/s.sock"), Some(0.0), false, &[]);
        assert!(!got.iter().any(|a| a.starts_with("--start")), "got {got:?}");
    }

    // ── The user's own flags ──────────────────────────────────────────────

    #[test]
    fn configured_video_args_are_appended_for_video() {
        let extra = vec!["--focus-on=never".to_string(), "--ontop".to_string()];
        let got = mpv_args(Path::new("/tmp/s.sock"), None, true, &extra);
        assert!(got.contains(&"--focus-on=never".to_string()));
        assert!(got.contains(&"--ontop".to_string()));
    }

    #[test]
    fn configured_video_args_come_last_so_they_win() {
        // mpv takes the last of two conflicting options, so a user who sets
        // `--force-window=no` gets what they asked for.
        let extra = vec!["--force-window=no".to_string()];
        let got = mpv_args(Path::new("/tmp/s.sock"), None, true, &extra);
        let ours = got
            .iter()
            .position(|a| a == "--force-window=yes")
            .expect("ours");
        let theirs = got
            .iter()
            .position(|a| a == "--force-window=no")
            .expect("theirs");
        assert!(ours < theirs, "got {got:?}");
    }

    #[test]
    fn configured_video_args_are_left_out_of_audio_playback() {
        // They are window-management flags. Passing them to an audio track would
        // be at best pointless and at worst — an unknown option is fatal to mpv —
        // enough to stop music playing at all.
        let extra = vec!["--focus-on=never".to_string()];
        let got = mpv_args(Path::new("/tmp/s.sock"), None, false, &extra);
        assert!(
            !got.contains(&"--focus-on=never".to_string()),
            "got {got:?}"
        );
    }
}
