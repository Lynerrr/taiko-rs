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

/// A lobby title: printable ASCII (what the font draws), at most 32 characters.
#[allow(dead_code)] // used by the online server (server/)
pub fn clean_title(title: &str, owner: &str) -> String {
    let t: String = title.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(32).collect();
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
