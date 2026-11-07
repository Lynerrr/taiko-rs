//! Settings tab: the list of options and how it is drawn. Values are read and changed by the
//! caller (it owns the config, audio engine and window).

use std::cell::Cell;

use crate::render::{Canvas, Glide, Hit, accent, approach, rgba};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    ScrollSpeed,
    PlayfieldArea,
    InterfaceEditor,
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
    MenuGlitch,
    BlackLane,
    TrueNotes,
    CalmMaps,
    DrumMenus,
    HideGreat,
    ChatFilter,
    MessageSound,
    NotifyPlaying,
    NotifyOnline,
    NotifyMessages,
    NotifyMentions,
    NotifyRequests,
    ShowConverts,
    SongHeadings,
    HiddenMaps,
    SongsFolder,
    DataFolder,
    MasterVolume,
    MusicVolume,
    UnfocusedVolume,
    FxVolume,
    AudioBuffer,
    BackgroundDim,
    BackgroundDimBlack,
    PresentMode,
    /// The graphics API (`config::RENDERERS`), from the next start.
    Renderer,
    MenuVsync,
    HalfResolution,
    FpsCap,
    ShowFps,
    Animations,
    Splash,
    /// The flashing-lights warning at startup (`Config::epilepsy_warning`).
    EpilepsyWarning,
    /// The old layout: tabs on top and no main menu (`Config::main_menu` off).
    LegacyMenu,
    /// The song's notes behind the main menu.
    MenuNotes,
    MenuNotesDim,
    /// The game smaller than the screen, black around it: on or off, its size, and where it sits.
    Letterbox,
    LetterboxSize,
    LetterboxX,
    LetterboxY,
    /// Clears every map's own offset, on a second press.
    ResetMapOffsets,
    /// Shows the first launch's guide again.
    Guide,
    CheckUpdates,
    UpdateChannel,
    /// What the global offset is set by: osu!'s universal offset or the sound.
    OffsetScale,
    Update,
    /// Opens what's new.
    Changelog,
    /// Only for krokodil_koban: closes taiko.rs and starts osu!stable.
    KrokodilKoban,
    HitFlash,
    Fullscreen,
    WindowSize,
    DiscordPresence,
    IdleScreen,
    /// After a crash, the next start sends its report in (`Config::send_crash_reports`).
    CrashReports,
    Skin,
    SkinFolder,
    MapSkins,
    NoteColours,
    DonColour,
    KatColour,
    Accent,
    AccentHue,
}

pub const ITEMS: [(&str, Item); 134] = [
    ("LIBRARY", Item::SongsFolder),
    ("LIBRARY", Item::ShowConverts),
    ("LIBRARY", Item::SongHeadings),
    ("LIBRARY", Item::HiddenMaps),
    ("LIBRARY", Item::SongSelectLayout),
    ("LIBRARY", Item::DataFolder),
    ("LIBRARY", Item::Screenshots),
    ("GAMEPLAY", Item::ScrollSpeed),
    ("GAMEPLAY", Item::Rate),
    ("GAMEPLAY", Item::RateKeepPitch),
    ("GAMEPLAY", Item::ResumeDelay),
    ("GAMEPLAY", Item::HideGreat),
    ("GAMEPLAY", Item::TrueNotes),
    ("GAMEPLAY", Item::CalmMaps),
    ("GAMEPLAY", Item::Interface),
    ("GAMEPLAY", Item::InterfaceHides),
    ("GAMEPLAY", Item::InterfaceKey),
    ("GAMEPLAY", Item::AutoplayStyle),
    ("GAMEPLAY", Item::AutoplayBinding),
    ("GAMEPLAY", Item::AutoplayLeftHanded),
    ("VIEW", Item::PlayfieldArea),
    ("VIEW", Item::InterfaceEditor),
    ("VIEW", Item::PlayfieldPreset),
    ("VIEW", Item::PlayfieldReset),
    ("VIEW", Item::ShowPlayfieldSize),
    ("VIEW", Item::WheelResize),
    ("VIEW", Item::BackgroundDim),
    ("VIEW", Item::BackgroundDimBlack),
    ("VIEW", Item::KiaiEffects),
    ("VIEW", Item::MenuGlitch),
    ("VIEW", Item::BlackLane),
    ("INPUT", Item::Key(0)),
    ("INPUT", Item::Key(1)),
    ("INPUT", Item::Key(2)),
    ("INPUT", Item::Key(3)),
    ("INPUT", Item::TouchLayout),
    ("INPUT", Item::DrumMenus),
    ("INPUT", Item::PresetKey(0)),
    ("INPUT", Item::PresetKey(1)),
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
    ("INPUT", Item::Hotkey(32)),
    ("INPUT", Item::Hotkey(33)),
    ("OFFSET", Item::Offset),
    ("OFFSET", Item::OffsetScale),
    ("OFFSET", Item::VisualOffset),
    ("OFFSET", Item::Calibrate),
    ("OFFSET", Item::ResetMapOffsets),
    ("OFFSET", Item::CalibrationBpm),
    ("OFFSET", Item::LiveOffset),
    ("OFFSET", Item::LiveOffsetUr),
    ("OFFSET", Item::VsyncCompensation),
    ("OFFSET", Item::VsyncTune),
    ("OFFSET", Item::VsyncCalibrate),
    ("AUDIO", Item::MasterVolume),
    ("AUDIO", Item::MusicVolume),
    ("AUDIO", Item::FxVolume),
    ("AUDIO", Item::UnfocusedVolume),
    ("AUDIO", Item::AudioBuffer),
    ("VIDEO", Item::PresentMode),
    ("VIDEO", Item::Renderer),
    ("VIDEO", Item::MenuVsync),
    ("VIDEO", Item::HalfResolution),
    ("VIDEO", Item::FpsCap),
    ("VIDEO", Item::ShowFps),
    ("VIDEO", Item::Fullscreen),
    ("VIDEO", Item::WindowSize),
    ("VIDEO", Item::Animations),
    ("VIDEO", Item::Letterbox),
    ("VIDEO", Item::LetterboxSize),
    ("VIDEO", Item::LetterboxX),
    ("VIDEO", Item::LetterboxY),
    ("SKIN", Item::Skin),
    ("SKIN", Item::SkinParts),
    ("SKIN", Item::SkinFolder),
    ("SKIN", Item::MapSkins),
    ("SKIN", Item::HitFlash),
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
    ("ONLINE", Item::DiscordPresence),
    ("ONLINE", Item::CrashReports),
    ("ONLINE", Item::KrokodilKoban),
    ("SYSTEM", Item::Language),
    ("SYSTEM", Item::Splash),
    ("SYSTEM", Item::EpilepsyWarning),
    ("SYSTEM", Item::IdleScreen),
    ("SYSTEM", Item::LegacyMenu),
    ("SYSTEM", Item::MenuNotes),
    ("SYSTEM", Item::MenuNotesDim),
    ("SYSTEM", Item::Guide),
    ("SYSTEM", Item::UpdateChannel),
    ("SYSTEM", Item::Update),
    ("SYSTEM", Item::Changelog),
    ("SYSTEM", Item::CheckUpdates),
];

/// The sections in order, with a one-line summary for the sidebar.
pub const SECTIONS: [(&str, &str); 10] = [
    ("LIBRARY", "Songs folder, hidden maps, data"),
    ("GAMEPLAY", "speed, rate, interface, autoplay"),
    ("VIEW", "playfield, background"),
    ("INPUT", "drum keys, hotkeys, touch"),
    ("OFFSET", "audio offset, calibration"),
    ("AUDIO", "volume, latency"),
    ("VIDEO", "frame pacing, display"),
    ("SKIN", "notes, hit flash, colour"),
    ("ONLINE", "chat, notifications, Discord"),
    ("SYSTEM", "language, startup, updates"),
];

/// First item of section `s`.
pub fn section_start(s: usize) -> usize {
    ITEMS
        .iter()
        .position(|(sec, _)| *sec == SECTIONS[s].0)
        .unwrap_or(0)
}

/// Section `s`'s name, as shown (`SECTIONS` has the English, which `ITEMS` refers to it by).
pub fn section_name(s: usize) -> &'static str {
    match SECTIONS.get(s).map_or("", |s| s.0) {
        "LIBRARY" => tr!("LIBRARY"),
        "GAMEPLAY" => tr!("GAMEPLAY"),
        "VIEW" => tr!("VIEW"),
        "INPUT" => tr!("INPUT"),
        "OFFSET" => tr!("OFFSET"),
        "AUDIO" => tr!("AUDIO"),
        "VIDEO" => tr!("VIDEO"),
        "SKIN" => tr!("SKIN"),
        "ONLINE" => tr!("ONLINE"),
        "SYSTEM" => tr!("SYSTEM"),
        other => other,
    }
}

/// Section `s`'s one-line summary for the sidebar.
pub fn section_summary(s: usize) -> &'static str {
    match SECTIONS.get(s).map_or("", |s| s.0) {
        "LIBRARY" => tr!("Songs folder, hidden maps, data"),
        "GAMEPLAY" => tr!("speed, rate, interface"),
        "VIEW" => tr!("playfield, background"),
        "INPUT" => tr!("drum keys, hotkeys, touch"),
        "OFFSET" => tr!("audio offset, calibration"),
        "AUDIO" => tr!("volume, latency"),
        "VIDEO" => tr!("frame pacing, display"),
        "SKIN" => tr!("notes, hit flash, colour"),
        "ONLINE" => tr!("chat, notifications, Discord"),
        "SYSTEM" => tr!("language, startup, updates"),
        _ => "",
    }
}

/// The small heading above a row that starts a group of its section.
pub fn group_heading(item: Item) -> Option<&'static str> {
    Some(match item {
        Item::Interface => tr!("Interface"),
        Item::AutoplayStyle => tr!("Autoplay"),
        Item::PlayfieldArea => tr!("Playfield"),
        Item::BackgroundDim => tr!("Background"),
        Item::Key(0) => tr!("Drum"),
        Item::PresetKey(0) => tr!("Playfield presets"),
        Item::Hotkey(0) => tr!("In a song"),
        Item::Hotkey(6) => tr!("Song select"),
        Item::Hotkey(16) => tr!("Results"),
        Item::Hotkey(19) => tr!("Anywhere"),
        Item::VsyncCompensation => tr!("Vsync"),
        Item::NoteColours => tr!("Colours"),
        Item::Letterbox => tr!("Black bars"),
        Item::NotifyMessages => tr!("Notifications"),
        Item::UpdateChannel => tr!("Updates"),
        _ => return None,
    })
}

/// A line of the settings list.
#[derive(Clone, Copy, Debug)]
pub enum Listed {
    Heading(&'static str),
    /// A row: index into `ITEMS`.
    Row(usize),
}

/// How a row shows its value, besides as text.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Control {
    Text,
    /// A switch, on or off.
    Switch(bool),
    /// A bar from 0 to 1, set by dragging it.
    Level(f32),
    /// A colour shown beside its value.
    Swatch(u32),
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
        0 => tr!("Left rim (kat)"),
        1 => tr!("Left centre (don)"),
        2 => tr!("Right centre (don)"),
        _ => tr!("Right rim (kat)"),
    }
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::ScrollSpeed => tr!("Scroll speed"),
            Item::PlayfieldArea => tr!("Playfield area"),
            Item::InterfaceEditor => tr!("Interface editor"),
            Item::PlayfieldReset => tr!("Reset playfield area"),
            Item::PlayfieldPreset => tr!("Playfield preset"),
            Item::Rate => tr!("Speed (rate)"),
            Item::RateKeepPitch => tr!("Rate keeps pitch"),
            Item::Offset => tr!("Global offset"),
            Item::OffsetScale => tr!("Offset scale"),
            Item::VisualOffset => tr!("Visual offset"),
            Item::Calibrate => tr!("Offset calibration"),
            Item::CalibrationBpm => tr!("Calibration tempo"),
            Item::LiveOffset => tr!("Live offset suggestion"),
            Item::LiveOffsetUr => tr!("Scale suggestion to UR"),
            Item::VsyncCompensation => {
                tr!("Vsync compensation")
            }
            Item::VsyncTune => tr!("Vsync compensation: fine tune"),
            Item::VsyncCalibrate => tr!("Vsync calibration"),
            Item::ResumeDelay => tr!("Resume delay"),
            Item::Interface => tr!("Interface"),
            Item::InterfaceHides => tr!("Interface toggle hides"),
            Item::InterfaceKey => tr!("Interface toggle key"),
            Item::SongSelectLayout => tr!("Song select layout"),
            Item::AutoplayStyle => tr!("Autoplay playstyle"),
            Item::AutoplayBinding => tr!("Autoplay binding"),
            Item::AutoplayLeftHanded => tr!("Autoplay left-handed"),
            Item::Key(i) => key_label(i),
            Item::PresetKey(0) => tr!("Previous preset key"),
            Item::PresetKey(_) => tr!("Next preset key"),
            Item::Hotkey(i) => crate::keys::Hotkey::ALL[i].label(),
            Item::TouchLayout => tr!("Touch layout"),
            Item::Screenshots => tr!("Screenshots"),
            // Also in English, so anyone who picked a language by mistake finds their way back.
            Item::Language => tr!("Language"),
            Item::ShowPlayfieldSize => tr!("Show playfield size"),
            Item::WheelResize => tr!("Wheel resizes playfield"),
            Item::KiaiEffects => tr!("Kiai effects"),
            Item::MenuGlitch => tr!("Menu glitch"),
            Item::BlackLane => tr!("Black lane"),
            Item::DrumMenus => tr!("Drum keys in menus"),
            Item::CalmMaps => tr!("Reduce map effects"),
            Item::TrueNotes => tr!("Show true notes"),
            Item::HideGreat => tr!("Hide GREAT splash"),
            Item::MapSkins => tr!("Beatmap skins"),
            Item::HitFlash => tr!("Hit flash"),
            Item::ChatFilter => tr!("Chat filter"),
            Item::MessageSound => tr!("Message sound"),
            Item::NotifyMessages => tr!("Notify: messages"),
            Item::NotifyMentions => tr!("Notify: chat mentions"),
            Item::NotifyRequests => tr!("Notify: friend requests"),
            Item::NotifyOnline => tr!("Notify: friend online"),
            Item::NotifyPlaying => tr!("Notify: friend playing"),
            Item::ShowConverts => tr!("Show osu!standard converts"),
            Item::SongHeadings => tr!("Song list headings"),
            Item::HiddenMaps => tr!("Hidden maps"),
            Item::SongsFolder => tr!("Songs folder"),
            Item::DataFolder => tr!("Game data folder"),
            Item::MasterVolume => tr!("Master volume"),
            Item::MusicVolume => tr!("Music volume"),
            Item::FxVolume => tr!("Hitsound volume"),
            Item::AudioBuffer => tr!("Audio buffer"),
            Item::BackgroundDim => tr!("Background dim"),
            Item::BackgroundDimBlack => {
                tr!("Dim to black")
            }
            Item::PresentMode => tr!("Present mode (gameplay)"),
            Item::Renderer => tr!("Renderer"),
            Item::MenuVsync => tr!("Menus use vsync"),
            Item::HalfResolution => tr!("Half resolution"),
            Item::FpsCap => tr!("FPS cap"),
            Item::ShowFps => tr!("FPS overlay"),
            Item::Animations => tr!("UI animations"),
            Item::Splash => tr!("Startup splash"),
            Item::EpilepsyWarning => tr!("Epilepsy warning"),
            Item::LegacyMenu => tr!("Legacy menu"),
            Item::Letterbox => tr!("Letterbox"),
            Item::LetterboxSize => tr!("Letterbox resolution"),
            Item::LetterboxX => tr!("Letterbox position: sideways"),
            Item::LetterboxY => tr!("Letterbox position: up and down"),
            Item::MenuNotes => tr!("Notes behind the main menu"),
            Item::UnfocusedVolume => tr!("Volume in the background"),
            Item::MenuNotesDim => tr!("Menu lane notes dim"),
            Item::ResetMapOffsets => tr!("Reset map offsets"),
            Item::Guide => tr!("First steps guide"),
            Item::UpdateChannel => tr!("Build channel"),
            Item::Update => tr!("Update"),
            Item::Changelog => tr!("What's new"),
            Item::KrokodilKoban => tr!("krokodil_koban mode"),
            Item::CheckUpdates => tr!("Check at startup"),
            Item::Fullscreen => tr!("Fullscreen"),
            Item::WindowSize => tr!("Window size"),
            Item::DiscordPresence => {
                tr!("Discord status")
            }
            Item::IdleScreen => tr!("Idle screen"),
            Item::CrashReports => tr!("Send crash reports"),
            Item::Skin => tr!("Skin"),
            Item::SkinParts => tr!("Skin parts"),
            Item::SkinPart(i) => crate::skin::Part::ALL
                .get(i)
                .map_or(tr!("Skin part"), |p| p.label()),
            Item::SkinFolder => tr!("Skins folder"),
            Item::Accent => tr!("UI colour"),
            Item::AccentHue => tr!("UI colour: fine tune"),
            Item::NoteColours => tr!("Note colours"),
            Item::DonColour => tr!("Don colour"),
            Item::KatColour => tr!("Kat colour"),
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
            | Item::UnfocusedVolume
            | Item::BackgroundDim
            | Item::MenuNotesDim
            | Item::FpsCap
            | Item::AccentHue
            | Item::VsyncTune
            | Item::DonColour
            | Item::KatColour
            | Item::LetterboxX
            | Item::LetterboxY => Kind::Value,
            Item::PlayfieldPreset
            | Item::TouchLayout
            | Item::Screenshots
            | Item::IdleScreen
            | Item::Language
            | Item::AudioBuffer
            | Item::PresentMode
            | Item::Renderer
            | Item::UpdateChannel
            | Item::OffsetScale
            | Item::Skin
            | Item::SkinParts
            | Item::SkinPart(_)
            | Item::NoteColours
            | Item::InterfaceHides
            | Item::AutoplayStyle
            | Item::AutoplayBinding
            | Item::WindowSize
            | Item::LetterboxSize
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
            | Item::MenuGlitch
            | Item::BlackLane
            | Item::DrumMenus
            | Item::MapSkins
            | Item::AutoplayLeftHanded
            | Item::HideGreat
            | Item::TrueNotes
            | Item::CalmMaps
            | Item::HitFlash
            | Item::ChatFilter
            | Item::MessageSound
            | Item::NotifyPlaying
            | Item::NotifyOnline
            | Item::NotifyMessages
            | Item::NotifyMentions
            | Item::NotifyRequests
            | Item::ShowConverts
            | Item::SongHeadings
            | Item::MenuVsync
            | Item::HalfResolution
            | Item::ShowFps
            | Item::Animations
            | Item::Splash
            | Item::EpilepsyWarning
            | Item::LegacyMenu
            | Item::Letterbox
            | Item::MenuNotes
            | Item::CheckUpdates
            | Item::Fullscreen
            | Item::DiscordPresence
            | Item::CrashReports => Kind::Toggle,
            Item::Key(_) | Item::PresetKey(_) | Item::InterfaceKey | Item::Hotkey(_) => Kind::Key,
            Item::PlayfieldArea
            | Item::InterfaceEditor
            | Item::PlayfieldReset
            | Item::ResetMapOffsets
            | Item::Guide
            | Item::Calibrate
            | Item::VsyncCalibrate
            | Item::HiddenMaps
            | Item::Update
            | Item::Changelog
            | Item::KrokodilKoban
            | Item::SongSelectLayout
            | Item::SkinFolder => Kind::Action,
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Item::ScrollSpeed => tr!("How fast notes travel. Shortcut: F3 / F4."),
            Item::PlayfieldArea => tr!(
                "Enter: edit where gameplay is drawn (drag, its edges, the wheel). In game: drag it about too."
            ),
            Item::InterfaceEditor => tr!(
                "Enter: move every part of the interface, add the UR and hit counters, and keep layouts as presets."
            ),
            Item::PlayfieldReset => tr!("Enter: make gameplay fill the whole screen again."),
            Item::PlayfieldPreset => {
                tr!("Enter: pick from a list (grouped by aspect ratio). Left/Right: previous/next.")
            }
            Item::Rate => tr!(
                "Song speed like DT (1.50x) / HT (0.75x). Song select: [ and ] (Shift = 0.25x steps)."
            ),
            Item::RateKeepPitch => tr!(
                "On: music sounds normal, just faster (DT). Off: pitch rises with speed (nightcore)."
            ),
            Item::Offset => tr!(
                "For every map. Positive brings the notes earlier, so lower it if you hit late. In a song, - / + set that map's own offset on top. Where 0 is depends on the offset scale below."
            ),
            Item::OffsetScale => tr!(
                "osu!: 0 is osu!'s, so an osu! universal offset carries over as it is. By sound: 0 is hitting exactly when you hear the note (the audio latency is taken off), and the number is how far ahead of the sound you hit. Switching keeps your timing as it is."
            ),
            Item::Calibrate => tr!(
                "Enter: hit along to a click track by ear, with no notes to watch; it measures and sets your offset."
            ),
            Item::CalibrationBpm => {
                tr!("Tempo of the calibration click track. Pick one close to the maps you play.")
            }
            Item::LiveOffset => tr!(
                "While playing, suggest an offset from your recent hits. To apply it with a key, give \"Song: apply offset suggestion\" one in the hotkeys."
            ),
            Item::LiveOffsetUr => tr!(
                "On: more hits and a +- margin when your UR is high. Off: plain average of the last 40 hits."
            ),
            Item::VsyncCompensation => tr!(
                "With vsync, notes are drawn where they'll be once the frame is on screen, as with unlimited fps."
            ),
            Item::VsyncTune => tr!(
                "Added to the vsync compensation: raise it if you hit late by sight with vsync. Vsync calibration finds it."
            ),
            Item::VsyncCalibrate => tr!(
                "Enter: hit notes by sight, sound off, in vsync; it sets the fine tune. Calibrate the offset first."
            ),
            Item::ResumeDelay => {
                tr!("Countdown before the song continues after the pause menu (0 = at once).")
            }
            Item::Interface => tr!(
                "Score, accuracy, bars and more during play. Its key (Shift+Tab unless changed below) toggles it in a song."
            ),
            Item::InterfaceHides => tr!(
                "What hiding the interface hides. Enter opens the list; Enter or a click ticks a part on or off."
            ),
            Item::InterfaceKey => tr!(
                "Enter, then press the key (with Shift or not) that shows / hides the interface in a song. Delete: none."
            ),
            Item::Key(_) => tr!(
                "Enter, then press the new key (Delete: none, Esc: cancel). Backspace: back to the default."
            ),
            Item::PresetKey(_) => tr!(
                "Steps through playfield presets anywhere (also in game). Enter, then press a key. Delete: none."
            ),
            Item::Hotkey(_) => tr!(
                "Enter, then press the new key, Ctrl / Alt / Shift too (Delete: none, Esc: cancel). Backspace: the default."
            ),
            Item::Language => tr!(
                "The language of menus, settings and messages. Song titles stay as their maps have them."
            ),
            Item::Screenshots => tr!(
                "Where a screenshot goes: on the clipboard to paste, as a picture in the Screenshots folder, or both."
            ),
            Item::TouchLayout => tr!(
                "Touch screens: the screen is four columns, left to right (K = rim / kat, D = centre / don)."
            ),
            Item::ShowPlayfieldSize => {
                tr!("Show the playfield's resolution in the bottom-right corner during play.")
            }
            Item::WheelResize => tr!(
                "During play, the mouse wheel makes the playfield bigger or smaller (it always does in its editor)."
            ),
            Item::ShowConverts => {
                tr!("Also list standard maps, converted to taiko like osu! does.")
            }
            Item::SongHeadings => tr!(
                "Group songs under headings you open and close (stars, letters, BPM...). Off: one list in the sort's order; by difficulty, each difficulty on its own."
            ),
            Item::HiddenMaps => {
                tr!("Right-click a song to hide it here (files stay). Enter lists them all again.")
            }
            Item::SongsFolder => tr!(
                "The folder your maps are in. Enter: browse, type a path, or go back to finding osu!'s."
            ),
            Item::DataFolder => tr!(
                "Replays, scores, videos, downloaded maps, skins and caches. Enter: move them to another folder or drive."
            ),
            Item::DrumMenus => tr!(
                "Song select, its mods and the pause menu follow the drum: the rims go up and down, either face picks. In song select Tab moves between the song list and the rows of buttons above and below it, and your drum keys don't type unless the search box is picked."
            ),
            Item::MapSkins => tr!(
                "Maps draw with the skin images in their own folder. Off: your skin everywhere. Right-click a song to turn one map's off."
            ),
            Item::BlackLane => tr!(
                "The lane the notes scroll on, and the drum's panel, are pure black: no kiai tint, no skin's lane picture."
            ),
            Item::MenuGlitch => tr!(
                "Notes on the main menu glitch when hit, and the whole screen with them and between menus. Off: they still spin and jump."
            ),
            Item::KiaiEffects => tr!(
                "During kiai time, tint the lane and pulse the hit target with the beat. The main menu joins in too."
            ),
            Item::CalmMaps => tr!(
                ".tko maps flash, glitch and tear the screen a quarter as hard, and turn, zoom and move the playfield half as far. Notes are hit at the same moments."
            ),
            Item::TrueNotes => tr!(
                "Some .tko maps draw notes as another colour or size until just before the hit. On: every note shows what it is."
            ),
            Item::HideGreat => tr!(
                "No flash and no \"GREAT\" on 300s, like skins with an empty hit300: only OKs and misses show."
            ),
            Item::HitFlash => tr!(
                "The coloured glow on the hit target as a note is hit (OKs light it up orange). Off: none."
            ),
            Item::ChatFilter => {
                tr!("Stars out swear words in friends' messages, the global chat and lobby chats.")
            }
            Item::MessageSound => {
                tr!("A soft chime when a friend messages you (not while you're playing).")
            }
            Item::NotifyMessages => {
                tr!("A pop-up when a friend messages you (they still count as unread when off).")
            }
            Item::NotifyMentions => tr!("A pop-up when someone says your name in the global chat."),
            Item::NotifyRequests => tr!("A pop-up when someone sends you a friend request."),
            Item::NotifyOnline => tr!("A pop-up when a friend comes online."),
            Item::NotifyPlaying => {
                tr!("A pop-up each time a friend starts a map (click it to watch).")
            }
            Item::MasterVolume => {
                tr!("Scales music and hitsounds. Anywhere: hold Alt and scroll the mouse wheel.")
            }
            Item::UnfocusedVolume => tr!(
                "How loud the game stays while another window has focus. 100% doesn't change it."
            ),
            Item::MusicVolume | Item::FxVolume => tr!("Left / Right to adjust."),
            Item::AudioBuffer => {
                tr!("Smaller = less audio latency; raise it if you hear crackling.")
            }
            Item::BackgroundDim => {
                tr!("How dark the map background is during play. 100% hides it.")
            }
            Item::BackgroundDimBlack => tr!(
                "On: in songs the background dims to pure black, and is black when there's none. Off: the interface's grey."
            ),
            Item::PresentMode => tr!(
                "Used during songs. vsync is smoothest on Wayland; mailbox/immediate add judder."
            ),
            Item::Renderer => tr!(
                "What draws the game: picking one restarts it. If it closes right after starting or crashes, try another: Vulkan or DirectX 12 on Windows (Auto picks DirectX 12). OpenGL always uses vsync on Linux."
            ),
            Item::MenuVsync => tr!(
                "Everything but playing a song uses vsync: song select (and its autoplay preview), settings, results, replays and autoplay."
            ),
            Item::HalfResolution => tr!(
                "Draw at half resolution, scaled up exactly 2x: chunky, crisp pixels and a quarter of the GPU work. For high-DPI screens."
            ),
            Item::FpsCap => tr!("mailbox/immediate only. A few times your refresh rate is plenty."),
            Item::ShowFps => tr!("Frame rate and frame time in the corner. Shortcut: F10."),
            Item::Animations => tr!(
                "Menu motion: screen fades, sliding highlights, opening lists, results count-up. Gameplay is unaffected."
            ),
            Item::Splash => {
                tr!("The animated intro when taiko.rs starts (any key or click skips it).")
            }
            Item::EpilepsyWarning => tr!(
                "A warning about flashing lights before anything else when taiko.rs starts (any key or click closes it)."
            ),
            Item::ResetMapOffsets => tr!(
                "Clears the offset each map has of its own, back to just the audio offset. Press twice. They're kept in map_offsets.backup.tsv."
            ),
            Item::Guide => tr!("The short guide shown on the first launch."),
            Item::Letterbox => tr!(
                "In fullscreen, the whole game runs at a resolution of your own with black around it. Turning it on goes fullscreen."
            ),
            Item::LetterboxSize => tr!(
                "Every resolution the monitor reports, custom ones from the graphics driver too, or type your own."
            ),
            Item::LetterboxX => tr!(
                "Where the game sits between the sides of the screen: -100 left, 0 the middle, 100 right."
            ),
            Item::LetterboxY => tr!(
                "Where the game sits between the top and the bottom: -100 top, 0 the middle, 100 bottom."
            ),
            Item::LegacyMenu => tr!(
                "The old layout: straight to song select with tabs along the top, no main menu, and Esc asks to quit."
            ),
            Item::MenuNotesDim => tr!(
                "How faint the song's notes are on the main menu's lane. 100% hides them. The notes drifting behind the menu stay as they are."
            ),
            Item::MenuNotes => tr!(
                "The playing song's notes drift along routes behind the main menu, one for each Multiverse universe."
            ),
            Item::VisualOffset => tr!(
                "Draws notes earlier to make up for display lag; raise it if notes reach the drum after the beat. Judging stays on the audio. In a song: Ctrl + - / +."
            ),
            Item::UpdateChannel => tr!(
                "dev: test builds that haven't reached everyone yet (may break). Switch, then Update installs that channel's build."
            ),
            Item::Update => tr!(
                "Enter: look for a newer build, or install it (Ctrl+U in any menu). taiko.rs restarts into it."
            ),
            // No explanation: krokodil_koban knows.
            Item::KrokodilKoban => "",
            Item::AutoplayStyle => tr!(
                "Which hand autoplay hits each note with: kddk full / semi alternate, single tap, roll, or ddkk."
            ),
            Item::AutoplayBinding => tr!(
                "The order autoplay's four keys light the drum in (kddk): 1324 puts each hand's don on the other side."
            ),
            Item::AutoplayLeftHanded => {
                tr!("Autoplay leads with the left hand instead of the right.")
            }
            Item::SongSelectLayout => tr!(
                "Drag the edge between the song list and the map panel. Enter: back to the default."
            ),
            Item::Changelog => {
                tr!("Enter: the latest changes. They also show by themselves after an update.")
            }
            Item::CheckUpdates => tr!(
                "Look for a newer build each time taiko.rs starts and install it straight away (the game restarts by itself once you're in a menu), and for newer versions of the songs you pick on osu!."
            ),
            Item::Fullscreen => tr!("Shortcut: F11."),
            Item::WindowSize => tr!(
                "Enter: pick a size (or type one); Left/Right: the next size. Dragging the window's edges works too."
            ),
            Item::CrashReports => tr!(
                "After a crash, the next start sends its report to the developers: the build, the system, the graphics card and where it crashed. No account or folder names."
            ),
            Item::IdleScreen => tr!(
                "After this long without a key, click or move, menus fade away and the song's disc turns over the blurred background. Anything brings them back."
            ),
            Item::DiscordPresence => tr!(
                "Show the map you're playing, your lobby or who you spectate in your Discord status."
            ),
            Item::Skin => {
                tr!("Enter opens the list. osu! skins from osu!'s or taiko.rs's Skins folder.")
            }
            Item::SkinParts | Item::SkinPart(_) => tr!(
                "What's drawn from a skin, and which: the skin above, built-in, or any other (mix them). Enter opens the list."
            ),
            Item::SkinFolder => tr!(
                "Enter: open taiko.rs's Skins folder. Drop .osk files there, then pick them above."
            ),
            Item::Accent => tr!(
                "The interface's accent; any but red tints the greys too. Left/Right: presets (or #rrggbb in the config)."
            ),
            Item::AccentHue => tr!(
                "Left/Right: turn the UI colour round the colour wheel, for any colour in between."
            ),
            Item::NoteColours => tr!(
                "Dons' and kats' colours (Multiverse's universes have their own: M in the mod select)."
            ),
            Item::DonColour | Item::KatColour => tr!(
                "Enter: type a colour like #eb452c. Left/Right turn it round the colour wheel. Makes the colours your own."
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
    /// What's typed into the search box, and the rows it finds (`ITEMS` indices, best first).
    pub search: String,
    results: Vec<usize>,
    /// What's listed as last drawn (a section, or the search's results: `RESULTS`) and the
    /// animated highlight position in it.
    shown: Cell<Option<usize>>,
    highlight: Cell<f32>,
    /// How far the list is scrolled, and how far it goes.
    scroll: Cell<f32>,
    scroll_max: Cell<f32>,
    /// The list moved by the wheel. Left out, it keeps the cursor in view.
    free: Cell<Option<Glide>>,
    /// Where the bars of `Control::Level` were drawn (x, width).
    track: Cell<(f32, f32)>,
}

/// Top of the settings screen, a row's height, a group heading's height, a wheel notch.
const TOP: f32 = 62.0;
const ROW: f32 = 30.0;
const GROUP_H: f32 = 18.0;
const WHEEL_STEP: f32 = ROW * 2.0;
/// `SettingsUi::shown` for the search's results.
const RESULTS: usize = usize::MAX;

/// A line of the list: a group's heading or a row (`ITEMS` index), and how far down it is.
enum Line {
    Group(&'static str, f32),
    Row(usize, f32),
}

/// The playfield preset dropdown: a cursor over the flattened list, plus where each row was
/// drawn (virtual units) so the mouse can hover and click them. The first group is the player's
/// own saved sizes, which can be added, edited, deleted and reordered.
#[derive(Default)]
pub struct Dropdown {
    pub cursor: usize,
    scroll: f32,
    /// Where the list goes when it next draws: centred on the cursor (just opened), or just far
    /// enough to show it (moved by key). Otherwise it stays where the wheel or the scroll bar left
    /// it: the pointer picking rows never moves it.
    follow: Follow,
    /// How far it scrolls, and the scroll bar's track and thumb height, as last drawn.
    max_scroll: f32,
    bar: Option<[f32; 4]>,
    thumb: f32,
    /// The scroll bar is held: where in the thumb it was grabbed.
    dragging: Option<f32>,
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

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Follow {
    #[default]
    Centre,
    Show,
    Stay,
}

/// How far a notch of the wheel moves the dropdown's list (virtual units: three rows).
const DROPDOWN_WHEEL: f32 = 45.0;

impl Dropdown {
    pub fn new(cursor: usize) -> Dropdown {
        Dropdown {
            cursor,
            ..Default::default()
        }
    }

    /// The cursor moved by key: the list scrolls to keep it in sight.
    pub fn show_cursor(&mut self) {
        self.follow = Follow::Show;
    }

    /// The wheel: moves the list `notches` (up positive), the cursor staying where it is.
    pub fn wheel(&mut self, notches: i32) {
        self.scroll = (self.scroll - notches as f32 * DROPDOWN_WHEEL).clamp(0.0, self.max_scroll);
        self.follow = Follow::Stay;
    }

    /// Whether (`x`, `y`) is on the scroll bar.
    pub fn on_bar(&self, x: f32, y: f32) -> bool {
        self.bar
            .is_some_and(|[bx, by, bw, bh]| inside(&[bx - 4.0, by, bw + 8.0, bh], x, y))
    }

    /// A press on the scroll bar: held by the thumb where it was grabbed, or (off the thumb)
    /// with the thumb's middle jumping to the pointer.
    pub fn grab_bar(&mut self, y: f32) {
        let Some([_, top, _, track]) = self.bar else {
            return;
        };
        let thumb_y = top + self.scroll / self.max_scroll.max(1.0) * (track - self.thumb);
        let grip = if (thumb_y..thumb_y + self.thumb).contains(&y) {
            y - thumb_y
        } else {
            self.thumb / 2.0
        };
        self.dragging = Some(grip);
        self.drag_bar(y);
    }

    /// The held scroll bar follows the pointer; false when it isn't held.
    pub fn drag_bar(&mut self, y: f32) -> bool {
        let (Some(grip), Some([_, top, _, track])) = (self.dragging, self.bar) else {
            return false;
        };
        let frac = ((y - grip - top) / (track - self.thumb).max(1.0)).clamp(0.0, 1.0);
        self.scroll = frac * self.max_scroll;
        self.follow = Follow::Stay;
        true
    }

    /// Lets go of the scroll bar; whether it was held.
    pub fn release_bar(&mut self) -> bool {
        self.dragging.take().is_some()
    }

    #[cfg(test)]
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    #[cfg(test)]
    pub fn bar(&self) -> Option<[f32; 4]> {
        self.bar
    }

    #[cfg(test)]
    pub fn max_scroll(&self) -> f32 {
        self.max_scroll
    }

    /// Index of the row under (`x`, `y`) in virtual units.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|(_, r)| inside(r, x, y))
            .map(|(i, _)| *i)
    }

    pub fn button_at(&self, x: f32, y: f32) -> Option<PresetAction> {
        self.buttons
            .iter()
            .find(|(_, r)| inside(r, x, y))
            .map(|(a, _)| *a)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        inside(&self.bounds, x, y)
    }

    fn button(&mut self, c: &mut Canvas, x: f32, y: f32, label: &str, action: PresetAction) -> f32 {
        let w = Canvas::text_width(6.0, label) + 10.0;
        let hover = c.hovered(x, y, w, 12.0);
        c.round_rect(
            [x, y],
            [w, 12.0],
            3.0,
            rgba(if hover { 0x4a4a64 } else { 0x2a2a3a }, 1.0),
            rgba(0x5a5a74, 1.0),
            1.0,
        );
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
        c.round_rect(
            [bx, by],
            [bw, bh],
            8.0,
            rgba(0x15151d, 0.98),
            rgba(accent(), 0.8),
            1.5,
        );
        let title_w = c.text(
            bx + 14.0,
            by + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            Item::PlayfieldPreset.label(),
        );
        let screen = tr!("screen {}x{}", native.0, native.1);
        c.text_right(
            bx + bw - 14.0,
            by + 13.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit(7.0, bw - 28.0 - title_w - 10.0, &screen),
        );
        let save_label = tr!("+ Save current {}x{}", playfield.0, playfield.1);
        let bw0 = self.button(
            c,
            bx + 14.0,
            by + 28.0,
            &save_label,
            PresetAction::SaveCurrent,
        );
        self.button(
            c,
            bx + 20.0 + bw0,
            by + 28.0,
            tr!("+ Add size"),
            PresetAction::Add,
        );

        let (top, view) = (by + 48.0, bh - 78.0);
        let focus = layout.iter().find(|(l, _)| match (l, &self.editing) {
            (Line::Input, _) => true,
            (Line::Row(i, ..), Some((Some(e), _))) => i == e,
            (Line::Row(i, ..), None) => *i == self.cursor,
            _ => false,
        });
        let focus_y = focus.map(|l| l.1).unwrap_or(0.0);
        self.max_scroll = (y - view).max(0.0);
        // A size being typed stays in sight; otherwise the list only moves when asked to.
        let follow = if self.editing.is_some() && self.follow == Follow::Stay {
            Follow::Show
        } else {
            self.follow
        };
        self.scroll = match follow {
            Follow::Centre => focus_y - view / 2.0,
            Follow::Show => self.scroll.min(focus_y).max(focus_y + ROW - view),
            Follow::Stay => self.scroll,
        }
        .clamp(0.0, self.max_scroll);
        if self.editing.is_none() {
            self.follow = Follow::Stay;
        }
        self.rows.clear();
        let caret = self.caret;
        let text_input = |c: &mut Canvas, ry: f32, text: &str| {
            c.round_rect(
                [bx + 8.0, ry],
                [bw - 16.0, ROW - 2.0],
                4.0,
                rgba(0x1d1d27, 1.0),
                rgba(0xFFD34D, 1.0),
                1.0,
            );
            c.text(bx + 22.0, ry + 3.5, 7.0, rgba(0xFFD34D, 1.0), text);
            c.rect(
                bx + 22.0 + caret_x(7.0, text, caret),
                ry + 2.5,
                1.0,
                9.0,
                rgba(0xFFD34D, 1.0),
            );
            c.text_right(
                bx + bw - 22.0,
                ry + 3.5,
                6.0,
                rgba(0x8a8aa0, 1.0),
                tr!("width x height"),
            );
        };
        for (line, ly) in layout {
            let ry = top + ly - self.scroll;
            if ry < top - 1.0 || ry + ROW > top + view + 1.0 {
                continue;
            }
            match line {
                Line::Head(name) => {
                    let name = match name {
                        SAVED => tr!("Saved"),
                        "Other" => tr!("Other"),
                        _ => name,
                    };
                    c.text(bx + 14.0, ry + 5.0, 7.0, rgba(accent(), 1.0), name);
                }
                Line::Hint => {
                    let hint = tr!("none yet: S saves the playfield's size, A adds one");
                    c.text(bx + 22.0, ry + 3.5, 6.0, rgba(0x6f6f86, 1.0), hint);
                }
                Line::Input => {
                    if let Some((_, text)) = &self.editing {
                        text_input(c, ry, text);
                    }
                }
                Line::Row(i, rw, rh) => {
                    if let Some((Some(e), text)) = &self.editing
                        && *e == i
                    {
                        text_input(c, ry, text);
                        continue;
                    }
                    let selected = i == self.cursor && self.editing.is_none();
                    if selected {
                        c.round_rect(
                            [bx + 8.0, ry],
                            [bw - 16.0, ROW - 2.0],
                            4.0,
                            rgba(0x34344a, 1.0),
                            rgba(accent(), 1.0),
                            1.0,
                        );
                    }
                    self.rows.push((i, [bx + 8.0, ry, bw - 16.0, ROW - 2.0]));
                    c.text(
                        bx + 22.0,
                        ry + 3.5,
                        7.0,
                        rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0),
                        &format!("{rw} x {rh}"),
                    );
                    let fits = rw <= native.0 && rh <= native.1;
                    let mut right = bx + bw - 22.0;
                    if selected && i < saved {
                        // Buttons for the saved size, right to left.
                        for (label, action) in [
                            (tr!("del"), PresetAction::Delete),
                            (tr!("edit"), PresetAction::Edit),
                            (tr!("down"), PresetAction::Down),
                            (tr!("up"), PresetAction::Up),
                        ] {
                            let lw = Canvas::text_width(6.0, label) + 10.0;
                            right -= lw;
                            self.button(c, right, ry + 0.5, label, action);
                            right -= 4.0;
                        }
                        right -= 6.0;
                    }
                    let (tag, col) = if !fits {
                        (tr!("too big"), 0x8a8aa0)
                    } else if Some((rw, rh)) == current {
                        (current_tag(), 0x86E05A)
                    } else {
                        ("", 0)
                    };
                    c.text_right(right, ry + 3.5, 7.0, rgba(col, 1.0), tag);
                }
            }
        }
        // The scroll bar, when there's more than fits: wheel, click or drag it.
        self.bar = None;
        if self.max_scroll > 0.0 {
            let (bar_x, bar_w) = (bx + bw - 7.0, 4.0);
            let bar = [bar_x, top, bar_w, view];
            self.bar = Some(bar);
            self.thumb = (view * view / (view + self.max_scroll)).clamp(18.0, view);
            let thumb_y = top + self.scroll / self.max_scroll * (view - self.thumb);
            let lit = self.dragging.is_some() || c.hovered(bar_x - 4.0, top, bar_w + 8.0, view);
            c.round_rect(
                [bar_x, top],
                [bar_w, view],
                2.0,
                rgba(0x0e0e14, 0.9),
                rgba(0x2a2a38, 1.0),
                1.0,
            );
            c.round_rect(
                [bar_x, thumb_y],
                [bar_w, self.thumb],
                2.0,
                rgba(if lit { accent() } else { 0x8a8aa8 }, 0.95),
                rgba(0, 0.0),
                0.0,
            );
        }
        let (line1, line2) = if self.editing.is_some() {
            (
                tr!("type a size like 1280x720   Enter save   Esc cancel"),
                "",
            )
        } else {
            (
                tr!("Up/Down or mouse choose   Enter/click apply   Esc close"),
                tr!("S save current   A add   E edit   Del delete   Shift+Up/Down move"),
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
        Edit {
            item,
            caret: text.chars().count(),
            text,
            fresh: true,
        }
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
    text.char_indices()
        .nth(caret)
        .map_or(text.len(), |(b, _)| b)
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
        Choices {
            item,
            names,
            current,
            note,
            cursor: current.unwrap_or(0),
        }
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
        c.round_rect(
            [bx, by],
            [bw, bh],
            8.0,
            rgba(0x15151d, 0.98),
            rgba(accent(), 0.8),
            1.5,
        );
        let title_w = c.text(
            bx + 14.0,
            by + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            self.item.label(),
        );
        c.text_right(
            bx + bw - 14.0,
            by + 13.0,
            7.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit_end(7.0, bw - 38.0 - title_w, &self.note),
        );
        let (top, view) = (by + 32.0, bh - 50.0);
        let scroll =
            (self.cursor as f32 * ROW - view / 2.0).clamp(0.0, (n as f32 * ROW - view).max(0.0));
        for (i, name) in self.names.iter().enumerate() {
            let y = top + i as f32 * ROW - scroll;
            if y < top - 1.0 || y + ROW > top + view + 1.0 {
                continue;
            }
            let selected = i == self.cursor;
            let hover = c.hovered(bx + 8.0, y, bw - 16.0, ROW - 2.0);
            if selected || hover {
                c.round_rect(
                    [bx + 8.0, y],
                    [bw - 16.0, ROW - 2.0],
                    4.0,
                    rgba(0x34344a, 1.0),
                    rgba(if selected { accent() } else { 0x3a3a4e }, 1.0),
                    1.0,
                );
            }
            c.hit(bx + 8.0, y, bw - 16.0, ROW, Hit::ChoiceRow(i));
            let current = self.current == Some(i);
            let room = bw
                - 40.0
                - if current {
                    Canvas::text_width(7.0, current_tag()) + 8.0
                } else {
                    0.0
                };
            c.text(
                bx + 20.0,
                y + 5.0,
                7.0,
                rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0),
                &Canvas::fit(7.0, room, name),
            );
            if current {
                c.text_right(
                    bx + bw - 20.0,
                    y + 5.0,
                    7.0,
                    rgba(0x86E05A, 1.0),
                    current_tag(),
                );
            }
        }
        let hint = tr!("Up/Down or mouse   Enter/click use   Esc close");
        c.text_centered(
            w / 2.0,
            by + bh - 11.0,
            6.0,
            rgba(0x6f6f86, 1.0),
            &Canvas::fit_hints(6.0, bw - 16.0, hint),
        );
    }
}

/// The mark on the choice in use.
fn current_tag() -> &'static str {
    tr!("current")
}

fn inside(r: &[f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x <= r[0] + r[2] && y >= r[1] && y <= r[1] + r[3]
}

impl SettingsUi {
    pub fn item(&self) -> Item {
        ITEMS[self.cursor].1
    }

    pub fn move_cursor(&mut self, delta: i32) {
        self.free.set(None);
        self.editing = None;
        if !self.search.is_empty() {
            let at = self
                .results
                .iter()
                .position(|&i| i == self.cursor)
                .unwrap_or(0) as i32;
            if let Some(&i) = self
                .results
                .get((at + delta).clamp(0, self.results.len() as i32 - 1) as usize)
            {
                self.cursor = i;
            }
            return;
        }
        let start = self.cursor;
        self.cursor = (self.cursor as i32 + delta).clamp(0, ITEMS.len() as i32 - 1) as usize;
        // Past a row that isn't shown, or back where it was at the end of the list.
        while !self.shows(ITEMS[self.cursor].1) {
            let next = self.cursor as i32 + delta.signum();
            self.cursor = if (0..ITEMS.len() as i32).contains(&next) && delta != 0 {
                next as usize
            } else {
                start
            };
            if self.cursor == start {
                break;
            }
        }
    }

    /// The wheel: moves the list `notches` on, within what's listed. The cursor stays.
    pub fn scroll_by(&self, notches: i32) {
        let mut glide = self.free.get().unwrap_or(Glide::at(self.scroll.get()));
        glide.goal = (glide.goal + notches as f32 * WHEEL_STEP).clamp(0.0, self.scroll_max.get());
        self.free.set(Some(glide));
    }

    /// `text` in the search box: lists what it finds and puts the cursor on the best of it.
    /// `key` gives a key setting's key as shown.
    pub fn set_search(&mut self, text: String, key: &dyn Fn(Item) -> String) {
        self.search = text;
        self.free.set(None);
        self.editing = None;
        self.results = if self.search.trim().is_empty() {
            Vec::new()
        } else {
            crate::settings_search::find(&self.search, key, &|item| self.shows(item))
        };
        if let Some(&first) = self.results.first() {
            self.cursor = first;
        }
    }

    /// Back to the sections.
    pub fn clear_search(&mut self) {
        self.search.clear();
        self.results.clear();
        self.free.set(None);
    }

    /// The bar's value under the pointer at `x`.
    pub fn level_at(&self, x: f32) -> f32 {
        let (left, w) = self.track.get();
        ((x - left) / w.max(1.0)).clamp(0.0, 1.0)
    }

    /// Whether `item` is in the list (krokodil_koban mode is only for them, and a phone's screen
    /// has no use for black bars).
    pub fn shows(&self, item: Item) -> bool {
        let letterbox = matches!(
            item,
            Item::Letterbox | Item::LetterboxSize | Item::LetterboxX | Item::LetterboxY
        );
        (item != Item::KrokodilKoban || self.krokodil)
            && !(letterbox && cfg!(target_os = "android"))
    }

    /// Section of the selected item.
    pub fn section(&self) -> usize {
        SECTIONS
            .iter()
            .position(|(name, _)| *name == ITEMS[self.cursor].0)
            .unwrap_or(0)
    }

    /// What's listed for `section`, for a screen with its own row sizes.
    pub fn listed(&self, section: usize) -> Vec<Listed> {
        self.lines(section)
            .0
            .into_iter()
            .map(|line| match line {
                Line::Group(name, _) => Listed::Heading(name),
                Line::Row(i, _) => Listed::Row(i),
            })
            .collect()
    }

    /// Where the bars of the rows are, for `level_at`.
    pub fn set_track(&self, left: f32, w: f32) {
        self.track.set((left, w));
    }

    pub fn track_rect(&self) -> (f32, f32) {
        self.track.get()
    }

    /// What's listed, in order: the section's rows under their groups' headings, or the
    /// search's results.
    fn lines(&self, section: usize) -> (Vec<Line>, f32) {
        let mut y = 0.0;
        if !self.search.is_empty() {
            let lines = self
                .results
                .iter()
                .enumerate()
                .map(|(k, &i)| Line::Row(i, k as f32 * ROW))
                .collect::<Vec<_>>();
            return (lines, self.results.len() as f32 * ROW);
        }
        let mut lines = Vec::new();
        for (i, (sec, item)) in ITEMS.iter().enumerate() {
            if *sec != SECTIONS[section].0 || !self.shows(*item) {
                continue;
            }
            if let Some(heading) = group_heading(*item) {
                lines.push(Line::Group(heading, y));
                y += GROUP_H;
            }
            lines.push(Line::Row(i, y));
            y += ROW;
        }
        (lines, y)
    }

    /// A sidebar of sections and the selected section's settings, or what the search found.
    /// `value` gives the current display value of each item, `control` how it's shown.
    pub fn draw(
        &self,
        c: &mut Canvas,
        value: &dyn Fn(Item) -> String,
        control: &dyn Fn(Item) -> Control,
        footer: &str,
    ) {
        let w = c.width;
        let total = (w - 48.0).min(780.0);
        let x0 = (w - total) / 2.0;
        let side_w = 170.0;
        let (cx, col) = (x0 + side_w + 16.0, total - side_w - 16.0);
        let section = self.section();
        let searching = !self.search.is_empty();

        // Sidebar: one panel, each section a note on it (don and kat in turn, like a lane): the
        // open one bigger and lit, the others dimmed, the selector sliding between them. While
        // searching, how much was found in each section.
        const PITCH: f32 = 40.0;
        crate::style::panel(
            c,
            x0 - 6.0,
            TOP - 8.0,
            side_w + 12.0,
            SECTIONS.len() as f32 * PITCH + 4.0,
        );
        let colours = crate::multiverse::note_colours();
        for (i, (id, _)) in SECTIONS.iter().enumerate() {
            let name = section_name(i);
            let found = self.results.iter().filter(|&&r| ITEMS[r].0 == *id).count();
            let about = if searching {
                tr!("found: {found}"; found)
            } else {
                section_summary(i).to_string()
            };
            let y = TOP + i as f32 * PITCH;
            let on = i == section && !searching;
            let hover = !on && c.hovered(x0, y, side_w, 34.0);
            let dim = searching && found == 0;
            if hover {
                c.round_rect(
                    [x0, y],
                    [side_w, 34.0],
                    7.0,
                    rgba(0x262634, 0.6),
                    rgba(0, 0.0),
                    0.0,
                );
            }
            let (nx, ny) = (x0 + 17.0, y + 17.0);
            let col = if i % 2 == 0 {
                colours.don()
            } else {
                colours.kat()
            };
            let r = if on { 11.0 } else { 8.0 };
            if on {
                crate::style::selector(
                    c,
                    "settings-section",
                    [x0 - 2.0, y - 1.0, side_w + 4.0, 36.0],
                );
            }
            crate::style::note(c, nx, ny, r, col, on, if dim { 0.35 } else { 0.6 });
            c.hit(x0, y, side_w, 34.0, Hit::SettingsSection(i));
            let text = if on || hover {
                0xffffff
            } else if dim {
                0x6f6f86
            } else {
                0xa8a8bc
            };
            c.text(x0 + 36.0, y + 7.0, 9.0, rgba(text, 1.0), name);
            c.text(
                x0 + 36.0,
                y + 21.0,
                6.0,
                rgba(0x8a8aa0, 1.0),
                &Canvas::fit(6.0, side_w - 44.0, &about),
            );
        }

        // The list: it scrolls by itself to keep the cursor in sight, and by the wheel.
        let list_top = TOP + 18.0;
        let (lines, height) = self.lines(section);
        // Room for the rows: the panel (help lines under them, however tall the text comes out)
        // ends above the keys' hint at the bottom.
        let help_room = 10.0 + 2.0 * (Canvas::text_height(7.0) + 4.0) + 14.0;
        let view = ((crate::render::VIRTUAL_HEIGHT - 26.0 - help_room - list_top) / ROW)
            .floor()
            .max(1.0)
            * ROW;
        let max = (height - view).max(0.0);
        self.scroll_max.set(max);
        // One panel for the section: its title, its rows and what the selected one does.
        let list_h = height.min(view);
        let glyph_h = Canvas::text_height(7.0);
        let line_h = glyph_h + 4.0;
        let help_y = list_top + list_h.max(ROW) + 10.0;
        crate::style::panel(
            c,
            cx - 10.0,
            TOP - 14.0,
            col + 20.0,
            help_y + line_h * 2.0 + 14.0 - (TOP - 14.0),
        );

        // The title and the search box beside it: typing anywhere goes into it.
        let box_w = (col * 0.5).min(240.0);
        let box_x = cx + col - box_w;
        let n = self.results.len();
        let title = if searching {
            tr!("Found: {n}"; n)
        } else {
            section_name(section).to_string()
        };
        crate::style::title(
            c,
            cx,
            TOP - 4.0,
            13.0,
            accent(),
            &Canvas::fit(13.0, col - box_w - 8.0, &title),
        );
        c.round_rect(
            [box_x, TOP - 7.0],
            [box_w, 19.0],
            5.0,
            rgba(0x23232f, 0.9),
            rgba(if searching { accent() } else { 0x3a3a4e }, 1.0),
            1.0,
        );
        let (text, col_text) = if searching {
            (
                Canvas::fit_end(8.0, box_w - 18.0, &format!("{}_", self.search)),
                0xffffff,
            )
        } else {
            (
                Canvas::fit(8.0, box_w - 18.0, tr!("type to search...")),
                0x6f6f86,
            )
        };
        c.text(box_x + 9.0, TOP - 1.5, 8.0, rgba(col_text, 1.0), &text);
        c.hit(box_x, TOP - 7.0, box_w, 19.0, Hit::SettingsSearch);

        let listing = if searching { RESULTS } else { section };
        let fresh = self.shown.get() != Some(listing);
        if fresh {
            self.shown.set(Some(listing));
            self.free.set(None);
            self.scroll.set(0.0);
        }
        let cursor_y = lines.iter().find_map(|l| match l {
            Line::Row(i, y) if *i == self.cursor => Some(*y),
            _ => None,
        });
        let mut scroll = self.scroll.get();
        if let Some(mut glide) = self.free.get() {
            glide.settle(max, c.dt_ms, 45.0);
            scroll = glide.now;
            self.free.set(Some(glide));
        } else if let Some(y) = cursor_y {
            let target = if y - GROUP_H < scroll {
                y - GROUP_H
            } else if y + ROW > scroll + view {
                y + ROW - view
            } else {
                scroll
            };
            if fresh {
                scroll = target.clamp(0.0, max);
            } else {
                approach(&mut scroll, target.clamp(0.0, max), c.dt_ms, 35.0);
            }
        }
        scroll = scroll.clamp(0.0, max);
        self.scroll.set(scroll);

        // The highlight glides within a list and jumps when the list changes.
        let mut hl = self.highlight.get();
        if let Some(y) = cursor_y {
            if fresh || !hl.is_finite() {
                hl = y;
            }
            approach(&mut hl, y, c.dt_ms, 35.0);
            self.highlight.set(hl);
        }
        if max > 0.0 {
            // Where the rows shown are in the whole list.
            let bar_h = view - 4.0;
            let (from, len) = (scroll / height * bar_h, (view / height * bar_h).max(12.0));
            c.round_rect(
                [cx + col + 4.0, list_top],
                [3.0, bar_h],
                1.5,
                rgba(0x2a2a38, 1.0),
                rgba(0, 0.0),
                0.0,
            );
            c.round_rect(
                [cx + col + 4.0, list_top + from],
                [3.0, len],
                1.5,
                rgba(accent(), 0.9),
                rgba(0, 0.0),
                0.0,
            );
        }
        if searching && lines.is_empty() {
            c.text(
                cx + 12.0,
                list_top + 10.0,
                8.0,
                rgba(0x8a8aa0, 1.0),
                tr!("Nothing found."),
            );
        }
        let track_w = (col * 0.2).clamp(50.0, 110.0);
        self.track.set((
            cx + col - 12.0 - Canvas::text_width(8.0, "100%") - 12.0 - track_w,
            track_w,
        ));
        let in_list = c.hovered(cx, list_top - 2.0, col, list_h + 2.0);
        let mut hovered = None;
        let (start, hits) = (c.shapes.len(), c.hits.len());
        for line in &lines {
            let y = list_top - scroll
                + match line {
                    Line::Group(_, y) | Line::Row(_, y) => *y,
                };
            if y + ROW < list_top - 2.0 || y > list_top + view {
                continue;
            }
            match line {
                Line::Group(heading, _) => {
                    c.text(
                        cx + 2.0,
                        y + 6.0,
                        7.0,
                        rgba(crate::style::selector_colour(), 0.85),
                        heading,
                    );
                }
                Line::Row(i, _) => {
                    let row_hover = in_list && c.hovered(cx, y, col, ROW - 4.0);
                    if row_hover {
                        hovered = Some(ITEMS[*i].1);
                    }
                    self.draw_row(c, *i, [cx, y, col], row_hover, value, control);
                }
            }
        }
        if cursor_y.is_some() {
            crate::style::brackets(
                c,
                [cx - 3.0, list_top + hl - scroll - 3.0, col + 6.0, ROW + 2.0],
                1.0,
            );
        }
        c.clip_since(start, cx - 2.0, list_top - 2.0, col + 4.0, view + 2.0);
        for h in &mut c.hits[hits..] {
            let [_, y, _, hh] = &mut h.rect;
            let (y0, y1) = (y.max(list_top - 2.0), (*y + *hh).min(list_top + view));
            (*y, *hh) = (y0, (y1 - y0).max(0.0));
        }

        // What the setting under the pointer does, or else the selected one: two lines, spaced
        // for however tall the text comes out (taller on a text grid), under a divider.
        c.rect(cx, help_y - 3.0, col, 1.0, rgba(0x2a2a38, 1.0));
        let help = hovered
            .or(cursor_y.map(|_| self.item()))
            .map_or("", Item::help);
        let (line1, line2) = split_help(help, 7.0, col - 20.0);
        // `text` centres taller glyphs on the line; this puts their top 6 below the box's.
        let first = help_y + 6.0 + (glyph_h - 7.0) / 2.0;
        c.text(cx + 10.0, first, 7.0, rgba(0x9a9ab0, 1.0), &line1);
        c.text(cx + 10.0, first + line_h, 7.0, rgba(0x9a9ab0, 1.0), &line2);
        c.text_centered(
            w / 2.0,
            462.0,
            8.0,
            rgba(0x55556a, 1.0),
            &Canvas::fit_hints(8.0, w - 48.0, footer),
        );
    }

    /// Row `i` of `ITEMS` at `[x, y, width]`: its name, and its value in the way that suits it.
    fn draw_row(
        &self,
        c: &mut Canvas,
        i: usize,
        [cx, y, col]: [f32; 3],
        hover: bool,
        value: &dyn Fn(Item) -> String,
        control: &dyn Fn(Item) -> Control,
    ) {
        let item = ITEMS[i].1;
        let selected = i == self.cursor;
        // Rows are lines on the panel: the selected one lit in the selector's colour, a divider under each.
        if selected {
            c.round_rect(
                [cx, y],
                [col, ROW - 4.0],
                6.0,
                rgba(
                    crate::style::selector_colour(),
                    0.10 + 0.05 * crate::style::pulse(),
                ),
                rgba(0, 0.0),
                0.0,
            );
        } else if hover {
            c.round_rect(
                [cx, y],
                [col, ROW - 4.0],
                6.0,
                rgba(0x262634, 0.6),
                rgba(0, 0.0),
                0.0,
            );
        }
        c.rect(
            cx + 8.0,
            y + ROW - 3.0,
            col - 16.0,
            1.0,
            rgba(0x23232f, 1.0),
        );
        c.hit(cx, y - 2.0, col, ROW, Hit::SettingRow(i));
        // Found by a search, the row says which section it's in.
        let mut x = cx + 12.0;
        if !self.search.is_empty() {
            let s = SECTIONS
                .iter()
                .position(|(name, _)| *name == ITEMS[i].0)
                .unwrap_or(0);
            x += c.text(
                x,
                y + 10.5,
                6.0,
                rgba(0x8a8aa0, 1.0),
                &format!("{}  ", section_name(s)),
            );
        }
        c.text(
            x,
            y + 9.5,
            8.0,
            rgba(if selected { 0xffffff } else { 0xc8c8d8 }, 1.0),
            item.label(),
        );
        let right = cx + col - 12.0;
        // The value gets what the label leaves, and is cut short past that.
        let room = right - x - 12.0 - Canvas::text_width(8.0, item.label());
        if let Some(edit) = self.editing.as_ref().filter(|e| selected && e.item == item) {
            // A text box in place of the value; the old value shows selected until typed over.
            let text = Canvas::fit_end(8.0, room - 24.0, &edit.text);
            let tw = Canvas::text_width(8.0, &text);
            let bw = (tw + 24.0).max(90.0).min(room);
            c.round_rect(
                [right + 6.0 - bw, y + 4.0],
                [bw, ROW - 12.0],
                4.0,
                rgba(0x15151d, 1.0),
                rgba(0xFFD34D, 1.0),
                1.0,
            );
            if edit.fresh && !text.is_empty() {
                c.rect(
                    right - 7.0 - tw,
                    y + 7.0,
                    tw + 2.0,
                    ROW - 18.0,
                    rgba(accent(), 0.45),
                );
            }
            c.text_right(right - 6.0, y + 9.5, 8.0, rgba(0xFFD34D, 1.0), &text);
            // At the caret in the text (at the end when the start was cut off to fit).
            let at = if text == edit.text {
                caret_x(8.0, &text, edit.caret)
            } else {
                tw
            };
            c.rect(
                right - 6.0 - tw + at,
                y + 7.0,
                1.0,
                ROW - 18.0,
                rgba(0xFFD34D, 1.0),
            );
            return;
        }
        if selected && self.binding.is_some() {
            c.text_right(
                right,
                y + 9.5,
                8.0,
                rgba(0xFFD34D, 1.0),
                tr!("press a key..."),
            );
            return;
        }
        let shown = value(item);
        match (item.kind(), control(item)) {
            (_, Control::Switch(on)) => {
                let (sw, sh) = (22.0, 11.0);
                let (sx, sy) = (right - sw, y + (ROW - 4.0 - sh) / 2.0);
                let fill = if on { accent() } else { 0x3a3a4e };
                c.round_rect(
                    [sx, sy],
                    [sw, sh],
                    sh / 2.0,
                    rgba(fill, 1.0),
                    rgba(fill, 1.0),
                    0.0,
                );
                let knob = if on { sx + sw - sh + 1.5 } else { sx + 1.5 };
                c.round_rect(
                    [knob, sy + 1.5],
                    [sh - 3.0, sh - 3.0],
                    (sh - 3.0) / 2.0,
                    rgba(0xffffff, 1.0),
                    rgba(0xffffff, 1.0),
                    0.0,
                );
                // The rare switch with more to say than on or off.
                if shown.chars().count() > 4 {
                    c.text_right(
                        sx - 8.0,
                        y + 9.5,
                        8.0,
                        rgba(0xc8c8d8, 1.0),
                        &Canvas::fit(8.0, room - sw - 8.0, &shown),
                    );
                }
            }
            (_, Control::Level(level)) => {
                let (tx, tw) = self.track.get();
                let ty = y + (ROW - 4.0) / 2.0 - 2.0;
                c.round_rect(
                    [tx, ty],
                    [tw, 4.0],
                    2.0,
                    rgba(0x3a3a4e, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                c.round_rect(
                    [tx, ty],
                    [(tw * level).max(4.0), 4.0],
                    2.0,
                    rgba(accent(), 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                c.round_rect(
                    [tx + tw * level - 4.0, ty - 3.0],
                    [8.0, 10.0],
                    3.0,
                    rgba(0xffffff, 1.0),
                    rgba(0, 0.0),
                    0.0,
                );
                c.hit(tx - 5.0, y, tw + 10.0, ROW - 4.0, Hit::SettingLevel(i));
                c.text_right(right, y + 9.5, 8.0, rgba(0xffffff, 1.0), &shown);
            }
            (Kind::Key, _) => {
                let text = Canvas::fit(8.0, room - 12.0, &shown);
                let bw = Canvas::text_width(8.0, &text) + 12.0;
                c.round_rect(
                    [right + 6.0 - bw, y + 5.0],
                    [bw, ROW - 14.0],
                    3.0,
                    rgba(0x2a2a3a, 1.0),
                    rgba(0x5a5a74, 1.0),
                    1.0,
                );
                c.text_right(right, y + 9.5, 8.0, rgba(0xffffff, 1.0), &text);
            }
            (Kind::Action, _) => {
                let text = Canvas::fit(8.0, room, &shown);
                c.text_right(
                    right,
                    y + 9.5,
                    8.0,
                    rgba(
                        if hover || selected {
                            0xffffff
                        } else {
                            0x9a9ab0
                        },
                        1.0,
                    ),
                    &text,
                );
            }
            (_, control) => {
                let stepped = selected || hover;
                let swatch = if matches!(control, Control::Swatch(_)) {
                    14.0
                } else {
                    0.0
                };
                let arrows = Canvas::text_width(8.0, "<    >");
                let text = Canvas::fit(8.0, room - arrows - swatch, &shown);
                let v = if stepped {
                    format!("<  {text}  >")
                } else {
                    text.clone()
                };
                c.text_right(right, y + 9.5, 8.0, rgba(0xffffff, 1.0), &v);
                let tw = Canvas::text_width(8.0, &v);
                if let Control::Swatch(colour) = control {
                    c.round_rect(
                        [right - tw - 15.0, y + 8.0],
                        [10.0, 10.0],
                        2.0,
                        rgba(colour, 1.0),
                        rgba(0xffffff, 0.6),
                        1.0,
                    );
                }
                if stepped {
                    // The "<" and ">" around the value step it.
                    c.hit(
                        right - tw - 6.0,
                        y,
                        20.0,
                        ROW - 4.0,
                        Hit::SettingStep(i, -1),
                    );
                    c.hit(right - 14.0, y, 24.0, ROW - 4.0, Hit::SettingStep(i, 1));
                }
            }
        }
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
        .next_back()
        .filter(|&i| i > 0)
        .unwrap_or(full);
    (
        text[..cut].to_string(),
        Canvas::fit(size, max_w, text[cut..].trim_start()),
    )
}

/// The logo and the name, top left; returns where they end.
pub fn brand(c: &mut Canvas) -> f32 {
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
    tx
}

/// The way back, as a button: what Esc does here.
pub fn back_button(c: &mut Canvas, y: f32, label: &str, hit: Hit) -> f32 {
    if crate::mobile::active() {
        c.button(0.0, 0.0, 96.0, 46.0, label, 12.0, hit, false);
        return 72.0;
    }
    let w = Canvas::text_width(8.0, label) + 20.0;
    c.button(24.0, y, w, 20.0, label, 8.0, hit, false);
    w
}

/// Top of a screen: the way back, what the screen is, and `right_text` to the side.
fn header(c: &mut Canvas, active: usize, right_text: &str) {
    let w = back_button(
        c,
        21.0,
        tr!("< BACK"),
        Hit::Key(winit::keyboard::KeyCode::Escape),
    );
    let title = [
        tr!("SONGS"),
        tr!("SETTINGS"),
        tr!("MULTIPLAYER"),
        tr!("DOWNLOAD"),
    ][active.min(3)];
    let x = 24.0 + w + 12.0;
    let phone = crate::mobile::active();
    let (title_y, title_size) = if phone { (14.0, 18.0) } else { (24.0, 13.0) };
    let used = crate::style::title(c, x, title_y, title_size, accent(), title);
    let right = c.width - crate::mobile::pick(24.0, 0.0);
    let room = right - (x + used + 16.0);
    if Canvas::text_width(8.0, right_text) <= room {
        let y = crate::mobile::pick(28.0, 16.0);
        c.text_right(right, y, 8.0, rgba(0x9a9ab0, 1.0), right_text);
    } else {
        c.text(
            x,
            45.0,
            6.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(6.0, c.width - 110.0 - x, right_text),
        );
    }
}

/// Top bar shared by the song list, settings and multiplayer: logo plus the three tabs.
pub fn tab_bar(c: &mut Canvas, active: usize, right_text: &str) {
    if !crate::render::legacy_menu() {
        return header(c, active, right_text);
    }
    thread_local! {
        /// Where the active tab's highlight is (x, width), so it slides between tabs.
        static PILL: Cell<(f32, f32)> = const { Cell::new((f32::NAN, 0.0)) };
    }
    let x0 = brand(c) + 24.0;
    let names = [
        tr!("SONGS"),
        tr!("SETTINGS"),
        tr!("MULTIPLAYER"),
        tr!("DOWNLOAD"),
    ];
    let mut x = x0;
    let mut target = (x0, 0.0);
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        let on = i == active;
        let hover = !on && c.hovered(x, 20.0, tw + 20.0, 22.0);
        let fill = if hover { 0x34344a } else { 0x23232f };
        c.round_rect(
            [x, 20.0],
            [tw + 20.0, 22.0],
            6.0,
            rgba(fill, 1.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
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
    c.round_rect(
        [px, 20.0],
        [pw, 22.0],
        6.0,
        rgba(accent(), 1.0),
        rgba(accent(), 1.0),
        1.0,
    );
    let mut x = x0;
    for (i, name) in names.iter().enumerate() {
        let tw = Canvas::text_width(9.0, name);
        c.text(
            x + 10.0,
            27.0,
            9.0,
            rgba(if i == active { 0xffffff } else { 0x9a9ab0 }, 1.0),
            name,
        );
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
        c.text(
            x0 + 2.0,
            45.0,
            6.0,
            rgba(0x9a9ab0, 1.0),
            &Canvas::fit(6.0, c.width - 110.0 - x0, right_text),
        );
    }
}
