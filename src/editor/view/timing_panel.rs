//! The Timing panel's controls for a red line, laid out like the Effects pages: small-caps
//! headings with a rule, numbers in a sunken field with a joined - and +, actions solid grey,
//! delete red.

use super::chrome::{DIM, sample_name};
use super::cmd::Cmd;
use super::lines::TAct;
use super::look::{self, Look};
use super::{Btn, Field, View};
use crate::editor::chart::{TimingPoint, fmt_num};
use crate::render::{Canvas, rgba};

/// A row of the panel is this tall.
const ROW: f32 = 16.0;
/// How wide a row's caption is; its controls start after it.
const CAPTION: f32 = 36.0;

impl View {
    /// A caption, then a number: a sunken field showing it (or what is being typed into it) with
    /// a - and a + joined to its right. Returns the x after them.
    #[allow(clippy::too_many_arguments)]
    fn number(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        caption: &str,
        f: Field,
        shown: &str,
        fw: f32,
    ) -> f32 {
        let ty = y + (ROW - look::TEXT) / 2.0;
        c.text(x, ty, look::TEXT, rgba(DIM, 1.0), caption);
        let fx = x + CAPTION;
        let typing = matches!(&self.typing, Some((tf, _)) if *tf == f);
        let text = match &self.typing {
            Some((tf, t)) if *tf == f => format!("{t}_"),
            _ => shown.to_string(),
        };
        self.styled(
            c,
            [fx, y, fw, ROW],
            &text,
            Btn::Edit(f),
            typing,
            Look::Field,
        );
        self.styled(
            c,
            [fx + fw + 1.0, y, ROW, ROW],
            "-",
            Btn::Dec(f),
            false,
            Look::Step,
        );
        self.styled(
            c,
            [fx + fw + 2.0 + ROW, y, ROW, ROW],
            "+",
            Btn::Inc(f),
            false,
            Look::Step,
        );
        fx + fw + 2.0 + 2.0 * ROW
    }

    /// A caption, then a row of buttons, each as wide as its label.
    fn actions(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        caption: &str,
        items: &[(&str, TAct, &str)],
    ) {
        c.text(
            x,
            y + (ROW - look::TEXT) / 2.0,
            look::TEXT,
            rgba(DIM, 1.0),
            caption,
        );
        let mut bx = x + CAPTION;
        for (label, act, tip) in items {
            let w = look::width(Look::Action, label).max(24.0);
            self.styled(c, [bx, y, w, ROW], label, Btn::T(*act), false, Look::Action);
            self.tips.push(([bx, y, w, ROW], tip.to_string()));
            bx += w + 3.0;
        }
    }

    /// The controls of the selected red line `p`; returns the y under them.
    pub(super) fn draw_red_line(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        p: &TimingPoint,
    ) -> f32 {
        let two = w >= 420.0;
        let col = if two { (w - 16.0) / 2.0 } else { w };
        let (lx, rx) = (x, if two { x + col + 16.0 } else { x });

        // Position: where the line sits, and moving it along the grid.
        let mut ly = self.heading_note(c, lx, y, col, "Position", "notes stay put");
        self.number(
            c,
            lx,
            ly,
            "Time",
            Field::Time,
            &format!("{} ms", fmt_num(p.time)),
            70.0,
        );
        ly += ROW + 3.0;
        self.actions(
            c,
            lx,
            ly,
            "Nudge",
            &[
                ("-10", TAct::Ms(-10), "Move the line 10 ms earlier"),
                ("-1", TAct::Ms(-1), "Move the line 1 ms earlier"),
                ("+1", TAct::Ms(1), "Move the line 1 ms later"),
                ("+10", TAct::Ms(10), "Move the line 10 ms later"),
            ],
        );
        ly += ROW + 3.0;
        self.actions(
            c,
            lx,
            ly,
            "Move",
            &[
                (
                    "-1 bar",
                    TAct::Bars(-1),
                    "Move the line one bar (its meter's worth of beats) earlier. The notes stay where they are.",
                ),
                (
                    "+1 bar",
                    TAct::Bars(1),
                    "Move the line one bar later. The notes stay where they are.",
                ),
                (
                    "-1 beat",
                    TAct::Beats(-1),
                    "Move the line one beat earlier. The notes stay where they are.",
                ),
                (
                    "+1 beat",
                    TAct::Beats(1),
                    "Move the line one beat later. The notes stay where they are.",
                ),
            ],
        );
        ly += ROW + 3.0;

        // Tempo.
        let mut ry = if two { y } else { ly + 2.0 };
        ry = self.heading(c, rx, ry, col, "Tempo");
        self.number(
            c,
            rx,
            ry,
            "BPM",
            Field::Bpm,
            &format!("{:.3}", p.bpm().unwrap_or(0.0)),
            70.0,
        );
        ry += ROW + 3.0;
        self.actions(
            c,
            rx,
            ry,
            "Step",
            &[
                ("-1", TAct::Bpm(-100), "1 BPM slower"),
                ("+1", TAct::Bpm(100), "1 BPM faster"),
                ("-0.01", TAct::Bpm(-1), "0.01 BPM slower"),
                ("+0.01", TAct::Bpm(1), "0.01 BPM faster"),
                ("x2", TAct::BpmTimes(true), "Twice the BPM"),
                ("/2", TAct::BpmTimes(false), "Half the BPM"),
            ],
        );
        ry += ROW + 3.0;
        let after = self.number(c, rx, ry, "Meter", Field::Meter, &p.meter.to_string(), 40.0);
        let mut bx = after + 6.0;
        for (label, b, on) in [
            ("Kiai", Btn::Kiai, p.kiai()),
            ("Omit bar", Btn::Omit, p.omits_barline()),
        ] {
            let bw = look::width(Look::Toggle, label);
            if bx + bw <= rx + col {
                self.styled(c, [bx, ry, bw, ROW], label, b, on, Look::Toggle);
                bx += bw + 3.0;
            }
        }
        ry += ROW + 3.0;

        // Hitsounds, and the line itself.
        let mut y2 = ly.max(ry) + 2.0;
        y2 = self.heading(c, x, y2, w, "Sound");
        let third = (w - 8.0) / 3.0;
        let fw = (third - CAPTION - 2.0 - 2.0 * ROW).max(24.0);
        self.number(c, x, y2, "Volume", Field::Volume, &p.volume.to_string(), fw);
        self.number(
            c,
            x + third + 4.0,
            y2,
            "Sample",
            Field::Sample,
            sample_name(p.sample_set),
            fw,
        );
        self.number(
            c,
            x + 2.0 * (third + 4.0),
            y2,
            "Index",
            Field::Index,
            &p.sample_index.to_string(),
            fw,
        );
        y2 += ROW + 5.0;
        let mut bx = x;
        for (label, b, how, tip) in [
            (
                "At playhead",
                Btn::T(TAct::Here),
                Look::Action,
                "Move the line to the playhead",
            ),
            (
                "Clone here",
                Btn::T(TAct::CloneHere),
                Look::Action,
                "Copy the line to the playhead",
            ),
            ("Delete", Btn::C(Cmd::DeleteLine), Look::Danger, ""),
        ] {
            let bw = look::width(how, label);
            self.styled(c, [bx, y2, bw, ROW], label, b, false, how);
            if !tip.is_empty() {
                self.tips.push(([bx, y2, bw, ROW], tip.to_string()));
            }
            bx += bw + 4.0;
        }
        y2 + ROW
    }
}
