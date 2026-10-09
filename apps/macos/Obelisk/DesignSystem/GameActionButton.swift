import SwiftUI

struct GameActionButton: View {
    let title: String
    /// Whether the button launches the game rather than installing it.
    let launches: Bool
    let transfer: GameTransfer?
    let isTransferring: Bool
    let isAvailable: Bool
    let action: () -> Void
    let cancel: () -> Void
    var isCancellable = false
    /// A step outside any Transfer holds the button: a pre-launch Cloud sync,
    /// or Voile installing before a Windows launch.
    var isStep = false
    /// The step's progress, when it has one.
    var stepFraction: Double? = nil
    /// Set while the game runs: the button then ends it.
    var quitStage: GameQuitStage? = nil

    /// A step outranks a byte-counted transfer: it has no per-file title to show.
    /// A closing game shows as a step.
    private enum Mode { case idle, transferring, step }
    private var mode: Mode {
        if isStep || quitStage == .closing { .step } else if isTransferring { .transferring } else { .idle }
    }

    private var colors: [Color] {
        switch quitStage {
        case .quit, .closing: ObeliskStyle.quitGradient
        case .forceQuit: ObeliskStyle.forceQuitGradient
        case nil: launches ? ObeliskStyle.launchGradient : ObeliskStyle.installGradient
        }
    }

    private var symbol: String {
        if quitStage != nil { return "stop.fill" }
        return launches ? "play.fill" : "arrow.down.to.line"
    }

    private var showsProgress: Bool {
        switch mode {
        case .step: true
        case .transferring: transfer?.isInProgress ?? true
        case .idle: false
        }
    }
    private var helpText: String {
        switch quitStage {
        case .closing: L10n.string("ui.game.closing_help")
        case .forceQuit: L10n.string("ui.game.force_quit_help")
        case .quit, nil: mode == .step ? title : transfer?.progressSummary ?? title
        }
    }
    private var accessibilityText: String {
        switch mode {
        case .step: title
        case .transferring: transfer?.progressSummary ?? L10n.string("ui.transfer.preparing_accessibility")
        case .idle: ""
        }
    }

    var body: some View {
        HStack(spacing: 8) {
            Button(action: action) {
                Group {
                    switch mode {
                    case .step:
                        Text(title)
                    case .transferring:
                        Text(transfer?.buttonTitle ?? L10n.string("ui.transfer.preparing_short"))
                    case .idle:
                        Label(title, systemImage: symbol)
                    }
                }
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(width: 150)
            }
            .buttonStyle(GameActionButtonStyle(colors: colors, isBusy: mode != .idle))
            .overlay(alignment: .bottom) {
                if showsProgress {
                    GameButtonProgress(fraction: mode == .step ? stepFraction : transfer?.progressFraction)
                        .frame(height: 3)
                        .padding(.horizontal, 24)
                        .padding(.bottom, 6)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .disabled(!isAvailable || mode != .idle)
            .help(helpText)
            .accessibilityValue(accessibilityText)

            if isTransferring || isCancellable {
                Button(action: cancel) {
                    StorefrontGlassIcon(systemName: "xmark", size: 12)
                }
                .obeliskButtonStyle(.plain)
                .foregroundStyle(.secondary)
                .accessibilityLabel(L10n.string("ui.transfer.cancel"))
                .help(L10n.string("ui.transfer.cancel"))
                .transition(.scale(scale: 0.5).combined(with: .opacity))
            }
        }
        .fixedSize()
    }
}

/// The game's launch button: an uppercase title on a gradient capsule seen
/// through glass, green to launch, blue to install, grey to quit and red to
/// force quit, as the library's cards.
private struct GameActionButtonStyle: ButtonStyle {
    let colors: [Color]
    /// A button showing progress keeps its color while it cannot be pressed.
    let isBusy: Bool

    func makeBody(configuration: Configuration) -> some View {
        Appearance(configuration: configuration, colors: colors, isBusy: isBusy)
    }

    private struct Appearance: View {
        let configuration: Configuration
        let colors: [Color]
        let isBusy: Bool
        @Environment(\.isEnabled) private var isEnabled
        @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
        @State private var isHovered = false

        private var isDimmed: Bool { !isEnabled && !isBusy }

        var body: some View {
            let label = configuration.label
                .font(.system(size: 14, weight: .bold))
                .textCase(.uppercase)
                .tracking(0.5)
                .foregroundStyle(.white.opacity(isDimmed ? 0.6 : 1))
                .shadow(color: .black.opacity(0.2), radius: 1.5, y: 0.5)
                .padding(.horizontal, 18)
                .frame(height: 44)
                .background {
                    Capsule()
                        .fill(LinearGradient(colors: colors, startPoint: .leading, endPoint: .trailing))
                        .saturation(isDimmed ? 0.35 : 1)
                        .opacity(isDimmed ? 0.5 : reduceTransparency ? 1 : 0.85)
                        .brightness(configuration.isPressed ? -0.08 : isHovered && isEnabled ? 0.06 : 0)
                }
            Group {
                if reduceTransparency {
                    label
                } else {
                    label.glassEffect(.regular.interactive(isEnabled), in: .capsule)
                }
            }
            .contentShape(.capsule)
            .onHover { isHovered = $0 }
            .pointerCursor()
        }
    }
}

private struct GameButtonProgress: View {
    let fraction: Double?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var moving = false

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.25))
                if let fraction {
                    Capsule().fill(.white)
                        .frame(width: geometry.size.width * fraction)
                } else {
                    Capsule().fill(.white)
                        .frame(width: geometry.size.width * 0.3)
                        .offset(x: moving && !reduceMotion ? geometry.size.width * 0.7 : 0)
                        .animation(reduceMotion ? nil : .easeInOut(duration: 0.9).repeatForever(autoreverses: true), value: moving)
                        .onAppear { moving = true }
                        .onDisappear { moving = false }
                }
            }
        }
    }
}

extension View {
    /// Asks before forcing `gameName` to quit, as macOS's own Force Quit does:
    /// what the game did not save is lost.
    func forceQuitConfirmation(_ gameName: String, isPresented: Binding<Bool>,
                               confirm: @escaping () -> Void) -> some View {
        alert(L10n.string("ui.game.force_quit_title", gameName), isPresented: isPresented) {
            Button(L10n.string("ui.common.cancel"), role: .cancel) {}
            Button(L10n.string("model.game.forceQuit"), role: .destructive, action: confirm)
        } message: {
            Text(L10n.string("ui.game.force_quit_message"))
        }
    }
}
