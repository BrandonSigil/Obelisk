import SwiftUI

/// A game's library artwork, in the order Steam's library tries it: the image
/// its PICS app info names, which newer games publish only under a hashed
/// folder, then the one at Steam's plain name, as while PICS has not told of
/// the game. A game without a cover has the portrait Steam generates for it.
struct SteamArtwork: View {
    let appId: UInt32
    var portrait = true
    var iconHash = ""
    @Environment(SteamLibraryArtwork.self) private var library: SteamLibraryArtwork?

    /// The image at `path` under the game's store assets.
    static func url(appId: UInt32, path: String) -> URL? {
        URL(string: "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/\(appId)/\(path)")
    }

    /// The header last, which games without library artwork still have.
    private var urls: [URL] {
        let artwork = library?.games[appId]
        let paths: [String?] = portrait
            ? [artwork?.cover, "library_600x900.jpg", "portrait.png?v=2", artwork?.header, "header.jpg"]
            : [artwork?.hero, "library_hero.jpg", artwork?.header, "header.jpg"]
        // A game such as ELDEN RING names its images at the plain names.
        var seen: Set<String> = []
        return paths.compactMap { $0 }.filter { seen.insert($0).inserted }
            .compactMap { Self.url(appId: appId, path: $0) }
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

/// The game's library logo over its hero, or its name where the game
/// publishes none.
struct SteamHeroLogo: View {
    let appId: UInt32
    let title: String
    @Environment(SteamLibraryArtwork.self) private var library: SteamLibraryArtwork?

    var body: some View {
        let path = library?.games[appId]?.logo ?? "logo.png"
        LibraryHeroLogo(url: SteamArtwork.url(appId: appId, path: path), title: title)
    }
}
