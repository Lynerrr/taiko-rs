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
    s.scene = Scene::Editor(Box::new(View::open(path, 0).unwrap()));
    s.mp_menu.account = Some("me".into());
    s
}

fn toggle_rect(s: &mut State) -> Option<[f32; 4]> {
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.3);
    s.hits
        .iter()
        .find(|h| h.hit == render::Hit::SocialToggle)
        .map(|h| h.rect)
}

#[test]
fn the_editor_scenes_have_no_edge_tab() {
    let mut s = editing();
    assert_eq!(s.social_tab_middle(), None);
    s.scene = Scene::EditorHome(Box::new(editor::home::Home::new(&[], "me")));
    assert_eq!(s.social_tab_middle(), None);
    s.scene = Scene::Select;
    assert!(s.social_tab_middle().is_some(), "other screens keep it");
}

#[test]
fn the_chip_sits_in_the_menu_bar_clear_of_the_title_and_toggles_the_panel() {
    crate::render::set_animations(false);
    let mut s = editing();
    let [x, y, w, h] = toggle_rect(&mut s).expect("a chip to click");
    assert!(y + h <= 14.0, "inside the menu bar");
    assert!(x >= View::menu_end(), "after the menus");
    assert!(
        x + w <= View::menu_end() + 6.0 + editor::view::FRIENDS_CHIP_ROOM,
        "inside the room the title leaves it"
    );
    assert!(!s.social.open);
    s.click(render::Hit::SocialToggle, None);
    assert!(s.social.open);
    s.click(render::Hit::SocialToggle, None);
    assert!(!s.social.open);
}

#[test]
fn the_open_panel_leaves_editor_keys_alone_until_a_field_is_clicked() {
    crate::render::set_animations(false);
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
    s.click(render::Hit::TextField, None);
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
    crate::render::set_animations(false);
    let mut s = crate::layout_tests::state();
    s.build_frame(&mut Canvas::default(), Instant::now(), 853.0);
    s.scene = Scene::Select;
    s.open_social();
    assert!(s.social_keys());
}
