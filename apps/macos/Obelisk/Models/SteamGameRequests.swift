import Foundation
import Observation

extension OverlayRequest {
    /// What Obelisk's overlay shows for a Steam game's request to Steam's
    /// overlay, by `account` while playing `appId`. Nil for its invite
    /// dialogs, which the friends list picks for, and for a page it cannot show.
    static func steam(_ kind: SteamOverlayKind, appId: UInt32, account: UInt64?) -> OverlayRequest? {
        switch kind {
        case .dialog(let dialog):
            switch dialog.lowercased() {
            case "friends", "players", "community":
                return .friends
            case let name where name.hasPrefix("chatroomgroup"):
                return .friends
            case "achievements", "stats":
                return .achievements
            case "officialgamegroup":
                return .webPage(GameOverlayController.communityURL(appId: appId))
            default:
                // "settings" and what Obelisk has no page for: the overlay itself.
                return .home
            }
        case .user(let dialog, let steamId):
            switch dialog.lowercased() {
            case "steamid", "chat":
                return .user(steamId: steamId)
            case "achievements", "stats":
                if steamId == account { return .achievements }
                return .webPage(GameOverlayController.achievementsURL(steamId: steamId, appId: appId))
            default:
                // Trading and friend requests happen on the person's profile.
                return .webPage(GameOverlayController.profileURL(steamId: steamId))
            }
        case .webPage(let url, _):
            guard let url = URL(string: url), let scheme = url.scheme?.lowercased(),
                  scheme == "https" || scheme == "http", url.host() != nil else { return nil }
            return .webPage(url)
        case .store(let appId, _):
            // Obelisk has no cart: the page lets the player add it there.
            return .webPage(GameOverlayController.storeURL(appId: appId))
        case .lobbyInvite, .connectInvite:
            return nil
        case .remotePlayTogetherInvite, .remotePlayTogether:
            return .friends
        }
    }
}

/// Follows the latest request a running Steam game's session shows, by its
/// sequence, which only grows within a session.
struct SessionSequence {
    private var followed: (session: String, sequence: UInt64)?

    /// Whether `session`'s request `sequence` is new. A request the session
    /// already showed when first seen is not; a request that went (nil)
    /// keeps its sequence.
    mutating func advance(session: String, sequence: UInt64?) -> Bool {
        let previous = followed?.session == session ? followed : nil
        followed = (session, sequence ?? previous?.sequence ?? 0)
        guard let previous, let sequence else { return false }
        return sequence > previous.sequence
    }
}

/// Invitations friends sent into the account's running Steam game, as its
/// Steam received them: the player joins one, or dismisses it here only.
@MainActor @Observable
final class SteamReceivedInvitations {
    /// An invitation, whenever its friend sent it.
    struct Key: Hashable {
        let friend: UInt64
        let invite: SteamGameInvite

        init(_ invite: SteamReceivedInvite) {
            friend = invite.friendSteamId
            self.invite = invite.invite
        }
    }

    /// The running game's invitations the player did not dismiss, newest first.
    private(set) var invites: [SteamReceivedInvite] = []
    /// The invitation being joined.
    private(set) var accepting: Key?
    /// Why joining an invitation failed.
    private(set) var failures: [Key: String] = [:]
    /// The running game's session.
    private(set) var session: String?

    @ObservationIgnored private let sessions: SteamGameSessions
    @ObservationIgnored private var received: [SteamReceivedInvite] = []
    /// When each dismissed invitation was received: sent again, it shows again.
    @ObservationIgnored private var dismissed: [Key: UInt64] = [:]

    init(sessions: SteamGameSessions) {
        self.sessions = sessions
    }

    /// Follows the running game's session `id` and its invitations. Returns
    /// those it did not show before, newest first; none when the session is
    /// first seen, such as one recovered.
    @discardableResult
    func follow(session id: String?, invites: [SteamReceivedInvite]) -> [SteamReceivedInvite] {
        guard let id else {
            if session != nil { session = nil }
            received = []
            dismissed = [:]
            if accepting != nil { accepting = nil }
            if !failures.isEmpty { failures = [:] }
            show()
            return []
        }
        let firstSight = session != id
        let known = Set(received.map(Key.init))
        if firstSight {
            session = id
            dismissed = [:]
            if accepting != nil { accepting = nil }
            if !failures.isEmpty { failures = [:] }
        }
        received = invites
        show()
        return firstSight ? [] : invites.filter { !known.contains(Key($0)) }
    }

    /// Hides `invite` until its friend sends it again.
    func dismiss(_ invite: SteamReceivedInvite) {
        let key = Key(invite)
        dismissed[key] = invite.receivedAt
        failures[key] = nil
        show()
    }

    /// Joins `invite`; true once the game is told.
    func accept(_ invite: SteamReceivedInvite) async -> Bool {
        guard let session, accepting == nil else { return false }
        let key = Key(invite)
        accepting = key
        failures[key] = nil
        do {
            try await sessions.acceptInvite(sessionId: session, friendSteamId: invite.friendSteamId, invite: invite.invite)
            guard self.session == session else { return false }
            accepting = nil
            return true
        } catch {
            guard self.session == session else { return false }
            accepting = nil
            failures[key] = SteamModel.message(error)
            return false
        }
    }

    private func show() {
        let visible = received.filter { invite in dismissed[Key(invite)].map { $0 < invite.receivedAt } ?? true }
        if invites != visible { invites = visible }
    }
}

/// The in-game purchase the account's running Steam game asks Steam to
/// authorize, until the player answers it.
@MainActor @Observable
final class SteamPurchaseAuthorization {
    /// The purchase waiting on the player.
    private(set) var pending: SteamMicroTxn?
    private(set) var isAnswering = false
    /// Why the last answer failed.
    private(set) var error: String?
    /// The running game's session.
    private(set) var session: String?

    @ObservationIgnored private let sessions: SteamGameSessions

    init(sessions: SteamGameSessions) {
        self.sessions = sessions
    }

    /// Follows the running game's session `id` and the purchase it waits on.
    /// True when another purchase starts waiting, except when the session is
    /// first seen, such as one recovered: opening the overlay shows it then.
    @discardableResult
    func follow(session id: String?, microTxn: SteamMicroTxn?) -> Bool {
        let firstSight = session != id
        if firstSight {
            session = id
            if isAnswering { isAnswering = false }
        }
        let current = id == nil ? nil : microTxn
        guard current?.orderId != pending?.orderId || firstSight else { return false }
        pending = current
        if error != nil { error = nil }
        return !firstSight && current != nil
    }

    /// The player's answer; true once the game is told.
    func answer(approve: Bool) async -> Bool {
        guard let session, let pending, !isAnswering else { return false }
        isAnswering = true
        error = nil
        defer { if self.session == session { isAnswering = false } }
        do {
            try await sessions.authorizeMicroTxn(sessionId: session, orderId: pending.orderId, approve: approve)
            return self.session == session
        } catch {
            if self.session == session && self.pending?.orderId == pending.orderId { self.error = SteamModel.message(error) }
            return false
        }
    }
}
