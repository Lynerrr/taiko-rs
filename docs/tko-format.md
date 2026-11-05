# The `.tko` map format

A `.tko` map is an `.osu` map that uses the client's own logic on top of an osu!-compatible
chart. It is made in the map editor (Gimmicks panel, Keyframes page, "Make .tko") and plays only
in this client; the `.osu` it was made from stays as it was and plays the same in osu!stable.

## File

Text, a superset of `.osu`:

```
osu file format v14
[General] ... [Metadata] ... [Difficulty] ... [Events] ... [TimingPoints] ... [HitObjects] ...

[Metadata]
...
TkoVersion: 1        <- the format version (Version: is the difficulty name)

[Gimmick]
{"field":{...},"groups":[...]}   <- one line of JSON, the last section
```

Everything but `TkoVersion` and `[Gimmick]` is the `.osu` format, so stars, pp, the song select
and the editor's stable-compatible rounding all work on a `.tko` the way they do on an `.osu`.

## Versions

- A file with a `TkoVersion` newer than the game's is refused with a message to update the game
  (never guessed at).
- An older one is brought up to date by the chain in `tko::format::MIGRATIONS` when it is read.
  Raising `tko::TKO_VERSION` needs a migration there and a fixture of the old format in the tests
  (`a_v1_fixture_keeps_reading` pins version 1).
- A `.tko` file without `TkoVersion` is an error.
- The editor writes the current version, and never rewrites a file it cannot read.

## Gimmick, version 1: visual only

A gimmick never changes when a note is hit or how it is judged, so scores, pp and replays of a
`.tko` map are as fair as those of its `.osu` (the tests judge a map with and without a gimmick
and compare).

```
{
  "field":  { "<property>": { "keys": [ {"t": ms, "v": value, "ease": "<name>"} ] } },
  "groups": [ { "name": "...", "notes": [ms, ...],
                "tracks": { "<property>": { "keys": [...] } } } ]
}
```

- Keys sit at chart time (ms, as in the `.osu`), sorted. A key's `ease` shapes the segment from it
  to the next; before the first key a track holds its first value, after the last its last.
- Easings: `linear`, `step`, `quad_in/out/in_out`, `cubic_in/out/in_out`, `sine_in/out/in_out`,
  `back_in/out`, `elastic_out`, `bounce_out`.
- Playfield properties: `x`, `y` (virtual units), `rotation` (degrees, about the hit target),
  `zoom`, `scroll` (a multiple of the map's own scroll speed; what is drawn, not when a note is
  hit).
- Group properties: `dx`, `dy`, `scale`, `opacity`, `hidden` (1 hides the note; it can still be
  hit). A group's notes are found by their start time. Don and kat notes are animated; rolls and
  swells are not yet.
- Limits (a file can't make the player hold an unbounded amount): 10,000 keys per track, 256
  groups, 200,000 grouped notes in all, 4 MiB of JSON. Values are held inside each property's range
  and unknown properties are dropped when a file is read.
