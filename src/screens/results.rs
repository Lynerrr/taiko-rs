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
    /// The map time the play started from (played again from there).
    pub(crate) from: Option<f64>,
}

impl ResultsScreen {
    pub(crate) fn scene(results: Box<Results>, entry: usize, from: Option<f64>) -> Scene {
        Scene::Other(Box::new(ResultsScreen {
            results,
            entry,
            from,
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
        Some((
            state.library.get(self.entry)?.label(),
            tr!(
                "Looking at the results",
                "リザルトを見ています",
                "Смотрит итоги"
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
        Some(self.entry)
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
            KeyCode::KeyR if state.modifiers_ctrl => state.retry_from(self.entry, self.from),
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
            KeyCode::Enter | KeyCode::NumpadEnter => state.retry_from(self.entry, self.from),
            KeyCode::Escape => {
                let path = state.library[self.entry].path.clone();
                state.show_select();
                state.select.select_path(&state.library, &path);
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
        results.draw(canvas);
        None
    }
}

/// Sends a play to the online leaderboards on a background thread.
pub(crate) fn submit_score(
    server: String,
    token: String,
    mut play: online::ScoreSubmit,
    path: PathBuf,
    params: pp::Play,
) -> Receiver<Result<online::Reply, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
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
        let _ = tx.send(online::call(
            &server,
            online::Request::SubmitScore {
                token,
                play: Box::new(play),
            },
        ));
    });
    rx
}

/// "Artist - Title [Version]  1.10x  +HDHR  from 1:23" for a results screen.
pub(crate) fn results_title(m: &beatmap::Meta, from: Option<f64>) -> String {
    let part = from
        .map(|t| format!("  from {}", fmt_time(t)))
        .unwrap_or_default();
    format!("{} - {} [{}]{part}", m.artist, m.title, m.version)
}

/// A results screen's background work (`ui::Detail`): the pp breakdown of `play` on the map at
/// `path`; with a replay file, the whole play's stats from it; with an online score id, its
/// replay fetched from `server` first (and saved, see `scores::save_online`).
/// Whether plays on library map `e` give pp (only ranked, approved and loved ones do, as on
/// osu!), from the mirrors' status else osu!.db's; None when it isn't known. The server has the
/// last word.
pub(crate) fn map_gives_pp(
    e: &Entry,
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
