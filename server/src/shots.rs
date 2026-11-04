//! Screenshots sent in the game's chat: DIR/shots/<id>, JPEGs, under an id nobody can guess (one
//! sent to a friend is seen by those with its link). Each is kept for `KEEP_SECS`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Largest screenshot taken (the game sends them at most 1600 wide).
pub const MAX_BYTES: usize = 2 << 20;
/// Widest and tallest: every player who opens it unpacks it, whatever it weighs.
const MAX_SIDE: u32 = 4096;
/// How long one stays.
const KEEP_SECS: u64 = 14 * 24 * 3600;
/// How long a player waits between two.
const WAIT_SECS: u64 = 20;
/// Kept at most: the oldest go first.
const MAX_KEPT: usize = 2000;

struct Store {
    dir: PathBuf,
    /// By player: when they last sent one.
    sent: Mutex<HashMap<String, u64>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Without it (tests) none can be sent.
pub fn init(dir: PathBuf) {
    let _ = STORE.set(Store {
        dir: dir.join("shots"),
        sent: Mutex::new(HashMap::new()),
    });
}

/// What can be asked for: 32 hex digits, which also keeps a path out of it.
fn valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Why `bytes` won't do, if so.
fn check(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_BYTES {
        return Err("that screenshot is too big".into());
    }
    if crate::avatars::kind(bytes) != Some("image/jpeg") {
        return Err("a screenshot has to be a JPEG".into());
    }
    match crate::avatars::size(bytes) {
        Some((w, h)) if w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE => Ok(()),
        _ => Err("that screenshot's size won't do".into()),
    }
}

/// Whether `key` may send one at `now`, if they've waited, noting it.
fn may_send(sent: &mut HashMap<String, u64>, key: &str, now: u64) -> bool {
    if sent.get(key).is_some_and(|&at| now < at + WAIT_SECS) {
        return false;
    }
    sent.retain(|_, at| now < *at + WAIT_SECS);
    sent.insert(key.to_string(), now);
    true
}

/// Which of these (name, when it was written) go at `now`: old ones, then the oldest over the limit.
fn expired(mut files: Vec<(PathBuf, u64)>, now: u64, room: usize) -> Vec<PathBuf> {
    files.sort_by_key(|f| f.1);
    let over = files.len().saturating_sub(room);
    files
        .into_iter()
        .enumerate()
        .filter(|(i, f)| *i < over || now >= f.1 + KEEP_SECS)
        .map(|(_, f)| f.0)
        .collect()
}

fn tidy(dir: &std::path::Path, now: u64) {
    let files = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let at = e.metadata().ok()?.modified().ok()?;
                    let at = at.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
                    Some((e.path(), at))
                })
                .collect()
        })
        .unwrap_or_default();
    for file in expired(files, now, MAX_KEPT - 1) {
        let _ = std::fs::remove_file(file);
    }
}

/// Keeps `key`'s screenshot; its id.
pub fn save(key: &str, bytes: &[u8], now: u64) -> Result<String, String> {
    check(bytes)?;
    let store = STORE.get().ok_or("screenshots aren't kept here")?;
    if !may_send(&mut store.sent.lock().unwrap(), key, now) {
        return Err("wait a little before sending another screenshot".into());
    }
    std::fs::create_dir_all(&store.dir).map_err(|e| e.to_string())?;
    tidy(&store.dir, now);
    let id: String = crate::random_bytes::<16>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(store.dir.join(&id), bytes).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn load(id: &str) -> Option<Vec<u8>> {
    let store = STORE.get()?;
    valid(id).then(|| std::fs::read(store.dir.join(id)).ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG's start, down to its frame header, `w` by `h`.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        b.extend(h.to_be_bytes());
        b.extend(w.to_be_bytes());
        b.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b
    }

    #[test]
    fn only_a_jpeg_of_a_sensible_size_goes() {
        assert_eq!(check(&jpeg(1600, 900)), Ok(()));
        assert!(check(&jpeg(9000, 900)).is_err());
        assert!(check(b"\x89PNG not a jpeg").is_err());
        assert!(check(b"nothing").is_err());
        let mut big = jpeg(1600, 900);
        big.resize(MAX_BYTES + 1, 0);
        assert!(check(&big).is_err());
    }

    #[test]
    fn an_id_cant_go_anywhere_else() {
        assert!(valid("0123456789abcdef0123456789abcdef"));
        for bad in [
            "",
            "../../accounts.json",
            "0123456789abcdef0123456789abcdeg",
            "0123456789ABCDEF0123456789ABCDEF",
        ] {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn a_player_waits_between_screenshots() {
        let mut sent = HashMap::new();
        assert!(may_send(&mut sent, "a", 100));
        assert!(!may_send(&mut sent, "a", 100 + WAIT_SECS - 1));
        assert!(may_send(&mut sent, "b", 101), "others aren't held up");
        assert!(may_send(&mut sent, "a", 100 + WAIT_SECS));
    }

    #[test]
    fn old_screenshots_go_and_the_oldest_when_theres_no_room() {
        let f = |n: &str, at| (PathBuf::from(n), at);
        let now = KEEP_SECS + 1000;
        let files = vec![f("new", now - 10), f("old", 500), f("mid", now - 20)];
        assert_eq!(expired(files.clone(), now, 10), [PathBuf::from("old")]);
        assert_eq!(
            expired(files, now, 1),
            [PathBuf::from("old"), PathBuf::from("mid")]
        );
    }
}
