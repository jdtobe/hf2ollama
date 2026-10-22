//! Two-step "arm, then confirm" for destructive buttons. Pure state plus one
//! egui input helper; shared by the Installed delete and Clear cached files.

#![allow(dead_code)] // stubs until the Installed view uses it

use std::time::{Duration, Instant};

/// How long the "Confirm" state stays.
pub const ARM_WINDOW: Duration = Duration::from_secs(4);
/// A click on the armed button is ignored until this long after arming, so a
/// double-click can never confirm.
pub const ARM_MIN_DELAY: Duration = Duration::from_millis(600);

/// At most one armed key at a time (a row name, or a fixed id for a button).
#[derive(Debug, Default)]
pub struct Confirm {
    armed: Option<(String, Instant)>,
}

impl Confirm {
    pub fn arm(&mut self, key: &str, now: Instant) {
        let _ = (key, now);
    }

    /// True while `key` shows its confirm state.
    pub fn is_armed(&self, key: &str, now: Instant) -> bool {
        let _ = (key, now);
        false
    }

    /// True when a click on `key` would confirm: armed, past the minimum
    /// delay and inside the window.
    pub fn ready(&self, key: &str, now: Instant) -> bool {
        let _ = (key, now);
        false
    }

    /// Consumes the armed state when a click on `key` may confirm.
    pub fn confirm(&mut self, key: &str, now: Instant) -> bool {
        let _ = (key, now);
        false
    }

    /// One click: arms, or confirms when already armed. True only on confirm.
    pub fn click(&mut self, key: &str, now: Instant) -> bool {
        let _ = (key, now);
        false
    }

    /// Drops the armed key when `live` no longer knows it.
    pub fn disarm_unless(&mut self, live: impl Fn(&str) -> bool) {
        let _ = live;
    }

    /// Clears an armed state whose window ran out.
    pub fn expire(&mut self, now: Instant) {
        let _ = now;
    }

    /// When the next repaint is due: the button turning ready, then expiry.
    pub fn next_due(&self, now: Instant) -> Option<Instant> {
        let _ = now;
        None
    }
}

/// True when this frame carries a key-repeat of Enter or Space. A held key
/// repeats `clicked()` on a focused button; only a fresh press may arm or
/// confirm.
pub fn activation_key_held(ui: &eframe::egui::Ui) -> bool {
    let _ = ui;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_click_arms_and_never_confirms() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        assert!(!c.click("a", t0));
        assert!(c.is_armed("a", t0));
        assert!(!c.is_armed("b", t0));
    }

    #[test]
    fn click_inside_min_delay_is_ignored_and_does_not_rearm() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.click("a", t0);
        for n in [0, 100, 599] {
            let now = t0 + ms(n);
            assert!(!c.click("a", now), "{n} ms");
            assert!(!c.confirm("a", now), "{n} ms");
            assert!(c.is_armed("a", now), "{n} ms");
            assert!(!c.ready("a", now), "{n} ms");
        }
        assert!(c.ready("a", t0 + ARM_MIN_DELAY), "delay was not restarted");
        assert!(!c.is_armed("a", t0 + ARM_WINDOW), "window was not extended");
    }

    #[test]
    fn click_after_the_delay_confirms_once() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.click("a", t0);
        assert!(c.click("a", t0 + ARM_MIN_DELAY));
        assert!(!c.is_armed("a", t0 + ARM_MIN_DELAY), "single use");
        assert!(!c.confirm("a", t0 + ARM_MIN_DELAY));
    }

    #[test]
    fn window_expires_after_4_seconds_and_a_late_click_rearms() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.arm("a", t0);
        assert!(c.is_armed("a", t0 + ms(3_999)));
        assert!(!c.is_armed("a", t0 + ARM_WINDOW));
        let late = t0 + Duration::from_secs(5);
        assert!(!c.click("a", late), "re-arms");
        assert!(c.is_armed("a", late));
        assert!(!c.ready("a", late), "delay restarted");
        assert!(c.click("a", late + ARM_MIN_DELAY));
    }

    #[test]
    fn arming_another_key_disarms_the_first_and_wrong_key_does_not_disarm() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.arm("a", t0);
        assert!(!c.confirm("b", t0 + ARM_MIN_DELAY), "other key");
        assert!(c.is_armed("a", t0 + ARM_MIN_DELAY), "still armed");
        c.arm("b", t0 + ms(100));
        assert!(!c.is_armed("a", t0 + ms(100)));
        assert!(c.is_armed("b", t0 + ms(100)));
        assert!(!c.confirm("a", t0 + ms(900)));
    }

    #[test]
    fn keys_are_compared_exactly() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.arm("Qwen2.5:7b", t0);
        let t1 = t0 + ARM_MIN_DELAY;
        assert!(!c.confirm("qwen2.5:7b", t1));
        assert!(!c.confirm("Qwen2.5", t1));
        assert!(c.confirm("Qwen2.5:7b", t1));
    }

    #[test]
    fn next_due_is_ready_time_then_expiry_and_expire_clears() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        assert_eq!(c.next_due(t0), None);
        c.arm("a", t0);
        assert_eq!(c.next_due(t0), Some(t0 + ARM_MIN_DELAY));
        assert_eq!(c.next_due(t0 + ARM_MIN_DELAY), Some(t0 + ARM_WINDOW));
        c.expire(t0 + ms(3_999));
        assert!(c.is_armed("a", t0 + ms(3_999)));
        c.expire(t0 + ARM_WINDOW);
        assert_eq!(c.next_due(t0 + ARM_WINDOW), None);
    }

    #[test]
    fn disarm_unless_drops_a_key_that_is_gone() {
        let mut c = Confirm::default();
        let t0 = Instant::now();
        c.arm("a", t0);
        c.disarm_unless(|k| k == "a");
        assert!(c.is_armed("a", t0));
        c.disarm_unless(|k| k == "b");
        assert!(!c.is_armed("a", t0));
    }

    #[test]
    fn repeat_of_enter_or_space_counts_as_held_but_a_fresh_press_does_not() {
        use eframe::egui;
        let held = |key: egui::Key, repeat: bool| {
            let ctx = egui::Context::default();
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let mut out = false;
            ctx.run_ui(input, |ui| out = activation_key_held(ui))
                .drop_without_applying_deltas();
            out
        };
        assert!(held(egui::Key::Enter, true));
        assert!(held(egui::Key::Space, true));
        assert!(!held(egui::Key::Enter, false));
        assert!(!held(egui::Key::A, true));
    }
}
