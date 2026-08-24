## Purpose

Governs how albums and the displayed playlist are created, populated, and kept
consistent as independently addressable, in-memory lists of tracks.

## ADDED Requirements

### Requirement: A move or add into an in-memory list is reflected immediately
Moving a track to another list, or adding a track (by URL or local file) to a
specific list, SHALL update that list's in-memory copy when one is currently
loaded — the displayed playlist, or one of the albums loaded under it — not
only the file on disk.

#### Scenario: Moving a track into a loaded album
- **WHEN** the user moves a track into an album that is currently loaded under
  the displayed playlist
- **THEN** the track's row appears in that album's group in the track list
  immediately, without switching to another playlist and back

#### Scenario: Moving a track out of an album into the displayed playlist
- **WHEN** the user moves a track from one of the displayed playlist's albums
  into the displayed playlist itself
- **THEN** the track's row appears among the displayed playlist's own tracks
  immediately

#### Scenario: Adding a URL targeted at a loaded album
- **WHEN** the user adds a track by URL and chooses one of the displayed
  playlist's own albums as the destination
- **THEN** once the track's metadata arrives, its row appears in that album's
  group in the track list immediately

### Requirement: Shuffle and loop mode follow the row's own list
Toggling shuffle or loop mode SHALL change the setting of the list the
selected row belongs to — an album's own setting when the cursor is on one of
that album's tracks, the displayed playlist's setting otherwise — never a
different list than the one the cursor is in.

#### Scenario: Toggling shuffle while browsing inside an album
- **WHEN** the selected row is a track belonging to one of the displayed
  playlist's albums and the user toggles shuffle
- **THEN** only that album's own shuffle setting changes; the displayed
  playlist's shuffle setting is unchanged

#### Scenario: Toggling loop mode while browsing inside an album
- **WHEN** the selected row is a track belonging to one of the displayed
  playlist's albums and the user toggles loop mode
- **THEN** only that album's own loop mode changes; the displayed playlist's
  loop mode is unchanged

#### Scenario: Toggling shuffle on the displayed playlist's own track
- **WHEN** the selected row is one of the displayed playlist's own tracks (not
  inside an album) and the user toggles shuffle
- **THEN** the displayed playlist's own shuffle setting changes, as before

### Requirement: An album can be created without a folder
The user SHALL be able to create a new, empty album under the displayed
playlist without pointing at a folder on disk.

#### Scenario: Creating an empty album
- **WHEN** the user creates a new album under the displayed playlist by name,
  without selecting a folder
- **THEN** an empty album is created, listed under the displayed playlist,
  with no folder linked for rescanning

### Requirement: A single local file can be added to a chosen list
The user SHALL be able to add one local media file directly to a specific
existing list — the displayed playlist or one of its albums — without
scanning a whole folder.

#### Scenario: Adding one file to an existing album
- **WHEN** the user adds a single local file and chooses an existing album as
  the destination
- **THEN** exactly one new track row for that file is added to the chosen
  album, and no other file is scanned or affected

### Requirement: A folder's contents can be merged into any chosen existing list
The user SHALL be able to import a folder's contents into any existing list
they choose — a normal playlist or an existing album — regardless of whether
that list is already linked to that folder for rescanning. Merging follows the
existing import rules: nothing already listed is deleted or reordered, and a
file already known to the library reuses its existing track document.

#### Scenario: Adding a second, different folder's contents to an existing album
- **WHEN** the user imports a folder into an album that is already linked to a
  different folder
- **THEN** the new folder's playable files are appended to the album without
  removing or reordering any track already in it, and the album's original
  rescan folder link is unchanged

#### Scenario: Re-adding a file that already has a track document
- **WHEN** an imported file resolves to a track already known to the library
- **THEN** the existing track document is reused and brought up to date rather
  than duplicated
