//! Drawing around a play: the replay bar and its pause button, watching everyone, and the
//! quick restarts after a fail.

#[allow(unused_imports)]
use crate::*;

/// A pause button's label, `held` or going.
pub(crate) fn pause_label(held: bool) -> &'static str {
    if held {
        tr!("Play", "再生", "Играть")
    } else {
        tr!("Pause", "ポーズ", "Пауза")
    }
}

/// A pause button's width: the same with either label, so nothing moves when it's pressed.
pub(crate) fn pause_width() -> f32 {
    Canvas::text_width(7.0, pause_label(true)).max(Canvas::text_width(7.0, pause_label(false)))
        + 12.0
}

/// Replays: a timeline along the bottom of the screen to click or drag through, over the
/// map's difficulty with the replay's misses marked. Returns the bar's rect (virtual units) for
/// `replay_seek_at`.
/// `grow`: how far it has grown towards its hovered height (0 to 1), kept between frames.
pub(crate) fn draw_replay_bar(
    c: &mut Canvas,
    game: &Game,
    active: bool,
    speed: f64,
    grow: &mut f32,
) -> [f32; 4] {
    let w = c.width;
    let (start, end) = game.span();
    let label = format!("{} / {}", fmt_time(game.time()), fmt_time(end));
    let y = 456.0;
    // Its buttons, then the time and the timeline.
    let mut left = 18.0;
    for (text, bw, hit) in [
        (pause_label(game.held()), pause_width(), KeyCode::Space),
        ("-", 20.0, KeyCode::ArrowDown),
        ("+", 20.0, KeyCode::ArrowUp),
    ] {
        c.button(
            left,
            y - 8.0,
            bw,
            16.0,
            text,
            7.0,
            hit::Hit::Key(hit),
            false,
        );
        left += bw + 4.0;
    }
    let shown = format!("{speed:.2}x");
    c.text(left + 2.0, y - 3.5, 7.0, rgba(0xc8c8d8, 1.0), &shown);
    left += Canvas::text_width(7.0, "0.00x") + 12.0;
    // Sized for the longest label so the bar doesn't twitch as the time changes.
    let lw = Canvas::text_width(7.0, &format!("{} / {}", fmt_time(end), fmt_time(end)));
    let (x, bw) = (left + lw + 10.0, w - 18.0 - (left + lw + 10.0));
    // The strip: difficulty bars standing on `base`, misses over them. Hovered or dragged, it
    // grows upwards to show more.
    let base = y + 1.0;
    let hover_at = |gh: f32| [x, base - gh, bw, gh + 2.0];
    let now_rect = hover_at(17.0 + *grow * 13.0);
    let hover = active
        || c.hovered(
            now_rect[0] - 6.0,
            now_rect[1],
            now_rect[2] + 12.0,
            now_rect[3],
        );
    render::approach(grow, if hover { 1.0 } else { 0.0 }, c.dt_ms, 60.0);
    let gh = 17.0 + *grow * 13.0;
    let rect = hover_at(gh);
    let top = base - gh;
    let span = (end - start).max(1.0);
    let x_of = |t: f64| x + (((t - start) / span).clamp(0.0, 1.0) as f32) * bw;
    let f = ((game.time() - start) / span).clamp(0.0, 1.0) as f32;
    c.text(left, y - 3.5, 7.0, rgba(0xc8c8d8, 1.0), &label);
    c.round_rect(
        [x - 2.0, top - 2.0],
        [bw + 4.0, gh + 4.0],
        4.0,
        rgba(0x0c0c12, 0.8),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    if let Some((graph, misses)) = game.replay_timeline() {
        // Kiai tinted, as on song select's graph.
        for &(s, e) in &graph.kiai {
            let (kx0, kx1) = (x_of(s), x_of(e));
            c.rect(kx0, top, (kx1 - kx0).max(1.0), gh, rgba(0xff8a3d, 0.12));
        }
        let (bx0, bx1) = (
            x_of(graph.peaks_start),
            x_of(graph.peaks_start + graph.peaks.len() as f64 * graph.section),
        );
        let n = (((bx1 - bx0) / 2.5) as usize).max(1);
        let bar_w = (bx1 - bx0) / n as f32;
        let now_x = x + bw * f;
        for (i, v) in graph.bins(n).into_iter().enumerate() {
            let bh = 1.0 + v * (gh - 3.0);
            let bx = bx0 + i as f32 * bar_w;
            let played = bx + bar_w / 2.0 <= now_x;
            let colour = rgba(preview::strain_colour(v), if played { 1.0 } else { 0.55 });
            c.rect(bx, base - 1.0 - bh, (bar_w - 0.5).max(0.5), bh, colour);
        }
        // Each miss: a glowing dot on the strip's top edge and a fine line down to the bars;
        // the ones still ahead are dimmed, like the bars.
        const MISS: u32 = 0xff4d6a;
        for &t in misses {
            let mx = x_of(t);
            let a = if mx <= now_x { 1.0 } else { 0.5 };
            c.rect(mx - 0.5, top + 2.0, 1.0, gh - 3.0, rgba(MISS, 0.35 * a));
            c.circle(mx, top - 1.0, 4.0, rgba(MISS, 0.18 * a), rgba(0, 0.0), 0.0);
            c.circle(
                mx,
                top - 1.0,
                2.0,
                rgba(MISS, a),
                rgba(0xffffff, 0.8 * a),
                0.75,
            );
        }
    }
    c.text(
        x,
        y + 10.0,
        6.0,
        rgba(0x6f6f86, 1.0),
        tr!(
            "Left/Right or drag: seek   Space: pause   Up/Down: speed",
            "左右かドラッグ: シーク   Space: ポーズ   上下: 速度",
            "Влево/вправо или тяните: перемотка   Пробел: пауза   Вверх/вниз: скорость"
        ),
    );
    // The playhead: a white line a little taller than the strip.
    let hx = x + bw * f;
    c.rect(hx - 0.75, top - 2.0, 1.5, gh + 4.0, rgba(0xffffff, 0.95));
    rect
}

/// Every spectated player at once: one lane per player, stacked, with their numbers.
/// Each player's strip shows where they play: the lane, or wherever the mods take it (rails,
/// Multiverse's playfield in focus...), fitted in and followed smoothly. `zones` keeps what each
/// strip shows between frames; `dt` is the time since the last one (s).
pub(crate) fn draw_spectate_all(
    c: &mut Canvas,
    games: &mut [(u32, Box<Play>)],
    players: &[(u32, String)],
    scroll: f64,
    zones: &mut Vec<[f32; 4]>,
    dt: f32,
) {
    let n = games.len().max(1) as f32;
    let (band_top, band_h) = game::lane_band();
    let (top, bottom) = (40.0, 470.0);
    let strip = ((bottom - top) / n).min(120.0);
    let k = strip / band_h;
    let full_w = c.width;
    // Drawn wider than the screen, so a plain lane fills the strip's width.
    let plain = [0.0, band_top, full_w / k, band_h];
    zones.resize(games.len(), [f32::NAN; 4]);
    let follow = 1.0 - (-dt / 0.15).exp();
    let head = tr!(
        "SPECTATING  -  everyone   Left/Right or A: one player   Esc leave",
        "観戦中  -  全員   左右かA: 一人   Esc 退出",
        "НАБЛЮДЕНИЕ  -  все   Влево/вправо или A: один игрок   Esc выйти"
    );
    c.text_centered(
        full_w / 2.0,
        16.0,
        9.0,
        rgba(0x4fb4ff, 1.0),
        &Canvas::fit(9.0, full_w - 16.0, head),
    );
    for (i, (id, g)) in games.iter_mut().enumerate() {
        let y = top + i as f32 * strip;
        let start = c.shapes.len();
        c.width = full_w / k;
        g.draw_playfield(c, scroll);
        c.width = full_w;
        let target = g
            .play_zone()
            .filter(|z| z[2] > 1.0 && z[3] > 1.0)
            .unwrap_or(plain);
        let z = &mut zones[i];
        if z[0].is_nan() {
            *z = target;
        } else {
            for (v, t) in z.iter_mut().zip(target) {
                *v += (t - *v) * follow;
            }
        }
        let [zx, zy, zw, zh] = *z;
        let scale = (full_w / zw).min(strip / zh);
        let (dx, dy) = (
            (full_w - zw * scale) / 2.0 - zx * scale,
            y + (strip - zh * scale) / 2.0 - zy * scale,
        );
        c.transform_since(start, [0.0, 0.0], scale, dx, dy);
        c.clip_since(start, 0.0, y, full_w, strip - 2.0);
        let name = players
            .iter()
            .find(|(p, _)| p == id)
            .map(|(_, n)| n.as_str())
            .unwrap_or("?");
        let st = g.stats();
        let line = format!(
            "{name}   {:08}   {:.2}%   {}x",
            st.score,
            st.accuracy() * 100.0,
            st.combo
        );
        let tw = Canvas::text_width(8.0, &line);
        c.round_rect(
            [full_w - tw - 28.0, y + 4.0],
            [tw + 16.0, 16.0],
            6.0,
            rgba(0x000000, 0.6),
            rgba(0, 0.0),
            0.0,
        );
        c.text_right(full_w - 20.0, y + 8.0, 8.0, rgba(0xffffff, 1.0), &line);
    }
}

/// Seconds the FAILED sign shows before a Sudden Death / Perfect fail starts the map over.
pub(crate) const FAIL_RESTART_LINGER: f64 = 0.6;

/// Whether a play that just ended starts over instead of showing results: one of the player's own
/// (not a replay) failed by Sudden Death / Perfect, with `Config::fail_restart` on.
pub(crate) fn fail_restarts(cfg: &Config, game: &Game) -> bool {
    cfg.fail_restart && game.failed_early() && game.replay().is_none()
}
