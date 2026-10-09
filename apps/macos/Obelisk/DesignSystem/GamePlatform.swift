import Foundation

enum GamePlatform {
    case macOS, windows

    static func preferred(macos: Bool, windows: Bool) -> Self? {
        if macos { return .macOS }
        if windows { return .windows }
        return nil
    }

    static func preferred(platforms: [String]) -> Self? {
        preferred(macos: platforms.contains("macos"), windows: platforms.contains("windows"))
    }

    var title: String {
        switch self {
        case .macOS: L10n.string("model.platform.macos")
        case .windows: L10n.string("model.platform.windows")
        }
    }

    var logoAssetName: String {
        switch self {
        case .macOS: "AppleLogo"
        case .windows: "WindowsLogo"
        }
    }
}
