#[cfg(test)]
mod tests {
    use crate::library::{CacheStatus, Library, MediaKind, Track, TrackOrigin};
    use crate::library_import::{
        album_name_for_folder, expand_tilde, merge_scan, path_from_input, scan_and_probe,
        unique_album_name, ImportedFile,
    };
    use crate::library_scan::{local_id, ProbedMeta};
    use crate::playlist::{LoopMode, Playlist, PlaylistKind};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ── helpers ───────────────────────────────────────────────────────────

    /// An album linked to `root`, as `handle_task_msg` builds one before merging.
    fn album(root: &Path) -> Playlist {
        Playlist {
            name: "Ultra 2026".to_string(),
            created: chrono::Utc::now(),
            loop_mode: LoopMode::None,
            shuffle: false,
            default_speed: None,
            tracks: Vec::new(),
            current_track: None,
            kind: PlaylistKind::Album,
            parent: Some("Live Sets".to_string()),
            source_folder: Some(root.to_path_buf()),
            // A fresh import arrives open, so what it brought in is visible.
            collapsed: false,
        }
    }

    /// A scanned-and-probed file, with the metadata an ffprobe-less machine
    /// produces unless a test says otherwise.
    fn imported(path: &Path) -> ImportedFile {
        ImportedFile {
            path: path.to_path_buf(),
            meta: ProbedMeta {
                title: "Day One".to_string(),
                artist: "Miss Monique".to_string(),
                duration: 3529,
                media: MediaKind::Audio,
            },
        }
    }

    fn imported_with(path: &Path, title: &str, duration: u64, media: MediaKind) -> ImportedFile {
        ImportedFile {
            path: path.to_path_buf(),
            meta: ProbedMeta {
                title: title.to_string(),
                artist: "Miss Monique".to_string(),
                duration,
                media,
            },
        }
    }

    /// A remote track, for the rows a rescan must not touch.
    fn remote_track(id: &str) -> Track {
        Track {
            url: "https://www.youtube.com/watch?v=vK2io4J708A".to_string(),
            source: "youtube.com".to_string(),
            title: "A set".to_string(),
            artist: "Artist".to_string(),
            channel: "Channel".to_string(),
            duration: 100,
            id: id.to_string(),
            cache_status: CacheStatus::Cached,
            file: Some(PathBuf::from("/tmp/does-not-exist.opus")),
            last_position: 12,
            speed: None,
            user_title: None,
            user_artist: None,
            added_at: chrono::Utc::now(),
            origin: TrackOrigin::Remote,
            media: MediaKind::Audio,
            resume: true,
        }
    }

    /// Create `path` and every directory above it, with some bytes inside.
    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(path, b"bytes").expect("write");
    }

    // ── expand_tilde ──────────────────────────────────────────────────────

    #[test]
    fn expand_tilde_turns_a_bare_tilde_into_the_home_directory() {
        let home = Path::new("/Users/den");
        assert_eq!(expand_tilde("~", Some(home)), PathBuf::from("/Users/den"));
    }

    #[test]
    fn expand_tilde_rewrites_a_path_under_home() {
        let home = Path::new("/Users/den");
        assert_eq!(
            expand_tilde("~/Music/Ultra", Some(home)),
            PathBuf::from("/Users/den/Music/Ultra")
        );
    }

    #[test]
    fn expand_tilde_leaves_an_absolute_path_alone() {
        let home = Path::new("/Users/den");
        assert_eq!(
            expand_tilde("/Volumes/Sets/Ultra", Some(home)),
            PathBuf::from("/Volumes/Sets/Ultra")
        );
    }

    /// `~alice` is another user's home, which only the shell can resolve. Left
    /// as typed rather than mangled into a path under *this* user's home.
    #[test]
    fn expand_tilde_leaves_another_users_home_alone() {
        let home = Path::new("/Users/den");
        assert_eq!(
            expand_tilde("~alice/Music", Some(home)),
            PathBuf::from("~alice/Music")
        );
    }

    #[test]
    fn expand_tilde_leaves_the_tilde_when_there_is_no_home_directory() {
        assert_eq!(expand_tilde("~/Music", None), PathBuf::from("~/Music"));
    }

    // ── path_from_input: what the clipboard actually hands over ────────────

    #[test]
    fn a_file_url_becomes_the_path_it_points_at() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("file:///Users/den/Music/Ultra", Some(home)),
            PathBuf::from("/Users/den/Music/Ultra")
        );
    }

    /// The reported case: macOS puts a percent-encoded URL on the clipboard, so
    /// every Cyrillic letter and every space arrives as an escape.
    #[test]
    fn a_file_url_is_percent_decoded() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input(
                "file:///Users/den/Downloads/%D0%9A%D0%B8%D0%BD%D0%BE%20-%201988/",
                Some(home)
            ),
            PathBuf::from("/Users/den/Downloads/Кино - 1988")
        );
    }

    #[test]
    fn a_file_url_may_name_localhost_as_its_host() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("file://localhost/Users/den/Music/Ultra", Some(home)),
            PathBuf::from("/Users/den/Music/Ultra")
        );
    }

    /// The guard that keeps decoding safe: a `%` in a *path* is a literal `%`.
    /// Only a `file://` URL is percent-encoded by definition.
    #[test]
    fn a_plain_path_is_never_percent_decoded() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("/Users/den/Music/%D0%9A%20100%", Some(home)),
            PathBuf::from("/Users/den/Music/%D0%9A%20100%")
        );
    }

    /// A malformed escape is not an escape. `%` with nothing usable after it
    /// stays as it is rather than eating the next character or panicking.
    #[test]
    fn an_incomplete_escape_in_a_url_survives_decoding() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("file:///Users/den/Music/100%%2Fdone%2", Some(home)),
            PathBuf::from("/Users/den/Music/100%/done%2")
        );
    }

    /// Dragging a folder into a terminal escapes the spaces, and the escaping is
    /// the shell's, not part of the name.
    #[test]
    fn a_shell_escaped_space_is_unescaped() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("/Users/den/Music/Group\\ blood\\ \\(1988\\)", Some(home)),
            PathBuf::from("/Users/den/Music/Group blood (1988)")
        );
    }

    /// A backslash before something no shell escapes is part of the name, not
    /// escaping — unescaping everything would quietly rewrite it.
    #[test]
    fn a_backslash_that_is_part_of_a_name_survives() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("/Users/den/Music/AC\\DC", Some(home)),
            PathBuf::from("/Users/den/Music/AC\\DC")
        );
    }

    #[test]
    fn surrounding_quotes_are_not_part_of_the_path() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("'/Users/den/Music/Group blood'", Some(home)),
            PathBuf::from("/Users/den/Music/Group blood")
        );
        assert_eq!(
            path_from_input("\"/Users/den/Music/Group blood\"", Some(home)),
            PathBuf::from("/Users/den/Music/Group blood")
        );
    }

    #[test]
    fn a_tilde_still_expands_through_the_same_door() {
        let home = Path::new("/Users/den");
        assert_eq!(
            path_from_input("~/Music/Ultra", Some(home)),
            PathBuf::from("/Users/den/Music/Ultra")
        );
    }

    // ── naming the album ──────────────────────────────────────────────────

    #[test]
    fn album_name_for_folder_uses_the_folder_name() {
        assert_eq!(
            album_name_for_folder(Path::new("/Users/den/Music/Ultra 2026")),
            "Ultra 2026"
        );
    }

    /// A folder name is not a filename: macOS happily holds a `:` in one, and a
    /// playlist file cannot.
    #[test]
    fn album_name_for_folder_replaces_characters_a_filename_cannot_hold() {
        assert_eq!(
            album_name_for_folder(Path::new("/Users/den/Music/Live: 2026/")),
            "Live_ 2026"
        );
    }

    #[test]
    fn album_name_for_folder_falls_back_when_there_is_no_folder_name() {
        assert_eq!(album_name_for_folder(Path::new("/")), "Imported folder");
    }

    #[test]
    fn unique_album_name_keeps_a_free_name() {
        let taken = vec!["Live Sets".to_string()];
        assert_eq!(unique_album_name("Ultra 2026", &taken), "Ultra 2026");
    }

    #[test]
    fn unique_album_name_numbers_a_taken_name() {
        let taken = vec!["Ultra 2026".to_string()];
        assert_eq!(unique_album_name("Ultra 2026", &taken), "Ultra 2026 (2)");
    }

    #[test]
    fn unique_album_name_skips_past_the_numbers_already_taken() {
        let taken = vec![
            "Ultra 2026".to_string(),
            "Ultra 2026 (2)".to_string(),
            "Ultra 2026 (3)".to_string(),
        ];
        assert_eq!(unique_album_name("Ultra 2026", &taken), "Ultra 2026 (4)");
    }

    // ── merge_scan: the first import ──────────────────────────────────────

    #[test]
    fn merge_scan_lists_every_file_in_scan_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);

        let files = vec![
            imported(&root.join("01 Day One.flac")),
            imported(&root.join("02 Day Two.flac")),
        ];
        let report = merge_scan(&mut lib, &mut pl, &root, files);

        assert_eq!(
            pl.tracks,
            vec![
                local_id(&root.join("01 Day One.flac")),
                local_id(&root.join("02 Day Two.flac")),
            ]
        );
        assert_eq!((report.added, report.updated, report.missing), (2, 0, 0));
    }

    #[test]
    fn merge_scan_writes_a_local_track_document_for_each_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Miss Monique - Day One.mkv");

        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(&path, "Day One", 3529, MediaKind::Video)],
        );

        let track = lib.get(&local_id(&path)).expect("document written");
        assert_eq!(track.origin, TrackOrigin::Local);
        assert_eq!(track.media, MediaKind::Video);
        assert_eq!(track.cache_status, CacheStatus::Cached);
        assert_eq!(track.file.as_deref(), Some(path.as_path()));
        assert_eq!(track.url, path.to_string_lossy());
        assert_eq!(track.title, "Day One");
        assert_eq!(track.artist, "Miss Monique");
        assert_eq!(track.duration, 3529);
        // Recording the position is the default; a local file is no exception.
        assert!(track.resume);
    }

    /// The document has to be on disk, not merely in memory: the next launch
    /// reads the library off the filesystem.
    #[test]
    fn merge_scan_saves_each_document_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.flac");
        touch(&path);

        merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);

        let reloaded = Library::load(dir.path()).expect("reload");
        assert!(reloaded.get(&local_id(&path)).is_some());
    }

    /// Being linked to the folder is what makes the album rescannable.
    #[test]
    fn merge_scan_links_the_album_to_the_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        pl.source_folder = None;

        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported(&root.join("a.flac"))],
        );

        assert_eq!(pl.source_folder.as_deref(), Some(root.as_path()));
    }

    // ── merge_scan: rescanning ────────────────────────────────────────────

    #[test]
    fn rescanning_an_unchanged_folder_changes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.flac");
        merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);

        let report = merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);

        assert_eq!(pl.tracks, vec![local_id(&path)]);
        assert_eq!((report.added, report.updated, report.missing), (0, 0, 0));
    }

    /// The whole point of the path-derived id: everything the user built up on a
    /// row survives the folder being scanned again.
    #[test]
    fn rescanning_keeps_a_position_a_speed_and_a_renamed_title() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.flac");
        merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);

        let id = local_id(&path);
        let track = lib.get_mut(&id).expect("track");
        track.last_position = 176;
        track.speed = Some(1.5);
        track.user_title = Some("The good one".to_string());
        track.resume = false;

        // Deliberately a rescan that *does* rewrite the document — with identical
        // metadata nothing is written at all, so this is the case where losing the
        // user's own state is possible.
        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(
                &path,
                "Day One (remaster)",
                4000,
                MediaKind::Audio,
            )],
        );

        let track = lib.get(&id).expect("track");
        assert_eq!(track.last_position, 176);
        assert_eq!(track.speed, Some(1.5));
        assert_eq!(track.user_title.as_deref(), Some("The good one"));
        assert!(!track.resume);
        // What the file says about itself did move.
        assert_eq!(track.title, "Day One (remaster)");
    }

    #[test]
    fn rescanning_appends_a_new_file_without_reordering_the_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let second = root.join("02 Day Two.flac");
        merge_scan(&mut lib, &mut pl, &root, vec![imported(&second)]);

        // Sorts *before* the row already there, and must still land after it.
        let first = root.join("01 Day One.flac");
        let report = merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported(&first), imported(&second)],
        );

        assert_eq!(pl.tracks, vec![local_id(&second), local_id(&first)]);
        assert_eq!(report.added, 1);
    }

    #[test]
    fn rescanning_marks_a_vanished_file_missing_and_keeps_its_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let gone = root.join("Day One.flac");
        let kept = root.join("Day Two.flac");
        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported(&gone), imported(&kept)],
        );

        let report = merge_scan(&mut lib, &mut pl, &root, vec![imported(&kept)]);

        assert_eq!(pl.tracks, vec![local_id(&gone), local_id(&kept)]);
        assert_eq!(
            lib.get(&local_id(&gone)).expect("row kept").cache_status,
            CacheStatus::Missing
        );
        // The path is kept, which is what lets the row heal itself later.
        assert_eq!(
            lib.get(&local_id(&gone)).expect("row kept").file.as_deref(),
            Some(gone.as_path())
        );
        assert_eq!((report.added, report.updated, report.missing), (0, 0, 1));
    }

    #[test]
    fn rescanning_heals_a_file_that_came_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.flac");
        merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);
        merge_scan(&mut lib, &mut pl, &root, vec![]);

        let report = merge_scan(&mut lib, &mut pl, &root, vec![imported(&path)]);

        assert_eq!(
            lib.get(&local_id(&path)).expect("track").cache_status,
            CacheStatus::Cached
        );
        assert_eq!((report.added, report.updated, report.missing), (0, 1, 0));
    }

    /// A rescan reports on the folder, so a track added from a URL is none of its
    /// business — however long the album has held it.
    #[test]
    fn rescanning_leaves_a_remote_row_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        lib.upsert(remote_track("youtube:vK2io4J708A"))
            .expect("upsert");
        pl.tracks.push("youtube:vK2io4J708A".to_string());

        let report = merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported(&root.join("a.flac"))],
        );

        assert_eq!(
            lib.get("youtube:vK2io4J708A")
                .expect("remote row")
                .cache_status,
            CacheStatus::Cached
        );
        assert_eq!(report.missing, 0);
    }

    /// A local file the user dropped in by hand from somewhere else. This folder's
    /// scan says nothing about whether *that* file is still there.
    #[test]
    fn rescanning_leaves_a_local_row_from_outside_the_folder_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let elsewhere = dir.path().join("Other").join("stray.flac");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let mut other = album(elsewhere.parent().expect("parent"));
        merge_scan(&mut lib, &mut other, &elsewhere, vec![imported(&elsewhere)]);
        pl.tracks.push(local_id(&elsewhere));

        let report = merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported(&root.join("a.flac"))],
        );

        assert_eq!(
            lib.get(&local_id(&elsewhere)).expect("stray").cache_status,
            CacheStatus::Cached
        );
        assert_eq!(report.missing, 0);
    }

    /// Imported without ffprobe, rescanned with it: the facts about the file are
    /// the file's to state, so the row picks up its real duration.
    #[test]
    fn rescanning_refreshes_a_duration_the_first_import_could_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.flac");
        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(&path, "Day One", 0, MediaKind::Audio)],
        );

        let report = merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(&path, "Day One", 3529, MediaKind::Audio)],
        );

        assert_eq!(lib.get(&local_id(&path)).expect("track").duration, 3529);
        assert_eq!((report.added, report.updated, report.missing), (0, 1, 0));
    }

    /// An `.mkv` holding nothing but audio needs no video window. Extension-based
    /// guessing cannot know that, so a rescan with ffprobe around has to be able
    /// to correct it — in both directions.
    #[test]
    fn rescanning_corrects_the_media_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("Ultra 2026");
        let mut lib = Library::load(dir.path()).expect("library");
        let mut pl = album(&root);
        let path = root.join("Day One.mkv");
        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(&path, "Day One", 3529, MediaKind::Video)],
        );

        merge_scan(
            &mut lib,
            &mut pl,
            &root,
            vec![imported_with(&path, "Day One", 3529, MediaKind::Audio)],
        );

        assert_eq!(
            lib.get(&local_id(&path)).expect("track").media,
            MediaKind::Audio
        );
    }

    // ── scan_and_probe ────────────────────────────────────────────────────

    /// Without ffprobe every title comes from the filename; with it, these empty
    /// files are unreadable and it says so, which lands in the same place. Either
    /// way the order is the scan's order and every file is reported once.
    #[tokio::test]
    async fn scan_and_probe_returns_every_file_in_scan_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("02 Day Two.flac"));
        touch(&dir.path().join("01 Day One.flac"));
        touch(&dir.path().join("cover.jpg"));

        let files = scan_and_probe(dir.path(), |_, _| {}).await;

        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["01 Day One.flac", "02 Day Two.flac"]);
        // `01 ` with no punctuation after it is part of the name, not a track
        // number — see `strip_track_number_prefix`.
        assert_eq!(files[0].meta.title, "01 Day One");
    }

    #[tokio::test]
    async fn scan_and_probe_reports_progress_once_per_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("a.flac"));
        touch(&dir.path().join("b.flac"));
        touch(&dir.path().join("c.flac"));

        let calls = AtomicUsize::new(0);
        let last = AtomicUsize::new(0);
        let files = scan_and_probe(dir.path(), |done, total| {
            calls.fetch_add(1, Ordering::Relaxed);
            last.store(done * 100 + total, Ordering::Relaxed);
        })
        .await;

        assert_eq!(files.len(), 3);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        // The last report is the finished one: 3 of 3.
        assert_eq!(last.load(Ordering::Relaxed), 303);
    }

    #[tokio::test]
    async fn scan_and_probe_on_a_folder_with_no_media_returns_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("notes.txt"));

        let files = scan_and_probe(dir.path(), |_, _| {}).await;

        assert!(files.is_empty());
    }
}
