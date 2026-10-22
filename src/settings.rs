//! Settings tab: the list of options and how it is drawn. Values are read and changed by the
//! caller (it owns the config, audio engine and window).

use crate::render::{rgba, Canvas};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    ScrollSpeed,
    Offset,
    Key(usize),
    ShowConverts,
    MusicVolume,
    FxVolume,
    AudioBuffer,
    PresentMode,
    FrameDelay,
    FpsCap,
    ShowFps,
    Fullscreen,
}

pub const ITEMS: [(&str, Item); 15] = [
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::Offset),
    ("GAMEPLAY", Item::Key(0)),
    ("GAMEPLAY", Item::Key(1)),
    ("GAMEPLAY", Item::Key(2)),
    ("GAMEPLAY", Item::Key(3)),
    ("GAMEPLAY", Item::ShowConverts),
    ("AUDIO", Item::MusicVolume),
    ("AUDIO", Item::FxVolume),
    ("AUDIO", Item::AudioBuffer),
    ("VIDEO", Item::PresentMode),
    ("VIDEO", Item::FrameDelay),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
];

const KEY_LABELS: [&str; 4] = ["Left rim (kat)", "Left centre (don)", "Right centre (don)", "Right rim (kat)"];

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::ScrollSpeed => "Scroll speed",
            Item::Offset => "Audio offset",
            Item::Key(i) => KEY_LABELS[i],
            Item::ShowConverts => "Show osu!standard converts",
            Item::MusicVolume => "Music volume",
            Item::FxVolume => "Hitsound volume",
            Item::AudioBuffer => "Audio buffer",
            Item::PresentMode => "Present mode",
            Item::FrameDelay => "Frame delay",
            Item::FpsCap => "FPS cap",
            Item::ShowFps => "FPS overlay",
            Item::Fullscreen => "Fullscreen",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Item::ScrollSpeed => "How fast notes travel. Shortcut: F3 / F4.",
            Item::Offset => "Raise it if you hit late on average (results screen tells you). Shift = 5 ms steps.",
            Item::Key(_) => "Enter, then press the new key. Esc cancels.",
            Item::ShowConverts => "Also list standard maps, converted to taiko like osu! does.",
            Item::MusicVolume | Item::FxVolume => "Left / Right to adjust.",
            Item::AudioBuffer => "Smaller = less audio latency; raise it if you hear crackling.",
            Item::PresentMode => "vsync is smoothest on Wayland; mailbox/immediate add judder.",
            Item::FrameDelay => "vsync only: draw later in each refresh for less lag. Near/over one refresh (16.7 ms at 60 Hz) frames miss the deadline.",
            Item::FpsCap => "mailbox/immediate only. A few times your refresh rate is plenty.",
            Item::ShowFps => "Frame rate and frame time in the corner. Shortcut: F10.",
            Item::Fullscreen => "Shortcut: F11.",
        }
    }
}

#[derive(Default)]
pub struct SettingsUi {
    pub cursor: usize,
    /// Waiting for a key press to bind drum key `n`.
    pub binding: Option<usize>,
}

impl SettingsUi {
    pub fn item(&self) -> Item {
        ITEMS[self.cursor].1
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.cursor = (self.cursor as i32 + delta).clamp(0, ITEMS.len() as i32 - 1) as usize;
    }

    /// `value` gives the current display value of each item.
    pub fn draw(&self, c: &mut Canvas, value: &dyn Fn(Item) -> String, footer: &str) {
        let w = c.width;
        let col = (w - 48.0).min(640.0);
        let x0 = (w - col) / 2.0;
        let mut y = 58.0;
        let mut section = "";
        for (i, (sec, item)) in ITEMS.iter().enumerate() {
            if *sec != section {
                section = sec;
                y += 4.0;
                c.text(x0, y, 8.0, rgba(0xEB452C, 1.0), sec);
                y += 13.0;
            }
            let selected = i == self.cursor;
            let (bg, border) = if selected { (0x34344a, 0xEB452C) } else { (0x1d1d27, 0x2a2a38) };
            c.round_rect(x0, y, col, 18.0, 5.0, rgba(bg, 1.0), rgba(border, 1.0), if selected { 1.5 } else { 1.0 });
            c.text(x0 + 10.0, y + 5.5, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), item.label());
            let v = if selected && self.binding.is_some() {
                "press a key...".to_string()
            } else if selected && !matches!(item, Item::Key(_)) {
                format!("<  {}  >", value(*item))
            } else {
                value(*item)
            };
            let v_col = if selected && self.binding.is_some() { 0xFFD34D } else { 0xffffff };
            c.text_right(x0 + col - 10.0, y + 5.5, 7.0, rgba(v_col, 1.0), &v);
            y += 21.0;
        }
        c.text_centered(w / 2.0, 446.0, 8.0, rgba(0x9a9ab0, 1.0), self.item().help());
        c.text_centered(w / 2.0, 462.0, 8.0, rgba(0x55556a, 1.0), footer);
    }
}

/// Top bar shared by the song list and settings: logo plus SONGS / SETTINGS tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    c.text(24.0, 24.0, 16.0, rgba(0xffffff, 1.0), "taiko-rs");
    let mut x = 24.0 + Canvas::text_width(16.0, "taiko-rs") + 24.0;
    for (i, name) in ["SONGS", "SETTINGS"].iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        let on = i == active;
        c.round_rect(x, 20.0, tw + 20.0, 22.0, 6.0, rgba(if on { 0xEB452C } else { 0x23232f }, 1.0), rgba(if on { 0xEB452C } else { 0x3a3a4e }, 1.0), 1.0);
        c.text(x + 10.0, 27.0, 9.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), name);
        x += tw + 28.0;
    }
    c.text(x + 4.0, 27.0, 8.0, rgba(0x55556a, 1.0), "Tab");
    c.text_right(c.width - 24.0, 28.0, 8.0, rgba(0x9a9ab0, 1.0), right_text);
}
