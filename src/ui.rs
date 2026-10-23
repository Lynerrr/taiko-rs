//! Song select and results screens.

use winit::keyboard::KeyCode;

use crate::game::{Stats, Windows};
use crate::library::Entry;
use crate::render::{rgba, Canvas};

pub struct Select {
    pub filter: String,
    /// Files read so far while the library is still being scanned.
    pub scanning: Option<usize>,
    /// Speed modifier shown in the header ("" at 1.0x).
    pub rate_label: String,
    pub show_converts: bool,
    visible: Vec<usize>,
    cursor: usize,
    scroll: f32,
    /// Songs visited with F2, so Shift+F2 can step back (library indices).
    random_history: Vec<usize>,
    rng: u64,
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
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9e37_79b9_7f4a_7c15);
        let mut s = Select {
            filter: String::new(),
            scanning: None,
            rate_label: String::new(),
            show_converts: !has_taiko,
            visible: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            random_history: Vec::new(),
            rng: seed | 1,
        };
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

    /// Mouse wheel: move the selection by `n` rows (negative = up).
    pub fn move_by(&mut self, n: i32) {
        let last = self.visible.len().saturating_sub(1) as i32;
        self.cursor = (self.cursor as i32 + n).clamp(0, last.max(0)) as usize;
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

    /// The selected difficulty (library index).
    pub fn selected(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    /// Width of the song list; the preview panel takes the rest.
    pub fn list_width(screen_w: f32) -> f32 {
        (screen_w * 0.52).max(300.0).min(screen_w - 48.0)
    }

    /// Songs (beatmap sets) as ranges of `visible`: consecutive difficulties sharing a folder.
    fn groups(&self, library: &[Entry]) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for i in 1..=self.visible.len() {
            let split = i == self.visible.len() || library[self.visible[i]].path.parent() != library[self.visible[i - 1]].path.parent();
            if split {
                out.push(start..i);
                start = i;
            }
        }
        out
    }

    /// F2: jump to a random song among those listed (search and filters apply), keeping the
    /// position within the difficulty list where possible.
    pub fn random(&mut self, library: &[Entry]) {
        let groups = self.groups(library);
        if groups.len() < 2 {
            return;
        }
        let current = groups.iter().position(|g| g.contains(&self.cursor));
        let offset = current.map(|g| self.cursor - groups[g].start).unwrap_or(0);
        // xorshift64*: plenty for picking songs, no dependency.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let r = self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let mut pick = (r % groups.len() as u64) as usize;
        if Some(pick) == current {
            pick = (pick + 1) % groups.len();
        }
        if let Some(&here) = self.visible.get(self.cursor) {
            self.random_history.push(here);
        }
        let g = &groups[pick];
        self.cursor = g.start + offset.min(g.len() - 1);
    }

    /// Shift+F2: back to the song before the last random jump.
    pub fn random_back(&mut self) {
        while let Some(prev) = self.random_history.pop() {
            if let Some(pos) = self.visible.iter().position(|&v| v == prev) {
                self.cursor = pos;
                return;
            }
        }
    }

    /// Library indices of the selected song's difficulties (for star rating requests).
    /// Orders the selected song's difficulties by star rating once all of them are known, like
    /// osu!, keeping the same difficulty selected. (Other songs are collapsed, so their order
    /// doesn't show until they are opened.)
    pub fn sort_selected_set(&mut self, library: &[Entry], stars: &dyn Fn(usize) -> Option<f32>) {
        let Some(g) = self.groups(library).into_iter().find(|g| g.contains(&self.cursor)) else { return };
        let Some(keys) = self.visible[g.clone()].iter().map(|&i| stars(i)).collect::<Option<Vec<f32>>>() else { return };
        if keys.windows(2).all(|w| w[0] <= w[1]) {
            return;
        }
        let current = self.visible[self.cursor];
        let mut set: Vec<(f32, usize)> = keys.into_iter().zip(self.visible[g.clone()].iter().copied()).collect();
        set.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (slot, (_, i)) in g.clone().zip(set) {
            self.visible[slot] = i;
        }
        if let Some(pos) = self.visible.iter().position(|&v| v == current) {
            self.cursor = pos;
        }
    }

    pub fn selected_set(&self, library: &[Entry]) -> Vec<usize> {
        self.groups(library).into_iter().find(|g| g.contains(&self.cursor)).map(|g| g.map(|i| self.visible[i]).collect()).unwrap_or_default()
    }

    /// `stars` looks up a difficulty's star rating (library index) if it has been calculated.
    pub fn draw(&mut self, c: &mut Canvas, library: &[Entry], dt_ms: f64, status: &str, stars: &dyn Fn(usize) -> Option<f32>) {
        let w = c.width;
        let mode = if self.show_converts { "taiko + converts" } else { "taiko only" };
        let groups = self.groups(library);
        crate::settings::tab_bar(c, 0, &format!("{} songs, {} maps  |  {mode}{}", groups.len(), self.visible.len(), self.rate_label));

        let lw = Self::list_width(w);
        let search = if self.filter.is_empty() { "type to search...".to_string() } else { format!("search: {}_", self.filter) };
        c.round_rect(24.0, 54.0, lw, 24.0, 6.0, rgba(0x23232f, 0.9), rgba(0x3a3a4e, 1.0), 1.0);
        let search_col = if self.filter.is_empty() { 0x6f6f86 } else { 0xffffff };
        c.text(34.0, 61.0, 10.0, rgba(search_col, 1.0), &search);

        if let Some(n) = self.scanning {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "Scanning your Songs folder...");
            c.text_centered(24.0 + lw / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), &format!("{n} files read"));
        } else if self.visible.is_empty() {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            c.text_centered(24.0 + lw / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), "Run: taiko-rs <Songs folder | map.osz | map.osu>");
        }

        // Rows: one per song, plus the selected song's difficulties under it.
        const SONG_H: f32 = 34.0;
        const DIFF_H: f32 = 22.0;
        enum Row {
            Song(std::ops::Range<usize>),
            Diff(usize),
        }
        let mut rows: Vec<(Row, f32, f32)> = Vec::new();
        let mut y = 0.0;
        let mut cursor_y = 0.0;
        for g in &groups {
            rows.push((Row::Song(g.clone()), y, SONG_H));
            y += SONG_H;
            if g.contains(&self.cursor) {
                for i in g.clone() {
                    if i == self.cursor {
                        cursor_y = y;
                    }
                    rows.push((Row::Diff(i), y, DIFF_H));
                    y += DIFF_H;
                }
            }
        }
        let (top, bottom) = (86.0, 440.0);
        let view = bottom - top;
        let target = (cursor_y - view * 0.4).clamp(0.0, (y - view).max(0.0));
        let k = 1.0 - (-dt_ms as f32 / 60.0).exp();
        self.scroll += (target - self.scroll) * k;
        let max_chars = |size: f32, room: f32| (room / (size / 7.0 * 6.0)).max(4.0) as usize;
        let fit = |s: String, n: usize| if s.chars().count() > n { s.chars().take(n.saturating_sub(3)).collect::<String>() + "..." } else { s };
        for (row, ry, rh) in &rows {
            let y = top + ry - self.scroll;
            if y + rh < top || y > bottom {
                continue;
            }
            match row {
                Row::Song(g) => {
                    let e = &library[self.visible[g.start]];
                    let open = g.contains(&self.cursor);
                    let (bg, border) = if open { (0x2b2b3d, 0x5a5a78) } else { (0x1d1d27, 0x2a2a38) };
                    c.round_rect(24.0, y, lw, rh - 4.0, 7.0, rgba(bg, 0.88), rgba(border, 1.0), 1.0);
                    let any_taiko = g.clone().any(|i| library[self.visible[i]].meta.mode == 1);
                    let tag = if any_taiko { 0xEB452C } else { 0x438EAD };
                    c.round_rect(32.0, y + 8.0, 5.0, rh - 20.0, 2.5, rgba(tag, 1.0), rgba(tag, 1.0), 0.0);
                    c.text(44.0, y + 6.0, 10.0, rgba(0xffffff, 1.0), &fit(e.meta.title.clone(), max_chars(10.0, lw - 110.0)));
                    let sub = format!("{}  |  mapped by {}", e.meta.artist, e.meta.creator);
                    c.text(44.0, y + 20.0, 7.0, rgba(0x9a9ab0, 1.0), &fit(sub, max_chars(7.0, lw - 110.0)));
                    let n = g.len();
                    c.text_right(24.0 + lw - 12.0, y + 11.0, 8.0, rgba(0x8a8aa0, 1.0), &format!("{n} diff{}", if n == 1 { "" } else { "s" }));
                }
                Row::Diff(i) => {
                    let e = &library[self.visible[*i]];
                    let selected = *i == self.cursor;
                    let (bg, border) = if selected { (0x34344a, 0xEB452C) } else { (0x191921, 0x2a2a38) };
                    c.round_rect(44.0, y, lw - 20.0, rh - 3.0, 5.0, rgba(bg, 0.92), rgba(border, 1.0), if selected { 1.5 } else { 1.0 });
                    let col = if selected { 0xffffff } else { 0xc8c8d8 };
                    let convert = if e.meta.mode == 0 { "  (convert)" } else { "" };
                    let pill = crate::stars::draw_pill(c, 24.0 + lw - 10.0, y + 6.0, 7.0, stars(self.visible[*i]));
                    c.text_right(24.0 + lw - 18.0 - pill, y + 6.0, 7.0, rgba(0x8a8aa0, 1.0), &format!("OD {:.1}", e.meta.od));
                    c.text(56.0, y + 6.0, 8.0, rgba(col, 1.0), &fit(format!("{}{convert}", e.meta.version), max_chars(8.0, lw - 170.0)));
                }
            }
        }

        c.text(24.0, 452.0, 7.0, rgba(0x6f6f86, 1.0), "Enter play  Up/Down difficulty  Left/Right song  F2 random  [ ] rate  Ctrl+Left/Right seek  Tab settings  -/= offset  Esc quit");
        c.text(24.0, 466.0, 7.0, rgba(0x55556a, 1.0), status);
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
