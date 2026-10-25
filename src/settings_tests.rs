//! Typed setting values and the Songs folder setting.

use super::*;

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
