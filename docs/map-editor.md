# Map editor

The editor opens from the main menu (EDITOR, right after PLAY) and edits osu!stable-compatible taiko maps (`.osu`), or `.tko`
maps (a `.osu` plus this client's own effects, see [tko-format.md](tko-format.md)). What it saves
as `.osu` plays the same in osu!stable. The first time it opens it runs a short guided tour;
Help > Take the tour shows it again, `?` lists every key, and **Help > Manual** is a full manual
inside the editor (English and Russian): timing, the SV tool, nodes, uploading.

## Making a map from a song

EDITOR in the main menu opens the start screen. Drop an audio file (mp3, ogg, wav, flac) on it, or
on the game window anywhere, or click to browse; recent maps and "pick in song select" are there for
going on with a map you already have.

1. **Details**: artist, title, mapper name, difficulty name, source, tags and a background picture.
   A name with non-English letters wants a romanised version too (osu! rule). The names are guessed
   from the file's name.
2. **Timing**: the game finds the BPM and the moment of the first beat from the whole song and says
   when it thinks the tempo changes. Tap the tempo with `T`, switch the metronome with `M`, loop 8
   bars with `L`, say "the beat is here" with `B`, go back to what was found with `D`. The
   waveform shows the beat grid: the wheel or a drag moves along it, Ctrl+wheel zooms, and the strip
   under it is the whole song (press or drag there to jump, also while it plays).
3. **Create** makes `Songs/<Artist> - <Title>` with the music, the background and a `.osu` (or
   `.tko`), adds it to your list and opens it in the editor.

### Timing a song

Every note snaps to a grid made from the red line, so get it right first. Check by ear: put notes on
four bars of beats, set Speed to 50% and play. If the notes are early or late by the same amount
everywhere, move the offset; if they drift the longer the song goes, the BPM is a little off. A song
with several tempos needs a red line (`Ctrl+P`) at each change: time the later part by ear the same
way. F3 Timing edits the lines, F6 lists all of them.

## The song folder

File > Open song folder shows the map's folder in the file manager. While the editor is open the
game watches that folder: a `.osu` or `.tko` dropped into it (another difficulty, a map from a
friend) joins the song list within a second, and one you change or delete there is updated or
removed, with no restart. A file that is still being copied in is picked up once it stops growing.

## Layout

- Menu bar: File, Edit, View, Timing, Help. Every command is in a menu with its key beside it.
- Mode bar: F1 Compose, F2 Gimmicks, F3 Timing, F4 Effects (`.tko`), plus play, snap, speed, undo, save.
- Timeline: notes, timing lines, bookmarks, and the music's waveform under the notes (View >
  Waveform on / off). Its colour is the kind of sound: red is bass (often a don), blue is treble
  (often a kat). The song overview below it is clickable.
- Right panel: changes with the mode. Hover any control for a tip.

## Keys (as in osu!stable)

| Keys | Action |
|---|---|
| Space / C, X | play or pause, play from the start |
| Z / V | first / last note |
| Left / Right (Shift x4), Up / Down | step by snap, previous / next timing line |
| Wheel (Shift x4), Ctrl+wheel, Alt+wheel | seek, beat snap, zoom |
| Shift+1..9 | beat snap |
| Ctrl+Up / Down (with Shift: 5%) | playback speed |
| 1 2 3 4 | Select, Note, Drumroll, Denden tools |
| W / E | kat (R too, as in osu!), big: the note in hand with the Note tool (click a placed note to paint it that colour), else the selection |
| J / K, Ctrl+D, Ctrl+G, Q | nudge, clone, reverse, flip don/kat |
| Ctrl+Z / Y / X / C / V / A, Delete | undo, redo, cut, copy, paste, select all, delete |
| Ctrl+B / Ctrl+Shift+B, Ctrl+Left / Right | add / remove bookmark, previous / next bookmark |
| Ctrl+P / Ctrl+Shift+P, Ctrl+I | red line, green line, delete line at the playhead |
| M / Shift+M | effect mark at the playhead / remove the nearest (`.tko`) |
| F6 | timing window (every line, its fields, the SV tool) |
| Ctrl+S, Ctrl+L, Esc | save, revert (press twice), leave (asks if unsaved) |

Mouse: click places with the Note tool, right click deletes, drag moves (Shift ignores the grid),
double click puts the playhead on a note, middle drag pans the timeline. Dragging a box or a note
past the edge of the timeline scrolls it. A press on empty lane (between notes too) starts a box;
Ctrl+drag boxes with any tool.

Bookmarks are saved in the `.osu` (`[Editor]` `Bookmarks:`), like stable.

## Timing window (F6)

F6 opens a window over the preview and the panels with every timing line: time, type, BPM or SV,
meter, sample set and index, volume, kiai and the omitted bar line. The timeline stays under it
and keeps working, so a stretch of notes can be selected while it is open; the lines inside that
stretch are tinted. Click a row to select it and jump there, Shift takes a range, Ctrl toggles;
Up/Down/PgUp/PgDn/Home/End walk the rows, Delete removes them. The filter chips show red or green
lines only. On the right the *Line* tab edits every field of the selected lines (volume, sample
set and index, kiai and omit apply to all of them; time, BPM, meter and SV to the main one) and the
*SV tool* tab holds the curve settings. Esc or Close shuts it.

## Slider velocity

Timing mode (F3) edits green lines in the graph: double click to add, drag a dot to change it, drag a
box to select several and move or scale them together (Alt adds instead of scaling), right click
to delete. With several selected there are set, multiply, copy, paste and delete tools.

The graph also draws the curves the green lines already follow (linear, geometric, poly, sine) as
a thin line with a chip such as `sin x1.20 > x2.00`. Click the curve, a dot on it or its chip
(Shift+click takes a single line) to pick it: the SV tool opens as that curve's editor (shape,
start and end speed), and the round handles at its ends can be dragged. The whole run is
rewritten along the same curve in one undo step; Done lets go of it. The *Brush* toggle in the graph's corner draws a curve: drag from one point to another and the
lines are written between them, on the notes, on a snap grid or on existing green lines. The *SV |
Volume* switch turns every gesture into a volume edit. The *SV tool* panel (Timing page or F6)
sets the shape, the start and end values, what is written (SV, volume or both) and where (notes,
beat grid, existing lines), with those choices in dropdowns, and under *Advanced* the
offset, buffer, minimum spacing and "ignore BPM" (SV is compared as scroll speed, SV x BPM, unless
that is on). With two or more green lines selected Apply fills each consecutive pair. Nothing is
saved or reloaded: the chart and the preview update at once and undo takes it back.
The tool is a port of svt-linux / svt-rs.

**Presets** (SV tool, "between selected lines"): ramp, geometric ramp (reads as an even
speed-up), multiply, a line on every note, ease in, ease out, S-curve, sine in and out, and a
curved line on every note. A preset sets the shape and where it is written; Apply writes it.

Recipes:

- *Slow into a drop*: green lines at the start and end of the slow stretch, preset Ease out, from
  x1.00 to x0.60.
- *Build-up*: two lines across two bars, preset Geometric ramp, x0.80 to x2.00.
- *Volume fade*: switch to Volume, ramp 100 to 30 across the last bars of a section.

## Effects (`.tko`)

F4 opens the Effects page on **Clips**: effects as blocks on rows under the timeline, each acting
from its start to its end and keeping time with the music. Click one of the ready-made effects on
the shelf (Beat pulse, Every 2nd beat, Sway, Tilt, Spin, Quarter turns, Spiral lane, Zigzag lane, Loop lane,
Turn over, Earthquake, Big note slam,
Kat flash, Drop, Build-up, Hide the notes, Faint notes, Bouncing notes, Swirl in, Speed up, Echo
trail, Kaleidoscope, Colour wash, Miss glitch) and it lands at the playhead: over the whole kiai
the playhead is in, else over the notes selected, else over the next four bars. A drag on an empty
row draws the effect used last. Drag a block to move it (to another row too), its ends to resize
it; right click deletes it, Delete and Ctrl+D act on the one picked. Clicking a block shows its
settings instead of the shelf: what it moves (`<` `>`: any playfield property, a shake, the lane's
shape (zigzag, U-turn, spiral, loop, elbow, meadow), or the notes: move, size, see-through, hidden,
swirl), when it acts (the whole time, every few beats, on notes
(all, dons, kats, big ones), on hits, on misses), the amount (click to type), the fades or the
motion of each repeat (kick, swing, step) and how many beats it takes, the easing, *keep after*, and
*Fit to kiai* / *Fit to selection*. Blocks add up with each other and with keyframes and nodes.

The preview can be moved by hand on this page: drag it to move the playfield, Shift+drag to turn
it around the hit target, Ctrl+drag to zoom it (up grows it, down shrinks it). Each drag sets keys
at the playhead (snapped to the grid) and undoes in one step. The keys show as diamonds on the
*Motion* strip above the rows: click one to go there, right click it to take that moment's keys
away. They are the playfield's keyframes, so the Keyframes page edits them too.
The format is in [tko-format.md](tko-format.md#clips).

*Advanced...* shows the two older pages. *Keyframes* animates the playfield (move, look and clone properties,
in tabs) and groups of notes over time, and *Nodes* wires events to actions on a canvas. Wheel up zooms in (far out for big projects, Fit frames everything, 100% resets). Click a
node's number to type it, pick its target from the grouped list, and use "Group from selection" on
a Note arrives node to follow the notes selected in Compose. The Recipe menu drops a ready-made
node group (17 of them: beat pulse, kiai drop, finisher build-up, combo zoom, miss glitch, echo
trail, section sweep, kiai colour wash, drumroll shake, breathing vignette, build-up between marks,
half turn every 4 bars, combo milestone flash, accuracy tint, hit jitter, kiai spin, big note
slam). Nodes that can't do anything yet wear a red `!` (hover it for the reason), and an empty
canvas shows the first steps of a first effect. See the [nodes guide](nodes-guide.md) for how
nodes work, worked examples and every node. `M` drops a named mark on the timeline for "At mark", "Between marks" and "Span progress".
Everything stays visual: judging, score and pp never change.

## Exporting a file

File > Export map file saves the map and packs its whole set (every taiko difficulty with notes,
the music, pictures, storyboard and hitsounds) into one file in taiko.rs's `Exports` folder, then
shows it in the file manager. A set with a `.tko` in it becomes a **`.rsz`**: taiko.rs's own map
archive, the same zip as an `.osz`, which only taiko.rs plays. A set of plain `.osu` maps becomes
an `.osz`, which osu! opens too. Send it to anyone: opening a `.rsz` (double click, or "open with")
or dropping it on the game imports it, as an `.osz` does. A `.rsz` dropped into the Songs folder is
picked up as well.

The game makes itself the program for `.rsz` files when it starts (Windows: the user's registry;
Linux: a MIME type and a desktop entry), and the Windows installer does too. Opening one while the
game runs hands it to that game. macOS doesn't open `.rsz` files with a double click yet: drop them
on the window.

## Uploading to taiko.rs

File > Upload sends the saved map to the server. A checklist comes first: logged in, saved, a taiko
map with at least 10 notes, artist / title / mapper filled in, the music in the folder, and your own
map (no osu! `BeatmapSetID`). The map's folder is packed (every taiko difficulty with enough notes,
the music, pictures, storyboard) and sent.

- A set is public at once as **CUSTOM**: it is on the website's CUSTOM tab with a download, and gives
  no pp.
- An admin or a loved nominator can approve it: **APPROVED CUSTOM** gives pp, like a loved map.
- The set's number is kept in the folder (`taiko-rs.custom`), so sending again updates the same set.
  What was approved keeps counting; changed difficulties wait for another approval.
- In the game custom maps have their own headings in song select, and `status:custom` finds them.
- Limits: 30 sets per account, 40 MB each, one send per 30 seconds.
