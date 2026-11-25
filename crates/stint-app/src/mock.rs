//! Made-up accounts for development and snapshots, matching the approved menu mockups.
//! Run the app with `STINT_MOCK=<scenario>` in a debug build to use one.

use jiff::{SignedDuration, Timestamp};
use stint_core::{Percentage, ProviderId, QuotaWindow, WindowKind};

use crate::quota::{AccountQuota, FetchError};

/// Every scenario, in the order snapshots are taken.
pub(crate) const SCENARIOS: [&str; 6] = [
    "default",
    "refreshing",
    "single",
    "busy",
    "offline",
    "empty",
];

/// The accounts of a scenario, or `None` for an unknown name.
pub(crate) fn scenario(name: &str, now: Timestamp) -> Option<Vec<AccountQuota>> {
    let accounts = match name {
        "default" | "refreshing" => default(now),
        "single" => default(now).into_iter().take(1).collect(),
        "busy" => busy(now),
        "offline" => offline(now),
        "empty" => Vec::new(),
        _ => return None,
    };
    Some(accounts)
}

/// Whether the scenario starts with a refresh in flight.
pub(crate) fn refreshing(name: &str) -> bool {
    name == "refreshing"
}

fn default(now: Timestamp) -> Vec<AccountQuota> {
    let fresh = Some(now - SignedDuration::from_mins(2));
    vec![
        account(
            ProviderId::Claude,
            "Personal",
            "Max 5x",
            vec![
                window(now, WindowKind::FiveHour, 77.0, 72),
                window(now, WindowKind::SevenDay, 43.0, 2 * 1440 + 780),
            ],
            fresh,
            None,
        ),
        account(
            ProviderId::Claude,
            "Work",
            "Team",
            vec![
                window(now, WindowKind::FiveHour, 20.0, 100),
                window(now, WindowKind::SevenDay, 8.0, 5 * 1440),
            ],
            Some(now - SignedDuration::from_hours(3)),
            Some(FetchError::SessionExpired),
        ),
        account(
            ProviderId::Claude,
            "Side",
            "Pro",
            vec![
                window(now, WindowKind::FiveHour, 8.0, 250),
                window(now, WindowKind::SevenDay, 21.0, 6 * 1440),
            ],
            fresh,
            None,
        ),
        account(
            ProviderId::Codex,
            "Personal",
            "Plus",
            vec![
                window(now, WindowKind::FiveHour, 34.0, 125),
                window(now, WindowKind::SevenDay, 61.0, 3 * 1440),
            ],
            fresh,
            None,
        ),
        account(
            ProviderId::Codex,
            "Work",
            "Team",
            vec![window(now, WindowKind::FiveHour, 92.0, 40)],
            Some(now - SignedDuration::from_mins(6)),
            Some(FetchError::RateLimited {
                retry_after: Some(SignedDuration::from_mins(4)),
            }),
        ),
        account(
            ProviderId::Codex,
            "Free",
            "Free",
            vec![window(now, WindowKind::ThirtyDay, 100.0, 9 * 1440)],
            fresh,
            None,
        ),
    ]
}

fn busy(now: Timestamp) -> Vec<AccountQuota> {
    let fresh = Some(now - SignedDuration::from_mins(2));
    let mut accounts = default(now);
    accounts.insert(
        3,
        account(
            ProviderId::Claude,
            "Client A",
            "Max 20x",
            vec![
                window(now, WindowKind::FiveHour, 52.0, 220),
                window(now, WindowKind::SevenDay, 19.0, 4 * 1440),
            ],
            fresh,
            None,
        ),
    );
    accounts.insert(
        6,
        account(
            ProviderId::Codex,
            "Client B",
            "Pro",
            vec![
                window(now, WindowKind::FiveHour, 91.0, 28),
                window(now, WindowKind::SevenDay, 48.0, 2 * 1440),
            ],
            fresh,
            None,
        ),
    );
    accounts.push(account(
        ProviderId::Gemini,
        "Personal",
        "AI Pro",
        vec![window(now, WindowKind::Daily, 14.0, 230)],
        fresh,
        None,
    ));
    accounts.push(account(
        ProviderId::Gemini,
        "Studio",
        "Ultra",
        Vec::new(),
        None,
        Some(FetchError::NotSignedIn),
    ));
    accounts
}

fn offline(now: Timestamp) -> Vec<AccountQuota> {
    let cached = Some(now - SignedDuration::from_mins(22));
    default(now)
        .into_iter()
        .map(|mut account| {
            // A lapsed sign-in stays lapsed offline; every other account just can't connect.
            if account.error != Some(FetchError::SessionExpired) {
                account.fetched_at = cached;
                account.error = Some(FetchError::Offline);
            }
            account
        })
        .collect()
}

fn account(
    provider: ProviderId,
    name: &str,
    plan: &str,
    windows: Vec<QuotaWindow>,
    fetched_at: Option<Timestamp>,
    error: Option<FetchError>,
) -> AccountQuota {
    AccountQuota {
        provider,
        name: name.to_owned(),
        plan: Some(plan.to_owned()),
        windows,
        fetched_at,
        error,
    }
}

fn window(now: Timestamp, kind: WindowKind, used: f64, resets_in_mins: i64) -> QuotaWindow {
    QuotaWindow {
        kind,
        scope: None,
        // Mock values are all within 0..=100.
        used: Percentage::new(used).unwrap_or(Percentage::ZERO),
        // Half a minute of slack keeps "resets 1h 12m" stable while a snapshot is taken.
        resets_at: Some(
            now + SignedDuration::from_mins(resets_in_mins) + SignedDuration::from_secs(30),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_resolves() {
        let now = Timestamp::now();
        for name in SCENARIOS {
            assert!(scenario(name, now).is_some(), "{name}");
        }
        assert!(scenario("nope", now).is_none());
    }
}
