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
    private(set) var activeFriendsAccount: FriendsAccountIdentity?
    /// The game whose page Home opened, while its storefront still shows it.
    var homeGame: HomeGame.ID?
    /// The storefronts whose saved session is still restoring since launch,
    /// so Home tells a library on its way from a signed-out one.
    private(set) var restoringStorefronts = Set(LibrarySection.storefronts)
    var appearance: AppAppearance {
        didSet { defaults.set(appearance.rawValue, forKey: "appearance") }
    }

    /// The last library of every storefront account.
    @ObservationIgnored let libraryCache: LibraryCache
    @ObservationIgnored private let defaults: UserDefaults

    /// The test host's models register no Shift-Tab and open no overlay.
    init(directory: URL = AppIdentity.libraryDirectory, defaults: UserDefaults = .standard, steam: SteamModel? = nil,
         epic: EpicModel? = nil, gog: GogModel? = nil,
         overlay: GameOverlayEnvironment = ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil
            ? .live : .inert) {
        libraryDirectory = directory.path
        self.defaults = defaults
        appearance = AppAppearance(rawValue: defaults.string(forKey: "appearance") ?? "system") ?? .system
        // Every storefront's library shares one cache, as every cache of the
        // directory does, `LaunchHistory.live`'s included.
        libraryCache = LibraryCache(directory: directory.path)
        self.steam = steam ?? SteamModel(directory: directory, preferences: defaults, libraryCache: libraryCache)
        self.epic = epic ?? EpicModel(directory: directory, libraryCache: libraryCache)
        self.gog = gog ?? GogModel(directory: directory, libraryCache: libraryCache)
        self.overlay = GameOverlayController(steam: self.steam, epic: self.epic, gog: self.gog, environment: overlay)
        self.steam.overlay = self.overlay
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
