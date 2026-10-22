# taiko-rs

A small, fast osu!taiko client in Rust, built for Linux/Wayland.

- **Native Wayland** via winit (falls back to X11 automatically), **Vulkan** via wgpu.
- **Smooth scrolling**: the game clock free-runs on the monotonic clock and is gently
  steered toward the audio device position, so notes never step or jitter even though audio
  arrives in buffer-sized chunks. Every note position is computed from that clock each frame
  (no accumulated motion), drawn as analytically anti-aliased SDF shapes in a single
  instanced draw call — sub-pixel accurate at any resolution and refresh rate.
- **Low latency**: Mailbox present with one frame in flight (uncapped FPS by default),
  native **PipeWire** output with a 256-frame buffer, hitsounds mixed directly in the audio
  callback, key presses timestamped when they arrive rather than at the next frame.
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
| type, Backspace, Tab | search, edit search, toggle converts |
| Space | skip intro |
| `` ` `` | restart map |
| Esc | back / quit |
| - / = (Shift: ×5) | audio offset −/+ 1 ms |
| F3 / F4 | scroll speed |
| F7 | present mode: mailbox → immediate → vsync |
| F8 | FPS cap: unlimited, 2000, 1000, 500, 240, 144, 60 |
| F10 / F11 | FPS overlay / fullscreen |

Settings (keys, offset, volumes, audio buffer size, present mode) live in
`~/.config/taiko-rs/config.txt`. The results screen shows UR and mean hit error and tells
you which way to move the offset.

## Diagnostics

- `TAIKO_STATS=1` prints frame-time percentiles, clock-vs-wall-time jitter and clock-vs-audio
  drift after each play.
- `TAIKO_SHOTS=/dir:1000,5000` saves PNG screenshots at those gameplay times (plus the select
  and results screens); `TAIKO_EXIT_AFTER=1` quits after the results screenshot.

## Layout

| File | |
| --- | --- |
| `src/beatmap.rs` | `.osu` parser, timing/SV control points, taiko conversion, barlines |
| `src/audio.rs` | decode (symphonia) + resample, cpal/PipeWire output, mixer, hitsound synth |
| `src/clock.rs` | audio-slaved smooth game clock |
| `src/game.rs` | judging, scoring, playfield drawing |
| `src/render.rs` | wgpu SDF shape renderer, text, present mode handling |
| `src/ui.rs` | song select and results screens |
| `src/library.rs` | Songs folder / `.osz` scanning |
