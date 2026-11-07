//! What is drawn over the editor: tooltips, the "unsaved changes" question, the shortcut sheet, the
//! first-run tour.

use std::time::Duration;

use super::chrome::{DIM, WARN, wrap};
use super::cmd::{BINDS, Cmd, MOUSE, Mode};
use super::{Btn, Layout, Modal, View, inside};
use crate::render::{Canvas, accent, rgba};

// ---- the tour ----

#[derive(Clone, Copy)]
enum At {
    All,
    Menu,
    Modes,
    Timeline,
    Palette,
    Panel,
    Graph,
    Overview,
}

impl At {
    fn rect(self, l: &Layout, w: f32) -> Option<[f32; 4]> {
        Some(match self {
            At::All => return None,
            At::Menu => l.menu,
            At::Modes => l.modes,
            At::Timeline => [
                l.palette[0],
                l.timeline[1],
                l.timeline[0] + l.timeline[2] - l.palette[0],
                l.timeline[3],
            ],
            At::Palette => l.palette,
            At::Panel => l.inspector,
            At::Graph => l.graph,
            At::Overview => [
                l.overview[0],
                l.overview[1],
                l.overview[2].min(w),
                l.status[1] + l.status[3] - l.overview[1],
            ],
        })
    }
}

struct Step {
    title: &'static str,
    text: &'static str,
    at: At,
    mode: Mode,
}

const STEPS: &[Step] = &[
    Step {
        title: "Welcome to the map editor",
        text: "It edits osu!stable-compatible taiko maps: what you save plays the same in osu!stable. A map saved as .tko adds this client's own animations on top and never changes the .osu it came from.\nThis tour takes a minute. Esc skips it; Help > Take the tour shows it again.",
        at: At::All,
        mode: Mode::Compose,
    },
    Step {
        title: "The menus",
        text: "Every command is in these menus with its key beside it. If you can't find something, look here first.",
        at: At::Menu,
        mode: Mode::Compose,
    },
    Step {
        title: "Four modes",
        text: "F1 Compose is for notes. F2 Gimmicks builds stable-compatible tricks. F3 Timing edits BPM and slider velocity. F4 Effects animates the playfield of a .tko. A mode changes the panel and the graph; the timeline stays.",
        at: At::Modes,
        mode: Mode::Compose,
    },
    Step {
        title: "The timeline",
        text: "Time runs left to right. Click or drag the ruler on top to move the playhead (the white line); drag past an edge to scroll on. Grid lines follow your red line and the snap; red and green ticks are timing lines, blue marks are bookmarks. Alt+wheel zooms and the middle button pans.",
        at: At::Timeline,
        mode: Mode::Compose,
    },
    Step {
        title: "Tools",
        text: "1 Select, 2 Note, 3 Drumroll, 4 Denden. With the Note tool a click on the lane places a note on the grid.",
        at: At::Palette,
        mode: Mode::Compose,
    },
    Step {
        title: "Placing notes",
        text: "Click to place. Right click deletes. W makes kats (and dons again), E makes big notes: they apply to the selection, or to the notes you place next. Drag a note to move it; hold Shift to ignore the grid.",
        at: At::Timeline,
        mode: Mode::Compose,
    },
    Step {
        title: "Snap and speed",
        text: "Snap sets how finely the grid divides a beat (Ctrl+wheel, or Shift+1 to 9). Speed slows the music down (Ctrl+Up and Down) so you can place by ear. Left and Right step the playhead by one snap.",
        at: At::Modes,
        mode: Mode::Compose,
    },
    Step {
        title: "The panel and the preview",
        text: "The panel is for the current mode: in Compose it shows what is selected and the hitsound toggles. The preview is the real game drawn at the playhead, with timing, gimmicks and animations applied.",
        at: At::Panel,
        mode: Mode::Compose,
    },
    Step {
        title: "The whole song",
        text: "The bar under the graph is the whole song: brown for notes, orange for kiai, blue for bookmarks. Click or drag it to jump. The line below it shows where you are and a tip for the tool in hand.",
        at: At::Overview,
        mode: Mode::Compose,
    },
    Step {
        title: "Timing mode",
        text: "The graph is slider velocity. Double click empty space to add a green line, drag a dot to change it, drag the box to select several, right click a dot to delete it. Red lines (BPM) are added with Ctrl+P at the playhead. Click a curve (or its dots) to pick it: its round ends can be dragged and the SV tool edits it. F6 opens the window with every line.",
        at: At::Graph,
        mode: Mode::Timing,
    },
    Step {
        title: "Gimmicks and Effects",
        text: "Gimmicks builds the structures mappers use in stable (fake sliders, anti-barline walls, generated SV). Effects is the client's own: keyframes and a node graph that move the playfield, only for .tko maps.",
        at: At::Panel,
        mode: Mode::Effects,
    },
    Step {
        title: "Nodes",
        text: "On the Nodes tab of Effects you wire an event (Every beat, a miss, kiai) to an action (Pulse the zoom, Shake). Click + Every beat, click + Pulse, drag from one dot to the other, pick the Pulse's target and press Space. A red ! on a node means it can't do anything yet. The Recipe menu drops ready-made groups.",
        at: At::Panel,
        mode: Mode::Effects,
    },
    Step {
        title: "Manual and uploading",
        text: "Help > Manual explains timing a song, the SV tool, nodes and more, step by step. File > Upload sends your map to taiko.rs: it is public as CUSTOM and gives pp once an admin approves it.",
        at: At::Menu,
        mode: Mode::Compose,
    },
    Step {
        title: "Saving",
        text: "Ctrl+S saves and keeps a .bak of the original the first time. Esc leaves; with unsaved changes you are asked first. Press ? at any time for every shortcut.",
        at: At::Modes,
        mode: Mode::Compose,
    },
];

impl View {
    /// True once, when the tour has been finished or skipped (the screen then remembers that).
    pub fn take_tour_ended(&mut self) -> bool {
        std::mem::take(&mut self.tour_ended)
    }

    pub fn start_tour(&mut self) {
        self.menu = None;
        self.sheet = false;
        self.tw = false;
        self.tour_goto(0);
    }

    pub(super) fn tour_goto(&mut self, i: usize) {
        if i >= STEPS.len() {
            return self.tour_end();
        }
        self.tour = Some(i);
        let mode = STEPS[i].mode;
        if self.mode != mode {
            self.set_mode(mode);
        }
    }

    pub(super) fn tour_end(&mut self) {
        if self.tour.take().is_some() {
            self.tour_ended = true;
            self.set_mode(Mode::Compose);
        }
    }

    // ---- tooltips ----

    pub(super) fn draw_tip(&mut self, c: &mut Canvas) {
        if self.menu.is_some()
            || self.modal.is_some()
            || self.tour.is_some()
            || self.sheet
            || self.svt.drop.is_some()
            || self.fx_drop.is_some()
            || self.snap_pop.is_some()
            || self.moved_at.elapsed() < Duration::from_millis(400)
            || !matches!(self.drag, super::Drag::None)
        {
            return;
        }
        let (mx, my) = self.mouse;
        let Some((_, text)) = self.tips.iter().rev().find(|(r, _)| inside(*r, mx, my)) else {
            return;
        };
        let lines = wrap(6.5, 200.0, text);
        let w = lines
            .iter()
            .map(|l| Canvas::text_width(6.5, l))
            .fold(0.0, f32::max)
            + 14.0;
        let h = lines.len() as f32 * 9.5 + 10.0;
        let x = (mx + 10.0).min(c.width - w - 4.0).max(4.0);
        let y = if my + 24.0 + h < 478.0 {
            my + 14.0
        } else {
            my - h - 6.0
        };
        c.round_rect(
            [x, y],
            [w, h],
            3.0,
            rgba(0x0c0c12, 0.97),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        for (i, l) in lines.iter().enumerate() {
            let col = if i == 0 { 0xffffff } else { 0xc0c0d4 };
            c.text(x + 7.0, y + 5.0 + i as f32 * 9.5, 6.5, rgba(col, 1.0), l);
        }
    }

    // ---- overlays ----

    /// Everything that goes over the screen, in the order it stacks.
    pub(super) fn draw_overlays(&mut self, c: &mut Canvas) {
        self.draw_menu_drop(c);
        self.draw_svt_drop(c);
        self.draw_fx_drop(c);
        self.draw_snap_pop(c);
        if self.node_mode() {
            self.draw_node_pick(c);
        }
        if self.sheet {
            self.modal_from = self.buttons.len();
            self.draw_sheet(c);
        }
        match self.modal {
            Some(Modal::Leave) => {
                self.modal_from = self.buttons.len();
                self.draw_question(c);
            }
            Some(Modal::Upload) => {
                self.modal_from = self.buttons.len();
                self.draw_upload(c);
            }
            Some(Modal::Manual) => {
                self.modal_from = self.buttons.len();
                self.draw_manual(c);
            }
            Some(Modal::Setup) => {
                self.modal_from = self.buttons.len();
                self.draw_setup(c);
            }
            None => {}
        }
        if self.tour.is_some() {
            self.modal_from = self.buttons.len();
            self.draw_tour(c);
        }
        self.draw_tip(c);
    }

    fn scrim(&self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
    }

    // ---- the question on leaving ----

    fn draw_question(&mut self, c: &mut Canvas) {
        self.scrim(c);
        let (w, h) = (330.0, 100.0);
        let (x, y) = ((c.width - w) / 2.0, 170.0);
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        c.text(
            x + 14.0,
            y + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            "Save your changes?",
        );
        c.text(
            x + 14.0,
            y + 30.0,
            7.0,
            rgba(DIM, 1.0),
            &Canvas::fit(7.0, w - 28.0, "This map has changes that aren't saved yet."),
        );
        let mut bx = x + 14.0;
        for (label, cmd, on) in [
            ("Save and exit", Cmd::LeaveSave, true),
            ("Exit without saving", Cmd::LeaveDiscard, false),
            ("Keep editing", Cmd::Stay, false),
        ] {
            let bw = Canvas::text_width(7.0, label) + 14.0;
            self.button(c, [bx, y + 66.0, bw, 20.0], label, Btn::C(cmd), on);
            bx += bw + 6.0;
        }
    }

    // ---- the shortcut sheet ----

    fn draw_sheet(&mut self, c: &mut Canvas) {
        self.scrim(c);
        let w = (c.width - 40.0).min(780.0);
        let (x, y, h) = ((c.width - w) / 2.0, 26.0, 428.0);
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        c.text(
            x + 14.0,
            y + 10.0,
            10.0,
            rgba(0xffffff, 1.0),
            "Keyboard and mouse",
        );
        c.text(
            x + 14.0,
            y + 26.0,
            6.5,
            rgba(DIM, 1.0),
            "Laid out like osu!stable's editor. Esc closes this sheet.",
        );
        let close = "Close";
        let cw = Canvas::text_width(7.0, close) + 16.0;
        self.button(
            c,
            [x + w - cw - 12.0, y + 10.0, cw, 18.0],
            close,
            Btn::C(Cmd::Shortcuts),
            false,
        );

        enum Row {
            Head(&'static str),
            Key(&'static str, &'static str),
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut group = "";
        for b in BINDS.iter().filter(|b| !b.label.is_empty()) {
            if b.group != group {
                group = b.group;
                rows.push(Row::Head(group));
            }
            rows.push(Row::Key(b.label, b.cmd.name()));
        }
        rows.push(Row::Head("Mouse"));
        for (k, t) in MOUSE {
            rows.push(Row::Key(k, t));
        }
        let cols = if w > 600.0 { 3 } else { 2 };
        let per = rows.len().div_ceil(cols);
        let colw = (w - 28.0) / cols as f32;
        // The key column is as wide as the longest key name in the sheet.
        let label_w = rows
            .iter()
            .filter_map(|r| match r {
                Row::Key(k, _) => Some(Canvas::text_width(6.5, k)),
                Row::Head(_) => None,
            })
            .fold(0.0, f32::max)
            + 8.0;
        let top = y + 44.0;
        let mut at = 0;
        for col in 0..cols {
            let cx = x + 14.0 + col as f32 * colw;
            let mut ry = top;
            let mut n = 0;
            while at < rows.len() && n < per {
                match &rows[at] {
                    Row::Head(name) => {
                        if n > 0 {
                            ry += 4.0;
                        }
                        c.text(cx, ry, 7.0, rgba(accent(), 1.0), name);
                        ry += 11.0;
                    }
                    Row::Key(k, t) => {
                        c.text(cx, ry, 6.5, rgba(0xffffff, 1.0), k);
                        for line in wrap(6.5, colw - label_w - 10.0, t) {
                            c.text(cx + label_w, ry, 6.5, rgba(0xb4b4c8, 1.0), &line);
                            ry += 9.0;
                        }
                        ry += 1.0;
                    }
                }
                at += 1;
                n += 1;
            }
        }
    }

    // ---- the tour ----

    fn draw_tour(&mut self, c: &mut Canvas) {
        let Some(i) = self.tour else { return };
        let step = &STEPS[i.min(STEPS.len() - 1)];
        let target = step.at.rect(&self.layout, c.width);
        match target {
            None => self.scrim(c),
            Some([tx, ty, tw, th]) => {
                let dim = rgba(0x000000, 0.66);
                let (px, py) = (4.0, 3.0);
                let (l, t, r, b) = (
                    (tx - px).max(0.0),
                    (ty - py).max(0.0),
                    (tx + tw + px).min(c.width),
                    (ty + th + py).min(480.0),
                );
                c.rect(0.0, 0.0, c.width, t, dim);
                c.rect(0.0, b, c.width, 480.0 - b, dim);
                c.rect(0.0, t, l, b - t, dim);
                c.rect(r, t, c.width - r, b - t, dim);
                c.round_rect(
                    [l, t],
                    [r - l, b - t],
                    3.0,
                    rgba(0, 0.0),
                    rgba(accent(), 1.0),
                    2.0,
                );
            }
        }
        // The card, on the side of the target that has the room.
        let cw = 300.0;
        let body = wrap(7.0, cw - 28.0, step.text);
        let ch = 62.0 + body.len() as f32 * 10.0;
        let (cx, cy) = match target {
            None => ((c.width - cw) / 2.0, (480.0 - ch) / 2.0),
            Some([tx, ty, tw, th]) => {
                let below = ty + th + 12.0;
                let y = if below + ch < 476.0 {
                    below
                } else {
                    (ty - ch - 12.0).max(8.0)
                };
                let x = (tx + tw / 2.0 - cw / 2.0).clamp(8.0, c.width - cw - 8.0);
                (x, y)
            }
        };
        c.round_rect(
            [cx, cy],
            [cw, ch],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(accent(), 1.0),
            1.5,
        );
        c.text(
            cx + 14.0,
            cy + 11.0,
            9.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(9.0, cw - 28.0, step.title),
        );
        for (n, l) in body.iter().enumerate() {
            c.text(
                cx + 14.0,
                cy + 28.0 + n as f32 * 10.0,
                7.0,
                rgba(0xc8c8dc, 1.0),
                l,
            );
        }
        let by = cy + ch - 26.0;
        c.text(
            cx + 14.0,
            by + 6.0,
            6.5,
            rgba(WARN, 1.0),
            &format!("{} / {}", i + 1, STEPS.len()),
        );
        let last = i + 1 == STEPS.len();
        let next = if last { "Done" } else { "Next" };
        let nw = Canvas::text_width(7.0, next) + 18.0;
        let mut x = cx + cw - 14.0 - nw;
        self.button(c, [x, by, nw, 18.0], next, Btn::TourNext, true);
        if i > 0 {
            let bw = Canvas::text_width(7.0, "Back") + 14.0;
            x -= bw + 4.0;
            self.button(c, [x, by, bw, 18.0], "Back", Btn::TourBack, false);
        }
        if !last {
            let bw = Canvas::text_width(7.0, "Skip tour") + 14.0;
            x -= bw + 8.0;
            self.button(c, [x, by, bw, 18.0], "Skip tour", Btn::TourSkip, false);
        }
    }
}
