use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui_kit::{Context, Task};
use jiff::Timestamp;
use stint_core::{ProviderId, QuotaStore, QuotaWindow, SqliteStore, StoreError};
use stint_providers::{ClaudeProvider, CodexProvider, ProviderError, UsageProvider};

// How often quota is polled in the background, on top of manual refreshes.
const POLL_INTERVAL: Duration = Duration::from_mins(5);

/// What went wrong on the last fetch, in the terms the popover shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FetchError {
    NotSignedIn,
    TokenExpired,
    KeychainLocked,
    RateLimited,
    Network,
    Other(String),
}

impl From<ProviderError> for FetchError {
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::AuthMissing => Self::NotSignedIn,
            ProviderError::TokenExpired => Self::TokenExpired,
            ProviderError::KeychainLocked => Self::KeychainLocked,
            ProviderError::RateLimited { .. } => Self::RateLimited,
            ProviderError::Transport(_) => Self::Network,
            other => Self::Other(other.to_string()),
        }
    }
}

/// The last known quota of one provider. A failed fetch keeps the previous windows.
#[derive(Debug, Clone)]
pub(crate) struct ProviderQuota {
    pub(crate) provider: ProviderId,
    pub(crate) windows: Vec<QuotaWindow>,
    pub(crate) fetched_at: Option<Timestamp>,
    pub(crate) error: Option<FetchError>,
}

impl ProviderQuota {
    fn empty(provider: ProviderId) -> Self {
        Self {
            provider,
            windows: Vec::new(),
            fetched_at: None,
            error: None,
        }
    }
}

type FetchResult = (
    ProviderId,
    Result<(Vec<QuotaWindow>, Timestamp), FetchError>,
);

/// Quota for every provider, shared by the popover and the background poller.
pub(crate) struct QuotaState {
    pub(crate) providers: Vec<ProviderQuota>,
    pub(crate) refreshing: bool,
    store: Arc<Mutex<SqliteStore>>,
    runtime: tokio::runtime::Handle,
    claude: Arc<ClaudeProvider>,
    codex: Arc<CodexProvider>,
    refresh_task: Option<Task<()>>,
    poll_task: Option<Task<()>>,
}

impl QuotaState {
    pub(crate) fn new(
        store: SqliteStore,
        runtime: tokio::runtime::Handle,
        claude: ClaudeProvider,
        codex: CodexProvider,
    ) -> Self {
        let providers = [ProviderId::Claude, ProviderId::Codex]
            .into_iter()
            .map(|provider| cached(&store, provider))
            .collect();
        Self {
            providers,
            refreshing: false,
            store: Arc::new(Mutex::new(store)),
            runtime,
            claude: Arc::new(claude),
            codex: Arc::new(codex),
            refresh_task: None,
            poll_task: None,
        }
    }

    /// Refreshes now and then every [`POLL_INTERVAL`] until the state is dropped.
    pub(crate) fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, QuotaState::refresh).is_err() {
                    return;
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
            }
        }));
    }

    /// Fetches every provider at once. A refresh already running is not restarted.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.notify();

        // Requests run on the Tokio runtime, which reqwest needs; GPUI only awaits the result.
        let claude = self
            .runtime
            .spawn(fetch_and_record(self.claude.clone(), self.store.clone()));
        let codex = self
            .runtime
            .spawn(fetch_and_record(self.codex.clone(), self.store.clone()));

        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            let mut results = Vec::with_capacity(2);
            for job in [claude, codex] {
                match job.await {
                    Ok(result) => results.push(result),
                    Err(err) => eprintln!("stint: refresh task failed: {err}"),
                }
            }
            this.update(cx, |state, cx| {
                state.apply(results);
                state.refreshing = false;
                cx.notify();
            })
            .ok();
        }));
    }

    fn apply(&mut self, results: Vec<FetchResult>) {
        for (provider, result) in results {
            let Some(quota) = self.providers.iter_mut().find(|q| q.provider == provider) else {
                continue;
            };
            match result {
                Ok((windows, fetched_at)) => {
                    quota.windows = windows;
                    quota.fetched_at = Some(fetched_at);
                    quota.error = None;
                }
                Err(err) => quota.error = Some(err),
            }
        }
    }
}

async fn fetch_and_record<P>(provider: Arc<P>, store: Arc<Mutex<SqliteStore>>) -> FetchResult
where
    P: UsageProvider + Send + Sync + 'static,
{
    let id = provider.id();
    let windows = match provider.fetch().await {
        Ok(windows) => windows,
        Err(err) => return (id, Err(err.into())),
    };

    let fetched_at = Timestamp::now();
    // Saving is best effort: the popover still shows a fetch that could not be stored.
    if let Err(err) = record(&store, id, &windows, fetched_at) {
        eprintln!("stint: save {id} quota: {err}");
    }
    (id, Ok((windows, fetched_at)))
}

fn record(
    store: &Mutex<SqliteStore>,
    provider: ProviderId,
    windows: &[QuotaWindow],
    fetched_at: Timestamp,
) -> Result<(), StoreError> {
    let mut store = store.lock().unwrap_or_else(PoisonError::into_inner);
    let account = store.ensure_active_account(provider, fetched_at)?;
    store.record_snapshot(&stint_core::QuotaSnapshot {
        account_id: account.id,
        fetched_at,
        windows: windows.to_vec(),
    })
}

/// The provider's last stored snapshot, so the popover has numbers before the first fetch.
fn cached(store: &SqliteStore, provider: ProviderId) -> ProviderQuota {
    let snapshot = store.accounts().ok().and_then(|accounts| {
        let account = accounts
            .into_iter()
            .find(|account| account.provider == provider && account.is_active)?;
        store.latest_snapshot(account.id).ok().flatten()
    });
    match snapshot {
        Some(snapshot) => ProviderQuota {
            provider,
            windows: snapshot.windows,
            fetched_at: Some(snapshot.fetched_at),
            error: None,
        },
        None => ProviderQuota::empty(provider),
    }
}
