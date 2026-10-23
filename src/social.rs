//! Friends and chat: the state behind the F9 overlay, and drawing it. Friends chat over the
//! social connection to the online server (no lobby needed); in a lobby there's also its chat.

use std::collections::HashMap;
use std::time::Instant;

use crate::online::{self, Activity, ChatLine, Friend};
use crate::render::{rgba, Canvas, Hit};

/// Overlay rows, top to bottom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    /// Type a name, Enter sends a friend request.
    AddFriend,
    /// The lobby's chat (while in a room).
    Lobby,
    Request(usize),
    Friend(usize),
}

/// Buttons on rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Accept,
    Decline,
    Watch,
    Join,
}

/// What clicking a notification does.
#[derive(Clone, PartialEq, Debug)]
pub enum NoteAction {
    /// Opens the overlay on this friend's conversation.
    Chat(String),
    /// Opens the overlay on the friend requests.
    Requests,
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
    pub open: bool,
    pub opened_at: Option<Instant>,
    pub cursor: usize,
    /// What's typed: a friend's name on the add row, else a message.
    pub input: String,
    /// The activity last sent to the server.
    pub sent_activity: Option<Activity>,
}

/// Conversation key for a friend.
pub fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Splits `text` into lines of at most `width` characters, at spaces where possible.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let len = line.chars().count();
            let room = if len == 0 { width } else { width.saturating_sub(len + 1) };
            if word.len() <= room {
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
            // A word longer than a line: cut it.
            let rest = word.split_off(width.max(1));
            lines.push(word.iter().collect());
            word = rest;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

impl Social {
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
        let mut rows = vec![Row::AddFriend];
        if in_lobby {
            rows.push(Row::Lobby);
        }
        rows.extend((0..self.requests.len()).map(Row::Request));
        rows.extend((0..self.friends.len()).map(Row::Friend));
        rows
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
        if let Row::Friend(i) = self.row(in_lobby) {
            if let Some(f) = self.friends.get(i) {
                self.unread.remove(&key(&f.name));
            }
        }
    }
}

const PANEL_W: f32 = 290.0;

/// The overlay: a translucent panel on the right over whatever screen is showing.
/// `lobby`: the room's chat when in one (name, line).
pub fn draw(c: &mut Canvas, s: &Social, logged_in: bool, lobby: Option<&[(String, String)]>, lobby_unread: usize) {
    let slide = s.opened_at.map(|t| crate::render::progress(t, 160.0)).unwrap_or(1.0);
    let x = c.width - PANEL_W - 6.0 + (1.0 - slide) * 30.0;
    let (y, h) = (24.0, 480.0 - 30.0);
    let a = slide;
    c.round_rect(x, y, PANEL_W, h, 8.0, rgba(0x0b0b12, 0.84 * a), rgba(0x3a3a4e, 0.9 * a), 1.0);
    c.hit(x, y, PANEL_W, h, Hit::SocialPanel);
    c.text(x + 12.0, y + 9.0, 9.0, rgba(0xEB452C, a), "FRIENDS");
    let head = if s.connected { format!("{} online   F9", s.online_count()) } else { "F9".into() };
    c.text_right(x + PANEL_W - 12.0, y + 10.0, 7.0, rgba(0x8a8aa0, a), &head);
    let inner = x + 8.0;
    let iw = PANEL_W - 16.0;
    if !logged_in {
        c.text(inner + 4.0, y + 34.0, 7.0, rgba(0x9a9ab0, a), "Log in under MULTIPLAYER > ONLINE");
        c.text(inner + 4.0, y + 46.0, 7.0, rgba(0x9a9ab0, a), "to add friends and chat.");
        return;
    }
    if !s.connected {
        c.text(inner + 4.0, y + 34.0, 7.0, rgba(0x9a9ab0, a), "connecting to the server...");
    }
    let in_lobby = lobby.is_some();
    let rows = s.rows(in_lobby);
    let cursor = s.cursor.min(rows.len() - 1);
    let selected = rows[cursor];

    // The list: rows from the top, scrolled so the cursor stays visible.
    let list_top = y + 26.0 + if s.connected { 0.0 } else { 14.0 };
    let list_bottom = y + 230.0;
    let height = |r: &Row| if matches!(r, Row::Friend(_)) { 28.0 } else { 22.0 };
    let mut first = 0;
    while rows[first..=cursor].iter().map(height).sum::<f32>() > list_bottom - list_top {
        first += 1;
    }
    let mut ry = list_top;
    for (i, r) in rows.iter().enumerate().skip(first) {
        let rh = height(r);
        if ry + rh > list_bottom {
            break;
        }
        let sel = i == cursor;
        let hover = c.hovered(inner, ry, iw, rh - 3.0);
        let bg = if sel { 0x2b2b3d } else if hover { 0x1d1d27 } else { 0x15151d };
        c.round_rect(inner, ry, iw, rh - 3.0, 5.0, rgba(bg, 0.85 * a), rgba(if sel { 0xEB452C } else { 0x2a2a38 }, a), 1.0);
        c.hit(inner, ry, iw, rh - 3.0, Hit::SocialRow(i));
        let ty = ry + (rh - 3.0) / 2.0 - 3.5;
        match *r {
            Row::AddFriend => {
                let text = if sel { format!("add friend: {}_", s.input) } else { "+ add a friend (type their name)".into() };
                c.text(inner + 8.0, ty, 7.0, rgba(if sel { 0xffffff } else { 0x8a8aa0 }, a), &text);
            }
            Row::Lobby => {
                c.text(inner + 8.0, ty, 8.0, rgba(0x4fb4ff, a), "# lobby chat");
                badge(c, inner + iw - 10.0, ty + 3.5, lobby_unread, a);
            }
            Row::Request(k) => {
                let name = s.requests.get(k).map(String::as_str).unwrap_or("");
                c.text(inner + 8.0, ty, 7.0, rgba(0xFFD34D, a), &format!("{name} wants to be friends"));
                let bx = inner + iw - 4.0;
                let bx = button(c, bx, ry + 3.0, "decline", Hit::SocialAction(i, Action::Decline), a);
                button(c, bx - 4.0, ry + 3.0, "accept", Hit::SocialAction(i, Action::Accept), a);
            }
            Row::Friend(k) => {
                let Some(f) = s.friends.get(k) else { continue };
                let online = f.activity.is_some();
                c.circle(inner + 10.0, ry + 8.0, 3.0, rgba(if online { 0x86E05A } else { 0x55556a }, a), rgba(0x000000, 0.0), 0.0);
                c.text(inner + 18.0, ry + 4.5, 8.0, rgba(if online { 0xffffff } else { 0x8a8aa0 }, a), &f.name);
                let what = f.activity.as_ref().map(Activity::describe).unwrap_or_else(|| "offline".into());
                let mut bx = inner + iw - 4.0;
                bx = match &f.activity {
                    Some(Activity::Playing { .. }) => button(c, bx, ry + 3.0, "watch", Hit::SocialAction(i, Action::Watch), a) - 4.0,
                    Some(Activity::Lobby { room: Some(_), .. }) => button(c, bx, ry + 3.0, "join", Hit::SocialAction(i, Action::Join), a) - 4.0,
                    _ => bx,
                };
                let max = ((bx - inner - 22.0) / 4.4).max(4.0) as usize;
                let what: String = what.chars().take(max).collect();
                c.text(inner + 18.0, ry + 15.5, 6.0, rgba(0x8a8aa0, a), &what);
                badge(c, bx - 8.0, ry + 8.0, s.unread.get(&key(&f.name)).copied().unwrap_or(0), a);
            }
        }
        ry += rh;
    }
    if s.friends.is_empty() && s.requests.is_empty() && s.connected {
        c.text(inner + 4.0, ry + 4.0, 7.0, rgba(0x6f6f86, a), "no friends yet: add one by name");
        ry += 14.0;
    }
    if !s.sent.is_empty() && ry + 10.0 < list_bottom {
        let waiting: String = format!("waiting for: {}", s.sent.join(", ")).chars().take(60).collect();
        c.text(inner + 4.0, ry + 3.0, 6.0, rgba(0x6f6f86, a), &waiting);
    }

    // The conversation.
    let chat_top = y + 236.0;
    c.rect(inner, chat_top - 2.0, iw, 1.0, rgba(0x2a2a38, a));
    let input_y = y + h - 40.0;
    let lines: Vec<(String, String, bool)> = match s.conversation(in_lobby) {
        Some(Ok(f)) => {
            c.text(inner + 2.0, chat_top + 4.0, 7.0, rgba(0x9a9ab0, a), &format!("chat with {}", f.name));
            let me = s.me.as_deref().map(key);
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
    // Newest at the bottom, wrapped; as many as fit.
    let width = ((iw - 8.0) / 4.2) as usize;
    let mut wrapped: Vec<(String, u32)> = Vec::new();
    for (from, text, mine) in &lines {
        let colour = if *mine { 0xFFD34D } else { 0xffffff };
        for (n, l) in wrap(&format!("{from}: {text}"), width).into_iter().enumerate() {
            wrapped.push((if n == 0 { l } else { format!("  {l}") }, colour));
        }
    }
    let line_h = 10.0;
    let fit = ((input_y - chat_top - 22.0) / line_h).max(0.0) as usize;
    let mut ly = input_y - 6.0 - line_h * wrapped.len().min(fit) as f32;
    for (l, colour) in wrapped.iter().skip(wrapped.len().saturating_sub(fit)) {
        c.text(inner + 4.0, ly, 6.5, rgba(*colour, a), l);
        ly += line_h;
    }
    // The message box.
    if matches!(selected, Row::Friend(_) | Row::Lobby) {
        c.round_rect(inner, input_y, iw, 18.0, 5.0, rgba(0x15151d, 0.9 * a), rgba(0x3a3a4e, a), 1.0);
        let text = if s.input.is_empty() { "type a message, Enter to send".to_string() } else { format!("{}_", s.input) };
        let shown: String = text.chars().rev().take(width - 2).collect::<Vec<_>>().into_iter().rev().collect();
        c.text(inner + 6.0, input_y + 5.5, 7.0, rgba(if s.input.is_empty() { 0x55556a } else { 0xffffff }, a), &shown);
    }
    c.text_centered(x + PANEL_W / 2.0, y + h - 14.0, 6.0, rgba(0x55556a, a), "Up/Down  Enter  Ctrl+W watch  Ctrl+J join  Del remove  Esc");
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
    c.round_rect(cx - w / 2.0, cy - 5.0, w, 10.0, 5.0, rgba(0xEB452C, a), rgba(0xEB452C, a), 0.0);
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
        c.text(x + slide + 9.0, y + 6.0, 7.0, rgba(n.colour, a), &n.title);
        let body: String = n.body.chars().take(44).collect();
        c.text(x + slide + 9.0, y + 17.0, 6.5, rgba(0xdedeea, a), &body);
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
    let w = Canvas::text_width(6.5, &label) + 14.0;
    // Just under the tab bar's right-hand text.
    let (x, y) = (c.width - w - 8.0, 41.0);
    let hover = c.hovered(x, y, w, 13.0);
    c.round_rect(x, y, w, 13.0, 6.0, rgba(if hover { 0x2b2b3d } else { 0x15151d }, 0.85), rgba(0x3a3a4e, 1.0), 1.0);
    c.text(x + 7.0, y + 3.5, 6.5, rgba(0xc8c8d8, 1.0), &label);
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
    }
}
