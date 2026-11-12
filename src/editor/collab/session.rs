//! A mapping session as the editor's screen sees it: the sync of the charts, who is in it, where
//! the others are (playhead, cursor, selection), the session chat, and the view's own choices
//! (following someone, colouring notes by who made them). It reads the server's `MapDown` lines
//! and makes the `MapUp` lines the game sends.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use taiko_proto::mapping::{MapDown, MapUp, Participant, SessionState};

use super::super::state::Editor;
use super::{OutOfStep, Sync};

/// Where the others are is sent at most this often (ms).
pub const PRESENCE_MS: f64 = 100.0;
/// Someone whose presence hasn't changed for this long shows as away (ms).
pub const AWAY_MS: f64 = 60_000.0;
/// Chat lines kept.
const CHAT_KEPT: usize = 200;

/// Another player's place in the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub diff: u32,
    /// Their playhead (map ms).
    pub time: f64,
    /// Where their pointer is on the timeline (map ms), when it is on it.
    pub cursor: Option<f64>,
    pub selection: Vec<u64>,
    /// When it last changed (the session's clock, ms).
    pub seen: f64,
}

/// What a line from the server asks of the game besides what the session does itself.
#[derive(Debug, PartialEq)]
pub enum Notice {
    /// Out of step with the server: resume the session (snapshot and the ops after it).
    Resume,
    /// One of our edits was refused: why.
    Refused(String),
    /// The session is over: why.
    Closed(String),
}

pub struct Session {
    pub sync: Sync,
    pub state: SessionState,
    pub me: u32,
    pub remotes: BTreeMap<u32, Remote>,
    /// (who, text), oldest first.
    pub chat: VecDeque<(u32, String)>,
    /// The view keeps to this player's playhead.
    pub follow: Option<u32>,
    /// Notes are drawn in the colour of who made them.
    pub by_author: bool,
    /// The last presence sent and when.
    sent: Option<(f64, Remote)>,
    /// What the session's clock counts from.
    started: Instant,
}

impl Session {
    pub fn new(sync: Sync, state: SessionState, me: u32) -> Session {
        Session {
            sync,
            state,
            me,
            remotes: BTreeMap::new(),
            chat: VecDeque::new(),
            follow: None,
            by_author: false,
            sent: None,
            started: Instant::now(),
        }
    }

    /// The session's clock (ms), for `handle`, `presence` and `away`.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub fn participant(&self, who: u32) -> Option<&Participant> {
        self.state.participants.iter().find(|p| p.id == who)
    }

    /// A player's colour (0xRRGGBB); grey for someone the session doesn't list.
    pub fn colour(&self, who: u32) -> u32 {
        self.participant(who).map_or(0x9A9AA8, |p| p.colour)
    }

    /// A difficulty's name.
    pub fn diff_name(&self, diff: u32) -> &str {
        self.state
            .documents
            .iter()
            .find(|d| d.diff == diff)
            .map_or("?", |d| d.name.as_str())
    }

    /// Whether a player in the session is away: online, but nothing of theirs moved for a while.
    pub fn away(&self, who: u32, now: f64) -> bool {
        self.remotes
            .get(&who)
            .is_some_and(|r| now - r.seen >= AWAY_MS)
    }

    /// Takes a line from the server (`now`: the session's clock, ms).
    pub fn handle(&mut self, ed: &mut Editor, down: MapDown, now: f64) -> Option<Notice> {
        match down {
            MapDown::State(state) => {
                // Someone who left takes their playhead with them, and isn't followed.
                self.remotes
                    .retain(|who, _| state.participants.iter().any(|p| p.id == *who && p.online));
                if self.follow.is_some_and(|f| !self.remotes.contains_key(&f)) {
                    self.follow = None;
                }
                self.state = state;
            }
            MapDown::Op(s) => {
                if self.sync.receive(ed, &s) == Err(OutOfStep) {
                    return Some(Notice::Resume);
                }
            }
            MapDown::Refused { client_seq, why } => {
                self.sync.refused(ed, client_seq);
                return Some(Notice::Refused(why));
            }
            MapDown::Presence {
                who,
                diff,
                time,
                cursor,
                selection,
            } if who != self.me => {
                let r = Remote {
                    diff,
                    time,
                    cursor,
                    selection,
                    seen: now,
                };
                self.remotes.insert(who, r);
            }
            MapDown::Chat { who, text, .. } => {
                self.chat.push_back((who, text));
                while self.chat.len() > CHAT_KEPT {
                    self.chat.pop_front();
                }
            }
            MapDown::Closed(why) => return Some(Notice::Closed(why)),
            MapDown::Presence { .. }
            | MapDown::Snapshot { .. }
            | MapDown::SnapshotWanted { .. }
            | MapDown::Pong => {}
        }
        None
    }

    /// Our place to send, when it changed and the last one went at least `PRESENCE_MS` ago.
    pub fn presence(&mut self, ed: &Editor, cursor: Option<f64>, now: f64) -> Option<MapUp> {
        let here = Remote {
            diff: self.sync.open,
            time: ed.time,
            cursor,
            selection: ed.selection.iter().copied().collect(),
            seen: now,
        };
        if let Some((at, last)) = &self.sent {
            let same = last.diff == here.diff
                && last.time == here.time
                && last.cursor == here.cursor
                && last.selection == here.selection;
            if same || now - at < PRESENCE_MS {
                return None;
            }
        }
        let up = MapUp::Presence {
            diff: here.diff,
            time: here.time,
            cursor: here.cursor,
            selection: here.selection.clone(),
        };
        self.sent = Some((now, here));
        Some(up)
    }

    /// The playhead to keep to while following someone on the same difficulty.
    pub fn followed_time(&self) -> Option<f64> {
        let r = self.remotes.get(&self.follow?)?;
        (r.diff == self.sync.open).then_some(r.time)
    }

    /// The colour a note of the open difficulty is drawn in while colouring by author.
    pub fn author_colour(&self, id: u64) -> Option<u32> {
        if !self.by_author {
            return None;
        }
        Some(self.colour(self.sync.author(self.sync.open, id)?))
    }

    /// Who has the object selected, other than us (the first of them).
    pub fn selected_by(&self, id: u64) -> Option<u32> {
        self.remotes
            .iter()
            .find(|(_, r)| r.diff == self.sync.open && r.selection.contains(&id))
            .map(|(who, _)| *who)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use taiko_proto::mapping::{Document, MapOp, Mode, Obj, Role, Sequenced};

    use super::super::super::chart::tests::MAP;
    use super::super::host_snapshot;
    use super::*;

    fn participant(id: u32, name: &str, role: Role, colour: u32) -> Participant {
        Participant {
            id,
            name: name.into(),
            role,
            colour,
            online: true,
        }
    }

    pub(crate) fn state() -> SessionState {
        SessionState {
            session: 1,
            title: "Test".into(),
            mode: Mode::Guest,
            host: 1,
            participants: vec![
                participant(1, "host", Role::Host, 0xF2D45C),
                participant(2, "anna", Role::Mapper, 0x5CC8F2),
                participant(3, "bob", Role::Modder, 0xB36BFF),
            ],
            documents: vec![
                Document {
                    diff: 0,
                    name: "Oni".into(),
                    owner: None,
                    parts: Vec::new(),
                    locked: false,
                    accepted: false,
                },
                Document {
                    diff: 1,
                    name: "Anna's Muzukashii".into(),
                    owner: Some(2),
                    parts: Vec::new(),
                    locked: false,
                    accepted: false,
                },
            ],
            comments: Vec::new(),
        }
    }

    /// The host's session on Oni, with the editor.
    pub(crate) fn session() -> (Session, Editor) {
        let files = vec![
            ("Oni".to_string(), MAP.to_string()),
            ("Anna's Muzukashii".to_string(), MAP.to_string()),
        ];
        let data = host_snapshot(&files).unwrap();
        let (sync, ed) = Sync::join(&data, 0, 1, 1, 0, PathBuf::from("map.osu")).unwrap();
        (Session::new(sync, state(), 1), ed)
    }

    fn presence(who: u32, diff: u32, time: f64, selection: Vec<u64>) -> MapDown {
        MapDown::Presence {
            who,
            diff,
            time,
            cursor: None,
            selection,
        }
    }

    #[test]
    fn the_others_places_are_kept_and_ours_ignored() {
        let (mut s, mut ed) = session();
        let id = ed.chart.objects[0].id;
        assert_eq!(
            s.handle(&mut ed, presence(2, 0, 1500.0, vec![id]), 10.0),
            None
        );
        assert_eq!(s.handle(&mut ed, presence(1, 0, 99.0, vec![]), 10.0), None);
        assert_eq!(s.remotes.len(), 1);
        assert_eq!(s.selected_by(id), Some(2));
        assert!(!s.away(2, 10.0 + AWAY_MS - 1.0) && s.away(2, 10.0 + AWAY_MS));
        // Following them keeps to their playhead, on our difficulty only.
        s.follow = Some(2);
        assert_eq!(s.followed_time(), Some(1500.0));
        s.handle(&mut ed, presence(2, 1, 2500.0, vec![]), 20.0);
        assert_eq!(s.followed_time(), None);
        assert_eq!(s.selected_by(id), None);
        // Gone offline: their playhead goes, and following them stops.
        let mut st = state();
        st.participants[1].online = false;
        s.handle(&mut ed, MapDown::State(st), 30.0);
        assert!(s.remotes.is_empty() && s.follow.is_none());
    }

    #[test]
    fn our_place_goes_out_when_it_moves_a_few_times_a_second() {
        let (mut s, mut ed) = session();
        assert!(s.presence(&ed, None, 0.0).is_some());
        assert!(s.presence(&ed, None, 500.0).is_none(), "nothing moved");
        ed.time = 1000.0;
        assert!(s.presence(&ed, None, 550.0).is_some());
        ed.time = 1100.0;
        assert!(s.presence(&ed, None, 600.0).is_none(), "too soon");
        assert_eq!(
            s.presence(&ed, Some(1200.0), 650.0),
            Some(MapUp::Presence {
                diff: 0,
                time: 1100.0,
                cursor: Some(1200.0),
                selection: Vec::new(),
            })
        );
    }

    #[test]
    fn edits_refusals_chat_and_the_end_come_through() {
        let (mut s, mut ed) = session();
        let op = MapOp::InsertObjects(vec![Obj {
            id: (2 << 40) | 1,
            line: "256,192,3000,1,0,0:0:0:0:".into(),
            fake: false,
            group: None,
        }]);
        let seq = |seq: u64| {
            MapDown::Op(Sequenced {
                seq,
                author: 2,
                epoch: 2,
                client_seq: seq,
                diff: Some(0),
                op: op.clone(),
            })
        };
        assert_eq!(s.handle(&mut ed, seq(1), 0.0), None);
        assert!(ed.chart.objects.iter().any(|o| o.time == 3000.0));
        // Notes by author: off until asked for.
        assert_eq!(s.author_colour((2 << 40) | 1), None);
        s.by_author = true;
        assert_eq!(s.author_colour((2 << 40) | 1), Some(0x5CC8F2));
        assert_eq!(
            s.handle(&mut ed, seq(3), 0.0),
            Some(Notice::Resume),
            "a gap in the order"
        );
        ed.place_hit(4000.0, false).unwrap();
        s.sync.collect(&mut ed);
        let sent = s.sync.outgoing(0.0, false);
        let MapUp::Op { client_seq, .. } = sent[0] else {
            panic!("an op");
        };
        let why = "only the host changes the timing".to_string();
        assert_eq!(
            s.handle(
                &mut ed,
                MapDown::Refused {
                    client_seq,
                    why: why.clone()
                },
                0.0
            ),
            Some(Notice::Refused(why))
        );
        assert!(!ed.chart.objects.iter().any(|o| o.time == 4000.0));
        for i in 0..250 {
            s.handle(
                &mut ed,
                MapDown::Chat {
                    who: 2,
                    text: format!("{i}"),
                    at: 0,
                },
                0.0,
            );
        }
        assert_eq!(s.chat.len(), 200);
        assert_eq!(s.chat.back(), Some(&(2, "249".to_string())));
        assert_eq!(
            s.handle(&mut ed, MapDown::Closed("bye".into()), 0.0),
            Some(Notice::Closed("bye".into()))
        );
    }
}
