//! Making a map from a song: its folder in the Songs folder, the audio and background copied in, and
//! a `.osu` (or `.tko`) with the metadata and the first timing line, ready for the editor.

use std::path::{Path, PathBuf};

use crate::beatmap::OSU_TIMING_MS;

/// Everything the new map's file needs.
#[derive(Clone, Debug)]
pub struct NewMap {
    pub audio: PathBuf,
    pub background: Option<PathBuf>,
    pub artist: String,
    pub title: String,
    /// The artist and title in Latin letters, when the ones above aren't (empty: the same).
    pub artist_romanised: String,
    pub title_romanised: String,
    pub creator: String,
    pub version: String,
    pub source: String,
    pub tags: String,
    pub bpm: f64,
    /// The first red line (ms of the chart, as stable shows it).
    pub offset_ms: f64,
    pub per_bar: u32,
    /// For where the song select preview starts.
    pub length_ms: f64,
    /// A `.tko` (this client's effects on top) rather than a plain `.osu`.
    pub tko: bool,
}

impl NewMap {
    /// A chart's time for a beat heard `audio_ms` into the song: stable plays the music a little
    /// ahead of the notes (see `OSU_TIMING_MS`), and the editor does the same.
    pub fn chart_time(audio_ms: f64) -> f64 {
        audio_ms - OSU_TIMING_MS
    }

    /// What is wrong with it, in words for the person making it (empty: fine).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, value) in [
            ("Artist", &self.artist),
            ("Title", &self.title),
            ("Creator", &self.creator),
            ("Difficulty name", &self.version),
        ] {
            if value.trim().is_empty() {
                out.push(format!("{name} is empty"));
            }
        }
        if !(crate::editor::tempo::MIN_BPM / 2.0..=crate::editor::tempo::MAX_BPM * 2.0)
            .contains(&self.bpm)
        {
            out.push("the BPM doesn't make sense".into());
        }
        if !self.audio.is_file() {
            out.push("the song file is gone".into());
        }
        out
    }

    /// Whether a name written in other scripts has no Latin version given: osu! wants both.
    pub fn wants_romanised(&self) -> bool {
        (!self.artist.is_ascii() && self.artist_romanised.trim().is_empty())
            || (!self.title.is_ascii() && self.title_romanised.trim().is_empty())
    }
}

/// `text` as a file or folder name on any system.
pub fn sanitise(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if name.chars().count() > 100 {
        name = name
            .chars()
            .take(100)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    const RESERVED: [&str; 8] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "LPT1", "LPT2"];
    if name.is_empty() {
        name = "map".into();
    } else if RESERVED.iter().any(|r| name.eq_ignore_ascii_case(r)) {
        name.insert(0, '_');
    }
    name
}

/// An artist and a title guessed from a song's file name ("01 - Artist - Title.mp3",
/// "Artist - Title.ogg"): the title alone when there is no " - ".
pub fn guess_names(path: &Path) -> (String, String) {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().replace('_', " "))
        .unwrap_or_default();
    let mut text = stem.trim();
    // A track number in front: "01 - ", "01. ", "01 ".
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = text[digits..].trim_start_matches(['.', ')', '-', ' ']);
        if !rest.is_empty() && rest.len() < text.len() {
            text = rest;
        }
    }
    match text.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (artist.trim().to_string(), title.trim().to_string())
        }
        _ => (String::new(), text.trim().to_string()),
    }
}

/// A number in the file the way osu! writes it: no trailing zeros.
fn number(v: f64) -> String {
    let text = format!("{v:.12}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The `.osu` text of the new map.
pub fn osu_text(m: &NewMap, audio_name: &str, background_name: Option<&str>) -> String {
    let romanised = |plain: &str, romanised: &str| {
        if romanised.trim().is_empty() {
            plain.trim().to_string()
        } else {
            romanised.trim().to_string()
        }
    };
    let line = |s: &str| s.replace(['\r', '\n'], " ");
    let preview = (m.length_ms * 0.4).max(0.0).round() as i64;
    let mut t = String::new();
    let mut add = |s: String| {
        t.push_str(&s);
        t.push_str("\r\n");
    };
    add("osu file format v14".into());
    add(String::new());
    add("[General]".into());
    add(format!("AudioFilename: {audio_name}"));
    add("AudioLeadIn: 0".into());
    add(format!("PreviewTime: {preview}"));
    add("Countdown: 0".into());
    add("SampleSet: Normal".into());
    add("StackLeniency: 0.7".into());
    add("Mode: 1".into());
    add("LetterboxInBreaks: 0".into());
    add("WidescreenStoryboard: 0".into());
    add(String::new());
    add("[Editor]".into());
    add("DistanceSpacing: 1".into());
    add("BeatDivisor: 4".into());
    add("GridSize: 32".into());
    add("TimelineZoom: 1".into());
    add(String::new());
    add("[Metadata]".into());
    add(format!(
        "Title:{}",
        line(&romanised(&m.title, &m.title_romanised))
    ));
    add(format!("TitleUnicode:{}", line(m.title.trim())));
    add(format!(
        "Artist:{}",
        line(&romanised(&m.artist, &m.artist_romanised))
    ));
    add(format!("ArtistUnicode:{}", line(m.artist.trim())));
    add(format!("Creator:{}", line(m.creator.trim())));
    add(format!("Version:{}", line(m.version.trim())));
    add(format!("Source:{}", line(m.source.trim())));
    add(format!("Tags:{}", line(m.tags.trim())));
    add("BeatmapID:0".into());
    add("BeatmapSetID:-1".into());
    add(String::new());
    add("[Difficulty]".into());
    add("HPDrainRate:5".into());
    add("CircleSize:5".into());
    add("OverallDifficulty:5".into());
    add("ApproachRate:5".into());
    add("SliderMultiplier:1.4".into());
    add("SliderTickRate:1".into());
    add(String::new());
    add("[Events]".into());
    add("//Background and Video events".into());
    if let Some(bg) = background_name {
        add(format!("0,0,\"{bg}\",0,0"));
    }
    add("//Break Periods".into());
    add("//Storyboard Layer 0 (Background)".into());
    add("//Storyboard Layer 1 (Fail)".into());
    add("//Storyboard Layer 2 (Pass)".into());
    add("//Storyboard Layer 3 (Foreground)".into());
    add("//Storyboard Layer 4 (Overlay)".into());
    add("//Storyboard Sound Samples".into());
    add(String::new());
    add("[TimingPoints]".into());
    add(format!(
        "{},{},{},1,0,100,1,0",
        m.offset_ms.round() as i64,
        number(60_000.0 / m.bpm),
        m.per_bar.max(1)
    ));
    add(String::new());
    add("[HitObjects]".into());
    t
}

/// The folder for a new set: `Artist - Title` in `songs`, numbered when that is taken by another.
fn set_folder(songs: &Path, m: &NewMap) -> PathBuf {
    let base = sanitise(&format!("{} - {}", m.artist.trim(), m.title.trim()));
    (0u32..)
        .map(|n| {
            if n == 0 {
                songs.join(&base)
            } else {
                songs.join(format!("{base} ({})", n + 1))
            }
        })
        .find(|dir| !dir.exists())
        .expect("a free folder name")
}

/// Makes the map: the folder, the song and background copied in, the file. Returns the file.
pub fn create(m: &NewMap, songs: &Path) -> Result<PathBuf, String> {
    let problems = m.problems();
    if !problems.is_empty() {
        return Err(problems.join(", "));
    }
    let dir = set_folder(songs, m);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let made = (|| {
        let ext = |p: &Path, fallback: &str| {
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
                .unwrap_or_else(|| fallback.to_string())
        };
        let audio_name = format!("audio.{}", ext(&m.audio, "mp3"));
        std::fs::copy(&m.audio, dir.join(&audio_name))
            .map_err(|e| format!("copying the song: {e}"))?;
        let background = match &m.background {
            Some(bg) => {
                let name = format!("bg.{}", ext(bg, "jpg"));
                std::fs::copy(bg, dir.join(&name))
                    .map_err(|e| format!("copying the background: {e}"))?;
                Some(name)
            }
            None => None,
        };
        let osu = osu_text(m, &audio_name, background.as_deref());
        let (text, extension) = if m.tko {
            let tko =
                crate::tko::format::write(&osu, &Default::default()).map_err(|e| e.to_string())?;
            (tko, "tko")
        } else {
            (osu, "osu")
        };
        let file = dir.join(format!(
            "{} [{}].{extension}",
            sanitise(&format!(
                "{} - {} ({})",
                m.artist.trim(),
                m.title.trim(),
                m.creator.trim()
            )),
            sanitise(&m.version)
        ));
        std::fs::write(&file, text).map_err(|e| format!("writing the map: {e}"))?;
        Ok(file)
    })();
    if made.is_err() {
        // Nothing half made is left in the player's songs.
        let _ = std::fs::remove_dir_all(&dir);
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path) -> NewMap {
        std::fs::write(dir.join("song.mp3"), b"not really audio").unwrap();
        std::fs::write(dir.join("pic.PNG"), b"not really a picture").unwrap();
        NewMap {
            audio: dir.join("song.mp3"),
            background: Some(dir.join("pic.PNG")),
            artist: "Some Artist".into(),
            title: "A: Song?".into(),
            artist_romanised: String::new(),
            title_romanised: String::new(),
            creator: "me".into(),
            version: "Oni".into(),
            source: String::new(),
            tags: "test map".into(),
            bpm: 174.97,
            offset_ms: 1211.0,
            per_bar: 4,
            length_ms: 100_000.0,
            tko: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-newmap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_made_safe_for_every_system() {
        assert_eq!(sanitise("A: Song?"), "A_ Song_");
        assert_eq!(sanitise("  dots...  "), "dots");
        assert_eq!(sanitise("con"), "_con");
        assert_eq!(sanitise("///"), "___");
        assert_eq!(sanitise(""), "map");
        assert_eq!(sanitise(&"x".repeat(300)).len(), 100);
        assert_eq!(sanitise("曲 - 名前"), "曲 - 名前");
    }

    #[test]
    fn artist_and_title_come_from_the_file_name() {
        let guess = |n: &str| guess_names(Path::new(n));
        assert_eq!(
            guess("Artist - Title.mp3"),
            ("Artist".into(), "Title".into())
        );
        assert_eq!(
            guess("01 - Artist - Title.mp3"),
            ("Artist".into(), "Title".into())
        );
        assert_eq!(
            guess("03. Artist - Title.ogg"),
            ("Artist".into(), "Title".into())
        );
        assert_eq!(
            guess("just_a_title.wav"),
            (String::new(), "just a title".into())
        );
        assert_eq!(guess("1999.mp3"), (String::new(), "1999".into()));
        assert_eq!(guess("a - b - c.mp3"), ("a".into(), "b - c".into()));
    }

    #[test]
    fn the_new_map_is_a_taiko_map_with_one_red_line_and_no_notes() {
        let dir = scratch("text");
        let m = sample(&dir);
        let text = osu_text(&m, "audio.mp3", Some("bg.png"));
        let chart = crate::editor::chart::Chart::parse(&text);
        assert!(chart.objects.is_empty());
        assert_eq!(chart.timing.len(), 1);
        assert!(chart.timing[0].is_red());
        assert_eq!(chart.timing[0].time, 1211.0);
        assert!((chart.timing[0].beat_length - 60_000.0 / 174.97).abs() < 1e-6);
        assert_eq!(chart.get_key("General", "Mode").as_deref(), Some("1"));
        assert_eq!(
            chart.get_key("General", "PreviewTime").as_deref(),
            Some("40000")
        );
        assert_eq!(
            chart.get_key("Metadata", "BeatmapSetID").as_deref(),
            Some("-1")
        );
        assert!(text.contains("0,0,\"bg.png\",0,0\r\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creating_copies_the_files_and_the_map_reads_back() {
        let dir = scratch("create");
        let songs = dir.join("Songs");
        std::fs::create_dir_all(&songs).unwrap();
        let m = sample(&dir);
        let file = create(&m, &songs).unwrap();
        let folder = file.parent().unwrap();
        assert_eq!(folder.file_name().unwrap(), "Some Artist - A_ Song_");
        assert!(folder.join("audio.mp3").is_file() && folder.join("bg.png").is_file());
        assert_eq!(
            file.file_name().unwrap(),
            "Some Artist - A_ Song_ (me) [Oni].osu"
        );
        let meta = crate::beatmap::read_meta_file(&file).unwrap();
        assert_eq!((meta.mode, meta.artist.as_str()), (1, "Some Artist"));
        assert_eq!(meta.title, "A: Song?", "the name inside is left as typed");
        assert_eq!(meta.version, "Oni");
        assert_eq!(meta.audio_file, "audio.mp3");
        assert_eq!(meta.background.as_deref(), Some("bg.png"));
        let chart = crate::editor::chart::Chart::read(&file).unwrap();
        assert_eq!(chart.timing.len(), 1);
        // The same song again gets a folder of its own.
        let again = create(&m, &songs).unwrap();
        assert_eq!(
            again.parent().unwrap().file_name().unwrap(),
            "Some Artist - A_ Song_ (2)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tko_is_a_tko_and_a_bad_map_leaves_nothing_behind() {
        let dir = scratch("tko");
        let songs = dir.join("Songs");
        std::fs::create_dir_all(&songs).unwrap();
        let mut m = sample(&dir);
        m.tko = true;
        m.background = None;
        let file = create(&m, &songs).unwrap();
        assert_eq!(file.extension().unwrap(), "tko");
        assert!(
            crate::editor::chart::Chart::read(&file)
                .unwrap()
                .gimmick
                .is_some()
        );
        // The song vanishes: refused, and no folder.
        m.artist = "Other".into();
        m.audio = dir.join("gone.mp3");
        assert!(create(&m, &songs).is_err());
        assert!(!songs.join("Other - A_ Song_").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_missing_is_named() {
        let dir = scratch("problems");
        let mut m = sample(&dir);
        assert!(m.problems().is_empty());
        m.artist = " ".into();
        m.bpm = 0.0;
        let said = m.problems().join("|");
        assert!(said.contains("Artist") && said.contains("BPM"));
        m.artist = "曲".into();
        assert!(m.wants_romanised());
        m.artist_romanised = "Kyoku".into();
        assert!(!m.wants_romanised());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_beat_heard_in_the_song_is_a_little_earlier_on_the_chart() {
        assert_eq!(NewMap::chart_time(1000.0), 1000.0 - OSU_TIMING_MS);
    }

    #[test]
    fn numbers_are_written_without_trailing_zeros() {
        assert_eq!(number(500.0), "500");
        assert_eq!(number(342.8571428571), "342.8571428571");
    }
}
