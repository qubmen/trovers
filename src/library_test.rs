#[cfg(test)]
mod tests {
    use crate::library::{
        make_id, platform_id_of, source_slug, CacheStatus, Library, MediaKind, Track, TrackOrigin,
    };

    fn track_with_id(id: &str) -> Track {
        Track {
            url: "https://example.com/x".to_string(),
            source: "youtube.com".to_string(),
            title: "Title".to_string(),
            artist: "Artist".to_string(),
            channel: "Channel".to_string(),
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

    // ── source_slug ───────────────────────────────────────────────────────

    #[test]
    fn source_slug_strips_tld() {
        assert_eq!(source_slug("youtube.com"), "youtube");
    }

    /// The reason the slug is the *registrable* label rather than the whole
    /// domain: the same video reached via music.youtube.com must not become a
    /// second document with its own independent playback position.
    #[test]
    fn source_slug_ignores_subdomains() {
        assert_eq!(source_slug("music.youtube.com"), "youtube");
        assert_eq!(source_slug("www.bandcamp.com"), "bandcamp");
    }

    #[test]
    fn source_slug_keeps_a_single_label_as_is() {
        assert_eq!(source_slug("local"), "local");
    }

    #[test]
    fn source_slug_falls_back_when_source_is_blank() {
        assert_eq!(source_slug(""), "unknown");
    }

    /// Host names are case-insensitive, so a URL spelled `YouTube.com` must not
    /// mint a second document alongside the `youtube.com` one.
    #[test]
    fn source_slug_is_case_insensitive() {
        assert_eq!(source_slug("YouTube.com"), "youtube");
    }

    // ── make_id ───────────────────────────────────────────────────────────

    #[test]
    fn make_id_joins_slug_and_platform_id() {
        assert_eq!(make_id("youtube.com", "vK2io4J708A"), "youtube:vK2io4J708A");
    }

    #[test]
    fn make_id_is_stable_across_host_spellings() {
        assert_eq!(
            make_id("music.youtube.com", "vK2io4J708A"),
            make_id("youtube.com", "vK2io4J708A")
        );
    }

    // ── platform_id_of ────────────────────────────────────────────────────

    #[test]
    fn platform_id_of_takes_everything_after_the_first_colon() {
        assert_eq!(platform_id_of("youtube:vK2io4J708A"), "vK2io4J708A");
    }

    /// Platform ids are opaque — yt-dlp mints them per site and some contain
    /// colons. Splitting on the *first* colon keeps those intact.
    #[test]
    fn platform_id_of_keeps_colons_inside_the_platform_id() {
        assert_eq!(platform_id_of("mixcloud:user:set-name"), "user:set-name");
    }

    #[test]
    fn platform_id_of_returns_a_bare_id_unchanged() {
        assert_eq!(platform_id_of("vK2io4J708A"), "vK2io4J708A");
    }

    // ── Track::platform_id ────────────────────────────────────────────────

    /// What the audio cache and yt-dlp are keyed by — the platform's own id,
    /// never the library id. Deriving it means there is no second field to fall
    /// out of step with `id`.
    #[test]
    fn track_platform_id_strips_the_slug() {
        assert_eq!(
            track_with_id("youtube:vK2io4J708A").platform_id(),
            "vK2io4J708A"
        );
    }

    #[test]
    fn track_platform_id_of_an_unmigrated_bare_id_is_the_id() {
        assert_eq!(track_with_id("vK2io4J708A").platform_id(), "vK2io4J708A");
    }

    // ── Library ───────────────────────────────────────────────────────────

    /// Write `track` into a library rooted at `root` and drop the library, so
    /// the assertions that follow read only what actually reached disk.
    fn write_track(root: &std::path::Path, track: Track) {
        let mut lib = Library::load(root).expect("load");
        lib.upsert(track).expect("upsert");
    }

    /// The `.toml` documents in `root`, sorted — the library's whole on-disk
    /// footprint.
    fn documents_in(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(root)
            .expect("read_dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn load_of_a_directory_that_does_not_exist_yet_is_an_empty_library() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lib = Library::load(&dir.path().join("tracks")).expect("load");
        assert!(lib.is_empty());
    }

    #[test]
    fn upsert_then_get_returns_the_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        lib.upsert(track_with_id("youtube:abc")).expect("upsert");
        assert_eq!(
            lib.get("youtube:abc").map(|t| t.title.as_str()),
            Some("Title")
        );
    }

    #[test]
    fn a_track_survives_a_round_trip_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = track_with_id("youtube:abc");
        track.last_position = 176;
        track.speed = Some(1.5);
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("youtube:abc").expect("track present");
        assert_eq!(got.last_position, 176);
        assert_eq!(got.speed, Some(1.5));
    }

    /// The filename is only a hint — macOS folds case, YouTube ids do not, so a
    /// document has to be found by the `id` written inside it.
    #[test]
    fn load_indexes_by_the_id_inside_the_document_not_the_filename() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_track(dir.path(), track_with_id("youtube:abc"));

        // Rename the file to something unrelated; the id must still resolve.
        let written = documents_in(dir.path());
        assert_eq!(written.len(), 1, "one track, one document");
        std::fs::rename(&written[0], dir.path().join("something-else.toml")).expect("rename");

        let reloaded = Library::load(dir.path()).expect("reload");
        assert!(reloaded.get("youtube:abc").is_some());
    }

    /// `speed`, `user_title`, `user_artist` and `file` are absent from a document
    /// whenever they are `None`, so a hand-written or freshly-migrated document
    /// carries none of them. Loading must not need them.
    #[test]
    fn a_document_without_the_optional_fields_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path()).expect("mkdir");
        std::fs::write(
            dir.path().join("bandcamp-min001.toml"),
            r#"
id = "bandcamp:min001"
url = "https://example.com/minimal"
source = "bandcamp.com"
title = "Minimal Track"
artist = "Minimal Artist"
channel = "MinChannel"
duration = 120
cache_status = "streaming"
last_position = 0
added_at = "2025-06-01T08:00:00Z"
"#,
        )
        .expect("write");

        let lib = Library::load(dir.path()).expect("load");
        let got = lib.get("bandcamp:min001").expect("track present");
        assert_eq!(got.speed, None);
        assert_eq!(got.user_title, None);
        assert_eq!(got.user_artist, None);
        assert_eq!(got.file, None);
    }

    #[test]
    fn load_skips_an_unparseable_document_and_keeps_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_track(dir.path(), track_with_id("youtube:good"));
        std::fs::write(dir.path().join("broken.toml"), "this is not toml {{{").expect("write");

        let reloaded = Library::load(dir.path()).expect("reload");
        assert!(reloaded.get("youtube:good").is_some());
        assert_eq!(reloaded.len(), 1);
    }

    /// Two ids differing only in case want the same filename on a
    /// case-insensitive filesystem. The second must land on its own file rather
    /// than overwrite the first.
    #[test]
    fn ids_differing_only_in_case_get_separate_documents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        lib.upsert(track_with_id("youtube:abc"))
            .expect("upsert lower");
        let mut upper = track_with_id("youtube:ABC");
        upper.title = "Upper".to_string();
        lib.upsert(upper).expect("upsert upper");

        assert_eq!(documents_in(dir.path()).len(), 2, "two ids, two documents");

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").map(|t| t.title.as_str()),
            Some("Title")
        );
        assert_eq!(
            reloaded.get("youtube:ABC").map(|t| t.title.as_str()),
            Some("Upper")
        );
    }

    /// Re-upserting an id must reuse its document rather than pile up `-2`,
    /// `-3`, ... copies beside it.
    #[test]
    fn upserting_the_same_id_twice_reuses_one_document() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        lib.upsert(track_with_id("youtube:abc")).expect("first");
        let mut again = track_with_id("youtube:abc");
        again.title = "Renamed".to_string();
        lib.upsert(again).expect("second");

        assert_eq!(documents_in(dir.path()).len(), 1);
        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").map(|t| t.title.as_str()),
            Some("Renamed")
        );
    }

    #[test]
    fn save_persists_a_mutation_made_through_get_mut() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        lib.upsert(track_with_id("youtube:abc")).expect("upsert");

        lib.get_mut("youtube:abc").expect("present").last_position = 42;
        lib.save("youtube:abc").expect("save");

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").expect("present").last_position,
            42
        );
    }

    #[test]
    fn remove_deletes_the_document_and_returns_the_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        lib.upsert(track_with_id("youtube:abc")).expect("upsert");

        let removed = lib.remove("youtube:abc").expect("remove");
        assert_eq!(removed.map(|t| t.id), Some("youtube:abc".to_string()));
        assert!(
            documents_in(dir.path()).is_empty(),
            "the document file must be gone"
        );
        assert!(lib.get("youtube:abc").is_none());
    }

    #[test]
    fn remove_of_an_unknown_id_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        assert!(lib.remove("youtube:nope").expect("remove").is_none());
    }

    // ── Library::load state repair ────────────────────────────────────────

    /// Crash recovery: nothing is downloading at startup, so a document left
    /// mid-download must not keep claiming it is.
    #[test]
    fn load_resets_a_downloading_track_to_streaming() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = track_with_id("youtube:abc");
        track.cache_status = CacheStatus::Downloading;
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").expect("present").cache_status,
            CacheStatus::Streaming
        );
    }

    #[test]
    fn load_downgrades_a_cached_track_whose_file_is_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = track_with_id("youtube:abc");
        track.cache_status = CacheStatus::Cached;
        track.file = Some(dir.path().join("never-written.opus"));
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("youtube:abc").expect("present");
        assert_eq!(got.cache_status, CacheStatus::Streaming);
        assert!(got.file.is_none(), "the dangling path must be cleared too");
    }

    #[test]
    fn load_keeps_a_cached_track_whose_file_is_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join("abc.opus");
        std::fs::write(&audio, b"audio").expect("write audio");
        let mut track = track_with_id("youtube:abc");
        track.cache_status = CacheStatus::Cached;
        track.file = Some(audio.clone());
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("youtube:abc").expect("present");
        assert_eq!(got.cache_status, CacheStatus::Cached);
        assert_eq!(got.file.as_deref(), Some(audio.as_path()));
    }

    /// `failed` is a real terminal state, not a crash artifact — it must survive
    /// a reload so a track nobody could cache stays distinguishable from one
    /// nobody has tried.
    #[test]
    fn load_does_not_reset_a_failed_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = track_with_id("youtube:abc");
        track.cache_status = CacheStatus::Failed;
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").expect("present").cache_status,
            CacheStatus::Failed
        );
    }

    // ── local tracks: origin, media kind, resume ──────────────────────────

    /// A local track: the user's own file, played from where it already sits.
    fn local_track(id: &str, file: &std::path::Path) -> Track {
        Track {
            url: file.to_string_lossy().to_string(),
            source: "local".to_string(),
            origin: TrackOrigin::Local,
            cache_status: CacheStatus::Cached,
            file: Some(file.to_path_buf()),
            ..track_with_id(id)
        }
    }

    /// Every document written before local media existed carries none of the three
    /// new fields, and must keep loading as exactly what it was: a remote audio
    /// track that resumes where it was left.
    #[test]
    fn a_document_without_the_new_fields_is_a_remote_resumable_audio_track() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path()).expect("mkdir");
        std::fs::write(
            dir.path().join("bandcamp-min001.toml"),
            r#"
id = "bandcamp:min001"
url = "https://example.com/minimal"
source = "bandcamp.com"
title = "Minimal Track"
artist = "Minimal Artist"
channel = "MinChannel"
duration = 120
cache_status = "streaming"
last_position = 0
added_at = "2025-06-01T08:00:00Z"
"#,
        )
        .expect("write");

        let lib = Library::load(dir.path()).expect("load");
        let got = lib.get("bandcamp:min001").expect("track present");
        assert_eq!(got.origin, TrackOrigin::Remote);
        assert_eq!(got.media, MediaKind::Audio);
        assert!(got.resume, "resuming is the default, not opting in");
    }

    #[test]
    fn origin_media_and_resume_survive_a_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let video = dir.path().join("clip.mkv");
        std::fs::write(&video, b"video").expect("write video");

        let mut track = local_track("local:deadbeef", &video);
        track.media = MediaKind::Video;
        track.resume = false;
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("local:deadbeef").expect("present");
        assert_eq!(got.origin, TrackOrigin::Local);
        assert_eq!(got.media, MediaKind::Video);
        assert!(!got.resume);
    }

    /// An unplugged drive or a file moved behind trovers' back. The row stays —
    /// `Missing` is what lets the UI say so instead of silently dropping it.
    #[test]
    fn load_marks_a_local_track_whose_file_is_gone_as_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let track = local_track("local:deadbeef", &dir.path().join("gone.flac"));
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("local:deadbeef").expect("present");
        assert_eq!(got.cache_status, CacheStatus::Missing);
        assert!(
            got.file.is_some(),
            "the path must be kept — it is how the row heals when the drive is back"
        );
    }

    /// The other half of the same behaviour: remounting the drive is all it takes.
    #[test]
    fn load_heals_a_missing_local_track_once_its_file_is_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join("back-again.flac");
        let mut track = local_track("local:deadbeef", &audio);
        track.cache_status = CacheStatus::Missing;
        write_track(dir.path(), track);
        std::fs::write(&audio, b"audio").expect("write audio");

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded
                .get("local:deadbeef")
                .expect("present")
                .cache_status,
            CacheStatus::Cached
        );
    }

    #[test]
    fn load_keeps_a_local_track_whose_file_is_present_cached() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio = dir.path().join("present.flac");
        std::fs::write(&audio, b"audio").expect("write audio");
        write_track(dir.path(), local_track("local:deadbeef", &audio));

        let reloaded = Library::load(dir.path()).expect("reload");
        let got = reloaded.get("local:deadbeef").expect("present");
        assert_eq!(got.cache_status, CacheStatus::Cached);
        assert_eq!(got.file.as_deref(), Some(audio.as_path()));
    }

    /// A local track's file is the only copy there is. Downgrading it to
    /// `Streaming` the way a remote row is downgraded would promise a stream that
    /// cannot exist, so a local row with no file is `Missing`.
    #[test]
    fn load_never_downgrades_a_local_track_to_streaming() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = local_track("local:deadbeef", &dir.path().join("gone.flac"));
        track.file = None;
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded
                .get("local:deadbeef")
                .expect("present")
                .cache_status,
            CacheStatus::Missing
        );
    }

    /// `Missing` says "the only copy is gone", which is never true of a remote
    /// track — that one can always be streamed again.
    #[test]
    fn load_turns_a_missing_remote_track_back_into_streaming() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut track = track_with_id("youtube:abc");
        track.cache_status = CacheStatus::Missing;
        write_track(dir.path(), track);

        let reloaded = Library::load(dir.path()).expect("reload");
        assert_eq!(
            reloaded.get("youtube:abc").expect("present").cache_status,
            CacheStatus::Streaming
        );
    }

    // ── migrate ───────────────────────────────────────────────────────────

    /// A `playlists/` and a `tracks/` directory side by side, as `main.rs` hands
    /// them to `migrate`.
    fn migration_dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let playlists = dir.path().join("playlists");
        let tracks = dir.path().join("tracks");
        std::fs::create_dir_all(&playlists).expect("mkdir playlists");
        (dir, playlists, tracks)
    }

    /// A playlist file in the old format: track data embedded in `[[tracks]]`,
    /// each row carrying its own `video_id`.
    fn write_legacy_playlist(playlists: &std::path::Path, name: &str, video_ids: &[&str]) {
        let mut raw = format!(
            "name = \"{name}\"\n\
             created = \"2025-06-01T08:00:00Z\"\n\
             loop_mode = \"playlist\"\n\
             shuffle = true\n\
             current_track = \"{}\"\n",
            video_ids.first().copied().unwrap_or("")
        );
        for (i, video_id) in video_ids.iter().enumerate() {
            raw.push_str(&format!(
                "\n[[tracks]]\n\
                 url = \"https://www.youtube.com/watch?v={video_id}\"\n\
                 source = \"youtube.com\"\n\
                 title = \"Track {video_id}\"\n\
                 artist = \"Artist\"\n\
                 channel = \"Channel\"\n\
                 duration = 100\n\
                 video_id = \"{video_id}\"\n\
                 cache_status = \"streaming\"\n\
                 last_position = {}\n\
                 added_at = \"2025-06-01T08:00:00Z\"\n",
                (i as u64 + 1) * 10
            ));
        }
        std::fs::write(playlists.join(format!("{name}.toml")), raw).expect("write legacy playlist");
    }

    /// The single `playlists.backup-*` directory beside `playlists`.
    fn backup_dir_beside(playlists: &std::path::Path) -> std::path::PathBuf {
        let parent = playlists.parent().expect("parent");
        let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(parent)
            .expect("read parent")
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("playlists.backup-"))
            })
            .collect();
        assert_eq!(found.len(), 1, "exactly one backup: {found:?}");
        found.remove(0)
    }

    #[test]
    fn migrate_rewrites_a_legacy_playlist_as_ids_and_writes_the_documents() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Live Sets", &["vK2io4J708A", "abc123"]);

        let report = crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("something to migrate");
        assert_eq!(report.playlists, 1);
        assert_eq!(report.tracks, 2);

        let pl = crate::playlist::Playlist::load(&playlists.join("Live Sets.toml")).expect("load");
        assert_eq!(
            pl.tracks,
            vec![
                "youtube:vK2io4J708A".to_string(),
                "youtube:abc123".to_string()
            ],
            "the running order must survive verbatim"
        );
        assert_eq!(
            pl.current_track.as_deref(),
            Some("youtube:vK2io4J708A"),
            "the cursor pointer was a bare video id and has to become a library id"
        );
        assert_eq!(pl.name, "Live Sets");
        assert!(pl.shuffle, "the playlist's own settings must be preserved");
        assert_eq!(pl.loop_mode, crate::playlist::LoopMode::Playlist);

        let lib = Library::load(&tracks).expect("load library");
        let first = lib.get("youtube:vK2io4J708A").expect("first document");
        assert_eq!(first.title, "Track vK2io4J708A");
        assert_eq!(
            first.last_position, 10,
            "a track's playback position is the whole point of keeping the data"
        );
        assert_eq!(lib.get("youtube:abc123").map(|t| t.last_position), Some(20));
    }

    /// The one that decides whether this is safe to run on every launch.
    #[test]
    fn migrate_is_nothing_to_do_the_second_time() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Live Sets", &["vK2io4J708A"]);

        crate::library::migrate(&playlists, &tracks).expect("first migrate");
        let after_first = std::fs::read_to_string(playlists.join("Live Sets.toml")).expect("read");

        let second = crate::library::migrate(&playlists, &tracks).expect("second migrate");
        assert!(second.is_none(), "an id-list playlist is not legacy");
        assert_eq!(
            std::fs::read_to_string(playlists.join("Live Sets.toml")).expect("read"),
            after_first,
            "the second run must not touch the file"
        );
        backup_dir_beside(&playlists); // asserts there is still exactly one
    }

    #[test]
    fn migrate_leaves_an_already_migrated_playlist_untouched() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Legacy", &["vK2io4J708A"]);
        let modern_raw = "name = \"Modern\"\n\
                          created = \"2025-06-01T08:00:00Z\"\n\
                          loop_mode = \"none\"\n\
                          shuffle = false\n\
                          tracks = [\"youtube:zzz\"]\n";
        std::fs::write(playlists.join("Modern.toml"), modern_raw).expect("write modern");

        let report = crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("the legacy one still needs migrating");
        assert_eq!(report.playlists, 1, "only the legacy playlist counts");

        assert_eq!(
            std::fs::read_to_string(playlists.join("Modern.toml")).expect("read"),
            modern_raw,
            "a playlist already holding ids must be left byte-for-byte alone"
        );
        assert_eq!(
            crate::playlist::Playlist::load(&playlists.join("Legacy.toml"))
                .expect("load")
                .tracks,
            vec!["youtube:vK2io4J708A".to_string()]
        );
    }

    /// The backup exists so a migration that goes wrong is recoverable, which
    /// means it has to hold the *original* files, not the rewritten ones.
    #[test]
    fn migrate_backs_up_the_playlists_before_rewriting_them() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Live Sets", &["vK2io4J708A"]);
        let original = std::fs::read_to_string(playlists.join("Live Sets.toml")).expect("read");

        let report = crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("something to migrate");

        let backup = backup_dir_beside(&playlists);
        assert_eq!(report.backup, backup);
        assert_eq!(
            std::fs::read_to_string(backup.join("Live Sets.toml")).expect("read backup"),
            original,
            "the backup must hold the embedded-track original"
        );
    }

    /// The quirk the whole model change exists to fix: one video in two
    /// playlists was two independent copies with two independent positions.
    #[test]
    fn migrate_gives_two_playlists_sharing_a_video_one_document() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "A", &["vK2io4J708A"]);
        write_legacy_playlist(&playlists, "B", &["vK2io4J708A"]);

        let report = crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("something to migrate");
        assert_eq!(report.playlists, 2);
        assert_eq!(report.tracks, 1, "the second sighting is not a new track");

        assert_eq!(documents_in(&tracks).len(), 1);
        for name in ["A", "B"] {
            assert_eq!(
                crate::playlist::Playlist::load(&playlists.join(format!("{name}.toml")))
                    .expect("load")
                    .tracks,
                vec!["youtube:vK2io4J708A".to_string()],
                "both playlists must reference the one document"
            );
        }
    }

    #[test]
    fn migrate_with_no_playlists_dir_is_nothing_to_do() {
        let dir = tempfile::tempdir().expect("tempdir");
        let migrated =
            crate::library::migrate(&dir.path().join("playlists"), &dir.path().join("tracks"))
                .expect("migrate");
        assert!(migrated.is_none());
    }

    #[test]
    fn migrate_of_an_empty_playlists_dir_is_nothing_to_do() {
        let (_dir, playlists, tracks) = migration_dirs();
        assert!(crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .is_none());
    }

    /// An unparseable playlist is left exactly as it is: migration cannot know
    /// what it meant, and refusing to launch over it would be worse.
    #[test]
    fn migrate_skips_an_unparseable_playlist_and_migrates_the_rest() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Good", &["vK2io4J708A"]);
        std::fs::write(playlists.join("Broken.toml"), "this is not toml {{{").expect("write");

        crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("the good one still migrates");

        assert_eq!(
            std::fs::read_to_string(playlists.join("Broken.toml")).expect("read"),
            "this is not toml {{{"
        );
        assert_eq!(
            crate::playlist::Playlist::load(&playlists.join("Good.toml"))
                .expect("load")
                .tracks,
            vec!["youtube:vK2io4J708A".to_string()]
        );
    }

    /// A legacy `current_track` naming a video the playlist does not list is
    /// stale; carrying it over as an id would leave a cursor pointing nowhere.
    #[test]
    fn migrate_drops_a_current_track_that_names_no_row() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Live Sets", &["vK2io4J708A"]);
        let raw = std::fs::read_to_string(playlists.join("Live Sets.toml")).expect("read");
        std::fs::write(
            playlists.join("Live Sets.toml"),
            raw.replace(
                "current_track = \"vK2io4J708A\"",
                "current_track = \"gone\"",
            ),
        )
        .expect("write");

        crate::library::migrate(&playlists, &tracks).expect("migrate");

        assert_eq!(
            crate::playlist::Playlist::load(&playlists.join("Live Sets.toml"))
                .expect("load")
                .current_track,
            None
        );
    }

    /// A `tracks = []` playlist is indistinguishable from a migrated one, and
    /// there is nothing in it to migrate either way.
    #[test]
    fn migrate_treats_an_empty_track_list_as_already_migrated() {
        let (_dir, playlists, tracks) = migration_dirs();
        write_legacy_playlist(&playlists, "Empty", &[]);

        assert!(crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .is_none());
    }

    /// Re-running migration must not mint a second document for a track that
    /// already has one — a playlist restored from the backup, say.
    #[test]
    fn migrate_reuses_an_existing_document_rather_than_duplicating_it() {
        let (_dir, playlists, tracks) = migration_dirs();
        let mut lib = Library::load(&tracks).expect("load");
        let mut existing = track_with_id("youtube:vK2io4J708A");
        existing.last_position = 999;
        lib.upsert(existing).expect("upsert");

        write_legacy_playlist(&playlists, "Live Sets", &["vK2io4J708A"]);
        let report = crate::library::migrate(&playlists, &tracks)
            .expect("migrate")
            .expect("the playlist is still legacy");
        assert_eq!(report.tracks, 0, "no new document was needed");

        assert_eq!(documents_in(&tracks).len(), 1);
        assert_eq!(
            Library::load(&tracks)
                .expect("reload")
                .get("youtube:vK2io4J708A")
                .map(|t| t.last_position),
            Some(999),
            "first writer wins — the existing document keeps its state"
        );
    }
}
