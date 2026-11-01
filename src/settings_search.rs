//! Finding a setting by what it's called, in any of the game's languages whichever one is on, by
//! what people call it instead, by what it says it does, or by the key it's on.

use std::sync::OnceLock;

use crate::lang::{self, Lang};
use crate::settings::{section_name, Item, Kind, ITEMS, SECTIONS};

/// What a setting can be found by, lower case, in all the languages at once.
struct Entry {
    label: String,
    words: String,
    section: String,
    help: String,
}

/// Other words for a setting, for the ones its name doesn't use.
fn words(item: Item) -> &'static str {
    match item {
        Item::ScrollSpeed => "sv speed lane fast slow скорость нот быстро медленно",
        Item::Rate => "dt ht nightcore double half time ускорение замедление",
        Item::RateKeepPitch => "nightcore daycore тон высота",
        Item::Offset | Item::VisualOffset | Item::Calibrate => "latency delay sync timing late early задержка рассинхрон тайминг опаздывает раньше позже",
        Item::ResetMapOffsets | Item::LiveOffset | Item::LiveOffsetUr => "latency timing задержка тайминг",
        Item::VsyncCompensation | Item::VsyncTune | Item::VsyncCalibrate => "latency lag input delay задержка лаг",
        Item::AudioBuffer => "latency lag crackle stutter wasapi задержка лаг треск заикание",
        Item::MasterVolume | Item::MusicVolume | Item::FxVolume => "sound loud quiet mute audio звук громко тихо мьют音",
        Item::MessageSound => "ping chat звук пинг",
        Item::BackgroundDim | Item::BackgroundDimBlack => "background dark brightness фон темнее яркость",
        Item::KiaiEffects | Item::HitFlash | Item::HideGreat => "effects flash эффекты вспышки",
        Item::Animations => "effects motion эффекты движение",
        Item::PresentMode | Item::MenuVsync => "vsync mailbox immediate fifo tearing фреймрейт разрывы",
        Item::FpsCap => "fps framerate limit lag фпс кадры лимит",
        Item::ShowFps => "fps framerate counter фпс кадры счетчик",
        Item::HalfResolution => "performance weak gpu lag производительность слабый лаги",
        Item::Fullscreen | Item::WindowSize => "resolution borderless windowed разрешение окно",
        Item::PlayfieldArea | Item::PlayfieldPreset | Item::PlayfieldReset | Item::ShowPlayfieldSize | Item::WheelResize => "size position resolution tablet размер позиция разрешение",
        Item::Skin | Item::SkinParts | Item::SkinFolder => "theme notes look тема внешний вид ноты",
        Item::Accent | Item::AccentHue => "theme color hue тема цвет оттенок",
        Item::NoteColours | Item::DonColour | Item::KatColour => "color palette red blue цвет палитра красный синий",
        Item::Language => "english japanese russian locale английский японский русский",
        Item::SongsFolder | Item::ShowConverts | Item::HiddenMaps => "library beatmaps maps osu stable библиотека карты песни",
        Item::DataFolder => "save scores replays path сохранения рекорды реплеи путь",
        Item::Screenshots => "picture clipboard screen картинка буфер экран",
        Item::ChatFilter => "swear profanity мат цензура",
        Item::NotifyMessages | Item::NotifyMentions | Item::NotifyRequests | Item::NotifyOnline | Item::NotifyPlaying => "notification ping friends оповещения пинг друзья",
        Item::DiscordPresence => "rich presence статус",
        Item::Update | Item::UpdateChannel | Item::CheckUpdates | Item::Changelog => "version release dev latest версия релиз новое",
        Item::Splash | Item::MainMenu | Item::Guide => "intro startup tutorial заставка запуск обучение",
        Item::Interface | Item::InterfaceHides | Item::InterfaceKey => "hud ui hide скрыть худ",
        Item::AutoplayStyle | Item::AutoplayBinding | Item::AutoplayLeftHanded => "auto bot кдд авто бот",
        Item::TouchLayout => "phone android tablet телефон андроид планшет",
        Item::Key(_) | Item::PresetKey(_) | Item::Hotkey(_) => "keybind binding shortcut hotkey бинд клавиша кнопка",
        _ => "",
    }
}

fn lower(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).map(|c| if c == 'ё' { 'е' } else { c }).collect()
}

fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        ITEMS
            .iter()
            .map(|&(section, item)| {
                let s = SECTIONS.iter().position(|(name, _)| *name == section).unwrap_or(0);
                let all = |text: &dyn Fn() -> String| Lang::ALL.map(|l| lang::with(l, text)).join("\n");
                Entry {
                    label: lower(&all(&|| item.label().to_string())),
                    words: lower(words(item)),
                    section: lower(&all(&|| section_name(s).to_string())),
                    help: lower(&all(&|| item.help().to_string())),
                }
            })
            .collect()
    })
}

/// How well `word` is found in `text`: from its start 0, from a word's start 1, inside a word 2.
fn found(text: &str, word: &str) -> Option<u32> {
    let mut best = None;
    for (at, _) in text.match_indices(word) {
        let tier = match text[..at].chars().next_back() {
            None | Some('\n') => 0,
            Some(c) if !c.is_alphanumeric() => 1,
            _ => 2,
        };
        best = Some(best.map_or(tier, |b: u32| b.min(tier)));
    }
    best
}

/// How far down the list a setting goes for `word`; None if it doesn't have it.
fn rank(entry: &Entry, key: &str, word: &str) -> Option<u32> {
    found(&entry.label, word)
        .or_else(|| (!key.is_empty() && key.split('+').any(|part| part == word)).then_some(2))
        .or_else(|| found(&entry.words, word).map(|tier| 3 + tier.min(1)))
        .or_else(|| found(&entry.section, word).map(|_| 5))
        .or_else(|| found(&entry.help, word).map(|_| 6))
}

/// The same keys pressed on the other keyboard layout (`pderr` for `звук`).
fn other_layout(text: &str) -> String {
    const EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
    const RU: &str = "йцукенгшщзхъфывапролджэячсмитьбюе";
    text.chars()
        .map(|c| {
            let swap = |from: &str, to: &str| from.chars().position(|f| f == c).and_then(|i| to.chars().nth(i));
            swap(EN, RU).or_else(|| swap(RU, EN)).unwrap_or(c)
        })
        .collect()
}

/// The rows of `ITEMS` matching `query`, best first. `key` gives a key setting's key as shown and
/// `shown` whether a row is in the list at all.
pub fn find(query: &str, key: &dyn Fn(Item) -> String, shown: &dyn Fn(Item) -> bool) -> Vec<usize> {
    let run = |query: &str| {
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut hits: Vec<(u32, usize)> = entries()
            .iter()
            .enumerate()
            .filter(|(i, _)| shown(ITEMS[*i].1))
            .filter_map(|(i, entry)| {
                let item = ITEMS[i].1;
                let key = if item.kind() == Kind::Key { lower(&key(item)) } else { String::new() };
                let total = words.iter().try_fold(0, |sum, word| rank(entry, &key, word).map(|r| sum + r))?;
                Some((total, i))
            })
            .collect();
        // A tie goes to the setting itself over a hotkey for it.
        hits.sort_by_key(|&(rank, i)| (rank, ITEMS[i].1.kind() == Kind::Key));
        hits.into_iter().map(|(_, i)| i).collect::<Vec<usize>>()
    };
    let query = lower(query);
    let hits = run(&query);
    if hits.is_empty() { run(&other_layout(&query)) } else { hits }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(query: &str) -> Option<Item> {
        find(query, &|item| if item == Item::Hotkey(8) { "F2".into() } else { "Ctrl+Q".into() }, &|_| true).first().map(|&i| ITEMS[i].1)
    }

    #[test]
    fn a_setting_is_found_by_its_name_in_any_language_whichever_is_on() {
        for query in ["music volume", "громкость музыки", "音楽の音量", "MUSIC"] {
            assert_eq!(first(query), Some(Item::MusicVolume), "{query}");
        }
    }

    #[test]
    fn the_start_of_a_name_beats_the_inside_of_one_and_those_beat_the_help() {
        assert_eq!(first("fps"), Some(Item::FpsCap));
        assert_eq!(first("hidden"), Some(Item::HiddenMaps));
    }

    #[test]
    fn other_words_for_a_setting_find_it_and_every_word_has_to_match() {
        assert_eq!(first("мат"), Some(Item::ChatFilter));
        assert_eq!(first("volume nonsense"), None);
        assert_eq!(first("ёлка"), None);
    }

    #[test]
    fn a_key_finds_what_it_is_bound_to() {
        assert_eq!(first("f2"), Some(Item::Hotkey(8)));
    }

    #[test]
    fn the_wrong_keyboard_layout_still_finds_it() {
        assert_eq!(other_layout("pderr"), "звукк");
        assert!(first("uhjvrjcnm").is_some(), "громкость typed on the English layout");
    }

    #[test]
    fn a_row_not_in_the_list_is_not_found() {
        let hits = find("krokodil", &|_| String::new(), &|item| item != Item::KrokodilKoban);
        assert!(hits.is_empty());
    }
}
