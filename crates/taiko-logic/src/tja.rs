//! `.tja` charts: Taiko Jiro's text format, as TJAPlayer3 and OpenTaiko read it.
//!
//! A file holds one song and a chart per course (Easy, Normal, Hard, Oni, Edit). Each course is
//! read straight into a `Beatmap` (no `.osu` text in between), timed the way OpenTaiko's default
//! mode times it: every division of a bar lasts `4 beats × MEASURE / divisions` at the BPM it
//! starts at, added up in `f64` with nothing rounded, so hundreds of `#BPMCHANGE`s don't drift.
//! Commands take effect where they stand in the bar.
//!
//! Only the Master branch of a branching chart is read (playing branches is a task of its own).

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::beatmap::{Barline, Beatmap, ControlPoint, Kind, Meta, Note};

/// The osu! slider multiplier a TJA chart scrolls like: one beat at `#SCROLL 1` covers the
/// distance an osu! taiko map with SliderMultiplier 1.4 covers.
pub const SLIDER_MULTIPLIER: f64 = 1.4;
/// osu!'s `LEGACY_TAIKO_VELOCITY_MULTIPLIER`, as `beatmap` applies it.
const TAIKO_VELOCITY_MULTIPLIER: f64 = 1.4;
/// Pops a balloon needs when the chart's `BALLOON:` list runs out.
pub const DEFAULT_BALLOON_HITS: u32 = 5;
/// The slowest scroll a note gets: `#SCROLL 0` (notes standing still) would never arrive.
const MIN_SCROLL: f64 = 0.01;
/// What `chart_hash` hashes first: a new normalization gets a new tag, so old and new hashes
/// never mix.
const HASH_TAG: &[u8] = b"taiko.rs tja chart v1\n";

/// A chart's difficulty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CourseKind {
    // The values go into `chart_hash`: never renumber them.
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
    /// Identifies the chart however the file was written (see `chart_hash`).
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
    bpm: Option<f64>,
    offset: f64,
    demo_start: f64,
    course: CourseKind,
    level: u32,
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
            bpm: None,
            offset: 0.0,
            demo_start: 0.0,
            // TJAPlayer3 reads a chart with no COURSE as Oni.
            course: CourseKind::Oni,
            level: 0,
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
        let map = build(chart, &mut warnings);
        if map.notes.is_empty() {
            warnings.add(format!(
                "the {} chart has no notes",
                chart.header.course.name()
            ));
            continue;
        }
        let hash = chart_hash(chart.header.course, &map.notes);
        courses.push(Course {
            kind: chart.header.course,
            level: chart.header.level,
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
                h.level = 0;
                h.balloon.clear();
                h.branch_balloon = [None, None, None];
            }
            None => warnings.add(format!("COURSE:{value} is not played: read as Oni")),
        },
        "LEVEL" => h.level = value.parse().unwrap_or(0),
        "BALLOON" => h.balloon = balloon_list(value),
        "BALLOONNOR" => h.branch_balloon[0] = Some(balloon_list(value)),
        "BALLOONEXP" => h.branch_balloon[1] = Some(balloon_list(value)),
        "BALLOONMAS" => h.branch_balloon[2] = Some(balloon_list(value)),
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

/// A line of a chart once the branch to play is chosen.
#[derive(Debug)]
enum Item<'a> {
    Command(String, &'a str),
    /// Note data, with the pops of each balloon head on it in order (None for a head that
    /// starts nothing).
    Data(&'a str, Vec<Option<u32>>),
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

/// The chart's lines with only the Master branch kept (or the highest one a branch section
/// has), each balloon given its pops.
fn pick_branch<'a>(chart: &RawChart<'a>, warnings: &mut Warnings) -> Vec<Item<'a>> {
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
    if !sections.is_empty() {
        warnings.add("branches: only the Master branch is played");
    }
    let chosen_in = |s: &[bool; 3]| (0..3).rev().find(|&i| s[i]);

    let h = &chart.header;
    let mut out = Vec::new();
    let mut section: Option<usize> = None;
    // Which branch the current lines belong to: None for lines every branch shares.
    branch = None;
    let mut chosen: Option<usize> = None;
    // The branch played: the one chosen in the last section (Master before any).
    let mut played = 2;
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
                out.push(Item::Command(name, arg));
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
            out.push(Item::Data(line, pops));
        }
    }
    out
}

/// Scroll velocity (osu!px per ms) at `bpm` and `#SCROLL scroll`: TJA notes move with both.
pub fn velocity(bpm: f64, scroll: f64) -> f64 {
    SLIDER_MULTIPLIER * TAIKO_VELOCITY_MULTIPLIER * 100.0 * scroll.abs().max(MIN_SCROLL) * bpm
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

/// A roll or balloon whose end (`8`) is still to come.
struct OpenRoll {
    time: f64,
    kind: Kind,
    big: bool,
    velocity: f64,
    tick_spacing: f64,
    hits: u32,
}

/// Where the chart is while it's read.
struct Cursor {
    time: f64,
    bpm: f64,
    scroll: f64,
    /// Bar length in whole notes (`#MEASURE 3/4` = 0.75).
    measure: f64,
    gogo: bool,
    barlines: bool,
    beat_origin: f64,
}

impl Cursor {
    fn velocity(&self) -> f64 {
        velocity(self.bpm, self.scroll)
    }

    fn control_point(&self) -> ControlPoint {
        ControlPoint {
            time: self.time,
            beat_origin: self.beat_origin,
            beat_length: 60_000.0 / self.bpm,
            sv: self.scroll.abs().max(MIN_SCROLL),
            kiai: self.gogo,
            volume: 1.0,
        }
    }
}

fn build(chart: &RawChart, warnings: &mut Warnings) -> Beatmap {
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
        measure: 1.0,
        gogo: false,
        barlines: true,
        beat_origin: start,
    };
    let mut notes: Vec<Note> = Vec::new();
    let mut barlines: Vec<Barline> = Vec::new();
    let mut points: Vec<ControlPoint> = vec![cur.control_point()];
    let mut roll: Option<OpenRoll> = None;

    // A bar: commands and note symbols in order, each symbol with its balloon pops if any.
    enum Step<'a> {
        Command(&'a str, &'a str),
        Symbol(char, Option<u32>),
    }
    let items = pick_branch(chart, warnings);
    let mut bar: Vec<Step> = Vec::new();
    let mut bars: Vec<Vec<Step>> = Vec::new();
    for item in &items {
        match item {
            Item::Command(name, arg) => bar.push(Step::Command(name, arg)),
            Item::Data(line, pops) => {
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
                        bar.push(Step::Symbol(c, p));
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
        let put_barline = |cur: &Cursor, barlines: &mut Vec<Barline>| {
            if cur.barlines {
                barlines.push(Barline {
                    time: cur.time,
                    velocity: cur.velocity(),
                });
            }
        };
        for step in bar {
            match *step {
                Step::Command(name, arg) => {
                    apply(name, arg, &mut cur, &mut points, warnings);
                }
                Step::Symbol(c, pops) => {
                    if barline_due {
                        put_barline(&cur, &mut barlines);
                        barline_due = false;
                    }
                    note(c, pops, &cur, &mut roll, &mut notes, warnings);
                    cur.time += 240_000.0 / cur.bpm * cur.measure / divisions as f64;
                }
            }
        }
        if barline_due {
            put_barline(&cur, &mut barlines);
        }
        if divisions == 0 {
            cur.time += 240_000.0 / cur.bpm * cur.measure;
        }
    }
    if roll.is_some() {
        warnings.add("a roll or balloon has no end (8): dropped");
    }

    // A negative #DELAY can set later notes before earlier ones.
    notes.sort_by(|a, b| a.time.total_cmp(&b.time));
    barlines.sort_by(|a, b| a.time.total_cmp(&b.time));
    points.sort_by(|a, b| a.time.total_cmp(&b.time));
    points.dedup_by(|later, earlier| {
        let same = later.time == earlier.time;
        if same {
            *earlier = *later;
        }
        same
    });

    let version = if h.level > 0 {
        format!("{} ★{}", h.course.name(), h.level)
    } else {
        h.course.name().to_string()
    };
    let meta = Meta {
        title: h.title.clone(),
        artist: h.subtitle.clone(),
        creator: h.maker.clone(),
        version,
        audio_file: h.wave.clone(),
        preview_time: h.demo_start * 1000.0,
        mode: 1,
        od: 5.0,
        hp: 5.0,
        slider_multiplier: SLIDER_MULTIPLIER,
        slider_tick_rate: 1.0,
        format_version: 14,
        genres: crate::genres::of(&h.genre),
        tags: h.genre.clone(),
        ..Default::default()
    };
    Beatmap {
        meta,
        notes,
        barlines,
        control_points: points,
        first_velocity: velocity(bpm, 1.0),
        gimmick: None,
        fakes: Vec::new(),
    }
}

fn apply(
    name: &str,
    arg: &str,
    cur: &mut Cursor,
    points: &mut Vec<ControlPoint>,
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
            Some(bpm) if bpm > 0.0 => {
                cur.bpm = bpm;
                cur.beat_origin = cur.time;
                points.push(cur.control_point());
            }
            Some(_) => warnings.add(format!("#BPMCHANGE {arg}: not above 0, ignored")),
            None => {}
        },
        "MEASURE" => {
            let parsed = arg.split_once('/').and_then(|(a, b)| {
                Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
            });
            match parsed {
                Some((a, b)) if a > 0.0 && b > 0.0 && (a / b).is_finite() => cur.measure = a / b,
                _ => warnings.add(format!("#MEASURE {arg}: not a time signature, ignored")),
            }
        }
        "SCROLL" => match scroll_value(arg) {
            Some(v) if v.is_finite() => {
                if v < 0.0 {
                    warnings.add("negative #SCROLL: played at the same speed from the right");
                }
                cur.scroll = v;
                points.push(cur.control_point());
            }
            _ => warnings.add(format!("#SCROLL {arg}: not a number, ignored")),
        },
        "DELAY" => {
            if let Some(s) = number(warnings) {
                cur.time += s * 1000.0;
                cur.beat_origin = cur.time;
                points.push(cur.control_point());
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
    notes: &mut Vec<Note>,
    warnings: &mut Warnings,
) {
    let hit = |kind: Kind, big: bool| Note {
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
    let start = |kind: Kind, big: bool, hits: u32, roll: &mut Option<OpenRoll>| {
        // A second head before the end is a blank, as in OpenTaiko.
        if roll.is_none() {
            *roll = Some(OpenRoll {
                time: cur.time,
                kind,
                big,
                velocity: cur.velocity(),
                tick_spacing: if kind == Kind::Roll {
                    tick_spacing(cur.bpm)
                } else {
                    0.0
                },
                hits,
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
            Some(r) => notes.push(Note {
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
            }),
            None => warnings.add("a roll end (8) with no roll: skipped"),
        },
        'C' => warnings.add("bombs (C) are not supported: skipped"),
        'F' => warnings.add("ADLib notes (F) are not supported: skipped"),
        _ => warnings.add(format!("unknown note {c:?}: skipped")),
    }
}

/// The hash a chart is known by (leaderboards, local scores, the upload check). It covers what
/// a play depends on: the course (its hit windows), each note's kind, size, times, pops and
/// speed; times to 1/100 ms and speeds to 1e-6, so float noise can't split a board.
pub fn chart_hash(kind: CourseKind, notes: &[Note]) -> ChartHash {
    let q = |v: f64, scale: f64| ((v * scale).round() as i64).to_le_bytes();
    let mut h = Sha256::new();
    h.update(HASH_TAG);
    h.update([kind.id()]);
    h.update((notes.len() as u64).to_le_bytes());
    for n in notes {
        let k = match n.kind {
            Kind::Don => 0u8,
            Kind::Kat => 1,
            Kind::Roll => 2,
            Kind::Swell => 3,
        };
        h.update([k, n.big as u8]);
        h.update(q(n.time, 100.0));
        h.update(q(n.end_time, 100.0));
        h.update(n.required_hits.to_le_bytes());
        h.update(q(n.velocity, 1e6));
    }
    ChartHash(h.finalize().into())
}

#[cfg(test)]
mod tests;
