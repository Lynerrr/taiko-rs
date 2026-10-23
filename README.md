# taiko-rs

A small, fast osu!taiko client in Rust, built for Linux/Wayland, with a Windows build.

- **Native Wayland** via winit (falls back to X11 automatically), **Vulkan** via wgpu.
- **Smooth scrolling**: the game clock free-runs on the monotonic clock and is gently
  steered toward the audio device position, so notes never step or jitter even though audio
  arrives in buffer-sized chunks. Every note position is computed from that clock each frame
  (no accumulated motion), drawn as analytically anti-aliased SDF shapes in a single
  instanced draw call — sub-pixel accurate at any resolution and refresh rate.
- **Low latency**: native **PipeWire** output with a 256-frame buffer, hitsounds mixed directly
  in the audio callback, and a dedicated render thread so key presses are handled and
  timestamped the instant they arrive — even while a vsync'd frame is waiting on the display.
- Plays native taiko maps and osu!standard converts (lazer's conversion rules: split
  sliders, drumrolls, swells, big notes), with lazer hit windows.

## Build & run

```sh
cargo build --release
./target/release/taiko-rs                       # scans your osu! stable Songs folder (osu-winello / Wine)
./target/release/taiko-rs ~/Downloads/*.osz     # or any Songs folder, map folder, .osz, or .osu
./target/release/taiko-rs map.osu               # a single .osu starts immediately
./target/release/taiko-rs --auto map.osu        # autoplay
./target/release/taiko-rs --info map.osu        # print converted map stats (headless)
```

Build needs `clang` and PipeWire headers for the default `pipewire` feature; build with
`--no-default-features` to use ALSA only, or set `TAIKO_ALSA=1` at runtime.

## Windows

Install or update with one PowerShell command (no admin needed):

```powershell
irm https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client/install.ps1 | iex
```

It downloads the latest build from the [releases](https://github.com/Lynerrr/taiko-rs/releases/tag/latest)
into `%LOCALAPPDATA%\Programs\taiko-rs`, adds Start menu and desktop shortcuts and launches it.
Maps come from your osu! stable Songs folder (`%LOCALAPPDATA%\osu!\Songs`); without osu!, drop
`.osz` files into `%APPDATA%\taiko-rs\Songs`. Settings live in `%APPDATA%\taiko-rs\config.txt`.
The build is produced by GitHub Actions on every push.

## Controls

| Key | Action |
| --- | --- |
| D / F / J / K | left rim (kat) / left centre (don) / right centre / right rim |
| Enter, ↑/↓, ←/→ | play, select difficulty, jump between sets |
| type, Backspace | search |
| Tab (or F1) | switch between the SONGS and SETTINGS tabs |
| [ / ] (Shift: ×5) | speed (rate) −/+ 0.05× on song select; host sets the match rate in the lobby |
| F5 / F6 (rebindable) | previous / next playfield resolution preset, anywhere |
| mouse (in game) | drag inside the playfield to move it, drag its edges/corners to resize it, wheel to scale it |
| Space | skip intro |
| Esc (in game) | pause menu: continue, retry, playfield size, settings, quit; Esc again resumes |
| F2 / Shift+F2 | random song / back to the previous random pick (song select) |
| Ctrl+← / → | seek the song select preview ±5 s |
| Shift+Enter | play the selected map from the start point picked on its timeline |
| Ctrl+Enter / click the autoplay box | fullscreen preview (song select); P in the multiplayer lobby |
| Alt + wheel | master volume (anywhere) |
| O (in game or pause menu) | apply the live offset suggestion shown under the hit error meter |
| `` ` `` | restart map |
| Esc (song select) | clear search / quit |
| - / = (Shift: ×5) | audio offset −/+ 1 ms |
| F3 / F4 | scroll speed |
| F7 | gameplay present mode: vsync → mailbox → immediate (menus stay on vsync by default) |
| F8 | FPS cap for mailbox/immediate: unlimited, 2000, 1000, 500, 240, 144, 60 |
| F10 / F11 | FPS overlay / fullscreen (the game starts fullscreen; F11 is remembered) |

**Mouse:** everything can also be clicked. The top tabs switch screens; in the song list the
first click selects a song or difficulty and clicking the selected difficulty plays it; settings
rows select on the first click and toggle/open on the second, and `<` / `>` step values; the pause
menu follows the pointer; results, calibration and the multiplayer menu, lobby and results have
buttons (each shows its key).

**Animations** (Settings → Video → *UI animations*, on by default): screens fade in, the tab and
selection highlights glide, a song's difficulties slide open, settings scroll smoothly, the pause
menu slides in, results count up with the grade popping in, toasts and the volume overlay slide,
and the preview fades in on a new map. Gameplay itself is never animated or delayed by this.

Everything is adjustable in the **Settings** tab (scroll speed, offset, key bindings, converts,
volumes, audio buffer, present mode, FPS cap, overlay, fullscreen) and saved
immediately to `~/.config/taiko-rs/config.txt` (Windows: `%APPDATA%\taiko-rs\config.txt`). Losing window focus mid-song pauses the game. The results screen shows UR and mean hit error and tells
you which way to move the offset.

## Song select

Songs are grouped: each beatmap set is one row, and the selected set expands to its difficulties,
ordered by star rating. Star ratings are osu!'s own (via [rosu-pp](https://github.com/MaxOhn/rosu-pp),
a precise port of lazer's calculator, converts included) at the current speed rate, coloured like
osu!'s spectrum and calculated in the background for the selected song. Up/Down moves through difficulties, Left/Right jumps between songs, the wheel
scrolls. The selected map's background fills the screen behind the menu, its music plays from the
map's preview point (looping), and the panel on the right shows its details (OD, BPM, length,
note counts) above a live autoplay preview synced to that music, and a difficulty timeline like
lazer's song progress graph: osu!'s strain over the whole map (kiai tinted, preview point and
playhead marked, densest second called out). Click or drag on it, use the wheel over it, or
Ctrl+←/→ to seek the music and autoplay to any part of the map. Wherever you last scrolled to is
remembered (a blue flag): **Play from 1:23** under the timeline (or Shift+Enter) starts the map there,
with the music two seconds earlier and only the notes from that point on counting; retry keeps the
start point. F2 picks a random song.

**Fullscreen preview:** click the autoplay box (or Ctrl+Enter) to watch the autoplay over the
whole screen, drawn in your playfield area so you can move and resize it right there (drag, drag
the edges, wheel; F5/F6 or the *Playfield* button for presets) and see how it will look. It is marked as a preview (blue frame,
PREVIEW header, no score) and has the full-width timeline: click to seek and pick a start point,
then Enter or **Play from 1:23**. Esc goes back. Both previews play at the selected speed rate
(music and autoplay; changing it carries on from the same point), and the match rate in the lobby. In a multiplayer lobby, P opens the same preview
of the lobby map; the host's button is **Start the match from 1:23**, which sets where everyone's
match starts (shown next to the map; players re-ready, as after a rate change). The lobby also
plays the lobby map's music. The background also shows behind
the lane during play, darkened by Settings → *Background dim*.

## Playfield size and speed

- **Playfield area:** gameplay can run in a smaller "window" inside a fullscreen game, handy on big
  monitors. Everything (lane, notes, HUD, pause menu, multiplayer scoreboard) is laid out inside
  the area as if it were the window, and clipped to it. During play, drag inside it to move it,
  drag an edge or corner to resize it (the cursor changes like on a window frame) and use the
  wheel to scale it. Settings → *Playfield area* opens an editor with a sample playfield (mouse,
  or arrows to move / Shift+arrows to resize, R for full screen); *Reset playfield area* restores
  full screen. While dragging, the area snaps to the screen's edges and centre lines (guides are
  shown; hold Shift to place freely). *Playfield preset* (Enter opens a list grouped by aspect
  ratio, largest first; or F5/F6 anywhere, `[` / `]` in the editor) sizes it like a window of a
  given resolution, centred: your monitor's modes plus common ones such as 1280x720, 1024x768 and
  800x600. The playfield's resolution is shown in the bottom-right corner during play (Settings →
  *Show playfield size*); click it, or choose *Playfield size* in the pause menu, to pick a preset
  from the same list mid-game. *HUD position* keeps score, accuracy, the hit error meter and
  map info inside the playfield, or fixed at the screen's corners.
- **Speed (rate):** 0.50×–2.00× in 0.05× steps (`[` / `]` on song select, Shift for 0.25×, or
  Settings → *Speed*). With *Rate keeps pitch* on (default) the music is time-stretched (WSOLA) so
  it sounds normal, like osu!'s DT/HT; off, it is resampled and the pitch follows the speed
  (nightcore-style). As in osu!, hit windows stay in map time, so faster is harder; offset, UR
  and the offset suggestion stay in real milliseconds. In multiplayer the host's rate applies to
  the match.

## Offset calibration

Settings → **Offset calibration** plays a generated click track (tempo set by **Calibration tempo**,
60–240 BPM) with a centre note exactly on every click, sample-accurate. Hit F/J along with the
clicks; your own hitsounds are muted so you sync to the click, and **H** hides the notes to
calibrate purely by ear. The overlay shows your average error and the suggested offset live; at
the end (or **Enter** to finish early) you get the result, using a median-filtered average that
ignores stray taps, with **Enter** to apply, **R** to retry (Left/Right changes the tempo) and
**Esc** to discard.

You can also skip calibration: while playing any map, the line under the hit error meter shows
your average error with its 95% margin, your current UR, and a suggested offset. It is scaled to
your consistency: the window grows until the average is known to about ±3 ms (≈40 hits at UR 100,
≈100 at UR 150, up to 200), it only flags an offset problem when the error exceeds that margin,
and noisy estimates suggest a proportionally smaller change. Press **O**
to apply it on the spot; hits recorded before the change are re-expressed under the new offset,
so the suggestion settles instead of overshooting. Toggle it with Settings → Live offset suggestion.

## Multiplayer

Versus play over direct TCP (default port **7777**). Tab to the **MULTIPLAYER** tab:

- **Host:** set your name and port, choose *HOST A GAME*. Friends need your public IP and the
  port must be reachable (port-forwarded / open). In the lobby press **M** to pick the map from
  your song list, **Space** to ready up, **Enter** to start once players are ready.
- **Join:** type the host's `ip` or `ip:port` and choose *JOIN GAME*; **Space** to ready up.

Maps are matched by a hash of the `.osu` file, so everyone needs the same difficulty in their
Songs folder (players without it show "no map"). Each client syncs its clock to the host with
pings, and the song starts at a shared instant 3 s after everyone has loaded. During the match a
live scoreboard shows everyone's score, accuracy and combo; there is no pause (Esc twice
forfeits). Results show score, accuracy, max combo, judgements and UR for everyone.

## Frame pacing on Wayland

Settings → *Present mode (gameplay)* applies during songs; with *Menus use vsync* (default) song
select, its autoplay preview, settings and results always use vsync.

**Use vsync (the default).** A Wayland compositor shows exactly one frame per refresh no matter
how many you render, so mailbox/immediate don't put more frames on screen: the compositor picks
whichever frame finished last, whose age is random each refresh (visible judder), and thousands
of buffer commits per second compete with the compositor for the GPU — especially on an iGPU.
Immediate only tears (the one real latency win) in fullscreen.

Judging never depends on the frame rate: keys are timestamped on their own thread and audio is
the clock, so vsync costs no accuracy.

If you do use mailbox, cap it (F8) to a few times your refresh rate; the limiter sleeps and then
spins so frames stay evenly spaced.

## Diagnostics

- `TAIKO_STATS=1` prints frame-time percentiles, clock-vs-wall-time jitter, clock-vs-audio
  drift and how long each frame blocked in acquire/present after each play.
- `TAIKO_PROFILE=1` prints the frame rate, CPU time to build a frame, submit time and shape
  count every 2 seconds, in any screen (try with menus off vsync to see the uncapped rate).
- `TAIKO_SHOTS=/dir:1000,5000` saves PNG screenshots at those gameplay times (plus the select
  and results screens); `TAIKO_EXIT_AFTER=1` quits after the results screenshot.
- `TAIKO_INPUT=2000:Tab,3000:Enter` injects key presses and `TAIKO_WALL_SHOTS=1500,2500` takes
  screenshots at wall-clock ms after launch, for scripted UI tests.

## Layout

| File | |
| --- | --- |
| `src/beatmap.rs` | `.osu` parser, timing/SV control points, taiko conversion, barlines |
| `src/audio.rs` | decode (symphonia) + resample, cpal/PipeWire output, mixer, hitsound synth |
| `src/clock.rs` | audio-slaved smooth game clock |
| `src/game.rs` | judging, scoring, playfield drawing |
| `src/render.rs` | wgpu SDF shape renderer, text, present mode handling |
| `src/ui.rs` | song select and results screens |
| `src/settings.rs` | settings tab items and layout, tab bar |
| `src/calibrate.rs` | offset calibration: estimate, overlay and result screen |
| `src/net.rs` | multiplayer protocol, room server, client with clock sync |
| `src/multiplayer.rs` | multiplayer session, menu, lobby, scoreboard and results screens |
| `src/library.rs` | Songs folder / `.osz` scanning |
