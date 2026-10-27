//! Mapguessr: a multiplayer lobby where each round shows a map everyone has, only its notes
//! scrolling by (no song, no title), and players pick it from their own song list. The room
//! (`net`) picks the maps and keeps the points; this is our side: sharing our map list, showing
//! the round's pattern, and the lobby screen.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audio::Track;
use crate::beatmap::{self, Beatmap};
use crate::game::Game;
use crate::library::Entry;
use crate::multiplayer::Session;
use crate::net::{self, ClientMsg, PlayerInfo};
use crate::render::{accent, rgba, Canvas, Hit};
use winit::keyboard::KeyCode;

/// The pattern moves on in steps this long, so autoplay hits every note even after a slow
/// frame; after a longer gap than `CATCH_UP` (a minimised window) it jumps ahead instead.
const STEP: Duration = Duration::from_millis(8);
const CATCH_UP: Duration = Duration::from_secs(1);

/// Rounds a game has unless the host picks otherwise, and the most.
pub const DEFAULT_ROUNDS: u32 = 10;
pub const MAX_ROUNDS: u32 = 50;

/// Which song a map is: guessing any difficulty of it counts.
pub fn song_key(artist: &str, title: &str) -> u64 {
    let norm = |s: &str| s.trim().to_lowercase();
    net::map_hash(format!("{}\0{}", norm(artist), norm(title)).as_bytes())
}

pub fn entry_song(e: &Entry) -> u64 {
    song_key(&e.meta.artist, &e.meta.title)
}

/// Our map list as (map hash, song key, file), worked out on a thread (it reads every map).
fn list_library(library: &[Entry]) -> Receiver<Vec<(u64, u64, PathBuf)>> {
    let maps: Vec<(PathBuf, u64)> = library.iter().map(|e| (e.path.clone(), entry_song(e))).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let list = maps.into_iter().filter_map(|(path, song)| Some((net::map_hash(&std::fs::read(&path).ok()?), song, path))).collect();
        let _ = tx.send(list);
    });
    rx
}

/// The round being played: its map's notes scroll by from `from` on, over and over.
pub struct Round {
    pub round: u32,
    pub rounds: u32,
    pub hash: u64,
    pub ends: Instant,
    from: f64,
    load: Option<Receiver<Result<Beatmap, String>>>,
    map: Option<Beatmap>,
    pub game: Option<Game>,
    /// Where the pattern starts (map ms).
    start: f64,
    /// When the pattern last moved on.
    ticked: Option<Instant>,
}

impl Round {
    fn new_game(&mut self) {
        let Some(map) = &self.map else { return };
        // No samples and no engine: the game runs on its own clock, silently.
        let mut g = Game::new(map.clone(), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
        g.mute_hits = true;
        g.skip_to(self.start - 50.0);
        g.start_from(self.start, None, 0.0);
        self.game = Some(g);
        self.ticked = None;
    }

    /// Moves the pattern on to `now` (from its start again once it's past the last note).
    fn advance(&mut self, now: Instant) {
        if self.game.as_ref().is_some_and(|g| g.time > g.end_time() + 1500.0) {
            self.new_game();
        }
        let Some(g) = &mut self.game else { return };
        let mut at = self.ticked.filter(|&t| t < now).unwrap_or(now);
        if now - at > CATCH_UP {
            g.skip_to(g.time + (now - at).as_secs_f64() * 1000.0 - 100.0);
            at = now - STEP;
        }
        loop {
            at = (at + STEP).min(now);
            g.update(at, None, 0.0);
            g.autoplay(None);
            if at >= now {
                break;
            }
        }
        self.ticked = Some(now);
    }
}

/// What was shown when a round ended.
pub struct Reveal {
    pub label: String,
    pub got: Vec<(u32, u32)>,
    pub last: bool,
}

#[derive(Default)]
pub struct Guessr {
    list_rx: Option<Receiver<Vec<(u64, u64, PathBuf)>>>,
    /// Our maps by hash, once listed (and sent to the room).
    paths: Option<HashMap<u64, PathBuf>>,
    pub round: Option<Round>,
    pub reveal: Option<Reveal>,
    /// Guesses left this round, and what we last guessed.
    pub tries: u8,
    pub last_guess: Option<String>,
    /// How many rounds the host will start the next game with.
    pub rounds: u32,
}

impl Guessr {
    /// Starts listing our maps, to send to the room.
    pub fn new(library: &[Entry]) -> Guessr {
        Guessr { list_rx: Some(list_library(library)), rounds: DEFAULT_ROUNDS, ..Default::default() }
    }

    pub fn listing(&self) -> bool {
        self.paths.is_none()
    }

    /// A round is on and we can still guess in it.
    pub fn can_guess(&self, s: &Session) -> bool {
        self.round.is_some() && self.reveal.is_none() && s.me().is_some_and(|p| p.status == net::Status::Playing && !p.guessed && p.tries > 0)
    }

    /// Sends our map list once it's ready, and keeps the round's pattern moving.
    pub fn tick(&mut self, s: &Session, now: Instant) {
        if let Some(list) = self.list_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.list_rx = None;
            let maps: Vec<(u64, u64)> = list.iter().map(|(h, song, _)| (*h, *song)).collect();
            let chunks: Vec<&[(u64, u64)]> = if maps.is_empty() { vec![&[]] } else { maps.chunks(net::LIBRARY_CHUNK).collect() };
            for (i, part) in chunks.iter().enumerate() {
                s.client.send(ClientMsg::Library { maps: part.to_vec(), done: i + 1 == chunks.len() });
            }
            self.paths = Some(list.into_iter().map(|(h, _, path)| (h, path)).collect());
            // A round that started while we were still listing.
            if let Some(r) = &mut self.round {
                if r.load.is_none() && r.map.is_none() {
                    r.load = self.paths.as_ref().and_then(|p| p.get(&r.hash)).map(|p| load(p.clone()));
                }
            }
        }
        let Some(r) = &mut self.round else { return };
        if let Some(result) = r.load.as_ref().and_then(|rx| rx.try_recv().ok()) {
            r.load = None;
            if let Ok(map) = result {
                // From `from` of the way between the first note and the last.
                let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
                let last = map.notes.iter().map(|n| n.end_time).fold(first, f64::max);
                r.start = first + (last - first) * r.from;
                r.map = Some(map);
                r.new_game();
            }
        }
        r.advance(now);
    }

    pub fn start_round(&mut self, round: u32, rounds: u32, hash: u64, from: f64, seconds: u32) {
        let load = self.paths.as_ref().and_then(|p| p.get(&hash)).map(|p| load(p.clone()));
        self.round = Some(Round { round, rounds, hash, ends: Instant::now() + Duration::from_secs(seconds as u64), from, load, map: None, game: None, start: 0.0, ticked: None });
        self.reveal = None;
        self.tries = net::GUESS_TRIES;
        self.last_guess = None;
    }

    /// The round is over: it was map `hash`.
    pub fn reveal(&mut self, library: &[Entry], hash: u64, got: Vec<(u32, u32)>, last: bool) {
        let path = self.paths.as_ref().and_then(|p| p.get(&hash));
        let label = path.and_then(|p| library.iter().find(|e| &e.path == p)).map(Entry::label).unwrap_or_else(|| "a map you have".into());
        self.reveal = Some(Reveal { label, got, last });
    }

    /// Back in the lobby after a game.
    pub fn game_over(&mut self) {
        self.round = None;
        self.reveal = None;
    }

    /// Our list sent, and (with `map`) a round showing it.
    #[cfg(test)]
    pub fn test_round(&mut self, map: Option<Beatmap>) {
        self.list_rx = None;
        self.paths = Some(HashMap::new());
        let Some(map) = map else { return };
        self.start_round(48, 50, 7, 0.3, 45);
        if let Some(r) = &mut self.round {
            r.start = map.notes[map.notes.len() / 3].time;
            r.map = Some(map);
            r.new_game();
        }
    }
}

fn load(path: PathBuf) -> Receiver<Result<Beatmap, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(beatmap::load(&path));
    });
    rx
}

/// The lane of `game`, fitted into the box at (x, y, w, h).
pub fn draw_pattern(c: &mut Canvas, game: &mut Game, [x, y, w, h]: [f32; 4], scroll_speed: f64) {
    let (band_top, band_h) = crate::game::lane_band();
    let k = (w / 640.0).min(h / band_h);
    let full_w = c.width;
    let start = c.shapes.len();
    c.width = w / k;
    game.draw_playfield(c, scroll_speed);
    let oy = y + (h - band_h * k) / 2.0 - band_top * k;
    c.transform_since(start, [0.0, 0.0], k, x, oy);
    c.width = full_w;
}

fn clock(r: &Round, now: Instant) -> String {
    let left = r.ends.saturating_duration_since(now).as_secs();
    format!("{}:{:02}", left / 60, left % 60)
}

/// A player's state in the list: guessing, got it, out of guesses...
fn player_state(p: &PlayerInfo, in_game: bool) -> (String, u32) {
    if p.spectator {
        return ("spectator".into(), 0x4fb4ff);
    }
    if in_game && p.status == net::Status::Playing {
        return match (p.guessed, p.tries) {
            (true, _) => ("got it".into(), 0x86E05A),
            (false, 0) => ("out of guesses".into(), 0xFF4B4B),
            (false, n) => (format!("guessing ({n} left)"), 0xFFD34D),
        };
    }
    match p.maps {
        Some(n) => (format!("{n} maps"), 0x8a8aa0),
        None => ("sharing maps...".into(), 0x8a8aa0),
    }
}

/// The mapguessr lobby: the round's pattern, the clock, the answer, everyone's points.
pub fn draw_lobby(c: &mut Canvas, s: &Session, g: &mut Guessr, scroll_speed: f64, now: Instant) {
    let w = c.width;
    let right = format!("{}   ping {:.0} ms", s.label, s.client.rtt_ms);
    crate::settings::tab_bar(c, 2, &right);
    let col = (w - 48.0).min(640.0);
    let x0 = (w - col) / 2.0;

    // The headline: the round and its clock, the answer, or what the game is.
    c.text(x0, 60.0, 8.0, rgba(accent(), 1.0), "MAPGUESSR");
    let (line, colour) = match (&g.round, &g.reveal) {
        (Some(r), None) => (format!("Round {} of {}   {}", r.round, r.rounds, clock(r, now)), 0xffffff),
        (_, Some(rv)) => (format!("It was: {}", rv.label), 0xFFD34D),
        (None, None) if g.listing() => ("Sharing your map list...".into(), 0x9a9ab0),
        (None, None) => match s.common {
            Some(0) => ("There's no map that everyone has.".into(), 0xFF4B4B),
            Some(n) => (format!("{n} maps everyone has. Guess each one from its notes alone."), 0xc8c8d8),
            None => ("Waiting for everyone's map list...".into(), 0x9a9ab0),
        },
    };
    c.text(x0, 74.0, 11.0, rgba(colour, 1.0), &Canvas::fit(11.0, col, &line));

    // The pattern.
    let (_, band_h) = crate::game::lane_band();
    let bh = (band_h * col / 640.0).min(130.0);
    let rect = [x0, 94.0, col, bh];
    c.round_rect(x0, 94.0, col, bh, 8.0, rgba(0x0c0c12, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
    match g.round.as_mut().map(|r| (r.game.as_mut(), r.load.is_some())) {
        Some((Some(game), _)) if g.reveal.is_none() => draw_pattern(c, game, rect, scroll_speed),
        Some((None, true)) => c.text_centered(w / 2.0, 94.0 + bh / 2.0 - 5.0, 9.0, rgba(0x8a8aa0, 1.0), "loading..."),
        Some((None, false)) if g.reveal.is_none() => {
            c.text_centered(w / 2.0, 94.0 + bh / 2.0 - 5.0, 9.0, rgba(0xFF4B4B, 1.0), "couldn't open this map")
        }
        _ => {
            let t = if g.reveal.is_some() { "next round in a moment..." } else { "the notes show here, without the song" };
            c.text_centered(w / 2.0, 94.0 + bh / 2.0 - 5.0, 9.0, rgba(0x6f6f86, 1.0), t);
        }
    }

    // Everyone, by points.
    let in_game = g.round.is_some();
    let mut players: Vec<&PlayerInfo> = s.players.iter().collect();
    players.sort_by(|a, b| b.points.cmp(&a.points).then_with(|| a.name.cmp(&b.name)));
    let top = 94.0 + bh + 12.0;
    c.text(x0, top, 8.0, rgba(accent(), 1.0), &format!("PLAYERS ({})", players.len()));
    let got: HashMap<u32, u32> = g.reveal.as_ref().map(|r| r.got.iter().copied().collect()).unwrap_or_default();
    let rows = (((428.0 - top - 14.0) / 24.0) as usize).max(1);
    for (i, p) in players.iter().enumerate().take(rows) {
        let y = top + 14.0 + i as f32 * 24.0;
        let me = Some(p.id) == s.client.id;
        c.round_rect(x0, y, col, 20.0, 6.0, rgba(if me { 0x2b2b3d } else { 0x1d1d27 }, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        let mut name = p.name.clone();
        if p.host {
            name.push_str("  (host)");
        }
        if me {
            name.push_str("  (you)");
        }
        let points = match got.get(&p.id) {
            Some(n) => format!("{}  (+{n})", p.points),
            None => p.points.to_string(),
        };
        let (state, state_colour) = player_state(p, in_game && g.reveal.is_none());
        c.text(x0 + 12.0, y + 6.0, 9.0, rgba(0xffffff, 1.0), &Canvas::fit(9.0, col * 0.45, &name));
        c.text_right(x0 + col * 0.72, y + 6.0, 9.0, rgba(0xFFD34D, 1.0), &points);
        c.text_right(x0 + col - 12.0, y + 6.0, 8.0, rgba(state_colour, 1.0), &state);
    }

    // The latest chat (mapguessr's notices too), where there's room.
    let chat_top = top + 14.0 + players.len().min(rows) as f32 * 24.0 + 6.0;
    let lines = ((424.0 - chat_top) / 11.0).max(0.0) as usize;
    if lines >= 2 && !s.chat.is_empty() {
        for (k, (from, text)) in s.chat.iter().skip(s.chat.len().saturating_sub(lines)).enumerate() {
            let colour = if from == net::GUESSR_NAME { 0xFFD34D } else { 0xdedeea };
            c.text(x0 + 4.0, chat_top + k as f32 * 11.0, 6.5, rgba(colour, 1.0), &Canvas::fit(6.5, col - 8.0, &format!("{from}: {text}")));
        }
    }

    // What you can do.
    let leave = if s.owns_room() { "Close room  (Esc)" } else { "Leave  (Esc)" };
    let me_spectator = s.me().is_some_and(|p| p.spectator);
    if in_game {
        let note = match &g.last_guess {
            _ if g.reveal.as_ref().is_some_and(|r| r.last) => "That was the last round.".to_string(),
            _ if g.reveal.is_some() => String::new(),
            _ if s.me().is_some_and(|p| p.guessed) => "You got it! Waiting for the others...".into(),
            Some(last) if g.tries == 0 => format!("Out of guesses (last: {last})"),
            Some(last) => format!("Not {last}. {} left", plural(g.tries as u32, "guess", "guesses")),
            None if s.me().is_some_and(|p| p.status != net::Status::Playing) => "You'll play from the next game.".into(),
            None => format!("Which map is it? {}", plural(g.tries as u32, "guess", "guesses")),
        };
        c.text_centered(w / 2.0, 428.0, 8.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(8.0, w - 48.0, &note));
        let mut buttons = Vec::new();
        if g.can_guess(s) {
            buttons.push(("Guess: pick it from your songs  (G)", Hit::Key(KeyCode::KeyG), true));
        }
        buttons.push((leave, Hit::Key(KeyCode::Escape), false));
        c.button_row(442.0, &buttons);
    } else {
        // The last game's winner, while the points still show.
        if let Some(best) = s.players.iter().map(|p| p.points).max().filter(|&n| n > 0) {
            let winners: Vec<&str> = s.players.iter().filter(|p| p.points == best).map(|p| p.name.as_str()).collect();
            let text = format!("Last game: {} with {best} points", winners.join(" & "));
            c.text_centered(w / 2.0, 428.0, 9.0, rgba(0xFFD34D, 1.0), &Canvas::fit(9.0, w - 48.0, &text));
        }
        let spectate = if me_spectator { "Play instead  (S)" } else { "Spectate  (S)" };
        if s.is_host() {
            let rounds = format!("{}  ([ ])", plural(g.rounds, "round", "rounds"));
            c.button_row(442.0, &[
                (&rounds, Hit::Key(KeyCode::BracketRight), false),
                ("Start  (Enter)", Hit::Key(KeyCode::Enter), true),
                (spectate, Hit::Key(KeyCode::KeyS), false),
                (leave, Hit::Key(KeyCode::Escape), false),
            ]);
        } else {
            c.button_row(442.0, &[(spectate, Hit::Key(KeyCode::KeyS), false), (leave, Hit::Key(KeyCode::Escape), false)]);
            c.text_centered(w / 2.0, 468.0, 7.0, rgba(0x6f6f86, 1.0), "the host starts the game");
        }
    }
}

/// The strip over song select while picking a guess: the pattern to match, and the clock.
pub fn draw_picking(c: &mut Canvas, g: &mut Guessr, scroll_speed: f64, now: Instant) {
    let w = c.width;
    let (x, y, h) = (24.0, 380.0, 92.0);
    let bw = w - 48.0;
    c.round_rect(x, y, bw, h, 8.0, rgba(0x15151d, 1.0), rgba(accent(), 1.0), 1.5);
    let pattern_w = (bw * 0.62).min(560.0);
    let rect = [x + 8.0, y + 8.0, pattern_w, h - 16.0];
    c.round_rect(rect[0], rect[1], rect[2], rect[3], 6.0, rgba(0x0c0c12, 1.0), rgba(0x2a2a38, 1.0), 1.0);
    let Some(r) = &mut g.round else { return };
    if let Some(game) = &mut r.game {
        draw_pattern(c, game, rect, scroll_speed);
    }
    let tx = x + 20.0 + pattern_w;
    let room = bw - pattern_w - 28.0;
    c.text(tx, y + 12.0, 9.0, rgba(accent(), 1.0), &Canvas::fit(9.0, room, &format!("ROUND {} OF {}", r.round, r.rounds)));
    c.text(tx, y + 28.0, 16.0, rgba(0xffffff, 1.0), &clock(r, now));
    let tries = plural(g.tries as u32, "guess", "guesses") + " left";
    c.text(tx, y + 52.0, 8.0, rgba(0xFFD34D, 1.0), &Canvas::fit(8.0, room, &tries));
    c.text(tx, y + 68.0, 7.0, rgba(0x8a8aa0, 1.0), &Canvas::fit(7.0, room, "Enter: it's this one   Esc: back"));
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_difficulty_of_a_song_is_the_same_song() {
        assert_eq!(song_key("Camellia", "Ghost"), song_key(" camellia ", "GHOST"));
        assert_ne!(song_key("Camellia", "Ghost"), song_key("Camellia", "Ghost (Extended)"));
        // The artist and title don't run into each other.
        assert_ne!(song_key("ab", "c"), song_key("a", "bc"));
    }

    #[test]
    fn the_pattern_never_shows_misses_even_after_slow_frames() {
        let mut g = Guessr::default();
        g.test_round(Some(beatmap::calibration_map(100.0, 0, 2000)));
        let r = g.round.as_mut().unwrap();
        let t0 = Instant::now();
        // Frames 0.3 s apart (a notes' miss window is far shorter), then a 5 s gap.
        for k in 1..=20 {
            r.advance(t0 + Duration::from_millis(300 * k));
        }
        r.advance(t0 + Duration::from_millis(11_000));
        r.advance(t0 + Duration::from_millis(11_300));
        let game = r.game.as_ref().unwrap();
        assert!(game.time > r.start + 5000.0, "it moved on: {} from {}", game.time, r.start);
        assert_eq!(game.stats.miss, 0);
        assert!(game.stats.great > 30, "{}", game.stats.great);
    }
}
