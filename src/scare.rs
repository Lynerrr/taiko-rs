//! Overtime's own scare: a picture (an animated GIF too) and a sound the player puts in the
//! Overtime folder, shown and played in place of the built-in bear and its scream. The first
//! picture and the first sound in the folder, by name, are the ones used.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// At most this many frames of a GIF are kept (each takes one of the map pictures' squares):
/// longer ones are thinned out evenly, each kept frame lasting as long as those it stands for.
pub const MAX_FRAMES: usize = 8;
/// The pictures' names among the map pictures (`pictures::Pictures::provide`): this, then the
/// frame's number.
pub const PREFIX: &str = "\u{1}overtime:";
/// Longer sounds are cut here: a scare, not a song.
const MAX_SOUND_S: f64 = 5.0;
/// Larger files are left alone.
const MAX_BYTES: u64 = 24 * 1024 * 1024;

/// The folder the player puts them in, beside the Songs and Skins ones.
pub fn dir() -> Option<PathBuf> {
    crate::config::own_dir("Overtime")
}

/// What is in the folder: each file's name, size and when it changed, so a change is noticed.
pub type Signature = Vec<(String, u64, Option<SystemTime>)>;

pub fn signature(dir: &Path) -> Signature {
    let mut out: Signature = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some((
                e.file_name().to_str()?.to_string(),
                meta.len(),
                meta.modified().ok(),
            ))
        })
        .collect();
    out.sort();
    out
}

fn is_sound(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".ogg", ".mp3", ".wav", ".flac"]
        .iter()
        .any(|x| l.ends_with(x))
}

/// The scare a folder holds.
#[derive(Default)]
pub struct Custom {
    /// Each frame squeezed into a picture square (`pictures::cell_of`), its width over its
    /// height, and how long it shows (ms).
    pub frames: Vec<(Vec<u8>, f32, f64)>,
    /// Mono samples at the rate asked for.
    pub sound: Option<Vec<f32>>,
}

/// Reads the first picture and the first sound in `dir` (by name); what can't be read is left
/// out, and said why on stderr.
pub fn load(dir: &Path, rate: u32) -> Custom {
    let sig = signature(dir);
    let first = |is: fn(&str) -> bool| {
        sig.iter()
            .filter(|(name, size, _)| is(name) && *size <= MAX_BYTES)
            .min_by_key(|(name, ..)| name.to_lowercase())
            .map(|(name, ..)| dir.join(name))
    };
    let frames = first(crate::pictures::is_picture)
        .map(|path| {
            frames_of(&path).unwrap_or_else(|e| {
                eprintln!("overtime picture {}: {e}", path.display());
                Vec::new()
            })
        })
        .unwrap_or_default();
    let sound = first(is_sound).and_then(|path| match crate::audio::decode(&path, rate) {
        Ok(track) => {
            let mut mono = crate::audio::mono(&track);
            mono.truncate((MAX_SOUND_S * rate as f64) as usize);
            Some(mono)
        }
        Err(e) => {
            eprintln!("overtime sound {}: {e}", path.display());
            None
        }
    });
    Custom { frames, sound }
}

/// A picture file's frames: one for a still picture, a GIF's (at most `MAX_FRAMES`) for a moving
/// one.
fn frames_of(path: &Path) -> Result<Vec<(Vec<u8>, f32, f64)>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let gif = path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("gif"));
    if gif {
        use image::AnimationDecoder;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes))
            .map_err(|e| e.to_string())?;
        let frames = decoder
            .into_frames()
            .collect_frames()
            .map_err(|e| e.to_string())?;
        let delays: Vec<f64> = frames
            .iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                // Browsers treat the tiny delays of many GIFs as 100 ms; so does this.
                let ms = n as f64 / d.max(1) as f64;
                if ms < 20.0 { 100.0 } else { ms }
            })
            .collect();
        let picked = thin(frames.len(), MAX_FRAMES);
        if picked.is_empty() {
            return Err("no frames".into());
        }
        return Ok(picked
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let until = picked.get(k + 1).copied().unwrap_or(frames.len());
                let image = image::DynamicImage::ImageRgba8(frames[i].buffer().clone());
                let (rgba, aspect) = crate::pictures::cell_of(&image);
                (rgba, aspect, delays[i..until].iter().sum())
            })
            .collect());
    }
    let (rgba, aspect) = crate::pictures::cell(&bytes)?;
    Ok(vec![(rgba, aspect, f64::INFINITY)])
}

/// Which of `n` frames to keep, at most `max`, spread evenly from the first.
fn thin(n: usize, max: usize) -> Vec<usize> {
    if n <= max {
        (0..n).collect()
    } else {
        (0..max).map(|k| k * n / max).collect()
    }
}

/// The frame to show `ms` into the scare, of frames lasting `delays` (looping).
pub fn frame_at(delays: &[f64], ms: f64) -> usize {
    let total: f64 = delays.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return 0;
    }
    let mut left = ms.rem_euclid(total);
    for (i, &d) in delays.iter().enumerate() {
        if left < d {
            return i;
        }
        left -= d;
    }
    delays.len() - 1
}

/// Frame `k`'s name among the map pictures.
pub fn frame_name(k: usize) -> String {
    format!("{PREFIX}{k}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_gifs_are_thinned_evenly_and_frames_follow_their_delays() {
        assert_eq!(thin(3, 8), vec![0, 1, 2]);
        assert_eq!(thin(16, 8), vec![0, 2, 4, 6, 8, 10, 12, 14]);
        assert_eq!(thin(10, 4), vec![0, 2, 5, 7]);
        let delays = [100.0, 50.0, 100.0];
        assert_eq!(frame_at(&delays, 0.0), 0);
        assert_eq!(frame_at(&delays, 120.0), 1);
        assert_eq!(frame_at(&delays, 160.0), 2);
        assert_eq!(frame_at(&delays, 260.0), 0, "it loops");
        assert_eq!(frame_at(&[f64::INFINITY], 5000.0), 0);
    }

    #[test]
    fn the_first_picture_and_sound_in_the_folder_are_used() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-scare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A two-frame GIF, red then blue, and a still PNG later by name.
        let mut gif = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
            for colour in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let buf = image::RgbaImage::from_pixel(4, 2, image::Rgba(colour));
                let delay = image::Delay::from_numer_denom_ms(70, 1);
                enc.encode_frame(image::Frame::from_parts(buf, 0, 0, delay))
                    .unwrap();
            }
        }
        std::fs::write(dir.join("a.gif"), &gif).unwrap();
        let png = {
            let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        std::fs::write(dir.join("b.png"), &png).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a picture").unwrap();
        let got = load(&dir, 48_000);
        assert_eq!(got.frames.len(), 2);
        assert!((got.frames[0].1 - 2.0).abs() < 1e-3, "4 wide, 2 tall");
        assert_eq!(got.frames[0].2, 70.0);
        // Red first: premultiplied, opaque.
        assert_eq!(&got.frames[0].0[..4], &[255, 0, 0, 255]);
        assert_eq!(&got.frames[1].0[..4], &[0, 0, 255, 255]);
        assert!(got.sound.is_none());
        let before = signature(&dir);
        std::fs::remove_file(dir.join("a.gif")).unwrap();
        assert_ne!(signature(&dir), before);
        let got = load(&dir, 48_000);
        assert_eq!(got.frames.len(), 1);
        assert_eq!(got.frames[0].2, f64::INFINITY);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
