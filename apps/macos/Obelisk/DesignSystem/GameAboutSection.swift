import SwiftUI

/// What a storefront tells of a game on its page: its summary, then facts
/// such as its developer or release date. Shows nothing when the storefront
/// tells neither.
struct GameAboutSection: View {
    struct Fact: Identifiable {
        let label: String
        let value: String
        var id: String { label }
    }

    let summary: String
    let facts: [Fact]
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        if !summary.isEmpty || !facts.isEmpty {
            VStack(alignment: .leading, spacing: 16) {
                Text(L10n.string("ui.game.about"))
                    .font(.system(size: 20, weight: .semibold))
                    .accessibilityAddTraits(.isHeader)
                VStack(alignment: .leading, spacing: 18) {
                    if !summary.isEmpty {
                        Text(summary)
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                            .lineSpacing(3)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                    if !facts.isEmpty {
                        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 24, verticalSpacing: 10) {
                            ForEach(facts) { fact in
                                GridRow {
                                    Text(fact.label)
                                        .foregroundStyle(.secondary)
                                        .gridColumnAlignment(.leading)
                                    Text(fact.value)
                                        .fixedSize(horizontal: false, vertical: true)
                                        .textSelection(.enabled)
                                }
                            }
                        }
                        .font(.system(size: 12))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(17)
                .background(ObeliskPalette(colorScheme: colorScheme).surface, in: RoundedRectangle(cornerRadius: 10))
            }
            .padding(.top, 5)
        }
    }
}
