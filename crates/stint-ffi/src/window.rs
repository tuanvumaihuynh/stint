//! What the desktop window shows, as plain data. The Swift side only draws it.

use jiff::Timestamp;
use stint_core::ProviderId;

use crate::format;
use crate::menu::{self, Entry, MenuData};
use crate::pin::MenuBarStyle;
use crate::quota::{AccountQuota, FetchError};

/// A provider an account can belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

impl From<ProviderId> for Provider {
    fn from(provider: ProviderId) -> Self {
        match provider {
            ProviderId::Claude => Self::Claude,
            ProviderId::Codex => Self::Codex,
            ProviderId::Antigravity => Self::Antigravity,
        }
    }
}

impl From<Provider> for ProviderId {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::Claude => Self::Claude,
            Provider::Codex => Self::Codex,
            Provider::Antigravity => Self::Antigravity,
        }
    }
}

/// How bad an account's problem is: red needs the user, orange fixes itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Severity {
    Error,
    Warning,
}

/// Why an account's numbers are missing or old, as "Session expired — sign in again".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Issue {
    pub title: String,
    pub detail: String,
    pub severity: Severity,
    /// Signing in again is what fixes it.
    pub needs_sign_in: bool,
}

/// One account in the window.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountRow {
    pub id: i64,
    pub provider: Provider,
    pub name: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    /// The last readings, such as "5-hour 77% · Weekly 43% · resets 1h 12m".
    pub readings: String,
    /// Set when the account is failing; the row shows it in place of the readings.
    pub issue: Option<Issue>,
    /// "Updated 2m ago", or `None` before the first numbers.
    pub updated: Option<String>,
    pub shown_in_menu: bool,
}

/// Everything the window draws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Overview {
    /// In provider order, as the menu lists them.
    pub accounts: Vec<AccountRow>,
    /// "5 accounts · 1 needs attention", under the app name in the sidebar.
    pub summary: String,
    /// "Last refreshed 2m ago · Auto-refresh every 5 min".
    pub footer: String,
    /// "Updated 2m ago" for the newest numbers, or `None` before any.
    pub updated: Option<String>,
    pub refreshing: bool,
    pub refresh_minutes: u32,
    /// The account the menu bar shows; `None` only without accounts.
    pub pinned_account: Option<i64>,
    pub menu_bar_style: MenuBarStyle,
}

/// The refresh intervals the window offers, in minutes.
pub(crate) const REFRESH_CHOICES: [u32; 4] = [2, 5, 10, 15];
pub(crate) const DEFAULT_REFRESH_MINUTES: u32 = 5;

pub(crate) fn overview(
    accounts: &[AccountQuota],
    refreshing: bool,
    refresh_minutes: u32,
    now: Timestamp,
) -> Overview {
    let offline = menu::is_offline(accounts);
    let data = MenuData {
        accounts: accounts.to_vec(),
        refreshing,
        pinned: None,
    };
    let summary = match menu::header(&data, offline) {
        Entry::Header { status, .. } => status,
        _ => String::new(),
    };
    let mut rows: Vec<AccountRow> = accounts
        .iter()
        .map(|account| row(account, offline, now))
        .collect();
    rows.sort_by_key(|row| {
        menu::PROVIDERS
            .iter()
            .position(|p| Provider::from(*p) == row.provider)
    });
    Overview {
        accounts: rows,
        summary,
        footer: footer(accounts, refreshing, refresh_minutes, now),
        updated: accounts
            .iter()
            .filter_map(|a| a.fetched_at)
            .max()
            .map(|at| updated(now, at)),
        refreshing,
        refresh_minutes,
        pinned_account: None,
        menu_bar_style: MenuBarStyle::default(),
    }
}

fn row(account: &AccountQuota, offline: bool, now: Timestamp) -> AccountRow {
    AccountRow {
        id: account.id.get(),
        provider: account.provider.into(),
        name: account.name.clone(),
        email: account.email.clone(),
        plan: account.plan.clone(),
        readings: menu::readings(&account.windows, now),
        issue: issue(account, offline, now),
        updated: account.fetched_at.map(|at| updated(now, at)),
        shown_in_menu: account.shown_in_menu,
    }
}

fn updated(now: Timestamp, at: Timestamp) -> String {
    match format::age(now, at) {
        Some(age) => format!("Updated {age} ago"),
        None => "Updated just now".to_owned(),
    }
}

fn issue(account: &AccountQuota, offline: bool, now: Timestamp) -> Option<Issue> {
    let name = menu::provider_name(account.provider);
    let issue = |title: &str, detail: String, severity, needs_sign_in| Issue {
        title: title.to_owned(),
        detail,
        severity,
        needs_sign_in,
    };
    match account.error.as_ref()? {
        // Offline as a whole: the footer says so, and the rows keep their last numbers.
        FetchError::Offline if offline && !account.windows.is_empty() => None,
        FetchError::Offline if offline => Some(issue(
            "Offline",
            "numbers appear once connected".to_owned(),
            Severity::Warning,
            false,
        )),
        FetchError::Offline => Some(issue(
            &format!("Can't reach {name}"),
            "retrying on refresh".to_owned(),
            Severity::Warning,
            false,
        )),
        FetchError::SessionExpired => Some(issue(
            "Session expired",
            "sign in again".to_owned(),
            Severity::Error,
            true,
        )),
        FetchError::NotSignedIn => Some(issue(
            "Not signed in",
            "sign in again".to_owned(),
            Severity::Error,
            true,
        )),
        FetchError::RateLimited { retry_at: Some(at) } if *at > now => Some(issue(
            "Rate limited",
            format!("retry in {}", format::duration(at.duration_since(now))),
            Severity::Warning,
            false,
        )),
        FetchError::RateLimited { .. } => Some(issue(
            "Rate limited",
            "retrying on refresh".to_owned(),
            Severity::Warning,
            false,
        )),
        FetchError::NeedsSetup => Some(issue(
            "Needs setup",
            format!("open {name} once, then refresh"),
            Severity::Warning,
            false,
        )),
    }
}

fn footer(
    accounts: &[AccountQuota],
    refreshing: bool,
    refresh_minutes: u32,
    now: Timestamp,
) -> String {
    let auto = format!("Auto-refresh every {refresh_minutes} min");
    if refreshing {
        return format!("Refreshing… · {auto}");
    }
    let offline = if menu::is_offline(accounts) {
        "Offline · "
    } else {
        ""
    };
    match accounts.iter().filter_map(|a| a.fetched_at).max() {
        None => auto,
        Some(latest) => match format::age(now, latest) {
            Some(age) => format!("{offline}Last refreshed {age} ago · {auto}"),
            None => format!("{offline}Last refreshed just now · {auto}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;
    use crate::mock;

    fn now() -> Timestamp {
        "2026-11-26T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn rows_follow_provider_order_and_carry_issues() {
        let accounts = mock::scenario("default", now()).unwrap();
        let overview = overview(&accounts, false, 5, now());
        assert_eq!(overview.accounts.len(), accounts.len());
        let positions: Vec<usize> = overview
            .accounts
            .iter()
            .map(|row| {
                menu::PROVIDERS
                    .iter()
                    .position(|p| Provider::from(*p) == row.provider)
                    .unwrap()
            })
            .collect();
        assert!(positions.is_sorted());
        assert!(overview.footer.ends_with("Auto-refresh every 5 min"));
    }

    #[test]
    fn expired_sessions_ask_to_sign_in_again() {
        let mut accounts = mock::scenario("default", now()).unwrap();
        accounts.truncate(1);
        accounts[0].error = Some(FetchError::SessionExpired);
        let row = &overview(&accounts, false, 5, now()).accounts[0];
        let issue = row.issue.as_ref().unwrap();
        assert_eq!(issue.title, "Session expired");
        assert!(issue.needs_sign_in);
        assert_eq!(issue.severity, Severity::Error);
    }

    #[test]
    fn footer_reports_the_latest_refresh() {
        let mut accounts = mock::scenario("default", now()).unwrap();
        for account in &mut accounts {
            account.error = None;
            account.fetched_at = Some(now() - SignedDuration::from_mins(2));
        }
        assert_eq!(
            footer(&accounts, false, 10, now()),
            "Last refreshed 2m ago · Auto-refresh every 10 min"
        );
        assert_eq!(
            footer(&accounts, true, 10, now()),
            "Refreshing… · Auto-refresh every 10 min"
        );
        assert_eq!(footer(&[], false, 5, now()), "Auto-refresh every 5 min");
    }
}
