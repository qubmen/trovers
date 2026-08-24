## Context

Since ADR-019, an album is `LoadedAlbum { name, path, playlist }` where `playlist` is an ordinary `Playlist` (`src/tui/mod.rs:264`). `playlist.current_track` already records the last track played out of *that specific list* (`src/tui/mod.rs:954-963`), and each `Track.last_position` already records where that track was left (`src/tui/mod.rs:936-943`, read back via `input::resume_start_pos`, `src/tui/input.rs:1276`). `play_from_list(source: RowSource, idx: usize, start_pos: Option<f64>)` (`src/tui/mod.rs:888`) already does everything needed to start playback of a given index within either the displayed playlist or an album, given a resolved `start_pos`.

Two things are missing, both purely in the TUI layer — no new persisted state:
1. A trigger that resolves "which index, which start_pos" for an album *as a whole* and calls `play_from_list`. Today every call to it is seeded from a row the cursor is already on.
2. A rendering signal in `render_track_table` (`src/tui/ui.rs:365`) for "this row is where the album will resume from," alongside the existing "this row is actually playing" (`row_is_playing`, `src/tui/ui.rs:225`).

See `proposal.md` for motivation and scope (albums only, marker-only, no numeric position).

## Goals / Non-Goals

**Goals:**
- Reuse `play_from_list` and the existing `current_track`/`last_position` fields exactly as they are; add no new `Playlist`/`Track` fields.
- Make the "play album" trigger and the "resume marker" share one resolution function, so they can never disagree about which track an album would resume from.

**Non-Goals:**
- Not touching the displayed (top-level) playlist's own `current_track` marking — explicitly out of scope per the proposal.
- Not adding any numeric position display (no mm:ss anywhere) — marker glyph only.
- Not changing what `Enter` does on an album header (fold/unfold stays).

## Decisions

### D1: `Space` on an album header triggers "play this album"

> **Superseded.** Two bugs surfaced once this shipped, both traced to the
> same cause: `Space`'s meaning depended on cursor position (header vs.
> track) rather than on whether the row under the cursor is the one actually
> playing. First, pausing while sitting on the *playing* album's own header
> re-ran "play this album from its resume point" instead of pausing — mpv
> jumped back a few seconds instead of stopping. Fixing that by always
> pause-toggling whenever a player is active created the second bug: there
> was then no way to switch playback to a *different* album from its header
> at all — Space on any header, playing or not, just paused whatever was
> already going.
>
> The rule that replaced this section's "always play" and the
> always-pause-if-a-player-exists fix that followed it: `Space` pauses/resumes
> only when the row under the cursor (a track, or a header's resolved resume
> target — D2) is the one `self.playing` already names; otherwise it starts
> that row, switching playback to it exactly as `Enter` already does for a
> plain track row. This is symmetric with `Enter` and needs no special case
> for headers versus tracks — "starting a new album" and "starting a
> different track" are the same action once the check is "is this what's
> already playing," not "is this a header." See
> `track-album-management-fixes`'s two follow-up commits on
> `feat/album-playback-resume` for the actual fix and its tests
> (`space_on_a_header_pauses_instead_of_restarting_when_the_album_is_already_playing`,
> `space_on_a_different_albums_header_switches_playback_to_it`).
>
> The paragraphs below describe the original (superseded) reasoning as
> history.

`Space`'s handler today (`src/tui/input.rs:249-265`) is: if a player is running, toggle pause; otherwise `play_row(app.selected)`. `play_row` returns `false` on a header row (`row_at` only matches `VisibleRow::Track`), so `Space` on a header currently does nothing when idle, and toggles pause (of whatever else is playing) when something is already playing.

New behavior: when the cursor is on an album header (`app.album_of(app.selected)` is `Some`), `Space` always means "play this album" — resolve its resume target (D2) and call `play_from_list(RowSource::Album(album), idx, start_pos)`, regardless of whether something else is currently playing (starting a new album is itself the pause-equivalent action for that row; there is nothing to "pause" about a header). Outside of a header row, `Space`'s existing behavior (pause toggle / play the row) is unchanged.

**Alternative considered:** a new dedicated key (e.g. `p`) for "play album," leaving `Space` untouched on headers. Rejected: `Space` already means "the play-related action for whatever's under the cursor" everywhere else in the track list, `p` is not otherwise reserved, but overloading `Space` keeps one consistent mental model ("space plays/resumes the thing under the cursor") instead of adding a second key that means nearly the same thing.

### D2: One helper resolves an album's resume target; both `Space` and the marker call it

```rust
/// The album's resume target: the index of its `current_track` within its own
/// list and that track's `last_position`, or index 0 / `None` if it has no
/// `current_track` or that track no longer exists in the album.
fn album_resume_target(loaded: &LoadedAlbum, library: &Library) -> Option<(usize, Option<f64>)>
```

Returns `None` only when the album has zero tracks (nothing to play/mark). Otherwise:
- If `current_track` is `Some(id)` and `id` is still in `playlist.tracks`, returns `(that index, library.get(id).and_then(resume_start_pos))`.
- Otherwise returns `(0, None)` — first track, from the start.

Both the `Space` handler (D1) and the row marker (D3) call this, so "which row does 'play album' start on" and "which row gets marked" can never drift apart — they are, by construction, the same row.

**Alternative considered:** compute the target inline at each of the two call sites. Rejected: the two would need to agree on the exact same fallback rule (stale `current_track` → track 0), and duplicating it risks the marker and the actual resume point silently diverging after a future edit to one copy.

### D3: Row marker checks `current_track` directly, not through `album_resume_target`

`render_track_table`'s per-row play-icon (`src/tui/ui.rs:450`, `let play_icon = if is_playing { "▶" } else { " " };`) gains a middle case:

```rust
let play_icon = if is_playing {
    "▶"
} else if is_resume_marker {
    "‣"
} else {
    " "
};
```

where `is_resume_marker` is true iff `source` is `RowSource::Album(album)`, `!is_playing` for this row, and `app.albums[album].playlist.current_track.as_deref() == Some(id)` — a direct comparison against the row's own id, **not** a call to `album_resume_target`.

**This was tried the other way first and a test caught the bug:** an earlier draft computed `is_resume_marker` by comparing `index` to `album_resume_target(...).map(|(i, _)| i)`. `album_resume_target` (D2) deliberately falls back to `(0, None)` whenever an album has no `current_track` (or a stale one) — correct for D1's "Space with nothing remembered still has to start somewhere," but wrong here: it made a never-played album's first track render as marked, contradicting the spec's explicit "no `current_track` → no marker" requirement. `resume_marker_shows_on_the_albums_current_track_when_it_is_not_playing`'s sibling test, `no_resume_marker_when_the_album_has_no_current_track`, failed against that draft. The fix is the direct-comparison form above: it has no fallback, so "nothing remembered" naturally means "nothing marked."

`album_resume_target` remains the single source of truth for D1 (where Space starts playback); D3 no longer calls it, since "what should play" and "what should be marked" are genuinely different questions once there's no real memory to act on — the first still needs an answer (track 0), the second doesn't.

`‣` (single right-pointing triangle, distinct from `▶`'s filled double-width look and from the sidebar's `▶`/`▼` and the album header's `▸`/`▾`) is used at reduced emphasis (dim, not bold, no background) so it reads as a passive marker, not an action indicator.

**Alternative considered:** reuse `▸` (the album header's own closed-glyph) for the row marker too, for visual consistency with "this is where you'd un-fold to." Rejected: `▸` already means "closed disclosure triangle" one column over in the same table (album header rows), and reusing it on a track row risks reading as "this row can be expanded," which it can't.

## Risks / Trade-offs

- **[Risk] `Space` on a header now always (re)starts the album, even if the user meant to just glance at the header while something else plays** → this risk materialized exactly as written and is what D1's superseding note above fixes: `Space` now (re)starts a header's album only when that album isn't already the one playing, pausing/resuming it otherwise.
- **[Trade-off] The marker and "is this album empty" are both resolved via the same `Option`-returning helper, so an empty album's header still supports `Space` being pressed harmlessly** → accepted: `Space` on an empty album's header is a no-op, matching `play_row`'s existing "a header names a group, not a track" contract for `Enter`/`Space` elsewhere.

## Migration Plan

No on-disk format changes and no new fields — `current_track` and `last_position` already exist and are already populated by today's binary. Ship as a normal release; rollback is a plain revert.
