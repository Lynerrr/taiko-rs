//! The Motion page (F4 Effects, .tko maps): the notes picked on the timeline get a group of
//! their own, and that group an approach path (picked from ready-made ones and shaped by hand in
//! the preview, with a pace along it and moments it takes them on and off the lane) and what each
//! note does as its own hit nears (ready-made moves, or keys on a small graph).

use super::{Btn, Drag, Keys, Mode, View, inside};
use crate::editor::motion::{BEFORES, PACES, PATHS, before_tracks, pace_track};
use crate::game::{HIT_X, LANE_Y};
use crate::render::{Canvas, accent, rgba};
use crate::tko::gimmick::{FieldProp, NoteProp};
use crate::tko::path::PathTable;
use crate::tko::track::{Ease, Key};
use crate::video::FRAME_VW;
use std::time::Instant;

/// The Motion page's tab.
pub(super) const PAGE: u8 = 5;
/// How close (preview units) to a point a press takes it.
const GRAB: f32 = 6.0;
/// Two presses this close in time and place are a double click.
const DOUBLE_MS: u128 = 350;
/// Height of a line of the page's buttons.
const CHIP_ROW: f32 = 15.0;
/// What the before-the-hit graph can show: its name on the page, and the range it shows.
pub(super) const GRAPH_PROPS: [(NoteProp, &str, f64, f64); 9] = [
    (NoteProp::Opacity, "Fade", 0.0, 1.0),
    (NoteProp::Scale, "Size", 0.0, 2.0),
    (NoteProp::Dy, "Up/down", -250.0, 250.0),
    (NoteProp::Dx, "Sideways", -250.0, 250.0),
    (NoteProp::Swirl, "Turn", -180.0, 180.0),
    (NoteProp::Hidden, "Hide", 0.0, 1.0),
    (NoteProp::StretchX, "Wide", 0.0, 2.0),
    (NoteProp::StretchY, "Tall", 0.0, 2.0),
    (NoteProp::Tint, "Tint", 0.0, 1.0),
];
/// The graph's beats: this many before the hit on the left, and after it on the right.
const GRAPH_FAR: f64 = 4.0;
const GRAPH_AFTER: f64 = 0.5;

/// A button on the Motion page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MAct {
    /// The page's tab: 0 the path, 1 before the hit.
    Tab(u8),
    /// The notes picked get (or keep) their own group, with a path if it has none (on the path
    /// tab).
    UseSelection,
    Pick(usize),
    Path(usize),
    Pace(usize),
    Show,
    /// Onto the path (true) or back to the lane from the playhead.
    Switch(bool),
    /// On the path all the time.
    Always,
    SelectNotes,
    Copy,
    Paste,
    Clear,
    /// A ready-made before-the-hit move.
    Before(usize),
    /// The property the graph shows (in `GRAPH_PROPS`).
    Prop(usize),
}

impl View {
    pub(super) fn motion_mode(&self) -> bool {
        self.mode == Mode::Effects && self.g_page == PAGE && self.ed.is_tko()
    }

    /// The group being shaped, if it is still there.
    fn motion_group(&self) -> Option<usize> {
        let g = self.m_group?;
        (g < self.ed.gimmick()?.groups.len()).then_some(g)
    }

    pub(super) fn press_motion(&mut self, a: MAct) {
        let t = self.ed.snapped(self.ed.time);
        let g = self.motion_group();
        let done = match (a, g) {
            (MAct::Tab(i), _) => {
                self.m_tab = i;
                false
            }
            (MAct::Prop(i), _) => {
                self.m_prop = i.min(GRAPH_PROPS.len() - 1);
                false
            }
            (MAct::UseSelection, _) => match self.ed.motion_group() {
                Ok(g) => {
                    self.m_group = Some(g);
                    if self.m_tab == 0 {
                        if self.ed.path_of(g).is_none() {
                            self.ed.set_path_preset(g, 0);
                        }
                        self.status =
                            "the picked notes fly in along this path: shape it in the preview"
                                .into();
                    } else {
                        self.status =
                            "pick a ready-made move or add keys on the graph: each note does it as its own hit nears"
                                .into();
                    }
                    true
                }
                Err(e) => {
                    self.status = e;
                    false
                }
            },
            (MAct::Pick(i), _) => {
                self.m_group = Some(i);
                false
            }
            (MAct::Path(i), Some(g)) => self.ed.set_path_preset(g, i),
            (MAct::Pace(i), Some(g)) => self.ed.edit_path(g, false, |p| p.pace = pace_track(i)),
            (MAct::Show, Some(g)) => self.ed.edit_path(g, false, |p| p.show = !p.show),
            (MAct::Switch(on), Some(g)) => self.ed.path_switch(g, t, on),
            (MAct::Always, Some(g)) => self.ed.edit_path(g, false, |p| p.weight.keys.clear()),
            (MAct::SelectNotes, Some(g)) => {
                self.ed.select_group(g);
                false
            }
            (MAct::Copy, Some(g)) => {
                self.m_copied = self.ed.motion_of(g);
                self.status = "motion copied: pick other notes and paste it".into();
                false
            }
            (MAct::Paste, _) => match (self.m_copied.clone(), self.ed.motion_group()) {
                (Some(m), Ok(g)) => {
                    self.m_group = Some(g);
                    self.ed.paste_motion(g, &m)
                }
                (None, _) => {
                    self.status = "copy a group's motion first".into();
                    false
                }
                (_, Err(e)) => {
                    self.status = e;
                    false
                }
            },
            (MAct::Clear, Some(g)) => self.ed.clear_path(g),
            (MAct::Before(i), Some(g)) => {
                // Show what the move keys first.
                if let Some((prop, _)) = BEFORES.get(i).and_then(|b| b.1.first()) {
                    self.m_prop = GRAPH_PROPS
                        .iter()
                        .position(|gp| gp.0 == *prop)
                        .unwrap_or(self.m_prop);
                }
                self.ed.set_before_preset(g, i)
            }
            (_, None) => {
                self.status = "pick notes on the timeline first".into();
                false
            }
        };
        if done {
            self.mark_stale();
        }
    }

    /// Buttons as wide as their text, wrapping within `w`, smaller than the shelf's so the page
    /// fits the panel. Returns where the next line starts.
    fn chips(
        &mut self,
        c: &mut Canvas,
        x0: f32,
        mut y: f32,
        w: f32,
        items: &[(String, Btn, bool)],
    ) -> f32 {
        let mut x = x0;
        for (label, b, on) in items {
            let bw = (Canvas::text_width(7.0, label) + 10.0).min(w);
            if x > x0 && x + bw > x0 + w {
                x = x0;
                y += CHIP_ROW;
            }
            self.button(c, [x, y, bw, CHIP_ROW - 2.0], label, *b, *on);
            x += bw + 3.0;
        }
        y + CHIP_ROW
    }

    pub(super) fn draw_motion_page(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let small = 6.0;
        let about = if self.m_tab == 0 {
            "Pick notes on the timeline and give them a path: they fly in along it and land on the drum on time."
        } else {
            "Pick notes on the timeline and give them a move: each note does it as its own hit nears."
        };
        c.text(x, y, small, dim, &Canvas::fit(small, inner, about));
        y += 10.0;
        // The tab, what to shape (the picked notes, or a group already moving), and the groups.
        let picked = self.ed.selection.len();
        let what = if self.m_tab == 0 { "a path" } else { "a move" };
        let mut items = vec![
            ("Path".to_string(), Btn::M(MAct::Tab(0)), self.m_tab == 0),
            (
                "Before the hit".to_string(),
                Btn::M(MAct::Tab(1)),
                self.m_tab == 1,
            ),
            (
                if picked == 0 {
                    format!("Give the picked notes {what}")
                } else {
                    format!("Give the {picked} picked notes {what}")
                },
                Btn::M(MAct::UseSelection),
                false,
            ),
        ];
        if self.m_copied.is_some() {
            items.push(("Paste motion".to_string(), Btn::M(MAct::Paste), false));
        }
        let current = self.motion_group();
        if let Some(g) = self.ed.gimmick() {
            items.extend(
                g.groups
                    .iter()
                    .enumerate()
                    .filter(|(_, gr)| gr.path.is_some() || !gr.before.is_empty())
                    .map(|(i, gr)| {
                        (
                            format!("{} ({})", gr.name, gr.notes.len()),
                            Btn::M(MAct::Pick(i)),
                            current == Some(i),
                        )
                    }),
            );
        }
        y = self.chips(c, x, y, inner, &items) + 2.0;
        let Some(g) = current else {
            self.m_graph = [0.0; 4];
            return;
        };
        if self.m_tab == 0 {
            self.m_graph = [0.0; 4];
            self.draw_path_tab(c, g, x, y, inner);
        } else {
            self.draw_before_tab(c, g, x, y, inner);
        }
    }

    fn draw_path_tab(&mut self, c: &mut Canvas, g: usize, x: f32, mut y: f32, inner: f32) {
        let head = rgba(accent(), 1.0);
        let path = self.ed.path_of(g).cloned();
        // Two columns: the path's shape on the left, when and what else on the right.
        let col = (inner - 10.0) / 2.0;
        let right = x + col + 10.0;
        let top = y;
        c.text(x, y, 6.5, head, "Where they come from");
        let items: Vec<(String, Btn, bool)> = PATHS
            .iter()
            .enumerate()
            .map(|(i, (name, pts))| {
                (
                    name.to_string(),
                    Btn::M(MAct::Path(i)),
                    path.as_ref().is_some_and(|p| p.points == pts.to_vec()),
                )
            })
            .collect();
        y = self.chips(c, x, y + 9.0, col, &items);
        let Some(path) = path else {
            c.text(
                right,
                top,
                6.0,
                rgba(0x9a9ab0, 1.0),
                "These notes have no path yet: pick one.",
            );
            return;
        };
        c.text(x, y, 6.5, head, "Pace along it");
        let items: Vec<(String, Btn, bool)> = PACES
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                (
                    name.to_string(),
                    Btn::M(MAct::Pace(i)),
                    path.pace == pace_track(i),
                )
            })
            .collect();
        self.chips(c, x, y + 9.0, col, &items);
        c.text(
            right,
            top,
            6.5,
            head,
            "When (\"here\": over a beat from the playhead)",
        );
        let items = vec![
            (
                "Always".to_string(),
                Btn::M(MAct::Always),
                path.weight.is_empty(),
            ),
            (
                "Onto the path here".to_string(),
                Btn::M(MAct::Switch(true)),
                false,
            ),
            (
                "Back to the lane here".to_string(),
                Btn::M(MAct::Switch(false)),
                false,
            ),
            ("Show the path".to_string(), Btn::M(MAct::Show), path.show),
        ];
        let y = self.chips(c, right, top + 9.0, col, &items);
        c.text(right, y, 6.5, head, "These notes");
        let items = vec![
            ("Select them".to_string(), Btn::M(MAct::SelectNotes), false),
            ("Copy motion".to_string(), Btn::M(MAct::Copy), false),
            ("Remove path".to_string(), Btn::M(MAct::Clear), false),
        ];
        self.chips(c, right, y + 9.0, col, &items);
    }

    fn draw_before_tab(&mut self, c: &mut Canvas, g: usize, x: f32, y: f32, inner: f32) {
        let head = rgba(accent(), 1.0);
        let dim = rgba(0x9a9ab0, 1.0);
        let before = self.ed.before_of(g).cloned().unwrap_or_default();
        let col = (inner - 10.0) * 0.42;
        let right = x + col + 10.0;
        let rw = inner - col - 10.0;
        c.text(x, y, 6.5, head, "Ready-made");
        let items: Vec<(String, Btn, bool)> = BEFORES
            .iter()
            .enumerate()
            .map(|(i, (name, _))| {
                let on = if i == 0 {
                    before.is_empty()
                } else {
                    before == before_tracks(i)
                };
                (name.to_string(), Btn::M(MAct::Before(i)), on)
            })
            .collect();
        let ly = self.chips(c, x, y + 9.0, col, &items);
        let items = vec![
            ("Select them".to_string(), Btn::M(MAct::SelectNotes), false),
            ("Copy motion".to_string(), Btn::M(MAct::Copy), false),
        ];
        self.chips(c, x, ly + 2.0, col, &items);
        // The graph: one property's keys, beats before the hit left to right.
        c.text(right, y, 6.5, head, "Keys by beats before each note's hit");
        let items: Vec<(String, Btn, bool)> = GRAPH_PROPS
            .iter()
            .enumerate()
            .map(|(i, (p, name, _, _))| {
                let keyed = before.contains_key(p.name());
                (
                    if keyed {
                        format!("{name}*")
                    } else {
                        name.to_string()
                    },
                    Btn::M(MAct::Prop(i)),
                    self.m_prop == i,
                )
            })
            .collect();
        let gy = self.chips(c, right, y + 9.0, rw, &items) + 1.0;
        let bottom = self.layout.inspector[1] + self.layout.inspector[3] - 6.0;
        let gh = (bottom - gy - 8.0).max(24.0);
        let rect = [right, gy, rw, gh];
        self.m_graph = rect;
        self.draw_before_graph(c, rect, &before);
        c.text(
            right,
            gy + gh + 1.0,
            5.5,
            dim,
            &Canvas::fit(
                5.5,
                rw,
                "click: add a key   drag: move it   right click: remove it   Shift: off the 1/8 beat grid",
            ),
        );
    }

    fn graph_prop(&self) -> (NoteProp, f64, f64) {
        let (p, _, lo, hi) = GRAPH_PROPS[self.m_prop.min(GRAPH_PROPS.len() - 1)];
        (p, lo, hi)
    }

    /// Graph point of beats `b` before the hit and value `v`.
    fn graph_at(&self, b: f64, v: f64) -> (f32, f32) {
        let [gx, gy, gw, gh] = self.m_graph;
        let (_, lo, hi) = self.graph_prop();
        let fx = (GRAPH_FAR - b) / (GRAPH_FAR + GRAPH_AFTER);
        let fy = (v - lo) / (hi - lo);
        (
            gx + fx as f32 * gw,
            gy + gh - fy.clamp(0.0, 1.0) as f32 * gh,
        )
    }

    /// Beats before the hit and value at a graph point.
    fn graph_value(&self, x: f32, y: f32, free: bool) -> (f64, f64) {
        let [gx, gy, gw, gh] = self.m_graph;
        let (p, lo, hi) = self.graph_prop();
        let fx = ((x - gx) / gw.max(1.0)).clamp(0.0, 1.0) as f64;
        let mut b = GRAPH_FAR - fx * (GRAPH_FAR + GRAPH_AFTER);
        if !free {
            b = (b * 8.0).round() / 8.0;
        }
        let fy = ((gy + gh - y) / gh.max(1.0)).clamp(0.0, 1.0) as f64;
        let mut v = lo + fy * (hi - lo);
        v = if p == NoteProp::Hidden {
            v.round()
        } else {
            (v * 100.0).round() / 100.0
        };
        (b, v)
    }

    fn draw_before_graph(
        &mut self,
        c: &mut Canvas,
        [gx, gy, gw, gh]: [f32; 4],
        before: &std::collections::BTreeMap<String, crate::tko::track::Track>,
    ) {
        c.round_rect(
            [gx, gy],
            [gw, gh],
            2.0,
            rgba(0x101018, 1.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        let dim = rgba(0x9a9ab0, 0.8);
        // Beat lines, the hit brighter.
        let mut b = GRAPH_FAR;
        while b >= -GRAPH_AFTER {
            let (lx, _) = self.graph_at(b, 0.0);
            let hit = b == 0.0;
            c.rect(
                lx - 0.5,
                gy + 1.0,
                1.0,
                gh - 2.0,
                rgba(if hit { accent() } else { 0x2a2a3a }, 1.0),
            );
            let label = if hit {
                "hit".to_string()
            } else {
                format!("{b}")
            };
            if b > -GRAPH_AFTER {
                c.text(lx + 2.0, gy + 2.0, 5.0, dim, &label);
            }
            b -= 1.0;
        }
        let (p, lo, hi) = self.graph_prop();
        // The value notes have without keys.
        let (_, ny) = self.graph_at(0.0, p.default_value());
        c.rect(gx + 1.0, ny - 0.5, gw - 2.0, 1.0, rgba(0x3a3a4e, 1.0));
        c.text(gx + 2.0, gy + gh - 7.0, 5.0, dim, &format!("{lo}"));
        c.text_right(gx + gw - 2.0, gy + 9.0, 5.0, dim, &format!("{hi}"));
        let Some(tr) = before.get(p.name()) else {
            return;
        };
        let steps = (gw / 1.5).max(8.0) as usize;
        for s in 0..=steps {
            let b = GRAPH_FAR - (GRAPH_FAR + GRAPH_AFTER) * s as f64 / steps as f64;
            let (px, py) = self.graph_at(b, tr.value(b, p.default_value()));
            c.rect(px - 0.7, py - 0.7, 1.4, 1.4, rgba(accent(), 0.95));
        }
        let live = match self.drag {
            Drag::BeforeKey { index, .. } => Some(index),
            _ => None,
        };
        let hover = self.graph_key_near(self.mouse.0, self.mouse.1);
        for (i, k) in tr.keys.iter().enumerate() {
            let (kx, ky) = self.graph_at(k.t, k.v);
            if !(gx - 1.0..=gx + gw + 1.0).contains(&kx) {
                continue;
            }
            let lit = live == Some(i) || hover == Some(i);
            c.round_rect(
                [kx - 2.5, ky - 2.5],
                [5.0, 5.0],
                1.0,
                rgba(0xffffff, if lit { 1.0 } else { 0.8 }),
                rgba(0x000000, 0.8),
                1.0,
            );
        }
    }

    /// The key of the graph's track near a point.
    fn graph_key_near(&self, x: f32, y: f32) -> Option<usize> {
        let g = self.motion_group()?;
        if self.m_tab != 1 || !inside(self.m_graph, x, y) {
            return None;
        }
        let (p, _, _) = self.graph_prop();
        let tr = self.ed.before_of(g)?.get(p.name())?;
        tr.keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let (kx, ky) = self.graph_at(k.t, k.v);
                (i, (kx - x).powi(2) + (ky - y).powi(2))
            })
            .filter(|(_, d)| *d <= GRAB * GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// A press on the graph: takes the key there, or adds one and takes it.
    pub(super) fn before_down(&mut self, x: f32, y: f32, keys: Keys) -> bool {
        if !self.motion_mode() || self.m_tab != 1 || !inside(self.m_graph, x, y) {
            return false;
        }
        let Some(g) = self.motion_group() else {
            return false;
        };
        let index = match self.graph_key_near(x, y) {
            Some(i) => i,
            None => {
                let (p, _, _) = self.graph_prop();
                let (b, v) = self.graph_value(x, y, keys.shift);
                let ease = if p == NoteProp::Hidden {
                    Ease::Step
                } else {
                    Ease::SineInOut
                };
                let mut at = 0;
                self.ed.edit_before(g, p, true, |t| {
                    at = t.set(Key { t: b, v, ease });
                });
                self.mark_stale();
                at
            }
        };
        self.drag = Drag::BeforeKey { group: g, index };
        true
    }

    pub(super) fn before_drag(&mut self, x: f32, y: f32) {
        let Drag::BeforeKey { group, index } = self.drag else {
            return;
        };
        let (p, _, _) = self.graph_prop();
        let (b, v) = self.graph_value(x, y, self.held.shift);
        let mut at = index;
        self.ed.edit_before(group, p, true, |t| {
            if let Some(k) = t.remove(index) {
                at = t.set(Key {
                    t: b,
                    v,
                    ease: k.ease,
                });
            }
        });
        self.drag = Drag::BeforeKey { group, index: at };
        self.mark_stale();
    }

    /// A right click on the graph: takes the key there away.
    pub(super) fn before_right(&mut self, x: f32, y: f32) -> bool {
        if !self.motion_mode() || self.m_tab != 1 || !inside(self.m_graph, x, y) {
            return false;
        }
        if let (Some(g), Some(i)) = (self.motion_group(), self.graph_key_near(x, y)) {
            let (p, _, _) = self.graph_prop();
            self.ed.edit_before(g, p, false, |t| {
                t.remove(i);
            });
            self.mark_stale();
        }
        true
    }

    // ---- the path in the preview ----

    /// Preview units per playfield unit.
    pub(super) fn path_scale(&self) -> f32 {
        self.layout.preview[2] / FRAME_VW
    }

    /// Where the drum is drawn in the preview, by the playfield's keys at the playhead.
    pub(super) fn drum_in_preview(&self) -> (f32, f32) {
        let [fx, fy, _, _] = self.layout.preview;
        let k = self.path_scale();
        let t = self.ed.time;
        let at = |p| {
            self.ed
                .value_of(crate::editor::keyframes::Subject::Field(p), t) as f32
        };
        (
            fx + (HIT_X + at(FieldProp::X)) * k,
            fy + (LANE_Y + at(FieldProp::Y)) * k,
        )
    }

    /// The points of the path being shaped, in the preview.
    pub(super) fn path_handles(&self) -> Vec<(f32, f32)> {
        let Some(p) = self.motion_group().and_then(|g| self.ed.path_of(g)) else {
            return Vec::new();
        };
        let (dx, dy) = self.drum_in_preview();
        let k = self.path_scale();
        p.points
            .iter()
            .map(|q| (dx + q[0] as f32 * k, dy + q[1] as f32 * k))
            .collect()
    }

    fn handle_near(&self, x: f32, y: f32) -> Option<usize> {
        self.path_handles()
            .iter()
            .enumerate()
            .map(|(i, (hx, hy))| (i, (hx - x).hypot(hy - y)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The path being shaped over the preview: a dotted curve and a handle on each point.
    pub(super) fn draw_path_gizmo(&mut self, c: &mut Canvas) {
        if !self.motion_mode() || self.game.is_none() {
            return;
        }
        let Some(path) = self
            .motion_group()
            .and_then(|g| self.ed.path_of(g))
            .cloned()
        else {
            return;
        };
        let (dx, dy) = self.drum_in_preview();
        let k = self.path_scale();
        if let Some(table) = PathTable::new(&path) {
            let step = 4.0 / k.max(0.05);
            let mut d = 0.0;
            while d < table.len() {
                let [px, py] = table.at(d);
                c.rect(
                    dx + px * k - 0.6,
                    dy + py * k - 0.6,
                    1.2,
                    1.2,
                    rgba(0xffffff, 0.6),
                );
                d += step;
            }
        }
        let live = match self.drag {
            Drag::PathPoint { point, .. } => Some(point),
            _ => None,
        };
        let (mx, my) = self.mouse;
        let hover = self.handle_near(mx, my);
        for (i, (hx, hy)) in self.path_handles().into_iter().enumerate() {
            let lit = live == Some(i) || hover == Some(i);
            let colour = if i == 0 { 0xffffff } else { accent() };
            c.round_rect(
                [hx - 3.0, hy - 3.0],
                [6.0, 6.0],
                1.0,
                rgba(colour, if lit { 1.0 } else { 0.75 }),
                rgba(0x000000, 0.8),
                1.0,
            );
        }
        c.ring(dx, dy, 5.0, rgba(accent(), 0.9), 1.0);
        let [fx, fy, fw, fh] = self.layout.preview;
        if c.hovered(fx, fy, fw, fh) {
            c.text(
                fx + 4.0,
                fy + fh - 9.0,
                5.5,
                rgba(0xffffff, 0.8),
                &Canvas::fit(
                    5.5,
                    fw - 8.0,
                    "drag a point: move it   double click: add one   right click: take it away",
                ),
            );
        }
    }

    /// A press in the preview while shaping a path. False when it isn't for the path.
    pub(super) fn path_down(&mut self, x: f32, y: f32) -> bool {
        if !self.motion_mode() || !inside(self.layout.preview, x, y) {
            return false;
        }
        let Some(g) = self
            .motion_group()
            .filter(|&g| self.ed.path_of(g).is_some())
        else {
            return false;
        };
        if let Some(point) = self.handle_near(x, y) {
            self.drag = Drag::PathPoint { group: g, point };
            self.m_click = None;
            return true;
        }
        let now = Instant::now();
        let double = self.m_click.is_some_and(|(at, (px, py))| {
            now.duration_since(at).as_millis() <= DOUBLE_MS && (px - x).hypot(py - y) <= GRAB
        });
        if double {
            self.m_click = None;
            self.add_path_point(g, x, y);
        } else {
            self.m_click = Some((now, (x, y)));
        }
        true
    }

    /// The playfield point (from the drum) under preview point `x`, `y`.
    fn path_point_at(&self, x: f32, y: f32) -> [f64; 2] {
        let (dx, dy) = self.drum_in_preview();
        let k = self.path_scale().max(1e-3);
        [((x - dx) / k).round() as f64, ((y - dy) / k).round() as f64]
    }

    /// A point where the double click was, between the two the curve passes closest to there.
    fn add_path_point(&mut self, g: usize, x: f32, y: f32) {
        let mut stops = self.path_handles();
        if stops.len() >= crate::tko::path::MAX_POINTS {
            self.status = "a path has at most 32 points".into();
            return;
        }
        stops.push(self.drum_in_preview());
        let seg = |(ax, ay): (f32, f32), (bx, by): (f32, f32)| {
            let (vx, vy) = (bx - ax, by - ay);
            let l = (vx * vx + vy * vy).max(1e-6);
            let f = (((x - ax) * vx + (y - ay) * vy) / l).clamp(0.0, 1.0);
            (ax + vx * f - x).hypot(ay + vy * f - y)
        };
        let at = stops
            .windows(2)
            .enumerate()
            .map(|(i, w)| (i, seg(w[0], w[1])))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(i, _)| i + 1);
        let p = self.path_point_at(x, y);
        if self
            .ed
            .edit_path(g, false, |path| path.points.insert(at, p))
        {
            self.mark_stale();
        }
    }

    pub(super) fn path_drag(&mut self, x: f32, y: f32) {
        let Drag::PathPoint { group, point } = self.drag else {
            return;
        };
        let p = self.path_point_at(x, y);
        if self.ed.edit_path(group, true, |path| {
            if let Some(q) = path.points.get_mut(point) {
                *q = p;
            }
        }) {
            self.mark_stale();
        }
    }

    /// A right click in the preview while shaping a path: the point under it goes (a path keeps
    /// at least one). False when it isn't for the path.
    pub(super) fn path_right(&mut self, x: f32, y: f32) -> bool {
        if !self.motion_mode() || !inside(self.layout.preview, x, y) {
            return false;
        }
        let (Some(g), Some(i)) = (self.motion_group(), self.handle_near(x, y)) else {
            return false;
        };
        if self.path_handles().len() <= 1 {
            self.status =
                "a path needs a point to come in from: Remove path takes it all away".into();
            return true;
        }
        if self.ed.edit_path(g, false, |p| {
            p.points.remove(i);
        }) {
            self.mark_stale();
        }
        true
    }
}
