//! Profile pictures, set on the website: DIR/avatars/<lowercased name>, as uploaded (PNG,
//! JPEG, GIF or WebP, checked by their first bytes; browsers scale them). Which players have
//! one is kept in memory, with a version for links, so pages don't look on disk for each.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Largest picture taken.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Widest and tallest picture taken: every visitor's browser unpacks it, whatever it weighs.
const MAX_SIDE: u32 = 4096;

struct Store {
    dir: PathBuf,
    /// By lowercased name: the picture's version (when it was set).
    has: Mutex<HashMap<String, u64>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Lists DIR/avatars. Without it (tests) nobody has a picture and none can be set.
pub fn init(dir: PathBuf) {
    let dir = dir.join("avatars");
    let has: HashMap<String, u64> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let modified = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                    Some((name, modified))
                })
                .collect()
        })
        .unwrap_or_default();
    println!("{} profile pictures ({})", has.len(), dir.display());
    let _ = STORE.set(Store { dir, has: Mutex::new(has) });
}

/// What kind of image `bytes` are (its content type), if one that's taken.
pub fn kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// Width and height, from the picture's header.
fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let le16 = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le24 = |at: usize| Some(le16(at)? | (*bytes.get(at + 2)? as u32) << 16);
    match kind(bytes)? {
        "image/png" => Some((be32(16)?, be32(20)?)),
        "image/gif" => Some((le16(6)?, le16(8)?)),
        "image/webp" => match bytes.get(12..16)? {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = le16(21)? | le16(23)? << 16;
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            _ => None,
        },
        _ => {
            // JPEG: segments, each with its length, up to the frame header.
            let mut at = 2;
            loop {
                let marker = *bytes.get(at + 1)?;
                if *bytes.get(at)? != 0xFF {
                    return None;
                }
                if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                    return Some((be16(at + 7)?, be16(at + 5)?));
                }
                at += 2 + be16(at + 2)? as usize;
            }
        }
    }
}

/// The version of `key`'s picture, if they have one.
pub fn version(key: &str) -> Option<u64> {
    STORE.get()?.has.lock().unwrap().get(key).copied()
}

/// `key`'s picture and its content type.
pub fn load(key: &str) -> Option<(&'static str, Vec<u8>)> {
    let s = STORE.get()?;
    if !s.has.lock().unwrap().contains_key(key) {
        return None;
    }
    let bytes = std::fs::read(s.dir.join(key)).ok()?;
    Some((kind(&bytes)?, bytes))
}

/// Sets `key`'s picture (`now`: its version).
pub fn save(key: &str, bytes: &[u8], now: u64) -> Result<(), String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("that picture is too big (at most {} MB)", MAX_BYTES / 1024 / 1024));
    }
    kind(bytes).ok_or("pick a PNG, JPEG, GIF or WebP picture")?;
    let (w, h) = size(bytes).ok_or("that picture can't be read")?;
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("that picture is too large (at most {MAX_SIDE} by {MAX_SIDE})"));
    }
    let s = STORE.get().ok_or("pictures can't be set here")?;
    std::fs::create_dir_all(&s.dir).map_err(|e| e.to_string())?;
    let tmp = s.dir.join(format!("{key}.tmp"));
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, s.dir.join(key))).map_err(|e| {
        eprintln!("avatars: saving {key}: {e}");
        "the server couldn't save it".to_string()
    })?;
    s.has.lock().unwrap().insert(key.to_string(), now);
    Ok(())
}

pub fn remove(key: &str) {
    let Some(s) = STORE.get() else { return };
    if s.has.lock().unwrap().remove(key).is_some() {
        let _ = std::fs::remove_file(s.dir.join(key));
    }
}

/// `old`'s picture is `new`'s from now on.
pub fn rename(old: &str, new: &str) {
    let Some(s) = STORE.get().filter(|_| old != new) else { return };
    let mut has = s.has.lock().unwrap();
    let Some(version) = has.remove(old) else { return };
    match std::fs::rename(s.dir.join(old), s.dir.join(new)) {
        Ok(()) => has.insert(new.to_string(), version),
        Err(e) => {
            eprintln!("renaming a picture: {e}");
            has.insert(old.to_string(), version)
        }
    };
}

/// The file in a `multipart/form-data` body (its field `field`), given the Content-Type
/// header's value.
pub fn multipart_file<'a>(content_type: &str, body: &'a [u8], field: &str) -> Option<&'a [u8]> {
    let boundary = content_type.split(';').find_map(|p| p.trim().strip_prefix("boundary="))?.trim_matches('"');
    let delimiter = format!("--{boundary}");
    let d = delimiter.as_bytes();
    let find = |hay: &[u8], needle: &[u8], from: usize| hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from);
    let mut at = find(body, d, 0)?;
    loop {
        let start = at + d.len();
        let next = find(body, d, start)?;
        // A part: headers, a blank line, the content, then CRLF before the next delimiter.
        let part = &body[start..next];
        if let Some(split) = find(part, b"\r\n\r\n", 0) {
            let headers = String::from_utf8_lossy(&part[..split]).to_ascii_lowercase();
            if headers.contains(&format!("name=\"{field}\"")) {
                let content = &part[split + 4..];
                return Some(content.strip_suffix(b"\r\n").unwrap_or(content));
            }
        }
        at = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_come_from_each_formats_header() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend([0, 0, 1, 0, 0, 1, 0, 0]);
        assert_eq!(size(&png), Some((256, 65536)));
        assert_eq!(size(&[b'G', b'I', b'F', b'8', b'9', b'a', 0, 2, 0, 1]), Some((512, 256)));
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0];
        jpeg.extend([0xFF, 0xC0, 0, 11, 8, 1, 0, 2, 0, 3]);
        assert_eq!(size(&jpeg), Some((512, 256)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([0; 8]);
        webp.extend([255, 1, 0, 255, 0, 0]);
        assert_eq!(size(&webp), Some((512, 256)));
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L".to_vec();
        lossless.extend([0, 0, 0, 0, 0x2F]);
        lossless.extend(((511u32) | (255 << 14)).to_le_bytes());
        assert_eq!(size(&lossless), Some((512, 256)));
        assert_eq!(size(&[0x89, b'P', b'N', b'G']), None);
        assert_eq!(size(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), None);
    }

    #[test]
    fn a_huge_picture_is_refused_whatever_it_weighs() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend([0, 1, 0, 0, 0, 1, 0, 0]);
        assert!(save("x", &png, 1).unwrap_err().contains("too large"));
    }

    #[test]
    fn pictures_go_by_their_first_bytes() {
        assert_eq!(kind(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(kind(b"\xFF\xD8\xFF\xE0"), Some("image/jpeg"));
        assert_eq!(kind(b"GIF89a"), Some("image/gif"));
        assert_eq!(kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(kind(b"<svg onload=alert(1)>"), None);
        assert_eq!(kind(b""), None);
    }

    #[test]
    fn the_file_comes_out_of_a_form() {
        let body = b"------x\r\nContent-Disposition: form-data; name=\"other\"\r\n\r\nhello\r\n------x\r\nContent-Disposition: form-data; name=\"avatar\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n\x89PNG\r\n\x1a\nDATA\r\n------x--\r\n";
        let file = multipart_file("multipart/form-data; boundary=----x", body, "avatar").unwrap();
        assert_eq!(file, b"\x89PNG\r\n\x1a\nDATA");
        assert_eq!(multipart_file("multipart/form-data; boundary=\"----x\"", body, "other").unwrap(), b"hello");
        assert!(multipart_file("multipart/form-data; boundary=----x", body, "nope").is_none());
        assert!(multipart_file("application/x-www-form-urlencoded", body, "avatar").is_none());
    }
}
