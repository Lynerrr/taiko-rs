//! Timing > Snap notes to the grid: the snap options in one window (which divisions, how far an
//! object may move, which part of the map, what besides the notes), with what it would do worked
//! out live underneath, so nothing changes until Snap is pressed (and one Ctrl+Z takes it back).
//! The rules are in `editor::resnap`.

use super::chrome::{DIM, WARN, wrap};
use super::look::{self, Look};
use super::{Btn, Modal, View};
use crate::editor::resnap::{DIVISORS, PATTERN_MS, SHIFTS, SnapOpts, SnapPlan};
use crate::hit::Canvas;
use crate::render::{accent, rgba};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RsAct {
    /// Ticks or unticks a division.
    Div(u32),
    /// The divisions of a family: 0 the halves (1/1..1/16), 1 the thirds (1/3..1/12), 2 the odd
    /// ones (1/5, 1/7, 1/9).
    Family(u8),
    /// Back to Map Cleaner's divisions (1/16 and 1/12).
    Default,
    /// The largest move (an index into `SHIFTS`).
    Reach(u8),
    /// Only the selection (true) or the whole map.
    Scope(bool),
    Ends,
    Greens,
    Apply,
    Cancel,
}

/// What a button of the window looks like.
pub(super) fn look(a: RsAct) -> Look {
    match a {
        RsAct::Div(_) | RsAct::Reach(_) | RsAct::Scope(_) => Look::Chip,
        RsAct::Family(_) | RsAct::Default => Look::Action,
        RsAct::Ends | RsAct::Greens => Look::Toggle,
        RsAct::Apply => Look::Primary,
        RsAct::Cancel => Look::Ghost,
    }
}

const FAMILIES: [&[u32]; 3] = [&[1, 2, 4, 8, 16], &[3, 6, 12], &[5, 7, 9]];
const FAMILY_NAMES: [&str; 3] = ["1/2 1/4 1/8 1/16", "1/3 1/6 1/12", "1/5 1/7 1/9"];

impl View {
    /// Timing > Snap notes to the grid.
    pub(super) fn open_resnap(&mut self) {
        self.commit_typing();
        self.modal = Some(Modal::Resnap);
    }

    /// What the window's options would do, worked out again only when they or the chart change.
    pub(super) fn resnap_plan(&mut self) -> SnapPlan {
        let key = (
            self.ed.history.rev(),
            self.rs_opts.clone(),
            self.ed.selection.clone(),
        );
        if let Some((k, plan)) = &self.rs_plan
            && *k == key
        {
            return plan.clone();
        }
        let plan = self.ed.plan_resnap(&self.rs_opts);
        self.rs_plan = Some((key, plan.clone()));
        plan
    }

    pub(super) fn press_resnap(&mut self, a: RsAct) {
        let o = &mut self.rs_opts;
        match a {
            RsAct::Div(d) => {
                if let Some(i) = o.divisors.iter().position(|x| *x == d) {
                    o.divisors.remove(i);
                } else {
                    o.divisors.push(d);
                    o.divisors.sort_unstable();
                }
            }
            RsAct::Family(f) => {
                // All of them ticked: untick them; else tick them all.
                let fam = FAMILIES[f as usize];
                if fam.iter().all(|d| o.divisors.contains(d)) {
                    o.divisors.retain(|d| !fam.contains(d));
                } else {
                    o.divisors.extend_from_slice(fam);
                }
                o.divisors.sort_unstable();
                o.divisors.dedup();
            }
            RsAct::Default => o.divisors = SnapOpts::default().divisors,
            RsAct::Reach(i) => o.max_shift = SHIFTS[(i as usize).min(SHIFTS.len() - 1)],
            RsAct::Scope(sel) => o.only_selected = sel,
            RsAct::Ends => o.ends = !o.ends,
            RsAct::Greens => o.greens = !o.greens,
            RsAct::Cancel => self.modal = None,
            RsAct::Apply => self.apply_resnap(),
        }
    }

    pub(super) fn apply_resnap(&mut self) {
        let opts = self.rs_opts.clone();
        if opts.divisors.is_empty() {
            self.status = "tick at least one division to snap to".into();
            return;
        }
        let plan = self.ed.resnap(&opts);
        self.modal = None;
        self.rs_plan = None;
        if plan.is_empty() {
            self.status = "everything is on the grid already".into();
            return;
        }
        self.mark_stale();
        let mut said = format!("snapped {} notes", plan.moved);
        if plan.ends > 0 {
            said += &format!(", {} ends", plan.ends);
        }
        if plan.lines > 0 {
            said += &format!(", {} green lines", plan.lines);
        }
        said += " (Ctrl+Z takes it back)";
        self.status = said;
    }

    pub(super) fn draw_resnap(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
        let plan = self.resnap_plan();
        let picked = !self.ed.selection.is_empty();
        let w = 460.0_f32.min(c.width - 20.0);
        let inner = w - 28.0;
        let small = 6.5;
        let line = 9.5;
        let about = wrap(
            small,
            inner,
            "Puts notes that are a little off back on the nearest tick of the divisions ticked, the way Mapping Tools' Map Cleaner does. Notes farther off than the reach were put there on purpose and stay. Green lines on a note go with it; lines close together (SV patterns) stay; no note lands on another.",
        );
        let summary = self.resnap_summary(&plan);
        // Title, about, then the four sections (heading 14 each): divisions (two rows of buttons
        // and a line), reach, where, also; then what it would do and the buttons.
        let h = 27.0
            + about.len() as f32 * line
            + 8.0
            + (14.0 + 20.0 + 20.0 + 18.0)
            + (14.0 + 22.0)
            + (14.0 + 22.0)
            + (14.0 + 22.0 + 8.0)
            + summary.len() as f32 * line
            + 12.0
            + 20.0
            + 12.0;
        let (x, y) = ((c.width - w) / 2.0, ((480.0 - h) / 2.0 - 6.0).max(4.0));
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        let x0 = x + 14.0;
        c.text(
            x0,
            y + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            "Snap notes to the grid",
        );
        let mut ty = y + 27.0;
        for l in &about {
            c.text(x0, ty, small, rgba(DIM, 1.0), l);
            ty += line;
        }
        ty += 8.0;

        // Divisions.
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Divisions");
        ty += 14.0;
        let mut bx = x0;
        for d in DIVISORS {
            let label = format!("1/{d}");
            let bw = look::width(Look::Chip, &label).max(26.0);
            let on = self.rs_opts.divisors.contains(&d);
            self.styled(
                c,
                [bx, ty, bw, 16.0],
                &label,
                Btn::Rs(RsAct::Div(d)),
                on,
                Look::Chip,
            );
            bx += bw + 3.0;
        }
        ty += 20.0;
        let mut bx = x0;
        for (i, name) in FAMILY_NAMES.iter().enumerate() {
            let bw = look::width(Look::Action, name);
            self.styled(
                c,
                [bx, ty, bw, 16.0],
                name,
                Btn::Rs(RsAct::Family(i as u8)),
                false,
                Look::Action,
            );
            self.tips.push((
                [bx, ty, bw, 16.0],
                "Ticks these divisions together, or unticks them when they all are.".into(),
            ));
            bx += bw + 3.0;
        }
        let label = "Default (1/16 + 1/12)";
        let bw = look::width(Look::Action, label);
        self.styled(
            c,
            [bx, ty, bw, 16.0],
            label,
            Btn::Rs(RsAct::Default),
            false,
            Look::Action,
        );
        ty += 20.0;
        let chosen: Vec<String> = self
            .rs_opts
            .divisors
            .iter()
            .map(|d| format!("1/{d}"))
            .collect();
        let chosen = if chosen.is_empty() {
            "none ticked: nothing can snap".to_string()
        } else {
            format!("A note goes to the nearest of: {}", chosen.join(" "))
        };
        c.text(
            x0,
            ty + 4.0,
            small,
            rgba(
                if self.rs_opts.divisors.is_empty() {
                    WARN
                } else {
                    DIM
                },
                1.0,
            ),
            &Canvas::fit(small, inner, &chosen),
        );
        ty += 18.0;

        // Reach.
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Reach");
        ty += 14.0;
        let mut bx = x0;
        for (i, s) in SHIFTS.iter().enumerate() {
            let label = if s.is_finite() {
                format!("{s} ms")
            } else {
                "any".to_string()
            };
            let bw = look::width(Look::Chip, &label).max(34.0);
            let on = self.rs_opts.max_shift == *s;
            self.styled(
                c,
                [bx, ty, bw, 16.0],
                &label,
                Btn::Rs(RsAct::Reach(i as u8)),
                on,
                Look::Chip,
            );
            bx += bw + 3.0;
        }
        c.text(
            bx + 6.0,
            ty + 4.5,
            small,
            rgba(DIM, 1.0),
            &Canvas::fit(small, x0 + inner - bx - 6.0, "the most a note moves"),
        );
        ty += 22.0;

        // Where.
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Where");
        ty += 14.0;
        let mut bx = x0;
        let n = self.ed.selection.len();
        let sel_label = if picked {
            format!("The {n} selected")
        } else {
            "The selected (none)".to_string()
        };
        for (label, sel) in [(sel_label, true), ("The whole map".to_string(), false)] {
            let bw = look::width(Look::Chip, &label);
            let on = if picked {
                self.rs_opts.only_selected == sel
            } else {
                !sel
            };
            self.styled(
                c,
                [bx, ty, bw, 16.0],
                &label,
                Btn::Rs(RsAct::Scope(sel)),
                on,
                Look::Chip,
            );
            bx += bw + 3.0;
        }
        ty += 22.0;

        // Also.
        c.text(x0, ty, 7.5, rgba(accent(), 1.0), "Also");
        ty += 14.0;
        let mut bx = x0;
        for (label, act, on, tip) in [
            (
                "Drumroll and denden ends",
                RsAct::Ends,
                self.rs_opts.ends,
                "Snaps where drumrolls and dendens end too (a drumroll never past the next note).",
            ),
            (
                "Green lines on no note",
                RsAct::Greens,
                self.rs_opts.greens,
                "Snaps green lines that aren't on a note. Lines on a note always go with it; a line with another within 10 ms is part of an SV pattern and stays.",
            ),
        ] {
            let bw = look::width(Look::Toggle, label);
            self.styled(c, [bx, ty, bw, 16.0], label, Btn::Rs(act), on, Look::Toggle);
            self.tips.push(([bx, ty, bw, 16.0], tip.to_string()));
            bx += bw + 6.0;
        }
        ty += 22.0 + 8.0;

        // What it would do.
        for (i, l) in summary.iter().enumerate() {
            let col = if i == 0 { 0xffffff } else { DIM };
            c.text(x0, ty, small, rgba(col, 1.0), l);
            ty += line;
        }
        ty += 12.0;
        let label = "Snap";
        let bw = look::width(Look::Primary, label) + 16.0;
        self.styled(
            c,
            [x0, ty, bw, 20.0],
            label,
            Btn::Rs(RsAct::Apply),
            false,
            Look::Primary,
        );
        let cw = look::width(Look::Ghost, "Cancel");
        self.styled(
            c,
            [x0 + bw + 6.0, ty, cw, 20.0],
            "Cancel",
            Btn::Rs(RsAct::Cancel),
            false,
            Look::Ghost,
        );
        c.text(
            x0 + bw + cw + 16.0,
            ty + 7.0,
            small,
            rgba(DIM, 1.0),
            "Enter snaps, Esc closes. One Ctrl+Z takes it all back.",
        );
    }

    /// The lines saying what a snap would do.
    fn resnap_summary(&self, p: &SnapPlan) -> Vec<String> {
        let mut out = Vec::new();
        let scope = if p.selection {
            "selected"
        } else {
            "in the map"
        };
        out.push(if p.is_empty() {
            format!(
                "Nothing to do: {} {scope} {} on the grid.",
                count(p.looked, "note"),
                if p.looked == 1 { "is" } else { "are" }
            )
        } else {
            let largest = if p.moved > 0 {
                format!(", by up to {} ms", fmt_ms(p.largest))
            } else {
                String::new()
            };
            format!(
                "{} of {} {scope} move{largest}; {} and {} snap.",
                p.moved,
                count(p.looked, "note"),
                count(p.ends, "end"),
                count(p.lines, "green line")
            )
        });
        let mut left = Vec::new();
        if p.far > 0 {
            left.push(format!(
                "{} farther off than the reach (on purpose)",
                count(p.far, "note")
            ));
        }
        if p.clashes > 0 {
            left.push(format!(
                "{} that would land on another",
                count(p.clashes, "note")
            ));
        }
        if p.patterns > 0 {
            left.push(format!(
                "{} within {PATTERN_MS} ms of another (SV patterns)",
                count(p.patterns, "green line")
            ));
        }
        if !left.is_empty() {
            out.push(format!("Left as they are: {}.", left.join(", ")));
        }
        out
    }
}

/// `n` and `what`, plural unless one.
fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

fn fmt_ms(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches(".0").to_string()
}
