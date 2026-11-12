//! A mapping session on the editor's screen: who is in it (a panel under the menu bar's session
//! button, never over the preview),
//! where the others are on the timeline (their playheads, pointers and selections in their
//! colours), the collab parts of the open difficulty along the ruler, and following someone.
//! None of it is drawn, and nothing here runs, while editing alone. It also keeps the session's
//! connection: what the server sends goes to the session, our edits and place go out, and a
//! dropped connection is resumed.

use std::time::{Duration, Instant};

use super::{Btn, Drag, Field, NOTE_R, RULER, View};
use crate::editor::collab::Sync;
use crate::editor::collab::connect::Ready;
use crate::editor::collab::session::{Notice, Rights, Session, Tab, first_stamp, stamp};
use crate::hit::Canvas;
use crate::render::rgba;
use taiko_proto::mapping::{Document, HostCmd, Link, LinkEvent, MapUp, Mode, Part, Role};
use taiko_proto::online::Request;

/// How to connect to the session again: the server, our token, the session and our epoch, and
/// when to try next.
pub struct Relink {
    pub server: String,
    pub token: String,
    pub session: u64,
    pub epoch: u32,
    pub retry_at: Option<Instant>,
}

/// A dropped connection is tried again after this long.
const RETRY: Duration = Duration::from_secs(2);

/// The longest chat line or review comment the server takes (characters).
pub const SAY_MAX: usize = 500;

/// Where a typed line goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Say {
    Chat,
    /// A review comment on the open difficulty at the playhead.
    Note,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CollabAct {
    /// Keep to this player's playhead, or stop (pressed again).
    Follow(u32),
    /// Colour the notes by who made them.
    ByAuthor,
    /// The menu bar's "Mapping session" button (only on a server that runs them): opens the
    /// session window, or closes it.
    Start,
    /// Picks this kind of session in the window.
    Pick(Mode),
    /// Start this kind of session.
    StartAs(Mode),
    /// What the panel shows.
    Tab(Tab),
    /// Type a chat line or a comment.
    Say(Say),
    /// Go to this map time (ms), from a time written in chat or a comment.
    Seek(i64),
    /// Resolve a review comment, or open it again.
    Resolve(u64),
    /// Host: the next role for this player (mapper, modder, viewer).
    Role(u32),
    Kick(u32),
    /// Host: the open difficulty to this player as a guest, or back.
    GiveDiff(u32),
    /// Host: the selected notes' stretch of the open difficulty to this player as a part (with
    /// none selected, from the playhead to the end).
    GivePart(u32),
    /// Host: every mapper maps anywhere in the open difficulty (outside the parts given to
    /// people), or only in their own parts again.
    Free,
    /// Host: only the host edits the open difficulty, or everyone with rights again.
    Lock,
    /// Host: the guest's difficulty is taken as final (and locked), or not any more.
    Accept,
    Close,
}

/// The participants panel's width and row height.
const PANEL_W: f32 = 118.0;
const PANEL_W_HOST: f32 = 180.0;
const ROW: f32 = 10.0;
const TEXT: f32 = 6.0;
/// The size of the small words on buttons and in chat.
const SMALL: f32 = 5.0;
/// Chat lines or comments the panel shows at once.
const LINES: usize = 6;

/// Where an open part spanning the whole song ends (ms): far past any song, and finite, as the
/// server wants part bounds.
const FREE_TO: f64 = 86_400_000.0;

/// The parts of a difficulty once `from..to` goes to `who` (`None`: open, every mapper's):
/// what overlapped it is cut away.
fn with_part(parts: &[Part], from: f64, to: f64, who: Option<u32>) -> Vec<Part> {
    let mut out = Vec::new();
    for p in parts {
        if p.to <= from || p.from >= to {
            out.push(p.clone());
            continue;
        }
        if p.from < from {
            out.push(Part {
                from: p.from,
                to: from,
                assignee: p.assignee,
            });
        }
        if p.to > to {
            out.push(Part {
                from: to,
                to: p.to,
                assignee: p.assignee,
            });
        }
    }
    out.push(Part {
        from,
        to,
        assignee: who,
    });
    out.sort_by(|a, b| a.from.total_cmp(&b.from));
    out
}

/// The parts with every mapper free to map: the parts given to people stay theirs, and the rest
/// of the song becomes open parts; or (`free` false) only the parts given to people are left.
fn freed(parts: &[Part], free: bool) -> Vec<Part> {
    let mut given: Vec<Part> = parts
        .iter()
        .filter(|p| p.assignee.is_some())
        .cloned()
        .collect();
    if !free {
        return given;
    }
    given.sort_by(|a, b| a.from.total_cmp(&b.from));
    let mut out = Vec::new();
    let mut at = 0.0_f64;
    for p in given {
        if p.from > at {
            out.push(Part {
                from: at,
                to: p.from,
                assignee: None,
            });
        }
        at = at.max(p.to);
        out.push(p);
    }
    if at < FREE_TO {
        out.push(Part {
            from: at,
            to: FREE_TO,
            assignee: None,
        });
    }
    out
}

fn next_role(r: Role) -> Role {
    match r {
        Role::Mapper => Role::Modder,
        Role::Modder => Role::Viewer,
        Role::Viewer | Role::Host => Role::Mapper,
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Collab => tr!("Collab", "コラボ", "Коллаб"),
        Mode::Guest => tr!("Guest difficulties", "ゲスト難易度", "Гостевые сложности"),
        Mode::Review => tr!("Modding review", "モッディング", "Моддинг"),
    }
}

/// What a kind of session is for, in a line or two.
fn mode_about(m: Mode) -> &'static str {
    match m {
        Mode::Collab => tr!(
            "One difficulty, mapped by several people: give each a part of the song.",
            "一つの難易度を複数人で。各自に曲のパートを割り当てます。",
            "Одна сложность на несколько человек: каждому своя часть песни."
        ),
        Mode::Guest => tr!(
            "Each guest maps a difficulty of their own; timing stays yours.",
            "ゲストがそれぞれ自分の難易度を作ります。タイミングはあなたのもの。",
            "Каждый гость делает свою сложность, тайминг остаётся за тобой."
        ),
        Mode::Review => tr!(
            "Modders leave comments on the timeline; only you edit.",
            "モッダーがタイムラインにコメントを残し、編集はあなただけ。",
            "Моддеры оставляют замечания на таймлайне, редактируешь только ты."
        ),
    }
}

/// A small triangle pointing down (up when `open`), centred on `x`, `y`.
fn caret(c: &mut Canvas, x: f32, y: f32, open: bool) {
    for i in 0..3 {
        let row = (if open { 2 - i } else { i }) as f32;
        let w = 5.0 - i as f32 * 2.0;
        c.rect(x - w / 2.0, y - 1.5 + row, w, 1.0, rgba(0xd8d0ff, 1.0));
    }
}

/// The colour of mapping sessions.
const VIOLET: u32 = 0x8c7cf0;
/// What can't be done yet, in the session panel.
const AMBER: u32 = 0xffc66b;

/// A song time as minutes and seconds (`1:05`).
fn short_time(ms: f64) -> String {
    let s = (ms.max(0.0) / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What we may edit in the open difficulty, in a line for the session panel (`end`: the song's
/// length, ms), and whether we may edit anything there; `None` for the host, who edits all.
fn rights_text(s: &Session, end: f64) -> Option<(String, bool)> {
    let diff = s.diff_name(s.sync.open()).to_string();
    let rights = s.rights();
    let text = match &rights {
        Rights::All => return None,
        Rights::Free => tr!(
            "Everyone maps anywhere here",
            "ここでは誰でもどこでもマッピングできます",
            "Тут все маппят где угодно"
        )
        .to_string(),
        Rights::Diff => {
            tr!("You map {diff}", "{diff} はあなたの担当です", "{diff} — твоя сложность"; [] [diff])
        }
        Rights::Parts(parts) => {
            let spans: Vec<String> = parts
                .iter()
                .map(|&(from, to)| {
                    let to = if to >= end {
                        tr!("end", "最後", "конец").to_string()
                    } else {
                        short_time(to)
                    };
                    format!("{}–{to}", short_time(from))
                })
                .collect();
            let spans = spans.join(", ");
            tr!("Your part: {spans}", "あなたのパート: {spans}", "Твоя часть: {spans}"; [] [spans])
        }
        Rights::NoPart => tr!(
            "The host hasn't given you a part yet: you can't place notes",
            "ホストがまだパートを割り当てていません。ノーツは置けません",
            "Хост ещё не дал тебе часть: ноты ставить пока нельзя"
        )
        .to_string(),
        Rights::NotOurs(Some(own)) => {
            tr!("{diff} isn't yours: open {own} to map", "{diff} はあなたの担当ではありません。{own} を開いてください", "{diff} не твоя: открой {own}"; [] [diff, own])
        }
        Rights::NotOurs(None) => tr!(
            "The host hasn't given you a difficulty yet: you can't place notes",
            "ホストがまだ難易度を割り当てていません。ノーツは置けません",
            "Хост ещё не дал тебе сложность: ноты ставить пока нельзя"
        )
        .to_string(),
        Rights::Locked => {
            tr!("{diff} is locked by the host", "{diff} はホストがロックしました", "{diff} закрыта хостом"; [] [diff])
        }
        Rights::ReadOnly => tr!(
            "You chat and comment here; only mappers place notes",
            "ここではチャットとコメントができます。ノーツを置けるのはマッパーだけです",
            "Ты тут пишешь в чат и замечания, ноты ставят только мапперы"
        )
        .to_string(),
    };
    Some((text, rights.edits()))
}

fn role_tag(r: Role) -> &'static str {
    match r {
        Role::Host => tr!("host", "ホスト", "хост"),
        Role::Mapper => tr!("mapper", "マッパー", "маппер"),
        Role::Modder => tr!("modder", "モッダー", "моддер"),
        Role::Viewer => tr!("viewer", "閲覧", "зритель"),
    }
}

/// A small button with a word on it.
fn chip(
    c: &mut Canvas,
    buttons: &mut Vec<([f32; 4], Btn)>,
    r: [f32; 4],
    label: &str,
    act: CollabAct,
) {
    c.rect(r[0], r[1], r[2], r[3], rgba(0x2c2c3c, 1.0));
    c.text(
        r[0] + 1.5,
        r[1] + (r[3] - SMALL) / 2.0,
        SMALL,
        rgba(0xe0e0ea, 1.0),
        &Canvas::fit(SMALL, r[2] - 3.0, label),
    );
    buttons.push((r, Btn::Collab(act)));
}

impl View {
    /// Following someone: the playhead goes where theirs is, and the view with it.
    pub(super) fn keep_to_followed(&mut self) {
        let Some(t) = self.collab.as_ref().and_then(|s| s.followed_time()) else {
            return;
        };
        if self.ed.time != t {
            self.ed.time = t;
            self.follow(false);
        }
    }

    pub(super) fn press_collab(&mut self, a: CollabAct) {
        match a {
            CollabAct::Start => {
                self.picking_mode = !self.picking_mode;
                return;
            }
            CollabAct::Pick(mode) => {
                self.session_pick = mode;
                return;
            }
            CollabAct::StartAs(mode) => {
                self.picking_mode = false;
                self.session_wanted = Some(mode);
                return;
            }
            CollabAct::Say(to) => {
                self.commit_typing();
                self.typing = Some((Field::Say(to), String::new()));
                return;
            }
            CollabAct::Seek(ms) => {
                if let Some(s) = self.collab.as_mut() {
                    s.follow = None;
                }
                self.ed.time = ms as f64;
                self.follow(true);
                return;
            }
            _ => {}
        }
        if let Some(cmd) = self.host_cmd(a) {
            if let (CollabAct::GivePart(who), Some((from, to))) = (a, self.part_range()) {
                let name = self
                    .collab
                    .as_ref()
                    .and_then(|s| s.participant(who))
                    .map_or("?", |p| p.name.as_str())
                    .to_string();
                let to = if to >= self.end_ms() {
                    tr!("the end", "最後", "конца").to_string()
                } else {
                    short_time(to)
                };
                let from = short_time(from);
                self.status = tr!("{name} maps {from} to {to}", "{name} の担当: {from} から {to}", "{name} маппит с {from} до {to}"; [] [name, from, to]);
            }
            self.send_host(cmd);
            return;
        }
        let Some(s) = self.collab.as_mut() else {
            return;
        };
        match a {
            CollabAct::Follow(who) => {
                s.follow = if s.follow == Some(who) {
                    None
                } else {
                    Some(who)
                };
            }
            CollabAct::ByAuthor => s.by_author = !s.by_author,
            CollabAct::Tab(t) => s.tab = t,
            CollabAct::Resolve(id) => {
                let resolved = s.state.comments.iter().any(|c| c.id == id && !c.resolved);
                if let Some(l) = &self.link {
                    l.send(MapUp::Resolve {
                        comment: id,
                        resolved,
                    });
                }
            }
            _ => {}
        }
    }

    /// The open difficulty of the session, as the session lists it.
    fn open_doc(&self) -> Option<&Document> {
        let s = self.collab.as_ref()?;
        s.state.documents.iter().find(|d| d.diff == s.sync.open())
    }

    /// What a host's button asks of the server; `None` for the other buttons, and for
    /// players who aren't the host.
    pub(super) fn host_cmd(&self, a: CollabAct) -> Option<HostCmd> {
        let s = self.collab.as_ref()?;
        if s.state.host != s.me {
            return None;
        }
        let doc = self.open_doc();
        let diff = s.sync.open();
        let cmd = match a {
            CollabAct::Role(who) => HostCmd::Role {
                who,
                role: next_role(s.participant(who)?.role),
            },
            CollabAct::Kick(who) => HostCmd::Kick(who),
            CollabAct::GiveDiff(who) => HostCmd::AssignDiff {
                diff,
                owner: (doc?.owner != Some(who)).then_some(who),
            },
            CollabAct::GivePart(who) => {
                let (from, to) = self.part_range()?;
                HostCmd::Parts {
                    diff,
                    parts: with_part(&doc?.parts, from, to, Some(who)),
                }
            }
            CollabAct::Free => HostCmd::Parts {
                diff,
                parts: freed(&doc?.parts, !s.free()),
            },
            CollabAct::Lock => HostCmd::Lock {
                diff,
                locked: !doc?.locked,
                accepted: doc?.accepted,
            },
            CollabAct::Accept => HostCmd::Lock {
                diff,
                locked: true,
                accepted: !doc?.accepted,
            },
            CollabAct::Close => HostCmd::Close,
            _ => return None,
        };
        Some(cmd)
    }

    /// The stretch of the song the host's "part" button gives (ms, `from..to`): from the first
    /// to the last selected note; with none selected (an empty song has none to select), from
    /// the playhead to the end of the song, so pressing it for each mapper in turn at the
    /// playhead splits the song between them.
    fn part_range(&self) -> Option<(f64, f64)> {
        let selected = self
            .ed
            .chart
            .objects
            .iter()
            .filter(|o| self.ed.selection.contains(&o.id))
            .map(|o| o.time)
            .fold(None, |r: Option<(f64, f64)>, t| {
                Some(r.map_or((t, t), |(a, b)| (a.min(t), b.max(t))))
            });
        Some(match selected {
            // The last selected note belongs to the part too.
            Some((from, to)) => (from, to + 1.0),
            None => {
                let from = self.ed.time.max(0.0);
                (from, self.end_ms().max(from) + 1.0)
            }
        })
    }

    /// What a typed line becomes: chat, or a comment at the playhead of the open difficulty.
    pub(super) fn said(&self, to: Say, text: &str) -> Option<MapUp> {
        let text = text.trim();
        let s = self.collab.as_ref()?;
        if text.is_empty() {
            return None;
        }
        let text = text.chars().take(SAY_MAX).collect();
        Some(match to {
            Say::Chat => MapUp::Chat(text),
            Say::Note => MapUp::Comment {
                diff: s.sync.open(),
                time: self.ed.time.round(),
                text,
            },
        })
    }

    pub(super) fn say(&mut self, to: Say, text: &str) {
        let Some(up) = self.said(to, text) else {
            return;
        };
        match &self.link {
            Some(l) => l.send(up),
            None => {
                self.status = tr!(
                    "not connected to the session: not sent",
                    "セッションに接続していません: 送信されません",
                    "нет связи с сессией: не отправлено"
                )
                .into()
            }
        }
    }

    /// Starting a session was asked for: the screen opens one of this kind for the set.
    pub fn take_session_request(&mut self) -> Option<Mode> {
        self.session_wanted.take()
    }

    /// Into a session the game just got into (`connect::Connecting`): the editor's chart is the
    /// session's, and the connection is the session's from here on.
    pub fn attach_session(&mut self, r: Ready, link: Link, relink: Relink) -> Result<(), String> {
        let (mut sync, mut ed) = Sync::join(&r.data, r.seq, r.me, r.epoch, r.open, r.path)?;
        for s in &r.tail {
            sync.receive(&mut ed, s)
                .map_err(|_| "the server's ops don't follow its snapshot".to_string())?;
        }
        ed.follow_timing = self.ed.follow_timing;
        self.ed = ed;
        self.mark_stale();
        let mut session = Session::new(sync, r.state, r.me);
        for d in r.later {
            session.handle(&mut self.ed, d, 0.0);
        }
        self.collab = Some(Box::new(session));
        self.link = Some(link);
        self.relink = Some(relink);
        // The panel opens on joining: who is here, and what we may edit.
        self.picking_mode = true;
        self.chat_seen = 0;
        Ok(())
    }

    /// The host's say in the session (an invite, a part given to someone...).
    pub fn send_host(&self, cmd: HostCmd) {
        if let Some(l) = &self.link {
            l.send(MapUp::Host(cmd));
        }
    }

    /// What friends and the website see we're editing: the map, and in a session who we're
    /// mapping it with. It travels in the activity's map text, so every server and older game
    /// shows it as it is.
    pub fn presence_label(&self) -> String {
        let map = self.label();
        let Some(s) = &self.collab else {
            return map;
        };
        let with: Vec<&str> = s
            .state
            .participants
            .iter()
            .filter(|p| p.online && p.id != s.me)
            .map(|p| p.name.as_str())
            .collect();
        if with.is_empty() {
            map
        } else {
            format!("{map} (mapping with {})", with.join(", "))
        }
    }

    /// Hosting the session open here.
    pub fn hosting(&self) -> bool {
        self.collab
            .as_ref()
            .is_some_and(|s| s.state.host == s.me && self.link.is_some())
    }

    /// Every frame in a session: what the connection reported, our edits and place sent, and a
    /// dropped connection resumed.
    pub(super) fn pump_session(&mut self, now: Instant) {
        if self.collab.is_none() {
            return;
        }
        let events = self.link.as_ref().map(Link::poll).unwrap_or_default();
        let ups = self.take_link_events(events, now);
        if let Some(l) = &self.link {
            for up in ups {
                l.send(up);
            }
        }
        let resume = self.link.is_none()
            && self
                .relink
                .as_ref()
                .is_some_and(|r| r.retry_at.is_some_and(|at| now >= at));
        if resume && let (Some(r), Some(s)) = (self.relink.as_mut(), self.collab.as_ref()) {
            r.retry_at = None;
            self.link = Some(Link::start(
                r.server.clone(),
                Request::ResumeMapping {
                    token: r.token.clone(),
                    session: r.session,
                    epoch: r.epoch,
                    last_seq: s.sync.last_seq(),
                },
            ));
        }
    }

    /// What the connection reported, taken in: the lines to send back (our edits and place
    /// among them while connected). A lost connection is tried again a little later; being
    /// out of step with the server, at once; the end of the session leaves the chart to edit
    /// alone.
    pub(super) fn take_link_events(&mut self, events: Vec<LinkEvent>, now: Instant) -> Vec<MapUp> {
        let rev = self.ed.history.rev();
        let out = self.link_events(events, now);
        // The others' edits are in the chart: the preview shows them now, not on our next edit.
        if self.ed.history.rev() != rev {
            self.mark_stale_remote();
        }
        out
    }

    fn link_events(&mut self, events: Vec<LinkEvent>, now: Instant) -> Vec<MapUp> {
        let end = self.end_ms();
        let Some(s) = self.collab.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut retry = None;
        let mut closed = None;
        for ev in events {
            let t = s.now();
            match ev {
                LinkEvent::Joined { done, .. } => s.resumed(&mut self.ed, done),
                LinkEvent::Down(d) => match s.handle(&mut self.ed, *d, t) {
                    Some(Notice::Resume) => retry = Some(now),
                    // Why, and what we may edit after all: a refused note is taken back, so
                    // the reason has to be plain.
                    Some(Notice::Refused(why)) => {
                        self.status = match rights_text(s, end) {
                            Some((rights, false)) => format!("{why}. {rights}"),
                            _ => why,
                        }
                    }
                    Some(Notice::Closed(why)) => {
                        closed = Some(why);
                        break;
                    }
                    Some(Notice::Send(lines)) => out.extend(lines),
                    None => {}
                },
                // Sessions live in the server's memory: after a restart the session is gone.
                // The chart stays here, ours to save or to open a new session with.
                LinkEvent::Refused(why) => {
                    closed = Some(why);
                    break;
                }
                LinkEvent::Lost(why) => {
                    self.status = tr!("mapping session: connection lost ({why}), reconnecting", "マッピングセッション: 接続が切れました ({why})、再接続中", "совместный маппинг: связь потеряна ({why}), переподключаюсь"; [] [why]);
                    retry = Some(now + RETRY);
                }
            }
        }
        if let Some(why) = closed {
            self.picking_mode = false;
            self.collab = None;
            self.link = None;
            self.relink = None;
            self.ed.history.stop_sharing();
            self.status = tr!("the mapping session ended: {why}", "マッピングセッションが終了しました: {why}", "совместный маппинг закончен: {why}"; [] [why]);
            return out;
        }
        if let Some(at) = retry {
            self.link = None;
            if let Some(r) = &mut self.relink {
                r.retry_at = Some(at);
            }
            return out;
        }
        if self.link.is_none() {
            return out;
        }
        let t = s.now();
        s.sync.collect(&mut self.ed);
        let dragging = !matches!(self.drag, Drag::None);
        out.extend(s.sync.outgoing(t, dragging));
        let [tx, ty, tw, th] = self.layout.timeline;
        let (mx, my) = self.mouse;
        let cursor = (mx >= tx && mx <= tx + tw && my >= ty && my <= ty + th)
            .then_some(self.view_start + (mx - tx) as f64 * self.ms_per_px);
        out.extend(s.presence(&self.ed, cursor, t));
        out
    }

    /// The session's name on its menu-bar button: who is online, or, between connections, how
    /// many of our edits wait to go out.
    fn session_title(&self) -> String {
        let Some(s) = self.collab.as_ref() else {
            return tr!(
                "Mapping session",
                "マッピングセッション",
                "Совместный маппинг"
            )
            .to_string();
        };
        if self.link.is_none() && self.relink.is_some() {
            let n = s.sync.pending();
            return tr!("Offline · {n} unsent", "オフライン · 未送信 {n}", "Нет связи · не отпр.: {n}"; [] [n]);
        }
        let online = s.state.participants.iter().filter(|p| p.online).count();
        format!(
            "{} · {online}",
            tr!(
                "Mapping session",
                "マッピングセッション",
                "Совместный маппинг"
            )
        )
    }

    /// The session's panel under its menu-bar button (`chip`), never over the preview: what we
    /// may edit, its tabs, and who is in it (with the host's buttons), the chat, or the review
    /// comments of the open difficulty. Times written in chat and comments are links to them.
    fn draw_participants(&mut self, c: &mut Canvas, chip: [f32; 4]) {
        let panel = self.panel_w();
        let end = self.end_ms();
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let rights = rights_text(s, end);
        let head: Vec<String> = rights.as_ref().map_or(Vec::new(), |(t, _)| {
            super::chrome::wrap(SMALL, panel - 8.0, t)
        });
        let head_h = 3.0 + head.len() as f32 * 7.0;
        let x = (chip[0] + chip[2] - panel).max(4.0);
        let y = chip[1] + chip[3] + 2.0;
        let tabs_y = y + head_h + 1.0;
        let top = tabs_y + 13.0;
        let h = top - y + self.panel_body_height() + 2.0;
        self.session_win = [x, y, panel, h];
        c.round_rect(
            [x, y],
            [panel, h],
            3.0,
            rgba(0x0c0c14, 0.96),
            rgba(VIOLET, 0.85),
            1.0,
        );
        let col = match rights {
            Some((_, false)) => AMBER,
            _ => 0xc8f0c8,
        };
        for (i, line) in head.iter().enumerate() {
            c.text(
                x + 4.0,
                y + 3.0 + i as f32 * 7.0,
                SMALL,
                rgba(col, 1.0),
                line,
            );
        }
        let open_notes = self
            .open_doc()
            .map(|d| {
                s.state
                    .comments
                    .iter()
                    .filter(|k| k.diff == d.diff && !k.resolved)
                    .count()
            })
            .unwrap_or(0);
        let unread = s.heard.saturating_sub(self.chat_seen);
        let chat = if unread > 0 && s.tab != Tab::Chat {
            format!("{} {unread}", tr!("Chat", "チャット", "Чат"))
        } else {
            tr!("Chat", "チャット", "Чат").to_string()
        };
        let tabs = [
            (Tab::People, tr!("People", "参加者", "Люди").to_string()),
            (Tab::Chat, chat),
            (
                Tab::Notes,
                format!("{} {open_notes}", tr!("Notes", "メモ", "Замеч.")),
            ),
        ];
        let tw = (panel - 8.0) / 3.0;
        let mut buttons = Vec::new();
        for (i, (tab, label)) in tabs.into_iter().enumerate() {
            let tx = x + 4.0 + i as f32 * tw;
            let on = s.tab == tab;
            c.rect(
                tx,
                tabs_y,
                tw - 1.0,
                9.0,
                rgba(0x2c2c3c, if on { 1.0 } else { 0.35 }),
            );
            c.text(
                tx + 2.0,
                tabs_y + 2.0,
                SMALL,
                rgba(0xffffff, if on { 0.95 } else { 0.6 }),
                &Canvas::fit(SMALL, tw - 4.0, &label),
            );
            buttons.push((
                [tx, tabs_y, tw - 1.0, 9.0],
                Btn::Collab(CollabAct::Tab(tab)),
            ));
        }
        if s.tab == Tab::Chat {
            self.chat_seen = s.heard;
        }
        self.buttons.extend(buttons);
        match self.collab.as_ref().map(|s| s.tab) {
            Some(Tab::Chat) => self.draw_chat(c, x, top),
            Some(Tab::Notes) => self.draw_notes(c, x, top),
            _ => self.draw_people(c, x, top),
        }
    }

    /// The panel is wider for the host, whose buttons are on every player's row.
    fn panel_w(&self) -> f32 {
        if self.collab.as_ref().is_some_and(|s| s.state.host == s.me) {
            PANEL_W_HOST
        } else {
            PANEL_W
        }
    }

    /// The height of what the open tab shows.
    fn panel_body_height(&self) -> f32 {
        let Some(s) = self.collab.as_ref() else {
            return 0.0;
        };
        match s.tab {
            Tab::People => {
                let host = if s.state.host == s.me { 22.0 } else { 0.0 };
                s.state.participants.len() as f32 * ROW + 11.0 + host
            }
            Tab::Chat | Tab::Notes => LINES as f32 * 8.0 + 11.0,
        }
    }

    /// Who is in the session: each player's colour, name and role, the difficulty they're on
    /// when it isn't ours, and whether they are away; a click follows them. The host also gets
    /// a player's role, the open difficulty or the selected notes' part, and kicking, then
    /// locking, accepting and closing below.
    fn draw_people(&mut self, c: &mut Canvas, x: f32, top: f32) {
        let panel = self.panel_w();
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let now = s.now();
        let hosting = s.state.host == s.me;
        let doc = s.state.documents.iter().find(|d| d.diff == s.sync.open());
        let mut rows: Vec<_> = s.state.participants.iter().collect();
        rows.sort_by_key(|p| (p.role != Role::Host, !p.online, p.id));
        let mut buttons = Vec::new();
        let mut ry = top;
        for p in rows {
            let me = p.id == s.me;
            if s.follow == Some(p.id) {
                c.rect(x + 1.0, ry - 1.0, panel - 2.0, ROW, rgba(p.colour, 0.22));
            }
            c.rect(x + 4.0, ry + 1.0, 6.0, 6.0, rgba(p.colour, 1.0));
            let idle = p.online && p.role == Role::Mapper && !s.has_work(p.id);
            let state = if !p.online {
                Some(tr!("offline", "オフライン", "не в сети").to_string())
            } else if idle {
                Some(match s.state.mode {
                    Mode::Guest => tr!("no diff", "難易度なし", "нет сложн.").to_string(),
                    _ => tr!("no part", "パートなし", "нет части").to_string(),
                })
            } else if s.away(p.id, now) {
                Some(tr!("away", "離席", "отошёл").to_string())
            } else {
                s.remotes
                    .get(&p.id)
                    .filter(|r| r.diff != s.sync.open())
                    .map(|r| s.diff_name(r.diff).to_string())
            };
            let name = if me {
                format!("{} ({})", p.name, tr!("you", "自分", "ты"))
            } else {
                p.name.clone()
            };
            let alpha = if p.online { 1.0 } else { 0.45 };
            let controls = hosting && p.role != Role::Host;
            c.text(
                x + 13.0,
                ry,
                TEXT,
                rgba(0xffffff, alpha),
                &Canvas::fit(TEXT, if controls { 42.0 } else { 58.0 }, &name),
            );
            if !me && p.online {
                buttons.push((
                    [x + 1.0, ry - 1.0, panel - 2.0, ROW],
                    Btn::Collab(CollabAct::Follow(p.id)),
                ));
            }
            if controls {
                let give = match s.state.mode {
                    Mode::Guest => {
                        let mine = doc.is_some_and(|d| d.owner == Some(p.id));
                        (if mine { "-GD" } else { "GD" }, CollabAct::GiveDiff(p.id))
                    }
                    _ => (tr!("part", "パート", "часть"), CollabAct::GivePart(p.id)),
                };
                let chips = [
                    (57.0, 33.0, role_tag(p.role), CollabAct::Role(p.id)),
                    (92.0, 23.0, give.0, give.1),
                    (
                        117.0,
                        23.0,
                        tr!("kick", "キック", "кик"),
                        CollabAct::Kick(p.id),
                    ),
                ];
                for (dx, w, label, act) in chips {
                    chip(c, &mut buttons, [x + dx, ry, w, 8.0], label, act);
                }
                if let Some(state) = state {
                    c.text_right(
                        x + panel - 4.0,
                        ry,
                        SMALL,
                        rgba(if idle { AMBER } else { 0x9a9ab0 }, alpha),
                        &Canvas::fit(SMALL, panel - 146.0, &state),
                    );
                }
            } else {
                let tag = state.unwrap_or_else(|| role_tag(p.role).to_string());
                c.text_right(
                    x + panel - 4.0,
                    ry,
                    TEXT,
                    rgba(if idle { AMBER } else { 0x9a9ab0 }, alpha),
                    &Canvas::fit(TEXT, 40.0, &tag),
                );
            }
            ry += ROW;
        }
        if hosting {
            let (locked, accepted) = doc.map_or((false, false), |d| (d.locked, d.accepted));
            let lock = if locked {
                tr!("unlock", "ロック解除", "открыть")
            } else {
                tr!("lock", "ロック", "закрыть")
            };
            let accept = if accepted {
                tr!("not final", "未確定に", "не финал")
            } else {
                tr!("accept", "承認", "принять")
            };
            let row = [
                (36.0, lock, CollabAct::Lock),
                (50.0, accept, CollabAct::Accept),
                (
                    60.0,
                    tr!("end session", "終了", "завершить"),
                    CollabAct::Close,
                ),
            ];
            let mut cx = x + 4.0;
            for (w, label, act) in row {
                chip(c, &mut buttons, [cx, ry, w, 9.0], label, act);
                cx += w + 2.0;
            }
            ry += 11.0;
            let free = if s.free() {
                tr!(
                    "[x] everyone maps anywhere",
                    "[x] 誰でもどこでもマッピング",
                    "[x] все маппят где угодно"
                )
            } else {
                tr!(
                    "[ ] everyone maps anywhere",
                    "[ ] 誰でもどこでもマッピング",
                    "[ ] все маппят где угодно"
                )
            };
            c.text(
                x + 4.0,
                ry + 2.0,
                TEXT,
                rgba(0xc8c8d8, 1.0),
                &Canvas::fit(TEXT, panel - 8.0, free),
            );
            buttons.push((
                [x + 1.0, ry, panel - 2.0, 11.0],
                Btn::Collab(CollabAct::Free),
            ));
            ry += 11.0;
        }
        let label = if s.by_author {
            tr!(
                "[x] colour by author",
                "[x] 作者で色分け",
                "[x] цвет по автору"
            )
        } else {
            tr!(
                "[ ] colour by author",
                "[ ] 作者で色分け",
                "[ ] цвет по автору"
            )
        };
        c.text(
            x + 4.0,
            ry + 2.0,
            TEXT,
            rgba(0xc8c8d8, 1.0),
            &Canvas::fit(TEXT, panel - 8.0, label),
        );
        buttons.push((
            [x + 1.0, ry, panel - 2.0, 11.0],
            Btn::Collab(CollabAct::ByAuthor),
        ));
        self.buttons.extend(buttons);
    }

    /// The last lines of chat, in their writers' colours; a line with a time in it goes there.
    fn draw_chat(&mut self, c: &mut Canvas, x: f32, top: f32) {
        let panel = self.panel_w();
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let lines: Vec<(u32, String, Option<f64>)> = s
            .chat
            .iter()
            .rev()
            .take(LINES)
            .rev()
            .map(|(who, text)| {
                let name = s.participant(*who).map_or("?", |p| p.name.as_str());
                (s.colour(*who), format!("{name}: {text}"), first_stamp(text))
            })
            .collect();
        let mut buttons = Vec::new();
        if lines.is_empty() {
            c.text(
                x + 4.0,
                top,
                SMALL,
                rgba(0x9a9ab0, 1.0),
                tr!(
                    "No messages yet.",
                    "メッセージはまだありません。",
                    "Сообщений пока нет."
                ),
            );
        }
        for (i, (colour, line, at)) in lines.into_iter().enumerate() {
            let ly = top + i as f32 * 8.0;
            c.rect(x + 4.0, ly + 1.0, 2.0, 5.0, rgba(colour, 1.0));
            let col = if at.is_some() { 0x9ec8ff } else { 0xe0e0ea };
            c.text(
                x + 8.0,
                ly,
                SMALL,
                rgba(col, 1.0),
                &Canvas::fit(SMALL, panel - 12.0, &line),
            );
            if let Some(at) = at {
                buttons.push((
                    [x + 1.0, ly - 1.0, panel - 2.0, 8.0],
                    Btn::Collab(CollabAct::Seek(at as i64)),
                ));
            }
        }
        self.buttons.extend(buttons);
        self.draw_say(c, x, top + LINES as f32 * 8.0, Say::Chat);
    }

    /// The review comments of the open difficulty, open ones first and then by time: a click
    /// goes to its time, the button on the right resolves it or opens it again.
    fn draw_notes(&mut self, c: &mut Canvas, x: f32, top: f32) {
        let panel = self.panel_w();
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let open = s.sync.open();
        let mut notes: Vec<_> = s.state.comments.iter().filter(|k| k.diff == open).collect();
        notes.sort_by(|a, b| a.resolved.cmp(&b.resolved).then(a.time.total_cmp(&b.time)));
        let mut buttons = Vec::new();
        if notes.is_empty() {
            c.text(
                x + 4.0,
                top,
                SMALL,
                rgba(0x9a9ab0, 1.0),
                tr!(
                    "No comments on this difficulty.",
                    "この難易度にコメントはありません。",
                    "Замечаний к этой сложности нет."
                ),
            );
        }
        for (i, k) in notes.into_iter().take(LINES).enumerate() {
            let ly = top + i as f32 * 8.0;
            let name = s.participant(k.author).map_or("?", |p| p.name.as_str());
            let line = format!("{} {name}: {}", stamp(k.time), k.text);
            let alpha = if k.resolved { 0.45 } else { 1.0 };
            c.rect(x + 4.0, ly + 1.0, 2.0, 5.0, rgba(s.colour(k.author), alpha));
            c.text(
                x + 8.0,
                ly,
                SMALL,
                rgba(0xe0e0ea, alpha),
                &Canvas::fit(SMALL, panel - 26.0, &line),
            );
            buttons.push((
                [x + 1.0, ly - 1.0, panel - 18.0, 8.0],
                Btn::Collab(CollabAct::Seek(k.time as i64)),
            ));
            let label = if k.resolved {
                tr!("open", "再開", "откр.")
            } else {
                tr!("done", "解決", "готово")
            };
            chip(
                c,
                &mut buttons,
                [x + panel - 17.0, ly - 1.0, 14.0, 7.0],
                label,
                CollabAct::Resolve(k.id),
            );
        }
        self.buttons.extend(buttons);
        self.draw_say(c, x, top + LINES as f32 * 8.0, Say::Note);
    }

    /// The box to type a chat line or a comment in: what is typed, or a hint to click.
    fn draw_say(&mut self, c: &mut Canvas, x: f32, y: f32, to: Say) {
        let panel = self.panel_w();
        let typed = match &self.typing {
            Some((Field::Say(t), text)) if *t == to => Some(text.clone()),
            _ => None,
        };
        let r = [x + 3.0, y, panel - 6.0, 9.0];
        c.rect(r[0], r[1], r[2], r[3], rgba(0x1c1c28, 1.0));
        let (text, col) = match typed {
            Some(t) => (format!("{t}_"), 0xffffff),
            None => (
                match to {
                    Say::Chat => tr!(
                        "say something (Ctrl+T: time)",
                        "発言する (Ctrl+T: 時間)",
                        "написать (Ctrl+T: время)"
                    ),
                    Say::Note => tr!(
                        "comment at the playhead",
                        "再生位置にコメント",
                        "замечание на этом месте"
                    ),
                }
                .to_string(),
                0x8a8aa0,
            ),
        };
        // The end of a long line stays in sight while typing.
        let mut shown = text.as_str();
        while Canvas::text_width(SMALL, shown) > r[2] - 4.0 && !shown.is_empty() {
            let cut = shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i);
            shown = &shown[cut..];
        }
        c.text(r[0] + 2.0, y + 2.0, SMALL, rgba(col, 1.0), shown);
        self.buttons.push((r, Btn::Collab(CollabAct::Say(to))));
    }

    /// Whether the session window (in a session: its panel) is open.
    #[cfg(test)]
    pub fn session_panel_open(&self) -> bool {
        self.picking_mode
    }

    /// Opens the session window, as its button does.
    #[cfg(test)]
    pub fn open_session_window(&mut self) {
        self.picking_mode = self.offers_session() || self.collab.is_some();
    }

    /// Whether the editor offers to start a session: alone, on a server that runs them.
    fn offers_session(&self) -> bool {
        self.sessions && self.link.is_none() && self.collab.is_none()
    }

    /// The menu bar's "Mapping session" button, its right edge at `right`: where it starts
    /// (`None` when it isn't offered).
    pub(super) fn draw_session_button(
        &mut self,
        c: &mut Canvas,
        right: f32,
        h: f32,
    ) -> Option<f32> {
        let in_session = self.collab.is_some();
        if !self.offers_session() && !in_session {
            self.picking_mode = false;
            return None;
        }
        let title = self.session_title();
        let label = title.as_str();
        // Chat not read yet: a mark on the button while the panel is shut.
        let unread = self
            .collab
            .as_ref()
            .is_some_and(|s| s.heard > self.chat_seen && !self.picking_mode);
        let w = Canvas::text_width(7.0, label) + 24.0 + if unread { 6.0 } else { 0.0 };
        let x = right - w;
        let open = self.picking_mode;
        let fill = if open {
            0x3b3478
        } else if c.hovered(x, 1.0, w, h - 2.0) {
            0x2c2852
        } else {
            0x1d1b34
        };
        c.round_rect(
            [x, 2.0],
            [w, h - 4.0],
            3.0,
            rgba(fill, 1.0),
            rgba(VIOLET, if open { 1.0 } else { 0.7 }),
            1.0,
        );
        // A pulse of the accent, as of someone live.
        c.circle(x + 7.0, h / 2.0, 2.2, rgba(VIOLET, 1.0), rgba(0, 0.0), 0.0);
        c.text(x + 12.0, 3.5, 7.0, rgba(0xffffff, 1.0), label);
        if unread {
            c.circle(
                x + w - 14.0,
                h / 2.0,
                2.0,
                rgba(AMBER, 1.0),
                rgba(0, 0.0),
                0.0,
            );
        }
        caret(c, x + w - 7.0, h / 2.0, open);
        self.buttons
            .push(([x, 0.0, w, h], Btn::Collab(CollabAct::Start)));
        Some(x)
    }

    /// The session window under its button: the kinds of session, each with what it is for,
    /// and the button that starts the one picked.
    pub(super) fn draw_session_window(&mut self, c: &mut Canvas) {
        let Some(&(chip, _)) = self
            .buttons
            .iter()
            .find(|(_, b)| *b == Btn::Collab(CollabAct::Start))
        else {
            return;
        };
        if !self.picking_mode {
            return;
        }
        if self.collab.is_some() {
            self.draw_participants(c, chip);
            return;
        }
        const W: f32 = 210.0;
        const CARD: f32 = 31.0;
        let x = (chip[0] + chip[2] - W).max(4.0);
        let y = chip[1] + chip[3] + 2.0;
        let intro = super::chrome::wrap(
            5.5,
            W - 16.0,
            tr!(
                "Open this set to your friends and edit it together, live. Everyone sees the others' notes, playheads and chat as they happen.",
                "このセットをフレンドに開放して、リアルタイムで一緒に編集します。他の人のノーツ、再生位置、チャットがすぐに見えます。",
                "Открой этот сет друзьям и редактируйте его вместе, вживую. Все сразу видят ноты, курсоры и чат друг друга."
            ),
        );
        let modes = [Mode::Collab, Mode::Guest, Mode::Review];
        let h = 22.0 + intro.len() as f32 * 8.5 + 4.0 + modes.len() as f32 * (CARD + 3.0) + 30.0;
        self.session_win = [x, y, W, h];
        c.round_rect(
            [x + 2.0, y + 3.0],
            [W, h],
            5.0,
            rgba(0x000000, 0.45),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [x, y],
            [W, h],
            5.0,
            rgba(0x12121c, 0.98),
            rgba(VIOLET, 0.85),
            1.0,
        );
        c.rect(x + 1.0, y + 1.0, W - 2.0, 2.0, rgba(VIOLET, 0.9));
        c.text(
            x + 8.0,
            y + 8.0,
            8.0,
            rgba(0xffffff, 1.0),
            tr!("Map together", "一緒にマッピング", "Маппим вместе"),
        );
        let mut ty = y + 21.0;
        for line in &intro {
            c.text(x + 8.0, ty, 5.5, rgba(0xa8a8bc, 1.0), line);
            ty += 8.5;
        }
        let mut cy = ty + 4.0;
        let mut buttons = Vec::new();
        for mode in modes {
            let on = self.session_pick == mode;
            let r = [x + 6.0, cy, W - 12.0, CARD];
            let hot = c.hovered(r[0], r[1], r[2], r[3]);
            c.round_rect(
                [r[0], r[1]],
                [r[2], r[3]],
                4.0,
                rgba(
                    if on {
                        0x2a2550
                    } else if hot {
                        0x1f1f2e
                    } else {
                        0x181824
                    },
                    1.0,
                ),
                rgba(if on { VIOLET } else { 0x2c2c3c }, 1.0),
                1.0,
            );
            // The radio mark.
            let (rx, ry) = (r[0] + 9.0, r[1] + 9.0);
            c.circle(rx, ry, 3.5, rgba(0x0e0e16, 1.0), rgba(VIOLET, 1.0), 1.0);
            if on {
                c.circle(rx, ry, 1.8, rgba(VIOLET, 1.0), rgba(0, 0.0), 0.0);
            }
            c.text(
                r[0] + 17.0,
                r[1] + 5.5,
                7.0,
                rgba(0xffffff, 1.0),
                mode_name(mode),
            );
            let about = super::chrome::wrap(5.0, r[2] - 22.0, mode_about(mode));
            for (i, line) in about.iter().take(2).enumerate() {
                c.text(
                    r[0] + 17.0,
                    r[1] + 15.0 + i as f32 * 7.0,
                    5.0,
                    rgba(0x9a9ab0, 1.0),
                    line,
                );
            }
            buttons.push((r, Btn::Collab(CollabAct::Pick(mode))));
            cy += CARD + 3.0;
        }
        let fy = y + h - 22.0;
        c.text(
            x + 8.0,
            fy + 5.0,
            5.0,
            rgba(0x7c7c92, 1.0),
            &Canvas::fit(
                5.0,
                W - 60.0,
                tr!(
                    "then invite friends (F9)",
                    "開始後にフレンドを招待 (F9)",
                    "потом зови друзей (F9)"
                ),
            ),
        );
        let start = tr!("Start", "開始", "Начать");
        let sw = Canvas::text_width(7.0, start) + 22.0;
        let sr = [x + W - sw - 8.0, fy, sw, 15.0];
        let hot = c.hovered(sr[0], sr[1], sr[2], sr[3]);
        c.round_rect(
            [sr[0], sr[1]],
            [sr[2], sr[3]],
            4.0,
            rgba(if hot { 0x9a8cff } else { VIOLET }, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        c.text(sr[0] + 11.0, sr[1] + 4.0, 7.0, rgba(0x0e0e16, 1.0), start);
        buttons.push((sr, Btn::Collab(CollabAct::StartAs(self.session_pick))));
        let close = "x";
        let cr = [x + W - 14.0, y + 5.0, 10.0, 10.0];
        let hot = c.hovered(cr[0], cr[1], cr[2], cr[3]);
        c.text(
            cr[0] + 3.0,
            cr[1] + 1.5,
            7.0,
            rgba(if hot { 0xffffff } else { 0x8a8aa0 }, 1.0),
            close,
        );
        buttons.push((cr, Btn::Collab(CollabAct::Start)));
        self.buttons.extend(buttons);
    }

    /// The others on the timeline, in their colours: the collab parts of the open difficulty on
    /// the ruler, notes by who made them, the notes they have selected, their pointers and
    /// their playheads with their names.
    pub(super) fn draw_collab_timeline(&self, c: &mut Canvas, [x, y, w, h]: [f32; 4], cy: f32) {
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let (from, to) = (self.view_start, self.view_end());
        let inside = |px: f32| px >= x && px <= x + w;
        if let Some(doc) = s.state.documents.iter().find(|d| d.diff == s.sync.open()) {
            for part in &doc.parts {
                if part.to < from || part.from > to {
                    continue;
                }
                let (a, b) = (self.x_of(part.from).max(x), self.x_of(part.to).min(x + w));
                let col = part.assignee.map_or(0x9A9AA8, |who| s.colour(who));
                c.rect(a, y + RULER - 3.0, b - a, 3.0, rgba(col, 0.85));
            }
        }
        // Review comments on the ruler: open ones bright, resolved ones dim.
        for k in s.state.comments.iter().filter(|k| k.diff == s.sync.open()) {
            let kx = self.x_of(k.time);
            if k.time >= from && k.time <= to && inside(kx) {
                let col = if k.resolved { 0x6a6a78 } else { 0xffd24a };
                c.rect(kx - 1.5, y + 1.0, 3.0, RULER - 4.0, rgba(col, 0.95));
            }
        }
        let first = self.ed.chart.objects.partition_point(|o| o.time < from);
        for o in self.ed.chart.objects[first..]
            .iter()
            .take_while(|o| o.time <= to)
        {
            let ox = self.note_x(o.time);
            if let Some(col) = s.author_colour(o.id) {
                c.rect(ox - 4.0, cy + NOTE_R + 2.0, 8.0, 2.0, rgba(col, 1.0));
            }
            if let Some(who) = s.selected_by(o.id) {
                c.ring(ox, cy, NOTE_R + 2.5, rgba(s.colour(who), 0.95), 1.5);
            }
        }
        for (&who, r) in &s.remotes {
            if r.diff != s.sync.open() {
                continue;
            }
            let col = s.colour(who);
            if let Some(t) = r.cursor {
                let cx = self.x_of(t);
                if inside(cx) {
                    c.rect(cx - 0.5, y + RULER, 1.0, h - RULER - 1.0, rgba(col, 0.35));
                }
            }
            let px = self.x_of(r.time);
            if !inside(px) {
                continue;
            }
            c.rect(px - 0.5, y + 1.0, 1.0, h - 2.0, rgba(col, 0.9));
            let name = s.participant(who).map_or("?", |p| p.name.as_str());
            let tw = Canvas::text_width(5.0, name) + 4.0;
            c.rect(px, y + 1.0, tw, 7.0, rgba(col, 0.9));
            c.text(px + 2.0, y + 2.0, 5.0, rgba(0x000000, 1.0), name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Keys, Layout, Mode as Layer};
    use super::*;
    use crate::editor::collab::session::tests::session;
    use taiko_proto::mapping::MapDown;
    use winit::keyboard::KeyCode;

    fn view(in_session: bool) -> View {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "taiko-rs-editor-collab-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("map.osu");
        std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
        let mut v = View::open(path, 0).unwrap();
        if in_session {
            let (s, mut ed) = session();
            ed.path = v.ed.path.clone();
            v.ed = ed;
            v.collab = Some(Box::new(s));
        }
        v.layout = Layout::new(853.0, Layer::Compose, false);
        v.view_start = 0.0;
        v.ms_per_px = 10.0;
        v
    }

    fn draw(v: &mut View) -> Canvas {
        let mut c = Canvas::default();
        c.clear(853.0);
        c.audit = true;
        v.draw(&mut c, 1.0, crate::autoplay::Settings::default());
        c
    }

    fn has_text(c: &Canvas, s: &str) -> bool {
        c.texts.iter().any(|(_, t)| t.contains(s))
    }

    /// `s` in the drawn text, lines wrapped or not.
    fn has_words(c: &Canvas, s: &str) -> bool {
        let all: Vec<&str> = c.texts.iter().map(|(_, t)| t.as_str()).collect();
        all.join(" ").contains(s)
    }

    fn collab_buttons(v: &View) -> Vec<([f32; 4], CollabAct)> {
        v.buttons
            .iter()
            .filter_map(|(r, b)| match b {
                Btn::Collab(a) => Some((*r, *a)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn editing_alone_shows_nothing_of_a_session() {
        let mut v = view(false);
        let c = draw(&mut v);
        assert!(!has_text(&c, "Mapping session"));
        assert!(collab_buttons(&v).is_empty());
    }

    #[test]
    fn the_others_show_on_the_timeline_and_a_click_follows_them() {
        let mut v = view(true);
        let id = v.ed.chart.objects[1].id;
        let s = v.collab.as_mut().unwrap();
        let now = s.now();
        s.handle(
            &mut v.ed,
            MapDown::Presence {
                who: 2,
                diff: 0,
                time: 1500.0,
                cursor: Some(2000.0),
                selection: vec![id],
            },
            now,
        );
        // The panel is shut: only the menu bar's button, nothing over the preview.
        let c = draw(&mut v);
        assert!(has_text(&c, "Mapping session · 3"), "on the button");
        assert!(!has_text(&c, "host (you)"));
        let [px, py, pw, ph] = v.layout.preview;
        let acts: Vec<CollabAct> = collab_buttons(&v).iter().map(|r| r.1).collect();
        assert_eq!(acts, vec![CollabAct::Start]);
        assert!(collab_buttons(&v)[0].0[1] < 14.0, "up in the menu bar");
        v.press(Btn::Collab(CollabAct::Start), Keys::default(), None);
        let c = draw(&mut v);
        assert!(has_text(&c, "host (you)"));
        assert!(has_text(&c, "anna"), "in the panel and on their playhead");
        // The button; the tabs; everyone but us can be followed, and the host has a say over
        // each of them; the host's row for the open difficulty and the session, letting
        // everyone map anywhere; the colour toggle is last.
        let rows = collab_buttons(&v);
        let acts: Vec<CollabAct> = rows.iter().map(|r| r.1).collect();
        assert_eq!(
            acts,
            vec![
                CollabAct::Start,
                CollabAct::Tab(Tab::People),
                CollabAct::Tab(Tab::Chat),
                CollabAct::Tab(Tab::Notes),
                CollabAct::Follow(2),
                CollabAct::Role(2),
                CollabAct::GiveDiff(2),
                CollabAct::Kick(2),
                CollabAct::Follow(3),
                CollabAct::Role(3),
                CollabAct::GiveDiff(3),
                CollabAct::Kick(3),
                CollabAct::Lock,
                CollabAct::Accept,
                CollabAct::Close,
                CollabAct::Free,
                CollabAct::ByAuthor
            ]
        );
        // The panel hangs under its button, on the screen.
        let [wx, wy, ww, wh] = v.session_win;
        assert!(wy >= rows[0].0[1] + rows[0].0[3] && wx >= 0.0 && wx + ww <= 853.0);
        for ([x, y, w, h], _) in &rows[1..] {
            assert!(*x >= wx && x + w <= wx + ww && *y >= wy && y + h <= wy + wh);
        }
        assert!(pw > 0.0 && ph > 0.0 && px >= 0.0 && py >= 0.0);
        v.press(Btn::Collab(CollabAct::Follow(2)), Keys::default(), None);
        draw(&mut v);
        assert_eq!(v.ed.time, 1500.0, "the playhead keeps to theirs");
        v.press(Btn::Collab(CollabAct::Follow(2)), Keys::default(), None);
        assert_eq!(
            v.collab.as_ref().unwrap().follow,
            None,
            "pressed again: stop"
        );
        v.press(Btn::Collab(CollabAct::ByAuthor), Keys::default(), None);
        assert!(v.collab.as_ref().unwrap().by_author);
    }

    #[test]
    fn the_others_edits_show_in_the_preview_without_an_edit_of_ours() {
        use taiko_proto::mapping::{MapOp, Obj, Sequenced};
        let mut v = view(true);
        draw(&mut v);
        let notes = |v: &View| v.game.as_ref().map_or(0, |g| g.map().notes.len());
        let before = notes(&v);
        assert!(before > 0 && v.stale.is_none(), "the preview is built");
        let op = MapOp::InsertObjects(vec![Obj {
            id: (2 << 40) | 1,
            line: "256,192,3000,1,0,0:0:0:0:".into(),
            fake: false,
            group: None,
        }]);
        let down = |seq: u64| {
            LinkEvent::Down(Box::new(MapDown::Op(Sequenced {
                seq,
                author: 2,
                epoch: 2,
                client_seq: seq,
                diff: Some(0),
                op: op.clone(),
            })))
        };
        v.take_link_events(vec![down(1)], Instant::now());
        assert!(v.ed.chart.objects.iter().any(|o| o.time == 3000.0));
        assert!(v.stale.is_some(), "the preview is behind the chart");
        // Their drags come many times a second: the preview catches up a moment later, as
        // during a drag of ours.
        draw(&mut v);
        assert_eq!(
            notes(&v),
            before,
            "not rebuilt on every line from the server"
        );
        v.stale = Some(Instant::now() - Duration::from_millis(200));
        draw(&mut v);
        assert_eq!(notes(&v), before + 1, "their note is in the preview");
        assert!(v.stale.is_none());
        // An edit of ours right after theirs shows at once.
        v.take_link_events(vec![down(2)], Instant::now());
        v.mark_stale();
        draw(&mut v);
        assert!(v.stale.is_none(), "ours is never held back");
    }

    #[test]
    fn a_session_is_offered_only_by_a_server_that_runs_them() {
        let mut v = view(false);
        let c = draw(&mut v);
        assert!(!has_text(&c, "Mapping session"));
        v.sessions = true;
        let c = draw(&mut v);
        assert!(has_text(&c, "Mapping session"), "a button in the menu bar");
        let buttons = collab_buttons(&v);
        assert_eq!(buttons.len(), 1);
        assert!(
            buttons[0].0[1] < 14.0,
            "up in the menu bar, not over the preview"
        );
        // Pressed: the session window, with the kinds of session to pick from.
        v.press(Btn::Collab(CollabAct::Start), Keys::default(), None);
        let c = draw(&mut v);
        assert!(has_text(&c, "Map together"));
        assert!(has_text(&c, "Guest difficulties"));
        let acts: Vec<CollabAct> = collab_buttons(&v).iter().map(|r| r.1).collect();
        assert_eq!(
            acts,
            vec![
                CollabAct::Start,
                CollabAct::Pick(Mode::Collab),
                CollabAct::Pick(Mode::Guest),
                CollabAct::Pick(Mode::Review),
                CollabAct::StartAs(Mode::Collab),
                CollabAct::Start,
            ]
        );
        v.press(
            Btn::Collab(CollabAct::Pick(Mode::Review)),
            Keys::default(),
            None,
        );
        assert_eq!(v.take_session_request(), None, "nothing started yet");
        draw(&mut v);
        let start = collab_buttons(&v)
            .into_iter()
            .find(|(_, a)| matches!(a, CollabAct::StartAs(_)))
            .unwrap();
        assert_eq!(start.1, CollabAct::StartAs(Mode::Review), "the kind picked");
        v.press(Btn::Collab(start.1), Keys::default(), None);
        assert_eq!(v.take_session_request(), Some(Mode::Review));
        assert_eq!(v.take_session_request(), None, "taken once");
        draw(&mut v);
        assert_eq!(collab_buttons(&v).len(), 1, "the window is closed again");
    }

    #[test]
    fn the_session_window_closes_on_esc_and_on_a_click_elsewhere_but_not_on_itself() {
        let mut v = view(false);
        v.open_session_window();
        assert!(!v.picking_mode, "not on a server without sessions");
        v.sessions = true;
        draw(&mut v);
        v.open_session_window();
        draw(&mut v);
        let [x, y, w, h] = v.session_win;
        assert!(w > 0.0 && h > 0.0);
        // A click on the window's empty space keeps it open and does nothing under it.
        v.mouse_down(x + 3.0, y + h - 3.0, Keys::default(), None);
        assert!(v.picking_mode);
        v.key(KeyCode::Escape, false, Keys::default(), None);
        assert!(!v.picking_mode, "Esc closes it");
        v.press(Btn::Collab(CollabAct::Start), Keys::default(), None);
        draw(&mut v);
        let [px, py, pw, ph] = v.layout.preview;
        v.mouse_down(px + 10.0, py + ph - 10.0, Keys::default(), None);
        assert!(!v.picking_mode, "a click elsewhere closes it");
        assert!(pw > 0.0);
    }

    #[test]
    fn a_lost_connection_is_resumed_later_and_the_end_of_the_session_leaves_the_chart() {
        let mut v = view(true);
        v.relink = Some(Relink {
            server: "127.0.0.1:9".into(),
            token: "t".into(),
            session: 9,
            epoch: 1,
            retry_at: None,
        });
        let now = Instant::now();
        let out = v.take_link_events(vec![LinkEvent::Lost("reset".into())], now);
        assert!(out.is_empty());
        assert!(v.status.contains("reset"), "{}", v.status);
        assert_eq!(v.relink.as_ref().unwrap().retry_at, Some(now + RETRY));
        let c = draw(&mut v);
        assert!(has_text(&c, "Offline · 0 unsent"));
        // Asked for our snapshot at our seq: it goes back.
        let seq = v.collab.as_ref().unwrap().sync.last_seq();
        let wanted = LinkEvent::Down(Box::new(MapDown::SnapshotWanted { seq }));
        let out = v.take_link_events(vec![wanted], now);
        assert!(matches!(out[..], [MapUp::Snapshot { .. }]));
        let objects = v.ed.chart.objects.len();
        let closed = LinkEvent::Down(Box::new(MapDown::Closed("bye".into())));
        v.take_link_events(vec![closed], now);
        // A resume the server refuses (the session went with a restart) ends it the same way.
        let mut w = view(true);
        w.take_link_events(vec![LinkEvent::Refused("no such session".into())], now);
        assert!(w.collab.is_none() && w.status.contains("no such session"));
        assert!(v.collab.is_none() && v.relink.is_none());
        assert!(v.status.contains("bye"), "{}", v.status);
        assert_eq!(v.ed.chart.objects.len(), objects, "the chart stays");
        v.ed.place_hit(8000.0, false).unwrap();
        assert!(v.ed.history.take_outgoing().is_empty(), "edits stay here");
    }

    #[test]
    fn the_hosts_buttons_ask_the_server_and_only_for_the_host() {
        let mut v = view(true);
        let cmd = |v: &View, a| v.host_cmd(a);
        assert_eq!(
            cmd(&v, CollabAct::Role(2)),
            Some(HostCmd::Role {
                who: 2,
                role: Role::Modder
            })
        );
        assert_eq!(
            cmd(&v, CollabAct::Role(3)),
            Some(HostCmd::Role {
                who: 3,
                role: Role::Viewer
            })
        );
        assert_eq!(cmd(&v, CollabAct::Kick(3)), Some(HostCmd::Kick(3)));
        assert_eq!(
            cmd(&v, CollabAct::GiveDiff(2)),
            Some(HostCmd::AssignDiff {
                diff: 0,
                owner: Some(2)
            })
        );
        assert_eq!(
            cmd(&v, CollabAct::Lock),
            Some(HostCmd::Lock {
                diff: 0,
                locked: true,
                accepted: false
            })
        );
        assert_eq!(
            cmd(&v, CollabAct::Accept),
            Some(HostCmd::Lock {
                diff: 0,
                locked: true,
                accepted: true
            })
        );
        assert_eq!(cmd(&v, CollabAct::Close), Some(HostCmd::Close));
        // Given back once it is theirs.
        v.collab.as_mut().unwrap().state.documents[0].owner = Some(2);
        assert_eq!(
            cmd(&v, CollabAct::GiveDiff(2)),
            Some(HostCmd::AssignDiff {
                diff: 0,
                owner: None
            })
        );
        // With no notes selected, a part runs from the playhead to the end of the song: an
        // empty song splits by moving the playhead and giving each mapper theirs in turn.
        v.ed.time = 1000.0;
        let end = v.end_ms();
        assert_eq!(
            cmd(&v, CollabAct::GivePart(2)),
            Some(HostCmd::Parts {
                diff: 0,
                parts: vec![Part {
                    from: 1000.0,
                    to: end.max(1000.0) + 1.0,
                    assignee: Some(2)
                }]
            })
        );
        v.press(Btn::Collab(CollabAct::GivePart(2)), Keys::default(), None);
        assert!(
            v.status.contains("anna maps 0:01 to the end"),
            "{}",
            v.status
        );
        // With notes selected: from the first to the last of them.
        let (a, b) = (&v.ed.chart.objects[0], &v.ed.chart.objects[2]);
        let (ta, tb) = (a.time, b.time);
        v.ed.selection = [a.id, b.id].into_iter().collect();
        assert_eq!(
            cmd(&v, CollabAct::GivePart(2)),
            Some(HostCmd::Parts {
                diff: 0,
                parts: vec![Part {
                    from: ta,
                    to: tb + 1.0,
                    assignee: Some(2)
                }]
            })
        );
        // Not the host: none of it, and the buttons aren't drawn.
        v.collab.as_mut().unwrap().me = 2;
        assert_eq!(cmd(&v, CollabAct::Kick(3)), None);
        v.picking_mode = true;
        draw(&mut v);
        assert!(!collab_buttons(&v).iter().any(|b| b.1 == CollabAct::Kick(3)));
        assert!(
            collab_buttons(&v)
                .iter()
                .any(|b| b.1 == CollabAct::Follow(3))
        );
    }

    #[test]
    fn a_part_given_cuts_what_it_overlaps() {
        let part = |from, to, who| Part {
            from,
            to,
            assignee: Some(who),
        };
        let parts = vec![part(0.0, 1000.0, 2), part(1000.0, 3000.0, 3)];
        assert_eq!(
            with_part(&parts, 500.0, 1500.0, Some(4)),
            vec![
                part(0.0, 500.0, 2),
                part(500.0, 1500.0, 4),
                part(1500.0, 3000.0, 3)
            ]
        );
        assert_eq!(
            with_part(&parts, 1200.0, 1800.0, Some(4)),
            vec![
                part(0.0, 1000.0, 2),
                part(1000.0, 1200.0, 3),
                part(1200.0, 1800.0, 4),
                part(1800.0, 3000.0, 3)
            ]
        );
        assert_eq!(
            with_part(&parts, 0.0, 5000.0, Some(4)),
            vec![part(0.0, 5000.0, 4)]
        );
    }

    #[test]
    fn the_host_lets_everyone_map_anywhere_but_in_the_parts_given_to_people() {
        let given = |from, to, who| Part {
            from,
            to,
            assignee: Some(who),
        };
        let open = |from, to| Part {
            from,
            to,
            assignee: None,
        };
        assert_eq!(freed(&[], true), vec![open(0.0, FREE_TO)]);
        let parts = vec![given(1000.0, 2000.0, 2), given(3000.0, 4000.0, 3)];
        let free = freed(&parts, true);
        assert_eq!(
            free,
            vec![
                open(0.0, 1000.0),
                given(1000.0, 2000.0, 2),
                open(2000.0, 3000.0),
                given(3000.0, 4000.0, 3),
                open(4000.0, FREE_TO)
            ]
        );
        assert_eq!(freed(&free, false), parts, "back to the given parts only");
        // The host's toggle sends that; pressed again, it takes the open parts back.
        let mut v = view(true);
        v.collab.as_mut().unwrap().state.mode = Mode::Collab;
        let free = v.host_cmd(CollabAct::Free).unwrap();
        assert_eq!(
            free,
            HostCmd::Parts {
                diff: 0,
                parts: vec![open(0.0, FREE_TO)]
            }
        );
        let HostCmd::Parts { parts, .. } = free else {
            unreachable!()
        };
        let s = v.collab.as_mut().unwrap();
        s.state.documents[0].parts = parts;
        assert!(s.free());
        assert_eq!(
            v.host_cmd(CollabAct::Free),
            Some(HostCmd::Parts {
                diff: 0,
                parts: Vec::new()
            })
        );
    }

    #[test]
    fn a_mapper_sees_where_they_may_place_notes_and_why_an_edit_was_refused() {
        let mut v = view(true);
        let s = v.collab.as_mut().unwrap();
        s.state.mode = Mode::Collab;
        s.state.documents[1].owner = None;
        s.me = 2;
        v.picking_mode = true;
        let c = draw(&mut v);
        assert!(has_words(&c, "The host hasn't given you a part yet"));
        // A refused note says why, and what we may do instead.
        let refused = LinkEvent::Down(Box::new(MapDown::Refused {
            client_seq: 0,
            why: "Oni isn't yours".into(),
        }));
        v.take_link_events(vec![refused], Instant::now());
        assert!(
            v.status
                .contains("Oni isn't yours. The host hasn't given you a part yet"),
            "{}",
            v.status
        );
        // Given a part, then the whole difficulty free.
        let s = v.collab.as_mut().unwrap();
        s.state.documents[0].parts = vec![Part {
            from: 1_000.0,
            to: 2_000.0,
            assignee: Some(2),
        }];
        assert!(v.end_ms() > 2_000.0);
        let c = draw(&mut v);
        assert!(has_words(&c, "Your part: 0:01–0:02"));
        // A part to the end of the song says so.
        v.collab.as_mut().unwrap().state.documents[0].parts[0].to = FREE_TO;
        let c = draw(&mut v);
        assert!(has_words(&c, "Your part: 0:01–end"));
        v.collab.as_mut().unwrap().state.documents[0].parts = freed(&[], true);
        let c = draw(&mut v);
        assert!(has_words(&c, "Everyone maps anywhere here"));
        // In guest difficulties: the difficulty that is ours is named.
        let s = v.collab.as_mut().unwrap();
        s.state.mode = Mode::Guest;
        s.state.documents[0].parts.clear();
        s.state.documents[1].owner = Some(2);
        let c = draw(&mut v);
        assert!(has_words(&c, "open Anna's Muzukashii"));
    }

    #[test]
    fn the_host_sees_who_has_nothing_to_map_yet() {
        let mut v = view(true);
        let s = v.collab.as_mut().unwrap();
        s.state.mode = Mode::Collab;
        s.state.documents[1].owner = None;
        v.picking_mode = true;
        let c = draw(&mut v);
        assert!(has_text(&c, "no part"), "anna, a mapper with no part");
        let s = v.collab.as_mut().unwrap();
        s.state.documents[0].parts = freed(&[], true);
        let c = draw(&mut v);
        assert!(!has_text(&c, "no part"), "an open part is everyone's");
    }

    #[test]
    fn unread_chat_marks_the_shut_panel_and_the_panel_shuts_with_the_session() {
        let mut v = view(true);
        let s = v.collab.as_mut().unwrap();
        let now = s.now();
        s.handle(
            &mut v.ed,
            MapDown::Chat {
                who: 2,
                text: "hi".into(),
                at: 0,
            },
            now,
        );
        let c = draw(&mut v);
        let amber = |c: &Canvas| c.shapes.len();
        let marked = amber(&c);
        v.chat_seen = 1;
        let c = draw(&mut v);
        assert!(amber(&c) < marked, "the mark goes once read");
        v.chat_seen = 0;
        v.picking_mode = true;
        v.collab.as_mut().unwrap().tab = Tab::Chat;
        let c = draw(&mut v);
        assert!(has_text(&c, "anna: hi"));
        assert_eq!(v.chat_seen, 1, "read in the open chat");
        let closed = LinkEvent::Down(Box::new(MapDown::Closed("bye".into())));
        v.take_link_events(vec![closed], Instant::now());
        assert!(!v.picking_mode, "no start window pops up after the session");
    }

    #[test]
    fn chat_and_comments_show_in_their_tabs_and_their_times_are_links() {
        use taiko_proto::mapping::Comment;
        let mut v = view(true);
        let s = v.collab.as_mut().unwrap();
        let now = s.now();
        for (who, text) in [(2, "hi"), (3, "01:02:345 too dense")] {
            s.handle(
                &mut v.ed,
                MapDown::Chat {
                    who,
                    text: text.into(),
                    at: 0,
                },
                now,
            );
        }
        for (id, diff, time, resolved) in [
            (1, 0, 4000.0, false),
            (2, 0, 2000.0, true),
            (3, 1, 10.0, false),
        ] {
            s.handle(
                &mut v.ed,
                MapDown::Comment(Comment {
                    id,
                    diff,
                    time,
                    author: 3,
                    text: format!("note {id}"),
                    resolved,
                }),
                now,
            );
        }
        v.picking_mode = true;
        let c = draw(&mut v);
        assert!(
            has_text(&c, "Notes 1"),
            "open comments of the open difficulty"
        );
        v.press(
            Btn::Collab(CollabAct::Tab(Tab::Chat)),
            Keys::default(),
            None,
        );
        let c = draw(&mut v);
        assert!(has_text(&c, "anna: hi"));
        assert!(has_text(&c, "bob: 01:02:345"));
        let acts: Vec<CollabAct> = collab_buttons(&v).iter().map(|r| r.1).collect();
        assert!(acts.contains(&CollabAct::Seek(62_345)), "{acts:?}");
        assert!(acts.contains(&CollabAct::Say(Say::Chat)));
        v.press(Btn::Collab(CollabAct::Seek(62_345)), Keys::default(), None);
        assert_eq!(v.ed.time, 62_345.0);
        // Typing a line: what goes to the server, the playhead's time written with Ctrl+T.
        v.press(
            Btn::Collab(CollabAct::Say(Say::Chat)),
            Keys::default(),
            None,
        );
        assert!(v.wants_text());
        v.type_text("look at");
        let ctrl = Keys {
            ctrl: true,
            ..Keys::default()
        };
        v.key(KeyCode::KeyT, false, ctrl, None);
        let c = draw(&mut v);
        assert!(has_text(&c, "look at 01:02:345 _"));
        assert_eq!(
            v.said(Say::Chat, "  look  "),
            Some(MapUp::Chat("look".into()))
        );
        assert_eq!(v.said(Say::Chat, "   "), None);
        assert_eq!(
            v.said(Say::Note, "kiai here"),
            Some(MapUp::Comment {
                diff: 0,
                time: 62_345.0,
                text: "kiai here".into()
            })
        );
        v.key(KeyCode::Enter, false, Keys::default(), None);
        assert!(!v.wants_text());
        assert!(
            v.status.contains("not sent"),
            "no connection here: {}",
            v.status
        );
        // The review comments of the open difficulty: open ones first.
        v.press(
            Btn::Collab(CollabAct::Tab(Tab::Notes)),
            Keys::default(),
            None,
        );
        let c = draw(&mut v);
        assert!(has_text(&c, "00:04:000 bob: note 1"));
        assert!(has_text(&c, "00:02:000 bob: note 2"));
        assert!(!has_text(&c, "note 3"), "another difficulty's");
        let acts: Vec<CollabAct> = collab_buttons(&v).iter().map(|r| r.1).collect();
        let first = acts
            .iter()
            .position(|a| *a == CollabAct::Resolve(1))
            .unwrap();
        let second = acts
            .iter()
            .position(|a| *a == CollabAct::Resolve(2))
            .unwrap();
        assert!(first < second);
        assert!(acts.contains(&CollabAct::Seek(4000)) && acts.contains(&CollabAct::Say(Say::Note)));
        // Every tab fits its panel under the menu bar's button.
        for tab in [Tab::People, Tab::Chat, Tab::Notes] {
            v.collab.as_mut().unwrap().tab = tab;
            draw(&mut v);
            let [wx, wy, ww, wh] = v.session_win;
            for ([x, y, w, h], a) in collab_buttons(&v) {
                if a == CollabAct::Start {
                    continue;
                }
                assert!(
                    x >= wx && x + w <= wx + ww && y >= wy && y + h <= wy + wh,
                    "{tab:?}"
                );
            }
        }
    }

    #[test]
    fn friends_see_who_we_are_mapping_with() {
        let alone = view(false);
        assert!(!alone.presence_label().contains("mapping with"));
        let mut v = view(true);
        let map = v.label();
        assert_eq!(
            v.presence_label(),
            format!("{map} (mapping with anna, bob)")
        );
        let s = v.collab.as_mut().unwrap();
        for p in &mut s.state.participants {
            p.online = p.id == 1;
        }
        assert_eq!(v.presence_label(), map, "nobody else online");
    }
}
