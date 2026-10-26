//! The website's pages, in the game's own look: its 5x7 LCD font, its greys and red, its notes
//! and HUD. The home page is a playfield that keeps switching mods on itself; around it are the
//! hub's live parts (who's online, open rooms, the latest plays), osu!-like profiles, rankings,
//! beatmap leaderboards, the mod select with clips of what the mods do, and the download page.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::online::{self, Activity, ChatLine, OnlinePlayer, OnlineScore, Profile, RoomInfo};
use crate::scores::{Extras, MapRow, Ranked};

const REPO: &str = "https://github.com/Lynerrr/taiko-rs";
/// Where the site lives, for the absolute links link previews need.
const SITE: &str = "https://rstaiko.auneu.space";
const TAGLINE: &str = "A taiko client for osu! beatmaps, with 73 mods that bend, break and multiply the lane. Rankings, profiles, chat and downloads.";
const RAW: &str = "https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client";

/// A clip of taiko.rs's own mods: what it shows, the mods on, its file under /clips/, and the
/// hover effect its card plays (the mod done to the card itself).
struct Clip {
    file: &'static str,
    title: &'static str,
    mods: &'static [&'static str],
    what: &'static str,
    fx: &'static str,
}

const CLIPS: [Clip; 6] = [
    Clip { file: "multiverse", title: "Multiverse", mods: &["MV"], what: "Eight playfields circle about, each with its own look and mods. The outlined one is live; the next one is already on its way.", fx: "mv" },
    Clip { file: "shapeshift", title: "Shapeshift", mods: &["SS", "CR"], what: "Every few bars the lane bends into another shape: a loop, a hook, a straight again.", fx: "ss" },
    Clip { file: "portal", title: "Portal + Barrel Roll", mods: &["AC", "LP", "SK", "BA", "PT"], what: "Notes dive into one portal and come out of another, streams slither and every kiai spins the lane.", fx: "ba" },
    Clip { file: "loop", title: "Loop + Crumble", mods: &["LP", "CR"], what: "A loop-the-loop in the middle of the lane, and every hit knocks a piece out of it.", fx: "lp" },
    Clip { file: "crumble", title: "Crumble", mods: &["CR"], what: "Hits knock the lane apart, harder as the combo grows; a miss puts it back together.", fx: "cr" },
    Clip { file: "shatter", title: "Shatter + Decoy", mods: &["BR", "WV", "DY", "LP", "SH", "CK"], what: "The lane breaks into pieces that slide apart, and see-through fakes fly between the real notes.", fx: "sh" },
];

/// Files served under /clips/ and /site/ (`web.rs`).
pub const STATIC_FILES: [(&str, &[u8]); 18] = [
    ("clips/multiverse.mp4", include_bytes!("../site/clips/multiverse.mp4")),
    ("clips/multiverse.jpg", include_bytes!("../site/clips/multiverse.jpg")),
    ("clips/loop.mp4", include_bytes!("../site/clips/loop.mp4")),
    ("clips/loop.jpg", include_bytes!("../site/clips/loop.jpg")),
    ("clips/shapeshift.mp4", include_bytes!("../site/clips/shapeshift.mp4")),
    ("clips/shapeshift.jpg", include_bytes!("../site/clips/shapeshift.jpg")),
    ("clips/portal.mp4", include_bytes!("../site/clips/portal.mp4")),
    ("clips/portal.jpg", include_bytes!("../site/clips/portal.jpg")),
    ("clips/crumble.mp4", include_bytes!("../site/clips/crumble.mp4")),
    ("clips/crumble.jpg", include_bytes!("../site/clips/crumble.jpg")),
    ("clips/shatter.mp4", include_bytes!("../site/clips/shatter.mp4")),
    ("clips/shatter.jpg", include_bytes!("../site/clips/shatter.jpg")),
    ("site/taiko5x7.woff2", include_bytes!("../site/taiko5x7.woff2")),
    ("site/logo.png", include_bytes!("../../assets/icon-64.png")),
    ("site/chat.js", include_bytes!("../site/chat.js")),
    ("site/copy.js", include_bytes!("../site/copy.js")),
    ("site/watch.js", include_bytes!("../site/watch.js")),
    ("site/og.jpg", include_bytes!("../site/og.jpg")),
];

/// A mod select column (server/site/mods.json, written by the game's tests from its own list).
#[derive(Deserialize)]
struct Group {
    title: String,
    colour: String,
    mods: Vec<ModInfo>,
}

#[derive(Deserialize)]
struct ModInfo {
    acronym: String,
    name: String,
    description: String,
}

fn groups() -> &'static [Group] {
    static GROUPS: OnceLock<Vec<Group>> = OnceLock::new();
    GROUPS.get_or_init(|| serde_json::from_str(include_str!("../site/mods.json")).unwrap_or_default())
}

fn mod_count() -> usize {
    groups().iter().map(|g| g.mods.len()).sum()
}

/// A mod's column colour and name, by its acronym.
pub(crate) fn mod_info(acronym: &str) -> Option<(&'static str, &'static str)> {
    groups().iter().find_map(|g| g.mods.iter().find(|m| m.acronym == acronym).map(|m| (g.colour.as_str(), m.name.as_str())))
}

/// Badges next to a player's name on their profile, like osu!'s groups: (lowercased name,
/// label, colour, what it means).
const BADGES: [(&str, &str, &str, &str); 3] = [
    ("auneu", "DEV", "#EB452C", "taiko.rs developer"),
    ("lynerrr", "DEV", "#EB452C", "taiko.rs developer"),
    ("claude", "MAIN DEV", "#d97757", "taiko.rs's main developer"),
];

fn badges(name: &str) -> String {
    let key = name.to_ascii_lowercase();
    BADGES
        .iter()
        .filter(|b| b.0 == key)
        .map(|(_, label, colour, what)| format!("<span class=badge style=\"--c:{colour}\" title=\"{what}\">{label}</span>"))
        .collect()
}

/// A player's roles (their badges), and what the one looking at their profile may do there.
pub struct Roles {
    pub admin: bool,
    pub nominator: bool,
    /// It's the viewer's own profile.
    pub own: bool,
    pub viewer_admin: bool,
    /// Their code for `>verify` on Discord (their own page, nominators only).
    pub verify_code: Option<String>,
}

fn role_badges(r: &Roles) -> String {
    let badge = |label: &str, colour: &str, what: &str| format!("<span class=badge style=\"--c:{colour}\" title=\"{what}\">{label}</span>");
    if r.admin {
        badge("ADMIN", "#9b59ff", "taiko.rs admin: makes loved nominators, and loves maps")
    } else if r.nominator {
        badge("LOVED", "#ff66aa", "loved nominator: loves graveyard maps so they give pp here")
    } else {
        String::new()
    }
}

/// The profile's tools for whoever may use them: their picture and Discord code (their own
/// page), loved nominator on or off (admins).
fn profile_tools(p: &Profile, r: &Roles) -> String {
    let mut parts = Vec::new();
    if r.own {
        let remove = if crate::avatars::version(&p.name.to_ascii_lowercase()).is_some() {
            "<form method=post action=\"/avatar/remove\"><button class=\"btn small\">remove picture</button></form>"
        } else {
            ""
        };
        parts.push(format!(
            "<div class=tool><b>PROFILE PICTURE</b><form method=post action=\"/avatar\" enctype=\"multipart/form-data\" class=upload>\
             <input type=file name=avatar accept=\"image/png,image/jpeg,image/gif,image/webp\" required aria-label=\"picture\"><button class=\"btn small\">upload</button></form>{remove}\
             <span class=dim>PNG, JPEG, GIF or WebP, at most {} MB. Square ones look best.</span></div>",
            crate::avatars::MAX_BYTES / 1024 / 1024
        ));
    }
    if let Some(code) = &r.verify_code {
        parts.push(format!(
            "<div class=tool><b>DISCORD</b><span>To love maps from Discord, send the taiko.rs bot <code>&gt;verify {}</code> (it works once, within 30 minutes).</span></div>",
            esc(code)
        ));
    }
    if r.viewer_admin && !r.own {
        let nominator = if r.admin {
            String::new()
        } else {
            let (on, label) = if r.nominator { ("0", "remove loved nominator") } else { ("1", "make loved nominator") };
            format!("<form method=post action=\"/u/{}/nominator\"><input type=hidden name=on value={on}><button class=\"btn small\">{label}</button></form>", esc(&p.name))
        };
        let remove = if crate::avatars::version(&p.name.to_ascii_lowercase()).is_some() {
            format!("<form method=post action=\"/u/{}/avatar/remove\"><button class=\"btn small\">remove picture</button></form>", esc(&p.name))
        } else {
            String::new()
        };
        parts.push(format!(
            "<div class=tool><b>ADMIN</b>{nominator}<form method=post action=\"/u/{0}/avatar\" enctype=\"multipart/form-data\" class=upload>\
             <input type=file name=avatar accept=\"image/png,image/jpeg,image/gif,image/webp\" required aria-label=\"their picture\"><button class=\"btn small\">set their picture</button></form>{remove}</div>",
            esc(&p.name)
        ));
    }
    if parts.is_empty() { String::new() } else { format!("<section class=\"panel tools\">{}</section>", parts.concat()) }
}

/// A map page's love button and state.
pub struct Love {
    /// Loved here: by whom and when.
    pub loved: Option<crate::loved::Loved>,
    /// The viewer is a loved nominator.
    pub can_love: bool,
    pub notice: Option<String>,
}

/// A set loved here, for the list of them.
pub struct LovedRow {
    pub set: u32,
    /// Its artist and title, if looked up.
    pub names: Option<(String, String)>,
    pub loved: crate::loved::Loved,
}

pub fn loved(rows: &[LovedRow]) -> String {
    let list = if rows.is_empty() {
        "<p class=dim>Nothing yet. Loved nominators pick graveyard maps to love; then plays on them give pp.</p>".to_string()
    } else {
        rows.iter()
            .map(|r| {
                let (artist, title) = r.names.clone().unwrap_or_else(|| (String::new(), format!("set {}", r.set)));
                format!(
                    "<a class=mini href=\"https://osu.ppy.sh/beatmapsets/{0}\"{1}><span class=art></span><span class=mini-info><b>{2}</b><span class=dim>{3}</span></span><span class=dim>by {4}, {5}</span></a>",
                    r.set,
                    cover_style(Some(r.set), 0, "list@2x"),
                    esc(&title),
                    esc(&artist),
                    esc(&r.loved.by),
                    ago(r.loved.at)
                )
            })
            .collect()
    };
    format!(
        "<div class=band><div class=inner><h1 class=glitch data-text=\"LOVED\">LOVED</h1><p class=dim>Maps loved here by taiko.rs's loved nominators: they give pp on taiko.rs, whatever their status on osu!.</p></div></div>\
         <div class=page><section class=panel>{list}</section></div>"
    )
}

/// How the one looking at a profile stands with its player.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Friendship {
    /// Not logged in, or no tie.
    None,
    Me,
    Friends,
    /// The viewer has asked; waiting on them.
    Asked,
    /// They asked the viewer.
    TheyAsked,
}

/// Someone the logged-in player can message, for the chat's list.
pub struct Conversation {
    pub name: String,
    pub online: bool,
    pub last: Option<ChatLine>,
}

/// What a link to a page shows when pasted into Discord and the like (Open Graph).
pub struct Meta {
    pub description: String,
    /// An absolute image URL; None: the site's own card.
    pub image: Option<String>,
}

impl Default for Meta {
    fn default() -> Meta {
        Meta { description: TAGLINE.into(), image: None }
    }
}

/// A profile's preview: rank, pp and the rest, over the cover of their best play.
pub fn profile_meta(p: &Profile) -> Meta {
    let rank = p.rank.map(|r| format!("#{r} · ")).unwrap_or_default();
    Meta {
        description: format!(
            "{rank}{}pp · {:.2}% accuracy · {} plays · level {:.0}. taiko.rs player since {}.",
            thousands(p.pp.round() as u64),
            p.stats.accuracy * 100.0,
            thousands(p.plays as u64),
            online::level(p.stats.total_score).floor(),
            month_year(p.joined)
        ),
        image: p.top.first().and_then(|s| cover(s.set_id, "cover@2x")),
    }
}

/// A fingerprint of the site's card image, in its link: a new card is a new link, so neither the
/// proxy nor Discord keeps showing the old one.
fn card_version() -> String {
    let bytes = STATIC_FILES.iter().find(|(f, _)| *f == "site/og.jpg").map(|(_, b)| *b).unwrap_or_default();
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    format!("{:08x}", hash >> 32)
}

/// A score's preview: its numbers, with its card (`card.rs`) as the picture.
pub fn score_meta(s: &OnlineScore) -> Meta {
    let pp = s.pp.map(|p| format!("{p:.0}pp · ")).unwrap_or_default();
    let mods = if s.mods.is_empty() { String::new() } else { format!(" +{}", s.mods.concat()) };
    let rate = if (s.rate - 1.0).abs() > 1e-6 { format!(" {:.2}x", s.rate) } else { String::new() };
    Meta {
        description: format!(
            "{} by {}: {pp}{:.2}% · {} · x{} · {} miss{}{mods}{rate}. Watch the replay in taiko.rs.",
            s.grade(),
            s.player,
            s.accuracy * 100.0,
            thousands(s.score),
            thousands(s.max_combo as u64),
            s.miss,
            if s.miss == 1 { "" } else { "es" }
        ),
        image: Some(format!("{SITE}/s/{}/card.png", s.id)),
    }
}

/// A beatmap's preview: stars, plays and who holds #1, over its cover.
pub fn map_meta(m: &MapRow, rows: &[OnlineScore]) -> Meta {
    let (_, version) = split_map(&m.name);
    let stars = m.stars.map(|s| format!("{s:.2}* ")).unwrap_or_default();
    let top = rows.first().map(|r| format!(" #1: {} with {} ({:.2}%).", r.player, thousands(r.score), r.accuracy * 100.0)).unwrap_or_default();
    Meta { description: format!("{stars}[{version}] · {} plays by {} players on taiko.rs.{top}", thousands(m.plays as u64), m.players), image: cover(m.set_id, "cover@2x") }
}

/// What's live on the server, for the home page.
pub struct Live<'a> {
    pub online: &'a [OnlinePlayer],
    pub rooms: &'a [RoomInfo],
    pub latest: &'a [OnlineScore],
}

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

/// A set's cover from osu!, unless it's NSFW, blocked or not checked yet (`covers`).
fn cover(set: Option<u32>, size: &str) -> Option<String> {
    set.filter(|&s| crate::covers::allowed(s)).map(|s| format!("https://assets.ppy.sh/beatmaps/{s}/covers/{size}.jpg"))
}

/// The style for a block with a map's cover behind it: the map's own hue (for when it has no
/// cover), and the picture if there is one.
fn cover_style(set: Option<u32>, hash: u64, size: &str) -> String {
    let img = cover(set, size).map(|u| format!(";--cover:url('{u}')")).unwrap_or_default();
    format!(" style=\"--h:{}{img}\"", hash % 360)
}

/// A player's picture: their initial on a colour of their own.
/// Time spent playing, like osu!'s: 3d 4h 12m.
fn play_time(ms: u64) -> String {
    let minutes = ms / 60_000;
    let (d, h, m) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    }
}

/// A player's picture, or (without one) their initial on a colour of its own.
fn avatar(name: &str, class: &str) -> String {
    let key = name.to_ascii_lowercase();
    if let Some(v) = crate::avatars::version(&key) {
        return format!("<img class=\"avatar {class}\" src=\"/a/{}?v={v}\" alt=\"\" loading=lazy>", esc(&key));
    }
    let hue = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b.to_ascii_lowercase() as u32)) % 360;
    let initial = name.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
    format!("<span class=\"avatar {class}\" style=\"--h:{hue}\" aria-hidden=true>{}</span>", esc(&initial.to_string()))
}

fn player_link(name: &str) -> String {
    format!("<a class=player href=\"/u/{0}\">{0}</a>", esc(name))
}

fn grade(g: &str) -> String {
    format!("<span class=\"grade g{}\">{g}</span>", g.to_lowercase())
}

/// Mod chips in their mod select column's colour.
fn mod_chips<S: AsRef<str>>(m: &[S], rate: f64) -> String {
    let mut out: String = m
        .iter()
        .map(|a| {
            let a = a.as_ref();
            match mod_info(a) {
                Some((colour, name)) => format!("<span class=mod style=\"--c:{colour}\" title=\"{}\">{}</span>", esc(name), esc(a)),
                None => format!("<span class=mod>{}</span>", esc(a)),
            }
        })
        .collect();
    if (rate - 1.0).abs() > 1e-6 {
        out += &format!("<span class=\"mod rate\">{rate:.2}x</span>");
    }
    out
}

/// osu!'s colour for a star rating: blue for easy through red and purple to black.
pub(crate) fn star_colour(s: f64) -> [u8; 3] {
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

/// A star, drawn in the font's own 5x5 pixels.
const STAR: &str = "<svg class=star viewBox=\"0 0 5 5\" aria-hidden=true><path d=\"M2 0h1v2h2v1h-1v2h-1v-1h-1v1h-1v-2h-1v-1h2z\"/></svg>";

fn stars(s: Option<f64>) -> String {
    s.map(|s| {
        let [r, g, b] = star_colour(s);
        let text = if s >= 6.5 { "#ffd966" } else { "#000" };
        format!("<span class=stars style=\"background:#{r:02x}{g:02x}{b:02x};color:{text}\">{STAR}{s:.2}</span>")
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
    note: &'static str,
    /// The download (and what to call it), and the other way to get it.
    file: &'static str,
    kind: &'static str,
    alt: Option<(&'static str, &'static str)>,
}

const BUILDS: [Build; 3] = [
    Build {
        os: Os::Windows,
        name: "Windows",
        note: "Windows 10 or 11, 64-bit. Installs for you alone (no admin), with a Start menu shortcut and an uninstaller; it then updates itself from the game.",
        file: "taiko-rs-windows-x64-setup.exe",
        kind: "installer",
        alt: Some(("taiko-rs-windows-x64.zip", "portable .zip")),
    },
    Build {
        os: Os::Linux,
        name: "Linux",
        note: "64-bit, glibc 2.39+ (Ubuntu 24.04, Debian 13, Fedora 40, Arch...). The command installs it, adds it to your apps and keeps it updated.",
        file: "taiko-rs-linux-x64.tar.gz",
        kind: ".tar.gz",
        alt: None,
    },
    Build { os: Os::Android, name: "Android", note: "Android 8.0+, 64-bit ARM.", file: "taiko-rs-android.apk", kind: "APK", alt: None },
];

/// The install commands.
fn linux_command() -> String {
    format!("curl -fsSL {RAW}/install.sh | sh")
}

fn powershell_command() -> String {
    format!("irm {RAW}/install.ps1 | iex")
}

/// A command to paste, with a button that copies it (`site/copy.js`).
fn cmd(command: &str) -> String {
    let c = esc(command);
    format!("<div class=cmd><code><span class=prompt>$ </span>{c}</code><button class=copy type=button data-copy=\"{c}\">copy</button></div>")
}

fn download_url(file: &str) -> String {
    format!("{REPO}/releases/download/latest/{file}")
}

/// The big download button for the visitor's system (or a pointer to the choices). On the
/// download page itself the choices are right below.
fn download_cta(os: Os, on_download_page: bool) -> String {
    let others = if on_download_page { String::new() } else { " &middot; <a href=\"/download\">other systems</a>".into() };
    match BUILDS.iter().find(|b| b.os == os) {
        Some(b) if b.os == Os::Linux => format!(
            "<div class=cta-label>Install on Linux: paste this in a terminal</div>{}<div class=hint>installs, adds it to your apps and keeps it updated &middot; <a href=\"{}\">or the .tar.gz</a>{others}</div>",
            cmd(&linux_command()),
            download_url(b.file)
        ),
        Some(b) => format!(
            "<a class=\"btn hot\" href=\"{}\">Download for {}</a><div class=hint>{}{others}</div>",
            download_url(b.file),
            b.name,
            b.note
        ),
        None if on_download_page => "<p class=dim>Pick your system below.</p>".into(),
        None => "<a class=\"btn hot\" href=\"/download\">Download taiko.rs</a><div class=hint>Windows, Linux and Android</div>".into(),
    }
}

/// The home page's bar: d/k a don or kat, D/K a finisher, _ a gap; a quarter of a second each,
/// 6 s in all.
const PATTERN: &str = "dkdd_kkd_DdkdkK_dd_kdk_d";

/// Bars the combo counts up through before it starts again from 0.
const COMBO_BARS: usize = 20;

/// The drum, as the game draws the keys being pressed: a don lights one half of its face red,
/// a kat one half of its rim blue, the hands taking turns; a finisher lights both halves. And
/// the combo, one more on every note. All in step with `notes`: (keyframes, markup).
fn drum() -> (String, String) {
    let hits: Vec<(f64, char)> = PATTERN.chars().enumerate().filter(|(_, c)| *c != '_').map(|(i, c)| (i as f64 * 0.25, c)).collect();
    // Which halves each hit lights: L, R, or both for a finisher.
    // Two bars, so the hands keep taking turns when the bar (an odd number of notes) loops.
    let mut tracks: [(&str, String); 4] = [("donl", String::new()), ("donr", String::new()), ("katl", String::new()), ("katr", String::new())];
    let two_bars = hits.iter().map(|&(t, c)| (t, c)).chain(hits.iter().map(|&(t, c)| (t + 6.0, c)));
    for (n, (t, c)) in two_bars.enumerate() {
        let p = t / 12.0 * 100.0;
        let frames = format!("{p:.2}%{{opacity:1}}{:.2}%{{opacity:0}}", p + 1.0);
        let (don, both) = (c.eq_ignore_ascii_case(&'d'), c.is_ascii_uppercase());
        let left = n % 2 == 0;
        for (i, (_, track)) in tracks.iter_mut().enumerate() {
            let (is_don, is_left) = (i < 2, i % 2 == 0);
            if is_don == don && (both || is_left == left) {
                *track += &frames;
            }
        }
    }
    let mut css: String = tracks
        .iter()
        .map(|(name, track)| format!("@keyframes {name}{{{}{track}}}", if track.starts_with("0.00%") { "" } else { "0%{opacity:0}" }))
        .collect();
    // The combo: a strip of numbers, moved one up on every hit.
    let total = COMBO_BARS as f64 * 6.0;
    let mut combo = String::from("@keyframes combo{0%{transform:translateY(0)}");
    let mut n = 0;
    for bar in 0..COMBO_BARS {
        for &(t, _) in &hits {
            n += 1;
            combo += &format!("{:.3}%{{transform:translateY(-{}em)}}", (bar as f64 * 6.0 + t) / total * 100.0, n as f64 * 3.25);
        }
    }
    css += &(combo + "}");
    let digits: String = (0..=n).map(|i| format!("<span>{i}</span>")).collect();
    let markup = format!(
        "<div class=drum><b class=\"fl rim l\"></b><b class=\"fl rim r\"></b><span class=face><b class=\"fl don l\"></b><b class=\"fl don r\"></b><span class=combo><span class=digits>{digits}</span></span></span></div>"
    );
    (format!("<style>{css}</style>"), markup)
}

/// The home page's lane: a bar of dons, kats, finishers and gaps, a quarter beat each. Every
/// note flies in on its own and bursts on the judgement circle on its beat, like a hit in the
/// game; the bar loops.
fn notes() -> String {
    PATTERN
        .chars()
        .enumerate()
        .filter_map(|(i, c)| {
            let class = match c {
                'd' => "d",
                'k' => "k",
                'D' => "d big",
                'K' => "k big",
                _ => return None,
            };
            Some(format!("<i class=\"n {class}\" style=\"--t:{}s\"></i>", i as f64 * 0.25))
        })
        .collect()
}

/// The home page's playfield: a lane of notes scrolling at the drum while mods switch on and
/// off it, with the game's HUD (filled in with the server's own numbers) around it.
fn stage(players: usize, plays: usize, online: usize, bg: &str, cta: &str) -> String {
    let notes = notes();
    let (flashes, drum) = drum();
    let lane = |class: &str| {
        let segs: String = (0..10i32).map(|i| format!("<b class=seg style=\"--dy:{};--r:{}\"></b>", (i * 37) % 9 - 4, (i * 53) % 7 - 3)).collect();
        format!("<div class=\"lane {class}\"><div class=segs>{segs}</div>{drum}<div class=judge></div><div class=track>{notes}</div></div>")
    };
    let mods = ["NM no mod", "TL tilt", "WV wave", "BA barrel roll", "MR mirror", "MV multiverse", "SH shatter"];
    let labels: String = mods.iter().chain(&mods[..1]).map(|m| format!("<span>{m}</span>")).collect();
    format!(
        "{flashes}<section class=stage{bg} aria-label=\"taiko.rs gameplay, switching mods\">\
         <div class=hud-tl>{online} online &middot; {players} ranked &middot; server up</div>\
         <div class=hud-tr aria-hidden=true><div class=hud-score>{:08}</div><div class=hp><i></i></div><div class=modname><div class=ticker>{labels}</div></div></div>\
         <div class=intro><h1 class=glitch data-text=\"TAIKO.RS\">TA<span class=narrow>I</span>KO.RS</h1>\
           <p>A taiko client for osu! beatmaps, with <em>{} mods</em> that bend, break and multiply the lane.</p></div>\
         <div class=field aria-hidden=true>{}{}{}{}<div class=hit-great>GREAT</div></div>\
         <div class=stage-cta>{}</div><div class=errbar aria-hidden=true><i></i><i></i><i></i><i></i><i></i></div></section>",
        plays,
        mod_count(),
        lane("ghost g1"),
        lane("ghost g2"),
        lane("ghost g3"),
        lane("main"),
        cta,
    )
}

fn clip_card(c: &Clip) -> String {
    format!(
        "<figure class=\"clip fx-{4}\" tabindex=0><div class=frame><img src=\"/clips/{0}.jpg\" alt=\"\" width=960 height=540 loading=lazy>\
         <video src=\"/clips/{0}.mp4\" poster=\"/clips/{0}.jpg\" width=960 height=540 autoplay muted loop playsinline preload=metadata aria-label=\"{1} gameplay\"></video></div>\
         <figcaption><div class=clip-head><b>{1}</b><span class=mods>{2}</span></div><p>{3}</p></figcaption></figure>",
        c.file,
        c.title,
        mod_chips(c.mods, 1.0),
        c.what,
        c.fx
    )
}

/// The game's mod select: every mod in its column, what it does when hovered or focused.
fn mod_board() -> String {
    let cols: String = groups()
        .iter()
        .map(|g| {
            let chips: String = g
                .mods
                .iter()
                .map(|m| {
                    format!(
                        "<li class=chip tabindex=0><b>{0}</b><span class=name>{1}</span><span class=desc><b>{1} ({0})</b>{2}</span></li>",
                        esc(&m.acronym),
                        esc(&m.name),
                        esc(&m.description)
                    )
                })
                .collect();
            format!("<div class=col style=\"--c:{}\"><h3>{}</h3><ul>{chips}</ul></div>", g.colour, esc(&g.title))
        })
        .collect();
    format!(
        "<section class=\"panel board\"><div class=panel-head><h2>MODS</h2><span class=dim>{} of them, in the game's mod select order. Point at one to read it.</span></div>\
         <div class=cols>{cols}</div><div class=descbar>Point at a mod (or tab to it) to see what it does.</div></section>",
        mod_count()
    )
}

fn online_list(online: &[OnlinePlayer]) -> String {
    if online.is_empty() {
        return "<p class=dim>Nobody's on right now. Be the first.</p>".into();
    }
    let rows: String = online
        .iter()
        .take(12)
        .map(|p| format!("<li><span class=dot></span>{}{}<span class=what>{}</span></li>", avatar(&p.name, "tiny"), player_link(&p.name), esc(&p.activity.describe())))
        .collect();
    let more = if online.len() > 12 { format!("<p class=dim>and {} more</p>", online.len() - 12) } else { String::new() };
    format!("<ul class=people>{rows}</ul>{more}")
}

fn room_list(rooms: &[RoomInfo]) -> String {
    if rooms.is_empty() {
        return "<p class=dim>No rooms open. Make one from the game's multiplayer menu.</p>".into();
    }
    rooms
        .iter()
        .take(8)
        .map(|r| {
            let state = if r.in_match { "<span class=\"tag live\">PLAYING</span>" } else { "<span class=tag>OPEN</span>" };
            let state = if r.mapguessr { format!("<span class=tag>MAPGUESSR</span>{state}") } else { state.to_string() };
            let map = r.map.as_deref().map(|m| format!("<div class=\"dim small\">{}</div>", esc(m))).unwrap_or_default();
            format!("<div class=room><div class=room-top><b>{}</b>{state}</div><div class=dim>host {} &middot; {} player{}</div>{map}</div>", esc(&r.title), esc(&r.host), r.players, if r.players == 1 { "" } else { "s" })
        })
        .collect()
}

/// A play in a feed or a list: grade, who (optionally), map, mods, accuracy and pp.
fn feed_row(s: &OnlineScore) -> String {
    let (song, version) = split_map(&s.map);
    format!(
        "<li class=feed-row>{}<div class=what>{} <span class=dim>on</span> <a href=\"/m/{:x}\">{}</a><div class=\"meta dim\"><span>{}</span>{}<span>{}</span></div></div><div class=right><span class=pp>{}</span><span class=dim>{:.2}%</span></div></li>",
        grade(s.grade()),
        player_link(&s.player),
        s.map_hash,
        esc(song),
        esc(version),
        mod_chips(&s.mods, s.rate),
        ago(s.date),
        s.pp.map(|p| format!("{p:.0}pp")).unwrap_or_default(),
        s.accuracy * 100.0
    )
}

/// The in-game leaderboard look: place, name, score-like pp.
fn top_player(r: &Ranked) -> String {
    format!(
        "<li><span class=place>{}.</span>{}{}<span class=pp>{}pp</span><span class=\"dim wide\">{:.2}%</span></li>",
        r.rank,
        avatar(&r.name, "tiny"),
        player_link(&r.name),
        thousands(r.pp.round() as u64),
        r.accuracy * 100.0
    )
}

pub fn home(rankings: &[Ranked], maps: &[MapRow], players: usize, plays: usize, os: Os, live: Live) -> String {
    let bg = maps.iter().find_map(|m| cover(m.set_id, "cover@2x")).map(|u| format!(" style=\"--cover:url('{u}')\"")).unwrap_or_default();
    let top: String = rankings.iter().take(8).map(top_player).collect();
    let top = if top.is_empty() { "<p class=dim>Nobody has sent a play yet.</p>".into() } else { format!("<ol class=board-list>{top}</ol>") };
    let latest: String = live.latest.iter().take(8).map(feed_row).collect();
    let latest = if latest.is_empty() { "<p class=dim>No plays yet.</p>".into() } else { format!("<ul class=feed>{latest}</ul>") };
    let clips: String = CLIPS[..3].iter().map(clip_card).collect();
    format!(
        "{}<div class=page>\
         <div class=hub>\
           <section class=panel><div class=panel-head><h2>ONLINE <span class=count>{}</span></h2></div>{}</section>\
           <section class=panel><div class=panel-head><h2>ROOMS <span class=count>{}</span></h2></div>{}</section>\
           <section class=\"panel wide-panel\"><div class=panel-head><h2>LATEST PLAYS</h2></div>{latest}</section>\
           <section class=panel><div class=panel-head><h2>TOP PLAYERS</h2><a class=more href=\"/rankings\">rankings</a></div>{top}</section>\
         </div>\
         <div class=section-head><h2 class=glitch-soft>MODS OSU! DOESN'T HAVE</h2><a class=more href=\"/mods\">all {} mods</a></div>\
         <div class=\"clips three\">{clips}</div>\
         <section class=panel><div class=panel-head><h2>RECENTLY PLAYED</h2><a class=more href=\"/beatmaps\">all beatmaps</a></div>{}</section></div>",
        stage(players, plays, live.online.len(), &bg, &download_cta(os, false)),
        live.online.len(),
        online_list(live.online),
        live.rooms.len(),
        room_list(live.rooms),
        mod_count(),
        map_cards(maps),
    )
}

pub fn mods_page() -> String {
    let clips: String = CLIPS.iter().map(clip_card).collect();
    format!(
        "<div class=band><div class=inner><h1 class=glitch data-text=\"MODS\">MODS</h1><p class=dim>osu!'s mods, lazer's taiko extras and {} of taiko.rs's own. Stack as many as you like.</p></div></div>\
         <div class=page><div class=section-head><h2>WHAT THEY LOOK LIKE</h2><span class=dim>Real plays, recorded in the game.</span></div><div class=clips>{clips}</div>{}</div>",
        mod_count().saturating_sub(24),
        mod_board()
    )
}

fn ranking_head() -> &'static str {
    "<thead><tr><th></th><th></th><th class=r>Accuracy</th><th class=\"r wide\">Plays</th><th class=r>pp</th><th class=\"r wide\">SS</th><th class=\"r wide\">S</th><th class=\"r wide\">A</th></tr></thead>"
}

fn ranking_row(r: &Ranked) -> String {
    format!(
        "<tr class=\"p{}\"><td class=place>#{}</td><td>{}{}</td><td class=\"r dim\">{:.2}%</td><td class=\"r dim wide\">{}</td><td class=\"r pp\">{}pp</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td><td class=\"r dim wide\">{}</td></tr>",
        r.rank.min(4),
        r.rank,
        avatar(&r.name, "tiny"),
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
    format!("<div class=band><div class=inner><h1 class=glitch data-text=\"RANKINGS\">RANKINGS</h1><p class=dim>Everyone by total pp, the best play counting most (osu!'s 95% weighting).</p></div></div><div class=page><section class=panel>{table}</section></div>")
}

/// The pp history as a step line, newest on the right.
fn pp_chart(history: &[(u64, f64)]) -> String {
    if history.len() < 2 {
        return "<p class=dim>The graph shows up after a few plays with pp.</p>".into();
    }
    let (w, h) = (600.0, 140.0);
    let (t0, t1) = (history[0].0 as f64, history[history.len() - 1].0 as f64);
    let top = history.iter().map(|p| p.1).fold(0.0, f64::max).max(1.0);
    let x = |t: u64| if t1 > t0 { (t as f64 - t0) / (t1 - t0) * w } else { w };
    let y = |pp: f64| h - pp / top * (h - 8.0);
    let mut d = format!("M0 {:.1}", y(history[0].1));
    for p in &history[1..] {
        d += &format!(" H{:.1} V{:.1}", x(p.0), y(p.1));
    }
    let area = format!("{d} H{w} V{h} H0Z");
    format!(
        "<div class=chart><svg viewBox=\"0 0 {w} {h}\" preserveAspectRatio=none aria-label=\"pp over time\"><path class=area d=\"{area}\"/><path class=line d=\"{d}\" vector-effect=non-scaling-stroke/></svg>\
         <div class=\"chart-axis dim\"><span>{}</span><span>{:.0}pp now</span></div></div>",
        ago(history[0].0),
        history[history.len() - 1].1
    )
}

pub fn profile(p: &Profile, extras: &Extras, activity: Option<&Activity>, friends: &[(String, bool)], tie: Friendship, notice: Option<&str>, roles: &Roles) -> String {
    let st = &p.stats;
    let level = online::level(st.total_score);
    let cover = p.top.first().map(|s| cover_style(s.set_id, s.map_hash, "cover@2x")).unwrap_or_default();
    let status = match activity {
        Some(a) => format!("<span class=\"status on\">ONLINE</span> <span class=dim>{}</span>", esc(&a.describe())),
        None if p.online => "<span class=\"status on\">ONLINE</span>".into(),
        None => "<span class=status>OFFLINE</span>".into(),
    };
    let rank = p.rank.map(|r| format!("#{}", thousands(r as u64))).unwrap_or_else(|| "-".into());
    let befriend = |label: &str| format!("<form method=post action=\"/friend/{0}\"><button class=btn>{label}</button></form>", esc(&p.name));
    let actions = match tie {
        Friendship::Me => "<a class=btn href=\"/chat\">open chat</a>".to_string(),
        Friendship::Friends => format!("<a class=\"btn hot small-hot\" href=\"/chat/{0}\">message</a><span class=\"tag live\">FRIENDS</span>", esc(&p.name)),
        Friendship::TheyAsked => befriend("accept friend request"),
        Friendship::Asked => "<span class=tag>FRIEND REQUEST SENT</span>".to_string(),
        Friendship::None => befriend("add friend"),
    };
    let notice = notice.map(|n| format!("<div class=notice>{}</div>", esc(n))).unwrap_or_default();
    let notice = notice + &profile_tools(p, roles);
    let hud = |k: &str, v: String| format!("<div class=hudstat><span>{k}</span><b>{v}</b></div>");
    let stats = [
        hud("RANKED SCORE", format!("{:08}", st.ranked_score)),
        hud("ACCURACY", format!("{:.2}%", st.accuracy * 100.0)),
        hud("PLAY COUNT", thousands(p.plays as u64)),
        hud("TOTAL SCORE", format!("{:08}", st.total_score)),
        hud("TOTAL HITS", thousands(st.total_hits)),
        hud("MAX COMBO", format!("{}x", thousands(st.max_combo as u64))),
        hud("PLAY TIME", play_time(p.play_ms)),
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
    let fav_mods = if extras.mods.is_empty() {
        "<p class=dim>Plays without mods so far.</p>".into()
    } else {
        extras.mods.iter().take(12).map(|(m, n)| format!("<span class=modcount>{}<span class=dim>{n}</span></span>", mod_chips(&[m], 1.0))).collect::<String>()
    };
    let most_played = if extras.most_played.is_empty() {
        "<p class=dim>Nothing yet.</p>".into()
    } else {
        extras
            .most_played
            .iter()
            .map(|(m, n)| {
                let (song, version) = split_map(&m.name);
                format!(
                    "<a class=mini href=\"/m/{:x}\"{}><span class=art></span><span class=mini-info><b>{}</b><span class=dim>{}</span></span><span class=times>{n}x</span></a>",
                    m.hash,
                    cover_style(m.set_id, m.hash, "list@2x"),
                    esc(song),
                    esc(version)
                )
            })
            .collect::<String>()
    };
    let friends_html = if friends.is_empty() {
        "<p class=dim>No friends added yet.</p>".into()
    } else {
        let rows: String = friends
            .iter()
            .take(16)
            .map(|(f, on)| format!("<li>{}{}{}</li>", if *on { "<span class=dot></span>" } else { "<span class=\"dot off\"></span>" }, avatar(f, "tiny"), player_link(f)))
            .collect();
        format!("<ul class=people>{rows}</ul>")
    };
    let best = if p.top.is_empty() {
        "<p class=dim>No performance records yet.</p>".into()
    } else {
        p.top.iter().enumerate().map(|(i, s)| play_row(s, Some(i), false)).collect::<String>()
    };
    let firsts = if p.firsts.is_empty() { "<p class=dim>No first place ranks yet.</p>".into() } else { p.firsts.iter().map(|s| play_row(s, None, false)).collect::<String>() };
    let recent = if p.recent.is_empty() { "<p class=dim>No recent plays.</p>".into() } else { p.recent.iter().map(|s| play_row(s, None, true)).collect::<String>() };
    format!(
        "<div class=cover{cover}></div>\
         <div class=page><div class=\"panel head\">\
           <div class=who>{}<div class=id><div class=name-row><h1>{}</h1>{}</div><div>{status}</div><div class=dim>joined {}</div><div class=actions>{actions}</div></div></div>\
           <div class=ranks><div class=bigstat><span>GLOBAL RANK</span><b>{rank}</b></div><div class=bigstat><span>PP</span><b class=pp>{}</b></div>\
             <div class=level><span>LEVEL <b>{}</b></span><div class=bar><i style=\"width:{:.0}%\"></i></div></div></div>\
         </div>{notice}\
         <div class=hudrow>{stats}</div>\
         <div class=cols-side><div class=main>\
             <section class=panel><div class=panel-head><h2>PP HISTORY</h2></div>{}</section>\
             <section class=panel><div class=panel-head><h2>BEST PERFORMANCE</h2></div>{best}</section>\
             <section class=panel><div class=panel-head><h2>FIRST PLACE RANKS <span class=count>{}</span></h2></div>{firsts}</section>\
             <section class=panel><div class=panel-head><h2>RECENT PLAYS</h2></div>{recent}</section>\
           </div><aside class=side>\
             <section class=panel><div class=panel-head><h2>GRADES</h2></div>{grades}</section>\
             <section class=panel><div class=panel-head><h2>FAVOURITE MODS</h2></div><div class=modcounts>{fav_mods}</div></section>\
             <section class=panel><div class=panel-head><h2>MOST PLAYED</h2></div>{most_played}</section>\
             <section class=panel><div class=panel-head><h2>FRIENDS <span class=count>{}</span></h2></div>{friends_html}</section>\
           </aside></div></div>",
        avatar(&p.name, "big"),
        esc(&p.name),
        badges(&p.name) + &role_badges(roles),
        month_year(p.joined),
        thousands(p.pp.round() as u64),
        level.floor(),
        level.fract() * 100.0,
        pp_chart(&extras.history),
        p.firsts.len(),
        friends.len(),
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
                    format!("<div class=weight>{:.0}% = {:.0}pp</div>", k * 100.0, pp * k)
                })
                .unwrap_or_default();
            format!("<div class=pp>{pp:.0}pp{w}</div>")
        }
        None => "<div class=pp>-</div>".into(),
    };
    let score = if show_score { format!("<span class=\"dim score\">{:08}</span>", s.score) } else { String::new() };
    format!(
        "<div class=play>{}<div class=what><a href=\"/m/{:x}\">{}</a><div class=meta>{}<span class=version>{}</span><span class=dim>{}</span></div></div><div class=mods>{}</div>{score}<div class=acc>{:.2}%</div>{pp}</div>",
        grade(s.grade()),
        s.map_hash,
        esc(song),
        stars(s.stars),
        esc(version),
        ago(s.date),
        mod_chips(&s.mods, s.rate),
        s.accuracy * 100.0
    )
}

/// One score: its numbers, and a button that opens it (and its replay) in the game, through
/// the `taiko-rs://` link the game registers. `watch`: from a "Watch replay" button, so the
/// page opens the game itself straight away (`site/watch.js`).
/// `admin`: the viewer is an admin, and this play's pp as sent and as set by an admin (if).
pub fn score_page(s: &OnlineScore, place: Option<u32>, watch: bool, admin: Option<(Option<f64>, Option<f64>)>, notice: Option<&str>) -> String {
    let (song, version) = split_map(&s.map);
    let (artist, title) = split_song(song);
    let head = format!(
        "<div class=\"cover map\"{}><div class=inner><div class=artist>{}</div><h1>{}</h1>\
         <div class=sub>{}<span class=version>[{}]</span><a class=\"btn small\" href=\"/m/{:x}\">map leaderboard</a></div></div></div>",
        cover_style(s.set_id, s.map_hash, "cover@2x"),
        esc(artist),
        esc(title),
        stars(s.stars),
        esc(version),
        s.map_hash
    );
    let mods = if s.mods.is_empty() && (s.rate - 1.0).abs() < 1e-6 { "-".into() } else { mod_chips(&s.mods, s.rate) };
    let place = place.map(|p| format!("<div class=place>#{p}</div>")).unwrap_or_default();
    let top = format!(
        "<div class=\"panel topscore\">{place}<span class=bigrade>{}</span><div class=who>{}<div>{}<div class=dim>{}</div></div></div>\
         <div class=hudrow-in><div class=hudstat><span>PP</span><b class=pp>{}</b></div><div class=hudstat><span>ACCURACY</span><b>{:.2}%</b></div>\
         <div class=hudstat><span>GREAT</span><b class=great>{}</b></div><div class=hudstat><span>OK</span><b class=ok>{}</b></div><div class=hudstat><span>MISS</span><b class=miss>{}</b></div>\
         <div class=hudstat><span>COMBO</span><b>{}x</b></div><div class=hudstat><span>SCORE</span><b>{:08}</b></div><div class=hudstat><span>MODS</span><b class=mods>{mods}</b></div></div></div>",
        grade(s.grade()),
        avatar(&s.player, "mid"),
        player_link(&s.player),
        ago(s.date),
        s.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
        s.accuracy * 100.0,
        s.great,
        s.ok,
        s.miss,
        thousands(s.max_combo as u64),
        s.score,
    );
    let auto = if watch { " data-autoopen" } else { "" };
    let watch_panel = format!(
        "<section class=panel><a class=\"btn hot\" href=\"taiko-rs://score/{}\"{auto}>Watch replay in taiko.rs</a>\
         <p class=hint>Opens the game on this score, ready to watch. It downloads the map if you haven't got it. \
         Nothing happens? <a href=\"/download\">Get taiko.rs</a>, open it once, and try again.</p></section>",
        s.id
    );
    let notice = notice.map(|n| format!("<div class=notice>{}</div>", esc(n))).unwrap_or_default();
    let admin = admin
        .map(|(sent, fixed)| {
            let sent = sent.map(|p| format!("{p:.2}pp")).unwrap_or_else(|| "no pp".into());
            let now = fixed.map(|p| format!("set to {p:.2}pp by an admin")).unwrap_or_else(|| "as sent".into());
            format!(
                "<section class=\"panel tools\"><div class=tool><b>ADMIN</b><form method=post action=\"/s/{}/pp\"><input name=pp inputmode=decimal placeholder=\"pp (empty: as sent)\" value=\"{}\" aria-label=pp><button class=\"btn small\">set pp</button></form>\
                 <span class=dim>sent with {sent}; {now}</span></div></section>",
                s.id,
                fixed.map(|p| format!("{p:.2}")).unwrap_or_default()
            )
        })
        .unwrap_or_default();
    format!("{head}<div class=page>{notice}{top}{watch_panel}{admin}</div><script src=\"/site/watch.js\" defer></script>")
}

pub fn map_page(m: &MapRow, rows: &[OnlineScore], love: &Love) -> String {
    let (song, version) = split_map(&m.name);
    let (artist, title) = split_song(song);
    let mut osu = m.set_id.map(|s| format!("<a class=\"btn small\" href=\"https://osu.ppy.sh/beatmapsets/{s}\">osu! page</a>")).unwrap_or_default();
    if let Some(l) = &love.loved {
        osu += &format!("<span class=\"tag loved\" title=\"loved by {0} {1}\">LOVED HERE</span>", esc(&l.by), ago(l.at));
    }
    if let (true, Some(set)) = (love.can_love, m.set_id) {
        let (on, label) = if love.loved.is_some() { ("0", "unlove") } else { ("1", "love: give pp here") };
        osu += &format!(
            "<form method=post action=\"/love/{set}\" class=inline><input type=hidden name=on value={on}><input type=hidden name=back value=\"/m/{:x}\"><button class=\"btn small hot\">{label}</button></form>",
            m.hash
        );
    }
    let notice = love.notice.as_ref().map(|n| format!("<div class=notice>{}</div>", esc(n))).unwrap_or_default();
    let head = format!(
        "<div class=\"cover map\"{}><div class=inner><div class=artist>{}</div><h1>{}</h1>\
         <div class=sub>{}<span class=version>[{}]</span><span class=dim>{} plays &middot; {} players</span>{osu}</div></div></div>",
        cover_style(m.set_id, m.hash, "cover@2x"),
        esc(artist),
        esc(title),
        stars(m.stars),
        esc(version),
        thousands(m.plays as u64),
        m.players
    );
    let Some(first) = rows.first() else { return format!("{head}<div class=page>{notice}<section class=panel><p class=dim>No scores yet.</p></section></div>") };
    let mods = if first.mods.is_empty() && (first.rate - 1.0).abs() < 1e-6 { "-".into() } else { mod_chips(&first.mods, first.rate) };
    let top = format!(
        "<div class=\"panel topscore\"><div class=place>#1</div><span class=bigrade>{}</span><div class=who>{}<div>{}<div class=dim>{}</div></div></div>\
         <div class=hudrow-in><div class=hudstat><span>SCORE</span><b>{:08}</b></div><div class=hudstat><span>ACCURACY</span><b>{:.2}%</b></div><div class=hudstat><span>COMBO</span><b>{}x</b></div>\
         <div class=hudstat><span>PP</span><b class=pp>{}</b></div><div class=hudstat><span>MODS</span><b class=mods>{mods}</b></div></div></div>",
        grade(first.grade()),
        avatar(&first.player, "mid"),
        player_link(&first.player),
        ago(first.date),
        first.score,
        first.accuracy * 100.0,
        thousands(first.max_combo as u64),
        first.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
    );
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                "<tr class=\"p{}\"><td class=place>#{}</td><td>{}</td><td>{:08}</td><td>{:.2}%</td><td>{}{}</td><td class=wide>{}x</td><td class=\"wide great\">{}</td><td class=\"wide ok\">{}</td><td class=\"wide miss\">{}</td><td class=pp>{}</td><td class=\"wide mods\">{}</td><td class=dim>{}</td></tr>",
                (i + 1).min(4),
                i + 1,
                grade(r.grade()),
                r.score,
                r.accuracy * 100.0,
                avatar(&r.player, "tiny"),
                player_link(&r.player),
                thousands(r.max_combo as u64),
                r.great,
                r.ok,
                r.miss,
                r.pp.map(|p| format!("{p:.0}")).unwrap_or_else(|| "-".into()),
                mod_chips(&r.mods, r.rate),
                ago(r.date)
            )
        })
        .collect();
    format!(
        "{head}<div class=page>{notice}{top}<section class=panel><div class=scroll><table class=list><thead><tr><th>Rank</th><th></th><th>Score</th><th>Acc</th><th>Player</th><th class=wide>Combo</th><th class=wide>Great</th><th class=wide>Ok</th><th class=wide>Miss</th><th>pp</th><th class=wide>Mods</th><th></th></tr></thead><tbody>{body}</tbody></table></div></section></div>"
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
                 <span class=meta>{}<span class=version>{}</span></span><span class=foot>{} plays &middot; {}</span></span></a>",
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
    format!("<div class=band><div class=inner><h1 class=glitch data-text=\"BEATMAPS\">BEATMAPS</h1><p class=dim>Every map someone has sent a play on, the latest first.</p></div></div><div class=page>{}</div>", map_cards(maps))
}

pub fn download(os: Os) -> String {
    let builds: String = BUILDS
        .iter()
        .map(|b| {
            let mine = b.os == os;
            let hot = if mine { " hot" } else { "" };
            let button = format!("<a class=\"btn{hot}\" href=\"{}\">Download {}</a>", download_url(b.file), b.kind);
            let alt = |(file, label): (&str, &str)| format!("<a class=alt href=\"{}\">or the {label}</a>", download_url(file));
            let how = match b.os {
                // Linux: the command, the archive for those who'd rather.
                Os::Linux => format!("<h3>In a terminal</h3>{}{}", cmd(&linux_command()), alt((b.file, b.kind))),
                Os::Windows => format!("{button}{}<h3>Or from PowerShell</h3>{}", b.alt.map(alt).unwrap_or_default(), cmd(&powershell_command())),
                _ => button,
            };
            format!(
                "<div class=\"panel build{}\"><h2>{}{}</h2><p class=dim>{}</p>{how}</div>",
                if mine { " mine" } else { "" },
                b.name,
                if mine { " <span class=yours>YOUR SYSTEM</span>" } else { "" },
                b.note,
            )
        })
        .collect();
    format!(
        "<div class=band><div class=inner><h1 class=glitch data-text=\"DOWNLOAD\">DOWNLOAD</h1><div class=band-cta>{}</div></div></div>\
         <div class=page><div class=builds>{builds}</div>\
         <p class=dim>No macOS build yet. Source and every build: <a href=\"{REPO}/releases\">GitHub releases</a>.</p></div>",
        download_cta(os, true)
    )
}

/// The login form; accounts are made in the game.
pub fn login(error: Option<&str>, name: &str) -> String {
    let error = error.map(|e| format!("<div class=\"notice bad\">{}</div>", esc(e))).unwrap_or_default();
    format!(
        "<div class=page><section class=\"panel login\"><h1 class=glitch data-text=\"LOG IN\">LOG IN</h1>\
         <p class=dim>The same name and password as in the game.</p>{error}\
         <form method=post action=\"/login\"><label>name<input name=name value=\"{}\" maxlength=16 autocomplete=username required autofocus></label>\
         <label>password<input name=password type=password maxlength=128 autocomplete=current-password required></label>\
         <button class=\"btn hot\">log in</button></form>\
         <p class=dim>No account yet? Make one in the game: Multiplayer, then \"create account\".</p></section></div>",
        esc(name)
    )
}

fn chat_line(l: &ChatLine) -> String {
    let t = l.at % 86_400;
    format!("<li><span class=time>{:02}:{:02}</span>{}<span class=text>{}</span></li>", t / 3600, t / 60 % 60, player_link(&l.from), esc(&l.text))
}

/// The chat, like osu!'s: channels on the left (the global one and every friend), the open one
/// on the right with a box to type in. `with`: the friend open (None: the global chat);
/// `can_send`: false when that's someone who isn't a friend.
pub fn chat(me: &str, with: Option<&str>, lines: &[ChatLine], people: &[Conversation], can_send: bool, error: Option<&str>) -> String {
    let channel = |href: &str, label: String, on: bool, extra: String| format!("<a class=\"chan{}\" href=\"{href}\">{label}{extra}</a>", if on { " on" } else { "" });
    let mut list = channel("/chat", "<span class=hash>#</span>global".into(), with.is_none(), String::new());
    for c in people {
        let on = with.is_some_and(|w| w.eq_ignore_ascii_case(&c.name));
        let last = c.last.as_ref().map(|l| format!("<span class=last>{}</span>", esc(&l.text))).unwrap_or_default();
        list += &channel(&format!("/chat/{}", esc(&c.name)), format!("<span class=\"dot{}\"></span>{}<span class=cname>{}</span>", if c.online { "" } else { " off" }, avatar(&c.name, "tiny"), esc(&c.name)), on, last);
    }
    if people.is_empty() {
        list += "<p class=\"dim small\">Add friends from their profiles to message them.</p>";
    }
    let (title, api, action) = match with {
        Some(w) => (format!("{} {}", avatar(w, "tiny"), player_link(w)), format!("/api/chat/{}", esc(w)), format!("/chat/{}", esc(w))),
        None => ("<span class=hash>#</span>global".to_string(), "/api/chat".to_string(), "/chat".to_string()),
    };
    let log: String = lines.iter().map(chat_line).collect();
    let empty = if lines.is_empty() { "<li class=\"dim empty\">No messages yet. Say hi.</li>" } else { "" };
    let error = error.map(|e| format!("<div class=\"notice bad\">{}</div>", esc(e))).unwrap_or_default();
    let say = if can_send {
        format!("<form class=say method=post action=\"{action}\"><input name=text maxlength=300 placeholder=\"type a message\" autocomplete=off required aria-label=\"Message\"><button class=\"btn hot\">send</button></form>")
    } else {
        format!("<div class=\"notice\">You can only message friends. <a href=\"/u/{0}\">Add {0} as a friend</a> first.</div>", esc(with.unwrap_or("")))
    };
    format!(
        "<div class=page><div class=chat><aside class=\"panel chans\"><div class=panel-head><h2>CHAT</h2></div>{list}</aside>\
         <section class=\"panel convo\" data-chat=\"{api}\" data-me=\"{}\"><div class=panel-head><h2>{title}</h2><span class=\"dim small\">also in the game (F9)</span></div>\
         <ol class=log>{log}{empty}</ol>{error}{say}</section></div></div><script src=\"/site/chat.js\" defer></script>",
        esc(me)
    )
}

pub fn not_found(why: &str) -> String {
    format!("<div class=page><section class=\"panel lost\"><h1 class=glitch data-text=\"MISS\">MISS</h1><p class=dim>{}.</p><p><a class=btn href=\"/\">Back to the menu</a></p></section></div>", esc(why))
}

/// A whole page: `nav` is the header link to mark as the current one ("" for none); `viewer`
/// the logged-in player, if any (their profile and chat in the header, else a log in button).
pub fn page(title: &str, nav: &str, viewer: Option<&str>, meta: &Meta, body: &str) -> String {
    let image = meta.image.clone().unwrap_or_else(|| format!("{SITE}/site/og.jpg?v={}", card_version()));
    let og = format!(
        "<meta name=description content=\"{0}\"><meta property=og:site_name content=taiko.rs><meta property=og:type content=website>\
         <meta property=og:title content=\"{1}\"><meta property=og:description content=\"{0}\"><meta property=og:image content=\"{2}\">\
         <meta name=twitter:card content=summary_large_image><meta name=twitter:title content=\"{1}\"><meta name=twitter:description content=\"{0}\"><meta name=twitter:image content=\"{2}\">",
        esc(&meta.description),
        esc(title),
        esc(&image)
    );
    let account = match viewer {
        Some(me) => {
            let (name, pic, on) = (esc(me), avatar(me, "tiny"), if nav == "chat" { " on" } else { "" });
            format!(
                "<a class=me href=\"/u/{name}\" title=\"my profile\">{pic}<span>{name}</span></a><a class=\"navbtn{on}\" href=\"/chat\">chat</a>\
                 <form method=post action=\"/logout\"><button class=navbtn title=\"log out\" aria-label=\"log out\">x</button></form>"
            )
        }
        None => "<a class=\"navbtn hot\" href=\"/login\">log in</a>".into(),
    };
    let links: String = [("rankings", "/rankings"), ("beatmaps", "/beatmaps"), ("loved", "/loved"), ("mods", "/mods"), ("download", "/download")]
        .iter()
        .map(|(name, href)| {
            let on = if *name == nav { " class=on aria-current=page" } else { "" };
            format!("<a href=\"{href}\"{on}>{name}</a>")
        })
        .collect();
    format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{} · taiko.rs</title>
<meta name=theme-color content=#EB452C><meta name=color-scheme content=dark><link rel=icon href="/site/logo.png">{og}
<link rel=preload href="/site/taiko5x7.woff2" as=font type="font/woff2" crossorigin><style>{STYLE}</style><script src="/site/copy.js" defer></script></head>
<body><nav><div class=inner><a class=logo href="/" aria-label="taiko.rs home"><img src="/site/logo.png" alt="" width=32 height=32>TA<span class=narrow>I</span>KO.RS</a><div class=links>{links}</div>
<form action="/u" role=search><input name=name placeholder="find a player" maxlength=16 aria-label="Find a player"></form><div class=account>{account}</div></div></nav>
<main>{}</main><footer><div class=inner><span class=brand>TAIKO.RS</span><a href="{REPO}">github</a><a href="/download">download</a><a href="/mods">mods</a><a href="{REPO}/blob/worktree-taiko-client/LICENSE">MIT license</a><span class=dim>Not affiliated with ppy or osu!. Beatmap covers load from osu!'s servers.</span></div></footer></body></html>"#,
        esc(title),
        body
    )
}

const STYLE: &str = r#"
@font-face{font-family:taiko5x7;src:url(/site/taiko5x7.woff2) format("woff2");font-display:swap}
@property --amp{syntax:"<number>";inherits:true;initial-value:0}
@property --sh{syntax:"<number>";inherits:true;initial-value:0}
@property --mv{syntax:"<number>";inherits:true;initial-value:0}
:root{--bg:#0b0b10;--bg2:#0c0c12;--panel:#15151d;--panel2:#20202b;--line:#2a2a38;--line2:#3a3a4e;--text:#c8c8d8;--white:#fff;--dim:#8a8aa0;--dim2:#6f6f86;
--acc:#EB452C;--don:#EB452C;--kat:#438EAD;--katl:#4fb4ff;--gold:#FFD34D;--green:#86E05A;--pink:#ff5fa2;--purple:#c77dff;--px:2px}
*{box-sizing:border-box}html{background:var(--bg);color-scheme:dark}
body{margin:0;color:var(--text);background:var(--bg);font:16px/24px taiko5x7,ui-monospace,monospace;-webkit-font-smoothing:none;font-synthesis:none;text-rendering:optimizeSpeed}
b,strong,h1,h2,h3{font-weight:400}.narrow{margin:0 -.125em}em{font-style:normal;color:var(--acc)}
a{color:var(--white);text-decoration:none}a:hover{color:var(--acc)}.dim{color:var(--dim)}.small{font-size:16px}
h1,h2,h3{margin:0;line-height:1.2;color:var(--white)}p{margin:8px 0}
:focus-visible{outline:2px solid var(--acc);outline-offset:2px}
img.pix,.logo img{image-rendering:pixelated}
.inner,.page{max-width:1280px;margin:0 auto;padding:0 24px}.page{padding-top:24px;padding-bottom:56px}main{min-height:60vh}
nav{background:var(--bg2);border-bottom:2px solid var(--line);position:sticky;top:0;z-index:20}
nav .inner{display:flex;align-items:center;gap:24px;height:56px}
.logo{display:flex;align-items:center;gap:10px;color:var(--white)!important;font-size:24px;letter-spacing:2px}
.links{display:flex;height:100%}.links a{display:flex;align-items:center;padding:0 14px;color:var(--dim);border-bottom:2px solid transparent;margin-bottom:-2px}
.links a:hover{color:var(--white)}.links a.on{color:var(--white);border-bottom-color:var(--acc);background:linear-gradient(0deg,rgba(235,69,44,.12),transparent)}
nav form{margin-left:auto}nav input{font:inherit;width:240px;height:36px;background:var(--panel);border:2px solid var(--line);color:var(--white);padding:0 12px}
nav input::placeholder{color:var(--dim2)}nav input:focus{outline:none;border-color:var(--acc)}
.btn{display:inline-flex;align-items:center;min-height:44px;padding:0 20px;background:var(--panel2);border:2px solid var(--line2);color:var(--white)!important;transition:border-color .12s,background .12s,transform .12s}
.btn:hover{border-color:var(--acc);background:#2a1c22}.btn:active{transform:translateY(2px)}
.btn.hot{background:var(--acc);border-color:var(--acc);font-size:24px;min-height:52px;padding:0 28px;box-shadow:4px 4px 0 #6b1d12}
.btn.hot:hover{background:#ff5a3d;border-color:#ff5a3d}.btn.small{min-height:32px;padding:0 12px}.hint{color:var(--dim);margin-top:10px}
/* The glitch the logo has: the red and the blue slipping off the white. */
.glitch{position:relative;display:inline-block;color:var(--white);text-shadow:-3px 0 var(--don),3px 0 var(--katl)}
.glitch:before,.glitch:after{content:attr(data-text);position:absolute;inset:0;pointer-events:none}
.glitch:before{color:var(--don);clip-path:inset(0 0 62% 0);transform:translate(-6px,0);animation:slip 3.1s steps(1) infinite}
.glitch:after{color:var(--katl);clip-path:inset(58% 0 0 0);transform:translate(6px,0);animation:slip 2.3s steps(1) infinite reverse}
@keyframes slip{0%,82%{transform:translate(0);opacity:0}84%{transform:translate(-8px,2px);opacity:1}88%{transform:translate(10px,-2px);opacity:1}92%{transform:translate(-4px,0);opacity:1}94%,100%{transform:translate(0);opacity:0}}
.glitch-soft{text-shadow:-2px 0 var(--don),2px 0 var(--katl)}
/* The home page's playfield. */
.stage{position:relative;height:620px;overflow:hidden;border-bottom:2px solid var(--line);
background:linear-gradient(0deg,var(--bg),transparent 30%),linear-gradient(rgba(11,11,16,.88),rgba(11,11,16,.88)),var(--cover,none) center/cover,var(--bg)}
.stage:after{content:"";position:absolute;inset:0;pointer-events:none;background:repeating-linear-gradient(0deg,rgba(0,0,0,.18) 0 2px,transparent 2px 4px)}
.hud-tl{position:absolute;top:12px;left:24px;color:var(--green);opacity:.85}
.hud-tr{position:absolute;top:12px;right:24px;text-align:right;z-index:3}
.hud-score{font-size:64px;line-height:64px;color:var(--white);letter-spacing:2px}
.hp{width:280px;height:8px;margin:8px 0 0 auto;background:var(--panel2);border:2px solid var(--line2)}.hp i{display:block;height:100%;width:70%;background:var(--gold);animation:hp 7s ease-in-out infinite alternate}
@keyframes hp{0%{width:35%;background:var(--acc)}100%{width:96%;background:var(--gold)}}
.modname{height:24px;overflow:hidden;margin-top:10px;color:var(--gold);text-transform:uppercase}.ticker{animation:ticker 28s linear infinite}.ticker span{display:block;height:24px;white-space:nowrap}
@keyframes ticker{0%,13.6%{transform:translateY(0)}14.3%,27.9%{transform:translateY(-24px)}28.6%,42.1%{transform:translateY(-48px)}42.9%,56.4%{transform:translateY(-72px)}57.1%,70.7%{transform:translateY(-96px)}71.4%,85%{transform:translateY(-120px)}85.7%,99.3%{transform:translateY(-144px)}100%{transform:translateY(-168px)}}
.intro{position:absolute;left:24px;top:84px;max-width:640px;z-index:3}
.intro h1{font-size:96px;line-height:96px;letter-spacing:4px}.intro p{font-size:24px;line-height:32px;color:var(--text);margin-top:16px}
.field{position:absolute;left:0;right:0;top:330px;height:140px;animation:mods 28s linear infinite;transform-origin:30% 50%}
.lane{position:absolute;left:0;right:0;top:0;height:140px;transform-origin:30% 50%}
.segs{position:absolute;inset:16px 0;display:flex}.seg{flex:1;background:#1b1b26;border-top:2px solid var(--line2);border-bottom:2px solid var(--line2);
transform:translateY(calc(var(--sh) * var(--dy) * 9px)) rotate(calc(var(--sh) * var(--r) * 2deg))}
.seg:first-child{flex:0 0 160px;background:var(--panel);border-right:2px solid var(--line2)}
/* The drum: a don lights half its face red, a kat half its rim blue; the combo counts up. */
.drum{position:absolute;left:44px;top:34px;width:72px;height:72px;border-radius:50%;background:#3a3a4e;box-shadow:0 0 0 6px #262634,0 0 0 8px var(--line2);display:grid;place-items:center}
.fl{position:absolute;inset:0;border-radius:50%;opacity:0;animation:12s step-end infinite}.fl.l{clip-path:inset(0 50% 0 0)}.fl.r{clip-path:inset(0 0 0 50%)}
.rim{background:var(--katl)}.rim.l{animation-name:katl}.rim.r{animation-name:katr}.don{background:var(--don)}.don.l{animation-name:donl}.don.r{animation-name:donr}
.face{position:relative;width:52px;height:52px;border-radius:50%;background:#f5ecdc;overflow:hidden;box-shadow:0 0 0 2px #262634}
.combo{position:absolute;inset:0;overflow:hidden;color:#15151d;font-size:16px}.digits{display:block;animation:combo 120s step-end infinite}
.digits span{display:block;height:3.25em;line-height:3.25em;text-align:center}
.judge{position:absolute;left:188px;top:38px;width:64px;height:64px;border-radius:50%;border:4px solid #4a4a60;box-shadow:inset 0 0 0 4px #1b1b26}
.track{position:absolute;left:160px;right:0;top:0;height:140px;overflow:hidden}
/* Each note: 6 s from off the right edge to the judgement circle, then a hit burst. */
.n{position:absolute;left:60px;top:50%;width:56px;height:56px;margin:-28px 0 0 -28px;border-radius:50%;background:radial-gradient(circle,#fff 0 12%,var(--c) 13% 64%,#fff 65% 74%,var(--c) 75%);
box-shadow:0 0 0 2px rgba(0,0,0,.35);transform:translateX(max(100vw,1800px));animation:fly 6s linear infinite,bob .9s ease-in-out infinite alternate;animation-delay:calc(var(--t) - 5.76s),calc(var(--t) * -1)}
.n:after{content:"";position:absolute;inset:-4px;border-radius:50%;border:4px solid #fff;opacity:0;animation:burst 6s linear infinite;animation-delay:calc(var(--t) - 5.76s)}
.n.d{--c:var(--don)}.n.k{--c:var(--kat)}.n.big{width:80px;height:80px;margin:-40px 0 0 -40px}
@keyframes fly{0%{transform:translateX(max(100vw,1800px));opacity:1;filter:none}96%{transform:translateX(0);opacity:1;filter:none}97.5%{transform:scale(1.18);opacity:1;filter:brightness(1.7)}100%{transform:scale(1.32);opacity:0;filter:brightness(2.2)}}
@keyframes burst{0%,96%{opacity:0;transform:scale(1)}97%{opacity:1;transform:scale(1.1)}100%{opacity:0;transform:scale(1.8)}}
@keyframes bob{from{translate:0 calc(var(--amp) * -1px)}to{translate:0 calc(var(--amp) * 1px)}}
.ghost{opacity:calc(var(--mv) * .5);filter:saturate(.6)}.g1{transform:translate(34%,-130px) rotate(-4deg) scale(.55)}.g2{transform:translate(-6%,160px) rotate(3deg) scale(.66)}.g3{transform:translate(28%,100px) rotate(-7deg) scale(.5)}
.main{transform:scale(calc(1 - var(--mv) * .25)) translateY(calc(var(--mv) * -18px));box-shadow:0 0 0 calc(var(--mv) * 3px) rgba(255,255,255,.55)}
.hit-great{position:absolute;left:182px;top:-8px;color:var(--gold);font-size:24px;animation:great .9s steps(1) infinite}@keyframes great{0%,40%{opacity:1;transform:translateY(0)}41%,100%{opacity:0;transform:translateY(-8px)}}
/* One mod after another, 4 s each: none, tilt, wave, barrel roll, mirror, multiverse, shatter. */
@keyframes mods{
0%,13%{transform:none;--amp:0;--mv:0;--sh:0}
15%,27%{transform:rotate(-7deg)}
29%{transform:none;--amp:0}31%,41%{--amp:30}
43%{transform:none;--amp:0}48%{transform:rotate(360deg)}56%{transform:rotate(360deg)}
57.1%{transform:scaleX(1)}58.5%,70%{transform:scaleX(-1)}
71.5%{transform:scaleX(1);--mv:0}73.5%,84%{--mv:1}
85.7%{--mv:0;--sh:0}88%,98%{--sh:1}100%{--sh:0;transform:none}}
.errbar{position:absolute;left:62%;bottom:44px;width:340px;height:8px;transform:translateX(-50%);background:linear-gradient(90deg,#8a2a20 0 15%,#5d7a2c 15% 35%,#c9a63a 35% 65%,#5d7a2c 65% 85%,#8a2a20 85%)}
.errbar i{position:absolute;top:-8px;width:2px;height:24px;background:var(--gold);animation:tick 1.7s steps(6) infinite}
.errbar i:nth-child(1){left:48%}.errbar i:nth-child(2){left:52%;animation-delay:-.4s}.errbar i:nth-child(3){left:44%;animation-delay:-.8s}.errbar i:nth-child(4){left:57%;animation-delay:-1.1s;background:var(--green)}.errbar i:nth-child(5){left:50%;width:4px;height:32px;top:-12px;background:#fff;animation:none}
@keyframes tick{0%{opacity:1}100%{opacity:.15}}
.stage-cta{position:absolute;left:24px;bottom:28px;z-index:3}.stage-cta .hint{max-width:520px}
/* Panels, as the game's menus draw them. */
.panel{background:var(--panel);border:2px solid var(--line);padding:18px 20px;margin-bottom:20px}
.panel-head{display:flex;align-items:baseline;gap:12px;margin:-2px 0 14px;flex-wrap:wrap}.panel-head h2{font-size:24px;letter-spacing:1px}
.count{color:var(--dim);font-size:16px}.more{margin-left:auto;color:var(--acc)}.more:hover{color:var(--white)}
.band{border-bottom:2px solid var(--line);padding:40px 0 28px;background:repeating-linear-gradient(0deg,rgba(0,0,0,.18) 0 2px,transparent 2px 4px),linear-gradient(180deg,var(--panel),var(--bg))}
.band h1{font-size:64px;line-height:64px;letter-spacing:3px}
.section-head{display:flex;align-items:baseline;gap:16px;margin:12px 0 18px;flex-wrap:wrap}.section-head h2{font-size:32px;letter-spacing:2px}
.hub{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 20px}.hub .panel{min-height:120px}
.people{list-style:none;margin:0;padding:0}.people li{display:flex;align-items:center;gap:10px;min-height:36px}.people .what{color:var(--dim);margin-left:auto;text-align:right;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:55%}
.dot{flex:none;width:8px;height:8px;background:var(--green)}.dot.off{background:var(--line2)}
.room{border-left:4px solid var(--line2);padding:6px 12px;margin-bottom:10px}.room-top{display:flex;gap:10px;align-items:center}.room-top b{color:var(--white)}
.tag{font-size:16px;color:var(--dim);border:2px solid var(--line2);padding:0 6px}.tag.live{color:var(--bg);background:var(--green);border-color:var(--green)}
.feed,.board-list{list-style:none;margin:0;padding:0}
.feed-row{display:flex;align-items:center;gap:14px;padding:8px 0;border-bottom:2px solid var(--line)}.feed-row:last-child{border:0}
.feed-row .what{flex:1;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.feed-row .meta{display:flex;gap:10px;align-items:center;overflow:hidden}
.feed-row .right{display:flex;flex-direction:column;align-items:flex-end}
.board-list li{display:flex;align-items:center;gap:10px;min-height:40px;padding:0 10px;background:linear-gradient(90deg,rgba(255,255,255,.04),transparent)}.board-list li:nth-child(even){background:none}
.place{width:40px;color:var(--dim)}.board-list .pp{margin-left:auto}.pp{color:var(--acc)}
.avatar{display:inline-grid;place-items:center;flex:none;background:hsl(var(--h) 45% 38%);color:#fff;box-shadow:inset -3px -3px 0 rgba(0,0,0,.25),inset 3px 3px 0 rgba(255,255,255,.15)}
.avatar.tiny{width:24px;height:24px;font-size:16px}.avatar.mid{width:56px;height:56px;font-size:32px;margin-right:14px}.avatar.big{width:136px;height:136px;font-size:96px;border:4px solid var(--panel)}
.grade{display:inline-block;width:40px;text-align:center;font-size:24px;line-height:24px;flex:none}
.gss{color:var(--gold);text-shadow:-2px 0 var(--don),2px 0 var(--katl)}.gs{color:var(--gold)}.ga{color:var(--green)}.gb{color:var(--katl)}.gc{color:var(--purple)}.gd{color:var(--acc)}
.mod{display:inline-block;padding:0 6px;margin:0 4px 2px 0;border:2px solid var(--c,var(--line2));color:var(--c,var(--dim));background:color-mix(in srgb,var(--c,#000) 14%,transparent);font-size:16px;line-height:20px}
.mod.rate{--c:var(--dim)}
.stars{display:inline-flex;align-items:center;gap:4px;padding:0 6px;font-size:16px;line-height:20px;white-space:nowrap}.star{width:10px;height:10px;fill:currentColor;shape-rendering:crispEdges}
.version{color:var(--white)}
/* Clips: each card does its own mod to itself when you point at it. */
.clips{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px;margin-bottom:28px}
.clip{margin:0;background:var(--panel);border:2px solid var(--line);display:flex;flex-direction:column;transition:border-color .12s}
.clip:hover,.clip:focus-visible{border-color:var(--acc);outline:none}
.frame{position:relative;aspect-ratio:16/9;background:#000;overflow:hidden}.frame img,.frame video{position:absolute;inset:0;width:100%;height:100%;object-fit:cover;display:block}
.clip figcaption{padding:12px 16px 16px}.clip-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.clip-head b{font-size:24px;color:var(--white)}.clip p{color:var(--dim);margin:6px 0 0}
.clips:not(.three) .clip:first-child{grid-column:span 2;grid-row:span 2}.clips:not(.three) .clip:first-child .frame{flex:1;aspect-ratio:auto;min-height:300px}
.fx-mv:hover .frame,.fx-mv:focus .frame{box-shadow:-12px -8px 0 0 rgba(235,69,44,.55),12px 8px 0 0 rgba(79,180,255,.55);transform:scale(.94);transition:transform .2s,box-shadow .2s;overflow:visible}
.fx-ba:hover .frame,.fx-ba:focus .frame{animation:roll 1s cubic-bezier(.6,0,.3,1)}@keyframes roll{to{transform:rotate(360deg)}}
.fx-ss:hover .frame,.fx-ss:focus .frame{animation:morph 1.6s ease-in-out infinite alternate}@keyframes morph{0%{border-radius:0}50%{border-radius:40% 8% 40% 8%;transform:skewX(-6deg)}100%{border-radius:50%;transform:scale(.8)}}
.fx-lp:hover .frame,.fx-lp:focus .frame{animation:flip 1.2s cubic-bezier(.6,0,.3,1)}@keyframes flip{to{transform:perspective(900px) rotateX(360deg)}}
.fx-cr:hover .frame,.fx-cr:focus .frame{animation:crumble .5s steps(2) infinite;clip-path:polygon(0 0,18% 4%,35% 0,52% 6%,70% 0,86% 5%,100% 0,100% 100%,82% 94%,64% 100%,47% 95%,30% 100%,12% 94%,0 100%)}
@keyframes crumble{0%{transform:translate(0,0) rotate(0)}50%{transform:translate(3px,-2px) rotate(-1deg)}100%{transform:translate(-2px,3px) rotate(1deg)}}
.fx-sh:hover .frame,.fx-sh:focus .frame{animation:shard .6s steps(3) infinite}
@keyframes shard{0%{clip-path:inset(0 0 66% 0);transform:translateX(-10px)}33%{clip-path:inset(33% 0 33% 0);transform:translateX(12px)}66%{clip-path:inset(66% 0 0 0);transform:translateX(-6px)}100%{clip-path:inset(0);transform:none}}
/* The mod select. */
.board{position:relative;padding-bottom:96px}.cols{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:20px 18px}
.col h3{color:var(--c);font-size:16px;letter-spacing:2px;border-bottom:2px solid var(--c);padding-bottom:4px;margin-bottom:8px}
.col ul{list-style:none;margin:0;padding:0}.chip{display:flex;gap:10px;align-items:center;min-height:32px;padding:0 8px;border:2px solid transparent;cursor:default;white-space:nowrap}
.chip>b{color:var(--c);width:24px;flex:none}.chip .name{overflow:hidden;text-overflow:ellipsis}
.chip:hover,.chip:focus{border-color:var(--c);background:color-mix(in srgb,var(--c) 14%,transparent);color:var(--white);outline:none}
.chip .desc{display:none;position:absolute;left:20px;right:20px;bottom:18px;min-height:56px;padding:8px 12px;background:var(--panel2);border:2px solid var(--c);color:var(--text);white-space:normal;z-index:2}
.chip .desc b{display:block;color:var(--c)}.chip:hover .desc,.chip:focus .desc{display:block}
.descbar{position:absolute;left:20px;right:20px;bottom:18px;min-height:56px;padding:8px 12px;border:2px dashed var(--line2);color:var(--dim2);display:flex;align-items:center}
/* Lists and tables. */
.scroll{overflow-x:auto}table.list{width:100%;border-collapse:collapse}
table.list th{text-align:left;color:var(--dim2);padding:0 10px 8px;white-space:nowrap;font-weight:400}
table.list td{padding:8px 10px;white-space:nowrap;border-top:2px solid var(--line)}table.list tr:hover td{background:var(--panel2)}
table.list .avatar{margin-right:10px;vertical-align:middle}.r{text-align:right}
tr.p1 .place{color:var(--gold)}tr.p2 .place{color:#d8d8e8}tr.p3 .place{color:#d98a4a}.great{color:var(--katl)}.ok{color:var(--green)}.miss{color:var(--acc)}
/* Covers. */
.cover{height:260px;position:relative;background:linear-gradient(0deg,var(--bg),transparent 70%),linear-gradient(rgba(11,11,16,.45),rgba(11,11,16,.45)),var(--cover,linear-gradient(135deg,hsl(var(--h,0) 40% 22%),hsl(calc(var(--h,0) + 60) 35% 12%))) center/cover}
.cover:after{content:"";position:absolute;inset:0;background:repeating-linear-gradient(0deg,rgba(0,0,0,.2) 0 2px,transparent 2px 4px)}
.cover.map{height:auto;min-height:300px;display:flex;align-items:flex-end;background:linear-gradient(0deg,var(--bg) 2%,transparent 70%),linear-gradient(90deg,rgba(11,11,16,.94) 10%,rgba(11,11,16,.7) 60%,rgba(11,11,16,.5)),var(--cover,linear-gradient(135deg,hsl(var(--h) 40% 22%),hsl(calc(var(--h) + 60) 35% 12%))) center/cover}
.cover.map .inner{position:relative;z-index:1;width:100%;padding-bottom:26px}.cover.map .artist{font-size:24px;color:var(--dim)}.cover.map h1{font-size:48px;line-height:56px;text-shadow:-3px 0 var(--don),3px 0 var(--katl)}
.cover .sub{display:flex;gap:14px;align-items:center;flex-wrap:wrap;margin-top:12px}
/* Profile. */
.head{margin-top:-100px;position:relative;z-index:1;display:flex;align-items:flex-end;gap:32px;flex-wrap:wrap;padding:22px 24px}
.who{display:flex;align-items:flex-end;gap:22px;min-width:0}.who h1{font-size:48px;line-height:56px;text-shadow:-2px 0 var(--don),2px 0 var(--katl)}.id{min-width:0}
.name-row{display:flex;align-items:center;gap:14px;flex-wrap:wrap}
.badge{display:inline-block;padding:2px 10px;background:var(--c);color:var(--white);font-size:24px;line-height:28px;letter-spacing:2px;box-shadow:3px 3px 0 color-mix(in srgb,var(--c) 45%,#000),-2px 0 0 var(--katl);cursor:default}
.status{color:var(--dim)}.status:before{content:"";display:inline-block;width:8px;height:8px;background:currentColor;margin-right:8px;vertical-align:2px}.status.on{color:var(--green)}
.ranks{margin-left:auto;display:flex;gap:36px;align-items:flex-end;flex-wrap:wrap}.bigstat span{display:block;color:var(--dim)}.bigstat b{font-size:48px;line-height:48px;color:var(--white)}.bigstat b.pp{color:var(--acc)}
.level{min-width:220px}.level span{color:var(--dim)}.level b{color:var(--white)}
.bar{height:16px;margin-top:6px;background:var(--panel2);border:2px solid var(--line2)}.bar i{display:block;height:100%;background:repeating-linear-gradient(90deg,var(--gold) 0 10px,transparent 10px 12px)}
.hudrow{display:grid;grid-template-columns:repeat(7,minmax(0,1fr));gap:2px;background:var(--line);border:2px solid var(--line);margin-bottom:20px}
.hudstat{background:var(--panel);padding:12px 14px}.hudstat span{display:block;color:var(--dim2)}.hudstat b{font-size:24px;line-height:32px;color:var(--white);white-space:nowrap}
.hudrow-in{display:flex;gap:28px;flex-wrap:wrap;margin-left:auto}.hudrow-in .hudstat{background:none;padding:0}.hudstat b.mods{font-size:16px}
.cols-side{display:grid;grid-template-columns:minmax(0,1fr) 340px;gap:20px;align-items:start}
.chart{position:relative}.chart svg{display:block;width:100%;height:160px;background:repeating-linear-gradient(0deg,transparent 0 38px,var(--line) 38px 40px)}
.chart .area{fill:rgba(235,69,44,.15)}.chart .line{fill:none;stroke:var(--acc);stroke-width:3}.chart-axis{display:flex;justify-content:space-between;margin-top:6px}
.grades{display:flex;justify-content:space-around}.grades div{display:flex;flex-direction:column;align-items:center;gap:4px}.grades .grade{font-size:48px;line-height:48px;width:auto}.grades b{color:var(--white)}
.modcounts{display:flex;flex-wrap:wrap;gap:6px 10px}.modcount{display:inline-flex;align-items:center;gap:2px}
.mini{position:relative;display:flex;align-items:center;gap:12px;min-height:56px;padding:4px 10px 4px 4px;margin-bottom:6px;background:var(--panel2);border:2px solid transparent}.mini:hover{border-color:var(--acc);color:var(--text)}
.mini .art{width:64px;height:44px;flex:none;background:var(--cover,linear-gradient(135deg,hsl(var(--h) 45% 34%),hsl(calc(var(--h) + 60) 40% 18%))) center/cover}
.mini-info{display:flex;flex-direction:column;min-width:0}.mini-info b,.mini-info span{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.mini-info b{color:var(--white)}.times{margin-left:auto;color:var(--gold)}
.play{display:flex;align-items:center;gap:16px;padding:10px 12px;margin-bottom:4px;background:var(--panel2);border-left:4px solid var(--line2)}.play:hover{border-left-color:var(--acc)}
.play .what{flex:1;min-width:0}.play .what a{display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.meta{display:flex;align-items:center;gap:10px;min-width:0;white-space:nowrap;overflow:hidden}.meta .version{overflow:hidden;text-overflow:ellipsis}
.play .mods{white-space:nowrap}.play .acc{width:80px;text-align:right;color:var(--dim)}.play .pp{width:150px;text-align:right;font-size:24px;line-height:28px}.weight{font-size:16px;color:var(--dim2);white-space:nowrap}
.topscore{display:flex;align-items:center;gap:28px;flex-wrap:wrap}.topscore .place{font-size:48px;width:auto;color:var(--gold)}.bigrade .grade{font-size:64px;line-height:64px;width:auto}
.topscore .who{align-items:center;gap:0}
/* Beatmap cards. */
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(320px,1fr));gap:14px}
.card{position:relative;display:block;min-height:128px;overflow:hidden;isolation:isolate;border:2px solid var(--line);background:var(--panel)}
.card .art{position:absolute;inset:0;z-index:-2;background:var(--cover,linear-gradient(135deg,hsl(var(--h) 45% 30%),hsl(calc(var(--h) + 60) 40% 14%))) center/cover;transition:transform .3s}
.card:before{content:"";position:absolute;inset:0;z-index:-1;background:linear-gradient(90deg,rgba(21,21,29,.97) 35%,rgba(21,21,29,.75) 70%,rgba(21,21,29,.4))}
.card:hover{border-color:var(--acc);color:var(--text)}.card:hover .art{transform:scale(1.06)}
.card .info{display:flex;flex-direction:column;gap:2px;padding:12px 16px;max-width:90%}
.card .title{font-size:24px;line-height:28px;color:var(--white);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.card .artist{color:var(--dim);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.card .meta{margin-top:6px}.card .foot{color:var(--dim2);margin-top:4px}
/* Account, login and chat. */
.account{display:flex;align-items:center;gap:8px}.account form{margin:0}
.me{display:flex;align-items:center;gap:8px;height:36px;padding:0 10px 0 6px;border:2px solid var(--line2);background:var(--panel)}.me:hover{border-color:var(--acc);color:var(--white)}
.me span{max-width:140px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.navbtn{display:inline-flex;align-items:center;height:36px;padding:0 12px;border:2px solid var(--line2);background:var(--panel);color:var(--dim);font:inherit;cursor:pointer}
.navbtn:hover,.navbtn.on{border-color:var(--acc);color:var(--white)}.navbtn.hot{background:var(--acc);border-color:var(--acc);color:var(--white)}
button.btn{font:inherit;cursor:pointer}.actions{display:flex;align-items:center;gap:10px;margin-top:10px;flex-wrap:wrap}.actions form{margin:0}.actions .btn{min-height:36px;padding:0 14px}
.btn.hot.small-hot{font-size:16px;min-height:36px;padding:0 14px;box-shadow:3px 3px 0 #6b1d12}
img.avatar{object-fit:cover;background:var(--panel2)}.tag.loved{color:#ff66aa;border-color:#ff66aa}form.inline{display:inline;margin:0}
.tools{display:flex;flex-direction:column;gap:14px}.tool{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.tool b{min-width:170px;letter-spacing:1px}.tool form{margin:0;display:flex;gap:10px;align-items:center;flex-wrap:wrap}.tool input[name=pp]{font:inherit;background:var(--panel2);color:var(--white);border:2px solid var(--line2);padding:6px 10px;width:220px}.tool input[type=file]{font:inherit;color:var(--dim);max-width:100%}.tool code{color:var(--gold)}
.notice{border:2px solid var(--gold);color:var(--gold);padding:8px 12px;margin:0 0 20px;background:rgba(255,211,77,.06)}.notice.bad{border-color:var(--acc);color:#ff8a78;background:rgba(235,69,44,.08)}
.login{max-width:520px;margin:24px auto}.login h1{font-size:48px;line-height:56px;margin-bottom:8px}.login form{display:flex;flex-direction:column;gap:14px;margin:18px 0}
.login label{display:flex;flex-direction:column;gap:6px;color:var(--dim)}
.login input,.say input{font:inherit;height:44px;background:var(--bg2);border:2px solid var(--line2);color:var(--white);padding:0 12px}.login input:focus,.say input:focus{outline:none;border-color:var(--acc)}
.login .btn{align-self:flex-start}
.chat{display:grid;grid-template-columns:300px minmax(0,1fr);gap:20px;align-items:stretch}
.chans{display:flex;flex-direction:column;gap:4px;max-height:640px;overflow-y:auto}
.chan{display:grid;grid-template-columns:auto auto 1fr;align-items:center;gap:4px 8px;padding:8px 10px;border:2px solid transparent;color:var(--text)}
.chan:hover{border-color:var(--line2);color:var(--white)}.chan.on{border-color:var(--acc);background:rgba(235,69,44,.1);color:var(--white)}
.chan .last{grid-column:1/-1;color:var(--dim2);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.hash{color:var(--acc);margin-right:4px}
.cname{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.convo{display:flex;flex-direction:column;min-height:640px;margin-bottom:20px}
.log{list-style:none;margin:0 0 14px;padding:0;flex:1;overflow-y:auto;max-height:520px;display:flex;flex-direction:column;gap:4px}
.log li{display:flex;gap:10px;align-items:baseline;padding:2px 0}.log .time{color:var(--dim2);flex:none}.log .player{flex:none}.log .text{overflow-wrap:anywhere;color:var(--text)}
.log li.mine .player{color:var(--acc)}.log .empty{justify-content:center;padding:40px 0}
.say{display:flex;gap:10px}.say input{flex:1;min-width:0}.say .btn.hot{font-size:16px;min-height:44px;box-shadow:3px 3px 0 #6b1d12}
/* Download. */
/* Install commands. */
.cmd{display:flex;align-items:stretch;max-width:100%;border:2px solid var(--line2);background:var(--bg2);margin:8px 0}
.cmd code{flex:1;min-width:0;padding:10px 14px;font:inherit;color:var(--green);word-break:break-all}.cmd .prompt{color:var(--dim2)}
.copy{flex:none;font:inherit;border:0;border-left:2px solid var(--line2);background:var(--panel2);color:var(--white);padding:0 16px;cursor:pointer}.copy:hover{background:var(--acc)}
.cta-label{color:var(--white);margin-bottom:2px}.stage-cta .cmd{max-width:820px;background:rgba(12,12,18,.92)}.alt{display:inline-block;margin-top:10px;color:var(--dim)}.alt:hover{color:var(--acc)}
/* Download. */
.builds{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px}.build{display:flex;flex-direction:column;align-items:flex-start;min-width:0}.build h2{font-size:32px}.build p{margin-bottom:18px}.build .cmd{align-self:stretch}
.build.mine{border-color:var(--acc)}.build .btn.hot{font-size:16px;min-height:44px;box-shadow:3px 3px 0 #6b1d12}.band-cta{margin-top:18px;max-width:820px}.yours{font-size:16px;color:var(--acc)}
pre{background:var(--bg2);border:2px solid var(--line);padding:12px 14px;overflow-x:auto;font:16px/24px taiko5x7,monospace;color:var(--green)}h3{font-size:16px;margin-top:16px;color:var(--dim)}
.lost{text-align:center;padding:56px 24px}.lost h1{font-size:96px;line-height:96px;color:var(--acc)}
footer{border-top:2px solid var(--line);background:var(--bg2);padding:24px 0}footer .inner{display:flex;flex-wrap:wrap;align-items:center;gap:24px;color:var(--dim)}footer a{color:var(--dim)}footer .brand{color:var(--white)}
@media(max-width:1100px){.cols{grid-template-columns:repeat(2,minmax(0,1fr))}.hudrow{grid-template-columns:repeat(3,minmax(0,1fr))}}
@media(max-width:1000px){.cols-side,.builds,.hub{grid-template-columns:minmax(0,1fr)}.clips{grid-template-columns:repeat(2,minmax(0,1fr))}.wide{display:none}.head{margin-top:-64px}.ranks{margin-left:0}.hudrow-in{margin-left:0}}
@media(max-width:700px){.inner,.page{padding:0 16px}.page{padding-top:16px}nav .inner{gap:4px}.links{overflow-x:auto;scrollbar-width:none}.links a{padding:0 8px;flex:none}nav form{display:none}.logo{font-size:0;gap:0}
.stage{height:600px}.intro{top:116px;left:16px;right:16px}.intro h1{font-size:48px;line-height:56px}.intro p{font-size:16px;line-height:24px}
.hud-tr{top:12px;right:16px}.errbar{display:none}.stage-cta{left:16px;right:16px;bottom:16px}.stage-cta .btn.hot{font-size:16px;min-height:44px}.hud-score{font-size:32px;line-height:32px}.hp{width:160px}.hud-tl{display:none}
.field{top:300px;transform-origin:20% 50%}.n{width:40px;height:40px;margin:-20px 0 0 -20px;left:48px}.n.big{width:56px;height:56px;margin:-28px 0 0 -28px}.seg:first-child{flex-basis:96px}.drum{left:14px;transform:scale(.8)}.judge{left:112px}.track{left:96px}.hit-great{left:106px}
.chat{grid-template-columns:minmax(0,1fr)}.chans{max-height:240px}.convo{min-height:480px}.me span{display:none}.me{padding:0 6px}.clips{grid-template-columns:minmax(0,1fr)}.clips:not(.three) .clip:first-child{grid-column:auto;grid-row:auto}.clips:not(.three) .clip:first-child .frame{flex:none;aspect-ratio:16/9;min-height:0}
.cols{grid-template-columns:minmax(0,1fr)}.board{padding-bottom:120px}.descbar,.chip .desc{min-height:80px}
.panel{padding:14px}.play{gap:10px}.play .mods,.play .acc,.play .score,.weight{display:none}.play .pp{width:auto}.hudrow{grid-template-columns:repeat(2,minmax(0,1fr))}
.avatar.big{width:88px;height:88px;font-size:64px}.who{gap:14px}.who h1{font-size:32px;line-height:40px;overflow-wrap:anywhere}.bigstat b{font-size:32px;line-height:32px}.ranks{gap:24px}
.cover.map h1{font-size:32px;line-height:40px}.cover{height:180px}.band h1{font-size:48px;line-height:48px}.section-head h2{font-size:24px}}
@media(prefers-reduced-motion:reduce){*,*:before,*:after{animation:none!important;transition:none!important}.frame video{display:none}.ghost{display:none}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(esc("<b>\"A&B'</b>"), "&lt;b&gt;&quot;A&amp;B&#39;&lt;/b&gt;");
        assert!(page("<x>", "", None, &Meta::default(), "").contains("&lt;x&gt; · taiko.rs"));
        assert!(page("Rankings", "rankings", None, &Meta::default(), "").contains("href=\"/rankings\" class=on"));
        assert!(page("x", "", None, &Meta::default(), "").contains("href=\"/login\""));
        let card = page("A \"quote\"", "", None, &Meta { description: "d<b>".into(), image: None }, "");
        assert!(card.contains(&format!("og:image content=\"https://rstaiko.auneu.space/site/og.jpg?v={}\"", card_version())) && card.contains("og:title content=\"A &quot;quote&quot;\"") && card.contains("d&lt;b&gt;"));
        let mine = page("x", "chat", Some("Ryuji"), &Meta::default(), "");
        assert!(mine.contains("href=\"/u/Ryuji\"") && mine.contains("action=\"/logout\"") && !mine.contains("href=\"/login\""));
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
    fn badges_go_by_name_in_any_case() {
        assert!(badges("Auneu").contains(">DEV<"));
        assert!(badges("claude").contains(">MAIN DEV<"));
        assert_eq!((play_time(59_999), play_time(3_660_000), play_time(90_061_000)), ("0m".into(), "1h 1m".into(), "1d 1h 1m".into()));
        assert!(badges("lynerrr").contains(">DEV<"));
        assert!(badges("Ryuji").is_empty());
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
    fn mod_list_and_files_are_there() {
        assert!(mod_count() >= 70, "{}", mod_count());
        assert_eq!(mod_info("MV").map(|m| m.1), Some("Multiverse"));
        assert!(mod_chips(&["HD", "??"], 1.5).contains("1.50x"));
        for c in &CLIPS {
            for ext in ["mp4", "jpg"] {
                let name = format!("clips/{}.{ext}", c.file);
                assert!(STATIC_FILES.iter().any(|(f, bytes)| *f == name && !bytes.is_empty()), "{name}");
            }
            for m in c.mods {
                assert!(mod_info(m).is_some(), "{m}");
            }
        }
    }

    #[test]
    fn pp_chart_needs_two_points() {
        assert!(pp_chart(&[(1, 10.0)]).contains("after a few plays"));
        let svg = pp_chart(&[(0, 10.0), (50, 20.0), (100, 40.0)]);
        assert!(svg.contains("<svg") && svg.contains("H600.0 V8.0"), "{svg}");
    }
}
