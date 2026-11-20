//! Song select and results screens.

use winit::keyboard::KeyCode;

use crate::game::{Stats, Windows};
use crate::hit::{Canvas, Hit};
use crate::library::Entry;
use crate::render::{Glide, accent, progress, rgba};

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
    /// The folder a song came from: a TJA pack's folders, else taiko.rs's maps or osu!'s.
    Folder,
}

impl SortMode {
    pub const ALL: [SortMode; 11] = [
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
        SortMode::Folder,
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
            SortMode::Folder => "folder",
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
        matches!(
            self,
            SortMode::Difficulty | SortMode::Length | SortMode::Bpm
        )
    }
}

/// The star heading of a star rating: its whole stars, 10 for all above, -1 for not known.
fn star_heading(stars: f64) -> i64 {
    if stars.is_nan() {
        -1
    } else {
        (stars.floor() as i64).min(10)
    }
}

/// The folder map `e` came from, as the list sorted by folder heads it, with its place among
/// them (TJA folders first). A TJA chart's is the folder its song's folder is in, below the TJA
/// folder (`Pack / Genre`), so a pack's songs and genres stay together however deep they are.
fn source_folder(e: &Entry) -> (u8, String) {
    if e.is_tja() {
        let file = crate::tja_maps::split(&e.path).map_or_else(|| e.path.clone(), |(f, _)| f);
        let song = file.parent().unwrap_or(&file);
        let group = song.parent().unwrap_or(song);
        let names: Vec<String> = crate::tja_maps::root_of(&file)
            .and_then(|root| {
                group
                    .strip_prefix(root)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .map(|rel| {
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        return match names.is_empty() {
            true => (0, "TJA".to_string()),
            false => (0, names.join(" / ")),
        };
    }
    if e.is_own() {
        (1, "taiko.rs maps".to_string())
    } else {
        (2, "osu! Songs".to_string())
    }
}

/// What a song is ordered by when sorted by folder: its folder (`source_folder`), then its title
/// and artist.
type FolderKey<'a> = ((u8, String), (&'a str, &'a str));

/// A heading of the song list and the songs (`Select::groups` indices) under it.
struct Section {
    label: String,
    /// None: the accent colour.
    colour: Option<u32>,
    groups: std::ops::Range<usize>,
}

#[cfg(test)]
mod list_layout_tests;

pub mod lens;
pub use lens::{Facets, Filters, LensItem, LensUi, Order, Source, Tabs, Token, course_colour};

/// Where the song list's rows go, worked out from the headings alone: a song's place is
/// arithmetic (every song row is the same height), so a library of any size costs the same per
/// frame as a small one. `extras` are the few songs that are taller than a row (the open one's
/// difficulties, and the one closing), as (index into `groups`, extra height), in order.
struct ListLayout<'a> {
    sections: &'a [Section],
    headings: bool,
    song_h: f32,
    heading_h: f32,
    extras: Vec<(usize, f32)>,
    /// Where each section starts (its heading, if it has one).
    tops: Vec<f32>,
    /// The list's whole height.
    total: f32,
}

impl<'a> ListLayout<'a> {
    fn new(
        sections: &'a [Section],
        open: &[bool],
        headings: bool,
        (song_h, heading_h): (f32, f32),
        extras: Vec<(usize, f32)>,
    ) -> Self {
        let mut tops = Vec::with_capacity(sections.len());
        let mut y = 0.0;
        for (si, sec) in sections.iter().enumerate() {
            tops.push(y);
            if headings {
                y += heading_h;
            }
            if open[si] {
                y += sec.groups.len() as f32 * song_h;
                y += extras
                    .iter()
                    .filter(|(g, _)| sec.groups.contains(g))
                    .map(|(_, e)| e)
                    .sum::<f32>();
            }
        }
        ListLayout {
            sections,
            headings,
            song_h,
            heading_h,
            extras,
            tops,
            total: y,
        }
    }

    /// The top of song `g`'s row, in open section `si`.
    fn song_y(&self, si: usize, g: usize) -> f32 {
        let sec = &self.sections[si];
        let head = if self.headings { self.heading_h } else { 0.0 };
        let before: f32 = self
            .extras
            .iter()
            .filter(|(e, _)| sec.groups.contains(e) && *e < g)
            .map(|(_, e)| e)
            .sum();
        self.tops[si] + head + (g - sec.groups.start) as f32 * self.song_h + before
    }

    /// The height a song takes besides its own row.
    fn extra(&self, g: usize) -> f32 {
        self.extras
            .iter()
            .find(|(e, _)| *e == g)
            .map_or(0.0, |(_, e)| *e)
    }

    /// The songs of open section `si` that reach into `from..to`.
    fn visible_songs(&self, si: usize, from: f32, to: f32) -> std::ops::Range<usize> {
        let groups = &self.sections[si].groups;
        let ends = |g: usize| self.song_y(si, g) + self.song_h + self.extra(g);
        // The first song that ends at or below `from` is found by halving (rows only get lower).
        let (mut lo, mut hi) = (groups.start, groups.end);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if ends(mid) < from {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let first = lo;
        let last = first
            + (first..groups.end)
                .take_while(|&g| self.song_y(si, g) <= to)
                .count();
        first..last
    }
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
    pub server_loved: ServerLoved,
    /// Map folders sent to (or downloaded from) taiko.rs custom maps: their set number there.
    pub custom_folder: std::collections::HashMap<std::path::PathBuf, u32>,
    /// Custom sets the server has approved.
    pub custom_approved: std::collections::HashSet<u32>,
    /// The maps marked EPIC on the online server.
    pub server_epic: ServerEpic,
    /// The account logged in is a loved nominator: the right-click menu can love sets.
    pub nominator: bool,
    /// The account logged in is an admin: the right-click menu can mark maps EPIC.
    pub admin: bool,
    /// The collections: the player's own (all of them), then osu!'s that have maps here.
    pub collections: Vec<Shown>,
    /// The collection shown (None: everything) and the open list's cursor.
    pub collection: Option<usize>,
    pub picker: Option<usize>,
    /// What the open list is.
    pub list: PickList,
    /// A collection's name being typed.
    pub naming: Option<Naming>,
    /// The collections list offers to export the player's own to osu!'s `collection.db`
    /// (there's an osu! install, and collections of their own).
    pub can_export: bool,
    /// Files read so far while the library is still being scanned.
    pub scanning: Option<usize>,
    /// The folder dialog is open (from the button shown when there are no maps at all).
    pub picking_folder: bool,
    /// Speed modifier shown in the header ("" at 1.0x).
    pub rate_label: String,
    /// The mods picked ("HDHR"), shown on the Mods button.
    pub mods_label: String,
    pub show_converts: bool,
    /// A difficulty or a whole set asked to be deleted (right-click → Delete, the Delete key):
    /// the confirmation over song select.
    pub deleting: Option<DeleteAsk>,
    /// Moving taiko.rs's maps out of osu!'s Songs folder, offered once over song select.
    pub moving: Option<MoveAsk>,
    /// The folders of the maps whose own skin is turned off (right-click → Turn off skin).
    pub map_skins_off: std::collections::HashSet<String>,
    /// The right-click menu of a song list row.
    pub menu: Option<SongMenu>,
    /// The songs liked (the music player's favourites).
    pub favourites: std::collections::HashSet<String>,
    /// Sets checked for a newer version on osu! this session, by set ID.
    pub updates: std::collections::HashMap<u32, SetUpdate>,
    /// Custom sets checked for a newer version on the website, by their number there.
    pub custom_updates: std::collections::HashMap<u32, SetUpdate>,
    /// Difficulties an update took out of their set (renamed or dropped on osu!): not listed.
    pub gone: std::collections::HashSet<std::path::PathBuf>,
    /// Difficulties osu! doesn't have though their set is on it (edited copies: OD changed, a
    /// rate baked in): not submitted, whatever the set's status.
    pub edited: std::collections::HashSet<std::path::PathBuf>,
    /// Maps just added: listed whatever the filters say, until they change (`pinned_view`).
    pinned: std::collections::HashSet<std::path::PathBuf>,
    /// The filters `pinned` was made under.
    pinned_view: View,
    /// The list being dragged with the left button.
    drag: Option<ListDrag>,
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
    /// The headings (by name) the other way round from how this list starts (`starts_open`):
    /// only the player's clicks and the selection change them. Scrolling never does.
    flipped: std::collections::HashSet<String>,
    /// What `flipped` was before a search, which opens everything: it comes back with the search
    /// cleared.
    flipped_before_search: Option<std::collections::HashSet<String>>,
    /// The sort `flipped` is for (another starts over).
    open_for: SortMode,
    /// Songs are listed under headings (`sections`). Off: one list in the sort's order, each
    /// difficulty on its own row when sorting by difficulty.
    pub headings: bool,
    /// The source listed, its order and filters, and each other source's lens as left.
    pub tabs: Tabs,
    /// Maps from each source (`Source::index`; all at 0) the list could show: not counting
    /// converts while they're hidden.
    pub source_maps: [usize; 4],
    /// What each source's tab lists, the search and filters as they are (all at 0).
    pub source_listed: [usize; 4],
    /// The shown tab's counts for each filter's choices: counted while the sort and filter
    /// panel is open.
    pub facets: Facets,
    /// The sort and filter panel, while it's open.
    pub lens_ui: Option<LensUi>,
    /// When the source last changed: the list's rows come in for it.
    source_switched: Option<std::time::Instant>,
    /// Songs (folders) listed: `groups` counts difficulties instead in a list without headings
    /// by difficulty.
    song_count: usize,
    /// The heading the selection was last under.
    cursor_label: Option<String>,
    /// The heading the selection opened by moving in: it closes when the selection moves out.
    /// One the player opened stays until they close it.
    auto_open: Option<String>,
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
    /// The list moved by hand, away from the selection. Left out, it follows the selection.
    free: Option<FreeScroll>,
    /// How far the list scrolls and the scrollbar's thumb, as last drawn.
    scroll_max: f32,
    thumb: f32,
    /// Where the pointer is while flying through the list (right drag, scrollbar): the heading
    /// passing by shows beside it.
    pub scrubbing: Option<f32>,
    /// Selections come faster than the list gets to them (`draw`): the banner's picture of
    /// where it's going isn't read ahead.
    hold_pictures: bool,
    /// Where the list was scrolled last frame, and how fast it's going (units a ms, smoothed):
    /// the cards' pictures ahead of it are read first.
    seen_scroll: Option<f32>,
    scroll_speed: f32,
    /// How far the routes behind lean away from the pointer now (virtual units, eased).
    pub parallax: [f32; 2],
    /// Songs visited with F2, so Shift+F2 can step back (library indices).
    random_history: Vec<usize>,
    rng: u64,
    /// The song list's share of the width (`Config::song_list_share`).
    pub list_share: f32,
    /// A banner takes the band under the panels (picking a map): they stop above it.
    pub banner: bool,
    /// The Legacy menu setting: tabs on top, keys spelled out at the bottom.
    pub legacy: bool,
    /// The way back leads to the main menu, not to a lobby.
    pub to_menu: bool,
    /// A mod's customization is open over the top of the screen: no way back to show.
    pub covered: bool,
    /// The main menu's look shows the library's counts and the status line (`Config::select_info`);
    /// the classic look always does.
    pub show_info: bool,
    /// On the way from the main menu (`menu_shift`): how far the list has come in
    /// (`Phases::cascade`), its cards dropping out of the search row one after another.
    pub arriving: Option<f32>,
    /// Where the list's own shapes start and end on the canvas, as last drawn (the search row's
    /// are before them, the scroll bar's and the rest after).
    pub list_shapes: [usize; 2],
}

/// A play's letter in its colour, the same as on the website.
pub(crate) fn grade_colour(grade: &str) -> u32 {
    match grade {
        "SS" | "S" => 0xFFD34D,
        "A" => 0x86E05A,
        "B" => 0x4fb4ff,
        "C" => 0xc77dff,
        _ => 0xEB452C,
    }
}

/// The filters on the song list: the search, the collection, whether converts show, the source
/// and the lens's filters.
type View = (String, Option<usize>, bool, Source, Filters);

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
    /// How lit each set's card is under the pointer (0 to 1), by its first row.
    card_light: std::collections::HashMap<usize, f32>,
    /// When each card picture (`render::path_picture_key`) was first drawn, to fade it in.
    covers: std::collections::HashMap<String, std::time::Instant>,
    /// When the transition before this one started (selections coming faster than they land).
    last_start: Option<std::time::Instant>,
    /// The selection last landed in the target, and when it did (its flash).
    landed: Option<usize>,
    flash: Option<std::time::Instant>,
    /// The set last opened (its first row) and when: its difficulties drop in one by one.
    opened_at: Option<(usize, std::time::Instant)>,
}

/// The list held by the pointer: it scrolls with it, then flies on and settles on a song.
struct ListDrag {
    /// Scroll when the drag began.
    scroll: f32,
    /// Scroll now, and recent (time, scroll) samples for the release speed.
    now: f32,
    samples: std::collections::VecDeque<(std::time::Instant, f32)>,
}

/// The list somewhere of the player's choosing. Selecting another song ends it.
struct FreeScroll {
    glide: Glide,
    /// How quickly it gets there (ms).
    tau: f32,
    /// The song that was selected (its folder).
    song: Option<std::path::PathBuf>,
}

/// How far a notch of the wheel moves the list, and how quickly: slow enough that the cards are
/// seen gliding, not jumping three rows on (which, the cards all alike, reads as standing still).
const WHEEL_STEP: f32 = SONG_ROW_H * 3.0;
const WHEEL_TAU: f32 = 110.0;
/// How far a notch past an end of the list pushes it before it springs back.
const WHEEL_END_BUMP: f32 = 10.0;
/// From a list moved by hand to a song picked in it: a longer glide the farther it goes.
const FROM_FREE_MS: (f32, f32) = (260.0, 520.0);
const SCRUB_TAU: f32 = 20.0;
const THROW_TAU: f32 = 130.0;

/// Length of a song list transition.
const LIST_ANIM_MS: f32 = 200.0;
/// Top and bottom of the song list (virtual units): with the tabs of the Legacy menu above it,
/// and without.
const LIST_TOP_LEGACY: f32 = 86.0;
const LIST_TOP: f32 = 52.0;
pub use crate::config::LIST_SHARE;
/// The narrowest the song list and the panel beside it get (virtual units).
const LIST_MIN: f32 = 260.0;
const PANEL_MIN: f32 = 220.0;
/// Height of a song's own row in the list (virtual units).
pub const SONG_ROW_H: f32 = 34.0;

/// The cards of the main menu's look: how far in a set's card sits unless it's open, how far one
/// under the pointer comes out, how bright the sets not open are, the corners, how long a card's
/// picture takes to fade in, the shade over it (left, under the title, to right), how far up the
/// list's bottom the last cards fade out, and how bright a difficulty's bar is unless selected.
const CARD_INSET: f32 = 12.0;
const CARD_HOVER: f32 = 4.0;
const CARD_DIM: f32 = 0.7;
const CARD_RADIUS: f32 = crate::style::radius::CARD;
/// How bright the faint edge round a card is (not the open one, which has the selection's).
const CARD_EDGE: f32 = 0.1;
/// A set opening: its difficulties drop in from this far above, this many ms apart, out from
/// the selected one both ways, the last of them at most `DIFF_CASCADE_SPAN_MS` after it (a set
/// of many difficulties drops them in closer together, its selected one first).
const DIFF_DROP: f32 = 8.0;
const DIFF_CASCADE_MS: f32 = 30.0;
const DIFF_CASCADE_SPAN_MS: f32 = 120.0;
const COVER_FADE_MS: f32 = 200.0;
const COVER_SHADE: (f32, f32) = (0.85, 0.4);
const BOTTOM_FADE: f32 = 16.0;
const DIFF_BAR_DIM: f32 = 0.35;
/// How long after the list has got to a new selection the banner's picture is read ahead.
const PICTURE_SETTLE_MS: f32 = 60.0;
/// The cards' pictures read ahead of the list: as far as it goes in this long (ms) at its speed,
/// smoothed over this long.
const CARD_AHEAD_MS: f32 = 400.0;
const CARD_AHEAD_TAU: f32 = 80.0;

/// The list as a lane through the selection's target brackets: the cards stay straight near the
/// target (`ARC_STRAIGHT` of the list's height), then bend away from it along an arc, `ARC_K`
/// units per (list height)² beyond that, at most `ARC_MAX`, and fade by up to `ARC_FADE`
/// (all of it `ARC_FADE_REACH` of the list's height beyond the straight part). A song or
/// difficulty landing in the target flashes it for `LAND_FLASH_MS`, its brackets opening out
/// `LAND_FLASH_GROW` units, like a note hit on the menu's target.
const ARC_STRAIGHT: f32 = 0.12;
const ARC_K: f32 = 64.0;
const ARC_MAX: f32 = 24.0;
const ARC_FADE: f32 = 0.3;
const ARC_FADE_REACH: f32 = 0.5;
const LAND_FLASH_MS: f32 = 300.0;
const LAND_FLASH_GROW: f32 = 6.0;

/// What a song's name starts with, as its heading when the list is by artist or title.
fn initial(name: &str) -> String {
    match name.trim_start().chars().next() {
        Some(c) if c.is_ascii_digit() => "0-9".to_string(),
        Some(c) if c.is_alphabetic() && (c as u32) < 0x250 => c.to_uppercase().collect(),
        _ => "Other".to_string(),
    }
}

/// What TJA charts are marked with (`Entry::is_tja`).
pub(crate) const TJA_COLOUR: u32 = 0xff8a3d;

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
        CUSTOM => ("taiko.rs custom", 0x9b6cff),
        CUSTOM_APPROVED => ("Custom, approved", 0x4fd1b0),
        TJA => ("TJA charts", TJA_COLOUR),
        _ => ("Status not known yet", 0x8a8aa0),
    }
}
/// The legacy layout's list runs down to its hint line.
const LIST_BOTTOM_LEGACY: f32 = 440.0;
/// Its frame (4 below) runs down to just over the status line, or stops at 436 while a
/// banner (picking a map) takes the band under the panels.
const LIST_BOTTOM: f32 = 454.0;
const LIST_BOTTOM_BANNER: f32 = 432.0;

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
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        let mut s = Select {
            list_share: LIST_SHARE.2,
            banner: false,
            legacy: false,
            to_menu: true,
            covered: false,
            show_info: false,
            arriving: None,
            list_shapes: [0, 0],
            filter: String::new(),
            searching: false,
            sort: SortMode::Artist,
            info: None,
            played: std::collections::HashMap::new(),
            status: std::collections::HashMap::new(),
            server_loved: ServerLoved::new(),
            custom_folder: std::collections::HashMap::new(),
            custom_approved: std::collections::HashSet::new(),
            custom_updates: std::collections::HashMap::new(),
            server_epic: ServerEpic::default(),
            nominator: false,
            admin: false,
            collections: Vec::new(),
            collection: None,
            picker: None,
            list: PickList::Sorts,
            naming: None,
            can_export: false,
            scanning: None,
            favourites: crate::playlist::load_favourites(),
            picking_folder: false,
            rate_label: String::new(),
            mods_label: String::new(),
            show_converts: !has_taiko,
            deleting: None,
            moving: None,
            map_skins_off: crate::config::load_map_skins_off(),
            menu: None,
            updates: std::collections::HashMap::new(),
            edited: std::collections::HashSet::new(),
            gone: std::collections::HashSet::new(),
            pinned: std::collections::HashSet::new(),
            pinned_view: Default::default(),
            drag: None,
            visible: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            sections: Vec::new(),
            section_of: Vec::new(),
            flipped: Default::default(),
            flipped_before_search: None,
            open_for: SortMode::Artist,
            headings: true,
            tabs: Tabs::default(),
            source_maps: [0; 4],
            source_listed: [0; 4],
            facets: Facets::default(),
            lens_ui: None,
            source_switched: None,
            song_count: 0,
            cursor_label: None,
            auto_open: None,
            closed_at: None,
            row_value: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            anim: ListAnim::default(),
            remembered: std::collections::HashMap::new(),
            hardest_pending: None,
            free: None,
            scroll_max: 0.0,
            thumb: 0.0,
            scrubbing: None,
            hold_pictures: false,
            seen_scroll: None,
            scroll_speed: 0.0,
            parallax: [0.0; 2],
            random_history: Vec::new(),
            rng: seed | 1,
        };
        s.refilter(library);
        s
    }

    /// Lists converts too if the settings say so (`Select::new` only does for a list without
    /// any taiko maps).
    pub fn keep_converts(&mut self, library: &[Entry], on: bool) {
        if on && !self.show_converts {
            self.show_converts = true;
            self.refilter(library);
        }
    }

    pub fn refilter(&mut self, library: &[Entry]) {
        if self.pinned_view != self.view() {
            self.pinned.clear();
        }
        // A search shows what it found: everything open, then back as it was.
        match (self.filter.is_empty(), self.flipped_before_search.take()) {
            (false, None) => {
                self.flipped_before_search = Some(std::mem::take(&mut self.flipped));
                self.auto_open = None;
            }
            (false, kept) => self.flipped_before_search = kept,
            (true, Some(kept)) => {
                self.flipped = kept;
                self.auto_open = None;
            }
            (true, None) => {}
        }
        let selected = self.visible.get(self.cursor).copied();
        let query = crate::search::Query::parse(&self.filter);
        let asks_epic = query.asks_epic();
        let only = self
            .collection
            .and_then(|c| self.collections.get(c))
            .map(|c| &c.maps);
        // `collection=` in the search: each the maps it takes in, and whether it's `!=`.
        let in_any = || -> std::collections::HashSet<usize> {
            self.collections
                .iter()
                .flat_map(|c| c.maps.iter().copied())
                .collect()
        };
        let by_collection: Vec<(bool, bool, std::collections::HashSet<usize>)> = query
            .collections()
            .map(|(name, negated)| match name {
                crate::search::InCollection::Named(name) => (
                    negated,
                    true,
                    self.collections
                        .iter()
                        .filter(|c| c.name.to_lowercase() == *name)
                        .flat_map(|c| c.maps.iter().copied())
                        .collect(),
                ),
                // In none: not in any.
                crate::search::InCollection::None => (negated, false, in_any()),
            })
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let info = self.info.as_ref().map(|i| i.lock().unwrap());
        let facts = |i: usize, e: &Entry| {
            let known = info.as_ref().and_then(|v| v.get(i).copied().flatten());
            crate::search::Facts {
                // A TJA course by its charter's level, as it shows.
                stars: e
                    .meta
                    .level
                    .map(f64::from)
                    .or(known.map(|x| x.stars as f64)),
                length_s: known.map(|x| x.length_ms as f64 / 1000.0),
                bpm: known.map(|x| x.bpm as f64),
                played_days: self
                    .played
                    .get(&e.path)
                    .filter(|&&t| t > 0)
                    .map(|&t| now.saturating_sub(t) as f64 / 86_400.0),
                status: self.status_of(e),
                epic: asks_epic && self.is_epic(e),
            }
        };
        let query_facts = query.needs_facts();
        let no_facts = crate::search::Facts::default();
        // The filters as each tab has them (`Filters::active_in`).
        let active = Source::ALL.map(|s| self.tabs.active(s));
        let lens_open = self.lens_ui.is_some();
        let filtering = !self.tabs.filters.is_empty() || lens_open;
        let picked = self.tabs.source;
        // One pass over the library: each source's maps (for its tab, and whether the picked one
        // has any), what each tab would list, and what's listed. While the panel is open, what
        // each filter's choices would list too, for each tab.
        let mut maps = [0; 4];
        let mut listed = [0; 4];
        let mut facets = [Facets::default(); 4];
        let mut visible = Vec::new();
        // What all sources list that the picked one doesn't, should the picked one have no maps.
        let mut others = Vec::new();
        for (i, e) in library.iter().enumerate() {
            if self.gone.contains(&e.path) {
                continue;
            }
            let listable = self.show_converts || e.meta.mode == 1;
            let source = listable.then(|| Source::of(e));
            if let Some(s) = source {
                maps[0] += 1;
                maps[s.index()] += 1;
                if lens_open && let Some(k) = lens::status_choice(self.status_of(e)) {
                    facets[0].has_status |= 1 << k;
                    facets[s.index()].has_status |= 1 << k;
                }
            }
            if self.pinned.contains(&e.path) {
                visible.push(i);
                continue;
            }
            let Some(source) = source else {
                continue;
            };
            if !(only.is_none_or(|set| set.contains(&i))
                && by_collection
                    .iter()
                    .all(|(negated, inside, set)| (set.contains(&i) == *inside) != *negated))
            {
                continue;
            }
            // A search by words alone needs no facts (they cost a few lookups by path each).
            let known = (query_facts || filtering).then(|| facts(i, e));
            if !query.is_empty()
                && !query.matches_text(&e.meta, e.text(), known.as_ref().unwrap_or(&no_facts))
            {
                continue;
            }
            // Whether its own tab lists it, and all.
            let (in_own, in_all) = match known.as_ref().filter(|_| filtering) {
                Some(f) => {
                    let course = e
                        .is_tja()
                        .then(|| crate::tja_maps::split(&e.path).map(|(_, k)| k))
                        .flatten();
                    let (own, all) = (&active[source.index()], &active[0]);
                    if lens_open {
                        facets[source.index()].count(own, f, course);
                        facets[0].count(all, f, course);
                    }
                    (own.keeps(f, course, None), all.keeps(f, course, None))
                }
                None => (true, true),
            };
            listed[source.index()] += in_own as usize;
            listed[0] += in_all as usize;
            if (picked == Source::All && in_all) || (picked == source && in_own) {
                visible.push(i);
            } else if in_all {
                others.push(i);
            }
        }
        drop(info);
        self.source_maps = maps;
        self.source_listed = listed;
        let shown = self.shown_source();
        if shown != picked {
            visible.extend(others);
            visible.sort_unstable();
        }
        self.visible = visible;
        self.facets = facets[shown.index()];
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

    /// Where the list is now, and the furthest it goes.
    #[cfg(test)]
    pub fn list_scroll(&self) -> f32 {
        self.anim.scroll
    }

    #[cfg(test)]
    pub fn list_end(&self) -> f32 {
        self.scroll_max
    }

    /// The listed songs in order, each as its difficulties' library indices.
    #[cfg(test)]
    pub fn songs(&self) -> Vec<Vec<usize>> {
        self.groups
            .iter()
            .map(|g| self.visible[g.clone()].to_vec())
            .collect()
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
        self.sections
            .iter()
            .map(|s| (s.label.clone(), s.groups.len()))
            .collect()
    }

    /// One past the last song of section `s`, as an index of `songs()`.
    #[cfg(test)]
    pub fn sections_end(&self, s: usize) -> usize {
        self.sections[s].groups.end
    }

    /// The headings whose songs are listed: the one holding the selection, unless closed.
    #[cfg(test)]
    pub fn open_sections(&self) -> Vec<usize> {
        (0..self.sections.len())
            .filter(|&s| self.section_open(s))
            .collect()
    }

    /// The taiko.rs custom set map `e` is in, if it's one.
    fn custom_id(&self, e: &Entry) -> Option<u32> {
        e.path
            .parent()
            .and_then(|d| self.custom_folder.get(d))
            .copied()
    }

    /// Whether a newer version of map `e`'s set is known of: on the website for a custom set
    /// (whatever osu! set it was made from), else on osu!.
    pub fn update_of(&self, e: &Entry) -> Option<&SetUpdate> {
        match self.custom_id(e) {
            Some(id) => self.custom_updates.get(&id),
            None => e.set_id().and_then(|id| self.updates.get(&id)),
        }
    }

    /// Whether map `e` is marked EPIC on the online server.
    pub fn is_epic(&self, e: &Entry) -> bool {
        self.server_epic.covers(e, || self.custom_id(e))
    }

    /// A map's status on osu! (`search`'s codes), loved here counting as loved.
    fn status_of(&self, e: &Entry) -> u8 {
        if e.is_tja() {
            return crate::search::TJA;
        }
        // A map in taiko.rs's own folder is never osu!'s, nor an edited copy of osu!'s.
        if self.edited.contains(&e.path) && !e.is_own() {
            return crate::search::NOT_SUBMITTED;
        }
        if let Some(id) = e.path.parent().and_then(|d| self.custom_folder.get(d)) {
            return if self.custom_approved.contains(id) {
                crate::search::CUSTOM_APPROVED
            } else {
                crate::search::CUSTOM
            };
        }
        if e.is_own() {
            return crate::search::NOT_SUBMITTED;
        }
        match e.set_id() {
            Some(_) if loved_here(&self.server_loved, e) => crate::search::LOVED,
            set => self
                .status
                .get(&e.path)
                .copied()
                .unwrap_or(if set.is_some() {
                    crate::search::UNKNOWN
                } else {
                    crate::search::NOT_SUBMITTED
                }),
        }
    }

    /// Orders songs by the sort mode, keeping each song's difficulties together (and in their
    /// order). A song's value is its highest (longest, most recent...). By difficulty each is
    /// listed by its own stars: a song is there once per star heading its difficulties are under.
    fn sort_visible(&mut self, library: &[Entry]) {
        self.sort_by_sort(library);
        self.order_songs(library);
    }

    /// The songs as `regroup` cuts them: ranges of `visible`.
    fn cuts(&self, library: &[Entry]) -> Vec<std::ops::Range<usize>> {
        let each = self.split_difficulties();
        let mut out = Vec::new();
        let mut start = 0;
        for i in 1..=self.visible.len() {
            let split = i == self.visible.len()
                || each
                // As bytes: `Path`'s own == goes component by component, and this runs for
                // every row of the list on each refilter.
                || library[self.visible[i]].set_key().as_os_str() != library[self.visible[i - 1]].set_key().as_os_str()
                // The last of one star heading and the first of the next can be the same song's.
                || (self.sort == SortMode::Difficulty && star_heading(self.row_value[i]) != star_heading(self.row_value[i - 1]));
            if split {
                out.push(start..i);
                start = i;
            }
        }
        out
    }

    /// Orders the songs under each heading by the lens's order (all of them without headings),
    /// the headings staying in the sort's order. Values not known go last either way.
    fn order_songs(&mut self, library: &[Entry]) {
        let (order, desc) = (self.tabs.order, self.tabs.desc);
        if order == Order::Sort && !desc {
            return;
        }
        let cuts = self.cuts(library);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        enum Key<'a> {
            Text(&'a str, &'a str),
            Value(f64),
        }
        let info = self.info.as_ref().map(|i| i.lock().unwrap());
        let known = |i: usize| info.as_ref().and_then(|v| v.get(i).copied().flatten());
        let most = |song: &std::ops::Range<usize>, value: &dyn Fn(usize) -> Option<f64>| {
            self.visible[song.clone()]
                .iter()
                .filter_map(|&i| value(i))
                .filter(|v| v.is_finite())
                .fold(f64::NAN, |a, b| if a.is_nan() || b > a { b } else { a })
        };
        let mut headings: std::collections::HashMap<String, usize> = Default::default();
        let mut keyed: Vec<(usize, Key, usize)> = Vec::with_capacity(cuts.len());
        for (k, song) in cuts.iter().enumerate() {
            let heading = if self.headings {
                self.section_heading(library, song.start, now).0
            } else {
                String::new()
            };
            let n = headings.len();
            let h = *headings.entry(heading).or_insert(n);
            let e = &library[self.visible[song.start]];
            let key = match order {
                Order::Sort => Key::Value(k as f64),
                Order::Title => Key::Text(&e.text().title, &e.text().artist),
                Order::Artist => Key::Text(&e.text().artist, &e.text().title),
                Order::Stars => Key::Value(most(song, &|i| {
                    library[i]
                        .meta
                        .level
                        .map(f64::from)
                        .or(known(i).map(|x| x.stars as f64))
                })),
                Order::Length => Key::Value(most(song, &|i| known(i).map(|x| x.length_ms as f64))),
                Order::Bpm => Key::Value(most(song, &|i| known(i).map(|x| x.bpm as f64))),
                Order::Added => Key::Value(most(song, &|i| Some(library[i].modified as f64))),
                Order::Played => Key::Value(most(song, &|i| {
                    self.played.get(&library[i].path).map(|&t| t as f64)
                })),
            };
            keyed.push((h, key, k));
        }
        keyed.sort_by(|a, b| {
            let by_key = match (&a.1, &b.1) {
                (Key::Text(x0, x1), Key::Text(y0, y1)) => {
                    let o = (x0, x1).cmp(&(y0, y1));
                    if desc { o.reverse() } else { o }
                }
                (Key::Value(x), Key::Value(y)) => match (x.is_nan(), y.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if desc => y.total_cmp(x),
                    _ => x.total_cmp(y),
                },
                _ => std::cmp::Ordering::Equal,
            };
            a.0.cmp(&b.0).then(by_key).then(a.2.cmp(&b.2))
        });
        let order: Vec<usize> = keyed.into_iter().map(|(_, _, k)| k).collect();
        drop(info);
        let (mut visible, mut values) = (Vec::with_capacity(self.visible.len()), Vec::new());
        for k in order {
            visible.extend_from_slice(&self.visible[cuts[k].clone()]);
            values.extend_from_slice(&self.row_value[cuts[k].clone()]);
        }
        self.visible = visible;
        self.row_value = values;
    }

    /// Orders songs by the sort mode (`sort_visible`, before the lens's order).
    fn sort_by_sort(&mut self, library: &[Entry]) {
        if self.sort == SortMode::Artist {
            // The library is already in artist / title order.
            self.row_value = vec![f64::NAN; self.visible.len()];
            return;
        }
        let info = self.info.as_ref().map(|i| i.lock().unwrap());
        let value = |i: usize| -> f64 {
            let info = || info.as_ref().and_then(|v| v.get(i).copied().flatten());
            match self.sort {
                SortMode::Difficulty => match library[i].meta.level {
                    Some(level) => level as f64,
                    None => info().map(|x| x.stars as f64).unwrap_or(f64::NAN),
                },
                SortMode::Length => info().map(|x| x.length_ms as f64).unwrap_or(f64::NAN),
                SortMode::Bpm => info().map(|x| x.bpm as f64).unwrap_or(f64::NAN),
                SortMode::Hp => library[i].meta.hp,
                SortMode::Od => library[i].meta.od,
                // Ordered as osu! has them (graveyard with pending); not known: last.
                SortMode::Status => match self.status_section(&library[i]) {
                    crate::search::UNKNOWN => f64::NAN,
                    c => c as f64,
                },
                SortMode::Played => self
                    .played
                    .get(&library[i].path)
                    .map(|&t| t as f64)
                    .unwrap_or(f64::NAN),
                SortMode::Added => library[i].modified as f64,
                SortMode::Artist | SortMode::Title | SortMode::Folder => 0.0,
            }
        };
        let mut folder_ids: std::collections::HashMap<(&std::path::Path, i64), usize> =
            std::collections::HashMap::new();
        let split = self.split_difficulties();
        let folder = |i: usize| {
            if split {
                // Each difficulty on its own: ordered by its own value.
                return (library[i].path.as_path(), 0);
            }
            let stars = if self.sort == SortMode::Difficulty {
                star_heading(value(i))
            } else {
                0
            };
            (library[i].set_key(), stars)
        };
        let mut set_value: Vec<f64> = Vec::new();
        let mut keyed = Vec::with_capacity(self.visible.len());
        for &i in &self.visible {
            let n = folder_ids.len();
            let f = *folder_ids.entry(folder(i)).or_insert(n);
            if f == set_value.len() {
                set_value.push(f64::NAN);
            }
            keyed.push((f, i));
            if self.sort != SortMode::Title {
                let v = value(i);
                if v.is_finite() && (set_value[f].is_nan() || set_value[f] < v) {
                    set_value[f] = v;
                }
            }
        }
        // Highest first: the newest, and the best status.
        let newest_first = matches!(
            self.sort,
            SortMode::Played | SortMode::Added | SortMode::Status
        );
        let title = |i: usize| {
            (
                library[i].text().title.as_str(),
                library[i].text().artist.as_str(),
            )
        };
        if self.sort == SortMode::Folder {
            // By folder, TJA packs first, then by title within each.
            let mut keys: Vec<Option<FolderKey>> = vec![None; folder_ids.len()];
            for &(f, i) in &keyed {
                keys[f].get_or_insert_with(|| (source_folder(&library[i]), title(i)));
            }
            keyed.sort_by(|a, b| keys[a.0].cmp(&keys[b.0]).then(a.0.cmp(&b.0)));
        } else if self.sort == SortMode::Title {
            let titles: Vec<(&str, &str)> = {
                let mut t = vec![("", ""); folder_ids.len()];
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
        self.row_value = keyed
            .iter()
            .map(|&(f, _)| {
                if matches!(self.sort, SortMode::Title | SortMode::Folder) {
                    f64::NAN
                } else {
                    set_value[f]
                }
            })
            .collect();
        self.visible = keyed.into_iter().map(|(_, i)| i).collect();
    }

    /// Ctrl+S: next (or previous) sort mode.
    pub fn cycle_sort(&mut self, dir: i32, library: &[Entry]) {
        self.set_sort(self.sort.step(dir), library);
    }

    /// Lists the songs by `sort`.
    pub fn set_sort(&mut self, sort: SortMode, library: &[Entry]) {
        self.sort = sort;
        self.picker = None;
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
        let each = self.split_difficulties();
        for song in self.cuts(library) {
            self.group_of
                .extend(std::iter::repeat_n(self.groups.len(), song.len()));
            self.groups.push(song);
        }
        self.sections.clear();
        self.section_of.clear();
        if self.open_for != self.sort {
            self.open_for = self.sort;
            self.flipped.clear();
            self.flipped_before_search = self
                .flipped_before_search
                .take()
                .map(|_| Default::default());
            self.cursor_label = None;
            self.auto_open = None;
        }
        self.song_count = if each {
            self.visible
                .iter()
                .map(|&i| library[i].set_key())
                .collect::<std::collections::HashSet<_>>()
                .len()
        } else {
            self.groups.len()
        };
        if !self.headings {
            // One section, never drawn, holding every song.
            if !self.groups.is_empty() {
                self.sections.push(Section {
                    label: String::new(),
                    colour: None,
                    groups: 0..self.groups.len(),
                });
            }
            self.section_of = vec![0; self.groups.len()];
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for g in 0..self.groups.len() {
            let (label, colour) = self.section_heading(library, self.groups[g].start, now);
            match self.sections.last_mut() {
                Some(last) if last.label == label => last.groups.end = g + 1,
                _ => self.sections.push(Section {
                    label,
                    colour,
                    groups: g..g + 1,
                }),
            }
            self.section_of.push(self.sections.len() - 1);
        }
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
            SortMode::Folder => return (source_folder(e).1, None),
            // From the song's value the list was ordered by (its best difficulty's status), not from
            // the first difficulty's: a song of mixed statuses would split its heading in two.
            SortMode::Status => {
                let code = if v.is_nan() {
                    crate::search::UNKNOWN
                } else {
                    v as u8
                };
                let (name, colour) = status_heading(code);
                return (name.to_string(), Some(colour));
            }
            _ if v.is_nan() && self.sort == SortMode::Played => "Never played".to_string(),
            _ if v.is_nan() => unknown(),
            SortMode::Difficulty => match star_heading(v) {
                10 => "10+ stars".to_string(),
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
                v => format!(
                    "{} - {} BPM",
                    (v / 20.0).floor() as i64 * 20,
                    (v / 20.0).floor() as i64 * 20 + 19
                ),
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

    /// Whether headings start open: a letter is only a divider in a long list, and a search
    /// lists what it found. The other sorts start closed, like osu!'s groups.
    fn starts_open(&self) -> bool {
        !self.filter.is_empty() || matches!(self.sort, SortMode::Artist | SortMode::Title)
    }

    fn label_open(&self, label: &str) -> bool {
        !self.headings || self.starts_open() != self.flipped.contains(label)
    }

    /// Each difficulty is a row of its own: sorting by difficulty without headings, like osu!.
    fn split_difficulties(&self) -> bool {
        !self.headings && self.sort == SortMode::Difficulty
    }

    /// Lists songs under headings or not.
    pub fn set_headings(&mut self, on: bool, library: &[Entry]) {
        if self.headings != on {
            self.headings = on;
            self.refilter(library);
        }
    }

    fn set_open(&mut self, label: &str, open: bool) {
        if open == self.starts_open() {
            self.flipped.remove(label);
        } else {
            self.flipped.insert(label.to_string());
        }
    }

    /// Whether section `s` lists its songs.
    fn section_open(&self, s: usize) -> bool {
        self.label_open(&self.sections[s].label)
    }

    /// The heading the selection is under is open, unless the player closed it with the selection
    /// there. If it had to open for the selection, it closes again when the selection moves out.
    fn open_cursor_section(&mut self) {
        let Some(s) = self.cursor_section() else {
            return;
        };
        let label = self.sections[s].label.clone();
        if self.cursor_label.as_ref() != Some(&label) {
            self.cursor_label = Some(label.clone());
            if let Some(left) = self.auto_open.take().filter(|left| *left != label) {
                self.set_open(&left, false);
            }
        }
        if self.closed_at != Some(self.cursor) && !self.label_open(&label) {
            self.set_open(&label, true);
            self.auto_open = Some(label);
        }
    }

    /// A click on heading `s`: opens it or closes it, the selection staying where it is and the
    /// list staying where it's looking.
    pub fn click_section(&mut self, s: usize, library: &[Entry]) {
        let Some(label) = self.sections.get(s).map(|sec| sec.label.clone()) else {
            return;
        };
        self.free(library, WHEEL_TAU);
        let open = !self.label_open(&label);
        self.set_open(&label, open);
        if self.auto_open.as_ref() == Some(&label) {
            self.auto_open = None;
        }
        if self.cursor_section() == Some(s) {
            self.closed_at = (!open).then_some(self.cursor);
        }
    }

    /// The selected song's set (`Entry::set_key`).
    fn selected_folder<'a>(&self, library: &'a [Entry]) -> Option<&'a std::path::Path> {
        self.selected().map(|i| library[i].set_key())
    }

    /// The list as moved by hand, from where it is now if it was following the selection.
    fn free(&mut self, library: &[Entry], tau: f32) -> &mut Glide {
        let song = self.selected_folder(library).map(|p| p.to_path_buf());
        let now = self.anim.scroll;
        let free = self.free.get_or_insert_with(|| FreeScroll {
            glide: Glide::at(now),
            tau,
            song,
        });
        free.tau = tau;
        &mut free.glide
    }

    /// The top of the list's frame: the source tabs, where there are, then the list.
    fn frame_top(&self) -> f32 {
        if self.legacy {
            LIST_TOP_LEGACY
        } else {
            LIST_TOP
        }
    }

    pub(crate) fn list_top(&self) -> f32 {
        if self.source_tabs().is_empty() {
            self.frame_top()
        } else {
            self.frame_top() + lens::SOURCE_TAB_H + 3.0
        }
    }

    fn list_bottom(&self) -> f32 {
        if self.legacy {
            LIST_BOTTOM_LEGACY
        } else if self.banner {
            LIST_BOTTOM_BANNER
        } else {
            LIST_BOTTOM
        }
    }

    /// The top and bottom of the song list's frame, for the panel beside it to line up with.
    pub fn frame_span(&self) -> [f32; 2] {
        [self.frame_top() - 4.0, self.list_bottom() + 4.0]
    }

    /// The wheel: moves the list `notches` on, the selection staying where it is.
    pub fn scroll_by(&mut self, notches: i32, library: &[Entry]) {
        let max = self.scroll_max;
        let glide = self.free(library, WHEEL_TAU);
        let goal = glide.goal + notches as f32 * WHEEL_STEP;
        // Past an end: a little push the list springs back from, so the wheel is never dead.
        let past = goal - goal.clamp(0.0, max);
        if past != 0.0 && crate::render::animations() && (glide.now - glide.goal).abs() < 1.0 {
            glide.now += past.signum() * WHEEL_END_BUMP;
        }
        glide.goal = goal.clamp(0.0, max);
    }

    /// Back to following the selection, gliding there from wherever the list is.
    pub fn follow_selection(&mut self) {
        if self.free.take().is_some() {
            let a = &mut self.anim;
            (a.scroll_from, a.outline_from, a.open_from, a.closing_from) =
                (a.scroll, a.outline, a.open_now, a.closing_now);
            a.last_start = a.start;
            a.start = Some(crate::render::frame_now());
            a.ms = 0.0;
        }
    }

    /// The songs (`groups` indices) whose headings are open, in order: what scrolling goes through.
    fn listed_groups(&self) -> Vec<usize> {
        let open: Vec<bool> = (0..self.sections.len())
            .map(|s| self.section_open(s))
            .collect();
        (0..self.groups.len())
            .filter(|&g| open[self.section_of[g]])
            .collect()
    }

    /// The song (range of `visible`) containing the cursor.
    fn cursor_group(&self) -> Option<std::ops::Range<usize>> {
        self.group_of
            .get(self.cursor)
            .map(|&g| self.groups[g].clone())
    }

    pub fn select_path(&mut self, library: &[Entry], path: &std::path::Path) {
        if let Some(pos) = self.visible.iter().position(|&i| library[i].path == path) {
            self.cursor = pos;
        }
    }

    /// The filters a pin holds under.
    fn view(&self) -> View {
        (
            self.filter.clone(),
            self.collection,
            self.show_converts,
            self.tabs.source,
            self.tabs.filters,
        )
    }

    /// Lists `maps`, just added, even if the filters would hide them, which stay as they are
    /// until the player changes them (then the maps go by them too).
    pub fn pin(&mut self, library: &[Entry], maps: Vec<std::path::PathBuf>) {
        if self.pinned_view != self.view() {
            self.pinned.clear();
            self.pinned_view = self.view();
        }
        self.pinned.extend(maps);
        self.refilter(library);
    }

    /// Pins `maps` and goes to the first of them that shows (one may have gone).
    pub fn reveal(&mut self, library: &[Entry], maps: Vec<std::path::PathBuf>) {
        self.pin(library, maps.clone());
        if let Some(path) = maps
            .iter()
            .find(|p| self.visible.iter().any(|&i| library[i].path == **p))
        {
            self.select_path(library, path);
            self.open_cursor_section();
        }
    }

    pub fn key(&mut self, key: KeyCode, library: &[Entry]) -> SelectAction {
        let n = self.visible.len();
        // Keys act on the selection, so that's where the list goes.
        if !matches!(key, KeyCode::Tab | KeyCode::F1) {
            self.follow_selection();
        }
        match key {
            KeyCode::ArrowDown => self.step(1, library),
            KeyCode::ArrowUp => self.step(-1, library),
            KeyCode::PageDown => self.jump_songs(5, library),
            KeyCode::PageUp => self.jump_songs(-5, library),
            KeyCode::Home | KeyCode::End => {
                let listed = self.listed_groups();
                if let Some(&g) = if key == KeyCode::Home {
                    listed.first()
                } else {
                    listed.last()
                } {
                    self.enter_song(self.groups[g].start, library);
                }
            }
            // Each difficulty on its own row: to the next or previous row, whatever its set.
            KeyCode::ArrowRight | KeyCode::ArrowLeft if self.split_difficulties() => {
                self.move_songs(
                    if key == KeyCode::ArrowRight { 1 } else { -1 },
                    library,
                    true,
                );
            }
            // Jump between beatmap sets.
            KeyCode::ArrowRight | KeyCode::ArrowLeft => {
                if let Some(&cur) = self.visible.get(self.cursor) {
                    let dir = library[cur].set_key();
                    let fwd = key == KeyCode::ArrowRight;
                    let mut i = self.cursor;
                    loop {
                        if fwd {
                            if i + 1 >= n {
                                break;
                            }
                            i += 1
                        } else {
                            if i == 0 {
                                break;
                            }
                            i -= 1
                        }
                        if library[self.visible[i]].set_key() != dir {
                            break;
                        }
                    }
                    if !fwd {
                        // Land on the first difficulty of the previous set.
                        let d = library[self.visible[i]].set_key();
                        while i > 0 && library[self.visible[i - 1]].set_key() == d {
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

    /// Up/Down (`n` steps, negative = up): through the open song's difficulties, and past its
    /// first or last one into the previous or next listed song. Past the last listed song it goes
    /// on into the next heading, which opens for it.
    pub fn step(&mut self, n: i32, library: &[Entry]) {
        for _ in 0..n.unsigned_abs() {
            let Some(g) = self.cursor_group() else { return };
            let next = self.cursor as i64 + n.signum() as i64;
            if (g.start as i64..g.end as i64).contains(&next) {
                self.cursor = next as usize;
            } else {
                self.move_songs(n.signum(), library, true);
            }
        }
    }

    /// Moves `n` songs on (negative = back) among those listed (the open headings'), onto each
    /// song's remembered or hardest difficulty.
    pub fn jump_songs(&mut self, n: i32, library: &[Entry]) {
        self.move_songs(n, library, false);
    }

    fn move_songs(&mut self, n: i32, library: &[Entry], open_next: bool) {
        let Some(&g) = self.group_of.get(self.cursor) else {
            return;
        };
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
            let edge = if n > 0 {
                listed.last().copied().unwrap_or(g) as i64 + 1
            } else {
                listed.first().copied().unwrap_or(g) as i64 - 1
            };
            if !(0..self.groups.len() as i64).contains(&edge) {
                return;
            }
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
        self.follow_selection();
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
        self.group_of
            .get(row)
            .is_some_and(|&g| self.groups[g].start == row)
    }

    /// Whether `row` is in the song that is open (selected).
    pub fn in_open_song(&self, row: usize) -> bool {
        self.group_of.get(row).is_some() && self.group_of.get(row) == self.group_of.get(self.cursor)
    }

    /// The songs as ranges of `rows`.
    /// Tests: the library index of list row `row`.
    #[cfg(test)]
    pub fn visible_index(&self, row: usize) -> usize {
        self.visible[row]
    }

    pub fn song_groups(&self) -> &[std::ops::Range<usize>] {
        &self.groups
    }

    /// What's listed, in order: the difficulties' library indices.
    pub fn rows(&self) -> &[usize] {
        &self.visible
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

    /// Width of the preview panel beside a song list of `share` (past the margins and the gap).
    pub fn panel_width(screen_w: f32, share: f32) -> f32 {
        screen_w - 24.0 - Self::list_width(screen_w, share) - 12.0 - 24.0
    }

    /// The shares of a `screen_w` wide screen the song list can take: within `LIST_SHARE`, and
    /// leaving the list and the panel (past the margins and the gap between them) their least.
    /// None when the screen is too narrow for both.
    pub fn share_range(screen_w: f32) -> Option<(f32, f32)> {
        let w = screen_w.max(1.0);
        let (min, max) = (
            LIST_SHARE.0.max(LIST_MIN / w),
            LIST_SHARE.1.min((w - 60.0 - PANEL_MIN) / w),
        );
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
        let Some(&g) = self.group_of.get(row) else {
            return;
        };
        let g = self.groups[g].clone();
        let song = library[self.visible[g.start]].set_key();
        let remembered = self
            .remembered
            .get(song)
            .copied()
            .and_then(|i| self.visible[g.clone()].iter().position(|&v| v == i));
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
        let d = self.drag.get_or_insert_with(|| ListDrag {
            scroll: a.scroll,
            now: a.scroll,
            samples: Default::default(),
        });
        d.now = d.scroll - dy;
        let now = std::time::Instant::now();
        d.samples.push_back((now, d.now));
        while d
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t).as_millis() > 100)
        {
            d.samples.pop_front();
        }
    }

    /// Drops a drag without selecting anything (left song select mid-drag).
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    /// Lets go of the list: it carries on at the speed it was thrown and stays there. What's
    /// selected doesn't change: a song is picked by clicking it.
    pub fn drag_end(&mut self, library: &[Entry]) {
        let Some(d) = self.drag.take() else { return };
        let velocity = match (d.samples.front(), d.samples.back()) {
            (Some(&(t0, s0)), Some(&(t1, s1))) if t1 > t0 => {
                (s1 - s0) / t1.duration_since(t0).as_secs_f32() / 1000.0
            }
            _ => 0.0,
        };
        // Released while still: no throw.
        let still = d
            .samples
            .back()
            .is_none_or(|(t, _)| t.elapsed().as_millis() > 60);
        let throw = if still {
            0.0
        } else {
            (velocity * 250.0).clamp(-3000.0, 3000.0)
        };
        let shown = self.anim.scroll;
        let glide = self.free(library, THROW_TAU);
        // From where the list is shown (held a little past an end, it springs back).
        *glide = Glide {
            now: shown,
            goal: d.now + throw,
        };
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

    /// The menu's items for its row: delete the difficulty (not on a song's own row), delete the
    /// set, and get the song's newest version from osu! (a set that has an ID).
    pub fn is_liked(&self, e: &Entry) -> bool {
        self.favourites.contains(&crate::playlist::favourite_key(e))
    }

    /// Likes the song of `e`, or not any more; true if it's liked now.
    pub fn toggle_like(&mut self, e: &Entry) -> bool {
        let key = crate::playlist::favourite_key(e);
        let liked = !self.favourites.remove(&key);
        if liked {
            self.favourites.insert(key);
        }
        crate::playlist::save_favourites(&self.favourites);
        liked
    }

    /// The map (library index) on list row `row`.
    pub fn row_map(&self, row: usize) -> Option<usize> {
        self.visible.get(row).copied()
    }

    /// Whether the map in folder `dir` has its own skin turned off.
    pub fn map_skin_off(&self, dir: &std::path::Path) -> bool {
        !self.map_skins_off.is_empty()
            && folder_name(dir).is_some_and(|n| self.map_skins_off.contains(&n))
    }

    /// Turns the map's own skin off or back on, for all its difficulties. Returns whether it's off.
    pub fn toggle_map_skin(&mut self, entry: &Entry) -> bool {
        let Some(name) = entry.path.parent().and_then(folder_name) else {
            return false;
        };
        let off = self.map_skins_off.insert(name.clone());
        if !off {
            self.map_skins_off.remove(&name);
        }
        crate::config::save_map_skins_off(&self.map_skins_off);
        off
    }

    /// The right-click menu's items, in drawing order. Items next to each other with the same
    /// label and part share a row, each a button of its own for its scope (`MenuScope`).
    pub fn menu_items(&self, library: &[Entry]) -> Vec<MenuItem> {
        let Some(m) = &self.menu else {
            return Vec::new();
        };
        let Some(&i) = self.visible.get(m.row) else {
            return Vec::new();
        };
        let e = &library[i];
        let mut items = Vec::new();
        let mut add = |label: String, scope, on, part, action| {
            items.push(MenuItem {
                label,
                scope,
                on,
                part,
                action,
            })
        };
        let (own, online, delete) = (MenuPart::Own, MenuPart::Online, MenuPart::Delete);
        let (one, set) = (Some(MenuScope::Difficulty), Some(MenuScope::Set));
        let liked = self.is_liked(e);
        add(
            if liked { "Unlike" } else { "Like" }.into(),
            None,
            false,
            own,
            MenuAction::Like,
        );
        if !m.header {
            add("Edit".into(), None, false, own, MenuAction::Edit);
            add(
                "New difficulty from this".into(),
                None,
                false,
                own,
                MenuAction::NewDifficulty,
            );
            add(
                "Collect".into(),
                one,
                false,
                own,
                MenuAction::Collect(false),
            );
        }
        add("Collect".into(), set, false, own, MenuAction::Collect(true));
        if e.set_id().is_some() || self.custom_id(e).is_some() {
            let label = match self.update_of(e) {
                Some(SetUpdate::Updating) => "Updating...",
                Some(SetUpdate::Available) => "Update",
                _ => "Download again",
            };
            add(label.into(), None, false, own, MenuAction::UpdateSong);
        }
        if let Some(version) = m.previous {
            let label = match version {
                0 => tr!(
                    "Previous version",
                    "前のバージョンに戻す",
                    "Вернуть прошлую версию"
                )
                .to_string(),
                v => tr!(
                    "Previous version (v{v})",
                    "前のバージョン (v{v}) に戻す",
                    "Прошлая версия (v{v})"; [] [v]
                ),
            };
            add(label, None, false, own, MenuAction::PreviousVersion);
        }
        if m.skin {
            let off = e.path.parent().is_some_and(|d| self.map_skin_off(d));
            let label = if off {
                tr!(
                    "Use this map's skin",
                    "この譜面のスキンを使う",
                    "Включить скин карты"
                )
            } else {
                tr!(
                    "Turn off this map's skin",
                    "この譜面のスキンをオフ",
                    "Выключить скин карты"
                )
            };
            add(label.to_string(), None, false, own, MenuAction::MapSkin);
        }
        if let Some(id) = e.set_id().filter(|_| self.nominator) {
            // Loved whole, there's no one difficulty of it to take back.
            let versions = self.server_loved.get(&id);
            let whole = versions.is_some_and(|v| v.is_empty());
            if !m.header && !whole {
                let on = versions.is_some_and(|v| covers(v, &e.meta.version));
                add(
                    "Love".into(),
                    one,
                    on,
                    online,
                    MenuAction::Love(id, !on, true),
                );
            }
            add(
                "Love".into(),
                set,
                whole,
                online,
                MenuAction::Love(id, !whole, false),
            );
        }
        if self.admin {
            if let Some(id) = self.custom_id(e) {
                // A custom set is marked whole.
                let on = self.server_epic.custom.contains(&id);
                add("EPIC".into(), set, on, online, MenuAction::Epic(!on, false));
            } else if let Some(id) = e.set_id() {
                let versions = self.server_epic.sets.get(&id);
                let whole = versions.is_some_and(|v| v.is_empty());
                if !m.header && !whole {
                    let on = versions.is_some_and(|v| covers(v, &e.meta.version));
                    add("EPIC".into(), one, on, online, MenuAction::Epic(!on, true));
                }
                add(
                    "EPIC".into(),
                    set,
                    whole,
                    online,
                    MenuAction::Epic(!whole, false),
                );
            }
        }
        let label = tr!("Delete", "削除", "Удалить").to_string();
        // A TJA chart's courses are one file: they go together.
        if !m.header && !e.is_tja() {
            add(
                label.clone(),
                one,
                false,
                delete,
                MenuAction::DeleteDifficulty,
            );
        }
        add(label, set, false, delete, MenuAction::DeleteSet);
        items
    }

    /// What an `Epic` menu action marks: the menu's map's custom set, or its osu! set or just
    /// its difficulty (`one`).
    pub fn epic_target(&self, library: &[Entry], one: bool) -> Option<crate::online::EpicMap> {
        let e = &library[self.menu_map()?];
        if let Some(id) = self.custom_id(e) {
            return Some(crate::online::EpicMap::Custom(id));
        }
        Some(crate::online::EpicMap::Set {
            set: e.set_id()?,
            version: one.then(|| e.meta.version.clone()),
        })
    }

    /// Takes deleted maps (`paths`) out of the list, keeping the selection where it was unless
    /// it went: then on another difficulty of its song, else the song that takes its place.
    pub fn forget_maps(&mut self, paths: &[std::path::PathBuf], library: &[Entry]) {
        let place = self.current_song();
        let current = self.visible.get(self.cursor).copied();
        self.gone.extend(paths.iter().cloned());
        for p in paths {
            self.pinned.remove(p);
            self.edited.remove(p);
            self.played.remove(p);
        }
        let stay = current
            .filter(|&c| !self.gone.contains(&library[c].path))
            .or_else(|| {
                let g = self.current_song()?;
                self.groups[g]
                    .clone()
                    .map(|r| self.visible[r])
                    .rev()
                    .find(|&v| !self.gone.contains(&library[v].path))
            });
        self.refilter(library);
        match stay.and_then(|v| self.visible.iter().position(|&x| x == v)) {
            Some(pos) => self.cursor = pos,
            None => {
                if let Some(g) = place {
                    self.go_to_song(g as i64, library);
                }
            }
        }
        self.open_cursor_section();
    }

    /// The scrollbar and osu!'s right-click scrolling: the pointer's height (`y`, virtual units)
    /// is where the list is, all of it, closed headings too. The thumb stays under the pointer.
    pub fn scroll_to(&mut self, y: f32, library: &[Entry]) {
        let top = self.list_top();
        let (track, thumb) = (self.list_bottom() - top, self.thumb);
        let frac = ((y - top - thumb / 2.0) / (track - thumb).max(1.0)).clamp(0.0, 1.0);
        let max = self.scroll_max;
        self.free(library, SCRUB_TAU).goal = frac * max;
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
        let Some(g) = self.cursor_group() else { return };
        let Some(keys) = self.visible[g.clone()]
            .iter()
            .map(|&i| stars(i))
            .collect::<Option<Vec<f32>>>()
        else {
            return;
        };
        if keys.windows(2).all(|w| w[0] <= w[1]) {
            return;
        }
        let current = self.visible[self.cursor];
        // Still on the row it entered on: it means "the hardest", wherever that ends up.
        let to_last = self.hardest_pending == Some(g.start) && self.cursor == g.end - 1;
        let mut set: Vec<(f32, usize)> = keys
            .into_iter()
            .zip(self.visible[g.clone()].iter().copied())
            .collect();
        set.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (slot, (_, i)) in g.clone().zip(set) {
            self.visible[slot] = i;
        }
        // Sorting only reorders within the song, so the groups stay valid. A hardest difficulty
        // with audio of its own stays unselected: the music has already started on this one.
        if to_last
            && library[self.visible[g.end - 1]].meta.audio_file == library[current].meta.audio_file
        {
            self.cursor = g.end - 1;
        } else if let Some(pos) = self.visible[g.clone()].iter().position(|&v| v == current) {
            self.cursor = g.start + pos;
        }
        self.hardest_pending = None;
    }

    pub fn selected_set(&self) -> Vec<usize> {
        self.cursor_group()
            .map(|g| self.visible[g].to_vec())
            .unwrap_or_default()
    }

    /// Remembers the selected difficulty for its song (call once a frame).
    pub fn remember_selection(&mut self, library: &[Entry]) {
        if let Some(&i) = self.visible.get(self.cursor) {
            let song = library[i].set_key();
            if self.remembered.get(song) != Some(&i) {
                self.remembered.insert(song.to_path_buf(), i);
            }
        }
    }

    /// `stars` looks up a difficulty's star rating (library index) if it has been calculated.
    /// `grade`: the best letter of the plays on a map.
    pub fn draw(
        &mut self,
        c: &mut Canvas,
        library: &[Entry],
        dt_ms: f64,
        status: &str,
        stars: &dyn Fn(usize) -> Option<f32>,
        grade: &dyn Fn(&Entry) -> Option<&'static str>,
    ) {
        self.open_cursor_section();
        let w = c.width;
        let mode = if self.show_converts {
            "taiko + converts"
        } else {
            "taiko only"
        };
        let groups = &self.groups;
        let summary = format!(
            "{} songs, {} maps  |  {mode}{}",
            self.song_count,
            self.visible.len(),
            self.rate_label
        );
        let lw = Self::list_width(w, self.list_share);
        let legacy = self.legacy;
        // The row with the search: under the tabs, or at the top beside the way back.
        let row = if legacy { 54.0 } else { 20.0 };
        let mut left = 24.0;
        if legacy {
            crate::settings::tab_bar(c, 0, &summary);
        } else {
            // The search row and the list are one window: the panel runs from above the row
            // down the list, a line between them. Not while a mod's customization covers the
            // top of the screen: it has the row's place.
            // The search row on a raised strip, no edge round it; the list's cards below it are
            // surfaces of their own, straight on the map's picture.
            if !self.covered {
                let (top, bottom) = (row - 6.0, self.frame_top() - 4.0);
                let rect = [20.0, top, lw + 8.0, bottom - top];
                crate::style::surface(c, rect, 8.0, crate::style::Surface::Raised);
            }
            // The way back: an arrow and where it goes, as bare as the menu's icons.
            let back = if self.to_menu {
                tr!("Menu", "メニュー", "Меню")
            } else {
                tr!("Back", "戻る", "Назад")
            };
            left += 8.0;
            if self.covered {
                left += crate::style::icon_button_width(back, None, 8.0) + 6.0;
            } else {
                let at = [left, row + 12.0];
                let bw = crate::style::icon_button(
                    c,
                    at,
                    &crate::style::BACK,
                    back,
                    None,
                    8.0,
                    false,
                    Hit::ToMenu,
                );
                left += bw + 6.0;
            }
            // On a small chip beside the search row, over the panel's side, where there is one.
            let (x, right) = (24.0 + lw + 12.0, w - 24.0);
            if self.show_info && right - x > 60.0 {
                let text = Canvas::fit(6.0, right - x - 16.0, &summary);
                let chip = [x, row + 4.0, Canvas::text_width(6.0, &text) + 16.0, 16.0];
                crate::style::surface(
                    c,
                    chip,
                    crate::style::radius::CARD,
                    crate::style::Surface::Raised,
                );
                c.text(
                    x + 8.0,
                    row + 12.0 - Canvas::text_height(6.0) / 2.0,
                    6.0,
                    rgba(crate::style::ink::CAPTION, 1.0),
                    &text,
                );
            }
        }
        // Mods, sort and collection buttons share the search row, right to left. On a narrow
        // screen the widest of them switches to a shorter label until the search box gets at
        // least SEARCH_MIN; past that the widest give up room (their labels get cut).
        const SEARCH_MIN: f32 = 140.0;
        let mods = if self.mods_label.is_empty() {
            vec!["Mods".to_string()]
        } else {
            vec![
                format!("Mods: {}", self.mods_label),
                format!("Mods ({})", self.mods_label.len() / 2),
            ]
        };
        let sort = self.sort.name();
        let then = match self.tabs.order {
            Order::Sort => String::new(),
            o => format!(", {}", o.label()),
        };
        let filters = self.tabs.active(self.shown_source()).tokens().len();
        let more = if filters > 0 {
            format!(" +{filters}")
        } else {
            String::new()
        };
        // Outside the Legacy layout they're bare icons: the word on a small caption above, the
        // value as the label.
        let sort_labels = if legacy {
            vec![format!("Sort: {sort}{then}{more}"), format!("{sort}{more}")]
        } else {
            vec![format!("{sort}{then}{more}"), format!("{sort}{more}")]
        };
        let mut buttons = vec![(
            sort_labels,
            Hit::SortList,
            filters > 0 || self.lens_ui.is_some(),
            (&crate::style::SORT, tr!("SORT", "ソート", "СОРТ")),
        )];
        if legacy {
            buttons.insert(
                0,
                (
                    mods.clone(),
                    Hit::ModsButton,
                    !self.mods_label.is_empty(),
                    (&crate::style::SPARK, ""),
                ),
            );
        }
        if !self.collections.is_empty() {
            let name = self
                .collection
                .and_then(|c| self.collections.get(c))
                .map_or("all", |c| c.name.as_str());
            let labels = if legacy {
                vec![format!("Collection: {name}"), name.to_string()]
            } else {
                vec![name.to_string()]
            };
            buttons.push((
                labels,
                Hit::CollectionPick,
                self.collection.is_some(),
                (
                    &crate::style::FOLDER,
                    tr!("COLLECTION", "コレクション", "КОЛЛЕКЦИЯ"),
                ),
            ));
        }
        let width = |label: &str, caption: &str| {
            if legacy {
                Canvas::text_width(7.0, label) + 20.0
            } else {
                crate::style::icon_button_width(label, Some(caption), 7.0)
            }
        };
        let budget = lw - (left - 24.0) - SEARCH_MIN - 6.0 * buttons.len() as f32;
        let mut pick = vec![0; buttons.len()];
        loop {
            let widths: Vec<f32> = buttons
                .iter()
                .zip(&pick)
                .map(|((labels, .., (_, cap)), &k)| width(&labels[k], cap))
                .collect();
            if widths.iter().sum::<f32>() <= budget {
                break;
            }
            let shorter = (0..buttons.len())
                .filter(|&i| pick[i] + 1 < buttons[i].0.len())
                .max_by(|&a, &b| widths[a].total_cmp(&widths[b]));
            let Some(i) = shorter else { break };
            pick[i] += 1;
        }
        let labels: Vec<&str> = buttons
            .iter()
            .zip(&pick)
            .map(|((labels, ..), &k)| labels[k].as_str())
            .collect();
        let natural: Vec<f32> = labels
            .iter()
            .zip(&buttons)
            .map(|(l, (.., (_, cap)))| width(l, cap))
            .collect();
        let cap = shared_cap(&natural, budget);
        let mut bx = 24.0 + lw - if legacy { 0.0 } else { 6.0 };
        for ((label, (_, hit, accent, (icon, caption))), nw) in
            labels.iter().zip(&buttons).zip(natural)
        {
            let bw = nw.min(cap);
            bx -= bw;
            if legacy {
                c.button(bx, row + 2.0, bw, 20.0, label, 7.0, *hit, *accent);
            } else {
                let room = bw - crate::style::icon_button_width("", None, 7.0);
                let label = Canvas::fit(7.0, room, label);
                let caption = Canvas::fit(5.0, room, caption);
                let at = [bx + 6.0, row + 12.0];
                crate::style::icon_button(c, at, icon, &label, Some(&caption), 7.0, *accent, *hit);
            }
            bx -= 6.0;
        }
        let search_w = bx - left;
        let edge = if self.searching {
            rgba(crate::render::accent(), 1.0)
        } else {
            rgba(0x3a3a4e, 1.0)
        };
        if legacy {
            c.round_rect(
                [left, row],
                [search_w, 24.0],
                6.0,
                rgba(0x23232f, 0.9),
                edge,
                1.0,
            );
        } else {
            // A lifted field, no edge; while typing a line of the accent under the text (the
            // focus).
            let field = [left, row, search_w, 24.0];
            crate::style::surface(c, field, 6.0, crate::style::Surface::Lifted);
            if self.searching {
                c.rect(
                    left + 6.0,
                    row + 21.0,
                    (search_w - 12.0).max(0.0),
                    1.5,
                    edge,
                );
            }
        }
        // Outside the Legacy layout the box starts with a magnifying glass.
        let glass = if legacy { 0.0 } else { 14.0 };
        if !legacy {
            crate::style::icon(
                c,
                &crate::style::SEARCH,
                [left + 12.0, row + 12.0],
                10.0,
                crate::style::ink::CAPTION,
                1.0,
            );
        }
        let left = left + glass;
        let search_w = search_w - glass;
        // The filters on, as tokens at the start of the box: the text goes after them.
        let token_room = search_w * 0.6;
        let mut tokens_w = 0.0;
        for (_, text, _) in self.tabs.active(self.shown_source()).tokens() {
            let tw = Canvas::text_width(7.0, &text) + 18.0;
            if tokens_w + tw > token_room {
                break;
            }
            tokens_w += tw + 4.0;
        }
        let text_x = left + 10.0 + tokens_w;
        let room = search_w - 20.0 - tokens_w;
        let caret = "";
        // Outside the Legacy layout the hint is smaller, and the key that opens the search sits
        // on a key cap at the box's right end.
        let hint_size = if legacy { 10.0 } else { 8.0 };
        let mut room = room;
        if !legacy && self.filter.is_empty() && !self.searching {
            let key = "ctrl+f";
            let kw = Canvas::text_width(5.0, key) + 8.0;
            if room - kw - 8.0 >= Canvas::text_width(hint_size, "search...") {
                let kx = left + search_w - 6.0 - kw;
                let cap = [kx, row + 6.0, kw, 12.0];
                crate::style::surface(c, cap, 3.0, crate::style::Surface::Raised);
                c.text(
                    kx + 4.0,
                    row + 12.0 - Canvas::text_height(5.0) / 2.0,
                    5.0,
                    rgba(crate::style::ink::CAPTION, 1.0),
                    key,
                );
                room -= kw + 8.0;
            }
        }
        let (search, search_col) = if self.filter.is_empty() {
            let (long, short) = if self.searching {
                ("type to search...", "search...")
            } else if legacy {
                ("click to search (ctrl+f)...", "search...")
            } else {
                ("click to search...", "search...")
            };
            let hint = if Canvas::text_width(hint_size, long) <= room {
                long
            } else {
                short
            };
            let quiet = if legacy {
                0x6f6f86
            } else {
                crate::style::ink::CAPTION
            };
            (
                Canvas::fit(hint_size, room, &format!("{hint}{caret}")),
                quiet,
            )
        } else {
            let full = format!("search: {}{caret}", self.filter);
            let text = if Canvas::text_width(10.0, &full) <= room {
                full
            } else {
                Canvas::fit_end(10.0, room, &format!("{}{caret}", self.filter))
            };
            (text, 0xffffff)
        };
        if self.searching {
            c.input(
                crate::text_input::Field::SongSearch,
                [text_x, row + 7.0, room, 12.0],
                10.0,
                &self.filter,
                "type to search...",
                rgba(0xffffff, 1.0),
                false,
                true,
            );
        } else if self.filter.is_empty() {
            let y = row + 12.0 - Canvas::text_height(hint_size) / 2.0;
            c.text(text_x, y, hint_size, rgba(search_col, 1.0), &search);
        } else {
            c.text(text_x, row + 7.0, 10.0, rgba(search_col, 1.0), &search);
        }
        c.hit(left - glass, row, search_w + glass, 24.0, Hit::TextField);
        self.draw_tokens(c, left + 6.0, row + 4.0, token_room);

        if let Some(n) = self.scanning {
            c.text_centered(
                24.0 + lw / 2.0,
                200.0,
                12.0,
                rgba(0xc8c8d8, 1.0),
                "Scanning your Songs folder...",
            );
            c.text_centered(
                24.0 + lw / 2.0,
                222.0,
                10.0,
                rgba(0x8a8aa0, 1.0),
                &format!("{n} files read"),
            );
        } else if library.is_empty() && !cfg!(target_os = "android") {
            // Most likely osu! lives somewhere it wasn't looked for.
            let cx = 24.0 + lw / 2.0;
            c.text_centered(cx, 190.0, 12.0, rgba(0xc8c8d8, 1.0), "No maps found.");
            let hint = "Show taiko.rs where your osu! Songs folder is, or get maps in DOWNLOAD.";
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(
                cx,
                212.0,
                size,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(size, lw, hint),
            );
            let label = if self.picking_folder {
                "Choosing in the folder dialog..."
            } else {
                "Choose Songs folder"
            };
            let bw = (Canvas::text_width(9.0, label) + 32.0).min(lw);
            c.button(
                cx - bw / 2.0,
                234.0,
                bw,
                26.0,
                label,
                9.0,
                Hit::SongsFolder,
                true,
            );
            let later = "or press Enter. Change it later in Settings > Library.";
            c.text_centered(
                cx,
                268.0,
                7.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(7.0, lw, later),
            );
        } else if self.visible.is_empty() {
            // Maps there are, but what's picked leaves none: say how to get them back.
            let narrowed = !self.filter.is_empty()
                || !self.tabs.active(self.shown_source()).is_empty()
                || self.collection.is_some();
            c.text_centered(
                24.0 + lw / 2.0,
                200.0,
                12.0,
                rgba(0xc8c8d8, 1.0),
                if narrowed {
                    "No maps match."
                } else {
                    "No maps found."
                },
            );
            let hint = if narrowed && self.collection.is_some() {
                "Ctrl+Backspace clears the search and filters, Ctrl+L: collections"
            } else if narrowed {
                "Ctrl+Backspace clears the search and filters"
            } else if cfg!(target_os = "android") {
                "Open .osz files with taiko.rs, or get maps in DOWNLOAD"
            } else {
                "Run: taiko-rs <Songs folder | map.osz | map.osu>"
            };
            let size = Canvas::fit_size(10.0, 7.0, lw, hint);
            c.text_centered(
                24.0 + lw / 2.0,
                222.0,
                size,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(size, lw, hint),
            );
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
        let (top, bottom) = (self.list_top(), self.list_bottom());
        let view = bottom - top;
        // Where everything ends up: the selected song open, the others closed.
        let open_start = self.group_of.get(self.cursor).map(|&g| groups[g].start);
        let section_open: Vec<bool> = (0..self.sections.len())
            .map(|s| self.section_open(s))
            .collect();
        let cursor_section = self.cursor_section();
        let cursor_hidden = cursor_section.is_some_and(|si| !section_open[si]);
        let cursor_group = self.group_of.get(self.cursor).copied();
        let final_layout = ListLayout::new(
            &self.sections,
            &section_open,
            self.headings,
            (SONG_H, SECTION_H),
            cursor_group
                .filter(|_| !cursor_hidden)
                .map(|g| (g, groups[g].len() as f32 * DIFF_H))
                .into_iter()
                .collect(),
        );
        let final_y = final_layout.total;
        let final_cursor = match (cursor_section, cursor_group) {
            // A closed heading holding the selection is the one to keep in view.
            (Some(si), _) if !section_open[si] => final_layout.tops[si],
            (Some(si), Some(g)) => {
                final_layout.song_y(si, g)
                    + SONG_H
                    + (self.cursor - groups[g].start) as f32 * DIFF_H
            }
            _ => 0.0,
        };

        let max_scroll = (final_y - view).max(0.0);
        self.scroll_max = max_scroll;
        let final_scroll = (final_cursor - view * 0.4).clamp(0.0, max_scroll);
        let final_outline = top + final_cursor - final_scroll;
        // Another song selected: the list goes to it.
        let from_free = self
            .free
            .as_ref()
            .is_some_and(|f| f.song.as_deref() != self.selected_folder(library));
        if from_free {
            self.free = None;
        }

        // A new selection starts a transition from the current state.
        let a = &mut self.anim;
        if a.cursor != Some(self.cursor) || a.open != open_start {
            if a.cursor.is_none() {
                // First frame: no motion.
                *a = ListAnim {
                    open: open_start,
                    open_from: 1.0,
                    scroll_from: final_scroll,
                    outline_from: final_outline,
                    ..Default::default()
                };
            } else {
                if open_start != a.open {
                    a.opened_at = open_start.map(|s| (s, crate::render::frame_now()));
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
                a.last_start = a.start;
                a.start = Some(crate::render::frame_now());
                // A release's longer glide applies to the transition it starts, then resets.
                if a.cursor != Some(usize::MAX) {
                    a.ms = 0.0;
                }
                // Picked in a list moved by hand: from where it was, at an easy pace.
                if from_free {
                    let far = (final_scroll - a.scroll_from).abs();
                    a.ms = (FROM_FREE_MS.0 + far * 1.2).min(FROM_FREE_MS.1);
                }
            }
            a.cursor = Some(self.cursor);
        }
        let ms = if a.ms > 0.0 { a.ms } else { LIST_ANIM_MS };
        let p = a.start.map(|t| progress(t, ms)).unwrap_or(1.0);
        let open_f = a.open_from + (1.0 - a.open_from) * p;
        let closing_f = a.closing_from * (1.0 - p);
        // Landed: the selection got to the target (not while the pointer flies the list), and
        // the target flashes. Not on the first frame, nor without animations.
        let flying = self.drag.is_some() || self.free.is_some();
        if p >= 1.0 && !flying && a.landed != a.cursor {
            if a.landed.is_some() && crate::render::animations() {
                a.flash = Some(crate::render::frame_now());
            }
            a.landed = a.cursor;
        }
        let mut scroll = a.scroll_from + (final_scroll - a.scroll_from) * p;
        let mut outline = a.outline_from + (final_outline - a.outline_from) * p;
        if let Some(f) = &mut self.free {
            f.glide.settle(max_scroll, c.dt_ms, f.tau);
            scroll = f.glide.now;
            outline = top + final_cursor - scroll;
        }
        if let Some(d) = &self.drag {
            // Held: exactly where the pointer has it (a little past the ends at most).
            let max = max_scroll;
            scroll = if d.now < 0.0 {
                d.now * 0.3
            } else if d.now > max {
                max + (d.now - max) * 0.3
            } else {
                d.now
            };
            outline = top + final_cursor - scroll;
        }
        (a.scroll, a.outline, a.open_now, a.closing_now) = (scroll, outline, open_f, closing_f);
        let (open_g, closing_g) = (a.open, a.closing);
        self.scroll = scroll;

        // (row, y, height, alpha): rows of opening / closing songs are squeezed and faded.
        // Only the rows in view are made: a big library has thousands of songs to scroll past.
        let group_at = |start: Option<usize>| {
            start
                .and_then(|s| self.group_of.get(s).copied())
                .filter(|&g| Some(groups[g].start) == start)
        };
        let anim_extra = |g: Option<usize>, f: f32| {
            g.map(|g| {
                (
                    g,
                    if f > 0.001 {
                        groups[g].len() as f32 * DIFF_H * f
                    } else {
                        0.0
                    },
                )
            })
        };
        let open_gi = group_at(open_g);
        let closing_gi = group_at(closing_g).filter(|g| Some(*g) != open_gi);
        let mut extras: Vec<(usize, f32)> = [
            anim_extra(open_gi, open_f),
            anim_extra(closing_gi, closing_f),
        ]
        .into_iter()
        .flatten()
        .collect();
        extras.sort_by_key(|(g, _)| *g);
        let layout = ListLayout::new(
            &self.sections,
            &section_open,
            self.headings,
            (SONG_H, SECTION_H),
            extras,
        );
        let mut rows: Vec<(Row, f32, f32, f32)> = Vec::new();
        let (from, to) = (scroll - 2.0, scroll + view + 2.0);
        for (si, sec) in self.sections.iter().enumerate() {
            let end = layout.tops.get(si + 1).copied().unwrap_or(layout.total);
            if end < from || layout.tops[si] > to {
                continue;
            }
            if self.headings {
                rows.push((Row::Section(si), layout.tops[si], SECTION_H, 1.0));
            }
            if !section_open[si] {
                continue;
            }
            let shown = layout.visible_songs(si, from, to);
            let mut y = if shown.is_empty() {
                0.0
            } else {
                layout.song_y(si, shown.start)
            };
            for gi in shown {
                let g = &groups[gi];
                rows.push((Row::Song(g.clone()), y, SONG_H, 1.0));
                y += SONG_H;
                let f = if Some(gi) == open_gi {
                    open_f
                } else if Some(gi) == closing_gi {
                    closing_f
                } else {
                    0.0
                };
                if f > 0.001 {
                    for i in g.clone() {
                        rows.push((Row::Diff(i), y, DIFF_H, f * f));
                        y += DIFF_H * f;
                    }
                }
            }
            let _ = sec;
        }
        // The cards' pictures near the list and ahead of it, the way it's going (at least a
        // screen, more the faster it goes), are read before they come into view: dragging or
        // wheeling through the list draws them from memory.
        let moved = self.seen_scroll.map_or(0.0, |s| scroll - s);
        self.seen_scroll = Some(scroll);
        if c.dt_ms > 0.0 {
            let speed = moved / c.dt_ms;
            crate::render::approach(&mut self.scroll_speed, speed, c.dt_ms, CARD_AHEAD_TAU);
        }
        if !legacy {
            let lead = (self.scroll_speed.abs() * CARD_AHEAD_MS).clamp(view, view * 8.0);
            let (back, ahead) = if self.scroll_speed < 0.0 {
                (lead, view * 0.5)
            } else {
                (view * 0.5, lead)
            };
            let (from, to) = (scroll - back, scroll + view + ahead);
            // Nearest the screen first, those ahead before those behind.
            let mut near: Vec<(f32, usize)> = Vec::new();
            for (si, &open) in section_open.iter().enumerate() {
                if !open {
                    continue;
                }
                for gi in layout.visible_songs(si, from, to) {
                    let y = layout.song_y(si, gi);
                    let off = if y < scroll {
                        (scroll - y) * if self.scroll_speed < 0.0 { 1.0 } else { 2.0 }
                    } else if y > scroll + view {
                        (y - scroll - view) * if self.scroll_speed < 0.0 { 2.0 } else { 1.0 }
                    } else {
                        0.0
                    };
                    near.push((off, gi));
                }
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, gi) in near {
                let e = &library[self.visible[groups[gi].start]];
                if let Some(bg) = crate::preview::background_path(e) {
                    c.want_card(&bg);
                }
            }
        }
        // The main menu's look: one dark panel the list runs down, rows as lines on it.
        let frame_top = self.frame_top();
        // Outside the Legacy layout it was drawn with the search row, unless that's covered.
        let frame = [20.0, frame_top - 4.0, lw + 8.0, bottom - frame_top + 8.0];
        if legacy {
            crate::style::panel(c, frame[0], frame[1], frame[2], frame[3]);
        }
        let tabs = self.source_tabs();
        if !tabs.is_empty() {
            self.draw_source_tabs(c, 24.0, frame_top, lw, &tabs);
        }
        // A new source: its rows come in from the right, one after another, through a flicker.
        let switch_ms = self
            .source_switched
            .map(|t| t.elapsed().as_secs_f32() * 1000.0)
            .filter(|_| crate::render::animations());
        if switch_ms.is_some_and(|ms| ms > 1000.0) {
            self.source_switched = None;
        }
        let mut drawn = 0usize;
        // The drawn row the selection is in, for its outline to come in with it.
        let mut cursor_row = 0usize;
        let arriving = self.arriving.filter(|_| !legacy);
        let list_start = c.shapes.len();
        let hits_start = c.hits.len();
        let fit = |s: String, size: f32, room: f32| {
            Canvas::fit(size, room.max(Canvas::char_width(size) * 4.0), &s)
        };
        // Under the pointer: Legacy's grey, the main menu's look the lifted surface.
        let hover_fill = |alpha: f32| {
            if legacy {
                rgba(0x24242f, 0.6 * alpha)
            } else {
                let [r, g, b, a] = crate::style::surface_fill(crate::style::Surface::Lifted);
                [r, g, b, a * alpha]
            }
        };
        // Flying through the list (the pointer or the scrollbar holding it, or a new selection
        // before the last one has got there: Random spammed, an arrow held down), the banner's
        // whole picture of where it's going isn't read ahead: the sets passing by would each start
        // a slow reading. The cards' strips are small and read all the same.
        let settle = LIST_ANIM_MS + PICTURE_SETTLE_MS;
        let ms = |t: std::time::Instant, since: std::time::Instant| {
            t.saturating_duration_since(since).as_secs_f32() * 1000.0
        };
        let now = crate::render::frame_now();
        let rushed = match (self.anim.start, self.anim.last_start) {
            (Some(t), Some(last)) => ms(t, last) < settle && ms(now, t) < settle,
            _ => false,
        };
        self.hold_pictures =
            self.scrubbing.is_some() || self.drag.is_some() || self.free.is_some() || rushed;
        // On its way to a new selection: the cards where it's going are read meanwhile, so
        // they come in with their pictures, and the banner's picture of the set it lands on
        // (not while selections come faster than the list gets to them).
        if !legacy
            && self.anim.start.is_some_and(|t| ms(now, t) < LIST_ANIM_MS)
            && let Some(g0) = cursor_group
        {
            let above = final_cursor - final_scroll;
            let up = (above / SONG_H).ceil() as usize + 1;
            let down = ((view - above) / SONG_H).ceil() as usize + 1;
            for g in g0.saturating_sub(up)..(g0 + down + 1).min(groups.len()) {
                let e = &library[self.visible[groups[g].start]];
                if let Some(bg) = crate::preview::background_path(e) {
                    c.want_card(&bg);
                    if g == g0 && !self.hold_pictures {
                        c.want_picture_path(&bg);
                    }
                }
            }
        }
        // How far a row's middle is from the target's, beyond the straight part (list heights).
        let target_y = outline + DIFF_H / 2.0;
        let away = |cy: f32| ((cy - target_y).abs() / view.max(1.0) - ARC_STRAIGHT).max(0.0);
        let mut lights = std::mem::take(&mut self.anim.card_light);
        let mut covers = std::mem::take(&mut self.anim.covers);
        for (row, ry, rh, alpha) in &rows {
            let y = top + ry - self.scroll;
            if y + rh < top || y > bottom || *alpha < 0.02 {
                continue;
            }
            let row_start = c.shapes.len();
            drawn += 1;
            match row {
                Row::Section(si) => {
                    let sec = &self.sections[*si];
                    // Without a colour of its own a heading is neutral outside Legacy: the
                    // accent is the selection's.
                    let col = sec.colour.unwrap_or_else(|| {
                        if legacy {
                            accent()
                        } else {
                            crate::style::role::NEUTRAL
                        }
                    });
                    let open = section_open[*si];
                    let hover = c.hovered(24.0, y, lw, rh - 2.0);
                    let holds_cursor = cursor_section == Some(*si) && !open;
                    // A heading on the panel: its name on a pill of its colour, a line of it on.
                    if hover {
                        c.round_rect(
                            [24.0, y],
                            [lw, rh - 3.0],
                            6.0,
                            hover_fill(1.0),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    if holds_cursor {
                        crate::style::brackets(c, [21.0, y - 3.0, lw + 6.0, rh + 3.0], 1.0);
                    }
                    c.hit(24.0, y, lw, *rh, Hit::SectionHeader(*si));
                    let n = sec.groups.len();
                    c.text(
                        32.0,
                        y + 6.0,
                        7.0,
                        rgba(col, 1.0),
                        if open { "v" } else { ">" },
                    );
                    let label = fit(sec.label.clone(), 7.0, lw - 140.0);
                    let pw = Canvas::text_width(7.0, &label) + 14.0;
                    crate::style::pill(c, 44.0, y + 1.5, pw, rh - 6.0, col);
                    c.text(51.0, y + 6.0, 7.0, rgba(0xffffff, 1.0), &label);
                    let count = format!("{n} song{}", if n == 1 { "" } else { "s" });
                    let after = c.text(
                        44.0 + pw + 8.0,
                        y + 6.0,
                        7.0,
                        rgba(0x8a8aa0, 1.0),
                        &fit(count, 7.0, lw - pw - 40.0),
                    );
                    let line_x = 44.0 + pw + 8.0 + after + 8.0;
                    if line_x < 16.0 + lw {
                        c.rect(
                            line_x,
                            y + rh / 2.0 - 1.5,
                            16.0 + lw - line_x,
                            1.0,
                            rgba(col, 0.3),
                        );
                    }
                }
                Row::Song(g) => {
                    let e = &library[self.visible[g.start]];
                    let open = g.contains(&self.cursor);
                    if open {
                        cursor_row = drawn - 1;
                    }
                    // Under the pointer by where the row is, not where its card has moved to.
                    let hover = !open && c.hovered(24.0, y, lw, rh - 4.0);
                    // The main menu's look: a card with the map's picture. The open set's card
                    // slides out to the list's left edge, the others sit in by CARD_INSET, and
                    // one under the pointer comes out a little and brightens.
                    let light = if legacy {
                        0.0
                    } else {
                        let l = lights.entry(g.start).or_insert(0.0);
                        let tau = crate::style::motion::MICRO_MS / 3.0;
                        crate::render::approach(l, if hover { 1.0 } else { 0.0 }, c.dt_ms, tau);
                        *l
                    };
                    let (x0, rw) = if legacy {
                        (24.0, lw)
                    } else {
                        let out = if Some(g.start) == open_g {
                            open_f
                        } else if Some(g.start) == closing_g {
                            closing_f
                        } else {
                            0.0
                        };
                        // Farther from the target, bent away along the lane's arc (not the open
                        // set's card as it slides out).
                        let e = away(y + rh / 2.0);
                        let bend = (ARC_K * e * e).min(ARC_MAX) * (1.0 - out);
                        let inset = (CARD_INSET * (1.0 - out) + bend - CARD_HOVER * light).max(0.0);
                        (24.0 + inset, lw - inset)
                    };
                    if legacy {
                        if open {
                            c.round_rect(
                                [24.0, y],
                                [lw, rh - 4.0],
                                7.0,
                                rgba(crate::style::selector_colour(), 0.08),
                                rgba(0, 0.0),
                                0.0,
                            );
                        } else if hover {
                            c.round_rect(
                                [24.0, y],
                                [lw, rh - 4.0],
                                7.0,
                                rgba(0x262634, 0.6),
                                rgba(0, 0.0),
                                0.0,
                            );
                        }
                        // Legacy's line under each row; the main menu's look parts rows by room.
                        c.rect(32.0, y + rh - 3.0, lw - 16.0, 1.0, rgba(0x23232f, 1.0));
                    } else {
                        let card = [x0, y, rw, rh - 4.0];
                        self.draw_set_card(c, e, card, open, light, &mut covers);
                    }
                    c.hit(x0, y, rw, *rh, Hit::SongHeader(g.start));
                    // Legacy: a note for the song, as on the main menu: a don for taiko maps, a
                    // kat for converts; the open song's bigger and lit. A card has its picture.
                    if legacy {
                        let any_taiko = g.clone().any(|i| library[self.visible[i]].meta.mode == 1);
                        let colours = crate::game::note_colours();
                        let tag = if any_taiko {
                            colours.don()
                        } else {
                            colours.kat()
                        };
                        let (nr, ny) = (if open { 6.5 } else { 5.0 }, y + (rh - 4.0) / 2.0);
                        crate::style::note(c, 33.0, ny, nr, tag, open, 0.7);
                    }
                    let tx = if legacy { 44.0 } else { x0 + 12.0 };
                    // A song with a map marked EPIC: a prism stripe down its left edge, its title
                    // in the shimmer, and a spark round the open one.
                    let epic = g.clone().any(|i| self.is_epic(&library[self.visible[i]]));
                    // A TJA chart's courses, as the arcade's ladder at the right: each in its
                    // colour with its level. The title stops short of it.
                    let one = self.split_difficulties();
                    let ladder: Vec<(u32, String, f32)> = if e.is_tja() && !one {
                        g.clone()
                            .map(|i| {
                                let course = &library[self.visible[i]];
                                let kind = crate::tja_maps::split(&course.path).map(|(_, k)| k);
                                let text = course
                                    .meta
                                    .level
                                    .map_or("?".to_string(), crate::style::level_text);
                                let bw = (Canvas::text_width(6.0, &text) + 6.0).max(12.0);
                                (kind.map_or(0x8a8aa0, course_colour), text, bw)
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let ladder_w: f32 = ladder.iter().map(|l| l.2 + 2.0).sum();
                    let title_room = (rw - 110.0).min(rw - 12.0 - ladder_w - 28.0);
                    let title = fit(e.meta.title.clone(), 10.0, title_room);
                    if epic {
                        crate::epic::stripe(c, x0, y + 3.0, rh - 10.0, 1.0);
                        crate::epic::text(c, tx, y + 6.0, 10.0, &title, 1.0);
                        if open {
                            crate::epic::spark_round(c, [x0, y, rw, rh - 4.0], 1.0);
                        }
                    } else {
                        c.text(tx, y + 6.0, 10.0, rgba(0xffffff, 1.0), &title);
                    }
                    // A row per difficulty: which one it is, rather than who mapped the set.
                    let sub = if one {
                        format!("{}  |  {}", e.meta.artist, e.meta.version)
                    } else {
                        format!("{}  |  mapped by {}", e.meta.artist, e.meta.creator)
                    };
                    // Centred at y + 23.5, in what's left between the title (taller on a text
                    // grid) and the bottom of the box, so the two never touch.
                    let title_bottom = 11.0 + Canvas::text_height(10.0) / 2.0;
                    let room = 2.0 * (23.5 - title_bottom - 1.5).min(rh - 4.0 - 1.5 - 23.5);
                    let sub_size = Canvas::fit_height(7.0, room);
                    c.text(
                        tx,
                        y + 20.0 + (7.0 - sub_size) / 2.0,
                        sub_size,
                        rgba(0x9a9ab0, 1.0),
                        &fit(sub, sub_size, rw - 110.0),
                    );
                    // A newer version on osu!: click (or Ctrl+Shift+U) to get it. Under the
                    // difficulty count (moved up for it), inside the row (not on one cut off at
                    // the list's end).
                    let update = self.update_of(e).filter(|u| {
                        matches!(u, SetUpdate::Available | SetUpdate::Updating)
                            && y + rh - 4.0 <= bottom
                    });
                    let n = g.len();
                    let diffs_y = if update.is_some() { y + 6.0 } else { y + 11.0 };
                    if one {
                        // What the list is ordered by (no mods, 1.0x); the open row has the mods'.
                        let sorted_by = self
                            .row_value
                            .get(g.start)
                            .copied()
                            .filter(|v| v.is_finite())
                            .map(|v| v as f32);
                        // A TJA course is ordered by its level, and shows it as its charter wrote it.
                        let level = e.meta.level.filter(|_| self.sort == SortMode::Difficulty);
                        // In its rating's colour only when it's the selected one (Legacy: all).
                        crate::style::map_pill_lit(
                            c,
                            x0 + rw - 12.0,
                            diffs_y - 1.0,
                            8.0,
                            level,
                            sorted_by,
                            legacy || open,
                        );
                    } else if !ladder.is_empty() {
                        let mut right = x0 + rw - 12.0;
                        for (colour, text, bw) in ladder.iter().rev() {
                            let (colour, bw) = (*colour, *bw);
                            right -= bw;
                            c.rect(right, diffs_y - 2.0, bw, 11.0, rgba(colour, 1.0));
                            c.rect(right, diffs_y + 8.0, bw, 1.0, rgba(0x000000, 0.35));
                            c.text_centered(
                                right + bw / 2.0,
                                diffs_y,
                                6.0,
                                rgba(0x15151d, 1.0),
                                text,
                            );
                            right -= 2.0;
                        }
                    } else if legacy {
                        // Legacy only: on the main menu look's cards, over the set's picture,
                        // it can't be read on many pictures (the open set lists them anyway).
                        c.text_right(
                            x0 + rw - 12.0,
                            diffs_y,
                            8.0,
                            rgba(0x8a8aa0, 1.0),
                            &format!("{n} diff{}", if n == 1 { "" } else { "s" }),
                        );
                    }
                    let (by, bh) = (y + 19.0, 9.0);
                    let ty = by + (bh - Canvas::text_height(6.0)) / 2.0;
                    match update {
                        Some(SetUpdate::Available) => {
                            let label = "update";
                            let bw = Canvas::text_width(6.0, label) + 10.0;
                            let bx = x0 + rw - 12.0 - bw;
                            let hover = c.hovered(bx, by, bw, bh);
                            c.round_rect(
                                [bx, by],
                                [bw, bh],
                                3.0,
                                rgba(if hover { 0x5a4a1a } else { 0x3a3218 }, 1.0),
                                rgba(0xFFD34D, 1.0),
                                1.0,
                            );
                            c.text_centered(bx + bw / 2.0, ty, 6.0, rgba(0xFFD34D, 1.0), label);
                            c.hit(bx, by, bw, bh, Hit::UpdateSet(self.visible[g.start]));
                        }
                        Some(SetUpdate::Updating) => c.text_right(
                            x0 + rw - 12.0,
                            ty,
                            6.0,
                            rgba(0x4fb4ff, 1.0),
                            "updating...",
                        ),
                        _ => {}
                    }
                }
                Row::Diff(i) => {
                    let a = *alpha;
                    let e = &library[self.visible[*i]];
                    let selected = *i == self.cursor;
                    if selected {
                        cursor_row = drawn - 1;
                    }
                    // The main menu's look: a slimmer card, a bar of its rating's colour at its
                    // left (full on the selected one).
                    if !legacy {
                        let [r, g, b, fa] =
                            crate::style::surface_fill(crate::style::Surface::Raised);
                        let card = [44.0, y, lw - 20.0, rh - 3.0];
                        let clear = rgba(0, 0.0);
                        c.round_rect(
                            [card[0], card[1]],
                            [card[2], card[3]],
                            4.0,
                            [r, g, b, fa * a],
                            clear,
                            0.0,
                        );
                        let bar = crate::style::role::rating(e.meta.level, stars(self.visible[*i]));
                        let lit = if selected { 1.0 } else { DIFF_BAR_DIM };
                        c.rect(44.0, y + 3.0, 3.0, rh - 9.0, rgba(bar, lit * a));
                    }
                    if selected {
                        c.round_rect(
                            [44.0, y],
                            [lw - 20.0, rh - 3.0],
                            5.0,
                            rgba(
                                crate::style::selector_colour(),
                                (0.12 + 0.05 * crate::style::pulse()) * a,
                            ),
                            rgba(0, 0.0),
                            0.0,
                        );
                    } else if c.hovered(44.0, y, lw - 20.0, rh - 3.0) {
                        c.round_rect(
                            [44.0, y],
                            [lw - 20.0, rh - 3.0],
                            5.0,
                            hover_fill(a),
                            rgba(0, 0.0),
                            0.0,
                        );
                    }
                    if legacy {
                        c.rect(52.0, y + rh - 2.0, lw - 36.0, 1.0, rgba(0x1d1d27, a));
                    }
                    c.hit(44.0, y, lw - 20.0, *rh, Hit::SongRow(*i));
                    let col = match (selected, legacy) {
                        (true, _) => crate::style::ink::PRIMARY,
                        (false, true) => 0xc8c8d8,
                        (false, false) => crate::style::ink::SECONDARY,
                    };
                    let convert = if e.meta.mode == 0 { "  (convert)" } else { "" };
                    let pill = if a > 0.6 {
                        crate::style::map_pill_lit(
                            c,
                            24.0 + lw - 10.0,
                            y + 6.0,
                            7.0,
                            e.meta.level,
                            stars(self.visible[*i]),
                            legacy || selected,
                        )
                    } else {
                        40.0
                    };
                    if let Some(g) = grade(e) {
                        let right = 24.0 + lw - 18.0 - pill;
                        // SS is split into don and kat colours, like on the website.
                        if g == "SS" {
                            c.text_right(right - 1.0, y + 5.0, 9.0, rgba(0xEB452C, a), g);
                            c.text_right(right + 1.0, y + 5.0, 9.0, rgba(0x4fb4ff, a), g);
                        }
                        c.text_right(right, y + 5.0, 9.0, rgba(grade_colour(g), a), g);
                    }
                    c.text(
                        56.0,
                        y + 6.0,
                        8.0,
                        rgba(col, a),
                        &fit(format!("{}{convert}", e.meta.version), 8.0, lw - 170.0),
                    );
                }
            }
            if !legacy {
                // The main menu's look: the sets not open dim (less under the pointer), and
                // the last cards fade out at the list's bottom instead of being cut.
                if let Row::Song(g) = row
                    && !g.contains(&self.cursor)
                {
                    let l = lights.get(&g.start).copied().unwrap_or(0.0);
                    c.fade_since(row_start, CARD_DIM + (1.0 - CARD_DIM) * l, 1.0);
                }
                // Farther from the target, a little fainter.
                let e = (away(y + rh / 2.0) / ARC_FADE_REACH).min(1.0);
                if e > 0.0 {
                    c.fade_since(row_start, 1.0 - ARC_FADE * e * e, 1.0);
                }
                let over = (y + rh - (bottom - BOTTOM_FADE)).max(0.0);
                let f = (1.0 - over / (rh + BOTTOM_FADE)).clamp(0.0, 1.0);
                if f < 1.0 {
                    c.fade_since(row_start, f * f * (3.0 - 2.0 * f), 1.0);
                }
            }
            // A set just opened: its difficulties drop in one after another, out from the
            // selected one.
            if !legacy
                && let Row::Diff(i) = row
                && let Some((start, at)) = self.anim.opened_at
                && let Some(&g) = self.group_of.get(*i)
                && groups[g].start == start
            {
                use crate::style::motion;
                let set = &groups[g];
                let from = if set.contains(&self.cursor) {
                    self.cursor
                } else {
                    start
                };
                let far = (from - set.start).max(set.end - 1 - from).max(1) as f32;
                let step = DIFF_CASCADE_MS.min(DIFF_CASCADE_SPAN_MS / far);
                let k = i.abs_diff(from) as f32;
                let since = at + std::time::Duration::from_secs_f32(step * k / 1000.0);
                let p = motion::entrance(since, motion::CONTENT_MS, motion::out_quint);
                if p < 1.0 {
                    c.transform_since(row_start, [0.0, 0.0], 1.0, 0.0, -DIFF_DROP * (1.0 - p));
                    c.fade_since(row_start, p, 1.0);
                }
            }
            if let Some(ms) = switch_ms {
                let t = ((ms - drawn as f32 * 18.0) / lens::SWITCH_MS).clamp(0.0, 1.0);
                if t < 1.0 {
                    let ease = 1.0 - (1.0 - t).powi(3);
                    c.transform_since(row_start, [0.0, 0.0], 1.0, (1.0 - ease) * 28.0, 0.0);
                    c.fade_since(row_start, ease, 1.0);
                }
            }
            // From the main menu: each card drops out of the search row after the one above.
            if let Some(f) = arriving {
                let (dy, a) = crate::menu_shift::card_drop(f, drawn - 1);
                if dy != 0.0 || a < 1.0 {
                    c.transform_since(row_start, [0.0, 0.0], 1.0, 0.0, dy);
                    c.fade_since(row_start, a, 1.0);
                }
            }
        }
        // Lights gone out are forgotten.
        lights.retain(|_, l| *l > 0.001);
        self.anim.card_light = lights;
        self.anim.covers = covers;
        if let Some(ms) = switch_ms.filter(|&ms| ms < 160.0) {
            crate::style::glitch_bars(c, [24.0, top, lw, bottom - top], ms / 160.0);
        }

        // The selection outline moves with the same transition.
        // Not with its heading closed: the heading shows where the selection is instead.
        let oy = outline;
        let outline_start = c.shapes.len();
        if !self.visible.is_empty() && !cursor_hidden && oy > top - DIFF_H && oy < bottom {
            let target = [41.0, oy - 3.0, lw - 14.0, DIFF_H + 3.0];
            crate::style::brackets(c, target, 1.0);
            // A song landing in it: a flash, the brackets opening out and fading.
            if let Some(t) = self
                .anim
                .flash
                .filter(|_| !legacy)
                .map(|t| progress(t, LAND_FLASH_MS))
            {
                if t < 1.0 {
                    let [x, y, w, h] = target;
                    let flash = rgba(0xffffff, 0.22 * (1.0 - t));
                    c.round_rect(
                        [x + 3.0, y + 3.0],
                        [w - 6.0, h - 3.0],
                        5.0,
                        flash,
                        flash,
                        0.0,
                    );
                    let g = LAND_FLASH_GROW * t;
                    let out = [x - g, y - g, w + 2.0 * g, h + 2.0 * g];
                    crate::style::brackets(c, out, 1.0 - t);
                } else {
                    self.anim.flash = None;
                }
            }
        }
        // From the main menu the outline comes in with the selection's card.
        if let Some(f) = arriving {
            let (dy, a) = crate::menu_shift::card_drop(f, cursor_row);
            c.transform_since(outline_start, [0.0, 0.0], 1.0, 0.0, dy);
            c.fade_since(outline_start, a, 1.0);
        }
        // Rows scrolling past the edges slide under them instead of over the search row.
        c.clip_since(list_start, 20.0, top - 2.0, lw + 8.0, bottom - top + 4.0);
        self.list_shapes = [list_start, c.shapes.len()];
        for h in &mut c.hits[hits_start..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(top - 2.0), (*y + *hh).min(bottom + 2.0));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }
        // The scroll bar (after the list's clip, outside it), in the margin left of the list: where
        // the list is, clicked or dragged to go somewhere (`scroll_to`, like right-dragging).
        if groups.len() > 1 {
            let (track_x, track_w) = (8.0, 8.0);
            let track = bottom - top;
            let hover = c.hovered(track_x - 4.0, top, track_w + 12.0, track);
            if legacy {
                c.round_rect(
                    [track_x, top],
                    [track_w, track],
                    4.0,
                    rgba(0x15151d, if hover { 0.9 } else { 0.6 }),
                    rgba(0x2a2a38, 1.0),
                    1.0,
                );
            } else {
                let level = if hover {
                    crate::style::Surface::Lifted
                } else {
                    crate::style::Surface::Raised
                };
                crate::style::surface(c, [track_x, top, track_w, track], 4.0, level);
            }
            let total = final_y.max(view);
            // Where each heading starts, as a mark on the track.
            if self.headings && self.sections.len() > 1 {
                for (si, sec) in self.sections.iter().enumerate() {
                    let ty = top + final_layout.tops[si] / total * track;
                    let col = sec.colour.unwrap_or(0x6f6f86);
                    c.rect(track_x - 3.0, ty, 3.0, 1.0, rgba(col, 0.9));
                }
            }
            let thumb = (track * view / total).clamp(18.0, track);
            self.thumb = thumb;
            let pos = (scroll / (total - view).max(1.0)).clamp(0.0, 1.0);
            c.round_rect(
                [track_x + 1.0, top + pos * (track - thumb)],
                [track_w - 2.0, thumb],
                3.0,
                // The accent is the selection's outside Legacy: under the pointer, white.
                rgba(
                    match (hover, legacy) {
                        (true, true) => accent(),
                        (true, false) => crate::style::ink::PRIMARY,
                        (false, _) => 0x8a8aa8,
                    },
                    0.95,
                ),
                rgba(0, 0.0),
                0.0,
            );
            c.hit(
                track_x - 4.0,
                top,
                track_w + 12.0,
                track,
                Hit::SongScrollbar,
            );
        }

        // Flying through the list: which heading it's passing.
        if let Some(py) = self.scrubbing {
            let line = scroll + view / 2.0;
            let heading = self
                .headings
                .then(|| layout.tops.partition_point(|t| *t <= line).checked_sub(1))
                .flatten();
            if let Some(si) = heading {
                let label = Canvas::fit(10.0, lw - 40.0, &self.sections[si].label);
                let bw = Canvas::text_width(10.0, &label) + 24.0;
                let by = (py - 13.0).clamp(top, bottom - 26.0);
                if legacy {
                    c.round_rect(
                        [30.0, by],
                        [bw, 26.0],
                        6.0,
                        rgba(0x15151d, 0.96),
                        rgba(accent(), 1.0),
                        1.5,
                    );
                } else {
                    let fill = rgba(crate::style::SURFACE_2, 0.96);
                    c.round_rect([30.0, by], [bw, 26.0], 6.0, fill, fill, 0.0);
                }
                c.text(42.0, by + 8.0, 10.0, rgba(0xffffff, 1.0), &label);
            }
        }
        // Scrolled away from the selection: the way back.
        let cursor_y = top + final_cursor - scroll;
        if self.free.is_some()
            && self.scrubbing.is_none()
            && !self.visible.is_empty()
            && (cursor_y < top - DIFF_H || cursor_y > bottom)
        {
            let label = tr!(
                "back to the selected song",
                "選択中の曲へ戻る",
                "к выбранной песне"
            );
            let bw = Canvas::text_width(7.0, label) + 20.0;
            c.button(
                24.0 + (lw - bw) / 2.0,
                bottom - 24.0,
                bw,
                20.0,
                label,
                7.0,
                Hit::ListBack,
                true,
            );
        }

        let hints = tr!(
            "Enter play  Up/Down difficulty  Left/Right song  Wheel or right drag scroll  F1 mods  F2 random  [ ] rate  Ctrl+S sort and filter  Ctrl+L collection  Tab settings  Esc quit",
            "Enter プレイ  上下 難易度  左右 曲  ホイールか右ドラッグ スクロール  F1 Mod  F2 ランダム  [ ] レート  Ctrl+S 並び替えと絞り込み  Ctrl+L コレクション  Tab 設定  Esc 終了",
            "Enter играть  Вверх/вниз сложность  Влево/вправо песня  Колесо или правый драг: прокрутка  F1 моды  F2 случайная  [ ] рейт  Ctrl+S сортировка и фильтры  Ctrl+L коллекция  Tab настройки  Esc выход"
        );
        if legacy {
            c.text(
                24.0,
                452.0,
                7.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit_hints(7.0, w - 48.0, hints),
            );
        }
        // The status line gives way (smaller) when the hints above come out taller.
        let status_size = Canvas::fit_height(
            7.0,
            2.0 * (469.5 - (455.5 + Canvas::text_height(7.0) / 2.0) - 3.0),
        );
        if legacy || self.show_info {
            c.text(
                24.0,
                466.0 + (7.0 - status_size) / 2.0,
                status_size,
                rgba(0x55556a, 1.0),
                &Canvas::fit(status_size, w - 48.0, status),
            );
        }
    }

    /// A set's card in the main menu's look over `[x, y, w, h]`: a surface (lifted when open or
    /// under the pointer), the map's picture over it behind a shade from dark under the title
    /// to lighter on the right (fading in as it arrives; the plain surface until then), lit by
    /// `light` under the pointer, and the selection's outline round the open one.
    fn draw_set_card(
        &self,
        c: &mut Canvas,
        e: &Entry,
        [x, y, w, h]: [f32; 4],
        open: bool,
        light: f32,
        covers: &mut std::collections::HashMap<String, std::time::Instant>,
    ) {
        use crate::style::{self, Surface};
        let level = if open || light > 0.5 {
            Surface::Lifted
        } else {
            Surface::Raised
        };
        style::surface(c, [x, y, w, h], CARD_RADIUS, level);
        if let Some(bg) = crate::preview::background_path(e) {
            let key = crate::render::path_picture_key(&bg);
            let shown = c.card_slots.contains_key(&key);
            if !shown {
                // Let go of to make room for others: it fades in again when it's back.
                covers.remove(&key);
            }
            let since = covers.get(&key).copied();
            let a = match since {
                Some(t) => crate::render::progress(t, COVER_FADE_MS),
                None if crate::render::animations() => 0.0,
                None => 1.0,
            };
            let inner = [x + 1.0, y + 1.0, w - 2.0, h - 2.0];
            // Until its picture is in, the card shows the picture's average colour (once known),
            // and the picture fades in over it: nothing on the card pops.
            let colour = c.card_colour(&bg);
            // Everything on the card is as round as it, 1 in.
            let round = CARD_RADIUS - 1.0;
            if let Some([r, g, b]) = colour
                && (!shown || a < 1.0)
            {
                let col = [r, g, b, 1.0];
                c.round_rect(
                    [inner[0], inner[1]],
                    [inner[2], inner[3]],
                    round,
                    col,
                    col,
                    0.0,
                );
            }
            let drawn = c.card_picture(&bg, inner, [1.0, 1.0, 1.0, a], round);
            if drawn && since.is_none() {
                covers.insert(key, crate::render::frame_now());
            }
            if drawn || colour.is_some() {
                // Over the colour the shade is there at once; over the bare surface it comes in
                // with the picture.
                let base = if colour.is_some() { 1.0 } else { a };
                // One sweep from dark under the title to lighter on the right, as round as the
                // card.
                let shade = |s: f32| rgba(style::SURFACE_1, s * base);
                let (left, right) = (shade(COVER_SHADE.0), shade(COVER_SHADE.1));
                c.shade_across(
                    [inner[0], inner[1]],
                    [inner[2], inner[3]],
                    round,
                    left,
                    right,
                );
            }
        }
        if open {
            // On the beat its edge and tone light a little; in a kiai the edge turns gold.
            let kiai = style::kiai();
            let col = crate::menu::tint(style::selector_colour(), style::KIAI_GOLD, kiai);
            let beat = style::pulse();
            c.round_rect(
                [x, y],
                [w, h],
                CARD_RADIUS,
                rgba(col, 0.1 + 0.03 * beat),
                rgba(col, 0.8 + 0.2 * beat),
                1.0,
            );
        } else {
            // A faint edge round every card, so neighbouring pictures of like colours don't run
            // into each other; brighter under the pointer.
            let lift = rgba(0xffffff, 0.06 * light);
            let edge = rgba(0xffffff, CARD_EDGE + 0.08 * light);
            c.round_rect([x, y], [w, h], CARD_RADIUS, lift, edge, 1.0);
        }
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
        self.draw_lens(c, 20.0, self.frame_top() - 2.0);
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

/// The mod select (F1): a column per group, each mod a chip to switch on or off, a row of the
/// player's presets (with the speed `rate` they're compared at) and one of their favourites.
/// `cursor` is the keyboard selection (index into `mods::ALL`).
pub fn draw_mod_select(
    c: &mut Canvas,
    mods: crate::mods::Mods,
    favourites: crate::mods::Mods,
    presets: &[crate::mods::Preset],
    rate: f64,
    cursor: usize,
    seed: u64,
) {
    use crate::mods::{ALL, GROUPS};
    const GOLD: u32 = 0xffd34d;
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
    let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
    c.round_rect(
        [px, py],
        [pw, ph],
        10.0,
        rgba(0x15151d, 0.98),
        rgba(accent(), 0.8),
        1.5,
    );
    c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), "MODS");
    let mult = mods.multiplier();
    let rank = if mods.unranked() {
        "  (not saved)"
    } else if mods.in_rework() {
        "  (no pp: being reworked)"
    } else {
        ""
    };
    let score = format!("score x{mult:.2}{rank}");
    c.text_right(px + pw - 16.0, py + 14.0, 8.0, rgba(0xc8c8d8, 1.0), &score);
    let hint = "arrows + Space toggle   F / right-click favourite   S saves a preset, 1-9 apply   R shuffles   Backspace clears   C customize (Random, Multiverse, Shapeshift, Tact, OD Override)   F1 / Esc closes";
    let room = pw - 70.0 - 16.0 - Canvas::text_width(8.0, &score) - 16.0;
    c.text(
        px + 70.0,
        py + 16.0,
        6.0,
        rgba(0x6f6f86, 1.0),
        &Canvas::fit(6.0, room, hint),
    );
    let col_w = (pw - 24.0) / GROUPS.len() as f32;
    // The presets' and favourites' rows go under the longest column.
    let fav_y = py + ph - 76.0;
    let preset_y = fav_y - 22.0;
    let rows = GROUPS.iter().map(|g| g.2.len()).max().unwrap_or(1) as f32;
    let row_h = ((preset_y - 4.0 - (py + 52.0)) / rows).min(27.0);
    let mut hovered = None;
    for (g, (title, accent, list)) in GROUPS.iter().enumerate() {
        let x = px + 12.0 + g as f32 * col_w;
        c.text(
            x + 4.0,
            py + 38.0,
            7.0,
            rgba(*accent, 1.0),
            &Canvas::fit(7.0, col_w - 8.0, title),
        );
        for (r, &m) in list.iter().enumerate() {
            let y = py + 52.0 + r as f32 * row_h;
            let i = ALL.iter().position(|&x| x == m).unwrap_or(0);
            let on = mods.has(m);
            let incompatible = mods.incompatible(m);
            let accent = if incompatible { &0x555560 } else { accent };
            let h = row_h - 4.0;
            let hover = c.hovered(x, y, col_w - 6.0, h);
            if hover {
                hovered = Some(m);
            }
            let bg = if i == cursor {
                0x34344a
            } else if hover {
                0x262634
            } else {
                0x1d1d27
            };
            let border = if i == cursor {
                crate::render::accent()
            } else if on {
                *accent
            } else {
                0x2a2a38
            };
            c.round_rect(
                [x, y],
                [col_w - 6.0, h],
                5.0,
                rgba(bg, 1.0),
                rgba(border, 1.0),
                if i == cursor { 1.5 } else { 1.0 },
            );
            // The acronym's badge keeps room for its letters when there are many rows.
            let inset = ((h - 13.0) / 2.0).clamp(1.0, 4.0);
            c.round_rect(
                [x + 4.0, y + inset],
                [22.0, h - 2.0 * inset],
                3.0,
                rgba(*accent, if on { 1.0 } else { 0.0 }),
                rgba(*accent, 1.0),
                1.0,
            );
            c.text_centered(
                x + 15.0,
                y + h / 2.0 - 3.5,
                7.0,
                rgba(if on { 0x15151d } else { *accent }, 1.0),
                m.acronym(),
            );
            let fav = favourites.has(m);
            if fav {
                c.text(x + col_w - 15.0, y + 3.0, 7.0, rgba(GOLD, 1.0), "*");
            }
            // Narrow columns show just the acronym; the name is in the description below.
            let name = Canvas::fit(6.0, col_w - if fav { 48.0 } else { 40.0 }, m.name());
            c.text(
                x + 31.0,
                y + h / 2.0 - 3.0,
                6.0,
                rgba(
                    if on {
                        0xffffff
                    } else if incompatible {
                        0x626270
                    } else {
                        0x9a9ab0
                    },
                    1.0,
                ),
                &name,
            );
            c.hit(x, y, col_w - 6.0, h, Hit::ModToggle(i));
        }
    }
    // Presets: a click (or 1-9) puts on their mods and speed; lit while that's what is on.
    const TEAL: u32 = 0x5ad6e0;
    let label_w = c.text(px + 16.0, preset_y + 4.5, 7.0, rgba(TEAL, 1.0), "PRESETS");
    let mut x = px + 28.0 + label_w.max(Canvas::text_width(7.0, "* FAVOURITES"));
    let row_end = px + pw - 16.0;
    let mut hovered_preset = None;
    for (k, p) in presets.iter().enumerate() {
        let text = format!("{} {}", k + 1, p.name);
        let cw = (Canvas::text_width(6.0, &text) + 14.0).min(140.0);
        if x + cw > row_end - 60.0 {
            c.text(
                x,
                preset_y + 5.0,
                7.0,
                rgba(0x8a8aa0, 1.0),
                &format!("+{}", presets.len() - k),
            );
            x += Canvas::text_width(7.0, &format!("+{}", presets.len() - k)) + 8.0;
            break;
        }
        if c.hovered(x, preset_y, cw, 16.0) {
            hovered_preset = Some(p);
        }
        let on = p.mods == mods && (p.rate - rate).abs() < 1e-6;
        c.round_rect(
            [x, preset_y],
            [cw, 16.0],
            4.0,
            rgba(if on { TEAL } else { 0x1d1d27 }, if on { 0.9 } else { 1.0 }),
            rgba(TEAL, if on { 1.0 } else { 0.6 }),
            1.0,
        );
        c.text(
            x + 7.0,
            preset_y + 5.0,
            6.0,
            rgba(if on { 0x15151d } else { 0xc8c8d8 }, 1.0),
            &Canvas::fit(6.0, cw - 12.0, &text),
        );
        c.hit(x, preset_y, cw, 16.0, Hit::ModPreset(k));
        x += cw + 5.0;
    }
    if presets.len() < crate::mods::MAX_PRESETS {
        let save = "+ Save";
        let cw = Canvas::text_width(6.0, save) + 14.0;
        if presets.is_empty() {
            let hint = "keep the mods on now (and the speed) to put back with one click or 1-9";
            c.text(
                x + cw + 8.0,
                preset_y + 5.0,
                6.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(6.0, row_end - x - cw - 8.0, hint),
            );
        }
        let hover = c.hovered(x, preset_y, cw, 16.0);
        c.round_rect(
            [x, preset_y],
            [cw, 16.0],
            4.0,
            rgba(if hover { 0x262634 } else { 0x15151d }, 1.0),
            rgba(TEAL, 0.6),
            1.0,
        );
        c.text(x + 7.0, preset_y + 5.0, 6.0, rgba(TEAL, 1.0), save);
        c.hit(x, preset_y, cw, 16.0, Hit::ModPresetSave);
    }
    // Favourites: one click away, lit while on.
    let label_w = c.text(px + 16.0, fav_y + 4.5, 7.0, rgba(GOLD, 1.0), "* FAVOURITES");
    let mut fx = px + 28.0 + label_w.max(Canvas::text_width(7.0, "PRESETS"));
    if favourites.is_empty() {
        c.text(
            fx,
            fav_y + 5.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit(
                6.0,
                px + pw - 16.0 - fx,
                "right-click a mod (or F on it) to keep it here",
            ),
        );
    }
    let favs: Vec<crate::mods::Mod> = favourites.iter().collect();
    for (k, &m) in favs.iter().enumerate() {
        let i = ALL.iter().position(|&x| x == m).unwrap_or(0);
        let col = GROUPS
            .iter()
            .find(|g| g.2.contains(&m))
            .map_or(0x9a9ab0, |g| g.1);
        let on = mods.has(m);
        let col = if mods.incompatible(m) { 0x555560 } else { col };
        let text = format!("{} {}", m.acronym(), m.name());
        let cw = Canvas::text_width(6.0, &text) + 14.0;
        if fx + cw > px + pw - 16.0 {
            c.text(
                fx,
                fav_y + 5.0,
                7.0,
                rgba(0x8a8aa0, 1.0),
                &format!("+{}", favs.len() - k),
            );
            break;
        }
        if c.hovered(fx, fav_y, cw, 16.0) {
            hovered = Some(m);
        }
        let border = if i == cursor {
            crate::render::accent()
        } else {
            col
        };
        c.round_rect(
            [fx, fav_y],
            [cw, 16.0],
            4.0,
            rgba(if on { col } else { 0x1d1d27 }, if on { 0.9 } else { 1.0 }),
            rgba(border, 1.0),
            1.0,
        );
        c.text(
            fx + 7.0,
            fav_y + 5.0,
            6.0,
            rgba(
                if on {
                    0x15151d
                } else if mods.incompatible(m) {
                    0x626270
                } else {
                    0xc8c8d8
                },
                1.0,
            ),
            &text,
        );
        c.hit(fx, fav_y, cw, 16.0, Hit::ModToggle(i));
        fx += cw + 5.0;
    }
    // What the pointed-at preset holds, else what the pointed-at (else the selected) mod does.
    let shown = hovered.unwrap_or(ALL[cursor.min(ALL.len() - 1)]);
    let dy = py + ph - 46.0;
    if let Some(p) = hovered_preset {
        c.text(px + 16.0, dy, 8.0, rgba(0xffffff, 1.0), &p.name);
        let what = format!("{}   (click applies, right-click removes)", p.summary());
        c.text(
            px + 16.0,
            dy + 13.0,
            7.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(7.0, pw - 32.0, &what),
        );
    }
    if hovered_preset.is_none() {
        c.text(
            px + 16.0,
            dy,
            8.0,
            rgba(0xffffff, 1.0),
            &format!("{} ({})", shown.name(), shown.acronym()),
        );
        let description = if mods.incompatible(shown) {
            let removed = mods
                .iter()
                .filter(|&m| crate::mods::Mods::parse_set(shown.acronym()).incompatible(m))
                .map(|m| m.acronym())
                .collect::<Vec<_>>()
                .join(" ");
            format!("Selecting this mod switches off: {removed}")
        } else if shown == crate::mods::Mod::Random {
            format!("Seed: {seed}   C: customize seed")
        } else {
            shown.description().to_string()
        };
        c.text(px + 16.0, dy + 13.0, 7.0, rgba(0x9a9ab0, 1.0), &description);
    }
    let bw = 60.0;
    let active = if mods.is_empty() {
        "none".to_string()
    } else {
        mods.iter()
            .map(|m| m.acronym())
            .collect::<Vec<_>>()
            .join(" ")
    };
    c.text(
        px + 16.0,
        py + ph - 16.0,
        7.0,
        rgba(0xc8c8d8, 1.0),
        &Canvas::fit(7.0, pw - 4.0 * bw - 64.0, &format!("on: {active}")),
    );
    c.button(
        px + pw - bw - 12.0,
        py + ph - 24.0,
        bw,
        16.0,
        "Close",
        7.0,
        Hit::Key(KeyCode::F1),
        true,
    );
    c.button(
        px + pw - 2.0 * bw - 18.0,
        py + ph - 24.0,
        bw,
        16.0,
        "Clear",
        7.0,
        Hit::Key(KeyCode::Backspace),
        false,
    );
    c.button(
        px + pw - 3.0 * bw - 24.0,
        py + ph - 24.0,
        bw,
        16.0,
        "Shuffle",
        7.0,
        Hit::Key(KeyCode::KeyR),
        false,
    );
    // The settings of the mods that have some (Multiverse's universes, Shapeshift's shapes),
    // only while one of them is on.
    if !crate::mods::customizable(mods).is_empty() {
        c.button(
            px + pw - 4.0 * bw - 30.0,
            py + ph - 24.0,
            bw,
            16.0,
            "Customize",
            7.0,
            Hit::Key(KeyCode::KeyC),
            true,
        );
    }
}

/// The mod customization's tabs, above its panel: one per customizable mod that's on.
pub fn draw_customization_tabs(c: &mut Canvas, tabs: &[crate::mods::Mod], open: crate::mods::Mod) {
    let phone = crate::mobile::active();
    let (y, h) = if phone { (0.0, 44.0) } else { (20.0, 18.0) };
    let mut x = if phone {
        // Over the mods' own bar: its buttons aren't for now.
        c.rect(0.0, 0.0, c.width, 49.0, rgba(0x0b0b10, 1.0));
        0.0
    } else {
        16.0 + c.text(16.0, y + 4.0, 9.0, rgba(0xffffff, 1.0), "MOD CUSTOMIZATION") + 12.0
    };
    for (i, &m) in tabs.iter().enumerate() {
        let name = m.name().to_uppercase();
        let tw = Canvas::text_width(7.0, &name) + crate::mobile::pick(20.0, 36.0);
        let on = m == open;
        c.round_rect(
            [x, y],
            [tw, h],
            5.0,
            rgba(if on { 0x34344a } else { 0x1d1d27 }, 1.0),
            rgba(if on { accent() } else { 0x3a3a4e }, 1.0),
            if on { 1.5 } else { 1.0 },
        );
        c.text_centered(
            x + tw / 2.0,
            y + (h - 7.0) / 2.0,
            7.0,
            rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0),
            &name,
        );
        c.hit(x, y, tw, h, Hit::CustomTab(i));
        x += tw + 6.0;
    }
    if tabs.len() > 1 && !phone {
        c.text(x + 4.0, y + 6.0, 6.0, rgba(0x6f6f86, 1.0), "Tab: next");
    }
}

/// A difficulty, or a whole set, asked to be deleted: what the confirmation says and does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteAsk {
    /// The difficulties that go (library indices).
    pub maps: Vec<usize>,
    /// The set's whole folder goes (the set asked for, or its last difficulty).
    pub whole: bool,
    /// The whole set was asked for (not one difficulty).
    pub set: bool,
    /// The set's folder.
    pub folder: std::path::PathBuf,
    /// "Artist - Title [Oni]", or "Artist - Title" for a set.
    pub name: String,
    /// It's in osu!'s Songs folder: osu! loses it too.
    pub osu: bool,
    /// Deleted maps go to the system's recycle bin (else the game's own Trash folder).
    pub system_bin: bool,
    /// The Delete button has the keys; at first Cancel has.
    pub on_delete: bool,
}

/// Moving taiko.rs's maps out of osu!'s Songs folder, offered over song select.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveAsk {
    /// What would move.
    pub found: Vec<crate::custom::migrate::Found>,
    pub sets: usize,
    pub bytes: u64,
    /// The Move button has the keys (it has at first).
    pub on_move: bool,
}

/// The right-click menu of a song list row, opened where the pointer was (virtual units).
pub struct SongMenu {
    pub row: usize,
    /// Opened on the song's own row (not one of its difficulties).
    pub header: bool,
    /// The map has a skin of its own to turn off or on.
    pub skin: bool,
    /// A custom set in taiko.rs's own folder with an older version kept: the website's version
    /// number of that one (0: not known).
    pub previous: Option<u32>,
    pub x: f32,
    pub y: f32,
}

/// A collection as song select lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub name: String,
    /// The player's own (else osu!'s).
    pub own: bool,
    /// Its maps here (library indices), and how many of its maps aren't.
    pub maps: std::collections::HashSet<usize>,
    pub missing: usize,
}

/// A row of the list opened over song select: its label, what's marked on it ("sorted",
/// "shown", "in it") and whether it has a list of its own (a collection's "...").
pub type PickRow = (String, Option<&'static str>, bool);

/// What the list opened over song select is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickList {
    Sorts,
    Collections,
    /// The player's collections to put these maps (library indices) in.
    AddTo(Vec<usize>),
    /// What can be done with collection `k`.
    Manage(usize),
    /// Is collection `k` to be deleted?
    Delete(usize),
    /// Are the player's collections to be written into osu!'s `collection.db`?
    Export,
}

/// What a collection's name is being typed for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameFor {
    /// A new collection holding these maps (library indices; none for an empty one).
    New(Vec<usize>),
    /// Collection `k` renamed.
    Rename(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Naming {
    pub text: String,
    pub then: NameFor,
}

/// Which maps a song menu item acts on, when its row has a button for each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuScope {
    Difficulty,
    Set,
}

impl MenuScope {
    /// The words on its button.
    pub fn label(self) -> &'static str {
        match self {
            MenuScope::Difficulty => tr!("difficulty", "難易度", "сложность"),
            MenuScope::Set => tr!("whole set", "セット全体", "весь сет"),
        }
    }
}

/// The song menu's parts, drawn in this order with a line between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuPart {
    /// What the player does with the map on their computer.
    Own,
    /// What a loved nominator or an admin does with it on taiko.rs.
    Online,
    /// Deleting it, kept away from the rest at the bottom.
    Delete,
}

/// One thing the song menu offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// Its row's words ("Collect"); the items sharing a row are told apart by `scope`.
    pub label: String,
    /// The button it is in its row, or none: the whole row is the item.
    pub scope: Option<MenuScope>,
    /// Already so for its scope (loved, EPIC): it takes that back.
    pub on: bool,
    pub part: MenuPart,
    pub action: MenuAction,
}

/// The song menu's items grouped into rows: the ranges of `items` drawn on one row each.
pub fn menu_rows(items: &[MenuItem]) -> Vec<std::ops::Range<usize>> {
    let mut rows: Vec<std::ops::Range<usize>> = Vec::new();
    for (k, it) in items.iter().enumerate() {
        match rows.last_mut() {
            Some(r)
                if it.scope.is_some()
                    && items[r.start].scope.is_some()
                    && items[r.start].label == it.label
                    && items[r.start].part == it.part =>
            {
                r.end = k + 1
            }
            _ => rows.push(k..k + 1),
        }
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    /// Delete the menu's difficulty (asked first).
    DeleteDifficulty,
    /// Delete the menu's whole set (asked first).
    DeleteSet,
    /// Loved nominators: love the map set on the online server (false: take it back), just the
    /// menu's difficulty when the last is true.
    Love(u32, bool, bool),
    /// Admins: mark the menu's map EPIC on the online server (false: take the mark back), just
    /// its difficulty when the last is true (osu! sets only: a custom set is marked whole).
    Epic(bool, bool),
    /// Download the song's set again, its newest version (replacing its difficulties).
    UpdateSong,
    /// Put back the older version of a custom set kept in taiko.rs's own folder.
    PreviousVersion,
    /// Like the song, or not any more.
    Like,
    /// Turn the map's own skin off, or back on.
    MapSkin,
    /// Open the difficulty in the map editor.
    Edit,
    /// Start a new, empty difficulty from this one's timing and music.
    NewDifficulty,
    /// Put the difficulty (or the whole song, if true) in a collection, or take it out.
    Collect(bool),
}

/// The map sets loved on the online server, each with the difficulties loved (none: all of them).
pub type ServerLoved = std::collections::HashMap<u32, Vec<String>>;

/// Whether `versions` (a set's in `ServerLoved`) take in difficulty `version`.
pub fn covers(versions: &[String], version: &str) -> bool {
    versions.is_empty()
        || versions
            .iter()
            .any(|v| v.trim().eq_ignore_ascii_case(version.trim()))
}

/// The maps marked EPIC on the online server: osu! sets with the difficulties marked (none: all
/// of them), and custom sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerEpic {
    pub sets: ServerLoved,
    pub custom: std::collections::HashSet<u32>,
}

impl ServerEpic {
    pub fn from_list(list: crate::online::EpicList) -> ServerEpic {
        ServerEpic {
            sets: list.sets.into_iter().collect(),
            custom: list.custom.into_iter().collect(),
        }
    }

    /// Whether map `e` is marked: its custom set (`custom`, when it's in one), or its osu! set,
    /// or this difficulty of it. `custom` is asked only when it can change the answer: a search
    /// asks this of every map, and finding a map's custom set hashes its whole path.
    pub fn covers(&self, e: &Entry, custom: impl FnOnce() -> Option<u32>) -> bool {
        // A map's set id may be read off its folder's name: not for an empty list.
        let in_set = !self.sets.is_empty() && loved_here(&self.sets, e);
        if !in_set && self.custom.is_empty() {
            return false;
        }
        match custom() {
            Some(id) => self.custom.contains(&id),
            None => in_set,
        }
    }
}

/// Whether map `e` is loved on the online server: its set, or this difficulty of it.
pub fn loved_here(loved: &ServerLoved, e: &Entry) -> bool {
    e.set_id()
        .and_then(|set| loved.get(&set))
        .is_some_and(|v| covers(v, &e.meta.version))
}

/// The name a map's folder goes by in `map_skins_off`: the same wherever the Songs folder is.
fn folder_name(dir: &std::path::Path) -> Option<String> {
    Some(dir.file_name()?.to_string_lossy().into_owned())
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
    /// The right-click menu, next to the pointer and kept on screen: the map's name once on top,
    /// then a row per action, an action on the difficulty or the whole set with a button for each.
    pub(crate) fn draw_menu(&self, c: &mut Canvas, library: &[Entry]) {
        // Sized for a thumb on a phone.
        let phone = crate::mobile::active();
        let Some(m) = &self.menu else { return };
        let Some(&i) = self.visible.get(m.row) else {
            return;
        };
        let items = self.menu_items(library);
        let rows = menu_rows(&items);
        let (mut row, size, pad): (f32, f32, f32) = if phone {
            (44.0, 12.0, 10.0)
        } else {
            (18.0, 7.0, 6.0)
        };
        let (title_size, sub_size, head) = if phone {
            (13.0, 10.0, 48.0)
        } else {
            (8.0, 6.0, 31.0)
        };
        // A line between parts, the online one with its name on it.
        let (gap, caption) = if phone { (12.0, 24.0) } else { (7.0, 13.0) };
        let bw = if phone { 380.0 } else { 236.0 };
        let chip_w = [MenuScope::Difficulty, MenuScope::Set]
            .iter()
            .map(|s| Canvas::text_width(size, s.label()))
            .fold(0.0, f32::max)
            + pad * 2.0;
        let part_gap = |p: MenuPart| match p {
            MenuPart::Own => 0.0,
            MenuPart::Online => caption,
            MenuPart::Delete => gap,
        };
        let mut bh = head + 8.0;
        let mut part = MenuPart::Own;
        for r in &rows {
            if items[r.start].part != part {
                part = items[r.start].part;
                bh += part_gap(part);
            }
        }
        // An admin's menu on a phone has the most rows: they shrink to stay on screen.
        row = row.min((464.0 - bh) / rows.len().max(1) as f32);
        bh += row * rows.len() as f32;
        let x = m.x.min(c.width - bw - 8.0).max(8.0);
        let y = m.y.min(480.0 - bh - 8.0).max(8.0);
        c.round_rect(
            [x, y],
            [bw, bh],
            6.0,
            rgba(0x15151d, 0.98),
            rgba(accent(), 0.8),
            1.5,
        );
        // The map's name, once for every row under it.
        let e = &library[i];
        let (title, sub) = if m.header {
            (e.meta.title.clone(), e.meta.artist.clone())
        } else {
            (e.meta.title.clone(), format!("[{}]", e.meta.version))
        };
        let tw = bw - 24.0;
        let top = y + if phone { 8.0 } else { 6.0 };
        c.text(
            x + 12.0,
            top,
            title_size,
            rgba(0xffffff, 1.0),
            &Canvas::fit(title_size, tw, &title),
        );
        c.text(
            x + 12.0,
            top + title_size + if phone { 6.0 } else { 3.0 },
            sub_size,
            rgba(0x9a9ab4, 1.0),
            &Canvas::fit(sub_size, tw, &sub),
        );
        let mut ry = y + head;
        c.rect(x + 8.0, ry - 2.0, bw - 16.0, 1.0, rgba(0x34344a, 1.0));
        let mut part = MenuPart::Own;
        for r in rows {
            let first = &items[r.start];
            if first.part != part {
                part = first.part;
                let line = ry + part_gap(part) / 2.0;
                if part == MenuPart::Online {
                    let label = "taiko.rs";
                    let lw = Canvas::text_width(sub_size, label);
                    c.text(
                        x + 12.0,
                        line - sub_size / 2.0,
                        sub_size,
                        rgba(0x7a7a96, 1.0),
                        label,
                    );
                    let lx = x + 18.0 + lw;
                    c.rect(lx, line, x + bw - 8.0 - lx, 1.0, rgba(0x34344a, 1.0));
                } else {
                    c.rect(x + 8.0, line, bw - 16.0, 1.0, rgba(0x34344a, 1.0));
                }
                ry += part_gap(part);
            }
            let danger = part == MenuPart::Delete;
            let colour = if danger {
                crate::style::DANGER
            } else {
                0xffffff
            };
            let (rx, rw, rh) = (x + 4.0, bw - 8.0, row - 2.0);
            let text_y = ry + (rh - size) / 2.0;
            if first.scope.is_none() {
                if c.hovered(rx, ry, rw, rh) {
                    c.round_rect(
                        [rx, ry],
                        [rw, rh],
                        4.0,
                        rgba(0x34344a, 1.0),
                        rgba(accent(), 1.0),
                        1.0,
                    );
                }
                c.hit(rx, ry, rw, rh, Hit::SongMenu(r.start));
                c.text(
                    x + 12.0,
                    text_y,
                    size,
                    rgba(colour, 1.0),
                    &Canvas::fit(size, bw - 20.0, &first.label),
                );
                ry += row;
                continue;
            }
            // The set's button on the right, the difficulty's left of it: in columns.
            let right = x + bw - 8.0;
            let chip_x = |s: MenuScope| match s {
                MenuScope::Set => right - chip_w,
                MenuScope::Difficulty => right - chip_w * 2.0 - 4.0,
            };
            c.text(
                x + 12.0,
                text_y,
                size,
                rgba(colour, 1.0),
                &Canvas::fit(size, right - chip_w * 2.0 - 8.0 - (x + 12.0), &first.label),
            );
            for k in r {
                let it = &items[k];
                let Some(scope) = it.scope else { continue };
                let (cx, cy, ch) = (chip_x(scope), ry + 1.0, rh - 2.0);
                let hover = c.hovered(cx, cy, chip_w, ch);
                // Lit in the colour its status shows in, not the accent: that can be red.
                let lit = match it.action {
                    MenuAction::Epic(..) => 0x9b7bff,
                    _ => 0xff66aa,
                };
                let (fill, border) = match (it.on, hover, danger) {
                    (true, false, _) => (rgba(lit, 0.85), rgba(lit, 1.0)),
                    (true, true, _) => (rgba(lit, 0.55), rgba(0xffffff, 1.0)),
                    (false, true, true) => (
                        rgba(crate::style::DANGER, 0.3),
                        rgba(crate::style::DANGER, 1.0),
                    ),
                    (false, true, false) => (rgba(0x34344a, 1.0), rgba(accent(), 1.0)),
                    (false, false, true) => (rgba(0x1d1d27, 1.0), rgba(crate::style::DANGER, 0.55)),
                    (false, false, false) => (rgba(0x1d1d27, 1.0), rgba(0x4a4a60, 1.0)),
                };
                c.round_rect([cx, cy], [chip_w, ch], 4.0, fill, border, 1.0);
                c.hit(cx, cy, chip_w, ch, Hit::SongMenu(k));
                let label = scope.label();
                let lw = Canvas::text_width(size, label);
                c.text(
                    cx + (chip_w - lw) / 2.0,
                    cy + (ch - size) / 2.0,
                    size,
                    rgba(if it.on { 0xffffff } else { colour }, 1.0),
                    label,
                );
            }
            ry += row;
        }
    }

    /// The collection or sort list over the song list.
    fn draw_picker(&self, c: &mut Canvas, cursor: usize) {
        // Sized for a thumb on a phone.
        let phone = crate::mobile::active();
        let (row, size) = if phone { (40.0, 12.0) } else { (18.0, 7.0) };
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.5));
        let (title, rows, hint) = self.picker_rows();
        let (bw, max_h) = if phone {
            (440.0, 472.0)
        } else {
            (320.0, 380.0)
        };
        let bh = (rows.len() as f32 * row + if phone { 46.0 } else { 56.0 }).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        c.round_rect(
            [bx, by],
            [bw, bh],
            8.0,
            rgba(0x15151d, 0.98),
            rgba(accent(), 0.8),
            1.5,
        );
        let title_size = if phone { 12.0 } else { 9.0 };
        c.text(
            bx + 14.0,
            by + 12.0,
            title_size,
            rgba(0xffffff, 1.0),
            &Canvas::fit(title_size, bw - 28.0, &title),
        );
        let (top, view) = if phone {
            (by + 38.0, bh - 44.0)
        } else {
            (by + 32.0, bh - 50.0)
        };
        let scroll = (cursor as f32 * row - view / 2.0)
            .clamp(0.0, (rows.len() as f32 * row - view).max(0.0));
        for (i, label) in rows.iter().enumerate() {
            let y = top + i as f32 * row - scroll;
            if y < top - 1.0 || y + row > top + view + 1.0 {
                continue;
            }
            let selected = i == cursor;
            let (label, mark, more) = (&label.0, label.1, label.2);
            let hover = c.hovered(bx + 8.0, y, bw - 16.0, row - 2.0);
            if selected || hover {
                c.round_rect(
                    [bx + 8.0, y],
                    [bw - 16.0, row - 2.0],
                    4.0,
                    rgba(0x34344a, 1.0),
                    rgba(if selected { accent() } else { 0x3a3a4e }, 1.0),
                    1.0,
                );
            }
            c.hit(bx + 8.0, y, bw - 16.0, row, Hit::PickRow(i));
            // A collection's own list (rename, delete, ...), to its right.
            let more_w = if more { row.max(28.0) } else { 0.0 };
            if more {
                let mx = bx + bw - 8.0 - more_w;
                c.hit(mx, y, more_w, row, Hit::PickRowMore(i));
                c.text_centered(
                    mx + more_w / 2.0,
                    y + (row - 2.0 - size) / 2.0,
                    size,
                    rgba(0x9a9ab0, 1.0),
                    "...",
                );
            }
            let shown_w = mark.map_or(0.0, |m| Canvas::text_width(size, m) + 10.0) + more_w;
            let ty = y + (row - 2.0 - size) / 2.0;
            c.text(
                bx + 20.0,
                ty,
                size,
                rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0),
                &Canvas::fit(size, bw - 40.0 - shown_w, label),
            );
            if let Some(mark) = mark {
                c.text_right(bx + bw - 20.0 - more_w, ty, size, rgba(0x86E05A, 1.0), mark);
            }
        }
        if !phone {
            c.text_centered(
                w / 2.0,
                by + bh - 11.0,
                6.0,
                rgba(0x6f6f86, 1.0),
                &Canvas::fit(6.0, bw - 24.0, hint),
            );
        }
    }

    pub fn picker_len(&self) -> usize {
        self.picker_rows().1.len()
    }

    /// How many collections are the player's own (they come first).
    pub fn own_collections(&self) -> usize {
        self.collections.iter().take_while(|c| c.own).count()
    }

    /// The open list's title, its rows (each with what's marked on it and whether it has a list
    /// of its own) and the keys it takes.
    pub fn picker_rows(&self) -> (String, Vec<PickRow>, &'static str) {
        let new = || ("+ New collection".to_string(), None, false);
        let count = |c: &Shown| {
            let n = c.maps.len();
            let mut label = match c.missing {
                0 => format!("{}  ({n})", c.name),
                m => format!("{}  ({n}, {m} not here)", c.name),
            };
            if !c.own {
                label += "  osu!";
            }
            label
        };
        match &self.list {
            PickList::Sorts => (
                "Sort songs by".into(),
                SortMode::ALL
                    .iter()
                    .map(|&m| {
                        (
                            m.name().to_string(),
                            (m == self.sort).then_some("sorted"),
                            false,
                        )
                    })
                    .collect(),
                "Up/Down or mouse   Enter/click sort   Esc close",
            ),
            PickList::Collections => (
                "Collections".into(),
                std::iter::once((
                    "All songs".to_string(),
                    self.collection.is_none().then_some("shown"),
                    false,
                ))
                .chain(self.collections.iter().enumerate().map(|(k, c)| {
                    (
                        count(c),
                        (self.collection == Some(k)).then_some("shown"),
                        true,
                    )
                }))
                .chain(std::iter::once(new()))
                .chain(
                    self.can_export
                        .then(|| ("Export my collections to osu!".to_string(), None, false)),
                )
                .collect(),
                "Enter show   Tab more   F2 rename   Del delete",
            ),
            PickList::AddTo(maps) => (
                if maps.len() == 1 {
                    "Put the difficulty in".into()
                } else {
                    "Put the song in".into()
                },
                self.collections[..self.own_collections()]
                    .iter()
                    .map(|c| {
                        let all = maps.iter().all(|i| c.maps.contains(i));
                        (count(c), all.then_some("in it"), false)
                    })
                    .chain(std::iter::once(new()))
                    .collect(),
                "Enter/click puts it in (or takes it out)   Esc close",
            ),
            PickList::Manage(k) => {
                let Some(c) = self.collections.get(*k) else {
                    return (String::new(), Vec::new(), "");
                };
                let rows: &[&str] = if c.own {
                    &["Rename", "Move up", "Move down", "Delete"]
                } else {
                    &["Copy to my collections"]
                };
                let mut rows: Vec<PickRow> =
                    rows.iter().map(|r| (r.to_string(), None, false)).collect();
                if c.missing > 0 {
                    rows.push((
                        match c.missing {
                            1 => "Download the map not here".into(),
                            n => format!("Download the {n} maps not here"),
                        },
                        None,
                        false,
                    ));
                }
                (c.name.clone(), rows, "Enter/click   Esc back")
            }
            PickList::Export => (
                "Put your collections in osu!?".into(),
                vec![
                    (
                        "Export (osu!'s stay; the old file is kept as collection.db.bak)".into(),
                        None,
                        false,
                    ),
                    ("Cancel".into(), None, false),
                ],
                "With osu! closed   Enter/click   Esc cancel",
            ),
            PickList::Delete(k) => {
                let Some(c) = self.collections.get(*k) else {
                    return (String::new(), Vec::new(), "");
                };
                (
                    format!("Delete {}?", c.name),
                    vec![
                        (
                            match c.maps.len() {
                                1 => "Delete it (1 map; the map stays)".to_string(),
                                n => format!("Delete it ({n} maps; the maps stay)"),
                            },
                            None,
                            false,
                        ),
                        ("Keep it".into(), None, false),
                    ],
                    "Enter/click   Esc keep it",
                )
            }
        }
    }
}

/// What a results screen works out in the background: a leaderboard score's whole stats (its
/// replay played through), the pp breakdown, and an online score's replay.
#[derive(Default)]
pub struct Detail {
    pub seed: Option<u64>,
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
    pub seed: Option<u64>,
    /// The map the player can like or dislike here (their own play, logged in), and what
    /// they said on this screen.
    pub vote_on: Option<crate::online::PlayedMap>,
    pub vote: Option<crate::online::MapVote>,
    /// When the player last voted here (the thumb's pop and burst).
    pub voted_at: Option<std::time::Instant>,
}

/// How far down the results scroll: the pp start a screen below them.
pub const RESULTS_SCROLL: f32 = 456.0;
/// The bottom of what scrolls (the buttons stay put under it).
const RESULTS_VIEW: f32 = 416.0;

impl Results {
    /// Says `vote` about the map, or takes it back when it's what was said already: the map
    /// and the player's say on it now (None: no say), or None when there's nothing to vote on.
    pub fn toggle_vote(
        &mut self,
        vote: crate::online::MapVote,
    ) -> Option<(crate::online::PlayedMap, Option<crate::online::MapVote>)> {
        let map = self.vote_on.clone()?;
        self.vote = (self.vote != Some(vote)).then_some(vote);
        self.voted_at = Some(std::time::Instant::now());
        Some((map, self.vote))
    }

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
            seed: None,
            vote_on: None,
            vote: None,
            voted_at: None,
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
        if let Some(seed) = d.seed {
            self.seed = Some(seed);
        }
        if let Some((st, windows)) = d.stats {
            // The counts stay the leaderboard's; the replay adds what it doesn't keep.
            let s = &mut self.stats;
            (s.errors, s.error_offsets, s.roll_ticks, s.swells, s.strong) = (
                st.errors,
                st.error_offsets,
                st.roll_ticks,
                st.swells,
                st.strong,
            );
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
        c.text_centered(
            cx,
            24.0 + (12.0 - size) / 2.0,
            size,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(size, w - 48.0, &self.title),
        );
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
        if self.mods.has(crate::mods::Mod::Random)
            && let Some(seed) = self.seed
        {
            c.text_centered(
                cx,
                66.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &format!("Random seed: {seed}"),
            );
        }
        c.text_centered(
            cx,
            170.0,
            28.0,
            rgba(0xffffff, 1.0),
            &format!("{:.2}%", acc * count as f64),
        );
        c.text_centered(
            cx,
            212.0,
            16.0,
            rgba(0xc8c8d8, 1.0),
            &format!("{:08}", (s.score as f64 * count as f64) as u64),
        );
        if let Some(pp) = self.pp {
            c.text_centered(
                cx,
                236.0,
                10.0,
                rgba(0xff8ad8, 1.0),
                &format!("{:.0} pp", pp * count as f64),
            );
        }

        let lines = [
            (0xFFD34D, format!("GREAT  {}", s.great)),
            (0x86E05A, format!("OK     {}", s.ok)),
            (0xFF4B4B, format!("MISS   {}", s.miss)),
            (0xffffff, format!("MAX COMBO  {}", s.max_combo)),
        ];
        for (i, (col, text)) in lines.iter().enumerate() {
            c.text(
                cx - 170.0,
                260.0 + i as f32 * 24.0,
                12.0,
                rgba(*col, 1.0),
                text,
            );
        }
        let right = if self.partial {
            [
                "UR      -".to_string(),
                "MEAN    -".into(),
                "DRUMROLL -".into(),
                "BIG x2  -".into(),
            ]
        } else {
            [
                format!("UR      {:.1}", s.unstable_rate()),
                format!("MEAN    {:+.1} ms", s.mean_error()),
                format!("DRUMROLL {}", s.roll_ticks),
                format!("BIG x2  {}", s.strong),
            ]
        };
        for (i, text) in right.iter().enumerate() {
            c.text(
                cx + 30.0,
                260.0 + i as f32 * 24.0,
                12.0,
                rgba(0xc8c8d8, 1.0),
                text,
            );
        }
        self.draw_vote(c, cx);
        let hint = if let Some(o) = &self.opened {
            let place = if o.online { "online" } else { "local" };
            format!(
                "{place} score by {}, {} ago",
                o.player,
                crate::scores::age(o.date)
            )
        } else if s.errors.len() >= 20 && s.mean_error().abs() > 5.0 {
            let dir = if s.mean_error() > 0.0 {
                "late"
            } else {
                "early"
            };
            format!(
                "You hit {:.0} ms {dir} on average; press {} a few times to adjust this map's offset.",
                s.mean_error().abs(),
                // As with the global offset: lower it when you hit late.
                if s.mean_error() > 0.0 { "-" } else { "=" }
            )
        } else {
            format!(
                "Windows: great +-{:.0}  ok +-{:.0} ms",
                self.windows.great, self.windows.ok
            )
        };
        c.text_centered(
            cx,
            380.0,
            9.0,
            rgba(0x8a8aa0, 1.0),
            &Canvas::fit(9.0, w - 48.0, &hint),
        );
        if let Some(e) = &self.replay_error {
            c.text_centered(
                cx,
                400.0,
                9.0,
                rgba(0xEB452C, 1.0),
                &Canvas::fit(9.0, w - 48.0, &format!("no replay: {e}")),
            );
        } else if self.opened.as_ref().is_some_and(|o| o.online) && self.replay.is_none() {
            c.text_centered(cx, 400.0, 9.0, rgba(0x4fb4ff, 1.0), "getting the replay...");
        } else if let Some(who) = &self.watched {
            c.text_centered(
                cx,
                400.0,
                9.0,
                rgba(0x4fb4ff, 1.0),
                &format!("replay of {who}"),
            );
        } else if let Some((rank, of)) = self.rank {
            c.text_centered(
                cx,
                400.0,
                9.0,
                rgba(0xFFD34D, 1.0),
                &format!("#{rank} of {of} local scores on this map"),
            );
        }
        self.draw_performance(c, count as f64);
        c.transform_since(page, [0.0, 0.0], 1.0, 0.0, -self.scroll);
        c.clip_since(page, 0.0, 0.0, w, RESULTS_VIEW);

        let mut buttons = Vec::new();
        if self.opened.is_none() {
            buttons.push(("Retry  (Enter)", Hit::Key(KeyCode::Enter), true));
        }
        if self.replay.is_some() {
            buttons.push((
                "Watch replay  (R)",
                Hit::Key(KeyCode::KeyR),
                self.opened.is_some(),
            ));
            buttons.push(("Export video  (V)", Hit::Key(KeyCode::KeyV), false));
        }
        let back = if self.opened.is_some() {
            "Back  (Esc)"
        } else {
            "Song select  (Esc)"
        };
        buttons.push((back, Hit::Key(KeyCode::Escape), false));
        // Taller on a phone: clear of the pill under them.
        let row_y = if crate::mobile::active() {
            402.0
        } else {
            424.0
        };
        c.button_row(row_y, &buttons);
        // Where the pp went: one click (or the wheel) away, and back.
        // Where the pp went: a pill under the buttons, its arrows bobbing the way to go.
        let down = self.scroll_to < RESULTS_SCROLL / 2.0;
        let (cue, key) = match (down, self.pp) {
            (true, Some(pp)) => (
                format!("scroll down: {pp:.0} pp breakdown"),
                KeyCode::PageDown,
            ),
            (true, None) => ("scroll down: performance".to_string(), KeyCode::PageDown),
            (false, _) => ("scroll up: results".to_string(), KeyCode::PageUp),
        };
        let a = progress(self.shown + std::time::Duration::from_millis(600), 300.0);
        let (cw, ch, cy) = (Canvas::text_width(8.0, &cue) + 48.0, 18.0, 450.0);
        let x0 = cx - cw / 2.0;
        let hover = c.hovered(x0, cy, cw, ch);
        let pink = 0xff8ad8;
        c.round_rect(
            [x0, cy],
            [cw, ch],
            5.0,
            rgba(if hover { 0x2b1f2a } else { 0x1d1d27 }, 0.95 * a),
            rgba(pink, if hover { a } else { 0.75 * a }),
            1.5,
        );
        c.text_centered(
            cx,
            cy + (ch - 8.0) / 2.0,
            8.0,
            rgba(if hover { 0xffffff } else { pink }, a),
            &cue,
        );
        let t = self.shown.elapsed().as_secs_f32();
        let bob = 1.5 * (t * 4.0).sin() * if down { 1.0 } else { -1.0 };
        for ax in [x0 + 14.0, x0 + cw - 14.0] {
            chevron(c, ax, cy + ch / 2.0 + bob, down, rgba(pink, a));
        }
        c.hit(x0, cy, cw, ch, Hit::Key(key));
    }

    /// Like and Not for me, right of the stats, in the main menu's look: one panel, the two
    /// answers on it as rows, each a pixel thumb (outlined and white like the menu's notes, its
    /// cuff in the answer's colour) and its name. Under the pointer a row lights up faintly; the
    /// answer given gets the selector over a tint of its colour, its thumb pops and throws pixels
    /// out, and the other answer goes grey.
    fn draw_vote(&self, c: &mut Canvas, cx: f32) {
        use crate::online::MapVote::{Dislike, Like};
        if self.vote_on.is_none() {
            return;
        }
        let a = progress(self.shown + std::time::Duration::from_millis(800), 300.0);
        if a <= 0.0 {
            return;
        }
        let phone = crate::mobile::active();
        let (full_w, row_h, label): (f32, f32, f32) = if phone {
            (196.0, 44.0, 13.0)
        } else {
            (172.0, 34.0, 10.0)
        };
        // Right of the stats, level with them and kept on screen. Where the answers' names
        // would run into the stats (a narrow window), the thumbs and keys alone.
        let at = |w: f32| (cx + VOTE_FROM_MIDDLE).min(c.width - 16.0 - w);
        let named = at(full_w) >= cx + STATS_RIGHT_END;
        let pw = if named { full_w } else { 84.0 };
        let (x, y) = (at(pw), 250.0);
        let ph = 36.0 + 2.0 * row_h + 4.0;
        let start = c.shapes.len();
        crate::style::panel(c, x, y, pw, ph);
        let title = "Rate this map";
        let size = Canvas::fit_size(9.0, 6.0, pw - 16.0, title);
        crate::style::title(
            c,
            x + pw / 2.0 - Canvas::text_width(size, title) / 2.0,
            y + 11.0 + (9.0 - size) / 2.0,
            size,
            0xffffff,
            title,
        );
        c.rect(x + 12.0, y + 28.0, pw - 24.0, 1.0, rgba(0x2a2a38, 1.0));
        let since = self.voted_at.map(|v| v.elapsed().as_secs_f32() * 1000.0);
        let rows = [
            (Like, 0x5ee07a, "Like", "L", KeyCode::KeyL),
            (Dislike, 0xff5a5a, "Not for me", "N", KeyCode::KeyN),
        ];
        let (rx, rw) = (x + 8.0, pw - 16.0);
        let mut icons = Vec::new();
        for (i, (vote, col, name, key_name, key)) in rows.into_iter().enumerate() {
            let ry = y + 34.0 + i as f32 * row_h;
            let rh = row_h - 4.0;
            let chosen = self.vote == Some(vote);
            let other = self.vote.is_some() && !chosen;
            // The page may be scrolled: the pointer is over where the row is drawn.
            let hover = self.scroll < 1.0 && c.hovered(rx, ry - self.scroll, rw, rh);
            if chosen {
                c.round_rect(
                    [rx, ry],
                    [rw, rh],
                    7.0,
                    rgba(col, 0.14 + 0.05 * crate::style::pulse()),
                    rgba(0, 0.0),
                    0.0,
                );
                crate::style::selector(c, "vote", [rx - 3.0, ry - 3.0, rw + 6.0, rh + 6.0]);
            } else if hover {
                c.round_rect(
                    [rx, ry],
                    [rw, rh],
                    7.0,
                    rgba(0x262634, 0.6),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            c.hit(rx, ry, rw, rh, Hit::Key(key));
            let mut unit = if chosen || hover { 1.0 } else { 0.85 };
            if phone {
                unit *= 1.3;
            }
            if chosen && let Some(ms) = since.filter(|ms| *ms < 320.0) {
                unit *= 1.0 + 0.35 * (1.0 - ms / 320.0).powi(2);
            }
            let look = if other { Look::Grey } else { Look::Colour(col) };
            let (ix, iy) = (rx + if phone { 24.0 } else { 18.0 }, ry + rh / 2.0);
            icons.push((ix, iy, unit, vote == Dislike, look, chosen.then_some(col)));
            let text_col = if other {
                0x5a5a6a
            } else if chosen || hover {
                0xffffff
            } else {
                0xa8a8bc
            };
            if named {
                let tx = rx + if phone { 50.0 } else { 38.0 };
                let room = rw - (tx - rx) - 16.0;
                c.text(
                    tx,
                    ry + (rh - label) / 2.0,
                    label,
                    rgba(text_col, 1.0),
                    &Canvas::fit(label, room, name),
                );
            }
            if !phone {
                let kw = Canvas::text_width(7.0, key_name);
                c.text(
                    rx + rw - 8.0 - kw,
                    ry + (rh - 7.0) / 2.0,
                    7.0,
                    rgba(0x6a6a80, 1.0),
                    key_name,
                );
            }
        }
        c.fade_since(start, a, 1.0);
        // The thumbs over their rows, drawn at the screen's own alpha (their pixels overlap a
        // little when solid, which a fade would show).
        for (ix, iy, unit, down, look, burst_col) in icons {
            pixel_thumb(c, ix, iy, crate::render::pixel_size(unit), down, look, a);
            if let (Some(col), Some(ms)) = (burst_col, since.filter(|ms| *ms < 600.0)) {
                burst(c, ix, iy, ms / 600.0, col);
            }
        }
    }

    /// The mods (and a speed of the player's own) as badges under the title, each in its mod
    /// select column's colour: up to two rows centred on `cx` from `y`, the rest counted.
    fn draw_mods(&self, c: &mut Canvas, cx: f32, y: f32) {
        let colour = |m: crate::mods::Mod| {
            crate::mods::GROUPS
                .iter()
                .find(|g| g.2.contains(&m))
                .map_or(0xc8c8d8, |g| g.1)
        };
        // A failed play says so first, in red.
        let failed = self.failed.then(|| ("FAILED".to_string(), 0xEB452C));
        let mut chips: Vec<(String, u32)> = failed
            .into_iter()
            .chain(
                self.mods
                    .iter()
                    .map(|m| (m.acronym().to_string(), colour(m))),
            )
            .collect();
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
                        if last.is_empty()
                            || last.iter().map(|(l, _)| width(l) + GAP).sum::<f32>() + width(&more)
                                <= room
                        {
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
            let total = row.iter().map(|(l, _)| width(l)).sum::<f32>()
                + GAP * row.len().saturating_sub(1) as f32;
            let (mut x, ry) = (cx - total / 2.0, y + r as f32 * (H + 3.0));
            for (label, col) in row {
                let w = width(label);
                c.round_rect([x, ry], [w, H], 4.0, rgba(*col, 0.2), rgba(*col, 0.9), 1.0);
                c.text_centered(
                    x + w / 2.0,
                    ry + (H - SIZE) / 2.0,
                    SIZE,
                    rgba(*col, 1.0),
                    label,
                );
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
        let h = if self.breakdown.is_some() {
            200.0
        } else {
            80.0
        };
        let (x, y) = (
            (w - pw) / 2.0,
            RESULTS_SCROLL + 64.0 + ((RESULTS_VIEW - 64.0 - h) / 2.0).max(0.0),
        );
        let (label, value) = (rgba(0x8a8aa0, 1.0), rgba(0xffffff, 1.0));
        let Some(b) = self.breakdown else {
            c.round_rect(
                [x, y],
                [pw, h],
                8.0,
                rgba(0x15151d, 0.9),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            c.text(x + 16.0, y + 12.0, 8.0, label, "PERFORMANCE");
            let note = if self.job.is_some() {
                "working it out..."
            } else {
                self.no_pp.unwrap_or("no pp for this play")
            };
            c.text_centered(
                x + pw / 2.0,
                y + 44.0,
                9.0,
                label,
                &Canvas::fit(9.0, pw - 32.0, note),
            );
            return;
        };
        c.round_rect(
            [x, y],
            [pw, h],
            8.0,
            rgba(0x15151d, 0.9),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        c.text(x + 16.0, y + 12.0, 8.0, label, "PERFORMANCE");
        crate::style::star_pill(c, x + pw - 14.0, y + 9.0, 8.0, Some(b.stars as f32));
        // Two columns: the pp and its parts, then what could have been and the total.
        let col_w = (pw - 48.0) / 2.0;
        let (lx, rx) = (x + 16.0, x + 32.0 + col_w);
        let big = c.text(
            lx,
            y + 34.0,
            26.0,
            rgba(0xff8ad8, 1.0),
            &format!("{:.0}", b.pp * count),
        );
        c.text(lx + 6.0 + big, y + 49.0, 10.0, rgba(0xff8ad8, 1.0), "pp");
        let of_ss = if b.if_ss > 0.0 {
            b.pp / b.if_ss * 100.0
        } else {
            0.0
        };
        c.text(lx, y + 72.0, 7.0, label, &format!("{of_ss:.0}% of an SS"));
        for (i, (name, v, max)) in [
            ("Difficulty", b.difficulty, b.ss_difficulty),
            ("Accuracy", b.accuracy, b.ss_accuracy),
        ]
        .into_iter()
        .enumerate()
        {
            let by = y + 100.0 + i as f32 * 40.0;
            c.text(lx, by, 8.0, rgba(0xc8c8d8, 1.0), name);
            c.text_right(lx + col_w, by, 8.0, value, &format!("{:.0}", v * count));
            c.round_rect(
                [lx, by + 15.0],
                [col_w, 6.0],
                3.0,
                rgba(0x2a2a38, 1.0),
                rgba(0, 0.0),
                0.0,
            );
            let f = if max > 0.0 {
                ((v / max).clamp(0.0, 1.0) * count) as f32
            } else {
                0.0
            };
            if f > 0.0 {
                c.round_rect(
                    [lx, by + 15.0],
                    [(col_w * f).max(6.0), 6.0],
                    3.0,
                    rgba(accent(), 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
            }
        }
        let mut rows = vec![
            ("Full combo", format!("{:.0} pp", b.if_fc), 0xffffff),
            ("SS", format!("{:.0} pp", b.if_ss), 0xffffff),
            (
                "Combo",
                format!("{} / {}", self.stats.max_combo, b.max_combo),
                0xffffff,
            ),
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
                rows.push((
                    "Rank",
                    t.rank
                        .map(|r| format!("#{r}"))
                        .unwrap_or_else(|| "-".into()),
                    0xc8c8d8,
                ));
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

/// How far right of the screen's middle the vote panel starts, and where the stats' right column
/// ends (its widest line, "MEAN    +0.0 ms", drawn from 30 right of the middle).
const VOTE_FROM_MIDDLE: f32 = 205.0;
const STATS_RIGHT_END: f32 = 195.0;

/// A thumbs up in pixels: `O` the outline, `H` light, `Y` the hand, `S` its shade, `D` the
/// creases between the fingers, `h` / `C` / `c` the cuff lit, plain and shaded (in the vote's
/// colour). Turned half a turn it is the thumbs down.
const THUMB: [&str; 24] = [
    "..........OOOO............",
    ".........OHHHSO...........",
    "........OHYYYSO...........",
    "........OHYYYSO...........",
    "........OHYYYSO...........",
    ".......OHYYYYSO...........",
    ".......OHYYYYSO...........",
    ".......OHYYYYSOOOOOOOOO...",
    ".OOOOOOHYYYYYYHHHHHHHHSO..",
    "OhhhhOHYYYYYYYYYYYYYYYSO..",
    "OhCCCOYYYYYYYYYYYYYYYYSO..",
    "OhCCCOYYYYYDDDDDDDDDDOOO..",
    "OhCCCOYYYYYYYYYYYYYYYYHSO.",
    "OhCCCOYYYYYYYYYYYYYYYYYSO.",
    "OhCCCOYYYYYYYYYYYYYYYYSSO.",
    "OhCCCOYYYYYDDDDDDDDDDOOO..",
    "OhCCCOYYYYYYYYYYYYYYYYSO..",
    "OhCCCOYYYYYYYYYYYYYYYYSO..",
    "OhCCCOYYYYYYYYYYYYYYYSSO..",
    "OhCCCOYYYYYDDDDDDDDDOOO...",
    "OhCCCOYYYYYYYYYYYYYYYSO...",
    "OhCCCOSYYYYYYYYYYYYYYSO...",
    "OhccccOHSSSSSSSSSSSSSSO...",
    ".OOOOO.OOOOOOOOOOOOOOO....",
];

/// How a thumb is drawn: white like the menu's notes with its cuff in `colour`, or all grey.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Colour(u32),
    Grey,
}

/// The colour of the sprite letter `ch` in `look`; the main menu's notes' palette: a dark
/// outline, white lit top left and shaded bottom right.
fn thumb_colour(ch: u8, look: Look) -> u32 {
    use crate::menu::tint;
    match (ch, look) {
        (b'O', _) => 0x0f0f16,
        (b'H', Look::Colour(_)) => 0xffffff,
        (b'Y', Look::Colour(_)) => 0xececf4,
        (b'S', Look::Colour(_)) => 0xc8c8d8,
        (b'D', Look::Colour(_)) => 0x9a9ab0,
        (b'C', Look::Colour(cuff)) => cuff,
        (b'h', Look::Colour(cuff)) => tint(cuff, 0xffffff, 0.3),
        (b'c', Look::Colour(cuff)) => tint(cuff, 0x000000, 0.28),
        (b'H' | b'h', Look::Grey) => 0x6a6a7a,
        (b'Y' | b'C', Look::Grey) => 0x5a5a6a,
        _ => 0x46465a,
    }
}

/// The rows of the thumb glitched like the menu's notes: (row, how many pixels the red goes out
/// on the left and the cyan on the right).
const THUMB_GLITCH: [(usize, usize); 3] = [(9, 1), (13, 2), (19, 1)];

/// Draws `THUMB` centred on (`x`, `y`) in `px`-sized pixels, upside down for a thumbs down, at
/// `alpha`. In colour it has the logo's red and cyan glitch down its sides.
fn pixel_thumb(c: &mut Canvas, x: f32, y: f32, px: f32, down: bool, look: Look, alpha: f32) {
    let (w, h) = (THUMB[0].len(), THUMB.len());
    let (x0, y0) = (
        crate::render::snap_to_grid(x - w as f32 * px / 2.0),
        crate::render::snap_to_grid(y - h as f32 * px / 2.0),
    );
    // Each row in runs of one colour, one strip each. Solid strips overlap a little so no seams
    // show at any size; see-through ones can't (the overlaps would show darker).
    let seal = if alpha >= 0.99 { 0.5 } else { 0.0 };
    let row_at = |r: usize| -> Vec<u8> {
        let row = THUMB[if down { h - 1 - r } else { r }].as_bytes();
        if down {
            row.iter().rev().copied().collect()
        } else {
            row.to_vec()
        }
    };
    for r in 0..h {
        let row = row_at(r);
        let mut k0 = 0;
        while k0 < w {
            let k1 = k0 + row[k0..].iter().take_while(|&&ch| ch == row[k0]).count();
            if row[k0] != b'.' {
                c.rect(
                    x0 + k0 as f32 * px,
                    y0 + r as f32 * px,
                    (k1 - k0) as f32 * px + seal,
                    px + seal,
                    rgba(thumb_colour(row[k0], look), alpha),
                );
            }
            k0 = k1;
        }
    }
    if look == Look::Grey {
        return;
    }
    for (r, out) in THUMB_GLITCH {
        let r = if down { h - 1 - r } else { r };
        let row = row_at(r);
        let (Some(left), Some(right)) = (
            row.iter().position(|&ch| ch != b'.'),
            row.iter().rposition(|&ch| ch != b'.'),
        ) else {
            continue;
        };
        let yy = y0 + r as f32 * px;
        c.rect(
            x0 + (left as f32 - out as f32) * px,
            yy,
            out as f32 * px,
            px,
            rgba(0x9f2137, alpha),
        );
        c.rect(
            x0 + (right + 1) as f32 * px,
            yy,
            out as f32 * px,
            px,
            rgba(0x2a9fb5, alpha),
        );
    }
}

/// Pixels thrown out from (`x`, `y`) when a vote is cast, `t` from 0 to 1 over their flight:
/// in the vote's colour and white, slowing and fading as they go.
fn burst(c: &mut Canvas, x: f32, y: f32, t: f32, col: u32) {
    let e = 1.0 - (1.0 - t).powi(3);
    for k in 0..10 {
        let angle = k as f32 * std::f32::consts::TAU / 10.0 + 0.26;
        let dist = 16.0 + 18.0 * e + if k % 2 == 0 { 4.0 } else { 0.0 };
        let size = 3.0 * (1.0 - 0.5 * t);
        let hex = if k % 3 == 0 { 0xffffff } else { col };
        c.rect(
            x + angle.cos() * dist - size / 2.0,
            y + angle.sin() * dist - size / 2.0,
            size,
            size,
            rgba(hex, 1.0 - t),
        );
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

#[cfg(test)]
mod thumb_tests {
    use super::*;

    /// The vote panel is right of the stats, clear of every line of them, and on screen: on a
    /// wide screen, a 16:9 one and the narrowest window (where it shows the thumbs alone).
    #[test]
    fn the_vote_stays_clear_of_the_stats_on_every_width() {
        let stats = crate::game::Stats {
            great: 1234,
            ok: 567,
            miss: 89,
            max_combo: 1890,
            roll_ticks: 321,
            strong: 456,
            ..Default::default()
        };
        for width in [1066.0, 853.3, 640.0] {
            let mut r = Results::new("map".into(), stats.clone(), crate::game::Windows::new(5.0));
            r.shown = std::time::Instant::now() - std::time::Duration::from_secs(10);
            r.vote_on = Some(crate::online::PlayedMap {
                hash: 1,
                name: "map".into(),
                set_id: None,
            });
            let mut c = Canvas {
                width,
                audit: true,
                ..Default::default()
            };
            r.draw(&mut c);
            let votes: Vec<[f32; 4]> = c
                .hits
                .iter()
                .filter(|h| matches!(h.hit, Hit::Key(KeyCode::KeyL | KeyCode::KeyN)))
                .map(|h| h.rect)
                .collect();
            assert_eq!(votes.len(), 2, "at {width}");
            let lines = [
                "GREAT",
                "OK ",
                "MISS",
                "MAX COMBO",
                "UR ",
                "MEAN",
                "DRUMROLL",
                "BIG x2",
            ];
            for (range, text) in &c.texts {
                if !lines.iter().any(|l| text.starts_with(l)) {
                    continue;
                }
                let right = c.shapes[range.clone()]
                    .iter()
                    .map(|s| s.center[0] + s.half[0])
                    .fold(f32::MIN, f32::max);
                for v in &votes {
                    assert!(
                        right < v[0],
                        "at {width}: {text:?} runs to {right}, the vote from {}",
                        v[0]
                    );
                    assert!(v[0] + v[2] <= width, "at {width}: the vote off screen");
                }
            }
        }
    }

    /// The vote thumb is drawn row by row from the first row's width: every row has it, and
    /// only the sprite's letters are in it; the glitched rows are in it.
    #[test]
    fn the_vote_thumb_is_a_whole_sprite() {
        let w = THUMB[0].len();
        for row in THUMB {
            assert_eq!(row.len(), w, "{row}");
            assert!(row.bytes().all(|ch| b".OHYSDChc".contains(&ch)), "{row}");
        }
        assert!(THUMB_GLITCH.iter().all(|&(r, _)| r < THUMB.len()));
    }
}
