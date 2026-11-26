import Foundation

/// The app's side of the Rust core: forwards its change notices to the main thread.
@MainActor
final class CoreClient {
    /// Runs on the main thread whenever what the app shows has changed.
    var onChange: (() -> Void)?

    private let stint: Stint

    init(mock: String?) throws {
        let relay = ChangeRelay()
        stint = try Stint(mock: mock, listener: relay)
        relay.client = self
    }

    func menu() -> Menu {
        stint.menu()
    }

    func refresh() {
        stint.refresh()
    }

    fileprivate func changed() {
        onChange?()
    }
}

/// Receives the core's notices on its background threads.
private final class ChangeRelay: Listener, @unchecked Sendable {
    // Set once, right after the core is created, before it can call back.
    weak var client: CoreClient?

    func changed() {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.client?.changed() }
        }
    }
}
