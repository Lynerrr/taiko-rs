//! Song select and results screens.

use winit::keyboard::KeyCode;

use crate::game::{Stats, Windows};
use crate::library::Entry;
use crate::render::{accent, progress, rgba, Canvas, Hit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    Artist,
    Title,
    Difficulty,
    Length,
    Bpm,
    Hp,
    Od,
    /// osu!'s status, ranked and loved first.
    Status,
    Played,
    Added,
}

impl SortMode {
    pub const ALL: [SortMode; 10] = [
        SortMode::Artist,
        SortMode::Title,
        SortMode::Difficulty,
        SortMode::Length,
        SortMode::Bpm,
        SortMode::Hp,
        SortMode::Od,
        SortMode::Status,
        SortMode::Played,
        SortMode::Added,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SortMode::Artist => "artist",
            SortMode::Title => "title",
            SortMode::Difficulty => "difficulty",
            SortMode::Length => "length",
            SortMode::Bpm => "bpm",
            SortMode::Hp => "hp",
            SortMode::Od => "od",
            SortMode::Status => "status",
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

/// A heading of the song list and the songs (`Select::groups` indices) under it.
struct Section {
    label: String,
    /// None: the accent colour.
    colour: Option<u32>,
    groups: std::ops::Range<usize>,
}

pub struct Select {
    pub filter: String,
    /// The search box has the keys (clicked, or Ctrl+F): everything typed goes into it.
    pub searching: bool,
    pub sort: SortMode,
    /// Star rating / length / BPM per library index, filled in the background.
    pub info: Option<crate::mapinfo::Shared>,
    /// When each map was last played (Unix seconds), from taiko.rs and osu!.
    pub played: std::collections::HashMap<std::path::PathBuf, u64>,
    /// Each map's status on osu! (`search`'s codes), from osu!.db and the mirrors, for the
    /// search's `status=` filter.
    pub status: std::collections::HashMap<std::path::PathBuf, u8>,
    /// Map sets loved on the online server (they give pp there, whatever osu! says).
    pub server_loved: std::collections::HashSet<u32>,
    /// The account logged in is a loved nominator: the right-click menu can love sets.
    pub nominator: bool,
    /// osu! collections: (name, library indices).
    pub collections: Vec<(String, std::collections::HashSet<usize>)>,
    /// The collection shown (None: everything) and the open collection list's cursor.
    pub collection: Option<usize>,
    pub picker: Option<usize>,
    /// Shown in the header while the analysis for the current sort is running.
    pub info_progress: String,
    /// Files read so far while the library is still being scanned.
    pub scanning: Option<usize>,
    /// The folder dialog is open (from the button shown when there are no maps at all).
    pub picking_folder: bool,
    /// Speed modifier shown in the header ("" at 1.0x).
    pub rate_label: String,
    /// The mods picked ("HDHR"), shown on the Mods button.
    pub mods_label: String,
    pub show_converts: bool,
    /// Maps hidden from the list (right-click → Hide); the files stay where they are.
    pub hidden: Hidden,
    /// The right-click menu of a song list row.
    pub menu: Option<SongMenu>,
    /// Sets checked for a newer version on osu! this session, by set ID.
    pub updates: std::collections::HashMap<u32, SetUpdate>,
    /// Difficulties an update took out of their set (renamed or dropped on osu!): not listed.
    pub gone: std::collections::HashSet<std::path::PathBuf>,
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
    /// The list's headings, in order: what the sort groups songs by, like osu!'s groups. Only the
    /// section holding the selection shows its songs.
    sections: Vec<Section>,
    /// The section of each song (`groups`).
    section_of: Vec<usize>,
    /// The headings (by name) whose songs are listed: the ones opened by clicking them, and the
    /// one the selection moved into. Scrolling never opens or closes any.
    open: std::collections::HashSet<String>,
    /// The sort `open` is for (another starts over with the selection's section).
    open_for: SortMode,
    /// The cursor when the player closed the heading it was under: it stays closed until the
    /// selection moves.
    closed_at: Option<usize>,
    /// What the sort ordered each row of `visible` by (its song's value; NaN: not known or the
    /// sort has none).
    row_value: Vec<f64>,
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
    /// The song list's share of the width (`Config::song_list_share`).
    pub list_share: f32,
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
/// The song list's share of the width: least, most and usual (dragging its edge).
pub const LIST_SHARE: (f32, f32, f32) = (0.3, 0.7, 0.52);
/// The narrowest the song list and the panel beside it get (virtual units).
const LIST_MIN: f32 = 260.0;
const PANEL_MIN: f32 = 220.0;
/// Height of a song's own row in the list (virtual units).
pub const SONG_ROW_H: f32 = 34.0;

/// What a song's name starts with, as its heading when the list is by artist or title.
fn initial(name: &str) -> String {
    match name.trim_start().chars().next() {
        Some(c) if c.is_ascii_digit() => "0-9".to_string(),
        Some(c) if c.is_alphabetic() && (c as u32) < 0x250 => c.to_uppercase().collect(),
        _ => "Other".to_string(),
    }
}

/// A status section's heading and colour in the song list.
fn status_heading(code: u8) -> (&'static str, u32) {
    use crate::search::*;
    match code {
        RANKED => ("Ranked", 0x6ecb63),
        APPROVED => ("Approved", 0x6ecb63),
        QUALIFIED => ("Qualified", 0x4fb4ff),
        LOVED => ("Loved", 0xff66aa),
        PENDING | GRAVEYARD => ("Pending / Graveyard", 0xb0a06a),
        NOT_SUBMITTED => ("Not submitted", 0x8a8aa0),
        _ => ("Status not known yet", 0x8a8aa0),
    }
}
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
            list_share: LIST_SHARE.2,
            filter: String::new(),
            searching: false,
            sort: SortMode::Artist,
            info: None,
            played: std::collections::HashMap::new(),
            status: std::collections::HashMap::new(),
            server_loved: std::collections::HashSet::new(),
            nominator: false,
            collections: Vec::new(),
            collection: None,
            picker: None,
            info_progress: String::new(),
            scanning: None,
            picking_folder: false,
            rate_label: String::new(),
            mods_label: String::new(),
            show_converts: !has_taiko,
            hidden: Hidden::load(),
            menu: None,
            updates: std::collections::HashMap::new(),
            gone: std::collections::HashSet::new(),
            drag: None,
            song_y: Vec::new(),
            visible: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            sections: Vec::new(),
            section_of: Vec::new(),
            open: Default::default(),
            open_for: SortMode::Artist,
            closed_at: None,
            row_value: Vec::new(),
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
        let query = crate::search::Query::parse(&self.filter);
        let only = self.collection.and_then(|c| self.collections.get(c)).map(|(_, set)| set);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let info = self.info.as_ref().map(|i| i.lock().unwrap());
        let facts = |i: usize, e: &Entry| {
            let known = info.as_ref().and_then(|v| v.get(i).copied().flatten());
            crate::search::Facts {
                stars: known.map(|x| x.stars as f64),
                length_s: known.map(|x| x.length_ms as f64 / 1000.0),
                bpm: known.map(|x| x.bpm as f64),
                played_days: self.played.get(&e.path).filter(|&&t| t > 0).map(|&t| now.saturating_sub(t) as f64 / 86_400.0),
                status: self.status_of(e),
            }
        };
        self.visible = library
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_converts || e.meta.mode == 1)
            .filter(|(i, _)| only.is_none_or(|set| set.contains(i)))
            .filter(|(_, e)| !self.hidden.contains(&e.path) && !self.gone.contains(&e.path))
            .filter(|&(i, e)| query.is_empty() || query.matches(&e.meta, &e.label(), &facts(i, e)))
            .map(|(i, _)| i)
            .collect();
        drop(info);
        self.sort_visible(library);
        self.regroup(library);
        match selected.and_then(|s| self.visible.iter().position(|&v| v == s)) {
            Some(pos) => self.cursor = pos,
            None => {
                self.cursor = 0;
                self.enter_song(0, library);
            }
        }
        self.open_cursor_section();
    }

    /// The listed songs in order, each as its difficulties' library indices.
    #[cfg(test)]
    pub fn songs(&self) -> Vec<Vec<usize>> {
        self.groups.iter().map(|g| self.visible[g.clone()].to_vec()).collect()
    }

    /// The status section a map is listed under: graveyard sorts (and lists) with pending.
    fn status_section(&self, e: &Entry) -> u8 {
        match self.status_of(e) {
            crate::search::GRAVEYARD => crate::search::PENDING,
            c => c,
        }
    }

    /// The list's headings: (heading, songs under it), in order.
    #[cfg(test)]
    pub fn sections(&self) -> Vec<(String, usize)> {
        self.sections.iter().map(|s| (s.label.clone(), s.groups.len())).collect()
    }

    /// One past the last song of section `s`, as an index of `songs()`.
    #[cfg(test)]
    pub fn sections_end(&self, s: usize) -> usize {
        self.sections[s].groups.end
    }

    /// The headings whose songs are listed: the one holding the selection, unless closed.
    #[cfg(test)]
    pub fn open_sections(&self) -> Vec<usize> {
        (0..self.sections.len()).filter(|&s| self.section_open(s)).collect()
    }

    /// A map's status on osu! (`search`'s codes), loved here counting as loved.
    fn status_of(&self, e: &Entry) -> u8 {
        match e.meta.set_id {
            Some(set) if self.server_loved.contains(&set) => crate::search::LOVED,
            set => self.status.get(&e.path).copied().unwrap_or(if set.is_some() { crate::search::UNKNOWN } else { crate::search::NOT_SUBMITTED }),
        }
    }

    /// Orders songs by the sort mode, keeping each song's difficulties together (and in their
    /// order). A song's value is its highest (hardest, longest, most recent...).
    fn sort_visible(&mut self, library: &[Entry]) {
        if self.sort == SortMode::Artist {
            // The library is already in artist / title order.
            self.row_value = vec![f64::NAN; self.visible.len()];
            return;
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
                SortMode::Hp => library[i].meta.hp,
                SortMode::Od => library[i].meta.od,
                // Ordered as osu! has them (graveyard with pending); not known: last.
                SortMode::Status => match self.status_section(&library[i]) {
                    crate::search::UNKNOWN => f64::NAN,
                    c => c as f64,
                },
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
        // Highest first: the newest, and the best status.
        let newest_first = matches!(self.sort, SortMode::Played | SortMode::Added | SortMode::Status);
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
        self.row_value = keyed.iter().map(|&(f, _)| if self.sort == SortMode::Title { f64::NAN } else { set_value[f] }).collect();
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
        self.sections.clear();
        self.section_of.clear();
        if self.open_for != self.sort {
            self.open_for = self.sort;
            self.open.clear();
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        for g in 0..self.groups.len() {
            let (label, colour) = self.section_heading(library, self.groups[g].start, now);
            match self.sections.last_mut() {
                Some(last) if last.label == label => last.groups.end = g + 1,
                _ => self.sections.push(Section { label, colour, groups: g..g + 1 }),
            }
            self.section_of.push(self.sections.len() - 1);
        }
        let labels: std::collections::HashSet<&str> = self.sections.iter().map(|s| s.label.as_str()).collect();
        self.open.retain(|l| labels.contains(l.as_str()));
    }

    /// The heading the song at row `row` is listed under for the sort: what it's grouped by.
    fn section_heading(&self, library: &[Entry], row: usize, now: u64) -> (String, Option<u32>) {
        let e = &library[self.visible[row]];
        let v = self.row_value.get(row).copied().unwrap_or(f64::NAN);
        let unknown = || "Not worked out yet".to_string();
        let age = |t: f64| {
            let days = now.saturating_sub(t as u64) / 86_400;
            match days {
                0 => "Today",
                1 => "Yesterday",
                2..=6 => "Last 7 days",
                7..=29 => "Last 30 days",
                30..=179 => "Last 6 months",
                _ => "Older",
            }
            .to_string()
        };
        let label = match self.sort {
            SortMode::Artist => initial(&e.meta.artist),
            SortMode::Title => initial(&e.meta.title),
            // From the song's value the list was ordered by (its best difficulty's status), not from
            // the first difficulty's: a song of mixed statuses would split its heading in two.
            SortMode::Status => {
                let code = if v.is_nan() { crate::search::UNKNOWN } else { v as u8 };
                let (name, colour) = status_heading(code);
                return (name.to_string(), Some(colour));
            }
            _ if v.is_nan() && self.sort == SortMode::Played => "Never played".to_string(),
            _ if v.is_nan() => unknown(),
            SortMode::Difficulty => match v.floor() as i64 {
                n if n >= 10 => "10+ stars".to_string(),
                n => format!("{n}.00 - {n}.99 stars"),
            },
            SortMode::Length => match (v / 60_000.0).floor() as i64 {
                0 => "Under 1 minute".to_string(),
                m if m >= 10 => "10+ minutes".to_string(),
                m => format!("{m} - {} minutes", m + 1),
            },
            SortMode::Bpm => match v {
                v if v < 60.0 => "Under 60 BPM".to_string(),
                v if v >= 300.0 => "300+ BPM".to_string(),
                v => format!("{} - {} BPM", (v / 20.0).floor() as i64 * 20, (v / 20.0).floor() as i64 * 20 + 19),
            },
            SortMode::Hp => format!("HP {:.0}.0 - {:.0}.9", v.floor(), v.floor()),
            SortMode::Od => format!("OD {:.0}.0 - {:.0}.9", v.floor(), v.floor()),
            SortMode::Played | SortMode::Added => age(v),
        };
        (label, None)
    }

    /// The heading the selection is under.
    fn cursor_section(&self) -> Option<usize> {
        self.group_of.get(self.cursor).map(|&g| self.section_of[g])
    }

    /// Whether section `s` lists its songs.
    fn section_open(&self, s: usize) -> bool {
        self.open.contains(&self.sections[s].label)
    }

    /// The heading the selection is under stays open, unless the player closed it with the
    /// selection there.
    fn open_cursor_section(&mut self) {
        if let Some(s) = self.cursor_section() {
            if self.closed_at != Some(self.cursor) && !self.sections[s].label.is_empty() {
                self.open.insert(self.sections[s].label.clone());
            }
        }
    }

    /// A click on heading `s`: opens it or closes it (the selection stays where it is).
    pub fn click_section(&mut self, s: usize) {
        let Some(sec) = self.sections.get(s) else { return };
        if !self.open.remove(&sec.label) {
            self.open.insert(sec.label.clone());
            if self.cursor_section() == Some(s) {
                self.closed_at = None;
            }
        } else if self.cursor_section() == Some(s) {
            self.closed_at = Some(self.cursor);
        }
    }

    /// The songs (`groups` indices) whose headings are open, in order: what scrolling goes through.
    fn listed_groups(&self) -> Vec<usize> {
        (0..self.groups.len()).filter(|&g| self.section_open(self.section_of[g])).collect()
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
            KeyCode::ArrowDown => self.step(1, library, true),
            KeyCode::ArrowUp => self.step(-1, library, true),
            KeyCode::PageDown => self.jump_songs(5, library),
            KeyCode::PageUp => self.jump_songs(-5, library),
            KeyCode::Home | KeyCode::End => {
                let listed = self.listed_groups();
                if let Some(&g) = if key == KeyCode::Home { listed.first() } else { listed.last() } {
                    self.enter_song(self.groups[g].start, library);
                }
            }
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
                    self.searching = false;
                    return SelectAction::Play(i);
                }
            }
            KeyCode::Tab => return SelectAction::Settings,
            KeyCode::F1 => return SelectAction::Mods,
            KeyCode::Backspace => {
                self.filter.pop();
                self.refilter(library);
            }
            // Searching: Escape just leaves the box, the search staying.
            KeyCode::Escape if self.searching => self.searching = false,
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
    /// difficulties, and past its first or last one into the previous or next listed song. With
    /// `open_next` (the keys), past the last listed song the next heading opens and goes on into
    /// it; the wheel stops there.
    pub fn step(&mut self, n: i32, library: &[Entry], open_next: bool) {
        for _ in 0..n.unsigned_abs() {
            let Some(g) = self.cursor_group() else { return };
            let next = self.cursor as i64 + n.signum() as i64;
            if (g.start as i64..g.end as i64).contains(&next) {
                self.cursor = next as usize;
            } else {
                self.move_songs(n.signum(), library, open_next);
            }
        }
    }

    /// Moves `n` songs on (negative = back) among those listed (the open headings'), onto each
    /// song's remembered or hardest difficulty.
    pub fn jump_songs(&mut self, n: i32, library: &[Entry]) {
        self.move_songs(n, library, false);
    }

    fn move_songs(&mut self, n: i32, library: &[Entry], open_next: bool) {
        let Some(&g) = self.group_of.get(self.cursor) else { return };
        let listed = self.listed_groups();
        // Where the selection's song is among them (or would be, if its heading was closed).
        let pos = listed.partition_point(|&k| k < g);
        let base = match listed.get(pos) {
            Some(&k) if k == g => pos as i64,
            _ if n > 0 => pos as i64 - 1,
            _ => pos as i64,
        };
        let idx = base + n as i64;
        let target = if (0..listed.len() as i64).contains(&idx) {
            listed[idx as usize]
        } else if open_next && n.abs() == 1 {
            // Past the end of what's listed: the heading next to it.
            let edge = if n > 0 { listed.last().copied().unwrap_or(g) as i64 + 1 } else { listed.first().copied().unwrap_or(g) as i64 - 1 };
            if !(0..self.groups.len() as i64).contains(&edge) {
                return;
            }
            let label = self.sections[self.section_of[edge as usize]].label.clone();
            self.open.insert(label);
            edge as usize
        } else {
            match listed.get(idx.clamp(0, listed.len() as i64 - 1) as usize) {
                Some(&k) => k,
                None => return,
            }
        };
        if target != g {
            self.enter_song(self.groups[target].start, library);
        }
    }

    pub fn type_text(&mut self, text: &str, library: &[Entry]) {
        if !self.searching {
            return;
        }
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
            self.open_cursor_section();
        }
    }

    /// The selected difficulty (library index).
    pub fn selected(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    /// Width of the song list (`share` of the screen, as the player dragged it); the preview
    /// panel takes the rest. Neither gets too narrow to use (`LIST_MIN`, `PANEL_MIN`); a screen
    /// too narrow for both keeps the list at its usual size.
    pub fn list_width(screen_w: f32, share: f32) -> f32 {
        match Self::share_range(screen_w) {
            Some((min, max)) => screen_w * share.clamp(min, max),
            None => (screen_w * LIST_SHARE.2).max(300.0).min(screen_w - 48.0),
        }
    }

    /// The shares of a `screen_w` wide screen the song list can take: within `LIST_SHARE`, and
    /// leaving the list and the panel (past the margins and the gap between them) their least.
    /// None when the screen is too narrow for both.
    pub fn share_range(screen_w: f32) -> Option<(f32, f32)> {
        let w = screen_w.max(1.0);
        let (min, max) = (LIST_SHARE.0.max(LIST_MIN / w), LIST_SHARE.1.min((w - 60.0 - PANEL_MIN) / w));
        (min <= max).then_some((min, max))
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
        self.open_cursor_section();
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
            .filter(|(_, y)| y.is_finite())
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

    /// The map (library index) the right-click menu is open on.
    pub fn menu_map(&self) -> Option<usize> {
        self.visible.get(self.menu.as_ref()?.row).copied()
    }

    /// The menu's items for its row: hide the difficulty (not on a song's own row), hide the song,
    /// and get the song's newest version from osu! (a set that has an ID).
    pub fn menu_items(&self, library: &[Entry]) -> Vec<(String, MenuAction)> {
        let Some(m) = &self.menu else { return Vec::new() };
        let Some(&i) = self.visible.get(m.row) else { return Vec::new() };
        let fit = |s: String| if s.chars().count() > 30 { s.chars().take(27).collect::<String>() + "..." } else { s };
        let mut items = Vec::new();
        if !m.header {
            items.push((fit(format!("Hide [{}]", library[i].meta.version)), MenuAction::HideDifficulty));
        }
        items.push((fit(format!("Hide {}", library[i].meta.title)), MenuAction::HideSong));
        if let Some(set) = library[i].meta.set_id.filter(|_| self.nominator) {
            let loved = self.server_loved.contains(&set);
            items.push((if loved { "Unlove on taiko.rs" } else { "Love on taiko.rs (gives pp)" }.to_string(), MenuAction::Love(set, !loved)));
        }
        if let Some(id) = library[i].set_id() {
            let label = match self.updates.get(&id) {
                Some(SetUpdate::Updating) => "Updating...".to_string(),
                Some(SetUpdate::Available) => format!("Update {}", library[i].meta.title),
                _ => format!("Download {} again", library[i].meta.title),
            };
            items.push((fit(label), MenuAction::UpdateSong));
        }
        items
    }

    /// Hides the menu's map or its whole song, keeping the selection near where it was.
    /// Returns what was hidden, for a message.
    pub fn hide(&mut self, action: MenuAction, library: &[Entry]) -> Option<String> {
        let m = self.menu.take()?;
        let &i = self.visible.get(m.row)?;
        let g = *self.group_of.get(m.row)?;
        let what = match action {
            MenuAction::UpdateSong => return None,
            MenuAction::HideDifficulty => {
                self.hidden.maps.insert(library[i].path.clone());
                format!("[{}]", library[i].meta.version)
            }
            MenuAction::HideSong => {
                self.hidden.songs.insert(library[i].path.parent()?.to_path_buf());
                let t = &library[i].meta.title;
                if t.chars().count() > 30 { t.chars().take(27).collect::<String>() + "..." } else { t.clone() }
            }
            MenuAction::Love(..) => return None,
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
    /// picks the song that far through the listed songs (the open headings'), top to bottom.
    pub fn scroll_to(&mut self, y: f32, library: &[Entry]) {
        let listed = self.listed_groups();
        if listed.is_empty() {
            return;
        }
        let frac = ((y - LIST_TOP) / (LIST_BOTTOM - LIST_TOP)).clamp(0.0, 1.0);
        let g = listed[(frac * (listed.len() - 1) as f32).round() as usize];
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
        self.open_cursor_section();
        let w = c.width;
        let mode = if self.show_converts { "taiko + converts" } else { "taiko only" };
        let groups = &self.groups;
        crate::settings::tab_bar(c, 0, &format!("{} songs, {} maps  |  {mode}{}{}", groups.len(), self.visible.len(), self.rate_label, self.info_progress));

        let lw = Self::list_width(w, self.list_share);
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
        let edge = if self.searching { rgba(crate::render::accent(), 1.0) } else { rgba(0x3a3a4e, 1.0) };
        c.round_rect(24.0, 54.0, search_w, 24.0, 6.0, rgba(0x23232f, 0.9), edge, 1.0);
        let room = search_w - 20.0;
        let caret = if self.searching { "_" } else { "" };
        let (search, search_col) = if self.filter.is_empty() {
            let (long, short) = if self.searching { ("type to search...", "search...") } else { ("click to search (ctrl+f)...", "search...") };
            let hint = if Canvas::text_width(10.0, long) <= room { long } else { short };
            (Canvas::fit(10.0, room, &format!("{hint}{caret}")), 0x6f6f86)
        } else {
            let full = format!("search: {}{caret}", self.filter);
            let text = if Canvas::text_width(10.0, &full) <= room { full } else { Canvas::fit_end(10.0, room, &format!("{}{caret}", self.filter)) };
            (text, 0xffffff)
        };
        c.text(34.0, 61.0, 10.0, rgba(search_col, 1.0), &search);
        c.hit(24.0, 54.0, search_w, 24.0, Hit::TextField);

        if let Some(n) = self.scanning {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "Scanning your Songs folder...");
            c.text_centered(24.0 + lw / 2.0, 222.0, 10.0, rgba(0x8a8aa0, 1.0), &format!("{n} files read"));
        } else if library.is_empty() && !cfg!(target_os = "android") {
            // Most likely osu! lives somewhere it wasn't looked for.
            let cx = 24.0 + lw / 2.0;
            c.text_centered(cx, 190.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            let hint = "Show taiko.rs where your osu! Songs folder is, or get maps in DOWNLOAD.";
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(cx, 212.0, size, rgba(0x8a8aa0, 1.0), &Canvas::fit(size, lw, hint));
            let label = if self.picking_folder { "Choosing in the folder dialog..." } else { "Choose Songs folder" };
            let bw = (Canvas::text_width(9.0, label) + 32.0).min(lw);
            c.button(cx - bw / 2.0, 234.0, bw, 26.0, label, 9.0, Hit::SongsFolder, true);
            let later = "or press Enter. Change it later in Settings > Library.";
            c.text_centered(cx, 268.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, lw, later));
        } else if self.visible.is_empty() {
            c.text_centered(24.0 + lw / 2.0, 200.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            let hint = if cfg!(target_os = "android") {
                "Open .osz files with taiko.rs, or get maps in DOWNLOAD"
            } else {
                "Run: taiko-rs <Songs folder | map.osz | map.osu>"
            };
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(24.0 + lw / 2.0, 222.0, size, rgba(0x8a8aa0, 1.0), &Canvas::fit(size, lw, hint));
        }

        // Rows: one per song, plus the selected song's difficulties under it.
        const SONG_H: f32 = SONG_ROW_H;
        const DIFF_H: f32 = 22.0;
        // A heading above each section's songs (what the sort groups by).
        const SECTION_H: f32 = 20.0;
        enum Row {
            Section(usize),
            Song(std::ops::Range<usize>),
            Diff(usize),
        }
        let _ = dt_ms;
        let (top, bottom) = (LIST_TOP, LIST_BOTTOM);
        let view = bottom - top;
        // Where everything ends up: the selected song open, the others closed.
        let open_start = self.group_of.get(self.cursor).map(|&g| groups[g].start);
        let section_open: Vec<bool> = (0..self.sections.len()).map(|s| self.section_open(s)).collect();
        let (mut final_y, mut final_cursor) = (0.0f32, 0.0f32);
        let cursor_section = self.cursor_section();
        let cursor_hidden = cursor_section.is_some_and(|si| !section_open[si]);
        for (si, sec) in self.sections.iter().enumerate() {
            // A closed heading holding the selection is the one to keep in view.
            if !section_open[si] && cursor_section == Some(si) {
                final_cursor = final_y;
            }
            final_y += SECTION_H;
            if !section_open[si] {
                continue;
            }
            for g in &groups[sec.groups.clone()] {
                final_y += SONG_H;
                if g.contains(&self.cursor) {
                    final_cursor = final_y + (self.cursor - g.start) as f32 * DIFF_H;
                    final_y += g.len() as f32 * DIFF_H;
                }
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
        for (si, sec) in self.sections.iter().enumerate() {
            rows.push((Row::Section(si), y, SECTION_H, 1.0));
            y += SECTION_H;
            for g in &groups[sec.groups.clone()] {
                if !section_open[si] {
                    // Not listed: nowhere to land when the list is let go.
                    self.song_y.push(f32::INFINITY);
                    continue;
                }
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
        }
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
        let max_chars = |size: f32, room: f32| (room / Canvas::char_width(size)).max(4.0) as usize;
        let fit = |s: String, n: usize| if s.chars().count() > n { s.chars().take(n.saturating_sub(3)).collect::<String>() + "..." } else { s };
        for (row, ry, rh, alpha) in &rows {
            let y = top + ry - self.scroll;
            if y + rh < top || y > bottom || *alpha < 0.02 {
                continue;
            }
            match row {
                Row::Section(si) => {
                    let sec = &self.sections[*si];
                    let col = sec.colour.unwrap_or_else(accent);
                    let open = section_open[*si];
                    let hover = c.hovered(24.0, y, lw, rh - 2.0);
                    let holds_cursor = cursor_section == Some(*si) && !open;
                    let border = if holds_cursor { rgba(accent(), 1.0) } else { rgba(col, if open { 0.7 } else { 0.35 }) };
                    c.round_rect(24.0, y, lw, rh - 3.0, 5.0, rgba(if hover { 0x24242f } else { 0x16161e }, 0.9), border, if holds_cursor { 1.5 } else { 1.0 });
                    c.hit(24.0, y, lw, *rh, Hit::SectionHeader(*si));
                    let n = sec.groups.len();
                    c.text(34.0, y + 6.0, 7.0, rgba(col, 1.0), if open { "v" } else { ">" });
                    let label = format!("{}  |  {n} song{}", sec.label, if n == 1 { "" } else { "s" });
                    c.text(48.0, y + 6.0, 7.0, rgba(col, 1.0), &fit(label, max_chars(7.0, lw - 60.0)));
                }
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
                    // Centred at y + 23.5, in what's left between the title (taller on a text
                    // grid) and the bottom of the box, so the two never touch.
                    let title_bottom = 11.0 + Canvas::text_height(10.0) / 2.0;
                    let room = 2.0 * (23.5 - title_bottom - 1.5).min(rh - 4.0 - 1.5 - 23.5);
                    let sub_size = Canvas::fit_height(7.0, room);
                    c.text(44.0, y + 20.0 + (7.0 - sub_size) / 2.0, sub_size, rgba(0x9a9ab0, 1.0), &fit(sub, max_chars(sub_size, lw - 110.0)));
                    // A newer version on osu!: click (or Ctrl+Shift+U) to get it. Under the
                    // difficulty count (moved up for it), inside the row (not on one cut off at
                    // the list's end).
                    let update = e.set_id().and_then(|id| self.updates.get(&id)).filter(|u| matches!(u, SetUpdate::Available | SetUpdate::Updating) && y + rh - 4.0 <= bottom);
                    let n = g.len();
                    let diffs_y = if update.is_some() { y + 6.0 } else { y + 11.0 };
                    c.text_right(24.0 + lw - 12.0, diffs_y, 8.0, rgba(0x8a8aa0, 1.0), &format!("{n} diff{}", if n == 1 { "" } else { "s" }));
                    let (by, bh) = (y + 19.0, 9.0);
                    let ty = by + (bh - Canvas::text_height(6.0)) / 2.0;
                    match update {
                        Some(SetUpdate::Available) => {
                            let label = "update";
                            let bw = Canvas::text_width(6.0, label) + 10.0;
                            let bx = 24.0 + lw - 12.0 - bw;
                            let hover = c.hovered(bx, by, bw, bh);
                            c.round_rect(bx, by, bw, bh, 3.0, rgba(if hover { 0x5a4a1a } else { 0x3a3218 }, 1.0), rgba(0xFFD34D, 1.0), 1.0);
                            c.text_centered(bx + bw / 2.0, ty, 6.0, rgba(0xFFD34D, 1.0), label);
                            c.hit(bx, by, bw, bh, Hit::UpdateSet(self.visible[g.start]));
                        }
                        Some(SetUpdate::Updating) => c.text_right(24.0 + lw - 12.0, ty, 6.0, rgba(0x4fb4ff, 1.0), "updating..."),
                        _ => {}
                    }
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
        // Not with its heading closed: the heading shows where the selection is instead.
        let oy = outline;
        if !self.visible.is_empty() && !cursor_hidden && oy > top - DIFF_H && oy < bottom {
            c.round_rect(44.0, oy, lw - 20.0, DIFF_H - 3.0, 5.0, rgba(0, 0.0), rgba(accent(), 1.0), 1.5);
        }
        // Rows scrolling past the edges slide under them instead of over the search row.
        c.clip_since(list_start, 20.0, top - 2.0, lw + 8.0, bottom - top + 4.0);
        for h in &mut c.hits[hits_start..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(top - 2.0), (*y + *hh).min(bottom + 2.0));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }
        // The scroll bar (after the list's clip, outside it), in the margin left of the list: where the list is, clicked or dragged
        // to go somewhere (`scroll_to`, like right-dragging).
        if groups.len() > 1 {
            let (track_x, track_w) = (8.0, 8.0);
            let track = bottom - top;
            let hover = c.hovered(track_x - 4.0, top, track_w + 12.0, track);
            c.round_rect(track_x, top, track_w, track, 4.0, rgba(0x15151d, if hover { 0.9 } else { 0.6 }), rgba(0x2a2a38, 1.0), 1.0);
            let total = final_y.max(view);
            let thumb = (track * view / total).clamp(18.0, track);
            let pos = (scroll / (total - view).max(1.0)).clamp(0.0, 1.0);
            c.round_rect(track_x + 1.0, top + pos * (track - thumb), track_w - 2.0, thumb, 3.0, rgba(if hover { accent() } else { 0x8a8aa8 }, 0.95), rgba(0, 0.0), 0.0);
            c.hit(track_x - 4.0, top, track_w + 12.0, track, Hit::SongScrollbar);
        }

        let hints = "Enter play  Up/Down difficulty  Left/Right song  F1 mods  F2 random  [ ] rate  Ctrl+S sort  Ctrl+L collection  Tab settings  Esc quit";
        c.text(24.0, 452.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(7.0, w - 48.0, hints));
        // The status line gives way (smaller) when the hints above come out taller.
        let status_size = Canvas::fit_height(7.0, 2.0 * (469.5 - (455.5 + Canvas::text_height(7.0) / 2.0) - 3.0));
        c.text(24.0, 466.0 + (7.0 - status_size) / 2.0, status_size, rgba(0x55556a, 1.0), &Canvas::fit(status_size, w - 48.0, status));
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

/// The mod select (F1): a column per group, each mod a chip to switch on or off, and a row of
/// the player's favourites. `cursor` is the keyboard selection (index into `mods::ALL`).
pub fn draw_mod_select(c: &mut Canvas, mods: crate::mods::Mods, favourites: crate::mods::Mods, cursor: usize) {
    use crate::mods::{ALL, GROUPS};
    const GOLD: u32 = 0xffd34d;
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
    let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
    c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
    c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "MODS");
    let mult = mods.multiplier();
    let rank = if mods.unranked() { "  (not saved)" } else { "" };
    let score = format!("score x{mult:.2}{rank}");
    c.text_right(px + pw - 16.0, py + 14.0, 8.0, rgba(0xc8c8d8, 1.0), &score);
    let hint = "arrows + Space toggle   F / right-click favourite   R shuffles   Backspace clears   C customize (Multiverse, Shapeshift)   F1 / Esc closes";
    let room = pw - 70.0 - 16.0 - Canvas::text_width(8.0, &score) - 16.0;
    c.text(px + 70.0, py + 16.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, room, hint));
    let col_w = (pw - 24.0) / GROUPS.len() as f32;
    // The favourites' row goes under the longest column.
    let fav_y = py + ph - 76.0;
    let rows = GROUPS.iter().map(|g| g.2.len()).max().unwrap_or(1) as f32;
    let row_h = ((fav_y - 4.0 - (py + 52.0)) / rows).min(27.0);
    let mut hovered = None;
    for (g, (title, accent, list)) in GROUPS.iter().enumerate() {
        let x = px + 12.0 + g as f32 * col_w;
        c.text(x + 4.0, py + 38.0, 7.0, rgba(*accent, 1.0), &Canvas::fit(7.0, col_w - 8.0, title));
        for (r, &m) in list.iter().enumerate() {
            let y = py + 52.0 + r as f32 * row_h;
            let i = ALL.iter().position(|&x| x == m).unwrap_or(0);
            let on = mods.has(m);
            let h = row_h - 4.0;
            let hover = c.hovered(x, y, col_w - 6.0, h);
            if hover {
                hovered = Some(m);
            }
            let bg = if i == cursor { 0x34344a } else if hover { 0x262634 } else { 0x1d1d27 };
            let border = if i == cursor { crate::render::accent() } else if on { *accent } else { 0x2a2a38 };
            c.round_rect(x, y, col_w - 6.0, h, 5.0, rgba(bg, 1.0), rgba(border, 1.0), if i == cursor { 1.5 } else { 1.0 });
            // The acronym's badge keeps room for its letters when there are many rows.
            let inset = ((h - 13.0) / 2.0).clamp(3.0, 4.0);
            c.round_rect(x + 4.0, y + inset, 22.0, h - 2.0 * inset, 3.0,rgba(*accent, if on { 1.0 } else { 0.0 }), rgba(*accent, 1.0), 1.0);
            c.text_centered(x + 15.0, y + h / 2.0 - 3.5, 7.0, rgba(if on { 0x15151d } else { *accent }, 1.0), m.acronym());
            let fav = favourites.has(m);
            if fav {
                c.text(x + col_w - 15.0, y + 3.0, 7.0, rgba(GOLD, 1.0), "*");
            }
            // Narrow columns show just the acronym; the name is in the description below.
            let name = Canvas::fit(6.0, col_w - if fav { 48.0 } else { 40.0 }, m.name());
            c.text(x + 31.0, y + h / 2.0 - 3.0, 6.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), &name);
            c.hit(x, y, col_w - 6.0, h, Hit::ModToggle(i));
        }
    }
    // Favourites: one click away, lit while on.
    let label_w = c.text(px + 16.0, fav_y + 4.5, 7.0, rgba(GOLD, 1.0), "* FAVOURITES");
    let mut fx = px + 28.0 + label_w;
    if favourites.is_empty() {
        c.text(fx, fav_y + 5.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, px + pw - 16.0 - fx, "right-click a mod (or F on it) to keep it here"));
    }
    let favs: Vec<crate::mods::Mod> = favourites.iter().collect();
    for (k, &m) in favs.iter().enumerate() {
        let i = ALL.iter().position(|&x| x == m).unwrap_or(0);
        let col = GROUPS.iter().find(|g| g.2.contains(&m)).map_or(0x9a9ab0, |g| g.1);
        let on = mods.has(m);
        let text = format!("{} {}", m.acronym(), m.name());
        let cw = Canvas::text_width(6.0, &text) + 14.0;
        if fx + cw > px + pw - 16.0 {
            c.text(fx, fav_y + 5.0, 7.0, rgba(0x8a8aa0, 1.0), &format!("+{}", favs.len() - k));
            break;
        }
        if c.hovered(fx, fav_y, cw, 16.0) {
            hovered = Some(m);
        }
        let border = if i == cursor { crate::render::accent() } else { col };
        c.round_rect(fx, fav_y, cw, 16.0, 4.0, rgba(if on { col } else { 0x1d1d27 }, if on { 0.9 } else { 1.0 }), rgba(border, 1.0), 1.0);
        c.text(fx + 7.0, fav_y + 5.0, 6.0, rgba(if on { 0x15151d } else { 0xc8c8d8 }, 1.0), &text);
        c.hit(fx, fav_y, cw, 16.0, Hit::ModToggle(i));
        fx += cw + 5.0;
    }
    // What the pointed-at (else the selected) mod does.
    let shown = hovered.unwrap_or(ALL[cursor.min(ALL.len() - 1)]);
    let dy = py + ph - 46.0;
    c.text(px + 16.0, dy, 8.0, rgba(0xffffff, 1.0), &format!("{} ({})", shown.name(), shown.acronym()));
    c.text(px + 16.0, dy + 13.0, 7.0, rgba(0x9a9ab0, 1.0), shown.description());
    let bw = 60.0;
    let active = if mods.is_empty() { "none".to_string() } else { mods.iter().map(|m| m.acronym()).collect::<Vec<_>>().join(" ") };
    c.text(px + 16.0, py + ph - 16.0, 7.0, rgba(0xc8c8d8, 1.0), &Canvas::fit(7.0, pw - 4.0 * bw - 64.0, &format!("on: {active}")));
    c.button(px + pw - bw - 12.0, py + ph - 24.0, bw, 16.0, "Close", 7.0, Hit::Key(KeyCode::F1), true);
    c.button(px + pw - 2.0 * bw - 18.0, py + ph - 24.0, bw, 16.0, "Clear", 7.0, Hit::Key(KeyCode::Backspace), false);
    c.button(px + pw - 3.0 * bw - 24.0, py + ph - 24.0, bw, 16.0, "Shuffle", 7.0, Hit::Key(KeyCode::KeyR), false);
    // The settings of the mods that have some (Multiverse's universes, Shapeshift's shapes),
    // only while one of them is on.
    if !crate::mods::customizable(mods).is_empty() {
        c.button(px + pw - 4.0 * bw - 30.0, py + ph - 24.0, bw, 16.0, "Customize", 7.0, Hit::Key(KeyCode::KeyC), true);
    }
}

/// The mod customization's tabs, above its panel: one per customizable mod that's on.
pub fn draw_customization_tabs(c: &mut Canvas, tabs: &[crate::mods::Mod], open: crate::mods::Mod) {
    let (y, h) = (20.0, 18.0);
    let mut x = 16.0 + c.text(16.0, y + 4.0, 9.0, rgba(0xffffff, 1.0), "MOD CUSTOMIZATION") + 12.0;
    for (i, &m) in tabs.iter().enumerate() {
        let name = m.name().to_uppercase();
        let tw = Canvas::text_width(7.0, &name) + 20.0;
        let on = m == open;
        c.round_rect(x, y, tw, h, 5.0, rgba(if on { 0x34344a } else { 0x1d1d27 }, 1.0), rgba(if on { accent() } else { 0x3a3a4e }, 1.0), if on { 1.5 } else { 1.0 });
        c.text_centered(x + tw / 2.0, y + 5.5, 7.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), &name);
        c.hit(x, y, tw, h, Hit::CustomTab(i));
        x += tw + 6.0;
    }
    if tabs.len() > 1 {
        c.text(x + 4.0, y + 6.0, 6.0, rgba(0x6f6f86, 1.0), "Tab: next");
    }
}

/// Maps hidden from the song list in taiko.rs only (osu! and the Songs folder are untouched):
/// whole songs by folder, single difficulties by file. Kept in `hidden.tsv` in the config folder.
#[derive(Default)]
pub struct Hidden {
    pub songs: std::collections::HashSet<std::path::PathBuf>,
    pub maps: std::collections::HashSet<std::path::PathBuf>,
}

impl Hidden {
    fn file() -> Option<std::path::PathBuf> {
        crate::config::data_dir().map(|d| d.join("hidden.tsv"))
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
    /// Loved nominators: love the map set on the online server (false: take it back).
    Love(u32, bool),
    /// Download the song's set again, its newest version (replacing its difficulties).
    UpdateSong,
}

/// Whether osu! has a newer version of a set than the one here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetUpdate {
    Checking,
    UpToDate,
    Available,
    Updating,
    /// The mirrors couldn't be asked.
    Unknown,
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
        c.round_rect(x, y, bw, bh, 6.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
        for (k, (label, _)) in items.iter().enumerate() {
            let ry = y + 6.0 + k as f32 * ROW;
            if c.hovered(x + 4.0, ry, bw - 8.0, ROW - 2.0) {
                c.round_rect(x + 4.0, ry, bw - 8.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(accent(), 1.0), 1.0);
            }
            c.hit(x + 4.0, ry, bw - 8.0, ROW - 2.0, Hit::SongMenu(k));
            c.text(x + 12.0, ry + 5.0, 7.0, rgba(0xffffff, 1.0), &Canvas::fit(7.0, bw - 20.0, label));
        }
        c.text(x + 12.0, y + bh - 14.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, bw - 24.0, "hiding: only in taiko.rs; files stay"));
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
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
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
                c.round_rect(bx + 8.0, y, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(if selected { accent() } else { 0x3a3a4e }, 1.0), 1.0);
            }
            c.hit(bx + 8.0, y, bw - 16.0, ROW, Hit::PickRow(i));
            let shown_w = if current { Canvas::text_width(7.0, "shown") + 10.0 } else { 0.0 };
            c.text(bx + 20.0, y + 5.0, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &Canvas::fit(7.0, bw - 40.0 - shown_w, label));
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

/// What a results screen works out in the background: a leaderboard score's whole stats (its
/// replay played through), the pp breakdown, and an online score's replay.
#[derive(Default)]
pub struct Detail {
    pub stats: Option<(Stats, Windows)>,
    pub breakdown: Option<crate::pp::Breakdown>,
    pub replay: Option<Result<std::path::PathBuf, String>>,
}

/// The player's online pp total after a play, and what it was before (unknown: no change
/// shown).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Total {
    pub before: Option<f64>,
    pub after: f64,
    /// Their place by pp.
    pub rank: Option<u32>,
}

/// A score opened from a leaderboard (not one just played).
pub struct Opened {
    pub player: String,
    /// Unix seconds.
    pub date: u64,
    pub online: bool,
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
    /// The play's performance points (kept plays only).
    pub pp: Option<f64>,
    /// The pp taken apart, like osu!'s performance breakdown (once worked out).
    pub breakdown: Option<crate::pp::Breakdown>,
    /// Why the play has no pp, said in the pp panel.
    pub no_pp: Option<&'static str>,
    /// Opened from a leaderboard: whose score it is.
    pub opened: Option<Opened>,
    /// Hit errors, drumrolls and big notes aren't known yet (a leaderboard score, until its
    /// replay has been played through).
    pub partial: bool,
    /// The song plays on, like osu!: the play's track, and the track ms it starts over at.
    pub music: Option<(std::sync::Arc<crate::audio::Track>, f64)>,
    pub job: Option<std::sync::mpsc::Receiver<Detail>>,
    /// Why an online score's replay couldn't be had.
    pub replay_error: Option<String>,
    /// What the play did to the player's online pp total, and whether the server is yet to say.
    pub total: Option<Total>,
    pub total_due: bool,
    /// How far the page is scrolled down (toward the pp), and where it's going.
    pub scroll: f32,
    pub scroll_to: f32,
    /// The mods played with, and the speed when it isn't theirs (DT / HT show as mods).
    pub mods: crate::mods::Mods,
    pub rate: Option<f64>,
}

/// How far down the results scroll: the pp start a screen below them.
pub const RESULTS_SCROLL: f32 = 456.0;
/// The bottom of what scrolls (the buttons stay put under it).
const RESULTS_VIEW: f32 = 416.0;

impl Results {
    /// A results screen with nothing but the essentials filled in.
    pub fn new(title: String, stats: Stats, windows: Windows) -> Results {
        Results {
            shown: std::time::Instant::now(),
            title,
            stats,
            windows,
            replay: None,
            rank: None,
            watched: None,
            failed: false,
            pp: None,
            breakdown: None,
            no_pp: None,
            opened: None,
            partial: false,
            music: None,
            job: None,
            replay_error: None,
            total: None,
            total_due: false,
            scroll: 0.0,
            scroll_to: 0.0,
            mods: crate::mods::Mods::NONE,
            rate: None,
        }
    }

    /// Takes in what the background work found.
    pub fn poll(&mut self) {
        let Some(rx) = &self.job else { return };
        let d = match rx.try_recv() {
            Ok(d) => d,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                return;
            }
        };
        self.job = None;
        if let Some((st, windows)) = d.stats {
            // The counts stay the leaderboard's; the replay adds what it doesn't keep.
            let s = &mut self.stats;
            (s.errors, s.error_offsets, s.roll_ticks, s.swells, s.strong) = (st.errors, st.error_offsets, st.roll_ticks, st.swells, st.strong);
            self.windows = windows;
            self.partial = false;
        }
        if let Some(b) = d.breakdown {
            self.breakdown = Some(b);
            self.pp.get_or_insert(b.pp);
        }
        match d.replay {
            Some(Ok(file)) => self.replay = Some(file),
            Some(Err(e)) => self.replay_error = Some(e),
            None => {}
        }
    }

    /// The mods a play had and its speed (shown on its own when the mods don't set it).
    pub fn set_mods(&mut self, mods: crate::mods::Mods, rate: f64) {
        self.mods = mods;
        self.rate = ((rate - 1.0).abs() > 1e-6 && mods.rate_override().is_none()).then_some(rate);
    }

    /// Scrolls all the way down to the pp (`down`) or back up to the results: there's no
    /// stopping in between.
    pub fn scroll(&mut self, down: bool) {
        self.scroll_to = if down { RESULTS_SCROLL } else { 0.0 };
    }

    /// Slides the page there, quickly.
    pub fn animate(&mut self, dt_ms: f32) {
        crate::render::approach(&mut self.scroll, self.scroll_to, dt_ms, 30.0);
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        let cx = w / 2.0;
        // The page scrolls under the buttons: the results, and a screen below them the pp.
        let page = c.shapes.len();
        let size = Canvas::fit_size(12.0, 9.0, w - 48.0, &self.title);
        // Above the friends chip (top right, from y 41).
        c.text_centered(cx, 24.0 + (12.0 - size) / 2.0, size, rgba(0x9a9ab0, 1.0), &Canvas::fit(size, w - 48.0, &self.title));
        let s = &self.stats;
        let acc = s.accuracy() * 100.0;
        let grade = crate::online::grade(s.great, s.ok, s.miss, s.accuracy());
        // A failed play gets F, and says so.
        let grade = if self.failed { "F" } else { grade };
        let count = progress(self.shown, 700.0);
        let pop = progress(self.shown + std::time::Duration::from_millis(450), 300.0);
        if pop > 0.0 {
            let size = 70.0 * (1.6 - 0.6 * pop);
            let col = if self.failed { 0xEB452C } else { 0xFFD34D };
            c.text_centered(cx, 80.0 + (70.0 - size) / 2.0, size, rgba(col, pop), grade);
        }
        self.draw_mods(c, cx, 44.0);
        c.text_centered(cx, 170.0, 28.0, rgba(0xffffff, 1.0), &format!("{:.2}%", acc * count as f64));
        c.text_centered(cx, 212.0, 16.0, rgba(0xc8c8d8, 1.0), &format!("{:08}", (s.score as f64 * count as f64) as u64));
        if let Some(pp) = self.pp {
            c.text_centered(cx, 236.0, 10.0, rgba(0xff8ad8, 1.0), &format!("{:.0} pp", pp * count as f64));
        }

        let lines = [
            (0xFFD34D, format!("GREAT  {}", s.great)),
            (0x86E05A, format!("OK     {}", s.ok)),
            (0xFF4B4B, format!("MISS   {}", s.miss)),
            (0xffffff, format!("MAX COMBO  {}", s.max_combo)),
        ];
        for (i, (col, text)) in lines.iter().enumerate() {
            c.text(cx - 170.0, 260.0 + i as f32 * 24.0, 12.0, rgba(*col, 1.0), text);
        }
        let right = if self.partial {
            ["UR      -".to_string(), "MEAN    -".into(), "DRUMROLL -".into(), "BIG x2  -".into()]
        } else {
            [
                format!("UR      {:.1}", s.unstable_rate()),
                format!("MEAN    {:+.1} ms", s.mean_error()),
                format!("DRUMROLL {}", s.roll_ticks),
                format!("BIG x2  {}", s.strong),
            ]
        };
        for (i, text) in right.iter().enumerate() {
            c.text(cx + 30.0, 260.0 + i as f32 * 24.0, 12.0, rgba(0xc8c8d8, 1.0), text);
        }
        let hint = if let Some(o) = &self.opened {
            let place = if o.online { "online" } else { "local" };
            format!("{place} score by {}, {} ago", o.player, crate::scores::age(o.date))
        } else if s.errors.len() >= 20 && s.mean_error().abs() > 5.0 {
            let dir = if s.mean_error() > 0.0 { "late" } else { "early" };
            format!("You hit {:.0} ms {dir} on average; press {} a few times to adjust this map's offset.", s.mean_error().abs(), if s.mean_error() > 0.0 { "=" } else { "-" })
        } else {
            format!("Windows: great +-{:.0}  ok +-{:.0} ms", self.windows.great, self.windows.ok)
        };
        c.text_centered(cx, 380.0, 9.0, rgba(0x8a8aa0, 1.0), &Canvas::fit(9.0, w - 48.0, &hint));
        if let Some(e) = &self.replay_error {
            c.text_centered(cx, 400.0, 9.0, rgba(0xEB452C, 1.0), &Canvas::fit(9.0, w - 48.0, &format!("no replay: {e}")));
        } else if self.opened.as_ref().is_some_and(|o| o.online) && self.replay.is_none() {
            c.text_centered(cx, 400.0, 9.0, rgba(0x4fb4ff, 1.0), "getting the replay...");
        } else if let Some(who) = &self.watched {
            c.text_centered(cx, 400.0, 9.0, rgba(0x4fb4ff, 1.0), &format!("replay of {who}"));
        } else if let Some((rank, of)) = self.rank {
            c.text_centered(cx, 400.0, 9.0, rgba(0xFFD34D, 1.0), &format!("#{rank} of {of} local scores on this map"));
        }
        self.draw_performance(c, count as f64);
        c.transform_since(page, [0.0, 0.0], 1.0, 0.0, -self.scroll);
        c.clip_since(page, 0.0, 0.0, w, RESULTS_VIEW);

        let mut buttons = Vec::new();
        if self.opened.is_none() {
            buttons.push(("Retry  (Enter)", Hit::Key(KeyCode::Enter), true));
        }
        if self.replay.is_some() {
            buttons.push(("Watch replay  (R)", Hit::Key(KeyCode::KeyR), self.opened.is_some()));
            buttons.push(("Export video  (V)", Hit::Key(KeyCode::KeyV), false));
        }
        let back = if self.opened.is_some() { "Back  (Esc)" } else { "Song select  (Esc)" };
        buttons.push((back, Hit::Key(KeyCode::Escape), false));
        c.button_row(424.0, &buttons);
        // Where the pp went: one click (or the wheel) away, and back.
        // Where the pp went: a pill under the buttons, its arrows bobbing the way to go.
        let down = self.scroll_to < RESULTS_SCROLL / 2.0;
        let (cue, key) = match (down, self.pp) {
            (true, Some(pp)) => (format!("scroll down: {pp:.0} pp breakdown"), KeyCode::PageDown),
            (true, None) => ("scroll down: performance".to_string(), KeyCode::PageDown),
            (false, _) => ("scroll up: results".to_string(), KeyCode::PageUp),
        };
        let a = progress(self.shown + std::time::Duration::from_millis(600), 300.0);
        let (cw, ch, cy) = (Canvas::text_width(8.0, &cue) + 48.0, 18.0, 450.0);
        let x0 = cx - cw / 2.0;
        let hover = c.hovered(x0, cy, cw, ch);
        let pink = 0xff8ad8;
        c.round_rect(x0, cy, cw, ch, ch / 2.0, rgba(if hover { 0x2b1f2a } else { 0x1d1d27 }, 0.95 * a), rgba(pink, if hover { a } else { 0.75 * a }), 1.5);
        c.text_centered(cx, cy + (ch - 8.0) / 2.0, 8.0, rgba(if hover { 0xffffff } else { pink }, a), &cue);
        let t = self.shown.elapsed().as_secs_f32();
        let bob = 1.5 * (t * 4.0).sin() * if down { 1.0 } else { -1.0 };
        for ax in [x0 + 14.0, x0 + cw - 14.0] {
            chevron(c, ax, cy + ch / 2.0 + bob, down, rgba(pink, a));
        }
        c.hit(x0, cy, cw, ch, Hit::Key(key));
    }

    /// The mods (and a speed of the player's own) as badges under the title, each in its mod
    /// select column's colour: up to two rows centred on `cx` from `y`, the rest counted.
    fn draw_mods(&self, c: &mut Canvas, cx: f32, y: f32) {
        let colour = |m: crate::mods::Mod| crate::mods::GROUPS.iter().find(|g| g.2.contains(&m)).map_or(0xc8c8d8, |g| g.1);
        // A failed play says so first, in red.
        let failed = self.failed.then(|| ("FAILED".to_string(), 0xEB452C));
        let mut chips: Vec<(String, u32)> = failed.into_iter().chain(self.mods.iter().map(|m| (m.acronym().to_string(), colour(m)))).collect();
        if let Some(rate) = self.rate {
            chips.push((format!("{rate:.2}x"), 0xc8c8d8));
        }
        if chips.is_empty() {
            return;
        }
        const SIZE: f32 = 8.0;
        const H: f32 = 14.0;
        const GAP: f32 = 4.0;
        // Clear of the friends chip in the top right (and the same on the left, to stay centred).
        let room = (c.width - 400.0).max(160.0);
        let width = |label: &str| Canvas::text_width(SIZE, label) + 12.0;
        // Rows as wide as fit; past two, the last badge says how many more.
        let mut rows: Vec<Vec<(String, u32)>> = vec![Vec::new()];
        let mut used = 0.0;
        for chip in &chips {
            let w = width(&chip.0);
            if used > 0.0 && used + GAP + w > room {
                if rows.len() == 2 {
                    // Room for the count of the rest (which grows with each badge given up).
                    loop {
                        let shown: usize = rows.iter().map(Vec::len).sum();
                        let more = format!("+{}", chips.len() - shown);
                        let last = rows.last_mut().unwrap();
                        if last.is_empty() || last.iter().map(|(l, _)| width(l) + GAP).sum::<f32>() + width(&more) <= room {
                            last.push((more, 0x8a8aa0));
                            break;
                        }
                        last.pop();
                    }
                    break;
                }
                rows.push(Vec::new());
                used = 0.0;
            }
            used += if used > 0.0 { GAP } else { 0.0 } + w;
            rows.last_mut().unwrap().push(chip.clone());
        }
        for (r, row) in rows.iter().enumerate() {
            let total = row.iter().map(|(l, _)| width(l)).sum::<f32>() + GAP * row.len().saturating_sub(1) as f32;
            let (mut x, ry) = (cx - total / 2.0, y + r as f32 * (H + 3.0));
            for (label, col) in row {
                let w = width(label);
                c.round_rect(x, ry, w, H, 4.0, rgba(*col, 0.2), rgba(*col, 0.9), 1.0);
                c.text_centered(x + w / 2.0, ry + (H - SIZE) / 2.0, SIZE, rgba(*col, 1.0), label);
                x += w + GAP;
            }
        }
    }

    /// The pp, a screen below the results, as osu!'s performance breakdown: the pp, the
    /// difficulty and accuracy parts measured against an SS's, what a full combo and an SS would
    /// give, and what the play did to the player's online total. `count` (0-1) counts it up.
    fn draw_performance(&self, c: &mut Canvas, count: f64) {
        let w = c.width;
        let pw = (w - 48.0).min(620.0);
        // Centred between the friends chip (top right, down to y 54) and the buttons.
        let h = if self.breakdown.is_some() { 200.0 } else { 80.0 };
        let (x, y) = ((w - pw) / 2.0, RESULTS_SCROLL + 64.0 + ((RESULTS_VIEW - 64.0 - h) / 2.0).max(0.0));
        let (label, value) = (rgba(0x8a8aa0, 1.0), rgba(0xffffff, 1.0));
        let Some(b) = self.breakdown else {
            c.round_rect(x, y, pw, h, 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
            c.text(x + 16.0, y + 12.0, 8.0, label, "PERFORMANCE");
            let note = if self.job.is_some() { "working it out..." } else { self.no_pp.unwrap_or("no pp for this play") };
            c.text_centered(x + pw / 2.0, y + 44.0, 9.0, label, &Canvas::fit(9.0, pw - 32.0, note));
            return;
        };
        c.round_rect(x, y, pw, h, 8.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
        c.text(x + 16.0, y + 12.0, 8.0, label, "PERFORMANCE");
        crate::stars::draw_pill(c, x + pw - 14.0, y + 9.0, 8.0, Some(b.stars as f32));
        // Two columns: the pp and its parts, then what could have been and the total.
        let col_w = (pw - 48.0) / 2.0;
        let (lx, rx) = (x + 16.0, x + 32.0 + col_w);
        let big = c.text(lx, y + 34.0, 26.0, rgba(0xff8ad8, 1.0), &format!("{:.0}", b.pp * count));
        c.text(lx + 6.0 + big, y + 49.0, 10.0, rgba(0xff8ad8, 1.0), "pp");
        let of_ss = if b.if_ss > 0.0 { b.pp / b.if_ss * 100.0 } else { 0.0 };
        c.text(lx, y + 72.0, 7.0, label, &format!("{of_ss:.0}% of an SS"));
        for (i, (name, v, max)) in [("Difficulty", b.difficulty, b.ss_difficulty), ("Accuracy", b.accuracy, b.ss_accuracy)].into_iter().enumerate() {
            let by = y + 100.0 + i as f32 * 40.0;
            c.text(lx, by, 8.0, rgba(0xc8c8d8, 1.0), name);
            c.text_right(lx + col_w, by, 8.0, value, &format!("{:.0}", v * count));
            c.round_rect(lx, by + 15.0, col_w, 6.0, 3.0, rgba(0x2a2a38, 1.0), rgba(0, 0.0), 0.0);
            let f = if max > 0.0 { ((v / max).clamp(0.0, 1.0) * count) as f32 } else { 0.0 };
            if f > 0.0 {
                c.round_rect(lx, by + 15.0, (col_w * f).max(6.0), 6.0, 3.0, rgba(accent(), 1.0), rgba(0, 0.0), 0.0);
            }
        }
        let mut rows = vec![
            ("Full combo", format!("{:.0} pp", b.if_fc), 0xffffff),
            ("SS", format!("{:.0} pp", b.if_ss), 0xffffff),
            ("Combo", format!("{} / {}", self.stats.max_combo, b.max_combo), 0xffffff),
        ];
        // What it did to the online total (sent plays only).
        match self.total {
            Some(t) => {
                let (change, col) = match t.before.map(|b| t.after - b) {
                    Some(g) if g >= 0.05 => (format!("+{g:.1} pp"), 0x86E05A),
                    Some(_) => ("none".to_string(), 0x8a8aa0),
                    None => ("-".to_string(), 0x8a8aa0),
                };
                rows.push(("Total", format!("{:.0} pp", t.after), 0xffffff));
                rows.push(("Change", change, col));
                rows.push(("Rank", t.rank.map(|r| format!("#{r}")).unwrap_or_else(|| "-".into()), 0xc8c8d8));
            }
            None if self.total_due => rows.push(("Total", "...".to_string(), 0x8a8aa0)),
            None => {}
        }
        for (i, (name, v, col)) in rows.iter().enumerate() {
            // The total under a line of its own.
            let ry = y + 36.0 + i as f32 * 24.0 + if i >= 3 { 12.0 } else { 0.0 };
            if i == 3 {
                c.rect(rx, ry - 13.0, col_w, 1.0, rgba(0x2a2a38, 1.0));
            }
            c.text(rx, ry, 8.0, label, name);
            c.text_right(rx + col_w, ry, 8.0, rgba(*col, 1.0), v);
        }
    }
}

/// A small arrow head at (`x`, `y`), pointing down (`down`) or up.
fn chevron(c: &mut Canvas, x: f32, y: f32, down: bool, col: [f32; 4]) {
    let tip = if down { -1.0 } else { 1.0 };
    for side in [-1.0f32, 1.0] {
        c.rect(x + side * 2.0 - 3.0, y - 0.9, 6.0, 1.8, col);
        if let Some(sh) = c.shapes.last_mut() {
            sh.angle = side * tip * std::f32::consts::FRAC_PI_4;
        }
    }
}
