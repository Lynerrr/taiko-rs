//! A chart's hit pattern: the order of its dons and kats, big or small, and nothing else. It
//! tells a map made from a TJA chart (an `.osu` a converter wrote from one, say) though its
//! timing, offset, scroll and file were all redone, so the server can refuse to take one as a
//! custom map: TJA charts are played on the computer they're on, never shared.
//!
//! Rolls and balloons are left out: converters make sliders and spinners of them, and a short
//! slider comes back as hits. Changing a few notes gives another pattern; this stops a chart
//! sent as it is, not one made over.

use sha2::{Digest, Sha256};

use crate::beatmap::{Beatmap, Kind};

/// Fewer dons and kats than this: too short to tell from another chart.
pub const MIN_HITS: usize = 50;
/// Fewer changes between don and kat than this: too plain (a long run of dons) to tell apart.
pub const MIN_CHANGES: usize = 8;
const TAG: &[u8] = b"taiko.rs hit pattern v1\n";

/// `map`'s hit pattern, as lowercase hex; None when it's too short or plain to tell.
pub fn hit_pattern(map: &Beatmap) -> Option<String> {
    let hits: Vec<(bool, bool)> = map
        .notes
        .iter()
        .filter_map(|note| match note.kind {
            Kind::Don => Some((false, note.big)),
            Kind::Kat => Some((true, note.big)),
            Kind::Roll | Kind::Swell => None,
        })
        .collect();
    let changes = hits.windows(2).filter(|w| w[0].0 != w[1].0).count();
    if hits.len() < MIN_HITS || changes < MIN_CHANGES {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(TAG);
    for (kat, big) in hits {
        hash.update([u8::from(kat) | (u8::from(big) << 1)]);
    }
    Some(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 64 hits, eight a bar: a TJA line per bar.
    const PATTERN: &str = "1211221231421121221112423121221211212221121231141212112221213121";

    fn tja(pattern: &str) -> Beatmap {
        let bars: Vec<String> = pattern
            .as_bytes()
            .chunks(8)
            .map(|bar| format!("{},", String::from_utf8_lossy(bar)))
            .collect();
        let text = format!(
            "TITLE:Test\nBPM:120\nOFFSET:0\nCOURSE:Oni\nLEVEL:8\n\n#START\n{}\n#END\n",
            bars.join("\n")
        );
        crate::tja::parse(&text).unwrap().courses.remove(0).map
    }

    /// The same chart as a converter writes it: another offset and scroll, whole ms.
    fn osu(pattern: &str) -> Beatmap {
        let hits: String = pattern
            .bytes()
            .enumerate()
            .map(|(i, symbol)| {
                let sound = match symbol {
                    b'1' => 0,
                    b'2' => 2,
                    b'3' => 4,
                    _ => 6,
                };
                format!("256,192,{},1,{sound},0:0:0:0:\n", 37 + i * 250)
            })
            .collect();
        let text = format!(
            "osu file format v14\n\n[General]\nAudioFilename: a.ogg\nMode: 1\n\n[Difficulty]\nOverallDifficulty:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n37,500,4,1,0,100,1,0\n1037,-50,4,1,0,100,0,0\n\n[HitObjects]\n{hits}"
        );
        crate::beatmap::from_text(&text).unwrap()
    }

    #[test]
    fn a_chart_converted_to_osu_keeps_its_hit_pattern() {
        let pattern = hit_pattern(&tja(PATTERN)).unwrap();
        assert_eq!(pattern.len(), 64);
        assert_eq!(hit_pattern(&osu(PATTERN)), Some(pattern));
    }

    #[test]
    fn another_note_or_a_big_one_gives_another_pattern() {
        let pattern = hit_pattern(&tja(PATTERN));
        let mut kat = PATTERN.to_string();
        kat.replace_range(0..1, "2");
        let mut big = PATTERN.to_string();
        big.replace_range(0..1, "3");
        assert_ne!(hit_pattern(&tja(&kat)), pattern);
        assert_ne!(hit_pattern(&tja(&big)), pattern);
        assert_ne!(hit_pattern(&tja(&kat)), hit_pattern(&tja(&big)));
    }

    #[test]
    fn short_or_plain_charts_have_no_pattern() {
        assert_eq!(hit_pattern(&tja(&PATTERN[..48])), None);
        assert_eq!(hit_pattern(&tja(&"1".repeat(64))), None);
        assert_eq!(
            hit_pattern(&tja(&format!("{}{}", "1".repeat(32), "2".repeat(32)))),
            None
        );
    }
}
