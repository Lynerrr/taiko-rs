//! The Journey screen: an endless, level-and-taste-matched map feed drawn in the client's
//! own visual language (`style.rs`): a mini session lane across the top, one focused map card
//! in the middle, and a Song-Select-style action bar along the bottom.

use super::screen::{Frame, Key, Screen};
use crate::journey::{LEVEL_CHOICES, MapSource, PastOutcome};
use crate::*;

pub(crate) struct JourneyScreen;

impl JourneyScreen {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(JourneyScreen))
    }
}

impl Screen for JourneyScreen {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Journey"
    }

    fn activity(&self) -> &'static str {
        "journey"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        let map = state
            .journey
            .current()
            .map(|q| format!("{} - {} [{}]", q.item.artist, q.item.title, q.item.version))
            .unwrap_or_else(|| "Choosing next map".into());
        Some((
            map,
            tr!("On a Journey", "ジャーニー中", "В режиме Journey").into(),
        ))
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
                KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight
            )
        {
            return;
        }
        if state.handle_global_key(code, window) {
            return;
        }

        if state.journey.needs_level {
            match code {
                KeyCode::ArrowUp | KeyCode::KeyW => {
                    state.journey.level_cursor = state.journey.level_cursor.saturating_sub(1);
                }
                KeyCode::ArrowDown | KeyCode::KeyS => {
                    state.journey.level_cursor =
                        (state.journey.level_cursor + 1).min(LEVEL_CHOICES.len() - 1);
                }
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let idx = state.journey.level_cursor;
                    state.journey_pick_level(idx);
                }
                KeyCode::Escape => {
                    state.exit_journey();
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Escape => state.exit_journey(),
            KeyCode::Enter | KeyCode::NumpadEnter => state.journey_play_current(),
            KeyCode::Tab | KeyCode::ArrowRight => state.journey_skip_current(),
            KeyCode::KeyS => state.journey_save_current(),
            KeyCode::F1 => state.open_mods(),
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        state.journey_tick();

        style::ground(canvas, 1.0);
        let w = canvas.width;
        let h = render::VIRTUAL_HEIGHT;

        if state.journey.needs_level {
            draw_level_picker(state, canvas, w, h);
            return None;
        }

        draw_top_lane(state, canvas, w);
        draw_center_card(state, canvas, w, h);
        None
    }
}

fn draw_level_picker(state: &State, c: &mut Canvas, w: f32, h: f32) {
    let pw = 300.0_f32.min(w - 32.0);
    let row_h = 30.0;
    let ph = 86.0 + LEVEL_CHOICES.len() as f32 * row_h + 26.0;
    let x = (w - pw) / 2.0;
    let y = (h - ph) / 2.0;

    style::panel(c, x, y, pw, ph);
    let heading = tr!("START THE JOURNEY", "ジャーニーを始める", "НАЧАТЬ JOURNEY");
    let tw = Canvas::text_width(12.0, heading);
    style::title(c, x + (pw - tw) / 2.0, y + 16.0, 12.0, 0xffffff, heading);

    let sub = tr!(
        "What star rating feels comfortable?",
        "ちょうどいい難易度はどのくらい？",
        "Какая сложность (звёзды) комфортна?"
    );
    let sub = Canvas::fit(8.0, pw - 28.0, sub);
    c.text_centered(x + pw / 2.0, y + 40.0, 8.0, rgba(0xa8a8bc, 1.0), &sub);
    style::rule(c, x + 16.0, y + 58.0, pw - 32.0);

    let rx = x + 16.0;
    let rw = pw - 32.0;
    for (i, (label, _)) in LEVEL_CHOICES.iter().enumerate() {
        let ry = y + 68.0 + i as f32 * row_h;
        let rh = row_h - 4.0;
        let selected = i == state.journey.level_cursor;
        let hovered = c.hovered(rx, ry, rw, rh);
        if selected {
            c.round_rect(
                [rx, ry],
                [rw, rh],
                6.0,
                rgba(0x22c8b8, 0.16),
                rgba(0, 0.0),
                0.0,
            );
            style::selector(c, "journey-lvl", [rx - 2.0, ry - 2.0, rw + 4.0, rh + 4.0]);
        } else if hovered {
            c.round_rect(
                [rx, ry],
                [rw, rh],
                6.0,
                rgba(0x262634, 0.7),
                rgba(0, 0.0),
                0.0,
            );
        }
        c.hit(rx, ry, rw, rh, hit::Hit::JourneyLevel(i));
        let col = if selected || hovered {
            0xffffff
        } else {
            0xb4b4c8
        };
        c.text_centered(
            rx + rw / 2.0,
            ry + (rh - 10.0) / 2.0,
            10.0,
            rgba(col, 1.0),
            label,
        );
    }

    let hint = tr!(
        "Up/Down choose   Enter start   Esc back",
        "上下で選択   Enter 開始   Esc 戻る",
        "Вверх/Вниз выбор   Enter старт   Esc назад"
    );
    c.text_centered(
        x + pw / 2.0,
        y + ph - 18.0,
        7.0,
        rgba(0x787890, 1.0),
        &Canvas::fit(7.0, pw - 24.0, hint),
    );
}

fn draw_top_lane(state: &State, c: &mut Canvas, w: f32) {
    let y = 34.0;
    let h = 42.0;
    let x = 24.0;
    let pw = w - 48.0;
    style::cold_panel(c, [x, y, pw, h]);

    let back = tr!("<- Exit", "<- 終了", "<- Выход");
    let bw = Canvas::text_width(8.0, back) + 22.0;
    c.button(
        x + 10.0,
        y + 10.0,
        bw,
        22.0,
        back,
        8.0,
        hit::Hit::Key(KeyCode::Escape),
        false,
    );

    let mode_tag = if state.journey.local_mode {
        let local = tr!("LOCAL", "ローカル", "ЛОКАЛЬНО");
        format!("~{:.1}*  {local}", state.journey.comfort_stars)
    } else {
        format!("~{:.1}*  JOURNEY", state.journey.comfort_stars)
    };
    let mw = Canvas::text_width(8.0, &mode_tag);
    c.text(
        x + pw - 14.0 - mw,
        y + 16.0,
        8.0,
        rgba(0x22c8b8, 0.95),
        &mode_tag,
    );

    let lane_left = x + 10.0 + bw + 18.0;
    let lane_right = x + pw - 24.0 - mw;
    if lane_right - lane_left < 120.0 {
        return;
    }
    let mid_y = y + h / 2.0;
    c.rect(
        lane_left,
        mid_y - 1.0,
        lane_right - lane_left,
        2.0,
        rgba(0x2a2a3c, 1.0),
    );

    let center_x = (lane_left + lane_right) / 2.0;
    for (k, past) in state.journey.history.iter().rev().take(5).enumerate() {
        let nx = center_x - 36.0 - k as f32 * 24.0;
        if nx < lane_left + 10.0 {
            break;
        }
        let col = match past.outcome {
            PastOutcome::Passed => 0x5ee07a,
            PastOutcome::Failed => 0xff5a5a,
            PastOutcome::Skipped => 0x5a5a70,
        };
        c.circle(nx, mid_y, 6.0, rgba(col, 0.85), rgba(0xffffff, 0.45), 1.0);
    }

    let (role_label, role_col) = state
        .journey
        .current()
        .map(|q| role_badge(&q.item.role))
        .unwrap_or(("LOADING", 0x787890));
    c.circle(
        center_x,
        mid_y,
        9.5,
        rgba(role_col, 1.0),
        rgba(0xffffff, 0.9),
        1.5,
    );
    style::selector(
        c,
        "journey-now",
        [center_x - 13.0, mid_y - 13.0, 26.0, 26.0],
    );

    for (k, next_q) in state.journey.queue.iter().skip(1).take(2).enumerate() {
        let nx = center_x + 36.0 + k as f32 * 28.0;
        if nx > lane_right - 14.0 {
            break;
        }
        let col = if next_q.source.is_ready() {
            0x8a8aa8
        } else {
            0x44445c
        };
        c.circle(nx, mid_y, 6.5, rgba(col, 0.85), rgba(0xffffff, 0.35), 1.0);
    }

    let rx = center_x + 96.0;
    if rx + Canvas::text_width(7.0, role_label) < lane_right {
        c.text(rx, mid_y - 3.5, 7.0, rgba(role_col, 0.95), role_label);
    }
}

fn draw_center_card(state: &State, c: &mut Canvas, w: f32, h: f32) {
    let cw = (w * 0.68).clamp(440.0, 680.0).min(w - 48.0);
    let cx = (w - cw) / 2.0;
    let cy = 92.0;
    let actions_h = 36.0;
    let ch = (h - cy - actions_h - 32.0).max(220.0);

    style::cold_panel(c, [cx, cy, cw, ch]);

    let Some(cur) = state.journey.current() else {
        let msg = if let Some(err) = &state.journey.error {
            err.as_str()
        } else if state.journey.fetching.is_some() {
            tr!(
                "Finding your next map...",
                "次の譜面を探しています...",
                "Подбираем следующую карту..."
            )
        } else {
            tr!(
                "No more maps in the current pool.",
                "これ以上候補の譜面がありません。",
                "В пуле закончились подходящие карты."
            )
        };
        c.text_centered(
            cx + cw / 2.0,
            cy + ch / 2.0 - 6.0,
            10.0,
            rgba(0xb4b4c8, 1.0),
            &Canvas::fit(10.0, cw - 40.0, msg),
        );
        return;
    };

    let item = &cur.item;
    let (role_label, role_col) = role_badge(&item.role);

    let pad = 20.0;
    let inner_w = cw - pad * 2.0;
    let mut ty = cy + 18.0;

    let reason_line = if item
        .reason
        .to_ascii_lowercase()
        .starts_with(&role_label.to_ascii_lowercase())
    {
        item.reason.clone()
    } else {
        format!("{role_label}  ·  {}", item.reason)
    };
    c.text(
        cx + pad,
        ty,
        8.0,
        rgba(role_col, 1.0),
        &Canvas::fit(8.0, inner_w - 130.0, &reason_line),
    );

    let source_text = if item.custom_id.is_some() {
        tr!("taiko.rs custom", "taiko.rsカスタム", "кастомка taiko.rs")
    } else if state.journey.local_mode {
        tr!("library", "ライブラリ", "библиотека")
    } else {
        "osu!"
    };
    let stars_text = if item.stars > 0.0 {
        format!("*{:.2}", item.stars)
    } else {
        "*-".into()
    };
    let sw = Canvas::text_width(9.0, &stars_text);
    c.text(
        cx + cw - pad - sw,
        ty - 1.0,
        9.0,
        rgba(0xffd34d, 1.0),
        &stars_text,
    );
    let src_w = Canvas::text_width(7.5, source_text);
    c.text(
        cx + cw - pad - sw - 14.0 - src_w,
        ty + 1.0,
        7.5,
        rgba(0x8888a4, 1.0),
        source_text,
    );

    ty += 24.0;
    let title = Canvas::fit(18.0, inner_w, &item.title);
    style::title(c, cx + pad, ty, 18.0, 0xffffff, &title);
    ty += 28.0;

    let artist = Canvas::fit(11.0, inner_w, &item.artist);
    c.text(cx + pad, ty, 11.0, rgba(0xc0c0d4, 1.0), &artist);
    ty += 20.0;

    let diff_line = format!("[{}]  by {}", item.version, item.creator);
    c.text(
        cx + pad,
        ty,
        9.0,
        rgba(0x90b4e8, 1.0),
        &Canvas::fit(9.0, inner_w, &diff_line),
    );
    ty += 22.0;

    style::rule(c, cx + pad, ty, inner_w);
    ty += 16.0;

    let bpm_str = if item.bpm > 0.0 {
        format!("{:.0}", item.bpm)
    } else {
        "-".into()
    };
    let len_str = if item.length_ms > 0 {
        let secs = item.length_ms / 1000;
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        "-".into()
    };
    let star_stat = if item.stars > 0.0 {
        format!("{:.2}", item.stars)
    } else {
        "-".into()
    };
    let status_str = match &cur.source {
        MapSource::Local(_) => tr!("In library", "ライブラリ済み", "В библиотеке").to_string(),
        MapSource::Saved(_) => tr!("Saved to library", "保存済み", "Сохранено").to_string(),
        MapSource::Cached { .. } => tr!(
            "Ready (temp cache)",
            "準備完了 (キャッシュ)",
            "Готово (кэш)"
        )
        .to_string(),
        MapSource::Pending => tr!("Queued", "待機中", "В очереди").to_string(),
        MapSource::Downloading(job) => {
            let pct = (job.fraction().unwrap_or(0.0) * 100.0).round() as u32;
            tr!(
                "Downloading {pct}%...",
                "ダウンロード中 {pct}%...",
                "Загрузка {pct}%...";
                [] [pct]
            )
        }
        MapSource::Failed(e) => {
            tr!("Error: {e}", "エラー: {e}", "Ошибка: {e}"; [] [e])
        }
    };
    let stats = [
        ("BPM", bpm_str),
        (tr!("LENGTH", "長さ", "ДЛИНА"), len_str),
        (tr!("STARS", "星", "ЗВЁЗДЫ"), star_stat),
        (tr!("STATUS", "状態", "СТАТУС"), status_str),
    ];
    let col_w = inner_w / 4.0;
    for (i, (k, v)) in stats.iter().enumerate() {
        let sx = cx + pad + i as f32 * col_w;
        c.text(sx, ty, 7.0, rgba(0x787892, 1.0), k);
        c.text(
            sx,
            ty + 13.0,
            9.5,
            rgba(0xe8e8f4, 1.0),
            &Canvas::fit(9.5, col_w - 8.0, v),
        );
    }
    ty += 42.0;

    if let Some(p) = cur.source.download_progress() {
        c.round_rect(
            [cx + pad, ty],
            [inner_w, 6.0],
            3.0,
            rgba(0x222232, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        c.round_rect(
            [cx + pad, ty],
            [inner_w * p.clamp(0.03, 1.0), 6.0],
            3.0,
            rgba(0x22c8b8, 1.0),
            rgba(0, 0.0),
            0.0,
        );
    }

    let save_dest = state.cfg.journey_save_to.name();
    let save_info = tr!(
        "Save moves cached maps to: {save_dest} (Settings > Library)",
        "保存先: {save_dest} (設定 > ライブラリ)",
        "Сохранение переносит карту в: {save_dest} (Настройки > Библиотека)";
        [] [save_dest]
    );
    c.text(
        cx + pad,
        cy + ch - 34.0,
        7.0,
        rgba(0x74748c, 1.0),
        &Canvas::fit(7.0, inner_w, &save_info),
    );
    let hints = tr!(
        "Enter: Play   Tab / ->: Skip   S: Save   F1: Mods   Esc: Exit",
        "Enter: プレイ   Tab / ->: スキップ   S: 保存   F1: Mod   Esc: 終了",
        "Enter: Играть   Tab / ->: Скип   S: Сохранить   F1: Моды   Esc: Выход"
    );
    c.text(
        cx + pad,
        cy + ch - 18.0,
        7.0,
        rgba(0x8c8ca8, 1.0),
        &Canvas::fit(7.0, inner_w, hints),
    );

    let ay = cy + ch + 10.0;
    let mid = ay + actions_h / 2.0;
    let mods_word = tr!("Mods", "Mod", "Моды");
    let mods_w = style::block_button(
        c,
        [cx + 6.0, mid],
        &style::SLIDERS,
        mods_word,
        None,
        8.0,
        !state.cfg.mods.is_empty(),
        hit::Hit::ModsButton,
        actions_h - 4.0,
    );

    let save_x = cx + mods_w + 10.0;
    let save_label = if cur.source.is_in_library() {
        tr!("In library", "ライブラリ済み", "В библиотеке")
    } else {
        tr!("Save (S)", "保存 (S)", "Сохранить (S)")
    };
    let save_w = Canvas::text_width(8.0, save_label) + 24.0;
    c.button(
        save_x,
        ay + 4.0,
        save_w,
        actions_h - 8.0,
        save_label,
        8.0,
        hit::Hit::JourneySave,
        false,
    );

    let play_w = 152.0;
    let play_x = cx + cw - play_w;
    let skip_label = tr!("Skip (Tab)", "スキップ (Tab)", "Скип (Tab)");
    let skip_w = Canvas::text_width(8.0, skip_label) + 24.0;
    c.button(
        play_x - 10.0 - skip_w,
        ay + 4.0,
        skip_w,
        actions_h - 8.0,
        skip_label,
        8.0,
        hit::Hit::Key(KeyCode::Tab),
        false,
    );

    let play_label = if cur.source.is_ready() {
        tr!("Play", "プレイ", "Играть")
    } else if matches!(cur.source, MapSource::Failed(_)) {
        tr!("Next", "次へ", "Дальше")
    } else {
        tr!("Loading...", "読込中...", "Загрузка...")
    };
    let bar = [play_x, ay + 2.0, play_w, actions_h - 4.0];
    style::lane_end_button(
        c,
        "journey-play",
        bar,
        play_label,
        hit::Hit::Key(KeyCode::Enter),
        true,
    );
}

fn role_badge(role: &str) -> (&'static str, u32) {
    match role {
        "warmup" => ("WARM-UP", 0x5ee07a),
        "stretch" => ("STRETCH", 0xff7a50),
        "custom" => ("CUSTOM", 0x22c8b8),
        "explore" => ("EXPLORE", 0x9b6cff),
        "cooldown" => ("COOL-DOWN", 0x56b6e8),
        _ => ("FOR YOU", 0xffd34d),
    }
}

impl State {
    /// Opens the Journey screen and starts fetching (or falls back to local library if offline).
    pub(crate) fn open_journey(&mut self) {
        self.main_menu = None;
        self.picking_lobby_map = false;
        self.journey.reset();
        self.journey_refill();
        self.scene = JourneyScreen::scene();
        self.journey_preview_current();
    }

    pub(crate) fn exit_journey(&mut self) {
        self.show_select();
    }

    pub(crate) fn open_mods(&mut self) {
        self.open_mod_select();
    }

    pub(crate) fn journey_pick_level(&mut self, idx: usize) {
        let Some(&(_, stars)) = LEVEL_CHOICES.get(idx) else {
            return;
        };
        self.journey.level_cursor = idx;
        self.journey.target_stars = Some(stars);
        self.journey.comfort_stars = stars;
        self.journey.needs_level = false;
        self.journey.pool_exhausted = false;
        self.journey.retry_after = None;
        self.journey_refill();
        self.journey_preview_current();
    }

    pub(crate) fn journey_tick(&mut self) {
        let fetched = self.journey.poll_fetch(&self.library);
        let dl_done = self.journey.poll_downloads();
        // If server returned an error with an empty queue, fall back to local library cleanly.
        if fetched && self.journey.error.is_some() && self.journey.queue.is_empty() {
            let err = self.journey.error.take();
            if !self.library.is_empty() {
                self.journey.local_mode = true;
                self.journey.pool_exhausted = false;
                self.journey.needs_level = self.journey.target_stars.is_none();
                if !self.journey.needs_level {
                    self.journey_fill_local();
                }
            } else {
                self.journey.error = err;
            }
        }
        if fetched || dl_done {
            self.journey_preview_current();
        }
        if self.journey.can_refill(Instant::now()) {
            self.journey_refill();
        }
    }

    fn journey_refill(&mut self) {
        if !self.journey.can_refill(Instant::now()) {
            return;
        }
        let can_online = !self.cfg.online_token.is_empty() && self.journey_supported;
        if can_online && !self.journey.local_mode {
            self.journey.request_next(
                self.cfg.online_server.clone(),
                self.cfg.online_token.clone(),
            );
        } else {
            self.journey.local_mode = true;
            if self.journey.target_stars.is_none() && self.journey.step == 0 {
                self.journey.needs_level = true;
                return;
            }
            self.journey_fill_local();
        }
    }

    fn journey_fill_local(&mut self) {
        let rate = self.cfg.rate;
        let stars = &self.stars;
        self.journey
            .fill_from_local(&self.library, |i| stars.get(i, rate));
    }

    fn journey_preview_current(&mut self) {
        let path = match self.journey.current().map(|q| &q.source) {
            Some(MapSource::Local(p) | MapSource::Saved(p)) => Some(p.clone()),
            Some(MapSource::Cached { entry, .. }) => Some(entry.path.clone()),
            _ => None,
        };
        if let Some(p) = path {
            self.select.select_path(&self.library, &p);
        }
    }

    /// Drops a failed download without sending a negative `JourneySkip` taste signal.
    fn journey_drop_failed(&mut self) {
        self.journey.advance(PastOutcome::Skipped);
        self.journey.pool_exhausted = false;
        self.journey_refill();
        self.journey_preview_current();
    }

    pub(crate) fn journey_skip_current(&mut self) {
        let preview_secs = self.journey.shown_at.elapsed().as_secs_f64();
        let Some(skipped) = self.journey.advance(PastOutcome::Skipped) else {
            return;
        };
        if !self.journey.local_mode && !self.cfg.online_token.is_empty() {
            let server = self.cfg.online_server.clone();
            let req = online::Request::JourneySkip {
                token: self.cfg.online_token.clone(),
                md5: skipped.item.md5,
                map_hash: skipped.item.map_hash,
                preview_secs,
            };
            std::thread::spawn(move || {
                let _ = online::call(&server, req);
            });
        }
        self.journey_refill();
        self.journey_preview_current();
    }

    pub(crate) fn journey_play_current(&mut self) {
        let entry_idx = match self.journey.current() {
            Some(q) => match &q.source {
                MapSource::Local(path) | MapSource::Saved(path) => {
                    let path = path.clone();
                    self.journey.playing_cached = None;
                    let Some(idx) = self.library.iter().position(|e| e.path == path) else {
                        self.journey_drop_failed();
                        return;
                    };
                    idx
                }
                MapSource::Cached { folder, entry } => {
                    let folder = folder.clone();
                    let path = entry.path.clone();
                    let version = q.item.version.clone();
                    self.journey.playing_cached = Some((folder.clone(), q.item.clone()));
                    if let Some(existing) = self.library.iter().position(|e| e.path == path) {
                        existing
                    } else {
                        let entries = crate::journey::scan_single_folder(&folder);
                        let pos = entries
                            .iter()
                            .position(|e| e.meta.version.eq_ignore_ascii_case(&version))
                            .unwrap_or(0);
                        let Some(loaded) = entries.into_iter().nth(pos) else {
                            self.journey_drop_failed();
                            return;
                        };
                        self.library.push(loaded);
                        self.library.len() - 1
                    }
                }
                MapSource::Pending | MapSource::Downloading(_) => {
                    self.toast(
                        tr!(
                            "Still downloading this map...",
                            "譜面をダウンロード中...",
                            "Карта ещё скачивается..."
                        )
                        .into(),
                    );
                    return;
                }
                MapSource::Failed(_) => {
                    // Never send a negative JourneySkip signal for a failed download.
                    self.journey_drop_failed();
                    return;
                }
            },
            None => return,
        };
        self.journey.in_play = true;
        self.start_loading_from(entry_idx, None);
    }

    pub(crate) fn journey_next_from_results(&mut self, failed: bool) {
        self.journey.in_play = false;
        self.journey.playing_cached = None;
        let outcome = if failed {
            PastOutcome::Failed
        } else {
            PastOutcome::Passed
        };
        self.journey.advance(outcome);
        self.journey_refill();
        self.scene = JourneyScreen::scene();
        self.journey_preview_current();
    }

    pub(crate) fn journey_save_current(&mut self) {
        let cached_info = if let Some((folder, item)) = self.journey.playing_cached.clone() {
            Some((folder, item, true))
        } else if let Some(q) = self.journey.current() {
            if let MapSource::Cached { folder, .. } = &q.source {
                Some((folder.clone(), q.item.clone(), false))
            } else {
                None
            }
        } else {
            None
        };

        let Some((folder, item, from_results)) = cached_info else {
            self.toast(
                tr!(
                    "Already in your library",
                    "すでにライブラリにあります",
                    "Карта уже в библиотеке"
                )
                .into(),
            );
            return;
        };

        // Resolve the real osu! Songs folder via library::song_dirs() / download_dir()
        // rather than falling back to own_songs_dir when cfg.songs_folder is None.
        let osu_songs = library::download_dir(&library::song_dirs());
        match crate::journey::save_cached_folder(
            &folder,
            item.custom_id.is_some(),
            self.cfg.journey_save_to,
            &osu_songs,
        ) {
            Ok(dest) => {
                // Remove stale JourneyCache entries from self.library before adding the moved files
                // so there are no duplicate dead paths.
                self.library.retain(|e| !e.path.starts_with(&folder));

                let added = crate::journey::scan_single_folder(&dest);
                let mut saved_path = None;
                for e in added {
                    let is_match = item.map_hash.is_some_and(|h| e.known_hash() == Some(h))
                        || e.meta.version.eq_ignore_ascii_case(&item.version);
                    if is_match || saved_path.is_none() {
                        saved_path = Some(e.path.clone());
                    }
                    if !self.library.iter().any(|x| x.path == e.path) {
                        self.library.push(e);
                    }
                }
                if let Some(path) = saved_path {
                    self.select.refilter(&self.library);
                    if !from_results && let Some(q) = self.journey.current_mut() {
                        q.source = MapSource::Saved(path);
                    }
                }
                if from_results {
                    self.journey.playing_cached = None;
                }
                let where_saved = self.cfg.journey_save_to.name();
                self.toast(tr!(
                    "Saved to {where_saved}",
                    "{where_saved}に保存しました",
                    "Сохранено в: {where_saved}";
                    [] [where_saved]
                ));
            }
            Err(e) => {
                self.toast(
                    tr!("Save failed: {e}", "保存失敗: {e}", "Ошибка сохранения: {e}"; [] [e]),
                );
            }
        }
    }
}
