use jiff::Timestamp;
use serde::Deserialize;
use stint_core::{Percentage, QuotaWindow, WindowKind};

use crate::error::ProviderError;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Response {
    five_hour: Option<NamedWindow>,
    seven_day: Option<NamedWindow>,
    limits: Vec<Limit>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NamedWindow {
    // Percent used, 0 to 100; above 100 once a limit is exceeded.
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Limit {
    kind: Option<String>,
    percent: Option<f64>,
    resets_at: Option<String>,
    scope: Option<Scope>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Scope {
    model: Option<Named>,
    surface: Option<Named>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Named {
    display_name: Option<String>,
}

/// Parses the OAuth usage response into quota windows.
///
/// Newer responses list every limit in `limits`; older ones only have the top-level
/// `five_hour` and `seven_day` objects, which are read when `limits` yields nothing.
/// Rows with an unknown kind or an unusable percent are skipped.
pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<QuotaWindow>, ProviderError> {
    let response: Response = serde_json::from_slice(bytes)?;

    let windows: Vec<QuotaWindow> = response.limits.iter().filter_map(from_limit).collect();
    if !windows.is_empty() {
        return Ok(windows);
    }

    let named = [
        (WindowKind::FiveHour, response.five_hour),
        (WindowKind::SevenDay, response.seven_day),
    ];
    let windows = named
        .into_iter()
        .filter_map(|(kind, window)| from_named(kind, window.as_ref()?))
        .collect();
    Ok(windows)
}

fn from_limit(limit: &Limit) -> Option<QuotaWindow> {
    let kind = window_kind(limit.kind.as_deref()?)?;
    let used = percentage(limit.percent?)?;
    let scope = limit.scope.as_ref().and_then(|scope| {
        let named = scope.model.as_ref().or(scope.surface.as_ref())?;
        named.display_name.clone()
    });

    Some(QuotaWindow {
        kind,
        scope,
        used,
        resets_at: timestamp(limit.resets_at.as_deref()),
    })
}

fn from_named(kind: WindowKind, window: &NamedWindow) -> Option<QuotaWindow> {
    Some(QuotaWindow {
        kind,
        scope: None,
        used: percentage(window.utilization?)?,
        resets_at: timestamp(window.resets_at.as_deref()),
    })
}

fn window_kind(kind: &str) -> Option<WindowKind> {
    match kind {
        "session" | "five_hour" => Some(WindowKind::FiveHour),
        "seven_day" => Some(WindowKind::SevenDay),
        weekly if weekly.starts_with("weekly_") => Some(WindowKind::SevenDay),
        _ => None,
    }
}

// Negative or non-finite percents are invalid; values above 100 clamp to 100.
fn percentage(value: f64) -> Option<Percentage> {
    if value < 0.0 {
        return None;
    }
    Percentage::new(value).ok()
}

fn timestamp(value: Option<&str>) -> Option<Timestamp> {
    value?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: &str = include_str!("testdata/usage_limits.json");
    const NAMED_ONLY: &str = include_str!("testdata/usage_named_only.json");

    fn ts(value: &str) -> Timestamp {
        value.parse().unwrap()
    }

    fn pct(value: f64) -> Percentage {
        Percentage::new(value).unwrap()
    }

    #[test]
    fn reads_every_limit_row() {
        let windows = parse(LIMITS.as_bytes()).unwrap();
        assert_eq!(
            windows,
            vec![
                QuotaWindow {
                    kind: WindowKind::FiveHour,
                    scope: None,
                    used: pct(62.0),
                    resets_at: Some(ts("2026-09-20T22:20:00.095287+00:00")),
                },
                QuotaWindow {
                    kind: WindowKind::SevenDay,
                    scope: None,
                    used: pct(48.0),
                    resets_at: Some(ts("2026-09-27T02:00:00.095306+00:00")),
                },
                QuotaWindow {
                    kind: WindowKind::SevenDay,
                    scope: Some("Opus".into()),
                    used: pct(15.0),
                    resets_at: Some(ts("2026-09-27T02:00:00+00:00")),
                },
                QuotaWindow {
                    kind: WindowKind::SevenDay,
                    scope: Some("Claude Code".into()),
                    used: Percentage::FULL,
                    resets_at: None,
                },
            ]
        );
    }

    #[test]
    fn falls_back_to_named_windows() {
        let windows = parse(NAMED_ONLY.as_bytes()).unwrap();
        assert_eq!(
            windows,
            vec![
                QuotaWindow {
                    kind: WindowKind::FiveHour,
                    scope: None,
                    used: pct(7.5),
                    resets_at: Some(ts("2026-09-20T22:20:00+00:00")),
                },
                QuotaWindow {
                    kind: WindowKind::SevenDay,
                    scope: None,
                    used: pct(30.0),
                    resets_at: None,
                },
            ]
        );
    }

    #[test]
    fn empty_response_has_no_windows() {
        assert_eq!(parse(b"{}").unwrap(), Vec::new());
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(parse(b"[1,"), Err(ProviderError::Decode(_))));
    }
}
