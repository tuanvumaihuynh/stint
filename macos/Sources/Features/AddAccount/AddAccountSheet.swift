import SwiftUI

/// Adds an account in four steps: provider, nickname, browser sign-in, outcome. Signing an
/// account in again starts at the sign-in.
struct AddAccountSheet: View {
    @Bindable var flow: SignInFlow
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            content
                .padding(.horizontal, 18)
                .padding(.top, 2)
                .padding(.bottom, 16)
            Divider()
            footer
                .padding(.horizontal, 18)
                .padding(.top, 11)
                .padding(.bottom, 14)
        }
        .frame(width: 428)
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(flow.title)
                .font(.system(size: 14.5, weight: .semibold))
            Spacer()
            if case .failed = flow.step {
                Text("Error")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.orange)
            } else {
                Text("Step \(flow.step.number) of 4")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 18)
        .padding(.top, 16)
        .padding(.bottom, 10)
    }

    @ViewBuilder
    private var content: some View {
        switch flow.step {
        case .provider: ProviderStep(flow: flow)
        case .nickname: NicknameStep(flow: flow)
        case .waiting: WaitingStep(flow: flow)
        case let .connected(email, plan): ConnectedStep(flow: flow, email: email, plan: plan)
        case let .failed(message): FailedStep(flow: flow, message: message)
        }
    }

    @ViewBuilder
    private var footer: some View {
        HStack {
            switch flow.step {
            case .provider:
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Spacer()
                Button("Continue", action: flow.chooseProvider)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!flow.provider.available)
            case .nickname:
                Button("Back", action: flow.goBack)
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Button(action: flow.signIn) {
                    Label("Sign In", systemImage: "arrow.up.forward.square")
                        .labelStyle(TrailingIconLabelStyle())
                }
                .keyboardShortcut(.defaultAction)
                .disabled(flow.nickname.trimmingCharacters(in: .whitespaces).isEmpty)
            case .waiting:
                Text("Account: \(flow.nickname) (\(flow.provider.title))")
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
            case .connected:
                if flow.account == nil {
                    Text("Added to menu bar by default")
                        .font(.system(size: 12.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done", action: close).keyboardShortcut(.defaultAction)
            case .failed:
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Spacer()
                Button("Try Again", action: flow.tryAgain).keyboardShortcut(.defaultAction)
            }
        }
        .controlSize(.large)
    }

    private func cancel() {
        flow.cancel()
        close()
    }
}

private struct ProviderStep: View {
    @Bindable var flow: SignInFlow

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Choose a subscription provider to connect to Stint.")
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(Array(Provider.all.enumerated()), id: \.element) { index, provider in
                    if index > 0 {
                        Divider().padding(.horizontal, 14)
                    }
                    ProviderChoice(provider: provider, selected: flow.provider == provider) {
                        flow.provider = provider
                    }
                }
            }
            .groupBox()
        }
    }
}

private struct ProviderChoice: View {
    let provider: Provider
    let selected: Bool
    let choose: () -> Void

    var body: some View {
        Button(action: choose) {
            HStack(spacing: 12) {
                IconTile(symbol: provider.symbol, colors: provider.tile, size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(provider.title).font(.system(size: 13, weight: .semibold))
                    Text(provider.subscription)
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if provider.available {
                    Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                        .font(.system(size: 16))
                        .foregroundStyle(selected ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                } else {
                    Badge(text: "Coming later")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!provider.available)
        .opacity(provider.available ? 1 : 0.48)
        .background {
            if selected {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(.tint, lineWidth: 2)
                    .background(.background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .padding(3)
            }
        }
    }
}

private struct NicknameStep: View {
    @Bindable var flow: SignInFlow
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(
                "Give this account a short nickname to tell it apart in the menu bar. "
                    + "Clicking Sign In opens \(flow.provider.title) in your default browser."
            )
            .font(.system(size: 12.5))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            VStack(spacing: 0) {
                HStack {
                    Text("Provider").foregroundStyle(.secondary).frame(width: 80, alignment: .leading)
                    IconTile(symbol: flow.provider.symbol, colors: flow.provider.tile, size: 18)
                    Text(flow.provider.title).fontWeight(.semibold)
                    Spacer()
                }
                .padding(.vertical, 10)
                Divider()
                HStack {
                    Text("Nickname").foregroundStyle(.secondary).frame(width: 80, alignment: .leading)
                    TextField("Nickname", text: $flow.nickname, prompt: Text("Personal"))
                        .labelsHidden()
                        .textFieldStyle(.roundedBorder)
                        .controlSize(.large)
                        .focused($focused)
                        .onSubmit(flow.signIn)
                }
                .padding(.vertical, 8)
            }
            .font(.system(size: 13))
            .padding(.horizontal, 12)
            .groupBox()

            if let error = flow.nicknameError {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.system(size: 11.5))
                    .foregroundStyle(.red)
            } else {
                Label("Stint stores this account's credentials in your macOS Keychain.", systemImage: "lock")
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
            }
        }
        .onAppear { focused = true }
    }
}

private struct WaitingStep: View {
    @Bindable var flow: SignInFlow

    var body: some View {
        VStack(spacing: 10) {
            ProgressView().controlSize(.regular).padding(.top, 8)
            Text("Waiting for browser sign-in…")
                .font(.system(size: 14, weight: .semibold))
            (Text("Finish signing in to \(flow.provider.title) for ")
                + Text(flow.nickname).fontWeight(.semibold).foregroundStyle(.primary)
                + Text(" in your browser. Stint will detect it automatically."))
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            Button(action: flow.openLinkAgain) {
                Label("Open the sign-in page again", systemImage: "arrow.up.forward.square")
                    .labelStyle(TrailingIconLabelStyle())
            }
            .buttonStyle(.bordered)
            .buttonBorderShape(.capsule)
            .disabled(flow.link == nil)
            .padding(.bottom, 4)

            if flow.provider == .claude {
                PasteCodeBox(flow: flow)
            }
        }
        .frame(maxWidth: .infinity)
    }
}

private struct PasteCodeBox: View {
    @Bindable var flow: SignInFlow

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            (Text("Didn't return automatically? ").fontWeight(.semibold)
                + Text("(Claude only)").foregroundStyle(.tertiary))
                .font(.system(size: 12.5))
            Text(flow.codeSent ? "Code sent. Waiting for Claude to confirm…" : "Paste the authorization code shown in the browser.")
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
            HStack {
                TextField("Code", text: $flow.code, prompt: Text("Paste code here…"))
                    .labelsHidden()
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.large)
                    .onSubmit(flow.submitCode)
                Button("Submit", action: flow.submitCode)
                    .controlSize(.large)
                    .disabled(flow.code.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if let error = flow.codeError {
                Text(error).font(.system(size: 11.5)).foregroundStyle(.red)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .groupBox()
    }
}

private struct ConnectedStep: View {
    let flow: SignInFlow
    let email: String?
    let plan: String?

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 30))
                .symbolRenderingMode(.palette)
                .foregroundStyle(.white, .green)
                .padding(.top, 8)
            Text("\(flow.provider.title) account ready")
                .font(.system(size: 14, weight: .semibold))
            Text("Stint verified your session and will start tracking quota limits.")
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            VStack(spacing: 0) {
                detail("Nickname") { Text(flow.nickname).fontWeight(.semibold) }
                if let email {
                    Divider()
                    detail("Email") { Text(email).fontWeight(.medium) }
                }
                if let plan {
                    Divider()
                    detail("Plan") { Badge(text: plan) }
                }
            }
            .font(.system(size: 13))
            .padding(.horizontal, 12)
            .groupBox()
            .padding(.top, 4)
        }
        .frame(maxWidth: .infinity)
    }

    private func detail(_ label: String, @ViewBuilder value: () -> some View) -> some View {
        HStack {
            Text(label).foregroundStyle(.secondary)
            Spacer()
            value()
        }
        .padding(.vertical, 9)
    }
}

private struct FailedStep: View {
    let flow: SignInFlow
    let message: String

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 18))
                .foregroundStyle(.orange)
                .frame(width: 32, height: 32)
                .background(.orange.opacity(0.16), in: Circle())
                .padding(.top, 8)
            Text("Couldn't finish signing in")
                .font(.system(size: 14, weight: .semibold))
            Group {
                if flow.account == nil {
                    Text("\(message) Your nickname (")
                        + Text(flow.nickname).fontWeight(.semibold).foregroundStyle(.primary)
                        + Text(") is kept—try signing in again.")
                } else {
                    Text(message)
                }
            }
            .font(.system(size: 12.5))
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity)
    }
}

/// Puts a label's icon after its title, as on "Sign In ↗".
private struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 5) {
            configuration.title
            configuration.icon
        }
    }
}

extension View {
    /// The rounded, lightly filled box that groups rows, as in grouped forms.
    func groupBox() -> some View {
        background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(.separator.opacity(0.6), lineWidth: 0.5)
            }
    }
}
