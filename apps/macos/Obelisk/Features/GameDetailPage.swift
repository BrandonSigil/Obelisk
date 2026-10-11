import SwiftUI

/// A storefront's game page: the game's wide artwork reaching under the
/// browser's glass bar, its cover overlapping the artwork's lower edge, its
/// launch row and metrics beside the cover, then what only the storefront shows.
/// Opened from the library, the page reveals itself around the cover flying
/// in: the artwork settles, then the header and the content rise.
struct GameDetailPage<Model: LibraryStorefront, HeroArtwork: View, Cover: View, Content: View>: View {
    @Bindable var model: Model
    let game: Model.Game
    let size: String?
    /// Whether a game never played says so, rather than showing no last
    /// played date and no playtime.
    let showsUnplayed: Bool
    private let context: GamePageContext<HeroArtwork, Cover>
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

    init(
        model: Model,
        game: Model.Game,
        context: GamePageContext<HeroArtwork, Cover>,
        size: String?,
        showsUnplayed: Bool = false,
        @ViewBuilder content: () -> Content
    ) {
        self.model = model
        self.game = game
        self.context = context
        self.size = size
        self.showsUnplayed = showsUnplayed
        self.content = content()
    }

    private var installations: GameInstallations<Model.Store> { model.installations }
    private var name: String { model.title(of: game) }

    /// Whether the page shows whole: at once, unless it opened with motion.
    private var isRevealed: Bool { appeared || transition?.source == nil }

    /// One step of the page's reveal, `delay` seconds after the cover leaves.
    private func reveal(_ delay: Double) -> Animation? {
        guard transition?.source != nil else { return nil }
        return reduceMotion ? .easeOut(duration: 0.2) : ObeliskStyle.pageAnimation(reduceMotion: false).delay(delay)
    }

    var body: some View {
        @Bindable var achievements = model.achievements
        return StorefrontScrollView {
            VStack(alignment: .leading, spacing: 0) {
                hero
                header
                details
                    .opacity(isRevealed ? 1 : 0)
                    .offset(y: isRevealed || reduceMotion ? 0 : 36)
                    .animation(reveal(0.1), value: isRevealed)
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
                .opacity(isRevealed ? 1 : 0)
                .animation(reveal(0), value: isRevealed)
                .ignoresSafeArea()
        }
        .ignoresSafeArea(.container, edges: .top)
        .onAppear { appeared = true }
        .sheet(isPresented: $achievements.isPresented) {
            GameAchievementsView(model: achievements, gameName: name, retry: model.reloadAchievements)
        }
    }

    private var hero: some View {
        HeroBackdrop {
            context.heroArtwork
                // The artwork settles from a little closer as it sharpens.
                .scaleEffect(isRevealed || reduceMotion ? 1 : 1.08)
                .blur(radius: isRevealed || reduceMotion ? 0 : 16)
                .opacity(isRevealed ? 1 : 0)
                .animation(reveal(0), value: isRevealed)
        }
        .heroFrame()
    }

    /// The cover overlapping the artwork, then its launch row and metrics
    /// beside it, below the artwork.
    private var header: some View {
        HStack(alignment: .top, spacing: isCompact ? 20 : 28) {
            coverView
            VStack(alignment: .leading, spacing: 0) {
                // The part of the cover over the artwork.
                Color.clear.frame(height: Self.coverOverlap)
                GameLaunchRow(metrics: metrics) {
                    GamePrimaryAction(installations: installations, game: game.id, gameName: name)
                    if let receipt = installations.selectedReceipt {
                        GameManagementButton(
                            gameName: name,
                            canVerify: installations.canVerifySelectedGame,
                            canUninstall: installations.canUninstallSelectedGame,
                            verify: { installations.verify(receipt) },
                            uninstall: { installations.uninstall(receipt) },
                            directory: receipt.directory
                        )
                        .id(receipt.id + receipt.directory)
                    }
                }
                .padding(.top, 18)
                .opacity(isRevealed ? 1 : 0)
                .offset(y: isRevealed || reduceMotion ? 0 : 24)
                .animation(reveal(0.07), value: isRevealed)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, ObeliskStyle.contentPadding)
        .padding(.top, -Self.coverOverlap)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var coverView: some View {
        let shape = RoundedRectangle(cornerRadius: ObeliskStyle.coverCornerRadius, style: .continuous)
        // A cover flown from the library shows whole all along; one opened
        // from the hero rises into place with the header.
        let flies = transition?.source == .cover && !reduceMotion
        let shown = isRevealed || flies
        return Color.clear
            .aspectRatio(ObeliskStyle.coverAspectRatio, contentMode: .fit)
            .overlay { context.cover }
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

    private var lastPlayed: String? {
        let date = model.lastPlayed(of: game)
        return showsUnplayed ? GameFormatting.lastPlayed(date) : date.map(GameFormatting.lastPlayed)
    }

    private var playtime: String? {
        let playtime = model.playtime(of: game)
        return showsUnplayed ? GameFormatting.playtime(playtime ?? .zero) : playtime.map(GameFormatting.playtime)
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
                SteamCloudFailureView(model: model, storefront: context.storeID)
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
    let gameName: String
    @State private var confirmsForceQuit = false

    var body: some View {
        let cancelsSynchronization = installations.cloud.canCancelSynchronization(for: game)
        GameActionButton(
            title: installations.primaryActionTitle,
            launches: installations.primaryActionLaunches,
            transfer: installations.transfer(for: game),
            isTransferring: installations.activeTransferGame == game,
            isAvailable: installations.primaryActionAvailable,
            action: {
                if installations.primaryQuitStage == .forceQuit { confirmsForceQuit = true }
                else { installations.primaryAction() }
            },
            cancel: {
                if cancelsSynchronization { installations.cloud.cancelSynchronization(for: game) }
                else { installations.cancel(game) }
            },
            isCancellable: cancelsSynchronization
                || (installations.isLaunching && installations.launchGame == game)
                || installations.isQueued(game),
            isStep: installations.isSynchronizingSaves || installations.runtimeInstallFraction != nil,
            stepFraction: installations.runtimeInstallFraction,
            quitStage: installations.primaryQuitStage
        )
        .forceQuitConfirmation(gameName, isPresented: $confirmsForceQuit, confirm: installations.forceQuitRunningGame)
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
        // Another game's operation matters only when it keeps this one waiting.
        if let activeGame = installations.activeTransferGame ?? installations.launchGame, activeGame != game,
           !installations.primaryActionAvailable {
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
