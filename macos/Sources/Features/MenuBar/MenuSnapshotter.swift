#if DEBUG
    import AppKit

    /// Photographs the status menu with every mock scenario, in light and dark mode, for
    /// design review: `Stint --snapshot <dir>` (debug builds only).
    ///
    /// The menu is opened for real and its window captured while it is open. An app may
    /// capture its own windows without the Screen Recording permission.
    @MainActor
    final class MenuSnapshotter: NSObject, NSApplicationDelegate {
        // Waits that let the status item appear and an opened menu finish drawing.
        private static let startupDelay: TimeInterval = 0.5
        private static let drawDelay: TimeInterval = 0.4
        // Menus sit at the pop-up menu window level.
        private static let menuWindowLayer = 101

        private let directory: URL
        private var current: Menu?
        private var controller: StatusMenuController?

        /// The output directory when the app was started with `--snapshot <dir>`.
        static func requestedDirectory() -> URL? {
            let arguments = CommandLine.arguments
            guard let flag = arguments.firstIndex(of: "--snapshot"), flag + 1 < arguments.count else {
                return nil
            }
            return URL(fileURLWithPath: arguments[flag + 1], isDirectory: true)
        }

        init(directory: URL) {
            self.directory = directory
        }

        func applicationDidFinishLaunching(_ notification: Notification) {
            controller = StatusMenuController(menu: { [weak self] in
                self?.current ?? Menu(entries: [], offline: false)
            }, perform: { _ in })
            Timer.scheduledTimer(withTimeInterval: Self.startupDelay, repeats: false) { _ in
                MainActor.assumeIsolated {
                    do {
                        try self.photographAll()
                        exit(0)
                    } catch {
                        FileHandle.standardError.write(Data("stint: snapshot: \(error)\n".utf8))
                        exit(1)
                    }
                }
            }
        }

        private func photographAll() throws {
            guard let controller else { return }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            for (name, mode) in [(NSAppearance.Name.aqua, "light"), (.darkAqua, "dark")] {
                let appearance = NSAppearance(named: name)
                NSApp.appearance = appearance
                controller.setAppearance(appearance)
                for scenario in mockScenarios() {
                    current = mockMenu(scenario: scenario)
                    controller.update()
                    let file = directory.appendingPathComponent("menu-\(scenario)-\(mode).png")
                    try photograph(to: file)
                    print("wrote \(file.path)")
                }
            }
        }

        /// Opens the menu, captures it once drawn, and closes it.
        private func photograph(to file: URL) throws {
            guard let controller else { return }
            var outcome: Result<Void, Error>?
            // Common modes let the timer fire while the menu tracks the mouse.
            let timer = Timer(timeInterval: Self.drawDelay, repeats: false) { _ in
                MainActor.assumeIsolated {
                    outcome = Result { try Self.captureMenu(to: file) }
                    controller.close()
                }
            }
            RunLoop.main.add(timer, forMode: .common)
            controller.open()
            guard let outcome else { throw SnapshotError("the menu closed before it was captured") }
            try outcome.get()
        }

        private static func captureMenu(to file: URL) throws {
            guard let window = menuWindow() else { throw SnapshotError("no open menu window") }
            guard let image = createWindowImage(window) else {
                throw SnapshotError("capture the menu window")
            }
            let bitmap = NSBitmapImageRep(cgImage: image)
            guard let png = bitmap.representation(using: .png, properties: [:]) else {
                throw SnapshotError("encode the capture as PNG")
            }
            try png.write(to: file)
        }

        /// The window number of this process's open menu.
        private static func menuWindow() -> CGWindowID? {
            let pid = Int(ProcessInfo.processInfo.processIdentifier)
            let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]]
            return windows?.first { info in
                info[kCGWindowOwnerPID as String] as? Int == pid
                    && info[kCGWindowLayer as String] as? Int == menuWindowLayer
            }.flatMap { $0[kCGWindowNumber as String] as? CGWindowID }
        }

        /// `CGWindowListCreateImage`, looked up at run time: Swift marks it unavailable in
        /// favor of ScreenCaptureKit, which needs the Screen Recording permission even for
        /// the app's own windows.
        private static func createWindowImage(_ window: CGWindowID) -> CGImage? {
            typealias CreateImage = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else {
                return nil
            }
            let create = unsafeBitCast(symbol, to: CreateImage.self)
            let includingWindow = CGWindowListOption.optionIncludingWindow.rawValue
            let options = CGWindowImageOption([.boundsIgnoreFraming, .bestResolution]).rawValue
            return create(.null, includingWindow, window, options)?.takeRetainedValue()
        }
    }

    private struct SnapshotError: Error, CustomStringConvertible {
        let description: String

        init(_ description: String) {
            self.description = description
        }
    }
#endif
