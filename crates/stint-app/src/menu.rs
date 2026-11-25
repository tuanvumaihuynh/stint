//! What the status menu shows, as plain data. The AppKit side only draws these entries.

use jiff::Timestamp;
use stint_core::{ProviderId, QuotaWindow, WindowKind};

use crate::format;
use crate::quota::{AccountQuota, FetchError};

/// Everything the menu is drawn from.
#[derive(Debug, Clone, Default)]
pub(crate) struct MenuData {
    pub(crate) accounts: Vec<AccountQuota>,
    pub(crate) refreshing: bool,
}

/// One row of the menu, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// App icon, "Stint", and a one-line status after a colored dot.
    Header {
        status: String,
        dot: Dot,
    },
    /// A provider name above its accounts.
    Section(&'static str),
    /// "Personal · Max 5x" over its readings. It has no action.
    Account {
        name: String,
        plan: Option<String>,
        subtitle: String,
    },
    /// A passive two-line message, such as the empty state.
    Notice {
        title: &'static str,
        subtitle: &'static str,
    },
    Command {
        command: Command,
        subtitle: Option<String>,
        enabled: bool,
    },
    Separator,
}

/// The header dot: healthy, something needs attention, or nothing live to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dot {
    Healthy,
    Attention,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Refresh,
    AddAccount,
    OpenStint,
    Settings,
    Quit,
}

impl Command {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Refresh => "Refresh",
            Self::AddAccount => "Add Account…",
            Self::OpenStint => "Open Stint",
            Self::Settings => "Settings…",
            Self::Quit => "Quit Stint",
        }
    }

    /// The key that, with ⌘, runs the command while the menu is open.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Refresh => "r",
            Self::AddAccount => "",
            Self::OpenStint => "o",
            Self::Settings => ",",
            Self::Quit => "q",
        }
    }
}

const PROVIDERS: [ProviderId; 3] = [ProviderId::Claude, ProviderId::Codex, ProviderId::Gemini];

/// The menu rows for `data` as of `now`.
pub(crate) fn entries(data: &MenuData, now: Timestamp) -> Vec<Entry> {
    let accounts = &data.accounts;
    let offline = is_offline(accounts);
    let mut rows = vec![header(data, offline), Entry::Separator];

    if accounts.is_empty() {
        rows.push(Entry::Notice {
            title: "No accounts yet",
            subtitle: "Add an account to start tracking usage.",
        });
        rows.push(command(Command::AddAccount, None, true));
    } else {
        for provider in PROVIDERS {
            let mut group = accounts
                .iter()
                .filter(|a| a.provider == provider)
                .peekable();
            if group.peek().is_none() {
                continue;
            }
            rows.push(Entry::Section(provider_name(provider)));
            rows.extend(group.map(|account| Entry::Account {
                name: account.name.clone(),
                plan: account.plan.clone(),
                subtitle: subtitle(account, offline, now),
            }));
            rows.push(Entry::Separator);
        }
        rows.push(command(
            Command::Refresh,
            Some(refresh_subtitle(data, offline, now)),
            !data.refreshing,
        ));
    }

    rows.extend([
        Entry::Separator,
        command(Command::OpenStint, None, true),
        command(Command::Settings, None, true),
        Entry::Separator,
        command(Command::Quit, None, true),
    ]);
    rows
}

fn command(command: Command, subtitle: Option<String>, enabled: bool) -> Entry {
    Entry::Command {
        command,
        subtitle,
        enabled,
    }
}

/// No account could connect, so the whole app is offline rather than one account. Lapsed
/// sign-ins stay lapsed offline, so they do not count against this.
pub(crate) fn is_offline(accounts: &[AccountQuota]) -> bool {
    let offline = |a: &&AccountQuota| a.error == Some(FetchError::Offline);
    let expired = |a: &&AccountQuota| a.error == Some(FetchError::SessionExpired);
    accounts.iter().any(|a| offline(&a)) && accounts.iter().all(|a| offline(&a) || expired(&a))
}

fn header(data: &MenuData, offline: bool) -> Entry {
    let total = data.accounts.len();
    let accounts = format::count(total, "account");
    let attention = data
        .accounts
        .iter()
        .filter(|a| needs_attention(a, offline))
        .count();

    let (status, dot) = if total == 0 {
        ("No accounts connected".to_owned(), Dot::Idle)
    } else if data.refreshing {
        (format!("Refreshing {accounts}…"), Dot::Idle)
    } else if offline {
        (format!("Offline · {accounts}"), Dot::Idle)
    } else if attention > 0 {
        let verb = if attention == 1 { "needs" } else { "need" };
        (
            format!("{accounts} · {attention} {verb} attention"),
            Dot::Attention,
        )
    } else {
        (format!("Tracking {accounts}"), Dot::Healthy)
    };
    Entry::Header { status, dot }
}

/// An account the user has to act on, or that is failing on its own. Being signed out of
/// a provider and the whole app being offline are not counted.
fn needs_attention(account: &AccountQuota, offline: bool) -> bool {
    match &account.error {
        None | Some(FetchError::NotSignedIn) => false,
        Some(FetchError::Offline) => !offline,
        Some(_) => true,
    }
}

fn subtitle(account: &AccountQuota, offline: bool, now: Timestamp) -> String {
    let name = provider_name(account.provider);
    match &account.error {
        None => readings(&account.windows, now),
        // Offline as a whole: show the last numbers, the header and Refresh say they are old.
        Some(FetchError::Offline) if offline && !account.windows.is_empty() => {
            readings(&account.windows, now)
        }
        Some(FetchError::Offline) if offline => "Offline".to_owned(),
        Some(FetchError::Offline) => format!("Can't reach {name} · retrying on refresh"),
        Some(FetchError::SessionExpired) => "Session expired — sign in again".to_owned(),
        Some(FetchError::NotSignedIn) => "Not signed in".to_owned(),
        Some(FetchError::RateLimited {
            retry_after: Some(wait),
        }) => format!("Rate limited · retry in {}", format::duration(*wait)),
        Some(FetchError::RateLimited { retry_after: None }) => "Rate limited".to_owned(),
    }
}

/// "5-hour 77% · Weekly 43% · resets 1h 12m", or "30-day 100% · Limit reached · resets 9d".
fn readings(windows: &[QuotaWindow], now: Timestamp) -> String {
    let shown = shown_windows(windows);
    if shown.is_empty() {
        return "No usage reported yet".to_owned();
    }

    let mut parts: Vec<String> = shown
        .iter()
        .map(|w| format!("{} {:.0}%", format::window_label(w.kind), w.used.get()))
        .collect();
    if shown.iter().any(|w| w.used.get() >= 100.0) {
        parts.push("Limit reached".to_owned());
    }
    let soonest = shown
        .iter()
        .filter_map(|w| w.resets_at)
        .filter(|at| *at > now)
        .min();
    if let Some(at) = soonest {
        parts.push(format!(
            "resets {}",
            format::duration(at.duration_since(now))
        ));
    }
    parts.join(" · ")
}

/// The plan-wide 5-hour and weekly windows; plans without those show what they have.
fn shown_windows(windows: &[QuotaWindow]) -> Vec<&QuotaWindow> {
    let plan_wide = || windows.iter().filter(|w| w.scope.is_none());
    let main: Vec<&QuotaWindow> = [WindowKind::FiveHour, WindowKind::SevenDay]
        .into_iter()
        .filter_map(|kind| plan_wide().find(|w| w.kind == kind))
        .collect();
    if main.is_empty() {
        plan_wide().collect()
    } else {
        main
    }
}

fn refresh_subtitle(data: &MenuData, offline: bool, now: Timestamp) -> String {
    if data.refreshing {
        return "Refreshing…".to_owned();
    }
    let latest = data.accounts.iter().filter_map(|a| a.fetched_at).max();
    let Some(latest) = latest else {
        return "Not updated yet".to_owned();
    };
    match (offline, format::age(now, latest)) {
        (true, Some(age)) => format!("Offline · showing {age}-old numbers"),
        (true, None) => "Offline · showing the last numbers".to_owned(),
        (false, Some(age)) => format!("Updated {age} ago"),
        (false, None) => "Updated just now".to_owned(),
    }
}

pub(crate) fn provider_name(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock;

    fn now() -> Timestamp {
        "2026-11-25T13:00:00Z".parse().unwrap()
    }

    fn data(scenario: &str) -> MenuData {
        MenuData {
            accounts: mock::scenario(scenario, now()).unwrap(),
            refreshing: mock::refreshing(scenario),
        }
    }

    fn accounts(rows: &[Entry]) -> Vec<(String, String)> {
        rows.iter()
            .filter_map(|row| match row {
                Entry::Account {
                    name,
                    plan,
                    subtitle,
                } => {
                    let title = match plan {
                        Some(plan) => format!("{name} · {plan}"),
                        None => name.clone(),
                    };
                    Some((title, subtitle.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn header_of(rows: &[Entry]) -> (String, Dot) {
        match &rows[0] {
            Entry::Header { status, dot } => (status.clone(), *dot),
            other => panic!("first row is {other:?}"),
        }
    }

    fn refresh_of(rows: &[Entry]) -> (Option<String>, bool) {
        rows.iter()
            .find_map(|row| match row {
                Entry::Command {
                    command: Command::Refresh,
                    subtitle,
                    enabled,
                } => Some((subtitle.clone(), *enabled)),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn default_menu_matches_the_mockup() {
        let rows = entries(&data("default"), now());
        assert_eq!(
            header_of(&rows),
            ("6 accounts · 2 need attention".to_owned(), Dot::Attention)
        );
        let pairs = accounts(&rows);
        let expected = [
            (
                "Personal · Max 5x",
                "5-hour 77% · Weekly 43% · resets 1h 12m",
            ),
            ("Work · Team", "Session expired — sign in again"),
            ("Side · Pro", "5-hour 8% · Weekly 21% · resets 4h 10m"),
            ("Personal · Plus", "5-hour 34% · Weekly 61% · resets 2h 05m"),
            ("Work · Team", "Rate limited · retry in 4m"),
            ("Free · Free", "30-day 100% · Limit reached · resets 9d"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(t, s)| ((*t).to_owned(), (*s).to_owned()))
            .collect();
        assert_eq!(pairs, expected);
        assert!(rows.contains(&Entry::Section("Claude")));
        assert!(rows.contains(&Entry::Section("Codex")));
        assert_eq!(refresh_of(&rows), (Some("Updated 2m ago".to_owned()), true));
    }

    #[test]
    fn refreshing_disables_refresh() {
        let rows = entries(&data("refreshing"), now());
        assert_eq!(header_of(&rows).0, "Refreshing 6 accounts…");
        assert_eq!(refresh_of(&rows), (Some("Refreshing…".to_owned()), false));
    }

    #[test]
    fn single_account_keeps_its_provider_header() {
        let rows = entries(&data("single"), now());
        assert_eq!(
            header_of(&rows),
            ("Tracking 1 account".to_owned(), Dot::Healthy)
        );
        assert!(rows.contains(&Entry::Section("Claude")));
        assert!(!rows.contains(&Entry::Section("Codex")));
    }

    #[test]
    fn offline_shows_cached_numbers() {
        let rows = entries(&data("offline"), now());
        assert_eq!(
            header_of(&rows),
            ("Offline · 6 accounts".to_owned(), Dot::Idle)
        );
        let pairs = accounts(&rows);
        assert_eq!(pairs[0].1, "5-hour 77% · Weekly 43% · resets 1h 12m");
        assert_eq!(pairs[1].1, "Session expired — sign in again");
        assert_eq!(
            refresh_of(&rows),
            (Some("Offline · showing 22m-old numbers".to_owned()), true)
        );
    }

    #[test]
    fn empty_state_offers_to_add_an_account() {
        let rows = entries(&data("empty"), now());
        assert_eq!(
            header_of(&rows),
            ("No accounts connected".to_owned(), Dot::Idle)
        );
        assert!(rows.contains(&Entry::Command {
            command: Command::AddAccount,
            subtitle: None,
            enabled: true,
        }));
        assert!(!rows.iter().any(|row| matches!(
            row,
            Entry::Command {
                command: Command::Refresh,
                ..
            }
        )));
    }

    #[test]
    fn busy_menu_groups_three_providers() {
        let rows = entries(&data("busy"), now());
        assert_eq!(header_of(&rows).0, "10 accounts · 2 need attention");
        assert!(rows.contains(&Entry::Section("Gemini")));
        assert_eq!(accounts(&rows).last().unwrap().1, "Not signed in");
    }
}
