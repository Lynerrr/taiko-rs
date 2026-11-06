//! The manual: Help > Manual opens a window over the editor with a chapter list on the left and
//! the chapter's text on the right (`manual_text`). Wheel, arrows and PgUp / PgDn scroll it.

use super::chrome::{DIM, wrap};
use super::manual_text::{self, TryIt};
use super::{Btn, Modal, View};
use crate::audio::Engine;
use crate::render::{Canvas, accent, rgba};
use crate::tko::recipes::Recipe;

/// A button of the manual.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ManAct {
    Chapter(u8),
    Try,
    Close,
}

#[derive(Default)]
pub struct ManualUi {
    pub chapter: usize,
    /// Lines scrolled off the top of the text.
    pub scroll: usize,
    /// Lines the text has and the window shows, from the last draw (to clamp scrolling).
    lines: usize,
    shown: usize,
}

const TEXT: f32 = 7.0;
const LINE: f32 = 10.0;
const LIST_W: f32 = 150.0;

/// One drawn line of a chapter.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Gap,
    Heading(String),
    Text(String, u32, f32),
    Tip(String),
}

/// A chapter's body laid out for `width`: headings, bullets (with a hanging indent), tips.
fn layout(body: &str, width: f32) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            rows.push(Row::Gap);
        } else if let Some(h) = line.strip_prefix("# ") {
            if !rows.is_empty() {
                rows.push(Row::Gap);
            }
            rows.push(Row::Heading(h.to_string()));
        } else if let Some(b) = line.strip_prefix("- ") {
            let indent = 10.0;
            for (i, l) in wrap(TEXT, width - indent, b).into_iter().enumerate() {
                let text = if i == 0 { format!("- {l}") } else { l };
                rows.push(Row::Text(text, 0xd0d0e0, if i == 0 { 0.0 } else { indent }));
            }
        } else if let Some(t) = line.strip_prefix("> ") {
            for l in wrap(TEXT, width - 16.0, t) {
                rows.push(Row::Tip(l));
            }
        } else {
            for l in wrap(TEXT, width, line) {
                rows.push(Row::Text(l, 0xd0d0e0, 0.0));
            }
        }
    }
    rows
}

/// The chapter's body with `{recipes}` written out from the recipe table.
fn body_of(chapter: &manual_text::Chapter) -> String {
    if !chapter.body.contains("{recipes}") {
        return chapter.body.to_string();
    }
    let list: String = Recipe::ALL
        .iter()
        .map(|r| format!("- {}: {}\n", r.label(), r.about()))
        .collect();
    chapter.body.replace("{recipes}", &list)
}

impl View {
    /// Help > Manual.
    pub(super) fn open_manual(&mut self) {
        self.modal = Some(Modal::Manual);
    }

    pub(super) fn press_manual(&mut self, act: ManAct, audio: Option<&Engine>) {
        match act {
            ManAct::Close => self.modal = None,
            ManAct::Chapter(i) => {
                self.manual.chapter = (i as usize).min(manual_text::COUNT - 1);
                self.manual.scroll = 0;
            }
            ManAct::Try => {
                let Some((_, what)) = manual_text::chapters()
                    .get(self.manual.chapter)
                    .and_then(|c| c.try_it)
                else {
                    return;
                };
                self.modal = None;
                match what {
                    TryIt::Tour => self.start_tour(),
                    TryIt::TimingWindow => self.run(super::cmd::Cmd::TimingList, audio),
                    TryIt::Nodes => {
                        self.run(super::cmd::Cmd::Mode(super::cmd::Mode::Effects), audio);
                        self.press(Btn::GPage(3), super::Keys::default(), audio);
                    }
                    TryIt::Motion => {
                        self.run(super::cmd::Cmd::Mode(super::cmd::Mode::Effects), audio);
                        self.press(
                            Btn::GPage(super::motion::PAGE),
                            super::Keys::default(),
                            audio,
                        );
                    }
                    TryIt::Showcase => {
                        self.run(super::cmd::Cmd::Mode(super::cmd::Mode::Effects), audio);
                        self.press(Btn::GPage(4), super::Keys::default(), audio);
                        if !self.ed.is_tko() {
                            let make = Btn::K(super::keys::KAct::MakeTko);
                            self.press(make, super::Keys::default(), audio);
                        }
                        self.showcase();
                    }
                }
            }
        }
    }

    /// Scrolls the text by `lines` (negative: up).
    pub(super) fn manual_scroll(&mut self, lines: i32) {
        let m = &mut self.manual;
        let most = m.lines.saturating_sub(m.shown);
        m.scroll = (m.scroll as i64 + lines as i64).clamp(0, most as i64) as usize;
    }

    /// A page of text, for PgUp / PgDn.
    pub(super) fn manual_page(&self) -> i32 {
        self.manual.shown.saturating_sub(2).max(1) as i32
    }

    pub(super) fn draw_manual(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.7));
        let w = (c.width - 24.0).min(900.0);
        let (x, y, h) = ((c.width - w) / 2.0, 14.0, 452.0);
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x15151d, 1.0),
            rgba(0x5a5a78, 1.0),
            1.0,
        );
        c.text(x + 12.0, y + 9.0, 9.0, rgba(0xffffff, 1.0), "Manual");
        let close = "Close  [Esc]";
        let cw = Canvas::text_width(7.0, close) + 16.0;
        self.button(
            c,
            [x + w - cw - 10.0, y + 7.0, cw, 18.0],
            close,
            Btn::Man(ManAct::Close),
            false,
        );
        let chapters = manual_text::chapters();
        let at = self.manual.chapter.min(chapters.len() - 1);
        // The chapter list.
        let top = y + 32.0;
        for (i, ch) in chapters.iter().enumerate() {
            let r = [x + 8.0, top + i as f32 * 20.0, LIST_W - 8.0, 18.0];
            self.button(
                c,
                r,
                &Canvas::fit(7.0, r[2] - 10.0, ch.title),
                Btn::Man(ManAct::Chapter(i as u8)),
                i == at,
            );
        }
        // The text.
        let (tx, ty) = (x + LIST_W + 14.0, top);
        let (tw, th) = (w - LIST_W - 30.0, h - 32.0 - 34.0);
        c.rect(tx - 6.0, ty - 4.0, 1.0, th + 8.0, rgba(0x2a2a38, 1.0));
        let chapter = &chapters[at];
        c.text(tx, ty, 10.0, rgba(accent(), 1.0), chapter.title);
        let rows = layout(&body_of(chapter), tw - 10.0);
        let body_y = ty + 18.0;
        let shown = (((th - 18.0 - if chapter.try_it.is_some() { 24.0 } else { 0.0 }) / LINE)
            .floor() as usize)
            .max(1);
        self.manual.lines = rows.len();
        self.manual.shown = shown;
        self.manual.scroll = self.manual.scroll.min(rows.len().saturating_sub(shown));
        let start = c.shapes.len();
        for (i, row) in rows.iter().skip(self.manual.scroll).take(shown).enumerate() {
            let ry = body_y + i as f32 * LINE;
            match row {
                Row::Gap => {}
                Row::Heading(h) => {
                    c.text(tx, ry, 8.0, rgba(0xF0B64A, 1.0), h);
                }
                Row::Text(t, col, indent) => {
                    c.text(tx + indent, ry, TEXT, rgba(*col, 1.0), t);
                }
                Row::Tip(t) => {
                    c.rect(tx, ry - 2.0, 2.0, LINE, rgba(accent(), 1.0));
                    c.text(tx + 8.0, ry, TEXT, rgba(0xffffff, 1.0), t);
                }
            }
        }
        c.clip_since(start, tx, body_y - 2.0, tw, shown as f32 * LINE + 2.0);
        // A thin scroll bar when the text is longer than the window.
        if rows.len() > shown {
            let (bx, bh) = (tx + tw - 2.0, shown as f32 * LINE);
            c.rect(bx, body_y, 2.0, bh, rgba(0x23232f, 1.0));
            let thumb = (bh * shown as f32 / rows.len() as f32).max(14.0);
            let free = bh - thumb;
            let most = (rows.len() - shown) as f32;
            c.rect(
                bx,
                body_y + free * self.manual.scroll as f32 / most,
                2.0,
                thumb,
                rgba(0x6a6a88, 1.0),
            );
            c.text(
                tx,
                y + h - 14.0,
                6.0,
                rgba(DIM, 1.0),
                "Wheel, arrows and PgUp / PgDn scroll",
            );
        }
        if let Some((label, _)) = chapter.try_it {
            let bw = Canvas::text_width(7.0, label) + 16.0;
            self.button(
                c,
                [tx, ty + th - 18.0, bw, 18.0],
                label,
                Btn::Man(ManAct::Try),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{self, Lang};

    #[test]
    fn every_chapter_exists_in_both_languages_with_the_same_shape() {
        let en = lang::with(Lang::En, manual_text::chapters);
        assert_eq!(en.len(), manual_text::COUNT);
        let ru_chapters = lang::with(Lang::Ru, manual_text::chapters);
        for (e, r) in en.iter().zip(ru_chapters) {
            assert!(!e.title.is_empty() && !r.title.is_empty());
            assert_ne!(e.title, r.title, "{} isn't translated", e.title);
            assert_eq!(e.try_it.map(|t| t.1), r.try_it.map(|t| t.1), "{}", e.title);
            // The same headings and the same number of bullets: nothing left out of a translation.
            let count = |b: &str, p: &str| b.lines().filter(|l| l.starts_with(p)).count();
            for prefix in ["# ", "- ", "> "] {
                assert_eq!(
                    count(e.body, prefix),
                    count(r.body, prefix),
                    "{}: lines starting {prefix:?}",
                    e.title
                );
            }
            assert_eq!(e.body.contains("{recipes}"), r.body.contains("{recipes}"));
        }
    }

    #[test]
    fn the_recipe_chapter_lists_every_recipe() {
        let chapters = lang::with(Lang::En, manual_text::chapters);
        let at = chapters
            .iter()
            .position(|c| c.body.contains("{recipes}"))
            .unwrap();
        let text = body_of(&chapters[at]);
        assert!(!text.contains("{recipes}"));
        for r in Recipe::ALL {
            assert!(text.contains(r.label()), "{} is missing", r.label());
        }
    }

    #[test]
    fn text_is_laid_out_in_rows_that_fit() {
        let rows = layout(
            "Hello world\n\n# Head\n- one two three four five six seven\n> a tip",
            60.0,
        );
        assert!(rows.contains(&Row::Heading("Head".into())));
        assert!(rows.iter().any(|r| matches!(r, Row::Tip(_))));
        for r in &rows {
            if let Row::Text(t, _, indent) = r {
                assert!(Canvas::text_width(TEXT, t) + indent <= 60.0 + 20.0, "{t}");
            }
        }
        assert!(
            rows.iter()
                .filter(|r| matches!(r, Row::Text(t, ..) if t.starts_with("- ")))
                .count()
                == 1
        );
    }
}
