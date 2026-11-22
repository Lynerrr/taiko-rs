//! Song select's card pictures: the middle strip of each set's picture, as wide for its height
//! as a card (`render::CARD_CELL`), in a texture of their own with room for hundreds of them
//! (`Renderer::set_card`).
//!
//! A strip is small, so the list keeps all of the ones on screen and those near it and ahead of it
//! (the way it's going) in: dragging or wheeling through the list draws them from memory. Each is
//! kept as a PNG in the cache, made ahead for the whole library (`prewarm`) and named with its
//! average colour, which a card shows until its strip is in (the strip fades in over it).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::render::{CARD_ASPECT, CARD_CELL, CARD_SLOTS, path_picture_key};
use crate::services::workers::{self, Ticket};

/// Larger files are left alone, as `pictures` does.
const MAX_BYTES: u64 = 24 * 1024 * 1024;
/// Strips read at once: one takes a ms or two from its copy.
const MAX_READS: usize = 4;
/// Strips that go into their slots (and up to the GPU) a frame: 256 KB each.
const PLACE_PER_FRAME: usize = 8;
/// How long a strip that couldn't be read waits before it is tried again.
const RETRY: Duration = Duration::from_secs(2);
/// The copies' kind, in their folder's name: a new one leaves the old folder to be removed.
const CARD_VERSION: u32 = 1;
/// How often the colours found are handed on to the canvas (a copy of the whole map each time).
const COLOURS_EVERY: Duration = Duration::from_millis(250);

/// A strip read: `CARD_CELL` premultiplied RGBA (none: called off before its turn, see
/// `workers::Ticket`).
type Read = (String, Option<Result<Vec<u8>, String>>);

/// The average colours found, by picture key, and whether any came since they were handed on.
#[derive(Default)]
struct Found {
    colours: Mutex<HashMap<String, [f32; 3]>>,
    new: AtomicBool,
}

impl Found {
    fn put(&self, key: String, colour: [f32; 3]) {
        if let Ok(mut c) = self.colours.lock() {
            c.insert(key, colour);
        }
        self.new.store(true, Ordering::Relaxed);
    }

    fn get(&self, key: &str) -> Option<[f32; 3]> {
        self.colours.lock().ok()?.get(key).copied()
    }
}

pub struct CardPictures {
    slots: Arc<HashMap<String, u32>>,
    /// What each slot holds, and the frame it was last wanted in (the longest unwanted goes first).
    held: Vec<Option<String>>,
    used: Vec<u64>,
    frame: u64,
    /// Being read, each with the ticket that calls it off.
    pending: HashMap<String, Ticket>,
    /// Unreadable, and when they're tried again.
    failed: HashMap<String, Instant>,
    /// Read and waiting for a slot.
    arrived: Vec<(String, Vec<u8>)>,
    uploads: Vec<(u32, Vec<u8>)>,
    found: Arc<Found>,
    colours: Arc<HashMap<String, [f32; 3]>>,
    colours_at: Option<Instant>,
    /// Where the copies are kept; none in tests (the strips are cut from the pictures each time).
    dir: Option<PathBuf>,
    /// Calls off the copies being made ahead.
    prewarm: Option<Arc<AtomicBool>>,
    /// Holds them while a song is played (`hold`).
    paused: Arc<AtomicBool>,
    tx: Sender<Read>,
    rx: Receiver<Read>,
}

impl Default for CardPictures {
    fn default() -> Self {
        let (tx, rx) = channel();
        CardPictures {
            slots: Arc::default(),
            held: vec![None; CARD_SLOTS as usize],
            used: vec![0; CARD_SLOTS as usize],
            frame: 0,
            pending: HashMap::new(),
            failed: HashMap::new(),
            arrived: Vec::new(),
            uploads: Vec::new(),
            found: Arc::default(),
            colours: Arc::default(),
            colours_at: None,
            dir: if cfg!(test) {
                None
            } else {
                crate::config::cache_dir()
                    .map(|d| d.join("thumbs").join(format!("cards-v{CARD_VERSION}")))
            },
            prewarm: None,
            paused: Arc::default(),
            tx,
            rx,
        }
    }
}

impl CardPictures {
    /// The strips in the texture, by picture key: their slots.
    pub fn slots(&self) -> Arc<HashMap<String, u32>> {
        self.slots.clone()
    }

    /// The strips' average colours known so far, by picture key.
    pub fn colours(&self) -> Arc<HashMap<String, [f32; 3]>> {
        self.colours.clone()
    }

    pub fn take_uploads(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// Makes the copies of the strips of the pictures at `paths` that aren't made yet, on a thread
    /// of its own, and learns the colours of those that are (from their names) first of all. A new
    /// call stops the last one's work.
    pub fn prewarm(&mut self, paths: Vec<PathBuf>) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        if let Some(stop) = self.prewarm.take() {
            stop.store(true, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.prewarm = Some(stop.clone());
        let found = self.found.clone();
        let paused = self.paused.clone();
        let spawned = std::thread::Builder::new()
            .name("card copies".into())
            .spawn(move || {
                crate::pictures::lower_priority();
                // Copies of an older kind aren't read any more.
                if let Some(parent) = dir.parent() {
                    for old in std::fs::read_dir(parent).into_iter().flatten().flatten() {
                        let name = old.file_name().to_string_lossy().into_owned();
                        if name.starts_with("cards-v") && old.path() != dir {
                            let _ = std::fs::remove_dir_all(old.path());
                        }
                    }
                }
                let made: HashMap<String, [f32; 3]> = std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|f| parse_name(&f.file_name().to_string_lossy()))
                    .collect();
                let mut missing = Vec::new();
                for path in paths {
                    let Some(stem) = copy_stem(&path) else {
                        continue;
                    };
                    match made.get(&stem) {
                        Some(&colour) => found.put(path_picture_key(&path), colour),
                        None => missing.push(path),
                    }
                }
                for path in missing {
                    if !crate::pictures::wait_turn(&stop, &paused) {
                        return;
                    }
                    let _ = read(&path, Some(&dir), &found);
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
        if let Err(e) = spawned {
            eprintln!("card copies: {e}");
        }
    }

    /// Holds the copies being made ahead while a song is played (`on`), or lets them go on.
    pub fn hold(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
    }

    /// Tests: the copies made ahead are held.
    #[cfg(test)]
    pub fn held(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Reads and keeps in the strips `wanted` (picture key, file, whether a card draws it now):
    /// the drawn ones first, then the others in their order, as many as there are slots.
    pub fn update(&mut self, wanted: &[(String, PathBuf, bool)]) {
        self.frame += 1;
        while let Ok((key, got)) = self.rx.try_recv() {
            self.pending.retain(|k| *k != key);
            match got {
                Ok(px) => {
                    self.arrived.retain(|(k, _)| *k != key);
                    self.arrived.push((key, px));
                }
                Err(e) => {
                    eprintln!("card picture {key}: {e}");
                    self.failed.insert(key, Instant::now() + RETRY);
                }
            }
        }
        let now = Instant::now();
        self.failed.retain(|_, at| *at > now);
        let order: Vec<&(String, PathBuf, bool)> = wanted
            .iter()
            .filter(|w| w.2)
            .chain(wanted.iter().filter(|w| !w.2))
            .take(CARD_SLOTS as usize)
            .collect();
        let keep: HashSet<&str> = order.iter().map(|w| w.0.as_str()).collect();
        // The strips of cards scrolled away while they waited their turn are called off: the
        // reading threads skip them instead of decoding pictures long gone by.
        self.pending.retain(|key, ticket| {
            let wanted = keep.contains(key.as_str());
            if !wanted {
                ticket.cancel();
            }
            wanted
        });
        for w in &order {
            if let Some(&slot) = self.slots.get(&w.0) {
                self.used[slot as usize] = self.frame;
            }
        }
        for _ in 0..PLACE_PER_FRAME {
            let next = order
                .iter()
                .find_map(|w| self.arrived.iter().position(|(k, _)| *k == w.0))
                .or((!self.arrived.is_empty()).then_some(0));
            let Some(i) = next else {
                break;
            };
            let (key, px) = self.arrived.remove(i);
            self.place(key, px, &keep);
        }
        let over = self.arrived.len().saturating_sub(CARD_SLOTS as usize);
        self.arrived.drain(..over);
        for &(key, path, _) in &order {
            if self.pending.len() >= MAX_READS {
                break;
            }
            if self.slots.contains_key(key)
                || self.pending.contains_key(key)
                || self.failed.contains_key(key)
                || self.arrived.iter().any(|(k, _)| k == key)
            {
                continue;
            }
            let ticket = Ticket::default();
            self.pending.insert(key.clone(), ticket.clone());
            let (tx, key, path) = (self.tx.clone(), key.clone(), path.clone());
            let (dir, found) = (self.dir.clone(), self.found.clone());
            workers::run(move || {
                let got = ticket.wanted().then(|| read(&path, dir.as_deref(), &found));
                let _ = tx.send((key, got));
            });
        }
        let due = self.colours_at.is_none_or(|t| now >= t + COLOURS_EVERY);
        if due && self.found.new.swap(false, Ordering::Relaxed) {
            if let Ok(c) = self.found.colours.lock() {
                self.colours = Arc::new(c.clone());
            }
            self.colours_at = Some(now);
        }
    }

    /// Puts a strip in a free slot, or in the one wanted longest ago that isn't wanted now.
    fn place(&mut self, key: String, px: Vec<u8>, keep: &HashSet<&str>) {
        let free = self.held.iter().position(Option::is_none).or_else(|| {
            (0..self.held.len())
                .filter(|&s| self.held[s].as_deref().is_some_and(|h| !keep.contains(h)))
                .min_by_key(|&s| self.used[s])
        });
        let Some(slot) = free else {
            // Every slot holds a strip drawn now; this one isn't wanted.
            return;
        };
        let mut slots = (*self.slots).clone();
        if let Some(old) = self.held[slot].take() {
            slots.remove(&old);
        }
        slots.insert(key.clone(), slot as u32);
        self.slots = Arc::new(slots);
        self.held[slot] = Some(key);
        self.used[slot] = self.frame;
        self.uploads.retain(|(s, _)| *s != slot as u32);
        self.uploads.push((slot as u32, px));
    }

    /// Tests: the strip for `key` is in and its colour known, as if read.
    #[cfg(test)]
    pub fn pretend_loaded(&mut self, key: &str, colour: [f32; 3]) {
        self.pretend_colour(key, colour);
        let slot = self
            .held
            .iter()
            .position(Option::is_none)
            .expect("a free slot");
        self.place(key.to_string(), Vec::new(), &HashSet::new());
        self.uploads.retain(|(s, _)| *s != slot as u32);
    }

    /// Tests: the colour of the strip for `key` is known (from its copy's name), the strip not
    /// read yet.
    #[cfg(test)]
    pub fn pretend_colour(&mut self, key: &str, colour: [f32; 3]) {
        let mut colours = (*self.colours).clone();
        colours.insert(key.to_string(), colour);
        self.colours = Arc::new(colours);
    }
}

/// The name of the copy of the strip of the picture at `path` (by the file, its size and when it
/// was changed, and the strip's size), before its colour.
fn copy_stem(path: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).ok()?;
    let changed = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), changed, CARD_CELL).hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

/// A copy's name: its stem, then its average colour as six hex digits.
fn copy_name(stem: &str, colour: [f32; 3]) -> String {
    let [r, g, b] = colour.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u32);
    format!("{stem}-{:06x}.png", (r << 16) | (g << 8) | b)
}

/// The stem and colour of a copy by its name.
fn parse_name(name: &str) -> Option<(String, [f32; 3])> {
    let (stem, hex) = name.strip_suffix(".png")?.split_once('-')?;
    if stem.len() != 16 || hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    let colour = [v >> 16, v >> 8, v].map(|c| (c & 0xff) as f32 / 255.0);
    Some((stem.to_string(), colour))
}

/// The strip of the picture at `path`, from its copy in `dir` when there is one (made from the
/// picture, and kept there, when not); its colour goes to `found`.
fn read(path: &Path, dir: Option<&Path>, found: &Found) -> Result<Vec<u8>, String> {
    let key = path_picture_key(path);
    let stem = dir.and_then(|_| copy_stem(path));
    if let (Some(dir), Some(stem)) = (dir, &stem)
        && let Some(colour) = found.get(&key)
        && let Ok(bytes) = std::fs::read(dir.join(copy_name(stem, colour)))
        && let Ok(img) = image::load_from_memory(&bytes)
    {
        return Ok(cell(img.to_rgba8()));
    }
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("{} MB is too big", size / 1024 / 1024));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let strip = strip(&image::load_from_memory(&bytes).map_err(|e| e.to_string())?);
    let colour = average(&strip);
    if let (Some(dir), Some(stem)) = (dir, &stem) {
        let _ = std::fs::create_dir_all(dir);
        // Written beside and moved in, so a half-written copy is never read. Opaque ones without
        // their alpha: a third smaller.
        let opaque = strip.pixels().all(|p| p[3] == 255);
        let copy = if opaque {
            image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(strip.clone()).to_rgb8())
        } else {
            image::DynamicImage::ImageRgba8(strip.clone())
        };
        let name = copy_name(stem, colour);
        let part = dir.join(format!("{name}.part"));
        if copy
            .save_with_format(&part, image::ImageFormat::Png)
            .is_ok()
        {
            let _ = std::fs::rename(&part, dir.join(name));
        }
    }
    found.put(key, colour);
    Ok(cell(strip))
}

/// The middle of `img` as wide for its height as a card's strip, at the strip's size.
fn strip(img: &image::DynamicImage) -> image::RgbaImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let (cw, ch) = if w as f32 / h as f32 > CARD_ASPECT {
        (((h as f32 * CARD_ASPECT).round() as u32).clamp(1, w), h)
    } else {
        (w, ((w as f32 / CARD_ASPECT).round() as u32).clamp(1, h))
    };
    img.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch)
        .resize_exact(
            CARD_CELL[0],
            CARD_CELL[1],
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8()
}

/// A strip as its slot takes it: `CARD_CELL`, premultiplied.
fn cell(img: image::RgbaImage) -> Vec<u8> {
    let img = if img.dimensions() == (CARD_CELL[0], CARD_CELL[1]) {
        img
    } else {
        image::imageops::resize(
            &img,
            CARD_CELL[0],
            CARD_CELL[1],
            image::imageops::FilterType::Triangle,
        )
    };
    let mut px = img.into_raw();
    crate::pictures::premultiply(&mut px);
    px
}

/// The average colour of a strip (straight, by how opaque each pixel is; mid grey for none).
fn average(img: &image::RgbaImage) -> [f32; 3] {
    let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
    for p in img.pixels() {
        let a = f64::from(p[3]) / 255.0;
        for (s, v) in sum.iter_mut().zip(&p.0[..3]) {
            *s += f64::from(*v) / 255.0 * a;
        }
        weight += a;
    }
    if weight <= 0.0 {
        return [0.5; 3];
    }
    sum.map(|s| (s / weight) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| image::Rgb(f(x, y))))
    }

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn all_green(s: &image::RgbaImage) -> bool {
        s.pixels().all(|p| p[1] > 200 && p[0] < 40 && p[2] < 40)
    }

    #[test]
    fn a_strip_is_the_middle_of_a_picture_at_a_cards_shape() {
        // Wider for its height than a strip: its sides go.
        let wide = solid(2000, 100, |x, _| match x {
            0..200 => RED,
            1800.. => BLUE,
            _ => GREEN,
        });
        let s = strip(&wide);
        assert_eq!(s.dimensions(), (CARD_CELL[0], CARD_CELL[1]));
        assert!(all_green(&s), "only the middle of a wide picture");
        // A usual picture: a band across its middle.
        let tall = solid(1600, 900, |_, y| match y {
            0..400 => RED,
            500.. => BLUE,
            _ => GREEN,
        });
        let s = strip(&tall);
        assert_eq!(s.dimensions(), (CARD_CELL[0], CARD_CELL[1]));
        assert!(all_green(&s), "the band across its middle");
        let c = average(&s);
        assert!(c[1] > 0.9 && c[0] < 0.1 && c[2] < 0.1, "{c:?}");
    }

    #[test]
    fn a_copys_name_keeps_its_colour() {
        let name = copy_name("0123456789abcdef", [1.0, 0.5, 0.0]);
        assert_eq!(name, "0123456789abcdef-ff8000.png");
        let (stem, colour) = parse_name(&name).expect("a copy's name");
        assert_eq!(stem, "0123456789abcdef");
        assert!((colour[1] - 128.0 / 255.0).abs() < 1e-6 && colour[0] == 1.0);
        assert_eq!(parse_name("0123456789abcdef-ff8000.png.part"), None);
        assert_eq!(parse_name("notes.txt"), None);
    }

    #[test]
    fn strips_drawn_now_are_never_let_go_for_others() {
        let mut p = CardPictures::default();
        let key = |i: usize| format!("@path/{i:016x}");
        for i in 0..CARD_SLOTS as usize {
            p.frame = i as u64;
            p.place(key(i), Vec::new(), &HashSet::new());
        }
        assert_eq!(p.slots.len(), CARD_SLOTS as usize, "every slot taken");
        // All but the first two drawn: a new one takes the slot wanted longest ago of those two.
        let drawn: Vec<String> = (2..CARD_SLOTS as usize).map(key).collect();
        let keep: HashSet<&str> = drawn.iter().map(String::as_str).collect();
        p.frame += 1;
        p.place("@path/new".into(), Vec::new(), &keep);
        assert!(p.slots.contains_key("@path/new"));
        assert!(!p.slots.contains_key(&key(0)), "the oldest unwanted went");
        assert!(p.slots.contains_key(&key(1)));
        assert!(drawn.iter().all(|k| p.slots.contains_key(k)));
    }

    #[test]
    fn strips_are_read_from_the_pictures_the_drawn_ones_first() {
        let dir = std::env::temp_dir().join(format!("taiko-card-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut wanted = Vec::new();
        for i in 0..MAX_READS + 3 {
            let path = dir.join(format!("bg{i}.png"));
            solid(320, 180, |_, _| GREEN).save(&path).unwrap();
            // The last one asked is the one drawn.
            let drawn = i == MAX_READS + 2;
            wanted.push((path_picture_key(&path), path, drawn));
        }
        let mut p = CardPictures::default();
        p.update(&wanted);
        assert_eq!(p.pending.len(), MAX_READS, "only a few read at once");
        assert!(
            p.pending.contains_key(&wanted[MAX_READS + 2].0),
            "the drawn one among them"
        );
        let until = Instant::now() + Duration::from_secs(20);
        let mut uploads = Vec::new();
        while p.slots.len() < wanted.len() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
            p.colours_at = None;
            p.update(&wanted);
            uploads.extend(p.take_uploads());
        }
        assert_eq!(p.slots.len(), wanted.len(), "all read and in");
        assert!(
            uploads
                .iter()
                .all(|(_, px)| px.len() == (CARD_CELL[0] * CARD_CELL[1] * 4) as usize)
        );
        let colour = p.colours()[&wanted[0].0];
        assert!(colour[1] > 0.9 && colour[0] < 0.1, "its colour: {colour:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
