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
            if matches!(
                code,
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape | KeyCode::Space
            ) {
                state.marathon_close();
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
            draw_summary(m, canvas);
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
                    m.end(MarathonEnd::OutOfMaps, Instant::now());
                }
                return None;
            }
        }
        let entered = *self.entered.get_or_insert(now);
        let since = now.saturating_duration_since(entered);
        draw_wheel(state, canvas, since, self.held, self.load.is_some());

        let due = self.go_now || since >= gap(&state.cfg);
        if due && !self.held {
            let Some((rx, entry)) = &mut self.load else {
                return None;
            };
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
        None
    }
}

/// A marathon's combo in a play's bottom-right corner (over Non-stop's "up next" card while it
/// shows).
pub(crate) fn draw_combo_chip(c: &mut Canvas, combo: u32, over_card: bool) {
    let text = format!("x{combo}");
    let label = tr!("MARATHON", "マラソン", "МАРАФОН");
    let w = Canvas::text_width(7.0, label) + 8.0 + Canvas::text_width(11.0, &text) + 20.0;
    let h = 22.0;
    let x = c.width - 12.0 - w;
    let y = if over_card {
        c.height - 62.0 - 6.0 - h
    } else {
        c.height - 12.0 - h
    };
    style::cold_panel(c, [x, y, w, h]);
    c.text(x + 10.0, y + 8.0, 7.0, rgba(0x22d4c0, 1.0), label);
    c.text(
        x + w - 10.0 - Canvas::text_width(11.0, &text),
        y + 5.0,
        11.0,
        rgba(0xffd24a, 1.0),
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
    let mid = c.height - 52.0;
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

/// A map's card on the wheel.
struct Card<'a> {
    item: &'a online::JourneyItem,
    stars: f64,
    /// Under the name: what became of it, or where it is.
    line: Option<(String, u32)>,
    lit: bool,
}

fn draw_card(c: &mut Canvas, [x, y, w, h]: [f32; 4], card: &Card, alpha: f32) {
    let start = c.shapes.len();
    if card.lit {
        style::panel(c, x, y, w, h);
    } else {
        style::cold_panel(c, [x, y, w, h]);
    }
    let item = card.item;
    let title = format!("{} - {}", item.artist, item.title);
    c.text(
        x + 12.0,
        y + 10.0,
        9.0,
        rgba(if card.lit { 0xffffff } else { 0xc8c8d8 }, 1.0),
        &Canvas::fit(9.0, w - 24.0, &title),
    );
    let stars = format!("{:.2}*", card.stars);
    let sw = Canvas::text_width(9.0, &stars);
    c.text(
        x + w - 12.0 - sw,
        y + 26.0,
        9.0,
        rgba(0xffd24a, 1.0),
        &stars,
    );
    c.text(
        x + 12.0,
        y + 27.0,
        8.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(8.0, w - 36.0 - sw, &format!("[{}]", item.version)),
    );
    if let Some((line, col)) = &card.line {
        c.text(
            x + 12.0,
            y + h - 20.0,
            8.0,
            rgba(*col, 1.0),
            &Canvas::fit(8.0, w - 24.0, line),
        );
    }
    c.fade_since(start, alpha, 1.0);
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

/// The wheel: the map just played slides off to the left as the next one comes to the middle,
/// with the one after it on the right; and under it, the marathon so far and what the keys do.
fn draw_wheel(state: &State, c: &mut Canvas, since: Duration, held: bool, loading: bool) {
    let Some(m) = &state.journey.marathon else {
        return;
    };
    let ms = since.as_secs_f32() * 1000.0;
    let w = c.width;
    let cw = (w * 0.3).clamp(180.0, 280.0);
    let ch = 62.0;
    let gap = 14.0;
    let y = 24.0;
    let turn = ease(ms, WHEEL_MS);
    let mid = (w - cw) / 2.0 + (1.0 - turn) * (cw + gap);
    let last = m.last.as_ref();
    let current = state.journey.current();

    if let Some(play) = last {
        let card = Card {
            item: &play.item,
            stars: play.item.stars,
            line: Some(outcome_line(play)),
            lit: false,
        };
        draw_card(
            c,
            [mid - cw - gap, y, cw, ch],
            &card,
            0.55 + 0.45 * (1.0 - turn),
        );
    }
    if let Some(q) = current {
        // The star rating rolls from the last map's to this one's.
        let from = last.map_or(q.item.stars, |p| p.item.stars);
        let roll = ease(ms, STARS_MS) as f64;
        let card = Card {
            item: &q.item,
            stars: from + (q.item.stars - from) * roll,
            line: Some(source_line(&q.source, loading)),
            lit: true,
        };
        draw_card(c, [mid, y, cw, ch], &card, 1.0);
    }
    if let Some(q) = state.journey.queue.get(1) {
        let card = Card {
            item: &q.item,
            stars: q.item.stars,
            line: Some((
                tr!("N: another", "N: ほかの譜面", "N: другую").into(),
                0x787890,
            )),
            lit: false,
        };
        draw_card(c, [mid + cw + gap, y, cw, ch], &card, 0.55 * turn);
    }

    // The middle: what happens next.
    let cy = 196.0;
    let (head, col) = if held {
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
    c.text_centered(w / 2.0, cy, 9.0, rgba(col, 1.0), head);
    if let Some(q) = current {
        c.text_centered(
            w / 2.0,
            cy + 18.0,
            14.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(14.0, w - 48.0, &q.item.title),
        );
        c.text_centered(
            w / 2.0,
            cy + 40.0,
            9.0,
            rgba(0xa8a8bc, 1.0),
            &Canvas::fit(
                9.0,
                w - 48.0,
                &format!("{}  [{}]", q.item.artist, q.item.version),
            ),
        );
    }

    // The marathon so far.
    let n = m.played() + 1;
    let time = clock(m.duration(Instant::now()));
    let mut so_far = tr!("MARATHON  ·  map {n}  ·  {time}", "マラソン  ·  {n}譜面目  ·  {time}", "МАРАФОН  ·  карта {n}  ·  {time}"; [] [n, time]);
    if let Some(acc) = m.average_accuracy() {
        so_far.push_str(&format!("  ·  {:.2}%", acc * 100.0));
    }
    if m.pp > 0.0 {
        so_far.push_str(&format!("  ·  {:.0}pp", m.pp));
    }
    if state.cfg.journey_hardcore {
        so_far.push_str(tr!("  ·  HARDCORE", "  ·  ハードコア", "  ·  ХАРДКОР"));
    }
    c.text_centered(
        w / 2.0,
        120.0,
        8.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(8.0, w - 32.0, &so_far),
    );
    if m.combo > 0 {
        let combo = m.combo;
        c.text_centered(
            w / 2.0,
            140.0,
            12.0,
            rgba(0xffd24a, 1.0),
            &tr!("MARATHON COMBO x{combo}", "マラソンコンボ x{combo}", "КОМБО МАРАФОНА x{combo}"; [] [combo]),
        );
    }
    draw_buttons(c, held);

    // Nothing needs a key: the next map starts by itself. The buttons above say theirs.
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
        c.height - 18.0,
        7.0,
        rgba(0x787890, 1.0),
        &Canvas::fit_hints(7.0, w - 24.0, &hints),
    );
}

/// What the marathon came to.
fn draw_summary(m: &Marathon, c: &mut Canvas) {
    let w = c.width;
    let pw = 360.0_f32.min(w - 32.0);
    let ph = 258.0;
    let x = (w - pw) / 2.0;
    let y = (c.height - ph) / 2.0;
    style::panel(c, x, y, pw, ph);
    let heading = match m.ended.map(|(why, _)| why) {
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
    let tw = Canvas::text_width(12.0, heading);
    style::title(c, x + (pw - tw) / 2.0, y + 16.0, 12.0, 0xffffff, heading);
    style::rule(c, x + 16.0, y + 40.0, pw - 32.0);

    let (played, passed, failed, skipped) = (m.played(), m.passed, m.failed, m.skipped);
    let rows: Vec<(&str, String)> = vec![
        (
            tr!("Maps played", "プレイした譜面", "Сыграно карт"),
            tr!("{played}  ({passed} passed, {failed} failed)", "{played}  (クリア{passed}, 失敗{failed})", "{played}  (пройдено {passed}, фейлов {failed})"; [] [played, passed, failed]),
        ),
        (tr!("Skipped", "スキップ", "Пропущено"), skipped.to_string()),
        (
            tr!("Time", "時間", "Время"),
            clock(m.duration(Instant::now())),
        ),
        (
            tr!("Average accuracy", "平均精度", "Средняя точность"),
            m.average_accuracy()
                .map_or("-".into(), |a| format!("{:.2}%", a * 100.0)),
        ),
        (
            tr!("pp from passes", "クリアのpp", "pp с пройденных"),
            format!("{:.0}", m.pp),
        ),
        (
            tr!("Best streak", "最長連続クリア", "Лучшая серия"),
            m.best_streak.to_string(),
        ),
        (
            tr!(
                "Best marathon combo",
                "最大マラソンコンボ",
                "Лучшее комбо марафона"
            ),
            m.best_combo.to_string(),
        ),
        (
            tr!("Best play", "ベストプレイ", "Лучшая игра"),
            m.best
                .as_ref()
                .map_or("-".into(), |(name, pp)| format!("{pp:.0}pp  {name}")),
        ),
    ];
    for (i, (label, value)) in rows.iter().enumerate() {
        let ry = y + 52.0 + i as f32 * 22.0;
        c.text(x + 18.0, ry, 9.0, rgba(0x9a9ab0, 1.0), label);
        let vx = x + pw * 0.45;
        c.text(
            vx,
            ry,
            9.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(9.0, x + pw - 18.0 - vx, value),
        );
    }
    c.text_centered(
        x + pw / 2.0,
        y + ph - 18.0,
        7.0,
        rgba(0x787890, 1.0),
        tr!(
            "Enter: back to Journey",
            "Enter: ジャーニーに戻る",
            "Enter: назад в Journey"
        ),
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
        let rate = self.cfg.rate;
        let speed = mods.speed(rate, self.cfg.rate_keep_pitch);
        let job = self.spawn_loader_at(path.clone(), speed, mods, None, true, None);
        if let Some(m) = &mut self.journey.marathon {
            m.preload = Some(Preload {
                path,
                mods,
                rate,
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
            && p.rate == self.cfg.rate;
        same.then_some(p)
    }

    /// The pause menu's Skip: the map being played is left (no score), the server hears of it
    /// as a skip, and the wheel turns to the next.
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
        }
    }
}
