//! A Journey marathon between two maps: a wheel of maps turns from the one just played to the
//! next, which loads meanwhile and starts by itself; and once the marathon is over, what it came
//! to.

use super::screen::{Frame, Key, Screen};
use crate::config::MarathonGap;
use crate::journey::{
    MapSource, Marathon, MarathonEnd, MarathonOutcome, MarathonPlay, PastOutcome, Preload,
};
use crate::*;

/// The shortest time between two maps: the wheel turns and the next map's name can be read.
pub(crate) const MARATHON_GAP: Duration = Duration::from_millis(2200);
/// The time between two maps with a short wheel (`MarathonGap::Short`).
const SHORT_GAP: Duration = Duration::from_millis(800);
/// How long before the last note of a map the next one starts loading.
const PRELOAD_LEAD_MS: f64 = 30_000.0;
/// How long the last map's song takes to fade out under the next one's, and the next one's to
/// fade in.
const CROSSFADE_OUT_MS: f64 = 1500.0;
const CROSSFADE_IN_MS: f64 = 400.0;

/// The time between two maps: none when seamless, the next map starting once it has loaded.
fn gap(cfg: &Config) -> Duration {
    match cfg.journey_gap {
        MarathonGap::Wheel => MARATHON_GAP,
        MarathonGap::Short => SHORT_GAP,
        MarathonGap::Seamless => Duration::ZERO,
    }
}

/// How long the wheel takes to turn by a map.
const WHEEL_MS: f32 = 650.0;
/// How long the star rating takes to roll from the last map's to the next one's.
const STARS_MS: f32 = 1000.0;

/// Between two maps of a marathon, or after it.
pub(crate) struct MarathonScreen {
    /// When the wheel started turning (the first frame).
    entered: Option<Instant>,
    /// The next map loading, and its library index.
    load: Option<(Job<Loaded>, usize)>,
    /// Space: the marathon waits here until Space again.
    pub(crate) held: bool,
    /// Enter: the next map starts as soon as it has loaded.
    go_now: bool,
}

impl MarathonScreen {
    pub(crate) fn scene() -> Scene {
        Scene::Other(Box::new(MarathonScreen {
            entered: None,
            load: None,
            held: false,
            go_now: false,
        }))
    }

    /// Gives up the map loading (it isn't to be played now).
    fn drop_load(&mut self, state: &mut State) {
        if self.load.take().is_some() {
            state.journey.in_play = false;
            state.journey.playing_cached = None;
        }
    }
}

impl Screen for MarathonScreen {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "Marathon"
    }

    fn activity(&self) -> &'static str {
        "in a marathon"
    }

    fn presence(&self, state: &State) -> Option<(String, String)> {
        let m = state.journey.marathon.as_ref()?;
        let n = m.played();
        Some((
            tr!("Journey marathon", "ジャーニー マラソン", "Марафон Journey").into(),
            tr!("{n} maps played", "{n}譜面プレイ", "Сыграно карт: {n}"; [] [n]),
        ))
    }

    fn map_entry(&self) -> Option<usize> {
        self.load.as_ref().map(|(_, entry)| *entry)
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn shows_own_background(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn loading(&self) -> bool {
        self.load.as_ref().is_some_and(|(rx, _)| rx.is_busy())
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if repeat || state.handle_global_key(code, window) {
            return;
        }
        let over = state
            .journey
            .marathon
            .as_ref()
            .is_none_or(|m| m.ended.is_some());
        if over {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape | KeyCode::Space => {
                    state.marathon_close();
                }
                KeyCode::KeyR => state.marathon_start(),
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Escape => {
                self.drop_load(state);
                state.marathon_end();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.go_now = true;
                self.held = false;
            }
            KeyCode::Space => self.held = !self.held,
            // Another map in place of the one coming up.
            KeyCode::Tab => {
                self.drop_load(state);
                if state.journey_skip_quiet(0.0) {
                    self.entered = None;
                }
            }
            KeyCode::KeyN => state.journey_skip_upcoming(),
            KeyCode::KeyL => state.marathon_vote(online::MapVote::Like),
            KeyCode::KeyD => state.marathon_vote(online::MapVote::Dislike),
            KeyCode::KeyS => state.marathon_save_last(),
            _ => {}
        }
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, frame: &Frame) -> Option<Scene> {
        let now = frame.now;
        state.journey_tick_quiet();
        let Some(m) = &state.journey.marathon else {
            return Some(super::journey::JourneyScreen::scene());
        };
        if m.ended.is_some() {
            draw_summary(state, m, canvas, now);
            return None;
        }
        // The next map loads as soon as it's here.
        if self.load.is_none() && !self.held {
            if state.journey.current().is_some() {
                if let Some(entry) = state.journey_current_entry(true) {
                    state.journey.in_play = true;
                    let rx = match state.marathon_take_preload(entry) {
                        Some(p) => {
                            state.play_begins(entry, None, true);
                            state.pending_mods = p.mods;
                            p.job
                        }
                        None => state.begin_load(entry, None, true).0,
                    };
                    self.load = Some((rx, entry));
                }
            } else if state.journey.pool_exhausted && state.journey.fetching.is_none() {
                if let Some(m) = &mut state.journey.marathon {
                    m.end(MarathonEnd::OutOfMaps, now);
                }
                return None;
            }
        }
        let entered = *self.entered.get_or_insert(now);
        let since = now.saturating_duration_since(entered);
        let due = self.go_now || since >= gap(&state.cfg);
        if due
            && !self.held
            && let Some((rx, entry)) = &mut self.load
        {
            let entry = *entry;
            match rx.poll_ended() {
                Some(Some(Ok((map, track)))) => {
                    return Some(state.marathon_start_map(entry, map, track));
                }
                Some(Some(Err(e))) => {
                    eprintln!(
                        "marathon: couldn't load {}: {e}",
                        state.library[entry].label()
                    );
                    self.drop_load(state);
                    state.journey_drop_unplayable();
                    self.entered = None;
                }
                Some(None) => self.drop_load(state),
                None => {}
            }
        }
        let loading = self.load.as_ref().is_some_and(|(rx, _)| rx.is_busy());
        draw_wheel(state, canvas, now, since, self.held, loading);
        None
    }
}

/// A marathon's combo in a play's bottom-right corner. Always stays anchored during live play
/// (showing muted `x0` after a miss) with a stable minimum width and smooth vertical lift when
/// the "Up next" card slides in underneath (`up_next_alpha` 0..=1).
pub(crate) fn draw_combo_chip(c: &mut Canvas, combo: u32, up_next_alpha: f32) {
    let text = format!("x{combo}");
    let label = tr!("MARATHON", "マラソン", "МАРАФОН");
    let val_w = Canvas::text_width(11.0, &text).max(Canvas::text_width(11.0, "x9999"));
    let w = Canvas::text_width(7.0, label) + 10.0 + val_w + 20.0;
    let h = 22.0;
    let x = c.width - 12.0 - w;
    let lift = up_next_alpha.clamp(0.0, 1.0) * 60.0;
    let y = c.height - 12.0 - h - lift;
    let active = combo > 0;
    style::cold_panel(c, [x, y, w, h]);
    c.round_rect(
        [x, y],
        [w, h],
        style::radius::CARD,
        rgba(0, 0.0),
        rgba(
            if active { 0xffd24a } else { style::tone::EDGE },
            if active { 0.35 } else { 0.45 },
        ),
        1.0,
    );
    c.text(
        x + 10.0,
        y + 8.0,
        7.0,
        rgba(if active { 0x22d4c0 } else { 0x5c7c82 }, 1.0),
        label,
    );
    c.text(
        x + w - 10.0 - Canvas::text_width(11.0, &text),
        y + 5.0,
        11.0,
        rgba(if active { 0xffd24a } else { 0x6c6c82 }, 1.0),
        &text,
    );
}

/// The wheel's buttons: hold, and the way out.
const PAUSE_ICON: style::Icon = [
    "................",
    "................",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "...####..####...",
    "................",
    "................",
];
const CROSS_ICON: style::Icon = [
    "................",
    "................",
    "..##........##..",
    "..###......###..",
    "...###....###...",
    "....###..###....",
    ".....######.....",
    "......####......",
    "......####......",
    ".....######.....",
    "....###..###....",
    "...###....###...",
    "..###......###..",
    "..##........##..",
    "................",
    "................",
];

/// The wheel's row of buttons, each doing what its key does (the key named on it).
fn draw_buttons(c: &mut Canvas, held: bool) {
    let size = 8.0;
    let hold = if held {
        (
            &style::PLAY,
            tr!("Go on", "再開", "Продолжить"),
            "Space",
            KeyCode::Space,
        )
    } else {
        (
            &PAUSE_ICON,
            tr!("Hold", "一時停止", "Пауза"),
            "Space",
            KeyCode::Space,
        )
    };
    let buttons: [(&'static style::Icon, &str, &str, KeyCode); 4] = [
        (
            &style::PLAY,
            tr!("Start now", "すぐ開始", "Начать сразу"),
            "Enter",
            KeyCode::Enter,
        ),
        hold,
        (
            &super::journey::SKIP_ICON,
            tr!("Another map", "別の譜面", "Другая карта"),
            "Tab",
            KeyCode::Tab,
        ),
        (
            &CROSS_ICON,
            tr!("End", "終了", "Закончить"),
            "Esc",
            KeyCode::Escape,
        ),
    ];
    let gap = 10.0;
    let widths: Vec<f32> = buttons
        .iter()
        .map(|(_, label, key, _)| style::icon_button_width(label, Some(key), size))
        .collect();
    let total = widths.iter().sum::<f32>() + gap * (buttons.len() - 1) as f32;
    let mut x = (c.width - total) / 2.0 + 6.0;
    let mid = c.height - 72.0;
    for ((icon, label, key, code), w) in buttons.iter().zip(&widths) {
        style::block_button(
            c,
            [x, mid],
            icon,
            label,
            Some(key),
            size,
            held && *code == KeyCode::Space,
            hit::Hit::Key(*code),
            30.0,
        );
        x += w + gap;
    }
}

fn ease(ms: f32, over: f32) -> f32 {
    if !render::animations() {
        return 1.0;
    }
    style::motion::out_quint((ms / over).clamp(0.0, 1.0))
}

/// "12:34", or "1:02:34" past an hour.
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn format_len(ms: u64) -> String {
    if ms == 0 {
        return "-".into();
    }
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

struct SideCard<'a> {
    eyebrow: &'a str,
    stripe_col: u32,
    item: &'a online::JourneyItem,
    line: Option<(String, u32)>,
    footer: Option<&'a str>,
    alpha: f32,
}

/// Side card on the marathon carousel (last played map on the left, map after next on the right).
fn draw_side_card(c: &mut Canvas, [x, y, w, h]: [f32; 4], card: &SideCard<'_>) {
    if card.alpha <= 0.01 {
        return;
    }
    let start = c.shapes.len();
    style::cold_panel(c, [x, y, w, h]);
    c.round_rect(
        [x, y],
        [w, h],
        style::radius::CARD,
        rgba(0, 0.0),
        rgba(card.stripe_col, 0.30),
        1.0,
    );
    c.round_rect(
        [x + 6.0, y + 1.5],
        [w - 12.0, 2.5],
        1.2,
        rgba(card.stripe_col, 0.85),
        rgba(0, 0.0),
        0.0,
    );
    c.text(x + 10.0, y + 10.0, 6.5, rgba(0x8888a4, 1.0), card.eyebrow);
    let stars = format!("{:.2}*", card.item.stars);
    let sw = Canvas::text_width(7.5, &stars);
    c.text(
        x + w - 10.0 - sw,
        y + 9.5,
        7.5,
        rgba(0xffd24a, 0.95),
        &stars,
    );

    c.text(
        x + 10.0,
        y + 26.0,
        9.0,
        rgba(0xe0e0ec, 1.0),
        &Canvas::fit(9.0, w - 20.0, &card.item.title),
    );
    c.text(
        x + 10.0,
        y + 40.0,
        7.5,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(7.5, w - 20.0, &card.item.artist),
    );
    let diff = format!("[{}]", card.item.version);
    c.text(
        x + 10.0,
        y + 53.0,
        7.0,
        rgba(0x787892, 1.0),
        &Canvas::fit(7.0, w - 20.0, &diff),
    );
    if let Some((text, col)) = &card.line {
        c.text(
            x + 10.0,
            y + h - 32.0,
            7.5,
            rgba(*col, 1.0),
            &Canvas::fit(7.5, w - 20.0, text),
        );
    }
    if let Some(foot) = card.footer {
        c.text(
            x + 10.0,
            y + h - 16.0,
            6.5,
            rgba(0x787890, 1.0),
            &Canvas::fit(6.5, w - 20.0, foot),
        );
    }
    c.fade_since(start, card.alpha, 1.0);
}

/// What became of a map: its card's line.
fn outcome_line(play: &MarathonPlay) -> (String, u32) {
    let acc = format!("{:.2}%", play.accuracy * 100.0);
    let mut line = match play.outcome {
        MarathonOutcome::Passed => {
            let pp = play.pp.map(|pp| format!("  {pp:.0}pp")).unwrap_or_default();
            let combo = play.max_combo;
            format!("PASS  {acc}  x{combo}{pp}")
        }
        MarathonOutcome::Failed => format!("FAIL  {acc}"),
        MarathonOutcome::Skipped => tr!("SKIPPED", "スキップ", "ПРОПУЩЕНА").to_string(),
    };
    match play.vote {
        Some(online::MapVote::Like) => line.push_str(tr!("  · liked", "  · いいね", "  · лайк")),
        Some(online::MapVote::Dislike) => {
            line.push_str(tr!("  · disliked", "  · よくない", "  · дизлайк"))
        }
        None => {}
    }
    let col = match play.outcome {
        MarathonOutcome::Passed => 0x4ee07a,
        MarathonOutcome::Failed => 0xff4d5a,
        MarathonOutcome::Skipped => 0x9a9ab0,
    };
    (line, col)
}

/// Where the map coming up is: its card's line.
fn source_line(source: &MapSource, loading: bool) -> (String, u32) {
    match source {
        MapSource::Pending => (
            tr!("Queued...", "待機中...", "В очереди...").into(),
            0x9a9ab0,
        ),
        MapSource::Downloading(job) => {
            let pct = (job.fraction().unwrap_or(0.0) * 100.0).round() as u32;
            (
                tr!("Downloading {pct}%...", "ダウンロード中 {pct}%...", "Загрузка {pct}%..."; [] [pct]),
                0x22d4c0,
            )
        }
        MapSource::Failed(_) => (
            tr!("Unavailable", "利用不可", "Недоступна").into(),
            0xff4d5a,
        ),
        _ if loading => (
            tr!("Loading...", "読み込み中...", "Загружается...").into(),
            0x22d4c0,
        ),
        _ => (tr!("Ready", "準備完了", "Готова").into(), 0x4ee07a),
    }
}

/// Background image path for a queued Journey map if its files are already on disk.
fn queued_background(state: &State, q: &crate::journey::QueuedMap) -> Option<std::path::PathBuf> {
    match &q.source {
        MapSource::Cached { entry, .. } => crate::preview::background_path(entry),
        MapSource::Local(path) | MapSource::Saved(path) => state
            .library
            .iter()
            .find(|e| e.path == *path)
            .and_then(crate::preview::background_path),
        _ => None,
    }
}

/// The marathon intermission screen: top status bar, focused hero card for the upcoming map
/// flanked by compact previous/upcoming cards, countdown bar, and action buttons.
fn draw_wheel(
    state: &State,
    c: &mut Canvas,
    now: Instant,
    since: Duration,
    held: bool,
    loading: bool,
) {
    let Some(m) = &state.journey.marathon else {
        return;
    };
    let w = c.width;
    let h = c.height;
    let last = m.last.as_ref();
    let current = state.journey.current();
    let wait_gap = gap(&state.cfg);

    // Seamless first-map launch: avoid flashing the full wheel for a few frames while the first
    // map loads synchronously from disk.
    if wait_gap.is_zero()
        && last.is_none()
        && !held
        && let Some(q) = current
    {
        let pw = 360.0_f32.min(w - 32.0);
        let ph = 92.0;
        let px = (w - pw) / 2.0;
        let py = (h - ph) / 2.0;
        style::cold_panel(c, [px, py, pw, ph]);
        c.text_centered(
            w / 2.0,
            py + 16.0,
            8.0,
            rgba(0x22d4c0, 1.0),
            tr!("STARTING MARATHON", "マラソン開始中", "ЗАПУСК МАРАФОНА"),
        );
        c.text_centered(
            w / 2.0,
            py + 36.0,
            12.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(12.0, pw - 28.0, &q.item.title),
        );
        let (status, scol) = source_line(&q.source, loading);
        c.text_centered(w / 2.0, py + 62.0, 8.0, rgba(scol, 1.0), &status);
        return;
    }

    // 1. Top status strip (styled like Journey's top lane).
    let bar_x = 24.0;
    let bar_y = 22.0;
    let bar_w = w - 48.0;
    let bar_h = 34.0;
    style::cold_panel(c, [bar_x, bar_y, bar_w, bar_h]);
    let accent = if state.cfg.journey_hardcore {
        style::DANGER
    } else {
        0xffd24a
    };
    c.rect(
        bar_x + 1.0,
        bar_y + 5.0,
        2.5,
        bar_h - 10.0,
        rgba(accent, 0.95),
    );

    let n = m.played() + 1;
    let time = clock(m.duration(now));
    let mut so_far = tr!(
        "MARATHON  ·  map {n}  ·  {time}",
        "マラソン  ·  {n}譜面目  ·  {time}",
        "МАРАФОН  ·  карта {n}  ·  {time}";
        [] [n, time]
    );
    if let Some(acc) = m.average_accuracy() {
        so_far.push_str(&format!("  ·  {:.2}%", acc * 100.0));
    }
    if m.pp > 0.0 {
        so_far.push_str(&format!("  ·  {:.0}pp", m.pp));
    }
    let mut right_used = 12.0f32;
    if state.cfg.journey_hardcore {
        let hc = tr!("HARDCORE", "ハードコア", "ХАРДКОР");
        let hw = Canvas::text_width(7.0, hc) + 12.0;
        let hx = bar_x + bar_w - right_used - hw;
        c.round_rect(
            [hx, bar_y + 9.0],
            [hw, 16.0],
            style::radius::SMALL,
            rgba(style::DANGER, 0.18),
            rgba(style::DANGER, 0.85),
            1.0,
        );
        c.text_centered(
            hx + hw / 2.0,
            bar_y + 13.0,
            7.0,
            rgba(style::DANGER, 1.0),
            hc,
        );
        right_used += hw + 8.0;
    }
    if m.combo > 0 {
        let combo = m.combo;
        let combo_txt = tr!(
            "MARATHON COMBO x{combo}",
            "マラソンコンボ x{combo}",
            "КОМБО МАРАФОНА x{combo}";
            [] [combo]
        );
        let cw_pill = Canvas::text_width(8.0, &combo_txt) + 16.0;
        let cx_pill = bar_x + bar_w - right_used - cw_pill;
        c.round_rect(
            [cx_pill, bar_y + 7.0],
            [cw_pill, 20.0],
            style::radius::CONTROL,
            rgba(0xffd24a, 0.16),
            rgba(0xffd24a, 0.72),
            1.0,
        );
        c.text_centered(
            cx_pill + cw_pill / 2.0,
            bar_y + 13.0,
            8.0,
            rgba(0xffd24a, 1.0),
            &combo_txt,
        );
        right_used += cw_pill + 10.0;
    }
    c.text(
        bar_x + 14.0,
        bar_y + 13.0,
        8.5,
        rgba(0xc8c8dc, 1.0),
        &Canvas::fit(8.5, (bar_w - 24.0 - right_used).max(60.0), &so_far),
    );

    // 2. Center carousel: Hero card for the upcoming map + compact dimmed side cards.
    let ms = since.as_secs_f32() * 1000.0;
    let turn = ease(ms, WHEEL_MS);
    let hero_w = (w * 0.48).clamp(320.0, 440.0);
    let hero_h = 186.0;
    let side_w = (w * 0.22).clamp(160.0, 206.0);
    let side_h = 136.0;
    let gap_x = 16.0;
    let hero_y = 122.0;
    let side_y = hero_y + (hero_h - side_h) / 2.0;
    let slide = (1.0 - turn) * (side_w + gap_x);
    let hero_x = (w - hero_w) / 2.0 + slide;

    // Left side card: previous map & outcome.
    if let Some(play) = last {
        let (line, col) = outcome_line(play);
        let mut foot = String::new();
        if play.vote_on.is_some() {
            foot.push_str(tr!("L/D: vote", "L/D: 評価", "L/D: оценка"));
        }
        if play.cached.is_some() {
            if !foot.is_empty() {
                foot.push_str("  ·  ");
            }
            foot.push_str(tr!("S: save", "S: 保存", "S: сохр."));
        } else if play.saved {
            if !foot.is_empty() {
                foot.push_str("  ·  ");
            }
            foot.push_str(tr!("Saved", "保存済み", "Сохранено"));
        }
        draw_side_card(
            c,
            [hero_x - gap_x - side_w, side_y, side_w, side_h],
            &SideCard {
                eyebrow: tr!("LAST MAP", "前の譜面", "ПРОШЛАЯ"),
                stripe_col: col,
                item: &play.item,
                line: Some((line, col)),
                footer: (!foot.is_empty()).then_some(foot.as_str()),
                alpha: 0.60 + 0.35 * (1.0 - turn),
            },
        );
    }

    // Center Hero card: the next map (or outage / searching state).
    let (eyebrow, eyebrow_col) = if held {
        (
            tr!(
                "Held: Space to go on",
                "一時停止中: Spaceで再開",
                "Пауза: Space, чтобы продолжить"
            ),
            0xffd24a,
        )
    } else if current.is_none() {
        (
            tr!(
                "Finding your next map...",
                "次の譜面を探しています...",
                "Подбираем следующую карту..."
            ),
            0x9a9ab0,
        )
    } else {
        (tr!("Up next", "次の譜面", "Дальше"), 0x22d4c0)
    };

    if let Some(q) = current {
        let (role_label, role_col) = super::journey::role_badge(&q.item.role);
        let from = last.map_or(q.item.stars, |p| p.item.stars);
        let roll = ease(ms, STARS_MS) as f64;
        let shown_stars = from + (q.item.stars - from) * roll;

        style::cold_panel(c, [hero_x, hero_y, hero_w, hero_h]);
        c.round_rect(
            [hero_x, hero_y],
            [hero_w, hero_h],
            style::radius::CARD,
            rgba(0, 0.0),
            rgba(if held { 0xffd24a } else { role_col }, 0.55),
            1.2,
        );

        // Artwork banner covering the upper 126px (title/artist/diff sit over its bottom fade,
        // matching Journey's own card with no hollow gap when artwork is absent).
        let banner_h = 126.0;
        let has_bg = queued_background(state, q).is_some_and(|bg| {
            let drawn = c.picture_cover_round_top(
                &bg,
                [hero_x, hero_y, hero_w, banner_h],
                [hero_x, hero_y, hero_w, banner_h],
                [0.85, 0.85, 0.85, 1.0],
                style::radius::CARD,
                Some(0.5),
            );
            if drawn {
                c.shade_eased_fade_down(
                    [hero_x, hero_y],
                    [hero_w, banner_h],
                    style::radius::CARD,
                    rgba(style::PANEL_COLD, 0.78),
                    rgba(style::PANEL_COLD, 0.14),
                    0.55,
                );
            }
            drawn
        });
        if !has_bg {
            c.rect(
                hero_x + 1.0,
                hero_y + 4.0,
                hero_w - 2.0,
                34.0,
                rgba(role_col, 0.06),
            );
        }
        c.round_rect(
            [hero_x + 6.0, hero_y + 1.5],
            [hero_w - 12.0, 2.5],
            1.2,
            rgba(if held { 0xffd24a } else { role_col }, 0.95),
            rgba(0, 0.0),
            0.0,
        );

        // Top row of hero banner: role pill + eyebrow on left, rolling star_chip on right.
        let pad = 14.0;
        let stars_opt = (shown_stars > 0.05).then_some(shown_stars as f32);
        let star_w = style::star_chip(
            c,
            hero_x + hero_w - pad,
            hero_y + 14.0,
            9.5,
            None,
            stars_opt,
        );

        let pill_w = Canvas::text_width(6.5, role_label) + 10.0;
        c.round_rect(
            [hero_x + pad, hero_y + 12.0],
            [pill_w, 14.0],
            style::radius::SMALL,
            rgba(role_col, 0.22),
            rgba(role_col, 0.8),
            1.0,
        );
        c.text_centered(
            hero_x + pad + pill_w / 2.0,
            hero_y + 15.5,
            6.5,
            rgba(role_col, 1.0),
            role_label,
        );
        c.text(
            hero_x + pad + pill_w + 8.0,
            hero_y + 15.0,
            8.0,
            rgba(eyebrow_col, 1.0),
            &Canvas::fit(
                8.0,
                (hero_w - pad * 2.0 - pill_w - star_w - 18.0).max(40.0),
                eyebrow,
            ),
        );

        // Title + artist + diff anchored at bottom of banner (consistent with Journey card).
        let title_y = hero_y + banner_h - 62.0;
        let inner_w = hero_w - pad * 2.0;
        let title = Canvas::fit(15.0, inner_w, &q.item.title);
        style::title(c, hero_x + pad, title_y, 15.0, 0xffffff, &title);
        c.text(
            hero_x + pad,
            title_y + 23.0,
            9.5,
            rgba(0xc8c8dc, 1.0),
            &Canvas::fit(9.5, inner_w, &q.item.artist),
        );
        let diff_line = format!("[{}]  by {}", q.item.version, q.item.creator);
        c.text(
            hero_x + pad,
            title_y + 39.0,
            8.5,
            rgba(role_col, 0.95),
            &Canvas::fit(8.5, inner_w, &diff_line),
        );

        style::rule(c, hero_x + 1.0, hero_y + banner_h, hero_w - 2.0);

        // Metadata row: LENGTH & BPM on left, honest ready/loading/downloading status on right.
        let meta_y = hero_y + banner_h + 18.0;
        let len_s = format_len(q.item.length_ms);
        let bpm_s = if q.item.bpm > 0.0 {
            format!("{:.0}", q.item.bpm * state.play_speed().0.max(0.01))
        } else {
            "-".into()
        };
        let meta = tr!(
            "LENGTH {len_s}   BPM {bpm_s}",
            "長さ {len_s}   BPM {bpm_s}",
            "ДЛИНА {len_s}   BPM {bpm_s}";
            [] [len_s, bpm_s]
        );
        c.text(hero_x + pad, meta_y, 8.0, rgba(0x9a9ab0, 1.0), &meta);

        let (status, scol) = source_line(&q.source, loading);
        let st_w = Canvas::text_width(8.0, &status);
        c.text(
            hero_x + hero_w - pad - st_w,
            meta_y,
            8.0,
            rgba(scol, 1.0),
            &status,
        );

        // Countdown / progress bar inset inside the bottom edge of the hero card.
        let bar_y_bot = hero_y + hero_h - 11.0;
        let track_w = inner_w;
        c.round_rect(
            [hero_x + pad, bar_y_bot],
            [track_w, 4.0],
            2.0,
            rgba(0x1b1f2e, 1.0),
            rgba(0, 0.0),
            0.0,
        );
        let frac = if held {
            1.0
        } else if !wait_gap.is_zero() {
            (1.0 - (since.as_secs_f32() / wait_gap.as_secs_f32()).clamp(0.0, 1.0)).clamp(0.0, 1.0)
        } else {
            q.source.download_progress().unwrap_or(1.0)
        };
        let bar_col = if held {
            0xffd24a
        } else if loading {
            0x22d4c0
        } else {
            role_col
        };
        c.round_rect(
            [hero_x + pad, bar_y_bot],
            [(track_w * frac).max(4.0), 4.0],
            2.0,
            rgba(bar_col, 0.92),
            rgba(0, 0.0),
            0.0,
        );
    } else {
        // Queue is empty: either fetching next batch or server outage.
        let outage = state.journey.outage(now) && !state.journey.local_mode;
        style::cold_panel(c, [hero_x, hero_y, hero_w, hero_h]);
        c.round_rect(
            [hero_x, hero_y],
            [hero_w, hero_h],
            style::radius::CARD,
            rgba(0, 0.0),
            rgba(
                if outage {
                    style::DANGER
                } else {
                    style::tone::EDGE
                },
                0.65,
            ),
            1.2,
        );
        if outage {
            c.text_centered(
                hero_x + hero_w / 2.0,
                hero_y + 38.0,
                11.0,
                rgba(0xffffff, 1.0),
                tr!(
                    "Can't reach the taiko.rs server",
                    "taiko.rs サーバーに接続できません",
                    "Сервер taiko.rs не отвечает"
                ),
            );
            if let Some(err) = &state.journey.error {
                c.text_centered(
                    hero_x + hero_w / 2.0,
                    hero_y + 62.0,
                    7.5,
                    rgba(0x9a9ab0, 1.0),
                    &Canvas::fit(7.5, hero_w - 32.0, err),
                );
            }
            let retry_lbl = tr!("Retry now", "今すぐ再試行", "Повторить");
            let off_lbl = tr!("Continue offline", "オフラインで続ける", "Играть оффлайн");
            let rw = Canvas::text_width(8.0, retry_lbl) + 34.0;
            let ow = Canvas::text_width(8.0, off_lbl) + 34.0;
            let bx = hero_x + (hero_w - (rw + 10.0 + ow)) / 2.0 + 6.0;
            let by = hero_y + 124.0;
            let bw = style::block_button(
                c,
                [bx, by],
                &style::PLAY,
                retry_lbl,
                None,
                8.0,
                true,
                hit::Hit::JourneyRetry,
                28.0,
            );
            style::block_button(
                c,
                [bx + bw + 10.0, by],
                &style::FOLDER,
                off_lbl,
                None,
                8.0,
                false,
                hit::Hit::JourneyOffline,
                28.0,
            );
        } else {
            c.text_centered(
                hero_x + hero_w / 2.0,
                hero_y + hero_h / 2.0 - 6.0,
                10.0,
                rgba(0xb4b4c8, 1.0),
                eyebrow,
            );
        }
    }

    // Right side card: map after next.
    if let Some(q) = state.journey.queue.get(1) {
        let len_s = format_len(q.item.length_ms);
        let (_, ncol) = super::journey::role_badge(&q.item.role);
        draw_side_card(
            c,
            [hero_x + hero_w + gap_x, side_y, side_w, side_h],
            &SideCard {
                eyebrow: tr!("AFTER THIS", "その次", "ПОТОМ"),
                stripe_col: ncol,
                item: &q.item,
                line: Some((
                    tr!("Length {len_s}", "長さ {len_s}", "Длина {len_s}"; [] [len_s]),
                    0x9a9ab0,
                )),
                footer: Some(tr!("N: swap", "N: 変更", "N: заменить")),
                alpha: 0.55 * turn,
            },
        );
    }

    // 3. Bottom controls & hints.
    draw_buttons(c, held);

    let mut hints = String::from(tr!(
        "N: another next one",
        "N: 次の次を変更",
        "N: другая следующая"
    ));
    if last.is_some_and(|p| p.vote_on.is_some()) {
        hints.push_str(tr!(
            "   L/D: like/dislike",
            "   L/D: いいね/よくない",
            "   L/D: лайк/дизлайк"
        ));
    }
    if last.is_some_and(|p| p.cached.is_some()) {
        hints.push_str(tr!("   S: save", "   S: 保存", "   S: сохранить"));
    }
    c.text_centered(
        w / 2.0,
        h - 18.0,
        7.0,
        rgba(0x787890, 1.0),
        &Canvas::fit_hints(7.0, w - 24.0, &hints),
    );
}

/// What the marathon came to: solid modal card with KPI tiles, full-width Best Play row,
/// run outcome dot strip, and Play again / Back to Journey buttons.
fn draw_summary(state: &State, m: &Marathon, c: &mut Canvas, now: Instant) {
    let w = c.width;
    let h = c.height;
    // Full-screen dark scrim so background art never competes with the summary.
    c.rect(0.0, 0.0, w, h, rgba(0x080a12, 0.80));

    let pw = 472.0_f32.min(w - 28.0);
    let has_outcomes = !m.outcomes.is_empty();
    let ph = if has_outcomes { 318.0 } else { 300.0 };
    let x = (w - pw) / 2.0;
    let y = ((h - ph) / 2.0).max(20.0);

    let end_reason = m.ended.map(|(why, _)| why);
    let is_hc_end = end_reason == Some(MarathonEnd::Hardcore);
    let top_col = if is_hc_end { style::DANGER } else { 0xffd24a };

    c.round_rect(
        [x, y],
        [pw, ph],
        10.0,
        rgba(style::PANEL_COLD, 0.99),
        rgba(
            if is_hc_end {
                style::DANGER
            } else {
                style::tone::EDGE
            },
            0.95,
        ),
        1.5,
    );
    c.round_rect(
        [x + 10.0, y + 1.5],
        [pw - 20.0, 2.5],
        1.2,
        rgba(top_col, 0.95),
        rgba(0, 0.0),
        0.0,
    );

    let heading = match end_reason {
        Some(MarathonEnd::Hardcore) => {
            tr!(
                "HARDCORE: A FAIL ENDS IT",
                "ハードコア: 失敗で終了",
                "ХАРДКОР: ФЕЙЛ ВСЁ ЗАКОНЧИЛ"
            )
        }
        Some(MarathonEnd::OutOfMaps) => tr!("NO MORE MAPS", "譜面がありません", "КАРТЫ КОНЧИЛИСЬ"),
        _ => tr!("MARATHON OVER", "マラソン終了", "МАРАФОН ОКОНЧЕН"),
    };
    let title_col = if is_hc_end { style::DANGER } else { 0xffffff };
    let tw = Canvas::text_width(13.0, heading);
    style::title(c, x + (pw - tw) / 2.0, y + 16.0, 13.0, title_col, heading);

    // Subtitle: elapsed time + active modifiers.
    let mut sub = clock(m.duration(now));
    if state.cfg.journey_hardcore {
        sub.push_str(tr!("  ·  Hardcore", "  ·  ハードコア", "  ·  Хардкор"));
    }
    if state.cfg.journey_focus != crate::config::JourneyFocus::Any {
        sub.push_str(&format!("  ·  {}", state.cfg.journey_focus.name()));
    }
    if state.cfg.journey_chill {
        sub.push_str(tr!("  ·  Chill", "  ·  チル", "  ·  Чилл"));
    }
    c.text_centered(
        x + pw / 2.0,
        y + 36.0,
        8.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(8.0, pw - 32.0, &sub),
    );
    let pad = 16.0;
    let inner_w = pw - pad * 2.0;
    style::rule(c, x + pad, y + 52.0, inner_w);

    // 2x2 KPI tile grid (labels above values: zero collisions in any language).
    let (played, passed, failed, skipped) = (m.played(), m.passed, m.failed, m.skipped);
    let tile_gap = 10.0;
    let tile_w = (inner_w - tile_gap) / 2.0;
    let tile_h = 58.0;
    let gx = x + pad;
    let gy = y + 62.0;

    let draw_kpi = |c: &mut Canvas,
                    tx: f32,
                    ty: f32,
                    caption: &str,
                    value: &str,
                    val_col: u32,
                    sub_line: Option<String>| {
        style::tile(
            c,
            [tx, ty, tile_w, tile_h],
            style::radius::CONTROL,
            style::Surface::Lifted,
        );
        c.text(
            tx + 12.0,
            ty + 8.0,
            7.0,
            rgba(0x8888a4, 1.0),
            &Canvas::fit(7.0, tile_w - 24.0, caption),
        );
        c.text(
            tx + 12.0,
            ty + 22.0,
            13.0,
            rgba(val_col, 1.0),
            &Canvas::fit(13.0, tile_w - 24.0, value),
        );
        if let Some(sub) = sub_line {
            c.text(
                tx + 12.0,
                ty + 42.0,
                6.5,
                rgba(0x9a9ab0, 1.0),
                &Canvas::fit(6.5, tile_w - 24.0, &sub),
            );
        }
    };

    let breakdown = tr!(
        "{passed} pass  ·  {failed} fail  ·  {skipped} skip",
        "{passed}クリア · {failed}失敗 · {skipped}スキップ",
        "{passed} ок  ·  {failed} фейл  ·  {skipped} скип";
        [] [passed, failed, skipped]
    );
    draw_kpi(
        c,
        gx,
        gy,
        tr!("Maps played", "プレイした譜面", "Сыграно карт"),
        &played.to_string(),
        0xffffff,
        Some(breakdown),
    );
    let acc_val = m
        .average_accuracy()
        .map_or("-".into(), |a| format!("{:.2}%", a * 100.0));
    draw_kpi(
        c,
        gx + tile_w + tile_gap,
        gy,
        tr!("Average accuracy", "平均精度", "Средняя точность"),
        &acc_val,
        0x22d4c0,
        None,
    );
    let pp_val = format!("{:.0}pp", m.pp);
    draw_kpi(
        c,
        gx,
        gy + tile_h + tile_gap,
        tr!("pp from passes", "クリアのpp", "pp с пройденных"),
        &pp_val,
        0xffd24a,
        None,
    );
    let streak = m.best_streak;
    let streak_sub = tr!(
        "Best streak: {streak}",
        "最長連続クリア: {streak}",
        "Лучшая серия: {streak}";
        [] [streak]
    );
    draw_kpi(
        c,
        gx + tile_w + tile_gap,
        gy + tile_h + tile_gap,
        tr!(
            "Best marathon combo",
            "最大マラソンコンボ",
            "Лучшее комбо марафона"
        ),
        &format!("x{}", m.best_combo),
        0xffd24a,
        Some(streak_sub),
    );

    // Full-width Best Play tile.
    let bp_y = gy + tile_h * 2.0 + tile_gap * 2.0;
    let bp_h = 44.0;
    style::tile(
        c,
        [gx, bp_y, inner_w, bp_h],
        style::radius::CONTROL,
        style::Surface::Lifted,
    );
    c.text(
        gx + 12.0,
        bp_y + 7.0,
        7.0,
        rgba(0x8888a4, 1.0),
        tr!("Best play", "ベストプレイ", "Лучшая игра"),
    );
    match &m.best {
        Some((name, pp)) => {
            let pp_s = format!("{pp:.0}pp");
            let pp_w = Canvas::text_width(11.0, &pp_s);
            c.text(
                gx + inner_w - 12.0 - pp_w,
                bp_y + 18.0,
                11.0,
                rgba(0xffd24a, 1.0),
                &pp_s,
            );
            c.text(
                gx + 12.0,
                bp_y + 21.0,
                9.0,
                rgba(0xffffff, 1.0),
                &Canvas::fit(9.0, inner_w - 34.0 - pp_w, name),
            );
        }
        None => {
            c.text(gx + 12.0, bp_y + 21.0, 9.0, rgba(0x787890, 1.0), "-");
        }
    }

    // Run history dot strip (green pass, red fail, gray skip).
    let mut next_y = bp_y + bp_h + 10.0;
    if has_outcomes {
        let shown: &[MarathonOutcome] = if m.outcomes.len() > 28 {
            &m.outcomes[m.outcomes.len() - 28..]
        } else {
            &m.outcomes
        };
        let step = 12.0;
        let strip_w = (shown.len().saturating_sub(1)) as f32 * step;
        let start_x = x + pw / 2.0 - strip_w / 2.0;
        let dot_y = next_y + 4.0;
        for (k, out) in shown.iter().enumerate() {
            let col = match out {
                MarathonOutcome::Passed => 0x4ee07a,
                MarathonOutcome::Failed => 0xff4d5a,
                MarathonOutcome::Skipped => 0x5c5c74,
            };
            c.circle(
                start_x + k as f32 * step,
                dot_y,
                3.5,
                rgba(col, 0.95),
                rgba(0xffffff, 0.3),
                0.8,
            );
        }
        next_y += 18.0;
    }

    // Clickable footer buttons: Play again (R) and Back to Journey (Enter).
    let again_lbl = tr!("Play again", "もう一度", "Ещё раз");
    let back_lbl = tr!("Back to Journey", "ジャーニーに戻る", "Назад в Journey");
    let aw = style::icon_button_width(again_lbl, Some("R"), 8.0);
    let bw = style::icon_button_width(back_lbl, Some("Enter"), 8.0);
    let btn_gap = 12.0;
    let total_w = aw + btn_gap + bw;
    let bx = x + (pw - total_w) / 2.0 + 6.0;
    let by = next_y + 18.0;
    style::block_button(
        c,
        [bx, by],
        &style::PLAY,
        again_lbl,
        Some("R"),
        8.0,
        false,
        hit::Hit::MarathonRestart,
        28.0,
    );
    style::block_button(
        c,
        [bx + aw + btn_gap, by],
        &style::BACK,
        back_lbl,
        Some("Enter"),
        8.0,
        true,
        hit::Hit::Key(KeyCode::Enter),
        28.0,
    );
}

impl State {
    /// Starts a marathon from the Journey screen: its first map comes up on the wheel.
    pub(crate) fn marathon_start(&mut self) {
        self.journey.settings = None;
        self.journey.marathon = Some(Marathon::new(Instant::now()));
        self.stop_audio();
        self.scene = MarathonScreen::scene();
    }

    /// A marathon's map played to its end (`play`, None when its item was gone): counted, and
    /// on to the next one, or the end of a hardcore marathon on a fail.
    pub(crate) fn marathon_after_play(
        &mut self,
        play: Option<MarathonPlay>,
        failed: bool,
    ) -> Scene {
        self.report_smoothness();
        self.journey.in_play = false;
        self.journey.playing_cached = None;
        self.journey.advance(if failed {
            PastOutcome::Failed
        } else {
            PastOutcome::Passed
        });
        self.journey_refill();
        let hardcore = self.cfg.journey_hardcore;
        if let Some(m) = &mut self.journey.marathon {
            m.map_ended();
            if let Some(play) = play {
                m.record(play);
            }
            if failed && hardcore {
                m.end(MarathonEnd::Hardcore, Instant::now());
            }
        }
        if self.cfg.journey_gap == MarathonGap::Seamless
            && let Some(next) = self.marathon_seamless_next()
        {
            return next;
        }
        MarathonScreen::scene()
    }

    /// The next map, already loaded when the last one ends: it starts at once (seamless).
    /// None when it isn't ready, for the wheel to wait for it.
    fn marathon_seamless_next(&mut self) -> Option<Scene> {
        if self.journey.marathon.as_ref()?.ended.is_some() {
            return None;
        }
        let entry = self.journey_current_entry(true)?;
        let mut p = self.marathon_take_preload(entry)?;
        match p.job.poll_ended() {
            Some(Some(Ok((map, track)))) => {
                self.journey.in_play = true;
                self.play_begins(entry, None, true);
                self.pending_mods = p.mods;
                Some(self.marathon_start_map(entry, map, track))
            }
            // Still loading: the wheel waits for it.
            None => {
                if let Some(m) = &mut self.journey.marathon {
                    m.preload = Some(p);
                }
                None
            }
            // It can't be: the wheel loads it again, and says so.
            Some(_) => None,
        }
    }

    /// A marathon's next map, loaded: it starts (past its intro, if so set), its song fading in
    /// over the last one's.
    fn marathon_start_map(&mut self, entry: usize, map: Beatmap, track: Arc<Track>) -> Scene {
        if let Some(a) = &self.audio {
            a.crossfade_next(CROSSFADE_OUT_MS, CROSSFADE_IN_MS);
        }
        let skip_intro = self.cfg.journey_skip_intro;
        super::loading::start_game(self, entry, None, skip_intro, map, track)
    }

    /// In a marathon's play at map time `time`, its last note at `last_note`: once the end is
    /// near, the map up next starts loading, to start the moment this one ends.
    pub(crate) fn marathon_preload(&mut self, time: f64, last_note: f64) {
        if time < last_note - PRELOAD_LEAD_MS {
            return;
        }
        if self
            .journey
            .marathon
            .as_ref()
            .is_none_or(|m| m.preload.is_some() || m.ended.is_some())
        {
            return;
        }
        let Some(path) = self.marathon_upcoming_path() else {
            return;
        };
        let mods = self.cfg.mods;
        let rate = self.journey_rate_for(1);
        let keep_pitch = self.cfg.rate_keep_pitch;
        let speed = mods.speed(rate, keep_pitch);
        let job = self.spawn_loader_at(path.clone(), speed, mods, None, true, None);
        if let Some(m) = &mut self.journey.marathon {
            m.preload = Some(Preload {
                path,
                mods,
                rate,
                keep_pitch,
                job,
            });
        }
    }

    /// The chart of the map up after the one played now, when it's here to be read.
    fn marathon_upcoming_path(&self) -> Option<std::path::PathBuf> {
        let q = self.journey.queue.get(1)?;
        match &q.source {
            MapSource::Local(path) | MapSource::Saved(path) => Some(path.clone()),
            MapSource::Cached { entry, .. } => Some(entry.path.clone()).filter(|p| p.is_file()),
            _ => None,
        }
    }

    /// The map loaded ahead of time, if it is `entry`'s, for the mods and speed set now.
    fn marathon_take_preload(&mut self, entry: usize) -> Option<Preload> {
        let p = self.journey.marathon.as_mut()?.preload.take()?;
        let same = self.library.get(entry).is_some_and(|e| e.path == p.path)
            && p.mods == self.cfg.mods
            && (p.rate - self.journey_rate_for(0)).abs() < 1e-6
            && p.keep_pitch == self.cfg.rate_keep_pitch;
        same.then_some(p)
    }

    /// The pause menu's Skip: the map being played is left (no score), the server hears of it
    /// as a skip, and the wheel turns to the next (or ends a hardcore marathon).
    pub(crate) fn marathon_skip_map(&mut self) {
        let seen_secs = self
            .play
            .as_deref()
            .map_or(0.0, |g| (g.time() / 1000.0).max(0.0));
        let item = self.journey.current().map(|q| q.item.clone());
        self.report_smoothness();
        self.stop_audio();
        self.journey.in_play = false;
        self.journey.playing_cached = None;
        self.journey_skip_quiet(seen_secs);
        if let Some(m) = &mut self.journey.marathon {
            m.combo_broken();
            if self.cfg.journey_hardcore {
                m.end(MarathonEnd::Hardcore, Instant::now());
            }
        }
        if let (Some(m), Some(item)) = (&mut self.journey.marathon, item) {
            m.record(MarathonPlay {
                item,
                outcome: MarathonOutcome::Skipped,
                accuracy: 0.0,
                max_combo: 0,
                misses: 0,
                pp: None,
                vote_on: None,
                vote: None,
                cached: None,
                saved: false,
            });
        }
        self.scene = MarathonScreen::scene();
    }

    /// Ends the marathon (from its pause menu, its wheel, or anything leaving a play): what it
    /// came to is shown. A map left partway stays up in Journey, uncounted.
    pub(crate) fn marathon_end(&mut self) {
        let Some(m) = &mut self.journey.marathon else {
            return;
        };
        m.end(MarathonEnd::Quit, Instant::now());
        if self.journey.in_play {
            self.report_smoothness();
            self.stop_audio();
            self.journey.in_play = false;
            self.journey.playing_cached = None;
        }
        self.scene = MarathonScreen::scene();
    }

    /// From the marathon's summary back to the Journey screen.
    pub(crate) fn marathon_close(&mut self) {
        self.journey.marathon = None;
        self.journey.shown_at = Instant::now();
        self.stop_audio();
        self.scene = super::journey::JourneyScreen::scene();
        self.journey_preview_current();
    }

    /// Likes or dislikes the marathon's last map (again: takes it back); liked, it's saved when
    /// Journey saves liked maps.
    pub(crate) fn marathon_vote(&mut self, vote: online::MapVote) {
        let Some(play) = self.journey.marathon.as_mut().and_then(|m| m.last.as_mut()) else {
            return;
        };
        let Some(map) = play.vote_on.clone() else {
            return;
        };
        let vote = (play.vote != Some(vote)).then_some(vote);
        play.vote = vote;
        self.send_vote(map, vote);
        if vote == Some(online::MapVote::Like) && self.cfg.journey_save_on_like {
            self.marathon_save_last();
        }
    }

    /// Saves the marathon's last map from Journey's cache to the library.
    pub(crate) fn marathon_save_last(&mut self) {
        let Some(cached) = self
            .journey
            .marathon
            .as_mut()
            .and_then(|m| m.last.as_mut())
            .and_then(|p| p.cached.clone())
        else {
            return;
        };
        // Saved the way a map is from its results: as the map just played.
        let next = self.journey.playing_cached.replace(cached);
        self.journey_save_current();
        let saved = self.journey.playing_cached.is_none();
        self.journey.playing_cached = next;
        if saved && let Some(p) = self.journey.marathon.as_mut().and_then(|m| m.last.as_mut()) {
            p.cached = None;
            p.saved = true;
        }
    }
}
