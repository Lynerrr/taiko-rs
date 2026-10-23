//! taiko-rs online server: accounts (used by multiplayer for now) and lobbies hosted here, so
//! players don't need an open port. Rooms run the game's own room code (`src/net.rs`); the
//! protocol in front of them is `src/online.rs`.
//!
//!   taiko-server [--port 7800] [--data DIR]
//!
//! Accounts and sessions live in DIR/accounts.json (passwords as argon2 hashes).

#[allow(dead_code)]
#[path = "../../src/net.rs"]
mod net;
#[allow(dead_code)]
#[path = "../../src/online.rs"]
mod online;

use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use online::{Envelope, Reply, Request, RoomInfo};
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
        State {
            accounts: Mutex::new(accounts),
            path,
            lobbies: Mutex::new(HashMap::new()),
            next_room: AtomicU32::new(1),
            limits: Mutex::new(HashMap::new()),
            dummy_hash: hash_password("not a real password").expect("argon2"),
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
        return reply(&mut reader, Reply::Error(format!("update taiko-rs (server protocol {}, yours {})", online::PROTOCOL, env.protocol)));
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
    a.users.insert(key.clone(), User { name: name.to_string(), hash, created: now_secs() });
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

fn main() {
    let mut port = online::DEFAULT_PORT;
    let mut data = PathBuf::from(".");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).expect("--port N"),
            "--data" => data = PathBuf::from(args.next().expect("--data DIR")),
            _ => {
                eprintln!("usage: taiko-server [--port 7800] [--data DIR]");
                std::process::exit(2);
            }
        }
    }
    std::fs::create_dir_all(&data).expect("data dir");
    let state = Arc::new(State::load(data.join("accounts.json")));
    {
        let state = state.clone();
        std::thread::spawn(move || reaper(state));
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
