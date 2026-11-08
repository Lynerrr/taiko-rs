//! The Motion page (F4 Effects, .tko maps): the notes picked on the timeline get a group of
//! their own, and that group an approach path (picked from ready-made ones and shaped by hand in
//! the preview, with a pace along it and moments it takes them on and off the lane) and what each
//! note does as its own hit nears (ready-made moves, or keys on a small graph).

use super::look::{self, Look, look_of};
use super::{Btn, Drag, Keys, Mode, View, draw_preview_hint, inside};
use crate::editor::motion::{
    BEFORES, PACES, PATHS, REVEALS, SPEEDS, ScrollMove, before_tracks, holds, pace_track,
};
use crate::game::{HIT_X, LANE_Y};
use crate::render::{Canvas, accent, rgba};
use crate::tko::gimmick::{BEFORE_BEATS, FieldProp, NoteProp, ShowColour, ShowSize};
use crate::tko::path::PathTable;
use crate::tko::track::{Ease, Key};
use crate::video::FRAME_VW;
use std::time::Instant;

/// The Motion page's tab.
pub(super) const PAGE: u8 = 5;
/// How close (preview units) to a point a press takes it.
const GRAB: f32 = 6.0;
/// How far inside the preview's edge the handle of a point off it is drawn and taken.
const HANDLE_INSET: f32 = 4.0;
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
    /// The page's tab: 0 the path, 1 before the hit, 2 show as, 3 the scroll, 4 fakes.
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
    /// The shared scroll's speed from the playhead (in `SPEEDS`).
    Speed(usize),
    /// The speed at the playhead a step faster (true) or slower.
    Nudge(bool),
    /// A move of the shared scroll from the playhead (in `ScrollMove::ALL`).
    Move(usize),
    /// The speed key at the playhead glides to the next one, or stops gliding.
    Glide,
    /// The hit target slides along the lane to a place in `DRUM_SPOTS` over a beat.
    DrumTo(usize),
    ClearScroll,
    /// The picked dons and kats become fakes, or real again.
    ToggleFake,
    /// Fake copies of the picked notes, `FAKE_SHIFTS[i]` beats away.
    FakeCopy(usize),
    /// The colour the group's notes show as (`SHOW_COLOURS`).
    ShowColour(usize),
    /// The size they show as (`SHOW_SIZES`).
    ShowSize(usize),
    /// When their truth shows (`REVEALS`).
    Reveal(usize),
    /// The one-line row of groups shows a page of later ones (true) or earlier ones.
    Groups(bool),
    /// The picked notes are drawn over every effect, or not again.
    Above,
    /// Deletes the group being shaped, with the effects on it.
    DeleteGroup,
}

/// What to do when nothing is picked to shape.
const PICK_FIRST: &str = "pick notes first: press 1 (Select), then click a note on the timeline or drag a box over several";

/// What each of the page's tabs is for.
const TAB_ABOUT: [&str; 5] = [
    "Pick notes on the timeline and give them a path: they fly in along it and land on the drum on time.",
    "Pick notes on the timeline and give them a move: each note does it as its own hit nears.",
    "Pick notes and make them show as another colour or size until just before the hit. Only the look lies.",
    "One speed for the whole lane, like mania SV: 0 freezes every note, below 0 runs them back. Green lines stay as they are.",
    "Fake notes look like notes and move like them, but are never hit: no score, no combo, no sound. They vanish at the drum.",
];

/// The Show as tab's tab number.
const SHOW_TAB: u8 = 2;
/// The Scroll tab's.
pub(super) const SCROLL_TAB: u8 = 3;

const SHOW_COLOURS: [(&str, ShowColour); 4] = [
    ("Their own", ShowColour::Same),
    ("All as dons", ShowColour::Don),
    ("All as kats", ShowColour::Kat),
    ("Swapped", ShowColour::Swap),
];

const SHOW_SIZES: [(&str, ShowSize); 4] = [
    ("Their own", ShowSize::Same),
    ("All small", ShowSize::Small),
    ("All big", ShowSize::Big),
    ("Swapped", ShowSize::Swap),
];

/// How far from the picked notes fake copies go, in beats.
const FAKE_SHIFTS: [(&str, f64); 5] = [
    ("Half a beat before", -0.5),
    ("A 1/4 before", -0.25),
    ("A 1/4 after", 0.25),
    ("Half a beat after", 0.5),
    ("A beat after", 1.0),
];

/// Where the drum can be slid to from the Scroll tab: virtual units along the lane.
const DRUM_SPOTS: [(&str, f64); 4] = [
    ("Home", 0.0),
    ("A quarter in", 160.0),
    ("Half way", 320.0),
    ("Far", 480.0),
];

/// A speed as mania mappers write it: 1x, 0.5x, -1x.
/// "1 note" or "3 notes".
fn count_word(n: usize) -> String {
    format!("{n} note{}", if n == 1 { "" } else { "s" })
}

/// "picked note" or "3 picked notes", after "the".
fn picked_word(n: usize) -> String {
    if n == 1 {
        "picked note".to_string()
    } else {
        format!("{n} picked notes")
    }
}

/// "1 note shows" or "3 notes show": `n` notes and a verb that agrees.
fn notes_word(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", count_word(n), if n == 1 { one } else { many })
}

fn speed_label(v: f64) -> String {
    format!("{}x", (v * 100.0).round() / 100.0)
}

impl View {
    pub(super) fn motion_mode(&self) -> bool {
        self.mode == Mode::Effects && self.g_page == PAGE && self.ed.is_tko()
    }

    /// The group being shaped: the one that holds just the notes picked on the timeline, or
    /// with nothing picked the one shaped last, if it is still there. None while the notes
    /// picked have no group of their own yet.
    pub(super) fn motion_group(&self) -> Option<usize> {
        if !self.ed.selection.is_empty() {
            return self.ed.group_of_selection();
        }
        let g = self.m_group?;
        (g < self.ed.gimmick()?.groups.len()).then_some(g)
    }

    /// The group a "give them ..." button works on: the picked notes' own, made for them if they
    /// have none yet; with nothing picked, the one shaped last. Says what to do when there is
    /// none.
    fn motion_target(&mut self) -> Option<usize> {
        if self.ed.selection.is_empty() {
            let g = self.motion_group();
            if g.is_none() {
                self.status = PICK_FIRST.into();
            }
            return g;
        }
        match self.ed.motion_group() {
            Ok(g) => {
                self.m_group = Some(g);
                Some(g)
            }
            Err(e) => {
                self.status = e;
                None
            }
        }
    }

    pub(super) fn press_motion(&mut self, a: MAct) {
        let t = self.ed.snapped(self.ed.time);
        let groups = |v: &View| v.ed.gimmick().map_or(0, |g| g.groups.len());
        let had = groups(self);
        // What gives the picked notes something makes them a group first; the rest work on the
        // group there is.
        let g = match a {
            MAct::Path(_)
            | MAct::Before(_)
            | MAct::ShowColour(_)
            | MAct::ShowSize(_)
            | MAct::Reveal(_) => self.motion_target(),
            _ => self.motion_group(),
        };
        if let Some(g) = g {
            self.m_group = Some(g);
        }
        let made = groups(self) != had;
        if matches!(
            a,
            MAct::Prop(_) | MAct::Pick(_) | MAct::Before(_) | MAct::Tab(_)
        ) {
            self.m_key = None;
        }
        let done = match (a, g) {
            (MAct::Tab(i), _) => {
                self.m_tab = i;
                false
            }
            (MAct::Prop(i), _) => {
                self.m_prop = i.min(GRAPH_PROPS.len() - 1);
                false
            }
            (MAct::Speed(i), _) => SPEEDS
                .get(i)
                .is_some_and(|&v| self.ed.set_scroll_speed(t, v)),
            (MAct::Nudge(up), _) => {
                let v = self.ed.scroll_speed(t);
                let step = if up { 0.05 } else { -0.05 };
                self.ed
                    .set_scroll_speed(t, ((v + step) * 100.0).round() / 100.0)
            }
            (MAct::Move(i), _) => ScrollMove::ALL
                .get(i)
                .is_some_and(|&m| self.ed.scroll_move(m, t)),
            (MAct::Glide, _) => self.ed.scroll_glide(t),
            (MAct::DrumTo(i), _) => DRUM_SPOTS
                .get(i)
                .is_some_and(|&(_, to)| self.ed.drum_to(t, to)),
            (MAct::ClearScroll, _) => self.ed.clear_scroll(),
            (MAct::Above, _) => match self.ed.toggle_above() {
                Ok((n, on)) => {
                    self.status = if on {
                        format!(
                            "{} over every effect now (a blackout too)",
                            notes_word(n, "shows", "show")
                        )
                    } else {
                        format!("{} under the effects again", notes_word(n, "is", "are"))
                    };
                    true
                }
                Err(e) => {
                    self.status = e;
                    false
                }
            },
            (MAct::ToggleFake, _) => match self.ed.toggle_fake() {
                Ok((n, on)) => {
                    self.status = if on {
                        format!("{n} notes are fakes now: drawn in the game, never hit")
                    } else {
                        format!("{n} fakes are real notes again")
                    };
                    true
                }
                Err(e) => {
                    self.status = e;
                    false
                }
            },
            (MAct::FakeCopy(i), _) => {
                match FAKE_SHIFTS.get(i).map(|&(_, b)| self.ed.add_fake_copies(b)) {
                    Some(Ok(n)) => {
                        self.status = format!("{n} fake copies added and selected");
                        true
                    }
                    Some(Err(e)) => {
                        self.status = e;
                        false
                    }
                    None => false,
                }
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
                    } else if self.m_tab == SHOW_TAB {
                        self.status =
                            "pick what the notes show as, and when their truth shows".into();
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
            (MAct::Groups(on), _) => {
                let by = self.m_strip_shown.max(1);
                self.page_groups(on, by);
                false
            }
            (MAct::Pick(i), _) => {
                // Its notes picked on the timeline too, so what is shaped is what is picked.
                self.m_group = Some(i);
                self.ed.select_group(i);
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
            (MAct::DeleteGroup, Some(g)) => {
                let name = self
                    .ed
                    .gimmick()
                    .and_then(|gm| gm.groups.get(g))
                    .map_or_else(String::new, |gr| gr.name.clone());
                let gone = self.ed.delete_group(g);
                if gone {
                    self.m_group = None;
                    self.m_key = None;
                    self.status = format!(
                        "{name} deleted, with its motion and the effects on it; its notes are plain notes again (Ctrl+Z brings it back)"
                    );
                }
                gone
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
            (MAct::ShowColour(i), Some(g)) => SHOW_COLOURS
                .get(i)
                .is_some_and(|&(_, c)| self.ed.edit_show(g, |s| s.colour = c)),
            (MAct::ShowSize(i), Some(g)) => SHOW_SIZES
                .get(i)
                .is_some_and(|&(_, z)| self.ed.edit_show(g, |s| s.size = z)),
            (MAct::Reveal(i), Some(g)) => REVEALS
                .get(i)
                .is_some_and(|&(_, b)| self.ed.edit_show(g, |s| s.reveal = b)),
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
                if self.ed.selection.is_empty() {
                    self.status = PICK_FIRST.into();
                } else if !matches!(
                    a,
                    MAct::Path(_)
                        | MAct::Before(_)
                        | MAct::ShowColour(_)
                        | MAct::ShowSize(_)
                        | MAct::Reveal(_)
                ) {
                    // (Those that give something said why they couldn't.)
                    self.status =
                        "the picked notes have no motion yet: give them some first".into();
                }
                false
            }
        };
        if done || made {
            self.mark_stale();
        }
    }

    /// The Motion chapter's example over the map's first kiai (or its first notes).
    pub(super) fn motion_showcase(&mut self) {
        let first_kiai = self
            .ed
            .chart
            .timing
            .iter()
            .find(|p| p.kiai())
            .and_then(|p| self.ed.kiai_around(p.time));
        let a = first_kiai.map_or_else(
            || self.ed.chart.objects.first().map_or(0.0, |o| o.time),
            |k| k.0,
        );
        match self.ed.motion_example(a) {
            Ok(parts) => {
                self.set_time(a);
                self.mark_stale();
                self.status = format!(
                    "example from {}: {}. Space plays it; Ctrl+Z takes off one part",
                    super::fmt_time(a),
                    parts.join(", ")
                );
            }
            Err(e) => self.status = e,
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
        for (k, (label, b, on)) in items.iter().enumerate() {
            let l = look_of(*b);
            let bw = look::width(l, label).min(w);
            if x > x0 && x + bw > x0 + w {
                x = x0;
                y += CHIP_ROW;
            }
            self.styled(c, [x, y, bw, CHIP_ROW - 2.0], label, *b, *on, l);
            let next = items.get(k + 1).map_or(Look::Plain, |n| look_of(n.1));
            x += bw + look::gap(l, next).min(3.0);
        }
        y + CHIP_ROW
    }

    /// The Motion page, scrolled when it is taller than the panel (a narrow window).
    pub(super) fn draw_motion_page(&mut self, c: &mut Canvas, x: f32, top: f32, inner: f32) {
        if self.m_scroll_tab != self.m_tab {
            self.m_scroll_tab = self.m_tab;
            self.m_scroll = 0.0;
        }
        self.m_strip = [0.0; 4];
        let (scroll, most) = self.draw_scrolled(c, x, top, inner, self.m_scroll, |v, c, y, s| {
            v.draw_motion_body(c, x, y, inner, s)
        });
        (self.m_scroll, self.m_page_most) = (scroll, most);
    }

    /// The wheel over the Motion page: over the row of groups it pages through them, elsewhere it
    /// scrolls the page when it is taller than the panel. False when it isn't its.
    pub(super) fn wheel_motion_page(&mut self, notches: i32) -> bool {
        let (mx, my) = self.mouse;
        if !self.motion_mode() || !inside(self.layout.inspector, mx, my) {
            return false;
        }
        if inside(self.m_strip, mx, my) {
            self.page_groups(notches < 0, 1);
            return true;
        }
        if self.m_page_most <= 0.0 {
            return false;
        }
        self.m_scroll = (self.m_scroll - notches as f32 * 20.0).clamp(0.0, self.m_page_most);
        true
    }

    /// On the timeline, under the notes, the group the Motion page shapes: a bracket under each
    /// run of its notes, named where it starts, so it can be told from the rest of the song.
    /// `bottom` is the lane's.
    pub(super) fn draw_group_on_timeline(&self, c: &mut Canvas, left: f32, bottom: f32) {
        if !self.motion_mode() || self.m_tab >= SCROLL_TAB {
            return;
        }
        let Some(gr) = self
            .motion_group()
            .and_then(|g| self.ed.gimmick()?.groups.get(g))
        else {
            return;
        };
        let (from, to) = (self.view_start, self.view_end());
        let pad = 40.0 * self.ms_per_px;
        let objs = &self.ed.chart.objects;
        let first = objs.partition_point(|o| o.time < from - pad);
        // Runs of the group's notes with none of the others between them.
        let mut runs: Vec<(f64, f64)> = Vec::new();
        let mut open: Option<(f64, f64)> = None;
        for o in &objs[first..] {
            if o.time > to + pad {
                break;
            }
            if holds(&gr.notes, o.time) {
                open = Some(open.map_or((o.time, o.time), |(a, _)| (a, o.time)));
            } else if let Some(run) = open.take() {
                runs.push(run);
            }
        }
        runs.extend(open);
        let col = accent();
        let by = bottom - 6.0;
        let mut named_to = f32::MIN;
        for (a, b) in runs {
            let (ax, bx) = (self.x_of(a) - 8.0, self.x_of(b) + 8.0);
            c.rect(ax, by, bx - ax, 2.5, rgba(col, 0.95));
            c.rect(ax, by - 4.0, 1.5, 6.5, rgba(col, 0.95));
            c.rect(bx - 1.5, by - 4.0, 1.5, 6.5, rgba(col, 0.95));
            // Named at its start (or the lane's left edge), unless the last name is still there.
            let nx = ax.max(left + 2.0) + 3.0;
            if nx >= named_to && bx > left {
                let label = format!("{} ({})", gr.name, gr.notes.len());
                c.text(nx, by - 8.0, 5.5, rgba(col, 1.0), &label);
                named_to = nx + Canvas::text_width(5.5, &label) + 8.0;
            }
        }
    }

    /// The groups the row shows: those that move or lie, as their index and label.
    pub(super) fn moving_groups(&self) -> Vec<(usize, String)> {
        self.ed.gimmick().map_or_else(Vec::new, |g| {
            g.groups
                .iter()
                .enumerate()
                .filter(|(_, gr)| gr.path.is_some() || !gr.before.is_empty() || !gr.show.is_true())
                .map(|(i, gr)| (i, format!("{} ({})", gr.name, gr.notes.len())))
                .collect()
        })
    }

    /// The row of groups shows `by` later ones (`on`) or earlier ones.
    fn page_groups(&mut self, on: bool, by: usize) {
        let n = self.moving_groups().len();
        self.m_first = if on {
            (self.m_first + by).min(n.saturating_sub(1))
        } else {
            self.m_first.saturating_sub(by)
        };
    }

    /// The groups already moving, in one line however many there are: what doesn't fit is paged
    /// to with the arrows (or the wheel), which say how many are hidden each way, and the group
    /// being shaped is kept in sight. Pasting what was copied leads it.
    fn draw_groups_row(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, current: Option<usize>) {
        let groups = self.moving_groups();
        let paste = self.m_copied.is_some();
        if groups.is_empty() && !paste {
            return;
        }
        let mut lx = x;
        if w >= 220.0 {
            // (Left out in a narrow window, for the groups' room.)
            c.text(x, y + 3.0, 6.0, rgba(0x9a9ab0, 1.0), "Moving:");
            lx += Canvas::text_width(6.0, "Moving:") + 5.0;
        }
        let end = x + w;
        self.m_strip = [x, y, w, CHIP_ROW];
        let chip = |label: &str| Canvas::text_width(7.0, label) + 10.0;
        if paste {
            let label = "Paste motion";
            let bw = chip(label);
            self.button(
                c,
                [lx, y, bw, CHIP_ROW - 2.0],
                label,
                Btn::M(MAct::Paste),
                false,
            );
            lx += bw + 3.0;
        }
        let n = groups.len();
        self.m_strip_shown = n;
        if n == 0 {
            return;
        }
        let widths: Vec<f32> = groups.iter().map(|(_, l)| chip(l) + 3.0).collect();
        let room = end - lx;
        if widths.iter().sum::<f32>() <= room {
            self.m_first = 0;
            for (i, label) in &groups {
                lx = self.group_chip(c, lx, y, end - lx, label, *i, current);
            }
            return;
        }
        // Paged: arrows at both ends, the groups between them.
        let arrow = |k: usize, on: bool| {
            if on {
                format!("{k} >")
            } else {
                format!("< {k}")
            }
        };
        let aw = chip(&arrow(n, true)) + 3.0;
        let room = (room - 2.0 * aw).max(0.0);
        // How many fit from `first` on (one at least, cut to the room).
        let fit = |first: usize| {
            let mut used = 0.0;
            let mut k = 0;
            for wd in &widths[first..] {
                if k > 0 && used + wd > room {
                    break;
                }
                used += wd;
                k += 1;
            }
            k
        };
        // The latest start that still fills the row, so the last page isn't mostly empty.
        let mut last = n - 1;
        while last > 0 && last - 1 + fit(last - 1) >= n {
            last -= 1;
        }
        let mut first = self.m_first.min(last);
        if self.m_first_for != current {
            // A newly shaped group comes into sight.
            self.m_first_for = current;
            if let Some(k) = groups.iter().position(|(i, _)| Some(*i) == current) {
                first = first.min(k);
                while first < k && first + fit(first) <= k {
                    first += 1;
                }
            }
        }
        self.m_first = first;
        let k = fit(first);
        self.m_strip_shown = k;
        let a = arrow(first, false);
        self.button(
            c,
            [lx, y, chip(&a), CHIP_ROW - 2.0],
            &a,
            Btn::M(MAct::Groups(false)),
            false,
        );
        let mut gx = lx + aw;
        for (i, label) in &groups[first..first + k] {
            gx = self.group_chip(c, gx, y, end - aw - gx, label, *i, current);
        }
        let b = arrow(n - first - k, true);
        let bw = chip(&b);
        self.button(
            c,
            [end - bw, y, bw, CHIP_ROW - 2.0],
            &b,
            Btn::M(MAct::Groups(true)),
            false,
        );
    }

    /// One group's button in the row, at most `w` wide. Returns where the next one goes.
    #[allow(clippy::too_many_arguments)]
    fn group_chip(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        i: usize,
        current: Option<usize>,
    ) -> f32 {
        // Never so narrow that its name is cut to nothing.
        let bw = (Canvas::text_width(7.0, label) + 10.0).min(w.max(44.0));
        let text = Canvas::fit(7.0, bw - 8.0, label);
        self.button(
            c,
            [x, y, bw, CHIP_ROW - 2.0],
            &text,
            Btn::M(MAct::Pick(i)),
            current == Some(i),
        );
        x + bw + 3.0
    }

    /// The page from `y`, scrolled `scroll` up.
    fn draw_motion_body(&mut self, c: &mut Canvas, x: f32, mut y: f32, inner: f32, scroll: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let small = 6.0;
        // The tabs, each saying what it is for when hovered.
        let tabs = self.motion_tabs();
        let first = self.buttons.len();
        y = self.chips(c, x, y, inner, &tabs) + 1.0;
        for (k, text) in TAB_ABOUT.iter().enumerate() {
            if let Some(&(r, _)) = self.buttons.get(first + k) {
                self.tips.push((r, text.to_string()));
            }
        }
        // Under them the groups moving already, in one line however many there are; before
        // there are any (and on the Scroll and Fakes tabs), what the tab is for.
        let current = self.motion_group();
        let listed = self.m_tab < SCROLL_TAB
            && (self.m_copied.is_some() || !self.moving_groups().is_empty());
        if listed {
            self.draw_groups_row(c, x, y, inner, current);
            y += CHIP_ROW;
        } else {
            let about = TAB_ABOUT[(self.m_tab as usize).min(TAB_ABOUT.len() - 1)];
            c.text(x, y + 1.0, small, dim, &Canvas::fit(small, inner, about));
            y += 10.0;
        }
        if self.m_tab >= SCROLL_TAB {
            let y = y + 1.0;
            self.m_graph = [0.0; 4];
            return if self.m_tab == SCROLL_TAB {
                self.draw_scroll_tab(c, x, y, inner)
            } else {
                self.draw_fakes_tab(c, x, y, inner)
            };
        }
        y += 1.0;
        // What is being shaped, and the step to take next: the notes picked on the timeline are
        // what the buttons below work on.
        let picked = self.ed.selection.len();
        let what = match self.m_tab {
            0 => "a path",
            SHOW_TAB => "a disguise",
            _ => "a move",
        };
        let mut items = Vec::new();
        let step = match current {
            Some(g) => {
                // What can be done to the group as a whole sits on its line.
                items.push(("Select them".to_string(), Btn::M(MAct::SelectNotes), false));
                items.push(("Copy motion".to_string(), Btn::M(MAct::Copy), false));
                if self.m_tab == 0 && self.ed.path_of(g).is_some() {
                    items.push(("Remove path".to_string(), Btn::M(MAct::Clear), false));
                }
                items.push(("Delete".to_string(), Btn::M(MAct::DeleteGroup), false));
                let (name, n) = self
                    .ed
                    .gimmick()
                    .and_then(|gm| gm.groups.get(g))
                    .map_or((String::new(), 0), |gr| (gr.name.clone(), gr.notes.len()));
                if picked == 0 {
                    format!("Shaping {name} ({n} notes); pick others to switch.")
                } else {
                    format!("Shaping the {n} picked notes ({name}).")
                }
            }
            None if picked == 0 => format!(
                "First pick notes: press 1 (Select), then click a note on the timeline or drag a box over several. Then give them {what} below."
            ),
            None => {
                items.push((
                    format!("Give the {} {what}", picked_word(picked)),
                    Btn::M(MAct::UseSelection),
                    true,
                ));
                format!("{} picked: pick {what} below, or", count_word(picked))
            }
        };
        let head = rgba(0xffffff, 1.0);
        if items.is_empty() {
            // Nothing to put beside it: wrapped, never cut off.
            y = self.paragraph(c, x, y + 2.0, inner, 6.5, 0xffffff, &step) + 2.0;
            if self.m_tab == 0 {
                self.m_graph = [0.0; 4];
                self.draw_path_tab(c, current, x, y, inner);
            } else if self.m_tab == SHOW_TAB {
                self.m_graph = [0.0; 4];
                self.draw_show_tab(c, current, x, y, inner);
            } else {
                self.draw_before_tab(c, current, x, y, inner, scroll);
            }
            return;
        }
        let chips_w: f32 = items
            .iter()
            .map(|(l, _, _)| Canvas::text_width(7.0, l) + 13.0)
            .sum();
        let lw = if items.is_empty() {
            inner
        } else {
            Canvas::text_width(6.5, &step).min((inner - chips_w - 6.0).max(inner * 0.4))
        };
        c.text(x, y + 3.0, 6.5, head, &Canvas::fit(6.5, lw, &step));
        if !items.is_empty() {
            // (Wrapping under itself in a narrow window rather than over what follows.)
            let first = self.buttons.len();
            y = self.chips(c, x + lw + 6.0, y, inner - lw - 6.0, &items) - CHIP_ROW;
            let delete: Vec<[f32; 4]> = self.buttons[first..]
                .iter()
                .filter(|(_, b)| *b == Btn::M(MAct::DeleteGroup))
                .map(|(r, _)| *r)
                .collect();
            for r in delete {
                self.tips.push((
                    r,
                    "Deletes this group: its path, moves and disguise, and the effect blocks on it. Its notes become plain notes. Ctrl+Z brings it back.".to_string(),
                ));
            }
        }
        y += CHIP_ROW + 2.0;
        if self.m_tab == 0 {
            self.m_graph = [0.0; 4];
            self.draw_path_tab(c, current, x, y, inner);
        } else if self.m_tab == SHOW_TAB {
            self.m_graph = [0.0; 4];
            self.draw_show_tab(c, current, x, y, inner);
        } else {
            self.draw_before_tab(c, current, x, y, inner, scroll);
        }
    }

    fn motion_tabs(&self) -> Vec<(String, Btn, bool)> {
        ["Path", "Before the hit", "Show as", "Scroll", "Fakes"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name.to_string(),
                    Btn::M(MAct::Tab(i as u8)),
                    self.m_tab == i as u8,
                )
            })
            .collect()
    }

    fn draw_show_tab(&mut self, c: &mut Canvas, g: Option<usize>, x: f32, y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let show = g.map(|g| self.ed.show_of(g)).unwrap_or_default();
        let col = (inner - 10.0) / 2.0;
        let right = x + col + 10.0;
        self.heading(c, x, y, col, "Colour");
        let items: Vec<(String, Btn, bool)> = SHOW_COLOURS
            .iter()
            .enumerate()
            .map(|(i, &(name, v))| {
                (
                    name.to_string(),
                    Btn::M(MAct::ShowColour(i)),
                    show.colour == v,
                )
            })
            .collect();
        let ly = self.chips(c, x, y + 9.0, col, &items) + 2.0;
        self.heading(c, x, ly, col, "Size");
        let items: Vec<(String, Btn, bool)> = SHOW_SIZES
            .iter()
            .enumerate()
            .map(|(i, &(name, v))| (name.to_string(), Btn::M(MAct::ShowSize(i)), show.size == v))
            .collect();
        self.chips(c, x, ly + 9.0, col, &items);
        self.heading(c, right, y, col, "The truth shows");
        let items: Vec<(String, Btn, bool)> = REVEALS
            .iter()
            .enumerate()
            .map(|(i, &(name, b))| (name.to_string(), Btn::M(MAct::Reveal(i)), show.reveal == b))
            .collect();
        let first = self.buttons.len();
        let ly = self.chips(c, right, y + 9.0, col, &items);
        let rects: Vec<[f32; 4]> = self.buttons[first..].iter().map(|(r, _)| *r).collect();
        for r in rects {
            self.tips.push((
                r,
                "When the lie gives way to the truth. What hits them, the score and the sound stay their own. Rolls and swells never lie. Players can turn lies off: Settings, Show true notes.".to_string(),
            ));
        }
        // Over the effects: for a note that must show through a blackout.
        let y = self.heading(c, right, ly + 2.0, col, "Over effects");
        let (above, picked) = self.ed.above_picked();
        if picked == 0 {
            c.text(
                right,
                y + 1.0,
                5.5,
                dim,
                &Canvas::fit(
                    5.5,
                    col,
                    "Pick notes to show them over every effect, a blackout too.",
                ),
            );
            return;
        }
        let label = format!(
            "show over every effect ({picked} note{})",
            if picked == 1 { "" } else { "s" }
        );
        let first = self.buttons.len();
        self.chips(
            c,
            right,
            y,
            col,
            &[(label, Btn::M(MAct::Above), above == picked)],
        );
        if let Some(&(r, _)) = self.buttons.get(first) {
            self.tips.push((
                r,
                "Drawn over every effect: a blackout, a flash or a glitch doesn't hide them, and effects on all the notes leave them be. They still move with the playfield, and with effects on a group they are in. On the timeline they wear a white dot.".to_string(),
            ));
        }
    }

    fn draw_fakes_tab(&mut self, c: &mut Canvas, x: f32, y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let picked = self.ed.selected();
        let hits = picked.iter().filter(|o| !o.fake).count();
        let fakes = picked.len() - hits;
        let col = (inner - 10.0) / 2.0;
        let right = x + col + 10.0;
        self.heading(c, x, y, col, "The picked notes");
        let label = if fakes > 0 && hits == 0 {
            format!("Make the {fakes} fakes real")
        } else {
            format!("Make the {} picked notes fakes", picked.len())
        };
        let ly = self.chips(
            c,
            x,
            y + 9.0,
            col,
            &[(label, Btn::M(MAct::ToggleFake), false)],
        );
        for (i, line) in [
            "On the timeline fakes are hollow with an F. Select, move,",
            "copy and delete them like notes; groups, paths and",
            "moves take them by their time like real ones.",
        ]
        .iter()
        .enumerate()
        {
            c.text(x, ly + 1.0 + i as f32 * 8.0, 5.5, dim, line);
        }
        self.heading(c, right, y, col, "Add fake copies of the picked notes");
        let items: Vec<(String, Btn, bool)> = FAKE_SHIFTS
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.to_string(), Btn::M(MAct::FakeCopy(i)), false))
            .collect();
        self.chips(c, right, y + 9.0, col, &items);
    }

    fn draw_scroll_tab(&mut self, c: &mut Canvas, x: f32, y: f32, inner: f32) {
        let dim = rgba(0x9a9ab0, 1.0);
        let t = self.ed.snapped(self.ed.time);
        let now = self.ed.scroll_speed(t);
        let col = (inner - 10.0) / 2.0;
        let right = x + col + 10.0;
        let label = format!("Speed from the playhead (now {})", speed_label(now));
        self.heading(c, x, y, col, &label);
        let items: Vec<(String, Btn, bool)> = SPEEDS
            .iter()
            .enumerate()
            .map(|(i, &v)| (speed_label(v), Btn::M(MAct::Speed(i)), now == v))
            .collect();
        let ly = self.chips(c, x, y + 9.0, col, &items);
        let items = vec![
            ("Slower".to_string(), Btn::M(MAct::Nudge(false)), false),
            ("Faster".to_string(), Btn::M(MAct::Nudge(true)), false),
            (
                "Glide to the next key".to_string(),
                Btn::M(MAct::Glide),
                false,
            ),
        ];
        let ly = self.chips(c, x, ly, col, &items);
        c.text(
            x,
            ly + 1.0,
            5.5,
            dim,
            &Canvas::fit(
                5.5,
                col,
                "The graph under the timeline is the speed: click adds a key, drag moves it.",
            ),
        );
        self.heading(c, right, y, col, "Moves from the playhead");
        let mut items: Vec<(String, Btn, bool)> = ScrollMove::ALL
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name().to_string(), Btn::M(MAct::Move(i)), false))
            .collect();
        items.push((
            "Clear the scroll".to_string(),
            Btn::M(MAct::ClearScroll),
            false,
        ));
        let ry = self.chips(c, right, y + 9.0, col, &items);
        self.heading(
            c,
            right,
            ry,
            col,
            "Slide the drum along the lane (over a beat)",
        );
        let items: Vec<(String, Btn, bool)> = DRUM_SPOTS
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.to_string(), Btn::M(MAct::DrumTo(i)), false))
            .collect();
        self.chips(c, right, ry + 9.0, col, &items);
    }

    fn draw_path_tab(&mut self, c: &mut Canvas, g: Option<usize>, x: f32, y: f32, inner: f32) {
        let path = g.and_then(|g| self.ed.path_of(g)).cloned();
        // Two columns: where the path starts on the left, when and how fast on the right.
        let col = (inner - 10.0) / 2.0;
        let right = x + col + 10.0;
        let top = y;
        self.heading(c, x, y, col, "Where they come from");
        let items: Vec<(String, Btn, bool)> = PATHS
            .iter()
            .enumerate()
            .map(|(i, (name, pts))| {
                (
                    name.to_string(),
                    Btn::M(MAct::Path(i)),
                    path.as_ref()
                        .is_some_and(|p| p.points == pts.to_vec() && p.straight == pts.is_empty()),
                )
            })
            .collect();
        self.chips(c, x, y + 9.0, col, &items);
        let Some(path) = path else {
            let dim = rgba(0x9a9ab0, 1.0);
            let lines: &[&str] = if self.ed.selection.is_empty() {
                &[]
            } else {
                &[
                    "These notes have no path yet: pick where",
                    "they come from on the left, then drag its",
                    "points in the preview above.",
                ]
            };
            for (i, line) in lines.iter().enumerate() {
                c.text(right, top + i as f32 * 8.0, 6.0, dim, line);
            }
            return;
        };
        self.heading(
            c,
            right,
            top,
            col,
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
        self.heading(c, right, y, col, "Pace along it");
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
        self.chips(c, right, y + 9.0, col, &items);
    }

    fn draw_before_tab(
        &mut self,
        c: &mut Canvas,
        g: Option<usize>,
        x: f32,
        y: f32,
        inner: f32,
        scroll: f32,
    ) {
        let dim = rgba(0x9a9ab0, 1.0);
        let before = g
            .and_then(|g| self.ed.before_of(g))
            .cloned()
            .unwrap_or_default();
        let col = (inner - 10.0) * 0.42;
        let right = x + col + 10.0;
        let rw = inner - col - 10.0;
        self.heading(c, x, y, col, "Ready-made");
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
        self.chips(c, x, y + 9.0, col, &items);
        if g.is_none() && self.ed.selection.is_empty() {
            self.m_graph = [0.0; 4];
            return;
        }
        // The graph: one property's keys, beats before the hit left to right (a key put on it
        // gives the picked notes their group).
        self.heading(c, right, y, rw, "Keys by beats before each note's hit");
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
        // As tall as the panel leaves, measured unscrolled so scrolling doesn't stretch it.
        let bottom = self.layout.inspector[1] + self.layout.inspector[3] - 6.0;
        let gh = (bottom - gy - scroll - 26.0).max(24.0);
        let rect = [right, gy, rw, gh];
        self.m_graph = rect;
        self.draw_before_graph(c, rect, &before);
        let ky = gy + gh + 2.0;
        match self.before_key() {
            Some(k) => {
                let w = (rw / 2.0 - 2.0).max(60.0);
                self.key_number(
                    c,
                    [right, ky, w],
                    super::keys::KF::BeforeValue,
                    "Value",
                    super::keys::trim_num(k.v),
                );
                self.key_number(
                    c,
                    [right + w + 4.0, ky, w],
                    super::keys::KF::BeforeBeats,
                    "Beats before",
                    super::keys::trim_num(k.t),
                );
            }
            None => {
                c.text(
                    right,
                    ky + 4.0,
                    5.5,
                    dim,
                    &Canvas::fit(
                        5.5,
                        rw,
                        "click: add a key   drag: move it   right click: remove it   Shift: free, off the 1/8 beat grid and round values",
                    ),
                );
            }
        }
    }

    /// The key picked on the before-the-hit graph, if it is still there.
    fn before_key(&self) -> Option<Key> {
        let g = self.motion_group()?;
        let (p, _, _) = self.graph_prop();
        let i = self.m_key?;
        self.ed.before_of(g)?.get(p.name())?.keys.get(i).copied()
    }

    /// The picked before-the-hit key gets `v` for its value or its beats before the hit.
    pub(super) fn before_key_set(&mut self, f: super::keys::KF, v: f64) {
        let (Some(g), Some(k), Some(i)) = (self.motion_group(), self.before_key(), self.m_key)
        else {
            return;
        };
        let (p, _, _) = self.graph_prop();
        let key = if f == super::keys::KF::BeforeValue {
            Key { v, ..k }
        } else {
            Key { t: v, ..k }
        };
        let mut at = i;
        if self.ed.edit_before(g, p, false, |t| {
            t.remove(i);
            at = t.set(key);
        }) {
            // Held in range by the edit, so look it up again by where it went.
            self.m_key = self
                .ed
                .before_of(g)
                .and_then(|b| b.get(p.name()))
                .and_then(|tr| tr.near(key.t.clamp(-BEFORE_BEATS.1, BEFORE_BEATS.0), 1e-6))
                .or(Some(at));
            self.mark_stale();
        }
    }

    /// Steps the picked before-the-hit key's value or beats by `n` notches.
    pub(super) fn before_key_step(&mut self, f: super::keys::KF, n: f64) {
        let Some(k) = self.before_key() else {
            return;
        };
        let (_, lo, hi) = self.graph_prop();
        let v = if f == super::keys::KF::BeforeValue {
            let step = super::keys::nice_step(hi - lo);
            super::keys::snap_value(k.v + n * step, (step * n.abs()).min(step))
        } else {
            // An eighth of a beat each.
            super::keys::snap_value(k.t + n / 8.0, (n.abs() / 8.0).min(0.125))
        };
        self.before_key_set(f, v);
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
        } else if free {
            (v * 100.0).round() / 100.0
        } else {
            super::keys::snap_value(v, super::keys::nice_step(hi - lo))
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
        // A key on the graph for notes picked with no group yet makes them one.
        let g = match self.motion_group() {
            Some(g) => Some(g),
            None if !self.ed.selection.is_empty() => self.motion_target(),
            None => None,
        };
        let Some(g) = g else {
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
        self.m_key = Some(index);
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
        self.m_key = Some(at);
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
            self.m_key = None;
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

    /// Where each point's handle is drawn and taken: on the point, or for a point off the
    /// preview (a path may come in from off screen), on the preview's edge towards it, so it can
    /// always be picked up again.
    pub(super) fn handle_spots(&self) -> Vec<(f32, f32)> {
        let [fx, fy, fw, fh] = self.layout.preview;
        let m = HANDLE_INSET.min(fw / 2.0).min(fh / 2.0);
        self.path_handles()
            .into_iter()
            .map(|(hx, hy)| (hx.clamp(fx + m, fx + fw - m), hy.clamp(fy + m, fy + fh - m)))
            .collect()
    }

    fn handle_near(&self, x: f32, y: f32) -> Option<usize> {
        self.handle_spots()
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
        let real = self.path_handles();
        for (i, (hx, hy)) in self.handle_spots().into_iter().enumerate() {
            let lit = live == Some(i) || hover == Some(i);
            let colour = if i == 0 { 0xffffff } else { accent() };
            // Off the preview: two dots point on from the edge to where it really is.
            let (rx, ry) = real[i];
            let (ox, oy) = (rx - hx, ry - hy);
            let off = ox.hypot(oy);
            if off > 0.5 {
                for d in [4.5, 6.5] {
                    let (px, py) = (hx + ox / off * d, hy + oy / off * d);
                    c.rect(px - 0.8, py - 0.8, 1.6, 1.6, rgba(colour, 0.9));
                }
            }
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
            draw_preview_hint(
                c,
                self.layout.preview,
                &[
                    "drag a point: move it (Shift: straight)",
                    "double click: add one",
                    "right click: take it away",
                ],
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
        if self.ed.edit_path(g, false, |path| {
            // A point drawn makes it a path of its own, off the straight lane.
            path.points.insert(at, p);
            path.straight = false;
        }) {
            self.mark_stale();
        }
    }

    /// Moves the point being dragged; with Shift held it lines up with a neighbour (the point
    /// before it, or the one after: the drum for the last), level or plumb, whichever is closer.
    pub(super) fn path_drag(&mut self, x: f32, y: f32, keys: Keys) {
        let Drag::PathPoint { group, point } = self.drag else {
            return;
        };
        let mut p = self.path_point_at(x, y);
        if keys.shift
            && let Some(path) = self.ed.path_of(group)
        {
            p = straightened(p, &path.points, point);
        }
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

/// `p` for point `i` of a path of `points` (playfield units, the drum at 0, 0), moved onto the
/// level or plumb line through a neighbour, the one it is nearest: the point before it, or the one
/// after (the drum, after the last).
fn straightened(p: [f64; 2], points: &[[f64; 2]], i: usize) -> [f64; 2] {
    let before = i.checked_sub(1).and_then(|j| points.get(j));
    let after = points.get(i + 1).copied().unwrap_or([0.0, 0.0]);
    before
        .into_iter()
        .copied()
        .chain([after])
        .flat_map(|n| [[p[0], n[1]], [n[0], p[1]]])
        .min_by(|a, b| {
            let d = |q: &[f64; 2]| (q[0] - p[0]).abs() + (q[1] - p[1]).abs();
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::straightened;

    #[test]
    fn shift_lines_a_path_point_up_with_its_nearest_neighbour() {
        let pts = [[300.0, -100.0], [120.0, -40.0], [50.0, -10.0]];
        // The middle point near level with the one before: it goes level with it.
        assert_eq!(straightened([200.0, -95.0], &pts, 1), [200.0, -100.0]);
        // Near plumb under the one after: it goes plumb.
        assert_eq!(straightened([53.0, -70.0], &pts, 1), [50.0, -70.0]);
        // The last point lines up with the drum.
        assert_eq!(straightened([90.0, 4.0], &pts, 2), [90.0, 0.0]);
        assert_eq!(straightened([-3.0, 80.0], &pts, 2), [0.0, 80.0]);
        // The first has only the one after it.
        assert_eq!(straightened([310.0, -38.0], &pts, 0), [310.0, -40.0]);
    }
}
