//! The frame around the work: the menu bar, the mode bar with the transport, the panel the Compose
//! and Timing modes put beside the preview, the whole-song overview and the status line.

use super::cmd::{Cmd, Mode};
use super::lines::TAct;
use super::menu;
use super::{Btn, Field, RED_LINE, SAct, View, fmt_time};
use crate::editor::chart::{HitObject, Taiko};
use crate::editor::state::Tool;
use crate::editor::timing;
use crate::render::{Canvas, accent, rgba};

pub(super) const DIM: u32 = 0x9a9ab0;
pub(super) const FAINT: u32 = 0x6a6a80;
pub(super) const WARN: u32 = 0xfcb806;
pub(super) const PANEL: u32 = 0x15151d;
pub(super) const EDGE: u32 = 0x2c2c3c;

/// `text` broken into lines no wider than `w` at `size`.
pub(super) fn wrap(size: f32, w: f32, text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let try_line = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && Canvas::text_width(size, &try_line) > w {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = try_line;
            }
        }
        out.push(line);
    }
    out
}

impl View {
    /// Draws a panel with a title and returns where its content starts.
    pub(super) fn panel_frame(&mut self, c: &mut Canvas, r: [f32; 4], title: &str) -> (f32, f32) {
        let [x, y, w, _] = r;
        c.round_rect(
            [x, y],
            [r[2], r[3]],
            3.0,
            rgba(PANEL, 0.95),
            rgba(EDGE, 1.0),
            1.0,
        );
        c.text(x + 8.0, y + 5.0, 6.5, rgba(accent(), 1.0), title);
        c.rect(x + 8.0, y + 15.0, w - 16.0, 1.0, rgba(EDGE, 1.0));
        (x + 8.0, y + 21.0)
    }

    /// Text in `dim`, wrapped to `w`. Returns the next y.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paragraph(
        &mut self,
        c: &mut Canvas,
        x: f32,
        mut y: f32,
        w: f32,
        size: f32,
        color: u32,
        text: &str,
    ) -> f32 {
        for line in wrap(size, w, text) {
            c.text(x, y, size, rgba(color, 1.0), &line);
            y += size + 3.0;
        }
        y
    }

    // ---- the menu bar ----

    pub(super) fn draw_menubar(&mut self, c: &mut Canvas) {
        let [_, _, w, h] = self.layout.menu;
        c.rect(0.0, 0.0, w, h, rgba(0x0e0e14, 1.0));
        c.rect(0.0, h - 1.0, w, 1.0, rgba(EDGE, 1.0));
        let mut x = 4.0;
        for (i, m) in menu::MENUS.iter().enumerate() {
            let tw = Canvas::text_width(7.0, m.name) + 14.0;
            let open = self.menu == Some(i);
            if open || c.hovered(x, 0.0, tw, h) {
                c.rect(
                    x,
                    0.0,
                    tw,
                    h - 1.0,
                    rgba(if open { 0x3a3a52 } else { 0x23232f }, 1.0),
                );
            }
            c.text(x + 7.0, 3.5, 7.0, rgba(0xffffff, 1.0), m.name);
            self.buttons.push(([x, 0.0, tw, h], Btn::Menu(i)));
            x += tw;
        }
        let mut right = w - 8.0;
        if self.ed.modified() {
            let mark = "unsaved changes";
            c.text_right(right, 3.5, 7.0, rgba(WARN, 1.0), mark);
            right -= Canvas::text_width(7.0, mark) + 12.0;
        }
        let room = (right - x - 16.0).max(40.0);
        c.text_right(
            right,
            3.5,
            7.0,
            rgba(DIM, 1.0),
            &Canvas::fit(7.0, room, &self.label()),
        );
    }

    // ---- the mode bar and the transport ----

    /// A small `-  value  +` group. Returns where it ended.
    #[allow(clippy::too_many_arguments)]
    fn stepper(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        label: &str,
        value: &str,
        dec: Cmd,
        inc: Cmd,
    ) -> f32 {
        let mut x = x;
        c.text(x, y + 6.5, 6.0, rgba(FAINT, 1.0), label);
        x += Canvas::text_width(6.0, label) + 5.0;
        self.button(c, [x, y + 2.0, 14.0, 16.0], "-", Btn::C(dec), false);
        x += 16.0;
        let vw = Canvas::text_width(8.0, value).max(26.0);
        c.text_centered(x + vw / 2.0 + 2.0, y + 6.0, 8.0, rgba(0xffffff, 1.0), value);
        x += vw + 4.0;
        self.button(c, [x, y + 2.0, 14.0, 16.0], "+", Btn::C(inc), false);
        x + 20.0
    }

    pub(super) fn draw_modebar(&mut self, c: &mut Canvas) {
        let [bx, by, bw, bh] = self.layout.modes;
        c.round_rect(
            [bx, by],
            [bw, bh],
            3.0,
            rgba(0x12121a, 1.0),
            rgba(EDGE, 1.0),
            1.0,
        );
        let mut x = bx + 3.0;
        for m in Mode::ALL {
            let on = self.mode == m;
            let tw = Canvas::text_width(8.0, m.name()) + Canvas::text_width(5.5, m.key()) + 20.0;
            let r = [x, by + 2.0, tw, bh - 4.0];
            let hover = c.hovered(r[0], r[1], r[2], r[3]);
            c.round_rect(
                [r[0], r[1]],
                [r[2], r[3]],
                3.0,
                rgba(
                    if on {
                        0x3a1d18
                    } else if hover {
                        0x2a2a3c
                    } else {
                        0x12121a
                    },
                    1.0,
                ),
                rgba(0, 0.0),
                0.0,
            );
            if on {
                c.rect(
                    r[0] + 4.0,
                    r[1] + r[3] - 2.0,
                    r[2] - 8.0,
                    2.0,
                    rgba(accent(), 1.0),
                );
            }
            c.text(
                r[0] + 8.0,
                r[1] + 5.0,
                8.0,
                rgba(if on { 0xffffff } else { 0xc8c8d8 }, 1.0),
                m.name(),
            );
            c.text(
                r[0] + 12.0 + Canvas::text_width(8.0, m.name()),
                r[1] + 7.0,
                5.5,
                rgba(FAINT, 1.0),
                m.key(),
            );
            self.buttons.push((r, Btn::C(Cmd::Mode(m))));
            self.tips.push((r, Cmd::Mode(m).tip()));
            x += tw + 2.0;
        }
        x += 10.0;
        let end = bx + bw - 4.0;
        // Right edge first, so what doesn't fit is what matters least.
        let mut right = end;
        let save = if self.ed.modified() { "Save *" } else { "Save" };
        let sw = Canvas::text_width(7.0, save) + 14.0;
        right -= sw;
        self.button(
            c,
            [right, by + 3.0, sw, bh - 6.0],
            save,
            Btn::C(Cmd::Save),
            self.ed.modified(),
        );
        for (label, cmd) in [("Redo", Cmd::Redo), ("Undo", Cmd::Undo)] {
            let w = Canvas::text_width(7.0, label) + 14.0;
            right -= w + 3.0;
            if right > x + 220.0 {
                self.button(c, [right, by + 3.0, w, bh - 6.0], label, Btn::C(cmd), false);
            }
        }
        let y = by + 1.0;
        // Play.
        let playing = self.playing.is_some();
        let pw = 40.0;
        self.button(
            c,
            [x, by + 3.0, pw, bh - 6.0],
            if playing { "Pause" } else { "Play" },
            Btn::C(Cmd::Play),
            playing,
        );
        x += pw + 6.0;
        let time = fmt_time(self.ed.time);
        c.text(x, by + 6.0, 9.0, rgba(0xffffff, 1.0), &time);
        x += Canvas::text_width(9.0, &time) + 12.0;
        if x + 100.0 < right {
            let snap = format!("1/{}", self.ed.snap);
            x = self.stepper(c, x, y, "Snap", &snap, Cmd::SnapStep(-1), Cmd::SnapStep(1));
        }
        if x + 110.0 < right {
            let rate = format!("{}%", self.rate);
            x = self.stepper(c, x, y, "Speed", &rate, Cmd::RateStep(-1), Cmd::RateStep(1));
        }
        if x + 90.0 < right {
            c.text(x, y + 6.5, 6.0, rgba(FAINT, 1.0), "Zoom");
            x += Canvas::text_width(6.0, "Zoom") + 5.0;
            self.button(
                c,
                [x, y + 2.0, 14.0, 16.0],
                "-",
                Btn::C(Cmd::ZoomOut),
                false,
            );
            self.button(
                c,
                [x + 16.0, y + 2.0, 14.0, 16.0],
                "+",
                Btn::C(Cmd::ZoomIn),
                false,
            );
            self.button(
                c,
                [x + 32.0, y + 2.0, 22.0, 16.0],
                "Fit",
                Btn::C(Cmd::ZoomFit),
                false,
            );
        }
    }

    // ---- the Compose panel ----

    fn tool_hint(&self) -> &'static str {
        match self.ed.tool {
            Tool::Select => {
                "Click a note to select it; drag empty space to select a box. Drag a note to move it, drag the end of a roll to stretch it."
            }
            Tool::Note => {
                "Click the lane to place a note on the grid. W or R makes the note in hand a kat, E a big one. Click a placed note to give it that colour. Right click deletes."
            }
            Tool::Roll => {
                "Press where the drumroll starts and drag to where it ends, or click the start and then the end."
            }
            Tool::Swell => {
                "Press where the denden starts and drag to where it ends, or click the start and then the end."
            }
        }
    }

    pub(super) fn draw_compose_panel(&mut self, c: &mut Canvas) {
        let r = self.layout.inspector;
        let (x, mut y) = self.panel_frame(c, r, "NOTES");
        // The numbers about the difficulty take the right side when there is room.
        let side = r[2] > 380.0;
        let w = if side {
            r[2] * 0.62 - 24.0
        } else {
            r[2] - 16.0
        };
        let white = rgba(0xffffff, 1.0);
        let sel: Vec<HitObject> = self.selected_objects().into_iter().cloned().collect();
        let head = match sel.as_slice() {
            [] => "Nothing selected".to_string(),
            [o] => {
                let what = match o.taiko() {
                    Taiko::Don => "don",
                    Taiko::Kat => "kat",
                    Taiko::Roll => "drumroll",
                    Taiko::Swell => "denden",
                };
                let end = self.ed.end_of(o);
                let big = if o.big() { "big " } else { "" };
                if end > o.time {
                    format!("{big}{what}  {} - {}", fmt_time(o.time), fmt_time(end))
                } else {
                    format!("{big}{what}  {}", fmt_time(o.time))
                }
            }
            many => format!("{} notes selected", many.len()),
        };
        c.text(x, y, 7.5, white, &Canvas::fit(7.5, w, &head));
        y += 14.0;
        // Hitsounds: they act on the selection, or on what is placed next.
        // The Note tool's keys are for the note in hand, not the one just placed.
        let on_sel = !sel.is_empty() && self.ed.tool != Tool::Note;
        let has = |bit: i32| on_sel && sel.iter().all(|o| o.sound & bit != 0);
        let (kat, clap, big) = if !on_sel {
            (self.ed.kat, false, self.ed.big)
        } else {
            (
                has(crate::editor::chart::SOUND_WHISTLE),
                has(crate::editor::chart::SOUND_CLAP),
                has(crate::editor::chart::SOUND_FINISH),
            )
        };
        let who = if !on_sel { "New notes:" } else { "Selected:" };
        c.text(x, y + 4.0, 6.0, rgba(FAINT, 1.0), who);
        let bx = x + Canvas::text_width(6.0, "New notes:") + 8.0;
        self.row(
            c,
            bx,
            y,
            &[
                ("Kat  W".to_string(), Btn::C(Cmd::Whistle), kat),
                ("Clap  R".to_string(), Btn::C(Cmd::Clap), clap),
                ("Big  E".to_string(), Btn::C(Cmd::Finish), big),
            ],
        );
        y += 22.0;
        if sel.is_empty() {
            let hint = self.tool_hint();
            self.paragraph(c, x, y, w, 6.5, DIM, hint);
        } else {
            let items = [
                ("Flip  Q".to_string(), Btn::C(Cmd::Flip), false),
                ("Resnap".to_string(), Btn::Resnap, false),
                ("Clone  Ctrl+D".to_string(), Btn::C(Cmd::Duplicate), false),
                ("Reverse  Ctrl+G".to_string(), Btn::C(Cmd::Reverse), false),
            ];
            self.row(c, x, y, &items);
            y += 20.0;
            self.row(
                c,
                x,
                y,
                &[
                    (
                        "Nudge earlier  J".to_string(),
                        Btn::C(Cmd::Nudge(false)),
                        false,
                    ),
                    (
                        "Nudge later  K".to_string(),
                        Btn::C(Cmd::Nudge(true)),
                        false,
                    ),
                    ("Delete".to_string(), Btn::C(Cmd::Delete), false),
                ],
            );
            y += 22.0;
            self.paragraph(
                c,
                x,
                y,
                w,
                6.0,
                FAINT,
                "Resnap puts notes that are a hair off the grid back on it.",
            );
        }
        if side {
            self.draw_map_facts(
                c,
                [
                    r[0] + r[2] * 0.62,
                    r[1] + 21.0,
                    r[2] * 0.38 - 8.0,
                    r[3] - 28.0,
                ],
            );
        }
    }

    /// The difficulty in numbers.
    fn draw_map_facts(&mut self, c: &mut Canvas, [x, mut y, w, h]: [f32; 4]) {
        c.rect(x - 8.0, y - 1.0, 1.0, h, rgba(EDGE, 1.0));
        c.text(x, y, 6.0, rgba(FAINT, 1.0), "THIS DIFFICULTY");
        y += 12.0;
        let (mut don, mut kat, mut roll, mut swell) = (0, 0, 0, 0);
        for o in &self.ed.chart.objects {
            match o.taiko() {
                Taiko::Don => don += 1,
                Taiko::Kat => kat += 1,
                Taiko::Roll => roll += 1,
                Taiko::Swell => swell += 1,
            }
        }
        let bpm = timing::active_red(&self.ed.chart.timing, self.ed.time)
            .and_then(|p| p.bpm())
            .unwrap_or(0.0);
        let lines = [
            format!("{} notes", don + kat),
            format!("{don} don, {kat} kat"),
            format!("{roll} drumrolls, {swell} dendens"),
            format!("{bpm:.1} BPM at the playhead"),
            format!("Length {}", fmt_time(self.end_ms())),
            format!("{} bookmarks", self.ed.bookmarks.len()),
        ];
        for l in lines {
            c.text(x, y, 7.0, rgba(0xd0d0e0, 1.0), &Canvas::fit(7.0, w, &l));
            y += 11.0;
        }
    }

    // ---- the Timing panel ----

    pub(super) fn draw_timing_panel(&mut self, c: &mut Canvas) {
        let r = self.layout.inspector;
        let (x, y) = self.panel_frame(c, r, "TIMING LINES");
        self.line_tabs(c, r[0] + r[2] - 8.0, r[1] + 2.0);
        if self.svt.tab == 1 {
            self.draw_svt_panel(c, x, y, r[2] - 16.0);
        } else {
            self.draw_line_editor(c, x, y, r[2] - 16.0);
        }
    }

    /// The Line / SV tool switch, ending at `right`.
    pub(super) fn line_tabs(&mut self, c: &mut Canvas, mut right: f32, y: f32) {
        for (label, tab) in [("SV tool", 1u8), ("Line", 0u8)] {
            let bw = Canvas::text_width(6.5, label) + 12.0;
            right -= bw;
            self.button(
                c,
                [right, y, bw, 12.0],
                label,
                Btn::S(SAct::Tab(tab)),
                self.svt.tab == tab,
            );
            right -= 3.0;
        }
    }

    /// What can be done to the selected timing line(s): its numbers, kiai, copies.
    pub(super) fn draw_line_editor(&mut self, c: &mut Canvas, x: f32, mut y: f32, w: f32) {
        let white = rgba(0xffffff, 1.0);
        if self.selected_lines().len() > 1 {
            self.draw_line_tools(c, x, y, w);
            return;
        }
        let line = self
            .sel_line
            .and_then(|id| self.ed.chart.timing_point(id).cloned());
        let head = match &line {
            Some(p) => format!(
                "{} line at {}",
                if p.is_red() { "Red" } else { "Green" },
                fmt_time(p.time)
            ),
            None => {
                let t = &self.ed.chart.timing;
                let p = timing::active_at(t, self.ed.time);
                let bpm = timing::active_red(t, self.ed.time)
                    .and_then(|p| p.bpm())
                    .unwrap_or(0.0);
                format!("Now: {bpm:.2} BPM   SV x{:.2}", p.map_or(1.0, |p| p.sv()))
            }
        };
        c.text(
            x,
            y,
            7.5,
            if matches!(&line, Some(p) if p.is_red()) {
                rgba(RED_LINE, 1.0)
            } else {
                white
            },
            &Canvas::fit(7.5, w, &head),
        );
        y += 14.0;
        if let Some(p) = line {
            let mut fields: Vec<(Field, String)> = Vec::new();
            if p.is_red() {
                fields.push((Field::Bpm, format!("BPM {:.3}", p.bpm().unwrap_or(0.0))));
                fields.push((Field::Meter, format!("Meter {}", p.meter)));
            } else {
                fields.push((Field::Sv, format!("SV x{:.2}", p.sv())));
            }
            fields.push((Field::Volume, format!("Volume {}", p.volume)));
            fields.push((
                Field::Sample,
                format!("Sample {}", sample_name(p.sample_set)),
            ));
            fields.push((Field::Index, format!("Index {}", p.sample_index)));
            fields.push((Field::Time, format!("Time {}", fmt_time(p.time))));
            let col = (w / 2.0 - 4.0).max(100.0);
            let mut fy = y;
            for (cell, (f, label)) in fields.into_iter().enumerate() {
                let cx = x + (cell % 2) as f32 * (col + 8.0);
                if cell % 2 == 0 && cell > 0 {
                    fy += 18.0;
                }
                let shown = match &self.typing {
                    Some((tf, text)) if *tf == f => format!("{label}: {text}_"),
                    _ => label,
                };
                let lw = col - 40.0;
                self.button(
                    c,
                    [cx, fy, lw, 14.0],
                    &shown,
                    Btn::Edit(f),
                    self.typing.as_ref().is_some_and(|(tf, _)| *tf == f),
                );
                self.button(c, [cx + lw + 3.0, fy, 16.0, 14.0], "-", Btn::Dec(f), false);
                self.button(c, [cx + lw + 21.0, fy, 16.0, 14.0], "+", Btn::Inc(f), false);
            }
            fy += 20.0;
            let mut items = vec![("Kiai".to_string(), Btn::Kiai, p.kiai())];
            if p.is_red() {
                items.push(("Omit bar".to_string(), Btn::Omit, p.omits_barline()));
            }
            items.push(("At playhead".to_string(), Btn::T(TAct::Here), false));
            items.push(("Clone here".to_string(), Btn::T(TAct::CloneHere), false));
            items.push(("Delete".to_string(), Btn::C(Cmd::DeleteLine), false));
            self.wrap_row(c, x, fy, w, &items);
        } else {
            self.row(
                c,
                x,
                y,
                &[
                    ("+ Red line  Ctrl+P".to_string(), Btn::C(Cmd::AddRed), false),
                    (
                        "+ Green line  Ctrl+Shift+P".to_string(),
                        Btn::C(Cmd::AddGreen),
                        false,
                    ),
                ],
            );
            self.paragraph(
                c,
                x,
                y + 22.0,
                w,
                6.5,
                DIM,
                "A red line sets the BPM and the beat grid; a green line sets the slider velocity (how fast notes scroll). Lines are added at the playhead. Click a line on the timeline or in the graph to edit it, F6 opens the window with them all.",
            );
        }
    }

    // ---- the whole song ----

    pub(super) fn draw_overview(&mut self, c: &mut Canvas) {
        let [x, y, w, h] = self.layout.overview;
        let end = self.end_ms().max(1.0);
        c.rect(x, y, w, h, rgba(0x0e0e14, 1.0));
        let px = |t: f64| x + (t / end).clamp(0.0, 1.0) as f32 * w;
        // Kiai.
        let pts = &self.ed.chart.timing;
        let mut from: Option<f64> = None;
        for p in pts {
            match (p.kiai(), from) {
                (true, None) => from = Some(p.time),
                (false, Some(s)) => {
                    c.rect(
                        px(s),
                        y,
                        (px(p.time) - px(s)).max(1.0),
                        h,
                        rgba(0xFFAA00, 0.18),
                    );
                    from = None;
                }
                _ => {}
            }
        }
        if let Some(s) = from {
            c.rect(px(s), y, (x + w - px(s)).max(1.0), h, rgba(0xFFAA00, 0.18));
        }
        // Where the notes are.
        let bins = (w as usize).clamp(1, 1200);
        let mut count = vec![0u16; bins];
        for o in &self.ed.chart.objects {
            let i = ((o.time / end).clamp(0.0, 0.9999) * bins as f64) as usize;
            count[i] = count[i].saturating_add(1);
        }
        let top = count.iter().copied().max().unwrap_or(1).max(1) as f32;
        let bw = w / bins as f32;
        for (i, n) in count.iter().enumerate() {
            if *n > 0 {
                let bh = (h - 1.0) * (*n as f32 / top).sqrt().max(0.25);
                c.rect(
                    x + i as f32 * bw,
                    y + h - bh,
                    bw.max(1.0),
                    bh,
                    rgba(0x8a6a60, 0.85),
                );
            }
        }
        for b in &self.ed.bookmarks {
            c.rect(px(*b) - 0.5, y, 1.5, h, rgba(0x4f9dff, 1.0));
        }
        if let Some(g) = self.ed.gimmick() {
            for m in &g.marks {
                c.rect(px(m.time) - 0.5, y, 1.5, h, rgba(super::MARK_COL, 1.0));
            }
        }
        // What the timeline shows, and the playhead.
        let (a, b) = (px(self.view_start), px(self.view_end()));
        c.round_rect(
            [a, y],
            [(b - a).max(2.0), h],
            0.0,
            rgba(0xffffff, 0.08),
            rgba(0xffffff, 0.45),
            1.0,
        );
        c.rect(
            px(self.ed.time) - 0.75,
            y - 1.0,
            1.5,
            h + 2.0,
            rgba(0xffffff, 1.0),
        );
        self.tips.push((
            self.layout.overview,
            "The whole song\nClick or drag to jump. Brown bars are the notes, orange is kiai, blue marks are bookmarks."
                .to_string(),
        ));
    }

    // ---- the status line ----

    pub(super) fn draw_status(&mut self, c: &mut Canvas) {
        let [x, y, w, _] = self.layout.status;
        let t = &self.ed.chart.timing;
        let bpm = timing::active_red(t, self.ed.time)
            .and_then(|p| p.bpm())
            .unwrap_or(0.0);
        let info = format!(
            "{}    1/{} snap    {bpm:.1} BPM    SV x{:.2}    {} notes{}",
            fmt_time(self.ed.time),
            self.ed.snap,
            timing::sv_at(t, self.ed.time),
            self.ed.chart.objects.len(),
            if self.ed.chart.gimmick.is_some() {
                "    .tko"
            } else {
                ""
            },
        );
        c.text(x, y, 8.0, rgba(0xffffff, 1.0), &Canvas::fit(8.0, w, &info));
        if self.status != self.status_seen {
            self.status_seen = self.status.clone();
            self.status_at = std::time::Instant::now();
        }
        let fresh = !self.status.is_empty() && self.status_at.elapsed().as_secs() < 8;
        let (msg, col) = if fresh {
            (self.status.clone(), 0xffe9a8)
        } else {
            (self.mode_hint().to_string(), DIM)
        };
        c.text(x, y + 13.0, 7.0, rgba(col, 1.0), &Canvas::fit(7.0, w, &msg));
        c.text_right(
            x + w,
            y + 22.0,
            5.5,
            rgba(FAINT, 1.0),
            "? shortcuts   F1-F4 modes   Ctrl+S save",
        );
    }

    fn mode_hint(&self) -> &'static str {
        match self.mode {
            Mode::Compose => "Compose: place and edit notes. Hover anything for a tip.",
            Mode::Gimmicks => {
                "Gimmicks: build stable-compatible structures at the playhead or on the selection."
            }
            Mode::Timing => {
                "Timing: double click the graph to add a green line, drag a dot to change its velocity."
            }
            Mode::Effects => {
                "Effects: animate the playfield. Only .tko maps play these; the .osu is untouched."
            }
        }
    }
}

/// A sample set as stable's timing panel names it.
pub(super) fn sample_name(set: i32) -> &'static str {
    ["Auto", "Normal", "Soft", "Drum"][set.clamp(0, 3) as usize]
}
