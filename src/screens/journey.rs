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
            KeyCode::Minus | KeyCode::BracketLeft => state.journey_step_level(-1),
            KeyCode::Equal | KeyCode::BracketRight => state.journey_step_level(1),
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

    // Interactive comfort star-rating control: [-] [~7.25* JOURNEY] [+] (0.25* steps).
    let mode_tag = if state.journey.local_mode {
        let local = tr!("LOCAL", "ローカル", "ЛОКАЛЬНО");
        format!("~{:.2}*  {local}", state.journey.comfort_stars)
    } else {
        format!("~{:.2}*  JOURNEY", state.journey.comfort_stars)
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
    let slide = state.journey.lane_slide;
    c.rect(
        lane_left,
        mid_y - 1.0,
        lane_right - lane_left,
        2.0,
        rgba(0x2a2a3c, 1.0),
    );
    let center_x = (lane_left + lane_right) / 2.0;
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
        if nx > lane_right - 14.0 || nx < center_x + 16.0 {
            continue;
        }
        let (_, ncol) = role_badge(&next_q.item.role);
        let alpha = if next_q.source.is_ready() { 0.85 } else { 0.4 };
        c.circle(nx, mid_y, 6.0, rgba(ncol, alpha), rgba(0xffffff, 0.35), 1.0);
    }

    let rx = center_x + 112.0;
    if rx + Canvas::text_width(7.0, role_label) < lane_right {
        c.text(rx, mid_y - 3.5, 7.0, rgba(role_col, 0.95), role_label);
    }
}

fn current_entry_for_preview<'a>(
    journey: &'a crate::journey::Journey,
    library: &'a [library::Entry],
) -> Option<&'a library::Entry> {
    let cur = journey.current()?;
    match &cur.source {
        MapSource::Local(p) | MapSource::Saved(p) => library.iter().find(|e| &e.path == p),
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
        style::cold_panel(c, [cx, cy, cw, ch]);
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
    let (item, is_ready, is_in_library, dl_progress, is_failed, status_str) = cur_cloned;
    let (role_label, role_col) = role_badge(&item.role);
    let bg_path = current_entry_for_preview(&state.journey, &state.library)
        .and_then(crate::preview::background_path);
    let od_val = current_entry_for_preview(&state.journey, &state.library).map(|e| e.meta.od);
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

    // Top-left role pill + difficulty delta + reason.
    let delta = if item.stars > 0.1 && state.journey.comfort_stars > 0.1 {
        Some(item.stars - state.journey.comfort_stars)
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
    let reason_room = (cw - pad * 2.0 - pill_w - 130.0).max(40.0);
    c.text(
        reason_x,
        cy + 15.5,
        7.5,
        rgba(0xd4d4e8, 0.92),
        &Canvas::fit(7.5, reason_room, &item.reason),
    );

    // Top-right star chip + source badge.
    let stars_opt = (item.stars > 0.05).then_some(item.stars as f32);
    let star_w = style::star_chip(c, cx + cw - pad, cy + 11.0, 9.5, None, stars_opt);
    let source_text = if item.custom_id.is_some() {
        tr!("taiko.rs custom", "taiko.rsカスタム", "кастомка")
    } else if state.journey.local_mode {
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
    let (bpm_val, len_ms, dons, kats, rolls, swells, pot_pp) = match loaded_stats {
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
            st.potential,
        ),
        None => (item.bpm, item.length_ms, None, None, 0, 0, None),
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
        current_entry_for_preview(&state.journey, &state.library).filter(|_| is_ready)
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

    // Difficulty vs comfort gauge.
    let comfort = state.journey.comfort_stars.max(1.0);
    let diff_label = if item.stars > 0.1 {
        let d = item.stars - comfort;
        format!("~{comfort:.1}*  ->  {:.2}* ({d:+.1}*)", item.stars)
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

    // Gauge track: blue (easier) -> green/gold (comfort) -> red (stretch).
    let g_h = 6.0;
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
        right_x + right_w * 0.36,
        ry + 1.0,
        right_w * 0.28,
        g_h - 2.0,
        rgba(0x4ee07a, 0.35),
    );
    // Stretch zone tint on the right.
    c.rect(
        right_x + right_w * 0.64,
        ry + 1.0,
        right_w * 0.36 - 1.0,
        g_h - 2.0,
        rgba(0xff4d5a, 0.28),
    );
    // Marker for current map.
    if item.stars > 0.1 {
        let frac = ((item.stars - comfort + 1.5) / 3.0).clamp(0.04, 0.96) as f32;
        let mx = right_x + right_w * frac;
        let mcol = if item.stars > comfort + 0.25 {
            0xff4d5a
        } else if item.stars < comfort - 0.3 {
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
    style::block_button(
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
    style::block_button(
        c,
        [play_x - 10.0 - skip_w + 6.0, mid],
        &SKIP_ICON,
        skip_label,
        None,
        8.0,
        false,
        hit::Hit::Key(KeyCode::Tab),
        bh,
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
        self.journey.level_cursor = idx;
        self.journey.set_manual_level(stars);
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
        self.journey.needs_level = true;
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
        let rate = self.cfg.rate;
        let stars = &self.stars;
        let info = self.select.info.as_ref().map(|m| m.lock().unwrap().clone());
        self.journey.fill_from_local(&self.library, |i| {
            stars.get(i, rate).or_else(|| {
                info.as_ref()
                    .and_then(|v| v.get(i).copied().flatten())
                    .map(|mi| mi.stars)
                    .filter(|s| s.is_finite() && *s > 0.1)
            })
        });
        if let Some(info) = info {
            for q in &mut self.journey.queue {
                if let Some(idx) = q
                    .item
                    .md5
                    .strip_prefix("local:")
                    .and_then(|s| s.parse::<usize>().ok())
                    && let Some(mi) = info.get(idx).copied().flatten()
                {
                    if q.item.bpm <= 0.0 && mi.bpm.is_finite() && mi.bpm > 0.0 {
                        q.item.bpm = f64::from(mi.bpm);
                    }
                    if q.item.length_ms == 0 && mi.length_ms.is_finite() && mi.length_ms > 0.0 {
                        q.item.length_ms = mi.length_ms as u64;
                    }
                }
            }
        }
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
