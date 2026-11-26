import AppKit
import Observation

/// One run of the Add Account sheet, or of signing an account in again: the step it is on
/// and what the sign-in has reported.
@MainActor
@Observable
final class SignInFlow: Identifiable {
    enum Step: Equatable {
        case provider
        case nickname
        case waiting
        case connected(email: String?, plan: String?)
        case failed(String)

        /// "Step 2 of 4".
        var number: Int {
            switch self {
            case .provider: 1
            case .nickname: 2
            case .waiting: 3
            case .connected, .failed: 4
            }
        }
    }

    /// The account signed in again, or `nil` when adding one.
    let account: AccountRow?
    private(set) var step: Step
    var provider: Provider = .claude
    var nickname = ""
    /// Why the nickname cannot be used, shown under the field.
    private(set) var nicknameError: String?
    /// The sign-in page, once the CLI has printed it.
    private(set) var link: URL?
    var code = ""
    private(set) var codeError: String?
    private(set) var codeSent = false

    @ObservationIgnored private let core: CoreClient?
    @ObservationIgnored private var session: SignInSession?
    @ObservationIgnored private var relay: SignInRelay?

    /// Adds a new account.
    init(core: CoreClient?) {
        self.core = core
        account = nil
        step = .provider
    }

    /// Signs `account` in again, starting right away.
    init(core: CoreClient?, signingInAgain account: AccountRow) {
        self.core = core
        self.account = account
        provider = account.provider
        nickname = account.name
        step = .waiting
    }

    #if DEBUG
        /// A flow parked on `step`, for snapshots.
        init(previewing step: Step, provider: Provider, nickname: String) {
            core = nil
            account = nil
            self.step = step
            self.provider = provider
            self.nickname = nickname
        }
    #endif

    var title: String {
        switch step {
        case .provider: "Add Account"
        case .nickname: "Name This \(provider.title) Account"
        case .waiting: "Sign In to \(provider.title)"
        case .connected: account == nil ? "Account Connected" : "Signed In Again"
        case .failed: "Sign-In Failed"
        }
    }

    func chooseProvider() {
        guard provider.available else { return }
        step = .nickname
    }

    func goBack() {
        nicknameError = nil
        step = .provider
    }

    /// Starts the CLI sign-in in the browser.
    func signIn() {
        guard let core else { return }
        nicknameError = nil
        codeError = nil
        codeSent = false
        code = ""
        link = nil
        let relay = SignInRelay()
        relay.flow = self
        do {
            if let account {
                session = try core.signInAgain(account.id, listener: relay)
            } else {
                session = try core.addAccount(provider, name: nickname, listener: relay)
            }
        } catch {
            if account == nil {
                nicknameError = userMessage(error)
                step = .nickname
            } else {
                step = .failed(userMessage(error))
            }
            return
        }
        self.relay = relay
        step = .waiting
    }

    func openLinkAgain() {
        if let link {
            NSWorkspace.shared.open(link)
        }
    }

    /// Sends the code pasted from the sign-in page; Claude Code only.
    func submitCode() {
        let code = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !code.isEmpty, let session else { return }
        do {
            try session.submitCode(code: code)
            codeError = nil
            codeSent = true
        } catch {
            codeError = userMessage(error)
        }
    }

    /// Stops a running sign-in; nothing it reports afterwards is shown.
    func cancel() {
        relay?.flow = nil
        relay = nil
        session?.cancel()
        session = nil
    }

    func tryAgain() {
        if account == nil {
            step = .nickname
        } else {
            signIn()
        }
    }

    fileprivate func found(link: String) {
        self.link = URL(string: link)
    }

    fileprivate func finished(_ outcome: Step) {
        relay = nil
        session = nil
        step = outcome
    }
}

/// Receives sign-in progress on the core's background threads.
private final class SignInRelay: SignInListener, @unchecked Sendable {
    // Cleared on the main thread when the flow stops listening.
    weak var flow: SignInFlow?

    func link(url: String) {
        onMain { $0.found(link: url) }
    }

    func codePrompt() {}

    func succeeded(email: String?, plan: String?) {
        onMain { $0.finished(.connected(email: email, plan: plan)) }
    }

    func failed(message: String, cancelled: Bool) {
        onMain { $0.finished(.failed(message)) }
    }

    private func onMain(_ body: @escaping @MainActor (SignInFlow) -> Void) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let flow = self?.flow, flow.isListening(to: self) else { return }
                body(flow)
            }
        }
    }
}

extension SignInFlow {
    fileprivate func isListening(to relay: SignInRelay?) -> Bool {
        relay != nil && self.relay === relay
    }
}
