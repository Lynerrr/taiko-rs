//! Whole journeys through the client, as a player makes them with the keyboard: song select,
//! loading, playing with the drum keys, pausing, retrying, the results, the settings and back.

use super::harness::{self, *};
use super::maps::{self, Player};
use super::snapshot::assert_snapshot;
use crate::*;
use std::fmt::Write as _;

const FRAME: Duration = Duration::from_millis(4);

/// A client whose library is `charts`, on song select with the first one selected.
fn client(tag: &str, charts: Vec<maps::Chart>) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &charts);
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert_eq!(s.library.len(), charts.len(), "the library is taken in");
    assert_eq!(scene_name(&s), "Select");
    (s, songs, now)
}

/// From song select: Enter, then waits for the map to load.
pub(super) fn start_selected(s: &mut State, now: &mut Instant) {
    press(s, KeyCode::Enter, *now);
    assert!(
        matches!(scene_name(s), "Loading" | "Play"),
        "Enter on song select starts the map, not {}",
        scene_name(s)
    );
    assert!(
        wait_for(s, now, |s| scene_name(s) == "Play"),
        "the map loads"
    );
}

/// Plays the rest of the map in `Play` with `presses` (map time, drum key 0-3) on the keyboard,
/// frame by frame, until the scene changes.
pub(super) fn play_out(s: &mut State, now: &mut Instant, presses: &[(f64, u8)]) {
    let keys = s.cfg.keys;
    let mut next = 0;
    for _ in 0..200_000 {
        if harness::game(s).is_none() {
            return;
        }
        let horizon = *now + FRAME;
        // Presses due before the next frame, each at its own instant.
        while let Some(&(t, k)) = presses.get(next) {
            let at = instant_of(s, *now, t);
            if at > horizon {
                break;
            }
            press(s, keys[k as usize], at.max(*now));
            next += 1;
        }
        *now = horizon;
        frame(s, *now);
    }
    panic!("the play never ended");
}

fn results_stats(s: &State) -> game::Stats {
    match s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        Some(r) => r.results.stats.clone(),
        _ => panic!("not on the results screen but {}", scene_name(s)),
    }
}

fn line(st: &game::Stats) -> String {
    format!(
        "great {} ok {} miss {} max combo {} score {} rolls {} swells {} strong {}",
        st.great, st.ok, st.miss, st.max_combo, st.score, st.roll_ticks, st.swells, st.strong
    )
}

#[test]
fn tapping_intro_skip_jumps_without_hitting_the_drum() {
    let mut chart = maps::basics();
    chart.objects = vec![maps::Obj::Don(8_000), maps::Obj::Kat(9_000)];
    let (mut s, _songs, mut now) = client("touch-intro-skip", vec![chart]);
    start_selected(&mut s, &mut now);
    s.touch_used = true;
    frame(&mut s, now);
    let [x, y, w, h] = s
        .hits
        .iter()
        .find(|area| area.hit == crate::hit::Hit::SkipIntro)
        .expect("a long intro offers a skip button on touch")
        .rect;
    let scale = f64::from(s.window_h / render::VIRTUAL_HEIGHT);
    s.touch(
        1,
        TouchPhase::Started,
        f64::from(x + w / 2.0) * scale,
        f64::from(y + h / 2.0) * scale,
        now,
        None,
    );
    assert_eq!(s.touch_flash, [None; 4]);
    frame(&mut s, now + FRAME);
    assert!(!harness::game(&s).unwrap().can_skip());
    assert!(
        !s.hits
            .iter()
            .any(|area| area.hit == crate::hit::Hit::SkipIntro)
    );
}

#[test]
fn picture_copies_made_ahead_wait_while_a_map_is_played() {
    // Song select's cards make copies of every set's picture in the background, and its hasher
    // hashes maps; decoding/hashing takes cores, which a low-end CPU can't spare mid-song.
    let (mut s, _songs, mut now) = client("prewarm-hold", vec![maps::rolls_and_swells()]);
    let held = |s: &State| (s.pictures.held(), s.card_pictures.held(), s.hasher.held());
    assert_eq!(held(&s), (false, false, false), "song select: they go on");
    start_selected(&mut s, &mut now);
    now += FRAME;
    frame(&mut s, now);
    assert_eq!(held(&s), (true, true, true), "playing: they wait");
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"),
        "the results come after the map"
    );
    // The hold follows the scene a frame behind (set as a frame starts).
    now += FRAME;
    frame(&mut s, now);
    assert_eq!(held(&s), (false, false, false), "the song over: they go on");
}

#[test]
fn a_map_is_played_with_the_keyboard_from_song_select_to_the_results_and_back() {
    let (mut s, _songs, mut now) = client("journey", vec![maps::rolls_and_swells()]);
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"),
        "the results come after the map"
    );
    let st = results_stats(&s);
    assert_eq!((st.ok, st.miss), (0, 0), "{}", line(&st));
    assert_eq!(
        st.great as usize,
        map.notes
            .iter()
            .filter(|n| n.kind == beatmap::Kind::Don || n.kind == beatmap::Kind::Kat)
            .count()
    );
    assert_snapshot("flows-keyboard-play-perfect", &line(&st));
    // Back to song select.
    press(&mut s, KeyCode::Escape, now);
    run_until(&mut s, &mut now, FRAME, 50, |s| scene_name(s) == "Select");
    assert_eq!(scene_name(&s), "Select");
}

#[test]
fn a_play_on_a_downloaded_custom_map_reports_nothing_of_its_own() {
    // A set from the site keeps its number in its folder, uploaded by this player or not: only
    // the server can tell whose it is, from the play it's sent.
    let (mut s, songs, mut now) = client("custom-not-own", vec![maps::basics()]);
    std::fs::write(
        songs.paths[0]
            .parent()
            .unwrap()
            .join(crate::custom::SIDECAR),
        "{\"id\": 12}\n",
    )
    .unwrap();
    s.cfg.online_token = "t".into();
    crate::ura::take_reported();
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"));
    // Tests play muted, which is all the game itself has to tell.
    assert_eq!(crate::ura::take_reported(), vec![crate::ura::Event::Deaf]);
}

#[test]
fn keyboard_play_judges_like_the_replay_of_it() {
    // The same presses, live on the keyboard and played back as a replay: the same results.
    for (chart, player) in [
        (maps::basics(), Player::Jitter(50.0)),
        (maps::big_notes(), Player::WrongColour(3)),
        (maps::sv_and_kiai(), Player::Skips(4)),
    ] {
        let name = chart.name;
        let beatmap = chart.beatmap();
        let presses = maps::presses(&beatmap, player);
        let replayed = super::gameplay::play(&beatmap, player, &[]);
        let (mut s, _songs, mut now) = client(&format!("vs-replay-{name}"), vec![chart]);
        start_selected(&mut s, &mut now);
        play_out(&mut s, &mut now, &presses);
        assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"));
        let live = results_stats(&s);
        assert_eq!(
            line(&live),
            line(replayed.stats()),
            "{name}: live vs replay"
        );
    }
}

#[test]
fn f12_takes_a_screenshot_over_the_mod_select_and_its_panels() {
    let (mut s, _songs, now) = client("shot-over-mods", vec![maps::rolls_and_swells()]);
    press(&mut s, KeyCode::F1, now);
    assert!(s.mod_select.is_some(), "F1 opens the mod select");
    press(&mut s, KeyCode::F12, now);
    assert!(s.screenshot_due, "F12 over the mod select");
    assert!(s.mod_select.is_some(), "and the mod select stays open");

    s.screenshot_due = false;
    s.open_customization(crate::mods::Mod::OdOverride);
    assert!(s.customization_tab().is_some());
    press(&mut s, KeyCode::F12, now);
    assert!(s.screenshot_due, "F12 over a mod's customization");
    assert!(s.customization_tab().is_some(), "which stays open");
    s.od_setup = None;

    s.screenshot_due = false;
    press(&mut s, KeyCode::KeyS, now);
    assert!(s.preset_naming.is_some(), "S names a preset");
    press(&mut s, KeyCode::F12, now);
    assert!(s.screenshot_due, "F12 while a preset is being named");
    assert!(s.preset_naming.is_some(), "which goes on");
}

#[test]
fn pausing_resuming_retrying_and_quitting() {
    let (mut s, _songs, mut now) = client("pause", vec![maps::basics()]);
    let mut log = String::new();
    start_selected(&mut s, &mut now);
    run_until(&mut s, &mut now, FRAME, 300, |_| false);
    // Escape pauses: the menu shows and the clock stops.
    press(&mut s, KeyCode::Escape, now);
    let c = frame(&mut s, now);
    let paused = harness::game(&s).unwrap().paused();
    let _ = writeln!(log, "after Escape: {} paused {:?}", scene_name(&s), paused);
    assert!(paused.is_some(), "Escape pauses");
    let t = harness::game(&s).unwrap().time();
    run_until(&mut s, &mut now, FRAME, 200, |_| false);
    assert_eq!(
        harness::game(&s).unwrap().time(),
        t,
        "paused, the map time stands still"
    );
    let _ = writeln!(log, "pause menu says: {:?}", pause_menu(&c));
    // Escape again resumes (after a countdown).
    press(&mut s, KeyCode::Escape, now);
    run_until(&mut s, &mut now, FRAME, 2000, |s| {
        harness::game(s).is_some_and(|g| g.paused().is_none() && g.resuming().is_none())
    });
    let g = harness::game(&s).unwrap();
    let _ = writeln!(
        log,
        "after the second Escape: paused {:?} resuming {}",
        g.paused(),
        g.resuming().is_some()
    );
    run_until(&mut s, &mut now, FRAME, 100, |_| false);
    assert!(harness::game(&s).unwrap().time() > t, "playing again");
    // Retry from the pause menu: the map starts over.
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    let _ = writeln!(log, "retry: {}", scene_name(&s));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let restarted = harness::game(&s).unwrap();
    assert_eq!(
        restarted.stats().great + restarted.stats().miss,
        0,
        "a fresh play"
    );
    // Quit to song select from the pause menu (its last item).
    press(&mut s, KeyCode::Escape, now);
    for _ in 1..game::PAUSE_ITEMS {
        press(&mut s, KeyCode::ArrowDown, now);
    }
    press(&mut s, KeyCode::Enter, now);
    run_until(&mut s, &mut now, FRAME, 50, |s| scene_name(s) == "Select");
    let _ = writeln!(log, "quit: {}", scene_name(&s));
    assert_eq!(scene_name(&s), "Select");
    assert_snapshot("flows-pause-retry-quit", &log);
}

/// The pause menu's items as drawn.
fn pause_menu(c: &crate::hit::Canvas) -> Vec<&'static str> {
    let mut v = vec![
        "Continue",
        "Retry",
        "Playfield size",
        "Settings",
        "Quit to song select",
    ];
    assert_eq!(v.len(), game::PAUSE_ITEMS);
    v.retain(|l| shows(c, l));
    v
}

#[test]
fn a_failed_play_ends_on_the_results_as_failed() {
    let mut cfg = harness::config();
    cfg.mods = Mods::parse("SD");
    let songs = Songs::new("failed", &[maps::basics()]);
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Skips(10)));
    let failed_early = scene_name(&s);
    let _ = wait_for(&mut s, &mut now, |s| scene_name(s) != "Play");
    let mut log = format!("after the miss: {failed_early} -> {}\n", scene_name(&s));
    if let Some(r) = s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        let _ = writeln!(
            log,
            "failed {} {}",
            r.results.failed,
            line(&r.results.stats)
        );
    }
    assert_snapshot("flows-sudden-death-fail", &log);
}

#[test]
fn settings_open_change_and_close_with_the_keyboard() {
    let (mut s, _songs, mut now) = client("settings", vec![maps::basics()]);
    let mut log = String::new();
    s.open_settings();
    frame(&mut s, now);
    let _ = writeln!(
        log,
        "open: {} cursor {}",
        scene_name(&s),
        s.settings_ui.cursor
    );
    // Walking down the list moves through the sections.
    let mut sections = Vec::new();
    for _ in 0..settings::ITEMS.len() + 2 {
        let sec = s.settings_ui.section();
        if sections.last() != Some(&sec) {
            sections.push(sec);
        }
        press(&mut s, KeyCode::ArrowDown, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let names: Vec<&str> = sections
        .iter()
        .map(|&i| settings::section_name(i))
        .collect();
    let _ = writeln!(log, "sections walked: {}", names.join(", "));
    assert_eq!(
        sections.len(),
        settings::SECTIONS.len(),
        "every section is reachable"
    );
    // Escape closes the settings and goes back where they were opened from.
    press(&mut s, KeyCode::Escape, now);
    frame(&mut s, now);
    let _ = writeln!(log, "after Escape: {}", scene_name(&s));
    assert_eq!(scene_name(&s), "Select");
    assert_snapshot("flows-settings-keyboard", &log);
}

#[test]
fn song_select_searches_sorts_and_moves_through_the_list() {
    let charts = maps::all();
    let n = charts.len();
    let (mut s, _songs, mut now) = client("select", charts);
    let mut log = String::new();
    let selected = |s: &State| {
        s.library[s.select.selected().unwrap_or(0)]
            .meta
            .version
            .clone()
    };
    let _ = writeln!(log, "first: {}", selected(&s));
    for _ in 0..n + 1 {
        press(&mut s, KeyCode::ArrowDown, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "down: {}", selected(&s));
    }
    press(&mut s, KeyCode::ArrowUp, now);
    frame(&mut s, now);
    let _ = writeln!(log, "up: {}", selected(&s));
    // Typing searches.
    s.select.searching = true;
    type_text(&mut s, "stream", now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let _ = writeln!(log, "search {:?}: {}", s.select.filter, selected(&s));
    assert!(selected(&s).contains("stream"));
    assert_snapshot("flows-song-select", &log);
}

#[test]
fn the_top_tabs_lead_to_every_part_of_the_game() {
    let (mut s, _songs, mut now) = client("tabs", vec![maps::basics()]);
    let mut log = String::new();
    for tab in 0..3 {
        s.click(crate::hit::Hit::Tab(tab), None);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "tab {tab}: {}", scene_name(&s));
        press(&mut s, KeyCode::Escape, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "  Escape: {}", scene_name(&s));
        s.scene = SongSelect::scene();
    }
    assert_snapshot("flows-top-tabs", &log);
}

#[test]
fn selecting_simplified_chinese_localizes_settings_and_formatted_text() {
    let (mut s, _songs, now) = client("simplified-chinese", vec![maps::basics()]);
    s.cfg.language = crate::lang::Lang::ZhCn;
    s.open_settings();
    let canvas = frame(&mut s, now);
    assert!(canvas.texts.iter().any(|(_, text)| text == "设置"));
    let n = 2;
    let name = "Don";
    assert_eq!(
        tr!("{n} local plays now show as {name}", "{n} 曲: {name}", "{n} игры: {name}"; [] [n, name]),
        "2 条本地成绩现已显示为 Don"
    );
}

// ---------------------------------------------------------------------------------------------
// The terms.

/// An account server that passes on each request it gets: a sign-up's first step is answered
/// with a mailed code, a saved login with the account, and the rest are hung up on.
fn terms_server() -> (String, std::sync::mpsc::Receiver<online::Request>) {
    terms_server_keeping(None)
}

/// `terms_server`, keeping the agreement to the terms when `kept` is the version it starts
/// with: it lists `online::TERMS_FEATURE` and answers `AcceptTerms` with the account.
fn terms_server_keeping(kept: Option<u32>) -> (String, std::sync::mpsc::Receiver<online::Request>) {
    use crate::online::{Envelope, Reply, Request};
    let kept = std::sync::Arc::new(std::sync::Mutex::new(kept));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = std::sync::mpsc::channel();
    let account = |token: &String, kept: Option<u32>| Reply::Account {
        name: "me".into(),
        token: token.clone(),
        nominator: false,
        admin: false,
        email: Some(true),
        features: kept
            .map(|_| vec![online::TERMS_FEATURE.into()])
            .unwrap_or_default(),
        terms: kept.unwrap_or(0),
        server: None,
    };
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let Ok(line) = online::read_line(&mut reader) else {
                continue;
            };
            let Ok(env) = serde_json::from_str::<Envelope>(&line) else {
                continue;
            };
            let reply = match &env.request {
                Request::Register { email, .. } => {
                    Some(Reply::CodeSent(email.clone().unwrap_or_default()))
                }
                Request::WhoAmI { token } => Some(account(token, *kept.lock().unwrap())),
                Request::AcceptTerms { token, version } if kept.lock().unwrap().is_some() => {
                    *kept.lock().unwrap() = Some(*version);
                    Some(account(token, Some(*version)))
                }
                _ => None,
            };
            let _ = tx.send(env.request);
            if let Some(r) = reply {
                let _ = online::write_line(&mut out, &r);
            }
        }
    });
    (addr, rx)
}

/// The next request of `kind` the server got, within a few seconds.
fn next_request(
    rx: &std::sync::mpsc::Receiver<online::Request>,
    want: impl Fn(&online::Request) -> bool,
) -> Option<online::Request> {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if let Ok(r) = rx.recv_timeout(Duration::from_millis(50))
            && want(&r)
        {
            return Some(r);
        }
    }
    None
}

#[test]
fn signing_up_shows_the_terms_and_goes_on_once_they_are_agreed_to() {
    use crate::multiplayer::{CodeFor, Form, Row};
    let (server, rx) = terms_server();
    let is_register = |r: &online::Request| matches!(r, online::Request::Register { .. });
    let mut cfg = harness::config();
    cfg.online_server = server;
    let mut s = state_with(cfg, Vec::new());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.scene = crate::screens::mp_menu::MpMenu::scene();
    s.mp_menu.open(Form::Register);
    s.mp_menu.user = "newplayer".into();
    s.mp_menu.password = "secret1".into();
    s.mp_menu.email = "new@example.com".into();
    s.mp_menu.select(Row::Register);

    // SEND CODE: the terms come first, and nothing goes to the server.
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(
        s.terms.as_ref().map(|t| t.purpose),
        Some(terms::Purpose::Register)
    );
    assert!(s.mp_menu.account_request.is_none(), "no sign-up sent yet");
    now += FRAME;
    let c = frame(&mut s, now);
    assert!(
        shows(&c, "taiko.rs: terms and your data"),
        "the terms are drawn"
    );
    assert!(shows(&c, "I agree") && shows(&c, "Not now"));

    // Enter still held from the form doesn't agree.
    s.key(KeyCode::Enter, true, None, now, None);
    assert!(s.terms.is_some(), "a held Enter doesn't answer");

    // Not now: still no sign-up, and the way back is said.
    press(&mut s, KeyCode::Escape, now);
    assert!(s.terms.is_none());
    assert_eq!(s.mp_menu.form, Form::Register, "the form stays");
    assert!(s.mp_status.contains("SEND CODE shows them again"));
    assert_eq!(s.cfg.terms_version, 0);
    assert!(s.mp_menu.account_request.is_none());

    // Again, and agreed: remembered, and the code is asked for at once.
    press(&mut s, KeyCode::Enter, now);
    assert!(s.terms.is_some(), "SEND CODE shows them again");
    press(&mut s, KeyCode::Enter, now);
    assert!(s.terms.is_none());
    assert_eq!(s.cfg.terms_version, terms::VERSION);
    let sent = next_request(&rx, is_register).expect("the sign-up went out");
    assert!(matches!(
        sent,
        online::Request::Register { ref name, code: None, .. } if name == "newplayer"
    ));
    assert!(
        wait_for(&mut s, &mut now, |s| s.mp_menu.code_for
            == Some(CodeFor::Register)),
        "the code was sent"
    );

    // Making the account with the code doesn't ask again.
    s.mp_menu.code = "123456".into();
    s.mp_menu.select(Row::Register);
    press(&mut s, KeyCode::Enter, now);
    assert!(s.terms.is_none(), "agreed already");
    assert!(next_request(&rx, is_register).is_some());
}

#[test]
fn an_account_from_before_the_terms_agrees_once_or_is_logged_out() {
    let start = |version: u32| {
        let (server, _rx) = terms_server();
        let mut cfg = harness::config();
        cfg.online_server = server;
        cfg.online_user = "me".into();
        cfg.online_token = "t".into();
        cfg.terms_version = version;
        let mut s = state_with(cfg, Vec::new());
        let mut now = Instant::now();
        ready(&mut s, &mut now);
        assert!(
            wait_for(&mut s, &mut now, |s| s.mp_menu.account_request.is_none()
                && s.mp_menu.online.is_some()),
            "the saved login is checked"
        );
        (s, now)
    };

    // Not agreed: logged out, the reason said.
    let (mut s, now) = start(0);
    assert_eq!(
        s.terms.as_ref().map(|t| t.purpose),
        Some(terms::Purpose::Account)
    );
    let c = frame(&mut s, now);
    assert!(shows(&c, "Log out"), "saying no logs out, and says so");
    press(&mut s, KeyCode::Escape, now);
    assert!(s.terms.is_none());
    assert!(s.mp_menu.account.is_none(), "logged out");
    assert!(s.cfg.online_token.is_empty(), "the saved login is gone");
    assert!(s.mp_status.contains("playing online needs the terms"));
    assert_eq!(s.cfg.terms_version, 0);

    // Agreed: stays logged in, and it's remembered.
    let (mut s, _) = start(0);
    assert!(s.terms.is_some());
    press(&mut s, KeyCode::Enter, Instant::now());
    assert!(s.terms.is_none());
    assert_eq!(s.cfg.terms_version, terms::VERSION);
    assert_eq!(s.mp_menu.account.as_deref(), Some("me"));
    assert_eq!(s.cfg.online_token, "t");

    // This version agreed to already: not asked again.
    let (s, _) = start(terms::VERSION);
    assert_eq!(s.mp_menu.account.as_deref(), Some("me"));
    assert!(s.terms.is_none(), "asked once per version");
}

#[test]
fn an_account_that_agreed_to_version_1_is_asked_again_for_version_2() {
    let (server, _rx) = terms_server();
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = 1;
    let mut s = state_with(cfg, Vec::new());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(
        wait_for(&mut s, &mut now, |s| s.mp_menu.account_request.is_none()
            && s.mp_menu.online.is_some()),
        "the saved login is checked"
    );
    assert_eq!(terms::VERSION, 2);
    assert_eq!(
        s.terms.as_ref().map(|t| t.purpose),
        Some(terms::Purpose::Account),
        "version 1 isn't enough any more"
    );
    press(&mut s, KeyCode::Enter, now);
    assert!(s.terms.is_none());
    assert_eq!(s.cfg.terms_version, 2);
    assert_eq!(s.mp_menu.account.as_deref(), Some("me"));
}

#[test]
fn every_translation_of_the_terms_says_players_answer_for_what_they_share() {
    for (text, heading, affiliation) in [
        (
            terms::ALL[0],
            "## What you share",
            "isn't affiliated with ppy (osu!), Bandai Namco (Taiko no Tatsujin) or OpenTaiko",
        ),
        (
            terms::ALL[1],
            "## 共有するもの",
            "OpenTaiko とは関係ありません",
        ),
        (
            terms::ALL[2],
            "## Что ты отправляешь",
            "не связана с ppy (osu!), Bandai Namco (Taiko no Tatsujin) и OpenTaiko",
        ),
    ] {
        assert!(
            text.lines().nth(1).is_some_and(|l| l.contains('2')),
            "version 2"
        );
        let section = text.split(heading).nth(1).expect("the section is there");
        let section = section.split("\n## ").next().unwrap();
        assert_eq!(section.lines().filter(|l| l.starts_with("- ")).count(), 4);
        assert!(
            section.contains("postmaster@auneu.space"),
            "where rights holders write"
        );
        assert!(text.contains(affiliation));
    }
}

#[test]
fn a_server_that_keeps_the_terms_hears_of_the_agreement_and_it_counts_on_every_install() {
    let is_accept = |r: &online::Request| matches!(r, online::Request::AcceptTerms { .. });
    let start = |kept: Option<u32>, version: u32| {
        let (server, rx) = terms_server_keeping(kept);
        let mut cfg = harness::config();
        cfg.online_server = server;
        cfg.online_user = "me".into();
        cfg.online_token = "t".into();
        cfg.terms_version = version;
        let mut s = state_with(cfg, Vec::new());
        let mut now = Instant::now();
        ready(&mut s, &mut now);
        assert!(
            wait_for(&mut s, &mut now, |s| s.mp_menu.account_request.is_none()
                && s.mp_menu.online.is_some()),
            "the saved login is checked"
        );
        (s, rx)
    };

    // Agreed in the game: the server is told which version.
    let (mut s, rx) = start(Some(0), 0);
    assert!(
        s.terms.is_some(),
        "asked, as the server has no agreement either"
    );
    assert!(
        next_request(&rx, is_accept).is_none(),
        "nothing before an answer"
    );
    press(&mut s, KeyCode::Enter, Instant::now());
    assert!(matches!(
        next_request(&rx, is_accept),
        Some(online::Request::AcceptTerms { ref token, version }) if token == "t" && version == terms::VERSION
    ));

    // Agreed on another install: not asked again here, and remembered.
    let (s, rx) = start(Some(terms::VERSION), 0);
    assert!(s.terms.is_none(), "the server's agreement counts");
    assert_eq!(s.cfg.terms_version, terms::VERSION);
    assert!(next_request(&rx, is_accept).is_none(), "nothing to tell it");

    // Agreed here before the server kept it: told at the next login, without asking.
    let (s, rx) = start(Some(0), terms::VERSION);
    assert!(s.terms.is_none());
    assert!(next_request(&rx, is_accept).is_some());

    // A server that doesn't keep it (the legacy one) never gets the request.
    let (mut s, rx) = start(None, 0);
    press(&mut s, KeyCode::Enter, Instant::now());
    assert_eq!(s.cfg.terms_version, terms::VERSION);
    assert!(next_request(&rx, is_accept).is_none());
}

#[test]
fn the_terms_hold_up_in_every_language() {
    let shape = |text: &str| {
        terms::layout(text, 10_000.0)
            .iter()
            .map(|l| l.kind)
            .collect::<Vec<_>>()
    };
    let english = shape(terms::ALL[0]);
    assert_eq!(english.first(), Some(&terms::Kind::Title));
    for text in terms::ALL {
        assert_eq!(
            shape(text),
            english,
            "every translation has the same headings and points:\n{text}"
        );
        // Line by line, as drawn: a Windows checkout ends them with \r\n.
        let undrawable: String = text
            .lines()
            .flat_map(str::chars)
            .filter(|&c| !crate::font::drawable(c))
            .collect();
        assert!(
            undrawable.is_empty(),
            "every letter can be drawn: {undrawable:?}"
        );
        assert!(text.contains("postmaster@auneu.space"), "where to ask");
        assert!(
            text.contains(&format!(" {}", terms::VERSION)),
            "the version is in the text"
        );
    }
}

#[test]
fn the_terms_break_lines_between_words_and_anywhere_in_japanese() {
    let width = 20;
    let room = width * crate::font::advance(' ');
    let fits = |lines: &[String]| {
        lines
            .iter()
            .all(|l| l.chars().map(crate::font::advance).sum::<usize>() <= room)
    };
    let en = terms::wrap("Your name, avatar, profile and scores are public.", width);
    assert!(fits(&en), "{en:?}");
    assert!(
        en.iter().all(|l| !l.starts_with(' ') && !l.ends_with(' ')),
        "{en:?}"
    );
    assert_eq!(
        en.join(" "),
        "Your name, avatar, profile and scores are public.",
        "only spaces break English"
    );
    let ja = terms::wrap(
        "taiko.rs は小さなチームが作っている無料のゲームです。アカウントはオンラインで遊ぶときだけ必要です。",
        width,
    );
    assert!(fits(&ja), "{ja:?}");
    assert!(
        ja[0].starts_with("taiko.rs は"),
        "the Latin word stays on the line with the Japanese after it: {ja:?}"
    );
    assert!(
        ja.iter().skip(1).all(|l| !l.starts_with(['、', '。'])),
        "no line starts with a closing mark: {ja:?}"
    );
    assert_eq!(
        ja.concat().replace(' ', ""),
        "taiko.rsは小さなチームが作っている無料のゲームです。アカウントはオンラインで遊ぶときだけ必要です。"
    );
}

/// A close request as Alt+F4 sends it, with the key's modifier held.
fn alt_f4(s: &mut State, alt: bool) {
    s.modifiers_alt = alt;
    s.close_requested();
    s.modifiers_alt = false;
}

#[test]
fn alt_f4_leaves_the_map_only_when_asked_to() {
    // On: Alt+F4 in a map quits it to song select and the game keeps running.
    let (mut s, _songs, mut now) = client("altf4-on", vec![maps::basics()]);
    s.cfg.alt_f4_leaves_map = true;
    start_selected(&mut s, &mut now);
    run_until(&mut s, &mut now, FRAME, 300, |_| false);
    assert_eq!(scene_name(&s), "Play");
    // The window's X button, no Alt: asks to close as ever.
    alt_f4(&mut s, false);
    assert!(s.quit_asked && !s.quit, "X still asks to close");
    s.quit_asked = false;
    assert_eq!(scene_name(&s), "Play");
    alt_f4(&mut s, true);
    run_until(&mut s, &mut now, FRAME, 50, |s| scene_name(s) == "Select");
    assert_eq!(scene_name(&s), "Select");
    assert!(!s.quit && !s.quit_asked, "the game keeps running");
    // Outside a map the same request closes the game as before.
    alt_f4(&mut s, true);
    assert!(s.quit_asked, "song select: asks to close");

    // Off: Alt+F4 in a map asks to close as before.
    let (mut s, _songs, mut now) = client("altf4-off", vec![maps::basics()]);
    start_selected(&mut s, &mut now);
    run_until(&mut s, &mut now, FRAME, 300, |_| false);
    alt_f4(&mut s, true);
    assert!(s.quit_asked, "off: asks to close");
    assert_eq!(scene_name(&s), "Play");
}

/// What `updating_server` saw: the key of every play sent to it, the plays it stored (once per
/// key) and how many social links it took.
#[derive(Default)]
struct Updating {
    sent: Vec<Option<String>>,
    /// The pp each play was sent with.
    pps: Vec<Option<f64>>,
    stored: std::collections::HashSet<String>,
    links: usize,
    /// The pp total its profile of the player says, and how many times it was asked for it.
    profile_pp: f64,
    profiles: usize,
}

/// What `score_server` does with the first play sent to it.
#[derive(Clone, Copy)]
enum FirstPlay {
    /// Stores it and closes the connection without an answer, then moves the social link
    /// (`Down::Moving`): the server is being updated.
    Vanishes,
    /// Answers `Reply::Error` with this text and stores nothing.
    Refused(&'static str),
    /// Stores it and answers, like every later play.
    Kept,
}

/// A server being updated as a play ends (`FirstPlay::Vanishes`).
fn updating_server() -> (String, std::sync::Arc<std::sync::Mutex<Updating>>) {
    score_server(FirstPlay::Vanishes)
}

/// A server that lists `SUBMIT_KEY_FEATURE` and `MOVING_FEATURE` and does `first_play` with the
/// first play sent. Every later play is stored once per key and answered.
fn score_server(first_play: FirstPlay) -> (String, std::sync::Arc<std::sync::Mutex<Updating>>) {
    use crate::online::{Down, Envelope, Reply, Request};
    use std::sync::{Arc, Mutex};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Updating::default()));
    let first_link: Arc<Mutex<Option<std::net::TcpStream>>> = Arc::default();
    let shared = seen.clone();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let seen = shared.clone();
            let first_link = first_link.clone();
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
                        online::MOVING_FEATURE.into(),
                        online::SUBMIT_KEY_FEATURE.into(),
                    ],
                    terms: 0,
                    server: None,
                };
                match env.request {
                    Request::Connect { .. } => {
                        let _ = online::write_line(&mut out, &account);
                        let first = {
                            let mut seen = seen.lock().unwrap();
                            seen.links += 1;
                            seen.links == 1
                        };
                        if first {
                            *first_link.lock().unwrap() = out.try_clone().ok();
                        }
                        // Held until the game or the server closes it.
                        while online::read_line(&mut reader).is_ok() {}
                    }
                    Request::WhoAmI { .. } => {
                        let _ = online::write_line(&mut out, &account);
                    }
                    Request::Profile { name } => {
                        let pp = {
                            let mut seen = seen.lock().unwrap();
                            seen.profiles += 1;
                            seen.profile_pp
                        };
                        let profile = online::Profile {
                            name,
                            pp,
                            rank: Some(7),
                            ..Default::default()
                        };
                        let _ = online::write_line(&mut out, &Reply::Profile(Box::new(profile)));
                    }
                    Request::SubmitScore { play, .. } => {
                        let key = play.operation_key.clone();
                        let first = {
                            let mut seen = seen.lock().unwrap();
                            // Answered with 100 pp, settled at 140 once its replay is played
                            // back (the real server does that just after answering).
                            seen.profile_pp = 140.0;
                            seen.sent.push(key.clone());
                            seen.pps.push(play.pp);
                            let first = seen.sent.len() == 1;
                            if let Some(k) = &key
                                && !(first && matches!(first_play, FirstPlay::Refused(_)))
                            {
                                seen.stored.insert(k.clone());
                            }
                            first
                        };
                        match first_play {
                            // Stored, then gone before answering: the old copy stops and
                            // moves the game to the new one.
                            FirstPlay::Vanishes if first => {
                                let _ = out.shutdown(std::net::Shutdown::Both);
                                if let Some(mut link) = first_link.lock().unwrap().take() {
                                    let _ = online::write_line(&mut link, &Down::Moving);
                                    let _ = link.shutdown(std::net::Shutdown::Both);
                                }
                                return;
                            }
                            FirstPlay::Refused(error) if first => {
                                let _ = online::write_line(&mut out, &Reply::Error(error.into()));
                                return;
                            }
                            _ => {}
                        }
                        let _ = online::write_line(
                            &mut out,
                            &Reply::Submitted {
                                relax: false,
                                map_rank: Some(1),
                                pp: 100.0,
                                rank: Some(7),
                                medals: Vec::new(),
                            },
                        );
                    }
                    _ => {
                        let _ = online::write_line(&mut out, &Reply::Error("bad request".into()));
                    }
                }
            });
        }
    });
    (addr, seen)
}

/// The server is updated as a play ends and never answers it: the game keeps the play and,
/// once its social link is on the updated server, sends it again with the same key. The
/// server stores it once, the player hears it counted, and nothing is left to send.
#[test]
fn a_score_the_server_never_answered_goes_again_once_it_is_back_and_counts_once() {
    let (server, seen) = updating_server();
    let songs = Songs::new("score-resend", &[maps::basics()]);
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(
        wait_for(&mut s, &mut now, |s| s.social.connected && s.submit_keys),
        "the social link connects to a server that keeps plays once per key"
    );
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));

    // Takes in the social link and the scores' answers until `done`, keeping what they toast
    // (a frame on the made-up clock, ahead of the real one, would drop a toast as it draws).
    let mut toasts: Vec<String> = Vec::new();
    let mut noting = |s: &mut State, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(30);
        while !done(s) && Instant::now() < until {
            s.poll_social();
            s.poll_online_scores();
            if let Some(t) = s.toast.take() {
                toasts.push(t.text);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done(s)
    };
    assert!(
        noting(&mut s, &|s| scene_name(s) == "Results"),
        "the results come after the map"
    );
    let twice = |s: &State| seen.lock().unwrap().sent.len() >= 2 && s.submits.is_empty();
    assert!(
        noting(&mut s, &twice),
        "the play goes again: {:?}",
        seen.lock().unwrap().sent
    );
    // A while longer: it doesn't go a third time.
    let started = Instant::now();
    noting(&mut s, &|_| started.elapsed() > Duration::from_secs(1));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.sent.len(), 2, "sent twice: {:?}", seen.sent);
    assert!(seen.sent[0].is_some(), "with a key");
    assert_eq!(seen.sent[0], seen.sent[1], "the same key both times");
    assert_eq!(seen.stored.len(), 1, "stored once");
    assert!(
        seen.links >= 2,
        "the social link moved to the updated server"
    );
    assert!(s.score_outbox.is_empty(), "nothing left to send");
    assert!(
        toasts
            .iter()
            .any(|t| t.starts_with("online: score not sent yet")),
        "the player hears the score will go again: {toasts:?}"
    );
    assert!(
        toasts
            .iter()
            .any(|t| t == "online: an earlier score #1 on this map  |  100 pp, #7 overall"),
        "and that it counted: {toasts:?}"
    );
}

/// Logs in to `server`, plays the map out without a miss and waits for the results.
fn play_to_server(server: String, songs: &Songs) -> State {
    play_to_server_with(server, songs, crate::mods::Mods::NONE)
}

/// `play_to_server` with `mods` on.
fn play_to_server_with(server: String, songs: &Songs, mods: crate::mods::Mods) -> State {
    let mut cfg = harness::config();
    cfg.mods = mods;
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(
        wait_for(&mut s, &mut now, |s| s.social.connected && s.submit_keys),
        "the social link connects to a server that keeps plays once per key"
    );
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    s
}

/// Takes in the social link and the scores' answers until `done` (30 s at most), keeping what
/// they toast (a frame on the made-up clock, ahead of the real one, would drop a toast as it
/// draws).
fn note_answers(s: &mut State, toasts: &mut Vec<String>, done: &dyn Fn(&State) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(30);
    while !done(s) && Instant::now() < until {
        s.poll_social();
        s.poll_online_scores();
        if let Some(t) = s.toast.take() {
            toasts.push(t.text);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    done(s)
}

/// The server's database is briefly out of reach as a play ends (as during a deploy), and it
/// answers "temporarily unavailable": the game keeps the play and sends it again with the same
/// key. The server stores it once, the player hears it counted, and nothing is left to send.
#[test]
fn a_score_the_server_could_not_take_just_now_goes_again_and_counts_once() {
    let (server, seen) = score_server(FirstPlay::Refused("progression is temporarily unavailable"));
    let songs = Songs::new("score-try-later", &[maps::basics()]);
    let mut s = play_to_server(server, &songs);
    let mut toasts = Vec::new();
    assert!(
        note_answers(&mut s, &mut toasts, &|s| scene_name(s) == "Results"),
        "the results come after the map"
    );
    let twice = |s: &State| seen.lock().unwrap().sent.len() >= 2 && s.submits.is_empty();
    assert!(
        note_answers(&mut s, &mut toasts, &twice),
        "the play goes again: {:?}",
        seen.lock().unwrap().sent
    );
    // A while longer: it doesn't go a third time.
    let started = Instant::now();
    note_answers(&mut s, &mut toasts, &|_| {
        started.elapsed() > Duration::from_secs(1)
    });
    let seen = seen.lock().unwrap();
    assert_eq!(seen.sent.len(), 2, "sent twice: {:?}", seen.sent);
    assert!(seen.sent[0].is_some(), "with a key");
    assert_eq!(seen.sent[0], seen.sent[1], "the same key both times");
    assert_eq!(seen.stored.len(), 1, "stored once");
    assert!(s.score_outbox.is_empty(), "nothing left to send");
    assert!(
        toasts.iter().any(|t| t
            == "online: score not sent yet (progression is temporarily unavailable), \
                it goes again once the server is back"),
        "the player hears the score will go again: {toasts:?}"
    );
    assert!(
        toasts
            .iter()
            .any(|t| t == "online: an earlier score #1 on this map  |  100 pp, #7 overall"),
        "and that it counted: {toasts:?}"
    );
}

/// Polls what keeps the player's own pp (the social link, the scores' answers and the profile
/// fetches) until `done`, `secs` at most.
fn poll_own_pp(s: &mut State, secs: u64, done: &dyn Fn(&State) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while !done(s) && Instant::now() < until {
        s.poll_social();
        s.poll_online_scores();
        s.poll_me();
        std::thread::sleep(Duration::from_millis(5));
    }
    done(s)
}

/// The server answers a play with the player's pp total at once and settles the play's pp a
/// moment later (its replay is played back): the game asks again shortly after, so the total
/// it shows is the website's without logging in again.
#[test]
fn the_own_pp_total_follows_the_server_after_a_play_settles() {
    let (server, seen) = score_server(FirstPlay::Kept);
    seen.lock().unwrap().profile_pp = 50.0;
    let songs = Songs::new("own-pp-settles", &[maps::basics()]);
    let mut s = play_to_server(server, &songs);
    let answered = |s: &State| !seen.lock().unwrap().sent.is_empty() && s.submits.is_empty();
    assert!(
        poll_own_pp(&mut s, 30, &answered),
        "the play is sent and answered"
    );
    // Without asking again it would stay at the answer's 100 until the next login.
    assert!(
        poll_own_pp(&mut s, 10, &|s| s.me == Some((140.0, Some(7)))),
        "the settled total comes in after the play: {:?}",
        s.me
    );
    assert!(
        seen.lock().unwrap().profiles >= 2,
        "asked on login and after the play"
    );
}

/// The total changes on the server with no play from this game (a recalculation, an admin, a
/// play on another device): the game asks again every `ME_REFRESH`.
#[test]
fn the_own_pp_total_is_fetched_again_every_minute() {
    let (server, seen) = score_server(FirstPlay::Kept);
    seen.lock().unwrap().profile_pp = 50.0;
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    cfg.terms_version = terms::VERSION;
    let songs = Songs::new("own-pp-refresh", &[maps::basics()]);
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(
        poll_own_pp(&mut s, 30, &|s| s.me == Some((50.0, Some(7)))),
        "the total on login: {:?}",
        s.me
    );
    let due = s.me_due.expect("asked for again later");
    let wait = due.saturating_duration_since(Instant::now());
    assert!(
        wait > ME_REFRESH - Duration::from_secs(10) && wait <= ME_REFRESH,
        "in about a minute: {wait:?}"
    );
    seen.lock().unwrap().profile_pp = 75.0;
    // Not before then.
    poll_own_pp(&mut s, 1, &|_| false);
    assert_eq!(s.me, Some((50.0, Some(7))), "kept until it is due");
    // A minute on.
    s.me_due = Some(Instant::now());
    assert!(
        poll_own_pp(&mut s, 10, &|s| s.me == Some((75.0, Some(7)))),
        "the new total comes in: {:?}",
        s.me
    );
}

/// A server that refuses the play itself: the game tells the player and doesn't send it again.
#[test]
fn a_score_the_server_refuses_is_not_sent_again() {
    let (server, seen) = score_server(FirstPlay::Refused("that play doesn't add up"));
    let songs = Songs::new("score-refused", &[maps::basics()]);
    let mut s = play_to_server(server, &songs);
    let mut toasts = Vec::new();
    let answered = |s: &State| !seen.lock().unwrap().sent.is_empty() && s.submits.is_empty();
    assert!(
        note_answers(&mut s, &mut toasts, &answered),
        "the play is sent and answered"
    );
    let started = Instant::now();
    note_answers(&mut s, &mut toasts, &|_| {
        started.elapsed() > Duration::from_secs(1)
    });
    assert_eq!(seen.lock().unwrap().sent.len(), 1, "sent once");
    assert!(s.score_outbox.is_empty(), "nothing kept to send again");
    assert!(
        toasts
            .iter()
            .any(|t| t == "online: score not sent (that play doesn't add up)"),
        "the player hears why: {toasts:?}"
    );
}

/// A play with a mod being reworked is sent like any other, without pp, and the results tell
/// the player why it has none.
#[test]
fn a_play_with_a_mod_being_reworked_is_sent_without_pp() {
    let (server, seen) = score_server(FirstPlay::Kept);
    let songs = Songs::new("score-rework", &[maps::basics()]);
    let mut s = play_to_server_with(server, &songs, crate::mods::Mods::parse("TC"));
    let mut toasts = Vec::new();
    let answered = |s: &State| !seen.lock().unwrap().sent.is_empty() && s.submits.is_empty();
    assert!(
        note_answers(&mut s, &mut toasts, &answered),
        "the play is sent and answered"
    );
    assert_eq!(seen.lock().unwrap().pps, [None], "sent without pp");
    let results = match s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        Some(r) => &r.results,
        None => panic!("not on the results screen but {}", scene_name(&s)),
    };
    assert_eq!(results.pp, None);
    assert_eq!(
        results.no_pp,
        Some("mods being reworked give no pp for now")
    );
}

/// Play (and clicking the selected difficulty) starts the song even with a control on Enter, as
/// an older build let a preset key go: it used to change the playfield's size instead.
#[test]
fn play_starts_the_song_whatever_is_on_enter() {
    let (mut s, _songs, mut now) = client("play-over-enter", vec![maps::basics()]);
    s.cfg.preset_keys[1] = keys::Chord::plain(KeyCode::Enter);
    let area = s.cfg.playfield_area;
    s.click(hit::Hit::Key(KeyCode::Enter), None);
    assert_eq!(s.cfg.playfield_area, area, "the playfield keeps its size");
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"),
        "Play starts the map, not {}",
        scene_name(&s)
    );
}
