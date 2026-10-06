import Foundation
import Observation

@MainActor
@Observable
final class SteamLibraryPlatforms {
    private(set) var platforms: [UInt32: [String]] = [:]
    private(set) var isLoading = false
    private(set) var hasFailures = false
    /// Each game's whole PICS app info, for the library cache.
    @ObservationIgnored private(set) var pics: [UInt32: JSONValue] = [:]

    @ObservationIgnored private var loadID: UUID?
    @ObservationIgnored private var task: Task<Void, Never>?

    deinit {
        task?.cancel()
    }

    /// Loads the games' platforms and PICS app info in batches. What is
    /// already known of them, such as the library cache's, shows until their
    /// batch replaces it; `finished` runs once the last batch answered.
    func load(appIds: [UInt32], session: any SteamSessionProtocol,
              finished: @escaping @MainActor () -> Void = {}) {
        cancel()
        var seen: Set<UInt32> = []
        let appIds = appIds.filter { seen.insert($0).inserted }
        platforms = platforms.filter { seen.contains($0.key) }
        pics = pics.filter { seen.contains($0.key) }
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
                    let result = try await session.platforms(appIds: batch)
                    // UniFFI can return after cancellation or account replacement.
                    guard !Task.isCancelled, let self, self.loadID == loadID else { return }
                    for game in result {
                        self.platforms[game.appId] = game.platforms
                        if let pics = JSONValue(json: game.pics) { self.pics[game.appId] = pics }
                    }
                } catch {
                    guard !Task.isCancelled, let self, self.loadID == loadID else { return }
                    self.hasFailures = true
                }
            }
            guard !Task.isCancelled, let self, self.loadID == loadID else { return }
            finished()
        }
    }

    /// Shows what the library cache knew of the games, until a load replaces it.
    func show(platforms: [UInt32: [String]], pics: [UInt32: JSONValue]) {
        reset()
        self.platforms = platforms
        self.pics = pics
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
        pics.removeAll()
    }
}
