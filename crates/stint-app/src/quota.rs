use std::time::Duration;

use gpui_kit::{Context, Task};
use jiff::Timestamp;
use stint_core::{ProviderId, QuotaStore, QuotaWindow, SqliteStore};

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
    /// Accounts added in Stint, with their last stored snapshots.
    Store(SqliteStore),
    /// Made-up accounts for development, picked by scenario name.
    Mock(String),
}

/// Every tracked account, shared by the status menu and the windows.
pub(crate) struct QuotaState {
    pub(crate) accounts: Vec<AccountQuota>,
    pub(crate) refreshing: bool,
    source: Source,
    refresh_task: Option<Task<()>>,
}

// How long a mock refresh pretends to take, so the refreshing state can be seen.
const MOCK_REFRESH: Duration = Duration::from_millis(1200);

impl QuotaState {
    pub(crate) fn new(source: Source) -> Self {
        let accounts = load(&source, Timestamp::now());
        Self {
            accounts,
            refreshing: false,
            source,
            refresh_task: None,
        }
    }

    /// Reloads every account. A refresh already running is not restarted.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        match &self.source {
            // Fetching from providers per account comes with account sign-in; until then
            // a refresh rereads what is stored.
            Source::Store(_) => {
                self.accounts = load(&self.source, Timestamp::now());
                cx.notify();
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
}

fn load(source: &Source, now: Timestamp) -> Vec<AccountQuota> {
    match source {
        Source::Store(store) => stored(store),
        Source::Mock(scenario) => mock::scenario(scenario, now).unwrap_or_default(),
    }
}

/// The stored accounts with their last snapshots.
fn stored(store: &SqliteStore) -> Vec<AccountQuota> {
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
                plan: None,
                windows,
                fetched_at,
                error: None,
            }
        })
        .collect()
}
