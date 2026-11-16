//! Configurable control chords, fullscreen switching, delayed retries and text input feedback.
use super::harness;
use crate::*;

fn press(s: &mut State, chord: &str, repeat: bool, at: Instant) {
    let c = keys::Chord::parse(chord).unwrap();
    s.modifiers_ctrl = c.ctrl;
    s.modifiers_shift = c.shift;
    s.modifiers_alt = c.alt;
    s.modifiers_super = c.super_key;
    s.key(c.key, repeat, None, at, None);
}

#[test]
fn fullscreen_supports_alt_enter_rebinding_disable_and_auto_repeat() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    let at = Instant::now();
    let initial = s.fullscreen;
    press(&mut s, "Alt+Enter", false, at);
    assert_eq!(s.fullscreen, !initial);
    press(&mut s, "Alt+Enter", true, at);
    assert_eq!(s.fullscreen, !initial);
    press(&mut s, "Alt+NumpadEnter", false, at);
    assert_eq!(s.fullscreen, initial);
    let h = keys::Hotkey::Fullscreen;
    s.cfg.hotkeys.set(h, keys::Chord::parse("Ctrl+Alt+F2"));
    press(&mut s, "Alt+Enter", false, at);
    press(&mut s, "F11", false, at);
    assert_eq!(
        s.fullscreen, initial,
        "both default keys stop after rebinding"
    );
    press(&mut s, "Ctrl+Alt+F2", false, at);
    assert_eq!(s.fullscreen, !initial);
    s.cfg.hotkeys.set(h, None);
    press(&mut s, "Ctrl+Alt+F2", false, at);
    assert_eq!(s.fullscreen, !initial);
}

#[test]
fn control_chords_round_trip_including_old_configs_and_system_modifiers() {
    let old = Config::parse_text("preset_keys = F5, F6\ninterface_key = Shift+Tab\n");
    assert_eq!(old.preset_keys[0].name(), "F5");
    assert_eq!(old.interface_key.name(), "Shift+Tab");
    let c = Config::parse_text(
        "preset_keys = Ctrl+Alt+F5, Super+F6\ninterface_key = Ctrl+Shift+Tab\nhotkey.fullscreen = Cmd+Enter\ninstant_restart = false\n",
    );
    let round = Config::parse_text(&c.to_text());
    assert_eq!(round.preset_keys, c.preset_keys);
    assert_eq!(round.interface_key, c.interface_key);
    assert_eq!(round.hotkeys, c.hotkeys);
    assert!(!round.instant_restart);
    assert_eq!(
        keys::Chord::parse("Cmd+Enter"),
        keys::Chord::parse("Super+Enter")
    );
    assert_eq!(keys::Chord::parse("Win+F6"), keys::Chord::parse("Super+F6"));
}

#[test]
fn binding_captures_modifiers_for_controls_but_drum_keys_remain_single_keys() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.open_settings();
    let now = Instant::now();
    for item in [
        Item::PresetKey(0),
        Item::InterfaceKey,
        Item::Hotkey(
            keys::Hotkey::ALL
                .iter()
                .position(|h| *h == keys::Hotkey::Fullscreen)
                .unwrap(),
        ),
    ] {
        s.settings_ui.binding = Some(item);
        press(&mut s, "Ctrl+Alt+Shift+Super+F2", false, now);
        let expected = keys::Chord::parse("Ctrl+Alt+Shift+Super+F2").unwrap();
        let actual = match item {
            Item::PresetKey(0) => s.cfg.preset_keys[0],
            Item::InterfaceKey => s.cfg.interface_key,
            _ => s.cfg.hotkeys.get(keys::Hotkey::Fullscreen).unwrap(),
        };
        assert_eq!(actual, expected);
    }
    s.settings_ui.binding = Some(Item::Key(0));
    press(&mut s, "Ctrl+Alt+Shift+Super+H", false, now);
    assert_eq!(s.cfg.keys[0], KeyCode::KeyH);
}

#[test]
fn interface_control_accepts_a_modified_drum_key_without_hitting_a_note() {
    let mut s = crate::settings_tests::in_a_song(false);
    let drum = s.cfg.keys[0];
    let c = keys::Chord {
        shift: true,
        ..keys::Chord::plain(drum)
    };
    s.cfg.interface_key = c;
    let before = s.cfg.interface_hidden;
    press(&mut s, &c.name(), false, Instant::now());
    assert_ne!(s.cfg.interface_hidden, before);
}

#[test]
fn manual_retry_fades_for_two_seconds_and_repeated_requests_do_not_extend_it() {
    for paused in [false, true] {
        let mut s = crate::settings_tests::in_a_song(paused);
        s.cfg.instant_restart = false;
        let at = Instant::now();
        press(&mut s, "Ctrl+R", false, at);
        assert!(s.scene.is_play() && s.pending_restart.is_some());
        s.poll_restart_delay(at);
        assert_eq!(s.restart_delay_progress(at), Some(0.0));
        s.poll_restart_delay(at + Duration::from_millis(1000));
        assert_eq!(
            s.restart_delay_progress(at + Duration::from_millis(1000)),
            Some(0.5)
        );
        press(&mut s, "Ctrl+R", false, at + Duration::from_millis(1000));
        s.poll_restart_delay(at + Duration::from_millis(1999));
        assert!(s.scene.is_play());
        s.poll_restart_delay(at + Duration::from_secs(2));
        assert!(!s.scene.is_play());
        assert!(s.pending_restart.is_none());
    }
}

#[test]
fn cancelling_delayed_retry_leaves_the_pause_state_unchanged() {
    for paused in [false, true] {
        let mut s = crate::settings_tests::in_a_song(paused);
        s.cfg.instant_restart = false;
        let at = Instant::now();
        press(&mut s, "Ctrl+R", false, at);
        s.poll_restart_delay(at);
        press(&mut s, "Escape", false, at + Duration::from_millis(500));
        assert!(s.pending_restart.as_ref().unwrap().released.is_some());
        assert_eq!(s.game().unwrap().paused().is_some(), paused);
        s.poll_restart_delay(at + Duration::from_secs(5));
        assert!(s.scene.is_play());
    }
}

#[test]
fn pause_menu_retry_is_instant_even_when_instant_restart_is_disabled() {
    let at = Instant::now();
    let mut instant = crate::settings_tests::in_a_song(false);
    instant.cfg.instant_restart = true;
    press(&mut instant, "Ctrl+R", false, at);
    assert!(!instant.scene.is_play());
    let mut delayed = crate::settings_tests::in_a_song(true);
    delayed.cfg.instant_restart = false;
    delayed.play.as_deref_mut().unwrap().set_paused(Some(1));
    press(&mut delayed, "Enter", false, at);
    assert!(delayed.pending_restart.is_none());
    assert!(!delayed.scene.is_play());
}

#[test]
fn slow_retry_tracks_the_held_key_and_fades_back_out_on_release() {
    for (binding, release) in [
        ("Backquote", KeyCode::Backquote),
        ("Super+Q", KeyCode::SuperLeft),
    ] {
        let mut s = crate::settings_tests::in_a_song(false);
        s.cfg.instant_restart = false;
        s.cfg
            .hotkeys
            .set(keys::Hotkey::QuickRetry, keys::Chord::parse(binding));
        let at = Instant::now();
        press(&mut s, binding, false, at);
        s.poll_restart_delay(at + Duration::from_secs(1));
        assert_eq!(
            s.restart_delay_progress(at + Duration::from_secs(1)),
            Some(0.5)
        );
        s.key_released_at(release, at + Duration::from_secs(1));
        assert!(s.pending_restart.as_ref().unwrap().released.is_some());
        assert_eq!(
            s.restart_delay_progress(at + Duration::from_millis(1100)),
            Some(0.3)
        );
        s.poll_restart_delay(at + Duration::from_millis(1300));
        assert!(s.pending_restart.is_none());
        s.poll_restart_delay(at + Duration::from_secs(3));
        assert!(s.scene.is_play(), "releasing the key must cancel the retry");
    }
}

#[test]
fn releasing_a_restart_chord_cancels_the_retry_while_paused() {
    for binding in ["Ctrl+R", "Backquote"] {
        let mut s = crate::settings_tests::in_a_song(true);
        s.play.as_deref_mut().unwrap().set_paused(Some(1));
        s.cfg.instant_restart = false;
        let at = Instant::now();
        press(&mut s, binding, false, at);
        s.key_released_at(
            keys::Chord::parse(binding).unwrap().key,
            at + Duration::from_millis(600),
        );
        assert!(s.pending_restart.as_ref().unwrap().released.is_some());
        s.poll_restart_delay(at + Duration::from_secs(3));
        assert!(s.scene.is_play());
        assert_eq!(s.game().unwrap().paused(), Some(1));
    }
}

#[test]
fn text_fields_use_an_ibeam_pointer_and_empty_fields_have_a_caret() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.window_w = 853.3;
    s.window_h = 480.0;
    for hit in [
        hit::Hit::TextField,
        hit::Hit::SettingsSearch,
        hit::Hit::ChoiceSearch,
        hit::Hit::LobbyChat,
    ] {
        let mut c = Canvas::default();
        c.clear(853.3);
        c.hit(10.0, 100.0, 120.0, 24.0, hit);
        s.hits = c.hits;
        assert_eq!(s.mouse_moved(50.0, 110.0), CursorIcon::Text);
    }
    let mut c = Canvas::default();
    c.caret(10.0, 20.0, 8.0, rgba(0xffffff, 1.0));
    assert_eq!(c.shapes.len(), 1);
    assert_eq!(c.shapes[0].half[0], 0.5);
    assert!(c.shapes[0].half[1] >= 4.0);
}

#[test]
fn assigning_a_secondary_default_resolves_the_conflict_without_shadowing_the_new_binding() {
    let mut b = keys::Bindings::default();
    let chord = keys::Chord::parse("Alt+Enter").unwrap();
    assert_eq!(
        b.set(keys::Hotkey::Quit, Some(chord)),
        Some(keys::Hotkey::Fullscreen)
    );
    assert_eq!(b.get(keys::Hotkey::Fullscreen), None);
    assert_eq!(
        b.resolve(chord, keys::Place::Other),
        keys::Resolved::As(keys::Hotkey::Quit.default())
    );
    assert_eq!(
        b.set(
            keys::Hotkey::Fullscreen,
            Some(keys::Hotkey::Fullscreen.default())
        ),
        Some(keys::Hotkey::Quit)
    );
    assert_eq!(b.get(keys::Hotkey::Quit), None);
}

#[test]
fn delayed_retry_draws_a_progressive_black_overlay_and_cancels_when_leaving_the_map() {
    let mut s = crate::settings_tests::in_a_song(false);
    s.cfg.instant_restart = false;
    let at = Instant::now();
    press(&mut s, "Ctrl+R", false, at);
    s.poll_restart_delay(at);
    let c = harness::frame(&mut s, at + Duration::from_secs(1));
    assert!(
        !c.hits
            .iter()
            .any(|h| matches!(h.hit, hit::Hit::PauseItem(_))),
        "holding retry must not open the pause menu over the running game"
    );
    assert!(
        c.shapes.iter().any(|shape| {
            shape.fill[..3] == [0.0; 3]
                && (shape.fill[3] - 0.475).abs() < 1e-3
                && (shape.half[0] * 2.0 - c.width).abs() < 1e-3
                && (shape.half[1] * 2.0 - 480.0).abs() < 1e-3
        }),
        "one second into the wait draws a half-strength full-screen fade"
    );
    s.show_select();
    s.poll_restart_delay(at + Duration::from_secs(5));
    assert!(s.pending_restart.is_none());
    assert!(s.scene.is_select());
}

#[test]
fn empty_chat_draws_its_caret_before_the_placeholder() {
    lang::set(lang::Lang::En);
    system_font::select("");
    render::set_text_grid(None);
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.social.open = true;
    s.social.tab = social::Tab::Chat;
    s.social.input.clear();
    let mut c = Canvas {
        audit: true,
        ..Default::default()
    };
    c.clear(853.3);
    social::draw(&mut c, &s.social, true, None, 0, false, false, None);
    let field = c
        .hits
        .iter()
        .find(|hit| hit.hit == hit::Hit::TextField)
        .expect("chat input");
    let [x, y, _, h] = field.rect;
    assert!(
        c.shapes.iter().any(|shape| {
            shape.half[0] == 0.5
                && shape.fill[3] > 0.0
                && (shape.center[0] - (x + 7.5)).abs() < 1e-3
                && (shape.center[1] - (y + h / 2.0)).abs() < 1e-3
        }),
        "an empty chat has a visible insertion point at the start, not after its hint"
    );
    super::snapshot::assert_snapshot("controls-empty-chat", &harness::screen(&c, &[]));
}

#[test]
fn hotkeys_execute_with_each_system_modifier_and_restore_the_physical_state() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    let at = Instant::now();
    for name in [
        "Ctrl+F2",
        "Alt+F2",
        "Shift+F2",
        "Super+F2",
        "Ctrl+Alt+Shift+Super+F2",
    ] {
        let chord = keys::Chord::parse(name).unwrap();
        s.cfg.hotkeys.set(keys::Hotkey::Screenshot, Some(chord));
        s.screenshot_due = false;
        press(&mut s, name, false, at);
        assert!(s.screenshot_due, "{name} executes the command");
        assert_eq!(
            s.chord(chord.key),
            chord,
            "dispatch restores the real modifiers"
        );
    }
}

#[test]
fn releasing_a_required_modifier_cancels_a_quick_retry_hold() {
    for (chord, release) in [
        ("Ctrl+Q", KeyCode::ControlLeft),
        ("Super+Q", KeyCode::SuperLeft),
        ("Shift+Q", KeyCode::ShiftLeft),
        ("Alt+Q", KeyCode::AltLeft),
    ] {
        let mut s = crate::settings_tests::in_a_song(false);
        s.cfg
            .hotkeys
            .set(keys::Hotkey::QuickRetry, keys::Chord::parse(chord));
        let at = Instant::now();
        press(&mut s, chord, false, at);
        assert!(s.retry_hold.is_some());
        s.key_released(release);
        s.poll_retry(at + Duration::from_secs(3));
        assert!(s.scene.is_play());
        assert!(s.retry_hold.is_none());
    }
}

#[test]
fn rebound_global_commands_work_with_a_picker_open_without_typing_into_it() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.open_settings();
    s.open_choices(Item::UiFont);
    s.cfg
        .hotkeys
        .set(keys::Hotkey::Screenshot, keys::Chord::parse("Super+F2"));
    press(&mut s, "Super+F2", false, Instant::now());
    assert!(s.screenshot_due);
    assert!(s.settings_ui.choices.as_ref().unwrap().search.is_empty());
}

#[test]
fn modified_delete_and_escape_can_be_assigned_without_clearing_or_cancelling_the_binding() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.open_settings();
    let i = keys::Hotkey::ALL
        .iter()
        .position(|h| *h == keys::Hotkey::Fullscreen)
        .unwrap();
    for name in ["Ctrl+Delete", "Alt+Escape"] {
        s.settings_ui.binding = Some(Item::Hotkey(i));
        press(&mut s, name, false, Instant::now());
        assert_eq!(
            s.cfg.hotkeys.get(keys::Hotkey::Fullscreen),
            keys::Chord::parse(name)
        );
        assert!(s.settings_ui.binding.is_none());
    }
    let before = s.cfg.hotkeys.get(keys::Hotkey::Fullscreen);
    s.settings_ui.binding = Some(Item::Hotkey(i));
    press(&mut s, "Escape", false, Instant::now());
    assert_eq!(s.cfg.hotkeys.get(keys::Hotkey::Fullscreen), before);
}

#[test]
fn mouse_pause_menu_retry_is_instant_and_a_new_hotkey_hold_can_start_during_fade_out() {
    let mut paused = crate::settings_tests::in_a_song(true);
    paused.cfg.instant_restart = false;
    paused.click(hit::Hit::PauseItem(1), None);
    assert!(!paused.scene.is_play());
    assert!(paused.pending_restart.is_none());

    let mut s = crate::settings_tests::in_a_song(false);
    s.cfg.instant_restart = false;
    let at = Instant::now();
    press(&mut s, "Ctrl+R", false, at);
    s.key_released_at(KeyCode::KeyR, at + Duration::from_secs(1));
    press(&mut s, "Ctrl+R", false, at + Duration::from_millis(1100));
    assert!(s.pending_restart.as_ref().unwrap().released.is_none());
    assert_eq!(
        s.restart_delay_progress(at + Duration::from_millis(1100)),
        Some(0.0)
    );
    s.poll_restart_delay(at + Duration::from_millis(3100));
    assert!(!s.scene.is_play());
}

#[test]
fn held_restart_keeps_play_running_and_accepts_drum_input_without_changing_the_held_key() {
    let mut s = crate::settings_tests::in_a_song(false);
    s.cfg.instant_restart = false;
    let at = Instant::now();
    press(&mut s, "Backquote", false, at);
    assert!(s.game().unwrap().paused().is_none());
    let drum = s.cfg.keys[0];
    let before = s.game().unwrap().presses();
    press(
        &mut s,
        &keys::Chord::plain(drum).name(),
        false,
        at + Duration::from_millis(500),
    );
    assert!(s.game().unwrap().paused().is_none());
    assert_ne!(
        s.game().unwrap().presses(),
        before,
        "drum input still reaches the game during the fade"
    );
    assert_eq!(
        s.pending_restart.as_ref().unwrap().held.unwrap().key,
        KeyCode::Backquote
    );
    s.key_released_at(drum, at + Duration::from_millis(600));
    assert!(s.pending_restart.as_ref().unwrap().released.is_none());
    s.key_released_at(KeyCode::Backquote, at + Duration::from_secs(1));
    assert!(s.game().unwrap().paused().is_none());
    assert!(s.game().unwrap().resuming().is_none());
    assert!(s.pending_restart.as_ref().unwrap().released.is_some());
}
