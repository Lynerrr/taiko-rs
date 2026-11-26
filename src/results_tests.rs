//! A score's results screen worked out in the background: from a local replay, and from an
//! online one fetched from a server.

use super::*;

/// A map of 400 notes (every third a kat), 150 ms apart, in a folder of its own.
fn map(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("taiko-rs-results-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    let mut text = String::from(
        "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:6\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n[HitObjects]\n",
    );
    for k in 0..400 {
        let sound = if k % 3 == 0 { 2 } else { 0 };
        text += &format!("256,192,{},1,{sound},0:0:0:0:\n", 1000 + k * 150);
    }
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

/// Every note hit `late` ms late, the right colour (in map time, which loading moves later by
/// `OSU_TIMING_MS`).
fn inputs(late: f64) -> Vec<(f64, u8)> {
    (0..400)
        .map(|k| {
            (
                (1000 + k * 150) as f64 + beatmap::OSU_TIMING_MS + late,
                if k % 3 == 0 { 0 } else { 1 },
            )
        })
        .collect()
}

fn wait(rx: &Receiver<ui::Detail>) -> ui::Detail {
    rx.recv_timeout(Duration::from_secs(60))
        .expect("the results' background work")
}

fn replay(path: &std::path::Path, inputs: Vec<(f64, u8)>) -> scores::Replay {
    scores::Replay {
        version: scores::new_version(),
        map_hash: 1,
        map_path: path.to_string_lossy().into_owned(),
        map_label: "map".into(),
        player: "someone".into(),
        date: 0,
        rate: 1.0,
        keep_pitch: true,
        anti_mash: true,
        mods: Vec::new(),
        scroll_speed: 1.0,
        score: 1_000_000,
        accuracy: 1.0,
        max_combo: 400,
        great: 400,
        ok: 0,
        miss: 0,
        ur: 0.0,
        inputs,
        seed: 0,
    }
}

#[test]
fn a_local_score_gets_its_stats_and_pp_from_its_replay() {
    let (dir, path) = map("local");
    let file = dir.join("replay.json");
    let r = replay(&path, inputs(12.0));
    std::fs::write(&file, serde_json::to_vec(&r).unwrap()).unwrap();
    let d = wait(&results_job(path.clone(), r.pp_play(), Some(file), None));
    let (stats, _) = d.stats.expect("played through");
    assert_eq!((stats.great, stats.ok, stats.miss), (400, 0, 0));
    assert!(
        (stats.mean_error() - 12.0).abs() < 1.0,
        "{}",
        stats.mean_error()
    );
    let b = d.breakdown.expect("pp");
    assert!(
        b.pp > 0.0 && (b.pp - b.if_ss).abs() < 1e-6,
        "an SS is its own SS: {b:?}"
    );
    assert!(d.replay.is_none(), "a local replay is already there");

    // On the results screen: the counts stay, the replay adds the rest.
    let mut results = ui::Results::new(
        "map".into(),
        game::Stats {
            great: 400,
            max_combo: 400,
            ..Default::default()
        },
        game::Windows::new(5.0),
    );
    results.partial = true;
    let (tx, rx) = mpsc::channel();
    tx.send(ui::Detail {
        stats: Some((stats, game::Windows::new(6.0))),
        breakdown: Some(b),
        replay: None,
        seed: Some(42),
    })
    .unwrap();
    results.job = Some(rx);
    results.poll();
    assert!(!results.partial && results.job.is_none());
    assert_eq!(results.stats.errors.len(), 400);
    assert_eq!(results.pp, Some(b.pp));
    assert_eq!(results.seed, Some(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_online_score_fetches_its_replay() {
    use std::io::{BufRead, Write};
    let (dir, path) = map("online");
    // A server that answers one request with the replay.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap().to_string();
    let presses = inputs(-8.0);
    let sent = presses.clone();
    let serve = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let asked: online::Envelope = serde_json::from_str(&line).unwrap();
        let id = match asked.request {
            online::Request::Replay { id } => id,
            other => panic!("asked for {other:?}"),
        };
        let map = net::MapInfo {
            hash: 1,
            artist: String::new(),
            title: String::new(),
            version: String::new(),
            rate: 1.0,
            from: None,
            set_id: None,
            mods: Vec::new(),
            multiverse: None,
            shapeshift: None,
            seed: None,
            defense: None,

            stats: None,
            host_mods: false,
        };
        let reply = online::Reply::Replay(Box::new(online::OnlineReplay {
            version: 2,
            map,
            inputs: sent,
        }));
        let mut out = serde_json::to_string(&reply).unwrap();
        out.push('\n');
        reader.get_mut().write_all(out.as_bytes()).unwrap();
        id
    });
    let id = 4_000_000_000 + std::process::id() as u64;
    let _ = std::fs::remove_file(scores::online_replay_path(id).unwrap());
    let score = online::OnlineScore {
        player: "friend".into(),
        map_hash: 1,
        score: 1_000_000,
        accuracy: 1.0,
        max_combo: 400,
        great: 400,
        rate: 1.0,
        id,
        ..Default::default()
    };
    let play = pp::Play {
        great: 400,
        ok: 0,
        miss: 0,
        max_combo: 400,
        rate: 1.0,
        mods: Mods::NONE,
        scroll_speed: 1.0,
        seed: 0,
        half: Default::default(),
    };
    let d = wait(&results_job(
        path.clone(),
        play,
        None,
        Some((server, score, "map".into())),
    ));
    assert_eq!(serve.join().unwrap(), id);
    let file = d.replay.expect("fetched").expect("saved");
    let r = scores::load_replay(&file).unwrap();
    assert_eq!(
        (r.player.as_str(), r.inputs.len(), r.map_path.as_str()),
        ("friend", presses.len(), path.to_str().unwrap())
    );
    let (stats, _) = d.stats.expect("played through");
    assert!(
        (stats.mean_error() + 8.0).abs() < 1.0,
        "{}",
        stats.mean_error()
    );
    assert!(d.breakdown.is_some());
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn servers_without_replays_say_so() {
    let (dir, path) = map("old-server");
    let score = online::OnlineScore {
        player: "friend".into(),
        id: 0,
        ..Default::default()
    };
    let play = pp::Play {
        great: 400,
        ok: 0,
        miss: 0,
        max_combo: 400,
        rate: 1.0,
        mods: Mods::NONE,
        scroll_speed: 1.0,
        seed: 0,
        half: Default::default(),
    };
    let d = wait(&results_job(
        path,
        play,
        None,
        Some(("127.0.0.1:9".into(), score, "map".into())),
    ));
    assert_eq!(
        d.replay,
        Some(Err("the server doesn't keep replays yet".into()))
    );
    assert!(d.stats.is_none() && d.breakdown.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_score_link_opens_its_results_on_its_map() {
    use std::io::{BufRead, Write};
    let (dir, path) = map("link");
    let other = dir.join("other.osu");
    std::fs::write(&other, b"another difficulty").unwrap();
    let meta = |version: &str, set_id| beatmap::Meta {
        artist: "A".into(),
        title: "T".into(),
        version: version.into(),
        set_id,
        mode: 1,
        ..Default::default()
    };
    let library = vec![
        library::Entry::new(other, meta("Easy", Some(123))),
        library::Entry::new(path.clone(), meta("Oni", Some(123))),
    ];
    let hash = library[1].hash();
    // A server with two scores: one on that map, one on a map nobody has (and no set to fetch).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let asked: online::Envelope = serde_json::from_str(&line).unwrap();
            let reply = match asked.request {
                online::Request::Score { id: 41 } => {
                    online::Reply::Score(Box::new(online::OnlineScore {
                        player: "friend".into(),
                        map: "A - T [Oni]".into(),
                        map_hash: hash,
                        score: 900_000,
                        accuracy: 0.97,
                        great: 380,
                        ok: 20,
                        rate: 1.0,
                        set_id: Some(123),
                        id: 41,
                        ..Default::default()
                    }))
                }
                online::Request::Score { id: 42 } => {
                    online::Reply::Score(Box::new(online::OnlineScore {
                        map: "B - U [Hard]".into(),
                        map_hash: 5,
                        id: 42,
                        ..Default::default()
                    }))
                }
                _ => online::Reply::Error("nothing here".into()),
            };
            let mut out = serde_json::to_string(&reply).unwrap();
            out.push('\n');
            let _ = reader.get_mut().write_all(out.as_bytes());
        }
    });
    let mut s = crate::layout_tests::state();
    s.cfg.online_server = server;
    let (tx, rx) = mpsc::channel();
    tx.send(Scanned::Done(library)).unwrap();
    s.scan = Some((rx, Arc::new(AtomicUsize::new(0))));
    // Song select, not straight into the first map (the test state's command-line map).
    s.play_when_scanned = false;
    s.poll_scan();
    let settle = |s: &mut State| {
        let until = Instant::now() + Duration::from_secs(10);
        while s.score_link.is_some() && Instant::now() < until {
            s.poll_score_link();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    s.open_link(link::Link::Score(41));
    settle(&mut s);
    match s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        Some(crate::screens::results::ResultsScreen { results, entry, .. }) => {
            assert_eq!(*entry, 1);
            assert_eq!(
                results
                    .opened
                    .as_ref()
                    .map(|o| (o.player.as_str(), o.online)),
                Some(("friend", true))
            );
        }
        _ => panic!("not on the score's results"),
    }
    assert_eq!(s.select.selected(), Some(1));
    s.scene = SongSelect::scene();
    s.open_link(link::Link::Score(42));
    settle(&mut s);
    assert!(s.scene.is_select());
    assert_eq!(
        s.toast.as_ref().map(|t| t.text.as_str()),
        Some("you haven't got B - U [Hard]")
    );
    s.open_link(link::Link::Score(43));
    settle(&mut s);
    assert_eq!(
        s.toast.as_ref().map(|t| t.text.as_str()),
        Some("can't open that score: nothing here")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replays_before_version_5_keep_the_windows_they_were_judged_with() {
    let mut r = replay(std::path::Path::new("x.osu"), Vec::new());
    r.version = 4;
    assert_eq!(r.window_rule(), crate::game::WindowRule::Exact);
    r.version = scores::new_version();
    assert_eq!(r.window_rule(), crate::game::WindowRule::Osu);
}

#[test]
fn an_old_replay_plays_back_with_the_windows_it_was_judged_with() {
    // OD 6: a great is 32 ms wide, 31.5 as osu! judges it. Every hit 31.8 ms late.
    let (dir, path) = map("old-windows");
    let mut r = replay(&path, inputs(31.8));
    r.version = 4;
    let (old, _) = scores::simulate(&path, &r).expect("played through");
    assert_eq!((old.great, old.ok), (400, 0), "judged as it was played");
    r.version = scores::new_version();
    let (new, windows) = scores::simulate(&path, &r).expect("played through");
    assert_eq!((new.great, new.ok), (0, 400));
    assert_eq!(windows.great, 31.5);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With "Restart on SD/PF fail" on, a Sudden Death fail goes straight into a retry of the map
/// (after a shorter FAILED sign); off, it shows the failed results as before.
#[test]
fn a_sudden_death_fail_starts_the_map_over_when_asked() {
    let (dir, path) = map("fail-restart");
    for restart in [true, false] {
        let mut s = crate::layout_tests::state();
        s.cfg.mods = Mods::parse("SD");
        s.cfg.fail_restart = restart;
        let meta = beatmap::Meta {
            mode: 1,
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        tx.send(Scanned::Done(vec![library::Entry::new(path.clone(), meta)]))
            .unwrap();
        s.scan = Some((rx, Arc::new(AtomicUsize::new(0))));
        s.play_when_scanned = false;
        s.poll_scan();
        s.start_loading_from(0, None);
        let frame = |s: &mut State| {
            let mut c = Canvas::default();
            s.build_frame(&mut c, Instant::now(), 853.0);
        };
        let until = Instant::now() + Duration::from_secs(10);
        while !s.scene.is_play() && Instant::now() < until {
            frame(&mut s);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (true, Some(game)) = (s.scene.is_play(), s.play.as_deref_mut()) else {
            panic!("the play never started");
        };
        assert_eq!(
            game.fail_linger(),
            if restart { FAIL_RESTART_LINGER } else { 1.6 }
        );
        // Three notes go by unhit: the first miss fails the play.
        let t = game.map().notes[2].time + 500.0;
        game.set_time(t);
        assert!(game.failed_early());
        game.set_fail_linger(0.0);
        while s.scene.is_play() && Instant::now() < until {
            frame(&mut s);
            std::thread::sleep(Duration::from_millis(2));
        }
        match (
            s.scene.screen_as::<crate::screens::loading::Loading>(),
            s.scene
                .screen_as::<crate::screens::results::ResultsScreen>(),
        ) {
            (Some(l), _) if restart => assert!(l.retry && l.entry == 0),
            (_, Some(r)) if !restart => assert!(r.results.failed),
            _ => panic!("restart {restart}: ended up elsewhere"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The results of a Sudden Death play failed on its first miss, by a player logged in or not.
fn failed_results(name: &str, logged_in: bool) -> State {
    let (_dir, path) = map(name);
    let mut s = crate::layout_tests::state();
    s.cfg.mods = Mods::parse("SD");
    s.cfg.fail_restart = false;
    // Nothing reaches a real server from here.
    s.cfg.online_server = "127.0.0.1:9".into();
    if logged_in {
        s.cfg.online_token = "t".into();
    }
    let meta = beatmap::Meta {
        mode: 1,
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    tx.send(Scanned::Done(vec![library::Entry::new(path, meta)]))
        .unwrap();
    s.scan = Some((rx, Arc::new(AtomicUsize::new(0))));
    s.play_when_scanned = false;
    s.poll_scan();
    s.start_loading_from(0, None);
    let frame = |s: &mut State| {
        let mut c = Canvas::default();
        s.build_frame(&mut c, Instant::now(), 853.0);
    };
    let until = Instant::now() + Duration::from_secs(10);
    while !s.scene.is_play() && Instant::now() < until {
        frame(&mut s);
        std::thread::sleep(Duration::from_millis(2));
    }
    let Some(game) = s.play.as_deref_mut() else {
        panic!("the play never started");
    };
    let t = game.map().notes[2].time + 500.0;
    game.set_time(t);
    game.set_fail_linger(0.0);
    while s.scene.is_play() && Instant::now() < until {
        frame(&mut s);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        s.scene
            .screen_is::<crate::screens::results::ResultsScreen>()
    );
    s
}

fn results_of(s: &mut State) -> &mut ui::Results {
    &mut s
        .scene
        .screen_mut::<crate::screens::results::ResultsScreen>()
        .expect("on the results")
        .results
}

#[test]
fn a_player_logged_in_can_like_or_dislike_the_map_they_just_played() {
    let mut s = failed_results("vote-logged-in", true);
    let label = s.library[0].label();
    let hash = s.library[0].hash();
    let r = results_of(&mut s);
    let on = r
        .vote_on
        .clone()
        .expect("their own play, even failed, can be voted on");
    assert_eq!((on.hash, on.name.as_str()), (hash, label.as_str()));
    use online::MapVote::{Dislike, Like};
    // Again takes it back; the other one replaces it.
    assert_eq!(r.toggle_vote(Like).map(|v| v.1), Some(Some(Like)));
    assert_eq!(r.toggle_vote(Like).map(|v| v.1), Some(None));
    assert_eq!(r.toggle_vote(Like).map(|v| v.1), Some(Some(Like)));
    assert_eq!(r.toggle_vote(Dislike).map(|v| v.1), Some(Some(Dislike)));
    assert_eq!(r.vote, Some(Dislike));
    // The thumb pops from the moment of the vote.
    assert!(
        r.voted_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
    );
    // N on the screen takes the dislike back, L likes.
    s.key(KeyCode::KeyN, false, None, Instant::now(), None);
    assert_eq!(results_of(&mut s).vote, None);
    s.key(KeyCode::KeyL, false, None, Instant::now(), None);
    assert_eq!(results_of(&mut s).vote, Some(Like));
    match s.vote_request(on.clone(), Some(Like)) {
        Some(online::Request::MapVote { token, map, vote }) => {
            assert_eq!((token.as_str(), map, vote), ("t", on, Some(Like)));
        }
        other => panic!("not a vote: {other:?}"),
    }
}

#[test]
fn a_player_logged_out_has_nobody_to_tell_what_they_think_of_the_map() {
    let mut s = failed_results("vote-logged-out", false);
    let r = results_of(&mut s);
    assert!(r.vote_on.is_none());
    assert!(r.toggle_vote(online::MapVote::Like).is_none());
    assert!(r.vote.is_none());
    let map = online::PlayedMap {
        hash: 1,
        name: "m".into(),
        set_id: None,
    };
    assert!(s.vote_request(map, None).is_none());
}

#[test]
fn the_music_player_keeps_its_song_when_the_library_is_reordered_to_the_same_length() {
    let mut s = crate::layout_tests::state();
    let (a, b) = (
        crate::layout_tests::entry(0, 0, false),
        crate::layout_tests::entry(1, 0, false),
    );
    let (pa, pb) = (a.path.clone(), b.path.clone());
    s.use_library(vec![a, b], None);
    s.player = Some(Box::new(crate::player::Player::new(&s.library, Some(1))));
    let playing = |s: &State| {
        let i = s.player.as_ref().and_then(|p| p.list.entry()).unwrap();
        s.library[i].path.clone()
    };
    assert_eq!(playing(&s), pb);
    // Same length, other order: the index 1 now names the other map.
    s.use_library(
        vec![
            crate::layout_tests::entry(1, 0, false),
            crate::layout_tests::entry(0, 0, false),
        ],
        None,
    );
    assert_eq!(playing(&s), pb, "still the same song");
    assert_ne!(playing(&s), pa);
}

#[test]
fn check_library_deletion_keeps_music_player_track_and_playback_settings() {
    let mut s = crate::layout_tests::state();
    s.play_when_scanned = false;
    let a = crate::layout_tests::entry(0, 0, false);
    let b = crate::layout_tests::entry(1, 0, false);
    let c = crate::layout_tests::entry(2, 0, false);
    let pc = c.path.clone();
    s.use_library(vec![a, b, c], None);
    s.scene = SongSelect::scene();
    s.mp = None;

    let mut player = crate::player::Player::new(&s.library, Some(2));
    player.list.repeat = crate::playlist::Repeat::One;
    player.list.toggle_shuffle(42);
    player.set_custom_rate(1.15, false);
    player.daycore = true;
    s.player = Some(Box::new(player));

    // Delete map 0 (its path is /nonexistent/..., so !is_file() triggers the !gone.is_empty()
    // branch in check_library that takes self.library before calling use_library).
    let remaining = vec![
        crate::layout_tests::entry(1, 0, false),
        crate::layout_tests::entry(2, 0, false),
    ];
    s.check_library(remaining);
    assert_eq!(s.library.len(), 2, "check_library replaced the library");

    let p = s.player.as_ref().expect("player stays open");
    let i = p.list.entry().expect("active track");
    assert_eq!(
        s.library[i].path, pc,
        "player must keep its active song across check_library deletion refresh"
    );
    assert_eq!(p.list.repeat, crate::playlist::Repeat::One);
    assert!(p.list.is_shuffled());
    assert!(p.daycore);
}

#[test]
fn changing_library_from_settings_over_results_re_resolves_or_guards_entry() {
    let mut s = crate::layout_tests::state();
    let e0 = crate::layout_tests::entry(0, 0, false);
    let e1 = crate::layout_tests::entry(1, 0, false);
    let target_path = e1.path.clone();
    s.use_library(vec![e0, e1], None);
    let r = Box::new(ui::Results::new(
        "A - T [Oni]".into(),
        game::Stats::default(),
        game::Windows::new(5.0),
    ));
    s.scene = crate::screens::results::ResultsScreen::scene(r, 1, None);
    s.open_settings();

    // Case 1: map shifts from index 1 to index 0 after use_library.
    let shifted = crate::layout_tests::entry(1, 0, false);
    assert_eq!(shifted.path, target_path);
    s.use_library(vec![shifted], None);
    s.close_settings();
    let rs = s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
        .expect("back on results");
    assert_eq!(rs.entry, 0);

    // Case 2: map disappears from the library completely (e.g. switched Songs folder).
    s.open_settings();
    s.use_library(Vec::new(), None);
    s.close_settings();

    // Ctrl+R does not panic OOB.
    s.modifiers_ctrl = true;
    s.key(KeyCode::KeyR, false, None, Instant::now(), None);
    s.modifiers_ctrl = false;
    assert!(
        s.scene
            .screen_is::<crate::screens::results::ResultsScreen>()
    );

    // Esc does not panic OOB and returns cleanly to Song Select.
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(s.scene.is_select());
}
