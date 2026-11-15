//! Beatmap downloads from public osu! mirrors (osu.direct, then catboy.best): searching for
//! sets and fetching `.osz` archives into the Songs folder, on background threads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde::Deserialize;

use crate::net;

/// Mirrors want clients to identify themselves.
const USER_AGENT: &str = concat!(
    "taiko-rs/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Lynerrr/taiko-rs)"
);

struct Mirror {
    name: &'static str,
    search: &'static str,
    /// A set's details (osu! API v2 beatmapset) by its ID.
    set: &'static str,
    query_key: &'static str,
    amount_key: &'static str,
    download: &'static str,
}

/// In the order they're tried. osu.direct first: it can sort searches on its side, and
/// catboy.best rate-limits downloads per IP (425) after a handful of sets.
const MIRRORS: [Mirror; 2] = [
    Mirror {
        name: "osu.direct",
        search: "https://osu.direct/api/v2/search",
        set: "https://osu.direct/api/v2/s/",
        query_key: "query",
        amount_key: "amount",
        download: "https://osu.direct/api/d/",
    },
    Mirror {
        name: "catboy.best",
        search: "https://catboy.best/api/v2/search",
        set: "https://catboy.best/api/v2/s/",
        query_key: "q",
        amount_key: "limit",
        download: "https://catboy.best/d/",
    },
];

/// Search categories, like the osu! website's (beatmap status filters).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    HasLeaderboard,
    Ranked,
    Qualified,
    Loved,
    Pending,
    Wip,
    Graveyard,
    Any,
    /// Custom sets on the taiko.rs website that give pp.
    CustomApproved,
    /// Every custom set on the taiko.rs website.
    Custom,
    /// osu! sets loved on the taiko.rs server (they give pp there), from the mirrors.
    LovedHere,
    /// Maps marked EPIC on the taiko.rs server: osu! sets from the mirrors, and custom sets.
    Epic,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::HasLeaderboard,
        Category::Ranked,
        Category::Qualified,
        Category::Loved,
        Category::Pending,
        Category::Wip,
        Category::Graveyard,
        Category::Any,
    ];

    /// The categories of taiko.rs's own maps: the website's custom sets, and the osu! sets
    /// loved on the server.
    pub const OURS: [Category; 4] = [
        Category::CustomApproved,
        Category::LovedHere,
        Category::Epic,
        Category::Custom,
    ];

    /// Whether this is one of taiko.rs's lists, not an osu! status.
    pub fn is_ours(self) -> bool {
        Category::OURS.contains(&self)
    }

    /// Whether this lists the taiko.rs website's custom sets (not osu! sets).
    pub fn is_custom(self) -> bool {
        matches!(self, Category::CustomApproved | Category::Custom)
    }

    pub fn name(self) -> &'static str {
        match self {
            Category::HasLeaderboard => "Has leaderboard",
            Category::Ranked => "Ranked",
            Category::Qualified => "Qualified",
            Category::Loved => "Loved",
            Category::Pending => "Pending",
            Category::Wip => "WIP",
            Category::Graveyard => "Graveyard",
            Category::Any => "Any",
            Category::CustomApproved => "Custom approved",
            Category::Custom => "All custom",
            Category::LovedHere => "Loved here",
            Category::Epic => "EPIC",
        }
    }

    /// osu! ranked status codes to ask for (empty: any).
    fn statuses(self) -> &'static [i32] {
        match self {
            Category::HasLeaderboard => &[1, 2, 3, 4],
            Category::Ranked => &[1, 2],
            Category::Qualified => &[3],
            Category::Loved => &[4],
            Category::Pending => &[0],
            Category::Wip => &[-1],
            Category::Graveyard => &[-2],
            Category::Any
            | Category::CustomApproved
            | Category::Custom
            | Category::LovedHere
            | Category::Epic => &[],
        }
    }
}

/// Search orders, like the osu! website's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    /// The mirror's order (best match); with no search text, newest ranked first.
    Relevance,
    Title,
    Artist,
    Difficulty,
    Ranked,
    Rating,
    Plays,
    Favourites,
}

impl SortKey {
    pub const ALL: [SortKey; 8] = [
        SortKey::Relevance,
        SortKey::Title,
        SortKey::Artist,
        SortKey::Difficulty,
        SortKey::Ranked,
        SortKey::Rating,
        SortKey::Plays,
        SortKey::Favourites,
    ];

    /// The orders the taiko.rs website's maps can be listed in.
    pub const OURS: [SortKey; 5] = [
        SortKey::Relevance,
        SortKey::Title,
        SortKey::Artist,
        SortKey::Ranked,
        SortKey::Plays,
    ];

    /// What it's called for the taiko.rs website's maps (sent, not ranked; downloaded, not played).
    pub fn name_for_ours(self) -> &'static str {
        match self {
            SortKey::Ranked => "Updated",
            SortKey::Plays => "Downloads",
            key => key.name(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SortKey::Relevance => "Relevance",
            SortKey::Title => "Title",
            SortKey::Artist => "Artist",
            SortKey::Difficulty => "Difficulty",
            SortKey::Ranked => "Ranked",
            SortKey::Rating => "Rating",
            SortKey::Plays => "Plays",
            SortKey::Favourites => "Favourites",
        }
    }

    /// The natural direction when first picked (text A-Z, numbers high to low).
    pub fn default_desc(self) -> bool {
        !matches!(self, SortKey::Title | SortKey::Artist | SortKey::Difficulty)
    }

    /// osu.direct's name for it, when it can sort that way itself.
    fn server(self) -> Option<&'static str> {
        match self {
            SortKey::Title => Some("title"),
            SortKey::Artist => Some("artist"),
            SortKey::Ranked => Some("ranked_date"),
            SortKey::Plays => Some("play_count"),
            SortKey::Favourites => Some("favourite_count"),
            SortKey::Relevance | SortKey::Difficulty | SortKey::Rating => None,
        }
    }
}

/// A beatmap set found on a mirror (only its taiko difficulties are kept), or a custom set on
/// the taiko.rs website (`custom` is then set and `id` is the website's number, not osu!'s).
#[derive(Clone, Debug, Default)]
pub struct SetInfo {
    pub id: u32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub status: String,
    /// Taiko difficulties, easiest first.
    pub diffs: Vec<Diff>,
    pub play_count: u64,
    pub favourite_count: u64,
    pub rating: f32,
    /// ISO 8601, empty if never ranked.
    pub ranked_date: String,
    /// What the website says of a custom set (None: an osu! set).
    pub custom: Option<CustomSet>,
}

/// A custom set on the taiko.rs website.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomSet {
    /// Every difficulty of it is approved (it gives pp).
    pub approved: bool,
    /// Its upload count: a new version replaces the old on the website.
    pub version: u32,
    /// The archive's size.
    pub bytes: u64,
    /// Seconds since 1970 of its last upload.
    pub updated: u64,
    /// A difficulty of it is a `.tko` (plays only in taiko.rs).
    pub tko: bool,
    /// The website has its background (`/c/<id>/bg`), for the download tab's cover.
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    /// osu! beatmap ID (0 when unknown).
    pub id: u32,
    pub version: String,
    pub stars: f32,
}

/// Preview clips are saved under this name; the preview treats such audio as a short clip
/// that starts at the map's preview point.
pub const CLIP_PREFIX: &str = "taiko-rs-preview-clip";

impl SetInfo {
    /// Whether `other` is the same set: the website's custom numbers aren't osu!'s.
    pub fn same(&self, other: &SetInfo) -> bool {
        self.id == other.id && self.custom.is_some() == other.custom.is_some()
    }

    /// Whether this is osu! set `id` (never a custom set).
    pub fn is_osu_set(&self, id: Option<u32>) -> bool {
        self.custom.is_none() && Some(self.id) == id
    }

    /// Folder name like osu! uses: "123456 Artist - Title", made safe for any filesystem. A
    /// custom set's is "c12 Artist - Title", which no osu! set number is read from.
    pub fn folder_name(&self) -> String {
        let prefix = if self.custom.is_some() { "c" } else { "" };
        let raw = format!("{prefix}{} {} - {}", self.id, self.artist, self.title);
        let clean: String = raw
            .chars()
            .map(|c| {
                if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        clean
            .trim_end_matches(['.', ' '])
            .chars()
            .take(120)
            .collect()
    }
}

#[derive(Deserialize)]
struct ApiSet {
    id: u32,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    creator: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    beatmaps: Vec<ApiMap>,
    #[serde(default)]
    play_count: u64,
    #[serde(default)]
    favourite_count: u64,
    #[serde(default)]
    rating: f32,
    #[serde(default)]
    ranked_date: Option<String>,
}

#[derive(Deserialize)]
struct ApiMap {
    #[serde(default)]
    id: u32,
    #[serde(default)]
    version: String,
    #[serde(default)]
    mode_int: Option<u8>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    difficulty_rating: f32,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

/// Parses a mirror's search response (osu! API v2 beatmapset objects): the sets with taiko
/// difficulties, and how many sets the reply had (a full page means there may be more).
fn parse_sets(json: &str) -> Result<(Vec<SetInfo>, usize), String> {
    let sets: Vec<ApiSet> =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply: {e}"))?;
    let count = sets.len();
    let sets = sets
        .into_iter()
        .filter_map(|s| {
            let mut diffs: Vec<Diff> = s
                .beatmaps
                .iter()
                .filter(|b| b.mode_int == Some(1) || b.mode.as_deref() == Some("taiko"))
                .map(|b| Diff {
                    id: b.id,
                    version: b.version.clone(),
                    stars: b.difficulty_rating,
                })
                .collect();
            diffs.sort_by(|a, b| a.stars.total_cmp(&b.stars));
            (!diffs.is_empty()).then_some(SetInfo {
                id: s.id,
                artist: s.artist,
                title: s.title,
                creator: s.creator,
                status: s.status,
                diffs,
                play_count: s.play_count,
                favourite_count: s.favourite_count,
                rating: s.rating,
                ranked_date: s.ranked_date.unwrap_or_default(),
                custom: None,
            })
        })
        .collect();
    Ok((sets, count))
}

/// Sets a search asks each mirror for at a time.
pub const PAGE: usize = 50;

/// Page `page` of one status (or any) from the first mirror that answers.
fn search_status(
    agent: &ureq::Agent,
    query: &str,
    status: Option<i32>,
    sort: Option<(&str, bool)>,
    page: usize,
) -> Result<(Vec<SetInfo>, usize), String> {
    crate::offline::internet()?;
    let mut errors = Vec::new();
    for m in &MIRRORS {
        let mut req = agent
            .get(m.search)
            .header("User-Agent", USER_AGENT)
            .query(m.query_key, query)
            .query("mode", "1")
            .query(m.amount_key, PAGE.to_string())
            .query("offset", (page * PAGE).to_string());
        if let Some(st) = status {
            req = req.query("status", st.to_string());
        }
        if let (Some((key, desc)), "osu.direct") = (sort, m.name) {
            req = req.query(
                "sort",
                format!("{key}:{}", if desc { "desc" } else { "asc" }),
            );
        }
        let result = req
            .call()
            .map_err(|e| e.to_string())
            .and_then(|mut r| {
                r.body_mut()
                    .with_config()
                    .limit(64 << 20)
                    .read_to_string()
                    .map_err(|e| e.to_string())
            })
            .and_then(|body| parse_sets(&body));
        match result {
            Ok(sets) => return Ok(sets),
            Err(e) => errors.push(format!("{}: {e}", m.name)),
        }
    }
    Err(errors.join("; "))
}

/// A set's status on osu! ("ranked", "loved", "graveyard"...), from the mirrors; `setstatus::MISSING`
/// when they all say they don't have the set.
pub fn set_status(set: u32) -> Result<String, String> {
    crate::offline::internet()?;
    #[derive(Deserialize)]
    struct Set {
        status: String,
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let mut errors = Vec::new();
    let mut not_found = 0;
    for base in [
        "https://osu.direct/api/v2/s/",
        "https://catboy.best/api/v2/s/",
    ] {
        let result = agent
            .get(&format!("{base}{set}"))
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| {
                not_found += matches!(e, ureq::Error::StatusCode(404)) as usize;
                e.to_string()
            })
            .and_then(|mut r| r.body_mut().read_to_string().map_err(|e| e.to_string()))
            .and_then(|body| serde_json::from_str::<Set>(&body).map_err(|e| e.to_string()));
        match result {
            Ok(s) => return Ok(s.status),
            Err(e) => errors.push(e),
        }
    }
    if not_found == errors.len() {
        return Ok(crate::setstatus::MISSING.to_string());
    }
    Err(errors.join("; "))
}

/// Searches the mirrors for taiko sets matching `query` in `category`, ordered by `sort`
/// (descending if `desc`): page `page` (from 0) of the results. Categories spanning several
/// statuses are fetched per status and merged. Also says whether there may be another page.
pub fn search(
    query: &str,
    category: Category,
    sort: SortKey,
    desc: bool,
    page: usize,
) -> Result<(Vec<SetInfo>, bool), String> {
    let agent = agent();
    // With no text, "relevance" means newest ranked first, as on the website.
    let (sort, desc) = if sort == SortKey::Relevance && query.trim().is_empty() {
        (SortKey::Ranked, true)
    } else {
        (sort, desc)
    };
    let server = sort.server().map(|k| (k, desc));
    let statuses: Vec<Option<i32>> = if category.statuses().is_empty() {
        vec![None]
    } else {
        category.statuses().iter().map(|&s| Some(s)).collect()
    };
    let results: Vec<Result<(Vec<SetInfo>, usize), String>> = std::thread::scope(|s| {
        let handles: Vec<_> = statuses
            .iter()
            .map(|&st| {
                let agent = &agent;
                s.spawn(move || search_status(agent, query, st, server, page))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err("search failed".into())))
            .collect()
    });
    let mut errors = Vec::new();
    let mut sets: Vec<SetInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut more = false;
    for r in results {
        match r {
            Ok((found, count)) => {
                more |= count >= PAGE;
                sets.extend(found.into_iter().filter(|s| seen.insert(s.id)));
            }
            Err(e) => errors.push(e),
        }
    }
    if sets.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    sort_sets(&mut sets, sort, desc);
    Ok((sets, more))
}

/// The sets loved on the server (`ids`, from its `LovedMaps`) that match `query`, ordered by
/// `sort` (`Relevance`: newest set first). There is no mirror call for many sets at once, so
/// each is asked for on its own, the sets shared out between the two mirrors and paced under
/// their limits, and each answer is kept on disk for `LOVED_FRESH` (a set's details hardly
/// change): only newly loved sets cost a request then. `progress` gets the list so far, and how
/// many sets are done of how many, as they come. Sets no mirror has, or with no taiko
/// difficulty, are left out; it fails only if none could be had.
pub fn loved_here(
    ids: &[u32],
    query: &str,
    sort: SortKey,
    desc: bool,
    progress: &(dyn Fn(Vec<SetInfo>, usize, usize) + Sync),
) -> Result<Vec<SetInfo>, String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    // Fetched sets by id (None: no taiko difficulty, so it's never asked again).
    static KNOWN: OnceLock<Mutex<HashMap<u32, Option<SetInfo>>>> = OnceLock::new();
    let known = KNOWN.get_or_init(Mutex::default);
    let listed = |known: &HashMap<u32, Option<SetInfo>>| {
        let mut sets: Vec<SetInfo> = ids
            .iter()
            .filter_map(|id| known.get(id).cloned().flatten())
            .filter(|set| matches_query(set, query))
            .collect();
        sets.sort_by_key(|s| std::cmp::Reverse(s.id));
        sort_sets(&mut sets, sort, desc);
        sets
    };
    // One load at a time: a search typed while the list loads waits for it, then finds the
    // sets known, instead of asking the mirrors for them all over again.
    static LOADING: Mutex<()> = Mutex::new(());
    let _loading = LOADING.lock().unwrap_or_else(|e| e.into_inner());
    // Newest first, as the list starts out: what comes in mostly goes below what's shown.
    let mut missing: Vec<u32> = {
        let known = known.lock().unwrap();
        ids.iter()
            .copied()
            .filter(|id| !known.contains_key(id))
            .collect()
    };
    missing.sort_unstable_by_key(|&id| std::cmp::Reverse(id));
    let errors = Mutex::new(Vec::new());
    let started = std::time::Instant::now();
    if !missing.is_empty() {
        let agent = agent();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done = std::sync::atomic::AtomicUsize::new(0);
        let lanes = [Lane::new(), Lane::new()];
        let have = ids.len() - missing.len();
        std::thread::scope(|s| {
            for _ in 0..missing.len().min(LOVED_WORKERS) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&id) = missing.get(i) else { break };
                        // Every other set from the other mirror first.
                        let result = loved_set(&agent, &lanes, i % MIRRORS.len(), id);
                        let mut known = known.lock().unwrap();
                        match result {
                            Ok(set) => {
                                known.insert(id, set);
                            }
                            Err(e) => errors.lock().unwrap().push(format!("set {id}: {e}")),
                        }
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let so_far = listed(&known);
                        drop(known);
                        progress(so_far, have + n, ids.len());
                    }
                });
            }
        });
    }
    let sets = listed(&known.lock().unwrap());
    let errors = errors.into_inner().unwrap();
    if sets.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    if !missing.is_empty() {
        eprintln!(
            "loved here: {} of {} sets looked up (on disk or the mirrors) in {:.1} s, {} not had{}",
            missing.len(),
            ids.len(),
            started.elapsed().as_secs_f32(),
            errors.len(),
            errors
                .first()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
    }
    Ok(sets)
}

/// Mirror requests the loved list has under way at once.
const LOVED_WORKERS: usize = 4;
/// Between two requests to the same mirror: osu.direct allows 120 a minute (after a burst of
/// 25), catboy.best 1200 an hour, and the game asks them for other things meanwhile.
const LOVED_PACE: Duration = Duration::from_millis(550);
/// How long a set's details kept on disk are used without asking again.
const LOVED_FRESH: Duration = Duration::from_secs(7 * 24 * 3600);

/// Where the mirrors' answers about loved sets are kept, one file per set.
fn loved_dir() -> PathBuf {
    covers_dir().with_file_name("loved-sets")
}

/// One loved set: from disk while fresh, else from the mirrors (and kept), else from disk however
/// old. None: it has no taiko difficulty.
fn loved_set(
    agent: &ureq::Agent,
    lanes: &[Lane],
    first: usize,
    id: u32,
) -> Result<Option<SetInfo>, String> {
    let file = loved_dir().join(format!("{id}.json"));
    let parse =
        |body: &str| parse_sets(&format!("[{body}]")).map(|(sets, _)| sets.into_iter().next());
    let kept = std::fs::read_to_string(&file).ok();
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < LOVED_FRESH));
    if fresh && let Some(set) = kept.as_deref().and_then(|b| parse(b).ok()) {
        return Ok(set);
    }
    match mirror_set(agent, lanes, first, id) {
        Ok(body) => {
            let set = parse(&body)?;
            let _ = std::fs::create_dir_all(loved_dir());
            let _ = std::fs::write(&file, &body);
            Ok(set)
        }
        Err(e) => kept.as_deref().map(parse).unwrap_or(Err(e)),
    }
}

/// One mirror's share of the loved list: when its next request may go, and whether it has
/// refused us (403) for the rest of this load.
struct Lane {
    next: std::sync::Mutex<std::time::Instant>,
    refused: AtomicBool,
}

impl Lane {
    fn new() -> Lane {
        Lane {
            next: std::sync::Mutex::new(std::time::Instant::now()),
            refused: AtomicBool::new(false),
        }
    }

    /// Waits for this mirror's turn.
    fn wait_turn(&self) {
        let at = {
            let mut next = self.next.lock().unwrap();
            let at = (*next).max(std::time::Instant::now());
            *next = at + LOVED_PACE;
            at
        };
        std::thread::sleep(at.saturating_duration_since(std::time::Instant::now()));
    }

    /// Holds every request to this mirror back for `pause`.
    fn back_off(&self, pause: Duration) {
        let mut next = self.next.lock().unwrap();
        *next = (*next).max(std::time::Instant::now() + pause);
    }
}

/// A set's details from the first mirror that has them, starting at mirror `first`, each
/// request in its mirror's turn (`lanes`). A "too many requests" holds that mirror back a while;
/// a "forbidden" (catboy.best's guard sometimes turns us away) drops it for the rest of the
/// load, unless no mirror is left.
fn mirror_set(
    agent: &ureq::Agent,
    lanes: &[Lane],
    first: usize,
    id: u32,
) -> Result<String, String> {
    crate::offline::internet()?;
    let mut errors = Vec::new();
    let all_refused = || lanes.iter().all(|l| l.refused.load(Ordering::Relaxed));
    for k in 0..MIRRORS.len() {
        let at = (first + k) % MIRRORS.len();
        let (m, lane) = (&MIRRORS[at], &lanes[at]);
        if lane.refused.load(Ordering::Relaxed) && !all_refused() {
            continue;
        }
        for attempt in 0..3 {
            lane.wait_turn();
            let result = agent
                .get(&format!("{}{id}", m.set))
                .header("User-Agent", USER_AGENT)
                .call()
                .and_then(|mut r| r.body_mut().read_to_string());
            match result {
                Ok(body) => return Ok(body),
                Err(ureq::Error::StatusCode(429)) if attempt < 2 => {
                    // osu.direct's burst allowance comes back within 5 s.
                    lane.back_off(Duration::from_secs(5));
                }
                Err(e) => {
                    if matches!(e, ureq::Error::StatusCode(403))
                        && !lane.refused.swap(true, Ordering::Relaxed)
                    {
                        eprintln!("loved here: {} refused us (403), asking the other", m.name);
                    }
                    errors.push(format!("{}: {e}", m.name));
                    break;
                }
            }
        }
    }
    Err(errors.join("; "))
}

/// Whether every word of `query` is in the set's artist, title, mapper or a difficulty's name
/// (or is its id), ignoring case.
fn matches_query(set: &SetInfo, query: &str) -> bool {
    let haystack = std::iter::once(set.artist.as_str())
        .chain([set.title.as_str(), set.creator.as_str()])
        .chain(set.diffs.iter().map(|d| d.version.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let id = set.id.to_string();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| word == id || haystack.contains(word))
}

/// Orders merged results (and does the sorts no mirror can do).
pub fn sort_sets(sets: &mut [SetInfo], sort: SortKey, desc: bool) {
    use std::cmp::Ordering;
    let hardest = |s: &SetInfo| s.diffs.last().map(|d| d.stars).unwrap_or(0.0);
    let order = |a: &SetInfo, b: &SetInfo| -> Ordering {
        match sort {
            SortKey::Relevance => Ordering::Equal,
            SortKey::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            SortKey::Artist => a.artist.to_lowercase().cmp(&b.artist.to_lowercase()),
            SortKey::Difficulty => hardest(a).total_cmp(&hardest(b)),
            // Custom sets: by when they were last sent.
            SortKey::Ranked if a.custom.is_some() && b.custom.is_some() => {
                let at = |s: &SetInfo| s.custom.as_ref().map_or(0, |c| c.updated);
                at(a).cmp(&at(b))
            }
            // Unranked sets have no date: they go last either way.
            SortKey::Ranked => match (a.ranked_date.is_empty(), b.ranked_date.is_empty()) {
                (false, false) => a.ranked_date.cmp(&b.ranked_date),
                (x, y) => y.cmp(&x).reverse(),
            },
            SortKey::Rating => a.rating.total_cmp(&b.rating),
            SortKey::Plays => a.play_count.cmp(&b.play_count),
            SortKey::Favourites => a.favourite_count.cmp(&b.favourite_count),
        }
    };
    if sort != SortKey::Relevance {
        sets.sort_by(|a, b| if desc { order(b, a) } else { order(a, b) });
    }
}

/// A download in progress. `done`/`total` are bytes (total 0 until known).
pub struct Job {
    pub set: SetInfo,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub rx: Receiver<Result<PathBuf, String>>,
    /// Content hashes of the difficulties an import brings (empty for downloads), so the same
    /// set dropped twice isn't unpacked twice.
    pub maps: Vec<u64>,
    /// A newer version of a set that's already here (see [`start_update`]).
    pub update: bool,
    /// Set to stop the download: it then ends with [`CANCELLED`].
    pub cancel: Arc<AtomicBool>,
    /// Set when what was unpacked is an older version of the set than osu! has: no mirror had
    /// the current one, and osu! didn't hand over its changed files either.
    pub old_version: Arc<AtomicBool>,
}

/// What a download stopped by [`Job::cancel`] ends with.
pub const CANCELLED: &str = "cancelled";

impl Job {
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// Downloads set `set` into `songs` (as "<id> <artist> - <title>/") on a background thread.
/// The job reports the extracted folder.
pub fn start(set: SetInfo, songs: PathBuf) -> Job {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (d, t) = (done.clone(), total.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let name = set.folder_name();
    // A custom set the preview fetched already is unpacked from there.
    let cached = set
        .custom
        .is_some()
        .then(|| crate::custom::cached_archive(&set));
    let sources = match &set.custom {
        Some(_) => vec![("taiko.rs", crate::custom::download_url(set.id))],
        None => mirror_sources(set.id),
    };
    // An osu! set is checked against osu!'s MD5s; a taiko.rs custom set has none.
    let mut check = set.custom.is_none().then(|| Check::set(set.id));
    let old_version = Arc::new(AtomicBool::new(false));
    let old = old_version.clone();
    std::thread::spawn(move || {
        let result = match cached.flatten() {
            Some(archive) => std::fs::create_dir_all(&songs)
                .map_err(|e| format!("{}: {e}", songs.display()))
                .and_then(|()| {
                    let dest = songs.join(&name);
                    extract(&archive, &dest).map(|()| dest)
                }),
            None => fetch(&sources, &name, &songs, &d, &t, &stop, check.as_mut()),
        };
        old.store(check.is_some_and(|c| c.older), Ordering::Relaxed);
        let _ = tx.send(result);
    });
    Job {
        set,
        done,
        total,
        rx,
        maps: Vec::new(),
        update: false,
        cancel,
        old_version,
    }
}

/// A difficulty in a set's folder, for an update: its beatmap ID, its [Version] and its file.
pub struct LocalDiff {
    pub id: Option<u32>,
    pub version: String,
    pub path: PathBuf,
}

/// Downloads set `set` again into its folder `folder`, for the newer version osu! has. Of
/// `here` (the difficulties in the folder), only the set's own (`is_own`, asked of osu! first)
/// make way for the new ones; everything else in the folder stays: a difficulty of your own,
/// a copy with a rate baked in or OD changed, whatever IDs it carries. If the download fails
/// they're put back.
pub fn start_update(set: SetInfo, folder: PathBuf, here: Vec<LocalDiff>) -> Job {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (d, t) = (done.clone(), total.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let old_version = Arc::new(AtomicBool::new(false));
    let old = old_version.clone();
    let id = set.id;
    std::thread::spawn(move || {
        let result = (|| {
            let songs = folder.parent().ok_or("no Songs folder")?;
            let name = folder
                .file_name()
                .ok_or("no set folder")?
                .to_string_lossy()
                .into_owned();
            let online = set_checksums(id)?;
            let own: Vec<LocalDiff> = here
                .into_iter()
                .filter(|m| {
                    std::fs::read(&m.path)
                        .is_ok_and(|bytes| is_own(m.id, &m.version, &md5_hex(&bytes), &online))
                })
                .collect();
            // Like osu! itself: only the files that changed, and no mirror asked at all.
            match update_from_osu(&folder, &own, &online, OSU_FILES) {
                Ok(()) => return Ok(folder.clone()),
                Err(e) => eprintln!("set {id}: osu!'s own files didn't do ({e}), downloading it"),
            }
            let replaced: Vec<PathBuf> = own.into_iter().map(|m| m.path).collect();
            let mut check = Check::new(json_sources(id), Some(online));
            let result = replacing(&replaced, || {
                fetch(
                    &mirror_sources(id),
                    &name,
                    songs,
                    &d,
                    &t,
                    &stop,
                    Some(&mut check),
                )
            });
            old.store(check.older, Ordering::Relaxed);
            result
        })();
        let _ = tx.send(result);
    });
    Job {
        set,
        done,
        total,
        rx,
        maps: Vec::new(),
        update: true,
        cancel,
        old_version,
    }
}

/// Downloads custom set `set`'s newest version from the website into its folder `folder`: the
/// archive's files take the place of those of the same name, and what else is in the folder (a
/// difficulty of your own) stays.
pub fn start_custom_update(set: SetInfo, folder: PathBuf) -> Job {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (d, t) = (done.clone(), total.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let sources = vec![("taiko.rs", crate::custom::download_url(set.id))];
    std::thread::spawn(move || {
        let result = (|| {
            let songs = folder.parent().ok_or("no Songs folder")?;
            let name = folder
                .file_name()
                .ok_or("no set folder")?
                .to_string_lossy()
                .into_owned();
            fetch(&sources, &name, songs, &d, &t, &stop, None)
        })();
        let _ = tx.send(result);
    });
    Job {
        set,
        done,
        total,
        rx,
        maps: Vec::new(),
        update: true,
        cancel,
        old_version: Arc::default(),
    }
}

/// What installing website set `set` says of it (`custom::store::Install`).
fn custom_install(set: &SetInfo, folder: Option<PathBuf>) -> crate::custom::store::Install {
    let custom = set.custom.clone().unwrap_or_default();
    crate::custom::store::Install {
        id: set.id,
        version: (custom.version > 0).then_some(custom.version),
        updated: (custom.updated > 0).then_some(custom.updated),
        origin: Some(crate::custom::Origin::Site),
        name: set.folder_name(),
        folder,
    }
}

/// Downloads website set `set` (or takes the archive its preview fetched) and installs it whole
/// into taiko.rs's own custom maps folder `custom` (`custom::store::install`), on a background
/// thread: a first download, or (`update`) its newer version over the folder that has it
/// (`into`, else the one holding the set). A failed download leaves the folder as it was. The
/// job reports the set's folder.
pub fn start_custom(set: SetInfo, custom: PathBuf, update: Option<PathBuf>) -> Job {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (d, t) = (done.clone(), total.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let cached = crate::custom::cached_archive(&set);
    let url = crate::custom::download_url(set.id);
    let install = custom_install(&set, update.clone());
    let update = update.is_some();
    std::thread::spawn(move || {
        let result = (|| {
            if let Some(archive) = cached {
                return crate::custom::store::install(&custom, &archive, &install);
            }
            // Not to be tried again (tests reach no website).
            crate::offline::url(&url).map_err(|e| format!("taiko.rs: {e}"))?;
            let dir = crate::custom::store::set_versions(&custom, install.id);
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let archive = dir.join("download.rsz.part");
            let mut error = String::new();
            for round in 0..ROUNDS {
                if round > 0 {
                    let until = std::time::Instant::now() + Duration::from_secs(5 * round);
                    while std::time::Instant::now() < until && !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&archive);
                    return Err(CANCELLED.into());
                }
                d.store(0, Ordering::Relaxed);
                t.store(0, Ordering::Relaxed);
                match download_to(&url, &archive, &d, &t, &stop) {
                    Ok(()) => {
                        let result = crate::custom::store::install(&custom, &archive, &install);
                        let _ = std::fs::remove_file(&archive);
                        return result;
                    }
                    Err((e, retry)) => {
                        let _ = std::fs::remove_file(&archive);
                        error = format!("taiko.rs: {e}");
                        if !retry {
                            break;
                        }
                    }
                }
            }
            Err(error)
        })();
        let _ = tx.send(result);
    });
    Job {
        set,
        done,
        total,
        rx,
        maps: Vec::new(),
        update,
        cancel,
        old_version: Arc::default(),
    }
}

/// Installs website set `install` from archive `archive` (one opened in the game, which names
/// the set) whole into taiko.rs's own custom maps folder `custom`, on a background thread, like
/// [`import`]. `temporary`: delete the archive whatever happens.
pub fn import_custom(
    archive: PathBuf,
    name: &str,
    custom: PathBuf,
    install: crate::custom::store::Install,
    temporary: bool,
    maps: Vec<u64>,
) -> Job {
    let set = SetInfo {
        id: install.id,
        title: name.to_string(),
        ..SetInfo::default()
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::custom::store::install(&custom, &archive, &install);
        if temporary || result.is_ok() {
            let _ = std::fs::remove_file(&archive);
        }
        let _ = tx.send(result);
    });
    Job {
        set,
        done: Arc::default(),
        total: Arc::default(),
        rx,
        maps,
        update: false,
        cancel: Arc::default(),
        old_version: Arc::default(),
    }
}

/// Where a lobby map we don't have comes from: the website for a taiko.rs custom map
/// (`custom_set` names its set by the map's hash), else the osu! set its ID names, else the
/// mirrors' set with its title and difficulty.
pub(crate) fn lobby_set(
    m: &net::MapInfo,
    custom_set: impl FnOnce(u64) -> Option<u32>,
) -> Result<SetInfo, String> {
    let set = |id, custom| SetInfo {
        id,
        artist: m.artist.clone(),
        title: m.title.clone(),
        custom,
        ..SetInfo::default()
    };
    if let Some(id) = custom_set(m.hash) {
        return Ok(set(id, Some(CustomSet::default())));
    }
    if let Some(id) = m.set_id {
        return Ok(set(id, None));
    }
    search(
        &format!("{} {}", m.artist, m.title),
        Category::Any,
        SortKey::Relevance,
        true,
        0,
    )
    .and_then(|(sets, _)| {
        sets.into_iter()
            .find(|s| {
                s.title.eq_ignore_ascii_case(&m.title)
                    && s.diffs.iter().any(|d| d.version == m.version)
            })
            .ok_or_else(|| "not found on the mirrors".to_string())
    })
}

/// Whether an archive is a taiko.rs custom set's: it has the website's marker
/// (`custom::SIDECAR`) or a `.tko` chart (which only taiko.rs plays).
pub fn is_custom_archive(archive: &Path) -> bool {
    let Some(mut zip) = std::fs::File::open(archive)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
    else {
        return false;
    };
    (0..zip.len()).any(|i| {
        zip.by_index_raw(i).is_ok_and(|e| {
            let name = e
                .name()
                .map(|n| n.replace('\\', "/").to_ascii_lowercase())
                .unwrap_or_default();
            let file = name.rsplit('/').next().unwrap_or(&name).to_string();
            file == crate::custom::SIDECAR || file.ends_with(".tko")
        })
    })
}

/// The manifest (`custom::SIDECAR`) an archive brings, if it has one: the website's archives of
/// its sets do.
pub fn archive_manifest(archive: &Path) -> Option<crate::custom::Manifest> {
    let file = std::fs::File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name(crate::custom::SIDECAR).ok()?;
    let mut text = String::new();
    entry
        .by_ref()
        .take(64 << 10)
        .read_to_string(&mut text)
        .ok()?;
    crate::custom::Manifest::parse(&text)
}

/// Runs `get` (which brings the new files) with `old` set aside, not deleted, until it worked;
/// if it fails they're put back.
fn replacing<T>(old: &[PathBuf], get: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let aside: Vec<(&PathBuf, PathBuf)> = old
        .iter()
        .map(|p| (p, p.with_extension("osu.old")))
        .filter(|(p, o)| std::fs::rename(p, o).is_ok())
        .collect();
    let result = get();
    for (p, o) in &aside {
        if result.is_ok() {
            let _ = std::fs::remove_file(o);
        } else {
            let _ = std::fs::rename(o, p);
        }
    }
    result
}

#[derive(Deserialize)]
struct ApiSetMaps {
    #[serde(default)]
    beatmaps: Vec<ApiChecksum>,
    /// When osu! last changed the set, as the mirror saw it (ISO 8601): which mirror's copy of
    /// the set is the newest.
    #[serde(default)]
    last_updated: Option<String>,
}

#[derive(Deserialize)]
struct ApiChecksum {
    id: u32,
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

/// A difficulty of a set as osu! has it now.
#[derive(Clone)]
pub struct OnlineDiff {
    pub md5: String,
    /// Its [Version] (None: the mirror didn't say).
    pub version: Option<String>,
}

pub type OnlineSet = std::collections::HashMap<u32, OnlineDiff>;

/// Where the mirrors describe osu! set `id` (name, URL of its JSON), in the order they're asked.
fn json_sources(id: u32) -> Vec<(&'static str, String)> {
    MIRRORS
        .iter()
        .map(|m| (m.name, format!("{}{id}", m.set)))
        .collect()
}

/// Each difficulty of set `id` as osu! has it now, by beatmap ID (from the first mirror that
/// answers).
pub fn set_checksums(id: u32) -> Result<OnlineSet, String> {
    crate::offline::internet()?;
    checksums_from(&json_sources(id), false)
}

/// `set_checksums`, from whichever mirror saw the newest version of the set: a mirror's
/// description of a set can be years old (catboy.best still had one set as it was in 2019 after
/// osu! updated and ranked it), so one that says a set here is out of date is checked against
/// the others.
pub fn freshest_checksums(id: u32) -> Result<OnlineSet, String> {
    crate::offline::internet()?;
    checksums_from(&json_sources(id), true)
}

/// A set's difficulties from `sources` (name, JSON URL): the first that answers, or with
/// `freshest` every one asked and the newest kept (`last_updated`; one that doesn't say counts
/// as the oldest).
pub(crate) fn checksums_from(
    sources: &[(&str, String)],
    freshest: bool,
) -> Result<OnlineSet, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut errors = Vec::new();
    let mut best: Option<(Option<String>, OnlineSet)> = None;
    for (name, url) in sources {
        let reply = crate::offline::url(url)
            .and_then(|()| {
                agent
                    .get(url)
                    .header("User-Agent", USER_AGENT)
                    .call()
                    .map_err(|e| e.to_string())
            })
            .and_then(|mut r| r.body_mut().read_to_string().map_err(|e| e.to_string()));
        match reply.and_then(|json| {
            serde_json::from_str::<ApiSetMaps>(&json).map_err(|e| format!("unexpected reply: {e}"))
        }) {
            Ok(set) if !set.beatmaps.is_empty() => {
                let online: OnlineSet = set
                    .beatmaps
                    .into_iter()
                    .filter_map(|b| {
                        Some((
                            b.id,
                            OnlineDiff {
                                md5: b.checksum?.to_ascii_lowercase(),
                                version: b.version,
                            },
                        ))
                    })
                    .collect();
                if !freshest {
                    return Ok(online);
                }
                // ISO 8601 times in UTC sort as text.
                if best.as_ref().is_none_or(|(at, _)| set.last_updated > *at) {
                    best = Some((set.last_updated, online));
                }
            }
            Ok(_) => errors.push(format!("{name}: no such set")),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    best.map(|(_, online)| online)
        .ok_or_else(|| errors.join("; "))
}

/// What a downloaded set is checked against: the MD5s osu! has for its difficulties, asked of
/// the mirrors (`sources`, JSON URLs) only once an archive needs checking.
pub(crate) struct Check {
    sources: Vec<(&'static str, String)>,
    online: Option<OnlineSet>,
    asked: bool,
    freshest: bool,
    /// Where osu!'s own `.osu` files are asked for (see [`OSU_FILES`]).
    files: String,
    /// What was unpacked is an older version of the set than osu! has (see [`fetch`]).
    pub(crate) older: bool,
}

impl Check {
    /// `known`: the set's difficulties if they were asked for already (an update has).
    pub(crate) fn new(sources: Vec<(&'static str, String)>, known: Option<OnlineSet>) -> Self {
        let asked = known.is_some();
        Self {
            sources,
            online: known,
            asked,
            freshest: false,
            files: OSU_FILES.to_string(),
            older: false,
        }
    }

    /// Asks `base` for osu!'s own files instead of osu! (tests).
    #[cfg(test)]
    pub(crate) fn with_osu_files(mut self, base: impl Into<String>) -> Self {
        self.files = base.into();
        self
    }

    /// For osu! set `id`, from the mirrors.
    fn set(id: u32) -> Self {
        Self::new(json_sources(id), None)
    }

    fn online(&mut self) -> Option<&OnlineSet> {
        if !self.asked {
            self.asked = true;
            self.online = checksums_from(&self.sources, false).ok();
        }
        self.online.as_ref()
    }

    /// Asks every mirror and keeps the newest description of the set (once).
    fn freshen(&mut self) {
        if !self.freshest {
            self.freshest = true;
            if let Ok(online) = checksums_from(&self.sources, true) {
                self.online = Some(online);
            }
        }
    }

    /// How many of `md5s` (an archive's `.osu` files) are osu!'s; None when that can't be told.
    fn matching(&mut self, md5s: &[String]) -> Option<usize> {
        let online = self.online()?;
        Some(md5s.iter().filter(|m| is_osus(m, online)).count())
    }

    /// Whether an archive with these `.osu` files is the set as osu! has it now: every file is
    /// one of osu!'s. When the first mirror's description says no, the others are asked too, in
    /// case it's that description that's old. An archive with no `.osu`, or one that can't be
    /// checked (no mirror describes the set), is taken as it is.
    fn current(&mut self, md5s: &[String]) -> bool {
        if md5s.is_empty() || self.matching(md5s).is_none_or(|n| n == md5s.len()) {
            return true;
        }
        self.freshen();
        self.matching(md5s).is_none_or(|n| n == md5s.len())
    }
}

impl Check {
    /// What makes an older archive of the set current (see [`Patch`]): osu!'s own files for the
    /// difficulties it changed or added since, fetched from osu! (see [`osus_files`]).
    fn patch(&mut self, archive: &Path) -> Result<Patch, String> {
        let base = self.files.clone();
        let online = self.online().ok_or("no description of the set")?.clone();
        let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|e| format!("not a beatmap archive ({e})"))?;
        // Every file of the archive (see `file_key`), and each `.osu`: name, key, MD5, header.
        let mut names = std::collections::HashSet::new();
        let mut osus: Vec<(String, String, String, crate::beatmap::Meta)> = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let Some(path) = entry.enclosed_name().filter(|_| entry.is_file()) else {
                continue;
            };
            let key = file_key(&path.to_string_lossy());
            if key.ends_with(".osu") {
                if path.parent().is_some_and(|p| !p.as_os_str().is_empty()) {
                    return Err("difficulties in a subfolder".into());
                }
                let mut bytes = Vec::new();
                entry
                    .by_ref()
                    .take(16 << 20)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                let meta = crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes));
                let name = path.to_string_lossy().into_owned();
                osus.push((name, key.clone(), md5_hex(&bytes), meta));
            }
            names.insert(key);
        }
        let have = osus.iter().map(|(_, _, md5, _)| md5.clone()).collect();
        let fetched = osus_files(&online, &have, &|f| names.contains(f), &base)?;
        let mut patch = Patch::default();
        for (id, bytes) in fetched {
            // The archive's old version of that difficulty makes way for it, under its name.
            // Its [Version] as osu!'s file has it (a mirror's description can word it otherwise).
            let version = crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes)).version;
            let old = osus.iter().find(|(_, key, md5, meta)| {
                !is_osus(md5, &online)
                    && !patch.skip.contains(key)
                    // By ID; one osu! doesn't know (a difficulty uploaded again) by its name.
                    && match meta.beatmap_id.filter(|b| online.contains_key(b)) {
                        Some(b) => b == id,
                        None => same_version(&version, &meta.version),
                    }
            });
            let name = match old {
                Some((name, key, ..)) => {
                    patch.skip.insert(key.clone());
                    name.clone()
                }
                None => {
                    let name = osu_file_name(&bytes);
                    if names.contains(&file_key(&name)) {
                        return Err(format!("{name} is in the archive already"));
                    }
                    name
                }
            };
            patch.files.push((name, bytes));
        }
        Ok(patch)
    }
}

/// An older archive of a set made current: its `.osu` files to leave out (old versions of
/// difficulties osu! has changed since, by [`file_key`]) and osu!'s own files to put in.
#[derive(Default)]
struct Patch {
    skip: std::collections::HashSet<String>,
    files: Vec<(String, Vec<u8>)>,
}

/// Where osu! hands out each difficulty's current `.osu` by beatmap ID, without a login: what
/// osu! itself updates a set with. The mirrors can keep a set as it was for weeks after osu!
/// changed it, and a new player downloading a lot hits the one that doesn't (catboy.best)
/// limiting downloads.
const OSU_FILES: &str = "https://osu.ppy.sh/osu/";

/// With more difficulties to bring than this, the whole archive is the quicker way.
const MAX_OSU_FILES: usize = 24;

/// How the files of a set are compared: maps name theirs in any case and with either slash.
fn file_key(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_lowercase()
}

/// The files a difficulty needs next to it: its music and its background (see [`file_key`]).
fn needed_files(meta: &crate::beatmap::Meta) -> Vec<String> {
    let audio = meta.audio_file.trim();
    let mut out = Vec::new();
    if !audio.is_empty() && !audio.eq_ignore_ascii_case("virtual") {
        out.push(file_key(audio));
    }
    out.extend(
        meta.background
            .as_deref()
            .filter(|b| !b.trim().is_empty())
            .map(file_key),
    );
    out
}

/// Whether two [Version]s name the same difficulty.
fn same_version(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The name osu! gives a difficulty's file: `Artist - Title (Creator) [Version].osu`.
fn osu_file_name(bytes: &[u8]) -> String {
    let m = crate::beatmap::read_meta(&String::from_utf8_lossy(bytes));
    let name = format!("{} - {} ({}) [{}]", m.artist, m.title, m.creator, m.version);
    format!("{}.osu", safe_folder_name(&name))
}

/// osu!'s current files for the difficulties of a set (`online`) that aren't among `have` (the
/// MD5s of the `.osu` files at hand), fetched from osu! itself (`base` + beatmap ID), in beatmap
/// ID order. An error when that can't be done: too many, osu! doesn't answer or answers with
/// another version than the set's description, or a file needs music or a background that
/// `present` (by [`file_key`]) says isn't there.
fn osus_files(
    online: &OnlineSet,
    have: &std::collections::HashSet<String>,
    present: &dyn Fn(&str) -> bool,
    base: &str,
) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let mut wanted: Vec<(u32, &OnlineDiff)> = online
        .iter()
        .filter(|(_, d)| !have.contains(&d.md5))
        .map(|(id, d)| (*id, d))
        .collect();
    if wanted.is_empty() {
        return Err("no difficulty to bring".into());
    }
    if wanted.len() > MAX_OSU_FILES {
        return Err(format!("{} difficulties to bring", wanted.len()));
    }
    wanted.sort_by_key(|(id, _)| *id);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut out = Vec::new();
    for (id, diff) in wanted {
        let url = format!("{base}{id}");
        crate::offline::url(&url)?;
        let bytes = agent
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("difficulty {id}: {e}"))?
            .body_mut()
            .with_config()
            .limit(16 << 20)
            .read_to_vec()
            .map_err(|e| format!("difficulty {id}: {e}"))?;
        // osu! answers a difficulty it doesn't have with an empty file.
        if md5_hex(&bytes) != diff.md5 {
            return Err(format!(
                "difficulty {id}: osu! has another version than the description says"
            ));
        }
        let meta = crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes));
        if let Some(missing) = needed_files(&meta).into_iter().find(|f| !present(f)) {
            return Err(format!("difficulty {id} needs {missing}, which isn't here"));
        }
        out.push((id, bytes));
    }
    Ok(out)
}

/// Every file in a set's folder, by [`file_key`] (relative to it).
fn folder_files(folder: &Path) -> std::collections::HashSet<String> {
    fn walk(dir: &Path, prefix: &str, depth: u32, out: &mut std::collections::HashSet<String>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for e in read.flatten() {
            let name = format!("{prefix}{}", e.file_name().to_string_lossy());
            match e.file_type() {
                Ok(t) if t.is_dir() && depth > 0 => {
                    walk(&e.path(), &format!("{name}/"), depth - 1, out)
                }
                Ok(t) if t.is_file() => {
                    out.insert(file_key(&name));
                }
                _ => {}
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    walk(folder, "", 4, &mut out);
    out
}

/// Brings set folder `folder` up to date the way osu! does, without an archive: osu!'s current
/// files (see [`osus_files`]) for the difficulties it changed or added, when the music and
/// pictures they need are in the folder already. `own`: the set's own difficulties here (see
/// [`is_own`]); an old one makes way for its new version, under its name, and so does the old
/// file of a difficulty osu! has under a new ID now (uploaded again: same name, an ID osu!
/// doesn't know). Home-made difficulties and edited copies stay as they are. An error (why)
/// leaves the folder untouched.
pub(crate) fn update_from_osu(
    folder: &Path,
    own: &[LocalDiff],
    online: &OnlineSet,
    base: &str,
) -> Result<(), String> {
    let present = folder_files(folder);
    let mut have = std::collections::HashSet::new();
    let mut taken = std::collections::HashSet::new();
    // Each `.osu` here that isn't osu!'s file: path, beatmap ID, [Version].
    let mut not_osus: Vec<(PathBuf, Option<u32>, String)> = Vec::new();
    for e in std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = e.path();
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("osu"))
            && let Ok(bytes) = std::fs::read(&path)
        {
            let md5 = md5_hex(&bytes);
            if !is_osus(&md5, online) {
                let meta = crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes));
                not_osus.push((path.clone(), meta.beatmap_id, meta.version));
            }
            have.insert(md5);
            taken.insert(file_key(&e.file_name().to_string_lossy()));
        }
    }
    let fetched = osus_files(online, &have, &|f| present.contains(f), base)?;
    // Where each goes: over the old version of that difficulty, else a file of its own.
    let mut old = Vec::new();
    let mut writes = Vec::new();
    for (id, bytes) in fetched {
        // Its [Version] as osu!'s file has it (a mirror's description can word it otherwise:
        // osu.direct puts "[4K] " before a mania difficulty's).
        let version = crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes)).version;
        let stale = not_osus.iter().find(|(path, file_id, file_version)| {
            !old.contains(path)
                && (own.iter().any(|d| d.path == *path && d.id == Some(id))
                    // An older version of that difficulty: same ID and name. A copy with a rate
                    // baked in keeps the ID under another name, and stays.
                    || same_version(&version, file_version)
                        && file_id.is_some_and(|b| b == id || b > 0 && !online.contains_key(&b)))
        });
        let path = match stale {
            Some((path, ..)) => {
                old.push(path.clone());
                path.clone()
            }
            None => {
                let name = osu_file_name(&bytes);
                if !taken.insert(file_key(&name)) {
                    return Err(format!("{name} is in the folder already"));
                }
                folder.join(name)
            }
        };
        writes.push((path, bytes));
    }
    let written = std::cell::RefCell::new(Vec::new());
    let result = replacing(&old, || {
        for (path, bytes) in &writes {
            std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            written.borrow_mut().push(path.clone());
        }
        Ok(())
    });
    if result.is_err() {
        // The new files that were written before it went wrong (the old ones are back).
        for p in written.borrow().iter().filter(|p| !old.contains(p)) {
            let _ = std::fs::remove_file(p);
        }
    }
    result
}

/// The MD5 of each `.osu` in an archive.
fn archive_md5s(archive: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a beatmap archive ({e})"))?;
    let mut md5s = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file()
            || !entry
                .name()
                .is_ok_and(|n| n.to_ascii_lowercase().ends_with(".osu"))
        {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(16 << 20)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        md5s.push(md5_hex(&bytes));
    }
    Ok(md5s)
}

/// Whether a difficulty here (beatmap ID, [Version], the `.osu`'s MD5) is one of the set's own,
/// as osu! has it now or an older version of it: its file is osu!'s, or its ID is in the set
/// under the same name. A copy edited here (a rate baked in, OD changed) keeps the original's
/// IDs but not its name or file, so it isn't, and an update leaves it alone.
pub fn is_own(id: Option<u32>, version: &str, md5: &str, online: &OnlineSet) -> bool {
    is_osus(md5, online)
        || id
            .and_then(|id| online.get(&id))
            .and_then(|d| d.version.as_deref())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case(version.trim()))
}

/// Whether osu! has a newer version of a set than the difficulties here (beatmap ID, [Version]
/// and the `.osu`'s MD5 each): one of the set's own changed. Home-made difficulties and edited
/// copies (see `is_own`) don't count, or a set with rates of its own would always look old.
pub fn outdated(local: &[(Option<u32>, String, String)], online: &OnlineSet) -> bool {
    local
        .iter()
        .any(|(id, version, md5)| is_own(*id, version, md5, online) && !is_osus(md5, online))
}

/// Whether a `.osu` with this MD5 is one of the set's on osu! (any of its difficulties; an
/// edited copy, OD changed or a rate baked in, isn't).
pub fn is_osus(md5: &str, online: &OnlineSet) -> bool {
    online.values().any(|d| d.md5 == md5)
}

/// MD5 (RFC 1321) of `data` in lowercase hex: how osu! tells versions of a difficulty apart.
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut h: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter()
        .flat_map(|x| x.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Passes over the mirrors before a download gives up: osu.direct often answers 503 for a
/// moment and catboy.best rate-limits (425), and the next try usually works.
const ROUNDS: u64 = 3;

/// Where the mirrors give osu! set `id`.
fn mirror_sources(id: u32) -> Vec<(&'static str, String)> {
    MIRRORS
        .iter()
        .map(|m| (m.name, format!("{}{id}", m.download)))
        .collect()
}

/// Downloads the archive from the first of `sources` (name, URL) that has it and unpacks it
/// into `songs`/`name`. With a `check`, an archive whose difficulties aren't osu!'s current ones
/// (a mirror can keep serving a set as it was before an update: empty placeholder difficulties,
/// old files) is set aside and the next mirror asked; when none has the current version, the
/// archive with the most of osu!'s files is unpacked. Each mirror is asked once for that.
pub(crate) fn fetch(
    sources: &[(&str, String)],
    name: &str,
    songs: &Path,
    done: &AtomicU64,
    total: &AtomicU64,
    cancel: &AtomicBool,
    mut check: Option<&mut Check>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(songs).map_err(|e| format!("{}: {e}", songs.display()))?;
    let dest = songs.join(name);
    let archive = songs.join(format!("{name}.osz.part"));
    let mut errors: Vec<Option<String>> = vec![None; sources.len()];
    // A mirror that doesn't have the set (404) isn't asked again.
    let mut gone = vec![false; sources.len()];
    // Archives of an older version of the set, kept in case no mirror has the current one.
    let mut older: Vec<PathBuf> = Vec::new();
    let forget = |older: &[PathBuf]| {
        for p in older {
            let _ = std::fs::remove_file(p);
        }
    };
    for round in 0..ROUNDS {
        if gone.iter().all(|&g| g) {
            break;
        }
        if round > 0 {
            // osu.direct's 503s ask for up to 30 s.
            let until = std::time::Instant::now() + Duration::from_secs(5 * round);
            while std::time::Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        for (k, (_, url)) in sources.iter().enumerate() {
            if gone[k] {
                continue;
            }
            done.store(0, Ordering::Relaxed);
            total.store(0, Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {
                let _ = std::fs::remove_file(&archive);
                forget(&older);
                return Err(CANCELLED.into());
            }
            match download_to(url, &archive, done, total, cancel) {
                Ok(()) => {
                    let current = match (check.as_deref_mut(), archive_md5s(&archive)) {
                        (Some(check), Ok(md5s)) => check.current(&md5s),
                        // Nothing to check against, or not an archive (unpacking says why).
                        _ => true,
                    };
                    if !current {
                        // The difficulties osu! changed since, from osu! itself: the music and
                        // pictures are the same, and the next mirror is often as old (or
                        // limiting downloads).
                        if let Some(check) = check.as_deref_mut() {
                            match check.patch(&archive).and_then(|patch| {
                                let tmp = partial_of(&dest);
                                let keep = |p: &Path| {
                                    not_runnable(p)
                                        && !(p.parent() == Some(tmp.as_path())
                                            && p.file_name().is_some_and(|n| {
                                                patch.skip.contains(&file_key(&n.to_string_lossy()))
                                            }))
                                };
                                extract_adding(&archive, &dest, &keep, &patch.files)
                            }) {
                                Ok(()) => {
                                    let _ = std::fs::remove_file(&archive);
                                    forget(&older);
                                    return Ok(dest);
                                }
                                Err(e) => eprintln!(
                                    "{name}: {} has an old version, and osu!'s own files didn't do: {e}",
                                    sources[k].0
                                ),
                            }
                        }
                        let kept = songs.join(format!("{name}.osz.{k}"));
                        if std::fs::rename(&archive, &kept).is_ok() {
                            older.push(kept);
                        }
                        (errors[k], gone[k]) = (Some("an older version of the set".into()), true);
                        continue;
                    }
                    let result = extract(&archive, &dest);
                    let _ = std::fs::remove_file(&archive);
                    match result {
                        Ok(()) => {
                            forget(&older);
                            return Ok(dest);
                        }
                        Err(e) => (errors[k], gone[k]) = (Some(e), true),
                    }
                }
                Err((e, retry)) => {
                    let _ = std::fs::remove_file(&archive);
                    (errors[k], gone[k]) = (Some(e), !retry);
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        forget(&older);
        return Err(CANCELLED.into());
    }
    // No mirror has the current version: the closest of the older ones beats none.
    if let Some(check) = check
        && !older.is_empty()
    {
        let best = older
            .iter()
            .enumerate()
            .max_by_key(|(i, p)| {
                let matching = archive_md5s(p)
                    .ok()
                    .and_then(|md5s| check.matching(&md5s))
                    .unwrap_or(0);
                // On a tie the mirror asked first.
                (matching, std::cmp::Reverse(*i))
            })
            .map(|(_, p)| p.clone());
        let result = best
            .map(|p| extract(&p, &dest))
            .unwrap_or_else(|| Err("no archive".into()));
        forget(&older);
        if result.is_ok() {
            eprintln!("{name}: no mirror has the current version, an older one was unpacked");
            check.older = true;
            return Ok(dest);
        }
    }
    forget(&older);
    Err(sources
        .iter()
        .zip(errors)
        .filter_map(|((source, _), e)| Some(format!("{source}: {}", e?)))
        .collect::<Vec<_>>()
        .join("; "))
}

/// Errors say whether trying again later could help (not for a set the mirror doesn't have).
pub(crate) fn download_to(
    url: &str,
    path: &Path,
    done: &AtomicU64,
    total: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), (String, bool)> {
    crate::offline::url(url).map_err(|e| (e, true))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => ("doesn't have this set".to_string(), false),
            e => (e.to_string(), true),
        })?;
    let body = response.body_mut();
    if let Some(len) = body.content_length() {
        total.store(len, Ordering::Relaxed);
    }
    // A mirror that has no such set answers with a small JSON error instead of an archive.
    if body
        .mime_type()
        .is_some_and(|m| m.contains("json") || m.contains("html"))
    {
        let text = body.read_to_string().unwrap_or_default();
        return Err((
            if text.is_empty() {
                "no archive".into()
            } else {
                text.chars().take(120).collect()
            },
            false,
        ));
    }
    let mut reader = body.with_config().limit(512 << 20).reader();
    let mut file = std::fs::File::create(path).map_err(|e| (e.to_string(), false))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err((CANCELLED.into(), false));
        }
        // A dropped transfer is worth another try.
        let n = reader.read(&mut buf).map_err(|e| (e.to_string(), true))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| (e.to_string(), false))?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush().map_err(|e| (e.to_string(), false))?;
    Ok(())
}

/// Raw bytes per transfer message (well under the room's line limit once base64-encoded).
pub const PART_BYTES: usize = 96 * 1024;

/// Packs a map set folder into an `.osz` (stored, not compressed: audio and images already
/// are). Refuses absurdly large folders. Only a set's own kinds of file go in
/// ([`is_set_file`]), and links aren't followed: what a host sends is the map, never other files
/// that happen to be next to it.
pub fn pack_folder(dir: &Path) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    const LIMIT: u64 = 300 << 20;
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let Ok(kind) = e.file_type() else { continue };
            let p = e.path();
            if kind.is_dir() {
                if files.len() < 10_000 {
                    stack.push(p);
                }
            } else if kind.is_file() && is_set_file(&p) {
                files.push(p);
            }
        }
    }
    let size: u64 = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();
    if size > LIMIT {
        return Err(format!("the set is too big to send ({} MB)", size >> 20));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::with_capacity(
        size as usize + 4096,
    )));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for f in files {
        let Ok(rel) = f.strip_prefix(dir) else {
            continue;
        };
        let name = rel.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(&f).map_err(|e| e.to_string())?;
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Whether `folder` is a set folder a lobby host may send: inside one of `roots` (the Songs
/// folders), and not one of them. A map lying loose in a Songs folder, or opened from anywhere
/// else (Downloads, the desktop), would otherwise send everything in the folder it's in.
pub fn is_set_folder(folder: &Path, roots: &[PathBuf]) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).ok();
    let Some(folder) = real(folder) else {
        return false;
    };
    roots
        .iter()
        .filter_map(|r| real(r))
        .any(|r| folder != r && folder.starts_with(&r))
}

/// A file's kind (its extension, lowercase) as Windows reads its name: without trailing dots and
/// spaces, since "x.exe." is saved as x.exe there. None for a name with a `:`, which on Windows
/// writes an NTFS stream ("x.exe:y.png" makes x.exe).
fn kind_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    if name.contains(':') {
        return None;
    }
    let name = name.trim_end_matches(['.', ' ']);
    Some(
        Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default(),
    )
}

/// What a map set is made of: its maps and storyboard, and the music, sounds, images and videos
/// they use, and a taiko.rs custom set's number (`custom::SIDECAR`). A set from a lobby host is
/// sent and unpacked with only these.
pub fn is_set_file(path: &Path) -> bool {
    const KINDS: &[&str] = &[
        "osu", "osb", "tko", "mp3", "ogg", "wav", "flac", "jpg", "jpeg", "png", "gif", "webp",
        "bmp", "mp4", "avi", "flv", "wmv", "mkv", "webm", "m4v", "mov", "mpg", "mpeg",
    ];
    path.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(crate::custom::SIDECAR))
        || kind_of(path).is_some_and(|k| KINDS.contains(&k.as_str()))
}

/// Programs, scripts and shortcuts: no map set or skin needs one, and a double click on it in a
/// file manager would run it. No archive is unpacked with them (nor with Windows's folder
/// settings files, which can point Explorer elsewhere).
fn is_runnable(path: &Path) -> bool {
    const KINDS: &[&str] = &[
        "exe",
        "dll",
        "scr",
        "com",
        "pif",
        "bat",
        "cmd",
        "ps1",
        "psm1",
        "psd1",
        "vbs",
        "vbe",
        "js",
        "jse",
        "wsf",
        "wsh",
        "hta",
        "msi",
        "msp",
        "mst",
        "msc",
        "cpl",
        "lnk",
        "url",
        "scf",
        "inf",
        "reg",
        "jar",
        "appref-ms",
        "application",
        "gadget",
        "sys",
        "drv",
        "ocx",
        "sh",
        "desktop",
        "appimage",
        "command",
        "app",
        "pkg",
        "dmg",
        "deb",
        "rpm",
        "run",
    ];
    let named = path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy().to_ascii_lowercase();
        let n = n.trim_end_matches(['.', ' ']);
        n == "desktop.ini" || n == "autorun.inf"
    });
    named || kind_of(path).is_none_or(|k| KINDS.contains(&k.as_str()))
}

fn not_runnable(path: &Path) -> bool {
    !is_runnable(path)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut lookup = [255u8; 256];
    for (i, &c) in B64.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = lookup[c as usize];
            if v == 255 {
                return None;
            }
            n |= (v as u32) << (18 - 6 * i);
        }
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// A folder name from another machine, made safe to create here.
pub fn safe_folder_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean
        .trim()
        .trim_matches('.')
        .chars()
        .take(120)
        .collect::<String>();
    if clean.is_empty() {
        "map from host".into()
    } else {
        clean
    }
}

/// Receives a map set from the multiplayer host: feed the parts (None aborts); the job
/// reports the unpacked folder like a mirror download.
pub fn receive(
    set: SetInfo,
    name: String,
    total: u64,
    map: u64,
    songs: PathBuf,
    custom: PathBuf,
) -> (Job, Sender<Option<Vec<u8>>>) {
    let (feed, parts) = mpsc::channel::<Option<Vec<u8>>>();
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total_bytes = Arc::new(AtomicU64::new(total));
    let d = done.clone();
    std::thread::spawn(move || {
        let result = (|| {
            std::fs::create_dir_all(&songs).map_err(|e| e.to_string())?;
            let name = safe_folder_name(&name);
            let archive = songs.join(format!("{name}.osz.part"));
            let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
            let mut got = 0u64;
            while got < total {
                match parts.recv() {
                    Ok(Some(bytes)) => {
                        file.write_all(&bytes).map_err(|e| e.to_string())?;
                        got += bytes.len() as u64;
                        d.store(got, Ordering::Relaxed);
                    }
                    _ => {
                        drop(file);
                        let _ = std::fs::remove_file(&archive);
                        return Err("the transfer was interrupted".to_string());
                    }
                }
            }
            drop(file);
            let result = holds_map(&archive, map).and_then(|_| {
                // In a folder of its own: what the host sent doesn't touch what's there. Only a
                // set's kinds of file: the host is just another player. A taiko.rs custom set goes
                // into taiko.rs's own folder, an osu! set into osu!'s Songs folder.
                let own = is_custom_archive(&archive);
                let root = if own { &custom } else { &songs };
                std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
                let dest = free_folder(root, &name);
                extract_keeping(&archive, &dest, &is_set_file)?;
                if own {
                    crate::custom::mark_origin(&dest, crate::custom::Origin::Lobby);
                }
                Ok(dest)
            });
            let _ = std::fs::remove_file(&archive);
            result
        })();
        let _ = tx.send(result);
    });
    (
        Job {
            set,
            done,
            total: total_bytes,
            rx,
            maps: Vec::new(),
            update: false,
            cancel: Arc::default(),
            old_version: Arc::default(),
        },
        feed,
    )
}

/// Imports an `.osz` that is already on disk (dropped on the window, opened with the game on
/// Android) like a download: unpacked into `songs/<name>/` on a background thread.
/// The archive is deleted once unpacked; if that fails it stays, unless it's `temporary` (a copy,
/// not the player's own file).
/// `maps`: the content hashes of its difficulties ([`archive_maps`]).
pub fn import(
    archive: PathBuf,
    name: &str,
    songs: PathBuf,
    temporary: bool,
    maps: Vec<u64>,
    origin: Option<crate::custom::Origin>,
) -> Job {
    let name = safe_folder_name(name);
    let set = SetInfo {
        // osu! names its exports "<set id> <artist> - <title>.osz".
        id: name
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0),
        artist: String::new(),
        title: name.clone(),
        creator: String::new(),
        status: String::new(),
        diffs: Vec::new(),
        play_count: 0,
        favourite_count: 0,
        rating: 0.0,
        ranked_date: String::new(),
        custom: None,
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::fs::create_dir_all(&songs)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                let dest = songs.join(&name);
                extract(&archive, &dest)?;
                if let Some(origin) = origin {
                    crate::custom::mark_origin(&dest, origin);
                }
                Ok(dest)
            });
        if temporary || result.is_ok() {
            let _ = std::fs::remove_file(&archive);
        }
        let _ = tx.send(result);
    });
    Job {
        set,
        done: Arc::default(),
        total: Arc::default(),
        rx,
        maps,
        update: false,
        cancel: Arc::default(),
        old_version: Arc::default(),
    }
}

/// The difficulties in an `.osz`: each `.osu`'s header and content hash (the same hash as
/// `Entry::hash`), to tell whether the set is already in the list before unpacking it.
/// Whether the archive has the map with this content hash.
fn holds_map(archive: &Path, map: u64) -> Result<(), String> {
    if archive_maps(archive)?.iter().any(|(_, hash)| *hash == map) {
        Ok(())
    } else {
        Err("that isn't the lobby's map".into())
    }
}

/// `name` in `songs`, or with a number if that's taken.
fn free_folder(songs: &Path, name: &str) -> PathBuf {
    let dest = songs.join(name);
    if !dest.exists() {
        return dest;
    }
    (2..)
        .map(|n| songs.join(format!("{name} ({n})")))
        .find(|d| !d.exists())
        .unwrap_or(dest)
}

/// Where `dest` is unpacked to first. Added to the name, not replacing its end: a set called
/// "A.B" shouldn't share this with "A.C".
pub fn partial_of(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    dest.with_file_name(name)
}

/// Biggest archive taken from a lobby host: more than a host sends (`pack_folder`).
pub const MAX_HOST_MAP: u64 = 512 << 20;

/// Most an archive may unpack to, and the most files in it: sets and skins are far under
/// these, and a few bytes of zip can claim to be (or turn out to be) any size.
const MAX_UNPACKED: u64 = 2 << 30;
const MAX_ENTRIES: usize = 20_000;

/// Unpacks a zip into `to`, which is gone again if it fails. Entries that would land outside
/// `to` fail it (zip-slip), links aren't made, and programs and shortcuts are left out
/// ([`is_runnable`]).
pub fn unpack(archive: &Path, to: &Path) -> Result<(), String> {
    unpack_keeping(archive, to, &not_runnable)
}

/// [`unpack`] with only the files `keep` takes.
fn unpack_keeping(archive: &Path, to: &Path, keep: &dyn Fn(&Path) -> bool) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(to);
    let result = unpack_within(archive, to, MAX_UNPACKED, keep);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(to);
    }
    result
}

fn unpack_within(
    archive: &Path,
    to: &Path,
    limit: u64,
    keep: &dyn Fn(&Path) -> bool,
) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a beatmap archive ({e})"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("too many files in the archive".into());
    }
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut left = limit;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_symlink() {
            continue;
        }
        let path = entry
            .enclosed_name()
            .map(|p| to.join(p))
            .ok_or("the archive has a bad file path")?;
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if !keep(&path) {
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        // One byte past what's left tells a file that fits exactly from one that doesn't.
        let written = std::io::copy(&mut entry.by_ref().take(left + 1), &mut out)
            .map_err(|e| e.to_string())?;
        if written > left {
            return Err("the archive unpacks to too much".into());
        }
        left -= written;
    }
    Ok(())
}

pub fn archive_maps(archive: &Path) -> Result<Vec<(crate::beatmap::Meta, u64)>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a beatmap archive ({e})"))?;
    let mut maps = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if !entry.is_file()
            || !entry.name().is_ok_and(|n| {
                let n = n.to_ascii_lowercase();
                // `.tko` is a map with the client's effects (custom maps made in the editor).
                n.ends_with(".osu") || n.ends_with(".tko")
            })
        {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(16 << 20)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        maps.push((
            crate::beatmap::read_meta(&String::from_utf8_lossy(&bytes)),
            crate::net::map_hash(&bytes),
        ));
    }
    Ok(maps)
}

/// Unpacks an `.osz` into `dest` (via a temporary folder, so a half-extracted set never shows up).
/// A set that is already there gets the archive's files on top: what else is in its folder (a
/// difficulty of your own) stays.
fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    extract_keeping(archive, dest, &not_runnable)
}

/// [`extract`] with only the files `keep` takes.
fn extract_keeping(
    archive: &Path,
    dest: &Path,
    keep: &dyn Fn(&Path) -> bool,
) -> Result<(), String> {
    extract_adding(archive, dest, keep, &[])
}

/// [`extract_keeping`], with files of its own (`add`: name, contents) put in the set's folder
/// next to the archive's.
fn extract_adding(
    archive: &Path,
    dest: &Path,
    keep: &dyn Fn(&Path) -> bool,
    add: &[(String, Vec<u8>)],
) -> Result<(), String> {
    let tmp = partial_of(dest);
    unpack_keeping(archive, &tmp, keep)?;
    for (name, bytes) in add {
        if let Err(e) = std::fs::write(tmp.join(name), bytes) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("{name}: {e}"));
        }
    }
    if dest.is_dir() {
        if tmp.join(crate::custom::SIDECAR).is_file() {
            set_aside_older_charts(&tmp, dest);
        }
        merge(&tmp, dest).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The suffix a chart put aside by [`set_aside_older_charts`] gets: song select no longer
/// lists it, and renaming it back brings it back.
pub const SET_ASIDE: &str = "replaced";

/// The website's archive of a custom set (unpacked in `new`) is the whole set: the charts in
/// its folder `folder` the archive doesn't have are an older version's (a difficulty renamed
/// since, or dropped), and would be listed next to the new ones, wearing the set's marks. They
/// are renamed to `<file>.replaced`, not deleted: a mapper's unsent difficulty can come back.
fn set_aside_older_charts(new: &Path, folder: &Path) {
    let Ok(dir) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let chart = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("tko"));
        if !chart || !path.is_file() || new.join(entry.file_name()).exists() {
            continue;
        }
        let mut aside = path.clone().into_os_string();
        aside.push(".");
        aside.push(SET_ASIDE);
        let aside = PathBuf::from(aside);
        let _ = std::fs::remove_file(&aside);
        if let Err(e) = std::fs::rename(&path, &aside) {
            eprintln!("{}: not put aside ({e})", path.display());
        }
    }
}

/// Moves everything in `from` into `into`, replacing files of the same name.
fn merge(from: &Path, into: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let to = into.join(entry.file_name());
        if entry.file_type()?.is_dir() && to.is_dir() {
            merge(&entry.path(), &to)?;
        } else {
            if to.is_dir() {
                std::fs::remove_dir_all(&to)?;
            }
            std::fs::rename(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Fetches what a preview needs without downloading the set: the difficulty's `.osu`, the
/// set's ~10 s music clip (from its preview point) and its cover, cached per set. Returns the
/// `.osu` path; the clip and cover sit next to it.
pub fn fetch_preview(set: u32, beatmap: u32) -> Result<PathBuf, String> {
    crate::offline::internet()?;
    let dir = crate::config::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("taiko-rs"))
        .join("previews")
        .join(set.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Short timeout: a throttled mirror shouldn't hold the preview up.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let get = |url: &str| -> Result<(Vec<u8>, String), String> {
        let mut r = agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| e.to_string())?;
        let mime = r.body().mime_type().unwrap_or("").to_string();
        let bytes = r
            .body_mut()
            .with_config()
            .limit(32 << 20)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok((bytes, mime))
    };
    let osu = dir.join(format!("{beatmap}.osu"));
    if !osu.is_file() {
        // osu! itself serves any difficulty's .osu without a login; the mirrors are the fallback
        // (catboy.best answers 425 once it rate-limits us, which browsing quickly did).
        let sources = [
            ("osu.ppy.sh", format!("https://osu.ppy.sh/osu/{beatmap}")),
            (
                "osu.direct",
                format!("https://osu.direct/api/osu/{beatmap}"),
            ),
            ("catboy.best", format!("https://catboy.best/osu/{beatmap}")),
        ];
        let mut errors = Vec::new();
        let mut saved = false;
        for (name, url) in &sources {
            match get(url) {
                Ok((bytes, _))
                    if bytes.starts_with(b"osu file format")
                        || bytes.windows(15).take(64).any(|w| w == b"osu file format") =>
                {
                    std::fs::write(&osu, bytes).map_err(|e| e.to_string())?;
                    saved = true;
                    break;
                }
                Ok(_) => errors.push(format!("{name}: not a beatmap")),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if !saved {
            return Err(format!("difficulty: {}", errors.join("; ")));
        }
    }
    let have_clip = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(CLIP_PREFIX))
        })
        .unwrap_or(false);
    if !have_clip {
        let (bytes, mime) = get(&format!("https://b.ppy.sh/preview/{set}.mp3"))
            .map_err(|e| format!("music: {e}"))?;
        let ext = if mime.contains("ogg") || bytes.starts_with(b"OggS") {
            "ogg"
        } else {
            "mp3"
        };
        std::fs::write(dir.join(format!("{CLIP_PREFIX}.{ext}")), bytes)
            .map_err(|e| e.to_string())?;
    }
    let cover = dir.join("cover.jpg");
    if !cover.is_file() {
        // Optional: the preview works without a background.
        if let Ok((bytes, _)) = get(&format!(
            "https://assets.ppy.sh/beatmaps/{set}/covers/cover.jpg"
        )) {
            let _ = std::fs::write(&cover, bytes);
        }
    }
    Ok(osu)
}

/// Where the download tab keeps its result covers (`pictures.rs` reads them from there).
pub fn covers_dir() -> PathBuf {
    crate::config::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("taiko-rs"))
        .join("covers")
}

/// A set's cover file in [`covers_dir`]: "o<id>.img" for osu!'s sets, "c<id>-v<n>.img" for the
/// website's custom ones (a new upload may bring a new background).
pub fn cover_name(set: &SetInfo) -> String {
    match &set.custom {
        Some(c) => format!("c{}-v{}.img", set.id, c.version),
        None => format!("o{}.img", set.id),
    }
}

/// Fetches a set's cover into [`covers_dir`] (osu!'s square list cover, or a custom set's
/// background from the website). Err when it has none or it couldn't be fetched.
pub fn fetch_cover(set: &SetInfo) -> Result<(), String> {
    crate::offline::internet()?;
    let url = match &set.custom {
        Some(c) if c.background => format!("{}/c/{}/bg", crate::online::WEBSITE, set.id),
        Some(_) => return Err("no background".into()),
        None => format!(
            "https://assets.ppy.sh/beatmaps/{}/covers/list@2x.jpg",
            set.id
        ),
    };
    let dir = covers_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let bytes = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(8 << 20)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let file = dir.join(cover_name(set));
    let part = file.with_extension("part");
    std::fs::write(&part, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&part, &file).map_err(|e| e.to_string())
}

/// The clip file in a preview folder.
pub fn clip_in(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with(CLIP_PREFIX))
}

/// The beatmap set ID a library folder or map belongs to: "123456 Artist - Title" folders, as
/// osu! names them.
pub fn set_id_of_folder(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty() && name[digits.len()..].starts_with(' '))
        .then(|| digits.parse().ok())
        .flatten()
}

/// How far along the download of the set `m` is from is, if there's one (None inside: it's going,
/// its size not known yet).
pub(crate) fn map_download(downloads: &[Job], m: &net::MapInfo) -> Option<Option<f32>> {
    downloads
        .iter()
        .find(|j| {
            j.set.is_osu_set(m.set_id) || (j.set.title == m.title && j.set.artist == m.artist)
        })
        .map(Job::fraction)
}

pub(crate) fn download_text(done: Option<f32>) -> String {
    match done {
        Some(f) => {
            let percent = f * 100.0;
            tr!("downloading {percent:.0}%", "ダウンロード中 {percent:.0}%", "скачивание {percent:.0}%"; [] [percent])
        }
        None => tr!("downloading...", "ダウンロード中...", "скачивание...").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip of (name, bytes) at a temporary path.
    fn a_zip(tag: &str, entries: &[(&str, &[u8])]) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("taiko-unpack-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("a.osz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zip.start_file(*name, stored).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        (dir, archive)
    }

    #[test]
    fn a_custom_sets_archive_is_told_by_its_marker_or_a_tko() {
        let (dir, osu) = a_zip(
            "kind-osu",
            &[("a [Oni].osu", b"osu"), ("audio.mp3", b"mp3")],
        );
        assert!(!is_custom_archive(&osu));
        assert_eq!(archive_manifest(&osu), None);
        let (_, tko) = a_zip("kind-tko", &[("a [Oni].TKO", b"tko")]);
        assert!(is_custom_archive(&tko));
        let (_, marked) = a_zip(
            "kind-marked",
            &[
                ("a [Oni].osu", b"osu"),
                (crate::custom::SIDECAR, b"{\"id\": 12, \"version\": 3}"),
            ],
        );
        assert!(is_custom_archive(&marked));
        let m = archive_manifest(&marked).unwrap();
        assert_eq!((m.id, m.version), (Some(12), Some(3)));
        std::fs::write(dir.join("not.zip"), "x").unwrap();
        assert!(!is_custom_archive(&dir.join("not.zip")));
    }

    #[test]
    fn a_custom_lobby_map_comes_from_the_website_not_the_osu_set_its_id_names() {
        let m = net::MapInfo {
            hash: 0x4afe_7841_b490_d3f8,
            artist: "KNOWER".into(),
            title: "Time Traveler".into(),
            version: "Inner Oni".into(),
            rate: 1.0,
            from: None,
            // A custom map made from osu! set 622136 keeps its ID.
            set_id: Some(622_136),
            mods: Vec::new(),
            multiverse: None,
            shapeshift: None,
            seed: None,
            defense: None,
            stats: None,
        };
        let site = lobby_set(&m, |h| (h == m.hash).then_some(12)).unwrap();
        assert_eq!(site.id, 12);
        assert!(site.custom.is_some(), "the website's set 12");
        // A map the website doesn't know: the osu! set, as before.
        let osu = lobby_set(&m, |_| None).unwrap();
        assert_eq!(osu.id, 622_136);
        assert!(osu.custom.is_none());
    }

    #[test]
    fn unpacking_stops_at_the_limit_and_leaves_nothing() {
        let (dir, archive) = a_zip(
            "limit",
            &[("a/map.osu", &[b'x'; 600]), ("b.bin", &[b'y'; 600])],
        );
        let to = dir.join("set");
        assert!(unpack_within(&archive, &to, 1200, &not_runnable).is_ok());
        assert_eq!(std::fs::read(to.join("a/map.osu")).unwrap().len(), 600);
        assert!(unpack_within(&archive, &dir.join("over"), 1199, &not_runnable).is_err());
        assert!(unpack(&archive, &to).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_archive_reaching_outside_fails_and_is_cleaned_up() {
        let (dir, archive) = a_zip("slip", &[("ok.txt", b"fine"), ("../escape.txt", b"no")]);
        let to = dir.join("set");
        assert!(unpack(&archive, &to).is_err());
        assert!(!to.exists() && !dir.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_the_host_sends_leaves_existing_sets_alone() {
        let (dir, archive) = a_zip(
            "host",
            &[("map [Oni].osu", b"the lobby map"), ("run-me.bat", b"x")],
        );
        let songs = dir.join("Songs");
        let theirs = songs.join("999 Someone - Set");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("map [Oni].osu"), b"mine").unwrap();
        let map = crate::net::map_hash(b"the lobby map");
        assert!(holds_map(&archive, map).is_ok());
        assert!(holds_map(&archive, map + 1).is_err());
        let dest = free_folder(&songs, "999 Someone - Set");
        extract(&archive, &dest).unwrap();
        assert_eq!(dest, songs.join("999 Someone - Set (2)"));
        assert_eq!(
            std::fs::read(theirs.join("map [Oni].osu")).unwrap(),
            b"mine"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_archive_unpacks_programs_or_shortcuts() {
        let (dir, archive) = a_zip(
            "runnable",
            &[
                ("map [Oni].osu", b"map"),
                ("bg.jpg", b"bg"),
                ("skin.ini", b"ini"),
                ("sb/notes.txt", b"txt"),
                ("setup.exe", b"MZ"),
                ("sb/run.BAT", b"x"),
                ("open me.lnk", b"x"),
                ("trailing.exe.", b"x"),
                ("trailing.exe ", b"x"),
                ("desktop.ini", b"x"),
            ],
        );
        let to = dir.join("set");
        unpack(&archive, &to).unwrap();
        let mut left: Vec<String> = Vec::new();
        let mut stack = vec![to.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    let rel = e.path().strip_prefix(&to).unwrap().to_path_buf();
                    left.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        left.sort();
        assert_eq!(
            left,
            ["bg.jpg", "map [Oni].osu", "sb/notes.txt", "skin.ini"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_set_from_the_host_keeps_only_map_files() {
        let (dir, archive) = a_zip(
            "host-kinds",
            &[
                ("map [Oni].osu", b"the lobby map"),
                ("audio.MP3", b"mp3"),
                ("sb/star.png", b"png"),
                (crate::custom::SIDECAR, b"{\"id\": 7}"),
                ("notes.txt", b"txt"),
                ("payload.dat", b"x"),
                ("x.exe:y.png", b"x"),
            ],
        );
        let dest = dir.join("Songs").join("set");
        extract_keeping(&archive, &dest, &is_set_file).unwrap();
        assert!(dest.join("map [Oni].osu").is_file());
        assert!(dest.join("audio.MP3").is_file());
        assert!(dest.join("sb/star.png").is_file());
        // A custom set stays one on the player's side.
        assert!(dest.join(crate::custom::SIDECAR).is_file());
        assert!(!dest.join("notes.txt").exists());
        assert!(!dest.join("payload.dat").exists());
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_packs_only_the_set_and_only_from_a_set_folder() {
        let dir = std::env::temp_dir().join(format!("taiko-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let songs = dir.join("Songs");
        let set = songs.join("1 A - T");
        std::fs::create_dir_all(set.join("sb")).unwrap();
        std::fs::write(set.join("map [Oni].osu"), b"map").unwrap();
        std::fs::write(set.join("sb").join("star.png"), b"png").unwrap();
        std::fs::write(set.join("passwords.txt"), b"secret").unwrap();
        std::fs::write(set.join("tool.exe"), b"MZ").unwrap();
        let bytes = pack_folder(&set).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["map [Oni].osu", "sb/star.png"]);

        // A loose map: its folder is the Songs folder itself, or somewhere else entirely.
        let downloads = dir.join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let roots = [songs.clone()];
        assert!(is_set_folder(&set, &roots));
        assert!(is_set_folder(&set.join("sb"), &roots));
        assert!(!is_set_folder(&songs, &roots));
        assert!(!is_set_folder(&downloads, &roots));
        assert!(!is_set_folder(&dir, &roots));
        assert!(!is_set_folder(&songs.join("..").join("Downloads"), &roots));
        assert!(!is_set_folder(&songs.join("gone"), &roots));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_download_stops_before_asking_anyone_and_says_so() {
        let songs = std::env::temp_dir().join(format!("taiko-rs-cancel-{}", std::process::id()));
        let (done, total) = Default::default();
        let cancel = AtomicBool::new(true);
        // Nothing listens there: asking would fail with another error, not "cancelled".
        let sources = [("nowhere", "http://127.0.0.1:9/set".to_string())];
        let started = std::time::Instant::now();
        let got = fetch(&sources, "1 A - T", &songs, &done, &total, &cancel, None);
        assert_eq!(got, Err(CANCELLED.to_string()));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!songs.join("1 A - T").exists());
        let _ = std::fs::remove_dir_all(&songs);
    }

    #[test]
    fn dotted_names_keep_their_own_partial_folder() {
        assert_ne!(
            partial_of(Path::new("Songs/A.B")),
            partial_of(Path::new("Songs/A.C"))
        );
    }

    #[test]
    fn parses_search_results_and_keeps_taiko_difficulties() {
        let json = r#"[{"id": 7, "artist": "A", "title": "T: x?", "creator": "c", "status": "ranked", "beatmaps": [
            {"version": "Oni", "mode": "taiko", "mode_int": 1, "difficulty_rating": 4.5},
            {"version": "Kantan", "mode": "taiko", "mode_int": 1, "difficulty_rating": 1.2},
            {"version": "Insane", "mode": "osu", "mode_int": 0, "difficulty_rating": 5.0}]},
            {"id": 8, "beatmaps": [{"version": "4K", "mode": "mania", "mode_int": 3, "difficulty_rating": 2.0}]}]"#;
        let (sets, count) = parse_sets(json).unwrap();
        assert_eq!(count, 2, "the reply's own count, for paging");
        assert_eq!(sets.len(), 1, "sets without taiko difficulties are dropped");
        let names: Vec<(&str, f32)> = sets[0]
            .diffs
            .iter()
            .map(|d| (d.version.as_str(), d.stars))
            .collect();
        assert_eq!(names, vec![("Kantan", 1.2), ("Oni", 4.5)]);
        assert_eq!(sets[0].folder_name(), "7 A - T_ x_");
        assert_eq!(set_id_of_folder(Path::new("/s/7 A - T_ x_")), Some(7));
        assert_eq!(set_id_of_folder(Path::new("/s/Some folder")), None);
    }

    #[test]
    fn md5_matches_the_rfc_examples() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    fn online_set() -> OnlineSet {
        let diff = |md5: &str, version: &str| OnlineDiff {
            md5: md5.into(),
            version: Some(version.into()),
        };
        [(1, diff("aa", "Oni")), (2, diff("bb", "Inner Oni"))].into()
    }

    #[test]
    fn a_set_is_outdated_when_one_of_its_own_difficulties_changed() {
        let online = online_set();
        let here = |v: &[(Option<u32>, &str, &str)]| {
            v.iter()
                .map(|(id, version, m)| (*id, version.to_string(), m.to_string()))
                .collect::<Vec<_>>()
        };
        assert!(!outdated(
            &here(&[
                (Some(1), "Oni", "aa"),
                (Some(2), "Inner Oni", "bb"),
                (None, "mine", "home-made")
            ]),
            &online
        ));
        // A new difficulty online alone doesn't make the ones here outdated.
        assert!(!outdated(&here(&[(Some(1), "Oni", "aa")]), &online));
        assert!(outdated(
            &here(&[(Some(1), "Oni", "aa"), (Some(2), "inner oni", "old")]),
            &online
        ));
        // Copies with a rate baked in keep the original's IDs: they aren't a newer version.
        assert!(!outdated(
            &here(&[
                (Some(1), "Oni", "aa"),
                (Some(1), "Oni 1.2x", "rated"),
                (Some(2), "Inner Oni (OD8)", "od")
            ]),
            &online
        ));
        // Nor is a difficulty osu! no longer has (or never had) under that ID.
        assert!(!outdated(&here(&[(Some(3), "Oni", "cc")]), &online));
        // A mirror that doesn't name the difficulties: only files osu! has are the set's own.
        let unnamed: OnlineSet = [(
            1,
            OnlineDiff {
                md5: "aa".into(),
                version: None,
            },
        )]
        .into();
        assert!(!outdated(&here(&[(Some(1), "Oni", "old")]), &unnamed));
        assert!(is_own(Some(9), "anything", "aa", &unnamed));
    }

    /// The bug a player hit: updating a set took the copies of its difficulties with rates baked
    /// in (they carry the original's IDs) along with the old difficulties.
    #[test]
    fn an_update_leaves_edited_copies_and_home_made_difficulties_alone() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-update-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let online = online_set();
        let files = [
            ("A - T [Oni].osu", Some(1), "Oni", "old version"),
            ("A - T [Oni 1.2x].osu", Some(1), "Oni 1.2x", "rated"),
            (
                "A - T [Inner Oni (OD8)].osu",
                Some(2),
                "Inner Oni (OD8)",
                "od changed",
            ),
            ("A - T [mine].osu", None, "mine", "home-made"),
        ];
        let mut here = Vec::new();
        for (name, id, version, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
            here.push(LocalDiff {
                id,
                version: version.into(),
                path: dir.join(name),
            });
        }
        let replaced: Vec<PathBuf> = here
            .iter()
            .filter(|m| {
                is_own(
                    m.id,
                    &m.version,
                    &md5_hex(&std::fs::read(&m.path).unwrap()),
                    &online,
                )
            })
            .map(|m| m.path.clone())
            .collect();
        assert_eq!(replaced, [dir.join("A - T [Oni].osu")]);
        replacing(&replaced, || {
            std::fs::write(dir.join("A - T [Oni].osu"), "new").map_err(|e| e.to_string())
        })
        .unwrap();
        for (name, _, _, text) in &files[1..] {
            assert_eq!(
                std::fs::read_to_string(dir.join(name)).unwrap(),
                *text,
                "{name}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("A - T [Oni].osu")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_update_replaces_the_old_difficulties_or_puts_them_back_if_it_fails() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (kept, renamed) = (
            dir.join("A - T [Oni].osu"),
            dir.join("A - T [Muzukashii].osu"),
        );
        let own = dir.join("A - T [mine].osu");
        for p in [&kept, &renamed, &own] {
            std::fs::write(p, "old").unwrap();
        }
        let old = [kept.clone(), renamed.clone()];
        assert!(replacing(&old, || Err::<(), _>("mirror down".into())).is_err());
        for p in [&kept, &renamed, &own] {
            assert_eq!(std::fs::read_to_string(p).unwrap(), "old");
        }
        // The new version keeps one difficulty's name and drops the other's.
        replacing(&old, || {
            std::fs::write(&kept, "new").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "new");
        assert!(!renamed.exists());
        assert_eq!(std::fs::read_to_string(&own).unwrap(), "old");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            2,
            "nothing set aside is left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_round_trips() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"taiko"), "dGFpa28=");
        assert_eq!(safe_folder_name("../../etc"), "_.._etc");
    }

    #[test]
    fn importing_a_set_again_keeps_what_else_is_in_its_folder() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("taiko-rs-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("set.osz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("map [Oni].osu", stored).unwrap();
        zip.write_all(b"new").unwrap();
        zip.finish().unwrap();
        let dest = root.join("123 Artist - Title");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("map [Oni].osu"), b"old").unwrap();
        std::fs::write(dest.join("map [My Inner Oni].osu"), b"mine").unwrap();
        extract(&archive, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("map [Oni].osu")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(dest.join("map [My Inner Oni].osu")).unwrap(),
            b"mine"
        );
        assert!(!dest.with_extension("partial").exists());
        let maps = archive_maps(&archive).unwrap();
        assert_eq!(
            maps.iter().map(|m| m.1).collect::<Vec<_>>(),
            [crate::net::map_hash(b"new")]
        );
        // A custom map's `.tko` counts as a map too.
        let tko = root.join("custom.osz");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&tko).unwrap());
            zip.start_file("a [Oni].tko", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, b"tko map").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(archive_maps(&tko).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_imported_archive_is_deleted_only_once_it_is_unpacked() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("taiko-rs-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let songs = root.join("Songs");

        let good = root.join("good.osz");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&good).unwrap());
        zip.start_file("map [Oni].osu", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"map").unwrap();
        zip.finish().unwrap();
        let unpacked = import(good.clone(), "good", songs.clone(), false, Vec::new(), None)
            .rx
            .recv()
            .unwrap()
            .unwrap();
        assert!(unpacked.join("map [Oni].osu").is_file());
        assert!(!good.exists());

        let broken = root.join("broken.osz");
        std::fs::write(&broken, b"not a zip").unwrap();
        assert!(
            import(
                broken.clone(),
                "broken",
                songs.clone(),
                false,
                Vec::new(),
                None
            )
            .rx
            .recv()
            .unwrap()
            .is_err()
        );
        assert!(broken.exists());
        // A temporary copy doesn't stay around either way.
        assert!(
            import(broken.clone(), "broken", songs, true, Vec::new(), None)
                .rx
                .recv()
                .unwrap()
                .is_err()
        );
        assert!(!broken.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_loved_here_search_matches_every_word_anywhere_or_the_set_id() {
        let set = SetInfo {
            id: 1098825,
            artist: "Camellia".into(),
            title: "Exit This Earth's Atomosphere".into(),
            creator: "Mapper".into(),
            diffs: vec![Diff {
                id: 1,
                version: "Inner Oni".into(),
                stars: 6.0,
            }],
            ..SetInfo::default()
        };
        assert!(matches_query(&set, ""));
        assert!(matches_query(&set, "camellia earth"));
        assert!(matches_query(&set, "INNER oni"));
        assert!(matches_query(&set, "1098825"));
        assert!(!matches_query(&set, "camellia kantan"));
        assert!(!matches_query(&set, "109882"));
    }

    #[test]
    fn nothing_loved_here_asks_no_mirror() {
        assert_eq!(
            loved_here(&[], "", SortKey::Relevance, true, &|_, _, _| {}).map(|s| s.len()),
            Ok(0)
        );
    }
}
