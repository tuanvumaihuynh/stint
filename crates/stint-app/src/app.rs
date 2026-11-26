use anyhow::Context as _;
use gpui_kit::{ActivationPolicy, App, AppContext, Entity, Global, QuitMode};
use stint_core::SqliteStore;

use crate::menu::{Command, MenuData};
use crate::mock;
use crate::quota::{QuotaState, Source};
use crate::status_menu::StatusMenu;

// Picks mock accounts in debug builds: `STINT_MOCK=default just app`.
const MOCK_ENV: &str = "STINT_MOCK";

struct AppState {
    quota: Entity<QuotaState>,
    menu: StatusMenu,
}

impl Global for AppState {}

pub(crate) fn run() -> anyhow::Result<()> {
    let source = source()?;
    gpui_kit::application()
        // Windows come and go; only Quit ends the app.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            if let Err(err) = start(cx, source) {
                eprintln!("stint: {err:#}");
                cx.quit();
            }
        });
    Ok(())
}

fn source() -> anyhow::Result<Source> {
    if cfg!(debug_assertions)
        && let Ok(scenario) = std::env::var(MOCK_ENV)
    {
        anyhow::ensure!(
            mock::SCENARIOS.contains(&scenario.as_str()),
            "unknown {MOCK_ENV} scenario {scenario:?}; try one of {:?}",
            mock::SCENARIOS
        );
        return Ok(Source::Mock(scenario));
    }
    let path = stint_core::database_path().context("find the home directory")?;
    let store =
        SqliteStore::open(&path).with_context(|| format!("open database {}", path.display()))?;
    Ok(Source::Store(store))
}

fn start(cx: &mut App, source: Source) -> anyhow::Result<()> {
    gpui_kit::init(cx);
    // A menu bar utility: no Dock icon and no app menu.
    cx.set_activation_policy(ActivationPolicy::Accessory);

    let refreshing = matches!(&source, Source::Mock(name) if mock::refreshing(name));
    let quota = cx.new(|_| QuotaState::new(source));
    if refreshing {
        quota.update(cx, QuotaState::refresh);
    }

    let (commands, received) = async_channel::unbounded();
    let menu = StatusMenu::new(commands).context("add the status bar item")?;
    menu.update(menu_data(&quota, cx));

    cx.observe(&quota, |quota, cx| {
        let data = menu_data(&quota, cx);
        cx.global::<AppState>().menu.update(data);
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(command) = received.recv().await {
            cx.update(|cx| handle(command, cx));
        }
    })
    .detach();

    cx.set_global(AppState { quota, menu });
    Ok(())
}

fn menu_data(quota: &Entity<QuotaState>, cx: &App) -> MenuData {
    let state = quota.read(cx);
    MenuData {
        accounts: state.accounts.clone(),
        refreshing: state.refreshing,
    }
}

fn handle(command: Command, cx: &mut App) {
    match command {
        Command::Refresh => {
            let quota = cx.global::<AppState>().quota.clone();
            quota.update(cx, QuotaState::refresh);
        }
        Command::Quit => cx.quit(),
        // The desktop window comes next; until then these do nothing.
        Command::AddAccount | Command::OpenStint | Command::Settings => {
            eprintln!("stint: {} is not available yet", command.title());
        }
    }
}
