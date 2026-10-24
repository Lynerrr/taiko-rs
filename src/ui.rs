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
    /// The mods picked ("HDHR"), shown on the Mods button.
    pub mods_label: String,
    pub show_converts: bool,
    /// Maps hidden from the list (right-click → Hide); the files stay where they are.
    pub hidden: Hidden,
    /// The right-click menu of a song list row.
    pub menu: Option<SongMenu>,
    /// The list being dragged with the left button.
    drag: Option<ListDrag>,
    /// Where each song's own row is in the list as last drawn (list units, top = 0).
    song_y: Vec<f32>,
    visible: Vec<usize>,
    /// Songs (beatmap sets) as ranges of `visible`: consecutive difficulties sharing a folder.
    /// Rebuilt when `visible` changes, not per frame (big libraries have tens of thousands of maps).
    groups: Vec<std::ops::Range<usize>>,
    /// Index into `groups` for each row of `visible`.
    group_of: Vec<usize>,
    cursor: usize,
    scroll: f32,
    anim: ListAnim,
    /// The difficulty last selected in each song (library index by folder), this session.
    remembered: std::collections::HashMap<std::path::PathBuf, usize>,
    /// Entered this song (first visible row) on its hardest difficulty: keep it there when the
    /// difficulties get sorted by stars.
    hardest_pending: Option<usize>,
    /// Selection changes jump instead of gliding (while right-click scrolling, where they come
    /// faster than the motion could follow).
    pub instant: bool,
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
    /// Length of the current transition (ms); 0 means `LIST_ANIM_MS`.
    ms: f32,
}

/// The list held by the pointer: it scrolls with it, then flies on and settles on a song.
struct ListDrag {
    /// Scroll and outline when the drag began.
    scroll: f32,
    outline: f32,
    /// Scroll now, and recent (time, scroll) samples for the release speed.
    now: f32,
    samples: std::collections::VecDeque<(std::time::Instant, f32)>,
}

/// Length of a song list transition.
const LIST_ANIM_MS: f32 = 200.0;
/// Top and bottom of the song list (virtual units).
const LIST_TOP: f32 = 86.0;
/// Height of a song's own row in the list (virtual units).
pub const SONG_ROW_H: f32 = 34.0;
const LIST_BOTTOM: f32 = 440.0;

pub enum SelectAction {
    None,
    Play(usize),
    Settings,
    Mods,
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
            mods_label: String::new(),
            show_converts: !has_taiko,
            hidden: Hidden::load(),
            menu: None,
            drag: None,
            song_y: Vec::new(),
            visible: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            anim: ListAnim::default(),
            remembered: std::collections::HashMap::new(),
            hardest_pending: None,
            instant: false,
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
            .filter(|(_, e)| !self.hidden.contains(&e.path))
            .filter(|(_, e)| needle.is_empty() || e.label().to_lowercase().contains(&needle) || e.meta.creator.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.sort_visible(library);
        self.regroup(library);
        match selected.and_then(|s| self.visible.iter().position(|&v| v == s)) {
            Some(pos) => self.cursor = pos,
            None => {
                self.cursor = 0;
                self.enter_song(0, library);
            }
        }
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
            KeyCode::ArrowDown => self.step(1, library),
            KeyCode::ArrowUp => self.step(-1, library),
            KeyCode::PageDown => self.jump_songs(5, library),
            KeyCode::PageUp => self.jump_songs(-5, library),
            KeyCode::Home => self.enter_song(0, library),
            KeyCode::End => self.enter_song(n.saturating_sub(1), library),
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
                    self.enter_song(i, library);
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(&i) = self.visible.get(self.cursor) {
                    return SelectAction::Play(i);
                }
            }
            KeyCode::Tab => return SelectAction::Settings,
            KeyCode::F1 => return SelectAction::Mods,
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

    /// Up/Down and the mouse wheel (`n` steps, negative = up): through the open song's
    /// difficulties, and past its first or last one into the previous or next song.
    pub fn step(&mut self, n: i32, library: &[Entry]) {
        for _ in 0..n.unsigned_abs() {
            let Some(g) = self.cursor_group() else { return };
            let next = self.cursor as i64 + n.signum() as i64;
            if (g.start as i64..g.end as i64).contains(&next) {
                self.cursor = next as usize;
            } else {
                self.jump_songs(n.signum(), library);
            }
        }
    }

    /// Moves `n` songs on (negative = back), onto each song's remembered or hardest difficulty.
    pub fn jump_songs(&mut self, n: i32, library: &[Entry]) {
        let Some(&g) = self.group_of.get(self.cursor) else { return };
        let target = (g as i64 + n as i64).clamp(0, self.groups.len() as i64 - 1) as usize;
        if target != g {
            self.enter_song(self.groups[target].start, library);
        }
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

    /// Whether `row` is the first row of its song (what a song's own row points at).
    pub fn is_song_start(&self, row: usize) -> bool {
        self.group_of.get(row).is_some_and(|&g| self.groups[g].start == row)
    }

    /// Whether `row` is in the song that is open (selected).
    pub fn in_open_song(&self, row: usize) -> bool {
        self.group_of.get(row).is_some() && self.group_of.get(row) == self.group_of.get(self.cursor)
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
    pub fn random(&mut self, library: &[Entry]) {
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
        let _ = offset;
        let start = groups[pick].start;
        self.enter_song(start, library);
    }

    /// Selects the song containing row `row`: the difficulty last picked there this session,
    /// else its hardest.
    pub fn enter_song(&mut self, row: usize, library: &[Entry]) {
        let Some(&g) = self.group_of.get(row) else { return };
        let g = self.groups[g].clone();
        let folder = library[self.visible[g.start]].path.parent().map(|p| p.to_path_buf());
        let remembered = folder.and_then(|f| self.remembered.get(&f).copied()).and_then(|i| self.visible[g.clone()].iter().position(|&v| v == i));
        match remembered {
            Some(pos) => {
                self.cursor = g.start + pos;
                self.hardest_pending = None;
            }
            None => {
                // Difficulties are in file size order until their star ratings are known, which
                // already puts the hardest last nearly always.
                self.cursor = g.end - 1;
                self.hardest_pending = Some(g.start);
            }
        }
    }

    /// Left-button drag: the list follows the pointer, `dy` (virtual units) from where it was
    /// pressed. The selection stays put until it's let go.
    pub fn drag_to(&mut self, dy: f32) {
        let a = &self.anim;
        let d = self.drag.get_or_insert_with(|| ListDrag { scroll: a.scroll, outline: a.outline, now: a.scroll, samples: Default::default() });
        d.now = d.scroll - dy;
        let now = std::time::Instant::now();
        d.samples.push_back((now, d.now));
        while d.samples.front().is_some_and(|(t, _)| now.duration_since(*t).as_millis() > 100) {
            d.samples.pop_front();
        }
    }

    /// Drops a drag without selecting anything (left song select mid-drag).
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// Lets go of the list: it carries on at the speed it was thrown, and the song that ends up
    /// on the selection line gets selected, gliding there.
    pub fn drag_end(&mut self, library: &[Entry]) {
        let Some(d) = self.drag.take() else { return };
        let velocity = match (d.samples.front(), d.samples.back()) {
            (Some(&(t0, s0)), Some(&(t1, s1))) if t1 > t0 => (s1 - s0) / t1.duration_since(t0).as_secs_f32() / 1000.0,
            _ => 0.0,
        };
        // Released while still: no throw.
        let still = d.samples.back().is_none_or(|(t, _)| t.elapsed().as_millis() > 60);
        let throw = if still { 0.0 } else { (velocity * 250.0).clamp(-3000.0, 3000.0) };
        // From where the list is shown (held a little past an end, it springs back).
        let line = self.anim.scroll + throw + (LIST_BOTTOM - LIST_TOP) * 0.4;
        // The song whose row is nearest the line (in the list as drawn).
        let target = self
            .song_y
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - line).abs().total_cmp(&(b.1 - line).abs()))
            .map(|(g, _)| g);
        // Glide from where the list was dropped; further takes a little longer.
        let ms = (LIST_ANIM_MS + throw.abs() * 0.15).clamp(LIST_ANIM_MS, 600.0);
        match target {
            Some(g) if self.group_of.get(self.cursor) != Some(&g) => {
                self.enter_song(self.groups[g].start, library);
                self.anim.ms = ms;
                // Begins the transition from here (not from where the selection was).
                self.anim.cursor = Some(usize::MAX);
            }
            _ => {
                let a = &mut self.anim;
                (a.scroll_from, a.outline_from, a.open_from, a.closing_from) = (a.scroll, a.outline, a.open_now, a.closing_now);
                a.start = Some(std::time::Instant::now());
                a.ms = ms;
            }
        }
    }

    /// Index of the song (group) holding the selection.
    pub fn current_song(&self) -> Option<usize> {
        self.group_of.get(self.cursor).copied()
    }

    /// Opens song `g` (clamped to the list), on its remembered or hardest difficulty.
    pub fn go_to_song(&mut self, g: i64, library: &[Entry]) {
        if self.groups.is_empty() {
            return;
        }
        let g = g.clamp(0, self.groups.len() as i64 - 1) as usize;
        if self.group_of.get(self.cursor) != Some(&g) {
            self.enter_song(self.groups[g].start, library);
        }
    }

    /// The menu's items for its row: hide the difficulty (not on a song's own row), hide the song.
    pub fn menu_items(&self, library: &[Entry]) -> Vec<(String, MenuAction)> {
        let Some(m) = &self.menu else { return Vec::new() };
        let Some(&i) = self.visible.get(m.row) else { return Vec::new() };
        let fit = |s: String| if s.chars().count() > 30 { s.chars().take(27).collect::<String>() + "..." } else { s };
        let mut items = Vec::new();
        if !m.header {
            items.push((fit(format!("Hide [{}]", library[i].meta.version)), MenuAction::HideDifficulty));
        }
        items.push((fit(format!("Hide {}", library[i].meta.title)), MenuAction::HideSong));
        items
    }

    /// Hides the menu's map or its whole song, keeping the selection near where it was.
    /// Returns what was hidden, for a message.
    pub fn hide(&mut self, action: MenuAction, library: &[Entry]) -> Option<String> {
        let m = self.menu.take()?;
        let &i = self.visible.get(m.row)?;
        let g = *self.group_of.get(m.row)?;
        let what = match action {
            MenuAction::HideDifficulty => {
                self.hidden.maps.insert(library[i].path.clone());
                format!("[{}]", library[i].meta.version)
            }
            MenuAction::HideSong => {
                self.hidden.songs.insert(library[i].path.parent()?.to_path_buf());
                let t = &library[i].meta.title;
                if t.chars().count() > 30 { t.chars().take(27).collect::<String>() + "..." } else { t.clone() }
            }
        };
        self.hidden.save();
        // The selection stays unless it was hidden: then another difficulty of that song, else
        // the song that takes its place in the list.
        let current = self.visible.get(self.cursor).copied();
        let stay = current.filter(|&c| !self.hidden.contains(&library[c].path)).or_else(|| {
            let g = self.current_song()?;
            self.groups[g].clone().map(|r| self.visible[r]).find(|&v| !self.hidden.contains(&library[v].path))
        });
        let place = self.current_song().unwrap_or(g);
        self.refilter(library);
        match stay.and_then(|v| self.visible.iter().position(|&x| x == v)) {
            Some(pos) => self.cursor = pos,
            None => self.go_to_song(place as i64, library),
        }
        Some(what)
    }

    /// Settings: lists everything that was hidden again.
    pub fn unhide_all(&mut self, library: &[Entry]) {
        self.hidden = Hidden::default();
        self.hidden.save();
        self.refilter(library);
    }

    /// osu!'s right-click scrolling: the pointer's height over the list (`y`, virtual units)
    /// picks the song that far through the whole list, top to bottom.
    pub fn scroll_to(&mut self, y: f32, library: &[Entry]) {
        let n = self.groups.len();
        if n == 0 {
            return;
        }
        let frac = ((y - LIST_TOP) / (LIST_BOTTOM - LIST_TOP)).clamp(0.0, 1.0);
        let g = (frac * (n - 1) as f32).round() as usize;
        if self.group_of.get(self.cursor) != Some(&g) {
            self.enter_song(self.groups[g].start, library);
        }
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
        // Still on the row it entered on: it means "the hardest", wherever that ends up.
        let to_last = self.hardest_pending == Some(g.start) && self.cursor == g.end - 1;
        let mut set: Vec<(f32, usize)> = keys.into_iter().zip(self.visible[g.clone()].iter().copied()).collect();
        set.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (slot, (_, i)) in g.clone().zip(set) {
            self.visible[slot] = i;
        }
        // Sorting only reorders within the song, so the groups stay valid.
        if to_last {
            self.cursor = g.end - 1;
        } else if let Some(pos) = self.visible[g.clone()].iter().position(|&v| v == current) {
            self.cursor = g.start + pos;
        }
        self.hardest_pending = None;
    }

    pub fn selected_set(&self) -> Vec<usize> {
        self.cursor_group().map(|g| self.visible[g].to_vec()).unwrap_or_default()
    }

    /// Remembers the selected difficulty for its song (call once a frame).
    pub fn remember_selection(&mut self, library: &[Entry]) {
        if let Some(&i) = self.visible.get(self.cursor) {
            if let Some(folder) = library[i].path.parent() {
                if self.remembered.get(folder) != Some(&i) {
                    self.remembered.insert(folder.to_path_buf(), i);
                }
            }
        }
    }

    /// `stars` looks up a difficulty's star rating (library index) if it has been calculated.
    pub fn draw(&mut self, c: &mut Canvas, library: &[Entry], dt_ms: f64, status: &str, stars: &dyn Fn(usize) -> Option<f32>) {
        let w = c.width;
        let mode = if self.show_converts { "taiko + converts" } else { "taiko only" };
        let groups = &self.groups;
        crate::settings::tab_bar(c, 0, &format!("{} songs, {} maps  |  {mode}{}{}", groups.len(), self.visible.len(), self.rate_label, self.info_progress));

        let lw = Self::list_width(w);
        // Mods, sort and collection buttons share the search row, right to left. On a narrow
        // screen the widest of them switches to a shorter label until the search box gets at
        // least SEARCH_MIN; past that the widest give up room (their labels get cut).
        const SEARCH_MIN: f32 = 140.0;
        let mods = if self.mods_label.is_empty() {
            vec!["Mods".to_string()]
        } else {
            vec![format!("Mods: {}", self.mods_label), format!("Mods ({})", self.mods_label.len() / 2)]
        };
        let sort = self.sort.name();
        let mut buttons = vec![(mods, Hit::ModsButton, !self.mods_label.is_empty()), (vec![format!("Sort: {sort}"), sort.to_string()], Hit::SortCycle, false)];
        if !self.collections.is_empty() {
            let name = self.collection.and_then(|c| self.collections.get(c)).map_or("all", |(name, _)| name.as_str());
            buttons.push((vec![format!("Collection: {name}"), name.to_string()], Hit::CollectionPick, self.collection.is_some()));
        }
        let width = |label: &str| Canvas::text_width(7.0, label) + 20.0;
        let budget = lw - SEARCH_MIN - 6.0 * buttons.len() as f32;
        let mut pick = vec![0; buttons.len()];
        loop {
            let widths: Vec<f32> = buttons.iter().zip(&pick).map(|((labels, ..), &k)| width(&labels[k])).collect();
            if widths.iter().sum::<f32>() <= budget {
                break;
            }
            let shorter = (0..buttons.len()).filter(|&i| pick[i] + 1 < buttons[i].0.len()).max_by(|&a, &b| widths[a].total_cmp(&widths[b]));
            let Some(i) = shorter else { break };
            pick[i] += 1;
        }
        let labels: Vec<&str> = buttons.iter().zip(&pick).map(|((labels, ..), &k)| labels[k].as_str()).collect();
        let natural: Vec<f32> = labels.iter().map(|l| width(l)).collect();
        let cap = shared_cap(&natural, budget);
        let mut bx = 24.0 + lw;
        for ((label, (_, hit, accent)), nw) in labels.iter().zip(&buttons).zip(natural) {
            let bw = nw.min(cap);
            bx -= bw;
            c.button(bx, 56.0, bw, 20.0, label, 7.0, *hit, *accent);
            bx -= 6.0;
        }
        let search_w = bx - 24.0;
        c.round_rect(24.0, 54.0, search_w, 24.0, 6.0, rgba(0x23232f, 0.9), rgba(0x3a3a4e, 1.0), 1.0);
        let room = search_w - 20.0;
        let (search, search_col) = if self.filter.is_empty() {
            let hint = if Canvas::text_width(10.0, "type to search...") <= room { "type to search..." } else { "search..." };
            (Canvas::fit(10.0, room, hint), 0x6f6f86)
        } else {
            let full = format!("search: {}_", self.filter);
            let text = if Canvas::text_width(10.0, &full) <= room { full } else { Canvas::fit_end(10.0, room, &format!("{}_", self.filter)) };
            (text, 0xffffff)
        };
        c.text(34.0, 61.0, 10.0, rgba(search_col, 1.0), &search);

        if let Some(n) = self.scanning {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "Scanning your Songs folder...");
            c.text_centered(24.0 + lw / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), &format!("{n} files read"));
        } else if self.visible.is_empty() {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            let hint = "Run: taiko-rs <Songs folder | map.osz | map.osu>";
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(24.0 + lw / 2.0, 222.0, size, rgba(0x8a8aa0, 1.0), &Canvas::fit(size, lw, hint));
        }

        // Rows: one per song, plus the selected song's difficulties under it.
        const SONG_H: f32 = SONG_ROW_H;
        const DIFF_H: f32 = 22.0;
        enum Row {
            Song(std::ops::Range<usize>),
            Diff(usize),
        }
        let _ = dt_ms;
        let (top, bottom) = (LIST_TOP, LIST_BOTTOM);
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
            if a.cursor.is_none() || self.instant {
                // First frame (or right-click scrolling): no motion.
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
                // A release's longer glide applies to the transition it starts, then resets.
                if a.cursor != Some(usize::MAX) {
                    a.ms = 0.0;
                }
            }
            a.cursor = Some(self.cursor);
        }
        let ms = if a.ms > 0.0 { a.ms } else { LIST_ANIM_MS };
        let p = a.start.map(|t| progress(t, ms)).unwrap_or(1.0);
        let open_f = a.open_from + (1.0 - a.open_from) * p;
        let closing_f = a.closing_from * (1.0 - p);
        let mut scroll = a.scroll_from + (final_scroll - a.scroll_from) * p;
        let mut outline = a.outline_from + (final_outline - a.outline_from) * p;
        if let Some(d) = &self.drag {
            // Held: exactly where the pointer has it (a little past the ends at most).
            let max = (final_y - view).max(0.0);
            let now = if d.now < 0.0 { d.now * 0.3 } else if d.now > max { max + (d.now - max) * 0.3 } else { d.now };
            outline = d.outline + (d.scroll - now);
            scroll = now;
        }
        (a.scroll, a.outline, a.open_now, a.closing_now) = (scroll, outline, open_f, closing_f);
        let (open_g, closing_g) = (a.open, a.closing);
        self.scroll = scroll;

        // (row, y, height, alpha): rows of opening / closing songs are squeezed and faded.
        let mut rows: Vec<(Row, f32, f32, f32)> = Vec::new();
        let mut y = 0.0;
        self.song_y.clear();
        for g in groups {
            self.song_y.push(y);
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
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
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
                    c.hit(24.0, y, lw, *rh, Hit::SongHeader(g.start));
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
        // Rows scrolling past the edges slide under them instead of over the search row.
        c.clip_since(list_start, 20.0, top - 2.0, lw + 8.0, bottom - top + 4.0);
        for h in &mut c.hits[hits_start..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(top - 2.0), (*y + *hh).min(bottom + 2.0));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }
        let hints = "Enter play  Up/Down difficulty  Left/Right song  F1 mods  F2 random  [ ] rate  Ctrl+S sort  Ctrl+L collection  Tab settings  Esc quit";
        c.text(24.0, 452.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(7.0, w - 48.0, hints));
        c.text(24.0, 466.0, 7.0, rgba(0x55556a, 1.0), &Canvas::fit(7.0, w - 48.0, status));
    }

    /// The collection picker and the right-click menu, over everything else on the screen
    /// (call after the map panel is drawn).
    pub fn draw_popups(&self, c: &mut Canvas, library: &[Entry]) {
        if let Some(cursor) = self.picker {
            self.draw_picker(c, cursor);
        }
        if self.menu.is_some() {
            self.draw_menu(c, library);
        }
    }
}

/// The largest width each of `widths` may take (narrow ones keep theirs) so they add up to at
/// most `total`; infinite if they already fit.
fn shared_cap(widths: &[f32], total: f32) -> f32 {
    let mut sorted = widths.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mut left = total.max(0.0);
    for (i, &w) in sorted.iter().enumerate() {
        let n = (sorted.len() - i) as f32;
        if w * n > left {
            return left / n;
        }
        left -= w;
    }
    f32::INFINITY
}

/// The mod select (F1): a column per group, each mod a chip to switch on or off. `cursor` is
/// the keyboard selection (index into `mods::ALL`).
pub fn draw_mod_select(c: &mut Canvas, mods: crate::mods::Mods, cursor: usize) {
    use crate::mods::{ALL, GROUPS};
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
    let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
    c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
    c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "MODS");
    c.text(px + 70.0, py + 16.0, 6.0, rgba(0x6f6f86, 1.0), "arrows + Space toggle   Backspace clears   F1 / Esc closes");
    let mult = mods.multiplier();
    let rank = if mods.unranked() { "  (not saved)" } else { "" };
    c.text_right(px + pw - 16.0, py + 14.0, 8.0, rgba(0xc8c8d8, 1.0), &format!("score x{mult:.2}{rank}"));
    let col_w = (pw - 24.0) / GROUPS.len() as f32;
    const ROW: f32 = 27.0;
    let mut hovered = None;
    for (g, (title, accent, list)) in GROUPS.iter().enumerate() {
        let x = px + 12.0 + g as f32 * col_w;
        c.text(x + 4.0, py + 38.0, 7.0, rgba(*accent, 1.0), title);
        for (r, &m) in list.iter().enumerate() {
            let y = py + 52.0 + r as f32 * ROW;
            let i = ALL.iter().position(|&x| x == m).unwrap_or(0);
            let on = mods.has(m);
            let hover = c.hovered(x, y, col_w - 6.0, ROW - 4.0);
            if hover {
                hovered = Some(m);
            }
            let bg = if i == cursor { 0x34344a } else if hover { 0x262634 } else { 0x1d1d27 };
            let border = if i == cursor { 0xEB452C } else if on { *accent } else { 0x2a2a38 };
            c.round_rect(x, y, col_w - 6.0, ROW - 4.0, 5.0, rgba(bg, 1.0), rgba(border, 1.0), if i == cursor { 1.5 } else { 1.0 });
            c.round_rect(x + 4.0, y + 4.0, 22.0, ROW - 12.0, 3.0, rgba(*accent, if on { 1.0 } else { 0.0 }), rgba(*accent, 1.0), 1.0);
            c.text_centered(x + 15.0, y + 8.0, 7.0, rgba(if on { 0x15151d } else { *accent }, 1.0), m.acronym());
            let room = ((col_w - 40.0) / 6.0).max(3.0) as usize;
            let name: String = if m.name().chars().count() > room { m.name().chars().take(room.saturating_sub(1)).collect::<String>() + "." } else { m.name().into() };
            c.text(x + 31.0, y + 8.5, 6.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), &name);
            c.hit(x, y, col_w - 6.0, ROW - 4.0, Hit::ModToggle(i));
        }
    }
    // What the pointed-at (else the selected) mod does.
    let shown = hovered.unwrap_or(ALL[cursor.min(ALL.len() - 1)]);
    let dy = py + ph - 46.0;
    c.text(px + 16.0, dy, 8.0, rgba(0xffffff, 1.0), &format!("{} ({})", shown.name(), shown.acronym()));
    c.text(px + 16.0, dy + 13.0, 7.0, rgba(0x9a9ab0, 1.0), shown.description());
    let active = if mods.is_empty() { "none".to_string() } else { mods.iter().map(|m| m.acronym()).collect::<Vec<_>>().join(" ") };
    c.text(px + 16.0, py + ph - 16.0, 7.0, rgba(0xc8c8d8, 1.0), &format!("on: {active}"));
    let bw = 60.0;
    c.button(px + pw - bw - 12.0, py + ph - 24.0, bw, 16.0, "Close", 7.0, Hit::Key(KeyCode::F1), true);
    c.button(px + pw - 2.0 * bw - 18.0, py + ph - 24.0, bw, 16.0, "Clear", 7.0, Hit::Key(KeyCode::Backspace), false);
}

/// Maps hidden from the song list in taiko-rs only (osu! and the Songs folder are untouched):
/// whole songs by folder, single difficulties by file. Kept in `hidden.tsv` in the config folder.
#[derive(Default)]
pub struct Hidden {
    pub songs: std::collections::HashSet<std::path::PathBuf>,
    pub maps: std::collections::HashSet<std::path::PathBuf>,
}

impl Hidden {
    fn file() -> Option<std::path::PathBuf> {
        crate::config::user_dir(false).map(|d| d.join("hidden.tsv"))
    }

    /// Lines of `song<TAB>folder` or `map<TAB>file`.
    pub fn load() -> Hidden {
        let mut h = Hidden::default();
        let Some(text) = Self::file().and_then(|f| std::fs::read_to_string(f).ok()) else { return h };
        for line in text.lines() {
            match line.split_once('\t') {
                Some(("song", p)) => h.songs.insert(p.into()),
                Some(("map", p)) => h.maps.insert(p.into()),
                _ => false,
            };
        }
        h
    }

    pub fn save(&self) {
        let Some(file) = Self::file() else { return };
        let mut text = String::new();
        for p in &self.songs {
            text += &format!("song\t{}\n", p.display());
        }
        for p in &self.maps {
            text += &format!("map\t{}\n", p.display());
        }
        if let Err(e) = std::fs::write(&file, text) {
            eprintln!("couldn't save {}: {e}", file.display());
        }
    }

    pub fn contains(&self, path: &std::path::Path) -> bool {
        self.maps.contains(path) || path.parent().is_some_and(|f| self.songs.contains(f))
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.maps.is_empty()
    }

    /// "2 songs, 1 difficulty" (settings value).
    pub fn describe(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match (self.songs.len(), self.maps.len()) {
            (0, 0) => "none".into(),
            (s, 0) => plural(s, "song", "songs"),
            (0, m) => plural(m, "difficulty", "difficulties"),
            (s, m) => format!("{}, {}", plural(s, "song", "songs"), plural(m, "difficulty", "difficulties")),
        }
    }
}

/// The right-click menu of a song list row, opened where the pointer was (virtual units).
pub struct SongMenu {
    pub row: usize,
    /// Opened on the song's own row (not one of its difficulties).
    pub header: bool,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    HideDifficulty,
    HideSong,
}

impl Select {
    /// The right-click menu, next to the pointer and kept on screen.
    fn draw_menu(&self, c: &mut Canvas, library: &[Entry]) {
        const ROW: f32 = 18.0;
        let Some(m) = &self.menu else { return };
        let items = self.menu_items(library);
        let bw = 200.0;
        let bh = items.len() as f32 * ROW + 26.0;
        let x = m.x.min(c.width - bw - 8.0).max(8.0);
        let y = m.y.min(480.0 - bh - 8.0).max(8.0);
        c.round_rect(x, y, bw, bh, 6.0, rgba(0x15151d, 0.98), rgba(0xEB452C, 0.8), 1.5);
        for (k, (label, _)) in items.iter().enumerate() {
            let ry = y + 6.0 + k as f32 * ROW;
            if c.hovered(x + 4.0, ry, bw - 8.0, ROW - 2.0) {
                c.round_rect(x + 4.0, ry, bw - 8.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(0xEB452C, 1.0), 1.0);
            }
            c.hit(x + 4.0, ry, bw - 8.0, ROW - 2.0, Hit::SongMenu(k));
            c.text(x + 12.0, ry + 5.0, 7.0, rgba(0xffffff, 1.0), label);
        }
        c.text(x + 12.0, y + bh - 14.0, 6.0, rgba(0x6f6f86, 1.0), "only in taiko-rs; files stay");
    }

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
    /// The saved replay of this play (or the replay that was watched).
    pub replay: Option<std::path::PathBuf>,
    /// Place among this map's local scores (1-based) and how many there are.
    pub rank: Option<(usize, usize)>,
    /// Watching someone's replay rather than playing.
    pub watched: Option<String>,
    /// Failed: Sudden Death / Perfect, or the HP bar under half at the end.
    pub failed: bool,
}

impl Results {
    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        let cx = w / 2.0;
        let size = Canvas::fit_size(12.0, 9.0, w - 48.0, &self.title);
        c.text_centered(cx, 40.0 + (12.0 - size) / 2.0, size, rgba(0x9a9ab0, 1.0), &Canvas::fit(size, w - 48.0, &self.title));
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
        // A failed play gets F, and says so.
        let grade = if self.failed { "F" } else { grade };
        let count = progress(self.shown, 700.0);
        let pop = progress(self.shown + std::time::Duration::from_millis(450), 300.0);
        if pop > 0.0 {
            let size = 70.0 * (1.6 - 0.6 * pop);
            let col = if self.failed { 0xEB452C } else { 0xFFD34D };
            c.text_centered(cx, 80.0 + (70.0 - size) / 2.0, size, rgba(col, pop), grade);
        }
        if self.failed {
            c.text_centered(cx, 56.0, 10.0, rgba(0xEB452C, 1.0), "FAILED");
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
            format!("You hit {:.0} ms {dir} on average; press {} a few times to adjust this map's offset.", s.mean_error().abs(), if s.mean_error() > 0.0 { "=" } else { "-" })
        } else {
            format!("Windows: great +-{:.0}  ok +-{:.0} ms", self.windows.great, self.windows.ok)
        };
        c.text_centered(cx, 380.0, 9.0, rgba(0x8a8aa0, 1.0), &hint);
        if let Some(who) = &self.watched {
            c.text_centered(cx, 404.0, 9.0, rgba(0x4fb4ff, 1.0), &format!("replay of {who}"));
        } else if let Some((rank, of)) = self.rank {
            c.text_centered(cx, 404.0, 9.0, rgba(0xFFD34D, 1.0), &format!("#{rank} of {of} local scores on this map"));
        }
        let mut buttons = vec![("Retry  (Enter)", Hit::Key(KeyCode::Enter), true)];
        if self.replay.is_some() {
            buttons.push(("Watch replay  (R)", Hit::Key(KeyCode::KeyR), false));
            buttons.push(("Export video  (V)", Hit::Key(KeyCode::KeyV), false));
        }
        buttons.push(("Song select  (Esc)", Hit::Key(KeyCode::Escape), false));
        c.button_row(424.0, &buttons);
    }
}
