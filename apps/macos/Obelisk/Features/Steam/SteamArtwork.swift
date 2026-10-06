import SwiftUI

struct SteamArtwork: View {
    let appId: UInt32
    var portrait = true
    var iconHash = ""

    private static func assets(_ appId: UInt32) -> String {
        "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/\(appId)"
    }

    /// The game's library logo, which some games do not publish.
    static func logoURL(appId: UInt32) -> URL? {
        URL(string: "\(assets(appId))/logo.png")
    }

    /// Some games only publish the standard library hero or the store header.
    private var urls: [URL] {
        let names = portrait ? ["library_600x900.jpg", "header.jpg"]
            : ["library_hero_2x.jpg", "library_hero.jpg", "header.jpg"]
        return names.compactMap { URL(string: "\(Self.assets(appId))/\($0)") }
    }

    var body: some View {
        // A cover decodes at its published size, a hero at most 2048 pixels wide as Epic's.
        CachedArtwork(urls: urls, maxPixelSize: portrait ? 900 : 2048) { image in
            image.resizable().scaledToFill()
        } placeholder: {
            ArtworkPlaceholder()
        } failure: {
            icon
        }
        .accessibilityHidden(true)
    }

    @ViewBuilder private var icon: some View {
        if iconHash.count == 40 && iconHash.allSatisfy(\.isHexDigit),
           let url = URL(string: "https://media.steampowered.com/steamcommunity/public/images/apps/\(appId)/\(iconHash).jpg") {
            CachedArtwork(url: url, maxPixelSize: 64) { image in
                ArtworkPlaceholder().overlay { image.resizable().scaledToFit().padding(30) }
            } placeholder: {
                ArtworkPlaceholder()
            }
        } else {
            ArtworkPlaceholder()
        }
    }
}
