//! The strip of timing lines between the ruler and the notes. Red lines (BPM) have its top row
//! and green lines (SV) its bottom one, so a BPM change and an SV change at one moment never
//! cover each other, and a label only shows where it has room.
//!
//! Lines are worked here the way notes are: a click picks one (Shift+click adds or takes it
//! away), a drag over empty strip boxes the lines in the rows it covers, and a picked line
//! dragged takes the whole selection along, on the grid, or to the millisecond with Shift held.
//! A red line dragged snaps to the grid of the lines that stay (its own moves with it), and with
//! "Notes follow" on its notes go with it.

use std::collections::HashSet;

use super::{Drag, GREEN_LINE, Keys, RED_LINE, RULER, View};
use crate::editor::chart::TimingPoint;
use crate::editor::timing;
use crate::hit::Canvas;
use crate::render::{accent, rgba};

/// How tall the strip is: two rows.
pub(super) const STRIP: f32 = 18.0;
const ROW_H: f32 = STRIP / 2.0;
const LABEL: f32 = 5.0;

impl View {
    /// The tops of the strip's red row and green row.
    fn strip_rows(&self) -> (f32, f32) {
        let y = self.layout.timeline[1] + RULER;
        (y, y + ROW_H)
    }

    /// The rows a box from `y0` to `y1` covers: (red, green).
    fn strip_box_rows(&self, y0: f32, y1: f32) -> (bool, bool) {
        let mid = self.strip_rows().1;
        (y0.min(y1) < mid, y0.max(y1) >= mid)
    }

    /// The line under (`x`, `y`) in the strip: one of the row's colour within 4 px.
    pub(super) fn strip_line_at(&self, x: f32, y: f32) -> Option<u64> {
        let red = y < self.strip_rows().1;
        let t = self.t_at(x);
        let slack = 4.0 * self.ms_per_px;
        let pts = &self.ed.chart.timing;
        let from = pts.partition_point(|p| p.time < t - slack);
        pts[from..]
            .iter()
            .take_while(|p| p.time <= t + slack)
            .filter(|p| p.is_red() == red)
            .min_by(|a, b| {
                (a.time - t)
                    .abs()
                    .total_cmp(&(b.time - t).abs())
                    .then(a.id.cmp(&b.id))
            })
            .map(|p| p.id)
    }

    pub(super) fn draw_strip(&mut self, c: &mut Canvas) {
        let [x, _, w, _] = self.layout.timeline;
        let (red_y, green_y) = self.strip_rows();
        c.rect(x + 1.0, red_y, w - 2.0, ROW_H, rgba(0x1b1418, 0.95));
        c.rect(x + 1.0, green_y, w - 2.0, ROW_H, rgba(0x131a15, 0.95));
        c.rect(x + 1.0, green_y, w - 2.0, 1.0, rgba(0x000000, 0.45));
        let (from, to) = (self.view_start, self.view_end());
        let picked: HashSet<u64> = self.selected_lines().into_iter().collect();
        let boxed = match self.drag {
            Drag::StripBox {
                from,
                to,
                y0,
                moved: true,
                ..
            } => {
                let (red, green) = self.strip_box_rows(y0, self.mouse.1);
                Some((from.min(to), from.max(to), red, green))
            }
            _ => None,
        };
        let hovered = match self.drag {
            Drag::None if super::inside([x, red_y, w, STRIP], self.mouse.0, self.mouse.1) => {
                self.strip_line_at(self.mouse.0, self.mouse.1)
            }
            _ => None,
        };
        let labels = self.ms_per_px < 12.0;
        // Where the last label of each row (green, red) ends: the next one waits for room.
        let mut label_end = [f32::MIN; 2];
        let pts = &self.ed.chart.timing;
        let first = pts.partition_point(|p| p.time < from - 2000.0);
        for p in pts[first..].iter().take_while(|p| p.time <= to + 2000.0) {
            let lx = self.x_of(p.time);
            let red = p.is_red();
            let row_y = if red { red_y } else { green_y };
            let in_box = boxed
                .is_some_and(|(a, b, r, g)| p.time >= a && p.time <= b && if red { r } else { g });
            let selected = picked.contains(&p.id) || in_box;
            let hot = hovered == Some(p.id);
            let col = if red { RED_LINE } else { GREEN_LINE };
            let (lw, shown) = if selected {
                (2.0, 0xffffff)
            } else if hot {
                (2.0, 0xffe9a8)
            } else {
                (1.0, col)
            };
            c.rect(
                lx - lw / 2.0,
                row_y + 1.0,
                lw,
                ROW_H - 1.0,
                rgba(shown, 1.0),
            );
            if !labels {
                continue;
            }
            let label = match p.bpm() {
                Some(b) => format!("{} BPM", trim(b)),
                None => format!("x{}", trim(p.sv())),
            };
            let k = red as usize;
            let tw = Canvas::text_width(LABEL, &label);
            let lx = lx + 3.0;
            if lx > x + 1.0 && lx >= label_end[k] && lx + tw < x + w - 2.0 {
                let a = if selected || hot { 1.0 } else { 0.85 };
                c.text(lx, row_y + 2.0, LABEL, rgba(shown, a), &label);
                label_end[k] = lx + tw + 4.0;
            }
        }
        if let Some((a, b, red, green)) = boxed {
            let (xa, xb) = (self.x_of(a).max(x), self.x_of(b).min(x + w));
            let top = if red { red_y } else { green_y };
            let bottom = if green { green_y + ROW_H } else { green_y };
            c.rect(xa, top, xb - xa, bottom - top, rgba(accent(), 0.2));
            c.round_rect(
                [xa, top],
                [xb - xa, bottom - top],
                0.0,
                rgba(0, 0.0),
                rgba(accent(), 0.85),
                1.0,
            );
        }
        self.tips.push((
            [x, red_y, w, STRIP],
            "Timing lines: red (BPM) on top, green (SV) below\nClick picks a line, Shift+click adds one, a drag over empty strip boxes them. Drag picked lines to move them along the grid; hold Shift to move them freely, to the millisecond. A click on empty strip moves the playhead."
                .to_string(),
        ));
    }

    /// A press in the strip at (`x`, `y`).
    pub(super) fn strip_down(&mut self, x: f32, y: f32, keys: Keys) {
        let t = self.t_at(x);
        let Some(id) = self.strip_line_at(x, y) else {
            self.drag = Drag::StripBox {
                from: t,
                to: t,
                y0: y,
                down_x: x,
                moved: false,
                add: keys.shift,
            };
            return;
        };
        self.ed.select_none();
        if keys.shift {
            self.toggle_line(id);
        } else if !self.selected_lines().contains(&id) {
            self.select_line(Some(id));
        } else {
            self.sel_line = Some(id);
        }
        let ids = self.selected_lines();
        if !ids.contains(&id) {
            return;
        }
        let Some(at) = self.ed.chart.timing_point(id).map(|p| p.time) else {
            return;
        };
        let wanted: HashSet<u64> = ids.into_iter().collect();
        self.line_origin = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| wanted.contains(&p.id))
            .map(|p| (p.id, p.time, 0.0))
            .collect();
        self.drag = Drag::StripLines {
            anchor: id,
            grab: t - at,
            down_x: x,
            moved: false,
        };
    }

    /// The picked lines dragged so the anchor is under `x`.
    pub(super) fn strip_drag(&mut self, x: f32, keys: Keys) {
        let Drag::StripLines {
            anchor,
            grab,
            down_x,
            moved,
        } = self.drag
        else {
            return;
        };
        // A click's worth of wobble doesn't move a line off its place.
        if !moved && (x - down_x).abs() < 3.0 {
            return;
        }
        self.drag = Drag::StripLines {
            anchor,
            grab,
            down_x,
            moved: true,
        };
        let Some(&(_, t0, _)) = self.line_origin.iter().find(|o| o.0 == anchor) else {
            return;
        };
        let want = self.t_at(x) - grab;
        let to = if keys.shift {
            want.round()
        } else {
            self.strip_snap(want)
        };
        let lowest = self
            .line_origin
            .iter()
            .map(|o| o.1)
            .fold(f64::INFINITY, f64::min);
        let dt = (to - t0).max(-lowest);
        let origin = self.line_origin.clone();
        let ids: Vec<u64> = origin.iter().map(|o| o.0).collect();
        let moved = self.ed.edit_timings(
            &ids,
            |p| {
                if let Some(o) = origin.iter().find(|o| o.0 == p.id) {
                    p.time = (o.1 + dt).round();
                }
            },
            true,
        );
        match moved {
            Ok(()) => self.mark_stale(),
            Err(e) => self.status = e,
        }
    }

    /// `t` on the grid of the red lines that aren't being dragged (a red line can't snap to the
    /// grid it carries along); to the millisecond when none stays.
    fn strip_snap(&self, t: f64) -> f64 {
        let moving: HashSet<u64> = self
            .line_origin
            .iter()
            .map(|o| o.0)
            .filter(|id| {
                self.ed
                    .chart
                    .timing_point(*id)
                    .is_some_and(TimingPoint::is_red)
            })
            .collect();
        if moving.is_empty() {
            return self.ed.snapped(t);
        }
        let others: Vec<TimingPoint> = self
            .ed
            .chart
            .timing
            .iter()
            .filter(|p| !moving.contains(&p.id))
            .cloned()
            .collect();
        if !others.iter().any(TimingPoint::is_red) {
            return t.round();
        }
        timing::snapped_ms(&others, t, self.ed.snap) as f64
    }

    /// The end of a drag of picked lines.
    pub(super) fn strip_lines_up(&mut self, moved: bool) {
        self.line_origin.clear();
        self.ed.end_drag();
        if moved {
            self.mark_stale();
            let n = self.selected_lines().len();
            if n > 1 {
                self.status = format!("moved {n} lines");
            }
        }
    }

    /// The end of a press on empty strip: a click moves the playhead, a box picks the lines in
    /// the rows it covers (Shift: as well as those picked).
    pub(super) fn strip_box_up(
        &mut self,
        from: f64,
        to: f64,
        y0: f32,
        y1: f32,
        moved: bool,
        add: bool,
    ) {
        if !moved {
            self.set_time(from);
            return;
        }
        let (red, green) = self.strip_box_rows(y0, y1);
        let (a, b) = (from.min(to), from.max(to));
        let mut got = if add {
            self.selected_lines()
        } else {
            Vec::new()
        };
        let mut have: HashSet<u64> = got.iter().copied().collect();
        let pts = &self.ed.chart.timing;
        let first = pts.partition_point(|p| p.time < a);
        for p in pts[first..].iter().take_while(|p| p.time <= b) {
            if (if p.is_red() { red } else { green }) && have.insert(p.id) {
                got.push(p.id);
            }
        }
        let n = got.len();
        self.sel_line = got.last().copied();
        self.sel_extra = got.into_iter().collect();
        if n > 0 {
            self.ed.select_none();
        }
        self.status = match n {
            0 => "no lines in the box".into(),
            1 => "1 line picked".into(),
            _ => format!("{n} lines picked: drag one to move them all"),
        };
    }
}

/// `v` with at most three decimals and no trailing zeros (`180`, `1.25`).
fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}
