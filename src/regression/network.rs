//! The client's side of the network: the wire format of every message it sends and reads (kept
//! golden, so a rename can't silently break talking to the live server or older builds), and
//! the client against a scripted room that answers like the server does. The server's own logic
//! is not tested here.

use super::harness::{self, *};
use super::maps::{self, Player};
use super::snapshot::assert_snapshot;
use crate::net::{self, ClientMsg, ServerMsg};
use crate::online::{self, Down, Envelope, Reply, Request, Up};
use crate::*;
use std::fmt::Write as _;
use std::io::{BufRead, Write as _};
use std::sync::Mutex;
use taiko_proto::mapping;

// ---------------------------------------------------------------------------------------------
// Wire format.

fn map_info() -> net::MapInfo {
    net::MapInfo {
        hash: 0xfeed_beef_0000_0001,
        artist: "Artist".into(),
        title: "Title".into(),
        version: "Oni".into(),
        rate: 1.5,
        from: Some(30_000.0),
        set_id: Some(123),
        mods: vec!["HD".into(), "DT".into()],
        multiverse: Some("mirror".into()),
        shapeshift: None,
        seed: Some(99),
        defense: Some(net::Defense::DEFAULT),
        stats: Some(Box::new(net::MapStats {
            stars: 5.25,
            length_ms: 120_000.0,
            bpm: 180.0,
            od: 6.0,
            hp: 5.0,
            creator: "mapper".into(),
        })),
    }
}

fn live() -> net::Live {
    net::Live {
        score: 123_456,
        combo: 78,
        accuracy: 0.975,
        miss: 2,
        progress: 0.5,
        great: 70,
        ok: 6,
    }
}

fn result() -> net::Final {
    net::Final {
        score: 900_000,
        accuracy: 0.99,
        max_combo: 300,
        great: 295,
        ok: 5,
        miss: 0,
        ur: 95.5,
        quit: false,
    }
}

fn player(id: u32, name: &str, host: bool) -> net::PlayerInfo {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": name, "host": host, "ready": true, "has_map": true,
        "status": "Lobby", "live": live(), "result": null,
    }))
    .expect("a player as older servers send it")
}

fn part() -> net::MapPart {
    net::MapPart {
        name: "123 Artist - Title".into(),
        total: 10,
        offset: 4,
        data: "AAEC".into(),
        error: None,
    }
}

fn client_messages() -> Vec<ClientMsg> {
    vec![
        ClientMsg::Hello {
            name: "me".into(),
            protocol: net::PROTOCOL,
            host_token: Some(7),
            password: Some("pw".into()),
        },
        ClientMsg::Hello {
            name: "me".into(),
            protocol: net::PROTOCOL,
            host_token: None,
            password: None,
        },
        ClientMsg::Ping { t: 12.5 },
        ClientMsg::SelectMap(map_info()),
        ClientMsg::HasMap(true),
        ClientMsg::Ready(false),
        ClientMsg::StartMatch,
        ClientMsg::Loaded(true),
        ClientMsg::Live(live()),
        ClientMsg::Finished(result()),
        ClientMsg::RequestMap,
        ClientMsg::MapFile {
            to: 3,
            part: part(),
        },
        ClientMsg::Inputs {
            inputs: vec![(1023.0, 1), (1100.5, 3)],
            anti_mash: true,
        },
        ClientMsg::Spectate(true),
        ClientMsg::Spectator(false),
        ClientMsg::GiveHost(4),
        ClientMsg::Skipped,
        ClientMsg::Chat("hi".into()),
        ClientMsg::Mods(vec!["HR".into()]),
        ClientMsg::Library {
            maps: vec![(1, 2), (3, 4)],
            done: true,
        },
        ClientMsg::StartGuessr { rounds: 5 },
        ClientMsg::Guess { hash: 9, song: 10 },
        ClientMsg::Defense(net::DefenseAct::Buy {
            item: net::DefenseItem::Shield,
            at: 5000.0,
            until: 0.0,
        }),
        ClientMsg::Defense(net::DefenseAct::TagIn { at: 6000.0 }),
        ClientMsg::SetPassword(Some("secret".into())),
        ClientMsg::SetSlots(8),
        ClientMsg::Kick(5),
    ]
}

fn server_messages() -> Vec<ServerMsg> {
    vec![
        ServerMsg::Welcome { id: 2 },
        ServerMsg::Rejected("full".into()),
        ServerMsg::Pong {
            t: 10.0,
            server: 20.0,
        },
        ServerMsg::Room {
            players: vec![player(1, "host", true), player(2, "me", false)],
            map: Some(map_info()),
            in_match: false,
            mapguessr: false,
            common: Some(3),
            team: Some(net::TeamState::default()),
            locked: true,
            slots: 8,
        },
        ServerMsg::Prepare(map_info()),
        ServerMsg::Start { at: 5000.0 },
        ServerMsg::Results(vec![player(1, "host", true)]),
        ServerMsg::MapRequested { from: 2 },
        ServerMsg::MapFile(part()),
        ServerMsg::Spectate {
            map: map_info(),
            at: 100.0,
            players: vec![(1, "host".into())],
        },
        ServerMsg::Inputs {
            from: 1,
            inputs: vec![(1000.0, 0)],
            anti_mash: true,
        },
        ServerMsg::Skipped { from: 1 },
        ServerMsg::SkipVotes { votes: 1, of: 2 },
        ServerMsg::SkipNow,
        ServerMsg::Chat {
            from: "host".into(),
            text: "gl".into(),
        },
        ServerMsg::GuessRound {
            round: 1,
            rounds: 5,
            hash: 9,
            from: 0.25,
            seconds: 45,
        },
        ServerMsg::GuessWrong { tries: 2 },
        ServerMsg::GuessReveal {
            hash: 9,
            got: vec![(2, 100)],
            last: false,
        },
        ServerMsg::Defense {
            from: 1,
            act: net::DefenseAct::TagIn { at: 1.0 },
        },
        ServerMsg::Disconnected("gone".into()),
    ]
}

fn requests() -> Vec<Request> {
    vec![
        Request::Register {
            name: "me".into(),
            password: "pw".into(),
            email: Some("a@b.c".into()),
            code: None,
        },
        Request::Login {
            name: "me".into(),
            password: "pw".into(),
        },
        Request::ResetPassword {
            who: "me".into(),
            code: Some("123".into()),
            password: None,
        },
        Request::LinkEmail {
            token: "t".into(),
            email: "a@b.c".into(),
            code: None,
        },
        Request::WhoAmI { token: "t".into() },
        Request::Logout { token: "t".into() },
        Request::ListRooms,
        Request::CreateRoom {
            token: "t".into(),
            title: "room".into(),
            mapguessr: false,
            settings: net::RoomSettings {
                password: Some("pw".into()),
                slots: 4,
            },
        },
        Request::JoinRoom {
            token: "t".into(),
            room: 3,
        },
        Request::Connect { token: "t".into() },
        Request::SubmitScore {
            token: "t".into(),
            play: Box::new(online::ScoreSubmit {
                map: map_info(),
                score: 900_000,
                accuracy: 0.99,
                max_combo: 300,
                great: 295,
                ok: 5,
                miss: 0,
                pp: Some(123.4),
                date: 1_700_000_000,
                replay_version: crate::scores::new_version(),
                inputs: vec![(1023.0, 1)],
                stars: Some(5.25),
                play_ms: Some(120_000),
                scroll_speed: Some(1.0),
                md5: Some("abc".into()),
                pp_version: online::PP_VERSION,
                timing: None,
                rolls: Some(12),
            }),
        },
        Request::MapScores {
            map: 1,
            relax: false,
        },
        Request::MapScores {
            map: 1,
            relax: true,
        },
        Request::Profile { name: "me".into() },
        Request::Replay { id: 5 },
        Request::Score { id: 5 },
        Request::SetStatuses {
            token: "t".into(),
            maps: vec![(1, 2)],
        },
        Request::LovedSets,
        Request::LovedMaps,
        Request::SetLoved {
            token: "t".into(),
            set: 1,
            loved: true,
            version: Some("Oni".into()),
        },
        Request::Played {
            token: "t".into(),
            ms: 1000,
            map: None,
        },
        Request::Medals { name: "me".into() },
        Request::OpenMapping {
            token: "t".into(),
            set: mapping::SetDescriptor {
                title: "Artist - Song".into(),
                folder: "1 Artist - Song".into(),
                diffs: vec!["Oni".into(), "Muzukashii".into()],
                assets: vec![mapping::AssetRef {
                    name: "audio.mp3".into(),
                    sha256: "ab".into(),
                    size: 3,
                }],
            },
            mode: mapping::Mode::Guest,
        },
        Request::JoinMapping {
            token: "t".into(),
            session: 9,
        },
        Request::ResumeMapping {
            token: "t".into(),
            session: 9,
            epoch: 2,
            last_seq: 40,
        },
        Request::AcceptTerms {
            token: "t".into(),
            version: 1,
        },
    ]
}

fn ups() -> Vec<Up> {
    vec![
        Up::Status(online::Activity::Menus),
        Up::AddFriend("friend".into()),
        Up::RemoveFriend("friend".into()),
        Up::Watch("friend".into()),
        Up::StopWatching,
        Up::PlayStart(online::SoloPlay {
            map: map_info(),
            anti_mash: true,
        }),
        Up::PlayEnd,
        Up::Ping,
        Up::GlobalChat("hello".into()),
        Up::Features(vec![mapping::FEATURE.into()]),
    ]
}

fn downs() -> Vec<Down> {
    vec![
        Down::Notice("maintenance".into()),
        Down::WatchStart {
            player: "friend".into(),
            play: online::SoloPlay {
                map: map_info(),
                anti_mash: true,
            },
            t: 100.0,
            paused: false,
            inputs: vec![(50.0, 1)],
        },
        Down::WatchFrame {
            t: 200.0,
            paused: true,
            inputs: vec![],
        },
        Down::WatchEnd {
            player: "friend".into(),
        },
        Down::WatchStopped("offline".into()),
        Down::Pong,
        Down::Medals(vec![online::MedalNote::default()]),
        Down::GlobalChatDeleted {
            from: "x".into(),
            at: 5,
        },
        Down::Groups(vec![("dev".into(), vec!["DEV".into()])]),
        Down::MappingInvite {
            from: "host".into(),
            session: 9,
            title: "Artist - Song".into(),
            mode: mapping::Mode::Collab,
        },
        Down::MappingClosed { session: 9 },
    ]
}

fn replies() -> Vec<Reply> {
    vec![
        Reply::CodeSent("a@b.c".into()),
        Reply::EmailLinked("a@b.c".into()),
        Reply::LovedSets(vec![1, 2]),
        Reply::LovedMaps(vec![(1, vec!["Oni".into()])]),
        Reply::SetStatuses(vec![(1, "ranked".into())]),
        Reply::Counted,
        Reply::LoggedOut,
        Reply::Error("nope".into()),
        Reply::Medals(Box::default()),
        Reply::Scores(vec![online::OnlineScore::default()]),
        Reply::Profile(Box::default()),
        Reply::Score(Box::default()),
        Reply::Account {
            name: "a".into(),
            token: "t".into(),
            nominator: false,
            admin: false,
            email: Some(true),
            features: Vec::new(),
            terms: 0,
            server: None,
        },
        Reply::Mapping {
            session: 9,
            epoch: 2,
            me: 2,
            needs: vec!["ab".into()],
            done: 0,
        },
        Reply::Account {
            name: "a".into(),
            token: "t".into(),
            nominator: false,
            admin: false,
            email: Some(true),
            features: vec![mapping::FEATURE.into()],
            terms: 0,
            server: None,
        },
        Reply::Account {
            name: "a".into(),
            token: "t".into(),
            nominator: false,
            admin: false,
            email: Some(true),
            features: vec![online::TERMS_FEATURE.into()],
            terms: 1,
            server: None,
        },
        // A server that moved, telling the game where it is now.
        Reply::Account {
            name: "a".into(),
            token: "t".into(),
            nominator: false,
            admin: false,
            email: Some(true),
            features: Vec::new(),
            terms: 0,
            server: Some("tls://130.78.31.127:17801".into()),
        },
    ]
}

fn mapping_state() -> mapping::SessionState {
    mapping::SessionState {
        session: 9,
        title: "Song".into(),
        mode: mapping::Mode::Guest,
        host: 1,
        participants: vec![mapping::Participant {
            id: 1,
            name: "host".into(),
            role: mapping::Role::Host,
            colour: 0xF2D45C,
            online: true,
        }],
        documents: vec![mapping::Document {
            diff: 0,
            name: "Oni".into(),
            owner: Some(2),
            parts: vec![mapping::Part {
                from: 0.0,
                to: 1000.0,
                assignee: Some(2),
            }],
            locked: false,
            accepted: false,
        }],
        comments: vec![mapping::Comment {
            id: 1,
            diff: 0,
            time: 1000.0,
            author: 1,
            text: "01:00:000 (1) - too dense".into(),
            resolved: false,
        }],
        folder: "1 Artist - Song".into(),
        assets: vec![mapping::AssetRef {
            name: "audio.mp3".into(),
            sha256: "ab".repeat(32),
            size: 1024,
        }],
    }
}

fn mapping_op() -> mapping::MapOp {
    mapping::MapOp::Composite(vec![
        mapping::MapOp::InsertObjects(vec![mapping::Obj {
            id: (2 << mapping::EPOCH_SHIFT) | 1,
            line: "256,192,1000,1,0,0:0:0:0:".into(),
            fake: false,
            group: None,
        }]),
        mapping::MapOp::EditTiming {
            before: vec![mapping::Line {
                id: 1,
                line: "0,500,4,1,0,100,1,0".into(),
            }],
            after: vec![mapping::Line {
                id: 1,
                line: "0,400,4,1,0,100,1,0".into(),
            }],
        },
        mapping::MapOp::SetKey {
            section: "Metadata".into(),
            key: "Version".into(),
            before: Some("Oni".into()),
            after: Some("Guest's Oni".into()),
        },
    ])
}

fn mapping_ups() -> Vec<mapping::MapUp> {
    vec![
        mapping::MapUp::Op {
            client_seq: 3,
            diff: Some(0),
            op: mapping_op(),
        },
        mapping::MapUp::Snapshot {
            seq: 4,
            part: 0,
            parts: 1,
            data: "{}".into(),
        },
        mapping::MapUp::Presence {
            diff: 0,
            time: 1500.0,
            cursor: Some(1750.0),
            selection: vec![5],
        },
        mapping::MapUp::Chat("hi".into()),
        mapping::MapUp::Comment {
            diff: 0,
            time: 1000.0,
            text: "01:00:000 (1) - too dense".into(),
        },
        mapping::MapUp::Resolve {
            comment: 1,
            resolved: true,
        },
        mapping::MapUp::Host(mapping::HostCmd::Invite("anna".into())),
        mapping::MapUp::Host(mapping::HostCmd::Parts {
            diff: 0,
            parts: vec![mapping::Part {
                from: 0.0,
                to: 1000.0,
                assignee: None,
            }],
        }),
        mapping::MapUp::Host(mapping::HostCmd::Lock {
            diff: 1,
            locked: true,
            accepted: true,
        }),
        mapping::MapUp::Host(mapping::HostCmd::Close),
        mapping::MapUp::Asset {
            sha256: "ab".into(),
            offset: 0,
            total: 3,
            data: "AAEC".into(),
        },
        mapping::MapUp::GetAsset {
            sha256: "ab".into(),
            offset: 0,
        },
        mapping::MapUp::Ping,
    ]
}

fn mapping_downs() -> Vec<mapping::MapDown> {
    vec![
        mapping::MapDown::State(mapping_state()),
        mapping::MapDown::Op(mapping::Sequenced {
            seq: 5,
            author: 2,
            epoch: 2,
            client_seq: 3,
            diff: None,
            op: mapping_op(),
        }),
        mapping::MapDown::Refused {
            client_seq: 4,
            why: "only the host changes the timing".into(),
        },
        mapping::MapDown::Presence {
            who: 2,
            diff: 0,
            time: 1500.0,
            cursor: None,
            selection: Vec::new(),
        },
        mapping::MapDown::Chat {
            who: 1,
            text: "ok".into(),
            at: 7,
        },
        mapping::MapDown::SnapshotWanted { seq: 2000 },
        mapping::MapDown::Comment(mapping_state().comments[0].clone()),
        mapping::MapDown::Resolved {
            comment: 1,
            resolved: true,
            by: 1,
        },
        mapping::MapDown::AssetPart {
            sha256: "ab".into(),
            offset: 0,
            total: 3,
            data: "AAEC".into(),
        },
        mapping::MapDown::Closed("the host closed the session".into()),
        mapping::MapDown::Pong,
    ]
}

/// Each message as its JSON line, checked to read back into the same line.
fn wire<T: serde::Serialize + serde::de::DeserializeOwned>(
    out: &mut String,
    title: &str,
    msgs: &[T],
) {
    let _ = writeln!(out, "== {title}");
    for m in msgs {
        let line = serde_json::to_string(m).unwrap();
        let back: T = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert_eq!(serde_json::to_string(&back).unwrap(), line, "round trip");
        let _ = writeln!(out, "{line}");
    }
}

#[test]
fn every_message_keeps_its_wire_format() {
    let mut out = format!(
        "room protocol {} | online protocol {} | pp version {}\n",
        net::PROTOCOL,
        online::PROTOCOL,
        online::PP_VERSION
    );
    wire(&mut out, "room: client -> server", &client_messages());
    wire(&mut out, "room: server -> client", &server_messages());
    let envelopes: Vec<Envelope> = requests()
        .into_iter()
        .map(|request| Envelope {
            protocol: online::PROTOCOL,
            request,
        })
        .collect();
    wire(&mut out, "online: requests", &envelopes);
    wire(&mut out, "online: replies", &replies());
    wire(&mut out, "social: up", &ups());
    wire(&mut out, "social: down", &downs());
    wire(&mut out, "mapping: up", &mapping_ups());
    wire(&mut out, "mapping: down", &mapping_downs());
    assert_snapshot("network-wire-format", &out);
}

#[test]
fn messages_from_older_servers_still_read() {
    // A room message from before mapguessr, Defense, passwords and slots.
    let room: ServerMsg = serde_json::from_str(
        r#"{"Room":{"players":[{"id":1,"name":"h","host":true,"ready":false,"has_map":true,"status":"Lobby","live":{"score":0,"combo":0,"accuracy":1.0,"miss":0,"progress":0.0},"result":null}],"map":{"hash":1,"artist":"a","title":"t","version":"v"},"in_match":false}}"#,
    )
    .expect("an old Room message");
    let ServerMsg::Room {
        players,
        map,
        mapguessr,
        locked,
        slots,
        ..
    } = room
    else {
        panic!("not a room");
    };
    assert_eq!(players.len(), 1);
    assert!(!mapguessr && !locked);
    assert_eq!(slots, 16);
    let map = map.unwrap();
    assert!((map.rate - 1.0).abs() < 1e-9, "no rate means 1x");
    assert!(map.mods.is_empty() && map.defense.is_none());
    // An old lobby's Defense settings (before tag-ins, the shop and the maze).
    let d: net::Defense = serde_json::from_str(r#"{"lives":3,"bars":4}"#).unwrap();
    assert_eq!((d.tag_ins, d.shop, d.maze), (2, true, true));
    // A tower bought by an old build (no `until`).
    let act: net::DefenseAct =
        serde_json::from_str(r#"{"Buy":{"item":"Shield","at":1.0}}"#).unwrap();
    assert!(matches!(act, net::DefenseAct::Buy { until, .. } if until == 0.0));
    // An account reply from a server before features: no mapping sessions, so the game never
    // sends it anything about them.
    let account: Reply =
        serde_json::from_str(r#"{"Account":{"name":"a","token":"t","nominator":false}}"#).unwrap();
    let Reply::Account { features, .. } = account else {
        panic!("not an account");
    };
    assert!(features.is_empty() && !mapping::supported(&features));
    // A message this build doesn't know is skipped by the reader, not a disconnect: it just
    // doesn't parse as a ServerMsg.
    assert!(serde_json::from_str::<ServerMsg>(r#"{"FutureThing":{"x":1}}"#).is_err());
}

/// A mapping session's connection against a scripted server: the request and its reply, the
/// stream both ways, and the end of the connection reported.
#[test]
fn a_mapping_session_connects_streams_and_reports_the_end() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let first = online::read_line(&mut reader).unwrap();
        let env: Envelope = serde_json::from_str(&first).unwrap();
        assert!(matches!(
            env.request,
            Request::JoinMapping { session: 9, .. }
        ));
        online::write_line(
            &mut out,
            &Reply::Mapping {
                session: 9,
                epoch: 3,
                me: 2,
                needs: Vec::new(),
                done: 0,
            },
        )
        .unwrap();
        online::write_line(&mut out, &mapping::MapDown::State(mapping_state())).unwrap();
        // A line from a newer server is passed over, not a disconnect.
        out.write_all(b"{\"FutureThing\":1}\n").unwrap();
        online::write_line(&mut out, &mapping::MapDown::Pong).unwrap();
        let up = online::read_line(&mut reader).unwrap();
        let up: mapping::MapUp = serde_json::from_str(&up).unwrap();
        assert_eq!(up, mapping::MapUp::Chat("hi".into()));
        // The server goes away.
    });
    let link = mapping::Link::start(
        format!("127.0.0.1:{port}"),
        Request::JoinMapping {
            token: "t".into(),
            session: 9,
        },
    );
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while events.len() < 3 && Instant::now() < deadline {
        events.extend(link.poll());
        std::thread::yield_now();
    }
    assert!(matches!(
        events[0],
        mapping::LinkEvent::Joined {
            session: 9,
            epoch: 3,
            me: 2,
            ..
        }
    ));
    assert!(
        matches!(&events[1], mapping::LinkEvent::Down(d) if matches!(**d, mapping::MapDown::State(_)))
    );
    assert!(matches!(&events[2], mapping::LinkEvent::Down(d) if **d == mapping::MapDown::Pong));
    link.send(mapping::MapUp::Chat("hi".into()));
    server.join().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lost = false;
    while !lost && Instant::now() < deadline {
        lost = link
            .poll()
            .iter()
            .any(|e| matches!(e, mapping::LinkEvent::Lost(_)));
        std::thread::yield_now();
    }
    assert!(lost, "the end of the connection is reported");
}

#[test]
fn chat_and_names_are_cleaned_the_same() {
    let mut out = String::new();
    for text in [
        "hello",
        "  spaced   out  ",
        "tab\tand\nnewline",
        "emoji 🎵 ok",
        "what the fuck",
        "\u{202e}rtl override",
    ] {
        let _ = writeln!(
            out,
            "{text:?} -> chat {:?} censor {:?} clean_text {:?}",
            net::clean_chat(text),
            net::censor(text),
            online::clean_text(text, 20)
        );
    }
    for name in [
        "ok_name",
        "a",
        "has space",
        "x".repeat(40).as_str(),
        "кириллица",
        "-dash-",
    ] {
        let _ = writeln!(out, "valid_name({name:?}) = {}", online::valid_name(name));
    }
    for pw in ["", "short", "long enough password"] {
        let _ = writeln!(
            out,
            "valid_password({pw:?}) = {}",
            online::valid_password(pw)
        );
    }
    let _ = writeln!(
        out,
        "clean_title = {:?}",
        online::clean_title("  My   Room  ", "owner", false)
    );
    let _ = writeln!(out, "map label = {}", map_info().label());
    assert_snapshot("network-text-cleaning", &out);
}

#[test]
fn crash_reports_leave_without_the_players_names() {
    let report = "--- taiko.rs crash at unix 1, build b (0.1.0, windows)\n\
        thread main: panicked at src/editor/mod.rs:42:7:\n\
        could not open C:\\Users\\Kowalski\\AppData\\Roaming\\taiko-rs\\scores.db\n\
        also c:\\users\\kowalski\\Songs\\a.osu and C:/USERS/Kowalski/Songs/b.osu\n\
        mac /Users/anna/Library/taiko-rs, linux /home/anna/.local/share/taiko-rs\n\
        own folder D:\\KOWALSKI\\taiko-rs\\Songs, log: kowalski's map, not kowalskis\n\
        cyrillic C:\\Users\\Ковальски\\x and D:\\ковальски\\y\n\
        scene: Editor, map Artist - Title [Oni]\n";
    let out = crate::app::crash::scrub_report(report, "Kowalski");
    assert_eq!(
        out,
        "--- taiko.rs crash at unix 1, build b (0.1.0, windows)\n\
        thread main: panicked at src/editor/mod.rs:42:7:\n\
        could not open ~\\AppData\\Roaming\\taiko-rs\\scores.db\n\
        also ~\\Songs\\a.osu and ~/Songs/b.osu\n\
        mac ~/Library/taiko-rs, linux ~/.local/share/taiko-rs\n\
        own folder D:\\<user>\\taiko-rs\\Songs, log: <user>'s map, not kowalskis\n\
        cyrillic ~\\x and D:\\ковальски\\y\n\
        scene: Editor, map Artist - Title [Oni]\n",
        "user folders and the system user name are gone, the rest reads as before"
    );
    assert_eq!(
        crate::app::crash::scrub_report("D:\\Ковальски\\y", "КОВАЛЬСКИ"),
        "D:\\<user>\\y",
        "a Cyrillic user name is found in any case"
    );
    assert_eq!(
        crate::app::crash::scrub_report("ed is in the editor", "ed"),
        "ed is in the editor",
        "a name too short to tell from a word stays"
    );
}

#[test]
fn online_lines_are_framed_as_json_lines() {
    let mut buf = Vec::new();
    online::write_line(&mut buf, &Request::ListRooms).unwrap();
    assert_eq!(buf.last(), Some(&b'\n'));
    let mut reader = std::io::BufReader::new(&buf[..]);
    let line = online::read_line(&mut reader).unwrap();
    let back: Request = serde_json::from_str(&line).unwrap();
    assert!(matches!(back, Request::ListRooms));
}

// ---------------------------------------------------------------------------------------------
// A scripted room.

/// A room server on this machine that records what the client sends and sends what the test
/// tells it to. It answers pings itself with its clock equal to the client's.
pub struct FakeRoom {
    pub addr: String,
    got: Arc<Mutex<Vec<ClientMsg>>>,
    out: Arc<Mutex<Option<std::net::TcpStream>>>,
    last_ping: Arc<Mutex<f64>>,
}

impl FakeRoom {
    pub fn new() -> FakeRoom {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let got = Arc::new(Mutex::new(Vec::new()));
        let out: Arc<Mutex<Option<std::net::TcpStream>>> = Arc::new(Mutex::new(None));
        let last_ping = Arc::new(Mutex::new(0.0));
        let (g, o, p) = (got.clone(), out.clone(), last_ping.clone());
        std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            *o.lock().unwrap() = Some(stream.try_clone().unwrap());
            let reader = std::io::BufReader::new(stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<ClientMsg>(&line) else {
                    continue;
                };
                if let ClientMsg::Ping { t } = msg {
                    *p.lock().unwrap() = t;
                    send_to(&o, &ServerMsg::Pong { t, server: t });
                    continue;
                }
                g.lock().unwrap().push(msg);
            }
        });
        FakeRoom {
            addr,
            got,
            out,
            last_ping,
        }
    }

    pub fn send(&self, msg: &ServerMsg) {
        let until = Instant::now() + Duration::from_secs(5);
        while self.out.lock().unwrap().is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        send_to(&self.out, msg);
    }

    /// Everything the client has sent so far (pings left out).
    pub fn received(&self) -> Vec<String> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .map(|m| serde_json::to_string(m).unwrap())
            .collect()
    }

    /// The client's clock (ms since it connected) at its last ping: the room's clock too.
    pub fn clock(&self) -> f64 {
        *self.last_ping.lock().unwrap()
    }

    /// Waits until the client has sent something matching `what`.
    pub fn wait_for_msg(&self, what: impl Fn(&ClientMsg) -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if self.got.lock().unwrap().iter().any(&what) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    pub fn close(&self) {
        if let Some(s) = self.out.lock().unwrap().take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn send_to(out: &Mutex<Option<std::net::TcpStream>>, msg: &ServerMsg) {
    if let Some(s) = out.lock().unwrap().as_mut() {
        let mut line = serde_json::to_string(msg).unwrap();
        line.push('\n');
        let _ = s.write_all(line.as_bytes());
    }
}

/// Draws frames (the client reads the room in them) until the room got a message matching `what`.
fn pump(
    s: &mut State,
    now: &mut Instant,
    room: &FakeRoom,
    what: impl Fn(&ClientMsg) -> bool,
) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if room.got.lock().unwrap().iter().any(&what) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
        *now += Duration::from_millis(16);
        frame(s, *now);
    }
    false
}

/// The kind of a client message (its JSON key), for logs that don't depend on timing.
fn kind(json: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(m) => m.keys().next().cloned().unwrap_or_default(),
        _ => "?".into(),
    }
}

/// The kinds in order, repeats of one kind in a row shown once. Live and Inputs are left out:
/// the client batches them on a real-time timer (every 100 ms), so how many go out during a
/// test depends on how fast the machine runs it.
fn kinds(msgs: &[String]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for m in msgs {
        let k = kind(m);
        if k == "Live" || k == "Inputs" {
            continue;
        }
        if v.last() != Some(&k) {
            v.push(k);
        }
    }
    v
}

#[test]
fn the_client_says_hello_and_keeps_its_clock_with_the_room() {
    let room = FakeRoom::new();
    let mut client =
        net::Client::connect_with(&room.addr, "me", Some(42), Some("pw".into())).unwrap();
    assert!(room.wait_for_msg(|m| matches!(m, ClientMsg::Hello { .. })));
    assert_eq!(
        room.received()[0],
        serde_json::to_string(&ClientMsg::Hello {
            name: "me".into(),
            protocol: net::PROTOCOL,
            host_token: Some(42),
            password: Some("pw".into()),
        })
        .unwrap()
    );
    room.send(&ServerMsg::Welcome { id: 7 });
    let until = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while client.id.is_none() && Instant::now() < until {
        seen.extend(client.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(client.id, Some(7), "Welcome gives the client its id");
    assert!(
        seen.iter().all(|m| !matches!(m, ServerMsg::Pong { .. })),
        "pongs stay inside"
    );
    // The room's clock is the client's (see FakeRoom): its times map to local instants as sent.
    let t = room.clock() + 1000.0;
    let at = client.local_instant(t);
    let now = client.local_instant(room.clock());
    let diff = at.duration_since(now).as_secs_f64() * 1000.0;
    assert!(
        (diff - 1000.0).abs() < 50.0,
        "1 s on the room's clock is 1 s here ({diff} ms)"
    );
    // The room going away is reported as a message, not a hang.
    room.close();
    let until = Instant::now() + Duration::from_secs(5);
    let mut gone = false;
    while !gone && Instant::now() < until {
        gone = client
            .poll()
            .iter()
            .any(|m| matches!(m, ServerMsg::Disconnected(_)));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(gone, "a dropped room reads as Disconnected");
}

/// A client on song select with `chart` in its library, joined to `room` as player 2.
fn guest(tag: &str, chart: maps::Chart, room: &FakeRoom) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &[chart]);
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let client = net::Client::connect(&room.addr, "me", None).unwrap();
    s.mp = Some(multiplayer::joined(client, "regression room"));
    s.scene = crate::screens::lobby::Lobby::scene();
    room.send(&ServerMsg::Welcome { id: 2 });
    now += Duration::from_millis(16);
    frame(&mut s, now);
    (s, songs, now)
}

fn lobby_map(s: &State) -> net::MapInfo {
    let e = &s.library[0];
    net::MapInfo {
        hash: e.hash(),
        artist: e.meta.artist.clone(),
        title: e.meta.title.clone(),
        version: e.meta.version.clone(),
        rate: 1.0,
        from: None,
        set_id: None,
        mods: Vec::new(),
        multiverse: None,
        shapeshift: None,
        seed: None,
        defense: None,
        stats: None,
    }
}

fn room_msg(map: Option<net::MapInfo>, in_match: bool) -> ServerMsg {
    ServerMsg::Room {
        players: vec![player(1, "host", true), player(2, "me", false)],
        map,
        in_match,
        mapguessr: false,
        common: None,
        team: None,
        locked: false,
        slots: 16,
    }
}

#[test]
fn a_guest_plays_a_whole_match() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("mp-match", maps::basics(), &room);
    let mut log = String::new();
    let map = lobby_map(&s);
    // The lobby picks our map: we say we have it.
    room.send(&room_msg(Some(map.clone()), false));
    assert!(
        pump(&mut s, &mut now, &room, |m| matches!(
            m,
            ClientMsg::HasMap(true)
        )),
        "HasMap(true)"
    );
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.map.is_some())
    });
    let _ = writeln!(
        log,
        "lobby: {} have_map {}",
        scene_name(&s),
        s.mp.as_ref().unwrap().have_map
    );
    // The match: load, say so, start when the room says.
    room.send(&ServerMsg::Prepare(map.clone()));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "MpWait"));
    let _ = writeln!(log, "prepare: {}", scene_name(&s));
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::Loaded(true)
    )));
    room.send(&ServerMsg::Start {
        at: room.clock() + 200.0,
    });
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "MpPlay"),
        "the match starts"
    );
    let _ = writeln!(log, "start: {}", scene_name(&s));
    // Play it with the keyboard.
    let beatmap = harness::game(&s).unwrap().map().clone();
    let presses = maps::presses(&beatmap, Player::Perfect);
    let keys = s.cfg.keys;
    let mut next = 0;
    while harness::game(&s).is_some() {
        let horizon = now + Duration::from_millis(4);
        while let Some(&(t, k)) = presses.get(next) {
            let at = instant_of(&s, now, t);
            if at > horizon {
                break;
            }
            press(&mut s, keys[k as usize], at.max(now));
            next += 1;
        }
        now = horizon;
        frame(&mut s, now);
    }
    let _ = writeln!(log, "after the map: {}", scene_name(&s));
    assert!(
        pump(&mut s, &mut now, &room, |m| matches!(
            m,
            ClientMsg::Finished(_)
        )),
        "Finished is sent"
    );
    let finished = room
        .received()
        .into_iter()
        .find(|m| kind(m) == "Finished")
        .unwrap();
    let Ok(ClientMsg::Finished(f)) = serde_json::from_str(&finished) else {
        panic!("not a Finished message: {finished}");
    };
    // Rounded: press instants turn into map times with sub-nanosecond noise.
    let _ = writeln!(
        log,
        "finished: score {} acc {} max combo {} great {} ok {} miss {} ur {} quit {}",
        f.score,
        super::snapshot::num(f.accuracy, 4),
        f.max_combo,
        f.great,
        f.ok,
        f.miss,
        super::snapshot::num(f.ur, 2),
        f.quit
    );
    // The room's results.
    room.send(&ServerMsg::Results(vec![
        player(1, "host", true),
        player(2, "me", false),
    ]));
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.results.is_some())
    });
    let _ = writeln!(log, "results: {}", scene_name(&s));
    let _ = writeln!(log, "client sent: {}", kinds(&room.received()).join(" "));
    assert_snapshot("network-guest-match", &log);
}

#[test]
fn the_lobby_follows_the_room() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("mp-lobby", maps::basics(), &room);
    let mut log = String::new();
    // A map we don't have.
    let mut other = lobby_map(&s);
    other.hash ^= 1;
    room.send(&room_msg(Some(other.clone()), false));
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::HasMap(false)
    )));
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.map.is_some())
    });
    let _ = writeln!(
        log,
        "unknown map: have_map {}",
        s.mp.as_ref().unwrap().have_map
    );
    // Preparing a match on it: we can't load it.
    room.send(&ServerMsg::Prepare(other));
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::Loaded(false)
    )));
    let _ = writeln!(log, "prepare without the map: {}", scene_name(&s));
    // Chat, filtered.
    room.send(&ServerMsg::Chat {
        from: "host".into(),
        text: "what the fuck".into(),
    });
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| !m.chat.is_empty())
    });
    let mp = s.mp.as_ref().unwrap();
    let _ = writeln!(log, "chat: {:?} unread {}", mp.chat, mp.chat_unread);
    // Skip votes.
    room.send(&ServerMsg::SkipVotes { votes: 1, of: 2 });
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.skip_votes.is_some())
    });
    let _ = writeln!(log, "skip votes: {:?}", s.mp.as_ref().unwrap().skip_votes);
    // Typing in the lobby chat sends a line.
    press(&mut s, KeyCode::KeyT, now);
    type_text(&mut s, "hi all", now);
    press(&mut s, KeyCode::Enter, now);
    let sent = pump(
        &mut s,
        &mut now,
        &room,
        |m| matches!(m, ClientMsg::Chat(t) if t == "hi all"),
    );
    let _ = writeln!(log, "typed chat sent: {sent}");
    // Turned away: back to the multiplayer menu with the reason.
    room.send(&ServerMsg::Rejected("the room is full".into()));
    wait_for(&mut s, &mut now, |s| s.mp.is_none());
    let _ = writeln!(log, "rejected: {} status {:?}", scene_name(&s), s.mp_status);
    let _ = writeln!(log, "client sent: {}", kinds(&room.received()).join(" "));
    assert_snapshot("network-lobby", &log);
}

#[test]
fn a_dropped_connection_mid_match_goes_back_to_the_menu() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("mp-drop", maps::basics(), &room);
    let map = lobby_map(&s);
    room.send(&room_msg(Some(map.clone()), false));
    room.send(&ServerMsg::Prepare(map));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "MpWait"));
    room.close();
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "MpMenu"),
        "a lost room ends the match: {}",
        scene_name(&s)
    );
    assert!(s.mp.is_none());
    assert!(!s.mp_status.is_empty(), "and says why");
}

/// A scripted mapping server for one game: answers `JoinMapping` for session 9 with a guest
/// difficulty for player 2, the set's music, the snapshot and an op after it; then reads the
/// game's lines until its first edit, and ends the session. Other connections (the account
/// check, the social connection) are dropped. Hands back the game's lines.
fn scripted_mapping_server(
    listener: std::net::TcpListener,
    music: Vec<u8>,
) -> std::thread::JoinHandle<Vec<mapping::MapUp>> {
    use crate::editor::collab::{connect::sha256_hex, host_snapshot};
    use base64::Engine as _;
    std::thread::spawn(move || {
        loop {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let Ok(first) = online::read_line(&mut reader) else {
                continue;
            };
            let Ok(env) = serde_json::from_str::<Envelope>(&first) else {
                continue;
            };
            let Request::JoinMapping { session: 9, .. } = env.request else {
                continue;
            };
            let send = |out: &mut std::net::TcpStream, d: mapping::MapDown| {
                online::write_line(out, &d).unwrap();
            };
            online::write_line(
                &mut out,
                &Reply::Mapping {
                    session: 9,
                    epoch: 2,
                    me: 2,
                    needs: Vec::new(),
                    done: 0,
                },
            )
            .unwrap();
            let mut state = mapping_state();
            state.participants.push(mapping::Participant {
                id: 2,
                name: "anna".into(),
                role: mapping::Role::Mapper,
                colour: 0x5CC8F2,
                online: true,
            });
            state.documents.push(mapping::Document {
                diff: 1,
                name: "Anna's Inner".into(),
                owner: Some(2),
                parts: Vec::new(),
                locked: false,
                accepted: false,
            });
            state.documents[0].owner = None;
            state.documents[0].parts.clear();
            state.assets = vec![mapping::AssetRef {
                name: "audio.mp3".into(),
                sha256: sha256_hex(&music),
                size: music.len() as u64,
            }];
            send(&mut out, mapping::MapDown::State(state));
            let charts = vec![
                (
                    "Oni".to_string(),
                    crate::editor::chart::tests::MAP.to_string(),
                ),
                (
                    "Anna's Inner".to_string(),
                    crate::editor::chart::tests::MAP.replace("Oni", "Anna's Inner"),
                ),
            ];
            let snapshot = host_snapshot(&charts).unwrap();
            for up in mapping::snapshot_lines(4, &snapshot) {
                let mapping::MapUp::Snapshot {
                    seq,
                    part,
                    parts,
                    data,
                } = up
                else {
                    unreachable!()
                };
                send(
                    &mut out,
                    mapping::MapDown::Snapshot {
                        seq,
                        part,
                        parts,
                        data,
                    },
                );
            }
            // The host put a note in our difficulty just after the snapshot.
            send(
                &mut out,
                mapping::MapDown::Op(mapping::Sequenced {
                    seq: 5,
                    author: 1,
                    epoch: 1,
                    client_seq: 7,
                    diff: Some(1),
                    op: mapping::MapOp::InsertObjects(vec![mapping::Obj {
                        id: (1 << mapping::EPOCH_SHIFT) | 50,
                        line: "256,192,7000,1,0,0:0:0:0:".into(),
                        fake: false,
                        group: None,
                    }]),
                }),
            );
            let mut seen = Vec::new();
            loop {
                let line = online::read_line(&mut reader).unwrap();
                let up: mapping::MapUp = serde_json::from_str(&line).unwrap();
                match &up {
                    // As the real server: one piece for each ask.
                    mapping::MapUp::GetAsset { sha256, offset } => {
                        let from = (*offset as usize).min(music.len());
                        let to = (from + mapping::ASSET_PIECE).min(music.len());
                        send(
                            &mut out,
                            mapping::MapDown::AssetPart {
                                sha256: sha256.clone(),
                                offset: *offset,
                                total: music.len() as u64,
                                data: base64::engine::general_purpose::STANDARD
                                    .encode(&music[from..to]),
                            },
                        );
                    }
                    mapping::MapUp::Op { .. } => {
                        seen.push(up);
                        send(
                            &mut out,
                            mapping::MapDown::Closed("the host ended it".into()),
                        );
                        return seen;
                    }
                    _ => {}
                }
                seen.push(up);
            }
        }
    })
}

/// An invite to a mapping session, accepted on its card in the host's conversation: the game
/// fetches the set's music piece by piece and the snapshot
/// from a scripted server, opens the editor on the difficulty given to it with the op that
/// followed the snapshot, sends its own edit, and edits alone again when the session ends.
/// The social connection declares `mapping/1` from its own reply, even while the account check
/// started with it is still out: a game that waited for that check never declared it, and the
/// server sent its invites as a plain notice with nothing to join from.
#[test]
fn the_social_link_declares_mapping_before_the_account_check_answers() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<Up>();
    std::thread::spawn(move || {
        // The account checks, held open and never answered.
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let Ok(first) = online::read_line(&mut reader) else {
                continue;
            };
            let Ok(env) = serde_json::from_str::<Envelope>(&first) else {
                continue;
            };
            match env.request {
                Request::Connect { .. } => {
                    online::write_line(
                        &mut out,
                        &Reply::Account {
                            name: "me".into(),
                            token: "t".into(),
                            nominator: false,
                            admin: false,
                            email: Some(true),
                            features: vec![mapping::FEATURE.into()],
                            terms: 0,
                            server: None,
                        },
                    )
                    .unwrap();
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        while let Ok(line) = online::read_line(&mut reader) {
                            if let Ok(up) = serde_json::from_str::<Up>(&line) {
                                let _ = tx.send(up);
                            }
                        }
                    });
                    held.push(out);
                }
                _ => held.push(out),
            }
        }
    });
    let mut cfg = harness::config();
    cfg.online_server = format!("127.0.0.1:{port}");
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    let mut s = state_with(cfg, Vec::new());
    let mut now = Instant::now();
    assert!(!s.mapping_sessions, "nothing heard from the server yet");
    assert!(
        wait_for(&mut s, &mut now, |s| s.social.connected),
        "the social link connects"
    );
    assert!(
        s.mp_menu.account_request.is_some(),
        "the account check is still out"
    );
    assert!(s.mapping_sessions, "the social reply's features count");
    let until = Instant::now() + Duration::from_secs(10);
    let mut declared = false;
    while !declared && Instant::now() < until {
        frame(&mut s, now);
        now += Duration::from_millis(16);
        if let Ok(up) = rx.recv_timeout(Duration::from_millis(50)) {
            declared = matches!(up, Up::Features(ref f) if f == &[mapping::FEATURE.to_string()]);
        }
    }
    assert!(declared, "the game declares mapping/1 on the social link");
}

#[test]
fn an_invited_player_joins_a_mapping_session_and_edits_in_it() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Music longer than one piece, as any song is.
    let music: Vec<u8> = (0..mapping::ASSET_PIECE as u32 * 2 + 5000)
        .map(|i| (i % 253) as u8)
        .collect();
    let server = scripted_mapping_server(listener, music.clone());
    let songs = Songs::new("mapping-join", &[]);
    std::fs::create_dir_all(&songs.dir).unwrap();
    let mut cfg = harness::config();
    cfg.online_server = format!("127.0.0.1:{port}");
    let mut s = state_with(cfg, Vec::new());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.songs_dir = songs.dir.clone();
    s.cfg.online_token = "t".into();

    // A server without the feature: the invite (were it sent) leads nowhere.
    s.join_mapping(9);
    assert!(s.mapping_join.is_none(), "nothing asked of a legacy server");
    assert!(s.social_features().is_empty());

    s.mapping_sessions = true;
    assert_eq!(s.social_features(), vec![mapping::FEATURE.to_string()]);
    s.social_down(Down::MappingInvite {
        from: "host".into(),
        session: 9,
        title: "Artist - Song".into(),
        mode: mapping::Mode::Guest,
    });
    // The invite is a card in the conversation with the host (and a notification that opens
    // that conversation); accepting it there joins.
    let note = s
        .social
        .notes
        .iter()
        .position(|n| n.action == social::NoteAction::Chat("host".into()))
        .expect("a notification of the invite");
    assert!(s.social.notes[note].title.contains("host"));
    let line = s.social.chats["host"].last().unwrap().text.clone();
    assert_eq!(line, social::invite_line(9));
    assert!(matches!(
        s.social.card_of(&line),
        Some(social::Card::Invite(9, ref i)) if i.title == "Artist - Song"
            && i.mode == mapping::Mode::Guest
            && i.state == social::InviteState::Open
    ));
    assert_eq!(s.social.unread.get("host"), Some(&1));
    s.click(crate::hit::Hit::MappingInvite(9, true), None);
    assert!(s.mapping_join.is_some(), "joining");
    assert_eq!(s.social.invites[&9].state, social::InviteState::Joined);
    s.click(crate::hit::Hit::MappingInvite(9, true), None);
    let joined = |s: &State| {
        s.scene
            .screen_as::<MapEditor>()
            .is_some_and(|e| e.view.collab.is_some())
    };
    assert!(
        wait_for(&mut s, &mut now, joined),
        "the editor opens on the session"
    );
    let folder = songs.dir.join("1 Artist - Song");
    assert_eq!(std::fs::read(folder.join("audio.mp3")).unwrap(), music);
    {
        let view = &mut s.scene.screen_mut::<MapEditor>().unwrap().view;
        assert_eq!(
            view.ed.path,
            folder.join("Song [Anna's Inner].osu"),
            "the guest difficulty given to us"
        );
        assert!(
            view.ed.chart.objects.iter().any(|o| o.time == 7000.0),
            "the op after the snapshot"
        );
        // The session panel opens on joining, under its menu-bar button, and says what we map.
        assert!(view.session_panel_open());
        assert_eq!(
            view.collab.as_ref().unwrap().rights(),
            crate::editor::collab::session::Rights::Diff
        );
        view.ed.place_hit(8000.0, false).unwrap();
    }
    let alone = |s: &State| {
        s.scene
            .screen_as::<MapEditor>()
            .is_some_and(|e| e.view.collab.is_none())
    };
    assert!(wait_for(&mut s, &mut now, alone), "the session ends");
    let seen = server.join().unwrap();
    assert!(
        seen.iter().any(|u| matches!(
            u,
            mapping::MapUp::Op {
                client_seq: 1,
                diff: Some(1),
                ..
            }
        )),
        "our edit went out: {seen:?}"
    );
    let view = &s.scene.screen_as::<MapEditor>().unwrap().view;
    assert!(view.ed.chart.objects.iter().any(|o| o.time == 8000.0));
    assert!(view.link.is_none() && view.relink.is_none());
}

/// Sessions live in the server's memory: after a restart, a resume is refused, and the link
/// says so (the game then ends the session instead of trying again).
#[test]
fn a_resume_the_server_refuses_is_reported_as_refused() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let first = online::read_line(&mut reader).unwrap();
        let env: Envelope = serde_json::from_str(&first).unwrap();
        assert!(matches!(
            env.request,
            Request::ResumeMapping {
                session: 9,
                epoch: 2,
                last_seq: 40,
                ..
            }
        ));
        online::write_line(&mut out, &Reply::Error("no such session".into())).unwrap();
    });
    let link = mapping::Link::start(
        format!("127.0.0.1:{port}"),
        Request::ResumeMapping {
            token: "t".into(),
            session: 9,
            epoch: 2,
            last_seq: 40,
        },
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    while events.is_empty() && Instant::now() < deadline {
        events.extend(link.poll());
        std::thread::yield_now();
    }
    server.join().unwrap();
    assert!(
        matches!(&events[..], [mapping::LinkEvent::Refused(why)] if why == "no such session"),
        "{events:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// The server moving to another address.

/// A server that answers every account check for `name` (with `moved_to` as where it says it
/// is now), and holds every other connection open unanswered. Its address, and how many
/// account checks it got.
fn account_server(name: &str, moved_to: Option<String>) -> (String, Arc<AtomicUsize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let checks = Arc::new(AtomicUsize::new(0));
    let counted = checks.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let Ok(line) = online::read_line(&mut reader) else {
                continue;
            };
            match serde_json::from_str::<Envelope>(&line).map(|env| env.request) {
                Ok(Request::WhoAmI { .. }) => {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let _ = online::write_line(
                        &mut out,
                        &Reply::Account {
                            name: name.clone(),
                            token: "t".into(),
                            nominator: false,
                            admin: false,
                            email: Some(true),
                            features: Vec::new(),
                            terms: 0,
                            server: moved_to.clone(),
                        },
                    );
                }
                _ => held.push(out),
            }
        }
    });
    (address, checks)
}

/// A game logged in on `server` that takes every server for ours, as it does the real ones.
fn logged_in_on(server: &str) -> State {
    let mut cfg = harness::config();
    cfg.online_server = server.into();
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    let mut s = state_with(cfg, Vec::new());
    s.trusted_server = |_| true;
    s
}

/// The server moved: its account reply says where it is now. The game checks that address
/// answers for the same account, then makes it its server: settings, the lobby list and the
/// social connection go there from then on.
#[test]
fn the_game_follows_its_server_to_the_address_it_moved_to() {
    let (new, new_checks) = account_server("me", None);
    let (old, _) = account_server("me", Some(new.clone()));
    let mut s = logged_in_on(&old);
    let mut now = Instant::now();
    assert!(
        wait_for(&mut s, &mut now, |s| s.cfg.online_server == new),
        "the game never moved to the new address"
    );
    assert!(
        new_checks.load(Ordering::SeqCst) >= 1,
        "the new address was checked first"
    );
    assert!(s.server_move.is_idle());
    assert_eq!(s.cfg.online_user, "me", "still logged in");
}

/// A new address that doesn't answer, or answers for someone else, isn't followed: the game
/// keeps its server (and asks again at the next login).
#[test]
fn the_game_stays_when_the_new_address_doesnt_answer_for_it() {
    // Nothing listens there.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("127.0.0.1:{}", listener.local_addr().unwrap().port())
    };
    let (stranger, stranger_checks) = account_server("someone-else", None);
    for (to, checks) in [(closed, None), (stranger, Some(stranger_checks))] {
        let (old, _) = account_server("me", Some(to));
        let mut s = logged_in_on(&old);
        let mut now = Instant::now();
        assert!(
            wait_for(&mut s, &mut now, |s| s.server_move.is_busy()),
            "the new address is checked"
        );
        assert!(
            wait_for(&mut s, &mut now, |s| s.server_move.is_idle()),
            "the check ends"
        );
        if let Some(checks) = checks {
            assert!(checks.load(Ordering::SeqCst) >= 1);
        }
        assert_eq!(s.cfg.online_server, old, "the game kept its server");
    }
}

/// Only our own servers are listened to: one that isn't (a LAN server, or anything reached
/// without our certificate) can't send the game, and its login, somewhere else.
#[test]
fn a_server_that_isnt_ours_cant_move_the_game() {
    let (new, new_checks) = account_server("me", None);
    let (old, old_checks) = account_server("me", Some(new));
    let mut s = logged_in_on(&old);
    s.trusted_server = online::is_official;
    let mut now = Instant::now();
    assert!(
        wait_for(&mut s, &mut now, |s| s.mp_menu.account_request.is_none()
            && old_checks.load(Ordering::SeqCst) >= 1),
        "the account check came back"
    );
    for _ in 0..20 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    assert!(s.server_move.is_idle(), "nothing was checked");
    assert_eq!(new_checks.load(Ordering::SeqCst), 0);
    assert_eq!(s.cfg.online_server, old);
}
