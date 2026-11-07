//! Finding a song's tempo and where its beats fall, for timing a new map: onsets are picked from
//! the audio, then a BPM is searched by folding them over a period (the right period piles every
//! onset up at one phase). Tapping along gives a tempo too, and a metronome can be mixed into audio
//! so a timing can be checked by ear.

/// What was found in a song.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub bpm: f64,
    /// The first beat of the grid in the song (ms, from 0 up to one beat): the red line goes here.
    pub offset_ms: f64,
    /// How much of the song's onsets sit on the beat grid (0 to 1).
    pub fit: f32,
    /// How many times more of them sit there than would by chance: 3 or more is a clear tempo.
    pub lift: f32,
    /// Where a stretch stops following the grid, if one does: the tempo may change there.
    pub unsteady_at: Option<f64>,
}

pub const MIN_BPM: f64 = 60.0;
pub const MAX_BPM: f64 = 300.0;
/// Where the search favours a tempo (BPM): the middle of what taiko maps use.
const PREFERRED_BPM: f64 = 150.0;
/// How far from it (in octaves) a tempo is still about as likely.
const PREFERRED_SPREAD: f64 = 0.7;
/// Onsets are looked for at this spacing (ms) and counted as one beat's if they are this close (ms).
const BIN_MS: f64 = 1.0;
const GRID_TOLERANCE_MS: f64 = 3.0;
/// The folded histogram's bin (ms).
const HIST_MS: f64 = 0.5;
/// Most onsets kept: the strongest.
const MAX_ONSETS: usize = 8000;
/// How much later a beat is heard than the onset detector finds it (ms): it finds where a drum's
/// attack starts to rise, and a beat is where its sound is felt. Measured against a real map's
/// red lines (the detector was 8.6 ms early on a 168 BPM song).
const ATTACK_LAG_MS: f64 = 7.0;
/// The stretch (ms) the first, rough search listens to.
const ROUGH_WINDOW_MS: f64 = 20_000.0;
/// A stretch of the steadiness check (ms).
const STEADY_WINDOW_MS: f64 = 15_000.0;

/// One onset: when (ms) and how strong.
pub type Onset = (f64, f32);

/// The tempo of interleaved stereo `samples` at `rate`. None when there is too little to go by
/// (silence, a few seconds of anything).
pub fn analyse(samples: &[f32], rate: f64) -> Option<Analysis> {
    let onsets = onsets(samples, rate);
    if onsets.len() < 8 {
        return None;
    }
    let duration = samples.len() as f64 / 2.0 / rate * 1000.0;
    let (bpm, _) = search(&onsets, duration)?;
    let (bpm, offset) = refine(&onsets, bpm);
    let period = 60_000.0 / bpm;
    let (fit, unsteady_at) = grid_fit(&onsets, offset, period);
    let offset = (offset + ATTACK_LAG_MS).rem_euclid(period);
    let chance = (4.0 * GRID_TOLERANCE_MS / period).min(1.0) as f32;
    Some(Analysis {
        bpm,
        offset_ms: offset,
        fit,
        lift: fit / chance.max(f32::MIN_POSITIVE),
        unsteady_at,
    })
}

// ---- onsets ----

/// Sudden rises of energy, in two bands (lows for kicks, highs for everything sharp), at 1 ms.
/// Each onset's time is the middle of its rise, so a click is found where it is.
pub fn onsets(samples: &[f32], rate: f64) -> Vec<Onset> {
    let frames = samples.len() / 2;
    let hop = ((rate * BIN_MS / 1000.0).round() as usize).max(1);
    if frames < hop * 64 {
        return Vec::new();
    }
    let bin_ms = hop as f64 / rate * 1000.0;
    // Prefix sums of the squares of both bands: a window's energy is two reads.
    let a = 1.0 - (-std::f64::consts::TAU * 180.0 / rate).exp();
    let (mut low, mut high) = (vec![0.0f64; frames + 1], vec![0.0f64; frames + 1]);
    let (mut y, mut prev) = (0.0f64, 0.0f64);
    for n in 0..frames {
        let m = (samples[2 * n] as f64 + samples[2 * n + 1] as f64) * 0.5;
        y += a * (m - y);
        let d = m - prev;
        prev = m;
        low[n + 1] = low[n] + y * y;
        high[n + 1] = high[n] + d * d;
    }
    let bins = frames / hop;
    let window = 4 * hop;
    let energy = |sums: &[f64]| -> Vec<f64> {
        let e: Vec<f64> = (0..bins)
            .map(|i| {
                let at = i * hop;
                let (lo, hi) = (at.saturating_sub(window / 2), (at + window / 2).min(frames));
                (sums[hi] - sums[lo]) / window as f64
            })
            .collect();
        let mean = e.iter().sum::<f64>() / e.len().max(1) as f64;
        if mean <= 1e-18 {
            return vec![0.0; bins];
        }
        // As amplitude relative to the song's average, so a hard hit counts for more than a soft one.
        e.iter().map(|v| (v / mean).sqrt()).collect()
    };
    let (low, high) = (energy(&low), energy(&high));
    const LAG: usize = 4;
    let flux: Vec<f64> = (0..bins)
        .map(|i| {
            if i < LAG {
                return 0.0;
            }
            (low[i] - low[i - LAG]).max(0.0) * 0.7 + (high[i] - high[i - LAG]).max(0.0)
        })
        .collect();
    // An onset: the top of its neighbourhood, and clear of the stretch around it.
    let mut sorted: Vec<f64> = flux.iter().copied().filter(|v| *v > 0.0).collect();
    if sorted.is_empty() {
        return Vec::new();
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let p99 = sorted[(sorted.len() as f64 * 0.99) as usize % sorted.len()];
    let mut near = vec![0.0f64; bins + 1];
    for i in 0..bins {
        near[i + 1] = near[i] + flux[i];
    }
    const SPAN: usize = 12;
    const AROUND: usize = 250;
    let mut found: Vec<Onset> = Vec::new();
    for i in SPAN..bins.saturating_sub(SPAN) {
        let v = flux[i];
        if v < 0.1 * p99 {
            continue;
        }
        let (lo, hi) = (i - SPAN, i + SPAN);
        // The first of equal values wins, so a plateau is found once.
        if flux[lo..i].iter().any(|o| *o >= v) || flux[i + 1..=hi].iter().any(|o| *o > v) {
            continue;
        }
        let (a, b) = (i.saturating_sub(AROUND), (i + AROUND).min(bins));
        let mean = (near[b] - near[a]) / (b - a) as f64;
        if v < 1.4 * mean {
            continue;
        }
        // Where its weight is, not the first bin of a plateau.
        let (mut sum, mut weight) = (0.0, 0.0);
        for (k, f) in flux
            .iter()
            .enumerate()
            .take(i + 4)
            .skip(i.saturating_sub(3))
        {
            sum += k as f64 * f;
            weight += f;
        }
        found.push((sum / weight * bin_ms, v as f32));
    }
    if found.len() > MAX_ONSETS {
        let mut strengths: Vec<f32> = found.iter().map(|o| o.1).collect();
        strengths.sort_by(|a, b| b.total_cmp(a));
        let cut = strengths[MAX_ONSETS - 1];
        found.retain(|o| o.1 >= cut);
        found.truncate(MAX_ONSETS);
    }
    found
}

// ---- the search ----

/// How much a tempo is favoured before looking at the music.
fn prior(bpm: f64) -> f64 {
    let octaves = (bpm / PREFERRED_BPM).log2() / PREFERRED_SPREAD;
    (-0.5 * octaves * octaves).exp()
}

/// The onsets folded over `period` (ms): how many pile up at the best phase, minus the average
/// pile (which a short period would otherwise win by), and that phase (ms).
fn fold(onsets: &[Onset], period: f64, hist: &mut Vec<f64>) -> (f64, f64) {
    let bins = (period / HIST_MS).ceil() as usize;
    hist.clear();
    hist.resize(bins, 0.0);
    let reach = (GRID_TOLERANCE_MS / HIST_MS) as i64;
    let mut total = 0.0;
    for &(t, s) in onsets {
        let at = (t % period) / HIST_MS;
        let centre = at.round() as i64;
        for k in -reach..=reach {
            let w = 1.0 - k.abs() as f64 / (reach + 1) as f64;
            let slot = (centre + k).rem_euclid(bins as i64) as usize;
            hist[slot] += s as f64 * w;
        }
        total += s as f64 * (reach + 1) as f64;
    }
    let (best, peak) = hist
        .iter()
        .enumerate()
        .fold((0, 0.0), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
    let mean = total / bins as f64;
    (peak - mean, best as f64 * HIST_MS)
}

/// A rough tempo from one stretch of the song, then a close one from all of it. Returns the tempo
/// and its score.
fn search(onsets: &[Onset], duration: f64) -> Option<(f64, f64)> {
    // The stretch with the most going on.
    let from = busiest(onsets, ROUGH_WINDOW_MS.min(duration));
    let part: Vec<Onset> = onsets
        .iter()
        .copied()
        .filter(|o| o.0 >= from && o.0 < from + ROUGH_WINDOW_MS)
        .map(|o| (o.0 - from, o.1))
        .collect();
    if part.len() < 6 {
        return None;
    }
    let mut hist = Vec::new();
    let mut rough: Vec<(f64, f64)> = Vec::new();
    let step = 2e-4;
    let mut bpm = MIN_BPM;
    while bpm <= MAX_BPM {
        let (score, _) = fold(&part, 60_000.0 / bpm, &mut hist);
        rough.push((bpm, score * prior(bpm)));
        bpm *= 1.0 + step;
    }
    // The best few that aren't the same peak.
    let mut best: Vec<(f64, f64)> = Vec::new();
    let mut order: Vec<usize> = (0..rough.len()).collect();
    order.sort_by(|a, b| rough[*b].1.total_cmp(&rough[*a].1));
    for i in order {
        if best.len() == 4 {
            break;
        }
        let b = rough[i].0;
        if best.iter().all(|c| (c.0 / b - 1.0).abs() > 0.004) {
            best.push(rough[i]);
        }
    }
    // The close search, with every onset: the drift over a whole song is what tells tempos apart.
    let mut winner: Option<(f64, f64)> = None;
    for (rough_bpm, _) in best {
        let mut rel = -6e-4;
        while rel <= 6e-4 {
            let bpm = rough_bpm * (1.0 + rel);
            let (score, _) = fold(onsets, 60_000.0 / bpm, &mut hist);
            let score = score * prior(bpm);
            if winner.is_none_or(|w| score > w.1) {
                winner = Some((bpm, score));
            }
            rel += 4e-6;
        }
    }
    winner
}

/// Where (ms) a stretch of `len` starts that holds the most onset strength.
fn busiest(onsets: &[Onset], len: f64) -> f64 {
    let mut best = (0.0, f64::MIN);
    let mut start = 0;
    let mut sum = 0.0;
    for o in onsets {
        sum += o.1 as f64;
        while onsets[start].0 < o.0 - len {
            sum -= onsets[start].1 as f64;
            start += 1;
        }
        if sum > best.1 {
            best = (onsets[start].0, sum);
        }
    }
    best.0
}

/// A tempo and phase settled by least squares over the onsets that sit on the grid.
fn refine(onsets: &[Onset], bpm: f64) -> (f64, f64) {
    let mut hist = Vec::new();
    let mut period = 60_000.0 / bpm;
    let (_, phase) = fold(onsets, period, &mut hist);
    let mut offset = phase;
    for _ in 0..3 {
        let (mut sw, mut sk, mut st, mut skk, mut skt) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(t, s) in onsets {
            let k = ((t - offset) / period).round();
            let miss = t - (offset + k * period);
            if miss.abs() > GRID_TOLERANCE_MS * 2.0 {
                continue;
            }
            let w = s as f64;
            sw += w;
            sk += w * k;
            st += w * t;
            skk += w * k * k;
            skt += w * k * t;
        }
        let det = sw * skk - sk * sk;
        if sw <= 0.0 || det.abs() < 1e-9 {
            break;
        }
        let slope = (sw * skt - sk * st) / det;
        let intercept = (st - slope * sk) / sw;
        // Never far from the search's answer: a few beats' fit can't be trusted over all of it.
        if (slope / period - 1.0).abs() < 3e-4 {
            period = slope;
        }
        offset = intercept;
    }
    (60_000.0 / period, offset.rem_euclid(period))
}

/// The fraction of onset strength within the grid's reach, and the start of the first stretch that
/// doesn't keep to it.
fn grid_fit(onsets: &[Onset], offset: f64, period: f64) -> (f32, Option<f64>) {
    let on_grid = |t: f64| {
        let k = ((t - offset) / period).round();
        (t - (offset + k * period)).abs() <= GRID_TOLERANCE_MS * 2.0
    };
    let total: f64 = onsets.iter().map(|o| o.1 as f64).sum();
    let aligned: f64 = onsets
        .iter()
        .filter(|o| on_grid(o.0))
        .map(|o| o.1 as f64)
        .sum();
    let fit = if total > 0.0 { aligned / total } else { 0.0 };
    // Stretch by stretch: one that has gone off the grid is a tempo change (or a free section).
    let end = onsets.last().map_or(0.0, |o| o.0);
    let mut unsteady = None;
    let mut from = 0.0;
    while from + STEADY_WINDOW_MS <= end {
        let in_window = onsets
            .iter()
            .filter(|o| o.0 >= from && o.0 < from + STEADY_WINDOW_MS);
        let (mut all, mut hit) = (0.0, 0.0);
        let (mut residual, mut count) = (0.0, 0);
        for o in in_window {
            all += o.1 as f64;
            if on_grid(o.0) {
                hit += o.1 as f64;
                count += 1;
                let k = ((o.0 - offset) / period).round();
                residual += o.1 as f64 * (o.0 - (offset + k * period));
            }
        }
        // Stretches with hardly anything in them (breaks) say nothing.
        if all > total * (STEADY_WINDOW_MS / end.max(1.0)) * 0.3 {
            // A stretch the grid hardly explains is only telling when the grid explains the song.
            let lost = fit >= 0.3 && hit / all < fit * 0.35;
            // One that is explained, but a little early or late all through, has the tempo off.
            let drift = if count >= 8 { residual / hit } else { 0.0 };
            if lost || drift.abs() > 12.0 {
                unsteady = Some(from);
                break;
            }
        }
        from += STEADY_WINDOW_MS / 2.0;
    }
    (fit as f32, unsteady)
}

// ---- tapping ----

/// The tempo and a beat's time (ms) from taps (song times, oldest first). The last 24 are used;
/// at least 4 are needed. A pause in the middle (a gap over 2.2 beats of the median) starts over.
pub fn tap(taps: &[f64]) -> Option<(f64, f64)> {
    let mut recent = taps;
    if recent.len() > 24 {
        recent = &recent[recent.len() - 24..];
    }
    if recent.len() < 4 {
        return None;
    }
    let mut gaps: Vec<f64> = recent.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(|a, b| a.total_cmp(b));
    let median = gaps[gaps.len() / 2];
    if median <= 0.0 {
        return None;
    }
    // From the last pause on.
    let start = recent
        .windows(2)
        .rposition(|w| w[1] - w[0] > median * 2.2)
        .map_or(0, |i| i + 1);
    let recent = &recent[start..];
    if recent.len() < 4 {
        return None;
    }
    // t = offset + n * period, by least squares on the tap number.
    let n = recent.len() as f64;
    let (mut sk, mut st, mut skk, mut skt) = (0.0, 0.0, 0.0, 0.0);
    for (k, t) in recent.iter().enumerate() {
        let k = k as f64;
        sk += k;
        st += t;
        skk += k * k;
        skt += k * t;
    }
    let period = (n * skt - sk * st) / (n * skk - sk * sk);
    if !(60_000.0 / MAX_BPM..=60_000.0 / MIN_BPM).contains(&period) {
        return None;
    }
    let offset = (st - period * sk) / n;
    Some((60_000.0 / period, offset))
}

// ---- the metronome ----

/// Mixes a click on every beat into interleaved stereo `window` (which starts at frame
/// `first_frame` of the song), the first beat of each bar higher and louder. `music` scales what
/// is already there.
pub fn mix_clicks(
    window: &mut [f32],
    rate: f64,
    first_frame: i64,
    bpm: f64,
    offset_ms: f64,
    per_bar: u32,
    music: f32,
) {
    for s in window.iter_mut() {
        *s *= music;
    }
    if bpm <= 0.0 || per_bar == 0 {
        return;
    }
    let frames = (window.len() / 2) as i64;
    let period = 60_000.0 / bpm;
    let start_ms = first_frame as f64 / rate * 1000.0;
    let end_ms = (first_frame + frames) as f64 / rate * 1000.0;
    let click_ms = CLICK_MS;
    let first = ((start_ms - click_ms - offset_ms) / period).floor() as i64;
    let last = ((end_ms - offset_ms) / period).ceil() as i64;
    for beat in first..=last {
        let accent = beat.rem_euclid(per_bar as i64) == 0;
        let at = ((offset_ms + beat as f64 * period) / 1000.0 * rate).round() as i64 - first_frame;
        put_click(window, rate, at, accent);
    }
}

/// One click starting at frame `at` of interleaved stereo `window`, higher and louder on a bar's
/// first beat.
fn put_click(window: &mut [f32], rate: f64, at: i64, accent: bool) {
    let frames = (window.len() / 2) as i64;
    // Loud enough to cut through a loudly mastered song (the output's limiter keeps the peaks):
    // at 0.35 it was lost under the music (player report, 2026-11-07).
    let (pitch, gain) = if accent {
        (1800.0, 1.0)
    } else {
        (1100.0, 0.75)
    };
    for k in 0..(CLICK_MS / 1000.0 * rate) as i64 {
        let f = at + k;
        if f < 0 || f >= frames {
            continue;
        }
        let t = k as f64 / rate;
        let v = ((std::f64::consts::TAU * pitch * t).sin() * (-t * 90.0).exp() * gain) as f32;
        window[2 * f as usize] += v;
        window[2 * f as usize + 1] += v;
    }
}

const CLICK_MS: f64 = 40.0;
/// How loud the music stays under the metronome's clicks.
pub const MUSIC_UNDER_CLICKS: f32 = 0.7;

/// One stretch of a map's timing: a red line's start, its beat length (ms) and beats to a bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub time: f64,
    pub beat: f64,
    pub per_bar: u32,
}

/// Sections closer together than this (ms) are gimmick lines: a click on each would be a buzz.
const MIN_CLICK_BEAT_MS: f64 = 60.0;

/// Where the clicks fall in map time `from..to` (ms) for `sections` (sorted by time): a beat of
/// each red line's grid, a bar's first accented, each line starting a bar of its own. The first
/// section's grid goes on backwards before its line, as the editor's grid does.
pub fn clicks_between(sections: &[Section], from: f64, to: f64) -> Vec<(f64, bool)> {
    let mut out = Vec::new();
    for (i, s) in sections.iter().enumerate() {
        if !(s.beat >= MIN_CLICK_BEAT_MS && s.beat.is_finite()) || s.per_bar == 0 {
            continue;
        }
        let lo = if i == 0 { from } else { from.max(s.time) };
        let hi = sections.get(i + 1).map_or(to, |n| to.min(n.time));
        if lo >= hi {
            continue;
        }
        let mut k = ((lo - s.time) / s.beat).ceil() as i64;
        loop {
            let t = s.time + k as f64 * s.beat;
            if t >= hi || out.len() > 20_000 {
                break;
            }
            out.push((t, k.rem_euclid(s.per_bar as i64) == 0));
            k += 1;
        }
    }
    out
}

/// Mixes the clicks of a map's `sections` into interleaved stereo `window` (starting at frame
/// `first_frame` of the track), and scales the music under them by `music`. The track plays the
/// map `speed` times as fast, `shift_ms` of map time behind its notes (see `OSU_TIMING_MS`), so
/// the clicks follow the playback speed.
#[allow(clippy::too_many_arguments)]
pub fn mix_section_clicks(
    window: &mut [f32],
    rate: f64,
    first_frame: i64,
    sections: &[Section],
    shift_ms: f64,
    speed: f64,
    music: f32,
) {
    for s in window.iter_mut() {
        *s *= music;
    }
    let speed = speed.max(0.01);
    let frames = (window.len() / 2) as i64;
    let to_map = |track_ms: f64| track_ms * speed - shift_ms;
    let start_ms = first_frame as f64 / rate * 1000.0;
    let end_ms = (first_frame + frames) as f64 / rate * 1000.0;
    for (t, accent) in clicks_between(sections, to_map(start_ms - CLICK_MS), to_map(end_ms)) {
        let track_ms = (t + shift_ms) / speed;
        let at = (track_ms / 1000.0 * rate).round() as i64 - first_frame;
        put_click(window, rate, at, accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// A song of clicks on every beat, quieter ones between, and some noise under it.
    fn clicks(bpm: f64, offset_ms: f64, secs: f64, noise: f32) -> Vec<f32> {
        let frames = (secs * RATE) as usize;
        let mut out = vec![0.0f32; frames * 2];
        let mut seed = 12345u32;
        let mut rand = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for f in 0..frames {
            let n = rand() * noise;
            out[2 * f] = n;
            out[2 * f + 1] = n;
        }
        let period = 60_000.0 / bpm;
        let mut put = |at_ms: f64, gain: f32| {
            let at = (at_ms / 1000.0 * RATE).round() as usize;
            for k in 0..480usize {
                if at + k >= frames {
                    break;
                }
                let t = k as f64 / RATE;
                let v =
                    ((std::f64::consts::TAU * 1500.0 * t).sin() * (-t * 400.0).exp()) as f32 * gain;
                out[2 * (at + k)] += v;
                out[2 * (at + k) + 1] += v;
            }
        };
        let mut beat = 0;
        loop {
            let t = offset_ms + beat as f64 * period;
            if t > secs * 1000.0 - 200.0 {
                break;
            }
            put(t, if beat % 4 == 0 { 0.9 } else { 0.6 });
            put(t + period / 2.0, 0.3);
            beat += 1;
        }
        out
    }

    fn check(bpm: f64, offset: f64, secs: f64, noise: f32) {
        let found = analyse(&clicks(bpm, offset, secs, noise), RATE).expect("a tempo");
        assert!(
            (found.bpm - bpm).abs() < 0.05,
            "{bpm} BPM found as {}",
            found.bpm
        );
        // The grid, not which beat comes first: the offset is within the song's first beat.
        let period = 60_000.0 / bpm;
        let miss = (found.offset_ms - offset - ATTACK_LAG_MS).rem_euclid(period);
        assert!(
            miss.min(period - miss) < 3.0,
            "offset {offset} found as {}",
            found.offset_ms
        );
        assert!(
            (0.0..period).contains(&found.offset_ms),
            "{}",
            found.offset_ms
        );
        assert!(found.lift > 2.0, "lift {}", found.lift);
        assert!(found.fit > 0.25, "fit {}", found.fit);
        assert!(
            found.unsteady_at.is_none(),
            "steady: {:?}",
            found.unsteady_at
        );
    }

    #[test]
    fn a_plain_tempo_is_found() {
        check(120.0, 500.0, 40.0, 0.0);
    }

    #[test]
    fn a_fractional_tempo_and_an_odd_offset_are_found_within_milliseconds() {
        check(174.97, 1234.0, 90.0, 0.02);
    }

    #[test]
    fn fast_and_slow_tempos_are_found() {
        check(200.0, 350.0, 45.0, 0.02);
        check(140.0, 2000.0, 50.0, 0.05);
    }

    #[test]
    fn silence_and_scraps_have_no_tempo() {
        assert!(analyse(&vec![0.0; 2 * 48_000 * 10], RATE).is_none());
        assert!(analyse(&vec![0.0; 100], RATE).is_none());
    }

    #[test]
    fn a_song_that_changes_tempo_is_flagged() {
        let mut song = clicks(150.0, 300.0, 40.0, 0.0);
        song.extend(clicks(171.0, 90.0, 40.0, 0.0));
        let found = analyse(&song, RATE).expect("a tempo");
        // Whichever half the search settled on, the other doesn't follow its grid.
        assert!(found.unsteady_at.is_some(), "{found:?}");
    }

    #[test]
    fn taps_give_a_tempo_and_a_beat() {
        // 128 BPM, a little shaky, starting at 800 ms.
        let period = 60_000.0 / 128.0;
        let taps: Vec<f64> = (0..12)
            .map(|k| 800.0 + k as f64 * period + [3.0, -4.0, 2.0, 0.0, -2.0, 5.0][k % 6])
            .collect();
        let (bpm, offset) = tap(&taps).unwrap();
        assert!((bpm - 128.0).abs() < 0.8, "{bpm}");
        assert!((offset - 800.0).abs() < 6.0, "{offset}");
        assert!(tap(&taps[..3]).is_none(), "three taps are too few");
    }

    #[test]
    fn a_pause_between_taps_starts_over() {
        let period = 500.0;
        let mut taps: Vec<f64> = (0..6).map(|k| k as f64 * 400.0).collect();
        let resume = 2000.0 + 5.0 * period;
        taps.extend((0..6).map(|k| resume + k as f64 * period));
        let (bpm, _) = tap(&taps).unwrap();
        assert!((bpm - 120.0).abs() < 0.1, "{bpm}");
    }

    #[test]
    fn the_metronome_clicks_on_the_beats_and_leaves_the_rest_alone() {
        let mut window = vec![0.0f32; 2 * 48_000];
        mix_clicks(&mut window, RATE, 0, 120.0, 100.0, 4, 1.0);
        let loud = |ms: f64| {
            let f = (ms / 1000.0 * RATE) as usize;
            (f..f + 200)
                .map(|i| window[2 * i].abs())
                .fold(0.0, f32::max)
        };
        assert!(loud(100.0) > 0.1, "the first beat");
        assert!(loud(600.0) > 0.1, "the next one, 500 ms later");
        assert!(loud(100.0) > loud(600.0), "the bar's first is louder");
        assert!(loud(400.0) < 0.001, "nothing between beats");
        // The music underneath is scaled.
        let mut music = vec![1.0f32; 2 * 100];
        mix_clicks(&mut music, RATE, 0, 120.0, 10_100.0, 4, 0.5);
        assert!(music.iter().all(|s| (*s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn the_metronome_follows_red_lines_meters_and_speed() {
        let sections = [
            Section {
                time: 0.0,
                beat: 500.0,
                per_bar: 4,
            },
            Section {
                time: 2100.0,
                beat: 250.0,
                per_bar: 3,
            },
        ];
        let c = clicks_between(&sections, 0.0, 3000.0);
        let times: Vec<f64> = c.iter().map(|x| x.0).collect();
        assert_eq!(
            times,
            vec![
                0.0, 500.0, 1000.0, 1500.0, 2000.0, 2100.0, 2350.0, 2600.0, 2850.0
            ]
        );
        let accents: Vec<f64> = c.iter().filter(|x| x.1).map(|x| x.0).collect();
        // The bar of 4 restarts at 0 and 2000; the red line at 2100 starts a bar of 3 of its own.
        assert_eq!(accents, vec![0.0, 2000.0, 2100.0, 2850.0]);
        // Before the first line the grid goes on backwards.
        let c = clicks_between(&sections[..1], -1000.0, 600.0);
        let times: Vec<f64> = c.iter().map(|x| x.0).collect();
        assert_eq!(times, vec![-1000.0, -500.0, 0.0, 500.0]);
        assert_eq!(
            c.iter().filter(|x| x.1).map(|x| x.0).collect::<Vec<_>>(),
            vec![0.0]
        );
        // A gimmick line's 1 ms beat makes no buzz.
        let gimmick = [Section {
            time: 0.0,
            beat: 1.0,
            per_bar: 4,
        }];
        assert!(clicks_between(&gimmick, 0.0, 1000.0).is_empty());
    }

    #[test]
    fn baked_clicks_land_where_the_slowed_track_plays_them() {
        let sections = [Section {
            time: 100.0,
            beat: 500.0,
            per_bar: 4,
        }];
        let loud = |w: &[f32], ms: f64| {
            let f = (ms / 1000.0 * RATE) as usize;
            (f..f + 200).map(|i| w[2 * i].abs()).fold(0.0, f32::max)
        };
        // At half speed the track is twice as long: the map's 500 ms beat is 1000 ms of track.
        let mut w = vec![0.0f32; 2 * 48_000 * 3];
        mix_section_clicks(&mut w, RATE, 0, &sections, 23.0, 0.5, 1.0);
        assert!(loud(&w, (100.0 + 23.0) / 0.5) > 0.1);
        assert!(loud(&w, (600.0 + 23.0) / 0.5) > 0.1);
        assert!(loud(&w, 600.0) < 0.001, "not at the full-speed spot");
        // A window cut from the middle holds the same click, by its first frame.
        let first = (1.2 * RATE) as i64;
        let mut part = vec![0.0f32; 2 * 48_000];
        mix_section_clicks(&mut part, RATE, first, &sections, 23.0, 0.5, 1.0);
        let at = ((600.0 + 23.0) / 0.5 / 1000.0 * RATE) as i64 - first;
        assert!(
            (0..200)
                .map(|i| part[2 * (at + i) as usize].abs())
                .fold(0.0, f32::max)
                > 0.1
        );
    }
}
