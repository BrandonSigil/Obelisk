import AppKit
import SwiftUI

/// A storefront page's scroll view. Its scrollbar floats over the content as
/// an overlay scroller, whatever the system setting, so artwork reaches the
/// window's edge and the content width never changes with the scrollbar.
struct StorefrontScrollView<Content: View>: View {
    private let content: Content
    private let topOverlayHeight: CGFloat

    init(topOverlayHeight: CGFloat = 0, @ViewBuilder content: () -> Content) {
        self.topOverlayHeight = topOverlayHeight
        self.content = content()
    }

    var body: some View {
        ScrollView {
            content
                .padding(.top, topOverlayHeight)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background {
                    StorefrontOverlayScroller(topInset: topOverlayHeight)
                        .frame(width: 0, height: 0)
                }
        }
    }
}

private struct StorefrontOverlayScroller: NSViewRepresentable {
    let topInset: CGFloat

    func makeNSView(context: Context) -> ScrollerConfigurationView {
        let view = ScrollerConfigurationView()
        view.topInset = topInset
        return view
    }

    func updateNSView(_ nsView: ScrollerConfigurationView, context: Context) {
        nsView.topInset = topInset
        nsView.configureScroller()
    }

    final class ScrollerConfigurationView: NSView {
        var topInset: CGFloat = 0

        override func viewDidMoveToSuperview() {
            super.viewDidMoveToSuperview()
            configureScroller()
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            configureScroller()
        }

        override func layout() {
            super.layout()
            configureScroller()
        }

        func configureScroller() {
            guard let scrollView = enclosingScrollView else { return }
            if scrollView.scrollerStyle != .overlay { scrollView.scrollerStyle = .overlay }
            if !scrollView.hasVerticalScroller { scrollView.hasVerticalScroller = true }
            // The scrollbar starts below the floating bar.
            if scrollView.scrollerInsets.top != topInset {
                scrollView.scrollerInsets.top = topInset
            }
        }
    }
}
