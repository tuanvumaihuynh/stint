import SwiftUI

/// The colored square behind a sidebar or provider symbol, as in System Settings.
struct IconTile: View {
    let symbol: String
    let colors: TileColors
    var size: CGFloat = 20

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let (top, bottom) = colorScheme == .dark ? colors.dark : colors.light
        RoundedRectangle(cornerRadius: size * 0.25, style: .continuous)
            .fill(LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom))
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.55, weight: .medium))
                    .foregroundStyle(.white)
            }
    }
}

/// A tile's top and bottom colors in light and dark mode.
struct TileColors {
    let light: (Color, Color)
    let dark: (Color, Color)

    static let general = TileColors(hex: (0x9C9CA4, 0x787880), dark: (0x8F8F98, 0x696972))
    static let accounts = TileColors(hex: (0x7472EE, 0x5654D6), dark: (0x7D7BF6, 0x5E5CE6))
    static let menuBar = TileColors(hex: (0x3EC4F6, 0x1296D2), dark: (0x52CEF9, 0x1FA2DE))

    private init(hex light: (Int, Int), dark: (Int, Int)) {
        self.light = (Color(hex: light.0), Color(hex: light.1))
        self.dark = (Color(hex: dark.0), Color(hex: dark.1))
    }
}

extension Color {
    init(hex: Int) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

extension Provider {
    var title: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex"
        case .gemini: "Gemini"
        }
    }

    /// What kind of subscription the provider sells, for choosing one.
    var subscription: String {
        switch self {
        case .claude: "Anthropic subscription (Pro, Max, Team)"
        case .codex: "ChatGPT subscription (Plus, Pro, Team)"
        case .gemini: "Google AI subscription"
        }
    }

    /// The provider's mark in the asset catalog.
    var logo: String {
        switch self {
        case .claude: "ProviderClaude"
        case .codex: "ProviderCodex"
        case .gemini: "ProviderGemini"
        }
    }

    /// Whether Stint can add accounts of this provider yet.
    var available: Bool {
        self != .gemini
    }

    static let all: [Provider] = [.claude, .codex, .gemini]
}

/// A provider's own mark on a rounded tile, as its app icon shows it.
struct ProviderLogo: View {
    let provider: Provider
    var size: CGFloat = 28

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * 0.24, style: .continuous)
        ZStack {
            switch provider {
            case .claude:
                shape.fill(Color(hex: 0xD97757))
                mark.foregroundStyle(.white)
            case .codex:
                shape.fill(Color(hex: 0x0D0D0D))
                mark.foregroundStyle(.white)
            case .gemini:
                shape.fill(.white)
                shape.strokeBorder(.black.opacity(0.1), lineWidth: 0.5)
                mark.foregroundStyle(
                    LinearGradient(
                        colors: [Color(hex: 0x4796E3), Color(hex: 0x9177C7), Color(hex: 0xCA6673)],
                        startPoint: .bottomLeading,
                        endPoint: .topTrailing
                    )
                )
            }
        }
        .frame(width: size, height: size)
    }

    private var mark: some View {
        Image(provider.logo)
            .resizable()
            .scaledToFit()
            .padding(size * 0.2)
    }
}

/// A small gray capsule, such as a plan name.
struct Badge: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.subheadline.weight(.medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6)
            .padding(.vertical, 1.5)
            .background(.quaternary.opacity(0.7), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
    }
}

extension Severity {
    var color: Color {
        switch self {
        case .error: .red
        case .warning: .orange
        }
    }
}

// Rows are told apart by their account's id.
extension AccountRow: Identifiable {}

extension Color {
    // Tones of the window, light and dark: a white sidebar over a light gray pane, as in
    // other native settings windows.
    static let sidebarBackground = adaptive(light: 1, dark: 0.16)
    static let sidebarSelection = adaptive(light: 0.96, dark: 0.24)
    static let paneBackground = adaptive(light: 0.98, dark: 0.12)

    private static func adaptive(light: CGFloat, dark: CGFloat) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(white: isDark ? dark : light, alpha: 1)
        })
    }
}
