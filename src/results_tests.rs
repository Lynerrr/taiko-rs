//! A score's results screen worked out in the background: from a local replay, and from an
//! online one fetched from a server.

use super::*;

/// A map of 400 notes (every third a kat), 150 ms apart, in a folder of its own.
fn map(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("taiko-rs-results-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    let mut text = String::from("osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:6\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n[HitObjects]\n");
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
    (0..400).map(|k| ((1000 + k * 150) as f64 + beatmap::OSU_TIMING_MS + late, if k % 3 == 0 { 0 } else { 1 })).collect()
}

fn wait(rx: &Receiver<ui::Detail>) -> ui::Detail {
    rx.recv_timeout(Duration::from_secs(60)).expect("the results' background work")
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
    assert!((stats.mean_error() - 12.0).abs() < 1.0, "{}", stats.mean_error());
    let b = d.breakdown.expect("pp");
    assert!(b.pp > 0.0 && (b.pp - b.if_ss).abs() < 1e-6, "an SS is its own SS: {b:?}");
    assert!(d.replay.is_none(), "a local replay is already there");

    // On the results screen: the counts stay, the replay adds the rest.
    let mut results = ui::Results::new("map".into(), game::Stats { great: 400, max_combo: 400, ..Default::default() }, game::Windows::new(5.0));
    results.partial = true;
    let (tx, rx) = mpsc::channel();
    tx.send(ui::Detail { stats: Some((stats, game::Windows::new(6.0))), breakdown: Some(b), replay: None }).unwrap();
    results.job = Some(rx);
    results.poll();
    assert!(!results.partial && results.job.is_none());
    assert_eq!(results.stats.errors.len(), 400);
    assert_eq!(results.pp, Some(b.pp));
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
        let map = net::MapInfo { hash: 1, artist: String::new(), title: String::new(), version: String::new(), rate: 1.0, from: None, set_id: None, mods: Vec::new(), multiverse: None, shapeshift: None, seed: None };
        let reply = online::Reply::Replay(Box::new(online::OnlineReplay { version: 2, map, inputs: sent }));
        let mut out = serde_json::to_string(&reply).unwrap();
        out.push('\n');
        reader.get_mut().write_all(out.as_bytes()).unwrap();
        id
    });
    let id = 4_000_000_000 + std::process::id() as u64;
    let _ = std::fs::remove_file(scores::online_replay_path(id).unwrap());
    let score = online::OnlineScore { player: "friend".into(), map_hash: 1, score: 1_000_000, accuracy: 1.0, max_combo: 400, great: 400, rate: 1.0, id, ..Default::default() };
    let play = pp::Play { great: 400, ok: 0, miss: 0, max_combo: 400, rate: 1.0, mods: Mods::NONE, scroll_speed: 1.0 };
    let d = wait(&results_job(path.clone(), play, None, Some((server, score, "map".into()))));
    assert_eq!(serve.join().unwrap(), id);
    let file = d.replay.expect("fetched").expect("saved");
    let r = scores::load_replay(&file).unwrap();
    assert_eq!((r.player.as_str(), r.inputs.len(), r.map_path.as_str()), ("friend", presses.len(), path.to_str().unwrap()));
    let (stats, _) = d.stats.expect("played through");
    assert!((stats.mean_error() + 8.0).abs() < 1.0, "{}", stats.mean_error());
    assert!(d.breakdown.is_some());
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn servers_without_replays_say_so() {
    let (dir, path) = map("old-server");
    let score = online::OnlineScore { player: "friend".into(), id: 0, ..Default::default() };
    let play = pp::Play { great: 400, ok: 0, miss: 0, max_combo: 400, rate: 1.0, mods: Mods::NONE, scroll_speed: 1.0 };
    let d = wait(&results_job(path, play, None, Some(("127.0.0.1:9".into(), score, "map".into()))));
    assert_eq!(d.replay, Some(Err("the server doesn't keep replays yet".into())));
    assert!(d.stats.is_none() && d.breakdown.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_score_link_opens_its_results_on_its_map() {
    use std::io::{BufRead, Write};
    let (dir, path) = map("link");
    let other = dir.join("other.osu");
    std::fs::write(&other, b"another difficulty").unwrap();
    let meta = |version: &str, set_id| beatmap::Meta { artist: "A".into(), title: "T".into(), version: version.into(), set_id, mode: 1, ..Default::default() };
    let library = vec![library::Entry::new(other, meta("Easy", Some(123))), library::Entry::new(path.clone(), meta("Oni", Some(123)))];
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
                online::Request::Score { id: 41 } => online::Reply::Score(Box::new(online::OnlineScore {
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
                })),
                online::Request::Score { id: 42 } => online::Reply::Score(Box::new(online::OnlineScore { map: "B - U [Hard]".into(), map_hash: 5, id: 42, ..Default::default() })),
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
    tx.send(library).unwrap();
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
    match &s.scene {
        Scene::Results { results, entry, .. } => {
            assert_eq!(*entry, 1);
            assert_eq!(results.opened.as_ref().map(|o| (o.player.as_str(), o.online)), Some(("friend", true)));
        }
        _ => panic!("not on the score's results"),
    }
    assert_eq!(s.select.selected(), Some(1));
    s.scene = Scene::Select;
    s.open_link(link::Link::Score(42));
    settle(&mut s);
    assert!(matches!(s.scene, Scene::Select));
    assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("you haven't got B - U [Hard]"));
    s.open_link(link::Link::Score(43));
    settle(&mut s);
    assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("can't open that score: nothing here"));
    let _ = std::fs::remove_dir_all(&dir);
}
