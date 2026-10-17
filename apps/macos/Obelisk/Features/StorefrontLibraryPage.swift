import SwiftUI

/// What a storefront model gives its library and its game page.
@MainActor protocol LibraryStorefront: AnyObject, Observable {
    associatedtype Store: InstallationStorefront
    associatedtype Game: Identifiable where Game.ID == Store.GameID, Game.ID: Comparable
    var installations: GameInstallations<Store> { get }
    var achievements: GameAchievementsModel { get }
    var gameSessionMessage: String? { get }
    func syncCloud()
    func reloadAchievements()
    var games: [Game] { get }
    func title(of game: Game) -> String
    /// How long the account played the game; nil until the storefront told
    /// it, or when it was never played.
    func playtime(of game: Game) -> Duration?
    /// When the account last played the game: on the storefront, or later
    /// from Obelisk.
    func lastPlayed(of game: Game) -> Date?
    /// The build Obelisk installs or launches first, once the storefront told
    /// the game's platforms.
    func platform(of game: Game) -> GamePlatform?
    /// What the storefront told of the game that `sort` orders it by, the
    /// greatest first, such as its release date or its critics' score; nil
    /// when it told nothing. Never asked for the orders every storefront
    /// shares: by name, playtime, last played and size.
    func libraryValue(of game: Game, for sort: StorefrontLibrarySort) -> Double?
    /// Changes whenever what the storefront told of the games apart from
    /// them that `sort` orders by may have, such as their release dates: only
    /// that order changes.
    func valuesRevision(for sort: StorefrontLibrarySort) -> Int
    /// Changes whenever a game, or what every order reads of one, may have.
    var libraryRevision: Int { get }
    /// Changes whenever what only a filter reads of the games may have, such
    /// as their platforms.
    var filterRevision: Int { get }
    /// Keeps the library's orders between two reads.
    var libraryOrdering: LibraryOrdering<Game> { get }
    var searchText: String { get set }
    var filter: StorefrontLibraryFilter { get set }
    var availableFilters: [StorefrontLibraryFilter] { get }
    var sort: StorefrontLibrarySort { get set }
    /// The orders the storefront records enough to offer.
    var availableSorts: [StorefrontLibrarySort] { get }
    var isLoadingGames: Bool { get }
    var storageWarning: String? { get set }
    var cloud: CloudModel<Store> { get }
    /// What the filter or the order still waits for, such as games'
    /// platforms or playtimes, while it loads.
    var pendingMetadata: String? { get }
    /// What the filter or the order could not load, offered a retry.
    var missingMetadata: String? { get }
    func refresh()
    /// The game whose page shows instead of the library.
    var selectedGame: Game? { get }
    func open(_ game: Game)
    /// Returns from the game's page to the library.
    func closeGame()
    /// What the library installs of `game` without opening its page, when
    /// the storefront has a build of it.
    func libraryOffer(of game: Game) -> GameOffer?
    /// The messages the account has not read, which its menu tells.
    var unreadCount: Int { get }
}

extension LibraryStorefront {
    func syncCloud() { installations.syncCloud() }
    var unreadCount: Int { 0 }
    var filterRevision: Int { 0 }

    /// The games the search and the filter keep, in the library's order.
    var filteredGames: [Game] { libraryOrdering.games(of: self) }

    /// The game the whole library shows first, as its hero, so a library
    /// always has one: the last one played, else the first of the library
    /// ordered by playtime, the most played first and equal ones by name.
    var featuredGame: Game? { libraryOrdering.featuredGame(of: self) }

    /// Names the game a Cloud question or banner is about.
    func cloudTitle(for game: Store.GameID?) -> String {
        guard let game else { return Store.cloudTitle }
        let name = games.first { $0.id == game }.map(title(of:))
            ?? installations.receipts.first { $0.gameID == game }?.name
            ?? Store.name(game)
        return "\(Store.cloudTitle) · \(name)"
    }

    /// A game of the library, as its Game action reads it: its native build
    /// first, and what the library installs without opening its page.
    func librarySelection(_ game: Game) -> InstallationSelection<Store> {
        InstallationSelection(game: game.id, platform: Store.nativePlatform, offer: libraryOffer(of: game))
    }
}

/// Where a library was scrolled, kept while a game's page replaces it. Nothing
/// observes it, so scrolling redraws nothing.
@MainActor
final class LibraryScrollMemory {
    var offset: CGFloat = 0
}

/// A storefront's signed-in library: the featured game as its hero, then every
/// game of the account under the browser's floating glass bar, and the
/// storefront's Transfer floating at the bottom.
struct StorefrontLibraryPage<Model: LibraryStorefront, HeroArtwork: View, HeroLogo: View, Artwork: View>: View {
    @Bindable var model: Model
    /// Names the storefront in accessibility identifiers.
    let storeID: String
    let storeTitle: String
    let libraryTitle: String
    let scrollMemory: LibraryScrollMemory
    /// Opens a game's page, from its cover or from the hero.
    let open: (Model.Game, GameOpeningSource) -> Void
    /// The featured game's wide artwork, reaching under the bar.
    @ViewBuilder let heroArtwork: (Model.Game) -> HeroArtwork
    @ViewBuilder let heroLogo: (Model.Game) -> HeroLogo
    /// A game's cover.
    @ViewBuilder let artwork: (Model.Game) -> Artwork
    @State private var position: ScrollPosition
    @Environment(\.storefrontBar) private var bar

    private static var columns: [GridItem] {
        [GridItem(.adaptive(minimum: 160, maximum: 220), spacing: 16, alignment: .top)]
    }

    init(
        model: Model,
        storeID: String,
        storeTitle: String,
        libraryTitle: String,
        scrollMemory: LibraryScrollMemory,
        open: @escaping (Model.Game, GameOpeningSource) -> Void,
        @ViewBuilder heroArtwork: @escaping (Model.Game) -> HeroArtwork,
        @ViewBuilder heroLogo: @escaping (Model.Game) -> HeroLogo,
        @ViewBuilder artwork: @escaping (Model.Game) -> Artwork
    ) {
        self.model = model
        self.storeID = storeID
        self.storeTitle = storeTitle
        self.libraryTitle = libraryTitle
        self.scrollMemory = scrollMemory
        self.open = open
        self.heroArtwork = heroArtwork
        self.heroLogo = heroLogo
        self.artwork = artwork
        // A library opened at its top stays free to grow above, as its hero
        // does once the games arrive.
        let offset = scrollMemory.offset
        _position = State(initialValue: offset > 0 ? ScrollPosition(y: offset) : ScrollPosition())
    }

    /// The hero belongs to the whole library, never to a search or a filter.
    private var heroGame: Model.Game? {
        guard model.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, model.filter == .all else {
            return nil
        }
        return model.featuredGame
    }

    var body: some View {
        let heroGame = heroGame
        let actions = GameActions(model.installations, in: .library)
        let backdropOffset = heroGame == nil ? 0 : ObeliskStyle.libraryHeroHeight
        return StorefrontScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let heroGame {
                    hero(heroGame, action: actions(model.librarySelection(heroGame)))
                }
                VStack(alignment: .leading, spacing: 22) {
                    StorefrontWarningBanner(message: $model.storageWarning)
                    if model.cloud.failurePresented {
                        SteamCloudFailureView(model: model, storefront: storeID)
                    }
                    library(actions)
                }
                .padding(.horizontal, ObeliskStyle.contentPadding)
                .padding(.top, heroGame == nil ? 12 : 30)
                .padding(.bottom, 36)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .scrollPosition($position)
        .onScrollGeometryChange(for: CGFloat.self, of: { $0.contentOffset.y + $0.contentInsets.top }) { _, offset in
            scrollMemory.offset = offset
        }
        // Whether games or the library's header are under the bar, rather
        // than the hero's artwork or nothing at all.
        .onScrollGeometryChange(for: Bool.self, of: { $0.contentOffset.y + $0.contentInsets.top > backdropOffset }) { _, under in
            bar?.showsBackdrop = under
            bar?.showsGameTitle = false
        }
        .ignoresSafeArea(.container, edges: .top)
    }

    private func hero(_ game: Model.Game, action: GameAction) -> some View {
        LibraryHero(
            title: model.title(of: game),
            details: [
                model.lastPlayed(of: game).map { .init(symbol: "calendar", text: GameFormatting.lastPlayed($0)) },
                model.playtime(of: game).map { .init(symbol: "clock", text: GameFormatting.playtime($0)) },
            ].compactMap { $0 },
            action: action,
            open: { open(game, .hero) }
        ) {
            heroArtwork(game)
        } logo: {
            heroLogo(game)
        }
    }

    private func library(_ actions: GameActions<Model.Store>) -> some View {
        let filtered = model.filteredGames
        return VStack(alignment: .leading, spacing: 22) {
            header(count: filtered.count)

            if let missing = model.missingMetadata {
                HStack(spacing: 12) {
                    Text(missing)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button(L10n.string("ui.common.retry"), action: model.refresh)
                        .obeliskButtonStyle(.glass)
                        .controlSize(.small)
                        .disabled(model.isLoadingGames || model.pendingMetadata != nil)
                }
            }

            if model.isLoadingGames && model.games.isEmpty {
                ProgressView(L10n.string("ui.library.fetching")).frame(maxWidth: .infinity, minHeight: 260)
            } else if filtered.isEmpty, let pending = model.pendingMetadata {
                ProgressView(pending).frame(maxWidth: .infinity, minHeight: 260)
            } else if filtered.isEmpty {
                StorefrontEmptyLibrary(searchText: $model.searchText, filter: $model.filter, refresh: model.refresh)
            } else {
                grid(filtered, actions: actions)
                if model.isLoadingGames || model.pendingMetadata != nil {
                    ProgressView().controlSize(.small).frame(maxWidth: .infinity)
                }
            }
        }
    }

    private func header(count: Int) -> some View {
        let title = VStack(alignment: .leading, spacing: 3) {
            Text(libraryTitle)
                .font(.system(size: 22, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            if !model.games.isEmpty {
                Text(L10n.string("ui.library.game_count", Int64(count)))
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
        .fixedSize()
        let filters = LibraryFilterPicker(storeTitle: storeTitle, filters: model.availableFilters, selection: $model.filter)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: 14) {
                title
                Spacer(minLength: 16)
                filters
                sortButton
            }
            VStack(alignment: .leading, spacing: 16) {
                HStack(spacing: 14) {
                    title
                    Spacer(minLength: 16)
                    sortButton
                }
                filters
            }
        }
    }

    /// Shown when the storefront records enough for more than one order.
    @ViewBuilder private var sortButton: some View {
        if model.availableSorts.count > 1 {
            LibrarySortButton(
                storeTitle: storeTitle,
                sortTitle: model.sort.title,
                options: model.availableSorts.map { sort in
                    StorefrontSortOption(id: sort.id, title: sort.menuTitle, isSelected: model.sort == sort) {
                        model.sort = sort
                    }
                }
            )
        }
    }

    private func grid(_ games: [Model.Game], actions: GameActions<Model.Store>) -> some View {
        LazyVGrid(columns: Self.columns, alignment: .leading, spacing: 18) {
            ForEach(games) { game in
                let title = model.title(of: game)
                LibraryCoverCard(
                    title: title,
                    accessibilityLabel: L10n.string("ui.library.game_accessibility", title,
                                                    GameFormatting.lastPlayed(model.lastPlayed(of: game))),
                    help: [title, model.playtime(of: game).map(GameFormatting.playtime)].compactMap { $0 }
                        .joined(separator: " · "),
                    platform: model.platform(of: game),
                    action: actions(model.librarySelection(game)),
                    matchID: game.id,
                    open: { open(game, .cover) }
                ) {
                    artwork(game)
                }
            }
        }
    }
}

extension SteamModel: LibraryStorefront {}
extension WebAccountModel: LibraryStorefront {}
