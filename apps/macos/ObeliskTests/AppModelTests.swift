import Foundation
import XCTest
@testable import Obelisk

@MainActor
final class AppModelTests: XCTestCase {
    func testAnotherSectionClearsTheSearchAndTheAppearanceIsRemembered() throws {
        let root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "ObeliskTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = AppModel(directory: root, defaults: defaults)

        model.searchText = "portal"
        model.selection = .steam
        XCTAssertTrue(model.searchText.isEmpty)

        model.appearance = .dark
        XCTAssertEqual(AppModel(directory: root, defaults: defaults).appearance, .dark)
    }

    func testGamesUpdateAutomaticallyUnlessThePlayerTurnsItOffInEveryStorefront() throws {
        let root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "ObeliskTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let model = AppModel(directory: root, defaults: defaults)
        XCTAssertTrue(model.updatesGamesAutomatically)
        XCTAssertTrue(model.steam.installations.updatesAutomatically)

        model.updatesGamesAutomatically = false
        XCTAssertFalse(model.steam.installations.updatesAutomatically)
        XCTAssertFalse(model.epic.installations.updatesAutomatically)
        XCTAssertFalse(model.gog.installations.updatesAutomatically)
        let restarted = AppModel(directory: root, defaults: defaults)
        XCTAssertFalse(restarted.updatesGamesAutomatically)
        XCTAssertFalse(restarted.gog.installations.updatesAutomatically)
    }
}
