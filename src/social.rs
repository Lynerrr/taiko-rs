//! Friends and chat: the state behind the F9 overlay, and drawing it. Friends chat over the
//! social connection to the online server (no lobby needed); in a lobby there's also its chat.
//! Besides friends the overlay lists everyone online (to add them) and has a global chat.

use std::cell::Cell;
use std::collections::HashMap;
use std::time::Instant;

use crate::online::{self, Activity, ChatLine, Friend, OnlinePlayer};
use crate::render::{Canvas, Hit, accent, rgba};

/// The overlay's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Friends,
    /// Everyone online.
    Players,
    /// The global chat.
    Chat,
    /// Admins only: who's on, the lobbies, and the admin commands.
    Admin,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Friends, Tab::Players, Tab::Chat, Tab::Admin];

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Friends => "FRIENDS",
            Tab::Players => "PLAYERS",
            Tab::Chat => "CHAT",
            Tab::Admin => "ADMIN",
        }
    }
}

/// Overlay rows, top to bottom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    /// Type a name, Enter sends a friend request.
    AddFriend,
    /// The lobby's chat (while in a room).
    Lobby,
    Request(usize),
    Friend(usize),
    /// The players tab's filter (what's typed there).
    Search,
    /// An index into `Social::players`.
    Player(usize),
    /// The global chat (the chat tab's only row).
    Global,
    /// The admin tab's command box.
    AdminInput,
    /// An index into the admin overview's players, and its lobbies.
    AdminPlayer(usize),
    AdminRoom(usize),
}

/// Buttons on rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Accept,
    Decline,
    Watch,
    Join,
    /// Sends a friend request to a player.
    Add,
    /// Opens the conversation with a friend from the players tab.
    Chat,
    /// Shows the player's profile.
    Profile,
    /// Admins: disconnect, mute for 30 minutes, look up, close a lobby.
    Kick,
    Mute,
    Lookup,
    CloseRoom,
}

/// Where a player in the online list stands with us.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    Me,
    Friend,
    /// They asked to be friends.
    AsksUs,
    /// We asked.
    Asked,
    Stranger,
}

/// What clicking a notification does.
#[derive(Clone, PartialEq, Debug)]
pub enum NoteAction {
    /// Opens the overlay on this friend's conversation.
    Chat(String),
    /// Opens the overlay on the friend requests.
    Requests,
    /// Opens the global chat.
    Global,
    /// Watches this friend's play.
    Watch(String),
    None,
}

/// A notification card (top right, a few seconds).
pub struct Note {
    pub title: String,
    pub body: String,
    pub colour: u32,
    pub action: NoteAction,
    pub at: Instant,
}

/// How long a notification stays up.
pub const NOTE_SECS: f32 = 6.0;

#[derive(Default)]
pub struct Social {
    pub notes: Vec<Note>,
    /// The friend list has been received since connecting (later changes notify).
    pub have_list: bool,
    pub conn: Option<online::Social>,
    /// Our account name once connected.
    pub me: Option<String>,
    pub connected: bool,
    pub friends: Vec<Friend>,
    pub requests: Vec<String>,
    pub sent: Vec<String>,
    /// Conversations by friend (lowercased), oldest first.
    pub chats: HashMap<String, Vec<ChatLine>>,
    /// Players in a group (lowercased) and their groups' tags ("DEV", "LOVED", "200"): a tag by
    /// their name in chats, and the name shimmers in the group's colours.
    pub groups: HashMap<String, Vec<String>>,
    pub unread: HashMap<String, usize>,
    /// Everyone online (None until the server says: older servers don't).
    pub players: Option<Vec<OnlinePlayer>>,
    /// How many are online (the list stops at `online::MAX_LISTED`).
    pub online_total: usize,
    /// The global chat, oldest first.
    pub global: Vec<ChatLine>,
    pub global_unread: usize,
    pub open: bool,
    /// A text field of the panel was clicked: it takes the letter keys. Only the editor scenes
    /// ask for it (elsewhere an open panel takes the keyboard whole).
    pub typing: bool,
    pub opened_at: Option<Instant>,
    /// There's a screenshot to send.
    pub shot_ready: bool,
    /// Where the tab is now (see `glide_tab`).
    tab_y: Option<f32>,
    /// When it last closed (it slides away).
    pub closed_at: Option<Instant>,
    pub tab: Tab,
    /// The friends tab shows the selected conversation (a friend's or the lobby's) over the
    /// whole panel instead of the list.
    pub viewing: bool,
    /// The selected row of the shown tab.
    pub cursor: usize,
    /// The other tabs' cursors, by `Tab::index`.
    tab_cursors: [usize; 4],
    /// What's typed: a friend's name on the add row, the filter on the players tab, else a message.
    pub input: String,
    /// Chat lines scrolled back from the newest, and the most that can be (set when drawn).
    pub chat_scroll: usize,
    chat_scroll_max: Cell<usize>,
    /// The activity last sent to the server.
    pub sent_activity: Option<Activity>,
    /// The account is an admin: the ADMIN tab and the admin commands.
    pub admin: bool,
    /// The server as the last overview had it, and when that was asked for.
    pub overview: Option<online::AdminOverview>,
    pub overview_asked: Option<Instant>,
    /// What the admin tab shows over its list (a lookup, the commands): a title and lines.
    pub info: Option<(String, Vec<String>)>,
}

/// The most spectators listed by name during a play.
const SPECTATORS_LISTED: usize = 8;

/// Who watches a solo play, by name, where a multiplayer match has its scoreboard.
pub fn draw_spectators(c: &mut Canvas, names: &[&str]) {
    if names.is_empty() {
        return;
    }
    let n = names.len();
    let (x, y0, h) = (18.0, 282.0, 14.0);
    let more = n.saturating_sub(SPECTATORS_LISTED);
    let title = tr!("Spectators ({n})", "観戦者 ({n})", "Зрители ({n})";);
    let rest = (more > 0).then(|| tr!("and {more} more", "他{more}人", "и ещё {more}";));
    let listed = &names[..n.min(SPECTATORS_LISTED)];
    let lines = 1 + listed.len() + usize::from(rest.is_some());
    let longest = listed
        .iter()
        .copied()
        .chain(rest.as_deref())
        .chain([title.as_str()])
        .map(|l| Canvas::text_width(7.0, l))
        .fold(0.0, f32::max);
    c.round_rect(
        [x - 6.0, y0 - 6.0],
        [longest + 12.0, lines as f32 * h + 8.0],
        6.0,
        rgba(0x000000, 0.45),
        rgba(0, 0.0),
        0.0,
    );
    c.text(x, y0, 7.0, rgba(0x4fb4ff, 1.0), &title);
    for (i, line) in listed.iter().copied().chain(rest.as_deref()).enumerate() {
        c.text(x, y0 + (i + 1) as f32 * h, 7.0, rgba(0xc8c8d8, 1.0), line);
    }
}

/// Conversation key for a friend.
pub fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Splits `text` into lines at most `width` Latin characters wide (a Japanese one is 4/3 of
/// one), at spaces where possible.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    // In font pixels.
    let px = |s: &mut dyn Iterator<Item = char>| s.map(crate::font::advance).sum::<usize>();
    let width = width * crate::font::advance(' ');
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let len = px(&mut line.chars());
            let room = if len == 0 {
                width
            } else {
                width.saturating_sub(len + crate::font::advance(' '))
            };
            if px(&mut word.iter().copied()) <= room {
                if len > 0 {
                    line.push(' ');
                }
                line.extend(word.iter());
                break;
            }
            if len > 0 {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            // A word longer than a line (or Japanese, which has no spaces): cut it.
            let mut fits = 0;
            let mut w = 0;
            while fits < word.len() && w + crate::font::advance(word[fits]) <= width {
                w += crate::font::advance(word[fits]);
                fits += 1;
            }
            let rest = word.split_off(fits.max(1));
            lines.push(word.iter().collect());
            word = rest;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// How long the overlay takes to slide in or out (ms).
pub const SLIDE_MS: f32 = 240.0;
/// How far the panel and the tab run on past the edge that cuts them off, to hide their corners.
const EDGE_BLEED: f32 = 12.0;
/// The tab on the right edge of the screen that pulls the overlay out, and rides on it.
const TAB_W: f32 = 17.0;
pub const TAB_MIDDLE: f32 = 240.0;
/// The main menu's panel reaches the sides at that height: under it.
pub const TAB_MENU_MIDDLE: f32 = 352.0;
const TAB_TEXT: f32 = 6.5;
/// The tab's key's row, and the room at its ends.
const TAB_KEY_ROW: f32 = 14.0;
const TAB_PAD: f32 = 6.0;
/// How long (ms) the tab takes to get most of the way to a new height.
const TAB_GLIDE_MS: f32 = 90.0;

impl Social {
    /// The tab's height `dt_ms` on, gliding to `middle` when another screen wants it elsewhere.
    pub fn glide_tab(&mut self, middle: f32, dt_ms: f32, animate: bool) -> f32 {
        let y = match self.tab_y {
            Some(y) if animate => {
                let y = y + (middle - y) * (1.0 - (-dt_ms / TAB_GLIDE_MS).exp());
                if (y - middle).abs() < 0.25 { middle } else { y }
            }
            _ => middle,
        };
        self.tab_y = Some(y);
        y
    }

    /// Closes the overlay; it slides away (`closing`).
    pub fn close(&mut self) {
        self.typing = false;
        if self.open {
            self.open = false;
            self.closed_at = Some(Instant::now());
        }
    }

    /// How far it has slid away (0-1) while closing.
    pub fn closing(&self) -> Option<f32> {
        let p = crate::render::progress(self.closed_at?, SLIDE_MS);
        (!self.open && p < 1.0).then_some(p)
    }

    /// Who's watching us: friends whose activity is watching us (only friends can).
    pub fn spectators(&self) -> Vec<&str> {
        let Some(me) = self.me.as_deref().filter(|_| self.connected) else {
            return Vec::new();
        };
        self.friends.iter().filter(|f| matches!(&f.activity, Some(Activity::Watching { name }) if key(name) == key(me))).map(|f| f.name.as_str()).collect()
    }

    pub fn notify(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        colour: u32,
        action: NoteAction,
    ) {
        self.notes.push(Note {
            title: title.into(),
            body: body.into(),
            colour,
            action,
            at: Instant::now(),
        });
        // The newest few.
        if self.notes.len() > 4 {
            self.notes.remove(0);
        }
    }

    /// Drops notifications whose time is up (`paused`: during gameplay they wait).
    pub fn expire_notes(&mut self, paused: bool) {
        if paused {
            let now = Instant::now();
            for n in &mut self.notes {
                n.at = now;
            }
        } else {
            self.notes
                .retain(|n| n.at.elapsed().as_secs_f32() < NOTE_SECS);
        }
    }

    pub fn rows(&self, in_lobby: bool) -> Vec<Row> {
        match self.tab {
            Tab::Friends => {
                let mut rows = vec![Row::AddFriend];
                if in_lobby {
                    rows.push(Row::Lobby);
                }
                rows.extend((0..self.requests.len()).map(Row::Request));
                // Online first, each part in the server's order.
                let (online, offline): (Vec<usize>, Vec<usize>) =
                    (0..self.friends.len()).partition(|&i| self.friends[i].activity.is_some());
                rows.extend(online.into_iter().chain(offline).map(Row::Friend));
                rows
            }
            Tab::Players => {
                let filter = key(self.input.trim());
                let players = self.players.as_deref().unwrap_or(&[]);
                let mut rows = vec![Row::Search];
                rows.extend(
                    players
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| key(&p.name).contains(&filter))
                        .map(|(i, _)| Row::Player(i)),
                );
                rows
            }
            Tab::Chat => vec![Row::Global],
            Tab::Admin => {
                let o = self.overview.as_ref();
                let mut rows = vec![Row::AdminInput];
                rows.extend((0..o.map_or(0, |o| o.players.len())).map(Row::AdminPlayer));
                rows.extend((0..o.map_or(0, |o| o.rooms.len())).map(Row::AdminRoom));
                rows
            }
        }
    }

    /// Shows another tab (each keeps its selected row; what's typed is dropped).
    pub fn set_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        self.tab_cursors[self.tab.index()] = self.cursor;
        self.tab = tab;
        self.cursor = self.tab_cursors[tab.index()];
        self.input.clear();
        self.chat_scroll = 0;
    }

    /// The tabs shown: the admin one for admins only.
    pub fn tabs(&self) -> &'static [Tab] {
        if self.admin {
            &Tab::ALL
        } else {
            &Tab::ALL[..3]
        }
    }

    /// The next (or previous) tab.
    pub fn cycle_tab(&mut self, back: bool) {
        let tabs = self.tabs();
        let n = tabs.len();
        let at = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let i = (at + if back { n - 1 } else { 1 }) % n;
        self.set_tab(tabs[i]);
    }

    /// A new overview from the server, keeping the selected player or lobby selected.
    pub fn set_overview(&mut self, o: online::AdminOverview, in_lobby: bool) {
        let picked = match self.row(in_lobby) {
            Row::AdminPlayer(i) => self
                .overview
                .as_ref()
                .and_then(|o| o.players.get(i))
                .map(|p| (true, key(&p.name))),
            Row::AdminRoom(i) => self
                .overview
                .as_ref()
                .and_then(|o| o.rooms.get(i))
                .map(|r| (false, r.id.to_string())),
            _ => None,
        };
        self.overview = Some(o);
        let rows = self.rows(in_lobby);
        let o = self.overview.as_ref();
        let at = picked.and_then(|(player, k)| {
            rows.iter().position(|r| match *r {
                Row::AdminPlayer(i) if player => o
                    .and_then(|o| o.players.get(i))
                    .is_some_and(|p| key(&p.name) == k),
                Row::AdminRoom(i) if !player => o
                    .and_then(|o| o.rooms.get(i))
                    .is_some_and(|r| r.id.to_string() == k),
                _ => false,
            })
        });
        if self.tab == Tab::Admin {
            self.cursor = at.unwrap_or(self.cursor.min(rows.len() - 1));
        }
    }

    /// Who is in a group, as the server says (`Down::Groups`): the whole list each time.
    pub fn set_groups(&mut self, list: Vec<(String, Vec<String>)>) {
        self.groups = list
            .into_iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(name, tags)| (key(&name), tags))
            .collect();
    }

    /// The tags of `name`'s groups (none for most).
    pub fn tags_of(&self, name: &str) -> &[String] {
        self.groups.get(&key(name)).map_or(&[], Vec::as_slice)
    }

    /// A global chat line an admin took down goes away here too.
    pub fn delete_global(&mut self, from: &str, at: u64) {
        self.global.retain(|l| !(l.from == from && l.at == at));
    }

    /// Mouse wheel over the overlay: scrolls the chat when one is shown, else the list.
    pub fn wheel(&mut self, notches: i32, in_lobby: bool) {
        let over_chat = match self.tab {
            Tab::Friends => self.viewing,
            Tab::Players | Tab::Admin => false,
            Tab::Chat => true,
        };
        if over_chat {
            let max = self.chat_scroll_max.get() as i32;
            self.chat_scroll = (self.chat_scroll as i32 + notches * 3).clamp(0, max) as usize;
        } else {
            let rows = self.rows(in_lobby).len() as i32;
            self.cursor =
                (self.cursor.min(rows as usize - 1) as i32 - notches).clamp(0, rows - 1) as usize;
            self.mark_read(in_lobby);
        }
    }

    /// Where `name` stands with us.
    pub fn standing(&self, name: &str) -> Standing {
        let k = key(name);
        if self.me.as_deref().map(key) == Some(k.clone()) {
            Standing::Me
        } else if self.friends.iter().any(|f| key(&f.name) == k) {
            Standing::Friend
        } else if self.requests.iter().any(|r| key(r) == k) {
            Standing::AsksUs
        } else if self.sent.iter().any(|r| key(r) == k) {
            Standing::Asked
        } else {
            Standing::Stranger
        }
    }

    pub fn row(&self, in_lobby: bool) -> Row {
        let rows = self.rows(in_lobby);
        rows[self.cursor.min(rows.len() - 1)]
    }

    pub fn online_count(&self) -> usize {
        self.friends.iter().filter(|f| f.activity.is_some()).count()
    }

    pub fn unread_total(&self) -> usize {
        self.unread.values().sum()
    }

    /// Selects the conversation with `name` and shows it.
    pub fn select_friend(&mut self, name: &str, in_lobby: bool) {
        self.set_tab(Tab::Friends);
        if let Some(i) = self.friends.iter().position(|f| key(&f.name) == key(name))
            && let Some(r) = self
                .rows(in_lobby)
                .iter()
                .position(|r| *r == Row::Friend(i))
        {
            self.cursor = r;
            self.open_conversation();
        }
    }

    /// Shows the selected row's conversation, when it has one.
    pub fn open_conversation(&mut self) {
        if self.tab == Tab::Friends && !self.viewing {
            self.viewing = true;
            self.chat_scroll = 0;
        }
    }

    /// Back from a conversation to the friend list.
    pub fn close_conversation(&mut self) {
        self.viewing = false;
        self.chat_scroll = 0;
        self.input.clear();
    }

    /// Whether the friends tab shows a conversation (`viewing`, on a row that has one).
    pub fn in_conversation(&self, in_lobby: bool) -> bool {
        self.viewing && self.tab == Tab::Friends && self.conversation(in_lobby).is_some()
    }

    /// The name of whoever the selected row is (a friend or an online player).
    fn selected_name(&self, in_lobby: bool) -> Option<String> {
        match self.row(in_lobby) {
            Row::Friend(i) => self.friends.get(i).map(|f| key(&f.name)),
            Row::Player(i) => self.players.as_ref()?.get(i).map(|p| key(&p.name)),
            _ => None,
        }
    }

    /// Puts the cursor back on `name` (lists change order as people come and go), else keeps
    /// it in range; a conversation whose friend is gone closes.
    fn reselect(&mut self, name: Option<String>, in_lobby: bool) {
        let rows = self.rows(in_lobby);
        let at = name.and_then(|n| {
            rows.iter().position(|r| match *r {
                Row::Friend(i) => self.friends.get(i).is_some_and(|f| key(&f.name) == n),
                Row::Player(i) => self
                    .players
                    .as_ref()
                    .and_then(|v| v.get(i))
                    .is_some_and(|p| key(&p.name) == n),
                _ => false,
            })
        });
        let was_friend = matches!(self.row(in_lobby), Row::Friend(_));
        match at {
            Some(r) => self.cursor = r,
            None => {
                self.cursor = self.cursor.min(rows.len() - 1);
                if was_friend && self.viewing {
                    self.viewing = false;
                }
            }
        }
    }

    /// A new friend list from the server, keeping the selected friend selected.
    pub fn set_friends(
        &mut self,
        friends: Vec<Friend>,
        requests: Vec<String>,
        sent: Vec<String>,
        in_lobby: bool,
    ) {
        let name = if self.tab == Tab::Friends {
            self.selected_name(in_lobby)
        } else {
            None
        };
        self.friends = friends;
        self.requests = requests;
        self.sent = sent;
        if self.tab == Tab::Friends {
            self.reselect(name, in_lobby);
        } else {
            let other = self.tab_cursors[Tab::Friends.index()];
            self.tab_cursors[Tab::Friends.index()] =
                other.min(self.requests.len() + self.friends.len() + usize::from(in_lobby));
        }
    }

    /// A new list of everyone online, keeping the selected player selected.
    pub fn set_players(&mut self, count: usize, players: Vec<OnlinePlayer>, in_lobby: bool) {
        let name = if self.tab == Tab::Players {
            self.selected_name(in_lobby)
        } else {
            None
        };
        self.online_total = count;
        self.players = Some(players);
        if self.tab == Tab::Players {
            self.reselect(name, in_lobby);
        }
    }

    /// How many drawn rows a new global chat line adds (scrolled back, the view stays put).
    pub fn global_rows_added(&self, line: &ChatLine) -> usize {
        let entry = |l: &ChatLine| {
            ChatEntry::new(
                &l.from,
                l.shown(),
                false,
                l.medal.as_ref(),
                Some(l.at),
                false,
            )
        };
        let last: Vec<ChatEntry> = self.global.last().map(entry).into_iter().collect();
        let before = chat_rows(&last, chat_width(), banner_width()).len();
        let after = chat_rows(
            &last.into_iter().chain([entry(line)]).collect::<Vec<_>>(),
            chat_width(),
            banner_width(),
        )
        .len();
        after - before
    }

    /// The conversation shown: (title, key) for a friend, or the lobby.
    fn conversation(&self, in_lobby: bool) -> Option<Result<&Friend, ()>> {
        match self.row(in_lobby) {
            Row::Friend(i) => self.friends.get(i).map(Ok),
            Row::Lobby => Some(Err(())),
            _ => None,
        }
    }

    /// Marks what's on screen as read.
    pub fn mark_read(&mut self, in_lobby: bool) {
        match self.row(in_lobby) {
            Row::Friend(i) if self.in_conversation(in_lobby) => {
                if let Some(f) = self.friends.get(i) {
                    self.unread.remove(&key(&f.name));
                }
            }
            Row::Global => self.global_unread = 0,
            _ => {}
        }
    }

    /// Whether a global chat line calls us by name.
    pub fn mentions_me(&self, text: &str) -> bool {
        let Some(me) = self.me.as_deref().map(key) else {
            return false;
        };
        text.to_lowercase()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .any(|w| w == me)
    }
}

const PANEL_W: f32 = 290.0;
const GREEN: u32 = 0x86E05A;
/// The banner of a line from the server that isn't about a medal.
const SERVER_COLOUR: u32 = 0xFFB84D;
/// The size chat is drawn at.
const CHAT_SIZE: f32 = 7.0;
/// How far a message sits in from its sender's name.
const MESSAGE_INDENT: f32 = 11.0;
/// The panel's fill, its header strip's, and a row's (idle, hovered, selected).
const PANEL_BG: u32 = 0x0d0d15;
const HEADER_BG: u32 = 0x14141f;
const ROW_BG: u32 = 0x161621;
const ROW_HOVER: u32 = 0x1e1e2c;
const ROW_SELECTED: u32 = 0x262639;
const LINE: u32 = 0x262634;
const DIM_TEXT: u32 = 0x6f6f86;
const SOFT_TEXT: u32 = 0x9a9ab0;
/// The header strip's height (the tabs).
const HEADER_H: f32 = 26.0;
/// The hint line's band at the bottom.
const FOOTER_H: f32 = 20.0;

/// Characters per chat message row (drawn at `CHAT_SIZE` under the name, in the panel's
/// padding).
pub fn chat_width() -> usize {
    ((PANEL_W - 16.0 - 8.0 - MESSAGE_INDENT) / Canvas::char_width(CHAT_SIZE)) as usize
}

/// Characters per row of the server's lines (smaller, on a banner the panel's width).
fn banner_width() -> usize {
    ((PANEL_W - 16.0 - 10.0) / Canvas::char_width(6.5)) as usize
}

/// The dot's colour for what someone is doing (None: offline).
fn status_colour(activity: Option<&Activity>) -> u32 {
    match activity {
        None => 0x55556a,
        Some(Activity::Menus) => GREEN,
        Some(Activity::Playing { .. }) => 0xFFB84D,
        Some(Activity::Match { .. }) => 0xFF7A59,
        Some(Activity::Lobby { .. }) => 0x4fb4ff,
        Some(Activity::Watching { .. }) => 0xB48CFF,
        Some(Activity::Editing { .. }) => 0xF2D45C,
    }
}

/// Someone's picture centred on (`cx`, `cy`), with a dot for what they're doing on its corner
/// (`ring`: the colour around the dot, what it sits on).
#[allow(clippy::too_many_arguments)]
fn face(
    c: &mut Canvas,
    name: &str,
    cx: f32,
    cy: f32,
    r: f32,
    activity: Option<&Activity>,
    ring: u32,
    a: f32,
) {
    c.avatar(
        name,
        cx,
        cy,
        r,
        name_colour(name, false),
        if activity.is_some() { a } else { 0.55 * a },
    );
    let d = r * 0.7;
    c.circle(
        cx + d,
        cy + d,
        3.0,
        rgba(status_colour(activity), a),
        rgba(ring, a),
        1.5,
    );
}

/// Names in chat get a colour each, the same every time (ours is the accent).
fn name_colour(name: &str, mine: bool) -> u32 {
    const PALETTE: [u32; 8] = [
        0x7FD1FF, 0xFF9AC1, 0xA8E06A, 0xFFC66B, 0xC39BFF, 0x6FE3C9, 0xFF8F7A, 0x9FB4FF,
    ];
    if mine {
        return accent();
    }
    let h = key(name)
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE[h as usize % PALETTE.len()]
}

/// The two colours a group's names shimmer between, the first also its tag's: the website's
/// badge colours (DEV, LOVED) and the "200" badge's fire.
fn group_colours(tag: &str) -> (u32, u32) {
    match tag {
        "DEV" => (0xEB452C, 0xFFB15C),
        "LOVED" => (0xFF66AA, 0xC39BFF),
        "200" => (0xE23A2E, 0xFFB940),
        // A group from a newer server: a quiet one.
        _ => (0x9FB4FF, 0xDEDEEA),
    }
}

/// `a` towards `b` by `t` (0..=1), channel by channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// The colour of letter `i` of `n` in a group member's name, `secs` into the shimmer: a soft
/// gradient between the group's two colours that drifts slowly along the name, and a faint glint
/// passing every few seconds. Slow and low in contrast on purpose: a flex, not a strobe.
fn shimmer(colours: (u32, u32), i: usize, n: usize, secs: f32) -> u32 {
    let x = (i as f32 + 0.5) / n.max(1) as f32;
    let wave = 0.5 + 0.5 * ((x * 1.3 - secs / 3.5) * std::f32::consts::TAU).sin();
    let base = mix(colours.0, colours.1, wave);
    // The glint crosses the name in the first part of each 4.5 s, then rests.
    let at = (secs / 4.5).fract() * 2.2 - 0.3;
    let d = (x - at) / 0.13;
    mix(base, 0xFFFFFF, 0.32 * (-d * d).exp())
}

/// Seconds since the shimmer started (the first name drawn), for its slow motion.
fn shimmer_secs() -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// A group member's name, letter by letter in the shimmer's colours.
fn shimmer_name(c: &mut Canvas, x: f32, y: f32, name: &str, colours: (u32, u32), a: f32) {
    let secs = shimmer_secs();
    let n = name.chars().count();
    let mut buf = [0u8; 4];
    let mut cx = x;
    for (i, ch) in name.chars().enumerate() {
        let colour = shimmer(colours, i, n, secs);
        cx += c.text(cx, y, CHAT_SIZE, rgba(colour, a), ch.encode_utf8(&mut buf));
    }
}

const TAG_SIZE: f32 = 5.5;

/// What a group's tag says: three characters at most, so every pill is about as wide ("LVD"
/// for the loved nominators; a longer tag from a newer server is cut).
fn tag_label(tag: &str) -> &str {
    match tag {
        "LOVED" => "LVD",
        _ => tag.char_indices().nth(3).map_or(tag, |(i, _)| &tag[..i]),
    }
}

fn tag_width(tag: &str) -> f32 {
    Canvas::text_width(TAG_SIZE, tag_label(tag)) + 6.0
}

/// A group's tag, a small pill at `x` on a name row; returns its width.
fn group_tag(c: &mut Canvas, x: f32, y: f32, tag: &str, a: f32) -> f32 {
    const SIZE: f32 = TAG_SIZE;
    let (colour, light) = group_colours(tag);
    let w = tag_width(tag);
    let h = Canvas::text_height(SIZE) + 3.0;
    let top = y + 1.0 + (Canvas::text_height(CHAT_SIZE) - h) / 2.0;
    c.round_rect(
        [x, top],
        [w, h],
        2.0,
        rgba(colour, 0.22 * a),
        rgba(colour, 0.85 * a),
        1.0,
    );
    c.text(x + 3.0, top + 1.5, SIZE, rgba(light, a), tag_label(tag));
    w
}

/// How long ago `at` was, short: "now", "5m", "3h", "2d".
pub fn ago(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// The overlay: a panel on the right over whatever screen is showing (`dim`: the rest of the
/// screen darkens a little, not over a play being watched). `lobby`: the room's chat when in
/// one (name, line). `solid`: nothing shows through it (over the editor, which isn't dimmed).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    s: &Social,
    logged_in: bool,
    lobby: Option<&[(String, String)]>,
    lobby_unread: usize,
    dim: bool,
    solid: bool,
    tab: Option<f32>,
) {
    // It slides out of the edge of the screen, and back in there.
    let slide = match s.closing() {
        Some(p) => 1.0 - p,
        None => s
            .opened_at
            .map(|t| crate::render::progress(t, SLIDE_MS))
            .unwrap_or(1.0),
    };
    let [left, y, _, h] = panel_rect(c.width);
    let x = left + (1.0 - slide) * PANEL_W;
    let a = 1.0;
    if dim {
        c.rect(
            -40.0,
            -40.0,
            c.width + 80.0,
            560.0,
            rgba(0x000000, 0.32 * slide),
        );
    }
    if let Some(middle) = tab {
        draw_tab(c, s, x, middle, lobby_unread);
    }
    // A soft shadow on the left, then the panel and its header strip.
    for (i, alpha) in [0.10, 0.07, 0.04].into_iter().enumerate() {
        let d = 2.0 * (i + 1) as f32;
        c.round_rect(
            [x - d, y - d + 2.0],
            [PANEL_W + EDGE_BLEED + 2.0 * d, h + 2.0 * d],
            8.0 + d,
            rgba(0x000000, alpha * a),
            rgba(0, 0.0),
            0.0,
        );
    }
    c.round_rect(
        [x, y],
        [PANEL_W + EDGE_BLEED, h],
        8.0,
        rgba(PANEL_BG, if solid { 1.0 } else { 0.97 } * a),
        rgba(0x34344a, a),
        1.0,
    );
    c.round_rect(
        [x + 1.0, y + 1.0],
        [PANEL_W + EDGE_BLEED, HEADER_H],
        7.0,
        rgba(HEADER_BG, a),
        rgba(0, 0.0),
        0.0,
    );
    c.rect(
        x + 1.0,
        y + HEADER_H - 6.0,
        PANEL_W + EDGE_BLEED,
        7.0,
        rgba(HEADER_BG, a),
    );
    c.rect(
        x + 1.0,
        y + HEADER_H + 1.0,
        PANEL_W + EDGE_BLEED,
        1.0,
        rgba(LINE, a),
    );
    c.hit(x, y, PANEL_W, h, Hit::SocialPanel);
    let inner = x + 8.0;
    let iw = PANEL_W - 16.0;
    let footer = |c: &mut Canvas, hints: &str| {
        c.rect(x + 1.0, y + h - FOOTER_H, PANEL_W - 2.0, 1.0, rgba(LINE, a));
        c.text_centered(
            x + PANEL_W / 2.0,
            y + h - FOOTER_H + 6.5,
            6.5,
            rgba(DIM_TEXT, a),
            &Canvas::fit_hints(6.5, iw, hints),
        );
    };
    if !logged_in {
        c.text(x + 12.0, y + 9.5, 8.5, rgba(accent(), a), "FRIENDS");
        c.text_right(x + PANEL_W - 12.0, y + 10.0, 7.0, rgba(DIM_TEXT, a), "F9");
        c.text(
            inner + 4.0,
            y + HEADER_H + 14.0,
            7.5,
            rgba(0xdedeea, a),
            "Not logged in",
        );
        c.text(
            inner + 4.0,
            y + HEADER_H + 28.0,
            7.0,
            rgba(SOFT_TEXT, a),
            "Log in under MULTIPLAYER > ONLINE",
        );
        c.text(
            inner + 4.0,
            y + HEADER_H + 40.0,
            7.0,
            rgba(SOFT_TEXT, a),
            "to add friends and chat.",
        );
        footer(c, "F9 or Esc close");
        return;
    }
    // The tabs, and how many are online in the corner: smaller and closer together when they
    // don't fit (an admin's four, text grown on a phone).
    let unread = |tab: Tab| match tab {
        Tab::Friends => s.unread_total() + s.requests.len() + lobby_unread,
        Tab::Players | Tab::Admin => 0,
        Tab::Chat => s.global_unread,
    };
    let tabs = s.tabs();
    let row_w = |size: f32, gap: f32| {
        tabs.iter()
            .map(|&t| {
                Canvas::text_width(size, t.label())
                    + if unread(t) > 0 {
                        badge_width(unread(t)) + 3.0
                    } else {
                        0.0
                    }
            })
            .sum::<f32>()
            + gap * (tabs.len() - 1) as f32
    };
    let (size, gap) = [(8.0, 16.0), (8.0, 10.0), (7.0, 8.0), (6.0, 6.0)]
        .into_iter()
        .find(|&(size, gap)| row_w(size, gap) <= PANEL_W - 24.0)
        .unwrap_or((6.0, 4.0));
    let mut tx = x + 12.0;
    for &tab in tabs {
        let label = tab.label();
        let tw = Canvas::text_width(size, label);
        let sel = tab == s.tab;
        let hover = c.hovered(
            tx - gap / 3.0,
            y + 2.0,
            tw + gap * 2.0 / 3.0,
            HEADER_H - 2.0,
        );
        c.text(
            tx,
            y + 13.5 - Canvas::text_height(size) / 2.0,
            size,
            rgba(
                if sel {
                    0xffffff
                } else if hover {
                    0xdedeea
                } else {
                    0x7a7a92
                },
                a,
            ),
            label,
        );
        if sel {
            c.round_rect(
                [tx - 1.0, y + HEADER_H - 2.5],
                [tw + 2.0, 2.5],
                1.0,
                rgba(accent(), a),
                rgba(0, 0.0),
                0.0,
            );
        }
        let n = unread(tab);
        let bw = if n > 0 {
            badge_after(c, tx + tw + 3.0, y + 13.0, n, a) + 3.0
        } else {
            0.0
        };
        c.hit(
            tx - gap / 3.0,
            y + 2.0,
            tw + gap * 2.0 / 3.0 + bw,
            HEADER_H - 2.0,
            Hit::SocialTab(tab),
        );
        tx += tw + gap + bw;
    }
    if s.connected && s.players.is_some() {
        let right = x + PANEL_W - 12.0;
        // What fits after the tabs (and their badges): "1234 online", "1234" or nothing.
        let room = right - (tx - gap + 14.0) - 10.0;
        let label = [
            format!("{} online", s.online_total),
            s.online_total.to_string(),
        ]
        .into_iter()
        .find(|l| Canvas::text_width(7.0, l) <= room);
        if let Some(label) = label {
            c.text_right(right, y + 10.5, 7.0, rgba(SOFT_TEXT, a), &label);
            let dot = right - Canvas::text_width(7.0, &label) - 6.0;
            c.circle(dot, y + 14.0, 2.5, rgba(GREEN, a), rgba(0x000000, 0.0), 0.0);
        }
    }
    let body = y + HEADER_H + 2.0;
    if !s.connected {
        c.text(
            inner + 4.0,
            body + 5.0,
            7.0,
            rgba(SOFT_TEXT, a),
            "connecting to the server...",
        );
    }
    let in_lobby = lobby.is_some();
    let rows = s.rows(in_lobby);
    let cursor = s.cursor.min(rows.len() - 1);
    let selected = rows[cursor];
    let bottom = y + h - FOOTER_H;
    let input_y = bottom - 24.0;
    let width = chat_width();
    let list_top = body + 6.0 + if s.connected { 0.0 } else { 14.0 };
    let me = s.me.as_deref().map(key);
    let now = crate::discord::unix_now();

    match s.tab {
        Tab::Friends if s.in_conversation(in_lobby) => {
            let head_h = 28.0;
            let hy = list_top - 2.0;
            // Back, then who it is with.
            let back_hover = c.hovered(inner, hy + 6.0, 16.0, 16.0);
            c.round_rect(
                [inner, hy + 6.0],
                [16.0, 16.0],
                5.0,
                rgba(if back_hover { ROW_HOVER } else { ROW_BG }, a),
                rgba(LINE, a),
                1.0,
            );
            c.text_centered(inner + 8.0, hy + 10.5, 7.0, rgba(0xdedeea, a), "<");
            c.hit(inner, hy + 6.0, 16.0, 16.0, Hit::SocialBack);
            let tx = inner + 24.0;
            let lines: Vec<ChatEntry> = match s.conversation(in_lobby) {
                Some(Ok(f)) => {
                    let mut bx = button(
                        c,
                        inner + iw,
                        hy + 8.0,
                        "profile",
                        Hit::SocialAction(cursor, Action::Profile),
                        a,
                    ) - 4.0;
                    bx = match &f.activity {
                        Some(Activity::Playing { .. } | Activity::Menus) => {
                            button(
                                c,
                                bx,
                                hy + 8.0,
                                "watch",
                                Hit::SocialAction(cursor, Action::Watch),
                                a,
                            ) - 4.0
                        }
                        Some(Activity::Lobby { room: Some(_), .. }) => {
                            button(
                                c,
                                bx,
                                hy + 8.0,
                                "join",
                                Hit::SocialAction(cursor, Action::Join),
                                a,
                            ) - 4.0
                        }
                        _ => bx,
                    };
                    face(
                        c,
                        &f.name,
                        tx + 9.0,
                        hy + 14.0,
                        9.5,
                        f.activity.as_ref(),
                        PANEL_BG,
                        a,
                    );
                    let tx = tx + 24.0;
                    c.text(
                        tx,
                        hy + 6.0,
                        8.0,
                        rgba(0xffffff, a),
                        &Canvas::fit(8.0, bx - tx - 4.0, &f.name),
                    );
                    let what = f
                        .activity
                        .as_ref()
                        .map(Activity::describe)
                        .unwrap_or_else(|| "offline".into());
                    c.text(
                        tx,
                        hy + 17.0,
                        6.5,
                        rgba(SOFT_TEXT, a),
                        &Canvas::fit(6.5, bx - tx - 4.0, &what),
                    );
                    s.chats
                        .get(&key(&f.name))
                        .map(|v| {
                            v.iter()
                                .map(|l| {
                                    ChatEntry::new(
                                        &l.from,
                                        l.text.clone(),
                                        Some(key(&l.from)) == me,
                                        None,
                                        Some(l.at),
                                        false,
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => {
                    c.text(tx, hy + 6.0, 8.0, rgba(0x4fb4ff, a), "# lobby chat");
                    c.text(
                        tx,
                        hy + 17.0,
                        6.5,
                        rgba(SOFT_TEXT, a),
                        "everyone in your room",
                    );
                    lobby
                        .unwrap_or(&[])
                        .iter()
                        .map(|(from, text)| {
                            ChatEntry::new(
                                from,
                                text.clone(),
                                Some(key(from)) == me,
                                None,
                                None,
                                false,
                            )
                        })
                        .collect()
                }
            };
            c.rect(inner, hy + head_h + 2.0, iw, 1.0, rgba(LINE, a));
            let top = hy + head_h + 8.0;
            if lines.is_empty() {
                c.text_centered(
                    x + PANEL_W / 2.0,
                    top + 10.0,
                    7.0,
                    rgba(DIM_TEXT, a),
                    "no messages yet: say hi",
                );
            }
            chat_lines(c, s, &lines, inner, top, input_y, width, now, false, a);
            message_box(c, s, inner, iw, input_y, "type a message, Enter to send", a);
            footer(
                c,
                "Esc back  Enter send  PgUp/PgDn scroll  Ctrl+W watch  Ctrl+P profile",
            );
        }
        Tab::Friends => {
            let mut ry = draw_rows(
                c,
                s,
                &rows,
                cursor,
                inner,
                iw,
                list_top,
                bottom - 4.0,
                lobby_unread,
                a,
            );
            if s.friends.is_empty() && s.requests.is_empty() && s.connected {
                c.text(
                    inner + 4.0,
                    ry + 6.0,
                    7.0,
                    rgba(DIM_TEXT, a),
                    "no friends yet: add one by name or in PLAYERS",
                );
                ry += 16.0;
            }
            if !s.sent.is_empty() && ry + 12.0 < bottom {
                c.text(
                    inner + 4.0,
                    ry + 5.0,
                    6.5,
                    rgba(DIM_TEXT, a),
                    &Canvas::fit(
                        6.5,
                        iw - 8.0,
                        &format!("waiting for: {}", s.sent.join(", ")),
                    ),
                );
            }
            let hints = match selected {
                Row::Friend(_) => {
                    "Enter chat  Up/Down  Ctrl+W watch  Ctrl+J join  Ctrl+P profile  Del remove"
                }
                Row::Lobby => "Enter open  Up/Down  Tab next tab",
                Row::Request(_) => "Enter accept  Del decline  Up/Down",
                _ => "type a name  Enter send request  Up/Down  Tab next tab",
            };
            footer(c, hints);
        }
        Tab::Players => {
            let ry = draw_rows(
                c,
                s,
                &rows,
                cursor,
                inner,
                iw,
                list_top,
                bottom - 4.0,
                lobby_unread,
                a,
            );
            if s.connected && s.players.is_none() {
                c.text(
                    inner + 4.0,
                    ry + 6.0,
                    7.0,
                    rgba(DIM_TEXT, a),
                    "this server doesn't list players yet",
                );
            } else if rows.len() == 1 && s.connected {
                c.text(
                    inner + 4.0,
                    ry + 6.0,
                    7.0,
                    rgba(DIM_TEXT, a),
                    "nobody online by that name",
                );
            }
            footer(
                c,
                "type to search  Up/Down  Enter add friend  Ctrl+P profile",
            );
        }
        Tab::Chat => {
            let lines: Vec<ChatEntry> = s
                .global
                .iter()
                .map(|l| {
                    ChatEntry::new(
                        &l.from,
                        l.shown(),
                        Some(key(&l.from)) == me,
                        l.medal.as_ref(),
                        Some(l.at),
                        s.mentions_me(&l.text),
                    )
                })
                .collect();
            if lines.is_empty() && s.connected {
                let note = if s.players.is_some() {
                    "nobody has said anything yet: say hi"
                } else {
                    "this server has no global chat yet"
                };
                c.text_centered(
                    x + PANEL_W / 2.0,
                    list_top + 10.0,
                    7.0,
                    rgba(DIM_TEXT, a),
                    note,
                );
            }
            chat_lines(
                c, s, &lines, inner, list_top, input_y, width, now, s.admin, a,
            );
            message_box(
                c,
                s,
                inner,
                iw,
                input_y,
                if s.admin {
                    "message everyone, or an admin /command"
                } else {
                    "message everyone online, Enter to send"
                },
                a,
            );
            footer(c, "Enter send  wheel or PgUp/PgDn scroll  /np share a map");
        }
        Tab::Admin => {
            let mut top = list_top;
            // The server at a glance.
            let stats = match &s.overview {
                Some(o) => format!(
                    "up {}   {} online   {} accounts   {} sessions   {} lobbies",
                    crate::admin::span(o.uptime_secs),
                    o.players.len(),
                    o.accounts,
                    o.sessions,
                    o.rooms.len()
                ),
                None => "asking the server...".into(),
            };
            c.text(
                inner + 3.0,
                top,
                6.5,
                rgba(SOFT_TEXT, a),
                &Canvas::fit(6.5, iw - 6.0, &stats),
            );
            top += Canvas::text_height(6.5) + 6.0;
            // A lookup, or the commands, over the list.
            if let Some((title, lines)) = &s.info {
                let lh = Canvas::text_height(6.5) + 3.0;
                let shown: Vec<String> = lines
                    .iter()
                    .flat_map(|l| wrap(l, banner_width()))
                    .take(10)
                    .collect();
                let h = Canvas::text_height(7.0) + 9.0 + shown.len() as f32 * lh;
                c.round_rect(
                    [inner, top],
                    [iw, h],
                    5.0,
                    rgba(HEADER_BG, a),
                    rgba(accent(), 0.6 * a),
                    1.0,
                );
                c.text(
                    inner + 7.0,
                    top + 4.0,
                    7.0,
                    rgba(accent(), a),
                    &Canvas::fit(7.0, iw - 60.0, title),
                );
                c.text_right(
                    inner + iw - 7.0,
                    top + 4.5,
                    6.0,
                    rgba(DIM_TEXT, a),
                    "Esc hides",
                );
                for (k, l) in shown.iter().enumerate() {
                    c.text(
                        inner + 7.0,
                        top + Canvas::text_height(7.0) + 7.0 + k as f32 * lh,
                        6.5,
                        rgba(0xdedeea, a),
                        l,
                    );
                }
                top += h + 6.0;
            }
            let ry = draw_rows(
                c,
                s,
                &rows,
                cursor,
                inner,
                iw,
                top,
                bottom - 4.0,
                lobby_unread,
                a,
            );
            // The latest admin actions under the list, as many as fit (newest first).
            if let Some(o) = s.overview.as_ref().filter(|o| !o.log.is_empty()) {
                let lh = Canvas::text_height(6.0) + 3.0;
                let fit = ((bottom - 4.0 - ry - 4.0 - HEADING_H) / lh)
                    .floor()
                    .max(0.0) as usize;
                if fit > 0 {
                    section(c, inner, iw, ry + 4.0, "LOG", o.log.len(), a);
                    for (k, l) in o.log.iter().rev().take(fit).enumerate() {
                        c.text(
                            inner + 3.0,
                            ry + 4.0 + HEADING_H + k as f32 * lh,
                            6.0,
                            rgba(DIM_TEXT, a),
                            &Canvas::fit(6.0, iw - 6.0, l),
                        );
                    }
                }
            }
            let hints = match selected {
                Row::AdminPlayer(_) => {
                    "Ctrl+W watch  Ctrl+M mute  Ctrl+K kick  Ctrl+L look up  Ctrl+P profile"
                }
                Row::AdminRoom(_) => "Ctrl+J join  Del close the lobby  Up/Down",
                _ => "type a /command, Enter runs it  /help  Up/Down",
            };
            footer(c, hints);
        }
    }
}

/// A small heading over a part of a list ("ONLINE 3").
fn section(c: &mut Canvas, inner: f32, iw: f32, y: f32, label: &str, n: usize, a: f32) {
    let text = format!("{label}  {n}");
    c.text(inner + 3.0, y + 2.5, 6.0, rgba(DIM_TEXT, a), &text);
    let after = inner + 3.0 + Canvas::text_width(6.0, &text) + 6.0;
    if after < inner + iw {
        c.rect(after, y + 5.5, inner + iw - after, 1.0, rgba(LINE, a));
    }
}

/// The heading row `i` starts under, when it begins a part of the friend list.
fn heading(s: &Social, rows: &[Row], i: usize) -> Option<(&'static str, usize)> {
    let part = |r: &Row| match *r {
        Row::Request(_) => Some(0),
        Row::Friend(k) => Some(if s.friends.get(k).is_some_and(|f| f.activity.is_some()) {
            1
        } else {
            2
        }),
        Row::AdminPlayer(_) => Some(3),
        Row::AdminRoom(_) => Some(4),
        _ => None,
    };
    let this = part(&rows[i])?;
    if i > 0 && part(&rows[i - 1]) == Some(this) {
        return None;
    }
    let n = rows.iter().filter(|r| part(r) == Some(this)).count();
    Some((
        ["REQUESTS", "ONLINE", "OFFLINE", "ONLINE", "LOBBIES"][this],
        n,
    ))
}

const HEADING_H: f32 = 13.0;

/// The rows of a list from `top`, scrolled so the cursor stays visible; returns where they end.
#[allow(clippy::too_many_arguments)]
fn draw_rows(
    c: &mut Canvas,
    s: &Social,
    rows: &[Row],
    cursor: usize,
    inner: f32,
    iw: f32,
    top: f32,
    bottom: f32,
    lobby_unread: usize,
    a: f32,
) -> f32 {
    let height = |i: usize| {
        let base = if matches!(
            rows[i],
            Row::Friend(_) | Row::Player(_) | Row::AdminPlayer(_) | Row::AdminRoom(_)
        ) {
            30.0
        } else {
            24.0
        };
        base + if heading(s, rows, i).is_some() {
            HEADING_H
        } else {
            0.0
        }
    };
    let mut first = 0;
    while (first..=cursor).map(height).sum::<f32>() > bottom - top {
        first += 1;
    }
    let mut ry = top;
    for (i, r) in rows.iter().enumerate().skip(first) {
        let rh = height(i);
        if ry + rh > bottom {
            break;
        }
        if let Some((label, n)) = heading(s, rows, i) {
            section(c, inner, iw, ry, label, n, a);
            ry += HEADING_H;
        }
        let rh = rh
            - if heading(s, rows, i).is_some() {
                HEADING_H
            } else {
                0.0
            };
        let bh = rh - 3.0;
        let sel = i == cursor;
        let hover = c.hovered(inner, ry, iw, bh);
        // Buttons show on the row under the pointer or the cursor; the rest stay quiet.
        let active = sel || hover;
        let bg = if sel {
            ROW_SELECTED
        } else if hover {
            ROW_HOVER
        } else {
            ROW_BG
        };
        c.round_rect(
            [inner, ry],
            [iw, bh],
            5.0,
            rgba(bg, 0.95 * a),
            rgba(
                if sel { 0x3a3a52 } else { 0x000000 },
                if sel { a } else { 0.0 },
            ),
            1.0,
        );
        if sel {
            c.round_rect(
                [inner, ry + 3.0],
                [2.5, bh - 6.0],
                1.0,
                rgba(accent(), a),
                rgba(0, 0.0),
                0.0,
            );
        }
        c.hit(inner, ry, iw, bh, Hit::SocialRow(i));
        let ty = ry + bh / 2.0 - 3.5;
        let by = ry + (bh - 12.0) / 2.0;
        match *r {
            Row::AddFriend => {
                // While typing, the end of the name stays in view.
                let text = if sel {
                    Canvas::fit_end(7.0, iw - 16.0, &format!("add friend: {}_", s.input))
                } else {
                    Canvas::fit(7.0, iw - 16.0, "+ add a friend (type their name)")
                };
                c.text(
                    inner + 9.0,
                    ty,
                    7.0,
                    rgba(if sel { 0xffffff } else { SOFT_TEXT }, a),
                    &text,
                );
            }
            Row::Lobby => {
                c.text(inner + 9.0, ty, 8.0, rgba(0x4fb4ff, a), "# lobby chat");
                badge_after(
                    c,
                    inner + iw - 8.0 - badge_width(lobby_unread),
                    ty + 3.5,
                    lobby_unread,
                    a,
                );
            }
            Row::Request(k) => {
                let name = s.requests.get(k).map(String::as_str).unwrap_or("");
                let bx = button(
                    c,
                    inner + iw - 5.0,
                    by,
                    "decline",
                    Hit::SocialAction(i, Action::Decline),
                    a,
                );
                let bx = button(
                    c,
                    bx - 4.0,
                    by,
                    "accept",
                    Hit::SocialAction(i, Action::Accept),
                    a,
                );
                c.text(
                    inner + 9.0,
                    ty,
                    7.0,
                    rgba(0xFFD34D, a),
                    &Canvas::fit(
                        7.0,
                        bx - inner - 14.0,
                        &format!("{name} wants to be friends"),
                    ),
                );
            }
            Row::Friend(k) => {
                let Some(f) = s.friends.get(k) else { continue };
                let online = f.activity.is_some();
                face(
                    c,
                    &f.name,
                    inner + 15.0,
                    ry + bh / 2.0,
                    9.0,
                    f.activity.as_ref(),
                    bg,
                    a,
                );
                let what = f
                    .activity
                    .as_ref()
                    .map(Activity::describe)
                    .unwrap_or_else(|| "offline".into());
                let mut bx = inner + iw - 5.0;
                if active {
                    bx = button(
                        c,
                        bx,
                        by,
                        "profile",
                        Hit::SocialAction(i, Action::Profile),
                        a,
                    ) - 4.0;
                    bx = match &f.activity {
                        Some(Activity::Playing { .. } | Activity::Menus) => {
                            button(c, bx, by, "watch", Hit::SocialAction(i, Action::Watch), a) - 4.0
                        }
                        Some(Activity::Lobby { room: Some(_), .. }) => {
                            button(c, bx, by, "join", Hit::SocialAction(i, Action::Join), a) - 4.0
                        }
                        _ => bx,
                    };
                }
                let unread = s.unread.get(&key(&f.name)).copied().unwrap_or(0);
                if unread > 0 {
                    let w = badge_width(unread);
                    badge_after(c, bx - w, ry + bh / 2.0, unread, a);
                    bx -= w + 4.0;
                }
                c.text(
                    inner + 31.0,
                    ry + 5.0,
                    8.0,
                    rgba(if online { 0xffffff } else { SOFT_TEXT }, a),
                    &Canvas::fit(8.0, bx - inner - 34.0, &f.name),
                );
                c.text(
                    inner + 31.0,
                    ry + 16.0,
                    6.5,
                    rgba(if online { SOFT_TEXT } else { DIM_TEXT }, a),
                    &Canvas::fit(6.5, bx - inner - 34.0, &what),
                );
            }
            Row::Search => {
                let text = format!(
                    "search: {}{}",
                    if s.input.is_empty() && !sel {
                        "type a name"
                    } else {
                        &s.input
                    },
                    if sel { "_" } else { "" }
                );
                let more = format!("first {} listed", online::MAX_LISTED);
                let capped = s.online_total > s.players.as_ref().map_or(0, Vec::len);
                let room = iw
                    - 18.0
                    - if capped {
                        Canvas::text_width(6.0, &more) + 8.0
                    } else {
                        0.0
                    };
                c.text(
                    inner + 9.0,
                    ty,
                    7.0,
                    rgba(
                        if sel || !s.input.is_empty() {
                            0xffffff
                        } else {
                            SOFT_TEXT
                        },
                        a,
                    ),
                    &Canvas::fit_end(7.0, room, &text),
                );
                if capped {
                    c.text_right(inner + iw - 8.0, ty + 0.5, 6.0, rgba(DIM_TEXT, a), &more);
                }
            }
            Row::Player(k) => {
                let Some(p) = s.players.as_ref().and_then(|v| v.get(k)) else {
                    continue;
                };
                face(
                    c,
                    &p.name,
                    inner + 15.0,
                    ry + bh / 2.0,
                    9.0,
                    Some(&p.activity),
                    bg,
                    a,
                );
                let right = inner + iw - 5.0;
                let standing = s.standing(&p.name);
                let bx = if active {
                    let right = button(
                        c,
                        right,
                        by,
                        "profile",
                        Hit::SocialAction(i, Action::Profile),
                        a,
                    ) - 4.0;
                    match standing {
                        Standing::Me => tag(c, right, by, "you", a),
                        Standing::Friend => {
                            button(c, right, by, "chat", Hit::SocialAction(i, Action::Chat), a)
                        }
                        Standing::AsksUs => button(
                            c,
                            right,
                            by,
                            "accept",
                            Hit::SocialAction(i, Action::Accept),
                            a,
                        ),
                        Standing::Asked => tag(c, right, by, "request sent", a),
                        Standing::Stranger => button(
                            c,
                            right,
                            by,
                            "+ friend",
                            Hit::SocialAction(i, Action::Add),
                            a,
                        ),
                    }
                } else {
                    match standing {
                        Standing::Me => tag(c, right, by, "you", a),
                        Standing::Friend => tag(c, right, by, "friend", a),
                        Standing::AsksUs => tag(c, right, by, "wants to be friends", a),
                        Standing::Asked => tag(c, right, by, "request sent", a),
                        Standing::Stranger => right,
                    }
                } - 4.0;
                c.text(
                    inner + 31.0,
                    ry + 5.0,
                    8.0,
                    rgba(0xffffff, a),
                    &Canvas::fit(8.0, bx - inner - 34.0, &p.name),
                );
                c.text(
                    inner + 31.0,
                    ry + 16.0,
                    6.5,
                    rgba(SOFT_TEXT, a),
                    &Canvas::fit(6.5, bx - inner - 34.0, &p.activity.describe()),
                );
            }
            Row::AdminInput => {
                let text = if sel || !s.input.is_empty() {
                    format!("admin: {}{}", s.input, if sel { "_" } else { "" })
                } else {
                    "admin: type a command (/help lists them)".into()
                };
                c.text(
                    inner + 9.0,
                    ty,
                    7.0,
                    rgba(
                        if sel || !s.input.is_empty() {
                            0xffffff
                        } else {
                            SOFT_TEXT
                        },
                        a,
                    ),
                    &Canvas::fit_end(7.0, iw - 18.0, &text),
                );
            }
            Row::AdminPlayer(k) => {
                let Some(p) = s.overview.as_ref().and_then(|o| o.players.get(k)) else {
                    continue;
                };
                face(
                    c,
                    &p.name,
                    inner + 15.0,
                    ry + bh / 2.0,
                    9.0,
                    p.activity.as_ref(),
                    bg,
                    a,
                );
                let now = crate::discord::unix_now();
                let muted = p.muted_until.filter(|&t| t > now);
                let mut bx = inner + iw - 5.0;
                if active {
                    bx = button(c, bx, by, "kick", Hit::SocialAction(i, Action::Kick), a) - 4.0;
                    bx = button(
                        c,
                        bx,
                        by,
                        if muted.is_some() { "unmute" } else { "mute" },
                        Hit::SocialAction(i, Action::Mute),
                        a,
                    ) - 4.0;
                    bx = button(c, bx, by, "info", Hit::SocialAction(i, Action::Lookup), a) - 4.0;
                    if matches!(p.activity, Some(Activity::Playing { .. } | Activity::Menus)) {
                        bx = button(c, bx, by, "watch", Hit::SocialAction(i, Action::Watch), a)
                            - 4.0;
                    }
                } else if let Some(t) = muted {
                    bx = tag(
                        c,
                        bx,
                        by,
                        &format!("muted {}", crate::admin::span(t - now)),
                        a,
                    ) - 4.0;
                }
                let name = if p.connections > 1 {
                    format!("{}  x{}", p.name, p.connections)
                } else {
                    p.name.clone()
                };
                let what = p
                    .activity
                    .as_ref()
                    .map(Activity::describe)
                    .unwrap_or_default();
                c.text(
                    inner + 31.0,
                    ry + 5.0,
                    8.0,
                    rgba(0xffffff, a),
                    &Canvas::fit(8.0, bx - inner - 34.0, &name),
                );
                c.text(
                    inner + 31.0,
                    ry + 16.0,
                    6.5,
                    rgba(SOFT_TEXT, a),
                    &Canvas::fit(6.5, bx - inner - 34.0, &what),
                );
            }
            Row::AdminRoom(k) => {
                let Some(r) = s.overview.as_ref().and_then(|o| o.rooms.get(k)) else {
                    continue;
                };
                c.round_rect(
                    [inner + 6.0, ry + bh / 2.0 - 9.0],
                    [18.0, 18.0],
                    5.0,
                    rgba(0x4fb4ff, 0.25 * a),
                    rgba(0x4fb4ff, a),
                    1.0,
                );
                c.text_centered(
                    inner + 15.0,
                    ry + bh / 2.0 - 3.0,
                    6.0,
                    rgba(0x4fb4ff, a),
                    &r.id.to_string(),
                );
                let mut bx = inner + iw - 5.0;
                if active {
                    bx = button(
                        c,
                        bx,
                        by,
                        "close",
                        Hit::SocialAction(i, Action::CloseRoom),
                        a,
                    ) - 4.0;
                    bx = button(c, bx, by, "join", Hit::SocialAction(i, Action::Join), a) - 4.0;
                }
                let state = match (&r.map, r.in_match) {
                    (Some(m), true) => format!("playing {m}"),
                    (Some(m), false) => m.clone(),
                    (None, _) => "no map yet".into(),
                };
                let what = format!("{}  {} in it  {state}", r.host, r.players);
                c.text(
                    inner + 31.0,
                    ry + 5.0,
                    8.0,
                    rgba(0xffffff, a),
                    &Canvas::fit(8.0, bx - inner - 34.0, &r.title),
                );
                c.text(
                    inner + 31.0,
                    ry + 16.0,
                    6.5,
                    rgba(SOFT_TEXT, a),
                    &Canvas::fit(6.5, bx - inner - 34.0, &what),
                );
            }
            Row::Global => {}
        }
        ry += rh;
    }
    ry
}

/// A line of a chat, before wrapping.
struct ChatEntry {
    from: String,
    text: String,
    mine: bool,
    /// The colour of the banner it sits on, when the server is saying it.
    banner: Option<u32>,
    /// When it was said (unix seconds), where known.
    at: Option<u64>,
    /// It calls us by name.
    mention: bool,
}

impl ChatEntry {
    fn new(
        from: &str,
        text: String,
        mine: bool,
        medal: Option<&online::ChatMedal>,
        at: Option<u64>,
        mention: bool,
    ) -> Self {
        let banner = (from == online::SERVER_NAME)
            .then(|| medal.map_or(SERVER_COLOUR, |m| online::medal_colour(m.tier, m.secret)));
        Self {
            from: from.to_string(),
            text,
            mine,
            banner,
            at,
            mention,
        }
    }
}

/// What a drawn chat row is.
#[derive(Debug, PartialEq)]
enum ChatKind {
    /// The sender's name over their messages, and when they started.
    Name { at: Option<u64> },
    /// A row of a message.
    Text {
        /// The map set it links to.
        set: Option<u32>,
        mention: bool,
    },
    /// A screenshot: click to see it.
    Shot(u128),
    /// A row of the server's line on its banner; `ends`: its last.
    Banner { ends: bool },
}

/// A chat row as it's drawn, one row of a wrapped `ChatEntry` (or the name over some).
#[derive(Debug, PartialEq)]
struct ChatRow {
    text: String,
    colour: u32,
    kind: ChatKind,
    /// Space above it: messages from someone else start apart.
    gap: f32,
    /// Which of the lines it's (part of).
    entry: usize,
}

impl ChatRow {
    fn height(&self) -> f32 {
        // As tall as the text comes out (it grows on a text grid), and a little apart.
        self.gap
            + match self.kind {
                ChatKind::Name { .. } => Canvas::text_height(CHAT_SIZE) + 3.0,
                ChatKind::Text { .. } => Canvas::text_height(CHAT_SIZE) + 2.5,
                ChatKind::Shot(_) => Canvas::text_height(CHAT_SIZE) + 8.0,
                ChatKind::Banner { ends } => {
                    Canvas::text_height(6.5) + if ends { 3.5 } else { 2.5 }
                }
            }
    }
}

/// Messages from one sender this close together share their name.
const GROUP_SECS: u64 = 5 * 60;

/// `lines` wrapped to `width` (`banner` for the server's): each sender's name over their
/// messages (once for a run of them), the server's lines alone on a banner.
fn chat_rows(lines: &[ChatEntry], width: usize, banner: usize) -> Vec<ChatRow> {
    let mut rows: Vec<ChatRow> = Vec::new();
    let mut last: Option<&ChatEntry> = None;
    for (entry, line) in lines.iter().enumerate() {
        let gap = if rows.is_empty() { 0.0 } else { 5.0 };
        if let Some(colour) = line.banner {
            let text = wrap(&line.text, banner);
            let n = text.len();
            rows.extend(text.into_iter().enumerate().map(|(k, text)| ChatRow {
                text,
                colour,
                kind: ChatKind::Banner { ends: k + 1 == n },
                gap: if k == 0 { gap } else { 0.0 },
                entry,
            }));
            last = None;
            continue;
        }
        let same = last.is_some_and(|l| {
            key(&l.from) == key(&line.from)
                && match (l.at, line.at) {
                    (Some(a), Some(b)) => b.saturating_sub(a) < GROUP_SECS,
                    _ => true,
                }
        });
        let mut gap = if same { 1.0 } else { gap };
        if !same {
            rows.push(ChatRow {
                text: line.from.clone(),
                colour: name_colour(&line.from, line.mine),
                kind: ChatKind::Name { at: line.at },
                gap,
                entry,
            });
            gap = 1.0;
        }
        if let Some(id) = crate::shots::shot_in(&line.text) {
            rows.push(ChatRow {
                text: tr!("Screenshot", "スクリーンショット", "Скриншот").into(),
                colour: 0x66ccff,
                kind: ChatKind::Shot(id),
                gap,
                entry,
            });
            last = Some(line);
            continue;
        }
        // Lines naming a map set (`/np`) are links, in blue.
        let set = map_set_in(&line.text);
        let colour = if set.is_some() { 0x66ccff } else { 0xdedeea };
        for (k, text) in wrap(&line.text, width).into_iter().enumerate() {
            rows.push(ChatRow {
                text,
                colour,
                kind: ChatKind::Text {
                    set,
                    mention: line.mention,
                },
                gap: if k == 0 { gap } else { 0.0 },
                entry,
            });
        }
        last = Some(line);
    }
    rows
}

/// Chat rows between `top` and the message box at `input_y`: newest at the bottom, as many as
/// fit, scrolled back by `s.chat_scroll` rows. `deletable`: an admin can take lines down (a
/// button on the one under the pointer).
#[allow(clippy::too_many_arguments)]
fn chat_lines(
    c: &mut Canvas,
    s: &Social,
    lines: &[ChatEntry],
    inner: f32,
    top: f32,
    input_y: f32,
    width: usize,
    now: u64,
    deletable: bool,
    a: f32,
) {
    let rows = chat_rows(lines, width, banner_width());
    let room = input_y - 6.0 - top;
    // How many of the newest rows fit, ending at `end`.
    let fitting = |end: usize| {
        let mut h = 0.0;
        let mut n = 0;
        for r in rows[..end].iter().rev() {
            h += r.height();
            if h > room {
                break;
            }
            n += 1;
        }
        n
    };
    let max = rows.len().saturating_sub(fitting(rows.len()));
    s.chat_scroll_max.set(max);
    let back = s.chat_scroll.min(max);
    let end = rows.len() - back;
    let shown = &rows[end - fitting(end)..end];
    let mut ly = input_y - 6.0 - shown.iter().map(ChatRow::height).sum::<f32>();
    let iw = PANEL_W - 16.0;
    for (k, row) in shown.iter().enumerate() {
        // The first row shown keeps no gap above it.
        if k > 0 {
            ly += row.gap;
        }
        let h = row.height() - row.gap;
        match row.kind {
            ChatKind::Name { at } => {
                c.avatar(
                    &row.text,
                    inner + 7.5,
                    ly + 1.0 + Canvas::text_height(CHAT_SIZE) / 2.0,
                    4.5,
                    row.colour,
                    a,
                );
                let mut nx = inner + 3.0 + MESSAGE_INDENT;
                let right = inner + iw;
                let tags = s.tags_of(&row.text);
                // Tags while the name keeps some room; then the name, cut to what's left.
                for tag in tags {
                    if nx + tag_width(tag) > right - 40.0 {
                        break;
                    }
                    nx += group_tag(c, nx, ly, tag, a) + 3.0;
                }
                let name = Canvas::fit(CHAT_SIZE, right - nx, &row.text);
                match tags.first() {
                    Some(tag) => shimmer_name(c, nx, ly + 1.0, &name, group_colours(tag), a),
                    None => {
                        c.text(nx, ly + 1.0, CHAT_SIZE, rgba(row.colour, a), &name);
                    }
                }
                if let Some(at) = at {
                    let after = nx + Canvas::text_width(CHAT_SIZE, &name) + 5.0;
                    let when = ago(at, now);
                    if after + Canvas::text_width(6.0, &when) < inner + iw {
                        c.text(after, ly + 2.0, 6.0, rgba(0x55556a, a), &when);
                    }
                }
            }
            ChatKind::Text { set, mention } => {
                if mention {
                    c.rect(inner, ly - 1.0, iw, h, rgba(accent(), 0.13 * a));
                    c.rect(inner, ly - 1.0, 2.0, h, rgba(accent(), a));
                }
                c.text(
                    inner + 3.0 + MESSAGE_INDENT,
                    ly,
                    CHAT_SIZE,
                    rgba(row.colour, a),
                    &row.text,
                );
                if let Some(set) = set {
                    c.hit(inner, ly - 1.0, iw, h, Hit::ChatMap(set));
                }
            }
            ChatKind::Shot(id) => {
                let x = inner + 3.0 + MESSAGE_INDENT;
                let hint = tr!("click to view", "クリックで表示", "нажмите, чтобы открыть");
                let w =
                    Canvas::text_width(CHAT_SIZE, &row.text) + Canvas::text_width(6.0, hint) + 20.0;
                let w = w.min(inner + iw - x);
                let hover = c.hovered(x, ly - 1.0, w, h);
                let fill = if hover { 0x2b2b3d } else { ROW_BG };
                c.round_rect(
                    [x, ly - 1.0],
                    [w, h - 1.0],
                    3.0,
                    rgba(fill, a),
                    rgba(row.colour, 0.7 * a),
                    1.0,
                );
                c.text(x + 6.0, ly + 2.0, CHAT_SIZE, rgba(row.colour, a), &row.text);
                let tx = x + 12.0 + Canvas::text_width(CHAT_SIZE, &row.text);
                c.text(tx, ly + 3.0, 6.0, rgba(SOFT_TEXT, a), hint);
                c.hit(x, ly - 1.0, w, h - 1.0, Hit::ChatShot(id));
            }
            ChatKind::Banner { ends } => {
                let bh = if ends { h - 1.0 } else { h };
                c.rect(inner, ly - 1.5, iw, bh, rgba(row.colour, 0.16 * a));
                c.rect(inner, ly - 1.5, 2.0, bh, rgba(row.colour, a));
                c.text(inner + 7.0, ly, 6.5, rgba(row.colour, a), &row.text);
            }
        }
        if deletable && c.hovered(inner, ly - 1.0, iw, h) {
            button(
                c,
                inner + iw,
                ly + (h - 12.0) / 2.0 - 1.0,
                "delete",
                Hit::SocialDeleteChat(row.entry),
                a,
            );
        }
        ly += h;
    }
    if back > 0 {
        let label = format!("{back} newer rows below  PgDn");
        let w = Canvas::text_width(6.0, &label) + 10.0;
        c.round_rect(
            [inner + iw - w, input_y - 14.0],
            [w, 11.0],
            5.0,
            rgba(HEADER_BG, 0.95 * a),
            rgba(LINE, a),
            1.0,
        );
        c.text(
            inner + iw - w + 5.0,
            input_y - 11.5,
            6.0,
            rgba(SOFT_TEXT, a),
            &label,
        );
    }
}

/// The map set an osu! link in `text` names (`https://osu.ppy.sh/beatmapsets/123...`).
pub fn map_set_in(text: &str) -> Option<u32> {
    let rest = &text[text.find("osu.ppy.sh/beatmapsets/")? + "osu.ppy.sh/beatmapsets/".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The message box at `input_y`.
fn message_box(c: &mut Canvas, s: &Social, inner: f32, iw: f32, input_y: f32, hint: &str, a: f32) {
    let typing = !s.input.is_empty();
    c.round_rect(
        [inner, input_y],
        [iw, 18.0],
        6.0,
        rgba(ROW_BG, a),
        rgba(if typing { accent() } else { 0x34344a }, a),
        1.0,
    );
    let text = if typing {
        format!("{}_", s.input)
    } else {
        hint.to_string()
    };
    let shown = Canvas::fit_end(7.0, iw - 14.0, &text);
    c.text(
        inner + 7.0,
        input_y + 5.5,
        7.0,
        rgba(if typing { 0xffffff } else { 0x5a5a70 }, a),
        &shown,
    );
    c.hit(inner, input_y, iw, 18.0, Hit::TextField);
    // The last F12's, on its way to whoever this is with.
    if s.shot_ready && !typing {
        let label = tr!("Send screenshot", "スクショを送信", "Отправить скриншот");
        button(c, inner + iw - 3.0, input_y + 3.0, label, Hit::SendShot, a);
    }
}

/// A label shaped like a button but inert, ending at `right`; returns its left edge.
fn tag(c: &mut Canvas, right: f32, y: f32, label: &str, a: f32) -> f32 {
    let w = Canvas::text_width(6.0, label) + 10.0;
    let x = right - w;
    c.round_rect(
        [x, y],
        [w, 12.0],
        6.0,
        rgba(0x000000, 0.0),
        rgba(0x30303f, a),
        1.0,
    );
    c.text(x + 5.0, y + 3.0, 6.0, rgba(DIM_TEXT, a), label);
    x
}

/// A small button ending at `right`; returns its left edge.
fn button(c: &mut Canvas, right: f32, y: f32, label: &str, hit: Hit, a: f32) -> f32 {
    let w = Canvas::text_width(6.0, label) + 12.0;
    let x = right - w;
    let hover = c.hovered(x, y, w, 12.0);
    c.round_rect(
        [x, y],
        [w, 12.0],
        3.5,
        rgba(if hover { accent() } else { 0x2a2a3c }, a),
        rgba(if hover { accent() } else { 0x3e3e54 }, a),
        1.0,
    );
    c.text(x + 6.0, y + 3.0, 6.0, rgba(0xffffff, a), label);
    c.hit(x, y, w, 12.0, hit);
    x
}

fn badge_label(n: usize) -> String {
    if n > 9 {
        "9+".to_string()
    } else {
        n.to_string()
    }
}

/// How wide a badge for `n` is (0 when there's none).
fn badge_width(n: usize) -> f32 {
    if n == 0 {
        0.0
    } else {
        (Canvas::text_width(6.0, &badge_label(n)) + 6.0).max(10.0)
    }
}

/// A count in a pill from `left`, centred on `cy`; returns its width (0 for none).
fn badge_after(c: &mut Canvas, left: f32, cy: f32, n: usize, a: f32) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let w = badge_width(n);
    c.round_rect(
        [left, cy - 5.0],
        [w, 10.0],
        5.0,
        rgba(accent(), a),
        rgba(accent(), a),
        0.0,
    );
    c.text_centered(
        left + w / 2.0,
        cy - 3.0,
        6.0,
        rgba(0xffffff, a),
        &badge_label(n),
    );
    w
}

/// A count in a pill centred on (`cx`, `cy`).
fn badge(c: &mut Canvas, cx: f32, cy: f32, n: usize, a: f32) {
    badge_after(c, cx - badge_width(n) / 2.0, cy, n, a);
}

/// The notification cards, stacked at the top right (newest on top); clicking one acts on it.
pub fn draw_notes(c: &mut Canvas, s: &Social, editor: bool) {
    let w = 210.0;
    // Clear of the tab, which rides on the overlay when it's open.
    let right = if s.open {
        panel_rect(c.width)[0]
    } else {
        c.width
    };
    let mut x = right - TAB_W - w - 8.0;
    let mut y = if s.open { 30.0 } else { 58.0 };
    if editor {
        // No tab here: over the preview at the left, clear of the menus, modes and inspector.
        x = EDITOR_NOTES_X;
        y = EDITOR_NOTES_Y;
    }
    for (i, n) in s.notes.iter().enumerate().rev() {
        let age = n.at.elapsed().as_secs_f32();
        let a = (age / 0.15).min(1.0) * ((NOTE_SECS - age) / 0.5).clamp(0.0, 1.0);
        let slide = (1.0 - (age / 0.2).min(1.0)) * 24.0;
        let h = 30.0;
        let hover = c.hovered(x, y, w, h);
        c.round_rect(
            [x + slide, y],
            [w, h],
            6.0,
            rgba(if hover { 0x1d1d27 } else { 0x0b0b12 }, 0.86 * a),
            rgba(n.colour, a),
            1.0,
        );
        c.rect(x + slide + 1.0, y + 4.0, 2.0, h - 8.0, rgba(n.colour, a));
        c.text(
            x + slide + 9.0,
            y + 6.0,
            7.0,
            rgba(n.colour, a),
            &Canvas::fit(7.0, w - 15.0, &n.title),
        );
        c.text(
            x + slide + 9.0,
            y + 17.0,
            6.5,
            rgba(0xdedeea, a),
            &Canvas::fit(6.5, w - 15.0, &n.body),
        );
        if n.action != NoteAction::None {
            c.hit(x, y, w, h, Hit::SocialNote(i));
        }
        y += h + 4.0;
    }
}

/// Where the notification cards sit in the editor's scenes: over the left preview.
pub const EDITOR_NOTES_X: f32 = 16.0;
pub const EDITOR_NOTES_Y: f32 = 70.0;

/// The editor's way in to the panel: a chip in its menu bar with `x` its left edge (the bar is
/// 14 tall), who is online and a badge for what is unread. Returns its width.
pub fn draw_editor_chip(c: &mut Canvas, s: &Social, x: f32, lobby_unread: usize) -> f32 {
    let unread = s.unread_total() + lobby_unread + s.requests.len();
    let label = if s.connected {
        format!("F9 friends {}", s.online_count())
    } else {
        "F9 friends".into()
    };
    let (y, h) = (2.0, 10.0);
    let w = Canvas::text_width(6.5, &label) + 10.0;
    let hover = c.hovered(x, y, w, h);
    let (bg, edge) = if s.open {
        (0x4a221b, accent())
    } else if hover {
        (0x2a2a3a, 0x6a6a88)
    } else {
        (0x15151d, 0x3a3a4e)
    };
    c.round_rect([x, y], [w, h], h / 2.0, rgba(bg, 1.0), rgba(edge, 1.0), 1.0);
    c.text(x + 5.0, y + 2.0, 6.5, rgba(0xb4b4c6, 1.0), &label);
    c.hit(x, y, w, h, Hit::SocialToggle);
    badge(c, x + w, y + 1.0, unread, 1.0);
    w
}

/// The overlay panel's rectangle (for clicks): x, y, w, h.
pub fn panel_rect(width: f32) -> [f32; 4] {
    [width - PANEL_W, 24.0, PANEL_W, 480.0 - 30.0]
}

/// The tab that opens the overlay (menus, when logged in), at the edge of the screen.
pub fn draw_chip(c: &mut Canvas, s: &Social, middle: f32, lobby_unread: usize) {
    draw_tab(c, s, c.width, middle, lobby_unread);
}

/// The tab with its right side at `right` and its middle at `middle`, with unread messages and
/// requests on it: its key upright at the top, and who's online reading up it below that. It's
/// drawn past `right`, for the panel or the screen's edge to cut it off.
fn draw_tab(c: &mut Canvas, s: &Social, right: f32, middle: f32, lobby_unread: usize) {
    let unread = s.unread_total() + lobby_unread + s.requests.len();
    let label = if s.connected {
        format!("friends {}", s.online_count())
    } else {
        "friends".into()
    };
    // Everyone online first, after a green dot (once the server says).
    let online = s
        .players
        .as_ref()
        .filter(|_| s.connected)
        .map(|_| format!("{} online   ", s.online_total));
    let online_w = online
        .as_deref()
        .map_or(0.0, |t| Canvas::text_width(TAB_TEXT, t) + 9.0);
    let reading = Canvas::text_width(TAB_TEXT, &label) + online_w;
    let len = TAB_PAD + TAB_KEY_ROW + reading + TAB_PAD;
    let (x, y) = (right - TAB_W, middle - len / 2.0);
    let hover = c.hovered(x, y, TAB_W, len);
    c.round_rect(
        [x, y],
        [TAB_W + EDGE_BLEED, len],
        6.0,
        rgba(if hover { 0x2b2b3d } else { 0x15151d }, 0.92),
        rgba(0x3a3a4e, 1.0),
        1.0,
    );
    let cx = x + TAB_W / 2.0;
    let colour = rgba(0xc8c8d8, 1.0);
    c.text_centered(cx, y + TAB_PAD, TAB_TEXT, colour, "F9");
    // Laid out along a line from (0, 0), then stood up to read from the bottom.
    let start = c.shapes.len();
    let mut along = 0.0;
    if let Some(online) = &online {
        c.circle(2.5, 0.0, 2.5, rgba(GREEN, 1.0), rgba(0x000000, 0.0), 0.0);
        c.text(9.0, -TAB_TEXT / 2.0, TAB_TEXT, colour, online);
        along = online_w;
    }
    c.text(along, -TAB_TEXT / 2.0, TAB_TEXT, colour, &label);
    let dest = [cx, y + len - TAB_PAD];
    c.rotate_since(start, [0.0, 0.0], -std::f32::consts::FRAC_PI_2, 1.0, dest);
    c.hit(x, y, TAB_W, len, Hit::SocialToggle);
    badge(c, x + 3.0, y + 1.0, unread, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshots_link_in_the_chat_is_a_row_to_click() {
        let id = 0xabc_u128;
        let url = crate::shots::shot_url(id);
        let entry = |text: &str| ChatEntry {
            from: "bob".into(),
            text: text.into(),
            at: None,
            mine: false,
            banner: None,
            mention: false,
        };
        let rows = chat_rows(&[entry(&url), entry("hi")], 40, 40);
        assert!(rows.iter().any(|r| r.kind == ChatKind::Shot(id)));
        assert!(
            !rows.iter().any(|r| r.text.contains("/shot/")),
            "no raw link"
        );
        assert!(
            rows.iter().any(|r| r.text == "hi"),
            "what follows is as ever"
        );
    }

    /// The tab's hit area with the overlay `open` or not, on a screen 853 wide.
    fn tab_rect(s: &Social, open: bool) -> [f32; 4] {
        let mut c = Canvas::default();
        c.clear(853.0);
        if open {
            draw(&mut c, s, true, None, 0, true, false, Some(TAB_MIDDLE));
        } else {
            draw_chip(&mut c, s, TAB_MIDDLE, 0);
        }
        c.hits
            .iter()
            .find(|h| h.hit == Hit::SocialToggle)
            .expect("the tab can be clicked")
            .rect
    }

    #[test]
    fn the_tab_glides_to_a_new_height_and_doesnt_overshoot() {
        let mut s = Social::default();
        assert_eq!(s.glide_tab(TAB_MENU_MIDDLE, 16.0, true), TAB_MENU_MIDDLE);
        let mut y = TAB_MENU_MIDDLE;
        let step = s.glide_tab(TAB_MIDDLE, 16.0, true);
        assert!(step < y && step > TAB_MIDDLE, "on its way: {step}");
        for _ in 0..200 {
            let next = s.glide_tab(TAB_MIDDLE, 16.0, true);
            assert!(next <= y && next >= TAB_MIDDLE);
            y = next;
        }
        assert_eq!(y, TAB_MIDDLE);
        // Without animations, it's just there.
        assert_eq!(s.glide_tab(TAB_MENU_MIDDLE, 16.0, false), TAB_MENU_MIDDLE);
    }

    #[test]
    fn the_tab_stands_on_the_edge_and_rides_on_the_open_panel() {
        crate::render::set_animations(false);
        let mut s = Social::default();
        let [x, _, w, h] = tab_rect(&s, false);
        assert_eq!(x + w, 853.0, "against the edge of the screen");
        assert!(h > w, "taller than it's wide");
        s.open = true;
        let [x, _, w, _] = tab_rect(&s, true);
        assert_eq!(x + w, panel_rect(853.0)[0], "on the panel's left side");
    }

    #[test]
    fn the_server_speaks_on_a_banner_in_the_medals_colour() {
        let medal = online::ChatMedal {
            tier: 4,
            ..Default::default()
        };
        let secret = online::ChatMedal {
            tier: 4,
            secret: true,
            ..Default::default()
        };
        let said = |from: &str, medal: Option<&online::ChatMedal>| {
            ChatEntry::new(from, "a b c d e f".into(), false, medal, None, false)
        };

        let rows = chat_rows(&[said(online::SERVER_NAME, Some(&medal))], 40, 5);
        assert_eq!(
            rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["a b c", "d e f"],
            "no name over it"
        );
        assert!(
            rows.iter()
                .all(|r| r.colour == online::medal_colour(4, false))
        );
        assert_eq!(
            rows.iter().map(|r| &r.kind).collect::<Vec<_>>(),
            [
                &ChatKind::Banner { ends: false },
                &ChatKind::Banner { ends: true }
            ]
        );

        assert_eq!(
            chat_rows(&[said(online::SERVER_NAME, Some(&secret))], 40, 40)[0].colour,
            online::SECRET_COLOUR
        );
        assert_eq!(
            chat_rows(&[said(online::SERVER_NAME, None)], 40, 40)[0].colour,
            SERVER_COLOUR
        );

        let person = chat_rows(&[said("auneu", Some(&medal))], 40, 5);
        assert_eq!(
            person.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            ["auneu", "a b c d e f"],
            "the name over the message"
        );
        assert!(
            matches!(person[0].kind, ChatKind::Name { .. })
                && matches!(person[1].kind, ChatKind::Text { .. })
        );
    }

    #[test]
    fn groups_come_from_the_server_whole_and_by_any_case() {
        let mut s = Social::default();
        let tags = |t: &[&str]| t.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        s.set_groups(vec![
            ("Auneu".into(), tags(&["DEV", "200"])),
            ("nobody".into(), Vec::new()),
        ]);
        assert_eq!(s.tags_of("AUNEU"), ["DEV", "200"]);
        assert!(s.tags_of("nobody").is_empty() && s.tags_of("anyone").is_empty());
        // The next list replaces it.
        s.set_groups(vec![("semfi".into(), tags(&["200"]))]);
        assert!(s.tags_of("auneu").is_empty());
        assert_eq!(s.tags_of("semfi"), ["200"]);
    }

    #[test]
    fn group_tags_say_three_characters_at_most() {
        assert_eq!(tag_label("DEV"), "DEV");
        assert_eq!(tag_label("LOVED"), "LVD");
        assert_eq!(tag_label("200"), "200");
        assert_eq!(tag_label("SOMETHING NEW"), "SOM");
        assert_eq!(tag_label("ÄÖÜß"), "ÄÖÜ");
        assert_eq!(tag_label("GM"), "GM");
        // The colours still go by the whole group.
        assert_eq!(group_colours("LOVED"), (0xFF66AA, 0xC39BFF));
    }

    #[test]
    fn a_shimmer_stays_near_its_colours_and_moves_slowly() {
        let (a, b) = group_colours("DEV");
        let channels = |c: u32| [(c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF];
        let (ca, cb) = (channels(a), channels(b));
        for step in 0..200 {
            let secs = step as f32 * 0.05;
            for i in 0..8 {
                let c = channels(shimmer((a, b), i, 8, secs));
                for k in 0..3 {
                    // Between the two colours, or lifted a little towards white by the glint.
                    let lo = ca[k].min(cb[k]);
                    let hi =
                        (ca[k].max(cb[k]) as f32 + (255 - ca[k].max(cb[k])) as f32 * 0.33) as u32;
                    assert!(c[k] + 1 >= lo && c[k] <= hi + 1, "{c:?} at {secs}");
                }
            }
        }
        // It moves, but a frame later a letter has barely changed: no flicker.
        let at = |secs: f32| channels(shimmer((a, b), 3, 8, secs));
        assert_ne!(at(0.0), at(1.0));
        let (x, y) = (at(1.0), at(1.0 + 1.0 / 60.0));
        assert!((0..3).all(|k| x[k].abs_diff(y[k]) <= 6), "{x:?} {y:?}");
        // A tag from a newer server still gets a colour.
        assert_ne!(group_colours("SOMETHING NEW"), group_colours("DEV"));
    }

    #[test]
    fn a_run_of_messages_from_one_sender_shares_the_name() {
        let line =
            |from: &str, at: u64| ChatEntry::new(from, "hi".into(), false, None, Some(at), false);
        let rows = chat_rows(
            &[
                line("ann", 1000),
                line("ann", 1060),
                line("bob", 1070),
                line("ann", 1080),
                line("ann", 1080 + GROUP_SECS),
            ],
            40,
            40,
        );
        let names: Vec<&str> = rows
            .iter()
            .filter(|r| matches!(r.kind, ChatKind::Name { .. }))
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(
            names,
            ["ann", "bob", "ann", "ann"],
            "a new name for another sender, and after a long pause"
        );
        assert_eq!(rows.len(), 4 + 5);
        assert_eq!(rows[0].gap, 0.0, "nothing above the first");
        assert!(rows[3].gap > rows[2].gap, "another sender starts apart");
        assert_eq!(name_colour("Ann", false), name_colour("ann", false));
        assert_eq!(name_colour("ann", true), accent());
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(ago(100, 130), "now");
        assert_eq!(ago(100, 100 + 5 * 60), "5m");
        assert_eq!(ago(0, 3 * 3600 + 5), "3h");
        assert_eq!(ago(0, 2 * 86400), "2d");
        assert_eq!(ago(200, 100), "now", "a clock behind the server's");
    }

    fn friend(name: &str, online: bool) -> Friend {
        Friend {
            name: name.into(),
            activity: online.then_some(Activity::Menus),
        }
    }

    #[test]
    fn friends_online_come_first_and_the_selection_follows_its_friend() {
        let mut s = Social {
            friends: vec![
                friend("ann", false),
                friend("bob", true),
                friend("cid", false),
            ],
            ..Social::default()
        };
        assert_eq!(
            s.rows(false),
            vec![
                Row::AddFriend,
                Row::Friend(1),
                Row::Friend(0),
                Row::Friend(2)
            ]
        );
        // Talking to cid, ann comes online and the list reorders: still cid.
        s.cursor = 3;
        s.open_conversation();
        assert!(s.in_conversation(false));
        s.set_friends(
            vec![
                friend("ann", true),
                friend("bob", true),
                friend("cid", false),
            ],
            Vec::new(),
            Vec::new(),
            false,
        );
        assert_eq!(s.row(false), Row::Friend(2));
        assert!(s.in_conversation(false));
        // cid unfriends us: back to the list.
        s.set_friends(
            vec![friend("ann", true), friend("bob", true)],
            Vec::new(),
            Vec::new(),
            false,
        );
        assert!(!s.in_conversation(false));
        assert!(s.cursor < s.rows(false).len());
    }

    #[test]
    fn a_conversation_is_read_only_once_open() {
        let mut s = Social {
            friends: vec![friend("ann", true)],
            ..Social::default()
        };
        s.unread.insert("ann".into(), 3);
        s.cursor = 1;
        s.mark_read(false);
        assert_eq!(s.unread_total(), 3, "selected in the list, not read yet");
        s.select_friend("Ann", false);
        s.mark_read(false);
        assert_eq!(s.unread_total(), 0);
        s.close_conversation();
        assert!(!s.in_conversation(false) && s.row(false) == Row::Friend(0));
    }

    #[test]
    fn the_players_list_keeps_the_selected_player() {
        let mut s = Social {
            tab: Tab::Players,
            ..Social::default()
        };
        s.set_players(3, vec![player("a"), player("b"), player("c")], false);
        s.cursor = 3;
        s.set_players(3, vec![player("x"), player("c"), player("a")], false);
        assert_eq!(s.row(false), Row::Player(1));
    }

    #[test]
    fn wraps_at_spaces_and_cuts_long_words() {
        assert_eq!(
            wrap("hello there friend", 11),
            vec!["hello there", "friend"]
        );
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 5), vec![""]);
        assert_eq!(wrap("привет как дела", 10), vec!["привет как", "дела"]);
        // Japanese is 4/3 as wide and has no spaces: 6 to an 8-character line.
        assert_eq!(
            wrap("こんにちは、元気ですか", 8),
            vec!["こんにちは、", "元気ですか"]
        );
    }

    fn player(name: &str) -> OnlinePlayer {
        OnlinePlayer {
            name: name.into(),
            activity: Activity::Menus,
        }
    }

    #[test]
    fn players_tab_filters_and_tabs_keep_their_rows() {
        let mut s = Social {
            me: Some("Lynerrr".into()),
            friends: vec![Friend {
                name: "den4ik".into(),
                activity: Some(Activity::Menus),
            }],
            requests: vec!["Kat".into()],
            ..Social::default()
        };
        s.players = Some(vec![
            player("den4ik"),
            player("Kat"),
            player("Lynerrr"),
            player("zzz"),
        ]);
        s.cursor = 2;
        s.set_tab(Tab::Players);
        assert_eq!(s.cursor, 0);
        assert_eq!(
            s.rows(false),
            vec![
                Row::Search,
                Row::Player(0),
                Row::Player(1),
                Row::Player(2),
                Row::Player(3)
            ]
        );
        s.input = "Ka".into();
        assert_eq!(s.rows(false), vec![Row::Search, Row::Player(1)]);
        assert_eq!(s.standing("kat"), Standing::AsksUs);
        assert_eq!(s.standing("DEN4IK"), Standing::Friend);
        assert_eq!(s.standing("lynerrr"), Standing::Me);
        assert_eq!(s.standing("zzz"), Standing::Stranger);
        s.cycle_tab(false);
        assert_eq!((s.tab, s.rows(false)), (Tab::Chat, vec![Row::Global]));
        assert!(s.input.is_empty());
        s.cycle_tab(false);
        assert_eq!((s.tab, s.cursor), (Tab::Friends, 2));
        s.cycle_tab(true);
        assert_eq!(s.tab, Tab::Chat);
    }

    #[test]
    fn mentions_need_the_whole_name() {
        let s = Social {
            me: Some("Kat".into()),
            ..Social::default()
        };
        assert!(s.mentions_me("hi kat!") && s.mentions_me("KAT, play?"));
        assert!(!s.mentions_me("katana") && !s.mentions_me("hello"));
    }

    #[test]
    fn wheel_scrolls_the_chat_within_its_history() {
        let mut s = Social {
            tab: Tab::Chat,
            ..Social::default()
        };
        s.chat_scroll_max.set(4);
        s.wheel(1, false);
        assert_eq!(s.chat_scroll, 3);
        s.wheel(5, false);
        assert_eq!(s.chat_scroll, 4);
        s.wheel(-10, false);
        assert_eq!(s.chat_scroll, 0);
    }
}
