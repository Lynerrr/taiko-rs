//! The results screen, and its work: its title, whether the map gives pp, submitting the
//! score and fetching a replay to show.

use super::screen::{Frame, Key, Screen};
use crate::*;

/// A play's results: the score, its numbers, and keys to play again, watch the replay or
/// make a video of it. Also a score opened from a leaderboard or a link (`results.opened`).
pub(crate) struct ResultsScreen {
    pub(crate) results: Box<Results>,
    /// The map played.
    pub(crate) entry: usize,
    /// The map is no longer in the library (e.g. Songs folder changed in Settings).
    pub(crate) gone: bool,
    /// The map time the play started from (played again from there).
    pub(crate) from: Option<f64>,
    /// Journey's Non-stop holds on these results: the player is reading them.
    pub(crate) held: bool,
}

impl ResultsScreen {
    pub(crate) fn scene(results: Box<Results>, entry: usize, from: Option<f64>) -> Scene {
        Scene::Other(Box::new(ResultsScreen {
            results,
            entry,
            gone: false,
            from,
            held: false,
        }))
    }
}

impl Screen for ResultsScreen {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Results"
    }

    fn activity(&self) -> &'static str {
        "results"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        if self.gone {
            return None;
        }
        Some((
            state.library.get(self.entry)?.label(),
            super::play::journey_presence(
                state,
                tr!(
                    "Looking at the results",
                    "リザルトを見ています",
                    "Смотрит итоги"
                )
                .into(),
            ),
        ))
    }

    fn presence_first(&self) -> bool {
        true
    }

    fn top_bar(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn idle_allowed(&self) -> bool {
        true
    }

    fn allows_settings(&self) -> bool {
        true
    }

    fn menu_screen(&self) -> Option<u8> {
        Some(2)
    }

    fn finger_scrolls(&self) -> bool {
        true
    }

    fn map_entry(&self) -> Option<usize> {
        (!self.gone).then_some(self.entry)
    }

    fn key_place(&self) -> Option<keys::Place> {
        Some(keys::Place::Results)
    }

    fn shows_own_background(&self) -> bool {
        true
    }

    fn opens_score_links(&self) -> bool {
        true
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
        let results = &mut self.results;
        // Non-stop: looking through the results holds them up; Esc stops the run of maps.
        if !matches!(
            code,
            KeyCode::Enter
                | KeyCode::NumpadEnter
                | KeyCode::Escape
                | KeyCode::KeyL
                | KeyCode::KeyN
                | KeyCode::KeyS
        ) {
            self.held = true;
        }
        match code {
            KeyCode::ArrowDown
            | KeyCode::ArrowUp
            | KeyCode::PageDown
            | KeyCode::PageUp
            | KeyCode::Home
            | KeyCode::End => {
                results.scroll(matches!(
                    code,
                    KeyCode::ArrowDown | KeyCode::PageDown | KeyCode::End
                ));
            }
            KeyCode::KeyR if state.modifiers_ctrl => {
                if !self.gone && self.entry < state.library.len() {
                    state.retry_from(self.entry, self.from);
                }
            }
            KeyCode::KeyL | KeyCode::KeyN if results.vote_on.is_some() => {
                let vote = if code == KeyCode::KeyL {
                    online::MapVote::Like
                } else {
                    online::MapVote::Dislike
                };
                if let Some((map, vote)) = results.toggle_vote(vote) {
                    state.send_vote(map, vote);
                    if vote == Some(online::MapVote::Like)
                        && state.playing_journey()
                        && state.cfg.journey_save_on_like
                        && state.journey.playing_cached.is_some()
                    {
                        state.journey_save_current();
                    }
                }
            }
            KeyCode::KeyS if state.playing_journey() => {
                state.journey_save_current();
            }
            KeyCode::KeyV if results.replay.is_some() => {
                if let Some(file) = results.replay.clone() {
                    state.open_video_editor(&file);
                }
            }
            // A leaderboard score's results: Enter watches it too.
            _ if results.replay.is_some()
                && (code == KeyCode::KeyR
                    || (results.opened.is_some()
                        && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter))) =>
            {
                if let Some(file) = results.replay.clone() {
                    state.watch_replay(&file);
                }
            }
            // Back to song select, the music playing on.
            _ if results.opened.is_some() => {
                if matches!(code, KeyCode::Escape | KeyCode::Backspace) {
                    state.scene = SongSelect::scene();
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if state.playing_journey() {
                    state.journey_next_from_results(results.failed);
                    state.journey.autostart =
                        state.cfg.journey_flow == crate::config::JourneyFlow::Nonstop;
                } else if !self.gone && self.entry < state.library.len() {
                    state.retry_from(self.entry, self.from);
                } else {
                    state.show_select();
                }
            }
            KeyCode::Escape => {
                // The map was played: leaving its results goes on with Journey, as Next does,
                // so it isn't left up to be skipped.
                if state.playing_journey() {
                    state.journey_next_from_results(results.failed);
                } else {
                    let path = (!self.gone)
                        .then(|| state.library.get(self.entry).map(|e| e.path.clone()))
                        .flatten();
                    state.show_select();
                    if let Some(path) = path {
                        state.select.select_path(&state.library, &path);
                    }
                }
            }
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let results = &mut self.results;
        results.poll();
        results.animate(canvas.dt_ms);
        // Back to the preview point when the song ends, like osu!.
        if let (Some((track, again)), Some(a)) = (&results.music, &state.audio) {
            let end = track.frames() as f64 / a.rate as f64 * 1000.0;
            if a.position_ms(frame.now).is_some_and(|p| p > end - 30.0) {
                a.play(track.clone(), *again);
            }
        }
        results.journey = state.playing_journey();
        results.journey_save = state.journey.playing_cached.is_some();
        results.draw(canvas);
        if results.journey
            && state.cfg.journey_flow == crate::config::JourneyFlow::Nonstop
            && !self.held
            && results.opened.is_none()
        {
            let left =
                NONSTOP_RESULTS.saturating_sub(frame.now.saturating_duration_since(results.shown));
            if left.is_zero() {
                let failed = results.failed;
                state.journey_next_from_results(failed);
                state.journey.autostart = true;
                return None;
            }
            let secs = left.as_secs_f32().ceil() as u32;
            let line = tr!(
                "Next map in {secs}...   Enter: now   Esc: stop",
                "{secs}秒後に次の譜面...   Enter: 今すぐ   Esc: 止める",
                "Следующая карта через {secs}...   Enter: сейчас   Esc: стоп";
                [] [secs]
            );
            canvas.text_centered(
                canvas.width / 2.0,
                canvas.height - 16.0,
                8.0,
                crate::render::rgba(0x22d4c0, 1.0),
                &line,
            );
        }
        None
    }
}

/// How long Journey's Non-stop shows a play's results before the next map.
const NONSTOP_RESULTS: std::time::Duration = std::time::Duration::from_secs(5);

/// A play sent to the online leaderboards: what the server answered, or, when it never did or
/// said to try later (`online::try_later`), the play to send again (`score_outbox`).
pub(crate) enum Sent {
    Answered {
        reply: Result<online::Reply, String>,
        key: Option<String>,
        /// Sent again from the outbox, not from the results just shown.
        again: bool,
    },
    Unanswered {
        error: String,
        unsent: Box<score_outbox::Unsent>,
        again: bool,
    },
}

/// Sends a play to the online leaderboards on a background thread, with a key of its own so
/// it can go again should the server not answer.
pub(crate) fn submit_score(
    server: String,
    token: String,
    player: String,
    mut play: online::ScoreSubmit,
    path: PathBuf,
    params: pp::Play,
) -> Receiver<Sent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if crate::tja_maps::is_course(&path) {
            // A TJA chart's board checks the play on its chart, sent along: no file, no stars.
            play.tja = crate::tja_maps::board(&path);
        } else {
            play.stars = stars::star_rating_played(
                &path,
                params.rate,
                params.mods,
                params.scroll_speed,
                params.seed,
                params.half,
            );
            play.md5 = std::fs::read(&path)
                .ok()
                .map(|bytes| download::md5_hex(&bytes));
        }
        play.operation_key.get_or_insert_with(score_outbox::new_key);
        let unsent = score_outbox::Unsent {
            server,
            player,
            queued: score_outbox::unix_now(),
            play,
        };
        let _ = tx.send(send_play(unsent, &token, false));
    });
    rx
}

/// Sends `unsent` again, on a background thread.
pub(crate) fn resend_score(unsent: score_outbox::Unsent, token: String) -> Receiver<Sent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(send_play(unsent, &token, true));
    });
    rx
}

fn send_play(unsent: score_outbox::Unsent, token: &str, again: bool) -> Sent {
    let key = unsent.play.operation_key.clone();
    let request = online::Request::SubmitScore {
        token: token.to_string(),
        play: Box::new(unsent.play.clone()),
    };
    match online::exchange(&unsent.server, request) {
        // Not taken just now (the server's database was out of reach): kept like a play it
        // never answered, so the same key goes again.
        Ok((online::Reply::Error(error), _)) if online::try_later(&error) => Sent::Unanswered {
            error,
            unsent: Box::new(unsent),
            again,
        },
        Ok((online::Reply::Error(e), _)) => Sent::Answered {
            reply: Err(e),
            key,
            again,
        },
        Ok((reply, _)) => Sent::Answered {
            reply: Ok(reply),
            key,
            again,
        },
        Err(error) => Sent::Unanswered {
            error,
            unsent: Box::new(unsent),
            again,
        },
    }
}

/// "Artist - Title [Version]  1.10x  +HDHR  from 1:23" for a results screen.
pub(crate) fn results_title(m: &beatmap::Meta, from: Option<f64>) -> String {
    let part = from
        .map(|t| format!("  from {}", fmt_time(t)))
        .unwrap_or_default();
    // A TJA chart says so: its scores stay on this computer.
    let tja = if matches!(m.windows, beatmap::WindowSource::Arcade { .. }) {
        "  · TJA"
    } else {
        ""
    };
    format!("{} - {} [{}]{part}{tja}", m.artist, m.title, m.version)
}

/// A results screen's background work (`ui::Detail`): the pp breakdown of `play` on the map at
/// `path`; with a replay file, the whole play's stats from it; with an online score id, its
/// replay fetched from `server` first (and saved, see `scores::save_online`).
/// Whether plays on library map `e` give pp (only ranked, approved and loved ones do, as on
/// osu!): the online server's verdict (`pp_verdict`), else from the mirrors' status, else
/// osu!.db's; None when it isn't known.
pub(crate) fn map_gives_pp(
    e: &Entry,
    pp_verdict: &std::collections::HashMap<u32, bool>,
    set_status: &std::collections::HashMap<u32, Option<String>>,
    db_status: &std::collections::HashMap<PathBuf, u8>,
    server_loved: &ui::ServerLoved,
) -> Option<bool> {
    let Some(set) = e.set_id() else {
        return Some(false);
    };
    // Loved on the online server (the set, or this difficulty): it gives pp there.
    if ui::loved_here(server_loved, e) {
        return Some(true);
    }
    if let Some(&gives) = pp_verdict.get(&set) {
        return Some(gives);
    }
    if let Some(Some(s)) = set_status
        .get(&set)
        .filter(|s| s.as_deref() != Some(setstatus::MISSING))
    {
        return Some(matches!(s.as_str(), "ranked" | "approved" | "loved"));
    }
    db_status
        .get(&e.path)
        .filter(|&&s| s != 0)
        .map(|s| matches!(s, 4 | 5 | 7))
}

pub(crate) fn results_job(
    path: PathBuf,
    play: pp::Play,
    local: Option<PathBuf>,
    online: Option<(String, online::OnlineScore, String)>,
) -> Receiver<ui::Detail> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut d = ui::Detail {
            breakdown: pp::breakdown(&path, &play),
            ..Default::default()
        };
        let replay = match (local, online) {
            (Some(file), _) => scores::load_replay(&file).ok(),
            (None, Some((server, score, label))) => {
                match fetch_replay(&server, &score, &path, label) {
                    Ok((file, r)) => {
                        d.replay = Some(Ok(file));
                        Some(r)
                    }
                    Err(e) => {
                        d.replay = Some(Err(e));
                        None
                    }
                }
            }
            (None, None) => None,
        };
        if let Some(r) = &replay {
            d.seed = Some(r.seed);
            if r.seed != play.seed {
                d.breakdown = pp::breakdown(&path, &r.pp_play());
            }
        }
        d.stats = replay.and_then(|r| scores::simulate(&path, &r));
        let _ = tx.send(d);
    });
    rx
}

/// Gets an online score's replay from the server and keeps it as a replay file of our own.
fn fetch_replay(
    server: &str,
    score: &online::OnlineScore,
    map_path: &std::path::Path,
    map_label: String,
) -> Result<(PathBuf, scores::Replay), String> {
    if score.id == 0 {
        return Err(tr!(
            "the server doesn't keep replays yet",
            "このサーバーはまだリプレイを保存していません",
            "сервер пока не хранит повторы"
        )
        .into());
    }
    if let Some(file) = scores::online_replay_path(score.id).filter(|f| f.is_file())
        && let Ok(r) = scores::load_replay(&file)
    {
        return Ok((file, r));
    }
    let got = match online::call(server, online::Request::Replay { id: score.id }) {
        Ok(online::Reply::Replay(r)) => *r,
        Ok(_) => return Err(crate::lang::odd_reply()),
        // Servers from before replays don't know the request.
        Err(e) if e == "bad request" => {
            return Err(tr!(
                "the server doesn't hand out replays yet",
                "このサーバーはまだリプレイを配っていません",
                "сервер пока не отдаёт повторы"
            )
            .into());
        }
        Err(e) => return Err(e),
    };
    if got.version > scores::new_version() {
        return Err(tr!(
            "replay from a newer version",
            "新しいバージョンのリプレイです",
            "повтор из более новой версии"
        )
        .into());
    }
    let replay = scores::Replay {
        version: got.version,
        map_hash: score.map_hash,
        map_path: map_path.to_string_lossy().into_owned(),
        map_label,
        player: score.player.clone(),
        date: score.date,
        rate: score.rate,
        keep_pitch: true,
        anti_mash: true,
        mods: score.mods.clone(),
        scroll_speed: score.scroll_speed.unwrap_or(1.0),
        score: score.score,
        accuracy: score.accuracy,
        max_combo: score.max_combo,
        great: score.great,
        ok: score.ok,
        miss: score.miss,
        ur: 0.0,
        inputs: got.inputs,
        seed: got.map.seed.unwrap_or(0),
    };
    let file = scores::save_online(score.id, &replay)?;
    Ok((file, replay))
}
