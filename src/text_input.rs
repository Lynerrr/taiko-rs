//! Reusable single-line text editing and drawing. Offsets are UTF-8 grapheme boundaries.
use crate::{
    clipboard,
    render::{Canvas, rgba},
};
use unicode_segmentation::UnicodeSegmentation;
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Social,
    SettingsSearch,
    FontSearch,
    Setting(crate::settings::Item),
    Resolution,
    SongSearch,
    BrowseSearch,
    PlayerSearch,
    LobbyChat,
    LobbyPassword,
    Account(u8),
    Preset,
    Collection,
    Random,
    Universe,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Rules {
    pub max: usize,
    pub secret: bool,
}

#[derive(Clone, Default)]
struct Checkpoint {
    text: String,
    caret: usize,
    anchor: usize,
}

/// The component owns editing state; callers bind it to their model using `sync` and `text`.
#[derive(Clone, Default)]
pub(crate) struct TextInput {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    undo: Vec<Checkpoint>,
    redo: Vec<Checkpoint>,
    last_click: Option<(std::time::Instant, usize, u8)>,
}

impl TextInput {
    pub fn new(text: String) -> Self {
        let caret = text.len();
        Self {
            text,
            caret,
            anchor: caret,
            ..Self::default()
        }
    }
    pub fn view(&self) -> Self {
        Self {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
            ..Self::default()
        }
    }
    pub fn place(&mut self, offset: usize, extend: bool) {
        self.caret = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.text.len()))
            .take_while(|i| *i <= offset)
            .last()
            .unwrap_or(0);
        if !extend {
            self.anchor = self.caret;
        }
    }
    pub fn click(&mut self, offset: usize, extend: bool, now: std::time::Instant) {
        let count = self
            .last_click
            .filter(|(at, previous, _)| {
                *previous == offset && now.saturating_duration_since(*at).as_millis() < 400
            })
            .map_or(1, |(_, _, n)| n % 3 + 1);
        self.last_click = Some((now, offset, count));
        self.place(offset, extend);
        if !extend && count == 2 {
            let offset = offset.min(self.text.len().saturating_sub(1));
            if let Some((start, word)) = self
                .text
                .split_word_bound_indices()
                .find(|(i, w)| *i <= offset && offset < *i + w.len())
            {
                self.anchor = start;
                self.caret = start + word.len();
            }
        } else if !extend && count == 3 {
            self.select_all();
        }
    }

    pub fn sync(&mut self, text: &str) {
        if self.text != text {
            *self = Self::new(text.into());
        }
    }
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }
    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }
    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        }
    }
    fn restore(&mut self, c: Checkpoint) {
        self.text = c.text;
        self.caret = c.caret;
        self.anchor = c.anchor;
    }
    fn remember(&mut self) {
        if self.undo.len() == 100 {
            self.undo.remove(0);
        }
        self.undo.push(self.checkpoint());
        self.redo.clear();
    }
    fn previous(&self, at: usize) -> usize {
        self.text[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self, at: usize) -> usize {
        self.text[at..]
            .graphemes(true)
            .next()
            .map_or(at, |s| at + s.len())
    }
    fn word(&self, forward: bool) -> usize {
        let mut at = self.caret;
        if forward {
            while at < self.text.len() && !self.text[at..].starts_with(char::is_whitespace) {
                at = self.next(at);
            }
            while at < self.text.len() && self.text[at..].starts_with(char::is_whitespace) {
                at = self.next(at);
            }
        } else {
            while at > 0 && self.text[..at].ends_with(char::is_whitespace) {
                at = self.previous(at);
            }
            while at > 0 && !self.text[..at].ends_with(char::is_whitespace) {
                at = self.previous(at);
            }
        }
        at
    }
    pub fn insert(&mut self, typed: &str, max: usize, accepts: impl Fn(char) -> bool) {
        let range = self.selection();
        let room = max.saturating_sub(
            self.text[..range.start].chars().count() + self.text[range.end..].chars().count(),
        );
        let typed: String = typed
            .chars()
            .filter(|c| !c.is_control() && accepts(*c))
            .take(room)
            .collect();
        if typed.is_empty() {
            return;
        }
        self.remember();
        self.text.replace_range(range.clone(), &typed);
        let at = range.start + typed.len();
        // Inserting a combining character can join the neighbouring grapheme.
        self.caret = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.text.len()))
            .find(|i| *i >= at)
            .unwrap();
        self.anchor = self.caret;
    }
    fn delete(&mut self, backward: bool, word: bool) {
        let mut range = self.selection();
        if range.is_empty() {
            if backward {
                range.start = if word {
                    self.word(false)
                } else {
                    self.previous(self.caret)
                };
            } else {
                range.end = if word {
                    self.word(true)
                } else {
                    self.next(self.caret)
                };
            }
        }
        if !range.is_empty() {
            self.remember();
            self.text.replace_range(range.clone(), "");
            self.place(range.start, false);
        }
    }
    /// Returns false for submit, cancel, tab and other commands belonging to the surrounding UI.
    pub fn key(
        &mut self,
        code: KeyCode,
        typed: Option<&str>,
        m: Modifiers,
        repeat: bool,
        rules: Rules,
        accepts: impl Fn(char) -> bool,
    ) -> bool {
        use KeyCode::*;
        let Rules { max, secret } = rules;
        self.last_click = None;
        let command = m.ctrl || m.super_key;
        let word = m.ctrl || (cfg!(target_os = "macos") && m.alt);
        match code {
            KeyA if command => self.select_all(),
            KeyC | KeyX if command => {
                if !repeat
                    && !secret
                    && !self.selection().is_empty()
                    && clipboard::set(self.selected())
                    && code == KeyX
                {
                    self.delete(false, false);
                }
            }
            KeyV if command => {
                if !repeat && let Some(text) = clipboard::get() {
                    self.insert(
                        &text.replace("\r\n", "\n").replace(['\r', '\n'], " "),
                        max,
                        accepts,
                    );
                }
            }
            KeyZ | KeyY if command => {
                if !repeat {
                    let redo = code == KeyY || m.shift;
                    let item = if redo {
                        self.redo.pop()
                    } else {
                        self.undo.pop()
                    };
                    if let Some(item) = item {
                        let current = self.checkpoint();
                        if redo {
                            self.undo.push(current);
                        } else {
                            self.redo.push(current);
                        }
                        self.restore(item);
                    }
                }
            }
            ArrowLeft | ArrowRight | Home | End => {
                let right = matches!(code, ArrowRight | End);
                self.caret = if code == Home || (code == ArrowLeft && m.super_key) {
                    0
                } else if code == End || (code == ArrowRight && m.super_key) {
                    self.text.len()
                } else if !m.shift && !self.selection().is_empty() {
                    if right {
                        self.selection().end
                    } else {
                        self.selection().start
                    }
                } else if word {
                    self.word(right)
                } else if right {
                    self.next(self.caret)
                } else {
                    self.previous(self.caret)
                };
                if !m.shift {
                    self.anchor = self.caret;
                }
            }
            Backspace | Delete => self.delete(code == Backspace, word),
            Space if !command && !m.alt => self.insert(typed.unwrap_or(" "), max, accepts),
            _ if !command
                && !m.alt
                && typed.is_some_and(|s| s.chars().any(|c| !c.is_control())) =>
            {
                self.insert(typed.unwrap(), max, accepts)
            }
            _ => return false,
        }
        true
    }
}

/// Geometry of a rendered field, shared by click/drag placement and horizontal scrolling.
#[derive(Clone)]
pub(crate) struct Region {
    pub field: Field,
    pub rect: [f32; 4],
    pub stops: Vec<(usize, f32)>,
}
impl Region {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, w, h] = self.rect;
        x >= rx && x <= rx + w && y >= ry && y <= ry + h
    }
    pub fn offset(&self, x: f32) -> usize {
        self.stops
            .iter()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |s| s.0)
    }
}

impl<H> Canvas<H> {
    /// Draws a single-line input, keeping the caret visible and recording mouse placement stops.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn input(
        &mut self,
        field: Field,
        rect: [f32; 4],
        size: f32,
        text: &str,
        hint: &str,
        color: [f32; 4],
        secret: bool,
        focused: bool,
    ) {
        let [x, y, w, h] = rect;
        let editor = self
            .input_editor
            .as_ref()
            .filter(|(id, e)| *id == field && e.text == text)
            .map(|(_, e)| e);
        let caret = editor.map_or(text.len(), |e| e.caret);
        let selection = editor.map_or(caret..caret, |e| e.selection());
        let display = |s: &str| {
            if secret {
                "*".repeat(s.graphemes(true).count())
            } else {
                s.to_owned()
            }
        };
        let mut stops = vec![(0, 0.0)];
        let mut advance = 0.0;
        for (i, grapheme) in text.grapheme_indices(true) {
            advance += Self::text_width(size, if secret { "*" } else { grapheme });
            stops.push((i + grapheme.len(), advance));
        }
        let caret_x = stops.iter().find(|s| s.0 == caret).map_or(0.0, |s| s.1);
        let scroll = (caret_x - w + 2.0).max(0.0);
        let start = stops.iter().find(|s| s.1 >= scroll).map_or(0, |s| s.0);
        let left = stops.iter().find(|s| s.0 == start).map_or(0.0, |s| s.1);
        let end = stops
            .iter()
            .rev()
            .find(|s| s.1 - left <= w - 1.0)
            .map_or(start, |s| s.0)
            .max(start);
        let caret_x = caret_x - left;
        if selection.start != selection.end {
            let at = |i| {
                stops
                    .iter()
                    .find(|s| s.0 == i)
                    .map_or(0.0, |s| s.1 - left)
                    .clamp(0.0, w)
            };
            let a = at(selection.start);
            let b = at(selection.end);
            self.rect(x + a, y - 1.0, b - a, h, rgba(crate::render::accent(), 0.4));
        }
        let shown = if text.is_empty() {
            Self::fit(size, w, hint)
        } else {
            display(&text[start..end])
        };
        self.text(
            x,
            y,
            size,
            if text.is_empty() {
                rgba(0x6f6f86, color[3])
            } else {
                color
            },
            &shown,
        );
        if focused {
            self.caret(x + caret_x, y, size, color);
        }
        self.input_regions.push(Region {
            field,
            rect: [x - 6.0, y - 4.0, w + 12.0, h + 8.0],
            stops: stops
                .into_iter()
                .map(|(i, px)| (i, x + px - left))
                .collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(e: &mut TextInput, code: KeyCode, modifiers: Modifiers) {
        assert!(e.key(
            code,
            None,
            modifiers,
            false,
            Rules {
                max: 200,
                secret: false
            },
            |_| true
        ));
    }
    #[test]
    fn moves_and_edits_between_unicode_graphemes() {
        let mut e = TextInput::new("Привет e\u{301}👩‍👩‍👧‍👦".into());
        key(&mut e, KeyCode::Backspace, Modifiers::default());
        assert_eq!(e.text, "Привет e\u{301}");
        key(&mut e, KeyCode::ArrowLeft, Modifiers::default());
        e.insert("!", 200, |_| true);
        assert_eq!(e.text, "Привет !e\u{301}");
        key(&mut e, KeyCode::Delete, Modifiers::default());
        assert_eq!(e.text, "Привет !");
        key(&mut e, KeyCode::Home, Modifiers::default());
        key(&mut e, KeyCode::Delete, Modifiers::default());
        assert_eq!(e.text, "ривет !");
    }
    #[test]
    fn selection_clipboard_and_undo_use_the_same_component() {
        clipboard::fake_for_this_thread();
        let mut e = TextInput::new("one two".into());
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        key(
            &mut e,
            KeyCode::ArrowLeft,
            Modifiers {
                shift: true,
                ..ctrl
            },
        );
        assert_eq!(e.selected(), "two");
        key(&mut e, KeyCode::KeyC, ctrl);
        assert_eq!(clipboard::get().as_deref(), Some("two"));
        key(&mut e, KeyCode::KeyX, ctrl);
        assert_eq!(e.text, "one ");
        key(&mut e, KeyCode::Home, Modifiers::default());
        key(&mut e, KeyCode::KeyV, ctrl);
        assert_eq!(e.text, "twoone ");
        key(&mut e, KeyCode::KeyZ, ctrl);
        assert_eq!(e.text, "one ");
        key(&mut e, KeyCode::KeyZ, ctrl);
        assert_eq!(e.text, "one two");
        key(
            &mut e,
            KeyCode::KeyZ,
            Modifiers {
                shift: true,
                ..ctrl
            },
        );
        assert_eq!(e.text, "one ");
    }
    #[test]
    fn selection_collapses_before_moving_and_typing_replaces_it() {
        let mut e = TextInput::new("abc".into());
        e.select_all();
        key(&mut e, KeyCode::ArrowLeft, Modifiers::default());
        assert_eq!((e.caret, e.anchor), (0, 0));
        key(
            &mut e,
            KeyCode::End,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        e.insert("Ж", 3, |_| true);
        assert_eq!(e.text, "Ж");
        assert_eq!(e.caret, "Ж".len());
        assert!(e.selection().is_empty());
    }
    #[test]
    fn masked_inputs_do_not_copy_or_cut_and_pastes_respect_limits() {
        clipboard::fake_for_this_thread();
        clipboard::set("unchanged");
        let mut e = TextInput::new("secret".into());
        e.select_all();
        assert!(e.key(
            KeyCode::KeyX,
            None,
            Modifiers {
                super_key: true,
                ..Default::default()
            },
            false,
            Rules {
                max: 200,
                secret: true
            },
            |_| true
        ));
        assert_eq!(e.text, "secret");
        assert_eq!(clipboard::get().as_deref(), Some("unchanged"));
        e.insert("12x345", 3, |c| c.is_ascii_digit());
        assert_eq!(e.text, "123");
    }
}
