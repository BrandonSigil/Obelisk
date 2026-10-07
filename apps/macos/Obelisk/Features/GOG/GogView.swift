import SwiftUI

struct GogView: View {
    @Bindable var model: GogModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary && model.selectedGame == nil {
                library
            } else if model.isConnected {
                StorefrontPage { titleBar } content: { content }
            } else {
                GogLoginView(model: model)
            }
        }
        .task { await model.start() }
        .errorAlert(L10n.string("ui.store.gog"), message: $model.errorMessage)
    }

    private var library: some View {
        StorefrontLibraryPage(
            model: model,
            storeID: "gog",
            storeTitle: L10n.string("ui.store.gog"),
            libraryTitle: L10n.string("ui.gog.library"),
            accountName: model.accountName,
            avatarURL: model.avatarURL,
            scrollMemory: libraryScroll
        ) { game in
            StorefrontArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } heroLogo: { game in
            // GOG's backgrounds leave out the game's name.
            LibraryHeroLogo(url: nil, title: game.title)
        } artwork: { game in
            StorefrontArtwork(imageURL: game.imageUrl ?? game.headerImageUrl)
        } accountMenu: {
            accountMenu
        }
    }

    /// A game's page.
    private var content: some View {
        VStack(alignment: .leading, spacing: 25) {
            StorefrontWarningBanner(message: $model.storageWarning)
            if let game = model.selectedGame {
                GogGameDetailView(model: model, game: game)
            }
        }
        .padding(ObeliskStyle.contentPadding)
        .padding(.top, 6)
        .frame(maxWidth: 1400, alignment: .leading)
    }

    private var titleBar: some View {
        StorefrontTitleBar(
            title: L10n.string("ui.store.gog"),
            accountName: model.accountName,
            avatarURL: model.avatarURL,
            isConnected: model.isConnected
        ) {
            accountMenu
        }
    }

    @ViewBuilder private var accountMenu: some View {
        Text(model.accountName ?? L10n.string("ui.gog.account"))
        Text(L10n.string("ui.library.account_game_count", Int64(model.games.count)))
        Divider()
        Button(L10n.string("ui.library.refresh"), systemImage: "arrow.clockwise", action: model.refresh)
            .disabled(model.isLoadingGames)
        Toggle(L10n.string("ui.account.remember_me"), isOn: $model.rememberSession)
        Divider()
        Button(L10n.string("ui.account.sign_out"), systemImage: "rectangle.portrait.and.arrow.right", action: model.signOut)
    }
}
