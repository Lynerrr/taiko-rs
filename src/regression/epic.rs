//! Maps marked EPIC on taiko.rs: where the mark comes from (the server's list, none from a
//! server that doesn't know it), how song select and the download list show it (the pill in the
//! status's place, the row's title and word), the search finding it, and an admin marking a map from the song menu while
//! nobody else is offered to.

use super::harness::{self, *};
use super::maps;
use crate::hit::Hit;
use crate::*;
use std::io::{BufRead, Write as _};
use std::sync::{Arc, Mutex};

/// A server on a port of its own answering `answer` to each request it gets, and keeping them.
fn server(
    answer: impl Fn(&online::Request) -> online::Reply + Send + 'static,
) -> (String, Arc<Mutex<Vec<online::Request>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let kept = heard.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let reply = match serde_json::from_str::<online::Envelope>(&line) {
                Ok(e) => {
                    let reply = answer(&e.request);
                    kept.lock().unwrap().push(e.request);
                    reply
                }
                Err(_) => online::Reply::Error("bad request".into()),
            };
            let mut out = serde_json::to_string(&reply).unwrap();
            out.push('\n');
            let _ = reader.get_mut().write_all(out.as_bytes());
        }
    });
    (address, heard)
}

/// Song select over the regression charts, the selected one given osu! set 42 (the charts have
/// none of their own).
fn client(tag: &str) -> (State, Songs, Instant, usize) {
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let i = s.select.selected().expect("a map is selected");
    s.library[i].meta.set_id = Some(42);
    (s, songs, now, i)
}

/// Asks the server for its EPIC list again and waits for the answer.
fn ask_epic(s: &mut State, now: &mut Instant, server: String) {
    s.cfg.online_server = server;
    s.epic_asked = false;
    s.poll_epic();
    assert!(
        wait_for(s, now, |s| s.epic_call.is_idle()),
        "the server answers"
    );
}

#[test]
fn the_epic_list_comes_from_the_server_and_an_older_one_marks_none() {
    let (mut s, _songs, mut now, i) = client("epic-list");
    let (address, heard) = server(|r| match r {
        online::Request::EpicMaps => online::Reply::EpicMaps(online::EpicList {
            sets: vec![(42, Vec::new())],
            custom: vec![7],
        }),
        _ => online::Reply::Error("bad request".into()),
    });
    ask_epic(&mut s, &mut now, address);
    assert!(matches!(
        heard.lock().unwrap().as_slice(),
        [online::Request::EpicMaps]
    ));
    assert!(s.select.is_epic(&s.library[i]), "set 42, every difficulty");
    assert!(s.select.server_epic.custom.contains(&7));
    let others = (0..s.library.len()).filter(|&k| k != i);
    assert!(others.clone().all(|k| !s.select.is_epic(&s.library[k])));

    // A server from before the mark doesn't know the request: no map is EPIC, nothing is said.
    let (old, _) = server(|_| online::Reply::Error("bad request".into()));
    s.select.server_epic = ui::ServerEpic::default();
    s.toast = None;
    ask_epic(&mut s, &mut now, old);
    assert!(!s.select.is_epic(&s.library[i]));
    assert!(s.toast.is_none(), "no message about it");
}

/// Linux only, as the other screen goldens.
#[cfg(target_os = "linux")]
#[test]
fn an_epic_map_wears_its_mark_in_song_select() {
    let (mut s, _songs, mut now, i) = client("epic-screen");
    // Ranked on osu!, and without the mark it says so.
    s.set_status.insert(42, Some("ranked".into()));
    assert!(shows(&frame(&mut s, now), "RANKED"), "the status");
    // Marked EPIC: the EPIC pill stands in the status's place, as on the website.
    s.select.server_epic.sets.insert(42, Vec::new());
    let c = frame(&mut s, now);
    assert!(shows(&c, epic::LABEL), "the pill");
    assert!(!shows(&c, "RANKED"), "no status beside it");
    settle(&mut s, &mut now);
    now += Duration::from_millis(16);
    super::snapshot::assert_snapshot(
        "screen-song-select-epic",
        &screen(&frame(&mut s, now), super::screens::VOLATILE),
    );
    // Just another difficulty of the set marked: this one has no pill.
    s.select
        .server_epic
        .sets
        .insert(42, vec!["some other difficulty".into()]);
    assert!(!s.select.is_epic(&s.library[i]));
    // A custom set goes by its number on the website, not by osu!'s set.
    s.select.server_epic = ui::ServerEpic::default();
    let folder = s.library[i].path.parent().unwrap().to_path_buf();
    s.select.custom_folder.insert(folder, 7);
    s.select.server_epic.custom.insert(7);
    assert!(s.select.is_epic(&s.library[i]));
}

/// The download list's row: a set's status, or EPIC in its place when a map of it is marked.
#[test]
fn the_download_list_shows_epic_in_a_sets_status_place() {
    let mut b = browse::Browse::default();
    let ranked = download::SetInfo {
        id: 42,
        status: "ranked".into(),
        ..download::SetInfo::default()
    };
    let draw = |b: &browse::Browse| {
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
        let w = b.status_pill(&mut c, &ranked, 10.0, 10.0);
        (c, w)
    };
    let (c, plain) = draw(&b);
    assert!(shows(&c, "ranked") && !shows(&c, epic::LABEL));
    let mut marked = ui::ServerEpic::default();
    marked.sets.insert(42, vec!["Oni".into()]);
    b.set_epic(&marked);
    let (c, w) = draw(&b);
    assert!(shows(&c, epic::LABEL), "the EPIC pill");
    assert!(!shows(&c, "ranked"), "no status beside it");
    assert!(w > 0.0 && w != plain);
}

#[test]
fn status_epic_finds_the_marked_maps() {
    let (mut s, _songs, mut now, i) = client("epic-search");
    s.select.server_epic.sets.insert(42, Vec::new());
    s.select.filter = "status=epic".into();
    s.select.refilter(&s.library);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let rows: Vec<usize> = s.select.rows().to_vec();
    assert_eq!(rows, vec![i], "only the marked map");
}

#[test]
fn only_an_admin_marks_maps_epic_and_the_server_hears_it() {
    let (mut s, _songs, mut now, i) = client("epic-menu");
    let open_menu = |s: &mut State| {
        s.select.menu = Some(ui::SongMenu {
            previous: None,
            row: s.select.cursor(),
            header: false,
            skin: false,
            x: 100.0,
            y: 100.0,
        });
    };
    open_menu(&mut s);
    let labels = |s: &State| -> Vec<String> {
        s.select
            .menu_items(&s.library)
            .into_iter()
            .map(|it| it.label)
            .collect()
    };
    assert!(
        labels(&s).iter().all(|l| !l.contains("EPIC")),
        "a player, loved nominator or not, gets no EPIC items"
    );
    s.select.nominator = true;
    assert!(labels(&s).iter().all(|l| !l.contains("EPIC")));

    let (address, heard) = server(|r| match r {
        online::Request::SetEpic {
            map: online::EpicMap::Set { set, version },
            epic: true,
            ..
        } => online::Reply::EpicMaps(online::EpicList {
            sets: vec![(*set, version.iter().cloned().collect())],
            custom: Vec::new(),
        }),
        _ => online::Reply::Error("bad request".into()),
    });
    s.cfg.online_server = address;
    s.cfg.online_token = "admin-token".into();
    s.select.admin = true;
    let version = s.library[i].meta.version.clone();
    // One EPIC row, a button for the difficulty and one for its whole set.
    let epic = |s: &State| -> Vec<(usize, Option<ui::MenuScope>, bool, ui::MenuAction)> {
        s.select
            .menu_items(&s.library)
            .into_iter()
            .enumerate()
            .filter(|(_, it)| it.label == "EPIC")
            .map(|(k, it)| (k, it.scope, it.on, it.action))
            .collect()
    };
    let row = epic(&s);
    assert_eq!(
        row.iter()
            .map(|&(_, s, on, a)| (s, on, a))
            .collect::<Vec<_>>(),
        [
            (
                Some(ui::MenuScope::Difficulty),
                false,
                ui::MenuAction::Epic(true, true)
            ),
            (
                Some(ui::MenuScope::Set),
                false,
                ui::MenuAction::Epic(true, false)
            ),
        ]
    );

    let k = row[0].0;
    s.click(Hit::SongMenu(k), None);
    assert!(wait_for(&mut s, &mut now, |s| s.epic_call.is_idle()));
    match heard.lock().unwrap().as_slice() {
        [
            online::Request::SetEpic {
                token,
                map:
                    online::EpicMap::Set {
                        set: 42,
                        version: v,
                    },
                epic: true,
            },
        ] => {
            assert_eq!(token, "admin-token");
            assert_eq!(v.as_deref(), Some(version.as_str()));
        }
        other => panic!("the server heard {other:?}"),
    }
    assert!(
        s.select.is_epic(&s.library[i]),
        "the answer is the new list"
    );
    let toast = s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default();
    assert!(toast.contains("is EPIC on taiko.rs now"), "{toast}");

    // Marked now: the menu offers to take it off instead.
    open_menu(&mut s);
    let row = epic(&s);
    assert_eq!(
        row.iter()
            .map(|&(_, s, on, a)| (s, on, a))
            .collect::<Vec<_>>(),
        [
            (
                Some(ui::MenuScope::Difficulty),
                true,
                ui::MenuAction::Epic(false, true)
            ),
            (
                Some(ui::MenuScope::Set),
                false,
                ui::MenuAction::Epic(true, false)
            ),
        ]
    );
}

/// Whether a map is EPIC is worked out only when a search asks (`status=epic`), and then its
/// custom set is looked up only when it can change the answer: hashing every map's path made
/// searching a third slower.
#[test]
fn a_search_looks_a_maps_custom_set_up_only_when_it_can_matter() {
    let (s, _songs, _now, i) = client("epic-lazy");
    let e = &s.library[i];
    let mut epic = ui::ServerEpic::default();
    let unasked = || -> Option<u32> { panic!("the custom set was looked up") };
    assert!(!epic.covers(e, unasked), "nothing is marked");
    epic.sets.insert(7, Vec::new());
    assert!(!epic.covers(e, unasked), "another set is marked");
    epic.sets.insert(42, Vec::new());
    assert!(epic.covers(e, || None));
    assert!(
        !epic.covers(e, || Some(3)),
        "in a custom set it goes by that set"
    );
    epic.custom.insert(3);
    assert!(epic.covers(e, || Some(3)));
    assert!(crate::search::Query::parse("status=epic").asks_epic());
    assert!(!crate::search::Query::parse("artist").asks_epic());
}
