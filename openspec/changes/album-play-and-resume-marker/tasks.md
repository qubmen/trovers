## 1. Resume-target resolution

- [x] 1.1 Add `album_resume_target(loaded: &LoadedAlbum, library: &Library) -> Option<(usize, Option<f64>)>` to `src/tui/mod.rs` per design D2, and verify with a unit test covering: `current_track` present and matching a track with `last_position` > 0; `current_track` present but matching a track with no `last_position`; `current_track` absent; `current_track` naming an id no longer in the album; zero-track album.

## 2. Play album from its header

- [x] 2.1 In `src/tui/input.rs`'s `Space` handler, branch on `app.album_of(app.selected)`: when `Some(album)`, resolve `album_resume_target`, and if it returns `Some((idx, start_pos))` call `app.play_from_list(RowSource::Album(album), idx, start_pos)`; if `None` (empty album), do nothing. Leave the existing pause-toggle/`play_row` behavior unchanged for every other row. Verify by pressing `Space` on an album header in a running build: an album never played starts at track 1; an album with a `current_track`/`last_position` resumes there; an empty album's header does nothing.
- [x] 2.2 Add a `[spc] play album` line under the "On an album header" section of the help overlay in `src/tui/ui.rs` (alongside the existing `[enter] open/close`), and verify by opening the in-app help (`?`) and reading the new line.

## 3. Resume-point marker in the track table

- [x] 3.1 In `src/tui/ui.rs`'s `render_track_table`, compute `is_resume_marker` for `RowSource::Album(album)` rows per design D3 (using `album_resume_target` from task 1.1) and extend the `play_icon` selection to `▶` / `‣` / `" "`, styled dim (not bold, no background) for the `‣` case. Verify with a rendering/unit test (or, if the existing test suite renders to a `Buffer`, an assertion on the emitted cell) confirming: the album's `current_track` row shows `‣` when the album isn't playing; that row shows `▶` instead once the album is the one actually playing; a different album's row for the same track id (if duplicated) is unaffected; an album with no `current_track` shows no marker on any row; the displayed (top-level) playlist's own rows never show `‣`.
  - Design correction made during implementation: `is_resume_marker` ended up comparing `current_track` directly by id, not via `album_resume_target` — see design.md D3 for why the first attempt (index-equality through the fallback-including helper) failed its own test.

## 4. Verification

- [x] 4.1 Run the existing test suite (`cargo test`) and confirm no regressions. 616 passed, 0 failed, 3 ignored (pre-existing, unrelated to this change).
- [ ] 4.2 Manual pass: create/open an album, play a track partway through, switch to another list, reopen the album folded — confirm the marker appears on the right row; press `Space` on the header — confirm playback resumes at the right track and position; let the marked track become the one actually playing — confirm the marker is replaced by `▶`.
