use super::*;

const SONG_H: f32 = 34.0;
const HEAD_H: f32 = 20.0;

/// A cheap deterministic generator, so the cases are the same every run.
fn next(seed: &mut u64) -> usize {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*seed >> 33) as usize
}

fn extra_of(extras: &[(usize, f32)], g: usize) -> f32 {
    extras
        .iter()
        .find(|(e, _)| *e == g)
        .map_or(0.0, |(_, e)| *e)
}

/// The list laid out row by row from the top, the way it was before it was made by
/// arithmetic: (section, song group, y) for every song of an open section, where each
/// section starts, and the total height.
fn walked(
    sections: &[Section],
    open: &[bool],
    headings: bool,
    extras: &[(usize, f32)],
) -> (Vec<(usize, usize, f32)>, Vec<f32>, f32) {
    let (mut y, mut songs, mut tops) = (0.0f32, Vec::new(), Vec::new());
    for (si, sec) in sections.iter().enumerate() {
        tops.push(y);
        if headings {
            y += HEAD_H;
        }
        if !open[si] {
            continue;
        }
        for g in sec.groups.clone() {
            songs.push((si, g, y));
            y += SONG_H + extra_of(extras, g);
        }
    }
    (songs, tops, y)
}

#[test]
fn the_arithmetic_layout_matches_walking_every_row() {
    let mut seed = 7;
    for case in 0..300 {
        let mut sections = Vec::new();
        let mut at = 0;
        for _ in 0..1 + next(&mut seed) % 6 {
            let n = 1 + next(&mut seed) % 40;
            sections.push(Section {
                label: String::new(),
                colour: None,
                groups: at..at + n,
            });
            at += n;
        }
        let open: Vec<bool> = sections
            .iter()
            .map(|_| !next(&mut seed).is_multiple_of(4))
            .collect();
        let headings = case % 2 == 0;
        let mut extras: Vec<(usize, f32)> = (0..next(&mut seed) % 3)
            .map(|_| (next(&mut seed) % at, 4.0 + (next(&mut seed) % 90) as f32))
            .collect();
        extras.sort_by_key(|(g, _)| *g);
        extras.dedup_by_key(|(g, _)| *g);

        let (songs, tops, total) = walked(&sections, &open, headings, &extras);
        let layout = ListLayout::new(&sections, &open, headings, (SONG_H, HEAD_H), extras.clone());
        assert_eq!(layout.tops, tops, "case {case}");
        assert!((layout.total - total).abs() < 0.01, "case {case}");
        for &(si, g, y) in &songs {
            assert!(
                (layout.song_y(si, g) - y).abs() < 0.01,
                "case {case} song {g}"
            );
        }
        // Any window of the list: exactly the songs of open sections that reach into it.
        for _ in 0..20 {
            let from = (next(&mut seed) % (total as usize + 50)) as f32 - 20.0;
            let to = from + (next(&mut seed) % 400) as f32;
            for si in (0..sections.len()).filter(|si| open[*si]) {
                let want: Vec<usize> = songs
                    .iter()
                    .filter(|(s, g, y)| {
                        *s == si && y + SONG_H + extra_of(&extras, *g) >= from && *y <= to
                    })
                    .map(|(_, g, _)| *g)
                    .collect();
                let got: Vec<usize> = layout.visible_songs(si, from, to).collect();
                assert_eq!(got, want, "case {case} section {si} window {from}..{to}");
            }
        }
    }
}
