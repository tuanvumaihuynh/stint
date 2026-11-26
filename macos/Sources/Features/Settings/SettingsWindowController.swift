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
        // A menu bar app shows in the Dock and the app switcher only while its window is open.
        NSApp.setActivationPolicy(.regular)
        showWindow(nil)
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }

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
