//! The online server: accounts (used by multiplayer for now) and lobbies hosted on the server,
//! so nobody needs an open port. This is the protocol shared by the game and `server/`, plus
//! the game's side of it. A connection sends one request line and reads one reply line; after
//! `Reply::Joined` the same connection speaks the room protocol (`net.rs`). Direct play
//! (`net::host` / `net::Client::connect`) stays as the fallback when the server is down.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 1;
pub const DEFAULT_PORT: u16 = 7800;
pub const DEFAULT_SERVER: &str = "jashincraft.space:7800";
/// Longest request or reply line (a lobby list of a few hundred rooms fits easily).
const MAX_LINE: u64 = 1 << 20;

#[derive(Serialize, Deserialize, Debug)]
pub struct Envelope {
    pub protocol: u32,
    pub request: Request,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
    Register { name: String, password: String },
    Login { name: String, password: String },
    /// Checks a saved session token.
    WhoAmI { token: String },
    Logout { token: String },
    ListRooms,
    /// Opens a lobby on the server and joins it as its host.
    CreateRoom { token: String, title: String },
    JoinRoom { token: String, room: u32 },
    /// The social connection (friends, chat, watching): after `Reply::Account` the connection
    /// stays open, the game sending `Up` lines and the server `Down` lines.
    Connect { token: String },
}

/// What a player is doing, for their friends.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Activity {
    Menus,
    /// A solo play (friends can watch it).
    Playing { map: String },
    Lobby { room: Option<u32>, title: String },
    Match { map: String },
    Watching { name: String },
}

impl Activity {
    pub fn describe(&self) -> String {
        match self {
            Activity::Menus => "in the menus".into(),
            Activity::Playing { map } => format!("playing {map}"),
            Activity::Lobby { title, .. } => format!("in lobby {title}"),
            Activity::Match { map } => format!("in a match: {map}"),
            Activity::Watching { name } => format!("watching {name}"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Friend {
    pub name: String,
    /// None when offline.
    pub activity: Option<Activity>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChatLine {
    pub from: String,
    pub to: String,
    pub text: String,
    /// Unix seconds.
    pub at: u64,
}

/// A solo play being streamed to friends.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SoloPlay {
    pub map: crate::net::MapInfo,
    pub anti_mash: bool,
}

/// Game -> server on the social connection.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Up {
    Status(Activity),
    /// Sends a friend request, or accepts theirs.
    AddFriend(String),
    /// Unfriends, or declines / withdraws a request.
    RemoveFriend(String),
    Chat { to: String, text: String },
    Watch(String),
    StopWatching,
    PlayStart(SoloPlay),
    /// Every ~100 ms of a solo play: its map time, whether it's paused, presses since the last.
    PlayFrame { t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    PlayEnd,
    Ping,
}

/// Server -> game on the social connection.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Down {
    /// Friends (with what they're doing), requests to us, and ours still waiting.
    Friends { friends: Vec<Friend>, requests: Vec<String>, sent: Vec<String> },
    Chat(ChatLine),
    Notice(String),
    /// The watched friend is playing: the play, where they are and every press so far.
    WatchStart { player: String, play: SoloPlay, t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    WatchFrame { t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    /// The watched friend's play ended (we keep watching for their next one).
    WatchEnd { player: String },
    /// Watching isn't possible (any more): why.
    WatchStopped(String),
    Pong,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Reply {
    /// Logged in (or the token checked out): the account's name and a session token.
    Account { name: String, token: String },
    LoggedOut,
    Rooms(Vec<RoomInfo>),
    /// In the room: from here on this connection speaks the room protocol. Send Hello with
    /// `host_token` (set when we created the room).
    Joined { room: u32, title: String, host_token: Option<u64> },
    Error(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RoomInfo {
    pub id: u32,
    pub title: String,
    pub host: String,
    pub players: u32,
    pub map: Option<String>,
    pub in_match: bool,
}

pub const NAME_RULES: &str = "3-16 letters, digits, _ or -";

pub fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn valid_password(password: &str) -> bool {
    (6..=128).contains(&password.chars().count())
}

/// Text the built-in font draws (printable ASCII and Cyrillic), at most `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ' || matches!(c, 'А'..='я' | 'Ё' | 'ё')).take(max).collect()
}

/// A lobby title, at most 32 characters.
#[allow(dead_code)] // used by the online server (server/)
pub fn clean_title(title: &str, owner: &str) -> String {
    let t = clean_text(title, 32);
    let t = t.trim();
    if t.is_empty() { format!("{owner}'s lobby") } else { t.to_string() }
}

/// One line of JSON (without the newline), at most `MAX_LINE` bytes.
pub fn read_line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let n = reader.take(MAX_LINE).read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    if !line.ends_with('\n') {
        return Err("message too large".into());
    }
    Ok(line)
}

pub fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), String> {
    let mut line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    line.push('\n');
    w.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

fn connect(server: &str) -> Result<TcpStream, String> {
    let server = server.trim();
    let with_port = if server.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { server.to_string() } else { format!("{server}:{DEFAULT_PORT}") };
    let addrs: Vec<_> = with_port.to_socket_addrs().map_err(|e| format!("can't find the server: {e}"))?.collect();
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(4)) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
                return Ok(s);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("the server is offline ({last})"))
}

/// Sends one request; returns the reply and the connection (still open).
pub fn exchange(server: &str, request: Request) -> Result<(Reply, BufReader<TcpStream>), String> {
    let mut stream = connect(server)?;
    write_line(&mut stream, &Envelope { protocol: PROTOCOL, request })?;
    let mut reader = BufReader::new(stream);
    let line = read_line(&mut reader)?;
    let reply: Reply = serde_json::from_str(&line).map_err(|e| format!("odd reply from the server: {e}"))?;
    Ok((reply, reader))
}

/// What the social connection reports to the game.
#[allow(dead_code)] // the game's side (the server only shares the protocol)
pub enum SocialEvent {
    /// (Re)connected as this account: resend what we're doing.
    Connected(String),
    Disconnected(String),
    /// The saved login is no longer valid.
    SessionExpired,
    Down(Down),
}

/// The game's social connection (friends, chat, watching). It reconnects by itself while the
/// server is away; dropping it disconnects.
#[allow(dead_code)]
pub struct Social {
    tx: std::sync::mpsc::Sender<Up>,
    rx: std::sync::mpsc::Receiver<SocialEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[allow(dead_code)]
impl Social {
    pub fn start(server: String, token: String) -> Social {
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;
        let (tx, out) = mpsc::channel::<Up>();
        let (events, rx) = mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let mut announced_down = false;
            while !stopped.load(Ordering::Relaxed) {
                let (reply, reader) = match exchange(&server, Request::Connect { token: token.clone() }) {
                    Ok(r) => r,
                    Err(e) => {
                        if !announced_down {
                            announced_down = true;
                            let _ = events.send(SocialEvent::Disconnected(e));
                        }
                        for _ in 0..20 {
                            if stopped.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(500));
                        }
                        continue;
                    }
                };
                let name = match reply {
                    Reply::Account { name, .. } => name,
                    Reply::Error(e) if e.contains("session") => {
                        let _ = events.send(SocialEvent::SessionExpired);
                        return;
                    }
                    _ => {
                        std::thread::sleep(Duration::from_secs(10));
                        continue;
                    }
                };
                // Socket timeouts are per handle on Windows (shared on Linux): the reader's own
                // handle must lose the 10 s request timeout, or a quiet stretch drops the link.
                let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(70)));
                let _ = reader.get_ref().set_write_timeout(None);
                let Ok(mut stream) = reader.get_ref().try_clone() else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(70)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
                // Whatever piled up while away is stale: the game resends its state on Connected.
                while out.try_recv().is_ok() {}
                let _ = events.send(SocialEvent::Connected(name));
                let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                {
                    let (events, alive) = (events.clone(), alive.clone());
                    std::thread::spawn(move || {
                        let mut reader = reader;
                        while let Ok(line) = read_line(&mut reader) {
                            if let Ok(d) = serde_json::from_str::<Down>(&line) {
                                if events.send(SocialEvent::Down(d)).is_err() {
                                    break;
                                }
                            }
                        }
                        alive.store(false, Ordering::Relaxed);
                    });
                }
                let mut last_ping = std::time::Instant::now();
                loop {
                    if stopped.load(Ordering::Relaxed) {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                    if !alive.load(Ordering::Relaxed) {
                        break;
                    }
                    let up = match out.recv_timeout(Duration::from_millis(300)) {
                        Ok(up) => Some(up),
                        Err(mpsc::RecvTimeoutError::Timeout) if last_ping.elapsed() > Duration::from_secs(25) => Some(Up::Ping),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    };
                    if let Some(up) = up {
                        if matches!(up, Up::Ping) {
                            last_ping = std::time::Instant::now();
                        }
                        if write_line(&mut stream, &up).is_err() {
                            break;
                        }
                    }
                }
                let _ = stream.shutdown(std::net::Shutdown::Both);
                let _ = events.send(SocialEvent::Disconnected("lost the connection".into()));
                announced_down = true;
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        Social { tx, rx, stop }
    }

    pub fn send(&self, up: Up) {
        let _ = self.tx.send(up);
    }

    pub fn poll(&self) -> Vec<SocialEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Social {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One request and its reply; a `Reply::Error` comes back as `Err`.
pub fn call(server: &str, request: Request) -> Result<Reply, String> {
    match exchange(server, request)?.0 {
        Reply::Error(e) => Err(e),
        reply => Ok(reply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_titles() {
        assert!(valid_name("Lynerrr") && valid_name("den4ik_228") && valid_name("a-b"));
        assert!(!valid_name("ab") && !valid_name("has space") && !valid_name("x".repeat(17).as_str()) && !valid_name("ünï"));
        assert!(valid_password("hunter2") && !valid_password("short"));
        assert_eq!(clean_title("  ", "Lynerrr"), "Lynerrr's lobby");
        assert_eq!(clean_title("taiko\u{7}  night", "x"), "taiko  night");
    }

    #[test]
    fn messages_round_trip() {
        let env = Envelope { protocol: PROTOCOL, request: Request::JoinRoom { token: "t".into(), room: 3 } };
        let s = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(back.request, Request::JoinRoom { room: 3, .. }));
        let mut buf = Vec::new();
        write_line(&mut buf, &Reply::Joined { room: 3, title: "x".into(), host_token: Some(7) }).unwrap();
        let line = read_line(&mut &buf[..]).unwrap();
        assert!(matches!(serde_json::from_str::<Reply>(&line).unwrap(), Reply::Joined { host_token: Some(7), .. }));
    }
}
