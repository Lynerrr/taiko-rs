//! Settings tab: the list of options and how it is drawn. Values are read and changed by the
//! caller (it owns the config, audio engine and window).

use std::cell::Cell;

use crate::render::{approach, rgba, Canvas, Hit};

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
    ResumeDelay,
    Key(usize),
    /// Previous (0) / next (1) playfield preset key.
    PresetKey(usize),
    ShowPlayfieldSize,
    KiaiEffects,
    ShowConverts,
    MasterVolume,
    MusicVolume,
    FxVolume,
    AudioBuffer,
    BackgroundDim,
    PresentMode,
    MenuVsync,
    FpsCap,
    ShowFps,
    Animations,
    Fullscreen,
}

pub const ITEMS: [(&str, Item); 33] = [
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::Rate),
    ("GAMEPLAY", Item::RateKeepPitch),
    ("GAMEPLAY", Item::ResumeDelay),
    ("GAMEPLAY", Item::ShowConverts),
    ("PLAYFIELD", Item::PlayfieldArea),
    ("PLAYFIELD", Item::PlayfieldPreset),
    ("PLAYFIELD", Item::PlayfieldReset),
    ("PLAYFIELD", Item::HudPosition),
    ("PLAYFIELD", Item::ShowPlayfieldSize),
    ("PLAYFIELD", Item::BackgroundDim),
    ("PLAYFIELD", Item::KiaiEffects),
    ("INPUT", Item::Key(0)),
    ("INPUT", Item::Key(1)),
    ("INPUT", Item::Key(2)),
    ("INPUT", Item::Key(3)),
    ("INPUT", Item::PresetKey(0)),
    ("INPUT", Item::PresetKey(1)),
    ("OFFSET", Item::Offset),
    ("OFFSET", Item::Calibrate),
    ("OFFSET", Item::CalibrationBpm),
    ("OFFSET", Item::LiveOffset),
    ("OFFSET", Item::LiveOffsetUr),
    ("AUDIO", Item::MasterVolume),
    ("AUDIO", Item::MusicVolume),
    ("AUDIO", Item::FxVolume),
    ("AUDIO", Item::AudioBuffer),
    ("VIDEO", Item::PresentMode),
    ("VIDEO", Item::MenuVsync),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
    ("VIDEO", Item::Animations),
];

/// The sections in order, with a one-line summary for the sidebar.
pub const SECTIONS: [(&str, &str); 6] = [
    ("GAMEPLAY", "speed, rate, resume"),
    ("PLAYFIELD", "size, position, HUD"),
    ("INPUT", "drum and preset keys"),
    ("OFFSET", "audio offset, calibration"),
    ("AUDIO", "volume, latency"),
    ("VIDEO", "frame pacing, display"),
];

/// First item of section `s`.
pub fn section_start(s: usize) -> usize {
    ITEMS.iter().position(|(sec, _)| *sec == SECTIONS[s].0).unwrap_or(0)
}

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
            Item::ResumeDelay => "Resume delay",
            Item::Key(i) => KEY_LABELS[i],
            Item::PresetKey(0) => "Previous preset key",
            Item::PresetKey(_) => "Next preset key",
            Item::ShowPlayfieldSize => "Show playfield size",
            Item::KiaiEffects => "Kiai effects",
            Item::ShowConverts => "Show osu!standard converts",
            Item::MasterVolume => "Master volume",
            Item::MusicVolume => "Music volume",
            Item::FxVolume => "Hitsound volume",
            Item::AudioBuffer => "Audio buffer",
            Item::BackgroundDim => "Background dim",
            Item::PresentMode => "Present mode (gameplay)",
            Item::MenuVsync => "Menus use vsync",
            Item::FpsCap => "FPS cap",
            Item::ShowFps => "FPS overlay",
            Item::Animations => "UI animations",
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
            Item::ResumeDelay => "Countdown before the song continues after the pause menu (0 = at once).",
            Item::Key(_) => "Enter, then press the new key. Esc cancels.",
            Item::PresetKey(_) => "Steps through playfield presets anywhere (also in game). Enter, then press a key.",
            Item::ShowPlayfieldSize => "Show the playfield's resolution in the bottom-right corner during play.",
            Item::ShowConverts => "Also list standard maps, converted to taiko like osu! does.",
            Item::KiaiEffects => "During kiai time, tint the lane and pulse the hit target with the beat.",
            Item::MasterVolume => "Scales music and hitsounds. Anywhere: hold Alt and scroll the mouse wheel.",
            Item::MusicVolume | Item::FxVolume => "Left / Right to adjust.",
            Item::AudioBuffer => "Smaller = less audio latency; raise it if you hear crackling.",
            Item::BackgroundDim => "How dark the map background is during play. 100% hides it.",
            Item::PresentMode => "Used during songs. vsync is smoothest on Wayland; mailbox/immediate add judder.",
            Item::MenuVsync => "Song select (and its autoplay preview), settings and results always use vsync.",
            Item::FpsCap => "mailbox/immediate only. A few times your refresh rate is plenty.",
            Item::ShowFps => "Frame rate and frame time in the corner. Shortcut: F10.",
            Item::Animations => "Menu motion: screen fades, sliding highlights, opening lists, results count-up. Gameplay is unaffected.",
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
    /// The section last drawn and the animated highlight position in it.
    scroll: Cell<f32>,
    highlight: Cell<f32>,
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

    /// Section of the selected item.
    pub fn section(&self) -> usize {
        SECTIONS.iter().position(|(name, _)| *name == ITEMS[self.cursor].0).unwrap_or(0)
    }

    /// A sidebar of sections and the selected section's settings. `value` gives the current
    /// display value of each item.
    pub fn draw(&self, c: &mut Canvas, value: &dyn Fn(Item) -> String, footer: &str) {
        const TOP: f32 = 62.0;
        const ROW: f32 = 30.0;
        let w = c.width;
        let total = (w - 48.0).min(780.0);
        let x0 = (w - total) / 2.0;
        let side_w = 170.0;
        let (cx, col) = (x0 + side_w + 16.0, total - side_w - 16.0);
        let section = self.section();

        // Sidebar.
        for (i, (name, about)) in SECTIONS.iter().enumerate() {
            let y = TOP + i as f32 * 40.0;
            let on = i == section;
            let hover = !on && c.hovered(x0, y, side_w, 34.0);
            let bg = if on { 0x2b1a17 } else if hover { 0x262634 } else { 0x1d1d27 };
            c.round_rect(x0, y, side_w, 34.0, 7.0, rgba(bg, 0.95), rgba(if on { 0xEB452C } else { 0x2a2a38 }, 1.0), if on { 1.5 } else { 1.0 });
            c.hit(x0, y, side_w, 34.0, Hit::SettingsSection(i));
            c.text(x0 + 12.0, y + 7.0, 9.0, rgba(if on { 0xffffff } else { 0xc8c8d8 }, 1.0), name);
            c.text(x0 + 12.0, y + 21.0, 6.0, rgba(0x8a8aa0, 1.0), about);
        }

        // The section's settings.
        let rows: Vec<(usize, Item)> = ITEMS.iter().enumerate().filter(|(_, (sec, _))| *sec == SECTIONS[section].0).map(|(i, (_, it))| (i, *it)).collect();
        c.text(cx, TOP - 2.0, 11.0, rgba(0xEB452C, 1.0), SECTIONS[section].0);
        let first = rows.first().map(|r| r.0).unwrap_or(0);
        let target = (self.cursor - first) as f32 * ROW;
        // The highlight glides within a section and jumps when the section changes.
        let mut hl = self.highlight.get();
        if self.scroll.get() != section as f32 || !hl.is_finite() {
            hl = target;
            self.scroll.set(section as f32);
        }
        approach(&mut hl, target, c.dt_ms, 35.0);
        self.highlight.set(hl);
        let list_top = TOP + 18.0;
        for (k, (i, item)) in rows.iter().enumerate() {
            let y = list_top + k as f32 * ROW;
            let selected = *i == self.cursor;
            let hover = !selected && c.hovered(cx, y, col, ROW - 4.0);
            let bg = if selected { 0x34344a } else if hover { 0x262634 } else { 0x1d1d27 };
            c.round_rect(cx, y, col, ROW - 4.0, 6.0, rgba(bg, 0.95), rgba(0x2a2a38, 1.0), 1.0);
            c.hit(cx, y - 2.0, col, ROW, Hit::SettingRow(*i));
            c.text(cx + 12.0, y + 9.5, 8.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), item.label());
            let v = if selected && self.binding.is_some() {
                "press a key...".to_string()
            } else if selected && !matches!(item, Item::Key(_) | Item::PresetKey(_)) {
                format!("<  {}  >", value(*item))
            } else {
                value(*item)
            };
            let v_col = if selected && self.binding.is_some() { 0xFFD34D } else { 0xffffff };
            c.text_right(cx + col - 12.0, y + 9.5, 8.0, rgba(v_col, 1.0), &v);
            if selected && self.binding.is_none() && !matches!(item, Item::Key(_) | Item::PresetKey(_)) {
                // The "<" and ">" around the value step it.
                let tw = Canvas::text_width(8.0, &v);
                c.hit(cx + col - 12.0 - tw - 6.0, y, 20.0, ROW - 4.0, Hit::SettingStep(*i, -1));
                c.hit(cx + col - 26.0, y, 24.0, ROW - 4.0, Hit::SettingStep(*i, 1));
            }
        }
        c.round_rect(cx, list_top + hl, col, ROW - 4.0, 6.0, rgba(0, 0.0), rgba(0xEB452C, 1.0), 1.5);

        // What the selected setting does.
        let help_y = list_top + rows.len() as f32 * ROW + 10.0;
        c.round_rect(cx, help_y, col, 30.0, 6.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
        let help = self.item().help();
        let max = ((col - 20.0) / (7.0 / 7.0 * 6.0)) as usize;
        let (line1, line2) = split_help(help, max);
        c.text(cx + 10.0, help_y + 6.0, 7.0, rgba(0x9a9ab0, 1.0), &line1);
        c.text(cx + 10.0, help_y + 17.0, 7.0, rgba(0x9a9ab0, 1.0), &line2);
        c.text_centered(w / 2.0, 462.0, 8.0, rgba(0x55556a, 1.0), footer);
    }
}

/// Wraps help text onto two lines of at most `max` characters.
fn split_help(text: &str, max: usize) -> (String, String) {
    if text.chars().count() <= max {
        return (text.to_string(), String::new());
    }
    let cut = text.char_indices().take_while(|(i, _)| *i <= max).filter(|(_, ch)| *ch == ' ').map(|(i, _)| i).last().unwrap_or(max.min(text.len()));
    let rest: String = text[cut..].trim_start().chars().take(max).collect();
    (text[..cut].to_string(), rest)
}

/// Top bar shared by the song list, settings and multiplayer: logo plus the three tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    thread_local! {
        /// Where the active tab's highlight is (x, width), so it slides between tabs.
        static PILL: Cell<(f32, f32)> = const { Cell::new((f32::NAN, 0.0)) };
    }
    c.text(24.0, 24.0, 16.0, rgba(0xffffff, 1.0), "taiko-rs");
    let x0 = 24.0 + Canvas::text_width(16.0, "taiko-rs") + 24.0;
    let names = ["SONGS", "SETTINGS", "MULTIPLAYER", "DOWNLOAD"];
    let mut x = x0;
    let mut target = (x0, 0.0);
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        let on = i == active;
        let hover = !on && c.hovered(x, 20.0, tw + 20.0, 22.0);
        let fill = if hover { 0x34344a } else { 0x23232f };
        c.round_rect(x, 20.0, tw + 20.0, 22.0, 6.0, rgba(fill, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
        c.hit(x, 20.0, tw + 20.0, 22.0, Hit::Tab(i));
        if on {
            target = (x, tw + 20.0);
        }
        x += tw + 28.0;
    }
    let (mut px, mut pw) = PILL.with(|p| p.get());
    if !px.is_finite() {
        (px, pw) = target;
    }
    approach(&mut px, target.0, c.dt_ms, 40.0);
    approach(&mut pw, target.1, c.dt_ms, 40.0);
    PILL.with(|p| p.set((px, pw)));
    c.round_rect(px, 20.0, pw, 22.0, 6.0, rgba(0xEB452C, 1.0), rgba(0xEB452C, 1.0), 1.0);
    let mut x = x0;
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        c.text(x + 10.0, 27.0, 9.0, rgba(if i == active { 0xffffff } else { 0x9a9ab0 }, 1.0), name);
        x += tw + 28.0;
    }
    c.text(x + 4.0, 27.0, 8.0, rgba(0x55556a, 1.0), "Tab");
    c.text_right(c.width - 24.0, 28.0, 8.0, rgba(0x9a9ab0, 1.0), right_text);
}
