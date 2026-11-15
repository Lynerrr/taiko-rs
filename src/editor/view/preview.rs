//! The preview point: where song select plays the song from (`PreviewTime`). Set at the
//! playhead from the Timing menu or Song setup, shown as a green P on the timeline.

use super::chrome::DIM;
use super::{Btn, Cmd, PREVIEW_COL, View, fmt_time};
use crate::hit::Canvas;
use crate::render::rgba;

impl View {
    /// Why the preview point can't change now, if it can't: a mapping session's ops carry Song
    /// setup's other keys only (`mapping::KEYS`), which older servers and games hold to.
    fn preview_locked(&self) -> Option<&'static str> {
        self.collab
            .is_some()
            .then_some("the preview point can't change in a mapping session yet")
    }

    /// Timing > Set preview point here.
    pub(super) fn set_preview_here(&mut self, t: f64) {
        if let Some(why) = self.preview_locked() {
            self.status = why.into();
            return;
        }
        self.status = if self.ed.set_preview(Some(t)) {
            format!(
                "preview point at {}: song select plays the song from here",
                fmt_time(t.max(0.0).round())
            )
        } else {
            "the preview point is already here".into()
        };
    }

    /// Timing > Clear preview point.
    pub(super) fn clear_preview(&mut self) {
        if let Some(why) = self.preview_locked() {
            self.status = why.into();
            return;
        }
        self.status = if self.ed.set_preview(None) {
            "no preview point: song select picks one".into()
        } else {
            "no preview point set".into()
        };
    }

    /// Song setup's preview point row at `y`: the time, Set to playhead, Go to and Clear.
    pub(super) fn draw_preview_row(&mut self, c: &mut Canvas, x0: f32, y: f32, inner: f32) {
        let small = 6.5;
        c.text(
            x0,
            y,
            small,
            rgba(DIM, 1.0),
            "Preview point (where song select plays the song from)",
        );
        let by = y + 9.0;
        let locked = self.preview_locked().is_some();
        let at = self.ed.preview_time();
        let mut buttons: Vec<(&str, Cmd)> = Vec::new();
        if !locked {
            buttons.push(("Set to playhead", Cmd::PreviewHere));
        }
        if at.is_some() {
            buttons.push(("Go to", Cmd::PreviewJump));
            if !locked {
                buttons.push(("Clear", Cmd::PreviewClear));
            }
        }
        let widths: Vec<f32> = buttons
            .iter()
            .map(|(l, _)| Canvas::text_width(7.0, l) + 14.0)
            .collect();
        let room: f32 = widths.iter().map(|w| w + 6.0).sum();
        let bw = inner - room;
        c.round_rect(
            [x0, by],
            [bw, 16.0],
            3.0,
            rgba(0x101018, 1.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        let (text, colour) = match at {
            Some(t) => (fmt_time(t), PREVIEW_COL),
            None => (
                "not set: song select picks a point itself".to_string(),
                0x60607a,
            ),
        };
        let text = if locked {
            format!("{text} (can't change in a mapping session)")
        } else {
            text
        };
        c.text(
            x0 + 5.0,
            by + 4.5,
            7.0,
            rgba(colour, 1.0),
            &Canvas::fit(7.0, bw - 10.0, &text),
        );
        let mut bx = x0 + bw + 6.0;
        for ((label, cmd), w) in buttons.into_iter().zip(widths) {
            self.button(c, [bx, by, w, 16.0], label, Btn::C(cmd), false);
            bx += w + 6.0;
        }
    }
}
