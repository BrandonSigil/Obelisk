import ImageIO
import SwiftUI

/// Decodes downloaded artwork at display size instead of retaining full originals.
enum ImageDecoding {
    nonisolated static func thumbnail(from data: Data, maxPixelSize: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else {
            return nil
        }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ] as CFDictionary)
    }
}

enum StorefrontLibraryFilter: String, CaseIterable, Identifiable, Sendable {
    case all, installed, played, macOS

    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: L10n.string("ui.filter.all")
        case .installed: L10n.string("ui.filter.installed")
        case .played: L10n.string("ui.filter.played")
        case .macOS: L10n.string("ui.filter.macos")
        }
    }
}

/// The orders a storefront library offers, as its menu lists them, each where
/// its store records what it needs.
enum StorefrontLibrarySort: String, CaseIterable, Identifiable, Sendable {
    case name, achievements, playtime, lastPlayed, releaseDate, dateAdded, size, criticScore, playerReviews

    var id: String { rawValue }

    var title: String {
        switch self {
        case .name: L10n.string("model.library.sort.name")
        case .achievements: L10n.string("model.library.sort.achievements")
        case .playtime: L10n.string("model.library.sort.playtime")
        case .lastPlayed: L10n.string("model.library.sort.lastPlayed")
        case .releaseDate: L10n.string("model.library.sort.releaseDate")
        case .dateAdded: L10n.string("model.library.sort.dateAdded")
        case .size: L10n.string("model.library.sort.size")
        case .criticScore: L10n.string("model.library.sort.criticScore")
        case .playerReviews: L10n.string("model.library.sort.playerReviews")
        }
    }

    var menuTitle: String {
        switch self {
        case .name: L10n.string("model.library.sortMenu.name")
        case .achievements: L10n.string("model.library.sortMenu.achievements")
        case .playtime: L10n.string("model.library.sortMenu.playtime")
        case .lastPlayed: L10n.string("model.library.sortMenu.lastPlayed")
        case .releaseDate: L10n.string("model.library.sortMenu.releaseDate")
        case .dateAdded: L10n.string("model.library.sortMenu.dateAdded")
        case .size: L10n.string("model.library.sortMenu.size")
        case .criticScore: L10n.string("model.library.sortMenu.criticScore")
        case .playerReviews: L10n.string("model.library.sortMenu.playerReviews")
        }
    }
}

/// Orders and narrows a storefront's library, the same way for every
/// storefront. Ordering every game is the costly part: each order is kept
/// until the library's revision, or what only that order reads, changes, and
/// a search or a filter only narrows a kept order.
@MainActor
final class LibraryOrdering<Game: Identifiable> where Game.ID: Comparable {
    private struct Criteria: Equatable {
        var query: String
        var sort: StorefrontLibrarySort
        var filter: StorefrontLibraryFilter
        /// Each installed game's size, read only by the installed filter and
        /// the order by size.
        var installed: [Game.ID: UInt64]
        /// What the order read apart from the games.
        var inputs: Inputs
    }

    /// What an order reads apart from the games: the storefront's values for
    /// it, and for the order by size, the installed games' sizes.
    private struct Inputs: Equatable {
        var values: Int
        var sizes: [Game.ID: UInt64]
    }

    private var revision: Int?
    private var filterRevision: Int?
    private var orders: [StorefrontLibrarySort: (inputs: Inputs, games: [Game])] = [:]
    private var narrowed: (criteria: Criteria, games: [Game])?
    /// The library's hero, once read: it may have none.
    private var featured: Game??

    /// The games the model's search and filter keep, in its order.
    func games<Model: LibraryStorefront>(of model: Model) -> [Game] where Model.Game == Game {
        // Read every input even when a result is kept, so the library keeps
        // updating after a search, a filter, an installation or new metadata.
        let games = model.games
        let revision = model.libraryRevision
        let filterRevision = model.filterRevision
        let filter = model.filter
        let sort = model.sort
        let receipts = filter == .installed || sort == .size ? model.installations.receipts : []
        let installed = Dictionary(receipts.map { ($0.gameID, $0.totalBytes) }, uniquingKeysWith: +)
        // What only one order reads, such as release dates or the sizes an
        // installation changes, changes only that order.
        let inputs = Inputs(values: model.valuesRevision(for: sort), sizes: sort == .size ? installed : [:])
        let criteria = Criteria(
            query: model.searchText.trimmingCharacters(in: .whitespacesAndNewlines), sort: sort, filter: filter,
            installed: installed, inputs: inputs
        )
        keep(revision)
        // What only a filter reads, such as platforms, never changes an order.
        if filterRevision != self.filterRevision {
            self.filterRevision = filterRevision
            narrowed = nil
        }
        if let narrowed, narrowed.criteria == criteria { return narrowed.games }
        let ordered: [Game]
        if let order = orders[sort], order.inputs == inputs {
            ordered = order.games
        } else {
            ordered = Self.order(games, title: model.title(of:), rank: Self.rank(sort, of: model, sizes: inputs.sizes))
            orders[sort] = (inputs, ordered)
        }
        let kept = criteria.query.isEmpty && filter == .all ? ordered : ordered.filter { game in
            guard criteria.query.isEmpty || model.title(of: game).localizedStandardContains(criteria.query) else { return false }
            switch filter {
            case .all: return true
            case .installed: return criteria.installed[game.id] != nil
            case .played: return model.playtime(of: game) != nil || model.lastPlayed(of: game) != nil
            case .macOS: return model.platform(of: game) == .macOS
            }
        }
        narrowed = (criteria, kept)
        return kept
    }

    /// The game the model's whole library shows first: the last one played,
    /// else the first by playtime.
    func featuredGame<Model: LibraryStorefront>(of model: Model) -> Game? where Model.Game == Game {
        keep(model.libraryRevision)
        if let featured { return featured }
        let games = model.games
        let sort: StorefrontLibrarySort = games.contains { model.lastPlayed(of: $0) != nil } ? .lastPlayed : .playtime
        let game = orders[sort].map { $0.games.first } ?? Self.first(games, title: model.title(of:), rank: Self.rank(sort, of: model, sizes: [:]))
        featured = .some(game)
        return game
    }

    /// Forgets every kept result once the library's revision changed.
    private func keep(_ revision: Int) {
        guard revision != self.revision else { return }
        self.revision = revision
        orders.removeAll()
        narrowed = nil
        featured = nil
    }

    /// What places each of the model's games in `sort` order, the greatest
    /// first; a game without one goes after the others. `sizes` are the
    /// installed games'.
    private static func rank<Model: LibraryStorefront>(
        _ sort: StorefrontLibrarySort, of model: Model, sizes: [Game.ID: UInt64]
    ) -> (Game) -> Double? where Model.Game == Game {
        switch sort {
        case .name: return { _ in nil }
        case .playtime: return { model.playtime(of: $0).map { Double($0.components.seconds) } }
        case .lastPlayed: return { model.lastPlayed(of: $0)?.timeIntervalSince1970 }
        case .size: return { sizes[$0.id].map { Double($0) } }
        case .achievements, .releaseDate, .dateAdded, .criticScore, .playerReviews:
            return { model.libraryValue(of: $0, for: sort) }
        }
    }

    /// `games` by `rank`, the greatest first and a game without one last,
    /// equal ones by name, then by ID.
    static func order(_ games: [Game], title: (Game) -> String, rank: (Game) -> Double?) -> [Game] {
        keyed(games, title: title, rank: rank)
            .sorted(by: precedes)
            .map(\.game)
    }

    /// The first of `games` by `rank`, without ordering the others.
    static func first(_ games: [Game], title: (Game) -> String, rank: (Game) -> Double?) -> Game? {
        keyed(games, title: title, rank: rank)
            .min(by: precedes)?
            .game
    }

    private typealias Keyed = (game: Game, title: String, rank: Double)

    /// Each game's keys, read once rather than at every comparison.
    private static func keyed(_ games: [Game], title: (Game) -> String, rank: (Game) -> Double?) -> [Keyed] {
        games.map { game in (game: game, title: title(game), rank: rank(game) ?? -.infinity) }
    }

    private static func precedes(_ left: Keyed, _ right: Keyed) -> Bool {
        if left.rank != right.rank { return left.rank > right.rank }
        let comparison = left.title.localizedStandardCompare(right.title)
        return comparison == .orderedSame ? left.game.id < right.game.id : comparison == .orderedAscending
    }
}

/// What a storefront tells of its games apart from the library, such as
/// Steam's license dates or each game's achievements: each order's values
/// load with the library, and what the Library cache or an earlier load knew
/// shows until then. A game the storefront did not answer for has no value,
/// as one it has nothing to tell of.
@MainActor
@Observable
final class LibraryRecords<ID: Hashable & Sendable> {
    /// Each order's values by game.
    private var values: [StorefrontLibrarySort: [ID: Double]] = [:]
    /// Each order's revision, which changes whenever its values may have.
    private var revisions: [StorefrontLibrarySort: Int] = [:]

    @ObservationIgnored private var tasks: [StorefrontLibrarySort: Task<Void, Never>] = [:]

    func value(for sort: StorefrontLibrarySort, of game: ID) -> Double? { values[sort]?[game] }

    func revision(for sort: StorefrontLibrarySort) -> Int { revisions[sort, default: 0] }

    /// Shows what the Library cache knew, until a load replaces it.
    func show(_ known: [ID: Double], for sort: StorefrontLibrarySort) {
        set(known, for: sort)
    }

    /// Loads `sort`'s values through `read`, unless they are loading.
    func load(_ sort: StorefrontLibrarySort, read: @escaping @Sendable () async throws -> [ID: Double]) {
        guard tasks[sort] == nil else { return }
        tasks[sort] = Task { [weak self] in
            let loaded = try? await read()
            // UniFFI can return after the account was replaced.
            guard !Task.isCancelled, let self else { return }
            self.tasks[sort] = nil
            // A failed load changes nothing: the library's next load asks again.
            if let loaded { self.set(loaded, for: sort) }
        }
    }

    /// Forgets every value, as another account's library replaces them.
    func reset() {
        tasks.values.forEach { $0.cancel() }
        tasks.removeAll()
        for sort in values.keys { set([:], for: sort) }
    }

    private func set(_ loaded: [ID: Double], for sort: StorefrontLibrarySort) {
        guard loaded != values[sort, default: [:]] else { return }
        values[sort] = loaded
        revisions[sort, default: 0] &+= 1
    }
}

/// How every storefront presents a game's history and size.
enum GameFormatting {
    static func playtime(_ playtime: Duration) -> String {
        let seconds = max(0, playtime.components.seconds)
        if seconds == 0 { return L10n.string("ui.game.never_played") }
        if seconds < 3600 { return L10n.string("ui.game.playtime_minutes", seconds / 60) }
        let hours = Double(seconds) / 3600
        return L10n.string("ui.game.playtime_hours", hours.formatted(.number.precision(.fractionLength(0...1)).locale(L10n.locale)))
    }

    static func lastPlayed(_ date: Date?) -> String {
        guard let date else { return L10n.string("ui.game.never_launched") }
        return date.formatted(.dateTime.day().month(.abbreviated).year().locale(L10n.locale))
    }

    static func bytes(_ value: UInt64) -> String {
        Int64(clamping: value).formatted(.byteCount(style: .file).locale(L10n.locale))
    }
}

/// Dismissable notice for a Keychain or receipt storage problem.
struct StorefrontWarningBanner: View {
    @Binding var message: String?

    var body: some View {
        if let message {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                Text(L10n.message(message)).font(.system(size: 12))
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                Button { self.message = nil } label: { Image(systemName: "xmark") }
                    .obeliskButtonStyle(.plain)
                    .accessibilityLabel(L10n.string("ui.common.dismiss_warning"))
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 13)
            .modifier(StorefrontPanelGlass(cornerRadius: 16, tint: .orange.opacity(0.12)))
        }
    }
}

/// Shown when the library is empty or no game matches its search and filter.
struct StorefrontEmptyLibrary: View {
    @Binding var searchText: String
    @Binding var filter: StorefrontLibraryFilter
    let refresh: () -> Void

    init(searchText: Binding<String>, filter: Binding<StorefrontLibraryFilter> = .constant(.all), refresh: @escaping () -> Void) {
        self._searchText = searchText
        self._filter = filter
        self.refresh = refresh
    }

    private var hasCriteria: Bool {
        filter != .all || !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: hasCriteria ? "magnifyingglass" : "gamecontroller")
                .font(.system(size: 36, weight: .light)).foregroundStyle(.secondary)
            Text(L10n.string(hasCriteria ? "ui.library.no_games_found" : "ui.library.empty"))
                .font(.system(size: 20, weight: .semibold))
            Button(L10n.string(filter != .all ? "ui.filter.show_all" : hasCriteria ? "ui.search.clear" : "ui.common.refresh")) {
                if hasCriteria {
                    searchText = ""
                    filter = .all
                } else {
                    refresh()
                }
            }
            .obeliskButtonStyle(.bordered)
        }
        .frame(maxWidth: .infinity, minHeight: 260)
    }
}

/// A storefront's artwork at the location its catalog gives, filling its
/// frame, or neutral artwork without one.
struct StorefrontArtwork: View {
    let imageURL: String?
    var maxPixelSize = 768

    var body: some View {
        CachedArtwork(url: imageURL.flatMap(URL.init(string:)), maxPixelSize: maxPixelSize) { image in
            image.resizable().scaledToFill()
        } placeholder: {
            ArtworkPlaceholder()
        }
        .accessibilityHidden(true)
    }
}

/// Neutral artwork while an image loads or when a store publishes none.
struct ArtworkPlaceholder: View {
    var body: some View {
        ZStack {
            LinearGradient(colors: [Color.blue.opacity(0.18), Color.primary.opacity(0.04)], startPoint: .topLeading, endPoint: .bottomTrailing)
            Image(systemName: "gamecontroller")
                .font(.system(size: 34, weight: .light))
                .foregroundStyle(.secondary)
        }
    }
}

extension View {
    /// Presents `message` in an alert and clears it when dismissed.
    func errorAlert(_ title: String, message: Binding<String?>) -> some View {
        alert(
            title,
            isPresented: Binding(get: { message.wrappedValue != nil }, set: { if !$0 { message.wrappedValue = nil } })
        ) {
            Button(L10n.string("ui.common.ok"), role: .cancel) { message.wrappedValue = nil }
        } message: { Text(L10n.message(message.wrappedValue ?? "")) }
    }
}
