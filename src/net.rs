//! Multiplayer networking: a host-authoritative room over TCP, one JSON message per line.
//!
//! Gameplay never crosses the network — everyone plays the same map locally with their own audio
//! clock — so this only carries lobby state, a synchronised start time and live scores. The host
//! runs the room on a background thread and joins it through localhost like any other player.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 1;
pub const DEFAULT_PORT: u16 = 7777;
/// Countdown between everyone having loaded and the song starting.
const START_LEAD_MS: f64 = 3000.0;
/// Players who haven't loaded the map by then are left out of the match.
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Live score broadcasts during a match.
const MATCH_BROADCAST: Duration = Duration::from_millis(100);
/// Longest accepted message; anything bigger drops the connection.
const MAX_LINE: u64 = 256 * 1024;

/// Reads one newline-terminated message, refusing oversized ones. `Ok(None)` at end of stream.
fn read_message(reader: &mut impl BufRead, buf: &mut String) -> std::io::Result<Option<()>> {
    buf.clear();
    let n = std::io::Read::take(&mut *reader, MAX_LINE).read_line(buf)?;
    if n == 0 {
        return Ok(None);
    }
    if !buf.ends_with('\n') && n as u64 >= MAX_LINE {
        return Err(std::io::Error::other("message too large"));
    }
    Ok(Some(()))
}

/// FNV-1a over the `.osu` file: identifies a difficulty across machines.
pub fn map_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapInfo {
    pub hash: u64,
    pub artist: String,
    pub title: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Live {
    pub score: u64,
    pub combo: u32,
    pub accuracy: f64,
    pub miss: u32,
    pub progress: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Final {
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub ur: f64,
    pub quit: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Lobby,
    Loading,
    Playing,
    Finished,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayerInfo {
    pub id: u32,
    pub name: String,
    pub host: bool,
    pub ready: bool,
    pub has_map: bool,
    pub status: Status,
    pub live: Live,
    pub result: Option<Final>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientMsg {
    /// `host_token` is set only by the host's own client, proving it owns the room.
    Hello { name: String, protocol: u32, host_token: Option<u64> },
    Ping { t: f64 },
    SelectMap(MapInfo),
    HasMap(bool),
    Ready(bool),
    StartMatch,
    Loaded(bool),
    Live(Live),
    Finished(Final),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMsg {
    Welcome { id: u32 },
    Rejected(String),
    Pong { t: f64, server: f64 },
    Room { players: Vec<PlayerInfo>, map: Option<MapInfo>, in_match: bool },
    Prepare(MapInfo),
    /// Start the song at this server time (ms).
    Start { at: f64 },
    Results(Vec<PlayerInfo>),
    /// Synthesised locally when the connection drops.
    Disconnected(String),
}

fn send_line<T: Serialize>(w: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(std::io::Error::other)?;
    line.push('\n');
    w.write_all(line.as_bytes())
}

/// Keeps printable ASCII (all the built-in font can draw) and caps the length.
fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(20).collect();
    let n = n.trim().to_string();
    if n.is_empty() { "player".into() } else { n }
}

// ---------------------------------------------------------------------------------------------
// Host side

/// A running room. Dropping it closes the room and disconnects everyone.
pub struct Server {
    stop: Arc<AtomicBool>,
    pub token: u64,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

enum Event {
    Joined(u32, Sender<ServerMsg>),
    Msg(u32, ClientMsg),
    Left(u32),
}

pub fn host(port: u16) -> Result<Server, String> {
    // Dual-stack when available, so friends can join over IPv4 or IPv6.
    let listener = TcpListener::bind(("::", port))
        .or_else(|_| TcpListener::bind(("0.0.0.0", port)))
        .map_err(|e| format!("can't listen on port {port}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let token = {
        use std::hash::{BuildHasher, Hasher};
        // RandomState is seeded from the OS per process: unguessable for this purpose.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.finish()
    };
    let (events, rx) = mpsc::channel();
    {
        let stop = stop.clone();
        thread::Builder::new()
            .name("mp-accept".into())
            .spawn(move || {
                let mut next_id = 1;
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            spawn_connection(stream, next_id, events.clone());
                            next_id += 1;
                        }
                        Err(_) => thread::sleep(Duration::from_millis(50)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    {
        let stop = stop.clone();
        thread::Builder::new().name("mp-room".into()).spawn(move || room_loop(rx, stop, token)).map_err(|e| e.to_string())?;
    }
    Ok(Server { stop, token })
}

fn spawn_connection(stream: TcpStream, id: u32, events: Sender<Event>) {
    let _ = stream.set_nodelay(true);
    let Ok(mut writer) = stream.try_clone() else { return };
    let (tx, rx) = mpsc::channel::<ServerMsg>();
    thread::spawn(move || {
        for msg in rx {
            if send_line(&mut writer, &msg).is_err() {
                break;
            }
        }
        // The room dropped this player (left, kicked or room closed): end the reader too.
        let _ = writer.shutdown(Shutdown::Both);
    });
    thread::spawn(move || {
        if events.send(Event::Joined(id, tx)).is_err() {
            return;
        }
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while let Ok(Some(())) = read_message(&mut reader, &mut line) {
            if let Ok(msg) = serde_json::from_str::<ClientMsg>(&line) {
                if events.send(Event::Msg(id, msg)).is_err() {
                    break;
                }
            }
        }
        let _ = events.send(Event::Left(id));
    });
}

struct Player {
    info: PlayerInfo,
    tx: Sender<ServerMsg>,
    hello: bool,
    in_match: bool,
    loaded: Option<bool>,
}

#[derive(PartialEq)]
enum Phase {
    Lobby,
    Loading { deadline: Instant },
    Playing,
}

fn room_loop(rx: Receiver<Event>, stop: Arc<AtomicBool>, token: u64) {
    let epoch = Instant::now();
    let now_ms = || epoch.elapsed().as_secs_f64() * 1000.0;
    let mut players: Vec<Player> = Vec::new();
    let mut map: Option<MapInfo> = None;
    let mut phase = Phase::Lobby;
    let mut dirty = false;
    let mut last_broadcast = Instant::now();
    let mut host_id: Option<u32> = None;

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Joined(id, tx)) => {
                players.push(Player {
                    info: PlayerInfo {
                        id,
                        name: String::new(),
                        host: false,
                        ready: false,
                        has_map: false,
                        status: Status::Lobby,
                        live: Live::default(),
                        result: None,
                    },
                    tx,
                    hello: false,
                    in_match: false,
                    loaded: None,
                });
            }
            Ok(Event::Left(id)) => {
                players.retain(|p| p.info.id != id);
                dirty = true;
            }
            Ok(Event::Msg(id, msg)) => {
                let is_host = host_id == Some(id);
                let Some(p) = players.iter_mut().find(|p| p.info.id == id) else { continue };
                match msg {
                    ClientMsg::Hello { name, protocol, host_token } => {
                        // Only the client holding the room's token becomes host, however fast
                        // anyone else connects.
                        if host_token == Some(token) && host_id.is_none() {
                            host_id = Some(id);
                            p.info.host = true;
                        }
                        if protocol != PROTOCOL {
                            let _ = p.tx.send(ServerMsg::Rejected(format!("version mismatch (host {PROTOCOL}, you {protocol})")));
                            players.retain(|p| p.info.id != id);
                            continue;
                        }
                        p.info.name = clean_name(&name);
                        p.hello = true;
                        let _ = p.tx.send(ServerMsg::Welcome { id });
                        dirty = true;
                    }
                    ClientMsg::Ping { t } => {
                        let _ = p.tx.send(ServerMsg::Pong { t, server: now_ms() });
                    }
                    ClientMsg::SelectMap(m) if is_host && phase == Phase::Lobby => {
                        map = Some(m);
                        for p in &mut players {
                            p.info.has_map = false;
                            p.info.ready = false;
                        }
                        dirty = true;
                    }
                    ClientMsg::HasMap(has) => {
                        p.info.has_map = has;
                        p.info.ready &= has;
                        dirty = true;
                    }
                    ClientMsg::Ready(r) if phase == Phase::Lobby => {
                        p.info.ready = r && p.info.has_map;
                        dirty = true;
                    }
                    ClientMsg::StartMatch if is_host && phase == Phase::Lobby => {
                        if let Some(m) = &map {
                            let mut any = false;
                            for p in players.iter_mut().filter(|p| p.hello && p.info.ready && p.info.has_map) {
                                p.in_match = true;
                                p.loaded = None;
                                p.info.status = Status::Loading;
                                p.info.result = None;
                                p.info.live = Live::default();
                                let _ = p.tx.send(ServerMsg::Prepare(m.clone()));
                                any = true;
                            }
                            if any {
                                phase = Phase::Loading { deadline: Instant::now() + LOAD_TIMEOUT };
                                dirty = true;
                            }
                        }
                    }
                    ClientMsg::Loaded(ok) => {
                        p.loaded = Some(ok);
                    }
                    ClientMsg::Live(l) => {
                        if p.in_match {
                            p.info.live = l;
                            dirty = true;
                        }
                    }
                    ClientMsg::Finished(f) => {
                        if p.in_match {
                            p.info.result = Some(f);
                            p.info.status = Status::Finished;
                            dirty = true;
                        }
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Everyone loaded (or the wait timed out): schedule the start.
        if let Phase::Loading { deadline } = phase {
            let waiting = players.iter().any(|p| p.in_match && p.loaded.is_none());
            if !waiting || Instant::now() >= deadline {
                let at = now_ms() + START_LEAD_MS;
                let mut started = false;
                for p in players.iter_mut().filter(|p| p.in_match) {
                    if p.loaded == Some(true) {
                        p.info.status = Status::Playing;
                        let _ = p.tx.send(ServerMsg::Start { at });
                        started = true;
                    } else {
                        p.in_match = false;
                        p.info.status = Status::Lobby;
                    }
                }
                phase = if started { Phase::Playing } else { Phase::Lobby };
                dirty = true;
            }
        }
        // Everyone still in the match has finished (or left): publish results, back to the lobby.
        if phase == Phase::Playing && players.iter().filter(|p| p.in_match).all(|p| p.info.status == Status::Finished) {
            let results: Vec<PlayerInfo> = players.iter().filter(|p| p.in_match).map(|p| p.info.clone()).collect();
            for p in &mut players {
                let _ = p.tx.send(ServerMsg::Results(results.clone()));
                p.in_match = false;
                p.info.ready = false;
                p.info.status = Status::Lobby;
            }
            phase = Phase::Lobby;
            dirty = true;
        }

        let interval = if phase == Phase::Lobby { Duration::ZERO } else { MATCH_BROADCAST };
        if dirty && last_broadcast.elapsed() >= interval {
            let room = ServerMsg::Room {
                players: players.iter().filter(|p| p.hello).map(|p| p.info.clone()).collect(),
                map: map.clone(),
                in_match: phase != Phase::Lobby,
            };
            for p in players.iter().filter(|p| p.hello) {
                let _ = p.tx.send(room.clone());
            }
            dirty = false;
            last_broadcast = Instant::now();
        }
    }
    // Dropping `players` closes every writer, which shuts the sockets.
}

// ---------------------------------------------------------------------------------------------
// Client side

pub struct Client {
    tx: Sender<ClientMsg>,
    /// Messages tagged with when they arrived, so ping round trips don't include poll delay.
    rx: Receiver<(Instant, ServerMsg)>,
    stream: TcpStream,
    epoch: Instant,
    best_rtt: f64,
    /// server_ms - client_ms, from the lowest-latency ping seen.
    offset: f64,
    pub rtt_ms: f64,
    pub id: Option<u32>,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Accepts `host`, `host:port`, `[v6]:port`; the port defaults to 7777.
fn resolve(addr: &str) -> Result<std::net::SocketAddr, String> {
    let addr = addr.trim();
    let with_port = if addr.parse::<std::net::SocketAddr>().is_ok() {
        addr.to_string()
    } else if addr.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{addr}]:{DEFAULT_PORT}")
    } else if addr.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        addr.to_string()
    } else {
        format!("{addr}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("can't resolve {addr}: {e}"))?
        .next()
        .ok_or_else(|| format!("can't resolve {addr}"))
}

impl Client {
    pub fn connect(addr: &str, name: &str, host_token: Option<u64>) -> Result<Client, String> {
        let sa = resolve(addr)?;
        let stream = TcpStream::connect_timeout(&sa, Duration::from_secs(5)).map_err(|e| format!("can't connect to {sa}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        let epoch = Instant::now();

        let (tx, out) = mpsc::channel::<ClientMsg>();
        thread::spawn(move || {
            for msg in out {
                if send_line(&mut writer, &msg).is_err() {
                    break;
                }
            }
        });
        let (inbox, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            while let Ok(Some(())) = read_message(&mut reader, &mut line) {
                if let Ok(msg) = serde_json::from_str::<ServerMsg>(&line) {
                    if inbox.send((Instant::now(), msg)).is_err() {
                        return;
                    }
                }
            }
            let _ = inbox.send((Instant::now(), ServerMsg::Disconnected("connection closed".into())));
        });
        // Ping quickly at first to sync clocks, then once a second to track drift.
        let pinger = tx.clone();
        thread::spawn(move || {
            for i in 0u32.. {
                let t = epoch.elapsed().as_secs_f64() * 1000.0;
                if pinger.send(ClientMsg::Ping { t }).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(if i < 10 { 100 } else { 1000 }));
            }
        });
        let _ = tx.send(ClientMsg::Hello { name: name.to_string(), protocol: PROTOCOL, host_token });
        Ok(Client { tx, rx, stream, epoch, best_rtt: f64::INFINITY, offset: 0.0, rtt_ms: 0.0, id: None })
    }

    pub fn send(&self, msg: ClientMsg) {
        let _ = self.tx.send(msg);
    }

    /// Drains incoming messages, handling clock sync internally.
    pub fn poll(&mut self) -> Vec<ServerMsg> {
        let mut out = Vec::new();
        while let Ok((arrived, msg)) = self.rx.try_recv() {
            match msg {
                ServerMsg::Pong { t, server } => {
                    let now = arrived.duration_since(self.epoch).as_secs_f64() * 1000.0;
                    let rtt = now - t;
                    self.rtt_ms = rtt;
                    // Trust the fastest round trip; let the bar rise slowly so clock drift is tracked.
                    self.best_rtt += 0.05;
                    if rtt <= self.best_rtt {
                        self.best_rtt = rtt;
                        self.offset = server + rtt / 2.0 - now;
                    }
                }
                ServerMsg::Welcome { id } => {
                    self.id = Some(id);
                    out.push(ServerMsg::Welcome { id });
                }
                other => out.push(other),
            }
        }
        out
    }

    /// Local instant corresponding to a server timestamp.
    pub fn local_instant(&self, server_ms: f64) -> Instant {
        let local_ms = (server_ms - self.offset).max(0.0);
        self.epoch + Duration::from_secs_f64(local_ms / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_addresses_with_default_port() {
        assert_eq!(resolve("127.0.0.1").unwrap().port(), DEFAULT_PORT);
        assert_eq!(resolve("127.0.0.1:9000").unwrap().port(), 9000);
        assert_eq!(resolve("::1").unwrap().port(), DEFAULT_PORT);
        assert_eq!(resolve("[::1]:9001").unwrap().port(), 9001);
    }

    fn wait_for<F: Fn(&ServerMsg) -> bool>(c: &mut Client, pred: F) -> ServerMsg {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(m) = c.poll().into_iter().find(|m| pred(m)) {
                return m;
            }
            assert!(Instant::now() < until, "timed out waiting for message");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn oversized_message_drops_the_connection() {
        let port = 48_000 + (std::process::id() % 1000) as u16;
        let _server = host(port).unwrap();
        let mut raw = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let junk = vec![b'x'; (MAX_LINE as usize) + 10];
        let _ = raw.write_all(&junk);
        raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 16];
        // The host closes the socket: read returns 0 (EOF) or an error, rather than hanging.
        let res = std::io::Read::read(&mut raw, &mut buf);
        assert!(matches!(res, Ok(0) | Err(_)), "connection still open: {res:?}");
    }

    #[test]
    fn full_match_over_localhost() {
        let port = 47_000 + (std::process::id() % 1000) as u16;
        let server = host(port).unwrap();
        let addr = format!("127.0.0.1:{port}");
        // An intruder racing in first must not become host.
        let mut b = Client::connect(&addr, "bob", None).unwrap();
        wait_for(&mut b, |m| matches!(m, ServerMsg::Welcome { .. }));
        let mut a = Client::connect(&addr, "alice", Some(server.token)).unwrap();
        wait_for(&mut a, |m| matches!(m, ServerMsg::Welcome { .. }));
        b.send(ClientMsg::SelectMap(MapInfo { hash: 7, artist: String::new(), title: String::new(), version: String::new() }));
        thread::sleep(Duration::from_millis(100));
        // Any state change makes the room broadcast again, so we can inspect it.
        a.send(ClientMsg::HasMap(false));
        let ServerMsg::Room { players, map, .. } = wait_for(&mut a, |m| matches!(m, ServerMsg::Room { players, .. } if players.len() == 2)) else { unreachable!() };
        assert!(map.is_none(), "non-host picked the map");
        assert!(players.iter().any(|p| p.name == "alice" && p.host));
        assert!(players.iter().any(|p| p.name == "bob" && !p.host));

        let map = MapInfo { hash: 42, artist: "a".into(), title: "t".into(), version: "v".into() };
        a.send(ClientMsg::SelectMap(map.clone()));
        thread::sleep(Duration::from_millis(100));
        for c in [&a, &b] {
            c.send(ClientMsg::HasMap(true));
            c.send(ClientMsg::Ready(true));
        }
        thread::sleep(Duration::from_millis(100));
        a.send(ClientMsg::StartMatch);
        for c in [&mut a, &mut b] {
            assert!(matches!(wait_for(c, |m| matches!(m, ServerMsg::Prepare(_))), ServerMsg::Prepare(m) if m.hash == 42));
            c.send(ClientMsg::Loaded(true));
        }
        let ServerMsg::Start { at: at_a } = wait_for(&mut a, |m| matches!(m, ServerMsg::Start { .. })) else { unreachable!() };
        let ServerMsg::Start { at: at_b } = wait_for(&mut b, |m| matches!(m, ServerMsg::Start { .. })) else { unreachable!() };
        assert_eq!(at_a, at_b);
        // Both clocks map the start to (nearly) the same local instant.
        let (ia, ib) = (a.local_instant(at_a), b.local_instant(at_b));
        let diff = if ia > ib { ia - ib } else { ib - ia };
        assert!(diff < Duration::from_millis(5), "start instants differ by {diff:?}");

        a.send(ClientMsg::Finished(Final { score: 900, ..Default::default() }));
        b.send(ClientMsg::Finished(Final { score: 1200, ..Default::default() }));
        let ServerMsg::Results(r) = wait_for(&mut a, |m| matches!(m, ServerMsg::Results(_))) else { unreachable!() };
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|p| p.name == "bob" && p.result.as_ref().unwrap().score == 1200));
    }
}
