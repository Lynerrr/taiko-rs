//! The terms: what the game keeps about a player and why. They're agreed to before an account
//! is made, and once by every account that hasn't agreed to this version
//! (`Config::terms_version`). The text lives in `assets/terms/`, one file per language: `# ` is
//! the title, `## ` a heading, `- ` a point, a blank line a gap.

use std::cell::Cell;

use winit::keyboard::KeyCode;

use crate::hit::{Canvas, Hit};
use crate::lang::Lang;
use crate::render::{accent, rgba};

/// The terms' version: raise it when the text changes in substance, and every account is asked
/// to agree again.
pub const VERSION: u32 = 1;

/// The terms in the language the game is in (English where there's no translation).
pub fn text() -> &'static str {
    match crate::lang::current() {
        Lang::Ja => include_str!("../assets/terms/ja.txt"),
        Lang::Ru => include_str!("../assets/terms/ru.txt"),
        Lang::En | Lang::ZhCn => include_str!("../assets/terms/en.txt"),
    }
}

/// Every translation, for checks that must hold in all of them.
#[cfg(test)]
pub const ALL: [&str; 3] = [
    include_str!("../assets/terms/en.txt"),
    include_str!("../assets/terms/ja.txt"),
    include_str!("../assets/terms/ru.txt"),
];

/// Why the terms are open: what agreeing goes on to do, and what saying no does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Before an account is made: agreeing sends the sign-up on, not agreeing just doesn't.
    Register,
    /// A logged-in account that hasn't agreed to this version: not agreeing logs it out.
    Account,
}

/// What a key did to the open terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Agreed,
    Declined,
}

pub struct View {
    pub purpose: Purpose,
    /// Lines scrolled past.
    scroll: usize,
    /// Lines that fit, and how many there are (as last drawn).
    shown: Cell<(usize, usize)>,
}

impl View {
    pub fn new(purpose: Purpose) -> View {
        View {
            purpose,
            scroll: 0,
            shown: Default::default(),
        }
    }

    /// A key while it's open: Enter agrees, Esc doesn't, the rest scroll.
    pub fn key(&mut self, code: KeyCode) -> Option<Answer> {
        let (fits, all) = self.shown.get();
        let page = fits.saturating_sub(2).max(1) as i32;
        let step = match code {
            KeyCode::Enter | KeyCode::NumpadEnter => return Some(Answer::Agreed),
            KeyCode::Escape | KeyCode::Backspace => return Some(Answer::Declined),
            KeyCode::ArrowUp => -1,
            KeyCode::ArrowDown => 1,
            KeyCode::PageUp => -page,
            KeyCode::PageDown | KeyCode::Space => page,
            KeyCode::Home => -(all as i32),
            KeyCode::End => all as i32,
            _ => 0,
        };
        self.scroll_by(step);
        None
    }

    pub fn scroll_by(&mut self, lines: i32) {
        let (fits, all) = self.shown.get();
        self.scroll =
            (self.scroll as i32 + lines).clamp(0, all.saturating_sub(fits) as i32) as usize;
    }

    pub fn draw(&self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.72));
        // Swallows clicks meant for what's underneath.
        c.hit(0.0, 0.0, w, 480.0, Hit::Overlay);
        let phone = crate::mobile::active();
        let pw = if phone { w } else { (w - 32.0).min(640.0) };
        let (px, py, ph) = if phone {
            (0.0, 0.0, 480.0)
        } else {
            ((w - pw) / 2.0, 24.0, 440.0)
        };
        c.round_rect(
            [px, py],
            [pw, ph],
            10.0,
            rgba(0x15151d, 0.98),
            rgba(accent(), 0.8),
            1.5,
        );
        let lines = layout(text(), pw - 40.0);
        let title = lines.first().map(|l| l.text.as_str()).unwrap_or("");
        c.text(px + 16.0, py + 12.0, 12.0, rgba(0xffffff, 1.0), title);
        let (by, bw, bh) = if phone {
            (py + ph - 52.0, 150.0, 44.0)
        } else {
            (py + ph - 28.0, 110.0, 18.0)
        };
        let size = crate::mobile::pick(7.0, 12.0);
        let (yes, no) = match self.purpose {
            Purpose::Register => (
                tr!("I agree", "同意する", "Согласен"),
                tr!("Not now", "今はしない", "Не сейчас"),
            ),
            Purpose::Account => (
                tr!("I agree", "同意する", "Согласен"),
                tr!("Log out", "ログアウト", "Выйти"),
            ),
        };
        c.button(
            px + pw - bw - 12.0,
            by,
            bw,
            bh,
            yes,
            size,
            Hit::Key(KeyCode::Enter),
            true,
        );
        c.button(
            px + pw - 2.0 * bw - 20.0,
            by,
            bw,
            bh,
            no,
            size,
            Hit::Key(KeyCode::Escape),
            false,
        );
        if !phone {
            c.text(
                px + 16.0,
                by + 5.5,
                6.0,
                rgba(0x6f6f86, 1.0),
                tr!(
                    "Up/Down, wheel: scroll",
                    "上下、ホイール: スクロール",
                    "Вверх/вниз, колесо: прокрутка"
                ),
            );
        }
        let body = &lines[1.min(lines.len())..];
        let (top, line_h) = (
            py + crate::mobile::pick(36.0, 44.0),
            crate::mobile::pick(12.0, 19.0),
        );
        let fits = ((by - 8.0 - top) / line_h).floor().max(1.0) as usize;
        self.shown.set((fits, body.len()));
        let scroll = self.scroll.min(body.len().saturating_sub(fits));
        for (k, line) in body.iter().skip(scroll).take(fits).enumerate() {
            let y = top + k as f32 * line_h;
            match line.kind {
                Kind::Title | Kind::Heading => {
                    c.text(px + 16.0, y, 7.0, rgba(accent(), 1.0), &line.text)
                }
                Kind::Point(first) => {
                    if first {
                        c.text(px + 16.0, y, 7.0, rgba(accent(), 1.0), "-");
                    }
                    c.text(px + 26.0, y, 7.0, rgba(0xc8c8d8, 1.0), &line.text)
                }
                Kind::Text => c.text(px + 16.0, y, 7.0, rgba(0xc8c8d8, 1.0), &line.text),
                Kind::Gap => 0.0,
            };
        }
        if body.len() > fits {
            let (bar_h, all) = (fits as f32 * line_h, body.len() as f32);
            let x = px + pw - 10.0;
            c.round_rect(
                [x, top],
                [3.0, bar_h],
                1.5,
                rgba(0x2a2a38, 1.0),
                rgba(0, 0.0),
                0.0,
            );
            let len = (fits as f32 / all * bar_h).max(12.0);
            c.round_rect(
                [x, top + scroll as f32 / all * bar_h],
                [3.0, len],
                1.5,
                rgba(accent(), 0.9),
                rgba(0, 0.0),
                0.0,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Title,
    Heading,
    /// A line of a point (whether the point starts here).
    Point(bool),
    Text,
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub kind: Kind,
    pub text: String,
}

/// The terms as lines that fit `width`: the title first.
pub(crate) fn layout(text: &str, width: f32) -> Vec<Line> {
    let chars =
        |size: f32, room: f32| ((room / Canvas::text_width(size, "M")).floor() as usize).max(8);
    let mut out = Vec::new();
    let mut push = |kind: Kind, text: String| out.push(Line { kind, text });
    for raw in text.lines() {
        let raw = raw.trim_end();
        if let Some(t) = raw.strip_prefix("# ") {
            push(Kind::Title, t.to_string());
        } else if let Some(t) = raw.strip_prefix("## ") {
            push(Kind::Gap, String::new());
            push(Kind::Heading, t.to_string());
        } else if let Some(t) = raw.strip_prefix("- ") {
            for (k, part) in wrap(t, chars(7.0, width - 12.0)).into_iter().enumerate() {
                push(Kind::Point(k == 0), part);
            }
        } else if raw.is_empty() {
            push(Kind::Gap, String::new());
        } else {
            for part in wrap(raw, chars(7.0, width)) {
                push(Kind::Text, part);
            }
        }
    }
    // One gap where the text has two in a row (a blank line before a heading).
    out.dedup_by(|b, a| a.kind == Kind::Gap && b.kind == Kind::Gap);
    out
}

/// `text` in lines `width` spaces wide. A line breaks after a space, or anywhere in Japanese
/// (which has no spaces), but not before its closing marks (`、。` and the like).
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let advance = crate::font::advance;
    let room = width * advance(' ');
    let wide = |c: char| c as u32 >= 0x2E80;
    let closing = |c: char| "、。，．）」』！？：".contains(c);
    // Where the next line may start in `line`: after its last space, or between wide letters.
    let last_break = |line: &[char]| {
        (1..line.len()).rev().find(|&i| {
            let (a, b) = (line[i - 1], line[i]);
            b != ' ' && (a == ' ' || ((wide(a) || wide(b)) && !closing(b)))
        })
    };
    let finish = |line: &[char]| line.iter().collect::<String>().trim_end().to_string();
    let mut lines = Vec::new();
    let mut line: Vec<char> = Vec::new();
    let mut used = 0;
    for c in text.chars() {
        // A space past the end is dropped with the line's end, so it never breaks one.
        if used + advance(c) > room && !line.is_empty() && c != ' ' {
            let at = last_break(&line).unwrap_or(line.len());
            let rest = line.split_off(at);
            lines.push(finish(&line));
            line = rest;
            used = line.iter().map(|&c| advance(c)).sum();
        }
        if line.is_empty() && c == ' ' {
            continue;
        }
        line.push(c);
        used += advance(c);
    }
    if !line.is_empty() {
        lines.push(finish(&line));
    }
    lines
}
