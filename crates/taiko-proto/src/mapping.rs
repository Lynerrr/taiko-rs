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

/// A file of the set other than its charts (audio, background, video, storyboard images).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AssetRef {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// The set a host opens a session for.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SetDescriptor {
    pub title: String,
    /// Its folder's name (the set is found by it again on resuming).
    pub folder: String,
    /// The difficulties' names.
    pub diffs: Vec<String>,
    pub assets: Vec<AssetRef>,
}

/// What only the host does in a session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum HostCmd {
    /// Invites a friend by name.
    Invite(String),
    Kick(u32),
    Role {
        who: u32,
        role: Role,
    },
    /// Gives a difficulty to a guest, or takes it back (`None`).
    AssignDiff {
        diff: u32,
        owner: Option<u32>,
    },
    Lock {
        diff: u32,
        locked: bool,
        accepted: bool,
    },
    Parts {
        diff: u32,
        parts: Vec<Part>,
    },
    AddDiff {
        name: String,
    },
    Close,
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
    /// the JSON of `SnapshotData` in parts (`snapshot_lines`).
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
    /// A review comment pinned to a time of a difficulty, and resolving one.
    Comment {
        diff: u32,
        time: f64,
        text: String,
    },
    Resolve {
        comment: u64,
        resolved: bool,
    },
    Host(HostCmd),
    /// Host: a piece of an asset the server needs (base64 `data`, at most `ASSET_PIECE` bytes
    /// before encoding).
    Asset {
        sha256: String,
        offset: u64,
        total: u64,
        data: String,
    },
    /// Asks for an asset from `offset` on (`MapDown::AssetPart`).
    GetAsset {
        sha256: String,
        offset: u64,
    },
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
    Comment(Comment),
    Resolved {
        comment: u64,
        resolved: bool,
        by: u32,
    },
    AssetPart {
        sha256: String,
        offset: u64,
        total: u64,
        data: String,
    },
    Closed(String),
    Pong,
}

/// Raw bytes of an asset per piece, and characters of snapshot JSON per part: both keep a line
/// well under the protocol's 1 MiB.
pub const ASSET_PIECE: usize = 512 * 1024;
pub const SNAPSHOT_PART: usize = 512 * 1024;

/// A snapshot as the lines that carry it.
pub fn snapshot_lines(seq: u64, data: &SnapshotData) -> Vec<MapUp> {
    let json = serde_json::to_string(data).unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    let mut rest = json.as_str();
    while !rest.is_empty() || parts.is_empty() {
        let mut cut = rest.len().min(SNAPSHOT_PART);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        parts.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    let n = parts.len() as u32;
    parts
        .into_iter()
        .enumerate()
        .map(|(i, data)| MapUp::Snapshot {
            seq,
            part: i as u32,
            parts: n,
            data,
        })
        .collect()
}

/// Puts a snapshot back together from its parts as they come.
#[derive(Default)]
pub struct SnapshotPieces {
    seq: u64,
    parts: Vec<Option<String>>,
}

impl SnapshotPieces {
    /// Takes a part; the whole snapshot (at its seq) once every part is in. A part of another
    /// snapshot starts over.
    pub fn add(
        &mut self,
        seq: u64,
        part: u32,
        parts: u32,
        data: String,
    ) -> Option<(u64, SnapshotData)> {
        if seq != self.seq || self.parts.len() != parts as usize {
            self.seq = seq;
            self.parts = vec![None; parts as usize];
        }
        *self.parts.get_mut(part as usize)? = Some(data);
        if self.parts.iter().any(Option::is_none) {
            return None;
        }
        let json: String = self.parts.drain(..).flatten().collect();
        serde_json::from_str(&json).ok().map(|d| (seq, d))
    }
}

/// What a mapping connection reports to the game.
#[derive(Debug)]
pub enum LinkEvent {
    /// In the session (`Reply::Mapping`).
    Joined {
        session: u64,
        epoch: u32,
        me: u32,
        needs: Vec<String>,
        done: u64,
    },
    Down(Box<MapDown>),
    /// The connection couldn't be made or was lost: why. The game resumes with
    /// `Request::ResumeMapping` when it wants to go on.
    Lost(String),
}

/// The game's connection to a mapping session: one request, then the mapping stream, read and
/// written on threads of its own. Dropping it disconnects.
pub struct Link {
    tx: std::sync::mpsc::Sender<MapUp>,
    rx: std::sync::mpsc::Receiver<LinkEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Link {
    /// Connects with `request` (`OpenMapping`, `JoinMapping` or `ResumeMapping`).
    pub fn start(server: String, request: crate::online::Request) -> Link {
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let (tx, out) = mpsc::channel::<MapUp>();
        let (events, rx) = mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let lost = |events: &mpsc::Sender<LinkEvent>, why: String| {
                let _ = events.send(LinkEvent::Lost(why));
            };
            let (reply, reader) = match crate::online::exchange(&server, request) {
                Ok(r) => r,
                Err(e) => return lost(&events, e),
            };
            match reply {
                crate::online::Reply::Mapping {
                    session,
                    epoch,
                    me,
                    needs,
                    done,
                } => {
                    let _ = events.send(LinkEvent::Joined {
                        session,
                        epoch,
                        me,
                        needs,
                        done,
                    });
                }
                crate::online::Reply::Error(e) => return lost(&events, e),
                _ => return lost(&events, "the server didn't open the session".into()),
            }
            let _ = reader
                .get_ref()
                .set_read_timeout(Some(Duration::from_secs(70)));
            let _ = reader.get_ref().set_write_timeout(None);
            let Ok(mut stream) = reader.get_ref().try_clone() else {
                return lost(&events, "the connection dropped".into());
            };
            let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
            let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            {
                let (events, alive) = (events.clone(), alive.clone());
                std::thread::spawn(move || {
                    let mut reader = reader;
                    while let Ok(line) = crate::online::read_line(&mut reader) {
                        // A line this build doesn't know is passed over.
                        if let Ok(d) = serde_json::from_str::<MapDown>(&line)
                            && events.send(LinkEvent::Down(Box::new(d))).is_err()
                        {
                            break;
                        }
                    }
                    alive.store(false, Ordering::Relaxed);
                });
            }
            let mut last_ping = Instant::now();
            loop {
                if stopped.load(Ordering::Relaxed) {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    return;
                }
                if !alive.load(Ordering::Relaxed) {
                    break;
                }
                if last_ping.elapsed() > Duration::from_secs(25) {
                    last_ping = Instant::now();
                    if crate::online::write_line(&mut stream, &MapUp::Ping).is_err() {
                        break;
                    }
                }
                let up = match out.recv_timeout(Duration::from_millis(100)) {
                    Ok(up) => up,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if crate::online::write_line(&mut stream, &up).is_err() {
                    break;
                }
            }
            let _ = stream.shutdown(std::net::Shutdown::Both);
            lost(&events, "the connection dropped".into());
        });
        Link { tx, rx, stop }
    }

    pub fn send(&self, up: MapUp) {
        let _ = self.tx.send(up);
    }

    pub fn poll(&self) -> Vec<LinkEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
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
    fn a_snapshot_goes_in_parts_and_comes_back_whole() {
        let data = SnapshotData {
            docs: vec![DocSnapshot {
                diff: 0,
                name: "Oni".into(),
                text: "ü".repeat(SNAPSHOT_PART),
                timing_ids: vec![1],
                object_ids: vec![2, 3],
                authors: vec![(2, 1)],
            }],
        };
        let lines = snapshot_lines(7, &data);
        assert_eq!(lines.len(), 3, "two bytes a letter: just over two parts");
        let mut pieces = SnapshotPieces::default();
        // A part of an older snapshot is dropped by a newer one starting.
        assert!(pieces.add(6, 0, 2, "{".into()).is_none());
        let mut whole = None;
        for l in lines.into_iter().rev() {
            let MapUp::Snapshot {
                seq,
                part,
                parts,
                data,
            } = l
            else {
                unreachable!()
            };
            assert!(data.len() <= SNAPSHOT_PART);
            whole = pieces.add(seq, part, parts, data);
        }
        assert_eq!(whole, Some((7, data)));
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
