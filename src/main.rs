// Release builds on Windows are a GUI app: no console window behind the game.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--recalc-pp") {
        if let Err(e) = taiko_rs::recalc_pp(&args[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    taiko_rs::desktop_main();
}
