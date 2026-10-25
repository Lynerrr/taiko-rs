//! osu! skins: the note images, hitsounds and combo break sound of a skin from osu!'s `Skins` folder, or from an
//! `.osk` dropped into taiko-rs's own one. Anything a skin doesn't have stays built-in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::audio;
use crate::render::{Atlas, Sprite, SPRITES};

/// taiko-rs's own Skins folder, next to its Songs folder. `.osk` files dropped here are
/// extracted into a folder of the same name.
pub fn own_dir() -> Option<PathBuf> {
    crate::config::own_dir("Skins")
}

/// Folders holding skins: osu!'s next to each Songs folder it has, then taiko-rs's own.
fn skin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = crate::library::song_dirs().iter().filter_map(|s| Some(s.parent()?.join("Skins"))).collect();
    dirs.extend(own_dir());
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

/// Installed skins as (name, folder), sorted by name. New `.osk` files are extracted first.
pub fn list() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let own = own_dir();
    for dir in skin_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            // osu! imports the `.osk` files in its own folder itself.
            let p = if Some(&dir) == own.as_ref() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osk")) {
                match extract_osk(&p) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("{}: {e}", p.display());
                        continue;
                    }
                }
            } else {
                p
            };
            let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
            // The first folder wins when osu! and taiko-rs both have a skin of that name.
            if p.is_dir() && !out.iter().any(|(n, _)| *n == name) {
                out.push((name, p));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// Extracts an `.osk` once into a folder beside it and returns the folder.
fn extract_osk(path: &Path) -> Result<PathBuf, String> {
    let dest = path.with_extension("");
    if dest.is_dir() {
        return Ok(dest);
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    // `extract` rejects entries that would escape the destination (zip-slip).
    zip.extract(&tmp).map_err(|e| e.to_string())?;
    // Some archives wrap the skin in a folder of its own.
    let entries: Vec<PathBuf> = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    let root = match entries.as_slice() {
        [only] if only.is_dir() => only.clone(),
        _ => tmp.clone(),
    };
    std::fs::rename(&root, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(dest)
}

/// A skin's files by lowercase name: skins are made on Windows, where names ignore case.
struct Files(HashMap<String, PathBuf>);

impl Files {
    fn read(dir: &Path) -> Files {
        let map = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Some((e.file_name().to_str()?.to_lowercase(), e.path())))
            .collect();
        Files(map)
    }

    /// The first of `stems` present with an audio extension osu! accepts.
    fn sound(&self, stems: &[&str]) -> Option<&Path> {
        stems.iter().flat_map(|s| ["wav", "ogg", "mp3"].map(|ext| format!("{s}.{ext}"))).find_map(|n| self.0.get(&n)).map(PathBuf::as_path)
    }

    /// The first of `stems` present as a PNG, preferring the `@2x` version. True when it is one.
    fn image(&self, stems: &[&str]) -> Option<(&Path, bool)> {
        stems.iter().find_map(|s| {
            let hd = self.0.get(&format!("{s}@2x.png")).map(|p| (p.as_path(), true));
            hd.or_else(|| self.0.get(&format!("{s}.png")).map(|p| (p.as_path(), false)))
        })
    }
}

/// The images osu!taiko uses: circles (tinted red or blue) and the overlays drawn on top,
/// drumroll bodies and ends (tinted yellow), and the spinner on its way in, its circle and its
/// closing ring.
const NOTE_IMAGES: [(Sprite, &[&str]); SPRITES] = [
    (Sprite::HitCircle, &["taikohitcircle"]),
    (Sprite::HitCircleOverlay, &["taikohitcircleoverlay", "taikohitcircleoverlay-0"]),
    (Sprite::BigCircle, &["taikobigcircle"]),
    (Sprite::BigCircleOverlay, &["taikobigcircleoverlay", "taikobigcircleoverlay-0"]),
    (Sprite::RollMiddle, &["taiko-roll-middle"]),
    (Sprite::RollEnd, &["taiko-roll-end"]),
    (Sprite::SpinnerWarning, &["spinner-warning"]),
    (Sprite::SpinnerCircle, &["spinner-circle"]),
    (Sprite::SpinnerApproach, &["spinner-approachcircle"]),
];

/// Packs the skin's note images into one atlas. Images are premultiplied (so filtering doesn't
/// darken their edges) and capped in size: nothing is drawn bigger than that. Rows wrap at
/// 2048 px, the smallest texture size every GPU takes.
fn note_atlas(files: &Files) -> Atlas {
    const ROW: u32 = 2048;
    let mut images = Vec::new();
    for (sprite, stems) in NOTE_IMAGES {
        let Some((path, hd)) = files.image(stems) else { continue };
        let mut img = match image::open(path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("skin: {}: {e}", path.display());
                continue;
            }
        };
        // osu! draws a 128 px (SD) circle image at the note's size.
        let sd = if hd { 2.0 } else { 1.0 };
        let size = [img.width() as f32 / sd / 128.0, img.height() as f32 / sd / 128.0];
        for px in img.pixels_mut() {
            let a = px[3] as u32;
            for c in &mut px.0[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
        // The spinner's circles are drawn bigger than notes.
        let max = if matches!(sprite, Sprite::SpinnerCircle | Sprite::SpinnerApproach) { 512 } else { 256 };
        let scale = (max as f32 / img.width().max(img.height()) as f32).min(1.0);
        if scale < 1.0 {
            let (w, h) = (((img.width() as f32 * scale).round() as u32).max(1), ((img.height() as f32 * scale).round() as u32).max(1));
            img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
        }
        images.push((sprite, img, size));
    }
    // The circle is what makes a skinned note; overlays alone stay unused.
    let has = |s: Sprite| images.iter().any(|(i, _, _)| *i as usize == s as usize);
    if !has(Sprite::HitCircle) && !has(Sprite::BigCircle) {
        images.retain(|(i, _, _)| !matches!(i, Sprite::HitCircleOverlay | Sprite::BigCircleOverlay));
    }
    if images.is_empty() {
        return Atlas::default();
    }
    // Each image sits in a cell one pixel bigger all round, filled with copies of its edge, so
    // filtering at the edge (or across a stretched roll body) never picks up a neighbour.
    let mut cells = Vec::new();
    let (mut x, mut y, mut row_h, mut width) = (0, 0, 0, 0);
    for (_, img, _) in &images {
        let (cw, ch) = (img.width() + 2, img.height() + 2);
        if x > 0 && x + cw > ROW {
            (x, y, row_h) = (0, y + row_h, 0);
        }
        cells.push((x, y));
        (x, row_h, width) = (x + cw, row_h.max(ch), width.max(x + cw));
    }
    let height = y + row_h;
    let mut atlas = Atlas { width, height, rgba: vec![0; (width * height * 4) as usize], ..Default::default() };
    for ((sprite, img, size), (cx, cy)) in images.into_iter().zip(cells) {
        let (iw, ih) = (img.width(), img.height());
        for py in 0..ih + 2 {
            for px in 0..iw + 2 {
                let p = img.get_pixel(px.saturating_sub(1).min(iw - 1), py.saturating_sub(1).min(ih - 1));
                let at = (((cy + py) * width + cx + px) * 4) as usize;
                atlas.rgba[at..at + 4].copy_from_slice(&p.0);
            }
        }
        let uv = [(cx + 1) as f32 / width as f32, (cy + 1) as f32 / height as f32, iw as f32 / width as f32, ih as f32 / height as f32];
        atlas.sprites[sprite as usize] = Some((uv, size));
    }
    atlas
}

pub struct Skin {
    pub hitsounds: audio::Custom,
    pub notes: Atlas,
}

/// Loads the skin called `name` (built-in for "" or one that isn't installed). `rate`: the
/// audio device's sample rate, or None to skip the sounds.
pub fn load(name: &str, rate: Option<u32>) -> Skin {
    let dir = (!name.is_empty()).then(|| list().into_iter().find(|(n, _)| n == name)).flatten().map(|(_, d)| d);
    let Some(dir) = dir else { return Skin { hitsounds: audio::Custom::default(), notes: Atlas::default() } };
    let files = Files::read(&dir);
    let hitsounds = match rate {
        Some(rate) => audio::Custom {
            // osu!taiko plays the clap for rim hits; some skins only ship the whistle.
            don: files.sound(&["taiko-normal-hitnormal"]).and_then(|p| decode_sound(p, rate)),
            kat: files.sound(&["taiko-normal-hitclap", "taiko-normal-hitwhistle"]).and_then(|p| decode_sound(p, rate)),
            finish: files.sound(&["taiko-normal-hitfinish"]).and_then(|p| decode_sound(p, rate)),
            combobreak: files.sound(&["combobreak"]).and_then(|p| decode_sound(p, rate)),
        },
        None => audio::Custom::default(),
    };
    Skin { hitsounds, notes: note_atlas(&files) }
}

/// A hitsound as mono samples. Skins mute a sound with a tiny empty file; one that fails to
/// decode keeps the built-in sound.
fn decode_sound(path: &Path, rate: u32) -> Option<Vec<f32>> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() < 128) {
        return Some(Vec::new());
    }
    match audio::decode(path, rate) {
        Ok(track) => Some(audio::mono(&track)),
        Err(e) => {
            eprintln!("skin: {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    #[test]
    fn osk_wrapped_in_a_folder_extracts_flat_and_names_ignore_case() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-skin-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let osk = dir.join("Test.osk");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&osk).unwrap());
        zip.start_file("Test/Taiko-Normal-HitNormal.WAV", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();

        let skin = super::extract_osk(&osk).unwrap();
        assert_eq!(skin, dir.join("Test"));
        assert!(super::Files::read(&skin).sound(&["taiko-normal-hitnormal"]).is_some());
        // Extracted once: a second call reuses the folder.
        assert_eq!(super::extract_osk(&osk).unwrap(), skin);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atlas_packs_rolls_and_spinners_with_their_edges_copied() {
        use crate::render::Sprite;
        let dir = std::env::temp_dir().join(format!("taiko-rs-atlas-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, w: u32, h: u32, px: [u8; 4]| image::RgbaImage::from_pixel(w, h, image::Rgba(px)).save(dir.join(name)).unwrap();
        // Overlays without a circle under them are left out.
        save("taikohitcircleoverlay.png", 128, 128, [255, 255, 255, 128]);
        save("spinner-warning.png", 128, 128, [255, 0, 0, 255]);
        let atlas = super::note_atlas(&super::Files::read(&dir));
        assert!(atlas.sprites[Sprite::HitCircleOverlay as usize].is_none() && atlas.sprites[Sprite::SpinnerWarning as usize].is_some());

        // A 1 px wide roll body, and a full skin too wide for one 2048 px row.
        save("taiko-roll-middle.png", 1, 128, [255, 255, 255, 255]);
        save("Spinner-Circle@2x.png", 1024, 1024, [0, 255, 0, 255]);
        save("spinner-approachcircle.png", 600, 600, [0, 0, 255, 255]);
        for name in ["taikohitcircle", "taikobigcircle", "taikobigcircleoverlay", "taiko-roll-end"] {
            save(&format!("{name}@2x.png"), 512, 512, [200, 200, 200, 255]);
        }
        let atlas = super::note_atlas(&super::Files::read(&dir));
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(atlas.width <= 2048 && atlas.height > 514, "{}x{}", atlas.width, atlas.height);
        // osu! sizes: 128 SD px is the radius it's drawn for, @2x counts half.
        assert_eq!(atlas.sprites[Sprite::SpinnerCircle as usize].unwrap().1, [4.0, 4.0]);
        assert_eq!(atlas.sprites[Sprite::RollMiddle as usize].unwrap().1, [1.0 / 128.0, 1.0]);
        assert!(atlas.sprites.iter().all(Option::is_some));
        // Around each image, its edge is repeated: the roll body is white on both sides.
        let (uv, _) = atlas.sprites[Sprite::RollMiddle as usize].unwrap();
        let (x, y) = ((uv[0] * atlas.width as f32).round() as u32, (uv[1] * atlas.height as f32).round() as u32);
        for px in [x - 1, x, x + 1] {
            let at = ((y + 64) * atlas.width + px) as usize * 4;
            assert_eq!(&atlas.rgba[at..at + 4], &[255, 255, 255, 255], "x = {px}");
        }
        // Every image is inside the atlas and none overlap.
        let rects: Vec<[f32; 4]> = atlas.sprites.iter().flatten().map(|(uv, _)| *uv).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a[0] >= 0.0 && a[1] >= 0.0 && a[0] + a[2] <= 1.0 && a[1] + a[3] <= 1.0);
            for b in &rects[i + 1..] {
                assert!(a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1]);
            }
        }
    }
}
