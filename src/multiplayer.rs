//! Multiplayer session state and screens: host/join menu, lobby, in-match scoreboard, results.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crate::game::{Game, Stats};
use crate::net::{self, Client, Final, Live, MapInfo, PlayerInfo, Server, Status};
use crate::online;
use crate::render::{accent, rgba, Canvas, Hit};
use winit::keyboard::KeyCode;

pub struct Session {
    pub client: Client,
    /// Present when we host the room.
    pub server: Option<Server>,
    pub players: Vec<PlayerInfo>,
    pub map: Option<MapInfo>,
    pub in_match: bool,
    pub results: Option<Vec<PlayerInfo>>,
    pub ready: bool,
    pub last_live: Instant,
    pub label: String,
    /// Last map hash we reported HasMap for, and whether we have it.
    pub reported_map: Option<u64>,
    pub have_map: bool,
    /// Our library entry for the lobby map.
    pub map_entry: Option<usize>,
    /// Presses of the current match already sent to the room.
    pub inputs_sent: usize,
    /// Lobby chat: (name, line), oldest first; and lines not seen yet.
    pub chat: Vec<(String, String)>,
    pub chat_unread: usize,
    /// The lobby's id on the online server (friends can join it from their list).
    pub online_room: Option<u32>,
    /// The room plays mapguessr; how many maps every player has (None: a list is missing).
    pub mapguessr: bool,
    pub common: Option<u32>,
}

impl Session {
    fn new(client: Client, server: Option<Server>, label: String) -> Session {
        Session {
            client,
            server,
            players: Vec::new(),
            map: None,
            in_match: false,
            results: None,
            ready: false,
            last_live: Instant::now(),
            label,
            reported_map: None,
            have_map: false,
            map_entry: None,
            inputs_sent: 0,
            chat: Vec::new(),
            chat_unread: 0,
            online_room: None,
            mapguessr: false,
            common: None,
        }
    }

    pub fn me(&self) -> Option<&PlayerInfo> {
        let id = self.client.id?;
        self.players.iter().find(|p| p.id == id)
    }

    /// Has the host role: picks maps and starts matches (it can be handed over).
    pub fn is_host(&self) -> bool {
        self.me().is_some_and(|p| p.host)
    }

    /// This machine runs the room (closing it ends it for everyone).
    pub fn owns_room(&self) -> bool {
        self.server.is_some()
    }
}

pub type Connecting = Receiver<Result<Session, String>>;

/// Opens a room (`mapguessr`: one playing mapguessr) and joins it ourselves, on a background
/// thread so the UI never stalls.
pub fn start_host(port: u16, name: String, mapguessr: bool) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let server = net::host(port, mapguessr)?;
            let token = Some(server.token);
            let client = Client::connect(&format!("127.0.0.1:{port}"), &name, token)
                .or_else(|_| Client::connect(&format!("[::1]:{port}"), &name, token))?;
            let label = format!("hosting on port {}", server.port);
            Ok(Session::new(client, Some(server), label))
        })();
        let _ = tx.send(result);
    });
    rx
}

pub fn start_join(address: String, name: String) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = Client::connect(&address, &name, None).map(|c| Session::new(c, None, format!("connected to {address}")));
        let _ = tx.send(result);
    });
    rx
}

pub fn live_from(game: &Game) -> Live {
    let s = &game.stats;
    Live { score: s.score, combo: s.combo, accuracy: s.accuracy(), miss: s.miss, progress: game.progress() }
}

pub fn final_from(stats: &Stats, quit: bool) -> Final {
    Final {
        score: stats.score,
        accuracy: stats.accuracy(),
        max_combo: stats.max_combo,
        great: stats.great,
        ok: stats.ok,
        miss: stats.miss,
        ur: stats.unstable_rate(),
        quit,
    }
}

// ---------------------------------------------------------------------------------------------
// Host / join menu

/// The multiplayer menu's rows, top to bottom: the online column (an account, then lobbies on
/// the server), then direct play (host on your own port / join by address) as the fallback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    User,
    Password,
    /// For a new account (where codes go), or an old one's to link.
    Email,
    /// The code mailed (`Menu::code_for`).
    Code,
    LogIn,
    Register,
    /// Forgot the password: a code to the account's email, then a new one.
    Forgot,
    /// Logged in without an email: link one (opens `Menu::linking`, then sends the code and
    /// checks it).
    LinkEmail,
    /// Out of linking an email, back to the lobbies.
    LinkBack,
    HostOnline,
    HostGuessr,
    Lobby(usize),
    LogOut,
    Name,
    Port,
    HostDirect,
    Address,
    JoinDirect,
}

impl Row {
    /// Enter (or a click) does something; other rows are text fields.
    pub fn is_action(self) -> bool {
        !matches!(self, Row::User | Row::Password | Row::Email | Row::Code | Row::Name | Row::Port | Row::Address)
    }
}

/// What a mailed code is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeFor {
    Register,
    Reset,
    Link,
}

#[derive(Default)]
pub struct Menu {
    pub cursor: usize,
    /// The login form.
    pub user: String,
    pub password: String,
    pub email: String,
    pub code: String,
    /// A code was mailed for this (the Code row shows), and the one asked for, until answered.
    pub code_for: Option<CodeFor>,
    pub asking: Option<CodeFor>,
    /// Logged in as (a saved session).
    pub account: Option<String>,
    /// Whether the account has an email (None: the server doesn't say).
    pub has_email: Option<bool>,
    /// Linking an email: its rows in place of the lobbies.
    pub linking: bool,
    pub rooms: Vec<online::RoomInfo>,
    /// Whether the server answered the last request (None: not asked yet).
    pub online: Option<Result<(), String>>,
    pub account_request: Option<Receiver<Result<online::Reply, String>>>,
    pub rooms_request: Option<Receiver<Result<Vec<online::RoomInfo>, String>>>,
    pub last_refresh: Option<Instant>,
    /// The saved session was loaded from the config (and is being checked).
    pub session_loaded: bool,
}

/// Lobbies shown at once (the list scrolls with the cursor).
const LOBBY_ROWS: usize = 5;

impl Menu {
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.account.is_some() && self.linking {
            rows.push(Row::Email);
            if self.code_for == Some(CodeFor::Link) {
                rows.push(Row::Code);
            }
            rows.extend([Row::LinkEmail, Row::LinkBack]);
        } else if self.account.is_some() {
            rows.push(Row::HostOnline);
            rows.push(Row::HostGuessr);
            rows.extend((0..self.rooms.len()).map(Row::Lobby));
            rows.push(Row::LogOut);
            if self.has_email == Some(false) {
                rows.push(Row::LinkEmail);
            }
        } else {
            rows.extend([Row::User, Row::Password, Row::Email]);
            if matches!(self.code_for, Some(CodeFor::Register | CodeFor::Reset)) {
                rows.push(Row::Code);
            }
            rows.extend([Row::LogIn, Row::Register, Row::Forgot]);
        }
        rows.extend([Row::Name, Row::Port, Row::HostDirect, Row::Address, Row::JoinDirect]);
        rows
    }

    pub fn row(&self) -> Row {
        let rows = self.rows();
        rows[self.cursor.min(rows.len() - 1)]
    }

    pub fn select(&mut self, row: Row) {
        if let Some(i) = self.rows().iter().position(|r| *r == row) {
            self.cursor = i;
        }
    }

    /// Asks the server for its lobbies (in the background).
    pub fn refresh(&mut self, server: &str) {
        if self.rooms_request.is_some() {
            return;
        }
        self.last_refresh = Some(Instant::now());
        let (tx, rx) = mpsc::channel();
        let server = server.to_string();
        std::thread::spawn(move || {
            let result = online::call(&server, online::Request::ListRooms).and_then(|r| match r {
                online::Reply::Rooms(rooms) => Ok(rooms),
                _ => Err(crate::lang::odd_reply()),
            });
            let _ = tx.send(result);
        });
        self.rooms_request = Some(rx);
    }

    /// Sends an account request (log in, register, log out, check a session) in the background.
    pub fn account_call(&mut self, server: &str, request: online::Request) {
        let (tx, rx) = mpsc::channel();
        let server = server.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(online::call(&server, request));
        });
        self.account_request = Some(rx);
    }
}

/// Creates or joins a lobby on the online server; the connection then becomes the room's.
pub fn start_online(server: String, request: online::Request, name: String) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| match online::exchange(&server, request)? {
            (online::Reply::Joined { room, title, host_token }, reader) => {
                let client = Client::over(reader, &name, host_token)?;
                let mut s = Session::new(client, None, format!("online: {title}"));
                s.online_room = Some(room);
                Ok(s)
            }
            (online::Reply::Error(e), _) => Err(e),
            _ => Err(crate::lang::odd_reply()),
        })();
        let _ = tx.send(result);
    });
    rx
}

/// Text shown for the server in the menu, without the default port.
fn server_label(server: &str) -> &str {
    server.strip_suffix(&format!(":{}", online::DEFAULT_PORT)).unwrap_or(server)
}

#[allow(clippy::too_many_arguments)]
pub fn draw_menu(c: &mut Canvas, menu: &Menu, name: &str, port: u16, address: &str, server: &str, status: &str, busy: bool) {
    let w = c.width;
    crate::settings::tab_bar(c, 2, "");
    let gap = 20.0;
    let col = ((w - 48.0 - gap) / 2.0).min(380.0);
    let left = w / 2.0 - gap / 2.0 - col;
    let right = w / 2.0 + gap / 2.0;
    let rows = menu.rows();
    let cursor = menu.cursor.min(rows.len() - 1);

    // One row: a text field (label left, value right) or an action (centred).
    let row = |c: &mut Canvas, i: usize, x: f32, y: f32, h: f32, label: &str, value: Option<(&str, bool)>| {
        let sel = i == cursor;
        let (bg, border) = if sel { (0x34344a, accent()) } else { (0x1d1d27, 0x2a2a38) };
        let bg = if !sel && c.hovered(x, y, col, h) { 0x262634 } else { bg };
        c.round_rect(x, y, col, h, 7.0, rgba(bg, 1.0), rgba(border, 1.0), if sel { 2.0 } else { 1.0 });
        c.hit(x, y, col, h, Hit::MpRow(i));
        match value {
            Some((v, placeholder)) => {
                c.text(x + 14.0, y + h / 2.0 - 4.5, 9.0, rgba(0x9a9ab0, 1.0), label);
                // The value keeps clear of the label: a long one shows its end (where you type).
                let room = col - 38.0 - Canvas::text_width(9.0, label);
                let shown = if sel { format!("{v}_") } else { v.to_string() };
                let shown = if placeholder { Canvas::fit(9.0, room, &shown) } else { Canvas::fit_end(9.0, room, &shown) };
                let colour = if placeholder && !sel { 0x55556a } else { 0xffffff };
                c.text_right(x + col - 14.0, y + h / 2.0 - 4.5, 9.0, rgba(colour, 1.0), &shown);
            }
            None => c.text_centered(x + col / 2.0, y + h / 2.0 - 5.5, 11.0, rgba(if sel { 0xffffff } else { 0xc8c8d8 }, 1.0), label),
        }
    };

    // Online.
    c.text(left, 62.0, 9.0, rgba(accent(), 1.0), "ONLINE");
    let (state, colour) = match &menu.online {
        Some(Ok(())) => ("online", 0x86E05A),
        Some(Err(_)) => ("offline", 0xFF4B4B),
        None => ("connecting...", 0x8a8aa0),
    };
    let room = col - Canvas::text_width(9.0, "ONLINE") - 12.0;
    c.text_right(left + col, 62.0, 8.0, rgba(colour, 1.0), &Canvas::fit(8.0, room, &format!("{}  {state}", server_label(server))));
    let mut y = 80.0;
    let lobby_rows: Vec<(usize, usize)> = rows.iter().enumerate().filter_map(|(i, r)| if let Row::Lobby(k) = r { Some((i, *k)) } else { None }).collect();
    // The visible window of lobbies follows the cursor.
    let first = lobby_rows.iter().position(|(i, _)| *i == cursor).map(|p| p.saturating_sub(LOBBY_ROWS - 1)).unwrap_or(0);
    for (i, r) in rows.iter().enumerate() {
        match *r {
            Row::User => {
                row(c, i, left, y, 30.0, "Username", Some(if menu.user.is_empty() { (online::NAME_RULES, true) } else { (&menu.user, false) }));
                y += 38.0;
            }
            Row::Password => {
                let stars = "*".repeat(menu.password.chars().count());
                let label = if menu.code_for == Some(CodeFor::Reset) { "New password" } else { "Password" };
                row(c, i, left, y, 30.0, label, Some((&stars, false)));
                y += 38.0;
            }
            Row::Email => {
                let hint = if menu.account.is_some() { "your email" } else { "for a new account" };
                row(c, i, left, y, 30.0, "Email", Some(if menu.email.is_empty() { (hint, true) } else { (&menu.email, false) }));
                y += 38.0;
            }
            Row::Code => {
                row(c, i, left, y, 30.0, "Code from the email", Some(if menu.code.is_empty() { ("6 digits", true) } else { (&menu.code, false) }));
                y += 38.0;
            }
            Row::Forgot => {
                let label = if menu.code_for == Some(CodeFor::Reset) { "SET NEW PASSWORD" } else { "FORGOT PASSWORD" };
                y += 6.0;
                row(c, i, left, y, 24.0, label, None);
            }
            Row::LinkEmail if menu.linking => {
                let label = if menu.code_for == Some(CodeFor::Link) { "LINK EMAIL" } else { "SEND CODE" };
                row(c, i, left, y, 30.0, label, None);
                y += 38.0;
            }
            Row::LinkBack => {
                row(c, i, left, y, 24.0, "BACK", None);
                y += 32.0;
                for (k, line) in ["A code goes to the email: type it above and LINK EMAIL.", "Then a forgotten password can be reset with it."].iter().enumerate() {
                    let size = Canvas::fit_size(7.0, 6.0, col, line);
                    c.text_centered(left + col / 2.0, y + 4.0 + 14.0 * k as f32, size, rgba(0x6f6f86, 1.0), &Canvas::fit(size, col, line));
                }
            }
            Row::LogIn => {
                row(c, i, left, y, 30.0, "LOG IN", None);
                y += 38.0;
            }
            Row::Register => {
                row(c, i, left, y, 30.0, "CREATE ACCOUNT", None);
                y += 38.0;
                for (k, line) in ["An account is only needed for online lobbies.", "Direct play on the right works without one."].iter().enumerate() {
                    let size = Canvas::fit_size(7.0, 6.0, col, line);
                    c.text_centered(left + col / 2.0, y + 4.0 + 14.0 * k as f32, size, rgba(0x6f6f86, 1.0), &Canvas::fit(size, col, line));
                }
                y += 30.0;
            }
            Row::HostOnline => {
                let who = menu.account.as_deref().unwrap_or("");
                c.text(left, y - 2.0, 8.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(8.0, col, &format!("logged in as {who}")));
                y += 12.0;
                if menu.has_email == Some(false) {
                    let line = "no email yet: link one to reset a lost password";
                    let size = Canvas::fit_size(7.0, 6.0, col, line);
                    c.text(left, y - 2.0, size, rgba(0xFFD34D, 1.0), &Canvas::fit(size, col, line));
                    y += 11.0;
                }
                row(c, i, left, y, 30.0, "HOST AN ONLINE LOBBY", None);
                y += 36.0;
            }
            Row::HostGuessr => {
                row(c, i, left, y, 30.0, "HOST A MAPGUESSR LOBBY", None);
                y += 42.0;
                c.text(left, y, 8.0, rgba(accent(), 1.0), &format!("LOBBIES ({})", menu.rooms.len()));
                y += 14.0;
                if menu.rooms.is_empty() {
                    let msg = match &menu.online {
                        Some(Err(e)) => format!("server unreachable ({e}): use direct play"),
                        _ => "nobody is hosting right now".into(),
                    };
                    c.text(left + 4.0, y + 4.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, col - 8.0, &msg));
                    y += 22.0;
                }
            }
            Row::Lobby(k) => {
                let shown = lobby_rows.iter().position(|(j, _)| *j == i).is_some_and(|p| p >= first && p < first + LOBBY_ROWS);
                if !shown {
                    continue;
                }
                let Some(r) = menu.rooms.get(k) else { continue };
                let sel = i == cursor;
                let (bg, border) = if sel { (0x34344a, accent()) } else { (0x1d1d27, 0x2a2a38) };
                let bg = if !sel && c.hovered(left, y, col, 32.0) { 0x262634 } else { bg };
                c.round_rect(left, y, col, 32.0, 7.0, rgba(bg, 1.0), rgba(border, 1.0), if sel { 2.0 } else { 1.0 });
                c.hit(left, y, col, 32.0, Hit::MpRow(i));
                let state = if r.in_match { "in a match" } else { "in the lobby" };
                c.text(left + 12.0, y + 6.0, 9.0, rgba(0xffffff, 1.0), &Canvas::fit(9.0, col - 32.0 - Canvas::text_width(8.0, state), &r.title));
                let players = format!("{} player{}", r.players, if r.players == 1 { "" } else { "s" });
                c.text_right(left + col - 12.0, y + 6.0, 8.0, rgba(if r.in_match { 0x438EAD } else { 0x86E05A }, 1.0), state);
                let mut info = format!("host {}   {players}", r.host);
                if r.mapguessr {
                    info = format!("MAPGUESSR   {info}");
                }
                if let Some(m) = &r.map {
                    info.push_str("   ");
                    info.push_str(m);
                }
                c.text(left + 12.0, y + 20.0, 6.5, rgba(0x8a8aa0, 1.0), &Canvas::fit(6.5, col - 24.0, &info));
                y += 36.0;
            }
            Row::LogOut => {
                if lobby_rows.len() > LOBBY_ROWS {
                    c.text_centered(left + col / 2.0, y, 7.0, rgba(0x6f6f86, 1.0), &format!("{} more (Up/Down)", lobby_rows.len() - LOBBY_ROWS));
                    y += 12.0;
                }
                y += 4.0;
                // Beside it, linking an email.
                let half = if menu.has_email == Some(false) { (col - 8.0) / 2.0 } else { col };
                half_row(c, i == cursor, i, left, y, half, "LOG OUT");
                if half < col {
                    half_row(c, i + 1 == cursor, i + 1, left + half + 8.0, y, half, "LINK AN EMAIL");
                }
            }
            _ => {}
        }
    }

    // Direct play.
    c.text(right, 62.0, 9.0, rgba(accent(), 1.0), "DIRECT (P2P)");
    let room = col - Canvas::text_width(9.0, "DIRECT (P2P)") - 12.0;
    let note = "no account or server needed";
    let size = Canvas::fit_size(8.0, 6.0, room, note);
    c.text_right(right + col, 62.0 + (8.0 - size) / 2.0, size, rgba(0x6f6f86, 1.0), &Canvas::fit(size, room, note));
    let mut y = 80.0;
    for (i, r) in rows.iter().enumerate() {
        match *r {
            Row::Name => row(c, i, right, y, 30.0, "Your name", Some((name, false))),
            Row::Port => row(c, i, right, y, 30.0, "Host on port", Some((&port.to_string(), false))),
            Row::HostDirect => row(c, i, right, y, 30.0, "HOST A GAME", None),
            Row::Address => {
                y += 10.0;
                row(c, i, right, y, 30.0, "Join address", Some(if address.is_empty() { ("ip or ip:port", true) } else { (address, false) }));
            }
            Row::JoinDirect => row(c, i, right, y, 30.0, "JOIN GAME", None),
            _ => continue,
        }
        y += 38.0;
    }
    for (k, line) in ["Hosting: friends join with your public IP and", "this port (it must be reachable)."].iter().enumerate() {
        let size = Canvas::fit_size(7.0, 6.0, col, line);
        c.text_centered(right + col / 2.0, y + 4.0 + 12.0 * k as f32, size, rgba(0x6f6f86, 1.0), &Canvas::fit(size, col, line));
    }

    let msg = if busy { "connecting..." } else { status };
    let room = w - 48.0;
    c.text_centered(w / 2.0, 432.0, 9.0, rgba(0xFFD34D, 1.0), &Canvas::fit(9.0, room, msg));
    let note = "Everyone needs the same map in their Songs folder (or gets it from the host).";
    let size = Canvas::fit_size(8.0, 6.5, room, note);
    c.text_centered(w / 2.0, 450.0, size, rgba(0x8a8aa0, 1.0), &Canvas::fit(size, room, note));
    c.text_centered(w / 2.0, 464.0, 8.0, rgba(0x55556a, 1.0), &Canvas::fit_hints(8.0, room, "Up/Down select   Left/Right column   type to edit   Enter   F5 refresh   Tab or Esc back"));
}

/// An action row narrower than a column (two side by side), 24 tall.
fn half_row(c: &mut Canvas, sel: bool, i: usize, x: f32, y: f32, w: f32, label: &str) {
    let (bg, border) = if sel { (0x34344a, accent()) } else { (0x1d1d27, 0x2a2a38) };
    let bg = if !sel && c.hovered(x, y, w, 24.0) { 0x262634 } else { bg };
    c.round_rect(x, y, w, 24.0, 7.0, rgba(bg, 1.0), rgba(border, 1.0), if sel { 2.0 } else { 1.0 });
    c.hit(x, y, w, 24.0, Hit::MpRow(i));
    let size = Canvas::fit_size(11.0, 7.0, w - 12.0, label);
    c.text_centered(x + w / 2.0, y + 12.0 - size / 2.0, size, rgba(if sel { 0xffffff } else { 0xc8c8d8 }, 1.0), label);
}

// ---------------------------------------------------------------------------------------------
// Lobby

/// `map_chosen`: "no map" only means something once the host has picked one.
fn status_text(p: &PlayerInfo, map_chosen: bool) -> (&'static str, u32) {
    match p.status {
        Status::Lobby if p.spectator => ("spectator", 0x4fb4ff),
        Status::Loading => ("loading", 0x9a9ab0),
        Status::Playing => ("playing", 0x438EAD),
        Status::Finished => ("finished", 0x9a9ab0),
        Status::Lobby if map_chosen && !p.has_map => ("no map", 0xFF4B4B),
        Status::Lobby if p.ready => ("ready", 0x86E05A),
        Status::Lobby => ("not ready", 0x8a8aa0),
    }
}

/// `download`: progress of fetching the lobby map, when it's being downloaded.
pub fn draw_lobby(c: &mut Canvas, s: &Session, have_map: bool, download: Option<String>) {
    let w = c.width;
    let right = format!("{}   ping {:.0} ms", s.label, s.client.rtt_ms);
    crate::settings::tab_bar(c, 2, &right);
    let col = (w - 48.0).min(640.0);
    let x0 = (w - col) / 2.0;

    let can_preview = s.map.is_some() && have_map;
    let hover = can_preview && c.hovered(x0, 62.0, col, 46.0);
    c.round_rect(x0, 62.0, col, 46.0, 8.0, rgba(if hover { 0x262634 } else { 0x1d1d27 }, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
    if can_preview {
        c.hit(x0, 62.0, col, 46.0, Hit::Key(KeyCode::KeyP));
        c.text_right(x0 + col - 14.0, 70.0, 7.0, rgba(0x6f6f86, 1.0), "click to preview");
    }
    c.text(x0 + 14.0, 70.0, 8.0, rgba(accent(), 1.0), "MAP");
    let note_room = col - 38.0 - Canvas::text_width(8.0, "MAP");
    match &s.map {
        Some(m) => {
            c.text(x0 + 14.0, 86.0, 10.0, rgba(0xffffff, 1.0), &Canvas::fit(10.0, col - 28.0, &m.label()));
            if let Some(d) = &download {
                c.text_right(x0 + col - 14.0, 70.0, 8.0, rgba(0xFFD34D, 1.0), &Canvas::fit(8.0, note_room, d));
            } else if !have_map {
                c.text_right(x0 + col - 14.0, 70.0, 8.0, rgba(0xFF4B4B, 1.0), &Canvas::fit(8.0, note_room, "not in your Songs folder - D: get it from the host"));
            }
        }
        None => {
            let t = if s.is_host() { "Press M to pick a map" } else { "Waiting for the host to pick a map" };
            c.text(x0 + 14.0, 86.0, 10.0, rgba(0x8a8aa0, 1.0), t);
        }
    }

    let my_id = s.client.id;
    c.text(x0, 122.0, 8.0, rgba(accent(), 1.0), &format!("PLAYERS ({})", s.players.len()));
    for (i, p) in s.players.iter().enumerate().take(10) {
        let y = 136.0 + i as f32 * 26.0;
        let me = Some(p.id) == my_id;
        c.round_rect(x0, y, col, 22.0, 6.0, rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        // The name gets what the status (and the host's "make host" button) leave.
        let mut name_end = x0 + col - 110.0;
        if s.is_host() && !me && !s.in_match {
            // Hand the host role over.
            let label = "make host";
            let bw = Canvas::text_width(7.0, label) + 14.0;
            name_end -= bw;
            c.button(name_end, y + 4.0, bw, 14.0, label, 7.0, Hit::GiveHost(p.id), false);
        }
        let mut name = p.name.clone();
        if p.host {
            name.push_str("  (host)");
        }
        if me {
            name.push_str("  (you)");
        }
        if !p.mods.is_empty() {
            name.push_str(&format!("  +{}", p.mods.concat()));
        }
        c.text(x0 + 12.0, y + 7.0, 9.0, rgba(0xffffff, 1.0), &Canvas::fit(9.0, name_end - 8.0 - (x0 + 12.0), &name));
        let (st, color) = status_text(p, s.map.is_some());
        c.text_right(x0 + col - 12.0, y + 7.0, 9.0, rgba(color, 1.0), st);
    }

    // The latest lobby chat (F9 to write).
    if !s.chat.is_empty() && s.players.len() <= 8 {
        let top = 136.0 + s.players.len().min(10) as f32 * 26.0 + 10.0;
        let lines = ((428.0 - top) / 11.0).max(0.0) as usize;
        c.text(x0, top, 7.0, rgba(0x4fb4ff, 1.0), "LOBBY CHAT  (F9 to write)");
        for (k, (from, text)) in s.chat.iter().skip(s.chat.len().saturating_sub(lines.saturating_sub(1))).enumerate() {
            c.text(x0 + 4.0, top + 12.0 + k as f32 * 11.0, 6.5, rgba(0xdedeea, 1.0), &Canvas::fit(6.5, col - 8.0, &format!("{from}: {text}")));
        }
    }
    let ready = s.players.iter().filter(|p| p.ready && p.has_map).count();
    let me_ready = s.me().is_some_and(|p| p.ready);
    let me_spectator = s.me().is_some_and(|p| p.spectator);
    let ready_label = if me_ready { "Unready  (Space)" } else { "Ready  (Space)" };
    let spectate_label = if me_spectator { "Play instead  (S)" } else { "Spectate  (S)" };
    let start_label = format!("Start  (Enter, {ready} ready)");
    if s.in_match {
        c.text_centered(w / 2.0, 428.0, 8.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(8.0, w - 48.0, "A match is in progress; you'll join the next one."));
        let me_playing = s.me().is_some_and(|p| p.status != Status::Lobby);
        if !me_playing && have_map {
            c.button_row(440.0, &[("Spectate  (S)", Hit::Key(KeyCode::KeyS), true), ("Leave  (Esc)", Hit::Key(KeyCode::Escape), false)]);
        }
    } else if s.is_host() {
        c.button_row(440.0, &[
            ("Pick map  (M)", Hit::Key(KeyCode::KeyM), false),
            ("Preview / start point  (P)", Hit::Key(KeyCode::KeyP), false),
            (if me_spectator { spectate_label } else { ready_label }, Hit::Key(if me_spectator { KeyCode::KeyS } else { KeyCode::Space }), false),
            (&start_label, Hit::Key(KeyCode::Enter), true),
            (if s.owns_room() { "Close room  (Esc)" } else { "Leave  (Esc)" }, Hit::Key(KeyCode::Escape), false),
        ]);
        let hint = if me_spectator { "you'll watch the match   S: play instead   [ ] match rate   F1 mods   Tab songs" } else { "S: spectate instead   [ ] match rate   F1 mods (speed: for all)   Tab songs" };
        c.text_centered(w / 2.0, 466.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(7.0, w - 48.0, hint));
    } else {
        let missing = s.map.is_some() && !have_map;
        if missing {
            let busy = download.is_some();
            let label = if busy { "Downloading..." } else { "Get it from the host  (D)" };
            c.button_row(440.0, &[
                (label, Hit::Key(KeyCode::KeyD), true),
                (if busy { "..." } else { "From the mirrors  (M)" }, Hit::Key(KeyCode::KeyM), false),
                ("Leave  (Esc)", Hit::Key(KeyCode::Escape), false),
            ]);
        } else {
            let mut buttons = vec![("Preview  (P)", Hit::Key(KeyCode::KeyP), false)];
            if !me_spectator {
                // Everyone picks their own mods (the speed is the host's).
                buttons.push(("Mods  (F1)", Hit::Key(KeyCode::F1), false));
                buttons.push((ready_label, Hit::Key(KeyCode::Space), true));
            }
            buttons.push((spectate_label, Hit::Key(KeyCode::KeyS), me_spectator));
            buttons.push(("Leave  (Esc)", Hit::Key(KeyCode::Escape), false));
            c.button_row(440.0, &buttons);
            if me_spectator {
                c.text_centered(w / 2.0, 466.0, 7.0, rgba(0x4fb4ff, 1.0), "you'll watch the next match from its start");
            }
        }
    }
    if let Some(r) = &s.results {
        let finished: Vec<(&PlayerInfo, &Final)> = r.iter().filter_map(|p| p.result.as_ref().map(|f| (p, f))).collect();
        if let Some(top) = finished.iter().map(|(_, f)| f.score).max() {
            let winners: Vec<&str> = finished.iter().filter(|(_, f)| f.score == top).map(|(p, _)| p.name.as_str()).collect();
            let acc = finished.iter().find(|(_, f)| f.score == top).map(|(_, f)| f.accuracy * 100.0).unwrap_or(0.0);
            let text = if winners.len() > 1 {
                format!("Last match: tie between {} at {top:08}", winners.join(" & "))
            } else {
                format!("Last match winner: {} with {top:08} ({acc:.2}%)", winners[0])
            };
            let size = Canvas::fit_size(9.0, 7.0, w - 48.0, &text);
            c.text_centered(w / 2.0, 428.0 + (9.0 - size) / 2.0, size, rgba(0xFFD34D, 1.0), &Canvas::fit(size, w - 48.0, &text));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// In match

/// Waiting for everyone to load, then the countdown to the shared start time.
pub fn draw_waiting(c: &mut Canvas, s: &Session, start: Option<Instant>, loaded: bool, now: Instant) {
    let w = c.width;
    if let Some(m) = &s.map {
        c.text_centered(w / 2.0, 120.0, 11.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(11.0, w - 48.0, &m.label()));
    }
    let big = match start {
        Some(at) if at > now => format!("{}", (at - now).as_secs_f64().ceil() as u32),
        Some(_) => "GO".into(),
        None if loaded => "waiting for players...".into(),
        None => "loading...".into(),
    };
    let size = if start.is_some() { 48.0 } else { 16.0 };
    c.text_centered(w / 2.0, 180.0, size, rgba(0xffffff, 1.0), &big);
    for (i, p) in s.players.iter().filter(|p| p.status != Status::Lobby).enumerate().take(10) {
        let (st, color) = status_text(p, true);
        c.text_centered(w / 2.0, 270.0 + i as f32 * 16.0, 9.0, rgba(color, 1.0), &Canvas::fit(9.0, w - 48.0, &format!("{}  -  {st}", p.name)));
    }
}

/// Live standings drawn over the playfield.
pub fn draw_scoreboard(c: &mut Canvas, s: &Session, my_live: &Live) {
    let y0 = 282.0;
    let my_id = s.client.id;
    let mut rows: Vec<(String, Live, bool, bool)> = s
        .players
        .iter()
        .filter(|p| p.status != Status::Lobby)
        .map(|p| {
            let me = Some(p.id) == my_id;
            // Our own row uses local numbers so it never lags behind the playfield.
            let live = if me { my_live.clone() } else { p.live.clone() };
            (p.name.clone(), live, me, p.result.as_ref().is_some_and(|r| r.quit))
        })
        .collect();
    rows.sort_by(|a, b| b.1.score.cmp(&a.1.score));
    let x = 18.0;
    let h = 16.0;
    let lines: Vec<(String, u32)> = rows
        .iter()
        .enumerate()
        .take(8)
        .map(|(i, (name, live, me, quit))| {
            let color = if *me { 0xFFD34D } else if *quit { 0x6f6f86 } else { 0xc8c8d8 };
            let name: String = name.chars().take(12).collect();
            (format!("{}. {:<12} {:08} {:6.2}% {:>4}x", i + 1, name, live.score, live.accuracy * 100.0, live.combo), color)
        })
        .collect();
    // As wide as its longest line (wider when text comes out bigger).
    let bw = lines.iter().map(|(l, _)| Canvas::text_width(8.0, l) + 12.0).fold(300.0, f32::max);
    c.round_rect(x - 6.0, y0 - 6.0, bw, lines.len() as f32 * h + 10.0, 6.0, rgba(0x000000, 0.45), rgba(0, 0.0), 0.0);
    for (i, (line, color)) in lines.iter().enumerate() {
        c.text(x, y0 + i as f32 * h, 8.0, rgba(*color, 1.0), line);
    }
}

pub fn draw_results(c: &mut Canvas, s: &Session) {
    let w = c.width;
    let final_results = s.results.as_ref();
    let title = if final_results.is_some() { "MATCH RESULTS" } else { "waiting for other players to finish..." };
    c.text_centered(w / 2.0, 50.0, 14.0, rgba(0xffffff, 1.0), title);
    if let Some(m) = &s.map {
        c.text_centered(w / 2.0, 76.0, 9.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(9.0, w - 48.0, &m.label()));
    }
    let mut rows: Vec<PlayerInfo> = match final_results {
        Some(r) => r.clone(),
        None => s.players.iter().filter(|p| p.status != Status::Lobby).cloned().collect(),
    };
    let score = |p: &PlayerInfo| p.result.as_ref().map(|r| r.score).unwrap_or(p.live.score);
    rows.sort_by_key(|p| std::cmp::Reverse(score(p)));
    let col = (w - 48.0).min(700.0);
    let x0 = (w - col) / 2.0;
    c.text(x0 + 14.0, 102.0, 8.0, rgba(0x6f6f86, 1.0), "#  PLAYER            SCORE      ACC    COMBO  GREAT  OK  MISS    UR");
    for (i, p) in rows.iter().enumerate().take(10) {
        let y = 118.0 + i as f32 * 28.0;
        let me = Some(p.id) == s.client.id;
        let border = if i == 0 && final_results.is_some() { 0xFFD34D } else { 0x2a2a38 };
        c.round_rect(x0, y, col, 24.0, 6.0, rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0), rgba(border, 1.0), 1.0);
        let name: String = p.name.chars().take(14).collect();
        let line = match &p.result {
            Some(r) => format!(
                "{}  {:<14} {:08} {:6.2}% {:>5}x {:>6} {:>3} {:>5} {:>5.0}{}",
                i + 1,
                name,
                r.score,
                r.accuracy * 100.0,
                r.max_combo,
                r.great,
                r.ok,
                r.miss,
                r.ur,
                if r.quit { "  quit".to_string() } else if p.mods.is_empty() { String::new() } else { format!("  +{}", p.mods.concat()) }
            ),
            None => format!("{}  {:<14} {:08} {:6.2}%   playing ({:.0}%)", i + 1, name, p.live.score, p.live.accuracy * 100.0, p.live.progress * 100.0),
        };
        c.text(x0 + 14.0, y + 8.0, 8.0, rgba(if me { 0xFFD34D } else { 0xffffff }, 1.0), &Canvas::fit(8.0, col - 28.0, &line));
    }
    c.button_row(440.0, &[("Back to lobby  (Esc)", Hit::Key(KeyCode::Escape), final_results.is_some())]);
}
