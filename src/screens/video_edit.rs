//! The video editor: a saved replay's cut, crop and export settings, rendered to an MP4 with
//! ffmpeg. The export itself is `State::export`, drawn on the render thread.

use super::screen::{Key, Screen};
use crate::*;

/// The video editor up, on a saved replay.
pub(crate) struct VideoEdit {
    pub(crate) ed: video::Editor,
    /// What the last export (or its start) said.
    pub(crate) status: String,
    /// The last video made here, for Show in folder.
    pub(crate) last_video: Option<PathBuf>,
}

impl VideoEdit {
    pub(crate) fn scene(ed: video::Editor, status: String) -> Scene {
        Scene::Other(Box::new(VideoEdit {
            ed,
            status,
            last_video: None,
        }))
    }

    /// Starts rendering the editor's cut to an MP4.
    fn start_export(&mut self, state: &mut State) {
        let ed = &mut self.ed;
        let Some((map, track)) = &ed.map else { return };
        let Some(ffmpeg) = video::find_ffmpeg() else {
            self.status = no_ffmpeg();
            return;
        };
        ed.playing = None;
        if let Some(a) = &state.audio {
            a.stop();
        }
        let sr = state.audio.as_ref().map(|a| a.rate).unwrap_or(48000);
        // The hitsounds as they were judged, and the music, for the cut.
        let mut sim = scores::replay_game(map, track, &ed.replay);
        sim.set_sfx_log(Some(Vec::new()));
        sim.set_time(ed.end);
        let sfx: Vec<(f64, audio::Sfx, f32)> = if ed.settings.hitsounds {
            sim.take_sfx_log()
                .unwrap_or_default()
                .into_iter()
                .map(|(t, s, v)| (t, s.into(), v))
                .collect()
        } else {
            Vec::new()
        };
        let sounds = audio::hitsound_set(
            sr,
            skin::hitsounds(&state.cfg.skin, &state.cfg.skin_parts, sr),
        );
        let (music, fx) = state.cfg.volumes();
        let speed = sim.speed();
        let samples = video::mix_audio(
            track,
            sr,
            &speed,
            ed.start,
            ed.end,
            music.max(0.05),
            &sfx,
            &sounds,
            fx,
        );
        let Some(dir) = video::videos_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join(format!(".audio-{}.wav", std::process::id()));
        if let Err(e) = video::write_wav(&wav, &samples, sr) {
            self.status = format!("can't write the audio: {e}");
            return;
        }
        let out = video::free_path(&dir, &video::file_name(&ed.replay));
        let settings = ed.settings;
        let size = video::SIZES[settings.size];
        let fps = video::FRAME_RATES[settings.fps];
        // Frames are evenly spaced in song time (what the video's audio follows).
        let step = 1000.0 / fps as f64;
        let (t0, t1) = (speed.track(ed.start), speed.track(ed.end));
        let frames = ((t1 - t0) / step).ceil().max(1.0) as u64;
        let job = video::Job {
            size,
            fps,
            crop: ed.crop_px(),
            settings,
            seconds: frames as f64 / fps as f64,
            wav: wav.clone(),
            out: out.clone(),
        };
        match video::start_encoder(&ffmpeg, job) {
            Ok(encoder) => {
                let mut game = Box::new(Play::new(scores::replay_game(map, track, &ed.replay)));
                game.set_time(ed.start);
                state.export = Some(video::Export {
                    game,
                    t: t0,
                    step,
                    t0,
                    sub: 0,
                    settings,
                    size,
                    encoder,
                    frame: 0,
                    frames,
                });
                self.status = String::new();
            }
            Err(e) => {
                let _ = std::fs::remove_file(&wav);
                self.status = e;
            }
        }
    }
}

impl State {
    /// The video editor for a saved replay.
    pub(crate) fn open_video_editor(&mut self, file: &std::path::Path) {
        let r = match scores::load_replay(file) {
            Ok(r) => r,
            Err(e) => {
                self.toast(tr!("can't open the replay: {e}", "リプレイを開けません: {e}", "не удаётся открыть повтор: {e}";));
                return;
            }
        };
        let entry = self
            .library
            .iter()
            .position(|e| e.path.to_string_lossy() == r.map_path && e.hash() == r.map_hash)
            .or_else(|| {
                self.library
                    .iter()
                    .position(|e| e.label() == r.map_label && e.hash() == r.map_hash)
            });
        let Some(entry) = entry else {
            let map = &r.map_label;
            self.toast(tr!("{map} isn't in your songs", "{map}は曲の中にありません", "{map} нет среди ваших песен";));
            return;
        };
        self.stop_audio();
        self.preview.show_background_for(&self.library[entry]);
        let rx = self.load_whole(entry, r.rate, Mods::from_list(&r.mods));
        let status = if video::find_ffmpeg().is_some() {
            String::new()
        } else {
            no_ffmpeg()
        };
        self.scene = VideoEdit::scene(video::Editor::new(r, rx, self.cfg.video), status);
    }
}

impl Screen for VideoEdit {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "VideoEdit"
    }

    fn activity(&self) -> &'static str {
        "video editor"
    }

    fn presence(&self) -> (String, String) {
        (
            tr!(
                "Making a replay video",
                "リプレイの動画を作成中",
                "Делает видео повтора"
            )
            .into(),
            self.ed.replay.map_label.clone(),
        )
    }

    fn dims_background(&self) -> bool {
        true
    }

    fn closes_main_menu(&self) -> bool {
        true
    }

    fn drags(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn loading(&self) -> bool {
        self.ed.rx.is_busy()
    }

    fn key(&mut self, state: &mut State, key: Key) {
        let Key {
            code,
            repeat,
            window,
            ..
        } = key;
        if state.export.is_some() {
            if code == KeyCode::Escape && !repeat {
                // Ending ffmpeg's input (or stopping a pass) ends it early; the encoder removes
                // what it made.
                if let Some(ex) = state.export.take() {
                    ex.encoder.cancel.store(true, Ordering::Relaxed);
                }
                self.status = tr!(
                    "export cancelled",
                    "書き出しを取り消しました",
                    "экспорт отменён"
                )
                .into();
            }
            return;
        }
        let ed = &mut self.ed;
        // Typing the target size takes the keys until Enter or Esc.
        if let Some(text) = &mut ed.typing {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    match text
                        .parse::<f64>()
                        .ok()
                        .filter(|mb| mb.is_finite() && *mb >= 1.0)
                    {
                        Some(mb) => {
                            ed.settings.target_mb = (mb.min(4000.0) * 10.0).round() / 10.0;
                            ed.typing = None;
                        }
                        None => {
                            self.status = tr!(
                                "type a size of at least 1 MB, like 10 or 25",
                                "1 MB以上のサイズを入力 (10や25など)",
                                "введите размер от 1 МБ, например 10 или 25"
                            )
                            .into()
                        }
                    }
                }
                KeyCode::Escape => ed.typing = None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::KeyQ => {
                    ed.typing = None;
                    ed.settings.quality = 0;
                }
                _ => {
                    let ch = match code {
                        KeyCode::Period
                        | KeyCode::Comma
                        | KeyCode::NumpadDecimal
                        | KeyCode::NumpadComma => Some('.'),
                        _ => size_char(code).filter(|c| c.is_ascii_digit()),
                    };
                    if let Some(ch) = ch.filter(|_| text.len() < 6) {
                        text.push(ch);
                    }
                }
            }
            if ed.typing.is_none() {
                self.status.clear();
            }
            state.cfg.video = ed.settings;
            state.cfg.save();
            return;
        }
        let rate = ed.replay.rate;
        let step = if state.modifiers_shift {
            5000.0
        } else {
            1000.0
        } * rate;
        // Shift steps back through a setting's choices.
        let dir = if state.modifiers_shift { -1 } else { 1 };
        let cycle = |i: usize, n: usize| (i as i32 + dir).rem_euclid(n as i32) as usize;
        let before = ed.settings;
        match code {
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let t = ed.cursor
                    + if code == KeyCode::ArrowRight {
                        step
                    } else {
                        -step
                    };
                ed.playing = None;
                if let Some(a) = &state.audio {
                    a.stop();
                }
                ed.seek(t);
            }
            _ if repeat => {}
            KeyCode::Space => {
                if ed.playing.take().is_some() {
                    if let Some(a) = &state.audio {
                        a.stop();
                    }
                } else if let (Some((_, track)), Some(a)) = (&ed.map, &state.audio) {
                    let at = ed
                        .game
                        .as_ref()
                        .map(|g| g.track_at(ed.cursor))
                        .unwrap_or(ed.cursor / rate);
                    a.play(track.clone(), at);
                    ed.playing = Some((Instant::now(), ed.cursor));
                }
            }
            KeyCode::KeyI if ed.cursor < ed.end => ed.start = ed.cursor,
            KeyCode::KeyO if ed.cursor > ed.start => ed.end = ed.cursor,
            KeyCode::KeyC => {
                let presets = video::crop_presets();
                let i = presets
                    .iter()
                    .position(|(n, _)| *n == ed.crop_name)
                    .map(|i| i + 1)
                    .unwrap_or(0)
                    % presets.len();
                (ed.crop_name, ed.crop) = presets[i];
            }
            KeyCode::KeyZ => ed.settings.size = cycle(ed.settings.size, video::SIZES.len()),
            KeyCode::KeyF => ed.settings.fps = cycle(ed.settings.fps, video::FRAME_RATES.len()),
            KeyCode::KeyB => ed.settings.blur = cycle(ed.settings.blur, video::BLURS.len()),
            KeyCode::KeyQ => {
                ed.settings.quality = cycle(ed.settings.quality, video::QUALITIES.len())
            }
            KeyCode::KeyT => {
                // Aiming at a size, and typing it.
                ed.settings.quality = video::QUALITIES.len() - 1;
                ed.typing = Some(String::new());
            }
            KeyCode::KeyE => ed.settings.speed = cycle(ed.settings.speed, video::SPEEDS.len()),
            KeyCode::KeyU => ed.settings.hud = !ed.settings.hud,
            KeyCode::KeyH => ed.settings.hitsounds = !ed.settings.hitsounds,
            KeyCode::Enter | KeyCode::NumpadEnter => self.start_export(state),
            KeyCode::KeyS => {
                if let Some(path) = self.last_video.clone() {
                    state.show_in_folder(&path);
                }
            }
            KeyCode::Escape => {
                state.show_select();
            }
            _ => {
                state.handle_global_key(code, window);
            }
        }
        // Remembered for the next video.
        if self.ed.settings != before {
            self.status.clear();
            state.cfg.video = self.ed.settings;
            state.cfg.save();
        }
    }

    fn pointer_moved(&mut self, state: &mut State, x: f64, y: f64) -> Option<CursorIcon> {
        if state.export.is_some() {
            return None;
        }
        let (vx, _) = state.mouse_virtual();
        let k = (state.window_h / render::VIRTUAL_HEIGHT).max(0.01) as f64;
        let ed = &mut self.ed;
        if ed.scrubbing {
            let t = ed.time_at(vx);
            ed.seek(t);
            return Some(CursorIcon::Pointer);
        }
        if let Some((grip, (px, py), from)) = ed.grip {
            let [_, _, fw, fh] = ed.frame_rect;
            ed.crop = layout::drag(
                from,
                grip,
                ((x - px) / (fw as f64 * k)) as f32,
                ((y - py) / (fh as f64 * k)) as f32,
            )
            .clamped();
            ed.crop_name = "custom";
            return Some(grip.cursor());
        }
        let [fx, fy, fw, fh] = ed.frame_rect;
        if let Some(g) = layout::hit(
            ed.crop,
            x - fx as f64 * k,
            y - fy as f64 * k,
            fw as f64 * k,
            fh as f64 * k,
        ) {
            return Some(g.cursor());
        }
        None
    }

    fn mouse_button(&mut self, state: &mut State, pressed: bool) -> bool {
        if state.export.is_some() {
            return false;
        }
        let ed = &mut self.ed;
        let (vx, vy) = (
            state.mouse.0 as f32 * render::VIRTUAL_HEIGHT / state.window_h.max(1.0),
            state.mouse.1 as f32 * render::VIRTUAL_HEIGHT / state.window_h.max(1.0),
        );
        let k = (state.window_h / render::VIRTUAL_HEIGHT).max(0.01) as f64;
        if !pressed {
            ed.scrubbing = false;
            ed.grip = None;
        } else if ed.in_timeline(vx, vy) {
            ed.scrubbing = true;
            ed.playing = None;
            if let Some(a) = &state.audio {
                a.stop();
            }
            let t = ed.time_at(vx);
            ed.seek(t);
            return true;
        } else {
            let [fx, fy, fw, fh] = ed.frame_rect;
            if let Some(g) = layout::hit(
                ed.crop,
                state.mouse.0 - fx as f64 * k,
                state.mouse.1 - fy as f64 * k,
                fw as f64 * k,
                fh as f64 * k,
            ) {
                ed.grip = Some((g, state.mouse, ed.crop));
                return true;
            }
        }
        false
    }

    fn draw(&mut self, state: &mut State, canvas: &mut Canvas, _now: Instant) -> Option<Scene> {
        let mut next = None;
        let ed = &mut self.ed;
        match ed.poll() {
            Err(e) => {
                state.status = format!("can't open the replay: {e}");
                next = Some(Scene::Select);
            }
            Ok(_) => {
                if let Some((at, from)) = ed.playing {
                    let elapsed = at.elapsed().as_secs_f64() * 1000.0;
                    let t = match &ed.game {
                        Some(g) => g.speed().map(g.track_at(from) + elapsed),
                        None => from + elapsed * ed.replay.rate,
                    };
                    if t >= ed.end {
                        ed.playing = None;
                        if let Some(a) = &state.audio {
                            a.stop();
                        }
                    } else {
                        ed.seek(t);
                    }
                }
            }
        }
        let progress = state.export.as_ref().map(|ex| ex.progress());
        if let Some(g) = &mut ed.game {
            g.view.kiai_effects = state.cfg.kiai_effects;
            g.view.hud = state.cfg.hud;
            g.view.hidden = state.cfg.hud.off;
        }
        ed.draw(
            canvas,
            state.cfg.scroll_speed,
            progress,
            &self.status,
            self.last_video.is_some(),
        );
        next
    }
}
