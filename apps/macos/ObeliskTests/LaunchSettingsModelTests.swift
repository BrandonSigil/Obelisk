import XCTest
@testable import Obelisk

final class LaunchSettingsModelTests: XCTestCase {
    private static func settings(choices: LaunchChoices = LaunchChoices(),
                                 engine: LaunchEngine = .rosetta) -> LaunchSettings {
        LaunchSettings(
            engine: engine, graphics: engine == .rosetta ? .d3dMetal : .dxmt, rayTracing: true,
            performanceHud: choices.performanceHud, steamClient: choices.steamClient ?? .obelisk,
            choices: choices, defaultEngine: .rosetta,
            engines: [
                LaunchEngineOffer(
                    engine: .rosetta, translator: .rosetta, defaultGraphics: .d3dMetal,
                    graphics: [
                        LaunchGraphicsOffer(graphics: .d3dMetal, options: [.rayTracing]),
                        LaunchGraphicsOffer(graphics: .dxmt, options: []),
                    ]
                ),
                LaunchEngineOffer(
                    engine: .linux, translator: .fex, defaultGraphics: .dxmt,
                    graphics: [LaunchGraphicsOffer(graphics: .dxmt, options: [])]
                ),
            ]
        )
    }

    @MainActor
    func testAChoiceKeepsTheOthersAndTheOffersFollowTheEngineRustReturns() async throws {
        var saved: [LaunchChoices] = []
        let saving = expectation(description: "saved")
        let model = LaunchSettingsModel(
            load: { Self.settings() },
            save: { choices in
                saved.append(choices)
                saving.fulfill()
                return Self.settings(choices: choices, engine: choices.engine ?? .rosetta)
            }
        )
        await model.appear()
        XCTAssertFalse(model.isCustomized)
        XCTAssertEqual(model.graphicsOffer?.options, [.rayTracing])

        model.choose { $0.engine = .linux }
        await fulfillment(of: [saving], timeout: 2)

        XCTAssertEqual(saved, [LaunchChoices(engine: .linux)])
        try await waitUntil { model.settings?.engine == .linux }
        XCTAssertTrue(model.isCustomized)
        XCTAssertEqual(model.engineOffer?.translator, .fex)
        XCTAssertEqual(model.graphicsOffer?.graphics, .dxmt)
        XCTAssertEqual(model.graphicsOffer?.options, [])
    }

    @MainActor
    func testRestoringDefaultsForgetsEveryChoice() async throws {
        var saved: [LaunchChoices] = []
        let model = LaunchSettingsModel(
            load: { Self.settings(choices: LaunchChoices(arguments: "-windowed", engine: .linux, rayTracing: false,
                                                                 performanceHud: true),
                                  engine: .linux) },
            save: { choices in
                saved.append(choices)
                return Self.settings(choices: choices)
            }
        )
        await model.appear()
        XCTAssertTrue(model.isCustomized)

        model.reset()

        try await waitUntil { !model.isCustomized }
        XCTAssertEqual(saved, [LaunchChoices()])
    }

    @MainActor
    func testAChangeRustCouldNotKeepIsReportedAndTheSettingsStay() async throws {
        struct NotSaved: LocalizedError { var errorDescription: String? { "not saved" } }
        let model = LaunchSettingsModel(load: { Self.settings() }, save: { _ in throw NotSaved() })
        await model.appear()

        model.choose { $0.rayTracing = false }

        try await waitUntil { model.error != nil }
        XCTAssertEqual(model.error, "not saved")
        XCTAssertEqual(model.settings, Self.settings())
    }

    @MainActor
    func testTypedArgumentsAreKeptTrimmedOnlyWhenTheyChange() async throws {
        var saved: [LaunchChoices] = []
        let model = LaunchSettingsModel(
            load: { Self.settings() },
            save: { choices in
                saved.append(choices)
                return Self.settings(choices: choices)
            }
        )
        await model.appear()

        model.setArguments("   ")
        model.setArguments("  -dx11 \"+exec auto exec.cfg\" ")
        try await waitUntil { model.settings?.choices.arguments != nil }
        model.setArguments("-dx11 \"+exec auto exec.cfg\"")
        model.setArguments("")
        try await waitUntil { saved.count == 2 }

        XCTAssertEqual(saved.map(\.arguments), ["-dx11 \"+exec auto exec.cfg\"", nil])
    }

    @MainActor
    func testWindowsSteamGetsTheLoginTheCodeApprovedThenTheSheetIsDone() async throws {
        let login = FakeQRLogin(approvedAfter: 1)
        var handed: [String] = []
        let model = WindowsSteamSignInModel(signIn: { handed.append($0.accountName) }, begin: { login })

        model.start()
        try await waitUntil { model.phase == .done }

        XCTAssertEqual(handed, ["goose"])
        XCTAssertGreaterThanOrEqual(login.polls, 2)
    }

    @MainActor
    func testWindowsSteamRefusingTheLoginSaysSoAndOffersANewCode() async throws {
        let model = WindowsSteamSignInModel(
            signIn: { _ in throw SteamGames.failure("backend.steam.windowsSteam.signInFailed") },
            begin: { FakeQRLogin(approvedAfter: 0) }
        )

        model.start()
        try await waitUntil { if case .failed = model.phase { true } else { false } }

        XCTAssertEqual(model.phase, .failed(L10n.string("backend.steam.windowsSteam.signInFailed")))
    }

    @MainActor
    private func waitUntil(_ condition: () -> Bool) async throws {
        for _ in 0..<500 where !condition() {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(condition())
    }
}

/// A QR sign-in the Steam Mobile App approves after `approvedAfter` polls.
private final class FakeQRLogin: SteamLoginProtocol, @unchecked Sendable {
    private let approvedAfter: Int
    private(set) var polls = 0

    init(approvedAfter: Int) { self.approvedAfter = approvedAfter }

    func challenge() async -> SteamChallenge {
        SteamChallenge(qrUrl: "https://s.team/q/1/2", confirmations: [], pollIntervalMs: 1, remoteInteraction: false)
    }

    func poll() async throws -> SteamCredentials? {
        polls += 1
        return polls > approvedAfter
            ? SteamCredentials(accountName: "goose", steamId: 76_561_197_960_287_930, refreshToken: "token") : nil
    }

    func submitGuard(code: String, kind: SteamGuardKind) async throws {}
}
