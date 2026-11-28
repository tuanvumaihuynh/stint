//! The Rust core as the macOS app sees it, exported to Swift with `UniFFI`.
//!
//! The app holds one [`Stint`]. It owns the database, the Tokio runtime the providers run
//! on, and the refresh schedule, and calls the app's [`Listener`] whenever what the app
//! shows has changed. The app then reads [`Stint::menu`] or [`Stint::overview`] and
//! redraws.

mod format;
mod menu;
mod mock;
mod pin;
mod quota;
mod signin;
mod window;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use stint_core::{AccountId, QuotaStore, SqliteStore, StoreError};
use stint_providers::{Accounts, Vault};

use menu::MenuData;
pub use menu::{Command, Dot, Entry};
pub use pin::MenuBarStyle;
use quota::{QuotaState, Refresh, Source};
use signin::Target;
pub use signin::{SignInListener, SignInSession};
pub use window::{AccountRow, Issue, Overview, Provider, Severity};

uniffi::setup_scaffolding!();

// How often the refresh schedule is checked against the chosen interval.
const POLL_TICK: Duration = Duration::from_secs(30);
const REFRESH_SETTING: &str = "refresh_minutes";
// How long a mock refresh pretends to take, so the refreshing state can be seen.
const MOCK_REFRESH: Duration = Duration::from_millis(1200);

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum StintError {
    #[error("unknown mock scenario {0:?}")]
    UnknownScenario(String),
    #[error("{0}")]
    Setup(String),
    /// A message to show the user as is.
    #[error("{0}")]
    Message(String),
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
    /// What the menu bar shows after the icon, such as "48%"; `None` for the icon alone.
    pub status_text: Option<String>,
}

/// The menu bar choices as stored; the pin may name a removed account.
#[derive(Debug, Clone, Copy)]
struct MenuBar {
    pinned: Option<AccountId>,
    style: MenuBarStyle,
}

/// The app's handle on the core.
#[derive(uniffi::Object)]
pub struct Stint {
    state: Mutex<QuotaState>,
    runtime: tokio::runtime::Runtime,
    accounts: Option<Accounts>,
    // Where sign-ins run; `None` for made-up accounts.
    data_dir: Option<PathBuf>,
    listener: Arc<dyn Listener>,
    refresh_minutes: AtomicU32,
    menu_bar: Mutex<MenuBar>,
    last_refresh: Mutex<Instant>,
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
        let (source, accounts, data_dir) = if let Some(scenario) = mock {
            if !mock::SCENARIOS.contains(&scenario.as_str()) {
                return Err(StintError::UnknownScenario(scenario));
            }
            (Source::Mock(scenario), None, None)
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
            (Source::Store(store), Some(accounts), Some(data_dir))
        };
        let refresh_minutes = match &source {
            Source::Store(store) => store
                .setting(REFRESH_SETTING)
                .ok()
                .flatten()
                .and_then(|value| value.parse().ok())
                .filter(|minutes| window::REFRESH_CHOICES.contains(minutes)),
            Source::Mock(_) => None,
        }
        .unwrap_or(window::DEFAULT_REFRESH_MINUTES);
        let stored = |key: &str| match &source {
            Source::Store(store) => store.setting(key).ok().flatten(),
            Source::Mock(_) => None,
        };
        let menu_bar = MenuBar {
            pinned: stored(pin::PIN_SETTING).and_then(|value| pin::parse_pin(&value)),
            style: stored(pin::STYLE_SETTING)
                .map_or_else(MenuBarStyle::default, |value| MenuBarStyle::parse(&value)),
        };
        let starts_refreshing = match &source {
            Source::Store(_) => true,
            Source::Mock(name) => mock::refreshing(name),
        };
        let stint = Arc::new(Self {
            state: Mutex::new(QuotaState::new(source)),
            runtime,
            accounts,
            data_dir,
            listener,
            refresh_minutes: AtomicU32::new(refresh_minutes),
            menu_bar: Mutex::new(menu_bar),
            last_refresh: Mutex::new(Instant::now()),
        });
        if starts_refreshing {
            stint.clone().refresh();
        }
        if stint.accounts.is_some() {
            let weak = Arc::downgrade(&stint);
            stint.runtime.spawn(async move {
                loop {
                    tokio::time::sleep(POLL_TICK).await;
                    let Some(stint) = weak.upgrade() else { return };
                    if stint.refresh_due() {
                        stint.refresh();
                    }
                }
            });
        }
        Ok(stint)
    }

    /// The status menu as of now. Accounts hidden from the menu are left out.
    pub fn menu(&self) -> Menu {
        let state = self.state();
        let shown: Vec<_> = state
            .accounts
            .iter()
            .filter(|account| account.shown_in_menu)
            .cloned()
            .collect();
        let menu_bar = *self.menu_bar();
        let pinned = pin::effective(&state.accounts, menu_bar.pinned);
        let account = state.accounts.iter().find(|a| Some(a.id) == pinned);
        let offline = menu::is_offline(&state.accounts);
        let mut menu = menu_of(&shown, state.refreshing, pinned, Timestamp::now());
        menu.status_text = pin::status_text(account, menu_bar.style, offline);
        menu
    }

    /// The window as of now.
    pub fn overview(&self) -> Overview {
        let state = self.state();
        let menu_bar = *self.menu_bar();
        let mut overview = window::overview(
            &state.accounts,
            state.refreshing,
            self.refresh_minutes.load(Ordering::SeqCst),
            Timestamp::now(),
        );
        overview.pinned_account =
            pin::effective(&state.accounts, menu_bar.pinned).map(AccountId::get);
        overview.menu_bar_style = menu_bar.style;
        overview
    }

    /// Shows an account's usage in the menu bar, and remembers it.
    pub fn set_pinned_account(&self, id: i64) -> Result<(), StintError> {
        let id = AccountId::new(id);
        let mut state = self.state();
        if !state.accounts.iter().any(|a| a.id == id) {
            return Err(StintError::Message("That account was removed.".to_owned()));
        }
        if let Some(store) = state.store_mut() {
            store
                .set_setting(pin::PIN_SETTING, &id.get().to_string())
                .map_err(|err| signin::store_failure(&err))?;
        }
        drop(state);
        self.menu_bar().pinned = Some(id);
        self.listener.changed();
        Ok(())
    }

    /// Sets what the menu bar shows next to the icon, and remembers it.
    pub fn set_menu_bar_style(&self, style: MenuBarStyle) -> Result<(), StintError> {
        if let Some(store) = self.state().store_mut() {
            store
                .set_setting(pin::STYLE_SETTING, style.as_str())
                .map_err(|err| signin::store_failure(&err))?;
        }
        self.menu_bar().style = style;
        self.listener.changed();
        Ok(())
    }

    /// The refresh intervals offered, in minutes.
    pub fn refresh_choices(&self) -> Vec<u32> {
        window::REFRESH_CHOICES.to_vec()
    }

    /// Sets how often accounts refresh on their own, and remembers it.
    pub fn set_refresh_minutes(&self, minutes: u32) -> Result<(), StintError> {
        if !window::REFRESH_CHOICES.contains(&minutes) {
            return Err(StintError::Message(format!(
                "{minutes} minutes is not a refresh interval."
            )));
        }
        if let Some(store) = self.state().store_mut() {
            store
                .set_setting(REFRESH_SETTING, &minutes.to_string())
                .map_err(|err| signin::store_failure(&err))?;
        }
        self.refresh_minutes.store(minutes, Ordering::SeqCst);
        self.listener.changed();
        Ok(())
    }

    /// Gives an account a new nickname.
    pub fn rename_account(&self, id: i64, name: &str) -> Result<(), StintError> {
        let name = nickname(name)?;
        self.edit(
            id,
            |store, id| store.rename_account(id, &name),
            |account| account.name.clone_from(&name),
        )
    }

    /// Lists an account in the status menu, or hides it there.
    pub fn set_shown_in_menu(&self, id: i64, shown: bool) -> Result<(), StintError> {
        self.edit(
            id,
            |store, id| store.set_shown_in_menu(id, shown),
            |account| account.shown_in_menu = shown,
        )
    }

    /// Stops tracking an account and deletes its saved login.
    pub fn remove_account(&self, id: i64) -> Result<(), StintError> {
        let id = AccountId::new(id);
        let mut state = self.state();
        let mut menu_bar = self.menu_bar();
        if menu_bar.pinned == Some(id) {
            // Cleared rather than moved, so the first account is pinned until one is chosen.
            menu_bar.pinned = None;
            if let Some(store) = state.store_mut()
                && let Err(err) = store.set_setting(pin::PIN_SETTING, "")
            {
                eprintln!("stint: forget the pinned account: {err}");
            }
        }
        drop(menu_bar);
        let Some(store) = state.store_mut() else {
            state.accounts.retain(|account| account.id != id);
            drop(state);
            self.listener.changed();
            return Ok(());
        };
        let key = store
            .accounts()
            .map_err(|err| signin::store_failure(&err))?
            .into_iter()
            .find(|account| account.id == id)
            .map(|account| account.credential_key);
        store
            .remove_account(id)
            .map_err(|err| signin::store_failure(&err))?;
        state.reload();
        drop(state);
        if let Some(key) = key
            && let Err(err) = Vault::new().delete(&key)
        {
            eprintln!("stint: delete the login of account {id}: {err}");
        }
        self.listener.changed();
        Ok(())
    }

    /// Adds an account by signing in to `provider` in the browser, under nickname `name`.
    pub fn add_account(
        self: Arc<Self>,
        provider: Provider,
        name: &str,
        listener: Arc<dyn SignInListener>,
    ) -> Result<Arc<SignInSession>, StintError> {
        let name = nickname(name)?;
        let target = Target::New {
            provider: provider.into(),
            name,
        };
        self.start_sign_in(target, listener)
    }

    /// Signs an account in again, keeping its nickname and history.
    pub fn sign_in_again(
        self: Arc<Self>,
        id: i64,
        listener: Arc<dyn SignInListener>,
    ) -> Result<Arc<SignInSession>, StintError> {
        let target = Target::Existing {
            id: AccountId::new(id),
        };
        self.start_sign_in(target, listener)
    }

    /// Fetches every account's quota in the background. A refresh already running is not
    /// restarted.
    pub fn refresh(self: Arc<Self>) {
        let refresh = self.state().begin_refresh();
        if !matches!(refresh, Refresh::Busy) {
            *self.last_refresh() = Instant::now();
        }
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

    fn menu_bar(&self) -> MutexGuard<'_, MenuBar> {
        self.menu_bar.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn last_refresh(&self) -> MutexGuard<'_, Instant> {
        self.last_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn refresh_due(&self) -> bool {
        let minutes = self.refresh_minutes.load(Ordering::SeqCst);
        self.last_refresh().elapsed() >= Duration::from_secs(u64::from(minutes) * 60)
    }

    /// Changes one account in the database, or in place for made-up accounts.
    fn edit(
        &self,
        id: i64,
        stored: impl FnOnce(&mut SqliteStore, AccountId) -> Result<(), StoreError>,
        made_up: impl FnOnce(&mut quota::AccountQuota),
    ) -> Result<(), StintError> {
        let id = AccountId::new(id);
        let mut state = self.state();
        if let Some(store) = state.store_mut() {
            stored(store, id).map_err(|err| match err {
                StoreError::DuplicateAccount { provider, label } => {
                    signin::duplicate(provider, &label)
                }
                err => signin::store_failure(&err),
            })?;
            state.reload();
        } else if let Some(account) = state.accounts.iter_mut().find(|a| a.id == id) {
            made_up(account);
        }
        drop(state);
        self.listener.changed();
        Ok(())
    }
}

/// A nickname as entered, trimmed; it cannot be blank.
fn nickname(name: &str) -> Result<String, StintError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StintError::Message("Enter a nickname.".to_owned()));
    }
    Ok(name.to_owned())
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
    let pinned = pin::effective(&accounts, None);
    let account = accounts.iter().find(|a| Some(a.id) == pinned);
    let offline = menu::is_offline(&accounts);
    let mut menu = menu_of(&accounts, mock::refreshing(scenario), pinned, now);
    menu.status_text = pin::status_text(account, MenuBarStyle::default(), offline);
    Some(menu)
}

fn menu_of(
    accounts: &[quota::AccountQuota],
    refreshing: bool,
    pinned: Option<AccountId>,
    now: Timestamp,
) -> Menu {
    let data = MenuData {
        accounts: accounts.to_vec(),
        refreshing,
        pinned,
    };
    Menu {
        entries: menu::entries(&data, now),
        offline: menu::is_offline(accounts),
        status_text: None,
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
    fn made_up_accounts_can_be_edited() {
        let counter = Arc::new(Counter::default());
        let stint = Stint::new(Some("default".into()), counter.clone()).unwrap();
        let first = stint.overview().accounts[0].clone();

        stint.rename_account(first.id, "  Main ").unwrap();
        assert_eq!(stint.overview().accounts[0].name, "Main");
        assert!(matches!(
            stint.rename_account(first.id, " "),
            Err(StintError::Message(_))
        ));

        let accounts_in_menu = |stint: &Stint| {
            stint
                .menu()
                .entries
                .iter()
                .filter(|entry| matches!(entry, Entry::Account { .. }))
                .count()
        };
        let before = accounts_in_menu(&stint);
        stint.set_shown_in_menu(first.id, false).unwrap();
        assert_eq!(accounts_in_menu(&stint), before - 1);

        let count = stint.overview().accounts.len();
        stint.remove_account(first.id).unwrap();
        assert_eq!(stint.overview().accounts.len(), count - 1);
        assert!(counter.0.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn refresh_interval_is_one_of_the_choices() {
        let counter = Arc::new(Counter::default());
        let stint = Stint::new(Some("default".into()), counter).unwrap();
        assert_eq!(stint.overview().refresh_minutes, 5);
        stint.set_refresh_minutes(10).unwrap();
        assert_eq!(stint.overview().refresh_minutes, 10);
        assert!(stint.set_refresh_minutes(7).is_err());
        assert!(!stint.refresh_due());
    }

    #[test]
    fn pin_and_style_follow_the_choices() {
        let counter = Arc::new(Counter::default());
        let stint = Stint::new(Some("default".into()), counter).unwrap();
        let accounts = stint.overview().accounts;
        assert_eq!(stint.overview().pinned_account, Some(accounts[0].id));
        assert_eq!(stint.menu().status_text.as_deref(), Some("77%"));

        stint.set_pinned_account(accounts[3].id).unwrap();
        assert_eq!(stint.overview().pinned_account, Some(accounts[3].id));
        assert_eq!(stint.menu().status_text.as_deref(), Some("34%"));
        assert!(stint.set_pinned_account(99).is_err());

        stint.set_menu_bar_style(MenuBarStyle::Icon).unwrap();
        assert_eq!(stint.overview().menu_bar_style, MenuBarStyle::Icon);
        assert_eq!(stint.menu().status_text, None);
        stint.set_menu_bar_style(MenuBarStyle::IconPercent).unwrap();

        stint.remove_account(accounts[3].id).unwrap();
        assert_eq!(stint.overview().pinned_account, Some(accounts[0].id));
        assert_eq!(stint.menu().status_text.as_deref(), Some("77%"));
    }

    #[test]
    fn made_up_accounts_cannot_sign_in() {
        struct Quiet;
        impl SignInListener for Quiet {
            fn link(&self, _: String) {}
            fn code_prompt(&self) {}
            fn succeeded(&self, _: Option<String>, _: Option<String>) {}
            fn failed(&self, _: String, _: bool) {}
        }
        let counter = Arc::new(Counter::default());
        let stint = Stint::new(Some("default".into()), counter).unwrap();
        assert!(matches!(
            stint.add_account(Provider::Claude, "New", Arc::new(Quiet)),
            Err(StintError::Message(_))
        ));
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
