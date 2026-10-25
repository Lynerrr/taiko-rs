//! Every screen drawn at the aspect ratios people play at, with long names everywhere: no text
//! may leave the screen, stick out of its box, get cut off or land on other text.
//! `TAIKO_LAYOUT_SHOTS=<dir>` also saves each frame as a PNG (drawn on the CPU) to look at,
//! in the UI colour `TAIKO_LAYOUT_ACCENT` (a preset name or #rrggbb) if set.

use super::*;

/// Virtual canvas widths (the canvas is 480 tall): 5:4, 4:3, 16:10, 16:9, 21:9.
const WIDTHS: [f32; 5] = [600.0, 640.0, 768.0, 853.3, 1120.0];
/// Phones (half resolution, text on the pixel grid): 16:9, 19.5:9, 20:9.
const PHONE_WIDTHS: [f32; 3] = [853.3, 1040.0, 1066.7];

thread_local! {
    /// Checking the phone layout (`PHONE_WIDTHS`) instead of `WIDTHS`.
    static PHONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn widths() -> &'static [f32] {
    if PHONE.with(std::cell::Cell::get) { &PHONE_WIDTHS } else { &WIDTHS }
}

fn entry(set: usize, diff: usize, long: bool) -> Entry {
    let meta = beatmap::Meta {
        title: if long { format!("Extremely Long Song Title That Goes On And On (Extended Mix Ver.) {set}") } else { format!("Song {set}") },
        artist: if long { "Camellia feat. Hatsune Miku & Some Other Very Long Artist Name".into() } else { "Artist".into() },
        creator: if long { "SomeMapperWithALongName".into() } else { "mapper".into() },
        version: if long { format!("Hyperdimensional Inner Oni Collab Difficulty {diff}") } else { format!("Oni {diff}") },
        mode: 1,
        od: 5.0,
        hp: 5.0,
        slider_multiplier: 1.4,
        slider_tick_rate: 1.0,
        format_version: 14,
        ..Default::default()
    };
    Entry::new(PathBuf::from(format!("/nonexistent/{set} set/{diff}.osu")), meta)
}

pub(crate) fn state() -> State {
    let mut cfg = Config::default();
    cfg.check_updates = false;
    cfg.mods = Mods::parse("HD,HR,DT,FL,PR,AC,WV,PY,SU,MR");
    cfg.master_volume = 0.0;
    cfg.animations = false;
    // Nothing to connect to: the menus show the server as offline instead of asking it.
    cfg.online_server = "127.0.0.1:9".into();
    if let Some(a) = std::env::var("TAIKO_LAYOUT_ACCENT").ok().and_then(|a| render::parse_accent(&a)) {
        cfg.accent = a;
    }
    let (tx, rx) = mpsc::channel();
    let library: Vec<Entry> = (0..12).flat_map(|s| (0..3).map(move |d| entry(s, d, s % 2 == 0))).collect();
    tx.send(library).unwrap();
    let mut s = State::new(cfg, None, None, PathBuf::from("/nonexistent"), Vec::new(), Some((rx, Arc::new(AtomicUsize::new(0)))), true, false);
    s.window_w = 1920.0;
    s.window_h = 1080.0;
    s.mouse = (-1000.0, -1000.0);
    s
}

/// Draws `s` at every width (twice, so fades settle) and collects what doesn't fit, prefixed
/// by `what`.
fn check(s: &mut State, what: &str, out: &mut Vec<String>) {
    // Toasts from earlier screens (maps that aren't really there fail to load) stay out of it.
    if what != "toast" {
        s.toast = None;
    }
    for &w in widths() {
        let mut c = Canvas { audit: true, ..Default::default() };
        s.build_frame(&mut c, Instant::now(), w);
        s.scene_since = Instant::now() - Duration::from_secs(5);
        s.build_frame(&mut c, Instant::now() + Duration::from_millis(16), w);
        for p in debug::layout_problems(&c) {
            out.push(format!("[{what} @{w:.0}] {p}"));
        }
        if let Some(dir) = std::env::var_os("TAIKO_LAYOUT_SHOTS") {
            let (iw, ih, rgb) = debug::rasterize(&c, 2.0);
            let name = format!("{}-{w:.0}.png", what.replace([' ', '/'], "_"));
            debug::write_png(&std::path::Path::new(&dir).join(name), iw, ih, &rgb).unwrap();
        }
    }
}

/// Like `check`, for something drawn on its own.
fn check_draw(what: &str, out: &mut Vec<String>, mut draw: impl FnMut(&mut Canvas)) {
    for &w in widths() {
        let mut c = Canvas { audit: true, ..Default::default() };
        c.clear(w);
        c.mouse = (-1000.0, -1000.0);
        draw(&mut c);
        for p in debug::layout_problems(&c) {
            out.push(format!("[{what} @{w:.0}] {p}"));
        }
        if let Some(dir) = std::env::var_os("TAIKO_LAYOUT_SHOTS") {
            let (iw, ih, rgb) = debug::rasterize(&c, 2.0);
            let name = format!("{}-{w:.0}.png", what.replace([' ', '/'], "_"));
            debug::write_png(&std::path::Path::new(&dir).join(name), iw, ih, &rgb).unwrap();
        }
    }
}

const LONG_NAME: &str = "FriendWithAVeryLongName";

/// A room hosted on this machine, filled with long names (its lobby, match and result screens).
fn session() -> multiplayer::Session {
    // Another test can take the free port found here before the room listens on it: try again.
    let mut s = (0..10)
        .find_map(|_| {
            let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
            multiplayer::start_host(port, LONG_NAME.into()).recv_timeout(Duration::from_secs(5)).ok()?.ok()
        })
        .expect("host a room on this machine");
    let e = entry(0, 0, true);
    s.map = Some(net::MapInfo {
        hash: 1,
        artist: e.meta.artist.clone(),
        title: e.meta.title.clone(),
        version: e.meta.version.clone(),
        rate: 1.5,
        from: Some(61_000.0),
        set_id: Some(1),
        mods: Mods::parse("HD,HR,DT,FL,PR,AC,WV,PY,SU,MR").list(),
    });
    let live = net::Live { score: 1_234_567, combo: 1234, accuracy: 0.9876, miss: 12, progress: 0.5 };
    let result = net::Final { score: 1_234_567, accuracy: 0.9876, max_combo: 1290, great: 1234, ok: 56, miss: 7, ur: 123.4, quit: false };
    s.players = (0..8)
        .map(|i| net::PlayerInfo {
            id: i,
            name: format!("{LONG_NAME}{i}"),
            host: i == 0,
            ready: i % 2 == 0,
            has_map: i % 3 != 0,
            status: [net::Status::Lobby, net::Status::Loading, net::Status::Playing, net::Status::Finished][i as usize % 4],
            live: live.clone(),
            result: Some(result.clone()),
            spectator: i == 7,
        })
        .collect();
    s.results = Some(s.players.clone());
    s.chat = (0..12).map(|i| (format!("{LONG_NAME}{i}"), "a pretty long chat message that should wrap or get cut somewhere sensible".to_string())).collect();
    s
}

#[test]
fn every_screen_fits_at_every_aspect_ratio() {
    assert_every_screen_fits();
}

/// Android's default: half resolution on a 1080 px tall phone, where text snaps to whole pixels
/// (and so mostly grows).
#[test]
fn every_screen_fits_on_a_phone_at_half_resolution() {
    PHONE.with(|p| p.set(true));
    render::set_text_grid(Some(540.0 / render::VIRTUAL_HEIGHT));
    assert_every_screen_fits();
}

fn assert_every_screen_fits() {
    let mut out = Vec::new();
    let mut s = state();
    // The first frame takes in the library.
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.select.collections = vec![("My Favourite Tech Maps For Practice".into(), Default::default()), ("dans".into(), Default::default())];
    check(&mut s, "select", &mut out);
    s.select.collection = Some(0);
    s.select.filter = "camellia long search query that is typed".into();
    s.select.refilter(&s.library);
    check(&mut s, "select filtered", &mut out);
    s.select.filter.clear();
    s.select.collection = None;
    s.select.refilter(&s.library);

    let hash = s.library[0].hash();
    s.scores = (0..5)
        .map(|i| scores::ScoreRow {
            map_hash: hash,
            date: 1_700_000_000 + i,
            score: 1_234_567 - i * 1000,
            accuracy: 0.9876,
            max_combo: 1290,
            miss: 12,
            rate: 1.25,
            player: LONG_NAME.into(),
            file: PathBuf::from("/nonexistent/replay.json"),
            mods: "HDHRDTFLPRACWVKBSUDK".into(),
            pp: Some(1234.56),
        })
        .collect();
    check(&mut s, "select with scores", &mut out);
    // Many scores, scrolled, with the song list and the preview dragged to their limits.
    let more: Vec<scores::ScoreRow> = (5..40)
        .map(|i| {
            let mut r = s.scores[0].clone();
            r.date += i;
            r.score -= i * 1000;
            r
        })
        .collect();
    s.scores.extend(more);
    for (share, size) in [(ui::LIST_SHARE.0, preview::PREVIEW_SIZE.0), (ui::LIST_SHARE.1, preview::PREVIEW_SIZE.0), (ui::LIST_SHARE.1, preview::PREVIEW_SIZE.1), (ui::LIST_SHARE.0, preview::PREVIEW_SIZE.1)] {
        s.cfg.song_list_share = share;
        s.cfg.song_preview_size = size;
        s.board_scroll = 10;
        check(&mut s, &format!("select resized {share} {size}"), &mut out);
    }
    s.cfg.song_list_share = ui::LIST_SHARE.2;
    s.cfg.song_preview_size = preview::PREVIEW_SIZE.1;
    s.scores.truncate(5);
    s.board_scroll = 0;
    // The online tab: other players, long names and mods, still loading, and the server away.
    s.board_online = true;
    let online_rows: Vec<online::OnlineScore> = (0..8)
        .map(|i| online::OnlineScore {
            player: if i == 1 { LONG_NAME.into() } else { format!("player{i}") },
            map: "A - T [Oni]".into(),
            map_hash: hash,
            score: 1_234_567 - i * 1000,
            accuracy: 0.9876,
            max_combo: 1290,
            miss: 12,
            rate: 1.25,
            mods: ["HD", "HR", "DT", "FL", "PR", "AC", "WV", "SU"].iter().map(|m| m.to_string()).collect(),
            pp: Some(456.7),
            date: 1_700_000_000,
            ..Default::default()
        })
        .collect();
    s.cfg.online_user = "player2".into();
    s.online_boards.insert(hash, (Instant::now(), Ok(online_rows)));
    check(&mut s, "select online scores", &mut out);
    s.board_scroll = 3;
    check(&mut s, "select online scores scrolled", &mut out);
    s.board_scroll = 0;
    s.online_boards.insert(hash, (Instant::now(), Err("the server is offline (connection refused)".into())));
    check(&mut s, "select online scores, server away", &mut out);
    s.online_boards.clear();
    s.board_online = false;
    s.select.menu = Some(ui::SongMenu { row: s.select.cursor(), header: true, x: 2000.0, y: 300.0 });
    check(&mut s, "song menu", &mut out);
    s.select.menu = None;

    s.open_preview();
    check(&mut s, "preview", &mut out);
    s.close_preview();

    s.open_mod_select();
    check(&mut s, "mod select", &mut out);
    // Favourites: more than fit in their row.
    s.cfg.favourite_mods = mods::ALL.iter().step_by(3).fold(mods::Mods::NONE, |f, &m| f.flip(m));
    check(&mut s, "mod select favourites", &mut out);
    s.cfg.favourite_mods = mods::Mods::NONE;
    // A customizable mod on: the Customize button and its tab (Multiverse and the rails conflict).
    let mods_before = s.cfg.mods;
    s.cfg.mods = mods::Mods::parse("MV,SS");
    check(&mut s, "mod select, customizable mod on", &mut out);
    s.open_customization(mods::Mod::Shapeshift);
    check(&mut s, "shapeshift setup", &mut out);
    s.cfg.shapeshift.shapes[2].1 = false;
    s.shapeshift_setup = Some(shapeshift::Setup { row: 3, col: 4 });
    check(&mut s, "shapeshift setup, shapes row", &mut out);
    s.shapeshift_setup = None;
    s.universe_setup = Some(multiverse::Setup::new(3, 7));
    check(&mut s, "multiverse setup", &mut out);
    // Presets with long names (more than fit), one of them the universes now, and own colours
    // being typed.
    s.cfg.universe_presets = ["chaos but readable", "A Very Long Preset Name Indeed", "x", "tournament warmup set"]
        .iter()
        .enumerate()
        .map(|(k, name)| multiverse::Preset { name: name.to_string(), universes: multiverse::shuffle(10 * k as u64 + 2, game::FIRST_FOCUS) })
        .collect();
    s.cfg.universes = s.cfg.universe_presets[1].universes;
    s.universe_setup = Some(multiverse::Setup { typing: Some(multiverse::Typing::Colours("#ff4020 #40a0ff and some more".into())), ..multiverse::Setup::new(2, 0) });
    check(&mut s, "multiverse setup presets", &mut out);
    s.universe_setup = None;
    s.cfg.universe_presets.clear();
    s.mod_select = None;
    s.cfg.mods = mods_before;
    s.cfg.shapeshift = Default::default();

    // A profile: long name, more plays than fit, long map names; then one that failed.
    let play = |i: u64| online::OnlineScore {
        player: LONG_NAME.into(),
        map: "Camellia feat. Hatsune Miku & Some Other Very Long Artist Name - Extremely Long Song Title [Hyperdimensional Inner Oni]".into(),
        map_hash: i,
        score: 1_234_567,
        accuracy: 0.9876,
        max_combo: 1290,
        miss: 12,
        rate: 1.25,
        mods: ["HD", "HR", "DT", "FL", "PR", "AC", "WV", "SU"].iter().map(|m| m.to_string()).collect(),
        pp: Some(1234.5 - i as f64),
        date: 1_700_000_000,
        ..Default::default()
    };
    let loaded = online::Profile { name: LONG_NAME.into(), joined: 1_700_000_000, plays: 12_345, pp: 12_345.6, rank: Some(1234), top: (0..20).map(play).collect(), recent: (0..20).map(play).collect(), ..Default::default() };
    let (_, rx) = std::sync::mpsc::channel();
    s.profile = Some(profile::View { name: LONG_NAME.into(), state: profile::State::Loading(rx) });
    check(&mut s, "profile loading", &mut out);
    s.profile = Some(profile::View { name: LONG_NAME.into(), state: profile::State::Loaded(Box::new(loaded)) });
    check(&mut s, "profile", &mut out);
    s.profile = Some(profile::View { name: "x".into(), state: profile::State::Failed("the server doesn't have profiles yet".into()) });
    check(&mut s, "profile failed", &mut out);
    s.profile = None;

    s.open_collections();
    check(&mut s, "collections", &mut out);
    s.select.picker = None;

    s.drag_hover = Some(Instant::now() - Duration::from_secs(1));
    check(&mut s, "files dragged over", &mut out);
    s.drag_hover = None;

    s.cfg.skin = "Some Extremely Long Skin Name By A Famous Player [HD Edition] v2.1".into();
    s.open_settings();
    let mut section = "";
    for (i, (name, _)) in settings::ITEMS.iter().enumerate() {
        s.settings_ui.cursor = i;
        if *name != section {
            section = name;
            check(&mut s, &format!("settings {section}"), &mut out);
        }
    }
    let skins = vec!["built-in".into(), s.cfg.skin.clone(), "- Rafis 2017 Remake (Taiko HD) by somebody".into()];
    s.settings_ui.choices = Some(settings::Choices::new(Item::Skin, skins, Some(1), "2 installed".into()));
    check(&mut s, "skin picker", &mut out);
    s.settings_ui.choices = None;
    for item in [Item::PresentMode, Item::AudioBuffer, Item::Accent, Item::SongsFolder, Item::WindowSize, Item::DataFolder, Item::InterfaceHides] {
        s.open_choices(item);
        check(&mut s, &format!("{item:?} list"), &mut out);
        s.settings_ui.choices = None;
    }
    s.cfg.songs_folder = Some(PathBuf::from("/home/someone/.local/share/osu-wine/A Very Long Folder Name For The osu! Install/Songs"));
    for item in [Item::SongsFolder, Item::ScrollSpeed] {
        s.start_edit(item);
        check(&mut s, &format!("typing {item:?}"), &mut out);
        s.settings_ui.editing.as_mut().unwrap().type_text("1", 400);
        check(&mut s, &format!("typed {item:?}"), &mut out);
        s.settings_ui.editing = None;
    }
    s.cfg.songs_folder = None;
    s.open_dropdown();
    check(&mut s, "playfield presets", &mut out);
    s.settings_ui.dropdown = None;
    s.close_settings();

    s.open_layout_editor();
    check(&mut s, "layout editor", &mut out);
    s.scene = Scene::Select;
    s.start_calibration();
    check(&mut s, "calibration", &mut out);
    let estimate = calibrate::Estimate { hits: 123, error_ms: -12.3, ur: 150.0, margin_ms: 2.5, suggested_offset: -45.0 };
    s.scene = Scene::CalibrateDone { estimate, back: Box::new(Scene::Select), vsync: false };
    check(&mut s, "calibration result", &mut out);
    s.scene = Scene::Select;
    s.start_calibration_as(true);
    check(&mut s, "vsync calibration", &mut out);
    s.scene = Scene::CalibrateDone { estimate, back: Box::new(Scene::Select), vsync: true };
    check(&mut s, "vsync calibration result", &mut out);
    s.scene = Scene::Select;

    // Not `open_download`, which would search the mirrors.
    s.scene = Scene::Download;
    check(&mut s, "download", &mut out);
    let e = entry(0, 0, true);
    s.browse.results = (0..10)
        .map(|i| download::SetInfo {
            id: i,
            artist: e.meta.artist.clone(),
            title: e.meta.title.clone(),
            creator: e.meta.creator.clone(),
            status: "ranked".into(),
            // Id 0: no preview gets fetched.
            diffs: (0..6).map(|d| download::Diff { id: 0, version: format!("Hyperdimensional Inner Oni {d}"), stars: 6.5 + d as f32 }).collect(),
            play_count: 1_234_567,
            favourite_count: 12_345,
            rating: 9.5,
            ranked_date: "2024-01-01T00:00:00Z".into(),
        })
        .collect();
    s.browse.query = "a long search query typed into the download tab".into();
    check(&mut s, "download results", &mut out);
    s.scene = Scene::MpMenu;
    check(&mut s, "multiplayer menu", &mut out);
    s.scene = Scene::Select;

    s.mp_menu.account = Some("MyOwnAccountName".into());
    s.social.open = true;
    s.social.connected = true;
    s.social.me = Some("MyOwnAccountName".into());
    s.social.friends = vec![
        online::Friend { name: "FriendWithAVeryLongName".into(), activity: Some(online::Activity::Playing { map: entry(0, 0, true).label() }) },
        online::Friend { name: "bob".into(), activity: None },
    ];
    check(&mut s, "friends", &mut out);
    s.social.requests = vec!["SomebodyNewWhoWantsIn".into()];
    s.social.sent = vec!["AnotherPersonWithLongName".into(), "and_another_one".into()];
    let line = |from: &str, to: &str| online::ChatLine { from: from.into(), to: to.into(), text: "a pretty long chat message that should wrap or get cut somewhere sensible, with more words".into(), at: 1_700_000_000 };
    s.social.chats.insert(social::key(LONG_NAME), (0..20).map(|i| if i % 2 == 0 { line(LONG_NAME, "MyOwnAccountName") } else { line("MyOwnAccountName", LONG_NAME) }).collect());
    s.social.cursor = 0;
    s.social.input = "a message being typed that is longer than the input box can show".into();
    check(&mut s, "friends chat", &mut out);
    s.social.input.clear();
    s.social.tab = social::Tab::Players;
    s.social.online_total = 1234;
    s.social.players = Some(
        (0..40)
            .map(|i| online::OnlinePlayer { name: format!("{LONG_NAME}{i}"), activity: online::Activity::Playing { map: entry(0, 0, true).label() } })
            .collect(),
    );
    check(&mut s, "friends players", &mut out);
    s.social.tab = social::Tab::Chat;
    s.social.global = (0..40).map(|i| line(&format!("{LONG_NAME}{i}"), "")).collect();
    s.social.global_unread = 12;
    s.social.input = "a message being typed that is longer than the input box can show".into();
    check(&mut s, "global chat", &mut out);
    s.social.tab = social::Tab::Friends;
    s.social.input.clear();
    s.social.open = false;
    s.social.notes = vec![social::Note {
        title: format!("{LONG_NAME} sent you a message"),
        body: "a pretty long chat message that should wrap or get cut somewhere sensible".into(),
        colour: 0x4fb4ff,
        action: social::NoteAction::Chat(LONG_NAME.into()),
        // Done sliding in.
        at: Instant::now() - Duration::from_secs(1),
    }];
    check(&mut s, "notification", &mut out);
    s.social.notes.clear();
    s.mp_menu.account = None;

    let change = |k: usize| changelog::Change {
        date: "2026-10-26".into(),
        title: format!("{k}: pp counter, results screen with pp breakdown and music, online replays, every hotkey rebindable"),
        points: vec!["Live pp counter in the HUD (rosu-pp's gradual calculation on its own thread), hideable".into(), "Clicking a local or online leaderboard score opens its results first".into()],
    };
    s.changelog = Some(changelog::View::loaded((0..12).map(change).collect()));
    check(&mut s, "what's new", &mut out);
    s.changelog = None;

    s.toast("a fairly long notification that tells you something went wrong with the download".into());
    check(&mut s, "toast", &mut out);
    s.toast = None;

    let mut map = beatmap::calibration_map(500.0, 4, 64);
    map.meta = entry(0, 0, true).meta;
    let game = Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, s.cfg.mods);
    s.scene = Scene::Play { game: Box::new(game), entry: 0, from: None };
    check(&mut s, "play", &mut out);
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    check(&mut s, "pause", &mut out);

    let results = |failed: bool| {
        let stats = game::Stats { great: 1234, ok: 56, miss: if failed { 300 } else { 7 }, max_combo: 1290, score: 1_234_567, ..Default::default() };
        let mut r = ui::Results::new(entry(0, 0, true).label(), stats, game::Windows::new(5.0));
        r.shown = Instant::now() - Duration::from_secs(10);
        r.replay = (!failed).then(|| PathBuf::from("/nonexistent/replay.json"));
        r.rank = (!failed).then_some((1, 12));
        r.watched = (!failed).then(|| LONG_NAME.to_string());
        r.failed = failed;
        r.pp = (!failed).then_some(456.7);
        r.no_pp = failed.then_some("failed plays get no pp");
        r.total = (!failed).then_some(ui::Total { before: Some(12_333.3), after: 12_345.6, rank: Some(99_999) });
        r.breakdown = (!failed).then_some(pp::Breakdown {
            pp: 456.7,
            difficulty: 1234.5,
            accuracy: 345.6,
            if_fc: 1500.2,
            if_ss: 1888.8,
            ss_difficulty: 1500.0,
            ss_accuracy: 500.0,
            stars: 12.34,
            max_combo: 12345,
        });
        r
    };
    // Logged in: the friends chip is drawn over the results too.
    s.mp_menu.account = Some(LONG_NAME.to_string());
    s.me = Some((12_345.6, Some(99_999)));
    s.me_for = LONG_NAME.to_string();
    s.scene = Scene::Select;
    check(&mut s, "select logged in", &mut out);
    s.scene = Scene::Results { results: results(false), entry: 0, from: None };
    check(&mut s, "results", &mut out);
    s.scene = Scene::Results { results: results(true), entry: 0, from: None };
    check(&mut s, "results failed", &mut out);
    let mut opened = results(false);
    (opened.watched, opened.rank, opened.replay, opened.partial) = (None, None, None, true);
    opened.opened = Some(ui::Opened { player: LONG_NAME.to_string(), date: 0, online: true });
    opened.replay_error = Some("that replay isn't on the server".into());
    s.scene = Scene::Results { results: opened, entry: 0, from: None };
    check(&mut s, "results of an online score", &mut out);
    // Scrolled down to the pp.
    for failed in [false, true] {
        let mut r = results(failed);
        (r.scroll, r.scroll_to) = (ui::RESULTS_SCROLL, ui::RESULTS_SCROLL);
        s.scene = Scene::Results { results: r, entry: 0, from: None };
        check(&mut s, if failed { "results failed, performance" } else { "results, performance" }, &mut out);
    }
    s.mp_menu.account = None;
    s.me = None;
    s.scene = Scene::Select;

    // The video editor (its map still loading): quality, a target size being typed, exporting.
    let json = format!(r#"{{"version":1,"map_hash":1,"map_path":"","map_label":"{}","player":"{LONG_NAME}","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[]}}"#, entry(0, 0, true).label());
    let (_video_tx, video_rx) = mpsc::channel();
    let settings = video::Settings { size: 4, fps: 3, blur: 2, speed: 1, ..Default::default() };
    s.scene = Scene::VideoEdit(Box::new(video::Editor::new(serde_json::from_str(&json).unwrap(), video_rx, settings)));
    check(&mut s, "video editor", &mut out);
    if let Scene::VideoEdit(ed) = &mut s.scene {
        ed.settings.quality = video::QUALITIES.len() - 1;
        ed.typing = Some("1234.5".into());
    }
    check(&mut s, "video editor target size", &mut out);
    if let Scene::VideoEdit(ed) = &mut s.scene {
        ed.typing = None;
    }
    s.video_status = "saved \"Camellia feat. Hatsune Miku - Extremely Long Song Title.mp4\" in the videos folder".into();
    check(&mut s, "video editor saved", &mut out);
    s.video_status.clear();
    s.scene = Scene::Select;

    // No maps anywhere: the Songs folder button.
    let library = std::mem::take(&mut s.library);
    s.select = Select::new(&s.library);
    check(&mut s, "no maps", &mut out);
    s.select.picking_folder = true;
    check(&mut s, "no maps, choosing", &mut out);
    s.library = library;
    s.select = Select::new(&s.library);

    let room = session();
    let player = net::Live { score: 1_234_567, combo: 1234, accuracy: 0.9876, miss: 12, progress: 0.5 };
    check_draw("lobby", &mut out, |c| multiplayer::draw_lobby(c, &room, false, Some("downloading the map: 45% of 12.3 MB".into())));
    check_draw("match waiting", &mut out, |c| multiplayer::draw_waiting(c, &room, None, false, Instant::now()));
    check_draw("match scoreboard", &mut out, |c| multiplayer::draw_scoreboard(c, &room, &player));
    check_draw("match results", &mut out, |c| multiplayer::draw_results(c, &room));

    for p in &out {
        eprintln!("{p}");
    }
    assert!(out.is_empty(), "{} layout problems", out.len());
}
