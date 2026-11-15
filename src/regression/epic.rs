//! Maps marked EPIC on taiko.rs: where the mark comes from (the server's list, none from a
//! server that doesn't know it), how song select shows it (the pill by the status, the row's
//! title and word), the search finding it, and an admin marking a map from the song menu while
//! nobody else is offered to.

use super::harness::{self, *};
use super::maps;
use super::snapshot::assert_snapshot;
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
    // Ranked on osu!: the status pill stays, the EPIC one goes at its right.
    s.set_status.insert(42, Some("ranked".into()));
    s.select.server_epic.sets.insert(42, Vec::new());
    let c = frame(&mut s, now);
    assert!(shows(&c, epic::LABEL), "the pill");
    assert!(shows(&c, "RANKED"), "next to the status");
    settle(&mut s, &mut now);
    now += Duration::from_millis(16);
    assert_snapshot(
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
            .map(|(label, _)| label)
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
    let items = labels(&s);
    let mark = format!("Mark [{version}] EPIC");
    assert!(items.contains(&mark), "{items:?}");
    assert!(items.contains(&"Mark the whole set EPIC".to_string()));

    let k = items.iter().position(|l| *l == mark).unwrap();
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
    let items = labels(&s);
    assert!(
        items.contains(&format!("Take EPIC off [{version}]")),
        "{items:?}"
    );
}
