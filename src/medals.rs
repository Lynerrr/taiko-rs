//! Medals in the game: the screen listing them, and the moment one is earned. The list and the
//! pictures come from the server, which decides who has what.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use winit::keyboard::KeyCode;

use crate::online::{self, medal_badge, medal_colour, said, MedalNote, MedalRow, MedalsView, BADGE};
use crate::render::{rgba, Canvas, Hit, VIRTUAL_HEIGHT};

/// Rows of medals on a page, and how tall each is.
const ROW_H: f32 = 44.0;
/// What the mod collection takes above the medals of its tab.
const TILES_H: f32 = 118.0;
const TILE_W: f32 = 20.0;
const TILE_H: f32 = 12.0;
/// Goals on the overview.
const NEXT_UP: usize = 6;
/// The medals shown one by one when several come at once; the rest get a line.
pub const SHOWN_AT_ONCE: usize = 3;

/// How long the medal takes to land, how long it stays, and how long it takes to leave (ms).
const LAND_MS: f32 = 720.0;
const STAY_MS: f32 = 4200.0;
const LEAVE_MS: f32 = 260.0;
/// Before this it can't be skipped: a key still held from the play shouldn't throw it away.
const SKIP_AFTER_MS: f32 = 450.0;

pub enum State {
    Loading(Receiver<Result<MedalsView, String>>),
    Loaded(Box<MedalsView>),
    Failed(String),
}

pub struct View {
    /// Whose medals; None: not logged in, just the list.
    pub name: Option<String>,
    pub state: State,
    /// 0: the overview; then a section each.
    pub tab: usize,
    /// How far the page is scrolled, in rows.
    pub scroll: usize,
}

/// The tabs after the overview, in the server's order of sections.
fn section_name(section: usize) -> &'static str {
    match section {
        0 => tr!("PATH", "道", "ПУТЬ"),
        1 => tr!("MODS", "モード", "МОДЫ"),
        2 => tr!("DAN", "段位", "ДАНЫ"),
        3 => tr!("SKILL", "腕前", "НАВЫК"),
        4 => tr!("TRIALS", "試練", "ИСПЫТАНИЯ"),
        5 => tr!("GROWTH", "成長", "РОСТ"),
        6 => tr!("RIVALS", "ライバル", "СОПЕРНИКИ"),
        7 => tr!("URA", "裏", "УРА"),
        _ => tr!("TOGETHER", "みんなで", "ВМЕСТЕ"),
    }
}

pub const TABS: usize = 10;

fn tab_name(tab: usize) -> &'static str {
    if tab == 0 {
        tr!("OVERVIEW", "概要", "ОБЗОР")
    } else {
        section_name(tab - 1)
    }
}

fn tier_name(tier: u8, secret: bool) -> &'static str {
    match (secret, tier) {
        (true, _) => "Ura",
        (_, 1) => "Kantan",
        (_, 2) => "Futsuu",
        (_, 3) => "Muzukashii",
        _ => "Oni",
    }
}

/// A medal's looks: its picture, tier, whether it's secret and whether it's in colour.
struct Look<'a> {
    art: &'a [String],
    tier: u8,
    secret: bool,
    lit: bool,
}

/// Draws a badge around (`cx`, `cy`): `scale` is screen pixels per badge pixel, across and down.
fn badge(c: &mut Canvas, [cx, cy]: [f32; 2], [sx, sy]: [f32; 2], look: Look, alpha: f32) {
    let half = BADGE as f32 / 2.0;
    for (y, row) in medal_badge(look.art, look.tier, look.secret, look.lit).iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            let Some(colour) = row[x] else {
                x += 1;
                continue;
            };
            let start = x;
            while x < row.len() && row[x] == Some(colour) {
                x += 1;
            }
            c.rect(cx + (start as f32 - half) * sx, cy + (y as f32 - half) * sy, (x - start) as f32 * sx, sy, rgba(colour, alpha));
        }
    }
}

/// A medal small enough for a row of them.
pub fn small_badge(c: &mut Canvas, cx: f32, cy: f32, m: &MedalNote) {
    badge(c, [cx, cy], [0.75, 0.75], Look { art: &m.art, tier: m.tier, secret: m.secret, lit: true }, 1.0);
}

/// A medal nobody has found: an empty ring and a question mark.
fn mystery(c: &mut Canvas, cx: f32, cy: f32, scale: f32) {
    badge(c, [cx, cy], [scale, scale], Look { art: &[], tier: 1, secret: true, lit: false }, 1.0);
    c.text_centered(cx, cy - 7.0 * scale / 1.5, 10.0 * scale / 1.5, rgba(0x6f6f86, 1.0), "?");
}

/// How far along a goal is, 0 to 1.
fn progress(m: &MedalRow) -> f32 {
    if m.need == 0 {
        0.0
    } else {
        (m.have as f32 / m.need as f32).min(1.0)
    }
}

/// What's closest to being earned: the goals to show first.
fn next_up(view: &MedalsView) -> Vec<&MedalRow> {
    let mut goals: Vec<&MedalRow> = view.medals.iter().filter(|m| m.at.is_none() && m.need > 0 && m.have > 0).collect();
    goals.sort_by(|a, b| progress(b).total_cmp(&progress(a)).then(a.tier.cmp(&b.tier)));
    goals.truncate(NEXT_UP);
    goals
}

/// The medals of a section, earned first.
fn of_section(view: &MedalsView, section: usize) -> Vec<&MedalRow> {
    let mut rows: Vec<&MedalRow> = view.medals.iter().filter(|m| usize::from(m.section) == section).collect();
    rows.sort_by_key(|m| m.at.is_none());
    rows
}

impl View {
    /// Asks the server for the medals (on a background thread).
    pub fn open(server: String, name: Option<String>) -> View {
        let (tx, rx) = mpsc::channel();
        let who = name.clone().unwrap_or_default();
        std::thread::spawn(move || {
            let result = online::call(&server, online::Request::Medals { name: who }).and_then(|r| match r {
                online::Reply::Medals(m) => Ok(*m),
                _ => Err(crate::lang::odd_reply()),
            });
            let no_medals = || tr!("this server doesn't have medals yet", "このサーバーにはまだメダルがありません", "на этом сервере ещё нет медалей").to_string();
            let _ = tx.send(result.map_err(|e| if e == "bad request" { no_medals() } else { e }));
        });
        View { name, state: State::Loading(rx), tab: 0, scroll: 0 }
    }

    pub fn poll(&mut self) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(v)) => self.state = State::Loaded(Box::new(v)),
                Ok(Err(e)) => self.state = State::Failed(e),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.state = State::Failed(tr!("lost the connection", "接続が切れました", "соединение потеряно").into()),
            }
        }
    }

    pub fn set_tab(&mut self, tab: usize) {
        self.tab = tab.min(TABS - 1);
        self.scroll = 0;
    }

    pub fn step_tab(&mut self, by: i32) {
        self.set_tab((self.tab as i32 + by).rem_euclid(TABS as i32) as usize);
    }

    pub fn wheel(&mut self, notches: i32) {
        self.scroll = self.scroll.saturating_add_signed(-notches as isize).min(self.rows().saturating_sub(1));
    }

    /// The rows on this tab that can scroll.
    fn rows(&self) -> usize {
        let State::Loaded(v) = &self.state else { return 0 };
        if self.tab == 0 {
            return 0;
        }
        of_section(v, self.tab - 1).len().div_ceil(2)
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.72));
        let (px, py, pw, ph) = (16.0, 20.0, w - 32.0, 444.0);
        c.round_rect([px, py], [pw, ph], 10.0, rgba(0x15151d, 0.98), rgba(0xFFD34D, 0.8), 1.5);
        let (by, bw) = (py + ph - 24.0, 56.0);
        c.button(px + pw - bw - 12.0, by, bw, 16.0, tr!("Back", "戻る", "Назад"), 7.0, Hit::Key(KeyCode::Escape), true);
        let title = tr!("MEDALS", "メダル", "МЕДАЛИ");
        c.text(px + 16.0, py + 12.0, 14.0, rgba(0xffffff, 1.0), title);
        let v = match &self.state {
            State::Loading(_) => {
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xc8c8d8, 1.0), tr!("loading the medals...", "メダルを読み込み中...", "загрузка медалей..."));
                return;
            }
            State::Failed(e) => {
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xFF4B4B, 1.0), &Canvas::fit(9.0, pw - 32.0, e));
                return;
            }
            State::Loaded(v) => v,
        };
        let (earned, total) = (v.earned, v.medals.len());
        let stats = match &self.name {
            Some(name) => tr!("{name}: {earned} of {total}", "{name}: {total}個中{earned}個", "{name}: {earned} из {total}";),
            None => tr!("Log in to earn them", "ログインで獲得できます", "Войди, чтобы их получать").to_string(),
        };
        let title_w = Canvas::text_width(14.0, title);
        c.text_right(px + pw - 16.0, py + 16.0, 8.0, rgba(0xFFD34D, 1.0), &Canvas::fit(8.0, pw - title_w - 56.0, &stats));
        // The tabs.
        let tab_w = (pw - 24.0) / TABS as f32;
        for tab in 0..TABS {
            let x = px + 12.0 + tab as f32 * tab_w;
            let on = tab == self.tab;
            let name = tab_name(tab);
            let hover = c.hovered(x, py + 32.0, tab_w, 14.0);
            let fill = if on { 0x2a2a38 } else if hover { 0x20202b } else { 0x1a1a24 };
            c.round_rect([x + 1.0, py + 32.0], [tab_w - 2.0, 14.0], 3.0, rgba(fill, 1.0), rgba(if on { 0xFFD34D } else { 0x2a2a38 }, 1.0), 1.0);
            c.text_centered(x + tab_w / 2.0, py + 36.0, 6.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), &Canvas::fit(6.0, tab_w - 6.0, name));
            c.hit(x, py + 32.0, tab_w, 14.0, Hit::MedalTab(tab));
        }
        let (top, bottom) = (py + 52.0, by - 4.0);
        let start = c.shapes.len();
        if self.tab == 0 {
            overview(c, v, px + 16.0, top, pw - 32.0, bottom);
        } else {
            let section = self.tab - 1;
            let mut y = top - self.scroll as f32 * ROW_H;
            if section == 1 {
                tiles(c, v, px + 16.0, y, pw - 32.0);
                y += TILES_H;
            }
            let rows = of_section(v, section);
            let col_w = (pw - 40.0) / 2.0;
            for (i, m) in rows.iter().enumerate() {
                let row_y = y + (i / 2) as f32 * ROW_H;
                if row_y + ROW_H < top || row_y > bottom {
                    continue;
                }
                medal_row(c, m, v.players, px + 16.0 + (i % 2) as f32 * (col_w + 8.0), row_y, col_w);
            }
            if rows.is_empty() {
                c.text_centered(px + pw / 2.0, top + 40.0, 8.0, rgba(0x6f6f86, 1.0), tr!("nothing here yet", "まだ何もありません", "здесь пока пусто"));
            }
        }
        c.clip_since(start, px + 2.0, top - 2.0, pw - 4.0, bottom - top + 2.0);
        c.text(px + 16.0, by + 5.0, 6.0, rgba(0x55556a, 1.0), &Canvas::fit_hints(6.0, pw - bw - 44.0, tr!("Left/Right tabs  wheel scrolls  Esc back", "左右でタブ  ホイールでスクロール  Escで戻る", "Влево/вправо вкладки  колесо листает  Esc назад")));
    }
}

/// One medal: its badge, its name, what it takes and how it's going.
fn medal_row(c: &mut Canvas, m: &MedalRow, players: u32, x: f32, y: f32, w: f32) {
    let lit = m.at.is_some();
    c.round_rect([x, y], [w, ROW_H - 3.0], 5.0, rgba(if lit { 0x1f1f2a } else { 0x18181f }, 1.0), rgba(0x2a2a38, 1.0), 1.0);
    let scale = 1.3;
    if m.art.is_empty() {
        mystery(c, x + 22.0, y + 20.5, scale);
    } else {
        badge(c, [x + 22.0, y + 20.5], [scale, scale], Look { art: &m.art, tier: m.tier, secret: m.secret, lit: lit || m.secret }, 1.0);
    }
    let room = w - 54.0;
    let name = m.name.as_ref().map_or("???", said);
    c.text(x + 44.0, y + 5.0, 8.0, rgba(if lit { 0xffffff } else { 0xb4b4c8 }, 1.0), &Canvas::fit(8.0, room, name));
    let about = m.how.as_ref().or(m.hint.as_ref()).map_or(tr!("Nobody has found this one", "まだ誰も見つけていません", "Пока никто не нашёл"), said);
    c.text(x + 44.0, y + 17.0, 6.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(6.0, room, about));
    let share = if players > 0 { f64::from(m.holders) * 100.0 / f64::from(players) } else { 0.0 };
    let tier = tier_name(m.tier, m.secret);
    c.text(x + 44.0, y + 28.0, 6.0, rgba(medal_colour(m.tier, m.secret), 1.0), tier);
    let right = x + w - 8.0;
    c.text_right(right, y + 28.0, 6.0, rgba(0x6f6f86, 1.0), &format!("{share:.0}%"));
    if !lit && m.need > 0 {
        let left = x + 44.0 + Canvas::text_width(6.0, tier) + 8.0;
        let bar = (right - 30.0 - left).max(0.0);
        c.rect(left, y + 30.0, bar, 3.0, rgba(0x2a2a38, 1.0));
        c.rect(left, y + 30.0, bar * progress(m), 3.0, rgba(medal_colour(m.tier, false), 1.0));
    }
}

/// The overview: how many, what's next, and the latest ones.
fn overview(c: &mut Canvas, v: &MedalsView, x: f32, top: f32, w: f32, bottom: f32) {
    let col_w = (w - 8.0) / 2.0;
    c.text(x, top, 7.0, rgba(0xFFD34D, 1.0), tr!("CLOSEST", "もうすぐ", "БЛИЖЕ ВСЕГО"));
    let goals = next_up(v);
    if goals.is_empty() {
        c.text(x, top + 14.0, 7.0, rgba(0x6f6f86, 1.0), tr!("Play a few maps to see your goals", "プレイすると目標が表示されます", "Сыграй пару карт, и тут появятся цели"));
    }
    for (i, m) in goals.iter().enumerate() {
        let y = top + 12.0 + i as f32 * ROW_H;
        if y + ROW_H <= bottom {
            medal_row(c, m, v.players, x, y, col_w);
        }
    }
    let right = x + col_w + 8.0;
    c.text(right, top, 7.0, rgba(0xFFD34D, 1.0), tr!("LATEST", "最新", "ПОСЛЕДНИЕ"));
    let mut latest: Vec<&MedalRow> = v.medals.iter().filter(|m| m.at.is_some()).collect();
    latest.sort_by_key(|m| std::cmp::Reverse(m.at));
    if latest.is_empty() {
        c.text(right, top + 14.0, 7.0, rgba(0x6f6f86, 1.0), tr!("None yet", "まだありません", "Пока нет"));
    }
    for (i, m) in latest.iter().take(NEXT_UP).enumerate() {
        let y = top + 12.0 + i as f32 * ROW_H;
        if y + ROW_H <= bottom {
            medal_row(c, m, v.players, right, y, col_w);
        }
    }
}

/// The mod collection: a tile per mod, by mod select column, lit once it's in.
fn tiles(c: &mut Canvas, v: &MedalsView, x: f32, top: f32, w: f32) {
    let have = v.collection.len();
    let size = v.collection_size;
    c.text(x, top, 7.0, rgba(0xFFD34D, 1.0), &tr!("COLLECTION  {have} of {size}", "コレクション  {size}個中{have}個", "КОЛЛЕКЦИЯ  {have} из {size}";));
    c.text_right(x + w, top, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit(6.0, w * 0.6, tr!("3★+, 80%+, up to 3 mods, no NF", "★3以上・精度80%以上・3モードまで・NFなし", "3★+, 80%+, до 3 модов, без NF")));
    for (row, (_, colour, mods)) in crate::mods::GROUPS.iter().enumerate() {
        for (col, m) in mods.iter().filter(|m| !matches!(m, crate::mods::Mod::Autoplay | crate::mods::Mod::Relax)).enumerate() {
            let (tx, ty) = (x + col as f32 * (TILE_W + 2.0), top + 12.0 + row as f32 * (TILE_H + 1.0));
            let acronym = m.acronym();
            let lit = v.collection.iter().any(|a| a == acronym);
            let (fill, ink) = if lit { (*colour, 0x0b0b10) } else { (0x20202b, 0x55556a) };
            c.rect(tx, ty, TILE_W, TILE_H, rgba(fill, 1.0));
            c.text_centered(tx + TILE_W / 2.0, ty + 2.5, 6.0, rgba(ink, 1.0), acronym);
        }
    }
}

/// The medals waiting to be shown, and the one on screen.
#[derive(Default)]
pub struct Unlock {
    queue: VecDeque<MedalNote>,
    current: Option<Showing>,
}

struct Showing {
    medal: MedalNote,
    since: Instant,
    /// When it was told to go.
    leaving: Option<Instant>,
}

impl Unlock {
    /// Medals to show, the rarest kinds first. Returns how many more there are than get shown.
    pub fn add(&mut self, mut medals: Vec<MedalNote>) -> usize {
        medals.sort_by_key(|m| std::cmp::Reverse((m.secret, m.tier)));
        let extra = medals.len().saturating_sub(SHOWN_AT_ONCE);
        medals.truncate(SHOWN_AT_ONCE);
        self.queue.extend(medals);
        extra
    }

    /// Something is on screen.
    pub fn showing(&self) -> bool {
        self.current.is_some()
    }

    /// Moves on as time passes: `allowed` is false while a song is playing. True when a new
    /// medal has just started, for its sound.
    pub fn tick(&mut self, allowed: bool) -> bool {
        if let Some(s) = &self.current {
            let ms = s.since.elapsed().as_secs_f32() * 1000.0;
            let gone = s.leaving.is_some_and(|l| l.elapsed().as_secs_f32() * 1000.0 >= LEAVE_MS);
            if gone {
                self.current = None;
            } else if ms >= STAY_MS && s.leaving.is_none() {
                self.skip();
            }
        }
        if self.current.is_none() && allowed
            && let Some(medal) = self.queue.pop_front() {
            self.current = Some(Showing { medal, since: Instant::now(), leaving: None });
            return true;
        }
        false
    }

    /// A key or a click: on to the next one, once it's had its moment.
    pub fn skip(&mut self) {
        if let Some(s) = &mut self.current
            && s.leaving.is_none()
            && s.since.elapsed().as_secs_f32() * 1000.0 >= SKIP_AFTER_MS {
            s.leaving = Some(Instant::now());
        }
    }

    pub fn draw(&self, c: &mut Canvas) {
        let Some(s) = &self.current else { return };
        let ms = s.since.elapsed().as_secs_f32() * 1000.0;
        let leave = s.leaving.map_or(0.0, |l| (l.elapsed().as_secs_f32() * 1000.0 / LEAVE_MS).min(1.0));
        draw_unlock(c, &s.medal, ms, 1.0 - leave);
    }
}

/// `t` eased out with a little overshoot.
fn ease_back(t: f32) -> f32 {
    let t = t - 1.0;
    1.0 + 2.7 * t * t * t + 1.7 * t * t
}

/// The moment itself: the screen dims and light gathers; the medal drops in spinning, hits with a
/// flash and a burst of sparks, and its name comes up. `ms`: since it began; `alpha`: 1, until it
/// leaves. It's a pure function of the time, so it can be tested and looked at frame by frame.
pub fn draw_unlock(c: &mut Canvas, m: &MedalNote, ms: f32, alpha: f32) {
    let colour = medal_colour(m.tier, m.secret);
    let (cx, cy) = (c.width / 2.0, 196.0);
    let dim = (ms / 300.0).min(1.0) * 0.86 * alpha;
    c.rect(0.0, 0.0, c.width, VIRTUAL_HEIGHT, rgba(0x000000, dim));
    let landed = ((ms - LAND_MS) / 400.0).clamp(0.0, 1.0);
    // Rays turning behind it, opening up as it lands.
    let rays = (ms / 500.0).min(1.0) * (0.35 + 0.65 * landed) * alpha;
    let spin = ms / 2600.0;
    for i in 0..12 {
        let start = c.shapes.len();
        let len = 150.0 + 40.0 * landed;
        c.rect(0.0, -3.0, len, 6.0, rgba(colour, 0.16 * rays));
        c.rect(0.0, -1.0, len * 0.8, 2.0, rgba(0xffffff, 0.10 * rays));
        c.rotate_since(start, [0.0, 0.0], spin + i as f32 * std::f32::consts::TAU / 12.0, 1.0, [cx, cy]);
    }
    c.glow(cx, cy, 130.0 + 70.0 * landed, 170.0, rgba(colour, 0.40 * alpha * (ms / 400.0).min(1.0)));
    // The medal drops in spinning and slows to face you.
    let fall = (ms / LAND_MS).min(1.0);
    let eased = ease_back(fall);
    let size = 4.0 * (0.25 + 0.75 * eased);
    let turn = (1.0 - fall).powi(2) * std::f32::consts::TAU * 3.0;
    let dy = (1.0 - eased) * -70.0;
    let lift = (-(ms - LAND_MS).max(0.0) / 130.0).exp();
    let sway = ((ms - LAND_MS).max(0.0) / 800.0).sin() * 2.0 * landed;
    // The flash and the ring going out on impact, and sparks flying.
    if ms >= LAND_MS {
        let since = ms - LAND_MS;
        let ring = (since / 520.0).min(1.0);
        if ring < 1.0 {
            c.ring(cx, cy, 44.0 + 150.0 * ring, rgba(colour, (1.0 - ring) * 0.9 * alpha), 3.0 * (1.0 - ring) + 0.5);
            c.ring(cx, cy, 30.0 + 90.0 * ring, rgba(0xffffff, (1.0 - ring) * 0.5 * alpha), 1.5);
        }
        for i in 0..40 {
            let angle = i as f32 * 2.399_963;
            let speed = 60.0 + (i * 37 % 100) as f32 * 1.7;
            let life = (since / (620.0 + (i * 53 % 500) as f32)).min(1.0);
            if life < 1.0 {
                let d = speed * (1.0 - (1.0 - life).powi(3));
                let (sx, sy) = (cx + angle.cos() * (40.0 + d), cy + angle.sin() * (40.0 + d) + 40.0 * life * life);
                let px = if i % 3 == 0 { 3.0 } else { 2.0 };
                c.rect(sx, sy, px, px, rgba(if i % 4 == 0 { 0xffffff } else { colour }, (1.0 - life) * alpha));
            }
        }
    }
    badge(c, [cx, cy + dy + sway], [size * turn.cos().abs().max(0.08), size], Look { art: &m.art, tier: m.tier, secret: m.secret, lit: true }, alpha);
    if ms >= LAND_MS {
        c.glow(cx, cy, 60.0 + 60.0 * (1.0 - lift), 60.0, rgba(0xffffff, lift * 0.75 * alpha));
    }
    // Its name.
    let line = ((ms - LAND_MS - 120.0) / 320.0).clamp(0.0, 1.0);
    if line > 0.0 {
        let a = line * alpha;
        let rise = (1.0 - line) * 10.0;
        let title = if m.secret { tr!("SECRET FOUND", "秘密を発見", "СЕКРЕТ НАЙДЕН") } else { tr!("MEDAL UNLOCKED", "メダル獲得", "МЕДАЛЬ ПОЛУЧЕНА") };
        c.text_centered(cx, 74.0 - rise, 12.0, rgba(colour, a), title);
        let name = said(&m.name);
        let size = Canvas::fit_size(18.0, 10.0, c.width - 64.0, name);
        c.text_centered(cx, 268.0 + rise, size, rgba(0xffffff, a), name);
        let about = said(&m.about);
        c.text_centered(cx, 300.0 + rise, 8.0, rgba(0xc8c8d8, a), &Canvas::fit(8.0, c.width - 64.0, about));
        c.text_centered(cx, 318.0 + rise, 7.0, rgba(colour, a), tier_name(m.tier, m.secret));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, tier: u8, secret: bool) -> MedalNote {
        MedalNote { id: id.into(), tier, secret, ..MedalNote::default() }
    }

    fn row(have: u64, need: u64, at: Option<u64>, section: u8) -> MedalRow {
        MedalRow { id: format!("{have}"), section, tier: 1, have, need, at, ..MedalRow::default() }
    }

    #[test]
    fn a_batch_shows_its_rarest_and_counts_the_rest() {
        let mut u = Unlock::default();
        let extra = u.add(vec![note("a", 1, false), note("b", 4, false), note("c", 2, true), note("d", 3, false), note("e", 1, false)]);
        assert_eq!(extra, 2);
        let order: Vec<&str> = u.queue.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(order, ["c", "b", "d"], "secrets first, then by tier");
    }

    #[test]
    fn nothing_starts_while_a_song_plays_and_it_can_wait() {
        let mut u = Unlock::default();
        u.add(vec![note("a", 1, false)]);
        assert!(!u.tick(false));
        assert!(!u.showing());
        assert!(u.tick(true), "starts as soon as it may, and says so for the sound");
        assert!(u.showing());
        assert!(!u.tick(true), "only once");
    }

    #[test]
    fn it_cant_be_skipped_by_a_key_still_held_from_the_play() {
        let mut u = Unlock::default();
        u.add(vec![note("a", 1, false)]);
        u.tick(true);
        u.skip();
        assert!(u.current.as_ref().unwrap().leaving.is_none());
    }

    #[test]
    fn goals_are_what_is_closest_and_already_started() {
        let view = MedalsView { medals: vec![row(1, 10, None, 0), row(9, 10, None, 0), row(0, 10, None, 0), row(9, 10, Some(1), 0), row(5, 0, None, 0)], ..MedalsView::default() };
        let goals = next_up(&view);
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].have, 9);
    }

    #[test]
    fn a_section_lists_earned_medals_first() {
        let view = MedalsView { medals: vec![row(1, 1, None, 2), row(2, 1, Some(5), 2), row(3, 1, None, 3)], ..MedalsView::default() };
        let rows = of_section(&view, 2);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].at.is_some());
    }

    #[test]
    fn the_landing_overshoots_and_settles() {
        assert!(ease_back(0.0).abs() < 1e-6);
        assert!((ease_back(1.0) - 1.0).abs() < 1e-6);
        assert!(ease_back(0.8) > 1.0);
    }

    #[test]
    fn the_collection_shows_every_mod_it_can_hold() {
        let n: usize = crate::mods::GROUPS.iter().map(|g| g.2.iter().filter(|m| !matches!(m, crate::mods::Mod::Autoplay | crate::mods::Mod::Relax)).count()).sum();
        assert_eq!(n, 74);
    }
}
