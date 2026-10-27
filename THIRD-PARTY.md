# Third-party notices

taiko.rs itself is under the MIT license (see `LICENSE`).

## osu!

taiko.rs is an independent project. It is not affiliated with, or endorsed by, ppy Pty Ltd;
"osu!" is a trademark of ppy Pty Ltd.

- It plays the beatmaps and skins the player already has (their osu! `Songs` and `Skins`
  folders, or files they open). No beatmaps, skins or osu! assets are bundled with it.
- Star ratings and pp are worked out by [rosu-pp](https://github.com/MaxOhn/rosu-pp) (MIT), a
  port of osu!lazer's difficulty and performance calculation. Beatmap conversion and hit windows
  follow osu!lazer's behaviour; [osu!lazer](https://github.com/ppy/osu) is MIT-licensed,
  Copyright (c) ppy Pty Ltd.
- Beatmap downloads, previews and the website's beatmap covers come from osu!'s servers and
  beatmap mirrors, as the player or the page asks for them.

## Bundled assets

- The built-in hitsounds (`assets/hitsounds/`: don, kat, finisher, combo break) are synthesized
  from scratch by `assets/hitsounds/synth.py`; a skin's own hitsounds replace them.
- The 5x7 LCD font is drawn in `src/font.rs`; the website's font (`server/site/taiko5x7.woff2`)
  is built from it by `server/site/build_font.py`.

## Rust libraries

The game and the server are built from crates.io libraries under their own licenses, all
permissive: MIT and/or Apache-2.0 for most, and some under BSD-2-Clause, BSD-3-Clause, ISC, Zlib,
Unlicense or 0BSD. The [Symphonia](https://github.com/pdeljanov/Symphonia) audio decoders are
under MPL-2.0 and are used unmodified. `cargo metadata` (or `cargo about`) lists every crate with
its license and authors.

Notable ones: [wgpu](https://github.com/gfx-rs/wgpu) and [winit](https://github.com/rust-windowing/winit)
(graphics and windows), [cpal](https://github.com/RustAudio/cpal) (audio output),
[Symphonia](https://github.com/pdeljanov/Symphonia) (audio decoding),
[rosu-pp](https://github.com/MaxOhn/rosu-pp) (stars and pp), [image](https://github.com/image-rs/image),
[zip](https://github.com/zip-rs/zip2), [ureq](https://github.com/algesten/ureq),
[serde](https://github.com/serde-rs/serde), and on the server [argon2](https://github.com/RustCrypto/password-hashes).
