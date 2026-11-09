//! The app shell: starting the game, its loop, the graphics' start and crash reports.

pub(crate) mod crash;
pub(crate) mod gpu;
pub(crate) mod run_loop;
pub(crate) mod shell;

#[cfg(test)]
mod tests {
    /// The shell's pieces stay in their own files: lib.rs is the game's state and screens.
    #[test]
    fn the_app_shell_lives_in_app() {
        let lib = include_str!("../lib.rs");
        for (piece, home) in [
            ("fn render_loop(", "run_loop.rs"),
            ("struct App {", "run_loop.rs"),
            ("fn desktop_main(", "shell.rs"),
            ("fn run(", "shell.rs"),
            ("fn log_crashes(", "crash.rs"),
            ("mod native_crashes", "crash.rs"),
            ("fn send_crash_report(", "crash.rs"),
            ("mod gpu_start", "gpu.rs"),
            ("fn graphics_backends(", "gpu.rs"),
        ] {
            assert!(!lib.contains(piece), "`{piece}` is back in lib.rs");
            let file = match home {
                "run_loop.rs" => include_str!("run_loop.rs"),
                "shell.rs" => include_str!("shell.rs"),
                "crash.rs" => include_str!("crash.rs"),
                _ => include_str!("gpu.rs"),
            };
            assert!(file.contains(piece), "`{piece}` isn't in app/{home}");
        }
    }

    /// What only the shell uses is the shell's own: the rest of the game can't reach the loop,
    /// the app's window state or the crash window.
    #[test]
    fn the_shell_keeps_its_loop_and_crash_window_to_itself() {
        for (file, pieces) in [
            (
                include_str!("run_loop.rs"),
                &["struct App {", "fn render_loop(", "fn sleep_until("][..],
            ),
            (
                include_str!("crash.rs"),
                &["fn log_crashes(", "fn crash_report(", "fn crash_box("][..],
            ),
            (
                include_str!("gpu.rs"),
                &["fn drawing_hook(", "static VULKAN:"][..],
            ),
        ] {
            for piece in pieces {
                assert!(
                    file.contains(&format!("pub(super) {piece}")),
                    "`{piece}` is open to more than app/"
                );
            }
        }
    }
}
