# Nodes guide

Nodes are the editor's visual way to make effects react to the music and to the player. They work in `.tko` maps, on **F4 Effects > Nodes**. Everything they do is visual: judging, score and pp never change. (The same text is in the editor: Help > Manual > Nodes.)

## The idea

[Keyframes](map-editor.md#effects-tko) move a property at fixed times. A node graph says *when* something happens and *what it does*, and wires the two together:

```
Every beat ──► Pulse (target: Zoom)
```

Read it as "on every beat, kick the zoom". Change the beat for a kiai start, a miss or a combo of 50 and the same pulse reacts to that instead.

There are four kinds of node, told apart by the colour of their header:

| Kind | What it is | Examples |
|---|---|---|
| Events | A moment in time | Every beat, Kiai starts, Note arrives, Hit, Miss, Combo reaches, At mark |
| Triggers | Reshape moments | Delay, Every Nth, Only first, Gate, Either |
| Values and math | A number that changes over time | Combo, Accuracy, In kiai, Beat phase, Wave, Progress, Add, Scale |
| Actions | The only nodes that change the picture | Pulse, Tween, Shake, Drive, Play clip |

And two kinds of wire:

- **Orange** wires carry *moments*: something happens now.
- **Cyan** wires carry *numbers*: a value you can read at any time.

A dot connects only to a dot of the same colour. Inputs are on the left of a node, its output on the right.

## Your first effect

1. Open F4 Effects and the **Nodes** tab. (A `.osu` map offers to become a `.tko` first.)
2. Click **+ Every beat** on the right. A node appears on the canvas.
3. Click **+ Pulse** (in Actions).
4. Drag from Every beat's right dot to Pulse's **on** dot. The wire is orange.
5. Click the Pulse node. In the list on the right pick its **target**: Zoom. Set `amount` to 0.06 and `ms` to 260 (click a number to type it).
6. Press Space. The playfield breathes on every beat.

The editor helps while you build: a card on the canvas ticks off these steps, a red **!** on a node means it can't do anything yet (hover it to read why), and every node has a tip.

## What the actions do

- **Pulse** kicks the target by `amount` at each moment, then lets it settle back over `ms`.
- **Tween** moves the target to a value and back, one way at each moment.
- **Drive** holds the target at the number wired in, times `gain`, all the time. Use it for anything that *follows* a value: the combo, kiai, a wave.
- **Shake** jolts the whole playfield in random directions, fading out. It needs no target.
- **Play clip** plays a short ready-made run of keyframes.
- The **scale** input of an action is optional: a number that multiplies the effect, read at the moment it happens. Left empty it counts as 1.

An action's motion is added to the property's own keyframes, so the two work together.

## Worked examples

- **Only every 4th beat**: Every beat → Every Nth (n = 4) → Pulse.
- **A flash only in kiai**: Every beat → Gate; wire In kiai into the Gate's *open* input; Gate → Pulse (target Flash).
- **Each finisher harder than the last**: Note arrives (type 3) → Progress; wire both the trigger and Progress into a Pulse's *on* and *scale* inputs.
- **Zoom follows the combo**: Combo → Scale (times 0.002) → Drive (target Zoom).
- **A turn between two points**: drop marks with `M`. Span progress (from 1 to 2) → Drive (gain 360, target Rotation).
- **Wobble forever**: Wave (beats 4, amount 10) → Drive (target X).

## Nothing happens?

- A red **!** on a node: it can't do anything yet. Hover it.
- An action with no target says "no target" in red. Pick one in the list on the right.
- The event must be wired into the action's first input (*on*, *flip* or *value*).
- Events fire as the song plays. After jumping the playhead, play from a little earlier.
- Kiai nodes need kiai switched on a timing line. *At mark*, *Between marks* and *Span progress* need marks (`M`), counted from 1 in time order.
- A very small amount is hard to see. Try Zoom 0.1 to 0.3, Shake 10 or more.

## Recipes

The **Recipe** menu on the Nodes page drops a ready-made group of wired nodes. Change its numbers, retarget it, or take it apart to see how it works. Each one is a single undo step.

- **Beat pulse**: The playfield breathes: a small zoom kick on every beat.
- **Kiai drop**: The moment kiai starts: a white flash, a zoom kick and a hard shake.
- **Finisher build-up**: Every finisher kicks the zoom, flashes and shakes, each harder than the one before (progress through the finishers scales them).
- **Combo zoom**: The playfield grows as the combo climbs and falls back on a miss.
- **Miss glitch**: A miss tears the picture into slices and shakes it.
- **Kiai echo trail**: In kiai the playfield leaves 4 fading echoes, and each beat flicks them round.
- **Section sweep**: The playfield turns once, from mark 1 to mark 2 (place the marks with M).
- **Kiai colour wash**: In kiai the playfield gets a colour wash whose hue drifts round and round.
- **Drumroll shake**: Each drumroll shakes the playfield when it arrives.
- **Breathing vignette**: The dark edges of the screen swell and fade, once every 4 beats.
- **Build-up between marks**: From mark 1 to mark 2 the zoom and the white glare build up, slowly at first and then fast (place the marks with M).
- **Half turn every 4 bars**: Every 16 beats (4 bars of 4/4) the playfield turns half a revolution, and back at the next.
- **Combo milestone flash**: Every 50 combo the screen flashes and the zoom kicks. Change the number in the first node.
- **Accuracy tint**: A red wash that grows as the accuracy falls: clean play stays clear.
- **Hit jitter**: Every hit gives the playfield a tiny jolt.
- **Kiai spin**: The moment kiai starts the playfield turns one full revolution.
- **Big note slam**: Each finisher slams the same way: a zoom kick, a hard shake and a colour split.

## Node reference

Numbers are shown as `name (default; range)`. For the Note arrives, Hit and Miss nodes, `group` 0 is every note; `type` is 0 any, 1 don, 2 kat, 3 finisher, 4 drumroll.

### Events (orange output: moments)

| Node | Inputs | Numbers (default, range) | What it does |
|---|---|---|---|
| Start | - | - | Fires once, when the map starts. |
| At time | - | ms (1000; 0 to 3600000) | Fires once, at this many milliseconds into the map. Prefer marks and notes: a time stops matching the moment the chart changes. |
| Every beat | - | every (1; 1 to 64), offset (0; 0 to 63) | Fires on the beat grid: every N beats, starting this many beats in. The grid follows the red lines. |
| Kiai starts | - | - | Fires each time kiai time begins. |
| Kiai ends | - | - | Fires each time kiai time ends. |
| Note arrives | - | group (0; 0 to 256), type (0; 0 to 4), nth (0; 0 to 1000) | Fires when a note arrives at the hit circle. Group 0 is every note, or pick a group; type narrows it to dons, kats, finishers or rolls; nth picks only the Nth of those (0 is all of them). |
| Hit | - | group (0; 0 to 256), judge (0; 0 to 2), type (0; 0 to 4) | Fires when a note is hit. Group 0 is any note; judge 0 is any hit, 1 only great, 2 only ok; type narrows it to dons, kats, finishers or rolls. |
| Miss | - | group (0; 0 to 256), type (0; 0 to 4) | Fires when a note is missed. Group 0 is any note; type narrows it to dons, kats, finishers or rolls. |
| Combo reaches | - | every (50; 1 to 1000) | Fires each time the combo reaches a multiple of this number. |
| At mark | - | mark (0; 0 to 256) | Fires at a mark you placed in the editor (M). Mark 0 fires at every mark, 1 at the first, and so on. |

### Triggers (orange in, orange out)

| Node | Inputs | Numbers (default, range) | What it does |
|---|---|---|---|
| Delay | in (moments) | ms (250; 0 to 60000) | Passes each moment on after a pause. |
| Every Nth | in (moments) | n (2; 1 to 1000) | Passes on every Nth moment and swallows the rest. |
| Only first | in (moments) | - | Passes on only the first moment. |
| Gate | in (moments), open (number) | - | Passes moments on only while the number wired in is at least 0.5. |
| Either | a (moments), b (moments) | - | Fires when either input fires. |

### Values (cyan output: numbers)

| Node | Inputs | Numbers (default, range) | What it does |
|---|---|---|---|
| Number | - | value (1; -1000 to 1000) | A fixed number. |
| Seconds | - | - | Seconds since the map started. |
| Beat phase | - | - | Climbs from 0 to 1 through each beat, then starts again. |
| Beat count | - | - | How many beats of the map's grid have passed, with the fraction. |
| In kiai | - | - | 1 during kiai time, 0 otherwise. |
| Combo | - | - | The player's current combo. |
| Accuracy | - | - | The player's accuracy so far, from 0 to 1. |
| Random | - | seed (1; 0 to 1000000), beats (1; 0.25 to 64) | A number that changes every few beats. It comes from the map's seed, so replays match. |
| Smooth noise | - | beats (1; 0.25 to 64), seed (1; 0 to 1000000) | A number from 0 to 1 that drifts smoothly, picking a new target every few beats (from the map's seed, so replays match). |
| Wave | - | beats (4; 0.25 to 64), amount (1; -1000 to 1000), shape (0; 0 to 3) | Swings between minus and plus the amount, once per this many beats. Shape 0 is a sine, 1 a triangle, 2 a saw, 3 a square. |
| Flip-flop | flip (moments) | - | Flips between 0 and 1 at each moment. |
| Fade out | in (moments) | ms (300; 1 to 60000) | Jumps to 1 at each moment and falls to 0 over this time. |
| Count | in (moments) | - | How many moments have happened so far: 0, then 1, 2, 3 ... |
| Progress | in (moments) | - | How far through all its moments the trigger is: 0 before the first, 1 after the last, in even steps between. Wire it to a scale input for a build-up. |
| Between marks | - | from (1; 0 to 256), to (2; 0 to 256) | 1 between two marks, 0 outside. Mark 0 stands for the start of the map (from) or its end (to). |
| Span progress | - | from (1; 0 to 256), to (2; 0 to 256) | Climbs from 0 to 1 between two marks and stays there. Mark 0 stands for the start of the map (from) or its end (to). |

### Math (cyan in, cyan out)

| Node | Inputs | Numbers (default, range) | What it does |
|---|---|---|---|
| Add | a (number), b (number) | - | a plus b. |
| Subtract | a (number), b (number) | - | a minus b. |
| Multiply | a (number), b (number) | - | a times b. |
| Greater than | a (number), b (number) | - | 1 while a is greater than b, else 0. |
| Not | a (number) | - | 1 when a is 0, else 0. |
| Absolute | a (number) | - | a without its sign. |
| Smaller | a (number), b (number) | - | The smaller of a and b. |
| Larger | a (number), b (number) | - | The larger of a and b. |
| Blend | a (number), b (number), by (number) | - | Slides from a to b as the third input goes from 0 to 1. |
| Power | a (number) | exp (2; -8 to 8) | a to this power (a below 0 gives 0). Bends a 0 to 1 number into a curve. |
| Steps | a (number) | steps (4; 1 to 256) | Rounds a down to this many even steps between 0 and 1. |
| Clamp | a (number) | min (0; -1000 to 1000), max (1; -1000 to 1000) | Keeps a between min and max. |
| Scale | a (number) | times (1; -1000 to 1000), plus (0; -1000 to 1000) | a times this, plus that. |

### Actions (they change the picture)

| Node | Inputs | Numbers (default, range) | What it does |
|---|---|---|---|
| Drive | value (number) | gain (1; -1000 to 1000) | Holds the target at this number times the gain, all the time. Added to its keyframes. |
| Pulse | on (moments), scale (number) | amount (0.1; -1000 to 1000), ms (300; 1 to 60000) | Kicks the target by an amount at each moment, then lets it settle back. The scale input multiplies the kick, read at the moment it happens. |
| Tween | flip (moments), scale (number) | to (1; -1000 to 1000), ms (300; 1 to 60000) | Moves the target to a value and back, one way at each moment. The scale input multiplies the value, read at the moment it happens. |
| Shake | on (moments), scale (number) | amount (12; 0 to 400), ms (300; 1 to 60000) | Jolts the whole playfield in random directions at each moment, fading out. The scale input multiplies the strength, read at the moment it happens. |
| Play clip | on (moments), scale (number) | hold (0; 0 to 1) | Plays a short ready-made run of keyframes at each moment. The scale input multiplies it, read at the moment it happens. |

