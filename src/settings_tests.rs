//! Typed setting values and the Songs folder setting.

use super::*;

/// The Songs folder and the game data folder are the whole program's: tests that change them
/// take turns.
static FOLDERS: Mutex<()> = Mutex::new(());

#[test]
fn volume_changes_debounce_persistence_and_clamped_turns_do_not_schedule_writes() {
    let mut s = layout_tests::state();
    s.cfg.master_volume = 1.0;
    s.turn_volume(1);
    assert!(s.volume_save_at.is_none());
    s.turn_volume(-1);
    assert!(s.cfg.master_volume < 1.0);
    let first = s.volume_save_at.unwrap();
    s.turn_volume(-1);
    assert!(s.volume_save_at.unwrap() >= first);
    s.cfg.master_volume = 0.0;
    s.volume_save_at = None;
    s.turn_volume(-1);
    assert!(s.volume_save_at.is_none());
}

#[test]
fn ctrl_o_opens_settings_from_anywhere_and_closes_them() {
    let press = |s: &mut State| {
        s.modifiers_ctrl = true;
        s.key_inner(KeyCode::KeyO, false, None, Instant::now(), None);
        s.modifiers_ctrl = false;
    };
    let mut s = layout_tests::state();
    for scene in [
        Scene::Select,
        crate::screens::download::Download::scene(),
        Scene::MpMenu,
    ] {
        s.scene = scene;
        press(&mut s);
        assert!(matches!(s.scene, Scene::Settings { .. }));
        press(&mut s);
        assert!(!matches!(s.scene, Scene::Settings { .. }));
    }
    // Over the main menu too, and back to it.
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    press(&mut s);
    assert!(!s.on_menu() && matches!(s.scene, Scene::Settings { .. }));
    press(&mut s);
    assert!(s.on_menu());
    // Typing a value, it's only a key.
    s.main_menu = None;
    press(&mut s);
    s.settings_ui.editing = Some(settings::Edit::new(Item::Offset, "12".into()));
    press(&mut s);
    assert!(matches!(s.scene, Scene::Settings { .. }));
    assert_eq!(keys::Hotkey::OpenSettings.default().name(), "Ctrl+O");
}

#[test]
fn the_main_menu_sits_over_song_select_and_esc_comes_back_to_it() {
    let esc = |s: &mut State| s.key_inner(KeyCode::Escape, false, None, Instant::now(), None);
    let mut s = layout_tests::state();
    esc(&mut s);
    assert!(s.on_menu(), "Esc in song select");
    assert!(!s.takes_text(), "drum keys aren't a search");
    esc(&mut s);
    assert!(s.quit_asked, "Esc in the menu asks to quit");
    s.quit_asked = false;
    s.cfg.main_menu = false;
    s.main_menu = None;
    esc(&mut s);
    assert!(s.quit_asked && !s.on_menu(), "menu switched off: as before");
    s.quit_asked = false;
    s.cfg.main_menu = true;

    // Download opens and comes back to it; the SONGS tab leaves it for the songs.
    s.main_menu = Some(menu::Menu::new());
    s.menu_open(menu::Item::Download);
    assert!(s.scene.screen_is::<crate::screens::download::Download>());
    s.scene = Scene::Select;
    assert!(s.on_menu());
    s.scene = crate::screens::download::Download::scene();
    s.goto_tab(0);
    assert!(matches!(s.scene, Scene::Select) && !s.on_menu());

    // Picking a lobby map shows the songs, not the menu.
    s.main_menu = Some(menu::Menu::new());
    s.picking_lobby_map = true;
    assert!(!s.on_menu());
}

#[test]
fn the_player_opens_from_the_menu_and_closes_with_it() {
    let key = |s: &mut State, code| s.key_inner(code, false, None, Instant::now(), None);
    let mut s = layout_tests::state();
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    s.music_control(player::Control::Open);
    assert!(s.player.is_some());
    s.player.as_mut().unwrap().move_pitch(3);
    assert!((s.player.as_ref().unwrap().pitch() - 2f64.powf(0.25)).abs() < 1e-9);
    key(&mut s, KeyCode::BracketLeft);
    assert_eq!(
        s.player.as_ref().unwrap().semitones,
        2,
        "[ takes the pitch down"
    );
    s.player
        .as_mut()
        .unwrap()
        .toggle_speed(player::Speed::Nightcore);
    assert_eq!(
        s.preview_rate(),
        (1.5, false),
        "the player's own speed, not the game's"
    );
    assert_eq!(s.cfg.rate, 1.0);
    s.player
        .as_mut()
        .unwrap()
        .toggle_speed(player::Speed::Nightcore);
    key(&mut s, KeyCode::Space);
    assert!(s.music_paused, "Space pauses in the player");
    key(&mut s, KeyCode::KeyF);
    assert!(!s.music_paused, "so does a don key");
    key(&mut s, KeyCode::Escape);
    assert!(
        s.player.is_none() && s.on_menu(),
        "Esc goes back to the menu"
    );
    assert!(!s.quit_asked);

    // Leaving the menu closes it and the music carries on.
    s.music_control(player::Control::Open);
    s.set_music_paused(true);
    s.scene = crate::screens::download::Download::scene();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(s.player.is_none() && !s.music_paused);
}

#[test]
fn the_menus_song_gives_way_to_the_next_when_it_ends() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.play_when_scanned = false;
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    let first = s.select.selected();
    assert!(first.is_some());
    s.preview.song_ended();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let second = s.select.selected();
    assert_ne!(second, first, "the next song");
    assert!(s.preview.from_top, "from its start");
    s.music_step(1);
    assert!(
        !s.preview.from_top,
        "a song picked by hand plays from its preview point"
    );
    let second = s.select.selected();
    // In song select, it just goes round as it always has.
    s.main_menu = None;
    s.preview.song_ended();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.select.selected(), second);
}

#[test]
fn one_end_of_the_menus_song_moves_on_once() {
    let mut s = layout_tests::state();
    let t0 = Instant::now();
    s.build_frame(&mut Canvas::default(), t0, 853.3);
    s.play_when_scanned = false;
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    let first = s.select.selected();
    s.preview.song_ended();
    s.build_frame(&mut Canvas::default(), t0, 853.3);
    let second = s.select.selected();
    assert_ne!(second, first);
    for k in 1..=60u64 {
        s.build_frame(
            &mut Canvas::default(),
            t0 + Duration::from_millis(50 * k),
            853.3,
        );
        assert_eq!(
            s.select.selected(),
            second,
            "still the next song after {k} frames"
        );
        assert!(!s.preview.take_finished(), "no second end after {k} frames");
    }
}

/// A watched friend's play, as the server announces it.
fn watched_play() -> online::SoloPlay {
    online::SoloPlay {
        map: net::MapInfo {
            hash: 1,
            artist: "artist".into(),
            title: "title".into(),
            version: "diff".into(),
            rate: 1.0,
            from: None,
            set_id: None,
            mods: Vec::new(),
            multiverse: None,
            shapeshift: None,
            seed: None,
            defense: None,
            stats: None,
        },
        anti_mash: true,
    }
}

/// A state with the list up from the cache and a scan checking it.
fn checking_library() -> (State, Vec<std::path::PathBuf>) {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let paths: Vec<_> = s.library.iter().map(|e| e.path.clone()).collect();
    assert!(paths.len() > 2, "the test library has maps");
    (s, paths)
}

fn entry_at(path: &std::path::Path, size: u64) -> library::Entry {
    let meta = beatmap::Meta {
        title: "fresh".into(),
        ..Default::default()
    };
    library::Entry::from_parts(path.to_path_buf(), meta, size, 7)
}

#[test]
fn a_checked_list_adds_and_updates_without_moving_what_was_there() {
    let (mut s, paths) = checking_library();
    let mut checked: Vec<_> = paths.iter().map(|p| entry_at(p, 0)).collect();
    for (e, old) in checked.iter_mut().zip(&s.library) {
        (e.size, e.modified) = (old.size, old.modified);
    }
    let changed = entry_at(&paths[1], 999);
    checked[1] = changed;
    checked.push(entry_at(std::path::Path::new("/new/map.osu"), 1));
    let titles: Vec<String> = s.library.iter().map(|e| e.meta.title.clone()).collect();
    s.check_library(checked);
    assert_eq!(
        s.library.len(),
        paths.len() + 1,
        "the new map joined the end"
    );
    assert_eq!(
        s.library[paths.len()].path,
        std::path::Path::new("/new/map.osu")
    );
    for (i, p) in paths.iter().enumerate() {
        assert_eq!(&s.library[i].path, p, "indices stay good");
    }
    assert_eq!(
        s.library[1].meta.title, "fresh",
        "the changed one is updated"
    );
    assert_eq!(
        s.library[0].meta.title, titles[0],
        "the rest aren't touched"
    );
}

#[test]
fn a_map_gone_from_disk_leaves_the_list_only_where_nothing_holds_an_index() {
    let (mut s, paths) = checking_library();
    let rest = |s: &State| -> Vec<library::Entry> {
        s.library[1..]
            .iter()
            .map(|e| library::Entry::from_parts(e.path.clone(), e.meta.clone(), e.size, e.modified))
            .collect()
    };
    let keep = paths[2].clone();
    s.select.select_path(&s.library, &keep);
    let selected = s.select.selected();
    // Somewhere that may hold an index: it waits for the next launch.
    s.scene = Scene::MpMenu;
    let checked = rest(&s);
    s.check_library(checked);
    assert_eq!(s.library.len(), paths.len());
    assert_eq!(s.select.selected(), selected);
    // On song select: it goes, and the selection stays on its map.
    s.scene = Scene::Select;
    let checked = rest(&s);
    s.check_library(checked);
    assert_eq!(s.library.len(), paths.len() - 1);
    let now = s.select.selected().map(|i| s.library[i].path.clone());
    assert_eq!(now, Some(keep));
}

#[test]
fn watching_a_play_of_a_map_being_downloaded_shows_how_far_along_it_is() {
    use std::sync::atomic::AtomicU64;
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let play = watched_play();
    let (_tx, rx) = std::sync::mpsc::channel();
    s.downloads.push(download::Job {
        set: download::SetInfo {
            id: 0,
            artist: play.map.artist.clone(),
            title: play.map.title.clone(),
            creator: String::new(),
            status: String::new(),
            diffs: Vec::new(),
            play_count: 0,
            favourite_count: 0,
            rating: 0.0,
            ranked_date: String::new(),
            custom: None,
        },
        done: Arc::new(AtomicU64::new(25)),
        total: Arc::new(AtomicU64::new(100)),
        rx,
        maps: Vec::new(),
        update: false,
        cancel: Arc::default(),
    });
    let mut v = WatchView::new("bob".into());
    v.play = Some(play);
    v.fetching = true;
    s.scene = Scene::Watch(Box::new(v));
    let mut c = Canvas {
        audit: true,
        ..Default::default()
    };
    s.build_frame(&mut c, Instant::now(), 853.3);
    assert!(
        c.texts.iter().any(|(_, t)| t == "downloading 25%"),
        "{:?}",
        c.texts.iter().map(|t| &t.1).collect::<Vec<_>>()
    );
}

#[test]
fn the_account_button_opens_your_profile_in_the_game() {
    let mut s = layout_tests::state();
    s.mp_menu.account = Some("Player".into());
    s.click(crate::hit::Hit::Account, None);
    assert_eq!(s.profile.as_ref().map(|p| p.name.as_str()), Some("Player"));
    // And the profile offers the website.
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == crate::hit::Hit::Key(winit::keyboard::KeyCode::KeyB))
    );
    assert!(profile_url("Player").ends_with("/u/Player"));
}

#[test]
fn a_song_liked_on_song_select_is_a_favourite_for_the_player_too() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let i = s.preview_target().expect("a song is showing");
    assert!(!s.select.is_liked(&s.library[i]));
    s.click(crate::hit::Hit::SelectLike, None);
    assert!(s.select.is_liked(&s.library[i]));
    assert!(
        s.select
            .favourites
            .contains(&playlist::favourite_key(&s.library[i]))
    );
    s.click(crate::hit::Hit::SelectLike, None);
    assert!(!s.select.is_liked(&s.library[i]));
}

/// A state on song select whose selected map's folder carries a skin image, with the song menu
/// open on it as a right click would. Returns the folder.
fn on_a_map_with_a_skin(s: &mut State) -> std::path::PathBuf {
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let i = s.select.selected().unwrap();
    let dir = std::env::temp_dir().join(format!("taiko-rs-skin-toggle-{}", std::process::id()));
    let dir = dir.join(format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::new(2, 2)
        .save(dir.join("Taiko-Roll-End.png"))
        .unwrap();
    s.library[i].path = dir.join("map.osu");
    s.select.menu = Some(ui::SongMenu {
        row: s.select.cursor(),
        header: true,
        skin: s.menu_has_skin(s.select.cursor()),
        x: 100.0,
        y: 100.0,
    });
    dir
}

fn map_skin_item(s: &State) -> Option<(usize, String)> {
    s.select
        .menu_items(&s.library)
        .into_iter()
        .enumerate()
        .find(|(_, (_, a))| *a == ui::MenuAction::MapSkin)
        .map(|(k, (label, _))| (k, label))
}

#[test]
fn a_maps_skin_is_turned_off_and_back_on_from_its_menu() {
    let mut s = layout_tests::state();
    let dir = on_a_map_with_a_skin(&mut s);
    s.follow_map_skin();
    assert_eq!(s.skin_map.as_ref(), Some(&dir));

    let (k, label) = map_skin_item(&s).unwrap();
    assert!(label.starts_with("Turn off"), "{label}");
    s.click(crate::hit::Hit::SongMenu(k), None);
    assert!(s.select.menu.is_none());
    s.follow_map_skin();
    assert_eq!(s.skin_map, None);

    s.select.menu = Some(ui::SongMenu {
        row: s.select.cursor(),
        header: false,
        skin: s.menu_has_skin(s.select.cursor()),
        x: 100.0,
        y: 100.0,
    });
    let (k, label) = map_skin_item(&s).unwrap();
    assert!(label.starts_with("Use"), "{label}");
    s.click(crate::hit::Hit::SongMenu(k), None);
    s.follow_map_skin();
    assert_eq!(s.skin_map.as_ref(), Some(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn beatmap_skins_off_in_settings_covers_every_map_and_its_menu() {
    let mut s = layout_tests::state();
    let dir = on_a_map_with_a_skin(&mut s);
    s.follow_map_skin();
    assert_eq!(s.skin_map.as_ref(), Some(&dir));

    s.cfg.map_skins = false;
    s.follow_map_skin();
    assert_eq!(s.skin_map, None);
    // Nothing for one map's switch to do while they're all off.
    assert!(!s.menu_has_skin(s.select.cursor()));

    s.cfg.map_skins = true;
    s.follow_map_skin();
    assert_eq!(s.skin_map.as_ref(), Some(&dir));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_map_without_a_skin_offers_no_switch() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    let row = s.select.cursor();
    assert!(!s.menu_has_skin(row));
    s.select.menu = Some(ui::SongMenu {
        row,
        header: true,
        skin: s.menu_has_skin(row),
        x: 100.0,
        y: 100.0,
    });
    assert!(map_skin_item(&s).is_none());
}

#[test]
fn beatmap_skins_stay_off_after_a_restart() {
    let mut cfg = config::Config::default();
    assert!(cfg.map_skins);
    cfg.map_skins = false;
    assert!(!config::Config::parse_text(&cfg.to_text()).map_skins);
}

#[test]
fn the_screen_glitches_between_menus_only_and_only_with_the_setting_on() {
    let mut s = layout_tests::state();
    s.cfg.animations = true;
    s.play_when_scanned = false;
    let at = Instant::now();
    let glitch = |s: &mut State, ms: u64| {
        let mut c = Canvas::default();
        s.glitch_between_menus(&mut c, at + Duration::from_millis(ms));
        c.glitch
    };
    s.main_menu = None;
    s.scene = Scene::Select;
    assert_eq!(glitch(&mut s, 0), None, "the first screen just appears");
    assert_eq!(glitch(&mut s, 1), None);

    s.scene = crate::screens::download::Download::scene();
    assert!(glitch(&mut s, 2).is_some_and(|[strength, _]| strength > 0.9));
    assert!(glitch(&mut s, 100).is_some(), "the same change carries on");
    assert_eq!(glitch(&mut s, 2000), None, "and it's over");

    // In and out of a song is no change between menus.
    s.scene = std::mem::replace(&mut in_a_song(false).scene, Scene::Select);
    assert_eq!(glitch(&mut s, 3000), None);
    s.scene = Scene::Select;
    assert_eq!(glitch(&mut s, 3001), None);

    s.cfg.menu_glitch = false;
    s.scene = crate::screens::download::Download::scene();
    assert_eq!(glitch(&mut s, 4000), None);
}

#[test]
fn the_menu_glitch_setting_stays_off_after_a_restart() {
    let mut cfg = config::Config::default();
    assert!(cfg.menu_glitch);
    // A config from before the setting has it on too.
    assert!(config::Config::parse_text("animations = true\n").menu_glitch);
    cfg.menu_glitch = false;
    assert!(!config::Config::parse_text(&cfg.to_text()).menu_glitch);
}

#[test]
fn the_exit_question_slides_in_from_above_and_stops() {
    let mut s = layout_tests::state();
    s.cfg.animations = true;
    s.ask_quit();
    let at = s.quit_asked_at.unwrap();
    assert_eq!(s.quit_slide(at), 0.0);
    let early = s.quit_slide(at + Duration::from_millis(40));
    assert!(early > 0.4 && early < 1.0, "{early}");
    assert_eq!(s.quit_slide(at + Duration::from_millis(200)), 1.0);
    s.cfg.animations = false;
    assert_eq!(s.quit_slide(at), 1.0);
}

#[test]
fn escape_after_the_last_note_goes_straight_to_the_results() {
    let mut s = in_a_song(false);
    let mut at = Instant::now();
    // With notes to come it's the pause menu, as ever.
    s.key(KeyCode::Escape, false, None, at, None);
    let Scene::Play { game, .. } = &mut s.scene else {
        panic!("still playing");
    };
    assert!(game.paused().is_some());
    game.set_paused(None);
    game.run_out_of_notes();

    s.key(KeyCode::Escape, false, None, at, None);
    let Scene::Play { game, .. } = &s.scene else {
        panic!("the results come with the next frame");
    };
    assert!(game.paused().is_none(), "not the pause menu");
    at += Duration::from_millis(16);
    s.build_frame(&mut Canvas::default(), at, 853.3);
    assert!(matches!(s.scene, Scene::Results { .. }));
}

#[test]
fn a_replay_still_pauses_after_its_last_note() {
    let mut s = in_a_song(false);
    let Scene::Play { game, .. } = &mut s.scene else {
        panic!("playing");
    };
    game.set_replay(Some((Vec::new(), 0)));
    game.run_out_of_notes();
    assert!(!game.can_skip_outro());
}

#[test]
fn a_songs_menu_likes_it() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.select.menu = Some(ui::SongMenu {
        row: s.select.cursor(),
        header: true,
        skin: false,
        x: 100.0,
        y: 100.0,
    });
    let i = s.select.menu_map().unwrap();
    let items = s.select.menu_items(&s.library);
    assert!(items[0].0.starts_with("Like") && items[0].1 == ui::MenuAction::Like);
    s.click(crate::hit::Hit::SongMenu(0), None);
    assert!(s.select.is_liked(&s.library[i]));
    assert!(s.select.menu.is_none());
}

#[test]
fn a_screenshot_opens_over_everything_and_any_click_or_key_closes_it() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.shot_view = Some(ShotView { rx: Job::idle() });
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    assert!(s.hits.is_empty(), "nothing behind it can be clicked");
    assert!(s.frame_clip.is_none());
    s.mouse_button(true, None);
    assert!(s.shot_view.is_none());

    s.shot_view = Some(ShotView { rx: Job::idle() });
    s.key_inner(
        winit::keyboard::KeyCode::Escape,
        false,
        None,
        Instant::now(),
        None,
    );
    assert!(s.shot_view.is_none());
    assert!(matches!(s.scene, Scene::Select), "the key only closed it");
}

#[test]
fn a_screenshot_needs_taking_and_logging_in_to_send() {
    let mut s = layout_tests::state();
    s.send_shot(social::Row::Global);
    assert!(s.shot_upload.is_none(), "not logged in");
    s.cfg.online_server = online::DEFAULT_SERVER.into();
    s.cfg.online_token = "t".into();
    s.send_shot(social::Row::Global);
    assert!(s.shot_upload.is_none(), "nothing taken yet");
    s.last_shot = Some(Arc::new((1, 1, vec![0, 0, 0])));
    s.send_shot(social::Row::AddFriend);
    assert!(s.shot_upload.is_none(), "not a chat");
}

#[test]
fn a_watched_play_is_watched_to_its_end_before_the_view_waits_for_the_next() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let mut v = WatchView::new("bob".into());
    v.play = Some(watched_play());
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    v.game = Some(Box::new(Play::new(Game::new(map, track, None))));
    v.clock = (30_000.0, Instant::now(), false);
    s.scene = Scene::Watch(Box::new(v));

    // They stop: we're seconds behind them, so the view plays on rather than going blank.
    s.social_down(online::Down::WatchEnd {
        player: "bob".into(),
    });
    let Scene::Watch(v) = &mut s.scene else {
        panic!("still watching")
    };
    assert!(
        v.game.is_some() && v.ending.is_some(),
        "the end is still to be watched"
    );
    // Their next play starts meanwhile: it waits its turn.
    s.social_down(online::Down::WatchStart {
        player: "bob".into(),
        play: watched_play(),
        t: 0.0,
        paused: false,
        inputs: Vec::new(),
    });
    let Scene::Watch(v) = &mut s.scene else {
        panic!("still watching")
    };
    assert!(v.game.is_some() && v.next.is_some());
    // Once the view has got to where they stopped it moves on to that play.
    v.game.as_mut().unwrap().put_at(30_000.0);
    s.watch_tick();
    let Scene::Watch(v) = &s.scene else {
        panic!("still watching")
    };
    assert!(v.game.is_none() && v.play.is_some() && v.next.is_none() && v.ending.is_none());
}

#[test]
fn a_spectator_sees_the_last_seconds_before_the_results() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let view = |games: Vec<(u32, Box<Play>)>| {
        Scene::Spectate(Box::new(SpectateView {
            rx: Job::idle(),
            loaded: None,
            start: Instant::now(),
            from: None,
            rate: 1.0,
            mods: Mods::NONE,
            map: watched_play().map,
            players: vec![(1, "a".into())],
            games,
            focus: Some(0),
            pending: std::collections::HashMap::new(),
            skipped: std::collections::HashSet::new(),
            skip_done: false,
            zones: Vec::new(),
            zones_at: None,
            ended: None,
        }))
    };
    let game = || {
        let map = beatmap::calibration_map(500.0, 0, 8);
        let track = Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        });
        Box::new(Play::new(Game::new(map, track, None)))
    };
    s.mp = Some(layout_tests::session(false));
    // Joined late, nothing on screen yet: straight to the results.
    s.scene = view(Vec::new());
    s.handle_server_msg(net::ServerMsg::Results(Vec::new()));
    assert!(matches!(s.scene, Scene::MpResults));
    // Watching: the room is over, but the view is `SPECTATE_DELAY` behind.
    s.scene = view(vec![(1, game())]);
    s.handle_server_msg(net::ServerMsg::Results(Vec::new()));
    let Scene::Spectate(v) = &mut s.scene else {
        panic!("the last seconds are still to be watched")
    };
    assert!(v.ended.is_some());
    v.ended = Some(Instant::now() - SPECTATE_DELAY - Duration::from_millis(600));
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(matches!(s.scene, Scene::MpResults), "then the results");
}

#[test]
fn applying_the_offset_suggestion_has_no_key_until_given_one() {
    use keys::{Bindings, Chord, Hotkey, Place, Resolved};
    let (o, shift_o) = (
        Chord::plain(KeyCode::KeyO),
        Chord {
            shift: true,
            ..Chord::plain(KeyCode::KeyO)
        },
    );
    let mut b = Bindings::default();
    assert_eq!(b.get(Hotkey::ApplyOffset), None);
    assert_eq!(
        b.resolve(o, Place::Play),
        Resolved::Nothing,
        "O does nothing in a song"
    );
    assert_eq!(b.resolve(o, Place::Paused), Resolved::Nothing);
    assert!(
        !b.config_lines().contains("apply_offset"),
        "nothing to write down"
    );
    // Given a key, it comes back through the config, O or any other.
    b.set(Hotkey::ApplyOffset, Some(o));
    assert_eq!(b.resolve(o, Place::Play), Resolved::Same);
    assert!(b.config_lines().contains("hotkey.apply_offset = O"));
    let mut again = Bindings::default();
    again.parse_line("apply_offset", "O");
    assert_eq!(again.get(Hotkey::ApplyOffset), Some(o));

    // Without a key, nothing reaches it: not Shift+O or a match, which skip the hotkey.
    let mut s = layout_tests::state();
    let before = (s.cfg.offset_ms, s.map_offsets.len());
    s.modifiers_shift = true;
    s.apply_live_offset();
    assert_eq!((s.cfg.offset_ms, s.map_offsets.len()), before);
    assert!(s.toast.is_none());
    assert_eq!(
        s.cfg.hotkeys.resolve(shift_o, Place::Play),
        Resolved::Same,
        "why the check is in the handler"
    );
}

#[test]
fn a_maps_offset_goes_by_with_its_song_and_the_global_one_doesnt() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.cfg.mods = Mods::NONE;
    s.cfg.set_osu_offset(2.0);
    assert_eq!(s.cfg.offset_ms, -25.0);
    let entry = s.select.selected().expect("a map");
    // The osu! way: -30 takes this map's notes 30 ms later.
    s.set_map_offset(entry, -30.0);

    s.cfg.rate = 1.0;
    s.refresh_play_offset();
    assert_eq!(s.play_offset, 5.0);
    // At 1.5x the song's 30 ms pass in 20: the machine's 25 stay 25.
    s.cfg.rate = 1.5;
    s.refresh_play_offset();
    assert_eq!(s.play_offset, -5.0);
    // Double Time goes by its own rate.
    s.cfg.rate = 1.0;
    s.cfg.mods = Mods::parse("DT");
    s.refresh_play_offset();
    assert_eq!(s.play_offset, -5.0);
}

#[test]
fn plus_on_a_map_moves_its_notes_the_way_plus_on_the_global_offset_does() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.cfg.mods = Mods::NONE;
    s.cfg.rate = 1.0;
    s.cfg.set_osu_offset(0.0);
    s.refresh_play_offset();
    let start = s.play_offset;
    s.cfg.set_osu_offset(5.0);
    s.refresh_play_offset();
    let global = s.play_offset - start;
    s.cfg.set_osu_offset(0.0);
    let entry = s.select.selected().expect("a map");
    s.set_map_offset(entry, 5.0);
    assert_eq!(s.play_offset - start, global);
    assert_eq!(
        config::osu_offset_of(s.play_offset, s.cfg.offset_latency_ms),
        5.0,
        "the total is the two added up"
    );
}

#[test]
fn map_offsets_reset_only_on_the_second_press() {
    let _folders = FOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = layout_tests::state();
    assert!(!s.reset_is_armed());
    s.press_reset_map_offsets();
    assert!(!s.reset_is_armed(), "none to reset: nothing to ask");
    s.map_offsets.insert(1, 12.0);
    s.map_offsets.insert(2, -7.0);
    s.press_reset_map_offsets();
    assert_eq!(s.map_offsets.len(), 2, "the first press only asks");
    assert!(s.reset_is_armed());
    s.reset_armed = Some(Instant::now() - RESET_CONFIRM);
    s.press_reset_map_offsets();
    assert_eq!(s.map_offsets.len(), 2, "too long after: asks again");
    s.press_reset_map_offsets();
    assert!(s.map_offsets.is_empty());
    assert!(!s.reset_is_armed());
}

#[test]
fn the_guide_turns_its_pages_and_takes_the_keys() {
    let key = |s: &mut State, code| s.key_inner(code, false, None, Instant::now(), None);
    let mut s = layout_tests::state();
    s.cfg.guide_seen = true;
    s.guide.page = Some(0);
    key(&mut s, KeyCode::F2);
    assert_eq!(s.guide.page, Some(0), "other keys wait");
    key(&mut s, KeyCode::KeyC);
    assert_eq!(s.guide.page, Some(0), "C only calibrates on its page");
    key(&mut s, KeyCode::ArrowLeft);
    assert_eq!(s.guide.page, Some(0), "nothing before the first");
    key(&mut s, KeyCode::Enter);
    assert_eq!(s.guide.page, Some(guide::OFFSET_PAGE));
    for _ in 1..guide::PAGES {
        assert!(s.guide.page.is_some());
        key(&mut s, KeyCode::ArrowRight);
    }
    assert_eq!(s.guide.page, None, "past the last it closes");
    s.guide.page = Some(3);
    key(&mut s, KeyCode::Escape);
    assert_eq!(s.guide.page, None);
    assert!(!s.quit_asked, "Esc only closes it");
}

#[test]
fn the_guide_counts_as_seen_once_closed_and_old_configs_havent_seen_it() {
    assert!(
        !Config::default().guide_seen,
        "a config without the line: not seen"
    );
    let mut s = layout_tests::state();
    s.guide.read[guide::OFFSET_PAGE] = true;
    s.guide.page = Some(2);
    s.guide_action(guide::Action::Next);
    assert!(!s.cfg.guide_seen, "still reading");
    s.guide_action(guide::Action::Close);
    assert!(s.cfg.guide_seen);
}

/// As if the page has been up for `ms`.
fn guide_up_for(s: &mut State, ms: u64) {
    s.guide.shown = s
        .guide
        .page
        .map(|p| (p, Instant::now() - Duration::from_millis(ms)));
    s.tick_guide();
}

#[test]
fn a_new_player_cant_leave_the_guide_without_reading_it() {
    let key = |s: &mut State, code| s.key_inner(code, false, None, Instant::now(), None);
    let mut s = layout_tests::state();
    s.guide.page = Some(0);
    s.tick_guide();
    for code in [KeyCode::Escape, KeyCode::Enter] {
        key(&mut s, code);
        assert_eq!(s.guide.page, Some(0), "{code:?} waits");
    }
    s.guide_action(guide::Action::Page(3));
    assert_eq!(s.guide.page, Some(0), "so do the dots");
    assert!(!s.cfg.guide_seen);

    guide_up_for(&mut s, 5000);
    key(&mut s, KeyCode::Escape);
    assert_eq!(
        s.guide.page,
        Some(guide::OFFSET_PAGE),
        "skipping still shows the offset"
    );
    s.tick_guide();
    key(&mut s, KeyCode::Escape);
    assert_eq!(s.guide.page, Some(guide::OFFSET_PAGE), "and it waits too");
    key(&mut s, KeyCode::ArrowLeft);
    assert_eq!(s.guide.page, Some(0), "going back is free");
    key(&mut s, KeyCode::Enter);
    assert_eq!(
        s.guide.page,
        Some(guide::OFFSET_PAGE),
        "a page already read doesn't wait again"
    );

    guide_up_for(&mut s, 5000);
    key(&mut s, KeyCode::Escape);
    assert_eq!(s.guide.page, None);
    assert!(s.cfg.guide_seen);
}

#[test]
fn the_guide_can_calibrate_straight_away_and_doesnt_wait_when_seen_before() {
    let mut s = layout_tests::state();
    s.guide.page = Some(guide::OFFSET_PAGE);
    s.tick_guide();
    s.guide_action(guide::Action::Calibrate);
    assert_eq!(
        s.guide.page, None,
        "the hold is on leaving, not on calibrating"
    );

    let mut s = layout_tests::state();
    s.cfg.guide_seen = true;
    s.guide.page = Some(0);
    s.tick_guide();
    s.guide_action(guide::Action::Close);
    assert_eq!(s.guide.page, None);
}

#[test]
fn the_clock_of_the_guide_waits_for_the_splash() {
    let mut s = layout_tests::state();
    s.guide.page = Some(0);
    s.splash = splash::Splash::new(Some(48000), true, false);
    assert!(s.splash.is_some());
    s.tick_guide();
    assert_eq!(s.guide.shown, None);
}

#[test]
fn the_guide_rebinds_the_drum_like_settings_do() {
    let key = |s: &mut State, code| s.key_inner(code, false, None, Instant::now(), None);
    let mut s = layout_tests::state();
    s.guide.page = Some(0);
    let d = s.cfg.keys;
    s.guide_action(guide::Action::Bind(1));
    key(&mut s, KeyCode::ShiftLeft);
    assert_eq!(s.guide.binding, Some(1), "a modifier isn't a drum key");
    key(&mut s, KeyCode::Escape);
    assert_eq!(
        (s.cfg.keys, s.guide.binding, s.guide.page),
        (d, None, Some(0)),
        "Esc keeps the key, and the guide"
    );
    s.guide_action(guide::Action::Bind(1));
    key(&mut s, KeyCode::KeyG);
    assert_eq!(s.cfg.keys[1], KeyCode::KeyG);
    assert_eq!(s.guide.page, Some(0), "still on its page");
    // A key that's another drum's swaps with it.
    key(&mut s, KeyCode::Digit1);
    key(&mut s, KeyCode::KeyG);
    assert_eq!((s.cfg.keys[0], s.cfg.keys[1]), (KeyCode::KeyG, d[0]));
    // What Settings refuses, this does too.
    s.guide_action(guide::Action::Bind(2));
    let preset = s.cfg.preset_keys[0];
    key(&mut s, preset);
    assert_eq!(s.cfg.keys[2], d[2]);
}

#[test]
fn calibrating_from_the_guide_comes_back_to_it() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.play_when_scanned = false;
    s.scene = Scene::Select;
    s.guide.page = Some(guide::OFFSET_PAGE);
    s.guide_action(guide::Action::Calibrate);
    assert_eq!(s.guide.page, None);
    assert_eq!(s.guide.after, Some(guide::OFFSET_PAGE + 1));
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(matches!(s.scene, Scene::Calibrate { .. }));
    assert_eq!(s.guide.page, None, "not over the calibration");
    // Its results, as they really end: out the way the player leaves them.
    let estimate = calibrate::Estimate {
        hits: 40,
        error_ms: 3.0,
        ur: 90.0,
        margin_ms: 2.0,
        suggested_offset: 12.0,
    };
    let Scene::Calibrate { back, .. } = std::mem::replace(&mut s.scene, Scene::Select) else {
        unreachable!()
    };
    s.scene = screens::calibrate_done::CalibrateDone::scene(estimate, back, false);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.guide.page, None, "not over its results either");
    s.leave_calibration(true);
    assert_eq!(s.cfg.offset_osu_ms, 12.0);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(
        s.guide.page,
        Some(guide::OFFSET_PAGE + 1),
        "back on the next page"
    );
}

#[test]
fn typed_numbers_take_units_and_commas() {
    assert_eq!(parse_number("1.25x"), Some(1.25));
    assert_eq!(parse_number(" -12 ms"), Some(-12.0));
    assert_eq!(parse_number("80%"), Some(80.0));
    assert_eq!(parse_number("0,75"), Some(0.75));
    assert_eq!(parse_number("unlimited"), Some(0.0));
    assert_eq!(parse_number("fast"), None);
    assert_eq!(parse_number(""), None);
}

#[test]
fn typed_values_are_clamped_and_bad_ones_refused() {
    let mut s = layout_tests::state();
    s.apply_typed(Item::ScrollSpeed, "9").unwrap();
    assert_eq!(s.cfg.scroll_speed, 4.0);
    s.apply_typed(Item::Rate, "1.337").unwrap();
    assert_eq!(s.cfg.rate, 1.34);
    s.apply_typed(Item::Offset, "-23.4 ms").unwrap();
    assert_eq!(s.cfg.offset_osu_ms, -23.0);
    s.apply_typed(Item::MusicVolume, "35%").unwrap();
    assert!((s.cfg.music_volume - 0.35).abs() < 1e-6);
    s.apply_typed(Item::ResumeDelay, "off").unwrap();
    assert_eq!(s.cfg.resume_delay_ms, 0.0);
    s.apply_typed(Item::FpsCap, "5").unwrap();
    assert_eq!(s.cfg.fps_cap, 30);
    s.apply_typed(Item::FpsCap, "unlimited").unwrap();
    assert_eq!(s.cfg.fps_cap, 0);
    s.apply_typed(Item::AccentHue, "33aaff").unwrap();
    assert_eq!(s.cfg.accent, 0x33aaff);
    assert!(s.apply_typed(Item::AccentHue, "blue-ish").is_err());
    assert!(s.apply_typed(Item::BackgroundDim, "dark").is_err());
}

#[test]
fn typing_replaces_the_old_value_then_adds_to_it() {
    let mut e = settings::Edit::new(Item::Offset, "12".into());
    e.type_text("-", 12);
    e.type_text("5", 12);
    assert_eq!(e.text, "-5");
    e.backspace();
    assert_eq!(e.text, "-");
    e.type_text("123456789012345", 12);
    assert_eq!(e.text.len(), 12);
}

#[test]
fn the_caret_moves_through_a_typed_value_to_fix_one_digit() {
    use winit::keyboard::KeyCode::*;
    let mut e = settings::Edit::new(Item::Offset, "1388".into());
    // Moving keeps the opened value to edit instead of typing over it.
    assert!(e.nav(Home));
    assert!(!e.fresh && e.caret == 0);
    e.nav(ArrowRight);
    e.nav(Delete);
    e.type_text("4", 12);
    assert_eq!(e.text, "1488");
    assert_eq!(e.caret, 2);
    e.backspace();
    assert_eq!(e.text, "188");
    e.nav(End);
    e.nav(ArrowRight);
    e.type_text("9", 12);
    assert_eq!(e.text, "1889");
    assert!(!e.nav(KeyA));
    // Other characters than ASCII too.
    let (mut t, mut c) = ("ёж".to_string(), 1);
    settings::caret_insert(&mut t, &mut c, "-", 12);
    assert_eq!((t.as_str(), c), ("ё-ж", 2));
}

#[test]
fn songs_folder_is_saved_scanned_and_found_inside_an_install() {
    let _turn = FOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    let install = std::env::temp_dir().join(format!("taiko-rs-songs-test-{}", std::process::id()));
    std::fs::create_dir_all(install.join("Songs")).unwrap();
    std::fs::write(install.join("osu!.db"), b"").unwrap();
    let mut s = layout_tests::state();
    // Picking osu!'s own folder means its Songs folder.
    s.set_songs_folder(Some(install.clone())).unwrap();
    assert_eq!(s.cfg.songs_folder, Some(install.join("Songs")));
    assert_eq!(s.songs_dir, install.join("Songs"));
    assert!(s.scan.is_some());
    assert_eq!(library::song_dirs(), vec![install.join("Songs")]);
    assert!(s.set_songs_folder(Some(install.join("missing"))).is_err());
    assert!(
        s.apply_typed(
            Item::SongsFolder,
            &format!("\"{}\"", install.join("Songs").display())
        )
        .is_ok()
    );
    // Back to looking for osu! where it usually is.
    s.set_songs_folder(None).unwrap();
    assert_eq!(s.cfg.songs_folder, None);
    let _ = std::fs::remove_dir_all(&install);
}

#[test]
fn typed_note_colours_become_your_own_keeping_the_other() {
    let mut s = layout_tests::state();
    s.cfg.note_colours = multiverse::Colours {
        palette: 1,
        ..multiverse::Colours::CLASSIC
    };
    s.apply_typed(Item::KatColour, "#a0b0c0").unwrap();
    // Neon's don stays; the kat is the typed one.
    assert_eq!(
        s.cfg.note_colours,
        multiverse::Colours::own(multiverse::PALETTES[1].1, 0xa0b0c0)
    );
    s.apply_typed(Item::DonColour, "102030").unwrap();
    assert_eq!(
        (s.cfg.note_colours.don(), s.cfg.note_colours.kat()),
        (0x102030, 0xa0b0c0)
    );
    assert!(s.apply_typed(Item::DonColour, "red").is_err());
}

#[test]
fn vsync_draws_notes_ahead_by_the_frames_on_their_way_plus_the_fine_tune() {
    let mut s = layout_tests::state();
    s.refresh_hz = 60.0;
    s.present = PresentChoice::Fifo;
    assert!((s.vsync_lead() - 2000.0 / 60.0).abs() < 1e-9);
    s.cfg.vsync_tune_ms = -5.0;
    assert!((s.vsync_lead() - (2000.0 / 60.0 - 5.0)).abs() < 1e-9);
    s.refresh_hz = 144.0;
    assert!((s.vsync_lead() - (2000.0 / 144.0 - 5.0)).abs() < 1e-9);
    // Without vsync frames show at once; and it can be switched off.
    s.present = PresentChoice::Mailbox;
    assert_eq!(s.vsync_lead(), 0.0);
    s.present = PresentChoice::Fifo;
    s.cfg.vsync_compensation = false;
    assert_eq!(s.vsync_lead(), 0.0);
}

#[test]
fn vsync_lead_goes_by_the_refresh_the_frames_came_at() {
    // Told 60 Hz (no screen read yet, on Wayland), frames seen coming at 180 Hz: two of its
    // refreshes ahead (11 ms), not two of 60 Hz's (33 ms).
    let mut s = layout_tests::state();
    s.refresh_hz = 60.0;
    s.present = PresentChoice::Fifo;
    s.seen_refresh_ms = Some(1000.0 / 180.0);
    assert!((s.vsync_lead() - 2000.0 / 180.0).abs() < 1e-9);
}

#[test]
fn vsync_calibration_is_by_sight_in_vsync_and_sets_the_fine_tune() {
    let mut s = layout_tests::state();
    s.cfg.set_osu_offset(-35.0);
    s.cfg.vsync_tune_ms = 3.0;
    s.start_calibration_as(true);
    assert!(
        matches!(s.scene, Scene::Calibrate { vsync: true, .. }),
        "not calibrating by sight"
    );
    assert_eq!(s.wanted_present(), PresentChoice::Fifo);
    let Scene::Calibrate { game, .. } = &mut s.scene else {
        unreachable!()
    };
    // Hit 8 ms late by sight on average: the frames show 8 ms later than the fine tune assumes.
    game.stats_mut().errors = vec![8.0; 20];
    s.finish_calibration();
    s.leave_calibration(true);
    assert_eq!((s.cfg.vsync_tune_ms, s.cfg.offset_ms), (11.0, 12.0));
}

#[test]
fn calibrating_by_ear_shows_no_notes_and_nothing_about_the_hits() {
    let mut s = layout_tests::state();
    s.start_calibration_as(false);
    let Scene::Calibrate { game, .. } = &mut s.scene else {
        unreachable!()
    };
    assert!(
        game.mute_hits(),
        "your own hitsounds would be something to sync to"
    );
    // 20 hits, all 37 ms early, well into the song.
    game.stats_mut().errors = vec![-37.0; 20];
    let at = game.map().notes[10].time;
    game.put_at(at);
    let mut c = Canvas {
        audit: true,
        ..Canvas::default()
    };
    calibrate::draw_by_ear(&mut c, game);
    let texts: Vec<&str> = c.texts.iter().map(|(_, t)| t.as_str()).collect();
    assert!(!texts.is_empty());
    for t in &texts {
        assert!(
            !t.contains("37")
                && !t.contains("early")
                && !t.contains("late")
                && !t.contains("suggest"),
            "{t:?} tells how the hits went"
        );
    }
}

#[test]
fn calibrating_takes_any_drum_key() {
    let mut s = layout_tests::state();
    s.start_calibration_as(false);
    let Scene::Calibrate { game, .. } = &s.scene else {
        unreachable!()
    };
    // No audio here: the clock runs on from where it started, so a press this far ahead lands on
    // the first note.
    let ahead = (game.map().notes[0].time - game.time()) / 1000.0;
    let rim = s.cfg.keys[0];
    s.calibration_key(
        rim,
        false,
        Instant::now() + Duration::from_secs_f64(ahead),
        None,
    );
    let Scene::Calibrate { game, .. } = &s.scene else {
        unreachable!()
    };
    assert_eq!(game.stats().errors.len(), 1, "a rim key missed the don");
}

#[test]
fn moving_the_game_data_runs_in_the_background_then_switches_folders() {
    let _turn = FOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("taiko-rs-move-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let old = root.join("old/taiko-rs");
    std::fs::create_dir_all(old.join("replays")).unwrap();
    std::fs::write(old.join("replays/1.json"), "replay").unwrap();
    std::fs::create_dir_all(root.join("drive")).unwrap();
    let mut s = layout_tests::state();
    s.cfg.data_folder = Some(old.clone());
    // A drive or any folder: the data gets a taiko-rs folder of its own there.
    s.move_data_to(Some(root.join("drive"))).unwrap();
    assert!(
        s.move_data_to(Some(root.join("elsewhere"))).is_err(),
        "one move at a time"
    );
    let start = Instant::now();
    while s.data_move.is_some() && start.elapsed() < Duration::from_secs(10) {
        s.poll_data_move();
    }
    let new = root.join("drive/taiko-rs");
    assert_eq!(s.cfg.data_folder, Some(new.clone()));
    assert_eq!(config::data_dir(), Some(new.clone()));
    assert_eq!(
        std::fs::read_to_string(new.join("replays/1.json")).unwrap(),
        "replay"
    );
    assert!(!old.join("replays").exists());
    config::set_data_folder(None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn window_sizes_fit_the_screen_and_step_within_the_aspect_ratio() {
    let mut s = layout_tests::state();
    s.monitor_size = (1920, 1080);
    s.monitor_modes = vec![(1920, 1080), (1680, 1050), (1280, 1024)];
    let sizes = s.window_sizes();
    // The screen's aspect ratio first, largest first in each, all with room for a title bar.
    assert_eq!(layout::aspect_name(sizes[0].0, sizes[0].1), "16:9");
    let groups = layout::group_presets(&sizes, (1920, 1080));
    assert!(
        groups
            .iter()
            .all(|(_, l)| l.windows(2).all(|w| w[0].0 * w[0].1 >= w[1].0 * w[1].1))
    );
    assert!(sizes.iter().all(|&(w, h)| w <= 1920 && h <= 1020));
    // 1280x720 steps to 16:9 sizes.
    (s.window_w, s.window_h) = (1280.0, 720.0);
    s.adjust_setting(Item::WindowSize, 1, None);
    assert_eq!(s.window_request, Some((1366, 768)));
    s.adjust_setting(Item::WindowSize, -1, None);
    assert_eq!(s.window_request, Some((1024, 576)));
    // Typed: what fits the screen, nothing bigger.
    s.apply_typed(Item::WindowSize, "1600x900").unwrap();
    assert_eq!(s.window_request, Some((1600, 900)));
    assert!(s.apply_typed(Item::WindowSize, "3840x2160").is_err());
}

#[test]
fn shift_tab_hides_the_interface_in_a_song_and_tab_stays_the_map_info() {
    let mut s = layout_tests::state();
    // The first frame takes in the library (and with it whatever it starts).
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.play_when_scanned = false;
    let map = beatmap::calibration_map(500.0, 4, 64);
    s.scene = Scene::Play {
        game: Box::new(Play::new(Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        ))),
        entry: 0,
        from: None,
    };
    let info = s.cfg.show_map_info;
    s.modifiers_shift = true;
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(s.cfg.interface_hidden);
    s.modifiers_shift = false;
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(s.cfg.interface_hidden, "plain Tab isn't the interface key");
    assert_ne!(s.cfg.show_map_info, info);
    // What's hidden reaches the game drawn.
    let mut c = Canvas {
        width: 853.3,
        ..Default::default()
    };
    s.build_frame(&mut c, Instant::now(), 853.3);
    let Scene::Play { game, .. } = &s.scene else {
        panic!()
    };
    assert_eq!(game.view.hidden, hud::Parts::DEFAULT_HIDES);
}

#[test]
fn ctrl_c_copies_what_is_typed_or_picked_and_paste_goes_only_where_text_does() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    assert!(s.takes_text());
    let picked = s.select.selected().map(|i| s.library[i].label());
    assert!(picked.is_some());
    assert_eq!(s.copy_text(), picked);
    s.open_settings();
    s.start_edit(Item::ScrollSpeed);
    assert_eq!(s.copy_text().as_deref(), Some("1.00"));
    // A pasted line arrives as typed text (Ctrl let go of).
    s.key(KeyCode::KeyV, false, Some("1.5x"), Instant::now(), None);
    assert_eq!(s.settings_ui.editing.as_ref().unwrap().text, "1.5x");
    // Not into a song (V could be a drum).
    s.settings_ui.editing = None;
    let map = beatmap::calibration_map(500.0, 4, 64);
    s.scene = Scene::Play {
        game: Box::new(Play::new(Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        ))),
        entry: 0,
        from: None,
    };
    assert!(!s.takes_text());
}

#[test]
fn ctrl_backspace_clears_the_search() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.play_when_scanned = false;
    s.scene = Scene::Select;
    s.select.filter = "camellia".into();
    s.modifiers_ctrl = true;
    s.key(KeyCode::Backspace, false, None, Instant::now(), None);
    assert!(s.select.filter.is_empty());
}

#[test]
fn ticking_interface_parts_keeps_the_list_open() {
    let mut s = layout_tests::state();
    s.open_choices(Item::InterfaceHides);
    let k = hud::Part::ALL
        .iter()
        .position(|&p| p == hud::Part::HitAnimation)
        .unwrap();
    s.pick_choice(k);
    assert!(!s.cfg.interface_hides.has(hud::Part::HitAnimation));
    assert_eq!(s.settings_ui.choices.as_ref().map(|c| c.cursor), Some(k));
    assert!(s.settings_ui.choices.as_ref().unwrap().names[k].starts_with("[ ]"));
}

#[test]
fn star_ratings_arriving_dont_change_the_song_thats_playing() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    for (other_audio, stays) in [(false, false), (true, true)] {
        s.select.jump_songs(1, &s.library);
        let entered = s.select.selected().unwrap();
        let hardest = s.select.selected_set()[0];
        assert_ne!(hardest, entered);
        let audio = s.library[entered].meta.audio_file.clone();
        s.library[hardest].meta.audio_file = if other_audio {
            format!("other {audio}")
        } else {
            audio
        };
        let stars = |i: usize| Some(if i == hardest { 5.0 } else { i as f32 / 100.0 });
        s.select.sort_selected_set(&s.library, &stars);
        assert_eq!(
            s.select.selected(),
            Some(if stays { entered } else { hardest })
        );
    }
}

#[test]
fn an_old_map_without_ids_in_its_file_goes_by_its_folders_set() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // Ranked, with no IDs in the .osu: nothing in old formats, -1 and 0 in guest difficulties.
    s.library[0].meta.set_id = None;
    s.library[0].meta.beatmap_id = None;
    assert_eq!(s.map_status(0).map(|x| x.0), Some("NOT SUBMITTED"));
    s.library[0].path = "/songs/210 Taiko no Tatsujin - Saitama2000/map [Oni].osu".into();
    assert_eq!(s.map_status(0), None, "nothing known yet");

    s.set_status_tx.send((210, Some("ranked".into()))).unwrap();
    s.poll_set_status(Instant::now());
    assert_eq!(s.map_status(0).map(|x| x.0), Some("RANKED"));
    assert_eq!(
        s.select.status.get(&s.library[0].path),
        Some(&search::RANKED)
    );
    assert_eq!(
        map_gives_pp(
            &s.library[0],
            &s.set_status,
            &s.db_status,
            &s.select.server_loved
        ),
        Some(true)
    );
}

#[test]
fn a_maps_status_comes_from_the_mirror_else_osu_db() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // No set id: never submitted.
    assert_eq!(s.map_status(0).map(|x| x.0), Some("NOT SUBMITTED"));
    s.library[0].meta.set_id = Some(2400428);
    assert_eq!(s.map_status(0), None, "nothing known yet");
    s.db_status.insert(s.library[0].path.clone(), 2);
    assert_eq!(s.map_status(0).map(|x| x.0), Some("PENDING"));
    // The mirror can tell pending from graveyard.
    s.set_status.insert(2400428, Some("graveyard".into()));
    assert_eq!(s.map_status(0).map(|x| x.0), Some("GRAVEYARD"));
}

#[test]
fn a_custom_maps_status_is_the_websites() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // A .tko set has no osu! number: before, the panel called it not submitted.
    s.library[0].meta.set_id = None;
    let folder = s.library[0].path.parent().unwrap().to_path_buf();
    s.select.custom_folder.insert(folder, 2);
    assert_eq!(s.map_status(0).map(|x| x.0), Some("CUSTOM"));
    s.select.custom_approved.insert(2);
    assert_eq!(s.map_status(0).map(|x| x.0), Some("CUSTOM APPROVED"));
    // An edited copy of it is nobody's, as in the song list.
    s.select.edited.insert(s.library[0].path.clone());
    assert_eq!(s.map_status(0).map(|x| x.0), Some("NOT SUBMITTED"));
}

#[test]
fn scroll_speed_changes_between_songs_only() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let now = Instant::now();
    let start = s.cfg.scroll_speed;
    s.key(KeyCode::F4, false, None, now, None);
    let faster = s.cfg.scroll_speed;
    assert!(faster > start, "song select changes it");
    // In a song (and its pause menu's settings) it stays, and the play keeps its own.
    let map = beatmap::calibration_map(500.0, 4, 64);
    let mut game = Play::new(Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    ));
    game.set_scroll_speed(faster);
    s.scene = Scene::Play {
        game: Box::new(game),
        entry: 0,
        from: None,
    };
    s.key(KeyCode::F3, false, None, now, None);
    assert_eq!(s.cfg.scroll_speed, faster);
    s.scene = Scene::Settings {
        prev: Box::new(std::mem::replace(&mut s.scene, Scene::Select)),
    };
    s.adjust_setting(settings::Item::ScrollSpeed, -1, None);
    assert_eq!(s.cfg.scroll_speed, faster);
    assert!(s.apply_typed(settings::Item::ScrollSpeed, "0.5").is_err());
    assert_eq!(s.cfg.scroll_speed, faster);
}

#[test]
fn sets_loved_on_the_server_count_as_loved() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.library[0].meta.set_id = Some(2275915);
    s.set_status.insert(2275915, Some("graveyard".into()));
    s.select.server_loved.insert(2275915, Vec::new());
    assert_eq!(s.map_status(0).map(|x| x.0), Some("LOVED HERE"));
    assert_eq!(
        map_gives_pp(
            &s.library[0],
            &s.set_status,
            &s.db_status,
            &s.select.server_loved
        ),
        Some(true)
    );
    // The search's status=l finds it.
    s.select.filter = "status=l".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.selected(), Some(0));
    // Only nominators get the menu item, and it says which way.
    s.select.filter.clear();
    s.select.refilter(&s.library);
    s.select.menu = Some(ui::SongMenu {
        row: 0,
        header: false,
        skin: false,
        x: 0.0,
        y: 0.0,
    });
    assert!(
        !s.select
            .menu_items(&s.library)
            .iter()
            .any(|(_, a)| matches!(a, ui::MenuAction::Love(..)))
    );
    s.select.nominator = true;
    let items = |s: &State| s.select.menu_items(&s.library);
    assert!(items(&s).iter().any(
        |(l, a)| l == "Unlove on taiko.rs" && *a == ui::MenuAction::Love(2275915, false, false)
    ));
    // Just some difficulties loved: the others give no pp, and each can be loved alone.
    let version = s.library[0].meta.version.clone();
    let other = |s: &State| {
        (0..s.library.len()).find(|&i| {
            s.library[i].set_id() == Some(2275915) && s.library[i].meta.version != version
        })
    };
    if other(&s).is_none() {
        s.library[1].meta.set_id = Some(2275915);
    }
    let j = other(&s).expect("another difficulty of the set");
    s.select
        .server_loved
        .insert(2275915, vec![format!(" {} ", version.to_uppercase())]);
    let gives = |s: &State, i: usize| {
        map_gives_pp(
            &s.library[i],
            &s.set_status,
            &s.db_status,
            &s.select.server_loved,
        )
    };
    assert_eq!(gives(&s, 0), Some(true));
    assert_eq!(
        gives(&s, j),
        Some(false),
        "graveyard, and not the one loved"
    );
    assert_eq!(s.map_status(0).map(|x| x.0), Some("LOVED HERE"));
    assert_ne!(s.map_status(j).map(|x| x.0), Some("LOVED HERE"));
    let menu = items(&s);
    assert!(menu.iter().any(
        |(l, a)| l.starts_with("Unlove [") && *a == ui::MenuAction::Love(2275915, false, true)
    ));
    assert!(
        menu.iter()
            .any(|(l, a)| l == "Love the whole set on taiko.rs"
                && *a == ui::MenuAction::Love(2275915, true, false))
    );
    s.select.server_loved.clear();
    let menu = items(&s);
    assert!(
        menu.iter().any(
            |(l, a)| l.starts_with("Love [") && *a == ui::MenuAction::Love(2275915, true, true)
        )
    );
    assert!(
        menu.iter()
            .any(|(_, a)| *a == ui::MenuAction::Love(2275915, true, false))
    );
}

#[test]
fn hotkeys_can_be_moved_and_removed() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let now = Instant::now();
    // Pause on P: Esc no longer pauses, P does.
    s.cfg
        .hotkeys
        .set(keys::Hotkey::Pause, Some(keys::Chord::plain(KeyCode::KeyP)));
    let map = beatmap::calibration_map(500.0, 4, 64);
    s.scene = Scene::Play {
        game: Box::new(Play::new(Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        ))),
        entry: 0,
        from: None,
    };
    s.key(KeyCode::Escape, false, None, now, None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused().is_none()));
    s.key(KeyCode::KeyP, false, None, now, None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused().is_some()));

    // Scroll speed up taken away: F4 does nothing.
    s.scene = Scene::Select;
    let speed = s.cfg.scroll_speed;
    s.cfg.hotkeys.set(keys::Hotkey::ScrollFaster, None);
    s.key(KeyCode::F4, false, None, now, None);
    assert_eq!(s.cfg.scroll_speed, speed);

    // Bound in settings: Enter on its row, then Ctrl+K; Delete takes it away, Backspace brings
    // the default back.
    s.open_settings();
    let h = keys::Hotkey::ScrollSlower;
    let i = keys::Hotkey::ALL.iter().position(|&k| k == h).unwrap();
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::Hotkey(i))
        .unwrap();
    s.key(KeyCode::Enter, false, None, now, None);
    s.modifiers_ctrl = true;
    s.key(KeyCode::ControlLeft, false, None, now, None);
    s.key(KeyCode::KeyK, false, None, now, None);
    s.modifiers_ctrl = false;
    assert_eq!(
        s.cfg.hotkeys.get(h),
        Some(keys::Chord {
            key: KeyCode::KeyK,
            ctrl: true,
            shift: false,
            alt: false
        })
    );
    s.key(KeyCode::Enter, false, None, now, None);
    s.key(KeyCode::Delete, false, None, now, None);
    assert_eq!(s.cfg.hotkeys.get(h), None);
    s.key(KeyCode::Backspace, false, None, now, None);
    assert_eq!(s.cfg.hotkeys.get(h), Some(h.default()));

    // A drum key can be taken away too.
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::Key(0))
        .unwrap();
    s.key(KeyCode::Enter, false, None, now, None);
    s.key(KeyCode::Delete, false, None, now, None);
    assert_eq!(s.cfg.keys[0], config::NO_KEY);
    s.key(KeyCode::Backspace, false, None, now, None);
    assert_eq!(s.cfg.keys[0], KeyCode::KeyD);
}

#[test]
fn krokodil_koban_mode_is_only_for_krokodil_koban() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.open_settings();
    let row = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::KrokodilKoban)
        .unwrap();
    s.settings_ui.cursor = row - 1;
    s.settings_ui.move_cursor(1);
    assert_ne!(
        s.settings_ui.item(),
        Item::KrokodilKoban,
        "skipped for anyone else"
    );
    s.mp_menu.account = Some("Krokodil_Koban".into());
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(s.settings_ui.krokodil);
    s.settings_ui.cursor = row - 1;
    s.settings_ui.move_cursor(1);
    assert_eq!(s.settings_ui.item(), Item::KrokodilKoban);
}

#[test]
fn the_friends_overlay_closes_on_a_click_outside_it_and_slides_away() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.mp_menu.account = Some("me".into());
    s.open_social();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // On the song list, far left (the window is 1920x1080: 2.25 px per virtual unit).
    let cursor = s.select.cursor();
    s.mouse = (60.0, 700.0);
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert!(!s.social.open);
    // It slides away (at once here: the tests turn UI animations off).
    assert!(s.social.closed_at.is_some());
    assert_eq!(s.select.cursor(), cursor, "the click only closed it");
    assert!(matches!(s.scene, Scene::Select));
    // Drawn while closing, but takes no clicks.
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    assert!(
        !s.hits
            .iter()
            .any(|h| matches!(h.hit, crate::hit::Hit::SocialPanel))
    );
}

/// Song select with 40 scores on the selected map (the window is 1920x1080: 2.25 px per unit).
fn select_with_scores() -> State {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    // A short title, so the panel's height is up to its preview (a long one wraps, and its
    // details alone fill the panel).
    let short = s
        .library
        .iter()
        .find(|e| e.meta.title.len() < 12)
        .expect("a short title")
        .path
        .clone();
    s.select.select_path(&s.library, &short);
    let hash = s.library[s.select.selected().unwrap()].hash();
    s.scores = (0..40)
        .map(|i| scores::ScoreRow {
            map_hash: hash,
            date: 1_700_000_000 + i,
            score: 1_000_000 - i * 1000,
            accuracy: 0.99,
            max_combo: 100,
            miss: 0,
            rate: 1.0,
            player: "me".into(),
            file: PathBuf::from(format!("/nonexistent/{i}.json")),
            mods: String::new(),
            pp: None,
        })
        .collect();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s
}

fn point(s: &mut State, x: f32, y: f32) {
    s.mouse_moved(x as f64 * 2.25, y as f64 * 2.25);
}

#[test]
fn song_select_dividers_drag_within_limits_and_reset() {
    let mut s = select_with_scores();
    let w = 853.3333f32;
    let frame = |s: &mut State| s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // The song list's edge, dragged right, then far past its limit.
    let lw = Select::list_width(w, s.cfg.song_list_share);
    // Its grip sits in the middle of the gap between the list's box (to 28 + lw) and the panel
    // (from 36 + lw).
    point(&mut s, 24.0 + lw + 8.0, 240.0);
    assert_eq!(
        s.hit_at(24.0 + lw + 8.0, 240.0),
        Some(crate::hit::Hit::SplitList)
    );
    assert_ne!(
        s.hit_at(24.0 + lw + 3.0, 240.0),
        Some(crate::hit::Hit::SplitList),
        "not on the list's box"
    );
    // Only its grip in the middle, not the whole edge.
    assert_ne!(
        s.hit_at(24.0 + lw + 8.0, 120.0),
        Some(crate::hit::Hit::SplitList)
    );
    s.mouse_button(true, None);
    point(&mut s, 24.0 + lw + 8.0 + 40.0, 240.0);
    assert!(
        (s.cfg.song_list_share - (ui::LIST_SHARE.2 + 40.0 / w)).abs() < 0.002,
        "{}",
        s.cfg.song_list_share
    );
    point(&mut s, w, 240.0);
    s.mouse_button(false, None);
    assert!(s.split_drag.is_none());
    let widest = Select::list_width(w, s.cfg.song_list_share);
    assert!(
        s.cfg.song_list_share <= ui::LIST_SHARE.1 && w - 24.0 - widest - 12.0 - 24.0 >= 219.9,
        "the panel keeps its room: {widest}"
    );
    // And far left: the list keeps its room too.
    frame(&mut s);
    point(&mut s, 24.0 + widest + 8.0, 240.0);
    s.mouse_button(true, None);
    point(&mut s, 0.0, 240.0);
    s.mouse_button(false, None);
    let narrowest = Select::list_width(w, s.cfg.song_list_share);
    assert!(
        s.cfg.song_list_share >= ui::LIST_SHARE.0 && narrowest >= 259.9,
        "{narrowest}"
    );

    // A double click on it puts it back.
    frame(&mut s);
    point(&mut s, 24.0 + narrowest + 8.0, 240.0);
    for _ in 0..2 {
        s.mouse_button(true, None);
        s.mouse_button(false, None);
    }
    assert_eq!(s.cfg.song_list_share, ui::LIST_SHARE.2);

    // The panel's bottom edge isn't dragged any more: nothing to grab there, and the preview is
    // at full size whatever an older config says.
    s.cfg.song_preview_size = preview::PREVIEW_SIZE.0;
    frame(&mut s);
    let edge = s.preview.panel_bottom + 4.0;
    let left = 24.0 + Select::list_width(w, s.cfg.song_list_share) + 12.0;
    let mid = (left + w - 24.0) / 2.0;
    assert_eq!(s.hit_at(mid, edge), None);
    assert_eq!(s.preview.size, preview::PREVIEW_SIZE.1);

    // Settings > Library puts the whole layout back.
    s.cfg.song_list_share = ui::LIST_SHARE.1;
    s.open_settings();
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::SongSelectLayout)
        .unwrap();
    assert!(
        s.setting_value(Item::SongSelectLayout)
            .starts_with("custom")
    );
    s.key(KeyCode::Enter, false, None, Instant::now(), None);
    assert_eq!(
        (s.cfg.song_list_share, s.cfg.song_preview_size),
        (ui::LIST_SHARE.2, preview::PREVIEW_SIZE.1)
    );
    assert_eq!(s.setting_value(Item::SongSelectLayout), "default");
}

#[test]
fn the_leaderboard_scrolls_with_the_wheel() {
    let mut s = select_with_scores();
    // Room for a few rows whatever the map's title (a long one wraps, making the panel taller).
    s.cfg.song_preview_size = preview::PREVIEW_SIZE.0;
    // The preview is up first: the leaderboard is a page the button slides to.
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.board_rect, None);
    s.click(crate::hit::Hit::BoardPage, None);
    let slid = || Canvas {
        dt_ms: 1000.0,
        ..Canvas::default()
    };
    s.build_frame(&mut slid(), Instant::now(), 853.3);
    let [bx, by, bw, bh] = s.board_rect.expect("a leaderboard");
    let shown = s.leaderboard.len();
    assert!(shown > 0 && shown < 40);
    point(&mut s, bx + bw / 2.0, by + bh / 2.0);
    let cursor = s.select.cursor();
    s.wheel(-2);
    assert_eq!(s.board_scroll, 2);
    assert_eq!(s.select.cursor(), cursor, "the song list stays");
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // The rows shown start at the 3rd best: a click opens the one clicked.
    assert_eq!(s.leaderboard[0], PathBuf::from("/nonexistent/2.json"));
    // No further than the last score.
    s.wheel(-1000);
    assert_eq!(s.board_scroll, 40 - shown);
    s.wheel(1000);
    assert_eq!(s.board_scroll, 0);
    // The other tab starts at the top.
    s.wheel(-3);
    s.click(crate::hit::Hit::BoardTab(true), None);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.board_scroll, 0);
}

#[test]
fn the_results_scroll_down_to_the_pp_and_back() {
    let mut s = select_with_scores();
    let mut r = ui::Results::new(
        "map".into(),
        game::Stats {
            great: 10,
            ..Default::default()
        },
        game::Windows::new(5.0),
    );
    r.no_pp = Some("no pp here");
    s.scene = Scene::Results {
        results: Box::new(r),
        entry: 0,
        from: None,
    };
    let scroll = |s: &State| match &s.scene {
        Scene::Results { results, .. } => results.scroll_to,
        _ => panic!("left the results"),
    };
    // One notch takes it all the way, either way.
    s.wheel(-1);
    assert_eq!(scroll(&s), ui::RESULTS_SCROLL);
    s.wheel(-5);
    assert_eq!(scroll(&s), ui::RESULTS_SCROLL, "no further than the pp");
    s.wheel(1);
    assert_eq!(scroll(&s), 0.0);
    s.key(KeyCode::ArrowDown, false, None, Instant::now(), None);
    assert_eq!(scroll(&s), ui::RESULTS_SCROLL);
    s.key(KeyCode::Home, false, None, Instant::now(), None);
    assert_eq!(scroll(&s), 0.0);
    // The hint under the buttons goes there too.
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.click(crate::hit::Hit::Key(KeyCode::PageDown), None);
    assert_eq!(scroll(&s), ui::RESULTS_SCROLL);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == crate::hit::Hit::Key(KeyCode::PageUp)),
        "and back up"
    );
}

#[test]
fn one_click_on_a_main_menu_note_opens_it() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    s.click(crate::hit::Hit::MenuItem(5), None);
    let m = s.main_menu.as_ref().unwrap();
    assert_eq!(m.current(), menu::Item::Settings);
    assert!(m.is_leaving(), "no second click at the circle needed");
}

fn settings_screen() -> State {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.main_menu = None;
    s.open_settings();
    s
}

fn type_key(s: &mut State, code: KeyCode, text: &str) {
    s.key(code, false, Some(text), Instant::now(), None);
}

#[test]
fn typing_in_the_settings_searches_and_escape_clears_before_it_leaves() {
    let mut s = settings_screen();
    for (code, text) in [
        (KeyCode::KeyM, "m"),
        (KeyCode::KeyU, "u"),
        (KeyCode::KeyS, "s"),
        (KeyCode::KeyI, "i"),
        (KeyCode::KeyC, "c"),
    ] {
        type_key(&mut s, code, text);
    }
    assert_eq!(s.settings_ui.search, "music");
    assert_eq!(s.settings_ui.item(), Item::MusicVolume);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    // The rows work as usual, and it's the results the arrows go through.
    s.key(KeyCode::ArrowRight, false, None, Instant::now(), None);
    s.key(KeyCode::Backspace, false, None, Instant::now(), None);
    assert_eq!(s.settings_ui.search, "musi", "not the row's reset");
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(s.settings_ui.search.is_empty());
    assert!(matches!(s.scene, Scene::Settings { .. }), "still here");
    s.key(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(!matches!(s.scene, Scene::Settings { .. }));
}

#[test]
fn space_and_punctuation_keep_their_jobs_until_something_is_typed() {
    let mut s = settings_screen();
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::HalfResolution)
        .unwrap();
    let was = s.cfg.half_resolution;
    type_key(&mut s, KeyCode::Space, " ");
    assert_ne!(s.cfg.half_resolution, was, "space works the row");
    assert!(s.settings_ui.search.is_empty());
    type_key(&mut s, KeyCode::Minus, "-");
    assert!(s.settings_ui.search.is_empty());
    assert_eq!(
        s.hotkey_place(KeyCode::Minus),
        Some(keys::Place::Other),
        "the offset's hotkey still works"
    );
    type_key(&mut s, KeyCode::KeyF, "f");
    type_key(&mut s, KeyCode::Space, " ");
    assert_eq!(s.settings_ui.search, "f ");
    assert_eq!(s.hotkey_place(KeyCode::Minus), None, "now it types");
}

#[test]
fn the_wheel_scrolls_the_settings_and_stays_in_the_section() {
    let mut s = settings_screen();
    s.settings_ui.cursor = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::Key(0))
        .unwrap();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let (cursor, section) = (s.settings_ui.cursor, s.settings_ui.section());
    s.wheel(-1000);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.settings_ui.cursor, cursor);
    assert_eq!(s.settings_ui.section(), section);
    // Down to the last rows, which are now there to click.
    let last = settings::ITEMS
        .iter()
        .rposition(|(sec, _)| *sec == settings::ITEMS[cursor].0)
        .unwrap();
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == crate::hit::Hit::SettingRow(last))
    );
}

#[test]
fn one_click_works_a_row_and_every_hotkey_has_one() {
    let mut s = settings_screen();
    let row = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::LegacyMenu)
        .unwrap();
    let was = s.cfg.main_menu;
    s.click(crate::hit::Hit::SettingRow(row), None);
    assert_ne!(s.cfg.main_menu, was);
    for i in 0..keys::Hotkey::ALL.len() {
        assert!(
            settings::ITEMS.iter().any(|(_, it)| *it == Item::Hotkey(i)),
            "{:?}",
            keys::Hotkey::ALL[i]
        );
    }
    for (section, _) in settings::ITEMS {
        assert!(
            settings::SECTIONS.iter().any(|(name, _)| *name == section),
            "{section}"
        );
    }
}

#[test]
fn a_volume_is_dragged_along_its_bar_and_saved_on_letting_go() {
    let mut s = settings_screen();
    let row = settings::ITEMS
        .iter()
        .position(|(_, it)| *it == Item::MusicVolume)
        .unwrap();
    s.settings_ui.cursor = row;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let bar = s
        .hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::SettingLevel(row))
        .expect("a bar")
        .rect;
    let y = bar[1] + bar[3] / 2.0;
    point(&mut s, bar[0] + 5.0, y);
    s.click(crate::hit::Hit::SettingLevel(row), None);
    assert_eq!(s.cfg.music_volume, 0.0);
    point(&mut s, bar[0] + bar[2] / 2.0, y);
    s.mouse_moved(s.mouse.0, s.mouse.1);
    assert!(
        (s.cfg.music_volume - 0.5).abs() < 0.02,
        "{}",
        s.cfg.music_volume
    );
    s.mouse_button(false, None);
    assert_eq!(s.settings_level, None);
}

#[test]
fn alt_wheel_turns_the_note_under_the_pointer_and_the_lane_then_needs_no_alt() {
    let mut s = settings_screen();
    s.cfg.master_volume = 0.5;
    s.cfg.fx_volume = 0.5;
    s.mouse = (-1000.0, -1000.0);
    s.modifiers_alt = true;
    s.wheel(1);
    assert_eq!(s.cfg.master_volume, 0.55, "the master first");
    s.modifiers_alt = false;
    // Hovering the hitsounds note picks it, before the wheel moves at all.
    let (w, h) = (s.window_w, s.window_h);
    let width = render::VIRTUAL_HEIGHT * w / h;
    point(&mut s, width - 40.0, 400.0);
    s.mouse_moved(s.mouse.0, s.mouse.1);
    assert_eq!(s.volume.selected, volume::HITSOUNDS);
    s.wheel(-2);
    assert_eq!(s.cfg.fx_volume, 0.4, "no Alt needed over the lane");
    assert_eq!(s.cfg.master_volume, 0.55);
    // Moved away, Alt + wheel is the overall volume again.
    point(&mut s, 10.0, 10.0);
    s.modifiers_alt = true;
    s.wheel(1);
    assert_eq!(s.cfg.master_volume, 0.6);
    assert_eq!(s.cfg.fx_volume, 0.4);
}

#[test]
fn alt_up_and_down_turn_the_volume_picked_with_alt_left_and_right() {
    let press = |s: &mut State, code: KeyCode, repeat: bool| {
        s.key_inner(code, repeat, None, Instant::now(), None)
    };
    for scene in [Scene::Select, crate::screens::download::Download::scene()] {
        let mut s = settings_screen();
        s.scene = scene;
        s.cfg.master_volume = 0.5;
        s.cfg.music_volume = 0.5;
        s.mouse = (-1000.0, -1000.0);
        s.modifiers_alt = true;
        press(&mut s, KeyCode::ArrowUp, false);
        assert_eq!(s.cfg.master_volume, 0.55, "the overall volume first");
        // Held down, it keeps turning.
        press(&mut s, KeyCode::ArrowDown, false);
        press(&mut s, KeyCode::ArrowDown, true);
        press(&mut s, KeyCode::ArrowDown, true);
        assert_eq!(s.cfg.master_volume, 0.4);
        // Alt+Left picks the music, which Up then turns.
        press(&mut s, KeyCode::ArrowLeft, false);
        assert_eq!(s.volume.selected, volume::MUSIC);
        press(&mut s, KeyCode::ArrowUp, false);
        assert_eq!((s.cfg.music_volume, s.cfg.master_volume), (0.55, 0.4));
        // Without Alt the arrows are the screen's again.
        s.modifiers_alt = false;
        press(&mut s, KeyCode::ArrowUp, false);
        assert_eq!((s.cfg.music_volume, s.cfg.master_volume), (0.55, 0.4));
    }
}

#[test]
fn a_lane_coming_up_under_a_parked_pointer_turns_the_overall_volume() {
    let mut s = settings_screen();
    s.cfg.master_volume = 0.5;
    s.cfg.fx_volume = 0.5;
    let width = render::VIRTUAL_HEIGHT * s.window_w / s.window_h;
    point(&mut s, width - 40.0, 400.0);
    s.modifiers_alt = true;
    s.wheel(1);
    s.wheel(1);
    assert_eq!(s.cfg.master_volume, 0.6);
    assert_eq!(s.cfg.fx_volume, 0.5);
}

#[test]
fn the_tabs_only_come_back_with_the_legacy_menu() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.main_menu = None;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let tab = |s: &State| {
        s.hits
            .iter()
            .any(|h| matches!(h.hit, crate::hit::Hit::Tab(i) if h.rect[1] < 50.0 && i < 4))
    };
    assert!(!tab(&s), "no tabs on top");
    assert!(s.hits.iter().any(|h| h.hit == crate::hit::Hit::ToMenu));
    // Download, settings and multiplayer are a step back, on the main menu.
    assert!(!s.hits.iter().any(|h| h.hit == crate::hit::Hit::Tab(3)));
    s.click(crate::hit::Hit::ToMenu, None);
    assert!(s.on_menu());

    s.click(
        crate::hit::Hit::SettingRow(
            settings::ITEMS
                .iter()
                .position(|(_, it)| *it == Item::LegacyMenu)
                .unwrap(),
        ),
        None,
    );
    assert!(!s.cfg.main_menu);
    assert!(s.main_menu.is_none(), "the menu goes at once");
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(tab(&s), "the old tabs");
    assert!(!s.hits.iter().any(|h| h.hit == crate::hit::Hit::ToMenu));
}

#[test]
fn the_back_button_of_a_screen_does_what_esc_does() {
    let mut s = settings_screen();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == crate::hit::Hit::Key(KeyCode::Escape))
    );
    s.click(crate::hit::Hit::Key(KeyCode::Escape), None);
    assert!(matches!(s.scene, Scene::Select));
}

#[test]
fn letterbox_lists_what_the_monitor_reports_custom_ones_too() {
    let mut s = settings_screen();
    s.monitor_size = (2560, 1440);
    s.monitor_modes = vec![(2560, 1440), (1337, 777), (1920, 1080)];
    assert!(
        s.letterbox_sizes().contains(&(1337, 777)),
        "a size only the driver knows"
    );
    s.open_choices(Item::LetterboxSize);
    let list = s.settings_ui.choices.take().unwrap();
    let at = list
        .names
        .iter()
        .position(|n| n.starts_with("1337 x 777"))
        .expect("listed");
    s.open_choices(Item::LetterboxSize);
    s.pick_choice(at);
    assert_eq!(s.cfg.letterbox_size, (1337, 777));
    // Or typed, as long as it fits.
    s.apply_typed(Item::LetterboxSize, "1111x600").unwrap();
    assert_eq!(s.cfg.letterbox_size, (1111, 600));
    assert!(s.apply_typed(Item::LetterboxSize, "3840x2160").is_err());
    assert!(s.apply_typed(Item::LetterboxSize, "100x100").is_err());
    assert_eq!(s.cfg.letterbox_size, (1111, 600));
}

#[test]
fn with_letterbox_the_game_is_its_own_size_and_the_pointer_lands_on_it() {
    let mut s = settings_screen();
    s.set_window(1920.0, 1080.0);
    s.fullscreen = true;
    s.cfg.letterbox = true;
    s.cfg.letterbox_size = (1280, 720);
    assert_eq!(s.sync_letterbox(), Some([320, 180, 1280, 720]));
    assert_eq!(
        (s.window_w, s.window_h),
        (1280.0, 720.0),
        "all of the game goes by this"
    );
    assert_eq!(s.to_game(320.0, 180.0), (0.0, 0.0), "the game's corner");
    assert_eq!(
        s.to_game(100.0, 100.0),
        (-220.0, -80.0),
        "a bar: off the game, so nothing's there"
    );
    let (x, y) = s.to_game(320.0 + 640.0, 180.0 + 360.0);
    s.mouse_moved(x, y);
    let (vx, vy) = s.mouse_virtual();
    assert!(
        (vx - render::VIRTUAL_HEIGHT * 640.0 / 720.0).abs() < 0.01
            && (vy - render::VIRTUAL_HEIGHT / 2.0).abs() < 0.01,
        "the middle of the game: {vx}, {vy}"
    );
    assert_eq!(s.setting_value(Item::WindowSize), "fullscreen (1920x1080)");
    // Moved towards a corner.
    s.cfg.letterbox_pos = (100.0, -100.0);
    assert_eq!(s.sync_letterbox(), Some([640, 0, 1280, 720]));
    // Out of fullscreen, or off: the game is the window again.
    s.fullscreen = false;
    assert_eq!(s.sync_letterbox(), None);
    assert_eq!((s.window_w, s.window_h), (1920.0, 1080.0));
    assert_eq!(s.to_game(5.0, 6.0), (5.0, 6.0));
}

#[test]
fn letterbox_settings_survive_a_restart() {
    let cfg = Config {
        letterbox: true,
        letterbox_size: (1337, 777),
        letterbox_pos: (-25.0, 40.0),
        ..Config::default()
    };
    let text = cfg.to_text();
    let back = Config::parse_text(&text);
    assert!(back.letterbox);
    assert_eq!(
        (back.letterbox_size, back.letterbox_pos),
        ((1337, 777), (-25.0, 40.0))
    );
}

fn dd(s: &State) -> &settings::Dropdown {
    s.settings_ui.dropdown.as_ref().unwrap()
}

#[test]
fn the_preset_list_scrolls_by_wheel_and_scroll_bar_never_by_pointing() {
    let mut s = layout_tests::state();
    let frame = |s: &mut State| s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.modifiers_ctrl = true;
    s.key_inner(KeyCode::KeyO, false, None, Instant::now(), None);
    s.modifiers_ctrl = false;
    // Plenty of saved sizes, so the list is longer than its box.
    s.cfg.saved_presets = (0..40).map(|k| (640 + k * 10, 360 + k * 5)).collect();
    s.open_dropdown();
    frame(&mut s);
    let max = dd(&s).max_scroll();
    assert!(
        max > 0.0 && dd(&s).bar().is_some(),
        "more sizes than fit, so it scrolls: {max}"
    );
    // Pointing at the rows nearest the bottom edge picks them and the list stays put.
    let at = dd(&s).scroll();
    for _ in 0..5 {
        let &(i, [x, y, w, h]) = dd(&s).rows.last().unwrap();
        point(&mut s, x + w / 2.0, y + h / 2.0);
        assert_eq!(dd(&s).cursor, i);
        frame(&mut s);
        assert_eq!(dd(&s).scroll(), at, "pointing never scrolls");
    }
    let picked = dd(&s).cursor;
    // The wheel moves the list, not the pick.
    s.wheel(-1);
    frame(&mut s);
    assert_eq!(dd(&s).scroll(), (at + 45.0).min(max));
    assert_eq!(dd(&s).cursor, picked);
    s.wheel(100);
    frame(&mut s);
    assert_eq!(dd(&s).scroll(), 0.0, "stops at the top");
    // The scroll bar: pressed at its bottom the list goes to its end, dragged up to its start.
    let [bx, by, bw, bh] = dd(&s).bar().unwrap();
    point(&mut s, bx + bw / 2.0, by + bh - 1.0);
    s.mouse_button(true, None);
    assert!(
        s.settings_ui.dropdown.is_some(),
        "a press on the bar doesn't close the list"
    );
    assert_eq!(dd(&s).scroll(), max);
    point(&mut s, bx + bw / 2.0, by - 50.0);
    assert_eq!(dd(&s).scroll(), 0.0);
    s.mouse_button(false, None);
    point(&mut s, bx + bw / 2.0, by + bh - 1.0);
    assert_eq!(dd(&s).scroll(), 0.0, "let go, it no longer follows");
    // Keys still bring the pick into sight.
    s.dropdown_key(KeyCode::End);
    frame(&mut s);
    assert_eq!(dd(&s).scroll(), max);
}

fn frame_off(s: &mut State) -> Option<f32> {
    s.build_frame(
        &mut Canvas::default(),
        Instant::now() + Duration::from_secs(60),
        853.3,
    );
    s.screen_fade
}

#[test]
fn a_screen_opened_from_the_main_menu_fades_in() {
    let mut s = layout_tests::state();
    s.cfg.animations = true;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    s.menu_open(menu::Item::Settings);
    assert!(matches!(s.scene, Scene::Settings { .. }));
    assert_eq!(s.screen_fade, Some(0.0));
    // Frames 50 ms apart.
    let mut at = Instant::now();
    let mut frame = || {
        at += Duration::from_millis(50);
        s.build_frame(&mut Canvas::default(), at, 853.3);
        s.screen_fade
    };
    assert!(frame().is_some_and(|ms| ms > 0.0), "fading in");
    for _ in 0..10 {
        frame();
    }
    assert_eq!(s.screen_fade, None, "then it's done");
    // With animations off it shows at once.
    s.close_settings();
    s.cfg.animations = false;
    s.menu_open(menu::Item::Settings);
    assert_eq!(frame_off(&mut s), None);
    // Exit only asks, over the menu: nothing fades.
    s.close_settings();
    s.menu_open(menu::Item::Exit);
    assert_eq!(s.screen_fade, None);
}

#[test]
fn pointing_at_a_main_menu_note_selects_it() {
    let mut s = layout_tests::state();
    // A first frame starts the map this state plays loading, which leaves the menu: open it after.
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.main_menu = Some(menu::Menu::new());
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(s.on_menu());
    // Along the panel's middle, the first spot on Medals.
    let medals = menu::ITEMS
        .iter()
        .position(|&i| i == menu::Item::Medals)
        .unwrap();
    let y = 240.0 - 30.0;
    let x = (0..853)
        .map(|x| x as f32)
        .find(|&x| s.hit_at(x, y) == Some(crate::hit::Hit::MenuItem(medals)))
        .expect("a note there");
    point(&mut s, x + 20.0, y);
    assert_eq!(s.main_menu.as_ref().unwrap().current(), menu::Item::Medals);
    // The keys move on from it, and a pointer left where it is doesn't take it back.
    s.menu_step(1);
    let (mx, my) = s.mouse;
    s.mouse_moved(mx, my);
    assert_eq!(
        s.main_menu.as_ref().unwrap().current(),
        menu::ITEMS[medals + 1]
    );
}

/// In a song of the test library, with `paused` or not.
fn in_a_song(paused: bool) -> State {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.play_when_scanned = false;
    let map = beatmap::calibration_map(500.0, 4, 64);
    let mut game = Play::new(Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    ));
    if paused {
        game.set_paused(Some(0));
    }
    s.scene = Scene::Play {
        game: Box::new(game),
        entry: 0,
        from: None,
    };
    s
}

fn restarted(s: &State) -> bool {
    !matches!(s.scene, Scene::Play { .. })
}

#[test]
fn quick_retry_needs_the_key_held_and_works_while_paused() {
    for paused in [false, true] {
        let mut s = in_a_song(paused);
        let at = Instant::now();
        s.key(KeyCode::Backquote, false, None, at, None);
        assert!(!restarted(&s), "a press alone doesn't restart");
        s.poll_retry(at + Duration::from_millis(149));
        assert!(!restarted(&s), "not before it's been held");
        let halfway = s.retry_progress(at + Duration::from_millis(75)).unwrap();
        assert!((halfway - 0.5).abs() < 0.01, "{halfway}");
        s.poll_retry(at + Duration::from_millis(150));
        assert!(restarted(&s), "paused: {paused}");
    }
}

#[test]
fn letting_go_of_quick_retry_calls_it_off() {
    let mut s = in_a_song(false);
    let at = Instant::now();
    s.key(KeyCode::Backquote, false, None, at, None);
    // Another key coming up doesn't.
    s.key_released(KeyCode::KeyD);
    assert!(s.retry_hold.is_some());
    s.key_released(KeyCode::Backquote);
    s.poll_retry(at + Duration::from_secs(5));
    assert!(!restarted(&s));
    assert_eq!(s.retry_progress(at), None);
}

#[test]
fn a_held_key_repeating_doesnt_start_the_hold_over() {
    let mut s = in_a_song(false);
    let at = Instant::now();
    s.key(KeyCode::Backquote, false, None, at, None);
    s.key(
        KeyCode::Backquote,
        true,
        None,
        at + Duration::from_millis(100),
        None,
    );
    s.poll_retry(at + Duration::from_millis(150));
    assert!(restarted(&s));
}

#[test]
fn a_rebound_quick_retry_is_let_go_of_by_its_own_key() {
    let mut s = in_a_song(false);
    let chord = keys::Chord::plain(KeyCode::KeyQ);
    s.cfg.hotkeys.set(keys::Hotkey::QuickRetry, Some(chord));
    let at = Instant::now();
    s.key(KeyCode::KeyQ, false, None, at, None);
    assert!(s.retry_hold.is_some());
    s.key_released(KeyCode::KeyQ);
    assert!(s.retry_hold.is_none());
}

#[test]
fn the_volume_drops_without_the_focus_and_comes_back() {
    let mut s = layout_tests::state();
    s.cfg.master_volume = 1.0;
    s.cfg.music_volume = 0.8;
    s.cfg.fx_volume = 0.4;
    s.cfg.unfocused_volume = 0.5;
    assert_eq!(s.heard_volumes(), (0.8, 0.4));
    s.focused = false;
    assert_eq!(s.heard_volumes(), (0.4, 0.2));
    // What's saved stays as it was.
    assert_eq!(s.cfg.volumes(), (0.8, 0.4));
    s.cfg.unfocused_volume = 1.0;
    assert_eq!(s.heard_volumes(), (0.8, 0.4));
}

#[test]
fn the_drum_stays_out_of_menus_unless_asked_for() {
    let mut s = in_a_song(true);
    let rim = s.cfg.keys[3];
    s.key(rim, false, None, Instant::now(), None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused() == Some(0)));
    s.cfg.drum_menus = true;
    s.key(rim, false, None, Instant::now(), None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused() == Some(1)));
    let left = s.cfg.keys[0];
    s.key(left, false, None, Instant::now(), None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused() == Some(0)));
}

#[test]
fn a_running_song_keeps_its_drum_with_drum_menus_on() {
    let mut s = in_a_song(false);
    s.cfg.drum_menus = true;
    let rim = s.cfg.keys[3];
    assert_eq!(s.drum_menu_key(rim), None);
}

/// On song select past the main menu, its buttons drawn. The menu comes up when the library is
/// in, which takes a frame or more: that's over first.
fn at_song_select(drum_menus: bool) -> State {
    let mut s = layout_tests::state();
    s.cfg.main_menu = true;
    s.cfg.drum_menus = drum_menus;
    s.play_when_scanned = false;
    for _ in 0..3 {
        s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
        s.main_menu = None;
        s.scene = Scene::Select;
    }
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.main_menu = None;
    s.scene = Scene::Select;
    s
}

#[test]
fn song_select_follows_the_drum_and_tab_moves_to_the_buttons() {
    let mut s = at_song_select(true);
    let (rim, face) = (s.cfg.keys[0], s.cfg.keys[1]);
    assert_eq!(s.drum_menu_key(rim), Some(KeyCode::ArrowUp));
    assert_eq!(s.drum_menu_key(face), Some(KeyCode::Enter));

    // Tab: the row above, on the search box; a face hit goes into it and the keys type.
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(
        matches!(s.scene, Scene::Select),
        "Tab doesn't open settings"
    );
    assert!(matches!(
        s.select_zone,
        drum_nav::Zone::Row { bottom: false, .. }
    ));
    assert_eq!(s.drum_menu_key(rim), Some(KeyCode::ArrowLeft));
    s.key(face, false, None, Instant::now(), None);
    assert!(s.select.searching);
    assert_eq!(s.drum_menu_key(rim), None, "typing now");

    // Tab leaves the box for the row below, and the list after it.
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(!s.select.searching);
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert_eq!(s.select_zone, drum_nav::Zone::List);
}

#[test]
fn tab_opens_settings_as_ever_without_drum_menus() {
    let mut s = at_song_select(false);
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(matches!(s.scene, Scene::Settings { .. }));
}

#[test]
fn time_in_songs_adds_up_over_plays_and_the_play_knows_its_map() {
    let mut s = in_a_song(false);
    let set = |s: &mut State, ms| {
        if let Scene::Play { game, .. } = &mut s.scene {
            game.set_played_ms(ms);
        }
    };
    set(&mut s, 1000.0);
    s.track_play();
    set(&mut s, 2500.0);
    s.track_play();
    s.track_play();
    assert_eq!(s.session_play_ms, 2500.0);
    let (_, map) = s.playing.clone().unwrap();
    let map = map.expect("the map goes with the play");
    assert_eq!(map.hash, s.library[0].hash());
    assert_eq!(map.name, s.library[0].label());

    // Retried: the next play's time starts from nothing, and adds to what's there.
    s.scene = Scene::Select;
    s.track_play();
    assert!(s.playing.is_none());
    let library = std::mem::take(&mut s.library);
    let mut next = in_a_song(false);
    next.library = library;
    next.session_play_ms = s.session_play_ms;
    if let Scene::Play { game, .. } = &mut next.scene {
        game.set_played_ms(500.0);
    }
    next.track_play();
    assert_eq!(next.session_play_ms, 3000.0);
}

#[test]
fn the_website_button_goes_to_the_site_whatever_the_map() {
    assert_eq!(map_url(0x1f3a), "https://rstaiko.auneu.space/m/1f3a");
}

fn watching_a_replay() -> State {
    let mut s = in_a_song(false);
    if let Scene::Play { game, .. } = &mut s.scene {
        game.set_replay(Some((Vec::new(), 0)));
    }
    s
}

fn held(s: &State) -> bool {
    matches!(&s.scene, Scene::Play { game, .. } if game.held())
}

#[test]
fn space_pauses_a_replay_without_a_menu_and_skips_in_a_play_as_ever() {
    let mut s = watching_a_replay();
    s.key(KeyCode::Space, false, None, Instant::now(), None);
    assert!(held(&s));
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused().is_none()));
    s.key(KeyCode::Space, false, None, Instant::now(), None);
    assert!(!held(&s));

    let mut s = in_a_song(false);
    s.key(KeyCode::Space, false, None, Instant::now(), None);
    assert!(!held(&s));
}

#[test]
fn a_held_replay_stays_where_it_is() {
    let mut s = watching_a_replay();
    s.key(KeyCode::Space, false, None, Instant::now(), None);
    let Scene::Play { game, .. } = &mut s.scene else {
        panic!()
    };
    let now = Instant::now();
    game.update(now, None, 0.0);
    let at = game.time();
    game.update(now + Duration::from_secs(5), None, 0.0);
    assert_eq!(game.time(), at);
}

#[test]
fn the_pause_menu_takes_over_from_a_held_replay() {
    let mut s = watching_a_replay();
    s.key(KeyCode::Space, false, None, Instant::now(), None);
    let Scene::Play { game, .. } = &mut s.scene else {
        panic!()
    };
    game.pause(None);
    assert!(!game.held(), "or Continue would leave it stopped");
}

#[test]
fn a_replays_speed_goes_in_steps_and_stops_at_the_ends() {
    let mut s = watching_a_replay();
    let up = |s: &mut State| s.key(KeyCode::ArrowUp, false, None, Instant::now(), None);
    let down = |s: &mut State| s.key(KeyCode::ArrowDown, false, None, Instant::now(), None);
    up(&mut s);
    assert_eq!(s.replay_speed_wanted, 1.25);
    for _ in 0..9 {
        up(&mut s);
    }
    assert_eq!(s.replay_speed_wanted, 2.0);
    for _ in 0..9 {
        down(&mut s);
    }
    assert_eq!(s.replay_speed_wanted, 0.25);
}

#[test]
fn leaving_a_replay_forgets_its_speed() {
    let mut s = watching_a_replay();
    s.key(KeyCode::ArrowUp, false, None, Instant::now(), None);
    s.scene = Scene::Select;
    s.poll_replay_speed();
    assert_eq!((s.replay_speed, s.replay_speed_wanted), (1.0, 1.0));
}

#[test]
fn a_config_from_before_plays_as_it_did_until_the_latency_is_known_then_without_a_jump() {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.cfg.mods = Mods::NONE;
    s.cfg.rate = 1.0;
    s.cfg.offset_ms = -25.0;
    s.cfg.offset_pending = true;
    s.refresh_play_offset();
    let entry = s.select.selected().expect("a map");
    let own = s.map_offset(Some(entry));
    assert_eq!(s.play_offset, -25.0 + own);
    // A latency still moving about doesn't convert it, and doesn't move the play offset.
    s.apply_latency(40.0, false);
    s.refresh_play_offset();
    assert_eq!((s.cfg.offset_pending, s.play_offset), (true, -25.0 + own));
    // Settled at 32 ms: -25 played here is -30 in osu!, and plays the same.
    s.apply_latency(32.0, true);
    s.refresh_play_offset();
    assert_eq!((s.cfg.offset_pending, s.cfg.offset_osu_ms), (false, -30.0));
    assert_eq!(s.play_offset, -25.0 + own, "no jump");
    // The latency changing later keeps the osu! offset and moves the play one.
    s.apply_latency(37.0, true);
    s.refresh_play_offset();
    assert_eq!((s.cfg.offset_osu_ms, s.play_offset), (-30.0, -30.0 + own));
}

#[test]
fn a_new_players_offset_is_osus_zero_and_plus_brings_the_notes_earlier() {
    let mut s = layout_tests::state();
    s.cfg = config::Config::default();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    s.cfg.mods = Mods::NONE;
    s.cfg.rate = 1.0;
    s.apply_latency(20.0, true);
    s.refresh_play_offset();
    assert_eq!(s.cfg.offset_osu_ms, 0.0);
    let entry = s.select.selected().expect("a map");
    let own = s.map_offset(Some(entry));
    let before = s.play_offset;
    assert_eq!(before, -23.0 - 20.0 + own);
    // The settings step and the in-play + both raise it, and the notes come earlier.
    s.adjust_setting(Item::Offset, 1, None);
    s.refresh_play_offset();
    assert_eq!(s.cfg.offset_osu_ms, 1.0);
    assert_eq!(s.play_offset, before - 1.0);
}

#[test]
fn the_offset_key_between_songs_raises_the_osu_offset() {
    let mut s = layout_tests::state();
    s.cfg = config::Config::default();
    s.scene = Scene::Select;
    assert!(s.song_entry().is_none());
    assert!(s.handle_global_key(KeyCode::Equal, None));
    assert_eq!(s.cfg.offset_osu_ms, 1.0);
    assert!(s.handle_global_key(KeyCode::Minus, None));
    assert!(s.handle_global_key(KeyCode::Minus, None));
    assert_eq!(s.cfg.offset_osu_ms, -1.0);
}

#[test]
fn the_offset_scale_setting_switches_the_number_not_the_timing() {
    let mut s = layout_tests::state();
    s.cfg = config::Config::default();
    s.scene = Scene::Select;
    s.cfg.mods = Mods::NONE;
    s.cfg.rate = 1.0;
    s.apply_latency(7.0, true);
    s.cfg.set_osu_offset(-20.0);
    s.refresh_play_offset();
    let before = s.play_offset;
    assert_eq!(s.setting_value(Item::OffsetScale), "osu!");
    s.adjust_setting(Item::OffsetScale, 1, None);
    assert_eq!(s.cfg.offset_scale, config::OffsetScale::Sound);
    assert_eq!(s.play_offset, before, "the timing doesn't move");
    assert_eq!(s.setting_value(Item::Offset), "+10 ms");
    // + still brings the notes earlier.
    s.adjust_setting(Item::Offset, 1, None);
    s.refresh_play_offset();
    assert_eq!(s.setting_value(Item::Offset), "+11 ms");
    assert_eq!(s.play_offset, before - 1.0);
}

/// A server from before single loved difficulties doesn't know `LovedMaps`: the game asks for the
/// sets instead, and takes each as loved whole; a newer one says which difficulties.
#[test]
fn loved_difficulties_come_from_the_server_and_an_older_one_gives_whole_sets() {
    use std::io::{BufRead, Write};
    for old in [true, false] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let reply = match serde_json::from_str::<online::Envelope>(&line) {
                    Ok(e) if matches!(e.request, online::Request::LovedSets) => {
                        online::Reply::LovedSets(vec![5])
                    }
                    Ok(e) if matches!(e.request, online::Request::LovedMaps) && !old => {
                        online::Reply::LovedMaps(vec![(5, vec!["Oni".into()])])
                    }
                    _ => online::Reply::Error("bad request".into()),
                };
                let mut out = serde_json::to_string(&reply).unwrap();
                out.push('\n');
                let _ = reader.get_mut().write_all(out.as_bytes());
            }
        });
        let mut s = layout_tests::state();
        s.cfg.online_server = server;
        let until = Instant::now() + Duration::from_secs(10);
        while s.select.server_loved.is_empty() && Instant::now() < until {
            s.poll_loved();
            std::thread::sleep(Duration::from_millis(5));
        }
        let want: Vec<String> = if old { vec![] } else { vec!["Oni".into()] };
        assert_eq!(
            s.select.server_loved.get(&5),
            Some(&want),
            "old server: {old}"
        );
    }
}
