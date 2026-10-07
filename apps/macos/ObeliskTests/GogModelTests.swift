import Foundation
import WebKit
import XCTest
@testable import Obelisk

/// What only GOG's adapter does. WebAccountModelTests covers the account
/// lifecycle, library and game page GOG shares with Epic.
@MainActor
final class GogModelTests: XCTestCase {
    func testTheLibraryInstallsTheNativeBuildFirst() {
        var both = GogGames.game("1", title: "Both")
        both.macos = true
        XCTAssertEqual(both.offer, .gog(productId: "1", title: "Both", platform: "macos"))
        let windows = GogGames.game("2", title: "Windows")
        XCTAssertEqual(windows.offer, .gog(productId: "2", title: "Windows", platform: "windows"))
        var neither = windows
        neither.windows = false
        XCTAssertNil(neither.offer)
    }

    func testPlaytimesAskEveryProductAndConvertMinutes() async throws {
        let games = [GogGames.game("1", title: "Alpha"), GogGames.game("2", title: "Beta"), GogGames.game("3", title: "Gamma")]
        let session = GogTestSession(credentials: GogGames.credentials("current"), games: games,
                                     playtimes: [GogPlaytime(productId: "2", minutes: 600)])
        let model = await connectedModel(session)
        defer { model.signOut() }

        try await eventually { !model.isLoadingPlaytimes }
        let asked = await session.playtimeRequests
        XCTAssertEqual(asked, [["1", "2", "3"]], "One request names every product")
        XCTAssertEqual(model.playtime(of: games[1]), .seconds(36_000))
    }

    func testAnAbsurdMinuteCountSaturates() {
        XCTAssertEqual(GogGames.played(minutes: .max), .seconds(Int64.max))
    }

    func testDetailsPassThePlayersLanguage() async throws {
        var owned = GogGames.game("1", title: "Alpha")
        owned.macos = true
        let session = GogTestSession(credentials: GogGames.credentials("current"), games: [owned])
        let model = await connectedModel(session, language: "fr-FR")
        defer { model.signOut() }

        model.open(owned)
        try await eventually { !model.isLoadingGameDetails }
        let requests = await session.detailsCalls
        XCTAssertEqual(requests, [GogDetailsRequest(productId: "1", platform: GogGames.nativePlatform, language: "fr-FR")])
    }

    func testAchievementsAskGogForTheProductAndFullTag() async throws {
        let owned = GogGames.game("1", title: "Alpha")
        let session = GogTestSession(credentials: GogGames.credentials("current"), games: [owned])
        let model = await connectedModel(session, language: "fr-FR")
        defer { model.signOut() }

        model.open(owned)
        try await eventually { model.achievements.items != nil }
        let products = await session.achievementGames
        let locales = await session.achievementLocales
        XCTAssertEqual(products, ["1"])
        XCTAssertEqual(locales, ["fr-FR"])
    }

    func testOnlyDecimalProductIDsNameAGogGame() {
        XCTAssertEqual(GogGames.game("1207658924"), "1207658924")
        XCTAssertNil(GogGames.game("../1207658924"))
        XCTAssertNil(GogGames.game(""))
    }

    func testGogSyncsNoCloud() async throws {
        let owned = GogGames.game("1", title: "Alpha")
        let directory = FileManager.default.temporaryDirectory.appending(path: "ObeliskGogCloudTests-\(UUID().uuidString)")
        // Installed, so an Epic game in its place would show an indicator.
        let backend = FakeInstallations<GogGames>(receipts: [InstallationReceipt(
            id: "1-windows", gameID: "1", name: "Alpha", directory: directory.appending(path: "Alpha").path,
            platform: "windows", totalBytes: 100
        )])
        backend.indicators = [("1", .neverSynced)]
        defer { try? FileManager.default.removeItem(at: directory) }
        let session = GogTestSession(credentials: GogGames.credentials("current"), games: [owned])
        let model = GogModel(directory: directory, keychain: MemoryStore(GogGames.credentials("current")),
                             restore: { _ in GogGames.session(session) }, gameSessions: GameSessions(),
                             installations: GogModel.installations(backend))
        await model.start()
        defer { model.signOut() }

        model.open(owned)
        try await eventually { !model.isLoadingGameDetails }
        XCTAssertNotNil(model.installations.selectedReceipt)
        await drainMainQueue(times: 2)
        XCTAssertFalse(model.installations.canSyncCloud)
        XCTAssertNil(model.installations.cloudIndicator)
        XCTAssertNil(model.cloud.indicator(for: "1"))
        XCTAssertEqual(backend.indicatorRequests, 0, "GOG never asks Rust for a Cloud indicator")
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
}
