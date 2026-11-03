//! Profile pictures from the website (`/a/<name>`), shrunk to a square cell of the renderer's
//! avatar texture. A picture is fetched on a background thread the first time it's drawn
//! (`Canvas::avatar` asks); until then, and for players without one, their initial shows.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// A cell's side (pixels), and how many cells to a side of the texture.
pub const CELL: u32 = 64;
pub const GRID: u32 = 8;
const SLOTS: usize = (GRID * GRID) as usize;
/// Fetches at once.
const IN_FLIGHT: usize = 4;
/// A shown picture is looked at again this long after (it may have changed).
const REFRESH: Duration = Duration::from_secs(15 * 60);
/// A player without one is asked about again after this; a failed fetch after less.
const RETRY_MISSING: Duration = Duration::from_secs(10 * 60);
const RETRY_FAILED: Duration = Duration::from_secs(60);
/// The largest picture read (the server takes up to 2 MB).
const MAX_BYTES: u64 = 3 << 20;

enum Entry {
    /// Being fetched for the first time (or again, after it was missing).
    Fetching,
    /// In its slot (`slots`), since `at`.
    Shown { at: Instant },
    /// None on the server, or it couldn't be had: asked again after `retry`.
    Missing { at: Instant, retry: Duration },
}

type Fetched = (String, Result<Option<Vec<u8>>, String>);

pub struct Avatars {
    /// Where the pictures are (`online::WEBSITE`); None: nowhere (another server, tests).
    base: Option<String>,
    entries: HashMap<String, Entry>,
    /// What the renderer has, by name: their slots (handed to the canvas each frame).
    slots: Arc<HashMap<String, u32>>,
    /// Each slot's name and when it was last drawn.
    owners: Vec<Option<(String, Instant)>>,
    fetching: usize,
    tx: Sender<Fetched>,
    rx: Receiver<Fetched>,
    /// Pictures for the renderer: slot, `CELL` square premultiplied RGBA.
    uploads: Vec<(u32, Vec<u8>)>,
}

impl Avatars {
    pub fn new(base: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel();
        Avatars {
            base,
            entries: HashMap::new(),
            slots: Arc::default(),
            owners: vec![None; SLOTS],
            fetching: 0,
            tx,
            rx,
            uploads: Vec::new(),
        }
    }

    /// The pictures the renderer has, for the canvas.
    pub fn slots(&self) -> Arc<HashMap<String, u32>> {
        self.slots.clone()
    }

    /// Pictures to put in the renderer's texture.
    pub fn take_uploads(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// After a frame: takes in finished fetches and starts the ones `wanted` (lowercased names
    /// drawn) needs.
    pub fn update(&mut self, wanted: &[String]) {
        let now = Instant::now();
        while let Ok((name, result)) = self.rx.try_recv() {
            self.fetching -= 1;
            match result {
                Ok(Some(rgba)) => self.store(name, rgba, now),
                Ok(None) => self.forget(name, now, RETRY_MISSING),
                Err(_) => self.forget(name, now, RETRY_FAILED),
            }
        }
        for name in wanted {
            if let Some(&slot) = self.slots.get(name) {
                self.owners[slot as usize] = Some((name.clone(), now));
            }
        }
        let Some(base) = self.base.clone() else {
            return;
        };
        for name in wanted {
            if self.fetching >= IN_FLIGHT {
                break;
            }
            let due = match self.entries.get(name) {
                None => true,
                Some(Entry::Fetching) => false,
                Some(Entry::Shown { at, .. }) => now.duration_since(*at) >= REFRESH,
                Some(Entry::Missing { at, retry }) => now.duration_since(*at) >= *retry,
            };
            if !due || !valid_name(name) {
                continue;
            }
            // A picture being looked at again stays up meanwhile.
            if !matches!(self.entries.get(name), Some(Entry::Shown { .. })) {
                self.entries.insert(name.clone(), Entry::Fetching);
            } else if let Some(Entry::Shown { at, .. }) = self.entries.get_mut(name) {
                *at = now;
            }
            self.fetching += 1;
            let (tx, name, url) = (self.tx.clone(), name.clone(), format!("{base}/a/{name}"));
            std::thread::spawn(move || {
                let _ = tx.send((name, fetch(&url)));
            });
        }
    }

    fn store(&mut self, name: String, rgba: Vec<u8>, now: Instant) {
        let slot = match self.slots.get(&name) {
            Some(&slot) => slot,
            None => {
                // A free slot, else the one drawn longest ago.
                let slot = self
                    .owners
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or_else(|| {
                        self.owners
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, o)| o.as_ref().map(|(_, at)| *at))
                            .map_or(0, |(i, _)| i)
                    }) as u32;
                if let Some((old, _)) = self.owners[slot as usize].take() {
                    Arc::make_mut(&mut self.slots).remove(&old);
                    // Fetched again when next drawn.
                    self.entries.remove(&old);
                }
                slot
            }
        };
        self.owners[slot as usize] = Some((name.clone(), now));
        Arc::make_mut(&mut self.slots).insert(name.clone(), slot);
        self.entries.insert(name, Entry::Shown { at: now });
        self.uploads.push((slot, rgba));
    }

    fn forget(&mut self, name: String, now: Instant, retry: Duration) {
        // A picture that was there and is gone (taken off) leaves its slot.
        if let Some(slot) = Arc::make_mut(&mut self.slots).remove(&name) {
            self.owners[slot as usize] = None;
        }
        let _ = self.entries.insert(name, Entry::Missing { at: now, retry });
    }

    #[cfg(test)]
    fn slot_of(&self, name: &str) -> Option<u32> {
        self.slots.get(name).copied()
    }
}

/// Names as accounts have them (so nothing odd goes into a URL).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The picture at `url` as a cell (None: the player has none).
fn fetch(url: &str) -> Result<Option<Vec<u8>>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = match agent
        .get(url)
        .header(
            "User-Agent",
            concat!("taiko-rs/", env!("CARGO_PKG_VERSION")),
        )
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    cell(&bytes).map(Some)
}

/// `bytes` (an image file) as a `CELL` square: the middle of it, premultiplied RGBA.
pub fn cell(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let mut px = image
        .resize_to_fill(CELL, CELL, image::imageops::FilterType::Triangle)
        .to_rgba8()
        .into_raw();
    for p in px.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for v in &mut p[..3] {
            *v = ((u16::from(*v) * a + 127) / 255) as u8;
        }
    }
    Ok(px)
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
    fn a_picture_becomes_a_premultiplied_square() {
        let px = cell(&png(300, 120, [200, 100, 50, 128])).unwrap();
        assert_eq!(px.len(), (CELL * CELL * 4) as usize);
        assert_eq!(&px[..4], &[100, 50, 25, 128]);
        assert!(cell(b"not an image").is_err());
    }

    #[test]
    fn slots_go_to_the_pictures_drawn_last() {
        let mut a = Avatars::new(None);
        let blank = vec![0u8; (CELL * CELL * 4) as usize];
        // In the past, so that being drawn now is the latest.
        let t0 = Instant::now() - Duration::from_secs(1);
        for i in 0..SLOTS {
            a.store(
                format!("p{i}"),
                blank.clone(),
                t0 + Duration::from_millis(i as u64),
            );
        }
        assert!(a.owners.iter().all(Option::is_some));
        // p0 is drawn again, so p1 is the one drawn longest ago.
        a.update(&["p0".to_string()]);
        a.store("new".into(), blank.clone(), Instant::now());
        assert_eq!(a.slot_of("new"), Some(1));
        assert!(!a.slots().contains_key("p1") && a.slots().contains_key("p0"));
        assert_eq!(a.take_uploads().len(), SLOTS + 1);
        // Taken off the website: its slot is free again.
        a.forget("new".into(), Instant::now(), RETRY_MISSING);
        assert!(a.owners[1].is_none() && !a.slots().contains_key("new"));
    }

    #[test]
    fn only_account_names_are_fetched() {
        assert!(valid_name("auneu") && valid_name("a_b-c9"));
        assert!(!valid_name("") && !valid_name("../x") && !valid_name("a b") && !valid_name("x?y"));
    }
}
