import SwiftUI

struct EpicGameDetailView: View {
    @Bindable var model: EpicModel
    let game: EpicGame

    private var installations: EpicInstallations { model.installations }

    var body: some View {
        GameDetailPage(
            model: model,
            game: game.libraryID,
            name: game.title,
            libraryTitle: L10n.string("ui.epic.library"),
            size: (installations.selectedReceipt?.totalBytes ?? model.selectedGameDetails?.installationBytes)
                .map(GameFormatting.bytes),
            lastPlayed: model.lastPlayed(of: game).map(GameFormatting.lastPlayed),
            playtime: model.playtime(of: game).map(GameFormatting.playtime),
            back: model.closeGame
        ) {
            EpicArtwork(imageURL: game.headerImageUrl ?? game.imageUrl, maxPixelSize: 2048)
        } content: {
            GameOperationStatus(installations: installations, game: game.libraryID)
        }
    }
}
