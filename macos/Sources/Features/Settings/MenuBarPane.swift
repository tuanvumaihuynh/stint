import SwiftUI

/// Which account the menu bar shows, and which accounts the status menu lists.
struct MenuBarPane: View {
    let core: CoreClient
    @State private var error: String?

    var body: some View {
        let accounts = core.overview.accounts
        let firstProvider = Provider.all.first { provider in accounts.contains { $0.provider == provider } }
        Form {
            if accounts.isEmpty {
                Section {
                    Text("Accounts you add appear here.").foregroundStyle(.secondary)
                }
            } else {
                Section {
                    Picker("Show in menu bar", selection: pinned) {
                        ForEach(accounts, id: \.id) { account in
                            Text("\(account.provider.title) · \(account.name)").tag(Optional(account.id))
                        }
                    }
                    Picker("Display style", selection: style) {
                        Text("Icon only").tag(MenuBarStyle.icon)
                        Text("Icon + 5-hour %").tag(MenuBarStyle.iconPercent)
                    }
                }
            }
            ForEach(Provider.all, id: \.self) { provider in
                let rows = accounts.filter { $0.provider == provider }
                if !rows.isEmpty {
                    Section {
                        ForEach(rows, id: \.id) { account in
                            MenuBarRow(account: account) { shown in
                                do {
                                    try core.setShownInMenu(account.id, shown)
                                } catch {
                                    self.error = userMessage(error)
                                }
                            }
                        }
                    } header: {
                        VStack(alignment: .leading, spacing: 12) {
                            if provider == firstProvider {
                                Text("Choose which accounts appear in the menu bar menu. Hidden accounts continue updating in this window.")
                                    .font(.body)
                                    .fontWeight(.regular)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Text(provider.title)
                        }
                        .textCase(nil)
                    }
                }
            }
            if !accounts.isEmpty {
                Section {} footer: {
                    HStack {
                        let shown = accounts.filter(\.shownInMenu).count
                        Text("\(shown) of \(accounts.count) accounts visible in menu bar")
                        Spacer()
                        Text("Order follows provider groups")
                    }
                    .font(.subheadline)
                    .foregroundStyle(.tertiary)
                }
            }
        }
        .formStyle(.grouped)
        .alert("Couldn't Change the Menu", isPresented: Binding { error != nil } set: { if !$0 { error = nil } }) {
            Button("OK") {}
        } message: {
            Text(error ?? "")
        }
    }
}

extension MenuBarPane {
    private var pinned: Binding<Int64?> {
        Binding {
            core.overview.pinnedAccount
        } set: { id in
            guard let id else { return }
            change { try core.setPinnedAccount(id) }
        }
    }

    private var style: Binding<MenuBarStyle> {
        Binding {
            core.overview.menuBarStyle
        } set: { style in
            change { try core.setMenuBarStyle(style) }
        }
    }

    private func change(_ action: () throws -> Void) {
        do {
            try action()
        } catch {
            self.error = userMessage(error)
        }
    }
}

private struct MenuBarRow: View {
    let account: AccountRow
    let setShown: (Bool) -> Void

    var body: some View {
        HStack {
            Toggle(isOn: Binding(get: { account.shownInMenu }, set: { setShown($0) })) {
                VStack(alignment: .leading, spacing: 2) {
                    (Text(account.name).fontWeight(.semibold)
                        + Text(account.plan.map { " · \($0)" } ?? "").foregroundStyle(.secondary))
                        .font(.body)
                    subtitle.font(.subheadline)
                }
                .padding(.leading, 4)
            }
            .toggleStyle(.checkbox)
            Spacer()
            Text(account.shownInMenu ? "Shown" : "Hidden")
                .font(.subheadline)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }

    private var subtitle: Text {
        let email = Text(account.email ?? "").foregroundStyle(.secondary)
        guard let issue = account.issue else { return email }
        let separator = account.email == nil ? "" : " · "
        return email + Text(separator).foregroundStyle(.secondary)
            + Text(issue.title).foregroundStyle(issue.severity.color)
    }
}
