//! Shared input behavior through the application's actual keyboard and pointer routes.
use super::harness;
use crate::text_input::Field;
use crate::*;

fn press(s: &mut State, chord: &str, typed: Option<&str>) {
    let c = keys::Chord::parse(chord).unwrap();
    s.modifiers_ctrl = c.ctrl;
    s.modifiers_super = c.super_key;
    s.modifiers_alt = c.alt;
    s.modifiers_shift = c.shift;
    s.key(c.key, false, typed, Instant::now(), None);
}
fn chat() -> State {
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.mp_menu.account = Some("tester".into());
    s.social.open = true;
    s.social.tab = social::Tab::Chat;
    s.social.input = "hello world".into();
    s
}

#[test]
fn chat_arrows_selection_delete_paste_and_undo_edit_the_insertion_point() {
    clipboard::fake_for_this_thread();
    let mut s = chat();
    press(&mut s, "Home", None);
    press(&mut s, "ArrowRight", None);
    press(&mut s, "KeyX", Some("X"));
    assert_eq!(s.social.input, "hXello world");
    press(&mut s, "Ctrl+Shift+ArrowRight", None);
    press(&mut s, "Ctrl+X", None);
    assert_eq!(clipboard::get().as_deref(), Some("ello "));
    assert_eq!(s.social.input, "hXworld");
    press(&mut s, "End", None);
    press(&mut s, "Ctrl+V", None);
    assert_eq!(s.social.input, "hXworldello ");
    press(&mut s, "Ctrl+Z", None);
    assert_eq!(s.social.input, "hXworld");
    press(&mut s, "Ctrl+A", None);
    press(&mut s, "KeyQ", Some("Привет"));
    assert_eq!(s.social.input, "Привет");
}

#[test]
fn search_inputs_share_navigation_and_keep_filtering_at_the_caret() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.open_settings();
    press(&mut s, "KeyA", Some("audio"));
    press(&mut s, "Home", None);
    press(&mut s, "KeyX", Some("x"));
    assert_eq!(s.settings_ui.search, "xaudio");
    press(&mut s, "Delete", None);
    assert_eq!(s.settings_ui.search, "xudio");
    s.open_choices(Item::UiFont);
    press(&mut s, "KeyA", Some("Arial"));
    press(&mut s, "Home", None);
    press(&mut s, "Delete", None);
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().search, "rial");
    press(&mut s, "Super+A", None);
    press(&mut s, "KeyI", Some("Inter"));
    assert_eq!(s.settings_ui.choices.as_ref().unwrap().search, "Inter");
}

#[test]
fn focused_song_search_uses_arrows_for_text_and_updates_the_song_filter() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.select.searching = true;
    s.select.filter = "abc".into();
    press(&mut s, "ArrowLeft", None);
    press(&mut s, "KeyX", Some("X"));
    assert_eq!(s.select.filter, "abXc");
    press(&mut s, "Shift+Home", None);
    press(&mut s, "Backspace", None);
    assert_eq!(s.select.filter, "c");
}

#[test]
fn mouse_placement_and_drag_selection_use_the_rendered_glyph_positions() {
    let mut s = chat();
    let at = Instant::now();
    let c = harness::frame(&mut s, at);
    let r = c
        .input_regions
        .iter()
        .find(|r| r.field == Field::Social)
        .unwrap()
        .clone();
    let y = r.rect[1] + r.rect[3] / 2.0;
    s.window_h = 480.0;
    s.window_w = c.width;
    let x = r.stops.iter().find(|(i, _)| *i == 2).unwrap().1;
    s.mouse_moved(x as f64, y as f64);
    s.mouse_button(true, None);
    assert_eq!(s.text_input.caret, 2);
    let end = r.stops.iter().find(|(i, _)| *i == 5).unwrap().1;
    s.mouse_moved(end as f64, y as f64);
    s.mouse_button(false, None);
    assert_eq!(s.text_input.selected(), "llo");
    press(&mut s, "KeyX", Some("!"));
    assert_eq!(s.social.input, "he! world");
}

#[test]
fn scrolling_keeps_the_caret_visible_and_does_not_expose_masked_text() {
    let mut c = Canvas::default();
    c.clear(240.0);
    let mut input = text_input::TextInput::new("abcdefghijklmnopqrstuvwxyz".into());
    c.input_editor = Some((Field::Social, input.view()));
    c.input(
        Field::Social,
        [10.0, 20.0, 50.0, 10.0],
        8.0,
        &input.text,
        "",
        rgba(0xffffff, 1.0),
        false,
        true,
    );
    let region = c.input_regions.last().unwrap();
    let end = region.stops.last().unwrap().1;
    assert!((10.0..=60.0).contains(&end));
    input.place(0, false);
    c.clear(240.0);
    c.input_editor = Some((Field::Social, input.view()));
    c.input(
        Field::Social,
        [10.0, 20.0, 50.0, 10.0],
        8.0,
        &input.text,
        "",
        rgba(0xffffff, 1.0),
        true,
        true,
    );
    assert_eq!(c.input_regions[0].stops[0], (0, 10.0));
    c.audit = true;
    c.input(
        Field::Social,
        [10.0, 20.0, 50.0, 10.0],
        8.0,
        &input.text,
        "",
        rgba(0xffffff, 1.0),
        true,
        true,
    );
    assert!(c.texts.iter().all(|(_, s)| !s.contains("abc")));
}

#[test]
fn changing_focus_and_submitting_chat_reset_the_component_for_the_new_value() {
    let mut s = chat();
    press(&mut s, "Home", None);
    s.social.input.clear();
    press(&mut s, "KeyX", Some("next"));
    assert_eq!(s.social.input, "next");
    s.social.open = false;
    s.open_settings();
    s.settings_ui.editing = Some(settings::Edit::new(Item::Offset, "123".into()));
    press(&mut s, "KeyX", Some("7"));
    assert_eq!(s.settings_ui.editing.as_ref().unwrap().text, "7");
}

#[test]
fn switching_between_equal_setting_values_does_not_reuse_the_old_selection() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    s.open_settings();
    s.settings_ui.editing = Some(settings::Edit::new(Item::Offset, "123".into()));
    press(&mut s, "Home", None);
    s.settings_ui.editing = Some(settings::Edit::new(Item::VisualOffset, "123".into()));
    press(&mut s, "KeyX", Some("7"));
    assert_eq!(s.settings_ui.editing.as_ref().unwrap().text, "7");
}

#[test]
fn first_click_in_another_account_field_focuses_it_and_places_the_caret() {
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.scene = MpMenu::scene();
    s.mp_menu.user = "tester".into();
    s.mp_menu.password = "password".into();
    s.mp_menu.select(multiplayer::Row::User);
    let c = harness::frame(&mut s, Instant::now());
    let region = c
        .input_regions
        .iter()
        .find(|r| r.field == Field::Account(1))
        .unwrap();
    let x = region.stops.iter().find(|s| s.0 == 2).unwrap().1;
    s.window_h = 480.0;
    s.window_w = c.width;
    s.mouse_moved(x as f64, (region.rect[1] + region.rect[3] / 2.0) as f64);
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert_eq!(s.mp_menu.row(), multiplayer::Row::Password);
    assert_eq!(s.text_input.caret, 2);
    press(&mut s, "KeyX", Some("X"));
    assert_eq!(s.mp_menu.password, "paXssword");
}

#[test]
fn double_click_selects_a_word_and_triple_click_selects_the_whole_line() {
    let mut e = text_input::TextInput::new("one Привет three".into());
    let at = Instant::now();
    let offset = "one При".len();
    e.click(offset, false, at);
    e.click(offset, false, at + Duration::from_millis(100));
    assert_eq!(e.selected(), "Привет");
    e.click(offset, false, at + Duration::from_millis(200));
    assert_eq!(e.selected(), e.text);
}

/// Song setup in the map editor (Discord: "can't use IME or keyboard shortcuts in the song setup
/// menu"): its boxes let the input method in and edit at a caret like the game's other boxes,
/// with arrows, Home/End, Ctrl+A, copy and paste.
#[test]
fn song_setup_boxes_take_the_input_method_arrows_and_shortcuts() {
    use editor::setup::Key;
    use editor::view::{Btn, Field as EdField, Keys, View};
    clipboard::fake_for_this_thread();
    let dir = std::env::temp_dir().join(format!("taiko-rs-setup-typing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.scene = MapEditor::scene(Box::new(View::open(path, 0).unwrap()));
    assert!(
        !s.takes_text(),
        "the editor's keys are its own until a box is typed into"
    );
    s.scene.screen_mut::<MapEditor>().unwrap().view.press(
        Btn::Edit(EdField::Meta(Key::Title)),
        Keys::default(),
        None,
    );
    assert!(s.takes_text(), "a Song setup box lets the input method in");
    let title = |s: &State| {
        s.scene
            .screen_as::<MapEditor>()
            .unwrap()
            .view
            .ed
            .setting(Key::Title)
    };
    // The box opens with its text selected: End keeps it, Home goes before it.
    press(&mut s, "End", None);
    press(&mut s, "Digit1", Some("!"));
    press(&mut s, "Home", None);
    press(&mut s, "KeyA", Some("A "));
    press(&mut s, "ArrowRight", None);
    press(&mut s, "Delete", None);
    harness::frame(&mut s, Instant::now());
    press(&mut s, "Shift+End", None);
    press(&mut s, "Ctrl+C", None);
    assert_eq!(clipboard::get().as_deref(), Some("st!"));
    // What the input method composes comes as text of its own (`Ime::Commit`).
    press(&mut s, "Ctrl+A", None);
    s.key(KeyCode::Lang1, false, Some("曲"), Instant::now(), None);
    press(&mut s, "Ctrl+V", None);
    harness::frame(&mut s, Instant::now());
    press(&mut s, "Enter", None);
    assert_eq!(title(&s), "曲st!");
    assert!(!s.takes_text(), "Enter keeps the text and leaves the box");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A number box in the map editor (Discord: "Can't paste in SV input field in map editor"):
/// Ctrl+V types the number on the clipboard, a comma read as a point and other characters
/// left out.
#[test]
fn editor_number_boxes_take_a_pasted_number() {
    use editor::view::{Btn, Field as EdField, Keys, View};
    clipboard::fake_for_this_thread();
    let dir = std::env::temp_dir().join(format!("taiko-rs-sv-paste-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.scene = MapEditor::scene(Box::new(View::open(path, 0).unwrap()));
    fn view(s: &mut State) -> &mut View {
        &mut s.scene.screen_mut::<MapEditor>().unwrap().view
    }
    // The green line's speed, typed by pasting.
    let green = view(&mut s).ed.chart.timing[1].id;
    view(&mut s).sel_line = Some(green);
    view(&mut s).press(Btn::Edit(EdField::Sv), Keys::default(), None);
    assert!(clipboard::set(" 1,25x\nmore"));
    press(&mut s, "Ctrl+V", None);
    press(&mut s, "Enter", None);
    let sv = view(&mut s).ed.chart.timing_point(green).unwrap().sv();
    assert!((sv - 1.25).abs() < 1e-9, "the pasted speed: {sv}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Several green lines picked (Discord idea "SV multiplier for editor": a finisher's line 1.5x
/// faster than the gradual SV around it): Multiply SV takes a typed factor to each picked green
/// line's own speed, a red line among them keeps its BPM, and one undo takes it back.
#[test]
fn editor_multiply_sv_scales_each_picked_green_line() {
    use editor::view::{Btn, Field as EdField, Keys, View};
    let dir = std::env::temp_dir().join(format!("taiko-rs-sv-times-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    let mut s = harness::state_with(harness::config(), Vec::new());
    harness::frame(&mut s, Instant::now());
    s.scene = MapEditor::scene(Box::new(View::open(path, 0).unwrap()));
    fn view(s: &mut State) -> &mut View {
        &mut s.scene.screen_mut::<MapEditor>().unwrap().view
    }
    // Two more green lines at speeds of their own, all three picked with the red one.
    let v = view(&mut s);
    let red = v.ed.chart.timing[0].clone();
    let mut greens = vec![v.ed.chart.timing[1].id];
    for (t, sv) in [(6000.0, 0.8), (8000.0, 0.5)] {
        greens.push(v.ed.add_green(t, sv).unwrap());
    }
    let before: Vec<f64> = greens
        .iter()
        .map(|id| v.ed.chart.timing_point(*id).unwrap().sv())
        .collect();
    v.sel_line = Some(greens[0]);
    v.sel_extra.extend(greens[1..].iter().copied());
    v.sel_extra.insert(red.id);
    v.press(Btn::Edit(EdField::SvTimes), Keys::default(), None);
    for chord in ["Digit1", "Period", "Digit5", "Enter"] {
        press(&mut s, chord, None);
    }
    let v = view(&mut s);
    let sv = |v: &View, id: u64| v.ed.chart.timing_point(id).unwrap().sv();
    for (id, was) in greens.iter().zip(&before) {
        assert!(
            (sv(v, *id) - was * 1.5).abs() < 1e-9,
            "{} vs {}",
            sv(v, *id),
            was * 1.5
        );
    }
    let now_red = v.ed.chart.timing_point(red.id).unwrap();
    assert_eq!(
        now_red.beat_length, red.beat_length,
        "the red line keeps its BPM"
    );
    // Shift + doubles them; one undo is one step back.
    v.press(
        Btn::Inc(EdField::SvTimes),
        Keys {
            shift: true,
            ..Keys::default()
        },
        None,
    );
    assert!((sv(v, greens[2]) - before[2] * 3.0).abs() < 1e-9);
    press(&mut s, "Ctrl+Z", None);
    let v = view(&mut s);
    assert!((sv(v, greens[2]) - before[2] * 1.5).abs() < 1e-9, "undone");
    let _ = std::fs::remove_dir_all(&dir);
}
