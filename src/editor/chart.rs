//! The chart the editor edits: a `.osu` file held as its own lines, with the two sections the
//! editor owns (`[TimingPoints]`, `[HitObjects]`) parsed into editable lists.
//!
//! Everything else in the file (storyboard events, breaks, colours, bookmarks, unknown keys) is
//! never modelled, so it can't be lost: it is written back byte for byte. A chart that was not
//! edited is written back as the exact text it was read from, and in an edited one every
//! timing point and hit object that wasn't touched keeps its own original line (osu! files of
//! older formats leave trailing fields out, and filling them in would change what stable reads).
//! The one deliberate loss: `//` comments and unparsable lines inside the two owned sections,
//! when the chart has been edited.

use std::path::Path;

pub const CIRCLE: i32 = 1;
pub const SLIDER: i32 = 2;
pub const SPINNER: i32 = 8;

pub const SOUND_WHISTLE: i32 = 2;
pub const SOUND_FINISH: i32 = 4;
pub const SOUND_CLAP: i32 = 8;

/// Bit 0 of a timing point's effects: kiai time.
pub const EFFECT_KIAI: i32 = 1;
/// Bit 3: the red line's own barline is left out.
pub const EFFECT_OMIT_BARLINE: i32 = 8;

/// 120 BPM: what a chart with no timing at all falls back to.
pub const DEFAULT_BEAT_LENGTH: f64 = 500.0;

/// Renders a number the way osu! writes one: a whole value without `.0`, anything else with the
/// digits it takes to read back exactly.
pub fn fmt_num(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// An integer field. Some editors write "4.0" where one is expected.
fn int_field(s: Option<&str>, default: i32) -> i32 {
    match s.map(str::trim) {
        None | Some("") => default,
        Some(t) => t
            .parse::<i32>()
            .ok()
            .or_else(|| {
                t.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(|v| v as i32)
            })
            .unwrap_or(default),
    }
}

#[derive(Clone, Debug)]
struct Raw<K> {
    line: String,
    key: K,
}

type TimingKey = (u64, u64, i32, i32, i32, i32, i32, i32);

#[derive(Clone, Debug)]
pub struct TimingPoint {
    /// Identity that survives edits and reordering (what undo refers to).
    pub id: u64,
    pub time: f64,
    pub beat_length: f64,
    pub meter: i32,
    pub sample_set: i32,
    pub sample_index: i32,
    pub volume: i32,
    /// The flag as authored (1 = red). Stored raw so a round trip reproduces it.
    pub uninherited: i32,
    pub effects: i32,
    raw: Option<Raw<TimingKey>>,
}

impl TimingPoint {
    pub fn new(time: f64, beat_length: f64, uninherited: bool) -> TimingPoint {
        TimingPoint {
            id: 0,
            time,
            beat_length,
            meter: 4,
            sample_set: 1,
            sample_index: 0,
            volume: 100,
            uninherited: i32::from(uninherited),
            effects: 0,
            raw: None,
        }
    }

    /// A red line at `bpm`.
    pub fn red(time: f64, bpm: f64) -> TimingPoint {
        TimingPoint::new(time, 60_000.0 / bpm, true)
    }

    /// A green line with slider velocity `sv`.
    pub fn green(time: f64, sv: f64) -> TimingPoint {
        TimingPoint::new(time, -100.0 / sv, false)
    }

    /// A red line with an explicit meter and the omit-first-barline flag.
    pub fn red_with(time: f64, bpm: f64, meter: i32, omit_barline: bool) -> TimingPoint {
        let mut p = TimingPoint::red(time, bpm);
        p.meter = meter;
        p.set_omit_barline(omit_barline);
        p
    }

    /// A green line that takes its hitsound settings, volume and kiai from `template`.
    pub fn green_like(time: f64, sv: f64, template: &TimingPoint) -> TimingPoint {
        let mut p = TimingPoint::green(time, sv);
        p.sample_set = template.sample_set;
        p.sample_index = template.sample_index;
        p.volume = template.volume;
        p.meter = template.meter;
        p.effects = template.effects & EFFECT_KIAI;
        p
    }

    pub fn is_red(&self) -> bool {
        self.uninherited == 1
    }

    pub fn kiai(&self) -> bool {
        self.effects & EFFECT_KIAI != 0
    }

    pub fn omits_barline(&self) -> bool {
        self.effects & EFFECT_OMIT_BARLINE != 0
    }

    pub fn bpm(&self) -> Option<f64> {
        (self.is_red() && self.beat_length > 0.0).then(|| 60_000.0 / self.beat_length)
    }

    /// Slider velocity multiplier: 1.0 for a red line, and for a NaN green line (every
    /// comparison against NaN is false, which stable reads as 1.0).
    pub fn sv(&self) -> f64 {
        if self.is_red() || self.beat_length >= 0.0 || self.beat_length.is_nan() {
            1.0
        } else {
            -100.0 / self.beat_length
        }
    }

    pub fn set_sv(&mut self, sv: f64) -> Result<(), String> {
        if self.is_red() {
            return Err("a red line has no slider velocity".into());
        }
        if !sv.is_finite() || sv <= 0.0 {
            return Err("slider velocity must be positive".into());
        }
        self.beat_length = -100.0 / sv;
        Ok(())
    }

    pub fn set_kiai(&mut self, on: bool) {
        if on {
            self.effects |= EFFECT_KIAI;
        } else {
            self.effects &= !EFFECT_KIAI;
        }
    }

    pub fn set_omit_barline(&mut self, on: bool) {
        if on {
            self.effects |= EFFECT_OMIT_BARLINE;
        } else {
            self.effects &= !EFFECT_OMIT_BARLINE;
        }
    }

    /// Whether any field differs from `other`'s.
    pub fn differs_from(&self, other: &TimingPoint) -> bool {
        self.key() != other.key()
    }

    fn key(&self) -> TimingKey {
        (
            self.time.to_bits(),
            self.beat_length.to_bits(),
            self.meter,
            self.sample_set,
            self.sample_index,
            self.volume,
            self.uninherited,
            self.effects,
        )
    }

    /// `true` if this is the point as it was read, so its original line can be written back.
    fn untouched(&self) -> bool {
        self.raw.as_ref().is_some_and(|r| r.key == self.key())
    }

    pub fn parse(line: &str) -> Option<TimingPoint> {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            return None;
        }
        let f: Vec<&str> = t.split(',').collect();
        if f.len() < 2 {
            return None;
        }
        let time = f[0].trim().parse::<f64>().ok()?;
        let beat_length = f[1].trim().parse::<f64>().ok()?;
        let mut p = TimingPoint {
            id: 0,
            time,
            beat_length,
            meter: int_field(f.get(2).copied(), 4),
            sample_set: int_field(f.get(3).copied(), 0),
            sample_index: int_field(f.get(4).copied(), 0),
            volume: int_field(f.get(5).copied(), 100),
            // Formats before the flag existed: a negative beat length is the green line.
            uninherited: int_field(f.get(6).copied(), i32::from(beat_length >= 0.0)),
            effects: int_field(f.get(7).copied(), 0),
            raw: None,
        };
        p.raw = Some(Raw {
            line: line.to_string(),
            key: p.key(),
        });
        Some(p)
    }

    pub fn to_line(&self, ending: &str) -> String {
        format!(
            "{},{},{},{},{},{},{},{}{ending}",
            fmt_num(self.time),
            fmt_num(self.beat_length),
            self.meter,
            self.sample_set,
            self.sample_index,
            self.volume,
            self.uninherited,
            self.effects
        )
    }

    /// Why this point can't be written, if it can't. Only checked for points that were changed
    /// (see `Chart::validate`): a map as authored may hold anything.
    pub fn problem(&self) -> Option<String> {
        let at = fmt_num(self.time);
        if !self.time.is_finite() || !self.beat_length.is_finite() {
            return Some(format!("timing point at {at} isn't a finite number"));
        }
        if self.beat_length == 0.0 {
            return Some(format!("timing point at {at} has a zero beat length"));
        }
        if self.is_red() && self.beat_length < 0.0 {
            return Some(format!("red line at {at} has a negative beat length"));
        }
        if !self.is_red() && self.beat_length > 0.0 {
            return Some(format!("green line at {at} has a positive beat length"));
        }
        if self.meter < 1 {
            return Some(format!("timing point at {at} has meter {}", self.meter));
        }
        if !(0..=100).contains(&self.volume) {
            return Some(format!("timing point at {at} has volume {}", self.volume));
        }
        if self.effects < 0 {
            return Some(format!("timing point at {at} has negative effects"));
        }
        None
    }
}

type ObjectKey = (u64, u64, u64, i32, i32, String);

#[derive(Clone, Debug)]
pub struct HitObject {
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub time: f64,
    /// Type bits: circle 1, slider 2, new combo 4, spinner 8.
    pub kind: i32,
    /// Hitsound bits: whistle 2, finish 4, clap 8.
    pub sound: i32,
    /// Everything after the hitsound, as written: curve, slides, length, edge sounds and sample
    /// for a slider, the end time for a spinner, the sample for a circle.
    pub rest: String,
    /// A `.tko` fake note: drawn in the game, never judged. Kept in the gimmick's `fakes` on
    /// save, never in the chart's hit objects.
    pub fake: bool,
    raw: Option<Raw<ObjectKey>>,
}

/// What a hit object plays as in taiko.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taiko {
    Don,
    Kat,
    Roll,
    Swell,
}

impl HitObject {
    fn bare(time: f64, kind: i32, sound: i32, rest: String) -> HitObject {
        HitObject {
            id: 0,
            x: 256.0,
            y: 192.0,
            time,
            kind,
            sound,
            rest,
            fake: false,
            raw: None,
        }
    }

    /// A don or kat, `big` for the finisher.
    pub fn hit(time: f64, kat: bool, big: bool) -> HitObject {
        let sound = if kat { SOUND_WHISTLE } else { 0 } | if big { SOUND_FINISH } else { 0 };
        HitObject::bare(time, CIRCLE, sound, "0:0:0:0:".into())
    }

    /// A plain circle at (`x`, `y`) with the given hitsound bits.
    pub fn circle_at(time: f64, x: f64, y: f64, sound: i32) -> HitObject {
        let mut o = HitObject::bare(time, CIRCLE, sound, "0:0:0:0:".into());
        o.x = x;
        o.y = y;
        o
    }

    /// A one-segment slider whose `length` is written as given (a gimmick's negative or tiny one
    /// included), in the short form osu!stable reads: `curve,slides,length,sample`.
    pub fn slider_at(time: f64, x: f64, y: f64, sound: i32, length: f64) -> HitObject {
        let mut o = HitObject::bare(
            time,
            SLIDER,
            sound,
            format!(
                "L|{}:{},1,{},0:0:0:0:",
                fmt_num(x + 100.0),
                fmt_num(y),
                fmt_num(length)
            ),
        );
        o.x = x;
        o.y = y;
        o
    }

    /// A circle (not a slider or swell).
    pub fn is_circle(&self) -> bool {
        self.kind & (CIRCLE | SLIDER | SPINNER) == CIRCLE
    }

    /// A drumroll `length` osu!pixels long.
    pub fn roll(time: f64, length: f64, big: bool) -> HitObject {
        let sound = if big { SOUND_FINISH } else { 0 };
        let mut o = HitObject::bare(
            time,
            SLIDER,
            sound,
            format!("L|{}:192,1,{}", fmt_num(256.0 + length), fmt_num(length)),
        );
        o.rest
            .push_str(&format!(",{sound}|{sound},0:0|0:0,0:0:0:0:"));
        o
    }

    /// A swell lasting until `end`.
    pub fn swell(time: f64, end: f64) -> HitObject {
        HitObject::bare(time, SPINNER, 0, format!("{},0:0:0:0:", fmt_num(end)))
    }

    pub fn taiko(&self) -> Taiko {
        if self.kind & SPINNER != 0 {
            Taiko::Swell
        } else if self.kind & SLIDER != 0 {
            Taiko::Roll
        } else if self.sound & (SOUND_WHISTLE | SOUND_CLAP) != 0 {
            Taiko::Kat
        } else {
            Taiko::Don
        }
    }

    pub fn big(&self) -> bool {
        self.sound & SOUND_FINISH != 0
    }

    /// The `rest` fields split on commas.
    fn rest_fields(&self) -> Vec<&str> {
        self.rest.split(',').map(str::trim).collect()
    }

    /// A slider's written length in osu!pixels (`NaN` and the like stay as they were read).
    pub fn length(&self) -> Option<f64> {
        if self.kind & SLIDER == 0 {
            return None;
        }
        self.rest_fields().get(2)?.parse().ok()
    }

    pub fn slides(&self) -> i32 {
        if self.kind & SLIDER == 0 {
            return 1;
        }
        int_field(self.rest_fields().get(1).copied(), 1).max(1)
    }

    /// A spinner's own end time.
    pub fn spinner_end(&self) -> Option<f64> {
        if self.kind & SPINNER == 0 {
            return None;
        }
        self.rest_fields().first()?.parse().ok()
    }

    pub fn set_spinner_end(&mut self, end: f64) {
        if self.kind & SPINNER == 0 {
            return;
        }
        let tail = self.rest.split_once(',').map(|(_, t)| t.to_string());
        self.rest = match tail {
            Some(t) => format!("{},{t}", fmt_num(end)),
            None => fmt_num(end),
        };
    }

    /// Rewrites a slider's length (and the end point of an `L` curve, which stable ignores but
    /// the osu! editor draws).
    pub fn set_length(&mut self, length: f64) {
        if self.kind & SLIDER == 0 {
            return;
        }
        let mut f: Vec<String> = self.rest.split(',').map(str::to_string).collect();
        if f.len() < 3 {
            return;
        }
        f[2] = fmt_num(length);
        if let Some(rest) = f[0].trim().strip_prefix("L|")
            && !rest.contains('|')
        {
            f[0] = format!("L|{}:{}", fmt_num(self.x + length), fmt_num(self.y));
        }
        self.rest = f.join(",");
    }

    fn key(&self) -> ObjectKey {
        (
            self.x.to_bits(),
            self.y.to_bits(),
            self.time.to_bits(),
            self.kind,
            self.sound,
            self.rest.clone(),
        )
    }

    fn untouched(&self) -> bool {
        self.raw.as_ref().is_some_and(|r| r.key == self.key())
    }

    pub fn parse(line: &str) -> Option<HitObject> {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            return None;
        }
        let mut it = t.splitn(6, ',');
        let x = it.next()?.trim().parse::<f64>().ok()?;
        let y = it.next()?.trim().parse::<f64>().ok()?;
        let time = it.next()?.trim().parse::<f64>().ok()?;
        let kind = int_field(it.next(), 1);
        let sound = int_field(it.next(), 0);
        let rest = it.next().unwrap_or("").to_string();
        let mut o = HitObject {
            id: 0,
            x,
            y,
            time,
            kind,
            sound,
            rest,
            fake: false,
            raw: None,
        };
        o.raw = Some(Raw {
            line: line.to_string(),
            key: o.key(),
        });
        Some(o)
    }

    pub fn to_line(&self, ending: &str) -> String {
        let head = format!(
            "{},{},{},{},{}",
            fmt_num(self.x),
            fmt_num(self.y),
            fmt_num(self.time),
            self.kind,
            self.sound
        );
        if self.rest.is_empty() {
            format!("{head}{ending}")
        } else {
            format!("{head},{}{ending}", self.rest)
        }
    }

    /// Where it ends: a roll by its length at the slider velocity `sv` and beat length
    /// `beat_length` in force, a swell by its end. The roll's arithmetic is the game's: the beat
    /// length and velocity are clamped to what gameplay timing allows, and the duration is whole
    /// milliseconds.
    pub fn end_time(&self, slider_multiplier: f64, beat_length: f64, sv: f64) -> f64 {
        match self.taiko() {
            Taiko::Swell => self.spinner_end().unwrap_or(self.time).max(self.time),
            Taiko::Roll => {
                let len = self
                    .length()
                    .filter(|l| l.is_finite())
                    .unwrap_or(0.0)
                    .max(0.0);
                let per_beat = slider_multiplier * 100.0 * sv.clamp(0.1, 10.0);
                let beat = beat_length.clamp(6.0, 60_000.0);
                self.time + (len * self.slides() as f64 / per_beat * beat).floor()
            }
            _ => self.time,
        }
    }

    /// The written length that makes a roll last `ms` at the slider velocity and beat length
    /// given (the inverse of `end_time`, rounded up so the whole-millisecond floor lands on `ms`).
    pub fn length_for(
        ms: f64,
        slides: i32,
        slider_multiplier: f64,
        beat_length: f64,
        sv: f64,
    ) -> f64 {
        let per_beat = slider_multiplier * 100.0 * sv.clamp(0.1, 10.0);
        let beat = beat_length.clamp(6.0, 60_000.0);
        (ms.max(0.0) * per_beat / beat / slides.max(1) as f64 * 10_000.0).ceil() / 10_000.0
    }

    /// The same object `delta` ms later (a swell's end moves with it).
    pub fn shifted(&self, delta: f64) -> HitObject {
        let mut o = self.clone();
        o.time += delta;
        if let Some(end) = self.spinner_end() {
            o.set_spinner_end(end + delta);
        }
        o
    }
}

#[derive(Clone, Debug)]
struct Section {
    name: String,
    /// Line index of the `[Header]`.
    header: usize,
    /// One past the last body line.
    end: usize,
}

#[derive(Clone, Debug)]
pub struct Chart {
    text: String,
    lines: Vec<String>,
    ending: &'static str,
    pub timing: Vec<TimingPoint>,
    pub objects: Vec<HitObject>,
    next_id: u64,
    /// Changed since it was read.
    pub dirty: bool,
    /// The file wasn't valid UTF-8: it can be saved as a new file but never over the original.
    pub lossy: bool,
    /// The client gimmick of a `.tko` chart (`None` for a plain `.osu`). It is written after the
    /// chart's own sections; the chart's lines never hold it.
    pub gimmick: Option<crate::tko::gimmick::Gimmick>,
}

fn section_name(line: &str) -> Option<&str> {
    let t = line.trim();
    (t.starts_with('[') && t.ends_with(']') && t.len() >= 2).then(|| &t[1..t.len() - 1])
}

fn ends_with_newline(s: &str) -> bool {
    s.ends_with('\n')
}

impl Chart {
    pub fn parse(text: &str) -> Chart {
        let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
        let mut chart = Chart {
            text: text.to_string(),
            lines,
            ending,
            timing: Vec::new(),
            objects: Vec::new(),
            next_id: 1,
            dirty: false,
            lossy: false,
            gimmick: None,
        };
        let mut section = String::new();
        for line in &chart.lines.clone() {
            if let Some(name) = section_name(line) {
                section = name.to_string();
                continue;
            }
            match section.as_str() {
                "TimingPoints" => {
                    if let Some(mut p) = TimingPoint::parse(line) {
                        p.id = chart.take_id();
                        chart.timing.push(p);
                    }
                }
                "HitObjects" => {
                    if let Some(mut o) = HitObject::parse(line) {
                        o.id = chart.take_id();
                        chart.objects.push(o);
                    }
                }
                _ => {}
            }
        }
        chart
    }

    pub fn read(path: &Path) -> Result<Chart, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (text, lossy) = match String::from_utf8(bytes) {
            Ok(t) => (t, false),
            Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
        };
        let mut chart =
            if crate::beatmap::is_tko(path) || crate::tko::format::version_of(&text).is_some() {
                Chart::parse_tko(&text)?
            } else {
                Chart::parse(&text)
            };
        chart.lossy = lossy;
        Ok(chart)
    }

    /// A `.tko` text: the chart, with the gimmick taken out of it. A newer format than this game
    /// reads, or a broken section, is an error (the editor never rewrites what it doesn't understand).
    pub fn parse_tko(text: &str) -> Result<Chart, String> {
        let parsed = crate::tko::format::read(text).map_err(|e| e.to_string())?;
        let (osu, _) = crate::tko::format::split(text);
        let mut chart = Chart::parse(&osu);
        let mut gimmick = parsed.gimmick;
        // Fake notes are edited as notes: they come out of the gimmick and go back on save.
        for f in std::mem::take(&mut gimmick.fakes) {
            let mut o = HitObject::hit(f.t, f.kat, f.big);
            o.fake = true;
            o.id = chart.take_id();
            let at = chart.objects.partition_point(|q| q.time <= o.time);
            chart.objects.insert(at, o);
        }
        chart.gimmick = Some(gimmick);
        Ok(chart)
    }

    pub fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn sections(&self) -> Vec<Section> {
        let mut out: Vec<Section> = Vec::new();
        for (i, l) in self.lines.iter().enumerate() {
            if let Some(name) = section_name(l) {
                if let Some(last) = out.last_mut() {
                    last.end = i;
                }
                out.push(Section {
                    name: name.to_string(),
                    header: i,
                    end: self.lines.len(),
                });
            }
        }
        out
    }

    /// A `Key:Value` line of a section.
    pub fn get_key(&self, section: &str, key: &str) -> Option<String> {
        let s = self.sections().into_iter().find(|s| s.name == section)?;
        self.lines[s.header + 1..s.end].iter().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    }

    pub fn number(&self, section: &str, key: &str, default: f64) -> f64 {
        self.get_key(section, key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Sets a `Key:Value` line, keeping the spacing the file uses, adding the line (and the
    /// section) if there isn't one.
    pub fn set_key(&mut self, section: &str, key: &str, value: &str) {
        let ending = self.ending;
        let found = self.sections().into_iter().find(|s| s.name == section);
        let Some(s) = found else {
            if self.lines.last().is_some_and(|l| !ends_with_newline(l)) {
                let last = self.lines.len() - 1;
                self.lines[last].push_str(ending);
            }
            self.lines.push(ending.to_string());
            self.lines.push(format!("[{section}]{ending}"));
            self.lines.push(format!("{key}:{value}{ending}"));
            self.dirty = true;
            return;
        };
        for i in s.header + 1..s.end {
            let line = &self.lines[i];
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            if k.trim() != key {
                continue;
            }
            let spaced = v.starts_with(' ');
            let end = if ends_with_newline(line) {
                if line.ends_with("\r\n") { "\r\n" } else { "\n" }
            } else {
                ""
            };
            self.lines[i] = format!("{k}:{}{value}{end}", if spaced { " " } else { "" });
            self.dirty = true;
            return;
        }
        let mut at = s.end;
        while at > s.header + 1 && self.lines[at - 1].trim().is_empty() {
            at -= 1;
        }
        if at > 0 && !ends_with_newline(&self.lines[at - 1]) {
            self.lines[at - 1].push_str(ending);
        }
        self.lines.insert(at, format!("{key}:{value}{ending}"));
        self.dirty = true;
    }

    /// Takes a `Key:Value` line out (undoing a key `set_key` added).
    pub fn remove_key(&mut self, section: &str, key: &str) {
        let Some(s) = self.sections().into_iter().find(|s| s.name == section) else {
            return;
        };
        let found = (s.header + 1..s.end).find(|&i| {
            self.lines[i]
                .split_once(':')
                .is_some_and(|(k, _)| k.trim() == key)
        });
        if let Some(i) = found {
            self.lines.remove(i);
            self.dirty = true;
        }
    }

    pub fn slider_multiplier(&self) -> f64 {
        self.number("Difficulty", "SliderMultiplier", 1.4)
    }

    // ---- editing ----

    /// Keeps `timing` ordered by time, equal times in the order they came in (stable and osu!
    /// both resolve a shared ms by file order, so a compound key would reorder SV and kiai).
    pub fn sort_timing(&mut self) {
        self.timing.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    pub fn sort_objects(&mut self) {
        self.objects.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// Adds a timing point after any point already at its time.
    pub fn insert_timing(&mut self, mut p: TimingPoint) -> u64 {
        if p.id == 0 {
            p.id = self.take_id();
        }
        let id = p.id;
        let at = self.timing.partition_point(|q| q.time <= p.time);
        self.timing.insert(at, p);
        self.dirty = true;
        id
    }

    pub fn insert_object(&mut self, mut o: HitObject) -> u64 {
        if o.id == 0 {
            o.id = self.take_id();
        }
        let id = o.id;
        let at = self.objects.partition_point(|q| q.time <= o.time);
        self.objects.insert(at, o);
        self.dirty = true;
        id
    }

    pub fn remove_timing(&mut self, id: u64) -> Option<TimingPoint> {
        let at = self.timing.iter().position(|p| p.id == id)?;
        self.dirty = true;
        Some(self.timing.remove(at))
    }

    pub fn remove_object(&mut self, id: u64) -> Option<HitObject> {
        let at = self.objects.iter().position(|o| o.id == id)?;
        self.dirty = true;
        Some(self.objects.remove(at))
    }

    #[cfg(test)]
    pub fn object_mut(&mut self, id: u64) -> Option<&mut HitObject> {
        self.dirty = true;
        self.objects.iter_mut().find(|o| o.id == id)
    }

    pub fn object(&self, id: u64) -> Option<&HitObject> {
        self.objects.iter().find(|o| o.id == id)
    }

    pub fn timing_point(&self, id: u64) -> Option<&TimingPoint> {
        self.timing.iter().find(|p| p.id == id)
    }

    // ---- writing ----

    /// Why the chart can't be written, if it can't: only points that were changed are checked.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(g) = self.gimmick_out() {
            crate::tko::format::write(&self.osu_text(), &g).map_err(|e| e.to_string())?;
        }
        for p in self.timing.iter().filter(|p| !p.untouched()) {
            if let Some(why) = p.problem() {
                return Err(why);
            }
        }
        if !self.timing.is_empty() && !self.timing.iter().any(TimingPoint::is_red) {
            return Err("a chart with timing points needs at least one red line".into());
        }
        for o in self.objects.iter().filter(|o| !o.untouched()) {
            if !o.time.is_finite() || !o.x.is_finite() || !o.y.is_finite() {
                return Err(format!(
                    "object at {} isn't a finite number",
                    fmt_num(o.time)
                ));
            }
        }
        Ok(())
    }

    fn timing_body(&self) -> Vec<String> {
        let mut pts: Vec<&TimingPoint> = self.timing.iter().collect();
        pts.sort_by(|a, b| a.time.total_cmp(&b.time));
        pts.into_iter()
            .map(|p| match (&p.raw, p.untouched()) {
                (Some(r), true) => with_ending(&r.line, self.ending),
                _ => p.to_line(self.ending),
            })
            .collect()
    }

    fn objects_body(&self) -> Vec<String> {
        let mut objs: Vec<&HitObject> = self.objects.iter().filter(|o| !o.fake).collect();
        objs.sort_by(|a, b| a.time.total_cmp(&b.time));
        objs.into_iter()
            .map(|o| match (&o.raw, o.untouched()) {
                (Some(r), true) => with_ending(&r.line, self.ending),
                _ => o.to_line(self.ending),
            })
            .collect()
    }

    /// The file's text. An unedited chart gives back exactly what was read.
    /// The file as it would be written: the chart, then (for a `.tko`) its gimmick.
    pub fn to_text(&self) -> String {
        let base = self.osu_text();
        match self.gimmick_out() {
            Some(g) => crate::tko::format::write(&base, &g).unwrap_or(base),
            None => base,
        }
    }

    /// The gimmick as written: with the fake notes put back in.
    fn gimmick_out(&self) -> Option<crate::tko::gimmick::Gimmick> {
        let mut g = self.gimmick.clone()?;
        g.fakes = self
            .objects
            .iter()
            .filter(|o| o.fake)
            .map(|o| crate::tko::gimmick::FakeNote {
                t: o.time,
                kat: o.taiko() == Taiko::Kat,
                big: o.big(),
            })
            .collect();
        g.sanitize();
        Some(g)
    }

    fn osu_text(&self) -> String {
        if !self.dirty {
            return self.text.clone();
        }
        let sections = self.sections();
        let mut out: Vec<String> = Vec::with_capacity(self.lines.len() + self.objects.len());
        let mut done = [false; 2];
        let owned = ["TimingPoints", "HitObjects"];
        let first = sections.first().map_or(self.lines.len(), |s| s.header);
        out.extend(self.lines[..first].iter().cloned());
        for s in &sections {
            out.push(self.lines[s.header].clone());
            let Some(which) = owned.iter().position(|n| *n == s.name) else {
                out.extend(self.lines[s.header + 1..s.end].iter().cloned());
                continue;
            };
            if done[which] {
                // A second section of the same name: osu! reads both, we keep the first.
                continue;
            }
            done[which] = true;
            out.extend(if which == 0 {
                self.timing_body()
            } else {
                self.objects_body()
            });
            // The blank lines that separate the section from the next stay as they were.
            let mut tail = s.end;
            while tail > s.header + 1 && self.lines[tail - 1].trim().is_empty() {
                tail -= 1;
            }
            out.extend(self.lines[tail..s.end].iter().cloned());
        }
        for (which, name) in owned.iter().enumerate() {
            let body = if which == 0 {
                self.timing_body()
            } else {
                self.objects_body()
            };
            if done[which] || body.is_empty() {
                continue;
            }
            if out.last().is_some_and(|l| !ends_with_newline(l)) {
                let last = out.len() - 1;
                out[last].push_str(self.ending);
            }
            out.push(self.ending.to_string());
            out.push(format!("[{name}]{}", self.ending));
            out.extend(body);
        }
        let mut text = out.concat();
        // A file that ended without a newline still does.
        if !self.text.ends_with('\n') && text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        text
    }

    /// Writes the chart to `path`: to a temporary file beside it first, read back and checked
    /// against the chart, then moved into place. `backup` keeps the first version a file had as
    /// `<name>.bak` (never overwritten by later saves).
    pub fn save(&self, path: &Path, backup: bool) -> Result<(), String> {
        self.validate()?;
        if self.lossy && path.exists() {
            return Err("the file isn't UTF-8: save it as a new difficulty instead".into());
        }
        let text = self.to_text();
        check_round_trip(self, &text)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!(
            ".{}.{}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("chart"),
            std::process::id()
        ));
        std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        if backup && path.exists() {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            let bak = std::path::PathBuf::from(bak);
            if !bak.exists() {
                std::fs::copy(path, &bak).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("{}: {e}", bak.display())
                })?;
            }
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
    }
}

fn with_ending(line: &str, ending: &str) -> String {
    if ends_with_newline(line) {
        line.to_string()
    } else {
        format!("{line}{ending}")
    }
}

/// Reads `text` back and checks it holds the chart's notes and timing: what the file says is
/// what the editor shows.
fn check_round_trip(chart: &Chart, text: &str) -> Result<(), String> {
    let back = Chart::parse(text);
    if back.objects.len() != chart.objects.len() {
        return Err(format!(
            "the saved file would hold {} objects, not {}",
            back.objects.len(),
            chart.objects.len()
        ));
    }
    if back.timing.len() != chart.timing.len() {
        return Err(format!(
            "the saved file would hold {} timing points, not {}",
            back.timing.len(),
            chart.timing.len()
        ));
    }
    let mut want: Vec<_> = chart
        .objects
        .iter()
        .map(|o| (o.time.to_bits(), o.kind, o.sound))
        .collect();
    let mut got: Vec<_> = back
        .objects
        .iter()
        .map(|o| (o.time.to_bits(), o.kind, o.sound))
        .collect();
    want.sort_unstable();
    got.sort_unstable();
    if want != got {
        return Err("the saved file's objects differ from the chart".into());
    }
    let mut want: Vec<_> = chart.timing.iter().map(TimingPoint::key).collect();
    let mut got: Vec<_> = back.timing.iter().map(TimingPoint::key).collect();
    want.sort_unstable();
    got.sort_unstable();
    if want != got {
        return Err("the saved file's timing points differ from the chart".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MAP: &str = "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: 1\r\n\r\n[Metadata]\r\nTitle:Test\r\nVersion:Oni\r\n\r\n[Difficulty]\r\nOverallDifficulty:5\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[Events]\r\n//Background and Video events\r\n0,0,\"bg.png\",0,0\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n4000,-50,4,1,0,100,0,1\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n256,192,1500,1,2,0:0:0:0:\r\n256,192,2000,1,12,0:0:0:0:\r\n256,192,2500,2,0,L|356:192,1,70\r\n256,192,9000,12,0,11000,0:0:0:0:\r\n";

    #[test]
    fn an_unedited_chart_is_written_back_byte_for_byte() {
        for text in [
            MAP,
            &MAP.replace("\r\n", "\n"),
            "osu file format v4\n[TimingPoints]\n0,500\n[HitObjects]\n256,192,500,1,0\n",
        ] {
            let c = Chart::parse(text);
            assert_eq!(c.to_text(), text);
        }
    }

    #[test]
    fn parses_the_two_owned_sections() {
        let c = Chart::parse(MAP);
        assert_eq!(c.timing.len(), 2);
        assert_eq!(c.objects.len(), 5);
        assert!(c.timing[0].is_red() && !c.timing[1].is_red());
        assert_eq!(c.timing[1].sv(), 2.0);
        assert!(c.timing[1].kiai());
        assert_eq!(c.objects[1].taiko(), Taiko::Kat);
        assert_eq!(c.objects[2].taiko(), Taiko::Kat);
        assert!(c.objects[2].big());
        assert_eq!(c.objects[3].taiko(), Taiko::Roll);
        assert_eq!(c.objects[3].length(), Some(70.0));
        assert_eq!(c.objects[4].taiko(), Taiko::Swell);
        assert_eq!(c.objects[4].spinner_end(), Some(11000.0));
    }

    #[test]
    fn an_edit_rewrites_only_what_changed() {
        let mut c = Chart::parse(MAP);
        let id = c.objects[0].id;
        c.object_mut(id).unwrap().time = 1010.0;
        c.insert_object(HitObject::hit(3000.0, false, false));
        let text = c.to_text();
        // Every other line, owned section or not, is the original byte for byte.
        let (old, new): (Vec<&str>, Vec<&str>) = (
            MAP.split_inclusive("\r\n").collect(),
            text.split_inclusive("\r\n").collect(),
        );
        assert_eq!(new.len(), old.len() + 1);
        assert!(new.contains(&"256,192,1010,1,0,0:0:0:0:\r\n"));
        assert!(new.contains(&"256,192,3000,1,0,0:0:0:0:\r\n"));
        for line in &old {
            if !line.starts_with("256,192,1000,") {
                assert!(new.contains(line), "lost {line:?}");
            }
        }
        // Still sorted by time.
        let times: Vec<f64> = Chart::parse(&text).objects.iter().map(|o| o.time).collect();
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    }

    #[test]
    fn old_format_timing_lines_are_not_padded_out() {
        let text = "osu file format v4\n[TimingPoints]\n0,500\n2000,-50,4,1,0,100\n[HitObjects]\n256,192,500,1,0\n";
        let mut c = Chart::parse(text);
        assert!(c.timing[0].is_red() && !c.timing[1].is_red());
        c.insert_object(HitObject::hit(900.0, false, false));
        let out = c.to_text();
        assert!(
            out.contains("\n0,500\n") && out.contains("\n2000,-50,4,1,0,100\n"),
            "{out}"
        );
    }

    #[test]
    fn a_gimmick_charts_odd_timing_lines_survive_an_edit() {
        // NaN and sub-millisecond beat lengths are what gimmick maps are made of.
        let text = "osu file format v14\n[TimingPoints]\n0,500,4,1,0,100,1,0\n0,NaN,4,1,0,100,0,0\n1000,0.000000001,4,1,0,100,1,8\n[HitObjects]\n256,192,500,1,0\n";
        let mut c = Chart::parse(text);
        c.insert_object(HitObject::hit(900.0, true, false));
        let out = c.to_text();
        for l in ["0,NaN,4,1,0,100,0,0\n", "1000,0.000000001,4,1,0,100,1,8\n"] {
            assert!(out.contains(l), "{out}");
        }
        assert!(c.validate().is_ok());
        assert!(Chart::parse(&out).timing[1].sv().is_finite());
    }

    #[test]
    fn same_time_timing_points_keep_their_file_order() {
        let mut c = Chart::parse(
            "osu file format v14\n[TimingPoints]\n1000,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,0\n",
        );
        c.insert_timing(TimingPoint::green(1000.0, 2.0));
        c.insert_timing(TimingPoint::green(1000.0, 3.0));
        let sv: Vec<f64> = c.timing.iter().map(TimingPoint::sv).collect();
        assert_eq!(sv, [1.0, 2.0, 3.0]);
        let back = Chart::parse(&c.to_text());
        let sv: Vec<f64> = back.timing.iter().map(TimingPoint::sv).collect();
        assert_eq!(sv, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn what_cant_be_written_is_refused() {
        let mut c = Chart::parse(MAP);
        c.insert_timing(TimingPoint::new(100.0, 0.0, false));
        assert!(c.validate().is_err());
        let mut c = Chart::parse(MAP);
        c.insert_timing(TimingPoint::new(100.0, 400.0, false));
        assert!(c.validate().unwrap_err().contains("positive"));
        let mut c = Chart::parse(MAP);
        c.timing.retain(|p| !p.is_red());
        c.dirty = true;
        assert!(c.validate().is_err());
        // Odd lines nobody touched are fine.
        assert!(
            Chart::parse("osu file format v14\n[TimingPoints]\n0,NaN,4,1,0,100,1,0\n")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn set_key_keeps_the_files_spacing_and_adds_missing_keys() {
        let mut c = Chart::parse(MAP);
        c.set_key("Metadata", "Version", "Inner");
        c.set_key("General", "AudioFilename", "song.ogg");
        c.set_key("Difficulty", "ApproachRate", "9");
        let out = c.to_text();
        assert!(
            out.contains("Version:Inner\r\n") && out.contains("AudioFilename: song.ogg\r\n"),
            "{out}"
        );
        let d = out.find("ApproachRate:9\r\n").expect("added");
        assert!(d > out.find("[Difficulty]").unwrap() && d < out.find("[Events]").unwrap());
        assert_eq!(
            Chart::parse(&out).get_key("Metadata", "Version").as_deref(),
            Some("Inner")
        );
    }

    #[test]
    fn numbers_are_written_the_way_osu_writes_them() {
        assert_eq!(fmt_num(1000.0), "1000");
        assert_eq!(fmt_num(-100.0), "-100");
        assert_eq!(fmt_num(333.3333333333333), "333.3333333333333");
        assert_eq!(fmt_num(0.000000001), "0.000000001");
        assert_eq!(fmt_num(-0.0001), "-0.0001");
    }

    #[test]
    fn the_editor_and_the_game_read_a_saved_chart_alike() {
        let mut c = Chart::parse(MAP);
        c.insert_object(HitObject::hit(3000.0, true, true));
        c.insert_object(HitObject::roll(5000.0, 140.0, false));
        c.insert_object(HitObject::swell(7000.0, 8000.0));
        let out = c.to_text();
        let map = crate::beatmap::tests::parse(&out);
        let kinds: Vec<_> = map.notes.iter().map(|n| n.kind).collect();
        use crate::beatmap::Kind::*;
        assert!(kinds.contains(&Roll) && kinds.contains(&Swell));
        let big = map.notes.iter().find(|n| n.time == 3000.0).unwrap();
        assert!(big.big && big.kind == Kat);
        let roll = map
            .notes
            .iter()
            .find(|n| n.kind == Roll && n.time == 5000.0)
            .unwrap();
        // 140 px at 1.4 SM is one beat (500 ms), at the 2x SV in force from 4000 half that; the
        // editor's own length arithmetic agrees with the game's.
        assert_eq!(roll.end_time - roll.time, 250.0);
        assert_eq!(
            HitObject::roll(5000.0, 140.0, false).end_time(1.4, 500.0, 2.0),
            5250.0
        );
        assert_eq!(
            HitObject::roll(5000.0, 140.0, false).end_time(1.4, 500.0, 1.0),
            5500.0
        );
    }

    #[test]
    fn a_file_on_disk_saves_through_a_checked_temp_file_with_one_backup() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-editor-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        std::fs::write(&path, MAP).unwrap();
        let mut c = Chart::read(&path).unwrap();
        c.insert_object(HitObject::hit(3000.0, false, false));
        c.save(&path, true).unwrap();
        c.insert_object(HitObject::hit(3500.0, false, false));
        c.save(&path, true).unwrap();
        let bak = std::fs::read_to_string(dir.join("map.osu.bak")).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(bak, MAP, "the backup is the first version, kept");
        assert_eq!(Chart::parse(&saved).objects.len(), 7);
        assert_eq!(leftovers, 2, "no temp file left behind");
    }

    /// Every map in `$TAIKO_CORPUS`: read and written back unedited it is the same bytes; and
    /// edited, then the edit undone, it is too; and with a note added the game reads the same
    /// notes plus the new one. Run with `--ignored`.
    #[test]
    #[ignore = "needs a folder of .osu files in TAIKO_CORPUS"]
    fn corpus_round_trips() {
        use super::super::history::{Command, History};
        let Some(dir) = std::env::var_os("TAIKO_CORPUS") else {
            panic!("set TAIKO_CORPUS to a folder of .osu files");
        };
        let (mut seen, mut lossy, mut unsorted) = (0, 0, 0);
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "osu") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let Ok(text) = String::from_utf8(bytes) else {
                lossy += 1;
                continue;
            };
            let name = path.display();
            let mut chart = Chart::parse(&text);
            assert_eq!(chart.to_text(), text, "{name}: unedited");
            if chart.objects.windows(2).any(|w| w[0].time > w[1].time) {
                unsorted += 1;
            }
            // Edit and undo: the file text comes back (a map whose objects weren't in time
            // order is sorted by any edit, which osu! does too, so only compare those sorted).
            let mut history = History::new();
            let before = chart.to_text();
            history.run(
                &mut chart,
                Command::InsertObjects(vec![HitObject::hit(123456.0, false, false)]),
            );
            let edited = chart.to_text();
            assert!(chart.validate().is_ok(), "{name}: {:?}", chart.validate());
            assert_eq!(
                Chart::parse(&edited).objects.len(),
                chart.objects.len(),
                "{name}"
            );
            history.undo(&mut chart);
            let sorted_before = chart.objects.windows(2).all(|w| w[0].time <= w[1].time);
            if sorted_before {
                let after = chart.to_text();
                if after != before {
                    let (a, b): (Vec<&str>, Vec<&str>) = (
                        before.split_inclusive('\n').collect(),
                        after.split_inclusive('\n').collect(),
                    );
                    let at = a.iter().zip(&b).position(|(x, y)| x != y);
                    panic!(
                        "{name}: edit then undo: {} vs {} lines, first difference at {at:?}: {:?} / {:?}",
                        a.len(),
                        b.len(),
                        at.map(|i| a[i]),
                        at.map(|i| b[i])
                    );
                }
            }
            // The game reads the edit: one note more, the others where they were.
            let (a, b) = (
                crate::beatmap::tests::parse(&text),
                crate::beatmap::tests::parse(&edited),
            );
            if a.meta.format_version >= 5 && !a.notes.is_empty() {
                assert_eq!(b.notes.len(), a.notes.len() + 1, "{name}");
            }
            seen += 1;
        }
        eprintln!(
            "{seen} maps round-tripped ({unsorted} with objects out of time order, {lossy} not UTF-8 skipped)"
        );
        assert!(seen > 0);
    }
}
