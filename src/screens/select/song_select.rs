//! Song select itself, and the main menu over it (`State::on_menu`). What it shows is
//! `State::select`, which the screens of its group (`Screen::song_select`) share.

use super::super::lobby::Lobby;
use super::super::screen::{Frame, Key, Screen};
use crate::*;

/// Song select (and the main menu over it).
pub(crate) struct SongSelect;

/// The source tab Ctrl and a digit (1 to 4) go to, 0 first.
fn source_digit(code: KeyCode) -> Option<usize> {
    [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ]
    .iter()
    .position(|&k| k == code)
}

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
            at,
        } = key;
        // Any key closes the row menu; Esc does nothing else.
        if state.select.menu.take().is_some() && code == KeyCode::Escape {
            return;
        }
        if state.select.lens_ui.is_some() {
            if state.modifiers_ctrl && code == KeyCode::KeyS {
                state.select.lens_ui = None;
            } else if let Some(n) = source_digit(code).filter(|_| state.modifiers_ctrl) {
                if state.select.source_key(n, &state.library) {
                    state.lens_changed();
                }
            } else if state.select.lens_key(code, &state.library) {
                state.lens_changed();
            }
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
                state.menu_key(code, at);
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
            // The sort and filter panel; with Shift, straight to the next sort.
            () if state.modifiers_ctrl && code == KeyCode::KeyS => {
                if state.modifiers_shift {
                    state.cycle_sort(1);
                } else {
                    state.toggle_lens();
                }
            }
            () if state.modifiers_ctrl && source_digit(code).is_some() => {
                if let Some(n) = source_digit(code)
                    && state.select.source_key(n, &state.library)
                {
                    state.lens_changed();
                }
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
            // The search and the filters.
            () if state.modifiers_ctrl && code == KeyCode::Backspace => {
                state.select.filter.clear();
                state.select.tabs.filters = ui::Filters::default();
                state.select.refilter(&state.library);
            }
            // In the search box Delete clears what's typed, as in the game's other boxes.
            () if code == KeyCode::Delete && state.select.searching => {
                state.select.filter.clear();
                state.select.refilter(&state.library);
            }
            // The selected difficulty; with Shift, its whole set. Asked first.
            () if code == KeyCode::Delete => {
                if let Some(i) = state.select.selected() {
                    state.ask_delete(i, state.modifiers_shift);
                }
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
                SelectAction::Quit if state.cfg.main_menu => state.back_to_menu(),
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
        // The camera between the menu and song select, a frame on.
        let dt = canvas.dt_ms;
        let shift = match state.menu_shift.as_mut() {
            Some(s) if menu_shift::animated() => {
                s.step(dt).then(|| (s.t(), s.into_songs, s.flying))
            }
            _ => None,
        };
        if shift.is_none() {
            state.menu_shift = None;
        }
        // Song select's routes drift on wherever they're drawn, and swell with a kiai as the
        // menu's do; leaving, they clear off.
        state.song_routes.clock += dt;
        if !state.on_menu() {
            let kiai = state.cfg.kiai_effects && !state.music_paused && state.preview.kiai();
            let ms = if kiai { 90.0 } else { 260.0 };
            render::approach(
                &mut state.song_routes.kiai,
                f32::from(u8::from(kiai)),
                dt,
                ms,
            );
        }
        if let Some((_, e)) = &mut state.song_routes_drain {
            *e += dt / menu_notes::DRAIN_MS;
            if *e >= 1.0 {
                state.song_routes_drain = None;
            }
        }
        match shift {
            Some((t, into, flying)) => draw_shift(state, canvas, frame, t, into, flying),
            None if frame.on_menu => {
                let draining = state.song_routes_drain.is_some();
                draw_crowd(
                    state,
                    canvas,
                    frame.now,
                    false,
                    if draining { 1.0 } else { 0.0 },
                    0.0,
                );
                draw_menu(state, canvas, frame, Layer::Whole);
            }
            None => {
                draw_crowd(state, canvas, frame.now, true, 1.0, 1.0);
                draw_songs(state, canvas, frame, None, false);
            }
        }
        None
    }
}

/// What of the main menu a frame draws.
#[derive(Clone, Copy)]
enum Layer {
    /// All of it: the menu is up.
    Whole,
    /// Only the routes behind it, which stay put while the camera moves.
    Routes,
    /// The rest, moved by the camera (`cam` of a screen right), the note `away` left out.
    Front { cam: f32, away: Option<menu::Item> },
}

/// The main menu (or the music player over it), as much of it as `layer` says.
fn draw_menu(state: &mut State, canvas: &mut Canvas, frame: &Frame, layer: Layer) {
    if matches!(layer, Layer::Whole) {
        state.board_rect = None;
        state.leaderboard.clear();
    }
    let shown = state
        .player
        .as_ref()
        .and_then(|p| p.list.entry())
        .or(state.select.selected());
    let song = shown.and_then(|i| state.library.get(i)).map(Entry::label);
    let k = |i: usize| config::key_name(state.cfg.keys[i]);
    let (a, b, d, e) = (k(0), k(3), k(1), k(2));
    let paused = state.music_paused;
    let menu = match state
        .menu_shift
        .as_mut()
        .and_then(|s| s.menu.as_deref_mut())
    {
        Some(m) => Some(m),
        None => state.main_menu.as_mut(),
    };
    let start = canvas.shapes.len();
    if let Some(p) = state
        .player
        .as_mut()
        .filter(|_| !matches!(layer, Layer::Routes))
    {
        let now = Instant::now();
        let engine = state.audio.as_ref();
        let sounding = !paused && state.preview.sounding(engine, now, &mut p.spectrum.samples);
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
    } else if let Some(m) = menu {
        let kiai = state.cfg.kiai_effects && !paused && state.preview.kiai();
        let shows = state.cfg.menu_notes && render::animations();
        let playing = state.preview.playing_time();
        let ghosts = state
            .menu_notes
            .time(playing, frame.now)
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
            shifting: !matches!(layer, Layer::Whole),
            away: match layer {
                Layer::Front { away, .. } => away,
                _ => None,
            },
        };
        match layer {
            Layer::Routes => m.draw_routes(canvas, &frame, &mut state.menu_eggs),
            _ => m.draw(canvas, &frame, &mut state.menu_eggs),
        }
    }
    // The camera has moved on: the menu goes the other way.
    if let Layer::Front { cam, .. } = layer {
        let dx = -cam * canvas.width;
        canvas.transform_since(start, [0.0, 0.0], 1.0, dx, 0.0);
    }
}

/// The lane of actions under song select's preview in the main menu's look (all but Legacy):
/// its height, and how much of the screen's width its Play end takes.
const ACTIONS_H: f32 = 50.0;
const PLAY_SHARE: f32 = 0.16;

/// Song select itself. `shift`: the camera's on its way from or to the menu; `play_away`: the
/// Play ▶ is still flying to its button.
fn draw_songs(
    state: &mut State,
    canvas: &mut Canvas,
    frame: &Frame,
    shift: Option<menu_shift::Phases>,
    play_away: bool,
) {
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
    let left_start = canvas.shapes.len();
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
    // On the way from the menu: the search row comes on over the lane that folded into it,
    // and the list drops out of it, its bottom edge going down with it.
    if let Some(ph) = shift {
        let [top, end] = state.select.frame_span();
        let bottom = top + ph.drop * (end - top);
        canvas.fade_since(left_start, ph.landed, 1.0);
        // The list's window has no edge: its bottom is where the clip cuts it.
        canvas.clip_since(left_start, 0.0, 0.0, lw + 40.0, bottom);
    }
    // What's right of the list comes with the camera.
    let right_start = canvas.shapes.len();
    // The panel beside the list: from `x`, `pw` wide, `panel_top` down to `panel_end`.
    let [x, panel_top, pw, panel_end] = if phone {
        state.mobile_select_panel(canvas.width)
    } else {
        let x = 24.0 + lw + style::space::PAD;
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
        // The main menu's look (all but Legacy): no panel under the details, and the preview's
        // lane runs on off the screen's right edge.
        let bare = !state.select.legacy;
        // Under both pages: Legacy's one button, or the lane the menu's look puts the actions on.
        const PAGE_BUTTON_H: f32 = 22.0;
        let actions_h = if bare { ACTIONS_H } else { PAGE_BUTTON_H };
        let panel_h = panel_end - panel_top - actions_h - style::space::SECTION;
        let clip_w = if bare {
            canvas.width - x + 4.0
        } else {
            pw + 4.0
        };
        let clip = [x - 2.0, panel_top - 2.0, clip_w, panel_h + 4.0];
        if t < 1.0 {
            let start = canvas.shapes.len();
            // The way to the leaderboard is where the way back is on the board (its
            // header's top right), so the mouse stays put going back and forth.
            let label = tr!("Leaderboard", "ランキング", "Рейтинг");
            let legacy_label = format!("{label} >");
            let bw = if state.select.legacy {
                Canvas::text_width(7.0, &legacy_label) + 24.0
            } else {
                style::icon_button_width(label, None, 7.0)
            };
            state.preview.title_room = bw + 10.0;
            state.preview.bare = bare;
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
            state.preview.bare = false;
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
            if state.select.legacy {
                canvas.button(bx, by, bw, 20.0, &legacy_label, 7.0, hit, false);
            } else {
                let at = [bx + 6.0, by + 10.0];
                style::icon_button(canvas, at, &style::PODIUM, label, None, 7.0, false, hit);
            }
            canvas.clip_since(start, clip[0], clip[1], clip[2], clip[3]);
        } else {
            state.preview.hide_panel();
        }
        let board_start = canvas.shapes.len();
        let page_x = x + (1.0 - t) * step;
        if t > 0.0 {
            if bare {
                let page = [page_x, panel_top, pw, panel_h];
                style::surface(canvas, page, 10.0, style::Surface::Raised);
            } else {
                style::panel(canvas, page_x, panel_top, pw, panel_h);
            }
        }
        // Inside the frame, with room right of the rows for their scroll bar.
        let (panel_x, panel_w) = (x, pw);
        let (x, pw) = (page_x + 10.0, pw - 28.0);
        if let Some(i) = state.select.selected().filter(|_| t > 0.0) {
            let top = panel_top + 10.0;
            // Bare, the lane of actions is under the board: its rows stop above it.
            let bottom = if bare { panel_top + panel_h } else { panel_end };
            let room = bottom - 10.0 - top - BOARD_HEAD;
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
        let (by, bh) = (panel_end - actions_h, actions_h);
        if !bare {
            // The Legacy layout keeps its one button.
            let label = Canvas::fit(8.0, panel_w - 12.0, play);
            let hit = hit::Hit::Key(KeyCode::Enter);
            canvas.button(panel_x, by, panel_w, bh, &label, 8.0, hit, true);
        } else {
            // The main menu's look: a lane of its own from beside the list off the screen's
            // right edge (a raised surface, no lines round it), Mods and Random bare icons on
            // it, and Play its end, filled with the accent and running off the screen.
            let w = canvas.width;
            style::lane_surface(canvas, [panel_x, by, w - panel_x + 2.0, bh]);
            let play_x = w - (w * PLAY_SHARE).clamp(120.0, 200.0);
            let mid = by + bh / 2.0;
            let mods_word = tr!("Mods", "Mod", "Моды");
            // The mods that are on, two letters each, after the word.
            let on: Vec<&str> = (0..mods.len() / 2)
                .filter_map(|i| mods.get(i * 2..i * 2 + 2))
                .collect();
            let mods_label = if on.is_empty() {
                mods_word.to_string()
            } else {
                format!("{mods_word} {}", on.join(" "))
            };
            let mut bx = panel_x + 16.0;
            let room = play_x - bx - style::icon_button_width("", None, 8.0) - 6.0;
            let mods_label = Canvas::fit(8.0, room, &mods_label);
            bx += style::icon_button(
                canvas,
                [bx, mid],
                &style::SPARK,
                &mods_label,
                None,
                8.0,
                !mods.is_empty(),
                hit::Hit::ModsButton,
            );
            let random = tr!("Random", "ランダム", "Случайная");
            if bx + 18.0 + style::icon_button_width(random, None, 8.0) < play_x {
                // The menu's small mark between two notes.
                canvas.rect(bx + 4.0, mid - 2.0, 4.0, 4.0, render::rgba(0x6a6a88, 0.8));
                style::icon_button(
                    canvas,
                    [bx + 18.0, mid],
                    &style::DICE,
                    random,
                    None,
                    8.0,
                    false,
                    hit::Hit::Key(KeyCode::F2),
                );
            }
            let hit = hit::Hit::Key(KeyCode::Enter);
            let bar = [play_x, by, bh];
            state.play_icon_at = Some(style::lane_end_icon(bar));
            style::lane_end_button(canvas, "select-play", bar, play, hit, !play_away);
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
    if let Some(ph) = shift {
        let dx = (1.0 - ph.cam) * canvas.width;
        canvas.transform_since(right_start, [0.0, 0.0], 1.0, dx, 0.0);
    }
    canvas.mouse = pointer;
    state.select.draw_popups(canvas, &state.library);
    if let Some(naming) = &state.select.naming {
        super::collections::draw_collection_naming(canvas, naming);
    }
    if let Some(ask) = &state.select.deleting {
        super::delete::draw_delete_ask(canvas, ask);
    }
    if let Some(ask) = &state.select.moving {
        super::move_custom::draw_move_ask(canvas, ask);
    }
    if state.cfg.drum_menus {
        state.select_rows = drum_nav::Rows::of(canvas);
        state.select_zone = state.select_zone.within(&state.select_rows);
        drum_nav::draw(canvas, state.select_zone, &state.select_rows);
    }
}

/// The routes behind song select past the menu's own (and those too, `base`) in the frame at
/// `now`: `shown` of the way in, the camera `cam` of the way to song select, clearing off if it's
/// leaving.
fn draw_crowd(
    state: &mut State,
    canvas: &mut Canvas,
    now: Instant,
    base: bool,
    shown: f32,
    cam: f32,
) {
    let shows = state.cfg.menu_notes && render::animations();
    let playing = state.preview.playing_time();
    let Some(view) = state
        .menu_notes
        .time(playing, now)
        .filter(|_| shows)
        .and_then(|t| state.menu_notes.view(t))
    else {
        return;
    };
    let menu_notes::Drift {
        clock,
        universe,
        kiai,
    } = state.song_routes;
    // In a kiai each beat kicks them, as on the menu.
    let kick = state
        .preview
        .beat()
        .filter(|_| kiai > 0.0)
        .map_or(0.0, |(into, len)| {
            (1.0 - (into.rem_euclid(len) as f32) / menu::BEAT_MS).clamp(0.0, 1.0) * kiai
        });
    if base {
        let mut gold = menu_notes::Gold::default();
        menu_notes::draw(canvas, &view, clock, kiai, kick, universe, &mut gold);
    }
    let crowd = menu_notes::Crowd {
        shown,
        cam,
        kiai,
        kick,
        drain: state.song_routes_drain,
    };
    menu_notes::draw_more(canvas, &view, clock, universe, crowd);
}

/// A frame on the way between the menu and song select, `t` of the way to song select: the
/// routes stay, the menu's lane folds up into the search row as the camera moves right, song
/// select comes with the camera, the menu goes the other way, and the hit note (`flying`) flies
/// to its button.
fn draw_shift(
    state: &mut State,
    canvas: &mut Canvas,
    frame: &Frame,
    t: f32,
    into: bool,
    flying: Option<menu::Item>,
) {
    let w = canvas.width;
    let ph = menu_shift::phases(t);
    draw_menu(state, canvas, frame, Layer::Routes);
    let shown = if into { (t * 2.0).min(1.0) } else { 1.0 };
    draw_crowd(state, canvas, frame.now, false, shown, ph.cam);
    if ph.landed < 1.0 {
        let lw = Select::list_width(w, state.cfg.song_list_share);
        let top = state.select.frame_span()[0];
        let row = [20.0, 14.0, lw + 8.0, top - 14.0];
        // A kiai on the menu stays on its lane as the camera carries it off, going out as the
        // lane folds into the search row.
        let (edge, fill) = state
            .menu_shift
            .as_ref()
            .and_then(|s| s.menu.as_deref())
            .or(state.main_menu.as_ref())
            .map_or((0.0, 0.0), menu::Menu::kiai_glow);
        let left = 1.0 - ph.fold;
        let kiai = (edge * left, fill * left);
        menu_shift::draw_lane(canvas, &menu_shift::lane(t, w, row), 1.0, kiai);
    }
    let in_air = flying.filter(|_| ph.flight > 0.0 && ph.flight < 1.0);
    let songs_hits = canvas.hits.len();
    let play_away = flying.is_some() && ph.flight < 1.0;
    draw_songs(state, canvas, frame, Some(ph), play_away);
    let menu_hits = canvas.hits.len();
    let away = flying.filter(|_| ph.flight > 0.0);
    draw_menu(state, canvas, frame, Layer::Front { cam: ph.cam, away });
    // Only the screen being gone to takes clicks.
    if into {
        canvas.hits.truncate(menu_hits);
    } else {
        canvas.hits.drain(songs_hits..menu_hits);
    }
    let from = state
        .menu_shift
        .as_ref()
        .and_then(|s| s.menu.as_deref())
        .map(|m| m.spot(w));
    if let (Some(item), Some(from), Some((to, size))) = (in_air, from, state.play_icon_at) {
        let to = [to[0] + (1.0 - ph.cam) * w, to[1]];
        menu_shift::draw_flight(canvas, item, from, (to, size), ph.flight);
    }
}
