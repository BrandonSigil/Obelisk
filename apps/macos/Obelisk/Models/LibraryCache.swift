import Foundation

/// A game as the library cache keeps it.
protocol LibraryCacheEntry: Codable {
    /// The game as Obelisk names it: a Steam app ID in decimal, an Epic
    /// library ID.
    var id: String { get }
}

/// The last library each account loaded, for every storefront, in one JSON
/// file of the library: `{"version": 1, "steam": {"<account>": [games]}, ...}`.
/// A library shows from it at once, then updates from its storefront. Each
/// game also keeps `lastLaunchedAt`, when the account last launched it from
/// Obelisk. The core's own `library.json` is the catalog of games added by hand.
@MainActor
final class LibraryCache {
    /// Its name in the library directory, beside the core's catalog.
    static let fileName = "library-cache.json"

    /// The cache of the library Obelisk shows.
    static let live = LibraryCache(file: AppIdentity.libraryDirectory.appending(path: fileName))

    /// One game's last launch from Obelisk, as the file keeps it.
    struct Launch: Equatable {
        let storefront: String
        let account: String
        let game: String
        let date: Date
    }

    private static let launchKey = "lastLaunchedAt"
    private let file: URL

    init(file: URL) {
        self.file = file
    }

    /// The account's last saved library, or nil when it has none.
    func games<Entry: LibraryCacheEntry>(_ type: Entry.Type, in storefront: SessionStorefront, account: String) -> [Entry]? {
        guard let library = read()[Self.name(storefront)] as? [String: Any], let games = library[account],
              let data = try? JSONSerialization.data(withJSONObject: games) else { return nil }
        return try? JSONDecoder().decode([Entry].self, from: data)
    }

    /// Replaces the account's library. Each game keeps its last launch.
    func save<Entry: LibraryCacheEntry>(_ games: [Entry], in storefront: SessionStorefront, account: String) {
        guard let data = try? JSONEncoder().encode(games),
              var entries = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
        var document = read()
        let launches = Dictionary(
            Self.entries(in: document, storefront, account: account).compactMap { entry in
                (entry["id"] as? String).flatMap { id in entry[Self.launchKey].map { (id, $0) } }
            },
            uniquingKeysWith: { first, _ in first }
        )
        for index in entries.indices {
            if let id = entries[index]["id"] as? String, let launch = launches[id] {
                entries[index][Self.launchKey] = launch
            }
        }
        Self.set(entries, in: &document, storefront, account: account)
        write(document)
    }

    /// Records that the account launched `game` from Obelisk, in the game's
    /// entry. A game whose library was never saved has no entry to keep it.
    func recordLaunch(of game: String, in storefront: SessionStorefront, account: String, at date: Date) {
        var document = read()
        var entries = Self.entries(in: document, storefront, account: account)
        guard let index = entries.firstIndex(where: { $0["id"] as? String == game }) else { return }
        entries[index][Self.launchKey] = ISO8601DateFormatter().string(from: date)
        Self.set(entries, in: &document, storefront, account: account)
        write(document)
    }

    /// Every game's last launch, for every storefront and account.
    func launches() -> [Launch] {
        let formatter = ISO8601DateFormatter()
        return read().flatMap { storefront, library -> [Launch] in
            guard let library = library as? [String: Any] else { return [] }
            return library.flatMap { account, games -> [Launch] in
                (games as? [[String: Any]] ?? []).compactMap { entry in
                    guard let game = entry["id"] as? String, let launch = entry[Self.launchKey] as? String,
                          let date = formatter.date(from: launch) else { return nil }
                    return Launch(storefront: storefront, account: account, game: game, date: date)
                }
            }
        }
    }

    /// Removes the account's library and its launches, as signing it out does.
    func forget(_ storefront: SessionStorefront, account: String) {
        var document = read()
        Self.set(nil, in: &document, storefront, account: account)
        write(document)
    }

    /// The file's libraries. An unreadable file has none, and the next save
    /// replaces it: the cache only spares the wait for a storefront.
    private func read() -> [String: Any] {
        guard let data = try? Data(contentsOf: file),
              let document = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              document["version"] as? Int == 1 else { return [:] }
        return document
    }

    /// Every change reads the file first, so one storefront's update keeps
    /// every other library.
    private func write(_ document: [String: Any]) {
        var document = document
        document["version"] = 1
        guard let data = try? JSONSerialization.data(withJSONObject: document, options: [.prettyPrinted, .sortedKeys])
        else { return }
        try? FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: file, options: .atomic)
    }

    private static func entries(in document: [String: Any], _ storefront: SessionStorefront,
                                account: String) -> [[String: Any]] {
        (document[name(storefront)] as? [String: Any])?[account] as? [[String: Any]] ?? []
    }

    private static func set(_ entries: [[String: Any]]?, in document: inout [String: Any],
                            _ storefront: SessionStorefront, account: String) {
        var library = document[name(storefront)] as? [String: Any] ?? [:]
        library[account] = entries
        document[name(storefront)] = library
    }

    /// How the file names a storefront.
    static func name(_ storefront: SessionStorefront) -> String { "\(storefront)" }
}
