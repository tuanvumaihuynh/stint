import Foundation
import Observation

/// The app's side of the Rust core. It keeps the window's data current and forwards the
/// core's change notices to the main thread.
@MainActor
@Observable
final class CoreClient {
    /// What the window shows; replaced whenever the core reports a change.
    private(set) var overview: Overview

    /// Runs on the main thread whenever what the app shows has changed.
    @ObservationIgnored var onChange: (() -> Void)?

    /// The mock scenario shown instead of real accounts, if any.
    @ObservationIgnored let scenario: String?
    @ObservationIgnored private let stint: Stint

    init(mock: String?) throws {
        scenario = mock
        let relay = ChangeRelay()
        stint = try Stint(mock: mock, listener: relay)
        overview = stint.overview()
        relay.client = self
    }

    func menu() -> Menu {
        stint.menu()
    }

    func refresh() {
        stint.refresh()
    }

    var refreshChoices: [UInt32] {
        stint.refreshChoices()
    }

    func setRefreshMinutes(_ minutes: UInt32) throws {
        try stint.setRefreshMinutes(minutes: minutes)
    }

    func rename(_ id: Int64, to name: String) throws {
        try stint.renameAccount(id: id, name: name)
    }

    func setShownInMenu(_ id: Int64, _ shown: Bool) throws {
        try stint.setShownInMenu(id: id, shown: shown)
    }

    func remove(_ id: Int64) throws {
        try stint.removeAccount(id: id)
    }

    func addAccount(_ provider: Provider, name: String, listener: SignInListener) throws -> SignInSession {
        try stint.addAccount(provider: provider, name: name, listener: listener)
    }

    func signInAgain(_ id: Int64, listener: SignInListener) throws -> SignInSession {
        try stint.signInAgain(id: id, listener: listener)
    }

    /// Rereads the window's data, so ages such as "Updated 2m ago" stay current.
    func reloadOverview() {
        let fresh = stint.overview()
        if fresh != overview {
            overview = fresh
        }
    }

    fileprivate func changed() {
        overview = stint.overview()
        onChange?()
    }
}

extension StintError {
    /// The words to show the user.
    var message: String {
        switch self {
        case let .Message(message), let .Setup(message), let .UnknownScenario(message):
            message
        }
    }
}

/// The words to show the user for an error thrown by the core.
func userMessage(_ error: Error) -> String {
    (error as? StintError)?.message ?? error.localizedDescription
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
