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

// The game's `tr!`, for the text in its files below (the server always speaks English).
#[allow(dead_code)]
#[macro_use]
#[path = "../../src/lang.rs"]
mod lang;
#[allow(dead_code)]
#[path = "../../src/net.rs"]
mod net;
#[allow(dead_code)]
#[path = "../../src/online.rs"]
mod online;
#[path = "../../src/tls.rs"]
mod tls;
mod avatars;
mod card;
mod covers;
mod osustatus;
mod discord;
#[allow(dead_code)]
#[path = "../../src/font.rs"]
mod font;
mod loved;
mod mail;
mod scores;
mod pages;
mod web;

use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, Read, Write};
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
/// Per IP: codes mailed per hour.
const MAILS_PER_HOUR: usize = 10;

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
    /// May love maps (see `loved`); admins give and take it.
    #[serde(default)]
    nominator: bool,
    /// Where codes for resetting the password go (lowercase; accounts from before emails have
    /// none until linked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
}

/// Admins (lowercased names): they make loved nominators, and nominate themselves.
const ADMINS: &[&str] = &["lynerrr", "auneu"];

fn is_admin(key: &str) -> bool {
    ADMINS.contains(&key)
}

/// Shown online all the time (in the menus), whether connected or not.
const ALWAYS_ONLINE: &str = "claude";
/// Claude's friends (made at startup, both ways, once their accounts exist).
const CLAUDE_FRIENDS: &[&str] = &["lynerrr", "auneu"];

/// How long a code for linking Discord to an account (`>verify`) lasts.
const VERIFY_FOR: Duration = Duration::from_secs(30 * 60);

const MAX_FRIENDS: usize = 200;
/// Chat lines kept per pair of friends, and of the global chat (in memory only).
const CHAT_HISTORY: usize = 100;
/// How often a changed online list goes out to everyone (at most; see `announcer`).
const PLAYERS_EVERY: Duration = Duration::from_secs(2);
/// How often pp is counted again as map sets' statuses become known.
const COUNT_PP_EVERY: Duration = Duration::from_secs(60);

/// One message of JSON with its newline, made once and shared by every connection it goes to.
type Line = Arc<str>;

fn line(d: &Down) -> Line {
    let mut s = serde_json::to_string(d).unwrap_or_default();
    s.push('\n');
    s.into()
}

/// The sending end of a social connection (its writer thread writes what comes through).
#[derive(Clone)]
struct Tx(mpsc::Sender<Line>);

impl Tx {
    fn send(&self, d: Down) -> Result<(), mpsc::SendError<Line>> {
        self.0.send(line(&d))
    }

    fn send_line(&self, l: &Line) -> Result<(), mpsc::SendError<Line>> {
        self.0.send(l.clone())
    }

    /// Has the writer close the connection, after what's already queued (an empty line: real
    /// ones end in a newline).
    fn close(&self) {
        let _ = self.0.send(Line::from(""));
    }
}

/// One open social connection (a player can have several, e.g. two PCs).
struct Conn {
    id: u64,
    /// The account's name as written.
    name: String,
    /// The session it was opened with: logging that out closes it.
    token: String,
    tx: Tx,
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
    /// How many connections watch each player (lowercased name), so a play's frames are only
    /// looked at for watchers when there are some.
    watchers: HashMap<String, u32>,
}

impl Hub {
    fn send_to(&self, key: &str, d: &Down) {
        let l = line(d);
        for c in self.conns.get(key).into_iter().flatten() {
            let _ = c.tx.send_line(&l);
        }
    }

    fn send_to_watchers(&self, key: &str, d: &Down) {
        if !self.watchers.contains_key(key) {
            return;
        }
        let l = line(d);
        for c in self.conns.values().flatten().filter(|c| c.watching.as_deref() == Some(key)) {
            let _ = c.tx.send_line(&l);
        }
    }

    /// Points connection `id` of `key` at the player `whom` watches (or at nobody).
    fn set_watching(&mut self, key: &str, id: u64, whom: Option<String>) {
        let Some(c) = self.conn(key, id) else { return };
        if let Some(old) = std::mem::replace(&mut c.watching, whom.clone()) {
            self.unwatch(&old);
        }
        if let Some(w) = whom {
            *self.watchers.entry(w).or_default() += 1;
        }
    }

    fn unwatch(&mut self, whom: &str) {
        if let Some(n) = self.watchers.get_mut(whom) {
            *n -= 1;
            if *n == 0 {
                self.watchers.remove(whom);
            }
        }
    }

    /// Stops `key` watching `whom` on all of `key`'s connections.
    fn stop_watching(&mut self, key: &str, whom: &str) {
        let ids: Vec<u64> = self.conns.get(key).into_iter().flatten().filter(|c| c.watching.as_deref() == Some(whom)).map(|c| c.id).collect();
        for id in ids {
            self.set_watching(key, id, None);
            if let Some(c) = self.conn(key, id) {
                let _ = c.tx.send(Down::WatchStopped("you can only watch friends".into()));
            }
        }
    }

    /// Closes every connection of `key`.
    fn close_account(&self, key: &str) {
        for c in self.conns.get(key).into_iter().flatten() {
            c.tx.close();
        }
    }

    /// Closes every connection opened with `token`.
    fn close_session(&self, token: &str) {
        for c in self.conns.values().flatten().filter(|c| c.token == token) {
            c.tx.close();
        }
    }

    fn conn(&mut self, key: &str, id: u64) -> Option<&mut Conn> {
        self.conns.get_mut(key)?.iter_mut().find(|c| c.id == id)
    }

    fn send_to_all(&self, d: &Down) {
        let l = line(d);
        for c in self.conns.values().flatten() {
            let _ = c.tx.send_line(&l);
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
        if !self.conns.contains_key(ALWAYS_ONLINE) {
            players.push(OnlinePlayer { name: ALWAYS_ONLINE.into(), activity: Activity::Menus });
        }
        players.sort_by_cached_key(|p| p.name.to_lowercase());
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

#[derive(Serialize, Deserialize, Clone)]
struct Session {
    /// Lowercased account name (the key in `users`).
    user: String,
    created: u64,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Accounts {
    /// By lowercased name: names are unique ignoring case.
    users: HashMap<String, User>,
    sessions: HashMap<String, Session>,
    /// Whom each account has asked to be friends: `requests` the other way round, kept in
    /// memory so a friend list doesn't go through every account.
    #[serde(skip)]
    asked: HashMap<String, Vec<String>>,
}

impl Accounts {
    fn index_requests(&mut self) {
        self.asked.clear();
        for (key, u) in &self.users {
            for from in &u.requests {
                self.asked.entry(from.clone()).or_default().push(key.clone());
            }
        }
    }

    /// `from` asks `to` to be friends (once).
    fn ask(&mut self, from: &str, to: &str) {
        let Some(them) = self.users.get_mut(to) else { return };
        if !them.requests.iter().any(|r| r == from) {
            them.requests.push(from.to_string());
            self.asked.entry(from.to_string()).or_default().push(to.to_string());
        }
    }

    /// Drops `from`'s request to `to`, if there is one.
    fn unask(&mut self, from: &str, to: &str) {
        if let Some(them) = self.users.get_mut(to) {
            them.requests.retain(|r| r != from);
        }
        if let Some(v) = self.asked.get_mut(from) {
            v.retain(|k| k != to);
            if v.is_empty() {
                self.asked.remove(from);
            }
        }
    }
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
    /// Codes mailed at this address's asking.
    mails: Vec<Instant>,
}

struct State {
    accounts: Mutex<Accounts>,
    path: PathBuf,
    /// Wakes `saver` to write accounts.json.
    saves: mpsc::Sender<()>,
    lobbies: Mutex<HashMap<u32, Lobby>>,
    next_room: AtomicU32,
    limits: Mutex<HashMap<IpAddr, Limits>>,
    /// Verified against when the name doesn't exist, so a login takes as long either way.
    dummy_hash: String,
    hub: Mutex<Hub>,
    scores: Mutex<scores::Scores>,
    /// Codes for `>verify` on Discord: code -> (lowercased name, when made).
    verify_codes: Mutex<HashMap<String, (String, Instant)>>,
}

/// What's saved at `path`: the default when there's no such file yet, an error when there is
/// one and it can't be read (starting empty then would overwrite it with nothing).
fn try_load<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> Result<T, String> {
    match try_text(path)? {
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        None => Ok(T::default()),
    }
}

/// The file at `path`, None when there's no such file yet.
fn try_text(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `try_load`, or the server doesn't start: better down than with its data gone.
fn load_saved<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    try_load(path).unwrap_or_else(|e| refuse_to_start(&e))
}

/// `try_text`, or the server doesn't start.
fn load_text(path: &std::path::Path) -> Option<String> {
    try_text(path).unwrap_or_else(|e| refuse_to_start(&e))
}

fn refuse_to_start(why: &str) -> ! {
    eprintln!("can't read saved data, not starting: {why}");
    std::process::exit(1);
}

/// What limits count by: the address, or for IPv6 its /64, which one home has all of.
fn limit_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6((u128::from(v6) & (!0u128 << 64)).into()),
        },
        v4 => v4,
    }
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
    /// The state, and what wakes the saver (see `saver`).
    fn load(path: PathBuf) -> (State, mpsc::Receiver<()>) {
        let mut accounts: Accounts = load_saved(&path);
        let cutoff = now_secs().saturating_sub(SESSION_DAYS * 86400);
        accounts.sessions.retain(|_, s| s.created >= cutoff);
        // Claude's profile (its badge is in `pages`): a password nobody knows, so nobody logs in.
        let mut save = !accounts.users.contains_key("claude");
        if save {
            let hash = hash_password(&new_token()).expect("argon2");
            accounts.users.insert("claude".into(), User { name: "claude".into(), hash, created: now_secs(), friends: Vec::new(), requests: Vec::new(), nominator: false, email: None });
        }
        // And its friends, both ways.
        for &friend in CLAUDE_FRIENDS {
            if !accounts.users.contains_key(friend) || accounts.users["claude"].friends.iter().any(|f| f == friend) {
                continue;
            }
            for (a, b) in [("claude", friend), (friend, "claude")] {
                if let Some(u) = accounts.users.get_mut(a) {
                    u.friends.retain(|f| f != b);
                    u.friends.push(b.to_string());
                    u.requests.retain(|r| r != b);
                }
            }
            save = true;
        }
        accounts.index_requests();
        println!("{} accounts, {} sessions ({})", accounts.users.len(), accounts.sessions.len(), path.display());
        // Scores live next to the accounts.
        let scores = scores::Scores::load(Some(path.parent().map(|d| d.to_path_buf()).unwrap_or_default()));
        let (saves, wake) = mpsc::channel();
        let state = State {
            accounts: Mutex::new(accounts),
            path,
            saves,
            lobbies: Mutex::new(HashMap::new()),
            next_room: AtomicU32::new(1),
            limits: Mutex::new(HashMap::new()),
            dummy_hash: hash_password("not a real password").expect("argon2"),
            hub: Mutex::new(Hub::default()),
            scores: Mutex::new(scores),
            verify_codes: Mutex::new(HashMap::new()),
        };
        if save {
            state.save();
        }
        (state, wake)
    }

    /// Has accounts.json written (by `saver`, right away but outside the lock).
    fn save(&self) {
        let _ = self.saves.send(());
    }

    /// Writes accounts.json atomically (temp file, then rename), readable by its owner only.
    fn write(&self, accounts: &Accounts) {
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
            Reply::Account { name, token, .. } => Ok((name, token)),
            Reply::Error(e) => Err(e),
            _ => Err("couldn't log in".into()),
        }
    }

    fn logout(&self, token: &str) {
        {
            let mut a = self.accounts.lock().unwrap();
            if a.sessions.remove(token).is_some() {
                self.save();
            }
        }
        self.hub.lock().unwrap().close_session(token);
    }

    /// The account name behind a session token.
    fn user(&self, token: &str) -> Option<String> {
        let a = self.accounts.lock().unwrap();
        let s = a.sessions.get(token)?;
        a.users.get(&s.user).map(|u| u.name.clone())
    }

    /// The reply for a logged-in `name` (as written) with session `token`.
    fn account(&self, name: String, token: String) -> Reply {
        let key = name.to_ascii_lowercase();
        let email = self.accounts.lock().unwrap().users.get(&key).map(|u| u.email.is_some());
        Reply::Account { nominator: self.is_nominator(&key), name, token, email }
    }

    /// Mails a code (`mail::send_code`), a few an hour per address.
    fn mail_code(&self, ip: IpAddr, for_: &str, email: &str, what: &str) -> Result<(), String> {
        let Some(slot) = self.take_slot(ip, |l| &mut l.mails, Duration::from_secs(3600), MAILS_PER_HOUR) else {
            return Err("too many codes asked for from here, try again in an hour".into());
        };
        let sent = mail::send_code(for_, email, what);
        if sent.is_err() {
            self.give_back(ip, |l| &mut l.mails, slot);
        }
        sent
    }

    /// The account (lowercased name) `email` belongs to.
    fn email_owner(&self, email: &str) -> Option<String> {
        let email = mail::normal(email);
        self.accounts.lock().unwrap().users.iter().find(|(_, u)| u.email.as_deref() == Some(email.as_str())).map(|(k, _)| k.clone())
    }

    fn new_session(&self, accounts: &mut Accounts, key: &str) -> String {
        let token = new_token();
        accounts.sessions.insert(token.clone(), Session { user: key.to_string(), created: now_secs() });
        token
    }

    /// Takes one of `max` slots `ip` has inside `window`, None when they're gone. Taken before
    /// the slow work: requests arriving during it have to see this one.
    fn take_slot(&self, ip: IpAddr, pick: fn(&mut Limits) -> &mut Vec<Instant>, window: Duration, max: usize) -> Option<Instant> {
        let mut limits = self.limits.lock().unwrap();
        let list = pick(limits.entry(limit_key(ip)).or_default());
        list.retain(|t| t.elapsed() < window);
        if list.len() >= max {
            return None;
        }
        let slot = Instant::now();
        list.push(slot);
        Some(slot)
    }

    /// Gives a slot back: what it was taken for didn't count after all.
    fn give_back(&self, ip: IpAddr, pick: fn(&mut Limits) -> &mut Vec<Instant>, slot: Instant) {
        let mut limits = self.limits.lock().unwrap();
        let list = pick(limits.entry(limit_key(ip)).or_default());
        if let Some(i) = list.iter().position(|t| *t == slot) {
            list.remove(i);
        }
    }

    fn rooms(&self) -> Vec<RoomInfo> {
        let lobbies = self.lobbies.lock().unwrap();
        let mut out: Vec<RoomInfo> = lobbies
            .iter()
            .filter_map(|(&id, l)| {
                let s = l.room.summary.lock().unwrap().clone();
                (s.players > 0).then(|| RoomInfo { id, title: l.title.clone(), host: if s.host.is_empty() { l.owner.clone() } else { s.host }, players: s.players, map: s.map, in_match: s.in_match, mapguessr: s.mapguessr })
            })
            .collect();
        out.sort_by_key(|r| r.id);
        out
    }
}

/// Handles one connection: a request, its reply, and for a lobby the hand-over to the room.
fn handle(state: &State, stream: TcpStream, slot: Slot, ip: IpAddr) {
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
        Request::Register { name, password, email, code } => register(state, ip, &name, &password, email.as_deref(), code.as_deref()),
        Request::Login { name, password } => login(state, ip, &name, &password),
        Request::ResetPassword { who, code, password } => reset_password(state, ip, &who, code.as_deref(), password.as_deref()),
        Request::LinkEmail { token, email, code } => link_email(state, ip, &token, &email, code.as_deref()),
        Request::WhoAmI { token } => match state.user(&token) {
            Some(name) => state.account(name, token),
            None => Reply::Error("your session expired: log in again".into()),
        },
        Request::Logout { token } => {
            state.logout(&token);
            Reply::LoggedOut
        }
        Request::ListRooms => Reply::Rooms(state.rooms()),
        Request::CreateRoom { token, title, mapguessr } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("log in to host a lobby".into())) };
            let mut lobbies = state.lobbies.lock().unwrap();
            if lobbies.len() >= MAX_ROOMS {
                return reply(&mut reader, Reply::Error("the server is full, try again later".into()));
            }
            let id = state.next_room.fetch_add(1, Ordering::Relaxed);
            let title = online::clean_title(&title, &name, mapguessr);
            let room = net::open_room(mapguessr);
            let host_token = room.token;
            println!("lobby {id} \"{title}\" opened by {name} ({ip})");
            reply(&mut reader, Reply::Joined { room: id, title: title.clone(), host_token: Some(host_token) });
            let _ = reader.get_ref().set_read_timeout(None);
            room.attach(reader, Some(name.clone()), slot);
            lobbies.insert(id, Lobby { room, title, owner: name, empty_since: Some(Instant::now()) });
            return;
        }
        Request::Connect { token } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("your session expired: log in again".into())) };
            reply(&mut reader, state.account(name.clone(), token.clone()));
            social_session(state, reader, name, token);
            return;
        }
        Request::SubmitScore { token, play } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("log in to send scores".into())) };
            let key = name.to_ascii_lowercase();
            let map = format!("{} - {} [{}]", play.map.artist, play.map.title, play.map.version);
            // Whether the map gives pp, so the total sent back is right.
            covers::learn(play.map.set_id);
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
        Request::Played { token, ms } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("your session expired: log in again".into())) };
            match state.scores.lock().unwrap().played(&name.to_ascii_lowercase(), ms) {
                Ok(()) => Reply::Counted,
                Err(e) => Reply::Error(e),
            }
        }
        // What the server knows of the sets (asking osu! in the background about the rest, for
        // logged-in players only: not anyone can spend the API's allowance).
        Request::SetStatuses { token, maps } => {
            let ask = state.user(&token).is_some();
            let maps: Vec<(u32, u32)> = maps.into_iter().take(online::MAX_STATUS_MAPS).collect();
            Reply::SetStatuses(osustatus::lookup(&maps, ask))
        }
        Request::LovedSets => Reply::LovedSets(loved::all().into_iter().map(|(set, _)| set).collect()),
        Request::SetLoved { token, set, loved: on } => {
            let Some(name) = state.user(&token) else { return reply(&mut reader, Reply::Error("your session expired: log in again".into())) };
            match state.love(&name.to_ascii_lowercase(), set, on) {
                Ok(_) => Reply::LovedSets(loved::all().into_iter().map(|(set, _)| set).collect()),
                Err(e) => Reply::Error(e),
            }
        }
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
            l.room.attach(reader, Some(name), slot);
            return;
        }
    };
    reply(&mut reader, out);
}

fn register(state: &State, ip: IpAddr, name: &str, password: &str, email: Option<&str>, code: Option<&str>) -> Reply {
    let name = name.trim();
    if !online::valid_name(name) {
        return Reply::Error(format!("names are {}", online::NAME_RULES));
    }
    if !online::valid_password(password) {
        return Reply::Error("the password needs at least 6 characters".into());
    }
    let Some(slot) = state.take_slot(ip, |l| &mut l.registered, Duration::from_secs(3600), REGISTER_PER_HOUR) else {
        return Reply::Error("too many new accounts from here, try again later".into());
    };
    let reply = create_account(state, ip, name, password, email, code);
    if !matches!(reply, Reply::Account { .. }) {
        state.give_back(ip, |l| &mut l.registered, slot);
    }
    reply
}

fn create_account(state: &State, ip: IpAddr, name: &str, password: &str, email: Option<&str>, code: Option<&str>) -> Reply {
    let key = name.to_ascii_lowercase();
    if state.accounts.lock().unwrap().users.contains_key(&key) {
        return Reply::Error("that name is taken".into());
    }
    // Where mail goes out, an account comes with an email, proven by a code sent there.
    let mut verified = None;
    if mail::on() {
        let Some(email) = email.map(mail::normal).filter(|e| !e.is_empty()) else {
            return Reply::Error("an email is needed for a new account (update taiko.rs if there's no field for it)".into());
        };
        if !mail::valid_email(&email) {
            return Reply::Error("that email doesn't look right".into());
        }
        if state.email_owner(&email).is_some() {
            return Reply::Error("that email is already used by an account".into());
        }
        let for_ = format!("reg:{key}");
        match code.map(str::trim).filter(|c| !c.is_empty()) {
            None => {
                return match state.mail_code(ip, &for_, &email, &format!("for creating the account {name}")) {
                    Ok(()) => {
                        println!("sign-up code for {name} sent ({ip})");
                        Reply::CodeSent(mail::masked(&email))
                    }
                    Err(e) => Reply::Error(e),
                };
            }
            Some(code) => {
                if mail::pending_email(&for_).is_some_and(|e| e != email) {
                    return Reply::Error("the code was sent to another email: ask for a new one".into());
                }
                match mail::check_code(&for_, code) {
                    Ok(e) => verified = Some(e),
                    Err(e) => return Reply::Error(e),
                }
            }
        }
    }
    // Hash outside the lock (it takes a moment), then check again.
    let Ok(hash) = hash_password(password) else { return Reply::Error("server error".into()) };
    let mut a = state.accounts.lock().unwrap();
    if a.users.contains_key(&key) {
        return Reply::Error("that name is taken".into());
    }
    let has_email = verified.is_some();
    a.users.insert(key.clone(), User { name: name.to_string(), hash, created: now_secs(), friends: Vec::new(), requests: Vec::new(), nominator: false, email: verified });
    let token = state.new_session(&mut a, &key);
    state.save();
    drop(a);
    println!("account {name} created ({ip})");
    Reply::Account { name: name.to_string(), token, nominator: is_admin(&key), email: Some(has_email) }
}

/// A forgotten password (see `Request::ResetPassword`).
fn reset_password(state: &State, ip: IpAddr, who: &str, code: Option<&str>, password: Option<&str>) -> Reply {
    let Some(slot) = state.take_slot(ip, |l| &mut l.failed, Duration::from_secs(600), FAILED_LOGINS) else {
        return Reply::Error("too many tries, wait a few minutes".into());
    };
    let mut failed = false;
    let reply = try_reset_password(state, ip, who, code, password, &mut failed);
    if !failed {
        state.give_back(ip, |l| &mut l.failed, slot);
    }
    reply
}

/// Sets `failed` when this try counts against the address's limit.
fn try_reset_password(state: &State, ip: IpAddr, who: &str, code: Option<&str>, password: Option<&str>, failed: &mut bool) -> Reply {
    let who = who.trim();
    let key = if who.contains('@') { state.email_owner(who) } else { Some(who.to_ascii_lowercase()) };
    let user = key.as_ref().and_then(|k| state.accounts.lock().unwrap().users.get(k).cloned());
    let (Some(key), Some(user)) = (key, user) else {
        *failed = true;
        return Reply::Error("no account by that name or email".into());
    };
    let Some(email) = user.email.clone() else {
        return Reply::Error("that account has no email linked: ask an admin on Discord".into());
    };
    let for_ = format!("reset:{key}");
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        return match state.mail_code(ip, &for_, &email, &format!("for a new password for {}", user.name)) {
            Ok(()) => {
                println!("password reset code for {} sent ({ip})", user.name);
                Reply::CodeSent(mail::masked(&email))
            }
            Err(e) => Reply::Error(e),
        };
    };
    let Some(password) = password.filter(|p| online::valid_password(p)) else {
        return Reply::Error("type a new password of at least 6 characters with the code".into());
    };
    if let Err(e) = mail::check_code(&for_, code) {
        *failed = true;
        return Reply::Error(e);
    }
    let Ok(hash) = hash_password(password) else { return Reply::Error("server error".into()) };
    let mut a = state.accounts.lock().unwrap();
    let Some(u) = a.users.get_mut(&key) else { return Reply::Error("no account by that name or email".into()) };
    u.hash = hash;
    // Whoever had the old password is logged out everywhere.
    a.sessions.retain(|_, s| s.user != key);
    let token = state.new_session(&mut a, &key);
    state.save();
    drop(a);
    state.hub.lock().unwrap().close_account(&key);
    println!("password of {} reset ({ip})", user.name);
    state.account(user.name, token)
}

/// Links an email to a logged-in account (see `Request::LinkEmail`).
fn link_email(state: &State, ip: IpAddr, token: &str, email: &str, code: Option<&str>) -> Reply {
    let Some(name) = state.user(token) else { return Reply::Error("your session expired: log in again".into()) };
    let key = name.to_ascii_lowercase();
    // The email gets the account back after a lost password: a session alone can't move that.
    if state.accounts.lock().unwrap().users.get(&key).is_some_and(|u| u.email.is_some()) {
        return Reply::Error("this account already has an email: ask an admin on Discord to change it".into());
    }
    let email = mail::normal(email);
    if !mail::valid_email(&email) {
        return Reply::Error("that email doesn't look right".into());
    }
    if state.email_owner(&email).is_some_and(|owner| owner != key) {
        return Reply::Error("that email is already used by another account".into());
    }
    let for_ = format!("link:{key}");
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        return match state.mail_code(ip, &for_, &email, &format!("for linking this email to {name}")) {
            Ok(()) => Reply::CodeSent(mail::masked(&email)),
            Err(e) => Reply::Error(e),
        };
    };
    if mail::pending_email(&for_).is_some_and(|e| e != email) {
        return Reply::Error("the code was sent to another email: ask for a new one".into());
    }
    let email = match mail::check_code(&for_, code) {
        Ok(e) => e,
        Err(e) => return Reply::Error(e),
    };
    let mut a = state.accounts.lock().unwrap();
    if let Some(u) = a.users.get_mut(&key) {
        u.email = Some(email.clone());
    }
    state.save();
    drop(a);
    println!("{name} linked an email");
    Reply::EmailLinked(mail::masked(&email))
}

fn login(state: &State, ip: IpAddr, name: &str, password: &str) -> Reply {
    let Some(slot) = state.take_slot(ip, |l| &mut l.failed, Duration::from_secs(600), FAILED_LOGINS) else {
        return Reply::Error("too many wrong passwords, wait a few minutes".into());
    };
    let key = name.trim().to_ascii_lowercase();
    let user = state.accounts.lock().unwrap().users.get(&key).cloned();
    let ok = verify_password(password, user.as_ref().map(|u| u.hash.as_str()).unwrap_or(&state.dummy_hash)) && user.is_some();
    let Some(user) = user.filter(|_| ok) else {
        std::thread::sleep(Duration::from_millis(400));
        return Reply::Error("wrong name or password".into());
    };
    state.give_back(ip, |l| &mut l.failed, slot);
    let mut a = state.accounts.lock().unwrap();
    let token = state.new_session(&mut a, &key);
    state.save();
    Reply::Account { name: user.name, token, nominator: is_admin(&key) || user.nominator, email: Some(user.email.is_some()) }
}

impl State {
    /// A player's profile, by their name in any case.
    fn profile(&self, name: &str) -> Option<online::Profile> {
        let key = name.trim().to_ascii_lowercase();
        let (name, joined) = self.accounts.lock().unwrap().users.get(&key).map(|u| (u.name.clone(), u.created))?;
        let mut p = self.scores.lock().unwrap().profile(&key, &name, joined);
        p.online = key == ALWAYS_ONLINE || self.hub.lock().unwrap().conns.contains_key(&key);
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
        let activity = self.hub.lock().unwrap().conns.get(&key).and_then(|v| v.last()).map(|c| c.activity.clone());
        activity.or_else(|| (key == ALWAYS_ONLINE).then_some(Activity::Menus))
    }

    /// `name`'s friends as written, and whether each is online.
    fn friends_online(&self, name: &str) -> Vec<(String, bool)> {
        let key = name.trim().to_ascii_lowercase();
        // Names first: the accounts and the hub are never locked at once here.
        let named: Vec<(String, String)> = self.friend_keys(&key).into_iter().map(|k| (self.display_name(&k), k)).collect();
        let hub = self.hub.lock().unwrap();
        let mut out: Vec<(String, bool)> = named.into_iter().map(|(name, k)| (name, k == ALWAYS_ONLINE || hub.conns.contains_key(&k))).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        out
    }

    /// Admins, and those they've made loved nominators.
    fn is_nominator(&self, key: &str) -> bool {
        is_admin(key) || self.accounts.lock().unwrap().users.get(key).is_some_and(|u| u.nominator)
    }

    /// Admin `by` (lowercased) makes `name` a loved nominator, or not any more.
    fn set_nominator(&self, by: &str, name: &str, on: bool) -> Result<String, String> {
        if !is_admin(by) {
            return Err("only admins can do that".into());
        }
        let key = name.trim().to_ascii_lowercase();
        if is_admin(&key) {
            return Err("admins are loved nominators already".into());
        }
        let mut a = self.accounts.lock().unwrap();
        let user = a.users.get_mut(&key).ok_or("there's no player by that name")?;
        user.nominator = on;
        let shown = user.name.clone();
        self.save();
        println!("{shown} {} loved nominator (by {by})", if on { "made" } else { "no longer" });
        Ok(if on { format!("{shown} is a loved nominator now") } else { format!("{shown} isn't a loved nominator any more") })
    }

    /// Nominator `by` (lowercased) loves map set `set` here, or takes it back: plays on it give
    /// pp (or don't) from now on, the ones already sent too.
    fn love(&self, by: &str, set: u32, on: bool) -> Result<String, String> {
        if !self.is_nominator(by) {
            return Err("only loved nominators can love maps".into());
        }
        if on && loved::contains(Some(set)) {
            return Err("that set is loved here already".into());
        }
        if on {
            // Only maps that don't give pp already, and that the mirrors know.
            covers::learn(Some(set));
            match covers::osu_status(set).as_deref() {
                None => return Err("couldn't look that set up on the beatmap mirrors: try again in a moment".into()),
                Some("missing") => return Err("the beatmap mirrors don't know that set".into()),
                Some("ranked" | "approved" | "loved") => return Err("that set gives pp already: it's ranked or loved on osu!".into()),
                Some(_) => {}
            }
        }
        let name = self.display_name(by);
        if !loved::set(set, on, &name, now_secs()) {
            return Err("that set isn't loved here".into());
        }
        self.scores.lock().unwrap().count_pp();
        println!("set {set} {} by {name}", if on { "loved" } else { "unloved" });
        Ok(if on { format!("Loved set {set}: its plays give pp now") } else { format!("Set {set} isn't loved here any more") })
    }

    /// A code `key` sends the Discord bot (`>verify`) to prove the Discord account is theirs.
    fn verify_code(&self, key: &str) -> String {
        let mut codes = self.verify_codes.lock().unwrap();
        codes.retain(|_, (_, made)| made.elapsed() < VERIFY_FOR);
        if let Some((code, _)) = codes.iter().find(|(_, (k, _))| k == key) {
            return code.clone();
        }
        // Letters and digits that don't look alike.
        const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let code: String = random_bytes::<8>().iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
        codes.insert(code.clone(), (key.to_string(), Instant::now()));
        code
    }

    /// Whose a `>verify` code is (it works once).
    fn take_verify_code(&self, code: &str) -> Option<String> {
        let (key, made) = self.verify_codes.lock().unwrap().remove(&code.trim().to_ascii_uppercase())?;
        (made.elapsed() < VERIFY_FOR).then_some(key)
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
            let sent = a.asked.get(key).into_iter().flatten().map(name).collect();
            (friends, requests, sent)
        };
        let hub = self.hub.lock().unwrap();
        let mut friends: Vec<Friend> = friends
            .into_iter()
            .map(|(k, name)| {
                let activity = hub.conns.get(&k).and_then(|v| v.last()).map(|c| c.activity.clone());
                Friend { name, activity: activity.or_else(|| (k == ALWAYS_ONLINE).then_some(Activity::Menus)) }
            })
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
                a.unask(&other, key);
                a.users.get_mut(key).unwrap().friends.push(other.clone());
                a.users.get_mut(&other).unwrap().friends.push(key.to_string());
                self.save();
                Ok((true, a.users[&other].name.clone()))
            } else {
                a.ask(key, &other);
                self.save();
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
fn social_session(state: &State, mut reader: BufReader<TcpStream>, name: String, token: String) {
    let key = name.to_ascii_lowercase();
    let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(90)));
    let Ok(mut writer) = reader.get_ref().try_clone() else { return };
    let (tx, rx) = mpsc::channel::<Line>();
    let tx = Tx(tx);
    // A quiet connection gets a Pong every 20 s anyway: games that only ping while they send
    // nothing else went 70 s without hearing from us during a solo play and dropped the link.
    let pong = line(&Down::Pong);
    let writing = std::thread::Builder::new().spawn(move || {
        loop {
            let l = match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(l) => l,
                Err(mpsc::RecvTimeoutError::Timeout) => pong.clone(),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if l.is_empty() || writer.write_all(l.as_bytes()).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });
    if let Err(e) = writing {
        eprintln!("social connection of {name}: {e}");
        return;
    }
    let id = {
        let mut hub = state.hub.lock().unwrap();
        hub.next_conn += 1;
        let id = hub.next_conn;
        hub.conns.entry(key.clone()).or_default().push(Conn { id, name: name.clone(), token, tx: tx.clone(), activity: Activity::Menus, watching: None, chat_times: VecDeque::new() });
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
        hub.set_watching(&key, id, None);
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

fn social_up(state: &State, key: &str, name: &str, id: u64, tx: &Tx, up: Up) {
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
                }
                if let Some(them) = a.users.get_mut(&other) {
                    them.friends.retain(|f| f != key);
                }
                a.unask(&other, key);
                a.unask(key, &other);
                state.save();
            }
            {
                let mut hub = state.hub.lock().unwrap();
                hub.stop_watching(key, &other);
                hub.stop_watching(&other, key);
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
            hub.set_watching(key, id, Some(other.clone()));
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
            state.hub.lock().unwrap().set_watching(key, id, None);
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
            let room = scores::MAX_INPUTS.saturating_sub(p.inputs.len());
            p.inputs.extend_from_slice(&inputs[..inputs.len().min(room)]);
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

/// Writes accounts.json each time it changes: a copy is taken under the lock and written
/// after, so logins and friend requests don't wait on the disk (changes made while one is being
/// written go out with the next).
fn saver(state: Arc<State>, wake: mpsc::Receiver<()>) {
    while wake.recv().is_ok() {
        while wake.try_recv().is_ok() {}
        let copy = state.accounts.lock().unwrap().clone();
        state.write(&copy);
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
    let mut wait = PLAYERS_EVERY;
    loop {
        std::thread::sleep(wait);
        let mut hub = state.hub.lock().unwrap();
        if std::mem::take(&mut hub.players_changed) {
            let d = hub.players();
            hub.send_to_all(&d);
            // With many online the list is long and changes all the time: it goes out less
            // often then (10 ms per connection), so the traffic stays about the same however many
            // there are (~2 MB/s at most).
            let conns: usize = hub.conns.values().map(Vec::len).sum();
            wait = PLAYERS_EVERY.max(Duration::from_millis(conns as u64 * 10));
        }
    }
}

/// Connections open at once from one address: a whole LAN party behind one router fits, a
/// flood of idle connections from one place doesn't take every thread.
const PER_ADDRESS: usize = 200;

/// Connections open per address.
#[derive(Default)]
struct Open(Mutex<HashMap<IpAddr, usize>>);

/// One open connection, counted until it's dropped.
struct Slot(Arc<Open>, IpAddr);

impl Open {
    fn take(self: &Arc<Open>, ip: IpAddr) -> Option<Slot> {
        let ip = limit_key(ip);
        let mut open = self.0.lock().unwrap();
        let n = open.entry(ip).or_default();
        if *n >= PER_ADDRESS {
            return None;
        }
        *n += 1;
        Some(Slot(self.clone(), ip))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut open = self.0 .0.lock().unwrap();
        if let Some(n) = open.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                open.remove(&self.1);
            }
        }
    }
}

/// Listens on `port`, on IPv6 and IPv4 (on Linux `::` is usually dual-stack and the second bind
/// just fails).
fn bind(port: u16) -> Vec<TcpListener> {
    ["::", "0.0.0.0"].iter().filter_map(|addr| TcpListener::bind((*addr, port)).ok()).collect()
}

fn main() {
    let mut port = online::DEFAULT_PORT;
    let mut data = PathBuf::from(".");
    let mut http_port = 0;
    let mut tls_port = 0;
    let mut behind_proxy = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).expect("--port N"),
            "--data" => data = PathBuf::from(args.next().expect("--data DIR")),
            "--http-port" => http_port = args.next().and_then(|p| p.parse().ok()).expect("--http-port N"),
            "--tls-port" => tls_port = args.next().and_then(|p| p.parse().ok()).expect("--tls-port N"),
            "--behind-proxy" => behind_proxy = true,
            _ => {
                eprintln!("usage: taiko-server [--port 7800] [--tls-port 7801] [--data DIR] [--http-port 8080 [--behind-proxy]]");
                std::process::exit(2);
            }
        }
    }
    std::fs::create_dir_all(&data).expect("data dir");
    covers::init(data.clone());
    osustatus::init(data.clone());
    loved::init(data.clone());
    avatars::init(data.clone());
    mail::init(data.clone());
    let (state, wake) = State::load(data.join("accounts.json"));
    let state = Arc::new(state);
    {
        let state = state.clone();
        std::thread::spawn(move || saver(state, wake));
    }
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
    {
        let state = state.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(COUNT_PP_EVERY);
            state.scores.lock().unwrap().count_pp();
        });
    }
    let mut listeners = bind(port);
    assert!(!listeners.is_empty(), "can't listen on port {port}");
    println!("taiko-server listening on port {port} (protocol {}, rooms protocol {})", online::PROTOCOL, net::PROTOCOL);
    let mut listeners: Vec<(TcpListener, Option<Arc<rustls::ServerConfig>>)> = listeners.drain(..).map(|l| (l, None)).collect();
    if tls_port != 0 {
        // Without its certificate the plain port still serves everyone.
        match tls::server_config(&data.join("tls-cert.der"), &data.join("tls-key.der")) {
            Ok(config) => {
                let encrypted = bind(tls_port);
                assert!(!encrypted.is_empty(), "can't listen on port {tls_port}");
                listeners.extend(encrypted.into_iter().map(|l| (l, Some(config.clone()))));
                println!("encrypted on port {tls_port}");
            }
            Err(e) => eprintln!("not encrypted: {e}"),
        }
    }
    let open = Arc::new(Open::default());
    let threads: Vec<_> = listeners
        .into_iter()
        .map(|(l, config)| {
            let state = state.clone();
            let open = open.clone();
            std::thread::spawn(move || {
                for stream in l.incoming().flatten() {
                    let ip = stream.peer_addr().map(|a| a.ip()).unwrap_or(IpAddr::from([0, 0, 0, 0]));
                    let Some(slot) = open.take(ip) else { continue };
                    let state = state.clone();
                    let config = config.clone();
                    // Out of threads (or memory): this one connection is turned away, not the server.
                    if let Err(e) = std::thread::Builder::new().spawn(move || {
                        let stream = match config {
                            Some(config) => match tls::accept(stream, config) {
                                Ok(plain) => plain,
                                Err(_) => return,
                            },
                            None => stream,
                        };
                        handle(&state, stream, slot, ip)
                    }) {
                        eprintln!("connection from {ip}: {e}");
                    }
                }
            })
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> User {
        User { name: name.into(), hash: String::new(), created: 0, friends: Vec::new(), requests: Vec::new(), nominator: false, email: None }
    }

    #[test]
    fn accounts_come_with_an_email_and_its_code_resets_the_password() {
        let dir = std::env::temp_dir().join(format!("taiko-server-mail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        mail::init_for_tests(dir.clone());
        let (state, _wake) = State::load(dir.join("accounts.json"));
        let ip: IpAddr = [10, 0, 0, 1].into();
        let code = || {
            let letter = mail::next().expect("a letter");
            letter.lines().find_map(|l| l.trim().parse::<u32>().ok().filter(|_| l.trim().len() == 6)).map(|c| format!("{c:06}")).expect("a code in it")
        };
        let err = |r: Reply| match r {
            Reply::Error(e) => e,
            _ => panic!("not an error"),
        };
        assert!(err(register(&state, ip, "newbie", "secret1", None, None)).contains("email"));
        assert!(matches!(register(&state, ip, "newbie", "secret1", Some("New@Mail.ru"), None), Reply::CodeSent(to) if to == "n**@mail.ru"));
        let right = code();
        assert_eq!(err(register(&state, ip, "newbie", "secret1", Some("new@mail.ru"), Some("000000"))), "wrong code");
        assert!(matches!(register(&state, ip, "newbie", "secret1", Some("new@mail.ru"), Some(&right)), Reply::Account { email: Some(true), .. }));
        assert!(err(register(&state, ip, "other", "secret1", Some("NEW@mail.ru"), None)).contains("already used"));
        // Forgot it: a code to the email, by name or by the email itself.
        assert!(matches!(reset_password(&state, ip, "new@mail.ru", None, None), Reply::CodeSent(_)));
        let right = code();
        assert!(err(reset_password(&state, ip, "Newbie", Some(&right), Some("x"))).contains("6 characters"));
        assert!(matches!(reset_password(&state, ip, "Newbie", Some(&right), Some("better1")), Reply::Account { .. }));
        assert!(matches!(login(&state, ip, "newbie", "better1"), Reply::Account { .. }));
        assert!(matches!(login(&state, ip, "newbie", "secret1"), Reply::Error(_)));
        // An account from before emails links one.
        state.accounts.lock().unwrap().users.insert("old".into(), user("Old"));
        let token = state.new_session(&mut state.accounts.lock().unwrap(), "old");
        assert!(matches!(state.account("Old".into(), token.clone()), Reply::Account { email: Some(false), .. }));
        assert!(err(reset_password(&state, ip, "old", None, None)).contains("no email"));
        assert!(matches!(link_email(&state, ip, &token, "old@x.io", None), Reply::CodeSent(_)));
        let right = code();
        assert!(matches!(link_email(&state, ip, &token, "old@x.io", Some(&right)), Reply::EmailLinked(_)));
        assert!(matches!(state.account("Old".into(), token), Reply::Account { email: Some(true), .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn asked_follows_the_requests() {
        let mut a = Accounts::default();
        for n in ["a", "b", "c"] {
            a.users.insert(n.into(), user(n));
        }
        a.ask("a", "b");
        a.ask("a", "b");
        a.ask("a", "c");
        a.ask("c", "b");
        a.unask("a", "c");
        let kept = a.asked.clone();
        a.index_requests();
        assert_eq!(kept, a.asked);
        assert_eq!(a.asked.get("a"), Some(&vec!["b".to_string()]));
        assert_eq!(a.users["b"].requests, ["a", "c"]);
        a.unask("a", "b");
        a.unask("c", "b");
        assert!(a.asked.is_empty() && a.users["b"].requests.is_empty());
    }

    fn test_state(tag: &str) -> State {
        let dir = std::env::temp_dir().join(format!("taiko-server-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        State::load(dir.join("accounts.json")).0
    }

    /// Adds a connection for `key`, as `social_session` does.
    fn connect(state: &State, key: &str, token: &str) -> (u64, Tx, mpsc::Receiver<Line>) {
        let (tx, rx) = mpsc::channel();
        let tx = Tx(tx);
        let mut hub = state.hub.lock().unwrap();
        hub.next_conn += 1;
        let id = hub.next_conn;
        hub.conns.entry(key.to_string()).or_default().push(Conn { id, name: key.to_string(), token: token.to_string(), tx: tx.clone(), activity: Activity::Menus, watching: None, chat_times: VecDeque::new() });
        (id, tx, rx)
    }

    fn lines(rx: &mpsc::Receiver<Line>) -> Vec<String> {
        rx.try_iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn a_linked_email_stays_put() {
        let state = test_state("linked");
        let ip = IpAddr::from([127, 0, 0, 1]);
        let token = {
            let mut a = state.accounts.lock().unwrap();
            a.users.insert("alice".into(), User { email: Some("a@mail.ru".into()), ..user("alice") });
            state.new_session(&mut a, "alice")
        };
        for code in [None, Some("000000")] {
            assert!(matches!(link_email(&state, ip, &token, "evil@mail.ru", code), Reply::Error(e) if e.contains("already has an email")));
        }
        assert_eq!(state.accounts.lock().unwrap().users["alice"].email.as_deref(), Some("a@mail.ru"));
    }

    #[test]
    fn saved_data_that_cant_be_read_is_an_error_not_a_blank_start() {
        let dir = std::env::temp_dir().join(format!("taiko-server-{}-load", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("accounts.json");
        assert!(try_load::<Accounts>(&file).unwrap().users.is_empty());
        std::fs::write(&file, "{ \"users\": {},, }").unwrap();
        assert!(try_load::<Accounts>(&file).is_err());
        std::fs::write(&file, [0xff, 0xfe]).unwrap();
        assert!(try_load::<Accounts>(&file).is_err());
        std::fs::write(&file, serde_json::to_string(&Accounts::default()).unwrap()).unwrap();
        assert!(try_load::<Accounts>(&file).is_ok());
    }

    #[test]
    fn limits_count_a_whole_ipv6_home_as_one() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(limit_key(ip("2001:db8:1:2:aaaa::1")), limit_key(ip("2001:db8:1:2:bbbb::2")));
        assert_ne!(limit_key(ip("2001:db8:1:2::1")), limit_key(ip("2001:db8:1:3::1")));
        assert_eq!(limit_key(ip("::ffff:1.2.3.4")), ip("1.2.3.4"));
        assert_eq!(limit_key(ip("1.2.3.4")), ip("1.2.3.4"));
        let open = Arc::new(Open::default());
        let slots: Vec<_> = (0..PER_ADDRESS).map(|i| open.take(ip(&format!("2001:db8::{i:x}"))).unwrap()).collect();
        assert!(open.take(ip("2001:db8::ffff")).is_none());
        drop(slots);
        assert!(open.take(ip("2001:db8::ffff")).is_some());
    }

    #[test]
    fn slots_run_out_and_come_back() {
        let state = test_state("slots");
        let ip = IpAddr::from([127, 0, 0, 1]);
        let take = || state.take_slot(ip, |l| &mut l.failed, Duration::from_secs(60), 2);
        let first = take().unwrap();
        take().unwrap();
        assert!(take().is_none());
        state.give_back(ip, |l| &mut l.failed, first);
        assert!(take().is_some() && take().is_none());
    }

    #[test]
    fn wrong_logins_all_at_once_stay_inside_the_limit() {
        let state = test_state("logins");
        let ip = IpAddr::from([127, 0, 0, 1]);
        let wrong = std::thread::scope(|s| {
            let tries: Vec<_> = (0..FAILED_LOGINS * 3).map(|_| s.spawn(|| login(&state, ip, "nobody", "wrong password"))).collect();
            tries.into_iter().map(|t| t.join().unwrap()).filter(|r| matches!(r, Reply::Error(e) if e.contains("wrong name"))).count()
        });
        assert_eq!(wrong, FAILED_LOGINS);
    }

    #[test]
    fn a_good_login_leaves_no_failure_behind() {
        let state = test_state("good-login");
        let ip = IpAddr::from([127, 0, 0, 1]);
        let hash = hash_password("a good password").unwrap();
        state.accounts.lock().unwrap().users.insert("alice".into(), User { hash, ..user("alice") });
        for _ in 0..FAILED_LOGINS + 1 {
            assert!(matches!(login(&state, ip, "alice", "a good password"), Reply::Account { .. }));
        }
    }

    #[test]
    fn a_taken_name_uses_up_no_registration() {
        let state = test_state("taken");
        let ip = IpAddr::from([127, 0, 0, 1]);
        state.accounts.lock().unwrap().users.insert("alice".into(), user("alice"));
        for _ in 0..REGISTER_PER_HOUR + 1 {
            assert!(matches!(register(&state, ip, "alice", "a good password", None, None), Reply::Error(e) if e.contains("taken")));
        }
    }

    #[test]
    fn unfriending_ends_the_watch_both_ways() {
        let state = test_state("unfriend");
        {
            let mut a = state.accounts.lock().unwrap();
            for n in ["alice", "bob"] {
                a.users.insert(n.into(), user(n));
            }
            a.users.get_mut("alice").unwrap().friends.push("bob".into());
            a.users.get_mut("bob").unwrap().friends.push("alice".into());
        }
        let (alice, alice_tx, alice_rx) = connect(&state, "alice", "a");
        let (bob, bob_tx, bob_rx) = connect(&state, "bob", "b");
        social_up(&state, "alice", "alice", alice, &alice_tx, Up::Watch("bob".into()));
        social_up(&state, "bob", "bob", bob, &bob_tx, Up::Watch("alice".into()));
        lines(&alice_rx);
        lines(&bob_rx);
        social_up(&state, "alice", "alice", alice, &alice_tx, Up::RemoveFriend("bob".into()));
        for rx in [&alice_rx, &bob_rx] {
            assert!(lines(rx).iter().any(|l| l.contains("WatchStopped")));
        }
        let hub = state.hub.lock().unwrap();
        assert!(hub.conns.values().flatten().all(|c| c.watching.is_none()));
        assert!(hub.watchers.is_empty());
    }

    #[test]
    fn logging_out_closes_that_sessions_connections_only() {
        let state = test_state("logout");
        let (_, _, gone) = connect(&state, "alice", "old");
        let (_, _, stays) = connect(&state, "alice", "other");
        state.logout("old");
        assert!(lines(&gone).iter().any(|l| l.is_empty()));
        assert!(lines(&stays).is_empty());
    }

    #[test]
    fn a_live_play_keeps_at_most_max_inputs() {
        let state = test_state("inputs");
        let (id, tx, _rx) = connect(&state, "alice", "a");
        let map = serde_json::from_str(r#"{"hash":1,"artist":"a","title":"t","version":"v"}"#).unwrap();
        social_up(&state, "alice", "alice", id, &tx, Up::PlayStart(SoloPlay { map, anti_mash: false }));
        for _ in 0..3 {
            let inputs = vec![(0.0, 1); scores::MAX_INPUTS / 2];
            social_up(&state, "alice", "alice", id, &tx, Up::PlayFrame { t: 0.0, paused: false, inputs });
        }
        assert_eq!(state.hub.lock().unwrap().plays["alice"].inputs.len(), scores::MAX_INPUTS);
    }
}
