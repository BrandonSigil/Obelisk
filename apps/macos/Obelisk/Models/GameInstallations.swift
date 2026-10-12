import Foundation
import Observation

/// What a storefront adds to its installation actions.
protocol InstallationStorefront: GameStorefront {
    /// A failed answer, as the storefront's models describe errors.
    @MainActor static func describe(_ error: Error) -> String
    /// The game Rust names `id`.
    static func game(_ id: String) -> GameID?
    /// How Rust names `game`.
    static func name(_ game: GameID) -> String
    /// Shown on a game's page while another game's operation holds the storefront.
    static var otherGameOperation: String { get }
    /// How the storefront names a build for macOS, which Obelisk prefers.
    static var nativePlatform: String { get }
    /// Whether Obelisk syncs the storefront's Cloud saves.
    static var syncsCloud: Bool { get }
    /// Names a Cloud question or banner; the game's name follows it.
    static var cloudTitle: String { get }
}

extension InstallationStorefront {
    static var syncsCloud: Bool { true }
}

/// One installation of the account, as its Installation receipt in Rust
/// records it. Its id names it in every action.
struct InstallationReceipt<GameID: Hashable & Sendable>: Identifiable, Equatable, Sendable {
    let id: String
    let gameID: GameID
    let name: String
    let directory: String
    let platform: String
    let totalBytes: UInt64
    /// A Transfer started over it and has not succeeded since.
    var needsVerification = false
    /// The storefront offers a newer build than the installed one.
    var updateAvailable = false
}

/// The game on screen, as the storefront model sees it.
struct InstallationSelection<Store: InstallationStorefront> {
    let game: Store.GameID
    let platform: String
    /// The build Obelisk can install, once the storefront knows one.
    let offer: GameOffer?
}

/// What may start now, of one kind of action, as Rust's Reservation shows it.
enum InstallationOpening<GameID: Hashable>: Equatable {
    case closed
    /// It may start, for any game but these.
    case open(except: [GameID])

    func allows(_ game: GameID) -> Bool {
        switch self {
        case .closed: false
        case let .open(except): !except.contains(game)
        }
    }
}

/// What Rust's Reservation lets start now.
struct InstallationsGate<GameID: Hashable>: Equatable {
    /// A Transfer, which waits in the download queue.
    var modify: InstallationOpening<GameID>
    /// An uninstall: no Transfer, launch or other uninstall of the storefront runs.
    var uninstall: InstallationOpening<GameID>
    /// A launch: a Transfer holds only its own game.
    var launch: InstallationOpening<GameID>
    var maintain: InstallationOpening<GameID>

    static var closed: Self { Self(modify: .closed, uninstall: .closed, launch: .closed, maintain: .closed) }

    init(modify: InstallationOpening<GameID>, uninstall: InstallationOpening<GameID>,
         launch: InstallationOpening<GameID>, maintain: InstallationOpening<GameID>) {
        self.modify = modify
        self.uninstall = uninstall
        self.launch = launch
        self.maintain = maintain
    }

    /// Rust's gate, whose games `game` names.
    init(_ gate: InstallationGate, game: (String) -> GameID?) {
        func opening(_ opening: GameOpening) -> InstallationOpening<GameID> {
            switch opening {
            case .closed: .closed
            case let .open(except): .open(except: except.compactMap(game))
            }
        }
        self.init(modify: opening(gate.modify), uninstall: opening(gate.uninstall), launch: opening(gate.launch),
                  maintain: opening(gate.maintain))
    }
}

/// A signed-in account's storefront operations. Every closure belongs to one
/// connection; signing out drops them.
struct InstallationPort<Store: InstallationStorefront> {
    /// Presents a failure; nil clears the last one.
    var report: @MainActor (Error?) -> Void
    var warn: @MainActor (String) -> Void
    /// The game's last installation left the receipts.
    var uninstalled: @MainActor (Store.GameID) -> Void = { _ in }
}

/// Everything one account's installations present, as Rust keeps them.
struct InstallationsSnapshot<Store: InstallationStorefront>: Equatable {
    var revision: UInt64
    var receipts: [InstallationReceipt<Store.GameID>]
    var transfer: GameTransfer?
    var transferGame: Store.GameID?
    var downloading: Bool
    var launching: Bool
    var verifyingBeforeLaunch: Bool
    var launchGame: Store.GameID?
    var uninstalling: Bool
    var uninstallGame: Store.GameID?
    /// The game whose Transfer stopped before its receipt.
    var resumable: Store.GameID?
    var report: InstallationReport?
    var warning: InstallationWarning?
    var gate: InstallationsGate<Store.GameID>
    var cloudSync: GameCloudSync<Store.GameID>?
    /// The games whose Transfer waits in the download queue.
    var queued: [Store.GameID] = []
}

/// One account's installations of one storefront, in Rust. Each action
/// returns at once; the snapshot tells how it goes.
@MainActor
protocol InstallationsBackend<Store>: AnyObject {
    associatedtype Store: InstallationStorefront
    func snapshot() -> InstallationsSnapshot<Store>
    /// Waits for a revision newer than `after`.
    func changed(after: UInt64) async -> InstallationsSnapshot<Store>
    func setDiagnostic(_ running: Bool)
    /// Installs `offer`'s build, or resumes the game's interrupted Transfer.
    func install(_ offer: GameOffer)
    /// Verifies `receipt`'s files and repairs or updates them in place, or
    /// resumes the game's interrupted Transfer.
    func update(_ receipt: InstallationReceipt<Store.GameID>)
    /// Stops `game`'s Transfer, or its launch still preparing.
    func cancel(_ game: Store.GameID)
    /// Asks the storefronts which installations have a newer build, unless
    /// they were asked a short while ago.
    func checkUpdates()
    /// Whether each newer build installs as soon as it is known, rather than
    /// at the game's next launch.
    func setAutomaticUpdates(_ enabled: Bool)
    /// Launches `receipt`'s installation, updating or repairing it first
    /// when it needs it.
    func launch(_ receipt: InstallationReceipt<Store.GameID>)
    func uninstall(_ receipt: InstallationReceipt<Store.GameID>)
    func signedOut()
    /// Syncs the game's Cloud saves: those of `installed`, or an uninstalled
    /// game's when nil.
    func maintainCloud(_ game: Store.GameID, installed: InstallationReceipt<Store.GameID>?)
    /// Answers the Cloud question the game shows: its maintenance's or its
    /// Game session's.
    func answerCloud(_ game: Store.GameID, decisionId: UInt64, choice: CloudChoice) async throws
    /// The games' Cloud indicators, and those of every other game of the
    /// account with pending work.
    func cloudIndicators(_ games: [Store.GameID]) async throws -> [(Store.GameID, CloudIndicator)]
}

/// One storefront's installed games and the actions on them, as Rust keeps
/// them: Transfers, launches and uninstalls. This presents them for
/// the game on screen and forwards the player's actions.
@MainActor @Observable
final class GameInstallations<Store: InstallationStorefront> {
    typealias Receipt = InstallationReceipt<Store.GameID>

    private(set) var receipts: [Receipt] = [] {
        // Every path that removes a game's last installation reports it here,
        // so no caller has to remember to. Each change is one assignment, so
        // replacing a receipt never reads as a removal.
        didSet {
            for game in Set(oldValue.map(\.gameID)).subtracting(receipts.map(\.gameID)) { port?.uninstalled(game) }
        }
    }
    private(set) var transfer: GameTransfer?
    /// The game the last Transfer progress belongs to.
    private(set) var transferGame: Store.GameID?
    private(set) var isDownloading = false
    private(set) var isLaunching = false
    private(set) var isVerifyingBeforeLaunch = false
    private(set) var launchGame: Store.GameID?
    private(set) var isUninstalling = false
    private(set) var uninstallGame: Store.GameID?
    /// The game whose Transfer stopped before its receipt: the player can
    /// resume it.
    private(set) var resumableGame: Store.GameID?
    /// The games whose Transfer waits in the download queue.
    private(set) var queuedGames: [Store.GameID] = []
    /// What Rust's Reservation lets start now.
    private(set) var gate = InstallationsGate<Store.GameID>.closed
    /// A DEBUG diagnostic fences the storefront like a Game session would.
    var diagnosticReservation = false {
        didSet { backend?.setDiagnostic(diagnosticReservation) }
    }
    /// The player keeps games up to date: each newer build installs as soon
    /// as it is known.
    var updatesAutomatically = false {
        didSet { if updatesAutomatically != oldValue { backend?.setAutomaticUpdates(updatesAutomatically) } }
    }
    @ObservationIgnored var selection: @MainActor () -> InstallationSelection<Store>? = { nil }

    let sessions: GameSessions<Store>
    let cloud: CloudModel<Store>
    private var account: Store.AccountID?
    @ObservationIgnored private var backend: (any InstallationsBackend<Store>)?
    @ObservationIgnored private var port: InstallationPort<Store>?
    @ObservationIgnored private var revision: UInt64 = 0
    @ObservationIgnored private var report: InstallationReport?
    @ObservationIgnored private var warning: InstallationWarning?
    @ObservationIgnored private var observation: Task<Void, Never>?
    /// Asks for newer builds each time Obelisk comes to the front.
    @ObservationIgnored private var activity: ApplicationActivityObservation?
    /// Whether a Game session was active, and its game running, when last seen.
    @ObservationIgnored private var observedSession: (busy: Bool, running: Bool)?

    init(sessions: GameSessions<Store>, cloud: CloudModel<Store>) {
        self.sessions = sessions
        self.cloud = cloud
        if Store.syncsCloud { observeSessions() }
    }

    /// The game on screen's Cloud indicator is read again once a Game
    /// session ends, and once its game runs: the game's guard and launch
    /// sync are on disk then.
    private func observeSessions() {
        let observed = withObservationTracking {
            (busy: sessions.isBusy, running: sessions.activeGame?.running == true)
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in self?.observeSessions() }
        }
        defer { observedSession = observed }
        guard let previous = observedSession else { return }
        if (previous.busy && !observed.busy) || (!previous.running && observed.running) { refreshCloudIndicator() }
    }

    func connect(account: Store.AccountID, backend: any InstallationsBackend<Store>, port: InstallationPort<Store>) {
        reset()
        self.account = account
        self.backend = backend
        self.port = port
        cloud.connect(account: account, backend: backend)
        backend.setDiagnostic(diagnosticReservation)
        backend.setAutomaticUpdates(updatesAutomatically)
        apply(backend.snapshot())
        backend.checkUpdates()
        activity = ApplicationActivityObservation { [weak self] active in
            guard active else { return }
            Task { @MainActor in self?.backend?.checkUpdates() }
        }
        observation = Task { [weak self, backend] in
            var after = self?.revision ?? 0
            while !Task.isCancelled {
                let snapshot = await backend.changed(after: after)
                guard let self, self.backend === backend else { return }
                after = snapshot.revision
                apply(snapshot)
            }
        }
    }

    /// Stops the account's Transfer. A running game keeps its Game session.
    func signedOut() {
        backend?.signedOut()
        reset()
    }

    private func reset() {
        observation?.cancel()
        observation = nil
        activity = nil
        port = nil
        backend = nil
        account = nil
        revision = 0
        report = nil
        warning = nil
        receipts = []
        transfer = nil
        transferGame = nil
        resumableGame = nil
        queuedGames = []
        gate = .closed
        isDownloading = false
        isLaunching = false
        isVerifyingBeforeLaunch = false
        launchGame = nil
        isUninstalling = false
        uninstallGame = nil
    }

    private func apply(_ snapshot: InstallationsSnapshot<Store>) {
        guard snapshot.revision > revision else { return }
        revision = snapshot.revision
        // Repeated snapshots do not invalidate unchanged presentation.
        if receipts != snapshot.receipts { receipts = snapshot.receipts }
        if transfer != snapshot.transfer { transfer = snapshot.transfer }
        if transferGame != snapshot.transferGame { transferGame = snapshot.transferGame }
        if isDownloading != snapshot.downloading { isDownloading = snapshot.downloading }
        if isLaunching != snapshot.launching { isLaunching = snapshot.launching }
        if isVerifyingBeforeLaunch != snapshot.verifyingBeforeLaunch {
            isVerifyingBeforeLaunch = snapshot.verifyingBeforeLaunch
        }
        if launchGame != snapshot.launchGame { launchGame = snapshot.launchGame }
        if isUninstalling != snapshot.uninstalling { isUninstalling = snapshot.uninstalling }
        if uninstallGame != snapshot.uninstallGame { uninstallGame = snapshot.uninstallGame }
        if resumableGame != snapshot.resumable { resumableGame = snapshot.resumable }
        if queuedGames != snapshot.queued { queuedGames = snapshot.queued }
        if gate != snapshot.gate { gate = snapshot.gate }
        if cloud.sync != snapshot.cloudSync { cloud.sync = snapshot.cloudSync }
        if report != snapshot.report {
            report = snapshot.report
            if let report { port?.report(Self.failure(report)) }
        }
        if warning != snapshot.warning {
            warning = snapshot.warning
            if let warning { port?.warn(L10n.string(warning.key)) }
        }
    }

    private static func failure(_ report: InstallationReport) -> Error {
        switch report {
        case let .storefront(key): Store.failure(key)
        case let .downloadDirectory(message): GameInstallationFailure.downloadDirectory(message)
        case let .uninstall(reason): GameInstallationFailure.uninstall(reason)
        }
    }

    /// Runs an action on the account's installations, then shows its state.
    private func act(_ action: (any InstallationsBackend<Store>) -> Void) {
        guard let backend else { return }
        port?.report(nil)
        action(backend)
        apply(backend.snapshot())
        sessions.refresh()
    }

    // MARK: Reservation gate

    /// Whether a Transfer of `game` may join the download queue, as Rust's
    /// Reservation shows it.
    func canModify(_ game: Store.GameID?) -> Bool {
        game.map(gate.modify.allows) ?? false
    }

    /// Whether `game`'s Transfer waits in the download queue.
    func isQueued(_ game: Store.GameID) -> Bool { queuedGames.contains(game) }

    /// The running Transfer's progress, when it belongs to the game Rust names `game`.
    func transfer(named game: String) -> GameTransfer? {
        Store.game(game).flatMap(transfer(for:))
    }

    /// Whether `game` may launch: no Game session runs, and no Transfer holds it.
    func canLaunch(_ game: Store.GameID) -> Bool {
        gate.launch.allows(game)
    }

    /// Whether the player may sync `game`'s Cloud saves.
    func canMaintainCloud(_ game: Store.GameID) -> Bool {
        Store.syncsCloud && account != nil && gate.maintain.allows(game)
    }

    /// Identifies the operation before its first progress snapshot arrives.
    var activeTransferGame: Store.GameID? {
        if isDownloading { return transferGame }
        if isLaunching && isVerifyingBeforeLaunch { return launchGame }
        return nil
    }

    /// The last Transfer progress, when it belongs to `game`.
    func transfer(for game: Store.GameID) -> GameTransfer? {
        transferGame == game ? transfer : nil
    }

    func receipt(for game: Store.GameID, platform: String) -> Receipt? {
        receipts.first { $0.gameID == game && $0.platform == platform }
    }

    /// The game's installation for `platform`, else its other one.
    func receipt(for game: Store.GameID, preferring platform: String) -> Receipt? {
        receipt(for: game, platform: platform) ?? receipts.first { $0.gameID == game }
    }

    // MARK: The game on screen

    /// The installation of the selected platform, else the game's other one: an
    /// installed build stays on screen whatever the storefront would install now.
    var selectedReceipt: Receipt? {
        selection().flatMap { receipt(for: $0.game, preferring: $0.platform) }
    }

    /// The active Game session when it is this account's, for the game on screen.
    var selectedSession: GameSessions<Store>.Session? {
        guard let active = sessions.active, active.account == account,
              active.game == selection()?.game else { return nil }
        return active
    }

    var canResumeDownload: Bool {
        guard let resumableGame else { return false }
        return canModify(resumableGame) && resumableGame == selection()?.game
    }

    /// Whether the game on screen's installation may be verified.
    var canVerifySelectedGame: Bool {
        account != nil && selectedReceipt != nil && canModify(selection()?.game)
    }

    /// Whether the game on screen's installation may be uninstalled.
    var canUninstallSelectedGame: Bool {
        guard account != nil, selectedReceipt != nil, let game = selection()?.game else { return false }
        return gate.uninstall.allows(game)
    }

    var isSynchronizingSaves: Bool {
        selection().map { cloud.isSynchronizing(for: $0.game) } ?? false
    }

    var canSyncCloud: Bool { selection().map { canMaintainCloud($0.game) } ?? false }

    /// Syncs the Cloud saves of the game on screen: of its installation, or
    /// of an uninstalled game.
    func syncCloud() {
        guard canSyncCloud, let selection = selection() else { return }
        let installed = selectedReceipt
        act { $0.maintainCloud(selection.game, installed: installed) }
    }

    /// Asks Rust again for the Cloud indicator of the game on screen.
    func refreshCloudIndicator() {
        guard Store.syncsCloud, let game = selection()?.game else { return }
        cloud.refreshIndicators(for: [game])
    }

    /// The game on screen's Cloud indicator, or nil when it is hidden.
    var cloudIndicator: CloudIndicatorPresentation? {
        guard Store.syncsCloud, let game = selection()?.game, account != nil else { return nil }
        let playing = sessions.activeGame.map { $0.game == game && $0.account == account } ?? false
        return CloudIndicatorPresentation.make(saved: cloud.indicator(for: game), isSyncing: isSynchronizingSaves,
                                               isPlaying: playing, isInstalled: selectedReceipt != nil)
    }

    var primaryActionTitle: String {
        let game = selection()?.game
        if isSynchronizingSaves { return L10n.string("ui.cloud.sync_short") }
        if sessions.isRecovering { return L10n.string(Store.recoveringSession) }
        if let quitStage = primaryQuitStage { return quitStage.title }
        if selectedSession?.running == true { return L10n.string("model.game.running") }
        if isUninstalling && uninstallGame == game { return L10n.string("model.game.uninstalling") }
        if isLaunching && launchGame == game {
            switch runtimeInstall?.component {
            case .voile: return L10n.string("model.game.installingVoile")
            case .steamComponents: return L10n.string("model.game.installingSteamComponents")
            case nil: break
            }
            return L10n.string(isVerifyingBeforeLaunch ? "model.game.verifying" : "model.game.launching")
        }
        if let game, isQueued(game) { return L10n.string("model.game.queued") }
        if selectedSession != nil { return L10n.string("model.game.launching") }
        if let selectedReceipt {
            return L10n.string(selectedReceipt.updateAvailable ? "model.game.update" : "model.game.launch")
        }
        return L10n.string(canResumeDownload ? "model.game.resume" : "model.game.download")
    }

    /// Which runtime downloads while the game on screen waits for it, and how much.
    var runtimeInstall: RuntimeInstallState? {
        guard isLaunching, let game = launchGame, game == selection()?.game,
              let active = sessions.active, active.game == game else { return nil }
        return active.runtimeInstall
    }

    var runtimeInstallFraction: Double? { runtimeInstall?.fraction }

    /// Whether the primary action launches the game rather than installing or
    /// updating it.
    var primaryActionLaunches: Bool {
        selectedSession != nil || selectedReceipt.map { !$0.updateAvailable } ?? false
    }

    /// While the game on screen runs, the primary action ends it instead.
    var primaryQuitStage: GameQuitStage? {
        selectedSession?.running == true ? sessions.quitStage : nil
    }

    /// An installation launches from its receipt; only a new download waits for
    /// the storefront to know what it can install.
    var primaryActionAvailable: Bool {
        if let primaryQuitStage { return primaryQuitStage != .closing }
        guard let selection = selection(), account != nil, !isQueued(selection.game) else { return false }
        if let selectedReceipt {
            return selectedReceipt.updateAvailable ? canModify(selection.game) : canLaunch(selection.game)
        }
        return selection.offer != nil && canModify(selection.game)
    }

    /// Launches the installed game, updates it when the storefront offers a
    /// newer build, otherwise installs it. While the game runs, it quits it,
    /// then forces it to quit once it may: the page asks its player to
    /// confirm that first.
    func primaryAction() {
        guard primaryActionAvailable else { return }
        if primaryQuitStage != nil {
            endRunningGame()
        } else if let receipt = selectedReceipt, !receipt.updateAvailable {
            launch(receipt)
        } else {
            downloadSelected()
        }
    }

    /// Quit, or Force Quit once the running game may be forced, from its
    /// page or from the library. Its player confirms a Force Quit first.
    func endRunningGame() {
        guard let active = sessions.active else { return }
        switch sessions.quitStage {
        case .quit:
            // Only a Mac build runs from its own installation.
            let macApps = active.account != account ? [] : receipts
                .filter { $0.gameID == active.game && $0.platform == Store.nativePlatform }
                .map(\.directory)
            Task { await sessions.quit(macApps: macApps) }
        case .forceQuit: forceQuitRunningGame()
        case .closing, nil: break
        }
    }

    /// The Force Quit its player confirmed, while the running game may still
    /// be forced.
    func forceQuitRunningGame() {
        Task { await sessions.forceQuit() }
    }

    /// Launches an installation, from its game's page or from the library.
    func launch(_ receipt: Receipt) {
        guard account != nil, canLaunch(receipt.gameID), receipts.contains(receipt) else { return }
        act { $0.launch(receipt) }
    }

    /// Whether the library may install `game`: no build of it is installed
    /// and no operation holds it.
    func canInstall(_ game: Store.GameID) -> Bool {
        account != nil && canModify(game) && !receipts.contains { $0.gameID == game }
    }

    /// Installs a game from the library, where no game is on screen. Rust
    /// resumes the game's interrupted Transfer instead, and picks every folder.
    func install(_ offer: GameOffer, of game: Store.GameID) {
        guard canInstall(game) else { return }
        act { $0.install(offer) }
    }

    /// Installs the game on screen, or updates its installation in place with
    /// the installed build. Rust resumes the game's interrupted Transfer
    /// instead, and picks every folder.
    func downloadSelected() {
        guard let selection = selection(), canModify(selection.game) else { return }
        if let receipt = selectedReceipt {
            act { $0.update(receipt) }
        } else if let offer = selection.offer {
            act { $0.install(offer) }
        }
    }

    // MARK: Transfers

    /// Stops `game`'s Transfer, or its launch still preparing. Another game's
    /// operation goes on, and a running game keeps its Game session.
    func cancel(_ game: Store.GameID) {
        let transfers = isDownloading && transferGame == game
        guard transfers || (isLaunching && launchGame == game) || isQueued(game), let backend else { return }
        backend.cancel(game)
        apply(backend.snapshot())
        sessions.refresh()
    }

    // MARK: Verification and uninstall

    /// Verifies an installation's files, from its game's page: the Transfer
    /// repairs or updates them in place.
    func verify(_ receipt: Receipt) {
        guard account != nil, canModify(receipt.gameID), receipts.contains(receipt) else { return }
        act { $0.update(receipt) }
    }

    func uninstall(_ receipt: Receipt) {
        guard account != nil, gate.uninstall.allows(receipt.gameID), receipts.contains(receipt) else { return }
        act { $0.uninstall(receipt) }
    }
}

extension TransferState {
    var isActive: Bool { self == .preparing || self == .verifying || self == .downloading }
}

extension SteamGames: InstallationStorefront {
    static let nativePlatform = "macos"
    static func game(_ id: String) -> UInt32? { UInt32(id) }
    static func name(_ game: UInt32) -> String { String(game) }
    static let otherGameOperation = "ui.steam.other_game_operation"
    @MainActor static func describe(_ error: Error) -> String { SteamModel.message(error) }
    static var cloudTitle: String { L10n.string("ui.steam.cloud.title") }
}

extension EpicGames: InstallationStorefront {
    static let nativePlatform = "Mac"
    /// Rust names an Epic game as its library ID does.
    static func game(_ id: String) -> String? { epicGameID(id) == nil ? nil : id }
    static func name(_ game: String) -> String { game }
    static let otherGameOperation = "ui.epic.other_game_operation"
}

extension GogGames: InstallationStorefront {
    static let nativePlatform = "macos"
    /// GOG product ids are decimal numbers.
    static func game(_ id: String) -> String? {
        !id.isEmpty && id.utf8.count <= 20 && id.utf8.allSatisfy { (48...57).contains($0) } ? id : nil
    }
    static func name(_ game: String) -> String { game }
    static let otherGameOperation = "ui.gog.other_game_operation"
    /// GOG Cloud saves need each game's own Galaxy credentials: not yet.
    static let syncsCloud = false
}

typealias SteamInstallations = GameInstallations<SteamGames>
typealias EpicInstallations = GameInstallations<EpicGames>
typealias GogInstallations = GameInstallations<GogGames>

/// The installations of a session that is not Rust's: it has none, and every
/// action does nothing.
final class NoInstallations<Store: InstallationStorefront>: InstallationsBackend {
    private let empty = InstallationsSnapshot<Store>(
        revision: 1, receipts: [], transfer: nil, transferGame: nil, downloading: false, launching: false,
        verifyingBeforeLaunch: false, launchGame: nil, uninstalling: false, uninstallGame: nil, resumable: nil,
        report: nil, warning: nil, gate: .closed, cloudSync: nil
    )

    func snapshot() -> InstallationsSnapshot<Store> { empty }
    func changed(after: UInt64) async -> InstallationsSnapshot<Store> {
        // Nothing ever changes: wait until the observer goes.
        while !Task.isCancelled { try? await Task.sleep(for: .seconds(3600)) }
        return empty
    }
    func setDiagnostic(_ running: Bool) {}
    func install(_ offer: GameOffer) {}
    func update(_ receipt: InstallationReceipt<Store.GameID>) {}
    func cancel(_ game: Store.GameID) {}
    func checkUpdates() {}
    func setAutomaticUpdates(_ enabled: Bool) {}
    func launch(_ receipt: InstallationReceipt<Store.GameID>) {}
    func uninstall(_ receipt: InstallationReceipt<Store.GameID>) {}
    func signedOut() {}
    func maintainCloud(_ game: Store.GameID, installed: InstallationReceipt<Store.GameID>?) {}
    func answerCloud(_ game: Store.GameID, decisionId: UInt64, choice: CloudChoice) async throws {
        throw Store.failure(Store.invalidSession)
    }
    func cloudIndicators(_ games: [Store.GameID]) async throws -> [(Store.GameID, CloudIndicator)] { [] }
}

/// The file that keeps `account`'s Installation receipts in `directory`.
func receiptsFile(in directory: URL, storefront: String, account: String) -> URL {
    // Account identifiers are opaque. Encode them before using them in a file name.
    let component = account.utf8.map { String(format: "%02x", $0) }.joined()
    return directory.appending(path: "\(storefront)-installations-\(component).json")
}

/// One account's installations in Rust, of either storefront: games cross
/// under the names Rust gives them, installations under their receipt's id.
final class RustInstallations<Store: InstallationStorefront>: InstallationsBackend {
    private let core: any AccountInstallationsProtocol

    init(_ core: any AccountInstallationsProtocol) { self.core = core }

    func snapshot() -> InstallationsSnapshot<Store> { Self.snapshot(core.state()) }
    func changed(after: UInt64) async -> InstallationsSnapshot<Store> {
        Self.snapshot(await core.changed(after: after))
    }
    func setDiagnostic(_ running: Bool) { core.setDiagnostic(running: running) }
    func install(_ offer: GameOffer) { core.install(offer: offer) }
    func update(_ receipt: InstallationReceipt<Store.GameID>) { core.update(installation: receipt.id) }
    func cancel(_ game: Store.GameID) { core.cancel(game: Store.name(game)) }
    func checkUpdates() { core.checkUpdates() }
    func setAutomaticUpdates(_ enabled: Bool) { core.setAutomaticUpdates(enabled: enabled) }
    func launch(_ receipt: InstallationReceipt<Store.GameID>) { core.launch(installation: receipt.id) }
    func uninstall(_ receipt: InstallationReceipt<Store.GameID>) { core.uninstall(installation: receipt.id) }
    func signedOut() { core.signedOut() }
    func maintainCloud(_ game: Store.GameID, installed: InstallationReceipt<Store.GameID>?) {
        core.maintainCloud(game: Store.name(game), installation: installed?.id)
    }
    func answerCloud(_ game: Store.GameID, decisionId: UInt64, choice: CloudChoice) async throws {
        do { try await core.answerCloud(game: Store.name(game), decisionId: decisionId, choice: choice) }
        catch { throw GameSessions<Store>.translated(error) }
    }
    func cloudIndicators(_ games: [Store.GameID]) async throws -> [(Store.GameID, CloudIndicator)] {
        try await core.cloudIndicators(games: games.map(Store.name)).compactMap { shown in
            Store.game(shown.game).map { ($0, shown.indicator) }
        }
    }

    private static func snapshot(_ state: InstallationsState) -> InstallationsSnapshot<Store> {
        InstallationsSnapshot(
            revision: state.revision,
            receipts: state.installations.compactMap { installation in
                Store.game(installation.game).map {
                    InstallationReceipt(id: installation.id, gameID: $0, name: installation.name,
                                        directory: installation.directory, platform: installation.platform,
                                        totalBytes: installation.totalBytes,
                                        needsVerification: installation.needsVerification,
                                        updateAvailable: installation.updateAvailable)
                }
            },
            transfer: state.transfer, transferGame: state.transferGame.flatMap(Store.game),
            downloading: state.downloading, launching: state.launching,
            verifyingBeforeLaunch: state.verifyingBeforeLaunch, launchGame: state.launchGame.flatMap(Store.game),
            uninstalling: state.uninstalling, uninstallGame: state.uninstallGame.flatMap(Store.game),
            resumable: state.resumable.flatMap(Store.game), report: state.report, warning: state.warning,
            gate: InstallationsGate(state.gate, game: Store.game),
            cloudSync: state.cloudSync.flatMap { sync in
                Store.game(sync.game).map {
                    GameCloudSync(game: $0, kind: sync.kind, decisionId: sync.decisionId, working: sync.working,
                                  reconnecting: sync.reconnecting, cancellable: sync.cancellable,
                                  question: sync.question)
                }
            },
            queued: state.queued.compactMap(Store.game)
        )
    }
}
