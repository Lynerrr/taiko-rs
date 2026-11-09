//! What a click does. On each screen, every clickable area is clicked in its middle, each on a
//! fresh client, and where that leads is kept golden: the scene after the click, a message it
//! shows, whether it quits, and which texts appeared on or went from the screen.
//!
//! These pin down the click routing (`crate::hit::Hit` → `State::click` and friends) so that
//! reshaping it can't quietly send a button somewhere else. Clicks never start other programs
//! or reach the internet (`crate::offline`).
//!
//! Linux only, like `screens`: other systems name keys, folders and renderers differently.
#![cfg(target_os = "linux")]

use super::harness::{self, *};
use super::maps;
use super::screens::VOLATILE;
use super::snapshot::assert_snapshot;
use crate::*;
use std::fmt::Write as _;

/// How many appeared/gone texts a line lists before it just counts the rest.
const LIST: usize = 6;

/// Draws until fades and background work settle; the texts on screen, without positions.
fn settled_texts(s: &mut State, now: &mut Instant) -> Vec<String> {
    frame(s, *now);
    s.scene_since = *now - Duration::from_secs(5);
    settle(s, now);
    *now += Duration::from_millis(16);
    let c = frame(s, *now);
    let mut texts: Vec<String> = c
        .texts
        .iter()
        .map(|(_, t)| t.trim().to_string())
        .filter(|t| !t.is_empty() && !VOLATILE.iter().any(|v| t.contains(v)))
        .collect();
    texts.sort();
    texts
}

/// `a` minus `b`, as multisets of sorted texts.
fn minus(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() {
        if j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if j < b.len() && b[j] < a[i] {
            j += 1;
        } else {
            out.push(a[i].clone());
            i += 1;
        }
    }
    out.dedup();
    out
}

fn list(sign: char, texts: &[String]) -> String {
    let mut out = String::new();
    for t in texts.iter().take(LIST) {
        let _ = write!(out, " {sign}\"{t}\"");
    }
    if texts.len() > LIST {
        let _ = write!(out, " {sign}{} more", texts.len() - LIST);
    }
    out
}

/// A client on song select with `songs`, with `open` applied and drawn.
fn on_screen(songs: &Songs, open: fn(&mut State, &mut Instant), now: &mut Instant) -> State {
    let mut s = state_with(harness::config(), songs.entries());
    ready(&mut s, now);
    open(&mut s, now);
    *now += Duration::from_millis(16);
    frame(&mut s, *now);
    s
}

/// Presses and lets go of the left button at virtual `(x, y)`.
fn click_at(s: &mut State, now: &mut Instant, x: f32, y: f32) {
    let k = s.window_h / render::VIRTUAL_HEIGHT;
    s.mouse_moved(f64::from(x * k), f64::from(y * k));
    s.mouse_button(true, None);
    *now += Duration::from_millis(16);
    frame(s, *now);
    s.mouse_button(false, None);
}

/// Clicks every clickable area of the screen `open` leads to and describes what each did.
fn clicks(tag: &str, open: fn(&mut State, &mut Instant)) -> String {
    let songs = Songs::new(&format!("clicks-{tag}"), &[maps::basics()]);
    let mut now = Instant::now();
    let mut s = on_screen(&songs, open, &mut now);
    let before_texts = settled_texts(&mut s, &mut now);
    let before_scene = scene_name(&s);
    // Each area once, by what a click in its middle actually reaches (a box under another one
    // gives way to the one on top).
    let mut targets: Vec<(String, f32, f32)> = Vec::new();
    for h in s.hits.clone() {
        let [x, y, w, ht] = h.rect;
        let (cx, cy) = (x + w / 2.0, y + ht / 2.0);
        let Some(hit) = s.hit_at(cx, cy) else {
            continue;
        };
        let name = format!("{hit:?}");
        if !targets.iter().any(|(n, _, _)| *n == name) {
            targets.push((name, cx, cy));
        }
    }
    let mut out = format!("screen {tag}: {before_scene}, {} areas\n", targets.len());
    for (name, x, y) in targets {
        let mut now = Instant::now();
        let mut s = on_screen(&songs, open, &mut now);
        settled_texts(&mut s, &mut now);
        click_at(&mut s, &mut now, x, y);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let mut toast = s.toast.as_ref().map(|t| t.text.clone());
        let quit = s.quit || s.quit_asked;
        s.toast = None;
        let mut after = settled_texts(&mut s, &mut now);
        // Background work (a folder dialog that tests refuse) can answer with a toast only while
        // the screen settles, on a slow runner: it's the same toast, not a text the click showed.
        if toast.is_none()
            && let Some(late) = s.toast.as_ref().map(|t| t.text.trim().to_string())
        {
            after.retain(|t| *t != late);
            toast = Some(late);
        }
        let mut line = format!(
            "{name} @{},{} -> {}",
            x.round() as i32,
            y.round() as i32,
            scene_name(&s)
        );
        if quit {
            line.push_str(" [quit]");
        }
        if let Some(t) = toast {
            let _ = write!(line, " [toast \"{t}\"]");
        }
        line.push_str(&list('+', &minus(&after, &before_texts)));
        line.push_str(&list('-', &minus(&before_texts, &after)));
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn nothing(_: &mut State, _: &mut Instant) {}

#[test]
fn clicks_on_song_select() {
    assert_snapshot("clicks-song-select", &clicks("song-select", nothing));
}

#[test]
fn clicks_in_settings() {
    assert_snapshot(
        "clicks-settings",
        &clicks("settings", |s, _| s.open_settings()),
    );
}

#[test]
fn clicks_on_the_multiplayer_menu() {
    assert_snapshot(
        "clicks-multiplayer-menu",
        &clicks("multiplayer", |s, _| s.open_multiplayer()),
    );
}

#[test]
fn clicks_on_the_download_tab() {
    assert_snapshot(
        "clicks-download",
        &clicks("download", |s, _| s.open_download()),
    );
}

#[test]
fn clicks_on_the_editor_home() {
    assert_snapshot(
        "clicks-editor-home",
        &clicks("editor-home", |s, _| {
            s.scene = Scene::EditorHome(Box::new(editor::home::Home::new(&[], "tester")));
        }),
    );
}

#[test]
fn clicks_in_the_interface_editor() {
    assert_snapshot(
        "clicks-interface-editor",
        &clicks("hud-edit", |s, _| s.open_hud_editor()),
    );
}

#[test]
fn clicks_in_the_playfield_editor() {
    assert_snapshot(
        "clicks-playfield-editor",
        &clicks("layout-edit", |s, _| s.open_layout_editor()),
    );
}

#[test]
fn clicks_in_the_guide() {
    // Opened again by a player who has seen it: the first page doesn't hold them for a while.
    assert_snapshot(
        "clicks-guide",
        &clicks("guide", |s, _| {
            s.cfg.guide_seen = true;
            s.guide = Some(0);
        }),
    );
}

#[test]
fn clicks_on_the_main_menu() {
    assert_snapshot(
        "clicks-main-menu",
        &clicks("main-menu", |s, _| {
            s.cfg.main_menu = true;
            s.main_menu = Some(menu::Menu::new());
        }),
    );
}

#[test]
fn clicks_in_the_music_player() {
    assert_snapshot(
        "clicks-music-player",
        &clicks("player", |s, _| {
            s.cfg.main_menu = true;
            s.main_menu = Some(menu::Menu::new());
            s.music_control(player::Control::Open);
        }),
    );
}

#[test]
fn clicks_in_the_friends_overlay() {
    assert_snapshot(
        "clicks-friends-overlay",
        &clicks("social", |s, _| s.open_social()),
    );
}

#[test]
fn clicks_on_the_medals() {
    assert_snapshot(
        "clicks-medals",
        &clicks("medals", |s, _| s.open_medals(None)),
    );
}

#[test]
fn clicks_on_the_results() {
    assert_snapshot(
        "clicks-results",
        &clicks("results", |s, now| {
            let stats = game::Stats {
                great: 60,
                ok: 3,
                miss: 1,
                max_combo: 50,
                score: 912_345,
                ..Default::default()
            };
            let mut r = ui::Results::new(s.library[0].label(), stats, game::Windows::new(5.0));
            r.shown = *now - Duration::from_secs(10);
            r.pp = Some(123.4);
            s.scene = Scene::Results {
                results: Box::new(r),
                entry: 0,
                from: None,
            };
        }),
    );
}

#[test]
fn clicks_on_the_pause_menu() {
    assert_snapshot(
        "clicks-pause",
        &clicks("pause", |s, now| {
            press(s, KeyCode::Enter, *now);
            assert!(
                wait_for(s, now, |s| scene_name(s) == "Play"),
                "the map loads"
            );
            press(s, KeyCode::Escape, *now);
        }),
    );
}
