//! The editor's front door: what the main menu's EDITOR opens. Make a map from a song (pick or drop
//! it, name it, time it with a waveform, a metronome and tap tempo), or go on with one you had.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use winit::keyboard::KeyCode;

use super::new_map::{self, NewMap};
use super::tempo;
#[cfg(target_os = "android")]
use crate::android::PickedFile;
use crate::audio::{Engine, Track};
use crate::beatmap::OSU_TIMING_MS;
use crate::hit::{Canvas, Hit};
use crate::library::FileKind;
use crate::render::{VIRTUAL_HEIGHT, accent, rgba};
use crate::settings::{caret_backspace, caret_insert, caret_nav};

/// The editor's own colour, as on the main menu's note.
const VIOLET: u32 = 0x9b6cff;
const PANEL: u32 = 0x1a1a24;
const EDGE: u32 = 0x3a3a4e;
const DIM: u32 = 0x8a8aa0;
const TEXT: u32 = 0xe8e8f0;
const WARN: u32 = 0xFFD34D;
const BAD: u32 = 0xff7a7a;
/// The longest stretch the metronome is baked for at a time (ms).
const BAKE_MS: f64 = 45_000.0;

/// The tag on a recent map: what kind of file it is, and its colour.
fn file_badge(path: &Path) -> (&'static str, u32) {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("tko") => ("TKO", VIOLET),
        Some(e) if e.eq_ignore_ascii_case("osu") => ("OSU", 0xff8fb8),
        _ => ("MAP", DIM),
    }
}

/// A map's name cut to `w`, keeping its difficulty (the `[...]` at the end) whole, since that
/// tells the maps of one song apart; the artist and title are cut instead.
fn fit_keeping_diff(size: f32, w: f32, name: &str) -> String {
    if Canvas::text_width(size, name) <= w {
        return name.to_string();
    }
    match name.rfind(" [") {
        Some(at) if name.ends_with(']') => {
            let diff = &name[at..];
            let room = w - Canvas::text_width(size, diff);
            if room < Canvas::text_width(size, "abcd...") {
                return Canvas::fit(size, w, name);
            }
            format!("{}{diff}", Canvas::fit(size, room, &name[..at]))
        }
        _ => Canvas::fit(size, w, name),
    }
}
/// Bars the loop repeats.
const LOOP_BARS: u32 = 8;
/// A pause (ms) after which the taps start over.
const TAP_RESET_MS: f64 = 2200.0;
/// The most characters a field takes.
const FIELD_MAX: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    /// Make a map or go on with one.
    Start,
    /// Its names.
    Details,
    /// Its tempo and first beat.
    Timing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Artist,
    Title,
    ArtistRomanised,
    TitleRomanised,
    Creator,
    Version,
    Source,
    Tags,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Artist,
        Field::Title,
        Field::ArtistRomanised,
        Field::TitleRomanised,
        Field::Creator,
        Field::Version,
        Field::Source,
        Field::Tags,
    ];

    fn index(self) -> usize {
        Field::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Field::Artist => tr!("Artist"),
            Field::Title => tr!("Title"),
            Field::ArtistRomanised => tr!("Artist in Latin letters (if it isn't)"),
            Field::TitleRomanised => tr!("Title in Latin letters (if it isn't)"),
            Field::Creator => tr!("Mapper (you)"),
            Field::Version => tr!("Difficulty name"),
            Field::Source => tr!("Source (a game, a series)"),
            Field::Tags => tr!("Tags (separated by spaces)"),
        }
    }
}

/// A number on the timing page that can be typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Num {
    Bpm,
    Offset,
}

/// A number being typed: the text so far, and whether the first key replaces all of it (the
/// field was just clicked, so the old value is as good as selected).
#[derive(Clone, Debug)]
struct NumEdit {
    which: Num,
    text: String,
    caret: usize,
    fresh: bool,
}

/// A typed BPM: digits with `.` or `,` for the decimals, held to the range `set_bpm` keeps.
fn parse_bpm(text: &str) -> Option<f64> {
    let v: f64 = text.trim().replace(',', ".").parse().ok()?;
    v.is_finite()
        .then(|| ((v * 1000.0).round() / 1000.0).clamp(20.0, 600.0))
}

/// A typed first beat (ms, may be negative), rounded to a whole ms.
fn parse_offset(text: &str) -> Option<f64> {
    let v: f64 = text.trim().replace(',', ".").parse().ok()?;
    (v.is_finite() && v.abs() < 1e7).then(|| v.round())
}

/// The beats of the grid inside `start..=end` (ms): each with its number, counted from the first
/// beat at `first`, so beats before it (and before the song) are drawn too.
fn beat_times(start: f64, end: f64, first: f64, period: f64) -> Vec<(f64, i64)> {
    let k0 = ((start - first) / period).ceil() as i64;
    (k0..k0 + 4000)
        .map(|k| (first + k as f64 * period, k))
        .take_while(|(t, _)| *t <= end)
        .collect()
}

/// What a click can mean here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Back,
    Next,
    Browse,
    PickInLibrary,
    Recent(usize),
    Tour,
    Focus(Field),
    PickBackground,
    ClearBackground,
    Format(bool),
    /// The waveform: a click puts the playhead there, a drag moves the view.
    Wave,
    /// The whole song under the waveform: the playhead goes where it is pressed and dragged.
    Overview,
    Play,
    Metronome,
    Loop,
    /// BPM changes by this many thousandths.
    Bpm(i16),
    /// The first beat moves by this many ms.
    Offset(i16),
    /// The first beat moves by this many beats.
    Shift(i8),
    Half,
    Double,
    Meter(u8),
    /// The BPM or the first beat's field: type a value there.
    EditNum(Num),
    Tap,
    Detect,
    BeatHere,
    Create,
}

/// What the screen asks of the game.
#[derive(Debug)]
pub enum Out {
    None,
    /// Back to the main menu.
    Leave,
    /// Song select, to pick a map to edit.
    PickInLibrary,
    Open(PathBuf),
    Tour,
    Create(Box<NewMap>),
    Toast(String),
}

/// A song, decoded and looked at.
struct Decoded {
    track: Arc<Track>,
    wave: super::wave::Wave,
    analysis: Option<tempo::Analysis>,
    rate: f64,
}

enum Loading {
    Nothing,
    Working(Receiver<Result<Decoded, String>>),
    Ready(Box<Decoded>),
    Failed(String),
}

/// A file dialog on its way.
struct Dialog {
    kind: FileKind,
    rx: Receiver<Result<Option<PickedFile>, String>>,
}

#[cfg(not(target_os = "android"))]
type PickedFile = PathBuf;

/// A drag that started on the timing page.
#[derive(Clone, Copy, Debug)]
enum Drag {
    /// On the waveform: from where (x), and the view's start then. A press that didn't move is
    /// a click.
    Wave {
        from_x: f32,
        start: f64,
        moved: bool,
    },
    /// On the overview: the music, if it plays, carries on from where it is let go.
    Overview,
}

struct Playing {
    /// Where it started (ms of the song), and when.
    from: f64,
    at: Instant,
    /// Up to where the metronome was baked, when it is on.
    baked_to: f64,
}

pub struct Home {
    pub page: Page,
    recent: Vec<PathBuf>,
    audio: Option<PathBuf>,
    texts: [String; 8],
    carets: [usize; 8],
    focus: Option<Field>,
    /// The BPM or first beat being typed on the timing page.
    num_edit: Option<NumEdit>,
    background: Option<PathBuf>,
    #[cfg(target_os = "android")]
    picked_song: Option<PickedFile>,
    #[cfg(target_os = "android")]
    picked_background: Option<PickedFile>,
    tko: bool,
    loading: Loading,
    dialog: Option<Dialog>,
    status: String,
    // Timing.
    bpm: f64,
    /// The first red line as the chart will have it (ms).
    offset: f64,
    per_bar: u32,
    analysis: Option<tempo::Analysis>,
    taps: Vec<f64>,
    tap_at: Option<Instant>,
    tap_origin: Instant,
    metronome: bool,
    looped: bool,
    playhead: f64,
    playing: Option<Playing>,
    view_start: f64,
    view_len: f64,
    wave: [f32; 4],
    overview: [f32; 4],
    /// The view keeps the playhead in sight while playing, until it is moved by hand.
    follow: bool,
    drag: Option<Drag>,
    clock: Instant,
    /// Whether the timing was touched: the detected one isn't overwritten by a late result.
    touched: bool,
}

pub fn is_audio(path: &Path) -> bool {
    FileKind::Audio.matches(path)
}

pub fn is_image(path: &Path) -> bool {
    FileKind::Image.matches(path)
}

impl Home {
    pub fn new(recent: &[String], creator: &str) -> Home {
        let mut h = Home {
            page: Page::Start,
            recent: recent
                .iter()
                .map(PathBuf::from)
                .filter(|p| p.is_file())
                .collect(),
            audio: None,
            texts: Default::default(),
            carets: [0; 8],
            focus: None,
            num_edit: None,
            background: None,
            #[cfg(target_os = "android")]
            picked_song: None,
            #[cfg(target_os = "android")]
            picked_background: None,
            tko: false,
            loading: Loading::Nothing,
            dialog: None,
            status: String::new(),
            bpm: 120.0,
            offset: 0.0,
            per_bar: 4,
            analysis: None,
            taps: Vec::new(),
            tap_at: None,
            tap_origin: Instant::now(),
            metronome: true,
            looped: false,
            playhead: 0.0,
            playing: None,
            view_start: 0.0,
            view_len: 8000.0,
            wave: [0.0; 4],
            overview: [0.0; 4],
            follow: true,
            drag: None,
            clock: Instant::now(),
            touched: false,
        };
        h.set_text(Field::Creator, creator);
        h.set_text(Field::Version, "Oni");
        h
    }

    fn text(&self, f: Field) -> &str {
        &self.texts[f.index()]
    }

    fn set_text(&mut self, f: Field, text: &str) {
        let i = f.index();
        self.texts[i] = text.to_string();
        self.carets[i] = text.chars().count();
    }

    fn decoded(&self) -> Option<&Decoded> {
        match &self.loading {
            Loading::Ready(d) => Some(d),
            _ => None,
        }
    }

    fn duration(&self) -> f64 {
        self.decoded()
            .map_or(0.0, |d| d.track.frames() as f64 / d.rate * 1000.0)
    }

    /// The first beat as it is heard in the song, not as the chart writes it.
    fn audio_offset(&self) -> f64 {
        self.offset + OSU_TIMING_MS
    }

    /// `TAIKO_HOME`'s page, for screenshots.
    pub fn debug_page(&mut self, page: &str) {
        self.page = match page {
            "details" => Page::Details,
            "timing" => Page::Timing,
            _ => self.page,
        };
        if page == "timing" {
            self.focus = None;
        }
    }

    pub fn needs_text(&self) -> bool {
        (self.page == Page::Details && self.focus.is_some())
            || (self.page == Page::Timing && self.num_edit.is_some())
    }

    /// The map as it would be made now.
    fn plan(&self) -> Option<NewMap> {
        Some(NewMap {
            audio: self.audio.clone()?,
            background: self.background.clone(),
            artist: self.text(Field::Artist).trim().to_string(),
            title: self.text(Field::Title).trim().to_string(),
            artist_romanised: self.text(Field::ArtistRomanised).trim().to_string(),
            title_romanised: self.text(Field::TitleRomanised).trim().to_string(),
            creator: self.text(Field::Creator).trim().to_string(),
            version: self.text(Field::Version).trim().to_string(),
            source: self.text(Field::Source).trim().to_string(),
            tags: self.text(Field::Tags).trim().to_string(),
            bpm: self.bpm,
            offset_ms: self.offset,
            per_bar: self.per_bar,
            length_ms: self.duration(),
            tko: self.tko,
        })
    }

    // ---- loading a song ----

    /// Starts on `path`: its names are guessed from the file name, and it is decoded and its tempo
    /// looked for in the background.
    pub fn set_song(&mut self, path: PathBuf, engine_rate: Option<u32>) {
        self.stop_playing(None);
        #[cfg(target_os = "android")]
        {
            self.picked_song = None;
        }
        let (artist, title) = new_map::guess_names(&path);
        self.set_text(Field::Artist, &artist);
        self.set_text(Field::Title, &title);
        self.set_text(Field::ArtistRomanised, "");
        self.set_text(Field::TitleRomanised, "");
        self.audio = Some(path.clone());
        self.analysis = None;
        self.touched = false;
        self.taps.clear();
        self.playhead = 0.0;
        self.view_start = 0.0;
        self.view_len = 8000.0;
        self.bpm = 120.0;
        self.offset = 0.0;
        self.status.clear();
        let rate = engine_rate.unwrap_or(44_100);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::audio::decode_shared(&path, rate).map(|track| {
                let wave = super::wave::Wave::build(&track.samples, rate as f64, 1.0);
                let analysis = tempo::analyse(&track.samples, rate as f64);
                Decoded {
                    track,
                    wave,
                    analysis,
                    rate: rate as f64,
                }
            });
            let _ = tx.send(result);
        });
        self.loading = Loading::Working(rx);
        self.page = Page::Details;
        self.focus = Some(if artist.is_empty() {
            Field::Artist
        } else {
            Field::Title
        });
    }

    fn apply_analysis(&mut self, a: &tempo::Analysis) {
        self.bpm = super::timing::whole_bpm_if_near((a.bpm * 1000.0).round() / 1000.0);
        self.offset = NewMap::chart_time(a.offset_ms).round();
        self.analysis = Some(a.clone());
    }

    fn set_background(&mut self, path: Option<PathBuf>) {
        self.background = path;
        #[cfg(target_os = "android")]
        {
            self.picked_background = None;
        }
    }

    /// A file dropped on the window.
    pub fn dropped(&mut self, path: PathBuf, engine_rate: Option<u32>) -> Out {
        if is_audio(&path) {
            self.set_song(path, engine_rate);
            Out::None
        } else if is_image(&path) && self.audio.is_some() {
            self.set_background(Some(path));
            self.page = Page::Details;
            Out::None
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("tko"))
        {
            Out::Open(path)
        } else {
            Out::Toast(tr!("drop a song (mp3, ogg, wav, flac) to make a map from it").into())
        }
    }

    fn ask_for_file(&mut self, kind: FileKind, title: &str) {
        if self.dialog.is_some() {
            return;
        }
        #[cfg(target_os = "android")]
        let rx = {
            let _ = title;
            crate::android::pick_file(kind)
        };
        #[cfg(not(target_os = "android"))]
        let rx = {
            let start = self
                .audio
                .as_ref()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .or_else(dirs_music)
                .unwrap_or_else(|| PathBuf::from("."));
            let (tx, rx) = mpsc::channel();
            let title = title.to_string();
            std::thread::spawn(move || {
                let _ = tx.send(crate::library::pick_file(kind, &title, &start));
            });
            rx
        };
        self.dialog = Some(Dialog { kind, rx });
    }

    // ---- every frame ----

    /// Moves things on: results of the decoding and of the dialog, the playhead.
    pub fn update(&mut self, now: Instant, audio: Option<&Engine>) -> Out {
        let mut out = Out::None;
        if let Loading::Working(rx) = &self.loading {
            match rx.try_recv() {
                Ok(Ok(d)) => {
                    if let (Some(a), false) = (&d.analysis, self.touched) {
                        let a = a.clone();
                        self.apply_analysis(&a);
                        self.view_start = (self.audio_offset() - 500.0).max(0.0);
                    }
                    self.loading = Loading::Ready(Box::new(d));
                }
                Ok(Err(e)) => self.loading = Loading::Failed(e),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = Loading::Failed(tr!("decoding stopped").into())
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(d) = &self.dialog
            && let Some(result) = match d.rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(tr!(
                    "file selection stopped",
                    "ファイルの選択が中断されました",
                    "выбор файла прерван"
                )
                .into())),
                Err(mpsc::TryRecvError::Empty) => None,
            }
        {
            let kind = d.kind;
            self.dialog = None;
            match result {
                Ok(Some(file)) => out = self.use_picked_file(file, kind, audio),
                Ok(None) => {}
                Err(e) => {
                    out = Out::Toast(if cfg!(target_os = "android") {
                        tr!(
                            "File selection failed: {}",
                            "ファイルの選択に失敗しました: {}",
                            "не удалось выбрать файл: {}";
                            e
                        )
                    } else {
                        e
                    });
                }
            }
        }
        self.follow_playback(now, audio);
        out
    }

    fn use_picked_file(&mut self, file: PickedFile, kind: FileKind, audio: Option<&Engine>) -> Out {
        #[cfg(target_os = "android")]
        let path = file.0.clone();
        #[cfg(not(target_os = "android"))]
        let path = file;
        if cfg!(target_os = "android") && !kind.matches(&path) {
            let extensions = kind.extensions().join(", ");
            return Out::Toast(tr!(
                "choose a file of type {}",
                "次の形式のファイルを選んでください: {}",
                "выберите файл одного из типов: {}";
                extensions
            ));
        }
        match kind {
            FileKind::Audio => {
                self.set_song(path, audio.map(|a| a.rate));
                #[cfg(target_os = "android")]
                {
                    self.picked_song = Some(file);
                }
            }
            FileKind::Image => {
                self.set_background(Some(path));
                #[cfg(target_os = "android")]
                {
                    self.picked_background = Some(file);
                }
            }
        }
        Out::None
    }

    fn follow_playback(&mut self, now: Instant, audio: Option<&Engine>) {
        let Some(p) = &self.playing else { return };
        // The playhead is in the hand: the music catches up when it is let go.
        if matches!(self.drag, Some(Drag::Overview)) {
            return;
        }
        let (from, started, baked_to) = (p.from, p.at, p.baked_to);
        let estimate = from + started.elapsed().as_secs_f64() * 1000.0;
        let pos = audio
            .and_then(|a| a.position_ms(now))
            .filter(|_| started.elapsed().as_millis() > 150)
            .unwrap_or(estimate);
        self.playhead = pos;
        if pos >= self.duration() {
            self.stop_playing(audio);
            return;
        }
        if self.looped {
            let (a, b) = self.loop_range();
            if pos >= b {
                self.start_playing(audio, a);
                return;
            }
        } else if self.metronome && pos > baked_to - 2000.0 {
            self.start_playing(audio, pos);
            return;
        }
        // The view keeps up with the playhead, unless it was moved away by hand.
        if self.follow && (pos < self.view_start || pos > self.view_start + self.view_len * 0.9) {
            self.view_start = (pos - self.view_len * 0.1).max(0.0);
        }
    }

    /// The loop's start and end in the song (ms).
    fn loop_range(&self) -> (f64, f64) {
        let period = 60_000.0 / self.bpm.max(1.0);
        // A first beat before the song starts: the loop begins on the first beat after it.
        let off = self.audio_offset();
        let a = if off < 0.0 {
            off + (-off / period).ceil() * period
        } else {
            off
        };
        (a, a + period * (self.per_bar * LOOP_BARS) as f64)
    }

    // ---- playback ----

    fn start_playing(&mut self, audio: Option<&Engine>, from: f64) {
        let (Some(a), Some(d)) = (audio, self.decoded()) else {
            return;
        };
        let track = d.track.clone();
        let rate = d.rate;
        let from = from.clamp(0.0, self.duration());
        let mut baked_to = f64::MAX;
        if self.metronome || self.looped {
            let len = if self.looped {
                let (s, e) = self.loop_range();
                (e - s) + 500.0
            } else {
                BAKE_MS
            };
            let f0 = ((from / 1000.0 * rate) as usize).min(track.frames());
            let f1 = (((from + len) / 1000.0 * rate) as usize).min(track.frames());
            let mut samples = track.samples[2 * f0..2 * f1].to_vec();
            if self.metronome {
                tempo::mix_clicks(
                    &mut samples,
                    rate,
                    f0 as i64,
                    self.bpm,
                    self.audio_offset(),
                    self.per_bar,
                    tempo::MUSIC_UNDER_CLICKS,
                );
            }
            baked_to = from + len;
            a.play_part(
                Arc::new(Track {
                    samples,
                    rate: 1.0,
                    stream: None,
                }),
                f0 as i64,
                from,
            );
        } else {
            a.play(track, from);
        }
        self.playing = Some(Playing {
            from,
            at: Instant::now(),
            baked_to,
        });
        self.playhead = from;
    }

    fn stop_playing(&mut self, audio: Option<&Engine>) {
        if self.playing.take().is_some()
            && let Some(a) = audio
        {
            a.stop();
        }
    }

    /// The metronome or the grid changed under a playing song: carry on from the same place.
    fn restart_playing(&mut self, audio: Option<&Engine>) {
        if self.playing.is_some() {
            let at = self.playhead;
            self.start_playing(audio, at);
        }
    }

    // ---- input ----

    pub fn click(&mut self, act: Act, mouse_x: f32, audio: Option<&Engine>) -> Out {
        self.focus = match act {
            Act::Focus(f) => Some(f),
            _ => None,
        };
        // A click elsewhere puts down what was being typed.
        if self
            .num_edit
            .as_ref()
            .is_some_and(|e| act != Act::EditNum(e.which))
        {
            self.commit_num(audio);
        }
        match act {
            Act::Back => return self.back(audio),
            Act::Next => self.next(audio),
            Act::Browse => self.ask_for_file(FileKind::Audio, tr!("Choose the song")),
            Act::PickInLibrary => return Out::PickInLibrary,
            Act::Recent(i) => {
                if let Some(p) = self.recent.get(i) {
                    return Out::Open(p.clone());
                }
            }
            Act::Tour => return Out::Tour,
            Act::Focus(_) => {}
            Act::PickBackground => self.ask_for_file(
                FileKind::Image,
                tr!("Choose the background", "背景画像を選ぶ", "Выбрать фон"),
            ),
            Act::ClearBackground => self.set_background(None),
            Act::Format(tko) => self.tko = tko,
            Act::Wave => {
                self.drag = Some(Drag::Wave {
                    from_x: mouse_x,
                    start: self.view_start,
                    moved: false,
                })
            }
            Act::Overview => {
                self.drag = Some(Drag::Overview);
                self.overview_to(mouse_x);
            }
            Act::Play => self.toggle_play(audio),
            Act::Metronome => {
                self.metronome = !self.metronome;
                self.restart_playing(audio);
            }
            Act::Loop => {
                self.looped = !self.looped;
                if self.looped {
                    let (a, _) = self.loop_range();
                    self.view_start = (a - self.view_len * 0.1).max(0.0);
                    if self.playing.is_some() {
                        self.start_playing(audio, a);
                    }
                } else {
                    self.restart_playing(audio);
                }
            }
            Act::Bpm(thousandths) => {
                self.set_bpm(
                    super::timing::stepped_bpm(self.bpm, thousandths as f64 / 1000.0),
                    audio,
                );
            }
            Act::Offset(ms) => {
                self.offset += ms as f64;
                self.changed(audio);
            }
            Act::Shift(beats) => {
                self.offset += beats as f64 * 60_000.0 / self.bpm.max(1.0);
                self.offset = self.offset.round();
                self.changed(audio);
            }
            Act::Half => self.set_bpm(self.bpm / 2.0, audio),
            Act::Double => self.set_bpm(self.bpm * 2.0, audio),
            Act::Meter(n) => {
                self.per_bar = n as u32;
                self.changed(audio);
            }
            Act::EditNum(which) => {
                if self.num_edit.as_ref().map(|e| e.which) != Some(which) {
                    let text = match which {
                        Num::Bpm => format!("{:.3}", self.bpm)
                            .trim_end_matches('0')
                            .trim_end_matches('.')
                            .to_string(),
                        Num::Offset => format!("{}", self.offset.round()),
                    };
                    self.num_edit = Some(NumEdit {
                        which,
                        caret: text.chars().count(),
                        text,
                        fresh: true,
                    });
                }
            }
            Act::Tap => self.tap(None, audio),
            Act::Detect => self.detect(),
            Act::BeatHere => {
                self.offset = (self.playhead - OSU_TIMING_MS).round();
                self.changed(audio);
            }
            Act::Create => return self.create(),
        }
        Out::None
    }

    /// Takes what was typed in the BPM or first beat field; text that isn't a number keeps the
    /// old value.
    fn commit_num(&mut self, audio: Option<&Engine>) {
        let Some(e) = self.num_edit.take() else {
            return;
        };
        match e.which {
            Num::Bpm => match parse_bpm(&e.text) {
                Some(v) => self.set_bpm(v, audio),
                None => self.status = "that isn't a BPM: kept the old one".into(),
            },
            Num::Offset => match parse_offset(&e.text) {
                Some(v) => {
                    self.offset = v;
                    self.changed(audio);
                }
                None => self.status = "that isn't a time: kept the old one".into(),
            },
        }
    }

    /// A key while a number is typed.
    fn type_num(&mut self, code: KeyCode, text: Option<&str>, ctrl: bool, audio: Option<&Engine>) {
        let Some(e) = self.num_edit.as_mut() else {
            return;
        };
        match code {
            KeyCode::Escape => self.num_edit = None,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Tab => self.commit_num(audio),
            KeyCode::Backspace => {
                if e.fresh {
                    e.text.clear();
                    e.caret = 0;
                } else {
                    caret_backspace(&mut e.text, &mut e.caret);
                }
                e.fresh = false;
            }
            KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Delete => {
                e.fresh = false;
                caret_nav(&mut e.text, &mut e.caret, code);
            }
            _ if ctrl => {}
            _ => {
                if let Some(typed) = text {
                    let which = e.which;
                    let keep = |ch: &char| {
                        ch.is_ascii_digit()
                            || matches!(ch, '.' | ',')
                            || (*ch == '-' && which == Num::Offset)
                    };
                    let typed: String = typed.chars().filter(keep).collect();
                    if !typed.is_empty() {
                        if e.fresh {
                            e.text.clear();
                            e.caret = 0;
                            e.fresh = false;
                        }
                        caret_insert(&mut e.text, &mut e.caret, &typed, 12);
                    }
                }
            }
        }
    }

    fn set_bpm(&mut self, bpm: f64, audio: Option<&Engine>) {
        self.bpm = ((bpm * 1000.0).round() / 1000.0).clamp(20.0, 600.0);
        self.changed(audio);
    }

    fn changed(&mut self, audio: Option<&Engine>) {
        self.touched = true;
        self.restart_playing(audio);
    }

    fn detect(&mut self) {
        match self.decoded().and_then(|d| d.analysis.clone()) {
            Some(a) => {
                self.apply_analysis(&a);
                self.touched = true;
                self.status = tr!("back to what was found").into();
            }
            None => {
                self.status =
                    tr!("no tempo could be found in this song: tap it or set it by hand").into()
            }
        }
    }

    fn create(&mut self) -> Out {
        let Some(plan) = self.plan() else {
            return Out::Toast(tr!("pick a song first").into());
        };
        let problems = plan.problems();
        if !problems.is_empty() {
            self.page = Page::Details;
            return Out::Toast(problems.join(", "));
        }
        Out::Create(Box::new(plan))
    }

    fn back(&mut self, audio: Option<&Engine>) -> Out {
        match self.page {
            Page::Start => Out::Leave,
            Page::Details => {
                self.stop_playing(audio);
                self.page = Page::Start;
                Out::None
            }
            Page::Timing => {
                self.stop_playing(audio);
                self.page = Page::Details;
                Out::None
            }
        }
    }

    fn next(&mut self, audio: Option<&Engine>) {
        match self.page {
            Page::Start => self.ask_for_file(FileKind::Audio, tr!("Choose the song")),
            Page::Details => {
                let said = self.plan().map(|p| p.problems()).unwrap_or_default();
                if said.is_empty() {
                    self.page = Page::Timing;
                } else {
                    self.status = said.join(", ");
                }
            }
            Page::Timing => {
                self.stop_playing(audio);
            }
        }
    }

    fn toggle_play(&mut self, audio: Option<&Engine>) {
        if self.playing.is_some() {
            self.stop_playing(audio);
        } else if self.decoded().is_some() {
            let from = if self.playhead >= self.duration() - 100.0 {
                0.0
            } else {
                self.playhead
            };
            self.follow = true;
            self.start_playing(audio, from);
        }
    }

    /// A tap at `at` (song time while playing): the tempo from the taps so far.
    fn tap(&mut self, song_ms: Option<f64>, audio: Option<&Engine>) {
        let now = self.tap_origin.elapsed().as_secs_f64() * 1000.0;
        if self
            .tap_at
            .is_some_and(|t| t.elapsed().as_secs_f64() * 1000.0 > TAP_RESET_MS)
        {
            self.taps.clear();
        }
        self.tap_at = Some(Instant::now());
        self.taps.push(song_ms.unwrap_or(now));
        if let Some((bpm, beat)) = tempo::tap(&self.taps) {
            self.bpm = ((bpm * 100.0).round() / 100.0).clamp(20.0, 600.0);
            // Taps follow the music's beats only when it plays; and a hand is late: with a
            // tempo found by listening the first beat is kept.
            if song_ms.is_some() && self.analysis.is_none() {
                let period = 60_000.0 / self.bpm;
                // The beat nearest the start of the song, which may be a little before it.
                let mut at = (beat - OSU_TIMING_MS).rem_euclid(period);
                if at > period / 2.0 {
                    at -= period;
                }
                self.offset = at.round();
            }
            self.status = format!("{} taps: {:.2} BPM", self.taps.len(), bpm);
        } else {
            self.status = format!("{} taps (4 give a tempo)", self.taps.len());
        }
        self.touched = true;
        self.restart_playing(audio);
    }

    fn seek_to_x(&mut self, x: f32, audio: Option<&Engine>) {
        let [wx, _, ww, _] = self.wave;
        if ww <= 0.0 {
            return;
        }
        let t = self.view_start + ((x - wx) / ww).clamp(0.0, 1.0) as f64 * self.view_len;
        self.playhead = t.clamp(0.0, self.duration());
        self.follow = true;
        if self.playing.is_some() {
            let at = self.playhead;
            self.start_playing(audio, at);
        }
    }

    /// Whether a drag started here is going on (the game sends the moves and the release).
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn mouse_move(&mut self, x: f32) {
        match self.drag {
            Some(Drag::Wave {
                from_x,
                start,
                moved,
            }) => {
                let [_, _, ww, _] = self.wave;
                if !moved && (x - from_x).abs() < 3.0 || ww <= 0.0 {
                    return;
                }
                self.drag = Some(Drag::Wave {
                    from_x,
                    start,
                    moved: true,
                });
                let ms = (from_x - x) as f64 / ww as f64 * self.view_len;
                self.pan_to(start + ms);
            }
            Some(Drag::Overview) => self.overview_to(x),
            None => {}
        }
    }

    pub fn release(&mut self, x: f32, audio: Option<&Engine>) {
        match self.drag.take() {
            Some(Drag::Wave { moved: false, .. }) => self.seek_to_x(x, audio),
            Some(Drag::Overview) if self.playing.is_some() => {
                let at = self.playhead;
                self.start_playing(audio, at);
            }
            _ => {}
        }
    }

    /// The view starts at `start` (held inside the song) and stops following the playhead.
    fn pan_to(&mut self, start: f64) {
        let max = (self.duration() - self.view_len).max(0.0);
        self.view_start = start.clamp(0.0, max);
        self.follow = false;
    }

    /// The playhead to the point of the song under `x` on the overview, with the view around it.
    fn overview_to(&mut self, x: f32) {
        let [ox, _, ow, _] = self.overview;
        if ow <= 0.0 {
            return;
        }
        let t = ((x - ox) / ow).clamp(0.0, 1.0) as f64 * self.duration();
        self.playhead = t;
        self.pan_to(t - self.view_len * 0.3);
        self.follow = true;
    }

    /// The wheel moves along the song; with `zoom` (Ctrl or Alt held) it zooms around the
    /// pointer instead.
    pub fn wheel(&mut self, notches: i32, mouse_x: f32, zoom: bool) {
        if self.page != Page::Timing || notches == 0 {
            return;
        }
        if !zoom {
            // Up goes back in the song, as a page scrolls up.
            self.pan_to(self.view_start - notches as f64 * self.view_len * 0.15);
            return;
        }
        let [wx, _, ww, _] = self.wave;
        let frac = if ww > 0.0 {
            ((mouse_x - wx) / ww).clamp(0.0, 1.0) as f64
        } else {
            0.5
        };
        let anchor = self.view_start + frac * self.view_len;
        let max = self.duration().max(2000.0);
        self.view_len = (self.view_len * 0.8f64.powi(notches)).clamp(1500.0, max);
        self.view_start =
            (anchor - frac * self.view_len).clamp(0.0, (max - self.view_len).max(0.0));
    }

    /// A key. `text`: what it types; `at`: when it was pressed (taps are timed by it).
    #[allow(clippy::too_many_arguments)]
    pub fn key(
        &mut self,
        code: KeyCode,
        text: Option<&str>,
        ctrl: bool,
        shift: bool,
        at: Instant,
        audio: Option<&Engine>,
    ) -> Out {
        if let Some(f) = self.focus.filter(|_| self.page == Page::Details) {
            return self.type_key(f, code, text, ctrl, shift, audio);
        }
        if self.page == Page::Timing && self.num_edit.is_some() {
            self.type_num(code, text, ctrl, audio);
            return Out::None;
        }
        match (self.page, code) {
            (_, KeyCode::Escape) => return self.back(audio),
            (Page::Start, KeyCode::Enter | KeyCode::NumpadEnter) => {
                self.ask_for_file(FileKind::Audio, tr!("Choose the song"))
            }
            (Page::Start, KeyCode::KeyL) => return Out::PickInLibrary,
            (Page::Start, KeyCode::KeyT) => return Out::Tour,
            (
                Page::Start,
                KeyCode::Digit1
                | KeyCode::Digit2
                | KeyCode::Digit3
                | KeyCode::Digit4
                | KeyCode::Digit5
                | KeyCode::Digit6,
            ) => {
                let i = match code {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    _ => 5,
                };
                if let Some(p) = self.recent.get(i) {
                    return Out::Open(p.clone());
                }
            }
            (Page::Details, KeyCode::Tab | KeyCode::Enter | KeyCode::NumpadEnter) => {
                self.focus = Some(Field::Artist);
            }
            (Page::Timing, KeyCode::Space) => self.toggle_play(audio),
            (Page::Timing, KeyCode::KeyT) => {
                let song = self
                    .playing
                    .is_some()
                    .then(|| audio.and_then(|a| a.position_ms(at)))
                    .flatten();
                self.tap(song, audio);
            }
            (Page::Timing, KeyCode::KeyM) => {
                return self.click(Act::Metronome, 0.0, audio);
            }
            (Page::Timing, KeyCode::KeyL) => {
                return self.click(Act::Loop, 0.0, audio);
            }
            (Page::Timing, KeyCode::KeyB) => {
                return self.click(Act::BeatHere, 0.0, audio);
            }
            (Page::Timing, KeyCode::KeyD) => self.detect(),
            (Page::Timing, KeyCode::Enter | KeyCode::NumpadEnter) => return self.create(),
            (Page::Timing, KeyCode::ArrowLeft | KeyCode::ArrowRight) => {
                let by = self.view_len * if shift { 0.5 } else { 0.2 };
                let sign = if code == KeyCode::ArrowLeft {
                    -1.0
                } else {
                    1.0
                };
                self.pan_to(self.view_start + sign * by);
            }
            (Page::Timing, KeyCode::Equal | KeyCode::NumpadAdd) => {
                self.wheel(1, self.wave[0] + self.wave[2] / 2.0, true)
            }
            (Page::Timing, KeyCode::Minus | KeyCode::NumpadSubtract) => {
                self.wheel(-1, self.wave[0] + self.wave[2] / 2.0, true)
            }
            _ => {}
        }
        Out::None
    }

    fn type_key(
        &mut self,
        f: Field,
        code: KeyCode,
        text: Option<&str>,
        ctrl: bool,
        shift: bool,
        audio: Option<&Engine>,
    ) -> Out {
        let i = f.index();
        match code {
            KeyCode::Escape => self.focus = None,
            KeyCode::Tab => {
                let n = Field::ALL.len();
                let next = (i + if shift { n - 1 } else { 1 }) % n;
                self.focus = Some(Field::ALL[next]);
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if i + 1 < Field::ALL.len() {
                    self.focus = Some(Field::ALL[i + 1]);
                } else {
                    self.focus = None;
                    self.next(audio);
                }
            }
            KeyCode::Backspace => caret_backspace(&mut self.texts[i], &mut self.carets[i]),
            KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Delete => {
                caret_nav(&mut self.texts[i], &mut self.carets[i], code);
            }
            KeyCode::KeyV if ctrl => {
                if let Some(pasted) = crate::clipboard::get() {
                    let line = crate::clipboard::one_line(&pasted);
                    caret_insert(&mut self.texts[i], &mut self.carets[i], &line, FIELD_MAX);
                }
            }
            _ if ctrl => {}
            _ => {
                if let Some(typed) = text {
                    caret_insert(&mut self.texts[i], &mut self.carets[i], typed, FIELD_MAX);
                }
            }
        }
        Out::None
    }

    // ---- drawing ----

    pub fn draw(&mut self, c: &mut Canvas) {
        let w = c.width;
        c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x0e0e15, 1.0));
        // A stripe of the editor's colour along the top, and the page's name.
        c.rect(0.0, 0.0, w, 3.0, rgba(VIOLET, 1.0));
        let (title, step) = match self.page {
            Page::Start => (tr!("MAP EDITOR"), None),
            Page::Details => (tr!("NEW MAP"), Some(tr!("1 of 2: the names"))),
            Page::Timing => (tr!("NEW MAP"), Some(tr!("2 of 2: the timing"))),
        };
        crate::style::title(c, 24.0, 14.0, 20.0, 0xffffff, title);
        if let Some(step) = step {
            c.text(24.0, 40.0, 8.0, rgba(DIM, 1.0), step);
        }
        match self.page {
            Page::Start => self.draw_start(c),
            Page::Details => self.draw_details(c),
            Page::Timing => self.draw_timing(c),
        }
        if self.dialog.is_some() {
            c.rect(0.0, 0.0, w, VIRTUAL_HEIGHT, rgba(0x000000, 0.55));
            c.text_centered(
                w / 2.0,
                VIRTUAL_HEIGHT / 2.0 - 5.0,
                10.0,
                rgba(TEXT, 1.0),
                tr!("Choose a file in the window that opened..."),
            );
        }
    }

    fn panel(&self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
        c.round_rect([x, y], [w, h], 6.0, rgba(PANEL, 1.0), rgba(EDGE, 1.0), 1.0);
    }

    fn draw_start(&mut self, c: &mut Canvas) {
        let w = c.width;
        let col = (w - 48.0).min(640.0);
        let x = (w - col) / 2.0;
        // The drop zone: a new map.
        let zone = [x, 66.0, col, 150.0];
        let hover = c.hovered(zone[0], zone[1], zone[2], zone[3]);
        c.round_rect(
            [zone[0], zone[1]],
            [zone[2], zone[3]],
            8.0,
            rgba(if hover { 0x242436 } else { PANEL }, 1.0),
            rgba(VIOLET, if hover { 1.0 } else { 0.7 }),
            2.0,
        );
        c.text_centered(
            w / 2.0,
            90.0,
            14.0,
            rgba(0xffffff, 1.0),
            tr!("Make a new map"),
        );
        c.text_centered(
            w / 2.0,
            116.0,
            8.0,
            rgba(TEXT, 1.0),
            if cfg!(target_os = "android") {
                tr!(
                    "Choose a song from your device: mp3, ogg, wav or flac",
                    "端末から曲を選んでください: mp3、ogg、wav、flac",
                    "Выберите песню на устройстве: mp3, ogg, wav или flac"
                )
            } else {
                tr!(
                    "Drop a song here: mp3, ogg, wav or flac",
                    "曲をここにドロップ: mp3、ogg、wav、flac",
                    "Перетащите сюда песню: mp3, ogg, wav или flac"
                )
            },
        );
        c.text_centered(
            w / 2.0,
            132.0,
            7.0,
            rgba(DIM, 1.0),
            tr!("The tempo and the first beat are found for you, and you can time it by ear"),
        );
        c.button(
            w / 2.0 - 120.0,
            160.0,
            240.0,
            26.0,
            if cfg!(target_os = "android") {
                tr!(
                    "Choose a song file...",
                    "曲のファイルを選ぶ...",
                    "Выбрать файл песни..."
                )
            } else {
                tr!(
                    "Choose a song file...   [Enter]",
                    "曲のファイルを選ぶ...   [Enter]",
                    "Выбрать файл песни...   [Enter]"
                )
            },
            8.0,
            Hit::EditorHome(Act::Browse),
            true,
        );
        c.hit(
            zone[0],
            zone[1],
            zone[2],
            zone[3],
            Hit::EditorHome(Act::Browse),
        );
        // Going on with one.
        let top = 232.0;
        c.text(x, top, 9.0, rgba(0xffffff, 1.0), tr!("Go on with a map"));
        let list_h = 6.0 * 22.0 + 8.0;
        self.panel(c, x, top + 16.0, col * 0.62, list_h);
        if self.recent.is_empty() {
            c.text(
                x + 12.0,
                top + 30.0,
                8.0,
                rgba(DIM, 1.0),
                &Canvas::fit(
                    8.0,
                    col * 0.62 - 24.0,
                    tr!("Maps you open in the editor will be listed here."),
                ),
            );
        }
        for (i, path) in self.recent.iter().take(6).enumerate() {
            let y = top + 20.0 + i as f32 * 22.0;
            let hover = c.hovered(x + 4.0, y, col * 0.62 - 8.0, 20.0);
            if hover {
                c.round_rect(
                    [x + 4.0, y],
                    [col * 0.62 - 8.0, 20.0],
                    4.0,
                    rgba(0x2b2b40, 1.0),
                    rgba(VIOLET, 0.7),
                    1.0,
                );
            }
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            c.text(
                x + 12.0,
                y + 6.0,
                7.0,
                rgba(DIM, 1.0),
                &format!("{}", i + 1),
            );
            // Which file it is: the .tko and the .osu of one map otherwise read the same.
            let (kind, tint) = file_badge(path);
            let bw = Canvas::text_width(6.0, kind) + 8.0;
            let right = x + col * 0.62 - 10.0;
            c.round_rect(
                [right - bw, y + 4.0],
                [bw, 12.0],
                3.0,
                rgba(tint, 0.18),
                rgba(tint, 0.8),
                1.0,
            );
            c.text_centered(right - bw / 2.0, y + 7.0, 6.0, rgba(tint, 1.0), kind);
            c.text(
                x + 26.0,
                y + 5.0,
                8.0,
                rgba(TEXT, 1.0),
                &fit_keeping_diff(8.0, right - bw - 6.0 - (x + 26.0), &name),
            );
            c.hit(
                x + 4.0,
                y,
                col * 0.62 - 8.0,
                20.0,
                Hit::EditorHome(Act::Recent(i)),
            );
        }
        let bx = x + col * 0.62 + 14.0;
        let bw = col * 0.38 - 14.0;
        c.button(
            bx,
            top + 16.0,
            bw,
            24.0,
            tr!("Pick from my songs   [L]"),
            8.0,
            Hit::EditorHome(Act::PickInLibrary),
            false,
        );
        c.button(
            bx,
            top + 46.0,
            bw,
            24.0,
            tr!("Take the tour   [T]"),
            8.0,
            Hit::EditorHome(Act::Tour),
            false,
        );
        c.text(
            bx,
            top + 86.0,
            7.0,
            rgba(DIM, 1.0),
            &Canvas::fit(7.0, bw, tr!("Or drop a .osu or .tko here.")),
        );
        c.text(
            24.0,
            VIRTUAL_HEIGHT - 20.0,
            7.0,
            rgba(DIM, 1.0),
            tr!("Esc: back to the menu"),
        );
    }

    /// A number you can click and type over: a sunken field between its - and + buttons.
    fn draw_num(&self, c: &mut Canvas, x: f32, y: f32, h: f32, which: Num, shown: &str) {
        let w = 76.0;
        let edit = self.num_edit.as_ref().filter(|e| e.which == which);
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x12121a, 1.0),
            rgba(if edit.is_some() { VIOLET } else { EDGE }, 1.0),
            if edit.is_some() { 1.5 } else { 1.0 },
        );
        match edit {
            Some(e) => {
                let before: String = e.text.chars().take(e.caret).collect();
                let bw = Canvas::text_width(9.0, &before);
                let tw = Canvas::text_width(9.0, &e.text);
                let left = x + (w - tw) / 2.0;
                c.text(left, y + 6.0, 9.0, rgba(TEXT, 1.0), &e.text);
                if (self.clock.elapsed().as_millis() / 500).is_multiple_of(2) {
                    c.rect(left + bw, y + 4.0, 1.0, h - 8.0, rgba(0xffffff, 1.0));
                }
            }
            None => c.text_centered(x + w / 2.0, y + 5.0, 10.0, rgba(0xffffff, 1.0), shown),
        }
        c.hit(x, y, w, h, Hit::EditorHome(Act::EditNum(which)));
    }

    fn draw_field(&self, c: &mut Canvas, f: Field, x: f32, y: f32, w: f32) {
        let i = f.index();
        let label_size = 6.5;
        let label = Canvas::fit(label_size, w, f.label());
        c.text(x, y, label_size, rgba(DIM, 1.0), &label);
        let box_y = y + Canvas::text_height(label_size) + 3.5;
        let focused = self.focus == Some(f);
        c.round_rect(
            [x, box_y],
            [w, 18.0],
            4.0,
            rgba(0x12121a, 1.0),
            rgba(if focused { VIOLET } else { EDGE }, 1.0),
            if focused { 1.5 } else { 1.0 },
        );
        let room = w - 12.0;
        let text = &self.texts[i];
        let caret = self.carets[i].min(text.chars().count());
        let before: String = text.chars().take(caret).collect();
        let after: String = text.chars().skip(caret).collect();
        let shown_before = Canvas::fit_end(8.0, room, &before);
        let bw = Canvas::text_width(8.0, &shown_before);
        c.text(x + 6.0, box_y + 5.0, 8.0, rgba(TEXT, 1.0), &shown_before);
        if bw < room {
            c.text(
                x + 6.0 + bw,
                box_y + 5.0,
                8.0,
                rgba(TEXT, 1.0),
                &Canvas::fit(8.0, room - bw, &after),
            );
        }
        if focused && (self.clock.elapsed().as_millis() / 500).is_multiple_of(2) {
            c.rect(x + 6.0 + bw, box_y + 3.0, 1.0, 12.0, rgba(0xffffff, 1.0));
        }
        c.hit(x, box_y, w, 18.0, Hit::EditorHome(Act::Focus(f)));
    }

    fn draw_details(&mut self, c: &mut Canvas) {
        let w = c.width;
        let col = (w - 48.0).min(700.0);
        let x = (w - col) / 2.0;
        let half = (col - 14.0) / 2.0;
        // The song and what is known of it.
        let name = self
            .audio
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.panel(c, x, 58.0, col, 30.0);
        c.text(
            x + 10.0,
            65.0,
            8.0,
            rgba(TEXT, 1.0),
            &Canvas::fit(8.0, col - 150.0, &name),
        );
        let state = match &self.loading {
            Loading::Working(_) => (tr!("reading the song...").to_string(), DIM),
            Loading::Ready(d) => {
                let secs = (d.track.frames() as f64 / d.rate) as u64;
                let found = d
                    .analysis
                    .as_ref()
                    .map_or(tr!("no tempo found").to_string(), |a| {
                        tr!("{:.2} BPM found", a.bpm)
                    });
                (
                    format!("{}:{:02}   {found}", secs / 60, secs % 60),
                    0x7ee08a,
                )
            }
            Loading::Failed(e) => (
                Canvas::fit(7.0, col - 20.0, &tr!("can't read it: {e}"; e)),
                BAD,
            ),
            Loading::Nothing => (String::new(), DIM),
        };
        c.text_right(x + col - 10.0, 65.0, 7.5, rgba(state.1, 1.0), &state.0);
        c.text(
            x + 10.0,
            77.0,
            6.5,
            rgba(DIM, 1.0),
            if cfg!(target_os = "android") {
                tr!(
                    "Go back to choose a different song.",
                    "別の曲を選ぶには戻ってください。",
                    "Вернитесь назад, чтобы выбрать другую песню."
                )
            } else {
                tr!(
                    "Drop another song on the window to use that instead.",
                    "別の曲をウィンドウにドロップすると差し替えられます。",
                    "Перетащите в окно другую песню, чтобы заменить текущую."
                )
            },
        );
        let mut y = 100.0;
        let rows: [(Field, Field); 3] = [
            (Field::Artist, Field::Title),
            (Field::ArtistRomanised, Field::TitleRomanised),
            (Field::Creator, Field::Version),
        ];
        for (l, r) in rows {
            self.draw_field(c, l, x, y, half);
            self.draw_field(c, r, x + half + 14.0, y, half);
            y += 40.0;
        }
        self.draw_field(c, Field::Source, x, y, half);
        self.draw_field(c, Field::Tags, x + half + 14.0, y, half);
        y += 44.0;
        // Background and format.
        c.text(
            x,
            y,
            6.5,
            rgba(DIM, 1.0),
            tr!("Background picture (optional)"),
        );
        let bg_name = self
            .background
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        c.button(
            x,
            y + 10.0,
            120.0,
            20.0,
            tr!("Choose a picture..."),
            7.5,
            Hit::EditorHome(Act::PickBackground),
            false,
        );
        match bg_name {
            Some(n) => {
                c.text(
                    x + 128.0,
                    y + 16.0,
                    7.5,
                    rgba(TEXT, 1.0),
                    &Canvas::fit(7.5, half - 170.0, &n),
                );
                c.button(
                    x + half - 36.0,
                    y + 10.0,
                    36.0,
                    20.0,
                    "x",
                    8.0,
                    Hit::EditorHome(Act::ClearBackground),
                    false,
                );
            }
            None => {
                c.text(
                    x + 128.0,
                    y + 16.0,
                    7.0,
                    rgba(DIM, 1.0),
                    &Canvas::fit(
                        7.0,
                        (half - 170.0).max(24.0),
                        tr!("none: the map has a plain background"),
                    ),
                );
            }
        }
        let fx = x + half + 14.0;
        c.text(fx, y, 6.5, rgba(DIM, 1.0), tr!("Format"));
        for (k, (tko, label)) in [
            (false, tr!(".osu  for osu!stable too")),
            (true, tr!(".tko  + client effects")),
        ]
        .into_iter()
        .enumerate()
        {
            let bx = fx + k as f32 * (half / 2.0 + 2.0);
            let on = self.tko == tko;
            let bw = half / 2.0 - 2.0;
            let hover = c.hovered(bx, y + 10.0, bw, 20.0);
            c.round_rect(
                [bx, y + 10.0],
                [bw, 20.0],
                4.0,
                rgba(
                    if on {
                        0x2a2145
                    } else if hover {
                        0x2b2b3c
                    } else {
                        PANEL
                    },
                    1.0,
                ),
                rgba(if on { VIOLET } else { EDGE }, 1.0),
                1.0,
            );
            c.text_centered(
                bx + bw / 2.0,
                y + 16.0,
                6.5,
                rgba(if on { 0xffffff } else { DIM }, 1.0),
                &Canvas::fit(6.5, bw - 6.0, label),
            );
            c.hit(bx, y + 10.0, bw, 20.0, Hit::EditorHome(Act::Format(tko)));
        }
        // What is missing.
        if let Some(plan) = self.plan() {
            let mut notes = plan.problems();
            if plan.wants_romanised() {
                notes.push(
                    tr!("a name in other letters should have a Latin one too (osu! asks for it)")
                        .into(),
                );
            }
            if let Some(n) = notes.first() {
                c.text(x, y + 40.0, 7.0, rgba(WARN, 1.0), &Canvas::fit(7.0, col, n));
            }
        }
        if !self.status.is_empty() {
            c.text(
                x,
                y + 52.0,
                7.0,
                rgba(BAD, 1.0),
                &Canvas::fit(7.0, col, &self.status),
            );
        }
        let by = VIRTUAL_HEIGHT - 36.0;
        c.button(
            x,
            by,
            90.0,
            24.0,
            tr!("< Back"),
            8.0,
            Hit::EditorHome(Act::Back),
            false,
        );
        c.button(
            x + col - 170.0,
            by,
            170.0,
            24.0,
            tr!("Next: the timing >"),
            8.0,
            Hit::EditorHome(Act::Next),
            true,
        );
        c.text(
            x + 100.0,
            by + 8.0,
            6.5,
            rgba(DIM, 1.0),
            tr!("Tab: next field   Esc: leave the field"),
        );
    }

    fn draw_timing(&mut self, c: &mut Canvas) {
        let w = c.width;
        let col = (w - 40.0).min(780.0);
        let x = (w - col) / 2.0;
        // The waveform, and the whole song under it.
        let wave = [x, 58.0, col, 100.0];
        self.wave = wave;
        self.panel(c, wave[0], wave[1], wave[2], wave[3]);
        self.draw_wave(c, wave);
        c.hit(
            wave[0],
            wave[1],
            wave[2],
            wave[3],
            Hit::EditorHome(Act::Wave),
        );
        let overview = [x, wave[1] + wave[3] + 3.0, col, 15.0];
        self.overview = overview;
        self.draw_overview(c, overview);
        c.hit(
            overview[0],
            overview[1],
            overview[2],
            overview[3],
            Hit::EditorHome(Act::Overview),
        );
        // What was found.
        let mut y = overview[1] + overview[3] + 6.0;
        let line = match (&self.loading, &self.analysis) {
            (Loading::Working(_), _) => (tr!("Reading the song...").to_string(), DIM),
            (Loading::Failed(e), _) => (tr!("Can't read the song: {e}"; e), BAD),
            (_, Some(a)) => (
                tr!(
                    "Found {:.2} BPM, {}. First beat at {} ms.",
                    a.bpm,
                    if a.lift >= 3.0 {
                        "and it is clear"
                    } else if a.lift >= 2.0 {
                        "probably right"
                    } else {
                        "but unsure: check it by ear with the metronome"
                    },
                    NewMap::chart_time(a.offset_ms).round()
                ),
                if a.lift >= 2.0 { 0x7ee08a } else { WARN },
            ),
            _ => (
                tr!("No tempo was found: tap it (T, while the song plays) or set it by hand.")
                    .into(),
                WARN,
            ),
        };
        c.text(
            x,
            y,
            7.5,
            rgba(line.1, 1.0),
            &Canvas::fit(7.5, col, &line.0),
        );
        y += 11.0;
        if let Some(at) = self.analysis.as_ref().and_then(|a| a.unsteady_at) {
            c.text(
                x,
                y,
                7.0,
                rgba(WARN, 1.0),
                &Canvas::fit(
                    7.0,
                    col,
                    &tr!("The tempo seems to change near {}:{:02}. Time the later part in the editor (F3, F6).",
                        (at / 60_000.0) as u32,
                        ((at / 1000.0) as u32) % 60),
                ),
            );
        } else if !self.status.is_empty() {
            c.text(
                x,
                y,
                7.0,
                rgba(DIM, 1.0),
                &Canvas::fit(7.0, col, &self.status),
            );
        }
        y += 18.0;
        // BPM.
        let bh = 20.0;
        c.text(x, y + 6.0, 8.0, rgba(0xffffff, 1.0), "BPM");
        let mut bx = x + 92.0;
        for (label, d) in [("-1", -1000i16), ("-.1", -100), ("-.01", -10)] {
            c.button(
                bx,
                y,
                34.0,
                bh,
                label,
                7.5,
                Hit::EditorHome(Act::Bpm(d)),
                false,
            );
            bx += 36.0;
        }
        let shown = format!("{:.3}", self.bpm)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
        self.draw_num(c, bx + 2.0, y, bh, Num::Bpm, &shown);
        bx += 84.0;
        for (label, d) in [("+.01", 10i16), ("+.1", 100), ("+1", 1000)] {
            c.button(
                bx,
                y,
                34.0,
                bh,
                label,
                7.5,
                Hit::EditorHome(Act::Bpm(d)),
                false,
            );
            bx += 36.0;
        }
        c.button(
            bx + 8.0,
            y,
            34.0,
            bh,
            "/2",
            7.5,
            Hit::EditorHome(Act::Half),
            false,
        );
        c.button(
            bx + 46.0,
            y,
            34.0,
            bh,
            "x2",
            7.5,
            Hit::EditorHome(Act::Double),
            false,
        );
        c.button(
            bx + 92.0,
            y,
            140.0,
            bh,
            tr!("Back to found  [D]"),
            7.5,
            Hit::EditorHome(Act::Detect),
            false,
        );
        y += 28.0;
        // The first beat.
        c.text(x, y + 6.0, 8.0, rgba(0xffffff, 1.0), tr!("First beat"));
        let mut bx = x + 92.0;
        for (label, d) in [("-10", -10i16), ("-1", -1)] {
            c.button(
                bx,
                y,
                34.0,
                bh,
                label,
                7.5,
                Hit::EditorHome(Act::Offset(d)),
                false,
            );
            bx += 36.0;
        }
        self.draw_num(
            c,
            bx + 2.0,
            y,
            bh,
            Num::Offset,
            &format!("{} ms", self.offset.round()),
        );
        bx += 84.0;
        for (label, d) in [("+1", 1i16), ("+10", 10)] {
            c.button(
                bx,
                y,
                34.0,
                bh,
                label,
                7.5,
                Hit::EditorHome(Act::Offset(d)),
                false,
            );
            bx += 36.0;
        }
        c.button(
            bx + 8.0,
            y,
            64.0,
            bh,
            tr!("< 1 beat"),
            7.5,
            Hit::EditorHome(Act::Shift(-1)),
            false,
        );
        c.button(
            bx + 74.0,
            y,
            64.0,
            bh,
            tr!("1 beat >"),
            7.5,
            Hit::EditorHome(Act::Shift(1)),
            false,
        );
        c.button(
            bx + 146.0,
            y,
            150.0,
            bh,
            tr!("Beat at the playhead  [B]"),
            7.5,
            Hit::EditorHome(Act::BeatHere),
            false,
        );
        y += 28.0;
        // Meter and tap.
        c.text(x, y + 6.0, 8.0, rgba(0xffffff, 1.0), tr!("Beats per bar"));
        let mut bx = x + 92.0;
        for n in [3u8, 4] {
            let on = self.per_bar == n as u32;
            c.button(
                bx,
                y,
                34.0,
                bh,
                &format!("{n}/4"),
                7.5,
                Hit::EditorHome(Act::Meter(n)),
                on,
            );
            bx += 36.0;
        }
        c.button(
            bx + 14.0,
            y,
            150.0,
            bh,
            tr!("Tap the tempo  [T]"),
            7.5,
            Hit::EditorHome(Act::Tap),
            false,
        );
        c.text(
            bx + 172.0,
            y + 6.0,
            6.5,
            rgba(DIM, 1.0),
            &Canvas::fit(
                6.5,
                (col - (bx + 172.0 - x)).max(40.0),
                tr!("While the song plays, tap on the beat."),
            ),
        );
        y += 30.0;
        // Playback.
        let play_label = if self.playing.is_some() {
            tr!("Stop  [Space]")
        } else {
            tr!("Play  [Space]")
        };
        c.button(
            x,
            y,
            110.0,
            22.0,
            play_label,
            8.0,
            Hit::EditorHome(Act::Play),
            self.playing.is_none(),
        );
        let on = |b: bool| if b { tr!("on") } else { tr!("off") };
        c.button(
            x + 118.0,
            y,
            130.0,
            22.0,
            &format!("Metronome {}  [M]", on(self.metronome)),
            7.5,
            Hit::EditorHome(Act::Metronome),
            self.metronome,
        );
        c.button(
            x + 256.0,
            y,
            170.0,
            22.0,
            &format!("Loop {LOOP_BARS} bars {}  [L]", on(self.looped)),
            7.5,
            Hit::EditorHome(Act::Loop),
            self.looped,
        );
        c.text(
            x + 436.0,
            y + 7.0,
            6.5,
            rgba(DIM, 1.0),
            &Canvas::fit(
                6.5,
                (col - 436.0).max(40.0),
                tr!("Wheel or drag: move along. Ctrl+wheel: zoom. Click: playhead."),
            ),
        );
        let by = VIRTUAL_HEIGHT - 36.0;
        c.button(
            x,
            by,
            90.0,
            24.0,
            tr!("< Back"),
            8.0,
            Hit::EditorHome(Act::Back),
            false,
        );
        c.button(
            x + col - 190.0,
            by,
            190.0,
            24.0,
            tr!("Create the map  [Enter]"),
            8.0,
            Hit::EditorHome(Act::Create),
            true,
        );
        c.text(
            x + 100.0,
            by + 8.0,
            6.5,
            rgba(DIM, 1.0),
            tr!("Not sure? Create it anyway: the editor has the same tools (F3, F6)."),
        );
    }

    /// The whole song, small: where the view is on it, and the playhead.
    fn draw_overview(&self, c: &mut Canvas, [x, y, w, h]: [f32; 4]) {
        self.panel(c, x, y, w, h);
        let Some(d) = self.decoded() else { return };
        let total = self.duration().max(1.0);
        d.wave
            .draw(c, [x + 1.0, y + 1.5, w - 2.0, h - 3.0], 0.0, total, 0.75);
        // The stretch the waveform shows.
        let a = x + (self.view_start / total).clamp(0.0, 1.0) as f32 * w;
        let b = x + ((self.view_start + self.view_len) / total).clamp(0.0, 1.0) as f32 * w;
        c.round_rect(
            [a, y],
            [(b - a).max(2.0), h],
            2.0,
            rgba(0xffffff, 0.10),
            rgba(0xffffff, 0.55),
            1.0,
        );
        let px = x + (self.playhead / total).clamp(0.0, 1.0) as f32 * w;
        c.rect(px - 0.5, y, 1.5, h, rgba(accent(), 1.0));
    }

    fn draw_wave(&self, c: &mut Canvas, [x, y, w, h]: [f32; 4]) {
        let Some(d) = self.decoded() else {
            let text = match &self.loading {
                Loading::Working(_) => tr!("reading the song..."),
                _ => "",
            };
            c.text_centered(x + w / 2.0, y + h / 2.0 - 4.0, 8.0, rgba(DIM, 1.0), text);
            return;
        };
        let (start, len) = (self.view_start, self.view_len);
        d.wave.draw(
            c,
            [x + 1.0, y + 3.0, w - 2.0, h - 6.0],
            start,
            start + len,
            1.0,
        );
        // The grid: bars heavy and red, beats thin.
        let period = 60_000.0 / self.bpm.max(1.0);
        let spacing = period / len * w as f64;
        if spacing >= 3.0 || self.per_bar as f64 * spacing >= 3.0 {
            for (t, k) in beat_times(start, start + len, self.audio_offset(), period) {
                let px = x + ((t - start) / len * w as f64) as f32;
                let bar = k.rem_euclid(self.per_bar as i64) == 0;
                if bar {
                    c.rect(px - 0.5, y + 2.0, 1.5, h - 4.0, rgba(0xff5a5a, 0.95));
                } else if spacing >= 3.0 {
                    c.rect(px - 0.25, y + 6.0, 1.0, h - 12.0, rgba(0xffffff, 0.45));
                }
            }
        }
        // The playhead.
        if self.playhead >= start && self.playhead <= start + len {
            let px = x + ((self.playhead - start) / len * w as f64) as f32;
            c.rect(px - 0.5, y, 1.5, h, rgba(accent(), 1.0));
        }
        if self.looped {
            let (a, b) = self.loop_range();
            let (pa, pb) = (
                x + (((a - start) / len).clamp(0.0, 1.0) * w as f64) as f32,
                x + (((b - start) / len).clamp(0.0, 1.0) * w as f64) as f32,
            );
            if pb > pa {
                c.rect(pa, y, pb - pa, 3.0, rgba(VIOLET, 0.9));
            }
        }
        let time = |ms: f64| format!("{}:{:05.2}", (ms / 60_000.0) as u32, (ms / 1000.0) % 60.0);
        c.text(x + 4.0, y + h - 11.0, 6.5, rgba(DIM, 1.0), &time(start));
        c.text_right(
            x + w - 4.0,
            y + h - 11.0,
            6.5,
            rgba(DIM, 1.0),
            &time(start + len),
        );
    }
}

/// Where music usually is, for the dialog to start in.
#[cfg(not(target_os = "android"))]
fn dirs_music() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let music = PathBuf::from(home).join("Music");
    music.is_dir().then_some(music)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_maps_say_which_file_they_are_and_keep_their_difficulty() {
        assert_eq!(file_badge(Path::new("a/x.tko")).0, "TKO");
        assert_eq!(file_badge(Path::new("a/x.OSU")).0, "OSU");
        let name = "Lynerrr - diss den4ik speedrun (krokodil_koban) [Oni]";
        let w = Canvas::text_width(8.0, name) * 0.6;
        let cut = fit_keeping_diff(8.0, w, name);
        assert!(
            cut.ends_with("...[Oni]") || cut.ends_with("... [Oni]"),
            "{cut}"
        );
        assert!(cut.starts_with("Lynerrr"));
        assert!(Canvas::text_width(8.0, &cut) <= w);
        assert_eq!(fit_keeping_diff(8.0, 1000.0, name), name);
        // No room for the title beside a long difficulty: cut as usual.
        assert!(Canvas::text_width(8.0, &fit_keeping_diff(8.0, 60.0, name)) <= 60.0);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_file_dialog_applies_the_selection_and_clears_on_cancel() {
        let mut h = home();
        let cover = PathBuf::from("cover.png");
        for selection in [Some(cover.clone()), None] {
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(selection)).unwrap();
            h.dialog = Some(Dialog {
                kind: FileKind::Image,
                rx,
            });
            assert!(matches!(h.update(Instant::now(), None), Out::None));
            assert_eq!(h.background.as_ref(), Some(&cover));
            assert!(h.dialog.is_none());
        }

        let (tx, rx) = mpsc::channel();
        tx.send(Err("blocked in tests: no programs, no internet".into()))
            .unwrap();
        h.dialog = Some(Dialog {
            kind: FileKind::Audio,
            rx,
        });
        assert!(matches!(
            h.update(Instant::now(), None),
            Out::Toast(message) if message == "blocked in tests: no programs, no internet"
        ));
        assert!(h.dialog.is_none());
    }

    fn home() -> Home {
        Home::new(&[], "me")
    }

    fn loaded(h: &mut Home, frames: usize, rate: f64) {
        h.loading = Loading::Ready(Box::new(Decoded {
            track: Arc::new(Track {
                samples: vec![0.2; frames * 2],
                rate: 1.0,
                stream: None,
            }),
            wave: super::super::wave::Wave::build(&vec![0.2; frames * 2], rate, 1.0),
            analysis: None,
            rate,
        }));
    }

    #[test]
    fn a_new_screen_starts_with_the_mapper_and_a_difficulty_name() {
        let h = home();
        assert_eq!(h.page, Page::Start);
        assert_eq!(h.text(Field::Creator), "me");
        assert_eq!(h.text(Field::Version), "Oni");
        assert!(!h.needs_text());
    }

    #[test]
    fn a_song_dropped_fills_the_names_and_goes_to_the_details() {
        let mut h = home();
        let out = h.dropped(PathBuf::from("/tmp/Some Artist - Some Title.mp3"), None);
        assert!(matches!(out, Out::None));
        assert_eq!(h.page, Page::Details);
        assert_eq!(h.text(Field::Artist), "Some Artist");
        assert_eq!(h.text(Field::Title), "Some Title");
        assert!(h.needs_text(), "the cursor is already in a field");
        // Something else isn't a song.
        assert!(matches!(
            h.dropped(PathBuf::from("/tmp/notes.txt"), None),
            Out::Toast(_)
        ));
        assert!(matches!(
            h.dropped(PathBuf::from("/tmp/x/Map [Oni].osu"), None),
            Out::Open(_)
        ));
    }

    #[test]
    fn typing_goes_into_the_focused_field_and_tab_moves_on() {
        let mut h = home();
        h.page = Page::Details;
        h.focus = Some(Field::Artist);
        let now = Instant::now();
        for ch in ["N", "e", "k", "o"] {
            h.key(KeyCode::KeyA, Some(ch), false, false, now, None);
        }
        assert_eq!(h.text(Field::Artist), "Neko");
        h.key(KeyCode::Backspace, None, false, false, now, None);
        assert_eq!(h.text(Field::Artist), "Nek");
        h.key(KeyCode::Tab, None, false, false, now, None);
        assert_eq!(h.focus, Some(Field::Title));
        h.key(KeyCode::Tab, None, false, true, now, None);
        assert_eq!(h.focus, Some(Field::Artist));
        // Escape lets go of the field first, and only then leaves the page.
        h.key(KeyCode::Escape, None, false, false, now, None);
        assert_eq!((h.focus, h.page), (None, Page::Details));
        h.key(KeyCode::Escape, None, false, false, now, None);
        assert_eq!(h.page, Page::Start);
    }

    #[test]
    fn the_timing_buttons_move_the_tempo_and_the_first_beat() {
        let mut h = home();
        h.page = Page::Timing;
        h.bpm = 150.0;
        h.offset = 100.0;
        h.click(Act::Bpm(10), 0.0, None);
        assert_eq!(h.bpm, 150.01);
        // A step off its grid goes to the grid's next mark first.
        h.click(Act::Bpm(-1000), 0.0, None);
        assert_eq!(h.bpm, 150.0);
        h.click(Act::Bpm(-1000), 0.0, None);
        assert_eq!(h.bpm, 149.0);
        h.bpm = 173.998;
        h.click(Act::Bpm(10), 0.0, None);
        assert_eq!(h.bpm, 174.0, "a detected 173.998 reaches 174 with +0.01");
        h.bpm = 149.01;
        h.click(Act::Double, 0.0, None);
        assert_eq!(h.bpm, 298.02);
        h.click(Act::Half, 0.0, None);
        h.bpm = 120.0;
        h.click(Act::Offset(-10), 0.0, None);
        assert_eq!(h.offset, 90.0);
        h.click(Act::Shift(1), 0.0, None);
        assert_eq!(h.offset, 590.0, "a beat at 120 BPM is 500 ms");
        h.playhead = 1000.0;
        h.click(Act::BeatHere, 0.0, None);
        assert_eq!(
            h.offset,
            1000.0 - OSU_TIMING_MS,
            "a chart's time is the song's minus 23"
        );
        assert!(h.touched);
    }

    fn type_into(h: &mut Home, keys: &[(KeyCode, Option<&str>)]) {
        let now = Instant::now();
        for (code, text) in keys {
            h.key(*code, *text, false, false, now, None);
        }
    }

    #[test]
    fn a_typed_bpm_is_taken_on_enter_and_kept_in_range() {
        let mut h = home();
        h.page = Page::Timing;
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        assert!(h.needs_text());
        // The first key replaces the old value; a comma is a decimal point.
        type_into(
            &mut h,
            &[
                (KeyCode::Digit1, Some("1")),
                (KeyCode::Digit7, Some("7")),
                (KeyCode::Digit4, Some("4")),
                (KeyCode::Comma, Some(",")),
                (KeyCode::Digit9, Some("9")),
                (KeyCode::Digit7, Some("7")),
                (KeyCode::KeyM, Some("m")),
            ],
        );
        assert_eq!(h.num_edit.as_ref().unwrap().text, "174,97");
        assert!(h.metronome, "M didn't reach the screen's hotkey");
        type_into(&mut h, &[(KeyCode::Enter, None)]);
        assert_eq!(h.bpm, 174.97);
        assert!(h.num_edit.is_none() && h.touched);
        // Too big is held at 600, and Backspace edits.
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        type_into(
            &mut h,
            &[
                (KeyCode::Digit9, Some("9")),
                (KeyCode::Digit9, Some("9")),
                (KeyCode::Digit9, Some("9")),
                (KeyCode::Backspace, None),
                (KeyCode::Digit8, Some("8")),
                (KeyCode::Enter, None),
            ],
        );
        assert_eq!(h.bpm, 600.0);
    }

    #[test]
    fn a_typed_value_can_be_cancelled_or_ignored_and_a_click_away_commits() {
        let mut h = home();
        h.page = Page::Timing;
        h.bpm = 140.0;
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        type_into(
            &mut h,
            &[(KeyCode::Digit9, Some("9")), (KeyCode::Escape, None)],
        );
        assert_eq!((h.bpm, h.page), (140.0, Page::Timing), "Esc only cancels");
        assert!(h.num_edit.is_none());
        // Nothing usable keeps the old value.
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        type_into(
            &mut h,
            &[(KeyCode::Period, Some(".")), (KeyCode::Enter, None)],
        );
        assert_eq!(h.bpm, 140.0);
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        type_into(
            &mut h,
            &[(KeyCode::Backspace, None), (KeyCode::Enter, None)],
        );
        assert_eq!(h.bpm, 140.0, "an empty field keeps the value");
        // Clicking another button commits it first.
        h.click(Act::EditNum(Num::Bpm), 0.0, None);
        type_into(
            &mut h,
            &[(KeyCode::Digit9, Some("9")), (KeyCode::Digit0, Some("0"))],
        );
        h.click(Act::Metronome, 0.0, None);
        assert_eq!(h.bpm, 90.0);
        assert!(h.num_edit.is_none());
    }

    #[test]
    fn the_first_beat_can_be_typed_and_can_be_negative() {
        let mut h = home();
        h.page = Page::Timing;
        h.audio = Some(PathBuf::from("song.mp3"));
        h.click(Act::EditNum(Num::Offset), 0.0, None);
        type_into(
            &mut h,
            &[
                (KeyCode::Minus, Some("-")),
                (KeyCode::Digit1, Some("1")),
                (KeyCode::Digit2, Some("2")),
                (KeyCode::Digit0, Some("0")),
                (KeyCode::Enter, None),
            ],
        );
        assert_eq!(h.offset, -120.0);
        assert_eq!(
            h.plan().unwrap().offset_ms,
            -120.0,
            "the new map gets the negative time"
        );
        // The step buttons carry on below zero too.
        h.click(Act::Offset(-10), 0.0, None);
        assert_eq!(h.offset, -130.0);
        // Letters never get in.
        h.click(Act::EditNum(Num::Offset), 0.0, None);
        type_into(
            &mut h,
            &[(KeyCode::KeyB, Some("b")), (KeyCode::Enter, None)],
        );
        assert_eq!(h.offset, -130.0);
    }

    #[test]
    fn a_negative_first_beat_is_written_to_the_map() {
        let m = NewMap {
            audio: PathBuf::from("a.mp3"),
            background: None,
            artist: "A".into(),
            title: "T".into(),
            artist_romanised: String::new(),
            title_romanised: String::new(),
            creator: "me".into(),
            version: "Oni".into(),
            source: String::new(),
            tags: String::new(),
            bpm: 120.0,
            offset_ms: -250.0,
            per_bar: 4,
            length_ms: 10_000.0,
            tko: false,
        };
        let text = new_map::osu_text(&m, "a.mp3", None);
        let chart = super::super::chart::Chart::parse(&text);
        assert_eq!(chart.timing[0].time, -250.0);
        assert!(text.contains("-250,500,4,"));
    }

    #[test]
    fn the_loop_starts_on_the_first_beat_inside_the_song() {
        let mut h = home();
        h.bpm = 120.0;
        h.offset = -1000.0 - OSU_TIMING_MS + 100.0;
        let (a, _) = h.loop_range();
        assert!((a - 100.0).abs() < 1e-6, "{a}");
    }

    #[test]
    fn tap_tempo_picks_the_beat_nearest_the_song_start() {
        let mut h = home();
        h.page = Page::Timing;
        // Beats at 120 BPM, 450 ms + k * 500 (audio time): the nearest to zero is -50.
        for k in 0..6 {
            h.tap(Some(450.0 + k as f64 * 500.0), None);
        }
        assert_eq!(h.offset, (-50.0 - OSU_TIMING_MS).round());
    }

    #[test]
    fn beat_ticks_start_at_the_beginning_of_the_song() {
        let ticks = beat_times(0.0, 3000.0, 1234.0, 500.0);
        assert_eq!(ticks.first().map(|t| t.0), Some(234.0));
        assert_eq!(ticks[0].1, -2, "extrapolated back from the first beat");
        assert!(ticks.iter().all(|t| t.0 >= 0.0 && t.0 <= 3000.0));
    }

    #[test]
    fn taps_set_the_tempo_after_four() {
        let mut h = home();
        h.page = Page::Timing;
        for k in 0..6 {
            h.tap(Some(1000.0 + k as f64 * 500.0), None);
        }
        assert!((h.bpm - 120.0).abs() < 0.05, "{}", h.bpm);
        assert!(h.status.contains("taps"));
    }

    #[test]
    fn creating_needs_a_song_and_names() {
        let mut h = home();
        assert!(matches!(h.click(Act::Create, 0.0, None), Out::Toast(_)));
        h.audio = Some(PathBuf::from("/definitely/not/here.mp3"));
        h.page = Page::Timing;
        let out = h.click(Act::Create, 0.0, None);
        assert!(
            matches!(out, Out::Toast(_)),
            "no artist, and the file is gone"
        );
        assert_eq!(h.page, Page::Details, "sent back to fix it");
    }

    #[test]
    fn the_wheel_zooms_around_the_pointer_and_stays_inside_the_song() {
        let mut h = home();
        loaded(&mut h, 48_000 * 60, 48_000.0);
        h.page = Page::Timing;
        h.wave = [100.0, 50.0, 800.0, 100.0];
        h.view_start = 10_000.0;
        h.view_len = 8000.0;
        let at = h.view_start + 0.5 * h.view_len;
        h.wheel(1, 500.0, true);
        assert!(h.view_len < 8000.0);
        let after = h.view_start + 0.5 * h.view_len;
        assert!(
            (after - at).abs() < 1.0,
            "the point under the pointer stays put"
        );
        for _ in 0..60 {
            h.wheel(-1, 100.0, true);
        }
        assert!(h.view_len <= 60_000.0 && h.view_start >= 0.0);
        for _ in 0..80 {
            h.wheel(1, 900.0, true);
        }
        assert_eq!(h.view_len, 1500.0, "no closer than that");
    }

    #[test]
    fn the_wheel_and_a_drag_move_along_the_song_and_a_click_moves_the_playhead() {
        let mut h = home();
        loaded(&mut h, 48_000 * 60, 48_000.0);
        h.page = Page::Timing;
        h.wave = [100.0, 50.0, 800.0, 100.0];
        h.view_start = 10_000.0;
        h.view_len = 8000.0;
        // Down the wheel: later in the song, same zoom.
        h.wheel(-1, 500.0, false);
        assert_eq!(h.view_len, 8000.0);
        assert!(h.view_start > 10_000.0);
        // Up, many times: back to the start and no further.
        for _ in 0..40 {
            h.wheel(1, 500.0, false);
        }
        assert_eq!(h.view_start, 0.0);
        // Dragging the waveform left moves the view later by as much.
        h.click(Act::Wave, 500.0, None);
        h.mouse_move(400.0);
        h.release(400.0, None);
        assert!((h.view_start - 1000.0).abs() < 1.0, "{}", h.view_start);
        assert_eq!(h.playhead, 0.0, "a drag doesn't move the playhead");
        // A click without moving does.
        h.click(Act::Wave, 500.0, None);
        h.release(500.0, None);
        assert!((h.playhead - 5000.0).abs() < 1.0, "{}", h.playhead);
        // The overview is the whole song: the middle of it is half a minute in.
        h.overview = [100.0, 160.0, 800.0, 15.0];
        h.click(Act::Overview, 500.0, None);
        h.mouse_move(500.0);
        assert!(h.dragging());
        h.release(500.0, None);
        assert!((h.playhead - 30_000.0).abs() < 50.0, "{}", h.playhead);
        assert!(h.view_start <= 30_000.0 && h.view_start + h.view_len >= 30_000.0);
        assert!(!h.dragging());
    }

    #[test]
    fn a_view_moved_by_hand_stays_while_the_song_plays() {
        let mut h = home();
        loaded(&mut h, 48_000 * 60, 48_000.0);
        h.page = Page::Timing;
        h.wave = [100.0, 50.0, 800.0, 100.0];
        h.playing = Some(Playing {
            from: 1000.0,
            at: Instant::now(),
            baked_to: f64::MAX,
        });
        h.view_start = 40_000.0;
        h.follow = true;
        h.follow_playback(Instant::now(), None);
        assert!(h.view_start < 2000.0, "it follows the playhead");
        h.wheel(-3, 500.0, false);
        let moved = h.view_start;
        h.follow_playback(Instant::now(), None);
        assert_eq!(h.view_start, moved, "until moved by hand");
    }

    #[test]
    fn leaving_goes_back_a_page_at_a_time() {
        let mut h = home();
        h.page = Page::Timing;
        assert!(matches!(h.click(Act::Back, 0.0, None), Out::None));
        assert_eq!(h.page, Page::Details);
        h.click(Act::Back, 0.0, None);
        assert_eq!(h.page, Page::Start);
        assert!(matches!(h.click(Act::Back, 0.0, None), Out::Leave));
    }
}
