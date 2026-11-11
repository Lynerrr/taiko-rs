//! Loading a map to play it: the game is made when the map and its music are in.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// A map on its way to being played.
pub(crate) struct Loading {
    pub(crate) rx: Job<Loaded>,
    pub(crate) label: String,
    pub(crate) entry: usize,
    /// The map time to start at (picked on the song select timeline), None: from the start.
    pub(crate) from: Option<f64>,
    /// A retry: the intro was heard already, so it's skipped.
    pub(crate) retry: bool,
}

impl Loading {
    pub(crate) fn scene(rx: Job<Loaded>, label: String, entry: usize, from: Option<f64>) -> Scene {
        Scene::Other(Box::new(Loading {
            rx,
            label,
            entry,
            from,
            retry: false,
        }))
    }
}

impl Screen for Loading {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Loading"
    }

    fn activity(&self) -> &'static str {
        "loading a map"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        Some((
            state.library.get(self.entry)?.label(),
            tr!("Loading...", "読み込み中...", "Загрузка...").into(),
        ))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn map_entry(&self) -> Option<usize> {
        Some(self.entry)
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn shows_own_background(&self) -> bool {
        true
    }

    fn loads_song(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn loading(&self) -> bool {
        self.rx.is_busy()
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if repeat
            && !matches!(
                code,
                KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::Backspace
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        {
            return;
        }
        if state.handle_global_key(code, window) {
            return;
        }
        if code == KeyCode::Escape {
            state.show_select();
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        let mut next = None;
        canvas.text_centered(
            canvas.width / 2.0,
            200.0,
            14.0,
            rgba(0xffffff, 1.0),
            "Loading",
        );
        canvas.text_centered(
            canvas.width / 2.0,
            226.0,
            10.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(10.0, canvas.width - 48.0, &self.label),
        );
        match self.rx.poll_ended() {
            Some(Some(Ok((map, track)))) => {
                let mods = std::mem::take(&mut state.pending_mods);
                // Random: the replay's colouring, otherwise the configured repeatable seed.
                let seed = match &state.pending_replay {
                    Some(r) => r.seed,
                    None if mods.has(mods::Mod::Random) => state.cfg.random_seed,
                    None => 0,
                };
                let half = state
                    .pending_replay
                    .as_ref()
                    .map_or(Default::default(), |r| r.half_rule());
                let mut game = Play::new(
                    Game::with_half(map, track, sound(&state.audio), mods, seed, half)
                        .with_settings(&crate::game::settings()),
                );
                game.set_scroll_speed(state.cfg.scroll_speed);
                let live_pp = pp::Live::start(
                    state.library[self.entry].path.clone(),
                    stars::read,
                    game.rate(),
                    mods,
                    game.scroll_speed(),
                    game.seed(),
                    half,
                );
                game.set_live_pp(live_pp);
                if let Some(t) = self.from {
                    game.start_from(t, sound(&state.audio), state.play_offset);
                } else if self.retry {
                    game.skip(sound(&state.audio), state.play_offset);
                }
                match state.pending_replay.take() {
                    Some(r) => {
                        game.set_mash(r.mash());
                        game.set_window_rule(r.window_rule());
                        game.set_scroll_speed(r.scroll_speed);
                        game.set_replay_of(Some(r.player.clone()));
                        game.view.banner = Some(format!("REPLAY  -  {}", r.player));
                        game.set_replay(Some((r.inputs, 0)));
                    }
                    // A test play from the editor keeps nothing.
                    None if state.editor_back.is_some() => {
                        game.set_record(false);
                        game.view.test_play = true;
                        game.view.banner = Some(State::test_banner(state.test_auto));
                    }
                    // Full plays (not autoplay, not from a start point) are kept.
                    None => {
                        game.set_record(
                            (!state.auto || std::env::var_os("TAIKO_RECORD_AUTO").is_some())
                                && self.from.is_none()
                                && !mods.unranked(),
                        );
                        if mods.has(mods::Mod::Autoplay) {
                            game.view.banner = Some("AUTOPLAY".into());
                        }
                        if state.cfg.fail_restart {
                            game.set_fail_linger(FAIL_RESTART_LINGER);
                        }
                    }
                }
                state.play = Some(Box::new(game));
                next = Some(super::play::PlayScreen::scene(self.entry, self.from));
            }
            Some(Some(Err(e))) => {
                state.status = format!("failed to load {}: {e}", self.label);
                eprintln!("{}", state.status);
                next = Some(SongSelect::scene());
            }
            None => {}
            Some(None) => next = Some(SongSelect::scene()),
        }
        next
    }
}
