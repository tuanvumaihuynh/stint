use anyhow::Context as _;
use gpui_kit::{ActivationPolicy, App, AppContext, Entity, Global, KeyBinding, QuitMode, actions};
use gpui_tray::Tray;
use stint_core::SqliteStore;
use stint_providers::{ClaudeProvider, CodexProvider};

use crate::icon;
use crate::popover;
use crate::quota::QuotaState;

actions!(stint, [TogglePopover, Refresh, Quit]);

// Two threads are plenty for a few HTTP requests every few minutes.
const IO_THREADS: usize = 2;

struct AppState {
    quota: Entity<QuotaState>,
    tray: Tray,
    // Owns the runtime the providers' requests run on; dropped when the app quits.
    _runtime: tokio::runtime::Runtime,
}

impl Global for AppState {}

pub(crate) fn run() -> anyhow::Result<()> {
    let db_path = stint_core::database_path().context("find the home directory")?;
    let store = SqliteStore::open(&db_path)
        .with_context(|| format!("open database {}", db_path.display()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(IO_THREADS)
        .thread_name("stint-io")
        .enable_all()
        .build()
        .context("start the network runtime")?;
    let http = stint_providers::http_client().context("build the HTTP client")?;
    let claude = ClaudeProvider::new(http.clone());
    let codex = CodexProvider::new(http);

    gpui_kit::application()
        // The bundled icon set: refresh, spinner, and notice icons.
        .with_assets(gpui_kit::assets::Assets)
        // The popover is the only window, so closing it must not quit the app.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            let started = start(cx, store, runtime, claude, codex);
            if let Err(err) = started {
                eprintln!("stint: {err:#}");
                cx.quit();
            }
        });
    Ok(())
}

fn start(
    cx: &mut App,
    store: SqliteStore,
    runtime: tokio::runtime::Runtime,
    claude: ClaudeProvider,
    codex: CodexProvider,
) -> anyhow::Result<()> {
    gpui_kit::init(cx);
    // A menu bar utility: no Dock icon and no app menu, but it still opens windows.
    cx.set_activation_policy(ActivationPolicy::Accessory);

    let handle = runtime.handle().clone();
    let quota = cx.new(|_| QuotaState::new(store, handle, claude, codex));

    cx.bind_keys([
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.on_action(|_: &TogglePopover, cx| {
        let quota = cx.global::<AppState>().quota.clone();
        popover::toggle(quota, cx);
    });
    cx.on_action(|_: &Refresh, cx| {
        let quota = cx.global::<AppState>().quota.clone();
        quota.update(cx, QuotaState::refresh);
    });
    cx.on_action(|_: &Quit, cx| {
        let tray = cx.global::<AppState>().tray.clone();
        if let Err(err) = tray.close(cx) {
            eprintln!("stint: close tray: {err}");
        }
        cx.quit();
    });

    let tray = Tray::builder()
        .icon(icon::template().context("draw the status bar icon")?)
        .tooltip("Stint")
        .on_activate(TogglePopover)
        .build(cx)
        .context("add the status bar item")?;

    quota.update(cx, QuotaState::start_polling);
    cx.set_global(AppState {
        quota,
        tray,
        _runtime: runtime,
    });
    Ok(())
}
