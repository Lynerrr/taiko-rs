//! osu! skins: the hitsounds of a skin from osu!'s `Skins` folder, or from an `.osk` dropped into
//! taiko-rs's own one. Anything a skin doesn't have keeps the built-in sound.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::audio;

/// taiko-rs's own Skins folder, next to its Songs folder. `.osk` files dropped here are
/// extracted into a folder of the same name.
pub fn own_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let dir = crate::config::user_dir(false);
    #[cfg(not(windows))]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("taiko-rs"));
    Some(dir?.join("Skins"))
}

/// Folders holding skins: osu!'s next to each Songs folder it has, then taiko-rs's own.
fn skin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = crate::library::default_song_dirs().iter().filter_map(|s| Some(s.parent()?.join("Skins"))).collect();
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
}

/// Hitsounds as mono samples at the device rate; `None` keeps the built-in one.
#[derive(Default)]
pub struct Hitsounds {
    pub don: Option<Vec<f32>>,
    pub kat: Option<Vec<f32>>,
    /// Played on top of don / kat for big notes.
    pub finish: Option<Vec<f32>>,
}

pub struct Skin {
    pub hitsounds: Hitsounds,
}

/// Loads the skin called `name` (built-in for "" or one that isn't installed). `rate`: the
/// audio device's sample rate, or None to skip the sounds.
pub fn load(name: &str, rate: Option<u32>) -> Skin {
    let dir = (!name.is_empty()).then(|| list().into_iter().find(|(n, _)| n == name)).flatten().map(|(_, d)| d);
    let Some(dir) = dir else { return Skin { hitsounds: Hitsounds::default() } };
    let files = Files::read(&dir);
    let hitsounds = match rate {
        Some(rate) => Hitsounds {
            // osu!taiko plays the clap for rim hits; some skins only ship the whistle.
            don: files.sound(&["taiko-normal-hitnormal"]).and_then(|p| decode_sound(p, rate)),
            kat: files.sound(&["taiko-normal-hitclap", "taiko-normal-hitwhistle"]).and_then(|p| decode_sound(p, rate)),
            finish: files.sound(&["taiko-normal-hitfinish"]).and_then(|p| decode_sound(p, rate)),
        },
        None => Hitsounds::default(),
    };
    Skin { hitsounds }
}

/// A hitsound as mono samples. Skins mute a sound with a tiny empty file; one that fails to
/// decode keeps the built-in sound.
fn decode_sound(path: &Path, rate: u32) -> Option<Vec<f32>> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() < 128) {
        return Some(Vec::new());
    }
    match audio::decode(path, rate) {
        Ok(track) => Some(track.samples.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect()),
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
}
