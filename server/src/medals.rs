//! Medals: what a player has earned, always worked out again from their plays, so a deleted score
//! takes its medal with it. The public ones are in `medal_list`. The secret ones are read from
//! DIR/medal-secrets.json, which isn't in this repository (like the other private files, it's
//! uploaded by `deploy-via-vps.sh`): the source is public and a secret here wouldn't be one.
//! Without that file there are just no secret medals. A rule is data for the same reason.
//!
//! When a medal was earned, and the counts of what doesn't show in plays (`Rule::Count`), are
//! kept in DIR/medals.json.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::online::{MedalNote, MedalRow, MedalSummary, MedalsView, OnlineScore, Said};

const DAY: u64 = 86_400;
/// How close to a target accuracy counts as hitting it: the game shows two decimals.
const ACCURACY_NEAR: f64 = 0.00005;
/// The medals a profile shows.
const PROFILE_MEDALS: usize = 5;
/// At most this share of the players has a medal of the third tier for it to be rare.
const RARE_SHARE: f64 = 0.20;

/// The section secret medals are in (`medal_list::SECTIONS`).
pub const SECRET_SECTION: u8 = 7;

/// A range of a number; either end may be open.
#[derive(Deserialize, Clone, Copy, Default, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Span {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl Span {
    pub fn from(min: f64) -> Span {
        Span { min: Some(min), max: None }
    }

    pub fn to(max: f64) -> Span {
        Span { min: None, max: Some(max) }
    }

    fn has(&self, v: f64) -> bool {
        self.min.is_none_or(|m| v >= m) && self.max.is_none_or(|m| v <= m)
    }
}

/// What a play has to be. Everything left out passes.
#[derive(Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Filter {
    /// On a map that gives pp, its original file, without No Fail.
    pub clean: bool,
    /// No miss, on 50 notes or more.
    pub full_combo: bool,
    pub accuracy: Span,
    /// Any of these accuracies, to the two decimals shown.
    pub accuracy_near: Vec<f64>,
    pub stars: Span,
    pub combo: Span,
    pub great: Span,
    pub ok: Span,
    pub miss: Span,
    pub notes: Span,
    pub rate: Span,
    pub scroll: Span,
    pub mod_count: Span,
    /// All of these mods on.
    pub mods: Vec<String>,
    /// None of them on.
    pub without: Vec<String>,
    /// Contains this, whatever the case.
    pub artist: Option<String>,
    pub title: Option<String>,
    pub version: Option<String>,
    /// The day it was played on, UTC.
    pub month: Option<u32>,
    pub day: Option<u32>,
    /// The two counts are the same.
    pub great_is_ok: bool,
    /// The longest combo is every hit: whatever was missed was at either end.
    pub combo_is_hits: bool,
    /// A mod that's on is named in the song's title.
    pub mod_in_title: bool,
}

impl Filter {
    fn passes(&self, p: &Play) -> bool {
        let notes = p.great + p.ok + p.miss;
        (!self.clean || p.clean())
            && (!self.full_combo || (p.miss == 0 && notes >= 50))
            && self.accuracy.has(p.accuracy)
            && (self.accuracy_near.is_empty() || self.accuracy_near.iter().any(|a| (p.accuracy - a).abs() < ACCURACY_NEAR))
            && self.stars.has(p.stars)
            && self.combo.has(f64::from(p.combo))
            && self.great.has(f64::from(p.great))
            && self.ok.has(f64::from(p.ok))
            && self.miss.has(f64::from(p.miss))
            && self.notes.has(f64::from(notes))
            && self.rate.has(p.rate)
            && (self.scroll == Span::default() || p.scroll.is_some_and(|s| self.scroll.has(s)))
            && self.mod_count.has(p.mods.len() as f64)
            && self.mods.iter().all(|m| p.has(m))
            && !self.without.iter().any(|m| p.has(m))
            && contains(&p.artist, &self.artist)
            && contains(&p.title, &self.title)
            && contains(&p.version, &self.version)
            && self.month.is_none_or(|m| month_day(p.date).0 == m)
            && self.day.is_none_or(|d| month_day(p.date).1 == d)
            && (!self.great_is_ok || p.great == p.ok)
            && (!self.combo_is_hits || p.combo == p.great + p.ok)
            && (!self.mod_in_title || crate::medal_list::mod_names().iter().any(|(acronym, name)| p.has(acronym) && has_word(&p.title, name)))
    }
}

fn contains(text: &str, part: &Option<String>) -> bool {
    part.as_ref().is_none_or(|part| text.to_lowercase().contains(&part.to_lowercase()))
}

/// Whether `word` is in `text` as whole words, whatever the case.
fn has_word(text: &str, word: &str) -> bool {
    let text = text.to_lowercase();
    let word = word.to_lowercase();
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    text.match_indices(&word).any(|(at, _)| boundary(text[..at].chars().next_back()) && boundary(text[at + word.len()..].chars().next()))
}

/// The UTC month and day of a Unix time.
fn month_day(secs: u64) -> (u32, u32) {
    // Howard Hinnant's civil_from_days.
    let z = (secs / DAY) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (month, day)
}

/// How a medal is earned.
#[allow(clippy::large_enum_variant)] // a list of a hundred or so, read once
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// One play like this.
    Play(Filter),
    /// This many plays kept.
    Plays { need: u64 },
    /// This many notes hit, in all.
    Hits { need: u64 },
    /// This many drumroll hits, in all.
    Rolls { need: u64 },
    /// This many different maps played like this.
    Maps { need: u64, filter: Filter },
    /// This many different songs of one artist played like this.
    Songs { need: u64, filter: Filter },
    /// A play like this on this many days in a row.
    Days { need: u64, filter: Filter },
    /// This many plays in a day.
    InADay { need: u64 },
    /// This many hours of playing.
    Hours { need: u64 },
    /// This many mods in the collection.
    Collection { need: u64 },
    /// All of these mods in the collection.
    Group { mods: Vec<String> },
    /// This many of `mods`, each in a play like this.
    Variety { need: u64, mods: Vec<String>, filter: Filter },
    /// This many maps first played with a helping mod, then at 90% without it.
    Weaned { need: u64 },
    /// One map at 90% slower than 1x, at 1x, and at 1.4x or more.
    SpeedLadder,
    /// Back at a map a week later, 3% better than the first time.
    Rematch,
    /// This many first places taken from someone else.
    Snipes { need: u64 },
    /// This many times in first place on a map of three players or more.
    Hills { need: u64 },
    /// A play before this time.
    Before { time: u64 },
    /// A play like `then`, some time after one like `first`.
    After { first: Filter, then: Filter },
    /// The same score on a map as before (not a perfect one).
    SameScore,
    /// This many of something counted outside of plays.
    Count { event: String, need: u64 },
}

/// A medal.
#[derive(Clone, Debug)]
pub struct Medal {
    pub id: String,
    pub section: u8,
    pub tier: u8,
    pub secret: bool,
    pub name: Said,
    /// How to earn it, or a secret's hint.
    pub about: Said,
    pub art: Vec<String>,
    pub rule: Rule,
    /// Never said in the chat.
    pub quiet: bool,
}

/// A line of DIR/medal-secrets.json.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Secret {
    id: String,
    tier: u8,
    name: Said,
    hint: Said,
    art: Vec<String>,
    rule: Rule,
}

/// A play, for the rules.
#[derive(Clone, Debug, Default)]
pub struct Play {
    pub date: u64,
    pub map: String,
    pub map_hash: u64,
    pub artist: String,
    pub title: String,
    pub version: String,
    pub mods: Vec<String>,
    pub accuracy: f64,
    pub stars: f64,
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub combo: u32,
    pub rate: f64,
    pub scroll: Option<f64>,
    pub score: u64,
    pub rolls: u32,
    /// The map gives pp and this is its original file.
    pub gives: bool,
}

impl Play {
    pub fn of(s: &OnlineScore, gives: bool) -> Play {
        let (song, version) = s.map.rsplit_once(" [").map_or((s.map.as_str(), ""), |(song, v)| (song, v.trim_end_matches(']')));
        let (artist, title) = song.split_once(" - ").unwrap_or(("", song));
        Play {
            date: s.date,
            map: s.map.clone(),
            map_hash: s.map_hash,
            artist: artist.to_string(),
            title: title.to_string(),
            version: version.to_string(),
            mods: s.mods.clone(),
            accuracy: s.accuracy,
            stars: s.stars.unwrap_or(0.0),
            great: s.great,
            ok: s.ok,
            miss: s.miss,
            combo: s.max_combo,
            rate: s.rate,
            scroll: s.scroll_speed,
            score: s.score,
            rolls: s.rolls,
            gives,
        }
    }

    fn has(&self, m: &str) -> bool {
        self.mods.iter().any(|x| x == m)
    }

    fn clean(&self) -> bool {
        self.gives && !self.has("NF")
    }

    /// The mods this play puts in the collection: a real play, with few enough mods to tell
    /// them apart.
    fn tiles(&self, all: &HashSet<&str>) -> Vec<&str> {
        if self.accuracy < 0.80 || self.stars < 3.0 || self.mods.len() > 3 {
            return Vec::new();
        }
        self.mods.iter().map(String::as_str).filter(|m| all.contains(m) && (*m == "NF" || !self.has("NF"))).collect()
    }
}

/// Who took which first places, when: worked out over everyone's plays at once.
#[derive(Default, Debug)]
pub struct Rivals {
    pub snipes: HashMap<String, Vec<u64>>,
    pub hills: HashMap<String, Vec<u64>>,
}

impl Rivals {
    /// From everyone's plays, oldest first, as (player, play): only maps that give pp count.
    pub fn of<'a>(plays: impl Iterator<Item = (&'a str, Play)>) -> Rivals {
        let mut boards: HashMap<u64, HashMap<&str, u64>> = HashMap::new();
        let mut out = Rivals::default();
        for (player, p) in plays {
            if !p.gives {
                continue;
            }
            let board = boards.entry(p.map_hash).or_default();
            let lead = board.iter().max_by_key(|(_, s)| **s).map(|(who, s)| (*who, *s));
            if p.score > board.get(player).copied().unwrap_or(0) {
                board.insert(player, p.score);
            }
            if lead.is_none_or(|(_, s)| p.score > s) {
                if lead.is_some_and(|(who, _)| who != player) {
                    out.snipes.entry(player.to_string()).or_default().push(p.date);
                }
                if board.len() >= 3 {
                    out.hills.entry(player.to_string()).or_default().push(p.date);
                }
            }
        }
        out
    }
}

/// What the rules know besides the plays.
struct Facts<'a> {
    ms: u64,
    events: Option<&'a HashMap<String, u64>>,
    snipes: &'a [u64],
    hills: &'a [u64],
    collectable: HashSet<&'a str>,
}

/// Where a rule stands: when it was met (0: at some point without a date), and how far along it is.
#[derive(Default, Debug, PartialEq)]
struct Standing {
    met: Option<u64>,
    have: u64,
    need: u64,
}

impl Standing {
    /// A count, with the date it got there.
    fn counted(have: u64, need: u64, met: Option<u64>) -> Standing {
        Standing { met, have: have.min(need), need }
    }
}

/// When a running count over the plays reaches `need`, and how far it got.
fn running(plays: &[Play], need: u64, count: impl Fn(&Play) -> u64) -> Standing {
    let mut total = 0;
    let mut met = None;
    for p in plays {
        total += count(p);
        if total >= need && met.is_none() {
            met = Some(p.date);
        }
    }
    Standing::counted(total, need, met)
}

/// When the number of different keys reaches `need`, and how far it got.
fn distinct<K: std::hash::Hash + Eq>(plays: &[Play], need: u64, key: impl Fn(&Play) -> Option<K>) -> Standing {
    let mut seen = HashSet::new();
    let mut met = None;
    for p in plays {
        if let Some(k) = key(p) {
            seen.insert(k);
            if seen.len() as u64 >= need && met.is_none() {
                met = Some(p.date);
            }
        }
    }
    Standing::counted(seen.len() as u64, need, met)
}

/// The longest run of days in a row with a play like `filter`.
fn days_in_a_row(plays: &[Play], need: u64, filter: &Filter) -> Standing {
    let mut days: Vec<u64> = plays.iter().filter(|p| filter.passes(p)).map(|p| p.date / DAY).collect();
    days.dedup();
    let (mut run, mut best, mut last, mut met) = (0, 0, None, None);
    for d in days {
        run = if last.is_some_and(|l: u64| l + 1 == d) { run + 1 } else { 1 };
        last = Some(d);
        best = best.max(run);
        if best >= need && met.is_none() {
            met = Some(d * DAY);
        }
    }
    Standing::counted(best, need, met)
}

fn collection<'a>(plays: &'a [Play], all: &HashSet<&str>, within: impl Fn(&str) -> bool, need: u64) -> Standing {
    let mut seen: HashSet<&'a str> = HashSet::new();
    let mut met = None;
    for p in plays {
        seen.extend(p.tiles(all).into_iter().filter(|m| within(m)));
        if seen.len() as u64 >= need && met.is_none() {
            met = Some(p.date);
        }
    }
    Standing::counted(seen.len() as u64, need, met)
}

/// Maps first played with a helping mod, and then at 90% without one.
fn weaned(plays: &[Play], need: u64) -> Standing {
    const HELP: [&str; 5] = ["NF", "EZ", "HT", "DC", "HA"];
    let mut helped = HashSet::new();
    let mut done = HashSet::new();
    let mut met = None;
    for p in plays {
        if HELP.iter().any(|m| p.has(m)) {
            helped.insert(p.map_hash);
        } else if helped.contains(&p.map_hash) && p.clean() && p.accuracy >= 0.90 {
            done.insert(p.map_hash);
            if done.len() as u64 >= need && met.is_none() {
                met = Some(p.date);
            }
        }
    }
    Standing::counted(done.len() as u64, need, met)
}

fn speed_ladder(plays: &[Play]) -> Option<u64> {
    let mut seen: HashMap<&str, [bool; 3]> = HashMap::new();
    for p in plays.iter().filter(|p| p.clean() && p.accuracy >= 0.90 && p.stars >= 3.0) {
        let band = if p.rate < 0.95 {
            0
        } else if p.rate < 1.05 {
            1
        } else if p.rate >= 1.4 {
            2
        } else {
            continue;
        };
        let bands = seen.entry(&p.map).or_default();
        bands[band] = true;
        if *bands == [true; 3] {
            return Some(p.date);
        }
    }
    None
}

fn rematch(plays: &[Play]) -> Option<u64> {
    let mut first: HashMap<u64, (u64, f64)> = HashMap::new();
    for p in plays {
        match first.get(&p.map_hash) {
            Some(&(date, accuracy)) => {
                if p.date - date >= 7 * DAY && p.accuracy >= accuracy + 0.03 && p.clean() && p.accuracy >= 0.85 {
                    return Some(p.date);
                }
            }
            None => {
                first.insert(p.map_hash, (p.date, p.accuracy));
            }
        }
    }
    None
}

fn same_score(plays: &[Play]) -> Option<u64> {
    let mut seen = HashSet::new();
    plays.iter().find(|p| !seen.insert((p.map_hash, p.score)) && p.score > 0 && p.accuracy < 0.99999).map(|p| p.date)
}

fn in_a_day(plays: &[Play], need: u64) -> Standing {
    let mut days: HashMap<u64, u64> = HashMap::new();
    let (mut best, mut met) = (0, None);
    for p in plays {
        let n = days.entry(p.date / DAY).or_default();
        *n += 1;
        best = best.max(*n);
        if *n >= need && met.is_none() {
            met = Some(p.date);
        }
    }
    Standing::counted(best, need, met)
}

/// One date meant "now": a rule met outside of plays.
const NOW: u64 = 0;

fn standing(rule: &Rule, plays: &[Play], facts: &Facts) -> Standing {
    let date = |met: Option<u64>| Standing { met, ..Standing::default() };
    let nth = |dates: &[u64], need: u64| Standing::counted(dates.len() as u64, need, dates.get(need as usize - 1).copied());
    match rule {
        Rule::Play(f) => date(plays.iter().find(|p| f.passes(p)).map(|p| p.date)),
        Rule::Plays { need } => running(plays, *need, |_| 1),
        Rule::Hits { need } => running(plays, *need, |p| u64::from(p.great + p.ok)),
        Rule::Rolls { need } => running(plays, *need, |p| u64::from(p.rolls)),
        Rule::Maps { need, filter } => distinct(plays, *need, |p| filter.passes(p).then_some(p.map_hash)),
        Rule::Songs { need, filter } => {
            let mut songs: HashMap<String, HashSet<String>> = HashMap::new();
            let mut met = None;
            let mut best = 0;
            for p in plays.iter().filter(|p| filter.passes(p)) {
                let of_artist = songs.entry(p.artist.to_lowercase()).or_default();
                of_artist.insert(p.title.to_lowercase());
                best = best.max(of_artist.len() as u64);
                if best >= *need && met.is_none() {
                    met = Some(p.date);
                }
            }
            Standing::counted(best, *need, met)
        }
        Rule::Days { need, filter } => days_in_a_row(plays, *need, filter),
        Rule::InADay { need } => in_a_day(plays, *need),
        Rule::Hours { need } => {
            let have = facts.ms / 3_600_000;
            Standing::counted(have, *need, (have >= *need).then_some(NOW))
        }
        Rule::Collection { need } => collection(plays, &facts.collectable, |_| true, *need),
        Rule::Group { mods } => collection(plays, &facts.collectable, |m| mods.iter().any(|x| x == m), mods.len() as u64),
        Rule::Variety { need, mods, filter } => {
            let mut seen = HashSet::new();
            let mut met = None;
            for p in plays.iter().filter(|p| filter.passes(p)) {
                seen.extend(p.mods.iter().filter(|m| mods.contains(m)));
                if seen.len() as u64 >= *need && met.is_none() {
                    met = Some(p.date);
                }
            }
            Standing::counted(seen.len() as u64, *need, met)
        }
        Rule::Weaned { need } => weaned(plays, *need),
        Rule::SpeedLadder => date(speed_ladder(plays)),
        Rule::Rematch => date(rematch(plays)),
        Rule::Snipes { need } => nth(facts.snipes, *need),
        Rule::Hills { need } => nth(facts.hills, *need),
        Rule::Before { time } => date(plays.first().filter(|p| p.date < *time).map(|p| p.date)),
        Rule::After { first, then } => {
            let start = plays.iter().position(|p| first.passes(p));
            date(start.and_then(|s| plays[s..].iter().find(|p| then.passes(p))).map(|p| p.date))
        }
        Rule::SameScore => date(same_score(plays)),
        Rule::Count { event, need } => {
            let have = facts.events.and_then(|e| e.get(event)).copied().unwrap_or(0);
            Standing::counted(have, *need, (have >= *need).then_some(NOW))
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    /// player -> medal -> when.
    earned: HashMap<String, BTreeMap<String, u64>>,
    /// player -> what -> how many.
    events: HashMap<String, HashMap<String, u64>>,
}

/// Every player's medals.
pub struct Medals {
    list: Vec<Medal>,
    file: Option<PathBuf>,
    saved: Saved,
    holders: HashMap<String, u32>,
}

/// A medal to say in the chat.
pub struct Announcement {
    pub medal: Medal,
    pub first: bool,
    pub holders: u32,
    pub players: u32,
}

impl Medals {
    /// Reads what's kept in `dir` (None: in memory only, for tests).
    pub fn load(dir: Option<&std::path::Path>) -> Medals {
        let mut list = crate::medal_list::all();
        let file = dir.map(|d| d.join("medals.json"));
        if let Some(dir) = dir {
            let secrets = dir.join("medal-secrets.json");
            match std::fs::read_to_string(&secrets) {
                Ok(text) => match serde_json::from_str::<Vec<Secret>>(&text) {
                    Ok(found) => {
                        println!("{} secret medals", found.len());
                        list.extend(found.into_iter().map(Secret::medal));
                    }
                    // Better without them than with half: and no medal gets taken away for it,
                    // because the next start finds them again.
                    Err(e) => eprintln!("{}: {e}", secrets.display()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => eprintln!("{}: {e}", secrets.display()),
            }
        }
        let saved = file.as_deref().map(crate::load_saved).unwrap_or_default();
        Medals { list, file, saved, holders: HashMap::new() }
    }

    fn save(&self) {
        if let Some(file) = &self.file
            && let Ok(text) = serde_json::to_string(&self.saved)
            && let Err(e) = crate::write_private(file, &text)
        {
            eprintln!("saving medals: {e}");
        }
    }

    fn medal(&self, id: &str) -> Option<&Medal> {
        self.list.iter().find(|m| m.id == id)
    }

    fn players(&self) -> u32 {
        self.saved.earned.values().filter(|e| !e.is_empty()).count() as u32
    }

    fn recount(&mut self) {
        self.holders.clear();
        for earned in self.saved.earned.values() {
            for id in earned.keys() {
                *self.holders.entry(id.clone()).or_default() += 1;
            }
        }
    }

    fn holders(&self, id: &str) -> u32 {
        self.holders.get(id).copied().unwrap_or(0)
    }

    fn facts<'a>(&'a self, player: &str, rivals: &'a Rivals, ms: u64) -> Facts<'a> {
        Facts {
            ms,
            events: self.saved.events.get(player),
            snipes: rivals.snipes.get(player).map_or(&[], Vec::as_slice),
            hills: rivals.hills.get(player).map_or(&[], Vec::as_slice),
            collectable: crate::medal_list::collectable(),
        }
    }

    /// Works out `player`'s medals again, and says which are new. `plays`: theirs, oldest first.
    /// `exact`: drops what they no longer meet. Otherwise a medal stays: what a rule sees can
    /// change for the moment (a map whose status isn't known yet), and earning one twice would
    /// announce it twice.
    fn settle_one(&mut self, player: &str, plays: &[Play], rivals: &Rivals, ms: u64, now: u64, exact: bool) -> Vec<String> {
        let before = self.saved.earned.get(player).cloned().unwrap_or_default();
        let mut after = if exact { BTreeMap::new() } else { before.clone() };
        let facts = self.facts(player, rivals, ms);
        for m in &self.list {
            if let Some(met) = standing(&m.rule, plays, &facts).met {
                after.insert(m.id.clone(), before.get(&m.id).copied().unwrap_or(if met == NOW { now } else { met }));
            }
        }
        let new = after.keys().filter(|id| !before.contains_key(*id)).cloned().collect();
        self.saved.earned.insert(player.to_string(), after);
        new
    }

    /// Everyone's medals, worked out at start: nobody is told, it's just what they have.
    pub fn settle_all(&mut self, everyone: &[(String, Vec<Play>, u64)], rivals: &Rivals, now: u64) {
        for (player, plays, ms) in everyone {
            self.settle_one(player, plays, rivals, *ms, now, false);
        }
        self.recount();
        self.save();
    }

    /// `player`'s medals after something changed: the new ones, as the game shows them.
    pub fn settle(&mut self, player: &str, plays: &[Play], rivals: &Rivals, ms: u64, now: u64) -> Vec<MedalNote> {
        let new = self.settle_one(player, plays, rivals, ms, now, false);
        self.recount();
        self.save();
        new.iter().filter_map(|id| self.medal(id)).map(Medal::note).collect()
    }

    /// `player`'s medals after a score of theirs was deleted: what it earned goes with it.
    pub fn revoke(&mut self, player: &str, plays: &[Play], rivals: &Rivals, ms: u64, now: u64) {
        self.settle_one(player, plays, rivals, ms, now, true);
        self.recount();
        self.save();
    }

    /// `from`'s medals and counts are `to`'s from now on.
    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(earned) = self.saved.earned.remove(from) {
            self.saved.earned.insert(to.to_string(), earned);
        }
        if let Some(events) = self.saved.events.remove(from) {
            self.saved.events.insert(to.to_string(), events);
        }
        self.save();
    }

    /// How many of the medals there are now `earned` has: a secret can go when its file does.
    fn held(&self, earned: &BTreeMap<String, u64>) -> u32 {
        self.list.iter().filter(|m| earned.contains_key(&m.id)).count() as u32
    }

    /// Adds to a count (`absolute`: sets it). Returns whether it changed.
    pub fn count(&mut self, player: &str, event: &str, by: u64, absolute: bool) -> bool {
        let n = self.saved.events.entry(player.to_string()).or_default().entry(event.to_string()).or_default();
        let next = if absolute { by } else { *n + by };
        let changed = next != *n;
        *n = next;
        changed
    }

    /// Everyone with something counted for them, plays or not.
    pub fn counted_players(&self) -> Vec<String> {
        self.saved.events.keys().cloned().collect()
    }

    /// The rarest of `new` that's rare enough to say in the chat, if any. Medals don't say
    /// themselves when they're just catching up with what's there: only this is ever announced.
    pub fn announcement(&self, new: &[MedalNote]) -> Option<Announcement> {
        let players = self.players();
        new.iter()
            .filter_map(|n| self.medal(&n.id))
            .filter(|m| {
                let holders = self.holders(&m.id);
                !m.quiet
                    && (m.tier >= 4
                        || (m.secret && m.tier >= 2)
                        || (m.tier == 3 && (holders <= 1 || f64::from(holders) <= f64::from(players) * RARE_SHARE)))
            })
            .min_by_key(|m| (self.holders(&m.id), std::cmp::Reverse(m.tier)))
            .map(|m| {
                let holders = self.holders(&m.id);
                Announcement { medal: m.clone(), first: holders <= 1, holders, players }
            })
    }

    /// Every medal, and how `player` stands with them (`rivals`, `plays`, `ms` being theirs).
    pub fn view(&self, player: Option<&str>, plays: &[Play], rivals: &Rivals, ms: u64) -> MedalsView {
        let all = crate::medal_list::collectable();
        let earned = player.and_then(|p| self.saved.earned.get(p));
        let facts = self.facts(player.unwrap_or(""), rivals, ms);
        let medals = self
            .list
            .iter()
            .map(|m| {
                let at = earned.and_then(|e| e.get(&m.id)).copied();
                let holders = self.holders(&m.id);
                let shown = !m.secret || holders > 0 || at.is_some();
                let s = if player.is_some() && !m.secret { standing(&m.rule, plays, &facts) } else { Standing::default() };
                MedalRow {
                    id: m.id.clone(),
                    section: m.section,
                    tier: m.tier,
                    secret: m.secret,
                    name: shown.then(|| m.name.clone()),
                    how: (!m.secret).then(|| m.about.clone()),
                    hint: (m.secret && shown).then(|| m.about.clone()),
                    art: if shown { m.art.clone() } else { Vec::new() },
                    at,
                    holders,
                    have: s.have,
                    need: s.need,
                }
            })
            .collect();
        let tiles: HashSet<&str> = plays.iter().flat_map(|p| p.tiles(&all)).collect();
        let mut collection: Vec<String> = tiles.into_iter().map(str::to_string).collect();
        collection.sort();
        MedalsView { players: self.players(), earned: earned.map_or(0, |e| self.held(e)), medals, collection, collection_size: all.len() as u32 }
    }

    /// What a profile shows of `player`'s medals.
    pub fn summary(&self, player: &str) -> MedalSummary {
        let earned = self.saved.earned.get(player);
        let has = |m: &&Medal| earned.is_some_and(|e| e.contains_key(&m.id));
        let dan = self.list.iter().filter(|m| m.id.starts_with("dan_")).rfind(has).map(|m| m.name.clone());
        let mut rarest: Vec<&Medal> = self.list.iter().filter(has).collect();
        rarest.sort_by_key(|m| (self.holders(&m.id), std::cmp::Reverse(m.tier)));
        MedalSummary {
            earned: earned.map_or(0, |e| self.held(e)),
            total: self.list.len() as u32,
            dan,
            rarest: rarest.into_iter().take(PROFILE_MEDALS).map(Medal::note).collect(),
        }
    }

    /// Who has the most medals, most first: (name key, how many).
    pub fn table(&self) -> Vec<(String, u32)> {
        let mut rows: Vec<(String, u32)> = self.saved.earned.iter().map(|(p, e)| (p.clone(), self.held(e))).filter(|(_, n)| *n > 0).collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        rows
    }
}

impl Medal {
    pub fn note(&self) -> MedalNote {
        MedalNote { id: self.id.clone(), name: self.name.clone(), about: self.about.clone(), tier: self.tier, secret: self.secret, art: self.art.clone() }
    }
}

impl Secret {
    fn medal(self) -> Medal {
        Medal { id: self.id, section: SECRET_SECTION, tier: self.tier, secret: true, name: self.name, about: self.hint, art: self.art, rule: self.rule, quiet: false }
    }
}

/// A medal's text as a line of plain English for the chat of older games.
pub fn chat_text(player: &str, a: &Announcement) -> String {
    let name = &a.medal.name[0];
    let line = if a.medal.secret { format!("{player} found the secret {name}") } else { format!("{player} earned {name}") };
    let rarity = if a.first { "first on the server".to_string() } else { format!("{} of {} have it", a.holders, a.players) };
    format!("{line} · {rarity}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(date: u64) -> Play {
        Play { date, map: "a - b [c]".into(), map_hash: 1, artist: "a".into(), title: "b".into(), version: "c".into(), accuracy: 0.95, stars: 5.0, great: 100, ok: 10, miss: 1, combo: 90, rate: 1.0, score: 1000, gives: true, ..Play::default() }
    }

    fn facts(collectable: HashSet<&str>) -> Facts<'_> {
        Facts { ms: 0, events: None, snipes: &[], hills: &[], collectable }
    }

    fn met(rule: &Rule, plays: &[Play]) -> Option<u64> {
        standing(rule, plays, &facts(crate::medal_list::collectable())).met
    }

    #[test]
    fn a_span_is_open_at_the_ends_left_out() {
        assert!(Span::default().has(5.0));
        assert!(Span::from(5.0).has(5.0) && !Span::from(5.0).has(4.9));
        assert!(Span::to(5.0).has(5.0) && !Span::to(5.0).has(5.1));
    }

    #[test]
    fn a_clean_play_is_on_a_map_with_pp_and_without_no_fail() {
        let f = Filter { clean: true, ..Filter::default() };
        assert!(f.passes(&play(1)));
        assert!(!f.passes(&Play { gives: false, ..play(1) }));
        assert!(!f.passes(&Play { mods: vec!["NF".into()], ..play(1) }));
    }

    #[test]
    fn accuracy_near_is_to_the_two_decimals_shown() {
        let f = Filter { accuracy_near: vec![0.5, 0.25], ..Filter::default() };
        assert!(f.passes(&Play { accuracy: 0.50004, ..play(1) }));
        assert!(f.passes(&Play { accuracy: 0.24996, ..play(1) }));
        assert!(!f.passes(&Play { accuracy: 0.5002, ..play(1) }));
    }

    #[test]
    fn a_mod_in_a_title_has_to_be_a_whole_word() {
        let f = Filter { mod_in_title: true, ..Filter::default() };
        let on = |title: &str, m: &str| Play { title: title.into(), mods: vec![m.into()], ..play(1) };
        assert!(f.passes(&on("The Mirror Song", "MR")));
        assert!(f.passes(&on("mirror", "MR")));
        assert!(!f.passes(&on("Mirrors", "MR")));
        assert!(!f.passes(&on("The Mirror Song", "HD")));
    }

    #[test]
    fn dates_come_out_right_on_leap_days_and_new_years() {
        assert_eq!(month_day(1_709_164_800), (2, 29), "2024-02-29");
        assert_eq!(month_day(1_704_067_199), (12, 31), "2023-12-31");
        assert_eq!(month_day(1_704_067_200), (1, 1), "2024-01-01");
    }

    #[test]
    fn days_in_a_row_start_over_after_a_gap() {
        let rule = Rule::Days { need: 3, filter: Filter::default() };
        let on = |days: &[u64]| days.iter().map(|d| play(d * DAY)).collect::<Vec<_>>();
        assert_eq!(met(&rule, &on(&[1, 2, 4, 5])), None);
        assert_eq!(met(&rule, &on(&[1, 2, 2, 3])), Some(3 * DAY));
        assert_eq!(standing(&rule, &on(&[1, 2, 4]), &facts(HashSet::new())).have, 2);
    }

    #[test]
    fn the_collection_takes_real_plays_with_few_mods() {
        let rule = Rule::Collection { need: 2 };
        let with = |mods: &[&str], accuracy: f64, stars: f64| Play { mods: mods.iter().map(|m| m.to_string()).collect(), accuracy, stars, ..play(1) };
        assert_eq!(met(&rule, &[with(&["HD", "HR"], 0.9, 4.0)]), Some(1));
        assert_eq!(met(&rule, &[with(&["HD", "HR"], 0.7, 4.0)]), None, "under 80%");
        assert_eq!(met(&rule, &[with(&["HD", "HR"], 0.9, 2.0)]), None, "too easy a map");
        assert_eq!(met(&rule, &[with(&["HD", "HR", "SW", "AB"], 0.9, 4.0)]), None, "four mods at once");
        assert_eq!(met(&rule, &[with(&["HD", "NF"], 0.9, 4.0)]), None, "No Fail only counts for itself");
        assert_eq!(met(&rule, &[with(&["AT", "RX"], 0.9, 4.0)]), None, "not in the collection");
    }

    #[test]
    fn weaning_needs_the_help_first() {
        let rule = Rule::Weaned { need: 1 };
        let helped = Play { mods: vec!["HT".into()], ..play(1) };
        assert_eq!(met(&rule, &[play(1), helped.clone()]), None);
        assert_eq!(met(&rule, &[helped, play(2)]), Some(2));
    }

    #[test]
    fn a_speed_ladder_takes_all_three_on_one_map() {
        let at = |rate: f64| Play { rate, ..play(1) };
        assert_eq!(met(&Rule::SpeedLadder, &[at(0.75), at(1.0)]), None);
        assert_eq!(met(&Rule::SpeedLadder, &[at(0.75), at(1.0), at(1.5)]), Some(1));
    }

    #[test]
    fn a_rematch_is_a_week_later_and_better() {
        let later = |days: u64, accuracy: f64| Play { accuracy, ..play(1 + days * DAY) };
        assert_eq!(met(&Rule::Rematch, &[later(0, 0.90), later(6, 0.95)]), None, "too soon");
        assert_eq!(met(&Rule::Rematch, &[later(0, 0.90), later(8, 0.92)]), None, "not enough");
        assert_eq!(met(&Rule::Rematch, &[later(0, 0.90), later(8, 0.93)]), Some(1 + 8 * DAY));
    }

    #[test]
    fn the_same_score_twice_counts_but_not_perfect_ones() {
        assert_eq!(met(&Rule::SameScore, &[play(1), play(2)]), Some(2));
        let perfect = |d: u64| Play { accuracy: 1.0, ..play(d) };
        assert_eq!(met(&Rule::SameScore, &[perfect(1), perfect(2)]), None);
    }

    #[test]
    fn taking_first_place_needs_someone_to_take_it_from() {
        let mut p = play(1);
        let rivals = Rivals::of([("a", p.clone()), ("a", Play { score: 2000, ..p.clone() }), ("b", Play { score: 1500, ..p.clone() }), ("b", Play { score: 3000, ..p.clone() }), ("c", Play { score: 3500, ..p.clone() })].into_iter());
        assert!(!rivals.snipes.contains_key("a"), "their own score");
        assert_eq!(rivals.snipes["b"].len(), 1);
        assert_eq!(rivals.snipes["c"].len(), 1);
        assert_eq!(rivals.hills["c"].len(), 1, "third on the board");
        assert!(!rivals.hills.contains_key("b"), "only two on the board then");
        p.gives = false;
        assert!(Rivals::of([("a", p.clone()), ("b", Play { score: 2000, ..p })].into_iter()).snipes.is_empty(), "maps without pp don't count");
    }

    #[test]
    fn a_deleted_score_takes_its_medal_with_it() {
        let mut m = Medals::load(None);
        let rivals = Rivals::default();
        let new = m.settle("a", &[play(1)], &rivals, 0, 10);
        assert!(new.iter().any(|n| n.id == "first_play"));
        assert_eq!(m.settle("a", &[play(1)], &rivals, 0, 20), vec![], "already had them");
        m.settle("a", &[], &rivals, 0, 25);
        assert!(m.summary("a").earned > 0, "only a deletion takes one back");
        m.revoke("a", &[], &rivals, 0, 30);
        assert_eq!(m.summary("a").earned, 0);
        assert_eq!(m.players(), 0);
    }

    #[test]
    fn when_a_medal_was_earned_stays_put() {
        let mut m = Medals::load(None);
        let rivals = Rivals::default();
        m.settle("a", &[play(100)], &rivals, 3_600_000, 500);
        let at = |m: &Medals, id: &str| m.saved.earned["a"][id];
        assert_eq!(at(&m, "first_play"), 100, "a medal of a play: that play's date");
        assert_eq!(at(&m, "hours_1"), 500, "no play to date it by: when it came");
        m.settle("a", &[play(100), play(200)], &rivals, 3_600_000, 900);
        assert_eq!(at(&m, "hours_1"), 500);
    }

    #[test]
    fn counts_outside_of_plays_earn_medals() {
        let mut m = Medals::load(None);
        let rivals = Rivals::default();
        assert!(m.count("a", "friends", 1, true));
        assert!(!m.count("a", "friends", 1, true), "no change");
        let new = m.settle("a", &[], &rivals, 0, 5);
        assert!(new.iter().any(|n| n.id == "v_friend"));
    }

    #[test]
    fn only_rare_medals_are_announced() {
        let mut m = Medals::load(None);
        let rivals = Rivals::default();
        // Ten players, all with a medal of the lowest tier.
        for p in 0..10 {
            m.settle(&format!("p{p}"), &[play(1)], &rivals, 0, 10);
        }
        let common = m.medal("first_play").unwrap().note();
        assert!(m.announcement(&[common]).is_none());
        // The first to get a Muzukashii one, whatever else came with it.
        let new = m.settle("p0", &[play(1), Play { accuracy: 1.0, stars: 5.5, miss: 0, combo: 300, ..play(2) }], &rivals, 0, 20);
        let a = m.announcement(&new).expect("rare");
        assert_eq!(a.medal.id, "ss_5");
        assert!(a.first);
        assert_eq!(a.holders, 1);
        assert_eq!(a.players, 10);
        // A quiet one never is, however rare.
        assert!(m.medal("r_founder").unwrap().quiet);
        let quiet = Medals { list: m.list.iter().filter(|x| x.id == "r_founder").cloned().map(|x| Medal { tier: 4, ..x }).collect(), file: None, saved: Saved::default(), holders: HashMap::new() };
        assert!(quiet.announcement(&[quiet.list[0].note()]).is_none());
    }

    #[test]
    fn a_secret_stays_hidden_until_someone_finds_it() {
        let mut m = Medals::load(None);
        m.list.push(Medal { id: "s".into(), section: SECRET_SECTION, tier: 2, secret: true, name: Default::default(), about: ["hint".into(), "h".into(), "h".into()], art: vec!["x".into()], rule: Rule::Play(Filter { combo: Span { min: Some(777.0), max: Some(777.0) }, ..Filter::default() }), quiet: false });
        let rivals = Rivals::default();
        let row = |m: &Medals, who: &str| m.view(Some(who), &[], &rivals, 0).medals.into_iter().find(|r| r.id == "s").unwrap();
        m.settle("a", &[play(1)], &rivals, 0, 5);
        let hidden = row(&m, "a");
        assert!(hidden.name.is_none() && hidden.hint.is_none() && hidden.how.is_none() && hidden.art.is_empty());
        m.settle("b", &[Play { combo: 777, ..play(1) }], &rivals, 0, 5);
        let found = row(&m, "a");
        assert!(found.name.is_some() && found.hint.is_some() && found.how.is_none(), "the hint opens for everyone, the rule never does");
    }

    #[test]
    fn secrets_are_read_from_their_file_and_a_broken_one_loses_nothing_else() {
        let dir = std::env::temp_dir().join(format!("taiko-medals-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = r#"[{"id":"s_x","tier":2,"name":["a","b","c"],"hint":["d","e","f"],"art":[],"rule":{"play":{"combo":{"min":5,"max":5},"accuracy_near":[0.5]}}},
                         {"id":"s_y","tier":3,"name":["a","b","c"],"hint":["d","e","f"],"art":[],"rule":{"days":{"need":5,"filter":{"mods":["AB"]}}}},
                         {"id":"s_z","tier":3,"name":["a","b","c"],"hint":["d","e","f"],"art":[],"rule":"same_score"}]"#;
        std::fs::write(dir.join("medal-secrets.json"), secret).unwrap();
        let m = Medals::load(Some(&dir));
        assert!(m.medal("s_x").is_some_and(|x| x.secret && x.section == SECRET_SECTION));
        assert_eq!(m.medal("s_z").unwrap().rule, Rule::SameScore);
        std::fs::write(dir.join("medal-secrets.json"), "{").unwrap();
        let m = Medals::load(Some(&dir));
        assert!(m.medal("s_x").is_none());
        assert!(m.medal("first_play").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_dan_on_a_profile_is_the_highest_one() {
        let mut m = Medals::load(None);
        m.settle("a", &[Play { stars: 7.5, ..play(1) }], &Rivals::default(), 0, 5);
        assert_eq!(m.summary("a").dan.unwrap()[0], m.medal("dan_7").unwrap().name[0]);
    }
}
