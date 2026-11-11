//! Song select's preview made big: the map played by autoplay over the whole screen (in the
//! playfield area, which can be moved and resized here), with its timeline to pick a start
//! point. Opened from song select or from the multiplayer lobby.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The fullscreen preview up.
pub(crate) struct Preview {
    /// Song select or the multiplayer lobby, which it goes back to.
    back: Box<Scene>,
}

impl Preview {
    /// Back to where it was opened from.
    fn close(&mut self, state: &mut State) {
        state.scene = *std::mem::replace(&mut self.back, Box::new(SongSelect::scene()));
    }
}

impl State {
    pub(crate) fn open_preview(&mut self) {
        if self.preview_target().is_none() {
            return;
        }
        let back = std::mem::replace(&mut self.scene, SongSelect::scene());
        self.scene = Scene::Other(Box::new(Preview {
            back: Box::new(back),
        }));
    }

    /// Back from the preview as Esc goes (the screen's own `close`), for the tests.
    #[cfg(test)]
    pub(crate) fn close_preview(&mut self) {
        let back = self
            .scene
            .screen_mut::<Preview>()
            .map(|p| std::mem::replace(&mut p.back, Box::new(SongSelect::scene())));
        self.scene = back.map_or(SongSelect::scene(), |back| *back);
    }

    /// Host, in the lobby preview: the picked start point becomes the match's.
    pub(crate) fn set_match_start(&mut self) {
        let Some(s) = &mut self.mp else { return };
        if !s.is_host() || s.in_match {
            return;
        }
        let (Some(i), Some(mut m)) = (s.map_entry, s.map.clone()) else {
            return;
        };
        m.from = self.preview.marker(&self.library[i]);
        let text = match m.from {
            Some(t) => format!("the match starts at {}", fmt_time(t)),
            None => "the match starts at the beginning".to_string(),
        };
        s.client.send(net::ClientMsg::SelectMap(m));
        self.toast(text);
    }

    /// The preview's button: play from the start point (or the match's start point).
    /// `over_lobby`: the preview is the lobby's.
    pub(crate) fn preview_button(&mut self, over_lobby: bool) {
        if over_lobby {
            self.set_match_start();
        } else if self.picking_lobby_map {
            if let Some(i) = self.select.selected() {
                self.pick_lobby_map(i);
            }
        } else {
            self.play_from_marker();
        }
    }

    /// Plays the selected map from the start point picked on its timeline.
    pub(crate) fn play_from_marker(&mut self) {
        let Some(i) = self.select.selected() else {
            return;
        };
        if let Some(t) = self.preview.marker(&self.library[i]) {
            self.start_loading_from(i, Some(t));
        }
    }
}

impl Screen for Preview {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Preview"
    }

    fn activity(&self) -> &'static str {
        "map preview"
    }

    /// Song select's.
    fn presence(&self, _state: &State) -> Option<(String, String)> {
        None
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn drags(&self) -> bool {
        true
    }

    fn area_editable(&self) -> bool {
        true
    }

    fn covers_social_tab(&self) -> bool {
        true
    }

    fn over_lobby(&self) -> bool {
        self.back.screen().is_some_and(|s| s.room_lobby())
    }

    fn song_select(&self) -> bool {
        true
    }

    fn map_from_song_select(&self) -> bool {
        true
    }

    fn allows_settings(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
            let step = if code == KeyCode::ArrowRight {
                5000.0
            } else {
                -5000.0
            };
            state.preview.seek_by(step, state.audio.as_ref());
            return;
        }
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        match code {
            KeyCode::Space => state.toggle_preview_held(),
            KeyCode::Escape | KeyCode::Backspace => self.close(state),
            KeyCode::Enter | KeyCode::NumpadEnter if state.modifiers_ctrl => self.close(state),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let marked = state
                    .preview_target()
                    .is_some_and(|i| state.preview.marker(&state.library[i]).is_some());
                let lobby = self.over_lobby();
                if marked || lobby || state.picking_lobby_map {
                    state.preview_button(lobby);
                } else if let Some(i) = state.select.selected() {
                    state.start_loading(i);
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let lobby = self.over_lobby();
        match frame.preview_target {
            Some(i) => {
                let scroll = state.cfg.scroll_speed;
                let see_through = state.bg_brightness > 0.0;
                let kiai = state.cfg.kiai_effects;
                if let Some(game) = state.preview.game_for(&state.library, i) {
                    game.view.kiai_effects = kiai;
                    draw_game_area(
                        canvas,
                        &state.cfg,
                        &mut state.frame_clip,
                        &state.grip,
                        &state.hover,
                        see_through,
                        |c| game.draw_playfield(c, scroll),
                    );
                }
                let e = &state.library[i];
                let stars = state.stars.get(i, state.play_speed().0);
                draw_preview_chrome(canvas, &state.cfg, e, stars, lobby, frame.size_label);
                let (x, y, w) = (24.0, 394.0, canvas.width - 48.0);
                canvas.round_rect(
                    [x, y],
                    [w, 78.0],
                    8.0,
                    rgba(0x15151d, 0.9),
                    rgba(0x2a2a38, 1.0),
                    1.0,
                );
                let button = match (lobby, frame.lobby_host) {
                    (false, _) => Some(("Play from", "Enter")),
                    (true, true) => Some(("Start the match from", "Enter")),
                    (true, false) => None,
                };
                state.preview.draw_timeline(
                    canvas,
                    e,
                    [x + 12.0, y + 10.0, w - 24.0, 34.0],
                    button,
                );
                // A thumb's over the timeline on a phone, where its times don't run into it.
                let [bx, by, bw, bh] = if mobile::active() {
                    [x + w - 130.0, y - 50.0, 130.0, 44.0]
                } else {
                    [x + 12.0, y + 54.0, pause_width(), 16.0]
                };
                canvas.button(
                    bx,
                    by,
                    bw,
                    bh,
                    pause_label(state.preview.held),
                    mobile::pick(7.0, 12.0),
                    hit::Hit::Key(KeyCode::Space),
                    false,
                );
            }
            None => {
                return Some(*std::mem::replace(
                    &mut self.back,
                    Box::new(SongSelect::scene()),
                ));
            }
        }
        None
    }
}
