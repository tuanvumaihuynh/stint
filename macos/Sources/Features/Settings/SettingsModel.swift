import Observation

/// A pane of the window, in sidebar order.
enum Pane: String, CaseIterable, Identifiable {
    case general
    case accounts
    case menuBar

    var id: Self { self }

    var title: String {
        switch self {
        case .general: "General"
        case .accounts: "Accounts"
        case .menuBar: "Menu Bar"
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape.fill"
        case .accounts: "person.2.fill"
        case .menuBar: "menubar.rectangle"
        }
    }

    var tile: TileColors {
        switch self {
        case .general: .general
        case .accounts: .accounts
        case .menuBar: .menuBar
        }
    }

    /// Words that find the pane from the sidebar search.
    var keywords: [String] {
        switch self {
        case .general: ["general", "refresh", "interval", "launch", "login", "startup", "version"]
        case .accounts: ["accounts", "add", "sign in", "rename", "remove", "claude", "codex"]
        case .menuBar: ["menu bar", "menu", "show", "hide", "visible"]
        }
    }

    func matches(_ query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespaces).lowercased()
        return query.isEmpty || title.lowercased().contains(query)
            || keywords.contains { $0.contains(query) }
    }
}

/// What the window shows: the pane, its back and forward history, and the open sheet.
@MainActor
@Observable
final class SettingsModel {
    private(set) var pane: Pane = .accounts
    private var back: [Pane] = []
    private var forward: [Pane] = []

    /// The sign-in the Add Account sheet is running, when it is open.
    var signIn: SignInFlow?
    /// The account whose removal is being confirmed.
    var removing: AccountRow?

    var canGoBack: Bool { !back.isEmpty }
    var canGoForward: Bool { !forward.isEmpty }

    func show(_ next: Pane) {
        guard next != pane else { return }
        back.append(pane)
        forward.removeAll()
        pane = next
    }

    func goBack() {
        guard let previous = back.popLast() else { return }
        forward.append(pane)
        pane = previous
    }

    func goForward() {
        guard let next = forward.popLast() else { return }
        back.append(pane)
        pane = next
    }
}
