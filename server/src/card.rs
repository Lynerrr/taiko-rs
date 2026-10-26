//! A score's card: the picture under the Discord bot's plays and in a score link's preview.
//! Grade, pp, accuracy, the hit counts and the mods, big enough to read at a glance, in the
//! game's own 5x7 font (`src/font.rs`) and the website's colours. Drawn at even font scales so
//! Discord halving it keeps every font pixel square.

use crate::online::OnlineScore;

pub const W: usize = 1200;
pub const H: usize = 480;

const BG: u32 = 0x0b0b10;
const PANEL: u32 = 0x15151d;
const LINE: u32 = 0x2a2a38;
const WHITE: u32 = 0xffffff;
const DIM: u32 = 0x8a8aa0;
const RED: u32 = 0xEB452C;
const KAT: u32 = 0x4fb4ff;
const GOLD: u32 = 0xFFD34D;
const GREEN: u32 = 0x86E05A;
const PURPLE: u32 = 0xc77dff;

/// A grade's colour, as the website gives it.
pub fn grade_colour(g: &str) -> u32 {
    match g {
        "SS" | "S" => GOLD,
        "A" => GREEN,
        "B" => KAT,
        "C" => PURPLE,
        _ => RED,
    }
}

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Canvas {
        let mut c = Canvas { px: vec![0; W * H * 3] };
        c.fill(0, 0, W, H, BG, 1.0);
        c
    }

    /// `colour` over the rectangle, `alpha` of it.
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, colour: u32, alpha: f32) {
        let rgb = [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8];
        for row in y.min(H)..(y + h).min(H) {
            for col in x.min(W)..(x + w).min(W) {
                let i = (row * W + col) * 3;
                for (px, &c) in self.px[i..i + 3].iter_mut().zip(&rgb) {
                    let old = *px as f32;
                    *px = (old + (c as f32 - old) * alpha).round() as u8;
                }
            }
        }
    }

    /// A box with a `border`-wide outline.
    fn frame(&mut self, x: usize, y: usize, w: usize, h: usize, border: usize, colour: u32) {
        self.fill(x, y, w, border, colour, 1.0);
        self.fill(x, y + h - border, w, border, colour, 1.0);
        self.fill(x, y, border, h, colour, 1.0);
        self.fill(x + w - border, y, border, h, colour, 1.0);
    }

    /// `text` with its top left at (x, y), font pixels `scale` wide; returns where it ends.
    fn text(&mut self, x: usize, y: usize, scale: usize, colour: u32, text: &str) -> usize {
        let mut cx = x;
        for ch in text.chars() {
            for (col, bits) in crate::font::glyph(ch).iter().enumerate() {
                for row in 0..7 {
                    if bits & (1 << row) != 0 {
                        self.fill(cx + col * scale, y + row * scale, scale, scale, colour, 1.0);
                    }
                }
            }
            cx += 6 * scale;
        }
        cx.saturating_sub(scale)
    }
}

/// How wide `text` is at `scale`.
fn width(text: &str, scale: usize) -> usize {
    (text.chars().count() * 6).saturating_sub(1) * scale
}

/// `text` cut to fit `max` pixels at `scale`, with "..." where it was cut.
fn fit(text: &str, scale: usize, max: usize) -> String {
    if width(text, scale) <= max {
        return text.to_string();
    }
    let room = (max / scale + 1) / 6;
    let kept: String = text.chars().take(room.saturating_sub(3)).collect();
    format!("{}...", kept.trim_end())
}

/// A colour from its hue (degrees), saturation and lightness (0..1).
fn hsl(h: f64, s: f64, l: f64) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The card of score `s` as a PNG.
pub fn png(s: &OnlineScore) -> Vec<u8> {
    encode(&draw(s).px)
}

fn draw(s: &OnlineScore) -> Canvas {
    let mut c = Canvas::new();
    // The map's band: its own hue (as the website's covers without a picture), with scanlines.
    let hue = (s.map_hash % 360) as f64;
    let band = 150;
    for x in (0..W).step_by(8) {
        let t = x as f64 / W as f64;
        c.fill(x, 0, 8, band, hsl(hue + 60.0 * t, 0.40 - 0.05 * t, 0.22 - 0.10 * t), 1.0);
    }
    for y in (0..band).step_by(4) {
        c.fill(0, y, W, 2, 0x000000, 0.2);
    }
    c.fill(0, band, W, 4, RED, 1.0);
    let (song, version) = match s.map.rfind(" [") {
        Some(i) if s.map.ends_with(']') => (&s.map[..i], &s.map[i + 2..s.map.len() - 1]),
        _ => (s.map.as_str(), ""),
    };
    c.text(40, 34, 4, WHITE, &fit(song, 4, W - 80));
    let mut x = 40;
    if !version.is_empty() {
        x = c.text(x, 92, 3, GOLD, &fit(&format!("[{version}]"), 3, W - 280)) + 24;
    }
    if let Some(stars) = s.stars {
        let label = format!("{stars:.2}*");
        let colour = star_colour(stars);
        let ink = if stars >= 6.5 { GOLD } else { 0x000000 };
        c.fill(x, 84, width(&label, 3) + 24, 37, colour, 1.0);
        c.text(x + 12, 92, 3, ink, &label);
    }

    // The grade, big, on the left (SS with the website's red and blue edges).
    let grade = s.grade();
    let colour = grade_colour(grade);
    let scale = if grade.len() > 1 { 14 } else { 20 };
    let gy = 190 + (140 - 7 * scale) / 2;
    let gx = 40 + (220 - width(grade, scale)) / 2;
    if grade == "SS" {
        c.text(gx - 4, gy, scale, RED, grade);
        c.text(gx + 4, gy, scale, KAT, grade);
    }
    c.text(gx, gy, scale, colour, grade);

    // pp and accuracy (sharing a baseline), then a full combo or how many misses.
    let left = 300;
    let mut x = match s.pp {
        Some(pp) => {
            let end = c.text(left, 190, 12, RED, &format!("{pp:.0}"));
            c.text(end + 12, 232, 6, RED, "pp")
        }
        None => c.text(left, 218, 8, DIM, "no pp"),
    } + 48;
    x = c.text(x, 218, 8, WHITE, &format!("{:.2}%", s.accuracy * 100.0)) + 32;
    let (label, colour) = if s.miss == 0 { ("FC".to_string(), GREEN) } else { (format!("{} MISS", s.miss), RED) };
    let w = width(&label, 4) + 48;
    if x + w < W {
        c.fill(x, 222, w, 48, colour, 0.18);
        c.frame(x, 222, w, 48, 4, colour);
        c.text(x + 24, 232, 4, colour, &label);
    }

    // The hit counts, the combo and the score.
    let stats = [
        ("GREAT", s.great.to_string(), KAT),
        ("OK", s.ok.to_string(), GREEN),
        ("MISS", s.miss.to_string(), if s.miss == 0 { DIM } else { RED }),
        ("COMBO", format!("{}x", thousands(s.max_combo as u64)), WHITE),
        ("SCORE", thousands(s.score), WHITE),
    ];
    let mut x = left;
    for (label, value, colour) in stats {
        c.text(x, 312, 2, DIM, label);
        let end = c.text(x, 334, 5, colour, &value);
        x = end.max(x + width(label, 2)) + 44;
    }

    // Mods and the rate, as the website's chips.
    c.fill(0, 396, W, H - 396, PANEL, 1.0);
    c.fill(0, 396, W, 2, LINE, 1.0);
    let mut x = 40;
    let chip = |c: &mut Canvas, x: &mut usize, label: &str, colour: u32| {
        let w = width(label, 4) + 28;
        c.fill(*x, 412, w, 48, colour, 0.14);
        c.frame(*x, 412, w, 48, 4, colour);
        c.text(*x + 14, 422, 4, colour, label);
        *x += w + 12;
    };
    for m in &s.mods {
        let colour = crate::pages::mod_info(m).and_then(|(hex, _)| u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()).unwrap_or(DIM);
        chip(&mut c, &mut x, m, colour);
    }
    if (s.rate - 1.0).abs() > 1e-6 {
        let colour = if s.rate > 1.0 { GOLD } else { KAT };
        chip(&mut c, &mut x, &format!("{:.2}x", s.rate), colour);
    }
    if s.mods.is_empty() && (s.rate - 1.0).abs() <= 1e-6 {
        c.text(40, 422, 4, DIM, "NO MODS");
    }
    // Who played it, on the right.
    let who = s.player.as_str();
    let end = W - 40;
    let brand = "taiko.rs";
    let bx = end - width(brand, 3);
    c.text(bx, 428, 3, RED, brand);
    c.text(bx.saturating_sub(width(who, 4) + 36), 422, 4, WHITE, who);
    c
}

/// osu!'s star rating colours (as the website's).
fn star_colour(s: f64) -> u32 {
    let [r, g, b] = crate::pages::star_colour(s);
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// An RGB image as a PNG (each row filtered by its left neighbour, then deflated).
fn encode(px: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((W * 3 + 1) * H);
    for row in px.chunks(W * 3) {
        raw.push(1);
        raw.extend(row.iter().enumerate().map(|(i, &v)| if i < 3 { v } else { v.wrapping_sub(row[i - 3]) }));
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend((W as u32).to_be_bytes());
    ihdr.extend((H as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(&out[start..]).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_checksums_are_right() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn long_text_is_cut_to_fit() {
        let long = "x".repeat(200);
        let cut = fit(&long, 4, 400);
        assert!(width(&cut, 4) <= 400 && cut.ends_with("..."), "{cut}");
        assert_eq!(fit("short", 4, 400), "short");
    }

    #[test]
    fn a_card_is_a_png_of_its_size() {
        let s = OnlineScore {
            player: "Lynerrr".into(),
            map: "Camellia - Exit This Earth's Atomosphere [Inner Oni]".into(),
            map_hash: 77,
            score: 1_234_567,
            accuracy: 0.9821,
            max_combo: 812,
            great: 900,
            ok: 30,
            miss: 3,
            rate: 1.25,
            mods: vec!["HD".into(), "HR".into()],
            pp: Some(312.4),
            stars: Some(6.1),
            ..Default::default()
        };
        let png = png(&s);
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(&png[16..24], &[0, 0, 0x04, 0xb0, 0, 0, 0x01, 0xe0]);
        assert!(png.len() < 100_000, "{} bytes", png.len());
    }
}
