import SwiftUI
import WebKit

/// The overlay over the dimmed game: its bar on top, the friends of the
/// game's storefront on the leading side, and the page it shows.
struct GameOverlayView: View {
    let overlay: GameOverlayController
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        ZStack {
            backdrop
            if overlay.session != nil {
                VStack(spacing: 18) {
                    OverlayTopBar(overlay: overlay)
                    HStack(alignment: .top, spacing: 18) {
                        if let friends = overlay.friends {
                            FriendsPanel(state: friends, avatars: overlay.avatars, chat: overlay.chat,
                                         picker: overlay.invitePicker, closable: false,
                                         accessory: showsHome ? nil : AnyView(OverlayInvitations(overlay: overlay, compact: true)),
                                         focus: overlay.focus,
                                         dismiss: overlay.hide, retry: overlay.retryFriends,
                                         signIn: overlay.openLibrary)
                                .frame(width: 400)
                                .id(friends.account)
                        }
                        page.frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
                .padding(32)
                .frame(maxWidth: 1480, maxHeight: 960)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L10n.string("ui.overlay.title"))
        .accessibilityIdentifier("game-overlay")
    }

    private var backdrop: some View {
        Rectangle()
            .fill(reduceTransparency ? AnyShapeStyle(Color.black.opacity(0.82)) : AnyShapeStyle(.ultraThinMaterial))
            .overlay { Color.black.opacity(reduceTransparency ? 0 : 0.45) }
            .ignoresSafeArea()
            .accessibilityHidden(true)
    }

    /// The home page shows the invitations into the game; other pages leave
    /// them to the friends column.
    private var showsHome: Bool {
        switch overlay.request {
        case .achievements, .webPage: false
        case .microTxn(let orderId): overlay.pendingPurchase?.orderId != orderId
        case .home, .friends, .invite, .user, .store: true
        }
    }

    @ViewBuilder private var page: some View {
        switch overlay.request {
        case .microTxn(let orderId) where overlay.pendingPurchase?.orderId == orderId:
            OverlayPurchasePage(overlay: overlay, purchase: overlay.steam.purchase)
        case .achievements:
            if let achievements = overlay.achievements {
                OverlayAchievementsPage(model: achievements, gameTitle: overlay.gameTitle,
                                        back: { overlay.back() }, retry: overlay.retryAchievements)
            }
        case .webPage(let url):
            OverlayWebPage(url: url, cookies: { await overlay.webCookies(for: url) }, close: { overlay.back() })
                .id(url)
        case .home, .friends, .invite, .user, .store, .microTxn:
            OverlayHome(overlay: overlay)
        }
    }
}

/// The running game, its storefront, how long it has run, the time, and how
/// to close the overlay.
private struct OverlayTopBar: View {
    let overlay: GameOverlayController

    var body: some View {
        HStack(spacing: 14) {
            OverlayGameArtwork(overlay: overlay, maxPixelSize: 400)
                .frame(width: 96, height: 45)
                .clipShape(RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 3) {
                Text(overlay.gameTitle)
                    .font(.system(size: 16, weight: .semibold))
                    .lineLimit(1)
                HStack(spacing: 6) {
                    if let store = overlay.session.flatMap({ LibrarySection(rawValue: $0.storeID) }) {
                        if let logo = store.logoAssetName {
                            Image(logo).resizable().scaledToFit().frame(width: 13, height: 13).accessibilityHidden(true)
                        }
                        Text(store.title)
                    }
                    if let startedAt = overlay.session?.startedAt {
                        Text(verbatim: "·").accessibilityHidden(true)
                        TimelineView(.everyMinute) { context in
                            Text(L10n.string("ui.overlay.playingFor",
                                             OverlayFormatting.duration(from: startedAt, to: context.date)))
                        }
                    }
                }
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
            Spacer(minLength: 16)
            TimelineView(.everyMinute) { context in
                Text(context.date, format: .dateTime.hour().minute().locale(L10n.locale))
            }
            .font(.system(size: 22, weight: .semibold))
            .monospacedDigit()
            Text(L10n.string("ui.overlay.closeHint"))
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .frame(height: 26)
                .background(.primary.opacity(0.08), in: Capsule())
            Button(action: overlay.hide) {
                StorefrontGlassIcon(systemName: "xmark", size: 13)
                    .foregroundStyle(.primary)
            }
            .obeliskButtonStyle(.plain)
            .accessibilityLabel(L10n.string("ui.overlay.close"))
            .accessibilityIdentifier("game-overlay-close")
            .help(L10n.string("ui.overlay.close"))
        }
        .padding(.leading, 12)
        .padding(.trailing, 14)
        .frame(height: 70)
        .modifier(StorefrontPanelGlass(cornerRadius: 24))
    }
}

/// The running game's wide artwork, as its storefront publishes it.
private struct OverlayGameArtwork: View {
    let overlay: GameOverlayController
    let maxPixelSize: Int

    var body: some View {
        if case .steam(let appId)? = overlay.session?.game {
            SteamArtwork(appId: appId, portrait: false)
        } else {
            StorefrontArtwork(imageURL: overlay.gameArtwork, maxPixelSize: maxPixelSize)
        }
    }
}

/// The game's artwork, and what the overlay opens for it.
private struct OverlayHome: View {
    let overlay: GameOverlayController

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            OverlayGameArtwork(overlay: overlay, maxPixelSize: 2048)
                .frame(maxWidth: .infinity)
                .frame(height: 300)
                .clipped()
                .overlay(alignment: .bottomLeading) {
                    Text(overlay.gameTitle)
                        .font(.system(size: 30, weight: .bold))
                        .foregroundStyle(.white)
                        .shadow(color: .black.opacity(0.5), radius: 8, y: 2)
                        .lineLimit(2)
                        .padding(24)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(LinearGradient(colors: [.clear, .black.opacity(0.65)], startPoint: .top, endPoint: .bottom))
                }
            // Friends' invitations come first; many of them scroll.
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    OverlayInvitations(overlay: overlay)
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 190), spacing: 12)], alignment: .leading, spacing: 12) {
                        OverlayTile(title: L10n.string("ui.game.achievements.title"), systemImage: "trophy",
                                    identifier: "game-overlay-achievements") { overlay.show(.achievements) }
                        if case .steam(let appId)? = overlay.session?.game {
                            OverlayTile(title: L10n.string("ui.overlay.storePage"), systemImage: "bag",
                                        identifier: "game-overlay-store") { overlay.show(.store(appId: appId)) }
                        }
                        OverlayTile(title: L10n.string("ui.overlay.library"), systemImage: "square.grid.2x2",
                                    identifier: "game-overlay-library") { overlay.openLibrary() }
                    }
                }
                .padding(20)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .clipShape(RoundedRectangle(cornerRadius: 24))
        .modifier(StorefrontPanelGlass(cornerRadius: 24))
    }
}

private struct OverlayTile: View {
    let title: String
    let systemImage: String
    let identifier: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemImage)
                    .font(.system(size: 15, weight: .semibold))
                    .frame(width: 22)
                    .accessibilityHidden(true)
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 16)
            .frame(height: 48)
            .modifier(StorefrontPanelGlass(cornerRadius: 14))
            .contentShape(RoundedRectangle(cornerRadius: 14))
        }
        .obeliskButtonStyle(.plain)
        .accessibilityIdentifier(identifier)
    }
}

/// A page's back button, to the overlay's home.
private struct OverlayBackButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            StorefrontGlassIcon(systemName: "chevron.left", size: 14)
                .foregroundStyle(.primary)
        }
        .obeliskButtonStyle(.plain)
        .accessibilityLabel(L10n.string("ui.overlay.back"))
        .accessibilityIdentifier("game-overlay-back")
        .help(L10n.string("ui.overlay.back"))
    }
}

private struct OverlayAchievementsPage: View {
    let model: GameAchievementsModel
    let gameTitle: String
    let back: () -> Void
    let retry: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 14) {
                OverlayBackButton(action: back)
                VStack(alignment: .leading, spacing: 4) {
                    Label(L10n.string("ui.game.achievements.title"), systemImage: "trophy")
                        .font(.title2.weight(.semibold))
                    Text(gameTitle).font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer()
                if let fraction = model.fraction {
                    HStack(spacing: 12) {
                        Text(model.countText).monospacedDigit()
                        ProgressView(value: fraction).frame(width: 160)
                    }
                    .accessibilityElement(children: .combine)
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 20)
            Divider()
            GameAchievementsList(model: model, retry: retry)
        }
        .modifier(StorefrontPanelGlass(cornerRadius: 24))
        .accessibilityIdentifier("game-overlay-achievements-page")
    }
}

/// A web page in the overlay, such as a Steam store page. Nothing it stores
/// outlives the page.
private struct OverlayWebPage: View {
    let url: URL
    /// Sign the page in, such as Steam's store as the player's account.
    let cookies: () async -> [HTTPCookie]
    let close: () -> Void
    @State private var browser: OverlayBrowser?

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                OverlayBackButton {
                    if browser?.canGoBack == true { browser?.webView.goBack() } else { close() }
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(browser?.title ?? url.host() ?? url.absoluteString)
                        .font(.system(size: 13, weight: .semibold))
                    Text((browser?.url ?? url).host() ?? "")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                .lineLimit(1)
                Spacer(minLength: 12)
                if browser?.isLoading == true { ProgressView().controlSize(.small) }
                Button {
                    browser?.webView.reload()
                } label: {
                    StorefrontGlassIcon(systemName: "arrow.clockwise", size: 13).foregroundStyle(.primary)
                }
                .obeliskButtonStyle(.plain)
                .accessibilityLabel(L10n.string("ui.overlay.web.reload"))
                .help(L10n.string("ui.overlay.web.reload"))
                Button {
                    NSWorkspace.shared.open(browser?.url ?? url)
                } label: {
                    StorefrontGlassIcon(systemName: "safari", size: 14).foregroundStyle(.primary)
                }
                .obeliskButtonStyle(.plain)
                .accessibilityLabel(L10n.string("ui.overlay.web.openInBrowser"))
                .help(L10n.string("ui.overlay.web.openInBrowser"))
            }
            .padding(.horizontal, 14)
            .frame(height: 60)
            Group {
                if let browser { OverlayWebView(webView: browser.webView) } else { Color.clear }
            }
            .clipShape(UnevenRoundedRectangle(bottomLeadingRadius: 24, bottomTrailingRadius: 24))
        }
        .modifier(StorefrontPanelGlass(cornerRadius: 24))
        .task(id: url) { browser = OverlayBrowser(url: url, cookies: await cookies()) }
        .accessibilityIdentifier("game-overlay-web-page")
    }
}

/// The page's web view and what its toolbar shows of it.
@MainActor @Observable
final class OverlayBrowser: NSObject, WKNavigationDelegate, WKUIDelegate {
    @ObservationIgnored let webView: WKWebView
    private(set) var title: String?
    private(set) var url: URL?
    private(set) var canGoBack = false
    private(set) var isLoading = true

    /// Its cookies last as long as the page: its data store keeps nothing,
    /// and Obelisk signs a Steam page in again each time it opens.
    init(url: URL, cookies: [HTTPCookie] = []) {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        webView.navigationDelegate = self
        webView.uiDelegate = self
        let store = configuration.websiteDataStore.httpCookieStore
        Task { [webView] in
            for cookie in cookies { await store.setCookie(cookie) }
            webView.load(URLRequest(url: url))
        }
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        isLoading = true
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) { update() }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        update()
        isLoading = false
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        isLoading = false
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        isLoading = false
    }

    /// A link that opens a new window opens in the page instead.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if navigationAction.targetFrame == nil { webView.load(navigationAction.request) }
        return nil
    }

    private func update() {
        title = webView.title.flatMap { $0.isEmpty ? nil : $0 }
        url = webView.url
        canGoBack = webView.canGoBack
    }
}

private struct OverlayWebView: NSViewRepresentable {
    let webView: WKWebView

    func makeNSView(context: Context) -> WKWebView { webView }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
}

enum OverlayFormatting {
    /// How long the game has run: minutes, then hours and minutes.
    static func duration(from start: Date, to now: Date) -> String {
        let seconds = Int64(max(0, now.timeIntervalSince(start)))
        return Duration.seconds(seconds - seconds % 60)
            .formatted(.units(allowed: [.hours, .minutes], width: .abbreviated).locale(L10n.locale))
    }
}
