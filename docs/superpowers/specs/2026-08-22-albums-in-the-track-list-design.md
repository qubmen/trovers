# Albums in the track list

**Status:** approved 2026-08-22
**Supersedes:** the sidebar half of ADR-016 (albums stay child playlist files; only
where they are *shown* changes)

## The problem

Albums landed as indented rows in the sidebar. The sidebar is 22 columns wide, of
which a nested album row has 14 for its name, so real album names arrive as
`Кино - Гр…` and `Суржиков …` — indistinguishable from each other and from
anything else imported from the same series. The panel that has room for a name is
the track table, and it is also where the album's contents belong: an album is a
part of the playlist you are looking at, not a sibling of it.

## The shape of the change

The track list stops being a flat window over `Vec<String>` and becomes a
two-level tree: the displayed playlist's own tracks, then each album under it as a
collapsible group. Albums leave the sidebar. The album is still an ordinary
playlist file with `kind = "album"` and `parent = "<stem>"` — nothing about storage
changes except one new field.

Everything that maps a cursor position to a track goes through
`App::track_index_at(cursor) -> Option<usize>`, an index into
`self.playlist.tracks`. That is the interface this change replaces, and the nine
call sites that depend on it are the work.

### Approaches considered

1. **A computed row list on `App`** — chosen. Each row records which playlist file
   it came from and which index in that file it is. Ownership stays explicit and
   every call site asks the row.
2. **Flatten album ids into one display vector** with a side table mapping row →
   owning file. Smaller diff, but ownership becomes implicit and every mutation
   (`d`, `J`/`K`, rescan) has to re-derive it — the bookkeeping ADR-015 removed.
   Rejected.
3. **Leave the tree alone and make the sidebar wider / truncate in the middle.**
   Cheapest, and it would fix the ugliness, but not what was asked for: the album's
   contents still would not be visible beside the tracks they belong with.

## 1. The row model

```rust
/// Which list a visible row's track comes out of.
pub enum RowSource {
    /// The displayed playlist's own tracks.
    Own,
    /// An album under it, by index into `App::albums`.
    Album(usize),
}

pub enum VisibleRow {
    Track { source: RowSource, index: usize },
    AlbumHeader { album: usize },
}
```

`index` indexes the `tracks` of whichever list `source` names — never the screen.

`App` gains:

```rust
/// The albums hanging under the displayed playlist, alphabetically. Loaded from
/// disk when the displayed playlist changes.
pub albums: Vec<LoadedAlbum>,
/// Every row on screen, in display order. Rebuilt from `playlist`, `albums`,
/// `search_query` and each album's `collapsed` — never edited in place.
pub rows: Vec<VisibleRow>,
/// The active search text. Held apart from `input_buf`, which is cleared the
/// moment the prompt closes while the filter stays on.
pub search_query: String,
```

```rust
pub struct LoadedAlbum {
    /// The file stem, which is what `parent` and the sidebar address it by.
    pub name: String,
    pub path: PathBuf,
    pub playlist: Playlist,
}
```

`App::filtered_indices` goes away. It held indices into `self.playlist.tracks`,
which no longer describes the screen; `rows` does, and the filter is one of its
inputs rather than a parallel copy of the answer.

`visible_track_count()` becomes `self.rows.len()` — the cursor and the scroll
window count rows, headers included.

### Building the rows

`rebuild_rows()` is the only writer of `self.rows`. Called after anything that can
change what is on screen: switching playlist, editing the search, toggling a
collapse, an import, a rescan, an album rename or delete, a reorder, a row
deletion.

Order: the parent's own tracks first, then the albums alphabetically, each header
followed by its own tracks when open.

Under a search filter:
- a track row survives if its track matches (the existing `track_matches`);
- an album whose **name** matches shows its header and *all* its tracks;
- an album with matching tracks shows its header and only those;
- an album with neither is hidden entirely, header included;
- a matching album is shown open regardless of `collapsed` — a search that
  silently hid its hits inside a folded album would be a bug, not a feature.

With no filter every album's header is shown even when the album is empty: the
album exists and the row is how the user reaches it.

## 2. What is on screen

```
┌ aBooks · 23 tracks · 32h 41m  [ 1–12 / 14 ] ──────────────────────┐
│ ▶ ◈  1  Ночной дозор                        Лукьяненко   8:12:03  │
│   ◈  2  Дневной дозор                       Лукьяненко   9:04:55  │
│   ▸ Кино - Группа крови - 1988-2019          10 tracks      46m   │
│   ▾ Суржиков Роман – Полари 06 … Том 1       20 tracks   16h 15m  │
│       ◈  1  Стрела                          Роман Сур…    48:20   │
│       ◈  2  Искра                           Роман Сур…    51:07   │
```

- Track numbering restarts at 1 inside each album: the number says where the track
  is in the list it belongs to, which is the list that plays it.
- Album tracks are indented in the title column.
- A header carries the album's name in the title column, its track count where the
  artist goes, and its total duration where the duration goes.
- The disclosure glyphs are `▸`/`▾`, not the sidebar's `▶`/`▼`: `▶` is already the
  playing marker and the two must not be confused.

### The panel title

` <name> · <N> tracks · <duration>  [ <first>–<last> / <rows> ] `

`N` and the duration describe the playlist's whole contents — its own tracks plus
every album's, folded or not — because that is what the list holds and folding is
a view. Under a search filter they describe the visible track rows instead, so the
title still agrees with the screen. The bracketed counter is the scroll window and
counts **rows**, so with albums present its denominator is larger than `N` by the
number of headers. With no albums the two are equal and the title is byte-for-byte
what it is today.

## 3. Keys

An album used to be reachable in the sidebar, which is where `r` renamed it and `d`
deleted it. Those move onto the header row, or they would be lost.

| Key | On an album header |
|---|---|
| `Enter` | open / close. A header is not playable. |
| `r` | rename the album (`InputMode::AlbumRename`) |
| `d` | delete the album (`InputMode::AlbumDelete`); files on disk are never touched |
| `R` | rescan this album's folder |
| `J`/`K` | nothing, with the status `Albums are sorted by name` |
| `n`/`b` | nothing: a header belongs to no running order |
| `m`, `c` | nothing, with a status — neither means anything for a group |

On an album's **track** row every key means what it means for an own track, applied
to the album's file: `Enter`/`Space` play it, `d` drops the row, `J`/`K` reorder it
within the album, `m` moves it out, `c` reports that a local file has nothing to
download.

Unchanged: `F` imports a folder as a new album under the displayed playlist —
including when the cursor is on a header, since nesting stays two deep. `R`
anywhere but a header rescans the displayed playlist's own folder. `a` adds a URL
to the displayed playlist.

## 4. What plays

An album plays as its own list. Starting a track from an album row builds
`PlayingSession { path: <album's path>, playlist: <the album>, track_id }`, so
`n`/`b`, `loop_mode`, `shuffle` and auto-advance all stay inside the album, each
with its own `shuffle_order` in its own file. This needs no change to
`PlayingSession`, which has carried its own `path` and `playlist` since ADR-011.

`request_playback` is generalised into one private `play_from_list(path, playlist,
index, start_pos)`, the single door into playback for any row on screen.
`request_playback(idx, start_pos)` stays as the displayed-playlist wrapper, so
`play_session_track` and the auto-advance path are untouched. Playing an album
track records `current_track` in the album's own file and saves it — nothing else
ever would.

`n`/`b` step within the list the cursor's row belongs to, never across the
boundary: from an album's last track they wrap to its first, not into the parent's
tracks. The `▶` marker already compares `(session.path, track_id)`, so it lights
the right row with no change.

## 5. What is on disk

One new field on `Playlist`:

```rust
/// Whether this album's rows are folded away in its parent's track list.
/// Defaults to folded, so an album opened for the first time is one row.
#[serde(default = "collapsed_by_default")]
pub collapsed: bool,
```

Written whenever the user toggles it. A freshly imported album is stored open, so
the import is visibly there; every album loaded from a file with no `collapsed` key
is folded.

Normal playlists carry the field too and ignore it — cheaper than a second struct
for one bool.

## 6. The sidebar

`sidebar_items()` lists only playlists that are not albums-with-a-live-parent. An
album whose parent is missing (deleted, renamed to nothing, naming another album or
itself) is still listed at the top level: otherwise deleting a parent would make
its albums unreachable. `playlist::nested_order` is replaced by
`playlist::sidebar_entries`, and `SidebarItem::Playlist` loses `depth` while
keeping `is_album` for the orphan's glyph.

## 7. Consequences elsewhere

- **`platform_id_referenced_elsewhere`** re-reads every other playlist file from
  disk to decide whether a track's cached audio is still needed. The loaded albums
  now hold unsaved edits — including the row just removed — so it must consult
  `self.albums` in memory and skip those files on disk. Without this, deleting a
  row from an album would always conclude "still referenced" and leak the document.
- **`move_track_to_playlist`** removes the row from the row's owning list and saves
  that file, not always the displayed one.
- **`handle_confirm_delete`** likewise, and its playback guard compares against the
  owning list's path.
- **`switch_to_playlist`** loads the new playlist's albums and rebuilds the rows;
  the cursor restored from `current_track` is converted from an own-track index to
  a cursor position.
- **`apply_import`** merges into the in-memory album when the target is one of the
  loaded ones, and pushes a newly created album into `self.albums`, so an import
  appears under the cursor and not only on disk.
- **Album rename/delete** re-point a playing session whose `path` is that album,
  exactly as the sidebar's versions already do.

## 8. Testing

Unit tests, in the existing files and style:

- `playlist_test.rs` — `sidebar_entries` hides an album with a live parent, keeps an
  orphan, keeps an album parented to an album, is alphabetical, lists everything
  exactly once; `collapsed` defaults to folded on a file without the key and round
  trips.
- `ui_test.rs` — row building: own tracks then albums alphabetically; a folded album
  contributes one row; an open one contributes its tracks; an empty album still has
  a header; a filter hides an album with no hits, opens one with hits, and shows all
  of a name match. Header rendering: glyph, count, duration. Panel title with
  albums. Keys: `Enter` toggles and does not play; `r`/`d`/`R` on a header reach the
  right album; `J`/`K` swap within the owning list and refuse on a header; `d` on an
  album row edits the album's file and leaves the file on disk; playing an album
  track builds a session pointing at the album; `n` from the album's last track
  wraps to its first rather than into the parent.

Manual, at a real terminal: import two folders under one playlist, fold and unfold
both, confirm the fold survives a restart; play through the end of an album and
confirm auto-advance stays inside it; rename and delete an album from its header and
confirm on disk that the folder is untouched.

## Out of scope

Reordering albums by hand (they are alphabetical), moving an album to another
parent, nesting deeper than two levels, and a search that matches an album's folder
path.
