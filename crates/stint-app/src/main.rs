#[cfg(target_os = "macos")]
mod app;
#[cfg(all(target_os = "macos", debug_assertions))]
mod enroll;
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
fn main() -> std::process::ExitCode {
    #[cfg(debug_assertions)]
    {
        // Debug builds can photograph the menu for design review: `just app-snapshot`.
        if let Some(dir) = snapshot::requested_dir() {
            return report(snapshot::run(&dir));
        }
        // And add real accounts before the desktop window exists.
        if let Some(args) = enroll::requested() {
            return enroll::run(&args);
        }
    }
    report(app::run())
}

#[cfg(target_os = "macos")]
fn report(result: anyhow::Result<()>) -> std::process::ExitCode {
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("stint: {err:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Stint's menu bar app runs on macOS only.");
    std::process::exit(1);
}
