import Foundation
import XCTest
@testable import Obelisk

/// The installation actions as the game on screen presents them, over
/// installations the test drives. Steam stands in for every storefront; Rust's
/// own suite covers the rules behind them.
@MainActor
final class GameInstallationsTests: XCTestCase {
    private let game = SteamGame(appId: 620, name: "Portal 2", iconHash: "", lastPlayed: 0, playtimeMinutes: 0)

    func testThePrimaryActionLaunchesTheInstallationAndCancelReachesItsPreparation() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()

        installations.primaryAction()

        XCTAssertEqual(fixture.backend.actions, [.launch(fixture.receipt)])
        fixture.backend.publish {
            $0.launching = true
            $0.launchGame = 620
            $0.gate = .action(620)
        }
        try await eventually { installations.isLaunching }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.launching"))
        XCTAssertFalse(installations.primaryActionAvailable)
        installations.cancel(620)
        XCTAssertEqual(fixture.backend.actions.last, .cancel(620))
    }

    func testALaunchNamesTheRuntimeItWaitsFor() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()
        installations.primaryAction()
        fixture.backend.publish {
            $0.launching = true
            $0.launchGame = 620
            $0.gate = .action(620)
        }
        try await eventually { installations.isLaunching }

        var session = GameSessionState.steam(appID: 620, phase: .preparing, observed: false)
        session.runtimeInstall = RuntimeInstallState(component: .steamComponents, fraction: 0.25)
        fixture.sessions.fake.show(session)
        try await eventually { installations.runtimeInstallFraction == 0.25 }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.installingSteamComponents"))

        session.runtimeInstall = RuntimeInstallState(component: .voile, fraction: 0.5)
        fixture.sessions.fake.show(session)
        try await eventually { installations.runtimeInstallFraction == 0.5 }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.installingVoile"))

        session.runtimeInstall = nil
        fixture.sessions.fake.show(session)
        try await eventually { installations.runtimeInstallFraction == nil }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.launching"))
    }

    func testAnInstallationLaunchesAndUpdatesBeforeTheStorefrontKnowsWhatItCanInstall() async throws {
        let fixture = Fixture(game: game)
        // The storefront would install the Windows build, once it knows it can.
        let installations = fixture.connect(platform: "windows", installable: false)

        XCTAssertEqual(installations.selectedReceipt?.platform, "macos")
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.launch"))
        XCTAssertTrue(installations.primaryActionAvailable)
        installations.downloadSelected()
        XCTAssertEqual(fixture.backend.actions, [.update(fixture.receipt)], "An update keeps the installed build")
    }

    func testANewInstallationIsWhatTheStorefrontOffers() async throws {
        let fixture = Fixture(game: game, installed: false)
        let installations = fixture.connect(platform: "windows")

        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.download"))
        installations.primaryAction()

        XCTAssertEqual(fixture.backend.actions, [.install(fixture.offer("windows"))])
    }

    func testAFailedTransferIsReportedAndResumesWhereItStopped() async throws {
        let fixture = Fixture(game: game, installed: false)
        let installations = fixture.connect()

        fixture.backend.publish {
            $0.resumable = 620
            $0.transferGame = 620
            $0.report = .storefront(key: "backend.steam.content.network")
        }

        try await eventually { installations.canResumeDownload }
        XCTAssertEqual(fixture.reported, [L10n.message("backend.steam.content.network")])
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.resume"))
        installations.primaryAction()
        XCTAssertEqual(fixture.backend.actions, [.install(fixture.offer("macos"))], "Rust resumes the Transfer")
    }

    func testThePageOffersOnlyWhatRustsGateLetsStart() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()
        XCTAssertTrue(installations.primaryActionAvailable)
        XCTAssertTrue(installations.canUninstallSelectedGame)
        XCTAssertTrue(installations.canSyncCloud)

        // The game runs: nothing touches it.
        fixture.backend.publish { $0.gate = .session(620) }
        try await eventually { !installations.primaryActionAvailable }
        XCTAssertFalse(installations.canUninstallSelectedGame)
        XCTAssertFalse(installations.canSyncCloud)

        // Another game runs: this one may go, but not launch or sync.
        fixture.backend.publish { $0.gate = .session(730) }
        try await eventually { installations.canUninstallSelectedGame }
        XCTAssertFalse(installations.primaryActionAvailable)
        XCTAssertFalse(installations.canSyncCloud)
        installations.primaryAction()

        // Another game's Transfer runs: only this game's Cloud may sync.
        fixture.backend.publish { $0.gate = .action(730) }
        try await eventually { installations.canSyncCloud }
        XCTAssertFalse(installations.canUninstallSelectedGame)
        installations.verify(fixture.receipt)
        installations.uninstall(fixture.receipt)
        installations.downloadSelected()
        XCTAssertEqual(fixture.backend.actions, [], "Swift forwards nothing the gate refuses")
    }

    func testANewerBuildTurnsThePrimaryActionIntoAnUpdate() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()
        XCTAssertEqual(fixture.backend.calls.filter { $0 == .checkUpdates }.count, 1, "Connecting asks for newer builds")
        XCTAssertTrue(installations.primaryActionLaunches)

        var updated = fixture.receipt
        updated.updateAvailable = true
        fixture.backend.publish { $0.receipts = [updated] }
        try await eventually { installations.selectedReceipt?.updateAvailable == true }

        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.update"))
        XCTAssertFalse(installations.primaryActionLaunches)
        XCTAssertTrue(installations.primaryActionAvailable)
        installations.primaryAction()
        XCTAssertEqual(fixture.backend.actions, [.update(updated)], "The update replaces the launch")

        // The game runs: its files stay as they are.
        fixture.backend.publish { $0.gate = .session(620) }
        try await eventually { !installations.primaryActionAvailable }
    }

    func testAnotherGamesTransferLeavesThisGameLaunchableAndItsCancelAlone() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()
        fixture.backend.publish {
            $0.downloading = true
            $0.transferGame = 730
            $0.gate = .transfer(730)
        }
        try await eventually { installations.isDownloading }

        XCTAssertTrue(installations.primaryActionAvailable)
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.launch"))
        installations.primaryAction()
        XCTAssertEqual(fixture.backend.actions, [.launch(fixture.receipt)])
        installations.cancel(620)
        XCTAssertEqual(fixture.backend.actions.count, 1, "this game has nothing to stop")
        installations.cancel(730)
        XCTAssertEqual(fixture.backend.actions.last, .cancel(730))
    }

    func testTheAutomaticUpdatesSettingReachesEachConnectionBeforeItsFirstCheck() throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()
        installations.updatesAutomatically = true
        XCTAssertEqual(fixture.backend.calls.last, .automaticUpdates(true))

        _ = fixture.connect()
        let calls = fixture.backend.calls
        let setting = try XCTUnwrap(calls.firstIndex(of: .automaticUpdates(true)))
        XCTAssertLessThan(setting, try XCTUnwrap(calls.firstIndex(of: .checkUpdates)))
    }

    func testAQueuedGameWaitsAndItsCancelTakesItOutOfTheQueue() async throws {
        let fixture = Fixture(game: game, installed: false)
        let installations = fixture.connect()
        fixture.backend.publish {
            $0.downloading = true
            $0.transferGame = 730
            $0.queued = [620]
            $0.gate = .transfer(730)
        }
        try await eventually { installations.isQueued(620) }

        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.queued"))
        XCTAssertFalse(installations.primaryActionAvailable)
        installations.primaryAction()
        XCTAssertEqual(fixture.backend.actions, [], "a queued game waits")
        installations.cancel(620)
        XCTAssertEqual(fixture.backend.actions, [.cancel(620)])
    }

    func testVerifyingAnInstallationRunsItsTransferInPlace() async throws {
        let fixture = Fixture(game: game)
        let installations = fixture.connect()

        installations.verify(fixture.receipt)

        XCTAssertEqual(fixture.backend.actions, [.update(fixture.receipt)], "Rust repairs or updates the installed build")
    }

    // MARK: Quitting the running game

    func testARunningGameIsAskedToQuitThenMayBeForcedOnceItHadTimeToClose() async throws {
        let quitting = FakeQuitting()
        let fixture = Fixture(game: game, quitting: quitting)
        let installations = fixture.connect()
        let running = GameSessionState.steam(appID: 620)
        fixture.sessions.fake.show(running)
        fixture.backend.publish { $0.gate = .session(620) }
        try await eventually { installations.primaryQuitStage == .quit }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.quit"))
        XCTAssertTrue(installations.primaryActionAvailable)

        installations.primaryAction()
        try await eventually { fixture.sessions.fake.quits == [running.id] && quitting.waiting == 1 }
        XCTAssertEqual(quitting.delays, [.seconds(10)], "The game gets ten seconds to close")
        XCTAssertEqual(quitting.macApps, [[fixture.receipt.directory]], "Its Mac build is asked first")
        XCTAssertEqual(installations.primaryQuitStage, .closing)
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.closing"))
        XCTAssertFalse(installations.primaryActionAvailable)
        installations.primaryAction()

        quitting.wake()
        try await eventually { installations.primaryQuitStage == .forceQuit }
        XCTAssertEqual(installations.primaryActionTitle, L10n.string("model.game.forceQuit"))
        XCTAssertTrue(installations.primaryActionAvailable)
        // The page asked its player to confirm.
        installations.primaryAction()
        try await eventually { fixture.sessions.fake.forceQuits == [running.id] }
        XCTAssertEqual(fixture.sessions.fake.quits, [running.id], "Quit is asked once")
        XCTAssertEqual(fixture.backend.actions, [], "Ending the game is no installation action")

        // The game exits: its session settles its Cloud saves, then ends.
        fixture.sessions.fake.show(.steam(appID: 620, id: running.id, phase: .finalizing))
        try await eventually { fixture.sessions.quitRequest == nil }
        XCTAssertNil(installations.primaryQuitStage)
    }

    func testAMacGameIsAskedToQuitAsItsOwnQuitMenuWould() async throws {
        let quitting = FakeQuitting()
        quitting.macAppRuns = true
        let fixture = Fixture(game: game, quitting: quitting)
        let installations = fixture.connect()
        fixture.sessions.fake.show(.steam(appID: 620))
        fixture.backend.publish { $0.gate = .session(620) }
        try await eventually { installations.primaryQuitStage == .quit }

        installations.primaryAction()
        try await eventually { quitting.macApps == [[fixture.receipt.directory]] && quitting.waiting == 1 }
        XCTAssertEqual(installations.primaryQuitStage, .closing)
        XCTAssertEqual(fixture.sessions.fake.quits, [], "No signal ends it before it could save")
    }

    func testForceQuitIsOfferedForAWhileThenTheGameMayBeAskedToQuitAgain() async throws {
        let quitting = FakeQuitting()
        let sessions = SteamGameSessions(quitting: quitting.quitting)
        let running = GameSessionState.steam(appID: 620)
        sessions.fake.show(running)
        try await eventually { sessions.quitStage == .quit }
        await sessions.quit()
        try await eventually { quitting.waiting == 1 }
        quitting.wake()
        try await eventually { sessions.quitStage == .forceQuit && quitting.waiting == 1 }
        XCTAssertEqual(quitting.delays, [.seconds(10), .seconds(60)])

        // The game asked its player to save first, and plays on.
        quitting.wake()
        try await eventually { sessions.quitStage == .quit }
        XCTAssertNil(sessions.quitRequest)
        await sessions.quit()
        XCTAssertEqual(sessions.quitStage, .closing)
        XCTAssertEqual(sessions.fake.quits, [running.id, running.id])
        XCTAssertEqual(sessions.fake.forceQuits, [])
    }

    func testTheQuitRequestLivesInTheModelForItsSessionOnly() async throws {
        let sessions = SteamGameSessions(quitting: GameQuitting(delay: .seconds(60), quitMacApps: { _ in false }))
        let first = GameSessionState.steam(appID: 620)
        sessions.fake.show(first)
        try await eventually { sessions.quitStage == .quit }
        await sessions.forceQuit()
        XCTAssertEqual(sessions.fake.forceQuits, [], "A game is asked to quit first")

        await sessions.quit()
        // Any page shown later reads the same stage, whatever it showed before.
        XCTAssertEqual(sessions.quitStage, .closing)
        XCTAssertEqual(sessions.quitRequest?.session, first.id)
        await sessions.quit()
        await sessions.forceQuit()
        XCTAssertEqual(sessions.fake.quits, [first.id])
        XCTAssertEqual(sessions.fake.forceQuits, [], "Not before the game had its time")

        let second = GameSessionState.steam(appID: 620)
        sessions.fake.show(second)
        try await eventually { sessions.active?.id == second.id }
        XCTAssertNil(sessions.quitRequest)
        XCTAssertEqual(sessions.quitStage, .quit)
        var reconnecting = second
        reconnecting.reconnecting = true
        sessions.fake.show(reconnecting)
        try await eventually { sessions.active?.reconnecting == true }
        XCTAssertNil(sessions.quitStage, "A session reconnecting to its supervisor is asked nothing")
    }

    func testAGameGetsTenSecondsToCloseThenAMinuteToBeForced() {
        XCTAssertEqual(GameQuitting().delay, .seconds(10))
        XCTAssertEqual(GameQuitting().window, .seconds(60))
    }

    func testOnlyAMacAppRunningFromTheGameIsAskedToQuit() {
        XCTAssertFalse(GameQuitting.quitApps(in: []))
        XCTAssertFalse(GameQuitting.quitApps(in: ["/nonexistent/obelisk-test-installation"]))
    }

    func testTheLibraryCardEndsTheRunningGameAsThePageDoes() {
        XCTAssertEqual(LibraryCardAction.running(.quit).label(title: "Portal 2"),
                       L10n.string("ui.game.quit_named", "Portal 2"))
        XCTAssertEqual(LibraryCardAction.running(.forceQuit).label(title: "Portal 2"),
                       L10n.string("ui.game.force_quit_named", "Portal 2"))
        XCTAssertTrue(LibraryCardAction.running(.quit).isEnabled)
        XCTAssertFalse(LibraryCardAction.running(.closing).isEnabled)
        XCTAssertTrue(LibraryCardAction.running(.forceQuit).isEnabled)
        // A session reconnecting still shows its game running, and asks nothing.
        XCTAssertNil(LibraryCardAction.running(nil).label(title: "Portal 2"))
        XCTAssertFalse(LibraryCardAction.running(nil).isEnabled)
    }
}

/// One signed-in Steam account with Portal 2 on screen, installed for macOS.
@MainActor
private final class Fixture {
    let sessions: SteamGameSessions
    let receipt: InstallationReceipt<UInt32>
    private(set) var backend = FakeInstallations<SteamGames>()
    private(set) var reported: [String] = []
    private let game: SteamGame
    private let installed: Bool
    private let installations: SteamInstallations

    init(game: SteamGame, installed: Bool = true, quitting: FakeQuitting = FakeQuitting()) {
        self.game = game
        self.installed = installed
        sessions = SteamGameSessions(quitting: quitting.quitting)
        receipt = .steam(appId: 620, name: game.name, directory: "/fixture/Games/Steam/steam-620-macos",
                         platform: "macos", totalBytes: 100)
        installations = SteamInstallations(sessions: sessions, cloud: SteamCloudModel(sessions: sessions))
    }

    /// What Steam offers to install, for `platform`.
    func offer(_ platform: String) -> GameOffer {
        GameOffer.steam(appId: 620, name: game.name, platform: platform, language: "english")
    }

    func connect(platform: String = "macos", installable: Bool = true) -> SteamInstallations {
        let offer = installable ? offer(platform) : nil
        installations.selection = { InstallationSelection(game: 620, platform: platform, offer: offer) }
        backend = FakeInstallations(receipts: installed ? [receipt] : [])
        installations.connect(account: 42, backend: backend, port: InstallationPort(
            report: { [unowned self] error in if let error { reported.append(SteamModel.message(error)) } },
            warn: { _ in }
        ))
        return installations
    }
}
