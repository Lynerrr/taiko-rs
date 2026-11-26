//! Installed UI fonts. Rasterized glyphs are cached once and uploaded into the font texture.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub const WIDTH: usize = 1024;
pub const HEIGHT: usize = 4096;
pub const BASE: f32 = 50000.0;
const PX: f32 = 32.0;

static ATLAS_HEIGHT: AtomicUsize = AtomicUsize::new(HEIGHT);

/// Height of the system-font region in the font texture, bounded by the GPU's texture limit.
pub fn atlas_height() -> usize {
    ATLAS_HEIGHT.load(Ordering::Relaxed)
}

/// Limits the system-font region to `height` rows (at most `HEIGHT`) to match the GPU texture.
pub fn set_atlas_height(height: usize) {
    let clamped = height.min(HEIGHT);
    if ATLAS_HEIGHT.swap(clamped, Ordering::Relaxed) != clamped {
        CACHE.with_borrow_mut(|cache| cache.reset(clamped));
    }
}

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        #[cfg(not(test))]
        {
            db.load_system_fonts();
            #[cfg(target_os = "android")]
            db.load_fonts_dir("/system/fonts");
        }
        #[cfg(test)]
        db.load_font_data(include_bytes!("regression/fixtures/Abel-Regular.ttf").to_vec());
        db
    })
}

pub fn families() -> Vec<String> {
    let mut names: Vec<_> = database()
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .filter(|n| !n.starts_with('.') && !n.contains(['\n', '\r']))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

#[derive(Clone)]
pub struct Glyph {
    /// Texel offset in the custom atlas region.
    pub slot: usize,
    /// Converts raster pixels back to the stable 32px layout metrics.
    pub design_scale: f32,
    pub metrics: fontdue::Metrics,
    pub pixels: Vec<u8>,
    pub revision: u64,
}

#[derive(Default)]
struct Cache {
    requested: String,
    font: Option<fontdue::Font>,
    cap: f32,
    glyphs: HashMap<(char, u16), Option<Rc<Glyph>>>,
    x: usize,
    y: usize,
    row_height: usize,
    atlas_height: usize,
    full: bool,
}

impl Cache {
    fn reset(&mut self, max_h: usize) {
        self.glyphs.clear();
        self.x = 0;
        self.y = 0;
        self.row_height = 0;
        self.atlas_height = max_h;
        self.full = false;
    }

    fn raster_with_height(&mut self, ch: char, px: u16, max_h: usize) -> Option<Rc<Glyph>> {
        if self.atlas_height != max_h {
            self.reset(max_h);
        }
        if let Some(glyph) = self.glyphs.get(&(ch, px)) {
            return glyph.clone();
        }
        let font = self.font.as_ref()?;
        if font.lookup_glyph_index(ch) == 0 {
            self.glyphs.insert((ch, px), None);
            return None;
        }
        let (metrics, pixels) = font.rasterize(ch, f32::from(px));
        let (w, h) = (metrics.width + 2, metrics.height + 2);
        if w > WIDTH || h > max_h {
            return None;
        }
        if self.x + w > WIDTH {
            self.x = 0;
            self.y += self.row_height;
            self.row_height = 0;
        }
        if self.y + h > max_h || self.full {
            self.full = true;
            return None;
        }
        let glyph = Rc::new(Glyph {
            slot: self.y * WIDTH + self.x,
            design_scale: PX / f32::from(px),
            metrics,
            pixels,
            revision: revision(),
        });
        self.x += w;
        self.row_height = self.row_height.max(h);
        self.glyphs.insert((ch, px), Some(glyph.clone()));
        Some(glyph)
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static PIXEL_SCALE: Cell<f32> = const { Cell::new(1.0) };
}

fn revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Failed requests are remembered too, so removed fonts don't trigger disk access each frame.
pub fn select(family: &str) -> bool {
    CACHE.with_borrow_mut(|cache| {
        if cache.requested == family {
            return family.is_empty() || cache.font.is_some();
        }
        let font = if family.is_empty() {
            None
        } else {
            let db = database();
            db.query(&fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                ..Default::default()
            })
            .and_then(|id| {
                db.with_face_data(id, |data, index| {
                    fontdue::Font::from_bytes(
                        data,
                        fontdue::FontSettings {
                            collection_index: index,
                            ..Default::default()
                        },
                    )
                    .ok()
                })
                .flatten()
            })
        };
        let ok = family.is_empty() || font.is_some();
        let cap = font
            .as_ref()
            .map_or(PX, |f| f.metrics('H', PX).height.max(1) as f32);
        *cache = Cache {
            requested: family.into(),
            font,
            cap,
            atlas_height: atlas_height(),
            ..Default::default()
        };
        ok
    })
}

/// Keep the atlas bounded without permanently losing glyphs after browsing many map titles.
/// Reset only between frames: all shapes in a frame must keep the slots they measured.
pub fn begin_frame() {
    let max_h = atlas_height();
    CACHE.with_borrow_mut(|cache| {
        if cache.full || cache.atlas_height != max_h {
            cache.reset(max_h);
        }
    });
}

pub fn active() -> bool {
    CACHE.with_borrow(|c| c.font.is_some())
}
pub fn scale(size: f32) -> f32 {
    CACHE.with_borrow(|c| size / c.cap.max(1.0))
}
pub fn cap() -> f32 {
    CACHE.with_borrow(|c| c.cap)
}

/// Physical pixels per UI unit, set by the screen or video renderer before building a frame.
pub fn set_pixel_scale(scale: f32) {
    PIXEL_SCALE.set(if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    });
}

/// Physical pixels per UI unit (`set_pixel_scale`).
pub fn pixel_scale() -> f32 {
    PIXEL_SCALE.get()
}

pub fn snap(value: f32) -> f32 {
    let scale = PIXEL_SCALE.get();
    (value * scale).round() / scale
}

/// Layout uses stable metrics, independently of the resolution of a glyph's bitmap.
#[cfg(test)]
pub fn glyph(ch: char) -> Option<Rc<Glyph>> {
    raster(ch, PX as u16)
}

/// Measurements do not allocate atlas space or depend on rounded raster sizes.
pub fn advance(ch: char) -> Option<f32> {
    CACHE.with_borrow(|cache| {
        let font = cache.font.as_ref()?;
        (font.lookup_glyph_index(ch) != 0).then(|| font.metrics(ch, PX).advance_width)
    })
}

pub fn glyph_at(ch: char, size: f32) -> Option<Rc<Glyph>> {
    let em = PX * scale(size) * PIXEL_SCALE.get();
    raster(ch, em.round().clamp(4.0, 512.0) as u16)
}

fn raster(ch: char, px: u16) -> Option<Rc<Glyph>> {
    let max_h = atlas_height();
    CACHE.with_borrow_mut(|cache| cache.raster_with_height(ch, px, max_h))
}

#[cfg(all(test, feature = "gpu-tests"))]
pub fn glyph_in_slot(slot: usize) -> Rc<Glyph> {
    CACHE.with_borrow(|cache| {
        cache
            .glyphs
            .values()
            .flatten()
            .find(|g| g.slot == slot)
            .unwrap()
            .clone()
    })
}

/// Revisions are global so newly created renderers and a change of font both upload correctly.
pub fn uploads(since: u64) -> Vec<Rc<Glyph>> {
    CACHE.with_borrow(|c| {
        c.glyphs
            .values()
            .filter_map(|g| g.as_ref())
            .filter(|g| g.revision > since)
            .cloned()
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_atlas_restarts_between_frames_with_new_upload_revisions() {
        assert!(select("Abel"));
        let old = glyph('W').unwrap();
        CACHE.with_borrow_mut(|cache| cache.full = true);
        assert!(glyph('i').is_none());
        // Already drawn glyphs remain valid until the frame is over.
        assert_eq!(glyph('W').unwrap().revision, old.revision);
        begin_frame();
        let new = glyph('i').unwrap();
        assert_eq!(new.slot, 0);
        assert!(new.revision > old.revision);
        assert_eq!(uploads(old.revision).len(), 1);
        select("");
    }

    #[test]
    fn reduced_atlas_height_keeps_all_glyphs_within_bounds() {
        let font = fontdue::Font::from_bytes(
            include_bytes!("regression/fixtures/Abel-Regular.ttf").as_slice(),
            fontdue::FontSettings::default(),
        )
        .unwrap();
        let mut cache = Cache {
            font: Some(font),
            cap: PX,
            atlas_height: 40,
            ..Default::default()
        };
        let mut placed = 0;
        for ch in '!'..='~' {
            if let Some(g) = cache.raster_with_height(ch, 32, 40) {
                let y = g.slot / WIDTH;
                let h = g.metrics.height + 2;
                assert!(y + h <= 40, "glyph {ch} exceeded reduced atlas height");
                placed += 1;
            }
        }
        assert!(placed > 0, "at least one glyph fits in the reduced atlas");
        assert!(cache.full, "cache marks full instead of overflowing max_h");
        // Zero height disables custom glyph placement cleanly.
        assert!(cache.raster_with_height('A', 32, 0).is_none());
    }
}
