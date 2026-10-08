import SwiftUI

/// What a storefront model gives its game page.
@MainActor protocol GameDetailModel: AnyObject {
    associatedtype Store: InstallationStorefront
    var installations: GameInstallations<Store> { get }
    var achievements: GameAchievementsModel { get }
    var gameSessionMessage: String? { get }
    func syncCloud()
    func reloadAchievements()
}

extension GameDetailModel {
    func syncCloud() { installations.syncCloud() }
}

extension SteamModel: GameDetailModel {}
extension WebAccountModel: GameDetailModel {}

/// A storefront's game page: the game's wide artwork reaching under the
/// browser's glass bar, its cover overlapping the artwork's lower edge, its
/// launch row and metrics beside the cover, then what only the storefront shows.
/// Opened from the library, the page reveals itself around the cover flying
/// in: the artwork settles, then the header and the content rise.
struct GameDetailPage<Model: LibraryStorefront, HeroArtwork: View, Cover: View, Content: View>: View {
    @Bindable var model: Model
    let game: Model.Game
    /// Names the storefront in accessibility identifiers.
    let storeID: String
    let size: String?
    let lastPlayed: String?
    let playtime: String?
    private let heroArtwork: HeroArtwork
    private let cover: Cover
    private let content: Content
    @State private var appeared = false
    @State private var isCompact = false
    @Environment(\.gameTransition) private var transition
    @Environment(\.storefrontBar) private var bar
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.colorScheme) private var colorScheme

    /// The artwork's height below the bar it reaches under, as a library hero's.
    private static var heroHeight: CGFloat { ObeliskStyle.libraryHeroHeight }
    /// How much of the cover rises over the artwork.
    private static var coverOverlap: CGFloat { 150 }
    private static var coverRadius: CGFloat { 20 }

    init(
        model: Model,
        game: Model.Game,
        storeID: String,
        size: String?,
        lastPlayed: String? = nil,
        playtime: String?,
        @ViewBuilder heroArtwork: () -> HeroArtwork,
        @ViewBuilder cover: () -> Cover,
        @ViewBuilder content: () -> Content
    ) {
        self.model = model
        self.game = game
        self.storeID = storeID
        self.size = size
        self.lastPlayed = lastPlayed
        self.playtime = playtime
        self.heroArtwork = heroArtwork()
        self.cover = cover()
        self.content = content()
    }

    private var installations: GameInstallations<Model.Store> { model.installations }
    private var name: String { model.title(of: game) }

    /// Whether the page shows whole: at once, unless it opened with motion.
    private var isRevealed: Bool { appeared || transition?.source == nil }

    /// One step of the page's reveal, `delay` seconds after the cover leaves.
    private func reveal(_ delay: Double) -> Animation? {
        guard transition?.source != nil else { return nil }
        return reduceMotion
            ? .easeOut(duration: 0.2)
            : .spring(response: 0.38, dampingFraction: 0.86).delay(delay)
    }

    var body: some View {
        @Bindable var achievements = model.achievements
        let revealed = isRevealed
        return StorefrontScrollView(topOverlayHeight: ObeliskStyle.titleBarHeight) {
            VStack(alignment: .leading, spacing: 0) {
                hero(revealed: revealed)
                header(revealed: revealed)
                details
                    .opacity(revealed ? 1 : 0)
                    .offset(y: revealed || reduceMotion ? 0 : 36)
                    .animation(reveal(0.1), value: revealed)
            }
        }
        // The bar's backdrop shows once the artwork scrolled under it, and the
        // game's title once its own scrolled past the bar.
        .onScrollGeometryChange(for: Bool.self, of: { $0.contentOffset.y + $0.contentInsets.top > Self.heroHeight }) { _, under in
            bar?.showsBackdrop = under
        }
        .onScrollGeometryChange(
            for: Bool.self,
            of: { $0.contentOffset.y + $0.contentInsets.top > Self.heroHeight - 40 }
        ) { _, scrolled in
            bar?.showsGameTitle = scrolled
        }
        .onGeometryChange(for: Bool.self, of: { $0.size.width < 760 }) { isCompact = $0 }
        // Hides the library beneath while the page is revealed, and leaving.
        .background {
            ObeliskPalette(colorScheme: colorScheme).background
                .opacity(revealed ? 1 : 0)
                .animation(reveal(0), value: revealed)
                .ignoresSafeArea()
        }
        .ignoresSafeArea(.container, edges: .top)
        .onAppear { appeared = true }
        .sheet(isPresented: $achievements.isPresented) {
            GameAchievementsView(model: achievements, gameName: name, retry: model.reloadAchievements)
        }
    }

    private func hero(revealed: Bool) -> some View {
        // How far the artwork reaches up under the browser's bar.
        let reach = ObeliskStyle.titleBarHeight
        return HeroBackdrop {
            heroArtwork
                // The artwork settles from a little closer as it sharpens.
                .scaleEffect(revealed || reduceMotion ? 1 : 1.08)
                .blur(radius: revealed || reduceMotion ? 0 : 16)
                .opacity(revealed ? 1 : 0)
                .animation(reveal(0), value: revealed)
        }
        .frame(height: Self.heroHeight + reach)
        .padding(.top, -reach)
        .environment(\.colorScheme, .dark)
    }

    /// The cover overlapping the artwork, then its launch row and metrics
    /// beside it, below the artwork.
    private func header(revealed: Bool) -> some View {
        HStack(alignment: .top, spacing: isCompact ? 20 : 28) {
            coverView(revealed: revealed)
            VStack(alignment: .leading, spacing: 0) {
                // The part of the cover over the artwork.
                Color.clear.frame(height: Self.coverOverlap)
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 10) {
                        GamePrimaryAction(installations: installations, game: game.id)
                        if let receipt = installations.selectedReceipt {
                            GameManagementButton(
                                gameName: name,
                                isAvailable: installations.canUninstallSelectedGame,
                                uninstall: { installations.uninstall(receipt) },
                                directory: receipt.directory
                            )
                            .id(receipt.id + receipt.directory)
                        }
                    }
                    GameMetricsRow(metrics: metrics)
                }
                .padding(.top, 18)
                .opacity(revealed ? 1 : 0)
                .offset(y: revealed || reduceMotion ? 0 : 24)
                .animation(reveal(0.07), value: revealed)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, ObeliskStyle.contentPadding)
        .padding(.top, -Self.coverOverlap)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func coverView(revealed: Bool) -> some View {
        let shape = RoundedRectangle(cornerRadius: Self.coverRadius, style: .continuous)
        // A cover flown from the library shows whole all along; one opened
        // from the hero rises into place with the header.
        let flies = transition?.fliesCover == true
        let shown = revealed || flies
        return Color.clear
            .aspectRatio(2.0 / 3.0, contentMode: .fit)
            .overlay { cover }
            .clipShape(shape)
            .overlay {
                shape.strokeBorder(.white.opacity(colorScheme == .dark ? 0.12 : 0.3), lineWidth: 1)
                    .allowsHitTesting(false)
            }
            .modifier(GameCoverMatch(game: AnyHashable(game.id), isPage: true))
            .frame(width: isCompact ? 150 : 190)
            .shadow(color: .black.opacity(colorScheme == .dark ? 0.5 : 0.3), radius: 24, y: 14)
            .opacity(shown ? 1 : 0)
            .scaleEffect(shown || reduceMotion ? 1 : 0.9, anchor: .bottom)
            .offset(y: shown || reduceMotion ? 0 : 30)
            .animation(flies ? nil : reveal(0.03), value: shown)
            // The page's heading: the artwork shows no name.
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(name)
            .accessibilityAddTraits(.isHeader)
    }

    private var metrics: GameMetrics {
        GameMetrics(
            isInstalled: installations.selectedReceipt != nil,
            size: size,
            lastPlayed: lastPlayed,
            playtime: playtime,
            cloud: installations.cloudIndicator.map {
                CloudIndicatorButton(presentation: $0, canSync: installations.canSyncCloud, sync: model.syncCloud)
            },
            achievements: model.achievements
        )
    }

    /// Notices, then what only the storefront shows of the game.
    private var details: some View {
        VStack(alignment: .leading, spacing: 26) {
            StorefrontWarningBanner(message: $model.storageWarning)
            if model.cloud.failurePresented {
                SteamCloudFailureView(model: model, storefront: storeID)
            }
            if let message = model.gameSessionMessage {
                Text(message).font(.system(size: 12)).foregroundStyle(.secondary)
            }
            content
        }
        .padding(.horizontal, ObeliskStyle.contentPadding)
        .padding(.top, 34)
        .padding(.bottom, 40)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The game's primary action. It reads the Transfer progress in its own body,
/// so a progress tick redraws only the button.
private struct GamePrimaryAction<Store: InstallationStorefront>: View {
    let installations: GameInstallations<Store>
    let game: Store.GameID

    var body: some View {
        let cancelsSynchronization = installations.cloud.canCancelSynchronization(for: game)
        GameActionButton(
            title: installations.primaryActionTitle,
            launches: installations.primaryActionLaunches,
            transfer: installations.transfer(for: game),
            isTransferring: installations.activeTransferGame == game,
            isAvailable: installations.primaryActionAvailable,
            action: installations.primaryAction,
            cancel: {
                if cancelsSynchronization { installations.cloud.cancelSynchronization(for: game) }
                else { installations.cancelDownload() }
            },
            isCancellable: cancelsSynchronization
                || (installations.isLaunching && installations.launchGame == game),
            isStep: installations.isSynchronizingSaves || installations.runtimeInstallFraction != nil,
            stepFraction: installations.runtimeInstallFraction
        )
    }
}

/// Reports a game's failed or cancelled Transfer, or another game's operation
/// holding the storefront.
struct GameOperationStatus<Store: InstallationStorefront>: View {
    let installations: GameInstallations<Store>
    let game: Store.GameID

    var body: some View {
        if let message {
            Text(message).font(.system(size: 12)).foregroundStyle(.secondary)
        }
    }

    private var message: String? {
        if let transfer = installations.transfer(for: game), transfer.state == .failed || transfer.state == .cancelled {
            return transfer.message.isEmpty ? transfer.activityTitle : L10n.message(transfer.message)
        }
        if let activeGame = installations.activeTransferGame ?? installations.launchGame, activeGame != game {
            return L10n.string(Store.otherGameOperation)
        }
        return nil
    }
}

/// A titled section of a game's page.
struct GamePageSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title)
                .font(.system(size: 22, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            content
        }
    }
}

/// A game page's card: a rounded surface, concentric with the covers.
struct GamePageCard: ViewModifier {
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let palette = ObeliskPalette(colorScheme: colorScheme)
        let shape = RoundedRectangle(cornerRadius: 18, style: .continuous)
        content
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(palette.surface, in: shape)
            .overlay { shape.strokeBorder(palette.border, lineWidth: 1).allowsHitTesting(false) }
    }
}
