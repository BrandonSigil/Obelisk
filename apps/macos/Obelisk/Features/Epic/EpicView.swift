import SwiftUI

struct EpicView: View {
    @Bindable var model: EpicModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary && model.selectedGame == nil {
                library
            } else if model.isConnected {
                StorefrontPage { titleBar } content: { content }
            } else {
                EpicLoginView(model: model)
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
            scrollMemory: libraryScroll
        ) { game in
            EpicArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } heroLogo: { _ in
            // Epic's wide artwork already shows the game's name, and its
            // catalog rarely has a separate logo.
            EmptyView()
        } artwork: { game in
            EpicArtwork(imageURL: game.imageUrl)
        } accountMenu: {
            accountMenu
        }
    }

    /// A game's page.
    private var content: some View {
        VStack(alignment: .leading, spacing: 25) {
            StorefrontWarningBanner(message: $model.storageWarning)
            if model.cloud.failurePresented {
                SteamCloudFailureView(model: model, storefront: "epic")
            }
            if let game = model.selectedGame {
                EpicGameDetailView(model: model, game: game)
            }
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
