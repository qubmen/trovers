## 1. Resume-target resolution

- [x] 1.1 Add `album_resume_target(loaded: &LoadedAlbum, library: &Library) -> Option<(usize, Option<f64>)>` to `src/tui/mod.rs` per design D2, and verify with a unit test covering: `current_track` present and matching a track with `last_position` > 0; `current_track` present but matching a track with no `last_position`; `current_track` absent; `current_track` naming an id no longer in the album; zero-track album.

## 2. Play album from its header

- [x] 2.1 In `src/tui/input.rs`'s `Space` handler, branch on `app.album_of(app.selected)`: when `Some(album)`, resolve `album_resume_target`, and if it returns `Some((idx, start_pos))` call `app.play_from_list(RowSource::Album(album), idx, start_pos)`; if `None` (empty album), do nothing. Leave the existing pause-toggle/`play_row` behavior unchanged for every other row. Verify by pressing `Space` on an album header in a running build: an album never played starts at track 1; an album with a `current_track`/`last_position` resumes there; an empty album's header does nothing.
- [x] 2.2 Add a `[spc] play album` line under the "On an album header" section of the help overlay in `src/tui/ui.rs` (alongside the existing `[enter] open/close`), and verify by opening the in-app help (`?`) and reading the new line.
  - **Reverted in §5**: removed once `Space` stopped being header-specific behavior — the general `[spc] play/pause` line already covers it, and a dedicated header line was now misleading (it no longer "always" plays; see §5).

## 3. Resume-point marker in the track table

- [x] 3.1 In `src/tui/ui.rs`'s `render_track_table`, compute `is_resume_marker` for `RowSource::Album(album)` rows per design D3 (using `album_resume_target` from task 1.1) and extend the `play_icon` selection to `▶` / `‣` / `" "`, styled dim (not bold, no background) for the `‣` case. Verify with a rendering/unit test (or, if the existing test suite renders to a `Buffer`, an assertion on the emitted cell) confirming: the album's `current_track` row shows `‣` when the album isn't playing; that row shows `▶` instead once the album is the one actually playing; a different album's row for the same track id (if duplicated) is unaffected; an album with no `current_track` shows no marker on any row; the displayed (top-level) playlist's own rows never show `‣`.
  - Design correction made during implementation: `is_resume_marker` ended up comparing `current_track` directly by id, not via `album_resume_target` — see design.md D3 for why the first attempt (index-equality through the fallback-including helper) failed its own test.

## 4. Verification

- [x] 4.1 Run the existing test suite (`cargo test`) and confirm no regressions. 616 passed, 0 failed, 3 ignored (pre-existing, unrelated to this change).
- [ ] 4.2 Manual pass: create/open an album, play a track partway through, switch to another list, reopen the album folded — confirm the marker appears on the right row; press `Space` on the header — confirm playback resumes at the right track and position; let the marked track become the one actually playing — confirm the marker is replaced by `▶`.

## 5. Follow-up fixes (found after this shipped)

Real usage surfaced two bugs in task 2.1's "`Space` on a header always means play this album" rule, both from the same root cause: the rule keyed off cursor position (header vs. track) instead of "is this row already what's playing." See `design.md`'s D1 for the full account.

- [x] 5.1 Bug: pausing (pressing `Space` again) while sitting on the header of the album that is *already playing* re-ran the play-this-album branch instead of pausing — it respawned mpv at the album's saved `current_track`/`last_position`, a few seconds behind the live position, which read as "seeks back and keeps playing" rather than stopping. Fixed by checking `app.player.is_some()` before the header branch. Test: `space_on_a_header_pauses_instead_of_restarting_when_the_album_is_already_playing`.
- [x] 5.2 Bug (introduced by 5.1's fix): with pause now checked first and unconditional whenever a player is active, there was no way left to switch playback to a *different* album from its header — `Space` on any header, while anything played, just paused it. Fixed by replacing the "header vs. track" branch with a "same row as what's playing vs. a different row" check: `Space` pauses/resumes only when the cursor's row (or a header's resolved resume target) is the exact `(path, track_id)` `self.playing` already names; otherwise it starts that row, switching playback to it exactly as `Enter` already does for a track. Tests: `space_on_a_different_albums_header_switches_playback_to_it`, `space_on_the_playing_track_pauses`, `space_on_a_different_track_switches_playback_to_it`.
- [x] 5.3 Removed the `[spc] play album` help-overlay line added in 2.2: `Space` is no longer a header-specific action, so the general "On the track list" section's `[spc] play/pause` line already describes it.
