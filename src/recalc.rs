//! Works out the pp and star rating of plays on the server again, from a copy of its
//! scores.jsonl and the `.osu` files found under some folders. The server takes the answer as
//! DIR/recalculated.json (it can't do this itself: it has neither the maps nor the calculator).
//!   taiko-rs --recalc-pp scores.jsonl recalculated.json FOLDER...

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mods::Mods;
use crate::online::OnlineScore;

/// A line of scores.jsonl.
#[derive(Deserialize)]
struct Line {
    id: u64,
    #[serde(default)]
    hidden: bool,
    score: OnlineScore,
}

/// What a play comes to now. Written as the server reads it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Recalculated {
    pub pp: f64,
    pub stars: Option<f64>,
}

/// The `.osu` files under `dirs`.
fn osu_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut stack = dirs.to_vec();
    let mut out = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu")) {
                out.push(path);
            }
        }
    }
    out
}

/// "Artist - Title [Version]", as the game names a map to the server.
fn label(text: &str) -> Option<String> {
    let field = |name: &str| text.lines().find_map(|l| l.strip_prefix(name)).map(str::trim);
    Some(format!("{} - {} [{}]", field("Artist:")?, field("Title:")?, field("Version:")?))
}

/// The maps by their hash, and by name for plays of a version since updated.
fn index(files: Vec<PathBuf>) -> (HashMap<u64, PathBuf>, HashMap<String, PathBuf>) {
    let (mut by_hash, mut by_label) = (HashMap::new(), HashMap::new());
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        by_hash.insert(crate::net::map_hash(&bytes), path.clone());
        if let Some(l) = label(&String::from_utf8_lossy(&bytes)) {
            by_label.insert(l, path);
        }
    }
    (by_hash, by_label)
}

/// The play on the map at `path` as it counts now. Random's colouring isn't in scores.jsonl: the
/// one from before seeds stands in for it.
fn recalculate(s: &OnlineScore, path: &Path) -> Option<Recalculated> {
    let mods = Mods::from_list(&s.mods);
    let scroll_speed = s.scroll_speed.unwrap_or(1.0);
    let half = crate::mods::HalfRule::of_pp_version(s.pp_version);
    let play = crate::pp::Play { great: s.great, ok: s.ok, miss: s.miss, max_combo: s.max_combo, rate: s.rate, mods, scroll_speed, seed: 0, half };
    let pp = crate::pp::play_pp(path, &play)?;
    Some(Recalculated { pp, stars: crate::stars::star_rating_played(path, s.rate, mods, scroll_speed, 0, half) })
}

/// Runs `--recalc-pp`.
pub fn run(args: &[String]) -> Result<(), String> {
    let [scores, out, dirs @ ..] = args else { return Err("usage: taiko-rs --recalc-pp scores.jsonl recalculated.json FOLDER...".into()) };
    let text = std::fs::read_to_string(scores).map_err(|e| format!("{scores}: {e}"))?;
    let (by_hash, by_label) = index(osu_files(&dirs.iter().map(PathBuf::from).collect::<Vec<_>>()));
    println!("{} maps found", by_hash.len());
    let mut done = BTreeMap::new();
    let mut missing = BTreeSet::new();
    let mut unreadable = 0;
    let mut by_name = 0;
    let mut changes = Vec::new();
    for line in text.lines().filter_map(|l| serde_json::from_str::<Line>(l).ok()) {
        let s = &line.score;
        // Plays that never had pp (a start point, Autoplay) and Claude's showpiece stay as they are.
        let Some(sent) = s.pp.filter(|_| !line.hidden) else { continue };
        let exact = by_hash.get(&s.map_hash);
        by_name += usize::from(exact.is_none() && by_label.contains_key(&s.map));
        let Some(path) = exact.or_else(|| by_label.get(&s.map)) else {
            missing.insert(s.set_id);
            continue;
        };
        let Some(new) = recalculate(s, path) else {
            unreadable += 1;
            continue;
        };
        if (new.pp - sent).abs() >= 1.0 {
            changes.push((new.pp - sent, line.id, s.player.clone(), s.mods.join(","), sent, new.pp));
        }
        done.insert(line.id, new);
    }
    let json = serde_json::to_string_pretty(&done).map_err(|e| e.to_string())?;
    std::fs::write(out, json).map_err(|e| format!("{out}: {e}"))?;
    changes.sort_by(|a, b| a.0.abs().total_cmp(&b.0.abs()).reverse());
    for (_, id, player, mods, sent, new) in changes.iter().take(30) {
        println!("{id:>6} {player:<17} {mods:<12} {sent:>8.0} -> {new:>8.0}");
    }
    println!("{} plays worked out ({by_name} of an updated map, by name), {} changed by 1pp or more, {unreadable} couldn't be", done.len(), changes.len());
    let sets: Vec<String> = missing.iter().map(|s| s.map_or("no set".to_string(), |s| s.to_string())).collect();
    println!("sets missing: {}", sets.join(" "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_are_worked_out_on_their_map_and_name() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-recalc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("set")).unwrap();
        let mut text = String::from(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Metadata]\nArtist:A\nTitle:T\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n[HitObjects]\n",
        );
        for k in 0..400 {
            text += &format!("256,192,{},1,{},0:0:0:0:\n", 1000 + k * 150, if k % 3 == 0 { 2 } else { 0 });
        }
        let path = dir.join("set/map.osu");
        std::fs::write(&path, &text).unwrap();
        let (by_hash, by_label) = index(osu_files(&[dir.clone()]));
        assert_eq!(by_hash.get(&crate::net::map_hash(text.as_bytes())), Some(&path));
        assert_eq!(by_label.get("A - T [V]"), Some(&path));

        let line = r#"{"id":5,"player":"a","score":{"player":"A","map":"A - T [V]","map_hash":1,"score":1,"accuracy":1.0,"max_combo":200,"miss":0,"rate":1.0,"mods":["HA"],"pp":900.0,"date":1,"great":200,"ok":0}}"#;
        let line: Line = serde_json::from_str(line).unwrap();
        let half = recalculate(&line.score, &path).unwrap();
        let full = crate::stars::star_rating(&path, 1.0).unwrap();
        assert!(half.stars.unwrap() < full && half.pp < 900.0, "{half:?}, {full}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
