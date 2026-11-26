import ServiceManagement
import SwiftUI

/// The refresh interval, launching at login, and what Stint is tracking.
struct GeneralPane: View {
    let core: CoreClient
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                HStack(spacing: 14) {
                    AppLogo(size: 30)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Stint \(Self.version)").font(.body.weight(.semibold))
                        Text(status).font(.subheadline).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 4)
            }

            Section("Refresh & Startup") {
                Picker(selection: refreshMinutes) {
                    ForEach(core.refreshChoices, id: \.self) { minutes in
                        Text("\(minutes) minutes").tag(minutes)
                    }
                } label: {
                    Text("Refresh every")
                    Text("Check subscription usage every \(choicesInWords).")
                }
                Toggle(isOn: launchAtLoginBinding) {
                    Text("Launch at login")
                    Text("Start Stint automatically when you sign in to macOS.")
                }
            }
        }
        .formStyle(.grouped)
        .alert("Couldn't Change the Setting", isPresented: errorShown) {
            Button("OK") {}
        } message: {
            Text(error ?? "")
        }
    }

    private var status: String {
        let overview = core.overview
        guard !overview.accounts.isEmpty else { return "No accounts connected yet" }
        return [overview.summary, overview.updated].compactMap(\.self).joined(separator: " · ")
    }

    private var choicesInWords: String {
        let choices = core.refreshChoices.map(String.init)
        guard let last = choices.last else { return "" }
        return choices.dropLast().joined(separator: ", ") + ", or \(last) minutes"
    }

    private var refreshMinutes: Binding<UInt32> {
        Binding {
            core.overview.refreshMinutes
        } set: { minutes in
            do { try core.setRefreshMinutes(minutes) } catch { self.error = userMessage(error) }
        }
    }

    private var launchAtLoginBinding: Binding<Bool> {
        Binding {
            launchAtLogin
        } set: { enabled in
            do {
                if enabled {
                    try SMAppService.mainApp.register()
                } else {
                    try SMAppService.mainApp.unregister()
                }
            } catch {
                self.error = error.localizedDescription
            }
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }

    private var errorShown: Binding<Bool> {
        Binding { error != nil } set: { if !$0 { error = nil } }
    }

    private static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }
}
