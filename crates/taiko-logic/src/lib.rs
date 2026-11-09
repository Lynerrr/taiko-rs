//! The game's rules, apart from the client that shows and plays them: the autoplay's hands, the
//! calibration's math, the clock a play keeps, the mods and their setups, and Defense's rules.
//!
//! The client (`taiko-rs`) re-exports each module under its old name, so `crate::tact` and the
//! rest still mean the same thing there. Nothing here draws, plays sound or opens a socket, and
//! the crate's dependencies keep it that way.

// First, so `tr!` is there for every module after it.
#[macro_use]
pub mod lang;
pub mod autoplay;
pub mod beatmap;
pub mod calibrate;
pub mod clock;
pub mod defense;
pub mod difficulty;
pub mod font;
pub mod genres;
pub mod mods;
pub mod multiverse;
pub mod od_override;
pub mod pp;
pub mod shapeshift;
pub mod sim;
pub mod stars;
pub mod tact;
pub mod tko;

#[cfg(test)]
mod tests {
    /// The crates that would let the rules reach for a window, the GPU, the sound card or the
    /// network: the client's engine has them, the logic must not.
    const ENGINE: [&str; 9] = [
        "wgpu",
        "winit",
        "cpal",
        "ureq",
        "symphonia",
        "image",
        "rustls",
        "arboard",
        "taiko-rs",
    ];

    #[test]
    fn the_logic_needs_no_window_gpu_sound_or_network() {
        let manifest = include_str!("../Cargo.toml");
        for line in manifest.lines().map(str::trim) {
            let name = line.split(['=', ' ', '.']).next().unwrap_or("");
            assert!(
                !ENGINE.contains(&name),
                "taiko-logic depends on {name}: {line}"
            );
        }
    }
}
