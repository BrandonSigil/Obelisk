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
    /// Changes whenever a game, or what an order reads of one, may have.
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
    func downloadFromLibrary(_ game: Game)
    /// Whether the storefront may have a build of `game` to install.
    func canInstallFromLibrary(_ game: Game) -> Bool
}

extension LibraryStorefront {
    func syncCloud() { installations.syncCloud() }
    func canInstallFromLibrary(_ game: Game) -> Bool { true }
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

    /// Launches the game's native installation first.
    func launchFromLibrary(_ game: Game) {
        guard let receipt = installations.receipt(for: game.id, preferring: Store.nativePlatform) else { return }
        installations.launch(receipt)
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
    /// The running game whose Force Quit its player is asked to confirm.
    @State private var forceQuitting: String?
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
        let backdropOffset = heroGame == nil ? 0 : ObeliskStyle.libraryHeroHeight
        return StorefrontScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let heroGame {
                    hero(heroGame)
                }
                VStack(alignment: .leading, spacing: 22) {
                    StorefrontWarningBanner(message: $model.storageWarning)
                    if model.cloud.failurePresented {
                        SteamCloudFailureView(model: model, storefront: storeID)
                    }
                    library
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
        .safeAreaBar(edge: .bottom, spacing: 0) {
            LibraryTransferBar(installations: model.installations, name: name)
        }
        .ignoresSafeArea(.container, edges: .top)
    }

    private func hero(_ game: Model.Game) -> some View {
        LibraryHero(
            title: model.title(of: game),
            details: [
                model.lastPlayed(of: game).map { .init(symbol: "calendar", text: GameFormatting.lastPlayed($0)) },
                model.playtime(of: game).map { .init(symbol: "clock", text: GameFormatting.playtime($0)) },
            ].compactMap { $0 },
            open: { open(game, .hero) }
        ) {
            heroArtwork(game)
        } logo: {
            heroLogo(game)
        }
    }

    private var library: some View {
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
                grid(filtered)
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

    private func grid(_ games: [Model.Game]) -> some View {
        // Read once for the whole grid: these change with an operation, not
        // with its progress.
        let installations = model.installations
        let installed = Set(installations.receipts.map(\.gameID))
        let transferring = installations.activeTransferGame
        let session = installations.sessions.active
        let running = session.flatMap { $0.running ? $0.game : nil }
        let preparing = Set([
            session?.game,
            installations.isLaunching ? installations.launchGame : nil,
            installations.isUninstalling ? installations.uninstallGame : nil,
        ].compactMap { $0 })
        func action(for game: Model.Game) -> LibraryCardAction {
            if game.id == running { return .running(installations.sessions.quitStage) }
            if game.id == transferring { return .transferring }
            if preparing.contains(game.id) { return .busy }
            if installed.contains(game.id) { return .launch(enabled: installations.canLaunch(game.id)) }
            return .download(enabled: installations.canModify(game.id) && model.canInstallFromLibrary(game))
        }
        return LazyVGrid(columns: Self.columns, alignment: .leading, spacing: 18) {
            ForEach(games) { game in
                let action = action(for: game)
                let title = model.title(of: game)
                LibraryCoverCard(
                    title: title,
                    accessibilityLabel: L10n.string("ui.library.game_accessibility", title,
                                                    GameFormatting.lastPlayed(model.lastPlayed(of: game))),
                    help: [title, model.playtime(of: game).map(GameFormatting.playtime)].compactMap { $0 }
                        .joined(separator: " · "),
                    platform: model.platform(of: game),
                    action: action,
                    progress: { installations.transfer(for: game.id)?.progressFraction },
                    matchID: game.id,
                    open: { open(game, .cover) },
                    perform: {
                        switch action {
                        case .launch: model.launchFromLibrary(game)
                        case .running(.forceQuit): forceQuitting = title
                        case .running: installations.endRunningGame()
                        default: model.downloadFromLibrary(game)
                        }
                    }
                ) {
                    artwork(game)
                }
            }
        }
        .forceQuitConfirmation(forceQuitting ?? "", isPresented: Binding(
            get: { forceQuitting != nil },
            set: { if !$0 { forceQuitting = nil } }
        )) {
            installations.forceQuitRunningGame()
        }
    }

    /// The game a Transfer belongs to, also once it left the library.
    private func name(_ game: Model.Store.GameID) -> String? {
        model.games.first { $0.id == game }.map(model.title(of:))
            ?? model.installations.receipts.first { $0.gameID == game }?.name
    }
}

/// The storefront's running Transfer. It reads the Transfer in its own body,
/// so a progress tick redraws the pill, not the library.
private struct LibraryTransferBar<Store: InstallationStorefront>: View {
    let installations: GameInstallations<Store>
    let name: (Store.GameID) -> String?

    var body: some View {
        if let transfer = installations.transfer, installations.isDownloading {
            LibraryTransferPill(
                gameName: installations.transferGame.flatMap(name),
                transfer: transfer,
                cancel: installations.cancelDownload
            )
            .padding(.bottom, 18)
            .frame(maxWidth: .infinity)
        }
    }
}

extension SteamModel: LibraryStorefront {}
extension WebAccountModel: LibraryStorefront {}
