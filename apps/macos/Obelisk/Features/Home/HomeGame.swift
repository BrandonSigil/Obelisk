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
    let isInstalled: Bool
    /// What the game's card and hero buttons show and do.
    let action: GameAction
    let cover: () -> AnyView
    let hero: Hero
    /// Opens the game's page in its storefront.
    let open: () -> Void

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
        let actions = GameActions(installations, in: .library)
        return games.map { game in
            HomeGame(
                id: HomeGame.ID(section: section, game: "\(game.id)"),
                title: title(of: game),
                lastPlayed: lastPlayed(of: game),
                playtime: playtime(of: game),
                isInstalled: actions.isInstalled(game.id),
                action: actions(librarySelection(game)),
                cover: { AnyView(cover(game)) },
                hero: HomeGame.Hero(artwork: { AnyView(heroArtwork(game)) }, logo: { AnyView(heroLogo(game)) }),
                open: { self.open(game) }
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
