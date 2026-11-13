//! The song list kept up to date while the game runs: maps saved in the editor and sets put into
//! a songs folder by hand join it without a restart. The songs folders are scanned again in the
//! background; a set folder whose time hasn't changed comes from the scan's cache, so only new
//! and rewritten sets are read (the editor saves through a rename, which touches its folder).

use crate::*;

/// Coming back to the window looks again at most this often.
const FOCUS_EVERY: Duration = Duration::from_secs(5);

impl State {
    /// Looks at the songs folders again in the background. `asked`: the player asked for it, and
    /// is told what came of it.
    pub(crate) fn refresh_library(&mut self, asked: bool) {
        if self.scan.is_some() {
            // One is running already: what it finds is what this would.
            if asked {
                match &mut self.refreshing {
                    Some(told) => *told = true,
                    None => self.toast(
                        tr!(
                            "the songs folders are still being scanned",
                            "曲フォルダをまだスキャン中です",
                            "папки с песнями ещё сканируются"
                        )
                        .to_string(),
                    ),
                }
            }
            return;
        }
        if self.song_dirs.is_empty() {
            if asked {
                self.toast(
                    tr!(
                        "there's no songs folder to look in",
                        "探す曲フォルダがありません",
                        "нет папки с песнями, где искать"
                    )
                    .to_string(),
                );
            }
            return;
        }
        if asked {
            self.toast(
                tr!(
                    "looking for new and changed maps...",
                    "新しい譜面と変更された譜面を探しています...",
                    "ищу новые и изменённые карты..."
                )
                .to_string(),
            );
        }
        self.scan = Some(library::start_scan(self.song_dirs.clone()));
        self.scan_checking = true;
        self.refreshing = Some(asked);
        self.refreshed_at = Some(Instant::now());
    }

    /// The window is in front again: sets may have been put into a songs folder meanwhile.
    pub(crate) fn focus_back(&mut self, now: Instant) {
        if !self.scene.is_select()
            || self
                .refreshed_at
                .is_some_and(|t| now.saturating_duration_since(t) < FOCUS_EVERY)
        {
            return;
        }
        self.refresh_library(false);
    }

    /// Back from the editor: its maps are read again at once (what was saved a moment ago
    /// included), then the songs folders in the background, which bring in new maps.
    pub(crate) fn left_editor_folder(&mut self, dir: &Path) {
        let (mut changed, mut gone) = (Vec::new(), Vec::new());
        for e in self.library.iter().filter(|e| e.path.parent() == Some(dir)) {
            if !e.path.is_file() {
                gone.push(e.path.clone());
            } else if library::file_stats(&e.path) != (e.size, e.modified) {
                changed.push(e.path.clone());
            }
        }
        if !changed.is_empty() || !gone.is_empty() {
            self.folder_changed(folder_watch::Changes { changed, gone });
        }
        self.folder_watch = None;
        self.refresh_library(false);
    }

    /// Library maps rewritten in place (index, path, modified, size): their star ratings and
    /// map info are worked out again.
    pub(crate) fn maps_rewritten(&mut self, changed: Vec<(usize, PathBuf, u64, u64)>) {
        if changed.is_empty() {
            return;
        }
        // Ratings are kept by library index: the old ones would stay with the new map.
        self.stars.clear();
        if let Some(info) = &self.mapinfo {
            info.extend(changed);
            self.mapinfo_sorted = false;
        }
    }

    /// What a look again found, said when it matters: new maps always, the rest when asked.
    pub(crate) fn report_refresh(
        &mut self,
        refreshing: Option<bool>,
        added: usize,
        changed: usize,
        gone: usize,
    ) {
        let Some(asked) = refreshing else {
            return;
        };
        if added + changed + gone == 0 {
            if asked {
                self.toast(
                    tr!(
                        "the song list is up to date",
                        "曲リストは最新です",
                        "список песен актуален"
                    )
                    .to_string(),
                );
            }
            return;
        }
        if !asked && added == 0 {
            return;
        }
        let mut parts = Vec::new();
        if added > 0 {
            parts.push(tr!("{added} new", "新規 {added}", "новых: {added}"; [] [added]));
        }
        if changed > 0 {
            parts.push(
                tr!("{changed} changed", "変更 {changed}", "изменённых: {changed}"; [] [changed]),
            );
        }
        if gone > 0 {
            parts.push(tr!("{gone} gone", "削除 {gone}", "удалённых: {gone}"; [] [gone]));
        }
        let parts = parts.join(", ");
        self.toast(
            tr!("song list: {parts}", "曲リスト: {parts}", "список песен: {parts}"; [] [parts]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osu(title: &str, od: u32) -> String {
        format!(
            "osu file format v14\n\n[General]\nMode: 1\n\n[Metadata]\nTitle:{title}\nVersion:Oni\n\n[Difficulty]\nOverallDifficulty:{od}\n"
        )
    }

    /// Song select over an empty list, its songs folder a fresh one of its own.
    fn menus(name: &str) -> (State, PathBuf) {
        let songs = std::env::temp_dir()
            .join(format!("taiko-rs-refresh-{name}-{}", std::process::id()))
            .join("Songs");
        let _ = std::fs::remove_dir_all(&songs);
        std::fs::create_dir_all(&songs).unwrap();
        let mut s = crate::layout_tests::state();
        s.play_when_scanned = false;
        // The test state's own list first, then none.
        settle(&mut s);
        s.use_library(Vec::new(), None);
        s.scene = SongSelect::scene();
        s.song_dirs = vec![songs.clone()];
        (s, songs)
    }

    fn put(songs: &Path, set: &str, od: u32) -> PathBuf {
        std::fs::create_dir_all(songs.join(set)).unwrap();
        let path = songs.join(set).join("oni.osu");
        std::fs::write(&path, osu(set, od)).unwrap();
        path
    }

    fn settle(s: &mut State) {
        let until = Instant::now() + Duration::from_secs(20);
        while s.scan.is_some() && Instant::now() < until {
            s.poll_scan();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(s.scan.is_none(), "the scan ends");
    }

    fn titles(s: &State) -> Vec<String> {
        s.library.iter().map(|e| e.meta.title.clone()).collect()
    }

    fn toast(s: &State) -> String {
        s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
    }

    #[test]
    fn sets_put_into_the_songs_folder_join_when_the_window_comes_back() {
        let (mut s, songs) = menus("focus");
        put(&songs, "1 A", 5);
        let now = Instant::now();
        s.focus_back(now);
        settle(&mut s);
        assert_eq!(titles(&s), ["1 A"]);
        assert_eq!(toast(&s), "song list: 1 new");
        // Put in by hand while the game was behind another window.
        put(&songs, "2 B", 5);
        s.focus_back(now + Duration::from_secs(1));
        assert!(s.scan.is_none(), "not looked at again straight away");
        s.focus_back(now + FOCUS_EVERY * 2);
        settle(&mut s);
        assert_eq!(titles(&s), ["1 A", "2 B"], "the new one joins the end");
        // Only song select looks: a song being played isn't disturbed.
        put(&songs, "3 C", 5);
        s.scene = crate::screens::settings::SettingsScreen::scene();
        s.focus_back(now + FOCUS_EVERY * 4);
        assert!(s.scan.is_none());
    }

    #[test]
    fn a_map_saved_in_the_editor_shows_its_new_settings_on_leaving_it() {
        let (mut s, songs) = menus("editor");
        let map = put(&songs, "1 A", 5);
        s.refresh_library(false);
        settle(&mut s);
        assert_eq!(s.library[0].meta.od, 5.0);
        // Saved a moment before leaving: no wait for the folder watch.
        std::fs::write(&map, osu("1 A", 10)).unwrap();
        s.left_editor_folder(map.parent().unwrap());
        assert_eq!(s.library[0].meta.od, 10.0, "read again at once");
        settle(&mut s);
        assert_eq!(s.library.len(), 1);
        assert_eq!(s.library[0].meta.od, 10.0);
    }

    #[test]
    fn asking_says_what_changed_and_a_folder_that_cant_be_read_keeps_the_list() {
        let (mut s, songs) = menus("asked");
        put(&songs, "1 A", 5);
        put(&songs, "2 B", 5);
        s.refresh_library(true);
        assert_eq!(toast(&s), "looking for new and changed maps...");
        settle(&mut s);
        assert_eq!(toast(&s), "song list: 2 new");
        s.refresh_library(true);
        settle(&mut s);
        assert_eq!(toast(&s), "the song list is up to date");
        std::fs::remove_dir_all(songs.join("2 B")).unwrap();
        s.refresh_library(true);
        settle(&mut s);
        assert_eq!(toast(&s), "song list: 1 gone");
        assert_eq!(titles(&s), ["1 A"]);
        // The folder unplugged: the list stays as it was.
        let away = songs.with_file_name("Songs away");
        std::fs::rename(&songs, &away).unwrap();
        s.refresh_library(true);
        settle(&mut s);
        assert_eq!(titles(&s), ["1 A"]);
        assert_eq!(toast(&s), "the songs folder can't be read just now");
        std::fs::rename(&away, &songs).unwrap();
    }

    #[test]
    fn looking_for_new_maps_has_no_key_until_the_player_gives_it_one() {
        use crate::keys::{Chord, Hotkey};
        let (mut s, songs) = menus("hotkey");
        put(&songs, "1 A", 5);
        assert_eq!(s.cfg.hotkeys.get(Hotkey::RefreshSongs), None);
        // Neither F5 (a preset key) nor Ctrl+R (restart) looks.
        s.key(KeyCode::F5, false, None, Instant::now(), None);
        s.modifiers_ctrl = true;
        s.key(KeyCode::KeyR, false, None, Instant::now(), None);
        s.modifiers_ctrl = false;
        assert!(s.scan.is_none());
        assert!(!s.cfg.hotkeys.config_lines().contains("refresh_songs"));
        s.cfg
            .hotkeys
            .set(Hotkey::RefreshSongs, Some(Chord::plain(KeyCode::F4)));
        s.key(KeyCode::F4, false, None, Instant::now(), None);
        assert!(s.scan.is_some(), "the key the player gave it looks");
        settle(&mut s);
        assert_eq!(titles(&s), ["1 A"]);
        assert!(
            s.cfg
                .hotkeys
                .config_lines()
                .contains("hotkey.refresh_songs = F4")
        );
    }

    /// The hotkeys' list in settings: each group's heading over the first hotkey of its place,
    /// however many hotkeys come before it.
    #[test]
    fn the_hotkey_headings_in_settings_stay_over_their_groups() {
        use crate::keys::{Hotkey, Place};
        use crate::settings::{Item, group_heading};
        let first = |place: &[Place]| {
            Hotkey::ALL
                .iter()
                .position(|h| h.places() == place)
                .unwrap()
        };
        crate::lang::with(crate::lang::Lang::En, || {
            assert_eq!(group_heading(Item::Hotkey(0)), Some("In a song"));
            assert_eq!(
                group_heading(Item::Hotkey(first(&[Place::Select]))),
                Some("Song select")
            );
            assert_eq!(
                group_heading(Item::Hotkey(first(&[Place::Results]))),
                Some("Results")
            );
            let anywhere = Hotkey::ALL
                .iter()
                .position(|h| h.places().len() == 5)
                .unwrap();
            assert_eq!(group_heading(Item::Hotkey(anywhere)), Some("Anywhere"));
        });
        assert!(
            Hotkey::ALL[first(&[Place::Select])..first(&[Place::Results])]
                .contains(&Hotkey::RefreshSongs)
        );
    }
}
