import Foundation
import Observation

@MainActor
@Observable
final class GogModel {
    private(set) var accountName: String?
    private(set) var avatarURL: URL?
    private(set) var games: [GogGame] = [] {
        didSet { libraryChanges &+= 1 }
    }
    /// Each played game's playtime in minutes, by product, once GOG answered.
    private var playtimes: [String: UInt64] = [:] {
        didSet { libraryChanges &+= 1 }
    }
    /// Changes with the games, their playtimes and the account whose
    /// launches they read.
    private var libraryChanges = 0
    private(set) var isLoadingPlaytimes = false
    private var playtimesFailed = false

    let gameSessions: GogGameSessions
    /// The Rust installations of an account a session signed in: its id, the
    /// language whose depots it installs, and the file its receipts keep.
    typealias MakeInstallations = @MainActor (any GogSessionProtocol, String, String,
                                              any StorefrontSessionsProtocol, URL) -> any InstallationsBackend<GogGames>
    @ObservationIgnored private let makeInstallations: MakeInstallations
    /// GOG keeps no Cloud saves for Obelisk: this stays empty.
    let cloud: GogCloudModel
    let rosetta: GogRosettaModel
    let installations: GogInstallations

    let achievements = GameAchievementsModel(storeID: "gog", describe: GogModel.message)
    private(set) var selectedGame: GogGame?
    private(set) var selectedGameDetails: GogGameDetails?
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
    var availableSorts: [StorefrontLibrarySort] { StorefrontLibrarySort.allCases }
    var rememberSession: Bool {
        get { rememberedSession.rememberSession }
        set { rememberedSession.rememberSession = newValue }
    }

    @ObservationIgnored private let rememberedSession: GogRememberedSession
    /// Where the account's last library is kept between launches. The
    /// account's session keeps every library it loads there.
    @ObservationIgnored private let libraryCache: LibraryCache?
    /// The account whose cached library shows while its session restores.
    private var cachedAccount: String? {
        didSet { if cachedAccount != oldValue { libraryChanges &+= 1 } }
    }
    @ObservationIgnored private let directory: URL
    @ObservationIgnored private let language: String
    @ObservationIgnored private let authorize: @MainActor () async throws -> String
    @ObservationIgnored private let createSession: @Sendable (String) async throws -> any GogSessionProtocol
    @ObservationIgnored private let restoreSession: @Sendable (GogCredentials) async throws -> any GogSessionProtocol
    @ObservationIgnored private var session: (any GogSessionProtocol)?

    /// Its launches are the games' last played dates.
    @ObservationIgnored private var accountID: String? {
        didSet { if accountID != oldValue { libraryChanges &+= 1 } }
    }
    @ObservationIgnored private var started = false
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var authenticationTask: Task<Void, Never>?
    @ObservationIgnored private var libraryTask: Task<Void, Never>?
    @ObservationIgnored private var detailsTask: Task<Void, Never>?
    @ObservationIgnored private var detailsGeneration = 0
    @ObservationIgnored private var playtimeTask: Task<Void, Never>?
    @ObservationIgnored let libraryOrdering = LibraryOrdering<GogGame>()

    init(
        directory: URL = AppIdentity.libraryDirectory,
        keychain: any CredentialStore<GogCredentials> = GogKeychain(),
        authorize: @escaping @MainActor () async throws -> String = { try await WebAuthentication.authorize(.gog) },
        signIn: (@Sendable (String) async throws -> any GogSessionProtocol)? = nil,
        restore: (@Sendable (GogCredentials) async throws -> any GogSessionProtocol)? = nil,
        language: String = Locale.preferredLanguages.first ?? "en-US",
        gameSessions: GogGameSessions = .shared,
        libraryCache: LibraryCache? = nil,
        installations makeInstallations: MakeInstallations? = nil
    ) {
        self.directory = directory
        self.libraryCache = libraryCache
        self.rememberedSession = GogRememberedSession(store: keychain)
        self.authorize = authorize
        self.createSession = signIn ?? { [libraryCache] in
            try await GogSession.signIn(authorizationCode: $0, library: libraryCache)
        }
        self.restoreSession = restore ?? { [libraryCache] in try await GogSession.restore(credentials: $0, library: libraryCache) }
        self.language = language
        self.gameSessions = gameSessions
        let locations = ObeliskLocations.of(library: directory)
        self.makeInstallations = makeInstallations ?? { session, account, language, sessions, receipts in
            guard let session = session as? GogSession, let sessions = sessions as? StorefrontSessions else {
                return NoInstallations<GogGames>()
            }
            return RustInstallations<GogGames>(try! AccountInstallations.gog(
                session: session, userId: account, language: language, sessions: sessions,
                receipts: receipts.path, locations: locations
            ))
        }
        self.rosetta = GogRosettaModel(sessions: gameSessions)
        self.cloud = GogCloudModel(sessions: gameSessions)
        self.installations = GogInstallations(sessions: gameSessions, cloud: cloud)
        installations.selection = { [weak self] in self?.installationSelection }
    }

    // Always read the observable account, even before the unobserved session exists.
    var isConnected: Bool { accountName != nil && session != nil }

    /// Whether the library shows: once connected, or from the cache while the
    /// saved session restores.
    var showsLibrary: Bool { isConnected || (cachedAccount != nil && isRestoring) }

    /// Shows the account's last library at once, while its session restores.
    private func showCachedLibrary(of account: String) {
        guard games.isEmpty, let cached = libraryCache?.gogLibrary(userId: account), !cached.isEmpty else { return }
        cachedAccount = account
        games = cached.map(\.game)
        playtimes = Dictionary(cached.compactMap { entry in entry.playtimeMinutes.map { (entry.game.productId, $0) } },
                               uniquingKeysWith: max)
    }

    /// Hides a cached library whose session did not restore: another account
    /// may sign in next. The cache keeps it for a later restoration.
    private func dropCachedLibrary() {
        guard cachedAccount != nil else { return }
        cachedAccount = nil
        games = []
        playtimes = [:]
    }

    /// The game on screen, for its installation actions.
    private var installationSelection: InstallationSelection<GogGames>? {
        guard let game = selectedGame else { return nil }
        return InstallationSelection(game: game.productId, platform: game.preferredPlatform, offer: game.offer)
    }

    func cloudTitle(for productID: String?) -> String { L10n.string("ui.store.gog") }

    var gameSessionMessage: String? {
        gameSessions.message(for: selectedGame?.productId, of: accountID,
                             otherGameRunning: L10n.string("model.gog.otherGameRunning"))
    }

    func title(of game: GogGame) -> String { game.title }

    /// Nil until GOG answered, or when the game was never played.
    func playtime(of game: GogGame) -> Duration? {
        playtimes[game.productId].flatMap { $0 > 0 ? .seconds(Int64(clamping: $0) * 60) : nil }
    }

    /// When this account last launched the game from Obelisk: GOG tells no
    /// last played date of its own.
    func lastPlayed(of game: GogGame) -> Date? {
        // The cached library's account, while its session restores.
        (accountID ?? cachedAccount).flatMap {
            gameSessions.history.lastLaunch(of: game.productId, account: $0, in: GogGames.self)
        }
    }

    func platform(of game: GogGame) -> GamePlatform? { .preferred(macos: game.macos, windows: game.windows) }

    var libraryRevision: Int { libraryChanges &+ gameSessions.history.revision }

    /// Whether the filter or the order reads the games' playtimes.
    private var usesPlaytime: Bool { filter == .played || sort == .playtime }

    var pendingMetadata: String? {
        usesPlaytime && isLoadingPlaytimes ? L10n.string("ui.library.loading_playtime") : nil
    }

    var missingMetadata: String? {
        usesPlaytime && playtimesFailed && !isLoadingPlaytimes ? L10n.string("ui.library.playtime_unavailable") : nil
    }

    func start() async {
        guard !started && !isSigningOut && !isConnecting && !isRestoring && !isConnected else { return }
        beginRestore()
        await authenticationTask?.value
    }

    private func beginRestore() {
        started = true
        isRestoring = true
        canRetryRestore = false
        let current = generation
        authenticationTask = Task { await restoreSavedSession(generation: current) }
    }

    func retryRestore() {
        guard !isConnected && !isRestoring && !isConnecting && !isSigningOut else { return }
        errorMessage = nil
        beginRestore()
    }

    private func restoreSavedSession(generation current: Int) async {
        defer { if current == generation { isRestoring = false } }
        let credentials: GogCredentials?
        do {
            credentials = try await rememberedSession.load()
        } catch {
            guard current == generation else { return }
            storageWarning = error.localizedDescription
            canRetryRestore = true
            return
        }
        guard current == generation && !Task.isCancelled, let credentials else { return }
        showCachedLibrary(of: credentials.userId)
        do {
            let connected = try await restoreSession(credentials)
            await accept(connected, generation: current)
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            dropCachedLibrary()
            errorMessage = Self.message(error)
            if case GogError.Unauthorized = error {
                await rememberedSession.forget().value
            } else {
                canRetryRestore = true
            }
        }
    }

    func signIn() {
        guard !isConnected && !isRestoring && !isConnecting && !isSigningOut else { return }
        generation += 1
        let current = generation
        isConnecting = true
        canRetryRestore = false
        errorMessage = nil
        authenticationTask = Task {
            defer { if current == generation { isConnecting = false } }
            do {
                let code = try await authorize()
                guard current == generation && !Task.isCancelled else { return }
                let connected = try await createSession(code)
                await accept(connected, generation: current)
            } catch is CancellationError {
                // Closing the GOG window is a normal cancellation.
            } catch {
                guard current == generation && !Task.isCancelled else { return }
                errorMessage = Self.message(error)
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

    private func accept(_ connected: any GogSessionProtocol, generation current: Int) async {
        guard current == generation && !Task.isCancelled else {
            await disconnectAbandonedSession(connected)
            return
        }
        do {
            let credentials = try await connected.credentials()
            guard current == generation && !Task.isCancelled else {
                await disconnectAbandonedSession(connected)
                return
            }
            if cachedAccount != credentials.userId {
                games = []
                playtimes = [:]
            }
            cachedAccount = nil
            session = connected
            accountID = credentials.userId
            accountName = credentials.username.isEmpty ? credentials.userId : credentials.username
            avatarURL = connected.avatarUrl().flatMap(URL.init(string:))
            isConnecting = false
            isRestoring = false
            errorMessage = nil
            isLoadingGames = true
            await rememberedSession.activate(connected, accountID: credentials.userId)
            guard current == generation else { return }
            rosetta.connect(accountId: credentials.userId)
            let backend = makeInstallations(connected, credentials.userId, language,
                                            gameSessions.core,
                                            receiptsFile(in: directory, storefront: "gog", account: credentials.userId))
            installations.connect(account: credentials.userId, backend: backend, port: installationPort())

            await reloadGames(connected, generation: current)
        } catch {
            await disconnectAbandonedSession(connected)
            guard current == generation && !Task.isCancelled else { return }
            errorMessage = Self.message(error)
        }
    }

    private func disconnectAbandonedSession(_ connected: any GogSessionProtocol) async {
        // The auth task may already be cancelled. Give the disconnection its own task.
        await Task { try? await connected.disconnect() }.value
    }

    func refresh() {
        guard let session, !isLoadingGames else { return }
        // Set synchronously so two clicks in the same run loop cannot overlap.
        isLoadingGames = true
        errorMessage = nil
        let current = generation
        libraryTask = Task { await reloadGames(session, generation: current) }
    }

    private func reloadGames(_ connected: any GogSessionProtocol, generation current: Int) async {
        defer { if current == generation { isLoadingGames = false } }
        do {
            let loaded = try await connected.games()
            guard current == generation && !Task.isCancelled else { return }
            games = loaded
            reloadPlaytimes(connected, of: loaded, generation: current)
            if let selectedGame {
                if let updated = loaded.first(where: { $0.productId == selectedGame.productId }) { open(updated) }
                else { closeGame() }
            }
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            if case GogError.Unauthorized = error { signOut() }
            errorMessage = Self.message(error)
        }
    }

    /// GOG answers each game's playtime on its own, once the games are known.
    /// It only orders and filters the library, so the library never waits
    /// for it.
    private func reloadPlaytimes(_ connected: any GogSessionProtocol, of games: [GogGame], generation current: Int) {
        playtimeTask?.cancel()
        isLoadingPlaytimes = true
        playtimesFailed = false
        let products = games.map(\.productId)
        playtimeTask = Task {
            do {
                let loaded = try await connected.playtimes(productIds: products)
                guard current == generation && !Task.isCancelled else { return }
                playtimes = Dictionary(loaded.map { ($0.productId, $0.minutes) }, uniquingKeysWith: max)
            } catch {
                guard current == generation && !Task.isCancelled else { return }
                playtimesFailed = true
            }
            isLoadingPlaytimes = false
        }
    }

    func signOut() {
        guard !isSigningOut else { return }
        if let account = accountID ?? cachedAccount {
            libraryCache?.forget(account: .gog(userId: account))
            gameSessions.history.forget(account: account, in: GogGames.self)
        }
        cachedAccount = nil
        generation += 1
        let current = generation
        let disconnected = session
        rosetta.connect(accountId: nil)
        installations.signedOut()
        cloud.signedOut()
        let removal = rememberedSession.forget()
        authenticationTask?.cancel()
        libraryTask?.cancel()
        authenticationTask = nil
        libraryTask = nil
        playtimeTask?.cancel()
        playtimeTask = nil
        isLoadingPlaytimes = false
        playtimesFailed = false
        playtimes = [:]
        session = nil
        accountID = nil
        accountName = nil
        avatarURL = nil
        games = []
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
            try? await disconnected?.disconnect()
        }
    }

    func open(_ game: GogGame) {
        closeGame()
        selectedGame = game
        reloadAchievements()
        guard let session else { return }
        let current = generation
        let selection = detailsGeneration
        detailsTask = Task { await loadDetails(game, session: session, generation: current, selection: selection) }
    }

    /// GOG's library already tells which builds a game ships, so the library
    /// downloads it without opening its page.
    func downloadFromLibrary(_ game: GogGame) {
        guard let offer = game.offer else { return }
        installations.install(offer, of: game.productId)
    }

    func canInstallFromLibrary(_ game: GogGame) -> Bool { game.offer != nil }

    func reloadAchievements() {
        guard let game = selectedGame, let session else { return }
        let locale = Locale(identifier: language).identifier(.bcp47)
        achievements.load {
            try await session.achievements(productId: game.productId, locale: locale).map(GameAchievement.init)
        }
    }

    func closeGame() {
        achievements.reset()
        detailsGeneration += 1
        detailsTask?.cancel()
        detailsTask = nil
        selectedGame = nil
        selectedGameDetails = nil
    }

    private func loadDetails(
        _ game: GogGame,
        session: any GogSessionProtocol,
        generation current: Int,
        selection: Int
    ) async {
        func isCurrent() -> Bool {
            current == generation && selection == detailsGeneration
                && selectedGame?.productId == game.productId && !Task.isCancelled
        }
        guard isCurrent() else { return }
        do {
            let details = try await session.gameDetails(
                productId: game.productId, platform: game.preferredPlatform, language: language
            )
            if isCurrent() { selectedGameDetails = details }
        } catch {
            // Secondary metadata must not prevent browsing the library.
        }
    }

    /// GOG's part of the installation actions. Every launch verifies its
    /// installation first, as Epic's does.
    private func installationPort() -> InstallationPort<GogGames> {
        InstallationPort(
            verifiesBeforeLaunch: { true },
            report: { [weak self] error in
                guard let self else { return }
                guard let error else { return errorMessage = nil }
                if case GogError.Unauthorized = error { signOut() }
                errorMessage = Self.message(error)
            },
            warn: { [weak self] in self?.storageWarning = $0 }
        )
    }

    static func message(_ error: Error) -> String {
        switch error {
        case GogError.Network: L10n.string("model.gog.network")
        case GogError.Unauthorized: L10n.string("model.gog.unauthorized")
        case GogError.InvalidCode: L10n.string("model.gog.invalidCode")
        case GogError.InvalidResponse: L10n.string("model.gog.invalidResponse")
        case let GogError.Service(message): L10n.message(message)
        case let browserError as WebAuthenticationError: browserError.localizedDescription
        case let failure as GameInstallationFailure: failure.localizedDescription
        default: L10n.string("model.gog.failure")
        }
    }
}

extension GogGame: Identifiable {
    /// The native build first.
    var preferredPlatform: String { macos ? GogGames.nativePlatform : "windows" }
    public var id: String { productId }

    /// What GOG can install of the game: its native build first.
    var offer: GameOffer? {
        guard macos || windows else { return nil }
        return .gog(productId: productId, title: title, platform: preferredPlatform)
    }
}
