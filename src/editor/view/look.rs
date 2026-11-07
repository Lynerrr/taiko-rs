//! How the Effects pages' buttons look, by what they are for, so a glance tells a page from a
//! choice from a thing that happens:
//!
//! - page tabs are text, the current one underlined in the accent;
//! - sub-tabs (the shelf's categories, Motion's tabs) are a bar of joined segments;
//! - choices (when, which notes, a path, a pace) are rounded chips, the chosen one in the accent;
//! - actions are solid grey buttons, the one a screen is for solid accent, and the ones that
//!   delete red;
//! - toggles have a box that is ticked;
//! - numbers sit in a sunken field with - and + beside it, and menus have a caret;
//! - ready-made effects are cards with their category's colour down the side.
//!
//! Buttons of the other modes keep the plain look.

use super::clips::FxAct;
use super::keys::KAct;
use super::motion::MAct;
use super::nodes::NAct;
use super::{Btn, Field, View};
use crate::render::{Canvas, accent, rgba};

/// The size of a button's text.
pub(super) const TEXT: f32 = 7.0;
/// Text that doesn't ask to be read first.
pub(super) const SOFT: u32 = 0xb4b4c6;
/// Section headings and their rule.
const HEAD: u32 = 0x7c7c96;
const RULE: u32 = 0x2a2a38;
/// Deleting.
pub(super) const DANGER: u32 = 0xff6b6b;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Look {
    /// The look of the other modes.
    Plain,
    /// A page: text, underlined when it is the one shown.
    Tab,
    /// One of a bar of joined segments: a sub-page or a category.
    Segment,
    /// A choice among others, the chosen one filled.
    Chip,
    /// Does something.
    Action,
    /// The one thing a screen is for.
    Primary,
    /// Deletes or clears.
    Danger,
    /// Goes back or puts something back: just text.
    Ghost,
    /// On or off, with a box ticked when on.
    Toggle,
    /// A number or text to type.
    Field,
    /// Opens a menu.
    Drop,
    /// Steps a value: - and +, < and >.
    Step,
    /// A ready-made effect, with its category's colour down its side.
    Card(u32),
}

/// What `b` looks like.
pub(super) fn look_of(b: Btn) -> Look {
    match b {
        Btn::GPage(_) => Look::Tab,
        Btn::Edit(Field::Fx(_) | Field::Key(_) | Field::FxText) => Look::Field,
        Btn::Dec(Field::Fx(_) | Field::Key(_)) | Btn::Inc(Field::Fx(_) | Field::Key(_)) => {
            Look::Step
        }
        Btn::Fx(a) => fx_look(a),
        Btn::M(a) => motion_look(a),
        Btn::K(a) => match a {
            KAct::Target(_) | KAct::Tab(_) => Look::Segment,
            KAct::Group(_) | KAct::Prop(_) => Look::Chip,
            KAct::NewGroup | KAct::SelectGroup | KAct::KeyHere => Look::Action,
            KAct::DeleteGroup | KAct::DeleteKey => Look::Danger,
            KAct::Ease => Look::Drop,
            KAct::MakeTko => Look::Primary,
        },
        Btn::N(a) => match a {
            NAct::Cat(_) => Look::Segment,
            NAct::Add(_) | NAct::FromSelection | NAct::Clip | NAct::Home | NAct::Fit => {
                Look::Action
            }
            NAct::Param(..) | NAct::Seed(_) => Look::Step,
            NAct::Edit(_) => Look::Field,
            NAct::Open(_) | NAct::AddRecipe(_) | NAct::Ease => Look::Drop,
            NAct::Delete => Look::Danger,
            NAct::NoTarget => Look::Ghost,
            NAct::SetTarget(_) | NAct::PickGroup(_) => Look::Plain,
        },
        _ => Look::Plain,
    }
}

fn fx_look(a: FxAct) -> Look {
    match a {
        FxAct::Tab(_) | FxAct::LayerText(_) | FxAct::EndPose(_) => Look::Segment,
        FxAct::Preset(_) => Look::Card(0x888899),
        FxAct::When(_) | FxAct::Notes(_) | FxAct::Motion(_) | FxAct::CopyNo(_) => Look::Chip,
        FxAct::Place(_)
        | FxAct::OnSelection
        | FxAct::SelectionInGroup(_)
        | FxAct::SelectGroup
        | FxAct::FitKiai
        | FxAct::FitSelection
        | FxAct::Duplicate
        | FxAct::Colour => Look::Action,
        FxAct::Delete => Look::Danger,
        FxAct::Disarm | FxAct::Back => Look::Ghost,
        FxAct::Menu(_) => Look::Drop,
        FxAct::What(_) | FxAct::Picture(_) | FxAct::Group(_) => Look::Step,
        FxAct::Keep | FxAct::Moves | FxAct::Behind => Look::Toggle,
        FxAct::SetPicture(_) | FxAct::SetWhat(_) | FxAct::SetEase(_) => Look::Plain,
    }
}

fn motion_look(a: MAct) -> Look {
    match a {
        MAct::Tab(_) => Look::Segment,
        MAct::UseSelection => Look::Primary,
        MAct::Pick(_)
        | MAct::Path(_)
        | MAct::Pace(_)
        | MAct::Always
        | MAct::Before(_)
        | MAct::Prop(_)
        | MAct::Speed(_)
        | MAct::ShowColour(_)
        | MAct::ShowSize(_)
        | MAct::Reveal(_) => Look::Chip,
        MAct::Show => Look::Toggle,
        MAct::Clear | MAct::ClearScroll => Look::Danger,
        MAct::Groups(_) => Look::Step,
        MAct::Switch(_)
        | MAct::SelectNotes
        | MAct::Copy
        | MAct::Paste
        | MAct::Nudge(_)
        | MAct::Glide
        | MAct::Move(_)
        | MAct::DrumTo(_)
        | MAct::ToggleFake
        | MAct::FakeCopy(_) => Look::Action,
    }
}

/// How wide a button of `look` with `label` is.
pub(super) fn width(look: Look, label: &str) -> f32 {
    let text = Canvas::text_width(TEXT, label);
    match look {
        Look::Tab => text + 10.0,
        Look::Toggle => text + 19.0,
        Look::Drop => text + 20.0,
        Look::Card(_) => text + 15.0,
        Look::Step | Look::Chip => text + 10.0,
        _ => text + 12.0,
    }
}

/// The gap after a button of `look` before one of `next`: none inside a bar of segments.
pub(super) fn gap(look: Look, next: Look) -> f32 {
    match (look, next) {
        (Look::Segment, Look::Segment) => 0.0,
        (Look::Chip, Look::Chip) => 3.0,
        _ => 4.0,
    }
}

/// Mixes `a` toward `b` by `k` (0 is `a`).
fn mix(a: u32, b: u32, k: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 0xff) as f32, ((b >> s) & 0xff) as f32);
        ((x + (y - x) * k).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

impl View {
    /// Draws button `b` as `look` and makes it clickable.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn styled(
        &mut self,
        c: &mut Canvas,
        [x, y, w, h]: [f32; 4],
        label: &str,
        b: Btn,
        on: bool,
        look: Look,
    ) {
        let hover = c.hovered(x, y, w, h);
        let none = rgba(0, 0.0);
        let ty = y + (h - TEXT) / 2.0;
        let centred = |c: &mut Canvas, col: u32| {
            c.text_centered(
                x + w / 2.0,
                ty,
                TEXT,
                rgba(col, 1.0),
                &Canvas::fit(TEXT, w - 4.0, label),
            );
        };
        match look {
            Look::Plain => {
                let (bg, border) = match (on, hover) {
                    (true, _) => (0x4a221b, accent()),
                    (false, true) => (0x34344a, 0x6a6a88),
                    (false, false) => (0x23232f, 0x3a3a4e),
                };
                c.round_rect([x, y], [w, h], 3.0, rgba(bg, 1.0), rgba(border, 1.0), 1.0);
                centred(c, 0xffffff);
            }
            Look::Tab => {
                if hover && !on {
                    c.round_rect([x, y], [w, h], 3.0, rgba(0xffffff, 0.05), none, 0.0);
                }
                centred(c, if on || hover { 0xffffff } else { 0x8c8ca4 });
                if on {
                    c.rect(x + 3.0, y + h - 1.0, w - 6.0, 2.0, rgba(accent(), 1.0));
                }
            }
            Look::Segment => {
                let bg = if on {
                    0x44445c
                } else if hover {
                    0x2c2c3c
                } else {
                    0x1b1b25
                };
                c.round_rect([x, y], [w, h], 0.0, rgba(bg, 1.0), rgba(0x34344a, 1.0), 1.0);
                centred(c, if on { 0xffffff } else { SOFT });
            }
            Look::Chip => {
                let r = h / 2.0;
                if on {
                    c.round_rect(
                        [x, y],
                        [w, h],
                        r,
                        rgba(mix(0x1b1b25, accent(), 0.45), 1.0),
                        rgba(accent(), 1.0),
                        1.0,
                    );
                } else {
                    let bg = if hover { 0x2a2a3a } else { 0x15151d };
                    let edge = if hover { 0x6a6a88 } else { 0x3a3a4e };
                    c.round_rect([x, y], [w, h], r, rgba(bg, 1.0), rgba(edge, 1.0), 1.0);
                }
                centred(c, if on { 0xffffff } else { SOFT });
            }
            Look::Action | Look::Step => {
                let bg = if hover { 0x40405a } else { 0x2f2f40 };
                c.round_rect([x, y], [w, h], 2.0, rgba(bg, 1.0), rgba(0x4a4a62, 1.0), 1.0);
                // A lit top edge: it stands up off the panel.
                c.rect(x + 2.0, y + 1.0, w - 4.0, 1.0, rgba(0xffffff, 0.06));
                centred(c, 0xffffff);
            }
            Look::Primary => {
                let bg = if hover {
                    mix(accent(), 0xffffff, 0.15)
                } else {
                    accent()
                };
                c.round_rect([x, y], [w, h], 2.0, rgba(bg, 1.0), rgba(bg, 1.0), 1.0);
                centred(c, 0xffffff);
            }
            Look::Danger => {
                let (bg, edge) = if hover {
                    (0x4a1e22, DANGER)
                } else {
                    (0x15151d, 0x5a2a30)
                };
                c.round_rect([x, y], [w, h], 2.0, rgba(bg, 1.0), rgba(edge, 1.0), 1.0);
                centred(c, DANGER);
            }
            Look::Ghost => {
                if hover {
                    c.round_rect([x, y], [w, h], 2.0, rgba(0xffffff, 0.06), none, 0.0);
                }
                centred(c, if hover { 0xffffff } else { 0x9a9ab0 });
            }
            Look::Toggle => {
                if hover {
                    c.round_rect([x, y], [w, h], 2.0, rgba(0xffffff, 0.05), none, 0.0);
                }
                let s = 8.0;
                let (bx, by) = (x + 3.0, y + (h - s) / 2.0);
                if on {
                    c.round_rect(
                        [bx, by],
                        [s, s],
                        1.5,
                        rgba(accent(), 1.0),
                        rgba(accent(), 1.0),
                        1.0,
                    );
                    c.rect(bx + 2.5, by + 2.5, s - 5.0, s - 5.0, rgba(0xffffff, 1.0));
                } else {
                    c.round_rect(
                        [bx, by],
                        [s, s],
                        1.5,
                        rgba(0x0e0e14, 1.0),
                        rgba(0x6a6a88, 1.0),
                        1.0,
                    );
                }
                c.text(
                    bx + s + 4.0,
                    ty,
                    TEXT,
                    rgba(if on || hover { 0xffffff } else { SOFT }, 1.0),
                    &Canvas::fit(TEXT, w - s - 9.0, label),
                );
            }
            Look::Field => {
                let edge = if on {
                    accent()
                } else if hover {
                    0x6a6a88
                } else {
                    0x2c2c3c
                };
                c.round_rect(
                    [x, y],
                    [w, h],
                    2.0,
                    rgba(0x0b0b10, 1.0),
                    rgba(edge, 1.0),
                    1.0,
                );
                centred(c, 0xffffff);
            }
            Look::Drop => {
                let edge = if on {
                    accent()
                } else if hover {
                    0x7a7a98
                } else {
                    0x45455c
                };
                c.round_rect(
                    [x, y],
                    [w, h],
                    2.0,
                    rgba(0x101016, 1.0),
                    rgba(edge, 1.0),
                    1.0,
                );
                c.text(
                    x + 6.0,
                    ty,
                    TEXT,
                    rgba(0xffffff, 1.0),
                    &Canvas::fit(TEXT, w - 18.0, label),
                );
                // The caret: a small V.
                let (cx, cy) = (x + w - 8.0, y + h / 2.0);
                for i in 0..3 {
                    let k = i as f32;
                    c.rect(cx - 2.5 + k, cy - 1.5 + k, 1.0, 1.0, rgba(SOFT, 1.0));
                    c.rect(cx + 1.5 - k, cy - 1.5 + k, 1.0, 1.0, rgba(SOFT, 1.0));
                }
            }
            Look::Card(col) => {
                let (bg, edge) = if on {
                    (mix(0x1b1b25, accent(), 0.4), accent())
                } else if hover {
                    (0x2c2c3c, 0x5a5a76)
                } else {
                    (0x1f1f2a, 0x30303f)
                };
                c.round_rect([x, y], [w, h], 2.0, rgba(bg, 1.0), rgba(edge, 1.0), 1.0);
                c.rect(x + 1.0, y + 1.0, 2.5, h - 2.0, rgba(col, 1.0));
                c.text(
                    x + 8.0,
                    ty,
                    TEXT,
                    rgba(0xffffff, 1.0),
                    &Canvas::fit(TEXT, w - 10.0, label),
                );
            }
        }
        self.buttons.push(([x, y, w, h], b));
        if let Btn::C(cmd) = b {
            self.tips.push(([x, y, w, h], cmd.tip()));
        }
    }

    /// A section's heading: small capitals and a rule to the right. Returns the y under it.
    pub(super) fn heading(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, text: &str) -> f32 {
        self.heading_note(c, x, y, w, text, "")
    }

    /// A heading with a note after it in plain letters. Returns the y under it.
    pub(super) fn heading_note(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        text: &str,
        note: &str,
    ) -> f32 {
        let size = 5.5;
        let text = text.to_uppercase();
        let mut tw = Canvas::text_width(size, &text);
        c.text(x, y, size, rgba(HEAD, 1.0), &Canvas::fit(size, w, &text));
        if !note.is_empty() && tw + 20.0 < w {
            let note = Canvas::fit(size, w - tw - 8.0, note);
            c.text(x + tw + 6.0, y, size, rgba(SOFT, 1.0), &note);
            tw += Canvas::text_width(size, &note) + 6.0;
        }
        if tw + 6.0 < w {
            c.rect(
                x + tw + 5.0,
                y + size / 2.0,
                w - tw - 5.0,
                1.0,
                rgba(RULE, 1.0),
            );
        }
        y + 9.0
    }
}

#[cfg(test)]
mod tests {
    use super::super::cmd::Cmd;
    use super::*;

    #[test]
    fn a_button_looks_like_what_it_does() {
        assert_eq!(look_of(Btn::GPage(4)), Look::Tab);
        assert_eq!(look_of(Btn::Fx(FxAct::Tab(0))), Look::Segment);
        assert_eq!(look_of(Btn::Fx(FxAct::When(0))), Look::Chip);
        assert_eq!(look_of(Btn::Fx(FxAct::Delete)), Look::Danger);
        assert_eq!(look_of(Btn::M(MAct::Clear)), Look::Danger);
        assert_eq!(look_of(Btn::Fx(FxAct::Keep)), Look::Toggle);
        assert_eq!(look_of(Btn::Fx(FxAct::Menu(1))), Look::Drop);
        assert_eq!(look_of(Btn::M(MAct::UseSelection)), Look::Primary);
        // The other modes keep theirs.
        assert_eq!(look_of(Btn::Resnap), Look::Plain);
        assert_eq!(look_of(Btn::C(Cmd::Save)), Look::Plain);
        assert_eq!(look_of(Btn::Dec(Field::Bpm)), Look::Plain);
    }

    #[test]
    fn segments_join_and_toggles_make_room_for_their_box() {
        assert_eq!(gap(Look::Segment, Look::Segment), 0.0);
        assert!(gap(Look::Action, Look::Danger) > 0.0);
        assert!(width(Look::Toggle, "keep after") > width(Look::Action, "keep after"));
    }
}
