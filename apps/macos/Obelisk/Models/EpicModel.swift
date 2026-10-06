import Foundation
import Observation

/// Epic library entries are unique per namespace and app name across platforms.
func epicLibraryID(namespace: String, appName: String) -> String { "\(namespace):\(appName)" }
/// The game a library ID names: an Epic namespace has no colon.
func epicGameID(_ libraryID: String) -> (namespace: String, appName: String)? {
    guard let colon = libraryID.firstIndex(of: ":") else { return nil }
    return (String(libraryID[..<colon]), String(libraryID[libraryID.index(after: colon)...]))
}

@MainActor
@Observable
final class EpicModel {
    private(set) var friends: FriendsPresentationState?
    private(set) var isFriendsPresented = false
    private(set) var accountName: String?
    private(set) var games: [EpicGame] = [] {
        didSet {
            libraryOrderCache.removeAll()
            filteredLibraryCache = nil
        }
    }
    /// Each played game's playtime in seconds, by app name, once Epic answered.
    private var playtimes: [String: UInt64] = [:] {
        didSet {
            // Only the playtime order reads them.
            libraryOrderCache[.playtime] = nil
            filteredLibraryCache = nil
        }
    }
    private(set) var isLoadingPlaytimes = false
    private var playtimesFailed = false

    let gameSessions: EpicGameSessions
    /// The Rust installations of an account a session signed in: its id, the
    /// language games launch in, and the file its receipts keep.
    typealias MakeInstallations = @MainActor (any EpicSessionProtocol, String, String,
                                              any StorefrontSessionsProtocol, URL) -> any InstallationsBackend<EpicGames>
    @ObservationIgnored private let makeInstallations: MakeInstallations
    let cloud: EpicCloudModel
    let rosetta: EpicRosettaModel
    let installations: EpicInstallations

    let achievements = GameAchievementsModel(storeID: "epic", describe: EpicModel.message)
    private(set) var selectedGame: EpicGame?
    private(set) var selectedGameDetails: EpicGameDetails?
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
    // Epic records each game's playtime, but no last played date.
    var availableSorts: [StorefrontLibrarySort] { [.name, .playtime] }
    var rememberSession: Bool {
        get { rememberedSession.rememberSession }
        set { rememberedSession.rememberSession = newValue }
    }

    @ObservationIgnored private let rememberedSession: EpicRememberedSession
    /// Where the account's last library is kept between launches.
    @ObservationIgnored private let libraryCache: LibraryCache?
    /// The account whose cached library shows while its session restores.
    private var cachedAccount: String?
    @ObservationIgnored private let directory: URL
    @ObservationIgnored private let launchLanguage: String
    @ObservationIgnored private let authorize: @MainActor () async throws -> String
    @ObservationIgnored private let createSession: @Sendable (String) async throws -> any EpicSessionProtocol
    @ObservationIgnored private let restoreSession: @Sendable (EpicCredentials) async throws -> any EpicSessionProtocol
    @ObservationIgnored private var session: (any EpicSessionProtocol)?
    @ObservationIgnored private var friendsTask: Task<Void, Never>?
    @ObservationIgnored private var friendsObservationGeneration = 0
    @ObservationIgnored private var wakeObserver: SystemWakeObservation?

    @ObservationIgnored private var accountID: String?
    @ObservationIgnored private var started = false
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var authenticationTask: Task<Void, Never>?
    @ObservationIgnored private var libraryTask: Task<Void, Never>?
    @ObservationIgnored private var detailsTask: Task<Void, Never>?
    @ObservationIgnored private var detailsGeneration = 0
    @ObservationIgnored private var playtimeTask: Task<Void, Never>?
    @ObservationIgnored private var libraryOrderCache: [StorefrontLibrarySort: [EpicGame]] = [:]
    @ObservationIgnored private var filteredLibraryCache: (
        query: String, sort: StorefrontLibrarySort, filter: StorefrontLibraryFilter, installed: Set<String>,
        games: [EpicGame]
    )?

    init(
        directory: URL = AppIdentity.libraryDirectory,
        keychain: any EpicCredentialStore = EpicKeychain(),
        authorize: @escaping @MainActor () async throws -> String = { try await EpicWebAuthentication.authorize() },
        signIn: @escaping @Sendable (String) async throws -> any EpicSessionProtocol = { try await EpicSession.signIn(authorizationCode: $0) },
        restore: @escaping @Sendable (EpicCredentials) async throws -> any EpicSessionProtocol = { try await EpicSession.restore(credentials: $0) },
        language: String = Locale.preferredLanguages.first ?? "en",
        gameSessions: EpicGameSessions = .shared,
        libraryCache: LibraryCache? = nil,
        installations makeInstallations: MakeInstallations? = nil
    ) {
        self.directory = directory
        self.libraryCache = libraryCache
        self.rememberedSession = EpicRememberedSession(store: keychain)
        self.authorize = authorize
        self.createSession = signIn
        self.restoreSession = restore
        self.launchLanguage = language
        self.gameSessions = gameSessions
        let locations = ObeliskLocations.of(library: directory)
        self.makeInstallations = makeInstallations ?? { session, account, language, sessions, receipts in
            guard let session = session as? EpicSession, let sessions = sessions as? StorefrontSessions else {
                return NoInstallations<EpicGames>()
            }
            return RustInstallations<EpicGames>(try! AccountInstallations.epic(
                session: session, accountId: account, language: language, sessions: sessions,
                receipts: receipts.path, locations: locations
            ))
        }
        self.rosetta = EpicRosettaModel(sessions: gameSessions)
        self.cloud = EpicCloudModel(sessions: gameSessions)
        self.installations = EpicInstallations(sessions: gameSessions, cloud: cloud)
        wakeObserver = SystemWakeObservation { [weak self] in
            Task { @MainActor [weak self] in self?.retryFriends() }
        }
        installations.selection = { [weak self] in self?.installationSelection }
    }

    deinit {
        friendsTask?.cancel()
    }

    // Always read the observable account, even before the unobserved session exists.
    var isConnected: Bool { accountName != nil && session != nil }

    /// Whether the library shows: once connected, or from the cache while the
    /// saved session restores.
    var showsLibrary: Bool { isConnected || (cachedAccount != nil && isRestoring) }

    /// Shows the account's last library at once, while its session restores.
    private func showCachedLibrary(of account: String) {
        guard games.isEmpty,
              let cached = libraryCache?.games(EpicLibraryEntry.self, in: .epic, account: account),
              !cached.isEmpty else { return }
        cachedAccount = account
        games = cached.map(\.game)
        playtimes = Dictionary(cached.compactMap { entry in entry.playtimeSeconds.map { (entry.appName, $0) } },
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

    /// Saves the library as it now shows, playtimes included.
    private func saveLibrary() {
        guard let libraryCache, let accountID else { return }
        libraryCache.save(games.map { EpicLibraryEntry($0, playtimeSeconds: playtimes[$0.appName]) },
                          in: .epic, account: accountID)
    }

    /// The game on screen, for its installation actions.
    private var installationSelection: InstallationSelection<EpicGames>? {
        guard let game = selectedGame else { return nil }
        return InstallationSelection(game: game.libraryID, platform: game.preferredPlatform, offer: game.offer)
    }

    func syncCloud() { installations.syncCloud() }

    func refreshCloudIndicator() {
        guard let game = selectedGame else { return }
        cloud.refreshIndicators(for: [game.libraryID])
    }

    /// Names the game a Cloud question or banner is about.
    func cloudTitle(for libraryID: String?) -> String {
        let title = L10n.string("ui.epic.cloud.title")
        guard let libraryID else { return title }
        let name = games.first { $0.libraryID == libraryID }?.title
            ?? installations.receipts.first { $0.gameID == libraryID }?.name
            ?? libraryID
        return "\(title) · \(name)"
    }

    private var selectedGameSession: EpicGameSessions.Session? {
        guard let active = gameSessions.active, active.account == accountID,
              active.game == selectedGame?.libraryID else { return nil }
        return active
    }

    var gameSessionMessage: String? {
        if gameSessions.isRecovering || gameSessions.active?.reconnecting == true {
            return L10n.string("model.epic.recoveringSession")
        }
        if let error = gameSessions.recoveryError { return error }
        if gameSessions.active != nil && selectedGameSession == nil {
            return L10n.string("model.epic.otherGameRunning")
        }
        return nil
    }

    /// Search and filters reuse the library's name order.
    var filteredGames: [EpicGame] {
        // Observe dependencies even when the cached result is reused.
        let source = games
        let playtimes = playtimes
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let order = sort
        let selectedFilter = filter
        let installedLibraryIDs = selectedFilter == .installed ? Set(installations.receipts.map(\.gameID)) : []
        if let cached = filteredLibraryCache, cached.query == query, cached.sort == order,
           cached.filter == selectedFilter, cached.installed == installedLibraryIDs {
            return cached.games
        }
        let ordered: [EpicGame]
        if let cached = libraryOrderCache[order] {
            ordered = cached
        } else {
            ordered = Self.order(source, by: order, playtimes: playtimes)
            libraryOrderCache[order] = ordered
        }
        let filtered = query.isEmpty && selectedFilter == .all ? ordered : ordered.filter { game in
            guard query.isEmpty || game.title.localizedStandardContains(query) else { return false }
            switch selectedFilter {
            case .all: return true
            case .installed: return installedLibraryIDs.contains(game.libraryID)
            case .played: return (playtimes[game.appName] ?? 0) > 0
            case .macOS: return game.macos
            }
        }
        filteredLibraryCache = (query, order, selectedFilter, installedLibraryIDs, filtered)
        return filtered
    }

    /// The most played games first for `.playtime`, then by name. Epic has no
    /// last played date, so `.lastPlayed` orders by name.
    static func order(_ games: [EpicGame], by sort: StorefrontLibrarySort, playtimes: [String: UInt64]) -> [EpicGame] {
        games.sorted {
            if sort == .playtime {
                let first = playtimes[$0.appName] ?? 0, second = playtimes[$1.appName] ?? 0
                if first != second { return first > second }
            }
            let comparison = $0.title.localizedStandardCompare($1.title)
            return comparison == .orderedSame ? $0.libraryID < $1.libraryID : comparison == .orderedAscending
        }
    }

    /// The game's playtime in seconds, once Epic answered; nil when it was
    /// never played.
    func playtime(of game: EpicGame) -> UInt64? { playtimes[game.appName] }

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
        let credentials: EpicCredentials?
        do {
            credentials = try await rememberedSession.load()
        } catch {
            guard current == generation else { return }
            storageWarning = error.localizedDescription
            canRetryRestore = true
            return
        }
        guard current == generation && !Task.isCancelled, let credentials else { return }
        showCachedLibrary(of: credentials.accountId)
        do {
            let connected = try await restoreSession(credentials)
            await accept(connected, generation: current)
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            dropCachedLibrary()
            errorMessage = Self.message(error)
            if case EpicError.Unauthorized = error {
                await rememberedSession.forget().value
            } else {
                canRetryRestore = true
            }
        }
    }

    func signIn() {
        beginSignIn(authorization: authorize)
    }

    func signIn(input: String) {
        beginSignIn { try epicAuthorizationCode(input: input) }
    }

    private func beginSignIn(authorization: @escaping @MainActor () async throws -> String) {
        guard !isConnected && !isRestoring && !isConnecting && !isSigningOut else { return }
        generation += 1
        let current = generation
        isConnecting = true
        canRetryRestore = false
        errorMessage = nil
        authenticationTask = Task {
            defer { if current == generation { isConnecting = false } }
            do {
                let code = try await authorization()
                guard current == generation && !Task.isCancelled else { return }
                let connected = try await createSession(code)
                await accept(connected, generation: current)
            } catch is CancellationError {
                // Closing the Epic window is a normal cancellation.
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

    private func accept(_ connected: any EpicSessionProtocol, generation current: Int) async {
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
            if cachedAccount != credentials.accountId {
                games = []
                playtimes = [:]
            }
            cachedAccount = nil
            session = connected
            accountID = credentials.accountId
            accountName = credentials.displayName.isEmpty ? credentials.accountId : credentials.displayName
            friends = .epic(accountID: credentials.accountId)
            observeFriends(connected, account: credentials.accountId, generation: current)
            isConnecting = false
            isRestoring = false
            errorMessage = nil
            isLoadingGames = true
            await rememberedSession.activate(connected, accountID: credentials.accountId)
            guard current == generation else { return }
            rosetta.connect(accountId: credentials.accountId)
            let backend = makeInstallations(connected, credentials.accountId, launchLanguage,
                                            gameSessions.core, receiptsFile(for: credentials.accountId))
            installations.connect(account: credentials.accountId, backend: backend, port: installationPort())

            await reloadGames(connected, generation: current)
        } catch {
            await disconnectAbandonedSession(connected)
            guard current == generation && !Task.isCancelled else { return }
            errorMessage = Self.message(error)
        }
    }

    private func disconnectAbandonedSession(_ connected: any EpicSessionProtocol) async {
        // The auth task may already be cancelled. Give revocation its own task.
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

    private func reloadGames(_ connected: any EpicSessionProtocol, generation current: Int) async {
        defer { if current == generation { isLoadingGames = false } }
        reloadPlaytimes(connected, generation: current)
        do {
            let loaded = try await connected.games()
            guard current == generation && !Task.isCancelled else { return }
            games = loaded
            saveLibrary()
            if let selectedGame {
                if let updated = loaded.first(where: { $0.libraryID == selectedGame.libraryID }) { open(updated) }
                else { closeGame() }
            }
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            if case EpicError.Unauthorized = error { signOut() }
            errorMessage = Self.message(error)
        }
    }

    /// Epic answers every game's playtime in one request. It only orders and
    /// filters the library, so the library never waits for it.
    private func reloadPlaytimes(_ connected: any EpicSessionProtocol, generation current: Int) {
        playtimeTask?.cancel()
        isLoadingPlaytimes = true
        playtimesFailed = false
        playtimeTask = Task {
            do {
                let loaded = try await connected.playtimes()
                guard current == generation && !Task.isCancelled else { return }
                playtimes = Dictionary(loaded.map { ($0.appName, $0.seconds) }, uniquingKeysWith: max)
                saveLibrary()
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
            libraryCache?.forget(.epic, account: account)
            gameSessions.history.forget(account: account, in: EpicGames.self)
        }
        cachedAccount = nil
        generation += 1
        let current = generation
        let disconnected = session
        rosetta.connect(accountId: nil)
        installations.signedOut()
        cloud.signedOut()
        let removal = rememberedSession.forget()
        friendsObservationGeneration += 1
        friendsTask?.cancel()
        friendsTask = nil
        friends = nil
        isFriendsPresented = false
        authenticationTask?.cancel()
        libraryTask?.cancel()
        authenticationTask = nil
        libraryTask = nil
        playtimeTask?.cancel()
        playtimeTask = nil
        session = nil
        accountID = nil
        accountName = nil
        games = []
        playtimes = [:]
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
            do { try await disconnected?.disconnect() }
            catch {
                guard current == generation else { return }
                let warning = L10n.string("model.epic.revocationUnconfirmed")
                storageWarning = storageWarning.map { $0 + " " + warning } ?? warning
            }
        }
    }

    func openFriends() {
        guard friends != nil, isConnected else { return }
        isFriendsPresented = true
    }

    func closeFriends() { isFriendsPresented = false }

    /// Replaces only the social socket. The OAuth session and downloads stay alive.
    func retryFriends() {
        guard let session, let accountID, isConnected,
              friends?.connection != .authenticationRequired else { return }
        observeFriends(session, account: accountID, generation: generation, reconnect: true)
    }

    private func observeFriends(_ connected: any EpicSessionProtocol, account: String, generation current: Int, reconnect: Bool = false) {
        friendsObservationGeneration += 1
        friendsTask?.cancel()
        let observation = friendsObservationGeneration
        friendsTask = Task { [weak self] in
            guard !Task.isCancelled else { return }
            await withTaskCancellationHandler {
                if reconnect { await connected.reconnectFriends() }
                else { await connected.startFriends() }
                var revision: UInt64 = 0
                while !Task.isCancelled {
                    let snapshot = await connected.nextFriends(afterRevision: revision)
                    guard let self, current == self.generation, account == self.accountID,
                          observation == self.friendsObservationGeneration, !Task.isCancelled else { return }
                    guard snapshot.accountId == account else { return }
                    revision = snapshot.revision
                    if snapshot.connection == .stopped { return }
                    self.friends?.apply(snapshot)
                }
            } onCancel: { connected.stopFriends() }
        }
    }

    func open(_ game: EpicGame) {
        closeGame()
        selectedGame = game
        reloadAchievements()
        refreshCloudIndicator()
        guard let session else { return }
        isLoadingGameDetails = true
        let current = generation
        let selection = detailsGeneration
        detailsTask = Task { await loadDetails(game, session: session, generation: current, selection: selection) }
    }

    /// When this account last launched the game from Obelisk: Epic records
    /// no last played date of its own.
    func lastLaunch(of game: EpicGame) -> Date? {
        accountID.flatMap { gameSessions.history.lastLaunch(of: game.libraryID, account: $0, in: EpicGames.self) }
    }

    /// The game the library shows first: the last one launched from Obelisk,
    /// else the most played.
    var featuredGame: EpicGame? {
        if let launched = games.compactMap({ game in lastLaunch(of: game).map { (game, $0) } }).max(by: { $0.1 < $1.1 }) {
            return launched.0
        }
        return games.filter { (playtimes[$0.appName] ?? 0) > 0 }
            .max { (playtimes[$0.appName] ?? 0) < (playtimes[$1.appName] ?? 0) }
    }

    /// Launches the game's native installation first.
    func launchFromLibrary(_ game: EpicGame) {
        guard let receipt = installations.receipt(for: game.libraryID, preferring: "Mac") else { return }
        installations.launch(receipt)
    }

    /// Epic's library already tells which builds a game ships, so the library
    /// downloads it without opening its page.
    func downloadFromLibrary(_ game: EpicGame) {
        guard let offer = game.offer else { return }
        installations.install(offer, of: game.libraryID)
    }

    func canInstallFromLibrary(_ game: EpicGame) -> Bool { game.offer != nil }

    func reloadAchievements() {
        guard let game = selectedGame, let session else { return }
        let locale = Locale(identifier: launchLanguage).language.languageCode?.identifier ?? "en"
        achievements.load {
            try await session.achievements(namespace: game.namespace, locale: locale).map(GameAchievement.init)
        }
    }

    func closeGame() {
        achievements.reset()
        detailsGeneration += 1
        detailsTask?.cancel()
        detailsTask = nil
        selectedGame = nil
        selectedGameDetails = nil
        isLoadingGameDetails = false
    }

    private func loadDetails(
        _ game: EpicGame,
        session: any EpicSessionProtocol,
        generation current: Int,
        selection: Int
    ) async {
        func isCurrent() -> Bool {
            current == generation && selection == detailsGeneration
                && selectedGame?.libraryID == game.libraryID && !Task.isCancelled
        }
        defer { if isCurrent() { isLoadingGameDetails = false } }
        guard isCurrent() else { return }
        do {
            let details = try await session.gameDetails(
                namespace: game.namespace,
                catalogItemId: game.catalogItemId,
                appName: game.appName,
                platform: game.preferredPlatform
            )
            if isCurrent() { selectedGameDetails = details }
        } catch {
            // Secondary metadata must not prevent browsing the library.
        }
    }

    private func receiptsFile(for account: String) -> URL {
        // Account identifiers are opaque. Encode them before using them in a file name.
        let component = account.utf8.map { String(format: "%02x", $0) }.joined()
        return directory.appending(path: "epic-installations-\(component).json")
    }

    /// Epic's part of the installation actions.
    private func installationPort() -> InstallationPort<EpicGames> {
        InstallationPort(
            verifiesBeforeLaunch: { true },
            report: { [weak self] error in
                guard let self else { return }
                guard let error else { return errorMessage = nil }
                if case EpicError.Unauthorized = error { signOut() }
                errorMessage = Self.message(error)
            },
            warn: { [weak self] in self?.storageWarning = $0 }
        )
    }

    static func message(_ error: Error) -> String {
        switch error {
        case EpicError.Network: L10n.string("model.epic.network")
        case EpicError.Unauthorized: L10n.string("model.epic.unauthorized")
        case EpicError.InvalidCode: L10n.string("model.epic.invalidCode")
        case EpicError.InvalidResponse: L10n.string("model.epic.invalidResponse")
        case let EpicError.Service(message): L10n.message(message)
        case let browserError as EpicWebAuthenticationError: browserError.localizedDescription
        case let failure as GameInstallationFailure: failure.localizedDescription
        default: L10n.string("model.epic.failure")
        }
    }
}

/// What the library cache keeps of an Epic game: everything Epic returned for
/// it, beside the fields the library reads.
struct EpicLibraryEntry: LibraryCacheEntry, Equatable {
    var id: String
    var appName: String
    var namespace: String
    var catalogItemId: String
    var title: String
    var description: String
    var imageUrl: String?
    var headerImageUrl: String?
    var developer: String
    var windows: Bool
    var macos: Bool
    /// Nil until Epic told the playtime, or when the game was never played.
    var playtimeSeconds: UInt64?
    /// Its catalog items, library records and assets, as Epic returned them.
    var metadata: JSONValue?

    init(_ game: EpicGame, playtimeSeconds: UInt64?) {
        id = game.libraryID
        appName = game.appName
        namespace = game.namespace
        catalogItemId = game.catalogItemId
        title = game.title
        description = game.description
        imageUrl = game.imageUrl
        headerImageUrl = game.headerImageUrl
        developer = game.developer
        windows = game.windows
        macos = game.macos
        self.playtimeSeconds = playtimeSeconds
        metadata = JSONValue(json: game.metadata)
    }

    var game: EpicGame {
        EpicGame(appName: appName, namespace: namespace, catalogItemId: catalogItemId, title: title,
                 description: description, imageUrl: imageUrl, headerImageUrl: headerImageUrl, developer: developer,
                 windows: windows, macos: macos, metadata: metadata?.json)
    }
}

extension EpicGame: Identifiable {
    var preferredPlatform: String { macos ? "Mac" : "Windows" }
    var libraryID: String { epicLibraryID(namespace: namespace, appName: appName) }
    public var id: String { libraryID }

    /// What Epic can install of the game: its native build first.
    var offer: GameOffer? {
        guard macos || windows else { return nil }
        return .epic(namespace: namespace, catalogItemId: catalogItemId, appName: appName, title: title,
                     platform: preferredPlatform)
    }
}

