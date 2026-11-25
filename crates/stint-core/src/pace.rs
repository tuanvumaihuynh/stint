use jiff::{SignedDuration, Timestamp};

use crate::quota::{Percentage, QuotaWindow};

/// Burn-rate pace for one quota window at a point in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    /// Usage a perfectly even burn would have reached by now.
    pub expected: Percentage,
    /// `used - expected` in percentage points; positive means burning too fast.
    pub delta: f64,
    /// When the window runs out at the current average rate, if that happens
    /// before it resets.
    pub exhausts_at: Option<Timestamp>,
}

impl Pace {
    /// Returns `None` when the window has no reset time to measure against.
    #[must_use]
    pub fn of(window: &QuotaWindow, now: Timestamp) -> Option<Self> {
        let resets_at = window.resets_at?;
        let length = window.kind.duration();
        let elapsed = (length - resets_at.duration_since(now)).clamp(SignedDuration::ZERO, length);

        let elapsed_secs = elapsed.as_secs_f64();
        let fraction = elapsed_secs / length.as_secs_f64();
        let expected = Percentage::new(fraction * 100.0).ok()?;
        let used = window.used.get();

        Some(Self {
            expected,
            delta: used - expected.get(),
            exhausts_at: exhausts_at(used, elapsed_secs, now, resets_at),
        })
    }

    #[must_use]
    pub fn is_ahead(&self) -> bool {
        self.delta > 0.0
    }
}

fn exhausts_at(
    used: f64,
    elapsed_secs: f64,
    now: Timestamp,
    resets_at: Timestamp,
) -> Option<Timestamp> {
    if used >= 100.0 {
        return Some(now);
    }
    if used <= 0.0 || elapsed_secs <= 0.0 {
        return None;
    }
    let secs_left = (100.0 - used) * elapsed_secs / used;
    let left = SignedDuration::try_from_secs_f64(secs_left).ok()?;
    let at = now.checked_add(left).ok()?;
    (at < resets_at).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::WindowKind;

    fn now() -> Timestamp {
        "2026-01-01T12:00:00Z".parse::<Timestamp>().unwrap()
    }

    fn five_hour(used: f64, resets_in: SignedDuration) -> QuotaWindow {
        QuotaWindow {
            kind: WindowKind::FiveHour,
            scope: None,
            used: Percentage::new(used).unwrap(),
            resets_at: Some(now() + resets_in),
        }
    }

    fn pace(used: f64, resets_in: SignedDuration) -> Pace {
        Pace::of(&five_hour(used, resets_in), now()).unwrap()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn halfway_through_window_expects_half_usage() {
        let pace = pace(50.0, SignedDuration::from_mins(150));
        assert_close(pace.expected.get(), 50.0);
        assert_close(pace.delta, 0.0);
        assert_eq!(pace.exhausts_at, None);
    }

    #[test]
    fn fast_burn_projects_exhaustion_before_reset() {
        // 1 hour elapsed, 40% used: 40%/h leaves 60% for 1.5 h.
        let pace = pace(40.0, SignedDuration::from_hours(4));
        assert_close(pace.expected.get(), 20.0);
        assert_close(pace.delta, 20.0);
        assert!(pace.is_ahead());
        assert_eq!(
            pace.exhausts_at,
            Some(now() + SignedDuration::from_mins(90))
        );
    }

    #[test]
    fn exhausted_window_reports_now() {
        assert_eq!(
            pace(100.0, SignedDuration::from_hours(1)).exhausts_at,
            Some(now())
        );
    }

    #[test]
    fn idle_window_never_exhausts() {
        let pace = pace(0.0, SignedDuration::from_hours(2));
        assert!(!pace.is_ahead());
        assert_eq!(pace.exhausts_at, None);
    }

    #[test]
    fn stale_reset_time_clamps_to_full_window() {
        assert_close(
            pace(10.0, SignedDuration::from_hours(-1)).expected.get(),
            100.0,
        );
    }

    #[test]
    fn window_without_reset_has_no_pace() {
        let mut window = five_hour(10.0, SignedDuration::from_hours(1));
        window.resets_at = None;
        assert_eq!(Pace::of(&window, now()), None);
    }
}
