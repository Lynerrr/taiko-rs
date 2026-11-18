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
    assert_eq!(versions, ["Easy", "Oni"], "the level isn't in the name");
    let levels: Vec<Option<f32>> = tja.iter().map(|e| e.meta.level).collect();
    assert_eq!(levels, [Some(2.0), Some(8.0)]);
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
        .position(|e| e.meta.version == "Oni")
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
        .position(|e| e.meta.version == "Easy")
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
        .position(|e| e.meta.version == "Oni")
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
    /// What an admin asked for on the social connection.
    admin: Vec<online::AdminOp>,
}

fn tja_server() -> (String, std::sync::Arc<std::sync::Mutex<Seen>>) {
    server_for(false)
}

/// `tja_server`, logging `admin` in as an admin or not.
fn server_for(admin: bool) -> (String, std::sync::Arc<std::sync::Mutex<Seen>>) {
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
                    admin,
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
                        while let Ok(line) = online::read_line(&mut reader) {
                            if let Ok(online::Up::Admin(op)) = serde_json::from_str(&line) {
                                seen.lock().unwrap().admin.push(op);
                            }
                        }
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
        .position(|e| e.meta.version == "Oni")
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

/// An admin's /delboard clears the selected map's board, a TJA course's by its chart hash;
/// /delboard with a hash clears that map's.
#[test]
fn an_admin_clears_the_selected_tja_course_s_board() {
    let f = Folders::new("delboard");
    let (server, seen) = server_for(true);
    let mut cfg = config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let (mut s, mut now) = client(&f, cfg);
    put(&f.tja, "Song", "song.tja", CHART);
    refresh(&mut s, &mut now);
    let until = Instant::now() + Duration::from_secs(30);
    while !(s.social.connected && s.admin) && Instant::now() < until {
        s.poll_social();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.admin, "logged in as an admin");
    let oni = s
        .library
        .iter()
        .position(|e| e.meta.version == "Oni")
        .unwrap();
    let hash = s.library[oni].hash();
    let path = s.library[oni].path.clone();
    s.select.select_path(&s.library, &path);
    s.admin_command("/delboard");
    s.admin_command("/delboard 00000000000000ff");
    let sent = |s: &mut State, now: &mut Instant| {
        wait_for(s, now, |_| seen.lock().unwrap().admin.len() >= 2)
    };
    assert!(sent(&mut s, &mut now), "both reach the server");
    assert_eq!(
        seen.lock().unwrap().admin,
        [
            online::AdminOp::DeleteBoard { map: hash },
            online::AdminOp::DeleteBoard { map: 0xff },
        ]
    );
}

/// The playfield of TJA chart `text`'s course (no inputs, `mods`), drawn at each of `times`
/// (map ms): where its small dons are along the lane, nearest first.
fn dons_drawn(text: &str, mods: &[&str], times: &[f64]) -> Vec<Vec<f32>> {
    dons_drawn_played(text, Vec::new(), mods, times)
}

/// The same, played with `inputs` (map ms, key).
fn dons_drawn_played(
    text: &str,
    inputs: Vec<(f64, u8)>,
    mods: &[&str],
    times: &[f64],
) -> Vec<Vec<f32>> {
    use crate::game::{Look, Play, set_frame_look};
    use crate::hit::Canvas;
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    crate::render::set_frame_time(Some(Instant::now()));
    let map = taiko_logic::tja::parse(text).unwrap().courses.remove(0).map;
    let r = super::gameplay::replay(inputs, mods, scores::new_version());
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
    drawn_played_at(text, Vec::new(), t)
}

/// The same, played with `inputs` (map ms, key).
fn drawn_played_at(
    text: &str,
    inputs: Vec<(f64, u8)>,
    t: f64,
) -> (Vec<(f32, f32)>, crate::hit::Canvas) {
    use crate::game::{Look, Play, set_frame_look};
    use crate::hit::Canvas;
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    crate::render::set_frame_time(Some(Instant::now()));
    let map = taiko_logic::tja::parse(text).unwrap().courses.remove(0).map;
    let r = super::gameplay::replay(inputs, &[], scores::new_version());
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

/// A negative #SCROLL brings a note in from the left over the drum panel: it isn't hidden under
/// it until the last few steps before the drum, as it was (a player's report: notes "way more
/// sudden" than they should be, without any #SUDDEN). A note passing the drum the usual way
/// still slides under the panel.
#[test]
fn a_note_from_the_left_comes_in_over_the_drum_panel() {
    let text = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n#SCROLL -1\n1,\n#END\n";
    // The note is at 2000; 300 ms ahead it is far left of the drum, where the panel is.
    let (dons, c) = drawn_at(text, 1700.0);
    assert_eq!(dons.len(), 1, "{dons:?}");
    let x = dons[0].0;
    assert!(
        x < taiko_logic::sim::HIT_X - 100.0,
        "coming from the left, over where the panel is: {x}"
    );
    let panel = c
        .shapes
        .iter()
        .position(|s| {
            (s.center[0] - crate::game::PANEL_W / 2.0).abs() < 0.5
                && (s.half[0] - crate::game::PANEL_W / 2.0).abs() < 0.5
                && s.fill[3] > 0.99
        })
        .expect("the drum panel");
    let note = c
        .shapes
        .iter()
        .position(|s| (s.half[0] - 31.0).abs() < 0.5 && (s.center[0] - x).abs() < 0.01)
        .unwrap();
    assert!(
        note > panel,
        "drawn over the panel: note {note}, panel {panel}"
    );
    // It still lands on the drum on time.
    let (dons, _) = drawn_at(text, 2000.0);
    assert!(
        dons.iter()
            .any(|d| (d.0 - taiko_logic::sim::HIT_X).abs() < 0.05),
        "{dons:?}"
    );
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

/// Notes coming in from the left over the drum panel and hit notes flying out of the drum go with
/// a target #JPOSSCROLL moved, like every other note.
#[test]
fn notes_over_the_panel_and_hit_ones_follow_the_moved_target() {
    let still = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n#SCROLL -1\n1,\n#END\n";
    let moved = still.replace("0,\n#SCROLL", "#JPOSSCROLL 0 60+30i 1\n0,\n#SCROLL");
    // From the left, 300 ms before its hit.
    let (home, _) = drawn_at(still, 1700.0);
    let (there, _) = drawn_at(&moved, 1700.0);
    assert_eq!(home.len(), 1, "{home:?}");
    assert_eq!(there.len(), 1, "{there:?}");
    let (dx, dy) = (there[0].0 - home[0].0, there[0].1 - home[0].1);
    assert!(
        (dx - 40.0).abs() < 0.05 && (dy - 20.0).abs() < 0.05,
        "moved with the target: {dx}, {dy}"
    );
    // Hit, it flies up out of the moved drum.
    let f = dons_drawn_played(&moved, vec![(2000.0, 1)], &[], &[2005.0]);
    assert!(
        f[0].iter()
            .any(|&x| (x - taiko_logic::sim::HIT_X - 40.0).abs() < 0.05),
        "{f:?}"
    );
}

/// With #JPOSSCROLL putting the target in the middle of the screen, notes come in from both
/// sides with as much lead (a player's video: from the left they popped up right next to the
/// target, cut off where the drum's home place would have been 191 units away).
#[test]
fn a_target_moved_to_the_middle_gets_notes_from_the_left_as_early_as_from_the_right() {
    let right = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n#JPOSSCROLL 0 400 1\n0,\n1,\n#END\n";
    let left = right.replace("0,\n1,", "0,\n#SCROLL -1\n1,");
    // OpenTaiko's 400 px are 266.67 units; the note is at 2000, 750 ms (294 units) away.
    let target = taiko_logic::sim::HIT_X + 400.0 * 2.0 / 3.0;
    let (r, _) = drawn_at(right, 1250.0);
    let (l, _) = drawn_at(&left, 1250.0);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(l.len(), 1, "drawn from the left too: {l:?}");
    let ahead = r[0].0 - target;
    assert!(ahead > 290.0, "{ahead}");
    assert!(
        (target - l[0].0 - ahead).abs() < 0.05,
        "as far left as the other is right: {l:?} {r:?}"
    );
}

/// #LYRIC lines show under the lane from their time until the next one.
#[test]
fn lyrics_show_under_the_lane() {
    let text = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n#LYRIC Sing along\n1,\n#LYRIC\n1,\n#END\n";
    assert!(!shows(&drawn_at(text, 1500.0).1, "Sing along"));
    assert!(shows(&drawn_at(text, 2500.0).1, "Sing along"));
    assert!(!shows(&drawn_at(text, 4500.0).1, "Sing along"), "cleared");
}

/// A TJA course shows its charter's LEVEL where maps show their star rating (as it was written:
/// "10", "1.5"), and by difficulty the list goes by it, whatever the star rating says.
#[test]
fn a_tja_course_shows_and_sorts_by_its_level() {
    let f = Folders::new("level");
    let chart = |title: &str, level: &str| {
        format!(
            "TITLE:{title}\nBPM:120\nWAVE:song.ogg\nCOURSE:Oni\nLEVEL:{level}\n#START\n1,\n#END\n"
        )
    };
    put(&f.tja, "Top", "top.tja", &chart("Top Level", "10"));
    put(&f.tja, "Low", "low.tja", &chart("Low Level", "1.5"));
    let (mut s, mut now) = client(&f, config());
    refresh(&mut s, &mut now);
    let top = s
        .library
        .iter()
        .position(|e| e.meta.title == "Top Level")
        .unwrap();
    let low = s
        .library
        .iter()
        .position(|e| e.meta.title == "Low Level")
        .unwrap();
    // Star ratings the other way round from the levels.
    let rated = |i: usize| (if i == top { 0.5 } else { 9.25 }) as f32;
    let mut info = vec![None; s.library.len()];
    for i in [top, low] {
        info[i] = Some(crate::mapinfo::Info {
            stars: rated(i),
            length_ms: 60_000.0,
            bpm: 120.0,
        });
    }
    s.select.info = Some(std::sync::Arc::new(std::sync::Mutex::new(info)));
    s.select.set_sort(ui::SortMode::Difficulty, &s.library);
    let order: Vec<usize> = s.select.songs().into_iter().flatten().collect();
    assert_eq!(order, [low, top], "by level, not by star rating");
    // Both open, so each shows its own row.
    let mut c = Canvas {
        audit: true,
        ..Canvas::default()
    };
    c.clear(853.3);
    s.select
        .draw(&mut c, &s.library, 16.0, "", &|i| Some(rated(i)), &|_| None);
    let pills: Vec<&str> = c
        .texts
        .iter()
        .map(|t| t.1.as_str())
        .filter(|t| t.parse::<f32>().is_ok())
        .collect();
    assert!(pills.contains(&"10") && pills.contains(&"1.5"), "{pills:?}");
    assert!(
        !pills.iter().any(|p| *p == "0.50" || *p == "9.25"),
        "no star rating for them: {pills:?}"
    );
}

/// A TJA chart is found however many folders down it is (a pack, its genres, then the song), from
/// the cache too, and a song added deep in a pack later shows up once the library is looked
/// through again.
#[test]
fn tja_charts_are_found_at_any_depth() {
    let f = Folders::new("deep");
    let (mut s, mut now) = client(&f, config());
    put(&f.tja, "Pack/Genre/Song", "song.tja", CHART);
    put(&f.tja, "A/B/C/D/E/F", "deep.tja", CHART);
    let titled = |s: &State| {
        let mut files: Vec<PathBuf> = tja_entries(s)
            .iter()
            .filter_map(|e| tja_maps::split(&e.path).map(|(file, _)| file))
            .collect();
        files.sort();
        files.dedup();
        files
    };
    refresh(&mut s, &mut now);
    let both = [
        f.tja.join("A/B/C/D/E/F/deep.tja"),
        f.tja.join("Pack/Genre/Song/song.tja"),
    ];
    assert_eq!(titled(&s), both, "two courses each, wherever they are");
    // Again, from the cache.
    refresh(&mut s, &mut now);
    assert_eq!(titled(&s), both);
    put(&f.tja, "Pack/Genre/Later", "later.tja", CHART);
    refresh(&mut s, &mut now);
    assert_eq!(
        titled(&s),
        [
            f.tja.join("A/B/C/D/E/F/deep.tja"),
            f.tja.join("Pack/Genre/Later/later.tja"),
            f.tja.join("Pack/Genre/Song/song.tja"),
        ]
    );
}

/// Sorted by folder, TJA charts are headed by the folder their song's folder is in (a pack and
/// its genre), packs first, then osu!'s songs.
#[test]
fn sorting_by_folder_heads_tja_charts_by_their_pack() {
    let f = Folders::new("folders");
    let (mut s, mut now) = client(&f, config());
    let chart = |title: &str| CHART.replace("Regression TJA", title);
    put(&f.tja, "Pack1/Genre A/One", "one.tja", &chart("One"));
    put(&f.tja, "Pack1/Genre A/Two", "two.tja", &chart("Two"));
    put(&f.tja, "Pack1/Genre B/Three", "three.tja", &chart("Three"));
    put(&f.tja, "Pack2/Four", "four.tja", &chart("Four"));
    put(&f.tja, "Five", "five.tja", &chart("Five"));
    put(
        &f.songs,
        "1 Artist - Osu",
        "osu.osu",
        "osu file format v14\n\n[General]\nMode: 1\n\n[Metadata]\nTitle:Osu\nArtist:Artist\nVersion:Oni\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    );
    refresh(&mut s, &mut now);
    s.select.headings = true;
    s.select.set_sort(ui::SortMode::Folder, &s.library);
    assert_eq!(
        s.select.sections(),
        [
            ("Pack1 / Genre A".to_string(), 2),
            ("Pack1 / Genre B".to_string(), 1),
            ("Pack2".to_string(), 1),
            ("TJA".to_string(), 1),
            ("osu! Songs".to_string(), 1),
        ]
    );
    let titles: Vec<&str> = s
        .select
        .songs()
        .iter()
        .map(|g| s.library[g[0]].meta.title.as_str())
        .collect();
    assert_eq!(titles, ["One", "Two", "Three", "Four", "Five", "Osu"]);
    assert_eq!(ui::SortMode::parse("folder"), Some(ui::SortMode::Folder));
}

/// A complex #SCROLL and #DIRECTION send notes in from above, below and the corners, as fast as
/// they would come along the lane and meeting the drum on time; OpenTaiko's own axes, an
/// imaginary part pointing down.
#[test]
fn notes_come_in_from_any_side() {
    let chart = |scroll: &str| {
        format!("TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n{scroll}\n0,\n1,\n#END\n")
    };
    let at = |text: &str, t: f64| {
        let (dons, _) = drawn_at(text, t);
        assert_eq!(dons.len(), 1, "{dons:?}");
        dons[0]
    };
    let plain = chart("");
    let (hx, hy) = at(&plain, 2000.0);
    let (px, _) = at(&plain, 1500.0);
    let d = px - hx;
    assert!(d > 50.0, "half a second of lane: {d}");
    let near = |(x, y): (f32, f32), (ex, ey): (f32, f32), what: &str| {
        assert!(
            (x - ex).abs() < 0.1 && (y - ey).abs() < 0.1,
            "{what}: at {x}, {y}, not {ex}, {ey}"
        );
    };
    for (scroll, (ux, uy)) in [
        ("#SCROLL i", (0.0, 1.0)),
        ("#SCROLL -i", (0.0, -1.0)),
        ("#SCROLL 1+i", (1.0, 1.0)),
        ("#DIRECTION 1", (0.0, -1.0)),
        ("#DIRECTION 7", (-1.0, 1.0)),
    ] {
        let text = chart(scroll);
        near(at(&text, 2000.0), (hx, hy), &format!("{scroll} at its hit"));
        near(
            at(&text, 1500.0),
            (hx + d * ux, hy + d * uy),
            &format!("{scroll} half a second before"),
        );
    }
}

/// Two bars of kats, then a branch picked by accuracy: Normal plays kats, Expert big kats,
/// Master small dons; then a shared bar of kats.
const BRANCHING: &str = "TITLE:Branches\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n2222,\n2222,\n#BRANCHSTART p,60,90\n#N\n2222,\n#E\n4444,\n#M\n1111,\n#BRANCHEND\n2222,\n#END\n";

/// Every kat of the two bars before the branch hit on time (left rim).
fn kats_hit() -> Vec<(f64, u8)> {
    (0..8).map(|k| (k as f64 * 500.0, 0)).collect()
}

/// A branching chart plays the branch the player earned: picked a bar ahead (at 2000 here), it
/// takes over where it starts (4000), and only its notes come in. Its name shows under the
/// lane.
#[test]
fn a_branching_chart_plays_the_branch_earned_and_shows_it() {
    let (dons, c) = drawn_played_at(BRANCHING, kats_hit(), 3500.0);
    assert!(dons.len() >= 2, "Master's dons come in: {dons:?}");
    assert!(shows(&c, "NORMAL"), "still Normal until the branch starts");
    assert!(!shows(&c, "MASTER"));
    let (_, c) = drawn_played_at(BRANCHING, kats_hit(), 4600.0);
    assert!(shows(&c, "MASTER"));
    // Nothing hit: Normal's kats come in, and none of Master's dons.
    let (dons, c) = drawn_played_at(BRANCHING, Vec::new(), 3500.0);
    assert!(dons.is_empty(), "{dons:?}");
    let (_, after) = drawn_played_at(BRANCHING, Vec::new(), 4600.0);
    assert!(shows(&c, "NORMAL") && shows(&after, "NORMAL"));
    // A chart that doesn't branch shows no branch.
    let (_, plain) = drawn_at(&BRANCHING.replace("#BRANCHSTART p,60,90\n", ""), 4600.0);
    assert!(!shows(&plain, "NORMAL") && !shows(&plain, "MASTER"));
}

/// The song list rates a branching chart by its Master branch, as the arcade lists it.
#[test]
fn a_branching_chart_is_listed_by_its_master_branch() {
    let f = Folders::new("branch-info");
    let master_only = BRANCHING
        .replace("#BRANCHSTART p,60,90\n#N\n2222,\n#E\n4444,\n#M\n", "")
        .replace("#BRANCHEND\n", "");
    let oni = taiko_logic::tja::CourseKind::Oni;
    let branching = put(&f.tja, "A", "a.tja", BRANCHING);
    let plain = put(&f.tja, "B", "b.tja", &master_only);
    let a = mapinfo_of(&tja_maps::course_path(&branching, oni));
    let b = mapinfo_of(&tja_maps::course_path(&plain, oni));
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

/// A negative #BPMCHANGE (with a negative #MEASURE, so time runs on) sends its notes in from the
/// left, as fast as from the right at that BPM, and they meet the drum on time; it was ignored,
/// the notes coming from the right at the BPM before.
#[test]
fn a_negative_bpm_brings_notes_in_from_the_left() {
    let plain = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n1,\n#END\n";
    let turned = plain.replace("0,\n1,", "0,\n#BPMCHANGE -120\n#MEASURE -4/4\n1,");
    let at = |text: &str, t: f64| {
        let (dons, _) = drawn_at(text, t);
        assert_eq!(dons.len(), 1, "{dons:?}");
        dons[0]
    };
    let hx = taiko_logic::sim::HIT_X;
    let (rx, ry) = at(plain, 1700.0);
    let (lx, ly) = at(&turned, 1700.0);
    assert!(rx > hx + 50.0, "the plain one from the right: {rx}");
    assert!(
        (hx - lx - (rx - hx)).abs() < 0.1 && (ly - ry).abs() < 0.1,
        "as far left of the drum as the plain one is right of it: {lx}, {rx}"
    );
    assert!(
        (at(&turned, 2000.0).0 - hx).abs() < 0.05,
        "on the drum at its hit"
    );
}

/// OpenTaiko's notes as they come in: a bomb dark, a Kadon purple, an ADLib not at all until
/// it's hit; a bomb hit goes off in a burst round the drum.
#[test]
fn bombs_and_kadons_show_and_adlibs_stay_hidden() {
    let text = |body: &str| {
        format!("TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n0,\n{body}\n#END\n")
    };
    // What is drawn coming in (300 ms ahead, well right of the drum), note-sized, by its fill.
    let notes = |body: &str| {
        let (_, c) = drawn_at(&text(body), 1700.0);
        c.shapes
            .iter()
            .filter(|s| {
                s.center[0] > taiko_logic::sim::HIT_X + 40.0
                    && (25.0..60.0).contains(&s.half[0])
                    && s.fill[3] > 0.5
            })
            .map(|s| s.fill)
            .collect::<Vec<_>>()
    };
    let bomb = notes("C,");
    assert!(
        bomb.iter().any(|f| f[0] < 0.3 && f[1] < 0.3 && f[2] < 0.3),
        "a dark bomb: {bomb:?}"
    );
    let kadon = notes("G,");
    assert!(
        kadon.iter().any(|f| f[0] > 0.5 && f[2] > 0.7 && f[1] < 0.4),
        "a purple Kadon: {kadon:?}"
    );
    assert!(notes("F,").is_empty(), "the ADLib is hidden");
    assert!(!notes("1,").is_empty(), "a don shows as ever");
    // Hit, the bomb goes off: red rings round the drum.
    let red = |c: &crate::hit::Canvas| {
        c.shapes
            .iter()
            .any(|s| s.border[3] > 0.3 && s.border[0] > 0.8 && s.border[1] < 0.4)
    };
    let (_, hit) = drawn_played_at(&text("C,"), vec![(2000.0, 1)], 2100.0);
    let (_, by) = drawn_played_at(&text("C,"), Vec::new(), 2100.0);
    assert!(red(&hit), "the burst");
    assert!(!red(&by), "nothing went off");
}

/// A chart's picture (`BGIMAGE`, else `PREIMAGE`) is its background in the library, with a
/// Windows path read as any system takes it.
#[test]
fn a_tja_chart_s_picture_is_its_background() {
    let f = Folders::new("tja-picture");
    let text =
        "TITLE:A\nBPM:120\nPREIMAGE:cover.jpg\nBGIMAGE:img\\bg.png\nCOURSE:Oni\n#START\n1,\n#END\n";
    let chart = put(&f.tja, "A", "a.tja", text);
    let entries = tja_maps::entries(&chart).unwrap();
    assert_eq!(entries[0].1.background.as_deref(), Some("img/bg.png"));
    let cover = put(
        &f.tja,
        "B",
        "b.tja",
        &text.replace("BGIMAGE:img\\bg.png\n", ""),
    );
    let entries = tja_maps::entries(&cover).unwrap();
    assert_eq!(entries[0].1.background.as_deref(), Some("cover.jpg"));
}

/// A pack song `title` playing `wave`, with the picture its genre's songs share.
fn pack_chart(title: &str, wave: &str) -> String {
    CHART
        .replace("Regression TJA", title)
        .replace("WAVE:song.ogg", &format!("WAVE:{wave}\nBGIMAGE:genre.png"))
}

/// Pack songs `titles` side by side in `dir` (below the TJA folder), each with its music,
/// and the genre's picture: their chart files.
fn pack_songs(f: &Folders, dir: &str, titles: &[&str]) -> Vec<PathBuf> {
    let files = titles
        .iter()
        .map(|t| {
            let name = t.to_lowercase();
            let wave = format!("{name}.ogg");
            std::fs::create_dir_all(f.tja.join(dir)).unwrap();
            std::fs::write(f.tja.join(dir).join(&wave), b"ogg").unwrap();
            put(&f.tja, dir, &format!("{name}.tja"), &pack_chart(t, &wave))
        })
        .collect();
    std::fs::write(f.tja.join(dir).join("genre.png"), b"png").unwrap();
    files
}

/// The chart files of the song select's songs, in order, each song's once.
fn listed_songs(s: &State) -> Vec<Vec<PathBuf>> {
    let rows = s.select.rows();
    s.select
        .song_groups()
        .iter()
        .map(|g| {
            let mut files: Vec<PathBuf> = rows[g.clone()]
                .iter()
                .map(|&i| s.library[i].file().to_path_buf())
                .collect();
            files.dedup();
            files
        })
        .collect()
}

/// Puts the cursor on the library map at `path`, looked for again if a frame moved the rows.
fn select_map(s: &mut State, now: Instant, path: &Path) {
    for _ in 0..5 {
        let at = s
            .select
            .rows()
            .iter()
            .position(|&i| s.library[i].path == path)
            .expect("the course is listed");
        s.select.set_cursor(at);
        frame(s, now);
        if s.select.selected().map(|i| s.library[i].path.as_path()) == Some(path) {
            return;
        }
    }
    panic!("{} isn't selected", path.display());
}

/// Where `original` went in the tests' bin, by the note left beside it.
fn in_bin(s: &State, original: &Path) -> Option<PathBuf> {
    let map_trash::Bin::Folder(bin) = &s.map_bin else {
        panic!("tests' maps go to a folder of their own");
    };
    std::fs::read_dir(bin).ok()?.flatten().find_map(|slot| {
        let note = std::fs::read_to_string(slot.path().join(map_trash::FROM_NOTE)).ok()?;
        (note.trim_end() == original.to_string_lossy())
            .then(|| slot.path().join(original.file_name().unwrap()))
    })
}

fn listed_course(s: &State, file: &Path) -> bool {
    s.select.rows().iter().any(|&i| s.library[i].file() == file)
}

/// A pack keeping many songs' `.tja` files in one folder lists a song per file, each with its
/// own courses, not the whole folder as one song with every course of every chart in it.
#[test]
fn a_folder_of_many_tja_files_is_a_song_per_file() {
    let f = Folders::new("flat");
    let (mut s, mut now) = client(&f, config());
    let files = pack_songs(&f, "Pack/Genre", &["One", "Two", "Three"]);
    refresh(&mut s, &mut now);
    let mut songs = listed_songs(&s);
    songs.sort();
    let mut each: Vec<Vec<PathBuf>> = files.iter().map(|p| vec![p.clone()]).collect();
    each.sort();
    assert_eq!(songs, each, "a song a file");
    for g in s.select.song_groups() {
        assert_eq!(g.len(), 2, "each song with its own two courses");
    }
    // Right goes to the next song, not past the folder.
    let first = s.select.rows()[0];
    let first_file = s.library[first].file().to_path_buf();
    let first_path = s.library[first].path.clone();
    select_map(&mut s, now, &first_path);
    press(&mut s, KeyCode::ArrowRight, now);
    let next = s
        .select
        .selected()
        .map(|i| s.library[i].file().to_path_buf());
    assert!(
        next.is_some_and(|n| n != first_file && files.contains(&n)),
        "the next song of the same folder"
    );
    // A course can't be deleted alone (its chart file holds every course): the menu offers the
    // song only.
    s.select.menu = Some(ui::SongMenu {
        row: s.select.cursor(),
        header: false,
        skin: false,
        previous: None,
        x: 100.0,
        y: 100.0,
    });
    let actions: Vec<ui::MenuAction> = s
        .select
        .menu_items(&s.library)
        .iter()
        .map(|it| it.action)
        .collect();
    assert!(actions.contains(&ui::MenuAction::DeleteSet));
    assert!(!actions.contains(&ui::MenuAction::DeleteDifficulty));
}

/// Deleting a song of a pack folder holding many takes its chart file and its music only: the
/// folder, the picture the other songs show, and the other songs stay.
#[test]
fn deleting_a_tja_song_of_a_pack_folder_takes_only_its_files() {
    let f = Folders::new("delete-flat");
    let (mut s, mut now) = client(&f, config());
    let files = pack_songs(&f, "Pack/Genre", &["One", "Two", "Three"]);
    let genre = f.tja.join("Pack/Genre");
    refresh(&mut s, &mut now);
    let oni = tja_maps::course_path(&files[0], taiko_logic::tja::CourseKind::Oni);
    select_map(&mut s, now, &oni);
    // Delete without Shift: a TJA course is its whole song.
    press(&mut s, KeyCode::Delete, now);
    let ask = s.select.deleting.clone().expect("asked first");
    assert!(ask.set && !ask.whole, "the song, not its folder");
    assert_eq!(
        ask.maps.len(),
        2,
        "its two courses, none of the other songs'"
    );
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    frame(&mut s, now);

    let said = s.toast.as_ref().map(|t| t.text.clone());
    assert!(!files[0].exists(), "its chart file went: {said:?}");
    assert!(in_bin(&s, &files[0]).is_some(), "into the bin: {said:?}");
    let music = genre.join("one.ogg");
    assert!(
        !music.exists() && in_bin(&s, &music).is_some(),
        "its music, no other chart's, goes too"
    );
    assert!(
        genre.join("genre.png").is_file(),
        "the picture the others name stays"
    );
    assert!(!listed_course(&s, &files[0]), "its rows go at once");
    for (file, wave) in files[1..].iter().zip(["two.ogg", "three.ogg"]) {
        assert!(file.is_file() && genre.join(wave).is_file());
        assert!(listed_course(&s, file), "the other songs stay listed");
    }
}

/// A song's folder with more songs in folders below it isn't taken whole with the song; a song
/// alone in its folder is.
#[test]
fn a_tja_song_folder_goes_whole_only_with_no_other_song_in_it() {
    let f = Folders::new("delete-nested");
    let (mut s, mut now) = client(&f, config());
    let top = pack_songs(&f, "Pack/Box", &["Top"]).remove(0);
    let deep = pack_songs(&f, "Pack/Box/Genre/Song", &["Deep"]).remove(0);
    refresh(&mut s, &mut now);

    select_map(
        &mut s,
        now,
        &tja_maps::course_path(&top, taiko_logic::tja::CourseKind::Oni),
    );
    s.modifiers_shift = true;
    press(&mut s, KeyCode::Delete, now);
    s.modifiers_shift = false;
    let ask = s.select.deleting.clone().expect("asked first");
    assert!(ask.set && !ask.whole, "a song below keeps the folder");
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    frame(&mut s, now);
    let said = s.toast.as_ref().map(|t| t.text.clone());
    assert!(!top.exists(), "its chart file went: {said:?}");
    assert!(in_bin(&s, &top).is_some(), "into the bin: {said:?}");
    assert!(
        deep.is_file() && listed_course(&s, &deep),
        "the song below stays"
    );

    select_map(
        &mut s,
        now,
        &tja_maps::course_path(&deep, taiko_logic::tja::CourseKind::Easy),
    );
    press(&mut s, KeyCode::Delete, now);
    let ask = s.select.deleting.clone().expect("asked first");
    assert!(
        ask.set && ask.whole,
        "alone in its folder: the folder goes with it"
    );
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    frame(&mut s, now);
    let folder = f.tja.join("Pack/Box/Genre/Song");
    assert!(!folder.exists() && in_bin(&s, &folder).is_some());
    assert!(f.tja.join("Pack/Box").is_dir());
}

/// A bar of zero (`#MEASURE 0/1`, TaikoManyGimmicks charts stack notes with it hundreds of
/// times) lasts no time, as in OpenTaiko: its notes and the next bar's meet the drum together.
/// It was ignored, the bar before's length used, and every one pushed the rest of the chart
/// later off the song.
#[test]
fn notes_after_a_bar_of_zero_meet_the_drum_on_time() {
    let text = "TITLE:Gimmicks\nBPM:120\nOFFSET:0\nCOURSE:Oni\n#START\n#MEASURE 1/4\n0,\n#MEASURE 0/1\n1,\n#MEASURE 4/4\n1,\n#END\n";
    let (dons, _) = drawn_at(text, 500.0);
    let hx = taiko_logic::sim::HIT_X;
    assert_eq!(dons.len(), 2, "{dons:?}");
    assert!(
        dons.iter().all(|d| (d.0 - hx).abs() < 0.05),
        "both on the drum at 500: {dons:?}"
    );
}
