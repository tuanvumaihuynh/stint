use jiff::{SignedDuration, Timestamp};
use stint_core::{Pace, WindowKind};

/// "5-hour", "7-day · Opus".
pub(crate) fn window_label(kind: WindowKind, scope: Option<&str>) -> String {
    let base = match kind {
        WindowKind::FiveHour => "5-hour",
        WindowKind::SevenDay => "7-day",
        WindowKind::Daily => "Daily",
    };
    match scope {
        Some(scope) => format!("{base} · {scope}"),
        None => base.to_owned(),
    }
}

/// "2h 14m", "3d 4h", "45m", "<1m". Negative durations read as "<1m".
pub(crate) fn duration(span: SignedDuration) -> String {
    let minutes = span.as_secs() / 60;
    if minutes < 1 {
        return "<1m".to_owned();
    }
    let (days, hours, mins) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("{mins}m"),
        (0, _) => format!("{hours}h {mins}m"),
        _ => format!("{days}d {hours}h"),
    }
}

/// "resets in 2h 14m", or "resetting" once the reset time has passed.
pub(crate) fn resets_in(now: Timestamp, resets_at: Timestamp) -> String {
    let left = resets_at.duration_since(now);
    if left <= SignedDuration::ZERO {
        return "resetting".to_owned();
    }
    format!("resets in {}", duration(left))
}

/// "Updated just now", "Updated 22m ago".
pub(crate) fn updated_ago(now: Timestamp, at: Timestamp) -> String {
    let ago = now.duration_since(at);
    if ago < SignedDuration::from_mins(1) {
        return "Updated just now".to_owned();
    }
    format!("Updated {} ago", duration(ago))
}

// Within this many points of an even burn counts as on pace.
const ON_PACE_POINTS: f64 = 3.0;

/// "Limit reached", "12 pts ahead, runs out in 1h 20m", "On pace", "8 pts behind".
pub(crate) fn pace(now: Timestamp, used: f64, pace: &Pace) -> String {
    if used >= 100.0 {
        return "Limit reached".to_owned();
    }
    let points = pace.delta.abs().round();
    if pace.delta.abs() < ON_PACE_POINTS {
        return "On pace".to_owned();
    }
    if pace.delta < 0.0 {
        return format!("{points:.0} pts behind");
    }
    match pace.exhausts_at {
        Some(exhausts_at) => format!(
            "{points:.0} pts ahead, runs out in {}",
            duration(exhausts_at.duration_since(now))
        ),
        None => format!("{points:.0} pts ahead"),
    }
}

#[cfg(test)]
mod tests {
    use stint_core::Percentage;

    use super::*;

    fn now() -> Timestamp {
        "2026-01-01T12:00:00Z".parse().unwrap()
    }

    fn pace_of(delta: f64, exhausts_in: Option<SignedDuration>) -> Pace {
        Pace {
            expected: Percentage::new(50.0).unwrap(),
            delta,
            exhausts_at: exhausts_in.map(|left| now() + left),
        }
    }

    #[test]
    fn labels_windows_with_scope() {
        assert_eq!(window_label(WindowKind::FiveHour, None), "5-hour");
        assert_eq!(
            window_label(WindowKind::SevenDay, Some("Opus")),
            "7-day · Opus"
        );
        assert_eq!(window_label(WindowKind::Daily, None), "Daily");
    }

    #[test]
    fn formats_durations_at_two_units() {
        assert_eq!(duration(SignedDuration::from_secs(30)), "<1m");
        assert_eq!(duration(SignedDuration::from_mins(45)), "45m");
        assert_eq!(duration(SignedDuration::from_mins(134)), "2h 14m");
        assert_eq!(duration(SignedDuration::from_hours(76)), "3d 4h");
        assert_eq!(duration(SignedDuration::from_mins(-5)), "<1m");
    }

    #[test]
    fn reset_countdown() {
        let later = now() + SignedDuration::from_mins(134);
        assert_eq!(resets_in(now(), later), "resets in 2h 14m");
        assert_eq!(resets_in(later, now()), "resetting");
    }

    #[test]
    fn last_update() {
        assert_eq!(updated_ago(now(), now()), "Updated just now");
        let earlier = now() - SignedDuration::from_mins(22);
        assert_eq!(updated_ago(now(), earlier), "Updated 22m ago");
    }

    #[test]
    fn pace_text() {
        assert_eq!(pace(now(), 50.0, &pace_of(2.0, None)), "On pace");
        assert_eq!(pace(now(), 40.0, &pace_of(-8.4, None)), "8 pts behind");
        assert_eq!(pace(now(), 55.0, &pace_of(5.0, None)), "5 pts ahead");
        assert_eq!(
            pace(
                now(),
                62.0,
                &pace_of(12.2, Some(SignedDuration::from_mins(80)))
            ),
            "12 pts ahead, runs out in 1h 20m"
        );
        assert_eq!(pace(now(), 100.0, &pace_of(50.0, None)), "Limit reached");
    }
}
