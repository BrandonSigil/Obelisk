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
        .onChange(of: model.gameSessions.isBusy) { _, isBusy in
            if !isBusy { model.installations.refreshCloudIndicator() }
        }
        // The game's guard and launch sync are on disk once it runs.
        .onChange(of: model.gameSessions.active?.running == true) { _, running in
            if running { model.installations.refreshCloudIndicator() }
        }
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
            scrollMemory: libraryScroll
        ) { game in
            SteamArtwork(appId: game.appId, portrait: false, iconHash: game.iconHash)
        } heroLogo: { game in
            LibraryHeroLogo(url: SteamArtwork.logoURL(appId: game.appId), title: game.name)
        } artwork: { game in
            SteamArtwork(appId: game.appId, iconHash: game.iconHash)
        } accountMenu: {
            accountMenu
        } gamePage: { game, context in
            SteamGameDetailView(model: model, game: game, context: context)
        }
    }

    @ViewBuilder private var accountMenu: some View {
        Text(model.displayName ?? L10n.string("ui.steam.account"))
        Text(L10n.string("ui.library.account_game_count", Int64(model.games.count)))
        Divider()
        Button(model.friends?.menuTitle ?? L10n.string("ui.friends.menuLoading"), systemImage: "person.2", action: model.openFriends)
            .accessibilityIdentifier("steam-friends-menu")
        Divider()
        Button(L10n.string("ui.library.refresh"), systemImage: "arrow.clockwise", action: model.refresh)
        Divider()
        Button(L10n.string("ui.account.sign_out"), systemImage: "rectangle.portrait.and.arrow.right", action: model.signOut)
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
