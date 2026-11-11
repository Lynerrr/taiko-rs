//! A multiplayer match's results: every player's score for the room's map. The results are
//! the session's (`State::mp`), which the lobby shows too (the last winner).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The match's results up.
pub(crate) struct MpResults;

impl MpResults {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(MpResults))
    }
}

impl Screen for MpResults {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "MpResults"
    }

    fn activity(&self) -> &'static str {
        "multiplayer results"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        Some((
            state
                .mp
                .as_ref()
                .and_then(|s| s.map.as_ref())
                .map(multiplayer::map_label)
                .unwrap_or_default(),
            tr!(
                "Multiplayer results",
                "マルチのリザルト",
                "Итоги мультиплеера"
            )
            .into(),
        ))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn in_room(&self) -> bool {
        true
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(7)
    }

    fn finger_scrolls(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        // Results stay on the session so the lobby can show the last winner.
        if matches!(
            code,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape
        ) {
            state.scene = super::lobby::Lobby::scene();
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        match &state.mp {
            Some(s) => {
                multiplayer::draw_results(canvas, s);
                None
            }
            None => Some(MpMenu::scene()),
        }
    }
}
