//! The system clipboard: Ctrl+V pastes into whatever text is being typed, Ctrl+C copies, and
//! F12 can put a screenshot on it.
//! arboard does it natively (Windows, macOS, X11 and Wayland); on Linux the usual command line
//! tools stand in when it can't (a Wayland compositor without the data-control protocol).

#[cfg(not(target_os = "android"))]
use std::sync::Mutex;

/// One clipboard for the whole game: on X11 and Wayland what's copied is served by its owner,
/// so it has to outlive the copy.
#[cfg(not(target_os = "android"))]
static BOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

#[cfg(not(target_os = "android"))]
fn with_board<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let mut board = BOARD.lock().unwrap_or_else(|e| e.into_inner());
    if board.is_none() {
        *board = arboard::Clipboard::new().ok();
    }
    f(board.as_mut()?)
}

/// The clipboard's text, if it has any.
pub fn get() -> Option<String> {
    #[cfg(not(target_os = "android"))]
    if let Some(text) = with_board(|b| b.get_text().ok()) {
        return Some(text);
    }
    tool(&[("wl-paste", &["--no-newline"]), ("xclip", &["-o", "-selection", "clipboard"]), ("xsel", &["-ob"])], None)
}

/// Puts `text` on the clipboard; false if there's none to put it on.
pub fn set(text: &str) -> bool {
    #[cfg(not(target_os = "android"))]
    if with_board(|b| b.set_text(text).ok()).is_some() {
        return true;
    }
    tool(&[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["-ib"])], Some(text.as_bytes())).is_some()
}

/// Puts a `w` x `h` RGB picture on the clipboard; false if there's none to put it on.
pub fn set_image(w: u32, h: u32, rgb: &[u8]) -> bool {
    #[cfg(not(target_os = "android"))]
    {
        let rgba: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let image = arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into() };
        if with_board(|b| b.set_image(image).ok()).is_some() {
            return true;
        }
    }
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    if image::ImageEncoder::write_image(encoder, rgb, w, h, image::ExtendedColorType::Rgb8).is_err() {
        return false;
    }
    tool(&[("wl-copy", &["--type", "image/png"]), ("xclip", &["-selection", "clipboard", "-t", "image/png"])], Some(&png)).is_some()
}

/// The first of the clipboard tools that works (Linux only): reading its output, or writing
/// `input` to it.
#[allow(unused_variables)]
fn tool(tries: &[(&str, &[&str])], input: Option<&[u8]>) -> Option<String> {
    #[cfg(all(unix, not(target_os = "android"), not(target_os = "macos")))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for (cmd, args) in tries {
            let mut c = Command::new(cmd);
            c.args(*args).stderr(Stdio::null());
            if let Some(text) = input {
                let Ok(mut child) = c.stdin(Stdio::piped()).stdout(Stdio::null()).spawn() else { continue };
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text);
                }
                // wl-copy stays behind to serve it; don't wait for it.
                return Some(String::new());
            }
            if let Ok(out) = c.stdin(Stdio::null()).output() {
                if out.status.success() {
                    return Some(String::from_utf8_lossy(&out.stdout).into_owned());
                }
            }
        }
    }
    None
}

/// Text pasted into a one-line field: its first line, without control characters.
pub fn one_line(text: &str) -> String {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().filter(|c| !c.is_control()).take(1000).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_text_is_one_clean_line() {
        assert_eq!(one_line("\n  hello\tworld \nsecond"), "helloworld");
        assert_eq!(one_line(""), "");
    }
}
