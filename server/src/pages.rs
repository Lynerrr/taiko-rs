//! The website's pages, laid out like osu!'s: a header bar, rankings, player profiles with their
//! cover, stats and plays, beatmap leaderboards and the download page.

use crate::online::{self, OnlineScore, Profile};
use crate::scores::{MapRow, Ranked};

const REPO: &str = "https://github.com/Lynerrr/taiko-rs";
const RAW: &str = "https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client";

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

fn cover(set: Option<u32>, size: &str) -> Option<String> {
    set.map(|s| format!("https://assets.ppy.sh/beatmaps/{s}/covers/{size}.jpg"))
}

/// A player's picture: their initial on a colour of their own.
fn avatar(name: &str, class: &str) -> String {
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b.to_ascii_lowercase() as u32)) % 360;
    let initial = name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
    format!("<span class=\"avatar {class}\" style=\"--h:{hue}\">{}</span>", esc(&initial.to_string()))
}

fn player_link(name: &str) -> String {
    format!("<a class=player href=\"/u/{0}\">{0}</a>", esc(name))
}

fn grade(g: &str) -> String {
    format!("<span class=\"grade g{}\">{g}</span>", g.to_lowercase())
}

fn mods(m: &[String], rate: f64) -> String {
    let mut out: String = m.iter().map(|a| format!("<span class=mod>{}</span>", esc(a))).collect();
    if (rate - 1.0).abs() > 1e-6 {
        out += &format!("<span class=mod>{rate:.2}x</span>");
    }
    out
}

fn stars(s: Option<f64>) -> String {
    s.map(|s| format!("<span class=stars>&#9733; {s:.2}</span>")).unwrap_or_default()
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

pub fn home(rankings: &[Ranked], maps: &[MapRow], players: usize, plays: usize, os: Os) -> String {
    let top: String = rankings.iter().take(10).map(ranking_row).collect();
    let top = if top.is_empty() { "<p class=dim>Nobody has sent a play yet.</p>".into() } else { format!("<table class=list>{}{top}</table>", ranking_head()) };
    format!(
        "<section class=hero><div class=inner><h1>taiko-rs</h1><p>osu!taiko, rebuilt: low-latency audio, your osu! library and skins, 70+ mods, online lobbies, scores and pp.</p>{}<div class=counts><span><b>{}</b> players ranked</span><span><b>{}</b> plays sent</span></div></div></section>\
         <div class=page><div class=cols2><section class=box><h2>Top players <a class=more href=\"/rankings\">all rankings</a></h2>{top}</section>\
         <section class=box><h2>Recently played <a class=more href=\"/beatmaps\">all beatmaps</a></h2>{}</section></div></div>",
        download_cta(os, false),
        thousands(players as u64),
        thousands(plays as u64),
        map_cards(&maps[..maps.len().min(6)], true)
    )
}

fn ranking_head() -> &'static str {
    "<tr><th></th><th></th><th class=r>Accuracy</th><th class=\"r wide\">Play Count</th><th class=r>Performance</th><th class=\"r wide\">SS</th><th class=\"r wide\">S</th><th class=\"r wide\">A</th></tr>"
}

fn ranking_row(r: &Ranked) -> String {
    format!(
        "<tr><td class=rank>#{}</td><td>{}{}</td><td class=\"r dim\">{:.2}%</td><td class=\"r dim wide\">{}</td><td class=\"r strong\">{}pp</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td></tr>",
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
    let table = if body.is_empty() { "<p class=dim>Nobody has sent a play yet.</p>".into() } else { format!("<table class=list>{}{body}</table>", ranking_head()) };
    format!("<div class=\"band\"><div class=inner><h1>Performance Ranking</h1></div></div><div class=page><section class=box>{table}</section></div>")
}

pub fn profile(p: &Profile) -> String {
    let st = &p.stats;
    let level = online::level(st.total_score);
    let cover_img = p.top.first().and_then(|s| cover(s.set_id, "cover@2x"));
    let cover_style = cover_img.map(|u| format!(" style=\"background-image:url('{u}')\"")).unwrap_or_default();
    let status = if p.online { "<span class=\"status on\">Online</span>" } else { "<span class=status>Offline</span>" };
    let rank = p.rank.map(|r| format!("#{}", thousands(r as u64))).unwrap_or_else(|| "-".into());
    let stat = |k: &str, v: String| format!("<div class=stat><span>{k}</span><b>{v}</b></div>");
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
        grade("SS"),
        st.ss,
        grade("S"),
        st.s,
        grade("A"),
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
        "<div class=cover{cover_style}></div>\
         <div class=page><div class=\"box head\">\
           <div class=who>{}<div><h1>{}</h1>{status}<div class=dim>Joined {}</div></div></div>\
           <div class=ranks><div class=bigstat><span>Global Ranking</span><b>{rank}</b></div><div class=bigstat><span>pp</span><b>{}</b></div>\
             <div class=level><span>Level {}</span><div class=bar><i style=\"width:{:.0}%\"></i></div></div></div>\
         </div>\
         <div class=cols-side><div class=\"box side\">{stats}{grades}</div>\
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
    let score = if show_score { format!("<span class=dim>{}</span>", thousands(s.score)) } else { String::new() };
    format!(
        "<div class=play>{}<div class=what><a href=\"/m/{:x}\">{}</a><div><span class=version>{}</span> {} <span class=dim>{}</span></div></div><div class=mods>{}</div>{score}<div class=acc>{:.2}%</div>{pp}</div>",
        grade(s.grade()),
        s.map_hash,
        esc(song),
        esc(version),
        stars(s.stars),
        ago(s.date),
        mods(&s.mods, s.rate),
        s.accuracy * 100.0
    )
}

pub fn map_page(m: &MapRow, rows: &[OnlineScore]) -> String {
    let (song, version) = split_map(&m.name);
    let cover_style = cover(m.set_id, "cover@2x").map(|u| format!(" style=\"background-image:url('{u}')\"")).unwrap_or_default();
    let osu = m.set_id.map(|s| format!("<a class=dim href=\"https://osu.ppy.sh/beatmapsets/{s}\">osu! beatmap page</a>")).unwrap_or_default();
    let head = format!(
        "<div class=\"cover map\"{cover_style}><div class=inner><h1>{}</h1><div class=sub><span class=version>{}</span> {} <span class=dim>{} plays &middot; {} players</span> {osu}</div></div></div>",
        esc(song),
        esc(version),
        stars(m.stars),
        thousands(m.plays as u64),
        m.players
    );
    let Some(first) = rows.first() else { return format!("{head}<div class=page><p class=dim>No scores yet.</p></div>") };
    let top = format!(
        "<div class=\"box topscore\"><div class=place>#1</div>{}<div class=who>{}<div>{}<div class=dim>{}</div></div></div>\
         <div class=tstat><span>Total Score</span><b>{}</b></div><div class=tstat><span>Accuracy</span><b>{:.2}%</b></div><div class=tstat><span>Max Combo</span><b>{}x</b></div>\
         <div class=tstat><span>pp</span><b>{}</b></div><div class=tstat><span>Mods</span><b>{}</b></div></div>",
        grade(first.grade()),
        avatar(&first.player, "mid"),
        player_link(&first.player),
        ago(first.date),
        thousands(first.score),
        first.accuracy * 100.0,
        thousands(first.max_combo as u64),
        first.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
        if first.mods.is_empty() && (first.rate - 1.0).abs() < 1e-6 { "-".into() } else { mods(&first.mods, first.rate) }
    );
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                "<tr><td class=rank>#{}</td><td>{}</td><td class=strong>{}</td><td>{:.2}%</td><td>{}{}</td><td class=wide>{}x</td><td class=wide>{}</td><td class=wide>{}</td><td class=wide>{}</td><td class=strong>{}</td><td class=wide>{}</td><td class=dim>{}</td></tr>",
                i + 1,
                grade(r.grade()),
                thousands(r.score),
                r.accuracy * 100.0,
                avatar(&r.player, "small"),
                player_link(&r.player),
                thousands(r.max_combo as u64),
                r.great,
                r.ok,
                r.miss,
                r.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
                mods(&r.mods, r.rate),
                ago(r.date)
            )
        })
        .collect();
    format!(
        "{head}<div class=page>{top}<section class=box><table class=list><tr><th>Rank</th><th></th><th>Score</th><th>Accuracy</th><th>Player</th><th class=wide>Max Combo</th><th class=wide>Great</th><th class=wide>Ok</th><th class=wide>Miss</th><th>pp</th><th class=wide>Mods</th><th></th></tr>{body}</table></section></div>"
    )
}

/// Beatmaps as cards with their cover; `compact`: two to a row in a side box.
fn map_cards(maps: &[MapRow], compact: bool) -> String {
    if maps.is_empty() {
        return "<p class=dim>No maps played yet.</p>".into();
    }
    let cards: String = maps
        .iter()
        .map(|m| {
            let (song, version) = split_map(&m.name);
            let bg = cover(m.set_id, "card@2x").map(|u| format!(" style=\"background-image:url('{u}')\"")).unwrap_or_default();
            format!(
                "<a class=card href=\"/m/{:x}\"><div class=art{bg}></div><div class=info><b>{}</b><span><span class=version>{}</span> {}</span><span class=dim>{} plays &middot; played {}</span></div></a>",
                m.hash,
                esc(song),
                esc(version),
                stars(m.stars),
                thousands(m.plays as u64),
                ago(m.last)
            )
        })
        .collect();
    format!("<div class=\"cards{}\">{cards}</div>", if compact { " compact" } else { "" })
}

pub fn beatmaps(maps: &[MapRow]) -> String {
    format!("<div class=band><div class=inner><h1>Beatmaps</h1><p class=dim>Every map someone has sent a play on, the latest first.</p></div></div><div class=page>{}</div>", map_cards(maps, false))
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
    format!("<div class=page><section class=box><h1>Not found</h1><p>{}.</p><p><a href=\"/\">Home</a></p></section></div>", esc(why))
}

pub fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{} · taiko-rs</title>
<link rel=preconnect href="https://fonts.googleapis.com"><link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=Nunito:wght@400;600;700;800&display=swap"><style>{STYLE}</style></head>
<body><nav><div class=inner><a class=logo href="/"><span>rs</span>taiko-rs</a><a href="/rankings">rankings</a><a href="/beatmaps">beatmaps</a><a href="/download">download</a>
<form action="/u"><input name=name placeholder="search players" maxlength=16 aria-label="Search players"></form></div></nav>
{}<footer><div class=inner><span>taiko-rs</span><a href="{REPO}">GitHub</a><a href="/download">Download</a><span class=dim>Map covers from osu!</span></div></footer></body></html>"#,
        esc(title),
        body
    )
}

const STYLE: &str = r#"
:root{--bg:#17141a;--bg2:#1f1a22;--box:#2a2330;--box2:#332a3a;--line:#3d3344;--text:#f0e9f3;--dim:#a493ad;--accent:#ff66ab;--accent2:#ff8ec4;--green:#b3d944}
*{box-sizing:border-box}html{background:var(--bg)}body{margin:0;color:var(--text);font:15px/1.5 Nunito,system-ui,sans-serif}
a{color:var(--text);text-decoration:none}a:hover{color:var(--accent2)}.dim{color:var(--dim)}h1,h2,h3{margin:0;font-weight:800}
.inner,.page{max-width:1200px;margin:0 auto;padding:0 20px}.page{padding-top:20px;padding-bottom:40px}
nav{background:#120f15;border-bottom:1px solid var(--line);position:sticky;top:0;z-index:5}
nav .inner{display:flex;align-items:center;gap:22px;height:58px}nav a{font-weight:700;color:var(--dim)}nav a:hover{color:var(--text)}
.logo{display:flex;align-items:center;gap:10px;color:var(--text)!important;font-size:19px;font-weight:800;margin-right:12px}
.logo span{display:grid;place-items:center;width:34px;height:34px;border-radius:50%;background:var(--accent);color:#fff;font-size:14px}
nav form{margin-left:auto}nav input{font:inherit;width:220px;background:var(--box);border:1px solid var(--line);border-radius:6px;color:var(--text);padding:6px 12px}
nav input:focus{outline:none;border-color:var(--accent)}
.hero{background:radial-gradient(circle at 20% 0,#4a2340,transparent 60%),linear-gradient(180deg,#2a1a2a,var(--bg));padding:60px 0 50px}
.hero h1{font-size:46px}.hero p{max-width:620px;color:var(--dim);font-size:17px}
.btn{display:inline-block;background:var(--accent);color:#fff!important;font-weight:800;padding:10px 22px;border-radius:8px;margin-top:10px}.btn:hover{background:var(--accent2)}
.btn.big{font-size:19px;padding:14px 30px}.btn.plain{background:var(--box2)}.hint{color:var(--dim);font-size:13px;margin-top:8px}
.counts{display:flex;gap:28px;margin-top:22px;color:var(--dim)}.counts b{color:var(--text);font-size:20px}
.band{background:linear-gradient(180deg,#2a1a2a,var(--bg));padding:34px 0 18px}.band h1{font-size:32px}
.box{background:var(--box);border-radius:10px;padding:18px 20px;margin-bottom:18px}.box h2{font-size:18px;margin-bottom:12px;display:flex;align-items:baseline;gap:10px}
.more{margin-left:auto;font-size:13px;color:var(--accent2)}.count{color:var(--dim);font-size:15px}
.cols2{display:grid;grid-template-columns:3fr 2fr;gap:18px;align-items:start}
table.list{width:100%;border-collapse:separate;border-spacing:0 4px}table.list th{text-align:left;font-weight:600;font-size:12px;color:var(--dim);padding:0 10px}
table.list td{background:var(--box2);padding:8px 10px;white-space:nowrap}table.list td:first-child{border-radius:6px 0 0 6px}table.list td:last-child{border-radius:0 6px 6px 0}
table.list tr:hover td{background:#3b3143}.r{text-align:right}.rank{font-weight:800}.strong{font-weight:800}
.avatar{display:inline-grid;place-items:center;vertical-align:middle;border-radius:22%;background:hsl(var(--h),45%,38%);color:#fff;font-weight:800}
.avatar.small{width:26px;height:26px;font-size:13px;margin-right:10px}.avatar.mid{width:56px;height:56px;font-size:26px;margin-right:14px}.avatar.big{width:120px;height:120px;font-size:56px;border:4px solid var(--box)}
.player{font-weight:700}
.grade{display:inline-block;width:46px;font-weight:800;font-style:italic;font-size:20px;text-align:center}
.gss{color:#ffd95c;text-shadow:0 0 8px #ffc40066}.gs{color:#ffbf1f}.ga{color:#88da20}.gb{color:#4fb4ff}.gc{color:#c77dff}.gd{color:#ff5a5a}
.mod{display:inline-block;font-weight:800;font-size:12px;color:var(--accent2);margin-right:4px}
.stars{color:#ffd95c;font-weight:700;font-size:13px}.version{font-weight:700;color:var(--dim)}
.cover{height:260px;background:linear-gradient(135deg,#4a2340,#1f2a4a) center/cover;position:relative}
.cover:after{content:"";position:absolute;inset:0;background:linear-gradient(180deg,rgba(23,20,26,.15) 30%,var(--bg))}.cover.map:after{background:linear-gradient(180deg,rgba(23,20,26,.35),rgba(23,20,26,.85) 70%,var(--bg))}
.cover.map{height:auto;min-height:220px;display:flex;align-items:flex-end}.cover.map .inner{position:relative;z-index:1;width:100%;padding-bottom:22px}
.cover.map h1{font-size:34px;text-shadow:0 2px 8px #000}.cover .sub{display:flex;gap:14px;align-items:baseline;flex-wrap:wrap;text-shadow:0 1px 4px #000}
.head{margin-top:-90px;position:relative;z-index:1;display:flex;align-items:flex-end;gap:30px;flex-wrap:wrap}
.who{display:flex;align-items:flex-end;gap:20px}.who h1{font-size:30px;line-height:1.1}.status{font-size:13px;color:var(--dim)}.status.on{color:var(--green)}
.ranks{margin-left:auto;display:flex;gap:34px;align-items:flex-end;flex-wrap:wrap}.bigstat span{display:block;font-size:12px;color:var(--dim)}.bigstat b{font-size:28px}
.level{min-width:180px}.level span{font-size:12px;color:var(--dim)}.bar{height:6px;background:var(--box2);border-radius:3px;margin-top:6px}.bar i{display:block;height:100%;background:var(--accent);border-radius:3px}
.cols-side{display:grid;grid-template-columns:270px 1fr;gap:18px;align-items:start}.cols-side>*,.cols2>*{min-width:0}
.stat{display:flex;justify-content:space-between;padding:5px 0;font-size:14px;border-bottom:1px solid var(--line)}.stat span{color:var(--dim)}
.grades{display:flex;justify-content:space-between;margin-top:16px}.grades div{text-align:center}.grades b{display:block}
.play{display:flex;align-items:center;gap:14px;background:var(--box2);border-radius:8px;padding:8px 14px;margin-bottom:5px}.play:hover{background:#3b3143}
.play .what{flex:1;min-width:0}.play .what a{font-weight:700;display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.play .what div{font-size:13px}
.play .acc{width:70px;text-align:right;color:var(--dim);font-weight:700}.play .pp{width:150px;text-align:right;font-size:20px;font-weight:800;color:var(--accent2)}
.play .pp small{font-size:12px;margin-left:2px}.weight{font-size:11px;color:var(--dim);font-weight:600;white-space:nowrap}.play .mods{white-space:nowrap}
.topscore{display:flex;align-items:center;gap:24px;flex-wrap:wrap}.place{font-size:28px;font-weight:800}.topscore .grade{font-size:40px;width:auto}
.topscore .who{align-items:center;gap:0;flex-direction:row}.tstat span{display:block;font-size:12px;color:var(--dim)}.tstat b{font-size:20px}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(330px,1fr));gap:14px}.cards.compact{grid-template-columns:1fr}
.card{display:flex;background:var(--box2);border-radius:10px;overflow:hidden;min-height:96px}.card:hover{background:#3b3143;color:var(--text)}
.card .art{width:120px;flex:none;background:linear-gradient(135deg,#4a2340,#1f2a4a) center/cover}.card .info{padding:10px 14px;display:flex;flex-direction:column;gap:2px;min-width:0}
.card b{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.card span{font-size:13px}
.builds{display:grid;grid-template-columns:repeat(3,1fr);gap:18px}.build.mine{outline:2px solid var(--accent)}.yours{font-size:12px;color:var(--accent2);font-weight:700}
pre{background:#120f15;border-radius:8px;padding:12px 14px;overflow-x:auto;font:14px ui-monospace,monospace}h3{font-size:15px;margin-top:14px}
footer{border-top:1px solid var(--line);padding:24px 0;margin-top:20px}footer .inner{display:flex;gap:22px;color:var(--dim)}footer a{color:var(--dim)}
@media(max-width:900px){.cols2,.cols-side,.builds{grid-template-columns:minmax(0,1fr)}.wide{display:none}nav input{width:130px}.head{margin-top:-60px}.ranks{margin-left:0}}
@media(max-width:560px){nav .inner{gap:12px}nav a:not(.logo){font-size:13px}nav form{display:none}.hero h1{font-size:34px}.play .mods,.play .acc,.weight{display:none}.play .pp{width:auto}.avatar.big{width:90px;height:90px;font-size:42px}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(esc("<b>\"A&B'</b>"), "&lt;b&gt;&quot;A&amp;B&#39;&lt;/b&gt;");
        assert!(page("<x>", "").contains("&lt;x&gt; · taiko-rs"));
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
        assert_eq!(month_year(1_792_916_574), "October 2026");
        assert_eq!(split_map("A - T [Inner Oni]"), ("A - T", "Inner Oni"));
        assert_eq!(split_map("no version"), ("no version", ""));
        assert!(Os::of("Mozilla/5.0 (Windows NT 10.0; Win64; x64)") == Os::Windows);
        assert!(Os::of("Mozilla/5.0 (Linux; Android 14)") == Os::Android);
        assert!(Os::of("Mozilla/5.0 (X11; Linux x86_64)") == Os::Linux);
        assert!(Os::of("Mozilla/5.0 (Macintosh)") == Os::Other);
    }
}
