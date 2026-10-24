//! `.osu` parsing and conversion into taiko hit objects.
//!
//! Conversion follows osu!lazer's `TaikoBeatmapConverter` so that both native taiko
//! maps and osu!standard converts produce the same note timings as the official client.

use std::path::Path;

/// osu! applies this to SliderMultiplier for every taiko map (`LEGACY_TAIKO_VELOCITY_MULTIPLIER`).
const TAIKO_VELOCITY_MULTIPLIER: f64 = 1.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Don,
    Kat,
    Roll,
    Swell,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub time: f64,
    pub end_time: f64,
    pub kind: Kind,
    pub big: bool,
    /// Scroll speed of this note in osu!pixels per millisecond (before the user multiplier).
    pub velocity: f64,
    /// Drumroll tick spacing in ms.
    pub tick_spacing: f64,
    /// Hits needed to clear a swell.
    pub required_hits: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Barline {
    pub time: f64,
    pub velocity: f64,
}

#[derive(Clone, Copy, Debug)]
struct RawTimingPoint {
    time: f64,
    beat_length: f64,
    meter: u32,
    uninherited: bool,
    effects: u32,
    /// Hitsound volume, 0..1.
    volume: f32,
}

/// Effective control state from one timing point onward.
#[derive(Clone, Copy, Debug)]
pub struct ControlPoint {
    pub time: f64,
    /// Time of the uninherited (red) point the beats are counted from: SV points don't move it.
    pub beat_origin: f64,
    pub beat_length: f64,
    pub sv: f64,
    pub kiai: bool,
    /// Hitsound volume from this point on, 0..1.
    pub volume: f32,
}

impl ControlPoint {
    /// How far into the current beat map time `t` is, 0..1. Counted from the red line: green
    /// lines often sit mid-beat, and restarting the beat at them made the beat-driven mods
    /// (Pulse, Stutter, Zoom...) twitch on SV-heavy maps.
    pub fn beat_phase(&self, t: f64) -> f64 {
        ((t - self.beat_origin) / self.beat_length).rem_euclid(1.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub title: String,
    pub artist: String,
    pub creator: String,
    pub version: String,
    pub audio_file: String,
    pub preview_time: f64,
    pub audio_lead_in: f64,
    pub mode: u8,
    pub od: f64,
    /// HP drain rate (taiko: how fast the HP bar fills and drains).
    pub hp: f64,
    /// Background image file (relative to the map folder), from the [Events] section.
    pub background: Option<String>,
    pub slider_multiplier: f64,
    pub slider_tick_rate: f64,
    pub format_version: i32,
    /// osu! beatmap set ID from [Metadata] (missing on old or unsubmitted maps).
    pub set_id: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Beatmap {
    pub meta: Meta,
    pub notes: Vec<Note>,
    pub barlines: Vec<Barline>,
    pub control_points: Vec<ControlPoint>,
}

impl Meta {
    pub fn is_playable(&self) -> bool {
        self.mode == 0 || self.mode == 1
    }
}

/// osu!'s `IBeatmapDifficultyInfo.DifficultyRange`.
pub fn difficulty_range(d: f64, min: f64, mid: f64, max: f64) -> f64 {
    if d > 5.0 {
        mid + (max - mid) * (d - 5.0) / 5.0
    } else if d < 5.0 {
        mid - (mid - min) * (5.0 - d) / 5.0
    } else {
        mid
    }
}

fn kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim(), v.trim()))
}

/// Has Cyrillic and nothing the font can't draw.
fn cyrillic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'А'..='я' | 'Ё' | 'ё')) && s.chars().all(crate::font::drawable)
}

fn num<T: std::str::FromStr>(s: Option<&str>) -> Option<T> {
    s?.trim().parse().ok()
}

/// Reads only the header sections (General/Metadata/Difficulty), stopping before events, timing
/// and hit objects, so scanning a library of tens of thousands of maps reads a few KB per file.
pub fn read_meta_file(path: &Path) -> Option<Meta> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let mut text = String::new();
    let mut line = Vec::new();
    let mut background = None;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        let l = String::from_utf8_lossy(&line);
        let t = l.trim();
        if t == "[Events]" {
            background = read_background(&mut reader);
            break;
        }
        if t == "[TimingPoints]" || t == "[HitObjects]" {
            break;
        }
        text.push_str(&l);
        // Malformed file without section headers: don't read the whole thing.
        if text.len() > 64 * 1024 {
            break;
        }
    }
    let mut meta = parse_sections(&text, true).0;
    meta.background = background;
    Some(meta)
}

/// The background is the first `0,0,"file",x,y` event, right at the top of [Events].
fn read_background(reader: &mut impl std::io::BufRead) -> Option<String> {
    let mut line = Vec::new();
    for _ in 0..400 {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            return None;
        }
        let l = String::from_utf8_lossy(&line);
        let t = l.trim();
        if t.starts_with('[') {
            return None;
        }
        if let Some(rest) = t.strip_prefix("0,0,") {
            let name = rest.split(',').next()?.trim().trim_matches('"');
            return (!name.is_empty()).then(|| name.to_string());
        }
    }
    None
}

/// How much earlier osu! plays a map's music against its note times than the music's exact
/// samples. Maps are timed by ear in osu!, so played sample-exact (as here) the music came late:
/// in 79 taiko maps (MP3 with and without a LAME header, OGG alike) the drum hits landed a median
/// 23 ms after their notes, against 0 for a click track. Loaded maps move this much later, so an
/// offset calibrated on the click track fits real maps too.
pub const OSU_TIMING_MS: f64 = 23.0;

pub fn load(path: &Path) -> Result<Beatmap, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (meta, body) = parse_sections(&text, false);
    if !meta.is_playable() {
        return Err(format!("unsupported mode {} (only osu!standard converts and taiko)", meta.mode));
    }
    let mut map = Beatmap { meta, notes: Vec::new(), barlines: Vec::new(), control_points: Vec::new() };
    convert(&mut map, body);
    if map.notes.is_empty() {
        return Err("beatmap has no hit objects".into());
    }
    map.shift(OSU_TIMING_MS);
    Ok(map)
}

impl Beatmap {
    /// Moves every map time (notes, barlines, timing) `ms` later. Audio positions (the preview
    /// point) and durations stay.
    pub fn shift(&mut self, ms: f64) {
        for n in &mut self.notes {
            n.time += ms;
            n.end_time += ms;
        }
        for b in &mut self.barlines {
            b.time += ms;
        }
        for cp in &mut self.control_points {
            cp.time += ms;
            cp.beat_origin += ms;
        }
    }
}

struct Body<'a> {
    timing: Vec<RawTimingPoint>,
    objects: Vec<&'a str>,
}

fn parse_sections(text: &str, header_only: bool) -> (Meta, Body<'_>) {
    let mut meta = Meta { slider_multiplier: 1.4, slider_tick_rate: 1.0, od: 5.0, hp: 5.0, format_version: 14, ..Default::default() };
    let mut body = Body { timing: Vec::new(), objects: Vec::new() };
    let mut section = "";
    let mut od_seen = false;
    let mut hp = 5.0;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if i == 0 {
            if let Some(v) = trimmed.trim_start_matches('\u{feff}').strip_prefix("osu file format v") {
                meta.format_version = v.trim().parse().unwrap_or(14);
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = &trimmed[1..trimmed.len() - 1];
            if header_only && (section == "Events" || section == "TimingPoints" || section == "HitObjects") {
                break;
            }
            continue;
        }
        match section {
            "General" => match kv(trimmed) {
                Some(("AudioFilename", v)) => meta.audio_file = v.to_owned(),
                Some(("AudioLeadIn", v)) => meta.audio_lead_in = v.parse().unwrap_or(0.0),
                Some(("PreviewTime", v)) => meta.preview_time = v.parse().unwrap_or(-1.0),
                Some(("Mode", v)) => meta.mode = v.parse().unwrap_or(0),
                _ => {}
            },
            "Metadata" => match kv(trimmed) {
                Some(("Title", v)) => meta.title = v.to_owned(),
                Some(("Artist", v)) => meta.artist = v.to_owned(),
                // Russian titles in Cyrillic (the font draws it); other scripts stay romanised.
                Some(("TitleUnicode", v)) if cyrillic(v) => meta.title = v.to_owned(),
                Some(("ArtistUnicode", v)) if cyrillic(v) => meta.artist = v.to_owned(),
                Some(("Creator", v)) => meta.creator = v.to_owned(),
                Some(("Version", v)) => meta.version = v.to_owned(),
                Some(("BeatmapSetID", v)) => meta.set_id = v.trim().parse().ok().filter(|&id: &u32| id > 0),
                _ => {}
            },
            "Difficulty" => match kv(trimmed) {
                Some(("OverallDifficulty", v)) => {
                    meta.od = v.parse().unwrap_or(5.0);
                    od_seen = true;
                }
                Some(("HPDrainRate", v)) => hp = v.parse().unwrap_or(5.0),
                Some(("SliderMultiplier", v)) => meta.slider_multiplier = v.parse().unwrap_or(1.4),
                Some(("SliderTickRate", v)) => meta.slider_tick_rate = v.parse().unwrap_or(1.0),
                _ => {}
            },
            "TimingPoints" => {
                let mut f = trimmed.split(',');
                let (Some(time), Some(beat_length)) = (num::<f64>(f.next()), num::<f64>(f.next())) else { continue };
                let meter = num::<u32>(f.next()).filter(|&m| m > 0).unwrap_or(4);
                let _sample_set = f.next();
                let _sample_index = f.next();
                // Very old maps have no volume (full); osu!'s editor goes no lower than 5%.
                let volume = num::<f32>(f.next()).map(|v| (v / 100.0).clamp(0.05, 1.0)).unwrap_or(1.0);
                // Old formats have no uninherited field: a negative beat length marks an SV point.
                let uninherited = num::<i32>(f.next()).map(|v| v != 0).unwrap_or(beat_length >= 0.0);
                let effects = num::<u32>(f.next()).unwrap_or(0);
                body.timing.push(RawTimingPoint { time, beat_length, meter, uninherited, effects, volume });
            }
            "HitObjects" => body.objects.push(trimmed),
            _ => {}
        }
    }
    // Very old maps have no OverallDifficulty; osu! falls back to HP.
    if !od_seen {
        meta.od = hp;
    }
    meta.hp = hp;
    (meta, body)
}

fn build_control_points(raw: &mut [RawTimingPoint]) -> Vec<ControlPoint> {
    // Stable sort keeps file order for simultaneous points (inherited after uninherited).
    raw.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut out: Vec<ControlPoint> = Vec::with_capacity(raw.len());
    let first_red = raw.iter().find(|p| p.uninherited);
    let mut beat_length = first_red.map(|p| p.beat_length).unwrap_or(500.0);
    let mut beat_origin = first_red.map(|p| p.time).unwrap_or(0.0);
    let mut sv;
    for p in raw.iter() {
        if p.uninherited {
            if p.beat_length > 0.0 {
                beat_length = p.beat_length.clamp(6.0, 60000.0);
            }
            beat_origin = p.time;
            sv = 1.0;
        } else if p.beat_length < 0.0 {
            sv = (-100.0 / p.beat_length).clamp(0.1, 10.0);
        } else {
            sv = 1.0;
        }
        let cp = ControlPoint { time: p.time, beat_origin, beat_length, sv, kiai: p.effects & 1 != 0, volume: p.volume };
        match out.last_mut() {
            Some(last) if last.time == p.time => *last = cp,
            _ => out.push(cp),
        }
    }
    if out.is_empty() {
        out.push(ControlPoint { time: 0.0, beat_origin: 0.0, beat_length: 500.0, sv: 1.0, kiai: false, volume: 1.0 });
    }
    out
}

pub fn control_point_at(points: &[ControlPoint], time: f64) -> &ControlPoint {
    let idx = points.partition_point(|p| p.time <= time);
    &points[idx.saturating_sub(1)]
}

fn convert(map: &mut Beatmap, mut body: Body) {
    // osu! shifts every timestamp of pre-v5 maps by 24ms.
    let version_offset = if map.meta.format_version < 5 { 24.0 } else { 0.0 };
    for p in &mut body.timing {
        p.time += version_offset;
    }
    let cps = build_control_points(&mut body.timing);
    let meta = &map.meta;
    let native = meta.mode == 1;
    let sm = meta.slider_multiplier * TAIKO_VELOCITY_MULTIPLIER;
    let tick_rate = meta.slider_tick_rate;
    let velocity_at = |t: f64| {
        let cp = control_point_at(&cps, t);
        sm * 100.0 * cp.sv / cp.beat_length
    };
    let swell_hit_multiplier = difficulty_range(meta.od, 3.0, 5.0, 7.5) * 1.65;

    let mut notes = Vec::with_capacity(body.objects.len());
    for line in &body.objects {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 5 {
            continue;
        }
        let Ok(time) = f[2].trim().parse::<f64>() else { continue };
        let time = time + version_offset;
        let ty: u32 = f[3].trim().parse().unwrap_or(1);
        let sound: u32 = f[4].trim().parse().unwrap_or(0);
        let velocity = velocity_at(time);

        if ty & 2 != 0 && f.len() >= 8 {
            // Slider: drumroll, or a stream of hits for short standard sliders.
            let spans = f[6].trim().parse::<u32>().unwrap_or(1).max(1);
            let length = f[7].trim().parse::<f64>().unwrap_or(0.0).max(0.0);
            let cp = control_point_at(&cps, time);
            let distance = length * TAIKO_VELOCITY_MULTIPLIER * spans as f64;
            let sv_beat_length = cp.beat_length / cp.sv;
            let taiko_velocity = 100.0 * sm;
            let duration = (distance / taiko_velocity * sv_beat_length).floor();

            let mut split = None;
            if !native {
                let osu_velocity = taiko_velocity * (1000.0 / sv_beat_length);
                let bl = if meta.format_version >= 8 { cp.beat_length } else { sv_beat_length };
                let tick_spacing = (bl / tick_rate).min(duration / spans as f64);
                if tick_spacing > 0.0 && distance / osu_velocity * 1000.0 < 2.0 * bl {
                    split = Some(tick_spacing);
                }
            }

            if let Some(spacing) = split {
                let node_sounds: Vec<u32> = f
                    .get(8)
                    .map(|s| s.split('|').filter_map(|x| x.trim().parse().ok()).collect())
                    .filter(|v: &Vec<u32>| !v.is_empty())
                    .unwrap_or_else(|| vec![sound; spans as usize + 1]);
                let mut t = time;
                let mut i = 0;
                while t <= time + duration + spacing / 8.0 {
                    let s = node_sounds[i % node_sounds.len()];
                    notes.push(hit(t, s, velocity_at(t)));
                    i += 1;
                    t += spacing;
                }
            } else {
                let roll_tick_rate = if tick_rate == 3.0 { 3.0 } else { 4.0 };
                notes.push(Note {
                    time,
                    end_time: time + duration,
                    kind: Kind::Roll,
                    big: sound & 4 != 0,
                    velocity,
                    tick_spacing: cp.beat_length / roll_tick_rate,
                    required_hits: 0,
                });
            }
        } else if ty & 8 != 0 {
            let end = f.get(5).and_then(|e| e.trim().parse::<f64>().ok()).map(|e| e + version_offset).unwrap_or(time).max(time);
            let required = ((end - time) / 1000.0 * swell_hit_multiplier).max(1.0) as u32;
            notes.push(Note {
                time,
                end_time: end,
                kind: Kind::Swell,
                big: false,
                velocity,
                tick_spacing: 0.0,
                required_hits: required,
            });
        } else if ty & 1 != 0 {
            notes.push(hit(time, sound, velocity));
        }
    }
    notes.sort_by(|a, b| a.time.total_cmp(&b.time));

    let end = notes.last().map(|n| n.end_time).unwrap_or(0.0);
    map.barlines = build_barlines(&body.timing, end, &velocity_at);
    map.notes = notes;
    map.control_points = cps;
}

/// The offset-calibration chart: one don on every beat from `first_beat` to `beats`, matching
/// the clicks of `audio::metronome` with the same `beat_ms`.
pub fn calibration_map(beat_ms: f64, first_beat: u32, beats: u32) -> Beatmap {
    let meta = Meta {
        title: "Offset calibration".into(),
        artist: "taiko-rs".into(),
        creator: "taiko-rs".into(),
        version: format!("{:.0} BPM click track", 60000.0 / beat_ms),
        mode: 1,
        // OD 0 gives the widest windows, so even a badly-off offset still registers as hits.
        od: 0.0,
        hp: 5.0,
        slider_multiplier: 1.4,
        slider_tick_rate: 1.0,
        format_version: 14,
        ..Default::default()
    };
    let velocity = meta.slider_multiplier * TAIKO_VELOCITY_MULTIPLIER * 100.0 / beat_ms;
    let notes = (first_beat..beats).map(|b| hit(b as f64 * beat_ms, 0, velocity)).collect();
    let barlines = (0..beats).step_by(4).map(|b| Barline { time: b as f64 * beat_ms, velocity }).collect();
    let control_points = vec![ControlPoint { time: 0.0, beat_origin: 0.0, beat_length: beat_ms, sv: 1.0, kiai: false, volume: 1.0 }];
    Beatmap { meta, notes, barlines, control_points }
}

fn hit(time: f64, sound: u32, velocity: f64) -> Note {
    let kat = sound & (2 | 8) != 0;
    Note {
        time,
        end_time: time,
        kind: if kat { Kind::Kat } else { Kind::Don },
        big: sound & 4 != 0,
        velocity,
        tick_spacing: 0.0,
        required_hits: 0,
    }
}

fn build_barlines(raw: &[RawTimingPoint], end: f64, velocity_at: &dyn Fn(f64) -> f64) -> Vec<Barline> {
    let timing: Vec<&RawTimingPoint> = raw.iter().filter(|p| p.uninherited && p.beat_length > 0.0).collect();
    let mut out = Vec::new();
    for (i, p) in timing.iter().enumerate() {
        let until = timing.get(i + 1).map(|n| n.time).unwrap_or(end + 1.0);
        let step = p.beat_length * p.meter as f64;
        if step < 1.0 {
            continue;
        }
        let mut t = p.time;
        let mut first = true;
        // Stop just short of the next timing point so barlines don't double up there.
        while t < until - 1.0 {
            if !(first && p.effects & 8 != 0) {
                out.push(Barline { time: t, velocity: velocity_at(t) });
            }
            first = false;
            t += step;
            if out.len() > 100_000 {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 0

[Difficulty]
OverallDifficulty:5
SliderMultiplier:1.4
SliderTickRate:1

[TimingPoints]
0,500,4,1,0,100,1,0
4000,-50,4,1,0,100,0,1

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,1500,1,2,0:0:0:0:
256,192,2000,1,12,0:0:0:0:
256,192,2500,2,0,L|356:192,1,70
256,192,5000,2,0,L|456:192,2,560
256,192,9000,12,0,11000,0:0:0:0:
";

    fn parse(text: &str) -> Beatmap {
        let (meta, body) = parse_sections(text, false);
        let mut map = Beatmap { meta, notes: Vec::new(), barlines: Vec::new(), control_points: Vec::new() };
        convert(&mut map, body);
        map
    }

    #[test]
    fn loaded_maps_move_to_where_osu_plays_their_music() {
        let text = "osu file format v14\n\n[General]\nAudioFilename: a.mp3\nMode: 1\n\n[Difficulty]\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n1000,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,2,0:0:0:0:\n";
        let dir = std::env::temp_dir().join(format!("taiko-rs-shift-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        std::fs::write(&path, text).unwrap();
        let (file, raw) = (load(&path).unwrap(), parse(text));
        let _ = std::fs::remove_dir_all(&dir);
        for (a, b) in file.notes.iter().zip(&raw.notes) {
            assert_eq!(a.time, b.time + OSU_TIMING_MS);
        }
        assert_eq!(file.control_points[0].beat_origin, raw.control_points[0].beat_origin + OSU_TIMING_MS);
        assert_eq!(file.barlines[0].time, raw.barlines[0].time + OSU_TIMING_MS);
        // Beat phase is unchanged at the shifted note times.
        let (cf, cr) = (&file.control_points[0], &raw.control_points[0]);
        assert!((cf.beat_phase(file.notes[1].time) - cr.beat_phase(raw.notes[1].time)).abs() < 1e-9);
    }

    #[test]
    fn converts_standard_objects() {
        let map = parse(MAP);
        let kinds: Vec<(f64, Kind, bool)> = map.notes.iter().map(|n| (n.time, n.kind, n.big)).collect();
        // Circles: plain = don, whistle = kat, clap+finish = big kat.
        assert_eq!(kinds[0], (1000.0, Kind::Don, false));
        assert_eq!(kinds[1], (1500.0, Kind::Kat, false));
        assert_eq!(kinds[2], (2000.0, Kind::Kat, true));
        // A short slider (70px at 1.4 SM = half a beat) splits into hits every beat/tickrate, capped by span length.
        assert_eq!(kinds[3].1, Kind::Don);
        assert_eq!(kinds[3].0, 2500.0);
        // 70px is a quarter beat-length at 1.4 SM: too short for a roll, so it splits into hits 250ms apart.
        assert_eq!(map.notes.iter().filter(|n| n.time >= 2500.0 && n.time < 3000.0).count(), 2);
        // The long slider under 2x SV becomes a drumroll: 560*2 px / (140 px/beat * 2 SV) = 4 beats = 2000ms.
        let roll = map.notes.iter().find(|n| n.kind == Kind::Roll).expect("drumroll");
        assert_eq!(roll.time, 5000.0);
        assert_eq!(roll.end_time, 7000.0);
        assert_eq!(roll.tick_spacing, 125.0);
        let swell = map.notes.iter().find(|n| n.kind == Kind::Swell).expect("swell");
        assert_eq!(swell.end_time, 11000.0);
        // OD5: 5 * 1.65 hits per second over 2 seconds.
        assert_eq!(swell.required_hits, 16);
        // Velocity doubles under the 2x SV point.
        assert!((roll.velocity / map.notes[0].velocity - 2.0).abs() < 1e-9);
    }

    #[test]
    fn hit_windows_match_lazer() {
        assert_eq!(difficulty_range(5.0, 50.0, 35.0, 20.0), 35.0);
        assert_eq!(difficulty_range(10.0, 50.0, 35.0, 20.0), 20.0);
        assert_eq!(difficulty_range(0.0, 120.0, 80.0, 50.0), 120.0);
    }

    #[test]
    fn old_format_negative_beat_length_is_an_sv_change() {
        // v4-era timing points: no uninherited flag, inherited points marked by negative beat length.
        let map = parse("osu file format v4\n[Difficulty]\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100\n2000,-50,4,1,0,100\n[HitObjects]\n256,192,1000,1,0\n256,192,3000,1,0\n");
        assert_eq!(map.notes.len(), 2);
        assert!((map.notes[1].velocity / map.notes[0].velocity - 2.0).abs() < 1e-9);
        // Pre-v5 maps are shifted by 24 ms, like osu! does.
        assert_eq!(map.notes[0].time, 1024.0);
    }

    #[test]
    fn timing_points_carry_the_hitsound_volume() {
        // Red line at 70%, green lines at 40% and (an editor-impossible) 0%, which osu! floors at 5%.
        let map = parse("osu file format v14\n[TimingPoints]\n0,500,4,1,0,70,1,0\n1000,-100,4,1,0,40,0,0\n2000,-100,4,1,0,0,0,0\n[HitObjects]\n256,192,500,1,0\n");
        let at = |t| control_point_at(&map.control_points, t).volume;
        assert_eq!((at(500.0), at(1500.0), at(2500.0)), (0.7, 0.4, 0.05));
        // No volume field at all (very old maps): full volume.
        let old = parse("osu file format v3\n[TimingPoints]\n0,500\n[HitObjects]\n256,192,500,1,0\n");
        assert_eq!(control_point_at(&old.control_points, 600.0).volume, 1.0);
    }

    #[test]
    fn beats_count_from_the_red_line_not_green_ones() {
        // Green lines mid-beat keep the beat; a new red line restarts it.
        let map = parse("osu file format v14\n[TimingPoints]\n100,500,4,1,0,100,1,0\n1230,-50,4,1,0,100,0,0\n3000,400,4,1,0,100,1,0\n3170,-80,4,1,0,100,0,0\n[HitObjects]\n256,192,500,1,0\n");
        let phase = |t| control_point_at(&map.control_points, t).beat_phase(t);
        assert!((phase(1350.0) - 0.5).abs() < 1e-9);
        assert!((phase(3300.0) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn short_spinner_line_does_not_panic() {
        let map = parse("osu file format v14\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,8,0\n256,192,2000,1,0\n");
        assert_eq!(map.notes.len(), 2);
        assert_eq!(map.notes[0].kind, Kind::Swell);
    }

    #[test]
    fn barlines_follow_meter() {
        let map = parse(MAP);
        assert_eq!(map.barlines[0].time, 0.0);
        assert_eq!(map.barlines[1].time, 2000.0);
    }
}
