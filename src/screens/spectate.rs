//! Watching a multiplayer match in progress, `SPECTATE_DELAY` behind it: one game per player,
//! fed with their presses.

use super::lobby::Lobby;
use super::mp_results::MpResults;
use super::screen::{Frame, Key, Screen};
use crate::multiplayer::map_label;
use crate::*;

/// How far behind the live match spectators watch, so players' presses arrive in time.
pub(crate) const SPECTATE_DELAY: Duration = Duration::from_millis(1500);

pub(crate) struct SpectateView {
    pub(crate) rx: Job<Loaded>,
    pub(crate) loaded: Option<(Beatmap, Arc<Track>)>,
    /// When the match started (local clock), its start point and speed.
    pub(crate) start: Instant,
    pub(crate) from: Option<f64>,
    pub(crate) rate: f64,
    pub(crate) mods: Mods,
    /// The match's map as the room named it, with the host's mod settings.
    pub(crate) map: net::MapInfo,
    pub(crate) players: Vec<(u32, String)>,
    /// One game per player, fed with their presses.
    pub(crate) games: Vec<(u32, Box<Play>)>,
    /// The player shown, or None for everyone at once.
    pub(crate) focus: Option<usize>,
    /// Presses that arrived before the games existed.
    pub(crate) pending: std::collections::HashMap<u32, Vec<(f64, u8)>>,
    /// Players who skipped the intro; once all have, so does the view.
    pub(crate) skipped: std::collections::HashSet<u32>,
    pub(crate) skip_done: bool,
    /// Watching everyone: what each player's strip shows (see `draw_spectate_all`), and when it
    /// was last drawn.
    pub(crate) zones: Vec<[f32; 4]>,
    pub(crate) zones_at: Option<Instant>,
    /// The match is over (the results came); we're `SPECTATE_DELAY` behind, so the last seconds
    /// are still to be watched.
    pub(crate) ended: Option<Instant>,
}

impl SpectateView {
    /// When every player has skipped the intro, skips it too, staying `SPECTATE_DELAY` behind.
    pub(crate) fn follow_skip(&mut self, audio: Option<&audio::Engine>, offset: f64) {
        if self.skip_done
            || self.games.is_empty()
            || !self.players.iter().all(|(id, _)| self.skipped.contains(id))
        {
            return;
        }
        self.skip_done = true;
        let target =
            self.games[0].1.skip_target() - SPECTATE_DELAY.as_secs_f64() * 1000.0 * self.rate;
        if self.games[0].1.time() < target {
            for (_, g) in &mut self.games {
                g.seek(target, audio.map(|a| a as &dyn game::Audio), offset);
            }
        }
    }
}

/// A match being watched.
pub(crate) struct Spectate {
    pub(crate) view: Box<SpectateView>,
}

impl Spectate {
    pub(crate) fn scene(view: Box<SpectateView>) -> Scene {
        Scene::Other(Box::new(Spectate { view }))
    }
}

impl Screen for Spectate {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Spectate"
    }

    fn activity(&self) -> &'static str {
        "spectating"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        let label = state
            .mp
            .as_ref()
            .and_then(|s| s.map.as_ref())
            .map(map_label)
            .unwrap_or_default();
        let v = &self.view;
        let who = v
            .focus
            .and_then(|k| v.games.get(k))
            .and_then(|(id, _)| v.players.iter().find(|p| p.0 == *id))
            .map(|p| p.1.clone());
        let doing = match who {
            Some(w) => tr!("Spectating {w}", "{w}を観戦中", "Наблюдает за {w}"; [] [w]),
            None => tr!("Spectating everyone", "全員を観戦中", "Наблюдает за всеми").into(),
        };
        Some((label, doing))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn in_song(&self) -> bool {
        true
    }

    fn watching(&self) -> bool {
        true
    }

    fn spectating(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn loading(&self) -> bool {
        self.view.rx.is_busy()
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        let v = &mut self.view;
        let n = v.players.len().max(1);
        match code {
            KeyCode::ArrowLeft | KeyCode::ArrowRight if !repeat => {
                let dir = if code == KeyCode::ArrowRight {
                    1
                } else {
                    n - 1
                };
                v.focus = Some(v.focus.map(|f| (f + dir) % n).unwrap_or(0));
            }
            KeyCode::KeyA | KeyCode::Tab if !repeat => {
                v.focus = if v.focus.is_some() { None } else { Some(0) }
            }
            KeyCode::Escape if !repeat => {
                if let Some(s) = &state.mp {
                    s.client.send(net::ClientMsg::Spectate(false));
                }
                state.stop_audio();
                state.scene = Lobby::scene();
            }
            _ => {
                if !repeat {
                    state.handle_global_key(code, window);
                }
            }
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let v = &mut self.view;
        let mut next = None;
        match v.rx.poll_ended() {
            Some(Some(Ok(l))) => v.loaded = Some(l),
            Some(Some(Err(e))) => {
                state.status = format!("can't spectate: {e}");
                next = Some(Lobby::scene());
            }
            None => {}
            Some(None) => next = Some(Lobby::scene()),
        }
        if v.ended
            .is_some_and(|t| frame.now >= t + SPECTATE_DELAY + Duration::from_millis(500))
        {
            if let Some(a) = &state.audio {
                a.stop();
            }
            next = Some(MpResults::scene());
        }
        // Start once the map is ready and the delay has passed: the music where the
        // players were SPECTATE_DELAY ago, every player's game snapping to it.
        if v.games.is_empty()
            && frame.now >= v.start + SPECTATE_DELAY
            && let (Some((map, track)), Some(audio)) = (v.loaded.take(), state.audio.as_ref())
        {
            let probe = Game::with_mods(map.clone(), track.clone(), None, v.mods);
            let first_ms = match v.from {
                Some(t) => {
                    (probe.track_at(t) - 2000.0).max(probe.track_at(probe.time()))
                        + state.play_offset
                }
                None => probe.track_at(probe.time()),
            };
            let behind = (frame.now - v.start - SPECTATE_DELAY).as_secs_f64() * 1000.0;
            audio.play(track.clone(), first_ms + behind);
            for (k, (id, name)) in v.players.iter().enumerate() {
                // The lobby's speed mods, and the player's own others.
                let own = state
                    .mp
                    .as_ref()
                    .and_then(|s| s.players.iter().find(|p| p.id == *id))
                    .map(|p| Mods::from_list(&p.mods).free())
                    .unwrap_or(Mods::NONE);
                let mut mods = if v.map.defense.is_some() {
                    v.mods
                } else {
                    v.mods.shared().plus(own)
                };
                if v.map.defense.is_some_and(|d| d.maze) && !mods.has(mods::Mod::Maze) {
                    mods.toggle(mods::Mod::Maze);
                }
                let mut g = Play::new(
                    Game::with_seed(
                        map.clone(),
                        track.clone(),
                        None,
                        mods,
                        v.map.seed.unwrap_or(0),
                    )
                    .with_settings(&crate::game::settings()),
                );
                use_mod_settings(&mut g, &v.map);
                // Defense: each player's game takes only their turns' notes, as theirs did
                // (the team in the order the players have it: by id).
                if let Some(d) = v.map.defense {
                    let mut team = v.players.clone();
                    team.sort_by_key(|p| p.0);
                    let me = team.iter().position(|p| p.0 == *id).unwrap_or(0);
                    g.set_defense(team, me, d);
                }
                if let Some(t) = v.from {
                    g.skip_to(t);
                }
                let inputs = v.pending.remove(id).unwrap_or_default();
                g.set_replay(Some((inputs, 0)));
                g.view.banner = Some(format!(
                    "SPECTATING  -  {name}  ({}/{})",
                    k + 1,
                    v.players.len()
                ));
                g.view.watched = true;
                v.games.push((*id, Box::new(g)));
            }
            // Players may have skipped the intro before the view was ready.
            v.follow_skip(state.audio.as_ref(), state.play_offset);
        }
        if v.games.is_empty() {
            canvas.text_centered(
                canvas.width / 2.0,
                220.0,
                12.0,
                rgba(0xffffff, 1.0),
                "joining the match...",
            );
        }
        let scroll = state.cfg.scroll_speed;
        for (k, (_, g)) in v.games.iter_mut().enumerate() {
            // Only the player watched (or the first one) is heard.
            g.set_mute_hits(v.focus.unwrap_or(0) != k);
            g.view.kiai_effects = state.cfg.kiai_effects;
            g.view.hidden = frame.hidden;
            g.view.hud = state.cfg.hud;
            g.view.hud_area = state.cfg.playfield_area;
            g.update(frame.now, sound(&state.audio), state.play_offset);
        }
        // Defense: the team's lives and gold, as the room has them.
        if let Some(team) = state.mp.as_ref().and_then(|s| s.team) {
            for (_, g) in v.games.iter_mut() {
                g.set_team(team);
            }
        }
        match v.focus {
            Some(k) if k < v.games.len() => {
                let g = &mut v.games[k].1;
                draw_game_area(
                    canvas,
                    &state.cfg,
                    &mut state.frame_clip,
                    &state.grip,
                    &state.hover,
                    state.bg_brightness > 0.0,
                    |c| {
                        g.draw_playfield(c, scroll);
                    },
                );
                g.draw_hud(canvas);
                canvas.text_centered(
                    canvas.width / 2.0,
                    470.0,
                    7.0,
                    rgba(0x9a9ab0, 1.0),
                    "Left/Right: other player   A: everyone   Esc: leave",
                );
            }
            _ if !v.games.is_empty() => {
                let dt = v.zones_at.map_or(0.0, |at| {
                    frame.now.saturating_duration_since(at).as_secs_f32()
                });
                v.zones_at = Some(frame.now);
                draw_spectate_all(canvas, &mut v.games, &v.players, scroll, &mut v.zones, dt);
            }
            _ => {}
        }
        next
    }
}
