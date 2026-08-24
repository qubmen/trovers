## Purpose

Covers how albums under a displayed playlist are created, organized, and — with this change — started playing, independent of the track list's own display and browsing state.

## ADDED Requirements

### Requirement: Play an album from its header
The system SHALL let the user start playback of an album directly from its header row, without first navigating into the album and selecting a track.

When the album has a remembered last-played track (`current_track`) that still exists in the album, playback SHALL start from that track, resuming at its remembered position (`last_position`) if it has one. When the album has no remembered last-played track, or that track no longer exists in the album, playback SHALL start from the album's first track, from the beginning.

Starting an album this way SHALL behave exactly as if the user had navigated to that resolved track's row and started it directly: the same playlist (the album's own file) becomes the one driving playback, and next/previous and auto-advance stay scoped to the album.

#### Scenario: Album has a remembered track with a saved position
- **WHEN** the user triggers "play album" on an album header whose `current_track` names a track with `last_position` of 95 seconds
- **THEN** playback starts on that track at 95 seconds, and the album becomes the list driving playback

#### Scenario: Album has never been played
- **WHEN** the user triggers "play album" on an album header with no `current_track`
- **THEN** playback starts on the album's first track from the beginning

#### Scenario: Remembered track was removed from the album
- **WHEN** the user triggers "play album" on an album header whose `current_track` no longer matches any track in the album
- **THEN** playback starts on the album's first track from the beginning

#### Scenario: Empty album
- **WHEN** the user triggers "play album" on an album header with zero tracks
- **THEN** nothing happens — there is no track to play
