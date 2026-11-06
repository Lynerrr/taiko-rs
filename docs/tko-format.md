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
TkoVersion: 3        <- the format version (Version: is the difficulty name)

[Gimmick]
{"field":{...},"groups":[...],"marks":[...],"graph":{...},"clips":[...]}   <- one line of JSON, the last section
```

Everything but `TkoVersion` and `[Gimmick]` is the `.osu` format, so stars, pp, the song select
and the editor's stable-compatible rounding all work on a `.tko` the way they do on an `.osu`.

## Versions

- A file with a `TkoVersion` newer than the game's is refused with a message to update the game
  (never guessed at).
- An older one is brought up to date by the chain in `tko::format::MIGRATIONS` when it is read.
  Raising `tko::TKO_VERSION` needs a migration there and a fixture of the old format in the tests
  (`a_v1_fixture_keeps_reading` pins version 1, `a_v2_fixture_keeps_reading` version 2). Versions 2
  and 3 only add optional keys (3 adds `clips`, text and pictures among them), so their migrations change nothing: the bumps are
  there so an older game refuses a map that uses them instead of dropping them silently.
- A `.tko` file without `TkoVersion` is an error.
- The editor writes the current version, and never rewrites a file it cannot read.

## Gimmick: visual only

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
- Playfield properties, in three groups. *Move*: `x`, `y` (virtual units), `rotation` (degrees,
  about the hit target), `zoom`, `scroll` (a multiple of the map's own scroll speed; what is
  drawn, not when a note is hit), `stretch_x`, `stretch_y`, `flip` (0..1: turned over left to right
  like a page, 1 mirrored with the drum on the right; it multiplies the Flip Flop mod's turn). *Look*: `flash` (0..1 white wash),
  `vignette` (0..1 dark corners), `wash` (0..1 colour overlay) with `wash_hue` (degrees), `glitch`
  (0..1: the notes and the drum torn into shifted slices; the lane stays whole so no note is covered), `chroma` (px of colour split). *Clones*: `echo`
  (0..8 fading copies of the playfield) with `echo_x`, `echo_y`, `echo_rot`, `echo_zoom` and
  `echo_fade` (each copy is offset, turned and shrunk once more than the one before), `kaleido`
  (2..8 copies turned about the screen centre).
- The effects are made from the shapes the playfield drew that frame (copied, tinted and moved), so
  they cost nothing at their defaults, copies are capped (8 echoes, 8 kaleidoscope arms, 12 glitch
  slices, 20,000 extra shapes a frame) and there is no true blur or colour grading.
- Group properties: `dx`, `dy`, `scale`, `opacity`, `hidden` (1 hides the note; it can still be
  hit), `swirl` (degrees a note is turned about the drum). A group's notes are found by their start time. Don and kat notes are animated; rolls and
  swells are not yet.
- Limits (a file can't make the player hold an unbounded amount): 10,000 keys per track, 256
  groups, 200,000 grouped notes in all, 4 MiB of JSON. Values are held inside each property's range
  and unknown properties are dropped when a file is read.

## Clips

`clips` in the same JSON are effect blocks on the timeline, the editor's simple way to make
effects (F4 Effects > Clips). Each one says what it moves, from when to when, how much and how
often:

```
"clips": [
  { "id": 1, "lane": 0, "what": "zoom", "start": 104265, "end": 126000, "amount": 0.12,
    "when": {"kind": "beats", "every": 2}, "motion": "kick", "length": 1, "ease": "quad_out" },
  { "id": 2, "lane": 1, "what": "notes.hidden", "start": 150000, "end": 170000, "amount": 1 }
]
```

- `what`: a playfield property (`zoom`, `rotation`, `flash`... as in `field`), `shake` (random
  offsets of the playfield, `amount` virtual units at most), `notes.<property>` (`dx`, `dy`,
  `scale`, `opacity`, `hidden`, `swirl`) for all the notes, or a group's with `"group": k` (1-based),
  or `lane.<shape>` (`zigzag`, `uturn`, `spiral`, `loop`, `elbow`, `meadow`: the Shapeshift mod's
  shapes), which bends the lane `amount` (0 straight, 1 the whole shape) of the way into that shape,
  so a fade in and out morphs it. The strongest lane clip at a time is the one drawn, and while one
  is, it goes before a rail mod the player picked.
- `start`, `end`: chart ms. `lane`: the editor's row (0..7). `id`: keeps the editor's selection.
- `when`: `{"kind": "span"}` (the default: the whole clip, coming in over `fade_in` beats and going
  out over `fade_out`), `beats` (every `every` beats from the start), `notes` (each note reaching the
  drum inside it; `"notes"`: `all`, `dons`, `kats` or `big`), `hits` (each note hit, same filter) or
  `misses`. Hits and misses are told to the runtime as the play goes, like the graph's play events.
- A repeating clip's `motion` at each moment: `kick` (the amount, settling back over `length`
  beats), `swing` (to the amount, then to minus it, `length` beats each way) or `step` (the amount
  more each time). `ease` shapes the fades, the kick, the swing and the step.
- `keep`: what it reached stays after its end (otherwise it lets go).
- What clips give is added to the keyframes and the graph's actions, and the sum is held inside
  the property's range (an opacity clip of `-0.7` leaves the notes at 0.3).
- Limits: 4,096 clips; lengths and fades from 1/64 to 256 beats; an amount as far as the property
  goes (64 times that for steps); the graph's cap of moments for each clip. A clip with an unknown
  `what`, no length, or a group that isn't there is dropped when the file is read.

### Text and pictures

A clip with `"what": "layer"` puts text or a picture on screen for its length instead of moving
anything:

```
{ "id": 3, "lane": 2, "what": "layer", "start": 104265, "end": 112000, "fade_in": 1,
  "fade_out": 1, "ease": "cubic_out",
  "layer": { "show": {"kind": "text", "text": "Here we go"},
             "from": {"x": -300, "y": 110, "size": 36, "turn": 0, "alpha": 1},
             "to":   {"x": 0, "y": 110, "size": 36, "turn": 0, "alpha": 1},
             "colour": 16777215 } }
```

- `show`: `{"kind": "text", "text": ...}` in the game's own font (at most 120 characters, no
  control characters), or `{"kind": "picture", "file": ...}`, an image in the map's folder (a
  plain relative name: one that climbs out of the folder drops the clip). Pictures are loaded as
  the map plays, squeezed into 512 by 512 and drawn back at their own shape; up to 15 at once.
- `from`: its pose at the start, in units of the 480 tall screen: `x` from the middle (right is
  positive), `y` from the top, `size` (its height), `turn` (degrees, clockwise), `alpha` (0..1).
  `to`, when there, is the pose at the end, reached by `ease`; without it the layer stays as it
  starts.
- `fade_in`, `fade_out` (beats) fade it in and out. Repeating (`when` other than `span`), it pulses:
  `amount` is how much bigger it gets, in percent, with the `motion` and `length` of any clip.
- `colour`: RGB tint (white by default). `behind`: drawn under the lane and notes instead of over.
- `keep`: it stays on screen after its end, at its end pose.
- Text and pictures are drawn in single player and in the editor; multiplayer and spectating show
  the text but not the pictures yet.

## Marks

`marks` in the same JSON names moments of the map (a section start, a drop, a break):
`[{"name": "drop", "time": 104265}]`, ascending by time. Nodes refer to a mark by its 1-based
place in the list instead of a typed time. The editor places them with `M` (and `Shift+M` takes
the nearest one away) and shows them as flags on the timeline.

## Node graph

`graph` in the same JSON adds events, logic and actions on top of the keyframes. Like them it only
moves what is drawn.

```
"graph": {
  "seed": 0,
  "nodes": [
    { "id": 1, "kind": "on_beat", "x": 20, "y": 20, "p": [1, 0] },
    { "id": 2, "kind": "pulse", "x": 260, "y": 20, "p": [0.2, 250], "ease": "quad_out",
      "target": "field.zoom", "inputs": [1] }
  ]
}
```

- `p` are the node's numbers, in the order its kind lists them (missing ones take their defaults,
  out-of-range ones are held in range). `inputs` lists, per input port, the id of the node feeding
  it, or `null`. `x`/`y` only place the box in the editor.
- Two kinds of wire. A *trigger* carries moments in time; a *number* carries a value that can be
  read at any time. A wire that joins the wrong kinds, comes from a missing node, or would close a
  loop is dropped when a file is read.
- Events (trigger sources): `on_start`, `on_time` (typed ms; prefer a mark), `on_beat` (every N
  beats from an offset, on the map's beat grid; a green line doesn't restart it), `on_kiai_start`,
  `on_kiai_end`, `on_mark`, `on_note_reach` (a group's notes, or every note: filter by `type`,
  0 any, 1 don, 2 kat, 3 finisher, 4 roll or swell, and by `nth` to take only the Nth of them),
  and the play events `on_hit` (group, judge, type), `on_miss` (group, type), `on_combo` (every
  N). Play events are told to the graph by the game as notes are
  judged, so a replay draws the same.
- Triggers: `delay`, `counter` (every Nth), `once`, `gate` (passes moments while a number is at
  least 0.5), `merge`.
- Values: `const`, `time`, `beat_phase`, `beats` (beats so far), `kiai` (1 inside kiai), `combo`,
  `accuracy`, `random` (steady within a beat, from the graph `seed`), `noise` (smooth, seeded),
  `wave` (sine, triangle, saw or square over N beats), `toggle` (flips at each moment), `envelope`
  (1 falling to 0 after each moment), `count` (moments so far), `progress` (how far through the
  trigger's moments we are, 0..1: "the 3rd of 4 finishers" is 0.75), `between` and `span`
  (1 between two marks / 0..1 across them). Math: `add`, `sub`, `mul`, `compare`, `not`, `abs`,
  `min`, `max`, `mix` (blend by a third number), `pow`, `steps`, `clamp`, `map` (times, plus). An
  unwired number input reads 0 (1 for `mul`).
- Actions end a chain and move a `target` (`field.<property>` or `group.<index>.<property>`): `drive`
  (a number, times a gain), `pulse` (a kick that settles over a time), `tween` (flips between
  nothing and a value at each moment), `shake` (random offset of the playfield, fading), `clip`
  (a short run of keys played from each moment). `pulse`, `tween`, `shake` and `clip` have a
  second, optional `scale` number input, read when each moment happens: its amount is multiplied
  by it (1 if unwired), so `progress` into `scale` makes each kick harder than the last. What
  actions give is added to the property's own
  keyframes and the sum is held inside the property's range.
- Limits: 2,000 nodes, 20,000 moments per node, 10,000 node reads per frame (and a read chain at
  most 512 deep). Randomness only comes from `seed` and the beat, never from the clock.
- Recipes (`tko::recipes`) are ready-made, wired groups of nodes the editor's Nodes page can drop
  in one click: beat pulse, kiai drop, finisher build-up, combo zoom, miss glitch, kiai echo trail
  and section sweep. `examples/tko_showcase.rs` builds a whole demo gimmick from a map's structure
  with the same API.
