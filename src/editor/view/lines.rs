//! Working with timing lines as a group: selecting several in the velocity graph, dragging them
//! together, and the tools that act on a selection (set, multiply, ramp, copy, delete).

use super::chrome::sample_name;
use super::{Btn, Drag, Field, Keys, SAct, View};
use crate::editor::chart::TimingPoint;
use crate::editor::timing;
use crate::render::{Canvas, accent, rgba};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TAct {
    /// Move the selected line to the playhead.
    Here,
    /// Copy the selected lines to the playhead.
    CloneHere,
    Copy,
    Paste,
    Delete,
}

impl View {
    /// Every selected timing line, the focused one first.
    pub(super) fn selected_lines(&self) -> Vec<u64> {
        let Some(main) = self.sel_line else {
            return Vec::new();
        };
        let mut out = vec![main];
        out.extend(self.sel_extra.iter().copied().filter(|id| *id != main));
        if out.len() > 1 {
            // A set: one chart lookup per selected line is slow with thousands of them.
            let have: std::collections::HashSet<u64> =
                self.ed.chart.timing.iter().map(|p| p.id).collect();
            out.retain(|id| have.contains(id));
        } else {
            out.retain(|id| self.ed.chart.timing_point(*id).is_some());
        }
        out
    }

    /// Makes `id` the only selected line (or none).
    pub(super) fn select_line(&mut self, id: Option<u64>) {
        self.sel_line = id;
        self.sel_extra.clear();
    }

    pub(super) fn toggle_line(&mut self, id: u64) {
        let mut all = self.selected_lines();
        if let Some(i) = all.iter().position(|x| *x == id) {
            all.remove(i);
        } else {
            all.push(id);
        }
        self.sel_line = all.last().copied();
        self.sel_extra = all.into_iter().collect();
    }

    /// Selects the lines a box over the graph covers.
    fn select_lines_in(&mut self, a: (f32, f32), b: (f32, f32), add: bool) {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        let range = self.graph_range();
        let mut got: Vec<u64> = if add {
            self.selected_lines()
        } else {
            Vec::new()
        };
        for p in &self.ed.chart.timing {
            let px = self.x_of(p.time);
            let hit = if p.is_red() && !self.svt.volume_view {
                px >= x0 && px <= x1
            } else {
                let py = self.val_y(self.graph_val(p), range);
                px >= x0 && px <= x1 && py >= y0 && py <= y1
            };
            if hit && !got.contains(&p.id) {
                got.push(p.id);
            }
        }
        self.sel_line = got.last().copied();
        self.sel_extra = got.into_iter().collect();
    }

    /// A press in the velocity graph (Timing mode): on a dot it selects and starts a drag of the
    /// selection, on empty space it starts a box (a plain click there adds a line).
    pub(super) fn line_graph_down(&mut self, x: f32, y: f32, keys: Keys) {
        if self.handle_down(x, y) {
            return;
        }
        if self.svt.brush {
            return self.brush_down(x, y, keys);
        }
        if let Some(id) = self.graph_dot_near(x, y) {
            if keys.shift {
                self.toggle_line(id);
            } else if !self.selected_lines().contains(&id) {
                self.select_line(Some(id));
            } else {
                self.sel_line = Some(id);
            }
            if !self.selected_lines().contains(&id) {
                return;
            }
            self.line_origin = self
                .selected_lines()
                .iter()
                .filter_map(|l| self.ed.chart.timing_point(*l))
                .map(|p| (p.id, p.time, self.graph_val(p)))
                .collect();
            self.g_frozen = Some(self.graph_range());
            self.g_down = (x, y);
            self.drag = Drag::Lines { anchor: id };
            return;
        }
        // A curve under the pointer is picked: its ends get handles and the SV tool shows it.
        if let Some(i) = self.curve_near(x, y) {
            self.press_svt(SAct::Pick(i));
            return;
        }
        self.drag = Drag::GBox {
            from: (x, y),
            to: (x, y),
        };
    }

    pub(super) fn line_drag(&mut self, anchor: u64, x: f32, y: f32, keys: Keys) {
        let Some(&(_, t0, v0)) = self.line_origin.iter().find(|o| o.0 == anchor) else {
            return;
        };
        let want_t = self.ed.snapped(self.t_at(x));
        let range = self.graph_range();
        let want = self.y_val(y, range).max(0.01);
        let dt = if keys.shift { 0.0 } else { want_t - t0 };
        let volume = self.svt.volume_view;
        // Speeds scale together (Alt: add the same amount instead), volumes shift together.
        let k = want / v0.max(1e-9);
        let add = want - v0;
        let by_add = keys.alt || volume;
        let origin = self.line_origin.clone();
        let ids: Vec<u64> = origin.iter().map(|o| o.0).collect();
        let moved = self.ed.edit_timings(
            &ids,
            |p| {
                if let Some(&(_, ot, ov)) = origin.iter().find(|o| o.0 == p.id) {
                    if !(volume && p.is_red()) {
                        p.time = (ot + dt).max(0.0).round();
                    }
                    if volume {
                        p.volume = (ov + add).round().clamp(0.0, 100.0) as i32;
                    } else if !p.is_red() {
                        let v = if by_add { ov + add } else { ov * k };
                        let _ = p.set_sv(((v * 100.0).round() / 100.0).max(0.01));
                    }
                }
            },
            true,
        );
        match moved {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    /// The end of a press in the graph that was a box or a click.
    pub(super) fn line_box_up(&mut self, from: (f32, f32), x: f32, y: f32, keys: Keys) {
        if (x - from.0).abs() < 4.0 && (y - from.1).abs() < 4.0 {
            // A single click on empty space lets go of the selection; a second one close by and
            // soon after puts a new green line right there.
            let now = std::time::Instant::now();
            let second = self.g_click.take().is_some_and(|(at, (px, py))| {
                now.duration_since(at).as_millis() < 450
                    && (px - x).abs() < 6.0
                    && (py - y).abs() < 6.0
            });
            if !second {
                self.g_click = Some((now, (x, y)));
                self.select_lines_in(from, (x, y), keys.shift);
                return;
            }
            let time = self.ed.snapped(self.t_at(x));
            let range = self.graph_range();
            let value = self.y_val(y, range);
            let sv = if self.svt.volume_view {
                timing::sv_at(&self.ed.chart.timing, time)
            } else {
                value.max(0.01)
            };
            match self.ed.add_green(time, sv) {
                Ok(id) => {
                    if self.svt.volume_view {
                        let v = value.round() as i32;
                        let _ = self.ed.edit_timing(id, |p| p.volume = v, false);
                    }
                    self.select_line(Some(id));
                    self.mark_stale();
                }
                Err(e) => self.status = e,
            }
        } else {
            self.select_lines_in(from, (x, y), keys.shift);
        }
    }

    pub(super) fn draw_line_selection(&mut self, c: &mut Canvas) {
        if let Drag::GBox { from, to } = self.drag {
            let (x0, y0) = (from.0.min(to.0), from.1.min(to.1));
            let (w, h) = ((from.0 - to.0).abs(), (from.1 - to.1).abs());
            c.rect(x0, y0, w, h, rgba(accent(), 0.16));
            c.round_rect(
                [x0, y0],
                [w, h],
                0.0,
                rgba(0, 0.0),
                rgba(accent(), 0.85),
                1.0,
            );
        }
    }

    // ---- the tools ----

    pub(super) fn press_line_action(&mut self, a: TAct) {
        let ids = self.selected_lines();
        let result: Result<String, String> = match a {
            TAct::Here => match self.sel_line {
                Some(id) => {
                    let at = self.ed.time;
                    self.ed
                        .edit_timing(id, |p| p.time = at, false)
                        .map(|()| format!("moved to {}", super::fmt_time(at)))
                }
                None => Err("no line selected".into()),
            },
            TAct::CloneHere => {
                let n = self.ed.copy_lines(&ids);
                let at = self.ed.time;
                self.ed
                    .paste_lines(at)
                    .map(|m| format!("cloned {m} of {n} lines"))
            }
            TAct::Copy => Ok(format!("copied {} lines", self.ed.copy_lines(&ids))),
            TAct::Paste => {
                let at = self.ed.time;
                self.ed.paste_lines(at).map(|n| format!("pasted {n} lines"))
            }
            TAct::Delete => self.ed.delete_timings(&ids).map(|n| {
                self.select_line(None);
                format!("deleted {n} lines")
            }),
        };
        match result {
            Ok(said) => {
                self.status = said;
                self.mark_stale();
            }
            Err(e) => self.status = e,
        }
    }

    /// The panel's content when several lines are selected.
    pub(super) fn draw_line_tools(&mut self, c: &mut Canvas, x: f32, mut y: f32, w: f32) {
        let ids = self.selected_lines();
        let (reds, greens) = ids
            .iter()
            .filter_map(|id| self.ed.chart.timing_point(*id))
            .fold(
                (0, 0),
                |(r, g), p| if p.is_red() { (r + 1, g) } else { (r, g + 1) },
            );
        c.text(
            x,
            y,
            7.5,
            rgba(0xffffff, 1.0),
            &Canvas::fit(
                7.5,
                w,
                &format!("{} lines selected  ({greens} green, {reds} red)", ids.len()),
            ),
        );
        y += 14.0;
        // One field for all of them: the value they share, else "mixed". Typing or - / + changes
        // every selected line (the speed only of the green ones).
        let lines: Vec<&TimingPoint> = ids
            .iter()
            .filter_map(|id| self.ed.chart.timing_point(*id))
            .collect();
        let shared = |label: &str,
                      of: &dyn Fn(&TimingPoint) -> String,
                      only: &dyn Fn(&TimingPoint) -> bool| {
            let mut vals = lines.iter().filter(|p| only(p)).map(|p| of(p));
            let first = vals.next().unwrap_or_default();
            if vals.all(|v| v == first) {
                format!("{label} {first}")
            } else {
                format!("{label} mixed")
            }
        };
        let mut fields: Vec<(Field, String)> = Vec::new();
        if greens > 0 {
            fields.push((
                Field::Sv,
                shared("SV", &|p| format!("x{:.2}", p.sv()), &|p| !p.is_red()),
            ));
        }
        fields.push((
            Field::Volume,
            shared("Volume", &|p| p.volume.to_string(), &|_| true),
        ));
        fields.push((
            Field::Sample,
            shared(
                "Sample",
                &|p| sample_name(p.sample_set).to_string(),
                &|_| true,
            ),
        ));
        fields.push((
            Field::Index,
            shared("Index", &|p| p.sample_index.to_string(), &|_| true),
        ));
        let all_kiai = lines.iter().all(|p| p.kiai());
        y = self.line_fields(c, x, y, w, fields);
        self.row(c, x, y, &[("Kiai".to_string(), Btn::Kiai, all_kiai)]);
        y += 20.0;
        self.row(
            c,
            x,
            y,
            &[
                ("Copy".to_string(), Btn::T(TAct::Copy), false),
                ("Paste at playhead".to_string(), Btn::T(TAct::Paste), false),
                ("Delete".to_string(), Btn::T(TAct::Delete), false),
                (
                    "Ramp, multiply, curves...".to_string(),
                    Btn::S(SAct::Tab(1)),
                    false,
                ),
            ],
        );
        self.tips.push((
            [x, y, w, 16.0],
            "Ramps between the selected lines, multiplying them and curves of every shape are the SV tool: pick a preset there, or drag a curve out with the brush.".to_string(),
        ));
    }
}
