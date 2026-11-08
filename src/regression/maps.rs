//! Charts written for the regression tests: every kind of object, timing and scroll the game
//! reads, as `.osu` text, plus the presses a player would make on them.

use crate::beatmap::{self, Beatmap, Kind};

/// One hit object as written in `[HitObjects]`.
#[derive(Clone, Copy)]
pub enum Obj {
    Don(i64),
    Kat(i64),
    BigDon(i64),
    BigKat(i64),
    /// A drumroll at `time`, `length` osu!px long, big or not.
    Roll(i64, f64, bool),
    /// A swell (spinner) from `time` to `end`.
    Swell(i64, i64),
}

/// A chart: difficulty settings, timing points as written and its objects.
pub struct Chart {
    pub name: &'static str,
    pub od: f64,
    pub hp: f64,
    pub slider_multiplier: f64,
    pub tick_rate: f64,
    /// `[TimingPoints]` lines.
    pub timing: Vec<String>,
    pub objects: Vec<Obj>,
}

impl Chart {
    pub fn text(&self) -> String {
        let mut t = format!(
            "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nAudioLeadIn: 0\n\
             PreviewTime: 1000\nMode: 1\n\n[Metadata]\nTitle:{name}\nArtist:Regression\n\
             Creator:tests\nVersion:{name}\nBeatmapID:0\nBeatmapSetID:-1\n\n[Difficulty]\n\
             HPDrainRate:{hp}\nCircleSize:5\nOverallDifficulty:{od}\nApproachRate:5\n\
             SliderMultiplier:{sm}\nSliderTickRate:{tr}\n\n[Events]\n\n[TimingPoints]\n",
            name = self.name,
            hp = self.hp,
            od = self.od,
            sm = self.slider_multiplier,
            tr = self.tick_rate,
        );
        for line in &self.timing {
            t += line;
            t.push('\n');
        }
        t += "\n[HitObjects]\n";
        for o in &self.objects {
            t += &match *o {
                Obj::Don(at) => format!("256,192,{at},1,0,0:0:0:0:\n"),
                Obj::Kat(at) => format!("256,192,{at},1,2,0:0:0:0:\n"),
                Obj::BigDon(at) => format!("256,192,{at},1,4,0:0:0:0:\n"),
                Obj::BigKat(at) => format!("256,192,{at},1,6,0:0:0:0:\n"),
                Obj::Roll(at, len, big) => format!(
                    "256,192,{at},2,{},L|456:192,1,{len}\n",
                    if big { 4 } else { 0 }
                ),
                Obj::Swell(at, end) => format!("256,192,{at},12,0,{end},0:0:0:0:\n"),
            };
        }
        t
    }

    pub fn beatmap(&self) -> Beatmap {
        beatmap::from_text(&self.text()).unwrap_or_else(|e| panic!("{}: {e}", self.name))
    }
}

/// `n` notes `step` ms apart from `start`, `pick(k)` choosing each.
fn run(start: i64, step: i64, n: i64, pick: impl Fn(i64, i64) -> Obj) -> Vec<Obj> {
    (0..n).map(|k| pick(k, start + k * step)).collect()
}

/// Every chart the regression tests play.
pub fn all() -> Vec<Chart> {
    vec![
        basics(),
        big_notes(),
        rolls_and_swells(),
        sv_and_kiai(),
        stream(),
        low_od(),
        high_od(),
    ]
}

/// 64 small notes at 200 BPM in eighths: a don/kat pattern with doubles and triples.
pub fn basics() -> Chart {
    Chart {
        name: "basics",
        od: 5.0,
        hp: 5.0,
        slider_multiplier: 1.4,
        tick_rate: 1.0,
        timing: vec!["1000,300,4,1,0,100,1,0".into()],
        objects: run(1000, 150, 64, |k, t| {
            if [1, 2, 5, 9, 10, 11].contains(&(k % 12)) {
                Obj::Kat(t)
            } else {
                Obj::Don(t)
            }
        }),
    }
}

/// Big and small notes mixed (finishers are big notes).
pub fn big_notes() -> Chart {
    Chart {
        name: "big-notes",
        od: 6.0,
        hp: 6.0,
        slider_multiplier: 1.4,
        tick_rate: 1.0,
        timing: vec!["500,400,4,1,0,100,1,0".into()],
        objects: run(500, 200, 40, |k, t| match k % 5 {
            0 => Obj::BigDon(t),
            1 => Obj::Kat(t),
            2 => Obj::BigKat(t),
            3 => Obj::Don(t),
            _ => Obj::Kat(t),
        }),
    }
}

/// Drumrolls (small and big, different lengths, tick rates) and swells between notes.
pub fn rolls_and_swells() -> Chart {
    let mut objects = vec![Obj::Don(1000), Obj::Kat(1250)];
    objects.push(Obj::Roll(1500, 280.0, false));
    objects.push(Obj::Don(3000));
    objects.push(Obj::Roll(3500, 560.0, true));
    objects.push(Obj::Kat(6000));
    objects.push(Obj::Swell(6500, 8500));
    objects.push(Obj::Don(9000));
    objects.push(Obj::Swell(9500, 10000));
    objects.extend(run(10500, 125, 8, |k, t| {
        if k % 2 == 0 { Obj::Don(t) } else { Obj::Kat(t) }
    }));
    objects.push(Obj::Roll(11750, 140.0, false));
    objects.push(Obj::Don(12500));
    Chart {
        name: "rolls-and-swells",
        od: 5.0,
        hp: 4.0,
        slider_multiplier: 1.4,
        tick_rate: 4.0,
        timing: vec!["1000,500,4,1,0,100,1,0".into()],
        objects,
    }
}

/// BPM changes, green lines (SV up and down), kiai and a barline omitted.
pub fn sv_and_kiai() -> Chart {
    Chart {
        name: "sv-and-kiai",
        od: 7.0,
        hp: 5.0,
        slider_multiplier: 1.6,
        tick_rate: 1.0,
        timing: vec![
            "800,375,4,1,0,100,1,0".into(),
            "2300,-50,4,1,0,100,0,0".into(),
            "3800,-200,4,1,0,80,0,1".into(),
            "5300,250,3,1,0,90,1,8".into(),
            "6800,-100,3,1,0,70,0,1".into(),
            "8300,-25,3,1,0,100,0,0".into(),
        ],
        objects: {
            let mut v = run(800, 375, 12, |k, t| {
                if k % 3 == 2 { Obj::Kat(t) } else { Obj::Don(t) }
            });
            v.extend(run(5300, 125, 24, |k, t| match k % 4 {
                0 => Obj::BigDon(t),
                1 | 2 => Obj::Kat(t),
                _ => Obj::Don(t),
            }));
            v.push(Obj::Roll(8300, 300.0, false));
            v.extend(run(9500, 250, 6, |_, t| Obj::Kat(t)));
            v
        },
    }
}

/// A long 1/4 stream at 180 BPM: dense enough for presses to fall between notes.
pub fn stream() -> Chart {
    let beat = 60_000.0 / 180.0;
    Chart {
        name: "stream",
        od: 8.0,
        hp: 7.0,
        slider_multiplier: 1.4,
        tick_rate: 1.0,
        timing: vec![format!("1000,{beat},4,1,0,100,1,0")],
        objects: (0..160)
            .map(|k| {
                let t = 1000 + (k as f64 * beat / 4.0).round() as i64;
                if (k / 3) % 2 == 0 {
                    Obj::Don(t)
                } else {
                    Obj::Kat(t)
                }
            })
            .collect(),
    }
}

/// OD 0, HP 0: the widest windows and the gentlest HP.
pub fn low_od() -> Chart {
    Chart {
        name: "low-od",
        od: 0.0,
        hp: 0.0,
        slider_multiplier: 1.0,
        tick_rate: 1.0,
        timing: vec!["2000,600,4,1,0,100,1,0".into()],
        objects: run(2000, 300, 24, |k, t| {
            if k % 2 == 0 { Obj::Don(t) } else { Obj::Kat(t) }
        }),
    }
}

/// OD 10, HP 10: the tightest windows and the harshest HP.
pub fn high_od() -> Chart {
    Chart {
        name: "high-od",
        od: 10.0,
        hp: 10.0,
        slider_multiplier: 2.0,
        tick_rate: 2.0,
        timing: vec!["700,240,4,1,0,100,1,0".into()],
        objects: run(700, 120, 48, |k, t| match k % 6 {
            0 | 3 => Obj::Don(t),
            1 | 4 => Obj::Kat(t),
            2 => Obj::BigKat(t),
            _ => Obj::BigDon(t),
        }),
    }
}

/// Keys: 0 left rim (kat), 1 left centre (don), 2 right centre (don), 3 right rim (kat).
pub const DON_KEYS: [u8; 2] = [1, 2];
pub const KAT_KEYS: [u8; 2] = [0, 3];

/// How the simulated player plays.
#[derive(Clone, Copy, Debug)]
pub enum Player {
    /// Every note right on time, the right colour, big notes with both keys, every roll tick
    /// and swell hit.
    Perfect,
    /// Every press this many ms late (negative: early).
    Offset(f64),
    /// Spread around the note by up to `ms` either way (a fixed pseudo-random sequence).
    Jitter(f64),
    /// Every `n`th note hit with the other colour.
    WrongColour(usize),
    /// Every `n`th note not hit at all.
    Skips(usize),
    /// Big notes hit with one key only.
    OneKeyBigs,
    /// Both colours on every note at once.
    Mash,
    /// Nothing pressed.
    Idle,
    /// Hits only the first half of the notes, then stops.
    GivesUp,
}

impl Player {
    pub fn label(self) -> String {
        match self {
            Player::Perfect => "perfect".into(),
            Player::Offset(ms) => format!("offset {ms:+}ms"),
            Player::Jitter(ms) => format!("jitter +-{ms}ms"),
            Player::WrongColour(n) => format!("wrong colour every {n}"),
            Player::Skips(n) => format!("skips every {n}"),
            Player::OneKeyBigs => "one key on big notes".into(),
            Player::Mash => "mash".into(),
            Player::Idle => "idle".into(),
            Player::GivesUp => "gives up halfway".into(),
        }
    }
}

/// A fixed pseudo-random sequence in -1..1 (no `rand`: the numbers must never change).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

/// The presses `player` makes on `map` (map time, key), in time order.
pub fn presses(map: &Beatmap, player: Player) -> Vec<(f64, u8)> {
    let mut out = Vec::new();
    let mut rng = Lcg(0x5eed);
    let n = map.notes.len();
    for (i, note) in map.notes.iter().enumerate() {
        let shift = match player {
            Player::Offset(ms) => ms,
            Player::Jitter(ms) => rng.next() * ms,
            _ => 0.0,
        };
        let t = note.time + shift;
        match (player, note.kind) {
            (Player::Idle, _) => {}
            (Player::GivesUp, _) if i >= n / 2 => {}
            (Player::Skips(k), _) if i % k == k - 1 => {}
            (_, Kind::Don | Kind::Kat) => {
                let don = (note.kind == Kind::Don)
                    != matches!(player, Player::WrongColour(k) if i % k == k - 1);
                let keys = if don { DON_KEYS } else { KAT_KEYS };
                if matches!(player, Player::Mash) {
                    out.push((t, DON_KEYS[0]));
                    out.push((t, KAT_KEYS[1]));
                } else if note.big && !matches!(player, Player::OneKeyBigs) {
                    out.push((t, keys[0]));
                    out.push((t + 5.0, keys[1]));
                } else {
                    out.push((t, keys[i % 2]));
                }
            }
            (_, Kind::Roll) => {
                let step = note.tick_spacing.max(20.0);
                let mut at = note.time;
                let mut k = 0;
                while at <= note.end_time {
                    out.push((at + shift.clamp(-5.0, 5.0), DON_KEYS[k % 2]));
                    at += step;
                    k += 1;
                }
            }
            (_, Kind::Swell) => {
                let hits = note.required_hits.max(1) as usize + 2;
                let span = (note.end_time - note.time).max(1.0);
                for k in 0..hits {
                    let at = note.time + span * (k as f64 + 0.5) / hits as f64;
                    let keys = if k % 2 == 0 { DON_KEYS } else { KAT_KEYS };
                    out.push((at, keys[k % 2]));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}
