//! A mapping session as the editor's screen sees it: the sync of the charts, who is in it, where
//! the others are (playhead, cursor, selection), the session chat, and the view's own choices
//! (following someone, colouring notes by who made them). It reads the server's `MapDown` lines
//! and makes the `MapUp` lines the game sends.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::Instant;

use taiko_proto::mapping::{
    CREDITS_KEY, MapDown, MapUp, Mode, Participant, Role, SessionState, SnapshotPieces,
    snapshot_lines,
};

use super::super::chart::Chart;
use super::super::state::Editor;
use super::{OutOfStep, Sync};

/// Where the others are is sent at most this often (ms).
pub const PRESENCE_MS: f64 = 100.0;
/// Someone whose presence hasn't changed for this long shows as away (ms).
pub const AWAY_MS: f64 = 60_000.0;
/// Chat lines kept.
const CHAT_KEPT: usize = 200;

/// What the session panel shows below its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    People,
    /// Who maps what: the difficulties (and the open one's parts in a collab).
    Work,
    Chat,
    /// Review comments on the open difficulty.
    Notes,
}

/// A map time as modders write it (`01:23:456`), which chat and comments turn into links.
pub fn stamp(ms: f64) -> String {
    let ms = ms.max(0.0).round() as u64;
    format!("{:02}:{:02}:{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}

/// The first map time written in `text` like `stamp` does (`mm:ss:mmm`, minutes of any length).
pub fn first_stamp(text: &str) -> Option<f64> {
    let b = text.as_bytes();
    let digits = |from: usize, n: usize| {
        (from + n <= b.len() && b[from..from + n].iter().all(u8::is_ascii_digit))
            .then(|| text[from..from + n].parse::<u64>().ok())
            .flatten()
    };
    for start in 0..b.len() {
        if !b[start].is_ascii_digit() || (start > 0 && b[start - 1].is_ascii_digit()) {
            continue;
        }
        let mut end = start;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        let (Some(min), true) = (digits(start, end - start), b.get(end) == Some(&b':')) else {
            continue;
        };
        let (Some(sec), true) = (digits(end + 1, 2), b.get(end + 3) == Some(&b':')) else {
            continue;
        };
        let Some(milli) = digits(end + 4, 3) else {
            continue;
        };
        if sec < 60 && !b.get(end + 7).is_some_and(u8::is_ascii_digit) {
            return Some((min * 60_000 + sec * 1000 + milli) as f64);
        }
    }
    None
}

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

/// What we may edit in the open difficulty, judged as the server judges our edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Rights {
    /// Everything: we host the session.
    All,
    /// The whole difficulty: the host gave it to us.
    Diff,
    /// Anywhere: the host let every mapper map the whole difficulty.
    Free,
    /// Inside these stretches of the song (ms, `from..to`): our parts of a collab difficulty,
    /// and the open ones (anyone's) between the parts the host gave to people.
    Parts(Vec<(f64, f64)>),
    /// Nothing yet: the host hasn't given us a part of this collab difficulty.
    NoPart,
    /// Nothing here: the difficulty is someone else's (or nobody's yet); ours is named, if any.
    NotOurs(Option<String>),
    /// Nothing: the host locked the difficulty.
    Locked,
    /// Nothing: modders and viewers don't edit.
    ReadOnly,
}

impl Rights {
    /// Whether we may place notes anywhere in the open difficulty.
    pub fn edits(&self) -> bool {
        matches!(
            self,
            Rights::All | Rights::Free | Rights::Diff | Rights::Parts(_)
        )
    }
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
    /// Lines to send back (our snapshot, when the server asks for one).
    Send(Vec<MapUp>),
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
    /// What the panel shows.
    pub tab: Tab,
    /// Chat lines heard in all (the kept ones are fewer), for the unread mark.
    pub heard: u64,
    /// The last presence sent and when.
    sent: Option<(f64, Remote)>,
    /// A snapshot coming in parts (after a resume).
    pieces: SnapshotPieces,
    /// The last of our ops the server had taken when we resumed (`Reply::Mapping::done`).
    done: u64,
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
            tab: Tab::People,
            heard: 0,
            sent: None,
            pieces: SnapshotPieces::default(),
            done: 0,
            started: Instant::now(),
        }
    }

    /// The session's clock (ms), for `handle`, `presence` and `away`.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    /// Whether we may take this comment down: our own, or any as the host.
    pub fn may_delete(&self, comment: u64) -> bool {
        self.state
            .comments
            .iter()
            .any(|k| k.id == comment && (k.author == self.me || self.state.host == self.me))
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

    /// What `who` may edit in the open difficulty, as the server's rules have it: the host
    /// everything; a mapper a guest difficulty given to them, their parts of a collab one and
    /// the open parts (nobody's: every mapper's); nobody but the host a locked difficulty;
    /// modders and viewers nothing.
    pub fn rights_of(&self, who: u32) -> Rights {
        if who == self.state.host {
            return Rights::All;
        }
        if self.participant(who).is_none_or(|p| p.role != Role::Mapper) {
            return Rights::ReadOnly;
        }
        let open = self.sync.open();
        let Some(doc) = self.state.documents.iter().find(|d| d.diff == open) else {
            return Rights::NotOurs(None);
        };
        if doc.locked || doc.accepted {
            return Rights::Locked;
        }
        if !doc.parts.is_empty() {
            if doc.parts.iter().all(|p| p.assignee.is_none()) {
                return Rights::Free;
            }
            let mine: Vec<(f64, f64)> = doc
                .parts
                .iter()
                .filter(|p| p.assignee.is_none_or(|a| a == who))
                .map(|p| (p.from, p.to))
                .collect();
            return if mine.is_empty() {
                Rights::NoPart
            } else {
                Rights::Parts(mine)
            };
        }
        if doc.owner == Some(who) {
            return Rights::Diff;
        }
        match self.state.mode {
            Mode::Collab => Rights::NoPart,
            _ => Rights::NotOurs(
                self.state
                    .documents
                    .iter()
                    .find(|d| d.owner == Some(who))
                    .map(|d| d.name.clone()),
            ),
        }
    }

    /// What we may edit in the open difficulty.
    pub fn rights(&self) -> Rights {
        self.rights_of(self.me)
    }

    /// Whether the open difficulty has open parts: every mapper maps there.
    pub fn free(&self) -> bool {
        let open = self.sync.open();
        self.state
            .documents
            .iter()
            .any(|d| d.diff == open && d.parts.iter().any(|p| p.assignee.is_none()))
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
                self.heard += 1;
                self.chat.push_back((who, text));
                while self.chat.len() > CHAT_KEPT {
                    self.chat.pop_front();
                }
            }
            MapDown::Comment(c) => match self.state.comments.iter_mut().find(|k| k.id == c.id) {
                Some(k) => *k = c,
                None => self.state.comments.push(c),
            },
            MapDown::Resolved {
                comment, resolved, ..
            } => {
                if let Some(k) = self.state.comments.iter_mut().find(|k| k.id == comment) {
                    k.resolved = resolved;
                }
            }
            MapDown::CommentDeleted { comment } => {
                self.state.comments.retain(|k| k.id != comment);
            }
            MapDown::Closed(why) => return Some(Notice::Closed(why)),
            // After a resume the server sends its snapshot when its log no longer holds what
            // we missed.
            MapDown::Snapshot {
                seq,
                part,
                parts,
                data,
            } => {
                let (seq, data) = self.pieces.add(seq, part, parts, data)?;
                if self.sync.resync(ed, &data, seq, &[], self.done).is_err() {
                    return Some(Notice::Resume);
                }
            }
            // Asked for our state to shorten the log: only at the seq we are at (the server
            // asks after sending us every op up to it).
            MapDown::SnapshotWanted { seq } if seq == self.sync.last_seq() => {
                let data = self.sync.snapshot().ok()?;
                return Some(Notice::Send(snapshot_lines(seq, &data)));
            }
            // Assets are fetched while joining, before the editor opens.
            MapDown::AssetPart { .. }
            | MapDown::Presence { .. }
            | MapDown::SnapshotWanted { .. }
            | MapDown::Pong => {}
        }
        None
    }

    /// The credits line of a difficulty's chart (`mapping::CREDITS_KEY`): the names of who
    /// made its objects, most first.
    pub fn credits_line(&self, diff: u32, chart: &Chart) -> String {
        self.sync
            .credits(diff, chart, self.state.host)
            .into_iter()
            .filter_map(|(who, _)| self.participant(who).map(|p| p.name.clone()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Puts a difficulty's credits in its chart. Every mapper but the host (the set's
    /// `Creator`) goes in osu!'s own `Tags`, as osu!'s ranking rules ask of guest and collab
    /// mappers, so a plain `.osu` keeps to the osu! format. A `.tko`, our own format, also
    /// keeps the full list, most objects first, in `CREDITS_KEY` for the custom map upload;
    /// a plain `.osu` loses that key if it has one.
    pub fn credit(&self, diff: u32, chart: &mut Chart) {
        let line = self.credits_line(diff, chart);
        if line.is_empty() {
            return;
        }
        if chart.gimmick.is_some() {
            chart.set_key("Metadata", CREDITS_KEY, &line);
        } else if chart.get_key("Metadata", CREDITS_KEY).is_some() {
            chart.remove_key("Metadata", CREDITS_KEY);
        }
        let host = self.participant(self.state.host).map(|p| p.name.as_str());
        let old = chart.get_key("Metadata", "Tags").unwrap_or_default();
        let mut tags: Vec<String> = old.split_whitespace().map(str::to_string).collect();
        for name in line.split(", ") {
            if Some(name) == host {
                continue;
            }
            // A tag is one word: osu! writes a name with spaces with underscores.
            let tag = name.split_whitespace().collect::<Vec<_>>().join("_");
            if !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                tags.push(tag);
            }
        }
        let new = tags.join(" ");
        if new != old.trim() {
            chart.set_key("Metadata", "Tags", &new);
        }
    }

    /// Saving in a session: the open difficulty's chart gets its credits, and every other
    /// difficulty of the session is written to its file in `dir` (found by its name, or a
    /// new `.osu` or `.tko` named like the set's), with its credits. Returns how many others
    /// were written.
    pub fn save_with_credits(&self, ed: &mut Editor) -> Result<usize, String> {
        self.credit(self.sync.open(), &mut ed.chart);
        let dir = ed.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let files = super::connect::charts_in(&dir);
        let mut written = 0;
        for (diff, name, chart) in self.sync.others() {
            let mut chart = chart.clone();
            self.credit(diff, &mut chart);
            let path = self.diff_path(&dir, &files, name, &chart);
            if path == ed.path {
                continue;
            }
            chart.save(&path, true)?;
            written += 1;
        }
        Ok(written)
    }

    /// Where a difficulty of the session (`name`, its chart `chart`) is kept in the set's
    /// folder `dir` (its charts `files`): the file of that name, or a new `.osu` or `.tko` named
    /// like the set's.
    pub fn diff_path(
        &self,
        dir: &Path,
        files: &[(std::path::PathBuf, String)],
        name: &str,
        chart: &Chart,
    ) -> std::path::PathBuf {
        let ext = if chart.gimmick.is_some() {
            "tko"
        } else {
            "osu"
        };
        let named: Vec<&std::path::PathBuf> = files
            .iter()
            .filter(|(_, text)| super::connect::version(text).as_deref() == Some(name))
            .map(|(p, _)| p)
            .collect();
        // A .tko never goes into the .osu it was made from: that stays an osu! map.
        let same_kind = named
            .iter()
            .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)));
        match (same_kind, named.first()) {
            (Some(p), _) => (*p).clone(),
            (None, Some(p)) => p.with_extension(ext),
            (None, None) => {
                let file = format!("{} [{name}].{ext}", self.state.title);
                dir.join(crate::download::safe_folder_name(&file))
            }
        }
    }

    /// The difficulties `who` has work in: a guest difficulty given to them, or parts.
    pub fn work_of(&self, who: u32) -> Vec<u32> {
        self.state
            .documents
            .iter()
            .filter(|d| d.owner == Some(who) || d.parts.iter().any(|p| p.assignee == Some(who)))
            .map(|d| d.diff)
            .collect()
    }

    /// Back on a resumed connection: `done` is the last of our ops the server had taken.
    pub fn resumed(&mut self, ed: &mut Editor, done: u64) {
        self.done = done;
        self.pieces = SnapshotPieces::default();
        self.sync.resumed(ed, done);
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
            folder: String::new(),
            assets: Vec::new(),
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
    fn a_difficulty_made_a_tko_is_saved_beside_its_osu_not_over_it() {
        let (s, ed) = session();
        let dir = PathBuf::from("set");
        let osu = dir.join("Test [Oni].osu");
        let files = vec![(osu.clone(), MAP.to_string())];
        let mut chart = ed.chart.clone();
        assert_eq!(s.diff_path(&dir, &files, "Oni", &chart), osu);
        chart.gimmick = Some(Default::default());
        assert_eq!(
            s.diff_path(&dir, &files, "Oni", &chart),
            dir.join("Test [Oni].tko")
        );
        // Once the .tko is there, that's where it goes; the .osu keeps the plain chart.
        let tko = dir.join("Test [Oni].tko");
        let files = vec![
            (osu.clone(), MAP.to_string()),
            (tko.clone(), MAP.to_string()),
        ];
        assert_eq!(s.diff_path(&dir, &files, "Oni", &chart), tko);
        chart.gimmick = None;
        assert_eq!(s.diff_path(&dir, &files, "Oni", &chart), osu);
    }

    #[test]
    fn map_times_are_written_and_found_like_modders_write_them() {
        assert_eq!(stamp(62_345.4), "01:02:345");
        assert_eq!(stamp(-5.0), "00:00:000");
        assert_eq!(stamp(3_725_007.0), "62:05:007");
        assert_eq!(first_stamp("see 01:02:345 (1,2) - x"), Some(62_345.0));
        assert_eq!(first_stamp("123:00:001"), Some(7_380_001.0));
        assert_eq!(first_stamp("at 1:2:345, 12:61:000, 01:02:3456"), None);
        assert_eq!(first_stamp("10:00 then 00:01:500"), Some(1500.0));
        assert_eq!(first_stamp("nothing"), None);
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
        let note = taiko_proto::mapping::Comment {
            id: 4,
            diff: 0,
            time: 1000.0,
            author: 3,
            text: "00:01:000 (1) - too dense".into(),
            resolved: false,
        };
        s.handle(&mut ed, MapDown::Comment(note.clone()), 0.0);
        s.handle(&mut ed, MapDown::Comment(note), 0.0);
        s.handle(
            &mut ed,
            MapDown::Resolved {
                comment: 4,
                resolved: true,
                by: 1,
            },
            0.0,
        );
        assert_eq!(
            s.state.comments.len(),
            1,
            "the same comment again replaces it"
        );
        assert!(s.state.comments[0].resolved);
        assert_eq!(
            s.handle(&mut ed, MapDown::Closed("bye".into()), 0.0),
            Some(Notice::Closed("bye".into()))
        );
    }

    #[test]
    fn a_resume_drops_what_the_server_took_and_snapshots_come_and_go_at_our_seq() {
        let (mut s, mut ed) = session();
        ed.place_hit(4000.0, false).unwrap();
        ed.place_hit(5000.0, false).unwrap();
        s.sync.collect(&mut ed);
        assert_eq!(s.sync.outgoing(0.0, false).len(), 2);
        // The connection dropped; the server had taken our first op (it comes back with the
        // ops we missed), not the second, which is sent again.
        s.resumed(&mut ed, 1);
        assert_eq!(s.sync.pending(), 1);
        let again = s.sync.outgoing(100.0, false);
        assert!(matches!(again[..], [MapUp::Op { client_seq: 2, .. }]));
        assert!(!ed.chart.objects.iter().any(|o| o.time == 4000.0));
        assert!(ed.chart.objects.iter().any(|o| o.time == 5000.0));
        // The server's log no longer held what we missed: its snapshot, in parts.
        let confirmed = s.sync.snapshot().unwrap();
        let lines = snapshot_lines(7, &confirmed);
        for up in lines {
            let MapUp::Snapshot {
                seq,
                part,
                parts,
                data,
            } = up
            else {
                panic!("a snapshot part");
            };
            let down = MapDown::Snapshot {
                seq,
                part,
                parts,
                data,
            };
            assert_eq!(s.handle(&mut ed, down, 0.0), None);
        }
        assert_eq!(s.sync.last_seq(), 7);
        assert!(
            ed.chart.objects.iter().any(|o| o.time == 5000.0),
            "our pending op stays on top"
        );
        // Asked for a snapshot at the seq we are at: ours goes back; at another, nothing.
        assert_eq!(
            s.handle(&mut ed, MapDown::SnapshotWanted { seq: 6 }, 0.0),
            None
        );
        let Some(Notice::Send(lines)) = s.handle(&mut ed, MapDown::SnapshotWanted { seq: 7 }, 0.0)
        else {
            panic!("our snapshot");
        };
        assert!(matches!(lines[0], MapUp::Snapshot { seq: 7, .. }));
    }

    #[test]
    fn saving_credits_every_difficulty_and_writes_the_others() {
        let (mut s, mut ed) = session();
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-session-credits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ed.path = dir.join("Test [Oni].osu");
        let note = |t: f64, n: u64| {
            MapDown::Op(Sequenced {
                seq: n,
                author: 2,
                epoch: 2,
                client_seq: n,
                diff: Some(0),
                op: MapOp::InsertObjects(
                    (0..6)
                        .map(|i| Obj {
                            id: (2 << 40) | (n * 10 + i),
                            line: format!("256,192,{},1,0,0:0:0:0:", t + i as f64 * 100.0),
                            fake: false,
                            group: None,
                        })
                        .collect(),
                ),
            })
        };
        s.handle(&mut ed, note(12000.0, 1), 0.0);
        assert_eq!(
            s.credits_line(0, &ed.chart),
            "anna, host",
            "six of anna's, five of the set's own"
        );
        // A plain .osu: the mappers in osu!'s Tags (not the host, the set's Creator), and no
        // key osu! doesn't know.
        ed.chart.set_key("Metadata", "Tags", "taiko Anna");
        ed.chart.set_key("Metadata", CREDITS_KEY, "stale");
        assert_eq!(s.save_with_credits(&mut ed).unwrap(), 1);
        ed.save().unwrap();
        let oni = std::fs::read_to_string(&ed.path).unwrap();
        assert!(
            oni.contains("Tags:taiko Anna\r\n"),
            "a name already tagged stays: {oni}"
        );
        assert!(!oni.contains(CREDITS_KEY), "{oni}");
        let other = std::fs::read_to_string(dir.join("Test [Anna's Muzukashii].osu")).unwrap();
        assert!(
            !other.contains(CREDITS_KEY) && !other.contains("Tags:"),
            "the host's own: {other}"
        );
        // A .tko keeps the full list for the upload, and the tags too.
        let mut tko = ed.chart.clone();
        tko.remove_key("Metadata", "Tags");
        tko.gimmick = Some(Default::default());
        s.credit(0, &mut tko);
        assert_eq!(
            tko.get_key("Metadata", CREDITS_KEY).as_deref(),
            Some("anna, host")
        );
        assert_eq!(tko.get_key("Metadata", "Tags").as_deref(), Some("anna"));
        // A name with spaces is one tag.
        s.state.participants[1].name = "anna maria".into();
        let mut plain = ed.chart.clone();
        plain.remove_key("Metadata", "Tags");
        s.credit(0, &mut plain);
        assert_eq!(
            plain.get_key("Metadata", "Tags").as_deref(),
            Some("anna_maria")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tko_difficulty_missing_from_the_folder_is_written_as_a_tko() {
        let (mut s, mut ed) = session();
        let dir = std::env::temp_dir().join(format!("taiko-rs-session-tko-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ed.path = dir.join("Test [Oni].osu");
        s.sync.set_gimmick_for_tests(1, Default::default());
        assert_eq!(s.save_with_credits(&mut ed).unwrap(), 1);
        assert!(dir.join("Test [Anna's Muzukashii].tko").exists());
        assert!(!dir.join("Test [Anna's Muzukashii].osu").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
