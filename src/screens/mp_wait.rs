//! Waiting for a multiplayer match to start: the room's map loads, the server is told it
//! did, and at the start time the server set the match begins (`Scene::MpPlay`).

use super::screen::{Frame, Key, Screen};
use crate::*;

/// The match's map loading, then the count to its start.
pub(crate) struct MpWait {
    rx: Job<Loaded>,
    loaded: Option<Box<(Beatmap, Arc<Track>)>>,
    /// When the match starts, once the server has said.
    pub(crate) start: Option<Instant>,
    /// Map time the match starts at.
    from: Option<f64>,
}

impl MpWait {
    /// Waiting for the match on the map `rx` loads, from map time `from`.
    pub(crate) fn scene(rx: Job<Loaded>, from: Option<f64>) -> Scene {
        Scene::Other(Box::new(MpWait {
            rx,
            loaded: None,
            start: None,
            from,
        }))
    }
}

impl Screen for MpWait {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "MpWait"
    }

    fn activity(&self) -> &'static str {
        "multiplayer match"
    }

    /// The match's own (`State::presence_text` asks `in_match`, as for the match itself).
    fn presence(&self, _state: &State) -> Option<(String, String)> {
        None
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn in_room(&self) -> bool {
        true
    }

    fn in_song(&self) -> bool {
        true
    }

    fn in_match(&self) -> bool {
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
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        if code == KeyCode::Escape {
            if let Some(s) = &state.mp {
                s.client.send(net::ClientMsg::Loaded(false));
                s.client.send(net::ClientMsg::Finished(net::Final {
                    quit: true,
                    ..Default::default()
                }));
            }
            state.scene = super::lobby::Lobby::scene();
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let mut next = None;
        match self.rx.poll_ended() {
            Some(Some(Ok(l))) => {
                self.loaded = Some(Box::new(l));
                if let Some(s) = &state.mp {
                    s.client.send(net::ClientMsg::Loaded(true));
                }
            }
            Some(Some(Err(e))) => {
                if let Some(s) = &state.mp {
                    s.client.send(net::ClientMsg::Loaded(false));
                }
                state.status = format!("failed to load map: {e}");
                next = Some(super::lobby::Lobby::scene());
            }
            None => {}
            Some(None) => next = Some(super::lobby::Lobby::scene()),
        }
        if self.start.is_some_and(|at| frame.now >= at)
            && let Some(loaded) = self.loaded.take()
        {
            let (map, track) = *loaded;
            let seed = state
                .mp
                .as_ref()
                .and_then(|s| s.map.as_ref())
                .and_then(|m| m.seed)
                .unwrap_or(0);
            let defense = state
                .mp
                .as_ref()
                .and_then(|s| s.map.as_ref())
                .and_then(|m| m.defense);
            // Defense is played on the Maze, whatever else bent the lane.
            let mut mods = state.pending_mods;
            if defense.is_some_and(|d| d.maze) && !mods.has(mods::Mod::Maze) {
                mods.toggle(mods::Mod::Maze);
            }
            let mut game = Play::new(
                Game::with_seed(map, track, sound(&state.audio), mods, seed)
                    .with_settings(&crate::game::settings()),
            );
            if let (Some(d), Some(s)) = (defense, &state.mp) {
                let (team, me) = multiplayer::defense_team(s);
                game.set_defense(team, me, d);
            }
            game.set_scroll_speed(state.cfg.scroll_speed);
            if let Some(i) = state.mp.as_ref().and_then(|s| s.map_entry) {
                let live_pp = pp::Live::start(
                    state.library[i].path.clone(),
                    stars::read,
                    game.rate(),
                    state.pending_mods,
                    game.scroll_speed(),
                    game.seed(),
                    Default::default(),
                );
                game.set_live_pp(live_pp);
            }
            if let Some(m) = state.mp.as_ref().and_then(|s| s.map.as_ref()) {
                use_mod_settings(&mut game, m);
            }
            if let Some(t) = self.from {
                game.start_from(t, sound(&state.audio), state.play_offset);
            }
            game.set_record(true);
            if let Some(s) = &mut state.mp {
                s.inputs_sent = 0;
                s.skip_asked = false;
                s.skip_votes = None;
            }
            next = Some(Scene::MpPlay {
                game: Box::new(game),
                quit_armed: None,
            });
        }
        if let Some(s) = &state.mp {
            multiplayer::draw_waiting(canvas, s, self.start, self.loaded.is_some(), frame.now);
        }

        next
    }
}
