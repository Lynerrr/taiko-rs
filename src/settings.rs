//! Settings tab: the list of options and how it is drawn. Values are read and changed by the
//! caller (it owns the config, audio engine and window).

use std::cell::Cell;

use crate::render::{accent, approach, rgba, Canvas, Hit};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    ScrollSpeed,
    PlayfieldArea,
    PlayfieldPreset,
    PlayfieldReset,
    Rate,
    RateKeepPitch,
    Offset,
    VisualOffset,
    Calibrate,
    CalibrationBpm,
    LiveOffset,
    LiveOffsetUr,
    VsyncCompensation,
    VsyncTune,
    VsyncCalibrate,
    ResumeDelay,
    Interface,
    InterfaceHides,
    /// Song select's layout (dragged): Enter puts it back.
    SongSelectLayout,
    InterfaceKey,
    /// How autoplay hits (taiko_arranger's): playstyle, binding, hand.
    AutoplayStyle,
    AutoplayBinding,
    AutoplayLeftHanded,
    /// Which skin each part of the playfield and interface comes from.
    SkinParts,
    /// The choices for one of them (`skin::Part::ALL[i]`): the list it opens, not a row.
    SkinPart(usize),
    Key(usize),
    /// Previous (0) / next (1) playfield preset key.
    PresetKey(usize),
    /// `keys::Hotkey::ALL[i]`'s key.
    Hotkey(usize),
    TouchLayout,
    Screenshots,
    Language,
    ShowPlayfieldSize,
    WheelResize,
    KiaiEffects,
    HideGreat,
    ChatFilter,
    MessageSound,
    NotifyPlaying,
    NotifyOnline,
    NotifyMessages,
    NotifyMentions,
    NotifyRequests,
    ShowConverts,
    HiddenMaps,
    SongsFolder,
    DataFolder,
    MasterVolume,
    MusicVolume,
    FxVolume,
    AudioBuffer,
    BackgroundDim,
    BackgroundDimBlack,
    PresentMode,
    MenuVsync,
    HalfResolution,
    FpsCap,
    ShowFps,
    Animations,
    Splash,
    CheckUpdates,
    UpdateChannel,
    Update,
    /// Opens what's new.
    Changelog,
    /// Only for krokodil_koban: closes taiko.rs and starts osu!stable.
    KrokodilKoban,
    Fullscreen,
    WindowSize,
    DiscordPresence,
    Skin,
    SkinFolder,
    NoteColours,
    DonColour,
    KatColour,
    Accent,
    AccentHue,
}

pub const ITEMS: [(&str, Item); 108] = [
    ("GENERAL", Item::Language),
    ("GENERAL", Item::SongsFolder),
    ("GENERAL", Item::ShowConverts),
    ("GENERAL", Item::HiddenMaps),
    ("GENERAL", Item::DataFolder),
    ("GENERAL", Item::Screenshots),
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::Rate),
    ("GAMEPLAY", Item::RateKeepPitch),
    ("GAMEPLAY", Item::ResumeDelay),
    ("GAMEPLAY", Item::Interface),
    ("GAMEPLAY", Item::InterfaceHides),
    ("GAMEPLAY", Item::InterfaceKey),
    ("GAMEPLAY", Item::SongSelectLayout),
    ("GAMEPLAY", Item::AutoplayStyle),
    ("GAMEPLAY", Item::AutoplayBinding),
    ("GAMEPLAY", Item::AutoplayLeftHanded),
    ("PLAYFIELD", Item::PlayfieldArea),
    ("PLAYFIELD", Item::PlayfieldPreset),
    ("PLAYFIELD", Item::PlayfieldReset),
    ("PLAYFIELD", Item::ShowPlayfieldSize),
    ("PLAYFIELD", Item::WheelResize),
    ("PLAYFIELD", Item::BackgroundDim),
    ("PLAYFIELD", Item::BackgroundDimBlack),
    ("PLAYFIELD", Item::KiaiEffects),
    ("PLAYFIELD", Item::HideGreat),
    ("INPUT", Item::Key(0)),
    ("INPUT", Item::Key(1)),
    ("INPUT", Item::Key(2)),
    ("INPUT", Item::Key(3)),
    ("INPUT", Item::PresetKey(0)),
    ("INPUT", Item::PresetKey(1)),
    ("INPUT", Item::TouchLayout),
    ("INPUT", Item::Hotkey(0)),
    ("INPUT", Item::Hotkey(1)),
    ("INPUT", Item::Hotkey(2)),
    ("INPUT", Item::Hotkey(3)),
    ("INPUT", Item::Hotkey(4)),
    ("INPUT", Item::Hotkey(5)),
    ("INPUT", Item::Hotkey(6)),
    ("INPUT", Item::Hotkey(7)),
    ("INPUT", Item::Hotkey(8)),
    ("INPUT", Item::Hotkey(9)),
    ("INPUT", Item::Hotkey(10)),
    ("INPUT", Item::Hotkey(11)),
    ("INPUT", Item::Hotkey(12)),
    ("INPUT", Item::Hotkey(13)),
    ("INPUT", Item::Hotkey(14)),
    ("INPUT", Item::Hotkey(15)),
    ("INPUT", Item::Hotkey(16)),
    ("INPUT", Item::Hotkey(17)),
    ("INPUT", Item::Hotkey(18)),
    ("INPUT", Item::Hotkey(19)),
    ("INPUT", Item::Hotkey(20)),
    ("INPUT", Item::Hotkey(21)),
    ("INPUT", Item::Hotkey(22)),
    ("INPUT", Item::Hotkey(23)),
    ("INPUT", Item::Hotkey(24)),
    ("INPUT", Item::Hotkey(25)),
    ("INPUT", Item::Hotkey(26)),
    ("INPUT", Item::Hotkey(27)),
    ("INPUT", Item::Hotkey(28)),
    ("INPUT", Item::Hotkey(29)),
    ("INPUT", Item::Hotkey(30)),
    ("INPUT", Item::Hotkey(31)),
    ("OFFSET", Item::Offset),
    ("OFFSET", Item::VisualOffset),
    ("OFFSET", Item::Calibrate),
    ("OFFSET", Item::CalibrationBpm),
    ("OFFSET", Item::LiveOffset),
    ("OFFSET", Item::LiveOffsetUr),
    ("OFFSET", Item::VsyncCompensation),
    ("OFFSET", Item::VsyncTune),
    ("OFFSET", Item::VsyncCalibrate),
    ("AUDIO", Item::MasterVolume),
    ("AUDIO", Item::MusicVolume),
    ("AUDIO", Item::FxVolume),
    ("AUDIO", Item::AudioBuffer),
    ("VIDEO", Item::PresentMode),
    ("VIDEO", Item::MenuVsync),
    ("VIDEO", Item::HalfResolution),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
    ("VIDEO", Item::WindowSize),
    ("VIDEO", Item::Animations),
    ("VIDEO", Item::DiscordPresence),
    ("VIDEO", Item::Splash),
    ("SKIN", Item::Skin),
    ("SKIN", Item::SkinParts),
    ("SKIN", Item::SkinFolder),
    ("SKIN", Item::NoteColours),
    ("SKIN", Item::DonColour),
    ("SKIN", Item::KatColour),
    ("SKIN", Item::Accent),
    ("SKIN", Item::AccentHue),
    ("ONLINE", Item::ChatFilter),
    ("ONLINE", Item::MessageSound),
    ("ONLINE", Item::NotifyMessages),
    ("ONLINE", Item::NotifyMentions),
    ("ONLINE", Item::NotifyRequests),
    ("ONLINE", Item::NotifyOnline),
    ("ONLINE", Item::NotifyPlaying),
    ("ONLINE", Item::KrokodilKoban),
    ("UPDATES", Item::UpdateChannel),
    ("UPDATES", Item::Update),
    ("UPDATES", Item::Changelog),
    ("UPDATES", Item::CheckUpdates),
];

/// The sections in order, with a one-line summary for the sidebar.
pub const SECTIONS: [(&str, &str); 11] = [
    ("LIBRARY", "Songs folder, data, screenshots"),
    ("GAMEPLAY", "speed, rate, interface, autoplay"),
    ("GENERAL", "language, Songs folder, data"),
    ("PLAYFIELD", "size, position"),
    ("INPUT", "drum keys, hotkeys, touch"),
    ("OFFSET", "audio offset, calibration"),
    ("AUDIO", "volume, latency"),
    ("VIDEO", "frame pacing, display, Discord"),
    ("SKIN", "notes, hitsounds, colour"),
    ("ONLINE", "chat filter, notifications"),
    ("UPDATES", "new versions"),
];

/// First item of section `s`.
pub fn section_start(s: usize) -> usize {
    ITEMS.iter().position(|(sec, _)| *sec == SECTIONS[s].0).unwrap_or(0)
}

/// Section `s`'s name, as shown (`SECTIONS` has the English, which `ITEMS` refers to it by).
pub fn section_name(s: usize) -> &'static str {
    match SECTIONS.get(s).map_or("", |s| s.0) {
        "GENERAL" => tr!("GENERAL", "一般", "ОБЩИЕ"),
        "GAMEPLAY" => tr!("GAMEPLAY", "ゲームプレイ", "ИГРА"),
        "PLAYFIELD" => tr!("PLAYFIELD", "プレイフィールド", "ИГРОВОЕ ПОЛЕ"),
        "INPUT" => tr!("INPUT", "入力", "УПРАВЛЕНИЕ"),
        "OFFSET" => tr!("OFFSET", "オフセット", "СМЕЩЕНИЕ"),
        "AUDIO" => tr!("AUDIO", "オーディオ", "ЗВУК"),
        "VIDEO" => tr!("VIDEO", "グラフィック", "ГРАФИКА"),
        "SKIN" => tr!("SKIN", "スキン", "СКИН"),
        "ONLINE" => tr!("ONLINE", "オンライン", "ОНЛАЙН"),
        "UPDATES" => tr!("UPDATES", "アップデート", "ОБНОВЛЕНИЯ"),
        other => other,
    }
}

/// Section `s`'s one-line summary for the sidebar.
pub fn section_summary(s: usize) -> &'static str {
    match SECTIONS.get(s).map_or("", |s| s.0) {
        "GENERAL" => tr!("language, Songs folder, data", "言語、Songsフォルダ、データ", "язык, папка Songs, данные"),
        "GAMEPLAY" => tr!("speed, rate, interface", "速度、レート、インターフェース", "скорость, рейт, интерфейс"),
        "PLAYFIELD" => tr!("size, position", "サイズ、位置", "размер, положение"),
        "INPUT" => tr!("drum keys, hotkeys, touch", "太鼓のキー、ショートカット、タッチ", "клавиши, горячие клавиши"),
        "OFFSET" => tr!("audio offset, calibration", "音声オフセット、調整", "смещение звука, калибровка"),
        "AUDIO" => tr!("volume, latency", "音量、遅延", "громкость, задержка"),
        "VIDEO" => tr!("frame pacing, display, Discord", "フレーム、画面、Discord", "кадры, экран, Discord"),
        "SKIN" => tr!("notes, hitsounds, colour", "ノーツ、ヒット音、色", "ноты, хитсаунды, цвет"),
        "ONLINE" => tr!("chat filter, notifications", "チャットフィルター、通知", "фильтр чата, уведомления"),
        "UPDATES" => tr!("new versions", "新しいバージョン", "новые версии"),
        _ => "",
    }
}

/// What Enter does to a setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Opens a list to pick from.
    List,
    /// Types a value.
    Value,
    /// Flips it on / off.
    Toggle,
    /// Waits for a key to bind.
    Key,
    /// Does something (opens an editor, starts calibration, ...).
    Action,
}

/// Drum key `i`'s name: left rim, left centre, right centre, right rim.
pub fn key_label(i: usize) -> &'static str {
    match i {
        0 => tr!("Left rim (kat)", "左ふち (カッ)", "Левый край (кат)"),
        1 => tr!("Left centre (don)", "左面 (ドン)", "Левый центр (дон)"),
        2 => tr!("Right centre (don)", "右面 (ドン)", "Правый центр (дон)"),
        _ => tr!("Right rim (kat)", "右ふち (カッ)", "Правый край (кат)"),
    }
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::ScrollSpeed => tr!("Scroll speed", "スクロール速度", "Скорость прокрутки"),
            Item::PlayfieldArea => tr!("Playfield area", "プレイフィールドの範囲", "Область игрового поля"),
            Item::PlayfieldReset => tr!("Reset playfield area", "プレイフィールドを元に戻す", "Сбросить игровое поле"),
            Item::PlayfieldPreset => tr!("Playfield preset", "プレイフィールドのプリセット", "Пресет игрового поля"),
            Item::Rate => tr!("Speed (rate)", "再生速度 (レート)", "Скорость (рейт)"),
            Item::RateKeepPitch => tr!("Rate keeps pitch", "レートで音程を保つ", "Рейт сохраняет тон"),
            Item::Offset => tr!("Audio offset", "音声オフセット", "Смещение звука"),
            Item::VisualOffset => tr!("Visual offset", "表示オフセット", "Смещение картинки"),
            Item::Calibrate => tr!("Offset calibration", "オフセット調整", "Калибровка смещения"),
            Item::CalibrationBpm => tr!("Calibration tempo", "調整のテンポ", "Темп калибровки"),
            Item::LiveOffset => tr!("Live offset suggestion", "プレイ中のオフセット提案", "Подсказка смещения в игре"),
            Item::LiveOffsetUr => tr!("Scale suggestion to UR", "提案をURに合わせる", "Подсказка с учётом UR"),
            Item::VsyncCompensation => tr!("Vsync compensation", "垂直同期の補正", "Компенсация vsync"),
            Item::VsyncTune => tr!("Vsync compensation: fine tune", "垂直同期の補正: 微調整", "Компенсация vsync: точно"),
            Item::VsyncCalibrate => tr!("Vsync calibration", "垂直同期の調整", "Калибровка vsync"),
            Item::ResumeDelay => tr!("Resume delay", "再開までの待ち時間", "Задержка продолжения"),
            Item::Interface => tr!("Interface", "インターフェース", "Интерфейс"),
            Item::InterfaceHides => tr!("Interface toggle hides", "切り替えで隠すもの", "Что скрывает переключение"),
            Item::InterfaceKey => tr!("Interface toggle key", "インターフェースの切替キー", "Клавиша интерфейса"),
            Item::SongSelectLayout => tr!("Song select layout", "選曲画面のレイアウト", "Макет выбора песни"),
            Item::AutoplayStyle => "Autoplay playstyle",
            Item::AutoplayBinding => "Autoplay binding",
            Item::AutoplayLeftHanded => "Autoplay left-handed",
            Item::Key(i) => key_label(i),
            Item::PresetKey(0) => tr!("Previous preset key", "前のプリセットのキー", "Клавиша: пред. пресет"),
            Item::PresetKey(_) => tr!("Next preset key", "次のプリセットのキー", "Клавиша: след. пресет"),
            Item::Hotkey(i) => crate::keys::Hotkey::ALL[i].label(),
            Item::TouchLayout => tr!("Touch layout", "タッチの配置", "Сенсорная раскладка"),
            Item::Screenshots => tr!("Screenshots (F12)", "スクリーンショット (F12)", "Скриншоты (F12)"),
            // Also in English, so anyone who picked a language by mistake finds their way back.
            Item::Language => tr!("Language", "言語 (Language)", "Язык (Language)"),
            Item::ShowPlayfieldSize => tr!("Show playfield size", "プレイフィールドのサイズを表示", "Показывать размер поля"),
            Item::WheelResize => tr!("Wheel resizes playfield", "ホイールでサイズを変える", "Колесо меняет размер поля"),
            Item::KiaiEffects => tr!("Kiai effects", "キアイの演出", "Эффекты киай"),
            Item::HideGreat => tr!("Hide GREAT splash", "GREATの表示を隠す", "Скрыть вспышку GREAT"),
            Item::ChatFilter => tr!("Chat filter", "チャットフィルター", "Фильтр чата"),
            Item::MessageSound => tr!("Message sound", "メッセージの音", "Звук сообщений"),
            Item::NotifyMessages => tr!("Notify: messages", "通知: メッセージ", "Уведомлять: сообщения"),
            Item::NotifyMentions => tr!("Notify: chat mentions", "通知: チャットでの呼びかけ", "Уведомлять: упоминания"),
            Item::NotifyRequests => tr!("Notify: friend requests", "通知: フレンド申請", "Уведомлять: заявки в друзья"),
            Item::NotifyOnline => tr!("Notify: friend online", "通知: フレンドのログイン", "Уведомлять: друг в сети"),
            Item::NotifyPlaying => tr!("Notify: friend playing", "通知: フレンドのプレイ", "Уведомлять: друг играет"),
            Item::ShowConverts => tr!("Show osu!standard converts", "osu!standardの変換譜面を表示", "Карты osu!standard (конверты)"),
            Item::HiddenMaps => tr!("Hidden maps", "非表示の譜面", "Скрытые карты"),
            Item::SongsFolder => tr!("Songs folder", "Songsフォルダ", "Папка Songs"),
            Item::DataFolder => tr!("Game data folder", "ゲームデータのフォルダ", "Папка данных игры"),
            Item::MasterVolume => tr!("Master volume", "全体の音量", "Общая громкость"),
            Item::MusicVolume => tr!("Music volume", "音楽の音量", "Громкость музыки"),
            Item::FxVolume => tr!("Hitsound volume", "ヒット音の音量", "Громкость хитсаундов"),
            Item::AudioBuffer => tr!("Audio buffer", "オーディオバッファ", "Аудиобуфер"),
            Item::BackgroundDim => tr!("Background dim", "背景の暗さ", "Затемнение фона"),
            Item::BackgroundDimBlack => tr!("Dim to black", "黒まで暗くする", "Затемнять до чёрного"),
            Item::PresentMode => tr!("Present mode (gameplay)", "表示モード (プレイ中)", "Режим вывода (в игре)"),
            Item::MenuVsync => tr!("Menus use vsync", "メニューは垂直同期", "Vsync в меню"),
            Item::HalfResolution => tr!("Half resolution", "半分の解像度", "Половинное разрешение"),
            Item::FpsCap => tr!("FPS cap", "FPSの上限", "Ограничение FPS"),
            Item::ShowFps => tr!("FPS overlay", "FPSの表示", "Счётчик FPS"),
            Item::Animations => tr!("UI animations", "UIのアニメーション", "Анимации интерфейса"),
            Item::Splash => tr!("Startup splash", "起動画面", "Заставка при запуске"),
            Item::UpdateChannel => tr!("Build channel", "ビルドのチャンネル", "Канал сборок"),
            Item::Update => tr!("Update", "アップデート", "Обновление"),
            Item::Changelog => tr!("What's new", "更新内容", "Что нового"),
            Item::KrokodilKoban => tr!("krokodil_koban mode", "krokodil_kobanモード", "Режим krokodil_koban"),
            Item::CheckUpdates => tr!("Check at startup", "起動時に確認", "Проверять при запуске"),
            Item::Fullscreen => tr!("Fullscreen", "フルスクリーン", "Полный экран"),
            Item::WindowSize => tr!("Window size", "ウィンドウのサイズ", "Размер окна"),
            Item::DiscordPresence => tr!("Discord status", "Discordのステータス", "Статус в Discord"),
            Item::Skin => tr!("Skin", "スキン", "Скин"),
            Item::SkinParts => tr!("Skin parts", "スキンのパーツ", "Части скина"),
            Item::SkinPart(i) => crate::skin::Part::ALL.get(i).map_or(tr!("Skin part", "スキンのパーツ", "Часть скина"), |p| p.label()),
            Item::SkinFolder => tr!("Skins folder", "スキンのフォルダ", "Папка скинов"),
            Item::Accent => tr!("UI colour", "UIの色", "Цвет интерфейса"),
            Item::AccentHue => tr!("UI colour: fine tune", "UIの色: 微調整", "Цвет интерфейса: точно"),
            Item::NoteColours => tr!("Note colours", "ノーツの色", "Цвета нот"),
            Item::DonColour => tr!("Don colour", "ドンの色", "Цвет дона"),
            Item::KatColour => tr!("Kat colour", "カッの色", "Цвет ката"),
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Item::ScrollSpeed
            | Item::Rate
            | Item::Offset
            | Item::VisualOffset
            | Item::CalibrationBpm
            | Item::ResumeDelay
            | Item::MasterVolume
            | Item::MusicVolume
            | Item::FxVolume
            | Item::BackgroundDim
            | Item::FpsCap
            | Item::AccentHue
            | Item::VsyncTune
            | Item::DonColour
            | Item::KatColour => Kind::Value,
            Item::PlayfieldPreset
            | Item::TouchLayout
            | Item::Screenshots
            | Item::Language
            | Item::AudioBuffer
            | Item::PresentMode
            | Item::UpdateChannel
            | Item::Skin
            | Item::SkinParts
            | Item::SkinPart(_)
            | Item::NoteColours
            | Item::InterfaceHides
            | Item::AutoplayStyle
            | Item::AutoplayBinding
            | Item::WindowSize
            | Item::Accent
            | Item::SongsFolder
            | Item::DataFolder => Kind::List,
            Item::RateKeepPitch
            | Item::LiveOffset
            | Item::LiveOffsetUr
            | Item::VsyncCompensation
            | Item::ShowPlayfieldSize
            | Item::WheelResize
            | Item::BackgroundDimBlack
            | Item::Interface
            | Item::KiaiEffects
            | Item::AutoplayLeftHanded
            | Item::HideGreat
            | Item::ChatFilter
            | Item::MessageSound
            | Item::NotifyPlaying
            | Item::NotifyOnline
            | Item::NotifyMessages
            | Item::NotifyMentions
            | Item::NotifyRequests
            | Item::ShowConverts
            | Item::MenuVsync
            | Item::HalfResolution
            | Item::ShowFps
            | Item::Animations
            | Item::Splash
            | Item::CheckUpdates
            | Item::Fullscreen
            | Item::DiscordPresence => Kind::Toggle,
            Item::Key(_) | Item::PresetKey(_) | Item::InterfaceKey | Item::Hotkey(_) => Kind::Key,
            Item::PlayfieldArea | Item::PlayfieldReset | Item::Calibrate | Item::VsyncCalibrate | Item::HiddenMaps | Item::Update | Item::Changelog | Item::KrokodilKoban | Item::SongSelectLayout | Item::SkinFolder => Kind::Action,
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Item::ScrollSpeed => tr!(
                "How fast notes travel. Shortcut: F3 / F4.",
                "ノーツが流れる速さ。ショートカット: F3 / F4。",
                "Как быстро летят ноты. Клавиши: F3 / F4."
            ),
            Item::PlayfieldArea => tr!(
                "Enter: edit where gameplay is drawn (drag, its edges, the wheel). In game: drag it about too.",
                "Enter: プレイを描く範囲を編集 (ドラッグ、端、ホイール)。プレイ中もドラッグで動かせます。",
                "Enter: где рисуется игра (тяните поле, его края, колесо). В игре его тоже можно тянуть."
            ),
            Item::PlayfieldReset => tr!(
                "Enter: make gameplay fill the whole screen again.",
                "Enter: プレイ画面を再び全画面にします。",
                "Enter: игра снова на весь экран."
            ),
            Item::PlayfieldPreset => tr!(
                "Enter: pick from a list (grouped by aspect ratio). Left/Right: previous/next.",
                "Enter: リストから選ぶ (縦横比ごと)。左/右: 前/次。",
                "Enter: выбрать из списка (по соотношению сторон). Влево/вправо: пред./след."
            ),
            Item::Rate => tr!(
                "Song speed like DT (1.50x) / HT (0.75x). Song select: [ and ] (Shift = 0.25x steps).",
                "DT (1.50x) / HT (0.75x) のような曲の速さ。選曲画面: [ と ] (Shiftで0.25x刻み)。",
                "Скорость песни, как DT (1.50x) / HT (0.75x). В выборе песни: [ и ] (Shift = шаг 0.25x)."
            ),
            Item::RateKeepPitch => tr!(
                "On: music sounds normal, just faster (DT). Off: pitch rises with speed (nightcore).",
                "オン: 音程はそのまま速くなる (DT)。オフ: 速さと共に音程も上がる (nightcore)。",
                "Вкл: музыка просто быстрее (DT). Выкл: тон растёт со скоростью (nightcore)."
            ),
            Item::Offset => tr!(
                "For every map; raise it if you hit late. In a song, - / + (and O) set that map's own offset on top.",
                "全譜面に共通。叩くのが遅れるなら上げます。曲中の - / + (と O) でその譜面だけのオフセットを足せます。",
                "Для всех карт; увеличьте, если бьёте поздно. В песне - / + (и O) задают смещение этой карты сверху."
            ),
            Item::Calibrate => tr!(
                "Enter: play a click track with notes exactly on the beat; it measures and sets your offset.",
                "Enter: 拍ぴったりのノーツでクリック音を流し、オフセットを測って設定します。",
                "Enter: метроном с нотами точно в такт; игра измерит и выставит смещение."
            ),
            Item::CalibrationBpm => tr!(
                "Tempo of the calibration click track. Pick one close to the maps you play.",
                "調整用クリック音のテンポ。よく遊ぶ譜面に近いものを選んでください。",
                "Темп метронома для калибровки. Выберите близкий к вашим картам."
            ),
            Item::LiveOffset => tr!(
                "While playing, suggest an offset from your recent hits; press O to apply it.",
                "プレイ中、最近の叩きからオフセットを提案します。Oで適用。",
                "Во время игры подсказывает смещение по последним ударам; O применяет его."
            ),
            Item::LiveOffsetUr => tr!(
                "On: more hits and a +- margin when your UR is high. Off: plain average of the last 40 hits.",
                "オン: URが高いと多めの叩きと±の幅を使う。オフ: 直近40回の単純な平均。",
                "Вкл: при высоком UR больше ударов и запас +-. Выкл: среднее последних 40 ударов."
            ),
            Item::VsyncCompensation => tr!(
                "With vsync, notes are drawn where they'll be once the frame is on screen, as with unlimited fps.",
                "垂直同期でも、FPS無制限と同じくフレームが映る時の位置にノーツを描きます。",
                "С vsync ноты рисуются там, где будут, когда кадр покажется, как без ограничения FPS."
            ),
            Item::VsyncTune => tr!(
                "Added to the vsync compensation: raise it if you hit late by sight with vsync. Vsync calibration finds it.",
                "垂直同期の補正に足す値。垂直同期で見て叩くと遅れるなら上げます。垂直同期の調整で求まります。",
                "Добавка к компенсации vsync: увеличьте, если с vsync бьёте поздно на глаз. Её находит калибровка vsync."
            ),
            Item::VsyncCalibrate => tr!(
                "Enter: hit notes by sight, sound off, in vsync; it sets the fine tune. Calibrate the offset first.",
                "Enter: 音なし・垂直同期で見て叩き、微調整を設定します。先にオフセットを調整してください。",
                "Enter: бейте ноты на глаз без звука с vsync; выставит точную настройку. Сначала смещение."
            ),
            Item::ResumeDelay => tr!(
                "Countdown before the song continues after the pause menu (0 = at once).",
                "ポーズメニューの後、曲が再開するまでのカウントダウン (0 = すぐ)。",
                "Отсчёт перед продолжением после паузы (0 = сразу)."
            ),
            Item::Interface => tr!(
                "Score, accuracy, bars and more during play. Its key (Shift+Tab unless changed below) toggles it in a song.",
                "プレイ中のスコア、精度、バーなど。そのキー (変更しなければShift+Tab) で曲中に切り替えます。",
                "Счёт, точность, полосы и прочее в игре. Его клавиша (Shift+Tab, если не изменить) переключает в песне."
            ),
            Item::InterfaceHides => tr!(
                "What hiding the interface hides. Enter opens the list; Enter or a click ticks a part on or off.",
                "インターフェースを隠す時に隠すもの。Enterでリスト。Enterかクリックで項目をオン/オフ。",
                "Что прячется вместе с интерфейсом. Enter открывает список; Enter или клик отмечает часть."
            ),
            Item::InterfaceKey => tr!(
                "Enter, then press the key (with Shift or not) that shows / hides the interface in a song. Delete: none.",
                "Enterを押してから、曲中にインターフェースを出し入れするキー (Shiftも可) を押す。Delete: なし。",
                "Enter, затем клавиша (можно с Shift), что прячет / показывает интерфейс в песне. Delete: нет."
            ),
            Item::Key(_) => tr!(
                "Enter, then press the new key (Delete: none, Esc: cancel). Backspace: back to the default.",
                "Enterを押してから新しいキーを押す (Delete: なし、Esc: 取消)。Backspace: 初期設定に戻す。",
                "Enter, затем новая клавиша (Delete: нет, Esc: отмена). Backspace: по умолчанию."
            ),
            Item::PresetKey(_) => tr!(
                "Steps through playfield presets anywhere (also in game). Enter, then press a key. Delete: none.",
                "どこでも (プレイ中も) プレイフィールドのプリセットを切り替えます。Enterの後にキー。Delete: なし。",
                "Листает пресеты поля везде (и в игре). Enter, затем клавиша. Delete: нет."
            ),
            Item::Hotkey(_) => tr!(
                "Enter, then press the new key, Ctrl / Alt / Shift too (Delete: none, Esc: cancel). Backspace: the default.",
                "Enterの後に新しいキー (Ctrl / Alt / Shiftも可)。Delete: なし、Esc: 取消。Backspace: 初期設定。",
                "Enter, затем новая клавиша, можно с Ctrl / Alt / Shift (Delete: нет, Esc: отмена). Backspace: исходная."
            ),
            Item::Language => tr!(
                "The language of menus, settings and messages. Song titles stay as their maps have them.",
                "メニュー、設定、メッセージの言語。曲名は譜面のままです。",
                "Язык меню, настроек и сообщений. Названия песен остаются как в картах."
            ),
            Item::Screenshots => tr!(
                "Where F12 puts a screenshot: on the clipboard to paste, as a picture in the Screenshots folder, or both.",
                "F12のスクリーンショットの行き先: 貼り付け用のクリップボード、Screenshotsフォルダの画像、またはその両方。",
                "Куда F12 кладёт скриншот: в буфер обмена, картинкой в папку Screenshots или туда и туда."
            ),
            Item::TouchLayout => tr!(
                "Touch screens: the screen is four columns, left to right (K = rim / kat, D = centre / don).",
                "タッチ画面: 画面を左から4列に分けます (K = ふち / カッ、D = 面 / ドン)。",
                "Сенсорный экран: четыре столбца слева направо (K = край / кат, D = центр / дон)."
            ),
            Item::ShowPlayfieldSize => tr!(
                "Show the playfield's resolution in the bottom-right corner during play.",
                "プレイ中、右下にプレイフィールドの解像度を表示します。",
                "Показывать разрешение поля в правом нижнем углу во время игры."
            ),
            Item::WheelResize => tr!(
                "During play, the mouse wheel makes the playfield bigger or smaller (it always does in its editor).",
                "プレイ中、マウスホイールでプレイフィールドを拡大・縮小します (エディタでは常に)。",
                "В игре колесо мыши увеличивает или уменьшает поле (в его редакторе всегда)."
            ),
            Item::ShowConverts => tr!(
                "Also list standard maps, converted to taiko like osu! does.",
                "standardの譜面も、osu!と同じく太鼓に変換して並べます。",
                "Показывать и карты standard, переделанные в taiko, как в osu!."
            ),
            Item::HiddenMaps => tr!(
                "Right-click a song to hide it here (files stay). Enter lists them all again.",
                "曲を右クリックでここから隠せます (ファイルは残ります)。Enterで全て再表示。",
                "Правый клик по песне прячет её здесь (файлы остаются). Enter показывает все снова."
            ),
            Item::SongsFolder => tr!(
                "The folder your maps are in. Enter: browse, type a path, or go back to finding osu!'s.",
                "譜面のあるフォルダ。Enter: 参照、パスを入力、またはosu!のものを探すに戻す。",
                "Папка с вашими картами. Enter: обзор, ввести путь или снова искать папку osu!."
            ),
            Item::DataFolder => tr!(
                "Replays, scores, videos, downloaded maps, skins and caches. Enter: move them to another folder or drive.",
                "リプレイ、スコア、動画、ダウンロードした譜面、スキン、キャッシュ。Enter: 別のフォルダやドライブへ移動。",
                "Повторы, рекорды, видео, скачанные карты, скины и кэш. Enter: перенести в другую папку или на диск."
            ),
            Item::KiaiEffects => tr!(
                "During kiai time, tint the lane and pulse the hit target with the beat.",
                "キアイタイム中、レーンに色を付け、判定枠を拍に合わせて脈打たせます。",
                "В киай подкрашивать дорожку и пульсировать мишень в такт."
            ),
            Item::HideGreat => tr!(
                "No flash and no \"GREAT\" on 300s, like skins with an empty hit300: only OKs and misses show.",
                "300で光らず\"GREAT\"も出しません (空のhit300のスキンと同じ)。OKとミスだけが出ます。",
                "Без вспышки и \"GREAT\" на 300, как у скинов с пустым hit300: видны только OK и промахи."
            ),
            Item::ChatFilter => tr!(
                "Stars out swear words in friends' messages, the global chat and lobby chats.",
                "フレンドのメッセージ、全体チャット、ロビーのチャットの悪口を伏せ字にします。",
                "Заменяет мат звёздочками в сообщениях друзей, общем чате и чатах лобби."
            ),
            Item::MessageSound => tr!(
                "A soft chime when a friend messages you (not while you're playing).",
                "フレンドからメッセージが届くと小さく鳴ります (プレイ中は鳴りません)。",
                "Тихий звон, когда друг пишет вам (не во время игры)."
            ),
            Item::NotifyMessages => tr!(
                "A pop-up when a friend messages you (they still count as unread when off).",
                "フレンドからメッセージが届くとポップアップ (オフでも未読として数えます)。",
                "Всплывашка, когда друг пишет вам (выключено — всё равно считаются непрочитанными)."
            ),
            Item::NotifyMentions => tr!(
                "A pop-up when someone says your name in the global chat.",
                "全体チャットで誰かがあなたの名前を出すとポップアップ。",
                "Всплывашка, когда кто-то называет вас в общем чате."
            ),
            Item::NotifyRequests => tr!(
                "A pop-up when someone sends you a friend request.",
                "フレンド申請が届くとポップアップ。",
                "Всплывашка, когда вам присылают заявку в друзья."
            ),
            Item::NotifyOnline => tr!(
                "A pop-up when a friend comes online.",
                "フレンドがログインするとポップアップ。",
                "Всплывашка, когда друг заходит в сеть."
            ),
            Item::NotifyPlaying => tr!(
                "A pop-up each time a friend starts a map (click it to watch).",
                "フレンドが譜面を始めるたびにポップアップ (クリックで観戦)。",
                "Всплывашка, когда друг начинает карту (клик — смотреть)."
            ),
            Item::MasterVolume => tr!(
                "Scales music and hitsounds. Anywhere: hold Alt and scroll the mouse wheel.",
                "音楽とヒット音の音量。どこでも: Altを押しながらマウスホイール。",
                "Громкость музыки и хитсаундов. Везде: Alt и колесо мыши."
            ),
            Item::MusicVolume | Item::FxVolume => tr!("Left / Right to adjust.", "左 / 右で調整。", "Влево / вправо — изменить."),
            Item::AudioBuffer => tr!(
                "Smaller = less audio latency; raise it if you hear crackling.",
                "小さいほど音の遅延が少ない。ノイズが聞こえたら上げます。",
                "Меньше = меньше задержка звука; увеличьте, если трещит."
            ),
            Item::BackgroundDim => tr!(
                "How dark the map background is during play. 100% hides it.",
                "プレイ中の譜面の背景の暗さ。100%で見えなくなります。",
                "Насколько тёмен фон карты в игре. 100% прячет его."
            ),
            Item::BackgroundDimBlack => tr!(
                "On: in songs the background dims to pure black, and is black when there's none. Off: the interface's grey.",
                "オン: 曲中の背景は真っ黒まで暗くなり、背景がなければ黒。オフ: インターフェースの灰色。",
                "Вкл: в песне фон темнеет до чёрного и чёрный, если фона нет. Выкл: серый цвет интерфейса."
            ),
            Item::PresentMode => tr!(
                "Used during songs. vsync is smoothest on Wayland; mailbox/immediate add judder.",
                "曲中に使います。Waylandではvsyncが一番なめらか。mailbox/immediateはカクつきます。",
                "Для песен. На Wayland плавнее всего vsync; mailbox/immediate дёргаются."
            ),
            Item::MenuVsync => tr!(
                "Everything but playing a song uses vsync: song select (and its autoplay preview), settings, results, replays and autoplay.",
                "曲のプレイ以外は垂直同期: 選曲 (とオートのプレビュー)、設定、リザルト、リプレイ、オート。",
                "Всё, кроме игры, с vsync: выбор песни (и превью автоплея), настройки, итоги, повторы и автоплей."
            ),
            Item::HalfResolution => tr!(
                "Draw at half resolution, scaled up exactly 2x: chunky, crisp pixels and a quarter of the GPU work. For high-DPI screens.",
                "半分の解像度で描き、ちょうど2倍に拡大: くっきり大きな画素、GPUの負荷は1/4。高DPIの画面向け。",
                "Рисовать в половинном разрешении и растягивать ровно в 2x: чёткие крупные пиксели, четверть работы GPU."
            ),
            Item::FpsCap => tr!(
                "mailbox/immediate only. A few times your refresh rate is plenty.",
                "mailbox/immediateのみ。リフレッシュレートの数倍で十分です。",
                "Только mailbox/immediate. Хватит нескольких частот обновления экрана."
            ),
            Item::ShowFps => tr!(
                "Frame rate and frame time in the corner. Shortcut: F10.",
                "隅にフレームレートとフレーム時間。ショートカット: F10。",
                "Частота и время кадра в углу. Клавиша: F10."
            ),
            Item::Animations => tr!(
                "Menu motion: screen fades, sliding highlights, opening lists, results count-up. Gameplay is unaffected.",
                "メニューの動き: 画面のフェード、ハイライトの移動、リストの開閉、リザルトのカウント。プレイは変わりません。",
                "Движение меню: переходы, скользящая подсветка, списки, отсчёт итогов. На игру не влияет."
            ),
            Item::Splash => tr!(
                "The animated intro when taiko.rs starts (any key or click skips it).",
                "taiko.rs起動時のアニメーション (キーかクリックで飛ばせます)。",
                "Анимация при запуске taiko.rs (любая клавиша или клик пропускает)."
            ),
            Item::VisualOffset => tr!(
                "Draws notes earlier to make up for display lag; raise it if notes reach the drum after the beat. Judging stays on the audio. In a song: Ctrl + - / +.",
                "画面の遅れを補うためノーツを早めに描きます。拍より遅れて届くなら上げます。判定は音のまま。曲中: Ctrl + - / +。",
                "Рисует ноты раньше, чтобы скрыть задержку экрана; увеличьте, если ноты опаздывают. Судит по звуку. В песне: Ctrl + - / +."
            ),
            Item::UpdateChannel => tr!(
                "dev: test builds that haven't reached everyone yet (may break). Switch, then Update installs that channel's build.",
                "dev: まだ全員に届いていない試験ビルド (壊れるかも)。切り替えてからアップデートでそのビルドを入れます。",
                "dev: тестовые сборки, ещё не для всех (могут сломаться). Переключите, и Обновление поставит сборку канала."
            ),
            Item::Update => tr!(
                "Enter: look for a newer build, or install it (Ctrl+U in any menu). taiko.rs restarts into it.",
                "Enter: 新しいビルドを探す、または入れる (どのメニューでもCtrl+U)。taiko.rsが再起動します。",
                "Enter: найти новую сборку или поставить её (Ctrl+U в любом меню). taiko.rs перезапустится."
            ),
            // No explanation: krokodil_koban knows.
            Item::KrokodilKoban => "",
            Item::AutoplayStyle => "Which hand autoplay hits each note with: kddk full / semi alternate, single tap, roll, or ddkk.",
            Item::AutoplayBinding => "The order autoplay's four keys light the drum in (kddk): 1324 puts each hand's don on the other side.",
            Item::AutoplayLeftHanded => "Autoplay leads with the left hand instead of the right.",
            Item::SongSelectLayout => tr!(
                "Drag the edges between the song list, the map panel and the leaderboard. Enter: back to the default.",
                "曲リスト、譜面パネル、ランキングの間の境目をドラッグ。Enter: 初期設定に戻す。",
                "Тяните границы между списком песен, панелью карты и таблицей рекордов. Enter: по умолчанию."
            ),
            Item::Changelog => tr!(
                "Enter: the latest changes. They also show by themselves after an update.",
                "Enter: 最新の変更点。アップデート後にも自動で表示されます。",
                "Enter: последние изменения. После обновления они показываются сами."
            ),
            Item::CheckUpdates => tr!(
                "Look for a newer build each time taiko.rs starts (a badge top right says when there is one), and for newer versions of the songs you pick on osu!.",
                "起動のたびに新しいビルドを探し (あれば右上に表示)、選んだ曲のosu!での新しい版も探します。",
                "Искать новую сборку при каждом запуске (значок справа вверху) и новые версии выбранных песен на osu!."
            ),
            Item::Fullscreen => tr!("Shortcut: F11.", "ショートカット: F11。", "Клавиша: F11."),
            Item::WindowSize => tr!(
                "Enter: pick a size (or type one); Left/Right: the next size. Dragging the window's edges works too.",
                "Enter: サイズを選ぶ (入力も可)。左/右: 次のサイズ。ウィンドウの端のドラッグでも変えられます。",
                "Enter: выбрать размер (или ввести); влево/вправо: следующий. Можно тянуть края окна."
            ),
            Item::DiscordPresence => tr!(
                "Show the map you're playing, your lobby or who you spectate in your Discord status.",
                "Discordのステータスに、プレイ中の譜面、ロビー、観戦相手を表示します。",
                "Показывать в статусе Discord карту, лобби или того, кого вы смотрите."
            ),
            Item::Skin => tr!(
                "Enter opens the list. osu! skins from osu!'s or taiko.rs's Skins folder.",
                "Enterでリスト。osu!かtaiko.rsのSkinsフォルダにあるosu!のスキン。",
                "Enter открывает список. Скины osu! из папки Skins osu! или taiko.rs."
            ),
            Item::SkinParts | Item::SkinPart(_) => tr!(
                "What's drawn from a skin, and which: the skin above, built-in, or any other (mix them). Enter opens the list.",
                "スキンから描くものとその出どころ: 上のスキン、内蔵、または他のどれか (混ぜられます)。Enterでリスト。",
                "Что берётся из скина и из какого: скин выше, встроенный или любой другой (можно смешивать). Enter: список."
            ),
            Item::SkinFolder => tr!(
                "Enter: open taiko.rs's Skins folder. Drop .osk files there, then pick them above.",
                "Enter: taiko.rsのSkinsフォルダを開く。.oskファイルを置いて、上で選びます。",
                "Enter: открыть папку Skins taiko.rs. Положите туда файлы .osk и выберите выше."
            ),
            Item::Accent => tr!(
                "The interface's accent; any but red tints the greys too. Left/Right: presets (or #rrggbb in the config).",
                "インターフェースのアクセント色。赤以外は灰色にも色が付きます。左/右: プリセット (設定ファイルで#rrggbbも可)。",
                "Акцент интерфейса; любой, кроме красного, подкрашивает и серые. Влево/вправо: пресеты (или #rrggbb в конфиге)."
            ),
            Item::AccentHue => tr!(
                "Left/Right: turn the UI colour round the colour wheel, for any colour in between.",
                "左/右: UIの色を色相環で回し、間のどんな色にもできます。",
                "Влево/вправо: крутить цвет интерфейса по цветовому кругу, для любого оттенка."
            ),
            Item::NoteColours => tr!(
                "Dons' and kats' colours (Multiverse's universes have their own: M in the mod select).",
                "ドンとカッの色 (Multiverseの宇宙にはそれぞれの色: Mod選択でM)。",
                "Цвета донов и катов (у вселенных Multiverse свои: M в выборе модов)."
            ),
            Item::DonColour | Item::KatColour => tr!(
                "Enter: type a colour like #eb452c. Left/Right turn it round the colour wheel. Makes the colours your own.",
                "Enter: #eb452cのように色を入力。左/右で色相環を回します。自分だけの色になります。",
                "Enter: введите цвет вроде #eb452c. Влево/вправо крутят его по кругу. Цвета станут вашими."
            ),
        }
    }
}

#[derive(Default)]
pub struct SettingsUi {
    pub cursor: usize,
    /// Waiting for a key press for this item (a drum key or a preset key).
    pub binding: Option<Item>,
    /// Logged in as krokodil_koban: their own setting shows.
    pub krokodil: bool,
    /// Open resolution dropdown.
    pub dropdown: Option<Dropdown>,
    /// Open list of a setting's choices.
    pub choices: Option<Choices>,
    /// Typing a value for a setting.
    pub editing: Option<Edit>,
    /// The section last drawn and the animated highlight position in it.
    scroll: Cell<f32>,
    highlight: Cell<f32>,
    /// The first row shown of a section longer than the screen.
    first_row: Cell<usize>,
}

/// The playfield preset dropdown: a cursor over the flattened list, plus where each row was
/// drawn (virtual units) so the mouse can hover and click them. The first group is the player's
/// own saved sizes, which can be added, edited, deleted and reordered.
#[derive(Default)]
pub struct Dropdown {
    pub cursor: usize,
    scroll: f32,
    pub rows: Vec<(usize, [f32; 4])>,
    pub bounds: [f32; 4],
    /// Typing a size: for a new saved one (None) or saved one `i`, and the text so far.
    pub editing: Option<(Option<usize>, String)>,
    /// Where typing goes in that text (a character index).
    pub caret: usize,
    /// Buttons drawn last frame.
    pub buttons: Vec<(PresetAction, [f32; 4])>,
}

/// What the dropdown's buttons (and their keys) do to the saved sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetAction {
    /// Save the playfield's current size.
    SaveCurrent,
    /// Type a new size.
    Add,
    Edit,
    Delete,
    Up,
    Down,
}

/// Name of the saved sizes' group.
pub const SAVED: &str = "Saved";

impl Dropdown {
    pub fn new(cursor: usize) -> Dropdown {
        Dropdown { cursor, ..Default::default() }
    }

    /// Index of the row under (`x`, `y`) in virtual units.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows.iter().find(|(_, r)| inside(r, x, y)).map(|(i, _)| *i)
    }

    pub fn button_at(&self, x: f32, y: f32) -> Option<PresetAction> {
        self.buttons.iter().find(|(_, r)| inside(r, x, y)).map(|(a, _)| *a)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        inside(&self.bounds, x, y)
    }

    fn button(&mut self, c: &mut Canvas, x: f32, y: f32, label: &str, action: PresetAction) -> f32 {
        let w = Canvas::text_width(6.0, label) + 10.0;
        let hover = c.hovered(x, y, w, 12.0);
        c.round_rect(x, y, w, 12.0, 3.0, rgba(if hover { 0x4a4a64 } else { 0x2a2a3a }, 1.0), rgba(0x5a5a74, 1.0), 1.0);
        c.text(x + 5.0, y + 3.0, 6.0, rgba(0xffffff, 1.0), label);
        self.buttons.push((action, [x, y, w, 12.0]));
        w
    }

    /// Draws the list centred on screen. `groups`: (aspect name, resolutions), the saved sizes
    /// first when there are any; `saved`: how many; `current`: the preset matching the playfield
    /// now; `playfield`: its size in pixels.
    pub fn draw(
        &mut self,
        c: &mut Canvas,
        groups: &[(&str, Vec<(u32, u32)>)],
        saved: usize,
        current: Option<(u32, u32)>,
        native: (u32, u32),
        playfield: (u32, u32),
    ) {
        const ROW: f32 = 15.0;
        const HEAD: f32 = 16.0;
        enum Line<'a> {
            Head(&'a str),
            Row(usize, u32, u32),
            /// No saved sizes yet.
            Hint,
            /// Typing a new saved size.
            Input,
        }
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.55));
        self.buttons.clear();

        // Lay out, then scroll so the cursor row (or the size being typed) stays visible.
        let adding = matches!(self.editing, Some((None, _)));
        let mut layout: Vec<(Line, f32)> = Vec::new();
        let mut y = 0.0;
        if saved == 0 {
            layout.push((Line::Head(SAVED), y));
            layout.push((if adding { Line::Input } else { Line::Hint }, y + HEAD));
            y += HEAD + ROW;
        }
        let mut index = 0;
        for (name, list) in groups {
            layout.push((Line::Head(name), y));
            y += HEAD;
            for &(rw, rh) in list {
                layout.push((Line::Row(index, rw, rh), y));
                index += 1;
                y += ROW;
            }
            // A new saved size is typed below the others.
            if adding && saved > 0 && index == saved {
                layout.push((Line::Input, y));
                y += ROW;
            }
        }

        let (bw, max_h) = (360.0, 390.0);
        let bh = (y + 8.0 + 78.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        self.bounds = [bx, by, bw, bh];
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
        let title_w = c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), Item::PlayfieldPreset.label());
        let screen = tr!("screen {}x{}", "画面 {}x{}", "экран {}x{}"; native.0, native.1);
        c.text_right(bx + bw - 14.0, by + 13.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit(7.0, bw - 28.0 - title_w - 10.0, &screen));
        let save_label = tr!("+ Save current {}x{}", "+ 今の{}x{}を保存", "+ Сохранить текущий {}x{}"; playfield.0, playfield.1);
        let bw0 = self.button(c, bx + 14.0, by + 28.0, &save_label, PresetAction::SaveCurrent);
        self.button(c, bx + 20.0 + bw0, by + 28.0, tr!("+ Add size", "+ サイズを追加", "+ Добавить размер"), PresetAction::Add);

        let (top, view) = (by + 48.0, bh - 78.0);
        let focus = layout.iter().find(|(l, _)| match (l, &self.editing) {
            (Line::Input, _) => true,
            (Line::Row(i, ..), Some((Some(e), _))) => i == e,
            (Line::Row(i, ..), None) => *i == self.cursor,
            _ => false,
        });
        let focus_y = focus.map(|l| l.1).unwrap_or(0.0);
        self.scroll = (focus_y - view / 2.0).clamp(0.0, (y - view).max(0.0));
        self.rows.clear();
        let caret = self.caret;
        let text_input = |c: &mut Canvas, ry: f32, text: &str| {
            c.round_rect(bx + 8.0, ry, bw - 16.0, ROW - 2.0, 4.0, rgba(0x1d1d27, 1.0), rgba(0xFFD34D, 1.0), 1.0);
            c.text(bx + 22.0, ry + 3.5, 7.0, rgba(0xFFD34D, 1.0), text);
            c.rect(bx + 22.0 + caret_x(7.0, text, caret), ry + 2.5, 1.0, 9.0, rgba(0xFFD34D, 1.0));
            c.text_right(bx + bw - 22.0, ry + 3.5, 6.0, rgba(0x8a8aa0, 1.0), tr!("width x height", "幅 x 高さ", "ширина x высота"));
        };
        for (line, ly) in layout {
            let ry = top + ly - self.scroll;
            if ry < top - 1.0 || ry + ROW > top + view + 1.0 {
                continue;
            }
            match line {
                Line::Head(name) => {
                    let name = match name {
                        SAVED => tr!("Saved", "保存済み", "Сохранённые"),
                        "Other" => tr!("Other", "その他", "Другие"),
                        _ => name,
                    };
                    c.text(bx + 14.0, ry + 5.0, 7.0, rgba(accent(), 1.0), name);
                }
                Line::Hint => {
                    let hint = tr!(
                        "none yet: S saves the playfield's size, A adds one",
                        "まだありません: Sで今のサイズを保存、Aで追加",
                        "пока нет: S сохраняет размер поля, A добавляет"
                    );
                    c.text(bx + 22.0, ry + 3.5, 6.0, rgba(0x6f6f86, 1.0), hint);
                }
                Line::Input => {
                    if let Some((_, text)) = &self.editing {
                        text_input(c, ry, text);
                    }
                }
                Line::Row(i, rw, rh) => {
                    if let Some((Some(e), text)) = &self.editing {
                        if *e == i {
                            text_input(c, ry, text);
                            continue;
                        }
                    }
                    let selected = i == self.cursor && self.editing.is_none();
                    if selected {
                        c.round_rect(bx + 8.0, ry, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(accent(), 1.0), 1.0);
                    }
                    self.rows.push((i, [bx + 8.0, ry, bw - 16.0, ROW - 2.0]));
                    c.text(bx + 22.0, ry + 3.5, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &format!("{rw} x {rh}"));
                    let fits = rw <= native.0 && rh <= native.1;
                    let mut right = bx + bw - 22.0;
                    if selected && i < saved {
                        // Buttons for the saved size, right to left.
                        for (label, action) in [
                            (tr!("del", "削除", "удалить"), PresetAction::Delete),
                            (tr!("edit", "編集", "изменить"), PresetAction::Edit),
                            (tr!("down", "下へ", "вниз"), PresetAction::Down),
                            (tr!("up", "上へ", "вверх"), PresetAction::Up),
                        ] {
                            let lw = Canvas::text_width(6.0, label) + 10.0;
                            right -= lw;
                            self.button(c, right, ry + 0.5, label, action);
                            right -= 4.0;
                        }
                        right -= 6.0;
                    }
                    let (tag, col) = if !fits {
                        (tr!("too big", "大きすぎ", "слишком большой"), 0x8a8aa0)
                    } else if Some((rw, rh)) == current {
                        (current_tag(), 0x86E05A)
                    } else {
                        ("", 0)
                    };
                    c.text_right(right, ry + 3.5, 7.0, rgba(col, 1.0), tag);
                }
            }
        }
        let (line1, line2) = if self.editing.is_some() {
            (tr!("type a size like 1280x720   Enter save   Esc cancel", "1280x720のように入力   Enter 保存   Esc 取消", "введите размер вроде 1280x720   Enter сохранить   Esc отмена"), "")
        } else {
            (
                tr!("Up/Down or mouse choose   Enter/click apply   Esc close", "上下かマウスで選択   Enter/クリック 適用   Esc 閉じる", "Вверх/вниз или мышь   Enter/клик применить   Esc закрыть"),
                tr!(
                    "S save current   A add   E edit   Del delete   Shift+Up/Down move",
                    "S 今のを保存   A 追加   E 編集   Del 削除   Shift+上下 移動",
                    "S сохранить   A добавить   E изменить   Del удалить   Shift+вверх/вниз двигать"
                ),
            )
        };
        c.text_centered(w / 2.0, by + bh - 20.0, 6.0, rgba(0x6f6f86, 1.0), line1);
        c.text_centered(w / 2.0, by + bh - 10.0, 6.0, rgba(0x6f6f86, 1.0), line2);
    }
}

/// A value being typed into a setting's row.
pub struct Edit {
    pub item: Item,
    pub text: String,
    /// Where typing goes: a character index into `text`.
    pub caret: usize,
    /// Still the value it opened with: the first key typed replaces it.
    pub fresh: bool,
}

impl Edit {
    pub fn new(item: Item, text: String) -> Edit {
        Edit { item, caret: text.chars().count(), text, fresh: true }
    }

    /// Adds typed text at the caret (control characters dropped), up to `max` characters.
    pub fn type_text(&mut self, typed: &str, max: usize) {
        if self.fresh {
            self.text.clear();
            self.caret = 0;
            self.fresh = false;
        }
        caret_insert(&mut self.text, &mut self.caret, typed, max);
    }

    pub fn backspace(&mut self) {
        if self.fresh {
            self.text.clear();
            self.caret = 0;
            self.fresh = false;
        } else {
            caret_backspace(&mut self.text, &mut self.caret);
        }
    }

    /// Left / Right / Home / End move the caret, Delete removes the character after it.
    /// Says whether the key was one of those.
    pub fn nav(&mut self, code: winit::keyboard::KeyCode) -> bool {
        // The value that opened is replaced by typing; moving into it keeps it to edit.
        let used = caret_nav(&mut self.text, &mut self.caret, code);
        if used {
            self.fresh = false;
        }
        used
    }
}

/// Inserts `typed` (control characters dropped) at char index `caret`, up to `max` characters.
pub fn caret_insert(text: &mut String, caret: &mut usize, typed: &str, max: usize) {
    for ch in typed.chars().filter(|ch| !ch.is_control()) {
        if text.chars().count() < max {
            text.insert(byte_at(text, *caret), ch);
            *caret += 1;
        }
    }
}

pub fn caret_backspace(text: &mut String, caret: &mut usize) {
    if *caret > 0 {
        *caret -= 1;
        text.remove(byte_at(text, *caret));
    }
}

/// Moves the caret (Left, Right, Home, End) or deletes after it (Delete); false for other keys.
pub fn caret_nav(text: &mut String, caret: &mut usize, code: winit::keyboard::KeyCode) -> bool {
    use winit::keyboard::KeyCode::*;
    let len = text.chars().count();
    *caret = (*caret).min(len);
    match code {
        ArrowLeft => *caret = caret.saturating_sub(1),
        ArrowRight => *caret = (*caret + 1).min(len),
        Home => *caret = 0,
        End => *caret = len,
        Delete => {
            if *caret < len {
                text.remove(byte_at(text, *caret));
            }
        }
        _ => return false,
    }
    true
}

fn byte_at(text: &str, caret: usize) -> usize {
    text.char_indices().nth(caret).map_or(text.len(), |(b, _)| b)
}

/// Where the caret is drawn: the text before it, in `size`, from the text's left edge.
pub fn caret_x(size: f32, text: &str, caret: usize) -> f32 {
    Canvas::text_width(size, &text.chars().take(caret).collect::<String>())
}

/// A setting's choices in one list (skins, present modes, ...), so picking one isn't stepping
/// through them all.
pub struct Choices {
    pub item: Item,
    pub names: Vec<String>,
    /// The one in use, marked "current".
    pub current: Option<usize>,
    /// Top right: how many skins are installed, the folder in use, ...
    pub note: String,
    pub cursor: usize,
}

impl Choices {
    /// Opens on the choice in use.
    pub fn new(item: Item, names: Vec<String>, current: Option<usize>, note: String) -> Choices {
        Choices { item, names, current, note, cursor: current.unwrap_or(0) }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.cursor = (self.cursor as i32 + delta).clamp(0, self.names.len() as i32 - 1) as usize;
    }

    pub fn draw(&self, c: &mut Canvas) {
        const ROW: f32 = 18.0;
        let w = c.width;
        c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.55));
        let n = self.names.len();
        let (bw, max_h) = ((w - 48.0).min(420.0), 380.0);
        let bh = (n as f32 * ROW + 56.0).min(max_h);
        let (bx, by) = ((w - bw) / 2.0, (480.0 - bh) / 2.0);
        c.round_rect(bx, by, bw, bh, 8.0, rgba(0x15151d, 0.98), rgba(accent(), 0.8), 1.5);
        let title_w = c.text(bx + 14.0, by + 12.0, 9.0, rgba(0xffffff, 1.0), self.item.label());
        c.text_right(bx + bw - 14.0, by + 13.0, 7.0, rgba(0x6f6f86, 1.0), &Canvas::fit_end(7.0, bw - 38.0 - title_w, &self.note));
        let (top, view) = (by + 32.0, bh - 50.0);
        let scroll = (self.cursor as f32 * ROW - view / 2.0).clamp(0.0, (n as f32 * ROW - view).max(0.0));
        for (i, name) in self.names.iter().enumerate() {
            let y = top + i as f32 * ROW - scroll;
            if y < top - 1.0 || y + ROW > top + view + 1.0 {
                continue;
            }
            let selected = i == self.cursor;
            let hover = c.hovered(bx + 8.0, y, bw - 16.0, ROW - 2.0);
            if selected || hover {
                c.round_rect(bx + 8.0, y, bw - 16.0, ROW - 2.0, 4.0, rgba(0x34344a, 1.0), rgba(if selected { accent() } else { 0x3a3a4e }, 1.0), 1.0);
            }
            c.hit(bx + 8.0, y, bw - 16.0, ROW, Hit::ChoiceRow(i));
            let current = self.current == Some(i);
            let room = bw - 40.0 - if current { Canvas::text_width(7.0, current_tag()) + 8.0 } else { 0.0 };
            c.text(bx + 20.0, y + 5.0, 7.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), &Canvas::fit(7.0, room, name));
            if current {
                c.text_right(bx + bw - 20.0, y + 5.0, 7.0, rgba(0x86E05A, 1.0), current_tag());
            }
        }
        let hint = tr!("Up/Down or mouse   Enter/click use   Esc close", "上下かマウス   Enter/クリック 使う   Esc 閉じる", "Вверх/вниз или мышь   Enter/клик выбрать   Esc закрыть");
        c.text_centered(w / 2.0, by + bh - 11.0, 6.0, rgba(0x6f6f86, 1.0), &Canvas::fit_hints(6.0, bw - 16.0, hint));
    }
}

/// The mark on the choice in use.
fn current_tag() -> &'static str {
    tr!("current", "使用中", "текущий")
}

fn inside(r: &[f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]
}

impl SettingsUi {
    pub fn item(&self) -> Item {
        ITEMS[self.cursor].1
    }

    pub fn move_cursor(&mut self, delta: i32) {
        let start = self.cursor;
        self.cursor = (self.cursor as i32 + delta).clamp(0, ITEMS.len() as i32 - 1) as usize;
        // Past a row that isn't shown, or back where it was at the end of the list.
        while !self.shows(ITEMS[self.cursor].1) {
            let next = self.cursor as i32 + delta.signum();
            self.cursor = if (0..ITEMS.len() as i32).contains(&next) && delta != 0 { next as usize } else { start };
            if self.cursor == start {
                break;
            }
        }
        self.editing = None;
    }

    /// Whether `item` is in the list (krokodil_koban mode is only for them).
    pub fn shows(&self, item: Item) -> bool {
        item != Item::KrokodilKoban || self.krokodil
    }

    /// Section of the selected item.
    pub fn section(&self) -> usize {
        SECTIONS.iter().position(|(name, _)| *name == ITEMS[self.cursor].0).unwrap_or(0)
    }

    /// A sidebar of sections and the selected section's settings. `value` gives the current
    /// display value of each item.
    pub fn draw(&self, c: &mut Canvas, value: &dyn Fn(Item) -> String, footer: &str) {
        const TOP: f32 = 62.0;
        const ROW: f32 = 30.0;
        let w = c.width;
        let total = (w - 48.0).min(780.0);
        let x0 = (w - total) / 2.0;
        let side_w = 170.0;
        let (cx, col) = (x0 + side_w + 16.0, total - side_w - 16.0);
        let section = self.section();

        // Sidebar.
        for i in 0..SECTIONS.len() {
            let (name, about) = (section_name(i), section_summary(i));
            let y = TOP + i as f32 * 40.0;
            let on = i == section;
            let hover = !on && c.hovered(x0, y, side_w, 34.0);
            let bg = if on { 0x2b1a17 } else if hover { 0x262634 } else { 0x1d1d27 };
            c.round_rect(x0, y, side_w, 34.0, 7.0, rgba(bg, 0.95), rgba(if on { accent() } else { 0x2a2a38 }, 1.0), if on { 1.5 } else { 1.0 });
            c.hit(x0, y, side_w, 34.0, Hit::SettingsSection(i));
            c.text(x0 + 12.0, y + 7.0, 9.0, rgba(if on { 0xffffff } else { 0xc8c8d8 }, 1.0), name);
            c.text(x0 + 12.0, y + 21.0, 6.0, rgba(0x8a8aa0, 1.0), &Canvas::fit(6.0, side_w - 20.0, about));
        }

        // The section's settings.
        let rows: Vec<(usize, Item)> = ITEMS.iter().enumerate().filter(|(_, (sec, it))| *sec == SECTIONS[section].0 && self.shows(*it)).map(|(i, (_, it))| (i, *it)).collect();
        c.text(cx, TOP - 2.0, 11.0, rgba(accent(), 1.0), section_name(section));
        let first = rows.first().map(|r| r.0).unwrap_or(0);
        // A long section (the hotkeys) scrolls to keep the cursor in sight, leaving room for the
        // help underneath.
        let fits = ((crate::render::VIRTUAL_HEIGHT - (TOP + 18.0) - 60.0) / ROW).floor().max(1.0) as usize;
        let at = self.cursor - first;
        let mut top_row = if self.scroll.get() != section as f32 { 0 } else { self.first_row.get() };
        if at < top_row {
            top_row = at;
        } else if at >= top_row + fits {
            top_row = at + 1 - fits;
        }
        top_row = top_row.min(rows.len().saturating_sub(fits));
        self.first_row.set(top_row);
        let target = (at - top_row) as f32 * ROW;
        // The highlight glides within a section and jumps when the section changes.
        let mut hl = self.highlight.get();
        if self.scroll.get() != section as f32 || !hl.is_finite() {
            hl = target;
            self.scroll.set(section as f32);
        }
        approach(&mut hl, target, c.dt_ms, 35.0);
        self.highlight.set(hl);
        let list_top = TOP + 18.0;
        if rows.len() > fits {
            // Where the rows shown are in the whole list.
            let (bar_h, all_h) = (fits as f32 * ROW - 4.0, rows.len() as f32 * ROW);
            let (from, len) = (top_row as f32 * ROW / all_h * bar_h, (fits as f32 * ROW / all_h * bar_h).max(12.0));
            c.round_rect(cx + col + 4.0, list_top, 3.0, bar_h, 1.5, rgba(0x2a2a38, 1.0), rgba(0, 0.0), 0.0);
            c.round_rect(cx + col + 4.0, list_top + from, 3.0, len, 1.5, rgba(accent(), 0.9), rgba(0, 0.0), 0.0);
        }
        let rows_shown = rows.len().min(fits);
        for (k, (i, item)) in rows.iter().skip(top_row).take(fits).enumerate() {
            let y = list_top + k as f32 * ROW;
            let selected = *i == self.cursor;
            let hover = !selected && c.hovered(cx, y, col, ROW - 4.0);
            let bg = if selected { 0x34344a } else if hover { 0x262634 } else { 0x1d1d27 };
            c.round_rect(cx, y, col, ROW - 4.0, 6.0, rgba(bg, 0.95), rgba(0x2a2a38, 1.0), 1.0);
            c.hit(cx, y - 2.0, col, ROW, Hit::SettingRow(*i));
            c.text(cx + 12.0, y + 9.5, 8.0, rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0), item.label());
            // The value gets what the label leaves, and is cut short past that.
            let room = col - 36.0 - Canvas::text_width(8.0, item.label());
            if let Some(edit) = self.editing.as_ref().filter(|e| selected && e.item == *item) {
                // A text box in place of the value; the old value shows selected until typed over.
                let text = Canvas::fit_end(8.0, room - 24.0, &edit.text);
                let tw = Canvas::text_width(8.0, &text);
                let bw = (tw + 24.0).max(90.0).min(room);
                let right = cx + col - 12.0;
                c.round_rect(right + 6.0 - bw, y + 4.0, bw, ROW - 12.0, 4.0, rgba(0x15151d, 1.0), rgba(0xFFD34D, 1.0), 1.0);
                if edit.fresh && !text.is_empty() {
                    c.rect(right - 7.0 - tw, y + 7.0, tw + 2.0, ROW - 18.0, rgba(accent(), 0.45));
                }
                c.text_right(right - 6.0, y + 9.5, 8.0, rgba(0xFFD34D, 1.0), &text);
                // At the caret in the text (at the end when the start was cut off to fit).
                let at = if text == edit.text { caret_x(8.0, &text, edit.caret) } else { tw };
                c.rect(right - 6.0 - tw + at, y + 7.0, 1.0, ROW - 18.0, rgba(0xFFD34D, 1.0));
                continue;
            }
            let v = if selected && self.binding.is_some() {
                tr!("press a key...", "キーを押してください...", "нажмите клавишу...").to_string()
            } else if selected && item.kind() != Kind::Key {
                format!("<  {}  >", Canvas::fit(8.0, room - Canvas::text_width(8.0, "<    >"), &value(*item)))
            } else {
                Canvas::fit(8.0, room, &value(*item))
            };
            let v_col = if selected && self.binding.is_some() { 0xFFD34D } else { 0xffffff };
            c.text_right(cx + col - 12.0, y + 9.5, 8.0, rgba(v_col, 1.0), &v);
            if selected && self.binding.is_none() && item.kind() != Kind::Key {
                // The "<" and ">" around the value step it.
                let tw = Canvas::text_width(8.0, &v);
                c.hit(cx + col - 12.0 - tw - 6.0, y, 20.0, ROW - 4.0, Hit::SettingStep(*i, -1));
                c.hit(cx + col - 26.0, y, 24.0, ROW - 4.0, Hit::SettingStep(*i, 1));
            }
        }
        c.round_rect(cx, list_top + hl, col, ROW - 4.0, 6.0, rgba(0, 0.0), rgba(accent(), 1.0), 1.5);

        // What the selected setting does.
        let help_y = list_top + rows_shown as f32 * ROW + 10.0;
        // Two lines, spaced for however tall the text comes out (taller on a text grid).
        let glyph_h = Canvas::text_height(7.0);
        let line_h = glyph_h + 4.0;
        c.round_rect(cx, help_y, col, line_h * 2.0 + 8.0, 6.0, rgba(0x15151d, 0.9), rgba(0x2a2a38, 1.0), 1.0);
        let help = self.item().help();
        let (line1, line2) = split_help(help, 7.0, col - 20.0);
        // `text` centres taller glyphs on the line; this puts their top 6 below the box's.
        let first = help_y + 6.0 + (glyph_h - 7.0) / 2.0;
        c.text(cx + 10.0, first, 7.0, rgba(0x9a9ab0, 1.0), &line1);
        c.text(cx + 10.0, first + line_h, 7.0, rgba(0x9a9ab0, 1.0), &line2);
        c.text_centered(w / 2.0, 462.0, 8.0, rgba(0x55556a, 1.0), &Canvas::fit_hints(8.0, w - 48.0, footer));
    }
}

/// Wraps help text onto two lines at most `max_w` wide.
fn split_help(text: &str, size: f32, max_w: f32) -> (String, String) {
    if Canvas::text_width(size, text) <= max_w {
        return (text.to_string(), String::new());
    }
    // After the last space (or Japanese comma or full stop) that keeps the line within the
    // width, else where it's full.
    let full = Canvas::fitting_prefix(size, max_w, text);
    let next = full + text[full..].chars().next().map_or(0, char::len_utf8);
    let cut = text[..next]
        .char_indices()
        .filter_map(|(i, ch)| match ch {
            ' ' => Some(i),
            '、' | '。' => Some(i + ch.len_utf8()).filter(|&end| end <= full),
            _ => None,
        })
        .last()
        .filter(|&i| i > 0)
        .unwrap_or(full);
    (text[..cut].to_string(), Canvas::fit(size, max_w, text[cut..].trim_start()))
}

/// Top bar shared by the song list, settings and multiplayer: logo plus the three tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    thread_local! {
        /// Where the active tab's highlight is (x, width), so it slides between tabs.
        static PILL: Cell<(f32, f32)> = const { Cell::new((f32::NAN, 0.0)) };
    }
    crate::logo::draw(c, 22.0, 18.0, 26.0);
    // The name like the wordmark: letters alternating white and red, "-rs" red.
    let mut tx = 54.0;
    for (i, ch) in "taiko.rs".chars().enumerate() {
        let s = ch.to_string();
        let col = match ch {
            '.' => 0x6f6f86,
            'r' | 's' => 0xEB452C,
            _ if i % 2 == 0 => 0xffffff,
            _ => 0xEB452C,
        };
        tx += c.text(tx, 24.0, 16.0, rgba(col, 1.0), &s);
    }
    let x0 = tx + 24.0;
    let names = [
        tr!("SONGS", "曲", "ПЕСНИ"),
        tr!("SETTINGS", "設定", "НАСТРОЙКИ"),
        tr!("MULTIPLAYER", "マルチ", "МУЛЬТИПЛЕЕР"),
        tr!("DOWNLOAD", "ダウンロード", "ЗАГРУЗКА"),
    ];
    let mut x = x0;
    let mut target = (x0, 0.0);
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        let on = i == active;
        let hover = !on && c.hovered(x, 20.0, tw + 20.0, 22.0);
        let fill = if hover { 0x34344a } else { 0x23232f };
        c.round_rect(x, 20.0, tw + 20.0, 22.0, 6.0, rgba(fill, 1.0), rgba(0x3a3a4e, 1.0), 1.0);
        c.hit(x, 20.0, tw + 20.0, 22.0, Hit::Tab(i));
        if on {
            target = (x, tw + 20.0);
        }
        x += tw + 28.0;
    }
    let (mut px, mut pw) = PILL.with(|p| p.get());
    if !px.is_finite() {
        (px, pw) = target;
    }
    approach(&mut px, target.0, c.dt_ms, 40.0);
    approach(&mut pw, target.1, c.dt_ms, 40.0);
    PILL.with(|p| p.set((px, pw)));
    c.round_rect(px, 20.0, pw, 22.0, 6.0, rgba(accent(), 1.0), rgba(accent(), 1.0), 1.0);
    let mut x = x0;
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        c.text(x + 10.0, 27.0, 9.0, rgba(if i == active { 0xffffff } else { 0x9a9ab0 }, 1.0), name);
        x += tw + 28.0;
    }
    c.text(x + 4.0, 27.0, 8.0, rgba(0x55556a, 1.0), "Tab");
    // Beside the tabs when there's room, else in small print under them (narrow screens),
    // clear of the friends chip in the top-right corner.
    let right = c.width - 24.0;
    let room = right - (x + 4.0 + Canvas::text_width(8.0, "Tab") + 16.0);
    if Canvas::text_width(8.0, right_text) <= room {
        c.text_right(right, 28.0, 8.0, rgba(0x9a9ab0, 1.0), right_text);
    } else {
        c.text(x0 + 2.0, 45.0, 6.0, rgba(0x9a9ab0, 1.0), &Canvas::fit(6.0, c.width - 110.0 - x0, right_text));
    }
}
