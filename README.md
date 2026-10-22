# taiko-rs

A small, fast osu!taiko client in Rust, built for Linux/Wayland.

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

## Controls

| Key | Action |
| --- | --- |
| D / F / J / K | left rim (kat) / left centre (don) / right centre / right rim |
| Enter, ↑/↓, ←/→ | play, select difficulty, jump between sets |
| type, Backspace | search |
| Tab (or F1) | switch between the SONGS and SETTINGS tabs |
| Space | skip intro |
| Esc (in game) | pause menu: continue, retry, settings, quit; Esc again resumes |
| `` ` `` | restart map |
| Esc (song select) | clear search / quit |
| - / = (Shift: ×5) | audio offset −/+ 1 ms |
| F3 / F4 | scroll speed |
| F7 | present mode: vsync → mailbox → immediate |
| F8 | FPS cap for mailbox/immediate: unlimited, 2000, 1000, 500, 240, 144, 60 |
| F9 | vsync frame delay: 0–14 ms (see below) |
| F10 / F11 | FPS overlay / fullscreen |

Everything is adjustable in the **Settings** tab (scroll speed, offset, key bindings, converts,
volumes, audio buffer, present mode, frame delay, FPS cap, overlay, fullscreen) and saved
immediately to `~/.config/taiko-rs/config.txt`. Losing window focus mid-song pauses the game. The results screen shows UR and mean hit error and tells
you which way to move the offset.

## Frame pacing on Wayland

**Use vsync (the default).** A Wayland compositor shows exactly one frame per refresh no matter
how many you render, so mailbox/immediate don't put more frames on screen: the compositor picks
whichever frame finished last, whose age is random each refresh (visible judder), and thousands
of buffer commits per second compete with the compositor for the GPU — especially on an iGPU.
Immediate only tears (the one real latency win) in fullscreen.

Judging never depends on the frame rate: keys are timestamped on their own thread and audio is
the clock, so vsync costs no accuracy. To cut vsync's *visual* latency, raise the **frame delay**
(F9): after vsync releases a buffer, the renderer waits that long before sampling the clock, so frames are
built closer to the compositor's deadline. Increase it until the FPS overlay's "worst" frame time
starts to jump or notes hitch, then step back one notch (a 60 Hz display has ~16.7 ms per frame).

If you do use mailbox, cap it (F8) to a few times your refresh rate; the limiter sleeps and then
spins so frames stay evenly spaced.

## Diagnostics

- `TAIKO_STATS=1` prints frame-time percentiles, clock-vs-wall-time jitter, clock-vs-audio
  drift and how long each frame blocked in acquire/present after each play.
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
| `src/library.rs` | Songs folder / `.osz` scanning |
