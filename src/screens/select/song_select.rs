//! Song select itself, and the main menu over it (`State::on_menu`). What it shows is
//! `State::select`, which the screens of its group (`Screen::song_select`) share.

use super::super::lobby::Lobby;
use super::super::screen::{Frame, Key, Screen};
use crate::*;

/// Song select (and the main menu over it).
pub(crate) struct SongSelect;

impl SongSelect {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(SongSelect))
    }
}

impl Screen for SongSelect {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Select"
    }

    fn activity(&self) -> &'static str {
        "song select"
    }

    fn presence(&self, _state: &State) -> Option<(String, String)> {
        None
    }

    fn select_screen(&self) -> bool {
        true
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            text,
            window,
            ..
        } = key;
        // Any key closes the row menu; Esc does nothing else.
        if state.select.menu.take().is_some() && code == KeyCode::Escape {
            return;
        }
        if let Some(cursor) = state.select.picker {
            if state.select.list == ui::PickList::Sorts {
                state.list_key(code, cursor);
            } else {
                state.collections_list_key(code, cursor);
            }
            return;
        }
        if state.mp.is_some()
            && !state.picking_lobby_map
            && code == KeyCode::Escape
            && state.select.filter.is_empty()
            && !state.select.searching
        {
            state.scene = Lobby::scene();
            return;
        }
        if state.picking_lobby_map && !repeat {
            match code {
                KeyCode::Escape if state.select.filter.is_empty() && !state.select.searching => {
                    state.picking_lobby_map = false;
                    state.scene = Lobby::scene();
                    return;
                }
                KeyCode::Tab | KeyCode::F1 => {
                    state.picking_lobby_map = false;
                    state.scene = Lobby::scene();
                    return;
                }
                _ => {}
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
        if state.on_menu() {
            if state.player.is_some() {
                state.player_key(code, text);
            } else {
                state.menu_key(code);
            }
            return;
        }
        match () {
            () if state.modifiers_ctrl && code == KeyCode::KeyF => state.select.searching = true,
            // In the search box the keys that have a job here otherwise (speed's [ and ]) type.
            () if state.select.searching
                && !state.modifiers_ctrl
                && matches!(code, KeyCode::BracketLeft | KeyCode::BracketRight) =>
            {
                if let Some(text) = text {
                    state.select.type_text(text, &state.library);
                }
            }
            () if state.modifiers_shift
                && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter)
                && !state.picking_lobby_map =>
            {
                state.play_from_marker();
            }
            () if state.modifiers_ctrl && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) => {
                state.open_preview()
            }
            () if state.modifiers_ctrl && code == KeyCode::KeyD => state.open_download(),
            () if state.modifiers_ctrl && code == KeyCode::KeyS => {
                let dir = if state.modifiers_shift { -1 } else { 1 };
                state.cycle_sort(dir);
            }
            () if state.modifiers_ctrl && code == KeyCode::KeyL => state.open_collections(),
            () if state.modifiers_ctrl && code == KeyCode::KeyK => state.collect_selected(),
            () if state.modifiers_ctrl && state.modifiers_shift && code == KeyCode::KeyU => {
                if let Some(i) = state.select.selected() {
                    state.update_set(i);
                }
            }
            // `Hotkey::RefreshSongs`, on whatever key the player gave it.
            () if code == KeyCode::F13 => state.refresh_library(true),
            () if code == KeyCode::F2 => {
                if state.modifiers_shift {
                    state.select.random_back();
                } else {
                    state.select.random(&state.library);
                }
            }
            () if state.modifiers_ctrl
                && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) =>
            {
                let step = if code == KeyCode::ArrowRight {
                    5000.0
                } else {
                    -5000.0
                };
                state.preview.seek_by(step, state.audio.as_ref());
            }
            () if matches!(code, KeyCode::BracketLeft | KeyCode::BracketRight) => {
                let step = if code == KeyCode::BracketRight {
                    0.05
                } else {
                    -0.05
                } * if state.modifiers_shift { 5.0 } else { 1.0 };
                state.cfg.rate = (((state.cfg.rate + step) * 100.0).round() / 100.0)
                    .clamp(RATE_RANGE.0, RATE_RANGE.1);
                state.cfg.save();
                let pitch = if state.cfg.rate_keep_pitch {
                    "keeps pitch"
                } else {
                    "pitch shifts"
                };
                state.toast(format!("speed {:.2}x ({pitch})", state.cfg.rate));
            }
            () if state.library.is_empty()
                && state.scan.is_none()
                && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter)
                && !cfg!(target_os = "android") =>
            {
                state.browse_songs_folder();
            }
            () if state.modifiers_ctrl && code == KeyCode::Backspace => {
                state.select.filter.clear();
                state.select.refilter(&state.library);
            }
            () => match state.select.key(code, &state.library) {
                SelectAction::Play(i) if state.picking_lobby_map => state.pick_lobby_map(i),
                SelectAction::Play(i) if state.pick_to_edit => {
                    state.pick_to_edit = false;
                    state.open_editor(i);
                }
                SelectAction::Play(i) => state.start_loading(i),
                SelectAction::Settings => state.open_settings(),
                SelectAction::Mods => state.open_mod_select(),
                SelectAction::Quit if state.cfg.main_menu => {
                    state.main_menu = Some(menu::Menu::new())
                }
                SelectAction::Quit => state.ask_quit(),
                SelectAction::None => {
                    if let Some(text) = text {
                        state.select.type_text(text, &state.library);
                    }
                }
            },
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        if frame.on_menu {
            state.board_rect = None;
            state.leaderboard.clear();
            let shown = state
                .player
                .as_ref()
                .and_then(|p| p.list.entry())
                .or(state.select.selected());
            let song = shown.and_then(|i| state.library.get(i)).map(Entry::label);
            let k = |i: usize| config::key_name(state.cfg.keys[i]);
            let (a, b, d, e) = (k(0), k(3), k(1), k(2));
            let paused = state.music_paused;
            if let Some(p) = &mut state.player {
                let now = Instant::now();
                let engine = state.audio.as_ref();
                let sounding =
                    !paused && state.preview.sounding(engine, now, &mut p.spectrum.samples);
                let hint = tr!("{a} {b} song   {d} {e} pause   L love   Tab folders", "{a} {b} 曲   {d} {e} ポーズ   L お気に入り   Tab フォルダ", "{a} {b} трек   {d} {e} пауза   L любимое   Tab папки"; [] [a, b, d, e]);
                let volume = state.cfg.music_volume;
                let frame = player::Frame {
                    song: song.as_deref().unwrap_or_default(),
                    hint: &hint,
                    paused,
                    times: state.preview.music_times(engine, now),
                    pictured: state.preview.has_background,
                    sounding,
                    rate: engine.map_or(48_000, |a| a.rate),
                    volume,
                };
                p.draw(canvas, &frame);
            } else if let Some(m) = &mut state.main_menu {
                let kiai = state.cfg.kiai_effects && !paused && state.preview.kiai();
                let shows = state.cfg.menu_notes && render::animations();
                let ghosts = state
                    .preview
                    .map_time(state.menu_notes.entry())
                    .filter(|_| shows)
                    .and_then(|t| state.menu_notes.view(t));
                let frame = menu::Frame {
                    beat: state.preview.beat(),
                    kiai,
                    ghosts,
                    lane_dim: state.cfg.menu_notes_dim,
                    song: song.as_deref(),
                    maps: state.library.len(),
                    paused,
                    session: (
                        frame
                            .now
                            .saturating_duration_since(state.started)
                            .as_secs_f64(),
                        state.session_play_ms / 1000.0,
                    ),
                    glitch: state.cfg.menu_glitch,
                };
                m.draw(canvas, &frame);
            }
        } else {
            // DT / HT show on the Mods button instead.
            state.select.rate_label = if (state.cfg.rate - 1.0).abs() > 1e-6
                && state.cfg.mods.rate_override().is_none()
            {
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
            if phone {
                state.draw_mobile_select(canvas);
            } else {
                state.select.draw(
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
                    if let Some(clip) = state.preview.draw_panel(
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
                        state.board_rect =
                            Some([x, top + BOARD_HEAD, pw + 18.0, fits as f32 * BOARD_ROW]);
                        if state.board_online {
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
            state.select.draw_popups(canvas, &state.library);
            if let Some(naming) = &state.select.naming {
                super::collections::draw_collection_naming(canvas, naming);
            }
            if state.cfg.drum_menus {
                state.select_rows = drum_nav::Rows::of(canvas);
                state.select_zone = state.select_zone.within(&state.select_rows);
                drum_nav::draw(canvas, state.select_zone, &state.select_rows);
            }
        }
        None
    }
}
