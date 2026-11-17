//! TJA charts in the library (`tja_maps`): read from their own folder only (never osu!'s Songs),
//! a set per file with a difficulty per course, played with their audio and their course's
//! windows, local scores kept by chart hash, copied in when dropped, and nothing about them
//! sent to the server.

use super::flows::{play_out, start_selected};
use super::harness::*;
use super::maps::{self, Player};
use crate::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Two courses (Oni written first, as charters often do), Japanese-style header.
const CHART: &str = "TITLE:Regression TJA\nSUBTITLE:--Tester\nBPM:120\nWAVE:song.ogg\nOFFSET:-1\n\nCOURSE:Oni\nLEVEL:8\nBALLOON:7\n#START\n1020,\n3040,\n7008,\n#END\n\nCOURSE:Easy\nLEVEL:2\n#START\n1000,\n1000,\n#END\n";

/// osu!'s Songs folder, taiko.rs's custom maps folder and its TJA folder, side by side.
struct Folders {
    root: PathBuf,
    songs: PathBuf,
    custom: PathBuf,
    tja: PathBuf,
}

impl Folders {
    fn new(tag: &str) -> Folders {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-tja-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let songs = root.join("osu!/Songs");
        let (custom, tja) = (root.join("taiko-rs/Custom"), root.join("taiko-rs/Tja"));
        for d in [&songs, &custom] {
            std::fs::create_dir_all(d).unwrap();
        }
        Folders {
            root,
            songs,
            custom,
            tja,
        }
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Writes `text` as `dir/set/name`.
fn put(dir: &Path, set: &str, name: &str, text: &str) -> PathBuf {
    let path = dir.join(set).join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

/// Song select scanning `f`'s folders, `cfg` its config.
fn client(f: &Folders, cfg: Config) -> (State, Instant) {
    let mut s = state_with(cfg, Vec::new());
    s.songs_dir = f.songs.clone();
    s.song_dirs = vec![f.songs.clone()];
    s.set_custom_dir(f.custom.clone());
    s.tja_dir = Some(f.tja.clone());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    (s, now)
}

fn refresh(s: &mut State, now: &mut Instant) {
    s.refresh_library(true);
    assert!(
        wait_for(s, now, |s| s.scan.is_none()),
        "the scan ends in time"
    );
}

fn tja_entries(s: &State) -> Vec<&Entry> {
    s.library.iter().filter(|e| e.is_tja()).collect()
}

#[test]
fn a_tja_file_is_one_set_with_a_difficulty_per_course() {
    let f = Folders::new("set");
    let (mut s, mut now) = client(&f, config());
    refresh(&mut s, &mut now);
    assert!(
        f.tja.is_dir(),
        "the TJA folder is made when the library is looked through"
    );
    let file = put(&f.tja, "Song", "song.tja", CHART);
    // A .tja in osu!'s Songs folder isn't read.
    put(&f.songs, "1 Other - Song", "other.tja", CHART);
    refresh(&mut s, &mut now);
    let tja = tja_entries(&s);
    let paths: Vec<&Path> = tja.iter().map(|e| e.path.as_path()).collect();
    assert_eq!(
        paths,
        [
            tja_maps::course_path(&file, taiko_logic::tja::CourseKind::Easy),
            tja_maps::course_path(&file, taiko_logic::tja::CourseKind::Oni)
        ],
        "easiest first, one entry per course"
    );
    let versions: Vec<&str> = tja.iter().map(|e| e.meta.version.as_str()).collect();
    assert_eq!(versions, ["Easy ★2", "Oni ★8"]);
    for e in &tja {
        assert_eq!(e.meta.title, "Regression TJA");
        assert_eq!(e.meta.artist, "Tester");
        assert_eq!(
            e.path.parent(),
            file.parent(),
            "the file's folder is the set's"
        );
        assert_eq!(preview::audio_path(e), f.tja.join("Song/song.ogg"));
    }
    let oni = s
        .library
        .iter()
        .position(|e| e.meta.version == "Oni ★8")
        .unwrap();
    assert_eq!(s.map_status(oni).map(|x| x.0), Some("TJA"));
    let map = beatmap::load(&s.library[oni].path).unwrap();
    assert_eq!(map.notes.len(), 5);
    assert_eq!(map.notes[4].required_hits, 7, "the balloon's pops");
    assert_eq!(
        map.meta.windows,
        beatmap::WindowSource::Arcade { narrow: true }
    );
    // The courses keep scores apart; a copy written otherwise (CRLF, a comment) shares them.
    let easy = s
        .library
        .iter()
        .position(|e| e.meta.version == "Easy ★2")
        .unwrap();
    assert_ne!(s.library[oni].hash(), s.library[easy].hash());
    let copy = put(
        &f.tja,
        "Copy",
        "copy.tja",
        &format!("// a copy\r\n{}", CHART.replace('\n', "\r\n")),
    );
    let copy_oni = Entry::new(
        tja_maps::course_path(&copy, taiko_logic::tja::CourseKind::Oni),
        beatmap::Meta::default(),
    );
    assert_eq!(copy_oni.hash(), s.library[oni].hash());
    // The map info (stars, length, BPM) comes from the chart.
    let info = mapinfo_of(&s.library[oni].path);
    assert!(
        info.stars > 0.0 && (info.bpm - 120.0).abs() < 1e-3,
        "{info:?}"
    );
}

/// The map info of library map `path`, worked out as the song list does it.
fn mapinfo_of(path: &Path) -> crate::mapinfo::Info {
    crate::mapinfo::analyze(path).expect("map info")
}

#[test]
fn a_tja_play_stays_on_this_computer() {
    let f = Folders::new("local");
    let mut cfg = config();
    // Logged in, with a server nobody answers at: a play of any other map would go to it.
    cfg.online_server = "127.0.0.1:9".into();
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let (mut s, mut now) = client(&f, cfg);
    put(&f.tja, "Song", "song.tja", CHART);
    refresh(&mut s, &mut now);
    let oni = s
        .library
        .iter()
        .position(|e| e.meta.version == "Oni ★8")
        .unwrap();
    let path = s.library[oni].path.clone();
    s.select.select_path(&s.library, &path);
    frame(&mut s, now);
    start_selected(&mut s, &mut now);
    let map = super::harness::game(&s).unwrap().map().clone();
    assert_eq!(
        super::harness::game(&s).unwrap().windows().great,
        25.5,
        "judged with Oni's arcade windows"
    );
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"));
    assert!(s.submits.is_empty(), "no score sent");
    assert!(s.score_outbox.is_empty(), "none kept to send later");
    let results = s
        .scene
        .screen_as::<screens::results::ResultsScreen>()
        .unwrap();
    assert!(results.results.vote_on.is_none(), "no vote to send");
    assert_eq!(results.results.no_pp, Some("TJA charts give no pp"));
    assert!(
        s.playing.is_none() && s.play_attempt.is_none(),
        "no play counted"
    );
    // Local scores and replays are kept by the course's chart hash.
    let song = taiko_logic::tja::parse(CHART).unwrap();
    assert_eq!(s.library[oni].hash(), song.courses[1].hash.short());
}

#[test]
fn a_dropped_tja_is_copied_into_the_tja_folder_and_listed() {
    let f = Folders::new("drop");
    let (mut s, mut now) = client(&f, config());
    let elsewhere = f.root.join("Downloads");
    let file = put(&elsewhere, "pack", "Song.tja", CHART);
    std::fs::write(elsewhere.join("pack/song.ogg"), b"ogg").unwrap();
    s.file_drag(crate::app::run_loop::FileDrag::Drop(vec![file.clone()]));
    assert!(wait_for(&mut s, &mut now, |s| s.scan.is_none()));
    assert!(f.tja.join("Song/Song.tja").is_file() && f.tja.join("Song/song.ogg").is_file());
    assert!(file.is_file(), "the dropped file stays where it was");
    assert_eq!(tja_entries(&s).len(), 2, "listed with its two courses");
}

#[test]
fn a_tja_chart_cant_be_opened_in_the_editor() {
    let f = Folders::new("blocked");
    let (mut s, mut now) = client(&f, config());
    put(&f.tja, "Song", "song.tja", CHART);
    refresh(&mut s, &mut now);
    let oni = s.library.iter().position(|e| e.is_tja()).unwrap();
    s.open_editor(oni);
    assert_eq!(scene_name(&s), "Select", "the editor doesn't open");
    assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("TJA")));
}

/// A server with TJA boards: it lists `TJA_BOARDS_FEATURE`, keeps every play it is sent and the
/// boards asked for, and answers them.
#[derive(Default)]
struct Seen {
    plays: Vec<online::ScoreSubmit>,
    boards: Vec<u64>,
}

fn tja_server() -> (String, std::sync::Arc<std::sync::Mutex<Seen>>) {
    use crate::online::{Envelope, Reply, Request};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Seen::default()));
    let shared = seen.clone();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let seen = shared.clone();
            std::thread::spawn(move || {
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut out = stream;
                let Ok(line) = online::read_line(&mut reader) else {
                    return;
                };
                let Ok(env) = serde_json::from_str::<Envelope>(&line) else {
                    return;
                };
                let account = Reply::Account {
                    name: "me".into(),
                    token: "t".into(),
                    nominator: false,
                    admin: false,
                    email: Some(true),
                    features: vec![
                        online::SUBMIT_KEY_FEATURE.into(),
                        online::TJA_BOARDS_FEATURE.into(),
                    ],
                    terms: 0,
                    server: None,
                };
                let reply = match env.request {
                    Request::Connect { .. } => {
                        let _ = online::write_line(&mut out, &account);
                        while online::read_line(&mut reader).is_ok() {}
                        return;
                    }
                    Request::WhoAmI { .. } => account,
                    Request::SubmitScore { play, .. } => {
                        seen.lock().unwrap().plays.push(*play);
                        Reply::Submitted {
                            relax: false,
                            map_rank: Some(1),
                            pp: 0.0,
                            rank: None,
                            medals: Vec::new(),
                        }
                    }
                    Request::MapScores { map, .. } => {
                        seen.lock().unwrap().boards.push(map);
                        Reply::Scores(Vec::new())
                    }
                    _ => Reply::Error("bad request".into()),
                };
                let _ = online::write_line(&mut out, &reply);
            });
        }
    });
    (addr, seen)
}

/// On a server with TJA boards a TJA play goes to the course's board, keyed by its chart hash
/// and sent with the chart the server checks it on (notes and timing, no file); song select
/// shows that board.
#[test]
fn a_tja_play_goes_to_a_server_with_tja_boards_with_its_chart() {
    let f = Folders::new("boards");
    let (server, seen) = tja_server();
    let mut cfg = config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let (mut s, mut now) = client(&f, cfg);
    put(&f.tja, "Song", "song.tja", CHART);
    refresh(&mut s, &mut now);
    let until = Instant::now() + Duration::from_secs(30);
    while !(s.social.connected && s.tja_boards) && Instant::now() < until {
        s.poll_social();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.tja_boards, "the server says it has TJA boards");
    let oni = s
        .library
        .iter()
        .position(|e| e.meta.version == "Oni ★8")
        .unwrap();
    let hash = s.library[oni].hash();
    let path = s.library[oni].path.clone();
    s.select.select_path(&s.library, &path);
    s.board_online = true;
    // Its board shown, on the panel's other page.
    s.mob.flip_board();
    let asked = |s: &mut State, now: &mut Instant| {
        wait_for(s, now, |_| seen.lock().unwrap().boards.contains(&hash))
    };
    assert!(
        asked(&mut s, &mut now),
        "song select asks for the course's board"
    );
    start_selected(&mut s, &mut now);
    let map = super::harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"));
    assert!(wait_for(&mut s, &mut now, |_| !seen
        .lock()
        .unwrap()
        .plays
        .is_empty()));
    let play = seen.lock().unwrap().plays.remove(0);
    assert_eq!(play.map.hash, hash);
    assert_eq!(play.map.set_id, None);
    assert_eq!((play.great, play.miss), (4, 0));
    assert!(!play.inputs.is_empty(), "with its replay");
    assert_eq!(
        (play.md5.as_deref(), play.stars),
        (None, None),
        "no file to tell of"
    );
    let chart = play.tja.expect("the chart goes with it");
    assert_eq!(chart.hash().short(), hash, "the chart is the board's");
    let sent = chart.beatmap().expect("a chart the server can play");
    assert_eq!(sent.notes.len(), map.notes.len());
    assert_eq!(
        sent.meta.windows,
        beatmap::WindowSource::Arcade { narrow: true }
    );
    // Still no pp, and nothing to vote on.
    let results = s
        .scene
        .screen_as::<screens::results::ResultsScreen>()
        .unwrap();
    assert_eq!(results.results.no_pp, Some("TJA charts give no pp"));
    assert!(results.results.vote_on.is_none());
}

/// The playfield of TJA chart `text`'s course (no inputs, `mods`), drawn at each of `times`
/// (map ms): where its small dons are along the lane, nearest first.
fn dons_drawn(text: &str, mods: &[&str], times: &[f64]) -> Vec<Vec<f32>> {
    use crate::game::{Look, Play, set_frame_look};
    use crate::hit::Canvas;
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    crate::render::set_frame_time(Some(Instant::now()));
    let map = taiko_logic::tja::parse(text).unwrap().courses.remove(0).map;
    let r = super::gameplay::replay(Vec::new(), mods, scores::new_version());
    let game = scores::replay_game(&map, &super::gameplay::silent_track(r.rate), &r);
    let mut play = Play::new(game);
    times
        .iter()
        .map(|&t| {
            play.game.set_time(t);
            let mut c = Canvas::default();
            c.clear(853.3);
            play.draw_playfield(&mut c, 1.0);
            // A small don's face: a circle of its radius in the don colour.
            let mut xs: Vec<f32> = c
                .shapes
                .iter()
                .filter(|s| {
                    (s.half[0] - 31.0).abs() < 0.5
                        && s.fill[3] > 0.5
                        && s.fill[0] > 0.8
                        && s.fill[1] < 0.4
                        && s.fill[2] < 0.4
                })
                .map(|s| s.center[0])
                .collect();
            xs.sort_by(f32::total_cmp);
            xs
        })
        .collect()
}

/// A beat apart at 120 BPM, then at 60 from 2000 ms.
const SLOWING: &str =
    "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0001,\n#BPMCHANGE 60\n1111,\n#END\n";

/// #HBSCROLL scrolls the whole lane by beats: once the BPM halves, every note on screen moves
/// half as fast, the one charted at 120 too, and they stay a beat apart. In normal mode a note
/// keeps the speed it was charted at.
#[test]
fn hbscroll_slows_every_note_on_screen_when_the_bpm_drops() {
    let hb = SLOWING.replace("#START\n", "#START\n#HBSCROLL\n");
    // 50 ms before 2000 (the change) and after it; the notes at 1500 (charted at 120), 2000
    // and 3000 (at 60).
    let at = [1600.0, 1650.0, 2100.0, 2150.0];
    let f = dons_drawn(&hb, &[], &at);
    assert!(f.iter().all(|xs| xs.len() >= 2), "{f:?}");
    // Before the change: the notes at 2000 and 3000 a beat apart, moving together.
    let (before, after) = (f[0][0] - f[1][0], f[2][0] - f[3][0]);
    assert!(
        (f[0][1] - f[0][0] - (f[1][1] - f[1][0])).abs() < 0.05,
        "{f:?}"
    );
    assert!(
        (f[2][1] - f[2][0] - (f[3][1] - f[3][0])).abs() < 0.05,
        "{f:?}"
    );
    let beat = f[0][1] - f[0][0];
    assert!(
        (f[2][1] - f[2][0] - beat).abs() < 0.05,
        "a beat is as long on screen at either BPM: {f:?}"
    );
    assert!(
        (before / after - 2.0).abs() < 0.01,
        "half as fast after the change: {before} then {after}"
    );
    // Normal mode: the note at 3000 moves as fast before the change as after it.
    let n = dons_drawn(SLOWING, &[], &at);
    let far = |xs: &Vec<f32>| *xs.last().unwrap();
    let (before, after) = (far(&n[0]) - far(&n[1]), far(&n[2]) - far(&n[3]));
    assert!(
        before > 0.0 && (before - after).abs() < 0.05,
        "{before} then {after}: {n:?}"
    );
    // Each note still reaches the drum on time.
    let on_time = dons_drawn(&hb, &[], &[3000.0]);
    assert!(
        on_time[0]
            .iter()
            .any(|&x| (x - taiko_logic::sim::HIT_X).abs() < 0.05),
        "{on_time:?}"
    );
}

/// #SUDDEN 1 0.5: a note isn't drawn until a second before its hit, then waits where it will be
/// half a second before it, and moves from there like any other.
#[test]
fn a_sudden_note_shows_up_late_and_waits_to_move() {
    let text =
        "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n#SUDDEN 1 0.5\n1,\n#END\n";
    let plain = text.replace("#SUDDEN 1 0.5\n", "");
    // The note is at 2000.
    let f = dons_drawn(text, &[], &[900.0, 1100.0, 1400.0, 1700.0, 2000.0]);
    assert!(f[0].is_empty(), "not drawn before a second ahead: {f:?}");
    assert_eq!(f[1].len(), 1, "{f:?}");
    assert_eq!(f[1], f[2], "still while it waits");
    let p = dons_drawn(&plain, &[], &[1500.0, 1700.0]);
    assert!(
        (f[1][0] - p[0][0]).abs() < 0.05,
        "it waits where it will be half a second before its hit: {f:?} {p:?}"
    );
    assert!(
        (f[3][0] - p[1][0]).abs() < 0.05,
        "then it moves like any note"
    );
    assert!((f[4][0] - taiko_logic::sim::HIT_X).abs() < 0.05);
    // With the scroll mods on top it still shows up late and lands on time.
    let m = dons_drawn(text, &["HD", "AC", "CS"], &[900.0, 2000.0]);
    assert!(m[0].is_empty(), "{m:?}");
}

/// The hit target's place and the small dons' along and across the lane at `t` (map ms) in
/// TJA chart `text`.
fn drawn_at(text: &str, t: f64) -> (Vec<(f32, f32)>, crate::hit::Canvas) {
    use crate::game::{Look, Play, set_frame_look};
    use crate::hit::Canvas;
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    crate::render::set_frame_time(Some(Instant::now()));
    let map = taiko_logic::tja::parse(text).unwrap().courses.remove(0).map;
    let r = super::gameplay::replay(Vec::new(), &[], scores::new_version());
    let game = scores::replay_game(&map, &super::gameplay::silent_track(r.rate), &r);
    let mut play = Play::new(game);
    play.game.set_time(t);
    // Auditing, so its texts are kept to look for.
    let mut c = Canvas {
        audit: true,
        ..Canvas::default()
    };
    c.clear(853.3);
    play.draw_playfield(&mut c, 1.0);
    let dons = c
        .shapes
        .iter()
        .filter(|s| {
            (s.half[0] - 31.0).abs() < 0.5
                && s.fill[3] > 0.5
                && s.fill[0] > 0.8
                && s.fill[1] < 0.4
                && s.fill[2] < 0.4
        })
        .map(|s| (s.center[0], s.center[1]))
        .collect();
    (dons, c)
}

/// #JPOSSCROLL slides the hit target, either way and up or down, and the notes still meet it on
/// time; OpenTaiko's 60 px are 40 virtual units here.
#[test]
fn jposscroll_moves_the_hit_target_and_notes_meet_it() {
    let still = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n1,\n#END\n";
    let moved = still.replace("0,\n1,", "#JPOSSCROLL 0 60+30i 1\n0,\n1,");
    let (home, _) = drawn_at(still, 2000.0);
    let (there, _) = drawn_at(&moved, 2000.0);
    assert_eq!(home.len(), 1, "{home:?}");
    assert_eq!(there.len(), 1, "{there:?}");
    let (dx, dy) = (there[0].0 - home[0].0, there[0].1 - home[0].1);
    assert!(
        (dx - 40.0).abs() < 0.05 && (dy - 20.0).abs() < 0.05,
        "on the moved target at its hit: {dx}, {dy}"
    );
    // Half a second before: half a second of lane from the moved target.
    let (before, _) = drawn_at(still, 1500.0);
    let (moved_before, _) = drawn_at(&moved, 1500.0);
    assert!((moved_before[0].0 - before[0].0 - 40.0).abs() < 0.05);
    // Leftwards too, which a .tko map's drum can't go.
    let left = still.replace("0,\n1,", "#JPOSSCROLL 0 -30 1\n0,\n1,");
    let (l, _) = drawn_at(&left, 2000.0);
    assert!((l[0].0 - home[0].0 + 20.0).abs() < 0.05, "{l:?}");
}

/// #LYRIC lines show under the lane from their time until the next one.
#[test]
fn lyrics_show_under_the_lane() {
    let text = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n#LYRIC Sing along\n1,\n#LYRIC\n1,\n#END\n";
    assert!(!shows(&drawn_at(text, 1500.0).1, "Sing along"));
    assert!(shows(&drawn_at(text, 2500.0).1, "Sing along"));
    assert!(!shows(&drawn_at(text, 4500.0).1, "Sing along"), "cleared");
}
