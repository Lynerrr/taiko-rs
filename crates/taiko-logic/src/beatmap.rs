//! `.osu` parsing and conversion into taiko hit objects.
//!
//! Conversion follows osu!lazer's `TaikoBeatmapConverter` so that both native taiko
//! maps and osu!standard converts produce the same note timings as the official client.

use std::path::Path;

/// osu! applies this to SliderMultiplier for every taiko map (`LEGACY_TAIKO_VELOCITY_MULTIPLIER`).
const TAIKO_VELOCITY_MULTIPLIER: f64 = 1.4;
/// The longest a roll or a swell lasts. Gimmick maps write sliders trillions of pixels long
/// (osu!team's FM Synthesis Experiment, set 2608578): a roll that long ended past anything a
/// number of sections or ticks can be counted to, and the game crashed on picking the map.
const LONGEST_NOTE_MS: f64 = 3_600_000.0;

/// A roll's or swell's length, as one that can be played: none when it works out to nothing
/// sensible (NaN from a NaN slider velocity), at most `LONGEST_NOTE_MS`.
fn playable_length(ms: f64) -> f64 {
    if ms.is_finite() {
        ms.clamp(0.0, LONGEST_NOTE_MS)
    } else {
        0.0
    }
}

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
    /// A fake slider (a drumroll with a negative length, a gimmick map staple): how many ms of
    /// scroll its body reaches right of its head, and its cap left of it. Only drawn; it plays
    /// as the zero-length roll it is. 0 for everything else.
    pub fake_len: f64,
    /// A fake slider whose cap lands under a ms ahead of its head (-0.0001): the head gets there
    /// first, so head and cap pass through the drum and the body fades instead of vanishing.
    pub fake_fades: bool,
    /// A fake slider's |length| × spans, as written: where a pop-up one stands (see game.rs).
    /// `fake_len` can't give it back: it went through the clamped beat length.
    pub fake_length: f64,
    /// A fake slider: written under 0.1 long, any negative included. Autoplay leaves it alone: a
    /// press for its zero-length roll went to the next note, a miss on a kat (owner,
    /// 2026-10-28).
    pub tiny: bool,
    /// A don/kat right before a slider of length NaN: stable can't hit it (owner, 2026-10-28).
    pub unhittable: bool,
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

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
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
    /// osu! beatmap ID of the difficulty (missing on old, unsubmitted or home-made ones).
    pub beatmap_id: Option<u32>,
    /// The genres its tags name (`genres::of`).
    #[serde(default)]
    pub genres: u32,
    /// The `.tko` format version of a map that uses client gimmicks; 0 for a plain chart.
    #[serde(default)]
    pub tko_version: u32,
}

#[derive(Clone, Debug)]
pub struct Beatmap {
    pub meta: Meta,
    pub notes: Vec<Note>,
    pub barlines: Vec<Barline>,
    pub control_points: Vec<ControlPoint>,
    /// Scroll velocity (osu!px per ms) at the first timing point: what stable's PreEmpt, and so
    /// when spinners show, comes from (see game.rs).
    pub first_velocity: f64,
    /// The client gimmick of a `.tko` map (visual only; its times are the chart's, before the
    /// shift `from_text` applies).
    pub gimmick: Option<std::sync::Arc<crate::tko::gimmick::Gimmick>>,
    /// A `.tko` map's fake notes: drawn like notes, never judged (see `Gimmick::fakes`).
    pub fakes: Vec<Note>,
}

impl Meta {
    pub fn is_playable(&self) -> bool {
        self.mode == 0 || self.mode == 1
    }
}

/// osu!'s `IBeatmapDifficultyInfo.DifficultyRange`.
/// Map time play starts at: two seconds before the first note, or the song's start, or the
/// map's audio lead-in before it, whichever is earliest. A `.tko` map's effects can start this
/// early, so the playfield is set before the song begins.
pub fn play_start(first_note: f64, audio_lead_in: f64) -> f64 {
    (first_note - 2000.0).min(0.0).min(-audio_lead_in)
}

pub fn difficulty_range(d: f64, min: f64, mid: f64, max: f64) -> f64 {
    if d > 5.0 {
        mid + (max - mid) * (d - 5.0) / 5.0
    } else if d < 5.0 {
        mid - (mid - min) * (5.0 - d) / 5.0
    } else {
        mid
    }
}

/// The OD a map plays like at `rate` (1.5 for DT): the great window shrinks with the speed, as
/// it is timed in the song, and this is the OD whose window is that wide in real time. Above 10
/// when faster, below the map's when slower, as osu! shows it.
pub fn od_at_rate(od: f64, rate: f64) -> f64 {
    od_of_great(great_ms(od) / rate.max(0.01))
}

/// Where the hit windows leave osu!'s straight lines, the OD of a 10 ms great. Those lines reach
/// 0 ms at OD 16.67 (great) and 18.33 (ok), and would then run below it.
const WINDOW_BEND_OD: f64 = 40.0 / 3.0;

/// A hit window in ms, from its widths at OD 0, 5 and 10. Past `WINDOW_BEND_OD` it keeps shrinking
/// at the rate it had there, as a share of what's left: it joins the line without a corner and,
/// however high the OD, never gets to zero.
pub fn window_ms(od: f64, min: f64, mid: f64, max: f64) -> f64 {
    if od <= WINDOW_BEND_OD {
        return difficulty_range(od, min, mid, max);
    }
    let at_bend = difficulty_range(WINDOW_BEND_OD, min, mid, max);
    let per_od = (mid - max) / 5.0;
    at_bend * (-(od - WINDOW_BEND_OD) * per_od / at_bend).exp()
}

/// A hit window as osu!(lazer) judges it, the way stable did: whole ms (stable's clock and
/// windows were ints, compared with <), less half a ms, compared with <=. Past `WINDOW_BEND_OD`
/// it bends as `window_ms` does, from the rounded width there, so it still never gets to zero.
pub fn judged_window_ms(od: f64, min: f64, mid: f64, max: f64) -> f64 {
    // A width that is a whole number on paper can come out a hair under it (29.000000000000004
    // or 28.999999999999996): it's still that whole number.
    let line = |d: f64| (difficulty_range(d, min, mid, max) + 1e-9).floor() - 0.5;
    if od <= WINDOW_BEND_OD {
        return line(od);
    }
    let at_bend = line(WINDOW_BEND_OD);
    let per_od = (mid - max) / 5.0;
    at_bend * (-(od - WINDOW_BEND_OD) * per_od / at_bend).exp()
}

/// The great window at `od`, in ms.
pub fn great_ms(od: f64) -> f64 {
    window_ms(od, 50.0, 35.0, 20.0)
}

/// The OD of a great window of `great` ms: `great_ms` the other way round.
fn od_of_great(great: f64) -> f64 {
    let at_bend = great_ms(WINDOW_BEND_OD);
    if great >= at_bend {
        // 50 - 3 * OD over the whole range.
        (50.0 - great) / 3.0
    } else {
        WINDOW_BEND_OD + (at_bend / great).ln() * at_bend / 3.0
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
/// A number in a map's header, else `default`. `nan`, `inf` and `1e999` all parse, but mean nothing
/// here, and a header holding one can't go through the library's cache (JSON has no such numbers).
fn number(v: &str, default: f64) -> f64 {
    v.parse()
        .ok()
        .filter(|n: &f64| n.is_finite())
        .unwrap_or(default)
}

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

/// The header of a `.osu` already in memory (one inside an `.osz`).
pub fn read_meta(text: &str) -> Meta {
    parse_sections(text, true).0
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
    if is_tko(path) && crate::tko::format::version_of(&text).is_none() {
        return Err(crate::tko::format::Error::MissingVersion.to_string());
    }
    from_text(&text)
}

/// Whether `path` is a `.tko` map.
pub fn is_tko(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tko"))
}

/// A map already in memory, converted exactly as `load` does (the editor's preview plays what the
/// chart says right now).
pub fn from_text(text: &str) -> Result<Beatmap, String> {
    // A map that says it is a .tko brings its gimmick, or is refused (a newer format than this
    // game knows, a broken section).
    let gimmick = if crate::tko::format::version_of(text).is_some() {
        let parsed = crate::tko::format::read(text).map_err(|e| e.to_string())?;
        (!parsed.gimmick.is_empty()).then(|| std::sync::Arc::new(parsed.gimmick))
    } else {
        None
    };
    let (meta, body) = parse_sections(text, false);
    if !meta.is_playable() {
        return Err(format!(
            "unsupported mode {} (only osu!standard converts and taiko)",
            meta.mode
        ));
    }
    let mut map = Beatmap {
        meta,
        notes: Vec::new(),
        barlines: Vec::new(),
        control_points: Vec::new(),
        first_velocity: 0.0,
        gimmick,
        fakes: Vec::new(),
    };
    convert(&mut map, body);
    if map.notes.is_empty() {
        return Err("beatmap has no hit objects".into());
    }
    map.shift(OSU_TIMING_MS);
    Ok(map)
}

/// The chart in `text` converted as it's written: no checks, no gimmick, and not moved to where
/// osu! plays its music (`from_text` does all that). For tests and tools.
pub fn parse_raw(text: &str) -> Beatmap {
    let (meta, body) = parse_sections(text, false);
    let mut map = Beatmap {
        meta,
        notes: Vec::new(),
        barlines: Vec::new(),
        control_points: Vec::new(),
        first_velocity: 0.0,
        gimmick: None,
        fakes: Vec::new(),
    };
    convert(&mut map, body);
    map
}

impl Beatmap {
    /// A map song select shows no playfield preview of: rolls or swells it made endless
    /// (`playable_length` cut them to the longest), or more notes than any real map. Its preview
    /// took the whole client down to 1 fps (osu!team's FM Synthesis Experiment).
    pub fn too_wild_to_preview(&self) -> bool {
        self.notes.len() > 100_000
            || self
                .notes
                .iter()
                .any(|n| n.end_time - n.time >= LONGEST_NOTE_MS || n.fake_len >= LONGEST_NOTE_MS)
    }

    /// Moves every map time (notes, barlines, timing) `ms` later. Audio positions (the preview
    /// point) and durations stay.
    pub fn shift(&mut self, ms: f64) {
        for n in self.notes.iter_mut().chain(&mut self.fakes) {
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
    let mut meta = Meta {
        slider_multiplier: 1.4,
        slider_tick_rate: 1.0,
        od: 5.0,
        hp: 5.0,
        format_version: 14,
        ..Default::default()
    };
    let mut body = Body {
        timing: Vec::new(),
        objects: Vec::new(),
    };
    let mut section = "";
    let mut od_seen = false;
    let mut hp = 5.0;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if i == 0 {
            if let Some(v) = trimmed
                .trim_start_matches('\u{feff}')
                .strip_prefix("osu file format v")
            {
                meta.format_version = v.trim().parse().unwrap_or(14);
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = &trimmed[1..trimmed.len() - 1];
            if header_only
                && (section == "Events" || section == "TimingPoints" || section == "HitObjects")
            {
                break;
            }
            continue;
        }
        match section {
            "General" => match kv(trimmed) {
                Some(("AudioFilename", v)) => meta.audio_file = v.to_owned(),
                Some(("AudioLeadIn", v)) => meta.audio_lead_in = number(v, 0.0),
                Some(("PreviewTime", v)) => meta.preview_time = number(v, -1.0),
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
                Some(("Tags", v)) => meta.genres = crate::genres::of(v),
                Some(("TkoVersion", v)) => meta.tko_version = v.trim().parse().unwrap_or(0),
                Some(("BeatmapSetID", v)) => {
                    meta.set_id = v.trim().parse().ok().filter(|&id: &u32| id > 0)
                }
                Some(("BeatmapID", v)) => {
                    meta.beatmap_id = v.trim().parse().ok().filter(|&id: &u32| id > 0)
                }
                _ => {}
            },
            "Difficulty" => match kv(trimmed) {
                Some(("OverallDifficulty", v)) => {
                    meta.od = number(v, 5.0);
                    od_seen = true;
                }
                Some(("HPDrainRate", v)) => hp = number(v, 5.0),
                Some(("SliderMultiplier", v)) => meta.slider_multiplier = number(v, 1.4),
                Some(("SliderTickRate", v)) => meta.slider_tick_rate = number(v, 1.0),
                _ => {}
            },
            "TimingPoints" => {
                let mut f = trimmed.split(',');
                let (Some(time), Some(beat_length)) = (num::<f64>(f.next()), num::<f64>(f.next()))
                else {
                    continue;
                };
                let meter = num::<u32>(f.next()).filter(|&m| m > 0).unwrap_or(4);
                let _sample_set = f.next();
                let _sample_index = f.next();
                // Very old maps have no volume (full); osu!'s editor goes no lower than 5%.
                let volume = num::<f32>(f.next())
                    .map(|v| (v / 100.0).clamp(0.05, 1.0))
                    .unwrap_or(1.0);
                // Old formats have no uninherited field: a negative beat length marks an SV point.
                let uninherited = num::<i32>(f.next())
                    .map(|v| v != 0)
                    .unwrap_or(beat_length >= 0.0);
                let effects = num::<u32>(f.next()).unwrap_or(0);
                body.timing.push(RawTimingPoint {
                    time,
                    beat_length,
                    meter,
                    uninherited,
                    effects,
                    volume,
                });
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
        let cp = ControlPoint {
            time: p.time,
            beat_origin,
            beat_length,
            sv,
            kiai: p.effects & 1 != 0,
            volume: p.volume,
        };
        match out.last_mut() {
            Some(last) if last.time == p.time => *last = cp,
            _ => out.push(cp),
        }
    }
    if out.is_empty() {
        out.push(ControlPoint {
            time: 0.0,
            beat_origin: 0.0,
            beat_length: 500.0,
            sv: 1.0,
            kiai: false,
            volume: 1.0,
        });
    }
    out
}

pub fn control_point_at(points: &[ControlPoint], time: f64) -> &ControlPoint {
    let idx = points.partition_point(|p| p.time <= time);
    &points[idx.saturating_sub(1)]
}

fn convert(map: &mut Beatmap, mut body: Body) {
    // osu! shifts every timestamp of pre-v5 maps by 24ms.
    let version_offset = if map.meta.format_version < 5 {
        24.0
    } else {
        0.0
    };
    for p in &mut body.timing {
        p.time += version_offset;
    }
    let cps = build_control_points(&mut body.timing);
    let meta = &map.meta;
    let native = meta.mode == 1;
    let sm = meta.slider_multiplier * TAIKO_VELOCITY_MULTIPLIER;
    let tick_rate = meta.slider_tick_rate;
    let scroll = scroll_table(&body.timing, sm);
    let velocity_at =
        |t: f64| scroll[scroll.partition_point(|&(at, _)| at <= t).saturating_sub(1)].1;
    let swell_hit_multiplier = difficulty_range(meta.od, 3.0, 5.0, 7.5) * 1.65;

    let mut notes = Vec::with_capacity(body.objects.len());
    // Sliders of length NaN: drawn as nothing, and in stable the don/kat right before each one
    // (left of it on the lane), however close, can't be hit (owner, 2026-10-28).
    let mut nan_sliders = Vec::new();
    for line in &body.objects {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 5 {
            continue;
        }
        let Ok(time) = f[2].trim().parse::<f64>() else {
            continue;
        };
        let time = time + version_offset;
        let ty: u32 = f[3].trim().parse().unwrap_or(1);
        let sound: u32 = f[4].trim().parse().unwrap_or(0);
        let velocity = velocity_at(time);

        if ty & 2 != 0 && f.len() >= 8 {
            // Slider: drumroll, or a stream of hits for short standard sliders.
            let spans = f[6].trim().parse::<u32>().unwrap_or(1).max(1);
            let raw_length = f[7].trim().parse::<f64>().unwrap_or(0.0);
            if raw_length.is_nan() {
                nan_sliders.push(time);
                continue;
            }
            let length = raw_length.max(0.0);
            let cp = control_point_at(&cps, time);
            let distance = length * TAIKO_VELOCITY_MULTIPLIER * spans as f64;
            let sv_beat_length = cp.beat_length / cp.sv;
            let taiko_velocity = 100.0 * sm;
            let duration = playable_length((distance / taiko_velocity * sv_beat_length).floor());
            // The duration a negative length works out to, made positive: osu! still draws that
            // much body ahead of the head (taiko_arranger's gameplay preview, `_draw_fake_slider`).
            // Not floored: the canonical -0.0001 is a fraction of a ms and stays just a head.
            let fake_len = if raw_length < 0.0 {
                playable_length(
                    -raw_length * TAIKO_VELOCITY_MULTIPLIER * spans as f64 / taiko_velocity
                        * sv_beat_length,
                )
            } else {
                0.0
            };

            let mut split = None;
            if !native {
                let osu_velocity = taiko_velocity * (1000.0 / sv_beat_length);
                let bl = if meta.format_version >= 8 {
                    cp.beat_length
                } else {
                    sv_beat_length
                };
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
                // 1/4 (1/3 at tick rate 3); twice as dense at 125 BPM or slower, half above 250.
                let roll_tick_rate = if tick_rate == 3.0 { 3.0 } else { 4.0 };
                let bpm = 60_000.0 / cp.beat_length;
                let roll_tick_rate = if bpm <= 125.0 {
                    roll_tick_rate * 2.0
                } else if bpm > 250.0 {
                    roll_tick_rate / 2.0
                } else {
                    roll_tick_rate
                };
                notes.push(Note {
                    time,
                    end_time: time + duration,
                    kind: Kind::Roll,
                    big: sound & 4 != 0,
                    velocity,
                    tick_spacing: cp.beat_length / roll_tick_rate,
                    required_hits: 0,
                    fake_len,
                    // The cap is fake_len ahead of the head; osu! counts whole ms (owner, 2026-10-28).
                    fake_fades: raw_length < 0.0 && fake_len < 1.0,
                    fake_length: (-raw_length).max(0.0) * spans as f64,
                    tiny: raw_length <= 0.1,
                    unhittable: false,
                });
            }
        } else if ty & 8 != 0 {
            let end = f
                .get(5)
                .and_then(|e| e.trim().parse::<f64>().ok())
                .map(|e| e + version_offset)
                .unwrap_or(time)
                .max(time);
            let end = time + playable_length(end - time);
            let required = ((end - time) / 1000.0 * swell_hit_multiplier).max(1.0) as u32;
            notes.push(Note {
                time,
                end_time: end,
                kind: Kind::Swell,
                big: false,
                velocity,
                tick_spacing: 0.0,
                required_hits: required,
                fake_len: 0.0,
                fake_fades: false,
                fake_length: 0.0,
                tiny: false,
                unhittable: false,
            });
        } else if ty & 1 != 0 {
            notes.push(hit(time, sound, velocity));
        }
    }
    notes.sort_by(|a, b| a.time.total_cmp(&b.time));
    for t in nan_sliders {
        let before = notes.partition_point(|n| n.time < t);
        if let Some(n) = notes[..before]
            .iter_mut()
            .rev()
            .find(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        {
            n.unhittable = true;
        }
    }

    let end = notes.last().map(|n| n.end_time).unwrap_or(0.0);
    map.barlines = build_barlines(&body.timing, end, &velocity_at);
    map.notes = notes;
    // A .tko map's fake notes, at the speed real ones at their time would have.
    map.fakes = map
        .gimmick
        .as_ref()
        .map(|g| {
            g.fakes
                .iter()
                .map(|f| {
                    let t = f.t + version_offset;
                    let sound = if f.kat { 2 } else { 0 } | if f.big { 4 } else { 0 };
                    hit(t, sound, velocity_at(t))
                })
                .collect()
        })
        .unwrap_or_default();
    map.control_points = cps;
    map.first_velocity = scroll[0].1;
}

/// The offset-calibration chart: one don on every beat from `first_beat` to `beats`, matching
/// the clicks of `audio::metronome` with the same `beat_ms`.
pub fn calibration_map(beat_ms: f64, first_beat: u32, beats: u32) -> Beatmap {
    let meta = Meta {
        title: "Offset calibration".into(),
        artist: "taiko.rs".into(),
        creator: "taiko.rs".into(),
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
    let notes = (first_beat..beats)
        .map(|b| hit(b as f64 * beat_ms, 0, velocity))
        .collect();
    let barlines = (0..beats)
        .step_by(4)
        .map(|b| Barline {
            time: b as f64 * beat_ms,
            velocity,
        })
        .collect();
    let control_points = vec![ControlPoint {
        time: 0.0,
        beat_origin: 0.0,
        beat_length: beat_ms,
        sv: 1.0,
        kiai: false,
        volume: 1.0,
    }];
    Beatmap {
        meta,
        notes,
        barlines,
        control_points,
        first_velocity: velocity,
        gimmick: None,
        fakes: Vec::new(),
    }
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
        fake_len: 0.0,
        fake_fades: false,
        fake_length: 0.0,
        tiny: false,
        unhittable: false,
    }
}

/// Scroll velocity from each timing point's ms on, worked out like taiko_arranger's preview: the
/// raw beat length and SV, floored at 1e-4 ms and 0.01x and never capped, because gimmick maps
/// (barline notes, hidden notes) depend on going far past the limits gameplay timing is clamped
/// to. A green line on a red line's ms wins over the red line's 1x reset. `raw` sorted by time.
fn scroll_table(raw: &[RawTimingPoint], sm: f64) -> Vec<(f64, f64)> {
    let mut beat = raw
        .iter()
        .find(|p| p.uninherited && p.beat_length > 0.0)
        .map_or(500.0, |p| p.beat_length);
    let mut out = Vec::new();
    for group in raw.chunk_by(|a, b| a.time == b.time) {
        let mut sv = 1.0;
        for p in group {
            if p.uninherited && p.beat_length > 0.0 {
                beat = p.beat_length;
            }
        }
        if let Some(p) = group
            .iter()
            .rev()
            .find(|p| !p.uninherited && p.beat_length < 0.0)
        {
            sv = -100.0 / p.beat_length;
        }
        out.push((
            group[0].time,
            sm * 100.0 * f64::max(sv, 0.01) / beat.max(1e-4),
        ));
    }
    if out.is_empty() {
        out.push((0.0, sm * 100.0 / beat));
    }
    out
}

/// One barline at every red line (unless it omits its first) and every measure after it, up to
/// the next red line. A section whose measure is under a millisecond (a gimmick line) gives only
/// its first: the rest would scroll by too fast to see.
fn build_barlines(
    raw: &[RawTimingPoint],
    end: f64,
    velocity_at: &dyn Fn(f64) -> f64,
) -> Vec<Barline> {
    let timing: Vec<&RawTimingPoint> = raw
        .iter()
        .filter(|p| p.uninherited && p.beat_length > 0.0)
        .collect();
    let mut out = Vec::new();
    for (i, p) in timing.iter().enumerate() {
        let until = timing.get(i + 1).map(|n| n.time).unwrap_or(end + 1.0);
        let step = p.beat_length * p.meter.max(1) as f64;
        let mut t = p.time;
        let mut first = true;
        // Per section, so one runaway section can't take every barline after it with it.
        for _ in 0..100_000 {
            if t >= until {
                break;
            }
            if !(first && p.effects & 8 != 0) {
                out.push(Barline {
                    time: t,
                    velocity: velocity_at(t),
                });
            }
            if step < 1.0 {
                break;
            }
            first = false;
            t += step;
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
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

    pub(crate) fn parse(text: &str) -> Beatmap {
        parse_raw(text)
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
        assert_eq!(
            file.control_points[0].beat_origin,
            raw.control_points[0].beat_origin + OSU_TIMING_MS
        );
        assert_eq!(file.barlines[0].time, raw.barlines[0].time + OSU_TIMING_MS);
        // Beat phase is unchanged at the shifted note times.
        let (cf, cr) = (&file.control_points[0], &raw.control_points[0]);
        assert!(
            (cf.beat_phase(file.notes[1].time) - cr.beat_phase(raw.notes[1].time)).abs() < 1e-9
        );
    }

    #[test]
    fn a_slider_trillions_of_pixels_long_is_a_roll_that_can_be_played() {
        // Like osu!team's FM Synthesis Experiment (set 2608578): NaN slider velocities and
        // sliders 3.6e12 px long, which crashed the game on picking the map.
        let text = "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n[Difficulty]\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n0,NaN,4,1,0,100,0,0\n2000,-0.0001,4,1,0,100,0,0\n\n[HitObjects]\n256,192,1000,2,0,L|31457196:20,1,3617087291392\n256,192,3000,2,0,L|31457196:20,1,3617087291392\n256,192,5000,12,0,1e300,0:0:0:0:\n";
        let map = parse(text);
        assert!(!map.notes.is_empty());
        for n in &map.notes {
            assert!(
                n.end_time.is_finite() && n.end_time - n.time <= LONGEST_NOTE_MS,
                "{n:?}"
            );
            assert!(n.fake_len.is_finite());
        }
        // What picking it in song select does: its difficulty graph and a game to preview.
        let graph = crate::difficulty::analyze(&map);
        assert!(graph.peaks.len() <= 1_000_000);
        assert!(
            map.too_wild_to_preview(),
            "song select shows it no playfield"
        );
        assert!(!parse(MAP).too_wild_to_preview());
        // And a game to preview: the client's beatmap_tests.
    }

    #[test]
    fn converts_standard_objects() {
        let map = parse(MAP);
        let kinds: Vec<(f64, Kind, bool)> =
            map.notes.iter().map(|n| (n.time, n.kind, n.big)).collect();
        // Circles: plain = don, whistle = kat, clap+finish = big kat.
        assert_eq!(kinds[0], (1000.0, Kind::Don, false));
        assert_eq!(kinds[1], (1500.0, Kind::Kat, false));
        assert_eq!(kinds[2], (2000.0, Kind::Kat, true));
        // A short slider (70px at 1.4 SM = half a beat) splits into hits every beat/tickrate, capped by span length.
        assert_eq!(kinds[3].1, Kind::Don);
        assert_eq!(kinds[3].0, 2500.0);
        // 70px is a quarter beat-length at 1.4 SM: too short for a roll, so it splits into hits 250ms apart.
        assert_eq!(
            map.notes
                .iter()
                .filter(|n| n.time >= 2500.0 && n.time < 3000.0)
                .count(),
            2
        );
        // The long slider under 2x SV becomes a drumroll: 560*2 px / (140 px/beat * 2 SV) = 4 beats = 2000ms.
        let roll = map
            .notes
            .iter()
            .find(|n| n.kind == Kind::Roll)
            .expect("drumroll");
        assert_eq!(roll.time, 5000.0);
        assert_eq!(roll.end_time, 7000.0);
        // 120 BPM: 1/8 ticks.
        assert_eq!(roll.tick_spacing, 62.5);
        let swell = map
            .notes
            .iter()
            .find(|n| n.kind == Kind::Swell)
            .expect("swell");
        assert_eq!(swell.end_time, 11000.0);
        // OD5: 5 * 1.65 hits per second over 2 seconds.
        assert_eq!(swell.required_hits, 16);
        // Velocity doubles under the 2x SV point.
        assert!((roll.velocity / map.notes[0].velocity - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_nan_slider_is_nothing_and_takes_the_note_before_it() {
        // A don at 1000, a kat at 1500, a NaN slider at 1499: the note before it is the don,
        // however close the kat is.
        let map = parse(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n142,139,1499,2,0,B|403:151|269:178,1,NaN\n256,192,1500,1,2\n",
        );
        let kinds: Vec<_> = map
            .notes
            .iter()
            .map(|n| (n.time, n.kind, n.unhittable))
            .collect();
        assert_eq!(
            kinds,
            [(1000.0, Kind::Don, true), (1500.0, Kind::Kat, false)]
        );
    }

    #[test]
    fn a_pop_up_fake_slider_keeps_its_written_length() {
        // senpai, notice me! [w] at 189607: a 1 ms red line under the slider, 387 ms after it.
        let map = parse(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.8\n\n[TimingPoints]\n0,387.096774193548,4,1,0,90,1,0\n1000,1,4,1,0,90,1,8\n1001,387.096774193548,4,1,0,90,1,8\n\n[HitObjects]\n166,117,1000,2,4,L|181:87,1,-300\n166,117,1000,2,0,L|181:87,1,-450\n",
        );
        let px: Vec<f64> = map.notes.iter().map(|n| n.fake_length).collect();
        assert!(px == [300.0, 450.0], "{px:?}");
        assert!(
            map.notes
                .iter()
                .all(|n| !n.fake_fades && n.velocity > 100.0)
        );
    }

    #[test]
    fn a_barline_note_draws_its_bars_and_hides_its_note() {
        // taiko_arranger's barline note: a gimmick red line on the note (omitting its own bar),
        // restores either side that each draw one bar, 1 ms from the next red line.
        let map = parse(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n999,500,4,1,0,100,1,0\n1000,0.000000001,4,1,0,100,1,8\n1001,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n256,192,1001,1,0\n256,192,4000,1,0\n",
        );
        let bars: Vec<f64> = map.barlines.iter().map(|b| b.time).collect();
        assert!(
            bars.contains(&999.0) && bars.contains(&1001.0) && !bars.contains(&1000.0),
            "{bars:?}"
        );
        // The bars scroll at the chart's speed, the note in the gimmick section far too fast to see.
        let normal = map.notes[1].velocity;
        assert!(map.barlines.iter().all(|b| b.velocity == normal));
        assert!(map.notes[0].velocity > normal * 1e6);
        // And the measures after the structure are all still there.
        assert!(bars.contains(&3001.0));
    }

    #[test]
    fn a_negative_length_slider_draws_a_body_but_plays_as_a_bare_head() {
        let map = parse(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,2,0,L|356:192,1,-70\n256,192,2000,2,0,L|356:192,1,-0.0001\n256,192,3000,2,0,L|356:192,1,70\n256,192,4000,2,0,L|356:192,1,-300\n",
        );
        let rolls: Vec<(f64, f64)> = map
            .notes
            .iter()
            .map(|n| (n.end_time - n.time, n.fake_len))
            .collect();
        // Only one whose cap can't land before its head fades at the drum; the rest vanish.
        let fades: Vec<bool> = map.notes.iter().map(|n| n.fake_fades).collect();
        assert_eq!(fades, [false, true, false, false]);
        // -70px at 1.4 SM is half a beat (250 ms) of body; it lasts no time at all.
        assert_eq!(rolls[0].0, 0.0);
        assert!((rolls[0].1 - 250.0).abs() < 1e-9);
        // The canonical gimmick value: a head and next to no body.
        assert!(rolls[1].1 > 0.0 && rolls[1].1 < 0.01);
        // A real roll has no fake body.
        assert_eq!(rolls[2], (250.0, 0.0));
    }

    #[test]
    fn sliders_written_under_0_1_long_are_tiny() {
        let lengths = ["0.1", "-0.1", "0.001", "-0.001", "0.11", "-0.11", "-50"];
        let objects: String = lengths
            .iter()
            .enumerate()
            .map(|(i, l)| format!("256,192,{},2,0,L|356:192,1,{l}\n", 1000 * (i + 1)))
            .collect();
        let map = parse(&format!(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n{objects}"
        ));
        let tiny: Vec<bool> = map.notes.iter().map(|n| n.tiny).collect();
        assert_eq!(tiny, [true, true, true, true, false, true, true]);
    }

    #[test]
    fn hit_windows_match_lazer() {
        assert_eq!(difficulty_range(5.0, 50.0, 35.0, 20.0), 35.0);
        assert_eq!(difficulty_range(10.0, 50.0, 35.0, 20.0), 20.0);
        assert_eq!(difficulty_range(0.0, 120.0, 80.0, 50.0), 120.0);
    }

    #[test]
    fn judged_windows_are_lazers() {
        // osu!(lazer)'s TaikoHitWindows: floor(DifficultyRange) - 0.5.
        let great = |od: f64| judged_window_ms(od, 50.0, 35.0, 20.0);
        let ok = |od: f64| judged_window_ms(od, 120.0, 80.0, 50.0);
        let miss = |od: f64| judged_window_ms(od, 135.0, 95.0, 70.0);
        assert_eq!([great(0.0), ok(0.0), miss(0.0)], [49.5, 119.5, 134.5]);
        assert_eq!([great(5.0), ok(5.0), miss(5.0)], [34.5, 79.5, 94.5]);
        assert_eq!([great(10.0), ok(10.0), miss(10.0)], [19.5, 49.5, 69.5]);
        assert_eq!(great(5.4), 32.5);
        assert_eq!(great(7.2), 27.5);
        // Whole numbers on paper stay whole, whatever the floating point does.
        for tenth in 0..=100 {
            let od = tenth as f64 / 10.0;
            let exact = 50.0 - 3.0 * od;
            assert_eq!(great(od), (exact + 1e-9).floor() - 0.5, "od {od}");
        }
    }

    #[test]
    fn judged_windows_keep_their_order_and_bend_without_a_jump() {
        let at = |od: f64| {
            [
                judged_window_ms(od, 50.0, 35.0, 20.0),
                judged_window_ms(od, 120.0, 80.0, 50.0),
                judged_window_ms(od, 135.0, 95.0, 70.0),
            ]
        };
        let below = at(WINDOW_BEND_OD);
        let above = at(WINDOW_BEND_OD + 1e-6);
        for k in 0..3 {
            assert!((below[k] - above[k]).abs() < 1e-3, "{below:?} {above:?}");
        }
        let mut last = at(0.0);
        for hundredth in 1..=2000 {
            let now = at(hundredth as f64 / 100.0);
            assert!(
                now[0] > 0.0 && now[0] < now[1] && now[1] < now[2],
                "{hundredth}: {now:?}"
            );
            for k in 0..3 {
                assert!(now[k] <= last[k], "{hundredth}: {now:?} after {last:?}");
            }
            last = now;
        }
    }

    #[test]
    fn hit_windows_stay_as_they_were_up_to_the_bend() {
        for tenth in 0..=133 {
            let od = tenth as f64 / 10.0;
            assert_eq!(
                window_ms(od, 50.0, 35.0, 20.0),
                difficulty_range(od, 50.0, 35.0, 20.0)
            );
            assert_eq!(
                window_ms(od, 120.0, 80.0, 50.0),
                difficulty_range(od, 120.0, 80.0, 50.0)
            );
            assert_eq!(
                window_ms(od, 135.0, 95.0, 70.0),
                difficulty_range(od, 135.0, 95.0, 70.0)
            );
        }
    }

    #[test]
    fn hit_windows_keep_their_order_and_shrink_all_the_way_to_od_20() {
        let at = |od: f64| {
            [
                great_ms(od),
                window_ms(od, 120.0, 80.0, 50.0),
                window_ms(od, 135.0, 95.0, 70.0),
            ]
        };
        let mut last = at(0.0);
        for hundredth in 1..=2000 {
            let now = at(hundredth as f64 / 100.0);
            assert!(
                now[0] > 0.0 && now[0] < now[1] && now[1] < now[2],
                "{hundredth}: {now:?}"
            );
            for k in 0..3 {
                assert!(now[k] < last[k], "{hundredth}: {now:?} after {last:?}");
                // No jump at the bend: a hundredth of an OD moves a window well under 0.1 ms.
                assert!(
                    last[k] - now[k] < 0.1,
                    "{hundredth}: {now:?} after {last:?}"
                );
            }
            last = now;
        }
        // OD 20: the numbers the mod's setup shows.
        assert!((last[0] - 10.0 * (-2.0f64).exp()).abs() < 1e-9);
        assert!((last[0] - 1.35).abs() < 0.01, "{last:?}");
        assert!((last[1] - 7.91).abs() < 0.01, "{last:?}");
        assert!((last[2] - 28.6).abs() < 0.1, "{last:?}");
    }

    #[test]
    fn od_of_great_undoes_great_ms() {
        for tenth in 0..=250 {
            let od = tenth as f64 / 10.0;
            assert!((od_of_great(great_ms(od)) - od).abs() < 1e-9, "{od}");
        }
        // Past the bend the rate goes on shifting it, up and down.
        assert!(od_at_rate(20.0, 1.5) > 20.0);
        assert!(od_at_rate(20.0, 0.75) < 20.0);
        assert!((od_at_rate(od_at_rate(20.0, 1.5), 1.0 / 1.5) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn od_follows_the_rate_through_the_great_window() {
        assert!((od_at_rate(5.0, 1.0) - 5.0).abs() < 1e-9);
        assert!((od_at_rate(0.0, 1.0)).abs() < 1e-9);
        assert!((od_at_rate(10.0, 1.0) - 10.0).abs() < 1e-9);
        // DT at OD 5: 35 ms becomes 23.3 ms, OD 8.89.
        assert!((od_at_rate(5.0, 1.5) - 80.0 / 9.0).abs() < 1e-9);
        // HT at OD 8: 26 ms becomes 34.7 ms, OD 5.11.
        assert!((od_at_rate(8.0, 0.75) - 46.0 / 9.0).abs() < 1e-9);
        assert!(
            od_at_rate(10.0, 1.5) > 10.0,
            "faster than OD 10 shows above it"
        );
        assert!(od_at_rate(7.0, 1.1) > od_at_rate(7.0, 1.05));
        // With the mods first: HR makes OD 5 into 7, which at DT plays like OD 10.22.
        let hr = crate::mods::Mods::parse("HR").od(5.0);
        assert!((od_at_rate(hr, 1.5) - 92.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn old_format_negative_beat_length_is_an_sv_change() {
        // v4-era timing points: no uninherited flag, inherited points marked by negative beat length.
        let map = parse(
            "osu file format v4\n[Difficulty]\nSliderMultiplier:1.4\n[TimingPoints]\n0,500,4,1,0,100\n2000,-50,4,1,0,100\n[HitObjects]\n256,192,1000,1,0\n256,192,3000,1,0\n",
        );
        assert_eq!(map.notes.len(), 2);
        assert!((map.notes[1].velocity / map.notes[0].velocity - 2.0).abs() < 1e-9);
        // Pre-v5 maps are shifted by 24 ms, like osu! does.
        assert_eq!(map.notes[0].time, 1024.0);
    }

    #[test]
    fn timing_points_carry_the_hitsound_volume() {
        // Red line at 70%, green lines at 40% and (an editor-impossible) 0%, which osu! floors at 5%.
        let map = parse(
            "osu file format v14\n[TimingPoints]\n0,500,4,1,0,70,1,0\n1000,-100,4,1,0,40,0,0\n2000,-100,4,1,0,0,0,0\n[HitObjects]\n256,192,500,1,0\n",
        );
        let at = |t| control_point_at(&map.control_points, t).volume;
        assert_eq!((at(500.0), at(1500.0), at(2500.0)), (0.7, 0.4, 0.05));
        // No volume field at all (very old maps): full volume.
        let old =
            parse("osu file format v3\n[TimingPoints]\n0,500\n[HitObjects]\n256,192,500,1,0\n");
        assert_eq!(control_point_at(&old.control_points, 600.0).volume, 1.0);
    }

    #[test]
    fn beats_count_from_the_red_line_not_green_ones() {
        // Green lines mid-beat keep the beat; a new red line restarts it.
        let map = parse(
            "osu file format v14\n[TimingPoints]\n100,500,4,1,0,100,1,0\n1230,-50,4,1,0,100,0,0\n3000,400,4,1,0,100,1,0\n3170,-80,4,1,0,100,0,0\n[HitObjects]\n256,192,500,1,0\n",
        );
        let phase = |t| control_point_at(&map.control_points, t).beat_phase(t);
        assert!((phase(1350.0) - 0.5).abs() < 1e-9);
        assert!((phase(3300.0) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn short_spinner_line_does_not_panic() {
        let map = parse(
            "osu file format v14\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,8,0\n256,192,2000,1,0\n",
        );
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
