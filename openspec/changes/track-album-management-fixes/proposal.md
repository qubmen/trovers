## Why

ADR-019 generalized track-list operations (delete, reorder within a list, move's *source*, playback stepping) to route through the row's owning list — the displayed playlist or one of its loaded albums — via `RowSource`/`source_playlist`. Two operations were left out of that generalization and now silently act on the wrong list: moving/adding a track into a list that happens to already be loaded in memory doesn't update that in-memory copy (only `apply_import` was fixed for this), and shuffle/loop-mode toggles always hit the displayed playlist even when the cursor sits inside an album. Separately, album creation and content-adding are needlessly coupled to a one-shot folder scan — there is no way to create an empty album, add a single file, or add a second folder's contents into an already-existing album — even though an album is architecturally just an ordinary playlist file (ADR-016) with no structural tie to a folder. A rendering bug in the track table (duration column one character too narrow, no gap before the scrollbar) rounds out this pass.

## What Changes

- Fix: moving a track (`m`) or adding a URL (`a`) into a list that is already loaded in memory (the displayed playlist itself, or one of its own loaded albums) now updates that in-memory copy immediately, the same way `apply_import` already does — so the row appears on screen without switching away and back.
- Fix: shuffle (`r`) and loop mode (`l`) now act on the list the selected row actually belongs to (an album's own file when the cursor is on one of its tracks), not unconditionally on the displayed playlist.
- New: an album can be created manually — empty, with no folder required — the same way `N` creates a normal playlist.
- New: a single local file can be added directly to any specific existing list (a normal playlist or an existing album), without going through a whole-folder scan.
- New: a folder's contents can be added into any specific existing list chosen by the user, decoupled from the `source_folder` identity check — previously, importing a folder that didn't exactly match a list's already-linked folder could only create a new sibling album, never merge into an existing one.
- Fix: the track table's duration column is widened to fit `HH:MM:SS` (currently clips the last digit on every track an hour or longer), and a one-column gap is reserved between the table and the scrollbar so content never sits flush against it.
- **BREAKING**: none. All changes are additive or bug fixes; no on-disk format changes, no existing keybinding removed or repurposed.

## Capabilities

### New Capabilities
- `album-management`: creating, populating, and keeping in sync albums and the displayed playlist as independently addressable lists — manual album creation, adding a single file or a folder's contents to a chosen existing list independent of any folder-identity check, keeping in-memory copies (the displayed playlist, loaded albums) consistent whenever they are the target of a move or an add, and routing per-list settings (shuffle, loop mode) to the list the cursor is actually in rather than always the displayed playlist.
- `track-list-display`: layout guarantees for the track table — column widths sized to fit the content they display, and spacing between the table and the scrollbar.

### Modified Capabilities
(none — these are the first specs recorded for this project)

## Impact

- `src/tui/mod.rs`: `move_track_to_playlist`, the `TaskMsg::MetaReady` handler, `import_target_for`/`apply_import`, plus new helpers for manual album creation and single-file/targeted-folder adds.
- `src/tui/input.rs`: the `r`/`l` key handlers (route through the row's owning list), new input flow(s) for manual album creation and single-file add.
- `src/tui/ui.rs`: track table column constraints (`widths`) and the scrollbar/table layout split.
- `src/library_import.rs`: a single-file counterpart to `scan`/`merge_scan` so one path can be folded in without a directory walk.
- Tests: `src/tui/ui_test.rs`, `src/library_import_test.rs`.
