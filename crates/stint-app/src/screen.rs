use gpui_kit::{DisplayId, Pixels, Point, point};
use objc2_app_kit::{NSEvent, NSScreen};
use objc2_foundation::{MainThreadMarker, NSNumber, NSRect, NSString};

// Room kept between the popover and the screen edges or the menu bar.
const EDGE_MARGIN: f64 = 8.0;
const MENU_BAR_GAP: f64 = 4.0;
// Used when the menu bar auto-hides and the visible frame reaches the top.
const FALLBACK_MENU_BAR_HEIGHT: f64 = 24.0;

/// Where to open a popover of `width` points so it hangs under the clicked status item.
pub(crate) struct Anchor {
    pub(crate) display: Option<DisplayId>,
    /// Top-left corner, relative to `display`.
    pub(crate) origin: Point<Pixels>,
}

/// Places the popover under the mouse, which is on the status item right after a click.
pub(crate) fn anchor_under_mouse(width: f64) -> Option<Anchor> {
    let mtm = MainThreadMarker::new()?;
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .find(|screen| contains(screen.frame(), mouse.x, mouse.y))?;

    let frame = screen.frame();
    let visible = screen.visibleFrame();
    // AppKit's origin is the bottom-left, so the menu bar is the gap above the visible frame.
    let menu_bar = frame.origin.y + frame.size.height - (visible.origin.y + visible.size.height);
    let menu_bar = if menu_bar > 0.0 {
        menu_bar
    } else {
        FALLBACK_MENU_BAR_HEIGHT
    };

    let (x, y) = place(mouse.x - frame.origin.x, frame.size.width, menu_bar, width);
    Some(Anchor {
        display: display_id(&screen),
        origin: point(Pixels::from(x), Pixels::from(y)),
    })
}

/// Centers the popover on `mouse_x` and keeps it inside the screen, just under the menu bar.
fn place(mouse_x: f64, screen_width: f64, menu_bar: f64, width: f64) -> (f64, f64) {
    let max_x = (screen_width - width - EDGE_MARGIN).max(EDGE_MARGIN);
    let x = (mouse_x - width / 2.0).clamp(EDGE_MARGIN, max_x);
    (x, menu_bar + MENU_BAR_GAP)
}

fn contains(rect: NSRect, x: f64, y: f64) -> bool {
    x >= rect.origin.x
        && x <= rect.origin.x + rect.size.width
        && y >= rect.origin.y
        && y <= rect.origin.y + rect.size.height
}

// GPUI names a macOS display by its CGDirectDisplayID, which AppKit lists as
// `NSScreenNumber` in the screen's device description.
fn display_id(screen: &NSScreen) -> Option<DisplayId> {
    let description = screen.deviceDescription();
    let number = description.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    let number = number.downcast::<NSNumber>().ok()?;
    Some(DisplayId::new(u64::from(number.unsignedIntValue())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_under_the_mouse() {
        assert_eq!(place(1000.0, 1512.0, 33.0, 340.0), (830.0, 37.0));
    }

    #[test]
    fn stays_inside_the_right_edge() {
        assert_eq!(place(1500.0, 1512.0, 33.0, 340.0), (1164.0, 37.0));
    }

    #[test]
    fn stays_inside_the_left_edge() {
        assert_eq!(place(20.0, 1512.0, 24.0, 340.0), (8.0, 28.0));
    }
}
