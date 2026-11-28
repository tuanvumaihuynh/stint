/// Asset catalog names for brand images. Debug builds load the "Dev" variants,
/// which carry a corner DEV ribbon so they are easy to tell apart from a release install.
enum AssetNames {
    #if DEBUG
    static let appLogo = "AppLogoDev"
    static let menuHeaderIcon = "MenuHeaderIconDev"
    static let statusIcon = "StatusIconDev"
    #else
    static let appLogo = "AppLogo"
    static let menuHeaderIcon = "MenuHeaderIcon"
    static let statusIcon = "StatusIcon"
    #endif
}
