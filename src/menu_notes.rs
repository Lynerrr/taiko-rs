//! What drifts behind the main menu: the song's own notes, from its hardest difficulty, riding
//! winding routes across the background. It's a nod to Multiverse: each route is one of the
//! player's universes, with its colours, note shape and scroll speed, all playing the same map.
//! It stays faint and out of the way of the menu's text: the menu is still what's being looked at.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use crate::beatmap::{self, Kind};
use crate::library::Entry;
use crate::multiverse;
use crate::render::{self, rgba, Canvas};
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
    Route { y: 66.0, waves: [(24.0, 1.1, 0.021), (10.0, 2.3, -0.013)], ms: 3000.0, r: 9.0, alpha: 0.2 },
    Route { y: 300.0, waves: [(40.0, 0.8, -0.017), (14.0, 1.9, 0.011)], ms: 4200.0, r: 11.0, alpha: 0.14 },
    Route { y: 424.0, waves: [(22.0, 1.4, 0.015), (9.0, 2.7, -0.019)], ms: 5200.0, r: 7.0, alpha: 0.17 },
];

/// How far past the end of its route a note carries on, fading (of the route's length).
const PAST: f32 = 0.05;
/// Where the menu's text is: the routes fade across it.
const TEXT_TOP: f32 = 255.0;
const TEXT_BOTTOM: f32 = 395.0;
/// How long the notes take to come in with a new song (ms).
const FADE_IN_MS: f32 = 900.0;

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
        Some(known) => set.iter().zip(known).fold(None, |best: Option<(usize, f32)>, (&i, s)| match best {
            Some((_, b)) if b > s => best,
            _ => Some((i, s)),
        }).map(|(i, _)| i),
        None => set.last().copied(),
    }
}

/// The difficulties of `entry`'s set that play its song, smallest file first: the taiko ones if
/// it has any, so a convert never stands in for them.
pub fn set_of(library: &[Entry], entry: usize) -> Vec<usize> {
    let Some(e) = library.get(entry) else { return Vec::new() };
    let (folder, song): (Option<&Path>, &str) = (e.path.parent(), &e.meta.audio_file);
    let mut set: Vec<usize> = (0..library.len()).filter(|&i| library[i].path.parent() == folder && library[i].meta.audio_file == song).collect();
    if set.iter().any(|&i| library[i].meta.mode == 1) {
        set.retain(|&i| library[i].meta.mode == 1);
    }
    set.sort_by_key(|&i| library[i].size);
    set
}

fn ghosts(path: &Path) -> Vec<Ghost> {
    let Ok(map) = beatmap::load(path) else { return Vec::new() };
    map.notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat) && !n.unhittable)
        .map(|n| Ghost { time: n.time, kat: n.kind == Kind::Kat, big: n.big })
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
    pub fn update(&mut self, entry: Option<usize>, library: &[Entry], stars: &mut Stars, rate: f64) {
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
        Some(View { notes: &self.notes, time, fade: render::progress(at, FADE_IN_MS) })
    }
}

/// What the background draws in a frame.
pub struct View<'a> {
    notes: &'a [Ghost],
    time: f64,
    fade: f32,
}

/// The routes and what's on them, under the menu. `clock` is the menu's (ms), `kiai` and `kick`
/// how far into a kiai and what's left of its beat, and `shift` moves the universes along the
/// routes, as a kiai does every few bars.
pub fn draw(c: &mut Canvas, view: &View, clock: f32, kiai: f32, kick: f32, shift: usize) {
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
        let from = view.notes.partition_point(|n| n.time < view.time - ms * PAST as f64);
        // Furthest first, so the nearer ones lie on top.
        let until = from + view.notes[from..].partition_point(|n| n.time <= view.time + ms);
        for note in view.notes[from..until].iter().rev() {
            let along = ((note.time - view.time) / ms) as f32;
            let [x, y] = route_point(route, along, secs, width, swell);
            let ends = ((along + PAST) / PAST).min((1.0 - along) * 8.0).clamp(0.0, 1.0);
            // Over the menu's text they all but go.
            let clear = if (TEXT_TOP..TEXT_BOTTOM).contains(&y) { ((x - 300.0) / 180.0).clamp(0.2, 1.0) } else { 1.0 };
            let col = if note.kat { universe.kat() } else { universe.don() };
            let r = route.r * if note.big { 1.45 } else { 1.0 } + kick * 1.5;
            multiverse::note(c, universe.shape, x, y, r, col, route.alpha * bright * ends * clear);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beatmap::Meta;
    use std::path::PathBuf;

    fn map(set: &str, name: &str, mode: u8, size: u64, audio: &str) -> Entry {
        let meta = Meta { version: name.into(), mode, audio_file: audio.into(), ..Default::default() };
        let mut e = Entry::new(PathBuf::from(format!("/nope/{set}/{name}.osu")), meta);
        e.size = size;
        e
    }

    #[test]
    fn the_hardest_is_the_most_stars_and_the_biggest_file_until_they_are_known() {
        let stars = |i: usize| [Some(1.5), Some(6.2), Some(4.0)][i];
        assert_eq!(hardest(&[0, 1, 2], &stars), Some(1), "not the last: the most stars");
        assert_eq!(hardest(&[0, 1, 2], &|i| if i == 1 { None } else { stars(i) }), Some(2), "one unknown");
        assert_eq!(hardest(&[0, 1, 2], &|_| Some(3.0)), Some(2), "the last of equals");
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
        let converts = vec![map("c", "easy", 0, 1, "s.ogg"), map("c", "hard", 0, 2, "s.ogg")];
        assert_eq!(set_of(&converts, 0), vec![0, 1], "only converts: those will do");
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
            text += &format!("256,192,{},1,{},0:0:0:0:\n", 1000 + k * 150, if k % 2 == 0 { 0 } else { 2 });
        }
        text += &"// padding\n".repeat(padding / 11);
        let path = dir.join(format!("{name}.osu"));
        std::fs::write(&path, text).unwrap();
        let meta = crate::beatmap::Meta { version: name.into(), mode: 1, audio_file: "a.mp3".into(), ..Default::default() };
        Entry::new(path, meta)
    }

    #[test]
    fn the_menu_shows_the_set_s_hardest_even_when_a_kantan_is_playing_and_its_file_is_bigger() {
        let dir = std::env::temp_dir().join(format!("taiko-menu-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = vec![osu(&dir, "kantan", 8, 200_000), osu(&dir, "oni", 400, 0)];
        assert!(library[0].size > library[1].size, "the file size alone would pick the kantan");
        let (mut notes, mut stars) = (Notes::default(), Stars::default());
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
        let ghosts: Vec<Ghost> = (0..2000).map(|k| Ghost { time: k as f64 * 100.0, kat: k % 3 == 0, big: k % 7 == 0 }).collect();
        let shapes = |time: f64| {
            let mut c = Canvas { width: 853.3, ..Default::default() };
            draw(&mut c, &View { notes: &ghosts, time, fade: 1.0 }, 5000.0, 1.0, 0.5, 3);
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
