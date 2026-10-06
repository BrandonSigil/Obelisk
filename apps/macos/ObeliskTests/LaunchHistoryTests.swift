import Foundation
import XCTest
@testable import Obelisk

@MainActor
final class LaunchHistoryTests: XCTestCase {
    private var directory: URL!

    override func setUp() async throws {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "ObeliskLaunchHistory-\(UUID().uuidString)", directoryHint: .isDirectory)
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private var file: URL { directory.appending(path: LibraryCache.fileName) }

    private func steamEntry(_ appId: UInt32, _ name: String) -> SteamLibraryEntry {
        SteamLibraryEntry(SteamGame(appId: appId, name: name, iconHash: "", lastPlayed: 0, playtimeMinutes: 0), platforms: nil)
    }

    private let rocket = EpicGame(appName: "rocket", namespace: "space", catalogItemId: "item", title: "Rocket League",
                                  description: "", imageUrl: nil, headerImageUrl: nil, developer: "", windows: true, macos: false)

    func testALaunchIsKeptInItsGamesLibraryEntryAndSurvivesARestart() throws {
        let cache = LibraryCache(file: file)
        cache.save([steamEntry(620, "Portal 2"), steamEntry(570, "Dota 2")], in: .steam, account: "7")
        cache.save([EpicLibraryEntry(rocket, playtimeSeconds: nil)], in: .epic, account: "epic-a")
        let history = LaunchHistory(cache: cache)
        let earlier = Date(timeIntervalSince1970: 1_790_000_000)
        let later = Date(timeIntervalSince1970: 1_790_000_600)
        history.record(rocket.libraryID, account: "epic-a", in: EpicGames.self, at: earlier)
        history.record(620, account: 7, in: SteamGames.self, at: earlier)
        history.record(rocket.libraryID, account: "epic-a", in: EpicGames.self, at: later)

        let reopened = LaunchHistory(cache: LibraryCache(file: file))
        XCTAssertEqual(reopened.lastLaunch(of: rocket.libraryID, account: "epic-a", in: EpicGames.self), later)
        XCTAssertEqual(reopened.lastLaunch(of: 620, account: 7, in: SteamGames.self), earlier)
        XCTAssertNil(reopened.lastLaunch(of: 570, account: 7, in: SteamGames.self))
        XCTAssertNil(reopened.lastLaunch(of: rocket.libraryID, account: "epic-b", in: EpicGames.self),
                     "Another account never sees these launches")

        let saved = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
        let epic = try XCTUnwrap(((saved["epic"] as? [String: Any])?["epic-a"] as? [[String: Any]])?.first)
        XCTAssertEqual(epic["id"] as? String, "space:rocket")
        XCTAssertEqual(epic["lastLaunchedAt"] as? String, "2026-09-21T14:23:20Z", "In the game's own library entry")
        let steam = try XCTUnwrap((saved["steam"] as? [String: Any])?["7"] as? [[String: Any]])
        XCTAssertEqual(steam.first { $0["id"] as? String == "620" }?["lastLaunchedAt"] as? String, "2026-09-21T14:13:20Z")
        XCTAssertNil(steam.first { $0["id"] as? String == "570" }?["lastLaunchedAt"])
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), [LibraryCache.fileName],
                       "The library cache is the only file")
    }

    func testReloadingALibraryKeepsItsLaunchesAndForgettingTheAccountDropsThem() throws {
        let cache = LibraryCache(file: file)
        cache.save([steamEntry(620, "Portal 2")], in: .steam, account: "7")
        let history = LaunchHistory(cache: cache)
        let launched = Date(timeIntervalSince1970: 1_790_000_000)
        history.record(620, account: 7, in: SteamGames.self, at: launched)

        cache.save([steamEntry(620, "Portal 2 renamed"), steamEntry(570, "Dota 2")], in: .steam, account: "7")
        XCTAssertEqual(LaunchHistory(cache: LibraryCache(file: file)).lastLaunch(of: 620, account: 7, in: SteamGames.self),
                       launched, "A refreshed library keeps each game's last launch")

        history.forget(account: 7, in: SteamGames.self)
        cache.forget(.steam, account: "7")
        XCTAssertNil(history.lastLaunch(of: 620, account: 7, in: SteamGames.self))
        XCTAssertNil(LaunchHistory(cache: LibraryCache(file: file)).lastLaunch(of: 620, account: 7, in: SteamGames.self))
    }

    func testSessionsRecordAGameTheySawStartButNotOneRecoveredRunning() async throws {
        let core = FakeSessions(recovering: true)
        let history = LaunchHistory(cache: LibraryCache(file: file))
        let recovered = GameSessionState.epic(account: "epic-a", appName: "recovered")
        core.show(recovered)
        let sessions = EpicGameSessions(core, history: history)
        try await eventually { sessions.active?.running == true }
        XCTAssertNil(history.lastLaunch(of: "namespace:recovered", account: "epic-a", in: EpicGames.self),
                     "A game recovered running was recorded when it started")
        core.show(nil)

        let launch = GameSessionState.epic(account: "epic-a", appName: "launched", phase: .preparing)
        core.show(launch)
        try await eventually { sessions.active?.id == launch.id }
        XCTAssertNil(history.lastLaunch(of: "namespace:launched", account: "epic-a", in: EpicGames.self),
                     "A launch that has not started yet is not a launch")
        var running = launch
        running.phase = .running
        core.show(running)
        try await eventually { sessions.active?.running == true }
        XCTAssertNotNil(history.lastLaunch(of: "namespace:launched", account: "epic-a", in: EpicGames.self))
    }
}
