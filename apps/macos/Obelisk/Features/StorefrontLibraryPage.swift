import SwiftUI

/// What a storefront model gives its library page.
@MainActor protocol LibraryStorefront: GameDetailModel, Observable {
    associatedtype Game: Identifiable where Game.ID == Store.GameID
    var games: [Game] { get }
    /// The games the search and the filter keep, in the library's order.
    var filteredGames: [Game] { get }
    var searchText: String { get set }
    var filter: StorefrontLibraryFilter { get set }
    var availableFilters: [StorefrontLibraryFilter] { get }
    var sort: StorefrontLibrarySort { get set }
    /// The orders the storefront records enough to offer.
    var availableSorts: [StorefrontLibrarySort] { get }
    var isLoadingGames: Bool { get }
    var storageWarning: String? { get set }
    var cloud: CloudModel<Store> { get }
    /// The game a library download waits on while the storefront finds which
    /// build the account can install.
    var preparingDownload: Store.GameID? { get }
    /// What the filter or the order still waits for, such as games'
    /// platforms or playtimes, while it loads.
    var pendingMetadata: String? { get }
    /// What the filter or the order could not load, offered a retry.
    var missingMetadata: String? { get }
    func refresh()
    func open(_ game: Game)
    func cloudTitle(for game: Store.GameID?) -> String
    func launchFromLibrary(_ game: Game)
    func downloadFromLibrary(_ game: Game)
    /// Whether the storefront may have a build of `game` to install.
    func canInstallFromLibrary(_ game: Game) -> Bool
    /// The game the whole library shows first, as its hero.
    var featuredGame: Game? { get }
}

extension LibraryStorefront {
    var preparingDownload: Store.GameID? { nil }
    func canInstallFromLibrary(_ game: Game) -> Bool { true }
}

/// How a library card presents its game.
struct LibraryCardContent {
    let title: String
    var accessibilityLabel: String?
    var help: String?
    var platform: GamePlatform?
}

/// Where a library was scrolled, kept while a game's page replaces it. Nothing
/// observes it, so scrolling redraws nothing.
@MainActor
final class LibraryScrollMemory {
    var offset: CGFloat = 0
}

/// A storefront's signed-in library: an optional hero, then every game of the
/// account under a floating glass bar, and the storefront's Transfer floating
/// at the bottom.
struct StorefrontLibraryPage<Model: LibraryStorefront, Hero: View, Artwork: View, AccountMenu: View>: View {
    @Bindable var model: Model
    /// Names the storefront in accessibility identifiers.
    let storeID: String
    let storeTitle: String
    let libraryTitle: String
    let accountName: String?
    let avatarURL: URL?
    let scrollMemory: LibraryScrollMemory
    /// The featured game at the top, its artwork reaching under the bar.
    @ViewBuilder let hero: (Model.Game) -> Hero
    let card: (Model.Game) -> LibraryCardContent
    @ViewBuilder let artwork: (Model.Game) -> Artwork
    @ViewBuilder let accountMenu: AccountMenu
    @State private var position: ScrollPosition
    /// Whether games or the library's header are under the bar, rather than
    /// the hero's artwork or nothing at all.
    @State private var contentUnderBar = false

    private static var columns: [GridItem] {
        [GridItem(.adaptive(minimum: 200, maximum: 270), spacing: 16, alignment: .top)]
    }

    init(
        model: Model,
        storeID: String,
        storeTitle: String,
        libraryTitle: String,
        accountName: String?,
        avatarURL: URL? = nil,
        scrollMemory: LibraryScrollMemory,
        @ViewBuilder hero: @escaping (Model.Game) -> Hero,
        card: @escaping (Model.Game) -> LibraryCardContent,
        @ViewBuilder artwork: @escaping (Model.Game) -> Artwork,
        @ViewBuilder accountMenu: () -> AccountMenu
    ) {
        self.model = model
        self.storeID = storeID
        self.storeTitle = storeTitle
        self.libraryTitle = libraryTitle
        self.accountName = accountName
        self.avatarURL = avatarURL
        self.scrollMemory = scrollMemory
        self.hero = hero
        self.card = card
        self.artwork = artwork
        self.accountMenu = accountMenu()
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
        return StorefrontScrollView(topOverlayHeight: ObeliskStyle.titleBarHeight, scrollerOverContent: true) {
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
                .frame(maxWidth: 1400, alignment: .leading)
            }
        }
        .scrollPosition($position)
        .onScrollGeometryChange(for: CGFloat.self, of: { $0.contentOffset.y + $0.contentInsets.top }) { _, offset in
            scrollMemory.offset = offset
        }
        .onScrollGeometryChange(for: Bool.self, of: { $0.contentOffset.y + $0.contentInsets.top > backdropOffset }) { _, under in
            contentUnderBar = under
        }
        // An overlay rather than a safe area bar: on macOS the bar's scroll
        // edge effect cannot be turned off, and it veils the hero's artwork.
        .overlay(alignment: .top) {
            LibraryGlassBar(
                storeTitle: storeTitle,
                accountName: accountName,
                avatarURL: avatarURL,
                searchText: $model.searchText,
                showsBackdrop: contentUnderBar
            ) {
                accountMenu
            }
        }
        .safeAreaBar(edge: .bottom, spacing: 0) {
            LibraryTransferBar(installations: model.installations, name: name)
        }
        .ignoresSafeArea(.container, edges: .top)
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
        let preparingDownload = model.preparingDownload
        let preparing = Set([
            session?.game,
            installations.isLaunching ? installations.launchGame : nil,
            installations.isUninstalling ? installations.uninstallGame : nil,
            preparingDownload,
        ].compactMap { $0 })
        func action(for game: Model.Game) -> LibraryCardAction {
            if game.id == running { return .running }
            if game.id == transferring { return .transferring }
            if preparing.contains(game.id) { return .busy }
            if installed.contains(game.id) { return .launch(enabled: installations.canLaunch(game.id)) }
            return .download(enabled: preparingDownload == nil && installations.canModify(game.id)
                && model.canInstallFromLibrary(game))
        }
        return LazyVGrid(columns: Self.columns, alignment: .leading, spacing: 18) {
            ForEach(games) { game in
                let action = action(for: game)
                let content = card(game)
                LibraryCoverCard(
                    title: content.title,
                    accessibilityLabel: content.accessibilityLabel ?? content.title,
                    help: content.help,
                    platform: content.platform,
                    action: action,
                    progress: { installations.transfer(for: game.id)?.progressFraction },
                    open: { model.open(game) },
                    perform: {
                        if case .launch = action { model.launchFromLibrary(game) }
                        else { model.downloadFromLibrary(game) }
                    }
                ) {
                    artwork(game)
                }
            }
        }
    }

    /// The game a Transfer belongs to, also once it left the library.
    private func name(_ game: Model.Store.GameID) -> String? {
        model.games.first { $0.id == game }.map { card($0).title }
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
extension EpicModel: LibraryStorefront {}
