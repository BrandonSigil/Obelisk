import Foundation
import Observation

/// Each library game's artwork as its PICS app info names it, which loads
/// with the games' platforms. Steam's artwork reads it from the environment.
@MainActor
@Observable
final class SteamLibraryArtwork {
    /// A game PICS has not told of yet has none.
    fileprivate(set) var games: [UInt32: SteamGameArtwork] = [:]
}

@MainActor
@Observable
final class SteamLibraryPlatforms {
    private(set) var platforms: [UInt32: [String]] = [:] {
        didSet { revision &+= 1 }
    }
    /// Apart from the platforms: new artwork changes no filter or order.
    let artwork = SteamLibraryArtwork()
    /// Changes whenever a game's platforms may have.
    private(set) var revision = 0
    private(set) var isLoading = false
    private(set) var hasFailures = false

    @ObservationIgnored private var loadID: UUID?
    @ObservationIgnored private var task: Task<Void, Never>?

    deinit {
        task?.cancel()
    }

    /// Loads the games' platforms, and their artwork in `language`, in
    /// batches. What is already known of them, such as the library cache's,
    /// shows until their batch replaces it.
    func load(appIds: [UInt32], language: String, session: any SteamSessionProtocol) {
        cancel()
        var seen: Set<UInt32> = []
        let appIds = appIds.filter { seen.insert($0).inserted }
        let kept = platforms.filter { seen.contains($0.key) }
        if kept.count != platforms.count { platforms = kept }
        // @Observable notifies no view of an equal value; only `platforms` bumps `revision` regardless.
        artwork.games = artwork.games.filter { seen.contains($0.key) }
        guard !appIds.isEmpty else { return }

        let loadID = UUID()
        self.loadID = loadID
        isLoading = true
        task = Task { [weak self] in
            defer {
                if self?.loadID == loadID {
                    self?.loadID = nil
                    self?.task = nil
                    self?.isLoading = false
                }
            }
            for start in stride(from: 0, to: appIds.count, by: 50) {
                guard !Task.isCancelled, self?.loadID == loadID else { return }
                let batch = Array(appIds[start..<min(start + 50, appIds.count)])
                do {
                    let result = try await session.platforms(appIds: batch, language: language)
                    // UniFFI can return after cancellation or account replacement.
                    guard !Task.isCancelled, let self, self.loadID == loadID else { return }
                    let merged = self.platforms.merging(result.map { ($0.appId, $0.platforms) }) { _, loaded in loaded }
                    if merged != self.platforms { self.platforms = merged }
                    self.artwork.games = self.artwork.games.merging(
                        result.compactMap { game in game.artwork.map { (game.appId, $0) } }
                    ) { _, loaded in loaded }
                } catch {
                    guard !Task.isCancelled, let self, self.loadID == loadID else { return }
                    self.hasFailures = true
                }
            }
        }
    }

    /// Shows what the library cache knew of the games, until a load replaces it.
    func show(platforms: [UInt32: [String]], artwork: [UInt32: SteamGameArtwork]) {
        reset()
        self.platforms = platforms
        self.artwork.games = artwork
    }

    /// Stops loading, keeping what loaded so far: the same account's next
    /// load refreshes it.
    func cancel() {
        task?.cancel()
        task = nil
        loadID = nil
        isLoading = false
        hasFailures = false
    }

    /// Forgets every game, as another account's library replaces them.
    func reset() {
        cancel()
        platforms.removeAll()
        artwork.games.removeAll()
    }
}
