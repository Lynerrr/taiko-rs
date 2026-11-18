//! What drifts behind the main menu: the song's own notes, from its hardest difficulty, riding
//! winding routes across the background. It's a nod to Multiverse: each route is one of the
//! player's universes, with its colours, note shape and scroll speed, all playing the same map.
//! It stays faint and out of the way of the menu's text: the menu is still what's being looked at.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crate::beatmap::{self, Kind};
use crate::hit::Canvas;
use crate::library::Entry;
use crate::multiverse;
use crate::render::{self, rgba};
use crate::stars::Stars;

/// A note as the background needs it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ghost {
    pub time: f64,
    pub kat: bool,
    pub big: bool,
}

/// One route: where it runs, its two waves (height, how many along the screen, how fast they
/// travel), how long a note takes along it, how big and how faint its notes are.
struct Route {
    y: f32,
    waves: [(f32, f32, f32); 2],
    ms: f64,
    r: f32,
    alpha: f32,
}

const ROUTES: [Route; 3] = [
    Route {
        y: 66.0,
        waves: [(24.0, 1.1, 0.021), (10.0, 2.3, -0.013)],
        ms: 3000.0,
        r: 9.0,
        alpha: 0.2,
    },
    Route {
        y: 300.0,
        waves: [(40.0, 0.8, -0.017), (14.0, 1.9, 0.011)],
        ms: 4200.0,
        r: 11.0,
        alpha: 0.14,
    },
    Route {
        y: 424.0,
        waves: [(22.0, 1.4, 0.015), (9.0, 2.7, -0.019)],
        ms: 5200.0,
        r: 7.0,
        alpha: 0.17,
    },
];

/// How far past the end of its route a note carries on, fading (of the route's length).
const PAST: f32 = 0.05;
/// How long the notes take to come in with a new song (ms).
const FADE_IN_MS: f32 = 900.0;

/// The stray gold note's part in the routes (`eggs::StrayNote`): one of the song's own notes,
/// turned gold, riding its route with the rest.
#[derive(Default)]
pub struct Gold {
    /// A note wanted on this route: the next one to come onto it is picked.
    pub want: Option<usize>,
    /// The note picked: its route and map time.
    pub note: Option<(usize, f64)>,
    /// Whether it's drawn gold, rather than hidden (caught, bursting).
    pub shown: bool,
    /// Where it was drawn this frame; None while it isn't on its route.
    pub at: Option<[f32; 2]>,
}

/// A point of route `route`: `along` 0 at its end (left) to 1 at its start, `secs` into the
/// routes' drift, `swell` how much its waves are raised.
fn route_point(route: &Route, along: f32, secs: f32, width: f32, swell: f32) -> [f32; 2] {
    let mut y = route.y;
    for (height, count, speed) in route.waves {
        y += height * swell * (std::f32::consts::TAU * (along * count + secs * speed)).sin();
    }
    [width * (0.06 + along), y]
}

/// The difficulty that stands for `set`: the most stars of all, the last of equals. Until every rating
/// is known, the last, as the biggest file is nearly always the hardest. `set` is lightest first.
pub fn hardest(set: &[usize], stars: &dyn Fn(usize) -> Option<f32>) -> Option<usize> {
    let known: Option<Vec<f32>> = set.iter().map(|&i| stars(i)).collect();
    match known {
        Some(known) => set
            .iter()
            .zip(known)
            .fold(None, |best: Option<(usize, f32)>, (&i, s)| match best {
                Some((_, b)) if b > s => best,
                _ => Some((i, s)),
            })
            .map(|(i, _)| i),
        None => set.last().copied(),
    }
}

/// The difficulties of `entry`'s set that play its song, smallest file first: the taiko ones if
/// it has any, so a convert never stands in for them.
pub fn set_of(library: &[Entry], entry: usize) -> Vec<usize> {
    let Some(e) = library.get(entry) else {
        return Vec::new();
    };
    let (folder, song): (Option<&Path>, &str) = (e.path.parent(), &e.meta.audio_file);
    let mut set: Vec<usize> = (0..library.len())
        .filter(|&i| library[i].path.parent() == folder && library[i].meta.audio_file == song)
        .collect();
    if set.iter().any(|&i| library[i].meta.mode == 1) {
        set.retain(|&i| library[i].meta.mode == 1);
    }
    set.sort_by_key(|&i| library[i].size);
    set
}

fn ghosts(path: &Path) -> Vec<Ghost> {
    let Ok(map) = beatmap::load(path) else {
        return Vec::new();
    };
    map.notes
        .iter()
        .filter(|n| n.judged())
        .map(|n| Ghost {
            time: n.time,
            kat: n.kind == Kind::Kat,
            big: n.big,
        })
        .collect()
}

/// The notes of the song on the menu, kept up with what's playing.
#[derive(Default)]
pub struct Notes {
    /// What's playing, its set and which difficulty the notes are (being) read from.
    entry: Option<usize>,
    library_len: usize,
    set: Vec<usize>,
    top: Option<usize>,
    loading: Option<Receiver<Vec<Ghost>>>,
    notes: Vec<Ghost>,
    arrived: Option<Instant>,
}

impl Notes {
    /// Once a frame on the menu. `entry` is what's playing. Asks for the set's star ratings, and
    /// moves on to the hardest as they come in.
    pub fn update(
        &mut self,
        entry: Option<usize>,
        library: &[Entry],
        stars: &mut Stars,
        rate: f64,
    ) {
        if entry != self.entry || library.len() != self.library_len {
            self.entry = entry;
            self.library_len = library.len();
            self.set = entry.map_or(Vec::new(), |i| set_of(library, i));
            self.top = None;
            self.loading = None;
            self.notes.clear();
            self.arrived = None;
        }
        for &i in &self.set {
            stars.request(i, &library[i].path, rate);
        }
        let top = hardest(&self.set, &|i| stars.get(i, rate));
        if top != self.top {
            self.top = top;
            self.loading = top.map(|i| {
                let (tx, rx) = mpsc::channel();
                let path = library[i].path.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(ghosts(&path));
                });
                rx
            });
        }
        if let Some(notes) = self.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.loading = None;
            self.notes = notes;
            self.arrived = self.arrived.or(Some(Instant::now()));
        }
    }

    pub fn entry(&self) -> Option<usize> {
        self.entry
    }

    /// The notes for a frame at map time `time`.
    pub fn view(&self, time: f64) -> Option<View<'_>> {
        let at = self.arrived?;
        Some(View {
            notes: &self.notes,
            time,
            fade: render::progress(at, FADE_IN_MS),
        })
    }
}

/// What the background draws in a frame.
#[derive(Clone, Copy)]
pub struct View<'a> {
    notes: &'a [Ghost],
    time: f64,
    fade: f32,
}

impl<'a> View<'a> {
    /// `notes` at map time `time`, all the way in.
    #[cfg(test)]
    pub fn of(notes: &'a [Ghost], time: f64) -> View<'a> {
        View {
            notes,
            time,
            fade: 1.0,
        }
    }

    /// The same notes `dim` (0 to 1) fainter: the lane's, as the player dims them.
    pub fn dimmed(mut self, dim: f32) -> Self {
        self.fade *= 1.0 - dim.clamp(0.0, 1.0);
        self
    }

    /// The song's position (map ms).
    pub fn time(&self) -> f64 {
        self.time
    }

    /// How far the notes have come in with this song (0 to 1).
    pub fn fade(&self) -> f32 {
        self.fade
    }

    /// The notes from map time `from` to `until` (both ms), in order.
    pub fn between(&self, from: f64, until: f64) -> &[Ghost] {
        let start = self.notes.partition_point(|n| n.time <= from);
        let end = start + self.notes[start..].partition_point(|n| n.time <= until);
        &self.notes[start..end]
    }
}

/// The routes and what's on them, under the menu. `clock` is the menu's (ms), `kiai` and `kick`
/// how far into a kiai and what's left of its beat, and `shift` moves the universes along the
/// routes, as a kiai does every few bars. `gold` picks and draws the stray gold note.
pub fn draw(
    c: &mut Canvas,
    view: &View,
    clock: f32,
    kiai: f32,
    kick: f32,
    shift: usize,
    gold: &mut Gold,
) {
    gold.at = None;
    let universes = multiverse::get();
    let (width, secs) = (c.width, clock / 1000.0);
    let swell = 1.0 + kiai * 0.35 + kick * 0.15;
    let bright = view.fade * (1.0 + kiai * 0.6);
    for (k, route) in ROUTES.iter().enumerate() {
        let universe = universes[(k + shift) % universes.len()];
        // The route itself: a dotted line, barely there.
        const DOTS: usize = 56;
        for dot in 0..DOTS {
            let [x, y] = route_point(route, dot as f32 / DOTS as f32, secs, width, swell);
            c.rect(x, y, 2.0, 2.0, rgba(universe.idle(), 0.09 * bright));
        }
        let ms = route.ms / universe.speed();
        if gold.want == Some(k) {
            // The next note to come on at the route's start, so it rides the whole way.
            let next = view.notes.partition_point(|n| n.time <= view.time + ms);
            if let Some(n) = view.notes.get(next) {
                gold.want = None;
                gold.note = Some((k, n.time));
                gold.shown = true;
            }
        }
        let from = view
            .notes
            .partition_point(|n| n.time < view.time - ms * PAST as f64);
        // Furthest first, so the nearer ones lie on top.
        let until = from + view.notes[from..].partition_point(|n| n.time <= view.time + ms);
        for note in view.notes[from..until].iter().rev() {
            let along = ((note.time - view.time) / ms) as f32;
            let [x, y] = route_point(route, along, secs, width, swell);
            if gold.note == Some((k, note.time)) {
                if gold.shown && along >= 0.0 {
                    gold.at = Some([x, y]);
                    crate::eggs::draw_stray(c, x, y, clock, -1.0, 1.0);
                }
                continue;
            }
            let ends = ((along + PAST) / PAST)
                .min((1.0 - along) * 8.0)
                .clamp(0.0, 1.0);
            let col = if note.kat {
                universe.kat()
            } else {
                universe.don()
            };
            let r = route.r * if note.big { 1.45 } else { 1.0 } + kick * 1.5;
            crate::game::shaped_note(c, universe.shape, x, y, r, col, route.alpha * bright * ends);
        }
    }
}

/// How many more routes song select has behind it than the menu: the same song, more universes.
pub const MORE: usize = 12;
/// How bright those are next to a route of the menu's (they're many: each is fainter).
const MORE_ALPHA: f32 = 0.45;
/// How long the more routes take to clear off on the way back to the menu (ms).
pub const DRAIN_MS: f32 = 700.0;

/// One of song select's routes past the menu's three, and how deep it lies: the deeper, the
/// further its notes slide with the camera.
struct More {
    route: Route,
    depth: f32,
}

/// Song select's routes past the menu's: spread down the screen, the same ones every run.
fn more() -> &'static [More] {
    static MORE_ROUTES: std::sync::OnceLock<Vec<More>> = std::sync::OnceLock::new();
    MORE_ROUTES.get_or_init(|| {
        let mut seed: u32 = 42;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        (0..MORE)
            .map(|i| {
                let y = 24.0 + ((i as f32 * 0.618 + next() * 0.3) % 1.0) * 430.0;
                let waves = [
                    (
                        10.0 + next() * 34.0,
                        0.6 + next() * 1.6,
                        (next() - 0.5) * 0.05,
                    ),
                    (
                        4.0 + next() * 12.0,
                        1.5 + next() * 1.8,
                        (next() - 0.5) * 0.04,
                    ),
                ];
                let ms = 2200.0 + f64::from(next()) * 4200.0;
                let (r, alpha) = (4.0 + next() * 9.0, 0.1 + next() * 0.22);
                More {
                    route: Route {
                        y,
                        waves,
                        ms,
                        r,
                        alpha,
                    },
                    depth: 0.3 + next() * 1.2,
                }
            })
            .collect()
    })
}

/// How song select's own routes stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crowd {
    /// How far they've come in (0 to 1).
    pub shown: f32,
    /// How far the camera is towards song select (0 on the menu to 1): deeper routes slide
    /// further with it.
    pub cam: f32,
    /// Leaving for the menu: the song's position then (map ms), and how far they've cleared
    /// off since (0 to 1). No more notes come on; the ones there speed off to the right.
    pub drain: Option<(f64, f32)>,
}

/// Song select's routes past the menu's three (`MORE` of them), as `crowd` has them. `clock` and
/// `shift` as for `draw`.
pub fn draw_more(c: &mut Canvas, view: &View, clock: f32, shift: usize, crowd: Crowd) {
    let universes = multiverse::get();
    let (width, secs) = (c.width, clock / 1000.0);
    let shown = crowd.shown.clamp(0.0, 1.0);
    if shown <= 0.0 {
        return;
    }
    for (k, more) in more().iter().enumerate() {
        let route = &more.route;
        let universe = universes[(k + 3 + shift) % universes.len()];
        let ms = route.ms / universe.speed();
        let slide = crowd.cam * more.depth * 1.4;
        // Clearing off: pushed right, faster and faster, and fading.
        let (push, left, from, until) = match crowd.drain {
            Some((at, e)) => {
                if e >= 1.0 {
                    return;
                }
                let push = 1.15 * e.powf(1.6) * (1.0 + more.depth);
                // Only what was on the route when it began: nothing new comes on.
                let first = at + f64::from(slide - PAST) * ms;
                let last = at + f64::from(1.0 + slide) * ms;
                (push, 1.0 - e, first, last)
            }
            None => (0.0, 1.0, f64::MIN, f64::MAX),
        };
        let bright = view.fade * shown * left;
        const DOTS: usize = 56;
        for dot in 0..DOTS {
            let [x, y] = route_point(route, dot as f32 / DOTS as f32, secs, width, 1.0);
            c.rect(x, y, 2.0, 2.0, rgba(universe.idle(), 0.09 * bright));
        }
        let from = from.max(view.time + f64::from(slide - push - PAST) * ms);
        let until = until.min(view.time + f64::from(1.0 + slide - push) * ms);
        if until <= from {
            continue;
        }
        let start = view.notes.partition_point(|n| n.time < from);
        let end = start + view.notes[start..].partition_point(|n| n.time <= until);
        for note in view.notes[start..end].iter().rev() {
            let along = ((note.time - view.time) / ms) as f32 - slide + push;
            let [x, y] = route_point(route, along, secs, width, 1.0);
            let ends = ((along + PAST) / PAST)
                .min((1.0 - along) * 8.0)
                .clamp(0.0, 1.0);
            let col = if note.kat {
                universe.kat()
            } else {
                universe.don()
            };
            let r = route.r * if note.big { 1.45 } else { 1.0 };
            let a = route.alpha * MORE_ALPHA * view.fade * shown * ends;
            crate::game::shaped_note(c, universe.shape, x, y, r, col, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beatmap::Meta;
    use std::path::PathBuf;

    fn map(set: &str, name: &str, mode: u8, size: u64, audio: &str) -> Entry {
        let meta = Meta {
            version: name.into(),
            mode,
            audio_file: audio.into(),
            ..Default::default()
        };
        let mut e = Entry::new(PathBuf::from(format!("/nope/{set}/{name}.osu")), meta);
        e.size = size;
        e
    }

    #[test]
    fn the_hardest_is_the_most_stars_and_the_biggest_file_until_they_are_known() {
        let stars = |i: usize| [Some(1.5), Some(6.2), Some(4.0)][i];
        assert_eq!(
            hardest(&[0, 1, 2], &stars),
            Some(1),
            "not the last: the most stars"
        );
        assert_eq!(
            hardest(&[0, 1, 2], &|i| if i == 1 { None } else { stars(i) }),
            Some(2),
            "one unknown"
        );
        assert_eq!(
            hardest(&[0, 1, 2], &|_| Some(3.0)),
            Some(2),
            "the last of equals"
        );
        assert_eq!(hardest(&[], &stars), None);
    }

    #[test]
    fn a_set_is_its_taiko_difficulties_of_the_song_lightest_first() {
        let library = vec![
            map("a", "oni", 1, 900, "song.mp3"),
            map("a", "kantan", 1, 100, "song.mp3"),
            map("a", "convert", 0, 5000, "song.mp3"),
            map("a", "other song", 1, 9000, "tv size.mp3"),
            map("b", "elsewhere", 1, 1, "song.mp3"),
        ];
        assert_eq!(set_of(&library, 0), vec![1, 0]);
        assert_eq!(set_of(&library, 1), vec![1, 0], "whichever one is playing");
        assert_eq!(set_of(&library, 3), vec![3]);
        assert_eq!(set_of(&library, 9), Vec::<usize>::new());
        let converts = vec![
            map("c", "easy", 0, 1, "s.ogg"),
            map("c", "hard", 0, 2, "s.ogg"),
        ];
        assert_eq!(
            set_of(&converts, 0),
            vec![0, 1],
            "only converts: those will do"
        );
    }

    #[test]
    fn song_selects_routes_are_the_same_every_run_and_stay_faint() {
        let a = more();
        assert_eq!(a.len(), MORE);
        assert!(std::ptr::eq(a, more()));
        for m in a {
            assert!(m.route.y > 0.0 && m.route.y < 480.0, "on the screen");
            assert!(
                m.route.alpha * MORE_ALPHA < 0.16,
                "fainter than the menu's own"
            );
            assert!(m.route.ms > 1000.0 && m.depth > 0.0);
        }
    }

    #[test]
    fn leaving_song_select_clears_its_routes_off_to_the_right_and_brings_none_on() {
        let notes: Vec<Ghost> = (0..400)
            .map(|i| Ghost {
                time: f64::from(i) * 100.0,
                kat: i % 2 == 0,
                big: false,
            })
            .collect();
        let draw = |time: f64, crowd: Crowd| {
            let mut c = Canvas {
                width: 853.3,
                ..Default::default()
            };
            draw_more(&mut c, &View::of(&notes, time), 0.0, 0, crowd);
            c
        };
        let still = Crowd {
            shown: 1.0,
            cam: 1.0,
            drain: None,
        };
        let full = draw(10_000.0, still);
        assert!(
            full.shapes.len() > MORE * 56,
            "every route with notes on it"
        );
        let none = draw(
            10_000.0,
            Crowd {
                shown: 0.0,
                ..still
            },
        );
        assert!(none.shapes.is_empty(), "not shown: nothing");
        // Half way through clearing off, at the same moment: fewer notes, none new (later).
        let leaving = |e: f32| Crowd {
            drain: Some((10_000.0, e)),
            ..still
        };
        let half = draw(10_000.0, leaving(0.5));
        assert!(half.shapes.len() < full.shapes.len());
        // Later in the song the routes would be full of other notes; draining, they stay empty.
        let later = draw(16_000.0, leaving(0.9));
        let count = |c: &Canvas| c.shapes.len();
        assert!(count(&later) < count(&draw(16_000.0, still)));
        assert!(
            draw(10_000.0, leaving(1.0)).shapes.is_empty(),
            "gone at the end"
        );
    }

    #[test]
    fn routes_stay_in_their_bands_and_keep_off_the_middle_of_the_lane() {
        for route in &ROUTES {
            let reach: f32 = route.waves.iter().map(|w| w.0).sum::<f32>() * 1.5;
            for step in 0..200 {
                let [x, y] = route_point(route, step as f32 / 200.0, step as f32 * 0.7, 853.3, 1.5);
                assert!((y - route.y).abs() <= reach + 0.01);
                assert!(x >= 0.0);
            }
            assert!(route.alpha * 1.6 < 0.35, "faint, even at its brightest");
        }
    }

    /// A map of `notes` notes, 150 ms apart, in a file padded out to about `padding` bytes.
    fn osu(dir: &Path, name: &str, notes: usize, padding: usize) -> Entry {
        let mut text = format!(
            "osu file format v14\n\n[General]\nAudioFilename: a.mp3\nMode: 1\n\n[Metadata]\nTitle:t\nArtist:a\nCreator:c\nVersion:{name}\n\n\
             [Difficulty]\nHPDrainRate:5\nCircleSize:5\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n\
             [TimingPoints]\n0,300,4,2,0,100,1,0\n\n[HitObjects]\n"
        );
        for k in 0..notes {
            text += &format!(
                "256,192,{},1,{},0:0:0:0:\n",
                1000 + k * 150,
                if k % 2 == 0 { 0 } else { 2 }
            );
        }
        text += &"// padding\n".repeat(padding / 11);
        let path = dir.join(format!("{name}.osu"));
        std::fs::write(&path, text).unwrap();
        let meta = crate::beatmap::Meta {
            version: name.into(),
            mode: 1,
            audio_file: "a.mp3".into(),
            ..Default::default()
        };
        Entry::new(path, meta)
    }

    #[test]
    fn the_menu_shows_the_set_s_hardest_even_when_a_kantan_is_playing_and_its_file_is_bigger() {
        let dir = std::env::temp_dir().join(format!("taiko-menu-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = vec![osu(&dir, "kantan", 8, 200_000), osu(&dir, "oni", 400, 0)];
        assert!(
            library[0].size > library[1].size,
            "the file size alone would pick the kantan"
        );
        let (mut notes, mut stars) = (Notes::default(), Stars::new(crate::stars::read));
        for _ in 0..500 {
            stars.poll();
            notes.update(Some(0), &library, &mut stars, 1.0);
            if notes.notes.len() == 400 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(notes.notes.len(), 400, "the oni's notes, not the kantan's");
        assert_eq!(notes.notes.iter().filter(|n| n.kat).count(), 200);
        assert_eq!(notes.entry(), Some(0), "and what plays stays what it was");
    }

    #[test]
    fn the_routes_draw_what_is_coming_and_nothing_once_the_notes_have_gone() {
        let ghosts: Vec<Ghost> = (0..2000)
            .map(|k| Ghost {
                time: k as f64 * 100.0,
                kat: k % 3 == 0,
                big: k % 7 == 0,
            })
            .collect();
        let shapes = |time: f64| {
            let mut c = Canvas {
                width: 853.3,
                ..Default::default()
            };
            draw(
                &mut c,
                &View {
                    notes: &ghosts,
                    time,
                    fade: 1.0,
                },
                5000.0,
                1.0,
                0.5,
                3,
                &mut Gold::default(),
            );
            c.shapes.len()
        };
        let idle = shapes(1e9);
        assert!(idle > 0, "the routes themselves");
        assert!(shapes(50_000.0) > idle, "and the notes on them");
    }

    #[test]
    fn the_universes_each_get_their_own_pace() {
        assert!(ROUTES.windows(2).all(|r| r[0].ms != r[1].ms));
    }
}
