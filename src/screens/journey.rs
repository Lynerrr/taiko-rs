//! The Journey screen: an endless, level-and-taste-matched map feed drawn in the client's
//! own visual language (`style.rs`): a mini session lane across the top, one focused map card
//! in the middle, and a Song-Select-style action bar along the bottom.

use super::screen::{Frame, Key, Screen};
use crate::journey::{Gate, LEVEL_CHOICES, MapSource, PastOutcome};
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

        if let Some(gate) = state.journey.gate {
            match (code, gate) {
                (KeyCode::Escape, _) => state.exit_journey(),
                (KeyCode::Enter | KeyCode::NumpadEnter, Gate::SignedOut) => {
                    state.journey_login(false);
                }
                (KeyCode::KeyR, Gate::SignedOut) => state.journey_login(true),
                (KeyCode::Enter | KeyCode::NumpadEnter, Gate::ServerDown) => {
                    state.journey_retry();
                }
                (KeyCode::KeyO, _) => state.journey_go_offline(),
                _ => {}
            }
            return;
        }

        if state.journey.needs_level {
            let has_auto = level_picker_has_auto(state);
            let total = LEVEL_CHOICES.len() + usize::from(has_auto);
            match code {
                KeyCode::ArrowUp | KeyCode::KeyW => {
                    state.journey.level_cursor = state.journey.level_cursor.saturating_sub(1);
                }
                KeyCode::ArrowDown | KeyCode::KeyS => {
                    state.journey.level_cursor = (state.journey.level_cursor + 1).min(total - 1);
                }
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let cur = state.journey.level_cursor.min(total - 1);
                    if has_auto && cur == 0 {
                        state.journey_pick_auto();
                    } else {
                        let idx = cur - usize::from(has_auto);
                        state.journey_pick_level(idx);
                    }
                }
                KeyCode::Escape => {
                    if !state.journey.queue.is_empty() {
                        state.journey.needs_level = false;
                    } else {
                        state.exit_journey();
                    }
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Escape => state.exit_journey(),
            KeyCode::Enter | KeyCode::NumpadEnter => state.journey_play_current(),
            KeyCode::Tab | KeyCode::ArrowRight => state.journey_skip_current(),
            KeyCode::ArrowLeft => state.journey_back(),
            KeyCode::KeyS => state.journey_save_current(),
            KeyCode::F1 => state.open_mods(),
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                state.step_rate(code == KeyCode::BracketRight);
            }
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _frame: &Frame) -> Option<Scene> {
        state.journey_tick();

        style::ground(canvas, 1.0);
        let w = canvas.width;
        let h = render::VIRTUAL_HEIGHT;

        if let Some(gate) = state.journey.gate {
            draw_gate(state, gate, canvas, w, h);
            return None;
        }

        if state.journey.needs_level {
            draw_level_picker(state, canvas, w, h);
            return None;
        }

        if render::animations() {
            render::approach(&mut state.journey.lane_slide, 0.0, canvas.dt_ms, 55.0);
            if state.journey.lane_slide.abs() < 0.1 {
                state.journey.lane_slide = 0.0;
            }
        } else {
            state.journey.lane_slide = 0.0;
        }

        draw_top_lane(state, canvas, w);
        draw_center_card(state, canvas, w, h);
        None
    }
}

/// What Journey asks before it goes on: log in or go offline (signed out), or wait for the
/// server or go offline (server down).
fn draw_gate(state: &State, gate: Gate, c: &mut Canvas, w: f32, h: f32) {
    let pw = (w - 48.0).clamp(420.0, 560.0);
    let ph = 230.0;
    let px = (w - pw) / 2.0;
    let py = ((h - ph) / 2.0).max(60.0);
    style::cold_panel(c, [px, py, pw, ph]);
    let (title, body) = match gate {
        Gate::SignedOut => (
            tr!(
                "Log in to start your Journey",
                "ジャーニーを始めるにはログイン",
                "Войди, чтобы начать Journey"
            ),
            tr!(
                "Journey picks maps for you from the whole taiko.rs pool, downloads them as you go and learns from your plays. It needs a taiko.rs account.",
                "ジャーニーは taiko.rs の全譜面からあなたに合う譜面を選び、その場でダウンロードし、プレイから学習します。taiko.rs アカウントが必要です。",
                "Journey подбирает карты из всего пула taiko.rs, сам их скачивает и учится на твоих играх. Для этого нужен аккаунт taiko.rs."
            ),
        ),
        Gate::ServerDown => (
            tr!(
                "Can't reach the taiko.rs server",
                "taiko.rs サーバーに接続できません",
                "Сервер taiko.rs не отвечает"
            ),
            tr!(
                "Journey keeps trying in the background and goes on by itself as soon as the server answers.",
                "バックグラウンドで再接続を続け、サーバーが応答するとそのまま再開します。",
                "Journey продолжает пробовать в фоне и сам продолжит, как только сервер ответит."
            ),
        ),
    };
    let cx = px + pw / 2.0;
    c.text_centered(cx, py + 22.0, 12.0, rgba(0xffffff, 0.96), title);
    let mut y = py + 48.0;
    for line in crate::mobile::wrap(8.0, pw - 48.0, body, 3) {
        c.text_centered(cx, y, 8.0, rgba(0xb4b4c8, 1.0), &line);
        y += 13.0;
    }
    if gate == Gate::ServerDown
        && let Some(err) = &state.journey.error
    {
        c.text_centered(
            cx,
            y + 2.0,
            7.0,
            rgba(0x787890, 1.0),
            &Canvas::fit(7.0, pw - 48.0, err),
        );
    }

    let by = py + 128.0;
    let bh = 26.0;
    let offline = tr!("Continue offline", "オフラインで続ける", "Играть оффлайн");
    let buttons: Vec<(&'static style::Icon, &str, bool, hit::Hit)> = match gate {
        Gate::SignedOut => vec![
            (
                &style::PLAY,
                tr!("Log in", "ログイン", "Войти"),
                true,
                hit::Hit::JourneyLogin(false),
            ),
            (
                &style::SPARK,
                tr!("Create account", "アカウント作成", "Создать аккаунт"),
                false,
                hit::Hit::JourneyLogin(true),
            ),
            (&style::FOLDER, offline, false, hit::Hit::JourneyOffline),
        ],
        Gate::ServerDown => vec![
            (
                &style::PLAY,
                tr!("Retry now", "今すぐ再試行", "Повторить"),
                true,
                hit::Hit::JourneyRetry,
            ),
            (&style::FOLDER, offline, false, hit::Hit::JourneyOffline),
        ],
    };
    // Centred: each about as wide as its label and icon (as the action bar reckons).
    let gap = 10.0;
    let row: f32 = buttons
        .iter()
        .map(|(_, label, _, _)| Canvas::text_width(8.0, label) + 34.0 + gap)
        .sum::<f32>()
        - gap;
    let mut bx = cx - row / 2.0;
    for (icon, label, lit, hit) in buttons {
        bx += style::block_button(c, [bx, by], icon, label, None, 8.0, lit, hit, bh) + gap;
    }
    let note = match gate {
        Gate::SignedOut => tr!(
            "Offline, Journey only picks from maps already in your library: no new map downloads and no online analysis of your plays.",
            "オフラインではライブラリ内の譜面だけから選びます。新しい譜面のダウンロードやプレイのオンライン分析はありません。",
            "Оффлайн Journey берёт только карты из твоей библиотеки: без скачивания новых карт и без онлайн-анализа твоих игр."
        ),
        Gate::ServerDown => tr!(
            "Offline, Journey only picks from maps already in your library, until the server is back: then it switches back by itself.",
            "オフラインではサーバーが戻るまでライブラリ内の譜面だけから選び、戻ると自動で切り替わります。",
            "Оффлайн Journey берёт только карты из твоей библиотеки, пока сервер не вернётся, а потом сам переключится обратно."
        ),
    };
    let mut ny = by + 26.0;
    for line in crate::mobile::wrap(7.0, pw - 48.0, note, 3) {
        c.text_centered(cx, ny, 7.0, rgba(0x8888a4, 1.0), &line);
        ny += 11.0;
    }
}

/// Whether the Journey level picker should offer an Auto row (server-estimated comfort stars).
fn level_picker_has_auto(state: &State) -> bool {
    !state.journey.local_mode
        && !state.cfg.online_token.is_empty()
        && (state.journey.auto_comfort_stars.is_some()
            || state.journey.target_stars.is_some()
            || !state.journey.queue.is_empty())
}

fn draw_level_picker(state: &State, c: &mut Canvas, w: f32, h: f32) {
    let has_auto = level_picker_has_auto(state);
    let mut rows: Vec<(String, hit::Hit)> = Vec::with_capacity(LEVEL_CHOICES.len() + 1);
    if has_auto {
        let auto_lbl = match state.journey.auto_comfort_stars {
            Some(s) => {
                let stars = format!("{s:.2}");
                tr!(
                    "Auto (~{stars}*)",
                    "オート (~{stars}*)",
                    "Авто (~{stars}*)";
                    [] [stars]
                )
            }
            None => tr!("Auto (from plays)", "オート (履歴から)", "Авто (по играм)").to_string(),
        };
        rows.push((auto_lbl, hit::Hit::JourneyLevelAuto));
    }
    for (i, (label, _)) in LEVEL_CHOICES.iter().enumerate() {
        rows.push(((*label).to_string(), hit::Hit::JourneyLevel(i)));
    }

    let pw = 300.0_f32.min(w - 32.0);
    let row_h = 30.0;
    let ph = 86.0 + rows.len() as f32 * row_h + 26.0;
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
    let cursor = state.journey.level_cursor.min(rows.len().saturating_sub(1));
    for (i, (label, hit_action)) in rows.iter().enumerate() {
        let ry = y + 68.0 + i as f32 * row_h;
        let rh = row_h - 4.0;
        let selected = i == cursor;
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
        c.hit(rx, ry, rw, rh, *hit_action);
        let is_auto = *hit_action == hit::Hit::JourneyLevelAuto;
        let col = if selected || hovered {
            if is_auto { 0x62f2df } else { 0xffffff }
        } else if is_auto {
            0x22d4c0
        } else {
            0xb4b4c8
        };
        c.text_centered(
            rx + rw / 2.0,
            ry + (rh - 10.0) / 2.0,
            10.0,
            rgba(col, 1.0),
            &Canvas::fit(10.0, rw - 16.0, label),
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

/// The Skip icon turned around: back to the map before.
const PREV_ICON: [&str; 16] = [
    "................",
    "................",
    "....##......##..",
    "...###.....###..",
    "..####....####..",
    ".#####...#####..",
    "######..######..",
    "######.#######..",
    "######..######..",
    ".#####...#####..",
    "..####....####..",
    "...###.....###..",
    "....##......##..",
    "................",
    "................",
    "................",
];

const SKIP_ICON: [&str; 16] = [
    "................",
    "................",
    "..##......##....",
    "..###.....###...",
    "..####....####..",
    "..#####...#####.",
    "..######..######",
    "..#######.######",
    "..######..######",
    "..#####...#####.",
    "..####....####..",
    "..###.....###...",
    "..##......##....",
    "................",
    "................",
    "................",
];

fn draw_top_lane(state: &State, c: &mut Canvas, w: f32) {
    let y = 30.0;
    let h = 38.0;
    let x = 24.0;
    let pw = w - 48.0;
    let mid_y = y + h / 2.0;

    let (role_label, role_col) = state
        .journey
        .current()
        .map(|q| role_badge(&q.item.role))
        .unwrap_or(("LOADING", 0x787890));

    style::cold_panel(c, [x, y, pw, h]);
    // Subtle role-coloured accent bar on the left edge of the session bar.
    c.rect(x + 1.0, y + 6.0, 2.5, h - 12.0, rgba(role_col, 0.9));

    let back = tr!("Exit", "終了", "Выход");
    let bw = style::block_button(
        c,
        [x + 12.0, mid_y],
        &style::BACK,
        back,
        None,
        8.0,
        false,
        hit::Hit::Key(KeyCode::Escape),
        26.0,
    );

    // Interactive comfort star-rating control: [-] [~7.25* AUTO] [+] (0.25* steps).
    let mode_tag = if state.journey.local_mode {
        let local = tr!("LOCAL", "ローカル", "ЛОКАЛЬНО");
        format!("~{:.2}*  {local}", state.journey.comfort_stars)
    } else if state.journey.target_stars.is_some() {
        let manual = tr!("MANUAL", "手動", "ВРУЧНУЮ");
        format!("{:.2}*  {manual}", state.journey.comfort_stars)
    } else {
        let auto = tr!("AUTO", "オート", "АВТО");
        format!("~{:.2}*  {auto}", state.journey.comfort_stars)
    };
    let step_w = 20.0;
    let mw = Canvas::text_width(8.0, &mode_tag) + 14.0;
    let total_ctrl_w = step_w + 4.0 + mw + 4.0 + step_w;
    let ctrl_x = x + pw - 10.0 - total_ctrl_w;

    // Minus (-0.25*)
    let minus_h = c.hovered(ctrl_x, mid_y - 11.0, step_w, 22.0);
    c.round_rect(
        [ctrl_x, mid_y - 11.0],
        [step_w, 22.0],
        style::radius::CONTROL,
        rgba(if minus_h { 0x2c3044 } else { 0x191c2a }, 0.95),
        rgba(if minus_h { role_col } else { style::tone::EDGE }, 0.75),
        1.0,
    );
    c.text_centered(
        ctrl_x + step_w / 2.0,
        mid_y - 4.5,
        9.0,
        rgba(0xffffff, 0.95),
        "-",
    );
    c.hit(
        ctrl_x,
        mid_y - 11.0,
        step_w,
        22.0,
        hit::Hit::JourneyLevelStep(-1),
    );

    // Clickable level chip (opens level picker)
    let mx = ctrl_x + step_w + 4.0;
    let chip_h = c.hovered(mx, mid_y - 11.0, mw, 22.0);
    c.round_rect(
        [mx, mid_y - 11.0],
        [mw, 22.0],
        style::radius::CONTROL,
        rgba(role_col, if chip_h { 0.24 } else { 0.14 }),
        rgba(role_col, if chip_h { 0.85 } else { 0.45 }),
        1.0,
    );
    c.text_centered(
        mx + mw / 2.0,
        mid_y - 4.0,
        8.0,
        rgba(role_col, 1.0),
        &mode_tag,
    );
    c.hit(mx, mid_y - 11.0, mw, 22.0, hit::Hit::JourneyLevelOpen);

    // Plus (+0.25*)
    let px = mx + mw + 4.0;
    let plus_h = c.hovered(px, mid_y - 11.0, step_w, 22.0);
    c.round_rect(
        [px, mid_y - 11.0],
        [step_w, 22.0],
        style::radius::CONTROL,
        rgba(if plus_h { 0x2c3044 } else { 0x191c2a }, 0.95),
        rgba(if plus_h { role_col } else { style::tone::EDGE }, 0.75),
        1.0,
    );
    c.text_centered(
        px + step_w / 2.0,
        mid_y - 4.5,
        9.0,
        rgba(0xffffff, 0.95),
        "+",
    );
    c.hit(
        px,
        mid_y - 11.0,
        step_w,
        22.0,
        hit::Hit::JourneyLevelStep(1),
    );

    let lane_left = x + 16.0 + bw + 14.0;
    let lane_right = ctrl_x - 16.0;
    if lane_right - lane_left < 120.0 {
        return;
    }
    let role_w = Canvas::text_width(7.0, role_label);
    let show_role = lane_right - lane_left >= role_w + 180.0;
    let track_right = if show_role {
        lane_right - role_w - 14.0
    } else {
        lane_right
    };
    let slide = state.journey.lane_slide;
    c.rect(
        lane_left,
        mid_y - 1.0,
        (track_right - lane_left).max(0.0),
        2.0,
        rgba(0x2a2a3c, 1.0),
    );
    let center_x = (lane_left + track_right) / 2.0;
    // Coloured glow along the active half of the track.
    c.rect(
        lane_left,
        mid_y - 1.0,
        (center_x - lane_left).max(0.0),
        2.0,
        rgba(role_col, 0.45),
    );

    for (k, past) in state.journey.history.iter().rev().take(6).enumerate() {
        let nx = center_x - 36.0 - k as f32 * 24.0 + slide;
        if nx < lane_left + 10.0 || nx > center_x - 16.0 {
            continue;
        }
        let col = match past.outcome {
            PastOutcome::Passed => 0x4ee07a,
            PastOutcome::Failed => 0xff4d5a,
            PastOutcome::Skipped => 0x5c5c74,
        };
        c.circle(nx, mid_y, 5.5, rgba(col, 0.9), rgba(0xffffff, 0.4), 1.0);
    }

    // Active node with soft role halo.
    c.circle(
        center_x,
        mid_y,
        13.5,
        rgba(role_col, 0.18 + 0.06 * style::pulse()),
        rgba(0, 0.0),
        0.0,
    );
    c.circle(
        center_x,
        mid_y,
        8.5,
        rgba(role_col, 1.0),
        rgba(0xffffff, 0.95),
        1.5,
    );
    style::selector(
        c,
        "journey-now",
        [center_x - 12.0, mid_y - 12.0, 24.0, 24.0],
    );

    for (k, next_q) in state.journey.queue.iter().skip(1).take(3).enumerate() {
        let nx = center_x + 36.0 + k as f32 * 26.0 + slide;
        if nx > track_right - 12.0 || nx < center_x + 16.0 {
            continue;
        }
        let (_, ncol) = role_badge(&next_q.item.role);
        let alpha = if next_q.source.is_ready() { 0.85 } else { 0.4 };
        c.circle(nx, mid_y, 6.0, rgba(ncol, alpha), rgba(0xffffff, 0.35), 1.0);
    }

    if show_role {
        c.text(
            lane_right - role_w,
            mid_y - 3.5,
            7.0,
            rgba(role_col, 0.95),
            role_label,
        );
    }
}

fn current_entry_for_preview<'a>(
    journey: &'a crate::journey::Journey,
    library: &'a [library::Entry],
    library_idx: Option<usize>,
) -> Option<&'a library::Entry> {
    let cur = journey.current()?;
    match &cur.source {
        MapSource::Local(_) | MapSource::Saved(_) => library_idx.and_then(|i| library.get(i)),
        MapSource::Cached { entry, .. } => Some(entry.as_ref()),
        _ => None,
    }
}

fn draw_center_card(state: &mut State, c: &mut Canvas, w: f32, h: f32) {
    let cw = (w - 48.0).clamp(460.0, 780.0);
    let cx = (w - cw) / 2.0;
    let cy = 76.0;
    let actions_h = 34.0;
    let ch = (h - cy - actions_h - 22.0).max(260.0);

    let enter = if render::animations() {
        style::motion::entrance(state.journey.shown_at, 220.0, style::motion::out_quint)
    } else {
        1.0
    };

    let Some(cur_cloned) = state.journey.current().map(|q| (q.item.clone(), q.source.is_ready(), q.source.is_in_library(), q.source.download_progress(), matches!(q.source, MapSource::Failed(_)), match &q.source {
        MapSource::Local(_) => tr!("In library", "ライブラリ済み", "В библиотеке").to_string(),
        MapSource::Saved(_) => tr!("Saved to library", "保存済み", "Сохранено").to_string(),
        MapSource::Cached { .. } => tr!("Cached · S to save", "キャッシュ · Sで保存", "В кэше · S сохранить").to_string(),
        MapSource::Pending => tr!("Queued...", "待機中...", "В очереди...").to_string(),
        MapSource::Downloading(job) => {
            let pct = (job.fraction().unwrap_or(0.0) * 100.0).round() as u32;
            tr!("Downloading {pct}%...", "ダウンロード中 {pct}%...", "Загрузка {pct}%..."; [] [pct])
        }
        MapSource::Failed(e) => tr!("Error: {e}", "エラー: {e}", "Ошибка: {e}"; [] [e]),
    })) else {
        state.preview.potential = None;
        style::cold_panel(c, [cx, cy, cw, ch]);
        let msg = if state.journey.outage_since.is_some() && !state.journey.local_mode {
            // A drop of a few seconds: retried before it's called an outage (`Gate::ServerDown`).
            tr!(
                "Reconnecting to the server...",
                "サーバーに再接続中...",
                "Переподключаемся к серверу..."
            )
        } else if let Some(err) = &state.journey.error {
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
    let (item, is_ready, is_in_library, dl_progress, is_failed, status_str) = cur_cloned;
    let (role_label, role_col) = role_badge(&item.role);
    let lib_idx = state.journey.current_library_index(&state.library);
    let preview_entry = current_entry_for_preview(&state.journey, &state.library, lib_idx);
    let is_epic = state.journey_map_is_epic(&item, preview_entry);
    let map_status = state.journey_map_status(&item, preview_entry, lib_idx);
    let (entry_path, bg_path, od_raw, entry_level) = match preview_entry {
        Some(e) => (
            is_ready.then(|| e.path.clone()),
            crate::preview::background_path(e),
            Some(e.meta.od),
            e.meta.level,
        ),
        None => (None, None, None, None),
    };
    let rate = state.play_speed().0;
    let (pot_pp, live_stars) = match &entry_path {
        Some(path) => {
            state
                .potentials
                .get_with_stars(path, rate, state.cfg.mods, state.cfg.scroll_speed)
        }
        None => (None, None),
    };
    state.preview.potential = pot_pp;
    state.preview.mods = state.cfg.mods;
    let shown_stars = live_stars
        .filter(|s| s.is_finite() && *s > 0.05)
        .unwrap_or(item.stars);
    let od_val = od_raw.map(|od| {
        crate::beatmap::od_at_rate(state.cfg.mods.od_with(od, crate::od_override::get()), rate)
    });
    let loaded_stats = state.preview.loaded_stats();

    let card_start = c.shapes.len();

    // Base cold panel + role-coloured accent edge and top glow.
    style::cold_panel(c, [cx, cy, cw, ch]);
    c.round_rect(
        [cx, cy],
        [cw, ch],
        style::radius::CARD,
        rgba(0, 0.0),
        rgba(role_col, 0.42),
        1.0,
    );

    // 1. Top artwork banner (like song select's split banner) with role tint.
    let banner_h = (ch * 0.34).clamp(96.0, 122.0);
    if let Some(bg) = bg_path {
        let drawn = c.picture_cover_round_top(
            &bg,
            [cx, cy, cw, banner_h],
            [cx, cy, cw, banner_h],
            [0.85, 0.85, 0.85, 1.0],
            style::radius::CARD,
            Some(0.55),
        );
        if drawn {
            c.shade_eased_fade_down(
                [cx, cy],
                [cw, banner_h],
                style::radius::CARD,
                rgba(style::PANEL_COLD, 0.72),
                rgba(style::PANEL_COLD, 0.12),
                0.55,
            );
        }
    }
    // Soft role colour wash at top of banner + crisp 2.5px accent strip.
    c.round_rect(
        [cx + 1.0, cy + 1.0],
        [cw - 2.0, 3.0],
        1.5,
        rgba(role_col, 0.95),
        rgba(0, 0.0),
        0.0,
    );
    c.rect(cx + 1.0, cy + 4.0, cw - 2.0, 26.0, rgba(role_col, 0.08));

    let pad = 16.0;
    let inner_w = cw - pad * 2.0;

    // Top-right star chip + map status badge (EPIC / RANKED / LOVED / GRAVEYARD / etc.).
    // Drawn before the reason text so we know the exact room left on the top banner row.
    // star_chip frames at y - 3, so y = cy + 14 aligns with the role pill and status badge along cy + 19.
    let stars_opt = (shown_stars > 0.05).then_some(shown_stars as f32);
    let star_w = style::star_chip(c, cx + cw - pad, cy + 14.0, 9.5, entry_level, stars_opt);
    let status_w = if is_epic {
        let pw = crate::epic::pill_width();
        let ex = cx + cw - pad - star_w - 8.0 - pw;
        crate::epic::pill(c, ex, cy + 12.5, 1.0, style::pulse());
        pw
    } else if let Some((label, col)) = map_status {
        let bw = Canvas::text_width(7.0, label) + 12.0;
        let bx = cx + cw - pad - star_w - 8.0 - bw;
        c.round_rect(
            [bx, cy + 11.5],
            [bw, 15.0],
            style::radius::SMALL,
            rgba(col, 0.16),
            rgba(col, 0.85),
            1.0,
        );
        c.text_centered(bx + bw / 2.0, cy + 15.5, 7.0, rgba(col, 1.0), label);
        bw
    } else {
        let source_text = if state.journey.local_mode {
            tr!("library", "ライブラリ", "библиотека")
        } else {
            "osu!"
        };
        let src_w = Canvas::text_width(7.0, source_text) + 10.0;
        let src_x = cx + cw - pad - star_w - 8.0 - src_w;
        c.round_rect(
            [src_x, cy + 12.0],
            [src_w, 14.0],
            style::radius::SMALL,
            rgba(0x161926, 0.78),
            rgba(style::tone::EDGE, 0.8),
            1.0,
        );
        c.text_centered(
            src_x + src_w / 2.0,
            cy + 15.5,
            7.0,
            rgba(0xb8b8d0, 1.0),
            source_text,
        );
        src_w
    };

    // Top-left role pill + difficulty delta + reason.
    let delta = if shown_stars > 0.1 && state.journey.comfort_stars > 0.1 {
        Some(shown_stars - state.journey.comfort_stars)
    } else {
        None
    };
    let pill_text = match delta {
        Some(d) if d.abs() >= 0.15 => format!("{role_label}  {d:+.1}*"),
        _ => role_label.to_string(),
    };
    let pill_w = Canvas::text_width(7.5, &pill_text) + 14.0;
    c.round_rect(
        [cx + pad, cy + 11.0],
        [pill_w, 16.0],
        style::radius::SMALL,
        rgba(role_col, 0.24),
        rgba(role_col, 0.85),
        1.0,
    );
    c.text_centered(
        cx + pad + pill_w / 2.0,
        cy + 15.0,
        7.5,
        rgba(role_col, 1.0),
        &pill_text,
    );
    let reason_x = cx + pad + pill_w + 10.0;
    let right_used = star_w + 8.0 + status_w + 12.0;
    let reason_room = (cw - pad * 2.0 - pill_w - 10.0 - right_used).max(40.0);
    c.text(
        reason_x,
        cy + 15.5,
        7.5,
        rgba(0xd4d4e8, 0.92),
        &Canvas::fit(7.5, reason_room, &item.reason),
    );

    // Bottom of banner: title, artist, difficulty + mapper.
    let title_y = cy + banner_h - 54.0;
    let title = Canvas::fit(15.0, inner_w, &item.title);
    style::title(c, cx + pad, title_y, 15.0, 0xffffff, &title);
    let artist = Canvas::fit(9.5, inner_w, &item.artist);
    c.text(cx + pad, title_y + 21.0, 9.5, rgba(0xd0d0e4, 1.0), &artist);
    let diff_text = format!("[{}]  by {}", item.version, item.creator);
    c.text(
        cx + pad,
        title_y + 36.0,
        8.5,
        rgba(role_col, 0.95),
        &Canvas::fit(8.5, inner_w, &diff_text),
    );

    // Divider under banner.
    let body_y = cy + banner_h;
    style::rule(c, cx + 1.0, body_y, cw - 2.0);

    // 2. Body split: left (~58%) = map stats + live preview + density graph;
    //                right (~42%) = match profile (comfort bar + 5 coloured feature bars).
    let body_h = (cy + ch - body_y).max(120.0);
    let left_w = (inner_w * 0.58).floor();
    let col_gap = 16.0;
    let right_x = cx + pad + left_w + col_gap;
    let right_w = (cx + cw - pad - right_x).max(120.0);

    // Vertical divider between left & right columns.
    c.rect(
        right_x - col_gap / 2.0,
        body_y + 10.0,
        1.0,
        body_h - 20.0,
        rgba(style::tone::EDGE, 0.65),
    );

    // --- LEFT COLUMN: stats, Don/Kat bar, pp row, live autoplay preview + density graph ---
    let mut ly = body_y + 10.0;
    let (raw_bpm, raw_len_ms, dons, kats, rolls, swells) = match loaded_stats {
        Some(st) => (
            if st.bpm > 0.0 { st.bpm } else { item.bpm },
            if st.length_ms > 0.0 {
                st.length_ms as u64
            } else {
                item.length_ms
            },
            Some(st.dons),
            Some(st.kats),
            st.rolls,
            st.swells,
        ),
        None => (item.bpm, item.length_ms, None, None, 0, 0),
    };
    let speed = rate.max(0.01);
    let bpm_val = raw_bpm * speed;
    let len_ms = if raw_len_ms > 0 {
        ((raw_len_ms as f64) / speed).round() as u64
    } else {
        0
    };
    let od_str = od_val
        .map(|od| format!("{od:.1}"))
        .unwrap_or_else(|| "-".into());
    let bpm_str = if bpm_val > 0.0 {
        format!("{bpm_val:.0}")
    } else {
        "-".into()
    };
    let len_str = if len_ms > 0 {
        let s = len_ms / 1000;
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        "-".into()
    };
    let notes_str = match (dons, kats) {
        (Some(d), Some(k)) => format!("{}", d + k),
        _ => "-".into(),
    };
    let stat_cells = [
        ("OD", od_str),
        ("BPM", bpm_str),
        (tr!("LENGTH", "長さ", "ДЛИНА"), len_str),
        (tr!("NOTES", "ノーツ", "НОТЫ"), notes_str),
    ];
    let cell_w = left_w / stat_cells.len() as f32;
    for (k, (cap, val)) in stat_cells.iter().enumerate() {
        let sx = cx + pad + k as f32 * cell_w;
        let cw_cap = c.text(sx, ly + 1.0, 6.5, rgba(0x787892, 1.0), cap);
        c.text(
            sx + cw_cap + 4.0,
            ly,
            8.0,
            rgba(0xf0f0fa, 1.0),
            &Canvas::fit(8.0, (cell_w - cw_cap - 8.0).max(16.0), val),
        );
    }
    ly += 15.0;

    // Don/Kat proportion bar (red + blue, matching song select).
    if let (Some(d), Some(k)) = (dons, kats) {
        let total = (d + k).max(1) as f32;
        let frac = d as f32 / total;
        let don_lbl = format!("{d} DON");
        let kat_lbl = format!("{k} KAT");
        c.text(cx + pad, ly, 6.5, rgba(0xe85a46, 0.95), &don_lbl);
        let kw = Canvas::text_width(6.5, &kat_lbl);
        c.text(
            cx + pad + left_w - kw,
            ly,
            6.5,
            rgba(0x4ea8ff, 0.95),
            &kat_lbl,
        );
        if rolls > 0 || swells > 0 {
            let mid_lbl = format!("{rolls}R · {swells}S");
            c.text_centered(
                cx + pad + left_w / 2.0,
                ly,
                6.5,
                rgba(0x8888a4, 0.9),
                &mid_lbl,
            );
        }
        ly += 9.0;
        let dw = (left_w * frac).max(2.0);
        c.rect(cx + pad, ly, dw, 3.0, rgba(0xe85a46, 0.95));
        c.rect(
            cx + pad + dw,
            ly,
            (left_w - dw).max(0.0),
            3.0,
            rgba(0x4ea8ff, 0.95),
        );
        ly += 8.0;
    } else {
        ly += 6.0;
    }

    // Potential pp row (SS / 99% / 98%) when calculated.
    if let Some([ss, p99, p98]) = pot_pp {
        let pp_items = [
            ("SS", format!("{ss:.0}pp")),
            ("99%", format!("{p99:.0}pp")),
            ("98%", format!("{p98:.0}pp")),
        ];
        let pw_cell = left_w / 3.0;
        for (k, (lbl, val)) in pp_items.iter().enumerate() {
            let px = cx + pad + k as f32 * pw_cell;
            let lw = c.text(px, ly + 1.0, 6.5, rgba(0x8888a4, 1.0), lbl);
            c.text(px + lw + 4.0, ly, 8.0, rgba(0xffd34d, 0.95), val);
        }
        ly += 14.0;
    }

    // Live autoplay preview lane + density graph (or download progress bar).
    let rem_h = (cy + ch - 12.0 - ly).max(58.0);
    let graph_h = (rem_h * 0.30).clamp(24.0, 42.0);
    let lane_h = (rem_h - graph_h - 14.0).max(40.0);
    let lane_rect = [cx + pad, ly, left_w, lane_h];
    let graph_rect = [cx + pad, ly + lane_h + 6.0, left_w, graph_h];

    let scroll_speed = state.cfg.scroll_speed;
    if let Some(entry) =
        current_entry_for_preview(&state.journey, &state.library, lib_idx).filter(|_| is_ready)
    {
        let active =
            state
                .preview
                .draw_embedded_preview(c, entry, lane_rect, graph_rect, scroll_speed);
        if !active {
            c.text_centered(
                cx + pad + left_w / 2.0,
                ly + lane_h / 2.0 - 4.0,
                8.0,
                rgba(0x8888a4, 1.0),
                tr!(
                    "Loading preview...",
                    "プレビュー読込中...",
                    "Загрузка превью..."
                ),
            );
        }
    } else {
        c.round_rect(
            [lane_rect[0], lane_rect[1]],
            [lane_rect[2], lane_rect[3]],
            style::radius::CONTROL,
            rgba(0x0b0d15, 0.9),
            rgba(role_col, 0.35),
            1.0,
        );
        c.text_centered(
            cx + pad + left_w / 2.0,
            ly + lane_h / 2.0 - 10.0,
            8.5,
            rgba(0xd0d0e4, 1.0),
            &Canvas::fit(8.5, left_w - 24.0, &status_str),
        );
        if let Some(p) = dl_progress {
            let bx = cx + pad + 24.0;
            let bw = (left_w - 48.0).max(40.0);
            let by = ly + lane_h / 2.0 + 6.0;
            c.round_rect(
                [bx, by],
                [bw, 5.0],
                2.5,
                rgba(0x222234, 1.0),
                rgba(0, 0.0),
                0.0,
            );
            c.round_rect(
                [bx, by],
                [bw * p.clamp(0.04, 1.0), 5.0],
                2.5,
                rgba(role_col, 0.95),
                rgba(0, 0.0),
                0.0,
            );
        }
    }

    // --- RIGHT COLUMN: WHY THIS MAP (comfort difficulty gauge + 5 coloured feature bars) ---
    let mut ry = body_y + 12.0;
    let heading = tr!("MATCH PROFILE", "マッチ分析", "ПРОФИЛЬ ПОДБОРА");
    c.text(right_x, ry, 7.0, rgba(role_col, 0.95), heading);
    ry += 14.0;

    // Difficulty vs comfort gauge (spans comfort - 1.5* .. comfort + 1.5*).
    const GAUGE_EASIER_DELTA: f64 = -0.30;
    const GAUGE_STRETCH_DELTA: f64 = 0.25;
    let comfort = state.journey.comfort_stars.max(1.0);
    let diff_label = if shown_stars > 0.1 {
        let d = shown_stars - comfort;
        format!("~{comfort:.1}*  ->  {shown_stars:.2}* ({d:+.1}*)")
    } else {
        format!("~{comfort:.1}*")
    };
    c.text(
        right_x,
        ry,
        7.0,
        rgba(0xc8c8dc, 1.0),
        &Canvas::fit(7.0, right_w, &diff_label),
    );
    ry += 11.0;

    // Gauge track: blue (easier) -> green (comfort) -> red (stretch), matching marker thresholds.
    let g_h = 6.0;
    let comfort_lo = ((GAUGE_EASIER_DELTA + 1.5) / 3.0) as f32;
    let comfort_hi = ((GAUGE_STRETCH_DELTA + 1.5) / 3.0) as f32;
    c.round_rect(
        [right_x, ry],
        [right_w, g_h],
        3.0,
        rgba(0x1b1f2e, 1.0),
        rgba(style::tone::EDGE, 0.6),
        1.0,
    );
    // Comfort zone highlight in the middle.
    c.rect(
        right_x + right_w * comfort_lo,
        ry + 1.0,
        right_w * (comfort_hi - comfort_lo),
        g_h - 2.0,
        rgba(0x4ee07a, 0.35),
    );
    // Stretch zone tint on the right.
    c.rect(
        right_x + right_w * comfort_hi,
        ry + 1.0,
        right_w * (1.0 - comfort_hi) - 1.0,
        g_h - 2.0,
        rgba(0xff4d5a, 0.28),
    );
    // Marker for current map.
    if shown_stars > 0.1 {
        let frac = ((shown_stars - comfort + 1.5) / 3.0).clamp(0.04, 0.96) as f32;
        let mx = right_x + right_w * frac;
        let mcol = if shown_stars > comfort + GAUGE_STRETCH_DELTA {
            0xff4d5a
        } else if shown_stars < comfort + GAUGE_EASIER_DELTA {
            0x4ea8ff
        } else {
            0x4ee07a
        };
        c.circle(
            mx,
            ry + g_h / 2.0,
            4.5,
            rgba(mcol, 1.0),
            rgba(0xffffff, 0.95),
            1.2,
        );
    }
    ry += 16.0;

    // 5 coloured bars: Map value (filled bar) vs Player comfort profile (white tick marker).
    let mp = item.profile;
    let pp = state.journey.player_profile;
    let scroll_too_fast = matches!(
        (state.journey.comfort_scroll, item.scroll_velocity),
        (Some(cs), sv) if cs > 0.01 && sv > cs * 1.25
    );
    let bars: [(&str, f64, Option<f64>, u32); 5] = [
        (
            tr!("Density", "密度", "Плотность"),
            mp.density,
            pp.map(|p| p.density),
            0xff9e44,
        ),
        (
            tr!("Streams", "ストリーム", "Стримы"),
            mp.streams,
            pp.map(|p| p.streams),
            0xf2545b,
        ),
        (
            tr!("Rhythm", "リズム", "Ритм"),
            mp.rhythm,
            pp.map(|p| p.rhythm),
            0xa66cff,
        ),
        ("SV", mp.sv, pp.map(|p| p.sv), 0x4ea8ff),
        (
            tr!("Scroll", "スクロール", "Скролл"),
            mp.scroll,
            pp.map(|p| p.scroll),
            if scroll_too_fast { 0xff4d5a } else { 0x22d4c0 },
        ),
    ];
    for (label, val, player_val, col) in bars {
        let val = (val.clamp(0.0, 1.0) as f32) * enter;
        let pct_txt = format!("{:.0}%", val * 100.0);
        c.text(right_x, ry, 7.0, rgba(0xb4b4cc, 1.0), label);
        let pw_txt = Canvas::text_width(6.5, &pct_txt);
        c.text(
            right_x + right_w - pw_txt,
            ry + 0.5,
            6.5,
            rgba(col, 0.95),
            &pct_txt,
        );
        ry += 10.0;
        let bh = 5.0;
        c.round_rect(
            [right_x, ry],
            [right_w, bh],
            2.5,
            rgba(0x1b1f2e, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        if val > 0.01 {
            c.round_rect(
                [right_x, ry],
                [(right_w * val).max(4.0), bh],
                2.5,
                rgba(col, 0.92),
                rgba(0, 0.0),
                0.0,
            );
        }
        if let Some(pv) = player_val {
            let tx = right_x + right_w * (pv.clamp(0.02, 0.98) as f32);
            c.rect(tx - 1.0, ry - 1.5, 2.0, bh + 3.0, rgba(0xffffff, 0.92));
        }
        ry += 12.0;
    }

    // Legend for white tick + separate Save button at bottom of right column (no duplicate text).
    if pp.is_some() {
        c.rect(right_x, ry + 2.0, 2.0, 7.0, rgba(0xffffff, 0.9));
        c.text(
            right_x + 7.0,
            ry + 2.0,
            6.5,
            rgba(0x8888a4, 1.0),
            tr!("Your comfort profile", "あなたの傾向", "Твой профиль"),
        );
    }
    let save_y = cy + ch - 28.0;
    if is_in_library {
        let saved_lbl = tr!("In library", "ライブラリ済み", "В библиотеке");
        c.round_rect(
            [right_x, save_y],
            [right_w, 20.0],
            style::radius::CONTROL,
            rgba(0x4ee07a, 0.12),
            rgba(0x4ee07a, 0.45),
            1.0,
        );
        c.text_centered(
            right_x + right_w / 2.0,
            save_y + 6.5,
            7.0,
            rgba(0x4ee07a, 0.95),
            saved_lbl,
        );
    } else if is_ready {
        let save_lbl = tr!(
            "Save to library (S)",
            "ライブラリに保存 (S)",
            "Сохранить (S)"
        );
        style::block_button(
            c,
            [right_x + 6.0, save_y + 10.0],
            &style::FOLDER,
            save_lbl,
            None,
            7.5,
            false,
            hit::Hit::JourneySave,
            22.0,
        );
    }

    // Smooth card entrance animation (slide from right + fade in, matching song select).
    if enter < 1.0 {
        c.transform_since(card_start, [0.0, 0.0], 1.0, (1.0 - enter) * 14.0, 0.0);
        c.fade_since(card_start, 0.25 + 0.75 * enter, 1.0);
    }

    // 3. Bottom action bar — unified with the reworked Song Select style (no duplicate Save).
    let ay = cy + ch + 8.0;
    let mid = ay + actions_h / 2.0;
    let bh = actions_h - 4.0;
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
        bh,
    );

    let play_w = 148.0;
    let play_x = cx + cw - play_w;
    let skip_label = tr!("Skip", "スキップ", "Скип");
    let skip_w = Canvas::text_width(8.0, skip_label) + 34.0;
    let skip_x = play_x - 10.0 - skip_w + 6.0;
    let mut right_left = skip_x - 6.0;
    style::block_button(
        c,
        [skip_x, mid],
        &SKIP_ICON,
        skip_label,
        None,
        8.0,
        false,
        hit::Hit::Key(KeyCode::Tab),
        bh,
    );
    if state.journey.can_go_back() {
        let back_label = tr!("Back", "戻る", "Назад");
        let back_w = Canvas::text_width(8.0, back_label) + 34.0;
        let back_x = skip_x - 10.0 - back_w;
        right_left = back_x - 6.0;
        style::block_button(
            c,
            [back_x, mid],
            &PREV_ICON,
            back_label,
            None,
            8.0,
            false,
            hit::Hit::Key(KeyCode::ArrowLeft),
            bh,
        );
    }
    let from = cx + mods_w + 10.0;
    let room = (right_left - 10.0 - from).max(0.0);
    crate::screens::select::song_select::draw_action_mods_and_rate(
        c,
        from,
        mid,
        bh,
        room,
        state.cfg.mods,
        state.cfg.rate,
    );

    let play_label = if is_ready {
        tr!("Play", "プレイ", "Играть")
    } else if is_failed {
        tr!("Next", "次へ", "Дальше")
    } else {
        tr!("Loading...", "読込中...", "Загрузка...")
    };
    let bar = [play_x, ay + 2.0, play_w, bh];
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
        "warmup" => ("WARM-UP", 0x4ee07a),
        "stretch" => ("STRETCH", 0xff4d5a),
        "custom" => ("CUSTOM", 0x22d4c0),
        "explore" => ("EXPLORE", 0xa66cff),
        "cooldown" => ("COOL-DOWN", 0x4ea8ff),
        _ => ("FOR YOU", 0xffd147),
    }
}

impl State {
    /// Opens the Journey screen and starts fetching (or falls back to local library if offline).
    pub(crate) fn open_journey(&mut self) {
        self.main_menu = None;
        self.picking_lobby_map = false;
        self.journey.reset();
        self.journey.screen_entered = Instant::now();
        if self.cfg.online_token.is_empty() {
            self.journey.gate = Some(Gate::SignedOut);
        }
        self.journey_refill();
        self.scene = JourneyScreen::scene();
        self.journey_preview_current();
    }

    pub(crate) fn exit_journey(&mut self) {
        self.journey.stop();
        self.show_select();
    }

    pub(crate) fn open_mods(&mut self) {
        self.open_mod_select();
    }

    pub(crate) fn journey_pick_level(&mut self, idx: usize) {
        let Some(&(_, stars)) = LEVEL_CHOICES.get(idx) else {
            return;
        };
        self.journey.level_cursor = idx + usize::from(level_picker_has_auto(self));
        self.journey.set_manual_level(stars);
        self.journey_refill();
        self.journey_preview_current();
    }

    /// Switches Journey back to server-estimated Auto comfort stars and refreshes the queue.
    pub(crate) fn journey_pick_auto(&mut self) {
        self.journey.level_cursor = 0;
        self.journey.set_auto_level();
        self.journey_refill();
        self.journey_preview_current();
    }

    /// Steps the manual target star rating by `delta * 0.25*` and refreshes the queue.
    pub(crate) fn journey_step_level(&mut self, delta: i32) {
        let base = self
            .journey
            .target_stars
            .unwrap_or(self.journey.comfort_stars);
        let next = ((base * 4.0).round() / 4.0 + delta as f64 * 0.25).clamp(1.0, 11.0);
        self.journey.set_manual_level(next);
        self.journey_refill();
        self.journey_preview_current();
    }

    /// Opens the comfort level picker overlay inside Journey.
    pub(crate) fn journey_open_level_picker(&mut self) {
        let has_auto = level_picker_has_auto(self);
        self.journey.level_cursor = if has_auto && self.journey.target_stars.is_none() {
            0
        } else {
            let cur = self.journey.comfort_stars;
            let best = LEVEL_CHOICES
                .iter()
                .enumerate()
                .min_by(|(_, (_, a)), (_, (_, b))| (a - cur).abs().total_cmp(&(b - cur).abs()))
                .map(|(i, _)| i)
                .unwrap_or(2);
            best + usize::from(has_auto)
        };
        self.journey.needs_level = true;
    }

    pub(crate) fn journey_tick(&mut self) {
        let was_offline = self.journey.offline == Some(Gate::ServerDown);
        let fetched = self.journey.poll_fetch(&self.library);
        let dl_done = self.journey.poll_downloads();
        if was_offline && self.journey.offline.is_none() {
            self.toast(
                tr!(
                    "Server is back: Journey picks online again",
                    "サーバーが復帰: オンラインの選曲に戻りました",
                    "Сервер вернулся: Journey снова подбирает онлайн"
                )
                .into(),
            );
        }
        let now = Instant::now();
        // Failed fetches are retried quietly while there's a map to play, and for a few seconds
        // with none: the server often drops only that long.
        if self.journey.gate.is_none()
            && !self.journey.local_mode
            && !self.journey.needs_level
            && self.journey.queue.is_empty()
            && self.journey.outage(now)
        {
            self.journey.gate = Some(Gate::ServerDown);
        }
        if fetched || dl_done {
            self.journey_preview_current();
        }
        if !self.cfg.online_token.is_empty() && self.journey.probe_due(now) {
            self.journey.probe(
                self.cfg.online_server.clone(),
                self.cfg.online_token.clone(),
                self.cfg.scroll_speed,
            );
        }
        if self.journey.can_refill(now) {
            self.journey_refill();
        }
    }

    fn journey_refill(&mut self) {
        if self.journey.gate == Some(Gate::SignedOut) || !self.journey.can_refill(Instant::now()) {
            return;
        }
        // A server known to be up without Journey leaves the library. Otherwise Journey asks:
        // the account check may not have answered yet, or the server may be down a moment.
        let without_journey =
            !self.journey_supported && matches!(self.mp_menu.online, Some(Ok(())));
        let can_online = !self.cfg.online_token.is_empty() && !without_journey;
        if can_online && !self.journey.local_mode {
            self.journey.request_next(
                self.cfg.online_server.clone(),
                self.cfg.online_token.clone(),
                self.cfg.scroll_speed,
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
        let rate = self.play_speed().0;
        let stars = &self.stars;
        let info = self.select.info.as_ref().map(|m| m.lock().unwrap());
        self.journey.fill_from_local(&self.library, |i| {
            let mi = info.as_deref().and_then(|v| v.get(i).copied().flatten());
            let st = stars
                .get(i, rate)
                .or_else(|| mi.map(|mi| mi.stars).filter(|s| s.is_finite() && *s > 0.1));
            let bpm = mi
                .map(|mi| mi.bpm)
                .filter(|b| b.is_finite() && *b > 0.0)
                .map_or(0.0, f64::from);
            let length_ms = mi
                .map(|mi| mi.length_ms)
                .filter(|l| l.is_finite() && *l > 0.0)
                .map_or(0, |l| l as u64);
            (st, bpm, length_ms)
        });
    }

    pub(crate) fn journey_preview_current(&mut self) {
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
        // A map gone back to had its verdict sent when it was first left; a library pick made
        // offline is no server map.
        if !skipped.revisited
            && !self.journey.local_mode
            && !self.cfg.online_token.is_empty()
            && !skipped.item.md5.starts_with("local:")
        {
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

    /// Back to the map shown before the current one.
    pub(crate) fn journey_back(&mut self) {
        if self.journey.go_back() {
            self.journey_preview_current();
        }
    }

    /// Leaves Journey for the account form: logging in (`register` false) or making an account.
    pub(crate) fn journey_login(&mut self, register: bool) {
        self.exit_journey();
        self.mp_menu.open(if register {
            multiplayer::Form::Register
        } else {
            multiplayer::Form::Main
        });
        self.goto_tab(2);
    }

    /// Goes on with the library only, for the reason Journey asked.
    pub(crate) fn journey_go_offline(&mut self) {
        let reason = self.journey.gate.unwrap_or(Gate::SignedOut);
        self.journey.go_offline(reason);
        self.journey_refill();
        self.journey_preview_current();
    }

    /// Asks the server again now instead of after the backoff.
    pub(crate) fn journey_retry(&mut self) {
        self.journey.retry_after = None;
        self.journey_refill();
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
                    // The difficulty picked when it was unpacked; gone from the cache since
                    // (pruned, deleted by hand), it can't be played.
                    if !entry.path.is_file() {
                        self.journey_drop_failed();
                        return;
                    }
                    let entry = library::Entry::from_parts(
                        entry.path.clone(),
                        entry.meta.clone(),
                        entry.size,
                        entry.modified,
                    );
                    self.journey.playing_cached = Some((folder.clone(), q.item.clone()));
                    match self.library.iter().position(|e| e.path == entry.path) {
                        Some(existing) => existing,
                        None => {
                            // Joins the end of the list: the indices handed out stay good.
                            self.append_maps(vec![entry]);
                            self.library.len() - 1
                        }
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
        // Cancel any background prefetch downloads/extractions before gameplay starts
        // so weak CPUs never stutter during play.
        self.journey.pause_background_downloads();
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
        // The map just played (from its results), else the one up on the Journey screen.
        let cached_info = if let Some((folder, item)) = self.journey.playing_cached.clone() {
            Some((folder, item))
        } else if let Some(q) = self.journey.current() {
            if let MapSource::Cached { folder, .. } = &q.source {
                Some((folder.clone(), q.item.clone()))
            } else {
                None
            }
        } else {
            None
        };

        let Some((folder, item)) = cached_info else {
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

        // osu!'s Songs folder: the one picked in Settings, else an osu! install found (never
        // taiko.rs's own, which the other choice is).
        let own = [
            Some(crate::config::own_songs_dir()),
            crate::config::own_dir("Songs"),
        ];
        let osu_songs = library::song_dirs()
            .into_iter()
            .find(|d| d.is_dir() && !own.iter().flatten().any(|o| o == d));
        match crate::journey::save_cached_folder(
            &folder,
            item.custom_id.is_some(),
            self.cfg.journey_save_to,
            osu_songs.as_deref(),
        ) {
            Ok(dest) => {
                // The maps the list has from the cache are where they were moved to now: the same
                // entries at the same indices (a play's results, Ctrl+R, still find theirs).
                let mut moved = Vec::new();
                for (i, e) in self.library.iter_mut().enumerate() {
                    if e.path.starts_with(&folder) {
                        e.path = crate::journey::moved_path(&e.path, &folder, &dest);
                        moved.push((i, e.path.clone(), e.modified, e.size));
                    }
                }
                if !moved.is_empty() {
                    self.invalidate_library_indices();
                    if let Some(info) = &self.mapinfo {
                        info.extend(moved);
                        self.mapinfo_sorted = false;
                    }
                }
                // The set's other difficulties join the list too.
                let fresh: Vec<_> = crate::journey::scan_single_folder(&dest)
                    .into_iter()
                    .filter(|e| !self.library.iter().any(|x| x.path == e.path))
                    .collect();
                if fresh.is_empty() {
                    self.select.refilter(&self.library);
                } else {
                    self.append_maps(fresh);
                }
                // Every queued map of the set is in the library now, and the play of it has
                // nothing left to save.
                self.journey.saved(&folder, &dest);
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
