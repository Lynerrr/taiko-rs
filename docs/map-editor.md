# Map editor

The editor opens from the main menu and edits osu!stable-compatible taiko maps (`.osu`), or `.tko`
maps (a `.osu` plus this client's own effects, see [tko-format.md](tko-format.md)). What it saves
as `.osu` plays the same in osu!stable. The first time it opens it runs a short guided tour;
Help > Take the tour shows it again, and `?` lists every key.

## Layout

- Menu bar: File, Edit, View, Timing, Help. Every command is in a menu with its key beside it.
- Mode bar: F1 Compose, F2 Gimmicks, F3 Timing, F4 Effects (`.tko`), plus play, snap, speed, undo, save.
- Timeline: notes, timing lines, bookmarks. The song overview below it is clickable.
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
| W / R / E | kat (whistle / clap), big (finish) on the selection or the next notes |
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

Timing mode (F3) edits green lines in the graph: click to add, drag a dot to change it, drag a
box to select several and move or scale them together (Alt adds instead of scaling), right click
to delete. With several selected there are set, multiply, copy, paste and delete tools.

The graph also draws the curves the green lines already follow (linear, geometric, poly, sine) as
a thin line with a chip such as `sin x1.20 > x2.00`. Click a chip to select its lines and drag
the diamond handles at its ends: the whole run is rewritten along the same curve in one undo
step. The *Brush* toggle in the graph's corner draws a curve: drag from one point to another and the
lines are written between them, on the notes, on a snap grid or on existing green lines. The *SV |
Volume* switch turns every gesture into a volume edit. The *SV tool* panel (Timing page or F6)
sets the curve, the start and end values, what is written (SV, volume or both), where, the
offset, buffer, minimum spacing and "ignore BPM" (SV is compared as scroll speed, SV x BPM, unless
that is on). With two or more green lines selected Apply fills each consecutive pair. Nothing is
saved or reloaded: the chart and the preview update at once and undo takes it back.
The tool is a port of svt-linux / svt-rs.

## Effects (`.tko`)

F4 opens the Effects page: *Keyframes* animates the playfield (move, look and clone properties,
in tabs) and groups of notes over time, and *Nodes* wires events to actions on a canvas. Wheel up zooms in (far out for big projects, Fit frames everything, 100% resets). Click a
node's number to type it, pick its target from the grouped list, and use "Group from selection" on
a Note arrives node to follow the notes selected in Compose. The Recipe menu drops a ready-made
node group (beat pulse, kiai drop, finisher build-up, combo zoom, miss glitch, echo trail, section
sweep). `M` drops a named mark on the timeline for "At mark", "Between marks" and "Span progress".
Everything stays visual: judging, score and pp never change.
