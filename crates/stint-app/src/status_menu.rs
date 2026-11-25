//! The status bar item and its native `NSMenu`.
//!
//! AppKit is reached through `objc2`; defining an Objective-C class and passing it as a
//! menu target are `unsafe` there. Each block states why it holds.
#![allow(
    unsafe_code,
    reason = "the status menu is drawn with AppKit through objc2"
)]

use std::cell::RefCell;

use anyhow::Context as _;
use async_channel::Sender;
use jiff::Timestamp;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSData, NSDictionary, NSMutableAttributedString, NSObject,
    NSObjectProtocol, NSSize, NSString,
};

use crate::menu::{self, Command, Dot, Entry, MenuData};

const TRAY_ICON: &[u8] = include_bytes!("../assets/tray-template@2x.png");
const APP_ICON_LIGHT: &[u8] = include_bytes!("../assets/app-icon-light@2x.png");
const APP_ICON_DARK: &[u8] = include_bytes!("../assets/app-icon-dark@2x.png");

// Sizes in points.
const TRAY_ICON_SIZE: f64 = 18.0;
const APP_ICON_SIZE: f64 = 28.0;
const MENU_WIDTH: f64 = 298.0;
const TITLE_SIZE: f64 = 13.0;
const STATUS_SIZE: f64 = 11.5;
// The tray icon is dimmed while every account is offline.
const OFFLINE_ICON_ALPHA: f64 = 0.45;

struct Ivars {
    commands: Sender<Command>,
    data: RefCell<MenuData>,
    app_icon_light: Retained<NSImage>,
    app_icon_dark: Retained<NSImage>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. The class is main-thread only and
    // its ivars are set before `init` returns.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct MenuController;

    // SAFETY: NSObjectProtocol adds no requirements.
    unsafe impl NSObjectProtocol for MenuController {}

    // SAFETY: The method matches `-[NSMenuDelegate menuNeedsUpdate:]`.
    unsafe impl NSMenuDelegate for MenuController {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            // Rebuilt on every open, so relative times like "Updated 2m ago" are current.
            self.rebuild(menu);
        }
    }

    // SAFETY: Each method has the `- (void)action:(id)sender` signature menu items call.
    impl MenuController {
        #[unsafe(method(stintRefresh:))]
        fn refresh(&self, _sender: &NSMenuItem) {
            self.send(Command::Refresh);
        }

        #[unsafe(method(stintAddAccount:))]
        fn add_account(&self, _sender: &NSMenuItem) {
            self.send(Command::AddAccount);
        }

        #[unsafe(method(stintOpen:))]
        fn open(&self, _sender: &NSMenuItem) {
            self.send(Command::OpenStint);
        }

        #[unsafe(method(stintSettings:))]
        fn settings(&self, _sender: &NSMenuItem) {
            self.send(Command::Settings);
        }

        #[unsafe(method(stintQuit:))]
        fn quit(&self, _sender: &NSMenuItem) {
            self.send(Command::Quit);
        }
    }
);

impl MenuController {
    fn new(mtm: MainThreadMarker, commands: Sender<Command>) -> anyhow::Result<Retained<Self>> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            commands,
            data: RefCell::new(MenuData::default()),
            app_icon_light: image(APP_ICON_LIGHT, APP_ICON_SIZE)?,
            app_icon_dark: image(APP_ICON_DARK, APP_ICON_SIZE)?,
        });
        // SAFETY: NSObject's `init` takes no arguments and returns the initialized object.
        Ok(unsafe { msg_send![super(this), init] })
    }

    fn send(&self, command: Command) {
        // The receiver lives as long as the app; a closed channel means it is quitting.
        self.ivars().commands.try_send(command).ok();
    }

    fn rebuild(&self, menu: &NSMenu) {
        let mtm = self.mtm();
        let dark = is_dark(&menu.effectiveAppearance());
        menu.removeAllItems();
        for entry in menu::entries(&self.ivars().data.borrow(), Timestamp::now()) {
            menu.addItem(&self.item(&entry, dark, mtm));
        }
    }

    fn item(&self, entry: &Entry, dark: bool, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        match entry {
            Entry::Separator => NSMenuItem::separatorItem(mtm),
            Entry::Section(title) => {
                NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(title), mtm)
            }
            Entry::Header { status, dot } => {
                let item = passive(mtm, "Stint", None);
                item.setAttributedTitle(Some(&header_title(status, *dot, dark)));
                let icon = if dark {
                    &self.ivars().app_icon_dark
                } else {
                    &self.ivars().app_icon_light
                };
                item.setImage(Some(icon));
                item
            }
            Entry::Account {
                name,
                plan,
                subtitle,
            } => {
                let item = passive(mtm, &plain_title(name, plan.as_deref()), Some(subtitle));
                item.setAttributedTitle(Some(&account_title(name, plan.as_deref())));
                item
            }
            Entry::Notice { title, subtitle } => passive(mtm, title, Some(subtitle)),
            Entry::Command {
                command,
                subtitle,
                enabled,
            } => {
                let item = self.command(mtm, *command);
                if let Some(subtitle) = subtitle {
                    item.setSubtitle(Some(&NSString::from_str(subtitle)));
                }
                item.setEnabled(*enabled);
                item
            }
        }
    }

    fn command(&self, mtm: MainThreadMarker, command: Command) -> Retained<NSMenuItem> {
        let action: Sel = match command {
            Command::Refresh => sel!(stintRefresh:),
            Command::AddAccount => sel!(stintAddAccount:),
            Command::OpenStint => sel!(stintOpen:),
            Command::Settings => sel!(stintSettings:),
            Command::Quit => sel!(stintQuit:),
        };
        // SAFETY: `action` names a method of `MenuController` with the action signature.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(command.title()),
                Some(action),
                &NSString::from_str(command.key()),
            )
        };
        let target: &AnyObject = self.as_ref();
        // SAFETY: The controller outlives every item: it is owned by `StatusMenu`, which
        // owns the menu the items live in.
        unsafe { item.setTarget(Some(target)) };
        item
    }
}

/// An item with no action. It stays enabled so macOS draws it at full contrast; a click
/// only closes the menu.
fn passive(mtm: MainThreadMarker, title: &str, subtitle: Option<&str>) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    if let Some(subtitle) = subtitle {
        item.setSubtitle(Some(&NSString::from_str(subtitle)));
    }
    item.setEnabled(true);
    item
}

/// "Stint" over "● 6 accounts · 2 need attention".
fn header_title(status: &str, dot: Dot, dark: bool) -> Retained<NSAttributedString> {
    // SAFETY: The font weight constants are immutable AppKit globals.
    let (semibold, regular) = unsafe { (NSFontWeightSemibold, NSFontWeightRegular) };
    let dot_color = match dot {
        Dot::Healthy if dark => srgb(0x00, 0xB8, 0x7C),
        Dot::Healthy => srgb(0x00, 0xA8, 0x70),
        Dot::Attention => NSColor::systemOrangeColor(),
        Dot::Idle => NSColor::tertiaryLabelColor(),
    };
    let title = NSMutableAttributedString::new();
    for (text, font, color) in [
        (
            "Stint\n",
            NSFont::systemFontOfSize_weight(TITLE_SIZE, semibold),
            NSColor::labelColor(),
        ),
        (
            "● ",
            NSFont::systemFontOfSize_weight(STATUS_SIZE, regular),
            dot_color,
        ),
        (
            status,
            NSFont::systemFontOfSize_weight(STATUS_SIZE, regular),
            NSColor::secondaryLabelColor(),
        ),
    ] {
        title.appendAttributedString(&styled(text, &font, &color));
    }
    title.into_super()
}

/// "Personal · Max 5x" as plain text, for accessibility and type-to-select.
fn plain_title(name: &str, plan: Option<&str>) -> String {
    match plan {
        Some(plan) => format!("{name} · {plan}"),
        None => name.to_owned(),
    }
}

/// "Personal" in the label color, " · Max 5x" in the secondary one.
fn account_title(name: &str, plan: Option<&str>) -> Retained<NSAttributedString> {
    let font = NSFont::menuFontOfSize(TITLE_SIZE);
    let title = NSMutableAttributedString::new();
    title.appendAttributedString(&styled(name, &font, &NSColor::labelColor()));
    if let Some(plan) = plan {
        let plan = format!(" · {plan}");
        title.appendAttributedString(&styled(&plan, &font, &NSColor::secondaryLabelColor()));
    }
    title.into_super()
}

fn styled(text: &str, font: &NSFont, color: &NSColor) -> Retained<NSAttributedString> {
    // SAFETY: The attribute name constants are immutable AppKit globals.
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font.as_ref(), color.as_ref()];
    let attributes = NSDictionary::from_slices(&keys, &values);
    // SAFETY: Both values have the types their attribute names require.
    unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(text), &attributes) }
}

fn srgb(red: u8, green: u8, blue: u8) -> Retained<NSColor> {
    let channel = |value: u8| f64::from(value) / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(red), channel(green), channel(blue), 1.0)
}

fn is_dark(appearance: &NSAppearance) -> bool {
    // SAFETY: The appearance name constants are immutable AppKit globals.
    let (aqua, dark_aqua) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    appearance
        .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark_aqua]))
        .is_some_and(|name| &*name == dark_aqua)
}

/// A PNG at 2x, drawn at `points` square.
fn image(png: &[u8], points: f64) -> anyhow::Result<Retained<NSImage>> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png))
        .context("decode a bundled image")?;
    image.setSize(NSSize::new(points, points));
    Ok(image)
}

/// The status bar item and its menu. Dropping it removes the item from the menu bar.
pub(crate) struct StatusMenu {
    status_item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    controller: Retained<MenuController>,
}

impl StatusMenu {
    pub(crate) fn new(commands: Sender<Command>) -> anyhow::Result<Self> {
        let mtm = MainThreadMarker::new().context("the status menu runs on the main thread")?;
        let status_item =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let button = status_item
            .button(mtm)
            .context("the status item has no button")?;
        let icon = image(TRAY_ICON, TRAY_ICON_SIZE)?;
        // A template image is tinted by macOS for light and dark menu bars.
        icon.setTemplate(true);
        button.setImage(Some(&icon));

        let controller = MenuController::new(mtm, commands)?;
        let menu = NSMenu::new(mtm);
        // Items without an action stay enabled instead of being dimmed.
        menu.setAutoenablesItems(false);
        menu.setMinimumWidth(MENU_WIDTH);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        status_item.setMenu(Some(&menu));

        Ok(Self {
            status_item,
            menu,
            controller,
        })
    }

    /// Shows `data`, also in a menu that is open right now.
    pub(crate) fn update(&self, data: MenuData) {
        let offline = menu::is_offline(&data.accounts);
        if let Some(button) = self.status_item.button(self.controller.mtm()) {
            button.setAlphaValue(if offline { OFFLINE_ICON_ALPHA } else { 1.0 });
        }
        *self.controller.ivars().data.borrow_mut() = data;
        self.controller.rebuild(&self.menu);
    }

    /// Opens the menu as a click on the item would. Returns once the menu closes.
    #[cfg(debug_assertions)]
    pub(crate) fn open(&self) {
        if let Some(button) = self.status_item.button(self.controller.mtm()) {
            // SAFETY: A nil sender is what a programmatic click passes.
            unsafe { button.performClick(None) };
        }
    }

    /// Draws the menu in `appearance` instead of the system one.
    #[cfg(debug_assertions)]
    pub(crate) fn set_appearance(&self, appearance: Option<&NSAppearance>) {
        self.menu.setAppearance(appearance);
    }

    /// Closes the menu if it is open.
    #[cfg(debug_assertions)]
    pub(crate) fn close(&self) {
        self.menu.cancelTracking();
    }
}
