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

thread_local! {
    /// How many frames the friends tab was checked on: none would mean it was never looked at.
    static TAB_SEEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn widths() -> &'static [f32] {
    if PHONE.with(std::cell::Cell::get) {
        &PHONE_WIDTHS
    } else {
        &WIDTHS
    }
}

fn entry(set: usize, diff: usize, long: bool) -> Entry {
    let meta = beatmap::Meta {
        title: if long {
            format!("Extremely Long Song Title That Goes On And On (Extended Mix Ver.) {set}")
        } else {
            format!("Song {set}")
        },
        artist: if long {
            "Camellia feat. Hatsune Miku & Some Other Very Long Artist Name".into()
        } else {
            "Artist".into()
        },
        creator: if long {
            "SomeMapperWithALongName".into()
        } else {
            "mapper".into()
        },
        version: if long {
            format!("Hyperdimensional Inner Oni Collab Difficulty {diff}")
        } else {
            format!("Oni {diff}")
        },
        mode: 1,
        od: 5.0,
        hp: 5.0,
        slider_multiplier: 1.4,
        slider_tick_rate: 1.0,
        format_version: 14,
        ..Default::default()
    };
    Entry::new(
        PathBuf::from(format!("/nonexistent/{set} set/{diff}.osu")),
        meta,
    )
}

pub(crate) fn state() -> State {
    let mut cfg = Config {
        check_updates: false,
        mods: Mods::parse("HD,HR,DT,FL,PR,AC,WV,PY,SU,MR"),
        master_volume: 0.0,
        animations: false,
        // Nothing to connect to: the menus show the server as offline instead of asking it.
        online_server: "127.0.0.1:9".into(),
        // Song select's headings, which most of these tests are about (one list is the default).
        song_headings: true,
        ..Config::default()
    };
    if let Some(a) = std::env::var("TAIKO_LAYOUT_ACCENT")
        .ok()
        .and_then(|a| render::parse_accent(&a))
    {
        cfg.accent = a;
    }
    let (tx, rx) = mpsc::channel();
    let library: Vec<Entry> = (0..12)
        .flat_map(|s| (0..3).map(move |d| entry(s, d, s % 2 == 0)))
        .collect();
    tx.send(Scanned::Done(library)).unwrap();
    let mut s = State::new(
        cfg,
        None,
        None,
        PathBuf::from("/nonexistent"),
        Vec::new(),
        Some((rx, Arc::new(AtomicUsize::new(0)))),
        true,
        false,
    );
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
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
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
        // The friends tab shows once logged in, on every screen: logged in, nothing's under it.
        let account = s.mp_menu.account.clone();
        s.mp_menu.account = account.clone().or(Some("Player".into()));
        // At its longest: connected, with everyone's count on it.
        let (connected, players, total) = (
            s.social.connected,
            s.social.players.take(),
            s.social.online_total,
        );
        s.social.connected = true;
        s.social.players = Some(Vec::new());
        s.social.online_total = 12_345;
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
        s.build_frame(&mut c, Instant::now() + Duration::from_millis(32), w);
        s.mp_menu.account = account;
        s.social.connected = connected;
        s.social.players = players;
        s.social.online_total = total;
        if let Some(problems) = debug::edge_tab_problems(&c, &s.hits) {
            TAB_SEEN.with(|n| n.set(n.get() + 1));
            for p in problems {
                out.push(format!("[{what} @{w:.0}] {p}"));
            }
        }
    }
}

/// Like `check`, for something drawn on its own.
fn check_draw(what: &str, out: &mut Vec<String>, mut draw: impl FnMut(&mut Canvas)) {
    for &w in widths() {
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
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
pub(crate) fn session(mapguessr: bool) -> multiplayer::Session {
    // Another test can take the free port found here before the room listens on it: try again.
    let mut s = (0..10)
        .find_map(|_| {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .ok()?
                .local_addr()
                .ok()?
                .port();
            multiplayer::start_host(port, LONG_NAME.into(), mapguessr)
                .recv_timeout(Duration::from_secs(5))
                .ok()?
                .ok()
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
        defense: Some(net::Defense::DEFAULT),
        stats: Some(Box::new(net::MapStats {
            stars: 12.34,
            length_ms: 754_000.0,
            bpm: 222.0,
            od: 10.0,
            hp: 10.0,
            creator: LONG_NAME.into(),
        })),
    });
    // A full lobby with a password.
    s.locked = true;
    s.slots = 16;
    let live = net::Live {
        score: 1_234_567,
        combo: 1234,
        accuracy: 0.9876,
        miss: 12,
        progress: 0.5,
        great: 1234,
        ok: 56,
    };
    let result = net::Final {
        score: 1_234_567,
        accuracy: 0.9876,
        max_combo: 1290,
        great: 1234,
        ok: 56,
        miss: 7,
        ur: 123.4,
        quit: false,
    };
    s.players = (0..16)
        .map(|i| net::PlayerInfo {
            id: i,
            name: format!("{LONG_NAME}{i}"),
            host: i == 0,
            ready: i % 2 == 0,
            has_map: i % 3 != 0,
            status: [
                net::Status::Lobby,
                net::Status::Loading,
                net::Status::Playing,
                net::Status::Finished,
            ][i as usize % 4],
            live: live.clone(),
            result: Some(result.clone()),
            spectator: i == 7,
            points: 12_345 * i,
            guessed: i % 2 == 0,
            tries: (i % 4) as u8,
            maps: (i != 3).then_some(123_456),
            mods: if i % 2 == 0 {
                vec!["HD".into(), "HR".into(), "FL".into(), "PR".into()]
            } else {
                Vec::new()
            },
        })
        .collect();
    s.results = Some(s.players.clone());
    s.contrib = (0..16).map(|i| (i, (99, 9))).collect();
    s.chat = (0..12)
        .map(|i| {
            (
                format!("{LONG_NAME}{i}"),
                "a pretty long chat message that should wrap or get cut somewhere sensible"
                    .to_string(),
            )
        })
        .collect();
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
    s.select.collections = vec![
        (
            "My Favourite Tech Maps For Practice".into(),
            Default::default(),
        ),
        ("dans".into(), Default::default()),
    ];
    check(&mut s, "select", &mut out);
    s.cfg.main_menu = false;
    check(&mut s, "select, legacy menu", &mut out);
    s.open_settings();
    check(&mut s, "settings, legacy menu", &mut out);
    s.close_settings();
    s.cfg.main_menu = true;
    // Each from scratch: the map this state plays once scanned may start loading meanwhile, and
    // that leaves the menu.
    for (i, item) in menu::ITEMS.iter().enumerate() {
        let mut m = menu::Menu::new();
        m.pick(i);
        s.main_menu = Some(m);
        s.scene = Scene::Select;
        check(&mut s, &format!("main menu, {item:?}"), &mut out);
    }
    for paused in [false, true] {
        s.main_menu = Some(menu::Menu::new());
        s.scene = Scene::Select;
        s.music_control(player::Control::Open);
        s.music_paused = paused;
        if paused {
            // Everything long at once: a search, a full queue, the longest labels.
            let p = s.player.as_mut().unwrap();
            p.list.queue.extend(
                (0..30)
                    .map(|i| i % p.list.tracks.len().max(1))
                    .filter(|_| !p.list.tracks.is_empty()),
            );
            p.semitones = -12;
            p.speed = Some(player::Speed::Nightcore);
            p.search("camellia".into());
        }
        check(&mut s, &format!("music player, paused {paused}"), &mut out);
        // The folders, every genre having songs.
        let p = s.player.as_mut().unwrap();
        p.folders_open = true;
        for t in p.list.tracks.iter_mut() {
            t.genres = (1 << genres::GENRES.len()) - 1;
        }
        check(
            &mut s,
            &format!("music player folders, paused {paused}"),
            &mut out,
        );
    }
    s.player = None;
    s.music_paused = false;
    s.main_menu = None;
    s.scene = Scene::Select;
    for page in 0..guide::PAGES {
        s.guide.page = Some(page);
        check(&mut s, &format!("guide page {page}"), &mut out);
    }
    s.guide.page = Some(0);
    s.guide.binding = Some(3);
    check(&mut s, "guide waiting for a key", &mut out);
    s.guide.binding = None;
    s.guide.page = None;
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
    for (share, size) in [
        (ui::LIST_SHARE.0, preview::PREVIEW_SIZE.0),
        (ui::LIST_SHARE.1, preview::PREVIEW_SIZE.0),
        (ui::LIST_SHARE.1, preview::PREVIEW_SIZE.1),
        (ui::LIST_SHARE.0, preview::PREVIEW_SIZE.1),
    ] {
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
            player: if i == 1 {
                LONG_NAME.into()
            } else {
                format!("player{i}")
            },
            map: "A - T [Oni]".into(),
            map_hash: hash,
            score: 1_234_567 - i * 1000,
            accuracy: 0.9876,
            max_combo: 1290,
            miss: 12,
            rate: 1.25,
            mods: ["HD", "HR", "DT", "FL", "PR", "AC", "WV", "SU"]
                .iter()
                .map(|m| m.to_string())
                .collect(),
            pp: Some(456.7),
            date: 1_700_000_000,
            ..Default::default()
        })
        .collect();
    s.cfg.online_user = "player2".into();
    s.online_boards
        .insert((hash, false), (Instant::now(), Ok(online_rows)));
    check(&mut s, "select online scores", &mut out);
    // With Relax on: its board, marked as such, and its total by the name.
    let usual_mods = s.cfg.mods;
    s.cfg.mods = mods::Mods::parse("RX");
    let rows = s.online_boards[&(hash, false)].1.clone();
    s.online_boards.insert((hash, true), (Instant::now(), rows));
    s.me_relax = Some((6_789.1, Some(12_345)));
    check(&mut s, "select online scores, Relax", &mut out);
    s.board_online = false;
    check(&mut s, "select local scores, Relax", &mut out);
    s.board_online = true;
    s.cfg.mods = usual_mods;
    s.board_scroll = 3;
    check(&mut s, "select online scores scrolled", &mut out);
    s.board_scroll = 0;
    s.online_boards.insert(
        (hash, false),
        (
            Instant::now(),
            Err("the server is offline (connection refused)".into()),
        ),
    );
    check(&mut s, "select online scores, server away", &mut out);
    s.online_boards.clear();
    s.board_online = false;
    // Every song has a newer version on osu! (one being downloaded).
    let sets: Vec<u32> = s.library.iter().filter_map(|e| e.set_id()).collect();
    s.select.updates = sets
        .iter()
        .map(|&id| {
            (
                id,
                if id == 2 {
                    ui::SetUpdate::Updating
                } else {
                    ui::SetUpdate::Available
                },
            )
        })
        .collect();
    check(&mut s, "select updates", &mut out);
    s.select.menu = Some(ui::SongMenu {
        row: s.select.cursor(),
        header: true,
        skin: false,
        x: 2000.0,
        y: 300.0,
    });
    check(&mut s, "song menu", &mut out);
    s.select.menu = None;
    s.select.updates.clear();

    s.open_preview();
    check(&mut s, "preview", &mut out);
    s.close_preview();

    s.open_mod_select();
    check(&mut s, "mod select", &mut out);
    // Favourites: more than fit in their row.
    s.cfg.favourite_mods = mods::ALL
        .iter()
        .step_by(3)
        .fold(mods::Mods::NONE, |f, &m| f.flip(m));
    check(&mut s, "mod select favourites", &mut out);
    // Presets: long names, more than fit, one of them the mods on now; and one being named.
    s.cfg.mod_presets = (0..mods::MAX_PRESETS)
        .map(|k| mods::Preset {
            name: if k % 2 == 0 {
                format!("{k} a very long preset name that goes on and on")
            } else {
                "x".into()
            },
            mods: if k == 1 {
                s.cfg.mods
            } else {
                mods::Mods::parse("HD,DT")
            },
            rate: s.cfg.rate,
        })
        .collect();
    check(&mut s, "mod select presets and favourites", &mut out);
    s.cfg.favourite_mods = mods::Mods::NONE;
    s.cfg.mod_presets.truncate(2);
    check(&mut s, "mod select presets", &mut out);
    s.preset_naming = Some("a name that is forty characters long ok!".into());
    check(&mut s, "mod preset naming", &mut out);
    s.preset_naming = None;
    s.cfg.mod_presets.clear();
    check(&mut s, "mod select, no presets", &mut out);
    // A customizable mod on: the Customize button and its tab (Multiverse and the rails conflict).
    let mods_before = s.cfg.mods;
    s.cfg.mods = mods::Mods::parse("MV,SS");
    check(&mut s, "mod select, customizable mod on", &mut out);
    s.open_customization(mods::Mod::Shapeshift);
    check(&mut s, "shapeshift setup", &mut out);
    s.cfg.shapeshift.shapes[2].1 = false;
    s.shapeshift_setup = Some(crate::mod_setup::shapeshift::Setup { row: 3, col: 4 });
    check(&mut s, "shapeshift setup, shapes row", &mut out);
    s.shapeshift_setup = None;
    s.universe_setup = Some(crate::mod_setup::multiverse::Setup::new(3, 7));
    check(&mut s, "multiverse setup", &mut out);
    // Presets with long names (more than fit), one of them the universes now, and own colours
    // being typed.
    s.cfg.universe_presets = [
        "chaos but readable",
        "A Very Long Preset Name Indeed",
        "x",
        "tournament warmup set",
    ]
    .iter()
    .enumerate()
    .map(|(k, name)| multiverse::Preset {
        name: name.to_string(),
        universes: multiverse::shuffle(10 * k as u64 + 2, game::FIRST_FOCUS),
    })
    .collect();
    s.cfg.universes = s.cfg.universe_presets[1].universes;
    s.universe_setup = Some(crate::mod_setup::multiverse::Setup {
        typing: Some(multiverse::Typing::Colours(
            "#ff4020 #40a0ff and some more".into(),
        )),
        ..crate::mod_setup::multiverse::Setup::new(2, 0)
    });
    check(&mut s, "multiverse setup presets", &mut out);
    s.universe_setup = None;
    s.cfg.universe_presets.clear();
    s.mod_select = None;
    s.cfg.mods = mods_before;
    s.cfg.shapeshift = Default::default();

    // A profile: long name, more plays than fit, long map names; then one that failed.
    let play = |i: u64| {
        online::OnlineScore {
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
    }
    };
    let loaded = online::Profile {
        name: LONG_NAME.into(),
        joined: 1_700_000_000,
        plays: 12_345,
        pp: 12_345.6,
        relax: online::RelaxStanding {
            plays: 12_345,
            pp: 6_789.1,
            rank: Some(12_345),
            top: Vec::new(),
        },
        rank: Some(1234),
        top: (0..20).map(play).collect(),
        recent: (0..20).map(play).collect(),
        ..Default::default()
    };
    let (_, rx) = std::sync::mpsc::channel();
    s.profile = Some(profile::View {
        name: LONG_NAME.into(),
        state: profile::State::Loading(rx),
        relax: false,
    });
    check(&mut s, "profile loading", &mut out);
    s.profile = Some(profile::View {
        name: LONG_NAME.into(),
        state: profile::State::Loaded(Box::new(loaded)),
        relax: false,
    });
    check(&mut s, "profile", &mut out);
    if let Some(view) = &mut s.profile {
        view.relax = true;
    }
    check(&mut s, "profile with Relax on", &mut out);
    s.profile = Some(profile::View {
        name: "x".into(),
        state: profile::State::Failed("the server doesn't have profiles yet".into()),
        relax: false,
    });
    check(&mut s, "profile failed", &mut out);
    s.profile = None;

    let row = |i: usize, name: &str, section: u8| online::MedalRow {
        id: format!("m{i}"),
        section,
        tier: (i % 4) as u8 + 1,
        name: Some([name.to_string(), name.to_string(), name.to_string()]),
        how: Some([
            "A very long way of saying how a medal is earned, longer than any there is".to_string(),
            "x".to_string(),
            "x".to_string(),
        ]),
        art: vec!["#".repeat(16); 16],
        at: i.is_multiple_of(2).then_some(1_700_000_000),
        holders: 12,
        have: 3,
        need: 10,
        ..Default::default()
    };
    let view = online::MedalsView {
        players: 21,
        earned: 123,
        medals: (0..135).map(|i| row(i, LONG_NAME, (i % 9) as u8)).collect(),
        collection: vec!["HD".into(), "MV".into()],
        collection_size: 74,
    };
    let (_, rx) = std::sync::mpsc::channel();
    s.medals = Some(medals::View {
        name: Some(LONG_NAME.into()),
        state: medals::State::Loading(rx),
        tab: 0,
        scroll: 0,
    });
    check(&mut s, "medals loading", &mut out);
    for tab in 0..medals::TABS {
        s.medals = Some(medals::View {
            name: Some(LONG_NAME.into()),
            state: medals::State::Loaded(Box::new(view.clone())),
            tab,
            scroll: 0,
        });
        check(&mut s, &format!("medals, tab {tab}"), &mut out);
    }
    s.medals = Some(medals::View {
        name: None,
        state: medals::State::Failed("this server doesn't have medals yet".into()),
        tab: 0,
        scroll: 0,
    });
    check(&mut s, "medals failed", &mut out);
    s.medals = None;

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
    for query in [
        "e",
        "volume",
        "нет такой настройки",
        "a very long search for something that goes on past the end of the box",
    ] {
        s.search_settings(query.into());
        check(&mut s, &format!("settings search {query}"), &mut out);
    }
    s.settings_ui.clear_search();
    s.volume.touch(Instant::now());
    check(&mut s, "volume lane", &mut out);
    // Gone again: on a slow runner (macOS CI) it was still up screens later, over their buttons.
    s.volume = crate::volume::Overlay::new();
    let was = s.mapinfo.take();
    let mut info = mapinfo::MapInfo::spawn(
        vec![(0, PathBuf::from("/nonexistent/a.osu"), 0, 0)],
        1,
        true,
    );
    info.started -= Duration::from_secs(60);
    info.total
        .store(123_456, std::sync::atomic::Ordering::Relaxed);
    s.mapinfo = Some(info);
    check(&mut s, "analysing maps", &mut out);
    s.mapinfo.take().unwrap().hold(false);
    s.mapinfo = was;
    let skins = vec![
        "built-in".into(),
        s.cfg.skin.clone(),
        "- Rafis 2017 Remake (Taiko HD) by somebody".into(),
    ];
    s.settings_ui.choices = Some(settings::Choices::new(
        Item::Skin,
        skins,
        Some(1),
        "2 installed".into(),
    ));
    check(&mut s, "skin picker", &mut out);
    s.settings_ui.choices = None;
    for item in [
        Item::PresentMode,
        Item::AudioBuffer,
        Item::Accent,
        Item::SongsFolder,
        Item::WindowSize,
        Item::LetterboxSize,
        Item::DataFolder,
        Item::InterfaceHides,
    ] {
        s.open_choices(item);
        check(&mut s, &format!("{item:?} list"), &mut out);
        s.settings_ui.choices = None;
    }
    s.cfg.songs_folder = Some(PathBuf::from(
        "/home/someone/.local/share/osu-wine/A Very Long Folder Name For The osu! Install/Songs",
    ));
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
    s.open_hud_editor();
    check(&mut s, "interface editor", &mut out);
    if let Some(h) = s.scene.screen_mut::<screens::hud_editor::HudEdit>() {
        h.ed.select(
            hud::Part::PLACED
                .iter()
                .position(|&p| p == hud::Part::Counts)
                .unwrap(),
        );
    }
    check(&mut s, "interface editor, hit counter", &mut out);
    s.close_hud_editor();
    s.scene = Scene::Select;
    s.start_calibration_as(false);
    check(&mut s, "calibration", &mut out);
    let estimate = calibrate::Estimate {
        hits: 123,
        error_ms: -12.3,
        ur: 150.0,
        margin_ms: 2.5,
        suggested_offset: -45.0,
    };
    s.scene =
        screens::calibrate_done::CalibrateDone::scene(estimate, Box::new(Scene::Select), false);
    check(&mut s, "calibration result", &mut out);
    s.scene = Scene::Select;
    s.start_calibration_as(true);
    check(&mut s, "vsync calibration", &mut out);
    s.scene =
        screens::calibrate_done::CalibrateDone::scene(estimate, Box::new(Scene::Select), true);
    check(&mut s, "vsync calibration result", &mut out);
    s.scene = Scene::Select;

    // Not `open_download`, which would search the mirrors.
    s.scene = crate::screens::download::Download::scene();
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
            diffs: (0..6)
                .map(|d| download::Diff {
                    id: 0,
                    version: format!("Hyperdimensional Inner Oni {d}"),
                    stars: 6.5 + d as f32,
                })
                .collect(),
            play_count: 1_234_567,
            favourite_count: 12_345,
            rating: 9.5,
            ranked_date: "2024-01-01T00:00:00Z".into(),
            custom: None,
        })
        .collect();
    s.browse.query = "a long search query typed into the download tab".into();
    check(&mut s, "download results", &mut out);
    s.scene = crate::screens::mp_menu::MpMenu::scene();
    check(&mut s, "multiplayer menu", &mut out);
    s.crash = Some("--- taiko.rs crash".into());
    check(&mut s, "crash report", &mut out);
    s.crash_sent = true;
    check(&mut s, "crash report sent", &mut out);
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(s.crash.is_none(), "Esc closes it");
    // Signing up with a code, a forgotten password, changing it logged in, and the errors.
    s.mp_menu.open(multiplayer::Form::Register);
    s.mp_menu.user = "MyOwnAccountName".into();
    s.mp_menu.email = "a.rather.long.email.address@example-mail.com".into();
    check(&mut s, "multiplayer menu register", &mut out);
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Register);
    s.mp_menu.code = "123456".into();
    s.mp_status = "code sent to g******@gmail.com: look in spam too if it isn't there".into();
    check(&mut s, "multiplayer menu code", &mut out);
    s.mp_menu.open(multiplayer::Form::Reset);
    check(&mut s, "multiplayer menu forgot", &mut out);
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Reset);
    check(&mut s, "multiplayer menu reset", &mut out);
    s.mp_menu.open(multiplayer::Form::Main);
    s.mp_status = online::network_error(&std::io::Error::from(std::io::ErrorKind::TimedOut));
    s.mp_menu.online = Some(Err(s.mp_status.clone()));
    check(&mut s, "multiplayer menu timed out", &mut out);
    s.mp_status.clear();
    // An old account without an email, with a full lobby list, and linking one.
    s.mp_menu.account = Some("MyOwnAccountName".into());
    s.mp_menu.has_email = Some(false);
    s.mp_menu.rooms = (0..8)
        .map(|i| online::RoomInfo {
            id: i,
            title: format!("{LONG_NAME}'s lobby {i}"),
            host: LONG_NAME.into(),
            players: 3,
            map: Some(entry(0, 0, true).label()),
            in_match: i % 2 == 0,
            mapguessr: false,
            locked: i % 3 == 0,
            slots: 16,
            defense: i % 2 == 1,
        })
        .collect();
    check(&mut s, "multiplayer menu no email", &mut out);
    // A new lobby with a password and 6 players, a mapguessr one, and a locked lobby's prompt.
    s.mp_menu.new_password = "hunter2hunter2hunter2hunter2".into();
    s.mp_menu.new_slots = 6;
    s.mp_menu.select(multiplayer::Row::NewPassword);
    check(&mut s, "multiplayer menu new lobby", &mut out);
    s.mp_menu.new_guessr = true;
    check(&mut s, "multiplayer menu new mapguessr lobby", &mut out);
    s.mp_menu.joining = s.mp_menu.rooms.first().cloned();
    s.mp_menu.join_password = "secret".into();
    s.mp_menu.cursor = 0;
    check(&mut s, "multiplayer menu join locked", &mut out);
    s.mp_menu.joining = None;
    s.mp_menu.new_guessr = false;
    s.mp_menu.open(multiplayer::Form::Link);
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Link);
    check(&mut s, "multiplayer menu linking", &mut out);
    s.mp_menu.has_email = Some(true);
    s.mp_menu.open(multiplayer::Form::Reset);
    s.mp_menu.code_for = Some(multiplayer::CodeFor::Reset);
    check(&mut s, "multiplayer menu change password", &mut out);
    // The account forms are translated: they fit in every language.
    for lang in [crate::lang::Lang::Ru, crate::lang::Lang::Ja] {
        s.cfg.language = lang;
        let code = lang.code();
        s.mp_menu.account = None;
        for form in [
            multiplayer::Form::Main,
            multiplayer::Form::Register,
            multiplayer::Form::Reset,
        ] {
            s.mp_menu.open(form);
            check(
                &mut s,
                &format!("multiplayer menu {form:?} {code}"),
                &mut out,
            );
            s.mp_menu.code_for = Some(if form == multiplayer::Form::Register {
                multiplayer::CodeFor::Register
            } else {
                multiplayer::CodeFor::Reset
            });
            check(
                &mut s,
                &format!("multiplayer menu {form:?} code {code}"),
                &mut out,
            );
        }
        s.mp_menu.open(multiplayer::Form::Main);
        s.mp_menu.account = Some("MyOwnAccountName".into());
        s.mp_menu.has_email = Some(false);
        check(
            &mut s,
            &format!("multiplayer menu lobbies {code}"),
            &mut out,
        );
        s.mp_menu.open(multiplayer::Form::Link);
        check(
            &mut s,
            &format!("multiplayer menu linking {code}"),
            &mut out,
        );
    }
    s.cfg.language = crate::lang::Lang::En;
    s.mp_menu = Default::default();
    s.scene = Scene::Select;

    s.mp_menu.account = Some("MyOwnAccountName".into());
    s.social.open = true;
    s.social.connected = true;
    s.social.me = Some("MyOwnAccountName".into());
    s.social.friends = vec![
        online::Friend {
            name: "FriendWithAVeryLongName".into(),
            activity: Some(online::Activity::Playing {
                map: entry(0, 0, true).label(),
            }),
        },
        online::Friend {
            name: "bob".into(),
            activity: None,
        },
    ];
    check(&mut s, "friends", &mut out);
    s.social.requests = vec!["SomebodyNewWhoWantsIn".into()];
    s.social.sent = vec!["AnotherPersonWithLongName".into(), "and_another_one".into()];
    let line = |from: &str, to: &str| {
        online::ChatLine { from: from.into(), to: to.into(), text: "a pretty long chat message that should wrap or get cut somewhere sensible, with more words".into(), at: 1_700_000_000, medal: None }
    };
    s.social.chats.insert(
        social::key(LONG_NAME),
        (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    line(LONG_NAME, "MyOwnAccountName")
                } else {
                    line("MyOwnAccountName", LONG_NAME)
                }
            })
            .collect(),
    );
    s.social.cursor = 0;
    s.social.input = "a message being typed that is longer than the input box can show".into();
    // The longest name with two group tags in front of it.
    let tags = |t: &[&str]| t.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    s.social
        .set_groups(vec![(LONG_NAME.into(), tags(&["DEV", "200"]))]);
    check(&mut s, "friends chat", &mut out);
    s.social.unread.insert("bob".into(), 12);
    s.social.cursor = 2;
    s.social.open_conversation();
    check(&mut s, "friends conversation", &mut out);
    s.social.close_conversation();
    s.social.tab = social::Tab::Players;
    s.social.online_total = 1234;
    s.social.players = Some(
        (0..40)
            .map(|i| online::OnlinePlayer {
                name: format!("{LONG_NAME}{i}"),
                activity: online::Activity::Playing {
                    map: entry(0, 0, true).label(),
                },
            })
            .collect(),
    );
    check(&mut s, "friends players", &mut out);
    s.social.tab = social::Tab::Chat;
    s.social.global = (0..40)
        .map(|i| line(&format!("{LONG_NAME}{i}"), ""))
        .collect();
    s.social.set_groups(
        (30..40)
            .map(|i| {
                let t: &[&str] = match i % 3 {
                    0 => &["DEV", "200"],
                    1 => &["LOVED"],
                    _ => &["SOMETHING NEW"],
                };
                (format!("{LONG_NAME}{i}"), tags(t))
            })
            .collect(),
    );
    for (i, tier, secret) in [
        (33, 1, false),
        (35, 2, false),
        (36, 3, false),
        (37, 4, false),
        (39, 4, true),
    ] {
        let medal = online::ChatMedal {
            player: LONG_NAME.into(),
            name: [
                LONG_NAME.to_string(),
                LONG_NAME.to_string(),
                LONG_NAME.to_string(),
            ],
            tier,
            secret,
            first: true,
            holders: 1,
            players: 28,
        };
        s.social.global[i] = online::ChatLine {
            from: online::SERVER_NAME.into(),
            medal: Some(medal),
            ..line("", "")
        };
    }
    // A run from one sender, a line calling us by name, and one of ours.
    s.social.global[30] = line(&format!("{LONG_NAME}29"), "");
    s.social.global[34].text = "hey MyOwnAccountName, want to play?".into();
    s.social.global[38] = line("MyOwnAccountName", "");
    s.social.global_unread = 12;
    s.social.input = "a message being typed that is longer than the input box can show".into();
    check(&mut s, "global chat", &mut out);
    // A screenshot in it, and one ready to send.
    let (typed, line_text) = (
        std::mem::take(&mut s.social.input),
        s.social.global[36].text.clone(),
    );
    s.social.global[36].text = shots::shot_url(7);
    s.social.shot_ready = true;
    check(&mut s, "global chat, screenshot", &mut out);
    s.social.shot_ready = false;
    s.social.global[36].text = line_text;
    s.social.input = typed;
    // An admin's tab: who's on, the lobbies and the log, then a lookup over them.
    s.admin = true;
    s.social.admin = true;
    s.social.input.clear();
    let playing = online::Activity::Playing {
        map: entry(0, 0, true).label(),
    };
    s.social.overview = Some(online::AdminOverview {
        uptime_secs: 3 * 86400 + 5 * 3600,
        accounts: 1543,
        sessions: 2210,
        players: (0..5)
            .map(|i| online::AdminPlayer {
                name: format!("{LONG_NAME}{i}"),
                activity: Some(if i % 3 == 0 {
                    playing.clone()
                } else {
                    online::Activity::Menus
                }),
                connections: if i == 1 { 2 } else { 1 },
                watching: None,
                muted_until: (i == 2).then(|| crate::discord::unix_now() + 20 * 60),
            })
            .collect(),
        rooms: vec![online::RoomInfo {
            id: 12,
            title: "A lobby with a very long title, yes".into(),
            host: LONG_NAME.into(),
            players: 8,
            map: Some(entry(0, 0, true).label()),
            in_match: true,
            mapguessr: false,
            locked: true,
            slots: 8,
            defense: true,
        }],
        log: (0..10)
            .map(|i| format!("12:{i:02} auneu muted {LONG_NAME}{i} for 30 minutes"))
            .collect(),
    });
    s.social.set_tab(social::Tab::Admin);
    s.social.cursor = 1;
    check(&mut s, "admin", &mut out);
    s.social.info = Some((
        LONG_NAME.into(),
        vec![
            "online  joined 120d 4h ago  345 plays  email linked".into(),
            "BANNED  loved nominator  muted for 20m".into(),
            format!("same places as: {LONG_NAME}1, {LONG_NAME}2, {LONG_NAME}3"),
        ],
    ));
    s.social.cursor = 0;
    s.social.input = "/mute FriendWithAVeryLongName 2h and a command longer than the box".into();
    check(&mut s, "admin info", &mut out);
    s.social.info = None;
    s.social.input.clear();
    s.social.set_tab(social::Tab::Chat);
    s.social.cursor = 0;
    check(&mut s, "global chat admin", &mut out);
    s.admin = false;
    s.social.admin = false;
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

    let change = |k: usize| {
        changelog::Change {
        date: "2026-10-26".into(),
        title: format!("{k}: pp counter, results screen with pp breakdown and music, online replays, every hotkey rebindable"),
        points: vec!["Live pp counter in the HUD (rosu-pp's gradual calculation on its own thread), hideable".into(), "Clicking a local or online leaderboard score opens its results first".into()],
    }
    };
    s.changelog = Some(changelog::View::loaded((0..12).map(change).collect()));
    check(&mut s, "what's new", &mut out);
    s.changelog = None;

    s.toast(
        "a fairly long notification that tells you something went wrong with the download".into(),
    );
    check(&mut s, "toast", &mut out);
    s.toast = None;

    let mut map = beatmap::calibration_map(500.0, 4, 64);
    map.meta = entry(0, 0, true).meta;
    let game = Play::new(Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        s.cfg.mods,
    ));
    s.play = Some(Box::new(game));
    s.scene = Scene::Play {
        entry: 0,
        from: None,
    };
    check(&mut s, "play", &mut out);
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    check(&mut s, "pause", &mut out);

    let results = |failed: bool| {
        let stats = game::Stats {
            great: 1234,
            ok: 56,
            miss: if failed { 300 } else { 7 },
            max_combo: 1290,
            score: 1_234_567,
            ..Default::default()
        };
        let mut r = ui::Results::new(entry(0, 0, true).label(), stats, game::Windows::new(5.0));
        r.shown = Instant::now() - Duration::from_secs(10);
        r.replay = (!failed).then(|| PathBuf::from("/nonexistent/replay.json"));
        r.rank = (!failed).then_some((1, 12));
        r.watched = (!failed).then(|| LONG_NAME.to_string());
        r.failed = failed;
        r.pp = (!failed).then_some(456.7);
        r.no_pp = failed.then_some("failed plays get no pp");
        // Lots of mods (two rows and a count of the rest) and a speed of its own.
        r.set_mods(
            Mods::parse(
                "HD,HR,DT,FL,PR,AC,WV,PY,SU,MR,SK,TL,FF,SH,CK,PT,LI,DY,MM,OT,BL,ST,SG,LF,PN,MW,SR",
            ),
            if failed { 1.25 } else { 1.0 },
        );
        r.total = (!failed).then_some(ui::Total {
            before: Some(12_333.3),
            after: 12_345.6,
            rank: Some(99_999),
        });
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
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(results(false)), 0, None);
    check(&mut s, "results", &mut out);
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(results(true)), 0, None);
    check(&mut s, "results failed", &mut out);
    let mut opened = results(false);
    (opened.watched, opened.rank, opened.replay, opened.partial) = (None, None, None, true);
    opened.opened = Some(ui::Opened {
        player: LONG_NAME.to_string(),
        date: 0,
        online: true,
    });
    opened.replay_error = Some("that replay isn't on the server".into());
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(opened), 0, None);
    check(&mut s, "results of an online score", &mut out);
    // Scrolled down to the pp.
    for failed in [false, true] {
        let mut r = results(failed);
        (r.scroll, r.scroll_to) = (ui::RESULTS_SCROLL, ui::RESULTS_SCROLL);
        s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
        check(
            &mut s,
            if failed {
                "results failed, performance"
            } else {
                "results, performance"
            },
            &mut out,
        );
    }
    s.mp_menu.account = None;
    s.me = None;
    s.scene = Scene::Select;

    // The video editor (its map still loading): quality, a target size being typed, exporting.
    let json = format!(
        r#"{{"version":1,"map_hash":1,"map_path":"","map_label":"{}","player":"{LONG_NAME}","date":0,"rate":1.0,"anti_mash":false,"score":0,"accuracy":1.0,"max_combo":0,"great":0,"ok":0,"miss":0,"ur":0.0,"inputs":[]}}"#,
        entry(0, 0, true).label()
    );
    let (_video_tx, video_rx) = mpsc::channel();
    let settings = video::Settings {
        size: 4,
        fps: 3,
        blur: 2,
        speed: 1,
        ..Default::default()
    };
    s.scene = screens::video_edit::VideoEdit::scene(
        video::Editor::new(serde_json::from_str(&json).unwrap(), video_rx, settings),
        String::new(),
    );
    fn video(s: &mut State) -> &mut screens::video_edit::VideoEdit {
        s.scene.screen_mut().unwrap()
    }
    check(&mut s, "video editor", &mut out);
    video(&mut s).ed.settings.quality = video::QUALITIES.len() - 1;
    video(&mut s).ed.typing = Some("1234.5".into());
    check(&mut s, "video editor target size", &mut out);
    video(&mut s).ed.typing = None;
    video(&mut s).status = "saved \"Camellia feat. Hatsune Miku - Extremely Long Song Title.mp4\" in the videos folder".into();
    check(&mut s, "video editor saved", &mut out);
    s.scene = Scene::Select;

    // No maps anywhere: the Songs folder button.
    let library = std::mem::take(&mut s.library);
    s.select = Select::new(&s.library);
    check(&mut s, "no maps", &mut out);
    s.select.picking_folder = true;
    check(&mut s, "no maps, choosing", &mut out);
    s.library = library;
    s.select = Select::new(&s.library);

    check_draw("spectators", &mut out, |c| {
        social::draw_spectators(
            c,
            &[
                "Ryuji",
                "a_sixteen_letter",
                "x",
                "y",
                "z",
                "u",
                "v",
                "w",
                "q",
                "r",
            ],
        )
    });

    let room = session(false);
    let player = net::Live {
        score: 1_234_567,
        combo: 1234,
        accuracy: 0.9876,
        miss: 12,
        progress: 0.5,
        great: 1234,
        ok: 56,
    };
    check_draw("lobby", &mut out, |c| {
        multiplayer::draw_lobby(
            c,
            &room,
            false,
            Some(("downloading the map: 45% of 12.3 MB".into(), Some(0.45))),
        )
    });
    check_draw("lobby, asking the host", &mut out, |c| {
        multiplayer::draw_lobby(
            c,
            &room,
            false,
            Some(("asking the host for the map...".into(), None)),
        )
    });
    check_draw("match waiting", &mut out, |c| {
        multiplayer::draw_waiting(c, &room, None, false, Instant::now())
    });
    check_draw("match scoreboard", &mut out, |c| {
        multiplayer::draw_scoreboard(c, &room, &player)
    });
    check_draw("match results", &mut out, |c| {
        multiplayer::draw_results(c, &room)
    });

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
    check_draw("mapguessr lobby", &mut out, |c| {
        mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now())
    });
    room.in_match = true;
    g.test_round(Some(beatmap::calibration_map(250.0, 0, 600)));
    g.tries = 2;
    g.last_guess = Some(format!(
        "{LONG_NAME} - a song with a very long title (TV Size) [Extra Oni]"
    ));
    // A few seconds in, so the notes are on the lane.
    for k in 0..4 {
        g.tick(&room, Instant::now() + Duration::from_secs(k));
    }
    check_draw("mapguessr round", &mut out, |c| {
        mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now())
    });
    g.reveal = Some(mapguessr::Reveal {
        label: format!("{LONG_NAME} - a song with a very long title (TV Size) [Extra Oni]"),
        got: vec![(0, 199), (2, 150)],
        last: true,
    });
    check_draw("mapguessr answer", &mut out, |c| {
        mapguessr::draw_lobby(c, &room, &mut g, 1.0, Instant::now())
    });
    g.reveal = None;
    s.mp = Some(guessr_room());
    s.guessr = Some(g);
    s.picking_lobby_map = true;
    s.scene = Scene::Select;
    check(&mut s, "mapguessr picking", &mut out);

    // The startup warning is translated: it fits in every language.
    for lang in crate::lang::Lang::ALL {
        crate::lang::set(lang);
        check_draw(
            &format!("epilepsy warning {}", lang.code()),
            &mut out,
            |c| splash::draw_warning(c, 1.0, 1.0, 0.5),
        );
    }
    crate::lang::set(crate::lang::Lang::En);

    for p in &out {
        eprintln!("{p}");
    }
    assert!(out.is_empty(), "{} layout problems", out.len());
    assert!(
        TAB_SEEN.with(std::cell::Cell::get) > 100,
        "the friends tab was hardly ever on screen to be checked"
    );
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
    let with: Vec<_> = s
        .library
        .iter()
        .filter(|e| e.meta.set_id == Some(sets[0]))
        .collect();
    assert!(
        with.iter()
            .all(|e| s.select.status.get(&e.path) == Some(&search::GRAVEYARD))
    );
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
    let players: Vec<(u32, String)> = (0..mods.len() as u32)
        .map(|i| (i, format!("player {i}")))
        .collect();
    for &w in widths() {
        let mut games: Vec<(u32, Box<Play>)> = mods
            .iter()
            .enumerate()
            .map(|(i, mods)| {
                let mut map = beatmap::calibration_map(500.0, 4, 64);
                map.meta = entry(0, 0, false).meta;
                let mut g = Play::new(Game::with_mods(
                    map,
                    Arc::new(Track {
                        samples: Vec::new(),
                        rate: 1.0,
                        stream: None,
                    }),
                    None,
                    Mods::parse(mods),
                ));
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
            debug::write_png(
                &std::path::Path::new(&dir).join(format!("spectate_everyone-{w:.0}.png")),
                iw,
                ih,
                &rgb,
            )
            .unwrap();
        }
        assert_eq!(c.clips.len(), games.len());
        for (k, (range, [x, y, cw, ch])) in c.clips.iter().enumerate() {
            let shapes = &c.shapes[range.clone()];
            let inside = shapes
                .iter()
                .filter(|s| {
                    (*x..x + cw).contains(&s.center[0]) && (*y..y + ch + 2.0).contains(&s.center[1])
                })
                .count();
            // Multiverse's other playfields circle off the strip (and are clipped): the one in
            // focus is there.
            let share = if mods[k].contains("MV") { 0.125 } else { 0.9 };
            assert!(
                inside as f32 >= shapes.len() as f32 * share,
                "{} at {w}: {inside} of {} shapes in their strip",
                mods[k],
                shapes.len()
            );
        }
    }
}

#[test]
fn song_select_search_takes_the_keys_only_once_clicked() {
    let mut s = state();
    let press = |s: &mut State, code: KeyCode, text: &str| {
        s.key_inner(code, false, Some(text), Instant::now(), None)
    };
    press(&mut s, KeyCode::KeyA, "a");
    assert_eq!(
        s.select.filter, "",
        "typing does nothing until the search box is clicked"
    );
    s.select.searching = true;
    for (code, text) in [
        (KeyCode::Minus, "-"),
        (KeyCode::Digit4, "4"),
        (KeyCode::Digit5, "5"),
        (KeyCode::BracketLeft, "["),
    ] {
        press(&mut s, code, text);
    }
    assert_eq!(
        s.select.filter, "-45[",
        "in the box every key types, - and [ too"
    );
    press(&mut s, KeyCode::Escape, "");
    assert!(
        !s.select.searching && s.select.filter == "-45[",
        "Escape leaves the box, keeping the search"
    );
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
        s.select
            .songs()
            .iter()
            .map(|song| {
                song.iter()
                    .map(|&i| of(&s.library[i]))
                    .fold(f64::MIN, f64::max)
            })
            .collect::<Vec<f64>>()
    };
    s.select.sort = ui::SortMode::Hp;
    s.select.refilter(&s.library);
    assert!(s.select.songs().len() == 12 && song_values(&s, |e| e.meta.hp).is_sorted());
    s.select.sort = ui::SortMode::Od;
    s.select.refilter(&s.library);
    assert!(song_values(&s, |e| e.meta.od).is_sorted());
    // Status: ranked before loved before pending, the unknown last.
    let codes = [
        search::PENDING,
        search::RANKED,
        search::LOVED,
        search::UNKNOWN,
    ];
    let paths: Vec<PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();
    for (k, p) in paths.iter().enumerate() {
        s.select
            .status
            .insert(p.clone(), codes[(k / 3) % codes.len()]);
    }
    s.select.sort = ui::SortMode::Status;
    s.select.refilter(&s.library);
    let first: Vec<u8> = s
        .select
        .songs()
        .iter()
        .map(|song| s.select.status[&s.library[song[0]].path])
        .collect();
    let rank = |c: u8| {
        [
            search::LOVED,
            search::RANKED,
            search::PENDING,
            search::UNKNOWN,
        ]
        .iter()
        .position(|&x| x == c)
        .unwrap()
    };
    assert!(
        first
            .iter()
            .map(|&c| rank(c))
            .collect::<Vec<_>>()
            .is_sorted(),
        "{first:?}"
    );
    // A song whose difficulties differ in status is listed under its best one, never splitting
    // a heading: the same status doesn't come twice.
    let first_song = s.select.songs()[0].clone();
    for (k, &i) in first_song.iter().enumerate() {
        s.select.status.insert(
            s.library[i].path.clone(),
            if k == 0 {
                search::NOT_SUBMITTED
            } else {
                search::RANKED
            },
        );
    }
    s.select.refilter(&s.library);
    let names: Vec<String> = s.select.sections().into_iter().map(|x| x.0).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "{names:?}");
    // Each status is a section of its own, with its songs counted; every sort has its headings.
    let sections = s.select.sections();
    assert_eq!(
        sections.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
        [
            "Loved",
            "Ranked",
            "Pending / Graveyard",
            "Status not known yet"
        ]
    );
    assert_eq!(
        sections.iter().map(|x| x.1).sum::<usize>(),
        s.select.songs().len()
    );
    for sort in ui::SortMode::ALL {
        s.select.sort = sort;
        s.select.refilter(&s.library);
        let sections = s.select.sections();
        assert!(
            !sections.is_empty()
                && sections.iter().map(|x| x.1).sum::<usize>() == s.select.songs().len(),
            "{sort:?}"
        );
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
    let mut stars = |i: usize, stars: f32| {
        info[i] = Some(crate::mapinfo::Info {
            stars,
            length_ms: 60_000.0,
            bpm: 120.0,
        })
    };
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
    assert_eq!(
        sections[..3]
            .iter()
            .map(|x| (x.0.as_str(), x.1))
            .collect::<Vec<_>>(),
        [
            ("3.00 - 3.99 stars", 1),
            ("4.00 - 4.99 stars", 1),
            ("5.00 - 5.99 stars", 2)
        ]
    );
    assert_eq!(songs.iter().map(Vec::len).sum::<usize>(), total);
    // Other sorts keep a song in one piece.
    s.select.sort = ui::SortMode::Length;
    s.select.refilter(&s.library);
    assert!(
        s.select
            .songs()
            .iter()
            .any(|song| song.contains(&spread[0]) && song.contains(&spread[2]))
    );
}

#[test]
fn without_headings_the_songs_are_one_list_and_by_difficulty_each_stands_alone() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let by_artist = s.select.songs();
    let maps = by_artist.iter().map(Vec::len).sum::<usize>();
    // Stars counting down through the library, so by difficulty its order turns round.
    let mut info = vec![None; s.library.len()];
    for (k, &i) in by_artist.iter().flatten().enumerate() {
        info[i] = Some(crate::mapinfo::Info {
            stars: 10.0 - k as f32 * 0.25,
            length_ms: 60_000.0,
            bpm: 120.0,
        });
    }
    s.select.info = Some(std::sync::Arc::new(std::sync::Mutex::new(info.clone())));
    s.cfg.song_headings = true;
    let on = s.setting_value(Item::SongHeadings);
    s.adjust_setting(Item::SongHeadings, 1, None);
    assert_ne!(s.setting_value(Item::SongHeadings), on);
    assert!(
        !s.select.headings && !s.cfg.song_headings,
        "the setting turns them off and is saved"
    );
    // Songs stay whole, in the same order, without headings: all of them listed.
    assert_eq!(s.select.songs(), by_artist);
    assert_eq!(s.select.sections(), vec![(String::new(), by_artist.len())]);
    assert_eq!(s.select.open_sections(), vec![0]);
    // By difficulty: one row per difficulty, easiest first, whatever its song.
    s.select.set_sort(ui::SortMode::Difficulty, &s.library);
    let rows = s.select.songs();
    assert_eq!(rows.len(), maps);
    assert!(rows.iter().all(|r| r.len() == 1));
    let stars: Vec<f32> = rows.iter().map(|r| info[r[0]].unwrap().stars).collect();
    assert!(stars.is_sorted(), "{stars:?}");
    assert_eq!(s.select.sections().len(), 1);
    let mut problems = Vec::new();
    check_draw("flat list", &mut problems, |c| {
        s.select.draw(c, &s.library, 16.0, "", &|_| None, &|_| None)
    });
    assert!(problems.is_empty(), "{problems:?}");
    // Right and Left go row by row, even between difficulties of one song.
    s.select.enter_song(0, &s.library);
    s.select.key(KeyCode::ArrowRight, &s.library);
    assert_eq!(s.select.selected(), Some(rows[1][0]));
    s.select.key(KeyCode::ArrowLeft, &s.library);
    assert_eq!(s.select.selected(), Some(rows[0][0]));
    // Headings back on: under their star headings again, songs together within them.
    s.adjust_setting(Item::SongHeadings, 1, None);
    assert_eq!(s.setting_value(Item::SongHeadings), on);
    assert!(s.select.headings && s.cfg.song_headings);
    assert!(s.select.sections().len() > 1 && s.select.sections().iter().all(|x| !x.0.is_empty()));
    assert!(s.select.songs().len() < maps);
}

#[test]
fn the_song_headings_setting_is_kept() {
    let mut cfg = crate::config::Config::default();
    assert!(!cfg.song_headings, "one list by default");
    cfg.song_headings = true;
    assert!(crate::config::Config::parse_text(&cfg.to_text()).song_headings);
    assert!(!crate::config::Config::parse_text("").song_headings);
}

#[test]
fn the_sort_button_lists_the_sorts() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let before = s.select.sort;
    s.click(crate::hit::Hit::SortList, None);
    assert_eq!(s.select.sort, before, "the button only opens the list");
    assert_eq!(s.select.picker_len(), ui::SortMode::ALL.len());
    let mut problems = Vec::new();
    check_draw("sort list", &mut problems, |c| {
        s.select.draw_popups(c, &s.library)
    });
    assert!(problems.is_empty(), "{problems:?}");
    let row = ui::SortMode::ALL
        .iter()
        .position(|&m| m == ui::SortMode::Added)
        .unwrap();
    s.click(crate::hit::Hit::PickRow(row), None);
    assert_eq!(s.select.sort, ui::SortMode::Added);
    assert_eq!(s.select.picker, None);
    assert_eq!(s.cfg.sort, "date added");
}

/// Song select sorted by status, its songs under three headings.
fn select_under_headings() -> State {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let paths: Vec<PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();
    for (k, p) in paths.iter().enumerate() {
        s.select.status.insert(
            p.clone(),
            [search::LOVED, search::RANKED, search::PENDING][(k / 3) % 3],
        );
    }
    s.select.sort = ui::SortMode::Status;
    s.select.refilter(&s.library);
    assert!(s.select.sections().len() >= 3);
    s
}

fn draw_select(s: &mut State) {
    check_draw("song select", &mut Vec::new(), |c| {
        s.select
            .draw(c, &s.library, 16.0, "", &|_| None, &|_| Some("SS"))
    });
}

#[test]
fn headings_open_and_close_by_click_and_scrolling_never_changes_them() {
    let mut s = select_under_headings();
    // The heading holding the selection is open, the rest closed.
    assert_eq!(s.select.open_sections(), vec![0]);
    // A click opens another without selecting anything, and closes it again.
    let at = s.select.cursor();
    s.select.click_section(1, &s.library);
    assert_eq!(s.select.open_sections(), vec![0, 1]);
    assert_eq!(s.select.cursor(), at);
    s.select.click_section(1, &s.library);
    assert_eq!(s.select.open_sections(), vec![0]);
    // The open one closes too, with the selection in it, and opens again.
    s.select.click_section(0, &s.library);
    assert!(s.select.open_sections().is_empty());
    draw_select(&mut s);
    s.select.click_section(0, &s.library);
    assert_eq!(s.select.open_sections(), vec![0]);
    // The wheel and right-click scrolling go all the way, and change neither.
    draw_select(&mut s);
    s.select.scroll_by(500, &s.library);
    s.select.scroll_to(1000.0, &s.library);
    s.select.scroll_to(-1000.0, &s.library);
    assert_eq!(s.select.open_sections(), vec![0]);
    assert_eq!(s.select.cursor(), at, "and the selection stays");
    // The arrow keys, past the last song listed, open the next heading and go on into it. It
    // opened for the selection, so it closes behind it.
    for _ in 0..40 {
        let _ = s.select.key(KeyCode::ArrowDown, &s.library);
    }
    let open = s.select.open_sections();
    assert!(
        open.contains(&0),
        "the player's own heading stays open: {open:?}"
    );
    assert!(
        open.len() == 2 && open[1] > 1,
        "only the last heading the keys went into is open too: {open:?}"
    );
    // It all draws.
    draw_select(&mut s);
}

#[test]
fn a_heading_the_player_opened_stays_open_when_the_selection_leaves() {
    let mut s = select_under_headings();
    s.select.click_section(1, &s.library);
    let first_of_next = s.select.sections_end(0) as i64;
    s.select.go_to_song(first_of_next, &s.library);
    assert_eq!(
        s.select.open_sections(),
        vec![1],
        "the first only opened for the selection"
    );
    s.select
        .go_to_song(first_of_next + s.select.sections()[1].1 as i64, &s.library);
    assert_eq!(
        s.select.open_sections(),
        vec![1, 2],
        "the player's stays when the selection leaves"
    );
}

#[test]
fn letters_are_dividers_and_new_ones_start_open() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let n = s.select.sections().len();
    assert_eq!(
        s.select.open_sections(),
        (0..n).collect::<Vec<_>>(),
        "sorted by artist"
    );
    s.select.click_section(0, &s.library);
    assert!(
        s.select.open_sections().len() < n,
        "closing one is up to the player"
    );
}

#[test]
fn a_search_opens_every_heading_and_clearing_it_puts_them_back() {
    let mut s = select_under_headings();
    assert_eq!(s.select.open_sections(), vec![0]);
    s.select.filter = "song".into();
    s.select.refilter(&s.library);
    let n = s.select.sections().len();
    assert_eq!(s.select.open_sections(), (0..n).collect::<Vec<_>>());
    s.select.filter.clear();
    s.select.refilter(&s.library);
    assert_eq!(s.select.open_sections(), vec![0]);
}

#[test]
fn the_list_scrolls_on_its_own_and_comes_back_for_the_selection() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    draw_select(&mut s);
    let (at, top) = (s.select.cursor(), s.select.list_scroll());
    // The wheel moves the list only, and walks past no difficulty.
    s.select.scroll_by(2, &s.library);
    draw_select(&mut s);
    assert_eq!(s.select.cursor(), at);
    assert!(
        s.select.list_scroll() > top,
        "{} vs {top}",
        s.select.list_scroll()
    );
    // Right-dragging to the bottom reaches the end of the whole list.
    s.select.scroll_to(1000.0, &s.library);
    draw_select(&mut s);
    assert_eq!(s.select.cursor(), at);
    assert_eq!(s.select.list_scroll(), s.select.list_end());
    // The heading passing by shows beside the pointer.
    s.select.scrubbing = Some(300.0);
    draw_select(&mut s);
    s.select.scrubbing = None;
    // An arrow key goes back to the selection.
    let _ = s.select.key(KeyCode::ArrowDown, &s.library);
    draw_select(&mut s);
    assert_ne!(s.select.list_scroll(), s.select.list_end());
}

#[test]
fn letting_go_of_the_list_leaves_it_there_and_selects_nothing() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    draw_select(&mut s);
    let at = s.select.cursor();
    s.select.drag_to(-120.0);
    draw_select(&mut s);
    let held = s.select.list_scroll();
    s.select.drag_end(&s.library);
    draw_select(&mut s);
    assert_eq!(s.select.cursor(), at);
    assert_eq!(s.select.list_scroll(), held, "stayed where it was let go");
}

#[test]
fn a_right_click_off_the_rows_does_nothing() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.right_press = Some((700.0, 400.0, None));
    let at = s.select.cursor();
    s.right_button(false);
    assert_eq!(s.select.cursor(), at);
}

#[test]
fn a_guests_mod_select_shows_the_hosts_speed_mods_on() {
    let mut s = state();
    let mut room = session(false);
    // We're player 1; player 0 hosts.
    room.client.id = Some(1);
    room.map.as_mut().unwrap().mods = Mods::parse("DT,HD").list();
    room.map.as_mut().unwrap().defense = None;
    s.mp = Some(room);
    s.scene = crate::screens::lobby::Lobby::scene();
    s.cfg.mods = Mods::parse("FL");
    let shown = s.shown_mods();
    assert!(
        shown.has(mods::Mod::DoubleTime) && shown.has(mods::Mod::Flashlight),
        "the host's speed and our own"
    );
    assert!(
        !shown.has(mods::Mod::Hidden),
        "the host's other mods aren't forced on us"
    );
    // Not in a lobby (or as the host), the mod select shows just ours.
    s.scene = Scene::Select;
    assert_eq!(s.shown_mods(), s.cfg.mods);
}

#[test]
fn in_defense_the_hosts_mods_are_everyones_and_a_guest_cant_change_them() {
    let mut s = state();
    let mut room = session(false);
    room.client.id = Some(1);
    {
        let m = room.map.as_mut().unwrap();
        m.defense = Some(net::Defense::DEFAULT);
        // As the host's game sends them.
        m.mods = Mods::parse("DT,HD,AT").lobby(m.defense).list();
    }
    s.mp = Some(room);
    s.scene = crate::screens::lobby::Lobby::scene();
    s.cfg.mods = Mods::parse("FL");
    let shown = s.shown_mods();
    assert!(shown.has(mods::Mod::Hidden) && shown.has(mods::Mod::Maze));
    assert!(
        !shown.has(mods::Mod::Autoplay),
        "a banned mod never gets to the team"
    );
    assert!(
        !shown.has(mods::Mod::Flashlight),
        "ours stay out of the team's match"
    );
    s.toggle_mod(mods::Mod::Flashlight);
    assert_eq!(s.cfg.mods, Mods::parse("FL"), "a guest can't change them");
    s.toggle_mod(mods::Mod::Hidden);
    assert!(!s.cfg.mods.has(mods::Mod::Hidden));
}

#[test]
fn np_names_the_map_on_screen_with_a_link_others_can_click() {
    let mut s = state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let np = s.now_playing().expect("a map is selected");
    let e = &s.library[s.select.selected().unwrap()];
    assert!(
        np.starts_with("is playing ")
            && np.contains(&e.meta.title)
            && np.contains(&format!("[{}]", e.meta.version)),
        "{np}"
    );
    if let Some(set) = e.meta.set_id {
        assert_eq!(social::map_set_in(&np), Some(set));
    }
    assert_eq!(
        social::map_set_in("look https://osu.ppy.sh/beatmapsets/1234#taiko/99 ok"),
        Some(1234)
    );
    assert_eq!(social::map_set_in("osu.ppy.sh/beatmapsets/"), None);
    assert_eq!(social::map_set_in("no link"), None);
}

/// A map just added shows and gets selected whatever the filters, until they change.
#[test]
fn added_maps_show_past_the_filters() {
    let mut s = state();
    let map = |set: usize, title: &str, mode: u8| {
        let e = entry(set, 0, false);
        Entry::new(
            e.path,
            beatmap::Meta {
                title: title.into(),
                mode,
                ..e.meta
            },
        )
    };
    s.library = vec![
        map(0, "Alpha", 1),
        map(1, "Beta", 1),
        map(2, "Gamma", 1),
        map(3, "Delta", 0),
    ];
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
    let friend = |name: &str, activity| online::Friend {
        name: name.into(),
        activity,
    };
    s.friends = vec![
        friend(
            "Ann",
            Some(online::Activity::Watching { name: "ME".into() }),
        ),
        friend(
            "Bob",
            Some(online::Activity::Watching {
                name: "someone".into(),
            }),
        ),
        friend("Cid", Some(online::Activity::Menus)),
        friend("Dee", None),
    ];
    assert!(s.spectators().is_empty(), "not connected");
    s.me = Some("me".into());
    s.connected = true;
    assert_eq!(s.spectators(), ["Ann"]);
}

#[test]
fn a_map_shows_the_best_letter_of_its_plays() {
    let row = |map_hash, accuracy, miss| scores::ScoreRow {
        map_hash,
        date: 0,
        score: 0,
        accuracy,
        max_combo: 0,
        miss,
        rate: 1.0,
        player: String::new(),
        file: PathBuf::new(),
        mods: String::new(),
        pp: None,
    };
    let rows = [
        row(1, 0.85, 3),
        row(1, 0.97, 0),
        row(1, 0.96, 2),
        row(2, 1.0, 0),
        row(3, 0.91, 1),
    ];
    assert_eq!(scores::best_grade(&rows, 1), Some("S"));
    assert_eq!(scores::best_grade(&rows, 2), Some("SS"));
    assert_eq!(scores::best_grade(&rows, 3), Some("A"));
    assert_eq!(scores::best_grade(&rows, 4), None);
}

#[test]
fn converts_stay_listed_when_the_settings_say_so() {
    let mut s = state();
    let map = |set: usize, mode: u8| {
        let e = entry(set, 0, false);
        Entry::new(e.path, beatmap::Meta { mode, ..e.meta })
    };
    s.library = vec![map(0, 1), map(1, 0)];
    s.select = Select::new(&s.library);
    assert_eq!(s.select.songs().len(), 1);
    s.select.keep_converts(&s.library, false);
    assert_eq!(s.select.songs().len(), 1);
    s.select.keep_converts(&s.library, true);
    assert!(s.select.show_converts);
    assert_eq!(s.select.songs().len(), 2);
}

#[test]
fn empty_update_notes_stay_closed_but_manual_view_stays_open() {
    let mut s = state();
    s.pending_changelog = Some(changelog::View::loaded(Vec::new()));
    s.poll_changelog();
    assert!(s.pending_changelog.is_none());
    assert!(s.changelog.is_none());

    let mut manual = changelog::View::loaded(Vec::new());
    manual.after_update = false;
    s.changelog = Some(manual);
    s.poll_changelog();
    let view = s
        .changelog
        .as_ref()
        .expect("manual empty results remain visible");
    assert!(!view.after_update);
    assert!(matches!(&view.state, changelog::State::Loaded(changes) if changes.is_empty()));
}

#[test]
fn update_notes_wait_for_the_guide_instead_of_covering_it() {
    let mut s = state();
    let change = changelog::parse(
        "fix(audio): align hitsounds\n\nChangelog: Hitsounds stay in sync.",
        "2026-10-26",
    )
    .unwrap();
    s.pending_changelog = Some(changelog::View::loaded(vec![change]));
    s.guide.page = Some(guide::OFFSET_PAGE);
    s.guide.read = [true; guide::PAGES];
    s.poll_changelog();
    assert!(
        s.changelog.is_none(),
        "nothing may cover the guide: its keys would close it unseen"
    );
    assert!(s.pending_changelog.is_some(), "the notes aren't lost");

    s.guide.page = None;
    s.guide.after = Some(guide::OFFSET_PAGE + 1);
    s.poll_changelog();
    assert!(
        s.changelog.is_none(),
        "calibrating from the guide isn't the end of it"
    );

    s.guide.after = None;
    s.poll_changelog();
    assert!(s.changelog.is_some());
}

#[test]
fn update_notes_open_when_ready_without_replacing_a_manual_view() {
    let mut s = state();
    let change = changelog::parse(
        "fix(audio): align hitsounds\n\nChangelog: Hitsounds stay in sync.",
        "2026-10-26",
    )
    .unwrap();
    s.pending_changelog = Some(changelog::View::loaded(vec![change.clone()]));
    s.poll_changelog();
    assert!(s.pending_changelog.is_none());
    assert!(s.changelog.as_ref().is_some_and(|view| view.after_update));

    let mut manual = changelog::View::loaded(Vec::new());
    manual.after_update = false;
    s.changelog = Some(manual);
    s.pending_changelog = Some(changelog::View::loaded(vec![change]));
    s.poll_changelog();
    assert!(s.pending_changelog.is_none());
    assert!(!s.changelog.as_ref().unwrap().after_update);
}

#[test]
fn the_lobby_chat_takes_typed_text_without_firing_hotkeys() {
    let mut s = state();
    let mut room = session(false);
    room.client.id = Some(0);
    s.mp = Some(room);
    s.scene = crate::screens::lobby::Lobby::scene();
    let at = Instant::now();
    let key = |s: &mut State, code, text: Option<&str>| s.key_inner(code, false, text, at, None);

    key(&mut s, KeyCode::KeyT, Some("t"));
    assert_eq!(s.mp.as_ref().unwrap().chat_input.as_deref(), Some(""));
    // Space would toggle ready and digits would step Defense's rules.
    key(&mut s, KeyCode::KeyH, Some("h"));
    key(&mut s, KeyCode::Space, Some(" "));
    key(&mut s, KeyCode::Digit1, Some("1"));
    let mp = s.mp.as_ref().unwrap();
    assert_eq!(mp.chat_input.as_deref(), Some("h 1"));
    assert!(!mp.ready, "Space typed a space");

    // Esc only puts the line away; the lobby stays.
    key(&mut s, KeyCode::Escape, None);
    assert!(s.mp.as_ref().unwrap().chat_input.is_none());
    assert!(s.scene.screen_is::<crate::screens::lobby::Lobby>());

    // Enter sends.
    key(&mut s, KeyCode::KeyT, Some("t"));
    key(&mut s, KeyCode::KeyO, Some("o"));
    key(&mut s, KeyCode::Backspace, None);
    key(&mut s, KeyCode::KeyK, Some("k"));
    assert_eq!(s.mp.as_ref().unwrap().chat_input.as_deref(), Some("k"));
    key(&mut s, KeyCode::Enter, None);
    assert_eq!(
        s.mp.as_ref().unwrap().chat_input.as_deref(),
        Some(""),
        "sent, and the box stays open for the next line"
    );
    assert!(
        s.scene.screen_is::<crate::screens::lobby::Lobby>(),
        "Enter didn't start a match"
    );
    // Enter on an empty line puts it away.
    key(&mut s, KeyCode::Enter, None);
    assert!(s.mp.as_ref().unwrap().chat_input.is_none());

    // A click on the line focuses it; a click elsewhere puts it away.
    s.click(crate::hit::Hit::LobbyChat, None);
    assert!(s.mp.as_ref().unwrap().chat_input.is_some());
    s.click(crate::hit::Hit::ListBack, None);
    assert!(s.mp.as_ref().unwrap().chat_input.is_none());
}

#[test]
fn the_audit_reports_touching_buttons_and_pills_but_not_spaced_ones() {
    use crate::hit::Hit;
    let audit = |draw: &dyn Fn(&mut Canvas)| {
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
        c.clear(640.0);
        c.mouse = (-1000.0, -1000.0);
        draw(&mut c);
        debug::layout_problems(&c)
    };
    let spaced = audit(&|c| {
        c.button(
            20.0,
            100.0,
            60.0,
            20.0,
            "One",
            8.0,
            Hit::Key(KeyCode::KeyA),
            false,
        );
        c.button(
            88.0,
            100.0,
            60.0,
            20.0,
            "Two",
            8.0,
            Hit::Key(KeyCode::KeyB),
            false,
        );
    });
    assert!(spaced.is_empty(), "{spaced:?}");
    let touching = audit(&|c| {
        c.button(
            20.0,
            100.0,
            60.0,
            20.0,
            "One",
            8.0,
            Hit::Key(KeyCode::KeyA),
            false,
        );
        c.button(
            82.0,
            100.0,
            60.0,
            20.0,
            "Two",
            8.0,
            Hit::Key(KeyCode::KeyB),
            false,
        );
    });
    assert!(
        touching.iter().any(|p| p.contains("buttons touch")),
        "{touching:?}"
    );
    let pill = audit(&|c| {
        c.round_rect(
            [20.0, 100.0],
            [60.0, 14.0],
            7.0,
            render::rgba(0x23232f, 1.0),
            render::rgba(0x3a3a4e, 1.0),
            1.0,
        );
        c.hit(20.0, 100.0, 60.0, 14.0, Hit::Key(KeyCode::KeyA));
    });
    assert!(pill.iter().any(|p| p.contains("pill")), "{pill:?}");
    // Plain rows of a list sit one on another by design.
    let rows = audit(&|c| {
        for (k, y) in [100.0, 120.0].into_iter().enumerate() {
            c.round_rect(
                [20.0, y],
                [200.0, 20.0],
                4.0,
                render::rgba(0x23232f, 1.0),
                render::rgba(0, 0.0),
                0.0,
            );
            c.hit(20.0, y, 200.0, 20.0, Hit::SongRow(k));
        }
    });
    assert!(rows.is_empty(), "{rows:?}");
}

/// An installed update restarts the game only where that loses nothing.
#[test]
fn an_update_restarts_the_game_only_in_a_quiet_menu() {
    let mut s = state();
    for scene in [
        Scene::Select,
        crate::screens::download::Download::scene(),
        crate::screens::mp_menu::MpMenu::scene(),
    ] {
        s.scene = scene;
        assert!(s.can_restart());
    }
    s.behind = Some(Box::new(Scene::Select));
    s.scene = crate::screens::settings::SettingsScreen::scene();
    assert!(s.can_restart());
    // The settings over something that's still going.
    s.behind = Some(Box::new(crate::screens::lobby::Lobby::scene()));
    s.scene = crate::screens::settings::SettingsScreen::scene();
    assert!(!s.can_restart());
    s.scene = crate::screens::lobby::Lobby::scene();
    assert!(!s.can_restart());
    s.scene = crate::screens::mp_results::MpResults::scene();
    assert!(!s.can_restart());
}

/// The mod select's presets: S names one of the mods on now and the speed, 1-9 (or a click) put
/// it back, a right-click takes it away; a lobby guest keeps the host's speed.
#[test]
fn mod_presets_are_saved_named_applied_and_removed() {
    let mut s = state();
    // Song select, not straight into the first map (the test state's command-line map).
    s.play_when_scanned = false;
    let key = |s: &mut State, code: KeyCode, text: Option<&str>| {
        s.key_inner(code, false, text, Instant::now(), None);
    };
    s.open_mod_select();
    s.cfg.mods = mods::Mods::parse("HD,PF");
    s.cfg.rate = 1.2;
    key(&mut s, KeyCode::KeyS, None);
    let auto = format!("{} 1.20x", s.cfg.mods.label());
    assert_eq!(s.preset_naming.as_deref(), Some(auto.as_str()));
    // Typing goes to the name, not to the mod select (S, F, R, digits).
    for _ in 0..4 {
        key(&mut s, KeyCode::Backspace, None);
    }
    key(&mut s, KeyCode::KeyR, Some("r"));
    key(&mut s, KeyCode::Digit1, Some("1"));
    assert_eq!(s.cfg.mods, mods::Mods::parse("HD,PF"));
    key(&mut s, KeyCode::Enter, None);
    assert_eq!(s.preset_naming, None);
    assert_eq!(
        s.cfg.mod_presets,
        vec![mods::Preset {
            name: format!("{}r1", &auto[..auto.len() - 4]),
            mods: mods::Mods::parse("HD,PF"),
            rate: 1.2,
        }]
    );
    // Escape leaves the name unsaved and the mod select open.
    key(&mut s, KeyCode::KeyS, None);
    key(&mut s, KeyCode::Escape, None);
    assert!(s.preset_naming.is_none() && s.mod_select.is_some());
    assert_eq!(s.cfg.mod_presets.len(), 1);
    // Applied by its digit.
    s.cfg.mods = mods::Mods::NONE;
    s.cfg.rate = 1.0;
    key(&mut s, KeyCode::Digit1, None);
    assert_eq!((s.cfg.mods, s.cfg.rate), (mods::Mods::parse("HD,PF"), 1.2));
    key(&mut s, KeyCode::Digit2, None);
    assert_eq!(s.cfg.mods, mods::Mods::parse("HD,PF"), "no preset 2");
    // Clicked in the drawn row, and right-clicked away.
    s.cfg.mods = mods::Mods::NONE;
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.0);
    let at = s
        .hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::ModPreset(0))
        .map(|h| h.rect)
        .expect("preset 1 is drawn");
    s.click(crate::hit::Hit::ModPreset(0), None);
    assert_eq!(s.cfg.mods, mods::Mods::parse("HD,PF"));
    assert!(at[2] > 0.0);
    s.remove_mod_preset(0);
    assert!(s.cfg.mod_presets.is_empty());
}

#[test]
fn mod_presets_are_saved_and_applied_with_the_mouse() {
    let mut s = state();
    s.play_when_scanned = false;
    s.open_mod_select();
    s.cfg.mods = mods::Mods::parse("HD,PF");
    let click = |s: &mut State, hit: crate::hit::Hit| {
        s.build_frame(&mut Canvas::default(), Instant::now(), 853.0);
        let r = s
            .hits
            .iter()
            .rev()
            .find(|h| h.hit == hit)
            .map(|h| h.rect)
            .unwrap_or_else(|| panic!("{hit:?} is drawn"));
        // Virtual units to window pixels (the test window is 1080 high).
        let k = s.window_h / render::VIRTUAL_HEIGHT;
        s.mouse = (
            f64::from((r[0] + r[2] / 2.0) * k),
            f64::from((r[1] + r[3] / 2.0) * k),
        );
        s.mouse_button(true, None);
        s.mouse_button(false, None);
    };
    click(&mut s, crate::hit::Hit::ModPresetSave);
    assert!(s.preset_naming.is_some(), "+ Save starts naming a preset");
    // The naming window's Cancel and Save take the mouse.
    click(&mut s, crate::hit::Hit::Key(KeyCode::Escape));
    assert!(s.preset_naming.is_none() && s.mod_select.is_some());
    assert!(s.cfg.mod_presets.is_empty(), "Cancel saves nothing");
    click(&mut s, crate::hit::Hit::ModPresetSave);
    click(&mut s, crate::hit::Hit::Key(KeyCode::Enter));
    assert!(s.preset_naming.is_none());
    assert_eq!(s.cfg.mod_presets.len(), 1, "Save keeps it");
    // A click elsewhere while naming does nothing (no mod toggles under the window).
    click(&mut s, crate::hit::Hit::ModPresetSave);
    s.mouse = (5.0, 5.0);
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert!(s.preset_naming.is_some());
    s.key_inner(KeyCode::Escape, false, None, Instant::now(), None);
    s.cfg.mods = mods::Mods::NONE;
    click(&mut s, crate::hit::Hit::ModPreset(0));
    assert_eq!(s.cfg.mods, mods::Mods::parse("HD,PF"), "a click applies it");
    assert!(s.mod_select.is_some());
}

#[test]
fn deleting_a_mod_preset_asks_first() {
    let mut s = state();
    s.play_when_scanned = false;
    s.open_mod_select();
    s.cfg.mod_presets = vec![mods::Preset {
        name: "HD PF".into(),
        mods: mods::Mods::parse("HD,PF"),
        rate: 1.0,
    }];
    let point_at = |s: &mut State, hit: crate::hit::Hit| {
        s.build_frame(&mut Canvas::default(), Instant::now(), 853.0);
        let r = s
            .hits
            .iter()
            .rev()
            .find(|h| h.hit == hit)
            .map(|h| h.rect)
            .unwrap_or_else(|| panic!("{hit:?} is drawn"));
        let k = s.window_h / render::VIRTUAL_HEIGHT;
        s.mouse = (
            f64::from((r[0] + r[2] / 2.0) * k),
            f64::from((r[1] + r[3] / 2.0) * k),
        );
    };
    // A right-click asks; Keep (a click) leaves it.
    point_at(&mut s, crate::hit::Hit::ModPreset(0));
    s.right_button(true);
    s.right_button(false);
    assert_eq!(s.preset_deleting, Some(0));
    assert_eq!(s.cfg.mod_presets.len(), 1, "nothing goes before the yes");
    point_at(&mut s, crate::hit::Hit::Key(KeyCode::Escape));
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert_eq!(s.preset_deleting, None);
    assert_eq!(s.cfg.mod_presets.len(), 1);
    // Keys while asking go to the question, not the mod select (1 would apply the preset).
    point_at(&mut s, crate::hit::Hit::ModPreset(0));
    s.right_button(true);
    s.right_button(false);
    s.cfg.mods = mods::Mods::NONE;
    s.key_inner(KeyCode::Digit1, false, None, Instant::now(), None);
    assert_eq!(s.cfg.mods, mods::Mods::NONE);
    // Delete (a click) deletes it.
    point_at(&mut s, crate::hit::Hit::Key(KeyCode::Enter));
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert!(s.cfg.mod_presets.is_empty());
    assert_eq!(s.preset_deleting, None);
    assert!(s.mod_select.is_some());
}
