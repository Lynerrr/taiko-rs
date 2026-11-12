//! The friends UI in the map editor: a chip in the menu bar instead of the edge tab, and a panel
//! that leaves the keys to the editor until one of its fields is clicked.

use super::*;
use editor::view::View;

/// A state with the editor open on a small map, logged in.
fn editing() -> State {
    let mut s = crate::layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.0);
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-test-friends-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    s.scene = MapEditor::scene(Box::new(View::open(path, 0).unwrap()));
    s.mp_menu.account = Some("me".into());
    s
}

fn toggle_rect(s: &mut State) -> Option<[f32; 4]> {
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    s.hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::SocialToggle)
        .map(|h| h.rect)
}

#[test]
fn the_editor_scenes_have_no_edge_tab() {
    let mut s = editing();
    assert_eq!(s.social_tab_middle(), None);
    s.scene = crate::screens::editor_home::EditorHome::scene(editor::home::Home::new(&[], "me"));
    assert_eq!(s.social_tab_middle(), None);
    s.scene = SongSelect::scene();
    assert!(s.social_tab_middle().is_some(), "other screens keep it");
}

#[test]
fn the_chip_sits_in_the_menu_bar_clear_of_the_title_and_toggles_the_panel() {
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    let mut s = editing();
    let [x, y, w, h] = toggle_rect(&mut s).expect("a chip to click");
    assert!(y + h <= 14.0, "inside the menu bar");
    assert!(x >= View::menu_end(), "after the menus");
    assert!(
        x + w <= View::menu_end() + 6.0 + editor::view::FRIENDS_CHIP_ROOM,
        "inside the room the title leaves it"
    );
    assert!(!s.social.open);
    s.click(crate::hit::Hit::SocialToggle, None);
    assert!(s.social.open);
    s.click(crate::hit::Hit::SocialToggle, None);
    assert!(!s.social.open);
}

/// Presses and lets go of the left button at the middle of `r` (virtual units), as a player does.
fn mouse_click(s: &mut State, r: [f32; 4]) {
    let k = s.window_h / crate::render::VIRTUAL_HEIGHT;
    s.mouse_moved(
        f64::from((r[0] + r[2] / 2.0) * k),
        f64::from((r[1] + r[3] / 2.0) * k),
    );
    s.mouse_button(true, None);
    s.mouse_button(false, None);
}

#[test]
fn the_chip_and_notifications_take_a_real_mouse_click_in_the_editor() {
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    let mut s = editing();
    let chip = toggle_rect(&mut s).expect("a chip to click");
    mouse_click(&mut s, chip);
    assert!(s.social.open, "the mouse opens the panel");
    s.social.close();
    // A notification over the editor: clicking it opens the conversation it is about.
    s.social.friends = vec![crate::online::Friend {
        name: "ann".into(),
        activity: None,
    }];
    s.social.notify(
        "ann",
        "hi",
        0x4fb4ff,
        social::NoteAction::Chat("ann".into()),
    );
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    let note = s
        .hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::SocialNote(0))
        .map(|h| h.rect)
        .expect("the notification is drawn");
    mouse_click(&mut s, note);
    assert!(s.social.open && s.social.notes.is_empty());
}

#[test]
fn the_open_panel_leaves_editor_keys_alone_until_a_field_is_clicked() {
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    let mut s = editing();
    s.open_social();
    assert!(!s.social_keys());
    s.key_inner(KeyCode::KeyA, false, Some("a"), Instant::now(), None);
    assert!(s.social.input.is_empty(), "the letter went to the editor");
    // Esc closes the panel.
    s.key_inner(KeyCode::Escape, false, None, Instant::now(), None);
    assert!(!s.social.open);
    // Clicking the message box gives the panel the keys.
    s.open_social();
    s.click(crate::hit::Hit::TextField, None);
    assert!(s.social_keys());
    s.key_inner(KeyCode::KeyA, false, Some("a"), Instant::now(), None);
    assert_eq!(s.social.input, "a");
    // It's off again once closed.
    s.social.close();
    s.open_social();
    assert!(!s.social_keys());
}

#[test]
fn elsewhere_the_open_panel_takes_the_keyboard_at_once() {
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    let mut s = crate::layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.0);
    s.scene = SongSelect::scene();
    s.open_social();
    assert!(s.social_keys());
}
