## Purpose

Covers how the track table renders the rows of the displayed playlist and its albums — icons, styling, and other visual signals distinct from the underlying data — independent of what drives those signals.

## ADDED Requirements

### Requirement: Album resume-point marker
The system SHALL visually mark the track row within an album that playback of that album would resume from, whenever that track is not the one actually driving playback right now.

The marker SHALL be distinguishable from the "currently playing" indicator shown on a track that the playing session is actually running — a resume point is not a claim that anything is playing. The marker SHALL disappear from a row once that album's remembered track changes to a different one, or once that track is the one actually playing (at which point the existing playing indicator applies instead).

This marker applies only to tracks inside an album (`RowSource::Album`). The displayed playlist's own tracks are out of scope for this requirement.

#### Scenario: Folded album, never opened this session
- **WHEN** the track table renders a row belonging to an album that is folded, and that row's track is the album's remembered `current_track`, and the album is not the one currently playing
- **THEN** the row shows the resume-point marker instead of a blank play-icon slot

#### Scenario: Open album, cursor elsewhere
- **WHEN** the track table renders an open album's rows and the cursor is not on the row matching `current_track`
- **THEN** that matching row still shows the resume-point marker

#### Scenario: The remembered track is actually playing
- **WHEN** the album's `current_track` is the track actually driving playback right now
- **THEN** the row shows the existing playing indicator, not the resume-point marker

#### Scenario: A different album is playing
- **WHEN** album A's `current_track` names track X, and album B (a different album) is the one actually playing
- **THEN** track X's row in album A still shows the resume-point marker, since album A is not currently playing

#### Scenario: Album with no remembered track
- **WHEN** an album has no `current_track`
- **THEN** none of its rows show the resume-point marker
