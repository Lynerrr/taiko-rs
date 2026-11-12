//! The mapping session's panel, opened from its menu-bar button: what we may edit here, then
//! tabs for who is in the session, who maps what (each difficulty, or the parts of a collab
//! one), the chat and the review comments. It sits beside the preview, under the mode bar and
//! above the timeline, and nothing under it reacts to the mouse while it is open. The host's
//! buttons say what they do in words; the ones that can't be undone (kicking, ending) ask for
//! a second click. The others' places and parts also show along the whole-song bar.

use std::time::{Duration, Instant};

use super::{Btn, Field, View, inside};
use crate::editor::collab::session::{Session, Tab, first_stamp, stamp};
use crate::hit::Canvas;
use crate::render::rgba;
use taiko_proto::mapping::{Mode, Role};

use super::collab::{
    AMBER, Armed, CollabAct, FREE_TO, GivePick, Say, VIOLET, mode_name, rights_text, short_time,
};

/// The panel's width.
pub(super) const PANEL_W: f32 = 236.0;
/// Its inner margin.
const PAD: f32 = 6.0;
const TITLE: f32 = 7.0;
const TEXT: f32 = 6.0;
const SMALL: f32 = 5.0;
/// A player's or a difficulty's row.
const ROW: f32 = 18.0;
/// A small button's height.
const BTN_H: f32 = 10.0;
/// A second click on a button that asks for one counts this long.
pub(super) const ARMED_FOR: Duration = Duration::from_secs(4);

const GREEN: u32 = 0x8fe3a0;
const GREY: u32 = 0x9a9ab0;
const DIM: u32 = 0x6e6e84;
const RED: u32 = 0xe2575f;

/// How a panel button looks.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Plain,
    /// The thing to press (violet).
    Main,
    /// Kicking, ending: red once a click armed it.
    Danger {
        armed: bool,
    },
}

/// What the panel draws into: the buttons and tips it adds, the rows' clip.
struct Ui {
    buttons: Vec<([f32; 4], Btn)>,
    tips: Vec<([f32; 4], String)>,
    /// The scrolling body: buttons outside it aren't clickable.
    body: [f32; 4],
}

impl Ui {
    fn push(&mut self, r: [f32; 4], act: CollabAct) {
        let [bx, by, bw, bh] = self.body;
        let top = r[1].max(by);
        let bottom = (r[1] + r[3]).min(by + bh);
        if bottom - top < 2.0 || r[0] + r[2] < bx || r[0] > bx + bw {
            return;
        }
        self.buttons
            .push(([r[0], top, r[2], bottom - top], Btn::Collab(act)));
    }

    fn tip(&mut self, r: [f32; 4], text: String) {
        self.tips.push((r, text));
    }

    /// A button with a word on it, sized to `w` (or to the word when `w` is 0); returns its
    /// rect.
    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        look: Look,
        act: CollabAct,
    ) -> [f32; 4] {
        let w = if w > 0.0 {
            w
        } else {
            Canvas::text_width(SMALL, label) + 8.0
        };
        let r = [x, y, w, BTN_H];
        let hot = c.hovered(r[0], r[1], r[2], r[3]);
        let (fill, edge, ink) = match look {
            Look::Plain => (if hot { 0x34344a } else { 0x24243a }, 0x3c3c58, 0xe6e6f0),
            Look::Main => (if hot { 0x9a8cff } else { VIOLET }, VIOLET, 0x0e0e16),
            Look::Danger { armed: true } => (RED, RED, 0x0e0e16),
            Look::Danger { armed: false } => {
                (if hot { 0x40242c } else { 0x2a1c24 }, 0x6a3040, 0xf0b8bc)
            }
        };
        c.round_rect(
            [r[0], r[1]],
            [r[2], r[3]],
            2.0,
            rgba(fill, 1.0),
            rgba(edge, 1.0),
            1.0,
        );
        c.text_centered(
            r[0] + r[2] / 2.0,
            r[1] + (BTN_H - SMALL) / 2.0,
            SMALL,
            rgba(ink, 1.0),
            &Canvas::fit(SMALL, r[2] - 4.0, label),
        );
        self.push(r, act);
        r
    }

    /// A box ticked or not, with what it does beside it, and a line under it saying more;
    /// the whole of it is the button. Returns its height.
    #[allow(clippy::too_many_arguments)]
    fn toggle(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        on: bool,
        label: &str,
        about: &str,
        act: CollabAct,
    ) -> f32 {
        let lines = super::chrome::wrap(SMALL, w - 12.0, about);
        let h = 10.0 + lines.len() as f32 * 6.5;
        let r = [x, y, w, h];
        if c.hovered(r[0], r[1], r[2], r[3]) {
            c.rect(r[0], r[1], r[2], r[3], rgba(0xffffff, 0.04));
        }
        c.round_rect(
            [x + 1.0, y + 1.5],
            [7.0, 7.0],
            1.5,
            rgba(if on { VIOLET } else { 0x14141e }, 1.0),
            rgba(if on { VIOLET } else { 0x5a5a78 }, 1.0),
            1.0,
        );
        if on {
            // A tick.
            c.rect(x + 2.5, y + 5.0, 1.5, 2.0, rgba(0x0e0e16, 1.0));
            c.rect(x + 3.5, y + 6.0, 1.5, 1.5, rgba(0x0e0e16, 1.0));
            c.rect(x + 4.5, y + 3.0, 1.5, 4.0, rgba(0x0e0e16, 1.0));
        }
        c.text(
            x + 12.0,
            y + 2.0,
            TEXT,
            rgba(0xffffff, 1.0),
            &Canvas::fit(TEXT, w - 12.0, label),
        );
        for (i, l) in lines.iter().enumerate() {
            c.text(
                x + 12.0,
                y + 9.5 + i as f32 * 6.5,
                SMALL,
                rgba(GREY, 1.0),
                l,
            );
        }
        self.push(r, act);
        h + 3.0
    }
}

/// A role in words.
fn role_name(r: Role) -> &'static str {
    match r {
        Role::Host => tr!("Host", "ホスト", "Хост"),
        Role::Mapper => tr!("Mapper", "マッパー", "Маппер"),
        Role::Modder => tr!("Modder", "モッダー", "Моддер"),
        Role::Viewer => tr!("Viewer", "閲覧", "Зритель"),
    }
}

/// A part's end in words (`end`: the song's length).
fn part_end(to: f64, end: f64) -> String {
    if to >= end {
        tr!("end", "最後", "конец").to_string()
    } else {
        short_time(to)
    }
}

/// What a player does in the session, for their row: their work, where they are.
fn activity(s: &Session, who: u32, end: f64, now: f64) -> (String, bool) {
    let Some(p) = s.participant(who) else {
        return (String::new(), false);
    };
    if !p.online {
        return (tr!("Offline", "オフライン", "Не в сети").to_string(), false);
    }
    let open = s.sync.open();
    let doc = s.state.documents.iter().find(|d| d.diff == open);
    let (mut text, short) = match p.role {
        Role::Host => (role_name(Role::Host).to_string(), false),
        Role::Modder => (
            tr!(
                "Modder: comments",
                "モッダー: コメント",
                "Моддер: замечания"
            )
            .to_string(),
            false,
        ),
        Role::Viewer => (
            tr!("Viewer: watches", "閲覧: 見るだけ", "Зритель: смотрит").to_string(),
            false,
        ),
        Role::Mapper => match s.state.mode {
            Mode::Guest => {
                let owned: Vec<&str> = s
                    .state
                    .documents
                    .iter()
                    .filter(|d| d.owner == Some(who))
                    .map(|d| d.name.as_str())
                    .collect();
                if owned.is_empty() {
                    (
                        tr!("No difficulty yet", "難易度なし", "Сложности пока нет").to_string(),
                        true,
                    )
                } else {
                    let names = owned.join(", ");
                    (
                        tr!("Maps {names}", "{names} を担当", "Маппит {names}"; [] [names]),
                        false,
                    )
                }
            }
            Mode::Review => (role_name(Role::Mapper).to_string(), false),
            Mode::Collab => {
                let parts: Vec<String> = doc
                    .map(|d| {
                        d.parts
                            .iter()
                            .filter(|p| p.assignee == Some(who))
                            .map(|p| format!("{}–{}", short_time(p.from), part_end(p.to, end)))
                            .collect()
                    })
                    .unwrap_or_default();
                if !parts.is_empty() {
                    let spans = parts.join(", ");
                    (
                        tr!("Part {spans}", "パート {spans}", "Часть {spans}"; [] [spans]),
                        false,
                    )
                } else if s.free() {
                    (
                        tr!("Maps anywhere", "どこでもマッピング", "Маппит где угодно").to_string(),
                        false,
                    )
                } else {
                    (
                        tr!("No part yet", "パートなし", "Части пока нет").to_string(),
                        true,
                    )
                }
            }
        },
    };
    if who != s.me
        && let Some(r) = s.remotes.get(&who)
    {
        if r.diff != open {
            text.push_str(" · ");
            text.push_str(s.diff_name(r.diff));
        }
        text.push_str(&format!(" · {}", short_time(r.time)));
        if s.away(who, now) {
            text.push_str(" · ");
            text.push_str(tr!("away", "離席", "отошёл"));
        }
    }
    (text, short)
}

/// A line of the chat or notes: its colour, its text, the map time it links to, and for a
/// note its id and whether it's resolved.
type TalkLine = (u32, String, Option<f64>, Option<(u64, bool)>);

impl View {
    /// Whether the mouse is over the open session panel: nothing under it hovers or tips.
    pub(super) fn over_session_panel(&self) -> bool {
        self.picking_mode && inside(self.session_win, self.mouse.0, self.mouse.1)
    }

    /// The wheel over the panel scrolls its list.
    pub(super) fn scroll_session_panel(&mut self, notches: i32) {
        self.panel_scroll = (self.panel_scroll - notches as f32 * ROW).max(0.0);
    }

    /// The session's panel, under the mode bar on the right, its button `chip` up in the
    /// menu bar.
    pub(super) fn draw_session_panel(&mut self, c: &mut Canvas, chip: [f32; 4]) {
        let end = self.end_ms();
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let w = PANEL_W;
        let x = (chip[0] + chip[2] - w).min(c.width - w - 4.0).max(4.0);
        let y = self.layout.modes[1] + self.layout.modes[3] + 4.0;
        let h = (self.layout.timeline[1] - 3.0 - y).max(120.0);
        self.session_win = [x, y, w, h];
        self.panel_tips_from = self.tips.len();
        // A shadow, then the panel, solid: what is under it doesn't show through.
        c.round_rect(
            [x + 2.0, y + 3.0],
            [w, h],
            4.0,
            rgba(0x000000, 0.5),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x11111b, 1.0),
            rgba(VIOLET, 0.9),
            1.0,
        );
        c.rect(x + 1.0, y + 1.0, w - 2.0, 2.0, rgba(VIOLET, 0.9));
        let mut ui = Ui {
            buttons: Vec::new(),
            tips: Vec::new(),
            body: [x, y, w, h],
        };
        // What kind of session, which set, who is on.
        c.circle(
            x + PAD + 2.5,
            y + 9.0,
            2.5,
            rgba(VIOLET, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        let title_w = w - 2.0 * PAD - if s.state.host == s.me { 90.0 } else { 20.0 };
        c.text(
            x + PAD + 8.0,
            y + 6.0,
            TITLE,
            rgba(0xffffff, 1.0),
            &Canvas::fit(TITLE, title_w, mode_name(s.state.mode)),
        );
        let online = s.state.participants.iter().filter(|p| p.online).count();
        let sub = format!(
            "{} · {}",
            s.state.title,
            tr!("{online} online", "{online} 人オンライン", "в сети: {online}"; [] [online])
        );
        c.text(
            x + PAD,
            y + 16.0,
            SMALL,
            rgba(GREY, 1.0),
            &Canvas::fit(SMALL, w - 2.0 * PAD - 12.0, &sub),
        );
        let close = [x + w - 14.0, y + 4.0, 10.0, 10.0];
        let hot = c.hovered(close[0], close[1], close[2], close[3]);
        c.text(
            close[0] + 3.0,
            close[1] + 1.5,
            TITLE,
            rgba(if hot { 0xffffff } else { 0x8a8aa0 }, 1.0),
            "x",
        );
        ui.push(close, CollabAct::Start);
        ui.tip(
            close,
            tr!(
                "Hides this panel\nThe session goes on; its button in the menu bar opens it again.",
                "このパネルを隠します\nセッションは続きます。メニューバーのボタンで再び開けます。",
                "Скрыть панель\nСессия идёт дальше; кнопка в меню снова её откроет."
            )
            .to_string(),
        );
        if s.state.host == s.me {
            let armed = self
                .armed
                .is_some_and(|(a, at)| a == Armed::Close && at.elapsed() < ARMED_FOR);
            let label = if armed {
                tr!("End for everyone?", "全員終了?", "Закончить у всех?")
            } else {
                tr!("End session", "終了", "Завершить")
            };
            let bw = Canvas::text_width(SMALL, label) + 10.0;
            let er = ui.button(
                c,
                close[0] - 4.0 - bw,
                y + 4.5,
                bw,
                label,
                Look::Danger { armed },
                CollabAct::Close,
            );
            ui.tip(
                er,
                tr!(
                    "Ends the session for everyone\nEach keeps the map as it is. Click twice.",
                    "全員のセッションを終了します\nマップはそのまま残ります。2回クリック。",
                    "Заканчивает сессию у всех\nКарта у каждого остаётся как есть. Нажми дважды."
                )
                .to_string(),
            );
        }
        // What we may do here, in a coloured box.
        let (said, col) = match rights_text(s, end) {
            Some((t, true)) => (t, GREEN),
            Some((t, false)) => (t, AMBER),
            None => (
                tr!(
                    "You're the host: you edit everything and give out the work. Invite friends with F9.",
                    "あなたはホストです。すべて編集でき、担当を割り当てます。F9 でフレンドを招待。",
                    "Ты хост: правишь всё и раздаёшь работу. Друзей зови через F9."
                )
                .to_string(),
                0xc8c0ff,
            ),
        };
        let lines = super::chrome::wrap(5.5, w - 2.0 * PAD - 8.0, &said);
        let lines = &lines[..lines.len().min(2)];
        let box_y = y + 25.0;
        let box_h = 5.0 + lines.len() as f32 * 7.0;
        c.rect(x + PAD, box_y, w - 2.0 * PAD, box_h, rgba(col, 0.1));
        c.rect(x + PAD, box_y, 2.0, box_h, rgba(col, 1.0));
        for (i, l) in lines.iter().enumerate() {
            c.text(
                x + PAD + 6.0,
                box_y + 2.5 + i as f32 * 7.0,
                5.5,
                rgba(col, 1.0),
                l,
            );
        }
        // The tabs.
        let tabs_y = box_y + box_h + 4.0;
        let open_notes = s
            .state
            .comments
            .iter()
            .filter(|k| k.diff == s.sync.open() && !k.resolved)
            .count();
        let unread = s.heard.saturating_sub(self.chat_seen);
        let people = s.state.participants.len();
        let work = match s.state.mode {
            Mode::Collab => tr!("Parts", "パート", "Части"),
            _ => tr!("Difficulties", "難易度", "Сложности"),
        };
        let chat = tr!("Chat", "チャット", "Чат");
        let tabs = [
            (
                Tab::People,
                format!("{} {people}", tr!("People", "参加者", "Люди")),
            ),
            (Tab::Work, work.to_string()),
            (
                Tab::Chat,
                if unread > 0 && s.tab != Tab::Chat {
                    format!("{chat} ({unread})")
                } else {
                    chat.to_string()
                },
            ),
            (
                Tab::Notes,
                format!("{} {open_notes}", tr!("Notes", "メモ", "Замеч.")),
            ),
        ];
        // Each tab as wide as its word, the room left shared out.
        let words: Vec<f32> = tabs
            .iter()
            .map(|(_, l)| Canvas::text_width(5.5, l) + 6.0)
            .collect();
        let spare = ((w - 2.0 * PAD) - words.iter().sum::<f32>()).max(0.0) / tabs.len() as f32;
        let mut tx = x + PAD;
        for (i, (tab, label)) in tabs.iter().enumerate() {
            let tw = words[i] + spare;
            let on = s.tab == *tab;
            let hot = c.hovered(tx, tabs_y, tw, 12.0);
            c.rect(
                tx,
                tabs_y,
                tw - 1.0,
                12.0,
                rgba(
                    if on {
                        0x2e2a58
                    } else if hot {
                        0x22223a
                    } else {
                        0x1a1a28
                    },
                    1.0,
                ),
            );
            if on {
                c.rect(tx, tabs_y + 10.5, tw - 1.0, 1.5, rgba(VIOLET, 1.0));
            }
            let alert = *tab == Tab::Chat && unread > 0 && !on;
            c.text_centered(
                tx + tw / 2.0,
                tabs_y + 3.0,
                5.5,
                rgba(
                    if alert {
                        AMBER
                    } else if on {
                        0xffffff
                    } else {
                        0xa8a8bc
                    },
                    1.0,
                ),
                &Canvas::fit(5.5, tw - 4.0, label),
            );
            ui.push([tx, tabs_y, tw - 1.0, 12.0], CollabAct::Tab(*tab));
            tx += tw;
        }
        let tab = s.tab;
        if tab == Tab::Chat {
            self.chat_seen = s.heard;
        }
        let body = [x + 1.0, tabs_y + 15.0, w - 2.0, y + h - tabs_y - 17.0];
        ui.body = body;
        let shapes = c.shapes.len();
        let used = match tab {
            Tab::People => self.draw_people_tab(c, &mut ui, body, end),
            Tab::Work => self.draw_work_tab(c, &mut ui, body, end),
            Tab::Chat => self.draw_talk_tab(c, &mut ui, body, Say::Chat),
            Tab::Notes => self.draw_talk_tab(c, &mut ui, body, Say::Note),
        };
        c.clip_since(shapes, body[0], body[1], body[2], body[3]);
        // Scrolled no further than the list goes.
        let most = (used - body[3]).max(0.0);
        self.panel_scroll = self.panel_scroll.min(most);
        if most > 0.0 {
            let track = body[3];
            let bar = (track * body[3] / used).max(8.0);
            let at = body[1] + (track - bar) * (self.panel_scroll / most);
            c.rect(body[0] + body[2] - 2.5, at, 1.5, bar, rgba(0xffffff, 0.25));
        }
        self.buttons.extend(ui.buttons);
        self.tips.extend(ui.tips);
    }

    /// Who is in the session: their colour, name, what they do and where; a click follows
    /// them. The host changes a player's role and kicks from their row, and ends the session
    /// below. Returns the height the list takes.
    fn draw_people_tab(&mut self, c: &mut Canvas, ui: &mut Ui, body: [f32; 4], end: f64) -> f32 {
        let Some(s) = self.collab.as_ref() else {
            return 0.0;
        };
        let [bx, by, bw, _] = body;
        let x = bx + PAD - 1.0;
        let right = bx + bw - PAD + 1.0;
        let top = by - self.panel_scroll;
        let now = s.now();
        let hosting = s.state.host == s.me;
        let armed = self
            .armed
            .filter(|(_, at)| at.elapsed() < ARMED_FOR)
            .map(|(a, _)| a);
        let mut rows: Vec<_> = s.state.participants.iter().collect();
        rows.sort_by_key(|p| (p.role != Role::Host, !p.online, p.id));
        let mut ry = top;
        for p in rows {
            let me = p.id == s.me;
            let row = [bx + 2.0, ry, bw - 4.0, ROW - 1.0];
            let followed = s.follow == Some(p.id);
            if followed {
                c.rect(row[0], row[1], row[2], row[3], rgba(p.colour, 0.18));
            } else if !me && c.hovered(row[0], row[1], row[2], row[3]) {
                c.rect(row[0], row[1], row[2], row[3], rgba(0xffffff, 0.04));
            }
            let alpha = if p.online { 1.0 } else { 0.45 };
            c.round_rect(
                [x, ry + 3.5],
                [7.0, 7.0],
                1.5,
                rgba(p.colour, alpha),
                rgba(0, 0.0),
                0.0,
            );
            let controls = hosting && !me;
            let name_w = if controls { bw - 108.0 } else { bw - 30.0 };
            let name = if me {
                format!("{} ({})", p.name, tr!("you", "自分", "ты"))
            } else {
                p.name.clone()
            };
            c.text(
                x + 11.0,
                ry + 2.0,
                TEXT,
                rgba(0xffffff, alpha),
                &Canvas::fit(TEXT, name_w, &name),
            );
            let (what, short) = activity(s, p.id, end, now);
            c.text(
                x + 11.0,
                ry + 10.0,
                SMALL,
                rgba(if short { AMBER } else { GREY }, alpha),
                &Canvas::fit(SMALL, name_w, &what),
            );
            // A click on the row follows them (the host's buttons on it stay theirs).
            if !me && p.online {
                ui.push(row, CollabAct::Follow(p.id));
                ui.tip(
                    row,
                    if followed {
                        tr!("Following {name}\nClick to stop.", "{name} を追従中\nクリックで解除。", "Следишь за {name}\nНажми, чтобы перестать."; [] [name])
                    } else {
                        tr!("Click to follow {name}\nYour playhead keeps to theirs.", "クリックで {name} を追従\n再生位置が相手に合わせて動きます。", "Нажми, чтобы следить за {name}\nТвой курсор пойдёт за его."; [] [name])
                    },
                );
            }
            if controls {
                let kick_armed = armed == Some(Armed::Kick(p.id));
                let kick = if kick_armed {
                    tr!("Sure?", "本当に?", "Точно?")
                } else {
                    tr!("Kick", "キック", "Кик")
                };
                let kr = ui.button(
                    c,
                    right - 28.0,
                    ry + 3.5,
                    28.0,
                    kick,
                    Look::Danger { armed: kick_armed },
                    CollabAct::Kick(p.id),
                );
                ui.tip(
                    kr,
                    tr!(
                        "Takes them out of the session\nClick twice. Their notes stay.",
                        "セッションから外します\n2回クリック。ノーツは残ります。",
                        "Убрать из сессии\nНажми дважды. Их ноты останутся."
                    )
                    .to_string(),
                );
                let role = format!("{} >", role_name(p.role));
                let rr = ui.button(
                    c,
                    right - 28.0 - 3.0 - 46.0,
                    ry + 3.5,
                    46.0,
                    &role,
                    Look::Plain,
                    CollabAct::Role(p.id),
                );
                ui.tip(
                    rr,
                    tr!(
                        "Their role: click for the next one\nMapper: places notes where given work. Modder: comments only. Viewer: watches.",
                        "役割: クリックで次へ\nマッパー: 担当の場所にノーツを置く。モッダー: コメントのみ。閲覧: 見るだけ。",
                        "Роль: нажми, чтобы сменить\nМаппер: ставит ноты там, где дали работу. Моддер: только замечания. Зритель: смотрит."
                    )
                    .to_string(),
                );
            } else if followed {
                c.text_right(
                    right,
                    ry + 6.0,
                    SMALL,
                    rgba(p.colour, 1.0),
                    tr!("following", "追従中", "следишь"),
                );
            }
            ry += ROW;
        }
        ry += 3.0;
        c.rect(x, ry, bw - 2.0 * PAD + 2.0, 1.0, rgba(0xffffff, 0.08));
        ry += 5.0;
        ry += ui.toggle(
            c,
            x,
            ry,
            bw - 2.0 * PAD + 2.0,
            s.by_author,
            tr!(
                "Colour notes by who placed them",
                "置いた人の色でノーツを表示",
                "Красить ноты по автору"
            ),
            tr!(
                "A mark under each note in its mapper's colour.",
                "各ノーツの下にマッパーの色の印。",
                "Под каждой нотой метка цвета её маппера."
            ),
            CollabAct::ByAuthor,
        );
        ry - top
    }

    /// Who maps what. Guest difficulties and reviews: each difficulty, who has it and in what
    /// state, a button to open it, and the host's buttons to give it out, lock it and mark it
    /// final. A collab: the open difficulty's parts, the host's button that gives the next
    /// one, and the switches for mapping anywhere and locking; then the other difficulties.
    fn draw_work_tab(&mut self, c: &mut Canvas, ui: &mut Ui, body: [f32; 4], end: f64) -> f32 {
        let Some(s) = self.collab.as_ref() else {
            return 0.0;
        };
        let [bx, by, bw, _] = body;
        let x = bx + PAD - 1.0;
        let right = bx + bw - PAD + 1.0;
        let inner = right - x;
        let top = by - self.panel_scroll;
        let mut ry = top;
        let hosting = s.state.host == s.me;
        let open = s.sync.open();
        let mappers: Vec<(u32, String, u32)> = s
            .state
            .participants
            .iter()
            .filter(|p| p.role == Role::Mapper)
            .map(|p| (p.id, p.name.clone(), p.colour))
            .collect();
        let pick = self.give_pick;
        // The names to give something to, under the button that asked.
        let picker = |c: &mut Canvas,
                      ui: &mut Ui,
                      ry: &mut f32,
                      act: &dyn Fn(Option<u32>) -> CollabAct,
                      nobody: bool| {
            if mappers.is_empty() {
                for l in super::chrome::wrap(
                    SMALL,
                    inner,
                    tr!(
                        "No mappers in the session yet. Invite friends (F9), or make someone a mapper in People.",
                        "マッパーがまだいません。フレンドを招待 (F9) するか、参加者でマッパーにしてください。",
                        "Мапперов пока нет. Позови друзей (F9) или сделай кого-то маппером во вкладке Люди."
                    ),
                ) {
                    c.text(x, *ry + 1.0, SMALL, rgba(AMBER, 1.0), &l);
                    *ry += 6.5;
                }
                *ry += 3.0;
                return;
            }
            let mut cx = x;
            let mut names: Vec<(Option<u32>, String, u32)> = mappers
                .iter()
                .map(|(id, n, col)| (Some(*id), n.clone(), *col))
                .collect();
            if nobody {
                names.insert(0, (None, tr!("Nobody", "なし", "Никто").to_string(), DIM));
            }
            for (who, name, col) in names {
                let bw = (Canvas::text_width(SMALL, &name) + 14.0).min(inner);
                if cx + bw > right {
                    cx = x;
                    *ry += BTN_H + 3.0;
                }
                let r = ui.button(c, cx, *ry, bw, "", Look::Plain, act(who));
                c.rect(r[0] + 3.0, r[1] + 3.0, 4.0, 4.0, rgba(col, 1.0));
                c.text(
                    r[0] + 9.0,
                    r[1] + 2.5,
                    SMALL,
                    rgba(0xffffff, 1.0),
                    &Canvas::fit(SMALL, bw - 11.0, &name),
                );
                cx += bw + 3.0;
            }
            *ry += BTN_H + 5.0;
        };
        if s.state.mode == Mode::Collab {
            let doc = s.state.documents.iter().find(|d| d.diff == open);
            let diff = s.diff_name(open).to_string();
            c.text(
                x,
                ry + 1.0,
                SMALL,
                rgba(DIM, 1.0),
                &Canvas::fit(
                    SMALL,
                    inner,
                    &tr!("PARTS OF {diff}", "{diff} のパート", "ЧАСТИ {diff}"; [] [diff])
                        .to_uppercase(),
                ),
            );
            ry += 9.0;
            let parts = doc.map(|d| d.parts.clone()).unwrap_or_default();
            if parts.is_empty() {
                for l in super::chrome::wrap(
                    SMALL,
                    inner,
                    tr!(
                        "No parts yet: only the host places notes here.",
                        "パートはまだありません。ここに置けるのはホストだけです。",
                        "Частей пока нет: ноты тут ставит только хост."
                    ),
                ) {
                    c.text(x, ry + 1.0, SMALL, rgba(GREY, 1.0), &l);
                    ry += 6.5;
                }
                ry += 3.0;
            }
            for (i, p) in parts.iter().enumerate() {
                let span = format!("{}–{}", short_time(p.from), part_end(p.to, end));
                c.text(x, ry + 2.0, TEXT, rgba(0xffffff, 1.0), &span);
                let (who, col) = match p.assignee {
                    Some(a) => (
                        s.participant(a).map_or("?".to_string(), |q| q.name.clone()),
                        s.colour(a),
                    ),
                    None => (tr!("everyone", "全員", "все").to_string(), 0x9a9aa8),
                };
                let nx = x + 64.0;
                c.rect(nx, ry + 3.5, 5.0, 5.0, rgba(col, 1.0));
                c.text(
                    nx + 8.0,
                    ry + 2.5,
                    5.5,
                    rgba(0xe0e0ea, 1.0),
                    &Canvas::fit(5.5, right - nx - 30.0, &who),
                );
                if hosting && p.assignee.is_some() {
                    let r = ui.button(
                        c,
                        right - 22.0,
                        ry + 1.0,
                        22.0,
                        tr!("Take", "外す", "Убрать"),
                        Look::Plain,
                        CollabAct::RemovePart(i as u32),
                    );
                    ui.tip(
                        r,
                        tr!(
                            "Takes this part back\nNotes already placed in it stay.",
                            "このパートを外します\n置かれたノーツは残ります。",
                            "Забрать эту часть\nУже поставленные ноты останутся."
                        )
                        .to_string(),
                    );
                }
                ry += 12.0;
            }
            if hosting {
                ry += 3.0;
                let (from, to, selected) = self.part_range_and_source();
                let span = format!("{}–{}", short_time(from), part_end(to - 1.0, end));
                let label =
                    tr!("Give {span} to...", "{span} を割り当て...", "Отдать {span}..."; [] [span]);
                let on = pick == Some(GivePick::Part);
                ui.button(
                    c,
                    x,
                    ry,
                    inner,
                    &label,
                    if on { Look::Plain } else { Look::Main },
                    CollabAct::Picker(GivePick::Part),
                );
                ry += BTN_H + 3.0;
                let how = if selected {
                    tr!(
                        "The selected notes' stretch (select none: from the playhead).",
                        "選択したノーツの範囲 (選択なし: 再生位置から)。",
                        "Отрезок выделенных нот (без выделения: от курсора)."
                    )
                } else {
                    tr!(
                        "From the playhead on (or select notes to give their stretch).",
                        "再生位置から (ノーツを選ぶとその範囲)。",
                        "От курсора (или выдели ноты, чтобы отдать их отрезок)."
                    )
                };
                for l in super::chrome::wrap(SMALL, inner, how) {
                    c.text(x, ry + 1.0, SMALL, rgba(GREY, 1.0), &l);
                    ry += 6.5;
                }
                ry += 3.0;
                if on {
                    picker(
                        c,
                        ui,
                        &mut ry,
                        &|who| CollabAct::GivePart(who.unwrap_or(0)),
                        false,
                    );
                }
                ry += ui.toggle(
                    c,
                    x,
                    ry,
                    inner,
                    s.free(),
                    tr!(
                        "Everyone maps anywhere",
                        "誰でもどこでもマッピング",
                        "Все маппят где угодно"
                    ),
                    tr!(
                        "Mappers place notes anywhere but in the parts given to someone.",
                        "誰かに割り当てたパート以外なら、マッパーはどこにでも置けます。",
                        "Мапперы ставят ноты везде, кроме частей, отданных кому-то."
                    ),
                    CollabAct::Free,
                );
                let locked = doc.is_some_and(|d| d.locked);
                ry += ui.toggle(
                    c,
                    x,
                    ry,
                    inner,
                    locked,
                    &tr!("Lock {diff}", "{diff} をロック", "Закрыть {diff}"; [] [diff]),
                    tr!(
                        "Only you edit it while it's locked; the parts stay.",
                        "ロック中はあなただけが編集できます。パートは残ります。",
                        "Пока закрыта, правишь только ты; части остаются."
                    ),
                    CollabAct::Lock(open),
                );
            }
            if s.state.documents.len() > 1 {
                ry += 4.0;
                c.text(
                    x,
                    ry + 1.0,
                    SMALL,
                    rgba(DIM, 1.0),
                    tr!("OTHER DIFFICULTIES", "他の難易度", "ДРУГИЕ СЛОЖНОСТИ"),
                );
                ry += 9.0;
            }
        }
        for d in &s.state.documents {
            if s.state.mode == Mode::Collab && d.diff == open {
                continue;
            }
            let is_open = d.diff == open;
            let controls = hosting && s.state.mode == Mode::Guest;
            let row_h = if controls { 32.0 } else { 20.0 };
            if is_open {
                c.rect(bx + 2.0, ry, bw - 4.0, row_h, rgba(VIOLET, 0.12));
                c.rect(bx + 2.0, ry, 2.0, row_h, rgba(VIOLET, 1.0));
            }
            c.text(
                x + 2.0,
                ry + 2.0,
                TEXT,
                rgba(0xffffff, 1.0),
                &Canvas::fit(TEXT, inner - 40.0, &d.name),
            );
            if is_open {
                c.text_right(
                    right,
                    ry + 3.0,
                    SMALL,
                    rgba(0xc8c0ff, 1.0),
                    tr!("open here", "表示中", "открыта"),
                );
            } else {
                let r = ui.button(
                    c,
                    right - 32.0,
                    ry + 1.5,
                    32.0,
                    tr!("Open", "開く", "Открыть"),
                    Look::Plain,
                    CollabAct::OpenDiff(d.diff),
                );
                ui.tip(
                    r,
                    tr!(
                        "Opens this difficulty here\nYou stay in the session.",
                        "この難易度を開きます\nセッションには残ります。",
                        "Открыть эту сложность\nТы остаёшься в сессии."
                    )
                    .to_string(),
                );
            }
            let (state, col) = if d.accepted {
                (
                    tr!("Final: done, locked", "完成: ロック済み", "Готова: закрыта").to_string(),
                    GREEN,
                )
            } else if d.locked {
                (
                    tr!(
                        "Locked: only the host edits it",
                        "ロック: ホストのみ編集",
                        "Закрыта: правит только хост"
                    )
                    .to_string(),
                    AMBER,
                )
            } else {
                match d.owner {
                    Some(o) => {
                        let name = s.participant(o).map_or("?", |p| p.name.as_str());
                        (
                            tr!("{name} maps it", "{name} が担当", "Маппит {name}"; [] [name]),
                            s.colour(o),
                        )
                    }
                    None if s.state.mode == Mode::Guest => (
                        tr!(
                            "Nobody's yet: only the host edits it",
                            "未割り当て: ホストのみ編集",
                            "Ничья пока: правит только хост"
                        )
                        .to_string(),
                        GREY,
                    ),
                    None => (String::new(), GREY),
                }
            };
            c.text(
                x + 2.0,
                ry + 10.5,
                SMALL,
                rgba(col, 1.0),
                &Canvas::fit(SMALL, inner - 4.0, &state),
            );
            if controls {
                let by = ry + 19.0;
                let picking = pick == Some(GivePick::Diff(d.diff));
                let gr = ui.button(
                    c,
                    x + 2.0,
                    by,
                    56.0,
                    tr!("Give to...", "割り当て...", "Отдать..."),
                    if picking { Look::Main } else { Look::Plain },
                    CollabAct::Picker(GivePick::Diff(d.diff)),
                );
                ui.tip(
                    gr,
                    tr!(
                        "Gives this difficulty to a mapper\nThey map it; it opens for them.",
                        "この難易度をマッパーに割り当てます\n相手の画面で開きます。",
                        "Отдать сложность мапперу\nОна у него сразу откроется."
                    )
                    .to_string(),
                );
                let lr = ui.button(
                    c,
                    x + 61.0,
                    by,
                    44.0,
                    if d.locked {
                        tr!("Unlock", "ロック解除", "Открыть")
                    } else {
                        tr!("Lock", "ロック", "Закрыть")
                    },
                    Look::Plain,
                    CollabAct::Lock(d.diff),
                );
                ui.tip(
                    lr,
                    tr!(
                        "Lock: only you edit it\nThe guest keeps it, and can't change it till you unlock.",
                        "ロック: あなただけが編集\nゲストの担当のまま、解除まで変更できません。",
                        "Закрыть: правишь только ты\nГость её не теряет, но менять не может, пока не откроешь."
                    )
                    .to_string(),
                );
                let fr = ui.button(
                    c,
                    x + 108.0,
                    by,
                    56.0,
                    if d.accepted {
                        tr!("Not final", "完成を取消", "Не готова")
                    } else {
                        tr!("Mark final", "完成にする", "Готова")
                    },
                    Look::Plain,
                    CollabAct::Final(d.diff),
                );
                ui.tip(
                    fr,
                    tr!(
                        "Final: you take the guest's difficulty as done\nIt's locked; click again to reopen it.",
                        "完成: ゲストの難易度を完成として受け取ります\nロックされます。もう一度で再開。",
                        "Готово: принимаешь гостевую сложность\nОна закрывается; нажми ещё раз, чтобы вернуть."
                    )
                    .to_string(),
                );
            }
            ry += row_h + 4.0;
            if controls && pick == Some(GivePick::Diff(d.diff)) {
                let diff = d.diff;
                picker(
                    c,
                    ui,
                    &mut ry,
                    &|who| CollabAct::GiveDiff { diff, to: who },
                    true,
                );
            }
        }
        ry - top
    }

    /// The chat, or the open difficulty's review comments, newest at the bottom, over the box
    /// to type in. A time written in a line is a link to it; a comment's button resolves it.
    fn draw_talk_tab(&mut self, c: &mut Canvas, ui: &mut Ui, body: [f32; 4], to: Say) -> f32 {
        let Some(s) = self.collab.as_ref() else {
            return 0.0;
        };
        let [bx, by, bw, bh] = body;
        let x = bx + PAD - 1.0;
        let right = bx + bw - PAD + 1.0;
        let line_h = 8.0;
        let room = ((bh - 16.0) / line_h).floor().max(1.0) as usize;
        let mut lines: Vec<TalkLine> = match to {
            Say::Chat => s
                .chat
                .iter()
                .map(|(who, text)| {
                    let name = s.participant(*who).map_or("?", |p| p.name.as_str());
                    (
                        s.colour(*who),
                        format!("{name}: {text}"),
                        first_stamp(text),
                        None,
                    )
                })
                .collect(),
            Say::Note => {
                let mut notes: Vec<_> = s
                    .state
                    .comments
                    .iter()
                    .filter(|k| k.diff == s.sync.open())
                    .collect();
                notes.sort_by(|a, b| a.resolved.cmp(&b.resolved).then(a.time.total_cmp(&b.time)));
                notes
                    .into_iter()
                    .map(|k| {
                        let name = s.participant(k.author).map_or("?", |p| p.name.as_str());
                        (
                            s.colour(k.author),
                            format!("{} {name}: {}", stamp(k.time), k.text),
                            Some(k.time),
                            Some((k.id, k.resolved)),
                        )
                    })
                    .collect()
            }
        };
        if lines.is_empty() {
            let empty = match to {
                Say::Chat => tr!(
                    "No messages yet.",
                    "メッセージはまだありません。",
                    "Сообщений пока нет."
                ),
                Say::Note => tr!(
                    "No comments on this difficulty.",
                    "この難易度にコメントはありません。",
                    "Замечаний к этой сложности нет."
                ),
            };
            c.text(x, by + 2.0, SMALL, rgba(GREY, 1.0), empty);
        }
        // Chat: the newest lines; comments: the first ones (open ones lead).
        if to == Say::Chat && lines.len() > room {
            lines.drain(..lines.len() - room);
        }
        for (i, (colour, line, at, note)) in lines.into_iter().take(room).enumerate() {
            let ly = by + 1.0 + i as f32 * line_h;
            let alpha = if note.is_some_and(|(_, r)| r) {
                0.45
            } else {
                1.0
            };
            c.rect(x, ly + 1.0, 2.0, 5.0, rgba(colour, alpha));
            let text_w = if note.is_some() {
                right - x - 34.0
            } else {
                right - x - 4.0
            };
            let col = if at.is_some() && note.is_none() {
                0x9ec8ff
            } else {
                0xe0e0ea
            };
            c.text(
                x + 4.0,
                ly,
                SMALL,
                rgba(col, alpha),
                &Canvas::fit(SMALL, text_w, &line),
            );
            if let Some(at) = at {
                ui.push(
                    [bx + 2.0, ly - 1.0, text_w + 6.0, line_h],
                    CollabAct::Seek(at as i64),
                );
            }
            if let Some((id, resolved)) = note {
                ui.button(
                    c,
                    right - 28.0,
                    ly - 1.5,
                    28.0,
                    if resolved {
                        tr!("Reopen", "再開", "Вернуть")
                    } else {
                        tr!("Done", "解決", "Готово")
                    },
                    Look::Plain,
                    CollabAct::Resolve(id),
                );
            }
        }
        self.draw_say(c, ui, [x, by + bh - 12.0, right - x, 10.0], to);
        bh
    }

    /// The box to type a chat line or a comment in: what is typed, or a hint to click.
    fn draw_say(&self, c: &mut Canvas, ui: &mut Ui, r: [f32; 4], to: Say) {
        let typed = match &self.typing {
            Some((Field::Say(t), text)) if *t == to => Some(text.clone()),
            _ => None,
        };
        c.round_rect(
            [r[0], r[1]],
            [r[2], r[3]],
            2.0,
            rgba(0x1c1c2a, 1.0),
            rgba(if typed.is_some() { VIOLET } else { 0x3c3c58 }, 1.0),
            1.0,
        );
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
        while Canvas::text_width(SMALL, shown) > r[2] - 6.0 && !shown.is_empty() {
            let cut = shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i);
            shown = &shown[cut..];
        }
        c.text(r[0] + 3.0, r[1] + 2.5, SMALL, rgba(col, 1.0), shown);
        ui.push(r, CollabAct::Say(to));
    }

    /// Along the whole-song bar in a session: the open difficulty's parts in their mappers'
    /// colours on its top edge, and where each of the others is (a line in their colour, dim
    /// when they are on another difficulty); hovering one says who.
    pub(super) fn draw_collab_overview(&mut self, c: &mut Canvas, end: f64) {
        let Some(s) = self.collab.as_ref() else {
            return;
        };
        let [x, y, w, h] = self.layout.overview;
        let px = |t: f64| x + (t / end).clamp(0.0, 1.0) as f32 * w;
        let open = s.sync.open();
        if let Some(doc) = s.state.documents.iter().find(|d| d.diff == open) {
            for p in &doc.parts {
                let (a, b) = (px(p.from), px(p.to.min(FREE_TO)));
                let (col, alpha) = p
                    .assignee
                    .map_or((0x9A9AA8, 0.4), |who| (s.colour(who), 0.95));
                c.rect(a, y, (b - a).max(1.0), 2.0, rgba(col, alpha));
            }
        }
        let mut tips = Vec::new();
        for (&who, r) in &s.remotes {
            let col = s.colour(who);
            let here = r.diff == open;
            let rx = px(r.time);
            c.rect(
                rx - 0.75,
                y - 2.0,
                1.5,
                h + 3.0,
                rgba(col, if here { 1.0 } else { 0.45 }),
            );
            // A notch on top to grab the eye.
            c.rect(
                rx - 2.0,
                y - 3.0,
                4.0,
                2.0,
                rgba(col, if here { 1.0 } else { 0.45 }),
            );
            let name = s.participant(who).map_or("?", |p| p.name.as_str());
            let at = short_time(r.time);
            let tip = if here {
                tr!("{name} is at {at}", "{name}: {at}", "{name} на {at}"; [] [name, at])
            } else {
                let diff = s.diff_name(r.diff).to_string();
                tr!("{name} is at {at} in {diff}", "{name}: {diff} の {at}", "{name} на {at} в {diff}"; [] [name, at, diff])
            };
            tips.push(([rx - 3.0, y - 3.0, 6.0, h + 4.0], tip));
        }
        self.tips.extend(tips);
    }

    /// The parts button's stretch (ms, `from..to`) and whether it comes from selected notes.
    fn part_range_and_source(&self) -> (f64, f64, bool) {
        let selected = self
            .ed
            .chart
            .objects
            .iter()
            .any(|o| self.ed.selection.contains(&o.id));
        let (from, to) = self.part_range().unwrap_or((0.0, 1.0));
        (from, to, selected)
    }

    /// Whether a click on a button that asks twice (kicking, ending) is the second one; the
    /// first only arms it.
    pub(super) fn confirmed(&mut self, what: Armed) -> bool {
        let now = Instant::now();
        match self.armed {
            Some((a, at)) if a == what && now.duration_since(at) < ARMED_FOR => {
                self.armed = None;
                true
            }
            _ => {
                self.armed = Some((what, now));
                false
            }
        }
    }
}
