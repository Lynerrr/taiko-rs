//! Song select preview: the selected map's background, its music from the preview point, and a
//! live autoplay of the selected difficulty with its details, all loaded in the background.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audio::{self, Engine, Track};
use crate::beatmap::{self, Beatmap, Kind};
use crate::game::{self, Game};
use crate::library::Entry;
use crate::render::{rgba, Canvas, VIRTUAL_HEIGHT};

/// Wait this long after the selection settles before loading, so scrolling stays cheap.
const DEBOUNCE: Duration = Duration::from_millis(180);

pub type Image = (u32, u32, Vec<u8>);

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
    requested: Option<usize>,
    map_rx: Option<Receiver<(usize, Result<Beatmap, String>)>>,
    bg_rx: Option<Receiver<(PathBuf, Option<Image>)>>,
    audio_rx: Option<Receiver<(PathBuf, Option<Arc<Track>>)>>,
    map: Option<(usize, Beatmap, Details)>,
    game: Option<Game>,
    last_time: f64,
    bg_path: Option<PathBuf>,
    /// New background for the renderer: `Some(None)` clears it.
    pub bg_upload: Option<Option<Image>>,
    pub has_background: bool,
    /// Decoded song for the selected set and where its preview starts (ms).
    audio: Option<(PathBuf, Arc<Track>, f64)>,
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

    /// The decoded song, for reuse when that map is played at normal speed.
    pub fn cached_track(&self, e: &Entry) -> Option<Arc<Track>> {
        self.audio.as_ref().filter(|(p, _, _)| *p == audio_path(e)).map(|(_, t, _)| t.clone())
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
    pub fn update(&mut self, now: Instant, library: &[Entry], selected: Option<usize>, engine: Option<&Engine>, in_menu: bool, in_select: bool, offset: f64) {
        if selected != self.seen {
            self.seen = selected;
            self.changed_at = Some(now);
        }
        // Start loads once the selection has settled.
        if self.seen != self.requested && self.changed_at.is_some_and(|t| now - t >= DEBOUNCE) {
            self.requested = self.seen;
            self.game = None;
            if let Some(i) = self.seen.filter(|&i| i < library.len()) {
                let e = &library[i];
                let path = e.path.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send((i, beatmap::load(&path)));
                });
                self.map_rx = Some(rx);
                self.show_background_for(e);
                let song = audio_path(e);
                let loaded = self.audio.as_ref().is_some_and(|(p, _, _)| *p == song);
                if !loaded {
                    if let Some(rate) = engine.map(|a| a.rate) {
                        let (tx, rx) = mpsc::channel();
                        std::thread::spawn(move || {
                            let track = audio::decode(&song, rate).ok().map(Arc::new);
                            let _ = tx.send((song, track));
                        });
                        self.audio_rx = Some(rx);
                    }
                }
            }
        }

        if let Some(rx) = &self.map_rx {
            if let Ok((i, result)) = rx.try_recv() {
                self.map_rx = None;
                if Some(i) == self.requested {
                    if let Ok(map) = result {
                        let details = Details::of(&map);
                        self.game = Some(new_preview_game(&map));
                        self.last_time = f64::NEG_INFINITY;
                        self.map = Some((i, map, details));
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
        if let Some(rx) = &self.audio_rx {
            if let Ok((path, track)) = rx.try_recv() {
                self.audio_rx = None;
                let wanted = self.requested.and_then(|i| library.get(i)).map(audio_path);
                if let (Some(track), Some(engine), true) = (track, engine, wanted.as_ref() == Some(&path)) {
                    let len_ms = track.frames() as f64 / engine.rate as f64 * 1000.0;
                    let preview_time = self.requested.and_then(|i| library.get(i)).map(|e| e.meta.preview_time).unwrap_or(-1.0);
                    let start = if preview_time >= 0.0 && preview_time < len_ms { preview_time } else { len_ms * 0.4 };
                    self.audio = Some((path, track, start));
                    self.audio_active = false;
                }
            }
        }

        // Music: start at the preview point, loop back to it at the end.
        let (Some(engine), true) = (engine, in_menu) else { return };
        if let Some((_, track, start)) = &self.audio {
            let len_ms = track.frames() as f64 / engine.rate as f64 * 1000.0;
            let restart = !self.audio_active || engine.position_ms(now).is_some_and(|p| p > len_ms - 30.0);
            if restart {
                engine.play(track.clone(), *start);
                self.audio_active = true;
            }
        }

        // Autoplay preview, driven by the music's clock.
        if !in_select || !self.audio_active {
            return;
        }
        let Some(game) = &mut self.game else { return };
        game.update(now, Some(engine), offset);
        if game.time < self.last_time - 1000.0 {
            // The music looped: start the preview over from the preview point.
            if let Some((_, map, _)) = &self.map {
                let mut fresh = new_preview_game(map);
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

    /// Details and the autoplay preview in the right-hand panel (virtual units). Returns the
    /// shape range to clip and the clip area.
    pub fn draw_panel(&mut self, c: &mut Canvas, library: &[Entry], x: f32, y: f32, w: f32, h: f32, scroll_speed: f64) -> Option<(std::ops::Range<usize>, crate::layout::Area)> {
        // The preview shows ~640 units of lane (several notes ahead), so its height follows the
        // panel width; the panel shrinks to fit instead of leaving empty space.
        let (band_top, band_h) = game::lane_band();
        let k = (w - 20.0) / 640.0;
        let preview_h = band_h * k + 12.0;
        let h = h.min(114.0 + preview_h + 10.0);
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
        c.text(x + 14.0, y + 64.0, 9.0, rgba(0xEB452C, 1.0), &fit(&format!("[{}]", e.meta.version), 9.0));
        let loaded = self.map.as_ref().filter(|(mi, _, _)| *mi == i);
        let line = match loaded {
            Some((_, _, d)) => {
                let bpm = if (d.bpm.1 - d.bpm.0).abs() < 0.5 { format!("{:.0} BPM", d.bpm.0) } else { format!("{:.0}-{:.0} BPM", d.bpm.0, d.bpm.1) };
                let secs = (d.length_ms / 1000.0).max(0.0) as u64;
                format!("OD {:.1}   {bpm}   {}:{:02}", e.meta.od, secs / 60, secs % 60)
            }
            None => format!("OD {:.1}", e.meta.od),
        };
        c.text(x + 14.0, y + 80.0, 8.0, rgba(0xc8c8d8, 1.0), &line);
        if let Some((_, _, d)) = loaded {
            let notes = format!("{} notes ({} don / {} kat)   {} rolls   {} swells", d.dons + d.kats, d.dons, d.kats, d.rolls, d.swells);
            c.text(x + 14.0, y + 94.0, 8.0, rgba(0x8a8aa0, 1.0), &fit(&notes, 8.0));
        }

        // Autoplay preview: just the lane band of the playfield, fitted to the box.
        let (bx, by, bw, bh) = (x + 10.0, y + 114.0, w - 20.0, preview_h);
        c.round_rect(bx, by, bw, bh, 6.0, rgba(0x0c0c12, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        let game = self.game.as_mut().filter(|_| loaded.is_some())?;
        let full_w = c.width;
        let start = c.shapes.len();
        c.width = bw / k;
        game.draw_playfield(c, scroll_speed);
        let oy = by + (bh - band_h * k) / 2.0 - band_top * k;
        c.transform_since(start, [0.0, 0.0], k, bx, oy);
        c.width = full_w;
        let clip = crate::layout::Area { x: bx / full_w, y: by / VIRTUAL_HEIGHT, w: bw / full_w, h: bh / VIRTUAL_HEIGHT };
        c.text_right(bx + bw - 6.0, by + 4.0, 6.0, rgba(0x6f6f86, 1.0), "autoplay preview");
        Some((start..c.shapes.len(), clip))
    }
}

fn new_preview_game(map: &Beatmap) -> Game {
    let mut g = Game::new(map.clone(), Arc::new(Track { samples: Vec::new(), rate: 1.0 }), None);
    g.mute_hits = true;
    g
}
