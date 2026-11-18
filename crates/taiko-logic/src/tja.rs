//! `.tja` charts: Taiko Jiro's text format, as TJAPlayer3 and OpenTaiko read it.
//!
//! A file holds one song and a chart per course (Easy, Normal, Hard, Oni, Edit). Each course is
//! read straight into a `Beatmap` (no `.osu` text in between), timed the way OpenTaiko's default
//! mode times it: every division of a bar lasts `4 beats × MEASURE / divisions` at the BPM it
//! starts at, added up in `f64` with nothing rounded, so hundreds of `#BPMCHANGE`s don't drift.
//! Commands take effect where they stand in the bar.
//!
//! A branching chart has every branch's notes, each tagged with its branch (`crate::branch`).
//!
//! How notes are drawn moving beyond their own velocity (`#BMSCROLL` / `#HBSCROLL`, `#SUDDEN`,
//! a negative `#SCROLL`) goes in the map's gimmick (`Gimmick::motion` and its shared scroll),
//! never in its board chart: it changes how a chart looks, not how it plays or scores.

use std::collections::HashSet;

use crate::beatmap::{Barline, Beatmap, ControlPoint, Kind, Meta, Note};
use crate::tko::gimmick::{Gimmick, Motion, NoteMotion};
use crate::tko::scroll::MAX_SPEED;
use crate::tko::track::{Ease, Key, Track};

mod board;
pub use board::{BoardChart, MAX_ITEMS};

/// The osu! slider multiplier a TJA chart scrolls like: one beat at `#SCROLL 1` covers the
/// distance an osu! taiko map with SliderMultiplier 1.4 covers.
pub const SLIDER_MULTIPLIER: f64 = 1.4;
/// osu!'s `LEGACY_TAIKO_VELOCITY_MULTIPLIER`, as `beatmap` applies it.
const TAIKO_VELOCITY_MULTIPLIER: f64 = 1.4;
/// Pops a balloon needs when the chart's `BALLOON:` list runs out.
pub const DEFAULT_BALLOON_HITS: u32 = 5;
/// The slowest scroll a note gets: `#SCROLL 0` (notes standing still) would never arrive.
const MIN_SCROLL: f64 = 0.01;

/// A chart's difficulty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CourseKind {
    // The values go into `BoardChart::hash` and over the wire: never renumber them.
    Easy = 0,
    Normal = 1,
    Hard = 2,
    Oni = 3,
    /// Ura Oni.
    Edit = 4,
}

impl CourseKind {
    /// `COURSE:` as written: the name (any case) or its number 0-4.
    pub fn parse(value: &str) -> Option<CourseKind> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "easy" | "0" => CourseKind::Easy,
            "normal" | "1" => CourseKind::Normal,
            "hard" | "2" => CourseKind::Hard,
            "oni" | "3" => CourseKind::Oni,
            "edit" | "ura" | "4" => CourseKind::Edit,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            CourseKind::Easy => "Easy",
            CourseKind::Normal => "Normal",
            CourseKind::Hard => "Hard",
            CourseKind::Oni => "Oni",
            CourseKind::Edit => "Ura Oni",
        }
    }

    /// One word for it, what `parse` reads back: in a path (`song.tja#oni`).
    pub fn slug(self) -> &'static str {
        match self {
            CourseKind::Easy => "easy",
            CourseKind::Normal => "normal",
            CourseKind::Hard => "hard",
            CourseKind::Oni => "oni",
            CourseKind::Edit => "edit",
        }
    }

    /// Judged with the arcade's narrow windows (Hard and up) rather than the wide ones (Easy,
    /// Normal).
    pub fn narrow_windows(self) -> bool {
        matches!(self, CourseKind::Hard | CourseKind::Oni | CourseKind::Edit)
    }

    fn id(self) -> u8 {
        self as u8
    }
}

/// A `.tja` file read.
#[derive(Clone, Debug)]
pub struct Song {
    pub title: String,
    /// `SUBTITLE:` without its `--`/`++` display prefix: usually the artist or the source.
    pub subtitle: String,
    /// The audio file, relative to the chart.
    pub wave: String,
    pub genre: String,
    pub maker: String,
    /// One chart per course found, easiest first.
    pub courses: Vec<Course>,
    /// What was skipped or guessed, once each.
    pub warnings: Vec<String>,
}

/// One course's chart.
#[derive(Clone, Debug)]
pub struct Course {
    pub kind: CourseKind,
    /// `LEVEL:`, the stars the chart shows.
    pub level: u32,
    pub map: Beatmap,
    /// Identifies the chart however the file was written (`BoardChart::hash`).
    pub hash: ChartHash,
}

/// SHA-256 of a normalized chart: the same for the same notes whatever the file's encoding,
/// comments, spacing or header order, and different as soon as a note moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartHash(pub [u8; 32]);

impl ChartHash {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The first 8 bytes, for what keys maps by a `u64`.
    pub fn short(&self) -> u64 {
        u64::from_be_bytes(self.0[..8].try_into().expect("8 bytes"))
    }
}

/// A chart file's text: UTF-8 (with or without a BOM), UTF-16 with a BOM, else Shift-JIS, which
/// most Japanese charts are in.
pub fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling(&bytes[bom..])
            .0
            .into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => encoding_rs::SHIFT_JIS
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

/// A chart file's bytes, read.
pub fn parse_bytes(bytes: &[u8]) -> Result<Song, String> {
    parse(&decode(bytes))
}

/// Header fields as they stand when a chart starts.
#[derive(Clone, Debug)]
struct Header {
    title: String,
    subtitle: String,
    wave: String,
    genre: String,
    maker: String,
    /// `NOTESDESIGNERn`: who charted each course (by `CourseKind`'s number), from the
    /// header before the courses.
    designers: [String; 5],
    /// Who charted this course: its own `NOTESDESIGNER`, else the one named before it.
    designer: String,
    /// Whether a `COURSE:` has been read: `NOTESDESIGNER` after one is that course's.
    in_course: bool,
    bpm: Option<f64>,
    offset: f64,
    demo_start: f64,
    course: CourseKind,
    /// `LEVEL:`; 0 when it has none. OpenTaiko writes some with a fraction (`9.5`).
    level: f32,
    /// `COMPAT:` TaikoJiro's or TaikoManyGimmicks': a complex `#SCROLL`'s imaginary part points
    /// up, and `#DIRECTION` sets the way whole. OpenTaiko's own (and TJAPlayer3's), the default:
    /// it points down, and a `#DIRECTION` leaves a complex `#SCROLL`'s vertical part be.
    jiro_axes: bool,
    balloon: Vec<u32>,
    /// `BALLOONNOR` / `BALLOONEXP` / `BALLOONMAS`: per-branch lists that win over `BALLOON`.
    branch_balloon: [Option<Vec<u32>>; 3],
}

impl Default for Header {
    fn default() -> Self {
        Header {
            title: String::new(),
            subtitle: String::new(),
            wave: String::new(),
            genre: String::new(),
            maker: String::new(),
            designers: Default::default(),
            designer: String::new(),
            in_course: false,
            bpm: None,
            offset: 0.0,
            demo_start: 0.0,
            // TJAPlayer3 reads a chart with no COURSE as Oni.
            course: CourseKind::Oni,
            level: 0.0,
            jiro_axes: false,
            balloon: Vec::new(),
            branch_balloon: [None, None, None],
        }
    }
}

/// Warnings, each said once.
#[derive(Default)]
struct Warnings {
    seen: HashSet<String>,
    list: Vec<String>,
}

impl Warnings {
    fn add(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.seen.insert(text.clone()) {
            self.list.push(text);
        }
    }
}

/// A chart between `#START` and `#END`.
struct RawChart<'a> {
    header: Header,
    /// 0: single player, 1: P1, 2: P2 (`STYLE: Double`).
    player: u8,
    body: Vec<&'a str>,
}

/// A `.tja` file's text, read.
pub fn parse(text: &str) -> Result<Song, String> {
    let mut warnings = Warnings::default();
    let mut header = Header::default();
    let mut charts: Vec<RawChart> = Vec::new();
    let mut open: Option<RawChart> = None;

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(chart) = open.as_mut() {
            if let Some((name, _)) = command(line)
                && name == "END"
            {
                charts.extend(open.take());
            } else if line.starts_with('#') || !is_header_like(line) {
                chart.body.push(line);
            } else {
                warnings.add(format!("header line inside a chart: {line}: ignored"));
            }
            continue;
        }
        if let Some((name, arg)) = command(line) {
            if name == "START" {
                let player = match arg.to_ascii_uppercase().as_str() {
                    "P1" | "1P" => 1,
                    "P2" | "2P" => 2,
                    _ => 0,
                };
                open = Some(RawChart {
                    header: header.clone(),
                    player,
                    body: Vec::new(),
                });
            }
            // Anything else outside a chart (#BMSCROLL and the like) is a mode this reader
            // doesn't have: noted where the chart reads its commands.
            continue;
        }
        if let Some((key, value)) = header_line(line) {
            read_header(&mut header, &key, value, &mut warnings);
        }
    }
    if let Some(chart) = open {
        warnings.add("a chart has no #END: read to the end of the file");
        charts.push(chart);
    }

    let mut picked: Vec<RawChart> = Vec::new();
    for chart in charts {
        if chart.player == 2 {
            continue;
        }
        match picked
            .iter_mut()
            .find(|c| c.header.course == chart.header.course)
        {
            // A single-player chart wins over P1 of a STYLE: Double one.
            Some(c) if c.player == 1 && chart.player == 0 => *c = chart,
            Some(_) => warnings.add(format!(
                "more than one {} chart: the first one is played",
                chart.header.course.name()
            )),
            None => picked.push(chart),
        }
    }
    picked.sort_by_key(|c| c.header.course);

    let mut courses = Vec::new();
    for chart in &picked {
        let map = read_course(chart, &mut warnings);
        if map.notes.is_empty() {
            warnings.add(format!(
                "the {} chart has no notes",
                chart.header.course.name()
            ));
            continue;
        }
        // Played as its board chart reads back, so a server playing a replay on the chart
        // it was sent plays the very chart this game did.
        let board = BoardChart::of(chart.header.course, &map);
        let hash = board.hash();
        let map = match board.beatmap() {
            Ok(played) => Beatmap {
                meta: map.meta,
                gimmick: map.gimmick,
                ..played
            },
            Err(why) => {
                warnings.add(format!(
                    "the {} chart can't go on a leaderboard: {why}",
                    chart.header.course.name()
                ));
                map
            }
        };
        courses.push(Course {
            kind: chart.header.course,
            level: chart.header.level as u32,
            map,
            hash,
        });
    }
    if courses.is_empty() {
        return Err("no playable chart (#START ... #END with notes)".into());
    }
    let first = &picked[0].header;
    Ok(Song {
        title: first.title.clone(),
        subtitle: first.subtitle.clone(),
        wave: first.wave.clone(),
        genre: first.genre.clone(),
        maker: first.maker.clone(),
        courses,
        warnings: warnings.list,
    })
}

/// A `KEY:value` line rather than notes, told apart as OpenTaiko does: notes start with a digit
/// or have no colon.
fn is_header_like(line: &str) -> bool {
    !line.starts_with(|c: char| c.is_ascii_digit()) && line.contains(':')
}

fn strip_comment(line: &str) -> &str {
    line.find("//").map_or(line, |i| &line[..i])
}

/// `#NAME arg`: the name in capitals (letters only, so `#BPMCHANGE180` reads too) and the rest.
fn command(line: &str) -> Option<(String, &str)> {
    let rest = line.strip_prefix('#')?;
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    (end > 0).then(|| (rest[..end].to_ascii_uppercase(), rest[end..].trim()))
}

/// `KEY:value`, the key in capitals.
fn header_line(line: &str) -> Option<(String, &str)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| (key.to_ascii_uppercase(), value.trim()))
}

fn read_header(h: &mut Header, key: &str, value: &str, warnings: &mut Warnings) {
    let number = |w: &mut Warnings| match value.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => {
            if !value.is_empty() {
                w.add(format!("{key}:{value} is not a number: ignored"));
            }
            None
        }
    };
    match key {
        "TITLE" => h.title = value.to_string(),
        "SUBTITLE" => {
            h.subtitle = value
                .strip_prefix("--")
                .or_else(|| value.strip_prefix("++"))
                .unwrap_or(value)
                .trim()
                .to_string()
        }
        "WAVE" => h.wave = value.to_string(),
        "GENRE" => h.genre = value.to_string(),
        "MAKER" => h.maker = value.to_string(),
        "BPM" => {
            if let Some(v) = number(warnings).filter(|v| *v > 0.0) {
                h.bpm = Some(v);
            }
        }
        "OFFSET" => h.offset = number(warnings).unwrap_or(h.offset),
        "DEMOSTART" => h.demo_start = number(warnings).unwrap_or(h.demo_start),
        "COURSE" => match CourseKind::parse(value) {
            Some(kind) => {
                h.course = kind;
                h.level = 0.0;
                h.designer = h.designers[kind as usize].clone();
                h.in_course = true;
                h.balloon.clear();
                h.branch_balloon = [None, None, None];
            }
            None => warnings.add(format!("COURSE:{value} is not played: read as Oni")),
        },
        "COMPAT" => match value.to_ascii_lowercase().as_str() {
            "jiro1" | "jiro2" | "tmg" => h.jiro_axes = true,
            "oos" | "tjap3" => h.jiro_axes = false,
            _ => warnings.add(format!("COMPAT:{value} is not known: OpenTaiko's used")),
        },
        "LEVEL" => {
            h.level = value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(0.0)
        }
        "BALLOON" => h.balloon = balloon_list(value),
        "BALLOONNOR" => h.branch_balloon[0] = Some(balloon_list(value)),
        "BALLOONEXP" => h.branch_balloon[1] = Some(balloon_list(value)),
        "BALLOONMAS" => h.branch_balloon[2] = Some(balloon_list(value)),
        // As OpenTaiko reads it: before the courses, with a number, the charter of that course;
        // without one, of every course; within a course, of that one, whatever number it has.
        key if key.starts_with("NOTESDESIGNER") => {
            let which = key["NOTESDESIGNER".len()..].parse::<usize>().ok();
            match which {
                _ if h.in_course => h.designer = value.to_string(),
                Some(n) if n < h.designers.len() => h.designers[n] = value.to_string(),
                Some(_) => warnings.add(format!("{key}: no such course, ignored")),
                None => {
                    h.designers = std::array::from_fn(|_| value.to_string());
                    h.designer = value.to_string();
                }
            }
        }
        _ => {}
    }
}

fn balloon_list(value: &str) -> Vec<u32> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<f64>()
                .map_or(DEFAULT_BALLOON_HITS, |n| n.max(1.0) as u32)
        })
        .collect()
}

/// Where a line of a chart is in its branches: None for one every branch shares, else the
/// branch point (by number) it comes after and its branch.
type Tag = Option<(u16, u8)>;

/// A line of a chart once the branch to play is chosen.
#[derive(Debug)]
enum Item<'a> {
    Command(String, &'a str, Tag),
    /// Note data, with the pops of each balloon head on it in order (None for a head that
    /// starts nothing).
    Data(&'a str, Vec<Option<u32>>, Tag),
}

/// Branch letters: Normal, Expert, Master.
const BRANCHES: [&str; 3] = ["N", "E", "M"];

/// Whether a symbol is a balloon head (and, when it starts one, takes the next `BALLOON:`
/// value). D, OpenTaiko's fuse roll, is a balloon there and takes one too.
fn is_balloon(c: char) -> bool {
    matches!(c, '7' | '9' | 'D')
}

/// Whether a symbol starts a roll or balloon.
fn is_roll_head(c: char) -> bool {
    matches!(c, '5' | '6' | '7' | '9' | 'D' | 'H' | 'I')
}

/// Whether a roll or balloon is open in each branch, to tell which balloon heads start a
/// balloon: a head while one is open is a blank (a kusudama written `9009` is one), as in
/// OpenTaiko, and takes no `BALLOON:` value.
#[derive(Default)]
struct Rolls([bool; 3]);

impl Rolls {
    /// For each balloon head on `line` in `branches`, the branches it starts a balloon in.
    fn balloons(&mut self, line: &str, branches: &[usize]) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        for c in line.chars() {
            if c == '8' {
                branches.iter().for_each(|&b| self.0[b] = false);
            } else if is_roll_head(c) {
                let mut started = Vec::new();
                for &b in branches {
                    if !self.0[b] {
                        self.0[b] = true;
                        started.push(b);
                    }
                }
                if is_balloon(c) {
                    out.push(started);
                }
            }
        }
        out
    }
}

/// The branches lines in `branch` play in: None is a line they all share.
fn affected(branch: Option<usize>) -> Vec<usize> {
    branch.map_or_else(|| vec![0, 1, 2], |b| vec![b])
}

/// Whether the chart branches.
fn branches(chart: &RawChart) -> bool {
    chart
        .body
        .iter()
        .any(|line| command(line).is_some_and(|(name, _)| name == "BRANCHSTART"))
}

/// The chart's lines as branch `pass` plays them (a branch section without it plays the nearest
/// one it has, the lower first), each balloon given its pops and each line its `Tag`.
/// `#BRANCHSTART`s stay in, for `build` to time the branch points.
fn pick_branch<'a>(chart: &RawChart<'a>, pass: usize, warnings: &mut Warnings) -> Vec<Item<'a>> {
    // The branches each #BRANCHSTART section has, and the balloons each branch plays (shared
    // lines count in all three).
    let mut sections: Vec<[bool; 3]> = Vec::new();
    let mut per_branch = [0usize; 3];
    let mut branch: Option<usize> = None;
    let mut rolls = Rolls::default();
    for line in &chart.body {
        match command(line).map(|(name, _)| name).as_deref() {
            Some("BRANCHSTART") => {
                sections.push([false; 3]);
                branch = None;
            }
            Some("BRANCHEND") => branch = None,
            Some(b @ ("N" | "E" | "M")) => {
                let i = BRANCHES.iter().position(|x| *x == b).expect("branch");
                if let Some(s) = sections.last_mut() {
                    s[i] = true;
                }
                branch = Some(i);
            }
            Some(_) => {}
            None => {
                for started in rolls.balloons(line, &affected(branch)) {
                    started.iter().for_each(|&b| per_branch[b] += 1);
                }
            }
        }
    }
    // A BALLOON: list as long as each branch's balloons is one list every branch reads from
    // its start (charters often write it so); otherwise it runs through the file once, shared
    // lines once and each branch's own in turn (tja2fumen reads both ways the same).
    let shared_list =
        !sections.is_empty() && per_branch.iter().all(|&n| n == chart.header.balloon.len());
    let chosen_in = |s: &[bool; 3]| {
        (0..3)
            .flat_map(|d| [pass.checked_sub(d), Some(pass + d).filter(|&b| b < 3)])
            .flatten()
            .find(|&b| s[b])
    };

    let h = &chart.header;
    let mut out = Vec::new();
    let mut section: Option<usize> = None;
    // A line in a branch is the pass's, whichever branch's lines stood in for it.
    let tag = |branch: Option<usize>, section: Option<usize>| match (branch, section) {
        (Some(_), Some(s)) => Some((s.min(u16::MAX as usize) as u16, pass as u8)),
        _ => None,
    };
    // Which branch the current lines belong to: None for lines every branch shares.
    branch = None;
    let mut chosen: Option<usize> = None;
    // The branch played: the one chosen in the last section (the pass's own before any).
    let mut played = pass;
    // Balloons seen so far: in the whole chart (for BALLOON:) and per branch (BALLOONxxx:).
    let mut seen_all = 0usize;
    let mut seen_branch = [0usize; 3];
    let mut rolls = Rolls::default();
    for line in &chart.body {
        if let Some((name, arg)) = command(line) {
            match name.as_str() {
                "BRANCHSTART" => {
                    let i = section.map_or(0, |i| i + 1);
                    section = Some(i);
                    chosen = sections.get(i).and_then(chosen_in);
                    played = chosen.unwrap_or(played);
                    branch = None;
                    out.push(Item::Command(name, arg, None));
                    continue;
                }
                "BRANCHEND" => {
                    branch = None;
                    chosen = None;
                    continue;
                }
                "N" | "E" | "M" => {
                    branch = BRANCHES.iter().position(|x| *x == name);
                    continue;
                }
                _ => {}
            }
            if branch.is_none() || branch == chosen {
                out.push(Item::Command(name, arg, tag(branch, section)));
            }
            continue;
        }
        let kept = branch.is_none() || branch == chosen;
        let mut pops = Vec::new();
        for started in rolls.balloons(line, &affected(branch)) {
            if !started.is_empty() {
                seen_all += 1;
            }
            started.iter().for_each(|&b| seen_branch[b] += 1);
            if !kept {
                continue;
            }
            if !started.contains(&played) {
                pops.push(None);
                continue;
            }
            // The list of the branch played when it has one, else the common BALLOON: list.
            let hits = match (&h.branch_balloon[played], sections.is_empty()) {
                (Some(list), false) => list.get(seen_branch[played] - 1).copied(),
                _ if shared_list => h.balloon.get(seen_branch[played] - 1).copied(),
                _ => h.balloon.get(seen_all - 1).copied(),
            };
            pops.push(Some(hits.unwrap_or_else(|| {
                warnings.add(format!(
                    "BALLOON: has too few values: {DEFAULT_BALLOON_HITS} pops used"
                ));
                DEFAULT_BALLOON_HITS
            })));
        }
        if kept {
            out.push(Item::Data(line, pops, tag(branch, section)));
        }
    }
    out
}

/// Scroll velocity (osu!px per ms) at `bpm` and `#SCROLL scroll`: TJA notes move with both.
pub fn velocity(bpm: f64, scroll: f64) -> f64 {
    // A negative BPM's notes are as fast as a positive one's (they only come the other way).
    SLIDER_MULTIPLIER * TAIKO_VELOCITY_MULTIPLIER * 100.0 * scroll.abs().max(MIN_SCROLL) * bpm.abs()
        / 60_000.0
}

/// Roll tick spacing at `bpm`: 1/4 beat, twice as dense at 125 BPM or slower and half above 250,
/// as osu! taiko rolls (`beatmap::convert`).
fn tick_spacing(bpm: f64) -> f64 {
    let rate = if bpm <= 125.0 {
        8.0
    } else if bpm > 250.0 {
        2.0
    } else {
        4.0
    };
    60_000.0 / bpm / rate
}

/// `#SCROLL` as written: a real number, or OpenTaiko's complex `a+bi`, whose real part is the
/// horizontal speed.
fn scroll_value(arg: &str) -> Option<f64> {
    let arg = arg.trim();
    if !arg.contains('i') {
        return arg.parse().ok();
    }
    let body = arg.trim_end_matches('i');
    match body.rfind(['+', '-']).filter(|&i| i > 0) {
        Some(i) => body[..i].trim().parse().ok(),
        // Only an imaginary part: no horizontal speed.
        None => Some(0.0),
    }
}

/// A number as OpenTaiko writes positions: real (`5`), imaginary (`3i`, `-i`) or both
/// (`5-3i`), as (real, imaginary).
fn complex_value(arg: &str) -> Option<(f64, f64)> {
    let arg = arg.trim();
    let Some(body) = arg.strip_suffix(['i', 'I']) else {
        return arg
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| (v, 0.0));
    };
    let finite = |v: f64| v.is_finite().then_some(v);
    let part = |s: &str| match s.trim() {
        "" | "+" => Some(1.0),
        "-" => Some(-1.0),
        s => s.parse::<f64>().ok().and_then(finite),
    };
    // Where the imaginary part starts: its sign, not one of an exponent.
    let split = body
        .char_indices()
        .rev()
        .filter(|&(i, c)| i > 0 && matches!(c, '+' | '-') && !body[..i].ends_with(['e', 'E']))
        .map(|(i, _)| i)
        .next();
    match split {
        Some(i) => Some((
            body[..i].trim().parse::<f64>().ok().and_then(finite)?,
            part(&body[i..])?,
        )),
        None => Some((0.0, part(body)?)),
    }
}

/// Where `#JPOSSCROLL`s take the hit target over the song, right and down. As OpenTaiko plays
/// them: in time order, each from where the last left it, and one cut short by the next goes only
/// as far as it got.
fn judge_tracks(moves: &mut [JudgeMove]) -> (Track, Track) {
    moves.sort_by(|a, b| a.t.total_cmp(&b.t));
    let (mut tx, mut ty) = (Track::default(), Track::default());
    let (mut x, mut y) = (0.0, 0.0);
    for i in 0..moves.len() {
        let m = moves[i];
        let dt = match moves.get(i + 1) {
            Some(next) if m.dt > 0.0 && next.t - m.t < m.dt => (next.t - m.t).max(0.0),
            _ => m.dt,
        };
        let rate = if m.dt > 0.0 { dt / m.dt } else { 1.0 };
        let (to_x, to_y) = (x + m.dx * rate, y + m.dy * rate);
        for (track, from, to) in [(&mut tx, x, to_x), (&mut ty, y, to_y)] {
            track.set(Key {
                t: m.t,
                v: from,
                ease: Ease::Linear,
            });
            track.set(Key {
                t: m.t + dt,
                v: to,
                ease: Ease::Step,
            });
        }
        (x, y) = (to_x, to_y);
    }
    // A way it never moves needs no track.
    let still = |t: &Track| t.keys.iter().all(|k| k.v == 0.0);
    if still(&tx) {
        tx = Track::default();
    }
    if still(&ty) {
        ty = Track::default();
    }
    (tx, ty)
}

/// A roll or balloon whose end (`8`) is still to come.
struct OpenRoll {
    time: f64,
    kind: Kind,
    big: bool,
    velocity: f64,
    tick_spacing: f64,
    hits: u32,
    /// How it is drawn moving: as its head says.
    motion: NoteMotion,
    tag: Tag,
}

/// Where the chart is while it's read.
#[derive(Clone, Copy)]
struct Cursor {
    time: f64,
    bpm: f64,
    /// `#SCROLL`'s real part: the horizontal speed, what the board chart knows.
    scroll: f64,
    /// Its imaginary part (`#SCROLL 1+2i`): drawn only.
    scroll_im: f64,
    /// `#DIRECTION`: 0 the usual way, 1-7 the others, as OpenTaiko numbers them.
    direction: i64,
    /// The chart's `COMPAT` (see `Header::jiro_axes`).
    jiro_axes: bool,
    /// Bar length in whole notes (`#MEASURE 3/4` = 0.75).
    measure: f64,
    gogo: bool,
    barlines: bool,
    beat_origin: f64,
    mode: ScrollMode,
    /// `#SUDDEN`: how long before its hit a note shows up and starts moving (ms; infinite: all
    /// along).
    show: f64,
    wait: f64,
    /// Rolls and balloons aren't drawn: TJAPlayer3 hides them when only the first `#SUDDEN`
    /// number is set, and OpenTaiko kept that.
    hide_long: bool,
    /// The branch the notes read now are in.
    tag: Tag,
}

/// How notes scroll, as OpenTaiko places them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollMode {
    /// By time: each note at its own BPM × `#SCROLL`, fixed (osu! SV).
    Normal,
    /// `#BMSCROLL`: by beats, the whole lane at the BPM of the moment (a change speeds up or
    /// slows every note at once, and none overtakes another); `#SCROLL` is left out, and a
    /// `#DELAY` stops the lane.
    Beats,
    /// `#HBSCROLL`: by beats like `#BMSCROLL`, with each note's `#SCROLL` on top.
    BeatsScrolled,
}

impl Cursor {
    fn velocity(&self) -> f64 {
        velocity(self.bpm, self.scroll)
    }

    /// The velocity a note here is drawn with (below 0 from the left), `reference` the BPM a
    /// beat-scrolled lane goes at speed 1 at.
    fn drawn_velocity(&self, reference: f64) -> f64 {
        let side = if self.scroll < 0.0 { -1.0 } else { 1.0 };
        match self.mode {
            // A negative BPM turns them round too, as OpenTaiko places notes at BPM × #SCROLL.
            ScrollMode::Normal => side * self.bpm.signum() * self.velocity(),
            ScrollMode::Beats => velocity(reference, 1.0),
            ScrollMode::BeatsScrolled => side * velocity(reference, self.scroll),
        }
    }

    /// How fast a lane scrolled by beats moves here: at the reference BPM, 1.
    fn lane_speed(&self, reference: f64) -> f64 {
        match self.mode {
            ScrollMode::Normal => 1.0,
            _ => (self.bpm / reference).clamp(-MAX_SPEED, MAX_SPEED),
        }
    }

    fn control_point(&self) -> ControlPoint {
        ControlPoint {
            time: self.time,
            beat_origin: self.beat_origin,
            // A negative BPM's beats are as long: it only turns time (and notes) round.
            beat_length: 60_000.0 / self.bpm.abs(),
            sv: self.scroll.abs().max(MIN_SCROLL),
            kiai: self.gogo,
            volume: 1.0,
        }
    }

    /// Where a note here sits for each unit it has to go, right and down from the drum, as
    /// OpenTaiko places it (`CTja::ApplyNoteXYDirection`): `#SCROLL` as a complex number (left
    /// out by `#BMSCROLL`), turned by `#DIRECTION`.
    fn way(&self) -> (f64, f64) {
        let (re, im) = match self.mode {
            ScrollMode::Beats => (1.0, 0.0),
            ScrollMode::BeatsScrolled => (self.scroll, self.scroll_im),
            // Placed at BPM × #SCROLL: a negative BPM turns the way round.
            ScrollMode::Normal => {
                let s = self.bpm.signum();
                (s * self.scroll, s * self.scroll_im)
            }
        };
        let y = if self.jiro_axes { -im } else { im };
        let (x, turned) = match self.direction {
            1 => (0.0, -re),
            2 => (0.0, re),
            3 => (re, -re),
            4 => (re, re),
            5 => (-re, 0.0),
            6 => (-re, -re),
            7 => (-re, re),
            _ => (re, y),
        };
        // OpenTaiko's own charts keep a complex #SCROLL's vertical part through a #DIRECTION.
        let y = if !self.jiro_axes && y != 0.0 {
            y
        } else {
            turned
        };
        (x, y)
    }

    /// How fast a note here goes for a scroll of `k` (its size, either way).
    fn speed(&self, k: f64, reference: f64) -> f64 {
        match self.mode {
            ScrollMode::Normal => velocity(self.bpm, k),
            ScrollMode::Beats => velocity(reference, 1.0) * k,
            ScrollMode::BeatsScrolled => velocity(reference, k),
        }
    }

    fn motion(&self, t: f64, reference: f64) -> NoteMotion {
        let (x, y) = self.way();
        let (velocity, dir) = if self.direction == 0 && y == 0.0 {
            (self.drawn_velocity(reference), None)
        } else if y == 0.0 || x.hypot(y) == 0.0 {
            let side = if x < 0.0 { -1.0 } else { 1.0 };
            (side * self.speed(x.abs(), reference), None)
        } else {
            let k = x.hypot(y);
            (self.speed(k, reference), Some([x / k, y / k]))
        };
        NoteMotion {
            t,
            velocity,
            show: self.show,
            wait: self.wait,
            hide_long: self.hide_long,
            dir,
        }
    }
}

/// How a chart's notes are drawn moving, gathered while it's read (see `Gimmick::motion`).
struct Drawing {
    /// The song's BPM: a lane scrolled by beats moves at speed 1 at it, as fast as a note at
    /// `#SCROLL 1` does by time.
    reference: f64,
    /// The lane's speed over the song (the shared scroll) for the beat-scrolled modes; 1 while
    /// the chart scrolls by time.
    speed: Track,
    /// A beat-scrolled mode was on somewhere.
    by_beats: bool,
    notes: Vec<NoteMotion>,
    barlines: Vec<(f64, f64)>,
    /// Barlines coming in some other way than along the lane (see `Motion::barline_dirs`).
    barline_dirs: Vec<(f64, [f64; 2])>,
    /// `#JPOSSCROLL`s as written, in virtual units.
    moves: Vec<JudgeMove>,
    /// `#LYRIC` lines.
    lyrics: Vec<(f64, String)>,
}

/// One `#JPOSSCROLL`: from time `t`, the hit target moves (`dx`, `dy`) over `dt` ms, at an
/// even pace.
#[derive(Clone, Copy, Debug)]
struct JudgeMove {
    t: f64,
    dt: f64,
    dx: f64,
    dy: f64,
}

/// OpenTaiko's pixels (of its 1280-wide screen) in this game's virtual units (853⅓ across).
const JUDGE_PX: f64 = 2.0 / 3.0;

impl Drawing {
    /// The lane's speed from here on.
    fn key(&mut self, cur: &Cursor) {
        self.key_at(cur.time, cur.lane_speed(self.reference));
    }

    fn key_at(&mut self, t: f64, v: f64) {
        self.speed.set(Key {
            t,
            v,
            ease: Ease::Step,
        });
    }

    /// What the map's gimmick gets: None when every note is drawn as its board chart says.
    fn gimmick(mut self) -> Option<Gimmick> {
        let plain = |n: &NoteMotion| {
            n.velocity >= 0.0 && n.show.is_infinite() && n.wait.is_infinite() && n.dir.is_none()
        };
        if !self.by_beats
            && self.notes.iter().all(plain)
            && self.barlines.iter().all(|b| b.1 >= 0.0)
            && self.barline_dirs.is_empty()
            && self.moves.is_empty()
            && self.lyrics.is_empty()
        {
            return None;
        }
        let (judge_x, judge_y) = judge_tracks(&mut self.moves);
        self.lyrics.sort_by(|a, b| a.0.total_cmp(&b.0));
        // A negative #DELAY can set later notes before earlier ones.
        self.notes.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.barlines.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.barline_dirs.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Gimmick {
            scroll: if self.by_beats {
                self.speed
            } else {
                Track::default()
            },
            motion: Motion {
                map_clock: true,
                notes: self.notes,
                barlines: self.barlines,
                barline_dirs: self.barline_dirs,
                judge_x,
                judge_y,
                lyrics: self.lyrics,
            },
            ..Default::default()
        })
    }
}

/// A `#BRANCHSTART` as one pass met it.
struct RawPoint {
    /// Where it is (map ms).
    start: f64,
    /// The bar line before it (OpenTaiko picks the branch there, a bar ahead).
    bar: f64,
    /// The end of the last roll after that bar line, if any: roll conditions wait for it.
    roll_end: Option<f64>,
    /// A roll was still going at it: roll conditions are judged right at it.
    roll_open: bool,
    arg: String,
}

/// A chart read as one branch plays it (`build`).
struct Built {
    /// No gimmick yet: `drawing` makes it, once every pass's notes are in.
    map: Beatmap,
    /// Each note's `Tag`.
    tags: Vec<Tag>,
    /// Its notes' motions in the same order as the notes.
    drawing: Drawing,
    points: Vec<RawPoint>,
    /// `#SECTION`s and `#LEVELHOLD`s outside the branches (map ms).
    resets: Vec<f64>,
    holds: Vec<f64>,
    /// The branch points whose branch (this pass's) has a `#LEVELHOLD`.
    level_holds: Vec<usize>,
}

impl Built {
    /// The map, its gimmick made.
    fn finish(self) -> Beatmap {
        Beatmap {
            gimmick: self.drawing.gimmick().map(std::sync::Arc::new),
            ..self.map
        }
    }
}

/// A course's chart: read once, or once per branch and merged when it branches.
fn read_course(chart: &RawChart, warnings: &mut Warnings) -> Beatmap {
    if !branches(chart) {
        return build(chart, 2, warnings).finish();
    }
    let passes: Vec<Built> = (0..3).map(|pass| build(chart, pass, warnings)).collect();
    merge(passes, warnings)
}

/// One map from the three passes of a branching chart: Master's (its shared notes, timing and
/// bar lines) and the other two's own notes, each tagged, with the branch points.
fn merge(mut passes: Vec<Built>, warnings: &mut Warnings) -> Beatmap {
    let count = passes[2].points.len();
    let mut points = Vec::with_capacity(count);
    let mut last_judge = f64::NEG_INFINITY;
    for i in 0..count {
        let master = &passes[2].points[i];
        let raw: Vec<&RawPoint> = passes
            .iter()
            .map(|p| p.points.get(i).unwrap_or(master))
            .collect();
        let start = master.start;
        let condition = crate::branch::Condition::parse(&master.arg).unwrap_or_else(|| {
            warnings.add(format!(
                "#BRANCHSTART {}: not a condition this game reads: the branch stays",
                master.arg
            ));
            crate::branch::Condition::KEEP
        });
        let bar = raw.iter().map(|p| p.bar).fold(f64::NEG_INFINITY, f64::max);
        let rolls = matches!(
            condition.measure,
            crate::branch::Measure::Rolls | crate::branch::Measure::BalloonHits
        );
        let judge = if rolls && raw.iter().any(|p| p.roll_open) {
            start
        } else if rolls {
            raw.iter().filter_map(|p| p.roll_end).fold(bar, f64::max)
        } else {
            bar
        };
        let judge = judge.max(last_judge).min(start);
        last_judge = judge;
        points.push(crate::branch::Point {
            judge,
            start,
            condition,
            hold: std::array::from_fn(|b| passes[b].level_holds.contains(&i)),
        });
    }
    if passes.iter().any(|p| p.points.len() != count) {
        warnings.add("branches: the branches don't agree on the branch points");
    }

    // Master's notes, then the other branches' own, in time order (a branch's before a later
    // one's on the same ms).
    let master = passes.pop().expect("three passes");
    let mut rows: Vec<(Note, Tag, NoteMotion)> = Vec::new();
    for p in passes {
        rows.extend(
            (p.map.notes.into_iter().zip(p.tags).zip(p.drawing.notes))
                .filter(|((_, tag), _)| tag.is_some())
                .map(|((n, tag), m)| (n, tag, m)),
        );
    }
    let Built {
        mut map,
        tags,
        mut drawing,
        resets,
        holds,
        ..
    } = master;
    rows.extend(
        (std::mem::take(&mut map.notes).into_iter())
            .zip(tags)
            .zip(std::mem::take(&mut drawing.notes))
            .map(|((n, tag), m)| (n, tag, m)),
    );
    rows.sort_by(|a, b| {
        a.0.time
            .total_cmp(&b.0.time)
            .then(a.1.map(|t| t.1).cmp(&b.1.map(|t| t.1)))
    });
    let mut notes = Vec::with_capacity(rows.len());
    let mut tags = Vec::with_capacity(rows.len());
    for (n, tag, m) in rows {
        notes.push(n);
        tags.push(tag);
        drawing.notes.push(m);
    }
    Beatmap {
        notes,
        gimmick: drawing.gimmick().map(std::sync::Arc::new),
        branching: Some(std::sync::Arc::new(crate::branch::Branching {
            notes: tags,
            points,
            resets,
            holds,
        })),
        ..map
    }
}

fn build(chart: &RawChart, pass: usize, warnings: &mut Warnings) -> Built {
    let h = &chart.header;
    let bpm = h.bpm.unwrap_or_else(|| {
        warnings.add("no BPM: 120 used");
        120.0
    });
    let start = -h.offset * 1000.0;
    let mut cur = Cursor {
        time: start,
        bpm,
        scroll: 1.0,
        scroll_im: 0.0,
        direction: 0,
        jiro_axes: h.jiro_axes,
        measure: 1.0,
        gogo: false,
        barlines: true,
        beat_origin: start,
        mode: ScrollMode::Normal,
        show: f64::INFINITY,
        wait: f64::INFINITY,
        hide_long: false,
        tag: None,
    };
    let mut notes: Vec<(Note, Tag)> = Vec::new();
    let mut raw_points: Vec<RawPoint> = Vec::new();
    let (mut resets, mut holds, mut level_holds) = (Vec::new(), Vec::new(), Vec::new());
    // The last bar line (shown or not) and roll end, for the branch points.
    let mut last_bar: Option<f64> = None;
    let mut last_roll_end = f64::NEG_INFINITY;
    let mut barlines: Vec<Barline> = Vec::new();
    let mut points: Vec<ControlPoint> = vec![cur.control_point()];
    let mut roll: Option<OpenRoll> = None;
    let mut drawing = Drawing {
        reference: bpm,
        speed: Track::default(),
        by_beats: false,
        notes: Vec::new(),
        barlines: Vec::new(),
        barline_dirs: Vec::new(),
        moves: Vec::new(),
        lyrics: Vec::new(),
    };
    drawing.key(&cur);

    // A bar: commands and note symbols in order, each symbol with its balloon pops if any.
    enum Step<'a> {
        Command(&'a str, &'a str, Tag),
        Symbol(char, Option<u32>, Tag),
    }
    let items = pick_branch(chart, pass, warnings);
    let mut bar: Vec<Step> = Vec::new();
    let mut bars: Vec<Vec<Step>> = Vec::new();
    for item in &items {
        match item {
            Item::Command(name, arg, tag) => bar.push(Step::Command(name, arg, *tag)),
            Item::Data(line, pops, tag) => {
                let mut pops = pops.iter().copied();
                for c in line.chars() {
                    if c == ',' {
                        bars.push(std::mem::take(&mut bar));
                    } else if c.is_ascii_alphanumeric() {
                        let p = if is_balloon(c) {
                            pops.next().flatten()
                        } else {
                            None
                        };
                        bar.push(Step::Symbol(c, p, *tag));
                    } else if !c.is_whitespace() {
                        warnings.add(format!("unknown symbol {c:?} in the notes: skipped"));
                    }
                }
            }
        }
    }
    if bar.iter().any(|s| matches!(s, Step::Symbol(..))) {
        warnings.add("the last bar has no closing comma: played as a bar");
        bars.push(bar);
    }

    for bar in &bars {
        let divisions = bar.iter().filter(|s| matches!(s, Step::Symbol(..))).count();
        cur.beat_origin = cur.time;
        points.push(cur.control_point());
        let mut barline_due = true;
        let put_barline = |cur: &Cursor, barlines: &mut Vec<Barline>, drawing: &mut Drawing| {
            if cur.barlines {
                barlines.push(Barline {
                    time: cur.time,
                    velocity: cur.velocity(),
                });
                let m = cur.motion(cur.time, drawing.reference);
                drawing.barlines.push((cur.time, m.velocity));
                if let Some(dir) = m.dir {
                    drawing.barline_dirs.push((cur.time, dir));
                }
            }
        };
        for step in bar {
            match *step {
                Step::Command("BRANCHSTART", arg, _) => raw_points.push(RawPoint {
                    start: cur.time,
                    // None before the first bar: OpenTaiko judges 4 beats before the chart.
                    bar: last_bar.unwrap_or(start - 240_000.0 / bpm),
                    roll_end: last_bar
                        .is_none_or(|b| last_roll_end >= b)
                        .then_some(last_roll_end)
                        .filter(|t| t.is_finite()),
                    roll_open: roll.is_some(),
                    arg: arg.to_string(),
                }),
                Step::Command("SECTION", _, _) => resets.push(cur.time),
                Step::Command("LEVELHOLD", _, tag) => match tag {
                    Some((point, _)) => level_holds.push(usize::from(point)),
                    None => holds.push(cur.time),
                },
                Step::Command(name, arg, _) => {
                    apply(name, arg, &mut cur, &mut points, &mut drawing, warnings);
                }
                Step::Symbol(c, pops, tag) => {
                    if barline_due {
                        put_barline(&cur, &mut barlines, &mut drawing);
                        last_bar = Some(cur.time);
                        barline_due = false;
                    }
                    cur.tag = tag;
                    let open = roll.is_some();
                    note(c, pops, &cur, &mut roll, &mut notes, &mut drawing, warnings);
                    if open && roll.is_none() {
                        last_roll_end = cur.time;
                    }
                    cur.time += 240_000.0 / cur.bpm * cur.measure / divisions as f64;
                }
            }
        }
        if barline_due {
            put_barline(&cur, &mut barlines, &mut drawing);
            last_bar = Some(cur.time);
        }
        if divisions == 0 {
            cur.time += 240_000.0 / cur.bpm * cur.measure;
        }
    }
    if roll.is_some() {
        warnings.add("a roll or balloon has no end (8): dropped");
    }

    // A negative #DELAY can set later notes before earlier ones. Their motions go along.
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| notes[a].0.time.total_cmp(&notes[b].0.time));
    let motions = std::mem::take(&mut drawing.notes);
    drawing.notes = order.iter().map(|&i| motions[i]).collect();
    let (notes, tags): (Vec<Note>, Vec<Tag>) = order.iter().map(|&i| notes[i].clone()).unzip();
    barlines.sort_by(|a, b| a.time.total_cmp(&b.time));
    points.sort_by(|a, b| a.time.total_cmp(&b.time));
    points.dedup_by(|later, earlier| {
        let same = later.time == earlier.time;
        if same {
            *earlier = *later;
        }
        same
    });

    let meta = Meta {
        title: h.title.clone(),
        artist: h.subtitle.clone(),
        creator: if h.designer.is_empty() {
            h.maker.clone()
        } else {
            h.designer.clone()
        },
        version: h.course.name().to_string(),
        level: (h.level > 0.0).then_some(h.level),
        audio_file: h.wave.clone(),
        preview_time: h.demo_start * 1000.0,
        genres: crate::genres::of(&h.genre),
        tags: h.genre.clone(),
        ..board::meta(h.course)
    };
    Built {
        map: Beatmap {
            meta,
            notes,
            barlines,
            control_points: points,
            first_velocity: velocity(bpm, 1.0),
            gimmick: None,
            fakes: Vec::new(),
            branching: None,
        },
        tags,
        drawing,
        points: raw_points,
        resets,
        holds,
        level_holds,
    }
}

fn apply(
    name: &str,
    arg: &str,
    cur: &mut Cursor,
    points: &mut Vec<ControlPoint>,
    drawing: &mut Drawing,
    warnings: &mut Warnings,
) {
    let number = |w: &mut Warnings| match arg.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => {
            w.add(format!("#{name} {arg}: not a number, ignored"));
            None
        }
    };
    match name {
        "BPMCHANGE" => match number(warnings) {
            // Below 0 (with a #MEASURE below 0, often) time runs back while it lasts, as in
            // OpenTaiko, and notes come the other way.
            Some(bpm) if bpm != 0.0 => {
                cur.bpm = bpm;
                cur.beat_origin = cur.time;
                points.push(cur.control_point());
                drawing.key(cur);
            }
            Some(_) => warnings.add(format!("#BPMCHANGE {arg}: 0 BPM never ends, ignored")),
            None => {}
        },
        "MEASURE" => {
            let parsed = arg.split_once('/').and_then(|(a, b)| {
                Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
            });
            match parsed {
                // A bar below 0 runs time back, as in OpenTaiko (a gimmick puts it with a BPM
                // below 0, which runs it forward again).
                Some((a, b)) if a != 0.0 && (a / b).is_finite() => cur.measure = a / b,
                _ => warnings.add(format!("#MEASURE {arg}: not a time signature, ignored")),
            }
        }
        "SCROLL" => match scroll_value(arg) {
            Some(v) if v.is_finite() => {
                cur.scroll = v;
                cur.scroll_im = complex_value(arg).map_or(0.0, |c| c.1);
                points.push(cur.control_point());
            }
            _ => warnings.add(format!("#SCROLL {arg}: not a number, ignored")),
        },
        "DELAY" => {
            if let Some(s) = number(warnings) {
                // Scrolled by beats, the lane stops for it.
                if cur.mode != ScrollMode::Normal && s > 0.0 {
                    drawing.key_at(cur.time, 0.0);
                }
                cur.time += s * 1000.0;
                cur.beat_origin = cur.time;
                points.push(cur.control_point());
                drawing.key(cur);
            }
        }
        "DIRECTION" => match arg.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => cur.direction = v as i64,
            _ => warnings.add(format!("#DIRECTION {arg}: not a number, ignored")),
        },
        "NMSCROLL" | "BMSCROLL" | "HBSCROLL" => {
            cur.mode = match name {
                "BMSCROLL" => ScrollMode::Beats,
                "HBSCROLL" => ScrollMode::BeatsScrolled,
                _ => ScrollMode::Normal,
            };
            drawing.by_beats |= cur.mode != ScrollMode::Normal;
            drawing.key(cur);
        }
        "JPOSSCROLL" => {
            let mut parts = arg.split_whitespace();
            let dt = parts
                .next()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite());
            let d = parts.next().and_then(complex_value);
            // Direction 0 (the default) moves it the way notes go, the other way from what is
            // written.
            let side = match parts.next().map(str::parse::<i64>) {
                Some(Ok(0)) | None => -1.0,
                Some(Ok(_)) => 1.0,
                Some(Err(_)) => {
                    warnings.add(format!("#JPOSSCROLL {arg}: not a direction, ignored"));
                    return;
                }
            };
            match (dt, d) {
                (Some(dt), Some((dx, dy))) => drawing.moves.push(JudgeMove {
                    t: cur.time,
                    dt: (dt * 1000.0).max(0.0),
                    dx: side * dx * JUDGE_PX,
                    dy: side * dy * JUDGE_PX,
                }),
                _ => warnings.add(format!(
                    "#JPOSSCROLL {arg}: not a time and a distance, ignored"
                )),
            }
        }
        "LYRIC" => drawing.lyrics.push((cur.time, arg.to_string())),
        "SUDDEN" => {
            let mut parts = arg
                .split_whitespace()
                // `inf` too, as OpenTaiko reads it: all along.
                .map(|v| v.parse::<f64>().ok().filter(|v| !v.is_nan()));
            match (parts.next().flatten(), parts.next().flatten()) {
                (Some(show), Some(wait)) => {
                    // Under a millisecond is off, as OpenTaiko reads it.
                    let set = |s: f64| (s * 1000.0).trunc().abs() >= 1.0;
                    let ms = |s: f64| {
                        if set(s) && s.is_finite() {
                            s * 1000.0
                        } else {
                            f64::INFINITY
                        }
                    };
                    cur.show = ms(show);
                    cur.wait = ms(wait);
                    cur.hide_long = set(show) && !set(wait);
                }
                _ => warnings.add(format!("#SUDDEN {arg}: not two numbers, ignored")),
            }
        }
        "GOGOSTART" | "GOGOEND" => {
            cur.gogo = name == "GOGOSTART";
            points.push(cur.control_point());
        }
        "BARLINEON" | "BARLINEOFF" => cur.barlines = name == "BARLINEON",
        // Branch bookkeeping (pick_branch has chosen) and score rules: nothing to time.
        "SECTION" | "LEVELHOLD" => {}
        _ => warnings.add(format!("#{name} is not supported: ignored")),
    }
}

fn note(
    c: char,
    pops: Option<u32>,
    cur: &Cursor,
    roll: &mut Option<OpenRoll>,
    notes: &mut Vec<(Note, Tag)>,
    drawing: &mut Drawing,
    warnings: &mut Warnings,
) {
    let before = notes.len();
    let hit = |kind: Kind, big: bool| {
        let note = Note {
            time: cur.time,
            end_time: cur.time,
            kind,
            big,
            velocity: cur.velocity(),
            tick_spacing: 0.0,
            required_hits: 0,
            fake_len: 0.0,
            fake_fades: false,
            fake_length: 0.0,
            tiny: false,
            unhittable: false,
        };
        (note, cur.tag)
    };
    let start = |kind: Kind, big: bool, hits: u32, roll: &mut Option<OpenRoll>| {
        // A second head before the end is a blank, as in OpenTaiko.
        if roll.is_none() {
            *roll = Some(OpenRoll {
                time: cur.time,
                kind,
                big,
                velocity: cur.velocity(),
                // Rolls and balloons stay on the lane, coming in from the right whatever way
                // the notes come (as TJAPlayer3 draws them): as fast as they would.
                motion: NoteMotion {
                    dir: None,
                    ..cur.motion(cur.time, drawing.reference)
                },
                tick_spacing: if kind == Kind::Roll {
                    tick_spacing(cur.bpm.abs())
                } else {
                    0.0
                },
                hits,
                tag: cur.tag,
            });
        }
    };
    match c {
        '0' => {}
        '1' => notes.push(hit(Kind::Don, false)),
        '2' => notes.push(hit(Kind::Kat, false)),
        // Big notes; A/B are the two-player joint ones.
        '3' | 'A' => notes.push(hit(Kind::Don, true)),
        '4' | 'B' => notes.push(hit(Kind::Kat, true)),
        // OpenTaiko's Kadon, hit with Don and Ka at once: there is no such note here.
        'G' => {
            warnings.add("Kadon notes (G) are played as big Ka");
            notes.push(hit(Kind::Kat, true));
        }
        // Rolls; H and I are OpenTaiko's big and small coloured ones.
        '5' | 'I' => start(Kind::Roll, false, 0, roll),
        '6' | 'H' => start(Kind::Roll, true, 0, roll),
        // D is OpenTaiko's fuse roll: a balloon with a bomb at its end, played as a balloon.
        'D' => {
            warnings.add("fuse rolls (D) are played as balloons");
            start(
                Kind::Swell,
                false,
                pops.unwrap_or(DEFAULT_BALLOON_HITS),
                roll,
            );
        }
        '7' | '9' => start(
            Kind::Swell,
            false,
            pops.unwrap_or(DEFAULT_BALLOON_HITS),
            roll,
        ),
        '8' => match roll.take() {
            Some(r) => {
                drawing.notes.push(r.motion);
                let note = Note {
                    time: r.time,
                    end_time: cur.time.max(r.time),
                    kind: r.kind,
                    big: r.big,
                    velocity: r.velocity,
                    tick_spacing: r.tick_spacing,
                    required_hits: r.hits,
                    fake_len: 0.0,
                    fake_fades: false,
                    fake_length: 0.0,
                    tiny: false,
                    unhittable: false,
                };
                notes.push((note, r.tag));
            }
            None => warnings.add("a roll end (8) with no roll: skipped"),
        },
        'C' => warnings.add("bombs (C) are not supported: skipped"),
        'F' => warnings.add("ADLib notes (F) are not supported: skipped"),
        _ => warnings.add(format!("unknown note {c:?}: skipped")),
    }
    // A don or kat put down here (a roll's comes with its end).
    if notes.len() > before && c != '8' {
        drawing.notes.push(cur.motion(cur.time, drawing.reference));
    }
}

#[cfg(test)]
mod tests;
