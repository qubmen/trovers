# Albums in the Track List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show each album as a collapsible group inside its parent's track list
instead of an indented row in the 22-column sidebar.

**Architecture:** `App::track_index_at(cursor) -> Option<usize>` is replaced by a
computed `Vec<VisibleRow>`, where a row knows which playlist file it came from and
which index in that file it is. Albums are loaded into `App::albums` when the
displayed playlist changes; each is still its own TOML file. Playback of any row
goes through one `play_from_list(path, playlist, index, start_pos)`.

**Tech Stack:** Rust 2021, ratatui 0.30, crossterm 0.29, serde/toml, tokio.

**Spec:** `docs/superpowers/specs/2026-08-22-albums-in-the-track-list-design.md`

## Global Constraints

- **English only** in code, comments, docs, commit messages, error and status
  strings — AGENTS.md "Language Policy". The chat language is irrelevant.
- Binary crate, no lib target: run `cargo test`, never `cargo test --lib`.
- TDD throughout: a failing test (or a compile error, which is Rust's type-level
  RED) before every implementation step.
- Atomic TOML writes only, via `Playlist::save` — never `fs::write` to a live path.
- **A user's own file is never deleted or moved** (ADR-018). Deleting an album
  deletes its playlist file and nothing else.
- Two levels of nesting, never three (ADR-016).
- `cargo fmt` clean and no new clippy warnings before each commit.

---

### Task 1: `collapsed` on `Playlist`, and the sidebar stops listing albums

**Files:**
- Modify: `src/playlist.rs` (add `collapsed`; replace `nested_order` with
  `sidebar_entries`)
- Modify: `src/tui/mod.rs:56-75` (`SidebarItem::Playlist` loses `depth`),
  `src/tui/mod.rs:964-985` (`sidebar_items`)
- Modify: `src/tui/ui.rs:147-173` (drop the indent)
- Test: `src/playlist_test.rs`, `src/tui/ui_test.rs:3120-3130`

**Interfaces:**
- Produces: `Playlist.collapsed: bool` (serde default `true`);
  `playlist::sidebar_entries(&[PlaylistEntry]) -> Vec<&PlaylistEntry>`;
  `SidebarItem::Playlist { name, path, is_album }`.

- [ ] **Step 1: Write the failing tests** in `src/playlist_test.rs`

```rust
#[test]
fn a_playlist_without_a_collapsed_key_loads_folded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Album.toml");
    std::fs::write(
        &path,
        "name = \"Album\"\ncreated = \"2026-01-01T00:00:00Z\"\nloop_mode = \"none\"\n\
         tracks = []\ncurrent_track = []\n",
    )
    .expect("write");
    // `current_track` is an Option<String>; TOML has no null, so it is omitted
    // in real files. Rewrite without it.
    std::fs::write(
        &path,
        "name = \"Album\"\ncreated = \"2026-01-01T00:00:00Z\"\nloop_mode = \"none\"\n\
         tracks = []\n",
    )
    .expect("write");
    assert!(Playlist::load(&path).expect("load").collapsed);
}

#[test]
fn a_collapse_toggle_survives_a_save_and_a_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Album.toml");
    let mut album = Playlist::empty("Album");
    album.collapsed = false;
    album.save(&path).expect("save");
    assert!(!Playlist::load(&path).expect("load").collapsed);
}

#[test]
fn the_sidebar_hides_an_album_that_has_a_live_parent() {
    let entries = vec![normal("aBooks"), album("Kino", Some("aBooks"))];
    assert_eq!(names(sidebar_entries(&entries)), vec!["aBooks"]);
}

#[test]
fn the_sidebar_keeps_an_orphaned_album_so_it_stays_reachable() {
    let entries = vec![normal("aBooks"), album("Kino", Some("gone"))];
    assert_eq!(names(sidebar_entries(&entries)), vec!["Kino", "aBooks"]);
}

#[test]
fn the_sidebar_keeps_an_album_parented_to_another_album() {
    let entries = vec![album("Inner", Some("Outer")), album("Outer", None)];
    assert_eq!(names(sidebar_entries(&entries)), vec!["Inner", "Outer"]);
}
```

`names` sorts nothing — `sidebar_entries` is alphabetical, so the expectations are
written in alphabetical order (`Kino` before `aBooks`: uppercase sorts first, which
is what `str::cmp` already does and what `nested_order` already produced).

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test playlist_test`
Expected: FAIL — `no field 'collapsed'`, `cannot find function 'sidebar_entries'`.

- [ ] **Step 3: Implement**

In `src/playlist.rs`, on `Playlist`:

```rust
    /// Whether this album's rows are folded away in its parent's track list.
    ///
    /// Folded by default, which is what an album written before this field
    /// existed loads as: a folder of two hundred files should arrive as one row
    /// and open on request. A normal playlist carries the field and ignores it.
    #[serde(default = "collapsed_by_default")]
    pub collapsed: bool,
```

```rust
/// `serde(default)` on a bool means `false`; an absent `collapsed` means folded.
fn collapsed_by_default() -> bool {
    true
}
```

Set `collapsed: collapsed_by_default()` in `Playlist::empty`.

Replace `nested_order` with:

```rust
/// The playlists the sidebar lists, alphabetically: everything except an album
/// that some *normal* playlist actually claims, which is shown inside that
/// playlist's track list instead.
///
/// An album whose parent is gone — deleted, or naming another album, or itself —
/// stays here at the top level. It is the only way left to reach it.
pub fn sidebar_entries(entries: &[PlaylistEntry]) -> Vec<&PlaylistEntry> {
    let claimed = |entry: &PlaylistEntry| {
        entry.kind == PlaylistKind::Album
            && entry.parent.as_ref().is_some_and(|parent| {
                entries
                    .iter()
                    .any(|e| &e.name == parent && e.kind == PlaylistKind::Normal)
            })
    };
    let mut rows: Vec<&PlaylistEntry> = entries.iter().filter(|e| !claimed(e)).collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}
```

Delete the `nested_order` tests and their `rows` helper in `playlist_test.rs`,
replacing them with the ones from Step 1.

In `src/tui/mod.rs`, drop `depth` from `SidebarItem::Playlist` and rewrite the loop:

```rust
            for entry in playlist::sidebar_entries(&self.available_playlists) {
                items.push(SidebarItem::Playlist {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    is_album: entry.kind == PlaylistKind::Album,
                });
            }
```

In `src/tui/ui.rs`, the `SidebarItem::Playlist` arm loses `depth` from its pattern
and uses a fixed `let indent = "   ";`.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS. `ui_test.rs:3120-3130` needs `depth` removed from its
destructuring and its tuple.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: albums leave the sidebar, and remember whether they are folded"
```

---

### Task 2: The row model

**Files:**
- Modify: `src/tui/mod.rs` (add `RowSource`, `VisibleRow`, `LoadedAlbum`, the three
  new `App` fields, `load_albums`, `rebuild_rows`, `row_at`; delete
  `filtered_indices` and `track_index_at`)
- Test: `src/tui/ui_test.rs`

**Interfaces:**
- Consumes: `Playlist.collapsed` (Task 1).
- Produces:
  ```rust
  pub enum RowSource { Own, Album(usize) }
  pub enum VisibleRow { Track { source: RowSource, index: usize }, AlbumHeader { album: usize } }
  pub struct LoadedAlbum { pub name: String, pub path: PathBuf, pub playlist: Playlist }
  impl App {
      pub fn load_albums(&mut self);
      pub fn rebuild_rows(&mut self);
      pub fn row_at(&self, cursor: usize) -> Option<&VisibleRow>;
      pub fn row_track_id(&self, cursor: usize) -> Option<String>;
      pub fn source_playlist(&self, source: RowSource) -> Option<(&Playlist, &Path)>;
      pub fn album_of(&self, cursor: usize) -> Option<usize>;
      pub fn cursor_of_own_index(&self, index: usize) -> Option<usize>;
      pub fn has_filter(&self) -> bool;
      pub fn total_track_count(&self) -> usize;
      pub fn total_duration_secs(&self) -> u64;
  }
  ```

- [ ] **Step 1: Write the failing tests** in `src/tui/ui_test.rs`

```rust
#[test]
fn the_rows_are_the_parents_own_tracks_and_then_its_albums() {
    let mut app = app_with_albums(&["a", "b"], &[("Kino", &["k1", "k2"])]);
    assert_eq!(
        row_shapes(&app),
        vec!["own:0", "own:1", "header:Kino"],
        "a folded album is one row"
    );
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    assert_eq!(
        row_shapes(&app),
        vec!["own:0", "own:1", "header:Kino", "album0:0", "album0:1"]
    );
}

#[test]
fn albums_come_after_the_parents_tracks_in_alphabetical_order() {
    let app = app_with_albums(&["a"], &[("Zed", &["z"]), ("Ace", &["x"])]);
    assert_eq!(row_shapes(&app), vec!["own:0", "header:Ace", "header:Zed"]);
}

#[test]
fn an_empty_album_still_has_a_header_to_reach_it_by() {
    let app = app_with_albums(&[], &[("Empty", &[])]);
    assert_eq!(row_shapes(&app), vec!["header:Empty"]);
}

#[test]
fn a_search_hides_an_album_with_no_matches_header_and_all() {
    let mut app = app_with_albums(&["alpha"], &[("Kino", &["beta"])]);
    app.search_query = "alpha".to_string();
    app.rebuild_rows();
    assert_eq!(row_shapes(&app), vec!["own:0"]);
}

#[test]
fn a_search_opens_a_folded_album_that_has_a_match() {
    let mut app = app_with_albums(&["alpha"], &[("Kino", &["beta", "alpha two"])]);
    assert!(app.albums[0].playlist.collapsed, "folded to begin with");
    app.search_query = "alpha".to_string();
    app.rebuild_rows();
    assert_eq!(
        row_shapes(&app),
        vec!["own:0", "header:Kino", "album0:1"],
        "only the matching track, and the header above it"
    );
}

#[test]
fn a_search_matching_an_albums_name_shows_all_of_it() {
    let mut app = app_with_albums(&["alpha"], &[("Kino", &["beta", "gamma"])]);
    app.search_query = "kino".to_string();
    app.rebuild_rows();
    assert_eq!(row_shapes(&app), vec!["header:Kino", "album0:0", "album0:1"]);
}

#[test]
fn the_title_counts_every_track_the_playlist_holds_folded_or_not() {
    let app = app_with_albums(&["a", "b"], &[("Kino", &["k1", "k2"])]);
    assert_eq!(app.total_track_count(), 4);
    assert_eq!(app.visible_track_count(), 3, "rows, not tracks");
}

#[test]
fn under_a_filter_the_title_counts_only_what_is_shown() {
    let mut app = app_with_albums(&["alpha"], &[("Kino", &["beta"])]);
    app.search_query = "alpha".to_string();
    app.rebuild_rows();
    assert_eq!(app.total_track_count(), 1);
}
```

Helpers, beside the existing `make_track`/`make_playlist` at the top of
`ui_test.rs`:

```rust
    /// An `App` displaying `own` tracks with `albums` under it. Every id is also
    /// its title, so a search expectation reads as the id it matches.
    fn app_with_albums(own: &[&str], albums: &[(&str, &[&str])]) -> App {
        let mut app = make_app_with_tracks(own);
        for (name, ids) in albums {
            for id in *ids {
                app.library.upsert(make_track_with_id(id)).expect("upsert");
            }
            let mut playlist = Playlist::empty(name);
            playlist.kind = PlaylistKind::Album;
            playlist.parent = Some(app.displayed_playlist_name());
            playlist.tracks = ids.iter().map(|s| s.to_string()).collect();
            app.albums.push(LoadedAlbum {
                name: (*name).to_string(),
                path: PathBuf::from(format!("/tmp/trovers-test/{name}.toml")),
                playlist,
            });
        }
        app.albums.sort_by(|a, b| a.name.cmp(&b.name));
        app.rebuild_rows();
        app
    }

    /// Each row as `own:<index>`, `album<n>:<index>` or `header:<name>` — the
    /// whole of what a row means, in one readable line per assertion.
    fn row_shapes(app: &App) -> Vec<String> {
        app.rows
            .iter()
            .map(|row| match row {
                VisibleRow::Track { source: RowSource::Own, index } => format!("own:{index}"),
                VisibleRow::Track { source: RowSource::Album(a), index } => {
                    format!("album{a}:{index}")
                }
                VisibleRow::AlbumHeader { album } => {
                    format!("header:{}", app.albums[*album].name)
                }
            })
            .collect()
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test ui_test`
Expected: FAIL to compile — `no field 'albums'`, `no field 'rows'`, `cannot find
type 'VisibleRow'`. This is Rust's RED.

- [ ] **Step 3: Implement** in `src/tui/mod.rs`

Types (above `App`):

```rust
// ── Visible rows ──────────────────────────────────────────────────────────

/// Which list a visible row's track comes out of.
///
/// A row on screen is no longer an index into one vector: the displayed playlist
/// and each album under it are separate files with separate running orders, and
/// the row has to say which one it means before anything can play, delete or
/// reorder it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowSource {
    /// The displayed playlist's own tracks.
    Own,
    /// An album under it, by index into `App::albums`.
    Album(usize),
}

/// One line of the track table.
#[derive(Debug, Clone, PartialEq)]
pub enum VisibleRow {
    /// `index` indexes the `tracks` of whichever list `source` names — never the
    /// screen, which is what `rows` itself is.
    Track { source: RowSource, index: usize },
    /// An album's own line: its name, its size, and whether it is open.
    AlbumHeader { album: usize },
}

/// An album under the displayed playlist, held open while it is on screen.
///
/// The `Playlist` is the same struct as any other, because an album *is* an
/// ordinary playlist file — which is what lets one play as its own list without
/// any of the rest of playback knowing that albums exist.
pub struct LoadedAlbum {
    /// The file stem, which is what `parent` and a rename address it by.
    pub name: String,
    pub path: PathBuf,
    pub playlist: Playlist,
}
```

`App`: delete `pub filtered_indices: Vec<usize>`, add

```rust
    /// The albums hanging under the displayed playlist, alphabetically.
    pub albums: Vec<LoadedAlbum>,
    /// Every row on screen, in display order — the cursor's coordinate system.
    /// Derived; `rebuild_rows` is its only writer.
    pub rows: Vec<VisibleRow>,
    /// The active search text, lowercased on use. Kept apart from `input_buf`,
    /// which is cleared the moment the prompt closes while the filter stays on.
    pub search_query: String,
```

In `App::new`, initialise the three (`Vec::new()`, `Vec::new()`, `String::new()`),
then replace the `current_track_index` block with:

```rust
        app.load_albums();
        app.rebuild_rows();
        if let Some(cursor) = app
            .current_track_index()
            .and_then(|index| app.cursor_of_own_index(index))
        {
            app.selected = cursor;
        }
        app
```

Methods (replacing `track_index_at`, `visible_track_count`,
`visible_duration_secs`):

```rust
    /// Load the albums that name the displayed playlist as their parent.
    ///
    /// From `available_playlists`, so it costs one small read per album rather
    /// than a directory scan, and an album that will not parse is skipped with a
    /// warning — a broken file must not take the playlist down with it.
    pub fn load_albums(&mut self) {
        let parent = self.displayed_playlist_name();
        let mut albums = Vec::new();
        for entry in &self.available_playlists {
            if entry.kind != PlaylistKind::Album || entry.parent.as_deref() != Some(&*parent) {
                continue;
            }
            match Playlist::load(&entry.path) {
                Ok(playlist) => albums.push(LoadedAlbum {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    playlist,
                }),
                Err(e) => {
                    warn!(err = %e, path = %entry.path.display(), "skipping an album that will not load")
                }
            }
        }
        albums.sort_by(|a, b| a.name.cmp(&b.name));
        self.albums = albums;
    }

    /// Whether a search filter is narrowing the rows.
    pub fn has_filter(&self) -> bool {
        !self.search_query.is_empty()
    }

    /// Rebuild `rows` from the playlist, its albums, the filter and each album's
    /// fold state. The only writer of `rows`.
    ///
    /// Own tracks first, then the albums alphabetically — so the list the user
    /// built by hand stays where they left it and the imported folders sit below
    /// it in a predictable order.
    pub fn rebuild_rows(&mut self) {
        let query = self.search_query.to_lowercase();
        let hit = |id: &String| {
            query.is_empty()
                || self
                    .library
                    .get(id)
                    .is_some_and(|track| track_matches(track, &query))
        };

        let mut rows = Vec::new();
        for (index, id) in self.playlist.tracks.iter().enumerate() {
            if hit(id) {
                rows.push(VisibleRow::Track {
                    source: RowSource::Own,
                    index,
                });
            }
        }

        for (album, loaded) in self.albums.iter().enumerate() {
            // A name match shows the whole album: the user asked for the album,
            // not for the tracks inside it whose titles happen to repeat it.
            let by_name = !query.is_empty() && loaded.name.to_lowercase().contains(&query);
            let indices: Vec<usize> = loaded
                .playlist
                .tracks
                .iter()
                .enumerate()
                .filter(|(_, id)| by_name || hit(id))
                .map(|(index, _)| index)
                .collect();
            // An empty album keeps its header — the album exists, and the row is
            // how the user reaches it. Under a filter it does not: nothing in it
            // was asked for.
            if self.has_filter() && indices.is_empty() {
                continue;
            }
            rows.push(VisibleRow::AlbumHeader { album });
            // A filter overrides the fold: hits hidden inside a folded album
            // would read as a search that missed them.
            if !loaded.playlist.collapsed || self.has_filter() {
                for index in indices {
                    rows.push(VisibleRow::Track {
                        source: RowSource::Album(album),
                        index,
                    });
                }
            }
        }
        self.rows = rows;
    }

    pub fn row_at(&self, cursor: usize) -> Option<&VisibleRow> {
        self.rows.get(cursor)
    }

    /// The list a row comes out of, and the file it lives in.
    pub fn source_playlist(&self, source: RowSource) -> Option<(&Playlist, &Path)> {
        match source {
            RowSource::Own => Some((&self.playlist, self.playlist_path.as_path())),
            RowSource::Album(album) => self
                .albums
                .get(album)
                .map(|loaded| (&loaded.playlist, loaded.path.as_path())),
        }
    }

    /// The library id a row names, whichever list it comes from.
    pub fn row_track_id(&self, cursor: usize) -> Option<String> {
        let &VisibleRow::Track { source, index } = self.row_at(cursor)? else {
            return None;
        };
        let (playlist, _) = self.source_playlist(source)?;
        playlist.tracks.get(index).cloned()
    }

    /// The album a header row is for — `None` on any other row.
    pub fn album_of(&self, cursor: usize) -> Option<usize> {
        match self.row_at(cursor)? {
            VisibleRow::AlbumHeader { album } => Some(*album),
            VisibleRow::Track { .. } => None,
        }
    }

    /// Where an own-track index sits on screen, for restoring a cursor.
    pub fn cursor_of_own_index(&self, index: usize) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(row, VisibleRow::Track { source: RowSource::Own, index: i } if *i == index)
        })
    }

    /// How many rows the cursor and the scroll window count — headers included.
    pub fn visible_track_count(&self) -> usize {
        self.rows.len()
    }

    /// Every track the playlist holds, folded or not — or, under a filter, every
    /// track row on screen, so the panel title agrees with what is there.
    pub fn total_track_count(&self) -> usize {
        if self.has_filter() {
            return self
                .rows
                .iter()
                .filter(|row| matches!(row, VisibleRow::Track { .. }))
                .count();
        }
        self.playlist.tracks.len()
            + self
                .albums
                .iter()
                .map(|loaded| loaded.playlist.tracks.len())
                .sum::<usize>()
    }

    /// What `total_track_count` covers, in seconds. A track whose document is
    /// gone, or whose duration was never learned, contributes nothing.
    pub fn total_duration_secs(&self) -> u64 {
        let sum = |ids: &[String]| -> u64 {
            ids.iter()
                .filter_map(|id| self.library.get(id))
                .map(|track| track.duration)
                .sum()
        };
        if self.has_filter() {
            return (0..self.rows.len())
                .filter_map(|cursor| self.row_track_id(cursor))
                .filter_map(|id| self.library.get(&id))
                .map(|track| track.duration)
                .sum();
        }
        sum(&self.playlist.tracks)
            + self
                .albums
                .iter()
                .map(|loaded| sum(&loaded.playlist.tracks))
                .sum::<u64>()
    }
```

`update_search` becomes:

```rust
    pub fn update_search(&mut self) {
        self.search_query = self.input_buf.clone();
        self.rebuild_rows();
        self.selected = 0;
        self.track_offset = 0;
    }

    /// Drop the search filter and show everything again.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.rebuild_rows();
        self.selected = 0;
        self.track_offset = 0;
    }
```

Delete `visible_duration_secs` and `track_index_at`.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: the new tests PASS; the nine former `track_index_at` call sites and the
four `filtered_indices` test sites are compile errors, which Tasks 3-6 fix. Fix the
call sites mechanically as the compiler names them, temporarily via
`row_track_id`/`row_at`, so the tree compiles before Task 3 refines each one.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: the track list becomes a list of rows, not of ids"
```

---

### Task 3: Rendering — headers, indented tracks, per-album numbering, the title

**Files:**
- Modify: `src/tui/ui.rs:217-422` (`row_is_playing`, `track_panel_title`,
  `render_track_table`; add `album_header_row`)
- Test: `src/tui/ui_test.rs`

**Interfaces:**
- Consumes: `App::rows`, `RowSource`, `VisibleRow`, `total_track_count`,
  `total_duration_secs`, `source_playlist` (Task 2).
- Produces: `ui::album_header_row(name, tracks, secs, open, is_selected) -> Row`;
  `ui::track_panel_title(name, tracks, first, last, rows, total_secs) -> String`;
  `ui::row_is_playing(app, source, id) -> bool`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_panel_title_counts_tracks_but_windows_over_rows() {
    assert_eq!(
        track_panel_title("aBooks", 23, 1, 12, 14, 117_660),
        " aBooks · 23 tracks · 32h 41m  [ 1–12 / 14 ] "
    );
}

#[test]
fn a_playlist_with_no_albums_has_the_title_it_always_had() {
    assert_eq!(
        track_panel_title("Live Sets", 42, 12, 20, 42, 22_320),
        " Live Sets · 42 tracks · 6h 12m  [ 12–20 / 42 ] "
    );
}

#[test]
fn an_album_header_says_how_big_it_is_and_whether_it_is_open() {
    assert_eq!(
        header_cells(&album_header_row("Kino", 10, 2_760, false, false)),
        vec!["", "", "  ▸ Kino", "10 tracks", "46m"]
    );
    assert_eq!(
        header_cells(&album_header_row("Kino", 1, 0, true, false))[2..],
        vec!["  ▾ Kino", "1 track", ""]
    );
}

#[test]
fn an_album_track_is_numbered_within_its_own_album() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1", "k2"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    assert_eq!(
        row_numbers(&app),
        vec!["  1 ", "", "  1 ", "  2 "],
        "the parent's track, the header, then the album's own 1 and 2"
    );
}

#[test]
fn the_playing_marker_follows_the_album_the_track_plays_from() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.playing = Some(PlayingSession {
        path: app.albums[0].path.clone(),
        playlist: app.albums[0].playlist.clone(),
        track_id: "k1".to_string(),
    });
    assert!(!row_is_playing(&app, RowSource::Own, "a"));
    assert!(row_is_playing(&app, RowSource::Album(0), "k1"));
}
```

`header_cells` and `row_numbers` render through the existing test helper that
already turns a `Row` into its cell strings (`row_cells` in `ui_test.rs`); add
`header_cells` as an alias only if `row_cells` is not already generic over `Row`.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test ui_test`
Expected: FAIL — `track_panel_title` takes 5 arguments, `album_header_row` not
found, `row_is_playing` takes 2 arguments.

- [ ] **Step 3: Implement**

`row_is_playing` takes the row's own source, so an album row lights up when the
album is what is playing:

```rust
/// Whether this row is the one actually driving playback — `(the file the row
/// belongs to, the id)`, so the same track listed in an album and in its parent
/// marks only the copy that is playing.
pub(crate) fn row_is_playing(app: &App, source: RowSource, id: &str) -> bool {
    let Some((_, path)) = app.source_playlist(source) else {
        return false;
    };
    app.playing
        .as_ref()
        .is_some_and(|p| p.path == path && p.track_id == id)
}
```

`track_panel_title` gains a `rows` parameter used only in the bracket:

```rust
pub(crate) fn track_panel_title(
    name: &str,
    tracks: usize,
    first: usize,
    last: usize,
    rows: usize,
    total_secs: u64,
) -> String {
    if rows == 0 {
        return format!(" {name} ");
    }
    let label = if tracks == 1 { "track" } else { "tracks" };
    let mut title = format!(" {name} · {tracks} {label}");
    if total_secs > 0 {
        title.push_str(" · ");
        title.push_str(&coarse_duration(total_secs));
    }
    // Two spaces: the counter is a separate reading from the summary, not
    // another item in its `·` list. It counts rows, which is what the scrollbar
    // and the cursor count — with albums present that is more than `tracks`.
    format!("{title}  [ {first}–{last} / {rows} ] ")
}
```

The header row:

```rust
/// An album's own line: a disclosure glyph, its name, how much it holds.
///
/// `▸`/`▾` rather than the sidebar's `▶`/`▼`, because `▶` is the playing marker
/// two columns to the left and the two must not read as the same thing. The count
/// and the duration take the artist and duration columns, so a header lines up
/// with the tracks under it instead of running across them.
pub(crate) fn album_header_row<'a>(
    name: &str,
    tracks: usize,
    total_secs: u64,
    open: bool,
    is_selected: bool,
) -> Row<'a> {
    let glyph = if open { "▾" } else { "▸" };
    let label = if tracks == 1 { "track" } else { "tracks" };
    let duration = if total_secs > 0 {
        coarse_duration(total_secs)
    } else {
        String::new()
    };
    let style = if is_selected {
        Style::new().fg(Color::White).bg(ROW_SELECTED_BG).bold()
    } else {
        Style::new().fg(GOLD).bold()
    };
    Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        Cell::from(format!("  {glyph} {name}")),
        Cell::from(Span::styled(
            format!("{tracks} {label}"),
            Style::new().fg(TEXT_DIM),
        )),
        Cell::from(Span::styled(duration, Style::new().fg(TEXT_DIM))),
    ])
    .style(style)
}
```

`render_track_table`'s title block and row loop:

```rust
    let rows_total = app.visible_track_count();
    let first = app.track_offset + 1;
    let last = (app.track_offset + app.track_list_height as usize).min(rows_total);

    let title = track_panel_title(
        &app.playlist.name,
        app.total_track_count(),
        first,
        last,
        rows_total,
        app.total_duration_secs(),
    );
```

and, inside the `filter_map` over cursors, before anything else:

```rust
            let is_selected = cursor == app.selected;
            let (source, track_idx) = match app.row_at(cursor)? {
                VisibleRow::AlbumHeader { album } => {
                    let loaded = app.albums.get(*album)?;
                    let secs = loaded
                        .playlist
                        .tracks
                        .iter()
                        .filter_map(|id| app.library.get(id))
                        .map(|track| track.duration)
                        .sum();
                    return Some(album_header_row(
                        &loaded.name,
                        loaded.playlist.tracks.len(),
                        secs,
                        !loaded.playlist.collapsed,
                        is_selected,
                    ));
                }
                VisibleRow::Track { source, index } => (*source, *index),
            };
            let (list, _) = app.source_playlist(source)?;
            let id = list.tracks.get(track_idx)?;
            let is_playing = row_is_playing(app, source, id);
            let num_str = format!("{:>3} ", track_idx + 1);
            // An album's tracks sit under its header rather than beside it.
            let indent = if matches!(source, RowSource::Album(_)) { "    " } else { "" };
```

with `Cell::from(truncate(&format!("{indent}{title_str}"), title_width))` for the
title cell, and `missing_document_row` given the same `indent`.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS, including the five existing `track_panel_title` tests once their
call sites gain the `rows` argument (equal to `total`, since none has albums).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: draw an album as a collapsible group in the track list"
```

---

### Task 4: Playing a row out of its own list

**Files:**
- Modify: `src/tui/mod.rs` (`play_from_list`, `request_playback`, `play_row`)
- Modify: `src/tui/input.rs:222-248` (`Enter`, `Space`), `:398-427` (`step_track`)
- Test: `src/tui/ui_test.rs`

**Interfaces:**
- Consumes: `row_at`, `source_playlist`, `RowSource` (Task 2).
- Produces: `App::play_row(&mut self, cursor: usize)`;
  `App::row_group(&self, cursor: usize) -> Option<(RowSource, Vec<usize>)>`
  (cursor positions of the row's own list, in display order).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn playing_an_album_track_plays_it_out_of_the_album() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1", "k2"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 2; // header at 1, so this is the album's first track
    app.play_row(app.selected);
    let session = app.playing.as_ref().expect("a session");
    assert_eq!(session.path, app.albums[0].path, "the album's file, not the parent's");
    assert_eq!(session.track_id, "k1");
    assert_eq!(session.playlist.tracks, vec!["k1", "k2"], "the album's running order");
}

#[test]
fn playing_an_album_track_records_it_in_the_albums_own_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_albums_in(&dir, &["a"], &[("Kino", &["k1"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 1;
    app.play_row(app.selected);
    let saved = Playlist::load(&app.albums[0].path).expect("load");
    assert_eq!(saved.current_track.as_deref(), Some("k1"));
}

#[test]
fn a_header_row_does_not_play_anything() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1"])]);
    app.selected = 1;
    app.play_row(app.selected);
    assert!(app.playing.is_none(), "a header is not a track");
}

#[test]
fn stepping_past_an_albums_last_track_wraps_inside_the_album() {
    let mut app = app_with_albums(&["a", "b"], &[("Kino", &["k1", "k2"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 4; // the album's second and last track
    step_track(&mut app, true);
    assert_eq!(app.selected, 3, "back to the album's first track, not to the parent's");
}

#[test]
fn stepping_past_the_parents_last_track_stays_among_the_parents() {
    let mut app = app_with_albums(&["a", "b"], &[("Kino", &["k1"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 1; // the parent's last own track
    step_track(&mut app, true);
    assert_eq!(app.selected, 0, "wraps to the parent's first, never into the album");
}
```

`step_track` becomes `pub(crate)` so the test can drive it, as `handle_tracklist`
already is.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test ui_test`
Expected: FAIL — `no method named 'play_row'`, `step_track` is private.

- [ ] **Step 3: Implement**

In `src/tui/mod.rs`, rename the body of `request_playback` to `play_from_list` and
thread the list through it:

```rust
    /// Start playback of the track at Vec index `idx` within the displayed
    /// playlist (`self.playlist`).
    pub fn request_playback(&mut self, idx: usize, start_pos: Option<f64>) {
        let path = self.playlist_path.clone();
        let list = self.playlist.clone();
        self.play_from_list(path, list, idx, start_pos);
    }

    /// Start playback of `index` within `list`, the playlist file at `path`.
    ///
    /// The one door into playback for a row on screen, whether that row belongs
    /// to the displayed playlist or to an album inside it. The session records
    /// the file the track came out of, which is what keeps `n`/`b`, `loop_mode`,
    /// `shuffle` and auto-advance inside that file.
    fn play_from_list(
        &mut self,
        path: PathBuf,
        mut list: Playlist,
        index: usize,
        start_pos: Option<f64>,
    ) {
        // ... the current body, with `self.playlist_path` → `path` and
        // `self.playlist` → `list`, and this in place of the `current_track` line:
        list.current_track = Some(id.clone());
        if path == self.playlist_path {
            self.playlist.current_track = Some(id.clone());
        } else if let Some(loaded) = self.albums.iter_mut().find(|a| a.path == path) {
            // Nothing else ever saves an album, so which track was last played
            // out of one has to be written here or it is lost.
            loaded.playlist.current_track = Some(id.clone());
            if let Err(e) = loaded.playlist.save(&path) {
                error!(err = %e, path = %path.display(), "failed to record the album's current track");
            }
        }
    }

    /// Play whatever the cursor is on, out of the list that row belongs to.
    /// A header is not a track and is silently left alone.
    pub fn play_row(&mut self, cursor: usize) {
        let Some(&VisibleRow::Track { source, index }) = self.row_at(cursor) else {
            return;
        };
        let Some((list, path)) = self.source_playlist(source) else {
            return;
        };
        let (list, path) = (list.clone(), path.to_path_buf());
        let start_pos = list
            .tracks
            .get(index)
            .and_then(|id| self.library.get(id))
            .and_then(input::resume_start_pos);
        self.play_from_list(path, list, index, start_pos);
    }

    /// The rows of the same list as `cursor`'s, as cursor positions in display
    /// order — what `n`/`b` walk, and nothing else. `None` on a header, which
    /// belongs to no running order.
    pub fn row_group(&self, cursor: usize) -> Option<(RowSource, Vec<usize>)> {
        let &VisibleRow::Track { source, .. } = self.row_at(cursor)? else {
            return None;
        };
        let group = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, VisibleRow::Track { source: s, .. } if *s == source))
            .map(|(cursor, _)| cursor)
            .collect();
        Some((source, group))
    }
```

In `src/tui/input.rs`, `Enter` and the play half of `Space` become
`app.play_row(app.selected)`, and `step_track`:

```rust
pub(crate) fn step_track(app: &mut App, forward: bool) {
    let Some((source, group)) = app.row_group(app.selected) else {
        return;
    };
    let Some(at) = group.iter().position(|&c| c == app.selected) else {
        return;
    };
    let (list, path) = match app.source_playlist(source) {
        Some((list, path)) => (list.shuffle, path.to_path_buf()),
        None => return,
    };

    let next = if app.has_filter() || !list {
        group[if forward {
            (at + 1) % group.len()
        } else {
            at.checked_sub(1).unwrap_or(group.len() - 1)
        }]
    } else {
        // Unfiltered, so this group is the whole of its list in order: a shuffled
        // step over indices is directly a position within the group.
        match app.step_index(&path, group.len(), true, at, forward) {
            Some(at) => group[at],
            None => return,
        }
    };

    app.selected = next;
    app.clamp_scroll();
    app.play_row(next);
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: an album plays as its own list"
```

---

### Task 5: The keys on a header, and edits that follow the row's owner

**Files:**
- Modify: `src/tui/mod.rs` (`toggle_album`, `move_selected_row`,
  `move_track_to_playlist`, `platform_id_referenced_elsewhere`, `rename_album`,
  `delete_album`, `rescan_album`, `InputMode`)
- Modify: `src/tui/input.rs` (`handle_tracklist`, `handle_confirm_delete`,
  `handle_album_rename`, `handle_album_delete`)
- Modify: `src/tui/ui.rs` (`footer_left_message`, `footer_center_context`, the
  rename/delete overlays, the help text)
- Test: `src/tui/ui_test.rs`

**Interfaces:**
- Consumes: everything from Tasks 2-4.
- Produces: `InputMode::AlbumRename`, `InputMode::AlbumDelete`;
  `App::toggle_album(album: usize)`, `App::rename_album(album: usize, new_name: &str)
  -> Result<()>`, `App::delete_album(album: usize)`, `App::rescan_album(album: usize)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn enter_on_a_header_opens_and_closes_the_album_without_playing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_albums_in(&dir, &["a"], &[("Kino", &["k1"])]);
    app.selected = 1;
    press(&mut app, KeyCode::Enter);
    assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino", "album0:0"]);
    assert!(app.playing.is_none(), "a header is not playable");
    assert!(!Playlist::load(&app.albums[0].path).expect("load").collapsed);
    press(&mut app, KeyCode::Enter);
    assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino"]);
    assert!(Playlist::load(&app.albums[0].path).expect("load").collapsed);
}

#[test]
fn shift_j_swaps_two_tracks_inside_the_album_they_belong_to() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_albums_in(&dir, &["a"], &[("Kino", &["k1", "k2"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 2;
    press(&mut app, KeyCode::Char('J'));
    assert_eq!(app.albums[0].playlist.tracks, vec!["k2", "k1"]);
    assert_eq!(app.playlist.tracks, vec!["a"], "the parent is untouched");
    assert_eq!(app.selected, 3, "the cursor stays on the row it moved");
    assert_eq!(
        Playlist::load(&app.albums[0].path).expect("load").tracks,
        vec!["k2", "k1"]
    );
}

#[test]
fn shift_j_refuses_to_move_a_track_across_the_boundary() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1"])]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.selected = 0; // the parent's last own track, header below it
    press(&mut app, KeyCode::Char('J'));
    assert_eq!(app.playlist.tracks, vec!["a"]);
    assert_eq!(app.albums[0].playlist.tracks, vec!["k1"]);
    assert_eq!(app.selected, 0);
}

#[test]
fn shift_j_on_a_header_says_albums_are_sorted() {
    let mut app = app_with_albums(&["a"], &[("Kino", &["k1"]), ("Zed", &["z1"])]);
    app.selected = 1;
    press(&mut app, KeyCode::Char('J'));
    assert_eq!(
        app.albums.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        vec!["Kino", "Zed"]
    );
    assert_eq!(status_of(&app), "Albums are sorted by name");
}

#[test]
fn deleting_an_album_row_edits_the_album_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media = dir.path().join("song.mp3");
    std::fs::write(&media, b"not really audio").expect("write");
    let mut app = app_with_local_album(&dir, &media);
    app.selected = 1;
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    assert!(app.albums[0].playlist.tracks.is_empty());
    assert!(media.exists(), "the user's file is never trovers' to delete");
}

#[test]
fn deleting_an_album_forgets_it_and_never_touches_the_folder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_albums_in(&dir, &["a"], &[("Kino", &["k1"])]);
    let album_path = app.albums[0].path.clone();
    app.selected = 1;
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    assert!(app.albums.is_empty());
    assert!(!album_path.exists(), "its playlist file goes");
    assert!(dir.path().exists(), "the folder it mirrored stays");
    assert!(app.library.get("k1").is_some(), "the track keeps its document");
    assert_eq!(row_shapes(&app), vec!["own:0"]);
}

#[test]
fn renaming_an_album_from_its_header_renames_its_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_albums_in(&dir, &["a"], &[("Kino", &["k1"])]);
    let old = app.albums[0].path.clone();
    app.selected = 1;
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.input_mode, InputMode::AlbumRename);
    assert_eq!(app.input_buf, "Kino", "the prompt opens on the current name");
    app.input_buf = "Viktor".to_string();
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.albums[0].name, "Viktor");
    assert!(!old.exists());
    assert!(app.albums[0].path.exists());
}
```

`press` is the existing helper that runs `handle_key` through the tokio test
runtime; `status_of` reads `app.status_message`. Both already exist in `ui_test.rs`.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test ui_test`
Expected: FAIL — `InputMode::AlbumRename` not found, `Enter` on a header plays
nothing but also toggles nothing.

- [ ] **Step 3: Implement**

`InputMode` gains `AlbumRename` and `AlbumDelete`, both excluded from the Tab
switch and from the `?` help toggle exactly as `PlaylistRename`/`PlaylistDelete`
are, and both given a `footer_left_message` prompt:

```rust
        (InputMode::AlbumRename, _) => {
            return "Rename album: Enter name · [enter] confirm · [esc] cancel".to_string();
        }
        (InputMode::AlbumDelete, _) => {
            return "Delete album? Its files stay · [y/enter] confirm · [n/esc] cancel"
                .to_string();
        }
```

and a `footer_center_context` label (`"Rename album"`, `"Delete album"`), and an
overlay each — the existing `render_playlist_rename_overlay` and
`render_playlist_delete_overlay` generalised to take the target name rather than
reading the sidebar selection.

`App::toggle_album`:

```rust
    /// Fold or unfold an album, and remember which in its own file.
    ///
    /// The cursor stays on the header: folding only ever changes rows *below* it,
    /// since an album's tracks follow its header and the albums after it come
    /// after those.
    pub fn toggle_album(&mut self, album: usize) {
        let Some(loaded) = self.albums.get_mut(album) else {
            return;
        };
        loaded.playlist.collapsed = !loaded.playlist.collapsed;
        let path = loaded.path.clone();
        if let Err(e) = loaded.playlist.save(&path) {
            error!(err = %e, path = %path.display(), "failed to save an album's fold state");
        }
        self.rebuild_rows();
        self.clamp_scroll();
    }
```

`move_selected_row` becomes row-aware:

```rust
    /// Move the selected row one place down (`down`) or up within the list it
    /// belongs to.
    ///
    /// Refused under a search filter, where ±1 on screen would jump the row over
    /// whatever the filter hides. Refused across the boundary between the
    /// parent's tracks and an album's, and between two albums: that is a move
    /// between lists, which is `m`. A header does not move at all — albums are
    /// ordered by name.
    pub fn move_selected_row(&mut self, down: bool) {
        if self.has_filter() {
            self.set_status("Clear the search to reorder");
            return;
        }
        if self.album_of(self.selected).is_some() {
            self.set_status("Albums are sorted by name");
            return;
        }
        let from = self.selected;
        let to = if down { from + 1 } else { from.wrapping_sub(1) };
        let (Some(&VisibleRow::Track { source, index: a }), Some(&VisibleRow::Track { source: neighbour, index: b })) =
            (self.row_at(from), self.row_at(to))
        else {
            return;
        };
        if source != neighbour {
            return;
        }

        match source {
            RowSource::Own => {
                self.playlist.tracks.swap(a, b);
                self.save_playlist();
            }
            RowSource::Album(album) => {
                let Some(loaded) = self.albums.get_mut(album) else {
                    return;
                };
                loaded.playlist.tracks.swap(a, b);
                let path = loaded.path.clone();
                if let Err(e) = loaded.playlist.save(&path) {
                    error!(err = %e, path = %path.display(), "failed to save a reordered album");
                }
            }
        }
        // The cursor stays on the row the user was holding, not on the position.
        self.selected = to;
        self.rebuild_rows();
        self.clamp_scroll();
        // `shuffle_order` is deliberately left alone. It holds indices, so after
        // a swap it is still a permutation of `0..len` — no track is skipped or
        // repeated, only two of them trade places in the shuffled run.
    }
```

`platform_id_referenced_elsewhere` consults memory before disk:

```rust
    pub fn platform_id_referenced_elsewhere(&self, platform_id: &str) -> bool {
        let lists = |ids: &[String]| {
            ids.iter()
                .any(|id| library::platform_id_of(id) == platform_id)
        };
        // In memory first. The displayed list and its albums hold edits that are
        // not on disk yet — including the row that was just removed, which read
        // from disk would still be there and would keep a dead document alive.
        if lists(&self.playlist.tracks) {
            return true;
        }
        if self.albums.iter().any(|a| lists(&a.playlist.tracks)) {
            return true;
        }
        let in_memory = |path: &Path| {
            path == self.playlist_path || self.albums.iter().any(|a| a.path == path)
        };
        self.available_playlists
            .iter()
            .filter(|entry| !in_memory(&entry.path))
            .any(|entry| match Playlist::load(&entry.path) {
                Ok(pl) => lists(&pl.tracks),
                Err(e) => {
                    warn!(err = %e, path = %entry.path.display(), "could not check playlist for shared cache file; keeping it");
                    true
                }
            })
    }
```

`delete_album`, `rename_album` and `rescan_album`, each mirroring the sidebar
version it replaces — including re-pointing a playing session whose `path` is that
album, and stopping playback when the album being deleted is the one playing:

```rust
    /// Forget an album: its playlist file goes, and nothing else.
    ///
    /// Not the folder it mirrored, not the files in it, not the documents of the
    /// tracks it listed — those live in the library and may well be listed
    /// elsewhere. Deleting a container has never meant deleting its contents
    /// here (ADR-018).
    pub fn delete_album(&mut self, album: usize) { /* ... */ }

    /// Rename an album, in its own file, in the listing, and in the row on screen.
    pub fn rename_album(&mut self, album: usize, new_name: &str) -> Result<()> { /* ... */ }

    /// Rescan the folder this album mirrors.
    pub fn rescan_album(&mut self, album: usize) {
        match self.albums.get(album).and_then(|a| a.playlist.source_folder.clone()) {
            Some(root) => self.import_folder(root),
            None => self.set_status("Not linked to a folder"),
        }
    }
```

`handle_tracklist` dispatches on whether the cursor is a header:

```rust
        KeyCode::Enter => match app.album_of(app.selected) {
            Some(album) => app.toggle_album(album),
            None => app.play_row(app.selected),
        },
```

and the same shape for `r` (rename vs shuffle toggle — on a header `r` renames,
elsewhere it keeps toggling shuffle), `d` (`AlbumDelete` vs `ConfirmDelete`), `R`
(the album's folder vs the displayed playlist's), `m` and `c` (a status on a
header).

`handle_confirm_delete` takes the row's owning list:

```rust
        let Some(&VisibleRow::Track { source, index }) = app.row_at(app.selected) else {
            app.input_mode = InputMode::Normal;
            return Ok(Action::Continue);
        };
        let Some((list, path)) = app.source_playlist(source) else { /* ... */ };
        let (id, path) = (list.tracks[index].clone(), path.to_path_buf());
        let is_current = app.is_playing_track(&path, &id);
        // ... unchanged from here, except the removal and the save:
        match source {
            RowSource::Own => { app.playlist.tracks.remove(index); app.save_playlist(); }
            RowSource::Album(album) => { /* remove and save the album */ }
        }
        app.clear_search();
```

`move_track_to_playlist` likewise resolves its source from the row rather than
always `self.playlist`.

Help overlay: add `  [enter] on an album opens it   [r]/[d]/[R] rename/delete/rescan`.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: rename, delete, rescan and reorder an album from its own row"
```

---

### Task 6: Imports and playlist switches keep the rows in step

**Files:**
- Modify: `src/tui/mod.rs` (`apply_import`, `switch_to_playlist`)
- Modify: `src/tui/input.rs` (`repoint_albums`, `handle_playlist_delete`)
- Test: `src/tui/ui_test.rs`

**Interfaces:**
- Consumes: `load_albums`, `rebuild_rows`, `LoadedAlbum` (Task 2).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_fresh_import_appears_as_an_open_album_under_the_cursor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_in(&dir, &["a"]);
    let root = dir.path().join("Kino");
    std::fs::create_dir_all(&root).expect("mkdir");
    app.apply_import(
        root.clone(),
        ImportTarget::NewAlbum { parent: Some(app.displayed_playlist_name()) },
        vec![imported_file(&root.join("one.mp3"))],
    );
    assert_eq!(app.albums.len(), 1);
    assert!(!app.albums[0].playlist.collapsed, "an import you cannot see did nothing");
    assert_eq!(row_shapes(&app), vec!["own:0", "header:Kino", "album0:0"]);
}

#[test]
fn a_rescan_updates_the_album_on_screen_not_only_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("Kino");
    std::fs::create_dir_all(&root).expect("mkdir");
    let mut app = app_with_linked_album(&dir, &root, &["one.mp3"]);
    app.albums[0].playlist.collapsed = false;
    app.rebuild_rows();
    app.apply_import(
        root.clone(),
        ImportTarget::Existing(app.albums[0].path.clone()),
        vec![imported_file(&root.join("one.mp3")), imported_file(&root.join("two.mp3"))],
    );
    assert_eq!(app.albums[0].playlist.tracks.len(), 2);
    assert_eq!(row_shapes(&app), vec!["header:Kino", "album0:0", "album0:1"]);
}

#[test]
fn switching_playlist_loads_that_playlists_own_albums() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = two_playlists_each_with_an_album(&dir);
    assert_eq!(app.albums.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), vec!["Kino"]);
    let (name, path) = ("Second".to_string(), dir.path().join("Second.toml"));
    app.switch_to_playlist(&name, &path).expect("switch");
    assert_eq!(app.albums.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), vec!["Zed"]);
    assert_eq!(row_shapes(&app), vec!["header:Zed"]);
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test ui_test`
Expected: FAIL — `app.albums` is empty after the import and after the switch.

- [ ] **Step 3: Implement**

`apply_import`:
- the `Existing(path)` branch, when `path` is one of `self.albums`, merges into that
  in-memory album, saves it, and calls `rebuild_rows()` — no re-read from disk;
- the `NewAlbum` branch sets `album.collapsed = false` before merging, and after
  saving pushes a `LoadedAlbum` into `self.albums` (sorted) when the new album's
  parent is the displayed playlist, then `rebuild_rows()`;
- the displayed-playlist branch calls `rebuild_rows()` too, since its own rows grew.

`switch_to_playlist`, after `self.playlist_path = path.to_path_buf();`:

```rust
        self.load_albums();
        self.search_query.clear();
        self.rebuild_rows();
```

and the cursor restore goes through `cursor_of_own_index`.

`repoint_albums` (a parent rename) reloads `self.albums` when the renamed playlist
is the displayed one; `handle_playlist_delete` reloads them when the deleted
playlist was a parent, so its albums orphan into the sidebar and out of the rows.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS. All pre-existing tests green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: keep the rows in step with imports, rescans and playlist switches"
```

---

### Task 7: Docs

**Files:**
- Modify: `AGENTS.md`, `docs/decisions.md`, `docs/progress.md`, `README.md`

- [ ] **Step 1: Write them**

- `AGENTS.md` — the "Albums" section describes the track-list tree rather than the
  sidebar; the keymap gains `Enter`/`r`/`d`/`R` on a header; the Playlist schema
  gains `collapsed`; `App::rows`/`RowSource`/`VisibleRow` named as the cursor's
  coordinate system in place of `track_index_at`.
- `docs/decisions.md` — **ADR-019: an album is shown inside its parent's track
  list, not in the sidebar.** Context: 14 columns for a name. Decision: a computed
  row list; albums leave the sidebar; orphans stay. Consequences:
  `track_index_at` gone, nine call sites now ask the row, the panel title's two
  denominators. Amend ADR-016 to point at it.
- `docs/progress.md` — a row per task in the "Albums and local folders" table.
- `README.md` — the "Your Own Folders (Albums)" section: an album shows up inside
  the playlist you imported it into, `enter` opens and closes it, and the keys on
  its row.

- [ ] **Step 2: Commit**

```bash
git add -A && git commit -m "docs: albums live in the track list"
```

---

## Self-review

**Spec coverage.** §1 → Task 2. §2 → Task 3. §3 → Task 5. §4 → Task 4. §5 →
Task 1 (`collapsed`) and Task 5 (the toggle that writes it). §6 → Task 1. §7 →
Tasks 5 (`platform_id_referenced_elsewhere`, `move_track_to_playlist`,
`handle_confirm_delete`) and 6 (`switch_to_playlist`, `apply_import`, rename and
delete re-pointing). §8 → the tests in each task, plus the manual list carried into
`docs/progress.md` by Task 7.

**Type consistency.** `RowSource`, `VisibleRow`, `LoadedAlbum`, `row_at`,
`row_track_id`, `source_playlist`, `album_of`, `row_group`, `has_filter`,
`total_track_count`, `total_duration_secs`, `cursor_of_own_index`, `play_row`,
`play_from_list`, `toggle_album`, `rename_album`, `delete_album`, `rescan_album`,
`clear_search` are each defined once (Tasks 2, 4, 5) and used under those names
throughout. `track_panel_title` has one signature, six arguments, from Task 3 on.
`visible_track_count` keeps its name and means rows.

**Known wrinkle, deliberate.** The panel title's `N tracks` and its `[ a–b / R ]`
have different denominators when albums are present. Stated in the spec and in the
comment on `track_panel_title`; the alternative — a count that changes when a
folder is folded — is worse.
