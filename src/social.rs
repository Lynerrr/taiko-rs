//! Friends and chat: the state behind the F9 overlay, and drawing it. Friends chat over the
//! social connection to the online server (no lobby needed); in a lobby there's also its chat.
//! Besides friends the overlay lists everyone online (to add them) and has a global chat.

use std::cell::Cell;
use std::collections::HashMap;
use std::time::Instant;

use crate::online::{self, Activity, ChatLine, Friend, OnlinePlayer};
use crate::render::{accent, rgba, Canvas, Hit};

/// The overlay's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Friends,
    /// Everyone online.
    Players,
    /// The global chat.
    Chat,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Friends, Tab::Players, Tab::Chat];

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Friends => "FRIENDS",
            Tab::Players => "PLAYERS",
            Tab::Chat => "CHAT",
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
    pub unread: HashMap<String, usize>,
    /// Everyone online (None until the server says: older servers don't).
    pub players: Option<Vec<OnlinePlayer>>,
    /// How many are online (the list stops at `online::MAX_LISTED`).
    pub online_total: usize,
    /// The global chat, oldest first.
    pub global: Vec<ChatLine>,
    pub global_unread: usize,
    pub open: bool,
    pub opened_at: Option<Instant>,
    /// When it last closed (it slides away).
    pub closed_at: Option<Instant>,
    pub tab: Tab,
    /// The selected row of the shown tab.
    pub cursor: usize,
    /// The other tabs' cursors, by `Tab::index`.
    tab_cursors: [usize; 3],
    /// What's typed: a friend's name on the add row, the filter on the players tab, else a message.
    pub input: String,
    /// Chat lines scrolled back from the newest, and the most that can be (set when drawn).
    pub chat_scroll: usize,
    chat_scroll_max: Cell<usize>,
    /// The activity last sent to the server.
    pub sent_activity: Option<Activity>,
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
            let room = if len == 0 { width } else { width.saturating_sub(len + crate::font::advance(' ')) };
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
pub const SLIDE_MS: f32 = 160.0;

impl Social {
    /// Closes the overlay; it slides away (`closing`).
    pub fn close(&mut self) {
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

    pub fn notify(&mut self, title: impl Into<String>, body: impl Into<String>, colour: u32, action: NoteAction) {
        self.notes.push(Note { title: title.into(), body: body.into(), colour, action, at: Instant::now() });
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
            self.notes.retain(|n| n.at.elapsed().as_secs_f32() < NOTE_SECS);
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
                rows.extend((0..self.friends.len()).map(Row::Friend));
                rows
            }
            Tab::Players => {
                let filter = key(self.input.trim());
                let players = self.players.as_deref().unwrap_or(&[]);
                let mut rows = vec![Row::Search];
                rows.extend(players.iter().enumerate().filter(|(_, p)| key(&p.name).contains(&filter)).map(|(i, _)| Row::Player(i)));
                rows
            }
            Tab::Chat => vec![Row::Global],
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

    /// The next (or previous) tab.
    pub fn cycle_tab(&mut self, back: bool) {
        let n = Tab::ALL.len();
        let i = (self.tab.index() + if back { n - 1 } else { 1 }) % n;
        self.set_tab(Tab::ALL[i]);
    }

    /// Mouse wheel over the overlay at `vy`: scrolls the chat under the pointer, else the list.
    pub fn wheel(&mut self, notches: i32, vy: f32, in_lobby: bool) {
        let over_chat = match self.tab {
            Tab::Friends => vy >= panel_rect(0.0)[1] + CHAT_TOP,
            Tab::Players => false,
            Tab::Chat => true,
        };
        if over_chat {
            let max = self.chat_scroll_max.get() as i32;
            self.chat_scroll = (self.chat_scroll as i32 + notches * 3).clamp(0, max) as usize;
        } else {
            let rows = self.rows(in_lobby).len() as i32;
            self.cursor = (self.cursor.min(rows as usize - 1) as i32 - notches).clamp(0, rows - 1) as usize;
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

    /// Selects the conversation with `name` (opening the overlay on it).
    pub fn select_friend(&mut self, name: &str, in_lobby: bool) {
        self.set_tab(Tab::Friends);
        if let Some(i) = self.friends.iter().position(|f| key(&f.name) == key(name)) {
            if let Some(r) = self.rows(in_lobby).iter().position(|r| *r == Row::Friend(i)) {
                self.cursor = r;
            }
        }
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
            Row::Friend(i) => {
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
        let Some(me) = self.me.as_deref().map(key) else { return false };
        text.to_lowercase().split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).any(|w| w == me)
    }
}

const PANEL_W: f32 = 290.0;
/// Where the friends tab's conversation starts, below the panel's top.
const CHAT_TOP: f32 = 236.0;
const GREEN: u32 = 0x86E05A;
/// Characters per chat line (drawn at 6.5 between 4 px margins in the panel's 8 px padding).
pub fn chat_width() -> usize {
    ((PANEL_W - 24.0) / Canvas::char_width(6.5)) as usize
}

/// The overlay: a translucent panel on the right over whatever screen is showing.
/// `lobby`: the room's chat when in one (name, line).
pub fn draw(c: &mut Canvas, s: &Social, logged_in: bool, lobby: Option<&[(String, String)]>, lobby_unread: usize) {
    let slide = s.opened_at.map(|t| crate::render::progress(t, SLIDE_MS)).unwrap_or(1.0);
    let x = c.width - PANEL_W - 6.0 + (1.0 - slide) * 30.0;
    let (y, h) = (24.0, 480.0 - 30.0);
    let a = slide;
    c.round_rect(x, y, PANEL_W, h, 8.0, rgba(0x0b0b12, 0.84 * a), rgba(0x3a3a4e, 0.9 * a), 1.0);
    c.hit(x, y, PANEL_W, h, Hit::SocialPanel);
    let inner = x + 8.0;
    let iw = PANEL_W - 16.0;
    if !logged_in {
        c.text(x + 12.0, y + 9.0, 9.0, rgba(accent(), a), "FRIENDS");
        c.text_right(x + PANEL_W - 12.0, y + 10.0, 7.0, rgba(0x8a8aa0, a), "F9");
        c.text(inner + 4.0, y + 34.0, 7.0, rgba(0x9a9ab0, a), "Log in under MULTIPLAYER > ONLINE");
        c.text(inner + 4.0, y + 46.0, 7.0, rgba(0x9a9ab0, a), "to add friends and chat.");
        return;
    }
    // The tabs, and how many are online in the corner.
    let mut tx = x + 12.0;
    for tab in Tab::ALL {
        let label = tab.label();
        let tw = Canvas::text_width(8.0, label);
        let sel = tab == s.tab;
        let hover = c.hovered(tx - 4.0, y + 4.0, tw + 8.0, 16.0);
        c.text(tx, y + 9.0, 8.0, rgba(if sel { accent() } else if hover { 0xdedeea } else { 0x8a8aa0 }, a), label);
        if sel {
            c.rect(tx, y + 19.0, tw, 1.5, rgba(accent(), a));
        }
        c.hit(tx - 4.0, y + 4.0, tw + 8.0, 16.0, Hit::SocialTab(tab));
        let unread = match tab {
            Tab::Friends => s.unread_total() + s.requests.len() + lobby_unread,
            Tab::Players => 0,
            Tab::Chat => s.global_unread,
        };
        badge(c, tx + tw + 4.0, y + 8.0, unread, a);
        tx += tw + 18.0;
    }
    if s.connected && s.players.is_some() {
        let right = x + PANEL_W - 12.0;
        // What fits after the tabs (and their badges): "1234 online", "1234" or nothing.
        let room = right - (tx - 18.0 + 14.0) - 10.0;
        let label = [format!("{} online", s.online_total), s.online_total.to_string()].into_iter().find(|l| Canvas::text_width(7.0, l) <= room);
        if let Some(label) = label {
            c.text_right(right, y + 10.0, 7.0, rgba(0xc8c8d8, a), &label);
            let dot = right - Canvas::text_width(7.0, &label) - 6.0;
            c.circle(dot, y + 13.5, 3.0, rgba(GREEN, a), rgba(0x000000, 0.0), 0.0);
        }
    }
    if !s.connected {
        c.text(inner + 4.0, y + 28.0, 7.0, rgba(0x9a9ab0, a), "connecting to the server...");
    }
    let in_lobby = lobby.is_some();
    let rows = s.rows(in_lobby);
    let cursor = s.cursor.min(rows.len() - 1);
    let selected = rows[cursor];
    let input_y = y + h - 40.0;
    let width = chat_width();
    let list_top = y + 28.0 + if s.connected { 0.0 } else { 14.0 };
    let me = s.me.as_deref().map(key);

    match s.tab {
        Tab::Friends => {
            let list_bottom = y + CHAT_TOP - 6.0;
            let mut ry = draw_rows(c, s, &rows, cursor, inner, iw, list_top, list_bottom, lobby_unread, a);
            if s.friends.is_empty() && s.requests.is_empty() && s.connected {
                c.text(inner + 4.0, ry + 4.0, 7.0, rgba(0x6f6f86, a), "no friends yet: add one by name or in PLAYERS");
                ry += 14.0;
            }
            if !s.sent.is_empty() && ry + 10.0 < list_bottom {
                c.text(inner + 4.0, ry + 3.0, 6.0, rgba(0x6f6f86, a), &Canvas::fit(6.0, iw - 8.0, &format!("waiting for: {}", s.sent.join(", "))));
            }
            // The conversation.
            let chat_top = y + CHAT_TOP;
            c.rect(inner, chat_top - 2.0, iw, 1.0, rgba(0x2a2a38, a));
            let lines: Vec<(String, String, bool)> = match s.conversation(in_lobby) {
                Some(Ok(f)) => {
                    c.text(inner + 2.0, chat_top + 4.0, 7.0, rgba(0x9a9ab0, a), &format!("chat with {}", f.name));
                    s.chats.get(&key(&f.name)).map(|v| v.iter().map(|l| (l.from.clone(), l.text.clone(), Some(key(&l.from)) == me)).collect()).unwrap_or_default()
                }
                Some(Err(())) => {
                    c.text(inner + 2.0, chat_top + 4.0, 7.0, rgba(0x4fb4ff, a), "# lobby chat");
                    lobby.unwrap_or(&[]).iter().map(|(from, text)| (from.clone(), text.clone(), false)).collect()
                }
                None => {
                    c.text(inner + 2.0, chat_top + 4.0, 7.0, rgba(0x6f6f86, a), "pick a friend to chat with");
                    Vec::new()
                }
            };
            chat_lines(c, s, &lines, inner, chat_top + 16.0, input_y, width, a);
            if matches!(selected, Row::Friend(_) | Row::Lobby) {
                message_box(c, s, inner, iw, input_y, "type a message, Enter to send", a);
            }
            c.text_centered(x + PANEL_W / 2.0, y + h - 14.0, 6.0, rgba(0x55556a, a), &Canvas::fit_hints(6.0, iw, "Tab  Up/Down  Ctrl+W watch  Ctrl+J join  Ctrl+P profile  Del remove  Esc"));
        }
        Tab::Players => {
            let ry = draw_rows(c, s, &rows, cursor, inner, iw, list_top, y + h - 24.0, lobby_unread, a);
            if s.connected && s.players.is_none() {
                c.text(inner + 4.0, ry + 4.0, 7.0, rgba(0x6f6f86, a), "this server doesn't list players yet");
            } else if rows.len() == 1 && s.connected {
                c.text(inner + 4.0, ry + 4.0, 7.0, rgba(0x6f6f86, a), "nobody online by that name");
            }
            c.text_centered(x + PANEL_W / 2.0, y + h - 14.0, 6.0, rgba(0x55556a, a), &Canvas::fit_hints(6.0, iw, "Tab  type to search  Up/Down  Enter add friend  Esc"));
        }
        Tab::Chat => {
            let lines: Vec<(String, String, bool)> = s.global.iter().map(|l| (l.from.clone(), l.text.clone(), Some(key(&l.from)) == me)).collect();
            if lines.is_empty() && s.connected {
                let note = if s.players.is_some() { "nobody has said anything yet: say hi" } else { "this server has no global chat yet" };
                c.text(inner + 4.0, list_top + 2.0, 7.0, rgba(0x6f6f86, a), note);
            }
            chat_lines(c, s, &lines, inner, list_top + 12.0, input_y, width, a);
            message_box(c, s, inner, iw, input_y, "message everyone online, Enter to send", a);
            c.text_centered(x + PANEL_W / 2.0, y + h - 14.0, 6.0, rgba(0x55556a, a), &Canvas::fit_hints(6.0, iw, "Tab  Enter send  wheel or PgUp/PgDn scroll  Esc"));
        }
    }
}

/// The rows of a list from `top`, scrolled so the cursor stays visible; returns where they end.
#[allow(clippy::too_many_arguments)]
fn draw_rows(c: &mut Canvas, s: &Social, rows: &[Row], cursor: usize, inner: f32, iw: f32, top: f32, bottom: f32, lobby_unread: usize, a: f32) -> f32 {
    let height = |r: &Row| if matches!(r, Row::Friend(_) | Row::Player(_)) { 28.0 } else { 22.0 };
    let mut first = 0;
    while rows[first..=cursor].iter().map(height).sum::<f32>() > bottom - top {
        first += 1;
    }
    let mut ry = top;
    for (i, r) in rows.iter().enumerate().skip(first) {
        let rh = height(r);
        if ry + rh > bottom {
            break;
        }
        let sel = i == cursor;
        let hover = c.hovered(inner, ry, iw, rh - 3.0);
        let bg = if sel { 0x2b2b3d } else if hover { 0x1d1d27 } else { 0x15151d };
        c.round_rect(inner, ry, iw, rh - 3.0, 5.0, rgba(bg, 0.85 * a), rgba(if sel { accent() } else { 0x2a2a38 }, a), 1.0);
        c.hit(inner, ry, iw, rh - 3.0, Hit::SocialRow(i));
        let ty = ry + (rh - 3.0) / 2.0 - 3.5;
        match *r {
            Row::AddFriend => {
                // While typing, the end of the name stays in view.
                let text = if sel { Canvas::fit_end(7.0, iw - 16.0, &format!("add friend: {}_", s.input)) } else { Canvas::fit(7.0, iw - 16.0, "+ add a friend (type their name)") };
                c.text(inner + 8.0, ty, 7.0, rgba(if sel { 0xffffff } else { 0x8a8aa0 }, a), &text);
            }
            Row::Lobby => {
                c.text(inner + 8.0, ty, 8.0, rgba(0x4fb4ff, a), "# lobby chat");
                badge(c, inner + iw - 10.0, ty + 3.5, lobby_unread, a);
            }
            Row::Request(k) => {
                let name = s.requests.get(k).map(String::as_str).unwrap_or("");
                let bx = inner + iw - 4.0;
                let bx = button(c, bx, ry + 3.0, "decline", Hit::SocialAction(i, Action::Decline), a);
                let bx = button(c, bx - 4.0, ry + 3.0, "accept", Hit::SocialAction(i, Action::Accept), a);
                c.text(inner + 8.0, ty, 7.0, rgba(0xFFD34D, a), &Canvas::fit(7.0, bx - inner - 14.0, &format!("{name} wants to be friends")));
            }
            Row::Friend(k) => {
                let Some(f) = s.friends.get(k) else { continue };
                let online = f.activity.is_some();
                c.circle(inner + 10.0, ry + 8.0, 3.0, rgba(if online { GREEN } else { 0x55556a }, a), rgba(0x000000, 0.0), 0.0);
                let what = f.activity.as_ref().map(Activity::describe).unwrap_or_else(|| "offline".into());
                let mut bx = button(c, inner + iw - 4.0, ry + 3.0, "profile", Hit::SocialAction(i, Action::Profile), a) - 4.0;
                bx = match &f.activity {
                    Some(Activity::Playing { .. }) => button(c, bx, ry + 3.0, "watch", Hit::SocialAction(i, Action::Watch), a) - 4.0,
                    Some(Activity::Lobby { room: Some(_), .. }) => button(c, bx, ry + 3.0, "join", Hit::SocialAction(i, Action::Join), a) - 4.0,
                    _ => bx,
                };
                c.text(inner + 18.0, ry + 4.5, 8.0, rgba(if online { 0xffffff } else { 0x8a8aa0 }, a), &Canvas::fit(8.0, bx - inner - 22.0, &f.name));
                c.text(inner + 18.0, ry + 15.5, 6.0, rgba(0x8a8aa0, a), &Canvas::fit(6.0, bx - inner - 22.0, &what));
                badge(c, bx - 8.0, ry + 8.0, s.unread.get(&key(&f.name)).copied().unwrap_or(0), a);
            }
            Row::Search => {
                let text = format!("search: {}{}", if s.input.is_empty() && !sel { "type a name" } else { &s.input }, if sel { "_" } else { "" });
                let more = format!("first {} listed", online::MAX_LISTED);
                let capped = s.online_total > s.players.as_ref().map_or(0, Vec::len);
                let room = iw - 16.0 - if capped { Canvas::text_width(6.0, &more) + 8.0 } else { 0.0 };
                c.text(inner + 8.0, ty, 7.0, rgba(if sel || !s.input.is_empty() { 0xffffff } else { 0x8a8aa0 }, a), &Canvas::fit_end(7.0, room, &text));
                if capped {
                    c.text_right(inner + iw - 8.0, ty + 0.5, 6.0, rgba(0x6f6f86, a), &more);
                }
            }
            Row::Player(k) => {
                let Some(p) = s.players.as_ref().and_then(|v| v.get(k)) else { continue };
                c.circle(inner + 10.0, ry + 8.0, 3.0, rgba(GREEN, a), rgba(0x000000, 0.0), 0.0);
                let right = button(c, inner + iw - 4.0, ry + 3.0, "profile", Hit::SocialAction(i, Action::Profile), a) - 4.0;
                let bx = match s.standing(&p.name) {
                    Standing::Me => tag(c, right, ry + 3.0, "you", a),
                    Standing::Friend => button(c, right, ry + 3.0, "chat", Hit::SocialAction(i, Action::Chat), a),
                    Standing::AsksUs => button(c, right, ry + 3.0, "accept", Hit::SocialAction(i, Action::Accept), a),
                    Standing::Asked => tag(c, right, ry + 3.0, "request sent", a),
                    Standing::Stranger => button(c, right, ry + 3.0, "+ friend", Hit::SocialAction(i, Action::Add), a),
                } - 4.0;
                c.text(inner + 18.0, ry + 4.5, 8.0, rgba(0xffffff, a), &Canvas::fit(8.0, bx - inner - 22.0, &p.name));
                c.text(inner + 18.0, ry + 15.5, 6.0, rgba(0x8a8aa0, a), &Canvas::fit(6.0, bx - inner - 22.0, &p.activity.describe()));
            }
            Row::Global => {}
        }
        ry += rh;
    }
    ry
}

/// Chat lines (from, text, ours) between `top` and the message box at `input_y`: newest at the
/// bottom, wrapped, as many as fit, scrolled back by `s.chat_scroll` lines.
#[allow(clippy::too_many_arguments)]
fn chat_lines(c: &mut Canvas, s: &Social, lines: &[(String, String, bool)], inner: f32, top: f32, input_y: f32, width: usize, a: f32) {
    // Lines naming a map set (`/np`) are links, in blue.
    let mut wrapped: Vec<(String, u32, Option<u32>)> = Vec::new();
    for (from, text, mine) in lines {
        let set = map_set_in(text);
        let colour = if set.is_some() { 0x66ccff } else if *mine { 0xFFD34D } else { 0xffffff };
        for (n, l) in wrap(&format!("{from}: {text}"), width).into_iter().enumerate() {
            wrapped.push((if n == 0 { l } else { format!("  {l}") }, colour, set));
        }
    }
    let line_h = 10.0;
    let fit = ((input_y - top - 6.0) / line_h).max(0.0) as usize;
    let max = wrapped.len().saturating_sub(fit);
    s.chat_scroll_max.set(max);
    let back = s.chat_scroll.min(max);
    let end = wrapped.len() - back;
    let shown = &wrapped[end.saturating_sub(fit)..end];
    let mut ly = input_y - 6.0 - line_h * shown.len() as f32;
    for (l, colour, set) in shown {
        c.text(inner + 4.0, ly, 6.5, rgba(*colour, a), l);
        if let Some(set) = set {
            c.hit(inner, ly - 2.0, PANEL_W - 16.0, line_h, Hit::ChatMap(*set));
        }
        ly += line_h;
    }
    if back > 0 {
        c.text_right(inner + PANEL_W - 20.0, input_y - 4.0, 6.0, rgba(0x6f6f86, a), &format!("{back} newer lines below"));
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
    c.round_rect(inner, input_y, iw, 18.0, 5.0, rgba(0x15151d, 0.9 * a), rgba(0x3a3a4e, a), 1.0);
    let text = if s.input.is_empty() { hint.to_string() } else { format!("{}_", s.input) };
    let width = ((iw - 12.0) / Canvas::text_width(7.0, "x")) as usize;
    let shown: String = text.chars().rev().take(width).collect::<Vec<_>>().into_iter().rev().collect();
    c.text(inner + 6.0, input_y + 5.5, 7.0, rgba(if s.input.is_empty() { 0x55556a } else { 0xffffff }, a), &shown);
    c.hit(inner, input_y, iw, 18.0, Hit::TextField);
}

/// A label shaped like a button but inert, ending at `right`; returns its left edge.
fn tag(c: &mut Canvas, right: f32, y: f32, label: &str, a: f32) -> f32 {
    let w = Canvas::text_width(6.0, label) + 10.0;
    let x = right - w;
    c.round_rect(x, y, w, 12.0, 4.0, rgba(0x15151d, a), rgba(0x2a2a38, a), 1.0);
    c.text(x + 5.0, y + 3.0, 6.0, rgba(0x8a8aa0, a), label);
    x
}

/// A small button ending at `right`; returns its left edge.
fn button(c: &mut Canvas, right: f32, y: f32, label: &str, hit: Hit, a: f32) -> f32 {
    let w = Canvas::text_width(6.0, label) + 10.0;
    let x = right - w;
    let hover = c.hovered(x, y, w, 12.0);
    c.round_rect(x, y, w, 12.0, 4.0, rgba(if hover { 0x3a3a4e } else { 0x23232f }, a), rgba(0x4a4a5e, a), 1.0);
    c.text(x + 5.0, y + 3.0, 6.0, rgba(0xffffff, a), label);
    c.hit(x, y, w, 12.0, hit);
    x
}

fn badge(c: &mut Canvas, cx: f32, cy: f32, n: usize, a: f32) {
    if n == 0 {
        return;
    }
    let label = if n > 9 { "9+".to_string() } else { n.to_string() };
    let w = Canvas::text_width(6.0, &label) + 6.0;
    c.round_rect(cx - w / 2.0, cy - 5.0, w, 10.0, 5.0, rgba(accent(), a), rgba(accent(), a), 0.0);
    c.text_centered(cx, cy - 3.0, 6.0, rgba(0xffffff, a), &label);
}

/// The notification cards, stacked under the chip (newest on top); clicking one acts on it.
pub fn draw_notes(c: &mut Canvas, s: &Social) {
    let w = 210.0;
    // Beside the overlay when it's open, else under the chip.
    let x = if s.open { panel_rect(c.width)[0] - w - 8.0 } else { c.width - w - 8.0 };
    let mut y = if s.open { 30.0 } else { 58.0 };
    for (i, n) in s.notes.iter().enumerate().rev() {
        let age = n.at.elapsed().as_secs_f32();
        let a = (age / 0.15).min(1.0) * ((NOTE_SECS - age) / 0.5).clamp(0.0, 1.0);
        let slide = (1.0 - (age / 0.2).min(1.0)) * 24.0;
        let h = 30.0;
        let hover = c.hovered(x, y, w, h);
        c.round_rect(x + slide, y, w, h, 6.0, rgba(if hover { 0x1d1d27 } else { 0x0b0b12 }, 0.86 * a), rgba(n.colour, a), 1.0);
        c.rect(x + slide + 1.0, y + 4.0, 2.0, h - 8.0, rgba(n.colour, a));
        c.text(x + slide + 9.0, y + 6.0, 7.0, rgba(n.colour, a), &Canvas::fit(7.0, w - 15.0, &n.title));
        c.text(x + slide + 9.0, y + 17.0, 6.5, rgba(0xdedeea, a), &Canvas::fit(6.5, w - 15.0, &n.body));
        if n.action != NoteAction::None {
            c.hit(x, y, w, h, Hit::SocialNote(i));
        }
        y += h + 4.0;
    }
}

/// The overlay panel's rectangle (for clicks): x, y, w, h.
pub fn panel_rect(width: f32) -> [f32; 4] {
    [width - PANEL_W - 6.0, 24.0, PANEL_W, 480.0 - 30.0]
}

/// The chip that opens the overlay (menus, when logged in): unread messages and requests.
pub fn draw_chip(c: &mut Canvas, s: &Social, lobby_unread: usize) {
    let unread = s.unread_total() + lobby_unread + s.requests.len();
    let label = if s.connected { format!("friends {}  F9", s.online_count()) } else { "friends  F9".into() };
    // Everyone online first, after a green dot (once the server says).
    let online = s.players.as_ref().filter(|_| s.connected).map(|_| format!("{} online   ", s.online_total));
    let online_w = online.as_deref().map_or(0.0, |t| Canvas::text_width(6.5, t) + 9.0);
    let w = Canvas::text_width(6.5, &label) + 14.0 + online_w;
    // Just under the tab bar's right-hand text.
    let (x, y) = (c.width - w - 8.0, 41.0);
    let hover = c.hovered(x, y, w, 13.0);
    c.round_rect(x, y, w, 13.0, 6.0, rgba(if hover { 0x2b2b3d } else { 0x15151d }, 0.85), rgba(0x3a3a4e, 1.0), 1.0);
    if let Some(online) = &online {
        c.circle(x + 10.0, y + 6.5, 2.5, rgba(GREEN, 1.0), rgba(0x000000, 0.0), 0.0);
        c.text(x + 16.0, y + 3.5, 6.5, rgba(0xc8c8d8, 1.0), online);
    }
    c.text(x + 7.0 + online_w, y + 3.5, 6.5, rgba(0xc8c8d8, 1.0), &label);
    c.hit(x, y, w, 13.0, Hit::SocialToggle);
    badge(c, x, y + 1.0, unread, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces_and_cuts_long_words() {
        assert_eq!(wrap("hello there friend", 11), vec!["hello there", "friend"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 5), vec![""]);
        assert_eq!(wrap("привет как дела", 10), vec!["привет как", "дела"]);
        // Japanese is 4/3 as wide and has no spaces: 6 to an 8-character line.
        assert_eq!(wrap("こんにちは、元気ですか", 8), vec!["こんにちは、", "元気ですか"]);
    }

    fn player(name: &str) -> OnlinePlayer {
        OnlinePlayer { name: name.into(), activity: Activity::Menus }
    }

    #[test]
    fn players_tab_filters_and_tabs_keep_their_rows() {
        let mut s = Social { me: Some("Lynerrr".into()), friends: vec![Friend { name: "den4ik".into(), activity: Some(Activity::Menus) }], requests: vec!["Kat".into()], ..Social::default() };
        s.players = Some(vec![player("den4ik"), player("Kat"), player("Lynerrr"), player("zzz")]);
        s.cursor = 2;
        s.set_tab(Tab::Players);
        assert_eq!(s.cursor, 0);
        assert_eq!(s.rows(false), vec![Row::Search, Row::Player(0), Row::Player(1), Row::Player(2), Row::Player(3)]);
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
        let s = Social { me: Some("Kat".into()), ..Social::default() };
        assert!(s.mentions_me("hi kat!") && s.mentions_me("KAT, play?"));
        assert!(!s.mentions_me("katana") && !s.mentions_me("hello"));
    }

    #[test]
    fn wheel_scrolls_the_chat_within_its_history() {
        let mut s = Social { tab: Tab::Chat, ..Social::default() };
        s.chat_scroll_max.set(4);
        s.wheel(1, 100.0, false);
        assert_eq!(s.chat_scroll, 3);
        s.wheel(5, 100.0, false);
        assert_eq!(s.chat_scroll, 4);
        s.wheel(-10, 100.0, false);
        assert_eq!(s.chat_scroll, 0);
    }
}
