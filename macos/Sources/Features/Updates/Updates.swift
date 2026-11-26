import AppKit
import Observation
import Sparkle

/// Stint's updates, through Sparkle: checking on a schedule or on request, and what the
/// menu and the General pane show about them.
///
/// Stint lives in the menu bar, so a scheduled check that finds a new version does not
/// open Sparkle's window over whatever the user is doing. The menu offers the update
/// instead, and choosing it opens the window.
@MainActor
@Observable
final class Updates: NSObject {
    /// The version a scheduled check found, such as "0.2.0", until its update is handled.
    private(set) var available: String?
    /// When Sparkle last looked for an update.
    private(set) var lastChecked: Date?
    private(set) var automaticallyChecks = true
    private(set) var automaticallyDownloads = false

    @ObservationIgnored private var controller: SPUStandardUpdaterController?
    /// Runs on the main thread whenever `available` changes, for the status menu.
    @ObservationIgnored var onChange: (() -> Void)?

    /// Starts the updater, or returns `nil` in builds that do not update themselves: debug
    /// builds, and builds without the key that signs updates.
    static func start() -> Updates? {
        #if DEBUG
            return nil
        #else
            let key = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String ?? ""
            guard !key.isEmpty else { return nil }
            let updates = Updates()
            let controller = SPUStandardUpdaterController(
                startingUpdater: false, updaterDelegate: updates, userDriverDelegate: updates
            )
            updates.controller = controller
            do {
                try controller.updater.start()
            } catch {
                NSLog("stint: updates: %@", String(describing: error))
                return nil
            }
            updates.readSettings()
            return updates
        #endif
    }

    #if DEBUG
        /// Made-up update state, for snapshots.
        static func preview(available: String?, lastChecked: Date) -> Updates {
            let updates = Updates()
            updates.available = available
            updates.lastChecked = lastChecked
            return updates
        }

        /// Pretends a scheduled check found `version`, for snapshots.
        func preview(available version: String?) {
            setAvailable(version)
        }
    #endif

    /// Looks for an update now, or shows the one already found, in Sparkle's window.
    func checkForUpdates() {
        guard let controller else { return }
        // A menu bar app has to come forward for Sparkle's window to show in front.
        NSApp.activate(ignoringOtherApps: true)
        controller.checkForUpdates(nil)
    }

    func setAutomaticallyChecks(_ enabled: Bool) {
        controller?.updater.automaticallyChecksForUpdates = enabled
        readSettings()
    }

    func setAutomaticallyDownloads(_ enabled: Bool) {
        controller?.updater.automaticallyDownloadsUpdates = enabled
        readSettings()
    }

    private func readSettings() {
        guard let updater = controller?.updater else { return }
        automaticallyChecks = updater.automaticallyChecksForUpdates
        automaticallyDownloads = updater.automaticallyDownloadsUpdates
        lastChecked = updater.lastUpdateCheckDate
    }

    private func setAvailable(_ version: String?) {
        guard available != version else { return }
        available = version
        onChange?()
    }
}

extension Updates: SPUUpdaterDelegate {
    nonisolated func updater(
        _ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: (any Error)?
    ) {
        MainActor.assumeIsolated { readSettings() }
    }
}

extension Updates: SPUStandardUserDriverDelegate {
    nonisolated var supportsGentleScheduledUpdateReminders: Bool { true }

    /// Sparkle shows a scheduled update itself only when Stint is already in front.
    nonisolated func standardUserDriverShouldHandleShowingScheduledUpdate(
        _ update: SUAppcastItem, andInImmediateFocus immediateFocus: Bool
    ) -> Bool {
        immediateFocus
    }

    nonisolated func standardUserDriverWillHandleShowingUpdate(
        _ handleShowingUpdate: Bool, forUpdate update: SUAppcastItem, state: SPUUserUpdateState
    ) {
        let version = update.displayVersionString
        MainActor.assumeIsolated {
            // Shown by Sparkle or not, the menu keeps offering it until the session ends.
            setAvailable(state.userInitiated ? nil : version)
        }
    }

    nonisolated func standardUserDriverWillFinishUpdateSession() {
        MainActor.assumeIsolated { setAvailable(nil) }
    }
}
