//! The map editor: charts edited as `.osu` files that play exactly as they do in osu!stable.
//!
//! `state` is what the editor does to a chart (placing, selecting, moving, copying, timing lines).
//! `view` is the screen. `chart` holds the file and writes it back without touching what the editor doesn't model,
//! `timing` is the snap grid and stable's millisecond rounding, `history` is undo and redo.

// Until the editor scene is wired into the game.
#![allow(dead_code)]

pub mod chart;
pub mod history;
pub mod state;
pub mod timing;
pub mod view;

use std::path::{Path, PathBuf};

/// A new, empty difficulty next to `source`: the same timing, music, storyboard and settings, no
/// notes. Returns the new file. A name that's taken gets a number.
pub fn new_difficulty(source: &Path) -> Result<PathBuf, String> {
    let mut chart = chart::Chart::read(source)?;
    chart.objects.clear();
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
        let candidate = dir.join(format!("{base} [{}].osu", name(n)));
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
