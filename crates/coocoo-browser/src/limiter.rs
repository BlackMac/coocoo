//! Screencast frame-rate cap.
//!
//! Frames arriving faster than the cap are not forwarded immediately; the newest one is held
//! and flushed when the interval elapses (trailing edge). Without the trailing flush a page that
//! stops changing right after a burst would leave viewers on a stale frame, because headless
//! Chromium only produces screencast frames when the content changes.

use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug)]
pub(crate) struct FpsLimiter {
    interval: Duration,
    last: Option<Instant>,
}

impl FpsLimiter {
    /// `max_fps == 0` disables the cap.
    pub fn new(max_fps: u32) -> Self {
        let interval = if max_fps == 0 { Duration::ZERO } else { Duration::from_secs(1) / max_fps };
        Self { interval, last: None }
    }

    /// `None`: forward now (recorded as sent). `Some(wait)`: hold the frame and flush after `wait`.
    pub fn check(&mut self, now: Instant) -> Option<Duration> {
        if let Some(last) = self.last {
            let next = last + self.interval;
            if now < next {
                return Some(next - now);
            }
        }
        self.last = Some(now);
        None
    }

    /// Record a deferred frame as sent.
    pub fn mark_sent(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_rate_and_reports_wait() {
        let t0 = Instant::now();
        let mut lim = FpsLimiter::new(10); // 100 ms
        assert_eq!(lim.check(t0), None);
        assert_eq!(lim.check(t0 + Duration::from_millis(30)), Some(Duration::from_millis(70)));
        assert_eq!(lim.check(t0 + Duration::from_millis(99)), Some(Duration::from_millis(1)));
        assert_eq!(lim.check(t0 + Duration::from_millis(100)), None);
        // Deferred flush counts as a send.
        assert!(lim.check(t0 + Duration::from_millis(150)).is_some());
        lim.mark_sent(t0 + Duration::from_millis(200));
        assert!(lim.check(t0 + Duration::from_millis(250)).is_some());
        assert_eq!(lim.check(t0 + Duration::from_millis(300)), None);
    }

    #[test]
    fn zero_disables_cap() {
        let t0 = Instant::now();
        let mut lim = FpsLimiter::new(0);
        for _ in 0..5 {
            assert_eq!(lim.check(t0), None);
        }
    }

    #[test]
    fn counts_frames_over_one_second() {
        let t0 = Instant::now();
        let mut lim = FpsLimiter::new(12);
        // 60 fps input for one second.
        let sent = (0..60).filter(|i| lim.check(t0 + Duration::from_micros(i * 16_667)).is_none()).count();
        assert!((11..=12).contains(&sent), "sent {sent}");
    }
}
