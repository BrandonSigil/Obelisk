import SwiftUI

/// A signed-in storefront: its library, or the page of the game it opened,
/// under one floating glass bar. Opening a game from its cover flies the cover
/// from the library to the game's page while the library falls back and the
/// page's artwork settles in; going back flies it home.
struct StorefrontBrowser<Model: LibraryStorefront, HeroArtwork: View, HeroLogo: View, Artwork: View,
                         AccountMenu: View, GamePage: View>: View {
    @Bindable var model: Model
    let storeID: String
    let storeTitle: String
    let libraryTitle: String
    let accountName: String?
    let avatarURL: URL?
    let scrollMemory: LibraryScrollMemory
    @ViewBuilder let heroArtwork: (Model.Game) -> HeroArtwork
    @ViewBuilder let heroLogo: (Model.Game) -> HeroLogo
    @ViewBuilder let artwork: (Model.Game) -> Artwork
    @ViewBuilder let accountMenu: AccountMenu
    @ViewBuilder let gamePage: (Model.Game) -> GamePage
    @Namespace private var namespace
    @State private var bar = StorefrontBarState()
    /// Where the open game came from, once a game opened with motion.
    @State private var source: GameOpeningSource?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(
        model: Model,
        storeID: String,
        storeTitle: String,
        libraryTitle: String,
        accountName: String?,
        avatarURL: URL? = nil,
        scrollMemory: LibraryScrollMemory,
        @ViewBuilder heroArtwork: @escaping (Model.Game) -> HeroArtwork,
        @ViewBuilder heroLogo: @escaping (Model.Game) -> HeroLogo,
        @ViewBuilder artwork: @escaping (Model.Game) -> Artwork,
        @ViewBuilder accountMenu: () -> AccountMenu,
        @ViewBuilder gamePage: @escaping (Model.Game) -> GamePage
    ) {
        self.model = model
        self.storeID = storeID
        self.storeTitle = storeTitle
        self.libraryTitle = libraryTitle
        self.accountName = accountName
        self.avatarURL = avatarURL
        self.scrollMemory = scrollMemory
        self.heroArtwork = heroArtwork
        self.heroLogo = heroLogo
        self.artwork = artwork
        self.accountMenu = accountMenu()
        self.gamePage = gamePage
    }

    /// How a game's page replaces its library and back.
    private var motion: Animation {
        reduceMotion ? .easeInOut(duration: 0.2) : .spring(response: 0.38, dampingFraction: 0.86)
    }

    var body: some View {
        ZStack {
            if let game = model.selectedGame {
                gamePage(game)
                    .id(game.id)
                    // Above the library it replaces, which falls back beneath it.
                    .zIndex(1)
                    // The page reveals itself; leaving, it fades over the
                    // library as its cover flies home.
                    .transition(.asymmetric(insertion: .identity, removal: .opacity))
            } else {
                StorefrontLibraryPage(
                    model: model,
                    storeID: storeID,
                    storeTitle: storeTitle,
                    libraryTitle: libraryTitle,
                    scrollMemory: scrollMemory,
                    open: open
                ) { game in
                    heroArtwork(game)
                } heroLogo: { game in
                    heroLogo(game)
                } artwork: { game in
                    artwork(game)
                }
                .transition(.asymmetric(insertion: .identity,
                                        removal: .opacity.combined(with: .scale(scale: reduceMotion ? 1 : 0.94))))
            }
        }
        // An overlay rather than a safe area bar: on macOS the bar's scroll
        // edge effect cannot be turned off, and it veils the hero's artwork.
        .overlay(alignment: .top) {
            StorefrontBrowserBar(
                model: model,
                bar: bar,
                storeTitle: storeTitle,
                libraryTitle: libraryTitle,
                accountName: accountName,
                avatarURL: avatarURL,
                back: back
            ) {
                accountMenu
            }
        }
        .environment(\.storefrontBar, bar)
        .environment(\.gameTransition, GameTransition(namespace: namespace, source: source, flies: !reduceMotion))
        .ignoresSafeArea(.container, edges: .top)
    }

    private func open(_ game: Model.Game, from source: GameOpeningSource) {
        self.source = source
        withAnimation(motion) { model.open(game) }
    }

    private func back() {
        withAnimation(motion) { model.closeGame() }
    }
}

/// The browser's bar. It reads the scroll state in its own body, so scrolling
/// redraws the bar alone.
private struct StorefrontBrowserBar<Model: LibraryStorefront, AccountMenu: View>: View {
    @Bindable var model: Model
    let bar: StorefrontBarState
    let storeTitle: String
    let libraryTitle: String
    let accountName: String?
    let avatarURL: URL?
    let back: () -> Void
    @ViewBuilder let accountMenu: AccountMenu

    var body: some View {
        LibraryGlassBar(
            storeTitle: storeTitle,
            accountName: accountName,
            avatarURL: avatarURL,
            searchText: $model.searchText,
            showsBackdrop: bar.showsBackdrop,
            game: model.selectedGame.map { game in
                LibraryGlassBar.GameBar(title: model.title(of: game), libraryTitle: libraryTitle,
                                        showsTitle: bar.showsGameTitle, back: back)
            }
        ) {
            accountMenu
        }
    }
}

/// What the page under a storefront's bar tells it as it scrolls.
@MainActor @Observable
final class StorefrontBarState {
    /// Whether content, rather than a hero's artwork or nothing, is under the bar.
    var showsBackdrop = false
    /// Whether a game's page scrolled its title under the bar.
    var showsGameTitle = false
}

/// Where a game's page was opened from.
enum GameOpeningSource {
    /// The game's cover in the library grid, which flies to the page.
    case cover
    /// The library's hero, whose artwork the page's own replaces in place.
    case hero
}

/// How a library and a game's page hand the game's cover to each other.
struct GameTransition {
    let namespace: Namespace.ID
    /// Where the open game came from; nil when it opened without motion, as
    /// when a test or a refresh opens it.
    let source: GameOpeningSource?
    /// Whether covers fly between the library and the page; while motion is
    /// reduced, the page only fades in and out.
    let flies: Bool

    /// Whether the open game's cover flies between its card and its page.
    var fliesCover: Bool { flies && source == .cover }
}

extension EnvironmentValues {
    @Entry var gameTransition: GameTransition?
    @Entry var storefrontBar: StorefrontBarState?
}

/// Matches a game's cover in the library with its cover on the game's page.
/// The page's cover only matches when the game opened from its cover, so a
/// game opened from the hero does not pull a card from the grid.
struct GameCoverMatch: ViewModifier {
    let game: AnyHashable
    let isPage: Bool
    @Environment(\.gameTransition) private var transition

    private struct ID: Hashable {
        let game: AnyHashable
        let matched: Bool
    }

    func body(content: Content) -> some View {
        if let transition {
            content.matchedGeometryEffect(
                id: ID(game: game, matched: !isPage || transition.fliesCover),
                in: transition.namespace
            )
        } else {
            content
        }
    }
}
