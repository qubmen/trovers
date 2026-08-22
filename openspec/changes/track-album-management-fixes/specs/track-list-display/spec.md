## Purpose

Governs the layout of the track table so that every column shows its content
in full and never collides with the scrollbar.

## ADDED Requirements

### Requirement: The duration column fits its longest format
The track table's duration column SHALL be wide enough to display the
`HH:MM:SS` format in full, for any track an hour or longer, without clipping
any character.

#### Scenario: A track over an hour long
- **WHEN** a row's track duration is one hour or longer
- **THEN** its duration is rendered as `HH:MM:SS` in full, with no digit
  clipped

#### Scenario: A track under an hour long
- **WHEN** a row's track duration is under an hour
- **THEN** its duration is rendered as `MM:SS` in full, unchanged from today

### Requirement: A visible gap separates the track table from the scrollbar
The track table SHALL leave at least one blank column between its rightmost
text column and the scrollbar, so no rendered character is ever adjacent to
or overlapped by the scrollbar track or thumb.

#### Scenario: The track list is long enough to scroll
- **WHEN** the displayed list has more rows than fit on screen and the
  scrollbar is drawn
- **THEN** there is a visible blank column between the duration column and
  the scrollbar for every row
