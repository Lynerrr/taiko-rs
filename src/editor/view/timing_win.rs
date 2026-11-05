//! The F6 timing window: every timing line in a table, the selected line's fields or the SV tool
//! on its right. It sits over the preview and the panels and leaves the timeline under it
//! working, so a stretch of notes can be selected while the window is open.

use winit::keyboard::KeyCode;

use super::chrome::{DIM, FAINT, sample_name};
use super::cmd::Cmd;
use super::lines::TAct;
use super::{Btn, Drag, Keys, RED_LINE, View, fmt_time, inside};
use crate::audio::Engine;
use crate::editor::timing;
use crate::render::{Canvas, accent, rgba};

const ROW: f32 = 13.0;
/// Where the columns start, from the table's left edge.
const COLS: [f32; 8] = [0.0, 62.0, 100.0, 168.0, 204.0, 266.0, 298.0, 330.0];
/// The table's width, without its scroll bar.
const TABLE_W: f32 = 362.0;

impl View {
    /// The window is showing (it needs the timeline page, so the node canvas closes it).
    pub(super) fn windowed(&self) -> bool {
        self.tw && !self.node_mode()
    }

    /// The lines the table lists, in time order.
    pub(super) fn tw_ids(&self) -> Vec<u64> {
        self.ed
            .chart
            .timing
            .iter()
            .filter(|p| match self.tw_filter {
                1 => p.is_red(),
                2 => !p.is_red(),
                _ => true,
            })
            .map(|p| p.id)
            .collect()
    }

    pub(super) fn tw_rows(&self) -> usize {
        (((self.layout.window[3] - 50.0) / ROW) as usize).max(1)
    }

    /// A click on a row: select it and go there; Shift takes the rows up to it, Ctrl toggles it.
    pub(super) fn pick_row(&mut self, id: u64, keys: Keys, audio: Option<&Engine>) {
        if keys.shift
            && let Some(main) = self.sel_line
        {
            let ids = self.tw_ids();
            let at = |t: u64| ids.iter().position(|x| *x == t);
            if let (Some(a), Some(b)) = (at(main), at(id)) {
                self.sel_extra = ids[a.min(b)..=a.max(b)].iter().copied().collect();
                self.sel_line = Some(id);
                return;
            }
        }
        if keys.ctrl {
            self.toggle_line(id);
            return;
        }
        self.select_line(Some(id));
        if let Some(t) = self.ed.chart.timing_point(id).map(|p| p.time) {
            self.stop(audio);
            self.set_time(t);
        }
    }

    /// Dragging the scroll bar's thumb to pointer height `y`.
    pub(super) fn tw_drag(&mut self, y: f32) {
        let [_, by, _, bh] = self.tw_bar;
        let most = self.tw_ids().len().saturating_sub(self.tw_rows());
        if bh > 0.0 {
            let f = ((y - by) / bh).clamp(0.0, 1.0);
            self.tw_scroll = (f * most as f32).round() as usize;
        }
    }

    /// Keys for the table while the pointer is over the window: walk the rows, delete.
    pub(super) fn tw_key(&mut self, code: KeyCode, keys: Keys, audio: Option<&Engine>) -> bool {
        if !self.windowed() || !inside(self.layout.window, self.mouse.0, self.mouse.1) {
            return false;
        }
        if matches!(code, KeyCode::Delete | KeyCode::Backspace) {
            if self.sel_line.is_none() {
                return false;
            }
            let b = if self.selected_lines().len() > 1 {
                Btn::T(TAct::Delete)
            } else {
                Btn::C(Cmd::DeleteLine)
            };
            self.press(b, keys, audio);
            return true;
        }
        let ids = self.tw_ids();
        let last = ids.len() as isize - 1;
        let rows = self.tw_rows() as isize;
        let cur = self
            .sel_line
            .and_then(|id| ids.iter().position(|x| *x == id))
            .map(|i| i as isize);
        let to = match code {
            KeyCode::ArrowUp => cur.map_or(0, |i| i - 1),
            KeyCode::ArrowDown => cur.map_or(0, |i| i + 1),
            KeyCode::PageUp => cur.map_or(0, |i| i - rows),
            KeyCode::PageDown => cur.map_or(0, |i| i + rows),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return false,
        };
        if last >= 0 {
            let id = ids[to.clamp(0, last) as usize];
            self.pick_row(id, Keys::default(), audio);
        }
        true
    }

    pub(super) fn draw_timing_window(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.window;
        let white = rgba(0xffffff, 1.0);
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x15151d, 1.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        c.text(x + 10.0, y + 9.0, 9.0, white, "TIMING LINES");
        let mut cx = x + 104.0;
        for (i, label) in ["All", "Red", "Green"].into_iter().enumerate() {
            let bw = Canvas::text_width(7.0, label) + 12.0;
            self.button(
                c,
                [cx, y + 6.0, bw, 14.0],
                label,
                Btn::TwFilter(i as u8),
                self.tw_filter == i as u8,
            );
            cx += bw + 3.0;
        }
        cx += 10.0;
        for (label, cmd) in [("+ Red", Cmd::AddRed), ("+ Green", Cmd::AddGreen)] {
            let bw = Canvas::text_width(7.0, label) + 12.0;
            self.button(c, [cx, y + 6.0, bw, 14.0], label, Btn::C(cmd), false);
            cx += bw + 3.0;
        }

        // ---- the table ----
        let tx = x + 8.0;
        let head_y = y + 30.0;
        for (off, name) in COLS.iter().zip([
            "Time", "Type", "BPM / SV", "Meter", "Sample", "Vol", "Kiai", "Omit",
        ]) {
            c.text(tx + 4.0 + off, head_y, 6.5, rgba(FAINT, 1.0), name);
        }
        c.rect(tx, head_y + 10.0, TABLE_W, 1.0, rgba(0x2c2c3c, 1.0));
        let ids = self.tw_ids();
        let rows = self.tw_rows();
        let most = ids.len().saturating_sub(rows);
        if self.tw_seen != self.sel_line {
            self.tw_seen = self.sel_line;
            if let Some(i) = self.sel_line.and_then(|s| ids.iter().position(|x| *x == s)) {
                if i < self.tw_scroll {
                    self.tw_scroll = i;
                } else if i >= self.tw_scroll + rows {
                    self.tw_scroll = i + 1 - rows;
                }
            }
        }
        self.tw_scroll = self.tw_scroll.min(most);
        let now = self.ed.time;
        let in_effect = timing::active_at(&self.ed.chart.timing, now).map(|p| p.id);
        // Lines inside the selected stretch of notes are tinted.
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for id in &self.ed.selection {
            if let Some(o) = self.ed.chart.object(*id) {
                lo = lo.min(o.time);
                hi = hi.max(self.ed.end_of(o).max(o.time));
            }
        }
        let picked = self.selected_lines();
        let rows_y = head_y + 14.0;
        if ids.is_empty() {
            c.text(
                tx + 4.0,
                rows_y + 4.0,
                7.0,
                rgba(DIM, 1.0),
                "No lines of this kind.",
            );
        }
        let first = self.tw_scroll;
        for (i, id) in ids.iter().skip(first).take(rows).enumerate() {
            let Some(p) = self.ed.chart.timing_point(*id).cloned() else {
                continue;
            };
            let ry = rows_y + i as f32 * ROW;
            let r = [tx, ry, TABLE_W, ROW - 1.0];
            if p.time >= lo && p.time <= hi {
                c.rect(r[0], r[1], r[2], r[3], rgba(0x1c2a22, 1.0));
            }
            let main = self.sel_line == Some(p.id);
            if main || picked.contains(&p.id) {
                c.rect(
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    rgba(if main { 0x5a2a20 } else { 0x3c241f }, 1.0),
                );
            } else if c.hovered(r[0], r[1], r[2], r[3]) {
                c.rect(r[0], r[1], r[2], r[3], rgba(0x23232f, 1.0));
            }
            if in_effect == Some(p.id) {
                c.rect(r[0], r[1], 2.0, r[3], rgba(accent(), 1.0));
            }
            let col = |k: usize| tx + 4.0 + COLS[k];
            let ty = ry + 2.5;
            c.text(col(0), ty, 7.0, rgba(0xd8d8e8, 1.0), &fmt_time(p.time));
            c.text(
                col(1),
                ty,
                7.0,
                rgba(if p.is_red() { RED_LINE } else { 0x4fd16b }, 1.0),
                if p.is_red() { "Red" } else { "Green" },
            );
            let val = match p.bpm() {
                Some(b) => format!("{b:.2}"),
                None => format!("x{:.2}", p.sv()),
            };
            c.text(col(2), ty, 7.0, white, &val);
            if p.is_red() {
                c.text(col(3), ty, 7.0, rgba(DIM, 1.0), &format!("{}/4", p.meter));
            }
            let set = format!("{}:{}", sample_name(p.sample_set), p.sample_index);
            c.text(col(4), ty, 7.0, rgba(DIM, 1.0), &set);
            c.text(col(5), ty, 7.0, rgba(DIM, 1.0), &p.volume.to_string());
            if p.kiai() {
                c.text(col(6), ty, 7.0, rgba(0xFFAA00, 1.0), "kiai");
            }
            if p.is_red() && p.omits_barline() {
                c.text(col(7), ty, 7.0, rgba(DIM, 1.0), "omit");
            }
            self.buttons.push((r, Btn::Line(p.id)));
        }
        self.tw_bar = [0.0; 4];
        if ids.len() > rows {
            let track = [tx + TABLE_W + 4.0, rows_y, 7.0, rows as f32 * ROW];
            c.rect(track[0], track[1], track[2], track[3], rgba(0x1d1d28, 1.0));
            let thumb = (track[3] * rows as f32 / ids.len() as f32).max(16.0);
            let ty = track[1] + (track[3] - thumb) * self.tw_scroll as f32 / most as f32;
            let lit = matches!(self.drag, Drag::TwScroll)
                || c.hovered(track[0], track[1], track[2], track[3]);
            c.round_rect(
                [track[0], ty],
                [track[2], thumb],
                3.0,
                rgba(if lit { 0x7a7a98 } else { 0x4a4a62 }, 1.0),
                rgba(0x4a4a62, 1.0),
                0.0,
            );
            self.buttons.push((track, Btn::TwBar));
            self.tw_bar = track;
        }
        c.text(
            tx + 4.0,
            y + h - 12.0,
            6.0,
            rgba(FAINT, 1.0),
            "Click selects and jumps, Shift takes a range, Ctrl toggles. Up/Down/PgUp/PgDn/Home/End walk the rows.",
        );

        // ---- the selected line / the SV tool ----
        let rx = tx + TABLE_W + 22.0;
        let rw = x + w - 10.0 - rx;
        c.rect(rx - 8.0, y + 26.0, 1.0, h - 34.0, rgba(0x2c2c3c, 1.0));
        let close = Canvas::text_width(7.0, "Close") + 14.0;
        self.button(
            c,
            [x + w - 8.0 - close, y + 6.0, close, 14.0],
            "Close",
            Btn::C(Cmd::TimingList),
            false,
        );
        self.line_tabs(c, x + w - 8.0 - close - 8.0, y + 7.0);
        let ry = y + 30.0;
        if self.svt.tab == 1 {
            self.draw_svt_panel(c, rx, ry, rw);
        } else {
            self.draw_line_editor(c, rx, ry, rw);
        }
    }
}
