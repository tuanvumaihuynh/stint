use jiff::Timestamp;
use std::collections::HashMap;
use stint_core::{AccountId, ProviderId, QuotaSnapshot, QuotaStore, QuotaWindow, SqliteStore};
use stint_providers::{Accounts, ProviderError};

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
    /// The provider needs the account set up in its own app before it shares quota.
    NeedsSetup,
}

/// One tracked account and its last known quota. A failed refresh keeps the old windows.
#[derive(Debug, Clone)]
pub(crate) struct AccountQuota {
    pub(crate) id: AccountId,
    pub(crate) provider: ProviderId,
    /// The nickname given when the account was added, such as "Personal".
    pub(crate) name: String,
    pub(crate) email: Option<String>,
    /// The subscription, such as "Max 5x", when known.
    pub(crate) plan: Option<String>,
    /// Whether the status menu lists the account.
    pub(crate) shown_in_menu: bool,
    pub(crate) windows: Vec<QuotaWindow>,
    pub(crate) fetched_at: Option<Timestamp>,
    pub(crate) error: Option<FetchError>,
}

/// Where the accounts come from.
pub(crate) enum Source {
    /// Accounts added in Stint: their stored snapshots, refreshed from the providers.
    Store(SqliteStore),
    /// Made-up accounts for development, picked by scenario name.
    Mock(String),
}

/// Every tracked account and whether a refresh is running.
pub(crate) struct QuotaState {
    pub(crate) accounts: Vec<AccountQuota>,
    pub(crate) refreshing: bool,
    source: Source,
    // Why each account's last refresh failed; cleared by its next success.
    errors: HashMap<AccountId, FetchError>,
}

/// What a refresh has to do after [`QuotaState::begin_refresh`].
pub(crate) enum Refresh {
    /// One is already running.
    Busy,
    /// There was nothing to fetch; the accounts were reloaded.
    Reloaded,
    /// Pretend to fetch, then call [`QuotaState::finish_mock`].
    Mock,
    /// Fetch these accounts, then call [`QuotaState::apply`].
    Fetch { jobs: Vec<(AccountId, String)> },
}

pub(crate) type FetchResult = Result<Vec<QuotaWindow>, ProviderError>;

impl QuotaState {
    pub(crate) fn new(source: Source) -> Self {
        let errors = HashMap::new();
        let accounts = load(&source, &errors, Timestamp::now());
        let refreshing = false;
        Self {
            accounts,
            refreshing,
            source,
            errors,
        }
    }

    /// The database, unless the accounts are made up.
    pub(crate) fn store_mut(&mut self) -> Option<&mut SqliteStore> {
        match &mut self.source {
            Source::Store(store) => Some(store),
            Source::Mock(_) => None,
        }
    }

    /// Forgets why an account last failed, once it has a new login.
    pub(crate) fn forget_error(&mut self, id: AccountId) {
        self.errors.remove(&id);
    }

    /// Rereads the accounts after the database changed. Made-up accounts keep their edits.
    pub(crate) fn reload(&mut self) {
        let Source::Store(store) = &self.source else {
            return;
        };
        let ids: Vec<AccountId> = store
            .accounts()
            .map_or_default(|accounts| accounts.into_iter().map(|a| a.id).collect());
        // Forget failures of removed accounts.
        self.errors.retain(|id, _| ids.contains(id));
        self.accounts = stored(store, &self.errors);
    }

    /// Marks a refresh as running and says what it has to fetch. A refresh already
    /// running is not restarted.
    pub(crate) fn begin_refresh(&mut self) -> Refresh {
        if self.refreshing {
            return Refresh::Busy;
        }
        let jobs = match &self.source {
            Source::Mock(_) => {
                self.refreshing = true;
                return Refresh::Mock;
            }
            Source::Store(store) => match store.accounts() {
                Ok(profiles) => profiles
                    .into_iter()
                    .map(|profile| (profile.id, profile.credential_key))
                    .collect::<Vec<_>>(),
                Err(err) => {
                    eprintln!("stint: read accounts: {err}");
                    return Refresh::Reloaded;
                }
            },
        };
        if jobs.is_empty() {
            self.accounts = load(&self.source, &self.errors, Timestamp::now());
            return Refresh::Reloaded;
        }
        self.refreshing = true;
        Refresh::Fetch { jobs }
    }

    /// Ends a mock refresh: every account without an error is fresh again.
    pub(crate) fn finish_mock(&mut self, now: Timestamp) {
        for account in &mut self.accounts {
            if account.error.is_none() {
                account.fetched_at = Some(now);
            }
        }
        self.refreshing = false;
    }

    /// Stores fresh numbers and remembers failures, then reloads the accounts.
    pub(crate) fn apply(&mut self, results: Vec<(AccountId, FetchResult)>, now: Timestamp) {
        self.refreshing = false;
        let Source::Store(store) = &mut self.source else {
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

/// Fetches each account in turn.
pub(crate) async fn fetch_all(
    accounts: &Accounts,
    jobs: Vec<(AccountId, String)>,
) -> Vec<(AccountId, FetchResult)> {
    let mut results = Vec::with_capacity(jobs.len());
    for (id, key) in jobs {
        results.push((id, accounts.fetch(&key).await));
    }
    results
}

/// How the menu reports a failed refresh.
fn fetch_error(err: &ProviderError, now: Timestamp) -> FetchError {
    match err {
        ProviderError::AuthMissing => FetchError::NotSignedIn,
        ProviderError::SignInExpired => FetchError::SessionExpired,
        ProviderError::RateLimited { retry_after } => FetchError::RateLimited {
            retry_at: retry_after.map(|wait| now + wait),
        },
        ProviderError::AccountSetup(_) => FetchError::NeedsSetup,
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
        Source::Store(store) => stored(store, errors),
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
                id: account.id,
                provider: account.provider,
                name: account.label,
                email: account.email,
                plan: account.plan,
                shown_in_menu: account.shown_in_menu,
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
