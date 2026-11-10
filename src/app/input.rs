//! Input the app reads before any screen: keys the input method owns, and how far a hidden
//! pointer has to move to show again.

use winit::keyboard::{Key, NamedKey};

/// Whether a key press belongs to the input method (Japanese, Chinese, Korean typing) rather than
/// the game: while a word is being composed (`preedit` shows it), Enter takes the word, Tab and the
/// arrows go through its suggestions, Backspace and Esc edit or drop it. Windows marks such keys
/// `Process` even when the word was just taken and `preedit` already cleared.
pub(crate) fn ime_has_key(logical: &Key, preedit: &str) -> bool {
    *logical == Key::Named(NamedKey::Process) || !preedit.is_empty()
}

/// How far (window pixels) a pointer that gameplay hid has to travel to show again: about 1% of
/// the window's height, so desk vibration or a jittery sensor doesn't keep bringing it back.
pub(crate) fn pointer_wake_px(window_h: f32) -> f64 {
    (window_h as f64 * 0.01).max(6.0)
}

#[cfg(test)]
mod ime_key_tests {
    use super::*;

    #[test]
    fn keys_go_to_the_input_method_while_a_word_is_composed() {
        let enter = Key::Named(NamedKey::Enter);
        let tab = Key::Named(NamedKey::Tab);
        // Nothing composed: Enter and Tab are the game's (next field, submit).
        assert!(!ime_has_key(&enter, ""));
        assert!(!ime_has_key(&tab, ""));
        // A word on its way: Enter takes it and Tab picks a suggestion, in the input method.
        assert!(ime_has_key(&enter, "にほ"));
        assert!(ime_has_key(&tab, "にほ"));
        // Windows says so itself, also for the Enter that has just taken the word.
        assert!(ime_has_key(&Key::Named(NamedKey::Process), ""));
    }
}

#[cfg(test)]
mod pointer_wake_tests {
    use super::*;

    #[test]
    fn a_hidden_pointer_needs_more_than_a_twitch_to_show() {
        // 1080p: a few pixels of desk vibration stay under it, a real move gets past it.
        let wake = pointer_wake_px(1080.0);
        assert!(wake > 3.0f64.hypot(3.0));
        assert!(wake < 20.0);
        // Bigger windows need a bigger move, small ones still ignore single-pixel noise.
        assert!(pointer_wake_px(2160.0) > wake);
        assert!(pointer_wake_px(300.0) >= 6.0);
    }
}
