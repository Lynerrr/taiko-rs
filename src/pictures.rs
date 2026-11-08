//! A `.tko` map's own pictures (the layers of its clips), read from the map's folder.
//!
//! The canvas lists the pictures drawn each frame (`Canvas::pictures_wanted`); this loads them
//! from the folder of the map being played or edited, off the main thread, squeezes each into a
//! `render::PICTURE_CELL` square and hands it to the renderer (`Renderer::set_picture`). The
//! picture keeps its own shape on screen: the canvas stretches the square back by its aspect.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::render::{PICTURE_CELL, PICTURE_SLOTS};

/// Larger files are left alone (a picture this big is a mistake, and a slow one).
const MAX_BYTES: u64 = 24 * 1024 * 1024;

/// How long a picture that couldn't be read waits before it is looked for again.
const RETRY: Duration = Duration::from_secs(2);

type Loaded = (PathBuf, String, Result<(Vec<u8>, f32), String>);

pub struct Pictures {
    dir: Option<PathBuf>,
    slots: Arc<HashMap<String, (u32, f32)>>,
    /// What each slot holds, by slot.
    held: Vec<Option<String>>,
    /// Being read, or unreadable (and not asked again for this folder).
    pending: Vec<String>,
    failed: Vec<String>,
    /// When the failed ones are tried again (a file put in the folder later, or fixed).
    retry_at: Instant,
    uploads: Vec<(u32, Vec<u8>)>,
    /// Pictures handed in ready (`provide`), not read from the map's folder: placed whenever
    /// drawn, whatever folder is followed.
    provided: HashMap<String, (Arc<Vec<u8>>, f32)>,
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
            pending: Vec::new(),
            failed: Vec::new(),
            retry_at: Instant::now(),
            uploads: Vec::new(),
            provided: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl Pictures {
    pub fn slots(&self) -> Arc<HashMap<String, (u32, f32)>> {
        self.slots.clone()
    }

    pub fn take_uploads(&mut self) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut self.uploads)
    }

    /// Hands in the pictures whose names start with `prefix` (lowercase), ready as `cell` made
    /// them, in place of any it had: drawn by those names like a map's own.
    pub fn provide(&mut self, prefix: &str, pictures: Vec<(String, Vec<u8>, f32)>) {
        self.provided.retain(|name, _| !name.starts_with(prefix));
        let gone: Vec<String> = self
            .slots
            .keys()
            .filter(|n| n.starts_with(prefix))
            .cloned()
            .collect();
        if !gone.is_empty() {
            let mut slots = (*self.slots).clone();
            for name in gone {
                slots.remove(&name);
                self.held
                    .iter_mut()
                    .filter(|h| h.as_ref() == Some(&name))
                    .for_each(|h| *h = None);
            }
            self.slots = Arc::new(slots);
        }
        for (name, rgba, aspect) in pictures {
            self.provided.insert(name, (Arc::new(rgba), aspect));
        }
    }

    /// Follows the map folder `dir` and loads the pictures `wanted` (lowercase file names).
    pub fn update(&mut self, dir: Option<&Path>, wanted: &[String]) {
        if self.dir.as_deref() != dir {
            self.dir = dir.map(Path::to_path_buf);
            self.slots = Arc::default();
            self.held.iter_mut().for_each(|h| *h = None);
            self.pending.clear();
            self.failed.clear();
        }
        if !self.failed.is_empty() && Instant::now() >= self.retry_at {
            self.failed.clear();
        }
        while let Ok((from, name, got)) = self.rx.try_recv() {
            if Some(&from) != self.dir.as_ref() {
                continue;
            }
            self.pending.retain(|n| *n != name);
            match got {
                Ok((rgba, aspect)) => self.place(name, rgba, aspect, wanted),
                Err(e) => {
                    eprintln!("map picture {name}: {e}");
                    self.failed.push(name);
                    self.retry_at = Instant::now() + RETRY;
                }
            }
        }
        for name in wanted {
            if !self.slots.contains_key(name)
                && let Some((rgba, aspect)) = self.provided.get(name).cloned()
            {
                self.place(name.clone(), (*rgba).clone(), aspect, wanted);
            }
        }
        let Some(dir) = self.dir.clone() else { return };
        for name in wanted {
            // Names starting with \u{1} are only ever handed in (`provide`), never files.
            if self.provided.contains_key(name) || name.starts_with('\u{1}') {
                continue;
            }
            if self.slots.contains_key(name)
                || self.pending.contains(name)
                || self.failed.contains(name)
            {
                continue;
            }
            if !safe_name(name) {
                self.failed.push(name.clone());
                self.retry_at = Instant::now() + RETRY;
                continue;
            }
            self.pending.push(name.clone());
            let (tx, dir, name) = (self.tx.clone(), dir.clone(), name.clone());
            std::thread::spawn(move || {
                let got = read(&dir, &name);
                let _ = tx.send((dir, name, got));
            });
        }
    }

    /// Puts a loaded picture in a free slot, or in one holding a picture not drawn now.
    fn place(&mut self, name: String, rgba: Vec<u8>, aspect: f32, wanted: &[String]) {
        let free = self.held.iter().position(Option::is_none).or_else(|| {
            self.held
                .iter()
                .position(|h| h.as_ref().is_some_and(|h| !wanted.contains(h)))
        });
        let Some(slot) = free else {
            // Every slot is drawn now: it waits for one to be let go.
            return;
        };
        let mut slots = (*self.slots).clone();
        if let Some(old) = self.held[slot].take() {
            slots.remove(&old);
        }
        slots.insert(name.clone(), (slot as u32, aspect));
        self.held[slot] = Some(name);
        self.slots = Arc::new(slots);
        self.uploads.push((slot as u32, rgba));
    }
}

/// A file name a map may give for a picture: inside its folder, nothing climbing out of it.
pub fn safe_name(name: &str) -> bool {
    let p = Path::new(name);
    !name.is_empty()
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The picture `name` in `dir` (matched ignoring case, as maps from Windows expect).
fn read(dir: &Path, name: &str) -> Result<(Vec<u8>, f32), String> {
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
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > MAX_BYTES {
        return Err(format!("{} MB is too big", size / 1024 / 1024));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    cell(&bytes)
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
    Ok(cell_of(&image))
}

/// `image` squeezed into a `PICTURE_CELL` square, premultiplied RGBA, and its width over its
/// height.
pub fn cell_of(image: &image::DynamicImage) -> (Vec<u8>, f32) {
    let aspect = image.width().max(1) as f32 / image.height().max(1) as f32;
    let mut px = image
        .resize_exact(
            PICTURE_CELL,
            PICTURE_CELL,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8()
        .into_raw();
    for p in px.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        for v in &mut p[..3] {
            *v = ((u16::from(*v) * a + 127) / 255) as u8;
        }
    }
    (px, aspect)
}

/// Whether `name` is a kind of picture a map can show.
pub fn is_picture(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif"]
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
            p.update(Some(&dir), &wanted);
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
        p.update(Some(&dir), &wanted);
        assert!(!p.slots().contains_key("missing.png"));
        p.retry_at = Instant::now();
        for _ in 0..400 {
            p.update(Some(&dir), &wanted);
            if p.slots().contains_key("missing.png") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(p.slots().contains_key("missing.png"), "{:?}", p.failed);
        // Another folder: nothing of this one is kept.
        p.update(None, &wanted);
        assert!(p.slots().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provided_pictures_are_placed_in_any_folder_and_replaced_by_prefix() {
        let mut p = Pictures::default();
        let cell = vec![0u8; (PICTURE_CELL * PICTURE_CELL * 4) as usize];
        let name = "\u{1}ot:0".to_string();
        p.provide("\u{1}ot:", vec![(name.clone(), cell.clone(), 2.0)]);
        p.update(None, std::slice::from_ref(&name));
        assert_eq!(p.slots().get(&name).map(|s| s.1), Some(2.0));
        assert_eq!(p.take_uploads().len(), 1);
        // Another folder starts over; it is placed again.
        let dir = std::env::temp_dir();
        p.update(Some(&dir), std::slice::from_ref(&name));
        assert!(p.slots().contains_key(&name));
        assert!(
            p.pending.is_empty() && p.failed.is_empty(),
            "never read as a file"
        );
        // Handed in again with none: gone.
        p.provide("\u{1}ot:", Vec::new());
        p.update(Some(&dir), std::slice::from_ref(&name));
        assert!(!p.slots().contains_key(&name));
        assert!(p.pending.is_empty() && p.failed.is_empty());
    }
}
