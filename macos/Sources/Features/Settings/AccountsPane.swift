import SwiftUI

/// Every account, grouped by provider, with what each one last reported.
struct AccountsPane: View {
    let core: CoreClient
    @Bindable var model: SettingsModel
    @State private var renaming: AccountRow?
    @State private var newName = ""
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
                                        rename: {
                                            newName = account.name
                                            renaming = account
                                        },
                                        remove: { model.removing = account }
                                    )
                                }
                            }
                        }
                    }
                    Section {} footer: {
                        Text(overview.footer)
                            .font(.system(size: 11))
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
                Button("Add Account…", action: addAccount)
                    .buttonStyle(.borderedProminent)
            }
        }
        .alert("Rename Account", isPresented: shown($renaming)) {
            TextField("Nickname", text: $newName)
            Button("Rename") {
                if let account = renaming {
                    run { try core.rename(account.id, to: newName) }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            if let account = renaming {
                Text("Choose a new nickname for this \(account.provider.title) account.")
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
                    Text(account.name).font(.system(size: 13, weight: .semibold))
                    if let plan = account.plan {
                        Badge(text: plan)
                    }
                    if let email = account.email {
                        Text(email)
                            .font(.system(size: 11.5))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                .frame(minHeight: 22)
                detail
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 5) {
                HStack(spacing: 8) {
                    status
                    actions
                }
                .frame(minHeight: 22)
                if let updated = account.updated {
                    Text(updated)
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
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
                    .font(.system(size: 11))
                    .foregroundStyle(issue.severity.color)
                (Text(issue.title).fontWeight(.medium).foregroundStyle(issue.severity.color)
                    + Text(" — \(issue.detail)").foregroundStyle(.secondary))
                    .font(.system(size: 12))
            }
        } else {
            Text(account.readings)
                .font(.system(size: 12))
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
                Text("OK").font(.system(size: 11.5)).foregroundStyle(.secondary)
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

    var body: some View {
        ContentUnavailableView {
            Label {
                Text("No Accounts Yet")
            } icon: {
                Image(systemName: "person.crop.circle.badge.plus")
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.secondary, .tint)
            }
        } description: {
            Text("Add a Claude or Codex subscription to track 5-hour and weekly usage limits in Stint and your menu bar.")
        } actions: {
            Button("Add Account…", action: add)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            Label("Sessions stay in your macOS Keychain", systemImage: "lock")
                .font(.system(size: 11.5))
                .foregroundStyle(.tertiary)
                .padding(.top, 8)
        }
    }
}
