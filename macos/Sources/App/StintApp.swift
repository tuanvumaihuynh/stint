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
    private var updates: Updates?
    private var termination: DispatchSourceSignal?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let launchCheck = CommandLine.arguments.contains("--launch-check")
        let core: CoreClient
        do {
            core = try CoreClient(mock: Self.mockScenario())
        } catch {
            NSLog("stint: %@", String(describing: error))
            if launchCheck { exit(1) }
            NSApp.terminate(nil)
            return
        }
        if launchCheck {
            Self.reportLaunch(core.menu())
            exit(0)
        }
        quitOnTerminationSignal()
        let updates = Updates.start()
        let statusMenu = StatusMenuController(
            menu: { core.menu() },
            perform: { [weak self] in self?.perform($0) },
            pin: { id in
                do {
                    try core.setPinnedAccount(id)
                } catch {
                    NSLog("stint: pin account: %@", userMessage(error))
                }
            },
            updates: updates
        )
        core.onChange = { statusMenu.update() }
        updates?.onChange = { statusMenu.update() }
        self.updates = updates
        statusMenu.update()
        self.core = core
        self.statusMenu = statusMenu
    }

    /// Quits as Quit Stint does when sent SIGTERM, as `kill` does, instead of dying on the
    /// spot, so an update Sparkle downloaded still installs on the way out.
    private func quitOnTerminationSignal() {
        signal(SIGTERM, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler { NSApp.terminate(nil) }
        source.resume()
        termination = source
    }

    /// `--launch-check`, for release smoke tests: the database opened and migrated, so print
    /// the menu's status line and quit without showing anything.
    private static func reportLaunch(_ menu: Menu) {
        for case .header(let status, _) in menu.entries {
            print("launch check: \(status)")
        }
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
            window = SettingsWindowController(core: core, updates: updates)
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
