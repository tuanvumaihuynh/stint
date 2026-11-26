//! Photographs the status menu with every mock scenario, in light and dark mode, for
//! design review: `stint-app snapshot [dir]` (debug builds only).
//!
//! The menu is opened for real and its window captured while it is open. An app may
//! capture its own windows without the Screen Recording permission.
#![allow(
    unsafe_code,
    reason = "the menu window is found and captured through CoreGraphics"
)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use anyhow::Context as _;
use block2::RcBlock;
use jiff::Timestamp;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    NSApplicationActivationPolicy, NSBitmapImageFileType, NSBitmapImageRep,
};
use objc2_core_foundation::{CFArray, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGWindowImageOption, CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{
    NSArray, NSDictionary, NSNumber, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, ns_string,
};

use crate::menu::MenuData;
use crate::mock;
use crate::status_menu::StatusMenu;

const DEFAULT_DIR: &str = "target/snapshots";
// Menus sit at the pop-up menu window level.
const MENU_WINDOW_LAYER: i64 = 101;
// Waits that let the status item appear and an opened menu finish drawing, in seconds.
const STARTUP_DELAY: f64 = 0.5;
const DRAW_DELAY: f64 = 0.4;

/// The output directory when the app was started as `stint-app snapshot [dir]`.
pub(crate) fn requested_dir() -> Option<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != "snapshot" {
        return None;
    }
    Some(
        args.next()
            .map_or_else(|| PathBuf::from(DEFAULT_DIR), PathBuf::from),
    )
}

pub(crate) fn run(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let mtm = MainThreadMarker::new().context("snapshots run on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let (commands, _ignored) = async_channel::unbounded();
    let menu = Rc::new(StatusMenu::new(commands)?);
    let dir = dir.to_owned();
    let start = RcBlock::new(move |_: NonNull<NSTimer>| {
        let code = match photograph_all(&menu, &dir, mtm) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("stint: snapshot: {err:#}");
                1
            }
        };
        std::process::exit(code);
    });
    // SAFETY: The block outlives the timer: the run loop retains it until it fires.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(STARTUP_DELAY, false, &start) };
    app.run();
    Ok(())
}

fn photograph_all(menu: &Rc<StatusMenu>, dir: &Path, mtm: MainThreadMarker) -> anyhow::Result<()> {
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: The appearance name constants are immutable AppKit globals.
    let appearances = unsafe {
        [
            (NSAppearanceNameAqua, "light"),
            (NSAppearanceNameDarkAqua, "dark"),
        ]
    };
    for (appearance, mode) in appearances {
        let appearance = NSAppearance::appearanceNamed(appearance);
        app.setAppearance(appearance.as_deref());
        menu.set_appearance(appearance.as_deref());
        for scenario in mock::SCENARIOS {
            menu.update(MenuData {
                accounts: mock::scenario(scenario, Timestamp::now()).unwrap_or_default(),
                refreshing: mock::refreshing(scenario),
            });
            let path = dir.join(format!("menu-{scenario}-{mode}.png"));
            photograph(menu, &path)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

/// Opens the menu, captures it once drawn, and closes it.
fn photograph(menu: &Rc<StatusMenu>, path: &Path) -> anyhow::Result<()> {
    let outcome: Rc<RefCell<Option<anyhow::Result<()>>>> = Rc::default();
    let (open_menu, target, result) = (menu.clone(), path.to_owned(), outcome.clone());
    let capture = RcBlock::new(move |_: NonNull<NSTimer>| {
        *result.borrow_mut() = Some(capture_menu(&target));
        open_menu.close();
    });
    // SAFETY: The block outlives the timer, which the run loop retains until it fires.
    // Common modes let the timer fire while the menu tracks the mouse.
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(DRAW_DELAY, false, &capture);
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
    menu.open();
    outcome
        .take()
        .context("the menu closed before it was captured")?
}

fn capture_menu(path: &Path) -> anyhow::Result<()> {
    let window = menu_window().context("no open menu window")?;
    let null_rect = CGRect::new(
        CGPoint::new(f64::INFINITY, f64::INFINITY),
        CGSize::new(0.0, 0.0),
    );
    #[expect(
        deprecated,
        reason = "ScreenCaptureKit needs the Screen Recording permission even for own windows"
    )]
    let image = objc2_core_graphics::CGWindowListCreateImage(
        null_rect,
        CGWindowListOption::OptionIncludingWindow,
        window,
        CGWindowImageOption::BoundsIgnoreFraming | CGWindowImageOption::BestResolution,
    )
    .context("capture the menu window")?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    // SAFETY: An empty dictionary is a valid set of PNG encoding properties.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .context("encode the capture as PNG")?;
    let written = png.writeToFile_atomically(&NSString::from_str(&path.to_string_lossy()), true);
    anyhow::ensure!(written, "write {}", path.display());
    Ok(())
}

/// The window number of this process's open menu.
fn menu_window() -> Option<u32> {
    let pid = i64::from(std::process::id());
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, 0)?;
    let list = window_infos(&list);
    list.iter().find_map(|info| {
        let number = |key: &NSString| -> Option<Retained<NSNumber>> {
            info.objectForKey(key)?.downcast::<NSNumber>().ok()
        };
        let owner = number(ns_string!("kCGWindowOwnerPID"))?.longLongValue();
        let layer = number(ns_string!("kCGWindowLayer"))?.longLongValue();
        (owner == pid && layer == MENU_WINDOW_LAYER)
            .then(|| number(ns_string!("kCGWindowNumber")).map(|n| n.unsignedIntValue()))
            .flatten()
    })
}

/// The window list as Foundation objects.
fn window_infos(list: &CFArray) -> &NSArray<NSDictionary<NSString, AnyObject>> {
    // SAFETY: `CFArray` is toll-free bridged to `NSArray`, and the window list holds
    // dictionaries keyed by strings.
    unsafe { &*std::ptr::from_ref(list).cast::<NSArray<NSDictionary<NSString, AnyObject>>>() }
}
