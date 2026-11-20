//! Song select's lens: which maps the list shows (the source tabs over it and the filters) and
//! how it lists them (the sort heading it, the order under each heading, headings or one list).
//! Each source keeps a lens of its own (`Tabs`): a TJA pack is best listed by its folders, osu!'s
//! songs the way the player always had them. The panel that changes it is drawn over the list
//! (`Select::draw_lens`).

use std::time::Instant;

use taiko_logic::tja::CourseKind;

use super::{Select, SortMode};
use crate::hit::{Canvas, Hit};
use crate::library::Entry;
use crate::render::{progress, rgba};
use crate::search::{self, Facts};

/// Where a map comes from: song select's tabs over its list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Source {
    #[default]
    All,
    Osu,
    Tja,
    Own,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::All, Source::Osu, Source::Tja, Source::Own];

    pub fn index(self) -> usize {
        self as usize
    }

    /// As the config writes it.
    pub fn name(self) -> &'static str {
        match self {
            Source::All => "all",
            Source::Osu => "osu",
            Source::Tja => "tja",
            Source::Own => "own",
        }
    }

    pub fn parse(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|x| x.name() == s.trim())
    }

    /// On its tab.
    pub fn label(self) -> &'static str {
        match self {
            Source::All => "All",
            Source::Osu => "osu!",
            Source::Tja => "TJA",
            Source::Own => "taiko.rs",
        }
    }

    pub fn colour(self) -> u32 {
        match self {
            Source::All => 0xc8c8d8,
            Source::Osu => 0xff66aa,
            Source::Tja => 0xff7028,
            Source::Own => 0x9b6cff,
        }
    }

    /// Where map `e` comes from: a TJA course, taiko.rs's own maps folder, else osu!'s Songs.
    pub fn of(e: &Entry) -> Source {
        if e.is_tja() {
            Source::Tja
        } else if e.is_own() {
            Source::Own
        } else {
            Source::Osu
        }
    }
}

/// How the songs under each heading are ordered (all of them without headings).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Order {
    /// As the sort puts them.
    #[default]
    Sort,
    Title,
    Artist,
    Stars,
    Length,
    Bpm,
    Added,
    Played,
}

impl Order {
    pub const ALL: [Order; 8] = [
        Order::Sort,
        Order::Title,
        Order::Artist,
        Order::Stars,
        Order::Length,
        Order::Bpm,
        Order::Added,
        Order::Played,
    ];

    /// As the config writes it.
    pub fn name(self) -> &'static str {
        match self {
            Order::Sort => "sort",
            Order::Title => "title",
            Order::Artist => "artist",
            Order::Stars => "stars",
            Order::Length => "length",
            Order::Bpm => "bpm",
            Order::Added => "added",
            Order::Played => "played",
        }
    }

    pub fn parse(s: &str) -> Option<Order> {
        Order::ALL.into_iter().find(|o| o.name() == s.trim())
    }

    pub fn label(self) -> &'static str {
        match self {
            Order::Sort => "as sorted",
            Order::Title => "title",
            Order::Artist => "artist",
            Order::Stars => "stars",
            Order::Length => "length",
            Order::Bpm => "BPM",
            Order::Added => "date added",
            Order::Played => "last played",
        }
    }

    /// Orders that wait on the background difficulty / length / BPM analysis.
    pub fn needs_info(self) -> bool {
        matches!(self, Order::Stars | Order::Length | Order::Bpm)
    }
}

/// How a source's songs are listed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lens {
    pub sort: SortMode,
    pub order: Order,
    /// The order the other way round.
    pub desc: bool,
    pub headings: bool,
}

impl Lens {
    /// The game's own (and a source's until the player changes it): one list by artist.
    pub const PLAIN: Lens = Lens {
        sort: SortMode::Artist,
        order: Order::Sort,
        desc: false,
        headings: false,
    };

    /// What a source starts with: TJA charts under their pack's folders, taiko.rs's maps by
    /// status (custom, approved, sent to osu!), the rest as the player lists everything.
    pub fn first(source: Source, all: Lens) -> Lens {
        match source {
            Source::Tja => Lens {
                sort: SortMode::Folder,
                headings: true,
                ..Lens::PLAIN
            },
            Source::Own => Lens {
                sort: SortMode::Status,
                headings: true,
                ..Lens::PLAIN
            },
            Source::All | Source::Osu => all,
        }
    }

    /// As the config writes it: `folder title asc headings`.
    pub fn text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.sort.name().replace(' ', "_"),
            self.order.name(),
            if self.desc { "desc" } else { "asc" },
            if self.headings { "headings" } else { "flat" }
        )
    }

    pub fn parse(s: &str) -> Option<Lens> {
        let mut words = s.split_whitespace();
        let sort = SortMode::parse(&words.next()?.replace('_', " "))?;
        let order = Order::parse(words.next()?)?;
        let desc = match words.next()? {
            "desc" => true,
            "asc" => false,
            _ => return None,
        };
        let headings = match words.next()? {
            "headings" => true,
            "flat" => false,
            _ => return None,
        };
        Some(Lens {
            sort,
            order,
            desc,
            headings,
        })
    }
}

/// The length filter's spans: a name (in minutes, as its caption says) and where each ends
/// (seconds).
pub const LENGTHS: [(&str, f64); 4] = [
    ("< 1.5", 90.0),
    ("1.5-3", 180.0),
    ("3-5", 300.0),
    ("5+", f64::INFINITY),
];

/// The status filter's choices: a name, its colour (the status headings') and the statuses in it.
pub const STATUSES: [(&str, u32, &[u8]); 5] = [
    (
        "Ranked",
        0x6ecb63,
        &[search::RANKED, search::APPROVED, search::QUALIFIED],
    ),
    ("Loved", 0xff66aa, &[search::LOVED]),
    ("Pending", 0xb0a06a, &[search::PENDING, search::GRAVEYARD]),
    ("Local", 0x8a8aa0, &[search::NOT_SUBMITTED, search::UNKNOWN]),
    (
        "Custom",
        0x9b6cff,
        &[search::CUSTOM, search::CUSTOM_APPROVED],
    ),
];

/// A TJA chart's courses, as the course filter and the song rows show them, with the arcade's
/// colours.
pub const COURSES: [(&str, u32, CourseKind); 5] = [
    ("Easy", 0xff6a3d, CourseKind::Easy),
    ("Normal", 0x8fd14f, CourseKind::Normal),
    ("Hard", 0x3fb8e8, CourseKind::Hard),
    ("Oni", 0xe8467c, CourseKind::Oni),
    ("Ura", 0x9b5cf0, CourseKind::Edit),
];

/// The colour of a TJA course.
pub fn course_colour(kind: CourseKind) -> u32 {
    COURSES
        .iter()
        .find(|c| c.2 == kind)
        .map_or(0x8a8aa0, |c| c.1)
}

/// The star filter's bar a rating is on: 0 for anything under 2 stars, 9 for 10 and up.
pub fn star_bin(stars: f64) -> usize {
    (stars.floor() as i64).clamp(1, 10) as usize - 1
}

fn length_bin(seconds: f64) -> usize {
    LENGTHS
        .iter()
        .position(|&(_, end)| seconds < end)
        .unwrap_or(LENGTHS.len() - 1)
}

/// What the filters keep. Each set of choices keeps what any of its choices takes; a map whose
/// value isn't known (yet) isn't kept by a filter on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Filters {
    /// The first and last star bar kept (`star_bin`).
    pub stars: Option<(u8, u8)>,
    /// `LENGTHS`, `STATUSES` and `COURSES` kept, a bit each.
    pub lengths: u8,
    pub statuses: u8,
    pub courses: u8,
    pub unplayed: bool,
}

/// A filter as one, for its token in the search box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Stars,
    Length,
    Status,
    Course,
    Unplayed,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Filters::default()
    }

    /// The filters that mean something under `tab`: the course filter only among TJA charts,
    /// the status filter (osu!'s and taiko.rs's) everywhere else. The others are kept for when
    /// the tab they're for is back, but don't filter, nor show as tokens.
    pub fn active_in(&self, tab: Source) -> Filters {
        let mut f = *self;
        if tab == Source::Tja {
            f.statuses = 0;
        } else {
            f.courses = 0;
        }
        f
    }

    /// Whether a map with `facts` (and `course`, a TJA chart's) is kept; `skip` leaves one
    /// filter out (what that filter's own choices count).
    pub fn keeps(&self, facts: &Facts, course: Option<CourseKind>, skip: Option<Token>) -> bool {
        if let (true, Some((lo, hi))) = (skip != Some(Token::Stars), self.stars) {
            match facts.stars {
                Some(s) if (lo..=hi).contains(&(star_bin(s) as u8)) => {}
                _ => return false,
            }
        }
        if self.lengths != 0 && skip != Some(Token::Length) {
            match facts.length_s {
                Some(l) if self.lengths & 1 << length_bin(l) != 0 => {}
                _ => return false,
            }
        }
        if self.statuses != 0
            && skip != Some(Token::Status)
            && !STATUSES
                .iter()
                .enumerate()
                .any(|(k, s)| self.statuses & 1 << k != 0 && s.2.contains(&facts.status))
        {
            return false;
        }
        if self.courses != 0 && skip != Some(Token::Course) {
            match course.and_then(|c| COURSES.iter().position(|x| x.2 == c)) {
                Some(k) if self.courses & 1 << k != 0 => {}
                _ => return false,
            }
        }
        !(self.unplayed && skip != Some(Token::Unplayed) && facts.played_days.is_some())
    }

    /// Whether they need the background analysis (stars of osu!'s maps, lengths).
    pub fn needs_info(&self) -> bool {
        self.stars.is_some() || self.lengths != 0
    }

    /// The filters on, each as its token: what it is, its text and its colour.
    pub fn tokens(&self) -> Vec<(Token, String, u32)> {
        let mut out = Vec::new();
        let names = |bits: u8, names: &mut dyn Iterator<Item = &'static str>| {
            names
                .enumerate()
                .filter(|(k, _)| bits & 1 << k != 0)
                .map(|(_, n)| n)
                .collect::<Vec<_>>()
                .join(" ")
        };
        if let Some((lo, hi)) = self.stars {
            let text = stars_text(Some((lo, hi)));
            let colour = crate::stars::color(lo as f32 + 1.5);
            out.push((Token::Stars, format!("stars {text}"), colour));
        }
        if self.lengths != 0 {
            let text = names(self.lengths, &mut LENGTHS.iter().map(|l| l.0));
            out.push((Token::Length, format!("{text} min"), 0xc8c8d8));
        }
        if self.statuses != 0 {
            let text = names(self.statuses, &mut STATUSES.iter().map(|s| s.0));
            let first = STATUSES[self.statuses.trailing_zeros() as usize].1;
            out.push((Token::Status, text, first));
        }
        if self.courses != 0 {
            let text = names(self.courses, &mut COURSES.iter().map(|c| c.0));
            let first = COURSES[self.courses.trailing_zeros() as usize].1;
            out.push((Token::Course, text, first));
        }
        if self.unplayed {
            out.push((Token::Unplayed, "not played".to_string(), 0xc8c8d8));
        }
        out
    }

    pub fn clear(&mut self, token: Token) {
        match token {
            Token::Stars => self.stars = None,
            Token::Length => self.lengths = 0,
            Token::Status => self.statuses = 0,
            Token::Course => self.courses = 0,
            Token::Unplayed => self.unplayed = false,
        }
    }

    /// A click on star bar `b`: just that bar; past the ones kept, out to it; on the one bar
    /// kept, any again.
    pub fn click_star(&mut self, b: u8) {
        self.stars = match self.stars {
            Some((lo, hi)) if lo == hi && lo == b => None,
            Some((lo, hi)) if b < lo => Some((b, hi)),
            Some((lo, hi)) if b > hi => Some((lo, b)),
            _ => Some((b, b)),
        };
    }
}

/// The star bars kept, as the token and the panel say it: `7`, `5-8`, `8+`, `any`.
pub fn stars_text(stars: Option<(u8, u8)>) -> String {
    let bar = |b: u8| match b {
        9 => "10+".to_string(),
        b => (b + 1).to_string(),
    };
    match stars {
        None | Some((0, 9)) => "any".to_string(),
        Some((lo, hi)) if lo == hi => bar(lo),
        Some((lo, 9)) => format!("{}+", lo + 1),
        Some((lo, hi)) => format!("{}-{}", bar(lo), hi + 1),
    }
}

/// The `STATUSES` choice a status code is under.
pub fn status_choice(status: u8) -> Option<usize> {
    STATUSES.iter().position(|s| s.2.contains(&status))
}

/// What each filter's choices would list, the other filters as they are: the star bars, and the
/// counts on the length, status and course blocks. Counted while the panel is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Facets {
    pub stars: [u32; 10],
    pub lengths: [u32; 4],
    pub statuses: [u32; 5],
    pub courses: [u32; 5],
    /// The statuses some map of the tab has, whatever the search and filters: the status
    /// choices the panel offers.
    pub has_status: u8,
}

impl Facets {
    /// Counts a map with `facts` under the filters `active`.
    pub fn count(&mut self, active: &Filters, facts: &Facts, course: Option<CourseKind>) {
        if let Some(s) = facts.stars
            && active.keeps(facts, course, Some(Token::Stars))
        {
            self.stars[star_bin(s)] += 1;
        }
        if let Some(l) = facts.length_s
            && active.keeps(facts, course, Some(Token::Length))
        {
            self.lengths[length_bin(l)] += 1;
        }
        if let Some(k) = status_choice(facts.status)
            && active.keeps(facts, course, Some(Token::Status))
        {
            self.statuses[k] += 1;
        }
        if let Some(k) = course.and_then(|c| COURSES.iter().position(|x| x.2 == c))
            && active.keeps(facts, course, Some(Token::Course))
        {
            self.courses[k] += 1;
        }
    }
}

/// The source shown, each source's lens, and the filters (one set for every source).
#[derive(Clone, PartialEq, Debug)]
pub struct Tabs {
    pub source: Source,
    /// The shown source's order (its sort and headings are `Select`'s).
    pub order: Order,
    pub desc: bool,
    /// The same whatever the source: a switch keeps them (see `Filters::active_in`).
    pub filters: Filters,
    /// Each source's lens as it was left (the shown one's is live).
    pub saved: [Lens; 4],
}

impl Default for Tabs {
    fn default() -> Tabs {
        Tabs {
            source: Source::All,
            order: Order::Sort,
            desc: false,
            filters: Filters::default(),
            saved: Source::ALL.map(|s| Lens::first(s, Lens::PLAIN)),
        }
    }
}

impl Tabs {
    /// Whether what's shown waits on the background analysis.
    pub fn needs_info(&self) -> bool {
        self.order.needs_info() || self.filters.needs_info()
    }

    /// The filters that filter under `tab`.
    pub fn active(&self, tab: Source) -> Filters {
        self.filters.active_in(tab)
    }
}

/// Something in the sort and filter panel to click, or to reach with the keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LensItem {
    /// The panel itself: takes clicks between its items.
    Panel,
    /// `SortMode::ALL[i]`.
    Sort(usize),
    Headings,
    /// `Order::ALL[i]`.
    Order(usize),
    Desc,
    /// Star bar `b` (`star_bin`).
    Star(u8),
    Length(u8),
    Status(u8),
    Course(u8),
    Unplayed,
    Reset,
}

/// The sort and filter panel, open.
#[derive(Clone, Copy, Debug)]
pub struct LensUi {
    pub opened: Instant,
    /// The item the keys are on: column, row, and the item in the row.
    pub at: (usize, usize, usize),
}

/// How long the panel takes to open, and the list to change places when the source does.
const OPEN_MS: f32 = 200.0;
pub(super) const SWITCH_MS: f32 = 260.0;

const ROW_H: f32 = 15.0;
const COL_SORT_W: f32 = 132.0;
const COL_ORDER_W: f32 = 112.0;
const COL_FILTER_W: f32 = 246.0;
const PAD: f32 = 12.0;
const GAP: f32 = 12.0;
const BARS_H: f32 = 46.0;
const BLOCK_H: f32 = 14.0;

impl Select {
    /// The sort and filter panel's items, as the keys reach them: a column each for the sort,
    /// the order and the filters, rows of items in each. Status and course filters only where
    /// the source shown has maps they apply to.
    pub fn lens_grid(&self) -> [Vec<Vec<LensItem>>; 3] {
        let sorts = (0..SortMode::ALL.len())
            .map(|i| vec![LensItem::Sort(i)])
            .chain(std::iter::once(vec![LensItem::Headings]))
            .collect();
        let orders = (0..Order::ALL.len())
            .map(|i| vec![LensItem::Order(i)])
            .chain(std::iter::once(vec![LensItem::Desc]))
            .collect();
        let mut filters = vec![
            (0..10).map(LensItem::Star).collect(),
            (0..LENGTHS.len() as u8).map(LensItem::Length).collect(),
        ];
        let statuses = self.lens_statuses();
        if !statuses.is_empty() {
            filters.push(statuses);
        }
        if self.lens_shows_courses() {
            filters.push((0..COURSES.len() as u8).map(LensItem::Course).collect());
        }
        filters.push(vec![LensItem::Unplayed, LensItem::Reset]);
        [sorts, orders, filters]
    }

    /// The status choices: the statuses the shown tab's maps have (and any picked), none among
    /// TJA charts.
    fn lens_statuses(&self) -> Vec<LensItem> {
        if self.shown_source() == Source::Tja {
            return Vec::new();
        }
        let on = self.tabs.filters.statuses | self.facets.has_status;
        (0..STATUSES.len() as u8)
            .filter(|k| on & 1 << k != 0)
            .map(LensItem::Status)
            .collect()
    }

    /// The course filter: in the TJA tab only (a course is a TJA chart's).
    fn lens_shows_courses(&self) -> bool {
        self.shown_source() == Source::Tja
    }

    /// Opens the sort and filter panel (on the sort the list has), or closes it.
    pub fn toggle_lens(&mut self, library: &[Entry]) {
        if self.lens_ui.take().is_some() {
            return;
        }
        self.picker = None;
        let row = SortMode::ALL
            .iter()
            .position(|&m| m == self.sort)
            .unwrap_or(0);
        self.lens_ui = Some(LensUi {
            opened: Instant::now(),
            at: (0, row, 0),
        });
        // The star bars count what's listed.
        self.refilter(library);
    }

    /// The keys over the open panel: the arrows move, Enter or Space picks, Esc or Backspace
    /// closes. Returns whether the lens changed (to be kept).
    pub fn lens_key(&mut self, code: winit::keyboard::KeyCode, library: &[Entry]) -> bool {
        use winit::keyboard::KeyCode;
        let Some(ui) = self.lens_ui else {
            return false;
        };
        let grid = self.lens_grid();
        let (mut col, mut row, mut item) = ui.at;
        col = col.min(2);
        row = row.min(grid[col].len() - 1);
        match code {
            KeyCode::ArrowUp => row = row.saturating_sub(1),
            KeyCode::ArrowDown => row = (row + 1).min(grid[col].len() - 1),
            KeyCode::ArrowLeft if item > 0 => item -= 1,
            KeyCode::ArrowLeft if col > 0 => {
                row = same_height(row, grid[col].len(), grid[col - 1].len());
                col -= 1;
                item = usize::MAX;
            }
            KeyCode::ArrowRight if item + 1 < grid[col][row].len() => item += 1,
            KeyCode::ArrowRight if col < 2 => {
                row = same_height(row, grid[col].len(), grid[col + 1].len());
                col += 1;
                item = 0;
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let pick = grid[col][row][item.min(grid[col][row].len() - 1)];
                self.lens_pick(pick, library);
                return true;
            }
            KeyCode::Escape | KeyCode::Backspace => {
                self.lens_ui = None;
                return false;
            }
            _ => {}
        }
        row = row.min(grid[col].len() - 1);
        item = item.min(grid[col][row].len() - 1);
        if let Some(ui) = &mut self.lens_ui {
            ui.at = (col, row, item);
        }
        false
    }

    /// What picking `item` does: the sort, the order or a filter changes, and the list with it.
    pub fn lens_pick(&mut self, item: LensItem, library: &[Entry]) {
        // The keys go where the mouse was.
        let grid = self.lens_grid();
        if let Some(ui) = &mut self.lens_ui {
            for (c, rows) in grid.iter().enumerate() {
                for (r, items) in rows.iter().enumerate() {
                    if let Some(k) = items.iter().position(|&x| x == item) {
                        ui.at = (c, r, k);
                    }
                }
            }
        }
        let f = &mut self.tabs.filters;
        match item {
            LensItem::Panel => return,
            LensItem::Sort(i) => self.sort = SortMode::ALL[i.min(SortMode::ALL.len() - 1)],
            LensItem::Headings => self.headings = !self.headings,
            LensItem::Order(i) => self.tabs.order = Order::ALL[i.min(Order::ALL.len() - 1)],
            LensItem::Desc => self.tabs.desc = !self.tabs.desc,
            LensItem::Star(b) => f.click_star(b.min(9)),
            LensItem::Length(k) => f.lengths ^= 1 << k,
            LensItem::Status(k) => f.statuses ^= 1 << k,
            LensItem::Course(k) => f.courses ^= 1 << k,
            LensItem::Unplayed => f.unplayed = !f.unplayed,
            LensItem::Reset => {
                let lens = Lens::first(self.shown_source(), Lens::PLAIN);
                self.set_lens(lens);
                self.tabs.filters = Filters::default();
            }
        }
        self.refilter(library);
    }

    /// The lens of the source shown.
    pub fn lens(&self) -> Lens {
        Lens {
            sort: self.sort,
            order: self.tabs.order,
            desc: self.tabs.desc,
            headings: self.headings,
        }
    }

    /// The lens of `source`: the live one if it's shown.
    pub fn lens_of(&self, source: Source) -> Lens {
        if source == self.tabs.source {
            self.lens()
        } else {
            self.tabs.saved[source.index()]
        }
    }

    fn set_lens(&mut self, lens: Lens) {
        self.sort = lens.sort;
        self.headings = lens.headings;
        self.tabs.order = lens.order;
        self.tabs.desc = lens.desc;
    }

    /// Shows the maps of `source` with its own lens (the one left keeps its own) and the same
    /// filters.
    pub fn set_source(&mut self, source: Source, library: &[Entry]) {
        if source == self.tabs.source {
            return;
        }
        let old = self.tabs.source.index();
        self.tabs.saved[old] = self.lens();
        self.tabs.source = source;
        let lens = self.tabs.saved[source.index()];
        self.set_lens(lens);
        self.source_switched = Some(Instant::now());
        self.refilter(library);
    }

    /// The source listed: the one picked, or all when it has no maps (gone since, or converts
    /// hidden).
    pub fn shown_source(&self) -> Source {
        match self.tabs.source {
            s if self.source_maps[s.index()] == 0 => Source::All,
            s => s,
        }
    }

    /// The sources with maps, all first: the tabs over the list. None with one source only.
    pub fn source_tabs(&self) -> Vec<Source> {
        let with: Vec<Source> = Source::ALL[1..]
            .iter()
            .copied()
            .filter(|s| self.source_maps[s.index()] > 0)
            .collect();
        if with.len() < 2 {
            return Vec::new();
        }
        std::iter::once(Source::All).chain(with).collect()
    }

    /// Ctrl+1 to Ctrl+4: tab `n` (0 first), if there is one.
    pub fn source_key(&mut self, n: usize, library: &[Entry]) -> bool {
        match self.source_tabs().get(n) {
            Some(&s) => {
                self.set_source(s, library);
                true
            }
            None => false,
        }
    }

    /// The sort and filter panel, over the list from (`x`, `y`).
    pub(super) fn draw_lens(&self, c: &mut Canvas, x: f32, y: f32) {
        let Some(ui) = self.lens_ui else { return };
        let grid = self.lens_grid();
        let t = progress(ui.opened, OPEN_MS);
        let ease = 1.0 - (1.0 - t).powi(3);
        let age = ui.opened.elapsed().as_secs_f32() * 1000.0;
        // Each item comes in a little after the one before, top to bottom.
        let stagger = |k: usize| {
            if t >= 1.0 {
                1.0
            } else {
                ((age - 50.0 - k as f32 * 12.0) / 120.0).clamp(0.0, 1.0)
            }
        };
        let rows_h = grid[0].len().max(grid[1].len()) as f32 * ROW_H;
        let filter_rows = grid[2].len() as f32;
        let filters_h = 14.0 + BARS_H + 22.0 + (filter_rows - 1.0) * (BLOCK_H + 18.0);
        let w = PAD * 2.0 + COL_SORT_W + COL_ORDER_W + COL_FILTER_W + GAP * 2.0;
        let h = 22.0 + rows_h.max(filters_h) + 26.0;
        let x = x.min(c.width - w - 8.0).max(8.0);
        // The rest of the screen dims behind it.
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.35 * ease));
        let start = c.shapes.len();
        // Under everything else of the panel: a click between its items stays on it.
        c.hit(x, y, w, h, Hit::Lens(LensItem::Panel));
        crate::style::panel(c, x, y, w, h);
        c.round_rect(
            [x + 1.0, y + 1.0],
            [w - 2.0, h - 2.0],
            9.0,
            rgba(crate::style::PANEL, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [x, y],
            [w, h],
            10.0,
            rgba(0, 0.0),
            rgba(crate::style::selector_colour(), 0.85),
            1.5,
        );
        let at = ui.at;
        let mut target = None;
        let heading = |c: &mut Canvas, hx: f32, text: &str| {
            crate::style::glitch_text(c, hx, y + 9.0, 7.0, 0x9a9ab0, text, (1.0 - t) * 2.5);
        };

        // The sort, and headings or one list.
        let cx = x + PAD;
        heading(c, cx, "SORT BY");
        for (r, items) in grid[0].iter().enumerate() {
            let ry = y + 22.0 + r as f32 * ROW_H;
            let a = stagger(r);
            let (label, on) = match items[0] {
                LensItem::Sort(i) => (
                    SortMode::ALL[i].name().to_string(),
                    SortMode::ALL[i] == self.sort,
                ),
                _ => (
                    if self.headings {
                        "headings: on".to_string()
                    } else {
                        "headings: off".to_string()
                    },
                    self.headings,
                ),
            };
            let rect = [cx, ry, COL_SORT_W, ROW_H - 1.0];
            option_row(c, rect, &label, on, a, Hit::Lens(items[0]));
            if at.0 == 0 && at.1 == r {
                target = Some(rect);
            }
        }

        // The order under each heading.
        let ox = cx + COL_SORT_W + GAP;
        heading(c, ox, "THEN BY");
        for (r, items) in grid[1].iter().enumerate() {
            let ry = y + 22.0 + r as f32 * ROW_H;
            let a = stagger(r + 1);
            let (label, on) = match items[0] {
                LensItem::Order(i) => (
                    Order::ALL[i].label().to_string(),
                    Order::ALL[i] == self.tabs.order,
                ),
                _ => (
                    if self.tabs.desc {
                        "descending".to_string()
                    } else {
                        "ascending".to_string()
                    },
                    self.tabs.desc,
                ),
            };
            let rect = [ox, ry, COL_ORDER_W, ROW_H - 1.0];
            option_row(c, rect, &label, on, a, Hit::Lens(items[0]));
            if at.0 == 1 && at.1 == r {
                target = Some(rect);
            }
        }

        // The filters.
        let fx = ox + COL_ORDER_W + GAP;
        let f = &self.tabs.filters;
        heading(c, fx, "FILTER");
        let mut fy = y + 22.0;
        for (r, items) in grid[2].iter().enumerate() {
            let a = stagger(r + 2);
            let rstart = c.shapes.len();
            match items[0] {
                LensItem::Star(_) => {
                    let range = stars_text(f.stars);
                    c.text(
                        fx,
                        fy,
                        6.0,
                        rgba(0x6f6f86, 1.0),
                        "stars (click a bar, then another)",
                    );
                    c.text_right(fx + COL_FILTER_W, fy, 6.0, rgba(0xffffff, 1.0), &range);
                    let by = fy + 10.0;
                    let bars = &self.facets.stars;
                    let most = bars.iter().copied().max().unwrap_or(0).max(1) as f32;
                    let bw = (COL_FILTER_W - 9.0 * 3.0) / 10.0;
                    for b in 0..10u8 {
                        let bx = fx + b as f32 * (bw + 3.0);
                        let n = bars[b as usize];
                        let kept = f.stars.is_none_or(|(lo, hi)| (lo..=hi).contains(&b));
                        let col = crate::stars::color(b as f32 + 1.5);
                        let bh = if n == 0 {
                            1.5
                        } else {
                            3.0 + (BARS_H - 14.0) * n as f32 / most
                        };
                        let hover = c.hovered(bx, by, bw, BARS_H);
                        c.rect(
                            bx,
                            by + BARS_H - 10.0 - bh,
                            bw,
                            bh,
                            rgba(col, if kept { 1.0 } else { 0.22 }),
                        );
                        // A light top edge: the darkest star colours (9 and up) would vanish
                        // into the panel otherwise.
                        if n > 0 {
                            c.rect(
                                bx,
                                by + BARS_H - 10.0 - bh,
                                bw,
                                1.0,
                                rgba(0xffffff, if kept { 0.45 } else { 0.12 }),
                            );
                        }
                        if hover {
                            c.rect(bx, by, bw, BARS_H - 10.0, rgba(0xffffff, 0.06));
                        }
                        c.text_centered(
                            bx + bw / 2.0,
                            by + BARS_H - 8.0,
                            5.0,
                            rgba(if kept { 0xc8c8d8 } else { 0x55556a }, 1.0),
                            &if b == 9 {
                                "10+".to_string()
                            } else {
                                (b + 1).to_string()
                            },
                        );
                        c.hit(bx, by, bw, BARS_H, Hit::Lens(LensItem::Star(b)));
                        if at.0 == 2 && at.1 == r && at.2 == b as usize {
                            target = Some([bx - 1.0, by - 1.0, bw + 2.0, BARS_H + 2.0]);
                        }
                    }
                    fy += 10.0 + BARS_H + 8.0;
                }
                first => {
                    let caption = match first {
                        LensItem::Length(_) => "length (minutes)",
                        LensItem::Status(_) => "status",
                        LensItem::Course(_) => "TJA course",
                        _ => "",
                    };
                    if !caption.is_empty() {
                        c.text(fx, fy, 6.0, rgba(0x6f6f86, 1.0), caption);
                        fy += 9.0;
                    }
                    // Status blocks as wide as with every status there, whichever are.
                    let n = match first {
                        LensItem::Status(_) => STATUSES.len(),
                        _ => items.len(),
                    } as f32;
                    let bw = (COL_FILTER_W - (n - 1.0) * 4.0) / n;
                    for (k, &item) in items.iter().enumerate() {
                        let bx = fx + k as f32 * (bw + 4.0);
                        let facets = &self.facets;
                        let (label, colour, on, count) = match item {
                            LensItem::Length(i) => (
                                LENGTHS[i as usize].0,
                                0xc8c8d8,
                                f.lengths & 1 << i != 0,
                                Some(facets.lengths[i as usize]),
                            ),
                            LensItem::Status(i) => {
                                let s = STATUSES[i as usize];
                                let n = facets.statuses[i as usize];
                                (s.0, s.1, f.statuses & 1 << i != 0, Some(n))
                            }
                            LensItem::Course(i) => {
                                let s = COURSES[i as usize];
                                let n = facets.courses[i as usize];
                                (s.0, s.1, f.courses & 1 << i != 0, Some(n))
                            }
                            LensItem::Unplayed => ("not played yet", 0xc8c8d8, f.unplayed, None),
                            _ => ("reset", crate::style::DANGER, false, None),
                        };
                        let rect = [bx, fy, bw, BLOCK_H];
                        block(c, rect, label, colour, on, count, Hit::Lens(item));
                        if at.0 == 2 && at.1 == r && at.2 == k {
                            target = Some(rect);
                        }
                    }
                    fy += BLOCK_H + 9.0;
                }
            }
            if a < 1.0 {
                c.fade_since(rstart, a, 1.0);
            }
        }

        // What it all comes to.
        let songs = self.song_count;
        let foot = y + h - 16.0;
        c.rect(x + PAD, foot - 5.0, w - PAD * 2.0, 1.0, rgba(0x2a2a3a, 1.0));
        c.text(
            x + PAD,
            foot,
            7.0,
            rgba(0xffffff, 1.0),
            &format!(
                "{songs} song{} in {}",
                if songs == 1 { "" } else { "s" },
                match self.shown_source() {
                    Source::All => "all sources",
                    s => s.label(),
                }
            ),
        );
        c.text_right(
            x + w - PAD,
            foot,
            6.0,
            rgba(0x6f6f86, 1.0),
            "arrows move  Enter pick  Esc or Ctrl+S close",
        );
        if let Some(rect) = target {
            crate::style::selector(c, "song-lens", rect);
        }

        // Opening: the panel unrolls top down behind a line of light.
        if t < 1.0 {
            let shown = h * ease;
            c.clip_since(start, x - 2.0, y - 2.0, w + 4.0, shown + 2.0);
            c.rect(
                x - 1.0,
                y + shown - 1.0,
                w + 2.0,
                1.5,
                rgba(crate::style::selector_colour(), 1.0 - t),
            );
        }
    }

    /// The filters on, as tokens from `x` along the search box at `y`; returns where they end.
    pub(super) fn draw_tokens(&self, c: &mut Canvas, x: f32, y: f32, room: f32) -> f32 {
        let mut tx = x;
        for (token, text, colour) in self.tabs.active(self.shown_source()).tokens() {
            let tw = Canvas::text_width(7.0, &text) + 18.0;
            if tx + tw > x + room {
                break;
            }
            let hover = c.hovered(tx, y, tw, 16.0);
            c.rect(
                tx,
                y,
                tw,
                16.0,
                rgba(if hover { 0x34344a } else { 0x2a2a38 }, 0.9),
            );
            c.rect(tx, y + 15.0, tw, 1.0, rgba(colour, 1.0));
            c.text(tx + 5.0, y + 4.5, 7.0, rgba(0xffffff, 1.0), &text);
            c.text_right(
                tx + tw - 4.0,
                y + 4.5,
                6.0,
                rgba(if hover { 0xff6b6b } else { 0x6f6f86 }, 1.0),
                "x",
            );
            c.hit(tx, y, tw, 16.0, Hit::LensToken(token));
            tx += tw + 4.0;
        }
        tx
    }

    /// The tabs over the list, one per source with maps, from (`x`, `y`) across `w`.
    pub(crate) fn draw_source_tabs(&self, c: &mut Canvas, x: f32, y: f32, w: f32, tabs: &[Source]) {
        let tw = w / tabs.len() as f32;
        let shown = self.shown_source();
        let mut target = None;
        for (k, &s) in tabs.iter().enumerate() {
            let tx = x + k as f32 * tw;
            let on = s == shown;
            let hover = c.hovered(tx, y, tw, SOURCE_TAB_H);
            // Each tab a chip of its own, as the search row's field is: the raised surface,
            // lifted under the pointer and while it's the one shown.
            let level = if on || hover {
                crate::style::Surface::Lifted
            } else {
                crate::style::Surface::Raised
            };
            crate::style::surface(
                c,
                [tx + 2.0, y + 1.0, tw - 4.0, SOURCE_TAB_H - 3.0],
                crate::style::radius::SMALL,
                level,
            );
            // What the tab lists, the search and filters as they are.
            let n = self.source_listed[s.index()];
            let label = s.label();
            let count = n.to_string();
            let lw = Canvas::text_width(7.0, label) + 6.0 + Canvas::text_width(6.0, &count);
            let lx = tx + (tw - lw - 9.0) / 2.0;
            c.rect(
                lx,
                y + 6.5,
                5.0,
                5.0,
                rgba(s.colour(), if on { 1.0 } else { 0.5 }),
            );
            let lx = lx + 9.0;
            let col = if on {
                0xffffff
            } else if hover {
                0xc8c8d8
            } else {
                0x8a8aa0
            };
            c.text(lx, y + 5.0, 7.0, rgba(col, 1.0), label);
            c.text(
                lx + Canvas::text_width(7.0, label) + 6.0,
                y + 6.0,
                6.0,
                rgba(0x6f6f86, 1.0),
                &count,
            );
            c.hit(tx, y, tw, SOURCE_TAB_H, Hit::SongSource(s.index()));
            if on {
                target = Some([tx + 2.0, y + 1.0, tw - 4.0, SOURCE_TAB_H - 3.0]);
            }
        }
        if let Some(rect) = target {
            crate::style::selector(c, "song-source", rect);
        }
    }
}

impl Select {
    /// Takes the source and each source's lens from the config.
    pub fn read_lenses(&mut self, cfg: &crate::config::Config) {
        let mut words = cfg.song_order.split_whitespace();
        let all = Lens {
            sort: SortMode::parse(&cfg.sort).unwrap_or(SortMode::Artist),
            order: words.next().and_then(Order::parse).unwrap_or_default(),
            desc: words.next() == Some("desc"),
            headings: cfg.song_headings,
        };
        let source = Source::parse(&cfg.song_source).unwrap_or_default();
        self.tabs.source = Source::All;
        self.set_lens(all);
        self.tabs.saved[Source::All.index()] = all;
        for (s, text) in [
            (Source::Osu, &cfg.lens_osu),
            (Source::Tja, &cfg.lens_tja),
            (Source::Own, &cfg.lens_own),
        ] {
            self.tabs.saved[s.index()] = Lens::parse(text).unwrap_or(Lens::first(s, all));
        }
        if source != Source::All {
            let lens = self.tabs.saved[source.index()];
            self.tabs.source = source;
            self.set_lens(lens);
        }
    }

    /// Puts the source and each source's lens in the config, as `read_lenses` takes them.
    pub fn write_lenses(&self, cfg: &mut crate::config::Config) {
        let all = self.lens_of(Source::All);
        cfg.sort = all.sort.name().to_string();
        cfg.song_headings = all.headings;
        cfg.song_order = if all.desc {
            format!("{} desc", all.order.name())
        } else {
            all.order.name().to_string()
        };
        cfg.song_source = self.tabs.source.name().to_string();
        let text = |s: Source| {
            let lens = self.lens_of(s);
            if lens == Lens::first(s, all) {
                String::new()
            } else {
                lens.text()
            }
        };
        cfg.lens_osu = text(Source::Osu);
        cfg.lens_tja = text(Source::Tja);
        cfg.lens_own = text(Source::Own);
    }
}

/// Row `row` of a column `from` rows long, as the row about as far down a column `to` rows long.
fn same_height(row: usize, from: usize, to: usize) -> usize {
    if from <= 1 || to <= 1 {
        return 0;
    }
    ((row as f32 * (to - 1) as f32 / (from - 1) as f32).round() as usize).min(to - 1)
}

/// The height of the source tabs over the list.
pub(super) const SOURCE_TAB_H: f32 = 18.0;

/// A row of the sort or order column: a square lit when it's the one on.
fn option_row(c: &mut Canvas, [x, y, w, h]: [f32; 4], label: &str, on: bool, alpha: f32, hit: Hit) {
    let start = c.shapes.len();
    if c.hovered(x, y, w, h) {
        c.rect(x, y, w, h, rgba(0x262634, 1.0));
    }
    let sq = if on {
        crate::style::selector_colour()
    } else {
        0x34344a
    };
    c.rect(x + 5.0, y + h / 2.0 - 2.5, 5.0, 5.0, rgba(sq, 1.0));
    c.text(
        x + 15.0,
        y + (h - 7.0) / 2.0,
        7.0,
        rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0),
        &Canvas::fit(7.0, w - 18.0, label),
    );
    c.hit(x, y, w, h, hit);
    if alpha < 1.0 {
        c.fade_since(start, alpha, 1.0);
    }
}

/// A filter's choice: a block, its colour's edge lit when it's on, and how many maps it would
/// list at its right.
#[allow(clippy::too_many_arguments)]
fn block(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    label: &str,
    colour: u32,
    on: bool,
    count: Option<u32>,
    hit: Hit,
) {
    let hover = c.hovered(x, y, w, h);
    let bg = match (on, hover) {
        (true, _) => 0x2e2e3e,
        (false, true) => 0x262634,
        _ => 0x1d1d27,
    };
    c.rect(x, y, w, h, rgba(bg, 1.0));
    c.rect(
        x,
        y + h - 1.0,
        w,
        1.0,
        rgba(if on { colour } else { 0x111118 }, 1.0),
    );
    // Its colour down the left edge: a square would take the room the label and count need.
    c.rect(x, y, 2.0, h, rgba(colour, if on { 1.0 } else { 0.35 }));
    let count = count.map(|n| n.to_string());
    let count_w = count
        .as_deref()
        .map_or(0.0, |n| Canvas::text_width(4.5, n) + 3.0);
    if let Some(n) = &count {
        let dim = if n == "0" { 0x45455a } else { 0x6f6f86 };
        c.text_right(x + w - 3.0, y + (h - 4.5) / 2.0, 4.5, rgba(dim, 1.0), n);
    }
    let room = w - 9.0 - count_w;
    let size = Canvas::fit_size(6.0, 4.5, room, label);
    c.text(
        x + 6.0,
        y + (h - size) / 2.0,
        size,
        rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0),
        &Canvas::fit(size, room, label),
    );
    c.hit(x, y, w, h, hit);
}
