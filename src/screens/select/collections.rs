//! Collections in the song select: the list of them (Ctrl+L, the "Collection" button), putting
//! the selected difficulty or song in one (its right-click menu, a long press on a phone), and
//! making, renaming, moving, deleting and copying them.

#[allow(unused_imports)]
use crate::*;

use crate::collections::{self, BadName, Finder, MapRef, Own};
use crate::ui::{NameFor, Naming, PickList, Shown};

impl State {
    /// Lists the collections (with osu!'s brought in again first, once `db` is read) and the
    /// library's maps in each.
    pub(crate) fn show_collections(&mut self, db: Option<&osudb::OsuDb>) {
        let now = unix_now();
        if let Some(db) = db
            && store::with(|s| collections::mirror_osu(s, db, now)) == Some(true)
        {
            eprintln!("collection.db: osu!'s collections brought in again");
        }
        match store::with(|s| s.collections()) {
            Some(all) => {
                self.collections = all;
                self.collections_read = true;
            }
            // Without the database: the player's own from this run, and osu!'s as they are now.
            None => {
                let osu = match db {
                    Some(db) => collections::from_osu(db),
                    None => self
                        .collections
                        .iter()
                        .filter(|c| c.source == collections::Source::Osu)
                        .cloned()
                        .collect(),
                };
                self.collections
                    .retain(|c| c.source == collections::Source::Local);
                self.collections.extend(osu);
            }
        }
        self.list_collections();
        eprintln!(
            "{} collections with maps here",
            self.select
                .collections
                .iter()
                .filter(|c| !c.maps.is_empty())
                .count()
        );
    }

    /// Song select's list of `self.collections`: the player's own, then osu!'s with maps here.
    /// The one shown stays shown.
    pub(crate) fn list_collections(&mut self) {
        let shown = self
            .select
            .collection
            .and_then(|k| self.select.collections.get(k))
            .map(|c| (c.name.clone(), c.own));
        // The player's own follow their maps when they're moved or updated.
        let finder = Finder::new(&self.library);
        let own = collections::own_len(&self.collections);
        let mut moved = false;
        for c in &mut self.collections[..own] {
            moved |= collections::repoint(c, &finder, &self.library);
        }
        if moved {
            self.keep_own_collections();
        }
        let found = collections::find(&self.collections, &self.library);
        self.select.collections = self
            .collections
            .iter()
            .zip(found)
            .map(|(c, f)| Shown {
                name: c.name.clone(),
                own: c.source == collections::Source::Local,
                maps: f.entries,
                missing: f.missing,
            })
            .filter(|c| c.own || !c.maps.is_empty())
            .collect();
        self.select.collection = shown.and_then(|(name, own)| {
            self.select
                .collections
                .iter()
                .position(|c| c.name == name && c.own == own)
        });
    }

    /// Where song select's collection `k` is in `self.collections`.
    fn collection_at(&self, k: usize) -> Option<usize> {
        let c = self.select.collections.get(k)?;
        let source = if c.own {
            collections::Source::Local
        } else {
            collections::Source::Osu
        };
        self.collections
            .iter()
            .position(|s| s.source == source && s.name == c.name)
    }

    /// Keeps the player's own collections in `taiko.db`, lists them again, and the songs shown
    /// if the one shown changed.
    fn collections_changed(&mut self) {
        if !self.keep_own_collections() && !cfg!(test) {
            self.toast("collections can't be kept this run: taiko.db couldn't be read".into());
        }
        self.list_collections();
        self.select.refilter(&self.library);
    }

    /// Writes the player's own collections to `taiko.db`; whether they were. Not before they
    /// were read from it: what this run has would replace what's there.
    fn keep_own_collections(&mut self) -> bool {
        if !self.collections_read {
            return false;
        }
        let own = &self.collections[..collections::own_len(&self.collections)];
        let now = unix_now();
        store::with(|s| {
            s.transaction(|s| s.write_collections(collections::Source::Local, own, now))
        })
        .is_some()
    }

    pub(crate) fn open_collections(&mut self) {
        self.select.list = PickList::Collections;
        self.select.picker = Some(self.select.collection.map(|c| c + 1).unwrap_or(0));
    }

    /// From the song's right-click menu: the player's collections to put the menu's difficulty
    /// (or its whole song) in.
    pub(crate) fn start_collecting(&mut self, whole: bool) {
        let Some(i) = self.select.menu_map() else {
            return;
        };
        self.select.menu = None;
        let maps = if whole {
            let dir = self.library[i].path.parent().map(Path::to_path_buf);
            self.library
                .iter()
                .enumerate()
                .filter(|(_, e)| e.path.parent().map(Path::to_path_buf) == dir)
                .map(|(j, _)| j)
                .collect()
        } else {
            vec![i]
        };
        self.select.list = PickList::AddTo(maps);
        self.select.picker = Some(0);
    }

    /// Ctrl+K: the player's collections to put the selected difficulty in.
    pub(crate) fn collect_selected(&mut self) {
        if let Some(i) = self.select.selected() {
            self.select.list = PickList::AddTo(vec![i]);
            self.select.picker = Some(0);
        }
    }

    /// Row `i` of a collections list picked.
    pub(crate) fn pick_collection_row(&mut self, i: usize) {
        let own = self.select.own_collections();
        match self.select.list.clone() {
            PickList::Sorts => {}
            PickList::Collections => {
                if i == self.select.collections.len() + 1 {
                    self.start_naming(NameFor::New(Vec::new()));
                } else {
                    self.select.pick_collection(i, &self.library);
                }
            }
            PickList::AddTo(maps) => {
                if i >= own {
                    self.start_naming(NameFor::New(maps));
                } else {
                    self.select.picker = None;
                    self.collect(i, &maps);
                }
            }
            PickList::Manage(k) => self.manage_collection(k, i),
            PickList::Delete(k) => {
                if i == 0 {
                    self.delete_collection(k);
                } else {
                    self.back_to_collections(k);
                }
            }
        }
    }

    /// What can be done with song select's collection `k` (its "..." or Tab).
    pub(crate) fn open_collection_menu(&mut self, k: usize) {
        if k < self.select.collections.len() {
            self.select.list = PickList::Manage(k);
            self.select.picker = Some(0);
        }
    }

    fn back_to_collections(&mut self, k: usize) {
        self.select.list = PickList::Collections;
        self.select.picker = Some((k + 1).min(self.select.collections.len()));
    }

    /// Row `row` of collection `k`'s own list.
    fn manage_collection(&mut self, k: usize, row: usize) {
        let Some(c) = self.select.collections.get(k) else {
            return;
        };
        if row == self.select.picker_len().saturating_sub(1) && c.missing > 0 {
            self.download_missing(k);
            return;
        }
        if !c.own {
            self.copy_collection(k);
            return;
        }
        match row {
            0 => self.start_naming(NameFor::Rename(k)),
            1 | 2 => self.move_collection(k, row == 1),
            _ => {
                self.select.list = PickList::Delete(k);
                self.select.picker = Some(1);
            }
        }
    }

    fn start_naming(&mut self, then: NameFor) {
        let text = match &then {
            NameFor::Rename(k) => self
                .select
                .collections
                .get(*k)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            NameFor::New(_) => String::new(),
        };
        self.select.picker = None;
        self.select.naming = Some(Naming { text, then });
    }

    /// Keys while a collection's name is typed: Enter keeps it, Esc doesn't.
    pub(crate) fn collection_naming_key(&mut self, code: KeyCode, text: Option<&str>) {
        let Some(naming) = &mut self.select.naming else {
            return;
        };
        match code {
            KeyCode::Escape => {
                let then = self.select.naming.take().map(|n| n.then);
                if let Some(NameFor::Rename(k)) = then {
                    self.open_collection_menu(k);
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let Some(Naming { text, then }) = self.select.naming.take() else {
                    return;
                };
                let done = match then.clone() {
                    NameFor::New(maps) => self.new_collection(&text, &maps),
                    NameFor::Rename(k) => self.rename_collection(k, &text),
                };
                if let Err(e) = done {
                    self.toast(match e {
                        BadName::Empty => "a collection needs a name".into(),
                        BadName::Taken => {
                            format!("there's a collection called {} already", text.trim())
                        }
                    });
                    self.select.naming = Some(Naming { text, then });
                }
            }
            KeyCode::Backspace => {
                naming.text.pop();
            }
            KeyCode::Delete => naming.text.clear(),
            KeyCode::KeyA if self.modifiers_ctrl => naming.text.clear(),
            _ => {
                if let Some(text) = text {
                    let room = collections::MAX_NAME.saturating_sub(naming.text.chars().count());
                    naming
                        .text
                        .extend(text.chars().filter(|c| !c.is_control()).take(room));
                }
            }
        }
    }

    /// Library entries `maps` as a collection keeps them; the ones whose file can't be read are
    /// left out.
    fn map_refs(&self, maps: &[usize]) -> Vec<(usize, MapRef)> {
        maps.iter()
            .filter_map(|&i| Some((i, MapRef::of(self.library.get(i)?)?)))
            .collect()
    }

    fn new_collection(&mut self, name: &str, maps: &[usize]) -> Result<(), BadName> {
        let refs = self.map_refs(maps).into_iter().map(|(_, m)| m).collect();
        Own(&mut self.collections).add(name, refs)?;
        let name = name.trim();
        self.collections_changed();
        self.toast(match maps.len() {
            0 => format!("collection {name} made"),
            1 => format!("put in {name}, a new collection"),
            n => format!("{n} difficulties put in {name}, a new collection"),
        });
        Ok(())
    }

    fn rename_collection(&mut self, k: usize, name: &str) -> Result<(), BadName> {
        let Some(at) = self.collection_at(k) else {
            return Ok(());
        };
        Own(&mut self.collections).rename(at, name)?;
        // The list keeps showing it under its new name.
        if self.select.collection == Some(k) {
            self.select.collections[k].name =
                name.trim().chars().take(collections::MAX_NAME).collect();
        }
        self.collections_changed();
        self.back_to_collections(k);
        Ok(())
    }

    fn move_collection(&mut self, k: usize, up: bool) {
        let Some(at) = self.collection_at(k) else {
            return;
        };
        let to = Own(&mut self.collections).shift(at, up);
        self.collections_changed();
        // The cursor goes with it.
        self.back_to_collections(to);
    }

    fn delete_collection(&mut self, k: usize) {
        let Some(at) = self.collection_at(k) else {
            return;
        };
        let name = self.collections[at].name.clone();
        if self.select.collection == Some(k) {
            self.select.collection = None;
        }
        Own(&mut self.collections).delete(at);
        self.collections_changed();
        self.back_to_collections(k.min(self.select.collections.len()));
        self.toast(format!("collection {name} deleted (its maps stay)"));
    }

    fn copy_collection(&mut self, k: usize) {
        let Some(at) = self.collection_at(k) else {
            return;
        };
        let Some(to) = Own(&mut self.collections).copy(at) else {
            return;
        };
        let name = self.collections[to].name.clone();
        self.collections_changed();
        self.back_to_collections(to);
        self.toast(format!("copied to your collections as {name}"));
    }

    /// Downloads the sets of collection `k`'s maps that aren't here (the ones with a set ID).
    fn download_missing(&mut self, k: usize) {
        let Some(at) = self.collection_at(k) else {
            return;
        };
        let finder = Finder::new(&self.library);
        let sets = collections::missing_sets(&self.collections[at], &finder);
        self.select.picker = None;
        if sets.is_empty() {
            self.toast("the maps not here have no set ID to download them by".into());
            return;
        }
        self.toast(match sets.len() {
            1 => "downloading 1 set".into(),
            n => format!("downloading {n} sets"),
        });
        for (id, folder) in sets {
            // "123 Artist - Title", as osu! names a set's folder.
            let name = folder
                .split_once(' ')
                .filter(|(n, _)| n.parse::<u32>().is_ok())
                .map_or(folder.as_str(), |(_, rest)| rest);
            let (artist, title) = name.split_once(" - ").unwrap_or(("", name));
            self.start_download(download::SetInfo {
                id,
                artist: artist.to_string(),
                title: title.to_string(),
                creator: String::new(),
                status: String::new(),
                diffs: Vec::new(),
                play_count: 0,
                favourite_count: 0,
                rating: 0.0,
                ranked_date: String::new(),
                custom: None,
            });
        }
    }

    /// Puts `maps` in the player's collection `k`, or takes them out if they're all in it.
    fn collect(&mut self, k: usize, maps: &[usize]) {
        let Some(at) = self.collection_at(k) else {
            return;
        };
        let refs = self.map_refs(maps);
        if refs.is_empty() {
            self.toast("the map's file can't be read".into());
            return;
        }
        let finder = Finder::new(&self.library);
        let name = self.collections[at].name.clone();
        let added = Own(&mut self.collections).toggle(at, &refs, &finder);
        self.collections_changed();
        self.toast(if added {
            format!("put in {name}")
        } else {
            format!("taken out of {name}")
        });
    }

    /// Keys on an open collections list (not the sorts).
    pub(crate) fn collections_list_key(&mut self, code: KeyCode, cursor: usize) {
        let row = cursor
            .checked_sub(1)
            .filter(|&k| k < self.select.collections.len());
        match (&self.select.list, code, row) {
            (PickList::Collections, KeyCode::Tab, Some(k)) => self.open_collection_menu(k),
            (PickList::Collections, KeyCode::F2, Some(k)) if self.select.collections[k].own => {
                self.start_naming(NameFor::Rename(k))
            }
            (PickList::Collections, KeyCode::Delete, Some(k)) if self.select.collections[k].own => {
                self.select.list = PickList::Delete(k);
                self.select.picker = Some(1);
            }
            (PickList::Collections, KeyCode::ArrowUp | KeyCode::ArrowDown, Some(k))
                if self.modifiers_ctrl && self.select.collections[k].own =>
            {
                self.move_collection(k, code == KeyCode::ArrowUp)
            }
            (
                PickList::Manage(k) | PickList::Delete(k),
                KeyCode::Escape | KeyCode::Backspace,
                _,
            ) => {
                let k = *k;
                self.back_to_collections(k)
            }
            _ => self.list_key(code, cursor),
        }
    }

    /// Moving through an open list, picking a row and closing it.
    pub(crate) fn list_key(&mut self, code: KeyCode, cursor: usize) {
        let n = self.select.picker_len().max(1);
        match code {
            KeyCode::ArrowUp => self.select.picker = Some(cursor.saturating_sub(1)),
            KeyCode::ArrowDown => self.select.picker = Some((cursor + 1).min(n - 1)),
            KeyCode::PageUp => self.select.picker = Some(cursor.saturating_sub(10)),
            KeyCode::PageDown => self.select.picker = Some((cursor + 10).min(n - 1)),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.pick_row(cursor),
            KeyCode::Escape | KeyCode::Backspace => self.select.picker = None,
            _ => {}
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A collection's name being typed, over the song select.
pub(crate) fn draw_collection_naming(c: &mut Canvas, naming: &Naming) {
    c.hits.clear();
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
    let bw = (w - 32.0).min(460.0);
    let (bx, inner) = ((w - bw) / 2.0, bw - 32.0);
    c.round_rect(
        [bx, 140.0],
        [bw, 164.0],
        10.0,
        rgba(0x15151d, 0.98),
        rgba(accent(), 0.8),
        1.5,
    );
    let (title, note) = match &naming.then {
        NameFor::Rename(_) => ("Rename the collection", "Its maps stay in it."),
        NameFor::New(maps) if maps.is_empty() => (
            "Name the new collection",
            "Put maps in it from a song's right-click menu.",
        ),
        NameFor::New(_) => (
            "Name the new collection",
            "The map goes in it. More from a song's right-click menu.",
        ),
    };
    c.text_centered(
        w / 2.0,
        156.0,
        14.0,
        rgba(0xffffff, 1.0),
        &Canvas::fit(14.0, inner, title),
    );
    let typed = format!("{}_", naming.text);
    let size = Canvas::fit_size(12.0, 7.0, inner, &typed);
    let typed = Canvas::fit(size, inner, &typed);
    c.text_centered(w / 2.0, 196.0, size, rgba(0xffffff, 1.0), &typed);
    c.text_centered(
        w / 2.0,
        230.0,
        7.0,
        rgba(0x9a9ab0, 1.0),
        &Canvas::fit(7.0, inner, note),
    );
    c.button(
        w / 2.0 - 100.0,
        264.0,
        90.0,
        24.0,
        "Save",
        9.0,
        hit::Hit::Key(KeyCode::Enter),
        true,
    );
    c.button(
        w / 2.0 + 10.0,
        264.0,
        90.0,
        24.0,
        "Cancel",
        9.0,
        hit::Hit::Key(KeyCode::Escape),
        false,
    );
}
