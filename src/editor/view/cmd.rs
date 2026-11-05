//! Everything the editor can be told to do, by name. One table maps keys to commands; the menus,
//! the tooltips and the shortcut sheet all read that same table, so none of them can disagree
//! with what a key really does.

use winit::keyboard::KeyCode;

use super::Keys;
use crate::editor::state::Tool;

/// What the middle of the screen is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Notes: placing, moving, hitsounds.
    Compose,
    /// The stable-compatible structures and generated slider velocity.
    Gimmicks,
    /// Red and green lines and the velocity graph.
    Timing,
    /// The client's own animation of a `.tko`: keyframes and nodes.
    Effects,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Compose, Mode::Gimmicks, Mode::Timing, Mode::Effects];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Compose => "Compose",
            Mode::Gimmicks => "Gimmicks",
            Mode::Timing => "Timing",
            Mode::Effects => "Effects",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Compose => "F1",
            Mode::Gimmicks => "F2",
            Mode::Timing => "F3",
            Mode::Effects => "F4",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Mode::Compose => "Place and edit notes",
            Mode::Gimmicks => "Stable-compatible gimmicks and generated SV",
            Mode::Timing => "Red and green lines, BPM and slider velocity",
            Mode::Effects => "Animate the playfield (.tko only)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cmd {
    // File.
    Save,
    Revert,
    Upload,
    Leave,
    // Edit.
    Undo,
    Redo,
    Cut,
    CopyNotes,
    Paste,
    SelectAll,
    Delete,
    Duplicate,
    Reverse,
    Flip,
    Nudge(bool),
    // Tools and sounds.
    Tool(Tool),
    Whistle,
    Finish,
    Clap,
    // Moving about.
    Play,
    PlayFromStart,
    FirstNote,
    LastNote,
    Step(i32),
    PrevLine,
    NextLine,
    // The view.
    ZoomIn,
    ZoomOut,
    ZoomFit,
    SnapStep(i32),
    SnapTo(u32),
    RateStep(i32),
    RateFine(i32),
    // Bookmarks.
    Bookmark,
    Unbookmark,
    PrevBookmark,
    NextBookmark,
    // Effect marks (.tko).
    EffectMark,
    EffectUnmark,
    // Timing.
    AddRed,
    AddGreen,
    DeleteLine,
    TimingList,
    // The editor itself.
    Mode(Mode),
    Tour,
    Shortcuts,
    // Answers to the "unsaved changes" question.
    LeaveSave,
    LeaveDiscard,
    Stay,
}

impl Cmd {
    pub fn name(self) -> &'static str {
        match self {
            Cmd::Save => "Save",
            Cmd::Revert => "Revert to last save",
            Cmd::Upload => "Upload to taiko.rs",
            Cmd::Leave => "Exit editor",
            Cmd::Undo => "Undo",
            Cmd::Redo => "Redo",
            Cmd::Cut => "Cut",
            Cmd::CopyNotes => "Copy",
            Cmd::Paste => "Paste",
            Cmd::SelectAll => "Select all",
            Cmd::Delete => "Delete",
            Cmd::Duplicate => "Clone selection",
            Cmd::Reverse => "Reverse selection",
            Cmd::Flip => "Flip don / kat",
            Cmd::Nudge(true) => "Nudge later",
            Cmd::Nudge(false) => "Nudge earlier",
            Cmd::Tool(Tool::Select) => "Select tool",
            Cmd::Tool(Tool::Note) => "Note tool",
            Cmd::Tool(Tool::Roll) => "Drumroll tool",
            Cmd::Tool(Tool::Swell) => "Denden tool",
            Cmd::Whistle => "Kat (whistle)",
            Cmd::Finish => "Big (finish)",
            Cmd::Clap => "Kat (clap)",
            Cmd::Play => "Play / pause",
            Cmd::PlayFromStart => "Play from the start",
            Cmd::FirstNote => "Go to the first note",
            Cmd::LastNote => "Go to the last note",
            Cmd::Step(n) if n < 0 => "Step back",
            Cmd::Step(_) => "Step forward",
            Cmd::PrevLine => "Previous timing line",
            Cmd::NextLine => "Next timing line",
            Cmd::ZoomIn => "Zoom in",
            Cmd::ZoomOut => "Zoom out",
            Cmd::ZoomFit => "Zoom to the whole song",
            Cmd::SnapStep(n) if n < 0 => "Coarser snap",
            Cmd::SnapStep(_) => "Finer snap",
            Cmd::SnapTo(_) => "Beat snap",
            Cmd::RateStep(n) if n < 0 => "Slower",
            Cmd::RateStep(_) => "Faster",
            Cmd::RateFine(n) if n < 0 => "Slower (5%)",
            Cmd::RateFine(_) => "Faster (5%)",
            Cmd::Bookmark => "Add bookmark",
            Cmd::Unbookmark => "Remove bookmark",
            Cmd::PrevBookmark => "Previous bookmark",
            Cmd::NextBookmark => "Next bookmark",
            Cmd::EffectMark => "Add effect mark",
            Cmd::EffectUnmark => "Remove effect mark",
            Cmd::AddRed => "Add red line (BPM)",
            Cmd::AddGreen => "Add green line (SV)",
            Cmd::DeleteLine => "Delete timing line",
            Cmd::TimingList => "Timing window (all lines, their fields, the SV tool)",
            Cmd::Mode(m) => m.name(),
            Cmd::Tour => "Take the tour",
            Cmd::Shortcuts => "Keyboard shortcuts",
            Cmd::LeaveSave => "Save and exit",
            Cmd::LeaveDiscard => "Exit without saving",
            Cmd::Stay => "Keep editing",
        }
    }

    /// One line on what it does, for the tooltip.
    pub fn about(self) -> &'static str {
        match self {
            Cmd::Save => "Writes the map. The first save keeps a .bak of the original.",
            Cmd::Revert => "Throws away every change since the last save.",
            Cmd::Upload => "Sends the saved map to taiko.rs: public as CUSTOM, pp once approved.",
            Cmd::Leave => "Back to song select.",
            Cmd::Undo => "Takes back the last change.",
            Cmd::Redo => "Does the undone change again.",
            Cmd::Cut | Cmd::CopyNotes => "Keeps the selected notes to paste elsewhere.",
            Cmd::Paste => "Puts the copied notes at the playhead.",
            Cmd::SelectAll => "Selects every note.",
            Cmd::Delete => "Removes the selected notes (right click removes one).",
            Cmd::Duplicate => "Copies the selection to one measure after its last note.",
            Cmd::Reverse => "Mirrors the selected notes in time.",
            Cmd::Flip => "Turns each selected don into a kat and back.",
            Cmd::Nudge(_) => "Moves the selection one snap step.",
            Cmd::Tool(Tool::Select) => "Click or drag a box to select, drag notes to move them.",
            Cmd::Tool(Tool::Note) => {
                "Click to place a don (or a kat, see W / R). Right click deletes."
            }
            Cmd::Tool(Tool::Roll) => "Drag, or click the start and the end, to make a drumroll.",
            Cmd::Tool(Tool::Swell) => "Drag, or click the start and the end, to make a denden.",
            Cmd::Whistle | Cmd::Clap => {
                "Selected notes become kats; with nothing selected, the next notes you place are kats."
            }
            Cmd::Finish => {
                "Selected notes become big; with nothing selected, the next notes you place are big."
            }
            Cmd::Play => "Plays the song from the playhead.",
            Cmd::PlayFromStart => "Plays the song from the beginning.",
            Cmd::FirstNote | Cmd::LastNote => "Moves the playhead there.",
            Cmd::Step(_) => "Moves the playhead one beat-snap step.",
            Cmd::PrevLine | Cmd::NextLine => "Moves the playhead to the neighbouring timing line.",
            Cmd::ZoomIn | Cmd::ZoomOut | Cmd::ZoomFit => {
                "Changes how much time the timeline shows."
            }
            Cmd::SnapStep(_) | Cmd::SnapTo(_) => {
                "How finely the grid divides a beat. Notes you place land on it."
            }
            Cmd::RateStep(_) | Cmd::RateFine(_) => "Plays the music slower, to place notes by ear.",
            Cmd::Bookmark => "Marks the playhead so you can jump back to it.",
            Cmd::Unbookmark => "Removes the bookmark nearest the playhead.",
            Cmd::PrevBookmark | Cmd::NextBookmark => "Jumps to the neighbouring bookmark.",
            Cmd::EffectMark => {
                "Names this moment for the effects: a node can fire at it, or sweep from one mark to the next. .tko maps only."
            }
            Cmd::EffectUnmark => "Removes the effect mark nearest the playhead.",
            Cmd::AddRed => "A BPM change at the playhead; it also resets the beat grid.",
            Cmd::AddGreen => "A slider velocity change at the playhead.",
            Cmd::DeleteLine => "Removes the timing line at the playhead.",
            Cmd::TimingList => "Every red and green line in a table.",
            Cmd::Mode(m) => m.about(),
            Cmd::Tour => "A short walk through the editor.",
            Cmd::Shortcuts => "Every key and mouse action in one sheet.",
            Cmd::LeaveSave | Cmd::LeaveDiscard | Cmd::Stay => "",
        }
    }

    /// The key label, from the binding table.
    pub fn hotkey(self) -> Option<&'static str> {
        BINDS
            .iter()
            .find(|b| b.cmd == self && !b.label.is_empty())
            .map(|b| b.label)
    }

    /// Name, key and what it does, for a tooltip.
    pub fn tip(self) -> String {
        let key = self
            .hotkey()
            .map(|k| format!("  [{k}]"))
            .unwrap_or_default();
        let about = self.about();
        if about.is_empty() {
            format!("{}{key}", self.name())
        } else {
            format!("{}{key}\n{about}", self.name())
        }
    }
}

/// One key and what it does. An empty `label` is a second key for the same thing (it isn't listed).
pub struct Bind {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Held down, the key repeats.
    pub repeat: bool,
    pub cmd: Cmd,
    pub label: &'static str,
    pub group: &'static str,
}

const fn b(
    code: KeyCode,
    mods: &str,
    repeat: bool,
    cmd: Cmd,
    label: &'static str,
    group: &'static str,
) -> Bind {
    let m = mods.as_bytes();
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut i = 0;
    while i < m.len() {
        match m[i] {
            b'c' => ctrl = true,
            b's' => shift = true,
            b'a' => alt = true,
            _ => {}
        }
        i += 1;
    }
    Bind {
        code,
        ctrl,
        shift,
        alt,
        repeat,
        cmd,
        label,
        group,
    }
}

pub const FILE: &str = "File";
pub const EDIT: &str = "Edit";
pub const TOOLS: &str = "Tools and sounds";
pub const MOVE: &str = "Playback and seeking";
pub const VIEW: &str = "View and snap";
pub const MARKS: &str = "Bookmarks";
pub const FX_MARKS: &str = "Effect marks";
pub const LINES: &str = "Timing";
pub const MODES: &str = "Modes";

/// The keyboard, laid out the way osu!stable's editor has it.
pub const BINDS: &[Bind] = &[
    b(KeyCode::KeyS, "c", false, Cmd::Save, "Ctrl+S", FILE),
    b(KeyCode::KeyL, "c", false, Cmd::Revert, "Ctrl+L", FILE),
    b(KeyCode::Escape, "", false, Cmd::Leave, "Esc", FILE),
    b(KeyCode::KeyZ, "c", true, Cmd::Undo, "Ctrl+Z", EDIT),
    b(KeyCode::KeyY, "c", true, Cmd::Redo, "Ctrl+Y", EDIT),
    b(KeyCode::KeyZ, "cs", true, Cmd::Redo, "", EDIT),
    b(KeyCode::KeyX, "c", false, Cmd::Cut, "Ctrl+X", EDIT),
    b(KeyCode::KeyC, "c", false, Cmd::CopyNotes, "Ctrl+C", EDIT),
    b(KeyCode::KeyV, "c", false, Cmd::Paste, "Ctrl+V", EDIT),
    b(KeyCode::KeyA, "c", false, Cmd::SelectAll, "Ctrl+A", EDIT),
    b(KeyCode::Delete, "", false, Cmd::Delete, "Delete", EDIT),
    b(KeyCode::Backspace, "", false, Cmd::Delete, "", EDIT),
    b(KeyCode::KeyD, "c", false, Cmd::Duplicate, "Ctrl+D", EDIT),
    b(KeyCode::KeyG, "c", false, Cmd::Reverse, "Ctrl+G", EDIT),
    b(KeyCode::KeyQ, "", false, Cmd::Flip, "Q", EDIT),
    b(KeyCode::KeyJ, "", true, Cmd::Nudge(false), "J", EDIT),
    b(KeyCode::KeyK, "", true, Cmd::Nudge(true), "K", EDIT),
    b(
        KeyCode::Digit1,
        "",
        false,
        Cmd::Tool(Tool::Select),
        "1",
        TOOLS,
    ),
    b(
        KeyCode::Digit2,
        "",
        false,
        Cmd::Tool(Tool::Note),
        "2",
        TOOLS,
    ),
    b(
        KeyCode::Digit3,
        "",
        false,
        Cmd::Tool(Tool::Roll),
        "3",
        TOOLS,
    ),
    b(
        KeyCode::Digit4,
        "",
        false,
        Cmd::Tool(Tool::Swell),
        "4",
        TOOLS,
    ),
    b(KeyCode::KeyW, "", false, Cmd::Whistle, "W", TOOLS),
    b(KeyCode::KeyE, "", false, Cmd::Finish, "E", TOOLS),
    b(KeyCode::KeyR, "", false, Cmd::Clap, "R", TOOLS),
    b(KeyCode::Space, "", false, Cmd::Play, "Space", MOVE),
    b(KeyCode::KeyC, "", false, Cmd::Play, "C", MOVE),
    b(KeyCode::KeyX, "", false, Cmd::PlayFromStart, "X", MOVE),
    b(KeyCode::KeyZ, "", false, Cmd::FirstNote, "Z", MOVE),
    b(KeyCode::KeyV, "", false, Cmd::LastNote, "V", MOVE),
    b(KeyCode::ArrowLeft, "", true, Cmd::Step(-1), "Left", MOVE),
    b(KeyCode::ArrowRight, "", true, Cmd::Step(1), "Right", MOVE),
    b(
        KeyCode::ArrowLeft,
        "s",
        true,
        Cmd::Step(-4),
        "Shift+Left",
        MOVE,
    ),
    b(
        KeyCode::ArrowRight,
        "s",
        true,
        Cmd::Step(4),
        "Shift+Right",
        MOVE,
    ),
    b(KeyCode::ArrowUp, "", true, Cmd::PrevLine, "Up", MOVE),
    b(KeyCode::ArrowDown, "", true, Cmd::NextLine, "Down", MOVE),
    b(
        KeyCode::ArrowUp,
        "c",
        false,
        Cmd::RateStep(1),
        "Ctrl+Up",
        MOVE,
    ),
    b(
        KeyCode::ArrowDown,
        "c",
        false,
        Cmd::RateStep(-1),
        "Ctrl+Down",
        MOVE,
    ),
    b(
        KeyCode::ArrowUp,
        "cs",
        false,
        Cmd::RateFine(1),
        "Ctrl+Shift+Up",
        MOVE,
    ),
    b(
        KeyCode::ArrowDown,
        "cs",
        false,
        Cmd::RateFine(-1),
        "Ctrl+Shift+Down",
        MOVE,
    ),
    b(KeyCode::Equal, "", true, Cmd::ZoomIn, "=", VIEW),
    b(KeyCode::NumpadAdd, "", true, Cmd::ZoomIn, "", VIEW),
    b(KeyCode::Minus, "", true, Cmd::ZoomOut, "-", VIEW),
    b(KeyCode::NumpadSubtract, "", true, Cmd::ZoomOut, "", VIEW),
    b(
        KeyCode::Digit1,
        "s",
        false,
        Cmd::SnapTo(1),
        "Shift+1..9",
        VIEW,
    ),
    b(KeyCode::Digit2, "s", false, Cmd::SnapTo(2), "", VIEW),
    b(KeyCode::Digit3, "s", false, Cmd::SnapTo(3), "", VIEW),
    b(KeyCode::Digit4, "s", false, Cmd::SnapTo(4), "", VIEW),
    b(KeyCode::Digit5, "s", false, Cmd::SnapTo(5), "", VIEW),
    b(KeyCode::Digit6, "s", false, Cmd::SnapTo(6), "", VIEW),
    b(KeyCode::Digit7, "s", false, Cmd::SnapTo(7), "", VIEW),
    b(KeyCode::Digit8, "s", false, Cmd::SnapTo(8), "", VIEW),
    b(KeyCode::Digit9, "s", false, Cmd::SnapTo(9), "", VIEW),
    b(KeyCode::KeyB, "c", false, Cmd::Bookmark, "Ctrl+B", MARKS),
    b(
        KeyCode::KeyB,
        "cs",
        false,
        Cmd::Unbookmark,
        "Ctrl+Shift+B",
        MARKS,
    ),
    b(
        KeyCode::ArrowLeft,
        "c",
        true,
        Cmd::PrevBookmark,
        "Ctrl+Left",
        MARKS,
    ),
    b(
        KeyCode::ArrowRight,
        "c",
        true,
        Cmd::NextBookmark,
        "Ctrl+Right",
        MARKS,
    ),
    b(KeyCode::KeyM, "", false, Cmd::EffectMark, "M", FX_MARKS),
    b(
        KeyCode::KeyM,
        "s",
        false,
        Cmd::EffectUnmark,
        "Shift+M",
        FX_MARKS,
    ),
    b(KeyCode::KeyP, "c", false, Cmd::AddRed, "Ctrl+P", LINES),
    b(
        KeyCode::KeyP,
        "cs",
        false,
        Cmd::AddGreen,
        "Ctrl+Shift+P",
        LINES,
    ),
    b(KeyCode::KeyI, "c", false, Cmd::DeleteLine, "Ctrl+I", LINES),
    b(KeyCode::F6, "", false, Cmd::TimingList, "F6", LINES),
    b(
        KeyCode::F1,
        "",
        false,
        Cmd::Mode(Mode::Compose),
        "F1",
        MODES,
    ),
    b(
        KeyCode::F2,
        "",
        false,
        Cmd::Mode(Mode::Gimmicks),
        "F2",
        MODES,
    ),
    b(KeyCode::F3, "", false, Cmd::Mode(Mode::Timing), "F3", MODES),
    b(
        KeyCode::F4,
        "",
        false,
        Cmd::Mode(Mode::Effects),
        "F4",
        MODES,
    ),
    b(KeyCode::Slash, "s", false, Cmd::Shortcuts, "?", MODES),
];

/// The mouse, for the shortcut sheet.
pub const MOUSE: &[(&str, &str)] = &[
    (
        "Click",
        "Place a note with the Note tool; select with the Select tool",
    ),
    ("Right click", "Delete the note under the pointer"),
    (
        "Drag a note",
        "Move it (the grid snaps it; hold Shift to ignore the grid)",
    ),
    ("Drag a roll's end", "Stretch it"),
    ("Drag, empty space", "Select a box of notes (Select tool)"),
    ("Double click a note", "Put the playhead on it"),
    ("Click the ruler", "Move the playhead; drag to scrub"),
    (
        "Wheel",
        "Move the playhead along the beat snap (Shift: 4 steps)",
    ),
    ("Ctrl+wheel", "Change the beat snap"),
    ("Alt+wheel", "Zoom the timeline"),
    ("Ctrl+Alt+wheel", "Change tool"),
    ("Middle drag", "Pan the timeline"),
];

/// The command a key means, if any.
pub fn find(code: KeyCode, keys: Keys, repeat: bool) -> Option<Cmd> {
    BINDS
        .iter()
        .find(|b| {
            b.code == code
                && b.ctrl == keys.ctrl
                && b.shift == keys.shift
                && b.alt == keys.alt
                && (b.repeat || !repeat)
        })
        .map(|b| b.cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_is_bound_twice() {
        for (i, a) in BINDS.iter().enumerate() {
            for z in &BINDS[i + 1..] {
                assert!(
                    !(a.code == z.code && a.ctrl == z.ctrl && a.shift == z.shift && a.alt == z.alt),
                    "{:?} is bound twice ({:?}, {:?})",
                    a.code,
                    a.cmd,
                    z.cmd
                );
            }
        }
    }

    #[test]
    fn the_table_is_what_a_key_does_and_what_the_label_says() {
        let none = Keys::default();
        let ctrl = Keys { ctrl: true, ..none };
        assert_eq!(find(KeyCode::KeyZ, none, false), Some(Cmd::FirstNote));
        assert_eq!(find(KeyCode::KeyZ, ctrl, false), Some(Cmd::Undo));
        assert_eq!(find(KeyCode::Escape, none, false), Some(Cmd::Leave));
        // A key that doesn't repeat is ignored when held.
        assert_eq!(find(KeyCode::KeyS, ctrl, true), None);
        assert_eq!(Cmd::Save.hotkey(), Some("Ctrl+S"));
        assert_eq!(Cmd::Play.hotkey(), Some("Space"));
        assert!(Cmd::Undo.tip().contains("Ctrl+Z"));
    }

    #[test]
    fn every_mode_has_its_function_key() {
        for m in Mode::ALL {
            assert_eq!(Cmd::Mode(m).hotkey(), Some(m.key()));
        }
    }
}
