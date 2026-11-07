//! The map editor: charts edited as `.osu` files that play exactly as they do in osu!stable.
//!
//! `state` is what the editor does to a chart (placing, selecting, moving, copying, timing lines).
//! `view` is the screen. `chart` holds the file and writes it back without touching what the editor doesn't model,
//! `timing` is the snap grid and stable's millisecond rounding, `history` is undo and redo.

pub mod chart;
pub mod clips;
pub mod fakes;
pub mod gimmicks;
pub mod history;
pub mod home;
pub mod keyframes;
pub mod motion;
pub mod new_map;
pub mod nodes;
pub mod setup;
pub mod snaps;
pub mod state;
pub mod sv_curves;
pub mod svt;
pub mod tempo;
pub mod timing;
pub mod tools;
pub mod view;
pub mod wave;

use std::path::{Path, PathBuf};

/// A new, empty difficulty next to `source`: the same timing, music, storyboard and settings, no
/// notes. Returns the new file. A name that's taken gets a number.
pub fn new_difficulty(source: &Path) -> Result<PathBuf, String> {
    let mut chart = chart::Chart::read(source)?;
    chart.objects.clear();
    // A .tko keeps its kind but not its animations: they were made for the notes that are gone.
    if chart.gimmick.is_some() {
        chart.gimmick = Some(Default::default());
    }
    chart.dirty = true;
    let version = chart
        .get_key("Metadata", "Version")
        .unwrap_or_else(|| "New".into());
    let name = |n: u32| {
        if n == 0 {
            "New difficulty".to_string()
        } else {
            format!("New difficulty {n}")
        }
    };
    let dir = source.parent().unwrap_or(Path::new("."));
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("map");
    // The file is named like the source with its [Version] swapped, the way osu! names them.
    let base = match stem.rfind(&format!("[{version}]")) {
        Some(at) => stem[..at].trim_end().to_string(),
        None => stem.to_string(),
    };
    for n in 0.. {
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("osu");
        let candidate = dir.join(format!("{base} [{}].{ext}", name(n)));
        if candidate.exists() {
            continue;
        }
        chart.set_key("Metadata", "Version", &name(n));
        // A new set of its own: the online ids belong to the source difficulty.
        chart.set_key("Metadata", "BeatmapID", "0");
        chart.save(&candidate, false)?;
        return Ok(candidate);
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_difficulty_of_a_tko_is_a_tko_without_the_animations() {
        use crate::tko::gimmick::FieldProp;
        let dir = std::env::temp_dir().join(format!("taiko-rs-newdiff-tko-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("Artist - Title (Mapper) [Oni].tko");
        let mut g = crate::tko::gimmick::Gimmick::default();
        g.key_field(
            FieldProp::Zoom,
            crate::tko::track::Key {
                t: 0.0,
                v: 2.0,
                ease: Default::default(),
            },
        );
        std::fs::write(
            &src,
            crate::tko::format::write(chart::tests::MAP, &g).unwrap(),
        )
        .unwrap();
        let made = new_difficulty(&src).unwrap();
        let chart = chart::Chart::read(&made).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(made.extension().unwrap(), "tko");
        assert!(chart.objects.is_empty());
        assert!(chart.gimmick.unwrap().is_empty());
    }

    #[test]
    fn a_new_difficulty_keeps_the_timing_and_drops_the_notes() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-newdiff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("Artist - Title (Mapper) [Oni].osu");
        std::fs::write(&src, chart::tests::MAP).unwrap();
        let a = new_difficulty(&src).unwrap();
        let b = new_difficulty(&src).unwrap();
        let (ca, cb) = (
            chart::Chart::read(&a).unwrap(),
            chart::Chart::read(&b).unwrap(),
        );
        let names = [
            a.file_name().unwrap().to_owned(),
            b.file_name().unwrap().to_owned(),
        ];
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ca.objects.is_empty() && ca.timing.len() == 2);
        assert_eq!(
            ca.get_key("Metadata", "Version").as_deref(),
            Some("New difficulty")
        );
        assert_eq!(
            cb.get_key("Metadata", "Version").as_deref(),
            Some("New difficulty 1")
        );
        assert_eq!(names[0], "Artist - Title (Mapper) [New difficulty].osu");
        assert_eq!(
            ca.get_key("General", "AudioFilename"),
            Some("audio.mp3".into())
        );
    }
}
