//! The Discord bot: plays and profiles from the server on Discord (`>rs`, `>top`, `>osu`, `>lb`,
//! `>ranking`... `>help` lists them), and the game's changes posted as they're pushed.
//!
//! Its token is DISCORD_TOKEN or DIR/discord-token (no token: no bot). Discord accounts linked
//! to players (`>link name`) are kept in DIR/discord-links.json; the channel the changelog goes
//! to is DIR/discord-changelog (a channel id), and the commits posted there
//! DIR/discord-changelog.json. It talks to Discord's gateway (a websocket) for messages and its
//! HTTP API for replies; everything runs on threads of its own, and a broken connection is
//! simply made again.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::Message;

use crate::online::{self, OnlineScore, WEBSITE};
use crate::State;

const API: &str = "https://discord.com/api/v10";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const USER_AGENT: &str = "DiscordBot (https://github.com/Lynerrr/taiko-rs, 1)";
/// The repository and branch whose commits make the changelog.
const REPO: &str = "Lynerrr/taiko-rs";
const BRANCH: &str = "worktree-taiko-client";
/// The interface's red.
const COLOUR: u32 = 0xEB452C;
/// Plays a page of `>top` and `>lb` shows.
const PAGE: usize = 5;
const LB_PAGE: usize = 10;

pub(crate) struct Bot {
    token: String,
    dir: PathBuf,
    state: Arc<State>,
    /// Discord user id -> player (their key).
    links: Mutex<HashMap<String, String>>,
    /// The map last shown in each channel (`>c`, `>lb`), and the map of each play the bot
    /// showed (replying to one with `>c`).
    last_map: Mutex<HashMap<String, u64>>,
    shown: Mutex<HashMap<String, u64>>,
    http: ureq::Agent,
}

/// Starts the bot when there's a token for it.
pub fn start(state: Arc<State>, dir: PathBuf) {
    let token = std::env::var("DISCORD_TOKEN").ok().or_else(|| std::fs::read_to_string(dir.join("discord-token")).ok()).map(|t| t.trim().to_string()).unwrap_or_default();
    if token.is_empty() {
        println!("discord: no token (DISCORD_TOKEN or discord-token), the bot is off");
        return;
    }
    let links = std::fs::read_to_string(dir.join("discord-links.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let http: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into();
    let bot = Arc::new(Bot { token, dir, state, links: Mutex::new(links), last_map: Mutex::default(), shown: Mutex::default(), http });
    {
        let bot = bot.clone();
        std::thread::Builder::new().name("discord".into()).spawn(move || gateway(bot)).expect("discord thread");
    }
    std::thread::Builder::new().name("changelog".into()).spawn(move || changelog(bot)).expect("changelog thread");
}

// ---------------------------------------------------------------------------------------------
// The gateway: messages in.

/// Why a gateway session ended for good.
enum Stop {
    /// Discord won't have the bot (a bad token, intents not allowed): it stays off.
    Refused(String),
}

fn gateway(bot: Arc<Bot>) {
    let mut wait = Duration::from_secs(5);
    loop {
        let started = Instant::now();
        match session(&bot) {
            Ok(Stop::Refused(why)) => {
                eprintln!("discord: {why}; the bot is off until the server restarts");
                return;
            }
            Err(e) => eprintln!("discord: {e}"),
        }
        // Back off while it keeps failing straight away.
        wait = if started.elapsed() > Duration::from_secs(60) { Duration::from_secs(5) } else { (wait * 2).min(Duration::from_secs(120)) };
        std::thread::sleep(wait);
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

/// One connection to the gateway, until it breaks.
fn session(bot: &Arc<Bot>) -> Result<Stop, String> {
    let host = "gateway.discord.gg";
    let tcp = TcpStream::connect((host, 443)).map_err(|e| format!("connecting: {e}"))?;
    let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| e.to_string())?;
    let tls = rustls::ClientConnection::new(tls_config(), name).map_err(|e| e.to_string())?;
    let stream = rustls::StreamOwned::new(tls, tcp);
    let (mut ws, _) = tungstenite::client(GATEWAY, stream).map_err(|e| format!("gateway handshake: {e}"))?;
    // Reads wake up now and then to send the heartbeat.
    ws.get_mut().sock.set_read_timeout(Some(Duration::from_millis(500))).map_err(|e| e.to_string())?;
    let mut seq: Value = Value::Null;
    let mut beat: Option<(Duration, Instant)> = None;
    let mut acked = true;
    loop {
        if let Some((every, next)) = &mut beat {
            if Instant::now() >= *next {
                if !acked {
                    return Err("no heartbeat ack, connecting again".into());
                }
                send(&mut ws, &json!({ "op": 1, "d": seq }))?;
                acked = false;
                *next += *every;
            }
        }
        let text = match ws.read() {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(frame)) => {
                let code = frame.map(|f| u16::from(f.code)).unwrap_or(0);
                return match code {
                    4004 => Ok(Stop::Refused("Discord refused the token (discord-token)".into())),
                    4014 => Ok(Stop::Refused("intents not allowed: turn on Message Content Intent in the Developer Portal".into())),
                    _ => Err(format!("Discord closed the connection ({code})")),
                };
            }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(format!("gateway: {e}")),
        };
        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
        if !event["s"].is_null() {
            seq = event["s"].clone();
        }
        match event["op"].as_u64() {
            // Hello: start the heartbeat (the first at a random point of the interval) and log in.
            Some(10) => {
                let every = Duration::from_millis(event["d"]["heartbeat_interval"].as_u64().unwrap_or(41_250));
                let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as f64 / 1e9;
                beat = Some((every, Instant::now() + every.mul_f64(jitter)));
                send(&mut ws, &identify(&bot.token))?;
            }
            Some(11) => acked = true,
            Some(1) => send(&mut ws, &json!({ "op": 1, "d": seq }))?,
            Some(7) => return Err("Discord asked to reconnect".into()),
            Some(9) => {
                std::thread::sleep(Duration::from_secs(3));
                return Err("session invalidated".into());
            }
            Some(0) => match event["t"].as_str() {
                Some("READY") => println!("discord: logged in as {}", event["d"]["user"]["username"].as_str().unwrap_or("?")),
                Some("MESSAGE_CREATE") => {
                    let message = event["d"].clone();
                    if message["author"]["bot"].as_bool() != Some(true) && message["content"].as_str().is_some_and(|c| c.trim_start().starts_with('>')) {
                        let bot = bot.clone();
                        std::thread::spawn(move || bot.answer(&message));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn send<S: std::io::Read + std::io::Write>(ws: &mut tungstenite::WebSocket<S>, v: &Value) -> Result<(), String> {
    ws.send(Message::text(v.to_string())).map_err(|e| format!("gateway send: {e}"))
}

/// Logging in: servers' messages (with what they say), DMs, and the status line.
fn identify(token: &str) -> Value {
    const GUILDS: u64 = 1;
    const GUILD_MESSAGES: u64 = 1 << 9;
    const DIRECT_MESSAGES: u64 = 1 << 12;
    const MESSAGE_CONTENT: u64 = 1 << 15;
    json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT,
            "properties": { "os": "linux", "browser": "taiko-server", "device": "taiko-server" },
            "presence": {
                "activities": [{ "type": 4, "name": "Custom Status", "state": ">help · rstaiko.auneu.space" }],
                "status": "online",
                "since": null,
                "afk": false
            }
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Commands: what's asked, and the answer.

/// A message's command: its word in lower case, the number after it (`>rs2`, `>top3`) and the
/// rest.
#[derive(Debug, PartialEq)]
pub(crate) struct Command {
    pub word: String,
    pub n: Option<usize>,
    pub rest: String,
}

pub(crate) fn parse(content: &str) -> Option<Command> {
    let body = content.trim().strip_prefix('>')?;
    let (head, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    let head = head.to_ascii_lowercase();
    let digits = head.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let word = head[..head.len() - digits.len()].to_string();
    if word.is_empty() {
        return None;
    }
    let n = if digits.is_empty() { None } else { Some(digits.parse().ok()?) };
    Some(Command { word, n, rest: rest.trim().to_string() })
}

/// A reply: text, or an embed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    Text(String),
    Embed(Value),
}

impl Bot {
    /// Answers a message (on a thread of its own: it waits on Discord's API).
    fn answer(&self, message: &Value) {
        let content = message["content"].as_str().unwrap_or("");
        let Some(cmd) = parse(content) else { return };
        let channel = message["channel_id"].as_str().unwrap_or("").to_string();
        let author = message["author"]["id"].as_str().unwrap_or("").to_string();
        // Replying to a play the bot showed: that play's map.
        let replied = message["referenced_message"]["id"].as_str().and_then(|id| self.shown.lock().unwrap().get(id).copied());
        let map = replied.or_else(|| self.last_map.lock().unwrap().get(&channel).copied());
        let Some((answer, shows)) = self.respond(&cmd, &author, map) else { return };
        match self.reply(&channel, message["id"].as_str(), &answer) {
            Ok(id) => {
                if let Some(m) = shows {
                    self.last_map.lock().unwrap().insert(channel, m);
                    let mut shown = self.shown.lock().unwrap();
                    if shown.len() > 5000 {
                        shown.clear();
                    }
                    shown.insert(id, m);
                }
            }
            Err(e) => eprintln!("discord: reply in {channel}: {e}"),
        }
    }

    /// The answer to `cmd` from Discord user `author`, with the map it shows (if one). `map`:
    /// the map last shown here (or replied to). None: not a command of the bot's.
    pub(crate) fn respond(&self, cmd: &Command, author: &str, map: Option<u64>) -> Option<(Answer, Option<u64>)> {
        let text = |s: String| Some((Answer::Text(s), None));
        let n = cmd.n.unwrap_or(1).max(1);
        match cmd.word.as_str() {
            "help" | "h" => Some((Answer::Embed(help()), None)),
            "link" => {
                let name = cmd.rest.trim();
                if name.is_empty() {
                    return text("`>link yourname`, with your taiko.rs account's name.".into());
                }
                let Some(display) = self.player(name) else { return text(format!("There's no player called `{}`.", clean(name))) };
                self.links.lock().unwrap().insert(author.to_string(), name.to_ascii_lowercase());
                self.save_links();
                text(format!("Linked to **{display}**. Now `>rs`, `>top` and `>osu` show your plays."))
            }
            "unlink" => {
                let had = self.links.lock().unwrap().remove(author).is_some();
                self.save_links();
                text(if had { "Unlinked.".into() } else { "You weren't linked to anyone.".into() })
            }
            "rs" | "recent" | "r" => {
                let key = match self.target(&cmd.rest, author) {
                    Ok(k) => k,
                    Err(e) => return text(e),
                };
                let name = self.state.display_name(&key);
                let plays = self.state.scores.lock().unwrap().plays_of(&key);
                if plays.is_empty() {
                    return text(format!("`{name}` hasn't sent a play yet."));
                }
                let Some(play) = plays.get(n - 1) else { return text(format!("`{name}` has only {} recent plays.", plays.len())) };
                Some((Answer::Embed(self.play_embed(&key, play)), Some(play.map_hash)))
            }
            "c" | "compare" => {
                let key = match self.target(&cmd.rest, author) {
                    Ok(k) => k,
                    Err(e) => return text(e),
                };
                let Some(map) = map else { return text("Show a play here first (`>rs`), or reply to one.".into()) };
                let name = self.state.display_name(&key);
                let mut plays: Vec<OnlineScore> = self.state.scores.lock().unwrap().plays_of(&key).into_iter().filter(|p| p.map_hash == map).collect();
                if plays.is_empty() {
                    let label = self.state.scores.lock().unwrap().map(map).map(|m| m.name).unwrap_or_else(|| "that map".into());
                    return text(format!("`{name}` hasn't played {label}."));
                }
                plays.sort_by(|a, b| b.score.cmp(&a.score));
                Some((Answer::Embed(self.plays_embed(&format!("{name} on {}", plays[0].map), &plays, 0, &key)), Some(map)))
            }
            "top" | "best" => {
                let key = match self.target(&cmd.rest, author) {
                    Ok(k) => k,
                    Err(e) => return text(e),
                };
                let name = self.state.display_name(&key);
                let best = self.state.scores.lock().unwrap().best_of(&key);
                if best.is_empty() {
                    return text(format!("`{name}` hasn't sent a play yet."));
                }
                let pages = best.len().div_ceil(PAGE);
                let page = n.min(pages);
                let shown: Vec<OnlineScore> = best.iter().skip((page - 1) * PAGE).take(PAGE).cloned().collect();
                let mut e = self.plays_embed(&format!("Best plays of {name}"), &shown, (page - 1) * PAGE, &key);
                e["footer"] = json!({ "text": format!("page {page}/{pages} · taiko.rs") });
                Some((Answer::Embed(e), None))
            }
            "osu" | "profile" | "p" => {
                let key = match self.target(&cmd.rest, author) {
                    Ok(k) => k,
                    Err(e) => return text(e),
                };
                let p = self.state.profile(&key)?;
                Some((Answer::Embed(self.profile_embed(&p)), None))
            }
            "lb" | "leaderboard" => {
                let Some(map) = map else { return text("Show a play here first (`>rs`), or reply to one.".into()) };
                let board = self.state.scores.lock().unwrap().map_scores(map);
                if board.is_empty() {
                    return text("Nobody has played that map here.".into());
                }
                let pages = board.len().div_ceil(LB_PAGE);
                let page = n.min(pages);
                let lines: Vec<String> = board
                    .iter()
                    .enumerate()
                    .skip((page - 1) * LB_PAGE)
                    .take(LB_PAGE)
                    .map(|(i, s)| {
                        format!(
                            "**#{}** [{}]({WEBSITE}/u/{}) · {} · {:.2}% · {}{}",
                            i + 1,
                            clean(&s.player),
                            url_part(&s.player),
                            thousands(s.score),
                            s.accuracy * 100.0,
                            pp_text(s.pp),
                            mods_text(s)
                        )
                    })
                    .collect();
                let first = &board[0];
                let e = json!({
                    "title": first.map,
                    "url": format!("{WEBSITE}/m/{}", first.map_hash),
                    "description": lines.join("\n"),
                    "color": COLOUR,
                    "thumbnail": cover_thumb(first.set_id),
                    "footer": { "text": format!("{} plays on this map · page {page}/{pages}", board.len()) },
                });
                Some((Answer::Embed(e), Some(map)))
            }
            "ranking" | "rankings" | "rank" => {
                let (rows, players, plays) = {
                    let s = self.state.scores.lock().unwrap();
                    let all = s.rankings(usize::MAX);
                    (all.iter().take(10).cloned().collect::<Vec<_>>(), all.len(), s.play_count())
                };
                if rows.is_empty() {
                    return text("Nobody has pp yet.".into());
                }
                let lines: Vec<String> = rows
                    .iter()
                    .map(|r| format!("**#{}** [{}]({WEBSITE}/u/{}) · **{:.0}pp** · {:.2}% · {} plays", r.rank, clean(&r.name), url_part(&r.name), r.pp, r.accuracy * 100.0, r.plays))
                    .collect();
                let e = json!({
                    "title": "taiko.rs ranking",
                    "url": format!("{WEBSITE}/rankings"),
                    "description": lines.join("\n"),
                    "color": COLOUR,
                    "footer": { "text": format!("{players} players · {plays} plays") },
                });
                Some((Answer::Embed(e), None))
            }
            _ => None,
        }
    }

    /// Whose plays a command means: a mention, a name, else the asker's own (linked) account.
    fn target(&self, arg: &str, author: &str) -> Result<String, String> {
        let arg = arg.trim();
        if arg.is_empty() {
            return self.links.lock().unwrap().get(author).cloned().ok_or_else(|| "Link your taiko.rs account first: `>link yourname`".to_string());
        }
        if let Some(id) = arg.strip_prefix("<@").and_then(|r| r.strip_suffix('>')).map(|r| r.trim_start_matches('!')) {
            return self.links.lock().unwrap().get(id).cloned().ok_or_else(|| format!("<@{id}> hasn't linked a taiko.rs account."));
        }
        match self.player(arg) {
            Some(_) => Ok(arg.to_ascii_lowercase()),
            None => Err(format!("There's no player called `{}`.", clean(arg))),
        }
    }

    /// A player's name as they wrote it, if they exist.
    fn player(&self, name: &str) -> Option<String> {
        self.state.accounts.lock().unwrap().users.get(&name.trim().to_ascii_lowercase()).map(|u| u.name.clone())
    }

    fn save_links(&self) {
        let text = serde_json::to_string(&*self.links.lock().unwrap()).unwrap_or_default();
        let file = self.dir.join("discord-links.json");
        let tmp = file.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &file)) {
            eprintln!("discord: saving links: {e}");
        }
    }

    /// One play: the map, the numbers, its place on the map, which try it was.
    fn play_embed(&self, key: &str, s: &OnlineScore) -> Value {
        let (place, tries) = {
            let scores = self.state.scores.lock().unwrap();
            let place = (s.id != 0).then(|| scores.place_of(s.map_hash, s.id)).flatten();
            let tries = scores.plays_of(key).iter().filter(|p| p.map_hash == s.map_hash && p.date <= s.date).count();
            (place, tries)
        };
        let mut lines = vec![
            format!("**{}**{} · **{:.2}%** · **{}**", online::grade(s.great, s.ok, s.miss, s.accuracy), mods_text(s), s.accuracy * 100.0, pp_text(s.pp)),
            format!("{} · x{} · [{}/{}/{}]", thousands(s.score), s.max_combo, s.great, s.ok, s.miss),
        ];
        if let Some(p) = place {
            lines.push(format!("🏆 **#{p}** on the map"));
        }
        json!({
            "author": { "name": s.player, "url": format!("{WEBSITE}/u/{}", url_part(&s.player)) },
            "title": s.map,
            "url": format!("{WEBSITE}/m/{}", s.map_hash),
            "description": lines.join("\n"),
            "color": COLOUR,
            "thumbnail": cover_thumb(s.set_id),
            "footer": { "text": format!("Try #{tries} · taiko.rs") },
            "timestamp": iso8601(s.date),
        })
    }

    /// Several plays, numbered from `from` + 1.
    fn plays_embed(&self, title: &str, plays: &[OnlineScore], from: usize, key: &str) -> Value {
        let lines: Vec<String> = plays
            .iter()
            .enumerate()
            .map(|(i, s)| {
                format!(
                    "**{}.** [{}]({WEBSITE}/m/{}){}\n**{}** · {:.2}% · {} · x{} · [{}/{}/{}] · {} ago",
                    from + i + 1,
                    clean(&s.map),
                    s.map_hash,
                    mods_text(s),
                    pp_text(s.pp),
                    s.accuracy * 100.0,
                    thousands(s.score),
                    s.max_combo,
                    s.great,
                    s.ok,
                    s.miss,
                    age(s.date)
                )
            })
            .collect();
        let name = self.state.display_name(key);
        json!({
            "author": { "name": name, "url": format!("{WEBSITE}/u/{}", url_part(&name)) },
            "title": title,
            "description": lines.join("\n"),
            "color": COLOUR,
            "thumbnail": cover_thumb(plays.first().and_then(|s| s.set_id)),
        })
    }

    fn profile_embed(&self, p: &online::Profile) -> Value {
        let st = &p.stats;
        let rank = p.rank.map(|r| format!("#{r}")).unwrap_or_else(|| "-".into());
        let mut lines = vec![
            format!("**Rank:** {rank} · **{:.0}pp** · **{:.2}%**", p.pp, st.accuracy * 100.0),
            format!("**Level:** {:.0} · **Plays:** {}", online::level(st.total_score), p.plays),
            format!("**Ranked score:** {} · **Max combo:** x{}", thousands(st.ranked_score), st.max_combo),
            format!("**Grades:** SS {} · S {} · A {}", st.ss, st.s, st.a),
        ];
        if let Some(best) = p.top.first() {
            lines.push(format!("**Best:** [{}]({WEBSITE}/m/{}) · **{}**", clean(&best.map), best.map_hash, pp_text(best.pp)));
        }
        lines.push(format!("Joined <t:{}:D>", p.joined));
        if p.online {
            if let Some(a) = self.state.activity(&p.name) {
                lines.push(format!("🟢 Online, {}", a.describe()));
            }
        }
        json!({
            "title": format!("taiko.rs profile for {}", p.name),
            "url": format!("{WEBSITE}/u/{}", url_part(&p.name)),
            "description": lines.join("\n"),
            "color": COLOUR,
            "thumbnail": cover_thumb(p.top.first().and_then(|s| s.set_id)),
        })
    }

    /// Posts `answer` in `channel` as a reply to message `to`; returns the new message's id.
    fn reply(&self, channel: &str, to: Option<&str>, answer: &Answer) -> Result<String, String> {
        let mut body = json!({ "allowed_mentions": { "parse": [] } });
        match answer {
            Answer::Text(t) => body["content"] = json!(t),
            Answer::Embed(e) => {
                // Without what it hasn't got (a cover).
                let mut e = e.clone();
                if let Some(o) = e.as_object_mut() {
                    o.retain(|_, v| !v.is_null());
                }
                body["embeds"] = json!([e]);
            }
        }
        if let Some(id) = to {
            body["message_reference"] = json!({ "message_id": id, "fail_if_not_exists": false });
        }
        self.post(&format!("{API}/channels/{channel}/messages"), &body)
    }

    /// A POST to Discord's API (once more after a rate limit); the new object's id.
    fn post(&self, url: &str, body: &Value) -> Result<String, String> {
        for _ in 0..2 {
            let mut reply = self
                .http
                .post(url)
                .header("Authorization", &format!("Bot {}", self.token))
                .header("User-Agent", USER_AGENT)
                .header("Content-Type", "application/json")
                .send(body.to_string())
                .map_err(|e| e.to_string())?;
            let status = reply.status().as_u16();
            let text = reply.body_mut().read_to_string().unwrap_or_default();
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            match status {
                200..=299 => return Ok(v["id"].as_str().unwrap_or("").to_string()),
                429 => std::thread::sleep(Duration::from_secs_f64(v["retry_after"].as_f64().unwrap_or(1.0).min(30.0))),
                _ => return Err(format!("Discord said {status}: {}", text.chars().take(200).collect::<String>())),
            }
        }
        Err("rate limited".into())
    }
}

fn help() -> Value {
    let lines = [
        "`>link name` link your taiko.rs account (`>unlink` to undo)",
        "`>rs` your latest play (`>rs2` the one before, `>rs name`, `>rs @someone`)",
        "`>c` your plays on the last map shown here (or reply to a play with it)",
        "`>top` your best plays by pp (`>top2` next page)",
        "`>osu` your profile",
        "`>lb` the leaderboard of the last map shown here",
        "`>ranking` the top players",
    ];
    json!({
        "title": "taiko.rs bot",
        "url": WEBSITE,
        "description": format!("Plays and profiles from the taiko.rs server.\n\n{}", lines.join("\n")),
        "color": COLOUR,
    })
}

/// A map set's small cover, unless it isn't to be shown (`covers`).
fn cover_thumb(set: Option<u32>) -> Value {
    match set.filter(|&s| crate::covers::allowed(s)) {
        Some(s) => json!({ "url": format!("https://assets.ppy.sh/beatmaps/{s}/covers/list@2x.jpg") }),
        None => Value::Null,
    }
}

fn mods_text(s: &OnlineScore) -> String {
    let rate = if (s.rate - 1.0).abs() > 1e-6 { format!(" {:.2}x", s.rate) } else { String::new() };
    if s.mods.is_empty() { rate } else { format!(" +{}{rate}", s.mods.concat()) }
}

fn pp_text(pp: Option<f64>) -> String {
    pp.map(|p| format!("{p:.0}pp")).unwrap_or_else(|| "no pp".into())
}

/// Text from players kept from turning into Discord markup or mentions.
fn clean(s: &str) -> String {
    s.chars().flat_map(|c| if "*_`~|>[]()@\\".contains(c) { vec!['\\', c] } else { vec![c] }).collect()
}

/// A name in a URL path.
fn url_part(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

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

/// "3d" style age of a Unix time.
fn age(date: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(date);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        86_400..=2_591_999 => format!("{}d", s / 86_400),
        _ => format!("{}mo", s / 2_592_000),
    }
}

/// A Unix time as Discord's timestamps want it (ISO 8601, UTC).
fn iso8601(t: u64) -> String {
    let days = (t / 86_400) as i64;
    let secs = t % 86_400;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

// ---------------------------------------------------------------------------------------------
// The changelog: the game's commits, posted as they're pushed.

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Posted {
    posted: Vec<String>,
}

/// Whether a commit goes in the changelog: not merges, not what's marked `Changelog: skip`, not
/// a player's own secret.
pub(crate) fn worth_posting(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let title = message.lines().next().unwrap_or("").trim();
    !title.is_empty() && !title.starts_with("Merge ") && !lower.lines().any(|l| l.trim() == "changelog: skip") && !lower.contains("krokodil")
}

/// A commit's message as a changelog post: its first line, then its points, without trailers.
pub(crate) fn post_text(message: &str) -> (String, String) {
    let mut lines = message.lines();
    let title = lines.next().unwrap_or("").trim().chars().take(256).collect();
    let body: Vec<&str> = lines
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            !["co-authored-by:", "claude-session:", "signed-off-by:", "changelog:"].iter().any(|t| l.starts_with(t))
        })
        .collect();
    let body = body.join("\n").trim().chars().take(3500).collect();
    (title, body)
}

fn changelog(bot: Arc<Bot>) {
    let Some(channel) = std::fs::read_to_string(bot.dir.join("discord-changelog")).ok().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) else {
        println!("discord: no changelog channel (discord-changelog)");
        return;
    };
    let file = bot.dir.join("discord-changelog.json");
    let mut posted: Posted = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut etag = String::new();
    loop {
        match new_commits(&bot, &posted, &mut etag) {
            Ok(commits) => {
                // A first start: what's there already isn't news.
                let first = posted.posted.is_empty();
                for c in commits {
                    let message = c["commit"]["message"].as_str().unwrap_or("");
                    if !first && worth_posting(message) {
                        let (title, body) = post_text(message);
                        let login = c["author"]["login"].as_str();
                        let author = login.or(c["commit"]["author"]["name"].as_str()).unwrap_or("?");
                        let mut embed = json!({
                            "title": title,
                            "url": c["html_url"],
                            "description": body,
                            "color": COLOUR,
                            "author": { "name": author, "url": format!("https://github.com/{}", login.unwrap_or("")), "icon_url": c["author"]["avatar_url"] },
                            "timestamp": c["commit"]["author"]["date"],
                            "footer": { "text": "taiko.rs · stable" },
                        });
                        if login.is_none() {
                            embed["author"] = json!({ "name": author });
                        }
                        if let Err(e) = bot.post(&format!("{API}/channels/{channel}/messages"), &json!({ "embeds": [embed], "allowed_mentions": { "parse": [] } })) {
                            eprintln!("discord: changelog: {e}");
                            break;
                        }
                    }
                    posted.posted.push(c["sha"].as_str().unwrap_or("").to_string());
                    // Plenty to know what's new; it doesn't grow forever.
                    if posted.posted.len() > 500 {
                        posted.posted.drain(..posted.posted.len() - 500);
                    }
                    let tmp = file.with_extension("json.tmp");
                    if let Err(e) = std::fs::write(&tmp, serde_json::to_string(&posted).unwrap_or_default()).and_then(|_| std::fs::rename(&tmp, &file)) {
                        eprintln!("discord: saving the changelog: {e}");
                    }
                }
            }
            Err(e) => eprintln!("discord: changelog: {e}"),
        }
        std::thread::sleep(Duration::from_secs(120));
    }
}

/// The branch's commits not posted yet, oldest first (none when GitHub says nothing changed).
fn new_commits(bot: &Bot, posted: &Posted, etag: &mut String) -> Result<Vec<Value>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/commits?sha={BRANCH}&per_page=30");
    let mut req = bot.http.get(&url).header("User-Agent", "taiko.rs-server (+https://rstaiko.auneu.space)").header("Accept", "application/vnd.github+json");
    if !etag.is_empty() {
        req = req.header("If-None-Match", etag.as_str());
    }
    let mut reply = req.call().map_err(|e| e.to_string())?;
    let status = reply.status().as_u16();
    if status == 304 {
        return Ok(Vec::new());
    }
    if status != 200 {
        return Err(format!("GitHub said {status} for {url}"));
    }
    if let Some(tag) = reply.headers().get("etag").and_then(|v| v.to_str().ok()) {
        *etag = tag.to_string();
    }
    let commits: Vec<Value> = serde_json::from_str(&reply.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let seen: HashSet<&str> = posted.posted.iter().map(String::as_str).collect();
    // Newest first from GitHub; up to the first one already posted, then oldest first.
    let mut fresh: Vec<Value> = commits.into_iter().take_while(|c| !seen.contains(c["sha"].as_str().unwrap_or(""))).collect();
    fresh.reverse();
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_with_their_numbers() {
        assert_eq!(parse(">rs"), Some(Command { word: "rs".into(), n: None, rest: String::new() }));
        assert_eq!(parse(">RS2 Lynerrr"), Some(Command { word: "rs".into(), n: Some(2), rest: "Lynerrr".into() }));
        assert_eq!(parse("  >top3  <@123>  "), Some(Command { word: "top".into(), n: Some(3), rest: "<@123>".into() }));
        assert_eq!(parse("rs"), None);
        assert_eq!(parse(">"), None);
        assert_eq!(parse(">123"), None);
    }

    #[test]
    fn the_changelog_leaves_out_merges_skips_and_secrets() {
        assert!(worth_posting("New mod, Crossfire\n\n- In the RAILS column\n\nCo-Authored-By: x"));
        assert!(!worth_posting("Merge pull request #53 from x/y"));
        assert!(!worth_posting("Server: no cap\n\nChangelog: skip\nCo-Authored-By: x"));
        assert!(!worth_posting("krokodil_koban mode says nothing about itself"));
        let (title, body) = post_text("Title here\n\n- one\n- two\n\nCo-Authored-By: C <c@x>\nClaude-Session: https://x\n");
        assert_eq!(title, "Title here");
        assert_eq!(body, "- one\n- two");
    }

    #[test]
    fn helpers_format_like_discord_wants() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_792_990_923), "2026-10-26T05:02:03Z");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(12), "12");
        assert_eq!(clean("a_b*c @everyone"), "a\\_b\\*c \\@everyone");
        assert_eq!(url_part("A b"), "A%20b");
    }

    /// A bot over a fresh server with two players, one with a play.
    fn bot() -> Arc<Bot> {
        let dir = std::env::temp_dir().join(format!("taiko-server-discord-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t").len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = Arc::new(State::load(dir.join("accounts.json")));
        {
            let mut a = state.accounts.lock().unwrap();
            for name in ["Ryuji", "Lynerrr"] {
                a.users.insert(name.to_ascii_lowercase(), crate::User { name: name.into(), hash: String::new(), created: 1_700_000_000, friends: Vec::new(), requests: Vec::new() });
            }
        }
        let play = online::ScoreSubmit {
            map: crate::net::MapInfo { hash: 7, artist: "A".into(), title: "T".into(), version: "Oni".into(), rate: 1.0, from: None, set_id: None, mods: vec!["HR".into()] },
            score: 1_000_000,
            accuracy: 0.98,
            max_combo: 500,
            great: 480,
            ok: 20,
            miss: 0,
            pp: Some(321.0),
            date: 1_792_990_923,
            replay_version: 2,
            inputs: vec![(1000.0, 1)],
            stars: Some(6.0),
            scroll_speed: Some(1.0),
        };
        state.scores.lock().unwrap().submit("ryuji", "Ryuji", play, 1_792_990_923).unwrap();
        let http: ureq::Agent = ureq::Agent::config_builder().build().into();
        Arc::new(Bot { token: String::new(), dir, state, links: Mutex::default(), last_map: Mutex::default(), shown: Mutex::default(), http })
    }

    fn answer(bot: &Bot, text: &str, author: &str, map: Option<u64>) -> (Answer, Option<u64>) {
        bot.respond(&parse(text).unwrap(), author, map).unwrap()
    }

    fn says(a: &Answer) -> String {
        match a {
            Answer::Text(t) => t.clone(),
            Answer::Embed(e) => e.to_string(),
        }
    }

    #[test]
    fn linking_and_whose_plays_a_command_means() {
        let b = bot();
        assert_eq!(says(&answer(&b, ">rs", "1", None).0), "Link your taiko.rs account first: `>link yourname`");
        assert_eq!(says(&answer(&b, ">link nobody", "1", None).0), "There's no player called `nobody`.");
        assert!(says(&answer(&b, ">link ryuji", "1", None).0).starts_with("Linked to **Ryuji**"));
        assert_eq!(std::fs::read_to_string(b.dir.join("discord-links.json")).unwrap(), r#"{"1":"ryuji"}"#);
        // Their latest play, with its place, and the map it shows.
        let (a, shows) = answer(&b, ">rs", "1", None);
        let e = says(&a);
        assert!(e.contains("A - T [Oni]") && e.contains("321pp") && e.contains("+HR") && e.contains("#1** on the map") && e.contains("Try #1"), "{e}");
        assert_eq!(shows, Some(7));
        assert_eq!(says(&answer(&b, ">rs2", "1", None).0), "`Ryuji` has only 1 recent plays.");
        assert_eq!(says(&answer(&b, ">rs lynerrr", "1", None).0), "`Lynerrr` hasn't sent a play yet.");
        assert_eq!(says(&answer(&b, ">rs <@2>", "1", None).0), "<@2> hasn't linked a taiko.rs account.");
        // The map shown: `>c` and `>lb`.
        assert!(says(&answer(&b, ">c", "1", Some(7)).0).contains("Ryuji on A - T [Oni]"));
        assert_eq!(says(&answer(&b, ">c lynerrr", "1", Some(7)).0), "`Lynerrr` hasn't played A - T [Oni].");
        assert!(says(&answer(&b, ">lb", "1", Some(7)).0).contains("1 plays on this map · page 1/1"));
        assert_eq!(says(&answer(&b, ">lb", "1", Some(8)).0), "Nobody has played that map here.");
        assert!(says(&answer(&b, ">top", "1", None).0).contains("Best plays of Ryuji"));
        let profile = says(&answer(&b, ">osu", "1", None).0);
        assert!(profile.contains("taiko.rs profile for Ryuji") && profile.contains("**Rank:** #1"), "{profile}");
        assert!(says(&answer(&b, ">ranking", "1", None).0).contains("taiko.rs ranking"));
        assert!(says(&answer(&b, ">help", "1", None).0).contains(">link name"));
        assert!(b.respond(&parse(">nope").unwrap(), "1", None).is_none(), "not the bot's");
        assert_eq!(says(&answer(&b, ">unlink", "1", None).0), "Unlinked.");
        assert_eq!(says(&answer(&b, ">unlink", "1", None).0), "You weren't linked to anyone.");
        let _ = std::fs::remove_dir_all(&b.dir);
    }
}
