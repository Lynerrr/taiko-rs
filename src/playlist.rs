//! The music player's list: every song once, however many difficulties it has, in folders (its
//! genres, the favourites), and what plays next (the queue, shuffle, repeat).

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::library::Entry;

pub struct Track {
    /// A difficulty of it, in the library.
    pub entry: usize,
    pub label: String,
    /// What the search looks through.
    find: String,
    audio: PathBuf,
    /// What favourites know it by: its folder and audio file, wherever Songs is.
    key: String,
    pub genres: u32,
}

/// A way of narrowing the list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Folder {
    All,
    Favourites,
    /// `genres::GENRES[i]`.
    Genre(usize),
    /// Tags that name no genre.
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    /// Round the list.
    All,
    /// This song again.
    One,
    /// Stop at the end of the list.
    Off,
}

impl Repeat {
    fn next(self) -> Repeat {
        match self {
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
            Repeat::Off => Repeat::All,
        }
    }
}

pub struct Playlist {
    pub tracks: Vec<Track>,
    /// The tracks the search leaves, in order.
    pub shown: Vec<usize>,
    pub filter: String,
    pub folder: Folder,
    pub favourites: HashSet<String>,
    /// The tracks of the folder: what plays in turn.
    scope: Vec<usize>,
    pub queue: VecDeque<usize>,
    pub repeat: Repeat,
    /// The track playing.
    now: usize,
    /// Tracks played before it, for going back.
    history: Vec<usize>,
    /// The shuffled order and where in it we are.
    shuffled: Option<(Vec<usize>, usize)>,
}

/// A number from `seed`, which moves on (xorshift).
fn roll(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

impl Playlist {
    /// Every song of `library`, by name, on the one `entry` belongs to.
    pub fn new(library: &[Entry], entry: Option<usize>) -> Playlist {
        let audio = |e: &Entry| e.path.parent().unwrap_or(std::path::Path::new(".")).join(&e.meta.audio_file);
        let key = |e: &Entry| {
            let folder = e.path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("{folder}/{}", e.meta.audio_file)
        };
        let mut tracks: Vec<Track> = library
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let label = format!("{} - {}", e.meta.artist, e.meta.title);
                Track { entry: i, find: label.to_lowercase(), label, audio: audio(e), key: key(e), genres: e.meta.genres }
            })
            .collect();
        tracks.sort_by(|a, b| a.find.cmp(&b.find).then(a.audio.cmp(&b.audio)).then(a.entry.cmp(&b.entry)));
        tracks.dedup_by(|a, b| {
            let same = a.audio == b.audio;
            if same {
                b.genres |= a.genres;
            }
            same
        });
        let now = entry.and_then(|i| library.get(i)).map(audio).and_then(|path| tracks.iter().position(|t| t.audio == path)).unwrap_or(0);
        let all: Vec<usize> = (0..tracks.len()).collect();
        let favourites = load_favourites();
        Playlist { tracks, shown: all.clone(), filter: String::new(), folder: Folder::All, favourites, scope: all, queue: VecDeque::new(), repeat: Repeat::All, now, history: Vec::new(), shuffled: None }
    }

    pub fn is_favourite(&self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|t| self.favourites.contains(&t.key))
    }

    /// How many tracks a folder holds.
    pub fn count(&self, folder: Folder) -> usize {
        (0..self.tracks.len()).filter(|&i| self.holds(folder, i)).count()
    }

    fn holds(&self, folder: Folder, track: usize) -> bool {
        match folder {
            Folder::All => true,
            Folder::Favourites => self.is_favourite(track),
            Folder::Genre(i) => self.tracks[track].genres >> i & 1 != 0,
            Folder::Other => self.tracks[track].genres == 0,
        }
    }

    /// The folders worth showing: everything, the favourites, then the genres there are songs of,
    /// biggest first, and what's left.
    pub fn folders(&self) -> Vec<(Folder, usize)> {
        let mut genres: Vec<(Folder, usize)> = (0..crate::genres::GENRES.len()).map(|i| (Folder::Genre(i), self.count(Folder::Genre(i)))).filter(|f| f.1 > 0).collect();
        genres.sort_by_key(|f| std::cmp::Reverse(f.1));
        let mut all = vec![(Folder::All, self.tracks.len()), (Folder::Favourites, self.count(Folder::Favourites))];
        all.extend(genres);
        let other = self.count(Folder::Other);
        if other > 0 {
            all.push((Folder::Other, other));
        }
        all
    }

    /// Narrows the list to `folder`. What's playing plays on; next comes from the folder.
    pub fn set_folder(&mut self, folder: Folder, seed: u64) {
        self.folder = folder;
        self.refresh(seed);
    }

    fn refresh(&mut self, seed: u64) {
        self.scope = (0..self.tracks.len()).filter(|&i| self.holds(self.folder, i)).collect();
        self.set_filter(self.filter.clone());
        if self.shuffled.take().is_some() {
            self.toggle_shuffle(seed);
        }
    }

    pub fn toggle_favourite(&mut self, track: usize, seed: u64) {
        let Some(key) = self.tracks.get(track).map(|t| t.key.clone()) else { return };
        if !self.favourites.remove(&key) {
            self.favourites.insert(key);
        }
        if self.folder == Folder::Favourites {
            self.refresh(seed);
        }
    }

    pub fn now(&self) -> usize {
        self.now
    }

    pub fn entry(&self) -> Option<usize> {
        self.tracks.get(self.now).map(|t| t.entry)
    }

    pub fn is_shuffled(&self) -> bool {
        self.shuffled.is_some()
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.next();
    }

    pub fn set_filter(&mut self, filter: String) {
        let words: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
        self.shown = self.scope.iter().copied().filter(|&i| words.iter().all(|w| self.tracks[i].find.contains(w))).collect();
        self.filter = filter;
    }

    /// Plays track `i` now.
    pub fn play(&mut self, i: usize) {
        if i >= self.tracks.len() || i == self.now {
            return;
        }
        self.history.push(self.now);
        self.now = i;
        if let Some((order, at)) = &mut self.shuffled
            && let Some(pos) = order.iter().position(|&t| t == i) {
            *at = pos;
        }
    }

    pub fn toggle_shuffle(&mut self, mut seed: u64) {
        if self.shuffled.take().is_some() {
            return;
        }
        let mut order = self.scope.clone();
        for i in (1..order.len()).rev() {
            order.swap(i, (roll(&mut seed) % (i as u64 + 1)) as usize);
        }
        // What plays now stays the start of it.
        match order.iter().position(|&t| t == self.now) {
            Some(pos) => order.swap(0, pos),
            None => order.insert(0, self.now),
        }
        self.shuffled = Some((order, 0));
    }

    /// Moves on to what's next: the queue, then the list. False at the end with repeat off.
    pub fn next(&mut self) -> bool {
        let next = if let Some(queued) = self.queue.pop_front() {
            Some(queued)
        } else if let Some((order, at)) = &self.shuffled {
            order.get(at + 1).copied().or(order.first().copied().filter(|_| self.repeat != Repeat::Off))
        } else {
            // From a song outside the folder, its first.
            let after = self.scope.iter().position(|&t| t == self.now).map_or(0, |pos| pos + 1);
            self.scope.get(after).copied().or(self.scope.first().copied().filter(|_| self.repeat != Repeat::Off))
        };
        let Some(next) = next else { return false };
        self.play(next);
        true
    }

    /// Back to what played before, or up the list.
    pub fn prev(&mut self) {
        let previous = self.history.pop().or_else(|| match &self.shuffled {
            Some((order, at)) => order.get(at.checked_sub(1)?).copied(),
            None => {
                let pos = self.scope.iter().position(|&t| t == self.now).unwrap_or(0);
                self.scope.get(pos.checked_sub(1).unwrap_or(self.scope.len().checked_sub(1)?)).copied()
            }
        });
        if let Some(i) = previous {
            let depth = self.history.len();
            self.play(i);
            self.history.truncate(depth);
        }
    }
}

fn favourites_file() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join("favourite-songs.txt"))
}

fn load_favourites() -> HashSet<String> {
    #[cfg(test)]
    return HashSet::new();
    #[cfg(not(test))]
    favourites_file().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

/// Keeps the favourites, sorted, so the file only changes by what changed.
pub fn save_favourites(favourites: &HashSet<String>) {
    let Some(path) = favourites_file() else { return };
    let mut lines: Vec<&str> = favourites.iter().map(String::as_str).collect();
    lines.sort_unstable();
    let tmp = path.with_extension("partial");
    if std::fs::write(&tmp, lines.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn list(songs: usize) -> Playlist {
        let tracks = (0..songs).map(|i| Track { entry: i, label: format!("song {i}"), find: format!("song {i}"), audio: Path::new("a").join(i.to_string()), key: format!("set/{i}"), genres: 0 }).collect();
        let all: Vec<usize> = (0..songs).collect();
        Playlist { tracks, shown: all.clone(), filter: String::new(), folder: Folder::All, favourites: HashSet::new(), scope: all, queue: VecDeque::new(), repeat: Repeat::All, now: 0, history: Vec::new(), shuffled: None }
    }

    #[test]
    fn it_goes_round_the_list_or_stops_with_repeat_off() {
        let mut l = list(3);
        assert!(l.next() && l.next());
        assert_eq!(l.now(), 2);
        l.repeat = Repeat::Off;
        assert!(!l.next());
        assert_eq!(l.now(), 2);
        l.repeat = Repeat::All;
        assert!(l.next());
        assert_eq!(l.now(), 0);
    }

    #[test]
    fn the_queue_goes_first_and_prev_comes_back() {
        let mut l = list(5);
        l.queue.push_back(3);
        l.next();
        assert_eq!(l.now(), 3);
        assert!(l.queue.is_empty());
        l.next();
        assert_eq!(l.now(), 4);
        l.prev();
        assert_eq!(l.now(), 3);
        l.prev();
        assert_eq!(l.now(), 0);
        l.prev();
        assert_eq!(l.now(), 4, "with nothing behind it, up the list and round");
    }

    #[test]
    fn shuffle_plays_everything_once_starting_from_the_song_on() {
        let mut l = list(50);
        l.play(7);
        l.toggle_shuffle(12345);
        let mut heard = vec![l.now()];
        for _ in 1..50 {
            assert!(l.next());
            heard.push(l.now());
        }
        assert_eq!(heard[0], 7);
        assert_ne!(heard[1..].iter().copied().take(10).collect::<Vec<_>>(), (8..18).collect::<Vec<_>>());
        heard.sort_unstable();
        assert_eq!(heard, (0..50).collect::<Vec<_>>());
        l.repeat = Repeat::Off;
        assert!(!l.next(), "and stops at its end");
    }

    #[test]
    fn the_search_takes_words_in_any_order_and_any_case() {
        let mut l = list(3);
        l.tracks[1].find = "camellia - ghost".into();
        l.set_filter("Ghost CAM".into());
        assert_eq!(l.shown, [1]);
        l.set_filter(String::new());
        assert_eq!(l.shown.len(), 3);
    }

    #[test]
    fn a_folder_plays_in_turn_and_shuffles_inside_itself() {
        let mut l = list(6);
        for t in [1, 3, 5] {
            l.tracks[t].genres = 0b10;
        }
        l.set_folder(Folder::Genre(1), 1);
        assert_eq!(l.shown, [1, 3, 5]);
        assert_eq!(l.now(), 0, "the song on plays on");
        l.next();
        assert_eq!(l.now(), 1, "then the folder's first");
        l.next();
        l.next();
        l.next();
        assert_eq!(l.now(), 1, "and round");
        l.toggle_shuffle(99);
        for _ in 0..20 {
            l.next();
            assert_eq!(l.now() % 2, 1);
        }
        l.set_folder(Folder::Other, 3);
        assert_eq!(l.shown, [0, 2, 4]);
    }

    #[test]
    fn favourites_are_a_folder_and_leave_it_when_unloved() {
        let mut l = list(4);
        l.toggle_favourite(2, 1);
        l.toggle_favourite(3, 1);
        assert_eq!(l.count(Folder::Favourites), 2);
        l.set_folder(Folder::Favourites, 1);
        assert_eq!(l.shown, [2, 3]);
        l.toggle_favourite(2, 1);
        assert_eq!(l.shown, [3]);
        assert!(!l.is_favourite(2) && l.is_favourite(3));
    }

    #[test]
    fn folders_list_the_biggest_genres_first_and_only_those_with_songs() {
        let mut l = list(5);
        l.tracks[0].genres = 0b1;
        for t in 1..4 {
            l.tracks[t].genres = 0b100;
        }
        let folders = l.folders();
        assert_eq!(folders, [(Folder::All, 5), (Folder::Favourites, 0), (Folder::Genre(2), 3), (Folder::Genre(0), 1), (Folder::Other, 1)]);
    }
}
