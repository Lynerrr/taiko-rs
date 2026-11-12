//! A mapping session on the editor's screen: who is in it (a panel over the preview's corner),
//! where the others are on the timeline (their playheads, pointers and selections in their
//! colours), the collab parts of the open difficulty along the ruler, and following someone.
//! None of it is drawn, and nothing here runs, while editing alone. It also keeps the session's
//! connection: what the server sends goes to the session, our edits and place go out, and a
//! dropped connection is resumed.

use std::time::{Duration, Instant};

use super::{Btn, Drag, NOTE_R, RULER, View};
use crate::editor::collab::Sync;
use crate::editor::collab::connect::Ready;
use crate::editor::collab::session::{Notice, Session};
use crate::hit::Canvas;
use crate::render::rgba;
use taiko_proto::mapping::{HostCmd, Link, LinkEvent, MapUp, Role};
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CollabAct {
    /// Keep to this player's playhead, or stop (pressed again).
    Follow(u32),
    /// Colour the notes by who made them.
    ByAuthor,
    /// Start a mapping session for this set (only offered by a server that runs them).
    Start,
}

/// The participants panel's width and row height.
const PANEL_W: f32 = 118.0;
const ROW: f32 = 10.0;
const TEXT: f32 = 6.0;

fn role_tag(r: Role) -> &'static str {
    match r {
        Role::Host => tr!("host", "ホスト", "хост"),
        Role::Mapper => tr!("mapper", "マッパー", "маппер"),
        Role::Modder => tr!("modder", "モッダー", "моддер"),
        Role::Viewer => tr!("viewer", "閲覧", "зритель"),
    }
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
        if a == CollabAct::Start {
            self.session_wanted = true;
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
            CollabAct::Start => {}
        }
    }

    /// Starting a session was asked for: the screen opens it for the set.
    pub fn take_session_request(&mut self) -> bool {
        std::mem::take(&mut self.session_wanted)
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
        let mut session = Session::new(sync, r.state, r.me);
        for d in r.later {
            session.handle(&mut self.ed, d, 0.0);
        }
        self.collab = Some(Box::new(session));
        self.link = Some(link);
        self.relink = Some(relink);
        Ok(())
    }

    /// The host's say in the session (an invite, a part given to someone...).
    pub fn send_host(&self, cmd: HostCmd) {
        if let Some(l) = &self.link {
            l.send(MapUp::Host(cmd));
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
                    Some(Notice::Refused(why)) => self.status = why,
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

    /// The panel of who is in the session, over the preview's top right corner: each player's
    /// colour, name and role, the difficulty they're on when it isn't ours, and whether they
    /// are away. A click on someone follows them.
    pub(super) fn draw_participants(&mut self, c: &mut Canvas) {
        let Some(s) = self.collab.as_ref() else {
            self.draw_start_session(c);
            return;
        };
        let now = s.now();
        let [px, py, pw, _] = self.layout.preview;
        let x = px + pw - PANEL_W - 4.0;
        let mut rows: Vec<_> = s.state.participants.iter().collect();
        rows.sort_by_key(|p| (p.role != Role::Host, !p.online, p.id));
        let h = 14.0 + rows.len() as f32 * ROW + 13.0;
        let y = py + 4.0;
        c.round_rect(
            [x, y],
            [PANEL_W, h],
            3.0,
            rgba(0x0c0c14, 0.88),
            rgba(0x2c2c3c, 1.0),
            1.0,
        );
        let title = format!(
            "{} · {}",
            tr!(
                "Mapping session",
                "マッピングセッション",
                "Совместный маппинг"
            ),
            rows.iter().filter(|p| p.online).count()
        );
        // Between connections: our edits wait to go out.
        let title = if self.link.is_none() && self.relink.is_some() {
            let n = s.sync.pending();
            tr!("Offline · {n} unsent", "オフライン · 未送信 {n}", "Нет связи · не отпр.: {n}"; [] [n])
        } else {
            title
        };
        c.text(
            x + 4.0,
            y + 3.0,
            TEXT,
            rgba(0xffffff, 0.9),
            &Canvas::fit(TEXT, PANEL_W - 8.0, &title),
        );
        let mut buttons = Vec::new();
        let mut ry = y + 14.0;
        for p in rows {
            let me = p.id == s.me;
            let following = s.follow == Some(p.id);
            if following {
                c.rect(x + 1.0, ry - 1.0, PANEL_W - 2.0, ROW, rgba(p.colour, 0.22));
            }
            c.rect(x + 4.0, ry + 1.0, 6.0, 6.0, rgba(p.colour, 1.0));
            let state = if !p.online {
                Some(tr!("offline", "オフライン", "не в сети").to_string())
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
            c.text(
                x + 13.0,
                ry,
                TEXT,
                rgba(0xffffff, alpha),
                &Canvas::fit(TEXT, 58.0, &name),
            );
            let tag = state.unwrap_or_else(|| role_tag(p.role).to_string());
            c.text_right(
                x + PANEL_W - 4.0,
                ry,
                TEXT,
                rgba(0x9a9ab0, alpha),
                &Canvas::fit(TEXT, 40.0, &tag),
            );
            if !me && p.online {
                buttons.push((
                    [x + 1.0, ry - 1.0, PANEL_W - 2.0, ROW],
                    Btn::Collab(CollabAct::Follow(p.id)),
                ));
            }
            ry += ROW;
        }
        let by_author = s.by_author;
        let label = if by_author {
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
            &Canvas::fit(TEXT, PANEL_W - 8.0, label),
        );
        buttons.push((
            [x + 1.0, ry, PANEL_W - 2.0, 11.0],
            Btn::Collab(CollabAct::ByAuthor),
        ));
        self.buttons.extend(buttons);
    }

    /// Alone, on a server that runs mapping sessions: a button to start one for this set.
    fn draw_start_session(&mut self, c: &mut Canvas) {
        if !self.sessions || self.link.is_some() {
            return;
        }
        let [px, py, pw, _] = self.layout.preview;
        let label = tr!(
            "Start mapping session",
            "マッピングセッションを開始",
            "Начать совместный маппинг"
        );
        let w = (Canvas::text_width(TEXT, label) + 10.0).min(PANEL_W);
        let (x, y) = (px + pw - w - 4.0, py + 4.0);
        c.round_rect(
            [x, y],
            [w, 12.0],
            3.0,
            rgba(0x0c0c14, 0.88),
            rgba(0x4c6cdc, 1.0),
            1.0,
        );
        c.text(
            x + 5.0,
            y + 3.0,
            TEXT,
            rgba(0xffffff, 0.92),
            &Canvas::fit(TEXT, w - 10.0, label),
        );
        self.buttons
            .push(([x, y, w, 12.0], Btn::Collab(CollabAct::Start)));
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
    use super::super::{Keys, Layout, Mode};
    use super::*;
    use crate::editor::collab::session::tests::session;
    use taiko_proto::mapping::MapDown;

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
        v.layout = Layout::new(853.0, Mode::Compose, false);
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
        let c = draw(&mut v);
        assert!(has_text(&c, "Mapping session · 3"));
        assert!(has_text(&c, "host (you)"));
        assert!(has_text(&c, "anna"), "in the panel and on their playhead");
        // Everyone but us can be followed; the colour toggle is the last row.
        let rows = collab_buttons(&v);
        let acts: Vec<CollabAct> = rows.iter().map(|r| r.1).collect();
        assert_eq!(
            acts,
            vec![
                CollabAct::Follow(2),
                CollabAct::Follow(3),
                CollabAct::ByAuthor
            ]
        );
        // Every row fits the preview's corner.
        let [px, py, pw, ph] = v.layout.preview;
        for ([x, y, w, h], _) in &rows {
            assert!(*x >= px && x + w <= px + pw && *y >= py && y + h <= py + ph);
        }
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
    fn a_session_is_offered_only_by_a_server_that_runs_them() {
        let mut v = view(false);
        let c = draw(&mut v);
        assert!(!has_text(&c, "Start mapping session"));
        v.sessions = true;
        let c = draw(&mut v);
        assert!(has_text(&c, "Start mapping session"));
        let acts: Vec<CollabAct> = collab_buttons(&v).iter().map(|r| r.1).collect();
        assert_eq!(acts, vec![CollabAct::Start]);
        v.press(Btn::Collab(CollabAct::Start), Keys::default(), None);
        assert!(v.take_session_request());
        assert!(!v.take_session_request(), "taken once");
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
}
