import AppKit
import SwiftUI

extension EnvironmentValues {
    /// Opens the bug report sheet over the whole window.
    @Entry var reportBug: (@MainActor (BugReportSubject) -> Void)?
}

/// The sheet behind a game's SOS button and Settings' "I found a bug": what
/// happened, the logs that go with it and how to reach the player, then the
/// code the report got.
struct BugReportSheet: View {
    @Bindable var model: BugReportModel
    let close: () -> Void
    @State private var copied = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Group {
            if case let .sent(code) = model.phase {
                sent(code)
            } else {
                form
            }
        }
        .padding(24)
        .frame(width: 500)
        .tint(ObeliskStyle.accent)
        .interactiveDismissDisabled(model.phase == .sending)
        .task { await model.load() }
        .accessibilityIdentifier("bug-report-sheet")
    }

    private var title: String {
        switch model.subject {
        case .app: L10n.string("ui.report.app_title", AppIdentity.name)
        case let .game(_, _, name): L10n.string("ui.report.game_title", name)
        }
    }

    private var fieldBackground: Color {
        .primary.opacity(colorScheme == .dark ? 0.08 : 0.05)
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Text(title)
                    .font(.system(size: 19, weight: .semibold))
                    .accessibilityAddTraits(.isHeader)
                Text(L10n.string("ui.report.message", AppIdentity.name))
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            field(L10n.string("ui.report.description")) {
                TextEditor(text: $model.description)
                    .font(.system(size: 13))
                    .scrollContentBackground(.hidden)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 6)
                    .frame(height: 110)
                    .background(fieldBackground, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .accessibilityLabel(L10n.string("ui.report.description"))
                    .accessibilityIdentifier("bug-report-description")
            }
            field(L10n.string("ui.report.contact")) {
                TextField(L10n.string("ui.report.contact"), text: $model.contact,
                          prompt: Text(L10n.string("ui.report.contact_prompt")))
                    .textFieldStyle(.roundedBorder)
                    .labelsHidden()
                    .accessibilityIdentifier("bug-report-contact")
            }
            logs
            if model.phase == .failed {
                Text(L10n.string("ui.report.failed"))
                    .font(.system(size: 12))
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button(L10n.string("ui.common.cancel"), role: .cancel, action: close)
                    .obeliskButtonStyle(.bordered)
                    .keyboardShortcut(.cancelAction)
                    .disabled(model.phase == .sending)
                Spacer(minLength: 8)
                if model.phase == .sending {
                    ProgressView().controlSize(.small)
                }
                Button(L10n.string("ui.report.send")) { Task { await model.send() } }
                    .obeliskButtonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!model.canSend)
                    .accessibilityIdentifier("bug-report-send")
            }
        }
    }

    private func field(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label)
                .font(.system(size: 12, weight: .medium))
                .accessibilityHidden(true)
            content()
        }
    }

    /// The Attach logs switch, what goes out while it is on, and what the
    /// report holds besides.
    private var logs: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Toggle(L10n.string("ui.report.attach_logs"), isOn: $model.attachesLogs)
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .font(.system(size: 12, weight: .medium))
                    .accessibilityIdentifier("bug-report-attach-logs")
                Spacer()
                if model.attachesLogs, model.files.contains(where: { $0.url != nil }) {
                    Button(L10n.string("ui.settings.show_in_finder"), systemImage: "folder", action: model.revealFiles)
                        .obeliskButtonStyle(.bordered)
                        .controlSize(.small)
                }
            }
            if model.attachesLogs {
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(model.files) { file in
                            HStack(spacing: 6) {
                                Image(systemName: file.url == nil ? "list.bullet.rectangle" : "doc.text")
                                    .frame(width: 14)
                                Text(file.url == nil ? L10n.string("ui.report.unified_log", AppIdentity.name) : file.name)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            .font(.system(size: 11, design: file.url == nil ? .default : .monospaced))
                            .foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                }
                .frame(maxHeight: 112)
                .fixedSize(horizontal: false, vertical: true)
                .background(fieldBackground, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
            Text(L10n.string("ui.report.privacy", AppIdentity.name))
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func sent(_ code: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(L10n.string("ui.report.sent"), systemImage: "checkmark.circle.fill")
                .font(.system(size: 19, weight: .semibold))
                .labelStyle(SentLabelStyle())
                .accessibilityAddTraits(.isHeader)
            Text(L10n.string("ui.report.sent_message"))
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Text(code)
                    .font(.system(size: 26, weight: .semibold, design: .monospaced))
                    .textSelection(.enabled)
                    .accessibilityIdentifier("bug-report-code")
                Spacer()
                Button(copied ? L10n.string("ui.report.copied") : L10n.string("ui.report.copy"),
                       systemImage: copied ? "checkmark" : "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    copied = true
                }
                .obeliskButtonStyle(.bordered)
                .accessibilityIdentifier("bug-report-copy")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(fieldBackground, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            HStack {
                Spacer()
                Button(L10n.string("ui.report.done"), action: close)
                    .obeliskButtonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
        }
    }
}

/// The sent title, its check in green.
private struct SentLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 10) {
            configuration.icon.foregroundStyle(.green)
            configuration.title
        }
    }
}

/// A game page's SOS button, beside its management menu: reports a problem
/// with the game, with its logs.
struct GameReportButton: View {
    let subject: BugReportSubject
    let gameName: String
    @Environment(\.reportBug) private var reportBug

    var body: some View {
        Button { reportBug?(subject) } label: {
            StorefrontGlassIcon(systemName: "sos", size: 13)
                .foregroundStyle(.primary)
        }
        .obeliskButtonStyle(.plain)
        .fixedSize()
        .help(L10n.string("ui.game.report"))
        .accessibilityLabel(L10n.string("ui.game.report_named", gameName))
        .accessibilityIdentifier("game-report-button")
    }
}
