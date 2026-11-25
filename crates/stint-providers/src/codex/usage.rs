use jiff::Timestamp;
use serde::Deserialize;
use stint_core::{Percentage, QuotaWindow, WindowKind};

use crate::error::ProviderError;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Response {
    account_id: Option<String>,
    rate_limit: Option<RateLimit>,
    additional_rate_limits: Vec<AdditionalLimit>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AdditionalLimit {
    limit_name: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Window {
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    reset_at: Option<Moment>,
}

/// Unix seconds as the endpoint sends them, or an RFC 3339 string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Moment {
    Seconds(i64),
    Text(String),
}

/// Parses the usage response for the account `account_id`.
///
/// The plan's own limits come from `rate_limit`; per-model limits from
/// `additional_rate_limits`, scoped to their `limit_name`. A window's kind comes from its
/// length, not from whether it is the primary or secondary one: the primary window is the
/// weekly one on plans without a five-hour limit.
pub(crate) fn parse(bytes: &[u8], account_id: &str) -> Result<Vec<QuotaWindow>, ProviderError> {
    let response: Response = serde_json::from_slice(bytes)?;
    if response
        .account_id
        .as_deref()
        .is_some_and(|answered| answered != account_id)
    {
        return Err(ProviderError::AccountMismatch);
    }

    let mut windows = windows_of(response.rate_limit.as_ref(), None);
    for limit in &response.additional_rate_limits {
        let Some(name) = limit.limit_name.as_deref().filter(|name| !name.is_empty()) else {
            continue;
        };
        windows.extend(windows_of(limit.rate_limit.as_ref(), Some(name)));
    }
    Ok(windows)
}

fn windows_of(limit: Option<&RateLimit>, scope: Option<&str>) -> Vec<QuotaWindow> {
    let Some(limit) = limit else {
        return Vec::new();
    };
    [&limit.primary_window, &limit.secondary_window]
        .into_iter()
        .filter_map(|window| from_window(window.as_ref()?, scope))
        .collect()
}

fn from_window(window: &Window, scope: Option<&str>) -> Option<QuotaWindow> {
    let kind = window_kind(window.limit_window_seconds?)?;
    let percent = window.used_percent?;
    if percent < 0.0 {
        return None;
    }

    Some(QuotaWindow {
        kind,
        scope: scope.map(str::to_owned),
        used: Percentage::new(percent).ok()?,
        resets_at: window.reset_at.as_ref().and_then(timestamp),
    })
}

fn window_kind(seconds: i64) -> Option<WindowKind> {
    match seconds {
        18_000 => Some(WindowKind::FiveHour),
        86_400 => Some(WindowKind::Daily),
        604_800 => Some(WindowKind::SevenDay),
        _ => None,
    }
}

// Seconds past the year 5000 would mean the field changed to milliseconds; refuse them.
const MAX_SECONDS: i64 = 100_000_000_000;

fn timestamp(moment: &Moment) -> Option<Timestamp> {
    match moment {
        Moment::Seconds(seconds) if (1..MAX_SECONDS).contains(seconds) => {
            Timestamp::from_second(*seconds).ok()
        }
        Moment::Seconds(_) => None,
        Moment::Text(text) => text.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: &str = include_str!("testdata/usage.json");
    const WEEKLY_ONLY: &str = include_str!("testdata/usage_weekly_only.json");
    const ACCOUNT: &str = "acct_fake_123";

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(seconds).unwrap()
    }

    fn pct(value: f64) -> Percentage {
        Percentage::new(value).unwrap()
    }

    #[test]
    fn reads_plan_and_model_windows() {
        let windows = parse(USAGE.as_bytes(), ACCOUNT).unwrap();
        assert_eq!(
            windows,
            vec![
                QuotaWindow {
                    kind: WindowKind::FiveHour,
                    scope: None,
                    used: pct(18.0),
                    resets_at: Some(at(1_790_100_000)),
                },
                QuotaWindow {
                    kind: WindowKind::SevenDay,
                    scope: None,
                    used: pct(45.5),
                    resets_at: Some("2026-09-27T02:00:00Z".parse().unwrap()),
                },
                QuotaWindow {
                    kind: WindowKind::FiveHour,
                    scope: Some("codex_other".into()),
                    used: Percentage::FULL,
                    resets_at: None,
                },
            ]
        );
    }

    #[test]
    fn kind_follows_window_length_not_slot() {
        let windows = parse(WEEKLY_ONLY.as_bytes(), ACCOUNT).unwrap();
        assert_eq!(
            windows,
            vec![QuotaWindow {
                kind: WindowKind::SevenDay,
                scope: None,
                used: pct(3.0),
                resets_at: Some(at(1_790_628_078)),
            }]
        );
    }

    #[test]
    fn answer_for_another_account_is_rejected() {
        assert!(matches!(
            parse(USAGE.as_bytes(), "acct_someone_else"),
            Err(ProviderError::AccountMismatch)
        ));
    }

    #[test]
    fn answer_without_account_or_limits_is_empty() {
        let body = br#"{"rate_limit":null,"additional_rate_limits":[]}"#;
        assert_eq!(parse(body, ACCOUNT).unwrap(), Vec::new());
    }

    #[test]
    fn malformed_json_is_decode_error() {
        assert!(matches!(
            parse(b"nope", ACCOUNT),
            Err(ProviderError::Decode(_))
        ));
    }
}
