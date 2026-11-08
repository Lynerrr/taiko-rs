//! Medals in the game: the screen listing them, and the moment one is earned. The list and the
//! pictures come from the server, which decides who has what.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use winit::keyboard::KeyCode;

use crate::hit::{Canvas, Hit};
use crate::online::{
    self, BADGE, MedalNote, MedalRow, MedalsView, medal_badge, medal_colour, said,
};
use crate::render::{VIRTUAL_HEIGHT, rgba};
use crate::splash::hash;

/// Rows of medals on a page, and how tall each is.
fn row_h() -> f32 {
    crate::mobile::pick(44.0, 62.0)
}
/// What the mod collection takes above the medals of its tab.
const TILES_H: f32 = 118.0;
const TILE_W: f32 = 20.0;
const TILE_H: f32 = 12.0;
/// Goals on the overview.
const NEXT_UP: usize = 6;
/// The medals shown one by one when several come at once; the rest get a line.
pub const SHOWN_AT_ONCE: usize = 3;

/// How long the medal takes to land (its sound hits then) and how long it takes to switch off (ms).
const LAND_MS: f32 = crate::medal_sound::LAND * 1000.0;
const LEAVE_MS: f32 = 320.0;
/// The logo's glitch colours, for tears, and the brighter ones the splash splits the picture into.
pub(crate) const TEAR_RED: u32 = 0x9f2137;
pub(crate) const TEAR_CYAN: u32 = 0x2a9fb5;
pub(crate) const SPLIT_RED: u32 = 0xff2a4a;
pub(crate) const SPLIT_CYAN: u32 = 0x2ad4ff;
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
    for (y, row) in medal_badge(look.art, look.tier, look.secret, look.lit)
        .iter()
        .enumerate()
    {
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
            c.rect(
                cx + (start as f32 - half) * sx,
                cy + (y as f32 - half) * sy,
                (x - start) as f32 * sx,
                sy,
                rgba(colour, alpha),
            );
        }
    }
}

/// A medal small enough for a row of them.
pub fn small_badge(c: &mut Canvas, cx: f32, cy: f32, m: &MedalNote) {
    badge(
        c,
        [cx, cy],
        [0.75, 0.75],
        Look {
            art: &m.art,
            tier: m.tier,
            secret: m.secret,
            lit: true,
        },
        1.0,
    );
}

/// A medal nobody has found: an empty ring and a question mark.
fn mystery(c: &mut Canvas, cx: f32, cy: f32, scale: f32) {
    badge(
        c,
        [cx, cy],
        [scale, scale],
        Look {
            art: &[],
            tier: 1,
            secret: true,
            lit: false,
        },
        1.0,
    );
    c.text_centered(
        cx,
        cy - 7.0 * scale / 1.5,
        10.0 * scale / 1.5,
        rgba(0x6f6f86, 1.0),
        "?",
    );
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
    let mut goals: Vec<&MedalRow> = view
        .medals
        .iter()
        .filter(|m| m.at.is_none() && m.need > 0 && m.have > 0)
        .collect();
    goals.sort_by(|a, b| {
        progress(b)
            .total_cmp(&progress(a))
            .then(a.tier.cmp(&b.tier))
    });
    goals.truncate(NEXT_UP);
    goals
}

/// The medals of a section, earned first.
fn of_section(view: &MedalsView, section: usize) -> Vec<&MedalRow> {
    let mut rows: Vec<&MedalRow> = view
        .medals
        .iter()
        .filter(|m| usize::from(m.section) == section)
        .collect();
    rows.sort_by_key(|m| m.at.is_none());
    rows
}

impl View {
    /// Asks the server for the medals (on a background thread).
    pub fn open(server: String, name: Option<String>) -> View {
        let (tx, rx) = mpsc::channel();
        let who = name.clone().unwrap_or_default();
        std::thread::spawn(move || {
            let result = online::call(&server, online::Request::Medals { name: who }).and_then(
                |r| match r {
                    online::Reply::Medals(m) => Ok(*m),
                    _ => Err(crate::lang::odd_reply()),
                },
            );
            let no_medals = || {
                tr!(
                    "this server doesn't have medals yet",
                    "このサーバーにはまだメダルがありません",
                    "на этом сервере ещё нет медалей"
                )
                .to_string()
            };
            let _ = tx.send(result.map_err(|e| if e == "bad request" { no_medals() } else { e }));
        });
        View {
            name,
            state: State::Loading(rx),
            tab: 0,
            scroll: 0,
        }
    }

    pub fn poll(&mut self) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(v)) => self.state = State::Loaded(Box::new(v)),
                Ok(Err(e)) => self.state = State::Failed(e),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.state = State::Failed(
                        tr!(
                            "lost the connection",
                            "接続が切れました",
                            "соединение потеряно"
                        )
                        .into(),
                    )
                }
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
        self.scroll = self
            .scroll
            .saturating_add_signed(-notches as isize)
            .min(self.rows().saturating_sub(1));
    }

    /// The rows on this tab that can scroll.
    fn rows(&self) -> usize {
        let State::Loaded(v) = &self.state else {
            return 0;
        };
        if self.tab == 0 {
            return 0;
        }
        of_section(v, self.tab - 1).len().div_ceil(2)
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.72));
        let phone = crate::mobile::active();
        let (px, py, pw, ph) = if phone {
            (0.0, 0.0, w, 480.0)
        } else {
            (16.0, 20.0, w - 32.0, 444.0)
        };
        // The tabs under the title.
        let (tab_y, tab_h) = if phone {
            (py + 46.0, 36.0)
        } else {
            (py + 32.0, 14.0)
        };
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.98),
            rgba(0xFFD34D, 0.8),
            1.5,
        );
        let (by, bw, bh) = if phone {
            (py + 6.0, 96.0, 36.0)
        } else {
            (py + ph - 24.0, 56.0, 16.0)
        };
        c.button(
            px + pw - bw - 12.0,
            by,
            bw,
            bh,
            tr!("Back", "戻る", "Назад"),
            crate::mobile::pick(7.0, 12.0),
            Hit::Key(KeyCode::Escape),
            true,
        );
        let title = tr!("MEDALS", "メダル", "МЕДАЛИ");
        crate::style::title(c, px + 16.0, py + 12.0, 14.0, 0xffffff, title);
        let v = match &self.state {
            State::Loading(_) => {
                c.text_centered(
                    px + pw / 2.0,
                    py + ph / 2.0,
                    9.0,
                    rgba(0xc8c8d8, 1.0),
                    tr!(
                        "loading the medals...",
                        "メダルを読み込み中...",
                        "загрузка медалей..."
                    ),
                );
                return;
            }
            State::Failed(e) => {
                c.text_centered(
                    px + pw / 2.0,
                    py + ph / 2.0,
                    9.0,
                    rgba(0xFF4B4B, 1.0),
                    &Canvas::fit(9.0, pw - 32.0, e),
                );
                return;
            }
            State::Loaded(v) => v,
        };
        let (earned, total) = (v.earned, v.medals.len());
        let stats = match &self.name {
            Some(name) => {
                tr!("{name}: {earned} of {total}", "{name}: {total}個中{earned}個", "{name}: {earned} из {total}";)
            }
            None => tr!(
                "Log in to earn them",
                "ログインで獲得できます",
                "Войди, чтобы их получать"
            )
            .to_string(),
        };
        let title_w = Canvas::text_width(14.0, title);
        c.text_right(
            px + pw - 16.0 - if phone { bw + 12.0 } else { 0.0 },
            py + 16.0,
            8.0,
            rgba(0xFFD34D, 1.0),
            &Canvas::fit(
                8.0,
                pw - title_w - 56.0 - if phone { bw + 12.0 } else { 0.0 },
                &stats,
            ),
        );
        // The tabs.
        let tab_w = (pw - 24.0) / TABS as f32;
        for tab in 0..TABS {
            let x = px + 12.0 + tab as f32 * tab_w;
            let on = tab == self.tab;
            let name = tab_name(tab);
            let hover = c.hovered(x, tab_y, tab_w, tab_h);
            let fill = if hover { 0x20202b } else { 0x1a1a24 };
            c.round_rect(
                [x + 1.0, tab_y],
                [tab_w - 2.0, tab_h],
                3.0,
                rgba(fill, 1.0),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            if on {
                // The tab open: a gold pill, the selector sliding to it.
                crate::style::pill(c, x + 1.0, tab_y, tab_w - 2.0, tab_h, 0xC89A2A);
                crate::style::selector(
                    c,
                    "medals",
                    [x - 2.0, tab_y - 3.0, tab_w + 4.0, tab_h + 6.0],
                );
            }
            c.text_centered(
                x + tab_w / 2.0,
                tab_y + (tab_h - 6.0) / 2.0,
                6.0,
                rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0),
                &Canvas::fit(6.0, tab_w - 6.0, name),
            );
            c.hit(x, tab_y, tab_w, tab_h, Hit::MedalTab(tab));
        }
        let (top, bottom) = if phone {
            (tab_y + tab_h + 8.0, py + ph - 6.0)
        } else {
            (py + 52.0, by - 4.0)
        };
        let start = c.shapes.len();
        if self.tab == 0 {
            overview(c, v, px + 16.0, top, pw - 32.0, bottom);
        } else {
            let section = self.tab - 1;
            let mut y = top - self.scroll as f32 * row_h();
            if section == 1 {
                tiles(c, v, px + 16.0, y, pw - 32.0);
                y += TILES_H;
            }
            let rows = of_section(v, section);
            let col_w = (pw - 40.0) / 2.0;
            for (i, m) in rows.iter().enumerate() {
                let row_y = y + (i / 2) as f32 * row_h();
                if row_y + row_h() < top || row_y > bottom {
                    continue;
                }
                medal_row(
                    c,
                    m,
                    v.players,
                    px + 16.0 + (i % 2) as f32 * (col_w + 8.0),
                    row_y,
                    col_w,
                );
            }
            if rows.is_empty() {
                c.text_centered(
                    px + pw / 2.0,
                    top + 40.0,
                    8.0,
                    rgba(0x6f6f86, 1.0),
                    tr!("nothing here yet", "まだ何もありません", "здесь пока пусто"),
                );
            }
        }
        c.clip_since(start, px + 2.0, top - 2.0, pw - 4.0, bottom - top + 2.0);
        c.text(
            px + 16.0,
            by + 5.0,
            6.0,
            rgba(0x55556a, 1.0),
            &Canvas::fit_hints(
                6.0,
                pw - bw - 44.0,
                tr!(
                    "Left/Right tabs  wheel scrolls  Esc back",
                    "左右でタブ  ホイールでスクロール  Escで戻る",
                    "Влево/вправо вкладки  колесо листает  Esc назад"
                ),
            ),
        );
    }
}

/// One medal: its badge, its name, what it takes and how it's going.
fn medal_row(c: &mut Canvas, m: &MedalRow, players: u32, x: f32, y: f32, w: f32) {
    let lit = m.at.is_some();
    c.round_rect(
        [x, y],
        [w, row_h() - 3.0],
        5.0,
        rgba(if lit { 0x1f1f2a } else { 0x18181f }, 1.0),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    let scale = 1.3;
    if m.art.is_empty() {
        mystery(c, x + 22.0, y + 20.5, scale);
    } else {
        badge(
            c,
            [x + 22.0, y + 20.5],
            [scale, scale],
            Look {
                art: &m.art,
                tier: m.tier,
                secret: m.secret,
                lit: lit || m.secret,
            },
            1.0,
        );
    }
    let room = w - 54.0;
    let name = m.name.as_ref().map_or("???", said);
    c.text(
        x + 44.0,
        y + crate::mobile::pick(5.0, 6.0),
        8.0,
        rgba(if lit { 0xffffff } else { 0xb4b4c8 }, 1.0),
        &Canvas::fit(8.0, room, name),
    );
    let about = m.how.as_ref().or(m.hint.as_ref()).map_or(
        tr!(
            "Nobody has found this one",
            "まだ誰も見つけていません",
            "Пока никто не нашёл"
        ),
        said,
    );
    c.text(
        x + 44.0,
        y + crate::mobile::pick(17.0, 23.0),
        6.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(6.0, room, about),
    );
    let share = if players > 0 {
        f64::from(m.holders) * 100.0 / f64::from(players)
    } else {
        0.0
    };
    let tier = tier_name(m.tier, m.secret);
    c.text(
        x + 44.0,
        y + crate::mobile::pick(28.0, 40.0),
        6.0,
        rgba(medal_colour(m.tier, m.secret), 1.0),
        tier,
    );
    let right = x + w - 8.0;
    c.text_right(
        right,
        y + crate::mobile::pick(28.0, 40.0),
        6.0,
        rgba(0x6f6f86, 1.0),
        &format!("{share:.0}%"),
    );
    if !lit && m.need > 0 {
        let left = x + 44.0 + Canvas::text_width(6.0, tier) + 8.0;
        let bar = (right - 30.0 - left).max(0.0);
        let bar_y = y + crate::mobile::pick(30.0, 46.0);
        c.rect(left, bar_y, bar, 3.0, rgba(0x2a2a38, 1.0));
        c.rect(
            left,
            bar_y,
            bar * progress(m),
            3.0,
            rgba(medal_colour(m.tier, false), 1.0),
        );
    }
}

/// The overview: how many, what's next, and the latest ones.
fn overview(c: &mut Canvas, v: &MedalsView, x: f32, top: f32, w: f32, bottom: f32) {
    let col_w = (w - 8.0) / 2.0;
    c.text(
        x,
        top,
        7.0,
        rgba(0xFFD34D, 1.0),
        tr!("CLOSEST", "もうすぐ", "БЛИЖЕ ВСЕГО"),
    );
    let goals = next_up(v);
    if goals.is_empty() {
        c.text(
            x,
            top + 14.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            tr!(
                "Play a few maps to see your goals",
                "プレイすると目標が表示されます",
                "Сыграй пару карт, и тут появятся цели"
            ),
        );
    }
    for (i, m) in goals.iter().enumerate() {
        let y = top + 12.0 + i as f32 * row_h();
        if y + row_h() <= bottom {
            medal_row(c, m, v.players, x, y, col_w);
        }
    }
    let right = x + col_w + 8.0;
    c.text(
        right,
        top,
        7.0,
        rgba(0xFFD34D, 1.0),
        tr!("LATEST", "最新", "ПОСЛЕДНИЕ"),
    );
    let mut latest: Vec<&MedalRow> = v.medals.iter().filter(|m| m.at.is_some()).collect();
    latest.sort_by_key(|m| std::cmp::Reverse(m.at));
    if latest.is_empty() {
        c.text(
            right,
            top + 14.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            tr!("None yet", "まだありません", "Пока нет"),
        );
    }
    for (i, m) in latest.iter().take(NEXT_UP).enumerate() {
        let y = top + 12.0 + i as f32 * row_h();
        if y + row_h() <= bottom {
            medal_row(c, m, v.players, right, y, col_w);
        }
    }
}

/// Whether a mod has a tile in the collection: the server's list (`medal_list::GROUPS`), which
/// leaves out the ones that play for you, Maze, Tact (newer than its medals) and OD Override
/// (its plays aren't saved).
fn in_collection(m: &crate::mods::Mod) -> bool {
    !matches!(
        m,
        crate::mods::Mod::Autoplay
            | crate::mods::Mod::Relax
            | crate::mods::Mod::Maze
            | crate::mods::Mod::Tact
            | crate::mods::Mod::OdOverride
    )
}

/// The mod collection: a tile per mod, by mod select column, lit once it's in.
fn tiles(c: &mut Canvas, v: &MedalsView, x: f32, top: f32, w: f32) {
    let have = v.collection.len();
    let size = v.collection_size;
    c.text(x, top, 7.0, rgba(0xFFD34D, 1.0), &tr!("COLLECTION  {have} of {size}", "コレクション  {size}個中{have}個", "КОЛЛЕКЦИЯ  {have} из {size}";));
    c.text_right(
        x + w,
        top,
        6.0,
        rgba(0x6f6f86, 1.0),
        &Canvas::fit(
            6.0,
            w * 0.6,
            tr!(
                "3★+, 80%+, up to 3 mods, no NF",
                "★3以上・精度80%以上・3モードまで・NFなし",
                "3★+, 80%+, до 3 модов, без NF"
            ),
        ),
    );
    // Autoplay and Relax never count; Tact isn't in the server's collection (yet).
    for (row, (_, colour, mods)) in crate::mods::GROUPS.iter().enumerate() {
        for (col, m) in mods.iter().filter(|m| in_collection(m)).enumerate() {
            let (tx, ty) = (
                x + col as f32 * (TILE_W + 2.0),
                top + 12.0 + row as f32 * (TILE_H + 1.0),
            );
            let acronym = m.acronym();
            let lit = v.collection.iter().any(|a| a == acronym);
            let (fill, ink) = if lit {
                (*colour, 0x0b0b10)
            } else {
                (0x20202b, 0x55556a)
            };
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

    /// All of `medals`, in this order: for looking at them with `TAIKO_UNLOCK`.
    pub fn preview(&mut self, medals: Vec<MedalNote>) {
        self.queue.extend(medals);
    }

    /// Moves on as time passes: `allowed` is false while a song is playing. The tier and whether
    /// it's secret of a medal that has just started, for its sound.
    pub fn tick(&mut self, allowed: bool) -> Option<(u8, bool)> {
        if let Some(s) = &self.current {
            let ms = s.since.elapsed().as_secs_f32() * 1000.0;
            let gone = s
                .leaving
                .is_some_and(|l| l.elapsed().as_secs_f32() * 1000.0 >= LEAVE_MS);
            if gone {
                self.current = None;
            } else if ms >= Force::of(&s.medal).stay() && s.leaving.is_none() {
                self.skip();
            }
        }
        if self.current.is_none()
            && allowed
            && let Some(medal) = self.queue.pop_front()
        {
            let sound = (medal.tier, medal.secret);
            self.current = Some(Showing {
                medal,
                since: Instant::now(),
                leaving: None,
            });
            return Some(sound);
        }
        None
    }

    /// A key or a click: on to the next one, once it's had its moment.
    pub fn skip(&mut self) {
        if let Some(s) = &mut self.current
            && s.leaving.is_none()
            && s.since.elapsed().as_secs_f32() * 1000.0 >= SKIP_AFTER_MS
        {
            s.leaving = Some(Instant::now());
        }
    }

    pub fn draw(&self, c: &mut Canvas) {
        let Some(s) = &self.current else { return };
        let ms = s.since.elapsed().as_secs_f32() * 1000.0;
        let leave = s.leaving.map_or(0.0, |l| {
            (l.elapsed().as_secs_f32() * 1000.0 / LEAVE_MS).min(1.0)
        });
        draw_unlock(c, &s.medal, ms, leave);
    }
}

/// How hard a medal's scene goes off: the rarer, the more of it falls apart.
#[derive(Clone, Copy)]
struct Force {
    /// How much glitch, 0 to 1.
    glitch: f32,
    /// A secret: it doesn't arrive, it comes through.
    ura: bool,
}

impl Force {
    fn of(m: &MedalNote) -> Force {
        let glitch = [0.12, 0.32, 0.6, 1.0][usize::from(m.tier.clamp(1, 4)) - 1];
        Force {
            glitch: if m.secret {
                f32::max(glitch, 0.75)
            } else {
                glitch
            },
            ura: m.secret,
        }
    }

    /// How long the medal stays up, in ms.
    fn stay(self) -> f32 {
        if self.ura {
            5000.0
        } else {
            2600.0 + self.glitch * 2200.0
        }
    }

    /// A short burst of the glitch coming back while the medal sits there: 1 during one. Never
    /// for the commonest medals.
    fn aftershock(self, ms: f32) -> f32 {
        const EVERY: f32 = 640.0;
        if ms < LAND_MS + 500.0 || self.glitch < 0.3 {
            return 0.0;
        }
        let n = (ms / EVERY) as u32;
        let start = n as f32 * EVERY + hash(n, 0, 22) * EVERY * 0.6;
        let long = 50.0 + 130.0 * self.glitch;
        if hash(n, 0, 21) < self.glitch * 0.85 && (start..start + long).contains(&ms) {
            1.0
        } else {
            0.0
        }
    }
}

/// `t` eased out.
fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A badge torn apart: `row` says how far each row is thrown (`None`: not there at all),
/// `split` throws red and cyan copies out from under it, and `ink` draws it all in one colour.
struct Torn<'a> {
    at: [f32; 2],
    scale: f32,
    look: Look<'a>,
    row: &'a dyn Fn(usize) -> Option<f32>,
    split: f32,
    ink: Option<u32>,
    alpha: f32,
}

fn torn_badge(c: &mut Canvas, t: Torn) {
    let half = BADGE as f32 / 2.0;
    let [cx, cy] = t.at;
    let rows = medal_badge(t.look.art, t.look.tier, t.look.secret, t.look.lit);
    let run = |row: &[Option<u32>; BADGE], from: usize, same: &dyn Fn(Option<u32>) -> bool| {
        let mut x = from;
        while x < BADGE && same(row[x]) {
            x += 1;
        }
        x
    };
    // The split copies go underneath, so they only show around the edges.
    if t.split > 0.5 {
        for (ink, by) in [(SPLIT_RED, -t.split), (SPLIT_CYAN, t.split)] {
            for (y, row) in rows.iter().enumerate() {
                let Some(shift) = (t.row)(y) else { continue };
                let mut x = 0;
                while x < BADGE {
                    if row[x].is_none() {
                        x += 1;
                        continue;
                    }
                    let end = run(row, x, &|p| p.is_some());
                    let left = cx + (x as f32 - half) * t.scale + shift + by;
                    c.rect(
                        left,
                        cy + (y as f32 - half) * t.scale,
                        (end - x) as f32 * t.scale,
                        t.scale,
                        rgba(ink, 0.75 * t.alpha),
                    );
                    x = end;
                }
            }
        }
    }
    for (y, row) in rows.iter().enumerate() {
        let Some(shift) = (t.row)(y) else { continue };
        let mut x = 0;
        while x < BADGE {
            let Some(colour) = row[x] else {
                x += 1;
                continue;
            };
            let end = run(row, x, &|p| p == Some(colour));
            let left = cx + (x as f32 - half) * t.scale + shift;
            c.rect(
                left,
                cy + (y as f32 - half) * t.scale,
                (end - x) as f32 * t.scale,
                t.scale,
                rgba(t.ink.unwrap_or(colour), t.alpha),
            );
            x = end;
        }
    }
}

/// `text` still coming through: past the part that's arrived, junk that changes every 40 ms.
fn decoding(text: &str, arrived: f32, bucket: u32) -> String {
    const JUNK: &[u8] = b"#%&*@$!?/<>[]{}01";
    let chars: Vec<char> = text.chars().collect();
    let shown = (arrived.clamp(0.0, 1.0) * chars.len() as f32).ceil() as usize;
    chars
        .iter()
        .enumerate()
        .map(|(i, &ch)| {
            if i < shown || ch == ' ' {
                ch
            } else {
                JUNK[(hash(i as u32, bucket, 41) * JUNK.len() as f32) as usize % JUNK.len()] as char
            }
        })
        .collect()
}

/// How a line stamps in: how far along it is, and how far its colours slip and it shakes.
struct Stamp {
    slam: f32,
    slip: f32,
    jitter: [f32; 2],
}

/// A line stamping in, the way the splash's title does: slammed down from huge, then shaking,
/// with its colours slipping apart while the glitch is on.
fn stamp(c: &mut Canvas, [cx, y]: [f32; 2], size: f32, colour: u32, text: &str, how: Stamp) {
    let Stamp { slam, slip, jitter } = how;
    let k = 1.0 + (1.0 - ease(slam)) * 2.2;
    let size = size * k;
    let a = (slam * 3.0).min(1.0);
    let top = y - (size - size / k) / 2.0;
    let (x, y) = (cx + jitter[0], top + jitter[1]);
    c.text_centered(x + 2.0, y + 2.0, size, rgba(0x000000, 0.7 * a), text);
    if slip > 0.3 {
        c.text_centered(x - slip, y, size, rgba(SPLIT_RED, 0.8 * a), text);
        c.text_centered(x + slip, y, size, rgba(SPLIT_CYAN, 0.8 * a), text);
    }
    c.text_centered(x, y, size, rgba(colour, a), text);
}

/// The moment itself, in the game's own language: time moves in 40 ms steps, a lane opens like a
/// CRT and the medal rides it in like a note; it hits with an impact frame, tears run across the
/// screen in the logo's colours, the name stamps in, and it all switches off to a line. How much
/// of that happens goes with the medal's rarity. A secret doesn't ride in: the screen loses the
/// signal and the medal comes through the static. `ms`: since it began; `leave`: 0, then to 1
/// as it switches off. A pure function of the time, so it can be tested and looked at frame by
/// frame.
pub fn draw_unlock(c: &mut Canvas, m: &MedalNote, ms: f32, leave: f32) {
    const LANE_H: f32 = 124.0;
    const CY: f32 = 170.0;
    const SIZE: f32 = 4.0;
    let force = Force::of(m);
    let g = force.glitch;
    let colour = medal_colour(m.tier, m.secret);
    let start = c.shapes.len();
    let (w, cx) = (c.width, c.width / 2.0);
    let bucket = (ms / 40.0) as u32;
    let since = ms - LAND_MS;
    let landed = since >= 0.0;
    // What drives everything after the hit: it dies away, and comes back in bursts.
    let shock = if landed {
        (-since / (150.0 + 320.0 * g)).exp()
    } else {
        0.0
    };
    let heat = shock.max(force.aftershock(ms) * 0.7);
    let look = || Look {
        art: &m.art,
        tier: m.tier,
        secret: m.secret,
        lit: true,
    };

    let dark = if force.ura { 1.0 } else { 0.8 + 0.16 * g };
    c.rect(
        -24.0,
        -24.0,
        w + 48.0,
        VIRTUAL_HEIGHT + 48.0,
        rgba(0x000000, ((bucket + 1) as f32 * 0.25).min(1.0) * dark),
    );

    // No signal: a secret's way in.
    if force.ura {
        let thin = if landed { 0.1 + 0.4 * heat } else { 1.0 };
        for i in 0..(420.0 * thin) as u32 {
            let (nx, ny) = (hash(i, bucket, 7), hash(i, bucket, 8));
            let v = 0.2 + hash(i, bucket, 9) * 0.5;
            c.rect(
                nx * w,
                ny * VIRTUAL_HEIGHT,
                3.0,
                3.0,
                [v, v * 0.9, v * 1.1, 0.55],
            );
        }
    }

    // The lane opens like a CRT coming on, and the hit circle is waiting on it.
    let opened = ease(ms / 150.0);
    let lane_h = 2.0 + (LANE_H - 2.0) * opened;
    let lane_y = CY - lane_h / 2.0;
    c.rect(-24.0, lane_y, w + 48.0, lane_h, rgba(0x0b0b10, 0.92));
    c.rect(-24.0, lane_y - 2.0, w + 48.0, 2.0, rgba(colour, 0.9));
    c.rect(-24.0, lane_y + lane_h, w + 48.0, 2.0, rgba(colour, 0.9));
    if opened >= 1.0 && !landed {
        c.ring(cx, CY, 54.0, rgba(0xffffff, 0.55), 2.0);
        // Speed lines: more and faster the rarer it is.
        for i in 0..(6.0 + 26.0 * g) as u32 {
            let speed = 1.1 + hash(i, 0, 33) * 1.6;
            let x = w - (ms * speed + hash(i, 0, 34) * w) % (w + 80.0);
            let y = lane_y + 6.0 + hash(i, 0, 35) * (lane_h - 12.0);
            c.rect(
                x,
                y,
                20.0 + 50.0 * hash(i, 0, 36),
                1.5,
                rgba(if i % 3 == 0 { colour } else { 0xffffff }, 0.3),
            );
        }
    }

    // Where the medal is.
    let flight = (ms / LAND_MS).min(1.0);
    let throw = |y: usize| -> Option<f32> {
        // Rows thrown sideways in bands, a new pattern every step, the way the logo slips.
        let band = y as u32 / 2;
        let thrown = hash(band, bucket, 5) > 0.72 - 0.34 * g;
        Some(if heat > 0.04 && thrown {
            (hash(band, bucket, 6) - 0.5) * (14.0 + 110.0 * g) * heat
        } else {
            0.0
        })
    };
    let coming_through = |y: usize| -> Option<f32> {
        // A secret arrives a row at a time, flickering.
        let there = hash(y as u32, 0, 3) < flight * 1.15
            && hash(y as u32, bucket, 4) < 0.35 + 0.65 * flight;
        there.then(|| (hash(y as u32, bucket, 6) - 0.5) * 60.0 * (1.0 - flight))
    };
    let row: &dyn Fn(usize) -> Option<f32> = if force.ura && !landed {
        &coming_through
    } else {
        &throw
    };
    let x = if force.ura || landed {
        cx
    } else {
        cx + (w / 2.0 + 90.0) * (1.0 - flight).powf(1.6)
    };

    // The trail behind it, in the logo's colours.
    if !landed && !force.ura {
        let split = |ink: u32, back: f32, alpha: f32, c: &mut Canvas| {
            torn_badge(
                c,
                Torn {
                    at: [x + back, CY],
                    scale: SIZE,
                    look: look(),
                    row: &|_| Some(0.0),
                    split: 0.0,
                    ink: Some(ink),
                    alpha,
                },
            );
        };
        let speed = 26.0 * (flight * 2.0).min(1.0);
        split(TEAR_RED, speed * 2.0, 0.3, c);
        split(TEAR_CYAN, speed, 0.5, c);
    }

    // The hit: frames of white with the medal cut out of it, one more of them the rarer it is.
    let frames = 1 + (g * 2.5) as u32;
    let impact = landed
        && (0..frames)
            .any(|k| (k as f32 * 110.0..k as f32 * 110.0 + 30.0 + 40.0 * g).contains(&since));
    if impact {
        let flash = if force.ura { colour } else { 0xffffff };
        c.rect(
            -24.0,
            -24.0,
            w + 48.0,
            VIRTUAL_HEIGHT + 48.0,
            rgba(flash, 0.4 + 0.55 * g),
        );
    }

    // Rings going out in steps.
    if landed {
        let step = (since / 40.0).floor() * 40.0;
        let ring = (step / (340.0 + 260.0 * g)).min(1.0);
        if ring < 1.0 {
            c.ring(
                cx,
                CY,
                54.0 + (90.0 + 170.0 * g) * ring,
                rgba(colour, (1.0 - ring) * 0.95),
                4.0 * (1.0 - ring) + 1.0,
            );
            c.ring(
                cx,
                CY,
                40.0 + (50.0 + 120.0 * g) * ring,
                rgba(0xffffff, (1.0 - ring) * 0.6),
                2.0,
            );
        }
        // Pixels flying off the hit.
        for i in 0..(12.0 + 60.0 * g) as u32 {
            let angle = i as f32 * 2.399_963;
            let life = (step / (420.0 + 400.0 * hash(i, 0, 51))).min(1.0);
            if life < 1.0 {
                let d = 56.0 + (70.0 + 200.0 * hash(i, 0, 52)) * ease(life);
                let size = if i % 3 == 0 { 4.0 } else { 3.0 };
                let ink = [colour, SPLIT_RED, SPLIT_CYAN, 0xffffff][i as usize % 4];
                c.rect(
                    ((cx + angle.cos() * d) / 3.0).round() * 3.0,
                    ((CY + angle.sin() * d * 0.7) / 3.0).round() * 3.0,
                    size,
                    size,
                    rgba(ink, 1.0 - life),
                );
            }
        }
    }

    // The medal, and on the hit a shove towards you.
    let scale = SIZE * (1.0 + 0.3 * shock);
    let ink = impact.then_some(0x000000);
    let split = heat * (1.5 + 11.0 * g);
    torn_badge(
        c,
        Torn {
            at: [x, CY],
            scale,
            look: look(),
            row,
            split,
            ink,
            alpha: 1.0,
        },
    );

    // Noise over it.
    if landed && g > 0.25 {
        for i in 0..(260.0 * g * heat) as u32 {
            let (nx, ny) = (hash(i, bucket, 7), hash(i, bucket, 8));
            let v = 0.5 + hash(i, bucket, 9) * 0.5;
            c.rect(
                cx - 120.0 + nx * 240.0,
                CY - 90.0 + ny * 180.0,
                3.0,
                3.0,
                [v, v, v, 0.75],
            );
        }
    }

    // Text.
    if since > 40.0 {
        let split = 1.0 + heat * (1.0 + 7.0 * g);
        let shake = |salt: u32| (hash(salt, bucket, 13) - 0.5) * 2.0 * heat * (1.0 + 7.0 * g);
        let jitter = [shake(1), shake(2)];
        let title = if m.secret {
            tr!("SECRET FOUND", "秘密を発見", "СЕКРЕТ НАЙДЕН")
        } else {
            tr!("MEDAL UNLOCKED", "メダル獲得", "МЕДАЛЬ ПОЛУЧЕНА")
        };
        stamp(
            c,
            [cx, 72.0],
            12.0,
            colour,
            title,
            Stamp {
                slam: (since - 40.0) / 150.0,
                slip: split,
                jitter,
            },
        );
        let decode_ms = 80.0 + 620.0 * g;
        let name = said(&m.name);
        let name_at = 150.0;
        if since > name_at {
            let size = Canvas::fit_size(20.0, 10.0, w - 64.0, name);
            let text = if g >= 0.3 {
                decoding(name, (since - name_at) / decode_ms, bucket)
            } else {
                name.to_string()
            };
            stamp(
                c,
                [cx, 252.0],
                size,
                0xffffff,
                &text,
                Stamp {
                    slam: (since - name_at) / 170.0,
                    slip: split,
                    jitter,
                },
            );
        }
        let about_at = 330.0;
        if since > about_at {
            let about = said(&m.about);
            let text = if g >= 0.6 {
                decoding(about, (since - about_at) / (decode_ms * 1.3), bucket)
            } else {
                about.to_string()
            };
            let a = ((since - about_at) / 120.0).min(1.0);
            c.text_centered(
                cx,
                288.0,
                8.0,
                rgba(0xc8c8d8, a),
                &Canvas::fit(8.0, w - 64.0, &text),
            );
            c.text_centered(cx, 306.0, 7.0, rgba(colour, a), tier_name(m.tier, m.secret));
        }
    }

    // Tears across the whole screen, a new set every step.
    let tears = (heat * (3.0 + 44.0 * g)) as u32;
    for i in 0..tears {
        let (y, from) = (
            hash(i, bucket, 61) * VIRTUAL_HEIGHT,
            hash(i, bucket, 62) * w * 0.7,
        );
        let long = (0.3 + 0.7 * hash(i, bucket, 63)) * w;
        let thick = 2.0 + (hash(i, bucket, 64) * hash(i, bucket, 64) * 14.0 * g).floor();
        c.rect(
            from,
            y,
            long,
            thick,
            rgba(if i % 2 == 0 { TEAR_RED } else { TEAR_CYAN }, 0.9),
        );
    }
    // Blocks of the picture gone wrong, for the rarest.
    if g >= 0.6 {
        for i in 0..(heat * 26.0 * g) as u32 {
            let (bx, by) = (
                hash(i, bucket, 71) * w,
                hash(i, bucket, 72) * VIRTUAL_HEIGHT,
            );
            let (bw, bh) = (
                30.0 + hash(i, bucket, 73) * 170.0,
                6.0 + hash(i, bucket, 74) * 30.0,
            );
            let ink = [colour, 0x000000, 0xffffff, TEAR_RED][i as usize % 4];
            c.rect(bx, by, bw, bh, rgba(ink, 0.5 + 0.4 * hash(i, bucket, 75)));
        }
    }
    // The whole screen flickers.
    if heat > 0.2 && g >= 0.3 && hash(bucket, 0, 12) > 0.55 {
        c.rect(
            -24.0,
            -24.0,
            w + 48.0,
            VIRTUAL_HEIGHT + 48.0,
            rgba(colour, 0.16 * g * heat),
        );
    }

    // And shakes, in steps too.
    let shake = heat * (0.5 + 16.0 * g);
    if shake > 0.1 {
        c.transform_since(
            start,
            [0.0, 0.0],
            1.0,
            (hash(bucket, 0, 31) - 0.5) * 2.0 * shake,
            (hash(bucket, 0, 32) - 0.5) * 2.0 * shake,
        );
    }

    // Switching off, like the splash: squashed to a line, the line to a dot.
    if leave > 0.0 {
        let ky = (1.0 - ease(leave / 0.55)).max(0.004);
        let kx = 1.0 - ease((leave - 0.55) / 0.35);
        let mid = VIRTUAL_HEIGHT / 2.0;
        for sh in &mut c.shapes[start..] {
            sh.center = [
                cx + (sh.center[0] - cx) * kx,
                mid + (sh.center[1] - mid) * ky,
            ];
            sh.half = [sh.half[0] * kx, sh.half[1] * ky];
        }
        c.rect(
            cx - w * kx / 2.0,
            mid - 1.5,
            w * kx,
            3.0,
            rgba(0xffffff, 0.9),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, tier: u8, secret: bool) -> MedalNote {
        MedalNote {
            id: id.into(),
            tier,
            secret,
            ..MedalNote::default()
        }
    }

    fn row(have: u64, need: u64, at: Option<u64>, section: u8) -> MedalRow {
        MedalRow {
            id: format!("{have}"),
            section,
            tier: 1,
            have,
            need,
            at,
            ..MedalRow::default()
        }
    }

    #[test]
    fn a_batch_shows_its_rarest_and_counts_the_rest() {
        let mut u = Unlock::default();
        let extra = u.add(vec![
            note("a", 1, false),
            note("b", 4, false),
            note("c", 2, true),
            note("d", 3, false),
            note("e", 1, false),
        ]);
        assert_eq!(extra, 2);
        let order: Vec<&str> = u.queue.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(order, ["c", "b", "d"], "secrets first, then by tier");
    }

    #[test]
    fn nothing_starts_while_a_song_plays_and_it_can_wait() {
        let mut u = Unlock::default();
        u.add(vec![note("a", 1, false)]);
        assert_eq!(u.tick(false), None);
        assert!(!u.showing());
        assert_eq!(
            u.tick(true),
            Some((1, false)),
            "starts as soon as it may, and says so for the sound"
        );
        assert!(u.showing());
        assert_eq!(u.tick(true), None, "only once");
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
        let view = MedalsView {
            medals: vec![
                row(1, 10, None, 0),
                row(9, 10, None, 0),
                row(0, 10, None, 0),
                row(9, 10, Some(1), 0),
                row(5, 0, None, 0),
            ],
            ..MedalsView::default()
        };
        let goals = next_up(&view);
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].have, 9);
    }

    #[test]
    fn a_section_lists_earned_medals_first() {
        let view = MedalsView {
            medals: vec![
                row(1, 1, None, 2),
                row(2, 1, Some(5), 2),
                row(3, 1, None, 3),
            ],
            ..MedalsView::default()
        };
        let rows = of_section(&view, 2);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].at.is_some());
    }

    fn shapes_at(m: &MedalNote, ms: f32) -> usize {
        let mut c = Canvas::default();
        c.clear(853.3);
        draw_unlock(&mut c, m, ms, 0.0);
        c.shapes.len()
    }

    fn medal(tier: u8, secret: bool) -> MedalNote {
        MedalNote {
            tier,
            secret,
            name: ["A name".into(), "A name".into(), "A name".into()],
            about: ["About".into(), "About".into(), "About".into()],
            art: vec!["#".repeat(16); 16],
            ..MedalNote::default()
        }
    }

    #[test]
    fn the_rarer_the_medal_the_more_of_it_falls_apart() {
        let on_the_hit = LAND_MS + 60.0;
        let counts: Vec<usize> = (1..=4)
            .map(|tier| shapes_at(&medal(tier, false), on_the_hit))
            .collect();
        assert!(counts.windows(2).all(|p| p[1] > p[0]), "{counts:?}");
    }

    #[test]
    fn the_scene_runs_from_start_to_finish_for_every_tier() {
        for m in [
            medal(1, false),
            medal(2, false),
            medal(3, false),
            medal(4, false),
            medal(1, true),
            medal(4, true),
        ] {
            for ms in (0..7000).step_by(17) {
                shapes_at(&m, ms as f32);
            }
            let mut c = Canvas::default();
            c.clear(853.3);
            draw_unlock(&mut c, &m, 3000.0, 1.0);
            assert!(
                c.shapes
                    .iter()
                    .all(|s| s.center[0].is_finite() && s.center[1].is_finite())
            );
        }
    }

    #[test]
    fn rarer_medals_stay_longer_and_only_they_flare_up_again() {
        let force = |tier| Force::of(&medal(tier, false));
        assert!(force(4).stay() > force(1).stay());
        assert!(
            (0..60_000).all(|ms| force(1).aftershock(ms as f32) == 0.0),
            "the commonest medals stay calm"
        );
        assert!((0..60_000).any(|ms| force(4).aftershock(ms as f32) > 0.0));
        assert!(
            Force::of(&medal(1, true)).glitch >= 0.75,
            "a secret is never a quiet one"
        );
    }

    #[test]
    fn names_come_through_a_letter_at_a_time_and_keep_their_length() {
        assert_eq!(decoding("Name here", 1.0, 3), "Name here");
        let half = decoding("Name here", 0.45, 3);
        assert!(half.starts_with("Name"));
        assert_ne!(half, "Name here");
        assert_eq!(half.chars().count(), 9);
        assert_eq!(half.chars().nth(4), Some(' '), "words keep their shape");
    }

    #[test]
    fn the_collection_shows_every_mod_it_can_hold() {
        let n: usize = crate::mods::GROUPS
            .iter()
            .map(|g| g.2.iter().filter(|m| in_collection(m)).count())
            .sum();
        assert_eq!(n, 74);
    }
}
