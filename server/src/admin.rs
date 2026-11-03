//! Admins' tools in the game (`Up::Admin`): what the website's admin buttons do, and kicks,
//! global chat mutes and takedowns, announcements, lobbies, a look at who's on, and which
//! accounts log in from the same places. Every one is checked here (`is_admin`), whatever the
//! game shows, and written to DIR/admin.log.
//!
//! Where accounts log in from is kept as the address (or its /64) in DIR/seen.json, a few per
//! account, only to tell admins which accounts share one: the addresses never leave the server.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::online::{self, AdminLookup, AdminOp, AdminOverview, AdminPlayer, ChatLine, Down};
use crate::{CHAT_HISTORY, State, is_admin, now_secs};

/// How long a kicked player stays out.
const KICKED_FOR: Duration = Duration::from_secs(120);
/// Addresses kept per account.
const SEEN_PER_ACCOUNT: usize = 8;
/// Admin actions kept in memory for the overview.
const LOG_KEPT: usize = 40;
/// The longest mute (a week).
const MAX_MUTE_MINUTES: u32 = 7 * 24 * 60;

pub struct AdminState {
    dir: PathBuf,
    /// Kicked players (lowercased) and when they may come back.
    kicked: Mutex<HashMap<String, Instant>>,
    /// Lowercased name -> (address, unix seconds last seen), the latest last.
    seen: Mutex<HashMap<String, Vec<(String, u64)>>>,
    seen_changed: AtomicBool,
    log: Mutex<VecDeque<String>>,
    started: Instant,
}

impl AdminState {
    pub fn load(dir: &Path) -> AdminState {
        let seen = crate::load_saved(&dir.join("seen.json"));
        AdminState {
            dir: dir.to_path_buf(),
            kicked: Mutex::default(),
            seen: Mutex::new(seen),
            seen_changed: AtomicBool::new(false),
            log: Mutex::default(),
            started: Instant::now(),
        }
    }

    /// How long `key` stays out after a kick, if they're still out.
    pub fn kicked_for(&self, key: &str) -> Option<Duration> {
        let mut kicked = self.kicked.lock().unwrap();
        kicked.retain(|_, until| *until > Instant::now());
        kicked
            .get(key)
            .map(|until| until.saturating_duration_since(Instant::now()))
    }

    /// `key` logged in or connected from `ip` (its `limit_key`).
    pub fn seen(&self, key: &str, ip: IpAddr) {
        let ip = ip.to_string();
        let mut seen = self.seen.lock().unwrap();
        let list = seen.entry(key.to_string()).or_default();
        list.retain(|(a, _)| *a != ip);
        list.push((ip, now_secs()));
        if list.len() > SEEN_PER_ACCOUNT {
            list.remove(0);
        }
        self.seen_changed.store(true, Ordering::Relaxed);
    }

    /// Other accounts seen at one of `key`'s addresses.
    fn alts(&self, key: &str) -> Vec<String> {
        let seen = self.seen.lock().unwrap();
        let Some(mine) = seen.get(key) else {
            return Vec::new();
        };
        let mut out: Vec<String> = seen
            .iter()
            .filter(|(k, list)| {
                *k != key && list.iter().any(|(a, _)| mine.iter().any(|(m, _)| m == a))
            })
            .map(|(k, _)| k.clone())
            .collect();
        out.sort();
        out
    }

    /// Writes seen.json if it changed (by the saver).
    pub fn write(&self) {
        if !self.seen_changed.swap(false, Ordering::Relaxed) {
            return;
        }
        let Ok(text) = serde_json::to_string(&*self.seen.lock().unwrap()) else {
            return;
        };
        if let Err(e) = crate::write_private(&self.dir.join("seen.json"), &text) {
            eprintln!("saving seen.json: {e}");
        }
    }

    /// Notes what admin `by` did: the server's output, DIR/admin.log and the overview.
    fn record(&self, by: &str, what: &str) {
        let secs = now_secs();
        let line = format!("{:02}:{:02} {by} {what}", secs / 3600 % 24, secs / 60 % 60);
        println!("admin: {by} {what}");
        let path = self.dir.join("admin.log");
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{secs} {by} {what}");
            }
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
        let mut log = self.log.lock().unwrap();
        log.push_back(line);
        if log.len() > LOG_KEPT {
            log.pop_front();
        }
    }
}

/// Admin `key` (`name` as written) does `op`; what happened goes back to them as a notice.
pub fn op(state: &State, key: &str, name: &str, op: AdminOp) {
    if !is_admin(key) {
        state.notice(key, "only admins can do that".into());
        return;
    }
    let result = run(state, key, op);
    match result {
        Ok(Some((msg, logged))) => {
            if logged {
                state.admin.record(name, &msg);
            }
            state.notice(key, msg);
        }
        Ok(None) => {}
        Err(e) => state.notice(key, e),
    }
}

/// Ok(Some((message, whether it goes in the log))), Ok(None) when the answer went out already.
fn run(state: &State, key: &str, op: AdminOp) -> Result<Option<(String, bool)>, String> {
    let done = |msg: String| Ok(Some((msg, true)));
    match op {
        AdminOp::Ban { name, on } => done(state.set_banned(key, &name, on)?),
        AdminOp::Rename { name, new } => done(format!(
            "{name} is called {} now",
            state.rename(key, &name, &new)?
        )),
        AdminOp::Nominator { name, on } => done(state.set_nominator(key, &name, on)?),
        AdminOp::RemoveAvatar { name } => {
            let (target, shown) = account(state, &name)?;
            crate::avatars::remove(&target);
            done(format!("took {shown}'s picture off"))
        }
        AdminOp::Kick { name } => {
            let (target, shown) = state.admin_target(key, &name)?;
            state
                .admin
                .kicked
                .lock()
                .unwrap()
                .insert(target.clone(), Instant::now() + KICKED_FOR);
            let hub = state.hub.lock().unwrap();
            if !hub.conns.contains_key(&target) {
                return Err(format!("{shown} isn't online"));
            }
            hub.send_to(&target, &Down::Notice("an admin disconnected you".into()));
            hub.close_account(&target);
            done(format!(
                "kicked {shown} (out for {} minutes)",
                KICKED_FOR.as_secs() / 60
            ))
        }
        AdminOp::Mute { name, minutes } => {
            let (target, shown) = state.admin_target(key, &name)?;
            let minutes = minutes.min(MAX_MUTE_MINUTES);
            {
                let mut a = state.accounts.lock().unwrap();
                let user = a
                    .users
                    .get_mut(&target)
                    .ok_or("there's no player by that name")?;
                user.muted_until = (minutes > 0).then(|| now_secs() + u64::from(minutes) * 60);
            }
            state.save();
            if minutes == 0 {
                state.notice(&target, "you can talk in the global chat again".into());
                done(format!("unmuted {shown}"))
            } else {
                state.notice(
                    &target,
                    format!(
                        "an admin muted you in the global chat for {}",
                        span(minutes)
                    ),
                );
                done(format!("muted {shown} for {}", span(minutes)))
            }
        }
        AdminOp::Announce(text) => {
            let text = crate::net::clean_chat(&text);
            if text.is_empty() {
                return Err("nothing to announce".into());
            }
            let line = ChatLine {
                from: online::SERVER_NAME.to_string(),
                to: String::new(),
                text: text.clone(),
                at: now_secs(),
                medal: None,
            };
            {
                let mut hub = state.hub.lock().unwrap();
                hub.global.push_back(line.clone());
                if hub.global.len() > CHAT_HISTORY {
                    hub.global.pop_front();
                }
                hub.chat_changed = true;
                hub.send_to_all(&Down::Notice(text.clone()));
                hub.send_to_all(&Down::GlobalChat(line));
            }
            state.save();
            done(format!("announced: {text}"))
        }
        AdminOp::DeleteGlobal { from, at } => {
            let n = take_down(state, |l| l.from == from && l.at == at);
            if n == 0 {
                return Err("that line isn't in the chat any more".into());
            }
            done(format!("took down a line of {from}"))
        }
        AdminOp::PurgeGlobal { name } => {
            let k = name.trim().to_ascii_lowercase();
            let n = take_down(state, |l| {
                l.from.to_ascii_lowercase() == k && l.medal.is_none()
            });
            done(format!("took down {n} global chat lines of {name}"))
        }
        AdminOp::Overview => {
            let overview = overview(state);
            state
                .hub
                .lock()
                .unwrap()
                .send_to(key, &Down::AdminOverview(Box::new(overview)));
            Ok(None)
        }
        AdminOp::Lookup { name } => {
            let lookup = lookup(state, &name)?;
            state
                .hub
                .lock()
                .unwrap()
                .send_to(key, &Down::AdminLookup(Box::new(lookup)));
            Ok(None)
        }
        AdminOp::DeletePlay { id } => {
            let player = state.scores.lock().unwrap().remove(id)?;
            done(format!("deleted play {id} of {player}"))
        }
        AdminOp::SetPp { id, pp } => {
            if pp.is_some_and(|v| !v.is_finite() || v < 0.0) {
                return Err("pp is a number, 0 or more".into());
            }
            state.scores.lock().unwrap().set_pp(id, pp)?;
            done(match pp {
                Some(v) => format!("play {id} is worth {v:.2}pp now"),
                None => format!("play {id} is worth what it was sent with again"),
            })
        }
        AdminOp::KickFromRoom { room, name } => {
            let lobbies = state.lobbies.lock().unwrap();
            let l = lobbies.get(&room).ok_or("that lobby is gone")?;
            l.room.kick(&name);
            done(format!("took {name} out of lobby {room} \"{}\"", l.title))
        }
        AdminOp::CloseRoom { room } => {
            let l = state
                .lobbies
                .lock()
                .unwrap()
                .remove(&room)
                .ok_or("that lobby is gone")?;
            done(format!("closed lobby {room} \"{}\"", l.title))
        }
    }
}

/// `name`'s key and name as written, for any account (admins included).
fn account(state: &State, name: &str) -> Result<(String, String), String> {
    let key = name.trim().to_ascii_lowercase();
    let shown = state
        .accounts
        .lock()
        .unwrap()
        .users
        .get(&key)
        .map(|u| u.name.clone());
    Ok((key, shown.ok_or("there's no player by that name")?))
}

/// "5 minutes", "2 hours", "3 days".
fn span(minutes: u32) -> String {
    let (n, unit) = if minutes >= 1440 && minutes.is_multiple_of(1440) {
        (minutes / 1440, "day")
    } else if minutes >= 60 && minutes.is_multiple_of(60) {
        (minutes / 60, "hour")
    } else {
        (minutes, "minute")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// Takes the global chat lines `which` picks down, for everyone; how many.
fn take_down(state: &State, which: impl Fn(&ChatLine) -> bool) -> usize {
    let mut hub = state.hub.lock().unwrap();
    let gone: Vec<ChatLine> = hub.global.iter().filter(|l| which(l)).cloned().collect();
    hub.global.retain(|l| !which(l));
    for l in &gone {
        hub.send_to_all(&Down::GlobalChatDeleted {
            from: l.from.clone(),
            at: l.at,
        });
    }
    if !gone.is_empty() {
        hub.chat_changed = true;
        drop(hub);
        state.save();
    }
    gone.len()
}

fn overview(state: &State) -> AdminOverview {
    let (accounts, sessions, mutes) = {
        let a = state.accounts.lock().unwrap();
        let mutes: HashMap<String, u64> = a
            .users
            .iter()
            .filter_map(|(k, u)| {
                u.muted_until
                    .filter(|&t| t > now_secs())
                    .map(|t| (k.clone(), t))
            })
            .collect();
        (a.users.len() as u32, a.sessions.len() as u32, mutes)
    };
    let mut players: Vec<AdminPlayer> = {
        let hub = state.hub.lock().unwrap();
        hub.conns
            .iter()
            .filter_map(|(k, conns)| {
                let last = conns.last()?;
                Some(AdminPlayer {
                    name: last.name.clone(),
                    activity: Some(match conns.iter().find_map(|c| c.watching.clone()) {
                        Some(w) => online::Activity::Watching {
                            name: shown_name(&w, &hub),
                        },
                        None => last.activity.clone(),
                    }),
                    connections: conns.len() as u32,
                    watching: conns.iter().find_map(|c| c.watching.clone()),
                    muted_until: mutes.get(k).copied(),
                })
            })
            .collect()
    };
    players.sort_by_cached_key(|p| p.name.to_lowercase());
    AdminOverview {
        uptime_secs: state.admin.started.elapsed().as_secs(),
        accounts,
        sessions,
        players,
        rooms: state.rooms(),
        log: state.admin.log.lock().unwrap().iter().cloned().collect(),
    }
}

/// A watched player's name as written, from the hub (the accounts lock isn't taken with it).
fn shown_name(key: &str, hub: &crate::Hub) -> String {
    hub.conns
        .get(key)
        .and_then(|v| v.last())
        .map(|c| c.name.clone())
        .unwrap_or_else(|| key.to_string())
}

fn lookup(state: &State, name: &str) -> Result<AdminLookup, String> {
    let key = name.trim().to_ascii_lowercase();
    let mut out = {
        let a = state.accounts.lock().unwrap();
        let u = a.users.get(&key).ok_or("there's no player by that name")?;
        AdminLookup {
            name: u.name.clone(),
            created: u.created,
            banned: u.banned,
            nominator: u.nominator || is_admin(&key),
            has_email: u.email.is_some(),
            muted_until: u.muted_until.filter(|&t| t > now_secs()),
            ..AdminLookup::default()
        }
    };
    out.online = state.hub.lock().unwrap().conns.contains_key(&key);
    out.plays = state.scores.lock().unwrap().plays_of(&key).len() as u32;
    let alts = state.admin.alts(&key);
    out.alts = alts.iter().map(|k| state.display_name(k)).collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span(1), "1 minute");
        assert_eq!(span(45), "45 minutes");
        assert_eq!(span(120), "2 hours");
        assert_eq!(span(1440), "1 day");
        assert_eq!(span(90), "90 minutes");
    }

    #[test]
    fn alts_share_an_address() {
        let dir = std::env::temp_dir().join(format!("taiko-admin-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let a = AdminState::load(&dir);
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        a.seen("troll", ip("1.2.3.4"));
        a.seen("troll2", ip("1.2.3.4"));
        a.seen("troll", ip("5.6.7.8"));
        a.seen("other", ip("9.9.9.9"));
        a.seen("third", ip("5.6.7.8"));
        assert_eq!(a.alts("troll"), ["third", "troll2"]);
        assert!(a.alts("other").is_empty() && a.alts("nobody").is_empty());
        for i in 0..20 {
            a.seen("busy", ip(&format!("10.0.0.{i}")));
        }
        assert_eq!(a.seen.lock().unwrap()["busy"].len(), SEEN_PER_ACCOUNT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
