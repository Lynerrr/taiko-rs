//! `taiko-recalc scores.jsonl recalculated.json FOLDER...`: see the library.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = taiko_recalc::run(&args) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
