use super::*;

/// A one-course chart at `bpm` with `body` between #START and #END.
fn chart(bpm: f64, body: &str) -> String {
    format!("TITLE:Test\nBPM:{bpm}\nOFFSET:0\nCOURSE:Oni\nLEVEL:8\n\n#START\n{body}\n#END\n")
}

fn one(text: &str) -> Course {
    let mut song = parse(text).expect("parses");
    assert_eq!(song.courses.len(), 1, "{:?}", song.warnings);
    song.courses.remove(0)
}

fn times(c: &Course) -> Vec<f64> {
    c.map.notes.iter().map(|n| n.time).collect()
}

fn close(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-6, "{a:?} vs {b:?}");
    }
}

#[test]
fn a_bar_splits_into_its_symbols() {
    let c = one(&chart(120.0, "1020,\n3400,"));
    close(&times(&c), &[0.0, 1000.0, 2000.0, 2500.0]);
    let kinds: Vec<(Kind, bool)> = c.map.notes.iter().map(|n| (n.kind, n.big)).collect();
    assert_eq!(
        kinds,
        [
            (Kind::Don, false),
            (Kind::Kat, false),
            (Kind::Don, true),
            (Kind::Kat, true)
        ]
    );
    let bars: Vec<f64> = c.map.barlines.iter().map(|b| b.time).collect();
    close(&bars, &[0.0, 2000.0]);
}

#[test]
fn offset_moves_the_first_bar() {
    // OFFSET is where the music is when the first bar starts, negated.
    let late = one(&chart(120.0, "1,").replace("OFFSET:0", "OFFSET:-1.5"));
    close(&times(&late), &[1500.0]);
    let early = one(&chart(120.0, "1,").replace("OFFSET:0", "OFFSET:2"));
    close(&times(&early), &[-2000.0]);
}

#[test]
fn a_bar_can_span_lines_and_an_empty_bar_still_lasts() {
    let c = one(&chart(120.0, "10\n10,\n,\n1,"));
    close(&times(&c), &[0.0, 1000.0, 4000.0]);
}

#[test]
fn hundreds_of_bpm_changes_dont_drift() {
    // osz2tja truncated time to whole ms at each change: 240 changes drifted up to 240 ms.
    let (a, b) = (151.3, 97.7);
    let mut body = String::new();
    for i in 0..240 {
        let bpm = if i % 2 == 0 { a } else { b };
        body += &format!("#BPMCHANGE {bpm}\n1,\n");
    }
    body += "1,";
    let c = one(&chart(a, &body));
    let last = *times(&c).last().unwrap();
    let exact = 120.0 * (240_000.0 / a + 240_000.0 / b);
    assert!((last - exact).abs() < 1e-6, "{last} vs {exact}");
}

#[test]
fn tuplets_land_where_they_are_written() {
    // 7 and 5 symbols in a 4/4 bar at 120 BPM (2000 ms).
    let c = one(&chart(120.0, "1111111,\n11111,"));
    let mut want: Vec<f64> = (0..7).map(|i| i as f64 * 2000.0 / 7.0).collect();
    want.extend((0..5).map(|i| 2000.0 + i as f64 * 400.0));
    close(&times(&c), &want);
}

#[test]
fn measure_sets_the_bar_length() {
    let c = one(&chart(
        120.0,
        "#MEASURE 7/8\n1000000,\n#MEASURE 5/16\n10000,\n#MEASURE 4/4\n1,",
    ));
    // 7/8 at 120 BPM: 1750 ms; 5/16: 625 ms.
    close(&times(&c), &[0.0, 1750.0, 2375.0]);
}

#[test]
fn delay_shifts_everything_after_it() {
    let c = one(&chart(120.0, "1,\n#DELAY 0.123\n1,\n1,"));
    close(&times(&c), &[0.0, 2123.0, 4123.0]);
    // Mid-bar, from where it stands.
    let mid = one(&chart(120.0, "11\n#DELAY 0.25\n11,"));
    close(&times(&mid), &[0.0, 500.0, 1250.0, 1750.0]);
}

#[test]
fn a_negative_delay_keeps_notes_in_time_order() {
    let c = one(&chart(120.0, "1,\n#DELAY -2.5\n2,"));
    close(&times(&c), &[-500.0, 0.0]);
    assert_eq!(c.map.notes[0].kind, Kind::Kat);
}

#[test]
fn a_bpm_change_mid_bar_applies_from_its_place() {
    let c = one(&chart(120.0, "11\n#BPMCHANGE 240\n11,\n1,"));
    close(&times(&c), &[0.0, 500.0, 1000.0, 1250.0, 1500.0]);
}

#[test]
fn speed_follows_bpm_and_scroll() {
    let c = one(&chart(
        120.0,
        "1,\n#SCROLL 2\n1,\n#BPMCHANGE 240\n#SCROLL 0.5\n1,\n#SCROLL 1+3i\n1,",
    ));
    let v: Vec<f64> = c.map.notes.iter().map(|n| n.velocity).collect();
    assert!((v[1] / v[0] - 2.0).abs() < 1e-9);
    // 240 BPM at 0.5 scrolls as 120 at 1.
    assert!((v[2] / v[0] - 1.0).abs() < 1e-9);
    // The complex scroll's real part.
    assert!((v[3] / v[0] - 2.0).abs() < 1e-9);
    assert!((v[0] - velocity(120.0, 1.0)).abs() < 1e-12);
    // At SliderMultiplier 1.4 a beat of 500 ms covers 196 osu!px.
    assert!((v[0] * 500.0 - 196.0).abs() < 1e-9);
}

#[test]
fn rolls_and_balloons() {
    let text = chart(120.0, "5008,\n7008,\n9008,\n6000,\n0080,")
        .replace("LEVEL:8", "LEVEL:8\nBALLOON:12,30");
    let c = one(&text);
    let n = &c.map.notes;
    assert_eq!(n.len(), 4);
    assert_eq!((n[0].kind, n[0].big), (Kind::Roll, false));
    assert!((n[0].end_time - 1500.0).abs() < 1e-9);
    // 120 BPM: 1/8 beat.
    assert!((n[0].tick_spacing - 62.5).abs() < 1e-9);
    assert_eq!((n[1].kind, n[1].required_hits), (Kind::Swell, 12));
    assert_eq!((n[2].kind, n[2].required_hits), (Kind::Swell, 30));
    // A big roll over a bar line.
    assert_eq!((n[3].kind, n[3].big), (Kind::Roll, true));
    assert!((n[3].time - 6000.0).abs() < 1e-9);
    assert!((n[3].end_time - 9000.0).abs() < 1e-9);
}

#[test]
fn missing_balloon_counts_fall_back() {
    let song = parse(&chart(120.0, "7008,")).unwrap();
    assert_eq!(
        song.courses[0].map.notes[0].required_hits,
        DEFAULT_BALLOON_HITS
    );
    assert!(song.warnings.iter().any(|w| w.contains("BALLOON")));
}

#[test]
fn courses_come_apart_with_their_own_fields() {
    let text = "TITLE:Two\nSUBTITLE:--Someone\nBPM:150\nWAVE:song.ogg\nDEMOSTART:12.5\n\
        COURSE:Oni\nLEVEL:9\nBALLOON:20\n#START\n7008,\n#END\n\
        COURSE:0\nLEVEL:2\nBALLOON:4\n#START\n7008,\n#END\n";
    let song = parse(text).unwrap();
    assert_eq!(song.title, "Two");
    assert_eq!(song.subtitle, "Someone");
    assert_eq!(song.wave, "song.ogg");
    let got: Vec<(CourseKind, u32, u32)> = song
        .courses
        .iter()
        .map(|c| (c.kind, c.level, c.map.notes[0].required_hits))
        .collect();
    assert_eq!(got, [(CourseKind::Easy, 2, 4), (CourseKind::Oni, 9, 20)]);
    let m = &song.courses[1].map.meta;
    assert_eq!(m.version, "Oni");
    assert_eq!(m.level, Some(9.0));
    assert_eq!(m.audio_file, "song.ogg");
    assert_eq!(m.preview_time, 12_500.0);
    assert_eq!(m.mode, 1);
    assert_ne!(song.courses[0].hash, song.courses[1].hash);
}

#[test]
fn only_the_master_branch_is_played() {
    let text = chart(
        120.0,
        "1,\n#BRANCHSTART p,50,80\n#N\n7008,\n#E\n7008,\n#M\n7008,\n#BRANCHEND\n7008,",
    )
    .replace("LEVEL:8", "LEVEL:8\nBALLOON:1,2,3,4");
    let c = one(&text);
    let n = &c.map.notes;
    assert_eq!(n.len(), 3);
    close(&times(&c), &[0.0, 2000.0, 4000.0]);
    // BALLOON: counts through every branch: the master one is the third.
    assert_eq!(n[1].required_hits, 3);
    assert_eq!(n[2].required_hits, 4);
}

#[test]
fn per_branch_balloon_lists_win() {
    let text = chart(
        120.0,
        "#BRANCHSTART p,50,80\n#N\n7008,\n#E\n7008,\n#M\n7008,\n7008,",
    )
    .replace("LEVEL:8", "LEVEL:8\nBALLOON:1,2,3,4\nBALLOONMAS:40,41");
    let n = one(&text).map.notes;
    assert_eq!(
        n.iter().map(|n| n.required_hits).collect::<Vec<_>>(),
        [40, 41]
    );
}

#[test]
fn a_balloon_list_as_long_as_each_branch_is_read_per_branch() {
    // Official charts often list one branch's balloons: common ones, then the branch's.
    let text = chart(
        120.0,
        "7008,\n#BRANCHSTART p,0,0\n#N\n7008,\n#E\n7008,\n#M\n7008,",
    )
    .replace("LEVEL:8", "LEVEL:8\nBALLOON:39,4");
    let n = one(&text).map.notes;
    assert_eq!(
        n.iter().map(|n| n.required_hits).collect::<Vec<_>>(),
        [39, 4]
    );
}

#[test]
fn a_head_inside_an_open_balloon_takes_no_count() {
    // A kusudama written 9090: the second 9 is a blank, as in OpenTaiko.
    let text = chart(120.0, "9090,\n0008,\n7008,").replace("LEVEL:8", "LEVEL:8\nBALLOON:10,20");
    let n = one(&text).map.notes;
    assert_eq!(n.len(), 2);
    assert!((n[0].end_time - 3500.0).abs() < 1e-9);
    assert_eq!(
        n.iter().map(|n| n.required_hits).collect::<Vec<_>>(),
        [10, 20]
    );
}

#[test]
fn a_branch_section_without_master_plays_expert() {
    let c = one(&chart(
        120.0,
        "#BRANCHSTART r,5,10\n#N\n1,\n#E\n2,\n#BRANCHEND\n1,",
    ));
    assert_eq!(c.map.notes[0].kind, Kind::Kat);
    close(&times(&c), &[0.0, 2000.0]);
}

#[test]
fn unknown_things_are_warnings() {
    let song = parse(&chart(120.0, "#BGAON 1 0\n1C2F,\n#WHATEVER\nBALLOON:3\n1,")).unwrap();
    let c = &song.courses[0];
    close(&times(c), &[0.0, 1000.0, 2000.0]);
    for w in ["#BGAON", "bombs", "ADLib", "#WHATEVER", "header line"] {
        assert!(
            song.warnings.iter().any(|x| x.contains(w)),
            "{w}: {:?}",
            song.warnings
        );
    }
}

#[test]
fn style_double_falls_back_to_p1() {
    let text = "BPM:120\nCOURSE:Oni\nSTYLE:Double\n#START P1\n1,\n#END\n#START P2\n2,\n#END\n";
    let c = one(text);
    assert_eq!(c.map.notes[0].kind, Kind::Don);
    let single = format!("{text}STYLE:Single\n#START\n3,\n#END\n");
    assert!(one(&single).map.notes[0].big);
}

#[test]
fn no_notes_is_an_error() {
    assert!(parse("TITLE:x\nBPM:120\n").is_err());
    assert!(parse(&chart(120.0, "0000,")).is_err());
}

#[test]
fn gogo_is_kiai() {
    let c = one(&chart(120.0, "1,\n#GOGOSTART\n1,\n#GOGOEND\n1,"));
    let kiai = |t: f64| crate::beatmap::control_point_at(&c.map.control_points, t).kiai;
    assert!(!kiai(100.0) && kiai(2100.0) && !kiai(4100.0));
}

#[test]
fn barlines_can_be_turned_off() {
    let c = one(&chart(120.0, "1,\n#BARLINEOFF\n1,\n#BARLINEON\n1,"));
    let bars: Vec<f64> = c.map.barlines.iter().map(|b| b.time).collect();
    close(&bars, &[0.0, 4000.0]);
}

const JAPANESE: &str = "TITLE:さくらんぼ\nSUBTITLE:--テスト\nBPM:120\nOFFSET:0\nCOURSE:Oni\n\
    LEVEL:7\n#START\n1020,\n#END\n";

#[test]
fn shift_jis_and_utf8_read_alike() {
    let (sjis, _, bad) = encoding_rs::SHIFT_JIS.encode(JAPANESE);
    assert!(!bad);
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend_from_slice(JAPANESE.as_bytes());
    let songs: Vec<Song> = [JAPANESE.as_bytes(), &sjis[..], &bom[..]]
        .iter()
        .map(|b| parse_bytes(b).unwrap())
        .collect();
    for s in &songs {
        assert_eq!(s.title, "さくらんぼ");
        assert_eq!(s.subtitle, "テスト");
        assert_eq!(s.courses[0].hash, songs[0].courses[0].hash);
    }
}

#[test]
fn the_hash_ignores_how_the_file_is_written() {
    let base = one(JAPANESE).hash;
    let rewritten = "// a comment\nCOURSE:oni\nBPM: 120\nLEVEL:7\nTITLE:other title\n\n\
        #START\n  10 // half\n20,\n#END\n";
    assert_eq!(one(rewritten).hash, base);
    // A note moved, a kind changed, another course: another chart.
    assert_ne!(one(&JAPANESE.replace("1020", "1002")).hash, base);
    assert_ne!(one(&JAPANESE.replace("1020", "1010")).hash, base);
    assert_ne!(
        one(&JAPANESE.replace("COURSE:Oni", "COURSE:Hard")).hash,
        base
    );
    assert_eq!(base.hex().len(), 64);
}

/// Notes of an official fumen `.bin` (the arcade's own chart format), from the branch `branch`
/// (0 normal, 1 expert, 2 master): (time ms, kind, big, end ms, pops). Layout as tja2fumen
/// documents it (MIT): a 520-byte header ending in the bar count, then per bar BPM, offset,
/// flags and three branches of 24-byte notes (rolls 8 bytes more).
fn read_fumen(bytes: &[u8], branch: usize) -> Vec<(f64, Kind, bool, f64, u32)> {
    let bars_le = u32::from_le_bytes(bytes[512..516].try_into().unwrap());
    let bars_be = u32::from_be_bytes(bytes[512..516].try_into().unwrap());
    let be = bars_be < bars_le;
    let at = std::cell::Cell::new(520usize);
    let take = |n: usize| {
        let mut b = [0u8; 4];
        let end = (at.get() + n).min(bytes.len());
        let got = &bytes[at.get().min(end)..end];
        b[..got.len()].copy_from_slice(got);
        at.set(at.get() + n);
        b
    };
    let u = |b: [u8; 4]| {
        if be {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    };
    let f = |b: [u8; 4]| f32::from_bits(u(b)) as f64;
    let half = |b: [u8; 4], hi: bool| {
        let (x, y) = if be {
            (
                u16::from_be_bytes([b[0], b[1]]),
                u16::from_be_bytes([b[2], b[3]]),
            )
        } else {
            (
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
            )
        };
        if hi { y } else { x }
    };
    let mut out = Vec::new();
    for _ in 0..bars_le.min(bars_be) {
        if at.get() >= bytes.len() {
            break;
        }
        let bpm = f(take(4));
        let offset = f(take(4));
        for _ in 0..8 {
            take(4);
        }
        for b in 0..3 {
            let count = half(take(4), false);
            take(4);
            for _ in 0..count {
                let ty = u(take(4));
                let pos = f(take(4));
                take(4);
                take(4);
                let pops = half(take(4), false) as u32;
                let duration = f(take(4));
                if matches!(ty, 0x6 | 0x9 | 0x62) {
                    take(4);
                    take(4);
                }
                if b != branch {
                    continue;
                }
                let t = offset + 240_000.0 / bpm + pos;
                let (kind, big) = match ty {
                    0x1..=0x3 => (Kind::Don, false),
                    0x4 | 0x5 => (Kind::Kat, false),
                    0x6 | 0x62 => (Kind::Roll, false),
                    0x7 | 0xb => (Kind::Don, true),
                    0x8 | 0xd => (Kind::Kat, true),
                    0x9 => (Kind::Roll, true),
                    0xa | 0xc => (Kind::Swell, false),
                    _ => continue,
                };
                let pops = if kind == Kind::Swell { pops } else { 0 };
                out.push((t, kind, big, t + duration, pops));
            }
        }
    }
    out
}

/// Every `.tja` in `TAIKO_TJA_FUMEN` against the official fumens beside it (`<name>_m.bin` for
/// Oni and so on), Master branch. Local only: the charts are Bandai Namco's and stay out of the
/// repository. `TAIKO_TJA_FUMEN=dir cargo test -p taiko-logic tja::tests::fumen -- --ignored`
#[test]
#[ignore]
fn fumen_corpus_matches_official_timing() {
    let dir = std::path::PathBuf::from(std::env::var("TAIKO_TJA_FUMEN").expect("TAIKO_TJA_FUMEN"));
    let mut charts = 0;
    let mut worst = 0.0f64;
    let mut failures = Vec::new();
    let mut roll_ends = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "tja") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let song = parse_bytes(&std::fs::read(&path).unwrap()).unwrap();
        for course in &song.courses {
            let suffix = match course.kind {
                CourseKind::Easy => "e",
                CourseKind::Normal => "n",
                CourseKind::Hard => "h",
                CourseKind::Oni => "m",
                CourseKind::Edit => "x",
            };
            let Ok(bin) = std::fs::read(dir.join(format!("{stem}_{suffix}.bin"))) else {
                continue;
            };
            let mut official = read_fumen(&bin, 2);
            if official.is_empty() {
                official = read_fumen(&bin, 0);
            }
            charts += 1;
            let ours = &course.map.notes;
            let name = format!("{stem} {}", course.kind.name());
            // The 2/4 bars of KAGEKIYO's official Ura chart have notes past the end of the bar,
            // which TJA can't write (tja2fumen skips those bars too): up to 10 ms allowed there.
            let allowed = if name == "genpe Ura Oni" { 10.0 } else { 1.0 };
            if ours.len() != official.len() {
                failures.push(format!(
                    "{name}: {} notes, official {}",
                    ours.len(),
                    official.len()
                ));
                continue;
            }
            for (n, o) in ours.iter().zip(&official) {
                let d = (n.time - o.0).abs();
                if allowed == 1.0 {
                    worst = worst.max(d);
                }
                if d > allowed || n.kind != o.1 || n.big != o.2 {
                    failures.push(format!(
                        "{name}: ours {:.3} {:?}{} vs official {:.3} {:?}{}",
                        n.time,
                        n.kind,
                        if n.big { " big" } else { "" },
                        o.0,
                        o.1,
                        if o.2 { " big" } else { "" }
                    ));
                    break;
                }
                if n.kind == Kind::Swell && n.required_hits != o.4 {
                    failures.push(format!(
                        "{name}: balloon at {:.0}: {} pops, official {}",
                        n.time, n.required_hits, o.4
                    ));
                    break;
                }
                // Roll ends are only counted: TJA charters place them by eye, so they are often
                // a division off the official chart (tja2fumen doesn't check them either).
                if n.kind != Kind::Don && n.kind != Kind::Kat && (n.end_time - o.3).abs() > 1.0 {
                    roll_ends += 1;
                }
            }
        }
    }
    eprintln!(
        "{charts} charts, worst note time difference {worst:.4} ms, {roll_ends} roll ends off"
    );
    assert!(charts > 0, "no chart with a fumen beside it");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_fuse_roll_is_a_balloon_and_takes_a_count() {
    // OpenTaiko reads D as a balloon: it takes the next BALLOON: value, so the balloons after
    // it keep theirs.
    let text = chart(120.0, "D008,\n7008,").replace("LEVEL:8", "LEVEL:8\nBALLOON:10,20");
    let song = parse(&text).unwrap();
    let n = &song.courses[0].map.notes;
    assert_eq!(
        n.iter()
            .map(|n| (n.kind, n.required_hits))
            .collect::<Vec<_>>(),
        [(Kind::Swell, 10), (Kind::Swell, 20)]
    );
    assert!(song.warnings.iter().any(|w| w.contains("fuse")));
}

#[test]
fn letters_opentaiko_doesnt_have_are_blanks() {
    // E is no note in OpenTaiko: skipped, its division still lasts.
    let song = parse(&chart(120.0, "1E2,")).unwrap();
    close(&times(&song.courses[0]), &[0.0, 4000.0 / 3.0]);
    assert!(song.warnings.iter().any(|w| w.contains("'E'")));
}

/// A course's board chart goes over the wire and back to the very chart the game played.
#[test]
fn a_board_chart_reads_back_to_the_chart_played() {
    let c = one(&chart(
        150.0,
        "1020,\n#GOGOSTART\n#SCROLL 2\n3400,\n#GOGOEND\n5000,\n0008,\n#BPMCHANGE 200\n7000,\n0080,",
    ));
    let board = BoardChart::of(c.kind, &c.map);
    assert_eq!(board.hash(), c.hash);
    let sent: BoardChart = serde_json::from_str(&serde_json::to_string(&board).unwrap()).unwrap();
    assert_eq!(sent, board);
    let played = sent.beatmap().unwrap();
    assert_eq!(BoardChart::of(c.kind, &played), board);
    assert_eq!(played.meta.windows, c.map.meta.windows);
    assert_eq!(played.notes.len(), c.map.notes.len());
    for (a, b) in played.notes.iter().zip(&c.map.notes) {
        assert_eq!(
            (
                a.time,
                a.end_time,
                a.kind,
                a.big,
                a.velocity,
                a.tick_spacing,
                a.required_hits
            ),
            (
                b.time,
                b.end_time,
                b.kind,
                b.big,
                b.velocity,
                b.tick_spacing,
                b.required_hits
            )
        );
    }
    let kiai = |m: &Beatmap| {
        m.control_points
            .iter()
            .map(|p| (p.time, p.kiai))
            .collect::<Vec<_>>()
    };
    assert_eq!(kiai(&played), kiai(&c.map));
}

/// Anything that changes how a chart plays or scores gives another board, gogo time included.
#[test]
fn gogo_time_and_bar_lines_are_part_of_the_hash() {
    let base = one(JAPANESE).hash;
    let gogo = JAPANESE.replace("#START\n", "#START\n#GOGOSTART\n");
    assert_ne!(one(&gogo).hash, base, "gogo time scores more");
    let no_bars = JAPANESE.replace("#START\n", "#START\n#BARLINEOFF\n");
    assert_ne!(one(&no_bars).hash, base);
}

/// What a server is sent isn't trusted: a chart no TJA file gives is refused.
#[test]
fn a_board_chart_no_file_gives_is_refused() {
    let good = BoardChart::of(CourseKind::Oni, &one(JAPANESE).map);
    assert!(good.beatmap().is_ok());
    let broken = |change: fn(&mut BoardChart)| {
        let mut b = good.clone();
        change(&mut b);
        b.beatmap().is_err()
    };
    assert!(broken(|b| b.course = 9));
    assert!(broken(|b| b.notes.clear()));
    assert!(broken(|b| b.points.clear()));
    assert!(broken(|b| b.notes[0].0 = f64::NAN));
    assert!(broken(|b| b.notes[0].2 = 4));
    assert!(broken(|b| b.notes[0].4 = 0.0));
    assert!(broken(|b| b.notes[1].1 = b.notes[1].0 - 1.0));
    assert!(broken(|b| b.notes.swap(0, 1)));
    assert!(broken(|b| b.points[0].2 = -1.0));
    assert!(broken(|b| {
        let n = b.notes[0];
        b.notes = vec![n; MAX_ITEMS + 1];
    }));
    // Easy plays with the wide windows, Oni the narrow ones.
    let mut easy = good.clone();
    easy.course = CourseKind::Easy as u8;
    assert_eq!(
        easy.beatmap().unwrap().meta.windows,
        crate::beatmap::WindowSource::Arcade { narrow: false }
    );
    assert_ne!(easy.hash(), good.hash());
}

/// How the map's gimmick draws its notes moving (TJA scroll gimmicks), if it has one.
fn motion(c: &Course) -> Option<&crate::tko::gimmick::Motion> {
    c.map.gimmick.as_deref().map(|g| &g.motion)
}

/// The shared scroll's speed keys: (time, speed).
fn lane_keys(c: &Course) -> Vec<(f64, f64)> {
    c.map
        .gimmick
        .as_deref()
        .map(|g| g.scroll.keys.iter().map(|k| (k.t, k.v)).collect())
        .unwrap_or_default()
}

#[test]
fn a_chart_without_scroll_gimmicks_has_no_gimmick() {
    let c = one(&chart(
        120.0,
        "1,\n#SCROLL 2\n#BPMCHANGE 240\n1,\n#DELAY 0.5\n1,",
    ));
    assert!(c.map.gimmick.is_none());
}

#[test]
fn bmscroll_steps_the_whole_lane_with_the_bpm() {
    let c = one(&chart(
        120.0,
        "#BMSCROLL\n1,\n#BPMCHANGE 240\n#SCROLL 3\n1,\n#BPMCHANGE 60\n1,",
    ));
    // Bars of 2000 ms at 120, 1000 at 240.
    close(&times(&c), &[0.0, 2000.0, 3000.0]);
    assert_eq!(lane_keys(&c), [(0.0, 1.0), (2000.0, 2.0), (3000.0, 0.5)]);
    let m = motion(&c).expect("a gimmick");
    assert!(m.map_clock, "TJA times aren't moved like an .osu's");
    // #SCROLL is left out: every note at the song's BPM, the lane does the rest.
    let drawn: Vec<f64> = m.notes.iter().map(|n| n.velocity).collect();
    close(&drawn, &[velocity(120.0, 1.0); 3]);
    // The board's own velocities are what normal mode gives.
    assert!((c.map.notes[1].velocity - velocity(240.0, 3.0)).abs() < 1e-12);
}

#[test]
fn a_delay_stops_a_lane_scrolled_by_beats() {
    let c = one(&chart(120.0, "#BMSCROLL\n1,\n#DELAY 0.5\n1,"));
    close(&times(&c), &[0.0, 2500.0]);
    assert_eq!(lane_keys(&c), [(0.0, 1.0), (2000.0, 0.0), (2500.0, 1.0)]);
    let table =
        crate::tko::scroll::ScrollTable::new(&c.map.gimmick.as_ref().unwrap().scroll).unwrap();
    assert_eq!(table.x(2000.0), table.x(2400.0), "frozen through the delay");
    // In normal mode a delay only moves the notes.
    let normal = one(&chart(120.0, "#SUDDEN 1 1\n1,\n#DELAY 0.5\n1,"));
    assert!(lane_keys(&normal).is_empty());
}

#[test]
fn hbscroll_keeps_each_notes_scroll_and_its_side() {
    let c = one(&chart(
        120.0,
        "#HBSCROLL\n1,\n#SCROLL 2\n#BPMCHANGE 240\n1,\n#SCROLL -0.5\n1,",
    ));
    assert_eq!(lane_keys(&c), [(0.0, 1.0), (2000.0, 2.0)]);
    let drawn: Vec<f64> = motion(&c)
        .unwrap()
        .notes
        .iter()
        .map(|n| n.velocity)
        .collect();
    // At the song's BPM times #SCROLL; a negative one comes from the left.
    close(
        &drawn,
        &[
            velocity(120.0, 1.0),
            velocity(120.0, 2.0),
            -velocity(120.0, 0.5),
        ],
    );
    // A negative #SCROLL in normal mode comes from the left too, at its own speed, with no
    // shared scroll; the warning it had is gone.
    let song = parse(&chart(120.0, "1,\n#SCROLL -2\n1,")).unwrap();
    let normal = &song.courses[0];
    assert!(lane_keys(normal).is_empty());
    let drawn: Vec<f64> = motion(normal)
        .unwrap()
        .notes
        .iter()
        .map(|n| n.velocity)
        .collect();
    close(&drawn, &[velocity(120.0, 1.0), -velocity(120.0, 2.0)]);
    assert!(song.warnings.is_empty(), "{:?}", song.warnings);
}

#[test]
fn nmscroll_goes_back_to_scrolling_by_time() {
    let c = one(&chart(
        120.0,
        "#BMSCROLL\n1,\n#BPMCHANGE 240\n#NMSCROLL\n#SCROLL 2\n1,",
    ));
    assert_eq!(lane_keys(&c), [(0.0, 1.0), (2000.0, 1.0)]);
    let drawn: Vec<f64> = motion(&c)
        .unwrap()
        .notes
        .iter()
        .map(|n| n.velocity)
        .collect();
    close(&drawn, &[velocity(120.0, 1.0), velocity(240.0, 2.0)]);
}

#[test]
fn sudden_sets_when_each_note_shows_up_and_starts_moving() {
    let c = one(&chart(
        120.0,
        "#SUDDEN 3 2\n1,\n#SUDDEN 0 0\n1,\n#SUDDEN 1.5 0\n5008,\n#SUDDEN 0.0004 1\n1,\n#SUDDEN 0.3 inf\n5008,",
    ));
    let m = &motion(&c).unwrap().notes;
    let shown: Vec<(f64, f64, f64, bool)> = m
        .iter()
        .map(|n| (n.t, n.show, n.wait, n.hide_long))
        .collect();
    let inf = f64::INFINITY;
    assert_eq!(
        shown,
        [
            (0.0, 3000.0, 2000.0, false),
            // #SUDDEN 0 0: off.
            (2000.0, inf, inf, false),
            // Only the first number: TJAPlayer3 never drew rolls then.
            (4000.0, 1500.0, inf, true),
            // Under a millisecond is off, as OpenTaiko reads it.
            (6000.0, inf, 1000.0, false),
            // `inf`: moving all along, and the rolls drawn.
            (8000.0, 300.0, inf, false),
        ]
    );
    assert!(lane_keys(&c).is_empty(), "no shared scroll in normal mode");
}

/// Scroll gimmicks only change how notes are drawn: the board (leaderboards, local scores) and
/// what is judged stay the chart's.
#[test]
fn scroll_gimmicks_leave_the_board_as_it_is() {
    let body =
        "1020,\n#BPMCHANGE 200\n#SCROLL 1.5\n1111,\n#DELAY 0.25\n#SCROLL -1\n5008,\n#GOGOSTART\n2,";
    let base = one(&chart(150.0, body));
    let with = |extra: &str| one(&chart(150.0, &format!("{extra}\n{body}")));
    for extra in [
        "#BMSCROLL",
        "#HBSCROLL",
        "#SUDDEN 2 1",
        "#HBSCROLL\n#SUDDEN 1 0",
    ] {
        let c = with(extra);
        assert_eq!(c.hash, base.hash, "{extra}");
        let board = BoardChart::of(c.kind, &c.map);
        assert_eq!(board, BoardChart::of(base.kind, &base.map), "{extra}");
        assert_eq!(board.hash(), c.hash, "{extra}");
        assert!(c.map.gimmick.is_some(), "{extra}");
    }
}

#[test]
fn complex_numbers_read_as_opentaiko_writes_them() {
    assert_eq!(complex_value("5"), Some((5.0, 0.0)));
    assert_eq!(complex_value("-10i"), Some((0.0, -10.0)));
    assert_eq!(complex_value("5-3i"), Some((5.0, -3.0)));
    assert_eq!(complex_value("-2+1i"), Some((-2.0, 1.0)));
    assert_eq!(complex_value("1e2+i"), Some((100.0, 1.0)));
    assert_eq!(complex_value("2e-1-i"), Some((0.2, -1.0)));
    assert_eq!(complex_value("x"), None);
    assert_eq!(complex_value("inf"), None);
}

#[test]
fn jposscroll_moves_the_hit_target_at_an_even_pace() {
    // 240 BPM: bars of 1000 ms. OpenTaiko's 30 px are 20 virtual units here.
    let c = one(&chart(
        240.0,
        "#JPOSSCROLL 0.5 30 1\n1,\n#JPOSSCROLL 0 -30i 1\n1,\n#JPOSSCROLL 2 15 0\n1,\n#JPOSSCROLL 0 0 1\n1,",
    ));
    let m = motion(&c).expect("a gimmick");
    let at = |t: f64| {
        let (x, y) = m.judge_at(t);
        ((x * 1000.0).round() / 1000.0, (y * 1000.0).round() / 1000.0)
    };
    assert_eq!(at(-100.0), (0.0, 0.0));
    assert_eq!(at(250.0), (10.0, 0.0), "half way");
    assert_eq!(at(900.0), (20.0, 0.0));
    // At once, and up: an imaginary distance moves it vertically.
    assert_eq!(at(1000.0), (20.0, -20.0));
    // Direction 0 goes the other way; cut short by the next move after half its time.
    assert_eq!(at(2500.0), (17.5, -20.0));
    assert_eq!(at(3000.0), (15.0, -20.0));
    assert_eq!(at(9000.0), (15.0, -20.0), "it stays where the cut left it");
    assert!(
        parse(&chart(240.0, "#JPOSSCROLL 1 30 1\n1,"))
            .unwrap()
            .warnings
            .is_empty()
    );
}

#[test]
fn lyrics_show_from_their_time_until_the_next() {
    let c = one(&chart(
        120.0,
        "#LYRIC Hello world\n1,\n#LYRIC 歌詞\n1,\n#LYRIC\n1,",
    ));
    let m = motion(&c).expect("a gimmick");
    assert_eq!(m.lyric_at(-1.0), None);
    assert_eq!(m.lyric_at(0.0), Some("Hello world"));
    assert_eq!(m.lyric_at(2500.0), Some("歌詞"));
    assert_eq!(m.lyric_at(4000.0), None, "an empty one clears it");
}

#[test]
fn judge_moves_and_lyrics_leave_the_board_as_it_is() {
    let body = "1020,\n#BPMCHANGE 200\n1111,\n5008,";
    let base = one(&chart(150.0, body));
    let c = one(&chart(
        150.0,
        &format!("#JPOSSCROLL 1 100+20i 1\n#LYRIC la la\n{body}"),
    ));
    assert_eq!(c.hash, base.hash);
    assert_eq!(
        BoardChart::of(c.kind, &c.map),
        BoardChart::of(base.kind, &base.map)
    );
}

/// One course of each kind named, each a bar of one don.
fn courses(header: &str, per_course: &[(&str, &str)]) -> Song {
    let mut text = format!("TITLE:Who\nBPM:120\n{header}");
    for (course, lines) in per_course {
        text += &format!("COURSE:{course}\n{lines}#START\n1,\n#END\n");
    }
    parse(&text).unwrap()
}

fn charters(song: &Song) -> Vec<(CourseKind, &str)> {
    song.courses
        .iter()
        .map(|c| (c.kind, c.map.meta.creator.as_str()))
        .collect()
}

#[test]
fn notes_designers_name_each_courses_charter() {
    // Numbered before the courses: that course's; within a course, that one's whatever its
    // number; none: MAKER.
    let song = courses(
        "MAKER:Team\nNOTESDESIGNER3:Oni Person\nNOTESDESIGNER0:Easy Person\n",
        &[
            ("Oni", ""),
            ("Easy", ""),
            ("Hard", "NOTESDESIGNER0:Hard Person\n"),
            ("Normal", ""),
        ],
    );
    assert_eq!(
        charters(&song),
        [
            (CourseKind::Easy, "Easy Person"),
            (CourseKind::Normal, "Team"),
            (CourseKind::Hard, "Hard Person"),
            (CourseKind::Oni, "Oni Person"),
        ]
    );
    // Unnumbered before the courses: every course's.
    let song = courses(
        "MAKER:Team\nNOTESDESIGNER:Everyone\nNOTESDESIGNER4:Ura Person\n",
        &[("Oni", ""), ("Edit", "")],
    );
    assert_eq!(
        charters(&song),
        [
            (CourseKind::Oni, "Everyone"),
            (CourseKind::Edit, "Ura Person")
        ]
    );
    assert!(song.warnings.is_empty(), "{:?}", song.warnings);
    // A course number there is no course for.
    let song = courses("NOTESDESIGNER7:Nobody\n", &[("Oni", "")]);
    assert!(song.warnings.iter().any(|w| w.contains("NOTESDESIGNER7")));
}

#[test]
fn level_is_kept_apart_from_the_course_name() {
    let song = courses(
        "",
        &[
            ("Oni", "LEVEL:9.5\n"),
            ("Hard", "LEVEL:7\n"),
            ("Normal", "LEVEL:lots\n"),
            ("Easy", ""),
        ],
    );
    let got: Vec<(&str, Option<f32>, u32)> = song
        .courses
        .iter()
        .map(|c| (c.map.meta.version.as_str(), c.map.meta.level, c.level))
        .collect();
    assert_eq!(
        got,
        [
            ("Easy", None, 0),
            ("Normal", None, 0),
            ("Hard", Some(7.0), 7),
            ("Oni", Some(9.5), 9),
        ]
    );
}

/// Each note's way in (rounded) and speed against a `#SCROLL 1` note's.
fn ways(c: &Course) -> Vec<(Option<[f64; 2]>, f64)> {
    let m = motion(c).expect("a gimmick");
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    m.notes
        .iter()
        .map(|n| {
            (
                n.dir.map(|[x, y]| [round(x), round(y)]),
                round(n.velocity / velocity(120.0, 1.0)),
            )
        })
        .collect()
}

#[test]
fn complex_scroll_and_direction_send_notes_in_from_any_side() {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let h = (h * 1000.0).round() / 1000.0;
    let c = one(&chart(
        120.0,
        "#SCROLL i\n1,\n#SCROLL -2i\n1,\n#SCROLL 1+i\n1,\n#SCROLL 1\n#DIRECTION 1\n1,\n#DIRECTION 3\n1,\n#DIRECTION 5\n1,\n#DIRECTION 0\n#SCROLL 0.5\n1,",
    ));
    assert_eq!(
        ways(&c),
        [
            // OpenTaiko's own axes: an imaginary part points down, so the note comes up from
            // under the drum.
            (Some([0.0, 1.0]), 1.0),
            (Some([0.0, -1.0]), 2.0),
            (Some([h, h]), 1.414),
            // #DIRECTION 1: down from above; 3: from up and right, as fast across as before.
            (Some([0.0, -1.0]), 1.0),
            (Some([h, -h]), 1.414),
            // 5: from the left, along the lane.
            (None, -1.0),
            (None, 0.5),
        ]
    );
    let barlines = &motion(&c).unwrap().barline_dirs;
    assert_eq!(
        barlines.len(),
        5,
        "only the ones off the lane: {barlines:?}"
    );
    assert!(
        parse(&chart(120.0, "#DIRECTION 2\n1,"))
            .unwrap()
            .warnings
            .is_empty()
    );
}

#[test]
fn taikojiro_axes_point_the_imaginary_part_up() {
    let c = one(&format!(
        "COMPAT:jiro2\n{}",
        chart(120.0, "#SCROLL i\n1,\n#SCROLL 1+i\n#DIRECTION 5\n1,")
    ));
    // Jiro's #DIRECTION sets the way whole: 5 is plain from the left.
    assert_eq!(ways(&c), [(Some([0.0, -1.0]), 1.0), (None, -1.0)]);
    // OpenTaiko's keeps the complex #SCROLL's vertical part through it.
    let c = one(&chart(120.0, "#SCROLL 1+i\n#DIRECTION 5\n1,"));
    let h = (std::f64::consts::FRAC_1_SQRT_2 * 1000.0).round() / 1000.0;
    assert_eq!(ways(&c), [(Some([-h, h]), 1.414)]);
}

#[test]
fn rolls_stay_on_the_lane_whatever_way_notes_come() {
    let c = one(&chart(120.0, "#SCROLL 2i\n5008,\n1,"));
    let m = motion(&c).expect("a gimmick");
    assert_eq!(m.notes[0].dir, None, "the roll");
    assert!((m.notes[0].velocity - velocity(120.0, 2.0)).abs() < 1e-9);
    assert_eq!(m.notes[1].dir, Some([0.0, 1.0]));
}

#[test]
fn ways_in_leave_the_board_as_it_is() {
    let body = "1020,\n#BPMCHANGE 200\n1111,\n5008,";
    let base = one(&chart(150.0, &format!("#SCROLL 2\n{body}")));
    for gimmick in [
        "#SCROLL 2+3i",
        "#SCROLL 2-i\n#DIRECTION 4",
        "#SCROLL 2\n#DIRECTION 6",
    ] {
        let c = one(&chart(150.0, &format!("{gimmick}\n{body}")));
        assert_eq!(c.hash, base.hash, "{gimmick}");
        assert_eq!(
            BoardChart::of(c.kind, &c.map),
            BoardChart::of(base.kind, &base.map),
            "{gimmick}"
        );
    }
}
