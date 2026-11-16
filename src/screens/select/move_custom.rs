//! Moving taiko.rs's maps out of osu!'s Songs folder (`custom::migrate`): offered once over
//! song select when some are found there, and any time from Settings > Library, which also
//! moves them back. The moves run in the background; what is keyed by a map's path (when it was
//! last played, the editor's recent maps) follows them, and the song list is read again.

#[allow(unused_imports)]
use crate::*;

use crate::custom::migrate::{self, Found, Moved};
use crate::ui::MoveAsk;

/// What a background look or move brings back.
pub(crate) enum MoveJob {
    /// The look found these (`asked`: from settings, which moves them at once).
    Found(Vec<Found>, bool),
    /// These moved (`back`: to osu!'s Songs folder), and why the rest couldn't.
    Moved(Vec<Moved>, Vec<String>, bool),
}

impl State {
    /// Each frame: once in a game's life, with the song list read, looks for taiko.rs's maps in
    /// osu!'s Songs folder unless that was offered already; then picks up what a look or move
    /// brought.
    pub(crate) fn poll_custom_move(&mut self) {
        if !self.cfg.custom_move_asked
            && !self.custom_move_looked
            && self.scene.is_select()
            && self.scan.is_none()
            && !self.library.is_empty()
            && self.custom_move.is_idle()
        {
            self.custom_move_looked = true;
            self.look_for_custom_maps(false);
        }
        match self.custom_move.poll() {
            Some(MoveJob::Found(found, asked)) => self.custom_maps_found(found, asked),
            Some(MoveJob::Moved(moved, failed, back)) => {
                self.custom_maps_moved(moved, failed, back)
            }
            None => {}
        }
    }

    /// Looks in the background for taiko.rs's maps in the Songs folders. `asked`: from
    /// settings: they're moved as soon as they're found.
    pub(crate) fn look_for_custom_maps(&mut self, asked: bool) {
        if self.custom_move.is_busy() {
            if asked {
                self.toast(
                    tr!(
                        "taiko.rs's maps are being moved already",
                        "taiko.rsの譜面はもう移動中です",
                        "карты taiko.rs уже переносятся"
                    )
                    .into(),
                );
            }
            return;
        }
        let roots = self.song_dirs.clone();
        let custom = self.custom_dir.clone();
        self.custom_move =
            Job::spawn(move || MoveJob::Found(migrate::find(&roots, &custom), asked));
    }

    fn custom_maps_found(&mut self, found: Vec<Found>, asked: bool) {
        if found.is_empty() {
            if asked {
                self.toast(
                    tr!(
                        "none of taiko.rs's maps are in osu!'s Songs folder",
                        "osu!のSongsフォルダにtaiko.rsの譜面はありません",
                        "в папке Songs osu! нет карт taiko.rs"
                    )
                    .into(),
                );
            } else {
                // Nothing to offer: no need to look at every start.
                self.cfg.custom_move_asked = true;
                self.cfg.save();
            }
            return;
        }
        if asked {
            self.start_custom_move(found);
        } else if self.scene.is_select() {
            self.select.moving = Some(MoveAsk {
                sets: found.len(),
                bytes: found.iter().map(|f| f.bytes).sum(),
                found,
                on_move: true,
            });
        }
    }

    /// Moves `found` into taiko.rs's own folder in the background.
    fn start_custom_move(&mut self, mut found: Vec<Found>) {
        // Only a set folder right in one of the Songs folders, never one of them.
        let roots = self.song_dirs.clone();
        found.retain(|f| {
            !roots.contains(&f.folder)
                && f.folder
                    .parent()
                    .is_some_and(|p| roots.iter().any(|r| r == p))
        });
        if found.is_empty() {
            return;
        }
        let n = found.len();
        let word = lang::ru_plural(n, ["сет", "сета", "сетов"]);
        self.toast(tr!(
            "moving {n} taiko.rs sets out of osu!'s Songs folder...",
            "taiko.rsのセット{n}個をosu!のSongsフォルダから移動中...",
            "переносим {n} {word} taiko.rs из папки Songs osu!..."; [] [n, word]
        ));
        let custom = self.custom_dir.clone();
        self.custom_move = Job::spawn(move || {
            let (moved, failed) = migrate::move_all(&found, &roots, &custom);
            MoveJob::Moved(moved, failed, false)
        });
    }

    /// Settings > Library: moves what was moved back to osu!'s Songs folder.
    pub(crate) fn move_custom_maps_back(&mut self) {
        if self.custom_move.is_busy() {
            self.look_for_custom_maps(true);
            return;
        }
        if migrate::moved_count(&self.custom_dir) == 0 {
            self.toast(
                tr!(
                    "nothing was moved out of osu!'s Songs folder",
                    "osu!のSongsフォルダから移動したものはありません",
                    "из папки Songs osu! ничего не переносилось"
                )
                .into(),
            );
            return;
        }
        let custom = self.custom_dir.clone();
        self.custom_move = Job::spawn(move || {
            let (moved, failed) = migrate::move_back(&custom);
            MoveJob::Moved(moved, failed, true)
        });
    }

    /// The moves are done: what is kept by a map's path follows it, and the list is read again.
    fn custom_maps_moved(&mut self, moved: Vec<Moved>, failed: Vec<String>, back: bool) {
        let pairs: Vec<(PathBuf, PathBuf)> = moved
            .iter()
            .map(|m| (m.from.clone(), m.to.clone()))
            .collect();
        if !pairs.is_empty() {
            store::with(|s| s.move_plays(&pairs));
            let played = std::mem::take(&mut self.select.played);
            self.select.played = played
                .into_iter()
                .map(|(p, at)| (migrate::moved_path(&p, &moved).unwrap_or(p), at))
                .collect();
            let mut recent_moved = false;
            for p in &mut self.cfg.editor_recent {
                if let Some(new) = migrate::moved_path(Path::new(p.as_str()), &moved) {
                    *p = new.to_string_lossy().into_owned();
                    recent_moved = true;
                }
            }
            if recent_moved {
                self.cfg.save();
            }
            // Where the sets are has changed under the website's answers: asked again.
            for (from, _) in &pairs {
                self.select.custom_folder.remove(from);
            }
            self.custom_asked = None;
            self.installed_sets = None;
            let scanning = self.scan.is_some();
            self.refresh_library(false);
            if !scanning {
                // Its "N new, N gone" would only say the move again, less clearly.
                self.refreshing = None;
            }
        }
        let n = moved.len();
        let word = lang::ru_plural(n, ["сет", "сета", "сетов"]);
        if back {
            self.toast(tr!(
                "moved {n} sets back to osu!'s Songs folder",
                "{n}個のセットをosu!のSongsフォルダに戻しました",
                "{n} {word} возвращено в папку Songs osu!"; [] [n, word]
            ));
        } else if n > 0 {
            self.toast(tr!(
                "moved {n} sets into the taiko.rs maps folder",
                "{n}個のセットをtaiko.rsの譜面フォルダに移動しました",
                "{n} {word} перенесено в папку карт taiko.rs"; [] [n, word]
            ));
        }
        if let Some(first) = failed.first() {
            let k = failed.len();
            self.toast(tr!(
                "{k} couldn't move and stay where they are ({first})",
                "{k}個は移動できず、そのままです ({first})",
                "не перенеслось: {k}, они остались на месте ({first})"; [] [k, first]
            ));
        }
    }

    /// Keys while the move is offered: Left / Right (or Tab) choose between Not now and Move
    /// (picked at first), Enter or Space does it, Esc is Not now.
    pub(crate) fn move_ask_key(&mut self, code: KeyCode, repeat: bool) {
        let Some(ask) = &mut self.select.moving else {
            return;
        };
        if repeat {
            return;
        }
        match code {
            KeyCode::Escape | KeyCode::Backspace => self.decline_custom_move(),
            KeyCode::ArrowLeft => ask.on_move = false,
            KeyCode::ArrowRight => ask.on_move = true,
            KeyCode::Tab | KeyCode::ArrowUp | KeyCode::ArrowDown => ask.on_move = !ask.on_move,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if ask.on_move {
                    self.confirm_custom_move();
                } else {
                    self.decline_custom_move();
                }
            }
            _ => {}
        }
    }

    /// The offer's Move.
    pub(crate) fn confirm_custom_move(&mut self) {
        let Some(ask) = self.select.moving.take() else {
            return;
        };
        self.cfg.custom_move_asked = true;
        self.cfg.save();
        self.start_custom_move(ask.found);
    }

    /// The offer's Not now: it isn't offered again; settings have it.
    fn decline_custom_move(&mut self) {
        self.select.moving = None;
        self.cfg.custom_move_asked = true;
        self.cfg.save();
        self.toast(
            tr!(
                "Settings > Library can move them any time",
                "設定 > ライブラリからいつでも移動できます",
                "Настройки > Библиотека перенесут их в любой момент"
            )
            .into(),
        );
    }
}

/// The offer over song select: how many sets and how big, where they go, Not now beside Move
/// (picked at first).
pub(crate) fn draw_move_ask(c: &mut Canvas, ask: &MoveAsk) {
    c.hits.clear();
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
    let bw = (w - 32.0).min(480.0);
    let (bx, inner) = ((w - bw) / 2.0, bw - 32.0);
    let (top, bh) = (128.0, 212.0);
    c.round_rect(
        [bx, top],
        [bw, bh],
        10.0,
        rgba(style::PANEL, 0.98),
        rgba(style::PANEL_EDGE, 1.0),
        1.5,
    );
    let title = tr!(
        "Move taiko.rs maps out of osu!'s Songs folder?",
        "taiko.rsの譜面をosu!のSongsフォルダから移動しますか？",
        "Перенести карты taiko.rs из папки Songs osu!?"
    );
    let size = Canvas::fit_size(14.0, 10.0, inner, title);
    c.text_centered(
        w / 2.0,
        top + 16.0,
        size,
        rgba(0xffffff, 1.0),
        &Canvas::fit(size, inner, title),
    );
    let n = ask.sets;
    let size = if ask.bytes >= 1_000_000 {
        format!("{:.1} MB", ask.bytes as f64 / 1e6)
    } else {
        format!("{} KB", ask.bytes.div_ceil(1000).max(1))
    };
    let word = lang::ru_plural(n, ["сет", "сета", "сетов"]);
    let what = tr!(
        "{n} taiko.rs sets ({size}) are in osu!'s Songs folder, where osu! lists them as its own.",
        "taiko.rsのセット{n}個 ({size}) がosu!のSongsフォルダにあり、osu!が自分の譜面として表示しています。",
        "{n} {word} taiko.rs ({size}) лежат в папке Songs osu!, и osu! показывает их как свои."; [] [n, size, word]
    );
    let line = |c: &mut Canvas, y: f32, colour: u32, text: &str| {
        let size = Canvas::fit_size(7.0, 5.0, inner, text);
        c.text_centered(
            w / 2.0,
            y,
            size,
            rgba(colour, 1.0),
            &Canvas::fit(size, inner, text),
        );
    };
    line(c, top + 50.0, 0xffffff, &what);
    line(
        c,
        top + 72.0,
        0xc8c8d8,
        tr!(
            "They go to the taiko.rs maps folder. Scores, replays and favourites stay.",
            "taiko.rsの譜面フォルダに移ります。スコア、リプレイ、お気に入りは残ります。",
            "Они уйдут в папку карт taiko.rs. Рекорды, реплеи и избранное остаются."
        ),
    );
    line(
        c,
        top + 88.0,
        0x9a9ab0,
        tr!(
            "osu!'s own sets stay where they are. Settings > Library can move them back.",
            "osu!のセットはそのままです。設定 > ライブラリから戻せます。",
            "Сеты osu! остаются на месте. Настройки > Библиотека вернут всё обратно."
        ),
    );
    line(
        c,
        top + 108.0,
        0x9a9ab0,
        tr!(
            "Close osu! first for a clean move.",
            "きれいに移動するには先にosu!を閉じてください。",
            "Для чистого переноса сначала закройте osu!."
        ),
    );
    let by = top + 148.0;
    c.button(
        w / 2.0 - 110.0,
        by,
        100.0,
        24.0,
        tr!("Not now", "今はしない", "Не сейчас"),
        9.0,
        hit::Hit::Key(KeyCode::Escape),
        !ask.on_move,
    );
    c.button(
        w / 2.0 + 10.0,
        by,
        100.0,
        24.0,
        tr!("Move", "移動", "Перенести"),
        9.0,
        hit::Hit::MoveCustomMaps,
        ask.on_move,
    );
    if !mobile::active() {
        line(
            c,
            top + bh - 16.0,
            0x6f6f86,
            tr!(
                "Left/Right choose   Enter   Esc not now",
                "左右で選択   Enter   Escで今はしない",
                "Влево/вправо выбор   Enter   Esc не сейчас"
            ),
        );
    }
}
