//! The song select search, with osu!'s filters: `stars>5 status=r length<2:30 creator=someone`.
//! A word with a known key and an operator (`=` `:` `!=` `<` `<=` `>` `>=`) filters on it;
//! anything else is text every map's name (or mapper) has to contain. Values can be quoted.
//!
//! - `star` `stars` `sr`, `od` `hp`, `bpm`: numbers. `=` matches to the precision typed, so
//!   `sr=5` is 4.5 to 5.5 and `sr=5.2` is 5.15 to 5.25; `<` `>` compare exactly.
//! - `length` `len`: seconds, or with units (`90s`, `1m30s`, `500ms`) or as `2:30`.
//! - `played`: days since last played (maps never played don't match); `unplayed` on its own.
//! - `status`: `r`anked `a`pproved `q`ualified `l`oved `p`ending `w`ip `g`raveyard `n`ot
//!   submitted `u`nknown (a prefix of the word will do). osu!.db can't tell pending from
//!   graveyard, so either matches both there. `status>=r` is ranked or better, as in osu!.
//!   `status=epic` (or `status!=epic`) goes by the taiko.rs EPIC mark instead, which a map has
//!   on top of its status.
//! - `creator` `mapper` `author`, `artist`, `title`, `diff` `version`, `tag` `tags`, `source`:
//!   text the field contains.
//! - `id` `beatmapid` `bid`, `set` `setid` `beatmapset`: the osu! beatmap or set ID (exact, or
//!   compared with `<` `>`). A bare number word also finds the map or set of that ID.
//!
//! A plain word is looked for in the name, the mapper, the osu! tags and the source.

/// osu!.db's status codes (plus graveyard, which the mirrors know and osu!.db doesn't).
pub const UNKNOWN: u8 = 0;
pub const NOT_SUBMITTED: u8 = 1;
pub const PENDING: u8 = 2;
pub const GRAVEYARD: u8 = 3;
pub const RANKED: u8 = 4;
pub const APPROVED: u8 = 5;
pub const QUALIFIED: u8 = 6;
pub const LOVED: u8 = 7;
/// Made in the editor and sent to taiko.rs: public, no pp; and once approved there, with pp.
pub const CUSTOM: u8 = 8;
pub const CUSTOM_APPROVED: u8 = 9;
/// A TJA chart's course: played on this computer only.
pub const TJA: u8 = 10;

/// The status code for the mirrors' status name.
pub fn status_code(name: &str) -> Option<u8> {
    Some(match name {
        "ranked" => RANKED,
        "approved" => APPROVED,
        "qualified" => QUALIFIED,
        "loved" => LOVED,
        "pending" | "wip" => PENDING,
        "graveyard" => GRAVEYARD,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Num {
    Stars,
    Od,
    Hp,
    Bpm,
    Length,
    Played,
    /// The osu! beatmap ID; maps without one never match.
    BeatmapId,
    /// The osu! beatmap set ID.
    SetId,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Field {
    Tags,
    Source,
    Creator,
    Artist,
    Title,
    Version,
}

#[derive(Clone, PartialEq, Debug)]
enum Cond {
    /// `value` within `tolerance` counts as equal.
    Num {
        key: Num,
        op: Op,
        value: f64,
        tolerance: f64,
    },
    Text {
        field: Field,
        negated: bool,
        value: String,
    },
    /// The codes the typed status stands for.
    Status {
        op: Op,
        codes: Vec<u8>,
    },
    /// Marked EPIC on taiko.rs (or not, when negated).
    Epic {
        negated: bool,
    },
    Unplayed,
    /// In the collection named so (any case), or in none (`collection=none`): the client knows
    /// its collections and applies these itself (`Query::collections`).
    Collection {
        negated: bool,
        name: InCollection,
    },
}

/// What a `collection=` filter asks for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InCollection {
    /// In the collection of this name (lowercased).
    Named(String),
    /// In no collection.
    None,
}

/// What a map is, for the filters. None where it isn't known (yet).
#[derive(Default, Clone, Copy)]
pub struct Facts {
    pub stars: Option<f64>,
    pub length_s: Option<f64>,
    pub bpm: Option<f64>,
    /// Days since last played.
    pub played_days: Option<f64>,
    pub status: u8,
    /// Marked EPIC on taiko.rs.
    pub epic: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Query {
    /// Lowercased.
    words: Vec<String>,
    conds: Vec<Cond>,
}

/// The query's words, quotes keeping spaces in (`artist="the quick"`).
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in s.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `key`, the operator and the value of `key<op>value`.
fn split(token: &str) -> Option<(&str, Op, &str)> {
    let at = token.find(['=', ':', '!', '<', '>'])?;
    let (key, rest) = token.split_at(at);
    let (op, len) = match rest.as_bytes() {
        [b'!', b'=', ..] => (Op::Ne, 2),
        [b'<', b'=', ..] => (Op::Le, 2),
        [b'>', b'=', ..] => (Op::Ge, 2),
        [b'=', b'=', ..] => (Op::Eq, 2),
        [b'<', ..] => (Op::Lt, 1),
        [b'>', ..] => (Op::Gt, 1),
        [b'=' | b':', ..] => (Op::Eq, 1),
        _ => return None,
    };
    Some((key, op, &rest[len..]))
}

/// A number and the half-step of its last digit (`5.2` -> (5.2, 0.05)).
fn number(s: &str) -> Option<(f64, f64)> {
    let v: f64 = s.parse().ok().filter(|v: &f64| v.is_finite())?;
    let decimals = s.split_once('.').map(|(_, d)| d.len()).unwrap_or(0) as i32;
    Some((v, 0.5 * 10f64.powi(-decimals)))
}

/// A length in seconds: `90`, `90s`, `1m30s`, `500ms`, `1h`, `2:30`; and its precision.
fn length(s: &str) -> Option<(f64, f64)> {
    if let Some((m, sec)) = s.split_once(':') {
        let (m, sec) = (m.parse::<f64>().ok()?, sec.parse::<f64>().ok()?);
        return Some((m * 60.0 + sec, 0.5));
    }
    if let Some(ms) = s.strip_suffix("ms") {
        let (v, tol) = number(ms)?;
        return Some((v / 1000.0, tol / 1000.0));
    }
    // Digits then a unit, any number of times; a bare number is seconds.
    let (mut total, mut tol) = (0.0, 0.5);
    let mut digits = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            digits.push(ch);
            continue;
        }
        let unit = match ch {
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        let (v, t) = number(&std::mem::take(&mut digits))?;
        total += v * unit;
        tol = t * unit;
    }
    if !digits.is_empty() {
        let (v, t) = number(&digits)?;
        total += v;
        tol = t;
    }
    Some((total, tol))
}

/// The codes a typed status (a prefix of its name) stands for.
fn statuses(s: &str) -> Option<Vec<u8>> {
    let names: [(&str, &[u8]); 12] = [
        ("ranked", &[RANKED]),
        ("approved", &[APPROVED]),
        ("qualified", &[QUALIFIED]),
        ("loved", &[LOVED]),
        ("pending", &[PENDING]),
        ("wip", &[PENDING]),
        // osu!.db can't tell graveyard from pending.
        ("graveyard", &[GRAVEYARD, PENDING]),
        ("notsubmitted", &[NOT_SUBMITTED]),
        ("unknown", &[UNKNOWN]),
        ("custom", &[CUSTOM, CUSTOM_APPROVED]),
        ("customapproved", &[CUSTOM_APPROVED]),
        ("tja", &[TJA]),
    ];
    let s = s.replace([' ', '_', '-'], "");
    if s.is_empty() {
        return None;
    }
    names
        .iter()
        .find(|(name, _)| name.starts_with(&s))
        .map(|(_, codes)| codes.to_vec())
}

/// Whether a typed status means the EPIC mark: a prefix of the word, none of osu!'s statuses
/// starting with an e.
fn is_epic_word(s: &str) -> bool {
    !s.is_empty() && "epic".starts_with(&s.to_lowercase())
}

/// An osu! ID: whole digits only.
fn id(s: &str) -> Option<(f64, f64)> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number(s)
}

fn cond(key: &str, op: Op, value: &str) -> Option<Cond> {
    let num = |key: Num, parsed: Option<(f64, f64)>| {
        parsed.map(|(value, tolerance)| Cond::Num {
            key,
            op,
            value,
            tolerance,
        })
    };
    let text = |field: Field| match op {
        Op::Eq | Op::Ne if !value.is_empty() => Some(Cond::Text {
            field,
            negated: op == Op::Ne,
            value: value.to_lowercase(),
        }),
        _ => None,
    };
    match key.to_lowercase().as_str() {
        "star" | "stars" | "sr" => num(Num::Stars, number(value)),
        "od" => num(Num::Od, number(value)),
        "hp" => num(Num::Hp, number(value)),
        "bpm" => num(Num::Bpm, number(value)),
        "length" | "len" => num(Num::Length, length(value)),
        "played" => num(Num::Played, number(value)),
        "id" | "bid" | "beatmapid" | "mapid" => num(Num::BeatmapId, id(value)),
        "set" | "setid" | "sid" | "beatmapset" | "beatmapsetid" => num(Num::SetId, id(value)),
        "status" if is_epic_word(value) => match op {
            Op::Eq | Op::Ne => Some(Cond::Epic {
                negated: op == Op::Ne,
            }),
            _ => None,
        },
        "status" => statuses(&value.to_lowercase()).map(|codes| Cond::Status { op, codes }),
        "creator" | "mapper" | "author" => text(Field::Creator),
        "artist" => text(Field::Artist),
        "title" => text(Field::Title),
        "diff" | "version" => text(Field::Version),
        "tag" | "tags" => text(Field::Tags),
        "source" => text(Field::Source),
        "collection" | "col" => match op {
            Op::Eq | Op::Ne if !value.is_empty() => Some(Cond::Collection {
                negated: op == Op::Ne,
                name: if value.eq_ignore_ascii_case("none") {
                    InCollection::None
                } else {
                    InCollection::Named(value.to_lowercase())
                },
            }),
            _ => None,
        },
        _ => None,
    }
}

/// `=` and `!=` go by the precision typed; the others by the number itself.
fn compare(op: Op, have: f64, value: f64, tolerance: f64) -> bool {
    let equal = (have - value).abs() < tolerance;
    match op {
        Op::Eq => equal,
        Op::Ne => !equal,
        Op::Lt => have < value,
        Op::Le => have <= value,
        Op::Gt => have > value,
        Op::Ge => have >= value,
    }
}

/// Normalized once when an entry is read; metadata reloads replace the entry and its keys.
#[derive(Clone, Debug)]
pub struct Text {
    label: String,
    creator: String,
    pub artist: String,
    pub title: String,
    /// The artist and title in their own script, found as well as the romanised ones: only for
    /// a map that has them (most don't), so the search over a big library reads no more.
    unicode: Option<Box<[String; 2]>>,
    version: String,
    tags: String,
    source: String,
}

impl Text {
    pub fn new(meta: &crate::beatmap::Meta, label: &str) -> Self {
        Self {
            label: label.to_lowercase(),
            creator: meta.creator.to_lowercase(),
            artist: meta.artist.to_lowercase(),
            title: meta.title.to_lowercase(),
            unicode: meta
                .original
                .as_ref()
                .map(|o| Box::new([o.artist.to_lowercase(), o.title.to_lowercase()])),
            version: meta.version.to_lowercase(),
            tags: meta.tags.to_lowercase(),
            source: meta.source.to_lowercase(),
        }
    }
}

impl Query {
    /// Whether the query asks about the EPIC mark (`Facts::epic`): working it out for every map
    /// costs a search, so a client leaves it false when nothing asks.
    pub fn asks_epic(&self) -> bool {
        self.conds.iter().any(|c| matches!(c, Cond::Epic { .. }))
    }

    /// Whether the query filters on maps' ranked statuses (`status=`): what osu!.db or the
    /// mirrors say of them changes what it lists.
    pub fn asks_status(&self) -> bool {
        self.conds.iter().any(|c| matches!(c, Cond::Status { .. }))
    }

    /// Whether the query filters on when maps were last played (`played`, `unplayed`).
    pub fn asks_played(&self) -> bool {
        self.conds.iter().any(|c| {
            matches!(
                c,
                Cond::Unplayed
                    | Cond::Num {
                        key: Num::Played,
                        ..
                    }
            )
        })
    }

    pub fn parse(s: &str) -> Query {
        let mut q = Query::default();
        for t in tokens(s) {
            if t.eq_ignore_ascii_case("unplayed") {
                q.conds.push(Cond::Unplayed);
                continue;
            }
            match split(&t).and_then(|(key, op, value)| cond(key, op, value)) {
                Some(c) => q.conds.push(c),
                None => q.words.push(t.to_lowercase()),
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.conds.is_empty()
    }

    /// Whether `matches_text` reads its `Facts`: without such filters (words, text fields,
    /// collections) any facts do.
    pub fn needs_facts(&self) -> bool {
        self.conds.iter().any(|c| {
            matches!(
                c,
                Cond::Num { .. } | Cond::Status { .. } | Cond::Epic { .. } | Cond::Unplayed
            )
        })
    }

    /// The `collection=` filters (`!=` when negated), for the caller to apply: `matches_text`
    /// lets everything through them.
    pub fn collections(&self) -> impl Iterator<Item = (&InCollection, bool)> {
        self.conds.iter().filter_map(|c| match c {
            Cond::Collection { negated, name } => Some((name, *negated)),
            _ => None,
        })
    }

    /// Filters on what the background analysis works out (stars, length, BPM).
    pub fn needs_info(&self) -> bool {
        self.conds.iter().any(|c| {
            matches!(
                c,
                Cond::Num {
                    key: Num::Stars | Num::Length | Num::Bpm,
                    ..
                }
            )
        })
    }

    #[cfg(test)]
    pub fn matches(&self, meta: &crate::beatmap::Meta, label: &str, facts: &Facts) -> bool {
        self.matches_text(meta, &Text::new(meta, label), facts)
    }

    pub fn matches_text(&self, meta: &crate::beatmap::Meta, text: &Text, facts: &Facts) -> bool {
        let words = self.words.iter().all(|w| {
            let w = w.as_str();
            text.label.contains(w)
                || text
                    .unicode
                    .as_ref()
                    .is_some_and(|u| u.iter().any(|n| n.contains(w)))
                || text.creator.contains(w)
                || text.tags.contains(w)
                || text.source.contains(w)
                || w.parse::<u32>()
                    .is_ok_and(|n| meta.beatmap_id == Some(n) || meta.set_id == Some(n))
        });
        words
            && self.conds.iter().all(|c| match c {
                Cond::Num {
                    key,
                    op,
                    value,
                    tolerance,
                } => {
                    let have = match key {
                        Num::Stars => facts.stars,
                        Num::Od => Some(meta.od),
                        Num::Hp => Some(meta.hp),
                        Num::Bpm => facts.bpm,
                        Num::Length => facts.length_s,
                        Num::Played => facts.played_days,
                        Num::BeatmapId => meta.beatmap_id.map(f64::from),
                        Num::SetId => meta.set_id.map(f64::from),
                    };
                    have.is_some_and(|h| compare(*op, h, *value, *tolerance))
                }
                Cond::Text {
                    field,
                    negated,
                    value,
                } => {
                    let (have, also) = match field {
                        Field::Creator => (&text.creator, None),
                        Field::Artist => (&text.artist, text.unicode.as_ref().map(|u| &u[0])),
                        Field::Title => (&text.title, text.unicode.as_ref().map(|u| &u[1])),
                        Field::Version => (&text.version, None),
                        Field::Tags => (&text.tags, None),
                        Field::Source => (&text.source, None),
                    };
                    let v = value.as_str();
                    (have.contains(v) || also.is_some_and(|a| !a.is_empty() && a.contains(v)))
                        != *negated
                }
                Cond::Status { op, codes } => match op {
                    Op::Eq => codes.contains(&facts.status),
                    Op::Ne => !codes.contains(&facts.status),
                    // Ordered as osu! has them (graveyard with pending).
                    _ => {
                        let rank = |c: u8| {
                            if c == GRAVEYARD {
                                PENDING as f64
                            } else if c >= CUSTOM {
                                // Not on osu!'s ladder: with the maps that aren't submitted.
                                NOT_SUBMITTED as f64
                            } else {
                                c as f64
                            }
                        };
                        compare(*op, rank(facts.status), rank(codes[0]), 0.5)
                    }
                },
                Cond::Epic { negated } => facts.epic != *negated,
                Cond::Unplayed => facts.played_days.is_none(),
                Cond::Collection { .. } => true,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beatmap::Meta;

    #[test]
    fn only_filters_on_numbers_status_epic_or_plays_need_the_maps_facts() {
        for (typed, needs) in [
            ("", false),
            ("artist title", false),
            ("artist=camellia creator=nyanko", false),
            ("collection=\"warm up\"", false),
            ("stars>5", true),
            ("length<90 bpm>200", true),
            ("status=ranked", true),
            ("status=epic", true),
            ("unplayed", true),
            ("artist stars<3", true),
        ] {
            assert_eq!(Query::parse(typed).needs_facts(), needs, "{typed:?}");
        }
        // Words alone match the same with no facts at all.
        let meta = Meta {
            artist: "Camellia".into(),
            title: "Ghost".into(),
            ..Default::default()
        };
        let q = Query::parse("camellia");
        assert!(q.matches(&meta, "Camellia - Ghost", &Facts::default()));
    }

    fn meta() -> Meta {
        Meta {
            artist: "Camellia".into(),
            title: "Ghost".into(),
            creator: "Mapper".into(),
            version: "Inner Oni".into(),
            od: 6.5,
            hp: 5.0,
            ..Default::default()
        }
    }

    fn facts() -> Facts {
        Facts {
            stars: Some(5.23),
            length_s: Some(150.0),
            bpm: Some(180.0),
            played_days: Some(3.0),
            status: RANKED,
            epic: false,
        }
    }

    fn hit(q: &str) -> bool {
        Query::parse(q).matches(&meta(), "Camellia - Ghost [Inner Oni]", &facts())
    }

    #[test]
    fn numbers_compare_to_the_precision_typed() {
        for q in [
            "sr>5",
            "stars>=5.23",
            "sr=5",
            "sr=5.2",
            "star:5.23",
            "sr<6",
            "sr!=4",
            "od=6.5",
            "hp<=5",
            "bpm=180",
            "bpm>179.5",
        ] {
            assert!(hit(q), "{q}");
        }
        for q in ["sr>6", "sr=5.3", "sr<5", "sr!=5", "od>7", "hp>5", "bpm<180"] {
            assert!(!hit(q), "{q}");
        }
    }

    #[test]
    fn collection_filters_are_parsed_for_the_caller_and_pass_here() {
        let q = Query::parse(r#"ghost collection="Warm Up" col!=dans collection=NONE"#);
        assert_eq!(
            q.collections().collect::<Vec<_>>(),
            vec![
                (&InCollection::Named("warm up".into()), false),
                (&InCollection::Named("dans".into()), true),
                (&InCollection::None, false),
            ]
        );
        assert!(q.matches(&meta(), "Camellia - Ghost [Inner Oni]", &facts()));
        // Without a name it's just a word.
        let q = Query::parse("collection=");
        assert_eq!(q.collections().count(), 0);
        assert!(!q.is_empty());
    }

    #[test]
    fn lengths_take_units_and_minutes() {
        for q in [
            "length=150",
            "length=2:30",
            "len=2m30s",
            "length>2m",
            "length<3m",
            "length=150000ms",
            "length<=150s",
        ] {
            assert!(hit(q), "{q}");
        }
        for q in ["length>2:30", "length<1m", "length=3m"] {
            assert!(!hit(q), "{q}");
        }
    }

    #[test]
    fn statuses_by_letter_word_or_order() {
        for q in [
            "status=r",
            "status=ranked",
            "status:rank",
            "status!=l",
            "status>=r",
            "status<l",
            "status>p",
        ] {
            assert!(hit(q), "{q}");
        }
        for q in ["status=l", "status=g", "status>r"] {
            assert!(!hit(q), "{q}");
        }
        // osu!.db's pending could be either.
        let pending = Facts {
            status: PENDING,
            ..facts()
        };
        assert!(Query::parse("status=g").matches(&meta(), "", &pending));
        assert!(Query::parse("status=p").matches(&meta(), "", &pending));
        let graveyard = Facts {
            status: GRAVEYARD,
            ..facts()
        };
        assert!(!Query::parse("status=p").matches(&meta(), "", &graveyard));
        assert!(Query::parse("status<r").matches(&meta(), "", &graveyard));
    }

    #[test]
    fn epic_goes_by_the_mark_on_top_of_the_status() {
        let epic = Facts {
            epic: true,
            ..facts()
        };
        for q in ["status=epic", "status=e", "STATUS:Ep", "status=r"] {
            assert!(Query::parse(q).matches(&meta(), "", &epic), "{q}");
        }
        assert!(!Query::parse("status!=epic").matches(&meta(), "", &epic));
        assert!(Query::parse("artist status!=e").asks_epic());
        assert!(!Query::parse("epic status=r").asks_epic());
        assert!(!hit("status=epic"));
        assert!(hit("status!=epic"));
        // Only `=` and `!=` mean anything for a mark: the rest is a word to look for.
        assert!(!hit("status>epic"));
        // A plain word still finds a name with it in.
        let label = "Camellia - Epic Ghost [Inner Oni]";
        assert!(Query::parse("epic").matches(&meta(), label, &facts()));
    }

    #[test]
    fn text_fields_words_and_quotes() {
        for q in [
            "ghost",
            "GHOST cam",
            "mapper",
            "creator=map",
            "artist=camellia title=ghost",
            "diff=oni",
            "artist!=xi",
            "title=\"gho\"",
            "sr>5 ghost status=r",
        ] {
            assert!(hit(q), "{q}");
        }
        for q in [
            "ghost xi",
            "artist=xi",
            "creator!=mapper",
            "diff=\"muzukashii\"",
        ] {
            assert!(!hit(q), "{q}");
        }
        // Unknown keys and odd values are just text.
        assert_eq!(Query::parse("ar=9").words, ["ar=9"]);
        assert_eq!(Query::parse("sr>abc").words, ["sr>abc"]);
        assert!(Query::parse("").is_empty());
    }

    fn tagged() -> Meta {
        Meta {
            tags: "Vocaloid Touhou  Speedcore".into(),
            source: "Touhou Project".into(),
            set_id: Some(456),
            beatmap_id: Some(123),
            ..meta()
        }
    }

    fn hit_tagged(q: &str) -> bool {
        Query::parse(q).matches(&tagged(), "Camellia - Ghost [Inner Oni]", &facts())
    }

    #[test]
    fn words_find_tags_and_source() {
        for q in [
            "vocaloid",
            "SPEEDCORE ghost",
            "touhou",
            "project",
            "tag=vocal",
            "tags:touhou",
            "source=\"touhou proj\"",
            "source!=anime",
        ] {
            assert!(hit_tagged(q), "{q}");
        }
        for q in ["anime", "tag=anime", "source=vocaloid", "tag!=touhou"] {
            assert!(!hit_tagged(q), "{q}");
        }
        // A map without tags doesn't match them, and nothing breaks.
        assert!(!hit("vocaloid") && !hit("source=touhou"));
    }

    #[test]
    fn numbers_find_beatmap_and_set_ids() {
        for q in [
            "123",
            "456",
            "id=123",
            "set=456",
            "setid:456",
            "bid=123",
            "id<200",
            "set>=456",
            "ghost 123",
        ] {
            assert!(hit_tagged(q), "{q}");
        }
        // Exact: 12 is not 123, and a set ID isn't a map ID.
        for q in ["12", "1234", "id=456", "set=123", "id>123", "id=abc"] {
            assert!(!hit_tagged(q), "{q}");
        }
        // No ID known: no match, and the number is still text.
        assert!(!hit("123") && !hit("id=1"));
        let titled = Meta {
            title: "Route 66".into(),
            ..meta()
        };
        assert!(Query::parse("66").matches(&titled, "Camellia - Route 66 [Oni]", &facts()));
    }

    #[test]
    fn a_cache_from_before_tags_still_loads() {
        let old = r#"{"title":"T","artist":"A","creator":"C","version":"V","audio_file":"a.mp3","preview_time":0.0,"audio_lead_in":0.0,"mode":1,"od":5.0,"hp":5.0,"background":null,"slider_multiplier":1.4,"slider_tick_rate":1.0,"format_version":14,"set_id":7,"beatmap_id":8}"#;
        let m: Meta = serde_json::from_str(old).unwrap();
        assert!(m.tags.is_empty() && m.source.is_empty());
        assert!(Query::parse("7").matches(&m, "A - T [V]", &facts()));
    }

    /// A map is found by its names in their own script as well as by the romanised ones.
    #[test]
    fn names_in_their_own_script_are_found_too() {
        let m = Meta {
            title: "Tolko zvyozdy nad nami".into(),
            artist: "hxvvxn".into(),
            original: Some(Box::new(crate::beatmap::OriginalNames {
                title: "только звёзды над нами".into(),
                artist: "хввхн".into(),
            })),
            ..meta()
        };
        let label = "hxvvxn - Tolko zvyozdy nad nami [Oni]";
        for q in [
            "звёзды",
            "tolko",
            "title=звёзды",
            "title=tolko",
            "artist=хввхн",
        ] {
            assert!(Query::parse(q).matches(&m, label, &facts()), "{q}");
        }
        assert!(!Query::parse("title!=звёзды").matches(&m, label, &facts()));
        assert!(!Query::parse("artist=звёзды").matches(&m, label, &facts()));
    }

    #[test]
    fn played_and_unplayed() {
        assert!(hit("played<7") && !hit("played>7") && !hit("unplayed"));
        let never = Facts {
            played_days: None,
            ..facts()
        };
        assert!(Query::parse("unplayed").matches(&meta(), "", &never));
        assert!(!Query::parse("played>7").matches(&meta(), "", &never));
    }

    #[test]
    fn status_and_played_filters_are_told_apart_in_any_case() {
        for q in ["status=ranked", "STATUS=Loved", "ar status!=graveyard"] {
            assert!(Query::parse(q).asks_status(), "{q}");
            assert!(!Query::parse(q).asks_played(), "{q}");
        }
        for q in ["unplayed", "UNPLAYED", "Unplayed", "played<7", "PLAYED>30"] {
            assert!(Query::parse(q).asks_played(), "{q}");
            assert!(!Query::parse(q).asks_status(), "{q}");
        }
        // Words that only contain the keys, and the EPIC mark, ask neither.
        for q in ["statusquo", "replayed", "status=epic", "sr>5"] {
            let p = Query::parse(q);
            assert!(!p.asks_status() && !p.asks_played(), "{q}");
        }
    }

    #[test]
    fn unknown_numbers_dont_match_until_known() {
        let analysing = Facts {
            stars: None,
            ..facts()
        };
        assert!(!Query::parse("sr>1").matches(&meta(), "", &analysing));
        assert!(Query::parse("sr>1").needs_info() && !Query::parse("od>1 status=r").needs_info());
    }
}
