import AppKit

/// Starts Stint as a menu bar app: no Dock icon and no app menu.
@main
@MainActor
enum StintApp {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let delegate: NSApplicationDelegate
        // Unit tests run inside the app; they must not open the database or real accounts.
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil {
            delegate = IdleDelegate()
            app.delegate = delegate
            withExtendedLifetime(delegate) { app.run() }
            return
        }
        #if DEBUG
            // Debug builds can photograph the menu and the window for design review: `just app-snapshot`.
            if let directory = MenuSnapshotter.requestedDirectory() {
                delegate = MenuSnapshotter(directory: directory)
            } else if let directory = WindowSnapshotter.requestedDirectory() {
                delegate = WindowSnapshotter(directory: directory)
            } else {
                delegate = AppDelegate()
            }
        #else
            delegate = AppDelegate()
        #endif
        app.delegate = delegate
        // The app keeps only a weak reference to its delegate.
        withExtendedLifetime(delegate) { app.run() }
    }
}

/// Hosts unit tests without starting Stint.
private final class IdleDelegate: NSObject, NSApplicationDelegate {}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var core: CoreClient?
    private var statusMenu: StatusMenuController?
    private var window: SettingsWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let core: CoreClient
        do {
            core = try CoreClient(mock: Self.mockScenario())
        } catch {
            NSLog("stint: %@", String(describing: error))
            NSApp.terminate(nil)
            return
        }
        let statusMenu = StatusMenuController(menu: { core.menu() }, perform: { [weak self] in self?.perform($0) })
        core.onChange = { statusMenu.update() }
        statusMenu.update()
        self.core = core
        self.statusMenu = statusMenu
    }

    private func perform(_ command: Command) {
        switch command {
        case .refresh:
            core?.refresh()
        case .quit:
            NSApp.terminate(nil)
        case .addAccount:
            settingsWindow()?.addAccount()
        case .openStint:
            settingsWindow()?.show(.accounts)
        case .settings:
            settingsWindow()?.show(.general)
        }
    }

    private func settingsWindow() -> SettingsWindowController? {
        if window == nil, let core {
            window = SettingsWindowController(core: core)
        }
        return window
    }

    /// Made-up accounts in debug builds: `STINT_MOCK=default just mac-run`.
    private static func mockScenario() -> String? {
        #if DEBUG
            let scenario = ProcessInfo.processInfo.environment["STINT_MOCK"] ?? ""
            return scenario.isEmpty ? nil : scenario
        #else
            return nil
        #endif
    }
}
