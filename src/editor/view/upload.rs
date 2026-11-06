//! Sending the map to taiko.rs: File > Upload opens a checklist; Send packs the map's folder
//! and posts it on a thread. The set's number is kept beside the map, so the next upload updates
//! the same set.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::chrome::{DIM, WARN};
use super::{Btn, Modal, View};
use crate::custom;
use crate::render::{Canvas, rgba};

/// A button of the upload window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum UpAct {
    Send,
    /// Send as a new set, though the folder was sent before.
    SendNew,
    Copy,
    Close,
}

/// One line of the checklist.
#[derive(Clone, PartialEq, Debug)]
pub struct Check {
    pub ok: bool,
    /// Sending waits until it is fine.
    pub blocking: bool,
    pub text: String,
}

type Sending = Result<(custom::Sent, Vec<String>, Vec<String>), String>;

#[derive(Default)]
pub struct UploadUi {
    pub checks: Vec<Check>,
    /// The set this folder was sent as before.
    pub id: Option<u32>,
    rx: Option<Receiver<Sending>>,
    /// How it ended: the set's page, or what went wrong.
    pub done: Option<Result<String, String>>,
    /// Difficulties that stayed behind, from the last send.
    pub left_out: Vec<String>,
    /// The difficulties that went.
    pub sent_diffs: Vec<String>,
    pub version: u32,
}

impl UploadUi {
    pub fn can_send(&self) -> bool {
        self.rx.is_none()
            && self.done.as_ref().is_none_or(Result::is_err)
            && !self.checks.iter().any(|c| c.blocking && !c.ok)
    }

    pub fn sending(&self) -> bool {
        self.rx.is_some()
    }
}

fn check(ok: bool, blocking: bool, text: impl Into<String>) -> Check {
    Check {
        ok,
        blocking,
        text: text.into(),
    }
}

/// What has to be right before the map at `path` (saved, if `saved`) can be sent.
pub fn checks(path: &Path, saved: bool, logged_in: bool) -> Vec<Check> {
    let mut out = vec![
        check(
            logged_in,
            true,
            if logged_in {
                "Logged in to taiko.rs"
            } else {
                "Log in to your taiko.rs account first (the main menu, Multiplayer)"
            },
        ),
        check(
            saved,
            true,
            if saved {
                "Saved"
            } else {
                "Save the map first (Ctrl+S): what is sent is the saved file"
            },
        ),
    ];
    let Some(meta) = crate::beatmap::read_meta_file(path) else {
        out.push(check(false, true, "Can't read the saved map"));
        return out;
    };
    let folder = path.parent().unwrap_or(Path::new("."));
    let (mode, notes) = std::fs::read_to_string(path)
        .map(|t| custom::mode_and_notes(&t))
        .unwrap_or((None, 0));
    out.push(check(
        mode == Some(1),
        true,
        if mode == Some(1) {
            "A taiko map"
        } else {
            "Only taiko maps (Mode: 1) can be sent"
        },
    ));
    let enough = notes >= custom::MIN_NOTES;
    out.push(check(
        enough,
        true,
        if enough {
            format!("{notes} notes")
        } else {
            format!(
                "Needs at least {} notes (it has {notes})",
                custom::MIN_NOTES
            )
        },
    ));
    for (what, value) in [
        ("Artist", &meta.artist),
        ("Title", &meta.title),
        ("Mapper name", &meta.creator),
    ] {
        let ok = !value.trim().is_empty();
        out.push(check(
            ok,
            true,
            if ok {
                format!("{what}: {value}")
            } else {
                format!("{what} is empty: fill it in File > Song setup")
            },
        ));
    }
    let audio = folder.join(&meta.audio_file);
    let music = !meta.audio_file.is_empty() && audio.is_file();
    out.push(check(
        music,
        true,
        if music {
            format!("Music: {}", meta.audio_file)
        } else {
            "The music file isn't in the map's folder".to_string()
        },
    ));
    let own = meta.set_id.is_none();
    out.push(check(
        own,
        true,
        if own {
            "Your own map (no osu! set number)".to_string()
        } else {
            "The map names an osu! set (BeatmapSetID): only your own maps can be sent".to_string()
        },
    ));
    if let Some(g) = std::fs::read_to_string(path)
        .ok()
        .filter(|t| crate::tko::format::version_of(t).is_some())
        .and_then(|t| crate::tko::format::read(&t).ok())
        .map(|p| p.gimmick)
    {
        out.extend(tko_checks(&g));
    }
    let background = meta
        .background
        .as_ref()
        .is_some_and(|b| folder.join(b).is_file());
    out.push(check(
        background,
        false,
        if background {
            "Background picture"
        } else {
            "No background picture: the page will show a plain colour (not required)"
        },
    ));
    out
}

/// What a player should know about a `.tko` map's tricks before it goes out: nothing blocks.
fn tko_checks(g: &crate::tko::gimmick::Gimmick) -> Vec<Check> {
    use crate::tko::gimmick::ShowColour;
    let mut out = Vec::new();
    let lying = g.groups.iter().filter(|gr| !gr.show.is_true());
    let lies: usize = lying.clone().map(|gr| gr.notes.len()).sum();
    if lies > 0 {
        out.push(check(
            false,
            false,
            format!(
                "{lies} notes show as another colour or size: players can turn this off (Settings, Gameplay, Show true notes)"
            ),
        ));
    }
    let blind = lying
        .filter(|gr| gr.show.colour != ShowColour::Same && gr.show.reveal == 0.0)
        .map(|gr| gr.notes.len())
        .sum::<usize>();
    if blind > 0 {
        out.push(check(
            false,
            false,
            format!(
                "{blind} of them show their true colour only at the hit: they can't be read, only learned"
            ),
        ));
    }
    if !g.fakes.is_empty() {
        out.push(check(
            true,
            false,
            format!("{} fake notes (drawn, never hit)", g.fakes.len()),
        ));
    }
    out
}

impl View {
    /// File > Upload.
    pub(super) fn open_upload(&mut self) {
        let folder = self.ed.path.parent().map(Path::to_path_buf);
        self.upload.checks = checks(&self.ed.path, !self.ed.modified(), self.account.is_some());
        self.upload.id = folder.as_deref().and_then(custom::sidecar_id);
        if !self.upload.sending() && self.upload.done.as_ref().is_some_and(Result::is_err) {
            self.upload.done = None;
        }
        self.modal = Some(Modal::Upload);
    }

    pub(super) fn press_upload(&mut self, act: UpAct) {
        match act {
            UpAct::Close => self.modal = None,
            UpAct::Copy => {
                if let Some(Ok(url)) = &self.upload.done {
                    self.status = if crate::clipboard::set(url) {
                        "link copied".into()
                    } else {
                        "couldn't copy the link".into()
                    };
                }
            }
            UpAct::Send | UpAct::SendNew => {
                let was = self.ed.path.clone();
                let folder: PathBuf = was.parent().unwrap_or(Path::new(".")).to_path_buf();
                self.open_upload();
                if !self.upload.can_send() {
                    return;
                }
                let Some(token) = self.account.clone() else {
                    return;
                };
                let id = self.upload.id.filter(|_| act == UpAct::Send);
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let sent = custom::pack(&folder).and_then(|packed| {
                        custom::upload(&token, id, &packed.bytes).map(|s| {
                            custom::write_sidecar(&folder, s.id);
                            (s, packed.sent, packed.left_out)
                        })
                    });
                    let _ = tx.send(sent);
                });
                self.upload.rx = Some(rx);
                self.upload.done = None;
                self.status = "sending the map...".into();
            }
        }
    }

    /// Takes the answer of a send in progress.
    pub(super) fn poll_upload(&mut self) {
        let Some(rx) = &self.upload.rx else { return };
        let got = match rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the sending stopped unexpectedly".to_string()),
            Ok(sent) => sent,
        };
        self.upload.rx = None;
        match got {
            Ok((sent, diffs, left_out)) => {
                self.upload.id = Some(sent.id);
                self.upload.version = sent.version;
                self.upload.left_out = left_out;
                self.upload.sent_diffs = diffs;
                self.status = format!("sent: set {}", sent.id);
                self.upload.done = Some(Ok(custom::page_url(sent.id)));
            }
            Err(e) => {
                self.status = "couldn't send the map".into();
                self.upload.done = Some(Err(e));
            }
        }
    }

    pub(super) fn draw_upload(&mut self, c: &mut Canvas) {
        c.rect(0.0, 0.0, c.width, 480.0, rgba(0x000000, 0.62));
        let lines = self.upload.checks.len() as f32;
        let extra = if self.upload.done.is_some() {
            64.0
        } else {
            0.0
        };
        let (w, h) = (
            470.0_f32.min(c.width - 20.0),
            76.0 + lines * 14.0 + extra + 30.0,
        );
        let (x, y) = ((c.width - w) / 2.0, (480.0 - h) / 2.0 - 10.0);
        self.overlay_rect = [x, y, w, h];
        c.round_rect(
            [x, y],
            [w, h],
            4.0,
            rgba(0x1b1b26, 1.0),
            rgba(0x6a6a88, 1.0),
            1.0,
        );
        c.text(
            x + 14.0,
            y + 12.0,
            9.0,
            rgba(0xffffff, 1.0),
            "Upload to taiko.rs",
        );
        let about = match self.upload.id {
            Some(id) => {
                format!("Sent before as set {id}: this updates it (approval is asked again).")
            }
            None => "Public as CUSTOM at once; gives pp once it is approved.".to_string(),
        };
        c.text(
            x + 14.0,
            y + 30.0,
            6.5,
            rgba(DIM, 1.0),
            &Canvas::fit(6.5, w - 28.0, &about),
        );
        let mut ty = y + 48.0;
        for k in &self.upload.checks {
            let (mark, col) = match (k.ok, k.blocking) {
                (true, _) => ("OK", 0x6fdc8c),
                (false, true) => ("!!", WARN),
                (false, false) => ("--", DIM),
            };
            c.text(x + 14.0, ty, 7.0, rgba(col, 1.0), mark);
            c.text(
                x + 36.0,
                ty,
                7.0,
                rgba(if k.ok { 0xd0d0e0 } else { col }, 1.0),
                &Canvas::fit(7.0, w - 50.0, &k.text),
            );
            ty += 14.0;
        }
        ty += 6.0;
        match &self.upload.done {
            Some(Ok(url)) => {
                c.text(
                    x + 14.0,
                    ty,
                    7.5,
                    rgba(0x6fdc8c, 1.0),
                    &format!(
                        "Sent (version {}). Anyone can play it now.",
                        self.upload.version.max(1)
                    ),
                );
                c.text(
                    x + 14.0,
                    ty + 14.0,
                    7.0,
                    rgba(0xffffff, 1.0),
                    &Canvas::fit(7.0, w - 28.0, url),
                );
                c.text(
                    x + 14.0,
                    ty + 28.0,
                    6.5,
                    rgba(DIM, 1.0),
                    &Canvas::fit(
                        6.5,
                        w - 28.0,
                        &format!("Sent: {}", self.upload.sent_diffs.join(", ")),
                    ),
                );
                if !self.upload.left_out.is_empty() {
                    c.text(
                        x + 14.0,
                        ty + 40.0,
                        6.5,
                        rgba(DIM, 1.0),
                        &Canvas::fit(
                            6.5,
                            w - 28.0,
                            &format!("Left out: {}", self.upload.left_out.join(", ")),
                        ),
                    );
                }
            }
            Some(Err(e)) => {
                c.text(x + 14.0, ty, 7.5, rgba(WARN, 1.0), "Couldn't send it:");
                c.text(
                    x + 14.0,
                    ty + 14.0,
                    7.0,
                    rgba(WARN, 1.0),
                    &Canvas::fit(7.0, w - 28.0, e),
                );
            }
            None if self.upload.sending() => {
                c.text(x + 14.0, ty, 7.5, rgba(0xffffff, 1.0), "Sending...");
            }
            None => {}
        }
        let by = y + h - 28.0;
        let mut bx = x + 14.0;
        let sent = matches!(self.upload.done, Some(Ok(_)));
        let can = self.upload.can_send();
        if sent {
            let label = "Copy link";
            let bw = Canvas::text_width(7.0, label) + 14.0;
            self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Copy), true);
            bx += bw + 6.0;
        } else if can {
            let label = if self.upload.id.is_some() {
                "Update the set"
            } else {
                "Send"
            };
            let bw = Canvas::text_width(7.0, label) + 14.0;
            self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Send), true);
            bx += bw + 6.0;
            if self.upload.id.is_some() {
                let label = "Send as a new set";
                let bw = Canvas::text_width(7.0, label) + 14.0;
                self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::SendNew), false);
                bx += bw + 6.0;
            }
        }
        let label = if sent { "Done" } else { "Close" };
        let bw = Canvas::text_width(7.0, label) + 14.0;
        self.button(c, [bx, by, bw, 20.0], label, Btn::U(UpAct::Close), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("taiko-upload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn map(notes: usize, set: Option<u32>) -> String {
        let mut text = format!(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: 1\r\n\r\n[Metadata]\r\nTitle:Song\r\nArtist:Band\r\nCreator:me\r\nVersion:Oni\r\n{}\r\n[Events]\r\n0,0,\"bg.png\",0,0\r\n\r\n[HitObjects]\r\n",
            set.map(|s| format!("BeatmapSetID:{s}\r\n"))
                .unwrap_or_default()
        );
        for k in 0..notes {
            text += &format!("256,192,{},1,0\r\n", 1000 + k * 200);
        }
        text
    }

    fn problems(list: &[Check]) -> Vec<&str> {
        list.iter()
            .filter(|c| c.blocking && !c.ok)
            .map(|c| c.text.as_str())
            .collect()
    }

    #[test]
    fn a_good_map_has_nothing_in_the_way() {
        let dir = folder("good");
        let path = dir.join("a [Oni].osu");
        std::fs::write(&path, map(20, None)).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 8]).unwrap();
        std::fs::write(dir.join("bg.png"), [1u8; 8]).unwrap();
        let list = checks(&path, true, true);
        assert!(problems(&list).is_empty(), "{:?}", problems(&list));
        assert!(list.iter().all(|c| c.ok));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_missing_is_said_and_blocks_sending() {
        let dir = folder("bad");
        let path = dir.join("a [Oni].osu");
        std::fs::write(&path, map(3, Some(123))).unwrap();
        let list = checks(&path, false, false);
        let bad = problems(&list);
        assert!(bad.iter().any(|t| t.contains("Log in")));
        assert!(bad.iter().any(|t| t.contains("Save the map")));
        assert!(bad.iter().any(|t| t.contains("at least 10 notes")));
        assert!(bad.iter().any(|t| t.contains("music")));
        assert!(bad.iter().any(|t| t.contains("osu! set")));
        let ui = UploadUi {
            checks: list,
            ..Default::default()
        };
        assert!(!ui.can_send());
        // No background is only a remark.
        let missing = checks(&path, true, true);
        let bg = missing
            .iter()
            .find(|c| c.text.contains("background"))
            .unwrap();
        assert!(!bg.ok && !bg.blocking);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_map_that_lies_says_so_without_blocking() {
        use crate::tko::gimmick::{FakeNote, Gimmick, Group, ShowAs, ShowColour};
        let mut g = Gimmick::default();
        g.groups.push(Group {
            notes: vec![1000.0, 1200.0],
            show: ShowAs {
                colour: ShowColour::Swap,
                ..ShowAs::default()
            },
            ..Group::default()
        });
        g.fakes.push(FakeNote {
            t: 1100.0,
            ..FakeNote::default()
        });
        let list = tko_checks(&g);
        assert!(list.iter().all(|c| !c.blocking));
        assert!(list[0].text.starts_with("2 notes show as"), "{list:?}");
        assert!(list[1].text.contains("only at the hit"));
        assert!(list[2].ok && list[2].text.starts_with("1 fake"));
        g.groups[0].show.reveal = 0.5;
        assert_eq!(tko_checks(&g).len(), 2);
        assert!(tko_checks(&Gimmick::default()).is_empty());
    }
}
