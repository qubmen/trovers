#[cfg(test)]
mod tests {
    use crate::library::{make_id, platform_id_of, source_slug, Library};
    use crate::playlist::{CacheStatus, Track};

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

    /// A playlist is an ordered id list, so resolution has to preserve that
    /// order — and tolerate an id whose document has gone missing rather than
    /// refuse to show the playlist at all.
    #[test]
    fn resolve_follows_the_given_order_and_skips_unknown_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut lib = Library::load(dir.path()).expect("load");
        for id in ["youtube:one", "youtube:two"] {
            let mut t = track_with_id(id);
            t.title = id.to_string();
            lib.upsert(t).expect("upsert");
        }

        let ids = vec![
            "youtube:two".to_string(),
            "youtube:gone".to_string(),
            "youtube:one".to_string(),
        ];
        let titles: Vec<&str> = lib.resolve(&ids).iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["youtube:two", "youtube:one"]);
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
}
