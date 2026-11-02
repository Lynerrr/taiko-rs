//! The online server: accounts (used by multiplayer for now) and lobbies hosted on the server,
//! so nobody needs an open port. This is the protocol shared by the game and `server/`, plus
//! the game's side of it. A connection sends one request line and reads one reply line; after
//! `Reply::Joined` the same connection speaks the room protocol (`net.rs`). Direct play
//! (`net::host` / `net::Client::connect`) stays as the fallback when the server is down.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 1;
/// Most maps one `Request::SetStatuses` is answered for.
pub const MAX_STATUS_MAPS: usize = 500;
pub const DEFAULT_PORT: u16 = 7800;
pub const DEFAULT_SERVER: &str = "jashincraft.space:7800";
/// The website (profiles at /u/<name>).
pub const WEBSITE: &str = "https://rstaiko.auneu.space";
/// Longest request or reply line (a lobby list of a few hundred rooms fits easily).
const MAX_LINE: u64 = 1 << 20;

#[derive(Serialize, Deserialize, Debug)]
pub struct Envelope {
    pub protocol: u32,
    pub request: Request,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
    /// A new account. Where the server sends mail, it takes an `email`: asked without `code`, it
    /// mails one there (`Reply::CodeSent`); asked again with it, the account is made.
    Register {
        name: String,
        password: String,
        #[serde(default)]
        email: Option<String>,
        #[serde(default)]
        code: Option<String>,
    },
    Login { name: String, password: String },
    /// A forgotten password: `who` is the name or the email. Without `code` one is mailed to the
    /// account's email (`Reply::CodeSent`); with it and the new `password`, the password is
    /// changed, every other session logged out, and this one logged in (`Reply::Account`).
    ResetPassword {
        who: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    /// Links `email` to an account without one (those from before emails; one that has an email
    /// keeps it): without `code` one is mailed there (`Reply::CodeSent`), with it the email is linked (`Reply::EmailLinked`).
    LinkEmail {
        token: String,
        email: String,
        #[serde(default)]
        code: Option<String>,
    },
    /// Checks a saved session token.
    WhoAmI { token: String },
    Logout { token: String },
    ListRooms,
    /// Opens a lobby on the server and joins it as its host; `mapguessr`: one that plays
    /// mapguessr.
    CreateRoom {
        token: String,
        title: String,
        #[serde(default)]
        mapguessr: bool,
    },
    JoinRoom { token: String, room: u32 },
    /// The social connection (friends, chat, watching): after `Reply::Account` the connection
    /// stays open, the game sending `Up` lines and the server `Down` lines.
    Connect { token: String },
    /// A kept play for the online leaderboards (`Reply::Submitted`).
    SubmitScore { token: String, play: Box<ScoreSubmit> },
    /// The best score of each player on a map, by its hash (`Reply::Scores`).
    MapScores { map: u64 },
    /// A player's profile (`Reply::Profile`).
    Profile { name: String },
    /// The presses of an online score, by its `OnlineScore::id` (`Reply::Replay`).
    Replay { id: u64 },
    /// One online score, by its `OnlineScore::id` (`Reply::Score`): what a score link opens.
    Score { id: u64 },
    /// The statuses on osu! of the sets the maps are of, each as (set ID, the ID of one of its
    /// maps): what the server knows now (`Reply::SetStatuses`), and it looks the rest up for
    /// the next time (when `token` is a logged-in player's).
    SetStatuses { token: String, maps: Vec<(u32, u32)> },
    /// The map sets loved on the server (`Reply::LovedSets`): they give pp there.
    LovedSets,
    /// A loved nominator loves a map set on the server, or takes it back (`Reply::LovedSets`).
    SetLoved { token: String, set: u32, loved: bool },
    /// A play that isn't on the leaderboards (failed, quit, restarted, from a start point, in
    /// a lobby): it counts in the profile's play count, and `ms` of playing in its play time
    /// (`Reply::Counted`).
    Played { token: String, ms: u64 },
    /// Every medal, and how `name` stands with them (`Reply::Medals`); no name: just the list.
    Medals { name: String },
}

/// What a replay's presses can't say about a play's timing: with it, how far off a play was can
/// be told from where it asked for the offset to be. Kept with the replay and shown to no one.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct PlayTiming {
    pub os: String,
    pub present: String,
    /// The offset in effect (the global one and the map's).
    pub offset_ms: f64,
    /// How far ahead the notes were drawn: the visual offset and any vsync compensation.
    pub visual_lead_ms: f64,
    /// The output latency the clock took off, and what the stream itself said it was.
    pub latency_ms: f64,
    pub reported_latency_ms: f64,
}

/// A play sent to the server: what the leaderboards show, and the replay's presses (kept so
/// plays can be watched and checked later).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ScoreSubmit {
    /// The map (its hash, names and set), with the rate and mods played.
    pub map: crate::net::MapInfo,
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub pp: Option<f64>,
    /// Unix seconds.
    pub date: u64,
    pub replay_version: u32,
    pub inputs: Vec<(f64, u8)>,
    /// Star rating at the rate played.
    #[serde(default)]
    pub stars: Option<f64>,
    /// Real milliseconds spent playing it (pauses left out), for the profile's play time.
    #[serde(default)]
    pub play_ms: Option<u64>,
    /// The scroll speed setting played at (pp count it).
    #[serde(default)]
    pub scroll_speed: Option<f64>,
    /// MD5 of the `.osu` played: a difficulty edited from a ranked one (OD changed, a rate
    /// baked in) isn't osu!'s and gives no pp.
    #[serde(default)]
    pub md5: Option<String>,
    /// How the game worked out `pp` and `stars` (`PP_VERSION`; 0: before it said).
    #[serde(default)]
    pub pp_version: u32,
    /// How the game's timing was set up (none from older games).
    #[serde(default)]
    pub timing: Option<PlayTiming>,
    /// Drumroll hits (none from older games).
    #[serde(default)]
    pub rolls: Option<u32>,
}

/// 1: on the notes `NOTE_MODS` leave, not the whole map. 2: Half keeps to the beat.
pub const PP_VERSION: u32 = 2;
/// The mods that change the notes: older games' pp of plays with them are wrong.
#[allow(dead_code)] // used by the online server (server/)
pub const NOTE_MODS: [&str; 3] = ["HA", "SW", "RD"];

/// A score on a leaderboard or a profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct OnlineScore {
    pub player: String,
    /// "Artist - Title [Version]".
    pub map: String,
    pub map_hash: u64,
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub miss: u32,
    pub rate: f64,
    pub mods: Vec<String>,
    pub pp: Option<f64>,
    pub date: u64,
    #[serde(default)]
    pub great: u32,
    #[serde(default)]
    pub ok: u32,
    /// The map's osu! set (for its cover), and its star rating at the rate played.
    #[serde(default)]
    pub set_id: Option<u32>,
    #[serde(default)]
    pub stars: Option<f64>,
    /// The server's number for the play, to ask for its replay by (0 from servers before
    /// replays).
    #[serde(default)]
    pub id: u64,
    /// The scroll speed setting it was played at (None: sent before that was).
    #[serde(default)]
    pub scroll_speed: Option<f64>,
    /// `ScoreSubmit::pp_version`.
    #[serde(default)]
    pub pp_version: u32,
    /// `ScoreSubmit::rolls`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rolls: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// An online score's replay: its presses, the map they were played on and how they are judged
/// (`scores::Replay::version`).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OnlineReplay {
    pub version: u32,
    pub map: crate::net::MapInfo,
    pub inputs: Vec<(f64, u8)>,
}

impl OnlineScore {
    #[allow(dead_code)] // used by the online server (server/)
    pub fn grade(&self) -> &'static str {
        grade(self.great, self.ok, self.miss, self.accuracy)
    }
}

/// A play's letter, as the results screen gives it.
pub fn grade(great: u32, ok: u32, miss: u32, accuracy: f64) -> &'static str {
    let acc = accuracy * 100.0;
    match () {
        _ if miss == 0 && ok == 0 && great > 0 => "SS",
        _ if acc >= 95.0 && miss == 0 => "S",
        _ if acc >= 90.0 => "A",
        _ if acc >= 80.0 => "B",
        _ if acc >= 70.0 => "C",
        _ => "D",
    }
}

/// A player as the server sees them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Profile {
    pub name: String,
    /// Unix seconds.
    pub joined: u64,
    /// Plays (sent to the leaderboards, and the others: failed, quit...), and their pp total
    /// (best per map, weighted like osu!).
    pub plays: u32,
    pub pp: f64,
    /// Place among everyone by pp (1-based; None without pp yet).
    pub rank: Option<u32>,
    /// Their best plays by pp, and the latest ones.
    pub top: Vec<OnlineScore>,
    pub recent: Vec<OnlineScore>,
    #[serde(default)]
    pub stats: Stats,
    /// Maps where theirs is the best score.
    #[serde(default)]
    pub firsts: Vec<OnlineScore>,
    #[serde(default)]
    pub online: bool,
    /// Time spent playing (real milliseconds, pauses left out), counted since this was added.
    #[serde(default)]
    pub play_ms: u64,
    #[serde(default)]
    pub medals: MedalSummary,
}

/// A player's totals, like osu!'s profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Stats {
    /// Their best score on each map, added up; and every score.
    pub ranked_score: u64,
    pub total_score: u64,
    /// Accuracy of their best plays, weighted like pp.
    pub accuracy: f64,
    pub total_hits: u64,
    pub max_combo: u32,
    /// Best plays per map graded SS, S and A.
    pub ss: u32,
    pub s: u32,
    pub a: u32,
}

/// osu!'s level for a total score, with the part of the way to the next.
#[allow(dead_code)] // used by the online server (server/)
pub fn level(total_score: u64) -> f64 {
    let needed = |n: f64| -> f64 {
        if n <= 100.0 {
            5000.0 / 3.0 * (4.0 * n.powi(3) - 3.0 * n.powi(2) - n) + 1.25 * 1.8f64.powf(n - 60.0)
        } else {
            26_931_190_827.0 + 99_999_999_999.0 * (n - 100.0)
        }
    };
    let score = total_score as f64;
    let mut n = 1.0;
    while needed(n + 1.0) <= score && n < 200.0 {
        n += 1.0;
    }
    let (lo, hi) = (needed(n), needed(n + 1.0));
    n + ((score - lo) / (hi - lo)).clamp(0.0, 0.999)
}

/// Leaderboard rows a map's scores reply holds at most.
#[allow(dead_code)] // used by the online server (server/)
pub const MAP_SCORES: usize = 50;

/// A player's pp total from their pp on each map (best per map): highest first, each counting
/// 5% less than the one before, like osu!.
#[allow(dead_code)] // used by the online server (server/)
pub fn weighted_pp(mut best: Vec<f64>) -> f64 {
    best.sort_by(|a, b| b.total_cmp(a));
    best.iter().enumerate().map(|(i, pp)| pp * 0.95f64.powi(i as i32)).sum()
}

/// What a player is doing, for their friends.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Activity {
    Menus,
    /// A solo play (friends can watch it).
    Playing { map: String },
    Lobby { room: Option<u32>, title: String },
    Match { map: String },
    Watching { name: String },
}

impl Activity {
    pub fn describe(&self) -> String {
        match self {
            Activity::Menus => tr!("in the menus", "メニュー中", "в меню").into(),
            Activity::Playing { map } => tr!("playing {map}", "プレイ中: {map}", "играет: {map}";),
            Activity::Lobby { title, .. } => tr!("in lobby {title}", "ロビー: {title}", "в лобби {title}";),
            Activity::Match { map } => tr!("in a match: {map}", "対戦中: {map}", "в матче: {map}";),
            Activity::Watching { name } => tr!("watching {name}", "{name}を観戦中", "смотрит {name}";),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Friend {
    pub name: String,
    /// None when offline.
    pub activity: Option<Activity>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChatLine {
    pub from: String,
    pub to: String,
    pub text: String,
    /// Unix seconds.
    pub at: u64,
    /// A medal the server is announcing (`SERVER_NAME` says it): `text` is the same in English,
    /// for games from before medals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medal: Option<ChatMedal>,
}

/// Who the server's own lines in the chat are from: a dot can't be in an account's name.
pub const SERVER_NAME: &str = "taiko.rs";

/// Text in each of the game's languages: English, Japanese, Russian.
pub type Said = [String; 3];

/// The side of `text` for the language the game is in.
#[allow(dead_code)] // used by the game
pub fn said(text: &Said) -> &str {
    match crate::lang::current() {
        crate::lang::Lang::En => &text[0],
        crate::lang::Lang::Ja => &text[1],
        crate::lang::Lang::Ru => &text[2],
    }
}

/// Medal tiers, named after taiko's difficulties (the colours are the website's).
pub const TIER_COLOURS: [u32; 4] = [0x86E05A, 0x4fb4ff, 0xFFD34D, 0xEB452C];
/// A secret medal's colour, whatever its tier: Ura, taiko's hidden difficulty.
pub const SECRET_COLOUR: u32 = 0xc77dff;

/// A medal's colour: `tier` is 1 to 4.
pub fn medal_colour(tier: u8, secret: bool) -> u32 {
    if secret {
        SECRET_COLOUR
    } else {
        TIER_COLOURS[usize::from(tier.clamp(1, 4)) - 1]
    }
}

/// A medal someone has just earned, as the game shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MedalNote {
    pub id: String,
    pub name: Said,
    /// How it was earned; a secret's stays a hint.
    pub about: Said,
    pub tier: u8,
    pub secret: bool,
    /// Its picture, for `medal_badge`.
    pub art: Vec<String>,
}

/// The server's line about a medal, for games to say it in their own language.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ChatMedal {
    pub player: String,
    pub name: Said,
    pub tier: u8,
    pub secret: bool,
    /// Nobody had it before them.
    pub first: bool,
    /// How many players have it now, of how many.
    pub holders: u32,
    pub players: u32,
}

impl ChatLine {
    /// What the line says, in the game's language.
    #[allow(dead_code)] // used by the game
    pub fn shown(&self) -> String {
        let Some(m) = &self.medal else { return self.text.clone() };
        let (player, name, holders, players) = (&m.player, said(&m.name), m.holders, m.players);
        let line = match m.secret {
            true => tr!("{player} found the secret {name}", "{player}が秘密のメダル「{name}」を見つけた", "{player} находит секрет «{name}»";),
            false => tr!("{player} earned {name}", "{player}がメダル「{name}」を獲得", "{player} получает «{name}»";),
        };
        let rarity = match m.first {
            true => tr!("first on the server", "サーバー初", "первым на сервере").to_string(),
            false => tr!("{holders} of {players} have it", "{players}人中{holders}人", "есть у {holders} из {players}";),
        };
        format!("{line} · {rarity}")
    }
}

/// How one medal stands.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MedalRow {
    pub id: String,
    /// The section it's in, the same order as the game's and the website's lists.
    pub section: u8,
    pub tier: u8,
    pub secret: bool,
    /// None: a secret nobody has found yet.
    pub name: Option<Said>,
    /// How to earn it; None for secrets.
    pub how: Option<Said>,
    /// A secret's hint, once somebody has found it.
    pub hint: Option<Said>,
    pub art: Vec<String>,
    /// When the player earned it (Unix seconds).
    pub at: Option<u64>,
    /// How many players have it.
    pub holders: u32,
    /// How far along the player is, where that can be counted (`need` 0: it can't).
    pub have: u64,
    pub need: u64,
}

/// `Reply::Medals`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MedalsView {
    /// Players with a play on the server.
    pub players: u32,
    /// The player's medals; 0 without a player.
    pub earned: u32,
    pub medals: Vec<MedalRow>,
    /// The mods in their collection, by acronym.
    pub collection: Vec<String>,
    /// The mods a collection can have.
    pub collection_size: u32,
}

/// What a profile says of a player's medals.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MedalSummary {
    pub earned: u32,
    pub total: u32,
    /// Their highest dan.
    pub dan: Option<Said>,
    /// Their rarest medals.
    pub rarest: Vec<MedalNote>,
}

/// A medal picture is this many pixels square, and a badge this many.
pub const ART: usize = 16;
pub const BADGE: usize = 28;

/// A medal's badge as colours (`None`: nothing there), the same in the game and on the website:
/// a ring in its tier's colour, more ornate the higher the tier, around its picture, whose
/// pixels are `.` nothing, `#` white, `x` the medal's colour and `o` a dark shade of it. `lit`:
/// earned; otherwise it's greyed out.
pub fn medal_badge(art: &[String], tier: u8, secret: bool, lit: bool) -> [[Option<u32>; BADGE]; BADGE] {
    let colour = medal_colour(tier, secret);
    let mix = |a: u32, b: u32, t: f32| {
        let part = |shift: u32| {
            let (x, y) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
            ((x + (y - x) * t) as u32) << shift
        };
        part(16) | part(8) | part(0)
    };
    let shade = |c: u32| if lit { c } else { mix(mix(c, 0x8a8aa0, 0.85), 0x000000, 0.45) };
    let points = [0.0, 4.0, 8.0, 12.0][usize::from(tier.clamp(1, 4)) - 1];
    let mut out = [[None; BADGE]; BADGE];
    let mid = (BADGE as f32 - 1.0) / 2.0;
    for (y, row) in out.iter_mut().enumerate() {
        for (x, px) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f32 - mid, y as f32 - mid);
            let d = dx.hypot(dy);
            // Each tier up adds points around the ring.
            let turn = (dy.atan2(dx) + std::f32::consts::FRAC_PI_4) / std::f32::consts::TAU;
            let spike = points > 0.0 && d <= 13.9 && ((turn * points).rem_euclid(1.0) - 0.5).abs() < 0.11;
            *px = if d <= 10.6 {
                Some(0x15151d)
            } else if d <= 12.4 {
                Some(shade(if dx + dy < 0.0 { mix(colour, 0xffffff, 0.35) } else { colour }))
            } else if d <= 13.4 || spike {
                Some(shade(mix(colour, 0x000000, 0.45)))
            } else {
                None
            };
        }
    }
    let inset = (BADGE - ART) / 2;
    for (y, row) in art.iter().take(ART).enumerate() {
        for (x, ink) in row.chars().take(ART).enumerate() {
            let c = match ink {
                '#' => 0xffffff,
                'x' => colour,
                'o' => mix(colour, 0x000000, 0.55),
                _ => continue,
            };
            out[y + inset][x + inset] = Some(shade(c));
        }
    }
    out
}

/// Someone connected to the server right now.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OnlinePlayer {
    pub name: String,
    pub activity: Activity,
}

/// The most players `Down::Players` lists (its count is everyone).
pub const MAX_LISTED: usize = 300;

/// A solo play being streamed to friends.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SoloPlay {
    pub map: crate::net::MapInfo,
    /// Older builds' mash penalty setting; always sent on (judging is `Mash::Nearest` now).
    pub anti_mash: bool,
}

/// Game -> server on the social connection.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Up {
    Status(Activity),
    /// Sends a friend request, or accepts theirs.
    AddFriend(String),
    /// Unfriends, or declines / withdraws a request.
    RemoveFriend(String),
    Chat { to: String, text: String },
    Watch(String),
    StopWatching,
    PlayStart(SoloPlay),
    /// Every ~100 ms of a solo play: its map time, whether it's paused, presses since the last.
    PlayFrame { t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    PlayEnd,
    Ping,
    /// A line in the global chat (everyone online sees it).
    GlobalChat(String),
}

/// Server -> game on the social connection.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Down {
    /// Friends (with what they're doing), requests to us, and ours still waiting.
    Friends { friends: Vec<Friend>, requests: Vec<String>, sent: Vec<String> },
    Chat(ChatLine),
    Notice(String),
    /// The watched friend is playing: the play, where they are and every press so far.
    WatchStart { player: String, play: SoloPlay, t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    WatchFrame { t: f64, paused: bool, inputs: Vec<(f64, u8)> },
    /// The watched friend's play ended (we keep watching for their next one).
    WatchEnd { player: String },
    /// Watching isn't possible (any more): why.
    WatchStopped(String),
    Pong,
    /// Everyone online (sorted by name, at most `MAX_LISTED`) and how many that is. Sent on
    /// connecting and then whenever it changes (at most every couple of seconds, less often
    /// with thousands online).
    Players { count: u32, players: Vec<OnlinePlayer> },
    /// A global chat line (the recent ones again on connecting).
    /// Medals earned away from a play.
    Medals(Vec<MedalNote>),
    GlobalChat(ChatLine),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Reply {
    /// Logged in (or the token checked out): the account's name and a session token, and
    /// whether it may love maps (a loved nominator or an admin).
    Account {
        name: String,
        token: String,
        #[serde(default)]
        nominator: bool,
        /// Whether the account has an email (None: a server from before emails).
        #[serde(default)]
        email: Option<bool>,
    },
    /// A code was mailed: where to ("g*****@gmail.com").
    CodeSent(String),
    /// The email is linked to the account.
    EmailLinked(String),
    /// The map sets loved on the server.
    LovedSets(Vec<u32>),
    /// (set ID, status on osu!) of the sets asked about that the server knows ("ranked",
    /// "loved", "graveyard"..., "missing" when osu! doesn't have it).
    SetStatuses(Vec<(u32, String)>),
    /// A `Played` play was counted.
    Counted,
    LoggedOut,
    Rooms(Vec<RoomInfo>),
    /// In the room: from here on this connection speaks the room protocol. Send Hello with
    /// `host_token` (set when we created the room).
    Joined { room: u32, title: String, host_token: Option<u64> },
    Error(String),
    /// The play is on the leaderboards: its place on the map (if it's the player's best there),
    /// and the player's pp total and rank now.
    Submitted {
        map_rank: Option<u32>,
        pp: f64,
        rank: Option<u32>,
        /// The medals this play earned.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        medals: Vec<MedalNote>,
    },
    Medals(Box<MedalsView>),
    Scores(Vec<OnlineScore>),
    Profile(Box<Profile>),
    Replay(Box<OnlineReplay>),
    Score(Box<OnlineScore>),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RoomInfo {
    pub id: u32,
    pub title: String,
    pub host: String,
    pub players: u32,
    pub map: Option<String>,
    pub in_match: bool,
    #[serde(default)]
    pub mapguessr: bool,
}

pub const NAME_RULES: &str = "3-16 letters, digits, _ or -";

pub fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// What logging in takes: an account's name, or its email.
pub fn valid_login(who: &str) -> bool {
    valid_name(who) || (who.contains('@') && who.len() <= 254)
}

pub fn valid_password(password: &str) -> bool {
    (6..=128).contains(&password.chars().count())
}

/// Text the built-in font draws (printable ASCII, Cyrillic and Japanese), at most `max`
/// characters.
pub fn clean_text(text: &str, max: usize) -> String {
    text.chars().filter(|&c| c.is_ascii_graphic() || c == ' ' || (!c.is_control() && !c.is_ascii() && crate::font::drawable(c))).take(max).collect()
}

/// A lobby title, at most 32 characters.
#[allow(dead_code)] // used by the online server (server/)
/// `mapguessr`: the default title says it's a mapguessr lobby.
pub fn clean_title(title: &str, owner: &str, mapguessr: bool) -> String {
    let t = clean_text(title, 32);
    let t = t.trim();
    if !t.is_empty() {
        t.to_string()
    } else if mapguessr {
        format!("{owner}'s mapguessr")
    } else {
        format!("{owner}'s lobby")
    }
}

/// One line of JSON (without the newline), at most `MAX_LINE` bytes.
pub fn read_line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let n = reader.take(MAX_LINE).read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed".into());
    }
    if !line.ends_with('\n') {
        return Err("message too large".into());
    }
    Ok(line)
}

pub fn write_line<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), String> {
    let mut line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    line.push('\n');
    w.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

fn connect(server: &str) -> Result<TcpStream, String> {
    let server = server.trim();
    let has_port = server.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok());
    let host = if has_port { server.rsplit_once(':').map_or(server, |(h, _)| h) } else { server };
    // The real server only ever encrypted, whatever port an older config has for it.
    let secure = host.eq_ignore_ascii_case(crate::tls::SERVER_HOST);
    let with_port = if secure {
        format!("{host}:{}", crate::tls::PORT)
    } else if has_port {
        server.to_string()
    } else {
        format!("{server}:{DEFAULT_PORT}")
    };
    let addrs: Vec<_> = with_port.to_socket_addrs().map_err(|e| tr!("can't find the server: {e}", "サーバーが見つかりません: {e}", "сервер не найден: {e}";))?.collect();
    let mut last = String::from(tr!("no address", "アドレスなし", "нет адреса"));
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(4)) {
            Ok(s) => {
                let s = if secure { crate::tls::connect(s).map_err(|e| tr!("couldn't connect securely ({e})", "安全に接続できません ({e})", "не удалось безопасно подключиться ({e})";))? } else { s };
                let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
                return Ok(s);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(tr!("the server is offline ({last})", "サーバーはオフラインです ({last})", "сервер недоступен ({last})";))
}

/// Sends one request; returns the reply and the connection (still open).
pub fn exchange(server: &str, request: Request) -> Result<(Reply, BufReader<TcpStream>), String> {
    let mut stream = connect(server)?;
    write_line(&mut stream, &Envelope { protocol: PROTOCOL, request })?;
    let mut reader = BufReader::new(stream);
    let line = read_line(&mut reader)?;
    let reply: Reply = serde_json::from_str(&line).map_err(|e| tr!("odd reply from the server: {e}", "サーバーからの応答がおかしい: {e}", "странный ответ сервера: {e}";))?;
    Ok((reply, reader))
}

/// What the social connection reports to the game.
#[allow(dead_code)] // the game's side (the server only shares the protocol)
pub enum SocialEvent {
    /// (Re)connected as this account: resend what we're doing.
    Connected(String),
    Disconnected(String),
    /// The saved login is no longer valid.
    SessionExpired,
    Down(Box<Down>),
}

/// The game's social connection (friends, chat, watching). It reconnects by itself while the
/// server is away; dropping it disconnects.
#[allow(dead_code)]
pub struct Social {
    tx: std::sync::mpsc::Sender<Up>,
    rx: std::sync::mpsc::Receiver<SocialEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[allow(dead_code)]
impl Social {
    pub fn start(server: String, token: String) -> Social {
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;
        let (tx, out) = mpsc::channel::<Up>();
        let (events, rx) = mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let mut announced_down = false;
            while !stopped.load(Ordering::Relaxed) {
                let (reply, reader) = match exchange(&server, Request::Connect { token: token.clone() }) {
                    Ok(r) => r,
                    Err(e) => {
                        if !announced_down {
                            announced_down = true;
                            let _ = events.send(SocialEvent::Disconnected(e));
                        }
                        for _ in 0..20 {
                            if stopped.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(500));
                        }
                        continue;
                    }
                };
                let name = match reply {
                    Reply::Account { name, .. } => name,
                    Reply::Error(e) if e.contains("session") => {
                        let _ = events.send(SocialEvent::SessionExpired);
                        return;
                    }
                    _ => {
                        std::thread::sleep(Duration::from_secs(10));
                        continue;
                    }
                };
                // Socket timeouts are per handle on Windows (shared on Linux): the reader's own
                // handle must lose the 10 s request timeout, or a quiet stretch drops the link.
                let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(70)));
                let _ = reader.get_ref().set_write_timeout(None);
                let Ok(mut stream) = reader.get_ref().try_clone() else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(70)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
                // Whatever piled up while away is stale: the game resends its state on Connected.
                while out.try_recv().is_ok() {}
                let _ = events.send(SocialEvent::Connected(name));
                let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                {
                    let (events, alive) = (events.clone(), alive.clone());
                    std::thread::spawn(move || {
                        let mut reader = reader;
                        while let Ok(line) = read_line(&mut reader) {
                            if let Ok(d) = serde_json::from_str::<Down>(&line)
                                && events.send(SocialEvent::Down(Box::new(d))).is_err() {
                                break;
                            }
                        }
                        alive.store(false, Ordering::Relaxed);
                    });
                }
                let mut last_ping = std::time::Instant::now();
                loop {
                    if stopped.load(Ordering::Relaxed) {
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                    if !alive.load(Ordering::Relaxed) {
                        break;
                    }
                    // Ping on a clock, not only when idle: a solo play streams a frame every
                    // 100 ms, and without pings the server has nothing to answer, so the reader
                    // timed out ~70 s into every map.
                    if last_ping.elapsed() > Duration::from_secs(25) {
                        last_ping = std::time::Instant::now();
                        if write_line(&mut stream, &Up::Ping).is_err() {
                            break;
                        }
                    }
                    let up = match out.recv_timeout(Duration::from_millis(300)) {
                        Ok(up) => up,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    };
                    if write_line(&mut stream, &up).is_err() {
                        break;
                    }
                }
                let _ = stream.shutdown(std::net::Shutdown::Both);
                let _ = events.send(SocialEvent::Disconnected(tr!("lost the connection", "接続が切れました", "соединение потеряно").into()));
                announced_down = true;
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        Social { tx, rx, stop }
    }

    pub fn send(&self, up: Up) {
        let _ = self.tx.send(up);
    }

    pub fn poll(&self) -> Vec<SocialEvent> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Social {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One request and its reply; a `Reply::Error` comes back as `Err`.
pub fn call(server: &str, request: Request) -> Result<Reply, String> {
    match exchange(server, request)?.0 {
        Reply::Error(e) => Err(e),
        reply => Ok(reply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_titles() {
        assert!(valid_name("Lynerrr") && valid_name("den4ik_228") && valid_name("a-b"));
        assert!(!valid_name("ab") && !valid_name("has space") && !valid_name("x".repeat(17).as_str()) && !valid_name("ünï"));
        assert!(valid_login("Lynerrr") && valid_login("a.long.name+taiko@mail.ru"));
        assert!(!valid_login("ab") && !valid_login("no.at.sign") && !valid_name("me@mail.ru"));
        assert!(valid_password("hunter2") && !valid_password("short"));
        assert_eq!(clean_title("  ", "Lynerrr", false), "Lynerrr's lobby");
        assert_eq!(clean_title("", "Lynerrr", true), "Lynerrr's mapguessr");
        assert_eq!(clean_title("taiko\u{7}  night", "x", true), "taiko  night");
    }

    #[test]
    fn messages_round_trip() {
        let env = Envelope { protocol: PROTOCOL, request: Request::JoinRoom { token: "t".into(), room: 3 } };
        let s = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(back.request, Request::JoinRoom { room: 3, .. }));
        let mut buf = Vec::new();
        write_line(&mut buf, &Reply::Joined { room: 3, title: "x".into(), host_token: Some(7) }).unwrap();
        let line = read_line(&mut &buf[..]).unwrap();
        assert!(matches!(serde_json::from_str::<Reply>(&line).unwrap(), Reply::Joined { host_token: Some(7), .. }));
        let d = Down::Players { count: 2, players: vec![OnlinePlayer { name: "a".into(), activity: Activity::Lobby { room: Some(1), title: "x".into() } }] };
        let back: Down = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert!(matches!(back, Down::Players { count: 2, ref players } if players.len() == 1));
        assert!(matches!(serde_json::from_str::<Up>(&serde_json::to_string(&Up::GlobalChat("hi".into())).unwrap()).unwrap(), Up::GlobalChat(t) if t == "hi"));
    }
}
