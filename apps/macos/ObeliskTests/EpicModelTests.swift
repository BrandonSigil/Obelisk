import Foundation
import XCTest
@testable import Obelisk

/// What only Epic's adapter does. WebAccountModelTests covers the account
/// lifecycle, library and game page Epic shares with GOG.
@MainActor
final class EpicModelTests: XCTestCase {
    func testOpeningAMacGameRequestsItsMacDetails() async throws {
        var owned = EpicGames.game("one", title: "One")
        owned.macos = true
        let session = EpicTestSession(credentials: EpicGames.credentials("current"), games: [owned])
        let model = await connectedModel(session)
        defer { model.signOut() }

        model.open(owned)
        try await eventually { !model.isLoadingGameDetails }
        let requests = await session.detailsCalls
        XCTAssertEqual(requests, [EpicDetailsRequest(namespace: "fixture", catalogItemId: "one", appName: "one", platform: "Mac")])
    }

    func testDetailsUseWindowsForWindowsAndUnspecifiedPlatformGames() async throws {
        let windowsGame = EpicGames.game("windows", title: "Windows")
        var unspecifiedGame = EpicGames.game("unspecified", title: "Unspecified")
        unspecifiedGame.windows = false
        let session = EpicTestSession(credentials: EpicGames.credentials("current"), games: [windowsGame, unspecifiedGame])
        let model = await connectedModel(session)
        defer { model.signOut() }

        for owned in [windowsGame, unspecifiedGame] {
            model.open(owned)
            try await eventually { !model.isLoadingGameDetails }
        }

        let requests = await session.detailsCalls
        XCTAssertEqual(requests.map(\.platform), ["Windows", "Windows"])
        XCTAssertEqual(requests.map(\.appName), ["windows", "unspecified"])
    }

    func testPlaytimesMapAppNamesToEveryLibraryEntry() async throws {
        var first = EpicGames.game("shared", title: "First")
        first.namespace = "first"
        var second = first
        second.namespace = "second"
        second.title = "Second"
        let session = EpicTestSession(credentials: EpicGames.credentials("current"), games: [first, second],
                                      playtimes: [EpicPlaytime(appName: "shared", seconds: 600)])
        let model = await connectedModel(session)
        defer { model.signOut() }

        try await eventually { !model.isLoadingPlaytimes }
        XCTAssertEqual(model.playtime(of: first), .seconds(600))
        XCTAssertEqual(model.playtime(of: second), .seconds(600), "Epic counts an app name's time for each of its entries")
    }

    func testAchievementsAskEpicForTheNamespaceAndBareLanguage() async throws {
        let owned = EpicGames.game("one", title: "One")
        let session = EpicTestSession(credentials: EpicGames.credentials("current"), games: [owned])
        let model = await connectedModel(session, language: "fr-FR")
        defer { model.signOut() }

        model.open(owned)
        try await eventually { model.achievements.items != nil }
        let namespaces = await session.achievementGames
        let locales = await session.achievementLocales
        XCTAssertEqual(namespaces, ["fixture"])
        XCTAssertEqual(locales, ["fr"])
    }
}
