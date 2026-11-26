//! The Rust core as the macOS app sees it, exported to Swift with `UniFFI`.
//!
//! The app holds one [`Stint`]. It owns the database, the Tokio runtime the providers run
//! on, and the refresh schedule, and calls the app's [`Listener`] whenever what the app
//! shows has changed. The app then reads [`Stint::menu`] and redraws.

mod format;
mod menu;
mod mock;
mod quota;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use stint_core::SqliteStore;
use stint_providers::{Accounts, Vault};

use menu::MenuData;
pub use menu::{Command, Dot, Entry};
use quota::{QuotaState, Refresh, Source};

uniffi::setup_scaffolding!();

// How often real accounts refresh on their own.
const POLL_INTERVAL: Duration = Duration::from_mins(5);
// How long a mock refresh pretends to take, so the refreshing state can be seen.
const MOCK_REFRESH: Duration = Duration::from_millis(1200);

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum StintError {
    #[error("unknown mock scenario {0:?}")]
    UnknownScenario(String),
    #[error("{0}")]
    Setup(String),
}

/// Told, from a background thread, that what the app shows has changed.
#[uniffi::export(with_foreign)]
pub trait Listener: Send + Sync {
    fn changed(&self);
}

/// What the status menu draws.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Menu {
    pub entries: Vec<Entry>,
    /// Every account is offline, so the menu bar icon is dimmed.
    pub offline: bool,
}

/// The app's handle on the core.
#[derive(uniffi::Object)]
pub struct Stint {
    state: Mutex<QuotaState>,
    runtime: tokio::runtime::Runtime,
    accounts: Option<Accounts>,
    listener: Arc<dyn Listener>,
}

#[uniffi::export]
impl Stint {
    /// Opens the database and starts refreshing. With `mock`, shows the made-up accounts of
    /// that scenario instead and never touches real accounts.
    #[uniffi::constructor]
    pub fn new(mock: Option<String>, listener: Arc<dyn Listener>) -> Result<Arc<Self>, StintError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| StintError::Setup(format!("start the runtime: {err}")))?;
        let (source, accounts) = if let Some(scenario) = mock {
            if !mock::SCENARIOS.contains(&scenario.as_str()) {
                return Err(StintError::UnknownScenario(scenario));
            }
            (Source::Mock(scenario), None)
        } else {
            let setup = |what: &str, err: &dyn std::fmt::Display| {
                StintError::Setup(format!("{what}: {err}"))
            };
            let no_home = || StintError::Setup("find the home directory".to_owned());
            let data_dir = stint_core::data_dir().ok_or_else(no_home)?;
            let path = stint_core::database_path().ok_or_else(no_home)?;
            let store = SqliteStore::open(&path).map_err(|err| setup("open the database", &err))?;
            let client = stint_providers::http_client()
                .map_err(|err| setup("start the HTTP client", &err))?;
            let accounts = Accounts::new(client, Vault::new(), &data_dir);
            (Source::Store(store), Some(accounts))
        };
        let starts_refreshing = match &source {
            Source::Store(_) => true,
            Source::Mock(name) => mock::refreshing(name),
        };
        let stint = Arc::new(Self {
            state: Mutex::new(QuotaState::new(source)),
            runtime,
            accounts,
            listener,
        });
        if starts_refreshing {
            stint.clone().refresh();
        }
        if stint.accounts.is_some() {
            let weak = Arc::downgrade(&stint);
            stint.runtime.spawn(async move {
                loop {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    let Some(stint) = weak.upgrade() else { return };
                    stint.refresh();
                }
            });
        }
        Ok(stint)
    }

    /// The status menu as of now.
    pub fn menu(&self) -> Menu {
        let state = self.state();
        menu_of(&state.accounts, state.refreshing, Timestamp::now())
    }

    /// Fetches every account's quota in the background. A refresh already running is not
    /// restarted.
    pub fn refresh(self: Arc<Self>) {
        let refresh = self.state().begin_refresh();
        match refresh {
            Refresh::Busy => {}
            Refresh::Reloaded => self.listener.changed(),
            Refresh::Mock => {
                self.listener.changed();
                let stint = self.clone();
                self.runtime.spawn(async move {
                    tokio::time::sleep(MOCK_REFRESH).await;
                    stint.state().finish_mock(Timestamp::now());
                    stint.listener.changed();
                });
            }
            Refresh::Fetch { jobs } => {
                self.listener.changed();
                let stint = self.clone();
                self.runtime.spawn(async move {
                    let Some(accounts) = &stint.accounts else {
                        return;
                    };
                    let results = quota::fetch_all(accounts, jobs).await;
                    stint.state().apply(results, Timestamp::now());
                    stint.listener.changed();
                });
            }
        }
    }
}

impl Stint {
    fn state(&self) -> MutexGuard<'_, QuotaState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The menu title of a command, such as "Settings…".
#[uniffi::export]
pub fn command_title(command: Command) -> String {
    command.title().to_owned()
}

/// The key that, with ⌘, runs a command while the menu is open, or "".
#[uniffi::export]
pub fn command_key(command: Command) -> String {
    command.key().to_owned()
}

/// Every mock scenario, in the order snapshots are taken.
#[uniffi::export]
pub fn mock_scenarios() -> Vec<String> {
    mock::SCENARIOS
        .iter()
        .map(|&name| name.to_owned())
        .collect()
}

/// The menu of a mock scenario as of now, for snapshots; `None` for an unknown name.
#[uniffi::export]
pub fn mock_menu(scenario: &str) -> Option<Menu> {
    let now = Timestamp::now();
    let accounts = mock::scenario(scenario, now)?;
    Some(menu_of(&accounts, mock::refreshing(scenario), now))
}

fn menu_of(accounts: &[quota::AccountQuota], refreshing: bool, now: Timestamp) -> Menu {
    let data = MenuData {
        accounts: accounts.to_vec(),
        refreshing,
    };
    Menu {
        entries: menu::entries(&data, now),
        offline: menu::is_offline(accounts),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl Listener for Counter {
        fn changed(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn mock_refresh_runs_once_and_reports_changes() {
        let counter = Arc::new(Counter::default());
        let stint = Stint::new(Some("default".into()), counter.clone()).unwrap();
        assert!(stint.menu().entries.len() > 1);

        stint.clone().refresh();
        stint.clone().refresh();
        assert!(stint.state().refreshing);
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);

        std::thread::sleep(MOCK_REFRESH + Duration::from_millis(500));
        assert!(!stint.state().refreshing);
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unknown_scenarios_are_rejected() {
        let counter = Arc::new(Counter::default());
        assert!(matches!(
            Stint::new(Some("nope".into()), counter),
            Err(StintError::UnknownScenario(name)) if name == "nope"
        ));
        assert!(mock_menu("nope").is_none());
    }

    #[test]
    fn every_scenario_has_a_menu() {
        for scenario in mock_scenarios() {
            assert!(mock_menu(&scenario).is_some());
        }
        assert_eq!(command_title(Command::Settings), "Settings…");
        assert_eq!(command_key(Command::Quit), "q");
    }
}
