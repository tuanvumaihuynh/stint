#if DEBUG
    import AppKit
    import ServiceManagement

    /// Photographs the desktop window in every pane and Add Account step, in light and dark
    /// mode, for design review: `Stint --window-snapshot <dir>` (debug builds only).
    @MainActor
    final class WindowSnapshotter: NSObject, NSApplicationDelegate {
        // Lets the window, a sheet, or an alert finish drawing or animating away.
        private static let settleDelay: Duration = .milliseconds(900)

        private let directory: URL
        private var windows: [String: SettingsWindowController] = [:]
        private var cores: [CoreClient] = []

        /// The output directory when the app was started with `--window-snapshot <dir>`.
        static func requestedDirectory() -> URL? {
            let arguments = CommandLine.arguments
            guard let flag = arguments.firstIndex(of: "--window-snapshot"), flag + 1 < arguments.count else {
                return nil
            }
            return URL(fileURLWithPath: arguments[flag + 1], isDirectory: true)
        }

        init(directory: URL) {
            self.directory = directory
        }

        func applicationDidFinishLaunching(_ notification: Notification) {
            Task {
                do {
                    try await photographAll()
                    exit(0)
                } catch {
                    FileHandle.standardError.write(Data("stint: snapshot: \(error)\n".utf8))
                    exit(1)
                }
            }
        }

        // Release builds show the Updates section; snapshots show it with made-up state.
        private let updates = Updates.preview(available: nil, lastChecked: Date(timeIntervalSinceNow: -2 * 3600))

        private struct Shot {
            let name: String
            let scenario: String
            let pane: Pane
            var prepare: @MainActor (SettingsWindowController, CoreClient) -> Void = { _, _ in }
        }

        private var shots: [Shot] {
            let flow = { (step: SignInFlow.Step, provider: Provider) in
                { @MainActor (window: SettingsWindowController, _: CoreClient) in
                    window.model.signIn = SignInFlow(previewing: step, provider: provider, nickname: "Client")
                }
            }
            return [
                Shot(name: "general", scenario: "default", pane: .general),
                Shot(name: "general-update", scenario: "default", pane: .general) { _, _ in
                    self.updates.preview(available: "0.2.0")
                },
                Shot(name: "accounts", scenario: "default", pane: .accounts),
                Shot(name: "accounts-empty", scenario: "empty", pane: .accounts),
                Shot(name: "accounts-remove", scenario: "default", pane: .accounts) { window, core in
                    window.model.removing = core.overview.accounts.first { $0.name == "Work" }
                },
                Shot(name: "add-1-provider", scenario: "default", pane: .accounts, prepare: flow(.provider, .claude)),
                Shot(name: "add-2-nickname", scenario: "default", pane: .accounts, prepare: flow(.nickname, .claude)),
                Shot(name: "add-3-waiting", scenario: "default", pane: .accounts, prepare: flow(.waiting, .claude)),
                Shot(
                    name: "add-4-connected", scenario: "default", pane: .accounts,
                    prepare: flow(.connected(email: "client@example.com", plan: "Max 5x"), .claude)
                ),
                Shot(
                    name: "add-4-failed", scenario: "default", pane: .accounts,
                    prepare: flow(.failed("The sign-in page expired or was closed before Claude returned a session."), .claude)
                ),
                Shot(name: "accounts-antigravity", scenario: "antigravity", pane: .accounts),
                Shot(
                    name: "add-1-provider-antigravity", scenario: "antigravity", pane: .accounts,
                    prepare: flow(.provider, .antigravity)
                ),
                Shot(
                    name: "add-3-waiting-antigravity", scenario: "antigravity", pane: .accounts,
                    prepare: flow(.waiting, .antigravity)
                ),
                Shot(name: "menubar", scenario: "default", pane: .menuBar) { _, core in
                    if let side = core.overview.accounts.first(where: { $0.name == "Side" }) {
                        try? core.setShownInMenu(side.id, false)
                    }
                },
            ]
        }

        private func photographAll() async throws {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            // The windows never take the keyboard, but a stray click could still flip Launch at
            // login, which is a real system setting; it is put back afterwards.
            let loginItem = SMAppService.mainApp.status
            print("launch at login: \(loginItem.rawValue) (0 not registered, 1 enabled, 2 needs approval, 3 not found)")
            defer {
                let after = SMAppService.mainApp.status
                if after == .enabled, loginItem != .enabled {
                    try? SMAppService.mainApp.unregister()
                    print("launch at login was switched on during the snapshots; switched it back off")
                }
            }
            for (name, mode) in [(NSAppearance.Name.aqua, "light"), (.darkAqua, "dark")] {
                NSApp.appearance = NSAppearance(named: name)
                for shot in shots {
                    let (window, core) = try controller(for: shot.scenario)
                    window.present(shot.pane)
                    if let shown = window.window {
                        shown.setContentSize(SettingsWindowController.size)
                        try await place(shown)
                    }
                    shot.prepare(window, core)
                    try await Task.sleep(for: Self.settleDelay)
                    for _ in 0..<3 where window.window?.occlusionState.contains(.visible) == false {
                        if let shown = window.window { try await place(shown) }
                        try await Task.sleep(for: Self.settleDelay)
                    }
                    let file = directory.appendingPathComponent("window-\(shot.name)-\(mode).png")
                    try capture(window, to: file)
                    print("wrote \(file.path)")
                    window.model.signIn = nil
                    window.model.removing = nil
                    updates.preview(available: nil)
                    window.window?.orderOut(nil)
                    try await Task.sleep(for: Self.settleDelay)
                }
            }
        }

        /// Centers the window on the first screen where nothing covers it, such as a
        /// full-screen app.
        private func place(_ window: NSWindow) async throws {
            for screen in NSScreen.screens {
                let visible = screen.visibleFrame
                window.setFrameOrigin(NSPoint(
                    x: visible.midX - window.frame.width / 2,
                    y: visible.midY - window.frame.height / 2
                ))
                window.orderFrontRegardless()
                try await Task.sleep(for: .milliseconds(300))
                if window.occlusionState.contains(.visible) {
                    return
                }
            }
        }

        private func controller(for scenario: String) throws -> (SettingsWindowController, CoreClient) {
            if let window = windows[scenario], let core = cores.first(where: { $0.scenario == scenario }) {
                return (window, core)
            }
            let core = try CoreClient(mock: scenario)
            let window = SettingsWindowController(core: core, updates: updates)
            cores.append(core)
            windows[scenario] = window
            return (window, core)
        }

        /// Captures the window together with any sheet or alert attached to it.
        private func capture(_ controller: SettingsWindowController, to file: URL) throws {
            guard let window = controller.window else { throw SnapshotError("no window") }
            guard window.occlusionState.contains(.visible) else {
                throw SnapshotError("the window is covered, so it cannot be captured")
            }
            var ids = [CGWindowID(window.windowNumber)]
            if let sheet = window.attachedSheet {
                ids.append(CGWindowID(sheet.windowNumber))
            }
            guard let image = Self.createImage(of: ids) else { throw SnapshotError("capture the window") }
            let bitmap = NSBitmapImageRep(cgImage: image)
            guard let png = bitmap.representation(using: .png, properties: [:]) else {
                throw SnapshotError("encode the capture as PNG")
            }
            try png.write(to: file)
        }

        /// The windows composited bottom to top, each captured on its own with
        /// `CGWindowListCreateImage`, looked up at run time as in the menu snapshots.
        private static func createImage(of windows: [CGWindowID]) -> CGImage? {
            typealias CreateImage = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else {
                return nil
            }
            let create = unsafeBitCast(symbol, to: CreateImage.self)
            let includingWindow = CGWindowListOption.optionIncludingWindow.rawValue
            let options = CGWindowImageOption([.boundsIgnoreFraming, .bestResolution]).rawValue
            let layers = windows.compactMap { id -> (CGImage, CGRect)? in
                guard let image = create(.null, includingWindow, id, options)?.takeRetainedValue(),
                      let bounds = bounds(of: id)
                else { return nil }
                return (image, bounds)
            }
            guard let (base, frame) = layers.first, let space = CGColorSpace(name: CGColorSpace.sRGB) else {
                return nil
            }
            let scale = CGFloat(base.width) / frame.width
            guard let context = CGContext(
                data: nil, width: base.width, height: base.height, bitsPerComponent: 8, bytesPerRow: 0,
                space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return nil }
            for (image, bounds) in layers {
                // Window bounds are top-left based; the context is bottom-left based.
                let rect = CGRect(
                    x: (bounds.minX - frame.minX) * scale,
                    y: (frame.maxY - bounds.maxY) * scale,
                    width: CGFloat(image.width),
                    height: CGFloat(image.height)
                )
                context.draw(image, in: rect)
            }
            return context.makeImage()
        }

        private static func bounds(of window: CGWindowID) -> CGRect? {
            let info = CGWindowListCopyWindowInfo(.optionIncludingWindow, window) as? [[String: Any]]
            guard let bounds = info?.first?[kCGWindowBounds as String] as? NSDictionary else { return nil }
            return CGRect(dictionaryRepresentation: bounds as CFDictionary)
        }
    }

    private struct SnapshotError: Error, CustomStringConvertible {
        let description: String

        init(_ description: String) {
            self.description = description
        }
    }
#endif
