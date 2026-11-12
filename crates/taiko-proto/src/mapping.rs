//! Collaborative mapping: the wire format of a live editing session (the spec is the web
//! backend's `docs/collab-mapping.md`). A session edits one map set; each difficulty is a
//! document, and every edit is an op in the shape of the editor's undo steps, with chart data
//! as the lines the `.osu` file holds, so the server can check rights without a chart model.
//!
//! Nothing here is ever sent to a server that didn't list [`FEATURE`] in its
//! `Reply::Account::features`: older servers don't know these messages.

use serde::{Deserialize, Serialize};

/// What a server that runs mapping sessions lists in `Reply::Account::features`.
pub const FEATURE: &str = "mapping/1";

/// Ids a join creates are `(epoch << EPOCH_SHIFT) | n`; snapshot ids are below `1 << EPOCH_SHIFT`.
pub const EPOCH_SHIFT: u32 = 40;

/// The first id a join with `epoch` gives out.
pub fn first_id(epoch: u32) -> u64 {
    (u64::from(epoch) << EPOCH_SHIFT) | 1
}

/// Whether a server's features include mapping sessions.
pub fn supported(features: &[String]) -> bool {
    features.iter().any(|f| f == FEATURE)
}

/// A hit object: its `.osu` line (`x,y,time,type,hitsound,...`), and for a `.tko` fake note
/// its flag and motion group.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Obj {
    pub id: u64,
    pub line: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fake: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u16>,
}

/// A timing point: its `.osu` line (`time,beatLength,meter,set,index,volume,uninherited,fx`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Line {
    pub id: u64,
    pub line: String,
}

/// One edit, as the editor's `history::Command`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum MapOp {
    InsertObjects(Vec<Obj>),
    RemoveObjects(Vec<Obj>),
    EditObjects {
        before: Vec<Obj>,
        after: Vec<Obj>,
    },
    InsertTiming(Vec<Line>),
    RemoveTiming(Vec<Line>),
    EditTiming {
        before: Vec<Line>,
        after: Vec<Line>,
    },
    /// The `.tko` gimmick as JSON, before and after.
    Gimmick {
        before: String,
        after: String,
    },
    SetKey {
        section: String,
        key: String,
        before: Option<String>,
        after: Option<String>,
    },
    Composite(Vec<MapOp>),
}

/// The keys an op may set: song setup's.
pub const KEYS: [(&str, &str); 11] = [
    ("Metadata", "Artist"),
    ("Metadata", "ArtistUnicode"),
    ("Metadata", "Title"),
    ("Metadata", "TitleUnicode"),
    ("Metadata", "Creator"),
    ("Metadata", "Version"),
    ("Metadata", "Source"),
    ("Metadata", "Tags"),
    ("Difficulty", "HPDrainRate"),
    ("Difficulty", "OverallDifficulty"),
    ("Difficulty", "SliderMultiplier"),
];

/// `KEYS`' own `&'static str`s for a section and key, if an op may set them.
pub fn known_key(section: &str, key: &str) -> Option<(&'static str, &'static str)> {
    KEYS.iter()
        .find(|(s, k)| *s == section && *k == key)
        .copied()
}

/// A hit object line's time (field 3).
pub fn object_time(line: &str) -> Option<f64> {
    line.split(',').nth(2)?.trim().parse().ok()
}

/// A timing line's time (field 1).
pub fn timing_time(line: &str) -> Option<f64> {
    line.split(',').next()?.trim().parse().ok()
}

/// Whether a timing line is a red line: its uninherited flag, or before the flag existed a
/// positive beat length.
pub fn timing_is_red(line: &str) -> bool {
    let f: Vec<&str> = line.split(',').map(str::trim).collect();
    match f.get(6).and_then(|v| v.parse::<f64>().ok()) {
        Some(v) => v as i32 == 1,
        None => f
            .get(1)
            .and_then(|v| v.parse::<f64>().ok())
            .is_some_and(|b| b >= 0.0),
    }
}

impl MapOp {
    /// Every hit object it inserts, removes or edits, before and after.
    pub fn objects(&self) -> Vec<&Obj> {
        let mut out = Vec::new();
        self.walk(&mut |op| match op {
            MapOp::InsertObjects(l) | MapOp::RemoveObjects(l) => out.extend(l.iter()),
            MapOp::EditObjects { before, after } => {
                out.extend(before.iter());
                out.extend(after.iter());
            }
            _ => {}
        });
        out
    }

    /// Every timing line it inserts, removes or edits, before and after.
    pub fn timing(&self) -> Vec<&Line> {
        let mut out = Vec::new();
        self.walk(&mut |op| match op {
            MapOp::InsertTiming(l) | MapOp::RemoveTiming(l) => out.extend(l.iter()),
            MapOp::EditTiming { before, after } => {
                out.extend(before.iter());
                out.extend(after.iter());
            }
            _ => {}
        });
        out
    }

    /// Every op in it (itself, or the parts of a composite, at any depth).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a MapOp)) {
        match self {
            MapOp::Composite(l) => l.iter().for_each(|op| op.walk(f)),
            op => f(op),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            MapOp::InsertObjects(l) | MapOp::RemoveObjects(l) => l.is_empty(),
            MapOp::EditObjects { after, .. } => after.is_empty(),
            MapOp::InsertTiming(l) | MapOp::RemoveTiming(l) => l.is_empty(),
            MapOp::EditTiming { after, .. } => after.is_empty(),
            MapOp::Gimmick { .. } | MapOp::SetKey { .. } => false,
            MapOp::Composite(l) => l.iter().all(MapOp::is_empty),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Several mappers share difficulties, by parts.
    Collab,
    /// Guests own whole difficulties.
    Guest,
    /// Modders read, comment and chat.
    Review,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    /// Edits what they are given: parts of difficulties, or whole ones as a guest.
    Mapper,
    /// Comments and chats.
    Modder,
    Viewer,
}

/// A time range `[from, to)` of a difficulty, given to a participant or left open (`None`:
/// only the host edits it).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Part {
    pub from: f64,
    pub to: f64,
    #[serde(default)]
    pub assignee: Option<u32>,
}

impl Part {
    pub fn contains(&self, t: f64) -> bool {
        t >= self.from && t < self.to
    }
}

/// A difficulty of the set in the session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Document {
    pub diff: u32,
    /// Its difficulty name (`Version`).
    pub name: String,
    /// A guest difficulty's owner.
    #[serde(default)]
    pub owner: Option<u32>,
    /// Parts of a collab difficulty; none: the whole difficulty is its owner's (or the host's).
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Locked: only the host edits it. Accepted: the host took the guest's work as final.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub accepted: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Participant {
    pub id: u32,
    pub name: String,
    pub role: Role,
    /// 0xRRGGBB.
    pub colour: u32,
    pub online: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Comment {
    pub id: u64,
    pub diff: u32,
    pub time: f64,
    pub author: u32,
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
}

/// Everything about a session but its charts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SessionState {
    pub session: u64,
    pub title: String,
    pub mode: Mode,
    pub host: u32,
    pub participants: Vec<Participant>,
    pub documents: Vec<Document>,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

/// Why `who` may not make `op` on difficulty `diff` (`None`: the set's shared red lines).
pub fn refusal(state: &SessionState, who: u32, diff: Option<u32>, op: &MapOp) -> Option<String> {
    if who == state.host {
        return None;
    }
    let Some(me) = state.participants.iter().find(|p| p.id == who) else {
        return Some("you aren't in this session".into());
    };
    if me.role != Role::Mapper {
        return Some("you can't edit in this session".into());
    }
    let Some(diff) = diff else {
        return Some("only the host changes the timing".into());
    };
    let Some(doc) = state.documents.iter().find(|d| d.diff == diff) else {
        return Some("no such difficulty".into());
    };
    if doc.locked || doc.accepted {
        return Some(format!("{} is locked", doc.name));
    }
    if op.timing().iter().any(|l| timing_is_red(&l.line)) {
        return Some("only the host changes the timing".into());
    }
    let mut refused = None;
    op.walk(&mut |o| {
        if refused.is_some() {
            return;
        }
        match o {
            MapOp::SetKey { section, key, .. }
                if section != "Difficulty" && !(section == "Metadata" && key == "Version") =>
            {
                refused = Some("only the host changes the song's metadata".to_string());
            }
            MapOp::SetKey { .. } | MapOp::Gimmick { .. } if !doc.parts.is_empty() => {
                refused = Some("only the host changes this in a collab difficulty".to_string());
            }
            _ => {}
        }
    });
    if refused.is_some() {
        return refused;
    }
    if doc.parts.is_empty() {
        return (doc.owner != Some(who)).then(|| format!("{} isn't yours", doc.name));
    }
    let mine = |t: f64| {
        doc.parts
            .iter()
            .any(|p| p.assignee == Some(who) && p.contains(t))
    };
    let times = op
        .objects()
        .into_iter()
        .map(|o| object_time(&o.line))
        .chain(op.timing().into_iter().map(|l| timing_time(&l.line)));
    for t in times {
        match t {
            Some(t) if mine(t) => {}
            Some(t) => {
                let whose = doc
                    .parts
                    .iter()
                    .find(|p| p.contains(t))
                    .and_then(|p| p.assignee)
                    .and_then(|a| state.participants.iter().find(|p| p.id == a))
                    .map_or_else(|| "the host's".to_string(), |p| format!("{}'s", p.name));
                return Some(format!("that's in {whose} part"));
            }
            None => return Some("unreadable line".into()),
        }
    }
    None
}

/// One difficulty in a snapshot: the file's text and the ids of its timing points and hit
/// objects in the order the game's parser reads them, and who made each object.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DocSnapshot {
    pub diff: u32,
    pub name: String,
    pub text: String,
    pub timing_ids: Vec<u64>,
    pub object_ids: Vec<u64>,
    #[serde(default)]
    pub authors: Vec<(u64, u32)>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SnapshotData {
    pub docs: Vec<DocSnapshot>,
}

/// An op as the server orders it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sequenced {
    pub seq: u64,
    pub author: u32,
    pub epoch: u32,
    pub client_seq: u64,
    /// The difficulty; `None` for the set's shared red lines (applied to every difficulty).
    pub diff: Option<u32>,
    pub op: MapOp,
}

/// Game -> server on a mapping stream.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum MapUp {
    Op {
        client_seq: u64,
        diff: Option<u32>,
        op: MapOp,
    },
    /// The confirmed state at `seq` (asked by `MapDown::SnapshotWanted`, or the first one),
    /// the JSON of `SnapshotData` in pieces.
    Snapshot {
        seq: u64,
        part: u32,
        parts: u32,
        data: String,
    },
    Presence {
        diff: u32,
        time: f64,
        cursor: Option<f64>,
        selection: Vec<u64>,
    },
    Chat(String),
    Ping,
}

/// Server -> game on a mapping stream.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum MapDown {
    State(SessionState),
    Snapshot {
        seq: u64,
        part: u32,
        parts: u32,
        data: String,
    },
    Op(Sequenced),
    Refused {
        client_seq: u64,
        why: String,
    },
    Presence {
        who: u32,
        diff: u32,
        time: f64,
        cursor: Option<f64>,
        selection: Vec<u64>,
    },
    Chat {
        who: u32,
        text: String,
        at: u64,
    },
    SnapshotWanted {
        seq: u64,
    },
    Closed(String),
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(parts: Vec<Part>, owner: Option<u32>) -> SessionState {
        SessionState {
            session: 1,
            title: "t".into(),
            mode: Mode::Collab,
            host: 1,
            participants: vec![
                Participant {
                    id: 1,
                    name: "host".into(),
                    role: Role::Host,
                    colour: 0,
                    online: true,
                },
                Participant {
                    id: 2,
                    name: "anna".into(),
                    role: Role::Mapper,
                    colour: 0,
                    online: true,
                },
                Participant {
                    id: 3,
                    name: "bob".into(),
                    role: Role::Modder,
                    colour: 0,
                    online: true,
                },
            ],
            documents: vec![Document {
                diff: 0,
                name: "Oni".into(),
                owner,
                parts,
                locked: false,
                accepted: false,
            }],
            comments: Vec::new(),
        }
    }

    fn note(t: f64) -> Obj {
        Obj {
            id: 7,
            line: format!("256,192,{t},1,0,0:0:0:0:"),
            fake: false,
            group: None,
        }
    }

    #[test]
    fn a_mapper_edits_only_inside_their_parts() {
        let s = state(
            vec![
                Part {
                    from: 0.0,
                    to: 1000.0,
                    assignee: Some(2),
                },
                Part {
                    from: 1000.0,
                    to: 2000.0,
                    assignee: None,
                },
            ],
            None,
        );
        let inside = MapOp::InsertObjects(vec![note(999.0)]);
        assert_eq!(refusal(&s, 2, Some(0), &inside), None);
        let at_end = MapOp::InsertObjects(vec![note(1000.0)]);
        assert_eq!(
            refusal(&s, 2, Some(0), &at_end).as_deref(),
            Some("that's in the host's part")
        );
        // Moving a note out of the part is refused too: its time after counts.
        let out = MapOp::EditObjects {
            before: vec![note(500.0)],
            after: vec![note(1500.0)],
        };
        assert!(refusal(&s, 2, Some(0), &out).is_some());
        assert_eq!(refusal(&s, 1, Some(0), &out), None, "the host edits all");
        assert!(
            refusal(&s, 3, Some(0), &inside).is_some(),
            "a modder edits nothing"
        );
    }

    #[test]
    fn a_guest_owns_their_difficulty_but_not_timing_or_metadata() {
        let mut s = state(Vec::new(), Some(2));
        let n = MapOp::InsertObjects(vec![note(5.0)]);
        assert_eq!(refusal(&s, 2, Some(0), &n), None);
        let red = MapOp::InsertTiming(vec![Line {
            id: 1,
            line: "0,500,4,1,0,100,1,0".into(),
        }]);
        let green = MapOp::InsertTiming(vec![Line {
            id: 1,
            line: "0,-50,4,1,0,100,0,0".into(),
        }]);
        assert!(refusal(&s, 2, Some(0), &red).is_some());
        assert_eq!(refusal(&s, 2, Some(0), &green), None);
        assert!(refusal(&s, 2, None, &green).is_some());
        let title = MapOp::SetKey {
            section: "Metadata".into(),
            key: "Title".into(),
            before: None,
            after: Some("x".into()),
        };
        let name = MapOp::SetKey {
            section: "Metadata".into(),
            key: "Version".into(),
            before: None,
            after: Some("Anna's Oni".into()),
        };
        assert!(refusal(&s, 2, Some(0), &title).is_some());
        assert_eq!(refusal(&s, 2, Some(0), &name), None);
        s.documents[0].accepted = true;
        assert!(
            refusal(&s, 2, Some(0), &n).is_some(),
            "accepted: no more edits"
        );
        s.documents[0].owner = Some(3);
        s.documents[0].accepted = false;
        assert!(refusal(&s, 2, Some(0), &n).is_some(), "not theirs");
    }

    #[test]
    fn red_lines_old_and_new() {
        assert!(timing_is_red("0,500,4,1,0,100,1,0"));
        assert!(!timing_is_red("0,-100,4,1,0,100,0,0"));
        assert!(timing_is_red("0,500"));
        assert!(!timing_is_red("0,-100"));
        assert_eq!(object_time("256,192,1234.5,1,0"), Some(1234.5));
        assert_eq!(timing_time("77,500"), Some(77.0));
    }
}
