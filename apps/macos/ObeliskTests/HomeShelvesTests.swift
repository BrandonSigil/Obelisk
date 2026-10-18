import SwiftUI
import XCTest
@testable import Obelisk

@MainActor
final class HomeShelvesTests: XCTestCase {
    func testTheLastGamePlayedAnywhereLeadsAndTheShelvesFollowIt() {
        let shelves = HomeShelves([
            game("Portal 2", played: 100, installed: true),
            game("Rocket League", in: .epic, played: 300, installed: true),
            game("Hades", in: .gog, played: 200),
            game("Never played", playtime: 9_000, installed: true),
        ])
        XCTAssertEqual(shelves.hero?.title, "Rocket League")
        XCTAssertEqual(shelves.recent.map(\.title), ["Hades", "Portal 2"])
        XCTAssertEqual(shelves.installed.map(\.title), ["Rocket League", "Portal 2", "Never played"])
    }

    func testWithNothingPlayedTheMostPlayedLeadsThenTheFirstByName() {
        let played = HomeShelves([game("Celeste", playtime: 60), game("Braid", playtime: 600), game("Antichamber")])
        XCTAssertEqual(played.hero?.title, "Braid")
        XCTAssertTrue(played.recent.isEmpty)
        XCTAssertEqual(HomeShelves([game("Celeste"), game("Braid")]).hero?.title, "Braid")
        XCTAssertNil(HomeShelves([]).hero)
    }

    func testTheRecentShelfKeepsTheLastPlayedGamesOnly() {
        let games = (0..<30).map { game("Game \($0)", played: TimeInterval($0)) }
        let shelves = HomeShelves(games)
        XCTAssertEqual(shelves.hero?.title, "Game 29")
        XCTAssertEqual(shelves.recent.count, HomeShelves.shelfLimit)
        XCTAssertEqual(shelves.recent.first?.title, "Game 28")
    }

    private func game(_ title: String, in section: LibrarySection = .steam, played: TimeInterval? = nil,
                      playtime: Int64? = nil, installed: Bool = false) -> HomeGame {
        HomeGame(
            id: HomeGame.ID(section: section, game: title),
            title: title,
            lastPlayed: played.map { Date(timeIntervalSince1970: $0) },
            playtime: playtime.map { .seconds($0) },
            isInstalled: installed,
            action: GameAction(kind: installed ? .launch : .download(.install),
                               title: L10n.string(installed ? "ui.game.play" : "model.game.download"),
                               isAvailable: true, launches: installed),
            cover: { AnyView(EmptyView()) },
            hero: HomeGame.Hero(artwork: { AnyView(EmptyView()) }, logo: { AnyView(EmptyView()) }),
            open: {}
        )
    }
}
