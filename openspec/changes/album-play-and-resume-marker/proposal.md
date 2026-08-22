## Why

An album already remembers exactly where playback left off — `LoadedAlbum.playlist.current_track` names the last track played out of it, and that track's `last_position` names the second it stopped at — but nothing in the TUI lets the user act on either fact. There is no key that starts an album playing, and no way to tell, just by looking at a folded or reopened album, which file it will resume from. The user has to remember and manually re-select the track.

## What Changes

- `Space` on an album header starts playback of that album — from `current_track` at its `last_position` if the album has one, otherwise from its first track — reusing the existing `play_from_list` path, no new playback machinery. **Amended in a follow-up fix**: this only fires when the album isn't already the one playing; `Space` on the header of the album that *is* currently playing pauses/resumes it instead of restarting it, and `Space` on a *different* album's header now switches playback to it (previously there was no way to do that from a header once anything was playing). See `design.md`'s D1 for what broke and why.
- A track row shows a dim marker in its otherwise-blank play-icon slot when that row's track is its album's `current_track` and the album is not the one actually driving playback right now (`row_is_playing` is false for it). This marks "this is where the album will resume from" without implying it is currently playing.
- No new persisted fields, no numeric position display — the marker is glyph-only. Scope is explicitly limited to albums (`RowSource::Album`); the displayed (top-level) playlist's own `current_track` is not marked, by request.

## Capabilities

### New Capabilities

- `album-management`: no `openspec/specs/album-management/spec.md` exists yet (the earlier `track-album-management-fixes` change scoped its own delta under its change directory but was never archived into `openspec/specs/`), so this change's delta is authored as a new capability. Covers "play this album" as an action available from an album header, resuming from the album's own last-played track and position.
- `track-list-display`: likewise not yet present under `openspec/specs/`; authored fresh here. Covers the resume-point marker glyph on a track row when it is an album's `current_track` and that album isn't the one currently playing.

### Modified Capabilities

(none — both capability paths above are new to `openspec/specs/`)

## Impact

- `src/tui/input.rs`: `Space` handler (currently only pause/resume-or-`play_row`) gains an album-header branch; `Space` documentation in the help overlay (`src/tui/ui.rs` help lines) gains an entry under "On an album header".
- `src/tui/mod.rs`: new small helper to resolve an album's resume target (`current_track` index + `last_position`, or track 0), used by both the new `Space` handler and (for symmetry) nothing else — `play_from_list` already takes an explicit `start_pos`.
- `src/tui/ui.rs`: `render_track_table`'s per-row play-icon rendering (`src/tui/ui.rs:450`) gains a third state (playing / resume-marker / blank) alongside the existing `row_is_playing` check.
- No changes to `Playlist`/`Track` schema, no migration.
