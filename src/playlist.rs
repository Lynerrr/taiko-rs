//! The music player's list: every song once, however many difficulties it has, and what plays
//! next (the queue, shuffle, repeat).

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::library::Entry;

pub struct Track {
    /// A difficulty of it, in the library.
    pub entry: usize,
    pub label: String,
    /// What the search looks through.
    find: String,
    audio: PathBuf,
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
        let mut tracks: Vec<Track> = library
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let label = format!("{} - {}", e.meta.artist, e.meta.title);
                Track { entry: i, find: label.to_lowercase(), label, audio: audio(e) }
            })
            .collect();
        tracks.sort_by(|a, b| a.find.cmp(&b.find).then(a.audio.cmp(&b.audio)).then(a.entry.cmp(&b.entry)));
        tracks.dedup_by(|a, b| a.audio == b.audio);
        let now = entry.and_then(|i| library.get(i)).map(audio).and_then(|path| tracks.iter().position(|t| t.audio == path)).unwrap_or(0);
        let shown = (0..tracks.len()).collect();
        Playlist { tracks, shown, filter: String::new(), queue: VecDeque::new(), repeat: Repeat::All, now, history: Vec::new(), shuffled: None }
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
        self.shown = (0..self.tracks.len()).filter(|&i| words.iter().all(|w| self.tracks[i].find.contains(w))).collect();
        self.filter = filter;
    }

    /// Plays track `i` now.
    pub fn play(&mut self, i: usize) {
        if i >= self.tracks.len() || i == self.now {
            return;
        }
        self.history.push(self.now);
        self.now = i;
        if let Some((order, at)) = &mut self.shuffled {
            if let Some(pos) = order.iter().position(|&t| t == i) {
                *at = pos;
            }
        }
    }

    pub fn toggle_shuffle(&mut self, mut seed: u64) {
        if self.shuffled.take().is_some() {
            return;
        }
        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, (roll(&mut seed) % (i as u64 + 1)) as usize);
        }
        // What plays now stays the start of it.
        if let Some(pos) = order.iter().position(|&t| t == self.now) {
            order.swap(0, pos);
        }
        self.shuffled = Some((order, 0));
    }

    /// Moves on to what's next: the queue, then the list. False at the end with repeat off.
    pub fn next(&mut self) -> bool {
        let next = if let Some(queued) = self.queue.pop_front() {
            Some(queued)
        } else if let Some((order, at)) = &self.shuffled {
            order.get(at + 1).copied().or(order.first().copied().filter(|_| self.repeat != Repeat::Off))
        } else if self.now + 1 < self.tracks.len() {
            Some(self.now + 1)
        } else {
            Some(0).filter(|_| self.repeat != Repeat::Off && !self.tracks.is_empty())
        };
        let Some(next) = next else { return false };
        self.play(next);
        true
    }

    /// Back to what played before, or up the list.
    pub fn prev(&mut self) {
        let previous = self.history.pop().or_else(|| match &self.shuffled {
            Some((order, at)) => order.get(at.checked_sub(1)?).copied(),
            None => Some((self.now + self.tracks.len()).checked_sub(1)? % self.tracks.len().max(1)),
        });
        if let Some(i) = previous {
            let depth = self.history.len();
            self.play(i);
            self.history.truncate(depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn list(songs: usize) -> Playlist {
        let tracks = (0..songs).map(|i| Track { entry: i, label: format!("song {i}"), find: format!("song {i}"), audio: Path::new("a").join(i.to_string()) }).collect();
        Playlist { tracks, shown: (0..songs).collect(), filter: String::new(), queue: VecDeque::new(), repeat: Repeat::All, now: 0, history: Vec::new(), shuffled: None }
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
}
