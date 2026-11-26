import AppKit

/// The status bar item and its native menu, drawn from the core's menu entries.
@MainActor
final class StatusMenuController: NSObject, NSMenuDelegate {
    // Sizes in points.
    private static let statusIconSize: CGFloat = 18
    private static let headerIconSize: CGFloat = 28
    private static let menuWidth: CGFloat = 298
    private static let titleSize: CGFloat = 13
    private static let statusSize: CGFloat = 11.5
    // The status icon is dimmed while every account is offline.
    private static let offlineAlpha: CGFloat = 0.45

    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let statusMenu = NSMenu()
    private let source: () -> Menu
    private let perform: (Command) -> Void
    // The command of each command item, by its tag.
    private var commands: [Command] = []

    /// `menu` is read every time the menu opens; `perform` runs a chosen command.
    init(menu: @escaping () -> Menu, perform: @escaping (Command) -> Void) {
        source = menu
        self.perform = perform
        super.init()
        if let icon = NSImage(named: "StatusIcon") {
            icon.size = NSSize(width: Self.statusIconSize, height: Self.statusIconSize)
            // Tinted by macOS for light and dark menu bars.
            icon.isTemplate = true
            statusItem.button?.image = icon
        }
        // Items without an action stay enabled instead of being dimmed.
        statusMenu.autoenablesItems = false
        statusMenu.minimumWidth = Self.menuWidth
        statusMenu.delegate = self
        statusItem.menu = statusMenu
    }

    /// Shows the current menu, also in a menu that is open right now.
    func update() {
        let menu = source()
        statusItem.button?.alphaValue = menu.offline ? Self.offlineAlpha : 1
        rebuild(from: menu)
    }

    // Rebuilt on every open, so relative times like "Updated 2m ago" are current.
    func menuNeedsUpdate(_ menu: NSMenu) {
        rebuild(from: source())
    }

    private func rebuild(from menu: Menu) {
        statusMenu.removeAllItems()
        commands.removeAll()
        for entry in menu.entries {
            statusMenu.addItem(item(for: entry))
        }
    }

    private func item(for entry: Entry) -> NSMenuItem {
        switch entry {
        case .separator:
            return .separator()
        case .section(let title):
            return .sectionHeader(title: title)
        case .header(let status, let dot):
            let item = passive("Stint")
            item.attributedTitle = Self.headerTitle(status: status, dot: dot)
            if let icon = NSImage(named: "MenuHeaderIcon") {
                icon.size = NSSize(width: Self.headerIconSize, height: Self.headerIconSize)
                item.image = icon
            }
            return item
        case .account(let name, let plan, let subtitle):
            let plain = plan.map { "\(name) · \($0)" } ?? name
            let item = passive(plain, subtitle: subtitle)
            item.attributedTitle = Self.accountTitle(name: name, plan: plan)
            return item
        case .notice(let title, let subtitle):
            return passive(title, subtitle: subtitle)
        case .command(let command, let subtitle, let enabled):
            // A custom selector: system ones such as `terminate:` get automatic icons.
            let item = NSMenuItem(
                title: commandTitle(command: command),
                action: #selector(runCommand(_:)),
                keyEquivalent: commandKey(command: command)
            )
            item.target = self
            item.tag = commands.count
            commands.append(command)
            item.subtitle = subtitle
            item.isEnabled = enabled
            return item
        }
    }

    @objc private func runCommand(_ sender: NSMenuItem) {
        guard commands.indices.contains(sender.tag) else { return }
        perform(commands[sender.tag])
    }

    /// An item with no action. It stays enabled so macOS draws it at full contrast; a click
    /// only closes the menu.
    private func passive(_ title: String, subtitle: String? = nil) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.subtitle = subtitle
        item.isEnabled = true
        return item
    }

    /// "Stint" over "● 6 accounts · 2 need attention".
    private static func headerTitle(status: String, dot: Dot) -> NSAttributedString {
        let title = NSMutableAttributedString()
        let regular = NSFont.systemFont(ofSize: statusSize, weight: .regular)
        title.append(styled("Stint\n", .systemFont(ofSize: titleSize, weight: .semibold), .labelColor))
        title.append(styled("● ", regular, dotColor(dot)))
        title.append(styled(status, regular, .secondaryLabelColor))
        return title
    }

    /// "Personal" in the label color, " · Max 5x" in the secondary one.
    private static func accountTitle(name: String, plan: String?) -> NSAttributedString {
        let font = NSFont.menuFont(ofSize: titleSize)
        let title = NSMutableAttributedString(attributedString: styled(name, font, .labelColor))
        if let plan {
            title.append(styled(" · \(plan)", font, .secondaryLabelColor))
        }
        return title
    }

    private static func dotColor(_ dot: Dot) -> NSColor {
        switch dot {
        case .healthy:
            // Signal Jade, a shade lighter on dark menus.
            return NSColor(name: nil) { appearance in
                let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
                return dark ? srgb(0x00, 0xB8, 0x7C) : srgb(0x00, 0xA8, 0x70)
            }
        case .attention:
            return .systemOrange
        case .idle:
            return .tertiaryLabelColor
        }
    }

    private static func styled(_ text: String, _ font: NSFont, _ color: NSColor) -> NSAttributedString {
        NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
    }

    private static func srgb(_ red: Int, _ green: Int, _ blue: Int) -> NSColor {
        NSColor(srgbRed: CGFloat(red) / 255, green: CGFloat(green) / 255, blue: CGFloat(blue) / 255, alpha: 1)
    }

    #if DEBUG
        /// Opens the menu as a click on the item would. Returns once the menu closes.
        func open() {
            statusItem.button?.performClick(nil)
        }

        /// Closes the menu if it is open.
        func close() {
            statusMenu.cancelTracking()
        }

        /// Draws the menu in `appearance` instead of the system one.
        func setAppearance(_ appearance: NSAppearance?) {
            statusMenu.appearance = appearance
        }
    #endif
}
