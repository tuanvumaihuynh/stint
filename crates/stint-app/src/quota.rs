use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{Context, Task};
use jiff::Timestamp;
use stint_core::{AccountId, ProviderId, QuotaSnapshot, QuotaStore, QuotaWindow, SqliteStore};
use stint_providers::{Accounts, ProviderError, Vault};

use crate::mock;

/// Why an account's numbers could not be refreshed, in the terms the menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FetchError {
    NotSignedIn,
    SessionExpired,
    /// The provider asked to wait; `retry_at` is when that wait ends, if it said.
    RateLimited {
        retry_at: Option<Timestamp>,
    },
    Offline,
}

/// One tracked account and its last known quota. A failed refresh keeps the old windows.
#[derive(Debug, Clone)]
pub(crate) struct AccountQuota {
    pub(crate) provider: ProviderId,
    /// The nickname given when the account was added, such as "Personal".
    pub(crate) name: String,
    /// The subscription, such as "Max 5x", when known.
    pub(crate) plan: Option<String>,
    pub(crate) windows: Vec<QuotaWindow>,
    pub(crate) fetched_at: Option<Timestamp>,
    pub(crate) error: Option<FetchError>,
}

/// Where the accounts come from.
pub(crate) enum Source {
    /// Accounts added in Stint: their stored snapshots, refreshed from the providers.
    Store {
        store: SqliteStore,
        /// Holds the renewal locks.
        data_dir: PathBuf,
    },
    /// Made-up accounts for development, picked by scenario name.
    Mock(String),
}

/// Every tracked account, shared by the status menu and the windows.
pub(crate) struct QuotaState {
    pub(crate) accounts: Vec<AccountQuota>,
    pub(crate) refreshing: bool,
    source: Source,
    // Why each account's last refresh failed; cleared by its next success.
    errors: HashMap<AccountId, FetchError>,
    refresh_task: Option<Task<()>>,
}

// How long a mock refresh pretends to take, so the refreshing state can be seen.
const MOCK_REFRESH: Duration = Duration::from_millis(1200);

impl QuotaState {
    pub(crate) fn new(source: Source) -> Self {
        let errors = HashMap::new();
        let accounts = load(&source, &errors, Timestamp::now());
        Self {
            accounts,
            refreshing: false,
            source,
            errors,
            refresh_task: None,
        }
    }

    pub(crate) fn is_mock(&self) -> bool {
        matches!(self.source, Source::Mock(_))
    }

    /// Fetches every account's quota. A refresh already running is not restarted.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        match &self.source {
            Source::Store { store, data_dir } => {
                let jobs: Vec<(AccountId, String)> = match store.accounts() {
                    Ok(profiles) => profiles
                        .into_iter()
                        .map(|profile| (profile.id, profile.credential_key))
                        .collect(),
                    Err(err) => {
                        eprintln!("stint: read accounts: {err}");
                        Vec::new()
                    }
                };
                if jobs.is_empty() {
                    self.accounts = load(&self.source, &self.errors, Timestamp::now());
                    cx.notify();
                    return;
                }
                self.refreshing = true;
                cx.notify();
                let data_dir = data_dir.clone();
                let fetched = cx
                    .background_executor()
                    .spawn(async move { fetch_all(&data_dir, jobs) });
                self.refresh_task = Some(cx.spawn(async move |this, cx| {
                    let results = fetched.await;
                    this.update(cx, |state, cx| {
                        state.apply(results, Timestamp::now());
                        state.refreshing = false;
                        cx.notify();
                    })
                    .ok();
                }));
            }
            Source::Mock(_) => {
                self.refreshing = true;
                cx.notify();
                self.refresh_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(MOCK_REFRESH).await;
                    this.update(cx, |state, cx| {
                        let now = Timestamp::now();
                        for account in &mut state.accounts {
                            if account.error.is_none() {
                                account.fetched_at = Some(now);
                            }
                        }
                        state.refreshing = false;
                        cx.notify();
                    })
                    .ok();
                }));
            }
        }
    }

    /// Stores fresh numbers and remembers failures, then reloads the accounts.
    fn apply(&mut self, results: Vec<(AccountId, FetchResult)>, now: Timestamp) {
        let Source::Store { store, .. } = &mut self.source else {
            return;
        };
        for (id, result) in results {
            match result {
                Ok(windows) => {
                    self.errors.remove(&id);
                    let snapshot = QuotaSnapshot {
                        account_id: id,
                        fetched_at: now,
                        windows,
                    };
                    if let Err(err) = store.record_snapshot(&snapshot) {
                        eprintln!("stint: save account {id} usage: {err}");
                    }
                }
                Err(err) => {
                    eprintln!("stint: refresh account {id}: {err}");
                    self.errors.insert(id, fetch_error(&err, now));
                }
            }
        }
        self.accounts = load(&self.source, &self.errors, now);
    }
}

type FetchResult = Result<Vec<QuotaWindow>, ProviderError>;

/// Fetches each account in turn. Blocks: run it on a background thread.
fn fetch_all(data_dir: &Path, jobs: Vec<(AccountId, String)>) -> Vec<(AccountId, FetchResult)> {
    // The HTTP client runs on Tokio; the app's executors are not Tokio's.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let (runtime, client) = match (runtime, stint_providers::http_client()) {
        (Ok(runtime), Ok(client)) => (runtime, client),
        (Err(err), _) => return failed(jobs, &err.to_string()),
        (_, Err(err)) => return failed(jobs, &err.to_string()),
    };
    let accounts = Accounts::new(client, Vault::new(), data_dir);
    runtime.block_on(async {
        let mut results = Vec::with_capacity(jobs.len());
        for (id, key) in jobs {
            results.push((id, accounts.fetch(&key).await));
        }
        results
    })
}

fn failed(jobs: Vec<(AccountId, String)>, reason: &str) -> Vec<(AccountId, FetchResult)> {
    eprintln!("stint: start fetching: {reason}");
    jobs.into_iter()
        .map(|(id, _)| (id, Err(ProviderError::Unsupported(reason.to_owned()))))
        .collect()
}

/// How the menu reports a failed refresh.
fn fetch_error(err: &ProviderError, now: Timestamp) -> FetchError {
    match err {
        ProviderError::AuthMissing => FetchError::NotSignedIn,
        ProviderError::SignInExpired => FetchError::SessionExpired,
        ProviderError::RateLimited { retry_after } => FetchError::RateLimited {
            retry_at: retry_after.map(|wait| now + wait),
        },
        // Anything else is retried on the next refresh, like an unreachable provider.
        _ => FetchError::Offline,
    }
}

fn load(
    source: &Source,
    errors: &HashMap<AccountId, FetchError>,
    now: Timestamp,
) -> Vec<AccountQuota> {
    match source {
        Source::Store { store, .. } => stored(store, errors),
        Source::Mock(scenario) => mock::scenario(scenario, now).unwrap_or_default(),
    }
}

/// The stored accounts with their last snapshots and refresh failures.
fn stored(store: &SqliteStore, errors: &HashMap<AccountId, FetchError>) -> Vec<AccountQuota> {
    let accounts = match store.accounts() {
        Ok(accounts) => accounts,
        Err(err) => {
            eprintln!("stint: read accounts: {err}");
            return Vec::new();
        }
    };
    accounts
        .into_iter()
        .map(|account| {
            let (windows, fetched_at) = match store.latest_snapshot(account.id) {
                Ok(Some(snapshot)) => (snapshot.windows, Some(snapshot.fetched_at)),
                _ => (Vec::new(), None),
            };
            AccountQuota {
                provider: account.provider,
                name: account.label,
                plan: account.plan,
                windows,
                fetched_at,
                error: errors.get(&account.id).cloned(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::*;

    #[test]
    fn provider_errors_map_to_menu_states() {
        let now: Timestamp = "2026-11-26T10:00:00Z".parse().unwrap();
        assert_eq!(
            fetch_error(&ProviderError::AuthMissing, now),
            FetchError::NotSignedIn
        );
        assert_eq!(
            fetch_error(&ProviderError::SignInExpired, now),
            FetchError::SessionExpired
        );
        assert_eq!(
            fetch_error(
                &ProviderError::RateLimited {
                    retry_after: Some(SignedDuration::from_mins(2))
                },
                now
            ),
            FetchError::RateLimited {
                retry_at: Some(now + SignedDuration::from_mins(2))
            }
        );
        assert_eq!(
            fetch_error(&ProviderError::Status { status: 502 }, now),
            FetchError::Offline
        );
    }
}
