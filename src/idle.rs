//! The idle screen: after a while with nothing pressed or moved, the interface fades away, the
//! background blurs and darkens, and the song's disc turns in the middle of the screen. Any key,
//! click, wheel or move brings it all back (the press that wakes it does nothing else).

use std::time::{Duration, Instant};

use crate::hit::Canvas;
use crate::player;
use crate::render::rgba;

/// The choices for how long until it shows (minutes; 0 is never).
pub const MINUTES: [u32; 5] = [0, 1, 2, 5, 10];
/// Fading in, and back out when woken (ms).
const FADE_IN_MS: f32 = 1600.0;
const FADE_OUT_MS: f32 = 300.0;
/// How much darker the background gets, at most.
pub const DARKEN: f32 = 0.55;
/// The disc's radius on the idle screen, and where its centre is (the rays reach
/// `player::RAYS_REACH` further, the song's name under them).
const DISC_R: f32 = 100.0;
const DISC_Y: f32 = 196.0;
/// How fast it turns (radians a second), a little slower than the player's.
const SPIN: f32 = 0.4;

/// A choice's name in settings.
pub fn minutes_name(minutes: u32) -> String {
    match minutes {
        0 => tr!("Off", "オフ", "Выкл").into(),
        1 => tr!("After 1 minute", "1分後", "Через 1 минуту").into(),
        m => {
            let ru = crate::lang::ru_plural(m as usize, ["минуту", "минуты", "минут"]);
            tr!("After {m} minutes", "{m}分後", "Через {m} {ru}";)
        }
    }
}

pub struct Idle {
    last_input: Instant,
    /// 0 (not showing) to 1 (all the way in), before easing.
    shown: f32,
    angle: f32,
    /// A press woke it: its release is swallowed too.
    pub swallow_left: bool,
    pub swallow_right: bool,
    /// The music's spectrum around the disc, fed by the caller while it shows.
    pub spectrum: player::Spectrum,
    /// `TAIKO_IDLE_MS` (tests): the wait in ms, whatever the setting says.
    after_override: Option<Duration>,
}

impl Idle {
    pub fn new(now: Instant) -> Idle {
        let after_override = std::env::var("TAIKO_IDLE_MS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .map(Duration::from_millis);
        Idle {
            last_input: now,
            shown: 0.0,
            angle: 0.0,
            swallow_left: false,
            swallow_right: false,
            spectrum: player::Spectrum::default(),
            after_override,
        }
    }

    /// Something was pressed or moved. True when the idle screen was showing: that input only
    /// wakes the game.
    pub fn input(&mut self, now: Instant) -> bool {
        self.last_input = now;
        self.shown > 0.05
    }

    /// Moves the fade on by `dt_ms`. `allowed`: this screen may go idle (not in a song, no
    /// dialog up); while it isn't, the wait starts over. Returns the eased amount shown, 0 to 1.
    pub fn update(&mut self, now: Instant, dt_ms: f32, allowed: bool, minutes: u32) -> f32 {
        if !allowed {
            self.last_input = now;
        }
        let after = self
            .after_override
            .or((minutes > 0).then(|| Duration::from_secs(minutes as u64 * 60)));
        let want =
            allowed && after.is_some_and(|a| now.saturating_duration_since(self.last_input) >= a);
        let step = dt_ms.max(0.0) / if want { FADE_IN_MS } else { FADE_OUT_MS };
        self.shown = if want {
            (self.shown + step).min(1.0)
        } else {
            (self.shown - step).max(0.0)
        };
        if self.shown > 0.0 {
            self.angle += SPIN * dt_ms / 1000.0;
        }
        self.amount()
    }

    /// The eased amount shown, 0 to 1.
    pub fn amount(&self) -> f32 {
        let t = self.shown;
        t * t * (3.0 - 2.0 * t)
    }

    /// Over a frame already drawn: fades everything in it by `amount`, then draws the disc
    /// (pictured: the renderer puts the background in it) and the song under it.
    pub fn draw(&self, c: &mut Canvas, pictured: bool, song: Option<(&str, &str)>) {
        let a = self.amount();
        if a <= 0.0 {
            return;
        }
        for sh in &mut c.shapes {
            sh.fill[3] *= 1.0 - a;
            sh.border[3] *= 1.0 - a;
        }
        // Nothing under the pointer once it has faded: the press that wakes it does nothing.
        if a > 0.5 {
            c.hits.clear();
        }
        // The disc grows in a little behind the fade, so the screen has cleared first.
        let grow = ((a - 0.25) / 0.75).clamp(0.0, 1.0);
        let grow = 1.0 - (1.0 - grow).powi(3);
        let (x, r) = (c.width / 2.0, DISC_R * (0.6 + 0.4 * grow));
        // The player's own disc (if it's open) makes way for this one.
        c.disc = None;
        if grow <= 0.0 {
            return;
        }
        self.spectrum.draw_rays(c, x, DISC_Y, r, grow);
        player::draw_disc_sized(c, x, DISC_Y, r, self.angle, pictured, grow);
        if pictured {
            c.disc = Some([x, DISC_Y, r * (player::DISC_RIM + 0.02), self.angle]);
        }
        if let Some((title, artist)) = song {
            let y = DISC_Y + DISC_R + player::RAYS_REACH + 12.0;
            let room = c.width - 64.0;
            let size = Canvas::fit_size(14.0, 9.0, room, title);
            c.text_centered(
                x,
                y,
                size,
                rgba(0xffffff, 0.9 * grow),
                &Canvas::fit(size, room, title),
            );
            c.text_centered(
                x,
                y + size + 10.0,
                9.0,
                rgba(0xb0b0c8, 0.8 * grow),
                &Canvas::fit(9.0, room, artist),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_shows_only_after_the_wait_and_input_brings_it_back() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        idle.after_override = None;
        assert_eq!(
            idle.update(t0 + Duration::from_secs(59), 16.0, true, 1),
            0.0
        );
        let mut now = t0 + Duration::from_secs(61);
        for _ in 0..200 {
            now += Duration::from_millis(16);
            idle.update(now, 16.0, true, 1);
        }
        assert_eq!(idle.amount(), 1.0);
        assert!(
            idle.input(now),
            "the input that wakes it is reported as waking it"
        );
        for _ in 0..40 {
            now += Duration::from_millis(16);
            idle.update(now, 16.0, true, 1);
        }
        assert_eq!(idle.amount(), 0.0);
        assert!(
            !idle.input(now),
            "input while it's gone does what it always does"
        );
    }

    #[test]
    fn never_when_off_or_not_allowed() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        idle.after_override = None;
        let late = t0 + Duration::from_secs(3600);
        assert_eq!(idle.update(late, 5000.0, true, 0), 0.0, "0 minutes is off");
        assert_eq!(
            idle.update(late, 5000.0, false, 1),
            0.0,
            "not on this screen"
        );
        // Being kept from it (in a song) starts the wait over.
        assert_eq!(
            idle.update(late + Duration::from_secs(30), 5000.0, true, 1),
            0.0
        );
    }
}
