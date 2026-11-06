//! The website, like osu!'s: rankings, player profiles, beatmap leaderboards and the download
//! page (`pages.rs` lays them out), and the same as JSON, on a port of its own. Real DDoS
//! protection is the proxy in front (Cloudflare); on its own it caps the connections at once,
//! requests per address and how long a request may take, and keeps nothing per request.
//!
//!   /                 home              /download        builds for the visitor's system
//!   /rankings         by pp             /api/rankings
//!   /u/<name>         a profile         /api/u/<name>
//!   /beatmaps         maps played       /api/beatmaps
//!   /m/<map hash>     a map's scores    /api/m/<map hash>
//!   /s/<id>           a score, and a button that opens it in the game (?watch: by itself)
//!   /s/<id>/card.png  its card (`card.rs`), for Discord   /api/s/<id>
//!   /mods             the mod select, and clips of what the mods do
//!   /loved            the maps loved here (they give pp)
//!   /custom           maps made in the editor and sent here   /api/custom   /api/custom/status
//!   /c/<id>           a custom set; /c/<id>/download its .osz, /c/<id>/bg its background
//!   /custom/upload    POST (an .osz, the game's token as a bearer; ?id=N updates one's own set)
//!   /c/<id>/approve   POST: a loved nominator approves a custom set (on=0: takes it back)
//!   /c/<id>/delete    POST: its sender (or an admin) deletes it
//!   /a/<name>         a player's profile picture
//!   /avatar           POST (a form with a file): set your picture; /avatar/remove takes it off
//!   /shot             POST (a JPEG, the game's token as a bearer): a chat screenshot; its id back
//!   /shot/<id>        a chat screenshot
//!   /u/<name>/avatar  POST: an admin sets (or with /remove takes off) someone else's picture
//!   /love/<set>       POST: a loved nominator loves a map set here (on=0: takes it back)
//!   /u/<name>/nominator  POST: an admin makes them a loved nominator (on=0: not any more)
//!   /u/<name>/ban     POST: an admin bans them (on=0: lets them back)
//!   /u/<name>/rename  POST (name): an admin gives them a new name
//!   /s/<id>/pp        POST: an admin sets a play's pp (empty: back to what was sent)
//!   /s/<id>/delete    POST: an admin deletes a play (the "200" group: a play of their own)
//!   /reset-account    POST: the "200" group wipes their own plays and play time
//!   /u/<name>/200     POST: an admin puts them in the "200" group (on=0: takes them out)
//!   /login /logout    the website's session (the game's accounts)
//!   /register         a new account, with a code mailed to its email
//!   /reset            a new password, with a code mailed to the account's email
//!   /chat[/<name>]    the global chat and messages to friends, like the game's   /api/chat[/<name>]
//!   /friend/<name>    POST: ask to be friends, or accept
//!   /clips/<file>     the gameplay clips (with byte ranges, for video players)
//!   /site/<file>      the font and the logo
//!   /api/claude/pings?after=N   the developer's pings of the Discord bot, for Claude
//!   /api/claude/reply           POST (channel, to, text): Claude's answer, posted by the bot
//!   /api/claude/mark            POST (channel, message, tag): a forum post tagged done (else ✅)
//!   /api/claude/score           POST (map hex, pp, score, mods a,b): a play on Claude's profile
//!                               only, off every board (`Scores::add_hidden`)
//!                               (all need `Authorization: Bearer` with DIR/claude-key)
//!   /api/mail/next              the oldest letter waiting (`mail.rs`), taken out; empty when
//!                               none is (needs `Authorization: Bearer` with DIR/mail-key)

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pages::{self, Os};
use crate::{State, WebAuth};

/// Connections being answered at once; more are closed straight away.
const MAX_CONNECTIONS: usize = 128;
/// Requests per address per minute.
const REQUESTS_PER_MINUTE: usize = 120;
/// Longest request (request line, headers and a posted form).
const MAX_REQUEST: u64 = 16 * 1024;
/// Longest posted form.
const MAX_FORM: usize = 4096;
/// Longest form from Claude (an answer in Discord).
const MAX_CLAUDE_FORM: usize = 256 * 1024;
/// The session cookie (the same sessions the game logs in with), and how long it lasts.
const COOKIE: &str = "rs_session";
const COOKIE_DAYS: u64 = 180;
const RANKINGS: usize = 100;
const MAPS: usize = 60;

pub struct Web {
    busy: AtomicUsize,
    recent: Mutex<HashMap<IpAddr, Vec<Instant>>>,
    /// Behind a proxy: the visitor's address is in its header, not the connection's.
    behind_proxy: bool,
}

/// Answers on `port` until the process ends (threads of its own).
pub fn serve(state: Arc<State>, port: u16, behind_proxy: bool) {
    let web = Arc::new(Web {
        busy: AtomicUsize::new(0),
        recent: Mutex::new(HashMap::new()),
        behind_proxy,
    });
    for addr in ["::", "0.0.0.0"] {
        let Ok(listener) = TcpListener::bind((addr, port)) else {
            continue;
        };
        let (state, web) = (state.clone(), web.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if web.busy.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                    web.busy.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let (state, web2) = (state.clone(), web.clone());
                let answering = std::thread::Builder::new().spawn(move || {
                    answer(&state, &web2, stream);
                    web2.busy.fetch_sub(1, Ordering::Relaxed);
                });
                // Out of threads: this request goes unanswered, the site stays up.
                if answering.is_err() {
                    web.busy.fetch_sub(1, Ordering::Relaxed);
                }
            }
        });
    }
    println!(
        "website on port {port}{}",
        if behind_proxy {
            " (behind a proxy)"
        } else {
            ""
        }
    );
    // Forget addresses now and then.
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(120));
            web.recent.lock().unwrap().retain(|_, v| {
                v.last()
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
            });
        }
    });
}

impl Web {
    fn allow(&self, ip: IpAddr) -> bool {
        let mut recent = self.recent.lock().unwrap();
        let list = recent.entry(crate::limit_key(ip)).or_default();
        list.retain(|t| t.elapsed() < Duration::from_secs(60));
        if list.len() >= REQUESTS_PER_MINUTE {
            return false;
        }
        list.push(Instant::now());
        true
    }
}

/// Custom maps being received at once: each one is kept in memory while it comes.
static UPLOADS: AtomicUsize = AtomicUsize::new(0);
const MAX_UPLOADS: usize = 3;

struct UploadSlot;

impl UploadSlot {
    fn take() -> Option<UploadSlot> {
        let mut n = UPLOADS.load(Ordering::Acquire);
        while n < MAX_UPLOADS {
            match UPLOADS.compare_exchange(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(UploadSlot),
                Err(now) => n = now,
            }
        }
        None
    }
}

impl Drop for UploadSlot {
    fn drop(&mut self) {
        UPLOADS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A request, read and with its visitor worked out.
struct Req<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    /// A posted form's fields.
    form: HashMap<String, String>,
    /// The file posted to /avatar.
    upload: Option<Vec<u8>>,
    os: Os,
    ip: IpAddr,
    /// The session cookie, and the account it belongs to (name as written).
    token: Option<String>,
    viewer: Option<String>,
    /// It came with Claude's key (`discord::claude_key`).
    claude: bool,
    /// It came with the mailer's key (`mail::mailer_key`).
    mailer: bool,
}

impl Req<'_> {
    /// The viewer's lowercased name, the key accounts go by.
    fn key(&self) -> Option<String> {
        self.viewer.as_ref().map(|v| v.to_ascii_lowercase())
    }

    fn arg(&self, name: &str) -> Option<String> {
        self.query
            .split('&')
            .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
            .map(decode)
    }

    fn field(&self, name: &str) -> &str {
        self.form.get(name).map(String::as_str).unwrap_or("")
    }

    fn page(&self, status: u16, title: &str, nav: &str, body: String) -> Reply {
        self.page_with(status, title, nav, &pages::Meta::default(), body)
    }

    /// A page with its own link preview.
    fn page_with(
        &self,
        status: u16,
        title: &str,
        nav: &str,
        meta: &pages::Meta,
        body: String,
    ) -> Reply {
        Reply::html(status, title, nav, self.viewer.as_deref(), meta, body)
    }

    /// A step of an account form: never kept, it has the email typed in it.
    fn account_step(&self, status: u16, title: &str, body: String) -> Reply {
        Reply {
            cache: "no-store",
            ..self.page(status, title, "", body)
        }
    }

    /// Whether a failed step of an account form goes back to where the code is typed: a code was
    /// typed, or one is on its way, or it failed asking for another (the last one still works).
    fn at_code(&self, error: &str) -> bool {
        self.form.contains_key("code")
            || self.form.contains_key("again")
            || error == crate::mail::JUST_SENT
    }

    fn not_found(&self, why: &str) -> Reply {
        self.page(404, "Not found", "", pages::not_found(why))
    }
}

/// A POST that didn't come from the site's own pages: the browser says so, or its origin isn't
/// the host asked (a subdomain next to ours counts as elsewhere: SameSite doesn't stop those).
fn from_elsewhere(fetch_site: &str, origin: Option<&str>, host: &str) -> bool {
    matches!(fetch_site, "cross-site" | "same-site")
        || origin.is_some_and(|o| {
            !o.split_once("://")
                .is_some_and(|(_, h)| h.eq_ignore_ascii_case(host))
        })
}

/// Where a form on a map's page goes back to, with nothing in it but the map's hash.
/// The content type of a stored background (the two we keep).
fn kind_static(kind: &str) -> &'static str {
    if kind == "image/png" {
        "image/png"
    } else {
        "image/jpeg"
    }
}

fn map_page(back: &str) -> bool {
    back.strip_prefix("/m/")
        .is_some_and(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn answer(state: &State, web: &Web, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let peer = stream
        .peer_addr()
        .map(|a| a.ip())
        .unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let mut reader = BufReader::new((&stream).take(MAX_REQUEST));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // Headers: the proxy's word for who's asking, their browser (for the download button), the
    // session cookie, and whether a post comes from another site.
    let mut visitor = None;
    let mut agent = String::new();
    let mut range = None;
    let mut token = None;
    let mut length = 0usize;
    let mut fetch_site = String::new();
    let mut origin = None;
    let mut host = String::new();
    let mut content_type = String::new();
    let mut bearer = None;
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return,
            Ok(_) if h == "\r\n" || h == "\n" => break,
            Ok(_) => {
                if let Some((k, v)) = h.split_once(':') {
                    let v = v.trim();
                    if k.eq_ignore_ascii_case("cf-connecting-ip")
                        || (visitor.is_none() && k.eq_ignore_ascii_case("x-forwarded-for"))
                    {
                        visitor = v
                            .split(',')
                            .next()
                            .and_then(|ip| ip.trim().parse::<IpAddr>().ok());
                    } else if k.eq_ignore_ascii_case("user-agent") {
                        agent = v.to_string();
                    } else if k.eq_ignore_ascii_case("range") {
                        range = Some(v.to_string());
                    } else if k.eq_ignore_ascii_case("cookie") {
                        token = v
                            .split(';')
                            .find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='))
                            .filter(|t| !t.is_empty())
                            .map(str::to_string);
                    } else if k.eq_ignore_ascii_case("content-length") {
                        length = v.parse().unwrap_or(usize::MAX);
                    } else if k.eq_ignore_ascii_case("sec-fetch-site") {
                        fetch_site = v.to_ascii_lowercase();
                    } else if k.eq_ignore_ascii_case("origin") {
                        origin = Some(v.to_string());
                    } else if k.eq_ignore_ascii_case("host") {
                        host = v.to_string();
                    } else if k.eq_ignore_ascii_case("content-type") {
                        content_type = v.to_string();
                    } else if k.eq_ignore_ascii_case("authorization") {
                        bearer = v.strip_prefix("Bearer ").map(|t| t.trim().to_string());
                    }
                }
            }
        }
    }
    let ip = if web.behind_proxy {
        visitor.unwrap_or(peer)
    } else {
        peer
    };
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // A picture upload may be bigger than a form.
    let upload = method == "POST"
        && (path == "/avatar" || (path.starts_with("/u/") && path.ends_with("/avatar")));
    // A chat screenshot comes from the game: the file is the body, the token its bearer.
    let shot = method == "POST" && path == "/shot";
    // A custom map comes from the editor the same way, only bigger: for a logged-in sender
    // and a few at a time (each is held in memory).
    let sending = method == "POST" && path == "/custom/upload";
    let slot = (sending && bearer.as_deref().is_some_and(|t| state.user(t).is_some()))
        .then(UploadSlot::take)
        .flatten();
    let custom = slot.is_some();
    // Claude's answers too (a long one in Cyrillic is ~6 bytes a letter).
    let claude = bearer.as_deref().is_some_and(crate::discord::claude_key);
    let mailer = bearer.as_deref().is_some_and(crate::mail::mailer_key);
    let max_body = if shot {
        crate::shots::MAX_BYTES
    } else if custom {
        crate::custom::MAX_OSZ
    } else if upload {
        crate::avatars::MAX_BYTES + MAX_FORM
    } else if claude {
        MAX_CLAUDE_FORM
    } else {
        MAX_FORM
    };
    let mut body = Vec::new();
    if method == "POST" && length <= max_body {
        if upload || claude || shot || custom {
            reader.get_mut().set_limit(max_body as u64);
        }
        body.resize(length, 0);
        if reader.read_exact(&mut body).is_err() {
            return;
        }
    }
    let viewer = token
        .as_deref()
        .or(bearer.as_deref().filter(|_| shot || custom))
        .and_then(|t| state.user(t));
    let file = upload
        .then(|| crate::avatars::multipart_file(&content_type, &body, "avatar").map(<[u8]>::to_vec))
        .flatten()
        .or_else(|| (shot || custom).then(|| body.clone()));
    let req = Req {
        method,
        path,
        query,
        form: if upload || shot || custom {
            HashMap::new()
        } else {
            form(&body)
        },
        upload: file,
        os: Os::of(&agent),
        ip,
        token,
        viewer,
        claude,
        mailer,
    };
    let mut reply = if !mailer && !web.allow(ip) {
        Reply::text(429, "too many requests, wait a minute")
    } else if sending && !custom {
        Reply::text(
            429,
            "log in first, and wait if other maps are being sent just now",
        )
    } else if method == "POST"
        && (from_elsewhere(&fetch_site, origin.as_deref(), &host) || length > max_body)
    {
        Reply::text(403, "no")
    } else if !matches!(method, "GET" | "HEAD" | "POST") {
        Reply::text(405, "only GET and POST")
    } else {
        route(state, &req)
    };
    // What a logged-in visitor sees is theirs alone.
    if req.viewer.is_some() && !reply.ranges {
        reply.cache = "private, no-store";
    }
    let mut extra = reply
        .location
        .as_ref()
        .map(|l| format!("Location: {l}\r\n"))
        .unwrap_or_default();
    if let Some(c) = &reply.cookie {
        let secure = if web.behind_proxy { "; Secure" } else { "" };
        extra += &format!(
            "Set-Cookie: {COOKIE}={c}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{secure}\r\n",
            if c.is_empty() {
                0
            } else {
                COOKIE_DAYS * 86_400
            }
        );
    }
    if let Some(name) = &reply.attachment {
        extra += &format!("Content-Disposition: attachment; filename=\"{name}\"\r\n");
    }
    if reply.kind.starts_with("text/html") {
        extra += "Vary: Cookie\r\n";
    }
    if reply.ranges {
        extra += "Accept-Ranges: bytes\r\n";
        let total = reply.body.len();
        match range.as_deref().map(|r| byte_range(r, total)) {
            Some(Some((start, end))) => {
                reply.status = 206;
                extra += &format!("Content-Range: bytes {start}-{end}/{total}\r\n");
                reply.body = match reply.body {
                    Cow::Borrowed(b) => Cow::Borrowed(&b[start..=end]),
                    Cow::Owned(b) => Cow::Owned(b[start..=end].to_vec()),
                };
            }
            Some(None) => {
                reply = Reply::text(416, "range not satisfiable");
                extra = format!("Content-Range: bytes */{total}\r\n");
            }
            None => {}
        }
    }
    let mut out = &stream;
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: {CSP}\r\n{extra}Connection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        reply.kind,
        reply.body.len(),
        reply.cache,
    );
    let _ = out.write_all(head.as_bytes());
    if method != "HEAD" {
        let _ = out.write_all(&reply.body);
    }
}

/// `%41+b` as "A b": a query or form value.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A text made safe to put in a link's query.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// An `a=1&b=2` form body.
fn form(body: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect()
}

/// The first byte range of a `Range: bytes=...` header for a body of `total` bytes, as
/// inclusive (start, end); None if it can't be served.
fn byte_range(header: &str, total: usize) -> Option<(usize, usize)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: usize = n.parse().ok()?;
            (total.checked_sub(n.min(total))?, total.checked_sub(1)?)
        }
        (s, "") => (s.parse().ok()?, total.checked_sub(1)?),
        (s, e) => (
            s.parse().ok()?,
            e.parse::<usize>().ok()?.min(total.checked_sub(1)?),
        ),
    };
    (start <= end && end < total).then_some((start, end))
}

/// Styles inline; the font, clips and the chat's script from here; map covers from osu!.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'self'; font-src 'self'; img-src 'self' https://assets.ppy.sh; media-src 'self'; form-action 'self'; frame-ancestors 'none'";

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
        303 => "See Other",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

struct Reply {
    status: u16,
    kind: &'static str,
    body: Cow<'static, [u8]>,
    location: Option<String>,
    /// A new session cookie ("" clears it).
    cookie: Option<String>,
    cache: &'static str,
    /// Byte ranges may be asked for (the clips).
    ranges: bool,
    /// The file name to save it as, for a download.
    attachment: Option<String>,
}

impl Reply {
    fn new(status: u16, kind: &'static str, body: Cow<'static, [u8]>) -> Reply {
        Reply {
            status,
            kind,
            body,
            location: None,
            cookie: None,
            cache: "public, max-age=30",
            ranges: false,
            attachment: None,
        }
    }

    /// A page; `nav` is the header link it's under ("" for none), `viewer` who's logged in.
    fn html(
        status: u16,
        title: &str,
        nav: &str,
        viewer: Option<&str>,
        meta: &pages::Meta,
        body: String,
    ) -> Reply {
        Reply::new(
            status,
            "text/html; charset=utf-8",
            pages::page(title, nav, viewer, meta, &body)
                .into_bytes()
                .into(),
        )
    }

    fn json<T: serde::Serialize>(value: &T) -> Reply {
        Reply::new(
            200,
            "application/json",
            serde_json::to_vec(value).unwrap_or_default().into(),
        )
    }

    fn text(status: u16, text: &str) -> Reply {
        Reply::new(
            status,
            "text/plain; charset=utf-8",
            text.as_bytes().to_vec().into(),
        )
    }

    fn redirect(to: String) -> Reply {
        Reply {
            location: Some(to),
            ..Reply::new(302, "text/plain", Cow::Borrowed(&[]))
        }
    }

    /// Where to go after a post.
    fn see_other(to: String) -> Reply {
        Reply {
            location: Some(to),
            ..Reply::new(303, "text/plain", Cow::Borrowed(&[]))
        }
    }

    /// A clip, its poster, the font or the logo: the same for everyone and never changing
    /// within a release.
    fn file(name: &str, bytes: &'static [u8]) -> Reply {
        let kind = match name.rsplit('.').next() {
            Some("mp4") => "video/mp4",
            Some("jpg") => "image/jpeg",
            Some("png") => "image/png",
            Some("woff2") => "font/woff2",
            Some("js") => "text/javascript; charset=utf-8",
            _ => "application/octet-stream",
        };
        Reply {
            cache: "public, max-age=86400",
            ranges: true,
            ..Reply::new(200, kind, Cow::Borrowed(bytes))
        }
    }

    /// Pages that differ by the visitor's system (the download button).
    fn per_visitor(mut self) -> Reply {
        self.cache = "private, max-age=30";
        self
    }
}

/// The reply to a request.
fn route(state: &State, req: &Req) -> Reply {
    let segs: Vec<&str> = req
        .path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if req.method == "POST" {
        return post(state, req, &segs);
    }
    match segs.as_slice() {
        [] => {
            let scores = state.scores.lock().unwrap();
            let ranked = scores.rankings(usize::MAX);
            let maps = scores.maps(6);
            let plays = scores.play_count();
            let latest = scores.latest(8);
            drop(scores);
            let (online, rooms) = (state.online_players(), state.rooms());
            let live = pages::Live {
                online: &online,
                rooms: &rooms,
                latest: &latest,
            };
            req.page(
                200,
                "a taiko client for osu! beatmaps",
                "",
                pages::home(&ranked, &maps, ranked.len(), plays, req.os, live),
            )
            .per_visitor()
        }
        ["rankings"] => req.page(
            200,
            "Performance Ranking",
            "rankings",
            pages::rankings(&state.scores.lock().unwrap().rankings(RANKINGS), false),
        ),
        ["rankings", "relax"] => req.page(
            200,
            "Relax Ranking",
            "rankings",
            pages::rankings(&state.scores.lock().unwrap().relax_rankings(RANKINGS), true),
        ),
        ["medals"] => {
            let (view, table) = state.medals_page(None);
            req.page(
                200,
                "Medals",
                "medals",
                pages::medals_page(&view, None, &table),
            )
        }
        ["u", name, "medals"] => match state.profile(name) {
            Some(p) => {
                let (view, table) = state.medals_page(Some(&p.name));
                req.page(
                    200,
                    &format!("{}'s medals", p.name),
                    "medals",
                    pages::medals_page(&view, Some(&p.name), &table),
                )
            }
            None => req.not_found("There is no player by that name"),
        },
        ["api", "rankings"] => Reply::json(&state.scores.lock().unwrap().rankings(RANKINGS)),
        ["api", "rankings", "relax"] => {
            Reply::json(&state.scores.lock().unwrap().relax_rankings(RANKINGS))
        }
        ["download"] => req
            .page(200, "Download", "download", pages::download(req.os))
            .per_visitor(),
        // The search box: /u?name=...
        ["u"] => match req.arg("name") {
            Some(name) if crate::online::valid_name(&name) => Reply::redirect(format!("/u/{name}")),
            _ => Reply::redirect("/rankings".into()),
        },
        ["u", name] => match state.profile(name) {
            Some(p) => {
                let key = p.name.to_ascii_lowercase();
                let extras = state.scores.lock().unwrap().extras(&key);
                let (activity, friends) = (state.activity(&p.name), state.friends_online(&p.name));
                let tie = req
                    .key()
                    .map(|me| state.friendship(&me, &key))
                    .unwrap_or(pages::Friendship::None);
                let notice = req.arg("msg");
                let own = req.key().as_deref() == Some(key.as_str());
                let nominator = state.is_nominator(&key);
                let roles = pages::Roles {
                    admin: crate::is_admin(&key),
                    nominator,
                    own,
                    viewer_admin: req.key().is_some_and(|me| crate::is_admin(&me)),
                    banned: state.is_banned(&key),
                    two_hundred: state.is_two_hundred(&key),
                    // Their own page tells a nominator how to love maps from Discord.
                    verify_code: (own && nominator).then(|| state.verify_code(&key)),
                };
                req.page_with(
                    200,
                    &p.name,
                    "rankings",
                    &pages::profile_meta(&p),
                    pages::profile(
                        &p,
                        &extras,
                        activity.as_ref(),
                        &friends,
                        tie,
                        notice.as_deref(),
                        &roles,
                    ),
                )
            }
            None => req.not_found("There is no player by that name"),
        },
        ["api", "u", name] => match state.profile(name) {
            Some(p) => Reply::json(&p),
            None => Reply::text(404, "no such player"),
        },
        ["me"] => match &req.viewer {
            Some(me) => Reply::redirect(format!("/u/{me}")),
            None => Reply::redirect("/login".into()),
        },
        ["login"] => match &req.viewer {
            Some(me) => Reply::redirect(format!("/u/{me}")),
            None => req.page(200, "Log in", "", pages::login(None, "")),
        },
        ["register"] => match &req.viewer {
            Some(me) => Reply::redirect(format!("/u/{me}")),
            None => req.page(
                200,
                "Sign up",
                "",
                pages::register(None, "", "", crate::mail::on()),
            ),
        },
        ["reset"] => req.page(200, "Reset password", "", pages::reset(None, "")),
        ["chat"] | ["chat", _] => {
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else {
                return Reply::redirect("/login".into());
            };
            let people = state.conversations(&key);
            let error = req.arg("err");
            match segs.get(1) {
                None => req.page(
                    200,
                    "Chat",
                    "chat",
                    pages::chat(
                        me,
                        None,
                        &state.global_chat(),
                        &people,
                        true,
                        error.as_deref(),
                    ),
                ),
                Some(other) => {
                    let Some(p) = state.profile(other) else {
                        return req.not_found("There is no player by that name");
                    };
                    let friends = state.friendship(&key, &p.name.to_ascii_lowercase())
                        == pages::Friendship::Friends;
                    let lines = if friends {
                        state.chat_with(&key, &p.name)
                    } else {
                        Vec::new()
                    };
                    req.page(
                        200,
                        &format!("Chat with {}", p.name),
                        "chat",
                        pages::chat(
                            me,
                            Some(&p.name),
                            &lines,
                            &people,
                            friends,
                            error.as_deref(),
                        ),
                    )
                }
            }
        }
        ["api", "mail", "next"] if req.mailer => {
            let mut r = Reply::text(200, &crate::mail::next().unwrap_or_default());
            r.cache = "no-store";
            r
        }
        ["api", "claude", "pings"] if req.claude => {
            let after = req.arg("after").and_then(|a| a.parse().ok()).unwrap_or(0);
            Reply {
                cache: "no-store",
                ..Reply::json(&crate::discord::pings(after))
            }
        }
        ["api", "chat"] | ["api", "chat", _] => {
            let Some(key) = req.key() else {
                return Reply::text(401, "log in first");
            };
            match segs.get(2) {
                None => Reply::json(&state.global_chat()),
                Some(other) => Reply::json(&state.chat_with(&key, other)),
            }
        }
        ["beatmaps"] => req.page(
            200,
            "Beatmaps",
            "beatmaps",
            pages::beatmaps(&state.scores.lock().unwrap().maps(MAPS)),
        ),
        ["api", "beatmaps"] => Reply::json(&state.scores.lock().unwrap().maps(MAPS)),
        ["m", hash] | ["api", "m", hash] | ["m", hash, "relax"] | ["api", "m", hash, "relax"] => {
            let Ok(map) = u64::from_str_radix(hash, 16) else {
                return req.not_found("That is not a map link");
            };
            let scores = state.scores.lock().unwrap();
            let relax = segs.last() == Some(&"relax");
            let rows = if relax {
                scores.relax_map_scores(map)
            } else {
                scores.map_scores(map)
            };
            if segs[0] == "api" {
                return Reply::json(&rows);
            }
            match scores.map(map) {
                Some(m) => {
                    drop(scores);
                    let love = pages::Love {
                        loved: m.set_id.and_then(crate::loved::get),
                        can_love: req.key().is_some_and(|me| state.is_nominator(&me)),
                        notice: req.arg("msg"),
                        custom: crate::custom::by_hash(map),
                    };
                    req.page_with(
                        200,
                        &m.name,
                        "beatmaps",
                        &pages::map_meta(&m, &rows),
                        pages::map_page(&m, &rows, &love, relax),
                    )
                }
                None => req.not_found("Nobody has sent a play on that map yet"),
            }
        }
        ["s", id] | ["s", id, "card.png"] | ["api", "s", id] => {
            let Ok(id) = id.parse::<u64>() else {
                return req.not_found("That is not a score link");
            };
            let (score, place) = {
                let scores = state.scores.lock().unwrap();
                let Some(s) = scores.score(id) else {
                    return req.not_found("That score isn't on the server");
                };
                let place = scores.place_of(s.map_hash, id);
                (s, place)
            };
            match segs.as_slice() {
                ["api", ..] => Reply::json(&score),
                [_, _, _] => Reply {
                    cache: "public, max-age=86400",
                    ..Reply::new(200, "image/png", crate::card::png(&score).into())
                },
                _ => {
                    let admin = req
                        .key()
                        .filter(|me| crate::is_admin(me))
                        .and_then(|_| state.scores.lock().unwrap().pp_of(score.id));
                    // The "200" group may delete their own plays.
                    let own_delete = admin.is_none()
                        && req.key().is_some_and(|me| {
                            me == score.player.to_ascii_lowercase() && state.is_two_hundred(&me)
                        });
                    let notice = req.arg("msg");
                    req.page_with(
                        200,
                        &score.map,
                        "beatmaps",
                        &pages::score_meta(&score),
                        pages::score_page(
                            &score,
                            place,
                            req.arg("watch").is_some(),
                            admin,
                            own_delete,
                            notice.as_deref(),
                        ),
                    )
                }
            }
        }
        ["mods"] => req.page(200, "Mods", "mods", pages::mods_page()),
        ["loved"] => {
            let mut sets: Vec<pages::LovedRow> = crate::loved::all()
                .into_iter()
                .map(|(set, loved)| pages::LovedRow {
                    set,
                    names: crate::covers::names(set),
                    loved,
                })
                .collect();
            // The latest loved first.
            sets.sort_by_key(|r| std::cmp::Reverse(r.loved.at));
            req.page(200, "Loved here", "loved", pages::loved(&sets))
        }
        ["custom"] => {
            let query = req.arg("q").unwrap_or_default();
            req.page(
                200,
                "Custom maps",
                "custom",
                pages::custom_list(&crate::custom::list(&query), &query),
            )
        }
        ["api", "custom"] => {
            // ?ids=1,2,3 picks sets by number (the game asks about the ones it has).
            let ids: Option<Vec<u32>> = req.arg("ids").map(|v| {
                v.split(',')
                    .take(500)
                    .filter_map(|i| i.trim().parse().ok())
                    .collect()
            });
            let sets: Vec<_> = crate::custom::list(&req.arg("q").unwrap_or_default())
                .iter()
                .filter(|s| ids.as_ref().is_none_or(|ids| ids.contains(&s.id)))
                .map(crate::custom::Set::public)
                .collect();
            Reply::json(&sets)
        }
        ["api", "custom", "status"] => {
            // ?hashes=<hex>,<hex>: where each of these maps stands, those that are custom only.
            let hashes: Vec<u64> = req
                .arg("hashes")
                .unwrap_or_default()
                .split(',')
                .take(500)
                .filter_map(|h| u64::from_str_radix(h.trim(), 16).ok())
                .collect();
            let found: HashMap<String, crate::custom::Status> = crate::custom::statuses(&hashes)
                .into_iter()
                .map(|(h, s)| (format!("{h:x}"), s))
                .collect();
            Reply::json(&found)
        }
        ["c", id] | ["api", "c", id] => {
            let Some(set) = id.parse().ok().and_then(crate::custom::get) else {
                return req.not_found("There is no custom set by that number");
            };
            if segs[0] == "api" {
                return Reply::json(&set.public());
            }
            let me = req.key();
            let view = pages::CustomView {
                can_approve: me.as_ref().is_some_and(|k| state.is_nominator(k)),
                can_delete: me
                    .as_ref()
                    .is_some_and(|k| *k == set.owner || crate::is_admin(k)),
                notice: req.arg("msg"),
            };
            let title = format!("{} - {}", set.artist, set.title);
            req.page(200, &title, "custom", pages::custom_page(&set, &view))
        }
        ["c", id, "download"] => match id.parse().ok().and_then(crate::custom::get) {
            Some(set) => match crate::custom::file(set.id) {
                Some(bytes) => {
                    crate::custom::count_download(set.id);
                    let bytes = crate::custom::with_sidecar(&bytes, set.id).unwrap_or(bytes);
                    let name: String = format!("c{} {} - {}.osz", set.id, set.artist, set.title)
                        .chars()
                        .map(|c| {
                            if c.is_alphanumeric() || " -_.()".contains(c) {
                                c
                            } else {
                                '_'
                            }
                        })
                        .collect();
                    Reply {
                        attachment: Some(name),
                        cache: "no-store",
                        ..Reply::new(200, "application/octet-stream", Cow::Owned(bytes))
                    }
                }
                None => Reply::text(404, "the file is gone"),
            },
            None => Reply::text(404, "no such custom set"),
        },
        ["c", id, "bg"] => match id.parse().ok().and_then(crate::custom::background) {
            Some((kind, bytes)) => Reply {
                cache: "public, max-age=3600",
                ..Reply::new(200, kind_static(&kind), Cow::Owned(bytes))
            },
            None => Reply::text(404, "no background"),
        },
        ["shot", id] => match crate::shots::load(id) {
            Some(bytes) => Reply::new(200, "image/jpeg", Cow::Owned(bytes)),
            None => Reply::text(404, "no such screenshot"),
        },
        ["a", name] => match crate::avatars::load(&name.to_ascii_lowercase()) {
            Some((kind, bytes)) => Reply {
                cache: "public, max-age=86400",
                ..Reply::new(200, kind, bytes.into())
            },
            None => Reply::text(404, "no picture"),
        },
        [dir @ ("clips" | "site"), name] => match pages::STATIC_FILES
            .iter()
            .find(|(f, _)| f.strip_prefix(dir).and_then(|f| f.strip_prefix('/')) == Some(name))
        {
            Some((f, bytes)) => Reply::file(f, bytes),
            None => req.not_found("There is no such file"),
        },
        ["robots.txt"] => Reply::text(200, "User-agent: *\nAllow: /\nDisallow: /chat\n"),
        _ => req.not_found("There is nothing at this address"),
    }
}

/// The reply to a posted form: logging in and out, friend requests, chat lines.
fn post(state: &State, req: &Req, segs: &[&str]) -> Reply {
    match segs {
        ["login"] => {
            let name = req.field("name").trim();
            match state.web_login(req.ip, name, req.field("password")) {
                Ok((name, token)) => Reply {
                    cookie: Some(token),
                    ..Reply::see_other(format!("/u/{name}"))
                },
                Err(e) => req.account_step(401, "Log in", pages::login(Some(&e), name)),
            }
        }
        // Forms come in two steps: without `code` a code is mailed, then with it (and the
        // password) the account is made.
        ["register"] => {
            let (name, email) = (req.field("name").trim(), req.field("email").trim());
            let (code, password) = (
                req.form.get("code").map(String::as_str),
                req.form.get("password").map(String::as_str),
            );
            match state.web_register(req.ip, name, password, email, code) {
                WebAuth::LoggedIn { name, token } => Reply {
                    cookie: Some(token),
                    ..Reply::see_other(format!("/u/{name}"))
                },
                WebAuth::CodeSent(to) => req.account_step(
                    200,
                    "Sign up",
                    pages::register_code(None, Some(&to), name, email),
                ),
                WebAuth::Failed(e) if req.at_code(&e) => req.account_step(
                    400,
                    "Sign up",
                    pages::register_code(Some(&e), None, name, email),
                ),
                WebAuth::Failed(e) => req.account_step(
                    400,
                    "Sign up",
                    pages::register(Some(&e), name, email, crate::mail::on()),
                ),
            }
        }
        ["reset"] => {
            let who = req.field("who").trim();
            let (code, password) = (
                req.form.get("code").map(String::as_str),
                req.form.get("password").map(String::as_str),
            );
            match state.web_reset(req.ip, who, code, password) {
                WebAuth::LoggedIn { name, token } => Reply {
                    cookie: Some(token),
                    ..Reply::see_other(format!("/u/{name}"))
                },
                WebAuth::CodeSent(to) => req.account_step(
                    200,
                    "Reset password",
                    pages::reset_code(None, Some(&to), who),
                ),
                WebAuth::Failed(e) if req.at_code(&e) => req.account_step(
                    400,
                    "Reset password",
                    pages::reset_code(Some(&e), None, who),
                ),
                WebAuth::Failed(e) => {
                    req.account_step(400, "Reset password", pages::reset(Some(&e), who))
                }
            }
        }
        ["logout"] => {
            if let Some(t) = &req.token {
                state.logout(t);
            }
            Reply {
                cookie: Some(String::new()),
                ..Reply::see_other("/".into())
            }
        }
        ["shot"] => {
            let Some(name) = &req.viewer else {
                return Reply::text(403, "log in to send screenshots");
            };
            let key = name.to_ascii_lowercase();
            if state.is_banned(&key) {
                return Reply::text(403, "you can't send screenshots");
            }
            let file = req.upload.as_deref().unwrap_or_default();
            match crate::shots::save(&key, file, crate::now_secs()) {
                Ok(id) => Reply::text(200, &id),
                Err(e) => Reply::text(400, &e),
            }
        }
        ["avatar"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let msg = match &req.upload {
                Some(file) => crate::avatars::save(&key, file, crate::now_secs())
                    .map(|_| {
                        state.medal_count(&key, "avatar", 1, true);
                        "Your picture is set".to_string()
                    })
                    .unwrap_or_else(|e| e),
                None => "pick a picture first".into(),
            };
            Reply::see_other(format!(
                "/u/{}?msg={}",
                state.display_name(&key),
                encode(&msg)
            ))
        }
        ["u", name, "avatar"] | ["u", name, "avatar", "remove"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Some(p) = state.profile(name) else {
                return req.not_found("There is no player by that name");
            };
            let target = p.name.to_ascii_lowercase();
            let msg = if !crate::is_admin(&key) {
                "only admins can do that".to_string()
            } else if segs.len() == 4 {
                crate::avatars::remove(&target);
                format!("{}'s picture is off", p.name)
            } else {
                match &req.upload {
                    Some(file) => crate::avatars::save(&target, file, crate::now_secs())
                        .map(|_| format!("{}'s picture is set", p.name))
                        .unwrap_or_else(|e| e),
                    None => "pick a picture first".into(),
                }
            };
            println!("{key}: {msg}");
            Reply::see_other(format!("/u/{}?msg={}", p.name, encode(&msg)))
        }
        ["avatar", "remove"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            crate::avatars::remove(&key);
            Reply::see_other(format!(
                "/u/{}?msg={}",
                state.display_name(&key),
                encode("Your picture is off")
            ))
        }
        ["s", id, "pp"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Ok(id) = id.parse::<u64>() else {
                return req.not_found("That is not a score link");
            };
            let value = req.field("pp").trim();
            let msg = if !crate::is_admin(&key) {
                "only admins can do that".to_string()
            } else {
                let pp = if value.is_empty() {
                    Ok(None)
                } else {
                    value
                        .parse::<f64>()
                        .map(Some)
                        .map_err(|_| "pp is a number".to_string())
                };
                match pp.and_then(|pp| state.scores.lock().unwrap().set_pp(id, pp).map(|_| pp)) {
                    Ok(pp) => {
                        println!("score {id}: pp set to {pp:?} by {key}");
                        match pp {
                            Some(v) => format!("This play is worth {v:.2}pp now"),
                            None => "This play is worth what it was sent with again".into(),
                        }
                    }
                    Err(e) => e,
                }
            };
            Reply::see_other(format!("/s/{id}?msg={}", encode(&msg)))
        }
        ["s", id, "delete"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Ok(id) = id.parse::<u64>() else {
                return req.not_found("That is not a score link");
            };
            match state.delete_play(&key, id) {
                Ok(player) => Reply::see_other(format!(
                    "/u/{player}?msg={}",
                    encode(&format!("Score {id} is deleted"))
                )),
                Err(e) => Reply::see_other(format!("/s/{id}?msg={}", encode(&e))),
            }
        }
        // Only ever the account logged in: no name in the address can point it elsewhere.
        ["reset-account"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let msg = match state.reset_own(&key) {
                Ok(n) => format!(
                    "Your account is reset: {n} play{} deleted",
                    if n == 1 { "" } else { "s" }
                ),
                Err(e) => e,
            };
            Reply::see_other(format!(
                "/u/{}?msg={}",
                state.display_name(&key),
                encode(&msg)
            ))
        }
        ["u", name, "200"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let msg = state
                .set_two_hundred(&key, name, req.field("on") != "0")
                .unwrap_or_else(|e| e);
            Reply::see_other(format!("/u/{}?msg={}", encode(name), encode(&msg)))
        }
        ["u", name, "ban"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Some(p) = state.profile(name) else {
                return req.not_found("There is no player by that name");
            };
            let msg = state
                .set_banned(&key, &p.name, req.field("on") != "0")
                .unwrap_or_else(|e| e);
            Reply::see_other(format!("/u/{}?msg={}", p.name, encode(&msg)))
        }
        ["u", name, "rename"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Some(p) = state.profile(name) else {
                return req.not_found("There is no player by that name");
            };
            match state.rename(&key, &p.name, req.field("name")) {
                Ok(new) => Reply::see_other(format!(
                    "/u/{new}?msg={}",
                    encode(&format!("{} is {new} now", p.name))
                )),
                Err(e) => Reply::see_other(format!("/u/{}?msg={}", p.name, encode(&e))),
            }
        }
        ["custom", "upload"] => {
            let Some(name) = &req.viewer else {
                return Reply::text(403, "log in to send maps");
            };
            let key = name.to_ascii_lowercase();
            if state.is_banned(&key) {
                return Reply::text(403, "you can't send maps");
            }
            let id = match req.arg("id") {
                Some(id) => match id.parse::<u32>() {
                    Ok(id) => Some(id),
                    Err(_) => return Reply::text(400, "id is a set's number"),
                },
                None => None,
            };
            let file = req.upload.as_deref().unwrap_or_default();
            match crate::custom::upload(
                &key,
                &state.display_name(&key),
                id,
                file,
                crate::now_secs(),
            ) {
                Ok(set) => {
                    println!(
                        "custom set {} v{} by {name}: {} - {}",
                        set.id, set.version, set.artist, set.title
                    );
                    Reply::json(&serde_json::json!({
                        "id": set.id,
                        "version": set.version,
                        "url": format!("/c/{}", set.id),
                    }))
                }
                Err(e) => Reply::text(400, &e),
            }
        }
        ["c", id, "approve"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Ok(id) = id.parse::<u32>() else {
                return req.not_found("There is no custom set by that number");
            };
            let on = req.field("on") != "0";
            let msg = if !state.is_nominator(&key) {
                "only loved nominators can approve maps".to_string()
            } else {
                let name = state.display_name(&key);
                match crate::custom::approve(id, on, &name, crate::now_secs()) {
                    Ok(true) => {
                        state.scores.lock().unwrap().count_pp();
                        println!(
                            "custom set {id} {} by {name}",
                            if on { "approved" } else { "unapproved" }
                        );
                        if on {
                            "Approved: plays on it give pp now".into()
                        } else {
                            "Approval taken back".into()
                        }
                    }
                    Ok(false) => "Nothing changed".into(),
                    Err(e) => e,
                }
            };
            Reply::see_other(format!("/c/{id}?msg={}", encode(&msg)))
        }
        ["c", id, "delete"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Ok(id) = id.parse::<u32>() else {
                return req.not_found("There is no custom set by that number");
            };
            match crate::custom::delete(id, &key, crate::is_admin(&key)) {
                Ok(()) => {
                    state.scores.lock().unwrap().count_pp();
                    println!("custom set {id} deleted by {key}");
                    Reply::see_other(format!("/custom?q={}", encode("")))
                }
                Err(e) => Reply::see_other(format!("/c/{id}?msg={}", encode(&e))),
            }
        }
        ["love", set] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let Ok(set) = set.parse::<u32>() else {
                return req.not_found("That is not a map set");
            };
            let back = req.field("back");
            let back = if map_page(back) {
                back.to_string()
            } else {
                "/loved".into()
            };
            let msg = state
                .love(&key, set, req.field("on") != "0")
                .unwrap_or_else(|e| e);
            Reply::see_other(format!("{back}?msg={}", encode(&msg)))
        }
        ["u", name, "nominator"] => {
            let Some(key) = req.key() else {
                return Reply::see_other("/login".into());
            };
            let msg = state
                .set_nominator(&key, name, req.field("on") != "0")
                .unwrap_or_else(|e| e);
            Reply::see_other(format!("/u/{}?msg={}", encode(name), encode(&msg)))
        }
        ["api", "claude", "reply"] if req.claude => {
            let to = Some(req.field("to")).filter(|t| !t.is_empty());
            match crate::discord::say(req.field("channel"), to, req.field("text")) {
                Ok(()) => Reply::text(200, "sent"),
                Err(e) => Reply::text(502, &e),
            }
        }
        ["api", "claude", "score"] if req.claude => {
            let map = u64::from_str_radix(req.field("map"), 16)
                .map_err(|_| "map is its hash, in hex".to_string());
            let pp = req
                .field("pp")
                .parse::<f64>()
                .map_err(|_| "pp is a number".to_string());
            let score = req.field("score").parse::<u64>().unwrap_or(1_000_000);
            let mods = req
                .field("mods")
                .split(',')
                .map(|m| crate::online::clean_text(m.trim(), 4))
                .filter(|m| !m.is_empty())
                .take(40)
                .collect();
            let name = state.display_name("claude");
            match map.and_then(|map| {
                state.scores.lock().unwrap().add_hidden(
                    "claude",
                    &name,
                    map,
                    pp?,
                    score,
                    mods,
                    crate::now_secs(),
                )
            }) {
                Ok(id) => Reply::text(200, &format!("score {id}")),
                Err(e) => Reply::text(400, &e),
            }
        }
        ["api", "claude", "mark"] if req.claude => {
            match crate::discord::mark(req.field("channel"), req.field("message"), req.field("tag"))
            {
                Ok(done) => Reply::text(200, &done),
                Err(e) => Reply::text(502, &e),
            }
        }
        ["friend", other] => {
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else {
                return Reply::see_other("/login".into());
            };
            let Some(p) = state.profile(other) else {
                return req.not_found("There is no player by that name");
            };
            let msg = state.add_friend(&key, me, &p.name).unwrap_or_else(|e| e);
            Reply::see_other(format!("/u/{}?msg={}", p.name, encode(&msg)))
        }
        ["chat"] | ["chat", _] => {
            let js = req.query.split('&').any(|kv| kv == "js=1");
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else {
                return if js {
                    Reply::json(&serde_json::json!({ "error": "log in first" }))
                } else {
                    Reply::see_other("/login".into())
                };
            };
            let text = req.field("text");
            let (sent, back) = match segs.get(1) {
                None => (
                    state.post_global(me, text, |hub| hub.web_chat_allowed(&key)),
                    "/chat".to_string(),
                ),
                Some(other) => (
                    state.post_chat(&key, me, other, text, |hub| hub.web_chat_allowed(&key)),
                    format!("/chat/{}", encode(other)),
                ),
            };
            match (sent, js) {
                (Ok(_), true) => Reply::json(&serde_json::json!({ "ok": true })),
                (Err(e), true) => Reply::json(&serde_json::json!({ "error": e })),
                (Ok(_), false) => Reply::see_other(back),
                (Err(e), false) => Reply::see_other(format!("{back}?err={}", encode(&e))),
            }
        }
        _ => req.not_found("There is nothing to post to here"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Req, byte_range, decode, encode, form, from_elsewhere, map_page, route};
    use std::collections::HashMap;
    use std::io::Write;

    fn osz() -> Vec<u8> {
        let mut map = String::from(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: 1\r\n\r\n[Metadata]\r\nTitle:Route Song\r\nArtist:Route Band\r\nCreator:me\r\nVersion:Oni\r\n\r\n[HitObjects]\r\n",
        );
        for k in 0..20 {
            map += &format!("256,192,{},1,0,0:0:0:0:\r\n", 1000 + k * 250);
        }
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("a [Oni].osu", stored).unwrap();
        zip.write_all(map.as_bytes()).unwrap();
        zip.start_file("audio.mp3", stored).unwrap();
        zip.write_all(&[0; 32]).unwrap();
        zip.finish().unwrap().into_inner()
    }

    fn request<'a>(
        method: &'a str,
        path: &'a str,
        query: &'a str,
        viewer: Option<&str>,
        upload: Option<Vec<u8>>,
        form: &[(&str, &str)],
    ) -> Req<'a> {
        Req {
            method,
            path,
            query,
            form: form
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
            upload,
            os: crate::pages::Os::of(""),
            ip: [127, 0, 0, 1].into(),
            token: None,
            viewer: viewer.map(str::to_string),
            claude: false,
            mailer: false,
        }
    }

    #[test]
    fn a_custom_map_goes_up_shows_on_the_site_downloads_and_is_approved_by_a_nominator() {
        let dir = std::env::temp_dir().join(format!("taiko-web-custom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::custom::init(Some(dir.clone()));
        let (state, _wake) = crate::State::load(dir.join("accounts.json"));
        let body = |r: &super::Reply| String::from_utf8_lossy(&r.body).into_owned();
        // Nobody logged in can't send.
        let r = route(
            &state,
            &request("POST", "/custom/upload", "", None, Some(osz()), &[]),
        );
        assert_eq!(r.status, 403);
        // A sender can; the answer names the set.
        let r = route(
            &state,
            &request("POST", "/custom/upload", "", Some("Amy"), Some(osz()), &[]),
        );
        assert_eq!(r.status, 200, "{}", body(&r));
        let id = serde_json::from_str::<serde_json::Value>(&body(&r)).unwrap()["id"]
            .as_u64()
            .unwrap();
        // The list, the page and the API tell about it.
        let list = route(&state, &request("GET", "/custom", "", None, None, &[]));
        assert!(body(&list).contains("Route Song") && body(&list).contains(">CUSTOM<"));
        let page = route(
            &state,
            &request("GET", &format!("/c/{id}"), "", None, None, &[]),
        );
        assert!(body(&page).contains("Route Band") && !body(&page).contains("/approve"));
        let api = route(&state, &request("GET", "/api/custom", "", None, None, &[]));
        assert!(body(&api).contains("\"approved\":false"));
        // The download is an archive with the set's number in it.
        let path = format!("/c/{id}/download");
        let file = route(&state, &request("GET", &path, "", None, None, &[]));
        assert_eq!(file.status, 200);
        assert!(
            file.attachment
                .as_deref()
                .is_some_and(|n| n.ends_with(".osz"))
        );
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file.body.to_vec())).unwrap();
        assert!(zip.by_name("taiko-rs.custom").is_ok());
        // Not a nominator: nothing changes. An admin approves it, and it says so.
        let path = format!("/c/{id}/approve");
        route(&state, &request("POST", &path, "", Some("amy"), None, &[]));
        assert!(!crate::custom::get(id as u32).unwrap().is_approved());
        route(
            &state,
            &request("POST", &path, "", Some("auneu"), None, &[]),
        );
        assert!(crate::custom::get(id as u32).unwrap().is_approved());
        let page = route(
            &state,
            &request("GET", &format!("/c/{id}"), "", None, None, &[]),
        );
        assert!(body(&page).contains("APPROVED CUSTOM"));
        // Only the sender or an admin deletes.
        let path = format!("/c/{id}/delete");
        route(&state, &request("POST", &path, "", Some("bob"), None, &[]));
        assert!(crate::custom::get(id as u32).is_some());
        route(&state, &request("POST", &path, "", Some("amy"), None, &[]));
        assert!(crate::custom::get(id as u32).is_none());
        let gone = route(
            &state,
            &request("GET", &format!("/c/{id}"), "", None, None, &[]),
        );
        assert_eq!(gone.status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn posts_only_come_from_the_site_itself() {
        let site = "rstaiko.auneu.space";
        assert!(!from_elsewhere(
            "same-origin",
            Some("https://rstaiko.auneu.space"),
            site
        ));
        assert!(!from_elsewhere("", None, site));
        assert!(!from_elsewhere(
            "",
            Some("http://localhost:8080"),
            "localhost:8080"
        ));
        assert!(from_elsewhere("cross-site", None, site));
        assert!(from_elsewhere("same-site", None, site));
        assert!(from_elsewhere("", Some("https://other.auneu.space"), site));
        assert!(from_elsewhere("", Some("null"), site));
    }

    #[test]
    fn going_back_is_only_to_a_map() {
        assert!(map_page("/m/1f3a"));
        assert!(!map_page("/m/"));
        assert!(!map_page("/m/1\r\nSet-Cookie: x=1"));
        assert!(!map_page("/m/1/../../x"));
    }

    #[test]
    fn forms_and_queries_decode() {
        assert_eq!(decode("a+b%21%D0%BF%"), "a b!п%");
        assert_eq!(decode(&encode("hi there & you?")), "hi there & you?");
        let f = form(b"name=Ryu%20ji&password=a%26b&x");
        assert_eq!(f.get("name").map(String::as_str), Some("Ryu ji"));
        assert_eq!(f.get("password").map(String::as_str), Some("a&b"));
    }

    #[test]
    fn byte_ranges() {
        assert_eq!(byte_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(byte_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=500-5000", 1000), Some((500, 999)));
        assert_eq!(byte_range("bytes=1000-", 1000), None);
        assert_eq!(byte_range("bytes=9-3", 1000), None);
        assert_eq!(byte_range("items=0-1", 1000), None);
    }
}
