//! A song played on its own (`PlayScreen`), and drawing around a play: the replay bar and its
//! pause button, watching everyone, and the quick restarts after a fail.

use super::results::{ResultsScreen, map_gives_pp, results_job, results_title, submit_score};
use super::screen::{Frame, Key, Screen};
use crate::*;

/// A pause button's label, `held` or going.
pub(crate) fn pause_label(held: bool) -> &'static str {
    if held {
        tr!("Play", "再生", "Играть")
    } else {
        tr!("Pause", "ポーズ", "Пауза")
    }
}

/// A pause button's width: the same with either label, so nothing moves when it's pressed.
pub(crate) fn pause_width() -> f32 {
    Canvas::text_width(7.0, pause_label(true)).max(Canvas::text_width(7.0, pause_label(false)))
        + 12.0
}

/// Replays: a timeline along the bottom of the screen to click or drag through, over the
/// map's difficulty with the replay's misses marked. Returns the bar's rect (virtual units) for
/// `replay_seek_at`.
/// `grow`: how far it has grown towards its hovered height (0 to 1), kept between frames.
pub(crate) fn draw_replay_bar(
    c: &mut Canvas,
    game: &Game,
    active: bool,
    speed: f64,
    grow: &mut f32,
) -> [f32; 4] {
    let w = c.width;
    let (start, end) = game.span();
    let label = format!("{} / {}", fmt_time(game.time()), fmt_time(end));
    let y = 456.0;
    // Its buttons, then the time and the timeline.
    let mut left = 18.0;
    for (text, bw, hit) in [
        (pause_label(game.held()), pause_width(), KeyCode::Space),
        ("-", 20.0, KeyCode::ArrowDown),
        ("+", 20.0, KeyCode::ArrowUp),
    ] {
        c.button(
            left,
            y - 8.0,
            bw,
            16.0,
            text,
            7.0,
            hit::Hit::Key(hit),
            false,
        );
        left += bw + 4.0;
    }
    let shown = format!("{speed:.2}x");
    c.text(left + 2.0, y - 3.5, 7.0, rgba(0xc8c8d8, 1.0), &shown);
    left += Canvas::text_width(7.0, "0.00x") + 12.0;
    // Sized for the longest label so the bar doesn't twitch as the time changes.
    let lw = Canvas::text_width(7.0, &format!("{} / {}", fmt_time(end), fmt_time(end)));
    let (x, bw) = (left + lw + 10.0, w - 18.0 - (left + lw + 10.0));
    // The strip: difficulty bars standing on `base`, misses over them. Hovered or dragged, it
    // grows upwards to show more.
    let base = y + 1.0;
    let hover_at = |gh: f32| [x, base - gh, bw, gh + 2.0];
    let now_rect = hover_at(17.0 + *grow * 13.0);
    let hover = active
        || c.hovered(
            now_rect[0] - 6.0,
            now_rect[1],
            now_rect[2] + 12.0,
            now_rect[3],
        );
    render::approach(grow, if hover { 1.0 } else { 0.0 }, c.dt_ms, 60.0);
    let gh = 17.0 + *grow * 13.0;
    let rect = hover_at(gh);
    let top = base - gh;
    let span = (end - start).max(1.0);
    let x_of = |t: f64| x + (((t - start) / span).clamp(0.0, 1.0) as f32) * bw;
    let f = ((game.time() - start) / span).clamp(0.0, 1.0) as f32;
    c.text(left, y - 3.5, 7.0, rgba(0xc8c8d8, 1.0), &label);
    c.round_rect(
        [x - 2.0, top - 2.0],
        [bw + 4.0, gh + 4.0],
        4.0,
        rgba(0x0c0c12, 0.8),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    if let Some((graph, misses)) = game.replay_timeline() {
        // Kiai tinted, as on song select's graph.
        for &(s, e) in &graph.kiai {
            let (kx0, kx1) = (x_of(s), x_of(e));
            c.rect(kx0, top, (kx1 - kx0).max(1.0), gh, rgba(0xff8a3d, 0.12));
        }
        let (bx0, bx1) = (
            x_of(graph.peaks_start),
            x_of(graph.peaks_start + graph.peaks.len() as f64 * graph.section),
        );
        let n = (((bx1 - bx0) / 2.5) as usize).max(1);
        let bar_w = (bx1 - bx0) / n as f32;
        let now_x = x + bw * f;
        for (i, v) in graph.bins(n).into_iter().enumerate() {
            let bh = 1.0 + v * (gh - 3.0);
            let bx = bx0 + i as f32 * bar_w;
            let played = bx + bar_w / 2.0 <= now_x;
            let colour = rgba(preview::strain_colour(v), if played { 1.0 } else { 0.55 });
            c.rect(bx, base - 1.0 - bh, (bar_w - 0.5).max(0.5), bh, colour);
        }
        // Each miss: a glowing dot on the strip's top edge and a fine line down to the bars;
        // the ones still ahead are dimmed, like the bars.
        const MISS: u32 = 0xff4d6a;
        for &t in misses {
            let mx = x_of(t);
            let a = if mx <= now_x { 1.0 } else { 0.5 };
            c.rect(mx - 0.5, top + 2.0, 1.0, gh - 3.0, rgba(MISS, 0.35 * a));
            c.circle(mx, top - 1.0, 4.0, rgba(MISS, 0.18 * a), rgba(0, 0.0), 0.0);
            c.circle(
                mx,
                top - 1.0,
                2.0,
                rgba(MISS, a),
                rgba(0xffffff, 0.8 * a),
                0.75,
            );
        }
    }
    c.text(
        x,
        y + 10.0,
        6.0,
        rgba(0x6f6f86, 1.0),
        tr!(
            "Left/Right or drag: seek   Space: pause   Up/Down: speed",
            "左右かドラッグ: シーク   Space: ポーズ   上下: 速度",
            "Влево/вправо или тяните: перемотка   Пробел: пауза   Вверх/вниз: скорость"
        ),
    );
    // The playhead: a white line a little taller than the strip.
    let hx = x + bw * f;
    c.rect(hx - 0.75, top - 2.0, 1.5, gh + 4.0, rgba(0xffffff, 0.95));
    rect
}

/// Every spectated player at once: one lane per player, stacked, with their numbers.
/// Each player's strip shows where they play: the lane, or wherever the mods take it (rails,
/// Multiverse's playfield in focus...), fitted in and followed smoothly. `zones` keeps what each
/// strip shows between frames; `dt` is the time since the last one (s).
pub(crate) fn draw_spectate_all(
    c: &mut Canvas,
    games: &mut [(u32, Box<Play>)],
    players: &[(u32, String)],
    scroll: f64,
    zones: &mut Vec<[f32; 4]>,
    dt: f32,
) {
    let n = games.len().max(1) as f32;
    let (band_top, band_h) = game::lane_band();
    let (top, bottom) = (40.0, 470.0);
    let strip = ((bottom - top) / n).min(120.0);
    let k = strip / band_h;
    let full_w = c.width;
    // Drawn wider than the screen, so a plain lane fills the strip's width.
    let plain = [0.0, band_top, full_w / k, band_h];
    zones.resize(games.len(), [f32::NAN; 4]);
    let follow = 1.0 - (-dt / 0.15).exp();
    let head = tr!(
        "SPECTATING  -  everyone   Left/Right or A: one player   Esc leave",
        "観戦中  -  全員   左右かA: 一人   Esc 退出",
        "НАБЛЮДЕНИЕ  -  все   Влево/вправо или A: один игрок   Esc выйти"
    );
    c.text_centered(
        full_w / 2.0,
        16.0,
        9.0,
        rgba(0x4fb4ff, 1.0),
        &Canvas::fit(9.0, full_w - 16.0, head),
    );
    for (i, (id, g)) in games.iter_mut().enumerate() {
        let y = top + i as f32 * strip;
        let start = c.shapes.len();
        c.width = full_w / k;
        g.draw_playfield(c, scroll);
        c.width = full_w;
        let target = g
            .play_zone()
            .filter(|z| z[2] > 1.0 && z[3] > 1.0)
            .unwrap_or(plain);
        let z = &mut zones[i];
        if z[0].is_nan() {
            *z = target;
        } else {
            for (v, t) in z.iter_mut().zip(target) {
                *v += (t - *v) * follow;
            }
        }
        let [zx, zy, zw, zh] = *z;
        let scale = (full_w / zw).min(strip / zh);
        let (dx, dy) = (
            (full_w - zw * scale) / 2.0 - zx * scale,
            y + (strip - zh * scale) / 2.0 - zy * scale,
        );
        c.transform_since(start, [0.0, 0.0], scale, dx, dy);
        c.clip_since(start, 0.0, y, full_w, strip - 2.0);
        let name = players
            .iter()
            .find(|(p, _)| p == id)
            .map(|(_, n)| n.as_str())
            .unwrap_or("?");
        let st = g.stats();
        let line = format!(
            "{name}   {:08}   {:.2}%   {}x",
            st.score,
            st.accuracy() * 100.0,
            st.combo
        );
        let tw = Canvas::text_width(8.0, &line);
        c.round_rect(
            [full_w - tw - 28.0, y + 4.0],
            [tw + 16.0, 16.0],
            6.0,
            rgba(0x000000, 0.6),
            rgba(0, 0.0),
            0.0,
        );
        c.text_right(full_w - 20.0, y + 8.0, 8.0, rgba(0xffffff, 1.0), &line);
    }
}

/// Seconds the FAILED sign shows before a Sudden Death / Perfect fail starts the map over.
pub(crate) const FAIL_RESTART_LINGER: f64 = 0.6;

/// Whether a play that just ended starts over instead of showing results: one of the player's own
/// (not a replay) failed by Sudden Death / Perfect, with `Config::fail_restart` on.
pub(crate) fn fail_restarts(cfg: &Config, game: &Game) -> bool {
    cfg.fail_restart && game.failed_early() && game.replay().is_none()
}

/// What a play's frame asks for, done once the frame is drawn.
#[derive(Default)]
pub(crate) struct Out {
    /// A screenshot of this frame, by name.
    pub(crate) capture: Option<String>,
    /// A test play from the editor is over: back to the editor.
    pub(crate) test_over: bool,
    /// A failed play starts over at once.
    pub(crate) fail_restart: bool,
}

/// A song played on its own (by the player, a replay or autoplay); its game is `State::play`.
pub(crate) struct PlayScreen {
    pub(crate) entry: usize,
    /// Where the play started (map time), None: from the start.
    pub(crate) from: Option<f64>,
    pub(crate) out: Out,
}

impl PlayScreen {
    pub(crate) fn scene(entry: usize, from: Option<f64>) -> Scene {
        Scene::Other(Box::new(PlayScreen {
            entry,
            from,
            out: Out::default(),
        }))
    }
}

impl Screen for PlayScreen {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Play"
    }

    fn activity(&self) -> &'static str {
        "playing"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        let game = state.play.as_deref()?;
        let label = state.library.get(self.entry)?.label();
        let doing = if let Some(who) = game.replay_of() {
            tr!("Watching {who}'s replay", "{who}のリプレイを観戦中", "Смотрит повтор {who}";)
        } else if game.paused().is_some() {
            tr!("Paused", "ポーズ中", "Пауза").into()
        } else {
            let stars = state
                .stars
                .get(self.entry, game.rate())
                .map(|s| format!("  {s:.2}*"))
                .unwrap_or_default();
            let rate = if (game.rate() - 1.0).abs() > 1e-6 {
                format!("  {:.2}x", game.rate())
            } else {
                String::new()
            };
            tr!("Playing{stars}{rate}", "プレイ中{stars}{rate}", "Играет{stars}{rate}";)
        };
        Some((label, doing))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn song_play(&self) -> Option<(usize, Option<f64>)> {
        Some((self.entry, self.from))
    }

    fn holds_game(&self) -> bool {
        true
    }

    fn map_entry(&self) -> Option<usize> {
        Some(self.entry)
    }

    fn in_song(&self) -> bool {
        true
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

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if let Some(game) = state.play.as_deref_mut() {
            if game.paused().is_some() {
                if !repeat
                    && !state.cfg.keys.contains(&code)
                    && !state.handle_global_key(code, window)
                {
                    state.pause_key(code);
                }
                return;
            }
            // Drum keys never come here: `State::key_inner` gives them to the game first.
            if game.replay().is_some() && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown) {
                state.step_replay_speed(if code == KeyCode::ArrowUp { 1 } else { -1 });
                return;
            }
            if game.replay().is_some() && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
                let step = if state.modifiers_shift {
                    15_000.0
                } else {
                    5_000.0
                };
                let t = game.time()
                    + if code == KeyCode::ArrowRight {
                        step
                    } else {
                        -step
                    };
                game.seek(t, sound(&state.audio), state.play_offset);
                return;
            }
        }
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
        match code {
            KeyCode::KeyO => state.apply_live_offset(),
            KeyCode::Tab => state.toggle_map_info(),
            KeyCode::Escape | KeyCode::Space => {
                let Some(game) = state.play.as_deref_mut() else {
                    return;
                };
                match code {
                    // After the last note there's nothing left to pause: straight to the
                    // results.
                    KeyCode::Escape if game.skip_outro() => {}
                    KeyCode::Escape => game.pause(sound(&state.audio)),
                    KeyCode::Space if game.replay().is_some() => {
                        game.toggle_held(sound(&state.audio))
                    }
                    _ => {
                        game.skip(sound(&state.audio), state.play_offset);
                    }
                }
            }
            KeyCode::Backquote => state.hold_retry(),
            KeyCode::KeyR if state.modifiers_ctrl => state.restart_map(),
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let game = state.play.as_deref_mut()?;
        let done = game.update(frame.now, sound(&state.audio), state.play_offset);
        if (state.auto || game.mods().has(mods::Mod::Autoplay)) && game.replay().is_none() {
            game.set_autoplay_style(state.cfg.autoplay);
            game.autoplay(sound(&state.audio));
        }
        if let Some(sm) = &mut state.smooth {
            if game.paused().is_some() {
                // A frozen clock isn't jitter; restart the frame-to-frame comparison on resume.
                sm.break_sequence();
            } else {
                let wall = frame.now.duration_since(state.started).as_secs_f64() * 1000.0;
                let shown = state
                    .frame_acquired
                    .unwrap_or(frame.now)
                    .duration_since(state.started)
                    .as_secs_f64()
                    * 1000.0;
                sm.sample(
                    wall,
                    game.raw_time(),
                    game.last_audio_ms(),
                    shown,
                    game.raw_time() + frame.visual_offset,
                );
            }
        }
        // Offset advice is for live play (and its line would sit on the replay timeline).
        game.view.show_offset_hint =
            state.cfg.show_offset_hint && state.cfg.show_map_info && game.replay().is_none();
        game.view.offset_key = state
            .cfg
            .hotkeys
            .get(keys::Hotkey::ApplyOffset)
            .map(keys::Chord::name);
        game.view.kiai_effects = state.cfg.kiai_effects;
        game.view.hidden = frame.hidden;
        game.view.hud = state.cfg.hud;
        game.view.hud_area = state.cfg.playfield_area;
        game.set_ur_scaled_hint(state.cfg.offset_hint_ur_scaled);
        game.set_visual_offset(frame.visual_offset);
        draw_game_area(
            canvas,
            &state.cfg,
            &mut state.frame_clip,
            &state.grip,
            &state.hover,
            state.bg_brightness > 0.0,
            |c| {
                game.draw_playfield(c, game.scroll_speed());
            },
        );
        game.draw_hud(canvas);
        if game.replay().is_none()
            && game.paused().is_none()
            && !frame.hidden.has(hud::Part::Spectators)
        {
            let names = state.social.spectators();
            game.placed_from(canvas, hud::Part::Spectators, [12.0, 276.0], |c| {
                social::draw_spectators(c, &names);
            });
        }
        if game.replay().is_some() && game.paused().is_none() {
            if game.replay_timeline().is_none() {
                let misses = game.replay_misses();
                let replay_timeline = Some((difficulty::analyze(game.map()), misses));
                game.set_replay_timeline(replay_timeline);
            }
            state.replay_bar = Some(draw_replay_bar(
                canvas,
                game,
                state.replay_scrub.is_some(),
                state.replay_speed_wanted,
                &mut state.replay_bar_grow,
            ));
        }
        if let Some(plan) = &mut state.shots
            && plan.times.last().is_some_and(|&t| game.time() >= t)
        {
            let t = plan.times.pop().unwrap();
            self.out.capture = Some(format!("play-{t:.0}.png"));
        }
        if done && state.editor_back.is_some() {
            // A test play has no results: back to the editor.
            self.out.test_over = true;
        } else if done && fail_restarts(&state.cfg, game) {
            // Failed plays aren't kept anyway: straight into the next try.
            self.out.fail_restart = true;
        } else if done {
            let title = results_title(&game.map().meta, self.from);
            let failed = game.failed();
            if game.failed_early() {
                if let Some(a) = &state.audio {
                    a.stop();
                }
                state.preview.invalidate_audio();
            }
            let mut results = Results::new(title, game.stats().clone(), game.windows());
            results.set_mods(game.mods(), game.rate());
            results.seed = Some(game.seed());
            results.watched = game.replay_of().clone();
            results.failed = failed;
            // The song plays on under the results (not after a fail that stopped it),
            // from the preview point again once it ends.
            if !game.failed_early() && state.audio.is_some() {
                let preview = game.map().meta.preview_time;
                let again = if preview >= 0.0 {
                    game.track_at(preview)
                } else {
                    game.track_at(game.end_time()) * 0.4
                };
                results.music = game::Song::into_any(game.track().clone())
                    .downcast::<audio::Track>()
                    .ok()
                    .map(|t| (t, again.max(0.0)));
            }
            let e = &state.library[self.entry];
            let play = pp::Play {
                great: game.stats().great,
                ok: game.stats().ok,
                miss: game.stats().miss,
                max_combo: game.stats().max_combo,
                rate: game.rate(),
                mods: game.mods(),
                scroll_speed: game.scroll_speed(),
                seed: game.seed(),
                half: game.half(),
            };
            results.no_pp = if game.mods().has(mods::Mod::OdOverride) {
                Some("OD Override gets no pp")
            } else if game.mods().unranked() {
                Some("Autoplay gets no pp")
            } else if failed {
                Some("failed plays get no pp")
            } else if self.from.is_some() {
                Some("plays from a start point get no pp")
            } else if map_gives_pp(
                &state.library[self.entry],
                &state.set_status,
                &state.db_status,
                &state.select.server_loved,
            ) == Some(false)
            {
                Some("only ranked and loved maps give pp")
            } else {
                results.job = Some(results_job(e.path.clone(), play, None, None));
                None
            };
            // Failed plays aren't kept, like osu!.
            if game.record() && !failed {
                let e = &state.library[self.entry];
                let st = game.stats();
                let replay = scores::Replay {
                    version: scores::new_version(),
                    map_hash: e.hash(),
                    map_path: e.path.to_string_lossy().into_owned(),
                    map_label: e.label(),
                    player: state.cfg.mp_name.clone(),
                    date: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    rate: game.rate(),
                    keep_pitch: state.cfg.rate_keep_pitch,
                    anti_mash: true,
                    mods: game.mods().list(),
                    scroll_speed: game.scroll_speed(),
                    score: st.score,
                    accuracy: st.accuracy(),
                    max_combo: st.max_combo,
                    great: st.great,
                    ok: st.ok,
                    miss: st.miss,
                    ur: st.unstable_rate(),
                    inputs: game.take_inputs(),
                    seed: game.seed(),
                };
                let pp = pp::play_pp(&e.path, &replay.pp_play());
                results.pp = pp.filter(|_| results.no_pp.is_none());
                if pp.is_none() && results.no_pp.is_none() {
                    results.no_pp = Some("the map couldn't be rated");
                }
                // Whole plays go to the online leaderboards too, when logged in.
                if self.from.is_none()
                    && !state.cfg.online_token.is_empty()
                    && !game.mods().unranked()
                {
                    let m = &game.map().meta;
                    let play = online::ScoreSubmit {
                        map: net::MapInfo {
                            hash: replay.map_hash,
                            artist: m.artist.clone(),
                            title: m.title.clone(),
                            version: m.version.clone(),
                            rate: replay.rate,
                            from: None,
                            set_id: e.set_id(),
                            mods: replay.mods.clone(),
                            multiverse: None,
                            shapeshift: None,
                            seed: (replay.seed != 0).then_some(replay.seed),
                            defense: None,

                            stats: None,
                        },
                        score: replay.score,
                        accuracy: replay.accuracy,
                        max_combo: replay.max_combo,
                        great: replay.great,
                        ok: replay.ok,
                        miss: replay.miss,
                        pp,
                        date: replay.date,
                        replay_version: replay.version,
                        inputs: replay.inputs.clone(),
                        rolls: Some(game.stats().roll_ticks),
                        stars: None,
                        pp_version: online::PP_VERSION,
                        scroll_speed: Some(replay.scroll_speed),
                        play_ms: Some(game.played_ms() as u64),
                        md5: None,
                        timing: {
                            let (latency_ms, reported_latency_ms) = state
                                .audio
                                .as_ref()
                                .map(|a| a.latencies_ms())
                                .unwrap_or_default();
                            Some(online::PlayTiming {
                                os: std::env::consts::OS.into(),
                                present: state.present.name().into(),
                                offset_ms: state.play_offset,
                                visual_lead_ms: frame.visual_offset - state.pace_shift_ms,
                                latency_ms,
                                reported_latency_ms,
                            })
                        },
                    };
                    state.submits.push(submit_score(
                        state.cfg.online_server.clone(),
                        state.cfg.online_token.clone(),
                        play,
                        e.path.clone(),
                        replay.pp_play(),
                    ));
                    // Counted with the play itself.
                    state.playing = None;
                    results.total_due = true;
                    state
                        .online_boards
                        .remove(&(replay.map_hash, game.mods().relax()));
                }
                match scores::save(&replay, pp) {
                    Ok(row) => {
                        results.replay = Some(row.file.clone());
                        state.scores.push(row);
                        let all = scores::top(
                            &state.scores,
                            replay.map_hash,
                            usize::MAX,
                            game.mods().relax(),
                        );
                        let rank = all
                            .iter()
                            .position(|r| r.date == replay.date && r.score == replay.score)
                            .map(|p| p + 1);
                        results.rank = rank.map(|r| (r, all.len()));
                    }
                    Err(e) => eprintln!("couldn't save the replay: {e}"),
                }
            }
            return Some(ResultsScreen::scene(
                Box::new(results),
                self.entry,
                self.from,
            ));
        }
        None
    }
}
