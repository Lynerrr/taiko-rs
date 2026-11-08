//! Download tab: search the osu! mirrors for taiko sets, or the taiko.rs website for its custom
//! ones, and download them into the Songs folder.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::download::{self, Category, Diff, Job, SetInfo, SortKey};
use crate::library::Entry;
use crate::render::{Canvas, Hit, accent, approach, rgba};

/// Search this long after the last keystroke.
const TYPE_DELAY: Duration = Duration::from_millis(450);
/// Fetch the selected set's preview once the selection rests this long.
const PREVIEW_DELAY: Duration = Duration::from_millis(250);

/// (custom set, set id, beatmap id) of a previewed difficulty: a custom set's numbers are the
/// website's, and its difficulties are numbered by their place in it.
type DiffKey = (bool, u32, u32);

/// A fetched preview: its difficulties (one from osu!, the whole set from the website), with
/// the star rating worked out here for those the website doesn't rate.
type Fetched = Result<Vec<(DiffKey, Entry, Option<f32>)>, String>;

/// Where the tab looks for maps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// osu!'s maps, from the mirrors.
    Osu,
    /// The taiko.rs website's custom maps.
    Ours,
}

impl Source {
    pub const ALL: [Source; 2] = [Source::Osu, Source::Ours];

    pub fn name(self) -> &'static str {
        match self {
            Source::Osu => "osu!",
            Source::Ours => "taiko.rs",
        }
    }
}

type SearchPage = Result<(Vec<SetInfo>, bool), String>;

pub struct Browse {
    pub query: String,
    pub category: Category,
    /// The category the other source had, for switching back to it.
    other_category: Category,
    pub sort: SortKey,
    pub desc: bool,
    pub results: Vec<SetInfo>,
    pub cursor: usize,
    scroll: f32,
    pub status: String,
    /// The search on its way and the page (from 0) it fetches: 0 replaces the results, later
    /// pages add to them.
    search: Option<(usize, Receiver<SearchPage>)>,
    /// The last page shown, and whether the mirrors may have more.
    page: usize,
    more: bool,
    edited: Option<Instant>,
    /// The query the shown results are for (None: nothing searched yet).
    searched: Option<String>,
    /// Previewable difficulties fetched so far (entries for the song select preview) and their keys.
    pub remote: Vec<Entry>,
    remote_keys: Vec<DiffKey>,
    /// The remote entry being previewed.
    pub shown: Option<usize>,
    /// Chosen difficulty per set (default: the hardest).
    diff_choice: HashMap<u32, usize>,
    wanted: Option<(DiffKey, Instant)>,
    fetching: Option<(DiffKey, Receiver<Fetched>)>,
    failed: HashSet<DiffKey>,
    /// Star ratings of custom difficulties, worked out from their previews (the website has none).
    rated: HashMap<DiffKey, f32>,
}

impl Default for Browse {
    fn default() -> Browse {
        Browse {
            query: String::new(),
            category: Category::HasLeaderboard,
            other_category: Category::CustomApproved,
            sort: SortKey::Relevance,
            desc: true,
            results: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            status: String::new(),
            search: None,
            page: 0,
            more: false,
            edited: None,
            searched: None,
            remote: Vec::new(),
            remote_keys: Vec::new(),
            shown: None,
            diff_choice: HashMap::new(),
            wanted: None,
            fetching: None,
            failed: HashSet::new(),
            rated: HashMap::new(),
        }
    }
}

/// What a result row shows on its right.
pub enum RowState {
    Installed,
    Downloading(Option<f32>),
    Available,
}

impl Browse {
    /// Starts a search for the current query, category and order now.
    pub fn search_now(&mut self) {
        let query = self.query.trim().to_string();
        self.searched = Some(query.clone());
        self.edited = None;
        self.status = if query.is_empty() {
            let what = self.category.name().to_lowercase();
            match self.source() {
                Source::Osu => format!("loading {what} taiko maps..."),
                Source::Ours => format!("loading {what} maps from taiko.rs..."),
            }
        } else {
            format!("searching \"{query}\"...")
        };
        self.more = false;
        self.fetch(query, 0);
    }

    fn fetch(&mut self, query: String, page: usize) {
        let (tx, rx) = mpsc::channel();
        let (category, sort, desc) = (self.category, self.sort, self.desc);
        std::thread::spawn(move || {
            let found = if category.is_ours() {
                // The website lists them all at once.
                crate::custom::search(&query, category == Category::CustomApproved, sort, desc)
                    .map(|sets| (sets, false))
            } else {
                download::search(&query, category, sort, desc, page)
            };
            let _ = tx.send(found);
        });
        self.search = Some((page, rx));
    }

    /// Fetches the next page of results once the selection gets near the end of the list.
    fn load_more_near_end(&mut self) {
        if self.more
            && self.search.is_none()
            && self.edited.is_none()
            && self.cursor + 10 >= self.results.len()
            && let Some(query) = self.searched.clone()
        {
            self.fetch(query, self.page + 1);
        }
    }

    pub fn source(&self) -> Source {
        if self.category.is_ours() {
            Source::Ours
        } else {
            Source::Osu
        }
    }

    /// The categories of the current source.
    pub fn categories(&self) -> &'static [Category] {
        match self.source() {
            Source::Osu => &Category::ALL,
            Source::Ours => &Category::OURS,
        }
    }

    /// The orders the current source can list in.
    pub fn sorts(&self) -> &'static [SortKey] {
        match self.source() {
            Source::Osu => &SortKey::ALL,
            Source::Ours => &SortKey::OURS,
        }
    }

    /// Switches to the osu! mirrors or the taiko.rs website, in the category it had last.
    pub fn set_source(&mut self, source: Source) {
        if source == self.source() {
            return;
        }
        std::mem::swap(&mut self.category, &mut self.other_category);
        if !self.sorts().contains(&self.sort) {
            self.sort = SortKey::Relevance;
            self.desc = true;
        }
        self.search_now();
    }

    pub fn set_category(&mut self, c: Category) {
        if c != self.category {
            self.category = c;
            self.search_now();
        }
    }

    /// Picks an order; picking the current one again flips its direction.
    pub fn set_sort(&mut self, s: SortKey) {
        if s == self.sort {
            self.desc = !self.desc;
        } else {
            self.sort = s;
            self.desc = s.default_desc();
        }
        self.search_now();
    }

    pub fn step_category(&mut self, dir: i32) {
        let all = self.categories();
        let i = all.iter().position(|&c| c == self.category).unwrap_or(0) as i32;
        self.set_category(all[(i + dir).rem_euclid(all.len() as i32) as usize]);
    }

    pub fn step_sort(&mut self, dir: i32) {
        let all = self.sorts();
        let i = all.iter().position(|&s| s == self.sort).unwrap_or(0) as i32;
        let next = all[(i + dir).rem_euclid(all.len() as i32) as usize];
        self.sort = next;
        self.desc = next.default_desc();
        self.search_now();
    }

    /// Searches on first open.
    pub fn opened(&mut self) {
        if self.searched.is_none() {
            self.search_now();
        }
    }

    /// The selected set's chosen difficulty.
    pub fn selected_diff(&self) -> Option<&Diff> {
        let set = self.selected()?;
        let i = self
            .diff_choice
            .get(&set.id)
            .copied()
            .unwrap_or(set.diffs.len().saturating_sub(1));
        set.diffs.get(i)
    }

    /// Left/Right: preview another difficulty of the selected set.
    pub fn cycle_diff(&mut self, dir: i32) {
        let Some(set) = self.selected() else { return };
        let (id, n) = (set.id, set.diffs.len());
        if n == 0 {
            return;
        }
        let cur = self.diff_choice.get(&id).copied().unwrap_or(n - 1) as i32;
        self.diff_choice
            .insert(id, (cur + dir).rem_euclid(n as i32) as usize);
    }

    /// The selected difficulty's star rating: a custom one's once its preview came.
    pub fn selected_stars(&self) -> Option<f32> {
        let set = self.selected()?;
        let d = self.selected_diff()?;
        match set.custom {
            Some(_) => self.rated.get(&(true, set.id, d.id)).copied(),
            None => Some(d.stars),
        }
    }

    /// Fetches the preview for the selection once it rests (one fetch at a time).
    fn poll_preview(&mut self, now: Instant) {
        if let Some((key, rx)) = &self.fetching
            && let Ok(result) = rx.try_recv()
        {
            let key = *key;
            self.fetching = None;
            match result {
                Ok(entries) => {
                    for (k, entry, stars) in entries {
                        if let Some(stars) = stars {
                            self.rated.insert(k, stars);
                        }
                        if !self.remote_keys.contains(&k) {
                            self.remote.push(entry);
                            self.remote_keys.push(k);
                        }
                    }
                    // A difficulty the set turned out not to have isn't asked for again.
                    if !self.remote_keys.contains(&key) {
                        self.failed.insert(key);
                    }
                }
                Err(e) => {
                    self.failed.insert(key);
                    self.status = format!("no preview: {e}");
                }
            }
        }
        let want = self
            .selected()
            .zip(self.selected_diff())
            .map(|(s, d)| (s.custom.is_some(), s.id, d.id));
        if want != self.wanted.map(|w| w.0) {
            self.wanted = want.map(|k| (k, now));
        }
        let Some((key, since)) = self.wanted else {
            self.shown = None;
            return;
        };
        self.shown = self.remote_keys.iter().position(|&k| k == key);
        let due = now - since >= PREVIEW_DELAY;
        // A fetch for another selection is abandoned (its files stay cached for later).
        let busy = self.fetching.as_ref().is_some_and(|(k, _)| *k == key);
        if self.shown.is_none() && due && !busy && !self.failed.contains(&key) && key.2 != 0 {
            let (tx, rx) = mpsc::channel();
            let set = self.selected().cloned();
            std::thread::spawn(move || {
                let result = match set.filter(|s| s.custom.is_some()) {
                    Some(set) => {
                        crate::custom::fetch_preview(&set).map(|dir| custom_entries(&set, &dir))
                    }
                    None => download::fetch_preview(key.1, key.2).and_then(|osu| {
                        let entry = make_entry(&osu).ok_or("unreadable difficulty")?;
                        Ok(vec![(key, entry, None)])
                    }),
                };
                let _ = tx.send(result);
            });
            self.fetching = Some((key, rx));
        }
    }

    pub fn poll(&mut self, now: Instant) {
        self.poll_preview(now);
        if self.edited.is_some_and(|t| now - t >= TYPE_DELAY)
            && self.searched.as_deref() != Some(self.query.trim())
        {
            self.search_now();
        }
        if let Some((page, rx)) = &self.search {
            let page = *page;
            if let Ok(result) = rx.try_recv() {
                self.search = None;
                match result {
                    Ok((sets, more)) if page == 0 => {
                        self.status = if sets.is_empty() {
                            "no taiko maps found".into()
                        } else {
                            String::new()
                        };
                        self.results = sets;
                        self.cursor = 0;
                        (self.page, self.more) = (0, more);
                    }
                    Ok((sets, more)) => {
                        // Sets that moved between pages meanwhile are listed once.
                        let known: HashSet<u32> = self.results.iter().map(|s| s.id).collect();
                        self.results
                            .extend(sets.into_iter().filter(|s| !known.contains(&s.id)));
                        (self.page, self.more) = (page, more);
                    }
                    Err(e) if page == 0 => self.status = format!("search failed: {e}"),
                    Err(e) => {
                        // No more paging until the next search.
                        self.status = format!("couldn't load more: {e}");
                        self.more = false;
                    }
                }
            }
        }
        self.load_more_near_end();
    }

    pub fn type_text(&mut self, text: &str) {
        let before = self.query.len();
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        if self.query.len() != before {
            self.edited = Some(Instant::now());
        }
    }

    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.edited = Some(Instant::now());
        }
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.edited = Some(Instant::now());
    }

    pub fn move_by(&mut self, n: i32) {
        let last = self.results.len().saturating_sub(1) as i32;
        self.cursor = (self.cursor as i32 + n).clamp(0, last.max(0)) as usize;
    }

    pub fn selected(&self) -> Option<&SetInfo> {
        self.results.get(self.cursor)
    }

    /// Draws the search box and results in a column `col` wide from the left edge (the preview
    /// panel goes to its right).
    pub fn draw(
        &mut self,
        c: &mut Canvas,
        jobs: &[Job],
        installed: &dyn Fn(&SetInfo) -> bool,
        songs_dir: &str,
        col: f32,
    ) {
        let w = c.width;
        let x0 = 24.0;
        let source = self.source();
        crate::settings::tab_bar(
            c,
            3,
            match source {
                Source::Osu => "maps from catboy.best / osu.direct",
                Source::Ours => "custom maps from the taiko.rs website",
            },
        );
        let busy = self.search.as_ref().is_some_and(|s| s.0 == 0) || self.edited.is_some();
        // Right of the search box: where to look, osu! or taiko.rs.
        let seg_w = Source::ALL
            .iter()
            .map(|s| Canvas::text_width(8.0, s.name()) + 16.0)
            .sum::<f32>();
        let field = col - seg_w - 8.0;
        let mut sx = x0 + field + 8.0;
        c.round_rect(
            [sx, 54.0],
            [seg_w, 24.0],
            6.0,
            rgba(0x23232f, 0.95),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        for (i, s) in Source::ALL.iter().enumerate() {
            let w = Canvas::text_width(8.0, s.name()) + 16.0;
            let on = *s == source;
            if on {
                c.round_rect(
                    [sx + 2.0, 56.0],
                    [w - 4.0, 20.0],
                    5.0,
                    rgba(accent(), 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
            } else if c.hovered(sx, 54.0, w, 24.0) {
                c.round_rect(
                    [sx + 2.0, 56.0],
                    [w - 4.0, 20.0],
                    5.0,
                    rgba(0x34344a, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            c.text_centered(
                sx + w / 2.0,
                62.0,
                8.0,
                rgba(if on { 0xffffff } else { 0xc8c8d8 }, 1.0),
                s.name(),
            );
            c.hit(sx, 54.0, w, 24.0, Hit::DlSource(i));
            sx += w;
        }
        let col_all = col;
        let col = field;
        c.round_rect(
            [x0, 54.0],
            [col, 24.0],
            6.0,
            rgba(0x23232f, 0.95),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        // The query keeps clear of "searching..." on the right.
        let room = col
            - 20.0
            - if busy {
                Canvas::text_width(8.0, "searching...") + 10.0
            } else {
                0.0
            };
        let text = if self.query.is_empty() {
            let hint = match source {
                Source::Osu => "search artist, title, mapper...",
                Source::Ours => "search song, mapper, sender...",
            };
            Canvas::fit(10.0, room, hint)
        } else {
            Canvas::fit_end(10.0, room, &format!("{}_", self.query))
        };
        c.text(
            x0 + 10.0,
            61.0,
            10.0,
            rgba(
                if self.query.is_empty() {
                    0x6f6f86
                } else {
                    0xffffff
                },
                1.0,
            ),
            &text,
        );
        c.hit(x0, 54.0, col, 24.0, Hit::TextField);
        if busy {
            c.text_right(
                x0 + col - 10.0,
                62.0,
                8.0,
                rgba(0xFFD34D, 1.0),
                "searching...",
            );
        }
        let col = col_all;
        // Website-style filters: status category, then order (click the active one to flip it).
        let chip = |c: &mut Canvas, x: f32, y: f32, label: &str, on: bool, hit: Hit| -> f32 {
            let w = Canvas::text_width(6.0, label) + 8.0;
            let hover = !on && c.hovered(x, y, w, 15.0);
            let bg = if on {
                accent()
            } else if hover {
                0x34344a
            } else {
                0x23232f
            };
            c.round_rect(
                [x, y],
                [w, 15.0],
                5.0,
                rgba(bg, 1.0),
                rgba(if on { accent() } else { 0x3a3a4e }, 1.0),
                1.0,
            );
            c.text(
                x + 4.0,
                y + 4.5,
                6.0,
                rgba(if on { 0xffffff } else { 0xc8c8d8 }, 1.0),
                label,
            );
            c.hit(x, y, w, 15.0, hit);
            w + 6.0
        };
        // Each group on its own line, wrapping onto more when the column is narrow; the list
        // starts under them.
        let mut y = 84.0;
        let group = |c: &mut Canvas, y: &mut f32, title: &str, chips: Vec<(String, bool, Hit)>| {
            c.text(x0, *y + 4.0, 6.0, rgba(0x8a8aa0, 1.0), title);
            let mut x = x0 + 32.0;
            for (label, on, hit) in chips {
                if x > x0 + 32.0 && x + Canvas::text_width(6.0, &label) + 8.0 > x0 + col {
                    x = x0 + 32.0;
                    *y += 19.0;
                }
                x += chip(c, x, *y, &label, on, hit);
            }
            *y += 20.0;
        };
        let cats = self
            .categories()
            .iter()
            .enumerate()
            .map(|(i, cat)| {
                (
                    cat.name().to_string(),
                    *cat == self.category,
                    Hit::DlCategory(i),
                )
            })
            .collect();
        group(c, &mut y, "STATUS", cats);
        let sorts = self
            .sorts()
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let on = *key == self.sort;
                let name = match source {
                    Source::Osu => key.name(),
                    Source::Ours => key.name_for_ours(),
                };
                let label = if on && *key != SortKey::Relevance {
                    format!("{name} {}", if self.desc { "v" } else { "^" })
                } else {
                    name.to_string()
                };
                (label, on, Hit::DlSort(i))
            })
            .collect();
        group(c, &mut y, "SORT", sorts);

        const ROW: f32 = 40.0;
        let (top, bottom) = (y + 2.0, 436.0);
        let view = bottom - top;
        // A last row says whether more are coming.
        let rows = self.results.len() + usize::from(!self.results.is_empty());
        let target =
            (self.cursor as f32 * ROW - view * 0.4).clamp(0.0, (rows as f32 * ROW - view).max(0.0));
        approach(&mut self.scroll, target, c.dt_ms, 50.0);
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
        for (i, set) in self.results.iter().enumerate() {
            let y = top + i as f32 * ROW - self.scroll;
            if y + ROW < top || y > bottom {
                continue;
            }
            let selected = i == self.cursor;
            let hover = !selected && c.hovered(x0, y, col, ROW - 4.0);
            let bg = if selected {
                0x2b2b3d
            } else if hover {
                0x262634
            } else {
                0x1d1d27
            };
            c.round_rect(
                [x0, y],
                [col, ROW - 4.0],
                7.0,
                rgba(bg, 0.95),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            if selected {
                crate::style::selector(c, "download", [x0 - 3.0, y - 3.0, col + 6.0, ROW + 2.0]);
            }
            c.hit(x0, y, col, ROW, Hit::DlRow(i));
            let state = if installed(set) {
                RowState::Installed
            } else if let Some(job) = jobs.iter().find(|j| j.set.same(set)) {
                RowState::Downloading(job.fraction())
            } else {
                RowState::Available
            };
            // Right side: star range and the download state / button.
            let right = x0 + col - 10.0;
            let action_w = if col < 420.0 { 64.0 } else { 92.0 };
            match state {
                RowState::Installed => {
                    c.text_right(right, y + 13.0, 8.0, rgba(0x86E05A, 1.0), "installed")
                }
                RowState::Downloading(f) => {
                    let (bx, by, bw) = (right - action_w, y + 11.0, action_w);
                    c.round_rect(
                        [bx, by],
                        [bw, 12.0],
                        4.0,
                        rgba(0x15151d, 1.0),
                        rgba(0x3a3a4e, 1.0),
                        1.0,
                    );
                    if let Some(f) = f {
                        c.round_rect(
                            [bx + 1.0, by + 1.0],
                            [(bw - 2.0) * f.clamp(0.0, 1.0), 10.0],
                            3.0,
                            rgba(0x438EAD, 1.0),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    let label = f
                        .map(|f| format!("{:.0}%", f * 100.0))
                        .unwrap_or_else(|| "starting...".into());
                    c.text_centered(bx + bw / 2.0, by + 3.0, 6.0, rgba(0xffffff, 1.0), &label);
                }
                RowState::Available => c.button(
                    right - action_w,
                    y + 8.0,
                    action_w,
                    20.0,
                    "Download",
                    8.0,
                    Hit::DlGet(i),
                    selected,
                ),
            }
            let lo = set.diffs.first().map(|d| d.stars).unwrap_or(0.0);
            let hi = set.diffs.last().map(|d| d.stars).unwrap_or(0.0);
            let pill_right = right - action_w - 10.0;
            // The website doesn't rate custom maps: their status takes the star pill's place.
            let pill = match &set.custom {
                Some(custom) => badge(c, pill_right, y + 10.0, custom.approved),
                None => crate::stars::draw_pill(c, pill_right, y + 13.0, 7.0, Some(hi)),
            };
            // The low end of the star range only when the title keeps room.
            let range = if (hi - lo).abs() < 0.05 || col < 420.0 || set.custom.is_some() {
                String::new()
            } else {
                format!("{lo:.2} -")
            };
            c.text_right(
                pill_right - pill - 4.0,
                y + 13.0,
                7.0,
                rgba(0x8a8aa0, 1.0),
                &range,
            );
            let text_room =
                pill_right - pill - Canvas::text_width(7.0, &range) - 16.0 - (x0 + 12.0);
            c.text(
                x0 + 12.0,
                y + 7.0,
                9.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(9.0, text_room, &format!("{} - {}", set.artist, set.title)),
            );
            let n = set.diffs.len();
            let diffs = format!("{n} taiko diff{}", if n == 1 { "" } else { "s" });
            let sub = match &set.custom {
                Some(custom) => format!(
                    "mapped by {}  |  {n} diff{}  |  {} download{}  |  {:.1} MB",
                    set.creator,
                    if n == 1 { "" } else { "s" },
                    set.play_count,
                    if set.play_count == 1 { "" } else { "s" },
                    custom.bytes as f64 / (1 << 20) as f64,
                ),
                None => format!("mapped by {}  |  {}  |  {diffs}", set.creator, set.status),
            };
            c.text(
                x0 + 12.0,
                y + 22.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(7.0, text_room, &sub),
            );
        }
        if !self.results.is_empty() {
            let y = top + self.results.len() as f32 * ROW - self.scroll;
            let n = self.results.len();
            let end = if self.search.is_some() {
                "loading more...".to_string()
            } else if source == Source::Ours {
                format!(
                    "{n} map{}  |  yours: editor > File > Upload",
                    if n == 1 { "" } else { "s" }
                )
            } else if self.more {
                format!("{n} maps so far: scroll on for more")
            } else {
                format!("all {n} map{} listed", if n == 1 { "" } else { "s" })
            };
            c.text_centered(
                x0 + col / 2.0,
                y + 13.0,
                8.0,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(8.0, col, &end),
            );
        }
        c.clip_since(
            list_start,
            x0 - 4.0,
            top - 2.0,
            col + 8.0,
            bottom - top + 4.0,
        );
        for h in &mut c.hits[hits_start..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(top - 2.0), (*y + *hh).min(bottom + 2.0));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }
        if self.results.is_empty() || !self.status.is_empty() {
            let msg = if self.status.is_empty() {
                "no results"
            } else {
                self.status.as_str()
            };
            c.text_centered(
                x0 + col / 2.0,
                if self.results.is_empty() {
                    220.0
                } else {
                    440.0
                },
                9.0,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(9.0, col, msg),
            );
        }
        let active = jobs.len();
        let footer = if active > 0 {
            format!("{active} downloading  |  saving to {songs_dir}")
        } else {
            format!("saving to {songs_dir}")
        };
        let hints = "Up/Down select   Left/Right difficulty   Enter download   Ctrl+Left/Right status   Ctrl+Up/Down sort   Ctrl+Tab osu! / taiko.rs   Esc clear / back";
        c.text(
            24.0,
            452.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit_hints(7.0, w - 48.0, hints),
        );
        c.text(
            24.0,
            466.0,
            7.0,
            rgba(0x55556a, 1.0),
            &Canvas::fit(7.0, w - 48.0, &footer),
        );
    }
}

/// A custom set's status, right-aligned at `right`: green when approved (it gives pp). Returns
/// its width.
fn badge(c: &mut Canvas, right: f32, y: f32, approved: bool) -> f32 {
    let (label, color) = if approved {
        ("approved", 0x86E05A)
    } else {
        ("custom", 0xB28CFF)
    };
    let w = Canvas::text_width(6.0, label) + 10.0;
    c.round_rect(
        [right - w, y],
        [w, 14.0],
        5.0,
        rgba(color, 0.18),
        rgba(color, 0.9),
        1.0,
    );
    c.text(right - w + 5.0, y + 4.0, 6.0, rgba(color, 1.0), label);
    w
}

/// The library entries of a custom set's preview, keyed by the difficulties the website lists
/// (matched by name: the archive's files are named by the mapper).
fn custom_entries(set: &SetInfo, dir: &std::path::Path) -> Vec<(DiffKey, Entry, Option<f32>)> {
    crate::library::scan(
        &[dir.to_path_buf()],
        &std::sync::atomic::AtomicUsize::new(0),
        &mut |_| {},
    )
    .into_iter()
    .filter_map(|e| {
        let d = set.diffs.iter().find(|d| d.version == e.meta.version)?;
        let stars = crate::stars::star_rating(&e.path, 1.0).map(|s| s as f32);
        Some(((true, set.id, d.id), e, stars))
    })
    .collect()
}

/// A library entry for a fetched preview: its `.osu`, with the music clip and cover next to it.
fn make_entry(osu: &std::path::Path) -> Option<Entry> {
    let mut meta = crate::beatmap::read_meta_file(osu)?;
    let dir = osu.parent()?;
    meta.audio_file = download::clip_in(dir)?;
    meta.background = dir
        .join("cover.jpg")
        .is_file()
        .then(|| "cover.jpg".to_string());
    Some(Entry::new(osu.to_path_buf(), meta))
}
