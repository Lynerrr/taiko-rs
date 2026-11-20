//! Song select as 1.0 drew it (the list, the details beside it, and Mods, Random and Play under
//! them), for its look under Settings > System > Legacy menu (`Config::legacy_menu`).

use super::*;

/// Song select itself, the main menu not over it: `draw_songs` as it was in 1.0.
pub(super) fn draw_songs_classic(state: &mut State, canvas: &mut Canvas, frame: &Frame) {
    // DT / HT show on the Mods button instead.
    state.select.rate_label =
        if (state.cfg.rate - 1.0).abs() > 1e-6 && state.cfg.mods.rate_override().is_none() {
            format!(
                "  |  {:.2}x{}",
                state.cfg.rate,
                if state.cfg.rate_keep_pitch {
                    ""
                } else {
                    " (pitch)"
                }
            )
        } else {
            String::new()
        };
    let (stars, rate, plays) = (&state.stars, state.play_speed().0, &state.scores);
    state.select.scrubbing =
        (state.right_scroll || state.scrollbar_drag).then(|| state.mouse_virtual().1);
    state.select.list_share = state.cfg.song_list_share;
    state.select.banner = state.pick_to_edit || state.picking_lobby_map;
    let phone = mobile::active();
    // Under the open sort and filter panel nothing lights up for the pointer: it's on the
    // panel, drawn over all this.
    let pointer = canvas.mouse;
    if state.select.lens_ui.is_some() {
        canvas.mouse = (f32::MIN, f32::MIN);
    }
    if phone {
        state.draw_mobile_select(canvas);
    } else {
        state.select.draw_classic(
            canvas,
            &state.library,
            frame.dt,
            &state.status,
            &|i| stars.get(i, rate),
            &|e| scores::best_grade(plays, e.hash()),
        );
    }
    let lw = Select::list_width(canvas.width, state.cfg.song_list_share);
    // The panel beside the list: from `x`, `pw` wide, `panel_top` down to `panel_end`.
    let [x, panel_top, pw, panel_end] = if phone {
        state.mobile_select_panel(canvas.width)
    } else {
        let x = 24.0 + lw + 12.0;
        // Level with the list's frame (the Legacy layout keeps its own).
        let [top, end] = if state.select.legacy {
            [54.0, 436.0]
        } else {
            state.select.frame_span()
        };
        [x, top, canvas.width - x - 24.0, end]
    };
    state.board_rect = None;
    // Always at full size (no longer dragged smaller).
    state.preview.size = preview::PREVIEW_SIZE.1;
    let selected_stars = state
        .select
        .selected()
        .and_then(|i| state.stars.get(i, rate));
    state.preview.potential = match state.select.selected() {
        Some(i) => state.potentials.get(
            &state.library[i].path,
            rate,
            state.cfg.mods,
            state.cfg.scroll_speed,
        ),
        None => None,
    };
    // The map the panel shows, and whether it's marked EPIC (its pill by the status).
    let shown_path = state
        .preview_target()
        .and_then(|i| state.library.get(i))
        .map(|e| e.path.clone());
    state.preview.epic = state
        .preview_target()
        .and_then(|i| state.library.get(i))
        .is_some_and(|e| state.select.is_epic(e));
    if phone {
        let status = state.select.selected().and_then(|i| state.map_status(i));
        let liked = state
            .preview_target()
            .and_then(|i| state.library.get(i))
            .map(|e| state.select.is_liked(e));
        let rect = [x, panel_top, pw, panel_end];
        state.draw_mobile_panels(canvas, rect, selected_stars, status, liked);
    } else {
        state.leaderboard.clear();
    }
    if pw > 160.0 && !phone {
        let status = state.select.selected().and_then(|i| state.map_status(i));
        let liked = state
            .preview_target()
            .and_then(|i| state.library.get(i))
            .map(|e| state.select.is_liked(e));
        // Two pages side by side, as on a phone: the preview, and the leaderboard
        // the button slides to.
        let t = state.mob.slide_board(canvas.dt_ms);
        let step = pw + 24.0;
        // The page buttons sit under both pages, in what's left below the preview.
        const PAGE_BUTTON_H: f32 = 22.0;
        let panel_h = panel_end - panel_top - PAGE_BUTTON_H - 8.0;
        let clip = [x - 2.0, panel_top - 2.0, pw + 4.0, panel_h + 4.0];
        if t < 1.0 {
            let start = canvas.shapes.len();
            // The way to the leaderboard is where the way back is on the board (its
            // header's top right), so the mouse stays put going back and forth.
            let label = tr!("Leaderboard >", "ランキング >", "Рейтинг >");
            let bw = Canvas::text_width(7.0, label) + 24.0;
            state.preview.title_room = bw + 10.0;
            if let Some(clip) = state.preview.draw_panel_classic(
                canvas,
                &state.library,
                x - t * step,
                panel_top,
                pw,
                panel_h,
                state.cfg.scroll_speed,
                selected_stars,
                Some(("Play from", "Shift+Enter")),
                status,
                liked,
            ) {
                state.frame_clip = Some(clip);
            }
            state.preview.title_room = 0.0;
            // An EPIC map: the music's spectrum round its pill, and on landing on it a
            // glint across the panel and sparks out of the pill.
            if let Some([ex, ey]) = state.preview.epic_at {
                let engine = state.audio.as_ref();
                let fx = &mut state.epic_fx;
                let sounding = !state.music_paused
                    && state
                        .preview
                        .sounding(engine, Instant::now(), &mut fx.spectrum.samples);
                let rate = engine.map_or(48_000, |a| a.rate);
                fx.spectrum.update(canvas.dt_ms, sounding, rate);
                crate::epic::aura(canvas, ex, ey, fx.spectrum.bands(), 1.0);
            }
            let fx = &mut state.epic_fx;
            fx.burst.show(
                shown_path.as_deref(),
                state.preview.epic,
                state.preview.epic_at,
                render::animations(),
            );
            // The panel as drawn: it shrinks to what it shows.
            let bottom = state.preview.panel_bottom.min(panel_top + panel_h);
            let panel = [x - t * step, panel_top, pw, bottom - panel_top];
            fx.burst.draw(canvas, panel, canvas.dt_ms);
            // As the board's back button: 10 in from the frame, then the rows' scroll
            // bar room (`board_tabs`, inside `pw - 28`).
            let (bx, by) = (x - t * step + pw - 18.0 - bw, panel_top + 10.0);
            let hit = hit::Hit::BoardPage;
            canvas.button(bx, by, bw, 20.0, label, 7.0, hit, false);
            canvas.clip_since(start, clip[0], clip[1], clip[2], clip[3]);
        } else {
            state.preview.hide_panel();
        }
        let board_start = canvas.shapes.len();
        let page_x = x + (1.0 - t) * step;
        if t > 0.0 {
            style::panel(canvas, page_x, panel_top, pw, panel_h);
        }
        // Inside the frame, with room right of the rows for their scroll bar.
        let (panel_x, panel_w) = (x, pw);
        let (x, pw) = (page_x + 10.0, pw - 28.0);
        if let Some(i) = state.select.selected().filter(|_| t > 0.0) {
            let top = panel_top + 10.0;
            let room = panel_end - 10.0 - top - BOARD_HEAD;
            let fits = (room / BOARD_ROW).floor().max(0.0) as usize;
            if fits > 0 {
                let hash = state.library[i].hash();
                let relax = state.cfg.mods.relax();
                if state.board_for != (hash, state.board_online, relax) {
                    state.board_for = (hash, state.board_online, relax);
                    state.board_scroll = 0;
                }
                state.board_rect = Some([x, top + BOARD_HEAD, pw + 18.0, fits as f32 * BOARD_ROW]);
                // TJA charts have online boards only on a server with TJA boards.
                if state.board_online && state.has_online_board(&state.library[i]) {
                    let board = state.online_board(hash, relax).cloned();
                    let total = board
                        .as_ref()
                        .and_then(|b| b.as_ref().ok())
                        .map_or(0, Vec::len);
                    state.board_max = total.saturating_sub(fits);
                    state.board_scroll = state.board_scroll.min(state.board_max);
                    draw_online_leaderboard(
                        canvas,
                        [x, top],
                        pw,
                        board.as_ref(),
                        &state.cfg.online_user,
                        fits,
                        state.board_scroll,
                        relax,
                    );
                    state.online_leaderboard = match board {
                        Some(Ok(rows)) => rows
                            .into_iter()
                            .skip(state.board_scroll)
                            .take(fits)
                            .collect(),
                        _ => Vec::new(),
                    };
                } else {
                    let all = scores::top(&state.scores, hash, usize::MAX, relax);
                    state.board_max = all.len().saturating_sub(fits);
                    state.board_scroll = state.board_scroll.min(state.board_max);
                    let rows: Vec<&scores::ScoreRow> = all
                        .iter()
                        .skip(state.board_scroll)
                        .take(fits)
                        .copied()
                        .collect();
                    state.leaderboard = rows.iter().map(|r| r.file.clone()).collect();
                    draw_leaderboard(
                        canvas,
                        x,
                        top,
                        pw,
                        &rows,
                        state.board_scroll,
                        all.len(),
                        relax,
                    );
                }
            }
        }
        canvas.clip_since(board_start, clip[0], clip[1], clip[2], clip[3]);
        let play = if state.picking_lobby_map {
            tr!("Pick", "選ぶ", "Выбрать")
        } else if state.pick_to_edit {
            tr!("Edit", "編集", "Править")
        } else {
            tr!("Play", "プレイ", "Играть")
        };
        let mods = &state.select.mods_label;
        let mods_label = if mods.is_empty() {
            tr!("Mods", "Mod", "Моды").to_string()
        } else {
            format!("{}: {mods}", tr!("Mods", "Mod", "Моды"))
        };
        let random = tr!("Random", "ランダム", "Случайная");
        // (label, hit, lit, share of the width). The Legacy layout has Mods on top.
        let mut buttons = vec![
            (
                mods_label.as_str(),
                hit::Hit::ModsButton,
                !mods.is_empty(),
                1.0,
            ),
            (random, hit::Hit::Key(KeyCode::F2), false, 1.0),
            (play, hit::Hit::Key(KeyCode::Enter), true, 1.3),
        ];
        if state.select.legacy {
            buttons.drain(..2);
        }
        let total: f32 = buttons.iter().map(|b| b.3).sum();
        let unit = (panel_w - 8.0 * (buttons.len() as f32 - 1.0)) / total;
        let (mut bx, by) = (panel_x, panel_end - PAGE_BUTTON_H);
        for (label, hit, lit, share) in buttons {
            let bw = unit * share;
            let label = Canvas::fit(8.0, bw - 12.0, label);
            canvas.button(bx, by, bw, PAGE_BUTTON_H, &label, 8.0, hit, lit);
            bx += bw + 8.0;
        }
        // The song list's right edge: the gap between the list's box (`ui`, ending
        // at 28 + lw) and the panel (from 36 + lw), the grip in its middle.
        let dragging = matches!(state.split_drag, Some((hit::Hit::SplitList, ..)));
        if !phone {
            draw_divider(
                canvas,
                [24.0 + lw + 4.0, panel_top, 8.0, panel_end - panel_top],
                true,
                dragging,
                hit::Hit::SplitList,
            );
        }
    }
    let guessing = state.mp.as_ref().is_some_and(|s| s.mapguessr);
    if let Some(g) = state
        .guessr
        .as_mut()
        .filter(|_| state.picking_lobby_map && guessing)
    {
        mapguessr::draw_picking(canvas, g, state.cfg.scroll_speed, Instant::now());
    } else if state.pick_to_edit && !phone {
        let w = canvas.width;
        canvas.round_rect(
            [24.0, 438.0],
            [w - 48.0, 34.0],
            8.0,
            rgba(0x1d1630, 1.0),
            rgba(0x9b6cff, 1.0),
            1.5,
        );
        canvas.text_centered(
            w / 2.0,
            450.0,
            10.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(
                10.0,
                w - 64.0,
                "Choose a map to edit: Enter opens it in the editor, Esc cancels",
            ),
        );
    } else if state.picking_lobby_map && !phone {
        let w = canvas.width;
        canvas.round_rect(
            [24.0, 438.0],
            [w - 48.0, 34.0],
            8.0,
            rgba(themed(0x2b1a17, 0x15151d, 0.84), 1.0),
            rgba(accent(), 1.0),
            1.5,
        );
        canvas.text_centered(
            w / 2.0,
            450.0,
            10.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(
                10.0,
                w - 64.0,
                "Picking the lobby map: Enter to choose, Esc to cancel",
            ),
        );
    }
    canvas.mouse = pointer;
    state.select.draw_popups(canvas, &state.library);
    if let Some(naming) = &state.select.naming {
        super::super::collections::draw_collection_naming(canvas, naming);
    }
    if let Some(ask) = &state.select.deleting {
        super::super::delete::draw_delete_ask(canvas, ask);
    }
    if let Some(ask) = &state.select.moving {
        super::super::move_custom::draw_move_ask(canvas, ask);
    }
    if state.cfg.drum_menus {
        state.select_rows = drum_nav::Rows::of(canvas);
        state.select_zone = state.select_zone.within(&state.select_rows);
        drum_nav::draw(canvas, state.select_zone, &state.select_rows);
    }
}
