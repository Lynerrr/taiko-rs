//! A `.tko` map's own pictures (the layers of its clips), read from the map's folder.
//!
//! The canvas lists the pictures drawn each frame (`Canvas::pictures_wanted`); this loads them
//! from the folder of the map being played or edited, off the main thread, squeezes each into a
//! `render::PICTURE_CELL` square and hands it to the renderer (`Renderer::set_picture`). The
//! picture keeps its own shape on screen: the canvas stretches the square back by its aspect.
//!
//! Animated GIF, WebP and PNG (APNG) pictures play: their frames are kept here and the one for
//! the time the canvas asks (`Canvas::picture_times`, ms since the layer began, looping) goes
//! into the slot whenever it changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::render::{PICTURE_CELL, PICTURE_SLOTS};
use crate::services::workers::{self, Ticket};

/// Larger files are left alone (a picture this big is a mistake, and a slow one).
const MAX_BYTES: u64 = 24 * 1024 * 1024;

/// Names starting with this are the download tab's result covers, read from
/// `download::covers_dir` whatever map folder is followed.
pub const COVER_PREFIX: &str = "@cover/";

/// Names starting with this are read from a path of their own (`Canvas::picture_cover`).
pub use crate::render::PATH_PICTURE_PREFIX as PATH_PREFIX;

/// A picture that doesn't come from the followed map folder.
fn external(name: &str) -> bool {
    name.starts_with(COVER_PREFIX) || name.starts_with(PATH_PREFIX)
}

/// At most this many pictures read by path at once (song select's cards and banner): flying
/// through the list or spamming Random would start a read for every set passing by, and all of
/// them decoding at once makes the game stutter. The ones still wanted next frame are asked again.
const MAX_PATH_READS: usize = 3;
/// How many read pictures go into their slots (and up to the GPU) a frame.
const PLACE_PER_FRAME: usize = 3;
/// How many of them may be pictures read by path (song select's banner, 2.7 MB each): one, or a
/// fast scroll sent several up in one frame beside the cards' strips and the background.
const PATH_PLACE_PER_FRAME: usize = 1;
/// What one picture takes to send up: its cell, RGBA.
const PICTURE_BYTES: usize = (PICTURE_CELL * PICTURE_CELL * 4) as usize;

/// How long a picture that couldn't be read waits before it is looked for again.
const RETRY: Duration = Duration::from_secs(2);

/// What one animation's frames may take together: past it they're kept smaller, at the next of
/// `SIDES`, and blown back up to the cell when shown.
const MAX_ANIMATION_BYTES: usize = 160 << 20;
const SIDES: [u32; 5] = [
    PICTURE_CELL,
    if PICTURE_CELL / 2 > 384 {
        PICTURE_CELL / 2
    } else {
        384
    },
    256,
    192,
    128,
];
/// The most frames kept of one animation; it loops over those.
const MAX_FRAMES: usize = 1200;
/// Frames saved to show for 10 ms or less show for this long, as browsers do (a GIF saved with
/// no delay means "as fast as you like", not "at once").
const SHORT_DELAY_MS: f64 = 100.0;

/// A picture read: its frames (one for a still), each a `side` square, premultiplied RGBA;
/// when each ends (ms from the first's start, ascending); its width over its height.
pub struct Image {
    frames: Vec<Vec<u8>>,
    ends: Vec<f64>,
    side: u32,
    aspect: f32,
}

impl Image {
    /// The frame at `t` ms into the animation (it loops).
    fn frame_at(&self, t: f64) -> usize {
        let total = self.ends.last().copied().unwrap_or(0.0);
        if self.frames.len() < 2 || total.is_nan() || total <= 0.0 || !t.is_finite() {
            return 0;
        }
        let t = t.rem_euclid(total);
        self.ends
            .partition_point(|&e| e <= t)
            .min(self.frames.len() - 1)
    }

    /// Frame `i` as the slot takes it: a `PICTURE_CELL` square.
    fn cell(&self, i: usize) -> Vec<u8> {
        let px = &self.frames[i];
        if self.side == PICTURE_CELL {
            return px.clone();
        }
        let small = image::RgbaImage::from_raw(self.side, self.side, px.clone())
            .expect("a frame is its side squared");
        image::imageops::resize(
            &small,
            PICTURE_CELL,
            PICTURE_CELL,
            image::imageops::FilterType::Triangle,
        )
        .into_raw()
    }
}

/// A picture read (none: called off before its turn, see `workers::Ticket`).
type Loaded = (PathBuf, String, Option<Result<Image, String>>);

pub struct Pictures {
    dir: Option<PathBuf>,
    slots: Arc<HashMap<String, (u32, f32)>>,
    /// What each slot holds, by slot.
    held: Vec<Option<String>>,
    /// Being read (each with the ticket that calls it off), or unreadable (and not asked again
    /// for this folder).
    pending: HashMap<String, Ticket>,
    failed: Vec<String>,
    /// When the failed ones are tried again (a file put in the folder later, or fixed).
    retry_at: Instant,
    uploads: Vec<(u32, Vec<u8>)>,
    /// Read and waiting for their slot: a few go up a frame (`update`).
    arrived: Vec<(String, Image)>,
    /// Where small copies of the pictures read by path are kept (`read_by_path`); none in tests.
    thumbs: Option<PathBuf>,
    /// Calls off the copies being made ahead (`prewarm`).
    prewarm: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Holds them while a song is played (`hold`).
    paused: Arc<std::sync::atomic::AtomicBool>,
    /// The animated pictures in slots, and the frame each slot shows.
    playing: HashMap<String, (Image, usize)>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Default for Pictures {
    fn default() -> Self {
        let (tx, rx) = channel();
        Pictures {
            dir: None,
            slots: Arc::default(),
            held: vec![None; PICTURE_SLOTS as usize],
            pending: HashMap::new(),
            failed: Vec::new(),
            retry_at: Instant::now(),
            uploads: Vec::new(),
            arrived: Vec::new(),
            thumbs: if cfg!(test) {
                None
            } else {
                crate::config::cache_dir().map(|d| d.join("thumbs"))
            },
            prewarm: None,
            paused: Arc::default(),
            playing: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl Pictures {
    pub fn slots(&self) -> Arc<HashMap<String, (u32, f32)>> {
        self.slots.clone()
    }

    /// Tests: `name` is in a slot, `aspect` wide per unit high, as if it had been read.
    #[cfg(test)]
    pub fn pretend_loaded(&mut self, name: &str, aspect: f32) {
        let mut slots = (*self.slots).clone();
        let slot = slots.len() as u32;
        slots.insert(name.to_string(), (slot, aspect));
        self.slots = Arc::new(slots);
    }

    /// Makes the small copies of the pictures at `paths` that aren't made yet, on a thread of
    /// its own, one at a time with a pause between: song select's cards then read in a few ms
    /// each, the first time they're seen too. A new call stops the last one's work.
    pub fn prewarm(&mut self, paths: Vec<PathBuf>) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let Some(thumbs) = self.thumbs.clone() else {
            return;
        };
        if let Some(stop) = self.prewarm.take() {
            stop.store(true, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.prewarm = Some(stop.clone());
        let paused = self.paused.clone();
        let spawned = std::thread::Builder::new()
            .name("picture copies".into())
            .spawn(move || {
                lower_priority();
                // The copies made as JPEGs (`THUMB_VERSION` 1) are no longer read.
                for old in std::fs::read_dir(&thumbs).into_iter().flatten().flatten() {
                    if !wait_turn(&stop, &paused) {
                        return;
                    }
                    if old.path().extension().is_some_and(|e| e == "jpg") {
                        let _ = std::fs::remove_file(old.path());
                    }
                }
                for path in paths {
                    if !wait_turn(&stop, &paused) {
                        return;
                    }
                    let made = thumb_path(&path, &thumbs).is_some_and(|(_, found)| found.is_some());
                    if !made {
                        let (Some(dir), Some(file)) = (path.parent(), path.file_name()) else {
                            continue;
                        };
                        let _ = read_by_path(dir, &file.to_string_lossy(), Some(&thumbs));
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("picture copies: {e}");
        }
    }

    /// Holds the copies being made ahead while a song is played (`on`), or lets them go on.
    pub fn hold(&self, on: bool) {
        self.paused.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Tests: the copies made ahead are held.
    #[cfg(test)]
    pub fn held(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn take_uploads(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// Follows the map folder `dir`, loads the pictures `wanted` (lowercase file names) and
    /// moves the animated ones to their frames at `times` (ms into them, by name).
    /// Tests: with a whole frame's budget each call.
    #[cfg(test)]
    pub fn update(
        &mut self,
        dir: Option<&Path>,
        wanted: &[String],
        times: &[(String, f64)],
        paths: &[(String, PathBuf)],
    ) {
        self.update_within(dir, wanted, times, paths, &mut crate::UPLOAD_BUDGET.clone());
    }

    /// `update`, the pictures read by path (song select's banner) going up to the GPU within
    /// `budget` (bytes this frame, what's left of it after them): one of them a frame at most,
    /// and none once something else took the budget. A map's own pictures and the covers go up
    /// as they did.
    pub fn update_within(
        &mut self,
        dir: Option<&Path>,
        wanted: &[String],
        times: &[(String, f64)],
        paths: &[(String, PathBuf)],
        budget: &mut usize,
    ) {
        if self.dir.as_deref() != dir {
            // The covers and the pictures read by path don't come from the folder: they stay.
            let cover = |n: &String| external(n);
            self.dir = dir.map(Path::to_path_buf);
            let slots = Arc::make_mut(&mut self.slots);
            slots.retain(|n, _| cover(n));
            for h in &mut self.held {
                if h.as_ref().is_some_and(|n| !cover(n)) {
                    *h = None;
                }
            }
            self.pending.retain(|n, _| cover(n));
            self.arrived.retain(|(n, _)| cover(n));
            self.failed.retain(cover);
            self.playing.retain(|n, _| cover(n));
        }
        if !self.failed.is_empty() && Instant::now() >= self.retry_at {
            self.failed.clear();
        }
        let covers = crate::download::covers_dir();
        while let Ok((from, name, got)) = self.rx.try_recv() {
            let ours = Some(&from) == self.dir.as_ref()
                || (name.starts_with(COVER_PREFIX) && from == covers)
                || name.starts_with(PATH_PREFIX);
            if !ours {
                continue;
            }
            // Called off: nothing came, and it's asked again if it's wanted again.
            let Some(got) = got else {
                continue;
            };
            self.pending.remove(&name);
            match got {
                Ok(image) => self.arrived.push((name, image)),
                Err(e) => {
                    eprintln!("map picture {name}: {e}");
                    self.failed.push(name);
                    self.retry_at = Instant::now() + RETRY;
                }
            }
        }
        // A few a frame go into their slots and up to the GPU (up to 3 MB each: many in one frame
        // is a hitch), the ones drawn now first. Read ones no longer drawn wait their turn; past a
        // texture's worth the oldest are let go.
        let mut by_path = 0;
        for _ in 0..PLACE_PER_FRAME {
            let next = self
                .arrived
                .iter()
                .position(|(n, _)| wanted.contains(n))
                .or((!self.arrived.is_empty()).then_some(0));
            let Some(i) = next else {
                break;
            };
            if self.arrived[i].0.starts_with(PATH_PREFIX) {
                // A banner is most of a frame's budget: one a frame, the first thing to go up.
                let spent = *budget < crate::UPLOAD_BUDGET;
                if by_path >= PATH_PLACE_PER_FRAME || (spent && *budget < PICTURE_BYTES) {
                    break;
                }
                by_path += 1;
                *budget = budget.saturating_sub(PICTURE_BYTES);
            }
            let (name, image) = self.arrived.remove(i);
            self.place(name, image, wanted);
        }
        let over = self.arrived.len().saturating_sub(PICTURE_SLOTS as usize);
        self.arrived.drain(..over);
        // A picture read by path no longer drawn (the banner of a set the selection has left) is
        // called off: the reading threads skip it instead of decoding it for nothing.
        self.pending.retain(|n, ticket| {
            let keep = !n.starts_with(PATH_PREFIX) || wanted.contains(n);
            if !keep {
                ticket.cancel();
            }
            keep
        });
        let mut path_reads = self
            .pending
            .keys()
            .filter(|n| n.starts_with(PATH_PREFIX))
            .count();
        for name in wanted {
            if self.slots.contains_key(name)
                || self.pending.contains_key(name)
                || self.failed.contains(name)
                || self.arrived.iter().any(|(n, _)| n == name)
            {
                continue;
            }
            // Read by path: the file the canvas named it for (no folder to stay inside).
            if name.starts_with(PATH_PREFIX) {
                let Some((dir, file)) = paths
                    .iter()
                    .find(|(k, _)| k == name)
                    .and_then(|(_, p)| Some((p.parent()?.to_path_buf(), p.file_name()?)))
                else {
                    continue;
                };
                if path_reads >= MAX_PATH_READS {
                    continue;
                }
                path_reads += 1;
                let ticket = Ticket::default();
                self.pending.insert(name.clone(), ticket.clone());
                let (tx, name) = (self.tx.clone(), name.clone());
                let file = file.to_string_lossy().into_owned();
                let thumbs = self.thumbs.clone();
                workers::run(move || {
                    let got = ticket
                        .wanted()
                        .then(|| read_by_path(&dir, &file, thumbs.as_deref()));
                    let _ = tx.send((dir, name, got));
                });
                continue;
            }
            let (dir, file) = match name.strip_prefix(COVER_PREFIX) {
                Some(file) => (covers.clone(), file),
                None => match &self.dir {
                    Some(dir) => (dir.clone(), name.as_str()),
                    None => continue,
                },
            };
            if !safe_name(file) {
                self.failed.push(name.clone());
                self.retry_at = Instant::now() + RETRY;
                continue;
            }
            // A map's own pictures and the covers are read whenever they were asked for.
            self.pending.insert(name.clone(), Ticket::default());
            let (tx, name, file) = (self.tx.clone(), name.clone(), file.to_string());
            workers::run(move || {
                let got = read(&dir, &file);
                let _ = tx.send((dir, name, Some(got)));
            });
        }
        for (name, t) in times {
            let Some(&(slot, _)) = self.slots.get(name) else {
                continue;
            };
            let Some((image, shown)) = self.playing.get_mut(name) else {
                continue;
            };
            let i = image.frame_at(*t);
            if i != *shown {
                *shown = i;
                // A frame not yet on the GPU is replaced, not queued behind.
                self.uploads.retain(|(s, _)| *s != slot);
                self.uploads.push((slot, image.cell(i)));
            }
        }
    }

    /// Puts a loaded picture in a free slot, or in one holding a picture not drawn now.
    fn place(&mut self, name: String, image: Image, wanted: &[String]) {
        let free = self.held.iter().position(Option::is_none).or_else(|| {
            self.held
                .iter()
                .position(|h| h.as_ref().is_some_and(|h| !wanted.contains(h)))
        });
        let Some(slot) = free else {
            // Every slot is drawn now: it waits for one to be let go.
            return;
        };
        let slots = Arc::make_mut(&mut self.slots);
        if let Some(old) = self.held[slot].take() {
            slots.remove(&old);
            self.playing.remove(&old);
        }
        slots.insert(name.clone(), (slot as u32, image.aspect));
        self.held[slot] = Some(name.clone());
        self.uploads.retain(|(s, _)| *s != slot as u32);
        self.uploads.push((slot as u32, image.cell(0)));
        if image.frames.len() > 1 {
            self.playing.insert(name, (image, 0));
        }
    }
}

pub use crate::tko::clips::safe_name;

/// The picture `name` in `dir` (matched ignoring case, as maps from Windows expect).
fn read(dir: &Path, name: &str) -> Result<Image, String> {
    let path = locate(dir, name);
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("{} MB is too big", size / 1024 / 1024));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    decode(&bytes)
}

/// Where the small copy of the picture at `path` goes in `thumbs` (by the file, its size and
/// when it was changed, and the cell's size), and the copy if it's there.
/// The small copies' kind, in their names: 1 were JPEGs (blocky, their colours in squares on a
/// card's dark shade), 2 are PNGs, exactly the picture as squeezed.
const THUMB_VERSION: u32 = 2;

fn thumb_path(path: &Path, thumbs: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).ok()?;
    let changed = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), changed, PICTURE_CELL, THUMB_VERSION).hash(&mut h);
    let stem = thumbs.join(format!("{:016x}", h.finish()));
    let found = Some(stem.with_extension("png")).filter(|p| p.is_file());
    Some((stem, found))
}

/// `read`, for a picture read by path (song select's backgrounds), through a small copy of it
/// in `thumbs`: the copy is the picture no wider or taller than a cell (its own shape kept, so
/// its aspect too), a PNG: lossless, as a JPEG's blocks show over the cards' shade. Reading that
/// takes a few ms where the
/// whole picture, decoded and squeezed, takes tens of them. Animations aren't copied.
fn read_by_path(dir: &Path, name: &str, thumbs: Option<&Path>) -> Result<Image, String> {
    let path = locate(dir, name);
    let Some((stem, found)) = thumbs.and_then(|t| thumb_path(&path, t)) else {
        return read(dir, name);
    };
    if let Some(small) = found
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| image::load_from_memory(&b).ok())
    {
        return Ok(still(small));
    }
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("{} MB is too big", size / 1024 / 1024));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    if image::guess_format(&bytes).is_ok_and(|f| f == image::ImageFormat::Gif) {
        return decode(&bytes);
    }
    let whole = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let small = whole.resize(
        PICTURE_CELL,
        PICTURE_CELL,
        image::imageops::FilterType::Triangle,
    );
    if let Some(parent) = stem.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Written beside and moved in, so a half-written copy is never read. Opaque ones without
    // their alpha: a third smaller.
    let opaque = !small.color().has_alpha() || small.to_rgba8().pixels().all(|p| p[3] == 255);
    let copy = if opaque {
        image::DynamicImage::ImageRgb8(small.to_rgb8())
    } else {
        small.clone()
    };
    let part = stem.with_extension("png.part");
    if copy
        .save_with_format(&part, image::ImageFormat::Png)
        .is_ok()
    {
        let _ = std::fs::rename(&part, stem.with_extension("png"));
    }
    Ok(still(small))
}

/// A still picture, squeezed into its cell.
fn still(img: image::DynamicImage) -> Image {
    let aspect = img.width().max(1) as f32 / img.height().max(1) as f32;
    Image {
        frames: vec![squeeze(img.to_rgba8(), PICTURE_CELL, true)],
        ends: vec![0.0],
        side: PICTURE_CELL,
        aspect,
    }
}

/// The file `name` in `dir`, matched ignoring case when it isn't there as written.
fn locate(dir: &Path, name: &str) -> PathBuf {
    let mut path = dir.join(name);
    if !path.is_file()
        && let Some(found) = std::fs::read_dir(path.parent().unwrap_or(dir))
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| Some(f.to_ascii_lowercase()) == file_of(name))
            })
    {
        path = found;
    }
    path
}

/// `bytes` (an image file), every frame of it if it is animated.
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    decode_within(bytes, MAX_ANIMATION_BYTES)
}

/// `decode`, an animation's frames kept within `budget` bytes.
fn decode_within(bytes: &[u8], budget: usize) -> Result<Image, String> {
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    use image::{AnimationDecoder, ImageFormat};
    use std::io::Cursor;
    let err = |e: image::ImageError| e.to_string();
    let frames = match image::guess_format(bytes).map_err(err)? {
        ImageFormat::Gif => Some(
            GifDecoder::new(Cursor::new(bytes))
                .map_err(err)?
                .into_frames(),
        ),
        ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(bytes)).map_err(err)?;
            d.has_animation().then(|| d.into_frames())
        }
        ImageFormat::Png => {
            let d = PngDecoder::new(Cursor::new(bytes)).map_err(err)?;
            if d.is_apng().map_err(err)? {
                Some(d.apng().map_err(err)?.into_frames())
            } else {
                None
            }
        }
        _ => None,
    };
    let Some(frames) = frames else {
        let (px, aspect) = cell(bytes)?;
        return Ok(Image {
            frames: vec![px],
            ends: vec![0.0],
            side: PICTURE_CELL,
            aspect,
        });
    };
    let mut image = Image {
        frames: Vec::new(),
        ends: Vec::new(),
        side: PICTURE_CELL,
        aspect: 1.0,
    };
    let mut end = 0.0;
    for frame in frames {
        let frame = frame.map_err(err)?;
        if image.frames.len() >= MAX_FRAMES {
            break;
        }
        let (n, d) = frame.delay().numer_denom_ms();
        let ms = f64::from(n) / f64::from(d.max(1));
        end += if ms <= 10.0 { SHORT_DELAY_MS } else { ms };
        let buf = frame.into_buffer();
        if image.frames.is_empty() {
            image.aspect = buf.width().max(1) as f32 / buf.height().max(1) as f32;
        }
        // Too many for this size: those kept so far go down to the next one.
        let count = image.frames.len() + 1;
        while count * (image.side as usize).pow(2) * 4 > budget {
            let Some(&next) = SIDES.iter().find(|&&s| s < image.side) else {
                break;
            };
            for f in &mut image.frames {
                *f = squeeze(
                    image::RgbaImage::from_raw(image.side, image.side, std::mem::take(f))
                        .expect("a frame is its side squared"),
                    next,
                    false,
                );
            }
            image.side = next;
        }
        image.frames.push(squeeze(buf, image.side, true));
        image.ends.push(end);
    }
    if image.frames.is_empty() {
        return Err("no frames".into());
    }
    Ok(image)
}

/// `img` squeezed into a `side` square (premultiplied first when `straight`, as image files
/// are).
fn squeeze(img: image::RgbaImage, side: u32, straight: bool) -> Vec<u8> {
    let mut px =
        image::imageops::resize(&img, side, side, image::imageops::FilterType::Triangle).into_raw();
    if straight {
        premultiply(&mut px);
    }
    px
}

pub(crate) fn premultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for v in &mut p[..3] {
            *v = ((u16::from(*v) * a + 127) / 255) as u8;
        }
    }
}

fn file_of(name: &str) -> Option<String> {
    Path::new(name)
        .file_name()
        .and_then(|f| f.to_str())
        .map(str::to_ascii_lowercase)
}

/// `bytes` (an image file) squeezed into a `PICTURE_CELL` square, premultiplied RGBA, and its
/// width over its height.
pub fn cell(bytes: &[u8]) -> Result<(Vec<u8>, f32), String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let aspect = image.width().max(1) as f32 / image.height().max(1) as f32;
    let mut px = image
        .resize_exact(
            PICTURE_CELL,
            PICTURE_CELL,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8()
        .into_raw();
    premultiply(&mut px);
    Ok((px, aspect))
}

/// Whether `name` is a kind of picture a map can show.
pub fn is_picture(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif"]
        .iter()
        .any(|x| l.ends_with(x))
}

/// Whether `name` is a video (which a map can't show: see the editor's hint on dropping one).
pub fn is_video(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".mp4", ".webm", ".mov", ".mkv", ".avi", ".m4v"]
        .iter()
        .any(|x| l.ends_with(x))
}

/// The pictures in `dir` a map can use, by name, sorted.
pub fn in_folder(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| is_picture(n))
        .collect();
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out
}

/// How often work held while a song is played looks again whether it may go on.
const HELD_POLL: Duration = Duration::from_millis(100);

/// For work made ahead (`prewarm`), before each piece: waits while it's `held` (a song being
/// played: decoding pictures takes a whole core, which a low-end CPU can't spare without the game
/// stuttering), and says whether to go on at all (false once `stop`ped).
pub(crate) fn wait_turn(
    stop: &std::sync::atomic::AtomicBool,
    held: &std::sync::atomic::AtomicBool,
) -> bool {
    use std::sync::atomic::Ordering;
    while held.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(HELD_POLL);
    }
    !stop.load(Ordering::Relaxed)
}

/// Puts the calling thread at the lowest priority (Linux: nice 19, Windows: the lowest thread
/// priority; this thread alone), so work made ahead only takes the CPU the game and the desktop
/// leave.
pub(crate) fn lower_priority() {
    workers::set_priority(workers::Priority::Lowest);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_keeps_its_shape_and_is_premultiplied() {
        let (px, aspect) = cell(&png(300, 100, [200, 100, 50, 128])).unwrap();
        assert_eq!(px.len(), (PICTURE_CELL * PICTURE_CELL * 4) as usize);
        assert_eq!(&px[..4], &[100, 50, 25, 128]);
        assert!((aspect - 3.0).abs() < 1e-6);
        assert!(cell(b"not an image").is_err());
    }

    const ANIMATED_WEBP: [u8; 188] = [
        82, 73, 70, 70, 180, 0, 0, 0, 87, 69, 66, 80, 86, 80, 56, 88, 10, 0, 0, 0, 18, 0, 0, 0, 7,
        0, 0, 3, 0, 0, 65, 78, 73, 77, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 65, 78, 77, 70, 40, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 7, 0, 0, 3, 0, 0, 100, 0, 0, 2, 86, 80, 56, 76, 15, 0, 0, 0, 47, 7, 192,
        0, 0, 7, 16, 253, 143, 254, 7, 34, 162, 255, 1, 0, 65, 78, 77, 70, 40, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 7, 0, 0, 3, 0, 0, 40, 0, 0, 0, 86, 80, 56, 76, 15, 0, 0, 0, 47, 7, 192, 0, 0, 7, 208,
        255, 136, 254, 7, 34, 162, 255, 1, 0, 65, 78, 77, 70, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0,
        0, 3, 0, 0, 60, 0, 0, 2, 86, 80, 56, 76, 15, 0, 0, 0, 47, 7, 192, 0, 16, 7, 16, 209, 255,
        2, 6, 34, 162, 255, 1, 0,
    ];
    const APNG: [u8; 286] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 8, 0, 0, 0, 4, 8, 6,
        0, 0, 0, 179, 205, 126, 240, 0, 0, 0, 8, 97, 99, 84, 76, 0, 0, 0, 3, 0, 0, 0, 0, 206, 237,
        186, 192, 0, 0, 0, 26, 102, 99, 84, 76, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 1, 0, 10, 0, 0, 183, 187, 19, 134, 0, 0, 0, 21, 73, 68, 65, 84, 120, 156, 99,
        252, 207, 192, 240, 159, 1, 15, 96, 194, 39, 73, 29, 5, 0, 4, 160, 2, 6, 243, 23, 219, 41,
        0, 0, 0, 26, 102, 99, 84, 76, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1, 0, 25, 0, 0, 50, 168, 228, 123, 0, 0, 0, 25, 102, 100, 65, 84, 0, 0, 0, 2, 120, 156,
        99, 100, 248, 207, 240, 159, 1, 15, 96, 194, 39, 73, 29, 5, 0, 3, 161, 2, 6, 155, 24, 34,
        101, 0, 0, 0, 26, 102, 99, 84, 76, 0, 0, 0, 3, 0, 0, 0, 8, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 3, 0, 50, 0, 0, 145, 230, 205, 243, 0, 0, 0, 25, 102, 100, 65, 84, 0, 0, 0, 4, 120,
        156, 99, 100, 96, 248, 223, 192, 128, 7, 48, 225, 147, 164, 142, 2, 0, 195, 19, 1, 135,
        107, 78, 108, 190, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    /// A GIF of `frames` (colours) `w`×`h`, each shown `delays` ms (in tens, as GIF keeps them).
    fn gif(w: u32, h: u32, frames: &[([u8; 4], u32)]) -> Vec<u8> {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame};
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for &(c, ms) in frames {
                let img = image::RgbaImage::from_pixel(w, h, image::Rgba(c));
                enc.encode_frame(Frame::from_parts(
                    img,
                    0,
                    0,
                    Delay::from_numer_denom_ms(ms, 1),
                ))
                .unwrap();
            }
        }
        out
    }

    #[test]
    fn an_animated_gif_keeps_its_frames_and_their_timing() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let blue = [0, 0, 255, 255];
        // A frame saved with no delay shows for 100 ms, as in a browser.
        let img = decode(&gif(30, 10, &[(red, 100), (green, 0), (blue, 50)])).unwrap();
        assert_eq!(img.frames.len(), 3);
        assert_eq!(img.ends, vec![100.0, 200.0, 250.0]);
        assert_eq!(img.side, PICTURE_CELL);
        assert!((img.aspect - 3.0).abs() < 1e-6);
        assert_eq!(&img.frames[1][..4], &[0, 255, 0, 255]);
        for (t, i) in [
            (0.0, 0),
            (99.0, 0),
            (100.0, 1),
            (199.0, 1),
            (220.0, 2),
            (250.0, 0),
            (260.0, 0),
            (360.0, 1),
            (-10.0, 2),
            (f64::NAN, 0),
        ] {
            assert_eq!(img.frame_at(t), i, "at {t} ms");
        }
        assert_eq!(
            img.cell(2).len(),
            (PICTURE_CELL * PICTURE_CELL * 4) as usize
        );
        // A GIF of one frame is a still.
        let still = decode(&gif(4, 4, &[(red, 100)])).unwrap();
        assert_eq!(still.frames.len(), 1);
        assert_eq!(still.frame_at(500.0), 0);
    }

    #[test]
    fn animated_webp_and_png_play_and_their_stills_stay_still() {
        for (what, bytes) in [("webp", &ANIMATED_WEBP[..]), ("apng", &APNG[..])] {
            let img = decode(bytes).unwrap();
            assert_eq!(img.frames.len(), 3, "{what}");
            assert_eq!(img.ends, vec![100.0, 140.0, 200.0], "{what}");
            assert!((img.aspect - 2.0).abs() < 1e-6, "{what}");
            assert_eq!(&img.frames[0][..4], &[255, 0, 0, 255], "{what}");
            // Half see-through blue, premultiplied.
            let p = &img.frames[2][..4];
            assert!(
                p[2].abs_diff(128) <= 1 && p[3].abs_diff(128) <= 1,
                "{what}: {p:?}"
            );
        }
        let still = decode(&png(6, 3, [1, 2, 3, 255])).unwrap();
        assert_eq!((still.frames.len(), still.side), (1, PICTURE_CELL));
        assert!(decode(b"not an image").is_err());
    }

    #[test]
    fn a_long_animation_is_kept_smaller_and_shown_full_size() {
        // Room for 4 full-size frames: 20 go down to fit.
        let budget = 4 * (PICTURE_CELL as usize).pow(2) * 4;
        let frames: Vec<([u8; 4], u32)> = (0..20u32)
            .map(|i| ([i as u8 * 10, 0, 0, 255], 400))
            .collect();
        let img = decode_within(&gif(8, 8, &frames), budget).unwrap();
        assert_eq!(img.frames.len(), 20);
        assert!(
            img.side < PICTURE_CELL,
            "20 frames at full size are too much"
        );
        let kept: usize = img.frames.iter().map(Vec::len).sum();
        assert!(kept <= budget, "{kept} > {budget}");
        assert!(
            img.frames
                .iter()
                .all(|f| f.len() == (img.side * img.side * 4) as usize)
        );
        let shown = img.cell(15);
        assert_eq!(shown.len(), (PICTURE_CELL * PICTURE_CELL * 4) as usize);
        assert_eq!(shown[0], 150, "the frame's colour, blown up");
        assert_eq!(img.ends.last().copied(), Some(8000.0));
        // Past the smallest size, what fits is kept and the rest dropped... never more than the
        // frame cap.
        assert!(MAX_FRAMES * (SIDES[SIDES.len() - 1] as usize).pow(2) * 4 <= MAX_ANIMATION_BYTES);
    }

    #[test]
    fn an_animated_picture_shows_the_frame_for_the_time_asked() {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-pictures-anim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        std::fs::write(dir.join("fx.gif"), gif(4, 4, &[(red, 100), (green, 100)])).unwrap();
        std::fs::write(dir.join("logo.png"), png(4, 4, [9, 9, 9, 255])).unwrap();
        let mut p = Pictures::default();
        let wanted = vec!["fx.gif".to_string(), "logo.png".to_string()];
        let at = |t: f64| vec![("fx.gif".to_string(), t), ("logo.png".to_string(), t)];
        for _ in 0..400 {
            p.update(Some(&dir), &wanted, &at(0.0), &[]);
            if p.slots().len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let slot = p.slots()["fx.gif"].0;
        let first = p.take_uploads();
        assert_eq!(first.len(), 2, "each picture once");
        let fx = first.iter().find(|u| u.0 == slot).unwrap();
        assert_eq!(&fx.1[..4], &red);
        // Same frame: nothing goes up again; the next frame: just the animation's.
        p.update(Some(&dir), &wanted, &at(50.0), &[]);
        assert!(p.take_uploads().is_empty());
        p.update(Some(&dir), &wanted, &at(150.0), &[]);
        let next = p.take_uploads();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].0, slot);
        assert_eq!(&next[0].1[..4], &green);
        // Two frames within one: only the last goes up.
        p.update(Some(&dir), &wanted, &at(210.0), &[]);
        p.update(Some(&dir), &wanted, &at(310.0), &[]);
        let ups = p.take_uploads();
        assert_eq!(ups.len(), 1);
        assert_eq!(&ups[0].1[..4], &green, "310 ms: the second frame again");
        // Another folder: the animation goes with it.
        p.update(None, &wanted, &at(0.0), &[]);
        assert!(p.playing.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_made_ahead_waits_while_held_and_ends_when_stopped() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (stop, held) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(true)),
        );
        let (s, h) = (stop.clone(), held.clone());
        let waiter = std::thread::spawn(move || wait_turn(&s, &h));
        std::thread::sleep(HELD_POLL * 3);
        assert!(!waiter.is_finished(), "held: it waits");
        held.store(false, Ordering::Relaxed);
        assert!(waiter.join().unwrap(), "let go: it goes on");
        // Stopped while held: it ends at once instead of waiting for the song to end.
        held.store(true, Ordering::Relaxed);
        let (s, h) = (stop.clone(), held.clone());
        let waiter = std::thread::spawn(move || wait_turn(&s, &h));
        stop.store(true, Ordering::Relaxed);
        assert!(!waiter.join().unwrap(), "stopped: no more work");
    }

    #[test]
    fn a_few_pictures_by_path_are_read_at_a_time_and_go_up_a_few_a_frame() {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-pictures-few-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<(String, PathBuf)> = (0..8)
            .map(|i| {
                let path = dir.join(format!("bg{i}.png"));
                std::fs::write(&path, png(8, 8, [i * 30, 0, 0, 255])).unwrap();
                (crate::render::path_picture_key(&path), path)
            })
            .collect();
        let wanted: Vec<String> = paths.iter().map(|(k, _)| k.clone()).collect();
        let mut p = Pictures::default();
        p.update(None, &wanted, &[], &paths);
        assert_eq!(p.pending.len(), MAX_PATH_READS, "only a few read at once");
        let mut most = 0;
        for _ in 0..400 {
            p.update(None, &wanted, &[], &paths);
            most = most.max(p.pending.len());
            assert!(
                p.take_uploads().len() <= PATH_PLACE_PER_FRAME,
                "one goes up a frame"
            );
            if p.slots().len() == paths.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(p.slots().len(), paths.len(), "all read in the end");
        assert!(most <= MAX_PATH_READS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_banner_goes_up_one_a_frame_and_not_after_the_budget_is_spent() {
        let still = || Image {
            frames: vec![Vec::new()],
            ends: vec![0.0],
            side: PICTURE_CELL,
            aspect: 1.0,
        };
        let mut p = Pictures::default();
        for i in 0..3 {
            p.arrived.push((format!("{PATH_PREFIX}{i:016x}"), still()));
        }
        let mut budget = crate::UPLOAD_BUDGET;
        p.update_within(None, &[], &[], &[], &mut budget);
        assert_eq!(p.slots().len(), 1, "one banner a frame");
        assert_eq!(budget, crate::UPLOAD_BUDGET - PICTURE_BYTES);
        // Something else took the frame's budget first: none this frame.
        let mut budget = PICTURE_BYTES - 1;
        p.update_within(None, &[], &[], &[], &mut budget);
        assert_eq!(p.slots().len(), 1, "it waits");
        let mut budget = crate::UPLOAD_BUDGET;
        p.update_within(None, &[], &[], &[], &mut budget);
        assert_eq!(p.slots().len(), 2, "the next frame it goes");
    }

    #[test]
    fn a_banner_no_longer_drawn_is_called_off_and_a_maps_own_pictures_are_not() {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-pictures-off-{}", std::process::id()));
        let path = dir.join("gone.png");
        let key = crate::render::path_picture_key(&path);
        let paths = vec![(key.clone(), path)];
        let mut p = Pictures::default();
        p.update(None, std::slice::from_ref(&key), &[], &paths);
        let ticket = p.pending.get(&key).cloned().expect("asked for");
        // The selection moved on: its banner isn't drawn any more.
        p.update(None, &[], &[], &paths);
        assert!(!p.pending.contains_key(&key), "no longer waited for");
        assert!(!ticket.wanted() || p.failed.contains(&key));
        // A map's own picture (no path of its own) is read whether drawn this frame or not.
        p.update(Some(&dir), &["layer.png".to_string()], &[], &[]);
        let own = p.pending.get("layer.png").cloned();
        p.update(Some(&dir), &[], &[], &[]);
        if let Some(own) = own {
            assert!(own.wanted(), "a map's own picture isn't called off");
        }
    }

    #[test]
    fn a_picture_read_by_path_is_kept_small_and_read_back_from_that() {
        let base =
            std::env::temp_dir().join(format!("taiko-rs-pictures-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, thumbs) = (base.join("set"), base.join("thumbs"));
        std::fs::create_dir_all(&dir).unwrap();
        let big = PICTURE_CELL * 2;
        std::fs::write(dir.join("BG.png"), png(big, big / 2, [200, 40, 40, 255])).unwrap();
        let first = read_by_path(&dir, "bg.png", Some(&thumbs)).unwrap();
        let (_, made) = thumb_path(&locate(&dir, "bg.png"), &thumbs).unwrap();
        let made = made.expect("a small copy is kept");
        assert_eq!(made.extension().unwrap(), "png", "lossless");
        let small = image::open(&made).unwrap();
        assert_eq!(small.width(), PICTURE_CELL, "a cell wide");
        assert!(
            small.height().abs_diff(PICTURE_CELL / 2) <= 1,
            "its shape kept"
        );
        // Read again: from the copy, the same shape and size.
        let again = read_by_path(&dir, "bg.png", Some(&thumbs)).unwrap();
        assert_eq!((again.aspect, again.side), (first.aspect, first.side));
        assert!((again.aspect - 2.0).abs() < 0.01, "{}", again.aspect);
        assert_eq!(
            again.frames[0].len(),
            (PICTURE_CELL * PICTURE_CELL * 4) as usize
        );
        // The picture changed: a copy of its own.
        std::fs::write(dir.join("BG.png"), png(big, big, [0, 0, 200, 255])).unwrap();
        let changed = read_by_path(&dir, "bg.png", Some(&thumbs)).unwrap();
        assert!((changed.aspect - 1.0).abs() < 0.01, "{}", changed.aspect);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_pictures_small_copy_keeps_its_colours_exactly() {
        let base =
            std::env::temp_dir().join(format!("taiko-rs-pictures-exact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, thumbs) = (base.join("set"), base.join("thumbs"));
        std::fs::create_dir_all(&dir).unwrap();
        // Dark red over its left half, dark blue over its right: a card's shade over a picture.
        let (w, h) = (PICTURE_CELL * 2, PICTURE_CELL);
        let img = image::RgbaImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                image::Rgba([40, 12, 16, 255])
            } else {
                image::Rgba([10, 14, 52, 255])
            }
        });
        img.save(dir.join("bg.png")).unwrap();
        read_by_path(&dir, "bg.png", Some(&thumbs)).unwrap();
        let (_, made) = thumb_path(&locate(&dir, "bg.png"), &thumbs).unwrap();
        let bytes = std::fs::read(made.expect("a copy")).unwrap();
        let copy = image::load_from_memory(&bytes).unwrap().to_rgb8();
        // Away from where the two meet (the squeeze blends just there), every pixel as it was:
        // a lossy copy would shift them, in blocks.
        let edge = copy.width() / 2;
        for (x, _, p) in copy.enumerate_pixels() {
            if x + 3 < edge {
                assert_eq!(p.0, [40, 12, 16], "at {x}");
            } else if x > edge + 3 {
                assert_eq!(p.0, [10, 14, 52], "at {x}");
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn names_may_not_leave_the_map_folder() {
        assert!(safe_name("logo.png"));
        assert!(safe_name("sb/logo.png"));
        assert!(!safe_name("../logo.png"));
        assert!(!safe_name("/etc/passwd"));
        assert!(!safe_name(""));
        assert!(!safe_name("a/../../b.png"));
    }

    #[test]
    fn pictures_load_from_the_folder_into_slots_and_go_with_it() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Logo.PNG"), png(40, 20, [255, 255, 255, 255])).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(in_folder(&dir), vec!["Logo.PNG".to_string()]);
        let mut p = Pictures::default();
        let wanted = vec!["logo.png".to_string(), "missing.png".to_string()];
        for _ in 0..400 {
            p.update(Some(&dir), &wanted, &[], &[]);
            if p.slots().contains_key("logo.png") && p.failed.len() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(p.slots().get("logo.png").map(|s| s.1), Some(2.0));
        assert_eq!(p.take_uploads().len(), 1);
        assert_eq!(p.failed, vec!["missing.png".to_string()]);
        // Put in the folder afterwards, it is found on a later look (not straight away).
        std::fs::write(dir.join("missing.png"), png(10, 10, [0, 0, 0, 255])).unwrap();
        p.update(Some(&dir), &wanted, &[], &[]);
        assert!(!p.slots().contains_key("missing.png"));
        p.retry_at = Instant::now();
        for _ in 0..400 {
            p.update(Some(&dir), &wanted, &[], &[]);
            if p.slots().contains_key("missing.png") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(p.slots().contains_key("missing.png"), "{:?}", p.failed);
        // Another folder: nothing of this one is kept.
        p.update(None, &wanted, &[], &[]);
        assert!(p.slots().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn covers_load_from_their_own_folder_and_stay_when_the_map_folder_changes() {
        let covers = crate::download::covers_dir();
        std::fs::create_dir_all(&covers).unwrap();
        let file = format!("test-{}.img", std::process::id());
        std::fs::write(covers.join(&file), png(30, 10, [9, 9, 9, 255])).unwrap();
        let name = format!("{COVER_PREFIX}{file}");
        let mut p = Pictures::default();
        let wanted = vec![name.clone()];
        // Without a map folder too.
        for _ in 0..400 {
            p.update(None, &wanted, &[], &[]);
            if p.slots().contains_key(&name) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(p.slots().get(&name).map(|s| s.1), Some(3.0));
        p.update(Some(&std::env::temp_dir()), &wanted, &[], &[]);
        assert!(
            p.slots().contains_key(&name),
            "a cover outlives the map folder"
        );
        // Nothing climbs out of the covers' folder either.
        let bad = format!("{COVER_PREFIX}../x.png");
        p.update(None, std::slice::from_ref(&bad), &[], &[]);
        assert!(p.failed.contains(&bad));
        let _ = std::fs::remove_file(covers.join(&file));
    }

    #[test]
    fn pictures_by_path_load_from_anywhere_and_stay_when_the_map_folder_changes() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-by-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Set A")).unwrap();
        // The file as a map names it, in another case than on disk.
        std::fs::write(
            dir.join("Set A").join("BG.png"),
            png(32, 18, [1, 2, 3, 255]),
        )
        .unwrap();
        let path = dir.join("Set A").join("bg.png");
        let name = crate::render::path_picture_key(&path);
        assert!(name.starts_with(PATH_PREFIX));
        assert_eq!(
            name,
            name.to_ascii_lowercase(),
            "the canvas's lowercase names fit it"
        );
        let mut p = Pictures::default();
        let wanted = vec![name.clone()];
        let paths = vec![(name.clone(), path.clone())];
        for _ in 0..400 {
            p.update(None, &wanted, &[], &paths);
            if p.slots().contains_key(&name) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let aspect = p.slots().get(&name).map(|s| s.1);
        assert!(
            aspect.is_some_and(|a| (a - 32.0 / 18.0).abs() < 1e-3),
            "{:?}",
            p.failed
        );
        p.update(Some(&dir), &wanted, &[], &paths);
        assert!(p.slots().contains_key(&name), "it outlives the map folder");
        // Without its path given there's nothing to read: it isn't asked for or failed.
        let other = crate::render::path_picture_key(&dir.join("nope.png"));
        p.update(None, std::slice::from_ref(&other), &[], &[]);
        assert!(!p.pending.contains_key(&other) && !p.failed.contains(&other));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
