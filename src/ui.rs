//! Song select and results screens.

use winit::keyboard::KeyCode;

use crate::game::{Stats, Windows};
use crate::library::Entry;
use crate::render::{progress, rgba, Canvas, Hit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    Artist,
    Title,
    Difficulty,
    Length,
    Bpm,
    Played,
    Added,
}

impl SortMode {
    pub const ALL: [SortMode; 7] = [SortMode::Artist, SortMode::Title, SortMode::Difficulty, SortMode::Length, SortMode::Bpm, SortMode::Played, SortMode::Added];

    pub fn name(self) -> &'static str {
        match self {
            SortMode::Artist => "artist",
            SortMode::Title => "title",
            SortMode::Difficulty => "difficulty",
            SortMode::Length => "length",
            SortMode::Bpm => "bpm",
            SortMode::Played => "recently played",
            SortMode::Added => "date added",
        }
    }

    pub fn parse(s: &str) -> Option<SortMode> {
        SortMode::ALL.into_iter().find(|m| m.name() == s)
    }

    pub fn step(self, dir: i32) -> SortMode {
        let i = SortMode::ALL.iter().position(|&m| m == self).unwrap_or(0) as i32;
        SortMode::ALL[(i + dir).rem_euclid(SortMode::ALL.len() as i32) as usize]
    }

    /// Sorts that need the background difficulty / length / BPM analysis.
    pub fn needs_info(self) -> bool {
        matches!(self, SortMode::Difficulty | SortMode::Length | SortMode::Bpm)
    }
}

pub struct Select {
    pub filter: String,
    pub sort: SortMode,
    /// Star rating / length / BPM per library index, filled in the background.
    pub info: Option<crate::mapinfo::Shared>,
    /// When each map was last played (Unix seconds), from taiko-rs and osu!.
    pub played: std::collections::HashMap<std::path::PathBuf, u64>,
    /// osu! collections: (name, library indices).
    pub collections: Vec<(String, std::collections::HashSet<usize>)>,
    /// The collection shown (None: everything) and the open collection list's cursor.
    pub collection: Option<usize>,
    pub picker: Option<usize>,
    /// Shown in the header while the analysis for the current sort is running.
    pub info_progress: String,
    /// Files read so far while the library is still being scanned.
    pub scanning: Option<usize>,
    /// Speed modifier shown in the header ("" at 1.0x).
    pub rate_label: String,
    pub show_converts: bool,
    visible: Vec<usize>,
    /// Songs (beatmap sets) as ranges of `visible`: consecutive difficulties sharing a folder.
    /// Rebuilt when `visible` changes, not per frame (big libraries have tens of thousands of maps).
    groups: Vec<std::ops::Range<usize>>,
    /// Index into `groups` for each row of `visible`.
    group_of: Vec<usize>,
    cursor: usize,
    scroll: f32,
    anim: ListAnim,
    /// Songs visited with F2, so Shift+F2 can step back (library indices).
    random_history: Vec<usize>,
    rng: u64,
}

/// Song list motion. Each selection change starts one eased transition from wherever things
/// are to where they end up, so the scroll, the newly opened song, the closing one and the
/// selection outline all move together and arrive at the same moment.
#[derive(Default)]
struct ListAnim {
    /// The selection the current transition is heading to (None: not drawn yet).
    cursor: Option<usize>,
    start: Option<std::time::Instant>,
    scroll_from: f32,
    /// Outline position on screen when the transition started.
    outline_from: f32,
    /// Song (first visible row) opening and how far open it was at the start (0-1).
    open: Option<usize>,
    open_from: f32,
    closing: Option<usize>,
    closing_from: f32,
    /// Where things were on the last frame, to start the next transition from.
    scroll: f32,
    outline: f32,
    open_now: f32,
    closing_now: f32,
}

/// Length of a song list transition.
const LIST_ANIM_MS: f32 = 200.0;

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
            sort: SortMode::Artist,
            info: None,
            played: std::collections::HashMap::new(),
            collections: Vec::new(),
            collection: None,
            picker: None,
            info_progress: String::new(),
            scanning: None,
            rate_label: String::new(),
            show_converts: !has_taiko,
            visible: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            anim: ListAnim::default(),
            random_history: Vec::new(),
            rng: seed | 1,
        };
        s.refilter(library);
        s
    }

    pub fn refilter(&mut self, library: &[Entry]) {
        let selected = self.visible.get(self.cursor).copied();
        let needle = self.filter.to_lowercase();
        let only = self.collection.and_then(|c| self.collections.get(c)).map(|(_, set)| set);
        self.visible = library
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_converts || e.meta.mode == 1)
            .filter(|(i, _)| only.is_none_or(|set| set.contains(i)))
            .filter(|(_, e)| needle.is_empty() || e.label().to_lowercase().contains(&needle) || e.meta.creator.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.sort_visible(library);
        self.cursor = selected.and_then(|s| self.visible.iter().position(|&v| v == s)).unwrap_or(0);
        self.regroup(library);
    }

    /// Orders songs by the sort mode, keeping each song's difficulties together (and in their
    /// order). A song's value is its highest (hardest, longest, most recent...).
    fn sort_visible(&mut self, library: &[Entry]) {
        if self.sort == SortMode::Artist {
            return; // the library is already in artist / title order
        }
        let mut folder_ids: std::collections::HashMap<&std::path::Path, usize> = std::collections::HashMap::new();
        let folder = |i: usize| library[i].path.parent().unwrap_or(std::path::Path::new(""));
        for &i in &self.visible {
            let n = folder_ids.len();
            folder_ids.entry(folder(i)).or_insert(n);
        }
        let info = self.info.as_ref().map(|i| i.lock().unwrap());
        let value = |i: usize| -> f64 {
            let info = || info.as_ref().and_then(|v| v.get(i).copied().flatten());
            match self.sort {
                SortMode::Difficulty => info().map(|x| x.stars as f64).unwrap_or(f64::NAN),
                SortMode::Length => info().map(|x| x.length_ms as f64).unwrap_or(f64::NAN),
                SortMode::Bpm => info().map(|x| x.bpm as f64).unwrap_or(f64::NAN),
                SortMode::Played => self.played.get(&library[i].path).map(|&t| t as f64).unwrap_or(f64::NAN),
                SortMode::Added => library[i].modified as f64,
                SortMode::Artist | SortMode::Title => 0.0,
            }
        };
        let mut set_value = vec![f64::NAN; folder_ids.len()];
        for &i in &self.visible {
            let f = folder_ids[folder(i)];
            let v = value(i);
            if v.is_finite() && !(set_value[f] >= v) {
                set_value[f] = v;
            }
        }
        let newest_first = matches!(self.sort, SortMode::Played | SortMode::Added);
        let title = |i: usize| (library[i].meta.title.to_lowercase(), library[i].meta.artist.to_lowercase());
        let mut keyed: Vec<(usize, usize)> = self.visible.iter().map(|&i| (folder_ids[folder(i)], i)).collect();
        if self.sort == SortMode::Title {
            let titles: Vec<(String, String)> = {
                let mut t = vec![(String::new(), String::new()); folder_ids.len()];
                for &(f, i) in &keyed {
                    if t[f].0.is_empty() {
                        t[f] = title(i);
                    }
                }
                t
            };
            keyed.sort_by(|a, b| titles[a.0].cmp(&titles[b.0]).then(a.0.cmp(&b.0)));
        } else {
            keyed.sort_by(|a, b| {
                let (x, y) = (set_value[a.0], set_value[b.0]);
                // Unknown values go last either way.
                let order = match (x.is_nan(), y.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if newest_first => y.total_cmp(&x),
                    _ => x.total_cmp(&y),
                };
                order.then(a.0.cmp(&b.0))
            });
        }
        self.visible = keyed.into_iter().map(|(_, i)| i).collect();
    }

    /// Ctrl+S / the Sort button: next (or previous) sort mode.
    pub fn cycle_sort(&mut self, dir: i32, library: &[Entry]) {
        self.sort = self.sort.step(dir);
        self.refilter(library);
    }

    /// Picks the collection list row `row` (0 = all songs).
    pub fn pick_collection(&mut self, row: usize, library: &[Entry]) {
        self.collection = row.checked_sub(1).filter(|&c| c < self.collections.len());
        self.picker = None;
        self.refilter(library);
    }

    fn regroup(&mut self, library: &[Entry]) {
        // Row indices change meaning: start the list's motion afresh.
        self.anim = ListAnim::default();
        self.groups.clear();
        self.group_of.clear();
        let mut start = 0;
        for i in 1..=self.visible.len() {
            let split = i == self.visible.len() || library[self.visible[i]].path.parent() != library[self.visible[i - 1]].path.parent();
            if split {
                self.group_of.extend(std::iter::repeat_n(self.groups.len(), i - start));
                self.groups.push(start..i);
                start = i;
            }
        }
    }

    /// The song (range of `visible`) containing the cursor.
    fn cursor_group(&self) -> Option<std::ops::Range<usize>> {
        self.group_of.get(self.cursor).map(|&g| self.groups[g].clone())
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

    /// Row of the visible list under the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, row: usize) {
        if row < self.visible.len() {
            self.cursor = row;
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

    /// F2: jump to a random song among those listed (search and filters apply), keeping the
    /// position within the difficulty list where possible.
    pub fn random(&mut self) {
        let groups = &self.groups;
        if groups.len() < 2 {
            return;
        }
        let current = self.group_of.get(self.cursor).copied();
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
        let g = groups[pick].clone();
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
    pub fn sort_selected_set(&mut self, stars: &dyn Fn(usize) -> Option<f32>) {
        let Some(g) = self.cursor_group() else { return };
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
        // Sorting only reorders within the song, so the groups stay valid.
        if let Some(pos) = self.visible[g.clone()].iter().position(|&v| v == current) {
            self.cursor = g.start + pos;
        }
    }

    pub fn selected_set(&self) -> Vec<usize> {
        self.cursor_group().map(|g| self.visible[g].to_vec()).unwrap_or_default()
    }

    /// `stars` looks up a difficulty's star rating (library index) if it has been calculated.
    pub fn draw(&mut self, c: &mut Canvas, library: &[Entry], dt_ms: f64, status: &str, stars: &dyn Fn(usize) -> Option<f32>) {
        let w = c.width;
        let mode = if self.show_converts { "taiko + converts" } else { "taiko only" };
        let groups = &self.groups;
        crate::settings::tab_bar(c, 0, &format!("{} songs, {} maps  |  {mode}{}{}", groups.len(), self.visible.len(), self.rate_label, self.info_progress));

        let lw = Self::list_width(w);
        // Sort and collection buttons share the search row.
        let sort_label = format!("Sort: {}", self.sort.name());
        let coll_label = match self.collection.and_then(|c| self.collections.get(c)) {
            Some((name, _)) => format!("Collection: {}", name.chars().take(18).collect::<String>()),
            None => "Collection: all".to_string(),
        };
        let sw = Canvas::text_width(7.0, &sort_label) + 20.0;
        let cw = if self.collections.is_empty() { 0.0 } else { Canvas::text_width(7.0, &coll_label) + 20.0 };
        let mut bx = 24.0 + lw - sw;
        c.button(bx, 56.0, sw, 20.0, &sort_label, 7.0, Hit::SortCycle, false);
        if cw > 0.0 {
            bx -= cw + 6.0;
            c.button(bx, 56.0, cw, 20.0, &coll_label, 7.0, Hit::CollectionPick, self.collection.is_some());
        }
        let search_w = bx - 24.0 - 6.0;
        let search = if self.filter.is_empty() { "type to search...".to_string() } else { format!("search: {}_", self.filter) };
        c.round_rect(24.0, 54.0, search_w, 24.0, 6.0, rgba(0x23232f, 0.9), rgba(0x3a3a4e, 1.0), 1.0);
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
        let _ = dt_ms;
        let (top, bottom) = (86.0, 440.0);
        let view = bottom - top;
        // Where everything ends up: the selected song open, the others closed.
        let open_start = self.group_of.get(self.cursor).map(|&g| groups[g].start);
        let (mut final_y, mut final_cursor) = (0.0f32, 0.0f32);
        for g in groups {
            final_y += SONG_H;
            if g.contains(&self.cursor) {
                final_cursor = final_y + (self.cursor - g.start) as f32 * DIFF_H;
                final_y += g.len() as f32 * DIFF_H;
            }
        }
        let final_scroll = (final_cursor - view * 0.4).clamp(0.0, (final_y - view).max(0.0));
        let final_outline = top + final_cursor - final_scroll;

        // A new selection starts a transition from the current state.
        let a = &mut self.anim;
        if a.cursor != Some(self.cursor) || a.open != open_start {
            if a.cursor.is_none() {
                // First frame: no motion.
                *a = ListAnim { open: open_start, open_from: 1.0, scroll_from: final_scroll, outline_from: final_outline, ..Default::default() };
            } else {
                if open_start != a.open {
                    let reopen = open_start.is_some() && open_start == a.closing;
                    let from = if reopen { a.closing_now } else { 0.0 };
                    a.closing = a.open;
                    a.closing_from = a.open_now;
                    a.open = open_start;
                    a.open_from = from;
                } else {
                    a.open_from = a.open_now;
                    a.closing_from = a.closing_now;
                }
                a.scroll_from = a.scroll;
                a.outline_from = a.outline;
                a.start = Some(std::time::Instant::now());
            }
            a.cursor = Some(self.cursor);
        }
        let p = a.start.map(|t| progress(t, LIST_ANIM_MS)).unwrap_or(1.0);
        let open_f = a.open_from + (1.0 - a.open_from) * p;
        let closing_f = a.closing_from * (1.0 - p);
        let scroll = a.scroll_from + (final_scroll - a.scroll_from) * p;
        let outline = a.outline_from + (final_outline - a.outline_from) * p;
        (a.scroll, a.outline, a.open_now, a.closing_now) = (scroll, outline, open_f, closing_f);
        let (open_g, closing_g) = (a.open, a.closing);
        self.scroll = scroll;

        // (row, y, height, alpha): rows of opening / closing songs are squeezed and faded.
        let mut rows: Vec<(Row, f32, f32, f32)> = Vec::new();
        let mut y = 0.0;
        for g in groups {
            rows.push((Row::Song(g.clone()), y, SONG_H, 1.0));
            y += SONG_H;
            let f = if Some(g.start) == open_g { open_f } else if Some(g.start) == closing_g { closing_f } else { 0.0 };
            if f > 0.001 {
                for i in g.clone() {
                    rows.push((Row::Diff(i), y, DIFF_H, f * f));
                    y += DIFF_H * f;
                }
            }
        }
        let max_chars = |size: f32, room: f32| (room / (size / 7.0 * 6.0)).max(4.0) as usize;
        let fit = |s: String, n: usize| if s.chars().count() > n { s.chars().take(n.saturating_sub(3)).collect::<String>() + "..." } else { s };
        for (row, ry, rh, alpha) in &rows {
            let y = top + ry - self.scroll;
            if y + rh < top || y > bottom || *alpha < 0.02 {
                continue;
            }
            match row {
                Row::Song(g) => {
                    let e = &library[self.visible[g.start]];
                    let open = g.contains(&self.cursor);
                    let (bg, border) = if open { (0x2b2b3d, 0x5a5a78) } else { (0x1d1d27, 0x2a2a38) };
                    let hover = !open && c.hovered(24.0, y, lw, rh - 4.0);
                    let bg = if hover { 0x262634 } else { bg };
                    c.round_rect(24.0, y, lw, rh - 4.0, 7.0, rgba(bg, 0.88), rgba(border, 1.0), 1.0);
                    c.hit(24.0, y, lw, *rh, Hit::SongRow(g.start));
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
                    let a = *alpha;
                    let e = &library[self.visible[*i]];
                    let selected = *i == self.cursor;
                    let bg = if selected { 0x34344a } else { 0x191921 };
                    let bg = if !selected && c.hovered(44.0, y, lw - 20.0, rh - 3.0) { 0x24242f } else { bg };
                    c.round_rect(44.0, y, lw - 20.0, rh - 3.0, 5.0, rgba(bg, 0.92 * a), rgba(0x2a2a38, a), 1.0);
                    c.hit(44.0, y, lw - 20.0, *rh, Hit::SongRow(*i));
                    let col = if selected { 0xffffff } else { 0xc8c8d8 };
                    let convert = if e.meta.mode == 0 { "  (convert)" } else { "" };
                    let pill = if a > 0.6 { crate::stars::draw_pill(c, 24.0 + lw - 10.0, y + 6.0, 7.0, stars(self.visible[*i])) } else { 40.0 };
                    c.text_right(24.0 + lw - 18.0 - pill, y + 6.0, 7.0, rgba(0x8a8aa0, a), &format!("OD {:.1}", e.meta.od));
                    c.text(56.0, y + 6.0, 8.0, rgba(col, a), &fit(format!("{}{convert}", e.meta.version), max_chars(8.0, lw - 170.0)));
                }
            }
        }

        // The selection outline moves with the same transition.
        let oy = outline;
        if !self.visible.is_empty() && oy > top - DIFF_H && oy < bottom {
            c.round_rect(44.0, oy, lw - 20.0, DIFF_H - 3.0, 5.0, rgba(0, 0.0), rgba(0xEB452C, 1.0), 1.5);
        }
        c.text(24.0, 452.0, 7.0, rgba(0x6f6f86, 1.0), "Enter play  Up/Down difficulty  Left/Right song  F2 random  [ ] rate  Ctrl+S sort  Ctrl+L collection  Tab settings  Esc quit");
        if let Some(cursor) = self.picker {
            self.draw_picker(c, cursor);
        }
        c.text(24.0, 466.0, 7.0, rgba(0x55556a, 1.0), status);
    }
}

impl Select {
    /// The collection list over the song list.
    fn draw_picker(&self, c: &mut Canvas, cursor: usize) {
        const ROW: f32 = 18.0;
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.5));
        let rows: Vec<String> = std::iter::once("All songs".to_string())
            .chain(self.collections.iter().map(|(name, maps)| format!("{name}  ({})", maps.len())))
            .collect();
        let (bw, max_h) = (320.0, 380.0);
        let bh = (rows.len() as f32 * ROW + 56.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
        c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), "osu! collections");
        let (top, view) = (by + 32.0, bh - 50.0);
        let scroll = (cursor as f32 * ROW - view / 2.0).clamp(0.0, (rows.len() as f32 * ROW - view).max(0.0));
        for (i, label) in rows.iter().enumerate() {
            let y = top + i as f32 * ROW - scroll;
            if y < top - 1.0 || y + ROW > top + view + 1.0 {
                continue;
            }
            let selected = i == cursor;
            let current = (i == 0 && self.collection.is_none()) || self.collection.is_some_and(|c| c + 1 == i);
            let hover = c.hovered(bx + 8.0, y, bw - 16.0, ROW - 2.0);
            if selected || hover {
                c.round_rect(bx + 8.0, y, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(if selected { 0xEB452C } else { 0x3a3a4e }, 1.0), 1.0);
            }
            c.hit(bx + 8.0, y, bw - 16.0, ROW, Hit::PickRow(i));
            let text: String = label.chars().take(40).collect();
            c.text(bx + 20.0, y + 5.0, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &text);
            if current {
                c.text_right(bx + bw - 20.0, y + 5.0, 7.0, rgba(0x86E05A, 1.0), "shown");
            }
        }
        c.text_centered(w / 2.0, by + bh - 11.0, 6.0, rgba(0x6f6f86, 1.0), "Up/Down or mouse   Enter/click show   Esc close");
    }

    pub fn picker_len(&self) -> usize {
        self.collections.len() + 1
    }
}

pub struct Results {
    pub shown: std::time::Instant,
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
        let count = progress(self.shown, 700.0);
        let pop = progress(self.shown + std::time::Duration::from_millis(450), 300.0);
        if pop > 0.0 {
            let size = 70.0 * (1.6 - 0.6 * pop);
            c.text_centered(cx, 80.0 + (70.0 - size) / 2.0, size, rgba(0xFFD34D, pop), grade);
        }
        c.text_centered(cx, 170.0, 28.0, rgba(0xffffff, 1.0), &format!("{:.2}%", acc * count as f64));
        c.text_centered(cx, 212.0, 16.0, rgba(0xc8c8d8, 1.0), &format!("{:08}", (s.score as f64 * count as f64) as u64));

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
        c.button_row(424.0, &[("Retry  (Enter)", Hit::Key(KeyCode::Enter), true), ("Song select  (Esc)", Hit::Key(KeyCode::Escape), false)]);
    }
}
