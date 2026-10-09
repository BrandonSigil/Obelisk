import SwiftUI

struct SteamView: View {
    @Bindable var model: SteamModel
    @State private var libraryScroll = LibraryScrollMemory()

    var body: some View {
        Group {
            if model.showsLibrary {
                browser
            } else {
                SteamLoginView(model: model)
            }
        }
        .task { await model.start() }
        .errorAlert(L10n.string("ui.store.steam"), message: $model.errorMessage)
    }

    private var browser: some View {
        StorefrontBrowser(
            model: model,
            storeID: "steam",
            storeTitle: L10n.string("ui.store.steam"),
            libraryTitle: L10n.string("ui.steam.library"),
            accountName: model.displayName,
            avatarURL: model.avatarURL,
            storePage: model.storePage,
            scrollMemory: libraryScroll,
            heroArtwork: Self.heroArtwork,
            heroLogo: Self.heroLogo,
            artwork: Self.cover
        ) {
            accountMenu
        } gamePage: { game, context in
            SteamGameDetailView(model: model, game: game, context: context)
        }
    }

    /// A game's wide artwork, as its library's hero and its page show it.
    static func heroArtwork(_ game: SteamGame) -> SteamArtwork {
        SteamArtwork(appId: game.appId, portrait: false, iconHash: game.iconHash)
    }

    static func heroLogo(_ game: SteamGame) -> LibraryHeroLogo {
        LibraryHeroLogo(url: SteamArtwork.logoURL(appId: game.appId), title: game.name)
    }

    static func cover(_ game: SteamGame) -> SteamArtwork {
        SteamArtwork(appId: game.appId, iconHash: game.iconHash)
    }

    @ViewBuilder private var accountMenu: some View {
        Text(model.displayName ?? L10n.string("ui.steam.account"))
        Text(L10n.string("ui.library.account_game_count", Int64(model.games.count)))
        Divider()
        SteamFriendsMenuButton(model: model)
        Divider()
        Button(L10n.string("ui.library.refresh"), systemImage: "arrow.clockwise", action: model.refresh)
        Divider()
        Button(L10n.string("ui.account.sign_out"), systemImage: "rectangle.portrait.and.arrow.right", action: model.signOut)
    }
}

/// Reads the friends and the unread messages in its own body: their every
/// change redraws the button alone, not the library.
private struct SteamFriendsMenuButton: View {
    let model: SteamModel

    var body: some View {
        Button(title, systemImage: "person.2", action: model.openFriends)
            .accessibilityIdentifier("steam-friends-menu")
    }

    private var title: String {
        let title = model.friends?.menuTitle ?? L10n.string("ui.friends.menuLoading")
        let unread = model.chat.totalUnread
        return unread > 0 ? [title, L10n.string("ui.chat.unreadCount", Int64(unread))].joined(separator: " · ") : title
    }
}

/// Both stores use the same progress text, accessibility and transfer controls.
extension GameTransfer {
    var activityTitle: String {
        switch state {
        case .preparing: L10n.string("ui.transfer.checking_updates")
        case .verifying: L10n.string("ui.transfer.verifying_files")
        case .downloading: L10n.string("ui.transfer.downloading")
        case .complete: L10n.string("ui.transfer.up_to_date")
        case .cancelled: L10n.string("ui.transfer.interrupted")
        case .failed: L10n.string("ui.transfer.failed")
        }
    }

    var verifiedProgressText: String {
        L10n.string("ui.transfer.verified_files", GameFormatting.bytes(completedBytes), GameFormatting.bytes(totalBytes))
    }

    var downloadedGameDataText: String? {
        if contentBytes > 0 {
            L10n.string("ui.transfer.downloaded_data", GameFormatting.bytes(contentBytes))
        } else if state == .verifying {
            L10n.string("ui.transfer.no_downloaded_data")
        } else {
            nil
        }
    }

    var buttonTitle: String {
        switch state {
        case .preparing: L10n.string("ui.transfer.preparing_short")
        case .verifying: L10n.string("ui.transfer.verifying_short")
        case .downloading: L10n.string("ui.transfer.downloading_short")
        case .complete: L10n.string("ui.transfer.finishing_short")
        case .cancelled: L10n.string("ui.transfer.cancelling_short")
        case .failed: L10n.string("ui.transfer.failed_short")
        }
    }

    var isInProgress: Bool {
        state == .preparing || state == .verifying || state == .downloading
    }

    var progressFraction: Double? {
        guard totalBytes > 0 else { return nil }
        return Double(min(completedBytes, totalBytes)) / Double(totalBytes)
    }

    var progressSummary: String {
        [
            activityTitle,
            totalBytes > 0 ? verifiedProgressText : nil,
            downloadedGameDataText,
            currentFile.isEmpty ? nil : L10n.message(currentFile),
            message.isEmpty ? nil : L10n.message(message),
        ]
        .compactMap { $0 }
        .joined(separator: "\n")
    }
}
