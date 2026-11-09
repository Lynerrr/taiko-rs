//! What the song select draws around the map's preview: its chrome and the analysis panel.

#[allow(unused_imports)]
use crate::*;

/// The maps being analysed, top right: how far it's got. In a song it waits, and this isn't shown.
pub(crate) fn draw_analysis(c: &mut Canvas, done: usize, total: usize) {
    const BAR: f32 = 54.0;
    let text = tr!("analysing maps {done} / {total}", "譜面を分析中 {done} / {total}", "анализ карт {done} / {total}";);
    let text = Canvas::fit(6.0, (c.width - BAR - 64.0).max(40.0), &text);
    let tw = Canvas::text_width(6.0, &text);
    let w = tw + BAR + 20.0;
    let (x, y) = (c.width - w - 8.0, 4.0);
    c.round_rect(
        [x, y],
        [w, 13.0],
        4.0,
        rgba(0x101018, 0.82),
        rgba(0x3a3a4e, 1.0),
        1.0,
    );
    c.text(x + 7.0, y + 3.5, 6.0, rgba(0xc8c8d8, 1.0), &text);
    let (bx, by) = (x + tw + 13.0, y + 5.0);
    c.round_rect(
        [bx, by],
        [BAR, 3.0],
        1.5,
        rgba(0x2a2a38, 1.0),
        rgba(0, 0.0),
        0.0,
    );
    let part = done as f32 / total.max(1) as f32;
    if part > 0.0 {
        c.round_rect(
            [bx, by],
            [(BAR * part).max(3.0), 3.0],
            1.5,
            rgba(render::accent(), 1.0),
            rgba(0, 0.0),
            0.0,
        );
    }
}

/// Marks the fullscreen preview as a preview: a blue frame and badge on the playfield area, and
/// a header with the map and the controls.
pub(crate) fn draw_preview_chrome(
    c: &mut Canvas,
    cfg: &Config,
    e: &Entry,
    stars: Option<f32>,
    lobby: bool,
    size_label: &str,
) {
    let blue = preview::MARKER;
    let a = cfg.playfield_area;
    let (ax, ay, aw, ah) = (
        a.x * c.width,
        a.y * render::VIRTUAL_HEIGHT,
        a.w * c.width,
        a.h * render::VIRTUAL_HEIGHT,
    );
    c.round_rect(
        [ax + 1.0, ay + 1.0],
        [aw - 2.0, ah - 2.0],
        4.0,
        rgba(0, 0.0),
        rgba(blue, 0.55),
        1.5,
    );
    let badge = tr!("AUTOPLAY PREVIEW", "オートのプレビュー", "ПРЕВЬЮ АВТОПЛЕЯ");
    let bw = Canvas::text_width(7.0, badge) + 16.0;
    // Below the header when the area reaches the top of the screen.
    let by = (ay + 8.0).max(52.0);
    c.round_rect(
        [ax + 8.0, by],
        [bw, 16.0],
        8.0,
        rgba(blue, 0.85),
        rgba(0, 0.0),
        0.0,
    );
    c.text(ax + 16.0, by + 4.5, 7.0, rgba(0x0c1a28, 1.0), badge);

    // A phone's bar: the way back, the map and its stars.
    if mobile::active() {
        let back = if lobby {
            tr!("< LOBBY", "< ロビー", "< ЛОББИ")
        } else {
            tr!("< BACK", "< 戻る", "< НАЗАД")
        };
        settings::back_button(c, 0.0, back, hit::Hit::Key(KeyCode::Escape));
        let pill = crate::style::star_pill(c, c.width, 14.0, 12.0, stars);
        let title = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
        let room = c.width - 112.0 - pill - 12.0;
        c.text(
            112.0,
            16.0,
            12.0,
            rgba(0xffffff, 1.0),
            &Canvas::fit(12.0, room, &title),
        );
        return;
    }
    c.round_rect(
        [24.0, 10.0],
        [c.width - 48.0, 34.0],
        8.0,
        rgba(0x15151d, 0.9),
        rgba(0x2a2a38, 1.0),
        1.0,
    );
    let tag = tr!("PREVIEW", "プレビュー", "ПРЕВЬЮ");
    let tag_w = Canvas::text_width(8.0, tag) + 18.0;
    c.round_rect(
        [32.0, 18.0],
        [tag_w, 18.0],
        9.0,
        rgba(0x173049, 1.0),
        rgba(blue, 1.0),
        1.0,
    );
    c.text(41.0, 23.0, 8.0, rgba(0xffffff, 1.0), tag);
    let pill = crate::style::star_pill(c, c.width - 36.0, 22.0, 8.0, stars);
    let back = if lobby {
        tr!("Lobby  (Esc)", "ロビー  (Esc)", "Лобби  (Esc)")
    } else {
        tr!("Back  (Esc)", "戻る  (Esc)", "Назад  (Esc)")
    };
    let bw = Canvas::text_width(8.0, back) + 24.0;
    let bx = c.width - 44.0 - pill - bw;
    c.button(
        bx,
        17.0,
        bw,
        20.0,
        back,
        8.0,
        hit::Hit::Key(KeyCode::Escape),
        false,
    );
    // Right to left: the size button, the hints and the title share what's left. On a narrow
    // screen the hints go first, then the size button gets shorter, so the title keeps TITLE_MIN.
    const TITLE_MIN: f32 = 120.0;
    let title_x = 32.0 + tag_w + 10.0;
    let short_size = size_label.split(" (").next().unwrap_or(size_label);
    let presets = [
        format!("{size_label}  (F5/F6)"),
        size_label.to_string(),
        short_size.to_string(),
    ]
    .into_iter()
    .find(|p| bx - 8.0 - (Canvas::text_width(8.0, p) + 24.0) - title_x - 12.0 >= TITLE_MIN)
    .unwrap_or_else(|| short_size.to_string());
    let pw = Canvas::text_width(8.0, &presets) + 24.0;
    let px = bx - 8.0 - pw;
    c.button(px, 17.0, pw, 20.0, &presets, 8.0, hit::Hit::Presets, false);
    let hints = if lobby {
        tr!(
            "drag / wheel: playfield",
            "ドラッグ / ホイール: プレイフィールド",
            "тяните / колесо: поле"
        )
    } else {
        tr!(
            "Enter play   drag / wheel: playfield",
            "Enter プレイ   ドラッグ / ホイール: プレイフィールド",
            "Enter играть   тяните / колесо: поле"
        )
    };
    let mut title_end = px - 12.0;
    if title_end - Canvas::text_width(7.0, hints) - 12.0 - title_x >= TITLE_MIN + 60.0 {
        c.text_right(title_end, 24.0, 7.0, rgba(0x8a8aa0, 1.0), hints);
        title_end -= Canvas::text_width(7.0, hints) + 12.0;
    }
    let title = format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version);
    c.text(
        title_x,
        22.0,
        10.0,
        rgba(0xffffff, 1.0),
        &Canvas::fit(10.0, title_end - title_x, &title),
    );
}
