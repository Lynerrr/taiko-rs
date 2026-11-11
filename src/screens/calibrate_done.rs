//! The offset calibration's result: what was measured, the offset it suggests, and keys to
//! take it (Enter), leave it (Esc), go again (R) or change the click's tempo (Left/Right).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The result of a calibration run.
pub(crate) struct CalibrateDone {
    pub(crate) estimate: calibrate::Estimate,
    /// Where calibrating was started from, and goes back to.
    pub(crate) back: Box<Scene>,
    /// It was the vsync compensation's (by sight), not the offset's (by ear).
    pub(crate) vsync: bool,
}

impl CalibrateDone {
    /// The result screen of a run, as a scene.
    pub(crate) fn scene(estimate: calibrate::Estimate, back: Box<Scene>, vsync: bool) -> Scene {
        Scene::Other(Box::new(CalibrateDone {
            estimate,
            back,
            vsync,
        }))
    }

    /// Goes back to where calibrating was started from; with `apply`, the suggestion is taken.
    pub(crate) fn leave(self, state: &mut State, apply: bool) {
        state.stop_audio();
        let CalibrateDone {
            estimate,
            back,
            vsync,
        } = self;
        if apply && vsync {
            state.cfg.vsync_tune_ms = estimate.suggested_offset.clamp(-200.0, 200.0);
            state.cfg.save();
            state.toast(tr!("vsync compensation fine tune set to {:+.0} ms", "垂直同期の補正の微調整を{:+.0} msにしました", "точная компенсация vsync: {:+.0} мс"; state.cfg.vsync_tune_ms));
        } else if apply {
            state.cfg.set_shown_offset(estimate.suggested_offset);
            state.cfg.save();
            state.toast(tr!("offset set to {:+.0} ms", "オフセットを{:+.0} msにしました", "смещение: {:+.0} мс"; estimate.suggested_offset));
        }
        state.scene = *back;
    }

    /// Takes the screen apart for `leave`, leaving it empty.
    fn take(&mut self) -> CalibrateDone {
        CalibrateDone {
            estimate: self.estimate,
            back: std::mem::replace(&mut self.back, Box::new(Scene::Select)),
            vsync: self.vsync,
        }
    }
}

impl Screen for CalibrateDone {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "CalibrateDone"
    }

    fn activity(&self) -> &'static str {
        "calibration"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some({
            (
                tr!("Calibrating", "調整中", "Калибровка").into(),
                String::new(),
            )
        })
    }

    fn background_dim(&self) -> Option<f32> {
        Some(0.0)
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn calibrating(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if repeat {
            return;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter => self.take().leave(state, true),
            KeyCode::Escape => self.take().leave(state, false),
            KeyCode::KeyR => {
                // Again, the same kind, going back to the same place in the end.
                let vsync = self.vsync;
                state.scene = *self.take().back;
                state.start_calibration_as(vsync);
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let step = if code == KeyCode::ArrowRight {
                    5.0
                } else {
                    -5.0
                };
                let (lo, hi) = calibrate::BPM_RANGE;
                state.cfg.calibration_bpm = (state.cfg.calibration_bpm + step).clamp(lo, hi);
                state.cfg.save();
            }
            KeyCode::F11 => state.toggle_fullscreen(window),
            KeyCode::F12 => state.screenshot_due = true,
            KeyCode::KeyQ if state.modifiers_ctrl => state.ask_quit(),
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        let current = if self.vsync {
            state.cfg.vsync_tune_ms
        } else {
            state.cfg.shown_offset()
        };
        calibrate::draw_result(
            canvas,
            &self.estimate,
            current,
            state.cfg.calibration_bpm,
            self.vsync,
        );
        None
    }
}
