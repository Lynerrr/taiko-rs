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
    Rate,
    RateKeepPitch,
    Offset,
    Calibrate,
    CalibrationBpm,
    LiveOffset,
    LiveOffsetUr,
    ResumeDelay,
    AntiMash,
    Key(usize),
    /// Previous (0) / next (1) playfield preset key.
    PresetKey(usize),
    TouchLayout,
    ShowPlayfieldSize,
    KiaiEffects,
    ShowConverts,
    HiddenMaps,
    MasterVolume,
    MusicVolume,
    FxVolume,
    AudioBuffer,
    BackgroundDim,
    PresentMode,
    MenuVsync,
    HalfResolution,
    FpsCap,
    ShowFps,
    Animations,
    Splash,
    CheckUpdates,
    UpdateChannel,
    Update,
    Fullscreen,
    DiscordPresence,
    Skin,
    SkinFolder,
}

pub const ITEMS: [(&str, Item); 43] = [
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::Rate),
    ("GAMEPLAY", Item::RateKeepPitch),
    ("GAMEPLAY", Item::ResumeDelay),
    ("GAMEPLAY", Item::AntiMash),
    ("GAMEPLAY", Item::ShowConverts),
    ("GAMEPLAY", Item::HiddenMaps),
    ("PLAYFIELD", Item::PlayfieldArea),
    ("PLAYFIELD", Item::PlayfieldPreset),
    ("PLAYFIELD", Item::PlayfieldReset),
    ("PLAYFIELD", Item::ShowPlayfieldSize),
    ("PLAYFIELD", Item::BackgroundDim),
    ("PLAYFIELD", Item::KiaiEffects),
    ("INPUT", Item::Key(0)),
    ("INPUT", Item::Key(1)),
    ("INPUT", Item::Key(2)),
    ("INPUT", Item::Key(3)),
    ("INPUT", Item::PresetKey(0)),
    ("INPUT", Item::PresetKey(1)),
    ("INPUT", Item::TouchLayout),
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
    ("VIDEO", Item::HalfResolution),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
    ("VIDEO", Item::Animations),
    ("VIDEO", Item::DiscordPresence),
    ("VIDEO", Item::Splash),
    ("SKIN", Item::Skin),
    ("SKIN", Item::SkinFolder),
    ("UPDATES", Item::UpdateChannel),
    ("UPDATES", Item::Update),
    ("UPDATES", Item::CheckUpdates),
];

/// The sections in order, with a one-line summary for the sidebar.
pub const SECTIONS: [(&str, &str); 8] = [
    ("GAMEPLAY", "speed, rate, resume"),
    ("PLAYFIELD", "size, position"),
    ("INPUT", "drum keys, touch layout"),
    ("OFFSET", "audio offset, calibration"),
    ("AUDIO", "volume, latency"),
    ("VIDEO", "frame pacing, display, Discord"),
    ("SKIN", "notes, hitsounds"),
    ("UPDATES", "new versions"),
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
            Item::Rate => "Speed (rate)",
            Item::RateKeepPitch => "Rate keeps pitch",
            Item::Offset => "Audio offset",
            Item::Calibrate => "Offset calibration",
            Item::CalibrationBpm => "Calibration tempo",
            Item::LiveOffset => "Live offset suggestion",
            Item::LiveOffsetUr => "Scale suggestion to UR",
            Item::ResumeDelay => "Resume delay",
            Item::AntiMash => "Early / wrong hits miss",
            Item::Key(i) => KEY_LABELS[i],
            Item::PresetKey(0) => "Previous preset key",
            Item::PresetKey(_) => "Next preset key",
            Item::TouchLayout => "Touch layout",
            Item::ShowPlayfieldSize => "Show playfield size",
            Item::KiaiEffects => "Kiai effects",
            Item::ShowConverts => "Show osu!standard converts",
            Item::HiddenMaps => "Hidden maps",
            Item::MasterVolume => "Master volume",
            Item::MusicVolume => "Music volume",
            Item::FxVolume => "Hitsound volume",
            Item::AudioBuffer => "Audio buffer",
            Item::BackgroundDim => "Background dim",
            Item::PresentMode => "Present mode (gameplay)",
            Item::MenuVsync => "Menus use vsync",
            Item::HalfResolution => "Half resolution",
            Item::FpsCap => "FPS cap",
            Item::ShowFps => "FPS overlay",
            Item::Animations => "UI animations",
            Item::Splash => "Startup splash",
            Item::UpdateChannel => "Build channel",
            Item::Update => "Update",
            Item::CheckUpdates => "Check at startup",
            Item::Fullscreen => "Fullscreen",
            Item::DiscordPresence => "Discord status",
            Item::Skin => "Skin",
            Item::SkinFolder => "Skins folder",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Item::ScrollSpeed => "How fast notes travel. Shortcut: F3 / F4.",
            Item::PlayfieldArea => "Enter: edit where gameplay is drawn. In game: drag to move, drag edges to resize, wheel to scale.",
            Item::PlayfieldReset => "Enter: make gameplay fill the whole screen again.",
            Item::PlayfieldPreset => "Enter: pick from a list (grouped by aspect ratio). Left/Right: previous/next.",
            Item::Rate => "Song speed like DT (1.50x) / HT (0.75x). Song select: [ and ] (Shift = 0.25x steps).",
            Item::RateKeepPitch => "On: music sounds normal, just faster (DT). Off: pitch rises with speed (nightcore).",
            Item::Offset => "For every map; raise it if you hit late. In a song, - / + (and O) set that map's own offset on top.",
            Item::Calibrate => "Enter: play a click track with notes exactly on the beat; it measures and sets your offset.",
            Item::CalibrationBpm => "Tempo of the calibration click track. Pick one close to the maps you play.",
            Item::LiveOffset => "While playing, suggest an offset from your recent hits; press O to apply it.",
            Item::LiveOffsetUr => "On: more hits and a +- margin when your UR is high. Off: plain average of the last 40 hits.",
            Item::ResumeDelay => "Countdown before the song continues after the pause menu (0 = at once).",
            Item::AntiMash => "osu!'s mash penalty: a wrong-colour or too-early hit misses the note. Off: such hits are ignored.",
            Item::Key(_) => "Enter, then press the new key. Esc cancels.",
            Item::PresetKey(_) => "Steps through playfield presets anywhere (also in game). Enter, then press a key.",
            Item::TouchLayout => "Touch screens: the screen is four columns, left to right (K = rim / kat, D = centre / don).",
            Item::ShowPlayfieldSize => "Show the playfield's resolution in the bottom-right corner during play.",
            Item::ShowConverts => "Also list standard maps, converted to taiko like osu! does.",
            Item::HiddenMaps => "Right-click a song to hide it here (files stay). Enter lists them all again.",
            Item::KiaiEffects => "During kiai time, tint the lane and pulse the hit target with the beat.",
            Item::MasterVolume => "Scales music and hitsounds. Anywhere: hold Alt and scroll the mouse wheel.",
            Item::MusicVolume | Item::FxVolume => "Left / Right to adjust.",
            Item::AudioBuffer => "Smaller = less audio latency; raise it if you hear crackling.",
            Item::BackgroundDim => "How dark the map background is during play. 100% hides it.",
            Item::PresentMode => "Used during songs. vsync is smoothest on Wayland; mailbox/immediate add judder.",
            Item::MenuVsync => "Song select (and its autoplay preview), settings and results always use vsync.",
            Item::HalfResolution => "Draw at half resolution, scaled up exactly 2x: chunky, crisp pixels and a quarter of the GPU work. For high-DPI screens.",
            Item::FpsCap => "mailbox/immediate only. A few times your refresh rate is plenty.",
            Item::ShowFps => "Frame rate and frame time in the corner. Shortcut: F10.",
            Item::Animations => "Menu motion: screen fades, sliding highlights, opening lists, results count-up. Gameplay is unaffected.",
            Item::Splash => "The animated intro when taiko-rs starts (any key or click skips it).",
            Item::UpdateChannel => "dev: test builds that haven't reached everyone yet (may break). Switch, then Update installs that channel's build.",
            Item::Update => "Enter: look for a newer build, or install it (Ctrl+U in any menu). taiko-rs restarts into it.",
            Item::CheckUpdates => "Look for a newer build each time taiko-rs starts; a badge top right says when there is one.",
            Item::Fullscreen => "Shortcut: F11.",
            Item::DiscordPresence => "Show the map you're playing, your lobby or who you spectate in your Discord status.",
            Item::Skin => "Enter opens the list. osu! skins from osu!'s or taiko-rs's Skins folder.",
            Item::SkinFolder => "Enter: open taiko-rs's Skins folder. Drop .osk files there, then pick them above.",
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
    /// Open skin list.
    pub skins: Option<SkinPicker>,
    /// The section last drawn and the animated highlight position in it.
    scroll: Cell<f32>,
    highlight: Cell<f32>,
}

/// The playfield preset dropdown: a cursor over the flattened list, plus where each row was
/// drawn (virtual units) so the mouse can hover and click them. The first group is the player's
/// own saved sizes, which can be added, edited, deleted and reordered.
#[derive(Default)]
pub struct Dropdown {
    pub cursor: usize,
    scroll: f32,
    pub rows: Vec<(usize, [f32; 4])>,
    pub bounds: [f32; 4],
    /// Typing a size: for a new saved one (None) or saved one `i`, and the text so far.
    pub editing: Option<(Option<usize>, String)>,
    /// Buttons drawn last frame.
    pub buttons: Vec<(PresetAction, [f32; 4])>,
}

/// What the dropdown's buttons (and their keys) do to the saved sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetAction {
    /// Save the playfield's current size.
    SaveCurrent,
    /// Type a new size.
    Add,
    Edit,
    Delete,
    Up,
    Down,
}

/// Name of the saved sizes' group.
pub const SAVED: &str = "Saved";

impl Dropdown {
    pub fn new(cursor: usize) -> Dropdown {
        Dropdown { cursor, ..Default::default() }
    }

    /// Index of the row under (`x`, `y`) in virtual units.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows.iter().find(|(_, r)| inside(r, x, y)).map(|(i, _)| *i)
    }

    pub fn button_at(&self, x: f32, y: f32) -> Option<PresetAction> {
        self.buttons.iter().find(|(_, r)| inside(r, x, y)).map(|(a, _)| *a)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        inside(&self.bounds, x, y)
    }

    fn button(&mut self, c: &mut Canvas, x: f32, y: f32, label: &str, action: PresetAction) -> f32 {
        let w = Canvas::text_width(6.0, label) + 10.0;
        let hover = c.hovered(x, y, w, 12.0);
        c.round_rect(x, y, w, 12.0, 3.0, rgba(if hover { 0x4a4a64 } else { 0x2a2a3a }, 1.0), rgba(0x5a5a74, 1.0), 1.0);
        c.text(x + 5.0, y + 3.0, 6.0, rgba(0xffffff, 1.0), label);
        self.buttons.push((action, [x, y, w, 12.0]));
        w
    }

    /// Draws the list centred on screen. `groups`: (aspect name, resolutions), the saved sizes
    /// first when there are any; `saved`: how many; `current`: the preset matching the playfield
    /// now; `playfield`: its size in pixels.
    pub fn draw(
        &mut self,
        c: &mut Canvas,
        groups: &[(&str, Vec<(u32, u32)>)],
        saved: usize,
        current: Option<(u32, u32)>,
        native: (u32, u32),
        playfield: (u32, u32),
    ) {
        const ROW: f32 = 15.0;
        const HEAD: f32 = 16.0;
        enum Line<'a> {
            Head(&'a str),
            Row(usize, u32, u32),
            /// No saved sizes yet.
            Hint,
            /// Typing a new saved size.
            Input,
        }
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.55));
        self.buttons.clear();

        // Lay out, then scroll so the cursor row (or the size being typed) stays visible.
        let adding = matches!(self.editing, Some((None, _)));
        let mut layout: Vec<(Line, f32)> = Vec::new();
        let mut y = 0.0;
        if saved == 0 {
            layout.push((Line::Head(SAVED), y));
            layout.push((if adding { Line::Input } else { Line::Hint }, y + HEAD));
            y += HEAD + ROW;
        }
        let mut index = 0;
        for (name, list) in groups {
            layout.push((Line::Head(name), y));
            y += HEAD;
            for &(rw, rh) in list {
                layout.push((Line::Row(index, rw, rh), y));
                index += 1;
                y += ROW;
            }
            // A new saved size is typed below the others.
            if adding && saved > 0 && index == saved {
                layout.push((Line::Input, y));
                y += ROW;
            }
        }

        let (bw, max_h) = (360.0, 390.0);
        let bh = (y + 8.0 + 78.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        self.bounds = [bx, by, bw, bh];
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
        let title_w = c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), "Playfield preset");
        let screen = format!("screen {}x{}", native.0, native.1);
        c.text_right(bx + bw - 14.0, by + 13.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, bw - 28.0 - title_w - 10.0, &screen));
        let save_label = format!("+ Save current {}x{}", playfield.0, playfield.1);
        let bw0 = self.button(c, bx + 14.0, by + 28.0, &save_label, PresetAction::SaveCurrent);
        self.button(c, bx + 20.0 + bw0, by + 28.0, "+ Add size", PresetAction::Add);

        let (top, view) = (by + 48.0, bh - 78.0);
        let focus = layout.iter().find(|(l, _)| match (l, &self.editing) {
            (Line::Input, _) => true,
            (Line::Row(i, ..), Some((Some(e), _))) => i == e,
            (Line::Row(i, ..), None) => *i == self.cursor,
            _ => false,
        });
        let focus_y = focus.map(|l| l.1).unwrap_or(0.0);
        self.scroll = (focus_y - view / 2.0).clamp(0.0, (y - view).max(0.0));
        self.rows.clear();
        let text_input = |c: &mut Canvas, ry: f32, text: &str| {
            c.round_rect(bx + 8.0, ry, bw - 16.0, ROW - 2.0, 4.0, rgba(0x1d1d27, 1.0), rgba(0xFFD34D, 1.0), 1.0);
            c.text(bx + 22.0, ry + 3.5, 7.0, rgba(0xFFD34D, 1.0), &format!("{text}_"));
            c.text_right(bx + bw - 22.0, ry + 3.5, 6.0, rgba(0x8a8aa0, 1.0), "width x height");
        };
        for (line, ly) in layout {
            let ry = top + ly - self.scroll;
            if ry < top - 1.0 || ry + ROW > top + view + 1.0 {
                continue;
            }
            match line {
                Line::Head(name) => {
                    c.text(bx + 14.0, ry + 5.0, 7.0, rgba(0xEB452C, 1.0), name);
                }
                Line::Hint => {
                    c.text(bx + 22.0, ry + 3.5, 6.0, rgba(0x6f6f86, 1.0), "none yet: S saves the playfield's size, A adds one");
                }
                Line::Input => {
                    if let Some((_, text)) = &self.editing {
                        text_input(c, ry, text);
                    }
                }
                Line::Row(i, rw, rh) => {
                    if let Some((Some(e), text)) = &self.editing {
                        if *e == i {
                            text_input(c, ry, text);
                            continue;
                        }
                    }
                    let selected = i == self.cursor && self.editing.is_none();
                    if selected {
                        c.round_rect(bx + 8.0, ry, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(0xEB452C, 1.0), 1.0);
                    }
                    self.rows.push((i, [bx + 8.0, ry, bw - 16.0, ROW - 2.0]));
                    c.text(bx + 22.0, ry + 3.5, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &format!("{rw} x {rh}"));
                    let fits = rw <= native.0 && rh <= native.1;
                    let mut right = bx + bw - 22.0;
                    if selected && i < saved {
                        // Buttons for the saved size, right to left.
                        for (label, action) in [("del", PresetAction::Delete), ("edit", PresetAction::Edit), ("down", PresetAction::Down), ("up", PresetAction::Up)] {
                            let lw = Canvas::text_width(6.0, label) + 10.0;
                            right -= lw;
                            self.button(c, right, ry + 0.5, label, action);
                            right -= 4.0;
                        }
                        right -= 6.0;
                    }
                    let (tag, col) = if !fits {
                        ("too big", 0x8a8aa0)
                    } else if Some((rw, rh)) == current {
                        ("current", 0x86E05A)
                    } else {
                        ("", 0)
                    };
                    c.text_right(right, ry + 3.5, 7.0, rgba(col, 1.0), tag);
                }
            }
        }
        let (line1, line2) = if self.editing.is_some() {
            ("type a size like 1280x720   Enter save   Esc cancel", "")
        } else {
            ("Up/Down or mouse choose   Enter/click apply   Esc close", "S save current   A add   E edit   Del delete   Shift+Up/Down move")
        };
        c.text_centered(w / 2.0, by + bh - 20.0, 6.0, rgba(0x6f6f86, 1.0), line1);
        c.text_centered(w / 2.0, by + bh - 10.0, 6.0, rgba(0x6f6f86, 1.0), line2);
    }
}

/// Every installed skin in one list, so picking one isn't stepping through them all.
pub struct SkinPicker {
    /// Skin names, "" (built-in) first.
    pub names: Vec<String>,
    pub cursor: usize,
}

impl SkinPicker {
    /// Opens on the skin in use.
    pub fn new(names: Vec<String>, current: &str) -> SkinPicker {
        let cursor = names.iter().position(|n| n == current).unwrap_or(0);
        SkinPicker { names, cursor }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.cursor = (self.cursor as i32 + delta).clamp(0, self.names.len() as i32 - 1) as usize;
    }

    pub fn draw(&self, c: &mut Canvas, current: &str) {
        const ROW: f32 = 18.0;
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.55));
        let n = self.names.len();
        let (bw, max_h) = ((w - 48.0).min(420.0), 380.0);
        let bh = (n as f32 * ROW + 56.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
        c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), "Skin");
        c.text_right(bx + bw - 14.0, by + 13.0, 7.0, rgba(0x6f6f86, 1.0), &format!("{} installed", n - 1));
        let (top, view) = (by + 32.0, bh - 50.0);
        let scroll = (self.cursor as f32 * ROW - view / 2.0).clamp(0.0, (n as f32 * ROW - view).max(0.0));
        for (i, name) in self.names.iter().enumerate() {
            let y = top + i as f32 * ROW - scroll;
            if y < top - 1.0 || y + ROW > top + view + 1.0 {
                continue;
            }
            let selected = i == self.cursor;
            let hover = c.hovered(bx + 8.0, y, bw - 16.0, ROW - 2.0);
            if selected || hover {
                c.round_rect(bx + 8.0, y, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(if selected { 0xEB452C } else { 0x3a3a4e }, 1.0), 1.0);
            }
            c.hit(bx + 8.0, y, bw - 16.0, ROW, Hit::SkinRow(i));
            let label = if name.is_empty() { "built-in" } else { name.as_str() };
            let room = bw - 40.0 - if name == current { Canvas::text_width(7.0, "current") + 8.0 } else { 0.0 };
            c.text(bx + 20.0, y + 5.0, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &Canvas::fit(7.0, room, label));
            if name == current {
                c.text_right(bx + bw - 20.0, y + 5.0, 7.0, rgba(0x86E05A, 1.0), "current");
            }
        }
        c.text_centered(w / 2.0, by + bh - 11.0, 6.0, rgba(0x6f6f86, 1.0), "Up/Down or mouse   Enter/click use   Esc close");
    }
}

fn inside(r: &[f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]
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
            c.text(x0 + 12.0, y + 21.0, 6.0, rgba(0x8a8aa0, 1.0), &Canvas::fit(6.0, side_w - 20.0, about));
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
            // The value gets what the label leaves, and is cut short past that.
            let room = col - 36.0 - Canvas::text_width(8.0, item.label());
            let v = if selected && self.binding.is_some() {
                "press a key...".to_string()
            } else if selected && !matches!(item, Item::Key(_) | Item::PresetKey(_)) {
                format!("<  {}  >", Canvas::fit(8.0, room - Canvas::text_width(8.0, "<    >"), &value(*item)))
            } else {
                Canvas::fit(8.0, room, &value(*item))
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
        // Two lines, spaced for however tall the text comes out (taller on a text grid).
        let glyph_h = Canvas::text_height(7.0);
        let line_h = glyph_h + 4.0;
        c.round_rect(cx, help_y, col, line_h * 2.0 + 8.0, 6.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
        let help = self.item().help();
        let max = ((col - 20.0) / Canvas::char_width(7.0)) as usize;
        let (line1, line2) = split_help(help, max);
        // `text` centres taller glyphs on the line; this puts their top 6 below the box's.
        let first = help_y + 6.0 + (glyph_h - 7.0) / 2.0;
        c.text(cx + 10.0, first, 7.0, rgba(0x9a9ab0, 1.0), &line1);
        c.text(cx + 10.0, first + line_h, 7.0, rgba(0x9a9ab0, 1.0), &line2);
        c.text_centered(w / 2.0, 462.0, 8.0, rgba(0x55556a, 1.0), &Canvas::fit_hints(8.0, w - 48.0, footer));
    }
}

/// Wraps help text onto two lines of at most `max` characters.
fn split_help(text: &str, max: usize) -> (String, String) {
    if text.chars().count() <= max {
        return (text.to_string(), String::new());
    }
    let cut = text.char_indices().take_while(|(i, _)| *i <= max).filter(|(_, ch)| *ch == ' ').map(|(i, _)| i).last().unwrap_or(max.min(text.len()));
    let rest = text[cut..].trim_start();
    let rest = if rest.chars().count() > max { format!("{}...", rest.chars().take(max.saturating_sub(3)).collect::<String>().trim_end()) } else { rest.to_string() };
    (text[..cut].to_string(), rest)
}

/// Top bar shared by the song list, settings and multiplayer: logo plus the three tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    thread_local! {
        /// Where the active tab's highlight is (x, width), so it slides between tabs.
        static PILL: Cell<(f32, f32)> = const { Cell::new((f32::NAN, 0.0)) };
    }
    crate::logo::draw(c, 22.0, 18.0, 26.0);
    // The name like the wordmark: letters alternating white and red, "-rs" red.
    let mut tx = 54.0;
    for (i, ch) in "taiko-rs".chars().enumerate() {
        let s = ch.to_string();
        let col = match ch {
            '-' => 0x6f6f86,
            'r' | 's' => 0xEB452C,
            _ if i % 2 == 0 => 0xffffff,
            _ => 0xEB452C,
        };
        tx += c.text(tx, 24.0, 16.0, rgba(col, 1.0), &s);
    }
    let x0 = tx + 24.0;
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
    // Beside the tabs when there's room, else in small print under them (narrow screens),
    // clear of the friends chip in the top-right corner.
    let right = c.width - 24.0;
    let room = right - (x + 4.0 + Canvas::text_width(8.0, "Tab") + 16.0);
    if Canvas::text_width(8.0, right_text) <= room {
        c.text_right(right, 28.0, 8.0, rgba(0x9a9ab0, 1.0), right_text);
    } else {
        c.text(x0 + 2.0, 45.0, 6.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(6.0, c.width - 110.0 - x0, right_text));
    }
}
