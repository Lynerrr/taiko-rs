//! taiko.rs online server: accounts (used by multiplayer for now) and lobbies hosted here, so
//! players don't need an open port. Rooms run the game's own room code (`src/net.rs`); the
//! protocol in front of them is `src/online.rs`.
//!
//!   taiko-server [--port 7800] [--data DIR] [--http-port 8080 [--behind-proxy]]
//!
//! With --http-port it also serves the website (`web.rs`); --behind-proxy takes the visitor's
//! address from the proxy's header (Cloudflare), for the per-address limits.
//!
//! Accounts and sessions live in DIR/accounts.json (passwords as argon2 hashes).

#[allow(dead_code)]
#[path = "../../src/net.rs"]
mod net;
#[allow(dead_code)]
#[path = "../../src/online.rs"]
mod online;
mod card;
mod covers;
mod discord;
#[allow(dead_code)]
#[path = "../../src/font.rs"]
mod font;
mod scores;
mod pages;
mod web;

use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, Read};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use online::{Activity, ChatLine, Down, Envelope, Friend, OnlinePlayer, Reply, Request, RoomInfo, SoloPlay, Up};
use serde::{Deserialize, Serialize};

/// Sessions last this long without logging in again.
const SESSION_DAYS: u64 = 180;
/// A lobby nobody is in is closed after this.
const EMPTY_ROOM: Duration = Duration::from_secs(20);
const MAX_ROOMS: usize = 200;
/// Per IP: new accounts per hour, failed logins per 10 minutes.
const REGISTER_PER_HOUR: usize = 5;
const FAILED_LOGINS: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
struct User {
    name: String,
    hash: String,
    created: u64,
    /// Friends and incoming friend requests (lowercased names).
    #[serde(default)]
    friends: Vec<String>,
    #[serde(default)]
    requests: Vec<String>,
}

const MAX_FRIENDS: usize = 200;
/// Chat lines kept per pair of friends, and of the global chat (in memory only).
const CHAT_HISTORY: usize = 100;
/// How often a changed online list goes out to everyone.
const PLAYERS_EVERY: Duration = Duration::from_secs(2);

/// One open social connection (a player can have several, e.g. two PCs).
struct Conn {
    id: u64,
    /// The account's name as written.
    name: String,
    tx: mpsc::Sender<Down>,
    activity: Activity,
    /// Whose solo play this connection watches (lowercased name).
    watching: Option<String>,
    chat_times: VecDeque<Instant>,
}

/// A solo play being streamed to friends.
struct Play {
    play: SoloPlay,
    inputs: Vec<(f64, u8)>,
    t: f64,
    paused: bool,
    /// The connection streaming it.
    conn: u64,
}

#[derive(Default)]
struct Hub {
    conns: HashMap<String, Vec<Conn>>,
    plays: HashMap<String, Play>,
    /// By (a, b) with a < b.
    history: HashMap<(String, String), VecDeque<ChatLine>>,
    global: VecDeque<ChatLine>,
    /// Someone came, left or changed what they're doing since the online list last went out.
    players_changed: bool,
    next_conn: u64,
    /// Messages sent from the website lately, per account (its own rate limit).
    web_chat: HashMap<String, VecDeque<Instant>>,
}

impl Hub {
    fn send_to(&self, key: &str, d: &Down) {
        for c in self.conns.get(key).into_iter().flatten() {
            let _ = c.tx.send(d.clone());
        }
    }

    fn send_to_watchers(&self, key: &str, d: &Down) {
        for c in self.conns.values().flatten().filter(|c| c.watching.as_deref() == Some(key)) {
            let _ = c.tx.send(d.clone());
        }
    }

    fn conn(&mut self, key: &str, id: u64) -> Option<&mut Conn> {
        self.conns.get_mut(key)?.iter_mut().find(|c| c.id == id)
    }

    fn send_to_all(&self, d: &Down) {
        for c in self.conns.values().flatten() {
            let _ = c.tx.send(d.clone());
        }
    }

    /// False when `key` is sending from the website too fast (the game's limit: 8 in 5 s).
    fn web_chat_allowed(&mut self, key: &str) -> bool {
        let times = self.web_chat.entry(key.to_string()).or_default();
        times.retain(|t| t.elapsed() < Duration::from_secs(5));
        if times.len() >= 8 {
            return false;
        }
        times.push_back(Instant::now());
        true
    }

    /// Everyone online, as `Down::Players`.
    fn players(&self) -> Down {
        let mut players: Vec<OnlinePlayer> = self.conns.values().filter_map(|v| v.last()).map(|c| OnlinePlayer { name: c.name.clone(), activity: c.activity.clone() }).collect();
        players.sort_by_key(|p| p.name.to_lowercase());
        let count = players.len() as u32;
        players.truncate(online::MAX_LISTED);
        Down::Players { count, players }
    }

    /// False (and a notice) when this connection is chatting too fast.
    fn chat_allowed(&mut self, key: &str, id: u64) -> bool {
        let Some(c) = self.conn(key, id) else { return true };
        c.chat_times.retain(|t| t.elapsed() < Duration::from_secs(5));
        if c.chat_times.len() >= 8 {
            let _ = c.tx.send(Down::Notice("slow down a little".into()));
            return false;
        }
        c.chat_times.push_back(Instant::now());
        true
    }
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a < b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) }
}

#[derive(Serialize, Deserialize)]
struct Session {
    /// Lowercased account name (the key in `users`).
    user: String,
    created: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Accounts {
    /// By lowercased name: names are unique ignoring case.
    users: HashMap<String, User>,
    sessions: HashMap<String, Session>,
}

struct Lobby {
    room: net::Room,
    title: String,
    owner: String,
    empty_since: Option<Instant>,
}

#[derive(Default)]
struct Limits {
    registered: Vec<Instant>,
    failed: Vec<Instant>,
}

struct State {
    accounts: Mutex<Accounts>,
    path: PathBuf,
    lobbies: Mutex<HashMap<u32, Lobby>>,
    next_room: AtomicU32,
    limits: Mutex<HashMap<IpAddr, Limits>>,
    /// Verified against when the name doesn't exist, so a login takes as long either way.
    dummy_hash: String,
    hub: Mutex<Hub>,
    scores: Mutex<scores::Scores>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("/dev/urandom");
    b
}

fn new_token() -> String {
    random_bytes::<32>().iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::encode_b64(&random_bytes::<16>()).map_err(|e| e.to_string())?;
    Argon2::default().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

impl State {
    fn load(path: PathBuf) -> State {
        let mut accounts: Accounts = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let cutoff = now_secs().saturating_sub(SESSION_DAYS * 86400);
        accounts.sessions.retain(|_, s| s.created >= cutoff);
        println!("{} accounts, {} sessions ({})", accounts.users.len(), accounts.sessions.len(), path.display());
        // Scores live next to the accounts.
        let scores = scores::Scores::load(Some(path.parent().map(|d| d.to_path_buf()).unwrap_or_default()));
        State {
            accounts: Mutex::new(accounts),
            path,
            lobbies: Mutex::new(HashMap::new()),
            next_room: AtomicU32::new(1),
            limits: Mutex::new(HashMap::new()),
            dummy_hash: hash_password("not a real password").expect("argon2"),
            hub: Mutex::new(Hub::default()),
            scores: Mutex::new(scores),
        }
    }

    /// Writes accounts.json atomically (temp file, then rename), readable by its owner only.
    fn save(&self, accounts: &Accounts) {
        let tmp = self.path.with_extension("json.tmp");
        let Ok(text) = serde_json::to_string_pretty(accounts) else { return };
        if let Err(e) = std::fs::write(&tmp, text) {
            eprintln!("saving accounts: {e}");
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            eprintln!("saving accounts: {e}");
        }
    }

    /// Logs in from the website: the account's name as written and a new session token.
    fn web_login(&self, ip: IpAddr, name: &str, password: &str) -> Result<(String, String), String> {
        match login(self, ip, name, password) {
            Reply::Account { name, token } => Ok((name, token)),
            Reply::Error(e) => Err(e),
            _ => Err("couldn't log in".into()),
        }
    }

    fn logout(&self, token: &str) {
        let mut a = self.accounts.lock().unwrap();
        if a.sessions.remove(token).is_some() {
            self.save(&a);
        }
    }

    /// The account name behind a session token.
    fn user(&self, token: &str) -> Option<String> {
        let a = self.accounts.lock().unwrap();
        let s = a.sessions.get(token)?;
        a.users.get(&s.user).map(|u| u.name.clone())
    }

    fn new_session(&self, accounts: &mut Accounts, key: &str) -> String {
        let token = new_token();
        accounts.sessions.insert(token.clone(), Session { user: key.to_string(), created: now_secs() });
        token
    }

    /// Counts an event for `ip` inside `window`; false when it's over `max`.
    fn allow(&self, ip: IpAddr, pick: fn(&mut Limits) -> &mut Vec<Instant>, window: Duration, max: usize) -> bool {
        let mut limits = self.limits.lock().unwrap();
        let list = pick(limits.entry(ip).or_default());
        list.retain(|t| t.elapsed() < window);
        list.len() < max
    }

    fn record(&self, ip: IpAddr, pick: fn(&mut Limits) -> &mut Vec<Instant>) {
        pick(self.limits.lock().unwrap().entry(ip).or_default()).push(Instant::now());
    }

    fn rooms(&self) -> Vec<RoomInfo> {
        let lobbies = self.lobbies.lock().unwrap();
        let mut out: Vec<RoomInfo> = lobbies
            .iter()
            .filter_map(|(&id, l)| {
                let s = l.room.summary.lock().unwrap().clone();
                (s.players > 0).then(|| RoomInfo { id, title: l.title.clone(), host: if s.host.is_empty() { l.owner.clone() } else { s.host }, players: s.players, map: s.map, in_match: s.in_match })
            })
            .collect();
        out.sort_by_key(|r| r.id);
        out
    }
}

/// Handles one connection: a request, its reply, and for a lobby the hand-over to the room.
fn handle(state: &State, stream: TcpStream) {
    let ip = stream.peer_addr().map(|a| a.ip()).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream);
    let reply = |reader: &mut BufReader<TcpStream>, r: Reply| {
        let _ = online::write_line(reader.get_mut(), &r);
    };
    let Ok(line) = online::read_line(&mut reader) else { return };
    let env: Envelope = match serde_json::from_str(&line) {
        Ok(e) => e,
        Err(_) => return reply(&mut reader, Reply::Error("bad request".into())),
    };
    if env.protocol != online::PROTOCOL {
        return reply(&mut reader, Reply::Error(format!("update taiko.rs (server protocol {}, yours {})", online::PROTOCOL, env.protocol)));
    }
    let out = match env.request {
        Request::Register { name, password } => register(state, ip, &name, &password),
        Request::Login { name, password } => login(state, ip, &name, &password),
        Request::WhoAmI { token } => match state.user(&token) {
            Some(name) => Reply::Account { name, token },
            None => Reply::Error("your session expired: log in again".into()),
        },
        Request::Logout { token } => {
            let mut a = state.accounts.lock().unwrap();
            if a.sessions.remove(&token).is_some() {
                state.save(&a);
            }
            Reply::LoggedOut
        }
        Request::ListRooms => Reply::Rooms(state.rooms()),
        Request::CreateRoom { token, title } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("log in to host a lobby".into())) };
            let mut lobbies = state.lobbies.lock().unwrap();
            if lobbies.len() >= MAX_ROOMS {
                return reply(&mut reader, Reply::Error("the server is full, try again later".into()));
            }
            let id = state.next_room.fetch_add(1, Ordering::Relaxed);
            let title = online::clean_title(&title, &name);
            let room = net::open_room();
            let host_token = room.token;
            println!("lobby {id} \"{title}\" opened by {name} ({ip})");
            reply(&mut reader, Reply::Joined { room: id, title: title.clone(), host_token: Some(host_token) });
            let _ = reader.get_ref().set_read_timeout(None);
            room.attach(reader, Some(name.clone()));
            lobbies.insert(id, Lobby { room, title, owner: name, empty_since: Some(Instant::now()) });
            return;
        }
        Request::Connect { token } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("your session expired: log in again".into())) };
            reply(&mut reader, Reply::Account { name: name.clone(), token });
            social_session(state, reader, name);
            return;
        }
        Request::SubmitScore { token, play } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("log in to send scores".into())) };
            let key = name.to_ascii_lowercase();
            let map = format!("{} - {} [{}]", play.map.artist, play.map.title, play.map.version);
            match state.scores.lock().unwrap().submit(&key, &name, *play, now_secs()) {
                Ok((map_rank, pp, rank)) => {
                    println!("score by {name} on {map}: #{map_rank:?}, {pp:.0} pp total");
                    Reply::Submitted { map_rank, pp, rank }
                }
                Err(e) => Reply::Error(e),
            }
        }
        Request::MapScores { map } => Reply::Scores(state.scores.lock().unwrap().map_scores(map)),
        Request::Replay { id } => match state.scores.lock().unwrap().replay(id) {
            Some(r) => Reply::Replay(Box::new(r)),
            None => Reply::Error("that replay isn't on the server".into()),
        },
        Request::Score { id } => match state.scores.lock().unwrap().score(id) {
            Some(s) => Reply::Score(Box::new(s)),
            None => Reply::Error("that score isn't on the server".into()),
        },
        Request::Profile { name } => match state.profile(&name) {
            Some(p) => Reply::Profile(Box::new(p)),
            None => Reply::Error("no such player".into()),
        },
        Request::JoinRoom { token, room } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("log in to join a lobby".into())) };
            let lobbies = state.lobbies.lock().unwrap();
            let Some(l) = lobbies.get(&room) else { return reply(&mut reader, Reply::Error("that lobby is gone".into())) };
            reply(&mut reader, Reply::Joined { room, title: l.title.clone(), host_token: None });
            let _ = reader.get_ref().set_read_timeout(None);
            l.room.attach(reader, Some(name));
            return;
        }
    };
    reply(&mut reader, out);
}

fn register(state: &State, ip: IpAddr, name: &str, password: &str) -> Reply {
    let name = name.trim();
    if !online::valid_name(name) {
        return Reply::Error(format!("names are {}", online::NAME_RULES));
    }
    if !online::valid_password(password) {
        return Reply::Error("the password needs at least 6 characters".into());
    }
    if !state.allow(ip, |l| &mut l.registered, Duration::from_secs(3600), REGISTER_PER_HOUR) {
        return Reply::Error("too many new accounts from here, try again later".into());
    }
    let key = name.to_ascii_lowercase();
    if state.accounts.lock().unwrap().users.contains_key(&key) {
        return Reply::Error("that name is taken".into());
    }
    // Hash outside the lock (it takes a moment), then check again.
    let Ok(hash) = hash_password(password) else { return Reply::Error("server error".into()) };
    let mut a = state.accounts.lock().unwrap();
    if a.users.contains_key(&key) {
        return Reply::Error("that name is taken".into());
    }
    a.users.insert(key.clone(), User { name: name.to_string(), hash, created: now_secs(), friends: Vec::new(), requests: Vec::new() });
    let token = state.new_session(&mut a, &key);
    state.save(&a);
    drop(a);
    state.record(ip, |l| &mut l.registered);
    println!("account {name} created ({ip})");
    Reply::Account { name: name.to_string(), token }
}

fn login(state: &State, ip: IpAddr, name: &str, password: &str) -> Reply {
    if !state.allow(ip, |l| &mut l.failed, Duration::from_secs(600), FAILED_LOGINS) {
        return Reply::Error("too many wrong passwords, wait a few minutes".into());
    }
    let key = name.trim().to_ascii_lowercase();
    let user = state.accounts.lock().unwrap().users.get(&key).cloned();
    let ok = verify_password(password, user.as_ref().map(|u| u.hash.as_str()).unwrap_or(&state.dummy_hash)) && user.is_some();
    let Some(user) = user.filter(|_| ok) else {
        state.record(ip, |l| &mut l.failed);
        std::thread::sleep(Duration::from_millis(400));
        return Reply::Error("wrong name or password".into());
    };
    let mut a = state.accounts.lock().unwrap();
    let token = state.new_session(&mut a, &key);
    state.save(&a);
    Reply::Account { name: user.name, token }
}

impl State {
    /// A player's profile, by their name in any case.
    fn profile(&self, name: &str) -> Option<online::Profile> {
        let key = name.trim().to_ascii_lowercase();
        let (name, joined) = self.accounts.lock().unwrap().users.get(&key).map(|u| (u.name.clone(), u.created))?;
        let mut p = self.scores.lock().unwrap().profile(&key, &name, joined);
        p.online = self.hub.lock().unwrap().conns.contains_key(&key);
        Some(p)
    }

    /// Everyone online with what they're doing, for the website.
    fn online_players(&self) -> Vec<OnlinePlayer> {
        match self.hub.lock().unwrap().players() {
            Down::Players { players, .. } => players,
            _ => Vec::new(),
        }
    }

    /// What `name` is doing, if they're online.
    fn activity(&self, name: &str) -> Option<Activity> {
        let key = name.trim().to_ascii_lowercase();
        self.hub.lock().unwrap().conns.get(&key).and_then(|v| v.last()).map(|c| c.activity.clone())
    }

    /// `name`'s friends as written, and whether each is online.
    fn friends_online(&self, name: &str) -> Vec<(String, bool)> {
        let key = name.trim().to_ascii_lowercase();
        // Names first: the accounts and the hub are never locked at once here.
        let named: Vec<(String, String)> = self.friend_keys(&key).into_iter().map(|k| (self.display_name(&k), k)).collect();
        let hub = self.hub.lock().unwrap();
        let mut out: Vec<(String, bool)> = named.into_iter().map(|(name, k)| (name, hub.conns.contains_key(&k))).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        out
    }

    fn display_name(&self, key: &str) -> String {
        self.accounts.lock().unwrap().users.get(key).map(|u| u.name.clone()).unwrap_or_else(|| key.to_string())
    }

    fn are_friends(&self, a: &str, b: &str) -> bool {
        self.accounts.lock().unwrap().users.get(a).is_some_and(|u| u.friends.iter().any(|f| f == b))
    }

    fn friend_keys(&self, key: &str) -> Vec<String> {
        self.accounts.lock().unwrap().users.get(key).map(|u| u.friends.clone()).unwrap_or_default()
    }

    /// The friend list as `key` sees it: friends with what they're doing, requests both ways.
    fn friends_view(&self, key: &str) -> Down {
        let (friends, requests, sent) = {
            let a = self.accounts.lock().unwrap();
            let Some(u) = a.users.get(key) else { return Down::Friends { friends: Vec::new(), requests: Vec::new(), sent: Vec::new() } };
            let name = |k: &String| a.users.get(k).map(|u| u.name.clone()).unwrap_or_else(|| k.clone());
            let friends: Vec<(String, String)> = u.friends.iter().map(|k| (k.clone(), name(k))).collect();
            let requests = u.requests.iter().map(name).collect();
            let sent = a.users.iter().filter(|(_, o)| o.requests.iter().any(|r| r == key)).map(|(_, o)| o.name.clone()).collect();
            (friends, requests, sent)
        };
        let hub = self.hub.lock().unwrap();
        let mut friends: Vec<Friend> = friends
            .into_iter()
            .map(|(k, name)| Friend { name, activity: hub.conns.get(&k).and_then(|v| v.last()).map(|c| c.activity.clone()) })
            .collect();
        friends.sort_by_key(|f| (f.activity.is_none(), f.name.to_lowercase()));
        Down::Friends { friends, requests, sent }
    }

    fn push_friends(&self, key: &str) {
        let view = self.friends_view(key);
        self.hub.lock().unwrap().send_to(key, &view);
    }

    /// Everyone who has `key` as a friend sees their new state.
    fn tell_friends(&self, key: &str) {
        for f in self.friend_keys(key) {
            self.push_friends(&f);
        }
    }

    fn notice(&self, key: &str, text: String) {
        self.hub.lock().unwrap().send_to(key, &Down::Notice(text));
    }

    /// Lines between `key` and `other`, oldest first.
    fn chat_with(&self, key: &str, other: &str) -> Vec<ChatLine> {
        let other = other.trim().to_ascii_lowercase();
        self.hub.lock().unwrap().history.get(&pair(key, &other)).map(|l| l.iter().cloned().collect()).unwrap_or_default()
    }

    fn global_chat(&self) -> Vec<ChatLine> {
        self.hub.lock().unwrap().global.iter().cloned().collect()
    }

    /// Who `key` can talk to: their friends, with whether each is online and their last line,
    /// the latest conversations first.
    fn conversations(&self, key: &str) -> Vec<pages::Conversation> {
        let named: Vec<(String, String)> = self.friend_keys(key).into_iter().map(|k| (self.display_name(&k), k)).collect();
        let hub = self.hub.lock().unwrap();
        let mut out: Vec<pages::Conversation> = named
            .into_iter()
            .map(|(name, k)| {
                let last = hub.history.get(&pair(key, &k)).and_then(|l| l.back().cloned());
                pages::Conversation { online: hub.conns.contains_key(&k), name, last }
            })
            .collect();
        out.sort_by(|a, b| {
            let at = |c: &pages::Conversation| c.last.as_ref().map(|l| l.at).unwrap_or(0);
            at(b).cmp(&at(a)).then(b.online.cmp(&a.online)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        out
    }

    /// How `viewer` and `other` (keys) stand.
    fn friendship(&self, viewer: &str, other: &str) -> pages::Friendship {
        let a = self.accounts.lock().unwrap();
        let (Some(me), Some(them)) = (a.users.get(viewer), a.users.get(other)) else { return pages::Friendship::None };
        if viewer == other {
            pages::Friendship::Me
        } else if me.friends.iter().any(|f| f == other) {
            pages::Friendship::Friends
        } else if me.requests.iter().any(|r| r == other) {
            pages::Friendship::TheyAsked
        } else if them.requests.iter().any(|r| r == viewer) {
            pages::Friendship::Asked
        } else {
            pages::Friendship::None
        }
    }

    /// `key` (`name` as written) asks `other` to be friends, or accepts their request. Tells
    /// both what happened; the message for `key` either way.
    fn add_friend(&self, key: &str, name: &str, other: &str) -> Result<String, String> {
        let other = other.trim().to_ascii_lowercase();
        let outcome = {
            let mut a = self.accounts.lock().unwrap();
            if other == key {
                Err("that's you".to_string())
            } else if !a.users.contains_key(&other) {
                Err(format!("there's no player named {other}"))
            } else if a.users[key].friends.contains(&other) {
                Err(format!("{} is already your friend", a.users[&other].name))
            } else if a.users[key].friends.len() >= MAX_FRIENDS {
                Err("your friend list is full".to_string())
            } else if a.users[key].requests.contains(&other) {
                // They asked first: now friends.
                let me = a.users.get_mut(key).unwrap();
                me.requests.retain(|r| *r != other);
                me.friends.push(other.clone());
                a.users.get_mut(&other).unwrap().friends.push(key.to_string());
                self.save(&a);
                Ok((true, a.users[&other].name.clone()))
            } else {
                let them = a.users.get_mut(&other).unwrap();
                if !them.requests.iter().any(|r| r == key) {
                    them.requests.push(key.to_string());
                }
                self.save(&a);
                Ok((false, a.users[&other].name.clone()))
            }
        };
        self.push_friends(key);
        self.push_friends(&other);
        match outcome? {
            (true, other_name) => {
                self.notice(&other, format!("{name} accepted your friend request"));
                Ok(format!("you and {other_name} are friends now"))
            }
            (false, other_name) => {
                self.notice(&other, format!("{name} sent you a friend request"));
                Ok(format!("friend request sent to {other_name}"))
            }
        }
    }

    /// A message from `key` (`name` as written) to their friend `to`, if `allowed` (the sender's
    /// rate limit) lets it through: kept with their chat and sent to both, game and website.
    fn post_chat(&self, key: &str, name: &str, to: &str, text: &str, allowed: impl FnOnce(&mut Hub) -> bool) -> Result<ChatLine, String> {
        let to = to.trim().to_ascii_lowercase();
        if !self.are_friends(key, &to) {
            return Err("you can only message friends".into());
        }
        let text = net::clean_chat(text);
        if text.is_empty() {
            return Err("nothing to send".into());
        }
        let to_name = self.display_name(&to);
        let mut hub = self.hub.lock().unwrap();
        if !allowed(&mut hub) {
            return Err("slow down a little".into());
        }
        let line = ChatLine { from: name.to_string(), to: to_name, text, at: now_secs() };
        let log = hub.history.entry(pair(key, &to)).or_default();
        log.push_back(line.clone());
        if log.len() > CHAT_HISTORY {
            log.pop_front();
        }
        let d = Down::Chat(line.clone());
        hub.send_to(key, &d);
        hub.send_to(&to, &d);
        Ok(line)
    }

    /// A line in the global chat from `name`, if `allowed` lets it through.
    fn post_global(&self, name: &str, text: &str, allowed: impl FnOnce(&mut Hub) -> bool) -> Result<ChatLine, String> {
        // The global chat is public: starred out for everyone, old clients included.
        let text = net::censor(&net::clean_chat(text));
        if text.is_empty() {
            return Err("nothing to send".into());
        }
        let mut hub = self.hub.lock().unwrap();
        if !allowed(&mut hub) {
            return Err("slow down a little".into());
        }
        let line = ChatLine { from: name.to_string(), to: String::new(), text, at: now_secs() };
        hub.global.push_back(line.clone());
        if hub.global.len() > CHAT_HISTORY {
            hub.global.pop_front();
        }
        hub.send_to_all(&Down::GlobalChat(line.clone()));
        Ok(line)
    }
}

/// A player's social connection: friend list, chat, watching and streaming solo plays.
fn social_session(state: &State, mut reader: BufReader<TcpStream>, name: String) {
    let key = name.to_ascii_lowercase();
    let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(90)));
    let Ok(mut writer) = reader.get_ref().try_clone() else { return };
    let (tx, rx) = mpsc::channel::<Down>();
    // A quiet connection gets a Pong every 20 s anyway: games that only ping while they send
    // nothing else went 70 s without hearing from us during a solo play and dropped the link.
    std::thread::spawn(move || {
        loop {
            let d = match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(d) => d,
                Err(mpsc::RecvTimeoutError::Timeout) => Down::Pong,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if online::write_line(&mut writer, &d).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });
    let id = {
        let mut hub = state.hub.lock().unwrap();
        hub.next_conn += 1;
        let id = hub.next_conn;
        hub.conns.entry(key.clone()).or_default().push(Conn { id, name: name.clone(), tx: tx.clone(), activity: Activity::Menus, watching: None, chat_times: VecDeque::new() });
        hub.players_changed = true;
        let _ = tx.send(hub.players());
        for line in &hub.global {
            let _ = tx.send(Down::GlobalChat(line.clone()));
        }
        id
    };
    state.push_friends(&key);
    {
        let friends = state.friend_keys(&key);
        let hub = state.hub.lock().unwrap();
        for f in friends {
            for line in hub.history.get(&pair(&key, &f)).into_iter().flatten() {
                let _ = tx.send(Down::Chat(line.clone()));
            }
        }
    }
    state.tell_friends(&key);
    while let Ok(line) = online::read_line(&mut reader) {
        let Ok(up) = serde_json::from_str::<Up>(&line) else { continue };
        social_up(state, &key, &name, id, &tx, up);
    }
    {
        let mut hub = state.hub.lock().unwrap();
        if let Some(v) = hub.conns.get_mut(&key) {
            v.retain(|c| c.id != id);
            if v.is_empty() {
                hub.conns.remove(&key);
            }
        }
        hub.players_changed = true;
        if hub.plays.get(&key).is_some_and(|p| p.conn == id) {
            hub.plays.remove(&key);
            hub.send_to_watchers(&key, &Down::WatchEnd { player: name.clone() });
        }
    }
    state.tell_friends(&key);
}

fn social_up(state: &State, key: &str, name: &str, id: u64, tx: &mpsc::Sender<Down>, up: Up) {
    match up {
        Up::Ping => {
            let _ = tx.send(Down::Pong);
        }
        Up::Status(activity) => {
            {
                let mut hub = state.hub.lock().unwrap();
                if let Some(c) = hub.conn(key, id) {
                    c.activity = activity;
                }
                hub.players_changed = true;
            }
            state.tell_friends(key);
        }
        Up::AddFriend(other) => {
            let msg = state.add_friend(key, name, &other).unwrap_or_else(|e| e);
            state.notice(key, msg);
        }
        Up::RemoveFriend(other) => {
            let other = other.trim().to_ascii_lowercase();
            {
                let mut a = state.accounts.lock().unwrap();
                if let Some(me) = a.users.get_mut(key) {
                    me.friends.retain(|f| *f != other);
                    me.requests.retain(|r| *r != other);
                }
                if let Some(them) = a.users.get_mut(&other) {
                    them.friends.retain(|f| f != key);
                    them.requests.retain(|r| r != key);
                }
                state.save(&a);
            }
            state.push_friends(key);
            state.push_friends(&other);
        }
        Up::Chat { to, text } => {
            if let Err(e) = state.post_chat(key, name, &to, &text, |hub| hub.chat_allowed(key, id)) {
                state.notice(key, e);
            }
        }
        Up::GlobalChat(text) => {
            let _ = state.post_global(name, &text, |hub| hub.chat_allowed(key, id));
        }
        Up::Watch(other) => {
            let other = other.trim().to_ascii_lowercase();
            if !state.are_friends(key, &other) {
                let _ = tx.send(Down::WatchStopped("you can only watch friends".into()));
                return;
            }
            let other_name = state.display_name(&other);
            let mut hub = state.hub.lock().unwrap();
            if let Some(c) = hub.conn(key, id) {
                c.watching = Some(other.clone());
            }
            match hub.plays.get(&other) {
                Some(p) => {
                    let _ = tx.send(Down::WatchStart { player: other_name, play: p.play.clone(), t: p.t, paused: p.paused, inputs: p.inputs.clone() });
                }
                None => {
                    let _ = tx.send(Down::WatchEnd { player: other_name });
                }
            }
        }
        Up::StopWatching => {
            if let Some(c) = state.hub.lock().unwrap().conn(key, id) {
                c.watching = None;
            }
        }
        Up::PlayStart(play) => {
            let mut hub = state.hub.lock().unwrap();
            // Before its first frame (JSON has no infinity: far before any map starts).
            let t = play.map.from.unwrap_or(-1.0e9);
            let d = Down::WatchStart { player: name.to_string(), play: play.clone(), t, paused: false, inputs: Vec::new() };
            hub.plays.insert(key.to_string(), Play { play, inputs: Vec::new(), t, paused: false, conn: id });
            hub.send_to_watchers(key, &d);
        }
        Up::PlayFrame { t, paused, inputs } => {
            let mut hub = state.hub.lock().unwrap();
            let Some(p) = hub.plays.get_mut(key).filter(|p| p.conn == id) else { return };
            p.t = t;
            p.paused = paused;
            p.inputs.extend_from_slice(&inputs);
            hub.send_to_watchers(key, &Down::WatchFrame { t, paused, inputs });
        }
        Up::PlayEnd => {
            let mut hub = state.hub.lock().unwrap();
            if hub.plays.get(key).is_some_and(|p| p.conn == id) {
                hub.plays.remove(key);
                hub.send_to_watchers(key, &Down::WatchEnd { player: name.to_string() });
            }
        }
    }
}

/// Closes lobbies that have been empty for a while.
fn reaper(state: Arc<State>) {
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let mut lobbies = state.lobbies.lock().unwrap();
        lobbies.retain(|id, l| {
            let players = l.room.summary.lock().unwrap().players;
            if players > 0 {
                l.empty_since = None;
                return true;
            }
            let since = *l.empty_since.get_or_insert_with(Instant::now);
            let keep = since.elapsed() < EMPTY_ROOM;
            if !keep {
                println!("lobby {id} \"{}\" closed (empty)", l.title);
            }
            keep
        });
    }
}

/// Sends the online list to everyone when it has changed.
fn announcer(state: Arc<State>) {
    loop {
        std::thread::sleep(PLAYERS_EVERY);
        let mut hub = state.hub.lock().unwrap();
        if std::mem::take(&mut hub.players_changed) {
            let d = hub.players();
            hub.send_to_all(&d);
        }
    }
}

fn main() {
    let mut port = online::DEFAULT_PORT;
    let mut data = PathBuf::from(".");
    let mut http_port = 0;
    let mut behind_proxy = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).expect("--port N"),
            "--data" => data = PathBuf::from(args.next().expect("--data DIR")),
            "--http-port" => http_port = args.next().and_then(|p| p.parse().ok()).expect("--http-port N"),
            "--behind-proxy" => behind_proxy = true,
            _ => {
                eprintln!("usage: taiko-server [--port 7800] [--data DIR] [--http-port 8080 [--behind-proxy]]");
                std::process::exit(2);
            }
        }
    }
    std::fs::create_dir_all(&data).expect("data dir");
    covers::init(data.clone());
    let state = Arc::new(State::load(data.join("accounts.json")));
    if http_port != 0 {
        web::serve(state.clone(), http_port, behind_proxy);
    }
    discord::start(state.clone(), data.clone());
    {
        let state = state.clone();
        std::thread::spawn(move || reaper(state));
    }
    {
        let state = state.clone();
        std::thread::spawn(move || announcer(state));
    }
    // IPv6 and IPv4 (on Linux `::` is usually dual-stack and the second bind just fails).
    let mut listeners = Vec::new();
    let mut errors = Vec::new();
    for addr in ["::", "0.0.0.0"] {
        match TcpListener::bind((addr, port)) {
            Ok(l) => listeners.push(l),
            Err(e) => errors.push(format!("[{addr}]:{port}: {e}")),
        }
    }
    assert!(!listeners.is_empty(), "can't listen: {}", errors.join(", "));
    println!("taiko-server listening on port {port} (protocol {}, rooms protocol {})", online::PROTOCOL, net::PROTOCOL);
    let threads: Vec<_> = listeners
        .into_iter()
        .map(|l| {
            let state = state.clone();
            std::thread::spawn(move || {
                for stream in l.incoming().flatten() {
                    let state = state.clone();
                    std::thread::spawn(move || handle(&state, stream));
                }
            })
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
}
