import SwiftUI

struct SteamView: View {
    @Bindable var model: SteamModel
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
        .onChange(of: model.gameSessions.active?.running == true) { _, running in
            if running { model.refreshCloudIndicator() }
        }
        .errorAlert(L10n.string("ui.store.steam"), message: $model.errorMessage)
    }

    private var library: some View {
        StorefrontLibraryPage(
            model: model,
            storeID: "steam",
            storeTitle: L10n.string("ui.store.steam"),
            libraryTitle: L10n.string("ui.steam.library"),
            accountName: model.displayName,
            avatarURL: model.avatarURL,
            scrollMemory: libraryScroll,
            hero: hero
        ) { game in
            LibraryCardContent(
                title: game.name,
                accessibilityLabel: L10n.string("ui.steam.game_accessibility", game.name, SteamFormatting.lastPlayed(game.lastPlayed)),
                help: L10n.string("ui.steam.game_help", game.name, SteamFormatting.playtime(game.playtimeMinutes)),
                platform: .preferred(platforms: model.libraryPlatforms.platforms[game.appId] ?? [])
            )
        } artwork: { game in
            SteamArtwork(appId: game.appId, iconHash: game.iconHash)
        } accountMenu: {
            accountMenu
        }
    }

    private func hero(_ game: SteamGame) -> some View {
        LibraryHero(
            details: [
                model.lastPlayed(of: game).map { .init(symbol: "calendar", text: SteamFormatting.lastPlayed($0)) },
                .init(symbol: "clock", text: SteamFormatting.playtime(game.playtimeMinutes)),
            ].compactMap { $0 },
            open: { model.open(game) }
        ) {
            SteamArtwork(appId: game.appId, portrait: false, iconHash: game.iconHash)
        } logo: {
            LibraryHeroLogo(url: SteamArtwork.logoURL(appId: game.appId), title: game.name)
        }
    }

    /// A game's page, or the sign-in while no account is connected.
    private var content: some View {
        VStack(alignment: .leading, spacing: 25) {
            StorefrontWarningBanner(message: $model.storageWarning)

            if model.isConnected {
                if model.cloud.failurePresented {
                    SteamCloudFailureView(model: model, storefront: "steam")
                }
                if let game = model.selectedGame {
                    SteamGameDetailView(model: model, game: game)
                }
            } else {
                SteamLoginView(model: model)
                    .padding(.top, 28)
            }
        }
        .padding(ObeliskStyle.contentPadding)
        .padding(.top, 6)
        .frame(maxWidth: 1400, alignment: .leading)
    }

    private var titleBar: some View {
        StorefrontTitleBar(
            title: L10n.string("ui.store.steam"),
            accountName: model.displayName,
            avatarURL: model.avatarURL,
            isConnected: model.isConnected
        ) {
            accountMenu
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
        L10n.string("ui.transfer.verified_files", SteamFormatting.bytes(completedBytes), SteamFormatting.bytes(totalBytes))
    }

    var downloadedGameDataText: String? {
        if contentBytes > 0 {
            L10n.string("ui.transfer.downloaded_data", SteamFormatting.bytes(contentBytes))
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
