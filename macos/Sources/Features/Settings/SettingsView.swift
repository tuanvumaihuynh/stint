import SwiftUI

/// The desktop window: a sidebar of panes and the selected pane.
struct SettingsView: View {
    let core: CoreClient
    @Bindable var model: SettingsModel
    @State private var query = ""

    var body: some View {
        NavigationSplitView {
            Sidebar(core: core, model: model, query: query)
                .navigationSplitViewColumnWidth(214)
        } detail: {
            detail
                .navigationTitle(model.pane.title)
                .toolbar {
                    ToolbarItem(placement: .navigation) {
                        ControlGroup {
                            Button("Back", systemImage: "chevron.left", action: model.goBack)
                                .disabled(!model.canGoBack)
                            Button("Forward", systemImage: "chevron.right", action: model.goForward)
                                .disabled(!model.canGoForward)
                        }
                        .controlGroupStyle(.navigation)
                    }
                }
        }
        .searchable(text: $query, placement: .sidebar, prompt: "Search Settings")
        .frame(minWidth: 640, minHeight: 440)
        .sheet(item: $model.signIn) { flow in
            AddAccountSheet(flow: flow) { model.signIn = nil }
        }
        .task {
            // Keeps "Updated 2m ago" and the like current while the window is open.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                core.reloadOverview()
            }
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch model.pane {
        case .general: GeneralPane(core: core)
        case .accounts: AccountsPane(core: core, model: model)
        case .menuBar: MenuBarPane(core: core)
        }
    }
}

private struct Sidebar: View {
    let core: CoreClient
    let model: SettingsModel
    let query: String

    var body: some View {
        List {
            AppHeader(summary: core.overview.accounts.isEmpty ? "No accounts yet" : core.overview.summary)
                .listRowInsets(EdgeInsets(top: 2, leading: 10, bottom: 10, trailing: 6))
                .listRowSeparator(.hidden)
            ForEach(Pane.allCases.filter { $0.matches(query) }) { pane in
                SidebarRow(
                    pane: pane,
                    selected: model.pane == pane,
                    badge: pane == .accounts && !core.overview.accounts.isEmpty
                        ? "\(core.overview.accounts.count)" : nil
                ) {
                    model.show(pane)
                }
                .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
            }
        }
        .listStyle(.sidebar)
        .scrollDisabled(true)
    }
}

private struct AppHeader: View {
    let summary: String

    var body: some View {
        HStack(spacing: 8) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 31, height: 31)
            VStack(alignment: .leading, spacing: 0) {
                Text("Stint").font(.system(size: 13, weight: .semibold))
                Text(summary)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }
}

/// A sidebar row. The selected one gets a neutral gray highlight, as in System Settings.
private struct SidebarRow: View {
    let pane: Pane
    let selected: Bool
    let badge: String?
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack(spacing: 9) {
                IconTile(symbol: pane.symbol, colors: pane.tile)
                Text(pane.title)
                    .font(.system(size: 13, weight: selected ? .semibold : .regular))
                    .foregroundStyle(.primary)
                Spacer()
                if let badge {
                    Text(badge).font(.system(size: 11)).foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 8)
            .frame(height: 32)
            .background {
                if selected {
                    RoundedRectangle(cornerRadius: 9, style: .continuous)
                        .fill(.primary.opacity(0.07))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
