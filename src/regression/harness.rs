//! Drives the whole client without a window, GPU or sound card: a `State` with real chart files
//! on disk, frames drawn at made-up instants, keys pressed at chosen moments, and the screen read
//! back as the texts and clickable areas drawn.

use super::maps::Chart;
use crate::hit::Canvas;
use crate::*;
use std::fmt::Write as _;

/// The virtual width the screens are drawn at (16:9).
pub const WIDTH: f32 = 853.3;

/// A Songs folder holding `charts`, one set folder each, removed when dropped.
pub struct Songs {
    pub dir: PathBuf,
    pub paths: Vec<PathBuf>,
}

impl Songs {
    pub fn new(tag: &str, charts: &[Chart]) -> Songs {
        let dir = std::env::temp_dir().join(format!(
            "taiko-rs-regression-songs-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut paths = Vec::new();
        for c in charts {
            let set = dir.join(format!("Regression - {}", c.name));
            std::fs::create_dir_all(&set).unwrap();
            let path = set.join(format!("{}.osu", c.name));
            std::fs::write(&path, c.text()).unwrap();
            paths.push(path);
        }
        Songs { dir, paths }
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.paths
            .iter()
            .map(|p| {
                let meta = beatmap::read_meta_file(p).expect("a chart's metadata");
                Entry::new(p.clone(), meta)
            })
            .collect()
    }
}

impl Drop for Songs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Settings for tests: nothing online, no sound, no update checks, no animations.
pub fn config() -> Config {
    Config {
        check_updates: false,
        master_volume: 0.0,
        animations: false,
        online_server: "127.0.0.1:9".into(),
        send_crash_reports: false,
        discord_presence: false,
        // Straight to song select (the main menu has its own tests).
        main_menu: false,
        // A new one is drawn from the clock on every start.
        random_seed: 1,
        ..Config::default()
    }
}

/// Takes in the library, selects the first song in the list (the client starts on a random
/// one) and waits for the background work on it (stars, length, BPM, pp) to be drawn.
pub fn ready(s: &mut State, now: &mut Instant) {
    frame(s, *now);
    *now += Duration::from_millis(16);
    frame(s, *now);
    s.select.set_cursor(0);
    settle(s, now);
}

/// Draws frames (in real time, for background threads) until the screen stops changing.
pub fn settle(s: &mut State, now: &mut Instant) {
    let until = Instant::now() + Duration::from_secs(20);
    let mut last = String::new();
    let mut same = 0;
    while Instant::now() < until {
        let info_done = s.select.info.as_ref().is_none_or(|i| {
            let i = i.lock().unwrap();
            i.len() >= s.library.len() && i.iter().all(Option::is_some)
        });
        *now += Duration::from_millis(16);
        let shown = screen(&frame(s, *now), &[]);
        if shown == last && info_done {
            same += 1;
            if same >= 8 {
                return;
            }
        } else {
            same = 0;
            last = shown;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// A client with `library` scanned (taken in on the first frame) and settings `cfg`.
pub fn state_with(cfg: Config, library: Vec<Entry>) -> State {
    // Other tests set the process-wide drawing switches from their own settings meanwhile.
    render::pin_animations_for_this_thread(Some(cfg.animations));
    render::pin_accent_for_this_thread(Some(render::DEFAULT_ACCENT));
    multiverse::pin_note_colours_for_this_thread(Some(multiverse::Colours::CLASSIC));
    let (tx, rx) = mpsc::channel();
    tx.send(Scanned::Done(library)).unwrap();
    let mut s = State::new(
        cfg,
        None,
        None,
        PathBuf::from("/nonexistent"),
        Vec::new(),
        Some((rx, Arc::new(AtomicUsize::new(0)))),
        false,
        false,
    );
    s.window_w = 1920.0;
    s.window_h = 1080.0;
    s.mouse = (-1000.0, -1000.0);
    s
}

/// Draws one frame at `now` and hands back what was drawn.
pub fn frame(s: &mut State, now: Instant) -> Canvas {
    let mut c = Canvas {
        audit: true,
        ..Default::default()
    };
    s.build_frame(&mut c, now, WIDTH);
    c
}

/// Draws frames `step` apart from `*now` until `done` holds (true) or `limit` frames pass
/// (false). `*now` is left at the last frame.
pub fn run_until(
    s: &mut State,
    now: &mut Instant,
    step: Duration,
    limit: usize,
    mut done: impl FnMut(&State) -> bool,
) -> bool {
    for _ in 0..limit {
        if done(s) {
            return true;
        }
        *now += step;
        frame(s, *now);
    }
    done(s)
}

/// Waits (in real time, for background threads such as map loading) until `done` holds.
pub fn wait_for(s: &mut State, now: &mut Instant, mut done: impl FnMut(&State) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if done(s) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
        *now += Duration::from_millis(16);
        frame(s, *now);
    }
    done(s)
}

pub fn press(s: &mut State, code: KeyCode, at: Instant) {
    s.key(code, false, None, at, None);
}

/// Types `text` as key presses with their characters.
pub fn type_text(s: &mut State, text: &str, at: Instant) {
    for ch in text.chars() {
        let code = match ch.to_ascii_lowercase() {
            'a'..='z' => {
                let i = ch.to_ascii_lowercase() as usize - 'a' as usize;
                config_key_letter(i)
            }
            ' ' => KeyCode::Space,
            '0'..='9' => [
                KeyCode::Digit0,
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
            ][ch as usize - '0' as usize],
            _ => KeyCode::Minus,
        };
        let t = ch.to_string();
        s.key(code, false, Some(&t), at, None);
    }
}

fn config_key_letter(i: usize) -> KeyCode {
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    LETTERS[i]
}

/// The scene's name, for checks and golden files.
pub fn scene_name(s: &State) -> &'static str {
    match &s.scene {
        Scene::Select => "Select",
        Scene::Loading { .. } => "Loading",
        Scene::Play { .. } => "Play",
        Scene::Results { .. } => "Results",
        Scene::Settings { .. } => "Settings",
        Scene::Calibrate { .. } => "Calibrate",
        Scene::CalibrateDone { .. } => "CalibrateDone",
        Scene::LayoutEdit { .. } => "LayoutEdit",
        Scene::HudEdit { .. } => "HudEdit",
        Scene::Preview { .. } => "Preview",
        Scene::Download => "Download",
        Scene::MpMenu => "MpMenu",
        Scene::Lobby => "Lobby",
        Scene::MpWait { .. } => "MpWait",
        Scene::MpPlay { .. } => "MpPlay",
        Scene::Spectate(_) => "Spectate",
        Scene::Watch(_) => "Watch",
        Scene::VideoEdit(_) => "VideoEdit",
        Scene::Editor(_) => "Editor",
        Scene::EditorHome(_) => "EditorHome",
        Scene::MpResults => "MpResults",
    }
}

/// The game being played, if any.
pub fn game(s: &State) -> Option<&Game> {
    match &s.scene {
        Scene::Play { game, .. } | Scene::MpPlay { game, .. } => Some(game),
        _ => None,
    }
}

/// The instant map time `t` is reached in the game being played, from a frame drawn at `at`
/// (no audio: the game's clock runs on the frames' instants, at the play's speed).
pub fn instant_of(s: &State, at: Instant, t: f64) -> Instant {
    let g = game(s).expect("a game in progress");
    let rate = g.speed().rate_at(g.time).max(0.01);
    let ms = (t - g.time) / rate;
    if ms >= 0.0 {
        at + Duration::from_secs_f64(ms / 1000.0)
    } else {
        at - Duration::from_secs_f64(-ms / 1000.0)
    }
}

/// What a screen shows, as text: every string drawn (with where, in whole virtual units) and
/// every clickable area, in reading order. Texts that change by themselves (clocks, frame
/// rates, timers) are left out by `volatile`.
pub fn screen(c: &Canvas, volatile: &[&str]) -> String {
    let mut texts: Vec<(i32, i32, String)> = c
        .texts
        .iter()
        .filter(|(_, s)| !s.trim().is_empty())
        .filter(|(_, s)| !volatile.iter().any(|v| s.contains(v)))
        .map(|(range, s)| {
            let (mut x, mut y) = (f32::MAX, f32::MAX);
            for sh in &c.shapes[range.clone()] {
                x = x.min(sh.center[0] - sh.half[0]);
                y = y.min(sh.center[1] - sh.half[1]);
            }
            if x == f32::MAX {
                (x, y) = (-1.0, -1.0);
            }
            (y.round() as i32, x.round() as i32, s.clone())
        })
        .collect();
    texts.sort();
    texts.dedup();
    let mut out = String::new();
    for (y, x, s) in texts {
        let _ = writeln!(out, "text {x:>4},{y:<4} {s}");
    }
    let mut hits: Vec<String> = c
        .hits
        .iter()
        .map(|h| {
            let r = h.rect;
            format!(
                "hit  {:>4},{:<4} {:>4}x{:<4} {:?}",
                r[0].round() as i32,
                r[1].round() as i32,
                r[2].round() as i32,
                r[3].round() as i32,
                h.hit
            )
        })
        .collect();
    hits.dedup();
    for h in hits {
        out.push_str(&h);
        out.push('\n');
    }
    out
}

pub fn shows(c: &Canvas, needle: &str) -> bool {
    c.texts.iter().any(|(_, s)| s.contains(needle))
}
