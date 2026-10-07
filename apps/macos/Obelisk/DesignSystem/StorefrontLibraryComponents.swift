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

/// The orders a storefront library offers, each where its store records what
/// it needs.
enum StorefrontLibrarySort: String, CaseIterable, Identifiable, Sendable {
    case lastPlayed, name, playtime

    var id: String { rawValue }

    var title: String {
        switch self {
        case .lastPlayed: L10n.string("model.library.sort.lastPlayed")
        case .name: L10n.string("model.library.sort.name")
        case .playtime: L10n.string("model.library.sort.playtime")
        }
    }

    var menuTitle: String {
        switch self {
        case .lastPlayed: L10n.string("model.library.sortMenu.lastPlayed")
        case .name: L10n.string("model.library.sortMenu.name")
        case .playtime: L10n.string("model.library.sortMenu.playtime")
        }
    }
}

/// Orders and narrows a storefront's library, the same way for every
/// storefront. Ordering every game is the costly part: each order is kept
/// until the library's revision changes, and a search or a filter only
/// narrows a kept order.
@MainActor
final class LibraryOrdering<Game: Identifiable> where Game.ID: Comparable {
    private struct Criteria: Equatable {
        var query: String
        var sort: StorefrontLibrarySort
        var filter: StorefrontLibraryFilter
        /// Read only by the installed filter.
        var installed: Set<Game.ID>
    }

    private var revision: Int?
    private var filterRevision: Int?
    private var orders: [StorefrontLibrarySort: [Game]] = [:]
    private var narrowed: (criteria: Criteria, games: [Game])?

    /// The games the model's search and filter keep, in its order.
    func games<Model: LibraryStorefront>(of model: Model) -> [Game] where Model.Game == Game {
        // Read every input even when a result is kept, so the library keeps
        // updating after a search, a filter, an installation or new metadata.
        let games = model.games
        let revision = model.libraryRevision
        let filterRevision = model.filterRevision
        let filter = model.filter
        let criteria = Criteria(
            query: model.searchText.trimmingCharacters(in: .whitespacesAndNewlines), sort: model.sort, filter: filter,
            installed: filter == .installed ? Set(model.installations.receipts.map(\.gameID)) : []
        )
        if revision != self.revision {
            self.revision = revision
            orders.removeAll()
            narrowed = nil
        }
        // What only a filter reads, such as platforms, never changes an order.
        if filterRevision != self.filterRevision {
            self.filterRevision = filterRevision
            narrowed = nil
        }
        if let narrowed, narrowed.criteria == criteria { return narrowed.games }
        let ordered = orders[criteria.sort] ?? Self.order(
            games, by: criteria.sort, title: model.title(of:), playtime: model.playtime(of:), lastPlayed: model.lastPlayed(of:)
        )
        orders[criteria.sort] = ordered
        let kept = criteria.query.isEmpty && filter == .all ? ordered : ordered.filter { game in
            guard criteria.query.isEmpty || model.title(of: game).localizedStandardContains(criteria.query) else { return false }
            switch filter {
            case .all: return true
            case .installed: return criteria.installed.contains(game.id)
            case .played: return model.playtime(of: game) != nil || model.lastPlayed(of: game) != nil
            case .macOS: return model.platform(of: game) == .macOS
            }
        }
        narrowed = (criteria, kept)
        return kept
    }

    /// `games` in `sort` order: the most played or the last played first,
    /// equal ones by name, then by ID.
    static func order(_ games: [Game], by sort: StorefrontLibrarySort, title: (Game) -> String,
                      playtime: (Game) -> Duration?, lastPlayed: (Game) -> Date?) -> [Game] {
        keyed(games, by: sort, title: title, playtime: playtime, lastPlayed: lastPlayed)
            .sorted(by: precedes)
            .map(\.game)
    }

    /// The first of `games` in `sort` order, without ordering the others.
    static func first(_ games: [Game], by sort: StorefrontLibrarySort, title: (Game) -> String,
                      playtime: (Game) -> Duration?, lastPlayed: (Game) -> Date?) -> Game? {
        keyed(games, by: sort, title: title, playtime: playtime, lastPlayed: lastPlayed)
            .min(by: precedes)?
            .game
    }

    private typealias Keyed = (game: Game, title: String, rank: Double)

    /// Each game's keys, read once rather than at every comparison.
    private static func keyed(_ games: [Game], by sort: StorefrontLibrarySort, title: (Game) -> String,
                              playtime: (Game) -> Duration?, lastPlayed: (Game) -> Date?) -> [Keyed] {
        games.map { game in
            let rank: Double = switch sort {
            case .name: 0
            case .playtime: Double(playtime(game)?.components.seconds ?? 0)
            case .lastPlayed: lastPlayed(game)?.timeIntervalSince1970 ?? -.infinity
            }
            return (game: game, title: title(game), rank: rank)
        }
    }

    private static func precedes(_ left: Keyed, _ right: Keyed) -> Bool {
        if left.rank != right.rank { return left.rank > right.rank }
        let comparison = left.title.localizedStandardCompare(right.title)
        return comparison == .orderedSame ? left.game.id < right.game.id : comparison == .orderedAscending
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
