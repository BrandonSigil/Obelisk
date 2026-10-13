import AppKit
import SwiftUI

/// Obelisk's settings: a hero in the violet of Obelisk's icon, its obelisk
/// glowing, reaching under the title bar band; then the appearance, the
/// language, the games, the storage and the storefronts, each a card of rows
/// as a game page's. The
/// bar's backdrop and the page's title show once the hero scrolled under it.
struct SettingsView: View {
    @Bindable var model: AppModel
    @State private var showsBackdrop = false
    @State private var showsTitle = false

    /// The hero's height below the bar it reaches under.
    private static var heroHeight: CGFloat { 210 }
    /// How wide the cards grow: rows any wider read poorly.
    private static var contentWidth: CGFloat { 900 }

    var body: some View {
        StorefrontScrollView {
            VStack(alignment: .leading, spacing: 0) {
                SettingsHero(height: Self.heroHeight)
                VStack(alignment: .leading, spacing: 34) {
                    GamePageSection(title: L10n.string("ui.settings.appearance")) {
                        SettingsCard {
                            AppearancePicker(selection: $model.appearance)
                                .padding(18)
                        }
                    }
                    GamePageSection(title: L10n.string("ui.settings.language")) {
                        SettingsCard {
                            LanguagePicker(selection: $model.language)
                                .padding(18)
                            if model.languageNeedsRestart {
                                SettingsDivider()
                                languageRestart
                                    .transition(.opacity)
                            }
                        }
                        .animation(.easeInOut(duration: 0.2), value: model.languageNeedsRestart)
                    }
                    GamePageSection(title: L10n.string("ui.settings.games")) {
                        SettingsCard {
                            automaticUpdates
                            SettingsDivider()
                            overlay
                        }
                    }
                    GamePageSection(title: L10n.string("ui.settings.storage")) {
                        SettingsCard {
                            libraryFolder
                        }
                    }
                    GamePageSection(title: L10n.string("ui.settings.stores")) {
                        SettingsCard {
                            ForEach(LibrarySection.storefronts) { section in
                                if section != LibrarySection.storefronts.first { SettingsDivider() }
                                SettingsStoreRow(section: section, status: status(of: section)) {
                                    model.selection = section
                                }
                            }
                        }
                    }
                }
                .frame(maxWidth: Self.contentWidth, alignment: .leading)
                .padding(.horizontal, ObeliskStyle.contentPadding)
                .padding(.top, 30)
                .padding(.bottom, 40)
            }
        }
        // The bar's backdrop shows once the hero scrolled under it, and the
        // page's title once the hero's own scrolled past the bar.
        .onScrollGeometryChange(for: Bool.self, of: { $0.contentOffset.y + $0.contentInsets.top > Self.heroHeight }) { _, under in
            showsBackdrop = under
        }
        .onScrollGeometryChange(for: Bool.self, of: { $0.contentOffset.y + $0.contentInsets.top > Self.heroHeight - 40 }) { _, under in
            showsTitle = under
        }
        .overlay(alignment: .top) {
            SettingsBar(showsBackdrop: showsBackdrop, showsTitle: showsTitle)
        }
        .ignoresSafeArea(.container, edges: .top)
    }

    /// Asks for the restart a newly chosen language waits for.
    private var languageRestart: some View {
        let language = model.language.resolved(system: AppLanguage.systemLanguages)
        return SettingsRow(title: L10n.string("ui.settings.language_restart_title")) {
            SettingsSymbol(name: "arrow.clockwise", colors: [ObeliskStyle.accent, Color(red: 0.45, green: 0.2, blue: 0.85)])
        } detail: {
            Text(L10n.string("ui.settings.language_restart", AppIdentity.name,
                             L10n.locale.localizedString(forLanguageCode: language) ?? AppLanguage.name(of: language)))
        } accessory: {
            SettingsAccentButton(title: L10n.string("ui.settings.restart"), identifier: "settings-restart",
                                 action: { model.restart() })
        }
    }

    private var automaticUpdates: some View {
        SettingsRow(title: L10n.string("ui.settings.update_automatically")) {
            SettingsSymbol(name: "arrow.down", colors: ObeliskStyle.installGradient)
        } detail: {
            Text(L10n.string("ui.settings.update_automatically_description"))
        } accessory: {
            Toggle(L10n.string("ui.settings.update_automatically"), isOn: $model.updatesGamesAutomatically)
                .toggleStyle(.switch)
                .labelsHidden()
                .pointerCursor()
                .accessibilityIdentifier("settings-update-automatically")
        }
    }

    private var overlay: some View {
        SettingsRow(title: L10n.string("ui.settings.overlay")) {
            SettingsSymbol(name: "rectangle.on.rectangle", colors: [ObeliskStyle.accent, Color(red: 0.45, green: 0.2, blue: 0.85)])
        } detail: {
            Text(L10n.string("ui.settings.overlay_description", AppIdentity.name))
        } accessory: {
            HStack(spacing: 5) {
                KeyCap(text: L10n.string("ui.settings.key_shift"))
                KeyCap(text: L10n.string("ui.settings.key_tab"))
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(L10n.string("ui.settings.overlay_shortcut"))
        }
    }

    private var libraryFolder: some View {
        SettingsRow(title: L10n.string("ui.settings.library_folder")) {
            SettingsSymbol(name: "folder.fill", colors: [Color(red: 0.38, green: 0.68, blue: 0.98), Color(red: 0.2, green: 0.5, blue: 0.93)])
        } detail: {
            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.string("ui.settings.library_folder_description", AppIdentity.name))
                Text(model.libraryDirectory)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .help(model.libraryDirectory)
            }
        } accessory: {
            SettingsGlassButton(title: L10n.string("ui.settings.show_in_finder"), systemImage: "folder",
                                identifier: "settings-show-library-folder") {
                NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: model.libraryDirectory)
            }
        }
    }

    /// What a storefront's row tells of its account, as Home's tile does.
    private func status(of section: LibrarySection) -> LibraryTile.Status {
        switch section {
        case .steam:
            status(of: model.steam, in: section, showsLibrary: model.steam.showsLibrary,
                   isRestoring: model.steam.isRestoring, account: model.steam.displayName,
                   avatarURL: model.steam.avatarURL)
        case .epic:
            status(of: model.epic, in: section, showsLibrary: model.epic.showsLibrary,
                   isRestoring: model.epic.isRestoring, account: model.epic.accountName,
                   avatarURL: model.epic.avatarURL)
        default:
            status(of: model.gog, in: section, showsLibrary: model.gog.showsLibrary,
                   isRestoring: model.gog.isRestoring, account: model.gog.accountName,
                   avatarURL: model.gog.avatarURL)
        }
    }

    private func status<Store: LibraryStorefront>(
        of store: Store, in section: LibrarySection, showsLibrary: Bool, isRestoring: Bool, account: String?,
        avatarURL: URL?
    ) -> LibraryTile.Status {
        if showsLibrary {
            .signedIn(account: account, avatarURL: avatarURL, games: store.games.count,
                      installed: Set(store.installations.receipts.map(\.gameID)).count)
        } else if isRestoring || model.restoringStorefronts.contains(section) {
            .restoring
        } else {
            .signedOut
        }
    }
}

/// The settings' bar: the window's drag region, whose backdrop and title show
/// once the hero scrolled under it, as a game page's.
private struct SettingsBar: View {
    let showsBackdrop: Bool
    let showsTitle: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        StorefrontWindowDragRegion()
            .frame(maxWidth: .infinity)
            .frame(height: ObeliskStyle.titleBarHeight)
            .overlay {
                ZStack {
                    if showsTitle {
                        Text(L10n.string("ui.settings.title"))
                            .font(.system(size: 13, weight: .semibold))
                            .transition(.opacity.combined(with: .offset(y: 6)))
                    }
                }
                .animation(reduceMotion ? nil : .easeOut(duration: 0.2), value: showsTitle)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
            .background {
                Rectangle()
                    .fill(.bar)
                    .opacity(showsBackdrop ? 1 : 0)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: showsBackdrop)
                    .allowsHitTesting(false)
            }
    }
}

/// The settings' hero: Obelisk's icon as a wide scene, then the page's title
/// and Obelisk's version, light in both appearances as a library hero's.
private struct SettingsHero: View {
    /// The hero's height below the bar it reaches under.
    let height: CGFloat

    var body: some View {
        let reach = ObeliskStyle.titleBarHeight
        return HeroBackdrop { SettingsHeroArtwork() }
            .overlay(alignment: .bottomLeading) {
                VStack(alignment: .leading, spacing: 10) {
                    Text(L10n.string("ui.settings.title"))
                        .font(.system(size: 40, weight: .bold))
                        .tracking(-1)
                    HStack(spacing: 16) {
                        Label {
                            Text(AppIdentity.name)
                        } icon: {
                            Image("MenuBarIcon")
                                .renderingMode(.template)
                                .resizable()
                                .scaledToFit()
                                .frame(width: 13, height: 13)
                        }
                        Label(L10n.string("ui.settings.version", AppIdentity.version), systemImage: "tag")
                            .textSelection(.enabled)
                    }
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.85))
                }
                .foregroundStyle(.white)
                .shadow(color: .black.opacity(0.35), radius: 10, y: 2)
                .padding(.horizontal, ObeliskStyle.contentPadding)
                .padding(.bottom, 28)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isHeader)
                // The text fades out as it slides under the bar, as a library
                // hero's.
                .visualEffect { [reach] content, geometry in
                    let top = geometry.frame(in: .scrollView).minY
                    return content.opacity(min(max((top - reach + 40) / 80, 0), 1))
                }
            }
            .frame(height: height + reach)
            .padding(.top, -reach)
            .environment(\.colorScheme, .dark)
    }
}

/// Obelisk's icon, wide: its obelisk glowing toward the trailing edge, on a
/// violet that darkens toward the leading edge and the title.
private struct SettingsHeroArtwork: View {
    var body: some View {
        MeshGradient(
            width: 3,
            height: 3,
            points: [
                [0, 0], [0.55, 0], [1, 0],
                [0, 0.5], [0.62, 0.42], [1, 0.5],
                [0, 1], [0.5, 1], [1, 1],
            ],
            colors: [
                Color(red: 0.22, green: 0.06, blue: 0.38), Color(red: 0.52, green: 0.16, blue: 0.84),
                Color(red: 0.76, green: 0.25, blue: 0.98),
                Color(red: 0.12, green: 0.05, blue: 0.26), Color(red: 0.34, green: 0.13, blue: 0.68),
                Color(red: 0.5, green: 0.22, blue: 0.9),
                Color(red: 0.07, green: 0.03, blue: 0.16), Color(red: 0.16, green: 0.08, blue: 0.4),
                Color(red: 0.29, green: 0.16, blue: 0.72),
            ]
        )
        .overlay(alignment: .bottomTrailing) {
            GeometryReader { geometry in
                let height = geometry.size.height * 0.84
                let center = CGPoint(x: geometry.size.width - 190, y: geometry.size.height - height / 2)
                ZStack {
                    // The light the obelisk sheds on the violet around it.
                    RadialGradient(colors: [.white.opacity(0.32), ObeliskStyle.accent.opacity(0.28), .clear],
                                   center: .center, startRadius: 0, endRadius: height * 1.1)
                        .frame(width: height * 2.4, height: height * 2.4)
                    Image("MenuBarIcon")
                        .renderingMode(.template)
                        .resizable()
                        .scaledToFit()
                        .foregroundStyle(.white)
                        .frame(height: height)
                        .shadow(color: Color(red: 0.95, green: 0.82, blue: 1).opacity(0.9), radius: 14)
                        .shadow(color: ObeliskStyle.accent, radius: 46)
                }
                .position(center)
            }
            .accessibilityHidden(true)
        }
    }
}

/// A card of settings rows, as a game page's card, its rows divided.
private struct SettingsCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .modifier(GamePageCard(padding: 0))
    }
}

/// The line between two rows, from their text's leading edge.
private struct SettingsDivider: View {
    var body: some View {
        Divider().padding(.leading, 64)
    }
}

/// A setting: its symbol, its title over what it does, then its control.
private struct SettingsRow<Icon: View, Detail: View, Accessory: View>: View {
    let title: String
    @ViewBuilder let icon: Icon
    @ViewBuilder let detail: Detail
    @ViewBuilder let accessory: Accessory

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            icon
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                detail
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            accessory
                .padding(.leading, 10)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
    }
}

/// A setting's symbol, white on a tile of its colors, as System Settings
/// shows its panes.
private struct SettingsSymbol: View {
    let name: String
    let colors: [Color]

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 9, style: .continuous)
        Image(systemName: name)
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 32, height: 32)
            .background(LinearGradient(colors: colors, startPoint: .top, endPoint: .bottom), in: shape)
            .overlay { shape.strokeBorder(.white.opacity(0.16), lineWidth: 0.5) }
            .accessibilityHidden(true)
    }
}

/// A row's action, on a glass capsule as the libraries' controls.
private struct SettingsGlassButton: View {
    let title: String
    var systemImage: String?
    var accessibilityLabel: String?
    let identifier: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let systemImage {
                    Image(systemName: systemImage)
                        .font(.system(size: 11, weight: .semibold))
                }
                Text(title)
            }
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(.primary)
            .lineLimit(1)
            .padding(.horizontal, 14)
            .frame(height: 32)
            .modifier(StorefrontControlGlass())
            .contentShape(.capsule)
        }
        .obeliskButtonStyle(.plain)
        .fixedSize()
        .accessibilityLabel(accessibilityLabel ?? title)
        .accessibilityIdentifier(identifier)
    }
}

/// A row's main action, on a capsule of the accent color.
private struct SettingsAccentButton: View {
    let title: String
    var accessibilityLabel: String?
    let identifier: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white)
                .lineLimit(1)
                .padding(.horizontal, 14)
                .frame(height: 32)
                .background(ObeliskStyle.accent, in: .capsule)
                .contentShape(.capsule)
        }
        .obeliskButtonStyle(.plain)
        .fixedSize()
        .accessibilityLabel(accessibilityLabel ?? title)
        .accessibilityIdentifier(identifier)
    }
}

/// A key of a keyboard shortcut.
private struct KeyCap: View {
    let text: String

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 7, style: .continuous)
        Text(text)
            .font(.system(size: 12, weight: .semibold))
            .padding(.horizontal, 9)
            .frame(minWidth: 30, minHeight: 26)
            .background(.primary.opacity(0.06), in: shape)
            .overlay { shape.strokeBorder(.primary.opacity(0.14), lineWidth: 0.75) }
            // The key's lower edge, as on a keyboard.
            .background(.primary.opacity(0.12), in: shape.offset(y: 1.5))
            .fixedSize()
    }
}

/// The three appearances as small windows of Obelisk, the chosen one ringed in
/// the accent color; the ring slides to a new choice.
private struct AppearancePicker: View {
    @Binding var selection: AppAppearance
    @State private var hovered: AppAppearance?
    @Namespace private var namespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 20) {
            ForEach(AppAppearance.allCases) { option($0) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L10n.string("ui.settings.theme"))
    }

    private func option(_ appearance: AppAppearance) -> some View {
        let isSelected = appearance == selection
        let window = RoundedRectangle(cornerRadius: 10, style: .continuous)
        return Button {
            withAnimation(reduceMotion ? nil : ObeliskStyle.selectionAnimation) { selection = appearance }
        } label: {
            VStack(spacing: 9) {
                AppearancePreview(appearance: appearance)
                    .frame(width: 150, height: 96)
                    .clipShape(window)
                    .overlay { window.strokeBorder(.primary.opacity(0.14), lineWidth: 1) }
                    .shadow(color: .black.opacity(0.18), radius: 6, y: 3)
                    .padding(4)
                    .overlay {
                        if isSelected {
                            RoundedRectangle(cornerRadius: 14, style: .continuous)
                                .strokeBorder(ObeliskStyle.accent, lineWidth: 2.5)
                                .matchedGeometryEffect(id: "selection", in: namespace)
                        }
                    }
                    .scaleEffect(hovered == appearance && !isSelected && !reduceMotion ? 1.03 : 1)
                Text(appearance.title)
                    .font(.system(size: 12, weight: isSelected ? .semibold : .medium))
                    .foregroundStyle(isSelected ? .primary : .secondary)
            }
            // The choices share the card's width.
            .frame(maxWidth: .infinity)
            .contentShape(.rect)
        }
        .obeliskButtonStyle(.plain)
        .onHover { hovered = $0 ? appearance : (hovered == appearance ? nil : hovered) }
        .animation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.75), value: hovered)
        .accessibilityLabel(appearance.title)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier("settings-appearance-\(appearance.id)")
    }
}

/// The Mac's language, then every language Obelisk is translated into, by its
/// flag and its own name, the chosen one ringed in the accent color as an
/// appearance is; the ring slides to a new choice.
private struct LanguagePicker: View {
    @Binding var selection: AppLanguage
    @State private var hovered: AppLanguage?
    @Namespace private var namespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 20) {
            ForEach(AppLanguage.choices) { option($0) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(L10n.string("ui.settings.language"))
    }

    private func option(_ language: AppLanguage) -> some View {
        let isSelected = language == selection
        let title = language.code.map(AppLanguage.name(of:)) ?? L10n.string("ui.settings.language_system")
        return Button {
            withAnimation(reduceMotion ? nil : ObeliskStyle.selectionAnimation) { selection = language }
        } label: {
            VStack(spacing: 9) {
                LanguageFlag(language: language)
                    .frame(width: 81, height: 54)
                    .shadow(color: .black.opacity(0.18), radius: 6, y: 3)
                    .padding(5)
                    .overlay {
                        if isSelected {
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .strokeBorder(ObeliskStyle.accent, lineWidth: 2.5)
                                .matchedGeometryEffect(id: "selection", in: namespace)
                        }
                    }
                    .scaleEffect(hovered == language && !isSelected && !reduceMotion ? 1.04 : 1)
                Text(title)
                    .font(.system(size: 12, weight: isSelected ? .semibold : .medium))
                    .foregroundStyle(isSelected ? .primary : .secondary)
            }
            // The choices share the card's width, as the appearances do.
            .frame(maxWidth: .infinity)
            .contentShape(.rect)
        }
        .obeliskButtonStyle(.plain)
        .onHover { hovered = $0 ? language : (hovered == language ? nil : hovered) }
        .animation(reduceMotion ? nil : .spring(response: 0.3, dampingFraction: 0.75), value: hovered)
        .accessibilityLabel(title)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier("settings-language-\(language.id)")
    }
}

/// A language's flag, two split across the diagonal as the system's
/// appearance is for a language several countries speak, and for the Mac's
/// language a globe; on a flag's rounded shape, edged so a white stripe stays
/// visible.
private struct LanguageFlag: View {
    let language: AppLanguage

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 7, style: .continuous)
        let flags = language.flags.filter { NSImage(named: $0) != nil }
        ZStack {
            if let first = flags.first {
                Image(first)
                    .resizable()
                if let second = flags.dropFirst().first {
                    // The second flag's middle, where its emblem sits, moves
                    // to the middle of its half.
                    GeometryReader { geometry in
                        Image(second)
                            .resizable()
                            .offset(x: geometry.size.width * (1 - (TrailingDiagonalHalf.top + TrailingDiagonalHalf.bottom) / 2) / 2)
                    }
                    .mask { TrailingDiagonalHalf() }
                    // A white edge parts two flags of the same colors.
                    DiagonalEdge()
                        .stroke(.white, lineWidth: 2.5)
                        .shadow(color: .black.opacity(0.35), radius: 1.5)
                }
            } else {
                LinearGradient(colors: [Color(red: 0.38, green: 0.68, blue: 0.98), Color(red: 0.2, green: 0.45, blue: 0.9)],
                               startPoint: .top, endPoint: .bottom)
                Image(systemName: language.code == nil ? "globe" : "character.bubble")
                    .font(.system(size: 22, weight: .medium))
                    .foregroundStyle(.white)
            }
        }
        .clipShape(shape)
        .overlay { shape.strokeBorder(.primary.opacity(0.14), lineWidth: 1) }
        .accessibilityHidden(true)
    }
}

/// A small window of Obelisk in one appearance; the system's is light, then
/// dark past a diagonal.
private struct AppearancePreview: View {
    let appearance: AppAppearance

    var body: some View {
        switch appearance {
        case .light: MiniatureWindow(colorScheme: .light)
        case .dark: MiniatureWindow(colorScheme: .dark)
        case .system:
            MiniatureWindow(colorScheme: .light)
                .overlay { MiniatureWindow(colorScheme: .dark).mask { TrailingDiagonalHalf() } }
        }
    }
}

/// Obelisk's window in miniature: its sidebar, a hero and a shelf of covers.
private struct MiniatureWindow: View {
    let colorScheme: ColorScheme

    private static let covers = [
        Color(red: 0.95, green: 0.58, blue: 0.25), Color(red: 0.3, green: 0.56, blue: 0.95),
        Color(red: 0.4, green: 0.76, blue: 0.45), Color(red: 0.92, green: 0.4, blue: 0.6),
    ]

    var body: some View {
        let isDark = colorScheme == .dark
        let line = isDark ? Color.white.opacity(0.22) : Color.black.opacity(0.13)
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 2.5) {
                    ForEach([Color.red, .yellow, .green], id: \.self) { color in
                        Circle().fill(color.opacity(0.85)).frame(width: 4.5, height: 4.5)
                    }
                }
                .padding(.bottom, 3)
                Capsule().fill(ObeliskStyle.accent.opacity(0.85)).frame(width: 24, height: 5)
                ForEach([18, 21, 15] as [CGFloat], id: \.self) { width in
                    Capsule().fill(line).frame(width: width, height: 4)
                }
            }
            .padding(6)
            .frame(width: 42, alignment: .topLeading)
            .frame(maxHeight: .infinity, alignment: .top)
            .background(isDark ? Color(white: 0.19) : Color(white: 0.91))
            VStack(alignment: .leading, spacing: 6) {
                LinearGradient(colors: [Color(red: 0.62, green: 0.2, blue: 0.92), Color(red: 0.25, green: 0.12, blue: 0.6)],
                               startPoint: .topTrailing, endPoint: .bottomLeading)
                    .frame(height: 38)
                    .overlay(alignment: .bottomLeading) {
                        Capsule().fill(.white.opacity(0.8)).frame(width: 30, height: 4).padding(6)
                    }
                HStack(spacing: 4) {
                    ForEach(Self.covers, id: \.self) { color in
                        RoundedRectangle(cornerRadius: 2.5, style: .continuous)
                            .fill(color.opacity(isDark ? 0.8 : 0.9))
                            .frame(width: 22, height: 33)
                    }
                }
                .padding(.leading, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .clipped()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .background(ObeliskPalette(colorScheme: colorScheme).background)
        }
    }
}

/// The part of a rectangle past a diagonal from its top to its bottom edge.
private struct TrailingDiagonalHalf: Shape {
    /// Where the diagonal meets the top and the bottom edge, from the leading
    /// one.
    static let top = 0.62
    static let bottom = 0.38

    func path(in rect: CGRect) -> Path {
        Path { path in
            path.move(to: CGPoint(x: rect.minX + rect.width * Self.top, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
            path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
            path.addLine(to: CGPoint(x: rect.minX + rect.width * Self.bottom, y: rect.maxY))
            path.closeSubpath()
        }
    }
}

/// The diagonal that bounds `TrailingDiagonalHalf`, past both edges so a
/// stroke reaches the corners.
private struct DiagonalEdge: Shape {
    func path(in rect: CGRect) -> Path {
        let slope = rect.width * (TrailingDiagonalHalf.bottom - TrailingDiagonalHalf.top) / rect.height
        return Path { path in
            path.move(to: CGPoint(x: rect.minX + rect.width * TrailingDiagonalHalf.top - slope * 4, y: rect.minY - 4))
            path.addLine(to: CGPoint(x: rect.minX + rect.width * TrailingDiagonalHalf.bottom + slope * 4,
                                     y: rect.maxY + 4))
        }
    }
}

/// A storefront and its account, as Home's tile tells it, opening its library
/// or its sign-in.
private struct SettingsStoreRow: View {
    let section: LibrarySection
    let status: LibraryTile.Status
    let open: () -> Void

    private var name: String { section.storeName ?? section.title }

    var body: some View {
        SettingsRow(title: name) {
            StoreTile(section: section)
        } detail: {
            detail
        } accessory: {
            accessory
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("settings-store-\(section.rawValue)")
    }

    @ViewBuilder private var detail: some View {
        switch status {
        case let .signedIn(account, avatarURL, games, installed):
            HStack(spacing: 6) {
                if let account, !account.isEmpty {
                    AccountAvatar(name: account, url: avatarURL)
                        .frame(width: 16, height: 16)
                    Text(account)
                        .foregroundStyle(.primary)
                    Text(verbatim: "·")
                }
                Text([L10n.string("ui.library.game_count", Int64(games)),
                      installed > 0 ? L10n.string("ui.home.installed_count", Int64(installed)) : nil]
                    .compactMap { $0 }
                    .joined(separator: " · "))
                    .monospacedDigit()
            }
            .lineLimit(1)
        case .restoring:
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text(L10n.string("ui.account.signing_in"))
            }
        case .signedOut:
            Text(L10n.string("ui.home.signed_out"))
        }
    }

    @ViewBuilder private var accessory: some View {
        switch status {
        case .signedIn:
            SettingsGlassButton(title: L10n.string("ui.settings.open"),
                                accessibilityLabel: L10n.string("ui.settings.open_store", name),
                                identifier: "settings-open-\(section.rawValue)", action: open)
        case .restoring:
            EmptyView()
        case .signedOut:
            SettingsAccentButton(title: L10n.string("ui.account.sign_in"),
                                 accessibilityLabel: L10n.string("ui.home.sign_in_store", name),
                                 identifier: "settings-sign-in-\(section.rawValue)", action: open)
        }
    }
}

/// A storefront's logo, white on a dark tile in its colors.
private struct StoreTile: View {
    let section: LibrarySection

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 9, style: .continuous)
        ZStack {
            if let logo = section.logoAssetName {
                Image(logo)
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 18, height: 18)
            }
        }
        .foregroundStyle(.white)
        .frame(width: 32, height: 32)
        .background {
            shape.fill(Color(white: 0.14))
                .overlay {
                    LinearGradient(colors: [section.tint.opacity(0.5), .clear], startPoint: .topLeading,
                                   endPoint: .bottomTrailing)
                        .clipShape(shape)
                }
        }
        .overlay { shape.strokeBorder(.white.opacity(0.14), lineWidth: 0.5) }
        .accessibilityHidden(true)
    }
}
