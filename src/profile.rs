//! A player's profile from the online server: rank, pp, their best and latest plays.

use std::sync::mpsc::{self, Receiver};

use winit::keyboard::KeyCode;

use crate::online::{self, OnlineScore, Profile};
use crate::render::{accent, rgba, Canvas, Hit};

pub enum State {
    Loading(Receiver<Result<Profile, String>>),
    Loaded(Box<Profile>),
    Failed(String),
}

pub struct View {
    pub name: String,
    pub state: State,
}

impl View {
    /// Asks the server for `name`'s profile (on a background thread).
    pub fn open(server: String, name: String) -> View {
        let (tx, rx) = mpsc::channel();
        let who = name.clone();
        std::thread::spawn(move || {
            let result = online::call(&server, online::Request::Profile { name: who }).and_then(|r| match r {
                online::Reply::Profile(p) => Ok(*p),
                _ => Err(crate::lang::odd_reply()),
            });
            // Servers from before profiles don't know the request.
            let no_profiles = || tr!("the server doesn't have profiles yet", "このサーバーにはまだプロフィールがありません", "на сервере ещё нет профилей").to_string();
            let _ = tx.send(result.map_err(|e| if e == "bad request" { no_profiles() } else { e }));
        });
        View { name, state: State::Loading(rx) }
    }

    pub fn poll(&mut self) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(p)) => self.state = State::Loaded(Box::new(p)),
                Ok(Err(e)) => self.state = State::Failed(e),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.state = State::Failed(tr!("lost the connection", "接続が切れました", "соединение потеряно").into()),
            }
        }
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
        let (px, py, pw, ph) = (16.0, 44.0, w - 32.0, 410.0);
        c.round_rect(px, py, pw, ph, 10.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
        let (by, bw) = (py + ph - 26.0, 56.0);
        c.button(px + pw - bw - 12.0, by, bw, 16.0, tr!("Back", "戻る", "Назад"), 7.0, Hit::Key(KeyCode::Escape), true);
        let p = match &self.state {
            State::Loading(_) => {
                c.text(px + 16.0, py + 12.0, 14.0, rgba(0xffffff, 1.0), &Canvas::fit(14.0, pw - 32.0, &self.name));
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xc8c8d8, 1.0), tr!("loading the profile...", "プロフィールを読み込み中...", "загрузка профиля..."));
                return;
            }
            State::Failed(e) => {
                c.text(px + 16.0, py + 12.0, 14.0, rgba(0xffffff, 1.0), &Canvas::fit(14.0, pw - 32.0, &self.name));
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xFF4B4B, 1.0), &Canvas::fit(9.0, pw - 32.0, e));
                return;
            }
            State::Loaded(p) => p,
        };
        // Header: name, then rank, pp, plays and when they joined on the right.
        let rank = p.rank.map(|r| format!("#{r}")).unwrap_or_else(|| tr!("unranked", "ランクなし", "без ранга").into());
        let hours = p.play_ms as f64 / 3_600_000.0;
        let minutes = hours * 60.0;
        let time = if hours >= 1.0 {
            tr!("{hours:.0}h played   ", "{hours:.0}時間プレイ   ", "{hours:.0} ч в игре   ";)
        } else if p.play_ms > 0 {
            tr!("{minutes:.0}m played   ", "{minutes:.0}分プレイ   ", "{minutes:.0} мин в игре   ";)
        } else {
            String::new()
        };
        let (pp, plays, joined) = (p.pp, p.plays, crate::scores::age(p.joined));
        let plays_ru = crate::lang::ru_plural(plays, ["игра", "игры", "игр"]);
        let stats = tr!(
            "{rank}   {pp:.0} pp   {plays} plays   {time}joined {joined} ago",
            "{rank}   {pp:.0} pp   {plays}プレイ   {time}{joined}前に登録",
            "{rank}   {pp:.0} pp   {plays} {plays_ru}   {time}с нами {joined}";
        );
        let stats_w = Canvas::text_width(8.0, &stats).min(pw * 0.6);
        c.text(px + 16.0, py + 12.0, 14.0, rgba(0xffffff, 1.0), &Canvas::fit(14.0, pw - stats_w - 48.0, &p.name));
        c.text_right(px + pw - 16.0, py + 16.0, 8.0, rgba(0xff8ad8, 1.0), &Canvas::fit(8.0, stats_w, &stats));
        // Two lists side by side.
        let col_w = (pw - 48.0) / 2.0;
        let top = py + 44.0;
        let bottom = by - 6.0;
        let lists = [
            (tr!("BEST PERFORMANCE", "ベストパフォーマンス", "ЛУЧШИЕ РЕЗУЛЬТАТЫ"), &p.top, tr!("no pp plays yet", "ppのプレイはまだありません", "пока нет игр с pp")),
            (tr!("RECENT PLAYS", "最近のプレイ", "ПОСЛЕДНИЕ ИГРЫ"), &p.recent, tr!("no plays yet", "プレイはまだありません", "пока нет игр")),
        ];
        for (k, (title, rows, empty)) in lists.into_iter().enumerate() {
            let x = px + 16.0 + k as f32 * (col_w + 16.0);
            c.text(x, top, 7.0, rgba(accent(), 1.0), title);
            if rows.is_empty() {
                c.text(x, top + 16.0, 7.0, rgba(0x6f6f86, 1.0), empty);
            }
            score_rows(c, rows, x, top + 14.0, col_w, bottom);
        }
    }
}

/// Plays as two-line rows: the map, then accuracy, combo, mods and rate; pp on the right.
fn score_rows(c: &mut Canvas, rows: &[OnlineScore], x: f32, top: f32, w: f32, bottom: f32) {
    const ROW: f32 = 26.0;
    for (i, r) in rows.iter().enumerate() {
        let y = top + i as f32 * ROW;
        if y + ROW - 3.0 > bottom {
            break;
        }
        c.round_rect(x, y, w, ROW - 3.0, 5.0, rgba(0x1d1d27, 1.0), rgba(0x2a2a38, 1.0), 1.0);
        let pp = r.pp.map(|p| format!("{p:.0}pp")).unwrap_or_else(|| "-".into());
        let pp_w = Canvas::text_width(8.0, &pp);
        c.text_right(x + w - 8.0, y + 7.0, 8.0, rgba(0xff8ad8, 1.0), &pp);
        let room = w - pp_w - 22.0;
        c.text(x + 8.0, y + 3.5, 7.0, rgba(0xffffff, 1.0), &Canvas::fit(7.0, room, &r.map));
        let rate = if (r.rate - 1.0).abs() > 1e-6 { format!("  {:.2}x", r.rate) } else { String::new() };
        let mods = if r.mods.is_empty() { String::new() } else { format!("  +{}", r.mods.concat()) };
        let (acc, combo, miss, age) = (r.accuracy * 100.0, r.max_combo, r.miss, crate::scores::age(r.date));
        let line = tr!(
            "{acc:.2}%  {combo}x  {miss} miss{mods}{rate}  {age}",
            "{acc:.2}%  {combo}x  ミス{miss}{mods}{rate}  {age}",
            "{acc:.2}%  {combo}x  {miss} мимо{mods}{rate}  {age}";
        );
        c.text(x + 8.0, y + 13.5, 6.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(6.0, room, &line));
    }
}
