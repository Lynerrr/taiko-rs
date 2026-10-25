//! The website's pages, laid out like osu!'s: a header bar, rankings, player profiles with their
//! cover, stats and plays, beatmap leaderboards and the download page, and on the home page clips
//! of the mods osu! doesn't have.

use crate::online::{self, OnlineScore, Profile};
use crate::scores::{MapRow, Ranked};

const REPO: &str = "https://github.com/Lynerrr/taiko-rs";
const RAW: &str = "https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client";

/// A clip on the home page: what it shows, the mods on, and its file under /clips/.
struct Clip {
    file: &'static str,
    title: &'static str,
    mods: &'static [&'static str],
    what: &'static str,
}

const CLIPS: [Clip; 6] = [
    Clip { file: "multiverse", title: "Multiverse", mods: &["MV"], what: "Eight playfields circle about, each with its own look and mods. The outlined one is live; the next one is already on its way." },
    Clip { file: "loop", title: "Loop + Crumble", mods: &["LP", "CR"], what: "A loop-the-loop in the middle of the lane, and every hit knocks a piece out of it." },
    Clip { file: "shapeshift", title: "Shapeshift", mods: &["SS", "CR"], what: "Every few bars the lane bends into another shape: a loop, a hook, a straight again." },
    Clip { file: "portal", title: "Portal + Barrel Roll", mods: &["AC", "LP", "SK", "BA", "PT"], what: "Notes dive into one portal and come out of another, streams slither and every kiai spins the lane." },
    Clip { file: "crumble", title: "Crumble", mods: &["CR"], what: "Hits knock the lane apart, harder as the combo grows; a miss puts it back together." },
    Clip { file: "shatter", title: "Shatter + Decoy", mods: &["BR", "WV", "DY", "LP", "SH", "CK"], what: "The lane breaks into pieces that slide apart, and see-through fakes fly between the real notes." },
];

/// The clips' files, served under /clips/ (`web.rs`).
pub const CLIP_FILES: [(&str, &[u8]); 12] = [
    ("multiverse.mp4", include_bytes!("../site/clips/multiverse.mp4")),
    ("multiverse.jpg", include_bytes!("../site/clips/multiverse.jpg")),
    ("loop.mp4", include_bytes!("../site/clips/loop.mp4")),
    ("loop.jpg", include_bytes!("../site/clips/loop.jpg")),
    ("shapeshift.mp4", include_bytes!("../site/clips/shapeshift.mp4")),
    ("shapeshift.jpg", include_bytes!("../site/clips/shapeshift.jpg")),
    ("portal.mp4", include_bytes!("../site/clips/portal.mp4")),
    ("portal.jpg", include_bytes!("../site/clips/portal.jpg")),
    ("crumble.mp4", include_bytes!("../site/clips/crumble.mp4")),
    ("crumble.jpg", include_bytes!("../site/clips/crumble.jpg")),
    ("shatter.mp4", include_bytes!("../site/clips/shatter.mp4")),
    ("shatter.jpg", include_bytes!("../site/clips/shatter.jpg")),
];

/// Text made safe for HTML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// 1234567 as "1,234,567".
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn ago(t: u64) -> String {
    let s = crate::now_secs().saturating_sub(t);
    let (n, unit) = match s {
        0..=59 => return "just now".into(),
        60..=3599 => (s / 60, "minute"),
        3600..=86_399 => (s / 3600, "hour"),
        86_400..=2_591_999 => (s / 86_400, "day"),
        2_592_000..=31_535_999 => (s / 2_592_000, "month"),
        _ => (s / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// "October 2026" for a Unix time.
fn month_year(t: u64) -> String {
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = (t / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{} {year}", MONTHS[(month - 1) as usize])
}

/// "Artist - Title [Version]" split into (artist - title, version).
fn split_map(name: &str) -> (&str, &str) {
    match name.rfind(" [") {
        Some(i) if name.ends_with(']') => (&name[..i], &name[i + 2..name.len() - 1]),
        _ => (name, ""),
    }
}

/// "Artist - Title" split into (artist, title).
fn split_song(song: &str) -> (&str, &str) {
    song.split_once(" - ").unwrap_or(("", song))
}

fn cover(set: Option<u32>, size: &str) -> Option<String> {
    set.map(|s| format!("https://assets.ppy.sh/beatmaps/{s}/covers/{size}.jpg"))
}

/// A map's own hue, for a cover it doesn't have.
fn hue(hash: u64) -> u64 {
    hash % 360
}

/// The style for a block with a map's cover behind it: its hue, and the picture if there is one.
fn cover_style(set: Option<u32>, hash: u64, size: &str) -> String {
    let img = cover(set, size).map(|u| format!(";--cover:url('{u}')")).unwrap_or_default();
    format!(" style=\"--h:{}{img}\"", hue(hash))
}

/// A player's picture: their initial on a colour of their own.
fn avatar(name: &str, class: &str) -> String {
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b.to_ascii_lowercase() as u32)) % 360;
    let initial = name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
    format!("<span class=\"avatar {class}\" style=\"--h:{hue}\" aria-hidden=true>{}</span>", esc(&initial.to_string()))
}

fn player_link(name: &str) -> String {
    format!("<a class=player href=\"/u/{0}\">{0}</a>", esc(name))
}

fn grade(g: &str, class: &str) -> String {
    format!("<span class=\"grade g{} {class}\">{g}</span>", g.to_lowercase())
}

/// What a mod does to a play, for its colour.
fn mod_kind(acronym: &str) -> &'static str {
    match acronym {
        "EZ" | "NF" | "HT" | "DC" | "NB" => "easy",
        "HR" | "SD" | "PF" | "DT" | "NC" | "HD" | "FL" | "PR" | "GN" => "hard",
        "AT" | "RX" | "CN" | "AP" => "auto",
        _ => "fun",
    }
}

fn mod_badges<S: AsRef<str>>(m: &[S], rate: f64) -> String {
    let mut out: String = m.iter().map(|a| format!("<span class=\"mod {}\">{}</span>", mod_kind(a.as_ref()), esc(a.as_ref()))).collect();
    if (rate - 1.0).abs() > 1e-6 {
        out += &format!("<span class=\"mod rate\">{rate:.2}x</span>");
    }
    out
}

/// osu!'s colour for a star rating: blue for easy through red and purple to black.
fn star_colour(s: f64) -> [u8; 3] {
    const STOPS: [(f64, [u8; 3]); 11] = [
        (0.1, [0x42, 0x90, 0xfb]),
        (1.25, [0x4f, 0xc0, 0xff]),
        (2.0, [0x4f, 0xff, 0xd5]),
        (2.5, [0x7c, 0xff, 0x4f]),
        (3.3, [0xf6, 0xf0, 0x5c]),
        (4.2, [0xff, 0x80, 0x68]),
        (4.9, [0xff, 0x4e, 0x6f]),
        (5.8, [0xc6, 0x45, 0xb8]),
        (6.7, [0x65, 0x63, 0xde]),
        (7.7, [0x18, 0x15, 0x8e]),
        (9.0, [0x00, 0x00, 0x00]),
    ];
    if s <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if s <= b {
            let t = (s - a) / (b - a);
            return [0, 1, 2].map(|i| (ca[i] as f64 + (cb[i] as f64 - ca[i] as f64) * t).round() as u8);
        }
    }
    STOPS[STOPS.len() - 1].1
}

fn stars(s: Option<f64>) -> String {
    s.map(|s| {
        let [r, g, b] = star_colour(s);
        let text = if s >= 6.5 { "#ffd966" } else { "rgba(0,0,0,.8)" };
        format!("<span class=stars style=\"background:#{r:02x}{g:02x}{b:02x};color:{text}\">&#9733; {s:.2}</span>")
    })
    .unwrap_or_default()
}

/// The visitor's system, from their browser's User-Agent.
#[derive(Clone, Copy, PartialEq)]
pub enum Os {
    Windows,
    Linux,
    Android,
    Other,
}

impl Os {
    pub fn of(user_agent: &str) -> Os {
        let ua = user_agent.to_ascii_lowercase();
        if ua.contains("android") {
            Os::Android
        } else if ua.contains("windows") {
            Os::Windows
        } else if ua.contains("linux") || ua.contains("x11") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

struct Build {
    os: Os,
    name: &'static str,
    file: &'static str,
    note: &'static str,
}

const BUILDS: [Build; 3] = [
    Build { os: Os::Windows, name: "Windows", file: "taiko-rs-windows-x64.zip", note: "Windows 10 or 11, 64-bit" },
    Build { os: Os::Linux, name: "Linux", file: "taiko-rs-linux-x64.tar.gz", note: "64-bit, glibc 2.39+ (Ubuntu 24.04, Debian 13, Fedora 40, Arch...)" },
    Build { os: Os::Android, name: "Android", file: "taiko-rs-android.apk", note: "Android 8.0+, 64-bit ARM" },
];

fn download_url(file: &str) -> String {
    format!("{REPO}/releases/download/latest/{file}")
}

/// The big download button for the visitor's system (or a pointer to the choices). On the
/// download page itself the choices are right below.
fn download_cta(os: Os, on_download_page: bool) -> String {
    let others = if on_download_page { String::new() } else { " &middot; <a href=\"/download\">other systems</a>".into() };
    match BUILDS.iter().find(|b| b.os == os) {
        Some(b) => format!(
            "<a class=\"btn big\" href=\"{}\">Download for {}</a><div class=hint>{}{others}</div>",
            download_url(b.file),
            b.name,
            b.note
        ),
        None if on_download_page => "<p class=dim>Pick your system below.</p>".into(),
        None => "<a class=\"btn big\" href=\"/download\">Download taiko-rs</a><div class=hint>Windows, Linux and Android</div>".into(),
    }
}

/// The clips of taiko-rs's own mods: the first one big, the rest around it.
fn showcase() -> String {
    let clips: String = CLIPS
        .iter()
        .map(|c| {
            format!(
                "<figure class=clip><div class=frame><img src=\"/clips/{0}.jpg\" alt=\"\" width=960 height=540 loading=lazy>\
                 <video src=\"/clips/{0}.mp4\" poster=\"/clips/{0}.jpg\" width=960 height=540 autoplay muted loop playsinline preload=metadata aria-label=\"{1} gameplay\"></video></div>\
                 <figcaption><div class=clip-head><b>{1}</b><span class=mods>{2}</span></div><p>{3}</p></figcaption></figure>",
                c.file,
                c.title,
                mod_badges(c.mods, 1.0),
                c.what
            )
        })
        .collect();
    format!(
        "<section class=showcase><div class=section-head><h2>Mods osu! doesn't have</h2><p class=dim>70+ mods, and some of them bend, break and multiply the lane. Every clip is a real play, recorded in the game.</p></div>\
         <div class=clips>{clips}</div></section>"
    )
}

pub fn home(rankings: &[Ranked], maps: &[MapRow], players: usize, plays: usize, os: Os) -> String {
    let top: String = rankings.iter().take(10).map(top_player).collect();
    let top = if top.is_empty() { "<p class=dim>Nobody has sent a play yet.</p>".into() } else { format!("<ol class=top>{top}</ol>") };
    let hero_cover = maps.iter().find_map(|m| cover(m.set_id, "cover@2x")).map(|u| format!(" style=\"--cover:url('{u}')\"")).unwrap_or_default();
    format!(
        "<section class=hero{hero_cover}><div class=inner><div class=hero-text><span class=eyebrow>osu!taiko client</span><h1>taiko-rs</h1>\
         <p>osu!taiko, rebuilt: low-latency audio, your osu! library and skins, 70+ mods, online lobbies, scores and pp.</p>{}</div>\
         <dl class=hero-stats><div><dt>players ranked</dt><dd>{}</dd></div><div><dt>plays sent</dt><dd>{}</dd></div></dl></div></section>\
         <div class=page>{}<div class=cols-home>\
         <section class=box><h2>Recently played <a class=more href=\"/beatmaps\">all beatmaps</a></h2>{}</section>\
         <section class=box><h2>Top players <a class=more href=\"/rankings\">all rankings</a></h2>{top}</section></div></div>",
        download_cta(os, false),
        thousands(players as u64),
        thousands(plays as u64),
        showcase(),
        map_cards(maps),
    )
}

/// A player in the home page's top list.
fn top_player(r: &Ranked) -> String {
    format!(
        "<li><span class=rank>#{}</span>{}<span class=name>{}</span><span class=pp>{}pp</span></li>",
        r.rank,
        avatar(&r.name, "small"),
        player_link(&r.name),
        thousands(r.pp.round() as u64)
    )
}

fn ranking_head() -> &'static str {
    "<thead><tr><th></th><th></th><th class=r>Accuracy</th><th class=\"r wide\">Play Count</th><th class=r>Performance</th><th class=\"r wide\">SS</th><th class=\"r wide\">S</th><th class=\"r wide\">A</th></tr></thead>"
}

fn ranking_row(r: &Ranked) -> String {
    format!(
        "<tr><td class=rank>#{}</td><td>{}{}</td><td class=\"r dim\">{:.2}%</td><td class=\"r dim wide\">{}</td><td class=\"r pp\">{}pp</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td></tr>",
        r.rank,
        avatar(&r.name, "small"),
        player_link(&r.name),
        r.accuracy * 100.0,
        thousands(r.plays as u64),
        thousands(r.pp.round() as u64),
        r.ss,
        r.s,
        r.a
    )
}

pub fn rankings(rows: &[Ranked]) -> String {
    let body: String = rows.iter().map(ranking_row).collect();
    let table = if body.is_empty() { "<p class=dim>Nobody has sent a play yet.</p>".into() } else { format!("<div class=scroll><table class=list>{}<tbody>{body}</tbody></table></div>", ranking_head()) };
    format!("<div class=band><div class=inner><h1>Performance Ranking</h1><p class=dim>Every player by their total pp, the best play counting most.</p></div></div><div class=page><section class=box>{table}</section></div>")
}

pub fn profile(p: &Profile) -> String {
    let st = &p.stats;
    let level = online::level(st.total_score);
    let cover = p.top.first().map(|s| cover_style(s.set_id, s.map_hash, "cover@2x")).unwrap_or_default();
    let status = if p.online { "<span class=\"status on\">Online</span>" } else { "<span class=status>Offline</span>" };
    let rank = p.rank.map(|r| format!("#{}", thousands(r as u64))).unwrap_or_else(|| "-".into());
    let stat = |k: &str, v: String| format!("<div class=stat><dt>{k}</dt><dd>{v}</dd></div>");
    let stats = [
        stat("Ranked Score", thousands(st.ranked_score)),
        stat("Hit Accuracy", format!("{:.2}%", st.accuracy * 100.0)),
        stat("Play Count", thousands(p.plays as u64)),
        stat("Total Score", thousands(st.total_score)),
        stat("Total Hits", thousands(st.total_hits)),
        stat("Maximum Combo", thousands(st.max_combo as u64)),
    ]
    .concat();
    let grades = format!(
        "<div class=grades><div>{}<b>{}</b></div><div>{}<b>{}</b></div><div>{}<b>{}</b></div></div>",
        grade("SS", ""),
        st.ss,
        grade("S", ""),
        st.s,
        grade("A", ""),
        st.a
    );
    let best = if p.top.is_empty() {
        "<p class=dim>No performance records yet.</p>".into()
    } else {
        p.top.iter().enumerate().map(|(i, s)| play_row(s, Some(i), false)).collect::<String>()
    };
    let firsts = if p.firsts.is_empty() { "<p class=dim>No first place ranks yet.</p>".into() } else { p.firsts.iter().map(|s| play_row(s, None, false)).collect::<String>() };
    let recent = if p.recent.is_empty() { "<p class=dim>No recent plays.</p>".into() } else { p.recent.iter().map(|s| play_row(s, None, true)).collect::<String>() };
    format!(
        "<div class=cover{cover}></div>\
         <div class=page><div class=\"box head\">\
           <div class=who>{}<div class=id><h1>{}</h1>{status}<div class=dim>Joined {}</div></div></div>\
           <div class=ranks><div class=bigstat><span>Global Ranking</span><b>{rank}</b></div><div class=bigstat><span>Performance</span><b class=pp>{}pp</b></div>\
             <div class=level><span>Level <b>{}</b></span><div class=bar><i style=\"width:{:.0}%\"></i></div></div></div>\
         </div>\
         <div class=cols-side><aside class=\"box side\"><dl>{stats}</dl>{grades}</aside>\
           <div class=main>\
             <section class=box><h2>Best Performance</h2>{best}</section>\
             <section class=box><h2>First Place Ranks <span class=count>{}</span></h2>{firsts}</section>\
             <section class=box><h2>Recent Plays</h2>{recent}</section>\
           </div></div></div>",
        avatar(&p.name, "big"),
        esc(&p.name),
        month_year(p.joined),
        thousands(p.pp.round() as u64),
        level.floor(),
        level.fract() * 100.0,
        p.firsts.len(),
    )
}

/// A play on a profile: grade, map (linked) and when, mods, accuracy and pp (`weighted`: its
/// place among the best, for the 95% weighting).
fn play_row(s: &OnlineScore, weighted: Option<usize>, show_score: bool) -> String {
    let (song, version) = split_map(&s.map);
    let pp = match s.pp {
        Some(pp) => {
            let w = weighted
                .map(|i| {
                    let k = 0.95f64.powi(i as i32);
                    format!("<div class=weight>weighted {:.0}% ({:.0}pp)</div>", k * 100.0, pp * k)
                })
                .unwrap_or_default();
            format!("<div class=pp>{pp:.0}<small>pp</small>{w}</div>")
        }
        None => "<div class=pp>-</div>".into(),
    };
    let score = if show_score { format!("<span class=\"dim score\">{}</span>", thousands(s.score)) } else { String::new() };
    format!(
        "<div class=play>{}<div class=what><a href=\"/m/{:x}\">{}</a><div class=meta>{}<span class=version>{}</span><span class=dim>{}</span></div></div><div class=mods>{}</div>{score}<div class=acc>{:.2}%</div>{pp}</div>",
        grade(s.grade(), ""),
        s.map_hash,
        esc(song),
        stars(s.stars),
        esc(version),
        ago(s.date),
        mod_badges(&s.mods, s.rate),
        s.accuracy * 100.0
    )
}

pub fn map_page(m: &MapRow, rows: &[OnlineScore]) -> String {
    let (song, version) = split_map(&m.name);
    let (artist, title) = split_song(song);
    let osu = m.set_id.map(|s| format!("<a class=\"btn plain small\" href=\"https://osu.ppy.sh/beatmapsets/{s}\">osu! beatmap page</a>")).unwrap_or_default();
    let head = format!(
        "<div class=\"cover map\"{}><div class=inner><div class=artist>{}</div><h1>{}</h1>\
         <div class=sub>{}<span class=version>{}</span><span>{} plays &middot; {} players</span>{osu}</div></div></div>",
        cover_style(m.set_id, m.hash, "cover@2x"),
        esc(artist),
        esc(title),
        stars(m.stars),
        esc(version),
        thousands(m.plays as u64),
        m.players
    );
    let Some(first) = rows.first() else { return format!("{head}<div class=page><section class=box><p class=dim>No scores yet.</p></section></div>") };
    let mods = if first.mods.is_empty() && (first.rate - 1.0).abs() < 1e-6 { "-".into() } else { mod_badges(&first.mods, first.rate) };
    let top = format!(
        "<div class=\"box topscore\"><div class=place>#1</div>{}<div class=who>{}<div>{}<div class=dim>{}</div></div></div>\
         <dl class=tstats><div><dt>Total Score</dt><dd>{}</dd></div><div><dt>Accuracy</dt><dd>{:.2}%</dd></div><div><dt>Max Combo</dt><dd>{}x</dd></div>\
         <div><dt>pp</dt><dd class=pp>{}</dd></div><div><dt>Mods</dt><dd class=mods>{mods}</dd></div></dl></div>",
        grade(first.grade(), "big"),
        avatar(&first.player, "mid"),
        player_link(&first.player),
        ago(first.date),
        thousands(first.score),
        first.accuracy * 100.0,
        thousands(first.max_combo as u64),
        first.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
    );
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                "<tr><td class=rank>#{}</td><td>{}</td><td class=strong>{}</td><td>{:.2}%</td><td>{}{}</td><td class=wide>{}x</td><td class=wide>{}</td><td class=wide>{}</td><td class=wide>{}</td><td class=pp>{}</td><td class=\"wide mods\">{}</td><td class=dim>{}</td></tr>",
                i + 1,
                grade(r.grade(), ""),
                thousands(r.score),
                r.accuracy * 100.0,
                avatar(&r.player, "small"),
                player_link(&r.player),
                thousands(r.max_combo as u64),
                r.great,
                r.ok,
                r.miss,
                r.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
                mod_badges(&r.mods, r.rate),
                ago(r.date)
            )
        })
        .collect();
    format!(
        "{head}<div class=page>{top}<section class=box><div class=scroll><table class=list><thead><tr><th>Rank</th><th></th><th>Score</th><th>Accuracy</th><th>Player</th><th class=wide>Max Combo</th><th class=wide>Great</th><th class=wide>Ok</th><th class=wide>Miss</th><th>pp</th><th class=wide>Mods</th><th></th></tr></thead><tbody>{body}</tbody></table></div></section></div>"
    )
}

/// Beatmaps as cards over their cover.
fn map_cards(maps: &[MapRow]) -> String {
    if maps.is_empty() {
        return "<p class=dim>No maps played yet.</p>".into();
    }
    let cards: String = maps
        .iter()
        .map(|m| {
            let (song, version) = split_map(&m.name);
            let (artist, title) = split_song(song);
            format!(
                "<a class=card href=\"/m/{:x}\"{}><span class=art></span><span class=info><b class=title>{}</b><span class=artist>{}</span>\
                 <span class=meta>{}<span class=version>{}</span></span><span class=foot>{} plays &middot; played {}</span></span></a>",
                m.hash,
                cover_style(m.set_id, m.hash, "card@2x"),
                esc(title),
                esc(artist),
                stars(m.stars),
                esc(version),
                thousands(m.plays as u64),
                ago(m.last)
            )
        })
        .collect();
    format!("<div class=cards>{cards}</div>")
}

pub fn beatmaps(maps: &[MapRow]) -> String {
    format!("<div class=band><div class=inner><h1>Beatmaps</h1><p class=dim>Every map someone has sent a play on, the latest first.</p></div></div><div class=page>{}</div>", map_cards(maps))
}

pub fn download(os: Os) -> String {
    let builds: String = BUILDS
        .iter()
        .map(|b| {
            let mine = b.os == os;
            format!(
                "<div class=\"box build{}\"><h2>{}{}</h2><p class=dim>{}</p><a class=\"btn{}\" href=\"{}\">Download {}</a></div>",
                if mine { " mine" } else { "" },
                b.name,
                if mine { " <span class=yours>your system</span>" } else { "" },
                b.note,
                if mine { "" } else { " plain" },
                download_url(b.file),
                b.file
            )
        })
        .collect();
    format!(
        "<div class=band><div class=inner><h1>Download taiko-rs</h1>{}</div></div>\
         <div class=page><div class=builds>{builds}</div>\
         <section class=box><h2>Install with one command</h2><p class=dim>Installs (or updates) taiko-rs and adds it to your apps; it then updates itself from the game.</p>\
         <h3>Windows (PowerShell)</h3><pre>irm {RAW}/install.ps1 | iex</pre><h3>Linux</h3><pre>curl -fsSL {RAW}/install.sh | sh</pre></section>\
         <p class=dim>No macOS build yet. Source and every build: <a href=\"{REPO}/releases\">GitHub releases</a>.</p></div>",
        download_cta(os, true)
    )
}

pub fn not_found(why: &str) -> String {
    format!("<div class=page><section class=\"box lost\"><h1>Not found</h1><p class=dim>{}.</p><p><a class=btn href=\"/\">Home</a></p></section></div>", esc(why))
}

/// A whole page: `nav` is the header link to mark as the current one ("" for none).
pub fn page(title: &str, nav: &str, body: &str) -> String {
    let links: String = [("rankings", "/rankings"), ("beatmaps", "/beatmaps"), ("download", "/download")]
        .iter()
        .map(|(name, href)| {
            let on = if *name == nav { " class=on aria-current=page" } else { "" };
            format!("<a href=\"{href}\"{on}>{name}</a>")
        })
        .collect();
    format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{} · taiko-rs</title>
<meta name=theme-color content=#1c1719><meta name=color-scheme content=dark>
<link rel=preconnect href="https://fonts.googleapis.com"><link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=Nunito:wght@400;600;700;800;900&display=swap"><style>{STYLE}</style></head>
<body><nav><div class=inner><a class=logo href="/" aria-label="taiko-rs home"><span aria-hidden=true>rs</span>taiko-rs</a><div class=links>{links}</div>
<form action="/u" role=search><input name=name placeholder="search players" maxlength=16 aria-label="Search players"></form></div></nav>
<main>{}</main><footer><div class=inner><span class=brand>taiko-rs</span><a href="{REPO}">GitHub</a><a href="/download">Download</a><span class=dim>Map covers from osu!</span></div></footer></body></html>"#,
        esc(title),
        body
    )
}

const STYLE: &str = r#"
:root{--hue:333;--bg:hsl(var(--hue) 10% 10%);--b6:hsl(var(--hue) 10% 8%);--b5:hsl(var(--hue) 10% 15%);--b4:hsl(var(--hue) 10% 19%);--b3:hsl(var(--hue) 10% 24%);--b2:hsl(var(--hue) 10% 30%);
--line:hsl(var(--hue) 10% 22%);--text:hsl(var(--hue) 20% 95%);--dim:hsl(var(--hue) 10% 70%);--pink:hsl(var(--hue) 100% 70%);--pink2:hsl(var(--hue) 100% 80%);--btn:hsl(var(--hue) 72% 46%);--btn2:hsl(var(--hue) 72% 53%);--green:#b3d944;
--r:10px;--ease:cubic-bezier(.2,.7,.3,1)}
*{box-sizing:border-box}html{background:var(--bg);color-scheme:dark}
body{margin:0;color:var(--text);background:var(--bg);font:16px/1.5 Nunito,system-ui,sans-serif;font-variant-numeric:tabular-nums;-webkit-font-smoothing:antialiased}
a{color:var(--text);text-decoration:none;transition:color .15s}a:hover{color:var(--pink2)}.dim{color:var(--dim)}h1,h2,h3{margin:0;font-weight:800;line-height:1.2}p{margin:.4em 0}
:focus-visible{outline:2px solid var(--pink);outline-offset:2px;border-radius:4px}
.inner,.page{max-width:1240px;margin:0 auto;padding:0 24px}.page{padding-top:24px;padding-bottom:48px}main{min-height:60vh}
nav{background:var(--b6);border-bottom:1px solid var(--line);position:sticky;top:0;z-index:10}
nav .inner{display:flex;align-items:center;gap:28px;height:60px}.links{display:flex;gap:4px;height:100%}
.links a{display:flex;align-items:center;padding:0 12px;font-weight:700;color:var(--dim);box-shadow:inset 0 -3px transparent;transition:color .15s,box-shadow .15s}
.links a:hover{color:var(--text)}.links a.on{color:var(--text);box-shadow:inset 0 -3px var(--pink)}
.logo{display:flex;align-items:center;gap:10px;color:var(--text)!important;font-size:20px;font-weight:900}
.logo span{display:grid;place-items:center;width:36px;height:36px;border-radius:50%;background:var(--pink);color:#2a0f1c;font-size:14px;font-weight:900}
nav form{margin-left:auto}nav input{font:inherit;font-size:15px;width:240px;height:38px;background:var(--b4);border:1px solid var(--line);border-radius:19px;color:var(--text);padding:0 16px;transition:border-color .15s}
nav input::placeholder{color:var(--dim)}nav input:focus{outline:none;border-color:var(--pink)}
.hero{position:relative;padding:72px 0 56px;background:linear-gradient(0deg,var(--bg),transparent 45%),linear-gradient(90deg,hsl(var(--hue) 10% 10%/.97) 15%,hsl(var(--hue) 10% 10%/.85) 50%,hsl(var(--hue) 10% 10%/.7)),var(--cover,linear-gradient(135deg,hsl(var(--hue) 40% 22%),hsl(250 30% 16%))) center/cover}
.hero .inner{display:flex;align-items:flex-end;justify-content:space-between;gap:40px;flex-wrap:wrap}.hero-text{max-width:640px}
.eyebrow{display:inline-block;font-size:13px;font-weight:800;letter-spacing:.12em;text-transform:uppercase;color:var(--pink)}
.hero h1{font-size:56px;font-weight:900;letter-spacing:-.01em;margin:4px 0 8px}.hero p{color:var(--dim);font-size:18px;max-width:560px}
.hero-stats{display:flex;gap:36px;margin:0}.hero-stats div{display:flex;flex-direction:column-reverse}.hero-stats dd{margin:0;font-size:34px;font-weight:900;line-height:1}.hero-stats dt{color:var(--dim);font-size:14px;margin-top:6px}
.btn{display:inline-flex;align-items:center;min-height:44px;background:var(--btn);color:#fff!important;font-weight:800;padding:0 24px;border-radius:22px;margin-top:14px;transition:background .15s,transform .15s var(--ease)}
.btn:hover{background:var(--btn2)}.btn:active{transform:scale(.97)}.btn.big{font-size:19px;min-height:52px;padding:0 32px;border-radius:26px}
.btn.plain{background:var(--b3)}.btn.plain:hover{background:var(--b2)}.btn.small{min-height:34px;font-size:14px;padding:0 16px;margin:0}
.hint{color:var(--dim);font-size:14px;margin-top:10px}
.band{background:linear-gradient(180deg,var(--b5),var(--bg));border-bottom:1px solid var(--line);padding:36px 0 24px}.band h1{font-size:34px;font-weight:900}
.box{background:var(--b5);border-radius:var(--r);padding:20px 22px;margin-bottom:20px}
.box h2{font-size:19px;margin-bottom:14px;display:flex;align-items:baseline;gap:10px}
.more{margin-left:auto;font-size:14px;font-weight:700;color:var(--pink)}.count{color:var(--dim);font-size:16px}
.section-head{margin:8px 0 18px}.section-head h2{font-size:28px;font-weight:900}.section-head p{max-width:640px}
.clips{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:18px;margin-bottom:28px}
.clip{margin:0;background:var(--b5);border-radius:var(--r);overflow:hidden;display:flex;flex-direction:column}.clip:first-child{grid-column:span 2;grid-row:span 2}
.frame{position:relative;aspect-ratio:16/9;background:#000}.frame img,.frame video{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;display:block}
.clip figcaption{padding:14px 18px 16px}.clip-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.clip-head b{font-size:17px;font-weight:800}.clip p{color:var(--dim);font-size:14px;margin:4px 0 0}
.clip:first-child .frame{flex:1;aspect-ratio:auto;min-height:300px}.clip:first-child .clip-head b{font-size:22px}.clip:first-child p{font-size:16px}
.cols-home{display:grid;grid-template-columns:minmax(0,1fr) 340px;gap:20px;align-items:start}
.cols-side{display:grid;grid-template-columns:280px minmax(0,1fr);gap:20px;align-items:start}
ol.top{list-style:none;margin:0;padding:0}ol.top li{display:flex;align-items:center;min-height:44px;padding:0 12px;border-radius:8px}ol.top li:nth-child(odd){background:var(--b4)}
ol.top .rank{width:40px;font-weight:800;color:var(--dim)}ol.top .name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}ol.top .pp{font-weight:800}
.scroll{overflow-x:auto}table.list{width:100%;border-collapse:separate;border-spacing:0 4px}
table.list th{text-align:left;font-weight:700;font-size:12px;color:var(--dim);padding:0 12px;white-space:nowrap}
table.list td{background:var(--b4);padding:9px 12px;white-space:nowrap;transition:background .15s}table.list td:first-child{border-radius:8px 0 0 8px}table.list td:last-child{border-radius:0 8px 8px 0}
table.list tr:hover td{background:var(--b3)}.r{text-align:right}.rank{font-weight:800}.strong{font-weight:800}.pp{color:var(--pink);font-weight:800}
.avatar{display:inline-grid;place-items:center;vertical-align:middle;flex:none;border-radius:24%;background:hsl(var(--h) 45% 40%);color:#fff;font-weight:900}
.avatar.small{width:28px;height:28px;font-size:14px;margin-right:10px}.avatar.mid{width:56px;height:56px;font-size:26px;margin-right:14px}.avatar.big{width:128px;height:128px;font-size:60px;border:4px solid var(--b5);border-radius:22%}
.player{font-weight:800}
.grade{display:inline-grid;place-items:center;flex:none;width:46px;height:24px;border-radius:12px;font-weight:900;font-style:italic;font-size:14px;color:#1d1418;letter-spacing:.02em}
.gss{background:linear-gradient(135deg,#fff6c2,#ffc93c)}.gs{background:#ffcc33}.ga{background:#8bdc2a}.gb{background:#5cb9ff}.gc{background:#c08aff}.gd{background:#ff6b6b}
.grade.big{width:80px;height:40px;border-radius:20px;font-size:26px}
.mod{display:inline-block;min-width:30px;padding:1px 7px;border-radius:5px;font-weight:900;font-size:11px;line-height:18px;letter-spacing:.05em;text-align:center;color:#1d1418;margin:0 3px 2px 0}
.mod.easy{background:#b5e46a}.mod.hard{background:#ff8080}.mod.auto{background:#6cc8ff}.mod.fun{background:#dba6ff}.mod.rate{background:var(--b2);color:var(--text)}
.stars{display:inline-block;padding:0 8px;border-radius:10px;font-weight:800;font-size:12px;line-height:20px;white-space:nowrap}
.version{font-weight:700}
.cover{height:280px;position:relative;background:linear-gradient(0deg,var(--bg),transparent 60%),linear-gradient(hsl(var(--hue) 10% 10%/.3),hsl(var(--hue) 10% 10%/.3)),var(--cover,linear-gradient(135deg,hsl(var(--h,333) 40% 26%),hsl(calc(var(--h,333) + 60) 35% 14%))) center/cover}
.cover.map{height:auto;min-height:280px;display:flex;align-items:flex-end;background:linear-gradient(0deg,var(--bg) 2%,transparent 60%),linear-gradient(90deg,hsl(var(--hue) 10% 10%/.92) 10%,hsl(var(--hue) 10% 10%/.75) 55%,hsl(var(--hue) 10% 10%/.55)),var(--cover,linear-gradient(135deg,hsl(var(--h) 40% 26%),hsl(calc(var(--h) + 60) 35% 14%))) center/cover}
.cover.map .inner{width:100%;padding-bottom:26px}.cover.map .artist{font-size:18px;font-weight:700;color:var(--dim)}.cover.map h1{font-size:40px;font-weight:900;text-shadow:0 2px 12px #0008}
.cover .sub{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin-top:10px}
.head{margin-top:-96px;position:relative;z-index:1;display:flex;align-items:flex-end;gap:32px;flex-wrap:wrap;padding:22px 26px}
.who{display:flex;align-items:flex-end;gap:22px;min-width:0}.who h1{font-size:32px;font-weight:900}.id{min-width:0}
.status{font-size:14px;font-weight:700;color:var(--dim)}.status:before{content:"";display:inline-block;width:8px;height:8px;border-radius:50%;background:currentColor;margin-right:6px;vertical-align:1px}.status.on{color:var(--green)}
.ranks{margin-left:auto;display:flex;gap:36px;align-items:flex-end;flex-wrap:wrap}.bigstat span{display:block;font-size:13px;color:var(--dim);font-weight:700}.bigstat b{font-size:30px;font-weight:900;line-height:1.1}
.level{min-width:190px}.level span{font-size:13px;color:var(--dim);font-weight:700}.level b{color:var(--text)}.bar{height:8px;background:var(--b3);border-radius:4px;margin-top:6px;overflow:hidden}.bar i{display:block;height:100%;background:var(--pink);border-radius:4px}
.side dl{margin:0}.stat{display:flex;justify-content:space-between;gap:12px;padding:8px 0;font-size:15px;border-bottom:1px solid var(--line)}.stat dt{color:var(--dim)}.stat dd{margin:0;font-weight:800}
.grades{display:flex;justify-content:space-between;margin-top:18px}.grades div{display:flex;flex-direction:column;align-items:center;gap:6px}.grades b{font-weight:800}
.play{display:flex;align-items:center;gap:16px;background:var(--b4);border-radius:8px;padding:10px 16px;margin-bottom:6px;transition:background .15s}.play:hover{background:var(--b3)}
.play .what{flex:1;min-width:0}.play .what a{font-weight:800;display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.meta{display:flex;align-items:center;gap:8px;font-size:14px;min-width:0;white-space:nowrap}.meta .version{overflow:hidden;text-overflow:ellipsis}
.play .mods{white-space:nowrap}.play .score{font-weight:700}.play .acc{width:72px;text-align:right;color:var(--dim);font-weight:800}
.play .pp{width:150px;text-align:right;font-size:22px;line-height:1.1}.play .pp small{font-size:13px;margin-left:2px}.weight{font-size:12px;color:var(--dim);font-weight:700;white-space:nowrap}
.topscore{display:flex;align-items:center;gap:28px;flex-wrap:wrap;padding:22px 26px}.place{font-size:30px;font-weight:900}
.topscore .who{align-items:center;gap:0}.tstats{display:flex;gap:32px;flex-wrap:wrap;margin:0 0 0 auto}.tstats dt{font-size:13px;color:var(--dim);font-weight:700}.tstats dd{margin:0;font-size:21px;font-weight:900;white-space:nowrap}.tstats dd.mods{font-size:inherit}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:14px}
.card{position:relative;display:block;min-height:124px;border-radius:var(--r);overflow:hidden;isolation:isolate;background:var(--b4)}
.card .art{position:absolute;inset:0;z-index:-2;background:var(--cover,linear-gradient(135deg,hsl(var(--h) 45% 34%),hsl(calc(var(--h) + 60) 40% 18%))) center/cover;transition:transform .4s var(--ease)}
.card:before{content:"";position:absolute;inset:0;z-index:-1;background:linear-gradient(90deg,hsl(var(--hue) 10% 12%/.96) 30%,hsl(var(--hue) 10% 12%/.78) 65%,hsl(var(--hue) 10% 12%/.4))}
.card:hover{color:var(--text)}.card:hover .art{transform:scale(1.05)}
.card .info{display:flex;flex-direction:column;gap:2px;padding:14px 18px;max-width:88%}
.card .title{font-size:17px;font-weight:800;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.card .artist{font-size:14px;color:var(--dim);font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.card .meta{margin-top:6px}.card .foot{font-size:13px;color:var(--dim);margin-top:4px}
.builds{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px}.build{display:flex;flex-direction:column;align-items:flex-start}.build .btn{margin-top:auto}.build p{margin-bottom:16px}
.build.mine{box-shadow:inset 0 0 0 2px var(--pink)}.yours{font-size:13px;color:var(--pink);font-weight:800}
pre{background:var(--b6);border-radius:8px;padding:14px 16px;overflow-x:auto;font:14px/1.5 ui-monospace,monospace}h3{font-size:16px;margin-top:16px}
.lost{text-align:center;padding:48px 24px}
footer{border-top:1px solid var(--line);background:var(--b6);padding:28px 0}footer .inner{display:flex;flex-wrap:wrap;align-items:center;gap:24px;color:var(--dim);font-size:15px}footer a{color:var(--dim)}footer .brand{color:var(--text);font-weight:900}
@media(max-width:1000px){.cols-home,.cols-side,.builds{grid-template-columns:minmax(0,1fr)}.clips{grid-template-columns:repeat(2,minmax(0,1fr))}.wide{display:none}.head{margin-top:-64px}.ranks{margin-left:0}.tstats{margin-left:0}}
@media(max-width:640px){.inner,.page{padding:0 16px}.page{padding-top:16px}nav .inner{gap:8px}.links a{padding:0 8px;font-size:14px}nav form{display:none}.logo{font-size:0;gap:0}
.hero{padding:44px 0 36px;background:linear-gradient(0deg,var(--bg),transparent 40%),linear-gradient(hsl(var(--hue) 10% 10%/.88),hsl(var(--hue) 10% 10%/.88)),var(--cover,none) center/cover}.clip:first-child .frame{flex:none;aspect-ratio:16/9;min-height:0}.hero h1{font-size:40px}.hero p{font-size:16px}.hero-stats dd{font-size:26px}.clips{grid-template-columns:minmax(0,1fr)}.clip:first-child{grid-column:auto;grid-row:auto}
.box{padding:16px}.play{gap:10px;padding:10px 12px}.play .mods,.play .acc,.play .score,.weight{display:none}.play .pp{width:auto;font-size:18px}
.avatar.big{width:92px;height:92px;font-size:44px}.who{gap:14px}.who h1{font-size:26px}.ranks{gap:24px}.cover.map h1{font-size:30px}.cover{height:200px}.cover.map{min-height:220px}}
@media(prefers-reduced-motion:reduce){*{transition:none!important;animation:none!important}.frame video{display:none}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(esc("<b>\"A&B'</b>"), "&lt;b&gt;&quot;A&amp;B&#39;&lt;/b&gt;");
        assert!(page("<x>", "", "").contains("&lt;x&gt; · taiko-rs"));
        assert!(page("Rankings", "rankings", "").contains("href=\"/rankings\" class=on"));
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
        assert_eq!(month_year(1_792_916_574), "October 2026");
        assert_eq!(split_map("A - T [Inner Oni]"), ("A - T", "Inner Oni"));
        assert_eq!(split_map("no version"), ("no version", ""));
        assert_eq!(split_song("xi - Blue Zenith"), ("xi", "Blue Zenith"));
        assert_eq!(split_song("untitled"), ("", "untitled"));
        assert!(Os::of("Mozilla/5.0 (Windows NT 10.0; Win64; x64)") == Os::Windows);
        assert!(Os::of("Mozilla/5.0 (Linux; Android 14)") == Os::Android);
        assert!(Os::of("Mozilla/5.0 (X11; Linux x86_64)") == Os::Linux);
        assert!(Os::of("Mozilla/5.0 (Macintosh)") == Os::Other);
    }

    #[test]
    fn star_colours() {
        assert_eq!(star_colour(0.0), [0x42, 0x90, 0xfb]);
        assert_eq!(star_colour(2.0), [0x4f, 0xff, 0xd5]);
        assert_eq!(star_colour(12.0), [0, 0, 0]);
        let mid = star_colour(4.55);
        assert!(mid[0] == 0xff && (0x4e..=0x80).contains(&mid[1]));
    }

    #[test]
    fn every_clip_has_its_files() {
        for c in &CLIPS {
            for ext in ["mp4", "jpg"] {
                let name = format!("{}.{ext}", c.file);
                assert!(CLIP_FILES.iter().any(|(f, bytes)| *f == name && !bytes.is_empty()), "{name}");
            }
        }
    }
}
