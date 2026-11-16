//! The song list's right-click menu: the map's name once on top, then a row per action. What
//! can be done to the difficulty or to its whole set is one row with a button for each, a lit
//! button is already so (loved, EPIC) and takes it back, and deleting comes last, apart.

use super::harness::{self, *};
use super::maps;
use super::snapshot::assert_snapshot;
use crate::*;

/// A client with the test charts, the selected one in osu! set 42.
fn client(tag: &str) -> (State, Songs, usize) {
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    settle(&mut s, &mut now);
    let i = s.select.selected().expect("a map is selected");
    s.library[i].meta.set_id = Some(42);
    (s, songs, i)
}

fn open(s: &mut State, header: bool) {
    s.select.menu = Some(ui::SongMenu {
        previous: None,
        row: s.select.cursor(),
        header,
        skin: false,
        x: 100.0,
        y: 60.0,
    });
}

/// The menu alone as it draws itself, and its rows' words and buttons. With
/// `TAIKO_LAYOUT_SHOTS` set, also a picture of it in that folder, as `name`.
fn drawn(s: &State, name: &str) -> (crate::hit::Canvas, String) {
    let mut c = crate::hit::Canvas {
        audit: true,
        width: WIDTH,
        ..Default::default()
    };
    s.select.draw_menu(&mut c, &s.library);
    if let Some(dir) = std::env::var_os("TAIKO_LAYOUT_SHOTS") {
        let (w, h, rgb) = crate::debug::rasterize(&c, 2.0);
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        crate::debug::write_png(&path, w, h, &rgb).unwrap();
    }
    let text = screen(&c, &[]);
    (c, text)
}

/// Every item takes clicks in one place of its own, inside the menu.
fn each_item_once(s: &State, c: &crate::hit::Canvas) {
    let items = s.select.menu_items(&s.library);
    for (k, item) in items.iter().enumerate() {
        let n = c
            .hits
            .iter()
            .filter(|h| h.hit == crate::hit::Hit::SongMenu(k))
            .count();
        assert_eq!(n, 1, "{item:?} takes clicks in one place");
    }
}

#[test]
fn a_difficultys_menu_names_it_once_and_pairs_its_actions_with_its_sets() {
    let (mut s, _songs, i) = client("song-menu-difficulty");
    let (title, version) = (
        s.library[i].meta.title.clone(),
        s.library[i].meta.version.clone(),
    );
    s.select.nominator = true;
    s.select.admin = true;
    s.select.server_loved.insert(42, vec![version.clone()]);
    open(&mut s, false);

    let items = s.select.menu_items(&s.library);
    let rows: Vec<Vec<(String, Option<ui::MenuScope>, bool)>> = ui::menu_rows(&items)
        .into_iter()
        .map(|r| {
            items[r]
                .iter()
                .map(|it| (it.label.clone(), it.scope, it.on))
                .collect()
        })
        .collect();
    use ui::MenuScope::{Difficulty, Set};
    let pair = |label: &str, one_on: bool| {
        vec![
            (label.to_string(), Some(Difficulty), one_on),
            (label.to_string(), Some(Set), false),
        ]
    };
    let alone = |label: &str| vec![(label.to_string(), None, false)];
    assert_eq!(
        rows,
        [
            alone("Like"),
            alone("Edit"),
            alone("New difficulty from this"),
            pair("Collect", false),
            alone("Download again"),
            pair("Love", true),
            pair("EPIC", false),
            pair("Delete", false),
        ]
    );
    assert!(
        items
            .iter()
            .all(|it| !it.label.contains(&title) && !it.label.contains(&version)),
        "no row names the map again"
    );
    let parts: Vec<ui::MenuPart> = items.iter().map(|it| it.part).collect();
    assert!(
        parts.is_sorted_by_key(|p| *p as u8),
        "parts in order: {parts:?}"
    );
    assert_eq!(
        items.last().map(|it| it.action),
        Some(ui::MenuAction::DeleteSet),
        "deleting comes last"
    );

    let (c, text) = drawn(&s, "song-menu-difficulty");
    let named = |needle: &str| c.texts.iter().filter(|(_, t)| t == needle).count();
    assert_eq!(named(&title), 1, "the title once, on top");
    assert_eq!(named(&format!("[{version}]")), 1);
    each_item_once(&s, &c);
    assert_snapshot("song-menu-difficulty", &text);
}

#[test]
fn a_songs_own_row_offers_only_what_is_done_to_its_whole_set() {
    let (mut s, _songs, _i) = client("song-menu-song");
    open(&mut s, true);
    let items = s.select.menu_items(&s.library);
    assert!(
        items
            .iter()
            .all(|it| it.scope != Some(ui::MenuScope::Difficulty)),
        "{items:?}"
    );
    assert!(
        items.iter().all(|it| !matches!(
            it.action,
            ui::MenuAction::Love(..) | ui::MenuAction::Epic(..)
        )),
        "a player gets nothing of taiko.rs's"
    );
    let (c, text) = drawn(&s, "song-menu-song");
    each_item_once(&s, &c);
    assert_snapshot("song-menu-song", &text);
}

#[test]
fn a_phones_song_menu_has_rows_and_buttons_for_a_thumb() {
    let (mut s, _songs, _i) = client("song-menu-phone");
    s.select.nominator = true;
    open(&mut s, false);
    mobile::phone_for_tests::set(true);
    let (c, text) = drawn(&s, "song-menu-phone");
    mobile::phone_for_tests::set(false);
    each_item_once(&s, &c);
    for h in &c.hits {
        assert!(h.rect[3] >= 38.0, "{:?} is tall enough for a thumb", h.hit);
    }
    assert_snapshot("song-menu-phone", &text);

    // An admin's has the most rows: still on the screen, and still for a thumb.
    s.select.admin = true;
    mobile::phone_for_tests::set(true);
    let (c, _) = drawn(&s, "song-menu-phone-admin");
    mobile::phone_for_tests::set(false);
    each_item_once(&s, &c);
    for h in &c.hits {
        assert!(
            h.rect[1] >= 0.0 && h.rect[1] + h.rect[3] <= 480.0,
            "{:?} on screen",
            h.hit
        );
        assert!(h.rect[3] >= 36.0, "{:?} is tall enough for a thumb", h.hit);
    }
}
