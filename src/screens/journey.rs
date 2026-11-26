//! The Journey screen: an endless, level-and-taste-matched map feed drawn in the client's
//! own visual language (`style.rs`): a mini session lane across the top, one focused map card
//! in the middle, and a Song-Select-style action bar along the bottom.

use super::screen::{Frame, Key, Screen};
use crate::config::{JourneyFlow, JourneyFocus, JourneyLength, MarathonGap};
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
            .unwrap_or_else(|| {
                tr!(
                    "Choosing the next map",
                    "次の譜面を選んでいます",
                    "Выбирает следующую карту"
                )
                .into()
            });
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

        // A key on the Journey screen takes over from Non-stop's own start.
        state.journey.autostart = false;

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

        if let Some(row) = state.journey.settings {
            let visible = visible_settings(state.cfg.journey_flow);
            let max_row = visible.len().saturating_sub(1);
            let cur = row.min(max_row);
            match code {
                KeyCode::ArrowUp | KeyCode::KeyW => {
                    state.journey.settings = Some(cur.saturating_sub(1));
                }
                KeyCode::ArrowDown | KeyCode::KeyS => {
                    state.journey.settings = Some((cur + 1).min(max_row));
                }
                KeyCode::ArrowLeft | KeyCode::KeyA => state.journey_change_setting(cur, -1),
                KeyCode::ArrowRight
                | KeyCode::KeyD
                | KeyCode::Enter
                | KeyCode::NumpadEnter
                | KeyCode::Space => state.journey_change_setting(cur, 1),
                KeyCode::Escape | KeyCode::KeyO => state.journey_toggle_settings(),
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
            KeyCode::KeyO => state.journey_toggle_settings(),
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

        // Non-stop: the map after a play starts by itself once it's ready.
        if state.journey_autostart_due() {
            state.journey_play_current_with(true);
            return None;
        }

        let hits_before = canvas.hits.len();
        draw_top_lane(state, canvas, w);
        draw_center_card(state, canvas, w, h);
        if state.journey.settings.is_some() {
            // Clear underlying hit areas while the settings modal is open so clicks outside
            // close the modal instead of activating buttons underneath.
            canvas.hits.truncate(hits_before);
            draw_settings(state, canvas, w, h);
        }
        None
    }
}

/// Logical settings in Journey's settings panel. Only settings relevant to the active
/// `JourneyFlow` are shown and navigable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SettingId {
    Focus,
    Length,
    Chill,
    Flow,
    Gap,
    SkipIntro,
    Hardcore,
    MarathonSave,
}

fn visible_settings(flow: JourneyFlow) -> &'static [SettingId] {
    match flow {
        JourneyFlow::Off => &[
            SettingId::Focus,
            SettingId::Length,
            SettingId::Chill,
            SettingId::Flow,
        ],
        JourneyFlow::Nonstop => &[
            SettingId::Focus,
            SettingId::Length,
            SettingId::Chill,
            SettingId::Flow,
            SettingId::SkipIntro,
        ],
        JourneyFlow::Marathon => &[
            SettingId::Focus,
            SettingId::Length,
            SettingId::Chill,
            SettingId::Flow,
            SettingId::Gap,
            SettingId::SkipIntro,
            SettingId::Hardcore,
            SettingId::MarathonSave,
        ],
    }
}

fn setting_help(state: &State, id: SettingId) -> &'static str {
    match id {
        SettingId::Focus if state.journey.local_mode => tr!(
            "Focus picks maps rich in streams, tech or SV. Requires online mode (using library offline).",
            "フォーカスは連打・テクニカル・SVの濃い譜面を選びます（オンライン専用）。",
            "Фокус подбирает карты со стримами, техникой или SV. Работает только онлайн."
        ),
        SettingId::Focus => tr!(
            "Prefer maps rich in streams, long deathstreams, rhythm/tech or SV for their star rating.",
            "その難易度の中で連打・長い連打・テクニカル・SVが多い譜面を優先して選びます。",
            "Подбирает карты, насыщенные стримами, дэтстримами, техникой или SV для своих звёзд."
        ),
        SettingId::Length => {
            if state.cfg.journey_flow == JourneyFlow::Marathon
                && state.cfg.journey_length == JourneyLength::Any
            {
                tr!(
                    "Filter chart duration. In Marathon, Any caps maps at 5:00 so hour-long marathons don't ambush you.",
                    "譜面の長さを制限します。マラソンでは「おまかせ」でも5分以下に制限されます。",
                    "Лимит длины карты. В марафоне «Любые» ограничены 5:00, чтобы не кидало часовые карты."
                )
            } else {
                tr!(
                    "Filter recommended charts by duration (applied both online and in your local library).",
                    "おすすめの譜面を長さで絞り込みます（オンライン・ローカル共通）。",
                    "Ограничивает длину подбираемых карт (работает и онлайн, и в локальной библиотеке)."
                )
            }
        }
        SettingId::Chill => tr!(
            "Easier pace: maps half a star below your comfort level and no stretch maps. Your level stays unchanged.",
            "レベルより0.5☆やさしい譜面のみ、ストレッチなし。推定レベルは変わりません。",
            "Спокойный темп: карты на 0.5* ниже твоего уровня и без челлендж-карт. Уровень не падает."
        ),
        SettingId::Flow => match state.cfg.journey_flow {
            JourneyFlow::Off => tr!(
                "Manual: see results after each map and start the next one yourself.",
                "手動: 毎回リザルトを表示し、次の譜面は自分で始めます。",
                "Вручную: результаты после каждой карты, следующую запускаешь сам."
            ),
            JourneyFlow::Nonstop => tr!(
                "Non-stop: results count down 5s, then the next map starts automatically. Press N on the up-next card to swap.",
                "ノンストップ: リザルト5秒後に次の譜面が自動開始。次の譜面表示中にNで変更できます。",
                "Без перерывов: 5с на результаты, затем автостарт следующей. N при показе следующей — заменить."
            ),
            JourneyFlow::Marathon => tr!(
                "Marathon: maps play back-to-back with no results screen and a carried combo until you stop.",
                "マラソン: リザルトなしで譜面が連続し、コンボを引き継ぎます。止めるまで続きます。",
                "Марафон: карты идут подряд без результатов и со сквозным комбо, пока не остановишь."
            ),
        },
        SettingId::Gap => match state.cfg.journey_gap {
            MarathonGap::Wheel => tr!(
                "Wheel: a short intermission card between maps (~2.2s) where you can hold, swap, vote or save.",
                "ホイール: 譜面間に約2.2秒のカード画面を挟み、一時停止・変更・評価・保存ができます。",
                "Колесо: ~2.2с между картами с карточкой следующей карты, паузой, заменой, оценкой и сохранением."
            ),
            MarathonGap::Short => tr!(
                "Short: a brief transition (~0.8s) showing the next map before starting.",
                "短め: 約0.8秒だけ次の譜面を表示してから開始します。",
                "Короткое: быстрая пауза (~0.8с) с показом следующей карты перед стартом."
            ),
            MarathonGap::Seamless => tr!(
                "Seamless: no wheel. The next map preloads during play and crossfades straight in.",
                "シームレス: ホイールなし。プレイ中に次の譜面を先読みし、クロスフェードで連続開始します。",
                "Бесшовно: без экрана между картами. Следующая грузится заранее и стартует с кроссфейдом."
            ),
        },
        SettingId::SkipIntro => tr!(
            "Auto-started maps jump straight to just before the first note, skipping long intros.",
            "自動で始まる譜面は長いイントロを飛ばして最初のノーツ直前から始まります。",
            "Карты с автостартом начинаются прямо перед первой нотой, пропуская длинные вступления."
        ),
        SettingId::Hardcore => tr!(
            "Hardcore marathon: the first failed map ends the run immediately. Pause Skip and Retry are disabled.",
            "ハードコア: 1度でも失敗すると即終了。プレイ中のスキップとリトライは無効になります。",
            "Хардкор: первый же фейл сразу заканчивает марафон. Скип и ретрай в паузе отключены."
        ),
        SettingId::MarathonSave => tr!(
            "Automatically save maps played in Marathon to your library (folder in Settings > Library). Off keeps them in temporary cache.",
            "マラソンでプレイした譜面を自動でライブラリに保存します（保存先は設定 > ライブラリ）。オフ時は一時キャッシュのみ。",
            "Автоматически сохраняет сыгранные в марафоне карты в библиотеку (папка в Настройки > Библиотека). Выкл — только в кэше."
        ),
    }
}

fn focus_chip_label(f: JourneyFocus) -> &'static str {
    match f {
        JourneyFocus::Any => tr!("Any", "おまかせ", "Любые"),
        JourneyFocus::Streams => tr!("Streams", "連打", "Стримы"),
        JourneyFocus::LongStreams => tr!("Long", "長連打", "Дэтстрим"),
        JourneyFocus::Tech => tr!("Tech", "技術", "Техника"),
        JourneyFocus::Sv => "SV",
    }
}

/// Journey's settings modal: opaque over a dark backdrop scrim, split into Map Pool (always active)
/// and Session Flow (showing only controls relevant to the selected flow mode).
fn draw_settings(state: &State, c: &mut Canvas, w: f32, h: f32) {
    // Full-screen dark scrim: dims the bright Journey card underneath and catches outside clicks.
    c.rect(0.0, 0.0, w, h, rgba(0x080a12, 0.80));
    c.hit(0.0, 0.0, w, h, hit::Hit::JourneySettings);

    let flow = state.cfg.journey_flow;
    let visible = visible_settings(flow);
    let cursor = state
        .journey
        .settings
        .unwrap_or(0)
        .min(visible.len().saturating_sub(1));
    let mut active_id = visible[cursor];

    let pw = 472.0_f32.min(w - 28.0);
    let pad = 16.0;
    let inner_w = pw - pad * 2.0;

    // Height adapts cleanly to the active flow mode so there's never dead space or hidden rows.
    let sub_h = match flow {
        JourneyFlow::Off => 30.0,
        JourneyFlow::Nonstop => 36.0,
        JourneyFlow::Marathon => 130.0,
    };
    let ph = 288.0 + sub_h;
    let x = (w - pw) / 2.0;
    let y = ((h - ph) / 2.0).max(16.0);

    // Solid modal surface (0.99 alpha over 0.80 scrim: zero bleed-through).
    c.round_rect(
        [x, y],
        [pw, ph],
        10.0,
        rgba(style::PANEL_COLD, 0.99),
        rgba(style::tone::EDGE, 1.0),
        1.5,
    );
    // Swallow clicks on empty panel space so they don't dismiss the modal.
    c.hit(x, y, pw, ph, hit::Hit::JourneySettingsBody);

    // Header + close button.
    let heading = tr!("JOURNEY SETTINGS", "ジャーニー設定", "НАСТРОЙКИ JOURNEY");
    let tw = Canvas::text_width(12.0, heading);
    style::title(c, x + (pw - tw) / 2.0, y + 14.0, 12.0, 0xffffff, heading);
    let close_x = x + pw - 32.0;
    let close_y = y + 10.0;
    let close_h = c.hovered(close_x, close_y, 22.0, 22.0);
    c.round_rect(
        [close_x, close_y],
        [22.0, 22.0],
        style::radius::SMALL,
        rgba(if close_h { 0x34344a } else { 0x1b1e2c }, 1.0),
        rgba(if close_h { 0x8a8aa8 } else { style::tone::EDGE }, 0.8),
        1.0,
    );
    c.text_centered(
        close_x + 11.0,
        close_y + 6.5,
        9.0,
        rgba(if close_h { 0xffffff } else { 0xa8a8bc }, 1.0),
        "x",
    );
    c.hit(close_x, close_y, 22.0, 22.0, hit::Hit::JourneySettings);
    style::rule(c, x + pad, y + 38.0, inner_w);

    let row_index_of = |id: SettingId| visible.iter().position(|&v| v == id);

    // Helper for row highlight + keyboard selector brackets.
    let draw_row_bg = |c: &mut Canvas,
                       rx: f32,
                       ry: f32,
                       rw: f32,
                       rh: f32,
                       id: SettingId,
                       accent_col: u32,
                       active_id: &mut SettingId| {
        let selected = row_index_of(id) == Some(cursor);
        let hovered = c.hovered(rx, ry, rw, rh);
        if hovered {
            *active_id = id;
        }
        if selected {
            c.round_rect(
                [rx, ry],
                [rw, rh],
                6.0,
                rgba(accent_col, 0.14),
                rgba(0, 0.0),
                0.0,
            );
            style::selector(c, "journey-set", [rx - 2.0, ry - 2.0, rw + 4.0, rh + 4.0]);
        } else if hovered {
            c.round_rect(
                [rx, ry],
                [rw, rh],
                6.0,
                rgba(0x262634, 0.72),
                rgba(0, 0.0),
                0.0,
            );
        }
    };

    let mut cy = y + 48.0;
    let rx = x + pad;

    // --- SECTION 1: MAP POOL (always active) ---
    c.text(
        rx + 4.0,
        cy,
        6.5,
        rgba(0x22d4c0, 0.95),
        tr!("MAP POOL", "選曲フィルター", "ПОДБОР КАРТ"),
    );
    cy += 14.0;

    // 1. Focus (segmented chips)
    let row_h = 28.0;
    draw_row_bg(
        c,
        rx,
        cy,
        inner_w,
        row_h,
        SettingId::Focus,
        0x22c8b8,
        &mut active_id,
    );
    let offline_focus = state.journey.local_mode;
    let label_col = if offline_focus { 0x74728a } else { 0xffffff };
    c.text(
        rx + 8.0,
        cy + 9.5,
        9.0,
        rgba(label_col, 1.0),
        tr!("Focus", "フォーカス", "Фокус"),
    );
    let seg_left = rx + 96.0;
    let seg_w = inner_w - 102.0;
    let chip_gap = 4.0;
    let chip_w =
        (seg_w - chip_gap * (JourneyFocus::ALL.len() - 1) as f32) / JourneyFocus::ALL.len() as f32;
    for (k, &f) in JourneyFocus::ALL.iter().enumerate() {
        let bx = seg_left + k as f32 * (chip_w + chip_gap);
        let by = cy + 4.0;
        let bh = row_h - 8.0;
        let on = state.cfg.journey_focus == f;
        let hov = c.hovered(bx, by, chip_w, bh);
        let fill = if on {
            rgba(if offline_focus { 0x4a4a62 } else { 0x22d4c0 }, 0.24)
        } else if hov {
            rgba(0x2d3042, 0.95)
        } else {
            rgba(0x161926, 0.95)
        };
        let edge = if on {
            rgba(if offline_focus { 0x6a6a84 } else { 0x22d4c0 }, 0.9)
        } else if hov {
            rgba(style::tone::EDGE_HOVER, 0.9)
        } else {
            rgba(style::tone::EDGE, 0.65)
        };
        c.round_rect(
            [bx, by],
            [chip_w, bh],
            style::radius::SMALL,
            fill,
            edge,
            1.0,
        );
        let lbl = focus_chip_label(f);
        c.text_centered(
            bx + chip_w / 2.0,
            by + (bh - 7.5) / 2.0,
            7.5,
            rgba(
                if on {
                    if offline_focus { 0xb4b4c8 } else { 0x62f2df }
                } else if offline_focus {
                    0x6e6e84
                } else {
                    0xc4c4d8
                },
                1.0,
            ),
            &Canvas::fit(7.5, chip_w - 6.0, lbl),
        );
        c.hit(bx, by, chip_w, bh, hit::Hit::JourneyFocus(f));
    }
    cy += 32.0;

    // 2. Length (segmented chips)
    draw_row_bg(
        c,
        rx,
        cy,
        inner_w,
        row_h,
        SettingId::Length,
        0x22c8b8,
        &mut active_id,
    );
    c.text(
        rx + 8.0,
        cy + 9.5,
        9.0,
        rgba(0xffffff, 1.0),
        tr!("Length", "長さ", "Длина"),
    );
    let marathon_flow = flow == JourneyFlow::Marathon;
    let lchip_w = (seg_w - chip_gap * (JourneyLength::ALL.len() - 1) as f32)
        / JourneyLength::ALL.len() as f32;
    for (k, &len) in JourneyLength::ALL.iter().enumerate() {
        let bx = seg_left + k as f32 * (lchip_w + chip_gap);
        let by = cy + 4.0;
        let bh = row_h - 8.0;
        let on = state.cfg.journey_length == len;
        let hov = c.hovered(bx, by, lchip_w, bh);
        let fill = if on {
            rgba(0x22d4c0, 0.24)
        } else if hov {
            rgba(0x2d3042, 0.95)
        } else {
            rgba(0x161926, 0.95)
        };
        let edge = if on {
            rgba(0x22d4c0, 0.9)
        } else if hov {
            rgba(style::tone::EDGE_HOVER, 0.9)
        } else {
            rgba(style::tone::EDGE, 0.65)
        };
        c.round_rect(
            [bx, by],
            [lchip_w, bh],
            style::radius::SMALL,
            fill,
            edge,
            1.0,
        );
        let lbl = len.chip(marathon_flow);
        c.text_centered(
            bx + lchip_w / 2.0,
            by + (bh - 7.5) / 2.0,
            7.5,
            rgba(if on { 0x62f2df } else { 0xc4c4d8 }, 1.0),
            &Canvas::fit(7.5, lchip_w - 6.0, lbl),
        );
        c.hit(bx, by, lchip_w, bh, hit::Hit::JourneyLength(len));
    }
    cy += 32.0;

    // 3. Chill (switch)
    let sw_row_h = 26.0;
    draw_row_bg(
        c,
        rx,
        cy,
        inner_w,
        sw_row_h,
        SettingId::Chill,
        0x22c8b8,
        &mut active_id,
    );
    if let Some(idx) = row_index_of(SettingId::Chill) {
        c.hit(rx, cy, inner_w, sw_row_h, hit::Hit::JourneySetting(idx, 1));
    }
    c.text(
        rx + 8.0,
        cy + 8.5,
        9.0,
        rgba(0xffffff, 1.0),
        tr!("Chill mode", "チルモード", "Чилл-режим"),
    );
    let chill_note = tr!(
        "-0.5* easier, no stretch maps",
        "-0.5* やさしめ、ストレッチなし",
        "-0.5* легче, без челлендж-карт"
    );
    c.text(
        rx + 102.0,
        cy + 9.5,
        7.5,
        rgba(0x8888a4, 1.0),
        &Canvas::fit(7.5, inner_w - 140.0, chill_note),
    );
    style::switch(
        c,
        [
            rx + inner_w - style::SWITCH[0] - 8.0,
            cy + (sw_row_h - style::SWITCH[1]) / 2.0,
        ],
        state.cfg.journey_chill,
    );
    cy += 30.0;

    style::rule(c, rx, cy, inner_w);
    cy += 10.0;

    // --- SECTION 2: SESSION FLOW ---
    c.text(
        rx + 4.0,
        cy,
        6.5,
        rgba(0x22d4c0, 0.95),
        tr!("SESSION FLOW", "進行モード", "РЕЖИМ СЕССИИ"),
    );
    cy += 14.0;

    // 4. Flow mode cards (Manual | Non-stop | Marathon)
    let flow_h = 30.0;
    draw_row_bg(
        c,
        rx,
        cy,
        inner_w,
        flow_h,
        SettingId::Flow,
        0x22c8b8,
        &mut active_id,
    );
    let fgap = 6.0;
    let fw = (inner_w - 12.0 - fgap * 2.0) / 3.0;
    for (k, &f) in JourneyFlow::ALL.iter().enumerate() {
        let bx = rx + 6.0 + k as f32 * (fw + fgap);
        let by = cy + 3.0;
        let bh = flow_h - 6.0;
        let on = flow == f;
        let hov = c.hovered(bx, by, fw, bh);
        let col = if f == JourneyFlow::Marathon {
            0xffd24a
        } else {
            0x22d4c0
        };
        let fill = if on {
            rgba(col, 0.22)
        } else if hov {
            rgba(0x2d3042, 0.95)
        } else {
            rgba(0x161926, 0.95)
        };
        let edge = if on {
            rgba(col, 0.92)
        } else if hov {
            rgba(style::tone::EDGE_HOVER, 0.9)
        } else {
            rgba(style::tone::EDGE, 0.65)
        };
        c.round_rect([bx, by], [fw, bh], style::radius::CONTROL, fill, edge, 1.0);
        c.text_centered(
            bx + fw / 2.0,
            by + (bh - 8.5) / 2.0,
            8.5,
            rgba(if on { 0xffffff } else { 0xb4b4c8 }, 1.0),
            &Canvas::fit(8.5, fw - 10.0, f.name()),
        );
        c.hit(bx, by, fw, bh, hit::Hit::JourneyFlow(f));
    }
    cy += flow_h + 8.0;

    // Contextual sub-card directly tied to the chosen Flow mode.
    let sub_accent = match flow {
        JourneyFlow::Off => style::tone::EDGE,
        JourneyFlow::Nonstop => 0x22d4c0,
        JourneyFlow::Marathon => 0xffd24a,
    };
    c.round_rect(
        [rx, cy],
        [inner_w, sub_h],
        style::radius::CARD,
        rgba(0x141724, 0.98),
        rgba(sub_accent, 0.35),
        1.0,
    );
    c.rect(rx + 1.0, cy + 5.0, 2.5, sub_h - 10.0, rgba(sub_accent, 0.9));

    let sub_rx = rx + 10.0;
    let sub_rw = inner_w - 16.0;
    match flow {
        JourneyFlow::Off => {
            let note = tr!(
                "Manual mode: results after each map, no auto-advance options.",
                "手動モード: 毎回リザルトを表示します（自動進行オプションなし）。",
                "Ручной режим: результаты после каждой карты, без автоперехода."
            );
            c.text(
                sub_rx + 4.0,
                cy + 11.0,
                7.5,
                rgba(0x8888a4, 1.0),
                &Canvas::fit(7.5, sub_rw - 8.0, note),
            );
        }
        JourneyFlow::Nonstop => {
            let sy = cy + 5.0;
            draw_row_bg(
                c,
                sub_rx,
                sy,
                sub_rw,
                26.0,
                SettingId::SkipIntro,
                0x22c8b8,
                &mut active_id,
            );
            if let Some(idx) = row_index_of(SettingId::SkipIntro) {
                c.hit(sub_rx, sy, sub_rw, 26.0, hit::Hit::JourneySetting(idx, 1));
            }
            c.text(
                sub_rx + 6.0,
                sy + 8.5,
                9.0,
                rgba(0xffffff, 1.0),
                tr!("Skip intros", "イントロをスキップ", "Скип интро"),
            );
            c.text(
                sub_rx + 100.0,
                sy + 9.5,
                7.5,
                rgba(0x8888a4, 1.0),
                &Canvas::fit(
                    7.5,
                    sub_rw - 138.0,
                    tr!(
                        "Jump to the first note",
                        "最初のノーツ直前から",
                        "Сразу к первой ноте"
                    ),
                ),
            );
            style::switch(
                c,
                [
                    sub_rx + sub_rw - style::SWITCH[0] - 6.0,
                    sy + (26.0 - style::SWITCH[1]) / 2.0,
                ],
                state.cfg.journey_skip_intro,
            );
        }
        JourneyFlow::Marathon => {
            c.text(
                sub_rx + 6.0,
                cy + 6.0,
                6.5,
                rgba(0xffd24a, 0.95),
                tr!("MARATHON OPTIONS", "マラソン設定", "НАСТРОЙКИ МАРАФОНА"),
            );
            let mut my = cy + 19.0;

            // Between maps (Wheel | Short | Seamless)
            draw_row_bg(
                c,
                sub_rx,
                my,
                sub_rw,
                26.0,
                SettingId::Gap,
                0xffd24a,
                &mut active_id,
            );
            c.text(
                sub_rx + 6.0,
                my + 8.5,
                8.5,
                rgba(0xffffff, 1.0),
                tr!("Between maps", "譜面の間", "Между картами"),
            );
            let g_left = sub_rx + 114.0;
            let g_w = (sub_rw - 120.0 - 8.0) / 3.0;
            for (k, &g) in MarathonGap::ALL.iter().enumerate() {
                let bx = g_left + k as f32 * (g_w + 4.0);
                let by = my + 3.0;
                let bh = 20.0;
                let on = state.cfg.journey_gap == g;
                let hov = c.hovered(bx, by, g_w, bh);
                let fill = if on {
                    rgba(0xffd24a, 0.22)
                } else if hov {
                    rgba(0x2d3042, 0.95)
                } else {
                    rgba(0x161926, 0.95)
                };
                let edge = if on {
                    rgba(0xffd24a, 0.9)
                } else if hov {
                    rgba(style::tone::EDGE_HOVER, 0.9)
                } else {
                    rgba(style::tone::EDGE, 0.65)
                };
                c.round_rect([bx, by], [g_w, bh], style::radius::SMALL, fill, edge, 1.0);
                c.text_centered(
                    bx + g_w / 2.0,
                    by + (bh - 7.5) / 2.0,
                    7.5,
                    rgba(if on { 0xffd24a } else { 0xc4c4d8 }, 1.0),
                    &Canvas::fit(7.5, g_w - 6.0, g.name()),
                );
                c.hit(bx, by, g_w, bh, hit::Hit::MarathonGap(g));
            }
            my += 28.0;

            // Skip intros
            let mrow_h = 24.0;
            draw_row_bg(
                c,
                sub_rx,
                my,
                sub_rw,
                mrow_h,
                SettingId::SkipIntro,
                0x22c8b8,
                &mut active_id,
            );
            if let Some(idx) = row_index_of(SettingId::SkipIntro) {
                c.hit(sub_rx, my, sub_rw, mrow_h, hit::Hit::JourneySetting(idx, 1));
            }
            c.text(
                sub_rx + 6.0,
                my + 7.5,
                8.5,
                rgba(0xffffff, 1.0),
                tr!("Skip intros", "イントロをスキップ", "Скип интро"),
            );
            style::switch(
                c,
                [
                    sub_rx + sub_rw - style::SWITCH[0] - 6.0,
                    my + (mrow_h - style::SWITCH[1]) / 2.0,
                ],
                state.cfg.journey_skip_intro,
            );
            my += 26.0;

            // Hardcore
            let hc_col = if state.cfg.journey_hardcore {
                style::DANGER
            } else {
                0x22c8b8
            };
            draw_row_bg(
                c,
                sub_rx,
                my,
                sub_rw,
                mrow_h,
                SettingId::Hardcore,
                hc_col,
                &mut active_id,
            );
            if let Some(idx) = row_index_of(SettingId::Hardcore) {
                c.hit(sub_rx, my, sub_rw, mrow_h, hit::Hit::JourneySetting(idx, 1));
            }
            c.text(
                sub_rx + 6.0,
                my + 7.5,
                8.5,
                rgba(
                    if state.cfg.journey_hardcore {
                        style::DANGER
                    } else {
                        0xffffff
                    },
                    1.0,
                ),
                tr!("Hardcore", "ハードコア", "Хардкор"),
            );
            let hc_note = tr!(
                "First fail ends run (no Skip/Retry)",
                "最初の失敗で終了（スキップ/リトライ不可）",
                "Первый фейл = конец (без скипа/ретрая)"
            );
            c.text(
                sub_rx + 86.0,
                my + 8.5,
                7.0,
                rgba(0x8888a4, 1.0),
                &Canvas::fit(7.0, sub_rw - 122.0, hc_note),
            );
            style::switch(
                c,
                [
                    sub_rx + sub_rw - style::SWITCH[0] - 6.0,
                    my + (mrow_h - style::SWITCH[1]) / 2.0,
                ],
                state.cfg.journey_hardcore,
            );
            my += 26.0;

            // Save played maps
            draw_row_bg(
                c,
                sub_rx,
                my,
                sub_rw,
                mrow_h,
                SettingId::MarathonSave,
                0x22c8b8,
                &mut active_id,
            );
            if let Some(idx) = row_index_of(SettingId::MarathonSave) {
                c.hit(sub_rx, my, sub_rw, mrow_h, hit::Hit::JourneySetting(idx, 1));
            }
            c.text(
                sub_rx + 6.0,
                my + 7.5,
                8.5,
                rgba(0xffffff, 1.0),
                tr!("Save maps", "譜面を保存", "Сохранять карты"),
            );
            let save_note = tr!(
                "Keep played maps in your library",
                "プレイした譜面をライブラリに保存",
                "Оставлять сыгранные в библиотеке"
            );
            let save_lbl_w =
                Canvas::text_width(8.5, tr!("Save maps", "譜面を保存", "Сохранять карты"));
            let note_x = sub_rx + 14.0 + save_lbl_w;
            c.text(
                note_x,
                my + 8.5,
                7.0,
                rgba(0x8888a4, 1.0),
                &Canvas::fit(
                    7.0,
                    (sub_rx + sub_rw - style::SWITCH[0] - 14.0 - note_x).max(40.0),
                    save_note,
                ),
            );
            style::switch(
                c,
                [
                    sub_rx + sub_rw - style::SWITCH[0] - 6.0,
                    my + (mrow_h - style::SWITCH[1]) / 2.0,
                ],
                state.cfg.journey_marathon_save,
            );
        }
    }
    cy += sub_h + 10.0;

    // Footer help strip: full wrapped description (never truncated with '...').
    c.round_rect(
        [rx, cy],
        [inner_w, 36.0],
        style::radius::SMALL,
        rgba(0x0a0c15, 0.96),
        rgba(style::tone::EDGE, 0.45),
        1.0,
    );
    let help = setting_help(state, active_id);
    for (k, line) in crate::mobile::wrap(7.0, inner_w - 16.0, help, 2)
        .into_iter()
        .enumerate()
    {
        c.text(
            rx + 8.0,
            cy + 7.0 + k as f32 * 12.0,
            7.0,
            rgba(0xb4b4cc, 1.0),
            &line,
        );
    }

    let hint = tr!(
        "Up/Down select   Left/Right change   Esc close",
        "上下で選択   左右で変更   Esc 閉じる",
        "Вверх/Вниз выбор   Влево/Вправо менять   Esc закрыть"
    );
    c.text_centered(
        x + pw / 2.0,
        y + ph - 16.0,
        7.0,
        rgba(0x787890, 1.0),
        &Canvas::fit(7.0, inner_w, hint),
    );
}

/// How long before the last note, and from how long into a break, Non-stop/Marathon shows up-next.
const UP_NEXT_LEAD_MS: f64 = 12_000.0;
/// A gap between notes at least this long is a break.
const BREAK_MS: f64 = 5_000.0;

/// Whether Non-stop/Marathon shows what's up next at map time `t`: towards the end of the map,
/// after it, and in its breaks (not right at their edges).
pub(crate) fn up_next_due(notes: &[beatmap::Note], t: f64) -> bool {
    let Some(last) = notes.last() else {
        return false;
    };
    if t >= last.time - UP_NEXT_LEAD_MS {
        return true;
    }
    let i = notes.partition_point(|n| n.time <= t);
    let (Some(prev), Some(next)) = (i.checked_sub(1).map(|p| &notes[p]), notes.get(i)) else {
        return false;
    };
    let prev_end = prev.end_time.max(prev.time);
    next.time - prev_end >= BREAK_MS && t - prev_end >= 500.0 && next.time - t >= 1_500.0
}

fn format_length_short(ms: u64) -> Option<String> {
    if ms == 0 {
        return None;
    }
    let s = ms / 1000;
    Some(format!("{}:{:02}", s / 60, s % 60))
}

/// In-play "Up next" card in the bottom-right corner with smooth entrance (`alpha` 0..=1),
/// difficulty-colored accent bar, map duration, and `[N] Swap` keycap.
pub(crate) fn draw_up_next(c: &mut Canvas, next: Option<&online::JourneyItem>, alpha: f32) {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 0.01 {
        return;
    }
    let start = c.shapes.len();
    let pw = 252.0;
    let ph = 54.0;
    let slide_x = (1.0 - alpha) * 14.0;
    let x = c.width - pw - 12.0 + slide_x;
    let y = c.height - ph - 12.0;
    let stripe_col = next.filter(|i| i.stars > 0.1).map_or(0x22d4c0, |i| {
        style::role::rating(None, Some(i.stars as f32))
    });

    style::cold_panel(c, [x, y, pw, ph]);
    c.round_rect(
        [x, y],
        [pw, ph],
        style::radius::CARD,
        rgba(0, 0.0),
        rgba(stripe_col, 0.35),
        1.0,
    );
    c.rect(x + 1.0, y + 6.0, 2.5, ph - 12.0, rgba(stripe_col, 0.95));

    c.text(
        x + 10.0,
        y + 7.0,
        6.5,
        rgba(0x22d4c0, 1.0),
        tr!("UP NEXT", "次の譜面", "ДАЛЬШЕ"),
    );
    match next {
        Some(item) => {
            let mut right_meta = format!("{:.2}*", item.stars);
            if let Some(len) = format_length_short(item.length_ms) {
                right_meta.push_str(&format!("  {len}"));
            }
            let mw = Canvas::text_width(7.0, &right_meta);
            c.text(
                x + pw - 10.0 - mw,
                y + 7.0,
                7.0,
                rgba(0xffd24a, 0.95),
                &right_meta,
            );

            let title = format!("{} - {}", item.artist, item.title);
            c.text(
                x + 10.0,
                y + 20.0,
                8.5,
                rgba(0xffffff, 1.0),
                &Canvas::fit(8.5, pw - 20.0, &title),
            );

            let swap = tr!("N: swap", "N: 変更", "N: заменить");
            let sw = Canvas::text_width(6.5, swap) + 10.0;
            let sx = x + pw - 10.0 - sw;
            let sy = y + 34.0;
            c.round_rect(
                [sx, sy],
                [sw, 13.0],
                3.0,
                rgba(0x1c2030, 0.95),
                rgba(style::tone::EDGE, 0.7),
                1.0,
            );
            c.text_centered(sx + sw / 2.0, sy + 3.5, 6.5, rgba(0xb4b4cc, 1.0), swap);

            let ver = format!("[{}]", item.version);
            c.text(
                x + 10.0,
                y + 36.0,
                7.0,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(7.0, (sx - (x + 10.0) - 8.0).max(20.0), &ver),
            );
        }
        None => {
            c.text(
                x + 10.0,
                y + 23.0,
                8.0,
                rgba(0x9a9ab0, 1.0),
                tr!(
                    "Picking the next map...",
                    "次の譜面を選んでいます...",
                    "Подбираем следующую карту..."
                ),
            );
        }
    }
    c.fade_since(start, alpha, 1.0);
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

pub(crate) const SKIP_ICON: style::Icon = [
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

    // Bottom of banner: title, artist, difficulty + mapper, plus active Journey filter chips on
    // the right so players can see active Focus/Length/Chill/Flow without opening Settings.
    let mut active_tags: Vec<(&str, u32)> = Vec::new();
    if state.cfg.journey_flow == JourneyFlow::Marathon {
        active_tags.push((tr!("MARATHON", "マラソン", "МАРАФОН"), 0xffd24a));
        if state.cfg.journey_hardcore {
            active_tags.push((tr!("HARDCORE", "ハードコア", "ХАРДКОР"), style::DANGER));
        }
    } else if state.cfg.journey_flow == JourneyFlow::Nonstop {
        active_tags.push((tr!("NON-STOP", "ノンストップ", "НОН-СТОП"), 0x22d4c0));
    }
    if state.cfg.journey_focus != JourneyFocus::Any && !state.journey.local_mode {
        active_tags.push((focus_chip_label(state.cfg.journey_focus), 0xa66cff));
    }
    if state.cfg.journey_length != JourneyLength::Any {
        active_tags.push((state.cfg.journey_length.chip(false), 0x4ea8ff));
    }
    if state.cfg.journey_chill {
        active_tags.push((tr!("CHILL", "チル", "ЧИЛЛ"), 0x4ee07a));
    }
    let mut tags_w = 0.0f32;
    if !active_tags.is_empty() {
        let tag_y = cy + banner_h - 22.0;
        let mut rx_tag = cx + cw - pad;
        for (lbl, col) in active_tags.iter().rev() {
            let tw_tag = Canvas::text_width(6.5, lbl) + 10.0;
            rx_tag -= tw_tag;
            c.round_rect(
                [rx_tag, tag_y],
                [tw_tag, 14.0],
                style::radius::SMALL,
                rgba(*col, 0.16),
                rgba(*col, 0.72),
                1.0,
            );
            c.text_centered(
                rx_tag + tw_tag / 2.0,
                tag_y + 3.5,
                6.5,
                rgba(*col, 1.0),
                lbl,
            );
            c.hit(rx_tag, tag_y, tw_tag, 14.0, hit::Hit::JourneySettings);
            rx_tag -= 5.0;
        }
        tags_w = (cx + cw - pad - rx_tag).max(0.0);
    }

    let title_y = cy + banner_h - 54.0;
    let title = Canvas::fit(15.0, inner_w, &item.title);
    style::title(c, cx + pad, title_y, 15.0, 0xffffff, &title);
    let artist = Canvas::fit(9.5, inner_w, &item.artist);
    c.text(cx + pad, title_y + 21.0, 9.5, rgba(0xd0d0e4, 1.0), &artist);
    let diff_text = format!("[{}]  by {}", item.version, item.creator);
    let diff_room = (inner_w - tags_w - 8.0).max(80.0);
    c.text(
        cx + pad,
        title_y + 36.0,
        8.5,
        rgba(role_col, 0.95),
        &Canvas::fit(8.5, diff_room, &diff_text),
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
    let gear_lit = state.cfg.journey_focus != JourneyFocus::Any
        || state.cfg.journey_length != JourneyLength::Any
        || state.cfg.journey_chill
        || state.cfg.journey_flow != JourneyFlow::Off;
    let gear_w = style::block_button(
        c,
        [cx + 6.0 + mods_w + 8.0, mid],
        &style::GEAR,
        tr!("Journey", "ジャーニー", "Journey"),
        None,
        8.0,
        gear_lit,
        hit::Hit::JourneySettings,
        bh,
    );
    let mods_w = mods_w + 8.0 + gear_w;

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

pub(crate) fn role_badge(role: &str) -> (&'static str, u32) {
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
        self.journey.focus = self.cfg.journey_focus;
        self.journey.length = self.cfg.journey_length;
        self.journey.marathon_cap = self.cfg.journey_flow == JourneyFlow::Marathon;
        self.journey.chill = self.cfg.journey_chill;
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

    /// Journey's upkeep away from its screen (a marathon): answers and downloads come in and the
    /// queue is kept filled, with no preview played.
    pub(crate) fn journey_tick_quiet(&mut self) {
        self.journey.poll_fetch(&self.library);
        self.journey.poll_downloads();
        if self.journey.can_refill(Instant::now()) {
            self.journey_refill();
        }
    }

    pub(crate) fn journey_refill(&mut self) {
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
        self.journey_drop_unplayable();
        self.journey_preview_current();
    }

    /// Goes past a map that can't be played (its download failed, its file is gone), telling
    /// the server nothing: it isn't the player's verdict.
    pub(crate) fn journey_drop_unplayable(&mut self) {
        self.journey.advance(PastOutcome::Skipped);
        self.journey.pool_exhausted = false;
        self.journey_refill();
    }

    pub(crate) fn journey_skip_current(&mut self) {
        let preview_secs = self.journey.shown_at.elapsed().as_secs_f64();
        if self.journey_skip_quiet(preview_secs) {
            self.journey_preview_current();
        }
    }

    /// Skips the current map (after `seen_secs` of it), telling the server, with no preview of
    /// the next one; false when there was none.
    pub(crate) fn journey_skip_quiet(&mut self, seen_secs: f64) -> bool {
        let Some(skipped) = self.journey.advance(PastOutcome::Skipped) else {
            return false;
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
                preview_secs: seen_secs,
            };
            std::thread::spawn(move || {
                let _ = online::call(&server, req);
            });
        }
        self.journey_refill();
        true
    }

    /// Back to the map shown before the current one.
    pub(crate) fn journey_back(&mut self) {
        if self.journey.go_back() {
            self.journey_preview_current();
        }
    }

    /// Journey's gear: its settings panel comes up, or goes.
    pub(crate) fn journey_toggle_settings(&mut self) {
        self.journey.settings = match self.journey.settings {
            Some(_) => None,
            None => Some(0),
        };
    }

    /// Steps a visible row of Journey's settings panel: kept in config, rebuilding the queue
    /// whenever the effective map filter changed.
    pub(crate) fn journey_change_setting(&mut self, row: usize, dir: i32) {
        let visible = visible_settings(self.cfg.journey_flow);
        let Some(&id) = visible.get(row) else {
            return;
        };
        self.journey.settings = Some(row);
        match id {
            SettingId::Focus => self.cfg.journey_focus = self.cfg.journey_focus.step(dir),
            SettingId::Length => self.cfg.journey_length = self.cfg.journey_length.step(dir),
            SettingId::Chill => self.cfg.journey_chill = !self.cfg.journey_chill,
            SettingId::Flow => self.cfg.journey_flow = self.cfg.journey_flow.step(dir),
            SettingId::Gap => self.cfg.journey_gap = self.cfg.journey_gap.step(dir),
            SettingId::SkipIntro => self.cfg.journey_skip_intro = !self.cfg.journey_skip_intro,
            SettingId::Hardcore => self.cfg.journey_hardcore = !self.cfg.journey_hardcore,
            SettingId::MarathonSave => {
                self.cfg.journey_marathon_save = !self.cfg.journey_marathon_save;
            }
        }
        self.journey_apply_settings();
    }

    pub(crate) fn journey_set_focus(&mut self, focus: JourneyFocus) {
        self.journey.settings = visible_settings(self.cfg.journey_flow)
            .iter()
            .position(|&s| s == SettingId::Focus);
        self.cfg.journey_focus = focus;
        self.journey_apply_settings();
    }

    pub(crate) fn journey_set_length(&mut self, length: JourneyLength) {
        self.journey.settings = visible_settings(self.cfg.journey_flow)
            .iter()
            .position(|&s| s == SettingId::Length);
        self.cfg.journey_length = length;
        self.journey_apply_settings();
    }

    pub(crate) fn journey_set_flow(&mut self, flow: JourneyFlow) {
        self.journey.settings = visible_settings(flow)
            .iter()
            .position(|&s| s == SettingId::Flow);
        self.cfg.journey_flow = flow;
        self.journey_apply_settings();
    }

    pub(crate) fn journey_set_gap(&mut self, gap: MarathonGap) {
        self.journey.settings = visible_settings(self.cfg.journey_flow)
            .iter()
            .position(|&s| s == SettingId::Gap);
        self.cfg.journey_gap = gap;
        self.journey_apply_settings();
    }

    fn journey_apply_settings(&mut self) {
        if let Some(row) = self.journey.settings {
            let max_row = visible_settings(self.cfg.journey_flow)
                .len()
                .saturating_sub(1);
            self.journey.settings = Some(row.min(max_row));
        }
        self.cfg.save();
        let marathon_cap = self.cfg.journey_flow == JourneyFlow::Marathon;
        if self.journey.set_style(
            self.cfg.journey_focus,
            self.cfg.journey_chill,
            self.cfg.journey_length,
            marathon_cap,
        ) {
            self.journey_refill();
            self.journey_preview_current();
        }
    }

    /// Non-stop: the map up on the Journey screen is to start by itself now (it's ready, and
    /// nothing is asked of the player first).
    pub(crate) fn journey_autostart_due(&mut self) -> bool {
        if !self.journey.autostart {
            return false;
        }
        if self.cfg.journey_flow != JourneyFlow::Nonstop
            || self.journey.gate.is_some()
            || self.journey.needs_level
            || self.journey.settings.is_some()
        {
            self.journey.autostart = false;
            return false;
        }
        let due = self
            .journey
            .current()
            .is_some_and(|q| q.source.is_ready() || matches!(q.source, MapSource::Failed(_)));
        if due {
            self.journey.autostart = false;
        }
        due
    }

    /// Non-stop / Marathon, in a play: no to the map up next. The server hears of it as a skip,
    /// and any marathon preload of the discarded map is dropped so the replacement can preload.
    pub(crate) fn journey_skip_upcoming(&mut self) {
        let Some(skipped) = self.journey.skip_upcoming() else {
            return;
        };
        if let Some(m) = &mut self.journey.marathon {
            m.preload = None;
        }
        if !self.journey.local_mode
            && !self.cfg.online_token.is_empty()
            && !skipped.md5.starts_with("local:")
        {
            let server = self.cfg.online_server.clone();
            let req = online::Request::JourneySkip {
                token: self.cfg.online_token.clone(),
                md5: skipped.md5,
                map_hash: skipped.map_hash,
                preview_secs: 0.0,
            };
            std::thread::spawn(move || {
                let _ = online::call(&server, req);
            });
        }
        // Downloads wait for the play to end; the refill only asks the server.
        self.journey_refill();
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
        self.journey_play_current_with(false);
    }

    /// Plays the map up in Journey; `by_itself`: Non-stop started it (its intro may be skipped).
    /// A marathon starts on its own screen instead.
    pub(crate) fn journey_play_current_with(&mut self, by_itself: bool) {
        if self.cfg.journey_flow == JourneyFlow::Marathon {
            if self.journey.current().is_some() {
                self.marathon_start();
            }
            return;
        }
        let Some(entry_idx) = self.journey_current_entry(false) else {
            return;
        };
        // Cancel any background prefetch downloads/extractions before gameplay starts
        // so weak CPUs never stutter during play.
        self.journey.pause_background_downloads();
        self.journey.in_play = true;
        self.journey.skip_intro_next = by_itself && self.cfg.journey_skip_intro;
        self.start_loading_from(entry_idx, None);
    }

    /// The library index of the map up in Journey, made ready to play (a cached one joins the
    /// library): None while it's still on its way (said with a toast unless `quiet`) or when it
    /// can't be played (then it's gone past, with no preview of the next when `quiet`).
    pub(crate) fn journey_current_entry(&mut self, quiet: bool) -> Option<usize> {
        let give_up = |s: &mut State| {
            if quiet {
                s.journey_drop_unplayable();
            } else {
                s.journey_drop_failed();
            }
        };
        let q = self.journey.current()?;
        let entry_idx = match &q.source {
            MapSource::Local(path) | MapSource::Saved(path) => {
                let path = path.clone();
                self.journey.playing_cached = None;
                let Some(idx) = self.library.iter().position(|e| e.path == path) else {
                    give_up(self);
                    return None;
                };
                idx
            }
            MapSource::Cached { folder, entry } => {
                // The difficulty picked when it was unpacked; gone from the cache since
                // (pruned, deleted by hand), it can't be played.
                if !entry.path.is_file() {
                    give_up(self);
                    return None;
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
                if !quiet {
                    self.toast(
                        tr!(
                            "Still downloading this map...",
                            "譜面をダウンロード中...",
                            "Карта ещё скачивается..."
                        )
                        .into(),
                    );
                }
                return None;
            }
            MapSource::Failed(_) => {
                // Never send a negative JourneySkip signal for a failed download.
                give_up(self);
                return None;
            }
        };
        Some(entry_idx)
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
        self.journey_save_current_with(true);
    }

    pub(crate) fn journey_save_current_with(&mut self, toast_on_success: bool) {
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
            if toast_on_success {
                self.toast(
                    tr!(
                        "Already in your library",
                        "すでにライブラリにあります",
                        "Карта уже в библиотеке"
                    )
                    .into(),
                );
            }
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
                if toast_on_success {
                    let where_saved = self.cfg.journey_save_to.name();
                    self.toast(tr!(
                        "Saved to {where_saved}",
                        "{where_saved}に保存しました",
                        "Сохранено в: {where_saved}";
                        [] [where_saved]
                    ));
                }
            }
            Err(e) => {
                self.toast(
                    tr!("Save failed: {e}", "保存失敗: {e}", "Ошибка сохранения: {e}"; [] [e]),
                );
            }
        }
    }
}
