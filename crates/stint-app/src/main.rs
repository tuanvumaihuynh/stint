#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod format;
#[cfg(target_os = "macos")]
mod icon;
#[cfg(target_os = "macos")]
mod popover;
#[cfg(target_os = "macos")]
mod quota;
#[cfg(target_os = "macos")]
mod screen;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    app::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Stint's menu bar app runs on macOS only.");
    std::process::exit(1);
}
