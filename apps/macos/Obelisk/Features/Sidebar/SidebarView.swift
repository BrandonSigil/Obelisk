import SwiftUI

struct SidebarView: View {
    @Bindable var model: AppModel

    private let stores: [LibrarySection] = [.steam, .epic, .gog, .other]

    var body: some View {
        List(selection: selection(where: { $0 != .settings })) {
            row(.recent)

            Section(L10n.string("ui.library.title")) {
                ForEach(stores) { row($0) }
            }
        }
        .listStyle(.sidebar)
        .toolbar(removing: .sidebarToggle)
        // Settings stays at the bottom of the sidebar, whatever its height.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            List(selection: selection(where: { $0 == .settings })) {
                row(.settings)
            }
            .listStyle(.sidebar)
            .scrollDisabled(true)
            .frame(height: 52)
        }
    }

    /// One list's share of the app's single selection.
    private func selection(where owns: @escaping (LibrarySection) -> Bool) -> Binding<LibrarySection?> {
        Binding(
            get: { owns(model.selection) ? model.selection : nil },
            set: { if let section = $0 { model.selection = section } }
        )
    }

    private func row(_ section: LibrarySection) -> some View {
        Label {
            Text(section.title)
        } icon: {
            sidebarIcon(section)
        }
        .listItemTint(section == .settings ? .monochrome : .fixed(ObeliskStyle.accent))
        .pointerCursor()
        .tag(section)
    }

    @ViewBuilder
    private func sidebarIcon(_ section: LibrarySection) -> some View {
        if let logoAssetName = section.logoAssetName {
            Image(logoAssetName)
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 16, height: 16)
                .accessibilityHidden(true)
        } else {
            Image(systemName: section.symbol)
        }
    }
}
