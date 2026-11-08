//! The snap popover: click Snap in the transport to pick a snap, switch between snap presets and
//! make or delete your own. The wheel and the snap keys then step only through the active
//! preset's divisions (`editor::snaps`).

use super::chrome::DIM;
use super::look::{self, Look};
use super::{Btn, View};
use crate::editor::snaps::{self, SnapPreset};
use crate::editor::timing::SNAP_DIVISORS;
use crate::render::{Canvas, rgba};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SnapAct {
    /// The transport's Snap: opens or closes the popover.
    Open,
    /// One division, picked directly.
    Div(u32),
    /// The built-in preset with every division.
    All,
    /// One of the mapper's presets.
    Preset(u8),
    /// Starts a new preset.
    New,
    /// Ticks or unticks a division of the preset being made.
    Draft(u32),
    /// Saves the preset being made.
    Create,
    /// Drops the preset being made.
    Cancel,
    /// Deletes the preset in use.
    Delete,
}

/// What a snap button looks like.
pub(super) fn look(a: SnapAct) -> Look {
    match a {
        SnapAct::Open => Look::Plain,
        SnapAct::Div(_) | SnapAct::All | SnapAct::Preset(_) => Look::Chip,
        SnapAct::Draft(_) => Look::Toggle,
        SnapAct::New => Look::Action,
        SnapAct::Create => Look::Primary,
        SnapAct::Cancel => Look::Ghost,
        SnapAct::Delete => Look::Danger,
    }
}

/// The open popover.
#[derive(Clone, Debug)]
pub struct Pop {
    /// Where it was last drawn (clicks on it don't close it).
    pub rect: [f32; 4],
    /// The divisions ticked for a new preset, while one is being made.
    pub draft: Option<Vec<u32>>,
    /// Delete was pressed once: a second press deletes the preset, anything else keeps it.
    pub deleting: bool,
}

const WIDTH: f32 = 300.0;
const PAD: f32 = 8.0;

type Items = Vec<(String, Btn, bool)>;

/// The height `items` take wrapped at `w` (what `View::wrap_row` advances).
fn rows_h(items: &Items, w: f32) -> f32 {
    let (mut x, mut rows) = (0.0, 1.0);
    for (k, (label, b, _)) in items.iter().enumerate() {
        let l = look::look_of(*b);
        let bw = look::width(l, label);
        if x > 0.0 && x + bw > w {
            x = 0.0;
            rows += 1.0;
        }
        let next = items.get(k + 1).map_or(Look::Plain, |n| look::look_of(n.1));
        x += bw + look::gap(l, next);
    }
    rows * 19.0
}

impl View {
    /// The divisions the wheel and the snap keys step through: the preset in use, else all.
    pub(super) fn snap_list(&self) -> Vec<u32> {
        match self.snap_active.and_then(|i| self.snap_presets.get(i)) {
            Some(p) => p.divisors.clone(),
            None => SNAP_DIVISORS.to_vec(),
        }
    }

    /// Switches to the preset `at` (`None`: all) and moves the snap to its nearest division.
    pub fn use_snap_preset(&mut self, at: Option<usize>) {
        self.snap_active = at.filter(|i| *i < self.snap_presets.len());
        self.ed.snap = snaps::nearest(&self.snap_list(), self.ed.snap);
        self.snap_dirty = true;
    }

    /// Loads the presets and the one in use (by name) from the config.
    pub fn set_snap_presets(&mut self, lines: &[String], active: &str) {
        self.snap_presets = snaps::parse_all(lines);
        self.snap_active = self.snap_presets.iter().position(|p| p.name == active);
        self.ed.snap = snaps::nearest(&self.snap_list(), self.ed.snap);
    }

    /// The presets as config values, and the name of the one in use ("" for all).
    pub fn snap_config(&self) -> (Vec<String>, String) {
        let name = self.snap_active.and_then(|i| self.snap_presets.get(i));
        (
            self.snap_presets.iter().map(SnapPreset::to_line).collect(),
            name.map(|p| p.name.clone()).unwrap_or_default(),
        )
    }

    pub(super) fn press_snap(&mut self, act: SnapAct) {
        // Delete asks first: anything but a second Delete keeps the preset.
        let armed = match &mut self.snap_pop {
            Some(p) => std::mem::take(&mut p.deleting),
            None => false,
        };
        match act {
            SnapAct::Open => {
                self.snap_pop = match self.snap_pop {
                    Some(_) => None,
                    None => Some(Pop {
                        rect: [0.0; 4],
                        draft: None,
                        deleting: false,
                    }),
                };
            }
            SnapAct::Div(d) => self.ed.snap = d,
            SnapAct::All => self.use_snap_preset(None),
            SnapAct::Preset(i) => self.use_snap_preset(Some(i as usize)),
            SnapAct::New => {
                if let Some(p) = &mut self.snap_pop {
                    p.draft = Some(Vec::new());
                }
            }
            SnapAct::Draft(d) => {
                if let Some(list) = self.snap_pop.as_mut().and_then(|p| p.draft.as_mut()) {
                    match list.iter().position(|x| *x == d) {
                        Some(at) => {
                            list.remove(at);
                        }
                        None => list.push(d),
                    }
                }
            }
            SnapAct::Cancel => {
                if let Some(p) = &mut self.snap_pop {
                    p.draft = None;
                }
            }
            SnapAct::Create => {
                let list = self.snap_pop.as_ref().and_then(|p| p.draft.clone());
                let Some(preset) = list.and_then(|l| SnapPreset::new("", &l)) else {
                    return;
                };
                let at = match self.snap_presets.iter().position(|p| p.name == preset.name) {
                    Some(i) => i,
                    None if self.snap_presets.len() >= snaps::MAX_PRESETS => {
                        self.status = "too many snap presets: delete one first".into();
                        return;
                    }
                    None => {
                        self.snap_presets.push(preset);
                        self.snap_presets.len() - 1
                    }
                };
                if let Some(p) = &mut self.snap_pop {
                    p.draft = None;
                }
                self.use_snap_preset(Some(at));
            }
            SnapAct::Delete => {
                if let Some(i) = self.snap_active.filter(|i| *i < self.snap_presets.len()) {
                    if !armed {
                        if let Some(p) = &mut self.snap_pop {
                            p.deleting = true;
                        }
                        self.status = format!(
                            "delete the snap preset {}? Press Delete again to delete it",
                            self.snap_presets[i].name
                        );
                        return;
                    }
                    let gone = self.snap_presets.remove(i);
                    self.status = format!("deleted the snap preset {}", gone.name);
                    self.use_snap_preset(None);
                }
            }
        }
    }

    /// The open popover, drawn over everything else.
    pub(super) fn draw_snap_pop(&mut self, c: &mut Canvas) {
        let Some(pop) = self.snap_pop.clone() else {
            return;
        };
        let w = WIDTH - 2.0 * PAD;
        let mut chips: Items = Vec::new();
        let (mut upper, mut lower): (Items, Items) = (Vec::new(), Vec::new());
        let mut custom = None;
        if let Some(draft) = &pop.draft {
            upper = SNAP_DIVISORS
                .iter()
                .map(|d| {
                    (
                        format!("1/{d}"),
                        Btn::Snap(SnapAct::Draft(*d)),
                        draft.contains(d),
                    )
                })
                .collect();
        } else {
            let active = self.snap_active.filter(|i| *i < self.snap_presets.len());
            chips = self
                .snap_list()
                .iter()
                .map(|d| {
                    (
                        format!("1/{d}"),
                        Btn::Snap(SnapAct::Div(*d)),
                        *d == self.ed.snap,
                    )
                })
                .collect();
            upper.push((snaps::ALL.into(), Btn::Snap(SnapAct::All), active.is_none()));
            for (i, p) in self.snap_presets.iter().enumerate() {
                let on = active == Some(i);
                upper.push((p.name.clone(), Btn::Snap(SnapAct::Preset(i as u8)), on));
            }
            lower.push(("New preset".into(), Btn::Snap(SnapAct::New), false));
            if let Some(i) = active {
                custom = Some(self.snap_presets[i].name.clone());
                let name = &self.snap_presets[i].name;
                if pop.deleting {
                    lower.push((
                        format!("Yes, delete {name}"),
                        Btn::Snap(SnapAct::Delete),
                        false,
                    ));
                    lower.push(("Keep it".into(), Btn::Snap(SnapAct::Cancel), false));
                } else {
                    lower.push((format!("Delete {name}"), Btn::Snap(SnapAct::Delete), false));
                }
            }
        }
        // The height first, so the panel can be drawn under what goes on it.
        let head = 10.0;
        let h = if pop.draft.is_some() {
            PAD + head + rows_h(&upper, w) + 3.0 + 11.0 + 20.0 + PAD - 2.0
        } else {
            PAD + head + rows_h(&chips, w) + 3.0 + head + rows_h(&upper, w) + 3.0 + 20.0 + PAD - 2.0
        };
        let [ax, ay, _, ah] = self.snap_anchor;
        let x = ax.min(c.width - WIDTH - 4.0).max(4.0);
        let y = ay + ah + 2.0;
        c.round_rect(
            [x, y],
            [WIDTH, h],
            3.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        if let Some(p) = &mut self.snap_pop {
            p.rect = [x, y, WIDTH, h];
        }
        let (ix, mut iy) = (x + PAD, y + PAD);
        if let Some(draft) = &pop.draft {
            iy = self.heading_note(c, ix, iy, w, "New preset", "tick its snaps") + 1.0;
            iy = self.wrap_row(c, ix, iy, w, &upper) + 3.0;
            let text = if draft.is_empty() {
                "tick at least one snap".to_string()
            } else {
                let mut d = draft.clone();
                d.sort_unstable();
                format!("Name: {}", snaps::auto_name(&d))
            };
            c.text(
                ix,
                iy + 1.0,
                6.5,
                rgba(DIM, 1.0),
                &Canvas::fit(6.5, w, &text),
            );
            iy += 13.0;
            let mut bx = ix;
            if !draft.is_empty() {
                let bw = look::width(Look::Primary, "Create");
                self.styled(
                    c,
                    [bx, iy, bw, 16.0],
                    "Create",
                    Btn::Snap(SnapAct::Create),
                    false,
                    Look::Primary,
                );
                bx += bw + 4.0;
            }
            let bw = look::width(Look::Ghost, "Cancel");
            self.styled(
                c,
                [bx, iy, bw, 16.0],
                "Cancel",
                Btn::Snap(SnapAct::Cancel),
                false,
                Look::Ghost,
            );
            return;
        }
        let note = match &custom {
            Some(n) => format!("in {n}"),
            None => "in All".into(),
        };
        iy = self.heading_note(c, ix, iy, w, "Snap", &note) + 1.0;
        iy = self.wrap_row(c, ix, iy, w, &chips) + 3.0;
        iy = self.heading_note(
            c,
            ix,
            iy,
            w,
            "Presets",
            "the wheel steps through the one in use",
        ) + 1.0;
        iy = self.wrap_row(c, ix, iy, w, &upper) + 3.0;
        self.row(c, ix, iy, &lower);
    }
}
