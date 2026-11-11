//! A calibration run: a click track (or, for the vsync compensation, silent notes hit by
//! sight) whose hits measure the offset. Its game is the one in `State::play`.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// A calibration run up.
pub(crate) struct Calibration {
    /// Where calibrating was started from, and goes back to.
    back: Box<Scene>,
    /// It is the vsync compensation's (by sight), not the offset's (by ear).
    vsync: bool,
}

impl Calibration {
    pub(crate) fn scene(back: Box<Scene>, vsync: bool) -> Scene {
        Scene::Other(Box::new(Calibration { back, vsync }))
    }

    /// Takes where to go back to, for leaving.
    pub(crate) fn take_back(&mut self) -> Box<Scene> {
        std::mem::replace(&mut self.back, Box::new(Scene::Select))
    }
}

impl State {
    /// Offset calibration, or with `vsync` the vsync compensation's: the same notes, in vsync and
    /// with no sound, hit by sight.
    pub(crate) fn start_calibration_as(&mut self, vsync: bool) {
        if self.behind_settings().is_some_and(|prev| prev.is_play()) {
            self.toast(
                tr!(
                    "quit the song first to calibrate",
                    "調整するには先に曲をやめてください",
                    "сначала выйдите из песни"
                )
                .into(),
            );
            return;
        }
        // Retrying from the result screen, it has put back where calibration was started from.
        let back = std::mem::replace(&mut self.scene, Scene::Select);
        let beat_ms = 60000.0 / self.cfg.calibration_bpm;
        let beats = calibrate::total_beats(beat_ms);
        let map = beatmap::calibration_map(beat_ms, calibrate::COUNT_IN_BEATS, beats);
        let rate = self.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
        let mut track = audio::metronome(rate, beat_ms, beats, 2000.0);
        if vsync {
            // Silence the length of the click track: only the notes to go by.
            track.samples.iter_mut().for_each(|s| *s = 0.0);
        }
        let track = Arc::new(track);
        let mut game = Play::new(
            Game::new(map, track, sound(&self.audio)).with_settings(&crate::game::settings()),
        );
        // Your own hitsounds would let you sync to yourself instead of to the click.
        game.set_mute_hits(true);
        game.view.show_hp = false;
        self.play = Some(Box::new(game));
        self.scene = Calibration::scene(Box::new(back), vsync);
    }

    /// Ends the calibration run up with the hits so far.
    pub(crate) fn finish_calibration(&mut self) {
        let Some(run) = self.scene.screen_mut::<Calibration>() else {
            return;
        };
        let (back, vsync) = (run.take_back(), run.vsync);
        self.finish_calibration_with(back, vsync);
    }

    /// Ends a calibration run (started from `back`) with the hits so far: its result screen,
    /// or back with a word when there were too few.
    fn finish_calibration_with(&mut self, back: Box<Scene>, vsync: bool) {
        self.scene = Scene::Select;
        let Some(game) = self.play.take() else {
            return;
        };
        self.stop_audio();
        // By sight in vsync, hitting late means the frames reach the screen later than assumed.
        let current = if vsync {
            self.cfg.vsync_tune_ms
        } else {
            self.cfg.offset_ms
        };
        let mut found = calibrate::estimate(&game.stats().errors, current);
        if let (Some(e), false) = (&mut found, vsync) {
            // The suggestion is in the global offset's own terms (osu!'s). Hitting late by e is
            // the clock's offset up by e, which is the osu! one down by e.
            e.suggested_offset = (self.cfg.shown_offset() - e.error_ms).round();
        }
        if let Some(e) = &found {
            audiolog::line(&format!(
                "calibration vsync={vsync} hits={} error={:.1} margin={:.1} ur={:.0} suggested={} current={current}",
                e.hits, e.error_ms, e.margin_ms, e.ur, e.suggested_offset
            ));
        }
        match found {
            Some(estimate) if estimate.hits >= 4 => {
                self.scene = screens::calibrate_done::CalibrateDone::scene(estimate, back, vsync)
            }
            _ => {
                self.scene = *back;
                self.toast(
                    tr!(
                        "calibration needs more hits",
                        "調整にはもっと叩く必要があります",
                        "для калибровки нужно больше ударов"
                    )
                    .into(),
                );
            }
        }
    }
}

impl Screen for Calibration {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Calibrate"
    }

    fn activity(&self) -> &'static str {
        "calibration"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some((
            tr!("Calibrating", "調整中", "Калибровка").into(),
            String::new(),
        ))
    }

    fn background_dim(&self) -> Option<f32> {
        Some(0.0)
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn in_song(&self) -> bool {
        true
    }

    fn holds_game(&self) -> bool {
        true
    }

    fn calibration(&self) -> Option<bool> {
        Some(self.vsync)
    }

    fn area_editable(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            at,
            window,
            ..
        } = key;
        let Some(game) = state.play.as_deref_mut() else {
            return;
        };
        if let Some(k) = state.cfg.keys.iter().position(|&k| k == code) {
            if !repeat {
                game.press(
                    calibrate::centre_key(k),
                    at,
                    sound(&state.audio),
                    state.play_offset,
                );
            }
            return;
        }
        if repeat {
            return;
        }
        match code {
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let back = self.take_back();
                state.finish_calibration_with(back, self.vsync);
            }
            KeyCode::Escape => {
                let back = self.take_back();
                state.stop_audio();
                state.scene = *back;
            }
            KeyCode::F11 => state.toggle_fullscreen(window),
            KeyCode::F12 => state.screenshot_due = true,
            KeyCode::KeyQ if state.modifiers_ctrl => state.ask_quit(),
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let game = state.play.as_deref_mut()?;
        if game.update(frame.now, sound(&state.audio), state.play_offset) {
            state.calibration_over = true;
        }
        if state.auto {
            game.set_autoplay_style(state.cfg.autoplay);
            game.autoplay(sound(&state.audio));
        }
        if self.vsync {
            game.set_visual_offset(frame.visual_offset);
            draw_game_area(
                canvas,
                &state.cfg,
                &mut state.frame_clip,
                &state.grip,
                &state.hover,
                state.bg_brightness > 0.0,
                |c| {
                    game.draw_playfield(c, state.cfg.scroll_speed);
                },
            );
            game.draw_hud(canvas);
            calibrate::draw_by_sight(canvas, game, state.cfg.vsync_tune_ms);
        } else {
            calibrate::draw_by_ear(canvas, game);
        }

        None
    }
}
