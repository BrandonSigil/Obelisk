import Foundation
@testable import Obelisk

// Older library/download fixtures have no social events. Suspend until the
// model cancels instead of making a network request or polling a fake cache.
extension SteamSessionProtocol {
    func achievements(appId: UInt32, language: String) async throws -> [SteamAchievement] {
        throw CocoaError(.featureUnsupported)
    }
    func platforms(appIds: [UInt32]) async throws -> [SteamGamePlatforms] { [] }
    func webCookies() async throws -> [SteamWebCookie] { [] }
    #if DEBUG
    func cloudAutocloudMetadata() async throws -> String {
        throw CocoaError(.featureUnsupported)
    }
    func cloudSdkCandidates() async throws -> String {
        throw CocoaError(.featureUnsupported)
    }
    func cloudInterruptionProbe(appId: UInt32, directory: String) async throws -> String {
        throw CocoaError(.featureUnsupported)
    }

    func cloudTransferProbe(appId: UInt32, directory: String) async throws -> String {
        throw CocoaError(.featureUnsupported)
    }
    #endif

    func cloudPreferences(appId: UInt32) async throws -> String {
        throw CocoaError(.featureUnsupported)
    }

    func cloudBackup(appId: UInt32, directory: String) async throws -> String {
        throw CocoaError(.featureUnsupported)
    }

    func nextFriends(afterRevision: UInt64) async -> SteamFriendsSnapshot {
        let stream = AsyncStream<Void>(bufferingPolicy: .bufferingNewest(1)) { _ in }
        for await _ in stream {}
        return SteamFriendsSnapshot(revision: afterRevision + 1, accountId: 0, connection: .disconnected, friends: [])
    }
    func shutdown() {}

    func setOnline() async throws {}
    func nextChat(afterSequence: UInt64) async -> SteamChatEvents {
        let stream = AsyncStream<Void>(bufferingPolicy: .bufferingNewest(1)) { _ in }
        for await _ in stream {}
        return SteamChatEvents(events: [], lastSequence: afterSequence, gap: false, connected: false)
    }
    func nextOwnGame(afterRevision: UInt64) async -> SteamOwnGame {
        let stream = AsyncStream<Void>(bufferingPolicy: .bufferingNewest(1)) { _ in }
        for await _ in stream {}
        return SteamOwnGame(revision: afterRevision + 1, connected: false, appId: 0, lobbyId: 0, connect: nil, invite: .none)
    }
    func refreshOwnGame() async throws {}
    func activeSessions() async throws -> [SteamChatSession] { [] }
    func recentMessages(friendSteamId: UInt64, before: SteamChatPosition?, count: UInt32) async throws -> SteamChatHistory {
        SteamChatHistory(messages: [], moreAvailable: false)
    }
    func sendMessage(friendSteamId: UInt64, text: String) async throws -> SteamSentMessage {
        throw CocoaError(.featureUnsupported)
    }
    func sendTyping(friendSteamId: UInt64) async throws {}
    func ack(friendSteamId: UInt64, timestamp: UInt32) async throws {}
    func inviteToGame(friendSteamId: UInt64, appId: UInt32) async throws -> SteamInviteKind {
        throw CocoaError(.featureUnsupported)
    }
    func inviteIntoGame(friendSteamId: UInt64, appId: UInt32, invite: SteamGameInvite) async throws -> SteamInviteKind {
        throw CocoaError(.featureUnsupported)
    }
}

/// Saves as Rust does: now, then after every rotation, in order.
private func saveEveryRotation(_ credentials: any CredentialSave, rotations: @escaping @Sendable (UInt64) async -> UInt64?) {
    Task.detached {
        credentials.save()
        var after: UInt64 = 0
        while let next = await rotations(after) {
            after = next
            credentials.save()
        }
    }
}

extension GogSessionProtocol {
    func persist(credentials: any CredentialSave) async {
        let session = self
        saveEveryRotation(credentials) { await session.nextCredentialsRevision(afterRevision: $0) }
    }
}

// Models take launches as injected closures, so a fake
// session never has to build a Rust-backed object.
extension EpicSessionProtocol {
    func webSignInUrl(redirectUrl: String) async throws -> String { throw CocoaError(.featureUnsupported) }
    func persist(credentials: any CredentialSave) async {
        let session = self
        saveEveryRotation(credentials) { await session.nextCredentialsRevision(afterRevision: $0) }
    }

    func achievements(namespace: String, locale: String) async throws -> [EpicAchievement] {
        throw CocoaError(.featureUnsupported)
    }
}
