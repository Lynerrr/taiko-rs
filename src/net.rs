//! Multiplayer networking: a host-authoritative room over TCP, one JSON message per line.
//!
//! Gameplay never crosses the network — everyone plays the same map locally with their own audio
//! clock — so this only carries lobby state, a synchronised start time and live scores. The host
//! runs the room on a background thread and joins it through localhost like any other player.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 7;
pub const DEFAULT_PORT: u16 = 7777;
/// Countdown between everyone having loaded and the song starting.
const START_LEAD_MS: f64 = 3000.0;
/// Players who haven't loaded the map by then are left out of the match.
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Live score broadcasts during a match.
const MATCH_BROADCAST: Duration = Duration::from_millis(100);
/// Longest accepted message; anything bigger drops the connection.
const MAX_LINE: u64 = 256 * 1024;
/// Mapguessr: how long a round lasts, how long its answer shows before the next, and guesses
/// per round.
pub const GUESS_ROUND: Duration = Duration::from_secs(45);
const GUESS_REVEAL: Duration = if cfg!(test) { Duration::from_millis(200) } else { Duration::from_secs(6) };
pub const GUESS_TRIES: u8 = 3;
/// Mapguessr: map list entries per `Library` message (well under `MAX_LINE`), and the most kept.
pub const LIBRARY_CHUNK: usize = 4000;
const MAX_LIBRARY: usize = 200_000;
/// Who mapguessr's notices in the lobby chat are from (not a name anyone can have).
pub const GUESSR_NAME: &str = "[mapguessr]";

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
    /// Speed modifier the host picked for the match.
    #[serde(default = "one")]
    pub rate: f64,
    /// Map time the match starts at (picked by the host in the preview); None = the beginning.
    #[serde(default)]
    pub from: Option<f64>,
    /// osu! beatmap set, so players without the map can download it.
    #[serde(default)]
    pub set_id: Option<u32>,
    /// Mods the host picked for the match (acronyms, e.g. "HD"); `rate` already includes DT/HT.
    #[serde(default)]
    pub mods: Vec<String>,
}

fn one() -> f64 {
    1.0
}

impl MapInfo {
    pub fn label(&self) -> String {
        let rate = if (self.rate - 1.0).abs() > 1e-6 { format!("  {:.2}x", self.rate) } else { String::new() };
        let from = self.from.map(|t| format!("  from {}:{:02}", (t / 1000.0) as u64 / 60, (t / 1000.0) as u64 % 60)).unwrap_or_default();
        let mods = if self.mods.is_empty() { String::new() } else { format!("  +{}", self.mods.concat()) };
        format!("{} - {} [{}]{rate}{mods}{from}", self.artist, self.title, self.version)
    }
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
    /// Watches matches instead of playing them.
    #[serde(default)]
    pub spectator: bool,
    /// Mapguessr: points this game, whether they got this round's map, guesses left in it, and
    /// how many maps their list has (None: not sent yet).
    #[serde(default)]
    pub points: u32,
    #[serde(default)]
    pub guessed: bool,
    #[serde(default)]
    pub tries: u8,
    #[serde(default)]
    pub maps: Option<u32>,
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
    /// A player without the lobby map asks the host for it.
    RequestMap,
    /// Host: a piece of the map set archive for player `to`.
    MapFile { to: u32, part: MapPart },
    /// During a match: presses judged since the last batch (map time, key). `anti_mash` is
    /// older builds' setting, always sent on (judging is `Mash::Nearest` now).
    Inputs { inputs: Vec<(f64, u8)>, anti_mash: bool },
    /// Start / stop watching the match in progress.
    Spectate(bool),
    /// Lobby: be a spectator (watch every match from its start) or a player.
    Spectator(bool),
    /// Host: hand the host role (picking maps, starting) to another player.
    GiveHost(u32),
    /// During a match: skipped the intro (to just before the first note).
    Skipped,
    /// A line of lobby chat.
    Chat(String),
    /// Mapguessr: part of our map list as (map hash, song key); `done` on the last part. A new
    /// list after a done one replaces it.
    Library { maps: Vec<(u64, u64)>, done: bool },
    /// Mapguessr, host: start a game of this many rounds.
    StartGuessr { rounds: u32 },
    /// Mapguessr: it's this map (any difficulty of the song counts: `song` is its song key).
    Guess { hash: u64, song: u64 },
}

/// A piece of a map set archive sent from the host's Songs folder. `data` is base64; the
/// transfer is complete when `offset + data` reaches `total`. `error` instead of data when the
/// host can't send it.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MapPart {
    /// Folder name for the set.
    pub name: String,
    pub total: u64,
    pub offset: u64,
    pub data: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerMsg {
    Welcome { id: u32 },
    Rejected(String),
    Pong { t: f64, server: f64 },
    /// `mapguessr`: the room plays mapguessr, where `common` maps are in every player's list
    /// (None while a list hasn't arrived).
    Room {
        players: Vec<PlayerInfo>,
        map: Option<MapInfo>,
        in_match: bool,
        #[serde(default)]
        mapguessr: bool,
        #[serde(default)]
        common: Option<u32>,
    },
    Prepare(MapInfo),
    /// Start the song at this server time (ms).
    Start { at: f64 },
    Results(Vec<PlayerInfo>),
    /// To the host: player `from` wants the lobby map's files.
    MapRequested { from: u32 },
    /// From the host: part of the lobby map's files.
    MapFile(MapPart),
    /// Watching the match: its map, when it started (server ms) and who plays (id, name).
    Spectate { map: MapInfo, at: f64, players: Vec<(u32, String)> },
    /// A player's presses, to spectators.
    Inputs { from: u32, inputs: Vec<(f64, u8)>, anti_mash: bool },
    /// A player skipped the intro, to spectators.
    Skipped { from: u32 },
    /// Lobby chat (also the recent lines, to someone who just joined).
    Chat { from: String, text: String },
    /// Mapguessr: round `round` of `rounds` shows map `hash` (in every player's list) from
    /// `from` (0-1, how far through its notes) for `seconds`.
    GuessRound { round: u32, rounds: u32, hash: u64, from: f64, seconds: u32 },
    /// Mapguessr: our guess was wrong; guesses left.
    GuessWrong { tries: u8 },
    /// Mapguessr: the round is over and it was `hash`; who got it and the points each got.
    /// `last`: the game ends with it.
    GuessReveal { hash: u64, got: Vec<(u32, u32)>, last: bool },
    /// Synthesised locally when the connection drops.
    Disconnected(String),
}

fn send_line<T: Serialize>(w: &mut TcpStream, msg: &T) -> std::io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(std::io::Error::other)?;
    line.push('\n');
    w.write_all(line.as_bytes())
}

/// Printable ASCII and Cyrillic: what the built-in font draws.
pub fn drawable_char(c: char) -> bool {
    c.is_ascii_graphic() || c == ' ' || matches!(c, 'А'..='я' | 'Ё' | 'ё')
}

/// A chat line: what the font draws, at most 200 characters, trimmed.
pub fn clean_chat(text: &str) -> String {
    let t: String = text.chars().filter(|c| drawable_char(*c)).take(200).collect();
    t.trim().to_string()
}

/// A chat line with its swear words starred out (same length, so the line reads the same).
/// Catches Russian and English swearing and slurs, also spelled with look-alike Latin letters
/// or digits ("xуй", "6ля") or stretched out ("бляяя", "fuuuck"); leaves ordinary words that
/// happen to contain one ("корабля", "застрахую", "хлеб") alone.
pub fn censor(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if swear_word(word) {
            out.extend(word.chars().map(|_| '*'));
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        // Digits and a few symbols stand in for letters ("п1зда", "f*ck", "@ss").
        if c.is_alphanumeric() || matches!(c, '@' | '$' | '*') {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Whether one word (letters, digits and stand-ins) is a swear word.
fn swear_word(word: &str) -> bool {
    if word.chars().count() < 3 {
        return false;
    }
    let lower = word.to_lowercase().replace('ё', "е");
    // As Cyrillic: Latin look-alikes and digits turned into the letters they stand for.
    let cyr: String = lower
        .chars()
        .filter_map(|c| {
            Some(match c {
                'a' | '@' => 'а',
                'b' => 'в',
                'c' => 'с',
                'e' => 'е',
                'h' => 'н',
                'k' => 'к',
                'm' => 'м',
                'o' | '0' => 'о',
                'p' => 'р',
                't' => 'т',
                'x' => 'х',
                'y' | 'u' => 'у',
                '3' => 'з',
                '6' => 'б',
                '4' => 'ч',
                '1' | 'i' => 'и',
                '*' => return None,
                c => c,
            })
        })
        .collect();
    // As Latin: digits and symbols turned into letters.
    let lat: String = lower
        .chars()
        .filter_map(|c| {
            Some(match c {
                '@' | '4' => 'a',
                '$' | '5' => 's',
                '0' => 'o',
                '1' | '!' => 'i',
                '3' => 'e',
                '*' => return None,
                c => c,
            })
        })
        .collect();
    [cyr.clone(), squeeze(&cyr)].iter().any(|w| russian_swear(w)) || [lat.clone(), squeeze(&lat)].iter().any(|w| english_swear(w))
}

/// Runs of three or more of one letter cut to one ("бляяя" -> "бля"); pairs stay ("shiite").
fn squeeze(w: &str) -> String {
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let run = chars[i..].iter().take_while(|&&c| c == chars[i]).count();
        let keep = if run >= 3 { 1 } else { run };
        out.extend(std::iter::repeat_n(chars[i], keep));
        i += run;
    }
    out
}

fn russian_swear(w: &str) -> bool {
    // Roots that are swearing wherever they turn up.
    const ANYWHERE: [&str; 7] = ["пизд", "пидор", "мудак", "мудил", "залуп", "гандон", "шлюх"];
    if ANYWHERE.iter().any(|r| w.contains(r)) {
        return true;
    }
    // At the start only: "скипидар", "спидран", "лапидарный" stay.
    if ["пидар", "пидр"].iter().any(|r| w.starts_with(r)) {
        return true;
    }
    // хуй, хуе, хуя, хуи anywhere but in "страхуй"; хую only at the start ("тихую", "плохую",
    // "психую").
    if ["хуй", "хуе", "хуя", "хуи"].iter().any(|r| w.match_indices(r).any(|(i, _)| !w[..i].ends_with("стра"))) || w.starts_with("хую") {
        return true;
    }
    // бля, блядь, блять: at the start of a word only ("корабля", "рубля"), not "бляха", "бляшка",
    // "блямба".
    if w.strip_prefix("бля").is_some_and(|rest| !rest.starts_with(['х', 'ш', 'м'])) {
        return true;
    }
    // сука and сучка in their forms, not "сукно" or "сучок" (a knot).
    const FORMS: [&str; 16] = ["сука", "суки", "суке", "суку", "сукой", "сучий", "сукин", "сукины", "сучка", "сучки", "сучке", "сучку", "сучкой", "сучара", "сучары", "сучье"];
    if FORMS.contains(&w) {
        return true;
    }
    // еб-: after a prefix ("заебал", "уебок", "долбоеб"), never inside a word ("хлеб", "небо",
    // "себе").
    const PREFIXES: [&str; 22] = [
        "", "за", "у", "вы", "на", "до", "от", "отъ", "про", "раз", "разъ", "рас", "по", "съ", "об", "объ", "подъ", "при", "пере", "недо", "долбо", "долба",
    ];
    PREFIXES.iter().any(|p| w.strip_prefix(p).is_some_and(|rest| rest.starts_with("еб") && !rest.starts_with("ебони")))
}

fn english_swear(w: &str) -> bool {
    const ANYWHERE: [&str; 11] = ["fuck", "shit", "bitch", "cunt", "nigger", "faggot", "whore", "asshole", "motherf", "blyat", "pizd"];
    const WHOLE: [&str; 18] = ["fck", "fuk", "fag", "fags", "slut", "sluts", "nigga", "niggas", "niggaz", "pussy", "retard", "retards", "suka", "huy", "hui", "xuy", "xyu", "xui"];
    ANYWHERE.iter().any(|r| w.contains(r)) || WHOLE.contains(&w) || ["nahui", "nahuy", "blyad", "ebat", "pidor", "pidar"].iter().any(|r| w.starts_with(r))
}

/// Keeps what the built-in font can draw and caps the length.
fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| drawable_char(*c)).take(20).collect();
    let n = n.trim().to_string();
    // Mapguessr's notices are from a name nobody else gets.
    if n.is_empty() || n.eq_ignore_ascii_case(GUESSR_NAME) { "player".into() } else { n }
}

// ---------------------------------------------------------------------------------------------
// Host side

/// A running room. Dropping it closes the room and disconnects everyone.
pub struct Server {
    stop: Arc<AtomicBool>,
    pub token: u64,
    /// Port actually listened on (useful when 0 was requested).
    pub port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

enum Event {
    /// A connection: its id, where to send to it, and a name that overrides its Hello's (the
    /// account name in a room on the online server).
    Joined(u32, Sender<ServerMsg>, Option<String>),
    Msg(u32, ClientMsg),
    Left(u32),
}

/// `mapguessr`: the room plays mapguessr instead of matches.
pub fn host(port: u16, mapguessr: bool) -> Result<Server, String> {
    // Listen on IPv6 and IPv4. On Linux the `::` socket is dual-stack, so the IPv4 bind just
    // fails as "in use"; on Windows `::` is IPv6-only and both are needed, or IPv4 friends
    // would be refused.
    let mut listeners = Vec::new();
    let mut last_err = None;
    let mut port = port;
    for addr in ["::", "0.0.0.0"] {
        match TcpListener::bind((addr, port)) {
            Ok(l) => {
                // With port 0 the OS picks one; the second family must use the same port.
                port = l.local_addr().map(|a| a.port()).unwrap_or(port);
                listeners.push(l)
            }
            Err(e) => last_err = Some(e),
        }
    }
    if listeners.is_empty() {
        let e = last_err.map(|e| e.to_string()).unwrap_or_default();
        return Err(format!("can't listen on port {port}: {e}"));
    }
    for l in &listeners {
        l.set_nonblocking(true).map_err(|e| e.to_string())?;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let token = new_token();
    let (events, rx) = mpsc::channel();
    {
        let stop = stop.clone();
        thread::Builder::new()
            .name("mp-accept".into())
            .spawn(move || {
                let mut next_id = 1;
                while !stop.load(Ordering::Relaxed) {
                    let mut accepted = false;
                    for listener in &listeners {
                        if let Ok((stream, _)) = listener.accept() {
                            let _ = stream.set_nonblocking(false);
                            spawn_connection(BufReader::new(stream), next_id, events.clone(), None);
                            next_id += 1;
                            accepted = true;
                        }
                    }
                    if !accepted {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    {
        let stop = stop.clone();
        thread::Builder::new().name("mp-room".into()).spawn(move || room_loop(rx, stop, token, None, mapguessr)).map_err(|e| e.to_string())?;
    }
    Ok(Server { stop, token, port })
}

/// Proves who opened a room. RandomState is seeded from the OS per process: unguessable for this.
fn new_token() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    h.finish()
}

/// A number below `n` (which isn't 0), at random.
fn random_below(n: usize) -> usize {
    (new_token() % n as u64) as usize
}

/// What a lobby list shows about a room.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub players: u32,
    pub host: String,
    pub map: Option<String>,
    pub in_match: bool,
    #[allow(dead_code)] // read by the online server (server/)
    pub mapguessr: bool,
}

/// A room run by the online server: connections are handed to it after their own handshake
/// there (account, which room). Dropping it closes the room.
#[allow(dead_code)] // used by the online server (server/)
pub struct Room {
    events: Sender<Event>,
    stop: Arc<AtomicBool>,
    next_id: AtomicU32,
    /// Given to whoever created the room, so it joins as host.
    pub token: u64,
    pub summary: Arc<Mutex<Summary>>,
}

impl Drop for Room {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[allow(dead_code)] // used by the online server (server/)
pub fn open_room(mapguessr: bool) -> Room {
    let stop = Arc::new(AtomicBool::new(false));
    let token = new_token();
    let summary = Arc::new(Mutex::new(Summary { mapguessr, ..Summary::default() }));
    let (events, rx) = mpsc::channel();
    {
        let (stop, summary) = (stop.clone(), summary.clone());
        thread::spawn(move || room_loop(rx, stop, token, Some(summary), mapguessr));
    }
    Room { events, stop, next_id: AtomicU32::new(1), token, summary }
}

#[allow(dead_code)] // used by the online server (server/)
impl Room {
    /// Hands over a connection (its reader may already hold buffered input). `name` replaces
    /// the one the client sends in Hello.
    pub fn attach(&self, reader: BufReader<TcpStream>, name: Option<String>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        spawn_connection(reader, id, self.events.clone(), name);
    }
}

fn spawn_connection(mut reader: BufReader<TcpStream>, id: u32, events: Sender<Event>, name: Option<String>) {
    let _ = reader.get_ref().set_nodelay(true);
    let Ok(mut writer) = reader.get_ref().try_clone() else { return };
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
        if events.send(Event::Joined(id, tx, name)).is_err() {
            return;
        }
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
    /// Overrides the name in Hello (see `Event::Joined`).
    forced_name: Option<String>,
    tx: Sender<ServerMsg>,
    hello: bool,
    in_match: bool,
    loaded: Option<bool>,
    spectating: bool,
}

/// A mapguessr game in progress.
struct GuessGame {
    rounds: u32,
    round: u32,
    /// Who plays it (everyone not spectating when it started).
    players: Vec<u32>,
    /// Songs already asked about.
    used: std::collections::HashSet<u64>,
    /// This round's map and its song key.
    answer: (u64, u64),
    ends: Instant,
    /// Showing the answer until then (or about to start the first round).
    reveal_until: Option<Instant>,
    /// Who got this round, with the points.
    got: Vec<(u32, u32)>,
    last: bool,
}

/// Each player's map list: map hash -> song key, and whether all of it has arrived.
type Libraries = std::collections::HashMap<u32, (std::collections::HashMap<u64, u64>, bool)>;

/// The maps (hash, song key) in every list of `ids`, leaving out songs in `used`.
fn common_maps(ids: &[u32], libraries: &Libraries, used: &std::collections::HashSet<u64>) -> Vec<(u64, u64)> {
    let lists: Option<Vec<&std::collections::HashMap<u64, u64>>> = ids.iter().map(|id| libraries.get(id).map(|l| &l.0)).collect();
    let Some(mut lists) = lists.filter(|l| !l.is_empty()) else { return Vec::new() };
    lists.sort_by_key(|l| l.len());
    let mut out: Vec<(u64, u64)> = lists[0].iter().filter(|(h, song)| !used.contains(*song) && lists[1..].iter().all(|l| l.contains_key(*h))).map(|(&h, &song)| (h, song)).collect();
    // The same order every time, so a pick doesn't depend on how the lists were stored.
    out.sort_unstable();
    out
}

/// A lobby chat line from mapguessr itself (not kept in the chat history).
fn notice(players: &[Player], text: String) {
    for p in players.iter().filter(|p| p.hello) {
        let _ = p.tx.send(ServerMsg::Chat { from: GUESSR_NAME.into(), text: text.clone() });
    }
}

#[derive(PartialEq)]
enum Phase {
    Lobby,
    Loading { deadline: Instant },
    Playing,
}

/// `summary`: kept up to date for the online server's lobby list. `mapguessr`: the room plays
/// mapguessr (see `GuessGame`) instead of matches.
fn room_loop(rx: Receiver<Event>, stop: Arc<AtomicBool>, token: u64, summary: Option<Arc<Mutex<Summary>>>, mapguessr: bool) {
    let epoch = Instant::now();
    let now_ms = || epoch.elapsed().as_secs_f64() * 1000.0;
    let mut players: Vec<Player> = Vec::new();
    let mut map: Option<MapInfo> = None;
    let mut phase = Phase::Lobby;
    let mut dirty = false;
    let mut last_broadcast = Instant::now();
    let mut host_id: Option<u32> = None;
    // Whoever opened the room (its machine runs the server); the host role can move around.
    let mut owner_id: Option<u32> = None;
    // The match in progress, for spectators: its map, start (server ms) and every player's
    // presses so far (with their mash setting).
    let mut match_map: Option<MapInfo> = None;
    let mut start_at: Option<f64> = None;
    let mut match_inputs: std::collections::HashMap<u32, (Vec<(f64, u8)>, bool)> = std::collections::HashMap::new();
    let mut match_skips: std::collections::HashSet<u32> = std::collections::HashSet::new();
    // Recent lobby chat (name, line) for whoever joins, and each player's last lines (flood limit).
    let mut chat: std::collections::VecDeque<(String, String)> = std::collections::VecDeque::new();
    let mut chat_times: std::collections::HashMap<u32, std::collections::VecDeque<Instant>> = std::collections::HashMap::new();
    // Mapguessr: everyone's map list, the game in progress, and how many maps all the players
    // have (None while a list is missing), worked out again when `common_dirty`.
    let mut libraries: Libraries = std::collections::HashMap::new();
    let mut guess: Option<GuessGame> = None;
    let mut common: Option<u32> = None;
    let mut common_dirty = false;

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Joined(id, tx, forced_name)) => {
                players.push(Player {
                    forced_name,
                    info: PlayerInfo {
                        id,
                        name: String::new(),
                        host: false,
                        ready: false,
                        has_map: false,
                        status: Status::Lobby,
                        live: Live::default(),
                        result: None,
                        spectator: false,
                        points: 0,
                        guessed: false,
                        tries: 0,
                        maps: None,
                    },
                    tx,
                    hello: false,
                    in_match: false,
                    loaded: None,
                    spectating: false,
                });
            }
            Ok(Event::Left(id)) => {
                players.retain(|p| p.info.id != id);
                chat_times.remove(&id);
                libraries.remove(&id);
                common_dirty = true;
                // A handed-over host leaving gives it back to the room's owner (or anyone left).
                if host_id == Some(id) {
                    host_id = owner_id.filter(|o| players.iter().any(|p| p.info.id == *o)).or_else(|| players.iter().find(|p| p.hello).map(|p| p.info.id));
                    for p in &mut players {
                        p.info.host = Some(p.info.id) == host_id;
                    }
                }
                dirty = true;
            }
            Ok(Event::Msg(id, msg)) => {
                let is_host = host_id == Some(id);
                // Map transfers are relayed between the host and one player, lobby only.
                let msg = match msg {
                    ClientMsg::RequestMap => {
                        if let (Some(h), true, true) = (host_id, phase == Phase::Lobby, map.is_some()) {
                            if let Some(host) = players.iter().find(|p| p.info.id == h) {
                                let _ = host.tx.send(ServerMsg::MapRequested { from: id });
                            }
                        }
                        continue;
                    }
                    ClientMsg::MapFile { to, part } => {
                        if is_host {
                            if let Some(dest) = players.iter().find(|p| p.info.id == to) {
                                let _ = dest.tx.send(ServerMsg::MapFile(part));
                            }
                        }
                        continue;
                    }
                    ClientMsg::GiveHost(to) if is_host => {
                        if players.iter().any(|q| q.info.id == to && q.hello) {
                            host_id = Some(to);
                            for q in &mut players {
                                q.info.host = q.info.id == to;
                            }
                            dirty = true;
                        }
                        continue;
                    }
                    ClientMsg::Inputs { inputs, anti_mash } => {
                        if players.iter().any(|p| p.info.id == id && p.in_match) {
                            let log = match_inputs.entry(id).or_default();
                            log.0.extend_from_slice(&inputs);
                            log.1 = anti_mash;
                            let msg = ServerMsg::Inputs { from: id, inputs, anti_mash };
                            for s in players.iter().filter(|p| p.spectating) {
                                let _ = s.tx.send(msg.clone());
                            }
                        }
                        continue;
                    }
                    ClientMsg::Skipped => {
                        if players.iter().any(|p| p.info.id == id && p.in_match) && match_skips.insert(id) {
                            for s in players.iter().filter(|p| p.spectating) {
                                let _ = s.tx.send(ServerMsg::Skipped { from: id });
                            }
                        }
                        continue;
                    }
                    ClientMsg::Chat(text) => {
                        // A room's chat is public: starred out for everyone.
                        let text = censor(&clean_chat(&text));
                        let Some(name) = players.iter().find(|p| p.info.id == id && p.hello).map(|p| p.info.name.clone()) else { continue };
                        // At most 5 lines per 3 s each.
                        let times = chat_times.entry(id).or_default();
                        times.retain(|t| t.elapsed() < Duration::from_secs(3));
                        if text.is_empty() || times.len() >= 5 {
                            continue;
                        }
                        times.push_back(Instant::now());
                        chat.push_back((name.clone(), text.clone()));
                        if chat.len() > 30 {
                            chat.pop_front();
                        }
                        for p in players.iter().filter(|p| p.hello) {
                            let _ = p.tx.send(ServerMsg::Chat { from: name.clone(), text: text.clone() });
                        }
                        continue;
                    }
                    ClientMsg::Spectate(on) => {
                        let roster: Vec<(u32, String)> = players.iter().filter(|p| p.in_match).map(|p| (p.info.id, p.info.name.clone())).collect();
                        let Some(p) = players.iter_mut().find(|p| p.info.id == id) else { continue };
                        p.spectating = false;
                        if let (true, Phase::Playing, false, Some(m), Some(at)) = (on, &phase, p.in_match, &match_map, start_at) {
                            p.spectating = true;
                            let _ = p.tx.send(ServerMsg::Spectate { map: m.clone(), at, players: roster });
                            // Everything played so far, so a late spectator catches up.
                            for (&from, (inputs, anti_mash)) in &match_inputs {
                                let _ = p.tx.send(ServerMsg::Inputs { from, inputs: inputs.clone(), anti_mash: *anti_mash });
                            }
                            for &from in &match_skips {
                                let _ = p.tx.send(ServerMsg::Skipped { from });
                            }
                        }
                        continue;
                    }
                    ClientMsg::Library { maps, done } if mapguessr => {
                        let list = libraries.entry(id).or_default();
                        if list.1 {
                            *list = Default::default();
                        }
                        let room = MAX_LIBRARY.saturating_sub(list.0.len());
                        list.0.extend(maps.into_iter().take(room));
                        if done {
                            list.1 = true;
                            let count = list.0.len() as u32;
                            if let Some(p) = players.iter_mut().find(|p| p.info.id == id) {
                                p.info.maps = Some(count);
                            }
                            common_dirty = true;
                            dirty = true;
                        }
                        continue;
                    }
                    ClientMsg::StartGuessr { rounds } if mapguessr && is_host && guess.is_none() => {
                        let ids: Vec<u32> = players.iter().filter(|p| p.hello && !p.info.spectator).map(|p| p.info.id).collect();
                        let waiting: Vec<String> = players.iter().filter(|p| ids.contains(&p.info.id) && !libraries.get(&p.info.id).is_some_and(|l| l.1)).map(|p| p.info.name.clone()).collect();
                        if !waiting.is_empty() {
                            notice(&players, format!("waiting for the map list of {}", waiting.join(", ")));
                        } else if ids.is_empty() {
                            notice(&players, "nobody is playing: everyone's spectating".into());
                        } else if common_maps(&ids, &libraries, &Default::default()).is_empty() {
                            notice(&players, "there's no map that everyone has".into());
                        } else {
                            for p in players.iter_mut().filter(|p| ids.contains(&p.info.id)) {
                                p.info.points = 0;
                            }
                            // The first round starts right away (see below).
                            let now = Instant::now();
                            guess = Some(GuessGame { rounds: rounds.clamp(1, 50), round: 0, players: ids, used: Default::default(), answer: (0, 0), ends: now, reveal_until: Some(now), got: Vec::new(), last: false });
                            dirty = true;
                        }
                        continue;
                    }
                    ClientMsg::Guess { song, .. } => {
                        let Some(g) = guess.as_mut().filter(|g| g.reveal_until.is_none() && g.players.contains(&id)) else { continue };
                        let Some(p) = players.iter_mut().find(|p| p.info.id == id && !p.info.guessed && p.info.tries > 0) else { continue };
                        if song == g.answer.1 {
                            // 100 for getting it, up to 100 more for getting it fast.
                            let left = g.ends.saturating_duration_since(Instant::now()).as_secs_f64() / GUESS_ROUND.as_secs_f64();
                            let points = 100 + (100.0 * left).round() as u32;
                            p.info.guessed = true;
                            p.info.points += points;
                            g.got.push((id, points));
                            let name = p.info.name.clone();
                            notice(&players, format!("{name} got it! (+{points})"));
                        } else {
                            p.info.tries -= 1;
                            let _ = p.tx.send(ServerMsg::GuessWrong { tries: p.info.tries });
                        }
                        dirty = true;
                        continue;
                    }
                    other => other,
                };
                let Some(p) = players.iter_mut().find(|p| p.info.id == id) else { continue };
                match msg {
                    ClientMsg::Hello { name, protocol, host_token } => {
                        // Only the client holding the room's token becomes host, however fast
                        // anyone else connects.
                        if host_token == Some(token) && host_id.is_none() {
                            host_id = Some(id);
                            owner_id = Some(id);
                            p.info.host = true;
                        }
                        if protocol != PROTOCOL {
                            let _ = p.tx.send(ServerMsg::Rejected(format!("version mismatch (host {PROTOCOL}, you {protocol})")));
                            players.retain(|p| p.info.id != id);
                            continue;
                        }
                        p.info.name = p.forced_name.clone().unwrap_or_else(|| clean_name(&name));
                        p.hello = true;
                        common_dirty = true;
                        let _ = p.tx.send(ServerMsg::Welcome { id });
                        for (from, text) in &chat {
                            let _ = p.tx.send(ServerMsg::Chat { from: from.clone(), text: text.clone() });
                        }
                        dirty = true;
                    }
                    ClientMsg::Ping { t } => {
                        let _ = p.tx.send(ServerMsg::Pong { t, server: now_ms() });
                    }
                    ClientMsg::SelectMap(m) if is_host && phase == Phase::Lobby && !mapguessr => {
                        // Same map at a new rate: everyone still has it, they just re-ready.
                        let same_map = map.as_ref().is_some_and(|old| old.hash == m.hash);
                        map = Some(m);
                        for p in &mut players {
                            if !same_map {
                                p.info.has_map = false;
                            }
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
                        p.info.ready = r && p.info.has_map && !p.info.spectator;
                        dirty = true;
                    }
                    ClientMsg::Spectator(on) => {
                        p.info.spectator = on;
                        if on {
                            p.info.ready = false;
                        }
                        common_dirty = true;
                        dirty = true;
                    }
                    ClientMsg::StartMatch if is_host && phase == Phase::Lobby && !mapguessr => {
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
                start_at = Some(at);
                match_map = map.clone();
                match_inputs.clear();
                match_skips.clear();
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
                // Spectators watch from the start.
                if let (true, Some(m)) = (started, &map) {
                    let roster: Vec<(u32, String)> = players.iter().filter(|p| p.in_match).map(|p| (p.info.id, p.info.name.clone())).collect();
                    for p in players.iter_mut().filter(|p| p.hello && p.info.spectator && !p.in_match) {
                        p.spectating = true;
                        let _ = p.tx.send(ServerMsg::Spectate { map: m.clone(), at, players: roster.clone() });
                    }
                }
                dirty = true;
            }
        }
        // Everyone still in the match has finished (or left): publish results, back to the lobby.
        if phase == Phase::Playing && players.iter().filter(|p| p.in_match).all(|p| p.info.status == Status::Finished) {
            let results: Vec<PlayerInfo> = players.iter().filter(|p| p.in_match).map(|p| p.info.clone()).collect();
            for p in &mut players {
                let _ = p.tx.send(ServerMsg::Results(results.clone()));
                p.in_match = false;
                p.spectating = false;
                p.info.ready = false;
                p.info.status = Status::Lobby;
            }
            phase = Phase::Lobby;
            dirty = true;
        }

        // Mapguessr: once the answer has shown, start the next round (or end the game); show the
        // answer once everyone has it or is out of guesses, or the time is up.
        if let Some(g) = &mut guess {
            g.players.retain(|id| players.iter().any(|p| p.info.id == *id));
            let now = Instant::now();
            match g.reveal_until {
                Some(until) if now >= until => {
                    let maps = common_maps(&g.players, &libraries, &g.used);
                    if g.last || g.players.is_empty() || maps.is_empty() {
                        for p in &mut players {
                            p.info.status = Status::Lobby;
                            p.info.guessed = false;
                            p.info.tries = 0;
                        }
                        guess = None;
                    } else {
                        let (hash, song) = maps[random_below(maps.len())];
                        g.round += 1;
                        g.answer = (hash, song);
                        g.used.insert(song);
                        g.reveal_until = None;
                        g.got.clear();
                        g.ends = now + GUESS_ROUND;
                        // Somewhere between a tenth and two thirds of the way through.
                        let from = 0.1 + random_below(1000) as f64 / 1000.0 * 0.55;
                        let msg = ServerMsg::GuessRound { round: g.round, rounds: g.rounds, hash, from, seconds: GUESS_ROUND.as_secs() as u32 };
                        for p in players.iter_mut().filter(|p| p.hello) {
                            if g.players.contains(&p.info.id) {
                                p.info.guessed = false;
                                p.info.tries = GUESS_TRIES;
                                p.info.status = Status::Playing;
                            }
                            let _ = p.tx.send(msg.clone());
                        }
                    }
                    dirty = true;
                }
                Some(_) => {}
                None => {
                    let done = |id: &u32| players.iter().find(|p| p.info.id == *id).is_none_or(|p| p.info.guessed || p.info.tries == 0);
                    if now >= g.ends || g.players.iter().all(done) {
                        g.last = g.round >= g.rounds || common_maps(&g.players, &libraries, &g.used).is_empty();
                        g.reveal_until = Some(now + GUESS_REVEAL);
                        let msg = ServerMsg::GuessReveal { hash: g.answer.0, got: g.got.clone(), last: g.last };
                        for p in players.iter().filter(|p| p.hello) {
                            let _ = p.tx.send(msg.clone());
                        }
                        dirty = true;
                    }
                }
            }
        }
        if mapguessr && std::mem::take(&mut common_dirty) {
            let ids: Vec<u32> = players.iter().filter(|p| p.hello && !p.info.spectator).map(|p| p.info.id).collect();
            let all_in = !ids.is_empty() && ids.iter().all(|id| libraries.get(id).is_some_and(|l| l.1));
            common = all_in.then(|| common_maps(&ids, &libraries, &Default::default()).len() as u32);
            dirty = true;
        }

        let in_match = phase != Phase::Lobby || guess.is_some();
        let interval = if in_match { MATCH_BROADCAST } else { Duration::ZERO };
        if dirty && last_broadcast.elapsed() >= interval {
            let room = ServerMsg::Room {
                players: players.iter().filter(|p| p.hello).map(|p| p.info.clone()).collect(),
                map: map.clone(),
                in_match,
                mapguessr,
                common,
            };
            for p in players.iter().filter(|p| p.hello) {
                let _ = p.tx.send(room.clone());
            }
            if let Some(s) = &summary {
                let mut s = s.lock().unwrap();
                s.players = players.iter().filter(|p| p.hello).count() as u32;
                s.host = players.iter().find(|p| p.info.host).map(|p| p.info.name.clone()).unwrap_or_default();
                s.map = map.as_ref().map(MapInfo::label);
                s.in_match = in_match;
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
        Client::over(BufReader::new(stream), name, host_token)
    }

    /// The room protocol over a connection that's already open (the online server's, after
    /// joining a room there; its reader may hold buffered input).
    pub fn over(reader: BufReader<TcpStream>, name: &str, host_token: Option<u64>) -> Result<Client, String> {
        // Timeouts are per handle on Windows: clear the request ones on the reader's own handle
        // too, not just the clone, or a quiet lobby drops after 10 s there.
        let _ = reader.get_ref().set_read_timeout(None);
        let _ = reader.get_ref().set_write_timeout(None);
        let stream = reader.get_ref().try_clone().map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(None);
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
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
            let mut reader = reader;
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

    /// For sending from another thread (big transfers).
    pub fn sender(&self) -> Sender<ClientMsg> {
        self.tx.clone()
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
    #[test]
    fn censor_stars_swearing_and_leaves_words_alone() {
        use super::censor;
        assert_eq!(censor("бля, ну ты и сука"), "***, ну ты и ****");
        assert_eq!(censor("ПИЗДЕЦ!!! нахуй это"), "******!!! ***** это");
        assert_eq!(censor("заебал уже, долбоёб"), "****** уже, *******");
        assert_eq!(censor("xуй 6ляяя п1зда"), "*** ***** *****");
        assert_eq!(censor("what the fuuuck, sh1t"), "what the ******, ****");
        assert_eq!(censor("f*ck you"), "**** you");
        // Ordinary words with a swear inside stay.
        for ok in ["корабля", "рубля", "застрахую", "хлеб", "небо", "себе", "сукно", "бляха", "бляшка", "спидран", "скипидар", "плохую", "тихую", "психую", "сучок", "оскорблять", "команда", "hello there", "shiitake"] {
            assert_eq!(censor(ok), ok);
        }
        assert_eq!(censor(""), "");
        // `TAIKO_CENSOR_WORDS=<file>`: print the words of a word list (one a line) it stars out.
        if let Some(f) = std::env::var_os("TAIKO_CENSOR_WORDS") {
            for w in std::fs::read_to_string(f).unwrap().lines() {
                if censor(w) != w {
                    println!("starred: {w}");
                }
            }
        }
        assert_eq!(censor("gg wp 300x"), "gg wp 300x");
    }

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
    fn room_accepts_ipv4_and_ipv6() {
        let server = host(0, false).unwrap();
        let port = server.port;
        let mut v4 = Client::connect(&format!("127.0.0.1:{port}"), "v4", None).unwrap();
        wait_for(&mut v4, |m| matches!(m, ServerMsg::Welcome { .. }));
        // IPv6 loopback may be disabled on some machines; only check it when it exists.
        if std::net::TcpListener::bind("[::1]:0").is_ok() {
            let mut v6 = Client::connect(&format!("[::1]:{port}"), "v6", None).unwrap();
            wait_for(&mut v6, |m| matches!(m, ServerMsg::Welcome { .. }));
        }
    }

    #[test]
    fn oversized_message_drops_the_connection() {
        let server = host(0, false).unwrap();
        let port = server.port;
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
    fn mapguessr_game_over_localhost() {
        let server = host(0, true).unwrap();
        let addr = format!("127.0.0.1:{}", server.port);
        let mut a = Client::connect(&addr, "alice", Some(server.token)).unwrap();
        wait_for(&mut a, |m| matches!(m, ServerMsg::Welcome { .. }));
        let mut b = Client::connect(&addr, "bob", None).unwrap();
        wait_for(&mut b, |m| matches!(m, ServerMsg::Welcome { .. }));
        // Maps 2 and 3 (songs 20 and 30) are the ones both have; bob's list comes in two parts.
        a.send(ClientMsg::Library { maps: vec![(1, 10), (2, 20), (3, 30)], done: true });
        b.send(ClientMsg::Library { maps: vec![(2, 20)], done: false });
        b.send(ClientMsg::Library { maps: vec![(3, 30), (4, 40)], done: true });
        wait_for(&mut a, |m| matches!(m, ServerMsg::Room { mapguessr: true, common: Some(2), .. }));
        // Only the host starts it.
        b.send(ClientMsg::StartGuessr { rounds: 5 });
        thread::sleep(Duration::from_millis(100));
        a.send(ClientMsg::StartGuessr { rounds: 5 });
        let ServerMsg::GuessRound { round: 1, hash, from, .. } = wait_for(&mut a, |m| matches!(m, ServerMsg::GuessRound { .. })) else { unreachable!() };
        assert!([2, 3].contains(&hash) && (0.1..0.7).contains(&from));
        assert!(matches!(wait_for(&mut b, |m| matches!(m, ServerMsg::GuessRound { .. })), ServerMsg::GuessRound { hash: h, .. } if h == hash));
        let song = hash * 10;
        // Alice gets it (any difficulty of the song: another hash, same song key); bob runs out.
        a.send(ClientMsg::Guess { hash: 99, song });
        for left in (0..GUESS_TRIES).rev() {
            b.send(ClientMsg::Guess { hash: 1, song: 10 });
            assert!(matches!(wait_for(&mut b, |m| matches!(m, ServerMsg::GuessWrong { .. })), ServerMsg::GuessWrong { tries } if tries == left));
        }
        let ServerMsg::GuessReveal { hash: answer, got, last } = wait_for(&mut b, |m| matches!(m, ServerMsg::GuessReveal { .. })) else { unreachable!() };
        assert_eq!(answer, hash);
        assert!(!last);
        assert!(got.len() == 1 && got[0].1 > 100 && got[0].1 <= 200, "{got:?}");
        // The other map's song next, and then it's over: there's no third map everyone has.
        let ServerMsg::GuessRound { round: 2, hash: second, .. } = wait_for(&mut a, |m| matches!(m, ServerMsg::GuessRound { .. })) else { unreachable!() };
        assert_ne!(second, hash);
        a.send(ClientMsg::Guess { hash: second, song: second * 10 });
        b.send(ClientMsg::Guess { hash: second, song: second * 10 });
        assert!(matches!(wait_for(&mut a, |m| matches!(m, ServerMsg::GuessReveal { .. })), ServerMsg::GuessReveal { last: true, ref got, .. } if got.len() == 2));
        let ServerMsg::Room { players, .. } = wait_for(&mut a, |m| matches!(m, ServerMsg::Room { in_match: false, players, .. } if players.iter().any(|p| p.points > 0))) else { unreachable!() };
        let points = |n: &str| players.iter().find(|p| p.name == n).map(|p| p.points).unwrap();
        assert!(points("alice") > points("bob") && points("bob") > 100);
    }

    #[test]
    fn full_match_over_localhost() {
        let server = host(0, false).unwrap();
        let port = server.port;
        let addr = format!("127.0.0.1:{port}");
        // An intruder racing in first must not become host.
        let mut b = Client::connect(&addr, "bob", None).unwrap();
        wait_for(&mut b, |m| matches!(m, ServerMsg::Welcome { .. }));
        let mut a = Client::connect(&addr, "alice", Some(server.token)).unwrap();
        wait_for(&mut a, |m| matches!(m, ServerMsg::Welcome { .. }));
        b.send(ClientMsg::SelectMap(MapInfo { hash: 7, artist: String::new(), title: String::new(), version: String::new(), rate: 1.0, from: None, set_id: None, mods: Vec::new() }));
        thread::sleep(Duration::from_millis(100));
        // Any state change makes the room broadcast again, so we can inspect it.
        a.send(ClientMsg::HasMap(false));
        let ServerMsg::Room { players, map, .. } = wait_for(&mut a, |m| matches!(m, ServerMsg::Room { players, .. } if players.len() == 2)) else { unreachable!() };
        assert!(map.is_none(), "non-host picked the map");
        assert!(players.iter().any(|p| p.name == "alice" && p.host));
        assert!(players.iter().any(|p| p.name == "bob" && !p.host));

        let map = MapInfo { hash: 42, artist: "a".into(), title: "t".into(), version: "v".into(), rate: 1.5, from: Some(30_000.0), set_id: Some(9), mods: vec!["HD".into()] };
        a.send(ClientMsg::SelectMap(map.clone()));
        thread::sleep(Duration::from_millis(100));
        for c in [&a, &b] {
            c.send(ClientMsg::HasMap(true));
            c.send(ClientMsg::Ready(true));
        }
        thread::sleep(Duration::from_millis(100));
        a.send(ClientMsg::StartMatch);
        for c in [&mut a, &mut b] {
            assert!(matches!(wait_for(c, |m| matches!(m, ServerMsg::Prepare(_))), ServerMsg::Prepare(m) if m.hash == 42 && m.rate == 1.5));
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
