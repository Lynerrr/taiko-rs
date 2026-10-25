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
//!   /clips/<file>     the home page's gameplay clips (with byte ranges, for video players)

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
/// Longest request head (request line and headers).
const MAX_HEAD: u64 = 8 * 1024;
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

fn answer(state: &State, web: &Web, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let peer = stream.peer_addr().map(|a| a.ip()).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let mut reader = BufReader::new((&stream).take(MAX_HEAD));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    // Headers: the proxy's word for who's asking, and their browser (for the download button).
    let mut visitor = None;
    let mut agent = String::new();
    let mut range = None;
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return,
            Ok(_) if h == "\r\n" || h == "\n" => break,
            Ok(_) => {
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("cf-connecting-ip") || (visitor.is_none() && k.eq_ignore_ascii_case("x-forwarded-for")) {
                        visitor = v.split(',').next().and_then(|ip| ip.trim().parse::<IpAddr>().ok());
                    } else if k.eq_ignore_ascii_case("user-agent") {
                        agent = v.trim().to_string();
                    } else if k.eq_ignore_ascii_case("range") {
                        range = Some(v.trim().to_string());
                    }
                }
            }
        }
    }
    let ip = if web.behind_proxy { visitor.unwrap_or(peer) } else { peer };
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut reply = if !web.allow(ip) {
        Reply::text(429, "too many requests, wait a minute")
    } else if method != "GET" && method != "HEAD" {
        Reply::text(405, "only GET")
    } else {
        route(state, target, Os::of(&agent))
    };
    let mut extra = reply.location.as_ref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
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

/// Styles inline and the font from Google; map covers from osu!; clips from here; no scripts at all.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' https://assets.ppy.sh; media-src 'self'; form-action 'self'";

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
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
    cache: &'static str,
    /// Byte ranges may be asked for (the clips).
    ranges: bool,
}

impl Reply {
    fn new(status: u16, kind: &'static str, body: Cow<'static, [u8]>) -> Reply {
        Reply { status, kind, body, location: None, cache: "public, max-age=30", ranges: false }
    }

    /// A page; `nav` is the header link it's under ("" for none).
    fn html(status: u16, title: &str, nav: &str, body: String) -> Reply {
        Reply::new(status, "text/html; charset=utf-8", pages::page(title, nav, &body).into_bytes().into())
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

    /// A clip or its poster: the same for everyone and never changing within a release.
    fn clip(name: &str, bytes: &'static [u8]) -> Reply {
        let kind = if name.ends_with(".mp4") { "video/mp4" } else { "image/jpeg" };
        Reply { cache: "public, max-age=86400", ranges: true, ..Reply::new(200, kind, Cow::Borrowed(bytes)) }
    }

    /// Pages that differ by the visitor's system (the download button).
    fn per_visitor(mut self) -> Reply {
        self.cache = "private, max-age=30";
        self
    }
}

fn not_found(why: &str) -> Reply {
    Reply::html(404, "Not found", "", pages::not_found(why))
}

/// The reply for a GET of `target` (path and query) from a visitor on `os`.
fn route(state: &State, target: &str, os: Os) -> Reply {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [] => {
            let scores = state.scores.lock().unwrap();
            let ranked = scores.rankings(usize::MAX);
            let maps = scores.maps(6);
            let plays = scores.play_count();
            drop(scores);
            Reply::html(200, "osu!taiko, rebuilt", "", pages::home(&ranked, &maps, ranked.len(), plays, os)).per_visitor()
        }
        ["rankings"] => Reply::html(200, "Performance Ranking", "rankings", pages::rankings(&state.scores.lock().unwrap().rankings(RANKINGS))),
        ["api", "rankings"] => Reply::json(&state.scores.lock().unwrap().rankings(RANKINGS)),
        ["download"] => Reply::html(200, "Download", "download", pages::download(os)).per_visitor(),
        // The search box: /u?name=...
        ["u"] => match query.split('&').find_map(|kv| kv.strip_prefix("name=")) {
            Some(name) if crate::online::valid_name(name) => Reply::redirect(format!("/u/{name}")),
            _ => Reply::redirect("/rankings".into()),
        },
        ["u", name] => match state.profile(name) {
            Some(p) => Reply::html(200, &p.name, "rankings", pages::profile(&p)),
            None => not_found("There is no player by that name"),
        },
        ["api", "u", name] => match state.profile(name) {
            Some(p) => Reply::json(&p),
            None => Reply::text(404, "no such player"),
        },
        ["beatmaps"] => Reply::html(200, "Beatmaps", "beatmaps", pages::beatmaps(&state.scores.lock().unwrap().maps(MAPS))),
        ["api", "beatmaps"] => Reply::json(&state.scores.lock().unwrap().maps(MAPS)),
        ["m", hash] | ["api", "m", hash] => {
            let Ok(map) = u64::from_str_radix(hash, 16) else { return not_found("That is not a map link") };
            let scores = state.scores.lock().unwrap();
            let rows = scores.map_scores(map);
            if segs[0] == "api" {
                return Reply::json(&rows);
            }
            match scores.map(map) {
                Some(m) => Reply::html(200, &m.name, "beatmaps", pages::map_page(&m, &rows)),
                None => not_found("Nobody has sent a play on that map yet"),
            }
        }
        ["clips", name] => match pages::CLIP_FILES.iter().find(|(f, _)| f == name) {
            Some((f, bytes)) => Reply::clip(f, bytes),
            None => not_found("There is no such clip"),
        },
        ["robots.txt"] => Reply::text(200, "User-agent: *\nAllow: /\n"),
        _ => not_found("There is nothing at this address"),
    }
}

#[cfg(test)]
mod tests {
    use super::byte_range;

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
