import AppKit
import SwiftUI

struct StorefrontSortOption: Identifiable {
    let id: String
    let title: String
    let isSelected: Bool
    let action: @MainActor () -> Void
}

/// The title bar of a storefront's game pages and of a store still to come:
/// the account and the storefront's name.
struct StorefrontTitleBar<AccountMenu: View>: View {
    let title: String

    let accountName: String?
    let avatarURL: URL?
    let isConnected: Bool
    private let accountMenu: AccountMenu

    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    init(
        title: String,
        accountName: String?,
        avatarURL: URL? = nil,
        isConnected: Bool,
        @ViewBuilder accountMenu: () -> AccountMenu
    ) {
        self.title = title
        self.accountName = accountName
        self.avatarURL = avatarURL
        self.isConnected = isConnected
        self.accountMenu = accountMenu()
    }

    var body: some View {
        HStack(spacing: 12) {
            if isConnected {
                Menu { accountMenu } label: { avatar }
                    .menuStyle(.button)
                    .obeliskButtonStyle(.plain)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .accessibilityLabel(accountLabel)
                    .accessibilityIdentifier("storefront-account-menu")
                    .help(accountLabel)
            } else {
                avatar.accessibilityLabel(accountLabel)
            }

            Text(title)
                .font(.system(size: 13, weight: .semibold))
                .lineLimit(1)

            Spacer(minLength: 12)
                .overlay { StorefrontWindowDragRegion() }
        }
        .padding(.horizontal, 20)
        .frame(height: ObeliskStyle.titleBarHeight)
        .background {
            if reduceTransparency {
                ObeliskPalette(colorScheme: colorScheme).background
            } else {
                StorefrontTitleBarMaterial()
            }
        }
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(ObeliskPalette(colorScheme: colorScheme).border)
                .frame(height: 1)
                .allowsHitTesting(false)
        }
    }

    private var accountLabel: String {
        let name = accountName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return name.isEmpty ? L10n.string("ui.account.store", title) : name
    }

    /// The library's account picture, glass as the title bar's control.
    private var avatar: some View {
        AccountAvatar(name: accountName, url: avatarURL)
            .frame(width: 28, height: 28)
            .modifier(StorefrontControlGlass())
            .contentShape(Circle())
            // The picture hides itself from accessibility; the bar names the account.
            .accessibilityElement(children: .ignore)
    }
}

struct StorefrontSortPopover: View {
    let options: [StorefrontSortOption]
    let dismiss: @MainActor () -> Void
    @FocusState private var focusedOption: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(options) { option in
                Button { select(option) } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark")
                            .font(.system(size: 11, weight: .semibold))
                            .opacity(option.isSelected ? 1 : 0)
                            .frame(width: 14)
                            .accessibilityHidden(true)
                        Text(option.title).lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .font(.system(size: 13))
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(focusedOption == option.id ? Color.primary.opacity(0.09) : .clear, in: RoundedRectangle(cornerRadius: 6))
                    .contentShape(Rectangle())
                }
                .obeliskButtonStyle(.plain)
                .focusable()
                .focused($focusedOption, equals: option.id)
                .focusEffectDisabled()
                .onHover { if $0 { focusedOption = option.id } }
                .accessibilityLabel(option.title)
                .accessibilityAddTraits(option.isSelected ? .isSelected : [])
                .accessibilityIdentifier("storefront-sort-option-\(option.id)")
            }
        }
        .padding(6)
        .frame(minWidth: 230)
        .fixedSize(horizontal: true, vertical: true)
        .onAppear { focusedOption = options.first(where: \.isSelected)?.id ?? options.first?.id }
        .onMoveCommand { direction in
            if direction == .up { moveFocus(by: -1) }
            else if direction == .down { moveFocus(by: 1) }
        }
        .onKeyPress(.return) {
            guard let option = options.first(where: { $0.id == focusedOption }) else { return .ignored }
            select(option)
            return .handled
        }
        .onExitCommand(perform: dismiss)
    }

    private func select(_ option: StorefrontSortOption) {
        option.action()
        dismiss()
    }

    private func moveFocus(by offset: Int) {
        guard !options.isEmpty else { return }
        let current = options.firstIndex(where: { $0.id == focusedOption }) ?? 0
        focusedOption = options[(current + offset + options.count) % options.count].id
    }
}

/// A storefront control's glass capsule, solid while transparency is reduced.
struct StorefrontControlGlass: ViewModifier {
    /// Whether the glass reacts to the pointer, as a button's does.
    var interactive = true
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    @ViewBuilder
    func body(content: Content) -> some View {
        if reduceTransparency {
            content.background {
                Capsule()
                    .fill(Color(nsColor: .controlBackgroundColor))
                    .overlay { outline }
            }
        } else {
            content.glassEffect(interactive ? .regular.interactive() : .regular, in: Capsule())
        }
    }

    private var outline: some View {
        Capsule().strokeBorder(
            LinearGradient(
                colors: [.white.opacity(colorScheme == .dark ? 0.24 : 0.65), .primary.opacity(0.1)],
                startPoint: .top,
                endPoint: .bottom
            ),
            lineWidth: 0.75
        )
    }
}

private struct StorefrontTitleBarMaterial: NSViewRepresentable {
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .headerView
        view.blendingMode = .withinWindow
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {}
}

struct StorefrontWindowDragRegion: NSViewRepresentable {
    func makeNSView(context: Context) -> DragRegion {
        let view = DragRegion()
        view.identifier = NSUserInterfaceItemIdentifier("storefront-titlebar-drag-region")
        return view
    }
    func updateNSView(_ nsView: DragRegion, context: Context) {}

    final class DragRegion: NSView {
        override func mouseDown(with event: NSEvent) {
            window?.performDrag(with: event)
        }
    }
}
