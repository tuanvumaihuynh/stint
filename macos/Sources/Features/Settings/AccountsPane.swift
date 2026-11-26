import SwiftUI

/// Every account, grouped by provider, with what each one last reported.
struct AccountsPane: View {
    let core: CoreClient
    @Bindable var model: SettingsModel
    @State private var renaming: AccountRow?
    @State private var error: String?

    var body: some View {
        let overview = core.overview
        Group {
            if overview.accounts.isEmpty {
                EmptyAccounts(add: addAccount)
            } else {
                Form {
                    ForEach(Provider.all, id: \.self) { provider in
                        let rows = overview.accounts.filter { $0.provider == provider }
                        if !rows.isEmpty {
                            Section(provider.title) {
                                ForEach(rows, id: \.id) { account in
                                    AccountRowView(
                                        account: account,
                                        signInAgain: { signInAgain(account) },
                                        rename: { renaming = account },
                                        remove: { model.removing = account }
                                    )
                                }
                            }
                        }
                    }
                    Section {} footer: {
                        Text(overview.footer)
                            .font(.subheadline)
                            .foregroundStyle(.tertiary)
                    }
                }
                .formStyle(.grouped)
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Refresh", systemImage: "arrow.clockwise", action: core.refresh)
                    .disabled(overview.accounts.isEmpty || overview.refreshing)
                    .help("Refresh every account now")
                Button("Add Account", systemImage: "plus", action: addAccount)
                    .help("Add an account")
            }
        }
        .sheet(item: $renaming) { account in
            RenameSheet(account: account) { name in
                try core.rename(account.id, to: name)
            }
        }
        .alert(removalTitle, isPresented: shown($model.removing)) {
            Button("Cancel", role: .cancel) {}
            Button("Remove", role: .destructive) {
                if let account = model.removing {
                    run { try core.remove(account.id) }
                }
            }
        } message: {
            Text(removalMessage)
        }
        .alert("Something Went Wrong", isPresented: shown($error)) {
            Button("OK") {}
        } message: {
            Text(error ?? "")
        }
    }

    private func addAccount() {
        model.signIn = SignInFlow(core: core)
    }

    private func signInAgain(_ account: AccountRow) {
        let flow = SignInFlow(core: core, signingInAgain: account)
        model.signIn = flow
        flow.signIn()
    }

    private var removalTitle: String {
        model.removing.map { "Remove “\($0.name)”?" } ?? ""
    }

    private var removalMessage: String {
        guard let account = model.removing else { return "" }
        let plan = account.plan.map { " \($0)" } ?? ""
        let email = account.email.map { " (\($0))" } ?? ""
        return "This removes your \(account.provider.title)\(plan) account\(email) from Stint and deletes "
            + "its saved session from your macOS Keychain. You can sign in again anytime."
    }

    private func run(_ action: () throws -> Void) {
        do { try action() } catch { self.error = userMessage(error) }
    }

    private func shown<Value>(_ value: Binding<Value?>) -> Binding<Bool> {
        Binding { value.wrappedValue != nil } set: { if !$0 { value.wrappedValue = nil } }
    }
}

/// One account: nickname, plan, and email over its readings or its problem.
struct AccountRowView: View {
    let account: AccountRow
    let signInAgain: () -> Void
    let rename: () -> Void
    let remove: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 7) {
                    Text(account.name).font(.body.weight(.semibold))
                    if let plan = account.plan {
                        Badge(text: plan)
                    }
                    if let email = account.email {
                        Text(email)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .frame(minHeight: 22)
                detail
            }
            // Readings with a line per model group get the room before "Updated" does.
            .frame(maxWidth: .infinity, alignment: .leading)
            .layoutPriority(1)
            VStack(alignment: .trailing, spacing: 5) {
                HStack(spacing: 8) {
                    status
                    actions
                }
                .frame(minHeight: 22)
                if let updated = account.updated {
                    Text(updated)
                        .font(.subheadline)
                        .foregroundStyle(.tertiary)
                        .fixedSize()
                }
            }
        }
        .padding(.vertical, 3)
    }

    @ViewBuilder
    private var detail: some View {
        if let issue = account.issue {
            HStack(spacing: 5) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(issue.severity.color)
                (Text(issue.title).fontWeight(.medium).foregroundStyle(issue.severity.color)
                    + Text(" — \(issue.detail)").foregroundStyle(.secondary))
                    .font(.callout)
            }
        } else {
            // One line per model group; a group's line never breaks.
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(account.readings.split(separator: "\n").enumerated()), id: \.offset) { _, line in
                    Text(line)
                        .lineLimit(1)
                }
            }
            .font(.callout)
            .monospacedDigit()
            .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var status: some View {
        if let issue = account.issue {
            if issue.needsSignIn {
                Button("Sign In Again", action: signInAgain)
                    .controlSize(.small)
                    .buttonBorderShape(.capsule)
            }
        } else {
            HStack(spacing: 5) {
                Circle().fill(.green).frame(width: 6.5, height: 6.5)
                Text("OK").font(.subheadline).foregroundStyle(.secondary)
            }
        }
    }

    private var actions: some View {
        SwiftUI.Menu {
            Button("Rename…", action: rename)
            Button("Sign In Again", action: signInAgain)
            Divider()
            Button("Remove…", role: .destructive, action: remove)
        } label: {
            Image(systemName: "ellipsis")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("More actions for \(account.name)")
    }
}

/// The first-run state: nothing tracked yet.
private struct EmptyAccounts: View {
    let add: () -> Void

    // Laid out by hand: ContentUnavailableView puts its actions side by side on a tinted
    // card, which cuts the Keychain note off.
    var body: some View {
        VStack(spacing: 0) {
            Image(systemName: "person.crop.circle.badge.plus")
                .symbolRenderingMode(.palette)
                .foregroundStyle(.secondary, .tint)
                .font(.system(size: 44))
                .padding(.bottom, 14)
            Text("No Accounts Yet")
                .font(.title2.weight(.semibold))
                .padding(.bottom, 6)
            Text("Add a Claude, Codex, or Antigravity subscription to track its usage limits in Stint and your menu bar.")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 360)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 18)
            Button("Add Account…", action: add)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .padding(.bottom, 12)
            Label("Sessions stay in your macOS Keychain", systemImage: "lock")
                .font(.subheadline)
                .foregroundStyle(.tertiary)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Gives an account a new nickname, starting from its current one.
private struct RenameSheet: View {
    let account: AccountRow
    let rename: (String) throws -> Void
    @State private var name: String
    @State private var error: String?
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss

    init(account: AccountRow, rename: @escaping (String) throws -> Void) {
        self.account = account
        self.rename = rename
        _name = State(initialValue: account.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Rename Account").font(.headline)
            Text("Choose a new nickname for this \(account.provider.title) account.")
                .foregroundStyle(.secondary)
            TextField("Nickname", text: $name)
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(save)
            if let error {
                Text(error).font(.subheadline).foregroundStyle(.red)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Rename", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || name == account.name)
            }
            .padding(.top, 4)
        }
        .padding(20)
        .frame(width: 320)
        .onAppear { focused = true }
    }

    private func save() {
        do {
            try rename(name)
            dismiss()
        } catch {
            self.error = userMessage(error)
        }
    }
}
