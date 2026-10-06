import SwiftUI

struct EpicView: View {
    @Bindable var model: EpicModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary && model.selectedGame == nil {
                library
            } else {
                StorefrontPage { titleBar } content: { content }
            }
        }
        .task { await model.start() }
        .onChange(of: model.gameSessions.isBusy) { _, isBusy in
            if !isBusy { model.refreshCloudIndicator() }
        }
        // The game's guard and launch sync are on disk once it runs.
        .onChange(of: model.gameSessions.active?.phase == .running) { _, running in
            if running { model.refreshCloudIndicator() }
        }
        .errorAlert(L10n.string("ui.store.epic"), message: $model.errorMessage)
    }

    private var library: some View {
        StorefrontLibraryPage(
            model: model,
            storeID: "epic",
            storeTitle: L10n.string("ui.store.epic"),
            libraryTitle: L10n.string("ui.epic.library"),
            accountName: model.accountName,
            scrollMemory: libraryScroll,
            hero: hero
        ) { game in
            LibraryCardContent(
                title: game.title,
                help: [
                    game.title,
                    game.developer.isEmpty ? nil : game.developer,
                    model.playtime(of: game).map(EpicFormatting.playtime),
                ].compactMap { $0 }.joined(separator: " · "),
                platform: .preferred(macos: game.macos, windows: game.windows)
            )
        } artwork: { game in
            EpicArtwork(imageURL: game.imageUrl)
        } accountMenu: {
            accountMenu
        }
    }

    private func hero(_ game: EpicGame) -> some View {
        LibraryHero(
            details: [
                model.lastLaunch(of: game).map { .init(symbol: "calendar", text: SteamFormatting.lastPlayed($0)) },
                model.playtime(of: game).map { .init(symbol: "clock", text: EpicFormatting.playtime($0)) },
            ].compactMap { $0 },
            open: { model.open(game) }
        ) {
            EpicArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } logo: {
            // Epic's wide artwork already shows the game's name, and its
            // catalog rarely has a separate logo.
            LibraryHeroLogo(url: nil, title: game.title, showsTitle: false)
        }
    }

    /// A game's page, or the sign-in while no account is connected.
    private var content: some View {
        VStack(alignment: .leading, spacing: 25) {
            StorefrontWarningBanner(message: $model.storageWarning)
            if model.isConnected {
                if model.cloud.failurePresented {
                    SteamCloudFailureView(model: model, storefront: "epic")
                }
                if let game = model.selectedGame {
                    EpicGameDetailView(model: model, game: game)
                }
            } else { EpicLoginView(model: model).padding(.top, 28) }
        }
        .padding(ObeliskStyle.contentPadding)
        .padding(.top, 6)
        .frame(maxWidth: 1400, alignment: .leading)
    }

    private var titleBar: some View {
        StorefrontTitleBar(
            title: L10n.string("ui.store.epic"),
            accountName: model.accountName,
            isConnected: model.isConnected
        ) {
            accountMenu
        }
    }

    @ViewBuilder private var accountMenu: some View {
        Text(model.accountName ?? L10n.string("ui.epic.account"))
        Text(L10n.string("ui.library.account_game_count", Int64(model.games.count)))
        Divider()
        Button(model.friends?.menuTitle ?? L10n.string("ui.friends.menuLoading"), systemImage: "person.2", action: model.openFriends)
            .disabled(model.friends == nil)
            .accessibilityIdentifier("epic-friends-menu")
        Button(L10n.string("ui.library.refresh"), systemImage: "arrow.clockwise", action: model.refresh)
            .disabled(model.isLoadingGames)
        Toggle(L10n.string("ui.account.remember_me"), isOn: $model.rememberSession)
        Divider()
        Button(L10n.string("ui.account.sign_out"), systemImage: "rectangle.portrait.and.arrow.right", action: model.signOut)
    }
}

struct EpicArtwork: View {
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
