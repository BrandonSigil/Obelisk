import SwiftUI

/// A game of any of the player's storefronts, as Home shows it beside the
/// other storefronts' games: what orders and names it, what its cover and hero
/// show, and what its buttons do.
@MainActor
struct HomeGame: Identifiable {
    /// The game within its storefront's library.
    struct ID: Hashable, Sendable {
        let section: LibrarySection
        let game: String
    }

    /// A storefront game's wide artwork and what names the game over it.
    struct Hero {
        let artwork: () -> AnyView
        let logo: () -> AnyView
    }

    let id: ID
    let title: String
    let lastPlayed: Date?
    let playtime: Duration?
    let platform: GamePlatform?
    let isInstalled: Bool
    let action: LibraryCardAction
    let cover: () -> AnyView
    let hero: Hero
    /// The game's Transfer, read only by the views that show its progress.
    let transfer: @MainActor () -> GameTransfer?
    let cancelTransfer: () -> Void
    /// Opens the game's page in its storefront.
    let open: () -> Void
    /// Runs the card's button. Force Quit runs at once: Home asks first.
    let perform: () -> Void

    /// The last played first, then the never played, each by name.
    static func lastPlayedFirst(_ left: HomeGame, _ right: HomeGame) -> Bool {
        switch (left.lastPlayed, right.lastPlayed) {
        case let (left?, right?) where left != right: return left > right
        case (.some, nil): return true
        case (nil, .some): return false
        default: return byName(left, right)
        }
    }

    static func byName(_ left: HomeGame, _ right: HomeGame) -> Bool {
        left.title.localizedStandardCompare(right.title) == .orderedAscending
    }
}

extension LibraryStorefront {
    /// The account's games as Home shows them, in the storefront's own artwork.
    func homeGames<Cover: View, Artwork: View, Logo: View>(
        in section: LibrarySection,
        cover: @escaping (Game) -> Cover,
        heroArtwork: @escaping (Game) -> Artwork,
        heroLogo: @escaping (Game) -> Logo
    ) -> [HomeGame] {
        let actions = LibraryCardActions(self)
        let installations = installations
        return games.map { game in
            let action = actions(game)
            return HomeGame(
                id: HomeGame.ID(section: section, game: "\(game.id)"),
                title: title(of: game),
                lastPlayed: lastPlayed(of: game),
                playtime: playtime(of: game),
                platform: platform(of: game),
                isInstalled: actions.isInstalled(game),
                action: action,
                cover: { AnyView(cover(game)) },
                hero: HomeGame.Hero(artwork: { AnyView(heroArtwork(game)) }, logo: { AnyView(heroLogo(game)) }),
                transfer: { installations.transfer(for: game.id) },
                cancelTransfer: installations.cancelDownload,
                open: { self.open(game) },
                perform: {
                    self.performCardAction(action, for: game, confirmForceQuit: installations.forceQuitRunningGame)
                }
            )
        }
    }
}

/// What Home shows of every library at once.
@MainActor
struct HomeShelves {
    /// The game Home shows first: the last one played anywhere, else the most
    /// played, as a library's hero.
    let hero: HomeGame?
    /// The games played before the hero's, the last played first.
    let recent: [HomeGame]
    /// Every game on the Mac, the last played first.
    let installed: [HomeGame]

    static let shelfLimit = 20

    init(_ games: [HomeGame]) {
        let played = games.filter { $0.lastPlayed != nil }.sorted(by: HomeGame.lastPlayedFirst)
        let hero = played.first ?? games.min { left, right in
            let (leftPlaytime, rightPlaytime) = (left.playtime ?? .zero, right.playtime ?? .zero)
            return leftPlaytime == rightPlaytime ? HomeGame.byName(left, right) : leftPlaytime > rightPlaytime
        }
        self.hero = hero
        recent = Array(played.filter { $0.id != hero?.id }.prefix(Self.shelfLimit))
        installed = games.filter(\.isInstalled).sorted(by: HomeGame.lastPlayedFirst)
    }
}
