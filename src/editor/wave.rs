//! The song's waveform, for the editor's timeline and the timing page of a new map.
//!
//! Made once from the decoded song (`Wave::build`, on a thread): for each short stretch the
//! loudest sample, the average loudness and how much of it is bass, middle and treble, at every
//! zoom (each level of `levels` halves the one before). Drawing then reads a few cells per
//! column, however long the song: the outline is the loudest, the solid body the average, and the
//! colour says what kind of sound it is. Bass leans to a don's red and treble to a kat's blue,
//! which is roughly where a taiko map puts them.

use crate::hit::Canvas;
use crate::render::rgba;

/// Frames of a cell of the finest level (1.3 ms at 48 kHz).
const BASE: usize = 64;
/// Cutoffs (Hz) between bass and middle, and middle and treble.
const LOW_HZ: f64 = 180.0;
const HIGH_HZ: f64 = 2500.0;
/// The colours of bass, middle and treble.
const BASS: [f32; 3] = [0.92, 0.30, 0.20];
const MIDDLE: [f32; 3] = [0.86, 0.80, 0.98];
const TREBLE: [f32; 3] = [0.25, 0.66, 0.92];
/// How much each band counts towards the colour: treble and the middle carry less energy than
/// bass for the same loudness to the ear.
const WEIGHTS: [f32; 3] = [1.0, 1.7, 3.4];
/// Finest cells the band loudness is averaged over (about 20 ms at 48 kHz): a bass note's own
/// cycles would otherwise flicker the colour from column to column.
const BAND_SMOOTH: usize = 15;
/// How far the body (the average loudness) reaches, of the outline (the loudest).
const BODY: f32 = 1.3;
/// Columns per unit of width the waveform is drawn with: about one pixel of a 1080p screen.
const COLUMNS_PER_UNIT: f32 = 2.0;

/// One stretch: loudest sample and average loudness, and the average loudness of each band.
/// All 0..1 of the song's loudest sample, kept in 16 bits.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Cell {
    peak: u16,
    rms: u16,
    bands: [u16; 3],
}

fn q(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

fn f(v: u16) -> f32 {
    v as f32 / 65535.0
}

/// What a column of the drawing shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Column {
    /// 0..1, of the song's loudest.
    pub peak: f32,
    pub rms: f32,
    pub colour: [f32; 3],
}

pub struct Wave {
    /// Frames of the decoded samples per ms of the song.
    frames_per_ms: f64,
    /// Finest first.
    levels: Vec<Vec<Cell>>,
}

impl Wave {
    /// From interleaved stereo `samples` at `rate` frames a second, made for playback at `speed`
    /// (one second of the samples is `speed` seconds of the song, as `Track::rate`).
    pub fn build(samples: &[f32], rate: f64, speed: f64) -> Wave {
        let frames = samples.len() / 2;
        let lp = |hz: f64| (1.0 - (-std::f64::consts::TAU * hz / rate.max(1.0)).exp()) as f32;
        let (a_low, a_high) = (lp(LOW_HZ), lp(HIGH_HZ));
        let (mut low, mut below_high) = (0.0f32, 0.0f32);
        // Sums first, then scaled by the loudest sample once it is known.
        let mut raw: Vec<[f32; 5]> = Vec::with_capacity(frames / BASE + 1);
        let mut loudest = 0.0f32;
        for block in samples.chunks(BASE * 2) {
            let mut acc = [0.0f32; 5];
            let n = (block.len() / 2).max(1) as f32;
            for fr in block.as_chunks::<2>().0 {
                let x = (fr[0] + fr[1]) * 0.5;
                low += a_low * (x - low);
                below_high += a_high * (x - below_high);
                let (mid, high) = (below_high - low, x - below_high);
                acc[0] = acc[0].max(x.abs());
                acc[1] += x * x;
                acc[2] += low * low;
                acc[3] += mid * mid;
                acc[4] += high * high;
            }
            for v in &mut acc[1..] {
                *v = (*v / n).sqrt();
            }
            loudest = loudest.max(acc[0]);
            raw.push(acc);
        }
        let scale = if loudest > 1e-6 { 1.0 / loudest } else { 0.0 };
        // The bands, averaged over a short window (as loudness: mean of squares).
        let mut sums = [0.0f64; 3];
        let mut smoothed: Vec<[f32; 3]> = Vec::with_capacity(raw.len());
        let half = BAND_SMOOTH / 2;
        let sq = |i: usize, b: usize| (raw[i][2 + b] as f64).powi(2);
        for i in 0..raw.len() + half {
            if i < raw.len() {
                for (b, sum) in sums.iter_mut().enumerate() {
                    *sum += sq(i, b);
                }
            }
            if i >= BAND_SMOOTH && i - BAND_SMOOTH < raw.len() {
                for (b, sum) in sums.iter_mut().enumerate() {
                    *sum -= sq(i - BAND_SMOOTH, b);
                }
            }
            if i >= half {
                let lo = (i + 1).saturating_sub(BAND_SMOOTH);
                let n = (i.min(raw.len() - 1) + 1 - lo).max(1) as f64;
                smoothed.push(sums.map(|v| (v.max(0.0) / n).sqrt() as f32));
            }
        }
        let finest: Vec<Cell> = raw
            .iter()
            .zip(&smoothed)
            .map(|(a, b)| Cell {
                peak: q(a[0] * scale),
                rms: q(a[1] * scale),
                bands: b.map(|v| q(v * scale)),
            })
            .collect();
        let mut levels = vec![finest];
        while levels.last().is_some_and(|l| l.len() > 1) {
            let up: Vec<Cell> = levels.last().unwrap().chunks(2).map(merge).collect();
            levels.push(up);
        }
        Wave {
            frames_per_ms: rate / 1000.0 / speed.max(1e-6),
            levels,
        }
    }

    /// How long the song is (ms) by what was looked at.
    #[cfg(test)]
    pub fn len_ms(&self) -> f64 {
        self.levels.first().map_or(0, |l| l.len()) as f64 * BASE as f64 / self.frames_per_ms
    }

    /// `n` columns evenly across `from..to` (ms of the song).
    pub fn columns(&self, from: f64, to: f64, n: usize) -> Vec<Column> {
        let Some(finest) = self.levels.first().filter(|l| !l.is_empty()) else {
            return vec![Column::default(); n];
        };
        let n = n.max(1);
        let cells_per_col = (to - from) * self.frames_per_ms / BASE as f64 / n as f64;
        let mut out = Vec::with_capacity(n);
        if cells_per_col <= 1.0 {
            // Closer than the finest cells: between their middles, so it is smooth, not steps.
            for k in 0..n {
                let t = from + (to - from) * (k as f64 + 0.5) / n as f64;
                let at = t * self.frames_per_ms / BASE as f64 - 0.5;
                if at < -0.5 || at > finest.len() as f64 - 0.5 {
                    out.push(Column::default());
                    continue;
                }
                let i = at.floor().max(0.0) as usize;
                let (a, b) = (
                    finest[i.min(finest.len() - 1)],
                    finest[(i + 1).min(finest.len() - 1)],
                );
                let w = (at - i as f64).clamp(0.0, 1.0) as f32;
                out.push(column(lerp(a, b, w)));
            }
            return out;
        }
        // The coarsest level whose cells still fit in a column, and the cells each one covers.
        let level = (cells_per_col.log2().floor() as usize).min(self.levels.len() - 1);
        let cells = &self.levels[level];
        let per_ms = self.frames_per_ms / (BASE << level) as f64;
        for k in 0..n {
            let t0 = from + (to - from) * k as f64 / n as f64;
            let t1 = from + (to - from) * (k + 1) as f64 / n as f64;
            let (i0, i1) = ((t0 * per_ms).floor(), (t1 * per_ms).ceil());
            if i1 <= 0.0 || i0 >= cells.len() as f64 {
                out.push(Column::default());
                continue;
            }
            let (i0, i1) = (i0.max(0.0) as usize, (i1 as usize).min(cells.len()));
            out.push(column(merge(&cells[i0..i1.max(i0 + 1)])));
        }
        out
    }

    /// The columns drawn for `from..to` in `n` across: on a grid fixed to the song's time (the
    /// column width), not to `from`, so a view that scrolls by a fraction of a column shows the
    /// same columns moved over instead of new ones (which flickered). Each comes with its start
    /// (ms); one before and after the view are added, so the edges smooth like the rest.
    pub fn grid_columns(&self, from: f64, to: f64, n: usize) -> Vec<(f64, Column)> {
        let n = n.max(1);
        let dt = (to - from) / n as f64;
        if dt <= 0.0 || !dt.is_finite() || !from.is_finite() {
            return Vec::new();
        }
        let start = ((from / dt).floor() - 1.0) * dt;
        let count = n + 3;
        let cols = smooth(&self.columns(start, start + count as f64 * dt, count));
        cols.into_iter()
            .enumerate()
            .map(|(k, c)| (start + k as f64 * dt, c))
            .collect()
    }

    /// Draws `from..to` (ms of the song) across `rect`, mirrored about its middle, at `alpha`.
    pub fn draw(&self, c: &mut Canvas, [x, y, w, h]: [f32; 4], from: f64, to: f64, alpha: f32) {
        if w <= 0.0 || h <= 0.0 || to <= from {
            return;
        }
        let n = ((w * COLUMNS_PER_UNIT) as usize).clamp(1, 4000);
        let cw = w / n as f32;
        let x_of = |t: f64| x + ((t - from) / (to - from)) as f32 * w;
        let (mid, half) = (y + h / 2.0, h / 2.0 - 1.0);
        for (t, col) in self.grid_columns(from, to, n) {
            // Cut to the rect: the grid's first and last columns stick out of it.
            let left = x_of(t).max(x);
            // A hair wider than the column, so neighbours meet without seams.
            let right = (x_of(t) + cw + 0.15).min(x + w);
            if right <= left {
                continue;
            }
            let outer = shape(col.peak) * half;
            if outer < 0.15 {
                continue;
            }
            let inner = shape((col.rms * BODY).min(col.peak)) * half;
            let [r, g, b] = col.colour;
            c.rect(
                left,
                mid - outer,
                right - left,
                outer * 2.0,
                [r, g, b, 0.42 * alpha],
            );
            if inner > 0.15 {
                c.rect(
                    left,
                    mid - inner,
                    right - left,
                    inner * 2.0,
                    [r, g, b, 0.95 * alpha],
                );
            }
        }
        c.rect(x, mid - 0.25, w, 0.5, rgba(0xffffff, 0.08 * alpha));
    }
}

/// Loudness as drawn: quiet parts are lifted a little so they still show.
fn shape(v: f32) -> f32 {
    v.clamp(0.0, 1.0).powf(0.9)
}

fn merge(cells: &[Cell]) -> Cell {
    let n = cells.len().max(1) as f32;
    let mean_sq = |get: &dyn Fn(&Cell) -> u16| {
        (cells.iter().map(|c| f(get(c)).powi(2)).sum::<f32>() / n).sqrt()
    };
    Cell {
        peak: cells.iter().map(|c| c.peak).max().unwrap_or(0),
        rms: q(mean_sq(&|c| c.rms)),
        bands: [
            q(mean_sq(&|c| c.bands[0])),
            q(mean_sq(&|c| c.bands[1])),
            q(mean_sq(&|c| c.bands[2])),
        ],
    }
}

fn lerp(a: Cell, b: Cell, w: f32) -> Cell {
    let l = |x: u16, y: u16| q(f(x) + (f(y) - f(x)) * w);
    Cell {
        peak: l(a.peak, b.peak),
        rms: l(a.rms, b.rms),
        bands: [
            l(a.bands[0], b.bands[0]),
            l(a.bands[1], b.bands[1]),
            l(a.bands[2], b.bands[2]),
        ],
    }
}

fn column(cell: Cell) -> Column {
    let w: Vec<f32> = (0..3).map(|i| f(cell.bands[i]) * WEIGHTS[i]).collect();
    let sum: f32 = w.iter().sum();
    let colour = if sum > 1e-6 {
        let mut c = [0.0f32; 3];
        for (band, colour) in [BASS, MIDDLE, TREBLE].iter().enumerate() {
            for ch in 0..3 {
                c[ch] += colour[ch] * w[band] / sum;
            }
        }
        c
    } else {
        MIDDLE
    };
    // A little more colour than the plain mix, which greys out between the bands.
    let grey = (colour[0] + colour[1] + colour[2]) / 3.0;
    let colour = colour.map(|v| (grey + (v - grey) * 1.35).clamp(0.0, 1.0));
    Column {
        peak: f(cell.peak),
        rms: f(cell.rms),
        colour,
    }
}

/// Softens column to column jitter (a quarter of each neighbour), keeping single hits.
fn smooth(cols: &[Column]) -> Vec<Column> {
    (0..cols.len())
        .map(|i| {
            let a = cols[i.saturating_sub(1)];
            let b = cols[i];
            let c = cols[(i + 1).min(cols.len() - 1)];
            let mix = |x: f32, y: f32, z: f32| 0.25 * x + 0.5 * y + 0.25 * z;
            Column {
                peak: mix(a.peak, b.peak, c.peak).max(b.peak * 0.85),
                rms: mix(a.rms, b.rms, c.rms),
                colour: b.colour,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` of a sine at `hz` (stereo, `rate`), then as long of silence.
    fn tone(hz: f64, rate: f64, secs: f64) -> Vec<f32> {
        let n = (rate * secs) as usize;
        let mut s = Vec::with_capacity(n * 4);
        for i in 0..n {
            let v = (std::f64::consts::TAU * hz * i as f64 / rate).sin() as f32 * 0.5;
            s.extend([v, v]);
        }
        s.extend(std::iter::repeat_n(0.0, n * 2));
        s
    }

    #[test]
    fn loud_and_quiet_land_where_they_are_at_any_zoom() {
        let w = Wave::build(&tone(440.0, 48_000.0, 1.0), 48_000.0, 1.0);
        assert!((w.len_ms() - 2000.0).abs() < 5.0);
        for n in [4, 50, 2000, 40_000] {
            let cols = w.columns(0.0, 2000.0, n);
            assert_eq!(cols.len(), n);
            let (first, last) = (cols[n / 8], cols[n - 1 - n / 8]);
            assert!(first.peak > 0.9 && first.rms > 0.5, "{n}: {first:?}");
            assert!(last.peak < 0.01, "{n}: {last:?}");
        }
        // Past the end, and before the start: nothing.
        assert_eq!(w.columns(5000.0, 6000.0, 3)[1], Column::default());
        assert_eq!(w.columns(-900.0, -100.0, 3)[1], Column::default());
    }

    #[test]
    fn bass_is_red_and_treble_is_blue() {
        let red = |hz: f64| {
            let w = Wave::build(&tone(hz, 48_000.0, 0.5), 48_000.0, 1.0);
            let c = w.columns(100.0, 400.0, 1)[0].colour;
            c[0] - c[2]
        };
        assert!(red(60.0) > 0.3, "a kick drum's bass leans red");
        assert!(red(6000.0) < -0.3, "a hi-hat's treble leans blue");
    }

    #[test]
    fn a_track_for_half_speed_covers_the_same_song() {
        // Twice as many frames for the same song, at half speed.
        let slow = Wave::build(&tone(440.0, 96_000.0, 1.0), 48_000.0, 0.5);
        assert!((slow.len_ms() - 2000.0).abs() < 5.0);
        assert!(slow.columns(0.0, 2000.0, 10)[2].peak > 0.9);
        assert!(slow.columns(0.0, 2000.0, 10)[8].peak < 0.01);
    }

    #[test]
    fn silence_and_nothing_draw_nothing() {
        let w = Wave::build(&[], 48_000.0, 1.0);
        assert_eq!(w.columns(0.0, 1000.0, 5), vec![Column::default(); 5]);
        let quiet = Wave::build(&vec![0.0; 9600], 48_000.0, 1.0);
        assert!(quiet.columns(0.0, 100.0, 5).iter().all(|c| c.peak == 0.0));
        let mut canvas = Canvas::default();
        quiet.draw(&mut canvas, [0.0, 0.0, 100.0, 40.0], 0.0, 100.0, 1.0);
    }

    #[test]
    fn a_view_scrolled_a_little_shows_the_same_columns_moved_over() {
        // Loud and quiet in turn, so a column cut differently would read differently.
        let mut samples = Vec::new();
        for k in 0..48_000 * 2 {
            let loud = if (k / 700) % 3 == 0 { 0.9 } else { 0.15 };
            let v = loud * ((k as f64) * 0.05).sin() as f32;
            samples.extend([v, v]);
        }
        let w = Wave::build(&samples, 48_000.0, 1.0);
        for (span, n) in [(1500.0, 400), (300.0, 900)] {
            let a = w.grid_columns(200.0, 200.0 + span, n);
            for shift in [0.37, 1.9, 2.6] {
                let b = w.grid_columns(200.0 + shift, 200.0 + shift + span, n);
                for (t, col) in &b[2..b.len() - 2] {
                    let same = a.iter().find(|(u, _)| (u - t).abs() < 1e-6);
                    if let Some((_, was)) = same {
                        assert_eq!(
                            was, col,
                            "column at {t} ms changed when the view moved {shift} ms"
                        );
                    }
                }
                let shared = b
                    .iter()
                    .filter(|(t, _)| a.iter().any(|(u, _)| (u - t).abs() < 1e-6));
                assert!(shared.count() > n / 2);
            }
        }
    }
}
