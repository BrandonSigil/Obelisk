import Foundation

typealias GogModel = WebAccountModel<GogGames>

extension GogGames: WebStorefront {
    typealias Session = any GogSessionProtocol
    typealias Credentials = GogCredentials
    typealias Game = GogGame
    typealias Details = GogGameDetails

    static let storeID = "gog"
    static let webSignIn = WebSignIn.gog
    static func keychain() -> any CredentialStore<GogCredentials> { GogKeychain() }

    static func signIn(code: String, library: LibraryCache?) async throws -> Session {
        try await GogSession.signIn(authorizationCode: code, library: library)
    }
    static func restore(_ credentials: GogCredentials, library: LibraryCache?) async throws -> Session {
        try await GogSession.restore(credentials: credentials, library: library)
    }
    static func accountID(_ credentials: GogCredentials) -> String { credentials.userId }
    static func accountName(_ credentials: GogCredentials) -> String { credentials.username }
    static func avatar(of session: Session) -> URL? { session.avatarUrl().flatMap(URL.init(string:)) }
    static func rotating(_ session: Session) -> RotatingSession<GogCredentials> {
        RotatingSession(
            identity: ObjectIdentifier(session),
            credentials: { try await session.credentials() },
            persist: { (saver: any CredentialSave) in await session.persist(credentials: saver) },
            shutdownObservers: { session.shutdownObservers() }
        )
    }
    /// GOG offers no revocation of a Galaxy token: this only forgets the session.
    static func disconnect(_ session: Session) async throws { try await session.disconnect() }

    static func games(of session: Session) async throws -> [GogGame] { try await session.games() }
    /// GOG answers each game's playtime on its own, in minutes.
    static func playtimes(of session: Session, games: [GogGame]) async throws -> [String: Duration] {
        let loaded = try await session.playtimes(productIds: games.map(\.productId))
        return Dictionary(loaded.map { ($0.productId, played(minutes: $0.minutes)) }, uniquingKeysWith: max)
    }
    static func cachedLibrary(in cache: LibraryCache, account: String) -> [(game: GogGame, playtime: Duration?)] {
        cache.gogLibrary(userId: account).map { ($0.game, $0.playtimeMinutes.map(played(minutes:))) }
    }
    /// `language` names the depots, such as "fr-FR".
    static func details(of game: GogGame, session: Session, language: String) async throws -> GogGameDetails {
        try await session.gameDetails(productId: game.productId, platform: game.preferredPlatform, language: language)
    }
    /// GOG names achievements in a full language tag, such as "fr-FR".
    static func achievements(of game: GogGame, session: Session, language: String) async throws -> [GameAchievement] {
        try await session.achievements(productId: game.productId, locale: Locale(identifier: language).identifier(.bcp47))
            .map(GameAchievement.init)
    }
    @MainActor static func installations(of session: Session, account: String, language: String,
                                          sessions: any StorefrontSessionsProtocol, receipts: URL,
                                          locations: ObeliskLocations) -> any InstallationsBackend<GogGames> {
        guard let session = session as? GogSession, let sessions = sessions as? StorefrontSessions else {
            return NoInstallations<GogGames>()
        }
        return RustInstallations<GogGames>(try! AccountInstallations.gog(
            session: session, userId: account, language: language, sessions: sessions,
            receipts: receipts.path, locations: locations
        ))
    }
    static func isUnauthorized(_ error: Error) -> Bool {
        if case GogError.Unauthorized = error { true } else { false }
    }
    @MainActor static func describe(_ error: Error) -> String {
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

    static var sessionNotSaved: String { L10n.string("model.gog.sessionNotSaved") }
    static var revocationUnconfirmed: String { L10n.string("model.gog.revocationUnconfirmed") }
    static var otherGameRunning: String { L10n.string("model.gog.otherGameRunning") }
    /// GOG asks no Cloud question: Obelisk does not sync its saves.
    static var cloudTitle: String { L10n.string("ui.store.gog") }

    /// GOG counts playtime in minutes. A count too large to hold saturates.
    static func played(minutes: UInt64) -> Duration {
        let (seconds, overflow) = minutes.multipliedReportingOverflow(by: 60)
        return .seconds(Int64(clamping: overflow ? .max : seconds))
    }
}

extension GogGame: WebGame {
    /// The native build first.
    var preferredPlatform: String { macos ? GogGames.nativePlatform : "windows" }
    public var id: String { productId }

    /// What GOG can install of the game: its native build first.
    var offer: GameOffer? {
        guard macos || windows else { return nil }
        return .gog(productId: productId, title: title, platform: preferredPlatform)
    }
}
