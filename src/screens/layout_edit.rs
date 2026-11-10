//! The playfield area's editor: drag, resize or scale where the play goes on the screen,
//! opened from settings.

use super::screen::Screen;
use crate::*;

/// The playfield area's editor up.
pub(crate) struct LayoutEdit {
    /// Where it was opened from, and goes back to.
    back: Box<Scene>,
}

impl State {
    pub(crate) fn open_layout_editor(&mut self) {
        let back = std::mem::replace(&mut self.scene, Scene::Select);
        self.scene = Scene::Other(Box::new(LayoutEdit {
            back: Box::new(back),
        }));
    }
}

impl Screen for LayoutEdit {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "LayoutEdit"
    }

    fn activity(&self) -> &'static str {
        "layout editor"
    }

    fn presence(&self) -> (String, String) {
        (
            tr!("Tweaking settings", "設定を調整中", "Копается в настройках").into(),
            String::new(),
        )
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

    fn key(
        &mut self,
        state: &mut State,
        code: KeyCode,
        _repeat: bool,
        _text: Option<&str>,
        _window: Option<&Window>,
    ) {
        let a = state.cfg.playfield_area;
        let step = 0.01;
        // Arrows move; Shift+arrows resize from the bottom-right, like a window.
        let resize = |dx: f32, dy: f32| {
            layout::drag(
                a,
                Grip::Resize {
                    left: false,
                    right: true,
                    top: false,
                    bottom: true,
                },
                dx,
                dy,
            )
        };
        let moved = |dx: f32, dy: f32| layout::drag(a, Grip::Move, dx, dy);
        let shift = state.modifiers_shift;
        let next = match code {
            KeyCode::ArrowLeft if shift => resize(-step, 0.0),
            KeyCode::ArrowRight if shift => resize(step, 0.0),
            KeyCode::ArrowUp if shift => resize(0.0, -step),
            KeyCode::ArrowDown if shift => resize(0.0, step),
            KeyCode::ArrowLeft => moved(-step, 0.0),
            KeyCode::ArrowRight => moved(step, 0.0),
            KeyCode::ArrowUp => moved(0.0, -step),
            KeyCode::ArrowDown => moved(0.0, step),
            KeyCode::KeyR => Area::FULL,
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                state.cycle_preset(if code == KeyCode::BracketRight { 1 } else { -1 });
                return;
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape => {
                state.cfg.save();
                state.scene = *std::mem::replace(&mut self.back, Box::new(Scene::Select));
                return;
            }
            _ => return,
        };
        state.cfg.playfield_area = next;
        state.cfg.save();
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas) -> Option<Scene> {
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
        sample.draw_parts(canvas);
        let area = state.cfg.playfield_area;
        layout::draw_frame(canvas, area, true);
        let w = canvas.width;
        let bw = (w - 32.0).min(660.0);
        canvas.round_rect(
            [w / 2.0 - bw / 2.0, 440.0],
            [bw, 34.0],
            8.0,
            rgba(0x000000, 0.75),
            rgba(accent(), 0.6),
            1.0,
        );
        let room = bw - 16.0;
        canvas.text_centered(
            w / 2.0,
            446.0,
            8.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(8.0, room, &format!("Playfield area: {}", area.describe())),
        );
        let help = if mobile::active() {
            "Back when done"
        } else {
            "Drag: move (snaps; Shift = free), edges: resize, wheel: scale.  [ ] presets, arrows move, Shift+arrows resize, R full, Enter done"
        };
        let size = Canvas::fit_size(7.0, 5.5, room, help);
        canvas.text_centered(
            w / 2.0,
            460.0 + (7.0 - size) / 2.0,
            size,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(size, room, help),
        );
        None
    }
}
