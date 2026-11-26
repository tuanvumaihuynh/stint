use jiff::{SignedDuration, Timestamp};
use stint_core::WindowKind;

/// "5-hour", "Weekly", "Daily", "30-day".
pub(crate) fn window_label(kind: WindowKind) -> &'static str {
    match kind {
        WindowKind::FiveHour => "5-hour",
        WindowKind::SevenDay => "Weekly",
        WindowKind::Daily => "Daily",
        WindowKind::ThirtyDay => "30-day",
    }
}

/// "45m", "2h 05m", "3d 4h", "9d", "<1m". Negative durations read as "<1m".
pub(crate) fn duration(span: SignedDuration) -> String {
    let minutes = span.as_secs() / 60;
    if minutes < 1 {
        return "<1m".to_owned();
    }
    let (days, hours, mins) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    match (days, hours) {
        (0, 0) => format!("{mins}m"),
        (0, _) => format!("{hours}h {mins:02}m"),
        (_, 0) => format!("{days}d"),
        _ => format!("{days}d {hours}h"),
    }
}

/// How long ago `at` was in its largest unit, such as "22m", "3h", or "2d", or `None`
/// when it was under a minute ago.
pub(crate) fn age(now: Timestamp, at: Timestamp) -> Option<String> {
    let minutes = now.duration_since(at).as_secs() / 60;
    match minutes {
        ..1 => None,
        1..60 => Some(format!("{minutes}m")),
        60..1440 => Some(format!("{}h", minutes / 60)),
        _ => Some(format!("{}d", minutes / 1440)),
    }
}

/// "1 account", "6 accounts".
pub(crate) fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-01-01T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn labels_windows() {
        assert_eq!(window_label(WindowKind::FiveHour), "5-hour");
        assert_eq!(window_label(WindowKind::SevenDay), "Weekly");
        assert_eq!(window_label(WindowKind::ThirtyDay), "30-day");
    }

    #[test]
    fn formats_durations_at_two_units() {
        assert_eq!(duration(SignedDuration::from_secs(30)), "<1m");
        assert_eq!(duration(SignedDuration::from_mins(45)), "45m");
        assert_eq!(duration(SignedDuration::from_mins(125)), "2h 05m");
        assert_eq!(duration(SignedDuration::from_hours(76)), "3d 4h");
        assert_eq!(duration(SignedDuration::from_hours(9 * 24)), "9d");
        assert_eq!(duration(SignedDuration::from_mins(-5)), "<1m");
    }

    #[test]
    fn age_is_silent_under_a_minute() {
        assert_eq!(age(now(), now() - SignedDuration::from_secs(20)), None);
        assert_eq!(
            age(now(), now() - SignedDuration::from_mins(22)).as_deref(),
            Some("22m")
        );
        assert_eq!(
            age(now(), now() - SignedDuration::from_mins(185)).as_deref(),
            Some("3h")
        );
        assert_eq!(
            age(now(), now() - SignedDuration::from_hours(50)).as_deref(),
            Some("2d")
        );
    }

    #[test]
    fn counts_with_plural() {
        assert_eq!(count(1, "account"), "1 account");
        assert_eq!(count(6, "account"), "6 accounts");
    }
}
