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
                    }
                }
            }
        }
    }
    let ip = if web.behind_proxy { visitor.unwrap_or(peer) } else { peer };
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let reply = if !web.allow(ip) {
        Reply::text(429, "too many requests, wait a minute")
    } else if method != "GET" && method != "HEAD" {
        Reply::text(405, "only GET")
    } else {
        route(state, target, Os::of(&agent))
    };
    let mut out = &stream;
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: {CSP}\r\n{}Connection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        reply.kind,
        reply.body.len(),
        // Pages differ by the visitor's system (the download button).
        if reply.per_visitor { "private, max-age=30" } else { "public, max-age=30" },
        reply.location.as_ref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default()
    );
    let _ = out.write_all(head.as_bytes());
    if method != "HEAD" {
        let _ = out.write_all(reply.body.as_bytes());
    }
}

/// Styles inline and the font from Google; map covers from osu!; no scripts at all.
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src https://assets.ppy.sh; form-action 'self'";

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        405 => "Method Not Allowed",
        429 => "Too Many Requests",
        _ => "Error",
    }
}

struct Reply {
    status: u16,
    kind: &'static str,
    body: String,
    location: Option<String>,
    per_visitor: bool,
}

impl Reply {
    fn html(status: u16, title: &str, body: String) -> Reply {
        Reply { status, kind: "text/html; charset=utf-8", body: pages::page(title, &body), location: None, per_visitor: false }
    }

    fn json<T: serde::Serialize>(value: &T) -> Reply {
        Reply { status: 200, kind: "application/json", body: serde_json::to_string(value).unwrap_or_default(), location: None, per_visitor: false }
    }

    fn text(status: u16, text: &str) -> Reply {
        Reply { status, kind: "text/plain; charset=utf-8", body: text.to_string(), location: None, per_visitor: false }
    }

    fn redirect(to: String) -> Reply {
        Reply { status: 302, kind: "text/plain", body: String::new(), location: Some(to), per_visitor: false }
    }

    fn per_visitor(mut self) -> Reply {
        self.per_visitor = true;
        self
    }
}

fn not_found(why: &str) -> Reply {
    Reply::html(404, "Not found", pages::not_found(why))
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
            Reply::html(200, "osu!taiko, rebuilt", pages::home(&ranked, &maps, ranked.len(), plays, os)).per_visitor()
        }
        ["rankings"] => Reply::html(200, "Performance Ranking", pages::rankings(&state.scores.lock().unwrap().rankings(RANKINGS))),
        ["api", "rankings"] => Reply::json(&state.scores.lock().unwrap().rankings(RANKINGS)),
        ["download"] => Reply::html(200, "Download", pages::download(os)).per_visitor(),
        // The search box: /u?name=...
        ["u"] => match query.split('&').find_map(|kv| kv.strip_prefix("name=")) {
            Some(name) if crate::online::valid_name(name) => Reply::redirect(format!("/u/{name}")),
            _ => Reply::redirect("/rankings".into()),
        },
        ["u", name] => match state.profile(name) {
            Some(p) => Reply::html(200, &p.name, pages::profile(&p)),
            None => not_found("There is no player by that name"),
        },
        ["api", "u", name] => match state.profile(name) {
            Some(p) => Reply::json(&p),
            None => Reply::text(404, "no such player"),
        },
        ["beatmaps"] => Reply::html(200, "Beatmaps", pages::beatmaps(&state.scores.lock().unwrap().maps(MAPS))),
        ["api", "beatmaps"] => Reply::json(&state.scores.lock().unwrap().maps(MAPS)),
        ["m", hash] | ["api", "m", hash] => {
            let Ok(map) = u64::from_str_radix(hash, 16) else { return not_found("That is not a map link") };
            let scores = state.scores.lock().unwrap();
            let rows = scores.map_scores(map);
            if segs[0] == "api" {
                return Reply::json(&rows);
            }
            match scores.map(map) {
                Some(m) => Reply::html(200, &m.name, pages::map_page(&m, &rows)),
                None => not_found("Nobody has sent a play on that map yet"),
            }
        }
        ["robots.txt"] => Reply::text(200, "User-agent: *\nAllow: /\n"),
        _ => not_found("There is nothing at this address"),
    }
}
