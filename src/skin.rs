//! osu! skins: the playfield's and interface's images, the hitsounds and the combo break sound
//! of a skin from osu!'s `Skins` folder, or from an `.osk` dropped into taiko.rs's own one. Each
//! part can come from a different skin (`Sources`); anything a skin doesn't have stays built-in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

use crate::audio;
use crate::render::{Atlas, SPRITES, SkinLayout, Sprite};

/// taiko.rs's own Skins folder, next to its Songs folder. `.osk` files dropped here are
/// extracted into a folder of the same name.
pub fn own_dir() -> Option<PathBuf> {
    crate::config::own_dir("Skins")
}

/// Folders holding skins: osu!'s next to each Songs folder it has, then taiko.rs's own.
fn skin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = crate::library::song_dirs()
        .iter()
        .filter_map(|s| Some(s.parent()?.join("Skins")))
        .collect();
    dirs.extend(own_dir());
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

/// Installed skins as (name, folder), sorted by name. New `.osk` files are extracted first.
pub fn list() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let own = own_dir();
    for dir in skin_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            // osu! imports the `.osk` files in its own folder itself.
            let p = if Some(&dir) == own.as_ref()
                && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("osk"))
            {
                match extract_osk(&p) {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("{}: {e}", p.display());
                        continue;
                    }
                }
            } else {
                p
            };
            let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            // The first folder wins when osu! and taiko.rs both have a skin of that name.
            if p.is_dir() && !out.iter().any(|(n, _)| *n == name) {
                out.push((name, p));
            }
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// Extracts an `.osk` once into a folder beside it and returns the folder.
fn extract_osk(path: &Path) -> Result<PathBuf, String> {
    let dest = path.with_extension("");
    if dest.is_dir() {
        return Ok(dest);
    }
    let tmp = crate::download::partial_of(&dest);
    crate::download::unpack(path, &tmp)?;
    // Some archives wrap the skin in a folder of its own.
    let entries: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .collect();
    let root = match entries.as_slice() {
        [only] if only.is_dir() => only.clone(),
        _ => tmp.clone(),
    };
    std::fs::rename(&root, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(dest)
}

/// A skin's files by lowercase path (`fonts/score-0.png`): skins are made on Windows, where
/// names ignore case. Subfolders are read too, for fonts kept in one (skin.ini's prefixes).
struct Files(HashMap<String, PathBuf>);

impl Files {
    fn read(dir: &Path) -> Files {
        fn walk(dir: &Path, rel: &str, depth: u32, out: &mut HashMap<String, PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let Some(name) = e.file_name().to_str().map(str::to_lowercase) else {
                    continue;
                };
                let key = if rel.is_empty() {
                    name
                } else {
                    format!("{rel}/{name}")
                };
                let path = e.path();
                if path.is_dir() {
                    if depth < 2 {
                        walk(&path, &key, depth + 1, out);
                    }
                } else {
                    out.insert(key, path);
                }
            }
        }
        let mut map = HashMap::new();
        walk(dir, "", 0, &mut map);
        Files(map)
    }

    /// The first of `stems` present with an audio extension osu! accepts.
    fn sound(&self, stems: &[&str]) -> Option<&Path> {
        stems
            .iter()
            .flat_map(|s| ["wav", "ogg", "mp3"].map(|ext| format!("{s}.{ext}")))
            .find_map(|n| self.0.get(&n))
            .map(PathBuf::as_path)
    }

    /// The first of `stems` present as a PNG, preferring the `@2x` version. True when it is one.
    fn image<S: AsRef<str>>(&self, stems: &[S]) -> Option<(&Path, bool)> {
        stems.iter().find_map(|s| {
            let s = s.as_ref();
            let hd = self
                .0
                .get(&format!("{s}@2x.png"))
                .map(|p| (p.as_path(), true));
            hd.or_else(|| {
                self.0
                    .get(&format!("{s}.png"))
                    .map(|p| (p.as_path(), false))
            })
        })
    }
}

/// What taiko.rs reads from a skin's skin.ini.
struct Ini {
    /// Where the score and combo fonts' images are (`score` for `score-0.png`...), lowercase,
    /// with `/` between folders.
    score_prefix: String,
    combo_prefix: String,
    /// How far (SD px) each digit overlaps the one before.
    score_overlap: f32,
    combo_overlap: f32,
    /// Skins older than version 2.1 lay the drum out differently.
    legacy_drum: bool,
}

impl Ini {
    fn read(files: &Files) -> Ini {
        let mut ini = Ini {
            score_prefix: "score".into(),
            combo_prefix: "score".into(),
            score_overlap: 0.0,
            combo_overlap: 0.0,
            legacy_drum: false,
        };
        // No skin.ini: the latest version. One without a version: 1.0.
        let Some(text) = files.0.get("skin.ini").and_then(|p| std::fs::read(p).ok()) else {
            return ini;
        };
        ini.legacy_drum = true;
        let prefix = |v: &str| v.replace('\\', "/").trim_matches('/').to_lowercase();
        for line in String::from_utf8_lossy(&text).lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let v = v.split("//").next().unwrap_or("").trim();
            match k.trim().to_lowercase().as_str() {
                "version" => {
                    ini.legacy_drum =
                        !v.eq_ignore_ascii_case("latest") && v.parse::<f32>().is_ok_and(|n| n < 2.1)
                }
                "scoreprefix" if !v.is_empty() => ini.score_prefix = prefix(v),
                "comboprefix" if !v.is_empty() => ini.combo_prefix = prefix(v),
                "scoreoverlap" => ini.score_overlap = v.parse().unwrap_or(0.0),
                "combooverlap" => ini.combo_overlap = v.parse().unwrap_or(0.0),
                _ => {}
            }
        }
        ini
    }
}

/// The parts of the playfield and interface a skin can draw. Each comes from a skin of its own
/// choosing (`Sources`), so skins can be mixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Notes,
    Rolls,
    Swells,
    HitTarget,
    Lane,
    Drum,
    Judgements,
    Score,
    Combo,
    Health,
    Hitsounds,
    /// Overtime's scare: its picture (or animation) and its scream.
    Overtime,
}

pub const PARTS: usize = 12;

impl Part {
    pub const ALL: [Part; PARTS] = [
        Part::Notes,
        Part::Rolls,
        Part::Swells,
        Part::HitTarget,
        Part::Lane,
        Part::Drum,
        Part::Judgements,
        Part::Score,
        Part::Combo,
        Part::Health,
        Part::Hitsounds,
        Part::Overtime,
    ];

    /// Its name in the config.
    pub fn key(self) -> &'static str {
        match self {
            Part::Notes => "notes",
            Part::Rolls => "rolls",
            Part::Swells => "swells",
            Part::HitTarget => "hit-target",
            Part::Lane => "lane",
            Part::Drum => "drum",
            Part::Judgements => "judgements",
            Part::Score => "score",
            Part::Combo => "combo",
            Part::Health => "health",
            Part::Hitsounds => "hitsounds",
            Part::Overtime => "overtime",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::Notes => "Notes",
            Part::Rolls => "Drumrolls",
            Part::Swells => "Swells",
            Part::HitTarget => "Hit target",
            Part::Lane => "Lane and barlines",
            Part::Drum => "Drum",
            Part::Judgements => "Judgements",
            Part::Score => "Score and accuracy",
            Part::Combo => "Combo",
            Part::Health => "Health bar",
            Part::Hitsounds => "Hitsounds",
            Part::Overtime => "Overtime scare",
        }
    }

    /// Where it comes from unless the player says otherwise: the notes and their sounds from the
    /// skin picked, the interface built-in.
    pub fn default_source(self) -> Source {
        match self {
            // A skin made for it brings its own scare.
            Part::Notes | Part::Rolls | Part::Swells | Part::Hitsounds | Part::Overtime => {
                Source::Main
            }
            _ => Source::BuiltIn,
        }
    }

    /// The images that make it up.
    fn sprites(self) -> &'static [Sprite] {
        use Sprite::*;
        match self {
            Part::Notes => &[HitCircle, HitCircleOverlay, BigCircle, BigCircleOverlay],
            Part::Rolls => &[RollMiddle, RollEnd, RollTick],
            Part::Swells => &[SpinnerWarning, SpinnerCircle, SpinnerApproach],
            Part::HitTarget => &[HitTarget, HitTargetBig],
            Part::Lane => &[LaneBar, LaneKiai, Barline],
            Part::Drum => &[
                DrumPanel,
                DrumInner,
                DrumInnerFlipped,
                DrumOuter,
                DrumOuterFlipped,
            ],
            Part::Judgements => &[JudgeGreat, JudgeOk, JudgeMiss],
            Part::Score => &[
                Score0,
                Score1,
                Score2,
                Score3,
                Score4,
                Score5,
                Score6,
                Score7,
                Score8,
                Score9,
                ScorePercent,
                ScoreDot,
            ],
            Part::Combo => &[
                Combo0, Combo1, Combo2, Combo3, Combo4, Combo5, Combo6, Combo7, Combo8, Combo9,
            ],
            Part::Health => &[HealthBg, HealthFill],
            Part::Hitsounds => &[],
            Part::Overtime => &[
                Overtime0, Overtime1, Overtime2, Overtime3, Overtime4, Overtime5, Overtime6,
                Overtime7,
            ],
        }
    }
}

/// Which skin a part comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// The skin picked in Settings -> Skin.
    Main,
    /// taiko.rs's own look.
    BuiltIn,
    Skin(String),
}

impl Source {
    /// In the config. Folder names can't have a `<` in them on Windows.
    const BUILT_IN: &str = "<built-in>";
    const MAIN: &str = "<main>";

    /// The skin's name: empty for the built-in look.
    pub fn resolve<'a>(&'a self, main: &'a str) -> &'a str {
        match self {
            Source::Main => main,
            Source::BuiltIn => "",
            Source::Skin(name) => name,
        }
    }
}

/// Where each part comes from (`Part::ALL` order).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sources(pub [Source; PARTS]);

impl Default for Sources {
    fn default() -> Sources {
        Sources(Part::ALL.map(Part::default_source))
    }
}

impl Sources {
    pub fn get(&self, part: Part) -> &Source {
        &self.0[part as usize]
    }

    pub fn set(&mut self, part: Part, source: Source) {
        self.0[part as usize] = source;
    }

    /// Config values (`notes = Some Skin`), for the parts not where they'd be by default.
    pub fn to_config(&self) -> Vec<String> {
        Part::ALL
            .iter()
            .filter(|&&p| *self.get(p) != p.default_source())
            .map(|&p| {
                let name = match self.get(p) {
                    Source::Main => Source::MAIN,
                    Source::BuiltIn => Source::BUILT_IN,
                    Source::Skin(name) => name,
                };
                format!("{} = {name}", p.key())
            })
            .collect()
    }

    /// Takes in one config value written by `to_config`; unknown parts are ignored.
    pub fn parse(&mut self, v: &str) {
        let Some((k, name)) = v.split_once('=') else {
            return;
        };
        let Some(&part) = Part::ALL.iter().find(|p| p.key() == k.trim()) else {
            return;
        };
        let name = name.trim();
        self.set(
            part,
            match name {
                Source::BUILT_IN => Source::BuiltIn,
                Source::MAIN | "" => Source::Main,
                _ => Source::Skin(name.to_string()),
            },
        );
    }
}

/// The names `sprite`'s image can have in a skin with this skin.ini, best first.
fn image_names(sprite: Sprite, ini: &Ini) -> Vec<String> {
    use Sprite::*;
    let fixed: &[&str] = match sprite {
        HitCircle => &["taikohitcircle"],
        HitCircleOverlay => &["taikohitcircleoverlay", "taikohitcircleoverlay-0"],
        BigCircle => &["taikobigcircle"],
        BigCircleOverlay => &["taikobigcircleoverlay", "taikobigcircleoverlay-0"],
        RollMiddle => &["taiko-roll-middle"],
        RollEnd => &["taiko-roll-end"],
        RollTick => &["sliderscorepoint"],
        SpinnerWarning => &["spinner-warning"],
        SpinnerCircle => &["spinner-circle"],
        SpinnerApproach => &["spinner-approachcircle"],
        LaneBar => &["taiko-bar-right"],
        LaneKiai => &["taiko-bar-right-glow"],
        Barline => &["taiko-barline"],
        HitTarget => &["approachcircle"],
        HitTargetBig => &["taikobigcircle"],
        DrumPanel => &["taiko-bar-left"],
        DrumInner | DrumInnerFlipped => &["taiko-drum-inner"],
        DrumOuter | DrumOuterFlipped => &["taiko-drum-outer"],
        JudgeGreat => &["taiko-hit300", "taiko-hit300-0"],
        JudgeOk => &["taiko-hit100", "taiko-hit100-0"],
        JudgeMiss => &["taiko-hit0", "taiko-hit0-0"],
        HealthBg => &["scorebar-bg"],
        HealthFill => &["scorebar-colour", "scorebar-colour-0"],
        _ => &[],
    };
    if !fixed.is_empty() {
        return fixed.iter().map(|s| s.to_string()).collect();
    }
    let i = sprite as usize;
    if i >= Overtime0 as usize {
        // Frames come from `overtime_frames`; the names here say a map folder has them.
        let n = i - Overtime0 as usize;
        return if n == 0 {
            vec!["overtime-0".into(), "overtime".into()]
        } else {
            vec![format!("overtime-{n}")]
        };
    }
    let glyph = if (Score0 as usize..=Score9 as usize).contains(&i) {
        format!("{}-{}", ini.score_prefix, i - Score0 as usize)
    } else if (Combo0 as usize..=Combo9 as usize).contains(&i) {
        format!("{}-{}", ini.combo_prefix, i - Combo0 as usize)
    } else if sprite == ScorePercent {
        format!("{}-percent", ini.score_prefix)
    } else {
        format!("{}-dot", ini.score_prefix)
    };
    vec![glyph]
}

/// A skin's files and settings, read once however many parts it's used for.
struct Loaded {
    files: Files,
    ini: Ini,
}

impl Loaded {
    fn read(dir: &Path) -> Loaded {
        let files = Files::read(dir);
        let ini = Ini::read(&files);
        Loaded { files, ini }
    }
}

/// `img` premultiplied (so filtering doesn't darken its edges) and no bigger than `max` px on its
/// longest side, with its size in SD px over 128 (osu! draws a 128 px circle image at the note's
/// size; `hd`: an `@2x` image, twice as big as it's drawn).
fn prepared(mut img: image::RgbaImage, hd: bool, max: u32) -> (image::RgbaImage, [f32; 2]) {
    let sd = if hd { 2.0 } else { 1.0 };
    let size = [
        img.width() as f32 / sd / 128.0,
        img.height() as f32 / sd / 128.0,
    ];
    for px in img.pixels_mut() {
        let a = px[3] as u32;
        for c in &mut px.0[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    let scale = (max as f32 / img.width().max(img.height()) as f32).min(1.0);
    if scale < 1.0 {
        let (w, h) = (
            ((img.width() as f32 * scale).round() as u32).max(1),
            ((img.height() as f32 * scale).round() as u32).max(1),
        );
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    (img, size)
}

/// How long each frame of an `overtime-N.png` animation shows (ms), osu!'s default of 60 fps
/// being too quick to see a scare's frames by.
const OVERTIME_PNG_FRAME_MS: f32 = 100.0;

/// A skin's Overtime scare: `overtime-0.png`, `overtime-1.png`... (an animation), else
/// `overtime.png`, else the frames of `overtime.gif`; at most `OVERTIME_FRAMES`, spread over the
/// whole animation when it has more. Each with whether it's `@2x`, and how long each frame shows
/// (ms).
fn overtime_frames(files: &Files) -> (Vec<(image::RgbaImage, bool)>, f32) {
    let open = |(path, hd): (&Path, bool)| match image::open(path) {
        Ok(i) => Some((i.to_rgba8(), hd)),
        Err(e) => {
            eprintln!("skin: {}: {e}", path.display());
            None
        }
    };
    let numbered: Vec<(&Path, bool)> = (0..)
        .map_while(|i| files.image(&[format!("overtime-{i}")]))
        .take(256)
        .collect();
    if !numbered.is_empty() {
        let picked = spread(numbered.len(), crate::render::OVERTIME_FRAMES);
        let ms = OVERTIME_PNG_FRAME_MS * numbered.len() as f32 / picked.len() as f32;
        return (
            picked.iter().filter_map(|&i| open(numbered[i])).collect(),
            ms,
        );
    }
    if let Some(one) = files.image(&["overtime"]).and_then(open) {
        return (vec![one], 0.0);
    }
    let Some(path) = files.0.get("overtime.gif") else {
        return (Vec::new(), 0.0);
    };
    match gif_frames(path) {
        Ok((frames, total_ms)) if !frames.is_empty() => {
            let picked = spread(frames.len(), crate::render::OVERTIME_FRAMES);
            let ms = total_ms / picked.len() as f32;
            let mut frames: Vec<Option<image::RgbaImage>> = frames.into_iter().map(Some).collect();
            let out = picked
                .iter()
                .filter_map(|&i| frames[i].take().map(|f| (f, false)))
                .collect();
            (out, ms)
        }
        Ok(_) => (Vec::new(), 0.0),
        Err(e) => {
            eprintln!("skin: {}: {e}", path.display());
            (Vec::new(), 0.0)
        }
    }
}

/// `want` indices (or all `n` if fewer) spread evenly over 0..`n`.
fn spread(n: usize, want: usize) -> Vec<usize> {
    let k = n.min(want);
    (0..k).map(|i| i * n / k).collect()
}

/// A GIF's frames, whole (each drawn over the ones before as the GIF says), and its length (ms).
/// Frames past `MAX_GIF_FRAMES` are left out.
fn gif_frames(path: &Path) -> Result<(Vec<image::RgbaImage>, f32), String> {
    use image::AnimationDecoder;
    const MAX_GIF_FRAMES: usize = 256;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file))
        .map_err(|e| e.to_string())?;
    let (mut frames, mut total) = (Vec::new(), 0.0);
    for frame in decoder.into_frames().take(MAX_GIF_FRAMES) {
        let frame = frame.map_err(|e| e.to_string())?;
        let (num, den) = frame.delay().numer_denom_ms();
        // Browsers show a 0 or 10 ms frame for 100 ms.
        let ms = num as f32 / den.max(1) as f32;
        total += if ms <= 10.0 { 100.0 } else { ms };
        frames.push(frame.into_buffer());
    }
    Ok((frames, total))
}

/// The largest an image is kept (longest side, px): nothing is drawn bigger than that.
fn max_size(sprite: Sprite) -> u32 {
    use Sprite::*;
    match sprite {
        SpinnerCircle | SpinnerApproach => 512,
        LaneBar | LaneKiai | HealthFill => 1024,
        // Often as big as the screen (a frame with the bar in its corner): a row to itself.
        HealthBg => 2046,
        s if (Score0 as usize..=Combo9 as usize).contains(&(s as usize)) => 128,
        _ => 256,
    }
}

/// Packs each sprite's image from the skin it comes from (`from`; None = built-in) into one
/// atlas. Images are premultiplied (so filtering doesn't darken their edges) and capped in size.
/// Rows wrap at 2048 px, the smallest texture size every GPU takes.
fn atlas(from: &[Option<&Loaded>; SPRITES]) -> Atlas {
    const ROW: u32 = 2048;
    let mut images = Vec::new();
    for sprite in Sprite::ALL {
        let Some(skin) = from[sprite as usize] else {
            continue;
        };
        if Part::Overtime.sprites().contains(&sprite) {
            continue;
        }
        let Some((path, hd)) = skin.files.image(&image_names(sprite, &skin.ini)) else {
            continue;
        };
        let img = match image::open(path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("skin: {}: {e}", path.display());
                continue;
            }
        };
        let (mut img, size) = prepared(img, hd, max_size(sprite));
        // The drum's other half is the mirror image of the one the skin has.
        if matches!(sprite, Sprite::DrumInnerFlipped | Sprite::DrumOuterFlipped) {
            image::imageops::flip_horizontal_in_place(&mut img);
        }
        images.push((sprite, img, size));
    }
    let mut overtime_frame_ms = 0.0;
    if let Some(skin) = from[Sprite::Overtime0 as usize] {
        let (frames, frame_ms) = overtime_frames(&skin.files);
        overtime_frame_ms = frame_ms;
        // One picture gets more room than each frame of an animation.
        let max = if frames.len() > 1 { 384 } else { 640 };
        for (i, (img, hd)) in frames.into_iter().enumerate() {
            let (img, size) = prepared(img, hd, max);
            images.push((Sprite::ALL[Sprite::Overtime0 as usize + i], img, size));
        }
    }
    let has = |images: &[(Sprite, image::RgbaImage, [f32; 2])], s: Sprite| {
        images.iter().any(|(i, _, _)| *i == s)
    };
    // The circle is what makes a skinned note; overlays alone stay unused.
    if !has(&images, Sprite::HitCircle) && !has(&images, Sprite::BigCircle) {
        images
            .retain(|(i, _, _)| !matches!(i, Sprite::HitCircleOverlay | Sprite::BigCircleOverlay));
    }
    // A font missing a digit isn't used at all (built-in numbers rather than gaps).
    for zero in [Sprite::Score0, Sprite::Combo0] {
        let digits = zero as usize..zero as usize + 10;
        if !digits.clone().all(|i| has(&images, Sprite::ALL[i])) {
            images.retain(|(i, _, _)| {
                !digits.contains(&(*i as usize))
                    && !(zero == Sprite::Score0
                        && matches!(i, Sprite::ScorePercent | Sprite::ScoreDot))
            });
        }
    }
    let layout = |part: Part| from[part.sprites()[0] as usize].map(|s| &s.ini);
    let layout = SkinLayout {
        score_overlap: layout(Part::Score).map_or(0.0, |i| i.score_overlap),
        combo_overlap: layout(Part::Combo).map_or(0.0, |i| i.combo_overlap),
        legacy_drum: layout(Part::Drum).is_some_and(|i| i.legacy_drum),
        overtime_frames: Part::Overtime
            .sprites()
            .iter()
            .take_while(|&&s| has(&images, s))
            .count(),
        overtime_frame_ms,
    };
    if images.is_empty() {
        return Atlas {
            layout,
            ..Default::default()
        };
    }
    // Tallest first, so each row wastes little height.
    images.sort_by_key(|(_, img, _)| std::cmp::Reverse(img.height()));
    // Each image sits in a cell one pixel bigger all round, filled with copies of its edge, so
    // filtering at the edge (or across a stretched roll body) never picks up a neighbour.
    let mut cells = Vec::new();
    let (mut x, mut y, mut row_h, mut width) = (0, 0, 0, 0);
    for (_, img, _) in &images {
        let (cw, ch) = (img.width() + 2, img.height() + 2);
        if x > 0 && x + cw > ROW {
            (x, y, row_h) = (0, y + row_h, 0);
        }
        cells.push((x, y));
        (x, row_h, width) = (x + cw, row_h.max(ch), width.max(x + cw));
    }
    let height = y + row_h;
    let mut atlas = Atlas {
        width,
        height,
        rgba: vec![0; (width * height * 4) as usize],
        layout,
        ..Default::default()
    };
    for ((sprite, img, size), (cx, cy)) in images.into_iter().zip(cells) {
        let (iw, ih) = (img.width(), img.height());
        for py in 0..ih + 2 {
            for px in 0..iw + 2 {
                let p = img.get_pixel(
                    px.saturating_sub(1).min(iw - 1),
                    py.saturating_sub(1).min(ih - 1),
                );
                let at = (((cy + py) * width + cx + px) * 4) as usize;
                atlas.rgba[at..at + 4].copy_from_slice(&p.0);
            }
        }
        let uv = [
            (cx + 1) as f32 / width as f32,
            (cy + 1) as f32 / height as f32,
            iw as f32 / width as f32,
            ih as f32 / height as f32,
        ];
        atlas.sprites[sprite as usize] = Some((uv, size));
    }
    atlas
}

pub struct Skin {
    pub hitsounds: audio::Custom,
    pub images: Atlas,
    pub map_spinner: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Key {
    pub main: String,
    pub parts: Sources,
    pub rate: Option<u32>,
    pub map_dir: Option<PathBuf>,
}

pub struct Prepared {
    pub images: Arc<Atlas>,
    pub sounds: Option<Arc<audio::Sounds>>,
    pub map_spinner: bool,
}

pub struct Ready {
    pub key: Key,
    pub assets: Arc<Prepared>,
    revision: u64,
}

enum Work {
    Load(u64, Box<Key>, bool),
    Recycle(Arc<Prepared>),
}

/// One worker coalesces rapid selection changes; stale assets are discarded off the UI thread.
pub struct Loader {
    tx: mpsc::Sender<Work>,
    rx: mpsc::Receiver<Ready>,
    revision: Arc<AtomicU64>,
}

impl Loader {
    pub fn new() -> Self {
        let (tx, jobs) = mpsc::channel();
        let (done, rx) = mpsc::channel();
        let revision = Arc::new(AtomicU64::new(0));
        let wanted = revision.clone();
        std::thread::Builder::new()
            .name("skin assets".into())
            .spawn(move || {
                let mut cached: Option<(Key, Arc<Prepared>)> = None;
                while let Ok(work) = jobs.recv() {
                    let mut next = None;
                    let mut reload = false;
                    for work in std::iter::once(work).chain(jobs.try_iter()) {
                        match work {
                            Work::Load(id, key, fresh) => {
                                next = Some((id, *key));
                                reload |= fresh;
                            }
                            Work::Recycle(assets) => drop(assets),
                        }
                    }
                    let Some((id, mut key)) = next else { continue };
                    key.map_dir = key.map_dir.filter(|dir| map_has_images(dir));
                    let assets = match &cached {
                        Some((old, assets)) if !reload && *old == key => assets.clone(),
                        _ => {
                            let skin =
                                load(&key.main, &key.parts, key.rate, key.map_dir.as_deref());
                            let sounds = key
                                .rate
                                .map(|rate| Arc::new(audio::hitsound_set(rate, skin.hitsounds)));
                            let assets = Arc::new(Prepared {
                                images: Arc::new(skin.images),
                                sounds,
                                map_spinner: skin.map_spinner,
                            });
                            cached = Some((key.clone(), assets.clone()));
                            assets
                        }
                    };
                    if wanted.load(Ordering::Relaxed) == id
                        && done
                            .send(Ready {
                                key,
                                assets,
                                revision: id,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("skin asset worker");
        Self { tx, rx, revision }
    }

    pub fn request(&self, key: Key, reload: bool) {
        let id = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(Work::Load(id, Box::new(key), reload));
    }

    pub fn poll(&self) -> Option<Ready> {
        for ready in self.rx.try_iter() {
            if ready.revision == self.revision.load(Ordering::Relaxed) {
                return Some(ready);
            }
            let _ = self.tx.send(Work::Recycle(ready.assets));
        }
        None
    }
}

/// The skins `main` and `parts` use, read: by the part they're for (None = built-in).
fn read_skins(main: &str, parts: &Sources) -> (Vec<Loaded>, [Option<usize>; PARTS]) {
    let wanted: Vec<&str> = Part::ALL
        .iter()
        .map(|&p| parts.get(p).resolve(main))
        .collect();
    let installed = if wanted.iter().all(|n| n.is_empty()) {
        Vec::new()
    } else {
        list()
    };
    let mut loaded: Vec<(PathBuf, Loaded)> = Vec::new();
    let mut of_part = [None; PARTS];
    for (k, name) in wanted.into_iter().enumerate() {
        // A skin that isn't installed (any more) is the built-in look.
        let Some((_, dir)) = installed
            .iter()
            .find(|(n, _)| !name.is_empty() && n == name)
        else {
            continue;
        };
        let i = match loaded.iter().position(|(d, _)| d == dir) {
            Some(i) => i,
            None => {
                loaded.push((dir.clone(), Loaded::read(dir)));
                loaded.len() - 1
            }
        };
        of_part[k] = Some(i);
    }
    (loaded.into_iter().map(|(_, l)| l).collect(), of_part)
}

// ponytail: global, since there's one skin loaded at a time; a field on Skin if that changes.
static MAP_SPINNER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the skin last loaded took `spinner-warning` from the map's folder (a gimmick map's
/// beatmap skin): its swells are then drawn as small notes, over the notes on their ms.
pub fn map_has_spinner_warning() -> bool {
    MAP_SPINNER.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn apply_map_spinner_warning(present: bool) {
    MAP_SPINNER.store(present, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the map folder `dir` carries any skin images of its own (a beatmap skin).
pub fn map_has_images(dir: &Path) -> bool {
    let map = Loaded::read(dir);
    Sprite::ALL
        .iter()
        .any(|&s| map.files.image(&image_names(s, &map.ini)).is_some())
}

/// Loads the skin called `main` (built-in for "" or one that isn't installed), each part from
/// where `parts` says. `rate`: the audio device's sample rate, or None to skip the sounds.
/// `map_dir`: the playing map's folder, whose images win over the skin's like osu!'s beatmap
/// skins (gimmick maps hide the roll body / spinner with see-through ones).
pub fn load(main: &str, parts: &Sources, rate: Option<u32>, map_dir: Option<&Path>) -> Skin {
    let (skins, of_part) = read_skins(main, parts);
    let skin_of = |p: Part| of_part[p as usize].map(|i| &skins[i]);
    let map = map_dir.map(Loaded::read);
    let mut from = [None; SPRITES];
    for part in Part::ALL {
        for &s in part.sprites() {
            from[s as usize] = skin_of(part);
        }
    }
    if let Some(map) = &map {
        for s in Sprite::ALL {
            if map.files.image(&image_names(s, &map.ini)).is_some() {
                from[s as usize] = Some(map);
            }
        }
    }
    let map_spinner = map.as_ref().is_some_and(|m| {
        m.files
            .image(&image_names(Sprite::SpinnerWarning, &m.ini))
            .is_some()
    });
    let mut hitsounds = match (rate, skin_of(Part::Hitsounds)) {
        (Some(rate), Some(skin)) => hitsounds_of(&skin.files, rate),
        _ => audio::Custom::default(),
    };
    if let (Some(rate), Some(skin)) = (rate, skin_of(Part::Overtime)) {
        hitsounds.scream = scream_of(&skin.files, rate);
    }
    Skin {
        hitsounds,
        images: atlas(&from),
        map_spinner,
    }
}

/// Just the hitsounds of `load` (for the video export's audio).
pub fn hitsounds(main: &str, parts: &Sources, rate: u32) -> audio::Custom {
    let mut only = Sources(std::array::from_fn(|_| Source::BuiltIn));
    for part in [Part::Hitsounds, Part::Overtime] {
        only.set(part, parts.get(part).clone());
    }
    let (skins, of_part) = read_skins(main, &only);
    let mut out = of_part[Part::Hitsounds as usize].map_or_else(audio::Custom::default, |i| {
        hitsounds_of(&skins[i].files, rate)
    });
    out.scream = of_part[Part::Overtime as usize].and_then(|i| scream_of(&skins[i].files, rate));
    out
}

fn hitsounds_of(files: &Files, rate: u32) -> audio::Custom {
    audio::Custom {
        // osu!taiko plays the clap for rim hits; some skins only ship the whistle.
        don: files
            .sound(&["taiko-normal-hitnormal"])
            .and_then(|p| decode_sound(p, rate)),
        kat: files
            .sound(&["taiko-normal-hitclap", "taiko-normal-hitwhistle"])
            .and_then(|p| decode_sound(p, rate)),
        finish: files
            .sound(&["taiko-normal-hitfinish"])
            .and_then(|p| decode_sound(p, rate)),
        combobreak: files
            .sound(&["combobreak"])
            .and_then(|p| decode_sound(p, rate)),
        scream: None,
    }
}

/// A skin's Overtime scream (`overtime.wav` / `.ogg` / `.mp3`), cut at `MAX_SCREAM_S`.
fn scream_of(files: &Files, rate: u32) -> Option<Vec<f32>> {
    const MAX_SCREAM_S: u32 = 5;
    let mut s = files
        .sound(&["overtime"])
        .and_then(|p| decode_sound(p, rate))?;
    s.truncate((rate * MAX_SCREAM_S) as usize);
    Some(s)
}

/// A hitsound as mono samples. Skins mute a sound with a tiny empty file; one that fails to
/// decode keeps the built-in sound.
fn decode_sound(path: &Path, rate: u32) -> Option<Vec<f32>> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() < 128) {
        return Some(Vec::new());
    }
    match audio::decode(path, rate) {
        Ok(track) => Some(audio::mono(&track)),
        Err(e) => {
            eprintln!("skin: {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::{Loaded, Part, Source, Sources};
    use crate::render::{Atlas, SPRITES, Sprite};

    fn wait_for_skin(loader: &super::Loader) -> super::Ready {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(ready) = loader.poll() {
                return ready;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "skin worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn skin_worker_discards_stale_requests_reuses_plain_maps_and_reloads_images() {
        let dir = std::env::temp_dir().join(format!("taiko-loader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = super::Key {
            main: String::new(),
            parts: Sources::default(),
            rate: Some(48_000),
            map_dir: None,
        };
        let loader = super::Loader::new();
        loader.request(key.clone(), false);
        let initial = wait_for_skin(&loader);
        for i in 0..100 {
            loader.request(
                super::Key {
                    map_dir: Some(dir.join(i.to_string())),
                    ..key.clone()
                },
                false,
            );
        }
        let plain = wait_for_skin(&loader);
        assert_eq!(plain.revision, 101);
        assert!(plain.key.map_dir.is_none());
        assert!(std::sync::Arc::ptr_eq(&initial.assets, &plain.assets));
        let image = dir.join("taikohitcircle.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(&image)
            .unwrap();
        let custom = super::Key {
            map_dir: Some(dir.clone()),
            ..key
        };
        loader.request(custom.clone(), false);
        let first = wait_for_skin(&loader);
        assert!(first.assets.images.sprites[Sprite::HitCircle as usize].is_some());
        assert!(first.assets.sounds.is_some());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]))
            .save(image)
            .unwrap();
        loader.request(custom, true);
        let reloaded = wait_for_skin(&loader);
        assert!(!std::sync::Arc::ptr_eq(&first.assets, &reloaded.assets));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Every sprite from the one skin in `dir`.
    fn atlas_of(dir: &std::path::Path) -> Atlas {
        let skin = Loaded::read(dir);
        super::atlas(&[Some(&skin); SPRITES])
    }

    #[test]
    fn overtime_takes_a_skins_picture_animation_or_gif_and_its_scream() {
        use crate::render::OVERTIME_FRAMES;
        let dir = std::env::temp_dir().join(format!("taiko-rs-overtime-{}", std::process::id()));
        let fresh = || {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
        };
        let png = |name: &str, w: u32, h: u32| {
            image::RgbaImage::from_pixel(w, h, image::Rgba([200, 0, 0, 255]))
                .save(dir.join(name))
                .unwrap()
        };
        // No scare of its own: the built-in bear.
        fresh();
        png("taikohitcircle.png", 128, 128);
        assert_eq!(atlas_of(&dir).layout.overtime_frames, 0);
        // One picture, kept at its shape (and no bigger than 640 px).
        png("Overtime.png", 1600, 900);
        let a = atlas_of(&dir);
        assert_eq!(a.layout.overtime_frames, 1);
        let (uv, size) = a.sprites[Sprite::Overtime0 as usize].unwrap();
        assert!((size[0] / size[1] - 16.0 / 9.0).abs() < 0.01);
        assert!((uv[2] * a.width as f32 - 640.0).abs() < 1.0);
        // Numbered frames win over it: twelve of them, eight kept, each showing longer.
        for i in 0..12 {
            png(&format!("overtime-{i}.png"), 64, 64);
        }
        let a = atlas_of(&dir);
        assert_eq!(a.layout.overtime_frames, OVERTIME_FRAMES);
        assert_eq!(a.layout.overtime_frame_ms, 150.0);
        // A GIF: its frames and their timing.
        fresh();
        {
            use image::codecs::gif::GifEncoder;
            let file = std::fs::File::create(dir.join("overtime.gif")).unwrap();
            let mut gif = GifEncoder::new(file);
            let frames = (0..3u8).map(|i| {
                image::Frame::from_parts(
                    image::RgbaImage::from_pixel(40, 30, image::Rgba([i * 80, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(200, 1),
                )
            });
            gif.encode_frames(frames).unwrap();
        }
        let a = atlas_of(&dir);
        assert_eq!(a.layout.overtime_frames, 3);
        assert!((a.layout.overtime_frame_ms - 200.0).abs() < 1.0);
        // Its scream, cut at five seconds.
        let rate: u32 = 8000;
        let mut wav = Vec::new();
        {
            let samples = 6 * rate;
            let data = samples * 2;
            wav.extend(b"RIFF");
            wav.extend((36 + data).to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(rate.to_le_bytes());
            wav.extend((rate * 2).to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(data.to_le_bytes());
            for i in 0..samples {
                wav.extend((((i % 50) as i16 - 25) * 800).to_le_bytes());
            }
        }
        std::fs::write(dir.join("overtime.wav"), wav).unwrap();
        let files = Loaded::read(&dir).files;
        let scream = super::scream_of(&files, rate).expect("the skin's scream");
        assert_eq!(scream.len(), 5 * rate as usize);
        assert!(
            super::scream_of(
                &Loaded::read(&std::env::temp_dir().join("nope")).files,
                rate
            )
            .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_maps_own_images_skin_it_over_the_built_in_look() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-map-skin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!super::map_has_images(&dir));
        image::RgbaImage::new(2, 2)
            .save(dir.join("Taiko-Roll-End.png"))
            .unwrap();
        assert!(super::map_has_images(&dir));
        let built_in = Sources(std::array::from_fn(|_| Source::BuiltIn));
        let skin = super::load("", &built_in, None, Some(&dir));
        assert!(skin.images.sprites[Sprite::RollEnd as usize].is_some());
        assert!(skin.images.sprites[Sprite::RollMiddle as usize].is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn osk_wrapped_in_a_folder_extracts_flat_and_names_ignore_case() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-skin-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let osk = dir.join("Test.osk");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&osk).unwrap());
        zip.start_file(
            "Test/Taiko-Normal-HitNormal.WAV",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();

        let skin = super::extract_osk(&osk).unwrap();
        assert_eq!(skin, dir.join("Test"));
        assert!(
            super::Files::read(&skin)
                .sound(&["taiko-normal-hitnormal"])
                .is_some()
        );
        // Extracted once: a second call reuses the folder.
        assert_eq!(super::extract_osk(&osk).unwrap(), skin);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atlas_packs_rolls_and_spinners_with_their_edges_copied() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-atlas-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, w: u32, h: u32, px: [u8; 4]| {
            image::RgbaImage::from_pixel(w, h, image::Rgba(px))
                .save(dir.join(name))
                .unwrap()
        };
        // Overlays without a circle under them are left out.
        save("taikohitcircleoverlay.png", 128, 128, [255, 255, 255, 128]);
        save("spinner-warning.png", 128, 128, [255, 0, 0, 255]);
        let atlas = atlas_of(&dir);
        assert!(
            atlas.sprites[Sprite::HitCircleOverlay as usize].is_none()
                && atlas.sprites[Sprite::SpinnerWarning as usize].is_some()
        );

        // A 1 px wide roll body, and a full skin too wide for one 2048 px row.
        save("taiko-roll-middle.png", 1, 128, [255, 255, 255, 255]);
        save("Spinner-Circle@2x.png", 1024, 1024, [0, 255, 0, 255]);
        save("spinner-approachcircle.png", 600, 600, [0, 0, 255, 255]);
        for name in [
            "taikohitcircle",
            "taikobigcircle",
            "taikobigcircleoverlay",
            "taiko-roll-end",
        ] {
            save(&format!("{name}@2x.png"), 512, 512, [200, 200, 200, 255]);
        }
        let atlas = atlas_of(&dir);
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(
            atlas.width <= 2048 && atlas.height > 514,
            "{}x{}",
            atlas.width,
            atlas.height
        );
        // osu! sizes: 128 SD px is the radius it's drawn for, @2x counts half.
        assert_eq!(
            atlas.sprites[Sprite::SpinnerCircle as usize].unwrap().1,
            [4.0, 4.0]
        );
        assert_eq!(
            atlas.sprites[Sprite::RollMiddle as usize].unwrap().1,
            [1.0 / 128.0, 1.0]
        );
        assert!(
            Sprite::ALL[..9]
                .iter()
                .all(|&s| atlas.sprites[s as usize].is_some())
        );
        // Around each image, its edge is repeated: the roll body is white on both sides.
        let (uv, _) = atlas.sprites[Sprite::RollMiddle as usize].unwrap();
        let (x, y) = (
            (uv[0] * atlas.width as f32).round() as u32,
            (uv[1] * atlas.height as f32).round() as u32,
        );
        for px in [x - 1, x, x + 1] {
            let at = ((y + 64) * atlas.width + px) as usize * 4;
            assert_eq!(&atlas.rgba[at..at + 4], &[255, 255, 255, 255], "x = {px}");
        }
        // Every image is inside the atlas and none overlap.
        let rects: Vec<[f32; 4]> = atlas.sprites.iter().flatten().map(|(uv, _)| *uv).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a[0] >= 0.0 && a[1] >= 0.0 && a[0] + a[2] <= 1.0 && a[1] + a[3] <= 1.0);
            for b in &rects[i + 1..] {
                assert!(
                    a[0] + a[2] <= b[0]
                        || b[0] + b[2] <= a[0]
                        || a[1] + a[3] <= b[1]
                        || b[1] + b[3] <= a[1]
                );
            }
        }
    }

    #[test]
    fn fonts_come_from_skin_ini_prefixes_and_need_every_digit() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-font-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Fonts")).unwrap();
        let save = |name: &str| {
            image::RgbaImage::from_pixel(20, 30, image::Rgba([255, 255, 255, 255]))
                .save(dir.join(name))
                .unwrap()
        };
        std::fs::write(dir.join("skin.ini"), "\u{feff}[General]\nVersion: 2.0\n[Fonts]\nScorePrefix: Fonts\\Score\nScoreOverlap: 3 // comment\nComboPrefix: combo\n").unwrap();
        for d in 0..10 {
            save(&format!("Fonts/score-{d}.png"));
        }
        save("Fonts/score-percent.png");
        // The combo font misses its 9.
        for d in 0..9 {
            save(&format!("combo-{d}.png"));
        }
        save("taiko-drum-inner.png");
        let atlas = atlas_of(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            atlas.sprites[Sprite::Score7 as usize].is_some()
                && atlas.sprites[Sprite::ScorePercent as usize].is_some()
        );
        assert!(atlas.sprites[Sprite::ScoreDot as usize].is_none());
        assert!(atlas.sprites[Sprite::Combo0 as usize].is_none());
        assert_eq!(atlas.layout.score_overlap, 3.0);
        assert!(atlas.layout.legacy_drum);
        // The drum's right half is made from its left one.
        assert!(atlas.sprites[Sprite::DrumInnerFlipped as usize].is_some());
    }

    #[test]
    fn part_sources_round_trip_through_the_config() {
        // By default the notes follow the skin and the interface doesn't.
        let mut s = Sources::default();
        assert_eq!(
            (s.get(Part::Rolls), s.get(Part::Lane)),
            (&Source::Main, &Source::BuiltIn)
        );
        assert!(s.to_config().is_empty());
        s.set(Part::Lane, Source::Main);
        s.set(Part::Drum, Source::Skin("Some = Skin".into()));
        s.set(Part::Hitsounds, Source::BuiltIn);
        let lines = s.to_config();
        assert_eq!(
            lines,
            [
                "lane = <main>",
                "drum = Some = Skin",
                "hitsounds = <built-in>"
            ]
        );
        let mut back = Sources::default();
        for l in &lines {
            back.parse(l);
        }
        back.parse("not-a-part = x");
        assert_eq!(back, s);
        // Every sprite belongs to exactly one part.
        let mut owned = [0; SPRITES];
        for p in Part::ALL {
            for &sp in p.sprites() {
                owned[sp as usize] += 1;
            }
        }
        assert!(owned.iter().all(|&n| n == 1), "{owned:?}");
    }
}
