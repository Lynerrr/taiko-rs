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
| F6 | timing lines list |
| Ctrl+S, Ctrl+L, Esc | save, revert (press twice), leave (asks if unsaved) |

Mouse: click places with the Note tool, right click deletes, drag moves (Shift ignores the grid),
double click puts the playhead on a note, middle drag pans the timeline.

Bookmarks are saved in the `.osu` (`[Editor]` `Bookmarks:`), like stable.

## Slider velocity

Timing mode (F3) edits green lines in the graph: click to add, drag a dot to change it, drag a
box to select several and move or scale them together, right click to delete. With several
selected there are set, multiply, linear and geometric ramp, copy, paste and delete tools.

## Effects (`.tko`)

F4 opens the Effects page: *Keyframes* animates the playfield (move, look and clone properties,
in tabs) and groups of notes over time, and *Nodes* wires events to actions on a canvas. Click a
node's number to type it, pick its target from the grouped list, and use "Group from selection" on
a Note arrives node to follow the notes selected in Compose. The Recipe menu drops a ready-made
node group (beat pulse, kiai drop, finisher build-up, combo zoom, miss glitch, echo trail, section
sweep). `M` drops a named mark on the timeline for "At mark", "Between marks" and "Span progress".
Everything stays visual: judging, score and pp never change.
