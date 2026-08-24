## Context

Since ADR-019 ("albums in the track list"), the displayed playlist and its
albums are separate in-memory objects — `App.playlist` and
`App.albums: Vec<LoadedAlbum>` — each backed by its own file. A row on screen
names which of these it came from via `RowSource` (`Own` or `Album(usize)`),
and `App::source_playlist(source)` resolves that to a `(&Playlist, &Path)`.
Most mutations already go through this: `remove_row`, the `J`/`K` swap,
`step_track` (`n`/`b`), and `apply_import` all resolve the row's owning list
first and mutate/save that specific in-memory copy.

Two call sites never adopted this pattern and instead hard-code
`self.playlist`:
- `move_track_to_playlist`'s *target* side (the source side is already
  correct) resolves the target path from `available_playlists` and always
  does `Playlist::load(&target_path)` — a fresh read from disk — even when
  that path is `self.playlist_path` or one of `self.albums[i].path`.
- `TaskMsg::MetaReady`'s "add URL to a non-displayed target" branch does the
  same: `Playlist::load(&owning_path)` regardless of whether `owning_path` is
  one of the lists already held in memory.
- The `r` (shuffle) and `l` (loop mode) handlers in `handle_tracklist` never
  consult `row_group`/`source_playlist` at all; they always read and write
  `self.playlist.shuffle` / `self.playlist.loop_mode`.

Separately, every way to put tracks into a list currently starts from a
folder scan (`library_import::scan`/`merge_scan`, driven by `F` and
`import_target_for`'s folder-identity matching). There is no path that
creates an album with zero files, adds exactly one file, or merges a folder
into a list whose `source_folder` doesn't already match it.

See `proposal.md` for the motivation and the full list of user-facing
changes; this document covers the approach.

## Goals / Non-Goals

**Goals:**
- One shared mechanism for "write to the list at this path, wherever it
  lives" that every add/move call site uses, so this class of bug cannot
  reappear at a sixth call site.
- Manual album creation and single-file/targeted-folder add reuse the
  existing `Playlist`/`Library`/`merge_scan` machinery — no new on-disk
  concepts, no new `PlaylistKind`.
- Keep `F`'s current one-key, no-prompt-needed behavior working exactly as
  it does today when the user does not ask for anything different.

**Non-Goals:**
- Not auditing or changing whether the move-to-playlist context menu (`m`)
  and the URL-add target picker should exclude foreign albums from their
  lists of choices — both already show every entry in `available_playlists`
  today, unfiltered, and this change does not revisit that filtering.
- Not fixing the pre-existing ~2-character mismatch between
  `render_track_table`'s manually computed `title_width` constant and the
  `Table`'s own column-width array. It under-uses the title column by a
  couple of characters but doesn't clip or overlap anything, so it's outside
  this change's scope (which is about clipping and overlap).
- Not adding drag-and-drop, multi-file selection, or a file-picker dialog —
  "add a single file" means typing/pasting one path, the same way folder
  import already works.

## Decisions

### D1: A single `with_list_at` helper resolves "the list at this path, wherever it lives"

Add one method on `App`:

```rust
/// Run `f` against the `Playlist` at `path`, wherever it currently lives —
/// `self.playlist` if `path` is the displayed playlist, the matching
/// `self.albums[i]` if it's one of the loaded albums, or a copy read fresh
/// from disk otherwise — then persist whichever one it was and, for the
/// first two cases, rebuild `self.rows` so the change is visible immediately.
fn with_list_at<R>(&mut self, path: &Path, f: impl FnOnce(&mut Playlist) -> R) -> Result<R>
```

`apply_import`'s three `ImportTarget::Existing` arms (own / loaded album /
on-disk-only) already implement exactly this dispatch by hand. `D1` lifts
that dispatch out into the shared helper and rewrites `apply_import` to call
it, then reuses it at the two broken call sites:

- `move_track_to_playlist`: replace the direct `Playlist::load(&target_path)`
  / save with `self.with_list_at(&target_path, |pl| pl.add_track(id))`. The
  existing "create the file if the entry is stale" fallback stays as a
  pre-step before the call, since `with_list_at`'s job is dispatch, not
  file creation.
- `TaskMsg::MetaReady`: replace the `owning_path`-not-displayed branch's
  `Playlist::load`/save pair with two `with_list_at` calls — one read-only
  (does this list already contain `id`?) and, once the track document is
  written and the download started, one that pushes the id and saves.

**Alternative considered:** thread a `&mut Playlist` reference through from
each call site's own `match` on `RowSource`/path, as `remove_row` and the
`J`/`K` swap already do for the *source* side. Rejected because the target
side, unlike the source side, is not already holding a `VisibleRow` to match
on — it's an arbitrary path chosen from `available_playlists` — so the
dispatch logic (own / loaded album / on-disk) has to exist somewhere as its
own thing regardless; naming it once is strictly less code than the
`apply_import`-style match block copied a second and third time.

### D2: `r`/`l` resolve the acting list via `row_group`, exactly like `step_track` already does

`step_track` (`src/tui/input.rs:424`) already calls
`app.row_group(app.selected)` to get `(RowSource, Vec<usize>)` for the
selected row, then `app.source_playlist(source)` to read that list's
`shuffle`. The `r`/`l` handlers get the same treatment: resolve
`row_group(app.selected)` first; if it names an album, mutate and save that
`LoadedAlbum`'s `playlist` via `with_list_at(&loaded.path, ...)`; otherwise
fall through to today's `self.playlist` behavior. On a header row (`r`
already means rename there) or when nothing is selected, behavior is
unchanged.

### D3: Manual album creation reuses `apply_import`'s `NewAlbum` bookkeeping, minus the scan

Extract the tail of `apply_import`'s `ImportTarget::NewAlbum` arm — build an
empty `Playlist` with `kind = Album`, resolve `parent` the same way
`import_target_for` already does (`self.playlist.parent.clone()` when the
displayed playlist is itself an album, so nesting never exceeds two levels;
otherwise `Some(self.displayed_playlist_name())`), save it, push it into
`available_playlists`, and — since the parent is always the displayed
playlist or its grandparent, never anything else — push it into
`self.albums` and `rebuild_rows` — into `App::create_album(&mut self, name: &str) -> Result<PathBuf>`.
`apply_import`'s `NewAlbum` arm calls it, then runs `merge_scan` against the
result; the new manual-creation flow calls it and stops, leaving the album
at 0 tracks.

Entered with a new key, `Shift+A` (`A`), parallel to `N` (new playlist):
`InputMode::NewAlbum`, typed name, `Enter` creates via `create_album`, `Esc`
cancels. Name validation reuses `validate_playlist_name`, same as `N` and
`AlbumRename`.

### D4: Single-file add is a new, smaller sibling of `merge_scan`, not a one-file call to it

`merge_scan`'s two extra jobs beyond "add rows" — marking rows whose file
vanished from `root`, and stamping `source_folder` — are folder concepts.
Reusing `merge_scan` for a single file would need a synthetic `root` (the
file's own parent directory, most likely) and would risk marking sibling
files in that same folder as "vanished" the next time that folder happens to
be scanned as a whole, since `merge_scan`'s vanished-check is scoped by
`track_is_under(track, root)`. Simpler and more honest: a new function in
`library_import.rs`,

```rust
pub fn add_single_file(library: &mut Library, list: &mut Playlist, path: &Path) -> SingleAddOutcome
```

that runs the existing `local_id` → `probe` → `refresh`-or-`local_track`
logic for exactly one path (reusing `refresh` and `local_track`, both made
`pub(crate)`), pushes the id onto `list.tracks` if not already present, and
touches nothing else — no `source_folder`, no vanished-marking. `probe` is
async (it may spawn `ffprobe`); the call site follows the same
scan-in-background-task, merge-on-arrival shape `import_folder`/
`apply_import` already use, just with one file instead of a `Vec`.

### D5: A folder can target any existing list; `source_folder` is set once, not overwritten

Today `import_target_for` auto-picks the target (an existing list whose
`source_folder` already matches the typed path, or else a new sibling
album), and the user has no way to override that pick. `FolderInput` gains
the same target-picker `UrlInput` already has: a `target_list_for_add:
Option<String>` field, `Tab`-cycled through `available_playlists`' names,
defaulting to `None` ("Auto" — today's `import_target_for` behavior,
unchanged for anyone who never touches Tab). Picking an explicit name
bypasses `import_target_for` entirely and merges into that list via
`with_list_at` + `merge_scan`, whatever its `source_folder` says.

Because a list can now receive files from more than one folder,
`merge_scan`'s trailing `album.source_folder = Some(root.to_path_buf());`
changes to `album.source_folder.get_or_insert(root.to_path_buf());` — the
first folder a list is ever populated from remains the one `R` rescans;
importing a second, different folder into the same list does not silently
repoint that link. A list's `source_folder` was always "the folder to
rescan," never "the folder it must contain only," so this is a narrowing of
an accident, not a behavior anyone relied on — nothing before this change
could have merged a second folder in, so no existing album has ever had its
link overwritten by one.

The same `target_list_for_add`, and the same "Auto = displayed playlist"
default, is reused when the typed/pasted path resolves to a file rather than
a directory (`path.is_dir()` vs a file check added alongside it), routing to
`D4`'s `add_single_file` instead of `scan_and_probe`/`merge_scan`. One prompt,
one target picker, branching only on what kind of path was given.

### D6: Track table — widen the duration column, reserve a real gap before the scrollbar

`format_duration` already produces `HH:MM:SS` (8 characters) for anything an
hour or longer; the `widths` array's `Constraint::Length(7)` becomes
`Constraint::Length(8)`. `render_track_table`'s `table_area` currently
reserves exactly one column (`inner.width.saturating_sub(1)`) for the
`scrollbar_area` that immediately follows it, with nothing in between;
reserving two (`saturating_sub(2)`) leaves the second-to-last column of
`inner` painted by neither the `Table` nor the `Scrollbar`, i.e. blank, which
is the gap. `title_width`'s manual subtraction updates from `7` to `8` to
match, so the title column doesn't shrink by the same character the duration
column just gained.

## Risks / Trade-offs

- **[Risk] `with_list_at`'s disk-read branch is called twice in the
  `MetaReady` rewrite** (once to check for a duplicate id, once to add) for
  a target that isn't in memory → **Mitigation**: this only happens when
  adding a URL to a playlist other than the displayed one or one of its
  albums — already the rarer path today — and it's one small TOML file, not
  a directory walk; the cost is a few extra milliseconds, not a correctness
  problem.
- **[Risk] Widening the duration column by one character narrows the title
  column by one character on every existing screen** → **Mitigation**: this
  is the same one character the column was silently stealing from every
  hour-plus track's duration before; net available width for text is
  unchanged, it's only redistributed correctly.
- **[Trade-off] `add_single_file` duplicates a little of `merge_scan`'s
  per-file logic** (calling the same `refresh`/`local_track` helpers, but
  with its own top-level shape) rather than making `merge_scan` handle a
  one-file case as a special mode → accepted per D4: the two have genuinely
  different vanished-tracking and `source_folder` semantics, and forcing one
  function to cover both would need a flag parameter that callers could pass
  wrong.

## Migration Plan

No on-disk format changes: `Playlist`/`Track` gain no new fields, and
`source_folder.get_or_insert` only changes behavior the next time a *second*
folder is merged into a list that already has one linked — every album that
exists today keeps exactly the `source_folder` it already has. Ship as a
normal release; rollback is a plain revert, since nothing written by the new
code paths (a manually created album, a single-file row, a second folder's
rows) is unreadable by the old binary — they're all ordinary `Playlist`/
`Track` documents.
