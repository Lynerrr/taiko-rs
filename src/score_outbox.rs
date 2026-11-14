//! Plays the server never answered (it was being updated, or the connection dropped on the
//! way), kept on disk and sent again later with the key they first went with. A server that
//! lists `online::SUBMIT_KEY_FEATURE` stores a play once per key, so one it did store before
//! the answer got lost answers the resend as it did the first send.

use crate::online;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How long an unsent play is kept: well within the week servers remember keys for.
pub const KEEP_SECS: u64 = 3 * 24 * 3600;
const FILE: &str = "unsent-scores.jsonl";

/// A play on its way: where it goes, for whom, since when.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Unsent {
    pub server: String,
    /// The account it was played on (`Config::online_user`): it only goes again from there.
    pub player: String,
    /// Unix seconds it was first sent.
    pub queued: u64,
    pub play: online::ScoreSubmit,
}

impl Unsent {
    pub fn key(&self) -> &str {
        self.play.operation_key.as_deref().unwrap_or_default()
    }

    fn fresh(&self, now: u64) -> bool {
        !self.key().is_empty() && now.saturating_sub(self.queued) < KEEP_SECS
    }
}

/// The unsent plays, kept in `file` (none: in memory only).
pub struct Outbox {
    file: Option<PathBuf>,
    items: Vec<Unsent>,
}

impl Outbox {
    /// The one in the data folder.
    pub fn load() -> Outbox {
        Outbox::at(crate::config::data_dir().map(|d| d.join(FILE)))
    }

    /// Kept in `file`, with what it holds already (past `KEEP_SECS` left out).
    pub fn at(file: Option<PathBuf>) -> Outbox {
        let now = unix_now();
        let items = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|line| serde_json::from_str::<Unsent>(line).ok())
                    .filter(|u| u.fresh(now))
                    .collect()
            })
            .unwrap_or_default();
        Outbox { file, items }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Keeps `unsent` to send again (once per key).
    pub fn push(&mut self, unsent: Unsent) {
        if !unsent.fresh(unix_now()) || self.items.iter().any(|u| u.key() == unsent.key()) {
            return;
        }
        self.items.push(unsent);
        self.save();
    }

    /// The server answered the play sent with `key`: it isn't kept any longer.
    pub fn remove(&mut self, key: &str) {
        let before = self.items.len();
        self.items.retain(|u| u.key() != key);
        if self.items.len() != before {
            self.save();
        }
    }

    /// What goes again to `server` for `player`, past `KEEP_SECS` dropped first.
    pub fn due(&mut self, server: &str, player: &str) -> Vec<Unsent> {
        let now = unix_now();
        let before = self.items.len();
        self.items.retain(|u| u.fresh(now));
        if self.items.len() != before {
            self.save();
        }
        self.items
            .iter()
            .filter(|u| u.server == server && u.player.eq_ignore_ascii_case(player))
            .cloned()
            .collect()
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if self.items.is_empty() {
            let _ = std::fs::remove_file(file);
            return;
        }
        let mut text = String::new();
        for u in &self.items {
            if let Ok(line) = serde_json::to_string(u) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        crate::config::write_whole(file, &text);
    }
}

/// A key no other play has: 32 hex digits from the clock, the process and the hasher's own
/// random seed.
pub fn new_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    let half = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(count);
        h.write_u32(std::process::id());
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}{:016x}", half(1), half(2))
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsent(key: &str, queued: u64) -> Unsent {
        Unsent {
            server: "s".into(),
            player: "Me".into(),
            queued,
            play: online::ScoreSubmit {
                map: crate::net::MapInfo {
                    hash: 1,
                    artist: "A".into(),
                    title: "T".into(),
                    version: "V".into(),
                    rate: 1.0,
                    from: None,
                    set_id: None,
                    mods: Vec::new(),
                    multiverse: None,
                    shapeshift: None,
                    seed: None,
                    defense: None,
                    stats: None,
                },
                score: 1,
                accuracy: 1.0,
                max_combo: 1,
                great: 1,
                ok: 0,
                miss: 0,
                pp: None,
                date: queued,
                replay_version: 0,
                inputs: Vec::new(),
                stars: None,
                play_ms: None,
                scroll_speed: None,
                md5: None,
                pp_version: online::PP_VERSION,
                timing: None,
                rolls: None,
                operation_key: Some(key.into()),
            },
        }
    }

    #[test]
    fn keys_differ_and_suit_the_server() {
        let keys: std::collections::HashSet<String> = (0..1000).map(|_| new_key()).collect();
        assert_eq!(keys.len(), 1000);
        assert!(
            keys.iter()
                .all(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit()))
        );
    }

    #[test]
    fn unsent_plays_outlive_the_game_until_answered_or_old() {
        let file =
            std::env::temp_dir().join(format!("taiko-rs-outbox-test-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let now = unix_now();
        let mut out = Outbox::at(Some(file.clone()));
        out.push(unsent("aaaaaaaaaaaaaaaa", now));
        out.push(unsent("aaaaaaaaaaaaaaaa", now));
        out.push(unsent("bbbbbbbbbbbbbbbb", now - 60));
        out.push(unsent("cccccccccccccccc", now - KEEP_SECS - 1));
        assert_eq!(out.len(), 2, "once per key, and not when too old");

        let mut again = Outbox::at(Some(file.clone()));
        assert_eq!(
            again.due("s", "me").len(),
            2,
            "kept on disk, for the same player"
        );
        assert!(again.due("s", "someone").is_empty());
        assert!(again.due("other", "me").is_empty());

        again.remove("aaaaaaaaaaaaaaaa");
        let mut last = Outbox::at(Some(file.clone()));
        let due = last.due("s", "me");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].key(), "bbbbbbbbbbbbbbbb");
        last.remove("bbbbbbbbbbbbbbbb");
        assert!(!file.exists(), "an empty outbox leaves no file");
    }
}
