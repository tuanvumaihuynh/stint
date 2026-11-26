import AppKit
import SwiftUI

/// Owns the desktop window. Stint is a menu bar app, so the window is created on first use
/// and kept, hidden, after it is closed.
@MainActor
final class SettingsWindowController: NSWindowController, NSWindowDelegate {
    static let size = NSSize(width: 720, height: 520)

    let model = SettingsModel()
    private let core: CoreClient

    init(core: CoreClient) {
        self.core = core
        let hosting = NSHostingController(rootView: SettingsView(core: core, model: model))
        hosting.sceneBridgingOptions = [.toolbars, .title]
        let window = NSWindow(contentViewController: hosting)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unified
        window.setContentSize(Self.size)
        window.setFrameAutosaveName("StintWindow")
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    /// Brings the window forward on `pane`.
    func show(_ pane: Pane) {
        model.show(pane)
        let opening = window?.isVisible == false
        // A menu bar app shows in the Dock and the app switcher only while its window is open.
        NSApp.setActivationPolicy(.regular)
        // Runs after the status menu has closed: activating while it is still tracking, or in
        // the same turn as the policy change, leaves the window behind other apps.
        DispatchQueue.main.async { [weak self] in
            guard let self, let window else { return }
            showWindow(nil)
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
            window.orderFrontRegardless()
            if opening {
                // SwiftUI focuses the search field once the window shows, which draws its
                // ring on every open; clear it after that.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                    window.makeFirstResponder(nil)
                }
            }
        }
    }

    #if DEBUG
        /// Shows the window on `pane` without activating Stint, so snapshots never take the
        /// keyboard from the app the user is typing in.
        func present(_ pane: Pane) {
            model.show(pane)
            window?.orderFrontRegardless()
        }
    #endif

    /// Opens the Accounts pane with the Add Account sheet, unless a sign-in is running.
    func addAccount() {
        show(.accounts)
        if model.signIn == nil {
            model.signIn = SignInFlow(core: core)
        }
    }

    func windowWillClose(_ notification: Notification) {
        // A sign-in has no window left to report to.
        model.signIn?.cancel()
        model.signIn = nil
        NSApp.setActivationPolicy(.accessory)
    }
}
