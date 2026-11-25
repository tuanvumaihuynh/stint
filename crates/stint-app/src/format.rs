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

/// "updated just now", "updated 22m ago".
pub(crate) fn updated_ago(now: Timestamp, at: Timestamp) -> String {
    let ago = now.duration_since(at);
    if ago < SignedDuration::from_mins(1) {
        return "updated just now".to_owned();
    }
    format!("updated {} ago", duration(ago))
}

// Within this many points of an even burn counts as on pace.
const ON_PACE_POINTS: f64 = 5.0;

/// "on pace", "12 pts ahead · runs out in 1h 20m", "8 pts under".
pub(crate) fn pace(now: Timestamp, pace: &Pace) -> String {
    let points = pace.delta.abs().round();
    let mut text = if pace.delta.abs() < ON_PACE_POINTS {
        "on pace".to_owned()
    } else if pace.delta > 0.0 {
        format!("{points:.0} pts ahead")
    } else {
        format!("{points:.0} pts under")
    };
    if let Some(exhausts_at) = pace.exhausts_at {
        text.push_str(" · runs out in ");
        text.push_str(&duration(exhausts_at.duration_since(now)));
    }
    text
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
        assert_eq!(updated_ago(now(), now()), "updated just now");
        let earlier = now() - SignedDuration::from_mins(22);
        assert_eq!(updated_ago(now(), earlier), "updated 22m ago");
    }

    #[test]
    fn pace_text() {
        assert_eq!(pace(now(), &pace_of(2.0, None)), "on pace");
        assert_eq!(pace(now(), &pace_of(-8.4, None)), "8 pts under");
        assert_eq!(
            pace(now(), &pace_of(12.2, Some(SignedDuration::from_mins(80)))),
            "12 pts ahead · runs out in 1h 20m"
        );
    }
}
