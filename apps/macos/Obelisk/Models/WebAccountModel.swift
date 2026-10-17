import Foundation
import Observation

/// A game of a storefront whose account signs in on the storefront's web
/// page. Its id names it in receipts, Game sessions and the Library cache.
protocol WebGame: Identifiable, Equatable, Sendable where ID == String {
    var title: String { get }
    var macos: Bool { get }
    var windows: Bool { get }
    /// How the storefront names the build Obelisk installs first: the native one.
    var preferredPlatform: String { get }
    /// What the storefront can install of the game; nil when it ships no build.
    var offer: GameOffer? { get }
    /// What the game tells that `sort` orders it by, among its storefront's
    /// `librarySorts`; nil when it tells nothing.
    func libraryValue(for sort: StorefrontLibrarySort) -> Double?
}

/// What lives as long as an account's connection besides its library, such
/// as Epic's friends. Account revocation stops it before the session goes.
@MainActor
protocol AccountPresence: AnyObject {
    associatedtype Session
    init()
    /// The session became the connected account's.
    func connect(_ session: Session, account: String)
    /// Stops its network work at once: nothing the revoked session sends later shows.
    func revoke()
}

/// The presence of a storefront that has none.
@MainActor
final class NoPresence<Session>: AccountPresence {
    init() {}
    func connect(_ session: Session, account: String) {}
    func revoke() {}
}

/// A storefront whose account signs in on its own web page and whose Rust
/// session rotates its refresh credentials: Epic and GOG. Its enum is the
/// adapter: it makes the session's calls, reads its Library cache and names
/// things. Each text is an `L10n.string` of a literal key, so the
/// localization check sees every storefront's.
///
/// `SendableMetatype`: the model's `@Sendable` closures call `Store`'s statics.
protocol WebStorefront: InstallationStorefront, SendableMetatype where GameID == String, AccountID == String {
    /// The Rust session, as its UniFFI protocol.
    associatedtype Session: Sendable
    associatedtype Credentials: Sendable
    associatedtype Game: WebGame
    /// What the game page reads besides the library entry.
    associatedtype Details: Equatable & Sendable
    associatedtype Presence: AccountPresence = NoPresence<Session> where Presence.Session == Session

    /// Names the storefront in receipts files and achievements.
    static var storeID: String { get }
    /// The orders its library offers, as their menu lists them.
    static var librarySorts: [StorefrontLibrarySort] { get }
    static var webSignIn: WebSignIn { get }
    /// The storefront's store, opened in Obelisk.
    static var storeURL: URL { get }
    @MainActor static var sessions: GameSessions<Self> { get }
    static func keychain() -> any CredentialStore<Credentials>

    static func signIn(code: String, library: LibraryCache?) async throws -> Session
    static func restore(_ credentials: Credentials, library: LibraryCache?) async throws -> Session
    static func accountID(_ credentials: Credentials) -> String
    /// The name the storefront shows. The account shows its id when it is empty.
    static func accountName(_ credentials: Credentials) -> String
    static func avatar(of session: Session) -> URL?
    /// How the remembered session saves each rotation of the session's credentials.
    static func rotating(_ session: Session) -> RotatingSession<Credentials>
    /// Revokes the session. Throws when the storefront did not confirm it.
    static func disconnect(_ session: Session) async throws

    static func games(of session: Session) async throws -> [Game]
    /// The playtime of each played game of `games`, by its id.
    static func playtimes(of session: Session, games: [Game]) async throws -> [String: Duration]
    /// The share of each of `games`' achievements the account unlocked, in
    /// the player's `language`, by game id: the Library cache's for a game
    /// not played since they were read, else the storefront's, which the
    /// session then keeps. A game without achievements, or that the
    /// storefront did not answer for, has none; it throws when no game could
    /// be told. Asked once the playtimes loaded.
    static func achievementShares(of games: [Game], session: Session, language: String) async throws -> [String: Double]
    /// The account's Library cache, with each game's playtime and achievement
    /// share when they were known.
    static func cachedLibrary(in cache: LibraryCache, account: String)
        -> [(game: Game, playtime: Duration?, achievements: Double?)]
    /// `language` is the player's, such as "fr-FR".
    static func details(of game: Game, session: Session, language: String) async throws -> Details
    /// The game's achievements in the player's `language`: the Library
    /// cache's while the game was not played since they were read, unless
    /// `fresh`, else the storefront's.
    static func achievements(of game: Game, session: Session, language: String,
                             fresh: Bool) async throws -> [GameAchievement]
    /// The account's installations in Rust; none for a session that is not Rust's.
    @MainActor static func installations(of session: Session, account: String, language: String,
                                          sessions: any StorefrontSessionsProtocol, receipts: URL,
                                          locations: ObeliskLocations) -> any InstallationsBackend<Self>
    /// An address that opens the storefront's site signed in as the
    /// session's account and goes on to `storeURL`; nil when its site has none.
    static func webSignInURL(session: Session) async throws -> URL?

    /// The storefront refused the account: a restore forgets the saved
    /// session, and any later call signs the account out.
    static func isUnauthorized(_ error: Error) -> Bool

    static var sessionNotSaved: String { get }
    /// Added to the storage warning when the sign-out's revocation fails.
    static var revocationUnconfirmed: String { get }
    static var otherGameRunning: String { get }
}

extension WebStorefront {
    static func avatar(of session: Session) -> URL? { nil }
    static func webSignInURL(session: Session) async throws -> URL? { nil }
}

/// One account of a storefront that signs in on its web page: its sign-in
/// and remembered session, its library (from the Library cache while the
/// session restores), the game on screen, and its installations, Cloud,
/// Rosetta prompts and presence.
///
/// Every sign-in, cancellation and sign-out starts a new generation. An
/// answer from an older one changes nothing, and a session it produced is
/// revoked in its own task. One authentication runs at a time, and none
/// starts before a sign-out's Keychain removal ends.
@MainActor
@Observable
final class WebAccountModel<Store: WebStorefront> {
    typealias Game = Store.Game
    /// The Rust installations of an account a session signed in: its id, the
    /// player's language, and the file its receipts keep.
    typealias MakeInstallations = @MainActor (Store.Session, String, String,
                                              any StorefrontSessionsProtocol, URL) -> any InstallationsBackend<Store>

    private(set) var accountName: String?
    private(set) var avatarURL: URL?
    var games: [Game] { library.games }
    private(set) var isLoadingPlaytimes = false
    private var playtimesFailed = false

    /// What the library shows and whose launches date it.
    private struct Library: Equatable {
        var games: [Game] = []
        /// Each played game's playtime, by id, once the storefront answered.
        var playtimes: [String: Duration] = [:]
        /// The account whose cached library shows while its session restores.
        var cachedAccount: String?
        /// Its launches are the games' last played dates.
        var accountID: String?
    }
    /// Every change moves `libraryRevision`, so the library's orders never go stale.
    private var library = Library() {
        didSet { libraryChanges &+= 1 }
    }
    private var libraryChanges = 0

    let gameSessions: GameSessions<Store>
    let cloud: CloudModel<Store>
    let rosetta: RosettaModel<GameSessions<Store>>
    let installations: GameInstallations<Store>
    let achievements = GameAchievementsModel(storeID: Store.storeID, describe: Store.describe)
    /// What lives with the connection, such as Epic's friends.
    let presence: Store.Presence

    private(set) var selectedGame: Game?
    private(set) var selectedGameDetails: Store.Details?
    private(set) var isLoadingGameDetails = false
    private(set) var isRestoring = false
    private(set) var isConnecting = false
    private(set) var isLoadingGames = false
    private(set) var isSigningOut = false
    private(set) var canRetryRestore = false
    var errorMessage: String?
    var storageWarning: String? {
        get { rememberedSession.storageWarning }
        set { rememberedSession.storageWarning = newValue }
    }
    var searchText = ""
    var filter: StorefrontLibraryFilter = .all
    var availableFilters: [StorefrontLibraryFilter] { [.all, .installed, .played, .macOS] }
    var sort: StorefrontLibrarySort = .name
    var availableSorts: [StorefrontLibrarySort] { Store.librarySorts }
    /// The share of each game's achievements the account unlocked, by game
    /// id, which loads after the playtimes.
    let libraryRecords = LibraryRecords<String>()
    var rememberSession: Bool {
        get { rememberedSession.rememberSession }
        set { rememberedSession.rememberSession = newValue }
    }

    @ObservationIgnored private let rememberedSession: RememberedSession<Store.Credentials>
    /// Where the account's last library is kept between launches. The
    /// account's session keeps every library it loads there.
    @ObservationIgnored private let libraryCache: LibraryCache?
    @ObservationIgnored private let directory: URL
    @ObservationIgnored private let language: String
    @ObservationIgnored private let authorize: @MainActor () async throws -> String
    @ObservationIgnored private let createSession: @Sendable (String) async throws -> Store.Session
    @ObservationIgnored private let restoreSession: @Sendable (Store.Credentials) async throws -> Store.Session
    @ObservationIgnored private let makeInstallations: MakeInstallations
    @ObservationIgnored private var session: Store.Session?
    @ObservationIgnored private var started = false
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var authenticationTask: Task<Void, Never>?
    @ObservationIgnored private var libraryTask: Task<Void, Never>?
    @ObservationIgnored private var playtimeTask: Task<Void, Never>?
    @ObservationIgnored private var detailsTask: Task<Void, Never>?
    @ObservationIgnored let libraryOrdering = LibraryOrdering<Game>()

    init(
        directory: URL = AppIdentity.libraryDirectory,
        keychain: any CredentialStore<Store.Credentials> = Store.keychain(),
        authorize: @escaping @MainActor () async throws -> String = { try await WebAuthentication.authorize(Store.webSignIn) },
        signIn: (@Sendable (String) async throws -> Store.Session)? = nil,
        restore: (@Sendable (Store.Credentials) async throws -> Store.Session)? = nil,
        language: String = Locale.preferredLanguages.first ?? "en-US",
        gameSessions: GameSessions<Store> = Store.sessions,
        libraryCache: LibraryCache? = nil,
        installations makeInstallations: MakeInstallations? = nil
    ) {
        self.directory = directory
        self.libraryCache = libraryCache
        rememberedSession = RememberedSession(keychain: keychain, accountID: Store.accountID,
                                              notSaved: Store.sessionNotSaved)
        self.authorize = authorize
        createSession = signIn ?? { [libraryCache] in try await Store.signIn(code: $0, library: libraryCache) }
        restoreSession = restore ?? { [libraryCache] in try await Store.restore($0, library: libraryCache) }
        self.language = language
        self.gameSessions = gameSessions
        let locations = ObeliskLocations.of(library: directory)
        self.makeInstallations = makeInstallations ?? { session, account, language, sessions, receipts in
            Store.installations(of: session, account: account, language: language, sessions: sessions,
                                receipts: receipts, locations: locations)
        }
        rosetta = RosettaModel(sessions: gameSessions)
        cloud = CloudModel(sessions: gameSessions)
        installations = GameInstallations(sessions: gameSessions, cloud: cloud)
        presence = Store.Presence()
        installations.selection = { [weak self] in self?.installationSelection }
    }

    // Always read the observable account, even before the unobserved session exists.
    var isConnected: Bool { accountName != nil && session != nil }

    /// Whether the library shows: once connected, or from the cache while the
    /// saved session restores.
    var showsLibrary: Bool { isConnected || (library.cachedAccount != nil && isRestoring) }

    var libraryRevision: Int { libraryChanges &+ gameSessions.history.revision }

    /// No authentication or sign-out runs, and no account is connected.
    private var isIdle: Bool { !isConnected && !isRestoring && !isConnecting && !isSigningOut }

    /// Whether the filter or the order reads the games' playtimes.
    private var usesPlaytime: Bool { filter == .played || sort == .playtime }

    var pendingMetadata: String? {
        usesPlaytime && isLoadingPlaytimes ? L10n.string("ui.library.loading_playtime") : nil
    }

    var missingMetadata: String? {
        usesPlaytime && playtimesFailed ? L10n.string("ui.library.playtime_unavailable") : nil
    }

    var gameSessionMessage: String? {
        gameSessions.message(for: selectedGame?.id, of: library.accountID, otherGameRunning: Store.otherGameRunning)
    }

    func title(of game: Game) -> String { game.title }

    /// Nil until the storefront answered, or when the game was never played.
    func playtime(of game: Game) -> Duration? {
        library.playtimes[game.id].flatMap { $0 > .zero ? $0 : nil }
    }

    /// When this account last launched the game from Obelisk: neither
    /// storefront records a last played date of its own.
    func lastPlayed(of game: Game) -> Date? {
        // The cached library's account, while its session restores.
        (library.accountID ?? library.cachedAccount).flatMap {
            gameSessions.history.lastLaunch(of: game.id, account: $0, in: Store.self)
        }
    }

    func platform(of game: Game) -> GamePlatform? { .preferred(macos: game.macos, windows: game.windows) }

    func libraryValue(of game: Game, for sort: StorefrontLibrarySort) -> Double? {
        sort == .achievements ? libraryRecords.value(for: sort, of: game.id) : game.libraryValue(for: sort)
    }

    /// The other orders read only the games.
    func valuesRevision(for sort: StorefrontLibrarySort) -> Int {
        sort == .achievements ? libraryRecords.revision(for: sort) : 0
    }

    // MARK: Account

    /// Restores the saved session once. Returns when the restoration and its
    /// first library load end.
    func start() async {
        guard !started && isIdle else { return }
        beginRestore()
        await authenticationTask?.value
    }

    func retryRestore() {
        guard isIdle else { return }
        errorMessage = nil
        beginRestore()
    }

    /// Opens the storefront's page and exchanges its code. Closing the page is not an error.
    func signIn() {
        guard isIdle else { return }
        generation += 1
        let current = generation
        isConnecting = true
        canRetryRestore = false
        errorMessage = nil
        authenticationTask = Task {
            defer { if current == generation { isConnecting = false } }
            do {
                let code = try await authorize()
                guard isCurrent(current) else { return }
                let connected = try await createSession(code)
                await accept(connected, generation: current)
            } catch is CancellationError {
                // Closing the storefront's window is a normal cancellation.
            } catch {
                guard isCurrent(current) else { return }
                errorMessage = Store.describe(error)
            }
        }
    }

    func cancelSignIn() {
        guard isConnecting || isRestoring else { return }
        generation += 1
        authenticationTask?.cancel()
        authenticationTask = nil
        isConnecting = false
        isRestoring = false
        canRetryRestore = false
        errorMessage = nil
    }

    /// Ignored while a load runs.
    func refresh() {
        guard let session, !isLoadingGames else { return }
        // Set synchronously so two clicks in the same run loop cannot overlap.
        isLoadingGames = true
        errorMessage = nil
        let current = generation
        libraryTask = Task { await reloadGames(session, generation: current) }
    }

    /// The storefront's store: signed in as the account when its storefront
    /// can sign it in, else as its site remembers.
    func storePage() async -> WebPageStart {
        var url = Store.storeURL
        if let session, let signedIn = try? await Store.webSignInURL(session: session) { url = signedIn }
        return WebPageStart(url: url, dataStore: Store.webSignIn.store.dataStore)
    }

    /// Account revocation: forgets the Library cache, the launches and the
    /// saved session, revokes the presence, installations and Cloud, then
    /// revokes the session once the Keychain removal ends.
    func signOut() {
        guard !isSigningOut else { return }
        Store.webSignIn.store.forgetSite()
        if let account = library.accountID ?? library.cachedAccount {
            libraryCache?.forget(account: Store.account(account))
            gameSessions.history.forget(account: account, in: Store.self)
        }
        generation += 1
        let current = generation
        let disconnected = session
        rosetta.connect(accountId: nil)
        installations.signedOut()
        cloud.signedOut()
        let removal = rememberedSession.forget()
        presence.revoke()
        authenticationTask?.cancel()
        authenticationTask = nil
        libraryTask?.cancel()
        libraryTask = nil
        playtimeTask?.cancel()
        playtimeTask = nil
        session = nil
        library = Library()
        libraryRecords.reset()
        accountName = nil
        avatarURL = nil
        isLoadingPlaytimes = false
        playtimesFailed = false
        closeGame()
        searchText = ""
        filter = .all
        errorMessage = nil
        storageWarning = nil
        isConnecting = false
        isLoadingGames = false
        isRestoring = false
        isSigningOut = true
        canRetryRestore = false
        Task {
            await removal.value
            if current == generation { isSigningOut = false }
            guard let disconnected else { return }
            do { try await Store.disconnect(disconnected) }
            catch {
                guard current == generation else { return }
                let warning = Store.revocationUnconfirmed
                storageWarning = storageWarning.map { $0 + " " + warning } ?? warning
            }
        }
    }

    /// Whether the running task still answers for the latest sign-in, cancellation or sign-out.
    private func isCurrent(_ current: Int) -> Bool { current == generation && !Task.isCancelled }

    private func beginRestore() {
        started = true
        isRestoring = true
        canRetryRestore = false
        let current = generation
        authenticationTask = Task { await restoreSavedSession(generation: current) }
    }

    private func restoreSavedSession(generation current: Int) async {
        defer { if current == generation { isRestoring = false } }
        let credentials: Store.Credentials?
        do {
            credentials = try await rememberedSession.load()
        } catch {
            guard current == generation else { return }
            storageWarning = error.localizedDescription
            canRetryRestore = true
            return
        }
        guard isCurrent(current), let credentials else { return }
        showCachedLibrary(of: Store.accountID(credentials))
        do {
            let connected = try await restoreSession(credentials)
            await accept(connected, generation: current)
        } catch {
            guard isCurrent(current) else { return }
            dropCachedLibrary()
            errorMessage = Store.describe(error)
            if Store.isUnauthorized(error) {
                await rememberedSession.forget().value
            } else {
                canRetryRestore = true
            }
        }
    }

    private func accept(_ connected: Store.Session, generation current: Int) async {
        guard isCurrent(current) else {
            await disconnectAbandonedSession(connected)
            return
        }
        let rotating = Store.rotating(connected)
        do {
            let credentials = try await rotating.credentials()
            guard isCurrent(current) else {
                await disconnectAbandonedSession(connected)
                return
            }
            let account = Store.accountID(credentials)
            let name = Store.accountName(credentials)
            var accepted = library.cachedAccount == account ? library : Library()
            if library.cachedAccount != account { libraryRecords.reset() }
            accepted.cachedAccount = nil
            accepted.accountID = account
            if library != accepted { library = accepted }
            session = connected
            accountName = name.isEmpty ? account : name
            avatarURL = Store.avatar(of: connected)
            presence.connect(connected, account: account)
            isConnecting = false
            isRestoring = false
            errorMessage = nil
            isLoadingGames = true
            await rememberedSession.activate(rotating: rotating, accountID: account)
            guard current == generation else { return }
            rosetta.connect(accountId: account)
            let backend = makeInstallations(connected, account, language, gameSessions.core,
                                            receiptsFile(in: directory, storefront: Store.storeID, account: account))
            installations.connect(account: account, backend: backend, port: installationPort())

            await reloadGames(connected, generation: current)
        } catch {
            await disconnectAbandonedSession(connected)
            guard isCurrent(current) else { return }
            errorMessage = Store.describe(error)
        }
    }

    private func disconnectAbandonedSession(_ connected: Store.Session) async {
        // The authentication task may already be cancelled: Account revocation gets its own task.
        await Task { try? await Store.disconnect(connected) }.value
    }

    // MARK: Library

    /// Shows the account's last library at once, while its session restores.
    private func showCachedLibrary(of account: String) {
        guard library.games.isEmpty, let libraryCache else { return }
        let cached = Store.cachedLibrary(in: libraryCache, account: account)
        guard !cached.isEmpty else { return }
        library.cachedAccount = account
        library.games = cached.map { $0.game }
        library.playtimes = Dictionary(cached.compactMap { entry in entry.playtime.map { (entry.game.id, $0) } },
                                       uniquingKeysWith: max)
        libraryRecords.show(Dictionary(cached.compactMap { entry in entry.achievements.map { (entry.game.id, $0) } },
                                       uniquingKeysWith: max), for: .achievements)
    }

    /// Hides a cached library whose session did not restore: another account
    /// may sign in next. The cache keeps it for a later restoration.
    private func dropCachedLibrary() {
        guard library.cachedAccount != nil else { return }
        library = Library()
        libraryRecords.reset()
    }

    private func reloadGames(_ connected: Store.Session, generation current: Int) async {
        defer { if current == generation { isLoadingGames = false } }
        // The playtimes reload with the games: a refresh shows them loading at once.
        playtimeTask?.cancel()
        playtimeTask = nil
        isLoadingPlaytimes = true
        playtimesFailed = false
        do {
            let loaded = try await Store.games(of: connected)
            guard isCurrent(current) else { return }
            if library.games != loaded { library.games = loaded }
            reloadPlaytimes(connected, of: loaded, generation: current)
            if let selectedGame {
                if let updated = loaded.first(where: { $0.id == selectedGame.id }) { open(updated) }
                else { closeGame() }
            }
        } catch {
            guard isCurrent(current) else { return }
            // Without the games, no playtime was asked: offer the retry.
            isLoadingPlaytimes = false
            playtimesFailed = true
            report(error)
        }
    }

    /// Asked once the games are known, which GOG's per-game requests need.
    /// Playtimes only order and filter the library, so the library never
    /// waits for them. The achievements follow: the session reads again
    /// those of the games played since.
    private func reloadPlaytimes(_ connected: Store.Session, of games: [Game], generation current: Int) {
        playtimeTask = Task {
            do {
                let loaded = try await Store.playtimes(of: connected, games: games)
                guard isCurrent(current) else { return }
                library.playtimes = loaded
            } catch {
                guard isCurrent(current) else { return }
                playtimesFailed = true
            }
            isLoadingPlaytimes = false
            // A refresh cancelled this load: its own reads the new games.
            guard !Task.isCancelled else { return }
            let language = language
            libraryRecords.load(.achievements) {
                try await Store.achievementShares(of: games, session: connected, language: language)
            }
        }
    }

    // MARK: Game page

    func open(_ game: Game) {
        closeGame()
        selectedGame = game
        load(achievementsOf: game, into: achievements, fresh: false)
        installations.refreshCloudIndicator()
        guard let session else { return }
        isLoadingGameDetails = true
        detailsTask = Task { await loadDetails(game, session: session) }
    }

    func closeGame() {
        achievements.reset()
        detailsTask?.cancel()
        detailsTask = nil
        selectedGame = nil
        selectedGameDetails = nil
        isLoadingGameDetails = false
    }

    /// The library already tells which builds a game ships, so it downloads
    /// one without opening the game's page.
    func libraryOffer(of game: Game) -> GameOffer? { game.offer }

    /// Reads the game's achievements from the storefront again, as the
    /// page's retry asks.
    func reloadAchievements() {
        guard let game = selectedGame else { return }
        load(achievementsOf: game, into: achievements, fresh: true)
    }

    /// Reads the achievements of the library's game `id` from the storefront
    /// into `model`, as the overlay asks for the running game.
    func loadAchievements(of id: Game.ID, into model: GameAchievementsModel) {
        guard let game = games.first(where: { $0.id == id }) else { return }
        load(achievementsOf: game, into: model, fresh: true)
    }

    /// The Library cache's achievements while the game was not played since,
    /// unless `fresh`.
    private func load(achievementsOf game: Game, into model: GameAchievementsModel, fresh: Bool) {
        guard let session else { return }
        let language = language
        model.load { try await Store.achievements(of: game, session: session, language: language, fresh: fresh) }
    }

    /// Every change of the game on screen, and the sign-out, cancels it in `closeGame`.
    private func loadDetails(_ game: Game, session: Store.Session) async {
        defer { if !Task.isCancelled { isLoadingGameDetails = false } }
        guard !Task.isCancelled else { return }
        do {
            let details = try await Store.details(of: game, session: session, language: language)
            if !Task.isCancelled { selectedGameDetails = details }
        } catch {
            // Secondary metadata must not prevent browsing the library.
        }
    }

    /// Shows a failed call. One the storefront refused signs the account out.
    private func report(_ error: Error) {
        if Store.isUnauthorized(error) { signOut() }
        errorMessage = Store.describe(error)
    }

    /// The game on screen, for its installation actions.
    private var installationSelection: InstallationSelection<Store>? {
        guard let game = selectedGame else { return nil }
        return InstallationSelection(game: game.id, platform: game.preferredPlatform, offer: game.offer)
    }

    /// The account's part of the installation actions.
    private func installationPort() -> InstallationPort<Store> {
        InstallationPort(
            report: { [weak self] error in
                guard let self else { return }
                guard let error else { return errorMessage = nil }
                report(error)
            },
            warn: { [weak self] in self?.storageWarning = $0 }
        )
    }
}
