//! Beatmap downloads from public osu! mirrors (osu.direct, then catboy.best): searching for
//! sets and fetching `.osz` archives into the Songs folder, on background threads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde::Deserialize;

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
    pub const OURS: [Category; 3] = [
        Category::CustomApproved,
        Category::LovedHere,
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
            Category::Any | Category::CustomApproved | Category::Custom | Category::LovedHere => {
                &[]
            }
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

/// A custom set on the taiko.rs website (`server/src/custom.rs`).
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
    std::thread::spawn(move || {
        let result = match cached.flatten() {
            Some(archive) => std::fs::create_dir_all(&songs)
                .map_err(|e| format!("{}: {e}", songs.display()))
                .and_then(|()| {
                    let dest = songs.join(&name);
                    extract(&archive, &dest).map(|()| dest)
                }),
            None => fetch(&sources, &name, &songs, &d, &t, &stop),
        };
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
            let replaced: Vec<PathBuf> = here
                .into_iter()
                .filter(|m| {
                    std::fs::read(&m.path)
                        .is_ok_and(|bytes| is_own(m.id, &m.version, &md5_hex(&bytes), &online))
                })
                .map(|m| m.path)
                .collect();
            replacing(&replaced, || {
                fetch(&mirror_sources(id), &name, songs, &d, &t, &stop)
            })
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
    }
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
pub struct OnlineDiff {
    pub md5: String,
    /// Its [Version] (None: the mirror didn't say).
    pub version: Option<String>,
}

pub type OnlineSet = std::collections::HashMap<u32, OnlineDiff>;

/// Each difficulty of set `id` as osu! has it now, by beatmap ID (from the first mirror that
/// answers).
pub fn set_checksums(id: u32) -> Result<OnlineSet, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut errors = Vec::new();
    for m in &MIRRORS {
        let reply = agent
            .get(&format!("{}{id}", m.set))
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| e.to_string())
            .and_then(|mut r| r.body_mut().read_to_string().map_err(|e| e.to_string()));
        match reply.and_then(|json| {
            serde_json::from_str::<ApiSetMaps>(&json).map_err(|e| format!("unexpected reply: {e}"))
        }) {
            Ok(set) if !set.beatmaps.is_empty() => {
                return Ok(set
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
                    .collect());
            }
            Ok(_) => errors.push(format!("{}: no such set", m.name)),
            Err(e) => errors.push(format!("{}: {e}", m.name)),
        }
    }
    Err(errors.join("; "))
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
/// into `songs`/`name`.
fn fetch(
    sources: &[(&str, String)],
    name: &str,
    songs: &Path,
    done: &AtomicU64,
    total: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(songs).map_err(|e| format!("{}: {e}", songs.display()))?;
    let dest = songs.join(name);
    let archive = songs.join(format!("{name}.osz.part"));
    let mut errors: Vec<Option<String>> = vec![None; sources.len()];
    // A mirror that doesn't have the set (404) isn't asked again.
    let mut gone = vec![false; sources.len()];
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
                return Err(CANCELLED.into());
            }
            match download_to(url, &archive, done, total, cancel) {
                Ok(()) => {
                    let result = extract(&archive, &dest);
                    let _ = std::fs::remove_file(&archive);
                    match result {
                        Ok(()) => return Ok(dest),
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
        return Err(CANCELLED.into());
    }
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
/// are). Refuses absurdly large folders.
pub fn pack_folder(dir: &Path) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    const LIMIT: u64 = 300 << 20;
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if files.len() < 10_000 {
                    stack.push(p);
                }
            } else {
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
                // In a folder of its own: what the host sent doesn't touch what's there.
                let dest = free_folder(&songs, &name);
                extract(&archive, &dest).map(|_| dest)
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
                extract(&archive, &dest).map(|_| dest)
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
/// `to` fail it (zip-slip), and links aren't made.
pub fn unpack(archive: &Path, to: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(to);
    let result = unpack_within(archive, to, MAX_UNPACKED);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(to);
    }
    result
}

fn unpack_within(archive: &Path, to: &Path, limit: u64) -> Result<(), String> {
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
    let tmp = partial_of(dest);
    unpack(archive, &tmp)?;
    if dest.is_dir() {
        merge(&tmp, dest).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    }
    Ok(())
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
    fn unpacking_stops_at_the_limit_and_leaves_nothing() {
        let (dir, archive) = a_zip(
            "limit",
            &[("a/map.osu", &[b'x'; 600]), ("b.bin", &[b'y'; 600])],
        );
        let to = dir.join("set");
        assert!(unpack_within(&archive, &to, 1200).is_ok());
        assert_eq!(std::fs::read(to.join("a/map.osu")).unwrap().len(), 600);
        assert!(unpack_within(&archive, &dir.join("over"), 1199).is_err());
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
    fn a_cancelled_download_stops_before_asking_anyone_and_says_so() {
        let songs = std::env::temp_dir().join(format!("taiko-rs-cancel-{}", std::process::id()));
        let (done, total) = Default::default();
        let cancel = AtomicBool::new(true);
        // Nothing listens there: asking would fail with another error, not "cancelled".
        let sources = [("nowhere", "http://127.0.0.1:9/set".to_string())];
        let started = std::time::Instant::now();
        let got = fetch(&sources, "1 A - T", &songs, &done, &total, &cancel);
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
        let unpacked = import(good.clone(), "good", songs.clone(), false, Vec::new())
            .rx
            .recv()
            .unwrap()
            .unwrap();
        assert!(unpacked.join("map [Oni].osu").is_file());
        assert!(!good.exists());

        let broken = root.join("broken.osz");
        std::fs::write(&broken, b"not a zip").unwrap();
        assert!(
            import(broken.clone(), "broken", songs.clone(), false, Vec::new())
                .rx
                .recv()
                .unwrap()
                .is_err()
        );
        assert!(broken.exists());
        // A temporary copy doesn't stay around either way.
        assert!(
            import(broken.clone(), "broken", songs, true, Vec::new())
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
