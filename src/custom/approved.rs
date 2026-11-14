//! Approved difficulties of a custom set: plays on them give pp, so the server takes an update
//! of the set only when it brings them back byte for byte. The upload window asks the website
//! which ones are approved, compares them with the folder, and can put them back: a changed one
//! is first kept as a new difficulty of its own, so no work is lost.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{MAX_OSZ, download_url};
use crate::online::WEBSITE;

/// A difficulty the website has approved: its name and the hash of its file.
#[derive(Clone, Debug, PartialEq)]
pub struct Approved {
    pub version: String,
    pub hash: u64,
}

/// How the folder has an approved difficulty now.
#[derive(Clone, Debug, PartialEq)]
pub enum Kept {
    /// The very file that was approved.
    Same,
    /// A file of that difficulty with other contents.
    Changed(PathBuf),
    /// No file of it.
    Missing,
}

/// The approved difficulties of website set `id`; empty when none is.
pub fn fetch(id: u32) -> Result<Vec<Approved>, String> {
    crate::offline::internet()?;
    let mut reply = ureq::get(format!("{WEBSITE}/api/c/{id}"))
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    let text = reply
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse(&text).ok_or_else(|| "the website's answer made no sense".to_string())
}

/// The approved difficulties in the website's answer about one set.
pub fn parse(text: &str) -> Option<Vec<Approved>> {
    let set: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut out = Vec::new();
    for diff in set.get("diffs")?.as_array()? {
        if diff.get("approved").and_then(serde_json::Value::as_bool) != Some(true) {
            continue;
        }
        out.push(Approved {
            version: diff.get("version")?.as_str()?.to_string(),
            hash: u64::from_str_radix(diff.get("hash")?.as_str()?, 16).ok()?,
        });
    }
    Some(out)
}

fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("tko"))
}

/// The map files of the folder: path, hash and difficulty name.
fn maps(folder: &Path) -> Vec<(PathBuf, u64, String)> {
    let Ok(read) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<_> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_map(p))
        .filter_map(|p| {
            let bytes = std::fs::read(&p).ok()?;
            let version = crate::beatmap::read_meta_file(&p)?.version;
            Some((p, crate::net::map_hash(&bytes), version))
        })
        .collect();
    out.sort();
    out
}

/// How the folder keeps each approved difficulty.
pub fn compare(folder: &Path, approved: &[Approved]) -> Vec<(Approved, Kept)> {
    let maps = maps(folder);
    approved
        .iter()
        .map(|a| {
            let kept = if maps.iter().any(|(_, hash, _)| *hash == a.hash) {
                Kept::Same
            } else if let Some((path, ..)) = maps.iter().find(|(.., v)| *v == a.version) {
                Kept::Changed(path.clone())
            } else {
                Kept::Missing
            };
            (a.clone(), kept)
        })
        .collect()
}

/// The archive of website set `id`, as the website gives it.
pub fn download(id: u32) -> Result<Vec<u8>, String> {
    crate::offline::internet()?;
    let mut reply = ureq::get(download_url(id))
        .config()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    reply
        .body_mut()
        .with_config()
        .limit(MAX_OSZ as u64 + (1 << 20))
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// The map file of `osz` whose hash is `hash`: its name in the archive and its bytes.
fn map_in(osz: &[u8], hash: u64) -> Result<(String, Vec<u8>), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(osz)).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = file.enclosed_name() else {
            continue;
        };
        if !file.is_file() || !is_map(&path) {
            continue;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if crate::net::map_hash(&bytes) == hash {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("approved.osu")
                .to_string();
            return Ok((name, bytes));
        }
    }
    Err("the website's copy of the set doesn't have it".into())
}

/// Puts the changed and missing approved difficulties back from the website's archive `osz`.
/// A changed file is first kept as a new difficulty by `keep` (it returns the new file), so the
/// changes stay. Returns `(old file, new file)` for each change kept.
pub fn restore(
    folder: &Path,
    list: &[(Approved, Kept)],
    osz: &[u8],
    keep: impl Fn(&Path) -> Result<PathBuf, String>,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut moved = Vec::new();
    for (approved, kept) in list {
        let (name, bytes) = match kept {
            Kept::Same => continue,
            _ => map_in(osz, approved.hash)?,
        };
        match kept {
            Kept::Changed(path) => {
                let new = keep(path)?;
                std::fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                moved.push((path.clone(), new));
            }
            _ => {
                let mut path = folder.join(&name);
                let mut n = 1;
                while path.exists() {
                    path = folder.join(format!("{n} {name}"));
                    n += 1;
                }
                std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("taiko-approved-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn map(version: &str, notes: usize) -> String {
        let mut text = format!(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: 1\r\n\r\n[Metadata]\r\nTitle:Song\r\nArtist:Band\r\nCreator:me\r\nVersion:{version}\r\n\r\n[HitObjects]\r\n"
        );
        for k in 0..notes {
            text += &format!("256,192,{},1,0\r\n", 1000 + k * 200);
        }
        text
    }

    fn approved(version: &str, text: &str) -> Approved {
        Approved {
            version: version.into(),
            hash: crate::net::map_hash(text.as_bytes()),
        }
    }

    fn osz(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            out.write_all(text.as_bytes()).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    #[test]
    fn only_the_approved_difficulties_of_the_answer_count() {
        let text = r#"{"id": 3, "diffs": [
            {"version": "Oni", "hash": "e29002fd23a7bb2", "tko": false, "approved": true},
            {"version": "Muzukashii", "hash": "1f", "tko": true, "approved": false}]}"#;
        assert_eq!(
            parse(text),
            Some(vec![Approved {
                version: "Oni".into(),
                hash: 0xe29002fd23a7bb2
            }])
        );
        assert_eq!(parse(r#"{"id": 3, "diffs": []}"#), Some(Vec::new()));
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn the_folder_keeps_an_approved_difficulty_changes_it_or_lost_it() {
        let dir = folder("compare");
        let (oni, kantan, futsuu) = (map("Oni", 12), map("Kantan", 12), map("Futsuu", 12));
        std::fs::write(dir.join("a [Oni].osu"), &oni).unwrap();
        std::fs::write(dir.join("a [Kantan].osu"), map("Kantan", 14)).unwrap();
        let list = compare(
            &dir,
            &[
                approved("Oni", &oni),
                approved("Kantan", &kantan),
                approved("Futsuu", &futsuu),
            ],
        );
        let kept: Vec<_> = list.iter().map(|(_, k)| k.clone()).collect();
        assert_eq!(
            kept,
            [
                Kept::Same,
                Kept::Changed(dir.join("a [Kantan].osu")),
                Kept::Missing
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restoring_keeps_the_changes_as_a_new_difficulty_and_brings_the_approved_back() {
        let dir = folder("restore");
        let (kantan, futsuu) = (map("Kantan", 12), map("Futsuu", 12));
        let changed = map("Kantan", 14);
        std::fs::write(dir.join("a [Kantan].osu"), &changed).unwrap();
        let list = compare(
            &dir,
            &[approved("Kantan", &kantan), approved("Futsuu", &futsuu)],
        );
        let archive = osz(&[("a [Kantan].osu", &kantan), ("a [Futsuu].osu", &futsuu)]);
        let moved = restore(&dir, &list, &archive, |path| {
            let new = path.with_file_name("a [Kantan (copy)].osu");
            std::fs::copy(path, &new).map_err(|e| e.to_string())?;
            Ok(new)
        })
        .unwrap();
        assert_eq!(
            moved,
            [(
                dir.join("a [Kantan].osu"),
                dir.join("a [Kantan (copy)].osu")
            )]
        );
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(read("a [Kantan].osu"), kantan);
        assert_eq!(read("a [Kantan (copy)].osu"), changed);
        assert_eq!(read("a [Futsuu].osu"), futsuu);
        assert!(
            compare(
                &dir,
                &list.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>()
            )
            .iter()
            .all(|(_, k)| *k == Kept::Same)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
