//! Test play from the editor (F5): the map as the editor has it plays in the real game, keeps
//! nothing, and leaving it (quit, the end) goes back to the editor as it was.

use super::*;
use editor::view::{Keys, View};

/// A state with the editor open on a small map in a folder of its own.
fn editing() -> State {
    let mut s = crate::layout_tests::state();
    // The library comes in on the first frame.
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.0);
    // A folder for each test: they run side by side.
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-test-play-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    let view = View::open(path, 0).unwrap();
    s.scene = MapEditor::scene(Box::new(view));
    s
}

fn frame(s: &mut State) {
    let mut c = Canvas::default();
    s.build_frame(&mut c, Instant::now(), 853.0);
}

/// Frames until the test play is on (its map loads on a thread).
fn until_playing(s: &mut State) {
    for _ in 0..500 {
        frame(s);
        if s.scene.is_play() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the test play never started");
}

fn press(s: &mut State, code: KeyCode, keys: Keys) {
    let Some(MapEditor { view, .. }) = s.scene.screen_mut::<MapEditor>() else {
        panic!("not in the editor");
    };
    view.key(code, false, keys, None);
}

#[test]
fn f5_plays_the_editors_map_as_it_is_and_quitting_comes_back_to_it() {
    let mut s = editing();
    let Some(MapEditor { view, .. }) = s.scene.screen_mut::<MapEditor>() else {
        unreachable!()
    };
    // An edit not saved yet: the test plays it anyway.
    view.ed.select_range(2000.0, 20000.0, false);
    assert_eq!(view.ed.delete_selected(), 3);
    view.set_time(1500.0);
    press(&mut s, KeyCode::F5, Keys::default());
    frame(&mut s);
    assert!(s.editor_back.is_some(), "the editor waits");
    until_playing(&mut s);
    {
        let (Some((_, from)), Some(game)) = (s.scene.song_play(), s.play.as_deref()) else {
            unreachable!()
        };
        assert_eq!(from, Some(1500.0), "from the playhead");
        assert_eq!(game.map().notes.len(), 2, "the map as the editor has it");
        assert!(!game.record(), "nothing is kept");
        assert!(game.mods().has(mods::Mod::NoFail));
        assert!(
            game.view
                .banner
                .as_deref()
                .is_some_and(|b| b.contains("TEST"))
        );
        assert!(game.view.test_play);
    }
    // A retry plays the editor's map again.
    s.restart_map();
    until_playing(&mut s);
    assert!(s.editor_back.is_some());
    // Quit (from the pause menu) goes back to the editor, as it was left.
    s.show_select();
    let Some(MapEditor { view, .. }) = s.scene.screen_as::<MapEditor>() else {
        panic!("quitting a test play went somewhere else");
    };
    assert_eq!(
        view.ed.chart.objects.len(),
        2,
        "the unsaved edit is still there"
    );
    assert_eq!(view.ed.time, 1500.0);
    assert!(s.editor_back.is_none());
    // Shift+F5: from the start.
    let shift = Keys {
        shift: true,
        ..Keys::default()
    };
    press(&mut s, KeyCode::F5, shift);
    frame(&mut s);
    until_playing(&mut s);
    let Some((_, from)) = s.scene.song_play() else {
        unreachable!()
    };
    assert_eq!(from, None);
    // Played to the end: no results, back to the editor.
    if let (true, Some(game)) = (s.scene.is_play(), s.play.as_deref_mut()) {
        game.run_out_of_notes();
        assert!(game.skip_outro());
    }
    for _ in 0..200 {
        frame(&mut s);
        if s.scene.screen_is::<MapEditor>() {
            break;
        }
    }
    assert!(
        s.scene.screen_is::<MapEditor>(),
        "the end of a test play has no results"
    );
    assert!(s.editor_back.is_none());
}

#[test]
fn a_map_not_in_the_song_list_is_not_test_played_and_says_why() {
    let mut s = editing();
    let Some(MapEditor { view, .. }) = s.scene.screen_mut::<MapEditor>() else {
        unreachable!()
    };
    view.entry = 9999;
    press(&mut s, KeyCode::F5, Keys::default());
    frame(&mut s);
    let Some(MapEditor { view, .. }) = s.scene.screen_as::<MapEditor>() else {
        panic!("it left the editor");
    };
    assert!(s.editor_back.is_none());
    assert!(view.status.contains("save it first"), "{}", view.status);
}

#[test]
fn tab_in_a_test_play_hands_it_to_autoplay_and_back_and_a_retry_keeps_it() {
    let mut s = editing();
    press(&mut s, KeyCode::F5, Keys::default());
    frame(&mut s);
    until_playing(&mut s);
    let autoplayed = |s: &State| {
        let (true, Some(game)) = (s.scene.is_play(), s.play.as_deref()) else {
            unreachable!()
        };
        assert!(!game.record(), "nothing is kept");
        assert!(game.view.test_play);
        let auto = game.mods().has(mods::Mod::Autoplay);
        assert_eq!(game.mods().has(mods::Mod::NoFail), !auto);
        assert_eq!(
            game.view
                .banner
                .as_deref()
                .is_some_and(|b| b.contains("AUTO")),
            auto
        );
        auto
    };
    assert!(!autoplayed(&s), "F5: by hand");
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(autoplayed(&s), "Tab: autoplay");
    s.restart_map();
    until_playing(&mut s);
    assert!(autoplayed(&s), "a retry is autoplayed again");
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    assert!(!autoplayed(&s), "Tab again: by hand");
    s.key(KeyCode::Tab, false, None, Instant::now(), None);
    // Back in the editor, the next test is by hand.
    s.show_select();
    assert!(s.scene.screen_is::<MapEditor>());
    press(&mut s, KeyCode::F5, Keys::default());
    frame(&mut s);
    until_playing(&mut s);
    assert!(!autoplayed(&s));
}

#[test]
fn friends_see_editing_through_the_editor_its_start_screen_and_a_test_play() {
    use online::Activity;
    let mut s = editing();
    assert!(matches!(s.activity(), Activity::Editing { map } if !map.is_empty()));
    // A test play of the map being edited is still editing: nothing to watch there.
    press(&mut s, KeyCode::F5, Keys::default());
    frame(&mut s);
    assert!(matches!(s.activity(), Activity::Editing { .. }), "loading");
    until_playing(&mut s);
    assert!(matches!(s.activity(), Activity::Editing { .. }), "playing");
    s.show_select();
    assert!(matches!(s.activity(), Activity::Editing { .. }), "back");
    // Leaving the editor goes back to the menus.
    s.editor_back = None;
    s.scene = Scene::Select;
    assert_eq!(s.activity(), Activity::Menus);
    // The editor's start screen has no map yet.
    s.open_editor_home();
    assert_eq!(s.activity(), Activity::Editing { map: String::new() });
}
