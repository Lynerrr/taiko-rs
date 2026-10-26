//! Typed setting values and the Songs folder setting.

use super::*;

/// The Songs folder and the game data folder are the whole program's: tests that change them
/// take turns.
static FOLDERS: Mutex<()> = Mutex::new(());

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
    assert_eq!(s.cfg.offset_ms, -23.0);
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
    assert!(s.apply_typed(Item::SongsFolder, &format!("\"{}\"", install.join("Songs").display())).is_ok());
    // Back to looking for osu! where it usually is.
    s.set_songs_folder(None).unwrap();
    assert_eq!(s.cfg.songs_folder, None);
    let _ = std::fs::remove_dir_all(&install);
}

#[test]
fn typed_note_colours_become_your_own_keeping_the_other() {
    let mut s = layout_tests::state();
    s.cfg.note_colours = multiverse::Colours { palette: 1, ..multiverse::Colours::CLASSIC };
    s.apply_typed(Item::KatColour, "#a0b0c0").unwrap();
    // Neon's don stays; the kat is the typed one.
    assert_eq!(s.cfg.note_colours, multiverse::Colours::own(multiverse::PALETTES[1].1, 0xa0b0c0));
    s.apply_typed(Item::DonColour, "102030").unwrap();
    assert_eq!((s.cfg.note_colours.don(), s.cfg.note_colours.kat()), (0x102030, 0xa0b0c0));
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
    s.cfg.offset_ms = 12.0;
    s.cfg.vsync_tune_ms = 3.0;
    s.start_calibration_as(true);
    assert!(matches!(s.scene, Scene::Calibrate { vsync: true, .. }), "not calibrating by sight");
    assert_eq!(s.wanted_present(), PresentChoice::Fifo);
    // H would hide the notes: not here.
    s.calibration_key(KeyCode::KeyH, false, Instant::now(), None);
    let Scene::Calibrate { game, .. } = &mut s.scene else { unreachable!() };
    assert!(!game.hide_notes);
    // Hit 8 ms late by sight on average: the frames show 8 ms later than the fine tune assumes.
    game.stats.errors = vec![8.0; 20];
    s.finish_calibration();
    s.leave_calibration(true);
    assert_eq!((s.cfg.vsync_tune_ms, s.cfg.offset_ms), (11.0, 12.0));
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
    assert!(s.move_data_to(Some(root.join("elsewhere"))).is_err(), "one move at a time");
    let start = Instant::now();
    while s.data_move.is_some() && start.elapsed() < Duration::from_secs(10) {
        s.poll_data_move();
    }
    let new = root.join("drive/taiko-rs");
    assert_eq!(s.cfg.data_folder, Some(new.clone()));
    assert_eq!(config::data_dir(), Some(new.clone()));
    assert_eq!(std::fs::read_to_string(new.join("replays/1.json")).unwrap(), "replay");
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
    assert!(groups.iter().all(|(_, l)| l.windows(2).all(|w| w[0].0 * w[0].1 >= w[1].0 * w[1].1)));
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
    s.scene = Scene::Play { game: Box::new(Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None)), entry: 0, from: None };
    let info = s.cfg.show_map_info;
    s.modifiers_shift = true;
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(s.cfg.interface_hidden);
    s.modifiers_shift = false;
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(s.cfg.interface_hidden, "plain Tab isn't the interface key");
    assert_ne!(s.cfg.show_map_info, info);
    // What's hidden reaches the game drawn.
    let mut c = Canvas { width: 853.3, ..Default::default() };
    s.build_frame(&mut c, Instant::now(), 853.3);
    let Scene::Play { game, .. } = &s.scene else { panic!() };
    assert_eq!(game.hidden, hud::Parts::DEFAULT_HIDES);
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
    s.scene = Scene::Play { game: Box::new(Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None)), entry: 0, from: None };
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
    let k = hud::Part::ALL.iter().position(|&p| p == hud::Part::HitAnimation).unwrap();
    s.pick_choice(k);
    assert!(!s.cfg.interface_hides.has(hud::Part::HitAnimation));
    assert_eq!(s.settings_ui.choices.as_ref().map(|c| c.cursor), Some(k));
    assert!(s.settings_ui.choices.as_ref().unwrap().names[k].starts_with("[ ]"));
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
fn sets_loved_on_the_server_count_as_loved() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.library[0].meta.set_id = Some(2275915);
    s.set_status.insert(2275915, Some("graveyard".into()));
    s.select.server_loved.insert(2275915);
    assert_eq!(s.map_status(0).map(|x| x.0), Some("LOVED HERE"));
    assert_eq!(map_gives_pp(&s.library[0], &s.set_status, &s.db_status, &s.select.server_loved), Some(true));
    // The search's status=l finds it.
    s.select.filter = "status=l".into();
    s.select.refilter(&s.library);
    assert_eq!(s.select.selected(), Some(0));
    // Only nominators get the menu item, and it says which way.
    s.select.filter.clear();
    s.select.refilter(&s.library);
    s.select.menu = Some(ui::SongMenu { row: 0, header: false, x: 0.0, y: 0.0 });
    assert!(!s.select.menu_items(&s.library).iter().any(|(_, a)| matches!(a, ui::MenuAction::Love(..))));
    s.select.nominator = true;
    assert!(s.select.menu_items(&s.library).iter().any(|(l, a)| l == "Unlove on taiko.rs" && *a == ui::MenuAction::Love(2275915, false)));
}

#[test]
fn hotkeys_can_be_moved_and_removed() {
    let mut s = layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    let now = Instant::now();
    // Pause on P: Esc no longer pauses, P does.
    s.cfg.hotkeys.set(keys::Hotkey::Pause, Some(keys::Chord::plain(KeyCode::KeyP)));
    let map = beatmap::calibration_map(500.0, 4, 64);
    s.scene = Scene::Play { game: Box::new(Game::new(map, Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None)), entry: 0, from: None };
    s.key(KeyCode::Escape, false, None, now, None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused.is_none()));
    s.key(KeyCode::KeyP, false, None, now, None);
    assert!(matches!(&s.scene, Scene::Play { game, .. } if game.paused.is_some()));

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
    s.settings_ui.cursor = settings::ITEMS.iter().position(|(_, it)| *it == Item::Hotkey(i)).unwrap();
    s.key(KeyCode::Enter, false, None, now, None);
    s.modifiers_ctrl = true;
    s.key(KeyCode::ControlLeft, false, None, now, None);
    s.key(KeyCode::KeyK, false, None, now, None);
    s.modifiers_ctrl = false;
    assert_eq!(s.cfg.hotkeys.get(h), Some(keys::Chord { key: KeyCode::KeyK, ctrl: true, shift: false, alt: false }));
    s.key(KeyCode::Enter, false, None, now, None);
    s.key(KeyCode::Delete, false, None, now, None);
    assert_eq!(s.cfg.hotkeys.get(h), None);
    s.key(KeyCode::Backspace, false, None, now, None);
    assert_eq!(s.cfg.hotkeys.get(h), Some(h.default()));

    // A drum key can be taken away too.
    s.settings_ui.cursor = settings::ITEMS.iter().position(|(_, it)| *it == Item::Key(0)).unwrap();
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
    let row = settings::ITEMS.iter().position(|(_, it)| *it == Item::KrokodilKoban).unwrap();
    s.settings_ui.cursor = row - 1;
    s.settings_ui.move_cursor(1);
    assert_ne!(s.settings_ui.item(), Item::KrokodilKoban, "skipped for anyone else");
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
    assert!(!s.hits.iter().any(|h| matches!(h.hit, render::Hit::SocialPanel)));
}

/// Song select with 40 scores on the selected map (the window is 1920x1080: 2.25 px per unit).
fn select_with_scores() -> State {
    let mut s = layout_tests::state();
    s.play_when_scanned = false;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    s.scene = Scene::Select;
    // A short title, so the panel's height is up to its preview (a long one wraps, and its
    // details alone fill the panel).
    let short = s.library.iter().find(|e| e.meta.title.len() < 12).expect("a short title").path.clone();
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
    point(&mut s, 24.0 + lw + 6.0, 240.0);
    assert_eq!(s.hit_at(24.0 + lw + 6.0, 240.0), Some(render::Hit::SplitList));
    // Only its grip in the middle, not the whole edge.
    assert_ne!(s.hit_at(24.0 + lw + 6.0, 120.0), Some(render::Hit::SplitList));
    s.mouse_button(true, None);
    point(&mut s, 24.0 + lw + 6.0 + 40.0, 240.0);
    assert!((s.cfg.song_list_share - (ui::LIST_SHARE.2 + 40.0 / w)).abs() < 0.002, "{}", s.cfg.song_list_share);
    point(&mut s, w, 240.0);
    s.mouse_button(false, None);
    assert!(s.split_drag.is_none());
    let widest = Select::list_width(w, s.cfg.song_list_share);
    assert!(s.cfg.song_list_share <= ui::LIST_SHARE.1 && w - 24.0 - widest - 12.0 - 24.0 >= 219.9, "the panel keeps its room: {widest}");
    // And far left: the list keeps its room too.
    frame(&mut s);
    point(&mut s, 24.0 + widest + 6.0, 240.0);
    s.mouse_button(true, None);
    point(&mut s, 0.0, 240.0);
    s.mouse_button(false, None);
    let narrowest = Select::list_width(w, s.cfg.song_list_share);
    assert!(s.cfg.song_list_share >= ui::LIST_SHARE.0 && narrowest >= 259.9, "{narrowest}");

    // A double click on it puts it back.
    frame(&mut s);
    point(&mut s, 24.0 + narrowest + 6.0, 240.0);
    for _ in 0..2 {
        s.mouse_button(true, None);
        s.mouse_button(false, None);
    }
    assert_eq!(s.cfg.song_list_share, ui::LIST_SHARE.2);

    // The panel's bottom edge, dragged up: the preview shrinks (no smaller than its limit) and
    // more of the leaderboard shows.
    frame(&mut s);
    let rows_before = s.leaderboard.len();
    let edge = s.preview.panel_bottom + 4.0;
    let left = 24.0 + Select::list_width(w, s.cfg.song_list_share) + 12.0;
    let mid = (left + w - 24.0) / 2.0;
    assert_ne!(s.hit_at(left + 20.0, edge), Some(render::Hit::SplitPreview));
    point(&mut s, mid, edge);
    assert_eq!(s.hit_at(mid, edge), Some(render::Hit::SplitPreview));
    s.mouse_button(true, None);
    point(&mut s, mid, edge - 1000.0);
    s.mouse_button(false, None);
    assert_eq!(s.cfg.song_preview_size, preview::PREVIEW_SIZE.0);
    frame(&mut s);
    assert!(s.preview.panel_bottom < edge - 4.0, "{} vs {edge}", s.preview.panel_bottom);
    assert!(s.leaderboard.len() > rows_before, "{} rows, {rows_before} before", s.leaderboard.len());

    // Settings > Gameplay puts the whole layout back.
    s.cfg.song_list_share = ui::LIST_SHARE.1;
    s.open_settings();
    s.settings_ui.cursor = settings::ITEMS.iter().position(|(_, it)| *it == Item::SongSelectLayout).unwrap();
    assert!(s.setting_value(Item::SongSelectLayout).starts_with("custom"));
    s.key(KeyCode::Enter, false, None, Instant::now(), None);
    assert_eq!((s.cfg.song_list_share, s.cfg.song_preview_size), (ui::LIST_SHARE.2, preview::PREVIEW_SIZE.1));
    assert_eq!(s.setting_value(Item::SongSelectLayout), "default");
}

#[test]
fn the_leaderboard_scrolls_with_the_wheel() {
    let mut s = select_with_scores();
    // Room for a few rows whatever the map's title (a long one wraps, making the panel taller).
    s.cfg.song_preview_size = preview::PREVIEW_SIZE.0;
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
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
    s.click(render::Hit::BoardTab(true), None);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert_eq!(s.board_scroll, 0);
}

#[test]
fn the_results_scroll_down_to_the_pp_and_back() {
    let mut s = select_with_scores();
    let mut r = ui::Results::new("map".into(), game::Stats { great: 10, ..Default::default() }, game::Windows::new(5.0));
    r.no_pp = Some("no pp here");
    s.scene = Scene::Results { results: r, entry: 0, from: None };
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
    s.click(render::Hit::Key(KeyCode::PageDown), None);
    assert_eq!(scroll(&s), ui::RESULTS_SCROLL);
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.3);
    assert!(s.hits.iter().any(|h| h.hit == render::Hit::Key(KeyCode::PageUp)), "and back up");
}
