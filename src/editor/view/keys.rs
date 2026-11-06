//! The keyframe page of the Gimmicks panel and the track graph under the timeline: pick the
//! playfield or a group of notes, pick a property, and add, drag, ease and delete keys on its
//! track the way an animation track is edited in a video editor.

use super::{Btn, Drag, Mode, View, fmt_time, inside};
use crate::editor::keyframes::Subject;
use crate::render::{Canvas, rgba};
use crate::tko::gimmick::{FieldProp, NoteProp, PropGroup};
use crate::tko::track::Key;

const KEY: u32 = 0xFCB806;
const KEY_ON: u32 = 0xFFFFFF;

/// What the keyframe page can be told to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum KAct {
    /// The playfield (`true`) or a group of notes.
    Target(bool),
    Group(i32),
    NewGroup,
    DeleteGroup,
    SelectGroup,
    Prop(usize),
    /// A group of playfield properties (Move, Look, Clones).
    Tab(usize),
    KeyHere,
    Ease,
    Value(i32),
    DeleteKey,
    MakeTko,
}

/// How fast or backwards the scroll graph shows at most; faster keys sit at its edge.
const SCROLL_SHOWN: f64 = 8.0;

impl View {
    /// The track being edited.
    pub(super) fn subject(&self) -> Subject {
        if self.scroll_mode() {
            Subject::Scroll
        } else if self.k_field {
            Subject::Field(FieldProp::ALL[self.k_prop.min(FieldProp::ALL.len() - 1)])
        } else {
            Subject::Group(
                self.k_group,
                NoteProp::ALL[self.k_prop.min(NoteProp::ALL.len() - 1)],
            )
        }
    }

    /// Whether the graph under the timeline is the keyframe graph.
    pub(super) fn keyframe_mode(&self) -> bool {
        self.mode == Mode::Effects && self.g_page == 2 && self.ed.is_tko() || self.scroll_mode()
    }

    /// Whether the graph under the timeline is the shared scroll's (the Motion page's Scroll
    /// tab).
    pub(super) fn scroll_mode(&self) -> bool {
        self.motion_mode() && self.m_tab == 2
    }

    fn group_count(&self) -> usize {
        self.ed.gimmick().map_or(0, |g| g.groups.len())
    }

    pub(super) fn draw_keys_page(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let white = rgba(0xffffff, 1.0);
        if !self.ed.is_tko() {
            self.draw_tko_explainer(c, x, y, inner);
            return;
        }
        let groups = self.group_count();
        let mut items = vec![
            (
                "Playfield".to_string(),
                Btn::K(KAct::Target(true)),
                self.k_field,
            ),
            (
                if groups == 0 {
                    "Notes".to_string()
                } else {
                    format!("Group {}/{}", self.k_group + 1, groups)
                },
                Btn::K(KAct::Target(false)),
                !self.k_field,
            ),
        ];
        if !self.k_field && groups > 0 {
            items.push(("<".to_string(), Btn::K(KAct::Group(-1)), false));
            items.push((">".to_string(), Btn::K(KAct::Group(1)), false));
            items.push(("Select".to_string(), Btn::K(KAct::SelectGroup), false));
            items.push(("Delete".to_string(), Btn::K(KAct::DeleteGroup), false));
        }
        items.push((
            "+ Group from selection".to_string(),
            Btn::K(KAct::NewGroup),
            false,
        ));
        self.row(c, x, y, &items);
        y += 20.0;
        if !self.k_field && groups == 0 {
            c.text(
                x,
                y,
                6.5,
                dim,
                &Canvas::fit(
                    6.5,
                    inner,
                    "Select notes on the timeline, then add them as a group to move them.",
                ),
            );
            return;
        }
        if self.k_field {
            let tabs: Vec<(String, Btn, bool)> = PropGroup::ALL
                .iter()
                .enumerate()
                .map(|(i, g)| (g.label().to_string(), Btn::K(KAct::Tab(i)), self.k_tab == i))
                .collect();
            self.row(c, x, y, &tabs);
            y += 19.0;
        }
        // (name, index into the property list, what it does)
        let names: Vec<(&str, usize, &str)> = if self.k_field {
            let tab = PropGroup::ALL[self.k_tab.min(PropGroup::ALL.len() - 1)];
            FieldProp::ALL
                .iter()
                .enumerate()
                .filter(|(_, p)| p.group() == tab)
                .map(|(i, p)| (p.name(), i, p.about()))
                .collect()
        } else {
            NoteProp::ALL
                .iter()
                .enumerate()
                .map(|(i, p)| (p.name(), i, p.about()))
                .collect()
        };
        let props: Vec<(String, Btn, bool)> = names
            .iter()
            .map(|(n, i, _)| (n.to_string(), Btn::K(KAct::Prop(*i)), self.k_prop == *i))
            .collect();
        let first = self.buttons.len();
        self.row(c, x, y, &props);
        for (k, (name, _, about)) in names.iter().enumerate() {
            let r = self.buttons[first + k].0;
            self.tips.push((r, format!("{name}\n{about}")));
        }
        y += 20.0;
        let s = self.subject();
        let now = self.ed.value_of(s, self.ed.time);
        let keys = self.ed.track_of(s).map_or(0, |t| t.keys.len());
        c.text(
            x,
            y,
            6.5,
            white,
            &Canvas::fit(
                6.5,
                inner,
                &format!(
                    "{} at {}: {:.2}   {keys} keys",
                    s.name(),
                    fmt_time(self.ed.time),
                    now
                ),
            ),
        );
        y += 12.0;
        let key = self
            .k_sel
            .and_then(|i| self.ed.track_of(s).and_then(|t| t.keys.get(i)).copied());
        let ease = key.map_or(self.k_ease, |k| k.ease);
        let mut row = vec![
            ("Key here".to_string(), Btn::K(KAct::KeyHere), false),
            (format!("Ease: {}", ease.label()), Btn::K(KAct::Ease), false),
        ];
        if key.is_some() {
            row.push(("Value -".to_string(), Btn::K(KAct::Value(-1)), false));
            row.push(("Value +".to_string(), Btn::K(KAct::Value(1)), false));
            row.push(("Delete key".to_string(), Btn::K(KAct::DeleteKey), false));
        }
        self.row(c, x, y, &row);
        y += 20.0;
        let hint = match key {
            Some(k) => format!(
                "key at {}: {:.2}   shift: x10, alt: x0.1",
                fmt_time(k.t),
                k.v
            ),
            None => {
                "click the graph below to add a key, drag to move it, right-click to delete".into()
            }
        };
        c.text(x, y, 6.0, dim, &Canvas::fit(6.0, inner, &hint));
    }

    pub(super) fn press_key_action(&mut self, a: KAct, shift: bool, alt: bool) {
        let step = if shift {
            10.0
        } else if alt {
            0.1
        } else {
            1.0
        };
        match a {
            KAct::Target(field) => {
                self.k_field = field;
                self.k_prop = 0;
                self.k_tab = 0;
                self.k_sel = None;
            }
            KAct::Group(d) => {
                let n = self.group_count() as i32;
                if n > 0 {
                    self.k_group = (self.k_group as i32 + d).rem_euclid(n) as usize;
                    self.k_sel = None;
                }
            }
            KAct::NewGroup => match self.ed.add_group_from_selection() {
                Ok(i) => {
                    self.k_field = false;
                    self.k_group = i;
                    self.k_prop = 0;
                    self.k_tab = 0;
                    self.k_sel = None;
                    self.status = format!("group {} made", i + 1);
                }
                Err(e) => self.status = e,
            },
            KAct::DeleteGroup => {
                if self.ed.delete_group(self.k_group) {
                    self.k_group = self.k_group.saturating_sub(1);
                    self.k_sel = None;
                    self.mark_stale();
                }
            }
            KAct::SelectGroup => self.ed.select_group(self.k_group),
            KAct::Prop(i) => {
                self.k_prop = i;
                self.k_sel = None;
            }
            KAct::Tab(i) => {
                self.k_tab = i.min(PropGroup::ALL.len() - 1);
                // The first property of the group, so the graph below is one of its own.
                if let Some(at) = FieldProp::ALL
                    .iter()
                    .position(|p| p.group() == PropGroup::ALL[self.k_tab])
                {
                    self.k_prop = at;
                }
                self.k_sel = None;
            }
            KAct::KeyHere => {
                let (s, t) = (self.subject(), self.ed.time);
                if self.ed.key_here(s, t, self.k_ease) {
                    self.k_sel = self.ed.track_of(s).and_then(|tr| tr.near(t, 0.0));
                    self.mark_stale();
                }
            }
            KAct::Ease => {
                let s = self.subject();
                match self
                    .k_sel
                    .and_then(|i| self.ed.track_of(s).and_then(|t| t.keys.get(i)).copied())
                {
                    Some(k) => {
                        let ease = k.ease.next();
                        if let Some(i) = self.k_sel
                            && self.ed.move_key(s, i, Key { ease, ..k }, false)
                        {
                            self.k_ease = ease;
                            self.mark_stale();
                        }
                    }
                    None => self.k_ease = self.k_ease.next(),
                }
            }
            KAct::Value(dir) => {
                let s = self.subject();
                let (lo, hi) = s.range();
                let unit = ((hi - lo) / 100.0).max(0.01) * step;
                if let Some(i) = self.k_sel
                    && let Some(k) = self.ed.track_of(s).and_then(|t| t.keys.get(i)).copied()
                    && self.ed.move_key(
                        s,
                        i,
                        Key {
                            v: k.v + dir as f64 * unit,
                            ..k
                        },
                        false,
                    )
                {
                    self.mark_stale();
                }
            }
            KAct::DeleteKey => {
                let s = self.subject();
                if let Some(i) = self.k_sel.take()
                    && self.ed.remove_key(s, i)
                {
                    self.mark_stale();
                }
            }
            KAct::MakeTko => match self.ed.make_tko() {
                Ok(p) => {
                    self.status = format!("made {}", p.display());
                    // The page it was made from (clips or keyframes) goes on.
                    if self.g_page != 4 {
                        self.g_page = 2;
                    }
                    self.mark_stale();
                }
                Err(e) => self.status = e,
            },
        }
    }

    // ---- the graph ----

    /// The values the graph shows: the keys' own range, with room, and never a hair-thin span.
    pub(super) fn key_range(&self) -> (f64, f64) {
        if let Some(r) = self.k_frozen {
            return r;
        }
        let s = self.subject();
        let (mut lo, mut hi) = (s.default_value(), s.default_value());
        if s == Subject::Scroll {
            // Stopped and as mapped always show; a jump's huge speed for a millisecond would
            // flatten everything else, so the graph stops short of it.
            lo = 0.0;
        }
        if let Some(t) = self.ed.track_of(s) {
            for k in &t.keys {
                let v = if s == Subject::Scroll {
                    k.v.clamp(-SCROLL_SHOWN, SCROLL_SHOWN)
                } else {
                    k.v
                };
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let span = (hi - lo).max(s.min_span());
        let mid = (hi + lo) / 2.0;
        let (lo, hi) = (mid - span * 0.6, mid + span * 0.6);
        let (rlo, rhi) = s.range();
        (lo.max(rlo.min(lo)), hi.min(rhi.max(hi)))
    }

    pub(super) fn key_y(&self, v: f64, r: (f64, f64)) -> f32 {
        let [_, y, _, h] = self.layout.graph;
        let f = ((v - r.0) / (r.1 - r.0).max(1e-9)).clamp(-0.05, 1.05);
        y + h - 6.0 - f as f32 * (h - 12.0)
    }

    fn y_key(&self, py: f32, r: (f64, f64)) -> f64 {
        let [_, y, _, h] = self.layout.graph;
        r.0 + ((y + h - 6.0 - py) / (h - 12.0)) as f64 * (r.1 - r.0)
    }

    pub(super) fn draw_key_graph(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.graph;
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x101016, 0.95),
            rgba(0x2a2a38, 1.0),
            1.0,
        );
        let start = c.shapes.len();
        let s = self.subject();
        let r = self.key_range();
        for v in [s.default_value(), r.0, r.1] {
            let ly = self.key_y(v, r);
            c.rect(
                x + 1.0,
                ly,
                w - 2.0,
                1.0,
                rgba(0xffffff, if v == s.default_value() { 0.22 } else { 0.07 }),
            );
            c.text(
                x + 3.0,
                ly - 8.0,
                6.0,
                rgba(0x6a6a80, 1.0),
                &format!("{v:.1}"),
            );
        }
        // The track as the game evaluates it.
        let mut px = x;
        while px < x + w {
            let v = self.ed.value_of(s, self.t_at(px));
            c.rect(px, self.key_y(v, r) - 0.5, 2.0, 1.5, rgba(KEY, 0.9));
            px += 2.0;
        }
        if let Some(t) = self.ed.track_of(s) {
            for (i, k) in t.keys.iter().enumerate() {
                let kx = self.x_of(k.t);
                if kx < x - 6.0 || kx > x + w + 6.0 {
                    continue;
                }
                let on = self.k_sel == Some(i);
                c.circle(
                    kx,
                    self.key_y(k.v, r),
                    if on { 5.0 } else { 4.0 },
                    rgba(if on { KEY_ON } else { KEY }, 1.0),
                    rgba(0x000000, 0.8),
                    1.0,
                );
            }
        }
        let px = self.x_of(self.ed.time);
        c.rect(px - 0.75, y + 1.0, 1.5, h - 2.0, rgba(0xffffff, 0.8));
        c.clip_since(start, x, y, w, h);
        let hint = format!("{}: click add   drag move   right-click delete", s.name());
        c.text(
            x + w - 4.0 - Canvas::text_width(6.0, &hint),
            y + 3.0,
            6.0,
            rgba(0x6a6a80, 1.0),
            &hint,
        );
    }

    fn key_near(&self, x: f32, y: f32) -> Option<usize> {
        let s = self.subject();
        let r = self.key_range();
        self.ed
            .track_of(s)?
            .keys
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                (self.x_of(k.t) - x).abs() <= 7.0 && (self.key_y(k.v, r) - y).abs() <= 8.0
            })
            .min_by(|a, b| {
                (self.x_of(a.1.t) - x)
                    .abs()
                    .total_cmp(&(self.x_of(b.1.t) - x).abs())
            })
            .map(|(i, _)| i)
    }

    pub(super) fn key_graph_down(&mut self, x: f32, y: f32) {
        let s = self.subject();
        self.k_frozen = Some(self.key_range());
        let r = self.k_frozen.unwrap_or((0.0, 1.0));
        if let Some(i) = self.key_near(x, y) {
            self.k_sel = Some(i);
            self.drag = Drag::Key { index: i };
            return;
        }
        let t = self.ed.snapped(self.t_at(x));
        let v = (self.y_key(y, r) * 100.0).round() / 100.0;
        if self.ed.key(
            s,
            Key {
                t,
                v,
                ease: self.k_ease,
            },
            false,
        ) {
            self.k_sel = self.ed.track_of(s).and_then(|tr| tr.near(t, 0.0));
            if let Some(i) = self.k_sel {
                self.drag = Drag::Key { index: i };
            }
            self.mark_stale();
        } else {
            self.k_frozen = None;
        }
    }

    pub(super) fn key_graph_drag(&mut self, index: usize, x: f32, y: f32) {
        let s = self.subject();
        let r = self.k_frozen.unwrap_or_else(|| self.key_range());
        let Some(old) = self.ed.track_of(s).and_then(|t| t.keys.get(index)).copied() else {
            return;
        };
        let t = self.ed.snapped(self.t_at(x)).max(0.0);
        let v = (self.y_key(y, r) * 100.0).round() / 100.0;
        if self.ed.move_key(s, index, Key { t, v, ..old }, true) {
            let now = self
                .ed
                .track_of(s)
                .and_then(|tr| tr.near(t, 0.0))
                .unwrap_or(index);
            self.k_sel = Some(now);
            self.drag = Drag::Key { index: now };
            self.mark_stale();
        }
    }

    pub(super) fn key_graph_right(&mut self, x: f32, y: f32) -> bool {
        if !inside(self.layout.graph, x, y) {
            return false;
        }
        if let Some(i) = self.key_near(x, y) {
            let s = self.subject();
            if self.ed.remove_key(s, i) {
                self.k_sel = None;
                self.mark_stale();
            }
        }
        true
    }
}
