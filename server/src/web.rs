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
//!   /mods             the mod select, and clips of what the mods do
//!   /login /logout    the website's session (the game's accounts)
//!   /chat[/<name>]    the global chat and messages to friends, like the game's   /api/chat[/<name>]
//!   /friend/<name>    POST: ask to be friends, or accept
//!   /clips/<file>     the gameplay clips (with byte ranges, for video players)
//!   /site/<file>      the font and the logo

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pages::{self, Os};
use crate::State;

/// Connections being answered at once; more are closed straight away.
const MAX_CONNECTIONS: usize = 128;
/// Requests per address per minute.
const REQUESTS_PER_MINUTE: usize = 120;
/// Longest request (request line, headers and a posted form).
const MAX_REQUEST: u64 = 16 * 1024;
/// Longest posted form.
const MAX_FORM: usize = 4096;
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
    let web = Arc::new(Web { busy: AtomicUsize::new(0), recent: Mutex::new(HashMap::new()), behind_proxy });
    for addr in ["::", "0.0.0.0"] {
        let Ok(listener) = TcpListener::bind((addr, port)) else { continue };
        let (state, web) = (state.clone(), web.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if web.busy.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                    web.busy.fetch_sub(1, Ordering::Relaxed);
                    continue;
                }
                let (state, web) = (state.clone(), web.clone());
                std::thread::spawn(move || {
                    answer(&state, &web, stream);
                    web.busy.fetch_sub(1, Ordering::Relaxed);
                });
            }
        });
    }
    println!("website on port {port}{}", if behind_proxy { " (behind a proxy)" } else { "" });
    // Forget addresses now and then.
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(120));
        web.recent.lock().unwrap().retain(|_, v| v.last().is_some_and(|t| t.elapsed() < Duration::from_secs(60)));
    });
}

impl Web {
    fn allow(&self, ip: IpAddr) -> bool {
        let mut recent = self.recent.lock().unwrap();
        let list = recent.entry(ip).or_default();
        list.retain(|t| t.elapsed() < Duration::from_secs(60));
        if list.len() >= REQUESTS_PER_MINUTE {
            return false;
        }
        list.push(Instant::now());
        true
    }
}

/// A request, read and with its visitor worked out.
struct Req<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    /// A posted form's fields.
    form: HashMap<String, String>,
    os: Os,
    ip: IpAddr,
    /// The session cookie, and the account it belongs to (name as written).
    token: Option<String>,
    viewer: Option<String>,
}

impl Req<'_> {
    /// The viewer's lowercased name, the key accounts go by.
    fn key(&self) -> Option<String> {
        self.viewer.as_ref().map(|v| v.to_ascii_lowercase())
    }

    fn arg(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=')).map(decode)
    }

    fn field(&self, name: &str) -> &str {
        self.form.get(name).map(String::as_str).unwrap_or("")
    }

    fn page(&self, status: u16, title: &str, nav: &str, body: String) -> Reply {
        self.page_with(status, title, nav, &pages::Meta::default(), body)
    }

    /// A page with its own link preview.
    fn page_with(&self, status: u16, title: &str, nav: &str, meta: &pages::Meta, body: String) -> Reply {
        Reply::html(status, title, nav, self.viewer.as_deref(), meta, body)
    }

    fn not_found(&self, why: &str) -> Reply {
        self.page(404, "Not found", "", pages::not_found(why))
    }
}

fn answer(state: &State, web: &Web, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let peer = stream.peer_addr().map(|a| a.ip()).unwrap_or(IpAddr::from([0, 0, 0, 0]));
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
    let mut cross_site = false;
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return,
            Ok(_) if h == "\r\n" || h == "\n" => break,
            Ok(_) => {
                if let Some((k, v)) = h.split_once(':') {
                    let v = v.trim();
                    if k.eq_ignore_ascii_case("cf-connecting-ip") || (visitor.is_none() && k.eq_ignore_ascii_case("x-forwarded-for")) {
                        visitor = v.split(',').next().and_then(|ip| ip.trim().parse::<IpAddr>().ok());
                    } else if k.eq_ignore_ascii_case("user-agent") {
                        agent = v.to_string();
                    } else if k.eq_ignore_ascii_case("range") {
                        range = Some(v.to_string());
                    } else if k.eq_ignore_ascii_case("cookie") {
                        token = v.split(';').find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('=')).filter(|t| !t.is_empty()).map(str::to_string);
                    } else if k.eq_ignore_ascii_case("content-length") {
                        length = v.parse().unwrap_or(usize::MAX);
                    } else if k.eq_ignore_ascii_case("sec-fetch-site") {
                        cross_site = v.eq_ignore_ascii_case("cross-site");
                    }
                }
            }
        }
    }
    let ip = if web.behind_proxy { visitor.unwrap_or(peer) } else { peer };
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut body = Vec::new();
    if method == "POST" && length <= MAX_FORM {
        body.resize(length, 0);
        if reader.read_exact(&mut body).is_err() {
            return;
        }
    }
    let viewer = token.as_deref().and_then(|t| state.user(t));
    let req = Req { method, path, query, form: form(&body), os: Os::of(&agent), ip, token, viewer };
    let mut reply = if !web.allow(ip) {
        Reply::text(429, "too many requests, wait a minute")
    } else if method == "POST" && (cross_site || length > MAX_FORM) {
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
    let mut extra = reply.location.as_ref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
    if let Some(c) = &reply.cookie {
        let secure = if web.behind_proxy { "; Secure" } else { "" };
        extra += &format!("Set-Cookie: {COOKIE}={c}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax{secure}\r\n", if c.is_empty() { 0 } else { COOKIE_DAYS * 86_400 });
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
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
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
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// An `a=1&b=2` form body.
fn form(body: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(body).split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect()
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
        (s, e) => (s.parse().ok()?, e.parse::<usize>().ok()?.min(total.checked_sub(1)?)),
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
}

impl Reply {
    fn new(status: u16, kind: &'static str, body: Cow<'static, [u8]>) -> Reply {
        Reply { status, kind, body, location: None, cookie: None, cache: "public, max-age=30", ranges: false }
    }

    /// A page; `nav` is the header link it's under ("" for none), `viewer` who's logged in.
    fn html(status: u16, title: &str, nav: &str, viewer: Option<&str>, meta: &pages::Meta, body: String) -> Reply {
        Reply::new(status, "text/html; charset=utf-8", pages::page(title, nav, viewer, meta, &body).into_bytes().into())
    }

    fn json<T: serde::Serialize>(value: &T) -> Reply {
        Reply::new(200, "application/json", serde_json::to_vec(value).unwrap_or_default().into())
    }

    fn text(status: u16, text: &str) -> Reply {
        Reply::new(status, "text/plain; charset=utf-8", text.as_bytes().to_vec().into())
    }

    fn redirect(to: String) -> Reply {
        Reply { location: Some(to), ..Reply::new(302, "text/plain", Cow::Borrowed(&[])) }
    }

    /// Where to go after a post.
    fn see_other(to: String) -> Reply {
        Reply { location: Some(to), ..Reply::new(303, "text/plain", Cow::Borrowed(&[])) }
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
        Reply { cache: "public, max-age=86400", ranges: true, ..Reply::new(200, kind, Cow::Borrowed(bytes)) }
    }

    /// Pages that differ by the visitor's system (the download button).
    fn per_visitor(mut self) -> Reply {
        self.cache = "private, max-age=30";
        self
    }
}

/// The reply to a request.
fn route(state: &State, req: &Req) -> Reply {
    let segs: Vec<&str> = req.path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
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
            let live = pages::Live { online: &online, rooms: &rooms, latest: &latest };
            req.page(200, "a taiko client for osu! beatmaps", "", pages::home(&ranked, &maps, ranked.len(), plays, req.os, live)).per_visitor()
        }
        ["rankings"] => req.page(200, "Performance Ranking", "rankings", pages::rankings(&state.scores.lock().unwrap().rankings(RANKINGS))),
        ["api", "rankings"] => Reply::json(&state.scores.lock().unwrap().rankings(RANKINGS)),
        ["download"] => req.page(200, "Download", "download", pages::download(req.os)).per_visitor(),
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
                let tie = req.key().map(|me| state.friendship(&me, &key)).unwrap_or(pages::Friendship::None);
                let notice = req.arg("msg");
                req.page_with(200, &p.name, "rankings", &pages::profile_meta(&p), pages::profile(&p, &extras, activity.as_ref(), &friends, tie, notice.as_deref()))
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
        ["chat"] | ["chat", _] => {
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else { return Reply::redirect("/login".into()) };
            let people = state.conversations(&key);
            let error = req.arg("err");
            match segs.get(1) {
                None => req.page(200, "Chat", "chat", pages::chat(me, None, &state.global_chat(), &people, true, error.as_deref())),
                Some(other) => {
                    let Some(p) = state.profile(other) else { return req.not_found("There is no player by that name") };
                    let friends = state.friendship(&key, &p.name.to_ascii_lowercase()) == pages::Friendship::Friends;
                    let lines = if friends { state.chat_with(&key, &p.name) } else { Vec::new() };
                    req.page(200, &format!("Chat with {}", p.name), "chat", pages::chat(me, Some(&p.name), &lines, &people, friends, error.as_deref()))
                }
            }
        }
        ["api", "chat"] | ["api", "chat", _] => {
            let Some(key) = req.key() else { return Reply::text(401, "log in first") };
            match segs.get(2) {
                None => Reply::json(&state.global_chat()),
                Some(other) => Reply::json(&state.chat_with(&key, other)),
            }
        }
        ["beatmaps"] => req.page(200, "Beatmaps", "beatmaps", pages::beatmaps(&state.scores.lock().unwrap().maps(MAPS))),
        ["api", "beatmaps"] => Reply::json(&state.scores.lock().unwrap().maps(MAPS)),
        ["m", hash] | ["api", "m", hash] => {
            let Ok(map) = u64::from_str_radix(hash, 16) else { return req.not_found("That is not a map link") };
            let scores = state.scores.lock().unwrap();
            let rows = scores.map_scores(map);
            if segs[0] == "api" {
                return Reply::json(&rows);
            }
            match scores.map(map) {
                Some(m) => req.page_with(200, &m.name, "beatmaps", &pages::map_meta(&m, &rows), pages::map_page(&m, &rows)),
                None => req.not_found("Nobody has sent a play on that map yet"),
            }
        }
        ["mods"] => req.page(200, "Mods", "mods", pages::mods_page()),
        [dir @ ("clips" | "site"), name] => match pages::STATIC_FILES.iter().find(|(f, _)| f.strip_prefix(dir).and_then(|f| f.strip_prefix('/')) == Some(name)) {
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
                Ok((name, token)) => Reply { cookie: Some(token), ..Reply::see_other(format!("/u/{name}")) },
                Err(e) => req.page(401, "Log in", "", pages::login(Some(&e), name)),
            }
        }
        ["logout"] => {
            if let Some(t) = &req.token {
                state.logout(t);
            }
            Reply { cookie: Some(String::new()), ..Reply::see_other("/".into()) }
        }
        ["friend", other] => {
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else { return Reply::see_other("/login".into()) };
            let Some(p) = state.profile(other) else { return req.not_found("There is no player by that name") };
            let msg = state.add_friend(&key, me, &p.name).unwrap_or_else(|e| e);
            Reply::see_other(format!("/u/{}?msg={}", p.name, encode(&msg)))
        }
        ["chat"] | ["chat", _] => {
            let js = req.query.split('&').any(|kv| kv == "js=1");
            let (Some(me), Some(key)) = (req.viewer.as_deref(), req.key()) else {
                return if js { Reply::json(&serde_json::json!({ "error": "log in first" })) } else { Reply::see_other("/login".into()) };
            };
            let text = req.field("text");
            let (sent, back) = match segs.get(1) {
                None => (state.post_global(me, text, |hub| hub.web_chat_allowed(&key)), "/chat".to_string()),
                Some(other) => (state.post_chat(&key, me, other, text, |hub| hub.web_chat_allowed(&key)), format!("/chat/{}", encode(other))),
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
    use super::{byte_range, decode, encode, form};

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
