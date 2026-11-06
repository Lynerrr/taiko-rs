//! The Motion page (F4 Effects, .tko maps): the notes picked on the timeline get an approach
//! path, picked from ready-made ones and shaped by hand in the preview, a pace along it, and
//! moments it takes them on and off the lane.

use super::{Btn, Drag, Mode, View, inside};
use crate::editor::motion::{PACES, PATHS, pace_track};
use crate::game::{HIT_X, LANE_Y};
use crate::render::{Canvas, accent, rgba};
use crate::tko::gimmick::FieldProp;
use crate::tko::path::PathTable;
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

/// A button on the Motion page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MAct {
    /// The notes picked get (or keep) their own group, with a path if it has none.
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
            (MAct::UseSelection, _) => match self.ed.motion_group() {
                Ok(g) => {
                    self.m_group = Some(g);
                    if self.ed.path_of(g).is_none() {
                        self.ed.set_path_preset(g, 0);
                    }
                    self.status =
                        "the picked notes fly in along this path: shape it in the preview".into();
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
            (_, None) => {
                self.status = "pick notes and give them a path first".into();
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
        let head = rgba(accent(), 1.0);
        let small = 6.0;
        c.text(
            x,
            y,
            small,
            dim,
            &Canvas::fit(
                small,
                inner,
                "Pick notes on the timeline and give them a path: they fly in along it and land on the drum on time.",
            ),
        );
        y += 10.0;
        // What to shape: the picked notes, or a group that already has a path.
        let picked = self.ed.selection.len();
        let mut items = vec![(
            if picked == 0 {
                "Give the picked notes a path".to_string()
            } else {
                format!("Give the {picked} picked notes a path")
            },
            Btn::M(MAct::UseSelection),
            false,
        )];
        if self.m_copied.is_some() {
            items.push(("Paste motion".to_string(), Btn::M(MAct::Paste), false));
        }
        let current = self.motion_group();
        if let Some(g) = self.ed.gimmick() {
            items.extend(
                g.groups
                    .iter()
                    .enumerate()
                    .filter(|(_, gr)| gr.path.is_some())
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
        let Some(g) = current else { return };
        let Some(path) = self.ed.path_of(g).cloned() else {
            c.text(x, y, small, dim, "This group has no path yet.");
            return;
        };
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
                    path.points == pts.to_vec(),
                )
            })
            .collect();
        y = self.chips(c, x, y + 9.0, col, &items);
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
