//! What's new: the changes between the build a player had and the one they have now, read from
//! the commits on GitHub (a CI build knows its commit, `update::BUILD`). It opens by itself the
//! first time a new build starts, and any time from its button.

use std::sync::mpsc::{self, Receiver};

use winit::keyboard::KeyCode;

use crate::render::{accent, rgba, Canvas, Hit};

/// One change: when, what, and its points (the commit's "- ..." lines).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub date: String,
    pub title: String,
    pub points: Vec<String>,
}

pub enum State {
    Loading(Receiver<Result<Vec<Change>, String>>),
    Loaded(Vec<Change>),
    Failed(String),
}

pub struct View {
    /// Opened by an update (the changes since the build before), not the button.
    pub after_update: bool,
    pub state: State,
    /// Lines scrolled past.
    scroll: usize,
    /// Lines that fit, and how many there are (as last drawn).
    shown: std::cell::Cell<(usize, usize)>,
}

impl View {
    /// The changes from build `old` to this one.
    pub fn since(old: String) -> View {
        let new = crate::update::BUILD.unwrap_or_default().to_string();
        View::fetch(true, move || commits(&format!("compare/{old}...{new}")))
    }

    /// The latest changes in this build (or its channel's, for a local build).
    pub fn recent(after_update: bool) -> View {
        let at = crate::update::BUILD.unwrap_or(crate::update::BUILD_CHANNEL.branch()).to_string();
        View::fetch(after_update, move || commits(&format!("commits?sha={at}&per_page=40")))
    }

    fn fetch(after_update: bool, get: impl FnOnce() -> Result<Vec<Change>, String> + Send + 'static) -> View {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(get());
        });
        View { after_update, state: State::Loading(rx), scroll: 0, shown: Default::default() }
    }

    /// Already loaded with `changes` (for tests).
    #[cfg(test)]
    pub fn loaded(changes: Vec<Change>) -> View {
        View { after_update: true, state: State::Loaded(changes), scroll: 0, shown: Default::default() }
    }

    pub fn poll(&mut self) {
        if let State::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(changes)) => self.state = State::Loaded(changes),
                Ok(Err(e)) => self.state = State::Failed(e),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.state = State::Failed(tr!("couldn't get the changes", "変更点を取得できませんでした", "не удалось получить изменения").into()),
            }
        }
    }

    /// A key while it's open; true when it closes.
    pub fn key(&mut self, code: KeyCode) -> bool {
        let (fits, all) = self.shown.get();
        let page = fits.saturating_sub(2).max(1) as i32;
        let step = match code {
            KeyCode::Escape | KeyCode::Backspace | KeyCode::Enter | KeyCode::NumpadEnter => return true,
            KeyCode::ArrowUp => -1,
            KeyCode::ArrowDown => 1,
            KeyCode::PageUp => -page,
            KeyCode::PageDown | KeyCode::Space => page,
            KeyCode::Home => -(all as i32),
            KeyCode::End => all as i32,
            _ => 0,
        };
        self.scroll_by(step);
        false
    }

    pub fn scroll_by(&mut self, lines: i32) {
        let (fits, all) = self.shown.get();
        self.scroll = (self.scroll as i32 + lines).clamp(0, all.saturating_sub(fits) as i32) as usize;
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
        // Swallows clicks meant for what's underneath.
        c.hit(0.0, 0.0, w, 480.0, Hit::Overlay);
        let pw = (w - 32.0).min(640.0);
        let (px, py, ph) = ((w - pw) / 2.0, 36.0, 420.0);
        c.round_rect([px, py], [pw, ph], 10.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
        c.text(px + 16.0, py + 12.0, 14.0, rgba(0xffffff, 1.0), tr!("What's new", "更新内容", "Что нового"));
        let about = match (self.after_update, crate::update::BUILD) {
            (true, Some(b)) => tr!("updated to {}", "{}に更新しました", "обновлено до {}"; crate::update::short(b)),
            (false, Some(b)) => tr!("this build: {}", "このビルド: {}", "эта сборка: {}"; crate::update::short(b)),
            (_, None) => tr!("a local build: the latest changes", "ローカルビルド: 最新の変更", "локальная сборка: последние изменения").into(),
        };
        c.text_right(px + pw - 16.0, py + 16.0, 7.0, rgba(0x8a8aa0, 1.0), &Canvas::fit(7.0, pw - 180.0, &about));
        let (by, bw) = (py + ph - 26.0, 64.0);
        c.button(px + pw - bw - 12.0, by, bw, 16.0, tr!("Close", "閉じる", "Закрыть"), 7.0, Hit::Key(KeyCode::Escape), true);
        c.text(px + 16.0, by + 4.5, 6.0, rgba(0x6f6f86, 1.0), tr!("Up/Down, wheel: scroll   Esc: close", "上下、ホイール: スクロール   Esc: 閉じる", "Вверх/вниз, колесо: прокрутка   Esc: закрыть"));
        let changes = match &self.state {
            State::Loading(_) => {
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xc8c8d8, 1.0), tr!("getting the changes...", "変更点を取得中...", "получаем изменения..."));
                return;
            }
            State::Failed(e) => {
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xFF4B4B, 1.0), &Canvas::fit(9.0, pw - 32.0, e));
                return;
            }
            State::Loaded(changes) if changes.is_empty() => {
                c.text_centered(px + pw / 2.0, py + ph / 2.0, 9.0, rgba(0xc8c8d8, 1.0), tr!("nothing new", "新しいものはありません", "ничего нового"));
                return;
            }
            State::Loaded(changes) => changes,
        };
        let lines = layout(changes, pw - 40.0);
        let (top, line_h) = (py + 40.0, 13.0);
        let fits = ((by - 8.0 - top) / line_h).floor().max(1.0) as usize;
        self.shown.set((fits, lines.len()));
        let scroll = self.scroll.min(lines.len().saturating_sub(fits));
        for (k, line) in lines.iter().skip(scroll).take(fits).enumerate() {
            let y = top + k as f32 * line_h;
            match line {
                Line::Head(date, title) => {
                    let dw = c.text(px + 16.0, y, 6.0, rgba(0x6f6f86, 1.0), date);
                    c.text(px + 22.0 + dw, y - 1.0, 8.0, rgba(0xffffff, 1.0), title);
                }
                Line::Title(text) => {
                    c.text(px + 22.0 + Canvas::text_width(6.0, "0000-00-00"), y - 1.0, 8.0, rgba(0xffffff, 1.0), text);
                }
                Line::Point(first, text) => {
                    let x = px + 30.0 + Canvas::text_width(6.0, "0000-00-00");
                    if *first {
                        c.text(x - 8.0, y, 7.0, rgba(accent(), 1.0), "-");
                    }
                    c.text(x, y, 7.0, rgba(0xc8c8d8, 1.0), text);
                }
            }
        }
        if lines.len() > fits {
            let (bar_h, all) = (fits as f32 * line_h, lines.len() as f32);
            let x = px + pw - 10.0;
            c.round_rect([x, top], [3.0, bar_h], 1.5, rgba(0x2a2a38, 1.0), rgba(0, 0.0), 0.0);
            let len = (fits as f32 / all * bar_h).max(12.0);
            c.round_rect([x, top + scroll as f32 / all * bar_h], [3.0, len], 1.5, rgba(accent(), 0.9), rgba(0, 0.0), 0.0);
        }
    }
}

enum Line {
    /// A change's first line: its date and the start of its title.
    Head(String, String),
    /// More of a long title.
    Title(String),
    /// A line of one of its points (whether the point starts here).
    Point(bool, String),
}

/// The changes as lines that fit `width`.
fn layout(changes: &[Change], width: f32) -> Vec<Line> {
    let indent = Canvas::text_width(6.0, "0000-00-00") + 14.0;
    let chars = |size: f32, room: f32| ((room / Canvas::text_width(size, "M")).floor() as usize).max(8);
    let mut out = Vec::new();
    for ch in changes {
        let title = crate::social::wrap(&ch.title, chars(8.0, width - indent + 8.0));
        for (k, t) in title.into_iter().enumerate() {
            out.push(if k == 0 { Line::Head(ch.date.clone(), t) } else { Line::Title(t) });
        }
        for p in &ch.points {
            for (k, t) in crate::social::wrap(p, chars(7.0, width - indent - 8.0)).into_iter().enumerate() {
                out.push(Line::Point(k == 0, t));
            }
        }
    }
    out
}

/// The changes a GitHub API call returns (`compare/...` or `commits?...`), newest first.
fn commits(path: &str) -> Result<Vec<Change>, String> {
    #[derive(serde::Deserialize)]
    struct Author {
        date: String,
    }
    #[derive(serde::Deserialize)]
    struct Commit {
        message: String,
        author: Author,
    }
    #[derive(serde::Deserialize)]
    struct Item {
        commit: Commit,
    }
    #[derive(serde::Deserialize)]
    struct Compare {
        commits: Vec<Item>,
    }
    let text = crate::update::api_get(path)?;
    let mut items: Vec<Item> = if path.starts_with("compare/") {
        // Oldest first there.
        let mut c = serde_json::from_str::<Compare>(&text).map_err(|e| format!("changes: {e}"))?.commits;
        c.reverse();
        c
    } else {
        serde_json::from_str(&text).map_err(|e| format!("changes: {e}"))?
    };
    Ok(items.drain(..).filter_map(|i| parse(&i.commit.message, &i.commit.author.date)).collect())
}

/// A commit as a change: its first line, and its "- " lines (continued by indented lines)
/// without the trailers. None for merges, and for what isn't for everyone: a `Changelog: skip`
/// line, or a player's own secret.
pub fn parse(message: &str, date: &str) -> Option<Change> {
    let lower = message.to_ascii_lowercase();
    if lower.lines().any(|l| l.trim() == "changelog: skip") || lower.contains("krokodil") {
        return None;
    }
    let mut lines = message.lines();
    let title = lines.next()?.trim().to_string();
    if title.is_empty() || title.starts_with("Merge ") {
        return None;
    }
    let mut points: Vec<String> = Vec::new();
    for line in lines {
        let trailer = ["Co-Authored-By:", "Claude-Session:", "Signed-off-by:"].iter().any(|t| line.to_ascii_lowercase().starts_with(&t.to_ascii_lowercase()));
        if trailer {
            continue;
        }
        if let Some(point) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            points.push(point.trim().to_string());
        } else if line.starts_with("  ") && !line.trim().is_empty()
            && let Some(last) = points.last_mut() {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    Some(Change { date: date.get(..10).unwrap_or(date).to_string(), title, points })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_become_changes_without_trailers_or_merges() {
        let msg = "pp counter and more\n\n- Live pp counter in the HUD\n- Results: the song plays on, back\n  to the preview point\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nClaude-Session: https://x\n";
        let c = parse(msg, "2026-10-26T01:02:03Z").unwrap();
        assert_eq!(c.title, "pp counter and more");
        assert_eq!(c.date, "2026-10-26");
        assert_eq!(c.points, vec!["Live pp counter in the HUD".to_string(), "Results: the song plays on, back to the preview point".into()]);
        assert!(parse("Merge remote-tracking branch 'origin/x'", "2026-10-26").is_none());
        assert!(parse("A quiet one\n\nChangelog: skip\n", "2026-10-26").is_none());
        assert!(parse("Krokodil_Koban mode: something\n", "2026-10-26").is_none());
    }

    #[test]
    fn long_changes_wrap_to_the_panel() {
        let ch = Change { date: "2026-10-26".into(), title: "word ".repeat(60), points: vec!["point ".repeat(40)] };
        let lines = layout(&[ch], 400.0);
        assert!(lines.len() > 4);
        assert!(matches!(lines[0], Line::Head(..)));
    }
}
