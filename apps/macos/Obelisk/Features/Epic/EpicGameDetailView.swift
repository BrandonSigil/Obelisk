import SwiftUI

struct EpicGameDetailView: View {
    @Bindable var model: EpicModel
    let game: EpicGame

    private var installations: EpicInstallations { model.installations }

    var body: some View {
        GameDetailPage(
            model: model,
            game: game,
            storeID: "epic",
            size: (installations.selectedReceipt?.totalBytes ?? model.selectedGameDetails?.installationBytes)
                .map(GameFormatting.bytes),
            lastPlayed: model.lastPlayed(of: game).map(GameFormatting.lastPlayed),
            playtime: model.playtime(of: game).map(GameFormatting.playtime)
        ) {
            StorefrontArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } cover: {
            StorefrontArtwork(imageURL: game.imageUrl)
        } content: {
            GameOperationStatus(installations: installations, game: game.libraryID)
            GameAboutSection(
                summary: game.description,
                facts: game.developer.isEmpty ? [] : [.init(label: L10n.string("ui.game.developer"), value: game.developer)]
            )
        }
    }
}
