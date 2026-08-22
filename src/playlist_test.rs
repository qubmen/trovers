#[cfg(test)]
mod tests {
    use crate::playlist::{nested_order, LoopMode, Playlist, PlaylistEntry, PlaylistKind};
    use std::path::{Path, PathBuf};

    fn playlist(name: &str) -> Playlist {
        Playlist {
            name: name.to_string(),
            created: chrono::Utc::now(),
            loop_mode: LoopMode::None,
            shuffle: false,
            default_speed: None,
            tracks: Vec::new(),
            current_track: None,
            kind: PlaylistKind::Normal,
            parent: None,
            source_folder: None,
        }
    }

    /// An album belonging to `parent`, saved under `dir`.
    fn write_album(dir: &Path, name: &str, parent: &str) -> PathBuf {
        let mut pl = playlist(name);
        pl.kind = PlaylistKind::Album;
        pl.parent = Some(parent.to_string());
        let path = dir.join(format!("{name}.toml"));
        pl.save(&path).expect("save album");
        path
    }

    fn write_normal(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.toml"));
        playlist(name).save(&path).expect("save playlist");
        path
    }

    /// Name and depth of each row, which is the whole of what the sidebar needs.
    fn shape(entries: &[PlaylistEntry]) -> Vec<(&str, usize)> {
        nested_order(entries)
            .into_iter()
            .map(|(entry, depth)| (entry.name.as_str(), depth))
            .collect()
    }

    fn entry(name: &str, kind: PlaylistKind, parent: Option<&str>) -> PlaylistEntry {
        PlaylistEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/fake/{name}.toml")),
            kind,
            parent: parent.map(str::to_string),
        }
    }

    fn normal(name: &str) -> PlaylistEntry {
        entry(name, PlaylistKind::Normal, None)
    }

    fn album(name: &str, parent: &str) -> PlaylistEntry {
        entry(name, PlaylistKind::Album, Some(parent))
    }

    // ── the new fields ────────────────────────────────────────────────────

    /// Every playlist file written before albums existed has to keep loading as
    /// exactly what it is: a top-level playlist with no parent.
    #[test]
    fn a_playlist_file_without_the_album_fields_loads_as_a_normal_playlist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Live Sets.toml");
        std::fs::write(
            &path,
            r#"
name = "Live Sets"
created = "2026-04-01T12:06:59Z"
loop_mode = "none"
tracks = ["youtube:vK2io4J708A"]
"#,
        )
        .expect("write");

        let pl = Playlist::load(&path).expect("load");
        assert_eq!(pl.kind, PlaylistKind::Normal);
        assert_eq!(pl.parent, None);
        assert_eq!(pl.source_folder, None);
    }

    #[test]
    fn kind_parent_and_source_folder_survive_a_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Ultra 2026.toml");
        let mut pl = playlist("Ultra 2026");
        pl.kind = PlaylistKind::Album;
        pl.parent = Some("Live Sets".to_string());
        pl.source_folder = Some(PathBuf::from("/Users/den/Music/Ultra 2026"));
        pl.save(&path).expect("save");

        let loaded = Playlist::load(&path).expect("load");
        assert_eq!(loaded.kind, PlaylistKind::Album);
        assert_eq!(loaded.parent.as_deref(), Some("Live Sets"));
        assert_eq!(
            loaded.source_folder.as_deref(),
            Some(Path::new("/Users/den/Music/Ultra 2026"))
        );
    }

    // ── list_entries ──────────────────────────────────────────────────────

    #[test]
    fn list_entries_reads_kind_and_parent_from_each_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_normal(dir.path(), "Live Sets");
        write_album(dir.path(), "Ultra 2026", "Live Sets");

        let entries = Playlist::list_entries(dir.path()).expect("list");
        assert_eq!(entries.len(), 2);

        let album = entries
            .iter()
            .find(|e| e.name == "Ultra 2026")
            .expect("album");
        assert_eq!(album.kind, PlaylistKind::Album);
        assert_eq!(album.parent.as_deref(), Some("Live Sets"));

        let parent = entries
            .iter()
            .find(|e| e.name == "Live Sets")
            .expect("parent");
        assert_eq!(parent.kind, PlaylistKind::Normal);
        assert_eq!(parent.parent, None);
    }

    /// Sorted by name, because that is the order the sidebar and the `tab` cycle
    /// through playlists both rely on being stable.
    #[test]
    fn list_entries_is_sorted_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_normal(dir.path(), "Rock");
        write_normal(dir.path(), "Ambient");
        write_normal(dir.path(), "Jazz");

        let names: Vec<String> = Playlist::list_entries(dir.path())
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Ambient", "Jazz", "Rock"]);
    }

    /// The name is the filename, not the `name` field inside: everything else —
    /// rename, delete, the parent links — addresses a playlist by its file.
    #[test]
    fn list_entries_names_a_playlist_after_its_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pl = playlist("what the field says");
        pl.save(&dir.path().join("what the file says.toml"))
            .expect("save");

        let entries = Playlist::list_entries(dir.path()).expect("list");
        assert_eq!(entries[0].name, "what the file says");
    }

    #[test]
    fn list_entries_ignores_files_that_are_not_playlists() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_normal(dir.path(), "Live Sets");
        std::fs::write(dir.path().join("notes.txt"), b"hello").expect("write");
        // A write interrupted mid-rename leaves one of these behind.
        std::fs::write(dir.path().join("Live Sets.toml.tmp"), b"partial").expect("write");

        let names: Vec<String> = Playlist::list_entries(dir.path())
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Live Sets"]);
    }

    /// A file too broken to parse still gets a row, so the user can see it and
    /// rename or delete it — dropping it silently would look like data loss.
    #[test]
    fn list_entries_keeps_an_unparseable_playlist_as_a_normal_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Broken.toml"), b"this is not toml = = =").expect("write");

        let entries = Playlist::list_entries(dir.path()).expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Broken");
        assert_eq!(entries[0].kind, PlaylistKind::Normal);
        assert_eq!(entries[0].parent, None);
    }

    // ── nested_order ──────────────────────────────────────────────────────

    #[test]
    fn nested_order_puts_each_album_under_its_parent() {
        let entries = vec![
            album("Ultra 2026", "Live Sets"),
            normal("Ambient"),
            normal("Live Sets"),
        ];
        assert_eq!(
            shape(&entries),
            vec![("Ambient", 0), ("Live Sets", 0), ("Ultra 2026", 1)]
        );
    }

    #[test]
    fn nested_order_sorts_the_albums_under_one_parent_by_name() {
        let entries = vec![
            album("Zurich", "Live Sets"),
            album("Amsterdam", "Live Sets"),
            normal("Live Sets"),
        ];
        assert_eq!(
            shape(&entries),
            vec![("Live Sets", 0), ("Amsterdam", 1), ("Zurich", 1)]
        );
    }

    /// Deleting a playlist leaves its albums on disk. They must still be
    /// reachable — as top-level rows — rather than disappearing with the parent.
    #[test]
    fn nested_order_shows_an_orphaned_album_at_the_top_level() {
        let entries = vec![normal("Ambient"), album("Ultra 2026", "Gone")];
        assert_eq!(shape(&entries), vec![("Ambient", 0), ("Ultra 2026", 0)]);
    }

    /// Two levels only. An album naming another album as its parent would be a
    /// third, so it is treated as unparented instead of nested deeper.
    #[test]
    fn nested_order_does_not_nest_an_album_under_an_album() {
        let entries = vec![
            normal("Live Sets"),
            album("Ultra 2026", "Live Sets"),
            album("Day Two", "Ultra 2026"),
        ];
        assert_eq!(
            shape(&entries),
            vec![("Day Two", 0), ("Live Sets", 0), ("Ultra 2026", 1)]
        );
    }

    /// An album claiming itself as its parent cannot be placed under itself —
    /// and must not vanish or loop.
    #[test]
    fn nested_order_survives_an_album_that_is_its_own_parent() {
        let entries = vec![album("Ultra 2026", "Ultra 2026")];
        assert_eq!(shape(&entries), vec![("Ultra 2026", 0)]);
    }

    /// The invariant behind every case above: nesting reorders rows, it never
    /// adds or loses one.
    #[test]
    fn nested_order_lists_every_entry_exactly_once() {
        let entries = vec![
            album("Ultra 2026", "Live Sets"),
            normal("Live Sets"),
            album("Orphan", "Gone"),
            normal("Ambient"),
            album("Amsterdam", "Live Sets"),
        ];
        let mut names: Vec<&str> = shape(&entries).into_iter().map(|(name, _)| name).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["Ambient", "Amsterdam", "Live Sets", "Orphan", "Ultra 2026"]
        );
    }
}
