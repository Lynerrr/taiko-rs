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
