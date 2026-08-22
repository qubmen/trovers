#[cfg(test)]
mod tests {
    use crate::library::{platform_id_of, MediaKind};
    use crate::library_scan::{
        local_id, parse_ffprobe, parse_filename, resolve_meta, scan, MAX_DEPTH,
    };
    use std::path::Path;

    /// Create `path` and every directory above it, with some bytes inside.
    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(path, b"bytes").expect("write");
    }

    /// The scanned paths, relative to `root` and slash-joined, which is far easier
    /// to assert on than a list of absolute temp paths.
    fn relative_paths(root: &Path) -> Vec<String> {
        scan(root)
            .into_iter()
            .map(|f| {
                f.path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    // ── local_id ──────────────────────────────────────────────────────────

    /// The whole reason the id is derived from the path: a rescan must land on
    /// the same id so the document — and with it `last_position` — is reused
    /// rather than duplicated.
    #[test]
    fn local_id_is_stable_for_the_same_path() {
        let a = local_id(Path::new("/Users/den/Music/set.flac"));
        let b = local_id(Path::new("/Users/den/Music/set.flac"));
        assert_eq!(a, b);
    }

    #[test]
    fn local_id_differs_across_paths() {
        let a = local_id(Path::new("/Users/den/Music/set.flac"));
        let b = local_id(Path::new("/Users/den/Music/other.flac"));
        assert_ne!(a, b);
    }

    /// `/a/./b` and `/a//b` name the same file, so they must not mint two
    /// documents for it. Lexical only — the file may be on an unplugged drive,
    /// and `canonicalize` needs it to exist.
    #[test]
    fn local_id_ignores_redundant_path_syntax() {
        let plain = local_id(Path::new("/Users/den/Music/set.flac"));
        assert_eq!(local_id(Path::new("/Users/den/./Music/set.flac")), plain);
        assert_eq!(local_id(Path::new("/Users/den//Music/set.flac")), plain);
    }

    /// A local id is an ordinary library id, so everything keyed by ids — the
    /// playlists, `platform_id_of`, the delete guards — needs no special case.
    #[test]
    fn local_id_is_a_library_id_under_the_local_slug() {
        let id = local_id(Path::new("/Users/den/Music/set.flac"));
        assert!(id.starts_with("local:"), "got {id}");
        let hex = platform_id_of(&id);
        assert!(
            !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()),
            "the platform half should be a bare hash, got {hex}"
        );
    }

    // ── parse_filename ────────────────────────────────────────────────────

    #[test]
    fn parse_filename_splits_artist_and_title() {
        let parsed = parse_filename("Miss Monique - Live at UNVRS");
        assert_eq!(parsed.artist.as_deref(), Some("Miss Monique"));
        assert_eq!(parsed.title, "Live at UNVRS");
    }

    #[test]
    fn parse_filename_without_a_separator_is_all_title() {
        let parsed = parse_filename("some_recording");
        assert_eq!(parsed.artist, None);
        assert_eq!(parsed.title, "some_recording");
    }

    /// A title is far likelier to contain a dash than a file is to name two
    /// artists, so the first separator wins and the rest stays in the title.
    #[test]
    fn parse_filename_splits_on_the_first_separator_only() {
        let parsed = parse_filename("ARTBAT - Live - Ultra 2026");
        assert_eq!(parsed.artist.as_deref(), Some("ARTBAT"));
        assert_eq!(parsed.title, "Live - Ultra 2026");
    }

    /// `01 - Intro` is a track number, not an artist called "01".
    #[test]
    fn parse_filename_treats_a_leading_number_as_a_track_number() {
        let parsed = parse_filename("01 - Intro");
        assert_eq!(parsed.artist, None);
        assert_eq!(parsed.title, "Intro");
    }

    #[test]
    fn parse_filename_strips_a_numbered_prefix_without_a_dash() {
        assert_eq!(parse_filename("03. Intro").title, "Intro");
        assert_eq!(parse_filename("7) Outro").title, "Outro");
    }

    /// A hyphen inside a word is not a separator — only a spaced one is.
    #[test]
    fn parse_filename_keeps_a_hyphenated_word_intact() {
        let parsed = parse_filename("Jean-Michel Jarre");
        assert_eq!(parsed.artist, None);
        assert_eq!(parsed.title, "Jean-Michel Jarre");
    }

    /// Whatever the filename, a row has to be labelled with something.
    #[test]
    fn parse_filename_never_yields_an_empty_title() {
        assert!(!parse_filename("").title.is_empty());
        assert!(!parse_filename("   ").title.is_empty());
        assert!(!parse_filename("Artist - ").title.is_empty());
    }

    // ── scan ──────────────────────────────────────────────────────────────

    #[test]
    fn scan_of_a_directory_that_does_not_exist_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(scan(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn scan_finds_media_files_and_ignores_everything_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("set.flac"));
        touch(&dir.path().join("cover.jpg"));
        touch(&dir.path().join("notes.txt"));
        touch(&dir.path().join("no-extension"));

        assert_eq!(relative_paths(dir.path()), vec!["set.flac"]);
    }

    /// Sorted, so an import produces the same running order every time and a
    /// rescan does not shuffle the album.
    ///
    /// `zzz.mp3` sits in the root and `sub/aaa.mp3` a level down, which is what
    /// makes this catch a missing sort rather than merely allow one: the walk
    /// reaches the root's own files first, so unsorted output puts `zzz.mp3`
    /// before the `sub/` entries that sort ahead of it.
    #[test]
    fn scan_recurses_and_sorts_by_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("zzz.mp3"));
        touch(&dir.path().join("sub/aaa.mp3"));
        touch(&dir.path().join("sub/deeper/bbb.mp3"));

        assert_eq!(
            relative_paths(dir.path()),
            vec!["sub/aaa.mp3", "sub/deeper/bbb.mp3", "zzz.mp3"]
        );
    }

    #[test]
    fn scan_labels_video_files_video_and_audio_files_audio() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("clip.mkv"));
        touch(&dir.path().join("song.opus"));

        let found = scan(dir.path());
        let kinds: Vec<MediaKind> = found.iter().map(|f| f.media).collect();
        assert_eq!(kinds, vec![MediaKind::Video, MediaKind::Audio]);
    }

    /// Extensions are spelled however the file was written.
    #[test]
    fn scan_matches_extensions_case_insensitively() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("SET.FLAC"));
        assert_eq!(relative_paths(dir.path()), vec!["SET.FLAC"]);
    }

    /// macOS scatters `._name.mp3` resource forks next to real files on
    /// non-native filesystems; they carry a media extension and are junk.
    #[test]
    fn scan_skips_hidden_files_and_appledouble_forks() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("set.flac"));
        touch(&dir.path().join("._set.flac"));
        touch(&dir.path().join(".hidden.mp3"));

        assert_eq!(relative_paths(dir.path()), vec!["set.flac"]);
    }

    #[test]
    fn scan_stops_at_the_depth_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("near/the/top.mp3"));

        let mut deep = dir.path().to_path_buf();
        for _ in 0..=MAX_DEPTH {
            deep.push("d");
        }
        touch(&deep.join("too-deep.mp3"));

        assert_eq!(relative_paths(dir.path()), vec!["near/the/top.mp3"]);
    }

    /// A symlink pointing at an ancestor is an infinite tree. Not following
    /// directory symlinks at all is the cheap, total answer.
    #[cfg(unix)]
    #[test]
    fn scan_does_not_follow_a_directory_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("sub/set.flac"));
        std::os::unix::fs::symlink(dir.path(), dir.path().join("sub/loop")).expect("symlink");

        assert_eq!(relative_paths(dir.path()), vec!["sub/set.flac"]);
    }

    // ── parse_ffprobe ─────────────────────────────────────────────────────

    /// A trimmed-down but shape-accurate `ffprobe -print_format json
    /// -show_format -show_streams` payload.
    fn ffprobe_json(streams: &str, tags: &str) -> String {
        format!(
            r#"{{
  "streams": [{streams}],
  "format": {{
    "filename": "x",
    "duration": "3529.123000",
    "tags": {{{tags}}}
  }}
}}"#
        )
    }

    #[test]
    fn parse_ffprobe_reads_duration_title_and_artist() {
        let json = ffprobe_json(
            r#"{"codec_type": "audio", "codec_name": "flac"}"#,
            r#""title": "Live at UNVRS", "artist": "Miss Monique""#,
        );
        let meta = parse_ffprobe(&json).expect("parsed");
        assert_eq!(meta.duration, Some(3529));
        assert_eq!(meta.title.as_deref(), Some("Live at UNVRS"));
        assert_eq!(meta.artist.as_deref(), Some("Miss Monique"));
        assert!(!meta.has_video);
    }

    /// The reason ffprobe is worth calling at all: the container's extension is
    /// a guess, and this is the answer.
    #[test]
    fn parse_ffprobe_calls_a_file_with_a_real_video_stream_video() {
        let json = ffprobe_json(
            r#"{"codec_type": "video", "codec_name": "h264"}, {"codec_type": "audio", "codec_name": "aac"}"#,
            "",
        );
        assert!(parse_ffprobe(&json).expect("parsed").has_video);
    }

    /// An `.mkv` holding nothing but audio is audio, whatever the extension says.
    #[test]
    fn parse_ffprobe_calls_an_audio_only_container_audio() {
        let json = ffprobe_json(r#"{"codec_type": "audio", "codec_name": "opus"}"#, "");
        assert!(!parse_ffprobe(&json).expect("parsed").has_video);
    }

    /// Embedded cover art is a video stream as far as ffprobe is concerned. An
    /// mp3 with artwork must not open a video window.
    #[test]
    fn parse_ffprobe_does_not_mistake_cover_art_for_video() {
        let json = ffprobe_json(
            r#"{"codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
                {"codec_type": "audio", "codec_name": "mp3"}"#,
            "",
        );
        assert!(!parse_ffprobe(&json).expect("parsed").has_video);
    }

    /// Same trap without the disposition flag, which older ffprobe builds omit:
    /// a still-image codec is artwork, not video.
    #[test]
    fn parse_ffprobe_treats_a_still_image_codec_as_artwork() {
        let json = ffprobe_json(
            r#"{"codec_type": "video", "codec_name": "png"}, {"codec_type": "audio", "codec_name": "mp3"}"#,
            "",
        );
        assert!(!parse_ffprobe(&json).expect("parsed").has_video);
    }

    #[test]
    fn parse_ffprobe_of_output_that_is_not_json_is_nothing() {
        assert!(parse_ffprobe("ffprobe: command not found").is_none());
    }

    #[test]
    fn parse_ffprobe_without_tags_or_duration_yields_no_metadata() {
        let json = r#"{"streams": [], "format": {"filename": "x"}}"#;
        let meta = parse_ffprobe(json).expect("parsed");
        assert_eq!(meta.title, None);
        assert_eq!(meta.artist, None);
        assert_eq!(meta.duration, None);
    }

    /// Tag keys are spelled inconsistently across containers — `TITLE` in
    /// Matroska, `title` in most others.
    #[test]
    fn parse_ffprobe_matches_tag_names_case_insensitively() {
        let json = ffprobe_json(
            r#"{"codec_type": "audio", "codec_name": "flac"}"#,
            r#""TITLE": "Shouted", "ARTIST": "Loud""#,
        );
        let meta = parse_ffprobe(&json).expect("parsed");
        assert_eq!(meta.title.as_deref(), Some("Shouted"));
        assert_eq!(meta.artist.as_deref(), Some("Loud"));
    }

    // ── resolve_meta ──────────────────────────────────────────────────────

    /// The no-ffprobe path: everything comes from the filename, and a zero
    /// duration is fine — auto-advance already copes with it.
    #[test]
    fn resolve_meta_without_a_probe_falls_back_to_the_filename() {
        let meta = resolve_meta(
            Path::new("/m/ARTBAT - Live at Ultra.mp4"),
            MediaKind::Video,
            None,
        );
        assert_eq!(meta.title, "Live at Ultra");
        assert_eq!(meta.artist, "ARTBAT");
        assert_eq!(meta.duration, 0);
        assert_eq!(meta.media, MediaKind::Video);
    }

    #[test]
    fn resolve_meta_prefers_the_probed_tags() {
        let json = ffprobe_json(
            r#"{"codec_type": "audio", "codec_name": "flac"}"#,
            r#""title": "Tagged Title", "artist": "Tagged Artist""#,
        );
        let meta = resolve_meta(
            Path::new("/m/whatever the file is called.flac"),
            MediaKind::Audio,
            parse_ffprobe(&json),
        );
        assert_eq!(meta.title, "Tagged Title");
        assert_eq!(meta.artist, "Tagged Artist");
        assert_eq!(meta.duration, 3529);
    }

    /// Tags are frequently half-filled: a title and no artist. The filename
    /// still has to cover what the tags leave out.
    #[test]
    fn resolve_meta_fills_only_what_the_probe_is_missing() {
        let json = ffprobe_json(
            r#"{"codec_type": "audio", "codec_name": "flac"}"#,
            r#""title": "Tagged Title""#,
        );
        let meta = resolve_meta(
            Path::new("/m/Filename Artist - Filename Title.flac"),
            MediaKind::Audio,
            parse_ffprobe(&json),
        );
        assert_eq!(meta.title, "Tagged Title");
        assert_eq!(meta.artist, "Filename Artist");
    }

    /// An empty tag is no tag — plenty of files carry `title=""`.
    #[test]
    fn resolve_meta_ignores_blank_tags() {
        let json = ffprobe_json(
            r#"{"codec_type": "audio", "codec_name": "flac"}"#,
            r#""title": "  ", "artist": """#,
        );
        let meta = resolve_meta(
            Path::new("/m/Real Artist - Real Title.flac"),
            MediaKind::Audio,
            parse_ffprobe(&json),
        );
        assert_eq!(meta.title, "Real Title");
        assert_eq!(meta.artist, "Real Artist");
    }

    /// ffprobe overrules the extension in both directions.
    #[test]
    fn resolve_meta_lets_the_probe_overrule_the_extension() {
        let audio_only = ffprobe_json(r#"{"codec_type": "audio", "codec_name": "opus"}"#, "");
        let meta = resolve_meta(
            Path::new("/m/audio-in-a-video-box.mkv"),
            MediaKind::Video,
            parse_ffprobe(&audio_only),
        );
        assert_eq!(meta.media, MediaKind::Audio);

        let with_video = ffprobe_json(r#"{"codec_type": "video", "codec_name": "h264"}"#, "");
        let meta = resolve_meta(
            Path::new("/m/video-in-an-audio-box.m4a"),
            MediaKind::Audio,
            parse_ffprobe(&with_video),
        );
        assert_eq!(meta.media, MediaKind::Video);
    }

    /// A file with no artist anywhere still needs a value — the track table has
    /// a column for it either way.
    #[test]
    fn resolve_meta_falls_back_to_a_placeholder_artist() {
        let meta = resolve_meta(Path::new("/m/no_artist_here.mp3"), MediaKind::Audio, None);
        assert_eq!(meta.title, "no_artist_here");
        assert!(!meta.artist.is_empty());
    }
}
