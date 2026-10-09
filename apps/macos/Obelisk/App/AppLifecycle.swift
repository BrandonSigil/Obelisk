import AppKit
import SwiftUI

/// Quitting Obelisk, or closing its window, hides it: it keeps running in the
/// menu bar, as Steam does, so its Game sessions and Transfers go on. It really
/// quits from its menu bar item, after stopping a running game, or when macOS
/// logs out, restarts or shuts down.
@MainActor
final class AppLifecycle: NSObject, NSApplicationDelegate {
    /// Asks the menu bar item, which can open windows, to show the library.
    static let showLibrary = Notification.Name("com.zagrios.obelisk.showLibrary")
    /// How long quitting waits for stopped games to settle their Cloud saves.
    private static let settleTimeout: Duration = .seconds(60)

    /// Whether a game runs, and how to stop every one of them.
    var games: (running: @MainActor () -> Bool, stop: @MainActor () async -> Void) = ({ false }, {})
    /// The library cache, whose last changes are still being written.
    var libraryCache: LibraryCache?
    private var quitting = false
    private var windowClosing: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // An Obelisk that died took its games' sessions along, but Wine's
        // Windows processes froze on their own. None of them can belong to
        // this Obelisk yet.
        // The test host is no Obelisk of its own: it leaves every game alone.
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil {
            let locations = ObeliskLocations.live
            Task.detached(priority: .utility) { _ = endOrphanedWindowsGames(locations: locations) }
        }
        // Closing the last library window hides Obelisk like quitting does.
        windowClosing = NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification, object: nil, queue: .main
        ) { notification in
            let closing = notification.object as? NSWindow
            MainActor.assumeIsolated {
                let others = NSApp.windows.filter { $0 !== closing && $0.isVisible && $0.canBecomeMain }
                if others.isEmpty { NSApp.setActivationPolicy(.accessory) }
            }
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if quitting { return .terminateNow }
        guard Self.systemQuit else {
            hide()
            return .terminateCancel
        }
        // macOS waits while the games settle, within its own logout deadline.
        quitting = true
        Task {
            await stopGames()
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        libraryCache?.flush()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        show()
        return false
    }

    /// Shows the library window and Obelisk's Dock icon.
    func show() {
        NSApp.setActivationPolicy(.regular)
        NotificationCenter.default.post(name: Self.showLibrary, object: nil)
        NSApp.activate()
    }

    /// Closes Obelisk's windows and hides its Dock icon; it keeps running.
    func hide() {
        for window in NSApp.windows where window.canBecomeMain { window.close() }
        NSApp.setActivationPolicy(.accessory)
    }

    /// Really quits: a running game is stopped first, once the player agrees,
    /// and its Cloud saves settle before Obelisk exits.
    func quit() {
        guard !quitting else { return }
        if games.running() {
            NSApp.activate()
            let alert = NSAlert()
            alert.messageText = L10n.string("app.quit.running.title")
            alert.informativeText = L10n.string("app.quit.running.message")
            alert.addButton(withTitle: L10n.string("app.quit.stop"))
            alert.addButton(withTitle: L10n.string("ui.common.cancel"))
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        quitting = true
        Task {
            await stopGames()
            NSApp.terminate(nil)
        }
    }

    private func stopGames() async {
        guard games.running() else { return }
        await games.stop()
        let deadline = ContinuousClock.now + Self.settleTimeout
        while games.running() && ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(250))
        }
    }

    /// Whether macOS itself asks Obelisk to quit, to log out, restart or
    /// shut down, rather than the player.
    private static var systemQuit: Bool {
        // The 'why?' attribute of the quit Apple event names that reason.
        let reason = AEKeyword(0x7768_793F)
        return NSAppleEventManager.shared().currentAppleEvent?.attributeDescriptor(forKeyword: reason) != nil
    }
}

/// Obelisk's menu bar item, which stays while its windows are closed. It
/// tells the lifecycle which games run.
struct AppMenuBarLabel: View {
    let model: AppModel
    let lifecycle: AppLifecycle
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image("MenuBarIcon")
            .onAppear {
                lifecycle.libraryCache = model.libraryCache
                model.overlay.showLibrary = { lifecycle.show() }
                let (steam, epic, gog) = (model.steam.gameSessions, model.epic.gameSessions, model.gog.gameSessions)
                lifecycle.games = (
                    running: { steam.active != nil || epic.active != nil || gog.active != nil },
                    stop: {
                        await steam.stop()
                        await epic.stop()
                        await gog.stop()
                    }
                )
            }
            .onReceive(NotificationCenter.default.publisher(for: AppLifecycle.showLibrary)) { _ in
                openWindow(id: "main")
            }
    }
}

struct AppMenuBarMenu: View {
    let lifecycle: AppLifecycle

    var body: some View {
        Button(L10n.string("app.menu.open")) { lifecycle.show() }
        Divider()
        Button(L10n.string("app.menu.quit")) { lifecycle.quit() }
    }
}
