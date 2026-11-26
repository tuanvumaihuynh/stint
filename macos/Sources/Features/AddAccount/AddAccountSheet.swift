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
                .font(.headline)
            Spacer()
            if case .failed = flow.step {
                Text("Error")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.orange)
            } else {
                Text("Step \(flow.step.number) of 4")
                    .font(.subheadline.weight(.medium))
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
                Button("Sign In", action: flow.signIn)
                    .keyboardShortcut(.defaultAction)
                .disabled(flow.nickname.trimmingCharacters(in: .whitespaces).isEmpty)
            case .waiting:
                Text("Account: \(flow.nickname) (\(flow.provider.title))")
                    .font(.body)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
            case .connected:
                if flow.account == nil {
                    Text("Added to menu bar by default")
                        .font(.body)
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
                .font(.body)
                .foregroundStyle(.secondary)
            Picker("Provider", selection: $flow.provider) {
                ForEach(Provider.all, id: \.self) { provider in
                    ProviderChoice(provider: provider)
                        .tag(provider)
                        .selectionDisabled(!provider.available)
                }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .groupBox()
        }
    }
}

private struct ProviderChoice: View {
    let provider: Provider

    var body: some View {
        HStack(spacing: 10) {
            ProviderLogo(provider: provider, size: 28)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(provider.title).font(.body.weight(.medium))
                    if !provider.available {
                        Text("Coming later")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                Text(provider.subscription)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 5)
        .padding(.leading, 4)
        .opacity(provider.available ? 1 : 0.5)
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
            .font(.body)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            VStack(spacing: 0) {
                HStack {
                    Text("Provider").foregroundStyle(.secondary).frame(width: 80, alignment: .leading)
                    ProviderLogo(provider: flow.provider, size: 18)
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
            .font(.body)
            .padding(.horizontal, 12)
            .groupBox()

            if let error = flow.nicknameError {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(.red)
            } else {
                Label("Stint stores this account's credentials in your macOS Keychain.", systemImage: "lock")
                    .font(.subheadline)
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
                .font(.headline)
            (Text("Finish signing in to \(flow.provider.title) for ")
                + Text(flow.nickname).fontWeight(.semibold).foregroundStyle(.primary)
                + Text(" in your browser. Stint will detect it automatically."))
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            Button("Open the Sign-In Page Again", action: flow.openLinkAgain)
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
                .font(.body)
            Text(flow.codeSent ? "Code sent. Waiting for Claude to confirm…" : "Paste the authorization code shown in the browser.")
                .font(.subheadline)
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
                Text(error).font(.subheadline).foregroundStyle(.red)
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
                .font(.headline)
            Text("Stint verified your session and will start tracking quota limits.")
                .font(.body)
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
            .font(.body)
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
                .font(.headline)
            Group {
                if flow.account == nil {
                    Text("\(message) Your nickname (")
                        + Text(flow.nickname).fontWeight(.semibold).foregroundStyle(.primary)
                        + Text(") is kept—try signing in again.")
                } else {
                    Text(message)
                }
            }
            .font(.body)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity)
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
