import Foundation
import WebKit
import XCTest
@testable import Obelisk

@MainActor
final class GogModelTests: XCTestCase {
    func testAutomaticLoginExchangesBrowserCodeAndLoadsLibrary() async {
        let store = GogMemoryStore(nil)
        let session = GogTestSession(credentials: credentials("current"), games: [game("1", title: "Alpha")])
        var codes: [String] = []
        let model = GogModel(
            keychain: store,
            authorize: { "browser-authorization-code" },
            signIn: { code in
                await MainActor.run { codes.append(code) }
                return session
            },
            gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache()))
        )

        model.signIn()
        XCTAssertTrue(model.isConnecting)
        await eventually { model.isConnected && !model.isLoadingGames }

        XCTAssertEqual(codes, ["browser-authorization-code"])
        let saved = await store.value
        XCTAssertEqual(saved?.refreshToken, "current", "The account is remembered as it signs in")
        XCTAssertEqual(model.accountName, "Geralt")
        XCTAssertEqual(model.avatarURL, URL(string: "https://images.gog.com/avatar_avm.jpg"))
        XCTAssertEqual(model.games.map(\.productId), ["1"])
        XCTAssertNil(model.errorMessage)
    }

    func testTheCachedLibraryShowsWhileGogRestoresAndSigningOutForgetsIt() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: "ObeliskGogCache-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let cached = game("7", title: "Cached game")
        let cache = try LibraryCache.holding(
            gog: ["48628349957132247": [CachedGogGame(game: cached, playtimeMinutes: 90)]], in: directory
        )
        let history = LaunchHistory(cache: cache)
        let launched = Date(timeIntervalSince1970: 1_790_000_000)
        history.record(cached.productId, account: "48628349957132247", in: GogGames.self, at: launched)
        let stored = credentials("current")
        let session = GogTestSession(credentials: stored, games: [game("1", title: "Alpha")])
        let restoration = GameSessionGate<GogTestSession>()
        let model = GogModel(keychain: GogMemoryStore(stored), restore: { _ in try await restoration.wait() },
                             gameSessions: GogGameSessions(FakeSessions(), history: history), libraryCache: cache)
        let start = Task { await model.start() }

        await eventually { model.showsLibrary }
        XCTAssertFalse(model.isConnected, "The cached library shows before GOG answers")
        XCTAssertEqual(model.games, [cached])
        XCTAssertEqual(model.lastPlayed(of: cached), launched)
        XCTAssertEqual(model.playtime(of: cached), .seconds(90 * 60), "The cached playtime shows too")
        XCTAssertEqual(model.featuredGame, cached, "The last game launched from Obelisk leads the library")
        await restoration.resolve(session)
        await start.value
        await eventually { model.games.map(\.productId) == ["1"] }

        model.signOut()
        XCTAssertTrue(cache.gogLibrary(userId: "48628349957132247").isEmpty, "Signing out forgets the account's library")
    }

    func testExpiredRestoreRemovesSavedSession() async {
        let store = GogMemoryStore(credentials("expired"))
        let model = GogModel(keychain: store, restore: { _ in throw GogError.Unauthorized },
                             gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache())))
        await model.start()
        XCTAssertFalse(model.isConnected)
        XCTAssertEqual(model.errorMessage, L10n.string("model.gog.unauthorized"))
        await eventually { await store.value == nil }
    }

    func testTheLibraryInstallsTheNativeBuildFirst() {
        var both = game("1", title: "Both")
        both.macos = true
        XCTAssertEqual(both.offer, .gog(productId: "1", title: "Both", platform: "macos"))
        let windows = game("2", title: "Windows")
        XCTAssertEqual(windows.offer, .gog(productId: "2", title: "Windows", platform: "windows"))
        var neither = windows
        neither.windows = false
        XCTAssertNil(neither.offer)
    }

    func testPlaytimesLoadOnceTheGamesAreKnownAndOrderTheLibrary() async {
        let session = GogTestSession(credentials: credentials("current"),
                                     games: [game("1", title: "Alpha"), game("2", title: "Beta"), game("3", title: "Gamma")],
                                     playtimes: [GogPlaytime(productId: "2", minutes: 600),
                                                 GogPlaytime(productId: "3", minutes: 5)])
        let model = GogModel(keychain: GogMemoryStore(nil), authorize: { "code" }, signIn: { _ in session },
                             gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache())))
        model.signIn()
        await eventually { model.isConnected && !model.isLoadingGames && !model.isLoadingPlaytimes }
        let asked = await session.playtimeRequests
        XCTAssertEqual(asked, [["1", "2", "3"]])
        XCTAssertEqual(model.availableSorts, StorefrontLibrarySort.allCases)
        model.sort = .playtime
        XCTAssertEqual(model.filteredGames.map(\.productId).prefix(2), ["2", "3"])
        model.filter = .played
        XCTAssertEqual(Set(model.filteredGames.map(\.productId)), ["2", "3"])
        XCTAssertEqual(model.featuredGame?.productId, "2", "The most played game leads before any launch")
    }

    func testEqualPlaytimesFeatureTheFirstGameByName() async {
        let session = GogTestSession(credentials: credentials("current"),
                                     games: [game("1", title: "Zeta"), game("2", title: "beta"), game("3", title: "Alpha")],
                                     playtimes: [GogPlaytime(productId: "1", minutes: 30),
                                                 GogPlaytime(productId: "2", minutes: 30)])
        let model = GogModel(keychain: GogMemoryStore(nil), authorize: { "code" }, signIn: { _ in session },
                             gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache())))
        model.signIn()
        await eventually { model.isConnected && !model.isLoadingGames && !model.isLoadingPlaytimes }
        XCTAssertEqual(model.featuredGame?.productId, "2", "Equal playtimes: the first by name, whatever its case")
    }

    func testANeverPlayedLibraryStillHasAHero() async {
        let session = GogTestSession(credentials: credentials("current"),
                                     games: [game("1", title: "The Evil Within"), game("2", title: "Celeste")])
        let model = GogModel(keychain: GogMemoryStore(nil), authorize: { "code" }, signIn: { _ in session },
                             gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache())))
        model.signIn()
        await eventually { model.isConnected && !model.isLoadingGames && !model.isLoadingPlaytimes }
        XCTAssertEqual(model.featuredGame?.productId, "2")
    }

    func testOpeningAGameLoadsItsAchievementsInThePlayersLanguage() async {
        let session = GogTestSession(credentials: credentials("current"), games: [game("1", title: "Alpha")])
        let model = GogModel(keychain: GogMemoryStore(nil), authorize: { "code" }, signIn: { _ in session },
                             language: "fr-FR",
                             gameSessions: GogGameSessions(FakeSessions(), history: LaunchHistory(cache: Self.emptyCache())))
        model.signIn()
        await eventually { model.isConnected && !model.isLoadingGames }
        model.open(model.games[0])
        await eventually { model.achievements.items != nil }
        let asked = await session.achievementRequests
        XCTAssertEqual(asked, ["1 fr-FR"])
        XCTAssertEqual(model.achievements.countText, "1/2")
        model.closeGame()
        XCTAssertNil(model.achievements.items)
    }

    func testGogSyncsNoCloud() {
        XCTAssertEqual(GogGames.game("1207658924"), "1207658924")
        XCTAssertNil(GogGames.game("../1207658924"))
        XCTAssertNil(GogGames.game(""))
    }

    func testTheGogWindowCapturesTheCodeBeforeTheRedirectLoads() async throws {
        let completed = expectation(description: "GOG callback intercepted")
        var loginWindow: NSWindow?
        let task = Task {
            defer { completed.fulfill() }
            return try await WebAuthentication(.gog).authorize { webView in
                loginWindow = webView.window
                webView.loadHTMLString("""
                    <html><body><script>
                    window.location.href = 'https://embed.gog.com/on_login_success?origin=client&code=fixture-gog';
                    </script></body></html>
                    """, baseURL: nil)
            }
        }
        await fulfillment(of: [completed], timeout: 10)
        task.cancel()
        let code = try await task.value
        XCTAssertEqual(code, "fixture-gog")
        XCTAssertEqual(loginWindow?.title, L10n.string("model.gogWeb.title"))
        XCTAssertEqual(loginWindow?.isVisible, false)
    }

    private static func emptyCache() -> LibraryCache {
        LibraryCache(directory: FileManager.default.temporaryDirectory
            .appending(path: "ObeliskGogEmpty-\(UUID().uuidString)").path)
    }

    private func credentials(_ token: String) -> GogCredentials {
        GogCredentials(userId: "48628349957132247", username: "Geralt", refreshToken: token)
    }

    private func game(_ id: String, title: String) -> GogGame {
        GogGame(productId: id, title: title, imageUrl: nil, headerImageUrl: nil, windows: true, macos: false,
                summary: "", developers: [], publishers: [], releaseDate: nil, genres: [], rating: nil, languages: [])
    }

    private func eventually(
        file: StaticString = #filePath,
        line: UInt = #line,
        _ predicate: @MainActor () async -> Bool
    ) async {
        for _ in 0..<200 {
            if await predicate() { return }
            try? await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Timed out waiting for the expected GOG state", file: file, line: line)
    }
}

actor GogMemoryStore: CredentialStore {
    private(set) var value: GogCredentials?

    init(_ value: GogCredentials?) { self.value = value }
    func load() async throws -> GogCredentials? { value }
    func save(_ credentials: GogCredentials) async throws { value = credentials }
    func remove() async throws { value = nil }
}

actor GogTestSession: GogSessionProtocol {
    private let current: GogCredentials
    private let owned: [GogGame]
    private let played: [GogPlaytime]
    private(set) var playtimeRequests: [[String]] = []
    private(set) var achievementRequests: [String] = []
    nonisolated let credentialRevisions = EpicCredentialRevisionFeed()

    init(credentials: GogCredentials, games: [GogGame], playtimes: [GogPlaytime] = []) {
        current = credentials
        owned = games
        played = playtimes
    }

    func playtimes(productIds: [String]) async throws -> [GogPlaytime] {
        playtimeRequests.append(productIds)
        return played
    }
    func achievements(productId: String, locale: String) async throws -> [GogAchievement] {
        achievementRequests.append("\(productId) \(locale)")
        return [
            GogAchievement(id: "a", name: "First", description: "", iconUrl: "", hidden: false, unlocked: true,
                           unlockTime: 1_790_000_000),
            GogAchievement(id: "b", name: "Second", description: "", iconUrl: "", hidden: true, unlocked: false,
                           unlockTime: nil),
        ]
    }

    nonisolated func avatarUrl() -> String? { "https://images.gog.com/avatar_avm.jpg" }
    func credentials() async throws -> GogCredentials { current }
    func disconnect() async throws {}
    func gameDetails(productId: String, platform: String, language: String) async throws -> GogGameDetails {
        GogGameDetails(installationBytes: 42)
    }
    func games() async throws -> [GogGame] { owned }
    func nextCredentialsRevision(afterRevision: UInt64) async -> UInt64? {
        await credentialRevisions.next(after: afterRevision)
    }
    nonisolated func persist(credentials: any CredentialSave) async {
        let feed = credentialRevisions
        Task.detached {
            credentials.save()
            var after: UInt64 = 0
            while let next = await feed.next(after: after) {
                after = next
                credentials.save()
            }
        }
    }
    nonisolated func shutdownObservers() { credentialRevisions.stop() }
}
