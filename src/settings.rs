//! Settings tab: the list of options and how it is drawn. Values are read and changed by the
//! caller (it owns the config, audio engine and window).

use crate::render::{rgba, Canvas};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    ScrollSpeed,
    PlayfieldArea,
    PlayfieldPreset,
    PlayfieldReset,
    HudPosition,
    Rate,
    RateKeepPitch,
    Offset,
    Calibrate,
    CalibrationBpm,
    LiveOffset,
    LiveOffsetUr,
    Key(usize),
    /// Previous (0) / next (1) playfield preset key.
    PresetKey(usize),
    ShowPlayfieldSize,
    ShowConverts,
    MusicVolume,
    FxVolume,
    AudioBuffer,
    BackgroundDim,
    PresentMode,
    MenuVsync,
    FpsCap,
    ShowFps,
    Fullscreen,
}

pub const ITEMS: [(&str, Item); 29] = [
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::PlayfieldArea),
    ("GAMEPLAY", Item::PlayfieldPreset),
    ("GAMEPLAY", Item::PlayfieldReset),
    ("GAMEPLAY", Item::PresetKey(0)),
    ("GAMEPLAY", Item::PresetKey(1)),
    ("GAMEPLAY", Item::HudPosition),
    ("GAMEPLAY", Item::ShowPlayfieldSize),
    ("GAMEPLAY", Item::Rate),
    ("GAMEPLAY", Item::RateKeepPitch),
    ("GAMEPLAY", Item::Offset),
    ("GAMEPLAY", Item::Calibrate),
    ("GAMEPLAY", Item::CalibrationBpm),
    ("GAMEPLAY", Item::LiveOffset),
    ("GAMEPLAY", Item::LiveOffsetUr),
    ("GAMEPLAY", Item::Key(0)),
    ("GAMEPLAY", Item::Key(1)),
    ("GAMEPLAY", Item::Key(2)),
    ("GAMEPLAY", Item::Key(3)),
    ("GAMEPLAY", Item::ShowConverts),
    ("AUDIO", Item::MusicVolume),
    ("AUDIO", Item::FxVolume),
    ("AUDIO", Item::AudioBuffer),
    ("VIDEO", Item::BackgroundDim),
    ("VIDEO", Item::PresentMode),
    ("VIDEO", Item::MenuVsync),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
];

const KEY_LABELS: [&str; 4] = ["Left rim (kat)", "Left centre (don)", "Right centre (don)", "Right rim (kat)"];

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::ScrollSpeed => "Scroll speed",
            Item::PlayfieldArea => "Playfield area",
            Item::PlayfieldReset => "Reset playfield area",
            Item::PlayfieldPreset => "Playfield preset",
            Item::HudPosition => "HUD position",
            Item::Rate => "Speed (rate)",
            Item::RateKeepPitch => "Rate keeps pitch",
            Item::Offset => "Audio offset",
            Item::Calibrate => "Offset calibration",
            Item::CalibrationBpm => "Calibration tempo",
            Item::LiveOffset => "Live offset suggestion",
            Item::LiveOffsetUr => "Scale suggestion to UR",
            Item::Key(i) => KEY_LABELS[i],
            Item::PresetKey(0) => "Previous preset key",
            Item::PresetKey(_) => "Next preset key",
            Item::ShowPlayfieldSize => "Show playfield size",
            Item::ShowConverts => "Show osu!standard converts",
            Item::MusicVolume => "Music volume",
            Item::FxVolume => "Hitsound volume",
            Item::AudioBuffer => "Audio buffer",
            Item::BackgroundDim => "Background dim",
            Item::PresentMode => "Present mode (gameplay)",
            Item::MenuVsync => "Menus use vsync",
            Item::FpsCap => "FPS cap",
            Item::ShowFps => "FPS overlay",
            Item::Fullscreen => "Fullscreen",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Item::ScrollSpeed => "How fast notes travel. Shortcut: F3 / F4.",
            Item::PlayfieldArea => "Enter: edit where gameplay is drawn. In game: drag to move, drag edges to resize, wheel to scale.",
            Item::PlayfieldReset => "Enter: make gameplay fill the whole screen again.",
            Item::PlayfieldPreset => "Enter: pick from a list (grouped by aspect ratio). Left/Right: previous/next.",
            Item::HudPosition => "With playfield: score etc. move with it. Screen: they stay at fixed screen positions.",
            Item::Rate => "Song speed like DT (1.50x) / HT (0.75x). Song select: [ and ] (Shift = 0.25x steps).",
            Item::RateKeepPitch => "On: music sounds normal, just faster (DT). Off: pitch rises with speed (nightcore).",
            Item::Offset => "Raise it if you hit late on average (results screen tells you). Shift = 5 ms steps.",
            Item::Calibrate => "Enter: play a click track with notes exactly on the beat; it measures and sets your offset.",
            Item::CalibrationBpm => "Tempo of the calibration click track. Pick one close to the maps you play.",
            Item::LiveOffset => "While playing, suggest an offset from your recent hits; press O to apply it.",
            Item::LiveOffsetUr => "On: more hits and a +- margin when your UR is high. Off: plain average of the last 40 hits.",
            Item::Key(_) => "Enter, then press the new key. Esc cancels.",
            Item::PresetKey(_) => "Steps through playfield presets anywhere (also in game). Enter, then press a key.",
            Item::ShowPlayfieldSize => "Show the playfield's resolution in the bottom-right corner during play.",
            Item::ShowConverts => "Also list standard maps, converted to taiko like osu! does.",
            Item::MusicVolume | Item::FxVolume => "Left / Right to adjust.",
            Item::AudioBuffer => "Smaller = less audio latency; raise it if you hear crackling.",
            Item::BackgroundDim => "How dark the map background is during play. 100% hides it.",
            Item::PresentMode => "Used during songs. vsync is smoothest on Wayland; mailbox/immediate add judder.",
            Item::MenuVsync => "Song select (and its autoplay preview), settings and results always use vsync.",
            Item::FpsCap => "mailbox/immediate only. A few times your refresh rate is plenty.",
            Item::ShowFps => "Frame rate and frame time in the corner. Shortcut: F10.",
            Item::Fullscreen => "Shortcut: F11.",
        }
    }
}

#[derive(Default)]
pub struct SettingsUi {
    pub cursor: usize,
    /// Waiting for a key press for this item (a drum key or a preset key).
    pub binding: Option<Item>,
    /// Open resolution dropdown.
    pub dropdown: Option<Dropdown>,
}

/// The playfield preset dropdown: a cursor over the flattened list, plus where each row was
/// drawn (virtual units) so the mouse can hover and click them.
#[derive(Default)]
pub struct Dropdown {
    pub cursor: usize,
    scroll: f32,
    pub rows: Vec<(usize, [f32; 4])>,
    pub bounds: [f32; 4],
}

impl Dropdown {
    pub fn new(cursor: usize) -> Dropdown {
        Dropdown { cursor, ..Default::default() }
    }

    /// Index of the row under (`x`, `y`) in virtual units.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows.iter().find(|(_, r)| x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]).map(|(i, _)| *i)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let b = self.bounds;
        x >= b[0] && x <= b[0] + b[2] && y >= b[1] && y <= b[1] + b[3]
    }

    /// Draws the list centred on screen. `groups`: (aspect name, resolutions), `current`: the
    /// preset matching the playfield now.
    pub fn draw(&mut self, c: &mut Canvas, groups: &[(&str, Vec<(u32, u32)>)], current: Option<(u32, u32)>, native: (u32, u32)) {
        const ROW: f32 = 15.0;
        const HEAD: f32 = 16.0;
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.55));
        let (bw, max_h) = (300.0, 380.0);
        let content: f32 = groups.iter().map(|(_, l)| HEAD + l.len() as f32 * ROW).sum::<f32>() + 8.0;
        let bh = (content + 50.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        self.bounds = [bx, by, bw, bh];
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
        c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), "Playfield preset");
        c.text_right(bx + bw - 14.0, by + 13.0, 7.0, rgba(0x6f6f86, 1.0), &format!("screen {}x{}", native.0, native.1));

        // Lay out, then scroll so the cursor row stays visible.
        let (top, view) = (by + 32.0, bh - 50.0);
        let mut y = 0.0;
        let mut layout = Vec::new();
        let mut index = 0;
        for (name, list) in groups {
            layout.push((None, *name, y));
            y += HEAD;
            for &(rw, rh) in list {
                layout.push((Some((index, rw, rh)), *name, y));
                index += 1;
                y += ROW;
            }
        }
        let cursor_y = layout.iter().find(|(r, _, _)| r.is_some_and(|(i, _, _)| i == self.cursor)).map(|l| l.2).unwrap_or(0.0);
        let target = (cursor_y - view / 2.0).clamp(0.0, (y - view).max(0.0));
        self.scroll = target;
        self.rows.clear();
        for (row, name, ry) in layout {
            let ry = top + ry - self.scroll;
            if ry < top - 1.0 || ry + ROW > top + view + 1.0 {
                continue;
            }
            match row {
                None => {
                    c.text(bx + 14.0, ry + 5.0, 7.0, rgba(0xEB452C, 1.0), name);
                }
                Some((i, rw, rh)) => {
                    let selected = i == self.cursor;
                    if selected {
                        c.round_rect(bx + 8.0, ry, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(0xEB452C, 1.0), 1.0);
                    }
                    c.text(bx + 22.0, ry + 3.5, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &format!("{rw} x {rh}"));
                    let tag = if Some((rw, rh)) == current { "current" } else { "" };
                    c.text_right(bx + bw - 22.0, ry + 3.5, 7.0, rgba(0x86E05A, 1.0), tag);
                    self.rows.push((i, [bx + 8.0, ry, bw - 16.0, ROW - 2.0]));
                }
            }
        }
        c.text_centered(w / 2.0, by + bh - 9.0, 6.0, rgba(0x6f6f86, 1.0), "Up/Down or mouse to choose   Enter/click apply   Esc close");
    }
}

impl SettingsUi {
    pub fn item(&self) -> Item {
        ITEMS[self.cursor].1
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.cursor = (self.cursor as i32 + delta).clamp(0, ITEMS.len() as i32 - 1) as usize;
    }

    /// `value` gives the current display value of each item. The list scrolls to keep the
    /// selected row visible.
    pub fn draw(&self, c: &mut Canvas, value: &dyn Fn(Item) -> String, footer: &str) {
        const TOP: f32 = 56.0;
        const BOTTOM: f32 = 436.0;
        const ROW: f32 = 19.0;
        const HEADER: f32 = 17.0;
        let w = c.width;
        let col = (w - 48.0).min(640.0);
        let x0 = (w - col) / 2.0;

        // Lay out every row once, then scroll so the cursor row sits inside the viewport.
        let mut rows = Vec::with_capacity(ITEMS.len());
        let mut headers = Vec::new();
        let mut y = 0.0;
        let mut section = "";
        for (sec, _) in ITEMS.iter() {
            if *sec != section {
                section = sec;
                headers.push((y, *sec));
                y += HEADER;
            }
            rows.push(y);
            y += ROW;
        }
        let content = y;
        let view = BOTTOM - TOP;
        let cursor_y = rows[self.cursor];
        let scroll = (cursor_y - view / 2.0).clamp(0.0, (content - view).max(0.0));
        let visible = |y: f32, h: f32| y - scroll >= -1.0 && y - scroll + h <= view + 1.0;

        for (hy, name) in headers {
            if visible(hy, HEADER) {
                c.text(x0, TOP + hy - scroll + 4.0, 8.0, rgba(0xEB452C, 1.0), name);
            }
        }
        for (i, (_, item)) in ITEMS.iter().enumerate() {
            if !visible(rows[i], ROW) {
                continue;
            }
            let y = TOP + rows[i] - scroll;
            let selected = i == self.cursor;
            let (bg, border) = if selected { (0x34344a, 0xEB452C) } else { (0x1d1d27, 0x2a2a38) };
            c.round_rect(x0, y, col, 16.0, 5.0, rgba(bg, 1.0), rgba(border, 1.0), if selected { 1.5 } else { 1.0 });
            c.text(x0 + 10.0, y + 4.5, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), item.label());
            let v = if selected && self.binding.is_some() {
                "press a key...".to_string()
            } else if selected && !matches!(item, Item::Key(_) | Item::PresetKey(_)) {
                format!("<  {}  >", value(*item))
            } else {
                value(*item)
            };
            let v_col = if selected && self.binding.is_some() { 0xFFD34D } else { 0xffffff };
            c.text_right(x0 + col - 10.0, y + 4.5, 7.0, rgba(v_col, 1.0), &v);
        }
        // Scroll hints.
        if scroll > 0.5 {
            c.text_centered(w / 2.0, TOP - 9.0, 7.0, rgba(0x6f6f86, 1.0), "^ more ^");
        }
        if content - scroll > view + 0.5 {
            c.text_centered(w / 2.0, BOTTOM + 1.0, 7.0, rgba(0x6f6f86, 1.0), "v more v");
        }
        c.text_centered(w / 2.0, 448.0, 8.0, rgba(0x9a9ab0, 1.0), self.item().help());
        c.text_centered(w / 2.0, 464.0, 8.0, rgba(0x55556a, 1.0), footer);
    }
}

/// Top bar shared by the song list, settings and multiplayer: logo plus the three tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    c.text(24.0, 24.0, 16.0, rgba(0xffffff, 1.0), "taiko-rs");
    let mut x = 24.0 + Canvas::text_width(16.0, "taiko-rs") + 24.0;
    for (i, name) in ["SONGS", "SETTINGS", "MULTIPLAYER"].iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        let on = i == active;
        c.round_rect(x, 20.0, tw + 20.0, 22.0, 6.0, rgba(if on { 0xEB452C } else { 0x23232f }, 1.0), rgba(if on { 0xEB452C } else { 0x3a3a4e }, 1.0), 1.0);
        c.text(x + 10.0, 27.0, 9.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), name);
        x += tw + 28.0;
    }
    c.text(x + 4.0, 27.0, 8.0, rgba(0x55556a, 1.0), "Tab");
    c.text_right(c.width - 24.0, 28.0, 8.0, rgba(0x9a9ab0, 1.0), right_text);
}
