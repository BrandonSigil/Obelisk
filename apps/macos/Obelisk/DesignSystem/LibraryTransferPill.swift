import SwiftUI

/// The amount and state of a storefront's running Transfer, floating over the
/// bottom of its library.
struct LibraryTransferPill: View {
    let gameName: String?
    let transfer: GameTransfer
    let cancel: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: symbol)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(ObeliskStyle.accent)
                .frame(width: 22)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Text(gameName ?? transfer.activityTitle)
                        .font(.system(size: 12, weight: .semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if let fraction = transfer.progressFraction {
                        Text(fraction, format: .percent.precision(.fractionLength(0)).locale(L10n.locale))
                            .font(.system(size: 11, weight: .semibold))
                            .monospacedDigit()
                    }
                }

                if let fraction = transfer.progressFraction {
                    ProgressView(value: fraction)
                } else {
                    ProgressView().progressViewStyle(.linear)
                }

                if let caption {
                    Text(caption)
                        .font(.system(size: 10))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            .frame(width: 340)

            Button(action: cancel) {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .bold))
                    .frame(width: 28, height: 28)
                    .glassEffect(.regular.interactive(), in: .circle)
                    .contentShape(.circle)
            }
            .obeliskButtonStyle(.plain)
            .accessibilityLabel(L10n.string("ui.common.cancel"))
            .help(L10n.string("ui.common.cancel"))
        }
        .padding(.leading, 18)
        .padding(.trailing, 12)
        .padding(.vertical, 12)
        .glassEffect(.regular, in: .rect(cornerRadius: 22))
        .help(transfer.progressSummary)
        .accessibilityElement(children: .contain)
    }

    /// The activity, when the game's name took the first line, then the amount.
    private var caption: String? {
        let parts = [gameName == nil ? nil : transfer.activityTitle, detail].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private var detail: String? {
        if transfer.totalBytes > 0 { return transfer.verifiedProgressText }
        if let data = transfer.downloadedGameDataText { return data }
        if !transfer.currentFile.isEmpty { return L10n.message(transfer.currentFile) }
        return transfer.message.isEmpty ? nil : L10n.message(transfer.message)
    }

    private var symbol: String {
        switch transfer.state {
        case .preparing: "arrow.clockwise"
        case .verifying: "checkmark.shield"
        case .downloading: "arrow.down.circle"
        case .complete: "checkmark.circle"
        case .cancelled: "pause.circle"
        case .failed: "exclamationmark.circle"
        }
    }
}
