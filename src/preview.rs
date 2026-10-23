//! Song select preview: the selected map's background, its music from the preview point, and a
//! live autoplay of the selected difficulty with its details, all loaded in the background.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audio::{self, Engine, Track};
use crate::beatmap::{self, Beatmap, Kind};
use crate::difficulty::{self, Graph};
use crate::game::{self, Game};
use crate::library::Entry;
use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

/// While scrolling, wait this long after the selection settles before loading, so scrolling
/// stays cheap. A single change after a quiet moment loads at once.
const DEBOUNCE: Duration = Duration::from_millis(150);
const QUIET: Duration = Duration::from_millis(300);
/// The preview music starts from a quick decode of this much audio at the preview point; the
/// full song replaces it a moment later without a gap.
const SNIPPET_MS: f64 = 15_000.0;

pub type Image = (u32, u32, Vec<u8>);

/// Decoded music for the selected set: all of it, or (until the full decode arrives) the part
/// from the preview point, which belongs at song frame `base`.
struct Song {
    path: PathBuf,
    track: Arc<Track>,
    base: i64,
    full: bool,
    /// Where the preview starts (track ms).
    start: f64,
    /// Speed it was prepared for (the track plays `rate` map ms per ms) and how.
    rate: f64,
    keep_pitch: bool,
}

impl Song {
    /// End of the decoded audio, in song ms.
    fn end_ms(&self, rate: u32) -> f64 {
        (self.base + self.track.frames() as i64) as f64 / rate as f64 * 1000.0
    }
}

/// A decode result: (track, base frame, whether it is the whole song), or None if it failed.
type Decoded = (PathBuf, Option<(Arc<Track>, i64, bool)>);

pub struct Details {
    pub bpm: (f64, f64),
    pub length_ms: f64,
    pub dons: usize,
    pub kats: usize,
    pub rolls: usize,
    pub swells: usize,
}

impl Details {
    fn of(map: &Beatmap) -> Details {
        let count = |k: Kind| map.notes.iter().filter(|n| n.kind == k).count();
        let first = map.notes.first().map(|n| n.time).unwrap_or(0.0);
        let last = map.notes.iter().map(|n| n.end_time).fold(0.0, f64::max);
        let bpms: Vec<f64> = map
            .control_points
            .iter()
            .filter(|p| p.time <= last)
            .map(|p| 60000.0 / p.beat_length)
            .collect();
        let bpm = bpms.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), &b| (lo.min(b), hi.max(b)));
        Details {
            bpm: if bpms.is_empty() { (0.0, 0.0) } else { bpm },
            length_ms: last - first,
            dons: count(Kind::Don),
            kats: count(Kind::Kat),
            rolls: count(Kind::Roll),
            swells: count(Kind::Swell),
        }
    }
}

#[derive(Default)]
pub struct Preview {
    seen: Option<usize>,
    changed_at: Option<Instant>,
    last_change: Option<Instant>,
    requested: Option<usize>,
    map_rx: Option<Receiver<(usize, Result<Beatmap, String>, Option<(Vec<f64>, f64, f64)>)>>,
    /// Speed (and keep-pitch) the loaded preview was prepared for.
    requested_rate: (f64, bool),
    /// After a speed change: map time to continue the music from once the new track is ready.
    resume_at: Option<f64>,
    bg_rx: Option<Receiver<(PathBuf, Option<Image>)>>,
    audio_rx: Option<Receiver<Decoded>>,
    map: Option<(usize, Beatmap, Details, Graph)>,
    /// Waiting for the music to reach a seek target before rebuilding the autoplay preview.
    pending_seek: Option<f64>,
    /// Where the difficulty timeline was drawn (virtual units) and the time range it spans.
    graph_rect: Option<[f32; 4]>,
    timeline: (f64, f64),
    /// Start point: where the timeline was last scrolled to (song file, map time). Kept while
    /// browsing the difficulties of that song.
    marker: Option<(PathBuf, f64)>,
    /// Where the "Play from" button was drawn.
    play_rect: Option<[f32; 4]>,
    /// Where the autoplay box was drawn (clicking it opens the fullscreen preview).
    box_rect: Option<[f32; 4]>,
    /// When the shown map's preview became ready (it fades in).
    ready_at: Option<Instant>,
    game: Option<Game>,
    last_time: f64,
    bg_path: Option<PathBuf>,
    /// New background for the renderer: `Some(None)` clears it.
    pub bg_upload: Option<Option<Image>>,
    pub has_background: bool,
    audio: Option<Song>,
    audio_active: bool,
}

fn audio_path(e: &Entry) -> PathBuf {
    e.path.parent().unwrap_or(Path::new(".")).join(&e.meta.audio_file)
}

fn background_path(e: &Entry) -> Option<PathBuf> {
    e.meta.background.as_ref().map(|b| e.path.parent().unwrap_or(Path::new(".")).join(b))
}

/// Decodes a background, shrunk to at most 1920x1200 (GPU texture limits, memory).
fn load_image(path: &Path) -> Option<Image> {
    let img = image::open(path).ok()?;
    let img = if img.width() > 1920 || img.height() > 1200 { img.resize(1920, 1200, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

impl Preview {
    /// Something else (gameplay, calibration) took over the audio engine.
    pub fn invalidate_audio(&mut self) {
        self.audio_active = false;
    }

    /// The decoded song, for reuse when that map is played at the same speed.
    pub fn cached_track(&self, e: &Entry, rate: f64, keep_pitch: bool) -> Option<Arc<Track>> {
        let same = |s: &&Song| s.full && s.path == audio_path(e) && (s.rate - rate).abs() < 1e-6 && (s.keep_pitch == keep_pitch || (rate - 1.0).abs() < 1e-6);
        self.audio.as_ref().filter(same).map(|s| s.track.clone())
    }

    /// Makes sure the background shown is `e`'s (e.g. a multiplayer map picked by the host).
    pub fn show_background_for(&mut self, e: &Entry) {
        let path = background_path(e);
        if path == self.bg_path {
            return;
        }
        self.bg_path = path.clone();
        match path {
            Some(p) => {
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let img = load_image(&p);
                    let _ = tx.send((p, img));
                });
                self.bg_rx = Some(rx);
            }
            None => {
                self.bg_upload = Some(None);
                self.has_background = false;
            }
        }
    }

    /// Call every frame. `in_menu`: menus where preview music keeps playing; `in_select`: the
    /// autoplay preview is visible and runs.
    #[allow(clippy::too_many_arguments)]
    /// `rate`: speed the music and the autoplay play at (and whether it keeps the pitch).
    pub fn update(&mut self, now: Instant, library: &[Entry], selected: Option<usize>, engine: Option<&Engine>, in_menu: bool, in_select: bool, offset: f64, rate: (f64, bool)) {
        let rate = if (rate.0 - 1.0).abs() < 1e-6 { (1.0, false) } else { rate };
        // A new speed changes the music and the strain curve: reload as if the selection
        // changed, carrying on from where the music was.
        if rate != self.requested_rate && self.requested.is_some() {
            if self.audio_active && self.requested == selected {
                // Where the music is now, in map time (the playing track's own speed).
                let playing = engine.and_then(|e| e.position_ms(now)).zip(self.audio.as_ref()).map(|(p, s)| p * s.rate);
                self.resume_at = self.pending_seek.or(playing).or(self.resume_at);
            }
            self.requested = None;
            self.changed_at = Some(now.checked_sub(DEBOUNCE).unwrap_or(now));
        }
        if selected != self.seen {
            self.seen = selected;
            self.resume_at = None;
            let quiet = self.last_change.is_none_or(|t| now - t >= QUIET);
            self.last_change = Some(now);
            self.changed_at = Some(if quiet { now.checked_sub(DEBOUNCE).unwrap_or(now) } else { now });
        }
        // Start loads once the selection has settled.
        if self.seen != self.requested && self.changed_at.is_some_and(|t| now - t >= DEBOUNCE) {
            self.requested = self.seen;
            self.requested_rate = rate;
            self.game = None;
            let (speed, keep_pitch) = rate;
            if let Some(i) = self.seen.filter(|&i| i < library.len()) {
                let e = &library[i];
                let path = e.path.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let map = beatmap::load(&path);
                    let first = map.as_ref().ok().and_then(|m| m.notes.first()).map(|n| n.time).unwrap_or(0.0);
                    let curve = std::panic::catch_unwind(|| crate::stars::strain_curve(&path, speed, first)).ok().flatten();
                    let _ = tx.send((i, map, curve));
                });
                self.map_rx = Some(rx);
                self.show_background_for(e);
                let song = audio_path(e);
                let loaded = self.audio.as_ref().is_some_and(|s| s.path == song && (s.rate, s.keep_pitch) == rate);
                if !loaded {
                    if let Some(sr) = engine.map(|a| a.rate) {
                        let (tx, rx) = mpsc::channel();
                        // Quick part first: from where the music was (speed change) or the preview point.
                        let part_from = self.resume_at.unwrap_or(e.meta.preview_time);
                        std::thread::spawn(move || {
                            if part_from >= 0.0 {
                                // SNIPPET_MS of playback covers that much more map time when sped up.
                                if let Ok((part, base)) = audio::decode_part(&song, sr, Some((part_from, SNIPPET_MS * speed))) {
                                    let part = audio::change_rate(part, sr, speed, keep_pitch);
                                    let base = (base as f64 / speed).round() as i64;
                                    if tx.send((song.clone(), Some((Arc::new(part), base, false)))).is_err() {
                                        return; // the selection moved on
                                    }
                                }
                            }
                            let track = audio::decode(&song, sr).ok().map(|t| (Arc::new(audio::change_rate(t, sr, speed, keep_pitch)), 0, true));
                            let _ = tx.send((song, track));
                        });
                        self.audio_rx = Some(rx);
                    }
                }
            }
        }

        if let Some(rx) = &self.map_rx {
            if let Ok((i, result, curve)) = rx.try_recv() {
                self.map_rx = None;
                if Some(i) == self.requested {
                    if let Ok(map) = result {
                        let details = Details::of(&map);
                        let mut graph = difficulty::analyze(&map);
                        if let Some(curve) = curve {
                            graph.use_curve(curve);
                        }
                        let same_map = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i);
                        if !same_map {
                            self.ready_at = Some(Instant::now());
                        }
                        self.game = Some(new_preview_game(&map, self.requested_rate.0));
                        self.last_time = f64::NEG_INFINITY;
                        self.map = Some((i, map, details, graph));
                    }
                }
            }
        }
        if let Some(rx) = &self.bg_rx {
            if let Ok((path, img)) = rx.try_recv() {
                self.bg_rx = None;
                if Some(&path) == self.bg_path.as_ref() {
                    self.has_background = img.is_some();
                    self.bg_upload = Some(img);
                }
            }
        }
        let decoded: Vec<Decoded> = self.audio_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for (path, result) in decoded {
            if !matches!(result, Some((_, _, false))) {
                self.audio_rx = None; // the full song (or a failure) is the last message
            }
            let wanted = self.requested.and_then(|i| library.get(i));
            let (Some((track, base, full)), Some(engine), Some(e)) = (result, engine, wanted.filter(|e| audio_path(e) == path)) else { continue };
            let (speed, keep_pitch) = self.requested_rate;
            // In map ms (the track plays `speed` map ms per ms).
            let len_ms = track.frames() as f64 / engine.rate as f64 * 1000.0 * speed;
            let preview_time = e.meta.preview_time;
            let start = (if preview_time >= 0.0 && (!full || preview_time < len_ms) { preview_time } else { len_ms * 0.4 }) / speed;
            // The full song takes over from the part that's already playing without a gap.
            let playing_part = self.audio.as_ref().is_some_and(|s| s.path == path && !s.full && (s.rate, s.keep_pitch) == (speed, keep_pitch));
            if full && playing_part && self.audio_active && in_menu {
                engine.swap(track.clone(), base);
            } else {
                self.audio_active = false;
            }
            self.audio = Some(Song { path, track, base, full, start, rate: speed, keep_pitch });
        }

        // Music: start at the preview point, loop back to it at the end.
        let (Some(engine), true) = (engine, in_menu) else { return };
        if let Some(song) = &self.audio {
            let end_ms = song.end_ms(engine.rate);
            let restart = !self.audio_active || engine.position_ms(now).is_some_and(|p| p > end_ms - 30.0);
            if restart {
                let at = self.resume_at.take().map(|t| t / song.rate).unwrap_or(song.start);
                engine.play_part(song.track.clone(), song.base, at);
                self.audio_active = true;
            }
        }

        // Autoplay preview, driven by the music's clock. It waits while the music is still the
        // previous speed's (its clock would read at the wrong rate).
        let stale = self.audio.as_ref().is_some_and(|s| (s.rate, s.keep_pitch) != self.requested_rate);
        if !in_select || !self.audio_active || stale {
            return;
        }
        if let Some(target) = self.pending_seek {
            // The engine applies seeks on its next callback; rebuild once the music is there so
            // the notes in between aren't lost.
            let speed = self.requested_rate.0;
            let arrived = engine.position_ms(now).is_some_and(|p| (p * speed - target).abs() < 150.0 * speed);
            if arrived {
                if let Some((_, map, _, _)) = &self.map {
                    let mut fresh = new_preview_game(map, speed);
                    fresh.update(now, Some(engine), offset);
                    fresh.skip_to(fresh.time - 50.0);
                    self.last_time = fresh.time;
                    self.game = Some(fresh);
                }
                self.pending_seek = None;
            }
            return;
        }
        let Some(game) = &mut self.game else { return };
        game.update(now, Some(engine), offset);
        if game.time < self.last_time - 1000.0 {
            // The music looped: start the preview over from the preview point.
            if let Some((_, map, _, _)) = &self.map {
                let mut fresh = new_preview_game(map, self.requested_rate.0);
                fresh.update(now, Some(engine), offset);
                fresh.skip_to(fresh.time - 50.0);
                *game = fresh;
            }
        } else if game.time > self.last_time + 2000.0 {
            // Jumped in (first frame, or the song changed): don't replay the skipped part.
            game.skip_to(game.time - 50.0);
        }
        self.last_time = game.time;
        game.autoplay(None);
    }

    /// Jumps the preview music (and the autoplay) to map time `t`, and remembers it as the
    /// start point (the blue flag): wherever you last scrolled to is where "Play from" starts.
    pub fn seek(&mut self, t: f64, engine: Option<&Engine>) {
        let (Some(engine), Some(song)) = (engine, &self.audio) else { return };
        if !self.audio_active {
            return;
        }
        // Until the full decode replaces the quick part, the song's length isn't known; seeking
        // outside the part is silent for that moment instead of being refused.
        let last = if song.full { ((song.end_ms(engine.rate) - 1000.0) * song.rate).max(0.0) } else { f64::INFINITY };
        let t = t.clamp(0.0, last);
        engine.seek(t / song.rate);
        self.pending_seek = Some(t);
        self.marker = Some((song.path.clone(), t));
    }

    pub fn seek_by(&mut self, delta_ms: f64, engine: Option<&Engine>) {
        let now = self.pending_seek.or_else(|| self.game.as_ref().map(|g| g.time));
        if let Some(t) = now {
            self.seek(t + delta_ms, engine);
        }
    }

    /// The start point picked for `e`'s song, if any.
    pub fn marker(&self, e: &Entry) -> Option<f64> {
        self.marker.as_ref().filter(|(p, _)| *p == audio_path(e)).map(|(_, t)| *t)
    }

    pub fn play_button_at(&self, x: f32, y: f32) -> bool {
        self.play_rect.is_some_and(|[bx, by, bw, bh]| x >= bx && x <= bx + bw && y >= by && y <= by + bh)
    }

    pub fn box_at(&self, x: f32, y: f32) -> bool {
        self.box_rect.is_some_and(|[bx, by, bw, bh]| x >= bx && x <= bx + bw && y >= by && y <= by + bh)
    }

    /// The autoplay preview of library entry `i`, once it has loaded.
    pub fn game_for(&mut self, i: usize) -> Option<&mut Game> {
        let loaded = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i) && self.seen == Some(i);
        self.game.as_mut().filter(|_| loaded)
    }

    /// The difficulty timeline of `e` (the selected map) in `rect`, with the start point and,
    /// if `button` is given as (verb, key hint), a "<verb> 1:23" button under it.
    pub fn draw_timeline(&mut self, c: &mut Canvas, e: &Entry, rect: [f32; 4], button: Option<(&str, &str)>) {
        self.graph_rect = None;
        self.play_rect = None;
        let loaded = self.map.as_ref().filter(|(mi, ..)| Some(*mi) == self.seen);
        let (Some((_, _, _, graph)), Some(game)) = (loaded, self.game.as_ref()) else { return };
        let marker = self.marker(e);
        self.timeline = draw_graph(c, graph, rect, game.time, e.meta.preview_time, marker);
        self.graph_rect = Some(rect);
        self.play_rect = draw_play_button(c, rect[0] + rect[2] / 2.0, rect[1] + rect[3] + 17.0, marker, button);
    }

    pub fn graph_rect(&self) -> Option<[f32; 4]> {
        self.graph_rect
    }

    /// Map time under a point of the difficulty timeline, if the point is on it.
    pub fn time_at(&self, x: f32, y: f32) -> Option<f64> {
        let [gx, gy, gw, gh] = self.graph_rect?;
        if x < gx - 2.0 || x > gx + gw + 2.0 || y < gy - 2.0 || y > gy + gh + 2.0 {
            return None;
        }
        let f = ((x - gx) / gw).clamp(0.0, 1.0) as f64;
        Some(self.timeline.0 + f * (self.timeline.1 - self.timeline.0))
    }

    /// Details and the autoplay preview in the right-hand panel (virtual units). Returns the
    /// shape range to clip and the clip area.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_panel(&mut self, c: &mut Canvas, library: &[Entry], x: f32, y: f32, w: f32, h: f32, scroll_speed: f64, stars: Option<f32>) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        // The preview shows ~640 units of lane (several notes ahead), so its height follows the
        // panel width; the panel shrinks to fit instead of leaving empty space.
        let (band_top, band_h) = game::lane_band();
        let k = (w - 20.0) / 640.0;
        let preview_h = band_h * k + 12.0;
        const GRAPH_H: f32 = 40.0;
        let h = h.min(114.0 + preview_h + 8.0 + GRAPH_H + 44.0);
        self.graph_rect = None;
        self.play_rect = None;
        self.box_rect = None;
        c.round_rect(x, y, w, h, 8.0, rgba(0x15151d, 0.82), rgba(0x2a2a38, 1.0), 1.0);
        let Some(i) = self.seen.filter(|&i| i < library.len()) else { return None };
        let e = &library[i];
        let fit = |s: &str, size: f32| -> String {
            let max = ((w - 28.0) / (size / 7.0 * 6.0)) as usize;
            if s.chars().count() > max { s.chars().take(max.saturating_sub(3)).collect::<String>() + "..." } else { s.to_string() }
        };
        c.text(x + 14.0, y + 14.0, 12.0, rgba(0xffffff, 1.0), &fit(&e.meta.title, 12.0));
        c.text(x + 14.0, y + 32.0, 9.0, rgba(0xc8c8d8, 1.0), &fit(&e.meta.artist, 9.0));
        c.text(x + 14.0, y + 46.0, 8.0, rgba(0x8a8aa0, 1.0), &fit(&format!("mapped by {}", e.meta.creator), 8.0));
        let pill = crate::stars::draw_pill(c, x + w - 12.0, y + 64.0, 9.0, stars);
        let version = fit(&format!("[{}]", e.meta.version), 9.0);
        let room = ((w - 40.0 - pill) / (9.0 / 7.0 * 6.0)) as usize;
        let version = if version.chars().count() > room { version.chars().take(room.saturating_sub(3)).collect::<String>() + "..." } else { version };
        c.text(x + 14.0, y + 64.0, 9.0, rgba(0xEB452C, 1.0), &version);
        let loaded = self.map.as_ref().filter(|(mi, _, _, _)| *mi == i);
        let line = match loaded {
            Some((_, _, d, _)) => {
                let bpm = if (d.bpm.1 - d.bpm.0).abs() < 0.5 { format!("{:.0} BPM", d.bpm.0) } else { format!("{:.0}-{:.0} BPM", d.bpm.0, d.bpm.1) };
                let secs = (d.length_ms / 1000.0).max(0.0) as u64;
                format!("OD {:.1}   {bpm}   {}:{:02}", e.meta.od, secs / 60, secs % 60)
            }
            None => format!("OD {:.1}", e.meta.od),
        };
        c.text(x + 14.0, y + 80.0, 8.0, rgba(0xc8c8d8, 1.0), &line);
        if let Some((_, _, d, _)) = loaded {
            let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
            let notes = format!("{} ({}/{} don/kat)  {}  {}", plural(d.dons + d.kats, "note"), d.dons, d.kats, plural(d.rolls, "roll"), plural(d.swells, "swell"));
            c.text(x + 14.0, y + 94.0, 8.0, rgba(0x8a8aa0, 1.0), &fit(&notes, 8.0));
        }

        // Autoplay preview: just the lane band of the playfield, fitted to the box.
        let (bx, by, bw, bh) = (x + 10.0, y + 114.0, w - 20.0, preview_h);
        c.round_rect(bx, by, bw, bh, 6.0, rgba(0x0c0c12, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        self.box_rect = Some([bx, by, bw, bh]);
        let loaded = loaded.is_some();
        self.draw_timeline(c, e, [bx, by + bh + 8.0, bw, GRAPH_H], Some(("Play from", "Shift+Enter")));
        let game = self.game.as_mut().filter(|_| loaded)?;
        let full_w = c.width;
        let start = c.shapes.len();
        c.width = bw / k;
        game.draw_playfield(c, scroll_speed);
        let oy = by + (bh - band_h * k) / 2.0 - band_top * k;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        c.text_right(bx + bw - 6.0, by + bh - 10.0, 6.0, rgba(0x6f6f86, 1.0), "click: fullscreen (Ctrl+Enter)");
        if let Some(at) = self.ready_at {
            let t = crate::render::progress(at, 220.0);
            if t < 1.0 {
                c.round_rect(bx, by, bw, bh, 6.0, rgba(0x0c0c12, 1.0 - t), rgba(0, 0.0), 0.0);
            }
        }
        let clip = crate::layout::Area { x: bx / full_w, y: by / VIRTUAL_HEIGHT, w: bw / full_w, h: bh / VIRTUAL_HEIGHT };
        Some((start..c.shapes.len(), clip))
    }
}

fn fmt_time(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Difficulty timeline like lazer's song progress graph: strain bars over the map's length,
/// kiai tinted, the preview point marked, and a playhead. Returns the time range it spans.
pub fn draw_graph(c: &mut Canvas, g: &Graph, [x, y, w, h]: [f32; 4], now: f64, preview_at: f64, marker: Option<f64>) -> (f64, f64) {
    let (t0, t1) = (g.start.min(0.0).max(g.start - 2000.0), g.end);
    let span = (t1 - t0).max(1.0);
    let x_of = |t: f64| x + (((t - t0) / span).clamp(0.0, 1.0) as f32) * w;
    c.round_rect(x, y, w, h, 5.0, rgba(0x0c0c12, 1.0), rgba(0x2a2a38, 1.0), 1.0);
    for &(s, e) in &g.kiai {
        c.rect(x_of(s), y + 1.0, (x_of(e) - x_of(s)).max(1.0), h - 2.0, rgba(0xff8a3d, 0.12));
    }
    // Bars cover the strain sections; the axis starts a little earlier for the intro.
    let (bx0, bx1) = (x_of(g.peaks_start), x_of(g.peaks_start + g.peaks.len() as f64 * g.section));
    let n = (((bx1 - bx0) / 2.5) as usize).max(1);
    let bar_w = (bx1 - bx0) / n as f32;
    for (i, v) in g.bins(n).into_iter().enumerate() {
        let bh = 2.0 + v * (h - 8.0);
        // Cool blue for easy parts, through yellow, to red at the peaks.
        let lerp = |a: u32, b: u32, t: f32| -> u32 {
            let ch = |s: u32| ((a >> s & 0xff) as f32 + ((b >> s & 0xff) as f32 - (a >> s & 0xff) as f32) * t) as u32;
            (ch(16) << 16) | (ch(8) << 8) | ch(0)
        };
        let col = if v < 0.5 { lerp(0x3d6f8c, 0xFCB806, v * 2.0) } else { lerp(0xFCB806, 0xEB452C, (v - 0.5) * 2.0) };
        let played = (bx0 + (i as f32 + 0.5) * bar_w) <= x_of(now);
        c.rect(bx0 + i as f32 * bar_w, y + h - 3.0 - bh, (bar_w - 0.5).max(0.5), bh, rgba(col, if played { 1.0 } else { 0.55 }));
    }
    if preview_at >= 0.0 {
        let px = x_of(preview_at);
        c.rect(px - 0.5, y + 1.0, 1.0, 5.0, rgba(0x86E05A, 1.0));
    }
    if let Some(m) = marker {
        // The picked start point: a blue line with a little flag.
        let mx = x_of(m);
        c.rect(mx - 0.75, y - 3.0, 1.5, h + 6.0, rgba(MARKER, 1.0));
        c.rect(mx + 0.75, y - 3.0, 6.0, 4.5, rgba(MARKER, 1.0));
    }
    let hx = x_of(now);
    c.rect(hx - 0.75, y - 2.0, 1.5, h + 4.0, rgba(0xffffff, 0.95));
    c.text(x, y + h + 5.0, 7.0, rgba(0xc8c8d8, 1.0), &format!("{} / {}", fmt_time(now - t0.min(0.0)), fmt_time(t1)));
    let peak = format!("peak {:.1} notes/s at {}", g.peak_nps, fmt_time(g.peak_nps_at));
    c.text_right(x + w, y + h + 5.0, 7.0, rgba(0x8a8aa0, 1.0), &peak);
    (t0, t1)
}

pub const MARKER: u32 = 0x4fb4ff;

/// "<verb> 1:23" under the timeline once a start point is picked (else a hint). `button` is
/// (verb, key hint); None shows no button (only the host picks a match's start). Returns the
/// button's rect.
fn draw_play_button(c: &mut Canvas, cx: f32, y: f32, marker: Option<f64>, button: Option<(&str, &str)>) -> Option<[f32; 4]> {
    let (Some(t), Some((verb, hint))) = (marker, button) else {
        let text = if button.is_some() { "click / drag / wheel to scroll - play from where you stop" } else { "click / drag / wheel to scroll" };
        c.text_centered(cx, y + 4.0, 6.0, rgba(0x6f6f86, 1.0), text);
        return None;
    };
    let label = format!("{verb} {}", fmt_time(t));
    let (lw, hw) = (Canvas::text_width(8.0, &label), Canvas::text_width(6.0, hint));
    let bw = 14.0 + 10.0 + lw + 10.0 + hw + 10.0;
    let (bx, bh) = (cx - bw / 2.0, 17.0);
    c.round_rect(bx, y, bw, bh, 5.0, rgba(0x173049, 1.0), rgba(MARKER, 1.0), 1.0);
    // A play triangle out of thin slices (the font is plain ASCII).
    for i in 0..7 {
        let f = i as f32;
        c.rect(bx + 9.0 + f, y + 4.5 + f * 0.5, 1.0, 8.0 - f, rgba(0xffffff, 1.0));
    }
    c.text(bx + 24.0, y + 5.0, 8.0, rgba(0xffffff, 1.0), &label);
    c.text(bx + 24.0 + lw + 10.0, y + 6.0, 6.0, rgba(0x9ac8f0, 1.0), hint);
    Some([bx, y, bw, bh])
}

fn new_preview_game(map: &Beatmap, rate: f64) -> Game {
    // No samples: the preview only borrows the music's clock; `rate` maps it to map time.
    let mut g = Game::new(map.clone(), Arc::new(Track { samples: Vec::new(), rate }), None);
    g.mute_hits = true;
    g
}
