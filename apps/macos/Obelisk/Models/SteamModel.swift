import AppKit
import Foundation
import Observation

/// How a Steam login is started. Both entry points go through one port, so
/// the model never holds a concrete `SteamLogin` and tests can drive sign-in
/// without opening a Steam HTTP session.
enum SteamLoginStart: Sendable, Equatable {
    case qr
    case credentials(account: String, password: String)
}

@MainActor
@Observable
final class SteamModel {
    private(set) var displayName: String?
    private(set) var avatarURL: URL?
    private(set) var friends: FriendsPresentationState?
    var isFriendsPresented = false
    /// Shows the game's invite picker over the running game.
    @ObservationIgnored weak var overlay: GameOverlayController?
    private(set) var games: [SteamGame] = [] {
        didSet { libraryChanges &+= 1 }
    }
    /// Changes with the games and the account whose launches they read.
    private var libraryChanges = 0
    private(set) var isRestoring = false
    private(set) var isConnecting = false
    private(set) var isLoadingGames = false
    private(set) var isSubmittingCredentials = false
    private(set) var isSubmittingGuard = false
    var qrURL: String? { challenge?.qrUrl }
    private(set) var challenge: SteamChallenge?
    private(set) var isCredentialChallenge = false
    private(set) var qrError: String?
    var errorMessage: String?
    var storageWarning: String?
    var rememberSession = true
    var searchText = ""
    var sort: StorefrontLibrarySort = .lastPlayed
    var availableSorts: [StorefrontLibrarySort] { StorefrontLibrarySort.allCases }
    var filter: StorefrontLibraryFilter = .all
    var availableFilters: [StorefrontLibraryFilter] { StorefrontLibraryFilter.allCases }
    var guardKind: SteamGuardKind = .deviceConfirmation

    private(set) var selectedGame: SteamGame? {
        // Every path that opens, closes or replaces the selection invalidates
        // the panel here, so no caller has to remember to.
        didSet { if selectedGame?.appId != oldValue?.appId { achievements.reset() } }
    }
    private(set) var details: SteamGameDetails?
    let achievements = GameAchievementsModel(storeID: "steam", describe: SteamModel.message)
    private(set) var news: [SteamNews] = []
    private(set) var isLoadingDetails = false
    private(set) var isLoadingNews = false
    private(set) var detailsError: String?
    private(set) var newsError: String?
    var platform: String {
        if let details, details.appId == selectedGame?.appId { return details.preferredPlatform }
        // Installation receipts are available before remote details. Never use
        // the previous game's platform to present the newly selected game.
        return installations.receipts.contains { $0.gameID == selectedGame?.appId && $0.platform == SteamGames.nativePlatform }
            ? SteamGames.nativePlatform : "windows"
    }
    private(set) var language: String
    let installations: SteamInstallations

    @ObservationIgnored private let directory: URL
    @ObservationIgnored private let keychain: any SteamCredentialStore
    @ObservationIgnored private let connectSession: @Sendable (SteamCredentials) async throws -> any SteamSessionProtocol
    @ObservationIgnored private let loadNews: @Sendable (UInt32) async throws -> [SteamNews]
    @ObservationIgnored private let loadWebCookies: @Sendable (any SteamSessionProtocol) async throws -> [SteamWebCookie]
    /// The account's web sign-in, kept a while: its access token lasts a day.
    @ObservationIgnored private var webSignIn: (steamId: UInt64, date: Date, cookies: [HTTPCookie])?
    @ObservationIgnored private let beginLogin: @Sendable (SteamLoginStart) async throws -> any SteamLoginProtocol
    let gameSessions: SteamGameSessions
    /// The Rust installations of the account a session signed in, whose
    /// receipts a file keeps.
    typealias MakeInstallations = @MainActor (any SteamSessionProtocol, SteamCredentials,
                                              any StorefrontSessionsProtocol, URL)
        -> any InstallationsBackend<SteamGames>
    @ObservationIgnored private let makeInstallations: MakeInstallations
    let cloud: SteamCloudModel
    let rosetta: SteamRosettaModel
    let libraryPlatforms = SteamLibraryPlatforms()
    /// Chat and game invitations, through the session the friends list follows.
    let chat: SteamChatModel
    /// Friends' invitations into the running game.
    let invitations: SteamReceivedInvitations
    /// The in-game purchase the running game waits on.
    let purchase: SteamPurchaseAuthorization
    /// Where the account's last library is kept between launches. The
    /// account's session keeps every library it loads there.
    @ObservationIgnored private let libraryCache: LibraryCache?
    /// The account whose cached library shows while its session connects.
    private var cachedAccount: UInt64? {
        didSet { if cachedAccount != oldValue { libraryChanges &+= 1 } }
    }
    @ObservationIgnored private var session: (any SteamSessionProtocol)?
    @ObservationIgnored private var login: (any SteamLoginProtocol)?
    private var steamId: UInt64? { sessionCredentials?.steamId }
    /// The saved session's restoration; true when it ended without a newer
    /// sign-in or sign-out.
    @ObservationIgnored private var restoration: Task<Bool, Never>?
    /// Whether the QR sign-in was offered once the restoration ended.
    @ObservationIgnored private var offeredSignIn = false
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var detailGeneration = 0
    @ObservationIgnored private var detailsRequestGeneration = 0
    @ObservationIgnored private var loginTask: Task<Void, Never>?
    @ObservationIgnored private var profileTask: Task<Void, Never>?
    @ObservationIgnored private var friendsTask: Task<Void, Never>?
    @ObservationIgnored private var friendsObservationGeneration = 0
    /// The connected account; observed because `isConnected` derives from it.
    private var sessionCredentials: SteamCredentials? {
        didSet { if sessionCredentials?.steamId != oldValue?.steamId { libraryChanges &+= 1 } }
    }
    @ObservationIgnored private let reconnectDelay: @Sendable (Int) async throws -> Void
    @ObservationIgnored private var wakeObserver: SystemWakeObservation?
    @ObservationIgnored private var connectionTask: Task<any SteamSessionProtocol, Error>?
    @ObservationIgnored private var pollingActive = false
    /// The Game session last seen, to tell when one ends.
    @ObservationIgnored private var observedGameSession: SteamGameSessions.Session?
    /// The running game's session and the last overlay request it showed.
    @ObservationIgnored private var followedOverlay = SessionSequence()
    @ObservationIgnored let libraryOrdering = LibraryOrdering<SteamGame>()

    init(
        directory: URL,
        keychain: any SteamCredentialStore = SteamKeychain(),
        connect: (@Sendable (SteamCredentials) async throws -> any SteamSessionProtocol)? = nil,
        news: @escaping @Sendable (UInt32) async throws -> [SteamNews] = { try await steamNews(appId: $0) },
        webCookies: @escaping @Sendable (any SteamSessionProtocol) async throws -> [SteamWebCookie] = {
            try await $0.webCookies()
        },
        beginLogin: @escaping @Sendable (SteamLoginStart) async throws -> any SteamLoginProtocol = { start in
            switch start {
            case .qr:
                try await SteamLogin.beginQr()
            case let .credentials(account, password):
                try await SteamLogin.beginCredentials(accountName: account, password: password)
            }
        },
        localeIdentifier: String = Locale.preferredLanguages.first ?? Locale.current.identifier,
        reconnectDelay: @escaping @Sendable (Int) async throws -> Void = { attempt in
            try await Task.sleep(for: .seconds(1 << attempt))
        },
        gameSessions: SteamGameSessions = .shared,
        libraryCache: LibraryCache? = nil,
        installations makeInstallations: MakeInstallations? = nil,
        chat: SteamChatModel? = nil
    ) {
        self.directory = directory
        self.keychain = keychain
        self.connectSession = connect ?? { [libraryCache] in try await SteamSessionConnection.connect($0, library: libraryCache) }
        self.loadNews = news
        self.loadWebCookies = webCookies
        self.beginLogin = beginLogin
        self.language = steamLanguage(locale: localeIdentifier)
        self.reconnectDelay = reconnectDelay
        self.gameSessions = gameSessions
        self.libraryCache = libraryCache
        let locations = ObeliskLocations.of(library: directory)
        self.makeInstallations = makeInstallations ?? { session, credentials, sessions, receipts in
            guard let session = session as? SteamSession, let sessions = sessions as? StorefrontSessions else {
                return NoInstallations<SteamGames>()
            }
            return RustInstallations<SteamGames>(try! AccountInstallations.steam(
                session: session, credentials: credentials, sessions: sessions, receipts: receipts.path,
                locations: locations
            ))
        }
        self.cloud = SteamCloudModel(sessions: gameSessions)
        self.rosetta = SteamRosettaModel(sessions: gameSessions)
        self.installations = SteamInstallations(sessions: gameSessions, cloud: cloud)
        self.chat = chat ?? SteamChatModel()
        self.invitations = SteamReceivedInvitations(sessions: gameSessions)
        self.purchase = SteamPurchaseAuthorization(sessions: gameSessions)
        wakeObserver = SystemWakeObservation { [weak self] in
            Task { @MainActor [weak self] in self?.retryFriends() }
        }
        installations.selection = { [weak self] in self?.installationSelection }
        self.chat.gameName = { [weak self] in self?.gameName(appId: $0) }
        observeGameSessions()
    }

    deinit {
        loginTask?.cancel()
        friendsTask?.cancel()
        connectionTask?.cancel()
        session?.shutdown()
    }

    var isConnected: Bool { sessionCredentials != nil }

    var unreadCount: Int { chat.totalUnread }

    /// Whether the library shows: once connected, or from the cache while the
    /// saved session restores.
    var showsLibrary: Bool { isConnected || (cachedAccount != nil && isRestoring) }

    /// The game on screen, for its installation actions. Only remote details
    /// tell which builds the account can install.
    private var installationSelection: InstallationSelection<SteamGames>? {
        guard let game = selectedGame else { return nil }
        let details = details?.appId == game.appId && !isLoadingDetails ? details : nil
        let offer = details.flatMap { $0.offer(for: game, language: language) }
        return InstallationSelection(game: game.appId, platform: platform, offer: offer)
    }

    /// The active session when it belongs to the game on screen.
    private var selectedGameSession: SteamGameSessions.Session? {
        guard let active = gameSessions.active, active.game == selectedGame?.appId else { return nil }
        return active
    }

    var gameSessionMessage: String? {
        if gameSessions.isRecovering { return L10n.string("model.steam.recoveringSession") }
        if let error = gameSessions.recoveryError { return error }
        guard gameSessions.active != nil else { return nil }
        guard let active = selectedGameSession else { return L10n.string("model.steam.otherGameRunning") }
        if active.reconnecting { return L10n.string("model.steam.recoveringSession") }
        switch active.host {
        case .disconnected?: return L10n.string("model.steam.gameDisconnected")
        case .lost?: return L10n.string("model.steam.gameHostLost")
        default: return nil
        }
    }

    func title(of game: SteamGame) -> String { game.name }

    func playtime(of game: SteamGame) -> Duration? { game.playtime }

    /// When the game was last played: on Steam, or later from Obelisk.
    func lastPlayed(of game: SteamGame) -> Date? {
        // The cached library's account, while its session connects.
        let launched = (steamId ?? cachedAccount).flatMap {
            gameSessions.history.lastLaunch(of: game.appId, account: $0, in: SteamGames.self)
        }
        return [game.lastPlayedOnSteam, launched].compactMap { $0 }.max()
    }

    /// Nil until Steam told the game's platforms.
    func platform(of game: SteamGame) -> GamePlatform? {
        libraryPlatforms.platforms[game.appId].flatMap(GamePlatform.preferred(platforms:))
    }

    var libraryRevision: Int { libraryChanges &+ gameSessions.history.revision }

    var filterRevision: Int { libraryPlatforms.revision }

    var pendingMetadata: String? {
        filter == .macOS && libraryPlatforms.isLoading ? L10n.string("ui.filter.loading_platforms") : nil
    }

    var missingMetadata: String? {
        filter == .macOS && libraryPlatforms.hasFailures ? L10n.string("ui.filter.platforms_unavailable") : nil
    }

    /// Restores the saved session once, then offers the QR sign-in when none
    /// restored, unless `signIn` is false: Home restores every storefront at
    /// launch, and a signed-out Steam waits for its own tab to ask Steam for a
    /// QR code. Returns once the restoration ends.
    func start(signIn: Bool = true) async {
        if restoration == nil {
            isRestoring = true
            restoration = Task { await restore() }
        }
        guard await restoration?.value == true, signIn, !offeredSignIn else { return }
        offeredSignIn = true
        if !isConnected { beginQR() }
    }

    private func restore() async -> Bool {
        let current = generation
        do {
            if let credentials = try await keychain.load() {
                guard current == generation else { return false }
                showCachedLibrary(of: credentials.steamId)
                await connect(credentials, generation: current, persist: false)
            }
        } catch {
            guard current == generation else { return false }
            storageWarning = error.localizedDescription
        }
        guard current == generation else { return false }
        isRestoring = false
        if !isConnected { dropCachedLibrary() }
        return true
    }

    /// Shows the account's last library at once, while its session connects.
    private func showCachedLibrary(of account: UInt64) {
        guard games.isEmpty, let cached = libraryCache?.steamLibrary(steamId: account), !cached.isEmpty else { return }
        cachedAccount = account
        games = cached.map(\.game)
        libraryPlatforms.show(platforms: Dictionary(
            cached.compactMap { entry in entry.platforms.map { (entry.game.appId, $0) } },
            uniquingKeysWith: { first, _ in first }
        ))
    }

    /// Hides a cached library whose session did not restore: another account
    /// may sign in next. The cache keeps it for a later restoration.
    private func dropCachedLibrary() {
        guard cachedAccount != nil else { return }
        cachedAccount = nil
        games = []
        libraryPlatforms.reset()
    }

    func beginQR() {
        guard !isConnected && !isRestoring && !isConnecting else { return }
        generation += 1
        let current = generation
        loginTask?.cancel()
        login = nil
        qrError = nil
        challenge = nil
        isCredentialChallenge = false
        isSubmittingCredentials = false
        isSubmittingGuard = false
        pollingActive = false
        loginTask = Task {
            do {
                let request = try await beginLogin(.qr)
                guard current == generation && !Task.isCancelled else { return }
                login = request
                await poll(request, generation: current)
            } catch {
                guard current == generation && !Task.isCancelled else { return }
                qrError = Self.message(error)
            }
        }
    }

    func signIn(account: String, password: String) {
        guard !isConnecting && !isSubmittingCredentials && !isRestoring else { return }
        generation += 1
        let current = generation
        loginTask?.cancel()
        login = nil
        challenge = nil
        errorMessage = nil
        isCredentialChallenge = true
        isSubmittingCredentials = true
        pollingActive = false
        loginTask = Task {
            do {
                let request = try await beginLogin(.credentials(account: account, password: password))
                guard current == generation && !Task.isCancelled else { return }
                login = request
                isSubmittingCredentials = false
                await poll(request, generation: current)
            } catch {
                guard current == generation && !Task.isCancelled else { return }
                isSubmittingCredentials = false
                errorMessage = Self.message(error)
                isCredentialChallenge = false
                beginQR()
            }
        }
    }

    func submitGuard(_ code: String) {
        guard let login, !isSubmittingGuard, !isConnecting else { return }
        let current = generation
        let kind = guardKind
        isSubmittingGuard = true
        errorMessage = nil
        Task {
            do {
                try await login.submitGuard(code: code, kind: kind)
                guard current == generation && !isConnected else { return }
                isSubmittingGuard = false
                // Keep an in-flight poll: it may already be receiving the tokens.
                if !pollingActive { loginTask = Task { await poll(login, generation: current) } }
            } catch {
                guard current == generation && !isConnected else { return }
                isSubmittingGuard = false
                errorMessage = Self.message(error)
            }
        }
    }

    private func poll(_ request: any SteamLoginProtocol, generation current: Int) async {
        pollingActive = true
        defer { if current == generation { pollingActive = false } }
        do {
            var initial = true
            while current == generation && !Task.isCancelled {
                let snapshot = await request.challenge()
                guard current == generation && !Task.isCancelled else { return }
                challenge = snapshot
                if initial {
                    let methods = snapshot.confirmations.map(\.kind)
                    guardKind = methods.contains(.deviceConfirmation) ? .deviceConfirmation
                        : methods.contains(.deviceCode) ? .deviceCode
                        : methods.contains(.emailCode) ? .emailCode
                        : methods.contains(.emailConfirmation) ? .emailConfirmation : .none
                    initial = false
                }
                try await Task.sleep(for: .milliseconds(max(snapshot.pollIntervalMs, 1000)))
                guard current == generation && !Task.isCancelled else { return }
                if let credentials = try await request.poll() {
                    guard current == generation && !Task.isCancelled else { return }
                    await connect(credentials, generation: current, persist: rememberSession)
                    if current == generation && !isConnected && !Task.isCancelled { beginQR() }
                    return
                }
            }
        } catch is CancellationError {
            return
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            if isCredentialChallenge { errorMessage = Self.message(error) }
            else { qrError = Self.message(error) }
        }
    }

    // Shared handoff for restored and freshly authenticated credentials. Keeping
    // it internal lets integration tests replace accounts without real login.
    func connect(_ credentials: SteamCredentials, generation current: Int, persist: Bool) async {
        guard current == generation && !Task.isCancelled else { return }
        isConnecting = true
        do {
            let connector = connectSession
            let attempt = Task { try await connector(credentials) }
            connectionTask?.cancel()
            connectionTask = attempt
            let connected = try await withTaskCancellationHandler {
                try await attempt.value
            } onCancel: {
                attempt.cancel()
            }
            guard current == generation && !Task.isCancelled else {
                connected.shutdown()
                return
            }
            stopFriends()
            session?.shutdown()
            // The cached library's platforms show until the reload replaces them.
            if cachedAccount == credentials.steamId { libraryPlatforms.cancel() }
            else { libraryPlatforms.reset() }
            cachedAccount = nil
            session = connected
            sessionCredentials = credentials
            displayName = nil
            avatarURL = nil
            friends = Self.emptyFriends(accountID: credentials.steamId)
            // Rust's session logged on Online; the chat follows it from here.
            chat.connect(connected, account: credentials.steamId)
            followGameSession(account: credentials.steamId)
            observeFriends(connected, generation: current)
            login = nil
            challenge = nil
            isCredentialChallenge = false
            isSubmittingCredentials = false
            isSubmittingGuard = false
            errorMessage = nil
            isConnecting = false
            isRestoring = false
            connectInstallations(connected, credentials: credentials)
            rosetta.connect(accountId: credentials.steamId)
            do {
                if persist { try await keychain.save(credentials) }
                else if !rememberSession { try await keychain.remove() }
            } catch {
                guard current == generation else { return }
                storageWarning = L10n.string("model.steam.sessionStillUsable", error.localizedDescription)
            }
            guard current == generation else { return }
            await reloadGames(connected, generation: current)
            #if DEBUG
            await runCloudBackupIfRequested(connected, generation: current)
            #endif
        } catch {
            guard current == generation && !Task.isCancelled else { return }
            isConnecting = false
            errorMessage = Self.message(error)
        }
    }

    func refresh() {
        guard let session, !isLoadingGames else { return }
        let current = generation
        Task { await reloadGames(session, generation: current) }
    }

    #if DEBUG
    /// A Cloud diagnostic needs the storefront idle, and keeps it fenced.
    private func reserveCloudProbe() -> Bool {
        let idle = InstallationOpening<UInt32>.open(except: [])
        guard installations.gate.launch == idle, installations.gate.modify == idle,
              gameSessions.recoveryError == nil else { return false }
        installations.diagnosticReservation = true
        return true
    }

    /// Opt-in developer probe. It consumes the normal in-memory session and
    /// writes only sanitized results. No credential leaves this process.
    private func runCloudBackupIfRequested(_ connected: any SteamSessionProtocol, generation current: Int) async {
        // Ordered by precedence: the first flag present wins, and a plain
        // backup is the default. Flag and report name stay in one row so a new
        // probe cannot pick up another probe's filename.
        let probes: [(flag: String, report: String)] = [
            ("--steam-cloud-autocloud-metadata", "cloud-autocloud-metadata.json"),
            ("--steam-cloud-sdk-candidates", "cloud-sdk-candidates.json"),
            ("--steam-cloud-preferences", "cloud-preferences-result.json"),
            ("--steam-cloud-interruption-probe", "cloud-interruption-result.json"),
            ("--steam-cloud-transfer-probe", "cloud-transfer-result.json"),
            ("--steam-cloud-backup", "cloud-backup-result.json"),
        ]
        let arguments = ProcessInfo.processInfo.arguments
        let probe = probes.first { arguments.contains($0.flag) } ?? probes[probes.count - 1]
        guard let index = arguments.firstIndex(of: probe.flag),
              arguments.count > index + 2, let appID = UInt32(arguments[index + 1]),
              appID != 0, current == generation, session === connected,
              let native = connected as? SteamSession else { return }
        let directory = URL(fileURLWithPath: arguments[index + 2], isDirectory: true)
        let report = directory.appending(path: probe.report)
        do {
            let json: String
            switch probe.flag {
            case "--steam-cloud-autocloud-metadata":
                json = try await native.cloudAutocloudMetadata()
            case "--steam-cloud-sdk-candidates":
                json = try await native.cloudSdkCandidates()
            case "--steam-cloud-preferences":
                json = try await native.cloudPreferences(appId: appID)
            case "--steam-cloud-interruption-probe":
                guard reserveCloudProbe() else { throw CocoaError(.featureUnsupported) }
                json = try await native.cloudInterruptionProbe(appId: appID, directory: directory.path)
                // Keep game admission fenced while the interrupted diagnostic
                // awaits a process restart, including after a diagnostic error.
                let result = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
                if result?["status"] as? String == "verified" { installations.diagnosticReservation = false }
            case "--steam-cloud-transfer-probe":
                guard reserveCloudProbe() else { throw CocoaError(.featureUnsupported) }
                defer { installations.diagnosticReservation = false }
                json = try await native.cloudTransferProbe(appId: appID, directory: directory.path)
            default:
                json = try await native.cloudBackup(appId: appID, directory: directory.path)
            }
            guard current == generation, session === connected else { return }
            try Data(json.utf8).write(to: report, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: report.path)
        } catch {
            // Steam errors contain only typed diagnostics, not request URLs.
            guard current == generation, session === connected else { return }
            let data = try? JSONSerialization.data(withJSONObject: ["status": "failed", "reason": error.localizedDescription])
            try? data?.write(to: report, options: .atomic)
        }
    }
    #endif

    private func reloadGames(_ session: any SteamSessionProtocol, generation current: Int) async {
        guard current == generation && self.session === session && !Task.isCancelled else { return }
        libraryPlatforms.cancel()
        isLoadingGames = true
        do {
            let loaded = try await session.games()
            guard current == generation && self.session === session else { return }
            games = loaded
            if let selected = selectedGame, let updated = loaded.first(where: { $0.appId == selected.appId }) {
                selectedGame = updated
            }
            isLoadingGames = false
            libraryPlatforms.load(appIds: loaded.map(\.appId), session: session)
        } catch {
            guard current == generation && self.session === session else { return }
            isLoadingGames = false
            errorMessage = Self.message(error)
        }
        guard current == generation && self.session === session && !Task.isCancelled else { return }
        reloadProfile(session, generation: current)
    }

    private func reloadProfile(_ session: any SteamSessionProtocol, generation current: Int) {
        profileTask?.cancel()
        profileTask = Task {
            do {
                let profile = try await session.profile()
                guard current == generation && self.session === session && !Task.isCancelled else { return }
                displayName = profile.displayName
                avatarURL = profile.avatarUrl.flatMap(URL.init(string:))
            } catch {
                // Optional profile information must not interrupt the library or login.
            }
        }
    }

    /// The game's name in the library, or as a friend playing it shows it.
    func gameName(appId: UInt32) -> String? {
        if let game = games.first(where: { $0.appId == appId }) { return game.name }
        return friends?.friends.lazy.compactMap(\.activity).first { $0.id == String(appId) }?.name
    }

    /// A game's own Steam session can leave the account's presence behind it,
    /// so the account shows Online again once its Game session ends. While it
    /// runs, the chat follows what the game can invite into.
    private func observeGameSessions() {
        let active = withObservationTracking { gameSessions.active } onChange: { [weak self] in
            Task { @MainActor [weak self] in self?.observeGameSessions() }
        }
        if let ended = observedGameSession, active?.id != ended.id, ended.account == steamId { chat.setOnline() }
        observedGameSession = active
        followGameSession(account: steamId)
    }

    /// The chat follows the Game session of `account`: whether it plays, and
    /// what its running game asked to invite friends into.
    private func followGameSession(account: UInt64?) {
        let active = gameSessions.active
        let playing = account != nil && active?.account == account ? active : nil
        chat.setPlaying(playing != nil)
        let running = playing?.running == true ? playing : nil
        if chat.followGame(session: running?.id, appId: running?.game ?? 0, request: running?.inviteRequest,
                           lobby: running?.lobbyId) {
            openInvitePicker()
        }
        followOverlayRequest(of: running)
        if let invite = invitations.follow(session: running?.id, invites: running?.receivedInvites ?? []).first {
            overlay?.announce(invite)
        }
        if purchase.follow(session: running?.id, microTxn: running?.microTxn), let pending = purchase.pending {
            overlay?.present(.microTxn(orderId: pending.orderId))
        }
    }

    /// The running game asked Steam's overlay for one of its pages: Obelisk's
    /// overlay shows its own. As for invite dialogs, a request the session
    /// showed when first seen opens nothing.
    private func followOverlayRequest(of running: SteamGameSessions.Session?) {
        guard let running else {
            followedOverlay = SessionSequence()
            return
        }
        guard followedOverlay.advance(session: running.id, sequence: running.overlayRequest?.sequence),
              let request = running.overlayRequest,
              let shown = OverlayRequest.steam(request.kind, appId: running.game, account: steamId) else { return }
        overlay?.present(shown)
    }

    /// The running game asked Steam's overlay to invite friends: Obelisk's
    /// overlay shows over it, its friends list as the picker.
    private func openInvitePicker() {
        guard friends != nil, let picker = chat.invitePicker, overlay?.present(.invite(picker)) == true else {
            chat.closeInvitePicker()
            return
        }
        chat.refreshOwnGame()
    }

    func openFriends() {
        guard friends != nil else { return }
        isFriendsPresented = true
        chat.refreshOwnGame()
    }

    func closeFriends() {
        isFriendsPresented = false
    }

    /// Only explicit retry or system wake can restart an exhausted reconnect cycle.
    func retryFriends() {
        guard isConnected, sessionCredentials != nil,
              friends?.connection != .authenticationRequired else { return }
        stopFriends()
        friends?.connection = .interrupted
        libraryPlatforms.cancel()
        session?.shutdown()
        chat.suspend()
        let previous = session
        session = nil
        observeFriends(previous, generation: generation)
    }

    private func stopFriends() {
        friendsObservationGeneration += 1
        friendsTask?.cancel()
        friendsTask = nil
    }

    private func observeFriends(_ initial: (any SteamSessionProtocol)?, generation current: Int) {
        friendsObservationGeneration += 1
        let observation = friendsObservationGeneration
        let connect = connectSession
        let delay = reconnectDelay
        guard let credentials = sessionCredentials else { return }
        friendsTask = Task { [weak self] in
            var active = initial
            var revision: UInt64 = 0
            while !Task.isCancelled {
                if let observed = active {
                    let snapshot = await withTaskCancellationHandler {
                        await observed.nextFriends(afterRevision: revision)
                    } onCancel: {
                        // This wakes a pending Rust watch even while content owns the CM mutex.
                        observed.shutdown()
                    }
                    guard let self, current == self.generation,
                          observation == self.friendsObservationGeneration, !Task.isCancelled else { return }
                    revision = snapshot.revision
                    guard snapshot.accountId == credentials.steamId else { continue }
                    self.applyFriends(snapshot)
                    switch snapshot.connection {
                    case .loading, .ready, .error:
                        continue
                    case .authenticationRequired:
                        observed.shutdown()
                        self.libraryPlatforms.cancel()
                        self.chat.suspend()
                        self.session = nil
                        return
                    case .disconnected:
                        observed.shutdown()
                        self.libraryPlatforms.cancel()
                        self.chat.suspend()
                        self.session = nil
                        active = nil
                    }
                }

                // One bounded cycle per interruption, with no status polling.
                for attempt in 0..<3 {
                    do {
                        try await delay(attempt)
                        try Task.checkCancellation()
                        let replacement = try await connect(credentials)
                        guard let self, current == self.generation,
                              observation == self.friendsObservationGeneration, !Task.isCancelled else {
                            replacement.shutdown()
                            return
                        }
                        self.libraryPlatforms.cancel()
                        self.session = replacement
                        self.chat.connect(replacement, account: credentials.steamId)
                        self.followGameSession(account: credentials.steamId)
                        // The previous session is shut down: Transfers,
                        // launches and Cloud maintenance go through this one.
                        self.connectInstallations(replacement, credentials: credentials)
                        active = replacement
                        revision = 0
                        Task { [weak self] in
                            guard let self else { return }
                            await self.reloadGames(replacement, generation: current)
                            guard self.session === replacement else { return }
                            if self.selectedGame != nil { self.reloadDetails() }
                        }
                        break
                    } catch {
                        guard let self, current == self.generation,
                              observation == self.friendsObservationGeneration, !Task.isCancelled else { return }
                        if case SteamError.AuthenticationRequired = error {
                            self.friends?.connection = .authenticationRequired
                            return
                        }
                        if attempt == 2 {
                            self.friends?.connection = .failed(L10n.string("model.friends.reconnectFailed"))
                            return
                        }
                    }
                }
            }
        }
    }

    private static func emptyFriends(accountID: UInt64) -> FriendsPresentationState {
        FriendsPresentationState(
            store: FriendsStoreIdentity(id: "steam", name: L10n.string("ui.store.steam"), logoAssetName: "SteamLogo"),
            account: FriendsAccountIdentity(storeID: "steam", accountID: String(accountID)),
            friends: [], connection: .loading
        )
    }

    private func applyFriends(_ snapshot: SteamFriendsSnapshot) {
        guard snapshot.accountId == steamId else { return }
        if snapshot.connection == .loading, let previous = friends,
           previous.connection != .loading {
            // A new transport must finish full hydration before replacing stale rows.
            friends?.connection = .interrupted
            return
        }
        var presentation = Self.emptyFriends(accountID: snapshot.accountId)
        presentation.friends = snapshot.friends.map { friend in
            FriendPresentation(
                id: FriendIdentity(account: presentation.account, friendID: String(friend.steamId)),
                name: friend.displayName,
                avatarURL: friend.avatarUrl.flatMap(URL.init(string:)),
                presence: FriendPresence(friend.presence),
                activity: friend.game.map { FriendGameActivity(id: $0.appId.map(String.init) ?? "unknown", name: $0.name) }
            )
        }
        switch snapshot.connection {
        case .loading: presentation.connection = .loading
        case .ready: presentation.connection = .connected
        case .disconnected: presentation.connection = .interrupted
        case .authenticationRequired: presentation.connection = .authenticationRequired
        case .error: presentation.connection = .failed(L10n.string("model.friends.unavailable"))
        }
        if snapshot.connection != .ready, snapshot.connection != .loading,
           let previous = friends, !previous.friends.isEmpty {
            presentation.friends = previous.friends
        }
        presentation.lastKnownOnlineCount = snapshot.connection == .ready
            ? presentation.onlineCount : friends?.lastKnownOnlineCount
        if friends != presentation { friends = presentation }
    }

    func signOut() {
        if let account = steamId ?? cachedAccount {
            libraryCache?.forget(account: .steam(steamId: account))
            gameSessions.history.forget(account: account, in: SteamGames.self)
        }
        cachedAccount = nil
        libraryPlatforms.reset()
        rosetta.connect(accountId: nil)
        installations.signedOut()
        cloud.signedOut()
        generation += 1
        detailGeneration += 1
        stopFriends()
        session?.shutdown()
        chat.reset()
        followedOverlay = SessionSequence()
        invitations.follow(session: nil, invites: [])
        purchase.follow(session: nil, microTxn: nil)
        sessionCredentials = nil
        webSignIn = nil
        friends = nil
        isFriendsPresented = false
        loginTask?.cancel()
        connectionTask?.cancel()
        connectionTask = nil
        profileTask?.cancel()
        profileTask = nil
        session = nil
        login = nil
        displayName = nil
        avatarURL = nil
        games = []
        selectedGame = nil
        details = nil
        news = []
        challenge = nil
        searchText = ""
        filter = .all
        errorMessage = nil
        storageWarning = nil
        isConnecting = false
        isLoadingGames = false
        isRestoring = true
        let current = generation
        Task {
            do { try await keychain.remove() }
            catch {
                guard current == generation else { return }
                storageWarning = L10n.string("model.steam.sessionMayRestore", error.localizedDescription)
            }
            guard current == generation && !Task.isCancelled else { return }
            isRestoring = false
            beginQR()
        }
    }

    func open(_ game: SteamGame) {
        guard session != nil else { return }
        selectedGame = game
        details = nil
        news = []
        detailsError = nil
        newsError = nil
        isLoadingNews = true
        detailGeneration += 1
        let request = detailGeneration
        let current = generation
        reloadDetails()
        reloadAchievements()
        installations.refreshCloudIndicator()
        Task {
            do {
                let loaded = try await loadNews(game.appId)
                guard current == generation && request == detailGeneration else { return }
                news = loaded
                isLoadingNews = false
            } catch {
                guard current == generation && request == detailGeneration else { return }
                newsError = Self.message(error)
                isLoadingNews = false
            }
        }
    }

    func reloadAchievements() {
        guard let game = selectedGame else { return }
        loadAchievements(appId: game.appId, into: achievements)
    }

    /// Loads the achievements of `appId` into `model`, such as the overlay's
    /// for the running game.
    func loadAchievements(appId: UInt32, into model: GameAchievementsModel) {
        guard let session else { return }
        let language = language
        model.load { try await session.achievements(appId: appId, language: language).map(GameAchievement.init) }
    }

    /// Remote details tell which builds the account can download, so an
    /// installed game never waits for them.
    func reloadDetails() {
        guard let game = selectedGame, let session else { return }
        detailsRequestGeneration += 1
        let detailsRequest = detailsRequestGeneration
        details = nil
        detailsError = nil
        isLoadingDetails = false
        guard installations.selectedReceipt == nil else { return }
        let selection = detailGeneration
        let current = generation
        let requestedLanguage = language
        isLoadingDetails = true
        Task {
            do {
                let loaded = try await session.details(appId: game.appId, language: requestedLanguage)
                guard current == generation && self.session === session && selection == detailGeneration && detailsRequest == detailsRequestGeneration else { return }
                details = loaded
                isLoadingDetails = false
            } catch {
                guard current == generation && self.session === session && selection == detailGeneration && detailsRequest == detailsRequestGeneration else { return }
                detailsError = Self.message(error)
                isLoadingDetails = false
            }
        }
    }

    /// Downloads a game without opening its page: Steam picks the build the
    /// account can install, the native one first, as the Transfer starts.
    func downloadFromLibrary(_ game: SteamGame) {
        guard session != nil, installations.canInstall(game.appId) else { return }
        installations.install(.steam(appId: game.appId, name: game.name, platform: nil, language: language), of: game.appId)
    }

    func closeGame() {
        detailGeneration += 1
        selectedGame = nil
        details = nil
        news = []
    }

    /// Cookies that sign Steam's sites in as the connected account, as the
    /// Steam client signs its own browser in: its session asks Steam for a web
    /// access token. None while signed out, or when Steam refuses them: the
    /// page then shows signed out.
    func webCookies() async -> [HTTPCookie] {
        guard let credentials = sessionCredentials, let session else { return [] }
        if let webSignIn, webSignIn.steamId == credentials.steamId, webSignIn.date.timeIntervalSinceNow > -3_600 {
            return webSignIn.cookies
        }
        guard let loaded = try? await loadWebCookies(session),
              sessionCredentials?.steamId == credentials.steamId else { return [] }
        let cookies = loaded.compactMap(Self.cookie)
        webSignIn = (credentials.steamId, Date(), cookies)
        return cookies
    }

    /// Steam's store, signed in as the connected account.
    func storePage() async -> WebPageStart {
        WebPageStart(url: GameOverlayController.storeURL(appId: 0), cookies: await webCookies())
    }

    nonisolated static func cookie(_ cookie: SteamWebCookie) -> HTTPCookie? {
        var properties: [HTTPCookiePropertyKey: Any] = [
            .domain: cookie.domain, .path: "/", .name: cookie.name, .value: cookie.value, .secure: "TRUE",
        ]
        if cookie.httpOnly { properties[HTTPCookiePropertyKey("HttpOnly")] = "TRUE" }
        return HTTPCookie(properties: properties)
    }

    private func connectInstallations(_ session: any SteamSessionProtocol, credentials: SteamCredentials) {
        let receipts = directory.appending(path: "steam-installations-\(credentials.steamId).json")
        installations.connect(account: credentials.steamId,
                              backend: makeInstallations(session, credentials, gameSessions.core, receipts),
                              port: installationPort())
    }

    /// Steam's part of the installation actions.
    private func installationPort() -> InstallationPort<SteamGames> {
        InstallationPort(
            report: { [weak self] error in self?.errorMessage = error.map(Self.message) },
            warn: { [weak self] in self?.storageWarning = $0 },
            uninstalled: { [weak self] game in
                if self?.selectedGame?.appId == game { self?.reloadDetails() }
            }
        )
    }

    static func message(_ error: Error) -> String {
        if case SteamError.AuthenticationRequired = error {
            return L10n.string("backend.steam.friends.authenticationRequired")
        }
        if case SteamError.Cancelled = error { return L10n.string("backend.steam.content.cancelled") }
        if case let SteamError.Failure(reason) = error { return L10n.message(reason) }
        return error.localizedDescription
    }
}

extension SteamGame: Identifiable {
    public var id: UInt32 { appId }

    /// Nil when the game was never played.
    var playtime: Duration? { playtimeMinutes > 0 ? .seconds(Int64(playtimeMinutes) * 60) : nil }

    /// When Steam last saw the game played; nil when it never did.
    var lastPlayedOnSteam: Date? { lastPlayed > 0 ? Date(timeIntervalSince1970: TimeInterval(lastPlayed)) : nil }
}

extension SteamGameDetails {
    /// Obelisk installs the native build when Steam ships one, otherwise Windows.
    var preferredPlatform: String { macos ? SteamGames.nativePlatform : "windows" }

    /// What Steam can install of `game` for the account: its native build first.
    func offer(for game: SteamGame, language: String) -> GameOffer? {
        guard macos || windows else { return nil }
        return .steam(appId: game.appId, name: game.name, platform: preferredPlatform, language: language)
    }
}
