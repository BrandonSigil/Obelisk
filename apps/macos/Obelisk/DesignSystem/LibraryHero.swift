import SwiftUI

/// A game shown wide at the top of a library, its artwork reaching under the
/// library's floating bar. Its text stays light, with its own shadow, in both
/// appearances.
struct LibraryHero<Artwork: View, Logo: View>: View {
    let details: [Detail]
    let open: () -> Void
    @ViewBuilder let artwork: Artwork
    /// The game's logo, or its title when the storefront has no logo.
    @ViewBuilder let logo: Logo
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    struct Detail: Identifiable {
        let symbol: String
        let text: String
        var id: String { symbol }
    }

    var body: some View {
        // How much of the scrolling the artwork follows less than the page.
        let parallax = reduceMotion ? 0 : 0.7
        // How far the artwork reaches up under the library's bar.
        let reach = ObeliskStyle.titleBarHeight
        return Color.clear
            .overlay {
                artwork
                    .visualEffect { content, geometry in
                        let top = geometry.frame(in: .scrollView).minY
                        let height = max(geometry.size.height, 1)
                        // Scrolled down, the artwork lags behind the page; pulled
                        // past the top, it stretches over the gap above it.
                        return content
                            .offset(y: top < 0 ? -top * parallax : 0)
                            .scaleEffect(top > 0 ? 1 + top / height : 1, anchor: .bottom)
                    }
            }
            .clipShape(OpenTopRectangle())
            .backgroundExtensionEffect()
            .overlay(alignment: .bottomLeading) {
                VStack(alignment: .leading, spacing: 14) {
                    logo
                        .shadow(color: .black.opacity(0.35), radius: 14, y: 4)
                    HStack(spacing: 16) {
                        ForEach(details) { detail in
                            Label(detail.text, systemImage: detail.symbol)
                        }
                    }
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.white)
                    .shadow(color: .black.opacity(0.6), radius: 6)
                    Button(action: open) {
                        Label(L10n.string("ui.library.hero.open"), systemImage: "arrow.right")
                            .labelStyle(TrailingIconLabelStyle())
                            .font(.system(size: 13, weight: .semibold))
                            .padding(.horizontal, 6)
                    }
                    .obeliskButtonStyle(.glassProminent)
                    .controlSize(.large)
                    .padding(.top, 4)
                }
                .padding(.horizontal, ObeliskStyle.contentPadding)
                .padding(.bottom, 28)
                // The text fades out as it slides under the bar's controls.
                .visualEffect { [reach] content, geometry in
                    // Fully visible until its top nears the bar, gone a little
                    // past the bar's bottom edge.
                    let top = geometry.frame(in: .scrollView).minY
                    return content.opacity(min(max((top - reach + 40) / 80, 0), 1))
                }
            }
            .frame(height: ObeliskStyle.libraryHeroHeight + reach)
            .padding(.top, -reach)
            .environment(\.colorScheme, .dark)
    }
}

/// The hero's frame, open above so the stretched artwork can fill an overscroll.
private struct OpenTopRectangle: Shape {
    func path(in rect: CGRect) -> Path {
        Path(CGRect(x: rect.minX, y: rect.minY - 10_000, width: rect.width, height: rect.height + 10_000))
    }
}

/// The title, then its icon, as in "View game →".
private struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 7) {
            configuration.title
            configuration.icon
        }
    }
}

/// The featured game's logo, or its title when its store publishes none.
/// Without `showsTitle`, the title is only read aloud, for artwork that
/// already shows it.
struct LibraryHeroLogo: View {
    let url: URL?
    let title: String
    var showsTitle = true

    var body: some View {
        Group {
            if let url {
                AsyncImage(url: url) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit()
                    } else if phase.error != nil {
                        titleText
                    } else {
                        Color.clear
                    }
                }
            } else if showsTitle {
                titleText
            } else {
                Color.clear
            }
        }
        .frame(width: 360, height: url != nil || showsTitle ? 120 : 0, alignment: .bottomLeading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityAddTraits(.isHeader)
    }

    private var titleText: some View {
        Text(title)
            .font(.system(size: 38, weight: .bold))
            .foregroundStyle(.white)
            .lineLimit(2)
            .minimumScaleFactor(0.6)
    }
}
