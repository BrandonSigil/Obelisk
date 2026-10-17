import Foundation
import Observation
import XCTest
@testable import Obelisk

@MainActor
final class StorefrontLibraryFilterTests: XCTestCase {
    func testSteamFiltersCombineWithSearchAndEverySort() async throws {
        let root = try temporaryDirectory()
        let games = [
            steamGame(1, name: "Portal Z", lastPlayed: 300, playtime: 0),
            steamGame(2, name: "Portal A", lastPlayed: 0, playtime: 60),
            steamGame(3, name: "Portal Never"),
            steamGame(4, name: "Other", lastPlayed: 900, playtime: 120)
        ]
        let model = steamModel(root: root, games: games, receipts: steamReceipts([(1, "windows"), (2, "macos")], root: root))
        await model.start()
        defer { model.signOut() }

        XCTAssertEqual(model.availableFilters, [.all, .installed, .played, .macOS])
        XCTAssertEqual(model.filteredGames.map(\.appId), [4, 1, 2, 3])
        model.searchText = "  portal  "
        model.filter = .played
        XCTAssertEqual(model.filteredGames.map(\.appId), [1, 2])
        model.sort = .name
        XCTAssertEqual(model.filteredGames.map(\.appId), [2, 1])
        model.sort = .playtime
        XCTAssertEqual(model.filteredGames.map(\.appId), [2, 1])

        model.filter = .installed
        model.searchText = ""
        XCTAssertEqual(model.filteredGames.map(\.appId), [2, 1])
        model.searchText = "Portal Z"
        XCTAssertEqual(model.filteredGames.map(\.appId), [1])
        model.filter = .all
        model.searchText = "Portal Never"
        XCTAssertEqual(model.filteredGames.map(\.appId), [3])
    }

    func testSteamMacOSFilterObservesLatePlatformBatchesAfterCachedReads() async throws {
        let root = try temporaryDirectory()
        let responses = FilterPlatformResponses()
        let games = (1...51).map { steamGame(UInt32($0), name: "Game \($0)") }
        let model = steamModel(root: root, games: games, platforms: { try await responses.request($0) })
        await model.start()
        defer { model.signOut() }
        try await eventually { await responses.requestCount == 1 }
        model.filter = .macOS
        model.sort = .name

        XCTAssertTrue(model.filteredGames.isEmpty)
        let firstChange = expectation(description: "Cached macOS results observe the first platform batch")
        withObservationTracking {
            XCTAssertTrue(model.filteredGames.isEmpty)
        } onChange: {
            firstChange.fulfill()
        }
        await responses.complete(0, macOSAppIDs: [1])
        try await eventually { await responses.requestCount == 2 }
        await fulfillment(of: [firstChange], timeout: 1)
        XCTAssertTrue(model.libraryPlatforms.isLoading)
        XCTAssertEqual(model.filteredGames.map(\.appId), [1])

        let secondChange = expectation(description: "Cached macOS results observe the next platform batch")
        withObservationTracking {
            XCTAssertEqual(model.filteredGames.map(\.appId), [1])
        } onChange: {
            secondChange.fulfill()
        }
        await responses.complete(1, macOSAppIDs: [51])
        try await eventually { !model.libraryPlatforms.isLoading }
        await fulfillment(of: [secondChange], timeout: 1)
        XCTAssertEqual(model.filteredGames.map(\.appId), [1, 51])
        model.searchText = "Game 51"
        XCTAssertEqual(model.filteredGames.map(\.appId), [51])
    }

    func testSteamOrdersByPICSFactsAndLoadsDatesAndAchievementsWithTheLibrary() async throws {
        let root = try temporaryDirectory()
        let games = [steamGame(1, name: "Alpha"), steamGame(2, name: "Beta"), steamGame(3, name: "Gamma")]
        let records = FilterRecordRequests()
        // PICS tells nothing of Gamma.
        let model = steamModel(root: root, games: games, platforms: { appIDs in
            appIDs.filter { $0 != 3 }.map { appID in
                SteamGamePlatforms(appId: appID, platforms: ["windows"], facts: appID == 1
                    ? SteamGameFacts(releaseDate: 300, metacriticScore: 80, reviewScore: 8, reviewPercentage: 99)
                    : SteamGameFacts(releaseDate: 100, metacriticScore: nil, reviewScore: 9, reviewPercentage: 96))
            }
        }, records: records)
        await model.start()
        defer { model.signOut() }
        try await eventually {
            !model.libraryPlatforms.isLoading && model.libraryPlatforms.facts.count == 2
                && model.libraryRecords.value(for: .dateAdded, of: 3) != nil
                && model.libraryRecords.value(for: .achievements, of: 3) != nil
        }
        var asked = await records.counts
        XCTAssertEqual(asked.acquisitions, 1, "The library dates load with the library, before any order asks")
        XCTAssertEqual(asked.achievements, 1)
        XCTAssertEqual(asked.achievementApps, [1, 2, 3], "One request names every game")

        model.sort = .releaseDate
        XCTAssertEqual(model.filteredGames.map(\.appId), [1, 2, 3], "The newest first, an undated game last")
        model.sort = .criticScore
        XCTAssertEqual(model.filteredGames.map(\.appId), [1, 2, 3], "Games without a score follow by name")
        model.sort = .playerReviews
        XCTAssertEqual(model.filteredGames.map(\.appId), [2, 1, 3], "Steam's verdict outranks the share")
        model.sort = .dateAdded
        XCTAssertEqual(model.filteredGames.map(\.appId), [3, 1, 2])
        model.sort = .achievements
        XCTAssertEqual(model.filteredGames.map(\.appId), [3, 2, 1])
        XCTAssertNil(model.pendingMetadata)
        asked = await records.counts
        XCTAssertEqual(asked.acquisitions + asked.achievements, 2, "Picking an order asks nothing")
    }

    func testSizeOrderPutsTheLargestInstallationFirstAndFollowsInstallations() async throws {
        let root = try temporaryDirectory()
        let small = epicGame("small", title: "Small"), large = epicGame("large", title: "Large")
        let absent = epicGame("absent", title: "Absent")
        let fake = FakeInstallations<EpicGames>(receipts: epicReceipts([(small, "windows"), (large, "windows"), (large, "macos")],
                                                                       root: root, bytes: [small.appName: 10, large.appName: 400]))
        let model = epicModel(root: root, games: [small, absent, large], installations: fake)
        await model.start()
        defer { model.signOut() }

        model.sort = .size
        XCTAssertEqual(model.filteredGames.map(\.appName), ["large", "small", "absent"],
                       "Both of a game's installations count, and a game not installed comes last")
        fake.publish { $0.receipts += self.epicReceipts([(absent, "windows")], root: root, bytes: [absent.appName: 5_000]) }
        try await eventually { model.filteredGames.map(\.appName) == ["absent", "large", "small"] }
    }

    func testEpicOrdersByTheDateEachGameJoinedTheLibrary() async throws {
        var older = epicGame("older", title: "Older"), newer = epicGame("newer", title: "Newer")
        older.addedAt = 1_590_000_000
        newer.addedAt = 1_700_000_000
        let undated = epicGame("undated", title: "Undated")
        let model = epicModel(root: try temporaryDirectory(), games: [older, undated, newer])
        await model.start()
        defer { model.signOut() }

        XCTAssertEqual(model.availableSorts, [.name, .achievements, .playtime, .lastPlayed, .dateAdded, .size])
        model.sort = .dateAdded
        XCTAssertEqual(model.filteredGames.map(\.appName), ["newer", "older", "undated"])
    }

    func testSteamSignOutResetsFilterAndCachedLibrary() async throws {
        let model = steamModel(root: try temporaryDirectory(), games: [steamGame(1, name: "Played", playtime: 1)])
        await model.start()
        model.filter = .played
        XCTAssertEqual(model.filteredGames.map(\.appId), [1])

        model.signOut()

        XCTAssertEqual(model.filter, .all)
        XCTAssertTrue(model.filteredGames.isEmpty)
        XCTAssertFalse(model.isConnected)
        try await eventually { !model.isRestoring && !model.isConnecting }
    }

    func testEpicFiltersUseNamespaceIdentityAndInstallationsOnEitherPlatform() async throws {
        let root = try temporaryDirectory()
        let installedWindows = epicGame("shared", namespace: "first", title: "Étoiles Z", macOS: true)
        let otherNamespace = epicGame("shared", namespace: "second", title: "Étoiles A")
        let installedMac = epicGame("native", title: "Étoiles B", macOS: true)
        let availableMac = epicGame("available", title: "Étoiles C", macOS: true)
        let unknown = EpicGame(appName: "unknown", namespace: "first", catalogItemId: "unknown", title: "Unknown",
                               description: "", imageUrl: nil, headerImageUrl: nil, developer: "", windows: false, macos: false)
        let model = epicModel(root: root, games: [installedWindows, unknown, availableMac, otherNamespace, installedMac],
                              receipts: epicReceipts([(installedWindows, "windows"), (installedMac, "macos")], root: root))
        await model.start()
        defer { model.signOut() }

        XCTAssertEqual(model.availableFilters, [.all, .installed, .played, .macOS], "Epic's playtimes tell which games were played")
        XCTAssertEqual(model.filteredGames.count, 5)
        model.searchText = "  etoiles  "
        XCTAssertEqual(model.filteredGames.map(\.libraryID), [otherNamespace, installedMac, availableMac, installedWindows].map(\.libraryID))

        model.filter = .installed
        XCTAssertEqual(model.filteredGames.map(\.libraryID), [installedMac, installedWindows].map(\.libraryID))
        model.filter = .macOS
        XCTAssertEqual(model.filteredGames.map(\.libraryID), [installedMac, availableMac, installedWindows].map(\.libraryID))
        model.searchText = "Unknown"
        XCTAssertTrue(model.filteredGames.isEmpty)
        model.filter = .all
        XCTAssertEqual(model.filteredGames.map(\.libraryID), [unknown.libraryID])
    }

    func testEpicSignOutResetsFilterAndCachedLibrary() async throws {
        let model = epicModel(root: try temporaryDirectory(), games: [epicGame("native", title: "Native", macOS: true)])
        await model.start()
        model.filter = .macOS
        XCTAssertEqual(model.filteredGames.map(\.appName), ["native"])

        model.signOut()

        XCTAssertEqual(model.filter, .all)
        XCTAssertTrue(model.filteredGames.isEmpty)
        XCTAssertFalse(model.isConnected)
        try await eventually { !model.isSigningOut }
    }

    private func temporaryDirectory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appending(path: "ObeliskLibraryFilters-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return root
    }

    private func steamGame(_ appID: UInt32, name: String, lastPlayed: UInt64 = 0, playtime: UInt32 = 0) -> SteamGame {
        SteamGame(appId: appID, name: name, iconHash: "", lastPlayed: lastPlayed, playtimeMinutes: playtime)
    }

    private func steamModel(root: URL, games: [SteamGame], receipts: [InstallationReceipt<UInt32>] = [],
                            platforms: @escaping @Sendable ([UInt32]) async throws -> [SteamGamePlatforms] = { _ in [] },
                            records: FilterRecordRequests = FilterRecordRequests()) -> SteamModel {
        let session = FilterSteamSession(games: games, platforms: platforms, records: records)
        return SteamModel(directory: root, keychain: FilterSteamCredentials(), connect: { _ in session }, news: { _ in [] },
                          beginLogin: { _ in throw FilterFixtureFailure.unexpectedOperation },
                          gameSessions: SteamGameSessions(),
                          installations: SteamModel.installations(FakeInstallations(receipts: receipts)))
    }

    private func epicGame(_ appName: String, namespace: String = "first", title: String, macOS: Bool = false) -> EpicGame {
        EpicGame(appName: appName, namespace: namespace, catalogItemId: "catalog-\(appName)", title: title,
                 description: "", imageUrl: nil, headerImageUrl: nil, developer: "", windows: true, macos: macOS)
    }

    private func epicModel(root: URL, games: [EpicGame], receipts: [InstallationReceipt<String>] = [],
                           installations: FakeInstallations<EpicGames>? = nil) -> EpicModel {
        let credentials = EpicCredentials(accountId: "filters", displayName: "Filter fixture", refreshToken: "fixture-only-never-sent")
        let session = EpicTestSession(credentials: credentials, games: games)
        return EpicModel(directory: root, keychain: EpicMemoryStore(credentials), restore: { _ in session },
                         installations: EpicModel.installations(installations ?? FakeInstallations(receipts: receipts)))
    }

    private func steamReceipts(_ entries: [(UInt32, String)], root: URL) -> [InstallationReceipt<UInt32>] {
        entries.map { appID, platform in
            .steam(appId: appID, name: "Game \(appID)", directory: root.appending(path: "steam-\(appID)-\(platform)").path,
                   platform: platform, totalBytes: 1)
        }
    }

    private func epicReceipts(_ entries: [(EpicGame, String)], root: URL,
                              bytes: [String: UInt64] = [:]) -> [InstallationReceipt<String>] {
        entries.map { game, platform in
            InstallationReceipt.epic(namespace: game.namespace, appName: game.appName, name: game.title,
                               directory: root.appending(path: "epic-\(game.namespace)-\(game.appName)-\(platform)").path,
                               platform: platform, totalBytes: bytes[game.appName] ?? 1)
        }
    }

    private func eventually(file: StaticString = #filePath, line: UInt = #line,
                            _ condition: @MainActor () async -> Bool) async throws {
        for _ in 0..<200 {
            if await condition() { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Library filter fixture did not finish", file: file, line: line)
        throw FilterFixtureFailure.timeout
    }
}

private enum FilterFixtureFailure: Error { case unexpectedOperation, timeout }

private actor FilterSteamCredentials: CredentialStore {
    func load() -> SteamCredentials? {
        SteamCredentials(accountName: "Filter fixture", steamId: 42, refreshToken: "fixture-only-never-sent")
    }
    func save(_ credentials: SteamCredentials) {}
    func remove() {}
}

private final class FilterSteamSession: SteamSessionProtocol, Sendable {
    private let library: [SteamGame]
    private let loadPlatforms: @Sendable ([UInt32]) async throws -> [SteamGamePlatforms]
    private let records: FilterRecordRequests

    init(games: [SteamGame], platforms: @escaping @Sendable ([UInt32]) async throws -> [SteamGamePlatforms],
         records: FilterRecordRequests) {
        library = games
        loadPlatforms = platforms
        self.records = records
    }

    func games() async throws -> [SteamGame] { library }
    func platforms(appIds: [UInt32], language: String) async throws -> [SteamGamePlatforms] { try await loadPlatforms(appIds) }
    func acquisitions() async throws -> [UInt32: UInt64] { await records.acquisitions() }
    func achievementShares(appIds: [UInt32], language: String) async throws -> [UInt32: Double] {
        await records.achievementShares(appIds)
    }
    func profile() async throws -> SteamProfile { SteamProfile(displayName: nil, avatarUrl: nil) }
    func details(appId: UInt32, language: String) async throws -> SteamGameDetails { throw FilterFixtureFailure.unexpectedOperation }
}

/// Steam's answers about the account's games: Gamma was got last, Alpha
/// before it and Beta through no license; Gamma's achievements are all
/// unlocked, half of Beta's, and Alpha has none.
private actor FilterRecordRequests {
    private(set) var counts = (acquisitions: 0, achievements: 0, achievementApps: [UInt32]())

    func acquisitions() -> [UInt32: UInt64] {
        counts.acquisitions += 1
        return [1: 1_500_000_000, 3: 1_600_000_000]
    }

    func achievementShares(_ appIDs: [UInt32]) -> [UInt32: Double] {
        counts.achievements += 1
        counts.achievementApps = appIDs
        return [2: 0.5, 3: 1]
    }
}

private actor FilterPlatformResponses {
    private var requests: [[UInt32]] = []
    private var pending: [Int: CheckedContinuation<[SteamGamePlatforms], any Error>] = [:]
    var requestCount: Int { requests.count }

    func request(_ appIDs: [UInt32]) async throws -> [SteamGamePlatforms] {
        let index = requests.count
        requests.append(appIDs)
        return try await withCheckedThrowingContinuation { pending[index] = $0 }
    }

    func complete(_ index: Int, macOSAppIDs: Set<UInt32>) {
        guard let continuation = pending.removeValue(forKey: index) else {
            XCTFail("No pending platform batch \(index)")
            return
        }
        continuation.resume(returning: requests[index].map {
            SteamGamePlatforms(appId: $0, platforms: macOSAppIDs.contains($0) ? ["macos", "windows"] : ["windows"])
        })
    }
}
