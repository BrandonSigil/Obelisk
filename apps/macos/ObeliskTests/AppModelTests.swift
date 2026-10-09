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
}
