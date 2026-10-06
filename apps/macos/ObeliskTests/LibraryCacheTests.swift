import Foundation
import XCTest
@testable import Obelisk

@MainActor
final class LibraryCacheTests: XCTestCase {
    private var directory: URL!

    override func setUp() async throws {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "ObeliskLibraryCache-\(UUID().uuidString)", directoryHint: .isDirectory)
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: directory)
    }

    private var file: URL { directory.appending(path: "library-cache.json") }

    private func steam(_ appId: UInt32, _ name: String, platforms: [String]? = nil) -> SteamLibraryEntry {
        SteamLibraryEntry(SteamGame(appId: appId, name: name, iconHash: "", lastPlayed: 1_790_000_000, playtimeMinutes: 90),
                          platforms: platforms)
    }

    func testEveryStorefrontAndAccountKeepsItsOwnLibraryInOneFile() throws {
        let cache = LibraryCache(file: file)
        cache.save([steam(620, "Portal 2", platforms: ["macos", "windows"])], in: .steam, account: "7")
        cache.save([steam(570, "Dota 2")], in: .steam, account: "8")
        // Another cache of the same file, as a second storefront would hold.
        let epic = EpicLibraryEntry(EpicGame(appName: "rocket", namespace: "space", catalogItemId: "item", title: "Rocket League",
                                             description: "", imageUrl: nil, headerImageUrl: nil, developer: "Psyonix",
                                             windows: true, macos: false), playtimeSeconds: 3600)
        LibraryCache(file: file).save([epic], in: .epic, account: "epic-a")

        let reopened = LibraryCache(file: file)
        XCTAssertEqual(reopened.games(SteamLibraryEntry.self, in: .steam, account: "7"), [steam(620, "Portal 2", platforms: ["macos", "windows"])])
        XCTAssertEqual(reopened.games(SteamLibraryEntry.self, in: .steam, account: "8")?.map(\.appId), [570])
        XCTAssertEqual(reopened.games(EpicLibraryEntry.self, in: .epic, account: "epic-a"), [epic],
                       "Saving one storefront's library keeps the others'")
        XCTAssertNil(reopened.games(SteamLibraryEntry.self, in: .steam, account: "9"))

        let saved = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
        XCTAssertEqual(saved["version"] as? Int, 1)
        XCTAssertEqual(((saved["steam"] as? [String: Any])?["7"] as? [[String: Any]])?.first?["name"] as? String, "Portal 2")
        XCTAssertNotNil((saved["epic"] as? [String: Any])?["epic-a"])

        reopened.forget(.steam, account: "7")
        XCTAssertNil(LibraryCache(file: file).games(SteamLibraryEntry.self, in: .steam, account: "7"))
        XCTAssertNotNil(LibraryCache(file: file).games(SteamLibraryEntry.self, in: .steam, account: "8"))
    }

    func testEverythingAStorefrontReturnedIsStoredAsJSON() throws {
        let game = SteamGame(appId: 620, name: "Portal 2", iconHash: "", lastPlayed: 0, playtimeMinutes: 0,
                             record: #"{"appid":620,"has_dlc":true,"content_descriptorids":[2,5]}"#)
        let pics = JSONValue(json: #"{"common":{"associations":{"0":{"type":"developer","name":"Valve"}}}}"#)
        let cache = LibraryCache(file: file)
        cache.save([SteamLibraryEntry(game, platforms: ["macos"], pics: pics)], in: .steam, account: "7")
        let epic = EpicGame(appName: "rocket", namespace: "space", catalogItemId: "item", title: "Rocket League",
                            description: "", imageUrl: nil, headerImageUrl: nil, developer: "", windows: true, macos: false,
                            metadata: #"{"catalogItems":{"item":{"customAttributes":{"CanRunOffline":{"value":"true"}}}}}"#)
        cache.save([EpicLibraryEntry(epic, playtimeSeconds: nil)], in: .epic, account: "epic-a")

        let saved = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
        let steamGame = try XCTUnwrap(((saved["steam"] as? [String: Any])?["7"] as? [[String: Any]])?.first)
        XCTAssertEqual((steamGame["record"] as? [String: Any])?["has_dlc"] as? Bool, true, "Stored as JSON, not as text")
        let association = ((steamGame["pics"] as? [String: Any])?["common"] as? [String: Any])?["associations"] as? [String: Any]
        XCTAssertEqual((association?["0"] as? [String: Any])?["name"] as? String, "Valve")
        let epicGame = try XCTUnwrap(((saved["epic"] as? [String: Any])?["epic-a"] as? [[String: Any]])?.first)
        XCTAssertNotNil((epicGame["metadata"] as? [String: Any])?["catalogItems"])

        let reopened = try XCTUnwrap(LibraryCache(file: file).games(SteamLibraryEntry.self, in: .steam, account: "7")?.first)
        XCTAssertEqual(reopened.pics, pics)
        XCTAssertEqual(JSONValue(json: reopened.game.record), JSONValue(json: game.record))
        let reopenedEpic = try XCTUnwrap(LibraryCache(file: file).games(EpicLibraryEntry.self, in: .epic, account: "epic-a")?.first)
        XCTAssertEqual(JSONValue(json: reopenedEpic.game.metadata), JSONValue(json: epic.metadata))
    }

    func testAnUnreadableFileHasNoLibraryAndIsReplacedByTheNextSave() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("not json".utf8).write(to: file)
        let cache = LibraryCache(file: file)
        XCTAssertNil(cache.games(SteamLibraryEntry.self, in: .steam, account: "7"))
        cache.save([steam(620, "Portal 2")], in: .steam, account: "7")
        XCTAssertEqual(cache.games(SteamLibraryEntry.self, in: .steam, account: "7")?.map(\.appId), [620])
    }
}
