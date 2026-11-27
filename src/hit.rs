//! What can be clicked: every clickable area the app draws is one of these. The renderer
//! only keeps them (`render::Canvas<H>`, `render::HitBox<H>`); what a click on one does is
//! the app's (`State::click` and the screens).

/// The canvas the app draws on: its clickable areas are `Hit`s.
pub type Canvas = crate::render::Canvas<Hit>;
/// A clickable area of the app.
pub type HitBox = crate::render::HitBox<Hit>;

/// Something clickable drawn this frame, for mouse navigation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// A button that does what this key does.
    Key(winit::keyboard::KeyCode),
    /// The interface editor: its panel (which only stops clicks), a part's row and its show /
    /// hide box (indices into `hud::Part::PLACED`), a preset and a button.
    HudPanel,
    HudRow(usize),
    HudToggle(usize),
    HudPreset(usize),
    HudButton(crate::hud_edit::Button),
    /// A colour for the selected part's text (None: its own).
    HudColour(Option<u32>),
    /// Top bar tab: 0 songs, 1 settings, 2 multiplayer.
    Tab(usize),
    /// A note of the main menu (index into `menu::ITEMS`).
    MenuItem(usize),
    /// A tab of the medals screen.
    MedalTab(usize),
    /// An unknown medal's question mark on the medals screen (index into its medals): tapped
    /// enough, it hatches (`medals::View::poke`).
    MedalMystery(usize),
    /// The main menu's logo, and the stray note crossing it (`eggs`).
    MenuLogo,
    StrayNote,
    /// Takes the song list back to the selection it was scrolled away from.
    ListBack,
    /// Song select's way out: the main menu, or back where it came from.
    ToMenu,
    /// A button of the first steps guide.
    Guide(crate::guide::Action),
    /// A button or field of the editor's front door.
    EditorHome(crate::editor::home::Act),
    /// The main menu's button to our Discord server.
    DiscordServer,
    /// A music button of the main menu or the player, and the player's timeline.
    Music(crate::player::Control),
    MusicBar,
    /// The player: its search box, a song (index into the list's tracks), its `+`, a row of the
    /// queue, the volume, Daycore, the speed (back to 1x) and keeping the pitch.
    PlayerSearch,
    PlayerRow(usize),
    /// Its heart on a song, the folders button, and a folder (index into `Playlist::folders`).
    PlayerFavourite(usize),
    /// A screenshot in the chat (`shots::shot_in`), and the button that sends one.
    ChatShot(u128),
    SendShot,
    /// A run of medal lines in the chat: opens or closes it (`Social::toggle_medal_run`).
    ChatMedals(u64),
    /// Song select's heart on the song it's showing.
    SelectLike,
    /// Song select's full-combo pp: opens the pp simulator in its place, or closes it; a step of
    /// one of its fields (0: accuracy, 1: misses) down or up.
    PpSim,
    PpSimStep(u8, i32),
    /// A whole field of the pp simulator (0: accuracy, 1: misses), for the wheel to step it.
    PpSimField(u8),
    /// The song preview's autoplay box, seek graph and play button. It only marks the preview as
    /// the topmost thing there, so whatever is drawn over it takes the click instead.
    Preview,
    /// A phone's speed panel: opens it, closes it, moves the speed by hundredths or sets it, flips
    /// keeping the pitch.
    RateOpen,
    RateClose,
    RateStep(i32),
    RateSet(i32),
    RateKeepPitch,
    /// The upright player: the list comes up or goes.
    PlayerList,
    /// A phone's song select: slides between the preview and the leaderboard.
    BoardPage,
    PlayerFolders,
    PlayerFolder(usize),
    PlayerQueue(usize),
    PlayerUnqueue(usize),
    PlayerVolume,
    PlayerDaycore,
    PlayerRate,
    PlayerKeepPitch,
    /// A row of the song list (index into its visible list): a difficulty, or a song's own row
    /// (pointing at its first difficulty).
    SongRow(usize),
    SongHeader(usize),
    /// A heading of the song list (index into its sections): opens or closes it.
    SectionHeader(usize),
    /// An item of the song list's right-click menu.
    SongMenu(usize),
    /// The confirmation's Delete button: the difficulty or set asked for goes.
    DeleteMaps,
    /// The offer's Move button: taiko.rs's maps move out of osu!'s Songs folder.
    MoveCustomMaps,
    /// A song's "update available" (library index of one of its difficulties).
    UpdateSet(usize),
    /// A mod in the mod select (index into `mods::ALL`) / the song select's Mods button.
    ModToggle(usize),
    /// A mod preset in the mod select (index into `Config::mod_presets`): applies it.
    ModPreset(usize),
    /// The mod select's "+ Save": keeps the mods on now (and the speed) as a preset.
    ModPresetSave,
    ModsButton,
    JourneyButton,
    JourneyLevel(usize),
    JourneyLevelAuto,
    JourneyLevelStep(i32),
    JourneyLevelOpen,
    JourneyNext,
    JourneySave,
    /// Journey's offer to a signed-out player: log in (false) or make an account (true).
    JourneyLogin(bool),
    /// Journey goes on offline, with the library only.
    JourneyOffline,
    /// Journey asks the server again now.
    JourneyRetry,
    /// Journey's gear: opens or closes its settings panel.
    JourneySettings,
    /// Empty area inside the Journey settings modal (captures clicks so they don't reach behind).
    JourneySettingsBody,
    /// A row of Journey's settings panel stepped back (-1) or on (+1).
    JourneySetting(usize, i32),
    /// Direct choice clicks inside Journey's settings panel.
    JourneyFocus(crate::config::JourneyFocus),
    JourneyLength(crate::config::JourneyLength),
    JourneyRates(crate::config::JourneyRates),
    JourneyFlow(crate::config::JourneyFlow),
    MarathonGap(crate::config::MarathonGap),
    /// Starts a fresh marathon from the marathon summary screen.
    MarathonRestart,
    SettingRow(usize),
    /// The `<` (-1) / `>` (+1) of the selected settings row.
    SettingStep(usize, i32),
    /// The bar of a setting (index into `settings::ITEMS`): set by clicking or dragging it.
    SettingLevel(usize),
    /// The settings' search box.
    SettingsSearch,
    PauseItem(usize),
    SkipIntro,
    MpRow(usize),
    /// Opens the playfield preset list.
    Presets,
    /// A download tab result row / its Download button.
    DlRow(usize),
    DlGet(usize),
    /// Song select: the sort and filter panel (on a phone, the sort list) / open the collection
    /// list / a row of that list.
    SortList,
    /// Something in song select's sort and filter panel.
    Lens(crate::ui::LensItem),
    /// A source tab over song select's list (`ui::Source::ALL` index).
    SongSource(usize),
    /// A filter's token in song select's search box: takes the filter off.
    LensToken(crate::ui::Token),
    CollectionPick,
    PickRow(usize),
    /// The "..." on a collection's row: what can be done with it.
    PickRowMore(usize),
    /// A row of a setting's list of choices.
    ChoiceRow(usize),
    /// Search in the installed font picker.
    ChoiceSearch,
    /// Multiverse setup: a universe's (row) colours, shape, speed or a mod (column).
    UniverseCell(usize, usize),
    /// Shapeshift setup: a choice (row, column) / move the selected shape earlier or later.
    ShapeshiftCell(usize, usize),
    ShapeshiftMove(i32),
    /// Tact setup: a step (index into `tact::STEPS`).
    TactCell(usize),
    /// Mod customization: a tab (index into the customizable mods that are on).
    CustomTab(usize),
    /// Multiverse setup: load a preset / delete one of the player's.
    UniversePreset(usize),
    UniversePresetDelete(usize),
    /// Song select with no maps: choose the Songs folder.
    SongsFolder,
    /// The song list's scroll bar.
    SongScrollbar,
    /// A settings sidebar section.
    SettingsSection(usize),
    /// The update badge: install the new build.
    Update,
    /// Download tab status category / sort chips.
    /// The download tab's source: osu! or taiko.rs (`browse::Source::ALL`).
    DlSource(usize),
    /// The download tab's drop-downs: 0 status, 1 order (`browse::Menu`).
    DlMenu(u8),
    /// Anywhere outside an open drop-down: closes it.
    DlMenuClose,
    /// Flips the download tab's order.
    DlSortDir,
    /// Shows or leaves out the sets already installed (`Config::download_hide_installed`).
    DlHideInstalled,
    /// Plays result `i`, which is installed.
    DlPlay(usize),
    /// Stops download `i` (`App::downloads`).
    DlCancel(usize),
    /// Tries failed download `i` again, or forgets it (`Browse::failed_jobs`).
    DlRetry(usize),
    DlDismiss(usize),
    DlCategory(usize),
    DlSort(usize),
    /// A row of the song select leaderboard (opens its results) / its video button, and a row
    /// of the online leaderboard.
    ScoreRow(usize),
    ScoreVideo(usize),
    OnlineScoreRow(usize),
    /// The What's new chip, and an overlay over everything (takes clicks meant for underneath).
    Changelog,
    /// The account badge: the website profile (or logging in).
    Account,
    /// Song select's divider between the song list and the panel (dragged).
    SplitList,
    Overlay,
    /// The leaderboard's tabs: local (false) or online (true).
    BoardTab(bool),
    /// Song select's right column: the tab with the preview.
    PreviewTab,
    /// Opens the selected map's page in the browser.
    BoardWebsite,
    /// Lobby: give the host role to this player.
    GiveHost(u32),
    /// Lobby host: takes this player (id) out of the lobby.
    Kick(u32),
    /// The friends overlay: its chip, its panel (swallows clicks), a tab, a row, a row's button, and a
    /// notification card.
    SocialToggle,
    SocialPanel,
    /// Back from a conversation to the friend list.
    SocialBack,
    SocialTab(crate::social::Tab),
    SocialRow(usize),
    SocialAction(usize, crate::social::Action),
    SocialNote(usize),
    /// An admin takes this global chat line (its index) down.
    SocialDeleteChat(usize),
    /// A chat line naming a map set (`/np`): downloads it, or picks it in song select.
    ChatMap(u32),
    /// An invite's card in a conversation: join the mapping session (true), or decline it.
    MappingInvite(u64, bool),
    /// The menus' bar of the mapping session left: back into it, or forget it.
    BackToSession,
    ForgetSession,
    /// A text field (a search box, the message box): on Android, tapping it brings up the
    /// on-screen keyboard.
    TextField,
    /// The line to write on under the lobby's chat.
    LobbyChat,
}
