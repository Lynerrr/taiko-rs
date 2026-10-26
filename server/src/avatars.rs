//! Profile pictures, set on the website: DIR/avatars/<lowercased name>, as uploaded (PNG,
//! JPEG, GIF or WebP, checked by their first bytes; browsers scale them). Which players have
//! one is kept in memory, with a version for links, so pages don't look on disk for each.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Largest picture taken.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

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
