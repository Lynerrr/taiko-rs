//! The interface editor: where each part of the play's interface goes, its size, colour and
//! presets, on a made-up play (`State::hud_sample`), opened from settings.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The interface editor up.
pub(crate) struct HudEdit {
    /// Where it was opened from, and goes back to.
    back: Box<Scene>,
    pub(crate) ed: hud_edit::Editor,
}

impl State {
    pub(crate) fn open_hud_editor(&mut self) {
        let back = std::mem::replace(&mut self.scene, SongSelect::scene());
        let ed = hud_edit::Editor::new(&self.cfg);
        self.scene = Scene::Other(Box::new(HudEdit {
            back: Box::new(back),
            ed,
        }));
    }

    pub(crate) fn close_hud_editor(&mut self) {
        let back = self
            .scene
            .screen_mut::<HudEdit>()
            .map(|h| std::mem::replace(&mut h.back, Box::new(SongSelect::scene())));
        self.cfg.save();
        self.hud_sample = None;
        self.scene = back.map_or(SongSelect::scene(), |back| *back);
    }

    pub(crate) fn hud_outcome(&mut self, outcome: hud_edit::Outcome) {
        match outcome {
            hud_edit::Outcome::Nothing => {}
            hud_edit::Outcome::Toast(text) => self.toast(text),
            hud_edit::Outcome::Close => self.close_hud_editor(),
        }
    }
}

impl Screen for HudEdit {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "HudEdit"
    }

    fn activity(&self) -> &'static str {
        "interface editor"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        Some({
            (
                tr!("Tweaking settings", "設定を調整中", "Копается в настройках").into(),
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

    fn drags(&self) -> bool {
        true
    }

    fn area_editable(&self) -> bool {
        true
    }

    fn wheel_scales_area(&self) -> bool {
        true
    }

    fn covers_social_tab(&self) -> bool {
        true
    }

    fn typing(&self) -> bool {
        self.ed.naming()
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key { code, text, .. } = key;
        let mods = (state.modifiers_shift, state.modifiers_ctrl);
        match self.ed.key(code, text, mods, &mut state.cfg) {
            hud_edit::Outcome::Nothing => {}
            hud_edit::Outcome::Toast(text) => state.toast(text),
            // `State::close_hud_editor`, from in here.
            hud_edit::Outcome::Close => {
                state.cfg.save();
                state.hud_sample = None;
                state.scene = *std::mem::replace(&mut self.back, Box::new(SongSelect::scene()));
            }
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        draw_game_area(
            canvas,
            &state.cfg,
            &mut state.frame_clip,
            &state.grip,
            &state.hover,
            false,
            game::draw_preview,
        );
        let sample = state
            .hud_sample
            .get_or_insert_with(|| Box::new(game::Play::hud_sample()));
        sample.view.hud = state.cfg.hud;
        sample.view.hud_area = state.cfg.playfield_area;
        sample.view.hidden = state.cfg.hud.off;
        sample.draw_stand_ins(canvas);
        sample.draw_parts(canvas);
        layout::draw_frame(canvas, state.cfg.playfield_area, state.grip.is_some());
        self.ed.draw(canvas, &state.cfg, sample);
        layout::draw_guides(canvas, &self.ed.guides);
        None
    }
}
