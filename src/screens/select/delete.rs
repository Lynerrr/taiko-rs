//! Deleting a map in the song select: one difficulty or its whole set, from the row's
//! right-click menu or the Delete key (Shift+Delete: the whole set). It's asked first, then the
//! files go to the recycle bin (`map_trash`), never for good, and the list goes on without them
//! at once: no rescan. Scores, replays and pp are kept by the map's content, so they stay.

#[allow(unused_imports)]
use crate::*;

use crate::map_trash::{self, Binned, Failure};
use crate::ui::DeleteAsk;

impl State {
    /// Asks whether to delete map `entry` (library index), or its whole set (`set`), unless it
    /// can't be deleted now: then says why.
    pub(crate) fn ask_delete(&mut self, entry: usize, set: bool) {
        let Some(e) = self.library.get(entry) else {
            return;
        };
        // A TJA chart's courses are one file: none goes alone.
        let set = set || e.is_tja();
        let Some(folder) = e.path.parent().map(Path::to_path_buf) else {
            return;
        };
        let song = e.set_key();
        let maps: Vec<usize> = if set {
            self.library
                .iter()
                .enumerate()
                .filter(|(_, m)| m.set_key() == song && !self.select.gone.contains(&m.path))
                .map(|(i, _)| i)
                .collect()
        } else {
            vec![entry]
        };
        if let Some(why) = self.delete_blocker(&maps, &folder) {
            self.toast(why);
            return;
        }
        let e = &self.library[entry];
        // The folder goes with the set, or with its last chart: never a Songs folder itself.
        let set_folder = self.is_set_folder(&folder);
        let others = charts_in(&folder)
            .iter()
            .any(|p| !maps.iter().any(|&i| self.library[i].file() == p));
        let name = if set {
            format!("{} - {}", e.meta.artist, e.meta.title)
        } else {
            format!("{} - {} [{}]", e.meta.artist, e.meta.title, e.meta.version)
        };
        self.select.deleting = Some(DeleteAsk {
            // An osu! set is its folder; a TJA song may share its folder with others.
            whole: set_folder && (!others || (set && !e.is_tja())),
            set,
            osu: in_osu_songs(&e.path),
            system_bin: self.map_bin.is_system(),
            folder,
            name,
            maps,
            on_delete: false,
        });
    }

    /// Why the maps `maps` (library indices) in `folder` can't be deleted now, if they can't.
    fn delete_blocker(&mut self, maps: &[usize], folder: &Path) -> Option<String> {
        let editing = self
            .scene
            .screen_mut::<screens::map_editor::MapEditor>()
            .map(|m| m.view.ed.path.clone());
        let library = &self.library;
        let has = |p: &Path| maps.iter().any(|&i| library[i].path == p);
        if let Some((i, _)) = self.scene.song_play()
            && maps.contains(&i)
        {
            return Some(
                tr!(
                    "can't delete it: it's being played",
                    "削除できません: プレイ中です",
                    "нельзя удалить: на ней идёт игра"
                )
                .into(),
            );
        }
        if editing
            .iter()
            .chain(self.editor_back.as_ref().map(|v| &v.ed.path))
            .any(|p| has(p))
        {
            return Some(
                tr!(
                    "can't delete it: it's open in the editor",
                    "削除できません: エディターで開いています",
                    "нельзя удалить: она открыта в редакторе"
                )
                .into(),
            );
        }
        if let Some(mp) = &self.mp {
            let lobby = mp.map_entry.is_some_and(|i| maps.contains(&i))
                || mp
                    .map
                    .as_ref()
                    .is_some_and(|m| maps.iter().any(|&i| library[i].hash() == m.hash));
            if lobby {
                return Some(
                    tr!(
                        "can't delete it: it's the multiplayer lobby's map",
                        "削除できません: マルチプレイのロビーの譜面です",
                        "нельзя удалить: это карта лобби мультиплеера"
                    )
                    .into(),
                );
            }
        }
        if maps
            .iter()
            .any(|&i| self.select.update_of(&library[i]) == Some(&ui::SetUpdate::Updating))
        {
            return Some(
                tr!(
                    "can't delete it: it's being updated",
                    "削除できません: 更新中です",
                    "нельзя удалить: она обновляется"
                )
                .into(),
            );
        }
        // Read out of an .osz in the Songs folder: it would only be unpacked again.
        if folder.starts_with(library::extracted_dir()) {
            return Some(
                tr!(
                    "can't delete it here: it's read from an .osz file in the Songs folder",
                    "ここでは削除できません: Songsフォルダの.oszファイルから読み込まれています",
                    "здесь не удалить: она читается из файла .osz в папке Songs"
                )
                .into(),
            );
        }
        None
    }

    /// Whether `folder` is a set's own folder: inside a folder of maps being scanned, not one
    /// of them (a map lying loose in the Songs folder has the Songs folder for its folder).
    fn is_set_folder(&self, folder: &Path) -> bool {
        let mut roots: Vec<PathBuf> = self.song_dirs.clone();
        roots.push(self.songs_dir.clone());
        roots.extend(self.cfg.songs.iter().cloned());
        roots.extend(library::song_inputs(&self.cfg));
        // taiko.rs's own maps folder is one too: a map lying loose in it never takes it along.
        roots.push(self.custom_dir.clone());
        // And the TJA folder: a song alone in its folder takes it along, a pack's never goes
        // (`charts_in` finds its other songs).
        roots.extend(self.tja_dir.clone());
        let inside = roots
            .iter()
            .any(|r| folder.starts_with(r) && folder != r.as_path());
        let holds_one = roots.iter().any(|r| r.starts_with(folder));
        inside && !holds_one
    }

    /// Keys while asked: Left / Right (or Tab) move between Cancel and Delete, Enter or Space
    /// does the one picked (Cancel at first), Esc cancels.
    pub(crate) fn delete_ask_key(&mut self, code: KeyCode, repeat: bool) {
        let Some(ask) = &mut self.select.deleting else {
            return;
        };
        if repeat {
            return;
        }
        match code {
            KeyCode::Escape | KeyCode::Backspace => self.select.deleting = None,
            KeyCode::ArrowLeft => ask.on_delete = false,
            KeyCode::ArrowRight => ask.on_delete = true,
            KeyCode::Tab | KeyCode::ArrowUp | KeyCode::ArrowDown => ask.on_delete = !ask.on_delete,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if ask.on_delete {
                    self.confirm_delete();
                } else {
                    self.select.deleting = None;
                }
            }
            _ => {}
        }
    }

    /// The confirmation's Delete: the files go to the recycle bin and the maps leave the list.
    pub(crate) fn confirm_delete(&mut self) {
        let Some(ask) = self.select.deleting.take() else {
            return;
        };
        // Things may have changed while it was asked.
        if let Some(why) = self.delete_blocker(&ask.maps, &ask.folder) {
            self.toast(why);
            return;
        }
        let paths: Vec<PathBuf> = ask
            .maps
            .iter()
            .map(|&i| self.library[i].path.clone())
            .collect();
        let hashes: Vec<u64> = ask.maps.iter().map(|&i| self.library[i].hash()).collect();
        let targets = if ask.whole {
            vec![ask.folder.clone()]
        } else {
            files_to_bin(&paths, &ask.folder)
        };
        let mut own = false;
        let mut failed = None;
        for t in &targets {
            match map_trash::throw(&self.map_bin, t) {
                Ok(Binned::System) => {}
                Ok(Binned::Folder(_)) => own = true,
                // Gone already (deleted by hand meanwhile): it leaves the list all the same.
                Err(Failure::Missing) => {}
                Err(Failure::Stayed(why)) => failed = Some(why),
            }
        }
        let gone: Vec<PathBuf> = paths
            .iter()
            .filter(|p| std::fs::symlink_metadata(tja_maps::file_of(p)).is_err())
            .cloned()
            .collect();
        // A custom set's difficulty: the website still having it doesn't make the set outdated.
        if !ask.whole {
            for (p, &hash) in paths.iter().zip(&hashes) {
                if gone.contains(p) {
                    custom::note_deleted(&ask.folder, hash);
                }
            }
        }
        if !gone.is_empty() {
            self.forget_deleted(&gone, &ask.folder);
        }
        let name = ask.name;
        self.toast(match failed {
            Some(why) => tr!("couldn't delete {name}: {why}", "{name}を削除できませんでした: {why}", "не удалось удалить {name}: {why}"; [] [name, why]),
            None if own => tr!("deleted {name}: in taiko.rs's Trash folder", "{name}を削除しました: taiko.rsのTrashフォルダにあります", "{name} удалена: в папке Trash taiko.rs"; [] [name]),
            None => tr!("deleted {name}: in the recycle bin", "{name}を削除しました: ごみ箱にあります", "{name} удалена: в корзине"; [] [name]),
        });
    }

    /// Maps deleted from disk (`gone`, of the set in `folder`): out of the list, the library
    /// cache and everything that knew them by their path. Scores, replays and pp stay, kept by
    /// the maps' content.
    fn forget_deleted(&mut self, gone: &[PathBuf], folder: &Path) {
        self.select.forget_maps(gone, &self.library);
        let indices: Vec<usize> = self
            .library
            .iter()
            .enumerate()
            .filter(|(_, e)| gone.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        // Collections keep them, as missing (osu! does the same).
        for c in &mut self.select.collections {
            for i in &indices {
                if c.maps.remove(i) {
                    c.missing += 1;
                }
            }
        }
        let paths: Vec<String> = gone
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let recent = self.cfg.editor_recent.len();
        self.cfg.editor_recent.retain(|p| !paths.contains(p));
        if self.cfg.editor_recent.len() != recent {
            self.cfg.save();
        }
        store::with(|s| s.forget_plays(gone));
        // What's kept by the song (its folder's name and music): only once none of it is left.
        let left = |library: &[Entry], gone_too: &std::collections::HashSet<PathBuf>, key: &str| {
            library.iter().any(|e| {
                !gone_too.contains(&e.path)
                    && playlist::favourite_key(e) == key
                    && e.file().is_file()
            })
        };
        let keys: Vec<String> = indices
            .iter()
            .map(|&i| playlist::favourite_key(&self.library[i]))
            .collect();
        let mut liked = false;
        for key in keys {
            if !left(&self.library, &self.select.gone, &key) && self.select.favourites.remove(&key)
            {
                liked = true;
            }
        }
        if liked {
            playlist::save_favourites(&self.select.favourites);
        }
        if !folder.exists() {
            self.select.custom_folder.remove(folder);
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(name) = name
                && self.select.map_skins_off.remove(&name)
            {
                config::save_map_skins_off(&self.select.map_skins_off);
            }
        }
        // The download tab may offer the set again.
        self.installed_sets = None;
        let gone = gone.to_vec();
        let folder = (!folder.exists()).then(|| folder.to_path_buf());
        if cfg!(test) {
            library::forget_cached(&gone, folder.as_deref());
        } else {
            std::thread::spawn(move || library::forget_cached(&gone, folder.as_deref()));
        }
    }
}

/// The chart files in a set's folder as the library reads them: `.osu` and `.tko` in it and
/// in its subfolders, `.tja` at any depth (a pack keeps its songs in folders below).
fn charts_in(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    charts_below(folder, 0, &mut out);
    out
}

/// How many folders down `charts_in` looks for `.tja` files: as deep as the library does.
const TJA_DEPTH: u32 = 12;

fn charts_below(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < TJA_DEPTH {
                charts_below(&p, depth + 1, out);
            }
        } else if (depth <= 1 && library::is_map_file(&p)) || tja_maps::is_tja_file(&p) {
            out.push(p);
        }
    }
}

/// What deleting the maps at `paths` (of the set in `folder`) takes off the disk: each map's
/// file, a TJA chart's once for all its courses, with the music and pictures it names that no
/// chart left in the folder names too (a pack can keep many songs in one folder).
fn files_to_bin(paths: &[PathBuf], folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let file = tja_maps::file_of(p);
        if !files.contains(&file) {
            files.push(file);
        }
    }
    let charts: Vec<PathBuf> = files
        .iter()
        .filter(|f| tja_maps::is_tja_file(f))
        .cloned()
        .collect();
    if charts.is_empty() {
        return files;
    }
    let left: Vec<PathBuf> = charts_in(folder)
        .into_iter()
        .filter(|c| !files.contains(c))
        .collect();
    // An osu! chart beside them may play the same music: then it all stays.
    if left.iter().any(|c| !tja_maps::is_tja_file(c)) {
        return files;
    }
    // Told apart whatever their case: on Windows "Song.ogg" and "song.ogg" are one file.
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    let needed: std::collections::HashSet<String> = left
        .iter()
        .flat_map(|c| tja_maps::files_named(c))
        .map(|p| key(&p))
        .collect();
    for chart in &charts {
        for named in tja_maps::files_named(chart) {
            if named.is_file() && !needed.contains(&key(&named)) && !files.contains(&named) {
                files.push(named);
            }
        }
    }
    files
}

/// Whether the map at `path` is in an osu! install's Songs folder (osu!.db or osu!.exe a few
/// folders up): osu! loses it too.
pub(crate) fn in_osu_songs(path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .take(4)
        .any(|d| d.join("osu!.db").is_file() || d.join("osu!.exe").is_file())
}

/// The confirmation over the song select: what goes, where to, and Cancel (picked at first)
/// beside a red Delete.
pub(crate) fn draw_delete_ask(c: &mut Canvas, ask: &DeleteAsk) {
    c.hits.clear();
    let w = c.width;
    c.rect(0.0, 0.0, w, 480.0, rgba(0x000000, 0.85));
    let bw = (w - 32.0).min(480.0);
    let (bx, inner) = ((w - bw) / 2.0, bw - 32.0);
    let (top, bh) = (128.0, 212.0);
    c.round_rect(
        [bx, top],
        [bw, bh],
        10.0,
        rgba(0x15151d, 0.98),
        rgba(style::DANGER, 0.8),
        1.5,
    );
    let title = if ask.set {
        tr!(
            "Delete this beatmapset?",
            "この譜面セットを削除しますか？",
            "Удалить этот мапсет?"
        )
    } else {
        tr!(
            "Delete this difficulty?",
            "この難易度を削除しますか？",
            "Удалить эту сложность?"
        )
    };
    c.text_centered(
        w / 2.0,
        top + 16.0,
        14.0,
        rgba(0xffffff, 1.0),
        &Canvas::fit(14.0, inner, title),
    );
    let size = Canvas::fit_size(12.0, 7.0, inner, &ask.name);
    c.text_centered(
        w / 2.0,
        top + 50.0,
        size,
        rgba(0xffffff, 1.0),
        &Canvas::fit(size, inner, &ask.name),
    );
    let n = ask.maps.len();
    let what = match (ask.set, ask.whole) {
        (false, false) => tr!(
            "Only its chart file goes: the song's music and background stay.",
            "譜面ファイルだけが消えます。曲の音楽と背景は残ります。",
            "Удаляется только файл сложности: музыка и фон песни остаются."
        )
        .to_string(),
        (false, true) => tr!(
            "It's the set's last difficulty: its whole folder goes, music and background too.",
            "セット最後の難易度です。フォルダごと消えます (音楽と背景も)。",
            "Это последняя сложность: уходит вся папка, с музыкой и фоном."
        )
        .to_string(),
        (true, true) if n == 1 => tr!(
            "Its 1 difficulty goes with the whole folder: music, background and all.",
            "1つの難易度がフォルダごと消えます (音楽と背景も)。",
            "Уходит вся папка с 1 сложностью, музыкой и фоном."
        )
        .to_string(),
        (true, true) => {
            let word = lang::ru_plural(n, ["сложностью", "сложностями", "сложностями"]);
            tr!("Its {n} difficulties go with the whole folder: music, background and all.", "{n}個の難易度がフォルダごと消えます (音楽と背景も)。", "Уходит вся папка с {n} {word}, музыкой и фоном."; [] [n, word])
        }
        (true, false) => {
            let word = lang::ru_plural(n, ["сложность", "сложности", "сложностей"]);
            tr!("Its {n} difficulties go: the folder they're in stays.", "{n}個の難易度が消えます。入っているフォルダは残ります。", "Удаляются {n} {word}: папка, где они лежат, остаётся."; [] [n, word])
        }
    };
    // Shrunk to fit rather than cut: the end says what stays.
    let line = |c: &mut Canvas, y: f32, colour: u32, text: &str| {
        let size = Canvas::fit_size(7.0, 5.0, inner, text);
        c.text_centered(
            w / 2.0,
            y,
            size,
            rgba(colour, 1.0),
            &Canvas::fit(size, inner, text),
        );
    };
    line(c, top + 80.0, 0xc8c8d8, &what);
    let bin = if ask.system_bin {
        tr!(
            "It goes to the recycle bin and can be brought back from there. Scores and replays stay.",
            "ごみ箱に移るので、そこから戻せます。スコアとリプレイは残ります。",
            "Всё уходит в корзину, оттуда можно вернуть. Рекорды и реплеи остаются."
        )
    } else {
        tr!(
            "It goes to taiko.rs's Trash folder and can be brought back from there. Scores and replays stay.",
            "taiko.rsのTrashフォルダに移るので、そこから戻せます。スコアとリプレイは残ります。",
            "Всё уходит в папку Trash taiko.rs, оттуда можно вернуть. Рекорды и реплеи остаются."
        )
    };
    line(c, top + 96.0, 0x9a9ab0, bin);
    if ask.osu {
        line(
            c,
            top + 116.0,
            style::DANGER,
            tr!(
                "It's in osu!'s Songs folder: this removes it from osu! too.",
                "osu!のSongsフォルダにあります。osu!からも消えます。",
                "Она в папке Songs osu!: из osu! она тоже пропадёт."
            ),
        );
    }
    let by = top + 148.0;
    c.button(
        w / 2.0 - 110.0,
        by,
        100.0,
        24.0,
        tr!("Cancel", "キャンセル", "Отмена"),
        9.0,
        hit::Hit::Key(KeyCode::Escape),
        !ask.on_delete,
    );
    style::danger_button(
        c,
        [w / 2.0 + 10.0, by, 100.0, 24.0],
        tr!("Delete", "削除", "Удалить"),
        hit::Hit::DeleteMaps,
        ask.on_delete,
    );
    if !mobile::active() {
        line(
            c,
            top + bh - 16.0,
            0x6f6f86,
            tr!(
                "Left/Right choose   Enter   Esc cancel",
                "左右で選択   Enter   Escでキャンセル",
                "Влево/вправо выбор   Enter   Esc отмена"
            ),
        );
    }
}
