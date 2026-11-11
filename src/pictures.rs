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

/// Names starting with this are the download tab's result covers, read from
/// `download::covers_dir` whatever map folder is followed.
pub const COVER_PREFIX: &str = "@cover/";

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

    /// Follows the map folder `dir` and loads the pictures `wanted` (lowercase file names).
    pub fn update(&mut self, dir: Option<&Path>, wanted: &[String]) {
        if self.dir.as_deref() != dir {
            // The covers don't come from the folder: they stay.
            let cover = |n: &String| n.starts_with(COVER_PREFIX);
            self.dir = dir.map(Path::to_path_buf);
            let mut slots = (*self.slots).clone();
            slots.retain(|n, _| cover(n));
            self.slots = Arc::new(slots);
            for h in &mut self.held {
                if h.as_ref().is_some_and(|n| !cover(n)) {
                    *h = None;
                }
            }
            self.pending.retain(cover);
            self.failed.retain(cover);
        }
        if !self.failed.is_empty() && Instant::now() >= self.retry_at {
            self.failed.clear();
        }
        let covers = crate::download::covers_dir();
        while let Ok((from, name, got)) = self.rx.try_recv() {
            if Some(&from) != self.dir.as_ref() && from != covers {
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
            if self.slots.contains_key(name)
                || self.pending.contains(name)
                || self.failed.contains(name)
            {
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
            self.pending.push(name.clone());
            let (tx, name, file) = (self.tx.clone(), name.clone(), file.to_string());
            std::thread::spawn(move || {
                let got = read(&dir, &file);
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

pub use crate::tko::clips::safe_name;

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
    Ok((px, aspect))
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
            p.update(None, &wanted);
            if p.slots().contains_key(&name) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(p.slots().get(&name).map(|s| s.1), Some(3.0));
        p.update(Some(&std::env::temp_dir()), &wanted);
        assert!(
            p.slots().contains_key(&name),
            "a cover outlives the map folder"
        );
        // Nothing climbs out of the covers' folder either.
        let bad = format!("{COVER_PREFIX}../x.png");
        p.update(None, std::slice::from_ref(&bad));
        assert!(p.failed.contains(&bad));
        let _ = std::fs::remove_file(covers.join(&file));
    }
}
