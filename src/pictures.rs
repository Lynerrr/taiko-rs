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

use crate::render::{PICTURE_CELL, PICTURE_SLOTS};

/// Larger files are left alone (a picture this big is a mistake, and a slow one).
const MAX_BYTES: u64 = 24 * 1024 * 1024;

type Loaded = (PathBuf, String, Result<(Vec<u8>, f32), String>);

pub struct Pictures {
    dir: Option<PathBuf>,
    slots: Arc<HashMap<String, (u32, f32)>>,
    /// What each slot holds, by slot.
    held: Vec<Option<String>>,
    /// Being read, or unreadable (and not asked again for this folder).
    pending: Vec<String>,
    failed: Vec<String>,
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
            self.dir = dir.map(Path::to_path_buf);
            self.slots = Arc::default();
            self.held.iter_mut().for_each(|h| *h = None);
            self.pending.clear();
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
                }
            }
        }
        let Some(dir) = self.dir.clone() else { return };
        for name in wanted {
            if self.slots.contains_key(name)
                || self.pending.contains(name)
                || self.failed.contains(name)
            {
                continue;
            }
            if !safe_name(name) {
                self.failed.push(name.clone());
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

/// The pictures in `dir` a map can use, by name, sorted.
pub fn in_folder(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            [".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif"]
                .iter()
                .any(|x| l.ends_with(x))
        })
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
        // Another folder: nothing of this one is kept.
        p.update(None, &wanted);
        assert!(p.slots().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
