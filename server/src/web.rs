//! The website: player rankings, profiles and map leaderboards as pages and as JSON, on a port
//! of its own. Real DDoS protection is a proxy in front (Cloudflare); on its own it caps the
//! connections at once, requests per address and how long a request may take, and keeps
//! nothing per request but a reply.
//!
//!   /                rankings          /api/rankings
//!   /u/<name>        a profile         /api/u/<name>
//!   /m/<map hash>    a map's scores    /api/m/<map hash>

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::online::{OnlineScore, Profile};
use crate::State;

/// Connections being answered at once; more are closed straight away.
const MAX_CONNECTIONS: usize = 128;
/// Requests per address per minute.
const REQUESTS_PER_MINUTE: usize = 120;
/// Longest request head (request line and headers).
const MAX_HEAD: u64 = 8 * 1024;
const RANKINGS: usize = 100;

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
    let web2 = web;
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(120));
        web2.recent.lock().unwrap().retain(|_, v| v.last().is_some_and(|t| t.elapsed() < Duration::from_secs(60)));
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
    // Headers: only the proxy's word for who's asking.
    let mut visitor = None;
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) | Err(_) => return,
            Ok(_) if h == "\r\n" || h == "\n" => break,
            Ok(_) => {
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("cf-connecting-ip") || (visitor.is_none() && k.eq_ignore_ascii_case("x-forwarded-for")) {
                        visitor = v.split(',').next().and_then(|ip| ip.trim().parse::<IpAddr>().ok());
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
        route(state, target)
    };
    let mut out = &stream;
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: public, max-age=30\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'\r\n{}Connection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        reply.kind,
        reply.body.len(),
        reply.location.as_ref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default()
    );
    let _ = out.write_all(head.as_bytes());
    if method != "HEAD" {
        let _ = out.write_all(reply.body.as_bytes());
    }
}

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
}

impl Reply {
    fn html(status: u16, title: &str, body: String) -> Reply {
        Reply { status, kind: "text/html; charset=utf-8", body: page(title, &body), location: None }
    }

    fn json<T: serde::Serialize>(value: &T) -> Reply {
        Reply { status: 200, kind: "application/json", body: serde_json::to_string(value).unwrap_or_default(), location: None }
    }

    fn text(status: u16, text: &str) -> Reply {
        Reply { status, kind: "text/plain; charset=utf-8", body: text.to_string(), location: None }
    }

    fn redirect(to: String) -> Reply {
        Reply { status: 302, kind: "text/plain", body: String::new(), location: Some(to) }
    }
}

#[derive(serde::Serialize)]
struct Ranked {
    rank: usize,
    name: String,
    pp: f64,
    plays: u32,
}

/// The reply for a GET of `target` (path and query).
fn route(state: &State, target: &str) -> Reply {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [] => rankings_page(state),
        ["api", "rankings"] => Reply::json(&rankings(state)),
        // The search box: /u?name=...
        ["u"] => match query.split('&').find_map(|kv| kv.strip_prefix("name=")) {
            Some(name) if crate::online::valid_name(name) => Reply::redirect(format!("/u/{name}")),
            _ => Reply::redirect("/".into()),
        },
        ["u", name] => match state.profile(name) {
            Some(p) => profile_page(&p),
            None => not_found("There is no player by that name"),
        },
        ["api", "u", name] => match state.profile(name) {
            Some(p) => Reply::json(&p),
            None => Reply::text(404, "no such player"),
        },
        ["m", hash] | ["api", "m", hash] => {
            let Ok(map) = u64::from_str_radix(hash, 16) else { return not_found("That is not a map link") };
            let rows = state.scores.lock().unwrap().map_scores(map);
            if segs[0] == "api" {
                return Reply::json(&rows);
            }
            let name = state.scores.lock().unwrap().map_name(map);
            match name {
                Some(name) => map_page(&name, &rows),
                None => not_found("Nobody has sent a play on that map yet"),
            }
        }
        ["robots.txt"] => Reply::text(200, "User-agent: *\nAllow: /\n"),
        _ => not_found("There is nothing at this address"),
    }
}

fn rankings(state: &State) -> Vec<Ranked> {
    let list = state.scores.lock().unwrap().rankings(RANKINGS);
    list.into_iter().enumerate().map(|(i, (name, pp, plays))| Ranked { rank: i + 1, name, pp, plays }).collect()
}

fn not_found(why: &str) -> Reply {
    Reply::html(404, "Not found", format!("<h1>Not found</h1><p>{}.</p><p><a href=\"/\">Back to the rankings</a></p>", esc(why)))
}

fn rankings_page(state: &State) -> Reply {
    let rows: String = rankings(state)
        .iter()
        .map(|r| format!("<tr><td class=n>{}</td><td>{}</td><td class=n>{:.0}</td><td class=n>{}</td></tr>", r.rank, player_link(&r.name), r.pp, r.plays))
        .collect();
    let table = if rows.is_empty() {
        "<p>No plays yet. They show up here once someone finishes a map in taiko-rs while logged in.</p>".to_string()
    } else {
        format!("<div class=scroll><table><tr><th class=n>#</th><th>Player</th><th class=n>pp</th><th class=n>Plays</th></tr>{rows}</table></div>")
    };
    Reply::html(200, "Rankings", format!("<h1>Rankings</h1>{table}"))
}

fn profile_page(p: &Profile) -> Reply {
    let rank = p.rank.map(|r| format!("#{r}")).unwrap_or_else(|| "unranked".into());
    let head = format!("<h1>{}</h1><p>{rank}, {:.0} pp, {} plays. Joined {}.</p>", esc(&p.name), p.pp, p.plays, ago(p.joined));
    let best = scores_table(&p.top, false, "No plays with pp yet.");
    let recent = scores_table(&p.recent, false, "No plays yet.");
    Reply::html(200, &p.name, format!("{head}<h2>Best plays</h2>{best}<h2>Recent plays</h2>{recent}"))
}

fn map_page(name: &str, rows: &[OnlineScore]) -> Reply {
    Reply::html(200, name, format!("<h1>{}</h1>{}", esc(name), scores_table(rows, true, "No scores yet.")))
}

/// Plays as a table: with `players` a column of who played (a map's board), else of the map.
fn scores_table(rows: &[OnlineScore], players: bool, empty: &str) -> String {
    if rows.is_empty() {
        return format!("<p>{empty}</p>");
    }
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let who = if players { player_link(&r.player) } else { format!("<a href=\"/m/{:x}\">{}</a>", r.map_hash, esc(&r.map)) };
            let mut mods = esc(&r.mods.concat());
            if (r.rate - 1.0).abs() > 1e-6 {
                mods += &format!(" {:.2}x", r.rate);
            }
            let pp = r.pp.map(|p| format!("{p:.0}")).unwrap_or_default();
            format!(
                "<tr><td class=n>{}</td><td>{who}</td><td class=n>{}</td><td class=n>{:.2}%</td><td class=n>{}</td><td class=n>{}</td><td>{mods}</td><td class=n>{pp}</td><td class=when>{}</td></tr>",
                i + 1,
                r.score,
                r.accuracy * 100.0,
                r.max_combo,
                r.miss,
                ago(r.date)
            )
        })
        .collect();
    let first = if players { "Player" } else { "Map" };
    format!(
        "<div class=scroll><table><tr><th class=n>#</th><th>{first}</th><th class=n>Score</th><th class=n>Acc</th><th class=n>Combo</th><th class=n>Miss</th><th>Mods</th><th class=n>pp</th><th class=when></th></tr>{body}</table></div>"
    )
}

fn player_link(name: &str) -> String {
    format!("<a href=\"/u/{0}\">{0}</a>", esc(name))
}

fn ago(t: u64) -> String {
    let s = crate::now_secs().saturating_sub(t);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        86_400..=2_591_999 => format!("{}d ago", s / 86_400),
        _ => format!("{}mo ago", s / 2_592_000),
    }
}

/// Text made safe for HTML.
fn esc(s: &str) -> String {
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

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>{} - taiko-rs</title><style>
body{{margin:0;background:#141418;color:#ddd;font:15px/1.5 Verdana,Geneva,sans-serif}}
header,main{{max-width:960px;margin:0 auto;padding:0 16px}}
header{{display:flex;flex-wrap:wrap;gap:6px 20px;align-items:baseline;padding-top:18px;padding-bottom:12px;border-bottom:1px solid #333}}
header .name{{font-weight:bold;color:#eb452c;text-decoration:none;margin-right:auto}}
a{{color:#ddd}}a:hover{{color:#fff}}
input{{font:inherit;background:#141418;color:#ddd;border:1px solid #444;padding:2px 6px;width:11em}}
h1{{font-size:24px;margin:24px 0 4px;overflow-wrap:anywhere}}h2{{font-size:16px;margin:28px 0 6px}}
.scroll{{overflow-x:auto}}table{{border-collapse:collapse;width:100%}}
th,td{{padding:5px 8px;text-align:left;border-bottom:1px solid #2a2a2e;vertical-align:top}}th{{font-weight:normal;color:#888;font-size:13px}}
td:nth-child(2){{overflow-wrap:anywhere}}.n{{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap;width:1%}}.when{{color:#888;white-space:nowrap;font-size:13px}}
@media(max-width:700px){{.when,td:nth-child(5),th:nth-child(5),td:nth-child(6),th:nth-child(6){{display:none}}}}
</style></head><body><header><a class=name href="/">taiko-rs</a><a href="/">Rankings</a><form action="/u"><input name=name placeholder="Player name" maxlength=16 aria-label="Find a player"></form></header><main>{}</main></body></html>"#,
        esc(title),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped() {
        assert_eq!(esc("<b>\"A&B'</b>"), "&lt;b&gt;&quot;A&amp;B&#39;&lt;/b&gt;");
        assert!(page("<x>", "").contains("&lt;x&gt; - taiko-rs"));
    }
}
