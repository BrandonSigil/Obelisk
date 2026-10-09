import SwiftUI

struct GogView: View {
    @Bindable var model: GogModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary {
                browser
            } else {
                WebSignInView(
                    model: model,
                    section: .gog,
                    title: L10n.string("ui.gog.sign_in_title"),
                    subtitle: L10n.string("ui.gog.sign_in_subtitle"),
                    connecting: L10n.string("ui.gog.connecting"),
                    accountSignIn: L10n.string("ui.gog.account_sign_in"),
                    instructions: L10n.string("ui.gog.sign_in_instructions"),
                    signInLabel: L10n.string("ui.gog.sign_in")
                )
            }
        }
        .task { await model.start() }
        .errorAlert(L10n.string("ui.store.gog"), message: $model.errorMessage)
    }

    private var browser: some View {
        StorefrontBrowser(
            model: model,
            storeID: "gog",
            storeTitle: L10n.string("ui.store.gog"),
            libraryTitle: L10n.string("ui.gog.library"),
            accountName: model.accountName,
            avatarURL: model.avatarURL,
            storePage: model.storePage,
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
        } gamePage: { game, context in
            GogGameDetailView(model: model, game: game, context: context)
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
