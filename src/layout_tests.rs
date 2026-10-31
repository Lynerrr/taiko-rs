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

/// A room hosted on this machine (`mapguessr`: playing mapguessr), filled with long names (its
/// lobby, match and result screens).
fn session(mapguessr: bool) -> multiplayer::Session {
    // Another test can take the free port found here before the room listens on it: try again.
    let mut s = (0..10)
        .find_map(|_| {
            let port = std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok()?.port();
            multiplayer::start_host(port, LONG_NAME.into(), mapguessr).recv_timeout(Duration::from_secs(5)).ok()?.ok()
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
        multiverse: None,
        shapeshift: None,
        seed: None,
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
            points: 12_345 * i,
            guessed: i % 2 == 0,
            tries: (i % 4) as u8,
            maps: (i != 3).then_some(123_456),
            mods: if i % 2 == 0 { vec!["HD".into(), "HR".into(), "FL".into(), "PR".into()] } else { Vec::new() },
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
    s.select.searching = true;
    check(&mut s, "select filtered, typing", &mut out);
    s.select.filter.clear();
    check(&mut s, "select, typing", &mut out);
    s.select.searching = false;
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
    // Every song has a newer version on osu! (one being downloaded).
    let sets: Vec<u32> = s.library.iter().filter_map(|e| e.set_id()).collect();
    s.select.updates = sets.iter().map(|&id| (id, if id == 2 { ui::SetUpdate::Updating } else { ui::SetUpdate::Available })).collect();
    check(&mut s, "select updates", &mut out);
    s.select.menu = Some(ui::SongMenu { row: s.select.cursor(), header: true, x: 2000.0, y: 300.0 });
    check(&mut s, "song menu", &mut out);
    s.select.menu = None;
    s.select.updates.clear();

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
    s.crash = Some("--- taiko.rs crash".into());
    check(&mut s, "crash report", &mut out);
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(s.crash.is_none(), "Esc closes it");
    // Signing up with a code, and resetting a password.
    s.mp_menu.email = "a.rather.long.email.address@example-mail.com".into();
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Register);
    s.mp_menu.code = "123456".into();
    check(&mut s, "multiplayer menu code", &mut out);
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Reset);
    s.mp_status = "code sent to g******@gmail.com (check spam too): type it and a new password, then SET NEW PASSWORD".into();
    check(&mut s, "multiplayer menu reset", &mut out);
    s.mp_status.clear();
    // An old account without an email, with a full lobby list, and linking one.
    s.mp_menu.code_for = None;
    s.mp_menu.account = Some("MyOwnAccountName".into());
    s.mp_menu.has_email = Some(false);
    s.mp_menu.rooms = (0..8).map(|i| online::RoomInfo { id: i, title: format!("{LONG_NAME}'s lobby {i}"), host: LONG_NAME.into(), players: 3, map: Some(entry(0, 0, true).label()), in_match: i % 2 == 0, mapguessr: false }).collect();
    check(&mut s, "multiplayer menu no email", &mut out);
    s.mp_menu.linking = true;
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Link);
    check(&mut s, "multiplayer menu linking", &mut out);
    s.mp_menu = Default::default();
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
        // Lots of mods (two rows and a count of the rest) and a speed of its own.
        r.set_mods(Mods::parse("HD,HR,DT,FL,PR,AC,WV,PY,SU,MR,SK,TL,FF,SH,CK,PT,LI,DY,MM,OT,BL,ST,SG,LF,PN,MW,SR"), if failed { 1.25 } else { 1.0 });
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

    check_draw("spectators", &mut out, |c| social::draw_spectators(c, &["Ryuji", "a_sixteen_letter", "x", "y", "z", "u", "v", "w", "q", "r"]));

    let room = session(false);
    let player = net::Live { score: 1_234_567, combo: 1234, accuracy: 0.9876, miss: 12, progress: 0.5 };
    check_draw("lobby", &mut out, |c| multiplayer::draw_lobby(c, &room, false, Some("downloading the map: 45% of 12.3 MB".into())));
    check_draw("match waiting", &mut out, |c| multiplayer::draw_waiting(c, &room, None, false, Instant::now()));
    check_draw("match scoreboard", &mut out, |c| multiplayer::draw_scoreboard(c, &room, &player));
    check_draw("match results", &mut out, |c| multiplayer::draw_results(c, &room));

    // The exit question, over song select.
    s.scene = Scene::Select;
    s.quit_asked = true;
    check(&mut s, "exit question", &mut out);
    s.quit_asked = false;

    // Mapguessr: the lobby (after a game, so with points), a round, its answer, and picking a
    // guess in song select.
    let guessr_room = || {
        let mut room = session(true);
        room.mapguessr = true;
        room.common = Some(123_456);
        room
    };
    let mut room = guessr_room();
    let mut g = mapguessr::Guessr::new(&[]);
    g.rounds = 50;
    g.test_round(None);
    check_draw("mapguessr lobby", &mut out, |c| mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now()));
    room.in_match = true;
    g.test_round(Some(beatmap::calibration_map(250.0, 0, 600)));
    g.tries = 2;
    g.last_guess = Some(format!("{LONG_NAME} - a song with a very long title (TV Size) [Extra Oni]"));
    // A few seconds in, so the notes are on the lane.
    for k in 0..4 {
        g.tick(&room, Instant::now() + Duration::from_secs(k));
    }
    check_draw("mapguessr round", &mut out, |c| mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now()));
    g.reveal = Some(mapguessr::Reveal { label: format!("{LONG_NAME} - a song with a very long title (TV Size) [Extra Oni]"), got: vec![(0, 199), (2, 150)], last: true });
    check_draw("mapguessr answer", &mut out, |c| mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now()));
    g.reveal = None;
    s.mp = Some(guessr_room());
    s.guessr = Some(g);
    s.picking_lobby_map = true;
    s.scene = Scene::Select;
    check(&mut s, "mapguessr picking", &mut out);

    for p in &out {
        eprintln!("{p}");
    }
    assert!(out.is_empty(), "{} layout problems", out.len());
}

#[test]
fn statuses_from_the_cache_reach_the_song_list_before_osu_db_does() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    for e in s.library.iter_mut().take(3) {
        e.meta.set_id = Some(77);
    }
    let sets = [77];
    s.select.status.clear();
    s.status_cache.put(sets[0], "graveyard", 1);
    s.apply_cached_statuses();
    let with: Vec<_> = s.library.iter().filter(|e| e.meta.set_id == Some(sets[0])).collect();
    assert!(with.iter().all(|e| s.select.status.get(&e.path) == Some(&search::GRAVEYARD)));
    // osu!.db's doesn't overwrite what the mirrors said.
    s.db_status.insert(with[0].path.clone(), 4);
    s.select.status.entry(with[0].path.clone()).or_insert(4);
    assert_eq!(s.select.status[&with[0].path], search::GRAVEYARD);
}

#[test]
fn an_unfocused_window_draws_at_most_30_fps() {
    let mut s = state();
    s.cfg.fps_cap = 0;
    assert_eq!(s.frame_cap(), 0);
    s.focused = false;
    assert_eq!(s.frame_cap(), 30);
    s.cfg.fps_cap = 20;
    assert_eq!(s.frame_cap(), 20, "a lower cap of the player's stays");
    s.cfg.fps_cap = 240;
    assert_eq!(s.frame_cap(), 30);
    s.focused = true;
    assert_eq!(s.frame_cap(), 240);
}

#[test]
fn exiting_asks_first() {
    let mut s = state();
    let key = |s: &mut State, code| {
        // Ctrl+Q asks; F12 is the screenshot.
        s.modifiers_ctrl = code == KeyCode::KeyQ;
        s.key(code, false, None, Instant::now(), None);
        s.modifiers_ctrl = false;
    };
    key(&mut s, KeyCode::F12);
    assert!(!s.quit_asked && s.screenshot_due, "F12 takes a screenshot");
    key(&mut s, KeyCode::KeyQ);
    assert!(s.quit_asked && !s.quit);
    // Esc says stay; other keys do nothing while it's asked.
    key(&mut s, KeyCode::Escape);
    assert!(!s.quit_asked && !s.quit);
    key(&mut s, KeyCode::KeyQ);
    key(&mut s, KeyCode::KeyA);
    assert!(s.quit_asked && !s.quit);
    key(&mut s, KeyCode::Enter);
    assert!(s.quit);
    // Like osu!: 2 stays, 1 exits.
    let mut s = state();
    key(&mut s, KeyCode::KeyQ);
    key(&mut s, KeyCode::Digit2);
    assert!(!s.quit_asked && !s.quit);
    key(&mut s, KeyCode::KeyQ);
    key(&mut s, KeyCode::Digit1);
    assert!(s.quit);
    // The close button twice exits too.
    let mut s = state();
    s.ask_quit();
    assert!(!s.quit);
    s.ask_quit();
    assert!(s.quit);
}

/// Watching everyone at once, each player's strip follows where they play: a plain lane, rails
/// lanes, Shapeshift's and Multiverse's playfield in focus all land in their strip.
#[test]
fn watching_everyone_shows_where_each_player_plays() {
    let mods = ["", "SI", "LP", "SS", "MV", "UT,MV"];
    let players: Vec<(u32, String)> = (0..mods.len() as u32).map(|i| (i, format!("player {i}"))).collect();
    for &w in widths() {
        let mut games: Vec<(u32, Box<Game>)> = mods
            .iter()
            .enumerate()
            .map(|(i, mods)| {
                let mut map = beatmap::calibration_map(500.0, 4, 64);
                map.meta = entry(0, 0, false).meta;
                let mut g = Game::with_mods(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None, Mods::parse(mods));
                g.set_time(9000.0);
                (i as u32, Box::new(g))
            })
            .collect();
        let mut zones = Vec::new();
        let mut c = Canvas::default();
        c.clear(w);
        draw_spectate_all(&mut c, &mut games, &players, 1.0, &mut zones, 0.0);
        if let Some(dir) = std::env::var_os("TAIKO_LAYOUT_SHOTS") {
            let (iw, ih, rgb) = debug::rasterize(&c, 2.0);
            debug::write_png(&std::path::Path::new(&dir).join(format!("spectate_everyone-{w:.0}.png")), iw, ih, &rgb).unwrap();
        }
        assert_eq!(c.clips.len(), games.len());
        for (k, (range, [x, y, cw, ch])) in c.clips.iter().enumerate() {
            let shapes = &c.shapes[range.clone()];
            let inside = shapes.iter().filter(|s| (*x..x + cw).contains(&s.center[0]) && (*y..y + ch + 2.0).contains(&s.center[1])).count();
            // Multiverse's other playfields circle off the strip (and are clipped): the one in
            // focus is there.
            let share = if mods[k].contains("MV") { 0.125 } else { 0.9 };
            assert!(inside as f32 >= shapes.len() as f32 * share, "{} at {w}: {inside} of {} shapes in their strip", mods[k], shapes.len());
        }
    }
}

#[test]
fn song_select_search_takes_the_keys_only_once_clicked() {
    let mut s = state();
    let press = |s: &mut State, code: KeyCode, text: &str| s.key_inner(code, false, Some(text), Instant::now(), None);
    press(&mut s, KeyCode::KeyA, "a");
    assert_eq!(s.select.filter, "", "typing does nothing until the search box is clicked");
    s.select.searching = true;
    for (code, text) in [(KeyCode::Minus, "-"), (KeyCode::Digit4, "4"), (KeyCode::Digit5, "5"), (KeyCode::BracketLeft, "[")] {
        press(&mut s, code, text);
    }
    assert_eq!(s.select.filter, "-45[", "in the box every key types, - and [ too");
    press(&mut s, KeyCode::Escape, "");
    assert!(!s.select.searching && s.select.filter == "-45[", "Escape leaves the box, keeping the search");
    assert!(matches!(s.scene, Scene::Select));
}

#[test]
fn songs_sort_by_hp_od_and_status() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    for (k, e) in s.library.iter_mut().enumerate() {
        e.meta.hp = ((k * 7) % 11) as f64;
        e.meta.od = ((k * 5) % 9) as f64;
    }
    // A song's value is its highest difficulty's, songs in order of it.
    let song_values = |s: &State, of: fn(&Entry) -> f64| {
        s.select.songs().iter().map(|song| song.iter().map(|&i| of(&s.library[i])).fold(f64::MIN, f64::max)).collect::<Vec<f64>>()
    };
    s.select.sort = ui::SortMode::Hp;
    s.select.refilter(&s.library);
    assert!(s.select.songs().len() == 12 && song_values(&s, |e| e.meta.hp).is_sorted());
    s.select.sort = ui::SortMode::Od;
    s.select.refilter(&s.library);
    assert!(song_values(&s, |e| e.meta.od).is_sorted());
    // Status: ranked before loved before pending, the unknown last.
    let codes = [search::PENDING, search::RANKED, search::LOVED, search::UNKNOWN];
    let paths: Vec<PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();
    for (k, p) in paths.iter().enumerate() {
        s.select.status.insert(p.clone(), codes[(k / 3) % codes.len()]);
    }
    s.select.sort = ui::SortMode::Status;
    s.select.refilter(&s.library);
    let first: Vec<u8> = s.select.songs().iter().map(|song| s.select.status[&s.library[song[0]].path]).collect();
    let rank = |c: u8| [search::LOVED, search::RANKED, search::PENDING, search::UNKNOWN].iter().position(|&x| x == c).unwrap();
    assert!(first.iter().map(|&c| rank(c)).collect::<Vec<_>>().is_sorted(), "{first:?}");
    // A song whose difficulties differ in status is listed under its best one, never splitting
    // a heading: the same status doesn't come twice.
    let first_song = s.select.songs()[0].clone();
    for (k, &i) in first_song.iter().enumerate() {
        s.select.status.insert(s.library[i].path.clone(), if k == 0 { search::NOT_SUBMITTED } else { search::RANKED });
    }
    s.select.refilter(&s.library);
    let names: Vec<String> = s.select.sections().into_iter().map(|x| x.0).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "{names:?}");
    // Each status is a section of its own, with its songs counted; every sort has its headings.
    let sections = s.select.sections();
    assert_eq!(sections.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["Loved", "Ranked", "Pending / Graveyard", "Status not known yet"]);
    assert_eq!(sections.iter().map(|x| x.1).sum::<usize>(), s.select.songs().len());
    for sort in ui::SortMode::ALL {
        s.select.sort = sort;
        s.select.refilter(&s.library);
        let sections = s.select.sections();
        assert!(!sections.is_empty() && sections.iter().map(|x| x.1).sum::<usize>() == s.select.songs().len(), "{sort:?}");
        assert!(sections.iter().all(|x| !x.0.is_empty()), "{sort:?}");
    }
}

#[test]
fn by_difficulty_each_is_listed_under_its_own_stars() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // A song of 3, 4 and 5 stars, with two of them next to each other across a heading.
    let songs = s.select.songs();
    let total = songs.iter().map(Vec::len).sum::<usize>();
    let (spread, other) = (songs[0].clone(), songs[1].clone());
    assert!(spread.len() >= 3 && other.len() >= 2);
    let mut info = vec![None; s.library.len()];
    let mut stars = |i: usize, stars: f32| info[i] = Some(crate::mapinfo::Info { stars, length_ms: 60_000.0, bpm: 120.0 });
    for &i in s.select.songs().iter().flatten() {
        stars(i, 7.0);
    }
    stars(spread[0], 3.5);
    stars(spread[1], 4.99);
    stars(spread[2], 5.0);
    // Two of a song under one heading stay one song there.
    stars(other[0], 5.2);
    stars(other[1], 5.3);
    s.select.info = Some(std::sync::Arc::new(std::sync::Mutex::new(info)));
    s.select.sort = ui::SortMode::Difficulty;
    s.select.refilter(&s.library);
    let songs = s.select.songs();
    assert_eq!(songs[0], [spread[0]]);
    assert_eq!(songs[1], [spread[1]]);
    assert_eq!(songs[2], [spread[2]]);
    assert_eq!(songs[3], other[..2]);
    let sections = s.select.sections();
    assert_eq!(sections[..3].iter().map(|x| (x.0.as_str(), x.1)).collect::<Vec<_>>(), [("3.00 - 3.99 stars", 1), ("4.00 - 4.99 stars", 1), ("5.00 - 5.99 stars", 2)]);
    assert_eq!(songs.iter().map(Vec::len).sum::<usize>(), total);
    // Other sorts keep a song in one piece.
    s.select.sort = ui::SortMode::Length;
    s.select.refilter(&s.library);
    assert!(s.select.songs().iter().any(|song| song.contains(&spread[0]) && song.contains(&spread[2])));
}

#[test]
fn the_sort_button_lists_the_sorts() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let before = s.select.sort;
    s.click(render::Hit::SortList, None);
    assert_eq!(s.select.sort, before, "the button only opens the list");
    assert_eq!(s.select.picker_len(), ui::SortMode::ALL.len());
    let mut problems = Vec::new();
    check_draw("sort list", &mut problems, |c| s.select.draw_popups(c, &s.library));
    assert!(problems.is_empty(), "{problems:?}");
    let row = ui::SortMode::ALL.iter().position(|&m| m == ui::SortMode::Added).unwrap();
    s.click(render::Hit::PickRow(row), None);
    assert_eq!(s.select.sort, ui::SortMode::Added);
    assert_eq!(s.select.picker, None);
    assert_eq!(s.cfg.sort, "date added");
}

#[test]
fn headings_open_and_close_by_click_and_scrolling_never_changes_them() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let paths: Vec<PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();
    for (k, p) in paths.iter().enumerate() {
        s.select.status.insert(p.clone(), [search::LOVED, search::RANKED, search::PENDING][(k / 3) % 3]);
    }
    s.select.sort = ui::SortMode::Status;
    s.select.refilter(&s.library);
    assert!(s.select.sections().len() >= 3);
    // The heading holding the selection is open, the rest closed.
    assert_eq!(s.select.open_sections(), vec![0]);
    // A click opens another without selecting anything, and closes it again.
    let at = s.select.cursor();
    s.select.click_section(1);
    assert_eq!(s.select.open_sections(), vec![0, 1]);
    assert_eq!(s.select.cursor(), at);
    s.select.click_section(1);
    assert_eq!(s.select.open_sections(), vec![0]);
    // The open one closes too, with the selection in it, and opens again.
    s.select.click_section(0);
    assert!(s.select.open_sections().is_empty());
    check_draw("sections closed", &mut Vec::new(), |c| s.select.draw(c, &s.library, 16.0, "", &|_| None));
    s.select.click_section(0);
    assert_eq!(s.select.open_sections(), vec![0]);
    // The wheel and right-click scrolling stay in the open heading, however far they go.
    s.select.step(-50, &s.library, false);
    s.select.step(500, &s.library, false);
    assert_eq!(s.select.open_sections(), vec![0], "the wheel opened nothing");
    s.select.scroll_to(1000.0, &s.library);
    assert_eq!(s.select.open_sections(), vec![0], "right-click scrolling opened nothing");
    let in_first = |s: &State| s.select.current_song().is_some_and(|g| g < s.select.sections_end(0));
    assert!(in_first(&s), "and the selection never left it");
    s.select.scroll_to(-1000.0, &s.library);
    assert!(in_first(&s));
    // The arrow keys, past the last song listed, open the next heading and go on into it.
    s.select.step(500, &s.library, false);
    let _ = s.select.key(KeyCode::ArrowDown, &s.library);
    for _ in 0..20 {
        let _ = s.select.key(KeyCode::ArrowDown, &s.library);
    }
    let open = s.select.open_sections();
    assert!(open.len() == 1 && open[0] > 0, "the keys went on into the next heading, and the one they left closed: {open:?}");
    // Moving the selection to another heading that is open closes the one it leaves.
    s.select.click_section(0);
    assert_eq!(s.select.open_sections().len(), 2);
    s.select.scroll_to(-1000.0, &s.library);
    let open = s.select.open_sections();
    assert!(open.len() == 1 && open[0] == 0, "scrolling back up left the other heading and closed it: {open:?}");
    // It all draws.
    check_draw("sections", &mut Vec::new(), |c| s.select.draw(c, &s.library, 16.0, "", &|_| None));
}

#[test]
fn a_guests_mod_select_shows_the_hosts_speed_mods_on() {
    let mut s = state();
    let mut room = session(false);
    // We're player 1; player 0 hosts.
    room.client.id = Some(1);
    room.map.as_mut().unwrap().mods = Mods::parse("DT,HD").list();
    s.mp = Some(room);
    s.scene = Scene::Lobby;
    s.cfg.mods = Mods::parse("FL");
    let shown = s.shown_mods();
    assert!(shown.has(mods::Mod::DoubleTime) && shown.has(mods::Mod::Flashlight), "the host's speed and our own");
    assert!(!shown.has(mods::Mod::Hidden), "the host's other mods aren't forced on us");
    // Not in a lobby (or as the host), the mod select shows just ours.
    s.scene = Scene::Select;
    assert_eq!(s.shown_mods(), s.cfg.mods);
}

#[test]
fn np_names_the_map_on_screen_with_a_link_others_can_click() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let np = s.now_playing().expect("a map is selected");
    let e = &s.library[s.select.selected().unwrap()];
    assert!(np.starts_with("is playing ") && np.contains(&e.meta.title) && np.contains(&format!("[{}]", e.meta.version)), "{np}");
    if let Some(set) = e.meta.set_id {
        assert_eq!(social::map_set_in(&np), Some(set));
    }
    assert_eq!(social::map_set_in("look https://osu.ppy.sh/beatmapsets/1234#taiko/99 ok"), Some(1234));
    assert_eq!(social::map_set_in("osu.ppy.sh/beatmapsets/"), None);
    assert_eq!(social::map_set_in("no link"), None);
}

/// A map just added shows and gets selected whatever the filters, until they change.
#[test]
fn added_maps_show_past_the_filters() {
    let mut s = state();
    let map = |set: usize, title: &str, mode: u8| {
        let e = entry(set, 0, false);
        Entry::new(e.path, beatmap::Meta { title: title.into(), mode, ..e.meta })
    };
    s.library = vec![map(0, "Alpha", 1), map(1, "Beta", 1), map(2, "Gamma", 1), map(3, "Delta", 0)];
    s.select = Select::new(&s.library);
    s.select.filter = "alpha".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.songs().len(), 1);

    let path = s.library[1].path.clone();
    s.select.reveal(&s.library, vec![path.clone()]);
    assert_eq!(s.select.selected(), Some(1));
    assert_eq!(s.select.songs().len(), 2);
    // A set with no taiko difficulties goes past hidden converts too.
    let path = s.library[3].path.clone();
    s.select.reveal(&s.library, vec![path]);
    assert_eq!(s.select.selected(), Some(3));
    assert_eq!(s.select.songs().len(), 3);

    s.select.filter = "gamma".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.songs().len(), 1);
    // Back to the old search: they're gone for good.
    s.select.filter = "alpha".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.songs().len(), 1);
    s.select.hidden.maps.insert(s.library[1].path.clone());
    let path = s.library[1].path.clone();
    s.select.reveal(&s.library, vec![path.clone()]);
    assert_eq!(s.select.songs().len(), 1, "hidden stays hidden");

    // The first of them hidden: the next one gets selected.
    let next = s.library[2].path.clone();
    s.select.reveal(&s.library, vec![path, next]);
    assert_eq!(s.select.selected(), Some(2));

    // Added during a game: listed, and the cursor stays.
    s.select.filter = "a".into();
    s.select.refilter(&s.library);
    s.select.filter = "alpha".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.songs().len(), 1);
    s.select.pin(&s.library, vec![s.library[3].path.clone()]);
    assert_eq!(s.select.songs().len(), 2);
    assert_eq!(s.select.selected(), Some(0));
}

#[test]
fn spectators_are_the_friends_watching_us() {
    let mut s = social::Social::default();
    let friend = |name: &str, activity| online::Friend { name: name.into(), activity };
    s.friends = vec![
        friend("Ann", Some(online::Activity::Watching { name: "ME".into() })),
        friend("Bob", Some(online::Activity::Watching { name: "someone".into() })),
        friend("Cid", Some(online::Activity::Menus)),
        friend("Dee", None),
    ];
    assert!(s.spectators().is_empty(), "not connected");
    s.me = Some("me".into());
    s.connected = true;
    assert_eq!(s.spectators(), ["Ann"]);
}
