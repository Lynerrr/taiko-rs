//! Song select and results screens.

use winit::keyboard::KeyCode;

use crate::game::{Stats, Windows};
use crate::library::Entry;
use crate::render::{rgba, Canvas};

pub struct Select {
    pub filter: String,
    /// Files read so far while the library is still being scanned.
    pub scanning: Option<usize>,
    pub show_converts: bool,
    visible: Vec<usize>,
    cursor: usize,
    scroll: f32,
}

pub enum SelectAction {
    None,
    Play(usize),
    Settings,
    Quit,
}

impl Select {
    pub fn new(library: &[Entry]) -> Select {
        let has_taiko = library.iter().any(|e| e.meta.mode == 1);
        let mut s = Select { filter: String::new(), scanning: None, show_converts: !has_taiko, visible: Vec::new(), cursor: 0, scroll: 0.0 };
        s.refilter(library);
        s
    }

    pub fn refilter(&mut self, library: &[Entry]) {
        let selected = self.visible.get(self.cursor).copied();
        let needle = self.filter.to_lowercase();
        self.visible = library
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_converts || e.meta.mode == 1)
            .filter(|(_, e)| needle.is_empty() || e.label().to_lowercase().contains(&needle) || e.meta.creator.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.cursor = selected.and_then(|s| self.visible.iter().position(|&v| v == s)).unwrap_or(0);
    }

    pub fn select_path(&mut self, library: &[Entry], path: &std::path::Path) {
        if let Some(pos) = self.visible.iter().position(|&i| library[i].path == path) {
            self.cursor = pos;
        }
    }

    pub fn key(&mut self, key: KeyCode, library: &[Entry]) -> SelectAction {
        let n = self.visible.len();
        match key {
            KeyCode::ArrowDown => self.cursor = (self.cursor + 1).min(n.saturating_sub(1)),
            KeyCode::ArrowUp => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::PageDown => self.cursor = (self.cursor + 10).min(n.saturating_sub(1)),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(10),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = n.saturating_sub(1),
            // Jump between beatmap sets.
            KeyCode::ArrowRight | KeyCode::ArrowLeft => {
                if let Some(&cur) = self.visible.get(self.cursor) {
                    let dir = library[cur].path.parent();
                    let fwd = key == KeyCode::ArrowRight;
                    let mut i = self.cursor;
                    loop {
                        if fwd { if i + 1 >= n { break } i += 1 } else { if i == 0 { break } i -= 1 }
                        if library[self.visible[i]].path.parent() != dir {
                            break;
                        }
                    }
                    if !fwd {
                        // Land on the first difficulty of the previous set.
                        let d = library[self.visible[i]].path.parent();
                        while i > 0 && library[self.visible[i - 1]].path.parent() == d {
                            i -= 1;
                        }
                    }
                    self.cursor = i;
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(&i) = self.visible.get(self.cursor) {
                    return SelectAction::Play(i);
                }
            }
            KeyCode::Tab | KeyCode::F1 => return SelectAction::Settings,
            KeyCode::Backspace => {
                self.filter.pop();
                self.refilter(library);
            }
            KeyCode::Escape => {
                if self.filter.is_empty() {
                    return SelectAction::Quit;
                }
                self.filter.clear();
                self.refilter(library);
            }
            _ => {}
        }
        SelectAction::None
    }

    pub fn type_text(&mut self, text: &str, library: &[Entry]) {
        let mut changed = false;
        for ch in text.chars().filter(|c| !c.is_control()) {
            self.filter.push(ch);
            changed = true;
        }
        if changed {
            self.refilter(library);
        }
    }

    pub fn draw(&mut self, c: &mut Canvas, library: &[Entry], dt_ms: f64, status: &str) {
        let w = c.width;
        let mode = if self.show_converts { "taiko + converts" } else { "taiko only" };
        crate::settings::tab_bar(c, 0, &format!("{} maps  |  {mode}", self.visible.len()));

        let search = if self.filter.is_empty() { "type to search...".to_string() } else { format!("search: {}_", self.filter) };
        c.round_rect(24.0, 54.0, w - 48.0, 24.0, 6.0, rgba(0x23232f, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
        let search_col = if self.filter.is_empty() { 0x6f6f86 } else { 0xffffff };
        c.text(34.0, 61.0, 10.0, rgba(search_col, 1.0), &search);

        if let Some(n) = self.scanning {
            c.text_centered(w / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "Scanning your Songs folder...");
            c.text_centered(w / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), &format!("{n} files read"));
        } else if self.visible.is_empty() {
            c.text_centered(w / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            c.text_centered(w / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), "Run: taiko-rs <Songs folder | map.osz | map.osu>");
        }

        let row_h: f32 = 30.0;
        let top = 90.0;
        let bottom = 440.0;
        let rows = ((bottom - top) / row_h).floor();
        // Smoothly scroll the list to keep the cursor centred.
        let target = (self.cursor as f32 - rows / 2.0 + 0.5).clamp(0.0, (self.visible.len() as f32 - rows).max(0.0));
        let k = 1.0 - (-dt_ms as f32 / 60.0).exp();
        self.scroll += (target - self.scroll) * k;
        let first = self.scroll.floor().max(0.0) as usize;
        for (row, &idx) in self.visible.iter().enumerate().skip(first).take(rows as usize + 2) {
            let y = top + (row as f32 - self.scroll) * row_h;
            if y < top - row_h || y > bottom {
                continue;
            }
            let e = &library[idx];
            let selected = row == self.cursor;
            let (bg, border) = if selected { (0x34344a, 0xEB452C) } else { (0x1d1d27, 0x2a2a38) };
            c.round_rect(24.0, y, w - 48.0, row_h - 4.0, 6.0, rgba(bg, 1.0), rgba(border, 1.0), if selected { 2.0 } else { 1.0 });
            let tag_col = if e.meta.mode == 1 { 0xEB452C } else { 0x438EAD };
            c.round_rect(32.0, y + 8.0, 6.0, row_h - 20.0, 3.0, rgba(tag_col, 1.0), rgba(tag_col, 1.0), 0.0);
            let max_chars = ((w - 160.0) / (10.0 / 7.0 * 6.0)) as usize;
            let mut label = e.label();
            if label.chars().count() > max_chars {
                label = label.chars().take(max_chars.saturating_sub(3)).collect::<String>() + "...";
            }
            c.text(46.0, y + 8.0, 10.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &label);
            c.text_right(w - 36.0, y + 8.0, 10.0, rgba(0x8a8aa0, 1.0), &format!("OD {:.1}", e.meta.od));
        }

        c.text(24.0, 452.0, 8.0, rgba(0x6f6f86, 1.0), "Enter play  Up/Down select  Left/Right set  Tab settings  -/= offset  F3/F4 speed  F11 fullscreen  Esc quit");
        c.text(24.0, 466.0, 8.0, rgba(0x55556a, 1.0), status);
    }
}

pub struct Results {
    pub title: String,
    pub stats: Stats,
    pub windows: Windows,
}

impl Results {
    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        let cx = w / 2.0;
        c.text_centered(cx, 40.0, 12.0, rgba(0x9a9ab0, 1.0), &self.title);
        let s = &self.stats;
        let acc = s.accuracy() * 100.0;
        let grade = if s.miss == 0 && s.ok == 0 {
            "SS"
        } else if acc >= 95.0 && s.miss == 0 {
            "S"
        } else if acc >= 90.0 {
            "A"
        } else if acc >= 80.0 {
            "B"
        } else if acc >= 70.0 {
            "C"
        } else {
            "D"
        };
        c.text_centered(cx, 80.0, 70.0, rgba(0xFFD34D, 1.0), grade);
        c.text_centered(cx, 170.0, 28.0, rgba(0xffffff, 1.0), &format!("{acc:.2}%"));
        c.text_centered(cx, 212.0, 16.0, rgba(0xc8c8d8, 1.0), &format!("{:08}", s.score));

        let lines = [
            (0xFFD34D, format!("GREAT  {}", s.great)),
            (0x86E05A, format!("OK     {}", s.ok)),
            (0xFF4B4B, format!("MISS   {}", s.miss)),
            (0xffffff, format!("MAX COMBO  {}", s.max_combo)),
        ];
        for (i, (col, text)) in lines.iter().enumerate() {
            c.text(cx - 170.0, 260.0 + i as f32 * 24.0, 12.0, rgba(*col, 1.0), text);
        }
        let right = [
            format!("UR      {:.1}", s.unstable_rate()),
            format!("MEAN    {:+.1} ms", s.mean_error()),
            format!("DRUMROLL {}", s.roll_ticks),
            format!("BIG x2  {}", s.strong),
        ];
        for (i, text) in right.iter().enumerate() {
            c.text(cx + 30.0, 260.0 + i as f32 * 24.0, 12.0, rgba(0xc8c8d8, 1.0), text);
        }
        let hint = if s.errors.len() >= 20 && s.mean_error().abs() > 5.0 {
            let dir = if s.mean_error() > 0.0 { "late" } else { "early" };
            format!("You hit {:.0} ms {dir} on average; press {} a few times to compensate.", s.mean_error().abs(), if s.mean_error() > 0.0 { "=" } else { "-" })
        } else {
            format!("Windows: great +-{:.0}  ok +-{:.0} ms", self.windows.great, self.windows.ok)
        };
        c.text_centered(cx, 380.0, 9.0, rgba(0x8a8aa0, 1.0), &hint);
        c.text_centered(cx, 430.0, 10.0, rgba(0x6f6f86, 1.0), "Enter: retry    Esc: song select");
    }
}
