import Foundation
import Observation

/// When each account last launched each of its games from Obelisk, for every
/// storefront. The library cache keeps each launch in its game's entry; a
/// storefront that records no last played date, as Epic, learns it here.
@MainActor @Observable
final class LaunchHistory {
    /// Each launch by storefront, account and game.
    private var launches: [String: Date]
    @ObservationIgnored private let cache: LibraryCache

    /// The history of the library Obelisk shows.
    static let live = LaunchHistory(cache: .live)

    init(cache: LibraryCache) {
        self.cache = cache
        launches = Dictionary(cache.launches().map { (Self.key($0.storefront, $0.account, $0.game), $0.date) },
                              uniquingKeysWith: max)
    }

    func record<Store: GameStorefront>(_ game: Store.GameID, account: Store.AccountID, in store: Store.Type,
                                        at date: Date = .now) {
        let storefront = LibraryCache.name(Store.storefront)
        launches[Self.key(storefront, Self.name(account), Self.name(game))] = date
        cache.recordLaunch(of: Self.name(game), in: Store.storefront, account: Self.name(account), at: date)
    }

    /// When `account` last launched `game` from Obelisk.
    func lastLaunch<Store: GameStorefront>(of game: Store.GameID, account: Store.AccountID,
                                            in store: Store.Type) -> Date? {
        launches[Self.key(LibraryCache.name(Store.storefront), Self.name(account), Self.name(game))]
    }

    /// Drops the account's launches, as signing it out drops its library.
    func forget<Store: GameStorefront>(account: Store.AccountID, in store: Store.Type) {
        let prefix = Self.key(LibraryCache.name(Store.storefront), Self.name(account), "")
        launches = launches.filter { !$0.key.hasPrefix(prefix) }
    }

    /// How the cache names an account or a game: a Steam app ID or account ID
    /// in decimal, an Epic library ID as it is.
    private static func name(_ value: some Hashable) -> String { "\(value)" }

    private static func key(_ storefront: String, _ account: String, _ game: String) -> String {
        [storefront, account, game].joined(separator: "\n")
    }
}
