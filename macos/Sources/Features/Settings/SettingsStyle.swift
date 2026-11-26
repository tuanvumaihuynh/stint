import SwiftUI

/// The colored square behind a sidebar or provider symbol, as in System Settings.
struct IconTile: View {
    let symbol: String
    let colors: TileColors
    var size: CGFloat = 22

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let (top, bottom) = colorScheme == .dark ? colors.dark : colors.light
        RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
            .fill(LinearGradient(colors: [top, bottom], startPoint: .top, endPoint: .bottom))
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.52, weight: .semibold))
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
    static let claude = TileColors(hex: (0xE2805F, 0xC85F3D), dark: (0xE58665, 0xCA6341))
    static let codex = TileColors(hex: (0x323742, 0x191C22), dark: (0x444956, 0x2B2F39))
    static let gemini = TileColors(hex: (0x6E8BFF, 0x4E67E6), dark: (0x7B96FF, 0x5872F0))

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

    var symbol: String {
        switch self {
        case .claude: "asterisk"
        case .codex: "hexagon"
        case .gemini: "sparkle"
        }
    }

    var tile: TileColors {
        switch self {
        case .claude: .claude
        case .codex: .codex
        case .gemini: .gemini
        }
    }

    /// Whether Stint can add accounts of this provider yet.
    var available: Bool {
        self != .gemini
    }

    static let all: [Provider] = [.claude, .codex, .gemini]
}

/// A small gray capsule, such as a plan name.
struct Badge: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 11, weight: .medium))
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
