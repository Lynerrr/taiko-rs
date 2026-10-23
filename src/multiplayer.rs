//! Multiplayer session state and screens: host/join menu, lobby, in-match scoreboard, results.

use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crate::game::{Game, Stats};
use crate::net::{self, Client, Final, Live, MapInfo, PlayerInfo, Server, Status};
use crate::render::{rgba, Canvas, Hit};
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

/// Opens a room and joins it ourselves, on a background thread so the UI never stalls.
pub fn start_host(port: u16, name: String) -> Connecting {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let server = net::host(port)?;
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

pub const ROW_NAME: usize = 0;
pub const ROW_PORT: usize = 1;
pub const ROW_HOST: usize = 2;
pub const ROW_ADDRESS: usize = 3;
pub const ROW_JOIN: usize = 4;
pub const MENU_ROWS: usize = 5;

#[derive(Default)]
pub struct Menu {
    pub cursor: usize,
}

pub fn draw_menu(c: &mut Canvas, menu: &Menu, name: &str, port: u16, address: &str, status: &str, busy: bool) {
    let w = c.width;
    crate::settings::tab_bar(c, 2, "");
    let col = (w - 48.0).min(520.0);
    let x0 = (w - col) / 2.0;
    let rows: [(&str, String, bool); MENU_ROWS] = [
        ("Your name", name.to_string(), true),
        ("Host on port", port.to_string(), true),
        ("HOST A GAME", String::new(), false),
        ("Join address", if address.is_empty() { "ip or ip:port".into() } else { address.to_string() }, true),
        ("JOIN GAME", String::new(), false),
    ];
    let mut y = 80.0;
    for (i, (label, value, editable)) in rows.iter().enumerate() {
        if i == ROW_ADDRESS {
            y += 18.0;
        }
        let sel = i == menu.cursor;
        let (bg, border) = if sel { (0x34344a, 0xEB452C) } else { (0x1d1d27, 0x2a2a38) };
        let bg = if !sel && c.hovered(x0, y, col, 30.0) { 0x262634 } else { bg };
        c.round_rect(x0, y, col, 30.0, 7.0, rgba(bg, 1.0), rgba(border, 1.0), if sel { 2.0 } else { 1.0 });
        c.hit(x0, y, col, 30.0, Hit::MpRow(i));
        if *editable {
            c.text(x0 + 14.0, y + 11.0, 9.0, rgba(0x9a9ab0, 1.0), label);
            let shown = if sel { format!("{value}_") } else { value.clone() };
            let col_v = if i == ROW_ADDRESS && address.is_empty() && !sel { 0x55556a } else { 0xffffff };
            c.text_right(x0 + col - 14.0, y + 11.0, 9.0, rgba(col_v, 1.0), &shown);
        } else {
            c.text_centered(x0 + col / 2.0, y + 10.0, 11.0, rgba(if sel { 0xffffff } else { 0xc8c8d8 }, 1.0), label);
        }
        y += 38.0;
    }
    let msg = if busy { "connecting..." } else { status };
    c.text_centered(w / 2.0, y + 16.0, 9.0, rgba(0xFFD34D, 1.0), msg);
    c.text_centered(w / 2.0, 430.0, 8.0, rgba(0x8a8aa0, 1.0), "Hosting: friends join with your public IP and this port (it must be reachable).");
    c.text_centered(w / 2.0, 446.0, 8.0, rgba(0x8a8aa0, 1.0), "Everyone needs the same map in their Songs folder.");
    c.text_centered(w / 2.0, 462.0, 8.0, rgba(0x55556a, 1.0), "Up/Down select   type to edit   Enter host/join   Tab or Esc back");
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
    c.text(x0 + 14.0, 70.0, 8.0, rgba(0xEB452C, 1.0), "MAP");
    match &s.map {
        Some(m) => {
            let label = m.label();
            c.text(x0 + 14.0, 86.0, 10.0, rgba(0xffffff, 1.0), &label);
            if let Some(d) = &download {
                c.text_right(x0 + col - 14.0, 70.0, 8.0, rgba(0xFFD34D, 1.0), d);
            } else if !have_map {
                c.text_right(x0 + col - 14.0, 70.0, 8.0, rgba(0xFF4B4B, 1.0), "not in your Songs folder - D: get it from the host");
            }
        }
        None => {
            let t = if s.is_host() { "Press M to pick a map" } else { "Waiting for the host to pick a map" };
            c.text(x0 + 14.0, 86.0, 10.0, rgba(0x8a8aa0, 1.0), t);
        }
    }

    let my_id = s.client.id;
    c.text(x0, 122.0, 8.0, rgba(0xEB452C, 1.0), &format!("PLAYERS ({})", s.players.len()));
    for (i, p) in s.players.iter().enumerate().take(10) {
        let y = 136.0 + i as f32 * 26.0;
        let me = Some(p.id) == my_id;
        c.round_rect(x0, y, col, 22.0, 6.0, rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        if s.is_host() && !me && !s.in_match {
            // Hand the host role over.
            let label = "make host";
            let bw = Canvas::text_width(7.0, label) + 14.0;
            c.button(x0 + col - 110.0 - bw, y + 4.0, bw, 14.0, label, 7.0, Hit::GiveHost(p.id), false);
        }
        let mut name = p.name.clone();
        if p.host {
            name.push_str("  (host)");
        }
        if me {
            name.push_str("  (you)");
        }
        c.text(x0 + 12.0, y + 7.0, 9.0, rgba(0xffffff, 1.0), &name);
        let (st, color) = status_text(p, s.map.is_some());
        c.text_right(x0 + col - 12.0, y + 7.0, 9.0, rgba(color, 1.0), st);
    }

    let ready = s.players.iter().filter(|p| p.ready && p.has_map).count();
    let me_ready = s.me().is_some_and(|p| p.ready);
    let me_spectator = s.me().is_some_and(|p| p.spectator);
    let ready_label = if me_ready { "Unready  (Space)" } else { "Ready  (Space)" };
    let spectate_label = if me_spectator { "Play instead  (S)" } else { "Spectate  (S)" };
    let start_label = format!("Start  (Enter, {ready} ready)");
    if s.in_match {
        c.text_centered(w / 2.0, 428.0, 8.0, rgba(0x9a9ab0, 1.0), "A match is in progress; you'll join the next one.");
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
        let hint = if me_spectator { "you'll watch the match   S: play instead   [ ] match rate   Tab songs" } else { "S: spectate instead   [ ] match rate   Tab songs" };
        c.text_centered(w / 2.0, 466.0, 7.0, rgba(0x6f6f86, 1.0), hint);
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
            c.text_centered(w / 2.0, 428.0, 9.0, rgba(0xFFD34D, 1.0), &text);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// In match

/// Waiting for everyone to load, then the countdown to the shared start time.
pub fn draw_waiting(c: &mut Canvas, s: &Session, start: Option<Instant>, loaded: bool, now: Instant) {
    let w = c.width;
    if let Some(m) = &s.map {
        c.text_centered(w / 2.0, 120.0, 11.0, rgba(0x9a9ab0, 1.0), &m.label());
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
        c.text_centered(w / 2.0, 270.0 + i as f32 * 16.0, 9.0, rgba(color, 1.0), &format!("{}  -  {st}", p.name));
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
    c.round_rect(x - 6.0, y0 - 6.0, 300.0, rows.len().min(8) as f32 * h + 10.0, 6.0, rgba(0x000000, 0.45), rgba(0, 0.0), 0.0);
    for (i, (name, live, me, quit)) in rows.iter().enumerate().take(8) {
        let y = y0 + i as f32 * h;
        let color = if *me { 0xFFD34D } else if *quit { 0x6f6f86 } else { 0xc8c8d8 };
        let name: String = name.chars().take(12).collect();
        let line = format!("{}. {:<12} {:08} {:6.2}% {:>4}x", i + 1, name, live.score, live.accuracy * 100.0, live.combo);
        c.text(x, y, 8.0, rgba(color, 1.0), &line);
    }
}

pub fn draw_results(c: &mut Canvas, s: &Session) {
    let w = c.width;
    let final_results = s.results.as_ref();
    let title = if final_results.is_some() { "MATCH RESULTS" } else { "waiting for other players to finish..." };
    c.text_centered(w / 2.0, 50.0, 14.0, rgba(0xffffff, 1.0), title);
    if let Some(m) = &s.map {
        c.text_centered(w / 2.0, 76.0, 9.0, rgba(0x9a9ab0, 1.0), &m.label());
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
                if r.quit { "  quit" } else { "" }
            ),
            None => format!("{}  {:<14} {:08} {:6.2}%   playing ({:.0}%)", i + 1, name, p.live.score, p.live.accuracy * 100.0, p.live.progress * 100.0),
        };
        c.text(x0 + 14.0, y + 8.0, 8.0, rgba(if me { 0xFFD34D } else { 0xffffff }, 1.0), &line);
    }
    c.button_row(440.0, &[("Back to lobby  (Esc)", Hit::Key(KeyCode::Escape), final_results.is_some())]);
}
