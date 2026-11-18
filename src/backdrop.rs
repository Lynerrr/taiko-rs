//! A picture for a TJA chart that names none (most don't: the games that play them draw a scene
//! of their own behind every song). Drawn here from the chart's genre colour, or one its title
//! picks, so a song isn't played over an empty screen: a dark wash with a glow and rings, like
//! a drum's skin seen from above, placed by the title so songs of a genre differ.

use std::path::PathBuf;

use crate::beatmap::Meta;

pub const WIDTH: u32 = 960;
pub const HEIGHT: u32 = 540;

/// Where the rings' centre can be, across the right half.
const PLACES: u64 = 8;

/// The colour it's drawn in: its first genre's, else one its title picks.
pub fn colour(meta: &Meta) -> u32 {
    let first = meta.genres.trailing_zeros() as usize;
    match taiko_logic::genres::GENRES.get(first) {
        Some(genre) if meta.genres != 0 => genre.colour,
        _ => hue((seed(meta) % 360) as f32),
    }
}

/// What it's told apart by, in place of a file's path: two charts with the same picture share it.
pub fn key(meta: &Meta) -> PathBuf {
    PathBuf::from(format!(
        "<drawn {:06x} {}>",
        colour(meta),
        seed(meta) % PLACES
    ))
}

/// The picture, `WIDTH` by `HEIGHT` RGBA.
pub fn rgba(meta: &Meta) -> Vec<u8> {
    let c = colour(meta);
    let tint = [c >> 16, c >> 8, c].map(|v| (v & 0xff) as f32 / 255.0);
    let dark = [0x0c, 0x0c, 0x12].map(|v| v as f32 / 255.0);
    let (w, h) = (WIDTH as f32, HEIGHT as f32);
    let centre = (0.6 + 0.04 * (seed(meta) % PLACES) as f32, 0.58);
    let mut out = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (u, v) = (x as f32 / w, y as f32 / h);
            let (dx, dy) = ((u - centre.0) * w / h, v - centre.1);
            let d = (dx * dx + dy * dy).sqrt();
            let wash = 0.10 + 0.14 * v;
            let glow = 0.32 * (-(d / 0.45).powi(2)).exp();
            let ring = 1.0 - ((d / 0.085).fract() - 0.5).abs() * 2.0;
            let rings = 0.10 * ring.powi(8) * (-d / 0.9).exp();
            let k = wash + glow + rings;
            // A little noise, under a step of 8 bits: no bands in the gradients.
            let noise = (dither(x, y) - 0.5) / 255.0;
            for i in 0..3 {
                let value = dark[i] + tint[i] * k + noise;
                out.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            out.push(255);
        }
    }
    out
}

/// The picture, ready for the renderer.
pub fn image(meta: &Meta) -> crate::render::BgImage {
    crate::render::BgImage::from_rgba(WIDTH, HEIGHT, &rgba(meta))
}

/// The title's hash (FNV-1a): the same chart, the same picture.
fn seed(meta: &Meta) -> u64 {
    meta.title.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// A bright, not glaring colour of hue `degrees`.
fn hue(degrees: f32) -> u32 {
    let (s, v) = (0.55, 0.95);
    let c = v * s;
    let x = c * (1.0 - ((degrees / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (degrees / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |f: f32| ((f + v - c) * 255.0 + 0.5) as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

/// Noise in 0..1, the same for a pixel every time.
fn dither(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str, genre: &str) -> Meta {
        Meta {
            title: title.into(),
            genres: taiko_logic::genres::of(genre),
            ..Default::default()
        }
    }

    #[test]
    fn a_chart_is_drawn_in_its_genres_colour_else_one_its_title_picks() {
        let anime = meta("A", "アニメ");
        assert_eq!(colour(&anime), 0xff7eb6);
        assert_eq!(
            colour(&meta("B", "Anime")),
            0xff7eb6,
            "the genre's, whatever the title"
        );
        let (x, y) = (meta("Song one", ""), meta("Song two", ""));
        assert_ne!(colour(&x), colour(&y));
        assert_eq!(
            colour(&x),
            colour(&meta("Song one", "")),
            "the same every time"
        );
    }

    #[test]
    fn the_picture_is_dark_lit_where_its_rings_are_and_the_same_every_time() {
        let m = meta("Song", "vocaloid");
        let px = rgba(&m);
        assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        assert_eq!(px, rgba(&m));
        let luma = |x: u32, y: u32| {
            let i = ((y * WIDTH + x) * 4) as usize;
            px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32
        };
        // Darkest at the top left, lit around the rings' centre; dark enough to read over.
        assert!(luma(0, 0) < luma(WIDTH * 3 / 4, HEIGHT * 58 / 100));
        assert!(px.chunks(4).all(|p| p[..3].iter().all(|&v| v < 200)));
        assert_ne!(
            key(&m),
            key(&meta("Song", "anime")),
            "another colour, another picture"
        );
    }
}
