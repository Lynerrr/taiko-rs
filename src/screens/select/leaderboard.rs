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
struct BoardLine<'a> {
    place: usize,
    player: &'a str,
    score: u64,
    accuracy: f64,
    combo: u32,
    pp: Option<f64>,
    rate: f64,
    mods: String,
    miss: u32,
    date: u64,
    mine: bool,
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

/// What the strip of the map's board under song select's details shows.
pub(crate) enum MiniBoard {
    /// The online board on its way: grey bars in the rows' places.
    Loading,
    /// The server couldn't be reached.
    Unavailable,
    /// The top plays, best first (none yet: a quiet note).
    Rows(Vec<MiniRow>),
}

/// A play in the strip: who, how well, and whether it's the player's own.
pub(crate) struct MiniRow {
    pub player: String,
    pub accuracy: f64,
    pub mine: bool,
}

/// The player's own best on the map, at the strip's right.
pub(crate) struct MiniBest {
    pub grade: &'static str,
    pub accuracy: f64,
    pub date: u64,
}

/// The strip's players' names' size (a little under its other text, to fit more of them).
const NAME: f32 = 6.5;

/// How many plays the strip shows.
pub(crate) const MINI_ROWS: usize = 3;

/// When the strip last changed: the map it's for (hash, Relax, online) and since when, and since
/// when its content (not the grey bars) has shown.
#[derive(Default)]
pub(crate) struct MiniSeen {
    map: Option<((u64, bool, bool), std::time::Instant)>,
    content: Option<std::time::Instant>,
}

impl MiniSeen {
    /// How long the strip has been on `map` (none yet on a new one).
    pub fn on(&mut self, map: (u64, bool, bool), now: std::time::Instant) -> std::time::Duration {
        match self.map {
            Some((m, t)) if m == map => now.saturating_duration_since(t),
            _ => {
                self.map = Some((map, now));
                self.content = None;
                std::time::Duration::ZERO
            }
        }
    }

    /// How far its content has faded in, once it shows (`ready`); the grey bars show at once.
    pub fn fade(&mut self, ready: bool, now: std::time::Instant) -> f32 {
        if !ready {
            self.content = None;
            return 1.0;
        }
        let since = *self.content.get_or_insert(now);
        crate::render::progress(since, crate::style::motion::CONTENT_MS)
    }
}

/// The strip of the map's board in `rect`, on a raised surface: the top plays side by side (place,
/// player, accuracy), and the player's own best at the right (grade, accuracy, when). A click on
/// it opens the whole board. Its content `alpha` of the way in.
pub(crate) fn draw_mini_board(
    c: &mut Canvas,
    rect: [f32; 4],
    board: &MiniBoard,
    best: Option<&MiniBest>,
    alpha: f32,
) {
    use crate::style::ink;
    let [x, y, w, h] = rect;
    crate::style::surface(c, rect, 6.0, crate::style::Surface::Raised);
    c.hit(x, y, w, h, hit::Hit::BoardPage);
    let start = c.shapes.len();
    let (size, small) = (7.0, 6.0);
    let mid = y + h / 2.0;
    let (ty, sy) = (mid - size / 2.0, mid - small / 2.0);
    let mut right = x + w - 9.0;
    if let Some(b) = best {
        // Right to left: when, the accuracy, the grade, and what it is.
        let grade = rgba(crate::ui::grade_colour(b.grade), 1.0);
        let parts = [
            (scores::age(b.date), small, sy, rgba(ink::CAPTION, 1.0), 6.0),
            (
                format!("{:.2}%", b.accuracy * 100.0),
                size,
                ty,
                rgba(ink::PRIMARY, 1.0),
                4.0,
            ),
            (b.grade.to_string(), size, ty, grade, 5.0),
            (
                tr!("best", "自己ベスト", "лучший").to_string(),
                small,
                sy,
                rgba(ink::CAPTION, 1.0),
                10.0,
            ),
        ];
        for (text, size, ty, colour, gap) in parts {
            c.text_right(right, ty, size, colour, &text);
            right -= Canvas::text_width(size, &text) + gap;
        }
    }
    let left = x + 9.0;
    let slot = ((right - left) / MINI_ROWS as f32).max(0.0);
    let note = |c: &mut Canvas, text: &str| {
        let text = Canvas::fit(small, right - left, text);
        c.text(left, sy, small, rgba(ink::CAPTION, 1.0), &text);
    };
    match board {
        MiniBoard::Loading => {
            for k in 0..MINI_ROWS {
                let sx = left + k as f32 * slot;
                let fill = rgba(ink::SECONDARY, 0.12);
                c.round_rect([sx, mid - 3.0], [slot * 0.7, 6.0], 3.0, fill, fill, 0.0);
            }
        }
        MiniBoard::Unavailable => note(
            c,
            tr!(
                "board offline",
                "ランキングに接続できません",
                "рейтинг недоступен"
            ),
        ),
        MiniBoard::Rows(rows) if rows.is_empty() => {
            note(
                c,
                tr!(
                    "no scores yet",
                    "スコアはまだありません",
                    "рекордов пока нет"
                ),
            );
        }
        MiniBoard::Rows(rows) => {
            for (k, r) in rows.iter().take(MINI_ROWS).enumerate() {
                let sx = left + k as f32 * slot;
                let end = sx + slot - 5.0;
                let medal = [0xFFD34D, 0xC8D2E6, 0xE0995E][k];
                let place = format!("{}", k + 1);
                let mut px =
                    sx + c.text(sx, mid - NAME / 2.0, NAME, rgba(medal, 1.0), &place) + 4.0;
                // The name first; its accuracy after it where there's room.
                let ink = if r.mine { accent() } else { ink::PRIMARY };
                let name = Canvas::fit(NAME, (end - px).max(0.0), &r.player);
                px += c.text(px, mid - NAME / 2.0, NAME, rgba(ink, 1.0), &name) + 5.0;
                let acc = format!("{:.1}%", r.accuracy * 100.0);
                if px + Canvas::text_width(small, &acc) <= end + 0.5 {
                    c.text(px, sy, small, rgba(ink::SECONDARY, 1.0), &acc);
                }
            }
        }
    }
    c.fade_since(start, alpha, 1.0);
}
