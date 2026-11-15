//! The background picture: File > Background picture (a file dialog), the row of Song setup,
//! and a picture dropped on the editor, which asks before it becomes the background.

use std::path::Path;
use std::sync::mpsc::{self, Receiver};

use super::chrome::{DIM, WARN};
use super::{Btn, Cmd, Modal, View};
use crate::hit::Canvas;
use crate::library::FileKind;
use crate::render::rgba;

#[cfg(target_os = "android")]
type Picked = crate::android::PickedFile;
#[cfg(not(target_os = "android"))]
type Picked = std::path::PathBuf;

/// A file dialog on its way: the picture picked, `None` when it was closed.
pub(super) type Pick = Receiver<Result<Option<Picked>, String>>;

/// Whether `path` can be a background: osu! shows PNG and JPEG ones only.
fn fits(path: &Path) -> bool {
    FileKind::Image.matches(path)
}

impl View {
    /// Why the background can't change now, if it can't: in a mapping session the others
    /// wouldn't get the picture.
    fn background_locked(&self) -> Option<&'static str> {
        self.collab.is_some().then_some(
            "the background can't change in a mapping session: the others wouldn't get the picture",
        )
    }

    /// File > Background picture: a file dialog for the picture.
    pub(super) fn pick_background(&mut self) {
        if self.bg_pick.is_some() {
            return;
        }
        if let Some(why) = self.background_locked() {
            self.status = why.into();
            return;
        }
        #[cfg(target_os = "android")]
        let rx = crate::android::pick_file(FileKind::Image);
        #[cfg(not(target_os = "android"))]
        let rx = {
            let start = self
                .ed
                .path
                .parent()
                .map_or_else(|| std::path::PathBuf::from("."), Path::to_path_buf);
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::library::pick_file(
                    FileKind::Image,
                    "Choose the background",
                    &start,
                ));
            });
            rx
        };
        self.bg_pick = Some(rx);
        self.status = "choose the background picture".into();
    }

    /// The file dialog's answer, once it comes.
    pub(super) fn poll_background_pick(&mut self) {
        let Some(rx) = &self.bg_pick else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("file selection stopped".into()),
        };
        self.bg_pick = None;
        match result {
            Ok(Some(file)) => {
                // Android's pick is a copy of its own, deleted once `file` goes: used before that.
                #[cfg(target_os = "android")]
                let path: &Path = &file.0;
                #[cfg(not(target_os = "android"))]
                let path: &Path = &file;
                if fits(path) {
                    self.use_background(path);
                } else {
                    self.status = "the background must be a .png or .jpg picture".into();
                }
            }
            Ok(None) => self.status = "no picture chosen".into(),
            Err(e) => self.status = format!("couldn't choose a picture: {e}"),
        }
    }

    /// Makes the picture at `from` the background (copied into the map's folder).
    fn use_background(&mut self, from: &Path) {
        if let Some(why) = self.background_locked() {
            self.status = why.into();
            return;
        }
        match self.ed.set_background_from(from) {
            Ok(name) => {
                self.mark_stale();
                self.status = format!("background: {name} (Ctrl+S saves it with the map)");
            }
            Err(e) => self.status = e,
        }
    }

    /// Song setup's Remove: no background picture (the file stays in the folder).
    pub(super) fn remove_background(&mut self) {
        if let Some(why) = self.background_locked() {
            self.status = why.into();
            return;
        }
        self.status = if self.ed.clear_background() {
            self.mark_stale();
            "no background picture now (Ctrl+S saves it with the map)".into()
        } else {
            "this difficulty has no background picture".into()
        };
    }

    /// A picture dropped on the editor with no picture block picked: the editor asks whether it
    /// is to be the background. One that can't be (a GIF, in a mapping session) is only copied
    /// into the map's folder, as before.
    pub(super) fn dropped_loose_picture(&mut self, from: &Path) {
        if self.background_locked().is_some() || !fits(from) {
            if let Some(name) = self.copy_picture_in(from) {
                self.status = format!(
                    "{name} is in the map's folder: put a Picture from the Text & pictures tab to show it"
                );
            }
            return;
        }
        self.commit_typing();
        self.menu = None;
        self.bg_ask = Some(from.to_path_buf());
        self.modal = Some(Modal::Background);
    }

    /// The answer to "make it the background?": yes, or only copy it into the folder.
    pub(super) fn answer_background(&mut self, use_it: bool) {
        if self.modal == Some(Modal::Background) {
            self.modal = None;
        }
        let Some(from) = self.bg_ask.take() else {
            return;
        };
        if use_it {
            self.use_background(&from);
        } else if let Some(name) = self.copy_picture_in(&from) {
            self.status = format!(
                "{name} is in the map's folder: put a Picture from the Text & pictures tab to show it"
            );
        }
    }

    pub(super) fn draw_background_question(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
        let name = self
            .bg_ask
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let choices = [
            (Cmd::BackgroundUse.name(), Cmd::BackgroundUse, true),
            (Cmd::BackgroundCopy.name(), Cmd::BackgroundCopy, false),
            ("Cancel", Cmd::Stay, false),
        ];
        let widths = choices.map(|(label, _, _)| Canvas::text_width(7.0, label) + 14.0);
        let row: f32 = widths.iter().sum::<f32>() + 6.0 * (widths.len() - 1) as f32;
        let (w, h) = ((row + 28.0).max(330.0).min(c.width - 20.0), 112.0);
        let (x, y) = ((c.width - w) / 2.0, 164.0);
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        let room = w - 28.0;
        c.text(
            x + 14.0,
            y + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            "Make it the background?",
        );
        c.text(
            x + 14.0,
            y + 30.0,
            7.0,
            rgba(0xe0e0ec, 1.0),
            &Canvas::fit(
                7.0,
                room,
                &format!("{name} becomes this difficulty's background picture."),
            ),
        );
        let now = match self.ed.background() {
            Some(old) => format!("Now: {old}. Enter sets it, Esc keeps things as they are."),
            None => "It has none now. Enter sets it, Esc keeps things as they are.".into(),
        };
        c.text(
            x + 14.0,
            y + 44.0,
            6.5,
            rgba(DIM, 1.0),
            &Canvas::fit(6.5, room, &now),
        );
        let mut bx = x + 14.0;
        for ((label, cmd, on), bw) in choices.into_iter().zip(widths) {
            self.button(c, [bx, y + 78.0, bw, 20.0], label, Btn::C(cmd), on);
            bx += bw + 6.0;
        }
    }

    /// Song setup's background row at `y`: the picture's name, Choose and Remove.
    pub(super) fn draw_background_row(&mut self, c: &mut Canvas, x0: f32, y: f32, inner: f32) {
        let small = 6.5;
        c.text(
            x0,
            y,
            small,
            rgba(DIM, 1.0),
            "Background picture (this difficulty)",
        );
        let by = y + 9.0;
        let locked = self.background_locked().is_some();
        let bg = self.ed.background();
        let choose = "Choose...";
        let remove = Cmd::NoBackground.name();
        let cw = Canvas::text_width(7.0, choose) + 14.0;
        let rw = Canvas::text_width(7.0, remove) + 14.0;
        let buttons = if locked {
            0.0
        } else if bg.is_some() {
            cw + rw + 12.0
        } else {
            cw + 6.0
        };
        let bw = inner - buttons;
        c.round_rect(
            [x0, by],
            [bw, 16.0],
            3.0,
            rgba(0x101018, 1.0),
            rgba(0x3a3a4e, 1.0),
            1.0,
        );
        let missing = bg
            .as_ref()
            .is_some_and(|b| self.ed.path.parent().is_some_and(|d| !d.join(b).is_file()));
        let (text, colour) = match (&bg, locked) {
            (_, true) => (
                format!(
                    "{}: can't change in a mapping session",
                    bg.as_deref().unwrap_or("none")
                ),
                DIM,
            ),
            (Some(b), _) if missing => (format!("{b} (not in the map's folder)"), WARN),
            (Some(b), _) => (b.clone(), 0xe0e0ec),
            (None, _) => (
                "none: drop a picture on the editor or choose one".into(),
                0x60607a,
            ),
        };
        c.text(
            x0 + 5.0,
            by + 4.5,
            7.0,
            rgba(colour, 1.0),
            &Canvas::fit(7.0, bw - 10.0, &text),
        );
        if locked {
            return;
        }
        let mut bx = x0 + bw + 6.0;
        self.button(
            c,
            [bx, by, cw, 16.0],
            choose,
            Btn::C(Cmd::Background),
            false,
        );
        bx += cw + 6.0;
        if bg.is_some() {
            self.button(
                c,
                [bx, by, rw, 16.0],
                remove,
                Btn::C(Cmd::NoBackground),
                false,
            );
        }
    }
}
