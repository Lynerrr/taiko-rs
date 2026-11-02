//! Mail from the server: codes for creating an account, linking an email to one and resetting a
//! password. The server doesn't speak SMTP itself (it sits behind a home connection, whose port
//! 25 no one takes mail from): it keeps the letters in an outbox, and the German VPS's mailer
//! (`deploy/setup-mail.sh`) takes them one at a time from `/api/mail/next` through the site's
//! tunnel and hands them to its Postfix, which signs and sends them as noreply@auneu.space.
//!
//! DIR/mail-key holds the mailer's key (`Authorization: Bearer`); without it mail is off and
//! accounts are made without an email, as before.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Letters waiting at most (a flood of sign-ups can't fill the memory).
const MAX_OUTBOX: usize = 500;
/// How long a code works, and how many wrong tries it takes.
const CODE_LIFE: Duration = Duration::from_secs(15 * 60);
const CODE_TRIES: u32 = 5;
/// Codes sent for one thing (an account's sign-up, reset...): one a minute; to one email, or for
/// one thing, a few an hour.
const PER_MINUTE: usize = 1;
const PER_HOUR: usize = 6;

struct Mail {
    dir: PathBuf,
    from: String,
    outbox: Mutex<VecDeque<String>>,
    codes: Mutex<HashMap<String, Pending>>,
    sent: Mutex<HashMap<String, Vec<Instant>>>,
}

/// A code waiting to be typed in: what it's for (`for_`, e.g. "reg:name") and to which email.
struct Pending {
    code: String,
    email: String,
    made: Instant,
    tries: u32,
}

static MAIL: OnceLock<Mail> = OnceLock::new();

pub fn init(dir: PathBuf) {
    let from = std::fs::read_to_string(dir.join("mail-from")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "noreply@auneu.space".into());
    let mail = Mail { dir, from, outbox: Mutex::default(), codes: Mutex::default(), sent: Mutex::default() };
    println!("mail {}", if key().is_some() { format!("on, from {}", mail.from) } else { "off (no mail-key)".into() });
    let _ = MAIL.set(mail);
}

fn key() -> Option<String> {
    let m = MAIL.get()?;
    std::fs::read_to_string(m.dir.join("mail-key")).ok().map(|k| k.trim().to_string()).filter(|k| k.len() >= 16)
}

/// Whether letters can be sent (the mailer's key is set up): then new accounts need an email.
pub fn on() -> bool {
    key().is_some()
}

/// Whether `bearer` is the mailer's key (compared in the same time whatever matches).
pub fn mailer_key(bearer: &str) -> bool {
    key().is_some_and(|want| want.len() == bearer.len() && want.bytes().zip(bearer.bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0)
}

/// The oldest letter waiting (taken out: the mailer hands it to Postfix, which keeps it).
pub fn next() -> Option<String> {
    MAIL.get()?.outbox.lock().unwrap().pop_front()
}

/// Whether `email` looks like one: name@domain.tld, nothing odd in it.
pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else { return false };
    (3..=254).contains(&email.len())
        && !local.is_empty()
        && local.len() <= 64
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && email.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '<' | '>' | '(' | ')' | ',' | ';' | ':' | '\\' | '"' | '[' | ']'))
        && !domain.contains('@')
}

/// The same email however it was typed: trimmed, lowercase.
pub fn normal(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// `email` with most of its name hidden: "g*****@gmail.com".
pub fn masked(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}{}@{domain}", "*".repeat(local.chars().count().saturating_sub(1).clamp(1, 6)))
        }
        None => "***".into(),
    }
}

fn six_digits() -> String {
    let mut b = [0u8; 8];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).expect("/dev/urandom");
    format!("{:06}", u64::from_le_bytes(b) % 1_000_000)
}

/// Why a code wasn't sent: the last one is still on its way.
pub const JUST_SENT: &str = "a code was just sent: wait a minute before asking again";

/// Mails a new code for `for_` (what it unlocks, e.g. "reg:name") to `email`; `what` says in the
/// letter what it's for. Err: too many asked for lately, or mail is off.
pub fn send_code(for_: &str, email: &str, what: &str) -> Result<(), String> {
    let m = MAIL.get().filter(|_| on()).ok_or("the server can't send mail right now")?;
    {
        let mut sent = m.sent.lock().unwrap();
        sent.retain(|_, v| {
            v.retain(|t| t.elapsed() < Duration::from_secs(3600));
            !v.is_empty()
        });
        for who in [normal(email), for_.to_string()] {
            let times = sent.get(&who).map(Vec::as_slice).unwrap_or_default();
            // A minute apart for the same thing; another (a sign-up, then a reset) may follow.
            if who == for_ && times.iter().filter(|t| t.elapsed() < Duration::from_secs(60)).count() >= PER_MINUTE {
                return Err(JUST_SENT.into());
            }
            if times.len() >= PER_HOUR {
                return Err("too many codes asked for, try again in an hour".into());
            }
        }
        for who in [normal(email), for_.to_string()] {
            sent.entry(who).or_default().push(Instant::now());
        }
    }
    let code = six_digits();
    let body = format!(
        "Your taiko.rs code {what}:\n\n    {code}\n\nType it where you asked for it. It works for 15 minutes.\n\nIf you didn't ask for it, ignore this letter: nothing changes without the code.\n\n-- \ntaiko.rs  https://rstaiko.auneu.space\n"
    );
    let letter = letter(&m.from, email, &format!("taiko.rs code: {code}"), &body, &html(&code, what));
    let mut outbox = m.outbox.lock().unwrap();
    if outbox.len() >= MAX_OUTBOX {
        return Err("the server is sending a lot of mail right now, try again in a few minutes".into());
    }
    outbox.push_back(letter);
    drop(outbox);
    let mut codes = m.codes.lock().unwrap();
    codes.retain(|_, p| p.made.elapsed() < CODE_LIFE);
    codes.insert(for_.to_string(), Pending { code, email: normal(email), made: Instant::now(), tries: 0 });
    Ok(())
}

/// Checks the code typed for `for_`: the email it was sent to when right (the code is used up),
/// else why not. Wrong ones count; after a few the code is gone.
pub fn check_code(for_: &str, code: &str) -> Result<String, String> {
    let m = MAIL.get().ok_or("the server can't send mail right now")?;
    let mut codes = m.codes.lock().unwrap();
    let Some(p) = codes.get_mut(for_).filter(|p| p.made.elapsed() < CODE_LIFE) else {
        return Err("that code has expired: ask for a new one".into());
    };
    let typed: String = code.chars().filter(char::is_ascii_digit).collect();
    if typed != p.code {
        p.tries += 1;
        if p.tries >= CODE_TRIES {
            codes.remove(for_);
            return Err("wrong code too many times: ask for a new one".into());
        }
        return Err("wrong code".into());
    }
    Ok(codes.remove(for_).map(|p| p.email).unwrap_or_default())
}

/// The email a code for `for_` is waiting at, if one is.
pub fn pending_email(for_: &str) -> Option<String> {
    MAIL.get()?.codes.lock().unwrap().get(for_).filter(|p| p.made.elapsed() < CODE_LIFE).map(|p| p.email.clone())
}

/// A letter with a plain text and an HTML version (mail apps show the last one they can), as
/// Postfix's `sendmail -t` takes it (lines end in \n there).
fn letter(from: &str, to: &str, subject: &str, text: &str, html: &str) -> String {
    let now = chrono_date();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = format!("{nanos}.{}@{}", six_digits(), from.split('@').nth(1).unwrap_or("localhost"));
    let b = format!("=_taiko_{nanos}_{}", six_digits());
    format!(
        "From: taiko.rs <{from}>\nTo: <{to}>\nSubject: {subject}\nDate: {now}\nMessage-ID: <{id}>\nMIME-Version: 1.0\nContent-Type: multipart/alternative; boundary=\"{b}\"\nAuto-Submitted: auto-generated\n\n--{b}\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n{text}\n--{b}\nContent-Type: text/html; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n{html}\n--{b}--\n"
    )
}

/// The game's 5x7 font (`src/font.rs`) for the characters the HTML letter draws: five columns,
/// the lowest bit the top row.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        '1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        '2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        '3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        '4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        '5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        '6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        '7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        '8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        '9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        '-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        _ => [0; 5],
    }
}

/// `text` in the game's pixel font, drawn with table cells (mail apps take tables, not web fonts):
/// white, with the site's red and blue fringes left and right half a pixel wide, so each font
/// pixel is 2x2 cells of `half` px. `tight` drops a glyph's empty side columns (the logo's I),
/// else every glyph is as wide (the code's digits). Each row is its own table of runs of one
/// colour, which keeps the letter small, and each cell goes on its own line: mail lines can't be
/// longer than 998 characters.
fn pixels(text: &str, half: u32, tight: bool) -> String {
    let mut cols = vec![0u8];
    for (i, c) in text.chars().enumerate() {
        if i > 0 {
            cols.push(0);
        }
        cols.extend(glyph(c).into_iter().filter(|&col| !tight || col != 0));
    }
    cols.push(0);
    let width = cols.len() * 2;
    let mut out = String::new();
    for sub_row in 0..14 {
        let row = sub_row / 2;
        let on = |x: usize| x < width && cols[x / 2] >> row & 1 == 1;
        let colour = |x: usize| {
            if on(x) {
                "#ffffff"
            } else if on(x + 1) {
                "#EB452C"
            } else if x > 0 && on(x - 1) {
                "#4fb4ff"
            } else {
                ""
            }
        };
        out.push_str(&format!("<table role=\"presentation\" width=\"{}\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"border-collapse:collapse;margin:0 auto;font-size:0;line-height:0\"><tr>\n", width as u32 * half));
        let mut x = 0;
        while x < width {
            let c = colour(x);
            let mut n = 1;
            while x + n < width && colour(x + n) == c {
                n += 1;
            }
            let bg = if c.is_empty() { String::new() } else { format!(" bgcolor=\"{c}\"") };
            out.push_str(&format!("<td width=\"{w}\" height=\"{half}\"{bg}></td>\n", w = n as u32 * half));
            x += n;
        }
        out.push_str("</tr></table>\n");
    }
    out
}

/// `s` safe inside HTML text.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The letter's HTML version, dressed like the game and its site: the pixel logo, a lane of
/// notes, the code big in pixels and again as text to copy.
fn html(code: &str, what: &str) -> String {
    let what = escape(what);
    let code = escape(code);
    let logo = pixels("TAIKO-RS", 3, true);
    let digits = pixels(&code, 4, false);
    let mono = "font-family:'Courier New',Courier,monospace";
    let note = |color: &str, size: u32| {
        format!("<td valign=\"middle\" style=\"padding:0 3px\"><div style=\"width:{size}px;height:{size}px;border-radius:50%;background:{color};border:2px solid #ffffff\"></div></td>\n")
    };
    let (d, k) = ("#EB452C", "#438EAD");
    let judge = "<td valign=\"middle\" style=\"padding:0 8px 0 0\"><div style=\"width:22px;height:22px;border-radius:50%;border:2px solid #4a4a60\"></div></td>\n";
    let lane: String = [judge.to_string(), note(d, 14), note(k, 14), note(d, 14), note(d, 22), note(k, 14), note(k, 14), note(k, 22)].concat();
    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="dark"><meta name="supported-color-schemes" content="dark"><title>taiko.rs code</title></head>
<body style="margin:0;padding:0;background:#0b0b10">
<div style="display:none;max-height:0;overflow:hidden;mso-hide:all">{code} is your taiko.rs code {what}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="#0b0b10" style="background:#0b0b10"><tr><td align="center" style="padding:32px 12px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="#12121a" style="max-width:480px;border-collapse:collapse;background:#12121a;border:2px solid #3a3a4e">
<tr><td align="center" style="padding:36px 16px 28px">
{logo}
</td></tr>
<tr><td bgcolor="#1b1b26" style="background:#1b1b26;border-top:3px solid #3a3a4e;border-bottom:3px solid #3a3a4e;padding:10px 16px">
<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
{lane}</tr></table>
</td></tr>
<tr><td align="center" style="padding:30px 24px 16px;{mono};font-size:15px;line-height:22px;color:#c8c8d8">Your code {what}</td></tr>
<tr><td align="center" style="padding:4px 16px 10px">
{digits}
</td></tr>
<tr><td align="center" style="padding:6px 24px 0;{mono};font-size:22px;line-height:30px;font-weight:bold;letter-spacing:6px;color:#ffffff">{code}</td></tr>
<tr><td align="center" style="padding:14px 24px 30px;{mono};font-size:13px;line-height:20px;color:#8a8aa0">Type it where you asked for it.<br>It works for <span style="color:#EB452C">15 minutes</span>.</td></tr>
<tr><td style="border-top:2px solid #3a3a4e;padding:16px 24px 20px;{mono};font-size:12px;line-height:18px;color:#6a6a80">If you didn't ask for it, ignore this letter: nothing changes without the code.<br><br>
<a href="https://rstaiko.auneu.space" style="color:#4fb4ff;text-decoration:none">rstaiko.auneu.space</a></td></tr>
</table>
</td></tr></table>
</body></html>"##
    )
}

/// Now as an RFC 5322 date, in UTC.
fn chrono_date() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86_400;
    let (h, mi, s) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
    // 1970-01-01 was a Thursday.
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize];
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize];
    format!("{weekday}, {d:02} {month} {y} {h:02}:{mi:02}:{s:02} +0000")
}

#[cfg(test)]
pub fn init_for_tests(dir: PathBuf) {
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mail-key"), "0123456789abcdef0123").unwrap();
    init(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_checked_and_masked() {
        for good in ["a@b.co", "galileo.985+taiko@gmail.com", "x_y@mail.yandex.ru"] {
            assert!(valid_email(good), "{good}");
        }
        for bad in ["", "a@b", "@b.co", "a b@c.de", "a@b..co", "a@.b.co", "a@b.co.", "<a@b.co>", "a@b@c.co", "a,b@c.de"] {
            assert!(!valid_email(bad), "{bad}");
        }
        assert_eq!(masked("galileo@gmail.com"), "g******@gmail.com");
        assert_eq!(masked("ab@x.io"), "a*@x.io");
        assert_eq!(normal("  Me@Mail.RU "), "me@mail.ru");
    }

    #[test]
    fn a_letter_has_text_and_html_and_short_lines() {
        let l = letter("noreply@a.co", "me@b.co", "taiko.rs code: 012345", "text 012345\n", &html("012345", "for creating the account <b>"));
        let b = l.split("boundary=\"").nth(1).unwrap().split('"').next().unwrap();
        assert_eq!(l.matches(&format!("--{b}\n")).count(), 2, "{l}");
        assert!(l.ends_with(&format!("--{b}--\n")));
        assert!(l.contains("text/plain") && l.contains("text/html") && l.contains("text 012345"));
        assert!(l.contains("account &lt;b&gt;") && !l.contains("account <b>"));
        assert!(l.lines().all(|line| line.len() < 998), "a line is too long for mail");
        if let Ok(path) = std::env::var("TAIKO_MAIL_PREVIEW") {
            std::fs::write(path, html("042137", "for creating the account Dekovner")).unwrap();
        }
    }

    #[test]
    fn a_date_reads_like_mail_wants() {
        let d = chrono_date();
        assert!(d.ends_with(" +0000") && d.len() == "Thu, 01 Jan 1970 00:00:00 +0000".len(), "{d}");
    }
}
