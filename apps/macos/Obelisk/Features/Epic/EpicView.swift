import SwiftUI

struct EpicView: View {
    @Bindable var model: EpicModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary {
                browser
            } else {
                WebSignInView(
                    model: model,
                    section: .epic,
                    title: L10n.string("ui.epic.sign_in_title"),
                    subtitle: L10n.string("ui.epic.sign_in_subtitle"),
                    connecting: L10n.string("ui.epic.connecting"),
                    accountSignIn: L10n.string("ui.epic.account_sign_in"),
                    instructions: L10n.string("ui.epic.sign_in_instructions"),
                    signInLabel: L10n.string("ui.epic.sign_in")
                )
            }
        }
        .task { await model.start() }
        .onChange(of: model.gameSessions.isBusy) { _, isBusy in
            if !isBusy { model.installations.refreshCloudIndicator() }
        }
        // The game's guard and launch sync are on disk once it runs.
        .onChange(of: model.gameSessions.active?.phase == .running) { _, running in
            if running { model.installations.refreshCloudIndicator() }
        }
        .errorAlert(L10n.string("ui.store.epic"), message: $model.errorMessage)
    }

    private var browser: some View {
        StorefrontBrowser(
            model: model,
            storeID: "epic",
            storeTitle: L10n.string("ui.store.epic"),
            libraryTitle: L10n.string("ui.epic.library"),
            accountName: model.accountName,
            storePage: { [model] in WebPageStart(url: await model.webPage(EpicGames.storeURL)) },
            scrollMemory: libraryScroll
        ) { game in
            StorefrontArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } heroLogo: { _ in
            // Epic's wide artwork already shows the game's name, and its
            // catalog rarely has a separate logo.
            EmptyView()
        } artwork: { game in
            StorefrontArtwork(imageURL: game.imageUrl)
        } accountMenu: {
            accountMenu
        } gamePage: { game, context in
            EpicGameDetailView(model: model, game: game, context: context)
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
