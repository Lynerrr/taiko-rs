//! Download tab: search the osu! mirrors for taiko sets, or the taiko.rs website for its custom
//! ones, and download them into the Songs folder.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::download::{self, Category, Diff, Job, SetInfo, SortKey};
use crate::hit::{Canvas, Hit};
use crate::library::Entry;
use crate::render::{accent, approach, rgba};

/// Search this long after the last keystroke.
const TYPE_DELAY: Duration = Duration::from_millis(450);
/// Fetch the selected set's preview once the selection rests this long.
const PREVIEW_DELAY: Duration = Duration::from_millis(250);
/// A result card's height, with the gap under it.
const ROW: f32 = 50.0;
/// Where the result list starts, under the search box and the filter row.
const LIST_TOP: f32 = 110.0;
/// Where it ends at the most (the hints go under it).
const LIST_BOTTOM: f32 = 436.0;
/// Covers fetched at once.
const COVER_FETCHES: usize = 4;
/// Downloads the panel under the list shows at once.
const PANEL_ROWS: usize = 3;

/// The filter row's drop-down lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    Status,
    Sort,
}

/// A result's cover in `download::covers_dir`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cover {
    Fetching,
    Ready,
    Missing,
}

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
    /// The drop-down list that is open.
    pub menu: Option<Menu>,
    /// Where the list scrolls to: kept on the selection unless the wheel moved it (`follow`).
    scroll_target: f32,
    follow: bool,
    /// The list was scrolled near its end last frame (more results get fetched then).
    near_end: bool,
    /// Covers by `download::cover_name`, and the fetches coming back.
    covers: HashMap<String, Cover>,
    cover_tx: Sender<(String, bool)>,
    cover_rx: Receiver<(String, bool)>,
    /// Downloads that failed (the set and why), for the panel's Retry.
    pub failed_jobs: Vec<(SetInfo, String)>,
    /// The osu! sets loved on the taiko.rs server (None: it hasn't said yet).
    loved_here: Option<HashSet<u32>>,
    /// The loved list as it loads: the sets so far, and how many are done of how many.
    loved_progress: Option<Receiver<(Vec<SetInfo>, usize, usize)>>,
    /// How many loved sets are done of how many, while they load.
    loved_count: Option<(usize, usize)>,
}

impl Default for Browse {
    fn default() -> Browse {
        let (cover_tx, cover_rx) = mpsc::channel();
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
            menu: None,
            scroll_target: 0.0,
            follow: true,
            near_end: false,
            covers: HashMap::new(),
            cover_tx,
            cover_rx,
            failed_jobs: Vec::new(),
            loved_here: None,
            loved_progress: None,
            loved_count: None,
        }
    }
}

/// What a result card shows on its right.
pub enum RowState {
    Installed,
    Downloading(Option<f32>),
    Available,
}

impl Browse {
    /// Whether a search, a previewed difficulty, a cover or the loved list is on its way.
    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.search.is_some()
            || self.fetching.is_some()
            || self.loved_progress.is_some()
            || self.covers.values().any(|c| matches!(c, Cover::Fetching))
    }

    /// Starts a search for the current query, category and order now.
    pub fn search_now(&mut self) {
        let query = self.query.trim().to_string();
        self.searched = Some(query.clone());
        self.edited = None;
        self.status = if query.is_empty() {
            let what = self.category.name().to_lowercase();
            match (self.source(), self.category) {
                (_, Category::LovedHere) => {
                    "loading loved maps (slower the first time)...".to_string()
                }
                (Source::Osu, _) => format!("loading {what} taiko maps..."),
                (Source::Ours, _) => format!("loading {what} maps from taiko.rs..."),
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
        let loved: Option<Vec<u32>> = self
            .loved_here
            .as_ref()
            .map(|l| l.iter().copied().collect());
        let (progress_tx, progress_rx) = mpsc::channel();
        self.loved_progress = (category == Category::LovedHere).then_some(progress_rx);
        self.loved_count = None;
        std::thread::spawn(move || {
            // The website and the loved list come all at once.
            let found = match (category, loved) {
                (Category::LovedHere, None) => {
                    Err("the taiko.rs server hasn't said which maps are loved there yet".into())
                }
                (Category::LovedHere, Some(ids)) => {
                    let progress = std::sync::Mutex::new(progress_tx);
                    download::loved_here(&ids, &query, sort, desc, &|sets, done, of| {
                        let _ = progress.lock().unwrap().send((sets, done, of));
                    })
                    .map(|sets| (sets, false))
                }
                (c, _) if c.is_custom() => {
                    crate::custom::search(&query, c == Category::CustomApproved, sort, desc)
                        .map(|sets| (sets, false))
                }
                _ => download::search(&query, category, sort, desc, page),
            };
            let _ = tx.send(found);
        });
        self.search = Some((page, rx));
    }

    /// Fetches the next page of results, if there is one and nothing is on its way.
    pub fn load_more(&mut self) {
        if self.more
            && self.search.is_none()
            && self.edited.is_none()
            && let Some(query) = self.searched.clone()
        {
            self.fetch(query, self.page + 1);
        }
    }

    /// Fetches the next page of results once the selection gets near the end of the list.
    fn load_more_near_end(&mut self) {
        if self.more
            && self.search.is_none()
            && self.edited.is_none()
            && (self.cursor + 10 >= self.results.len() || self.near_end)
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

    /// The orders the current category can list in (the loved sets are osu!'s, with all of
    /// osu!'s details).
    pub fn sorts(&self) -> &'static [SortKey] {
        if self.category.is_custom() {
            &SortKey::OURS
        } else {
            &SortKey::ALL
        }
    }

    /// Takes the sets the taiko.rs server says are loved there, and shows them if the loved
    /// list is open (it may have been waiting for them).
    pub fn set_loved_here(&mut self, sets: impl IntoIterator<Item = u32>) {
        let sets: HashSet<u32> = sets.into_iter().collect();
        if self.loved_here.as_ref() == Some(&sets) {
            return;
        }
        self.loved_here = Some(sets);
        if self.category == Category::LovedHere && self.searched.is_some() {
            self.search_now();
        }
    }

    /// Whether `set` is an osu! set loved on the taiko.rs server.
    fn is_loved_here(&self, set: &SetInfo) -> bool {
        set.custom.is_none()
            && self
                .loved_here
                .as_ref()
                .is_some_and(|l| l.contains(&set.id))
    }

    /// Shows `sets` instead, with the same set selected if it's still there.
    fn replace_keeping_selection(&mut self, sets: Vec<SetInfo>) {
        let selected = self
            .results
            .get(self.cursor)
            .map(|s| (s.id, s.custom.is_some()));
        self.results = sets;
        self.cursor = selected
            .and_then(|key| {
                self.results
                    .iter()
                    .position(|s| (s.id, s.custom.is_some()) == key)
            })
            .unwrap_or(0);
    }

    /// Drops the results of another list, so they don't stand in for this one while it loads
    /// (or if it fails).
    fn forget_results(&mut self) {
        self.results.clear();
        self.cursor = 0;
        (self.page, self.more) = (0, false);
        (self.scroll_target, self.follow) = (0.0, true);
    }

    /// Keeps the order to one the current category has.
    fn fit_sort(&mut self) {
        if !self.sorts().contains(&self.sort) {
            self.sort = SortKey::Relevance;
            self.desc = true;
        }
    }

    /// Switches to the osu! mirrors or the taiko.rs website, in the category it had last.
    pub fn set_source(&mut self, source: Source) {
        if source == self.source() {
            return;
        }
        std::mem::swap(&mut self.category, &mut self.other_category);
        self.fit_sort();
        self.forget_results();
        self.search_now();
    }

    /// Opens drop-down `m`, or closes it if it's open.
    pub fn toggle_menu(&mut self, m: Menu) {
        self.menu = if self.menu == Some(m) { None } else { Some(m) };
    }

    /// Flips the order's direction.
    pub fn flip_sort(&mut self) {
        self.desc = !self.desc;
        self.search_now();
    }

    pub fn set_category(&mut self, c: Category) {
        self.menu = None;
        if c != self.category {
            self.category = c;
            self.fit_sort();
            self.forget_results();
            self.search_now();
        }
    }

    /// Picks an order; picking the current one again flips its direction.
    pub fn set_sort(&mut self, s: SortKey) {
        self.menu = None;
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
        // The loved list fills in as it loads: the selection stays where it is.
        if let Some(rx) = &self.loved_progress
            && self.search.is_some()
            && let Some((sets, done, of)) = rx.try_iter().last()
        {
            self.loved_count = Some((done, of));
            if !sets.is_empty() {
                // The count above the list says how far it got.
                self.status.clear();
            }
            self.replace_keeping_selection(sets);
        }
        let growing = self.loved_count.is_some();
        if let Some((page, rx)) = &self.search {
            let page = *page;
            if let Ok(result) = rx.try_recv() {
                self.search = None;
                self.loved_progress = None;
                self.loved_count = None;
                match result {
                    Ok((sets, more)) if page == 0 => {
                        self.status = if sets.is_empty() {
                            "no taiko maps found".into()
                        } else {
                            String::new()
                        };
                        if growing {
                            self.replace_keeping_selection(sets);
                        } else {
                            (self.scroll_target, self.follow) = (0.0, true);
                            self.results = sets;
                            self.cursor = 0;
                        }
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
        while let Ok((name, ok)) = self.cover_rx.try_recv() {
            self.covers
                .insert(name, if ok { Cover::Ready } else { Cover::Missing });
        }
    }

    /// Asks for a result's cover, unless it's here, on its way or missing, or enough are.
    fn want_cover(&mut self, set: &SetInfo) {
        let name = download::cover_name(set);
        if self.covers.contains_key(&name) {
            return;
        }
        if download::covers_dir().join(&name).is_file() {
            self.covers.insert(name, Cover::Ready);
            return;
        }
        let fetching = self
            .covers
            .values()
            .filter(|c| **c == Cover::Fetching)
            .count();
        if fetching >= COVER_FETCHES {
            return;
        }
        self.covers.insert(name.clone(), Cover::Fetching);
        let (tx, set) = (self.cover_tx.clone(), set.clone());
        std::thread::spawn(move || {
            let _ = tx.send((name, download::fetch_cover(&set).is_ok()));
        });
    }

    /// The mouse wheel: scrolls the list without moving the selection.
    pub fn scroll_by(&mut self, notches: i32) {
        self.follow = false;
        self.scroll_target -= notches as f32 * ROW * 1.5;
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
        self.follow = true;
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
            match (source, self.category) {
                (Source::Osu, _) => "maps from catboy.best / osu.direct",
                (Source::Ours, Category::LovedHere) => {
                    "osu! maps loved on taiko.rs (they give pp here), from the mirrors"
                }
                (Source::Ours, _) => "custom maps from the taiko.rs website",
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
            let hint = if self.category.is_custom() {
                "search song, mapper, sender..."
            } else {
                "search artist, title, mapper..."
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

        // The filter row: status and order as drop-downs, the order's direction, the count.
        let fy = 84.0;
        let status_x = x0;
        let status_w = dropdown(
            c,
            status_x,
            fy,
            "STATUS",
            self.category.name(),
            Hit::DlMenu(0),
            self.menu == Some(Menu::Status),
        );
        let sort_x = status_x + status_w + 6.0;
        let custom = self.category.is_custom();
        let sort_name = |key: SortKey| {
            if custom {
                key.name_for_ours()
            } else {
                key.name()
            }
        };
        let sort_w = dropdown(
            c,
            sort_x,
            fy,
            "SORT",
            sort_name(self.sort),
            Hit::DlMenu(1),
            self.menu == Some(Menu::Sort),
        );
        let mut right_of_filters = sort_x + sort_w;
        if self.sort != SortKey::Relevance {
            let (dx, dw) = (right_of_filters + 4.0, 20.0);
            let hover = c.hovered(dx, fy, dw, 20.0);
            c.round_rect(
                [dx, fy],
                [dw, 20.0],
                5.0,
                rgba(if hover { 0x34344a } else { 0x23232f }, 0.95),
                rgba(0x3a3a4e, 1.0),
                1.0,
            );
            chevron(
                c,
                dx + dw / 2.0,
                fy + 10.0,
                3.5,
                self.desc,
                rgba(0xc8c8d8, 1.0),
            );
            c.hit(dx, fy, dw, 20.0, Hit::DlSortDir);
            right_of_filters = dx + dw;
        }
        if !self.results.is_empty() {
            let n = self.results.len();
            let count = match self.loved_count {
                Some((done, of)) => format!("{done} of {of} loaded"),
                None => format!(
                    "{n}{} map{}",
                    if self.more { "+" } else { "" },
                    if n == 1 { "" } else { "s" }
                ),
            };
            if right_of_filters + Canvas::text_width(7.0, &count) + 12.0 < x0 + col {
                c.text_right(x0 + col - 4.0, fy + 7.0, 7.0, rgba(0x8a8aa0, 1.0), &count);
            }
        }

        // The downloads panel under the list, when something is downloading or failed.
        let shown_jobs = jobs.len().min(PANEL_ROWS);
        let shown_failed = self.failed_jobs.len().min(PANEL_ROWS - shown_jobs);
        let panel_rows = shown_jobs + shown_failed;
        let panel_h = if panel_rows > 0 {
            18.0 + panel_rows as f32 * 18.0
        } else {
            0.0
        };
        let (top, bottom) = (LIST_TOP, LIST_BOTTOM - panel_h);
        let view = bottom - top;
        // A last row says whether more are coming.
        let rows = self.results.len() + usize::from(!self.results.is_empty());
        let max_scroll = (rows as f32 * ROW - view).max(0.0);
        if self.follow {
            // Just far enough to keep the selection in view.
            let sel = self.cursor as f32 * ROW;
            if sel < self.scroll_target {
                self.scroll_target = sel;
            } else if sel + ROW > self.scroll_target + view {
                self.scroll_target = sel + ROW - view;
            }
        }
        self.scroll_target = self.scroll_target.clamp(0.0, max_scroll);
        approach(&mut self.scroll, self.scroll_target, c.dt_ms, 50.0);
        self.near_end = self.scroll_target + view >= rows as f32 * ROW - 3.0 * ROW;
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
        let mut covers_wanted = Vec::new();
        let action_w = if col < 420.0 { 64.0 } else { 84.0 };
        for (i, set) in self.results.iter().enumerate() {
            let y = top + i as f32 * ROW - self.scroll;
            if y + ROW < top || y > bottom {
                continue;
            }
            let card_h = ROW - 4.0;
            let selected = i == self.cursor;
            let hover = !selected && c.hovered(x0, y, col, card_h);
            let bg = if selected {
                0x2b2b3d
            } else if hover {
                0x262634
            } else {
                0x1d1d27
            };
            c.round_rect(
                [x0, y],
                [col, card_h],
                7.0,
                rgba(bg, 0.95),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            if selected {
                crate::style::selector(c, "download", [x0 - 3.0, y - 3.0, col + 6.0, card_h + 6.0]);
            }
            c.hit(x0, y, col, card_h, Hit::DlRow(i));

            // The cover, cut to a square on the left.
            let th = card_h - 8.0;
            let (tx, ty) = (x0 + 4.0, y + 4.0);
            let name = download::cover_name(set);
            let drawn = self.covers.get(&name) == Some(&Cover::Ready) && {
                let start = c.shapes.len();
                let file = format!("{}{name}", crate::pictures::COVER_PREFIX);
                let ok = c.picture(&file, tx + th / 2.0, ty + th / 2.0, th, 0.0, [1.0; 4]);
                c.clip_since(start, tx, ty, th, th);
                ok
            };
            if !drawn {
                c.round_rect(
                    [tx, ty],
                    [th, th],
                    5.0,
                    rgba(0x15151d, 1.0),
                    rgba(0x2a2a38, 1.0),
                    1.0,
                );
                let tag = if set.custom.is_some() { "rs" } else { "osu" };
                c.text_centered(
                    tx + th / 2.0,
                    ty + th / 2.0 - 3.0,
                    6.0,
                    rgba(0x4a4a5e, 1.0),
                    tag,
                );
            }
            if !self.covers.contains_key(&name) {
                covers_wanted.push(i);
            }

            let state = if installed(set) {
                RowState::Installed
            } else if let Some(job) = jobs.iter().find(|j| j.set.same(set)) {
                RowState::Downloading(job.fraction())
            } else {
                RowState::Available
            };
            let right = x0 + col - 8.0;
            let (bx, by, bh) = (right - action_w, y + (card_h - 20.0) / 2.0, 20.0);
            match state {
                RowState::Installed => {
                    c.button(bx, by, action_w, bh, "Play", 8.0, Hit::DlPlay(i), selected);
                    c.text_centered(
                        bx + action_w / 2.0,
                        by + bh + 2.0,
                        5.0,
                        rgba(0x86E05A, 1.0),
                        "installed",
                    );
                }
                RowState::Downloading(f) => progress(c, bx, by + 4.0, action_w, f),
                RowState::Available => c.button(
                    bx,
                    by,
                    action_w,
                    bh,
                    "Download",
                    8.0,
                    Hit::DlGet(i),
                    selected,
                ),
            }

            // Title, artist and mapper, then the status, the difficulties and the stars.
            let tx2 = tx + th + 8.0;
            let room = bx - 8.0 - tx2;
            c.text(
                tx2,
                y + 4.0,
                9.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(9.0, room, &set.title),
            );
            let by_line = format!("{}  |  mapped by {}", set.artist, set.creator);
            c.text(
                tx2,
                y + 18.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(7.0, room, &by_line),
            );
            let ly = y + 30.0;
            let (label, color) = status_look(set, self.is_loved_here(set));
            let mut lx = tx2 + pill(c, tx2, ly, label, color) + 6.0;
            let chosen = selected.then(|| {
                self.diff_choice
                    .get(&set.id)
                    .copied()
                    .unwrap_or(set.diffs.len().saturating_sub(1))
            });
            const DOTS: usize = 10;
            for (k, d) in set.diffs.iter().enumerate().take(DOTS) {
                if lx + 8.0 > tx2 + room {
                    break;
                }
                let stars = match set.custom {
                    Some(_) => self.rated.get(&(true, set.id, d.id)).copied(),
                    None => Some(d.stars),
                };
                let fill = stars
                    .filter(|s| *s > 0.0)
                    .map_or(0x5a5a70, crate::stars::color);
                let ring = if chosen == Some(k) {
                    rgba(0xffffff, 1.0)
                } else {
                    rgba(0, 0.0)
                };
                c.circle(lx + 3.5, ly + 5.0, 3.5, rgba(fill, 1.0), ring, 1.2);
                lx += 9.0;
            }
            if set.diffs.len() > DOTS && lx + 20.0 < tx2 + room {
                lx += c.text(
                    lx,
                    ly + 2.0,
                    6.0,
                    rgba(0x8a8aa0, 1.0),
                    &format!("+{}", set.diffs.len() - DOTS),
                ) + 4.0;
            }
            let extra = match &set.custom {
                Some(custom) => format!(
                    "{} download{}  |  {:.1} MB",
                    set.play_count,
                    if set.play_count == 1 { "" } else { "s" },
                    custom.bytes as f64 / (1 << 20) as f64
                ),
                None => {
                    let lo = set.diffs.first().map_or(0.0, |d| d.stars);
                    let hi = set.diffs.last().map_or(0.0, |d| d.stars);
                    if (hi - lo).abs() < 0.05 {
                        format!("{hi:.2}")
                    } else {
                        format!("{lo:.2} - {hi:.2}")
                    }
                }
            };
            let extra_room = tx2 + room - lx - 4.0;
            if extra_room > 30.0 {
                c.text(
                    lx + 4.0,
                    ly + 2.0,
                    6.0,
                    rgba(0x8a8aa0, 1.0),
                    &Canvas::fit(6.0, extra_room, &extra),
                );
            }
        }
        if !self.results.is_empty() {
            let y = top + self.results.len() as f32 * ROW - self.scroll;
            let n = self.results.len();
            let end = if self.search.is_some() {
                "loading more...".to_string()
            } else if self.category.is_custom() {
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
        for i in covers_wanted {
            if let Some(set) = self.results.get(i).cloned() {
                self.want_cover(&set);
            }
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
                    bottom - 12.0
                },
                9.0,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(9.0, col, msg),
            );
        }

        // The downloads panel: progress and Cancel, or why one failed and Retry.
        if panel_rows > 0 {
            let py = bottom + 4.0;
            c.round_rect(
                [x0, py],
                [col, panel_h - 4.0],
                7.0,
                rgba(0x15151d, 0.95),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            let more = jobs.len() + self.failed_jobs.len() - panel_rows;
            let title = if more > 0 {
                format!("DOWNLOADS  (+{more} more)")
            } else {
                "DOWNLOADS".to_string()
            };
            c.text(x0 + 8.0, py + 5.0, 6.0, rgba(0x8a8aa0, 1.0), &title);
            let mut ry = py + 15.0;
            let name_of = |s: &SetInfo| {
                if s.artist.is_empty() {
                    s.title.clone()
                } else {
                    format!("{} - {}", s.artist, s.title)
                }
            };
            for (k, job) in jobs.iter().enumerate().take(shown_jobs) {
                let bar_w = (col * 0.3).min(110.0);
                let bar_x = x0 + col - 8.0 - 16.0 - 6.0 - bar_w;
                c.text(
                    x0 + 8.0,
                    ry + 4.0,
                    7.0,
                    rgba(0xc8c8d8, 1.0),
                    &Canvas::fit(7.0, bar_x - x0 - 16.0, &name_of(&job.set)),
                );
                progress(c, bar_x, ry + 2.0, bar_w, job.fraction());
                close_button(c, x0 + col - 8.0 - 16.0, ry, Hit::DlCancel(k));
                ry += 18.0;
            }
            for (k, (set, why)) in self.failed_jobs.iter().enumerate().take(shown_failed) {
                let retry_w = 44.0;
                let retry_x = x0 + col - 8.0 - 16.0 - 6.0 - retry_w;
                let name = format!("failed: {}  ({why})", name_of(set));
                c.text(
                    x0 + 8.0,
                    ry + 4.0,
                    7.0,
                    rgba(0xFF6B6B, 1.0),
                    &Canvas::fit(7.0, retry_x - x0 - 16.0, &name),
                );
                c.button(
                    retry_x,
                    ry,
                    retry_w,
                    15.0,
                    "Retry",
                    7.0,
                    Hit::DlRetry(k),
                    false,
                );
                close_button(c, x0 + col - 8.0 - 16.0, ry, Hit::DlDismiss(k));
                ry += 18.0;
            }
        }

        let active = jobs.len();
        let footer = if active > 0 {
            format!("{active} downloading  |  saving to {songs_dir}")
        } else {
            format!("saving to {songs_dir}")
        };
        let hints = "Up/Down select   Left/Right difficulty   Enter download / play   Ctrl+Left/Right status   Ctrl+Up/Down sort   Ctrl+Tab osu! / taiko.rs   Esc back";
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

        // An open drop-down goes over everything; a click anywhere else closes it.
        if let Some(menu) = self.menu {
            c.hit(0.0, 0.0, w, 480.0, Hit::DlMenuClose);
            let (mx, items): (f32, Vec<(String, bool, Hit)>) = match menu {
                Menu::Status => (
                    status_x,
                    self.categories()
                        .iter()
                        .enumerate()
                        .map(|(i, k)| {
                            (
                                k.name().to_string(),
                                *k == self.category,
                                Hit::DlCategory(i),
                            )
                        })
                        .collect(),
                ),
                Menu::Sort => (
                    sort_x,
                    self.sorts()
                        .iter()
                        .enumerate()
                        .map(|(i, k)| (sort_name(*k).to_string(), *k == self.sort, Hit::DlSort(i)))
                        .collect(),
                ),
            };
            let mw = items
                .iter()
                .map(|(l, _, _)| Canvas::text_width(8.0, l))
                .fold(90.0_f32, f32::max)
                + 24.0;
            let my = fy + 24.0;
            let mh = items.len() as f32 * 18.0 + 6.0;
            c.round_rect(
                [mx, my],
                [mw, mh],
                7.0,
                rgba(0x1b1b25, 0.98),
                rgba(0x4a4a60, 1.0),
                1.0,
            );
            for (k, (label, on, hit)) in items.iter().enumerate() {
                let ry = my + 3.0 + k as f32 * 18.0;
                let hover = c.hovered(mx + 3.0, ry, mw - 6.0, 18.0);
                if *on || hover {
                    c.round_rect(
                        [mx + 3.0, ry],
                        [mw - 6.0, 18.0],
                        5.0,
                        rgba(
                            if *on { accent() } else { 0x34344a },
                            if *on { 0.9 } else { 1.0 },
                        ),
                        rgba(0, 0.0),
                        0.0,
                    );
                }
                c.text(mx + 10.0, ry + 5.5, 8.0, rgba(0xffffff, 1.0), label);
                c.hit(mx + 3.0, ry, mw - 6.0, 18.0, *hit);
            }
        }
    }
}

/// A drop-down button of the filter row: a small `title`, then the `value` and a chevron.
/// Returns its width.
fn dropdown(c: &mut Canvas, x: f32, y: f32, title: &str, value: &str, hit: Hit, open: bool) -> f32 {
    let w = Canvas::text_width(6.0, title) + Canvas::text_width(8.0, value) + 32.0;
    let hover = c.hovered(x, y, w, 20.0);
    c.round_rect(
        [x, y],
        [w, 20.0],
        5.0,
        rgba(if open || hover { 0x34344a } else { 0x23232f }, 0.95),
        rgba(if open { accent() } else { 0x3a3a4e }, 1.0),
        1.0,
    );
    let tx = x + 8.0 + c.text(x + 8.0, y + 7.0, 6.0, rgba(0x8a8aa0, 1.0), title) + 6.0;
    c.text(tx, y + 6.0, 8.0, rgba(0xffffff, 1.0), value);
    chevron(c, x + w - 10.0, y + 10.0, 3.0, !open, rgba(0xc8c8d8, 1.0));
    c.hit(x, y, w, 20.0, hit);
    w
}

/// A small chevron centred on (`x`, `y`), `r` from its tip to its arms: pointing down, or up.
fn chevron(c: &mut Canvas, x: f32, y: f32, r: f32, down: bool, color: [f32; 4]) {
    let dy = if down { -r * 0.25 } else { r * 0.25 };
    for side in [-1.0_f32, 1.0] {
        let angle = side * if down { -0.785 } else { 0.785 };
        c.shapes.push(crate::render::Shape {
            center: [x + side * r * 0.5, y + dy],
            half: [r * 0.72, 0.6],
            fill: color,
            border: color,
            params: [0.6, 0.0, 0.0, 0.0],
            angle,
        });
    }
}

/// A progress bar `w` wide at (`x`, `y`) with the percentage on it ("starting..." until the
/// size is known).
fn progress(c: &mut Canvas, x: f32, y: f32, w: f32, f: Option<f32>) {
    c.round_rect(
        [x, y],
        [w, 12.0],
        4.0,
        rgba(0x15151d, 1.0),
        rgba(0x3a3a4e, 1.0),
        1.0,
    );
    if let Some(f) = f {
        c.round_rect(
            [x + 1.0, y + 1.0],
            [(w - 2.0) * f.clamp(0.0, 1.0), 10.0],
            3.0,
            rgba(0x438EAD, 1.0),
            rgba(0, 0.0),
            0.0,
        );
    }
    let label = f
        .map(|f| format!("{:.0}%", f * 100.0))
        .unwrap_or_else(|| "starting...".into());
    c.text_centered(x + w / 2.0, y + 3.0, 6.0, rgba(0xffffff, 1.0), &label);
}

/// A small square button with an x, 16 wide at (`x`, `y`).
fn close_button(c: &mut Canvas, x: f32, y: f32, hit: Hit) {
    let hover = c.hovered(x, y, 16.0, 15.0);
    c.round_rect(
        [x, y],
        [16.0, 15.0],
        4.0,
        rgba(if hover { 0x4a2a30 } else { 0x23232f }, 1.0),
        rgba(0x3a3a4e, 1.0),
        1.0,
    );
    c.text_centered(x + 8.0, y + 4.5, 7.0, rgba(0xc8c8d8, 1.0), "x");
    c.hit(x, y, 16.0, 15.0, hit);
}

/// A set's status as a word and a colour: osu!'s, the website's for a custom set, or loved
/// here (`loved_here`) whatever osu! says.
fn status_look(set: &SetInfo, loved_here: bool) -> (&'static str, u32) {
    match &set.custom {
        Some(c) if c.approved => ("approved", 0x86E05A),
        Some(_) => ("custom", 0xB28CFF),
        None if loved_here => ("loved here", 0xFF8AD0),
        None => match set.status.as_str() {
            "ranked" => ("ranked", 0x66CCFF),
            "approved" => ("approved", 0x66CCFF),
            "qualified" => ("qualified", 0x7FE0C8),
            "loved" => ("loved", 0xFF66AA),
            "pending" => ("pending", 0xFFD966),
            "wip" => ("wip", 0xFFB366),
            "graveyard" => ("graveyard", 0x8a8aa0),
            _ => ("unknown", 0x8a8aa0),
        },
    }
}

/// A status pill at (`x`, `y`), 10 tall. Returns its width.
fn pill(c: &mut Canvas, x: f32, y: f32, label: &str, color: u32) -> f32 {
    let w = Canvas::text_width(5.5, label) + 8.0;
    c.round_rect(
        [x, y],
        [w, 10.0],
        4.0,
        rgba(color, 0.16),
        rgba(color, 0.85),
        1.0,
    );
    c.text(x + 4.0, y + 2.5, 5.5, rgba(color, 1.0), label);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: u32) -> SetInfo {
        SetInfo {
            id,
            ..SetInfo::default()
        }
    }

    #[test]
    fn a_growing_list_keeps_the_same_set_selected() {
        let mut b = Browse {
            results: vec![set(3), set(1)],
            cursor: 1,
            ..Browse::default()
        };
        b.replace_keeping_selection(vec![set(5), set(3), set(2), set(1)]);
        assert_eq!(b.results[b.cursor].id, 1);
        b.replace_keeping_selection(vec![set(7)]);
        assert_eq!(b.cursor, 0);
    }

    #[test]
    fn the_loved_list_waits_for_the_server_and_then_loads() {
        let mut b = Browse::default();
        b.set_category(Category::LovedHere);
        assert!(b.results.is_empty());
        let (_, rx) = b.search.take().unwrap();
        let waited = rx.recv().unwrap();
        assert!(waited.is_err_and(|e| e.contains("hasn't said")));
        // Nothing loved: no mirror is asked.
        b.set_loved_here([]);
        let (_, rx) = b.search.take().unwrap();
        assert_eq!(rx.recv().unwrap().map(|(sets, _)| sets.len()), Ok(0));
        // The same answer again doesn't search again.
        b.set_loved_here([]);
        assert!(b.search.is_none());
    }

    #[test]
    fn custom_orders_dont_carry_over_to_other_lists() {
        let mut b = Browse {
            category: Category::LovedHere,
            sort: SortKey::Difficulty,
            ..Browse::default()
        };
        // The loved sets are osu!'s: every osu! order works.
        b.fit_sort();
        assert_eq!(b.sort, SortKey::Difficulty);
        b.category = Category::Custom;
        b.fit_sort();
        assert_eq!((b.sort, b.desc), (SortKey::Relevance, true));
    }
}
