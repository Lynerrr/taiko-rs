//! The editor manual's words, in English and Russian (Japanese falls back to English). A body is
//! lines: `# ` a heading, `- ` a bullet, `> ` a tip, an empty line a gap, anything else a
//! paragraph. `{recipes}` stands for the list of node recipes, written from `tko::recipes`.

use crate::lang::{self, Lang};

pub struct Chapter {
    pub title: &'static str,
    pub body: &'static str,
    /// A button at the end that goes and does the thing: (label, what).
    pub try_it: Option<(&'static str, TryIt)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TryIt {
    TimingWindow,
    Nodes,
    Tour,
    /// A set of clips laid over the map's first kiai, to see and pick apart.
    Showcase,
    /// Paths, a move, fakes, a freeze, the drum sliding and a lie laid over the map's first kiai.
    MotionShowcase,
}

fn ru() -> bool {
    lang::current() == Lang::Ru
}

/// The chapters in the current language.
pub fn chapters() -> &'static [Chapter] {
    if ru() { &RU } else { &EN }
}

pub const COUNT: usize = 12;

static EN: [Chapter; COUNT] = [
    Chapter {
        title: "Start here",
        try_it: Some(("Take the tour", TryIt::Tour)),
        body: "\
The editor makes taiko maps. What you save as .osu plays the same in osu!stable; a map saved as .tko adds this client's own animations on top and never changes the .osu it came from.

# Making a map from a song
- Main menu > EDITOR, or drop an mp3 / ogg / wav / flac on the game window.
- Details page: artist, title, mapper name, difficulty name, background. Non-English names want a romanised version too (osu! rules).
- Timing page: the game finds the BPM and the first beat for you. Check it by ear (Timing a song).
- Create makes the folder in your Songs and opens it here.

# Song setup
File > Song setup changes the names and settings any time later: artist and title (each with its romanised version), mapper, difficulty name, source and tags, and HP drain, accuracy (OD) and base SV.
- Click a box and type; Enter or Tab keeps it, Tab goes on to the next box. Numbers also step with - and + (Shift for bigger steps, Alt for fine ones).
- The window shows the hit windows of the OD and the star rating as you change them.
- Base SV is how fast notes scroll at 1.0x SV. Changing it keeps every drumroll as long as it was.
- Each change is one undo step; Ctrl+S saves them with the map.

# The screen
- Modes: F1 Compose (notes), F2 Gimmicks (stable tricks), F3 Timing (BPM and slider velocity), F4 Effects (effect clips, keyframes and nodes, .tko only).
- The timeline runs left to right; the white line is the playhead. Alt+wheel zooms, the middle button pans.
- The preview is the real game drawn at the playhead.
- Hover anything for a tip. Every command is in the menus with its key beside it.

> Save often: Ctrl+S. The first save keeps a .bak of the original. Ctrl+Z undoes, Ctrl+L reverts to the last save (press twice).

File > Open song folder shows the map's folder. Drop another difficulty (.osu or .tko) into it and it joins your song list within a second, no restart.
",
    },
    Chapter {
        title: "Timing a song",
        try_it: Some(("Open the timing window (F6)", TryIt::TimingWindow)),
        body: "\
A map is only as good as its timing: every note snaps to a grid made from the red line (BPM and the moment of the first beat). Get this right first.

# In the new-map wizard
- Detect: the game listens to the whole song and offers a BPM and an offset. If the tempo changes in the song it says so.
- Tap: press T on the beat for a few bars; the BPM follows your taps.
- Metronome (M) clicks on every beat and louder on the bar. Loop 8 bars (L) repeats a stretch so you can listen.
- Beat at the playhead (B): play to a clear drum hit, stop on it and press B to say that is a beat.
- Meter: 4/4 or 3/4 sets where the bar starts.

# Check it by ear
- Put notes on 4 bars of beats, set Speed to 50% and Play. If the notes are early or late by the same amount everywhere, move the offset. If they drift further the longer the song goes, the BPM is a little off.
- The BPM and the offset have small +/- buttons for fine steps (down to 0.01 BPM and 1 ms).
- Compare the grid lines with the waveform spikes: a spike should sit on a line.
- The waveform's colour is the kind of sound: red is bass (a kick, often a don), blue is treble (a snare or a hat, often a kat). It is under the notes in the editor too (View > Waveform on / off).

# Inside the editor
- F3 Timing: select the red line to edit its BPM, offset and meter. F6 lists every line.
- Ctrl+P adds a red line at the playhead (a BPM change; it resets the grid). Ctrl+I deletes the line at the playhead.
- The Kiai switch on a line marks kiai time. The overview bar shows it in orange.

> A song with several tempos needs a red line at each change. Time the later part by ear, as above, then add its red line.
",
    },
    Chapter {
        title: "Notes and sounds",
        try_it: None,
        body: "\
# Placing
- Tools: 1 Select, 2 Note, 3 Drumroll, 4 Denden. With Note a click on the lane places on the grid; right click deletes (on a selected note: the whole selection); drag moves (Shift ignores the grid).
- Snap sets how finely a beat divides: Ctrl+wheel or Shift+1 to 9. Left / Right step by one snap.
- Speed (Ctrl+Up / Down) slows the music so you can place by ear.

# Colour and size
- W makes kats (press it again for dons), E makes big notes. They apply to the selection, or to the notes you place next.
- Q flips don and kat. J / K nudge, Ctrl+D clones, Ctrl+G reverses.

# Selecting and copying
- Drag a box on empty lane (Ctrl+drag with any tool). Ctrl+A selects all.
- Ctrl+X / C / V cut, copy, paste at the playhead.

# Bookmarks and the song
- Ctrl+B adds a bookmark, Ctrl+Left / Right jump between them. They are saved in the .osu.
- Z / V jump to the first / last note. The bar under the graph is the whole song: click to jump.
",
    },
    Chapter {
        title: "SV and the SV tool",
        try_it: Some(("Open the timing window (F6)", TryIt::TimingWindow)),
        body: "\
# What SV is
A green line sets the slider velocity: how fast notes scroll and how long sliders are. x1.00 is normal, x2.00 is twice as fast. Notes keep their time; only the spacing on screen changes. Mappers use it to make a drop feel heavy, a build-up feel like it speeds up, a breakdown feel slow.

# The graph (F3)
- Double click empty space to add a green line, drag a dot to change it, right click to delete.
- Drag a box to select several: move them together, or scale them (Alt adds instead of scaling).
- A curve the lines already follow is drawn thin with a chip like sin x1.20 > x2.00. Click it to edit it as a curve: the round handles at its ends can be dragged.
- The SV | Volume switch turns every gesture into a volume edit. Brush draws a curve by dragging from one point to another; an end dropped right by a line clings to it, so strokes join the lines already there (Alt keeps it free).

# The SV tool
It writes many green lines for you along a curve. Open it in the Timing panel or in F6 > SV tool.
- Select two green lines (click one, Shift+click the other), pick a preset from Presets, set the start and end speed, press Apply. The tool shows a ghost of what it would write first; nothing changes until Apply, and Ctrl+Z takes it back.
- Presets: ramp, geometric ramp (feels like an even speed-up), multiply, a line on every note, ease in, ease out, S-curve, sine curves.
- Written where: on the lines already there, on the beat grid, or on the notes.

# Recipes
- Slow into a drop: green lines at the start and end of the slow stretch, preset Ease out, from x1.00 to x0.60.
- Build-up: two lines across 2 bars, preset Geometric ramp, x0.80 to x2.00.
- Volume fade: switch to Volume, ramp 100 down to 30 across the last bars of a section.
- Every note its own speed: preset Curved line on every note.

> SV is compared as scroll speed (SV x BPM) unless Ignore BPM is on, so a BPM change does not make the curve jump.
",
    },
    Chapter {
        title: "Stable gimmicks",
        try_it: None,
        body: "\
F2 Gimmicks builds the structures mappers use in osu!stable: fake sliders, anti-barline walls, generated SV. Choose a tool, set its numbers in the panel, then apply it at the playhead or to the selection. Preview shows what it would do first; Keep writes it, Cancel throws it away. Each result is one undo step.

These work in plain .osu maps and play the same in stable. Nothing here needs the client's own animations.
",
    },
    Chapter {
        title: "Effect clips",
        try_it: Some(("Put an example on this map", TryIt::Showcase)),
        body: "\
F4 Effects opens on Clips: the simple way to make a .tko map do anything to the playfield. An effect is a block on the rows under the timeline. Its start and end are where it acts, and it keeps time with the music.

# Making one
- The shelf has tabs: Playfield (pulses, turns, zooms, shakes), Lane (its shape), Notes, Look (flashes, colours, echoes, see-through, blackout) and Text & pictures.
- Copies: up to four copies of the playfield behind it, each moved by its own blocks (shows, x, y, turn, size, lane see-through). copy 1-4 in a block's settings picks which. Try Mirror twin, Triple lane, Shadow clones, Spinning ghost. Only the real playfield is played.
- Cut in pieces cuts the picture into 2-12 upright pieces; Pieces up / down pushes every other one the other way, Pieces apart spreads them from the middle. Try Broken lane, Shatter on the beat, Split open.
- Blackout and Fade to black turn the screen black (text over the lane still shows); Ghost playfield makes it faint; Only the notes leaves notes and drum alone on black. Notes that can't be seen are still there to hit.
- Click an effect to pick it, then drag on a row from where it should start to where it should end. A click on a row lays one bar. Over the kiai here and Over the notes selected lay it over those. Shift keeps it picked for more; Esc puts it back.
- Drag a block to move it (also to another row), drag its ends to make it longer or shorter (a short block's ends stick out a little), right click deletes it. Blocks never sit on top of each other: one dropped or moved over another goes to the nearest free row there, and an end stops at the next block.
- Drag over empty rows to pick every block in the box; Shift or Ctrl+click adds or takes one away. Blocks picked move together, Ctrl+D copies them after themselves, Delete removes them.
- As in Ableton: Ctrl+drag (or Ctrl+Shift+drag) a block to drag a copy of it (of all picked, when it is one of them). Ctrl+C or Ctrl+X keeps the blocks picked, Ctrl+V lays them at the playhead and moves the playhead to their end, so pasting again lays them one after another. They paste into another map too. Ctrl+A picks every block.
- There is always a free row at the bottom; when the rows don't fit, the wheel over them scrolls.

# Changing one
- Click a block: its settings replace the shelf. Click what it moves for a menu of everything, in columns: Playfield (zoom, turn, shake...), Look, Echoes & cuts, Lane (spiral, zigzag, loop...), Notes (move, size, hide, swirl, stretch, tint) and Copies. < and > step through them. The amount keeps its size when you switch.
- When it acts: the whole time, every few beats, on notes (all, dons, kats, big ones), on hits, on misses.
- Amount is how much: click it to type a number (Enter or a click elsewhere keeps it), or use - and + (Shift x10, Alt x0.1).
- A block that acts the whole time fades in and out over the beats you set. One that repeats does a kick (jumps and settles), a swing (one side, then the other) or a step (a bit more each time), taking the beats you set.
- Keep after: what it reached stays when it ends (a turn that stops half way round stays there).
- Fit to kiai and Fit to selection stretch it over those.

# Text and pictures
- Text, Fly-in text and Pulsing text on the shelf put words on screen in the game's font. Click the words in the block's settings and type: the first key replaces them, Ctrl+Backspace takes a word.
- Picture and Logo on the beat show an image from the map's folder: drop a .png or .jpg on the editor window (it is copied there and put on the picked picture block) or put it in the folder yourself. < and > go through them; click its name for a list of them all, with a look at each. It goes with the map in an .rsz.
- With one picked, drag the preview to place it: Shift turns it, Ctrl sizes it. x, y, size, turn and opacity (0 invisible, 1 solid) can be typed too; with moves, a different opacity at its end fades it.
- moves: it glides from its start pose to a pose of its own at its end. Edit start pose and edit end pose choose which one the numbers and a drag change; the preview frames both, with a dotted path between them.
- spin keeps it turning by that many degrees each beat (360 is a full turn a beat, minus turns the other way).
- behind the notes puts it under the lane; the fades bring it in and out; pulses kicks it bigger on beats, notes or hits (the amount is percent).

# Live
- While the song plays, keys 1 to 9 record the effects of the shelf's open tab: press a key where it should start and again where it should end. Esc drops one being recorded.

# Moving it by hand
- Drag the preview: the playfield moves. Shift+drag turns it, Ctrl+drag zooms it (up grows, down shrinks).
- Each drag leaves keys at the playhead, so move the playhead, drag, move it again, drag again: the playfield glides between them. Ctrl+Z undoes a whole drag.
- The keys are the diamonds on the Motion strip above the rows. Click one to go there, right click it to take it away.

# Recipes
- Every second beat of the kiai: Every 2nd beat, dropped with the playhead in the kiai.
- A shake on every big note: Big note slam.
- The notes disappear in the last chorus: select its notes, click Hide the notes.
- A quarter turn each bar that stays: Quarter turns.

Blocks add up with each other, with keyframes and with nodes. Keyframes and Nodes are under Advanced... for what clips can't do yet.

> Everything here is visual only: judging, score and pp never change.
",
    },
    Chapter {
        title: "Note paths",
        try_it: Some(("Put an example on this map", TryIt::MotionShowcase)),
        body: "\
F4 Effects, Motion page (.tko maps): notes you pick fly in along a path you draw instead of scrolling along the lane, and land on the drum exactly when they are hit.

# Giving notes a path
- Select notes on the timeline (one is fine), then press Give the picked notes a path. They get a group of their own and come in From above to start with.
- Pick another start under Where they come from: from below, from behind the drum, the corner, an arc, a drop, a zigzag, a spiral, a loop.
- Shape it in the preview: drag a point to move it, double click to add one, right click to take one away. The white point is where notes come in; the drum always ends the path.

# How they move on it
- A note is as far along the path as it would be along the lane: it comes in at the start when it would come in at the lane's end. Slider velocity still works, so slow notes are slow on the path too.
- Pace along it bends that: Stop half way holds them still in the middle, Rush the end makes them dart in, Soft landing slows them at the drum, Back up makes them step back once.

# When
- Always: on the path the whole song.
- Onto the path here and Back to the lane here glide them over one beat from the playhead; use both to fly only through a chorus.
- Show the path draws it faintly for players, so they can read where notes will come from.

# More notes, the same motion
- Copy motion, pick other notes, Paste motion: they get the same path, pace and keyframes.
- Select them picks a group's notes on the timeline again. A note flies with one group: giving it a new path takes it out of its old group.

# Before the hit
- The Before the hit tab gives the picked notes a move that each one does as its own hit nears, wherever it is: fade in late, vanish early, pop in, drop in, rise from below, jump up, spin in, flicker in, shake, squash on the hit.
- Or key it yourself on the graph: pick what to change (fade, size, up/down, sideways, turn, hide, wide, tall, tint), click to add a key, drag it, right click to remove it. Left is 4 beats before the hit, the bright line is the hit itself. Keys sit on 1/8 beats; hold Shift to place them freely.
- It works with a path and with the Keyframes page: moves and turns add up, size and fade multiply.

# Scroll (mania SV)
- The Scroll tab sets one speed for the whole lane over the song, like SV in osu!mania: 1x plays as mapped, 2x twice as fast, 0x freezes every note where it is, -1x runs them back. Your green lines stay as they are; this goes on top, so a 2x green line under a 0.5x scroll plays at 1x.
- Pick a speed and it holds from the playhead until the next key. Slower and Faster nudge it, Glide to the next key eases into the next speed instead of jumping.
- Moves from the playhead: Freeze a beat, Back up a beat, Lurch on each beat (a bar), Jump a beat ahead. Each comes back to the speed it started from.
- The graph under the timeline shows the speed: click adds a key, drag moves it, right click deletes it.
- Slide the drum along the lane moves the hit target to Home, A quarter in, Half way or Far over one beat from the playhead; notes meet it wherever it is. It is the Move the drum effect, so the Keyframes page and clips can move it too.
- However the scroll runs, every note still reaches the drum exactly when it is hit.

# Fake notes
- The Fakes tab turns the picked dons and kats into fakes (and back): they look and move like notes, but are never hit, give no score or combo, make no sound, and vanish at the drum.
- Add fake copies of the picked notes puts fakes half a beat or a 1/4 before or after them, or a beat after, where no note is yet.
- On the timeline fakes are hollow with an F. Select, move, copy and delete them like any note; groups, paths, moves and the scroll take them by their time.

# Show as
- The Show as tab makes the picked notes lie about their look: all as dons, all as kats or swapped, and all small, all big or swapped.
- The truth shows takes them back to what they are, over a 1/4 beat ending at the hit or up to two beats before it. A colour that only shows at the hit can't be read, only learned: Upload warns about it.
- What hits a note, its score and its sound stay its own. Fakes lie with their group too; drumrolls and swells never lie.
- Players can turn every lie off in Settings, Gameplay, Show true notes. With it on, your own preview shows the truth too.

- Put an example on this map (the button below) lays all of this over the map's first kiai, four beats each: a path, a pop in with fakes, a freeze as the drum slides, swapped colours. Each part is its own Ctrl+Z.

> Paths, moves, the scroll, fakes and lies are visual only: notes are hit at the same moments, and score and pp stay the same. Drumrolls and swells stay on the lane. Players can tone down flashes, glitches and camera moves with Settings, Gameplay, Reduce map effects.
",
    },
    Chapter {
        title: "Keyframes",
        try_it: None,
        body: "\
F4 Effects, Keyframes page: animate the playfield (move, look and clone properties in tabs) and groups of notes over time, only in .tko maps.

# How
- Pick the target: the playfield, or a group of notes. Then pick a property (for example Zoom).
- The graph under the timeline shows its keys: click adds one, drag moves it, right click deletes it. Or put the playhead somewhere and use Key here; Value +/- changes it.
- Between two keys the value glides; Ease sets how.
- Groups: select notes in Compose and make a new group, then give it its own motion (shift, scale, opacity, hide, swirl).

Keyframes are fixed in time. If you want something to react to the beat, the combo or a miss, use Nodes (next chapter). The two add up: a node's motion is added to the property's own keyframes.

> Everything here is visual only: judging, score and pp never change.
",
    },
    Chapter {
        title: "Nodes",
        try_it: Some(("Open the Nodes page", TryIt::Nodes)),
        body: "\
Nodes are a small visual program. Instead of animating by hand, you say WHEN something happens and WHAT it does, and wire the two together: Every beat -> Pulse the zoom. They live on F4 Effects > Nodes (.tko maps; a .osu offers to become one).

# Four kinds of node (by header colour)
- Events: moments in time. Start, At time, Every beat, Kiai starts / ends, Note arrives, Hit, Miss, Combo reaches, At mark.
- Triggers: reshape moments. Delay, Every Nth, Only first, Gate, Either.
- Values and math: numbers that change over time. Combo, Accuracy, In kiai, Beat phase, Wave, Random, Fade out, Progress, Span progress, Add, Scale, Blend...
- Actions: the only nodes that change the picture. Pulse, Tween, Shake, Drive, Play clip.

# Two kinds of wire
- Orange: moments. Something happens now.
- Cyan: numbers. A value you can read at any time.
A dot connects only to a dot of the same colour. Inputs are on the left of a node, the output on the right. Drag from a dot to a dot to wire; right click a node to delete it.

# Your first effect
- 1. F4, Nodes page. Click + Every beat on the right: a node appears.
- 2. Click + Pulse (Actions). A second node appears.
- 3. Drag from Every beat's right dot to Pulse's on dot. The wire is orange.
- 4. Click the Pulse node. In the list on the right pick its target, Zoom. Set amount 0.06 and ms 260 (click a number to type it).
- 5. Press Space. The playfield breathes on every beat.

# What the actions do
- Pulse: kicks the target by amount at each moment, then lets it settle back over ms.
- Tween: moves the target to a value and back, one way at each moment.
- Drive: holds the target at the number wired in, times gain, all the time. Use it for things that follow a value (combo, kiai, a wave).
- Shake: jolts the whole playfield in random directions, fading out. It needs no target.
- Play clip: a short ready-made run of keyframes.
- The scale input of an action is optional: a number that multiplies the effect, read at the moment. Left empty it counts as 1.

# Worked examples
- Only every 4th beat: Every beat -> Every Nth (n = 4) -> Pulse.
- Flash only in kiai: Every beat -> Gate; wire In kiai into the Gate's open input; Gate -> Pulse (target Flash).
- Each finisher harder than the last: Note arrives (type 3) -> Progress; wire both the trigger and Progress into a Pulse's on and scale inputs.
- Zoom follows the combo: Combo -> Scale (times 0.002) -> Drive (target Zoom).
- A turn from one point to another: put marks with M. Span progress (from 1 to 2) -> Drive (gain 360, target Rotation).
- Wobble forever: Wave (beats 4, amount 10) -> Drive (target X).

# Nothing happens?
- A red ! on a node means it can't do anything yet. Hover it: it tells you what is missing.
- An action with no target says no target in red. Pick one on the right.
- The event must be wired into the action's first input (on / flip / value).
- Events fire as the song plays. After jumping the playhead, play from a little earlier.
- Kiai nodes need kiai switched on a timing line. At mark, Between and Span need marks (M); they count from 1 in time order.
- A very small amount is hard to see. Try bigger numbers: Zoom 0.1 to 0.3, Shake 10 or more.
- The recipes (next chapter) are working examples you can open and read.
",
    },
    Chapter {
        title: "Node recipes",
        try_it: Some(("Open the Nodes page", TryIt::Nodes)),
        body: "\
The Recipe menu on the Nodes page drops a ready-made group of wired nodes. Change its numbers, retarget it, or take it apart to see how it works.

{recipes}

> Several recipes can be on one map. Each is one undo step.
",
    },
    Chapter {
        title: "Upload and share",
        try_it: None,
        body: "\
File > Upload sends the saved map to taiko.rs.

# What happens
- A checklist first: you are logged in, the map is saved, a taiko map with at least 10 notes, artist / title / mapper are filled in, the music is in the folder, and it is your own map (no osu! BeatmapSetID).
- Send packs the map's folder (every taiko difficulty with enough notes, the music, pictures, storyboard) and uploads it.
- The set is public at once as CUSTOM: it shows in the website's CUSTOM tab and anyone can download it. Plays on it give no pp.
- An admin or a loved nominator can approve it. APPROVED CUSTOM gives pp.

# Updating
The set's number is kept in the map's folder, so sending again updates the same set (the button says Update the set). What was approved keeps counting; changed difficulties wait for another approval.

# In the game
Custom maps have their own headings in song select (taiko.rs custom, Custom, approved), and status:custom finds them. Maps downloaded from the website come with the mark too.

> Sets are limited: 30 per account, 40 MB each, one send per 30 seconds.
",
    },
    Chapter {
        title: "Keys",
        try_it: None,
        body: "\
# Playing and moving
- Space or C: play or pause. X: play from the start (as in stable).
- F5: test play. The map as it is now (saved or not) plays in the real game from the playhead, Shift+F5 from the start, with No Fail and nothing saved; Quit or the end of the map comes back here.
- Tab: the same test, played by autoplay (Shift+Tab: from the start), to watch the map.
- Z / V: first / last note. Left / Right: by snap. Up / Down: previous / next timing line.
- Wheel: seek. Ctrl+wheel: snap. Alt+wheel: zoom over the timeline, the volume elsewhere. Middle drag: pan (View > Left drag pans lets the left button pan empty timeline too; then Ctrl+drag selects a box).
- Ctrl+Up / Down: playback speed (with Shift 5%).

# Notes
- 1 2 3 4: tools. W E: kat, big. Q: flip. J K: nudge. Ctrl+D: clone. Ctrl+G: reverse.
- Ctrl+Z / Y: undo, redo. Ctrl+X / C / V: cut, copy, paste. Ctrl+A: select all. Delete.

# Timing and the rest
- Ctrl+P: red line. Ctrl+Shift+P: green line. Ctrl+I: delete line. F6: the timing window.
- Ctrl+B: bookmark. Ctrl+Left / Right: previous / next bookmark.
- M: effect mark at the playhead (.tko). Shift+M removes the nearest.
- F1 F2 F3 F4: modes. Ctrl+S: save. Esc: leave (asks if unsaved).
- ? lists every key.
",
    },
];

static RU: [Chapter; COUNT] = [
    Chapter {
        title: "С чего начать",
        try_it: Some(("Пройти обучение", TryIt::Tour)),
        body: "\
Редактор делает тайко-карты. Что сохранено как .osu, играется так же в osu!stable; карта в .tko добавляет поверх анимации этого клиента и никогда не меняет исходный .osu.

# Карта из песни
- Главное меню > EDITOR, или бросьте mp3 / ogg / wav / flac на окно игры.
- Страница Details: исполнитель, название, имя маппера, название сложности, фон. Для неанглийских названий нужна ещё латинская версия (правила osu!).
- Страница Timing: игра сама находит BPM и первый бит. Проверьте на слух (глава «Тайминг песни»).
- Create создаёт папку в Songs и открывает карту здесь.

# Song setup
File > Song setup меняет названия и настройки в любой момент: исполнителя и название (каждое с латинской версией), маппера, название сложности, источник и теги, а также HP drain, точность (OD) и базовую скорость (base SV).
- Кликните по полю и печатайте; Enter или Tab сохраняют, Tab переходит к следующему полю. Числа меняются и кнопками - и + (Shift — шаг больше, Alt — мелкий шаг).
- В окне видны окна попадания для OD и звёзды, они пересчитываются сразу.
- Base SV — скорость прокрутки нот при 1.0x SV. При её изменении длина каждого драмролла остаётся прежней.
- Каждое изменение — один шаг отмены; Ctrl+S сохраняет их вместе с картой.

# Экран
- Режимы: F1 Compose (ноты), F2 Gimmicks (трюки для stable), F3 Timing (BPM и скорость слайдеров), F4 Effects (эффекты-блоки, ключевые кадры и ноды, только .tko).
- Таймлайн идёт слева направо; белая линия — курсор воспроизведения. Alt+колесо — масштаб, средняя кнопка — сдвиг.
- Превью — настоящая игра, нарисованная в точке курсора.
- Наведите на что угодно — появится подсказка. Все команды есть в меню с клавишами рядом.

> Сохраняйтесь чаще: Ctrl+S. Первое сохранение оставляет .bak оригинала. Ctrl+Z отменяет, Ctrl+L возвращает к последнему сохранению (нажать дважды).

File > Open song folder открывает папку карты. Бросьте в неё другую сложность (.osu или .tko), и она появится в списке песен за секунду, без перезапуска.
",
    },
    Chapter {
        title: "Тайминг песни",
        try_it: Some(("Открыть окно таймингов (F6)", TryIt::TimingWindow)),
        body: "\
Карта хороша ровно настолько, насколько точен тайминг: каждая нота привязана к сетке от красной линии (BPM и момент первого бита). Сделайте это правильно в первую очередь.

# В мастере новой карты
- Detect: игра слушает всю песню и предлагает BPM и смещение. Если темп в песне меняется, она об этом скажет.
- Tap: жмите T в такт несколько тактов; BPM подстроится под ваши нажатия.
- Метроном (M) щёлкает на каждый бит, на сильную долю громче. Loop 8 bars (L) повторяет отрезок, чтобы послушать.
- Beat at the playhead (B): проиграйте до ясного удара, остановитесь на нём и нажмите B — это бит.
- Meter: 4/4 или 3/4 — где начинается такт.

# Проверка на слух
- Поставьте ноты на 4 такта по битам, поставьте Speed 50% и жмите Play. Если ноты везде раньше или позже на одно и то же — двигайте смещение. Если расходятся всё сильнее к концу — BPM чуть неточный.
- У BPM и смещения есть маленькие кнопки +/- для тонкой подстройки (до 0.01 BPM и 1 мс).
- Сравните линии сетки с пиками волны: пик должен лежать на линии.
- Цвет волны — какой это звук: красный — бас (бочка, чаще дон), синий — верха (малый, хэт, чаще кат). В эдиторе волна видна и под нотами (View > Waveform on / off).

# Внутри редактора
- F3 Timing: выберите красную линию, чтобы править BPM, смещение и размер. F6 показывает все линии.
- Ctrl+P добавляет красную линию в курсоре (смена BPM; сетка начинается заново). Ctrl+I удаляет линию в курсоре.
- Переключатель Kiai на линии отмечает kiai-время. На полосе обзора оно оранжевое.

> Если в песне несколько темпов, нужна красная линия на каждую смену. Таймите вторую часть на слух, как выше, потом добавьте её красную линию.
",
    },
    Chapter {
        title: "Ноты и звуки",
        try_it: None,
        body: "\
# Расстановка
- Инструменты: 1 Select, 2 Note, 3 Drumroll, 4 Denden. С Note клик по дорожке ставит ноту на сетку; правый клик удаляет (по выделенной ноте — всё выделение); перетаскивание двигает (Shift — без сетки).
- Snap — на сколько делится бит: Ctrl+колесо или Shift+1..9. Влево / вправо — на один шаг.
- Speed (Ctrl+вверх / вниз) замедляет музыку, чтобы ставить на слух.

# Цвет и размер
- W — кат (ещё раз — снова дон), E — большая нота. Применяется к выделению или к следующим нотам.
- Q меняет дон на кат. J / K сдвигают, Ctrl+D клонирует, Ctrl+G разворачивает.

# Выделение и копирование
- Рамка по пустой дорожке (Ctrl+рамка любым инструментом). Ctrl+A выделяет всё.
- Ctrl+X / C / V — вырезать, копировать, вставить в курсор.

# Закладки и песня
- Ctrl+B ставит закладку, Ctrl+влево / вправо — переход между ними. Они сохраняются в .osu.
- Z / V — к первой / последней ноте. Полоса под графиком — вся песня: клик переходит.
",
    },
    Chapter {
        title: "SV и SV tool",
        try_it: Some(("Открыть окно таймингов (F6)", TryIt::TimingWindow)),
        body: "\
# Что такое SV
Зелёная линия задаёт скорость слайдеров: как быстро идут ноты и какой длины слайдеры. x1.00 — обычная, x2.00 — вдвое быстрее. Время нот не меняется, меняется только расстояние между ними на экране. Мапперы делают так «тяжёлый» дроп, ощущение разгона на подъёме, медленный брейкдаун.

# График (F3)
- Двойной клик по пустому месту добавляет зелёную линию, перетащите точку, чтобы изменить, правый клик удаляет.
- Рамкой выделите несколько: двигайте вместе или масштабируйте (Alt добавляет вместо масштаба).
- Кривая, по которой уже идут линии, рисуется тонкой с меткой вроде sin x1.20 > x2.00. Кликните по ней, чтобы править как кривую: круглые ручки на концах можно тянуть.
- Переключатель SV | Volume превращает все жесты в правку громкости. Brush рисует кривую перетаскиванием от точки к точке; конец, отпущенный рядом с линией, прилипает к ней, так штрихи соединяют уже поставленные линии (Alt оставляет его свободным).

# SV tool
Он сам пишет много зелёных линий вдоль кривой. Открывается в панели Timing или в F6 > SV tool.
- Выделите две зелёные линии (клик по одной, Shift+клик по другой), выберите пресет в Presets, задайте начальную и конечную скорость, нажмите Apply. Сначала инструмент показывает «призрак» того, что запишет; пока не нажат Apply, ничего не меняется, а Ctrl+Z всё возвращает.
- Пресеты: ramp, геометрический ramp (ощущается как ровный разгон), умножение, линия на каждую ноту, ease in, ease out, S-кривая, синусоиды.
- Куда пишет: на уже существующие линии, на сетку битов или на ноты.

# Рецепты
- Замедление перед дропом: зелёные линии в начале и конце медленного куска, пресет Ease out, от x1.00 до x0.60.
- Разгон: две линии на 2 такта, пресет Geometric ramp, от x0.80 до x2.00.
- Затухание громкости: переключитесь на Volume, плавно с 100 до 30 на последних тактах части.
- Своя скорость у каждой ноты: пресет Curved line on every note.

> SV сравнивается как скорость прокрутки (SV x BPM), пока не включён Ignore BPM, поэтому смена BPM не создаёт скачка кривой.
",
    },
    Chapter {
        title: "Гиммики для stable",
        try_it: None,
        body: "\
F2 Gimmicks собирает конструкции, которые мапперы делают в osu!stable: фейк-слайдеры, анти-барлайн стены, генерируемый SV. Выберите инструмент, задайте числа в панели и примените в курсоре или к выделению. Preview сначала показывает, что получится; Keep записывает, Cancel выбрасывает. Каждый результат — один шаг отмены.

Это работает в обычных .osu картах и играется так же в stable. Ничего из анимаций клиента здесь не нужно.
",
    },
    Chapter {
        title: "Эффекты-блоки",
        try_it: Some(("Положить пример на эту карту", TryIt::Showcase)),
        body: "\
F4 Effects открывается на Clips: самый простой способ заставить .tko карту делать с игровым полем что угодно. Эффект — это блок на дорожках под таймлайном. Где он начинается и кончается, там и действует, и идёт в такт музыке.

# Как сделать
- На полке вкладки: Playfield (пульс, повороты, зум, тряска), Lane (форма лайна), Notes, Look (вспышки, цвета, эхо, прозрачность, затемнение) и Text & pictures.
- Copies: до четырёх копий плейфилда позади него, каждую двигают свои блоки (shows, x, y, turn, size, прозрачность лайна). copy 1-4 в настройках блока выбирает, какую. Попробуйте Mirror twin, Triple lane, Shadow clones, Spinning ghost. Играется только настоящий плейфилд.
- Cut in pieces режет картинку на 2-12 вертикальных кусков; Pieces up / down двигает их через один вверх и вниз, Pieces apart раздвигает от середины. Попробуйте Broken lane, Shatter on the beat, Split open.
- Blackout и Fade to black делают экран чёрным (текст над лайном всё равно виден); Ghost playfield делает поле полупрозрачным; Only the notes оставляет на чёрном только ноты и барабан. Невидимые ноты всё равно нужно бить.
- Нажмите эффект, чтобы взять его, и протяните по дорожке от места, где он начинается, до места, где кончается. Клик по дорожке кладёт один такт. Over the kiai here и Over the notes selected кладут его на кийай или на выделенные ноты. С Shift он остаётся в руке; Esc кладёт обратно.
- Блок тащится мышью (и на другую дорожку), за края — длиннее или короче (у короткого блока края чуть выступают), правый клик удаляет. Блоки не ложатся друг на друга: брошенный или сдвинутый на другой уходит на ближайшую свободную там дорожку, а край останавливается у соседнего блока.
- Протяните рамку по пустым дорожкам, чтобы выбрать все блоки в ней; Shift или Ctrl+клик добавляет или убирает один. Выбранные двигаются вместе, Ctrl+D копирует их следом, Delete удаляет.
- Как в Ableton: Ctrl+перетаскивание (или Ctrl+Shift) тащит копию блока (всех выбранных, если он среди них). Ctrl+C или Ctrl+X запоминает выбранные блоки, Ctrl+V кладёт их от плейхеда и переносит плейхед на их конец, так что следующая вставка ляжет сразу за ними. Вставлять можно и в другую карту. Ctrl+A выбирает все блоки.
- Внизу всегда есть свободная дорожка; когда дорожки не влезают, колесо над ними прокручивает.

# Как изменить
- Нажмите на блок: вместо полки появятся его настройки. Нажмите на то, что он двигает, — откроется меню со всем по колонкам: Playfield (зум, поворот, тряска...), Look, Echoes & cuts, Lane (спираль, зигзаг, петля...), Notes (сдвиг, размер, скрыть, закрутить, растянуть, оттенок) и Copies. < и > листают по одному. Amount при смене сохраняет свою величину.
- Когда он действует: всё время, каждые несколько долей, на нотах (все, доны, каты, большие), на попаданиях, на промахах.
- Amount — сколько: нажмите, чтобы ввести число (Enter или клик в другое место его сохраняют), или - и + (Shift x10, Alt x0.1).
- Блок «всё время» плавно входит и выходит за заданные доли. Повторяющийся делает толчок (прыгает и успокаивается), качели (в одну сторону, потом в другую) или шаг (каждый раз чуть больше) за заданные доли.
- Keep after: то, к чему он пришёл, остаётся после конца (поворот, остановившийся на полпути, так и остаётся).
- Fit to kiai и Fit to selection растягивают его на кийай или на выделение.

# Текст и картинки
- Text, Fly-in text и Pulsing text на полке выводят слова шрифтом игры. Кликните по словам в настройках блока и печатайте: первая клавиша заменяет их целиком, Ctrl+Backspace стирает слово.
- Picture и Logo on the beat показывают картинку из папки карты: бросьте .png или .jpg на окно редактора (она скопируется туда и встанет в выбранный блок с картинкой) или положите её в папку сами. < и > листают их; клик по имени открывает список всех, с превью каждой. В .rsz картинка уезжает вместе с картой.
- Когда блок выбран, тащите превью, чтобы поставить его: Shift поворачивает, Ctrl меняет размер. x, y, size, turn и opacity (0 невидимо, 1 полностью) можно и набрать; с moves другая opacity в конце плавно его проявляет или гасит.
- moves: блок плавно едет от начальной позы к своей позе в конце. Edit start pose и edit end pose выбирают, какую из двух меняют числа и перетаскивание; превью обводит обе рамками с пунктиром между ними.
- spin крутит его на столько градусов за долю (360 — полный оборот за долю, минус — в другую сторону).
- behind the notes кладёт его под линию нот; затухания вводят и убирают его; pulses толкает его крупнее на долях, нотах или попаданиях (amount в процентах).

# Вживую
- Пока играет песня, клавиши 1-9 записывают эффекты открытой вкладки полки: нажмите клавишу там, где он начинается, и ещё раз там, где кончается. Esc отменяет запись.

# Двигать руками
- Тащите превью: поле едет. Shift+тащить поворачивает его, Ctrl+тащить меняет масштаб (вверх больше, вниз меньше).
- Каждое перетаскивание ставит ключи на плейхеде: передвиньте плейхед, потащите, передвиньте ещё, потащите снова, и поле плавно едет между ними. Ctrl+Z отменяет всё перетаскивание целиком.
- Ключи это ромбики на полоске Motion над рядами. Клик по ромбику ведёт к нему, правый клик убирает его.

# Рецепты
- Каждая вторая доля в кийае: Every 2nd beat, брошенный с плейхедом в кийае.
- Тряска на каждой большой ноте: Big note slam.
- Ноты пропадают в последнем припеве: выделите его ноты, нажмите Hide the notes.
- Четверть оборота каждый такт, и так и остаётся: Quarter turns.

Блоки складываются друг с другом, с ключевыми кадрами и с нодами. Keyframes и Nodes спрятаны под Advanced... — для того, чего блоки пока не умеют.

> Всё здесь только визуальное: судейство, очки и pp не меняются.
",
    },
    Chapter {
        title: "Траектории нот",
        try_it: Some(("Положить пример на эту карту", TryIt::MotionShowcase)),
        body: "\
F4 Effects, страница Motion (.tko-карты): выбранные ноты летят по нарисованному пути, а не по лайну, и приходят на барабан ровно в момент удара.

# Как дать нотам путь
- Выделите ноты на таймлайне (можно одну) и нажмите Give the picked notes a path. Они получат свою группу и для начала прилетят сверху (From above).
- В Where they come from можно выбрать другой старт: снизу, из-за барабана, из угла, дугой, падением, зигзагом, спиралью, петлёй.
- Путь правится прямо на превью: тащите точку, двойной клик добавляет новую, правый клик убирает. Белая точка: откуда ноты прилетают; заканчивается путь всегда на барабане.

# Как они по нему летят
- Нота проходит такую же долю пути, какую прошла бы по лайну: появляется в начале пути, когда появилась бы в конце лайна. SV продолжает работать: медленные ноты медленные и на пути.
- Pace along it меняет это: Stop half way останавливает их на середине, Rush the end бросает к барабану рывком, Soft landing тормозит у барабана, Back up один раз откатывает назад.

# Когда
- Always: на пути всю песню.
- Onto the path here и Back to the lane here плавно переводят ноты за одну долю от плейхеда; вместе ими можно пустить ноты по пути только в припеве.
- Show the path рисует путь тускло для игрока, чтобы было видно, откуда прилетят ноты.

# Больше нот с тем же движением
- Copy motion, выделите другие ноты, Paste motion: у них будет тот же путь, темп и ключевые кадры.
- Select them снова выделяет ноты группы. Нота летит с одной группой: новый путь забирает её из старой группы.

# Перед ударом
- Вкладка Before the hit даёт выбранным нотам движение, которое каждая делает перед своим ударом, где бы она ни была: проявиться поздно, исчезнуть заранее, выскочить, упасть сверху, подняться снизу, подпрыгнуть, закрутиться, мигнуть, задрожать, сплющиться в момент удара.
- Или задайте его сами на графике: выберите, что меняется (прозрачность, размер, вверх/вниз, вбок, поворот, скрытие, ширина, высота, оттенок), клик добавляет ключ, его можно тащить, правый клик убирает. Слева 4 доли до удара, яркая линия это сам удар. Ключи встают на 1/8 доли; с Shift ставятся свободно.
- Работает вместе с путём и со страницей Keyframes: сдвиги и повороты складываются, размер и прозрачность перемножаются.

# Скролл (SV как в mania)
- Вкладка Scroll задаёт одну скорость для всего лайна по ходу песни, как SV в osu!mania: 1x как в карте, 2x вдвое быстрее, 0x замораживает все ноты на месте, -1x гонит их назад. Зелёные линии остаются как есть; скролл ложится сверху, так что зелёная линия 2x под скроллом 0.5x играется как 1x.
- Выбранная скорость держится от плейхеда до следующего ключа. Slower и Faster подкручивают её, Glide to the next key плавно переводит к следующей скорости вместо скачка.
- Движения от плейхеда: Freeze a beat, Back up a beat, Lurch on each beat (a bar), Jump a beat ahead. Каждое возвращается к скорости, с которой началось.
- График под таймлайном показывает скорость: клик добавляет ключ, перетаскивание двигает, правый клик удаляет.
- Slide the drum along the lane за одну долю от плейхеда сдвигает мишень: Home (на место), A quarter in (на четверть), Half way (на середину), Far (далеко). Ноты встречают её там, где она стоит. Это эффект Move the drum, так что его можно двигать и на странице Keyframes, и клипами.
- Как бы ни шёл скролл, каждая нота приходит на барабан ровно в момент удара.

# Фейковые ноты
- Вкладка Fakes делает выбранные доны и каты фейками (и обратно): они выглядят и двигаются как ноты, но по ним не бьют, они не дают счёта и комбо, не звучат и исчезают на барабане.
- Add fake copies of the picked notes ставит фейки на полдоли или 1/4 до или после выбранных нот, или на долю после, где ноты ещё нет.
- На таймлайне фейки полые с буквой F. Их выделяют, двигают, копируют и удаляют как обычные ноты; группы, траектории, движения и скролл берут их по времени.

# Show as
- Вкладка Show as заставляет выбранные ноты врать о своём виде: все как доны, все как каты или наоборот, и все маленькие, все большие или наоборот.
- The truth shows возвращает им настоящий вид за 1/4 доли, к самому удару или за две доли до него. Цвет, который виден только в момент удара, не прочитать, только выучить: Upload об этом предупредит.
- Чем бить ноту, её счёт и звук остаются её собственными. Фейки врут вместе со своей группой; дроби и свели не врут никогда.
- Игроки могут выключить любой обман в настройках: Игра, «Показывать настоящие ноты». Если это включено у вас самих, превью тоже покажет правду.

- Положить пример на эту карту (кнопка внизу) раскладывает всё это по первому киаю карты, по четыре доли: траектория, выскакивание с фейками, заморозка, пока едет барабан, перепутанные цвета. Каждая часть отменяется своим Ctrl+Z.

> Траектории, движения, скролл, фейки и обман только визуальные: удары в те же моменты, счёт и pp не меняются. Дроби и свели остаются на лайне. Игроки могут ослабить вспышки, глитчи и движения камеры в настройках: Игра, «Ослабить эффекты карт».
",
    },
    Chapter {
        title: "Ключевые кадры",
        try_it: None,
        body: "\
F4 Effects, страница Keyframes: анимация игрового поля (свойства движения, вида и клонов во вкладках) и групп нот во времени, только в .tko картах.

# Как
- Выберите цель: игровое поле или группу нот. Затем свойство (например Zoom).
- График под таймлайном показывает его ключи: клик добавляет, перетаскивание двигает, правый клик удаляет. Или поставьте курсор и нажмите Key here; Value +/- меняет значение.
- Между двумя ключами значение плавно переходит; Ease задаёт, как именно.
- Группы: выделите ноты в Compose и создайте новую группу, затем дайте ей свою анимацию (сдвиг, масштаб, прозрачность, скрытие, вращение).

Ключи жёстко привязаны ко времени. Если нужно, чтобы что-то реагировало на бит, комбо или промах, используйте Nodes (следующая глава). Они складываются: движение ноды добавляется к ключам самого свойства.

> Всё это только визуально: оценка попаданий, счёт и pp не меняются.
",
    },
    Chapter {
        title: "Ноды",
        try_it: Some(("Открыть страницу Nodes", TryIt::Nodes)),
        body: "\
Ноды — маленькая визуальная программа. Вместо того чтобы анимировать вручную, вы говорите, КОГДА что-то происходит и ЧТО оно делает, и соединяете одно с другим: Every beat -> Pulse по Zoom. Они живут в F4 Effects > Nodes (.tko карты; .osu предложит стать .tko).

# Четыре вида нод (по цвету шапки)
- События (Events): моменты времени. Start, At time, Every beat, Kiai starts / ends, Note arrives, Hit, Miss, Combo reaches, At mark.
- Триггеры: переделывают моменты. Delay, Every Nth, Only first, Gate, Either.
- Значения и математика: числа, меняющиеся во времени. Combo, Accuracy, In kiai, Beat phase, Wave, Random, Fade out, Progress, Span progress, Add, Scale, Blend...
- Действия (Actions): единственные ноды, которые меняют картинку. Pulse, Tween, Shake, Drive, Play clip.

# Два вида проводов
- Оранжевый: моменты. «Что-то происходит сейчас».
- Голубой: числа. Значение, которое можно прочитать в любой момент.
Точка соединяется только с точкой того же цвета. Входы слева от ноды, выход справа. Тяните от точки к точке, чтобы соединить; правый клик по ноде удаляет её.

# Ваш первый эффект
- 1. F4, страница Nodes. Нажмите + Every beat справа: появится нода.
- 2. Нажмите + Pulse (Actions). Появится вторая нода.
- 3. Протяните от правой точки Every beat к точке on у Pulse. Провод оранжевый.
- 4. Нажмите на ноду Pulse. В списке справа выберите цель — Zoom. Поставьте amount 0.06 и ms 260 (клик по числу — ввести).
- 5. Нажмите Space. Поле дышит на каждый бит.

# Что делают действия
- Pulse: толкает цель на amount в каждый момент и возвращает за ms.
- Tween: сдвигает цель к значению и обратно, по разу в каждый момент.
- Drive: постоянно держит цель на подведённом числе, умноженном на gain. Для всего, что следует за значением (комбо, kiai, волна).
- Shake: трясёт всё поле в случайные стороны, затухая. Цель не нужна.
- Play clip: короткая готовая серия ключей.
- Вход scale у действия необязателен: число, которое умножает эффект, читается в момент срабатывания. Пустой — считается за 1.

# Разобранные примеры
- Только каждый 4-й бит: Every beat -> Every Nth (n = 4) -> Pulse.
- Вспышка только в kiai: Every beat -> Gate; In kiai подключите ко входу open у Gate; Gate -> Pulse (цель Flash).
- Каждый финишер сильнее предыдущего: Note arrives (type 3) -> Progress; подключите и триггер, и Progress ко входам on и scale у Pulse.
- Зум следует за комбо: Combo -> Scale (times 0.002) -> Drive (цель Zoom).
- Поворот от одной точки до другой: поставьте метки клавишей M. Span progress (from 1 to 2) -> Drive (gain 360, цель Rotation).
- Бесконечное покачивание: Wave (beats 4, amount 10) -> Drive (цель X).

# Ничего не происходит?
- Красный ! на ноде значит, что она пока ничего не делает. Наведите на него — подскажет, чего не хватает.
- У действия без цели красным написано no target. Выберите цель справа.
- Событие должно быть подключено к первому входу действия (on / flip / value).
- События срабатывают, пока играет песня. После прыжка курсора запускайте воспроизведение чуть раньше.
- Kiai-нодам нужен включённый kiai на линии таймингов. At mark, Between и Span требуют меток (M); они считаются с 1 по порядку времени.
- Совсем малый amount трудно разглядеть. Попробуйте больше: Zoom 0.1-0.3, Shake от 10.
- Рецепты (следующая глава) — готовые рабочие примеры, их можно открыть и прочитать.
",
    },
    Chapter {
        title: "Рецепты нод",
        try_it: Some(("Открыть страницу Nodes", TryIt::Nodes)),
        body: "\
Меню Recipe на странице Nodes вставляет готовую группу соединённых нод. Меняйте её числа, перенацеливайте или разберите, чтобы понять, как она устроена.

{recipes}

> На одной карте может быть сколько угодно рецептов. Каждый — один шаг отмены.
",
    },
    Chapter {
        title: "Загрузка и статусы",
        try_it: None,
        body: "\
File > Upload отправляет сохранённую карту на taiko.rs.

# Что происходит
- Сначала чек-лист: вы вошли в аккаунт, карта сохранена, это тайко-карта минимум с 10 нотами, заполнены исполнитель / название / маппер, музыка лежит в папке и это ваша собственная карта (без osu! BeatmapSetID).
- Send упаковывает папку карты (все тайко-сложности с достаточным числом нот, музыку, картинки, сториборд) и загружает.
- Сет сразу публичный со статусом CUSTOM: он виден во вкладке CUSTOM на сайте, его может скачать любой. Игры на нём не дают pp.
- Админ или loved-номинатор может его одобрить. APPROVED CUSTOM даёт pp.

# Обновление
Номер сета хранится в папке карты, поэтому повторная отправка обновляет тот же сет (кнопка Update the set). Что было одобрено, продолжает считаться; изменённые сложности ждут нового одобрения.

# В игре
У кастомных карт свои заголовки в выборе песен (taiko.rs custom, Custom approved), а status:custom находит их. Карты, скачанные с сайта, тоже приходят с отметкой.

> Лимиты: 30 сетов на аккаунт, 40 МБ каждый, одна отправка в 30 секунд.
",
    },
    Chapter {
        title: "Клавиши",
        try_it: None,
        body: "\
# Воспроизведение и перемещение
- Space или C: играть / пауза. X: играть с начала (как в stable).
- F5: тест-плей. Карта как она есть сейчас (сохранённая или нет) играется в настоящей игре с плейхеда, Shift+F5 — с начала, с No Fail и без сохранения результата; выход или конец карты возвращают сюда.
- Tab: тот же тест, но играет автоплей (Shift+Tab — с начала), чтобы посмотреть карту.
- Z / V: первая / последняя нота. Влево / вправо: по шагу сетки. Вверх / вниз: предыдущая / следующая линия таймингов.
- Колесо: перемотка. Ctrl+колесо: snap. Alt+колесо: масштаб над таймлайном, в остальных местах громкость. Средняя кнопка: сдвиг (View > Left drag pans даёт сдвигать пустой таймлайн и левой кнопкой; рамка тогда через Ctrl+перетаскивание).
- Ctrl+вверх / вниз: скорость воспроизведения (с Shift по 5%).

# Ноты
- 1 2 3 4: инструменты. W E: кат, большая. Q: сменить цвет. J K: сдвиг. Ctrl+D: клон. Ctrl+G: разворот.
- Ctrl+Z / Y: отмена, повтор. Ctrl+X / C / V: вырезать, копировать, вставить. Ctrl+A: выделить всё. Delete.

# Тайминг и прочее
- Ctrl+P: красная линия. Ctrl+Shift+P: зелёная линия. Ctrl+I: удалить линию. F6: окно таймингов.
- Ctrl+B: закладка. Ctrl+влево / вправо: предыдущая / следующая закладка.
- M: метка эффектов в курсоре (.tko). Shift+M убирает ближайшую.
- F1 F2 F3 F4: режимы. Ctrl+S: сохранить. Esc: выйти (спросит, если не сохранено).
- ? показывает все клавиши.
",
    },
];
