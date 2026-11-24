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
        host_mods: true,
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
                operation_key: None,
                tja: None,
            }),
        },
        // A TJA play, with its course's chart as the board knows it.
        Request::SubmitScore {
            token: "t".into(),
            play: Box::new(online::ScoreSubmit {
                map: map_info(),
                score: 1_000,
                accuracy: 1.0,
                max_combo: 1,
                great: 1,
                ok: 0,
                miss: 0,
                pp: None,
                date: 1_700_000_000,
                replay_version: crate::scores::new_version(),
                inputs: vec![(0.0, 1)],
                stars: None,
                play_ms: Some(2_000),
                scroll_speed: Some(1.0),
                md5: None,
                pp_version: online::PP_VERSION,
                timing: None,
                rolls: Some(0),
                operation_key: Some("k".into()),
                tja: Some(taiko_logic::tja::BoardChart {
                    course: 3,
                    notes: vec![(0.0, 0.0, 0, false, 0.49, 0.0, 0)],
                    barlines: vec![(0.0, 0.49)],
                    points: vec![(0.0, 0.0, 500.0, 1.0, true)],
                    first_velocity: 0.49,
                    branches: None,
                    specials: None,
                }),
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
            pp: false,
        },
        Request::SetStatuses {
            token: "t".into(),
            maps: vec![(1, 2)],
            pp: true,
        },
        Request::LovedSets,
        Request::LovedMaps,
        Request::SetLoved {
            token: "t".into(),
            set: 1,
            loved: true,
            version: Some("Oni".into()),
        },
        Request::EpicMaps,
        Request::SetEpic {
            token: "t".into(),
            map: online::EpicMap::Set {
                set: 1,
                version: Some("Oni".into()),
            },
            epic: true,
        },
        Request::SetEpic {
            token: "t".into(),
            map: online::EpicMap::Set {
                set: 1,
                version: None,
            },
            epic: false,
        },
        Request::SetEpic {
            token: "t".into(),
            map: online::EpicMap::Custom(7),
            epic: true,
        },
        Request::Played {
            token: "t".into(),
            ms: 1000,
            map: None,
            attempt: None,
        },
        Request::Played {
            token: "t".into(),
            ms: 24_000,
            map: Some(online::PlayedMap {
                hash: 0x1f3a,
                name: "Artist - Song [Oni]".into(),
                set_id: Some(1),
            }),
            attempt: Some(online::PlayedAttempt {
                ended: online::AttemptEnd::Quit,
                at_ms: 36_000,
                length_ms: 90_000,
                from_ms: Some(12_000),
                great: 80,
                ok: 10,
                miss: 10,
                max_combo: 50,
                mods: vec!["HR".into()],
                rate: 1.5,
                scroll_speed: 1.2,
            }),
        },
        Request::MapVote {
            token: "t".into(),
            map: online::PlayedMap {
                hash: 0x1f3a,
                name: "Artist - Song [Oni]".into(),
                set_id: Some(1),
            },
            vote: Some(online::MapVote::Dislike),
        },
        Request::MapVote {
            token: "t".into(),
            map: online::PlayedMap {
                hash: 0x1f3a,
                name: "Artist - Song [Oni]".into(),
                set_id: None,
            },
            vote: None,
        },
        Request::Medals { name: "me".into() },
        Request::ClientEvent {
            token: "t".into(),
            event: "ura_logo".into(),
            key: "k".into(),
        },
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
        Down::Moving,
    ]
}

fn replies() -> Vec<Reply> {
    vec![
        Reply::CodeSent("a@b.c".into()),
        Reply::EmailLinked("a@b.c".into()),
        Reply::LovedSets(vec![1, 2]),
        Reply::LovedMaps(vec![(1, vec!["Oni".into()])]),
        Reply::EpicMaps(online::EpicList {
            sets: vec![(1, vec![]), (2, vec!["Oni".into()])],
            custom: vec![7],
        }),
        Reply::EpicMaps(online::EpicList::default()),
        Reply::SetStatuses(vec![(1, "ranked".into())]),
        Reply::SetVerdicts(vec![online::SetVerdict {
            set: 1,
            status: "ranked".into(),
            pp: "yes".into(),
        }]),
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

#[test]
fn the_encrypted_connection_does_not_name_the_server() {
    // The first thing the game sends is the handshake's hello, in the clear. A VPN that reads a
    // name there sends the connection to that name instead of the address dialed, which was the
    // old forwarder: the game only saw "the connection dropped". So no name goes out.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let raw = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let client = std::thread::spawn(move || taiko_proto::tls::connect(raw).is_ok());
    let (mut server, _) = listener.accept().unwrap();
    server
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let mut header = [0u8; 5];
    std::io::Read::read_exact(&mut server, &mut header).unwrap();
    assert_eq!(header[0], 0x16, "a handshake record comes first");
    let mut hello = vec![0u8; u16::from_be_bytes([header[3], header[4]]) as usize];
    std::io::Read::read_exact(&mut server, &mut hello).unwrap();
    assert_eq!(hello[0], 0x01, "and it's the client's hello");
    let name = taiko_proto::tls::SERVER_HOST.as_bytes();
    assert!(
        !hello.windows(name.len()).any(|w| w == name),
        "the hello names {}",
        taiko_proto::tls::SERVER_HOST
    );
    drop(server);
    assert!(!client.join().unwrap(), "no handshake without a server");
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
        host_mods: false,
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

/// A chat line longer than the lobby's chat box wraps onto the lines under it: every word of it
/// is on screen, none of it cut off with "...".
#[test]
fn a_long_lobby_chat_line_shows_whole() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("mp-chat-wrap", maps::basics(), &room);
    room.send(&room_msg(Some(lobby_map(&s)), false));
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.map.is_some())
    });
    let line = "YO 2nd time this has happened to me in a row, the map just skipped to the end";
    room.send(&ServerMsg::Chat {
        from: "RubyDear".into(),
        text: line.into(),
    });
    wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| !m.chat.is_empty())
    });
    let c = frame(&mut s, now);
    let rows: Vec<&str> = c
        .texts
        .iter()
        .map(|(_, t)| t.as_str())
        .skip_while(|t| !t.starts_with(": YO"))
        .collect();
    assert!(!rows.is_empty(), "the chat line is drawn");
    let mut words = line.split_whitespace();
    for row in &rows {
        for w in row.trim_start_matches(": ").split_whitespace() {
            if words.clone().next() == Some(w) {
                words.next();
            }
        }
    }
    assert_eq!(words.next(), None, "every word shows, in order: {rows:?}");
    assert!(
        rows.iter().all(|r| !r.contains("...")),
        "nothing cut: {rows:?}"
    );
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
    // A guest's copy goes into taiko.rs's own folder, not osu!'s.
    let custom = songs.dir.with_file_name(format!(
        "{}-custom",
        songs.dir.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&custom);
    s.set_custom_dir(custom.clone());
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
    let folder = custom.join("1 Artist - Song");
    assert_eq!(std::fs::read(folder.join("audio.mp3")).unwrap(), music);
    assert!(!songs.dir.join("1 Artist - Song").exists());
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
    let _ = std::fs::remove_dir_all(&custom);
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
// The server being updated under the social link.

/// A social server that lists `moving/1`. Once the game declared it on the first connection,
/// the server is updated: it closes that connection, after `Down::Moving` when `says_moving`.
/// Returns its address and how many social connections it took.
fn updated_social_server(says_moving: bool) -> (String, Arc<AtomicUsize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let connects = Arc::new(AtomicUsize::new(0));
    let counted = connects.clone();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let Ok(line) = online::read_line(&mut reader) else {
                continue;
            };
            if !matches!(
                serde_json::from_str::<Envelope>(&line).map(|env| env.request),
                Ok(Request::Connect { .. })
            ) {
                held.push(out);
                continue;
            }
            let _ = online::write_line(
                &mut out,
                &Reply::Account {
                    name: "me".into(),
                    token: "t".into(),
                    nominator: false,
                    admin: false,
                    email: Some(true),
                    features: vec![online::MOVING_FEATURE.into()],
                    terms: 0,
                    server: None,
                },
            );
            if counted.fetch_add(1, Ordering::SeqCst) > 0 {
                held.push(out);
                continue;
            }
            // The first connection: wait for the game's features, then move it.
            while let Ok(line) = online::read_line(&mut reader) {
                if let Ok(Up::Features(features)) = serde_json::from_str::<Up>(&line) {
                    assert!(
                        features.iter().any(|f| f == online::MOVING_FEATURE),
                        "the game declares moving/1: {features:?}"
                    );
                    break;
                }
            }
            if says_moving {
                let _ = online::write_line(&mut out, &Down::Moving);
            }
            let _ = out.shutdown(std::net::Shutdown::Both);
        }
    });
    (address, connects)
}

/// Runs the game against an updated social server until its link is back on a second
/// connection. Returns whether the red "offline" note showed on the way, and how long the
/// move took.
fn moved_social_link(says_moving: bool) -> (bool, Duration) {
    let (server, connects) = updated_social_server(says_moving);
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_user = "me".into();
    cfg.online_token = "t".into();
    let mut s = state_with(cfg, Vec::new());
    let mut now = Instant::now();
    assert!(
        wait_for(&mut s, &mut now, |s| s.social.connected),
        "the social link connects"
    );
    let started = Instant::now();
    let mut offline = false;
    let back = wait_for(&mut s, &mut now, |s| {
        offline |= s.social.notes.iter().any(|note| note.title == "offline");
        connects.load(Ordering::SeqCst) >= 2 && s.social.connected
    });
    assert!(back, "the social link reconnects");
    (offline, started.elapsed())
}

#[test]
fn a_server_update_moves_the_social_link_without_calling_it_lost() {
    let (offline, took) = moved_social_link(true);
    assert!(!offline, "no offline note for a move the server announced");
    assert!(
        took < Duration::from_secs(2),
        "reconnects at once, took {took:?}"
    );
    // The same close without the announcement is a lost connection, as before.
    let (offline, _) = moved_social_link(false);
    assert!(offline, "an unannounced close still shows the offline note");
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

/// The lobby map's preview (P) opens over the lobby, stays up and goes back to the lobby.
fn lobby_preview_stays(tag: &str, defense: Option<net::Defense>) {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest(tag, maps::basics(), &room);
    let mut map = lobby_map(&s);
    map.defense = defense;
    room.send(&room_msg(Some(map), false));
    assert!(wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.map_entry.is_some())
    }));
    press(&mut s, KeyCode::KeyP, now);
    assert_eq!(scene_name(&s), "Preview");
    for _ in 0..120 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(scene_name(&s), "Preview", "the preview stays up");
    // Past the fade between screens.
    s.scene_since = now - Duration::from_secs(5);
    press(&mut s, KeyCode::Escape, now);
    assert_eq!(scene_name(&s), "Lobby", "Esc goes back to the lobby");
    assert!(s.mp.is_some());
}

#[test]
fn the_mapguessr_host_types_the_rounds_and_right_clicks_them_down() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("guessr-rounds", maps::basics(), &room);
    room.send(&ServerMsg::Room {
        players: vec![player(2, "me", true), player(1, "guest", false)],
        map: None,
        in_match: false,
        mapguessr: true,
        common: None,
        team: None,
        locked: false,
        slots: 16,
    });
    assert!(wait_for(&mut s, &mut now, |s| {
        s.guessr.is_some() && s.mp.as_ref().is_some_and(|m| m.mapguessr && m.is_host())
    }));
    let rounds = |s: &State| s.guessr.as_ref().map(|g| g.rounds);
    assert_eq!(rounds(&s), Some(mapguessr::DEFAULT_ROUNDS));

    press(&mut s, KeyCode::Digit2, now);
    assert_eq!(rounds(&s), Some(2), "the first digit starts a new number");
    press(&mut s, KeyCode::Numpad5, now);
    assert_eq!(rounds(&s), Some(25), "the next goes on its end");
    press(&mut s, KeyCode::Digit7, now);
    assert_eq!(rounds(&s), Some(7), "past the most, a digit starts again");
    press(&mut s, KeyCode::Digit0, now);
    assert_eq!(
        rounds(&s),
        Some(7),
        "70 is past the most and 0 rounds is none: it stays 7"
    );
    press(&mut s, KeyCode::Backspace, now);
    assert_eq!(
        rounds(&s),
        Some(1),
        "Backspace takes the last digit, down to 1"
    );
    press(&mut s, KeyCode::Digit0, now);
    assert_eq!(rounds(&s), Some(10), "1 then 0 is 10");
    press(&mut s, KeyCode::BracketRight, now);
    press(&mut s, KeyCode::Digit3, now);
    assert_eq!(
        rounds(&s),
        Some(3),
        "after another key, a digit starts a new number"
    );

    // The rounds button: a left click counts up, a right click down.
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let button = s
        .hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::Key(KeyCode::BracketRight))
        .map(|h| h.rect)
        .expect("the host has a rounds button");
    let k = s.window_h / crate::render::VIRTUAL_HEIGHT;
    s.mouse = (
        f64::from((button[0] + button[2] / 2.0) * k),
        f64::from((button[1] + button[3] / 2.0) * k),
    );
    s.right_button(true);
    s.right_button(false);
    assert_eq!(rounds(&s), Some(2), "a right click counts down");
    s.mouse_button(true, None);
    s.mouse_button(false, None);
    assert_eq!(rounds(&s), Some(3), "a left click counts up");
}

#[test]
fn a_lobby_map_previews_and_goes_back_to_the_lobby() {
    lobby_preview_stays("mp-preview", None);
}

#[test]
fn a_defense_lobby_map_previews_and_goes_back_to_the_lobby() {
    lobby_preview_stays("mp-preview-defense", Some(net::Defense::DEFAULT));
}

#[test]
fn the_lobby_map_picker_opens_mods_on_f1_and_picks_from_preview_play() {
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("lobby-picker-f1-play", maps::basics(), &room);
    room.send(&ServerMsg::Room {
        players: vec![player(2, "me", true), player(1, "guest", false)],
        map: None,
        in_match: false,
        mapguessr: false,
        common: None,
        team: None,
        locked: false,
        slots: 16,
    });
    assert!(wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref().is_some_and(|m| m.is_host())
    }));

    // M opens the lobby map picker; F1 opens Mods (and closes it) without leaving the picker.
    press(&mut s, KeyCode::KeyM, now);
    assert!(s.picking_lobby_map, "M opens the map picker");
    assert_eq!(scene_name(&s), "Select");

    press(&mut s, KeyCode::F1, now);
    assert!(
        s.mod_select.is_some(),
        "F1 opens mods while picking a lobby map"
    );
    assert!(s.picking_lobby_map, "opening mods stays in the map picker");
    assert_eq!(scene_name(&s), "Select");

    press(&mut s, KeyCode::F1, now);
    assert!(s.mod_select.is_none(), "F1 again closes mods");
    assert!(s.picking_lobby_map, "closing mods stays in the map picker");

    // Once the preview has loaded and a start marker is set on its timeline, clicking the
    // preview's Play button picks the map for the lobby instead of starting it solo.
    let selected = s.select.selected().expect("a map is selected");
    s.preview.prime_with_marker(&s.library, selected, 5000.0);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let (vx, vy) = s
        .preview
        .play_button_virtual()
        .expect("preview play button is shown after seeking");
    let k = s.window_h / crate::render::VIRTUAL_HEIGHT;
    s.mouse = (f64::from(vx * k), f64::from(vy * k));
    s.mouse_button(true, None);
    s.mouse_button(false, None);

    assert_eq!(
        scene_name(&s),
        "Lobby",
        "clicking Play in the preview picks the map for the lobby"
    );
    assert!(!s.picking_lobby_map);
    assert!(
        room.wait_for_msg(|m| matches!(m, ClientMsg::SelectMap(_))),
        "the picked map is sent to the lobby"
    );
}

// Downloads from the mirrors: a mirror can keep serving a set as it was before osu! updated it
// (osu.direct gave Marika - Soul Release with three empty placeholder difficulties and old Oni
// files after the set was ranked), and its description of the set can be years old (catboy.best
// still described that set as it was in 2019).

/// The .osu text of a difficulty, different for each `version` and `notes`.
fn mirror_osu(version: &str, notes: usize) -> Vec<u8> {
    let mut text =
        format!("osu file format v14\n\n[Metadata]\nVersion:{version}\n\n[HitObjects]\n");
    for i in 0..notes {
        let _ = writeln!(text, "256,192,{},1,0", 1000 + i * 250);
    }
    text.into_bytes()
}

/// An `.osz` holding `files` (name, contents).
fn mirror_osz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A mirror's description of a set: when osu! last changed it, and each difficulty (beatmap
/// ID, [Version], the .osu it has for it).
fn mirror_json(last_updated: &str, maps: &[(u32, &str, &[u8])]) -> Vec<u8> {
    let beatmaps: Vec<serde_json::Value> = maps
        .iter()
        .map(|(id, version, osu)| {
            serde_json::json!({
                "id": id,
                "version": version,
                "checksum": crate::download::md5_hex(osu),
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "last_updated": last_updated,
        "beatmaps": beatmaps,
    }))
    .unwrap()
}

/// Mirrors on this machine: each path in `routes` answers with its body (JSON when it starts
/// with `{`, else an archive), anything else with 404; every path asked for is passed on.
fn mirror_server(
    routes: Vec<(&'static str, Vec<u8>)>,
) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            // The rest of the request's head.
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                    break;
                }
            }
            let _ = tx.send(path.clone());
            let reply = match routes.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    let kind = if body.first() == Some(&b'{') {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(body);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = out.write_all(&reply);
        }
    });
    (base, rx)
}

/// The files in a set's folder: (name, contents), by name.
type SetFiles = Vec<(String, Vec<u8>)>;

/// What a download from mirrors `a` and `b` (in that order; each a description and an archive)
/// unpacks: each file in the set's folder, the paths the mirrors were asked for in order, and
/// whether any archive was left lying in the Songs folder.
fn download_from_mirrors(
    name: &str,
    a: (Vec<u8>, Vec<u8>),
    b: (Vec<u8>, Vec<u8>),
) -> (SetFiles, Vec<String>, bool) {
    let (base, asked) = mirror_server(vec![
        ("/a/s/1", a.0),
        ("/a/d/1", a.1),
        ("/b/s/1", b.0),
        ("/b/d/1", b.1),
    ]);
    let songs = std::env::temp_dir().join(format!(
        "taiko-rs-regression-mirrors-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&songs);
    let sources = [
        ("a", format!("{base}/a/d/1")),
        ("b", format!("{base}/b/d/1")),
    ];
    let mut check = crate::download::Check::new(
        vec![
            ("a", format!("{base}/a/s/1")),
            ("b", format!("{base}/b/s/1")),
        ],
        None,
    );
    let (done, total) = (
        std::sync::atomic::AtomicU64::new(0),
        std::sync::atomic::AtomicU64::new(0),
    );
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let folder = crate::download::fetch(
        &sources,
        "1 Marika - Soul Release",
        &songs,
        &done,
        &total,
        &cancel,
        Some(&mut check),
    )
    .unwrap();
    let mut maps: Vec<(String, Vec<u8>)> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    maps.sort();
    let left = std::fs::read_dir(&songs)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains(".osz"));
    let _ = std::fs::remove_dir_all(&songs);
    (maps, asked.try_iter().collect(), left)
}

#[test]
fn a_mirror_serving_an_old_version_of_a_set_is_passed_over_for_one_with_osus() {
    let (oni, kantan) = (mirror_osu("Oni", 40), mirror_osu("Kantan", 12));
    let (old_oni, empty_kantan) = (mirror_osu("Oni", 39), mirror_osu("Kantan", 0));
    let current = [(1, "Oni", oni.as_slice()), (2, "Kantan", kantan.as_slice())];
    let (maps, asked, left) = download_from_mirrors(
        "old-archive",
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &old_oni), ("kantan.osu", &empty_kantan)]),
        ),
        (
            mirror_json("2019-03-11T04:01:36Z", &[(1, "Oni", old_oni.as_slice())]),
            mirror_osz(&[("oni.osu", &oni), ("kantan.osu", &kantan)]),
        ),
    );
    assert_eq!(
        maps,
        vec![
            ("kantan.osu".to_string(), kantan),
            ("oni.osu".to_string(), oni)
        ],
        "the second mirror's current archive is the one unpacked"
    );
    // The first mirror's archive, its description, then every description (to be sure it's the
    // archive that's old), then the second archive.
    assert_eq!(asked, ["/a/d/1", "/a/s/1", "/a/s/1", "/b/s/1", "/b/d/1"]);
    assert!(!left, "no archive is left in the Songs folder");
}

#[test]
fn a_current_archive_from_the_first_mirror_is_taken_without_asking_the_second() {
    let oni = mirror_osu("Oni", 40);
    let (maps, asked, left) = download_from_mirrors(
        "current",
        (
            mirror_json("2026-11-04T00:38:02Z", &[(1, "Oni", oni.as_slice())]),
            mirror_osz(&[("oni.osu", &oni)]),
        ),
        (b"{}".to_vec(), Vec::new()),
    );
    assert_eq!(maps, vec![("oni.osu".to_string(), oni)]);
    assert_eq!(asked, ["/a/d/1", "/a/s/1"]);
    assert!(!left);
}

#[test]
fn an_old_description_doesnt_make_a_current_archive_look_old() {
    let (oni, old_oni) = (mirror_osu("Oni", 40), mirror_osu("Oni", 39));
    let (maps, asked, _) = download_from_mirrors(
        "old-description",
        (
            mirror_json("2019-03-11T04:01:36Z", &[(1, "Oni", old_oni.as_slice())]),
            mirror_osz(&[("oni.osu", &oni)]),
        ),
        (
            mirror_json("2026-11-04T00:38:02Z", &[(1, "Oni", oni.as_slice())]),
            mirror_osz(&[("oni.osu", &old_oni)]),
        ),
    );
    assert_eq!(maps, vec![("oni.osu".to_string(), oni)]);
    // The second mirror's archive is never downloaded.
    assert_eq!(asked, ["/a/d/1", "/a/s/1", "/a/s/1", "/b/s/1"]);
}

#[test]
fn when_no_mirror_has_the_current_version_the_closest_archive_is_unpacked() {
    let (oni, kantan) = (mirror_osu("Oni", 40), mirror_osu("Kantan", 12));
    let (old_oni, empty_kantan) = (mirror_osu("Oni", 39), mirror_osu("Kantan", 0));
    let current = [(1, "Oni", oni.as_slice()), (2, "Kantan", kantan.as_slice())];
    let (maps, asked, left) = download_from_mirrors(
        "none-current",
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &old_oni), ("kantan.osu", &empty_kantan)]),
        ),
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &oni), ("kantan.osu", &empty_kantan)]),
        ),
    );
    assert_eq!(
        maps,
        vec![
            ("kantan.osu".to_string(), empty_kantan),
            ("oni.osu".to_string(), oni)
        ],
        "the archive with one of osu!'s two files beats the one with none"
    );
    // Each mirror's archive is downloaded once: no loop over old archives.
    assert_eq!(
        asked.iter().filter(|p| p.contains("/d/")).count(),
        2,
        "{asked:?}"
    );
    assert!(!left);
}

#[test]
fn the_newest_description_of_a_set_is_the_one_believed() {
    let (oni, old_oni) = (mirror_osu("Oni", 40), mirror_osu("Oni", 39));
    let (base, _) = mirror_server(vec![
        (
            "/a/s/1",
            mirror_json("2019-03-11T04:01:36Z", &[(1, "Oni", old_oni.as_slice())]),
        ),
        (
            "/b/s/1",
            mirror_json("2026-11-04T00:38:02Z", &[(1, "Oni", oni.as_slice())]),
        ),
    ]);
    let sources = [
        ("a", format!("{base}/a/s/1")),
        ("b", format!("{base}/b/s/1")),
    ];
    let md5 = |set: &crate::download::OnlineSet| set[&1].md5.clone();
    assert_eq!(
        md5(&crate::download::checksums_from(&sources, false).unwrap()),
        crate::download::md5_hex(&old_oni),
        "the first mirror's, when only the first is asked"
    );
    assert_eq!(
        md5(&crate::download::checksums_from(&sources, true).unwrap()),
        crate::download::md5_hex(&oni),
        "the newest, when all are"
    );
}

/// The host takes the mods for everyone (F in the lobby): a guest's mod select shows the
/// host's, the guest can't switch them, the room hears none of the guest's own and the match
/// plays the host's. When the host gives them back, the ones the host had become the guest's
/// own instead of vanishing. OD Override is a mod of a player's own like the rest (a play with
/// it just isn't saved).
#[test]
fn the_host_picks_the_mods_for_everyone_and_gives_them_back() {
    use crate::mods::{Mod, Mods};
    let room = FakeRoom::new();
    let (mut s, _songs, mut now) = guest("mp-host-mods", maps::basics(), &room);
    let mut log = String::new();
    s.cfg.mods = Mods::parse("HD");
    let mut map = lobby_map(&s);
    map.host_mods = true;
    map.mods = vec!["DT".into(), "HR".into(), "FL".into()];
    room.send(&room_msg(Some(map.clone()), false));
    assert!(wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref()
            .and_then(|m| m.map.as_ref())
            .is_some_and(|m| m.host_mods)
    }));
    let _ = writeln!(log, "host's mods: shown {}", s.shown_mods().label());
    s.toggle_mod(Mod::Easy);
    let _ = writeln!(
        log,
        "guest switches EZ: own {} ({})",
        s.cfg.mods.label(),
        s.toast.as_ref().map_or("", |t| t.text.as_str())
    );
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(
        !room
            .received()
            .iter()
            .any(|m| kind(m) == "Mods" && m.contains("HD")),
        "the guest's own HD isn't told to the room"
    );
    let _ = writeln!(log, "the room hears none of the guest's own");
    // The match plays the host's.
    room.send(&ServerMsg::Prepare(map.clone()));
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::Loaded(true)
    )));
    room.send(&ServerMsg::Start {
        at: room.clock() + 200.0,
    });
    assert!(wait_for(&mut s, &mut now, |s| harness::game(s).is_some()));
    let _ = writeln!(
        log,
        "match plays: {}",
        harness::game(&s).unwrap().mods().label()
    );
    // Back in the lobby, the host gives the mods back: the host's others are now ours.
    s.scene = crate::screens::lobby::Lobby::scene();
    s.play = None;
    map.host_mods = false;
    map.mods = vec!["DT".into()];
    room.send(&room_msg(Some(map.clone()), false));
    assert!(wait_for(&mut s, &mut now, |s| {
        s.mp.as_ref()
            .and_then(|m| m.map.as_ref())
            .is_some_and(|m| !m.host_mods)
    }));
    let _ = writeln!(
        log,
        "free again: own {}, shown {}",
        s.cfg.mods.label(),
        s.shown_mods().label()
    );
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::Mods(list) if list == &["HR", "FL"]
    )));
    let _ = writeln!(log, "the room hears them as the guest's own");
    // OD Override goes with them, and to the match.
    s.toggle_mod(Mod::OdOverride);
    assert!(pump(&mut s, &mut now, &room, |m| matches!(
        m,
        ClientMsg::Mods(list) if list.iter().any(|a| a == "OV")
    )));
    let _ = writeln!(
        log,
        "with OV: shown {}, unranked {}",
        s.shown_mods().label(),
        s.shown_mods().unranked()
    );
    assert_snapshot("network-host-mods", &log);
}

/// Keeps what a slow link takes in ahead of it small (connections accepted inherit it): the
/// system would otherwise take megabytes in at once (macOS does on loopback) and stand for a
/// fast link, the backlog left in the sender's socket.
#[cfg(unix)]
fn small_receive_buffer(listener: &std::net::TcpListener) {
    use std::os::fd::AsRawFd;
    let value: libc::c_int = 64 * 1024;
    // SAFETY: a valid socket, and a c_int option given with its size.
    unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

#[cfg(not(unix))]
fn small_receive_buffer(_listener: &std::net::TcpListener) {}

/// How long a new map may take to come past a set being sent. On macOS the test's slow link
/// doesn't hold the set back the way it does elsewhere (CI took about 3-5 s there, with the
/// client's own limits in place, and it's not known yet where on the way it waits), so there
/// only its coming at all is checked.
fn quick(limit: Duration) -> Duration {
    if cfg!(target_os = "macos") {
        Duration::from_secs(9)
    } else {
        limit
    }
}

/// A way to `to` that carries what the client sends at about `bytes_per_s` (what comes back is
/// passed straight on): a player on a slow upload. Takes one connection.
fn slow_upload(to: String, bytes_per_s: usize) -> String {
    use std::io::Read as _;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    small_receive_buffer(&listener);
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        let Ok((mut client, _)) = listener.accept() else {
            return;
        };
        let Ok(mut room) = std::net::TcpStream::connect(&to) else {
            return;
        };
        let (Ok(mut back_from), Ok(mut back_to)) = (room.try_clone(), client.try_clone()) else {
            return;
        };
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut back_from, &mut back_to);
        });
        let mut buf = [0u8; 4096];
        while let Ok(n) = client.read(&mut buf) {
            if n == 0 || room.write_all(&buf[..n]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs_f64(n as f64 / bytes_per_s as f64));
        }
    });
    addr
}

/// Draws a frame of each client at `now` until `done` holds (true) or `limit` of real time
/// passes (false).
fn both_until(
    a: &mut State,
    b: &mut State,
    now: &mut Instant,
    limit: Duration,
    mut done: impl FnMut(&State, &State) -> bool,
) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if done(a, b) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
        *now += Duration::from_millis(16);
        frame(a, *now);
        frame(b, *now);
    }
    done(a, b)
}

/// A room with a host on a slow upload (1 MB/s: a 4 MB set takes about 5 s to send) and a
/// guest, both in its lobby. The host has two maps, the first with a big song file; the guest
/// has `guest_has` of them. Hands back (host, guest, both libraries, the frames' clock).
fn slow_host_lobby(
    tag: &str,
    guest_has: &[maps::Chart],
) -> (State, State, (Songs, Songs), Instant) {
    let host_songs = Songs::new(&format!("{tag}-host"), &[maps::basics(), maps::big_notes()]);
    let big_set = host_songs.paths[0].parent().unwrap().to_path_buf();
    std::fs::write(big_set.join("audio.mp3"), vec![7u8; 4 << 20]).unwrap();
    let guest_songs = Songs::new(&format!("{tag}-guest"), guest_has);
    let mut h = state_with(harness::config(), host_songs.entries());
    let mut g = state_with(harness::config(), guest_songs.entries());
    h.song_dirs = vec![host_songs.dir.clone()];
    h.songs_dir = host_songs.dir.clone();
    g.song_dirs = vec![guest_songs.dir.clone()];
    g.songs_dir = guest_songs.dir.clone();
    let mut now = Instant::now();
    ready(&mut h, &mut now);
    ready(&mut g, &mut now);
    let server = net::host(0, false).unwrap();
    let room = format!("127.0.0.1:{}", server.port);
    let link = slow_upload(room.clone(), 1 << 20);
    let mut host = multiplayer::joined(
        net::Client::connect(&link, "host", Some(server.token)).unwrap(),
        "regression room",
    );
    host.server = Some(server);
    h.mp = Some(host);
    h.scene = crate::screens::lobby::Lobby::scene();
    g.mp = Some(multiplayer::joined(
        net::Client::connect(&room, "guest", None).unwrap(),
        "regression room",
    ));
    g.scene = crate::screens::lobby::Lobby::scene();
    let in_room = |s: &State| s.mp.as_ref().is_some_and(|m| m.players.len() == 2);
    assert!(both_until(
        &mut h,
        &mut g,
        &mut now,
        Duration::from_secs(10),
        |h, g| { in_room(h) && in_room(g) && h.mp.as_ref().unwrap().is_host() }
    ));
    (h, g, (host_songs, guest_songs), now)
}

/// The lobby map's title as a client has it.
fn lobby_title(s: &State) -> Option<String> {
    s.mp.as_ref()
        .and_then(|m| m.map.as_ref())
        .map(|m| m.title.clone())
}

/// The host picks the `i`th song of its list for the lobby (M, then Enter).
fn pick_for_lobby(h: &mut State, i: usize, now: Instant) {
    press(h, KeyCode::KeyM, now);
    assert!(h.picking_lobby_map, "M opens the map picker");
    h.select.set_cursor(i);
    frame(h, now);
    press(h, KeyCode::Enter, now);
    assert_eq!(scene_name(h), "Lobby", "a pick goes back to the lobby");
}

/// The host picks the big map, which the guest doesn't have: the guest asks the host for its
/// set, and its first part has arrived.
fn guest_receiving_the_big_set(h: &mut State, g: &mut State, now: &mut Instant) {
    pick_for_lobby(h, 0, *now);
    assert!(
        both_until(h, g, now, Duration::from_secs(10), |_, g| {
            lobby_title(g).as_deref() == Some("basics") && g.host_transfer.is_some()
        }),
        "the guest gets the big set from the host"
    );
}

/// The host picks a map the guest doesn't have, so the guest asks the host for its set and the
/// host starts sending it on a slow upload. The host changes its mind: the new map reaches the
/// room at once, not after the whole set has gone up, and the guest, who has the new map, isn't
/// left waiting on the old one's transfer.
#[test]
fn the_host_changes_the_map_while_sending_a_set() {
    let (mut h, mut g, _songs, mut now) = slow_host_lobby("mp-change", &[maps::big_notes()]);
    let mut log = String::new();
    guest_receiving_the_big_set(&mut h, &mut g, &mut now);
    let _ = writeln!(
        log,
        "guest gets the first map from the host: {}",
        g.host_transfer.is_some()
    );
    // The host picks the map the guest has, while the set is still going up.
    pick_for_lobby(&mut h, 1, now);
    let picked = Instant::now();
    let arrived = both_until(&mut h, &mut g, &mut now, Duration::from_secs(10), |h, g| {
        lobby_title(h).as_deref() == Some("big-notes")
            && lobby_title(g).as_deref() == Some("big-notes")
    });
    let waited = picked.elapsed();
    assert!(arrived, "the new map reaches the room");
    assert!(
        waited < quick(Duration::from_millis(1500)),
        "the new map waited {waited:?} behind the set being sent"
    );
    let _ = writeln!(log, "the room has the new map within 1.5 s: true");
    // The guest has it: no transfer left hanging on the old one, ready to play.
    assert!(both_until(
        &mut h,
        &mut g,
        &mut now,
        Duration::from_secs(5),
        |_, g| {
            g.mp.as_ref()
                .is_some_and(|m| m.have_map && m.me().is_some_and(|p| p.has_map))
        }
    ));
    let _ = writeln!(
        log,
        "guest has the new map: true, still receiving the old one: {}",
        g.host_transfer.is_some()
    );
    press(&mut g, KeyCode::Space, now);
    assert!(
        both_until(&mut h, &mut g, &mut now, Duration::from_secs(2), |h, _| {
            h.mp.as_ref()
                .is_some_and(|m| m.players.iter().any(|p| !p.host && p.ready))
        }),
        "the guest's Ready reaches the room at once too"
    );
    let _ = writeln!(log, "guest ready: true");
    assert_snapshot("network-host-changes-map-mid-transfer", &log);
}

/// As above, but the guest doesn't have the new map either: the host stops sending the old set,
/// and the guest asks for the new one at once and gets it, instead of after the old set has
/// gone up (or never, waiting on it).
#[test]
fn a_new_lobby_map_comes_from_the_host_without_waiting_for_the_old_set() {
    let (mut h, mut g, _songs, mut now) = slow_host_lobby("mp-change-both", &[]);
    let mut log = String::new();
    guest_receiving_the_big_set(&mut h, &mut g, &mut now);
    pick_for_lobby(&mut h, 1, now);
    let picked = Instant::now();
    let have_new = |g: &State| {
        lobby_title(g).as_deref() == Some("big-notes") && g.mp.as_ref().is_some_and(|m| m.have_map)
    };
    let got = both_until(&mut h, &mut g, &mut now, Duration::from_secs(15), |_, g| {
        have_new(g)
    });
    let waited = picked.elapsed();
    assert!(got, "the guest gets the new map from the host");
    // The old set had about 4 s left to go up.
    assert!(
        waited < quick(Duration::from_millis(2500)),
        "the new map came {waited:?} after it was picked: behind the old set"
    );
    let titles: Vec<&str> = g.library.iter().map(|e| e.meta.title.as_str()).collect();
    let _ = writeln!(log, "guest got the new map in time: true");
    let _ = writeln!(log, "guest's library: {titles:?}");
    let _ = writeln!(
        log,
        "downloads left: {}, receiving: {}",
        g.downloads.len(),
        g.host_transfer.is_some()
    );
    assert_snapshot("network-new-lobby-map-skips-the-old-set", &log);
}

/// A TJA chart with two courses that plays `song.ogg`, as a host has it in its TJA folder.
const LOBBY_TJA: &str = "TITLE:Lobby TJA\nSUBTITLE:--Tester\nBPM:120\nWAVE:song.ogg\nOFFSET:0\n\nCOURSE:Oni\nLEVEL:8\n#START\n1020,\n3040,\n#END\n\nCOURSE:Easy\nLEVEL:2\n#START\n1000,\n1000,\n#END\n";

/// A client's map folders (osu!'s Songs, taiko.rs's custom maps, its TJA charts) under `root`,
/// gone when the test is.
struct TjaPlayer {
    root: std::path::PathBuf,
}

impl TjaPlayer {
    fn new(tag: &str, who: &str) -> TjaPlayer {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-mp-tja-{tag}-{who}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["Songs", "Custom", "Tja"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        TjaPlayer { root }
    }

    fn songs(&self) -> std::path::PathBuf {
        self.root.join("Songs")
    }

    fn tja(&self) -> std::path::PathBuf {
        self.root.join("Tja")
    }

    /// Writes `bytes` as `Tja/<path>`.
    fn put(&self, path: &str, bytes: &[u8]) {
        let p = self.tja().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// A client looking through these folders.
    fn client(&self, now: &mut Instant) -> State {
        let mut s = state_with(harness::config(), Vec::new());
        s.songs_dir = self.songs();
        s.song_dirs = vec![self.songs()];
        s.set_custom_dir(self.root.join("Custom"));
        s.tja_dir = Some(self.tja());
        ready(&mut s, now);
        s.refresh_library(true);
        assert!(wait_for(&mut s, now, |s| s.scan.is_none()), "the scan ends");
        s
    }
}

impl Drop for TjaPlayer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The files under `dir`, by their paths from it, sorted.
fn files_under(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                let rel = e.path().strip_prefix(dir).unwrap().to_path_buf();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

/// A room of a host whose TJA folder holds `LOBBY_TJA` (beside a file it doesn't name) and a
/// guest whose holds `guest_has` (path, contents), with the host picking its Oni course;
/// `tja_files`: the host's server lets TJA charts through.
fn tja_lobby(
    tag: &str,
    guest_has: &[(&str, &[u8])],
    tja_files: bool,
) -> (State, State, TjaPlayer, TjaPlayer, Instant) {
    let (host_dirs, guest_dirs) = (TjaPlayer::new(tag, "host"), TjaPlayer::new(tag, "guest"));
    // Named like an osu! set's folder, which it isn't.
    host_dirs.put("1234 Lobby Song/song.tja", LOBBY_TJA.as_bytes());
    host_dirs.put("1234 Lobby Song/song.ogg", &[7u8; 64 << 10]);
    host_dirs.put("1234 Lobby Song/notes.txt", b"not the chart's");
    for (path, bytes) in guest_has {
        guest_dirs.put(path, bytes);
    }
    let mut now = Instant::now();
    let mut h = host_dirs.client(&mut now);
    let mut g = guest_dirs.client(&mut now);
    // The server lets TJA charts through (`online::TJA_LOBBY_FILES_FEATURE`).
    h.tja_lobby_files = tja_files;
    let server = net::host(0, false).unwrap();
    let room = format!("127.0.0.1:{}", server.port);
    let mut host = multiplayer::joined(
        net::Client::connect(&room, "host", Some(server.token)).unwrap(),
        "regression room",
    );
    host.server = Some(server);
    h.mp = Some(host);
    h.scene = crate::screens::lobby::Lobby::scene();
    g.mp = Some(multiplayer::joined(
        net::Client::connect(&room, "guest", None).unwrap(),
        "regression room",
    ));
    g.scene = crate::screens::lobby::Lobby::scene();
    let in_room = |s: &State| s.mp.as_ref().is_some_and(|m| m.players.len() == 2);
    assert!(both_until(
        &mut h,
        &mut g,
        &mut now,
        Duration::from_secs(10),
        |h, g| { in_room(h) && in_room(g) && h.mp.as_ref().unwrap().is_host() }
    ));
    press(&mut h, KeyCode::KeyM, now);
    assert!(h.picking_lobby_map, "M opens the map picker");
    let oni = h
        .select
        .rows()
        .iter()
        .position(|&i| h.library[i].is_tja() && h.library[i].meta.version == "Oni")
        .expect("the host lists the chart's Oni course");
    h.select.set_cursor(oni);
    frame(&mut h, now);
    press(&mut h, KeyCode::Enter, now);
    assert_eq!(scene_name(&h), "Lobby", "a pick goes back to the lobby");
    (h, g, host_dirs, guest_dirs, now)
}

/// Whether `s` has the lobby map and the room knows it.
fn has_lobby_map(s: &State) -> bool {
    s.mp.as_ref()
        .is_some_and(|m| m.have_map && m.me().is_some_and(|p| p.has_map))
}

/// The host picks a course of a TJA chart the guest doesn't have: the guest gets it from the
/// host like any lobby map, as the chart with the music and pictures it names, into its TJA
/// folder (the only place charts are read from), and can play it.
#[test]
fn a_tja_chart_comes_from_the_lobby_host_into_the_tja_folder() {
    let (mut h, mut g, _host_dirs, guest_dirs, mut now) = tja_lobby("get", &[], true);
    assert!(
        both_until(&mut h, &mut g, &mut now, Duration::from_secs(15), |_, g| {
            has_lobby_map(g)
        }),
        "the guest gets the chart from the host"
    );
    let map =
        h.mp.as_ref()
            .and_then(|m| m.map.clone())
            .expect("the lobby map");
    let mut log = String::new();
    let _ = writeln!(
        log,
        "lobby map: {} - {} [{}], osu! set: {:?}",
        map.artist, map.title, map.version, map.set_id
    );
    let _ = writeln!(log, "guest has the map: true");
    let _ = writeln!(
        log,
        "guest's TJA folder: {:?}",
        files_under(&guest_dirs.tja())
    );
    let _ = writeln!(
        log,
        "guest's Songs folder: {:?}",
        files_under(&guest_dirs.songs())
    );
    let got = &g.library[library::find_map(&g.library, &map).expect("in the guest's list")];
    assert!(got.is_tja());
    let _ = writeln!(
        log,
        "guest's entry: {} [{}], TJA: {}",
        got.meta.title,
        got.meta.version,
        got.is_tja()
    );
    let host_entry = &h.library[library::find_map(&h.library, &map).unwrap()];
    let times = |p: &std::path::Path| -> Vec<f64> {
        beatmap::load(p)
            .unwrap()
            .notes
            .iter()
            .map(|n| n.time)
            .collect()
    };
    assert_eq!(
        times(&got.path),
        times(&host_entry.path),
        "the guest plays the host's chart"
    );
    // A match on it stays off the online server, like a TJA play on its own.
    assert!(
        h.live_is_tja() && g.live_is_tja(),
        "the lobby's map is a TJA chart"
    );
    assert_snapshot("network-tja-chart-from-the-host", &log);
}

/// A guest with the same chart written otherwise (CRLF, a comment, another folder) has the
/// host's course already: nothing is sent.
#[test]
fn a_guest_with_the_same_tja_chart_plays_it_without_a_transfer() {
    let copy = format!("// my copy\r\n{}", LOBBY_TJA.replace('\n', "\r\n"));
    // On a server that passes no TJA charts on: none is needed.
    let (mut h, mut g, _host_dirs, guest_dirs, mut now) = tja_lobby(
        "have",
        &[
            ("Mine/chart.tja", copy.as_bytes()),
            ("Mine/song.ogg", &[7u8; 64 << 10]),
        ],
        false,
    );
    assert!(
        both_until(&mut h, &mut g, &mut now, Duration::from_secs(10), |_, g| {
            has_lobby_map(g)
        }),
        "the guest's own copy is the lobby's map"
    );
    assert!(g.host_transfer.is_none() && g.downloads.is_empty());
    assert_eq!(
        files_under(&guest_dirs.tja()),
        ["Mine/chart.tja", "Mine/song.ogg"],
        "nothing came from the host"
    );
}

/// On a server that doesn't list `TJA_LOBBY_FILES_FEATURE` (switched off) the host sends no TJA
/// chart: the guest is told so, gets nothing, and doesn't look for it on the mirrors.
#[test]
fn a_host_sends_no_tja_chart_through_a_server_that_doesnt_pass_them_on() {
    let (mut h, mut g, _host_dirs, guest_dirs, mut now) = tja_lobby("off", &[], false);
    let told = "the server doesn't pass the host's TJA chart on";
    let toast = |g: &State| g.toast.as_ref().map(|t| t.text.clone());
    assert!(
        both_until(&mut h, &mut g, &mut now, Duration::from_secs(10), |_, g| {
            toast(g).as_deref() == Some(told)
        }),
        "the guest is told the host can't send it: {:?}",
        toast(&g)
    );
    // A little longer: nothing more comes.
    let _ = both_until(
        &mut h,
        &mut g,
        &mut now,
        Duration::from_millis(500),
        |_, _| false,
    );
    assert!(!has_lobby_map(&g));
    assert!(
        files_under(&guest_dirs.tja()).is_empty(),
        "nothing came from the host"
    );
    assert!(
        g.downloads.is_empty() && g.host_transfer.is_none() && !g.lobby_lookup.is_busy(),
        "no download, no mirror search"
    );
}

/// A difficulty as osu! serves it: its beatmap ID, the music it plays and `notes` notes.
fn osu_diff(id: u32, version: &str, audio: &str, notes: usize) -> Vec<u8> {
    let mut text = format!(
        "osu file format v14\n\n[General]\nAudioFilename: {audio}\nMode: 1\n\n[Metadata]\nTitle:Song\nArtist:Band\nCreator:me\nVersion:{version}\nBeatmapID:{id}\nBeatmapSetID:1\n\n[HitObjects]\n"
    );
    for i in 0..notes {
        let _ = writeln!(text, "256,192,{},1,0", 1000 + i * 250);
    }
    text.into_bytes()
}

/// What a download unpacks when the first mirror (`a`: description, archive) may hold an old
/// version of the set and osu! answers `osu` (beatmap ID, file) for its own files: the set's
/// folder, the paths asked for in order, and whether it's flagged as an older version.
fn download_with_osus_files(
    name: &str,
    a: (Vec<u8>, Vec<u8>),
    b: (Vec<u8>, Vec<u8>),
    osu: &[(u32, Vec<u8>)],
) -> (SetFiles, Vec<String>, bool) {
    let mut routes = vec![
        ("/a/s/1", a.0),
        ("/a/d/1", a.1),
        ("/b/s/1", b.0),
        ("/b/d/1", b.1),
    ];
    for (id, file) in osu {
        let path: &'static str = Box::leak(format!("/osu/{id}").into_boxed_str());
        routes.push((path, file.clone()));
    }
    let (base, asked) = mirror_server(routes);
    let songs = std::env::temp_dir().join(format!(
        "taiko-rs-regression-osus-files-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&songs);
    let sources = [
        ("a", format!("{base}/a/d/1")),
        ("b", format!("{base}/b/d/1")),
    ];
    let mut check = crate::download::Check::new(
        vec![
            ("a", format!("{base}/a/s/1")),
            ("b", format!("{base}/b/s/1")),
        ],
        None,
    )
    .with_osu_files(format!("{base}/osu/"));
    let (done, total) = (
        std::sync::atomic::AtomicU64::new(0),
        std::sync::atomic::AtomicU64::new(0),
    );
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let folder = crate::download::fetch(
        &sources,
        "1 Band - Song",
        &songs,
        &done,
        &total,
        &cancel,
        Some(&mut check),
    )
    .unwrap();
    let mut files: SetFiles = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    let left: Vec<String> = std::fs::read_dir(&songs)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".osz") || n.contains("partial"))
        .collect();
    assert!(left.is_empty(), "left in the Songs folder: {left:?}");
    let _ = std::fs::remove_dir_all(&songs);
    (files, asked.try_iter().collect(), check.older)
}

#[test]
fn an_old_archive_is_made_current_with_osus_own_files_without_asking_the_next_mirror() {
    let (oni, kantan) = (
        osu_diff(1, "Oni", "audio.mp3", 40),
        osu_diff(2, "Kantan", "audio.mp3", 12),
    );
    // As osu.direct had a set ranked a week before: the Oni before its last fix, the Kantan an
    // empty placeholder.
    let (old_oni, empty_kantan) = (
        osu_diff(1, "Oni", "audio.mp3", 39),
        osu_diff(2, "Kantan", "audio.mp3", 0),
    );
    let current = [(1, "Oni", oni.as_slice()), (2, "Kantan", kantan.as_slice())];
    let (files, asked, older) = download_with_osus_files(
        "patched",
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[
                ("Band - Song (me) [Oni].osu", &old_oni),
                ("Band - Song (me) [Kantan].osu", &empty_kantan),
                ("audio.mp3", b"mp3"),
            ]),
        ),
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("never.osu", &oni)]),
        ),
        &[(1, oni.clone()), (2, kantan.clone())],
    );
    assert_eq!(
        files,
        vec![
            ("Band - Song (me) [Kantan].osu".to_string(), kantan),
            ("Band - Song (me) [Oni].osu".to_string(), oni),
            ("audio.mp3".to_string(), b"mp3".to_vec()),
        ],
        "osu!'s files take the old ones' places, the music stays"
    );
    assert!(
        !asked.iter().any(|p| p == "/b/d/1"),
        "the second mirror's archive isn't needed: {asked:?}"
    );
    assert!(asked.iter().any(|p| p == "/osu/1") && asked.iter().any(|p| p == "/osu/2"));
    assert!(!older);
}

#[test]
fn osus_file_needing_music_the_archive_lacks_leaves_it_to_the_next_mirror() {
    // The update brought new music: an old archive can't be made current with .osu files.
    let oni = osu_diff(1, "Oni", "new.mp3", 40);
    let old_oni = osu_diff(1, "Oni", "audio.mp3", 39);
    let current = [(1, "Oni", oni.as_slice())];
    let (files, asked, older) = download_with_osus_files(
        "new-music",
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &old_oni), ("audio.mp3", b"old")]),
        ),
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &oni), ("new.mp3", b"new")]),
        ),
        &[(1, oni.clone())],
    );
    assert_eq!(
        files,
        vec![
            ("new.mp3".to_string(), b"new".to_vec()),
            ("oni.osu".to_string(), oni),
        ]
    );
    assert!(asked.iter().any(|p| p == "/b/d/1"), "{asked:?}");
    assert!(!older);
}

#[test]
fn an_empty_answer_from_osu_leaves_it_to_the_next_mirror() {
    // osu! answers a difficulty it can't hand out with an empty file.
    let oni = osu_diff(1, "Oni", "audio.mp3", 40);
    let old_oni = osu_diff(1, "Oni", "audio.mp3", 39);
    let current = [(1, "Oni", oni.as_slice())];
    let (files, asked, older) = download_with_osus_files(
        "empty-answer",
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &old_oni), ("audio.mp3", b"mp3")]),
        ),
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &oni), ("audio.mp3", b"mp3")]),
        ),
        &[(1, Vec::new())],
    );
    assert_eq!(
        files,
        vec![
            ("audio.mp3".to_string(), b"mp3".to_vec()),
            ("oni.osu".to_string(), oni),
        ]
    );
    assert!(asked.iter().any(|p| p == "/b/d/1"), "{asked:?}");
    assert!(!older);
}

#[test]
fn an_old_version_unpacked_for_want_of_a_current_one_is_said_to_be_old() {
    // Song select would offer the update again on every launch: the game must not call it done.
    let oni = osu_diff(1, "Oni", "audio.mp3", 40);
    let old_oni = osu_diff(1, "Oni", "audio.mp3", 39);
    let current = [(1, "Oni", oni.as_slice())];
    let old_set = || {
        (
            mirror_json("2026-11-04T00:38:02Z", &current),
            mirror_osz(&[("oni.osu", &old_oni), ("audio.mp3", b"mp3")]),
        )
    };
    let (files, _, older) = download_with_osus_files("all-old", old_set(), old_set(), &[]);
    assert_eq!(
        files,
        vec![
            ("audio.mp3".to_string(), b"mp3".to_vec()),
            ("oni.osu".to_string(), old_oni),
        ]
    );
    assert!(older);
}

/// A set's folder for an update: each file (name, contents).
fn set_folder(name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-regression-update-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (n, bytes) in files {
        std::fs::write(dir.join(n), bytes).unwrap();
    }
    dir
}

fn online_set(maps: &[(u32, &str, &[u8])]) -> crate::download::OnlineSet {
    maps.iter()
        .map(|(id, version, osu)| {
            (
                *id,
                crate::download::OnlineDiff {
                    md5: crate::download::md5_hex(osu),
                    version: Some(version.to_string()),
                },
            )
        })
        .collect()
}

fn folder_contents(dir: &std::path::Path) -> SetFiles {
    let mut files: SetFiles = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn an_update_takes_only_the_changed_files_from_osu_and_leaves_edits_alone() {
    let (old_oni, oni) = (
        osu_diff(1, "Oni", "audio.mp3", 39),
        osu_diff(1, "Oni", "audio.mp3", 40),
    );
    let kantan = osu_diff(2, "Kantan", "audio.mp3", 12);
    // A copy with a rate baked in keeps the original's IDs under another name.
    let rate = osu_diff(1, "Oni 1.2x", "audio.mp3", 39);
    let dir = set_folder(
        "only-changed",
        &[
            ("Band - Song (me) [Oni].osu", &old_oni),
            ("Band - Song (me) [Oni 1.2x].osu", &rate),
            ("audio.mp3", b"mp3"),
        ],
    );
    let (base, asked) = mirror_server(vec![("/osu/1", oni.clone()), ("/osu/2", kantan.clone())]);
    let own = vec![crate::download::LocalDiff {
        id: Some(1),
        version: "Oni".into(),
        path: dir.join("Band - Song (me) [Oni].osu"),
    }];
    let online = online_set(&[(1, "Oni", &oni), (2, "Kantan", &kantan)]);
    crate::download::update_from_osu(&dir, &own, &online, &format!("{base}/osu/")).unwrap();
    assert_eq!(
        folder_contents(&dir),
        vec![
            ("Band - Song (me) [Kantan].osu".to_string(), kantan),
            ("Band - Song (me) [Oni 1.2x].osu".to_string(), rate),
            ("Band - Song (me) [Oni].osu".to_string(), oni),
            ("audio.mp3".to_string(), b"mp3".to_vec()),
        ],
        "the old Oni is replaced in place, the rate copy stays, no .osu.old is left"
    );
    assert_eq!(asked.try_iter().collect::<Vec<_>>(), ["/osu/1", "/osu/2"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_update_needing_new_music_leaves_the_folder_as_it_was() {
    let old_oni = osu_diff(1, "Oni", "audio.mp3", 39);
    let oni = osu_diff(1, "Oni", "new.mp3", 40);
    let dir = set_folder(
        "new-music",
        &[
            ("Band - Song (me) [Oni].osu", &old_oni),
            ("audio.mp3", b"mp3"),
        ],
    );
    let before = folder_contents(&dir);
    let (base, _) = mirror_server(vec![("/osu/1", oni.clone())]);
    let own = vec![crate::download::LocalDiff {
        id: Some(1),
        version: "Oni".into(),
        path: dir.join("Band - Song (me) [Oni].osu"),
    }];
    let online = online_set(&[(1, "Oni", &oni)]);
    let got = crate::download::update_from_osu(&dir, &own, &online, &format!("{base}/osu/"));
    assert!(got.is_err_and(|e| e.contains("new.mp3")));
    assert_eq!(folder_contents(&dir), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_difficulty_uploaded_again_under_a_new_id_replaces_its_old_file_by_name() {
    // Fractal Vertex: the archive's Endless Vortex still has the ID of the upload osu! deleted.
    let vortex = osu_diff(5939066, "Endless Vortex", "audio.mp3", 40);
    let old_vortex = osu_diff(5776969, "Endless Vortex", "audio.mp3", 39);
    let current = [(5939066, "Endless Vortex", vortex.as_slice())];
    let (files, asked, older) = download_with_osus_files(
        "new-id",
        (
            mirror_json("2026-11-13T18:40:28Z", &current),
            mirror_osz(&[
                ("Band - Song (me) [Endless Vortex].osu", &old_vortex),
                ("audio.mp3", b"mp3"),
            ]),
        ),
        (
            mirror_json("2026-11-13T18:40:28Z", &current),
            mirror_osz(&[("never.osu", &vortex)]),
        ),
        &[(5939066, vortex.clone())],
    );
    assert_eq!(
        files,
        vec![
            ("Band - Song (me) [Endless Vortex].osu".to_string(), vortex),
            ("audio.mp3".to_string(), b"mp3".to_vec()),
        ]
    );
    assert!(!asked.iter().any(|p| p == "/b/d/1"), "{asked:?}");
    assert!(!older);
}

#[test]
fn an_update_replaces_the_old_file_of_a_difficulty_uploaded_again_under_a_new_id() {
    // TUYU - Kako ni Torawarete Iru: the folder's [Bow down] has the ID osu! deleted with it.
    let old_bow = osu_diff(2250000, "Bow down", "audio.mp3", 39);
    let bow = osu_diff(2589409, "Bow down", "audio.mp3", 40);
    // A home-made difficulty without an ID, and one that's another set's, stay.
    let mine = osu_diff(0, "Bow down", "audio.mp3", 10);
    let dir = set_folder(
        "uploaded-again",
        &[
            ("Band - Song (me) [Bow down].osu", &old_bow),
            ("mine.osu", &mine),
            ("audio.mp3", b"mp3"),
        ],
    );
    let (base, _) = mirror_server(vec![("/osu/2589409", bow.clone())]);
    let online = online_set(&[(2589409, "Bow down", &bow)]);
    crate::download::update_from_osu(&dir, &[], &online, &format!("{base}/osu/")).unwrap();
    assert_eq!(
        folder_contents(&dir),
        vec![
            ("Band - Song (me) [Bow down].osu".to_string(), bow),
            ("audio.mp3".to_string(), b"mp3".to_vec()),
            ("mine.osu".to_string(), mine),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_update_goes_by_the_name_in_osus_file_not_the_mirrors_wording_of_it() {
    // osu.direct calls a mania difficulty "[4K] Bow down"; its file says "Bow down".
    let old_bow = osu_diff(2589409, "Bow down", "audio.mp3", 39);
    let bow = osu_diff(2589409, "Bow down", "audio.mp3", 40);
    let rate = osu_diff(2589409, "Bow down 1.2x", "audio.mp3", 39);
    let dir = set_folder(
        "mirror-wording",
        &[
            ("Band - Song (me) [Bow down].osu", &old_bow),
            ("Band - Song (me) [Bow down 1.2x].osu", &rate),
            ("audio.mp3", b"mp3"),
        ],
    );
    let (base, _) = mirror_server(vec![("/osu/2589409", bow.clone())]);
    let online = online_set(&[(2589409, "[4K] Bow down", &bow)]);
    crate::download::update_from_osu(&dir, &[], &online, &format!("{base}/osu/")).unwrap();
    assert_eq!(
        folder_contents(&dir),
        vec![
            ("Band - Song (me) [Bow down 1.2x].osu".to_string(), rate),
            ("Band - Song (me) [Bow down].osu".to_string(), bow),
            ("audio.mp3".to_string(), b"mp3".to_vec()),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A mirror that turns the game away (403, catboy.best's guard) is asked once per download, not
/// again on every pass: asking again only keeps the guard shut, and the waits between passes
/// held the error back for no gain.
#[test]
fn a_mirror_refusing_a_download_is_not_asked_again() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, asked) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let reply: &[u8] = if path.starts_with("/guard/") {
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            };
            let _ = tx.send(path);
            let _ = out.write_all(reply);
        }
    });
    let songs = std::env::temp_dir().join(format!(
        "taiko-rs-regression-mirror-403-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&songs);
    let sources = [
        ("none", format!("{base}/none/d/1")),
        ("guard", format!("{base}/guard/d/1")),
    ];
    let (done, total) = (
        std::sync::atomic::AtomicU64::new(0),
        std::sync::atomic::AtomicU64::new(0),
    );
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let result = crate::download::fetch(
        &sources,
        "1 Marika - Soul Release",
        &songs,
        &done,
        &total,
        &cancel,
        None,
    );
    let _ = std::fs::remove_dir_all(&songs);
    let err = result.unwrap_err();
    assert!(err.contains("403"), "{err}");
    let asked: Vec<String> = asked.try_iter().collect();
    assert_eq!(asked, ["/none/d/1", "/guard/d/1"]);
    // No pass after the first, so none of the waits between them.
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
}

/// When the primary mirror fails (e.g. osu.direct 503) and a fallback (mirror.nekoha.moe)
/// answers, future searches — including after a restart — go straight to the working mirror
/// instead of stalling on the broken one every time.
#[test]
fn failing_mirror_switches_search_permanently_to_nekoha_fallback() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, asked) = std::sync::mpsc::channel();
    let nekoha_body = r#"{"beatmapsets":[{"id":2617901,"artist":"kaitendaentai","title":"Hubris","creator":"ulko","status":"ranked","ranked_date":"2026-11-24T00:03:28Z","beatmaps":[{"id":5872564,"version":"Oni","mode":"taiko","mode_int":1,"difficulty_rating":6.2}]}]}"#;
    let nekoha_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        nekoha_body.len(),
        nekoha_body
    );
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let _ = tx.send(path.clone());
            let reply = if path.starts_with("/primary/") {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else if path.starts_with("/blocked/") {
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                &nekoha_resp
            };
            let _ = out.write_all(reply.as_bytes());
        }
    });

    let primary_search: &'static str = Box::leak(format!("{base}/primary/search").into_boxed_str());
    let nekoha_search: &'static str = Box::leak(format!("{base}/nekoha/search").into_boxed_str());
    let blocked_search: &'static str = Box::leak(format!("{base}/blocked/search").into_boxed_str());
    let mirrors = [
        crate::download::Mirror {
            name: "osu.direct",
            search: primary_search,
            search_style: crate::download::SearchStyle::V2 {
                query_key: "query",
                amount_key: "amount",
                server_sort: true,
            },
            set: "",
            download: "",
            osu: "",
        },
        crate::download::Mirror {
            name: "mirror.nekoha.moe",
            search: nekoha_search,
            search_style: crate::download::SearchStyle::Nekoha,
            set: "",
            download: "",
            osu: "",
        },
        crate::download::Mirror {
            name: "catboy.best",
            search: blocked_search,
            search_style: crate::download::SearchStyle::V2 {
                query_key: "q",
                amount_key: "limit",
                server_sort: false,
            },
            set: "",
            download: "",
            osu: "",
        },
    ];

    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-regression-mirror-switch-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let state_path = dir.join("mirror-order");
    let defaults: Vec<String> = mirrors.iter().map(|m| m.name.to_string()).collect();
    let router = std::sync::Mutex::new(crate::download::MirrorRouter::new(
        defaults.clone(),
        Some(state_path.clone()),
    ));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .into();

    // First search: primary 503s -> falls back to nekoha -> succeeds and switches permanently.
    let (sets, more) = crate::download::search_status_from(
        &agent,
        &mirrors,
        &router,
        "hubris",
        Some(1),
        (crate::download::SortKey::Ranked, true),
        0,
    )
    .unwrap();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].id, 2617901);
    assert!(!more);

    let first_round: Vec<String> = asked.try_iter().collect();
    assert_eq!(first_round.len(), 2);
    assert!(first_round[0].starts_with("/primary/search?"));
    assert!(first_round[1].starts_with("/nekoha/search?"));
    assert!(first_round[1].contains("mode=taiko"));
    assert!(first_round[1].contains("page=1"));
    assert!(first_round[1].contains("status=ranked"));
    assert!(first_round[1].contains("sort=ranked"));
    assert!(first_round[1].contains("order=desc"));
    assert!(first_round[1].contains("q=hubris"));

    // Second search: goes straight to nekoha without touching the failing primary.
    let (sets2, _) = crate::download::search_status_from(
        &agent,
        &mirrors,
        &router,
        "hubris",
        Some(1),
        (crate::download::SortKey::Ranked, true),
        0,
    )
    .unwrap();
    assert_eq!(sets2[0].id, 2617901);
    let second_round: Vec<String> = asked.try_iter().collect();
    assert_eq!(second_round.len(), 1);
    assert!(second_round[0].starts_with("/nekoha/search?"));

    // Fresh router (simulated restart): loads persisted order and still goes straight to nekoha.
    let restarted = std::sync::Mutex::new(crate::download::MirrorRouter::new(
        defaults,
        Some(state_path),
    ));
    let (sets3, _) = crate::download::search_status_from(
        &agent,
        &mirrors,
        &restarted,
        "hubris",
        Some(1),
        (crate::download::SortKey::Ranked, true),
        0,
    )
    .unwrap();
    assert_eq!(sets3[0].id, 2617901);
    let third_round: Vec<String> = asked.try_iter().collect();
    assert_eq!(third_round.len(), 1);
    assert!(third_round[0].starts_with("/nekoha/search?"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// A geo-blocked mirror (403) is demoted and skipped on subsequent `set_status` calls, while a
/// 404 on one set does not demote a healthy mirror.
#[test]
fn blocked_mirror_is_skipped_on_subsequent_set_status_lookups() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, asked) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let _ = tx.send(path.clone());
            let reply = if path.starts_with("/blocked/") {
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else if path.ends_with("/999") {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{\"status\":\"ranked\"}"
            };
            let _ = out.write_all(reply.as_bytes());
        }
    });

    let blocked_set: &'static str = Box::leak(format!("{base}/blocked/s/").into_boxed_str());
    let good_set: &'static str = Box::leak(format!("{base}/good/s/").into_boxed_str());
    let mirrors = [
        crate::download::Mirror {
            name: "catboy.best",
            search: "",
            search_style: crate::download::SearchStyle::V2 {
                query_key: "q",
                amount_key: "limit",
                server_sort: false,
            },
            set: blocked_set,
            download: "",
            osu: "",
        },
        crate::download::Mirror {
            name: "mirror.nekoha.moe",
            search: "",
            search_style: crate::download::SearchStyle::Nekoha,
            set: good_set,
            download: "",
            osu: "",
        },
    ];
    let defaults: Vec<String> = mirrors.iter().map(|m| m.name.to_string()).collect();
    let router = std::sync::Mutex::new(crate::download::MirrorRouter::new(defaults, None));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .into();

    // First lookup: blocked returns 403 -> good returns ranked -> blocked is demoted & cooled down.
    assert_eq!(
        crate::download::set_status_from(&agent, &mirrors, &router, 1).unwrap(),
        "ranked"
    );
    assert_eq!(
        asked.try_iter().collect::<Vec<_>>(),
        ["/blocked/s/1", "/good/s/1"]
    );

    // Next lookup skips the blocked mirror completely; 404 returns MISSING without hurting good.
    assert_eq!(
        crate::download::set_status_from(&agent, &mirrors, &router, 999).unwrap(),
        crate::setstatus::MISSING
    );
    assert_eq!(asked.try_iter().collect::<Vec<_>>(), ["/good/s/999"]);
}
