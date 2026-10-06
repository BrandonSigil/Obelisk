import Foundation

enum LibrarySection: String, CaseIterable, Identifiable, Sendable {
    case home, steam, epic, gog, other, settings

    var id: String { rawValue }

    var title: String {
        switch self {
        case .home: L10n.string("model.section.home.title")
        case .steam: L10n.string("model.section.steam.title")
        case .epic: L10n.string("model.section.epic.title")
        case .gog: L10n.string("model.section.gog.title")
        case .other: L10n.string("model.section.other.title")
        case .settings: L10n.string("model.section.settings.title")
        }
    }

    var symbol: String {
        switch self {
        case .home: "house"
        case .steam: "gearshape.2"
        case .epic: "shield.lefthalf.filled"
        case .gog: "square.stack.3d.up"
        case .other: "square.grid.2x2"
        case .settings: "gearshape"
        }
    }

    var logoAssetName: String? {
        switch self {
        case .steam: "SteamLogo"
        case .epic: "EpicGamesLogo"
        case .gog: "GOGLogo"
        default: nil
        }
    }

    var subtitle: String {
        switch self {
        case .home: L10n.string("model.section.home.subtitle")
        case .steam: L10n.string("model.section.steam.subtitle")
        case .epic: L10n.string("model.section.epic.subtitle")
        case .gog: L10n.string("model.section.gog.subtitle")
        case .other: L10n.string("model.section.other.subtitle")
        case .settings: L10n.string("model.section.settings.subtitle", AppIdentity.name)
        }
    }

    var isStorefront: Bool { self == .steam || self == .epic || self == .gog }

    /// Store tabs in sidebar order; the single source for every storefront list.
    static let storefronts: [LibrarySection] = allCases.filter(\.isStorefront)
}
