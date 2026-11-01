//! The first steps: a few pages over everything on a new player's first launch, and any time from
//! Settings. Its pictures are drawn with the game's own parts, and the keys it names are the
//! player's own, so it can't fall out of date with the game.

use crate::config::{interface_key_name, key_name, Config};
use crate::game::{DON, KAT};
use crate::keys::Hotkey;
use crate::menu::{self, Item};
use crate::render::{accent, pixel_size, rgba, snap_to_grid, Canvas, Hit, VIRTUAL_HEIGHT};

pub const PAGES: usize = 7;
/// The page with the button that starts calibrating.
pub const OFFSET_PAGE: usize = 1;

/// What the guide's buttons do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Back,
    Next,
    Close,
    Calibrate,
    Page(usize),
    /// Wait for a new key for this drum (`Config::keys`).
    Bind(usize),
}

/// A key (or a few) and what it does.
type Row = (String, &'static str);

struct Page {
    title: &'static str,
    /// A line or two above the keys.
    intro: [&'static str; 2],
    rows: Vec<Row>,
}

/// A key's name as it's printed on the key.
fn printed(name: String) -> String {
    [("BracketLeft", "["), ("BracketRight", "]"), ("Backquote", "`"), ("Minus", "-"), ("Equal", "="), ("Escape", "Esc"), ("Backslash", "\\")]
        .into_iter()
        .fold(name, |n, (from, to)| n.replace(from, to))
}

/// What's bound to `h` now, or a dash.
fn key(cfg: &Config, h: Hotkey) -> String {
    cfg.hotkeys.get(h).map_or_else(|| tr!("no key", "なし", "нет").to_string(), |c| printed(c.name()))
}

fn both(cfg: &Config, a: Hotkey, b: Hotkey) -> String {
    format!("{} {}", key(cfg, a), key(cfg, b))
}

fn both_presets(cfg: &Config) -> String {
    format!("{} {}", key_name(cfg.preset_keys[0]), key_name(cfg.preset_keys[1]))
}

fn page(i: usize, cfg: &Config) -> Page {
    use Hotkey::*;
    let drum = |a: usize, b: usize| format!("{} {}", printed(key_name(cfg.keys[a])), printed(key_name(cfg.keys[b])));
    // Words on one cap (the name is split into caps at plain spaces).
    let cap = |s: &str| s.replace(' ', "\u{a0}");
    match i {
        0 => Page {
            title: tr!("Your drum", "太鼓のたたき方", "Твой барабан"),
            intro: [
                tr!("Red notes are don, the drum's face. Blue notes are kat, its rim.", "赤いノーツはドン (面)、青いノーツはカッ (ふち) です。", "Красные ноты: дон, центр барабана. Синие: кат, обод."),
                tr!("Each has two keys, so you can play with both hands in turn.", "それぞれキーが2つあり、左右交互に叩けます。", "На каждую по две клавиши, чтобы бить обеими руками по очереди."),
            ],
            rows: vec![
                (drum(1, 2), tr!("don, red notes", "ドン (赤)", "дон, красные ноты")),
                (drum(0, 3), tr!("kat, blue notes", "カッ (青)", "кат, синие ноты")),
                ("Ctrl+O".into(), tr!("Settings from anywhere: keys are in Input", "どこからでも設定へ。キーは入力の項目に", "настройки откуда угодно, клавиши в разделе Управление")),
                (key(cfg, Settings), tr!("songs and settings", "曲と設定を切り替え", "песни и настройки")),
                ("Esc".into(), tr!("back; from the main menu, it asks to quit", "戻る (メインメニューでは終了の確認)", "назад, а в главном меню спросит о выходе")),
                ("Alt+wheel".into(), tr!("volume, anywhere", "音量 (どこでも)", "громкость, где угодно")),
                (cap(tr!("mouse", "マウス", "мышь")), tr!("everything can be clicked too, the wheel scrolls", "すべてクリックでも操作できます", "всё можно нажимать мышью, колесо листает")),
            ],
        },
        1 => Page {
            title: tr!("Offset: the most important setting", "オフセット: いちばん大切な設定", "Оффсет (смещение): самая важная настройка"),
            intro: [
                tr!("If notes feel early or late, it's your sound and screen's delay, not you.", "ノーツがずれて感じるのは、音と画面の遅延のせいです。", "Если ноты кажутся ранними или поздними, виноваты задержки звука и экрана, а не ты."),
                tr!("Calibrating measures it: tap along with the beat for a moment.", "調整で測れます。しばらくリズムに合わせて叩くだけです。", "Калибровка измеряет это: просто постучи в такт."),
            ],
            rows: vec![
                ("C".into(), tr!("calibrate now (later: Settings > Offset)", "今すぐ調整 (あとで 設定 > オフセット)", "калибровать сейчас (потом: настройки > Смещение)")),
                (cap(tr!("Audio offset", "オーディオ", "Audio offset")), tr!("raise it if you hit late, lower it if you hit early", "遅いなら上げ、早いなら下げます", "бьёшь поздно: подними, рано: опусти")),
                (both(cfg, OffsetDown, OffsetUp), tr!("in a song: this map's own offset", "曲中: この譜面のオフセット", "в песне: оффсет только этой карты")),
                (both(cfg, VisualOffsetDown, VisualOffsetUp), tr!("draw the notes earlier or later, judging stays", "ノーツの表示だけを前後に動かします", "сдвиг картинки нот, судейство не меняется")),
                (cap(tr!("Results", "リザルト", "Итоги")), tr!("show how early or late you were on average", "平均して早いか遅いかを表示します", "показывают, насколько рано или поздно ты бил")),
                (cap(tr!("Suggestion", "提案", "Подсказка")), tr!("Settings > Offset can suggest an offset during a song", "設定 > オフセット で曲中に提案を表示できます", "настройки > Смещение: подскажет оффсет прямо в песне")),
                (cap(tr!("Reset", "リセット", "Сброс")), tr!("Settings > Offset clears every map's own offset", "設定 > オフセット で譜面ごとのオフセットを消せます", "настройки > Смещение: убрать оффсеты всех карт")),
            ],
        },
        2 => Page {
            title: tr!("Smooth picture", "なめらかな画面", "Плавная картинка"),
            intro: [
                tr!("Less delay means easier reading. With vsync on, calibrate its compensation too.", "遅延が少ないほど読みやすくなります。垂直同期なら補正も調整しましょう。", "Чем меньше задержка, тем проще читать ноты. С vsync откалибруй и его компенсацию."),
                tr!("All of it is in Settings > Video, and these change it on the spot.", "すべて 設定 > グラフィック にあり、次のキーでもすぐ変えられます。", "Всё есть в настройках > Графика, а эти клавиши меняют на лету."),
            ],
            rows: vec![
                (key(cfg, Fullscreen), tr!("fullscreen", "フルスクリーン", "полный экран")),
                (key(cfg, PresentMode), tr!("next present mode (vsync, mailbox, immediate)", "次の表示モード (垂直同期など)", "режим вывода (vsync, mailbox, immediate)")),
                (key(cfg, FpsCap), tr!("next FPS cap", "次のFPS上限", "ограничение FPS")),
                (key(cfg, FpsOverlay), tr!("show FPS and frame times", "FPSとフレーム時間を表示", "показать FPS и время кадра")),
                (key(cfg, Screenshot), tr!("screenshot", "スクリーンショット", "скриншот")),
                (cap(tr!("Vsync", "垂直同期", "Vsync")), tr!("Settings > Offset has its compensation and calibration", "補正と調整は 設定 > オフセット に", "компенсация и её калибровка: настройки > Смещение")),
                (cap(tr!("Half resolution", "半分の解像度", "Половинное")), tr!("Settings > Video: for a weak graphics card", "設定 > グラフィック: 非力なGPU向け", "настройки > Графика: для слабой видеокарты")),
            ],
        },
        3 => Page {
            title: tr!("Finding a song", "曲を探す", "Как найти песню"),
            intro: [
                tr!("Just start typing to search. Filters work too: stars>5 length<2:30 creator=name", "そのまま入力すると検索できます。stars>5 length<2:30 なども使えます。", "Просто начни печатать. Работают фильтры: stars>5 length<2:30 creator=name"),
                tr!("No maps yet? Download finds them right inside the game.", "譜面がなければ、ゲーム内のダウンロードで探せます。", "Нет карт? Вкладка загрузки найдёт их прямо в игре."),
            ],
            rows: vec![
                ("Enter".into(), tr!("play", "プレイ", "играть")),
                (key(cfg, Random), tr!("random song (Shift: back)", "ランダム (Shift: 戻る)", "случайная песня (Shift: назад)")),
                (key(cfg, Download), tr!("download maps", "譜面をダウンロード", "скачать карты")),
                (both(cfg, Sort, Collections), tr!("sort, collections", "並べ替え、コレクション", "сортировка, коллекции")),
                (key(cfg, Preview), tr!("watch autoplay", "オートのプレビュー", "посмотреть автоплей")),
                (key(cfg, PlayFrom), tr!("play from the flag on the timeline", "タイムラインの旗からプレイ", "играть с флажка на таймлайне")),
                (both(cfg, SpeedDown, SpeedUp), tr!("speed down and up", "速度を下げる / 上げる", "скорость ниже и выше")),
            ],
        },
        4 => Page {
            title: tr!("In a song", "プレイ中", "Во время песни"),
            intro: [
                tr!("Scroll speed is yours to choose: many read a faster lane more easily.", "スクロール速度は自由に変えられます。", "Скорость прокрутки выбираешь сам: многим быстрая дорожка читается легче."),
                tr!("After a song, look at the results: early or late, your UR, pp.", "曲のあとはリザルトで早い・遅い、UR、ppを確認できます。", "После песни смотри итоги: рано или поздно, UR, pp."),
            ],
            rows: vec![
                (key(cfg, Pause), tr!("pause", "ポーズ", "пауза")),
                (key(cfg, SkipIntro), tr!("skip the intro", "イントロを飛ばす", "пропустить вступление")),
                (format!("{} / {}", key(cfg, QuickRetry), key(cfg, Restart)), tr!("restart", "最初から", "заново")),
                (both(cfg, ScrollSlower, ScrollFaster), tr!("scroll speed", "スクロール速度", "скорость прокрутки")),
                (key(cfg, MapInfo), tr!("offset and UR line", "オフセットとURの行", "строка оффсета и UR")),
                (interface_key_name(cfg.interface_key), tr!("hide the interface", "インターフェースを隠す", "скрыть интерфейс")),
                (format!("{} {}", key(cfg, WatchReplay), key(cfg, ExportVideo)), tr!("in results: watch the replay, make a video", "リザルト: リプレイ、動画", "в итогах: смотреть повтор, сделать видео")),
            ],
        },
        5 => Page {
            title: tr!("Mods and playing with others", "Modとみんなで遊ぶ", "Моды и игра с другими"),
            intro: [
                tr!("Mods go far beyond osu!'s: try the ones that bend the whole playfield.", "Modはosu!より豊富です。画面全体が変わるものも試してみましょう。", "Модов намного больше, чем в osu!: попробуй те, что ломают всё поле."),
                tr!("Log in from Multiplayer to climb the ranks and save scores online.", "マルチプレイからログインすると、ランキングに参加できます。", "Войди через мультиплеер, чтобы попасть в рейтинг и хранить результаты онлайн."),
            ],
            rows: vec![
                (key(cfg, Mods), tr!("mods; a right click makes one a favourite", "Mod (右クリックでお気に入り)", "моды, правый клик: в избранное")),
                ("C".into(), tr!("in mods: customise the mods that can be", "Mod画面: Modをカスタマイズ", "в модах: настроить мод, если он настраивается")),
                (key(cfg, Friends), tr!("friends and chat", "フレンドとチャット", "друзья и чат")),
                (cap(tr!("Multiplayer", "マルチ", "Мультиплеер")), tr!("rooms to play together, spectate, chat", "ルームで一緒に遊ぶ、観戦、チャット", "комнаты: игра вместе, наблюдение, чат")),
                (cap("Discord"), tr!("our server, on the main menu", "メインメニューから公式サーバーへ", "наш сервер, в главном меню")),
            ],
        },
        _ => Page {
            title: tr!("Make it yours", "自分好みに", "Настрой под себя"),
            intro: [
                tr!("taiko.rs is a music player too: the disc button in Now playing opens it.", "taiko.rsは音楽プレーヤーにもなります。再生中のディスクボタンから。", "taiko.rs ещё и музыкальный плеер: кнопка с диском в блоке Сейчас играет."),
                tr!("Skins, note colours, the interface colour and the playfield are in Settings.", "スキン、ノーツの色、UIの色、プレイフィールドは設定にあります。", "Скины, цвета нот, цвет интерфейса и игровое поле есть в настройках."),
            ],
            rows: vec![
                ("L".into(), tr!("in the player: love the song", "プレーヤー: お気に入り", "в плеере: в любимые")),
                ("Tab".into(), tr!("in the player: genre folders and favourites", "プレーヤー: ジャンルとお気に入り", "в плеере: папки жанров и любимые")),
                ("[ ]".into(), tr!("in the player: move the pitch, DT NC HT DC on the buttons", "プレーヤー: ピッチを変える", "в плеере: питч, а DT NC HT DC на кнопках")),
                (key(cfg, Update), tr!("update the game", "アップデート", "обновить игру")),
                (key(cfg, Quit), tr!("quit", "終了", "выйти")),
                (both_presets(cfg), tr!("previous and next playfield preset", "プレイフィールドのプリセット", "пресеты игрового поля")),
                (cap(tr!("Keys", "キー", "Клавиши")), tr!("every key in this guide can be moved: Settings > Input", "このガイドのキーはすべて変更できます (入力)", "любую клавишу отсюда можно поменять: настройки > Управление")),
                (cap(tr!("This guide", "このガイド", "Этот гид")), tr!("is always in Settings > Video", "いつでも 設定 > グラフィック から", "всегда есть в настройках > Графика")),
            ],
        },
    }
}

/// A key on a little keycap; returns its width.
fn keycap(c: &mut Canvas, x: f32, y: f32, label: &str, fill: u32) -> f32 {
    let w = Canvas::text_width(8.0, label) + 12.0;
    c.round_rect([x, y], [w, 17.0], 3.0, rgba(fill, 1.0), rgba(crate::menu::tint(fill, 0x000000, 0.4), 1.0), 1.0);
    c.rect(x + 2.0, y + 16.0, w - 4.0, 1.0, rgba(0x000000, 0.4));
    c.text(x + 6.0, y + 4.5, 8.0, rgba(0xffffff, 1.0), label);
    w
}

/// The page's picture, in the band `y`..`y + PICTURE_H`.
fn draw_picture(c: &mut Canvas, i: usize, cx: f32, y: f32, w: f32, binding: Option<usize>, cfg: &Config) {
    let mid = y + PICTURE_H / 2.0;
    let px = pixel_size(1.0);
    c.round_rect([cx - w / 2.0, y], [w, PICTURE_H], 4.0, rgba(0x121219, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
    match i {
        0 => {
            // The drum, split like the keys: rim, face, face, rim. The keys are live: a click
            // waits for a new one.
            let (cy, r) = (y + 42.0, 34.0);
            menu::pixel_ring(c, [cx, cy], r, r, 0, px, rgba(0x23232f, 1.0));
            menu::pixel_ring(c, [cx, cy], r, 11.0, 0, px, rgba(KAT, 1.0));
            menu::pixel_ring(c, [cx, cy], 22.0, 22.0, 0, px, rgba(DON, 1.0));
            c.rect(cx - 0.5, cy - r, 1.0, r * 2.0, rgba(0xffffff, 0.35));
            let names: [String; 4] = std::array::from_fn(|slot| if binding == Some(slot) { "?".to_string() } else { printed(key_name(cfg.keys[slot])) });
            let width = |slot: usize| Canvas::text_width(8.0, &names[slot]) + 12.0;
            let at = [cx - r - 16.0 - width(1) - 8.0 - width(0), cx - r - 16.0 - width(1), cx + r + 16.0, cx + r + 16.0 + width(2) + 8.0];
            for (slot, x) in at.into_iter().enumerate() {
                let colour = if slot == 0 || slot == 3 { KAT } else { DON };
                let hover = c.hovered(x - 3.0, cy - 12.0, width(slot) + 6.0, 24.0);
                let fill = if binding == Some(slot) { 0xffd34d } else if hover { crate::menu::tint(colour, 0xffffff, 0.2) } else { colour };
                keycap(c, x, cy - 8.0, &names[slot], fill);
                c.hit(x - 3.0, cy - 12.0, width(slot) + 6.0, 24.0, Hit::Guide(Action::Bind(slot)));
            }
            let name = binding.map(|slot| names[slot].clone()).map_or(tr!("Click a key to change it", "キーをクリックして変更", "Нажми на клавишу, чтобы её сменить"), |_| tr!("Press the new key, Esc to keep the old one", "新しいキーを押してください (Esc: そのまま)", "Нажми новую клавишу, Esc: оставить старую"));
            c.text_centered(cx, y + PICTURE_H - 15.0, 8.0, rgba(if binding.is_some() { 0xffd34d } else { 0x9a9ab0 }, 1.0), name);
        }
        1 => {
            // The hit error bar: where hits land against the middle.
            let half = (w / 2.0 - 56.0).min(150.0);
            c.rect(cx - half, mid - 2.0, half * 2.0, 4.0, rgba(0x3a3a4e, 1.0));
            c.rect(cx - half * 0.3, mid - 2.0, half * 0.6, 4.0, rgba(0x5ee6c8, 1.0));
            c.rect(cx - 1.0, mid - 16.0, 2.0, 32.0, rgba(0xffffff, 1.0));
            for (n, spread) in [0.42f32, 0.5, 0.36, 0.56, 0.47, 0.6, 0.39].into_iter().enumerate() {
                c.rect(snap_to_grid(cx + half * spread), mid - 9.0 + n as f32 * 2.0, 2.0, 6.0, rgba(0xffd34d, 1.0));
            }
            c.text_right(cx - half - 8.0, mid - 3.5, 7.0, rgba(0x9a9ab0, 1.0), tr!("early", "早い", "рано"));
            c.text(cx + half + 8.0, mid - 3.5, 7.0, rgba(0xffd34d, 1.0), tr!("late", "遅い", "поздно"));
        }
        2 => {
            // Frame times: an even row of bars, and what a hitch looks like.
            let bars = ((w - 40.0) / 6.0) as usize;
            let left = cx - bars as f32 * 3.0;
            for b in 0..bars {
                let spike = b % 37 == 20;
                let h = if spike { 44.0 } else { 12.0 + (b as f32 * 1.7).sin().abs() * 4.0 };
                c.rect(left + b as f32 * 6.0, y + PICTURE_H - 10.0 - h, 4.0, h, rgba(if spike { DON } else { 0x86e05a }, 1.0));
            }
        }
        3 => {
            // Rows of a song list, the one picked lit.
            for (n, label) in ["Artist - Song [Kantan]", "Artist - Song [Oni]", "Artist - Song [Inner Oni]"].into_iter().enumerate() {
                let (row, on) = (y + 18.0 + n as f32 * 26.0, n == 1);
                let bw = (w - 60.0).min(300.0);
                c.round_rect([cx - bw / 2.0, row], [bw, 22.0], 4.0, rgba(if on { 0x23232f } else { 0x1a1a24 }, 1.0), rgba(if on { accent() } else { 0x3a3a4e }, 1.0), 1.0);
                c.text(cx - bw / 2.0 + 10.0, row + 7.5, 7.0, rgba(if on { 0xffffff } else { 0x9a9ab0 }, 1.0), &Canvas::fit(7.0, bw - 76.0, label));
                c.text_right(cx + bw / 2.0 - 10.0, row + 7.5, 7.0, rgba(0xffd34d, 1.0), ["1.82*", "4.96*", "6.31*"][n]);
            }
        }
        4 => {
            // A stretch of the lane, the notes riding to the hit circle.
            let (left, right) = (cx - w / 2.0 + 10.0, cx + w / 2.0 - 10.0);
            c.rect(left, mid - 26.0, right - left, 52.0, rgba(0x15151d, 1.0));
            let circle = left + 44.0;
            let px = pixel_size(1.0);
            menu::pixel_ring(c, [circle, mid], 22.0, px * 2.0, 0, px, rgba(0xffffff, 0.6));
            let mut x = circle + 62.0;
            let mut n = 0;
            while x < right - 18.0 {
                let kat = [false, false, true, false, true, true][n % 6];
                menu::pixel_ring(c, [x, mid], 17.0, 17.0, 0, px, rgba(0xffffff, 1.0));
                menu::pixel_ring(c, [x, mid], 14.0, 14.0, 0, px, rgba(if kat { KAT } else { DON }, 1.0));
                x += 52.0;
                n += 1;
            }
        }
        5 => {
            // A row of mods, and two players.
            let chips = [("HD", 0xffd34d), ("HR", 0xff5fa2), ("DT", 0x4fb4ff), ("FL", 0x8a8aa0), ("Shatter", 0xc77dff), ("Multiverse", 0x5ee6c8)];
            let total: f32 = chips.iter().map(|m| Canvas::text_width(8.0, m.0) + 18.0).sum::<f32>() + 6.0 * 5.0;
            let mut x = cx - total / 2.0;
            for (name, colour) in chips {
                let bw = Canvas::text_width(8.0, name) + 18.0;
                c.round_rect([x, mid - 12.0], [bw, 24.0], 5.0, rgba(crate::menu::tint(colour, 0x000000, 0.65), 1.0), rgba(colour, 1.0), 1.5);
                c.text_centered(x + bw / 2.0, mid - 4.0, 8.0, rgba(0xffffff, 1.0), name);
                x += bw + 6.0;
            }
        }
        _ => {
            // The player: its notes and a bit of spectrum.
            let items = [Item::Play, Item::Multiplayer, Item::Settings];
            for (n, item) in items.into_iter().enumerate() {
                menu::note(c, item, cx - 92.0 + n as f32 * 56.0, mid, 24.0);
            }
            for b in 0..16 {
                let h = 14.0 + ((b as f32 * 1.3).sin().abs() * 34.0);
                c.rect(cx + 44.0 + b as f32 * 5.0, mid + 24.0 - h, 3.0, h, rgba(accent(), 1.0));
            }
        }
    }
}

const PICTURE_H: f32 = 108.0;

/// Page `i` of the guide, over everything.
pub fn draw(c: &mut Canvas, i: usize, binding: Option<usize>, cfg: &Config) {
    let p = page(i, cfg);
    c.rect(0.0, 0.0, c.width, VIRTUAL_HEIGHT, rgba(0x000000, 0.7));
    c.hit(0.0, 0.0, c.width, VIRTUAL_HEIGHT, Hit::Overlay);
    let w = 600.0f32.min(c.width - 24.0);
    let (x, y, h) = ((c.width - w) / 2.0, 14.0, 452.0);
    let cx = c.width / 2.0;
    c.round_rect([x, y], [w, h], 8.0, rgba(0x1d1d27, 1.0), rgba(accent(), 1.0), 1.5);
    c.text_centered(cx, y + 12.0, 13.0, rgba(0xffffff, 1.0), &Canvas::fit(13.0, w - 32.0, p.title));
    draw_picture(c, i, cx, y + 36.0, w - 32.0, binding, cfg);
    let mut row = y + 36.0 + PICTURE_H + 9.0;
    for line in p.intro {
        c.text(x + 16.0, row, 8.0, rgba(0xc8c8d8, 1.0), &Canvas::fit(8.0, w - 32.0, line));
        row += 15.0;
    }
    row += 4.0;
    const KEYS_W: f32 = 130.0;
    for (keys, what) in &p.rows {
        let mut kx = x + 16.0;
        for name in keys.split(' ').filter(|k| !k.is_empty()) {
            kx += if name == "/" { c.text(kx, row + 4.5, 8.0, rgba(0x9a9ab0, 1.0), name) + 4.0 } else { keycap(c, kx, row, name, 0x34344a) + 3.0 };
        }
        let tx = (x + 16.0 + KEYS_W).max(kx + 6.0);
        c.text(tx, row + 4.5, 8.0, rgba(0xffffff, 1.0), &Canvas::fit(8.0, x + w - 16.0 - tx, what));
        row += 21.0;
    }

    // The way through: dots, back and on.
    let bottom = y + h - 32.0;
    const DOT: f32 = 12.0;
    let dots_x = cx - PAGES as f32 * DOT / 2.0;
    for d in 0..PAGES {
        let dx = dots_x + d as f32 * DOT;
        let size = pixel_size(6.0);
        c.rect(snap_to_grid(dx + 3.0), snap_to_grid(bottom - 13.0), size, size, rgba(if d == i { accent() } else { 0x3a3a4e }, 1.0));
        c.hit(dx, bottom - 18.0, DOT, 16.0, Hit::Guide(Action::Page(d)));
    }
    let bw = 112.0f32.min((w - 56.0) / 3.0);
    let last = i + 1 == PAGES;
    if i > 0 {
        c.button(x + 16.0, bottom, bw, 22.0, tr!("< Back", "< 戻る", "< Назад"), 8.0, Hit::Guide(Action::Back), false);
    } else {
        c.button(x + 16.0, bottom, bw, 22.0, tr!("Skip (Esc)", "閉じる (Esc)", "Пропустить (Esc)"), 8.0, Hit::Guide(Action::Close), false);
    }
    if i == OFFSET_PAGE {
        c.button(cx - bw / 2.0, bottom, bw, 22.0, tr!("Calibrate (C)", "調整 (C)", "Калибровать (C)"), 8.0, Hit::Guide(Action::Calibrate), true);
    }
    let next = if last { tr!("Done", "完了", "Готово") } else { tr!("Next >", "次へ >", "Далее >") };
    c.button(x + w - 16.0 - bw, bottom, bw, 22.0, next, 8.0, Hit::Guide(if last { Action::Close } else { Action::Next }), true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_names_the_players_own_keys() {
        let mut cfg = Config::default();
        cfg.hotkeys.set(Hotkey::Random, Some(crate::keys::Chord::plain(winit::keyboard::KeyCode::KeyX)));
        let keys: Vec<String> = page(3, &cfg).rows.into_iter().map(|r| r.0).collect();
        assert!(keys.contains(&"X".to_string()), "{keys:?}");
    }

    #[test]
    fn every_page_says_something() {
        let cfg = Config::default();
        for i in 0..PAGES {
            let p = page(i, &cfg);
            assert!(!p.title.is_empty() && !p.rows.is_empty(), "page {i}");
            assert!(p.rows.len() <= 8, "page {i} has no room for {} rows", p.rows.len());
        }
    }
}
