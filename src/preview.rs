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
use crate::render::{accent, rgba, Canvas, VIRTUAL_HEIGHT};

/// While scrolling, wait this long after the selection settles before loading, so scrolling
/// stays cheap. A single change after a quiet moment loads at once.
const DEBOUNCE: Duration = Duration::from_millis(150);
const QUIET: Duration = Duration::from_millis(300);
/// The preview music starts from a quick decode of this much audio at the preview point; the
/// full song replaces it a moment later without a gap.
const SNIPPET_MS: f64 = 15_000.0;

pub type Image = crate::render::BgImage;

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
    /// When the shown map's preview became ready (it fades in), and for which entry.
    ready_at: Option<Instant>,
    ready_for: Option<usize>,
    game: Option<Game>,
    last_time: f64,
    bg_path: Option<PathBuf>,
    /// New background for the renderer: `Some(None)` clears it.
    pub bg_upload: Option<Option<Image>>,
    pub has_background: bool,
    /// Passed on to the autoplay preview (the kiai effects setting, how autoplay hits).
    pub kiai_effects: bool,
    pub autoplay: crate::autoplay::Settings,
    /// The list is being dragged through (scrollbar, right-click scrolling): the selection
    /// races past songs, so nothing loads until it is let go. Loading each song paused on
    /// (decoding its music, parsing it) is what made dragging stutter.
    pub hold: bool,
    /// The selected map's own offset, shown with its details.
    pub map_offset: f64,
    /// The pp a full combo of the selected map gives with the mods picked, at `pp::POTENTIAL`'s
    /// accuracies (SS, 99%, 98%), shown with its details once worked out.
    pub potential: Option<[f64; 3]>,
    /// Bottom of the song select panel as last drawn (virtual units).
    pub panel_bottom: f32,
    /// How big the autoplay preview is drawn (`PREVIEW_SIZE`; 0 = full), and how much its height
    /// changes per unit of that (for dragging the panel's bottom edge).
    pub size: f32,
    pub size_unit: f32,
    /// The size it was drawn at, and the most that fits the panel's height.
    pub size_shown: f32,
    pub size_cap: f32,
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
    Some(Image::from_rgba(rgba.width(), rgba.height(), &rgba))
}

impl Preview {
    /// The entry list changes meaning (song select <-> download previews): forget what was shown.
    pub fn forget(&mut self) {
        self.seen = None;
        self.requested = None;
        self.map = None;
        self.game = None;
        self.map_rx = None;
        self.pending_seek = None;
    }

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
        if self.seen != self.requested && !self.hold && self.changed_at.is_some_and(|t| now - t >= DEBOUNCE) {
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
                        let clip_at = e.meta.preview_time.max(0.0);
                        let is_clip = song.file_name().is_some_and(|n| n.to_string_lossy().starts_with(crate::download::CLIP_PREFIX));
                        std::thread::spawn(move || {
                            if is_clip {
                                // A download preview: ~10 s of music from the preview point, placed there.
                                let clip = audio::decode(&song, sr).ok().map(|t| {
                                    let base = (clip_at / 1000.0 * sr as f64 / speed).round() as i64;
                                    (Arc::new(audio::change_rate(t, sr, speed, keep_pitch)), base, false)
                                });
                                let failed = clip.is_none();
                                let _ = tx.send((song.clone(), clip));
                                if !failed {
                                    let _ = tx.send((song, None));
                                }
                                return;
                            }
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
                            let track = audio::decode_shared(&song, sr).ok().map(|t| (audio::with_rate(t, sr, speed, keep_pitch), 0, true));
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
        // previous song's or speed's (its clock would put the notes in the wrong place).
        let wanted = self.requested.and_then(|i| library.get(i)).map(audio_path);
        let stale = self.audio.as_ref().is_none_or(|s| (s.rate, s.keep_pitch) != self.requested_rate || Some(&s.path) != wanted.as_ref());
        if !in_select || !self.audio_active || stale {
            return;
        }
        let speed = self.requested_rate.0;
        if let Some(target) = self.pending_seek {
            // The engine applies seeks on its next callback; rebuild once the music is there so
            // the notes in between aren't lost.
            let arrived = engine.position_ms(now).is_some_and(|p| (p * speed - target).abs() < 150.0 * speed);
            if arrived {
                if let Some((_, map, _, _)) = &self.map {
                    let mut fresh = new_preview_game(map, speed);
                    fresh.skip_to(target - 50.0);
                    fresh.update(now, Some(engine), offset);
                    self.last_time = fresh.time;
                    self.game = Some(fresh);
                }
                self.pending_seek = None;
            }
            return;
        }
        let Some(game) = &mut self.game else { return };
        // Where the music is, in map time, before the game expires anything: notes skipped over
        // are marked played first, so jumping in never shows misses.
        if let Some(t) = engine.position_ms(now).map(|p| (p - offset) * speed) {
            if t < self.last_time - 1000.0 {
                // The music looped: start the preview over from the preview point.
                if let Some((_, map, _, _)) = &self.map {
                    *game = new_preview_game(map, speed);
                    game.skip_to(t - 50.0);
                }
            } else if t > self.last_time + 2000.0 {
                // Jumped in (first frame, or the song changed): don't replay the skipped part.
                game.skip_to(t - 50.0);
            }
        }
        game.update(now, Some(engine), offset);
        self.last_time = game.time;
        game.autoplay_style = self.autoplay;
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

    /// Whether entry `i`'s preview can be shown: its map is loaded and its own music is playing
    /// (until then the autoplay would run on the previous song's clock).
    fn ready(&self, library: &[Entry], i: usize) -> bool {
        let Some(e) = library.get(i) else { return false };
        let map = self.map.as_ref().is_some_and(|(mi, ..)| *mi == i) && self.seen == Some(i) && self.game.is_some();
        let music = self.audio_active
            && self.audio.as_ref().is_some_and(|s| s.path == audio_path(e) && (s.rate, s.keep_pitch) == self.requested_rate);
        map && music && self.pending_seek.is_none()
    }

    /// The autoplay preview of library entry `i`, once it is ready.
    pub fn game_for(&mut self, library: &[Entry], i: usize) -> Option<&mut Game> {
        let ready = self.ready(library, i);
        self.game.as_mut().filter(|_| ready)
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
    /// `button`: the action under the timeline as (verb, key hint), e.g. ("Play from", "Shift+Enter").
    /// `status`: the map's on osu! (label, colour), shown by the mapper's name.
    pub fn draw_panel(&mut self, c: &mut Canvas, library: &[Entry], x: f32, y: f32, w: f32, h: f32, scroll_speed: f64, stars: Option<f32>, button: Option<(&str, &str)>, status: Option<(&str, u32)>) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        // The preview shows ~640 units of lane (several notes ahead), so its height follows the
        // panel width; the panel shrinks to fit instead of leaving empty space.
        let (band_top, band_h) = game::lane_band();
        const GRAPH_H: f32 = 40.0;
        // Under the preview box: the timeline and its button.
        let after_box = 8.0 + GRAPH_H + 44.0;
        // The preview's height per unit of its size. Smaller, it shows more of the lane ahead in
        // a shorter box: as small as dragged, and smaller still where the panel would otherwise
        // be taller than `h` (the details above it being `head` tall).
        let unit = band_h * (w - 20.0) / 640.0;
        self.size_unit = unit;
        let wanted = if self.size > 0.0 { self.size.clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1) } else { PREVIEW_SIZE.1 };
        let fitted = |head: f32| ((h - head - after_box - 12.0) / unit).clamp(PREVIEW_SIZE.0, PREVIEW_SIZE.1);
        self.graph_rect = None;
        self.play_rect = None;
        self.box_rect = None;
        let Some(i) = self.seen.filter(|&i| i < library.len()) else {
            (self.size_cap, self.size_shown) = (fitted(110.0), wanted.min(fitted(110.0)));
            let below = unit * self.size_shown + 12.0 + after_box;
            self.panel_bottom = y + h.min(110.0 + below);
            c.round_rect(x, y, w, h.min(110.0 + below), 8.0, rgba(0x15151d, 0.82), rgba(0x2a2a38, 1.0), 1.0);
            return None;
        };
        let e = &library[i];
        // Map details, the autoplay and the timeline appear together, once the map's music plays.
        let ready = self.ready(library, i) || (self.ready_for == Some(i) && self.map.as_ref().is_some_and(|(mi, ..)| *mi == i));
        if ready && self.ready_for != Some(i) {
            self.ready_for = Some(i);
            self.ready_at = Some(Instant::now());
        }
        let fade = self.ready_at.filter(|_| ready).map(|t| crate::render::progress(t, 180.0)).unwrap_or(0.0);
        let loaded = self.map.as_ref().filter(|(mi, _, _, _)| *mi == i && ready);

        // Text sits on a 14 unit inset; long names wrap onto a second line before they are cut.
        let (lx, iw) = (x + 14.0, w - 28.0);
        let (th_title, th_body, th_small) = (Canvas::text_height(12.0), Canvas::text_height(9.0), Canvas::text_height(8.0));
        let title = wrap(12.0, iw, &e.meta.title, 2);
        let artist = wrap(9.0, iw, &e.meta.artist, 2);
        // The star pill is about this wide; the difficulty name wraps beside it.
        let pill_w = Canvas::text_width(9.0, "0.00") + 9.0 * 1.6 + 6.0;
        let version = wrap(9.0, iw - pill_w - 8.0, &format!("[{}]", e.meta.version), 2);
        // Stats as label + value, flowed left to right; the first group is the map's, the second its notes.
        let mut stats: [Vec<(&str, String, f32)>; 3] = [vec![("OD", format!("{:.1}", e.meta.od), 1.0)], Vec::new(), Vec::new()];
        // What a full combo would give: SS, 99%, 98%.
        if let Some(pp) = self.potential {
            for (label, pp) in ["SS", "99%", "98%"].into_iter().zip(pp) {
                stats[2].push((label, format!("{:.0}pp", pp), 1.0));
            }
        }
        if let Some((_, _, d, _)) = loaded {
            // At the speed played (DT, HT, the rate): as fast and as long as it'll be.
            let speed = self.requested_rate.0.max(0.01);
            let bpm = if (d.bpm.1 - d.bpm.0).abs() < 0.5 { format!("{:.0}", d.bpm.0 * speed) } else { format!("{:.0}-{:.0}", d.bpm.0 * speed, d.bpm.1 * speed) };
            stats[0].push(("BPM", bpm, fade));
            stats[0].push(("Length", fmt_time(d.length_ms / speed), fade));
            if self.map_offset != 0.0 {
                stats[0].push(("Offset", format!("{:+.0} ms", self.map_offset), fade));
            }
            stats[1].push(("Notes", (d.dons + d.kats).to_string(), fade));
            stats[1].push(("Don", d.dons.to_string(), fade));
            stats[1].push(("Kat", d.kats.to_string(), fade));
            if d.rolls > 0 {
                stats[1].push(("Rolls", d.rolls.to_string(), fade));
            }
            if d.swells > 0 {
                stats[1].push(("Swells", d.swells.to_string(), fade));
            }
        }
        let cell_w = |(label, value, _): &(&str, String, f32)| Canvas::text_width(8.0, &format!("{label} {value}"));
        let rows: Vec<Vec<Vec<&(&str, String, f32)>>> = stats
            .iter()
            .map(|group| {
                let mut rows: Vec<Vec<&(&str, String, f32)>> = vec![Vec::new()];
                let mut used = 0.0;
                for cell in group {
                    let cw = cell_w(cell);
                    if used > 0.0 && used + STAT_GAP + cw > iw {
                        rows.push(Vec::new());
                        used = 0.0;
                    }
                    used += if used > 0.0 { STAT_GAP } else { 0.0 } + cw;
                    rows.last_mut().unwrap().push(cell);
                }
                rows
            })
            .collect();
        // Three rows are always kept (the pp one too), so the panel doesn't jump when the details
        // arrive.
        let stat_rows = rows.iter().map(Vec::len).sum::<usize>().max(3);
        let (title_step, body_step, row_h) = (th_title + 4.0, th_body + 3.0, th_small + 4.0);
        let head = 12.0 + title.len() as f32 * title_step + 3.0 + artist.len() as f32 * body_step + 4.0 + 13.0 + 6.0 + (version.len() as f32 * body_step).max(15.0) + 6.0 + stat_rows as f32 * row_h + 6.0;
        (self.size_cap, self.size_shown) = (fitted(head), wanted.min(fitted(head)));
        let k = (w - 20.0) / 640.0 * self.size_shown;
        let preview_h = band_h * k + 12.0;
        let h = h.min(head + preview_h + after_box);
        self.panel_bottom = y + h;
        c.round_rect(x, y, w, h, 8.0, rgba(0x15151d, 0.82), rgba(0x2a2a38, 1.0), 1.0);

        let mut cy = y + 12.0;
        for line in &title {
            c.text(lx, cy, 12.0, rgba(0xffffff, 1.0), line);
            cy += title_step;
        }
        cy += 3.0;
        for line in &artist {
            c.text(lx, cy, 9.0, rgba(0xc8c8d8, 1.0), line);
            cy += body_step;
        }
        cy += 4.0;
        // The status on osu! at the right of the mapper's line.
        let status_w = match status.filter(|s| !s.0.is_empty()) {
            Some((label, col)) => {
                let lw = Canvas::text_width(7.0, label) + 12.0;
                let sx = x + w - 14.0 - lw;
                c.round_rect(sx, cy, lw, 13.0, 6.5, rgba(col, 0.15), rgba(col, 0.9), 1.0);
                c.text_centered(sx + lw / 2.0, cy + 3.0, 7.0, rgba(col, 1.0), label);
                lw + 8.0
            }
            None => 0.0,
        };
        let mapper = Canvas::fit(8.0, iw - status_w, &format!("mapped by {}", e.meta.creator));
        c.text(lx, cy + (13.0 - th_small) / 2.0, 8.0, rgba(0x8a8aa0, 1.0), &mapper);
        cy += 13.0 + 6.0;
        crate::stars::draw_pill(c, x + w - 14.0, cy + 3.0, 9.0, stars);
        for line in &version {
            c.text(lx, cy + 3.0, 9.0, rgba(accent(), 1.0), line);
            cy += body_step;
        }
        cy += (15.0 - version.len() as f32 * body_step).max(0.0) + 6.0;
        // Muted labels, bright values.
        for group in &rows {
            for row in group {
                let mut sx = lx;
                for (label, value, alpha) in row.iter().copied() {
                    sx += c.text(sx, cy, 8.0, rgba(0x8a8aa0, *alpha), &format!("{label} "));
                    sx += c.text(sx, cy, 8.0, rgba(0xe8e8f4, *alpha), value) + STAT_GAP;
                }
                cy += row_h;
            }
        }
        cy = cy.max(y + 12.0) + (stat_rows.saturating_sub(rows.iter().map(Vec::len).sum()) as f32) * row_h + 6.0;

        // Autoplay preview: just the lane band of the playfield, fitted to the box.
        let (bx, by, bw, bh) = (x + 10.0, cy, w - 20.0, preview_h);
        c.round_rect(bx, by, bw, bh, 6.0, rgba(0x0c0c12, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        self.box_rect = Some([bx, by, bw, bh]);
        let loaded = loaded.is_some();
        if loaded {
            self.draw_timeline(c, e, [bx, by + bh + 8.0, bw, GRAPH_H], button);
            if fade < 1.0 {
                c.rect(bx - 1.0, by + bh + 4.0, bw + 2.0, GRAPH_H + 40.0, rgba(0x15151d, 1.0 - fade));
            }
        } else {
            self.graph_rect = None;
            self.play_rect = None;
        }
        let game = self.game.as_mut().filter(|_| loaded)?;
        let full_w = c.width;
        let start = c.shapes.len();
        c.width = bw / k;
        game.kiai_effects = self.kiai_effects;
        game.draw_playfield(c, scroll_speed);
        let oy = by + (bh - band_h * k) / 2.0 - band_top * k;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        if button.is_some() {
            // Song select only (download previews have no fullscreen view).
            c.text_right(bx + bw - 6.0, by + bh - 10.0, 6.0, rgba(0x6f6f86, 1.0), "click: fullscreen (Ctrl+Enter)");
        }
        if fade < 1.0 {
            c.round_rect(bx, by, bw, bh, 6.0, rgba(0x0c0c12, 1.0 - fade), rgba(0, 0.0), 0.0);
        }
        let clip = crate::layout::Area { x: bx / full_w, y: by / VIRTUAL_HEIGHT, w: bw / full_w, h: bh / VIRTUAL_HEIGHT };
        Some((start..c.shapes.len(), clip))
    }
}

/// How big the autoplay preview gets: smallest and full (dragging the panel's bottom edge).
pub const PREVIEW_SIZE: (f32, f32) = (0.4, 1.0);

/// Space between the stats in the details.
const STAT_GAP: f32 = 10.0;

/// `s` as up to `lines` lines of text `size` tall within `max_w`, broken at spaces (or mid-word
/// when a word alone is too long); what still doesn't fit ends the last line with "...".
fn wrap(size: f32, max_w: f32, s: &str, lines: usize) -> Vec<String> {
    let per_line = ((max_w / Canvas::char_width(size)) as usize).max(4);
    let mut out: Vec<String> = Vec::new();
    let mut rest: Vec<char> = s.chars().collect();
    while !rest.is_empty() && out.len() < lines {
        if rest.len() <= per_line {
            out.push(rest.drain(..).collect());
            break;
        }
        // Break after the last space that keeps the line within the width, else at the width.
        let at = rest[..=per_line].iter().rposition(|&ch| ch == ' ').filter(|&p| p > 0).unwrap_or(per_line);
        out.push(rest[..at].iter().collect::<String>().trim_end().to_string());
        rest.drain(..at);
        while rest.first() == Some(&' ') {
            rest.remove(0);
        }
    }
    if !rest.is_empty() {
        if let Some(last) = out.last_mut() {
            *last = Canvas::fit(size, max_w, &format!("{last} {}", rest.iter().collect::<String>()));
        }
    }
    out
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
