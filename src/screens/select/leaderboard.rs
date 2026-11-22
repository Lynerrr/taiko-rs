//! The song select's leaderboard: local and online scores of the picked map, its tabs, rows
//! and scrollbar.

#[allow(unused_imports)]
use crate::*;

fn score_video_label() -> &'static str {
    tr!("video", "動画", "видео")
}

/// How wide a leaderboard row's video button is.
fn score_video_width() -> f32 {
    Canvas::text_width(6.0, score_video_label()) + 12.0
}

/// Where a leaderboard's rows start under its buttons.
pub(crate) const BOARD_HEAD: f32 = 28.0;

/// The leaderboard's Local / Online buttons from `x`, and the way back to the preview and the
/// map's website ending at `right`;
/// returns the free span between them, for a hint.
fn board_tabs(c: &mut Canvas, x: f32, y: f32, online: bool, relax: bool, right: f32) -> [f32; 2] {
    let mut tx = x;
    for (label, on) in [
        (tr!("Local", "ローカル", "Локально"), false),
        (tr!("Online", "オンライン", "Онлайн"), true),
    ] {
        let tw = Canvas::text_width(7.0, label) + 24.0;
        c.button(
            tx,
            y,
            tw,
            20.0,
            label,
            7.0,
            hit::Hit::BoardTab(on),
            on == online,
        );
        tx += tw + 6.0;
    }
    // With Relax on, these are its scores, which count apart.
    if relax {
        let label = "RELAX";
        let tw = Canvas::text_width(7.0, label) + 14.0;
        c.round_rect(
            [tx, y],
            [tw, 20.0],
            5.0,
            rgba(0xFFD34D, 0.15),
            rgba(0xFFD34D, 0.9),
            1.0,
        );
        c.text_centered(tx + tw / 2.0, y + 10.0, 7.0, rgba(0xFFD34D, 1.0), label);
        tx += tw + 6.0;
    }
    // Back to the preview at the far right, the website left of it.
    let back = tr!("< Preview", "< プレビュー", "< Превью");
    let bw = Canvas::text_width(7.0, back) + 24.0;
    c.button(
        right - bw,
        y,
        bw,
        20.0,
        back,
        7.0,
        hit::Hit::BoardPage,
        true,
    );
    let right = right - bw - 6.0;
    let label = tr!("Website", "ウェブ", "Сайт");
    let tw = Canvas::text_width(7.0, label) + 24.0;
    c.button(
        right - tw,
        y,
        tw,
        20.0,
        label,
        7.0,
        hit::Hit::BoardWebsite,
        false,
    );
    [tx + 4.0, right - tw - 10.0]
}

/// A leaderboard's message in place of its rows, centred under its buttons.
fn board_note(c: &mut Canvas, x: f32, y: f32, w: f32, note: &str) {
    for (k, line) in mobile::wrap(7.0, w - 16.0, note, 3).iter().enumerate() {
        let ly = y + BOARD_HEAD + 14.0 + k as f32 * 14.0;
        c.text_centered(x + w / 2.0, ly, 7.0, rgba(0x9a9ab0, 1.0), line);
    }
}

/// The server's scores for the selected map: each player's best (`board` None while loading).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_online_leaderboard(
    c: &mut Canvas,
    [x, y]: [f32; 2],
    w: f32,
    board: Option<&Result<Vec<online::OnlineScore>, String>>,
    me: &str,
    fits: usize,
    first: usize,
    relax: bool,
) {
    let [free, free_end] = board_tabs(c, x, y, true, relax, x + w);
    let note = match board {
        None => Some(tr!("loading...", "読み込み中...", "загрузка...").to_string()),
        Some(Err(e)) => Some(
            tr!("can't reach the server: {e}", "サーバーに届きません: {e}", "сервер недоступен: {e}"; [] [e]),
        ),
        Some(Ok(rows)) if rows.is_empty() => Some(
            tr!(
                "no online scores yet - log in (F9) and finish a play",
                "オンラインのスコアはまだありません - ログイン (F9) してプレイを終えましょう",
                "онлайн-рекордов пока нет - войдите (F9) и доиграйте карту"
            )
            .to_string(),
        ),
        Some(Ok(_)) => None,
    };
    if let Some(note) = note {
        board_note(c, x, y, w, &note);
        return;
    }
    let Some(Ok(rows)) = board else { return };
    let hint = tr!(
        "click: results and replay",
        "クリック: リザルトとリプレイ",
        "клик: итоги и повтор"
    );
    c.text_right(
        free_end,
        y + 7.0,
        6.0,
        rgba(0x6f6f86, 1.0),
        &Canvas::fit_hints(6.0, free_end - free, hint),
    );
    let shown = rows.len().saturating_sub(first).min(fits);
    board_scrollbar(
        c,
        x + w + 5.0,
        y + BOARD_HEAD,
        shown.max(1),
        first,
        rows.len(),
    );
    for (k, r) in rows.iter().skip(first).take(fits).enumerate() {
        let line = BoardLine {
            place: first + k + 1,
            player: &r.player,
            score: r.score,
            accuracy: r.accuracy,
            combo: r.max_combo,
            pp: r.pp,
            rate: r.rate,
            mods: r.mods.concat(),
            miss: r.miss,
            date: r.date,
            mine: r.player.eq_ignore_ascii_case(me),
        };
        let ry = y + BOARD_HEAD + k as f32 * BOARD_ROW;
        board_row(c, x, ry, w, &line, hit::Hit::OnlineScoreRow(k), None);
    }
}

/// A leaderboard row's height, the gap under it included.
pub(crate) const BOARD_ROW: f32 = 34.0;

/// What a leaderboard row shows, local or online.
pub(crate) struct BoardLine<'a> {
    pub(crate) place: usize,
    pub(crate) player: &'a str,
    pub(crate) score: u64,
    pub(crate) accuracy: f64,
    pub(crate) combo: u32,
    pub(crate) pp: Option<f64>,
    pub(crate) rate: f64,
    pub(crate) mods: String,
    pub(crate) miss: u32,
    pub(crate) date: u64,
    pub(crate) mine: bool,
}

impl<'a> BoardLine<'a> {
    /// A local play at `place`.
    pub(crate) fn local(place: usize, r: &'a scores::ScoreRow) -> Self {
        BoardLine {
            place,
            player: &r.player,
            score: r.score,
            accuracy: r.accuracy,
            combo: r.max_combo,
            pp: r.pp,
            rate: r.rate,
            mods: r.mods.clone(),
            miss: r.miss,
            date: r.date,
            mine: false,
        }
    }

    /// A play on the server at `place`, `me` the player logged in.
    pub(crate) fn online(place: usize, r: &'a online::OnlineScore, me: &str) -> Self {
        BoardLine {
            place,
            player: &r.player,
            score: r.score,
            accuracy: r.accuracy,
            combo: r.max_combo,
            pp: r.pp,
            rate: r.rate,
            mods: r.mods.concat(),
            miss: r.miss,
            date: r.date,
            mine: !me.is_empty() && r.player.eq_ignore_ascii_case(me),
        }
    }
}

/// What song select's embedded leaderboard has to show.
pub(crate) enum BoardShown<'a> {
    /// The rows from place `first` on (as many as fit), of `total`, and the player's own best
    /// pinned under them.
    Rows {
        rows: Vec<BoardLine<'a>>,
        first: usize,
        total: usize,
        best: Option<BoardLine<'a>>,
    },
    /// Nothing yet: why, said quietly.
    Note(String),
    /// The local board without a play.
    NoLocal,
}

/// The embedded leaderboard's parts (in the interface's own units): its tabs' row, a row, and
/// the pinned best's row; the room round them.
pub(crate) const SECTION_ROW: f32 = 34.0;
/// The right column's row of tabs over its body (`draw_column_tabs`): how tall it is.
pub(crate) const COLUMN_TAB_H: f32 = 24.0;
const SECTION_PAD: f32 = 8.0;
/// How far in from the section's sides its tabs and rows are: as far as the details and the
/// preview's lane above and under it.
const SECTION_INSET: f32 = 14.0;
/// Between two of the column's tabs.
const SECTION_TAB_GAP: f32 = 8.0;
/// The gap under a row (in `SECTION_ROW`).
const SECTION_ROW_GAP: f32 = 3.0;

/// Where the embedded leaderboard in `[x, y, w, h]` lists its rows (`best`: with the player's
/// best pinned under them), and how many fit.
pub(crate) fn section_rows([x, y, w, h]: [f32; 4], best: bool) -> ([f32; 4], usize) {
    let top = y + SECTION_PAD;
    let pinned = if best { SECTION_ROW + 6.0 } else { 0.0 };
    let room = (y + h - SECTION_PAD - pinned - top).max(0.0);
    let fits = ((room + SECTION_ROW_GAP) / SECTION_ROW).floor() as usize;
    let rows = [
        x + SECTION_INSET,
        top,
        w - 2.0 * SECTION_INSET - 6.0,
        fits as f32 * SECTION_ROW,
    ];
    (rows, fits)
}

/// What the right column's body shows (the main menu's look): the map's preview, or its local or
/// online leaderboard. One at a time, picked on the row of tabs over it (`draw_column_tabs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnTab {
    Preview,
    Local,
    Online,
}

/// The right column's row of tabs in `[x, y, w, h]`: Preview, Local and Online as words, the one
/// shown bright with a bar of the accent under it, the others muted (brighter under the pointer);
/// with Relax on and a board shown, a RELAX badge after them (its scores count apart); the way to
/// the map's website at the right.
pub(crate) fn draw_column_tabs(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    tab: ColumnTab,
    relax: bool,
) {
    use crate::style::{ink, radius};
    let (ty, th) = (y + (h - 18.0) / 2.0, 18.0);
    let mut tx = x + SECTION_INSET;
    for (label, which, hit) in [
        (
            tr!("Preview", "プレビュー", "Превью"),
            ColumnTab::Preview,
            hit::Hit::PreviewTab,
        ),
        (
            tr!("Local", "ローカル", "Локально"),
            ColumnTab::Local,
            hit::Hit::BoardTab(false),
        ),
        (
            tr!("Online", "オンライン", "Онлайн"),
            ColumnTab::Online,
            hit::Hit::BoardTab(true),
        ),
    ] {
        let tw = Canvas::text_width(7.0, label) + 20.0;
        let lit = which == tab;
        let hover = c.hovered(tx, ty, tw, th);
        if lit {
            c.rect(
                tx + radius::SMALL,
                ty + th - 3.0,
                tw - 2.0 * radius::SMALL,
                2.0,
                rgba(accent(), 1.0),
            );
        }
        let col = if lit {
            ink::PRIMARY
        } else if hover {
            ink::SECONDARY
        } else {
            ink::CAPTION
        };
        c.text_centered(
            tx + tw / 2.0,
            Canvas::text_y_mid(ty + th / 2.0, 7.0),
            7.0,
            rgba(col, 1.0),
            label,
        );
        c.hit(tx, ty, tw, th, hit);
        tx += tw + SECTION_TAB_GAP;
    }
    // With Relax on, the boards are its scores, which count apart.
    if relax && tab != ColumnTab::Preview {
        let label = "RELAX";
        let tw = Canvas::text_width(6.0, label) + 12.0;
        crate::style::badge(c, [tx + 4.0, ty + 2.0, tw, th - 4.0], 0xFFD34D);
        let ink = rgba(0xFFD34D, 1.0);
        c.text_centered(
            tx + 4.0 + tw / 2.0,
            Canvas::text_y_mid(ty + th / 2.0, 6.0),
            6.0,
            ink,
            label,
        );
    }
    // The map's website at the right.
    let label = tr!("Website", "ウェブ", "Сайт");
    let ww = Canvas::text_width(6.0, label) + 16.0;
    let wx = x + w - SECTION_INSET - ww;
    let hover = c.hovered(wx, ty, ww, th);
    let col = if hover { ink::PRIMARY } else { ink::CAPTION };
    c.text_centered(
        wx + ww / 2.0,
        Canvas::text_y_mid(ty + th / 2.0, 6.0),
        6.0,
        rgba(col, 1.0),
        label,
    );
    c.hit(wx, ty, ww, th, hit::Hit::BoardWebsite);
}

/// Song select's leaderboard as the right column's body in `rect` (the main menu's look, its
/// tabs over it: `draw_column_tabs`): the plays as dense two-line
/// rows (the place, the player's picture and name, the mods and the pp; under it accuracy,
/// combo, misses, the speed and when), a scroll bar, and the player's best pinned at the bottom.
/// `video`: the local board, whose rows can be exported (under the pointer).
pub(crate) fn draw_board_section(c: &mut Canvas, rect: [f32; 4], online: bool, shown: &BoardShown) {
    use crate::style::{Surface, ink, radius, surface};
    let [x, y, w, h] = rect;
    match shown {
        BoardShown::Rows {
            rows,
            first,
            total,
            best,
        } => {
            let ([rx, ry, rw, _], fits) = section_rows(rect, best.is_some());
            for (k, line) in rows.iter().enumerate() {
                let at = [
                    rx,
                    ry + k as f32 * SECTION_ROW,
                    rw,
                    SECTION_ROW - SECTION_ROW_GAP,
                ];
                let hit = if online {
                    hit::Hit::OnlineScoreRow(k)
                } else {
                    hit::Hit::ScoreRow(k)
                };
                let video = (!online).then_some(hit::Hit::ScoreVideo(k));
                dense_row(c, at, line, hit, video, false);
            }
            // The scroll bar right of the rows.
            if *total > fits && fits > 0 {
                let bh = fits as f32 * SECTION_ROW - SECTION_ROW_GAP;
                let thumb = (bh * fits as f32 / *total as f32).max(10.0);
                let at = (bh - thumb) * *first as f32 / (*total - fits) as f32;
                let sx = rx + rw + 3.0;
                c.round_rect(
                    [sx, ry],
                    [2.0, bh],
                    1.0,
                    rgba(0x2a2a38, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                let thumb_fill = rgba(accent(), 0.9);
                c.round_rect(
                    [sx, ry + at],
                    [2.0, thumb],
                    1.0,
                    thumb_fill,
                    thumb_fill,
                    0.0,
                );
            }
            // The player's best, pinned under a line at the bottom.
            if let Some(best) = best {
                let by = y + h - SECTION_PAD - (SECTION_ROW - SECTION_ROW_GAP);
                crate::style::rule(c, rx, by - 4.0, rw);
                let hit = if online {
                    hit::Hit::OnlineScoreRow(rows.len())
                } else {
                    hit::Hit::ScoreRow(rows.len())
                };
                let at = [rx, by, rw, SECTION_ROW - SECTION_ROW_GAP];
                c.round_rect(
                    [at[0], at[1]],
                    [at[2], at[3]],
                    radius::SMALL,
                    rgba(accent(), 0.08),
                    rgba(accent(), 0.0),
                    0.0,
                );
                dense_row(c, at, best, hit, None, true);
            }
        }
        BoardShown::Note(note) => section_note(c, rect, note),
        BoardShown::NoLocal => {
            // Calm: a line saying so, and the way to the online board.
            let mid = y + h / 2.0 - 16.0;
            let none = tr!(
                "No plays on this map yet",
                "このマップのプレイはまだありません",
                "На этой карте пока нет игр"
            );
            c.text_centered(x + w / 2.0, mid, 7.0, rgba(ink::CAPTION, 1.0), none);
            let label = tr!(
                "Show online scores",
                "オンラインのスコアを見る",
                "Показать онлайн"
            );
            let bw = Canvas::text_width(7.0, label) + 24.0;
            let (bx, by, bh) = (x + (w - bw) / 2.0, mid + 16.0, 20.0);
            let hover = c.hovered(bx, by, bw, bh);
            let level = if hover {
                Surface::Lifted
            } else {
                Surface::Raised
            };
            surface(c, [bx, by, bw, bh], radius::CONTROL, level);
            c.round_rect(
                [bx, by],
                [bw, bh],
                radius::CONTROL,
                rgba(0, 0.0),
                rgba(crate::style::tone::EDGE, 1.0),
                crate::style::tone::LINE,
            );
            let col = if hover { ink::PRIMARY } else { ink::SECONDARY };
            c.text_centered(
                bx + bw / 2.0,
                Canvas::text_y_mid(by + bh / 2.0, 7.0),
                7.0,
                rgba(col, 1.0),
                label,
            );
            c.hit(bx, by, bw, bh, hit::Hit::BoardTab(true));
        }
    }
}

/// A leaderboard section's message in place of its rows, in their middle.
fn section_note(c: &mut Canvas, [x, y, w, h]: [f32; 4], note: &str) {
    let lines = mobile::wrap(7.0, w - 32.0, note, 3);
    let step = Canvas::text_height(7.0) + 4.0;
    let top = y + (h - lines.len() as f32 * step) / 2.0;
    for (k, line) in lines.iter().enumerate() {
        let ink = rgba(crate::style::ink::CAPTION, 1.0);
        c.text_centered(x + w / 2.0, top + k as f32 * step, 7.0, ink, line);
    }
}

/// One play as a dense two-line row in `[x, y, w, h]`, as the profile's best performances are:
/// the place, the player's picture, their name and the mods, the pp at the right; under it how
/// it went and when. `video`: the export button, under the pointer. `pinned`: the player's best
/// pinned under the rows, marked PB in the place's stead (the place goes under it).
fn dense_row(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    l: &BoardLine,
    hit: hit::Hit,
    video: Option<hit::Hit>,
    pinned: bool,
) {
    use crate::style::{Surface, ink, radius, tile};
    let hover = c.hovered(x, y, w, h);
    // A tile of a tone a step lighter than the panel's, no line round it; lifted under the
    // pointer, and the pinned best on the lifted one.
    let level = if hover || pinned {
        Surface::Lifted
    } else {
        Surface::Raised
    };
    tile(c, [x, y, w, h], radius::SMALL, level);
    // The player's own play: a bar of the accent down its left edge.
    if l.mine {
        c.rect(x, y + 3.0, 2.0, h - 6.0, rgba(accent(), 1.0));
    }
    c.hit(x, y, w, h, hit);
    // The podium in gold, silver and bronze.
    let medal = match l.place {
        1 => Some(0xFFD34D),
        2 => Some(0xC8D2E6),
        3 => Some(0xE0995E),
        _ => None,
    };
    let mid = y + h / 2.0;
    let place = format!("#{}", l.place);
    if pinned {
        c.text_centered(
            x + 14.0,
            Canvas::text_y_mid(mid, 7.0),
            7.0,
            rgba(accent(), 1.0),
            "PB",
        );
    } else {
        let place_ink = rgba(medal.unwrap_or(ink::CAPTION), 1.0);
        c.text_centered(
            x + 14.0,
            Canvas::text_y_mid(mid, 7.0),
            7.0,
            place_ink,
            &place,
        );
    }
    c.avatar(l.player, x + 36.0, mid, 8.0, accent(), 1.0);
    let tx = x + 50.0;
    // The two lines' ink a quarter of the room in from the row's top and bottom (at most 4) and
    // the rest between them, so the age reads apart from the value over it (text on a phone's
    // grid comes out taller).
    let (name_h, under_h) = (Canvas::text_height(8.0), Canvas::text_height(6.0));
    let pad = ((h - name_h - under_h) / 4.0).clamp(0.0, 4.0);
    let top = Canvas::text_y_at(y + pad, 8.0);
    let under = Canvas::text_y_at(y + h - pad - under_h, 6.0);
    // Top line: the name and the mods, the pp (or the score without any) at the right.
    let mut right = x + w - 8.0;
    let (value, value_ink) = match l.pp {
        Some(pp) => (format!("{pp:.0}pp"), 0xff8ad8),
        None => (format!("{}", l.score), ink::SECONDARY),
    };
    c.text_right(right, top, 8.0, rgba(value_ink, 1.0), &value);
    let value_w = Canvas::text_width(8.0, &value);
    let mods = if l.mods.is_empty() {
        String::new()
    } else {
        format!("+{}", l.mods)
    };
    let mods_w = if mods.is_empty() {
        0.0
    } else {
        Canvas::text_width(6.0, &mods) + 8.0
    };
    let name_room = (right - value_w - 10.0 - mods_w - tx).max(0.0);
    let white = rgba(ink::PRIMARY, 1.0);
    let nw = social::styled_name(c, tx, top, 8.0, l.player, white, name_room);
    if !mods.is_empty() {
        let room = right - value_w - 10.0 - (tx + nw + 8.0);
        let mods = Canvas::fit(6.0, room.max(0.0), &mods);
        c.text(tx + nw + 8.0, top + 1.0, 6.0, rgba(0xFFC66B, 1.0), &mods);
    }
    // Under it: how it went, and when at the right (the export button there under the pointer).
    let when = scores::age(l.date);
    if let (Some(video), true) = (video, hover) {
        let label = score_video_label();
        let bw = score_video_width();
        let (bx, bh) = (right - bw, 12.0);
        c.button(bx, under - 3.0, bw, bh, label, 6.0, video, false);
        right = bx - 6.0;
    } else {
        c.text_right(right, under, 6.0, rgba(ink::CAPTION, 1.0), &when);
        right -= Canvas::text_width(6.0, &when) + 8.0;
    }
    let mut parts = Vec::new();
    if pinned {
        parts.push((place, medal.unwrap_or(ink::SECONDARY)));
    }
    parts.extend([
        (format!("{:.2}%", l.accuracy * 100.0), ink::PRIMARY),
        (format!("{}x", l.combo), ink::SECONDARY),
        (miss_label(l.miss), ink::SECONDARY),
    ]);
    if (l.rate - 1.0).abs() > 1e-6 {
        parts.push((format!("{:.2}x", l.rate), 0x4fb4ff));
    }
    let mut px = tx;
    for (k, (text, colour)) in parts.into_iter().enumerate() {
        if k > 0 {
            let dot = rgba(ink::CAPTION, 1.0);
            px += c.text(px, under, 6.0, dot, " · ");
        }
        let shown = Canvas::fit(6.0, right - px, &text);
        if shown.is_empty() {
            break;
        }
        px += c.text(px, under, 6.0, rgba(colour, 1.0), &shown);
        if shown != text {
            break;
        }
    }
}

/// One play on a leaderboard, a card at `ry`: its place, the player's picture, then their name
/// (as chat shows it) over how it went, the score and when on the right. `video`: the local
/// board's export button, at the far right.
#[allow(clippy::too_many_arguments)]
fn board_row(
    c: &mut Canvas,
    x: f32,
    ry: f32,
    w: f32,
    l: &BoardLine,
    hit: hit::Hit,
    video: Option<hit::Hit>,
) {
    let h = BOARD_ROW - 4.0;
    let hover = c.hovered(x, ry, w, h);
    let bg = if hover {
        0x262634
    } else if l.mine {
        0x23232f
    } else {
        0x15151d
    };
    // The podium in gold, silver and bronze.
    let medal = match l.place {
        1 => Some(0xFFD34D),
        2 => Some(0xC8D2E6),
        3 => Some(0xE0995E),
        _ => None,
    };
    let edge = if l.mine {
        accent()
    } else {
        medal.unwrap_or(0x2a2a38)
    };
    let edge_a = if l.mine || medal.is_none() { 1.0 } else { 0.45 };
    c.round_rect(
        [x, ry],
        [w, h],
        7.0,
        rgba(bg, 0.92),
        rgba(edge, edge_a),
        1.0,
    );
    c.hit(x, ry, w, h, hit);
    let mid = ry + h / 2.0;
    let place = format!("#{}", l.place);
    let place_colour = medal.unwrap_or(0x9a9ab0);
    c.text_centered(x + 16.0, mid, 8.0, rgba(place_colour, 1.0), &place);
    c.avatar(l.player, x + 42.0, mid, 10.0, accent(), 1.0);
    let tx = x + 58.0;
    let mut right = x + w - 8.0;
    if let Some(video) = video {
        let bw = score_video_width();
        right -= bw;
        c.button(
            right,
            mid - 7.0,
            bw,
            14.0,
            score_video_label(),
            6.0,
            video,
            false,
        );
        right -= 10.0;
    }
    let (top, under) = (ry + 5.0, ry + 18.0);
    // Top line: the player, the score at the right.
    let score = format!("{:08}", l.score);
    let score_w = Canvas::text_width(8.0, &score);
    c.text_right(right, top, 8.0, rgba(0xffffff, 1.0), &score);
    let white = rgba(0xffffff, 1.0);
    social::styled_name(
        c,
        tx,
        top,
        8.0,
        l.player,
        white,
        right - score_w - 10.0 - tx,
    );
    // Under it: how it went, and misses and age at the right.
    let when = misses_and_age(l.miss, l.date);
    let when_w = Canvas::text_width(6.5, &when);
    c.text_right(right, under, 6.5, rgba(0x6f6f86, 1.0), &when);
    let mut parts = vec![
        (format!("{:.2}%", l.accuracy * 100.0), 0xdedeea),
        (format!("{}x", l.combo), 0x9a9ab0),
    ];
    if let Some(pp) = l.pp {
        parts.push((format!("{pp:.0}pp"), 0xff8ad8));
    }
    if (l.rate - 1.0).abs() > 1e-6 {
        parts.push((format!("{:.2}x", l.rate), 0x4fb4ff));
    }
    if !l.mods.is_empty() {
        parts.push((format!("+{}", l.mods), 0xFFC66B));
    }
    let end = right - when_w - 10.0;
    let mut px = tx;
    for (text, colour) in parts {
        let shown = Canvas::fit(6.5, end - px, &text);
        if shown.is_empty() {
            break;
        }
        px += c.text(px, under, 6.5, rgba(colour, 1.0), &shown) + 8.0;
        if shown != text {
            break;
        }
    }
}

/// A leaderboard row's "3 miss".
fn miss_label(miss: u32) -> String {
    tr!("{miss} miss", "ミス{miss}", "{miss} мимо"; [] [miss])
}

/// A leaderboard row's "3 miss  2d".
pub(crate) fn misses_and_age(miss: u32, date: u64) -> String {
    let age = scores::age(date);
    tr!("{miss} miss  {age}", "ミス{miss}  {age}", "{miss} мимо  {age}"; [] [miss, age])
}

/// Local scores for the selected map, under the song select panel: `rows` from place `first`
/// on, of `total`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_leaderboard(
    c: &mut Canvas,
    x: f32,
    y: f32,
    w: f32,
    rows: &[&scores::ScoreRow],
    first: usize,
    total: usize,
    relax: bool,
) {
    let [free, free_end] = board_tabs(c, x, y, false, relax, x + w);
    if rows.is_empty() {
        let none = tr!(
            "No plays yet: finish one to set a score.",
            "まだありません - プレイを終えると記録されます",
            "пока нет - доиграйте карту"
        );
        board_note(c, x, y, w, none);
        return;
    }
    let room = free_end - free;
    let hint = tr!(
        "click: results and replay   video: export it",
        "クリック: リザルトとリプレイ   動画: 書き出し",
        "клик: итоги и повтор   видео: экспорт"
    );
    c.text_right(
        free_end,
        y + 7.0,
        6.0,
        rgba(0x6f6f86, 1.0),
        &Canvas::fit_hints(6.0, room, hint),
    );
    for (k, r) in rows.iter().enumerate() {
        let line = BoardLine {
            place: first + k + 1,
            player: &r.player,
            score: r.score,
            accuracy: r.accuracy,
            combo: r.max_combo,
            pp: r.pp,
            rate: r.rate,
            mods: r.mods.clone(),
            miss: r.miss,
            date: r.date,
            mine: false,
        };
        let ry = y + BOARD_HEAD + k as f32 * BOARD_ROW;
        let video = Some(hit::Hit::ScoreVideo(k));
        board_row(c, x, ry, w, &line, hit::Hit::ScoreRow(k), video);
    }
    board_scrollbar(
        c,
        x + w + 5.0,
        y + BOARD_HEAD,
        rows.len().max(1),
        first,
        total.max(rows.len()),
    );
}

/// A song select divider in `rect` (`upright`: between left and right, else between top and
/// bottom): a short grip, lit along its length while hovered or dragged. Only the grip takes the
/// mouse, not the whole edge: passing along it shouldn't light it up.
pub(crate) fn draw_divider(
    c: &mut Canvas,
    [x, y, w, h]: [f32; 4],
    upright: bool,
    dragging: bool,
    hit: hit::Hit,
) {
    const GRIP: f32 = 48.0;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let [gx, gy, gw, gh] = if upright {
        [x, cy - GRIP / 2.0, w, GRIP]
    } else {
        [cx - GRIP / 2.0, y, GRIP, h]
    };
    let lit = dragging || c.hovered(gx, gy, gw, gh);
    if upright {
        let len = if lit { h } else { 28.0 };
        c.round_rect(
            [cx - 1.0, cy - len / 2.0],
            [2.0, len],
            1.0,
            rgba(if lit { accent() } else { 0x3a3a4e }, 1.0),
            rgba(0, 0.0),
            0.0,
        );
    } else {
        let len = if lit { w } else { 28.0 };
        c.round_rect(
            [cx - len / 2.0, cy - 1.0],
            [len, 2.0],
            1.0,
            rgba(if lit { accent() } else { 0x3a3a4e }, 1.0),
            rgba(0, 0.0),
            0.0,
        );
    }
    c.hit(gx, gy, gw, gh, hit);
}

/// A leaderboard's scroll bar right of its rows, when there are more than the `shown`.
fn board_scrollbar(c: &mut Canvas, x: f32, y: f32, shown: usize, first: usize, total: usize) {
    if total <= shown || shown == 0 {
        return;
    }
    let h = shown as f32 * BOARD_ROW - 4.0;
    let thumb = (h * shown as f32 / total as f32).max(10.0);
    let at = (h - thumb) * first as f32 / (total - shown) as f32;
    c.round_rect(
        [x, y],
        [3.0, h],
        1.5,
        rgba(0x2a2a38, 1.0),
        rgba(0, 0.0),
        0.0,
    );
    c.round_rect(
        [x, y + at],
        [3.0, thumb],
        1.5,
        rgba(accent(), 0.9),
        rgba(0, 0.0),
        0.0,
    );
}

/// Fetches a map's online leaderboard on a background thread.
pub(crate) fn fetch_board(
    server: String,
    map: u64,
    relax: bool,
) -> Receiver<Result<Vec<online::OnlineScore>, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rows =
            online::call(&server, online::Request::MapScores { map, relax }).and_then(
                |r| match r {
                    online::Reply::Scores(rows) => Ok(rows),
                    _ => Err(crate::lang::odd_reply()),
                },
            );
        let _ = tx.send(rows);
    });
    rx
}
