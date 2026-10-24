// Release builds on Windows are a GUI app: no console window behind the game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    taiko_rs::desktop_main();
}
