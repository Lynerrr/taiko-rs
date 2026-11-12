//! A mapping session played out in memory: a fake server that orders ops like the real one
//! (refusals, deduplication by join and client sequence, a log with snapshots) and games that
//! edit through the real editor, with random interleavings of edits, sends and deliveries.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use taiko_proto::mapping::{
    self, Document, MapUp, Mode, Part, Participant, Role, Sequenced, SessionState, SnapshotData,
};

use super::super::chart::tests::MAP;
use super::super::chart::{Chart, HitObject, TimingPoint};
use super::super::history::{Command, History};
use super::super::setup::Key;
use super::super::state::Editor;
use super::*;

/// A small deterministic generator (xorshift), so a failing interleaving can be replayed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

enum Msg {
    Op(Sequenced),
    Refused(u64),
}

struct Server {
    state: SessionState,
    /// The latest snapshot and the seq it is at; `log` holds every op after it.
    base: (u64, SnapshotData),
    log: Vec<Sequenced>,
    next_seq: u64,
    /// The last client sequence taken from each join (in order: one connection at a time).
    done: HashMap<u32, u64>,
    inbox: Vec<VecDeque<Msg>>,
}

impl Server {
    fn new(state: SessionState, base: SnapshotData, games: usize) -> Server {
        Server {
            state,
            base: (0, base),
            log: Vec::new(),
            next_seq: 1,
            done: HashMap::new(),
            inbox: (0..games).map(|_| VecDeque::new()).collect(),
        }
    }

    fn submit(&mut self, from: usize, author: u32, epoch: u32, up: MapUp) {
        let MapUp::Op {
            client_seq,
            diff,
            op,
        } = up
        else {
            return;
        };
        let done = self.done.entry(epoch).or_default();
        if client_seq <= *done {
            return;
        }
        *done = client_seq;
        if mapping::refusal(&self.state, author, diff, &op).is_some() {
            self.inbox[from].push_back(Msg::Refused(client_seq));
            return;
        }
        let s = Sequenced {
            seq: self.next_seq,
            author,
            epoch,
            client_seq,
            diff,
            op,
        };
        self.next_seq += 1;
        self.log.push(s.clone());
        for q in &mut self.inbox {
            q.push_back(Msg::Op(s.clone()));
        }
    }

    /// Shortens the log with a game's snapshot of its confirmed state.
    fn compact(&mut self, seq: u64, data: SnapshotData) {
        if seq > self.base.0 {
            self.log.retain(|s| s.seq > seq);
            self.base = (seq, data);
        }
    }

    /// Every difficulty as the log makes it, from the snapshot.
    fn replayed(&self) -> Vec<Chart> {
        let mut docs: Vec<Chart> = self.base.1.docs.iter().map(|d| load(d).unwrap()).collect();
        for s in &self.log {
            let Ok(cmd) = from_wire(&s.op) else { continue };
            for (diff, c) in docs.iter_mut().enumerate() {
                if s.diff.is_none_or(|d| d == diff as u32) {
                    apply_tolerant(&cmd, c);
                }
            }
        }
        docs
    }
}

struct Game {
    sync: Sync,
    ed: Editor,
    me: u32,
    epoch: u32,
    online: bool,
    /// Sent, not yet at the server (lost if the connection drops).
    wire: VecDeque<MapUp>,
}

impl Game {
    fn join(server: &Server, me: u32, epoch: u32, open: u32) -> Game {
        let (sync, ed) = Sync::join(
            &server.base.1,
            server.base.0,
            me,
            epoch,
            open,
            PathBuf::from("map.osu"),
        )
        .unwrap();
        Game {
            sync,
            ed,
            me,
            epoch,
            online: true,
            wire: VecDeque::new(),
        }
    }

    fn flush(&mut self, now: f64) {
        self.sync.collect(&mut self.ed);
        self.wire.extend(self.sync.outgoing(now, false));
    }

    fn take(&mut self, msg: Msg, server: &Server) {
        match msg {
            Msg::Op(s) => {
                if self.sync.receive(&mut self.ed, &s) == Err(OutOfStep) {
                    self.resume(server);
                }
            }
            Msg::Refused(cs) => self.sync.refused(&mut self.ed, cs),
        }
    }

    fn resume(&mut self, server: &Server) {
        let (seq, data) = &server.base;
        let done = server.done.get(&self.epoch).copied().unwrap_or(0);
        // A new connection: nothing of the old one arrives any more.
        self.wire.clear();
        self.sync
            .resync(&mut self.ed, data, *seq, &server.log, done)
            .unwrap();
        self.online = true;
    }
}

/// A chart as compared: the file it writes and each thing's id.
/// The file, then each timing point's and object's id and line.
type Canon = (String, Vec<(u64, String)>, Vec<(u64, String)>);

fn canon(c: &Chart) -> Canon {
    (
        c.to_text(),
        c.timing.iter().map(|p| (p.id, p.to_line(""))).collect(),
        c.objects.iter().map(|o| (o.id, o.to_line(""))).collect(),
    )
}

fn participant(id: u32, role: Role) -> Participant {
    Participant {
        id,
        name: format!("p{id}"),
        role,
        colour: 0,
        online: true,
    }
}

/// Host 1; mappers 2 and 3 own the first and second half of "Oni" (collab); 2 owns "Muzukashii"
/// whole (a guest difficulty).
fn session() -> SessionState {
    SessionState {
        session: 1,
        title: "Test".into(),
        mode: Mode::Collab,
        host: 1,
        participants: vec![
            participant(1, Role::Host),
            participant(2, Role::Mapper),
            participant(3, Role::Mapper),
        ],
        documents: vec![
            Document {
                diff: 0,
                name: "Oni".into(),
                owner: None,
                parts: vec![
                    Part {
                        from: 0.0,
                        to: 5000.0,
                        assignee: Some(2),
                    },
                    Part {
                        from: 5000.0,
                        to: 20000.0,
                        assignee: Some(3),
                    },
                ],
                locked: false,
                accepted: false,
            },
            Document {
                diff: 1,
                name: "Muzukashii".into(),
                owner: Some(2),
                parts: Vec::new(),
                locked: false,
                accepted: false,
            },
        ],
        comments: Vec::new(),
        folder: String::new(),
        assets: Vec::new(),
    }
}

fn files() -> Vec<(String, String)> {
    vec![
        ("Oni".into(), MAP.into()),
        (
            "Muzukashii".into(),
            MAP.replace("Version:Oni", "Version:Muzukashii"),
        ),
    ]
}

/// One random edit through the editor, as a player makes them.
fn edit(g: &mut Game, rng: &mut Rng) {
    let ed = &mut g.ed;
    let pick = |ed: &Editor, rng: &mut Rng| -> Option<u64> {
        let n = ed.chart.objects.len() as u64;
        (n > 0).then(|| ed.chart.objects[rng.below(n) as usize].id)
    };
    match rng.below(11) {
        0..=2 => {
            let t = rng.below(80) as f64 * 125.0;
            ed.place_hit(t, rng.below(2) == 0);
        }
        3 => {
            if let Some(id) = pick(ed, rng) {
                ed.selection = [id].into();
                let delta = (rng.below(9) as f64 - 4.0) * 125.0;
                ed.move_selected(delta, false);
            }
        }
        4 => {
            // A drag: several steps, one undo.
            if let Some(id) = pick(ed, rng) {
                ed.selection = [id].into();
                for _ in 0..3 {
                    ed.move_selected(10.0, true);
                }
                ed.end_drag();
            }
        }
        5 => {
            if let Some(id) = pick(ed, rng) {
                ed.selection = [id].into();
                ed.delete_selected();
            }
        }
        6 => {
            if let Some(id) = pick(ed, rng) {
                ed.selection = [id].into();
                ed.retype_selected(Some(rng.below(2) == 0), None);
            }
        }
        7 => {
            let t = rng.below(40) as f64 * 250.0;
            let _ = ed.add_green(t, 0.5 + rng.below(4) as f64 * 0.25);
        }
        8 => {
            let _ = ed.set_setting(Key::Od, &format!("{}", rng.below(10)));
        }
        9 => {
            if rng.below(2) == 0 {
                ed.undo();
            } else {
                ed.redo();
            }
        }
        _ => {
            // A BPM change: the host's goes to every difficulty, anyone else's is refused.
            let red = ed.chart.timing.iter().find(|p| p.is_red()).map(|p| p.id);
            if let Some(id) = red {
                let beat = 300.0 + rng.below(5) as f64 * 50.0;
                let _ = ed.edit_timing(id, |p| p.beat_length = beat, false);
            }
        }
    }
}

/// Plays a session of `steps` random events and checks every game ends with the same charts as
/// the server's log makes.
fn play(seed: u64, steps: usize, opens: [u32; 3]) {
    let mut rng = Rng(seed);
    let data = host_snapshot(&files()).unwrap();
    let mut server = Server::new(session(), data, 3);
    let mut games: Vec<Game> = (0..3)
        .map(|i| Game::join(&server, i as u32 + 1, i as u32 + 1, opens[i]))
        .collect();
    let mut now = 0.0;
    for _ in 0..steps {
        now += 16.0;
        let i = rng.below(3) as usize;
        match rng.below(12) {
            0..=3 => edit(&mut games[i], &mut rng),
            4..=5 => games[i].flush(now),
            6..=7 => {
                // Offline, what the game sends goes nowhere.
                if let Some(up) = games[i].wire.pop_front()
                    && games[i].online
                {
                    let (me, epoch) = (games[i].me, games[i].epoch);
                    server.submit(i, me, epoch, up);
                }
            }
            8..=9 => {
                if games[i].online
                    && let Some(msg) = server.inbox[i].pop_front()
                {
                    games[i].take(msg, &server);
                }
            }
            10 => {
                // The connection drops: what was on its way either way is lost.
                if games[i].online && rng.below(4) == 0 {
                    games[i].online = false;
                    games[i].wire.clear();
                    server.inbox[i].clear();
                } else if !games[i].online {
                    games[i].resume(&server);
                }
            }
            _ => {
                // The server shortens its log with an up-to-date game's snapshot.
                if games[i].online && server.inbox[i].is_empty() && rng.below(3) == 0 {
                    let data = games[i].sync.snapshot().unwrap();
                    server.compact(games[i].sync.last_seq(), data);
                }
            }
        }
    }
    // Everyone back online, everything sent and delivered.
    for g in &mut games {
        if !g.online {
            g.resume(&server);
        }
    }
    for _ in 0..20 {
        for (i, g) in games.iter_mut().enumerate() {
            now += 100.0;
            g.flush(now);
            while let Some(up) = g.wire.pop_front() {
                server.submit(i, g.me, g.epoch, up);
            }
        }
        for (i, g) in games.iter_mut().enumerate() {
            while let Some(msg) = server.inbox[i].pop_front() {
                g.take(msg, &server);
            }
        }
    }
    let truth = server.replayed();
    for (i, g) in games.iter().enumerate() {
        assert_eq!(g.sync.pending(), 0, "seed {seed}: game {i} still waits");
        for (diff, c) in truth.iter().enumerate() {
            assert_eq!(
                canon(g.sync.confirmed(diff as u32).unwrap()),
                canon(c),
                "seed {seed}: game {i}, difficulty {diff}"
            );
        }
        assert_eq!(
            canon(&g.ed.chart),
            canon(&truth[opens[i] as usize]),
            "seed {seed}: game {i}'s editor"
        );
    }
}

#[test]
fn random_interleavings_end_with_the_same_charts_everywhere() {
    for seed in 1..=40 {
        // The host and both collab mappers on Oni; then the guest on their own difficulty.
        play(seed * 7919, 600, [0, 0, 0]);
        play(seed * 104_729, 600, [0, 1, 0]);
    }
}

fn pair() -> (Server, Game, Game) {
    let data = host_snapshot(&files()).unwrap();
    let server = Server::new(session(), data, 2);
    let host = Game::join(&server, 1, 1, 0);
    let mapper = Game::join(&server, 2, 2, 0);
    (server, host, mapper)
}

/// Sends what each game has and delivers everything, until nothing moves.
fn settle(server: &mut Server, games: &mut [&mut Game]) {
    for _ in 0..3 {
        for (i, g) in games.iter_mut().enumerate() {
            g.flush(1e9);
            while let Some(up) = g.wire.pop_front() {
                server.submit(i, g.me, g.epoch, up);
            }
        }
        for (i, g) in games.iter_mut().enumerate() {
            while let Some(msg) = server.inbox[i].pop_front() {
                g.take(msg, server);
            }
        }
    }
}

fn has_note(ed: &Editor, t: f64) -> bool {
    ed.chart.objects.iter().any(|o| o.time == t)
}

#[test]
fn undo_takes_back_only_your_own_edits() {
    let (mut server, mut host, mut mapper) = pair();
    host.ed.place_hit(6000.0, false).unwrap();
    mapper.ed.place_hit(3000.0, true).unwrap();
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(has_note(&host.ed, 3000.0) && has_note(&mapper.ed, 6000.0));
    // The mapper's undo takes their own note away, not the host's newer one.
    assert!(mapper.ed.undo());
    settle(&mut server, &mut [&mut host, &mut mapper]);
    for g in [&host, &mapper] {
        assert!(!has_note(&g.ed, 3000.0) && has_note(&g.ed, 6000.0));
    }
    // And redo puts it back for everyone.
    assert!(mapper.ed.redo());
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(has_note(&host.ed, 3000.0));
}

#[test]
fn an_undo_others_overtook_is_passed_over() {
    let (mut server, mut host, mut mapper) = pair();
    let id = mapper.ed.place_hit(3000.0, false).unwrap();
    mapper.ed.selection = [id].into();
    assert!(mapper.ed.move_selected(250.0, false));
    settle(&mut server, &mut [&mut host, &mut mapper]);
    // The host moves the mapper's note on.
    host.ed.selection = [id].into();
    assert!(host.ed.move_selected(500.0, false));
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(has_note(&mapper.ed, 3750.0));
    // The mapper's move was overtaken: undo passes over it to the placing, which also finds
    // the note changed, so there is nothing of theirs left to take back.
    assert!(!mapper.ed.undo());
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(has_note(&host.ed, 3750.0) && has_note(&mapper.ed, 3750.0));
}

#[test]
fn an_edit_outside_your_part_is_refused_and_taken_back() {
    let (mut server, mut host, mut mapper) = pair();
    // The second half of Oni is mapper 3's.
    mapper.ed.place_hit(8000.0, false).unwrap();
    assert!(has_note(&mapper.ed, 8000.0), "shown at once");
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(!has_note(&mapper.ed, 8000.0) && !has_note(&host.ed, 8000.0));
    assert_eq!(mapper.sync.pending(), 0);
}

#[test]
fn a_collab_mapper_changes_the_shared_gimmick_for_everyone() {
    let (mut server, mut host, mut mapper) = pair();
    let after = crate::tko::gimmick::Gimmick {
        above: vec![1000.0],
        ..Default::default()
    };
    let cmd = Command::Gimmick {
        before: Box::default(),
        after: Box::new(after.clone()),
    };
    mapper.ed.history.run(&mut mapper.ed.chart, cmd);
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert_eq!(mapper.sync.pending(), 0, "taken, not refused");
    assert_eq!(host.ed.chart.gimmick.as_ref(), Some(&after));
    assert_eq!(mapper.ed.chart.gimmick.as_ref(), Some(&after));
}

#[test]
fn edits_made_offline_survive_a_reconnect() {
    let (mut server, mut host, mut mapper) = pair();
    mapper.ed.place_hit(1250.0, false).unwrap();
    mapper.flush(0.0);
    // The connection drops with the op on its way: lost.
    mapper.online = false;
    mapper.wire.clear();
    mapper.ed.place_hit(1750.0, true).unwrap();
    host.ed.place_hit(7000.0, false).unwrap();
    settle(&mut server, &mut [&mut host]);
    server.inbox[1].clear();
    mapper.resume(&server);
    assert!(has_note(&mapper.ed, 7000.0), "the host's note came in");
    assert!(has_note(&mapper.ed, 1250.0) && has_note(&mapper.ed, 1750.0));
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert!(has_note(&host.ed, 1250.0) && has_note(&host.ed, 1750.0));
    assert_eq!(canon(&host.ed.chart), canon(&mapper.ed.chart));
}

#[test]
fn an_op_the_server_already_has_is_not_applied_twice() {
    let (mut server, mut host, mut mapper) = pair();
    mapper.ed.place_hit(1250.0, false).unwrap();
    mapper.flush(0.0);
    let up = mapper.wire.pop_front().unwrap();
    server.submit(1, 2, 2, up.clone());
    // The ack is lost with the connection; on resuming the op is sent again.
    server.inbox[1].clear();
    mapper.resume(&server);
    server.submit(1, 2, 2, up);
    settle(&mut server, &mut [&mut host, &mut mapper]);
    assert_eq!(server.log.len(), 1);
    assert_eq!(mapper.sync.pending(), 0);
}

#[test]
fn the_hosts_red_lines_reach_every_difficulty() {
    let data = host_snapshot(&files()).unwrap();
    // The red line at 0 is the same in both: one id.
    assert_eq!(data.docs[0].timing_ids[0], data.docs[1].timing_ids[0]);
    assert_ne!(data.docs[0].timing_ids[1], data.docs[1].timing_ids[1]);
    let mut server = Server::new(session(), data, 2);
    let mut host = Game::join(&server, 1, 1, 0);
    let mut guest = Game::join(&server, 2, 2, 1);
    let red = host.ed.chart.timing[0].id;
    host.ed
        .edit_timing(red, |p| p.beat_length = 400.0, false)
        .unwrap();
    settle(&mut server, &mut [&mut host, &mut guest]);
    assert_eq!(guest.ed.chart.timing_point(red).unwrap().beat_length, 400.0);
    // The guest can't change it.
    guest
        .ed
        .edit_timing(red, |p| p.beat_length = 300.0, false)
        .unwrap();
    settle(&mut server, &mut [&mut host, &mut guest]);
    assert_eq!(guest.ed.chart.timing_point(red).unwrap().beat_length, 400.0);
    // Their own difficulty's notes are theirs.
    guest.ed.place_hit(8000.0, false).unwrap();
    settle(&mut server, &mut [&mut host, &mut guest]);
    assert!(
        host.sync
            .confirmed(1)
            .unwrap()
            .objects
            .iter()
            .any(|o| o.time == 8000.0)
    );
    let id = guest
        .ed
        .chart
        .objects
        .iter()
        .find(|o| o.time == 8000.0)
        .unwrap()
        .id;
    assert_eq!(host.sync.author(1, id), Some(2));
}

#[test]
fn a_snapshot_reads_back_with_the_same_ids_and_authors() {
    let (mut server, mut host, mut mapper) = pair();
    mapper.ed.place_hit(1250.0, false).unwrap();
    host.ed.add_green(6000.0, 1.5).unwrap();
    settle(&mut server, &mut [&mut host, &mut mapper]);
    let data = host.sync.snapshot().unwrap();
    let (back, _) = Sync::join(&data, 9, 3, 3, 0, PathBuf::from("x.osu")).unwrap();
    for diff in [0, 1] {
        assert_eq!(
            canon(back.confirmed(diff).unwrap()),
            canon(host.sync.confirmed(diff).unwrap())
        );
    }
    let id = mapper
        .ed
        .chart
        .objects
        .iter()
        .find(|o| o.time == 1250.0)
        .unwrap()
        .id;
    assert_eq!(back.author(0, id), Some(2));
}

#[test]
fn every_command_survives_the_wire() {
    let mut c = Chart::parse(MAP);
    let o = c.objects[0].clone();
    let mut moved = o.clone();
    moved.time = 1100.0;
    let p = c.timing[1].clone();
    let mut q = p.clone();
    q.set_sv(2.0).unwrap();
    let mut new = HitObject::hit(3000.0, true, true);
    new.id = c.take_id();
    let mut red = TimingPoint::red(8000.0, 200.0);
    red.id = c.take_id();
    let g = crate::tko::gimmick::Gimmick::default();
    let cmd = Command::Composite(vec![
        Command::InsertObjects(vec![new]),
        Command::RemoveObjects(vec![c.objects[1].clone()]),
        Command::EditObjects {
            before: vec![o],
            after: vec![moved],
        },
        Command::InsertTiming(vec![red]),
        Command::RemoveTiming(vec![p.clone()]),
        Command::EditTiming {
            before: vec![p],
            after: vec![q],
        },
        Command::Gimmick {
            before: Box::new(g.clone()),
            after: Box::new(g),
        },
        Command::SetKey {
            section: "Metadata",
            key: "Title",
            before: Some("Test".into()),
            after: Some("Other".into()),
        },
    ]);
    let wire = to_wire(&cmd);
    let back = from_wire(&wire).unwrap();
    assert_eq!(to_wire(&back), wire);
    let json = serde_json::to_string(&wire).unwrap();
    assert_eq!(serde_json::from_str::<mapping::MapOp>(&json).unwrap(), wire);
    // An unchanged object goes as the line the file holds.
    let MapOp::Composite(parts) = &wire else {
        unreachable!()
    };
    assert_eq!(
        parts[1],
        MapOp::RemoveObjects(vec![mapping::Obj {
            id: c.objects[1].id,
            line: "256,192,1500,1,2,0:0:0:0:".into(),
            fake: false,
            group: None,
        }])
    );
    // A key an op may not set, and a line with no id, are not commands.
    let bad = MapOp::SetKey {
        section: "General".into(),
        key: "AudioFilename".into(),
        before: None,
        after: Some("x.mp3".into()),
    };
    assert!(from_wire(&bad).is_err());
    let no_id = MapOp::InsertObjects(vec![mapping::Obj {
        id: 0,
        line: "256,192,1,1,0".into(),
        fake: false,
        group: None,
    }]);
    assert!(from_wire(&no_id).is_err());
}

#[test]
fn outside_a_session_nothing_is_kept_to_send_and_undo_is_as_before() {
    let mut c = Chart::parse(MAP);
    let mut h = History::new();
    h.run(
        &mut c,
        Command::InsertObjects(vec![HitObject::hit(3000.0, false, false)]),
    );
    assert!(h.take_outgoing().is_empty());
    // Undo takes back the step even when the chart changed under it (the single-player editor
    // never shares its chart).
    let id = c.objects.iter().find(|o| o.time == 3000.0).unwrap().id;
    c.object_mut(id).unwrap().sound = 2;
    assert!(h.undo(&mut c));
    assert!(c.object(id).is_none());
}

#[test]
fn a_drag_goes_out_a_few_times_a_second() {
    let (_, mut host, _) = pair();
    let id = host.ed.chart.objects[0].id;
    host.ed.selection = [id].into();
    let mut sent = 0;
    for step in 0..30 {
        host.ed.move_selected(1.0, true);
        host.sync.collect(&mut host.ed);
        sent += host.sync.outgoing(step as f64 * 10.0, true).len();
    }
    host.ed.end_drag();
    host.sync.collect(&mut host.ed);
    sent += host.sync.outgoing(300.0, false).len();
    assert!((5..=8).contains(&sent), "{sent} ops for a 300 ms drag");
    // Whatever went out, the last state did.
    assert_eq!(host.sync.pending(), sent);
}
