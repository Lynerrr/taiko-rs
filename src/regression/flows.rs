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
fn start_selected(s: &mut State, now: &mut Instant) {
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
fn play_out(s: &mut State, now: &mut Instant, presses: &[(f64, u8)]) {
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
