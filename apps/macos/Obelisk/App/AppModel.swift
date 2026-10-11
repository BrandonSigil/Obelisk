import Observation
import SwiftUI

/// A storefront whose account shows its friends: Steam and Epic.
@MainActor
protocol FriendsStorefront: AnyObject {
    var friends: FriendsPresentationState? { get }
    func closeFriends()
    func retryFriends()
    func signOut()
}

extension SteamModel: FriendsStorefront {}
extension WebAccountModel: FriendsStorefront where Store == EpicGames {}

@MainActor
@Observable
final class AppModel {
    var selection: LibrarySection = .home {
        didSet { searchText = "" }
    }
    var searchText = ""
    let libraryDirectory: String
    let steam: SteamModel
    let epic: EpicModel
    let gog: GogModel
    /// The in-game overlay, over whichever storefront's game runs.
    let overlay: GameOverlayController
    /// Every storefront's Transfers, in their one queue.
    let downloads: DownloadsModel
    /// The storefront's store, open over the whole window.
    var presentedStore: StorePresentation?
    private(set) var activeFriendsAccount: FriendsAccountIdentity?
    /// The game whose page Home opened, while its storefront still shows it.
    var homeGame: HomeGame.ID?
    /// The storefronts whose saved session is still restoring since launch,
    /// so Home tells a library on its way from a signed-out one.
    private(set) var restoringStorefronts = Set(LibrarySection.storefronts)
    var appearance: AppAppearance {
        didSet { defaults.set(appearance.rawValue, forKey: "appearance") }
    }
    /// Whether every storefront installs a game's newer build as soon as it
    /// is published, rather than at the game's next launch.
    var updatesGamesAutomatically: Bool {
        didSet {
            defaults.set(updatesGamesAutomatically, forKey: Self.automaticUpdatesKey)
            applyAutomaticUpdates()
        }
    }
    static let automaticUpdatesKey = "obelisk.games.updateAutomatically"

    /// The last library of every storefront account.
    @ObservationIgnored let libraryCache: LibraryCache
    @ObservationIgnored private let defaults: UserDefaults

    /// The test host's models register no Shift-Tab and open no overlay.
    init(directory: URL = AppIdentity.libraryDirectory, defaults: UserDefaults = .standard, steam: SteamModel? = nil,
         epic: EpicModel? = nil, gog: GogModel? = nil, downloads: DownloadsModel? = nil,
         overlay: GameOverlayEnvironment = ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil
            ? .live : .inert) {
        libraryDirectory = directory.path
        self.defaults = defaults
        appearance = AppAppearance(rawValue: defaults.string(forKey: "appearance") ?? "system") ?? .system
        updatesGamesAutomatically = defaults.object(forKey: Self.automaticUpdatesKey) as? Bool ?? true
        // Every storefront's library shares one cache, as every cache of the
        // directory does, `LaunchHistory.live`'s included.
        libraryCache = LibraryCache(directory: directory.path)
        self.steam = steam ?? SteamModel(directory: directory, libraryCache: libraryCache)
        self.epic = epic ?? EpicModel(directory: directory, libraryCache: libraryCache)
        self.gog = gog ?? GogModel(directory: directory, libraryCache: libraryCache)
        self.overlay = GameOverlayController(steam: self.steam, epic: self.epic, gog: self.gog, environment: overlay)
        self.steam.overlay = self.overlay
        self.downloads = downloads ?? DownloadsModel()
        self.downloads.progress = { [weak self] item in self?.downloadTransfer(item) }
        applyAutomaticUpdates()
    }

    /// A queued Transfer's progress, from its storefront's installations.
    func downloadTransfer(_ item: DownloadItem) -> GameTransfer? {
        switch item.storefront {
        case .steam: steam.installations.transfer(named: item.game)
        case .epic: epic.installations.transfer(named: item.game)
        case .gog: gog.installations.transfer(named: item.game)
        }
    }

    /// What the download queue shows of a queued Transfer's game.
    func downloadGame(_ item: DownloadItem) -> DownloadGame {
        switch item.storefront {
        case .steam: steam.downloadGame(item, cover: SteamView.cover, artwork: SteamView.heroArtwork)
        case .epic: epic.downloadGame(item, cover: EpicView.cover, artwork: EpicView.heroArtwork)
        case .gog: gog.downloadGame(item, cover: GogView.cover, artwork: GogView.heroArtwork)
        }
    }

    /// Opens a queued Transfer's game page, in its storefront's library.
    func openDownload(_ item: DownloadItem) {
        downloads.isPresented = false
        switch item.storefront {
        case .steam: open(item, in: steam)
        case .epic: open(item, in: epic)
        case .gog: open(item, in: gog)
        }
    }

    private func open<Model: LibraryStorefront>(_ item: DownloadItem, in model: Model) {
        guard let id = Model.Store.game(item.game), let game = model.games.first(where: { $0.id == id }) else { return }
        selection = item.storefront.section
        model.open(game)
    }

    private func applyAutomaticUpdates() {
        steam.installations.updatesAutomatically = updatesGamesAutomatically
        epic.installations.updatesAutomatically = updatesGamesAutomatically
        gog.installations.updatesAutomatically = updatesGamesAutomatically
    }

    /// The Cloud or Rosetta question shown over the whole app, whichever tab is
    /// open. One sheet at a time, in a fixed order.
    enum StorefrontPrompt: Equatable {
        case steamRosetta, steamCloud, epicRosetta, epicCloud, gogRosetta
    }

    var presentedPrompt: StorefrontPrompt? {
        if steam.rosetta.prompt != nil { return .steamRosetta }
        if steam.cloud.promptPresented { return .steamCloud }
        if epic.rosetta.prompt != nil { return .epicRosetta }
        if epic.cloud.promptPresented { return .epicCloud }
        if gog.rosetta.prompt != nil { return .gogRosetta }
        return nil
    }

    /// The storefront whose friends `storeID` names; GOG shows none.
    private func friendsModel(storeID: String?) -> (any FriendsStorefront)? {
        switch storeID {
        case "steam": steam
        case "epic": epic
        default: nil
        }
    }

    var presentedFriends: FriendsPresentationState? {
        guard let activeFriendsAccount else { return nil }
        let state = friendsModel(storeID: activeFriendsAccount.storeID)?.friends
        return state?.account == activeFriendsAccount ? state : nil
    }

    func presentFriends(storeID: String) {
        guard let model = friendsModel(storeID: storeID), let state = model.friends else { return }
        activeFriendsAccount = state.account
        for other in [steam, epic] as [any FriendsStorefront] where other !== model { other.closeFriends() }
    }

    func closeFriends(storeID: String? = nil) {
        if let storeID, activeFriendsAccount?.storeID != storeID { return }
        activeFriendsAccount = nil
        steam.closeFriends()
        epic.closeFriends()
    }

    func validateFriendsAccount() {
        if activeFriendsAccount != nil && presentedFriends == nil { closeFriends() }
    }

    func retryPresentedFriends() {
        friendsModel(storeID: activeFriendsAccount?.storeID)?.retryFriends()
    }

    func signInPresentedFriends() {
        friendsModel(storeID: activeFriendsAccount?.storeID)?.signOut()
        closeFriends()
    }

    /// Restores every storefront's saved session, so Home shows each library
    /// from launch. A signed-out store offers its sign-in on its own tab.
    func restoreStorefronts() async {
        let restorations = [
            Task { await steam.start(signIn: false); restoringStorefronts.remove(.steam) },
            Task { await epic.start(); restoringStorefronts.remove(.epic) },
            Task { await gog.start(); restoringStorefronts.remove(.gog) },
        ]
        for restoration in restorations { await restoration.value }
    }

    /// Reloads the open storefront's library; Home's, every storefront's.
    func refresh() {
        switch selection {
        case .home:
            steam.refresh()
            epic.refresh()
            gog.refresh()
        case .steam: steam.refresh()
        case .epic: epic.refresh()
        case .gog: gog.refresh()
        case .settings: break
        }
    }
}
