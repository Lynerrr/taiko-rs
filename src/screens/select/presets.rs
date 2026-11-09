//! Mod presets in the song select: naming one, deleting one, and their keys.

#[allow(unused_imports)]
use crate::*;

/// The mod preset (0-8) a digit key 1-9 (top row or keypad) picks in the mod select.
pub(crate) fn preset_key(code: KeyCode) -> Option<usize> {
    use KeyCode::*;
    [
        (Digit1, Numpad1),
        (Digit2, Numpad2),
        (Digit3, Numpad3),
        (Digit4, Numpad4),
        (Digit5, Numpad5),
        (Digit6, Numpad6),
        (Digit7, Numpad7),
        (Digit8, Numpad8),
        (Digit9, Numpad9),
    ]
    .iter()
    .position(|&(a, b)| code == a || code == b)
}

/// Over the mod select: is preset `k` (named `name`) to be deleted?
pub(crate) fn draw_preset_deleting(c: &mut Canvas, k: usize, name: &str) {
    c.hits.clear();
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
    let bw = (w - 32.0).min(460.0);
    let (bx, inner) = ((w - bw) / 2.0, bw - 32.0);
    c.round_rect(
        [bx, 140.0],
        [bw, 164.0],
        10.0,
        rgba(0x15151d, 0.98),
        rgba(0xff6b6b, 0.8),
        1.5,
    );
    let title = format!("Delete preset {}?", k + 1);
    c.text_centered(
        w / 2.0,
        156.0,
        14.0,
        rgba(0xffffff, 1.0),
        &Canvas::fit(14.0, inner, &title),
    );
    let size = Canvas::fit_size(12.0, 7.0, inner, name);
    c.text_centered(
        w / 2.0,
        196.0,
        size,
        rgba(0xffffff, 1.0),
        &Canvas::fit(size, inner, name),
    );
    c.text_centered(
        w / 2.0,
        230.0,
        7.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(
            7.0,
            inner,
            "It can't be brought back. The mods on now stay on. Enter deletes, Esc keeps it.",
        ),
    );
    c.button(
        w / 2.0 - 100.0,
        264.0,
        90.0,
        24.0,
        "Delete",
        9.0,
        hit::Hit::Key(KeyCode::Enter),
        true,
    );
    c.button(
        w / 2.0 + 10.0,
        264.0,
        90.0,
        24.0,
        "Keep",
        9.0,
        hit::Hit::Key(KeyCode::Escape),
        false,
    );
}

/// Over the mod select: the name a new mod preset gets, being typed.
pub(crate) fn draw_preset_naming(c: &mut Canvas, name: &str) {
    c.hits.clear();
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
    let bw = (w - 32.0).min(460.0);
    let (bx, inner) = ((w - bw) / 2.0, bw - 32.0);
    c.round_rect(
        [bx, 140.0],
        [bw, 164.0],
        10.0,
        rgba(0x15151d, 0.98),
        rgba(accent(), 0.8),
        1.5,
    );
    c.text_centered(
        w / 2.0,
        156.0,
        14.0,
        rgba(0xffffff, 1.0),
        &Canvas::fit(14.0, inner, "Name this preset"),
    );
    let typed = format!("{name}_");
    let size = Canvas::fit_size(12.0, 7.0, inner, &typed);
    let typed = Canvas::fit(size, inner, &typed);
    c.text_centered(w / 2.0, 196.0, size, rgba(0xffffff, 1.0), &typed);
    c.text_centered(
        w / 2.0,
        230.0,
        7.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(
            7.0,
            inner,
            "It keeps the mods on now and the speed: 1-9 or a click puts them back.",
        ),
    );
    c.button(
        w / 2.0 - 100.0,
        264.0,
        90.0,
        24.0,
        "Save",
        9.0,
        hit::Hit::Key(KeyCode::Enter),
        true,
    );
    c.button(
        w / 2.0 + 10.0,
        264.0,
        90.0,
        24.0,
        "Cancel",
        9.0,
        hit::Hit::Key(KeyCode::Escape),
        false,
    );
}
