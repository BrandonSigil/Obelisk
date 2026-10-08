import AppKit
import SwiftUI

struct GameManagementButton: View {
    let gameName: String
    let isAvailable: Bool
    let uninstall: () -> Void
    /// The installed game's directory, revealed in Finder from the menu.
    let directory: String
    @State private var confirmsUninstall = false

    var body: some View {
        Menu {
            Button(L10n.string("ui.game.show_files"), systemImage: "folder") {
                NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: directory)
            }
            Divider()
            Button(L10n.string("ui.game.uninstall"), systemImage: "trash", role: .destructive) {
                confirmsUninstall = true
            }
            .disabled(!isAvailable)
        } label: {
            Image(systemName: "gearshape.fill")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(.primary)
                .frame(width: 36, height: 36)
                .modifier(StorefrontControlGlass())
                .contentShape(.circle)
        }
        .menuStyle(.button)
        .obeliskButtonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(!isAvailable)
        .accessibilityLabel(L10n.string("ui.game.manage_named", gameName))
        .accessibilityIdentifier("game-management-menu")
        .help(L10n.string("ui.game.manage"))
        .alert(L10n.string("ui.game.uninstall_title", gameName), isPresented: $confirmsUninstall) {
            Button(L10n.string("ui.common.cancel"), role: .cancel) {}
            Button(L10n.string("ui.game.uninstall"), role: .destructive, action: uninstall)
                .disabled(!isAvailable)
        } message: {
            Text(L10n.string("ui.game.uninstall_message"))
        }
    }
}

/// What a game page's metrics report. The row orders them as Steam does and,
/// when it runs out of width, drops the least useful ones first.
struct GameMetrics {
    var isInstalled: Bool
    var size: String?
    var lastPlayed: String?
    var playtime: String?
    var cloud: CloudIndicatorButton?
    var achievements: GameAchievementsModel
}

/// A game page's metrics, under its launch button.
struct GameMetricsRow: View {
    let metrics: GameMetrics

    var body: some View {
        // The last level keeps only the controls. When even they do not fit,
        // the Cloud value truncates.
        ViewThatFits(in: .horizontal) {
            ForEach(0...4, id: \.self) { metricRow(dropping: $0) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Each level drops one more metric: an installed game's size, then the
    /// last played date, the playtime and the space a game needs, which matters
    /// most before it is installed. The Cloud and achievement metrics are
    /// controls and never leave.
    private func metricRow(dropping level: Int) -> some View {
        HStack(spacing: 2) {
            if let cloud = metrics.cloud { cloud }
            if !metrics.isInstalled, level < 4, let size = metrics.size {
                GameMetricLabel(caption: L10n.string("ui.game.metric.space_required")) {
                    GameMetricSymbol(name: "internaldrive")
                } value: {
                    Text(size)
                }
            }
            if level < 2, let lastPlayed = metrics.lastPlayed {
                GameMetricLabel(caption: L10n.string("ui.game.metric.last_played")) {
                    GameMetricSymbol(name: "calendar")
                } value: {
                    Text(lastPlayed)
                }
            }
            if level < 3, let playtime = metrics.playtime {
                GameMetricLabel(caption: L10n.string("ui.game.metric.playtime")) {
                    GameMetricSymbol(name: "clock")
                } value: {
                    Text(playtime)
                }
            }
            GameAchievementsButton(model: metrics.achievements)
            // An installed game's size moves last, where it leaves first.
            if metrics.isInstalled, level < 1, let size = metrics.size {
                GameMetricLabel(caption: L10n.string("ui.game.metric.size")) {
                    GameMetricSymbol(name: "internaldrive")
                } value: {
                    Text(size)
                }
            }
        }
        // Aligns the first metric's symbol with the launch button's edge.
        .padding(.leading, -10)
    }
}

/// A game page metric: an uppercase caption over its value, after an optional
/// large symbol. A metric that is a control highlights under the pointer.
struct GameMetricLabel<Icon: View, Value: View>: View {
    let caption: String
    private let isHighlighted: Bool
    private let icon: Icon
    private let value: Value

    init(caption: String, isHighlighted: Bool = false, @ViewBuilder icon: () -> Icon,
         @ViewBuilder value: () -> Value) {
        self.caption = caption
        self.isHighlighted = isHighlighted
        self.icon = icon()
        self.value = value()
    }

    var body: some View {
        HStack(spacing: 9) {
            icon
            VStack(alignment: .leading, spacing: 4) {
                Text(caption)
                    .font(.system(size: 10, weight: .semibold))
                    .tracking(0.4)
                    .textCase(.uppercase)
                    .foregroundStyle(.secondary)
                value
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.primary)
            }
            .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.primary.opacity(isHighlighted ? 0.08 : 0), in: .capsule)
        .contentShape(.capsule)
    }
}

extension GameMetricLabel where Icon == EmptyView {
    init(caption: String, @ViewBuilder value: () -> Value) {
        self.init(caption: caption, icon: { EmptyView() }, value: value)
    }
}

/// The symbol before a game page metric.
struct GameMetricSymbol: View {
    let name: String
    var isWarning = false

    var body: some View {
        Image(systemName: name)
            .font(.system(size: 18, weight: .regular))
            .foregroundStyle(isWarning ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
            .frame(width: 26, height: 26)
    }
}
