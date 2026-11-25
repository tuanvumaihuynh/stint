#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod format;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "macos")]
mod mock;
#[cfg(target_os = "macos")]
mod quota;
#[cfg(all(target_os = "macos", debug_assertions))]
mod snapshot;
#[cfg(target_os = "macos")]
mod status_menu;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    // Debug builds can photograph the menu for design review: `just app-snapshot`.
    #[cfg(debug_assertions)]
    if let Some(dir) = snapshot::requested_dir() {
        return snapshot::run(&dir);
    }
    app::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Stint's menu bar app runs on macOS only.");
    std::process::exit(1);
}
