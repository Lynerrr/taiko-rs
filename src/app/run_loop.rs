//! The render thread's loop and the window thread's event handling (`App`).

#[allow(unused_imports)]
use crate::*;

#[cfg(windows)]
use super::crash::crash_box;
use super::crash::{CRASH_CONTEXT, crash_report, send_crash_report};
use super::gpu::{VULKAN, VULKAN_ON_AUTO, gpu_start};
use super::shell::whats_new;

/// Puts an F12 screenshot on the clipboard, saves it as the next free `taiko-rs-N.png` in the
/// Screenshots folder, or both (`cfg.screenshot_to`), off the render thread (the encoding takes
/// a moment), and says where it went.
pub(super) fn save_screenshot(state: &Arc<Mutex<State>>, w: u32, h: u32, rgb: Vec<u8>) {
    let state = state.clone();
    {
        let mut s = lock(&state);
        s.last_shot = Some(Arc::new((w, h, rgb.clone())));
        s.social.shot_ready = true;
    }
    std::thread::spawn(move || {
        let to = lock(&state).cfg.screenshot_to;
        let copied = to.clipboard() && clipboard::set_image(w, h, &rgb);
        let text = if !to.folder() {
            if copied {
                "screenshot copied".to_string()
            } else {
                "screenshot failed: no clipboard to copy to".to_string()
            }
        } else {
            let saved = (|| {
                let dir =
                    config::own_dir("Screenshots").ok_or("no folder to keep screenshots in")?;
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = (1..)
                    .map(|n| dir.join(format!("taiko-rs-{n}.png")))
                    .find(|p| !p.exists())
                    .unwrap();
                image::save_buffer(&path, &rgb, w, h, image::ColorType::Rgb8)
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            })();
            match saved {
                Ok(path) => {
                    let name = path
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    format!(
                        "screenshot {}saved: Screenshots/{name}",
                        if copied { "copied and " } else { "" }
                    )
                }
                Err(e) => format!(
                    "screenshot {}failed: {e}",
                    if copied { "copied, saving " } else { "" }
                ),
            }
        };
        eprintln!("{text}");
        lock(&state).toast = Some(Toast {
            text,
            until: Instant::now() + Duration::from_millis(4000),
        });
    });
}

pub(crate) fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // A panic elsewhere shouldn't take the game down with a poisoned lock.
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sleeps until `deadline` with sub-100µs precision: coarse sleep, then spin the last stretch.
pub(super) fn sleep_until(deadline: Instant) {
    const SPIN: Duration = Duration::from_micros(500);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > SPIN {
            std::thread::sleep(left - SPIN);
        } else {
            std::hint::spin_loop();
        }
    }
}

/// The render loop. Order matters for latency: (under vsync) optionally delay, wait for the
/// swapchain, and only then sample the clock and build the frame.
/// What the render thread tells the window thread.
pub(crate) enum UserEvent {
    /// The render thread stopped (quit requested or screenshot run finished).
    Stopped,
    /// Gameplay wants the pointer hidden (it's been still a second) or back.
    HideCursor(bool),
    /// Files dragged over the window, on Wayland (winit reports them itself elsewhere).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    FileDrag(FileDrag),
    /// A window size was picked (`State::window_request`).
    WindowRequest,
    /// Android: the on-screen keyboard typed something (or came up or went away).
    #[cfg(target_os = "android")]
    Typed(android::Typed),
    /// A `taiko-rs://` link opened while the game runs (`link::listen`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Link(link::Link),
    /// A map archive opened from the file manager while the game runs (`link::listen`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Open(PathBuf),
}

/// Files dragged over the window, or dropped on it.
pub(crate) enum FileDrag {
    Hover,
    Left,
    Drop(Vec<PathBuf>),
}

pub(super) fn render_loop(
    state: Arc<Mutex<State>>,
    mut renderer: Renderer,
    proxy: EventLoopProxy<UserEvent>,
) {
    let mut canvas = Canvas::default();
    let mut export_canvas = Canvas::default();
    let mut blend = video::Blend::default();
    let mut next_frame = Instant::now();
    let mut last_sample: Option<Instant> = None;
    let mut requested_present: Option<PresentChoice> = None;
    let mut pacer = clock::VsyncPacer::default();
    // TAIKO_NO_PACE=1: draw notes for when each image was handed out, as before (to compare).
    let pace = std::env::var_os("TAIKO_NO_PACE").is_none();
    let mut waits = (0.0, 0.0, 0.0);
    let mut cursor_hidden = false;
    // TAIKO_PROFILE=1: every 2 s, print where frame time goes (build = CPU under the state lock).
    let profile = std::env::var_os("TAIKO_PROFILE").is_some();
    // The waits are only measured for the profile and the smoothness report: reading the clock
    // can cost microseconds a time (HPET).
    let timing = profile || std::env::var_os("TAIKO_STATS").is_some();
    // TAIKO_HITCH: every frame over 10 ms, split into where its time went.
    let hitch = std::env::var_os("TAIKO_HITCH").is_some();
    let mut h_end = Instant::now();
    let (mut prof_start, mut prof_n, mut prof_build, mut prof_submit, mut prof_shapes) =
        (Instant::now(), 0u32, 0.0f64, 0.0f64, 0usize);
    let (mut prof_acquire, mut prof_acquire_max) = (0.0f64, 0.0f64);
    // TAIKO_LAYOUT=1: print each text that doesn't fit (once), see `debug::layout_problems`.
    canvas.audit = std::env::var_os("TAIKO_LAYOUT").is_some();
    let mut layout_seen = std::collections::HashSet::new();
    let mut frames = 0u32;
    loop {
        frames = frames.saturating_add(1);
        if frames == gpu_start::FRAMES {
            gpu_start::going();
        }
        let (
            wanted,
            cap,
            refresh_hz,
            surface,
            size,
            atlas,
            letterbox,
            safe,
            pixel_scale,
            avatar_uploads,
            picture_uploads,
        ) = {
            let mut s = lock(&state);
            if s.quit {
                // In game.log: the game was closed on purpose, not lost.
                eprintln!(
                    "closing: quit from the game ({})",
                    if s.quit_asked {
                        "answered the exit question"
                    } else {
                        "no question asked"
                    }
                );
                break;
            }
            s.poll_skin();
            let surface = s.surface_swap.take();
            let size = s.want_size.take();
            let letterbox = s.sync_letterbox();
            let safe = s.safe_area();
            if s.scene.is_play()
                && let Some(sm) = &mut s.smooth
            {
                sm.sample_waits(waits.0, waits.1, waits.2);
            }
            (
                s.wanted_present(),
                s.frame_cap(),
                s.refresh_hz,
                surface,
                size,
                s.sprites_upload.take(),
                letterbox,
                safe,
                if s.cfg.half_resolution { 2 } else { 1 },
                s.avatars.take_uploads(),
                s.pictures.take_uploads(),
            )
        };
        // GPU waits and uploads never hold the input/state mutex.
        if let Some((surface, w, h)) = surface {
            renderer.set_surface(surface, w, h);
        }
        if let Some((w, h)) = size {
            renderer.resize(w, h);
        }
        renderer.letterbox = letterbox;
        renderer.safe = safe;
        if requested_present != Some(wanted) {
            renderer.set_present(wanted);
            requested_present = Some(wanted);
            lock(&state).present = renderer.present;
        }
        let present = renderer.present;
        renderer.pixel_scale = pixel_scale;
        for (slot, rgba) in avatar_uploads {
            renderer.set_avatar(slot, &rgba);
        }
        for (slot, rgba) in picture_uploads {
            renderer.set_picture(slot, &rgba);
        }
        if let Some(atlas) = atlas {
            renderer.set_sprites(&atlas);
            for c in [&mut canvas, &mut export_canvas] {
                c.sprites = render::PerSprite(atlas.sprites.map(|s| s.map(|(_, size)| size)));
                c.skin_layout = atlas.layout;
            }
        }

        let h_top = Instant::now();
        let t0 = timing.then(Instant::now);
        let Some(frame) = renderer.acquire() else {
            // In the background nothing is drawn, but the music is still looked after: the next
            // song of the player comes from the frame. A few times a second is plenty.
            let mut s = lock(&state);
            if s.backgrounded {
                if s.music_goes_on() {
                    s.build_frame(&mut canvas, Instant::now(), renderer.virtual_width());
                }
                drop(s);
                // Often enough that the media controls' presses feel immediate.
                std::thread::sleep(Duration::from_millis(60));
            } else {
                drop(s);
                std::thread::sleep(Duration::from_millis(2));
            }
            continue;
        };
        let acquired = Instant::now();
        let now = acquired;
        // With vsync the notes are drawn for this frame's place on the refresh grid; the game
        // itself (and judging) runs on the real clock.
        let pace_shift_ms = if present == PresentChoice::Fifo && pace {
            let t = pacer.frame_time(acquired, refresh_hz);
            if t >= acquired {
                (t - acquired).as_secs_f64() * 1000.0
            } else {
                -(acquired - t).as_secs_f64() * 1000.0
            }
        } else {
            pacer.reset();
            0.0
        };
        // A video export renders offscreen for a while each UI frame (which shows its progress).
        let exporting = Instant::now();
        while exporting.elapsed() < Duration::from_millis(30) {
            let job = lock(&state).export_frame(&mut export_canvas);
            let Some(ExportView {
                size: (w, h),
                tx,
                brightness,
                clear,
                blended,
                last,
            }) = job
            else {
                break;
            };
            let shown = renderer.bg_brightness;
            renderer.bg_brightness = brightness;
            let vw = export_canvas.width.max(1.0);
            let clips: Vec<(std::ops::Range<usize>, [f32; 4])> = export_canvas
                .clips
                .iter()
                .map(|(r, [x, y, cw, ch])| {
                    (
                        r.clone(),
                        [
                            x / vw,
                            y / render::VIRTUAL_HEIGHT,
                            cw / vw,
                            ch / render::VIRTUAL_HEIGHT,
                        ],
                    )
                })
                .collect();
            let frame = renderer.render_offscreen(&export_canvas.shapes, &clips, w, h, clear);
            renderer.bg_brightness = shown;
            let Some(px) = frame else { break };
            // Motion blur: the frame is the average of its views.
            let px = if blended {
                blend.add(&px);
                if !last {
                    continue;
                }
                blend.take()
            } else {
                px
            };
            if tx.send(px).is_err() {
                break;
            }
        }
        let h_lock = Instant::now();
        let h_build;
        let h_built;
        let (capture, clip, backdrop, background) = {
            let mut s = lock(&state);
            h_build = Instant::now();
            // Video export frames above are drawn without the grid; the screen's frame with it.
            render::set_text_grid(renderer.text_grid());
            render::set_text_bias(renderer.text_bias());
            render::set_frame_time(Some(now));
            s.frame_acquired = Some(acquired);
            s.pace_shift_ms = pace_shift_ms;
            s.seen_refresh_ms = pacer.refresh_ms();
            let capture = s.build_frame(&mut canvas, now, renderer.virtual_width());
            h_built = Instant::now();
            if let Some([x, y, _, h]) = safe {
                // The margins in the interface's units.
                let (side, edge) = (x as f32, y as f32);
                let k = render::VIRTUAL_HEIGHT / h.max(1) as f32;
                canvas.bleed([side * k, edge * k, side * k, edge * k]);
            }
            render::set_frame_time(None);
            render::set_text_grid(None);
            if canvas.audit {
                for p in debug::layout_problems(&canvas) {
                    if layout_seen.insert(p.clone()) {
                        eprintln!("layout: {p}");
                    }
                }
            }
            // Picked from a scripted key or a frame: the window thread applies it.
            if s.window_request.is_some() {
                let _ = proxy.send_event(UserEvent::WindowRequest);
            }
            let hide = s.cursor_hidden(now);
            if hide != cursor_hidden {
                cursor_hidden = hide;
                // The pointer resting on an edge of the area goes away with its frame.
                if hide {
                    s.hover = None;
                }
                let _ = proxy.send_event(UserEvent::HideCursor(hide));
            }
            let background = s.preview.bg_upload.take();
            if background.is_some() {
                s.bg_changed = Some(Instant::now());
            }
            renderer.bg_brightness = s.bg_brightness;
            renderer.bg_blur = s.idle.amount();
            // A new background cross-fades from the previous one.
            let fade = if s.on_menu() {
                MENU_BACKGROUND_FADE_MS
            } else {
                300.0
            };
            renderer.bg_mix = s
                .bg_changed
                .map(|t| render::progress(t, fade))
                .unwrap_or(1.0);
            (
                capture,
                s.frame_clip.take(),
                s.backdrop(s.dims_background()),
                background,
            )
        };
        if let Some(image) = background {
            renderer.set_background(image);
        }
        // The playfield / preview clip (fractions) plus the lists' (virtual units).
        let vw = canvas.width.max(1.0);
        let to_fraction = |[x, y, w, h]: [f32; 4]| {
            [
                x / vw,
                y / render::VIRTUAL_HEIGHT,
                w / vw,
                h / render::VIRTUAL_HEIGHT,
            ]
        };
        let opaque = canvas
            .opaque_rect(clip.as_ref().map(|(range, a)| {
                (
                    range,
                    [
                        a.x * vw,
                        a.y * render::VIRTUAL_HEIGHT,
                        a.w * vw,
                        a.h * render::VIRTUAL_HEIGHT,
                    ],
                )
            }))
            .map(to_fraction);
        let mut clips: Vec<(std::ops::Range<usize>, [f32; 4])> = clip
            .into_iter()
            .map(|(range, a)| (range, [a.x, a.y, a.w, a.h]))
            .collect();
        clips.extend(
            canvas
                .clips
                .drain(..)
                .map(|(range, r)| (range, to_fraction(r))),
        );
        renderer.disc = canvas.disc.map(|d| (d, vw));
        renderer.glitch = canvas.glitch;
        let t1 = timing.then(Instant::now);
        let shot = renderer.submit(
            frame,
            &canvas.shapes,
            backdrop,
            capture.is_some(),
            clips,
            opaque,
        );
        if let (Some(t0), Some(t1)) = (t0, t1) {
            let presented = Instant::now();
            // Clock-sample-to-next-refresh: approximates how stale the previous frame was when shown.
            let sample_age = last_sample
                .map(|s| (acquired - s).as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            last_sample = Some(now);
            waits = (
                (acquired - t0).as_secs_f64() * 1000.0,
                (presented - t1).as_secs_f64() * 1000.0,
                sample_age,
            );
            if profile {
                prof_n += 1;
                prof_acquire += waits.0;
                prof_acquire_max = prof_acquire_max.max(waits.0);
                prof_build += (t1 - now).as_secs_f64() * 1000.0;
                prof_submit += (presented - t1).as_secs_f64() * 1000.0;
                prof_shapes = prof_shapes.max(canvas.shapes.len());
                if prof_start.elapsed() >= Duration::from_secs(2) {
                    let n = prof_n.max(1) as f64;
                    let gpu = renderer
                        .take_gpu_ms()
                        .map(|ms| format!("  gpu {ms:.2} ms"))
                        .unwrap_or_default();
                    // Overdraw: the pixels this frame's quads cover, in screens (TAIKO_PROFILE=shapes
                    // also lists the biggest).
                    let ppu = renderer.size().1 as f32 / render::VIRTUAL_HEIGHT;
                    let area = |sh: &render::Shape| {
                        let pad = 1.5 + sh.params[2].max(0.0) * ppu;
                        (sh.half[0] * ppu + pad) * 2.0 * (sh.half[1] * ppu + pad) * 2.0
                    };
                    let screen = (renderer.size().0 * renderer.size().1).max(1) as f32;
                    let cover: f32 = canvas.shapes.iter().map(area).sum::<f32>() / screen;
                    eprintln!(
                        "profile: {:.0} fps  acquire {:.2} ms (max {:.2})  build {:.2} ms  submit {:.2} ms  shapes {}{gpu}  cover {cover:.2}",
                        n / prof_start.elapsed().as_secs_f64(),
                        prof_acquire / n,
                        prof_acquire_max,
                        prof_build / n,
                        prof_submit / n,
                        prof_shapes
                    );
                    if std::env::var("TAIKO_PROFILE").is_ok_and(|v| v == "shapes") {
                        let mut big: Vec<&render::Shape> = canvas.shapes.iter().collect();
                        big.sort_by(|a, b| area(b).total_cmp(&area(a)));
                        for sh in big.iter().take(12) {
                            eprintln!(
                                "  {:.3}  c {:?} h {:?} fill {:?} border {:?} p {:?}",
                                area(sh) / screen,
                                sh.center,
                                sh.half,
                                sh.fill,
                                sh.border,
                                sh.params
                            );
                        }
                    }
                    (prof_start, prof_n, prof_build, prof_submit, prof_shapes) =
                        (Instant::now(), 0, 0.0, 0.0, 0);
                    (prof_acquire, prof_acquire_max) = (0.0, 0.0);
                }
            }
        }
        if let (Some(name), Some((w, h, rgb))) = (capture, shot) {
            if name == SCREENSHOT {
                save_screenshot(&state, w, h, rgb);
            } else if let Some(dir) = lock(&state).shots.as_ref().map(|p| p.dir.clone()) {
                let path = dir.join(name);
                match debug::write_png(&path, w, h, &rgb) {
                    Ok(()) => eprintln!("saved {}", path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
            }
        }

        if hitch {
            let ms = |a: Instant, b: Instant| b.saturating_duration_since(a).as_secs_f64() * 1000.0;
            let done = Instant::now();
            if ms(h_end, done) > 10.0 {
                eprintln!(
                    "hitch {:.1} ms: sleep/loop {:.1} acquire {:.1} export {:.1} lock {:.1} build {:.1} after-build {:.1} submit {:.1}",
                    ms(h_end, done),
                    ms(h_end, h_top),
                    ms(h_top, acquired),
                    ms(acquired, h_lock),
                    ms(h_lock, h_build),
                    ms(h_build, h_built),
                    ms(h_built, t1.unwrap_or(done)),
                    ms(t1.unwrap_or(done), done)
                );
            }
            h_end = done;
        }
        if present != PresentChoice::Fifo && cap > 0 {
            // Fixed grid so frames are evenly spaced; resync if we fell behind.
            let period = Duration::from_secs_f64(1.0 / cap as f64);
            next_frame += period;
            let late = Instant::now();
            if next_frame < late {
                next_frame = late + period;
            }
            sleep_until(next_frame);
        }
    }
    // Closed before the frames counted it: the start went fine all the same.
    gpu_start::going();
    let _ = proxy.send_event(UserEvent::Stopped);
}

pub(super) struct App {
    pub(super) state: Arc<Mutex<State>>,
    /// Last cursor icon set, so it is only changed when the hover target changes.
    pub(super) cursor: CursorIcon,
    /// The pixel pointer (None if the platform refused it: the system one is used).
    pub(super) cursors: Option<cursor::Cursors>,
    /// The accent `cursors` were made in.
    pub(super) cursor_accent: u32,
    pub(super) cursor_hidden: bool,
    pub(super) window: Option<Arc<Window>>,
    /// The window takes input methods (Japanese, ...): only while text is being typed, so a
    /// song's keys never wait on one.
    #[cfg(not(target_os = "android"))]
    pub(super) ime: bool,
    /// Kept to make a surface for each new window (Android, after coming back from the background).
    pub(super) instance: Option<wgpu::Instance>,
    pub(super) render_thread: Option<JoinHandle<()>>,
    pub(super) proxy: EventLoopProxy<UserEvent>,
}

impl App {
    fn set_cursor(&self, icon: CursorIcon) {
        let Some(w) = &self.window else { return };
        match &self.cursors {
            Some(c) => w.set_cursor(c.get(icon)),
            None => w.set_cursor(icon),
        }
    }
}

impl App {
    fn handle_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        // A new UI colour: the pointer is drawn in it too.
        if self.cursors.is_some()
            && self.cursor_accent != render::accent()
            && let Some(window) = &self.window
        {
            self.cursors = cursor::Cursors::new(event_loop, window.scale_factor());
            self.cursor_accent = render::accent();
            self.set_cursor(self.cursor);
        }
        match event {
            WindowEvent::CloseRequested => lock(&self.state).ask_quit(),
            WindowEvent::HoveredFile(_) => lock(&self.state).file_drag(FileDrag::Hover),
            WindowEvent::HoveredFileCancelled => lock(&self.state).file_drag(FileDrag::Left),
            WindowEvent::DroppedFile(path) => {
                lock(&self.state).file_drag(FileDrag::Drop(vec![path]))
            }
            WindowEvent::Resized(size) => self.resized(size),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // Timestamp before anything else, including taking the lock.
                let at = Instant::now();
                if event.state == ElementState::Released
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    lock(&self.state).key_released(code);
                    return;
                }
                // Synthetic presses are keys already held when the window gets focus (the Tab of
                // an Alt+Tab back into the game): not real presses.
                if event.state != ElementState::Pressed || is_synthetic {
                    return;
                }
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => code,
                    // Android's Back button / gesture has no key code, only this name; it does
                    // what Escape does (pause, close, go back).
                    _ if event.logical_key == Key::Named(NamedKey::BrowserBack) => KeyCode::Escape,
                    _ => return,
                };
                let mut s = lock(&self.state);
                // Alt+Tab (switching windows) isn't a Tab press for the game; nor is anything
                // that arrives while another window has focus.
                if !s.focused || (s.modifiers_alt && code == KeyCode::Tab) {
                    return;
                }
                if ime_has_key(&event.logical_key, &s.ime_preedit) {
                    return;
                }
                s.key(
                    code,
                    event.repeat,
                    event.text.as_deref(),
                    at,
                    self.window.as_deref(),
                );
            }
            WindowEvent::Ime(ime) => {
                let mut s = lock(&self.state);
                match ime {
                    winit::event::Ime::Preedit(text, _) => s.ime_preedit = text,
                    winit::event::Ime::Commit(text) => {
                        s.ime_preedit.clear();
                        // Typed like the on-screen keyboard's text (Android): a key with no
                        // meaning of its own, carrying each character.
                        let at = Instant::now();
                        for ch in text.chars().filter(|c| !c.is_control()) {
                            s.key(
                                KeyCode::Lang1,
                                false,
                                Some(&ch.to_string()),
                                at,
                                self.window.as_deref(),
                            );
                        }
                    }
                    winit::event::Ime::Enabled | winit::event::Ime::Disabled => {
                        s.ime_preedit.clear()
                    }
                }
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => self.read_monitor(),
            WindowEvent::Focused(true) => {
                self.read_monitor();
                let mut s = lock(&self.state);
                s.focused = true;
                s.focus_seen = true;
                s.apply_volume();
            }
            WindowEvent::ModifiersChanged(m) => {
                let mut s = lock(&self.state);
                s.modifiers_shift = m.state().shift_key();
                s.modifiers_ctrl = m.state().control_key();
                s.modifiers_alt = m.state().alt_key();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let mut s = lock(&self.state);
                s.wheel_acc += match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    // Touchpads report pixels; ~50 px per notch feels like a wheel.
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 50.0,
                };
                let notches = s.wheel_acc.trunc();
                s.wheel_acc -= notches;
                s.wheel(notches as i32);
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                lock(&self.state).mouse_button(
                    button_state == ElementState::Pressed,
                    self.window.as_deref(),
                );
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: winit::event::MouseButton::Right,
                ..
            } => {
                lock(&self.state).right_button(button_state == ElementState::Pressed);
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: winit::event::MouseButton::Middle,
                ..
            } => {
                lock(&self.state).middle_button(button_state == ElementState::Pressed);
            }
            WindowEvent::Touch(t) => {
                let at = Instant::now();
                let mut s = lock(&self.state);
                let (x, y) = s.to_game(t.location.x, t.location.y);
                s.touch(t.id, t.phase, x, y, at, self.window.as_deref());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let icon = {
                    let mut s = lock(&self.state);
                    let (x, y) = s.to_game(position.x, position.y);
                    s.mouse_moved(x, y)
                };
                if icon != self.cursor {
                    self.cursor = icon;
                    self.set_cursor(icon);
                }
                // Moving brings a hidden pointer back at once, not a frame later.
                if self.cursor_hidden && !lock(&self.state).cursor_hidden(Instant::now()) {
                    self.cursor_hidden = false;
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(true);
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => lock(&self.state).mouse_left(),
            WindowEvent::Focused(false) => {
                // Losing focus mid-song (alt-tab, notification) pauses instead of failing the map.
                let mut guard = lock(&self.state);
                let s = &mut *guard;
                s.focused = false;
                s.apply_volume();
                // A word left half composed is dropped with the focus: kept, it would hold every
                // key for the input method (`ime_has_key`) after coming back.
                s.ime_preedit.clear();
                // The release may happen in another window.
                s.retry_hold = None;
                // The release may happen in another window.
                s.right_scroll = false;
                // Alt is released in the other window; don't keep it "held" here.
                s.modifiers_alt = false;
                if let (true, Some(game), false) =
                    (s.scene.is_play(), s.play.as_deref_mut(), s.auto)
                {
                    game.pause(sound(&s.audio));
                }
            }
            _ => {}
        }
        if self.render_thread.is_none() && lock(&self.state).quit {
            event_loop.exit();
        }
    }

    /// The screen the window is on: its size, modes and refresh. Read again as the window is
    /// resized, moved or focused: on Wayland it isn't known until the window is on screen, and
    /// the window can go to another one.
    fn read_monitor(&self) {
        let Some(m) = self.window.as_ref().and_then(|w| w.current_monitor()) else {
            return;
        };
        let mut s = lock(&self.state);
        s.monitor_size = (m.size().width, m.size().height);
        let modes: Vec<(u32, u32)> = m
            .video_modes()
            .map(|v| (v.size().width, v.size().height))
            .collect();
        if !modes.is_empty() {
            s.monitor_modes = modes;
        }
        if let Some(hz) = m
            .refresh_rate_millihertz()
            .map(|mhz| mhz as f64 / 1000.0)
            .filter(|&hz| hz >= 24.0)
            && (hz - s.refresh_hz).abs() > 0.01
        {
            eprintln!("screen refresh {hz:.2} Hz");
            s.refresh_hz = hz;
            s.refresh_status();
        }
    }

    /// The window's new size (a resize event, or a size requested and granted at once).
    fn resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        self.read_monitor();
        let mut s = lock(&self.state);
        s.want_size = Some((size.width, size.height));
        s.set_window(size.width as f32, size.height as f32);
        s.refresh_presets();
        // Out of fullscreen, the size is remembered for the next start (saved once it stops
        // changing: dragging an edge resizes many times a second).
        if !s.fullscreen && size.width > 0 && size.height > 0 {
            s.cfg.window_size = Some((size.width, size.height));
            s.window_resized_at = Some(Instant::now());
        }
    }

    /// A window size picked in settings: out of fullscreen if need be, then that size.
    /// Lets input methods in while text is being typed, and keeps them out otherwise.
    #[cfg(not(target_os = "android"))]
    fn update_ime(&mut self) {
        let Some(window) = &self.window else { return };
        let mut s = lock(&self.state);
        let want = s.takes_text() && !s.quit_asked && s.splash.is_none();
        if want != self.ime {
            self.ime = want;
            window.set_ime_allowed(want);
            if want {
                // Its candidates go by where the composed text is shown (`ime_preedit`).
                let scale = window.inner_size().height as f64 / render::VIRTUAL_HEIGHT as f64;
                let x = window.inner_size().width as f64 / 2.0;
                window.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(x, 344.0 * scale),
                    winit::dpi::PhysicalSize::new(1.0, 24.0 * scale),
                );
            } else {
                s.ime_preedit.clear();
            }
        }
    }

    fn apply_window_request(&mut self) {
        let Some(window) = &self.window else { return };
        let mut s = lock(&self.state);
        let Some((w, h)) = s.window_request.take() else {
            return;
        };
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
            s.fullscreen = false;
            s.cfg.fullscreen = false;
        }
        let granted = window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
        s.cfg.window_size = Some((w, h));
        s.cfg.save();
        drop(s);
        // Granted at once (Wayland): no resize event comes for it.
        if let Some(size) = granted {
            self.resized(size);
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            // Back from the background (Android): the old window is gone, draw into the new one.
            let Some(instance) = &self.instance else {
                return;
            };
            let size = window.inner_size();
            match instance.create_surface(window.clone()) {
                Ok(surface) => {
                    let mut s = lock(&self.state);
                    s.surface_swap = Some((Some(surface), size.width, size.height));
                    s.set_window(size.width as f32, size.height as f32);
                    s.set_background(false);
                }
                Err(e) => eprintln!("can't draw to the new window: {e}"),
            }
            return;
        }
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut attrs = Window::default_attributes()
            .with_title("taiko.rs")
            .with_window_icon(logo::window_icon())
            .with_min_inner_size(winit::dpi::LogicalSize::new(MIN_WINDOW.0, MIN_WINDOW.1))
            .with_fullscreen(
                lock(&self.state)
                    .cfg
                    .fullscreen
                    .then_some(Fullscreen::Borderless(None)),
            );
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            attrs = attrs.with_name("taiko-rs", "taiko-rs");
        }
        let attrs = match lock(&self.state).cfg.window_size {
            Some((w, h)) => attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h)),
            None => attrs.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(RawWindowHandle::Win32(h)) = window.window_handle().map(|h| h.as_raw()) {
                lock(&self.state).window_handle = Some(h.hwnd.get() as usize);
            }
        }
        #[cfg(target_os = "linux")]
        {
            use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            if let Ok(RawDisplayHandle::Wayland(h)) = window.display_handle().map(|h| h.as_raw()) {
                let proxy = self.proxy.clone();
                wayland_dnd::start(h.display.as_ptr(), move |drag| {
                    let _ = proxy.send_event(UserEvent::FileDrag(drag));
                });
            }
        }
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut api = lock(&self.state).cfg.graphics_api.clone();
        // The last start crashed in the graphics driver: Vulkan from now on (kept, or every other
        // start would crash again).
        #[cfg(windows)]
        if api == "auto" && gpu_start::failed_last_time() {
            eprintln!(
                "graphics: the last start never got going: Vulkan from now on (graphics_api in config.txt)"
            );
            api = "vulkan".into();
            let mut s = lock(&self.state);
            s.cfg.graphics_api = api.clone();
            s.cfg.save();
        }
        // The last run crashed in an overlay or capture hook while drawing with Vulkan: the
        // same, they get along with DX12 (and it would crash again).
        #[cfg(windows)]
        if gpu_start::hook_crashed_last_time() && api == "auto" {
            eprintln!(
                "graphics: the last run crashed in a hook into Vulkan's drawing: DX12 from now on (graphics_api in config.txt)"
            );
            api = "dx12".into();
            let mut s = lock(&self.state);
            s.cfg.graphics_api = api.clone();
            s.cfg.save();
        }
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        ));
        desc.backends = graphics_backends(&api);
        // WGPU_BACKEND still has the last word.
        let instance = wgpu::Instance::new(desc.with_env());
        gpu_start::starting();
        let present = lock(&self.state).cfg.present;
        let renderer = match Renderer::new(&instance, window.clone(), present) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("failed to initialise GPU: {e}");
                // Without a console the window just closed: say why.
                #[cfg(windows)]
                crash_box(&format!(
                    "taiko.rs couldn't start the graphics card, so it closed.\n\n{e}\n\nUpdating the graphics driver usually fixes this. If it doesn't, send this message in #bugs on the taiko.rs Discord."
                ));
                event_loop.exit();
                return;
            }
        };
        let refresh_hz = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| mhz as f64 / 1000.0)
            .unwrap_or(60.0);
        {
            let mut s = lock(&self.state);
            s.adapter = renderer.adapter_name.clone();
            s.set_window(
                window.inner_size().width as f32,
                window.inner_size().height as f32,
            );
            s.monitor_modes = window
                .current_monitor()
                .map(|m| {
                    m.video_modes()
                        .map(|v| (v.size().width, v.size().height))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(m) = window.current_monitor() {
                s.monitor_size = (m.size().width, m.size().height);
            }
            s.refresh_presets();
            s.refresh_hz = refresh_hz;
            s.refresh_status();
            eprintln!(
                "renderer: {}, present: {}, refresh {refresh_hz:.2} Hz",
                renderer.adapter_name,
                renderer.present.name()
            );
            let _ = CRASH_CONTEXT.set(format!(
                "renderer: {}, present: {}, setting graphics_api = {}",
                renderer.adapter_name,
                renderer.present.name(),
                s.cfg.graphics_api
            ));
            VULKAN.store(renderer.backend == wgpu::Backend::Vulkan, Ordering::Relaxed);
            VULKAN_ON_AUTO.store(
                renderer.backend == wgpu::Backend::Vulkan && s.cfg.graphics_api == "auto",
                Ordering::Relaxed,
            );
            // What it could do, not what was asked: OpenGL on Linux has only vsync, and that
            // mustn't become the setting (it stayed vsync after going back to Vulkan).
            s.present = renderer.present;
            s.present_modes = renderer.present_choices();
            s.refresh_hz = refresh_hz;
        }
        let state = self.state.clone();
        let proxy = self.proxy.clone();
        self.render_thread = Some(
            std::thread::Builder::new()
                .name("render".into())
                .spawn(move || render_loop(state, renderer, proxy))
                .expect("spawn render thread"),
        );
        // Input only wakes this thread when there is something to do.
        event_loop.set_control_flow(ControlFlow::Wait);
        self.cursors = cursor::Cursors::new(event_loop, window.scale_factor());
        self.cursor_accent = render::accent();
        self.window = Some(window);
        self.instance = Some(instance);
        self.set_cursor(self.cursor);
    }

    /// Android only: the window is about to be destroyed. The render thread must let go of its
    /// surface first, so wait (briefly) until it has.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        {
            let mut s = lock(&self.state);
            s.surface_swap = Some((None, 0, 0));
            s.set_background(true);
        }
        let deadline = Instant::now() + Duration::from_millis(1000);
        while self
            .render_thread
            .as_ref()
            .is_some_and(|t| !t.is_finished())
            && lock(&self.state).surface_swap.is_some()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Stopped => event_loop.exit(),
            UserEvent::FileDrag(drag) => lock(&self.state).file_drag(drag),
            UserEvent::WindowRequest => self.apply_window_request(),
            UserEvent::Link(link) => {
                lock(&self.state).open_link(link);
                if let Some(w) = &self.window {
                    w.set_minimized(false);
                    w.focus_window();
                }
            }
            UserEvent::Open(path) => {
                lock(&self.state).open_archive(path);
                if let Some(w) = &self.window {
                    w.set_minimized(false);
                    w.focus_window();
                }
            }
            #[cfg(target_os = "android")]
            UserEvent::Typed(typed) => {
                lock(&self.state).soft_keyboard(typed, self.window.as_deref())
            }
            UserEvent::HideCursor(hide) => {
                if hide != self.cursor_hidden {
                    self.cursor_hidden = hide;
                    if let Some(w) = &self.window {
                        w.set_cursor_visible(!hide);
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, event);
        self.apply_window_request();
        #[cfg(not(target_os = "android"))]
        self.update_ime();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        lock(&self.state).quit = true;
        // The Discord status goes with the game (dropping it clears it and says goodbye).
        let discord = lock(&self.state).discord.take();
        drop(discord);
        if let Some(t) = self.render_thread.take() {
            // A vsync'd present can block indefinitely while the window is minimised; don't let
            // that keep the game from closing.
            let deadline = Instant::now() + Duration::from_millis(750);
            while !t.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
        lock(&self.state).cfg.save();
        config::flush_saves();
    }
}

impl State {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        mut cfg: Config,
        audio: Option<Engine>,
        splash: Option<splash::Splash>,
        songs_dir: PathBuf,
        song_dirs: Vec<PathBuf>,
        scan: Option<(Receiver<Scanned>, Arc<AtomicUsize>)>,
        play_when_scanned: bool,
        auto: bool,
    ) -> State {
        lang::set(cfg.language);
        let library = Vec::new();
        let mut select = Select::new(&library);
        let (set_status_tx, set_status_rx) = mpsc::channel();
        let status_cache = setstatus::Cache::load();
        select.sort = ui::SortMode::parse(&cfg.sort).unwrap_or(ui::SortMode::Artist);
        select.headings = cfg.song_headings;
        select.mods_label = cfg.mods.label();
        select.played = load_plays();
        let updater = update::Updater::new(cfg.check_updates);
        let pending_changelog = whats_new(&mut cfg);
        let present = cfg.present;
        let fullscreen = cfg.fullscreen;
        SEND_CRASH_REPORTS.store(cfg.send_crash_reports, Ordering::Relaxed);
        let crash = crash_report();
        let crash_sent = cfg.send_crash_reports
            && crash
                .as_ref()
                .is_some_and(|r| send_crash_report(&cfg.online_server, r));
        // Pictures are on the website of our own server only (and kept on disk between runs).
        let own_server = cfg.online_server == online::DEFAULT_SERVER;
        let avatars = avatars::Avatars::new(
            own_server.then(|| online::WEBSITE.to_string()),
            config::cache_dir()
                .filter(|_| own_server)
                .map(|d| d.join(avatars::DIR)),
        );
        State {
            avatars,
            pictures: pictures::Pictures::default(),
            admin: false,
            crash,
            crash_sent,
            cfg,
            library,
            select,
            scene: SongSelect::scene(),
            play: None,
            calibration_over: false,
            behind: None,
            audio,
            toast: None,
            medals: debug::medals_from_env().map(|view| medals::View {
                name: Some("TAIKO_MEDALS".into()),
                state: medals::State::Loaded(Box::new(view)),
                tab: 0,
                scroll: 0,
            }),
            unlock: {
                let mut unlock = medals::Unlock::default();
                unlock.preview(debug::unlock_from_env());
                unlock
            },
            ime_preedit: String::new(),
            drag_hover: None,
            stats: FrameStats {
                last: Instant::now(),
                frames: 0,
                window_start: Instant::now(),
                fps: 0.0,
                worst_ms: 0.0,
                worst_acc: 0.0,
            },
            status: String::new(),
            modifiers_shift: false,
            modifiers_ctrl: false,
            modifiers_alt: false,
            volume: volume::Overlay::new(),
            volume_save_at: None,
            splash,
            main_menu: None,
            screen_fade: None,
            shown_scene: None,
            menu_notes: Default::default(),
            guide: guide::Tour::default(),
            reset_armed: None,
            music_seeked: None,
            player: None,
            music_paused: false,
            media: None,
            media_told: None,
            window_handle: None,
            updater,
            hits: Vec::new(),
            browse: browse::Browse::default(),
            downloads: Vec::new(),
            set_check: None,
            set_check_due: None,
            #[cfg(target_os = "android")]
            imports_checked: Instant::now(),
            #[cfg(target_os = "android")]
            keyboard: android::Keyboard::default(),
            songs_dir,
            installed_sets: None,
            lobby_lookup: Job::idle(),
            score_link: None,
            preview_remote: false,
            host_transfer: None,
            host_map_asked: false,
            host_asked_at: None,
            auto_fetched: None,
            scores: scores::load_index(),
            pp_backfill: Job::idle(),
            pp_backfill_started: false,
            submits: Vec::new(),
            session_play_ms: 0.0,
            last_key: (KeyCode::Escape, Instant::now()),
            retry_hold: None,
            menu_screen_seen: None,
            menu_changed_at: None,
            replay_speed: 1.0,
            replay_speed_wanted: 1.0,
            replay_speed_rx: Job::idle(),
            select_zone: drum_nav::Zone::List,
            select_rows: drum_nav::Rows::default(),
            me: None,
            me_relax: None,
            me_for: String::new(),
            me_fetch: Job::idle(),
            board_online: false,
            profile: None,
            online_boards: std::collections::HashMap::new(),
            board_fetch: None,
            pending_replay: None,
            changelog: None,
            pending_changelog,
            pending_mods: Mods::NONE,
            mod_select: None,
            universe_setup: None,
            shapeshift_setup: None,
            random_setup: None,
            preset_naming: None,
            collections: Vec::new(),
            collections_read: false,
            preset_deleting: None,
            tact_setup: None,
            od_setup: None,
            leaderboard: Vec::new(),
            board_scroll: 0,
            board_for: (0, false, false),
            board_max: 0,
            board_rect: None,
            split_drag: None,
            split_click: None,
            online_leaderboard: Vec::new(),
            export: None,
            bg_changed: None,
            sprites_upload: None,
            skin_loader: None,
            skin_key: None,
            skin_map: None,
            skin_map_seen: None,
            focused: true,
            right_scroll: false,
            settings_level: None,
            scrollbar_drag: false,
            right_press: None,
            list_drag: None,
            focus_seen: false,
            mapinfo: None,
            mapinfo_sorted: false,
            song_dirs,
            osudb_rx: Job::idle(),
            db_status: Default::default(),
            set_status: status_cache
                .iter()
                .map(|(set, status)| (set, Some(status.to_string())))
                .collect(),
            status_cache,
            status_saved: Instant::now(),
            status_resorted: Instant::now(),
            status_new: false,
            status_sweep_due: None,
            set_status_tx,
            set_status_rx,
            loved_call: None,
            loved_asked: false,
            custom_call: Job::idle(),
            custom_asked: None,
            folder_watch: None,
            editor_back: None,
            test_auto: false,
            playing: None,
            status_wait: None,
            scene_kind: None,
            scene_since: Instant::now(),
            scrubbing: None,
            auto,
            shots: debug::ShotPlan::from_env(),
            pending_screen_shots: vec!["select", "results"],
            exit_when_done: std::env::var_os("TAIKO_EXIT_AFTER").is_some(),
            quit: false,
            quit_asked: false,
            quit_asked_at: None,
            screenshot_due: false,
            smooth: std::env::var_os("TAIKO_STATS")
                .is_some()
                .then(debug::Smoothness::default),
            frame_acquired: None,
            pace_shift_ms: 0.0,
            seen_refresh_ms: None,
            started: Instant::now(),
            want_size: None,
            touch_used: false,
            touch_mouse: None,
            touch_flash: [None; 4],
            surface_swap: None,
            present,
            present_modes: PresentChoice::ALL.to_vec(),
            refresh_hz: 60.0,
            settings_ui: SettingsUi::default(),
            fullscreen,
            adapter: String::new(),
            input_script: debug::InputScript::from_env(),
            wall_shots: debug::wall_shots_from_env(),
            mp: None,
            mp_connecting: None,
            mp_menu: multiplayer::Menu::default(),
            mp_status: String::new(),
            picking_lobby_map: false,
            pick_to_edit: false,
            guessr: None,
            discord: None,
            map_offsets: config::load_map_offsets(),
            play_offset: 0.0,
            social: social::Social::default(),
            stream: None,
            presence: None,
            presence_at: None,
            scan,
            scan_checking: false,
            last_shot: None,
            shot_upload: None,
            shot_view: None,
            folder_pick: Job::idle(),
            folder_pick_for: Item::SongsFolder,
            data_move: None,
            play_when_scanned,
            wheel_acc: 0.0,
            mouse: (0.0, 0.0),
            grip: None,
            hover: None,
            window_w: 1280.0,
            window_h: 720.0,
            real_window: (1280.0, 720.0),
            letterbox: None,
            mob: mobile::Ui::default(),
            backgrounded: false,
            #[cfg(target_os = "android")]
            music_service: false,
            #[cfg(target_os = "android")]
            may_turn: false,
            #[cfg(target_os = "android")]
            media_sent: None,
            safe_insets: None,
            #[cfg(target_os = "android")]
            safe_asked: None,
            frame_clip: None,
            guides: Vec::new(),
            hud_sample: None,
            readout_rect: None,
            replay_bar: None,
            replay_scrub: None,
            replay_bar_grow: 0.0,
            mouse_moved_at: Instant::now(),
            mouse_moved_from: (0.0, 0.0),
            preview: preview::Preview::default(),
            stars: stars::Stars::new(stars::read),
            potentials: pp::Potentials::new(stars::read),
            bg_brightness: 0.0,
            idle: idle::Idle::new(Instant::now()),
            resolutions: Vec::new(),
            monitor_modes: Vec::new(),
            monitor_size: (1920, 1080),
            window_request: None,
            window_resized_at: None,
        }
    }
}
