import SwiftUI

/// The launch settings of one installed Windows game, as its sheet shows
/// them: read from Rust, and kept there at each change.
@MainActor @Observable
final class LaunchSettingsModel: Identifiable {
    private(set) var settings: LaunchSettings?
    private(set) var isLoading = true
    private(set) var error: String?
    @ObservationIgnored private let load: () async throws -> LaunchSettings?
    @ObservationIgnored private let save: (LaunchChoices) async throws -> LaunchSettings?

    init(load: @escaping () async throws -> LaunchSettings?,
         save: @escaping (LaunchChoices) async throws -> LaunchSettings?) {
        self.load = load
        self.save = save
    }

    func appear() async {
        isLoading = true
        do {
            settings = try await load()
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
        isLoading = false
    }

    /// Changes one of the player's choices and keeps them: Rust drops those
    /// the game already has by default.
    func choose(_ change: (inout LaunchChoices) -> Void) {
        guard var choices = settings?.choices else { return }
        change(&choices)
        Task { await keep(choices) }
    }

    /// Keeps what the player typed as the game's launch arguments, when it
    /// changed. Spaces alone are no arguments.
    func setArguments(_ text: String) {
        let arguments = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let settings, (settings.choices.arguments ?? "") != arguments else { return }
        choose { $0.arguments = arguments.isEmpty ? nil : arguments }
    }

    /// Forgets every choice: the game runs as Obelisk knows it best.
    func reset() {
        Task { await keep(LaunchChoices()) }
    }

    private func keep(_ choices: LaunchChoices) async {
        do {
            if let saved = try await save(choices) { settings = saved }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    var isCustomized: Bool { settings.map { $0.choices != LaunchChoices() } ?? false }

    /// What the chosen engine offers.
    var engineOffer: LaunchEngineOffer? {
        settings.flatMap { settings in settings.engines.first { $0.engine == settings.engine } }
    }

    /// What the chosen backend offers on the chosen engine.
    var graphicsOffer: LaunchGraphicsOffer? {
        guard let settings else { return nil }
        return engineOffer?.graphics.first { $0.graphics == settings.graphics }
    }
}

/// The sheet behind a Windows game's Launch Settings: the arguments added to
/// its command line, its engine, then the CPU translator that engine has,
/// then the graphics backend and its options, each as the chosen engine
/// allows, every one said in a sentence.
struct LaunchSettingsSheet: View {
    let model: LaunchSettingsModel
    let gameName: String
    /// The arguments as the player types them, kept once the field is done.
    @State private var arguments = ""
    @FocusState private var editsArguments: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GameSheetHeader(title: L10n.string("ui.launch_settings.title"), systemImage: "slider.horizontal.3",
                            gameName: gameName, closeIdentifier: "launch-settings-close")
                .padding(.horizontal, 24)
                .padding(.top, 24)
                .padding(.bottom, 8)
            content
        }
        .frame(width: 540, height: 680)
        .task {
            await model.appear()
            arguments = model.settings?.choices.arguments ?? ""
        }
        .onChange(of: model.settings?.choices.arguments) { _, kept in
            if !editsArguments { arguments = kept ?? "" }
        }
        .onChange(of: editsArguments) { _, editing in
            if !editing { model.setArguments(arguments) }
        }
        .onDisappear { model.setArguments(arguments) }
        .accessibilityIdentifier("launch-settings-sheet")
    }

    @ViewBuilder
    private var content: some View {
        if model.isLoading {
            ProgressView(L10n.string("ui.launch_settings.loading"))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let settings = model.settings {
            form(settings)
        } else if let error = model.error {
            Text(error)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func form(_ settings: LaunchSettings) -> some View {
        Form {
            Section {
                // A whole command line: a bordered field across the row,
                // under its label, rather than the form's trailing value.
                VStack(alignment: .leading, spacing: 8) {
                    Text(L10n.string("ui.launch_settings.arguments"))
                    TextField(L10n.string("ui.launch_settings.arguments"), text: $arguments,
                              prompt: Text(L10n.string("ui.launch_settings.arguments_placeholder")))
                        .textFieldStyle(.roundedBorder)
                        .labelsHidden()
                        .focused($editsArguments)
                        .onSubmit { model.setArguments(arguments) }
                        .accessibilityIdentifier("launch-settings-arguments")
                }
                caption(L10n.string("ui.launch_settings.arguments_detail"))
            }
            Section {
                Picker(L10n.string("ui.launch_settings.engine"), selection: Binding(
                    get: { settings.engine },
                    set: { engine in model.choose { $0.engine = engine } }
                )) {
                    ForEach(settings.engines, id: \.engine) { offer in
                        Text(marked(Self.name(of: offer.engine), isDefault: offer.engine == settings.defaultEngine))
                            .tag(offer.engine)
                    }
                }
                .pickerStyle(.menu)
                .accessibilityIdentifier("launch-settings-engine")
                caption(Self.detail(of: settings.engine))
            }
            Section(L10n.string("ui.launch_settings.cpu")) {
                if let offer = model.engineOffer {
                    LabeledContent(L10n.string("ui.launch_settings.translator"), value: Self.name(of: offer.translator))
                }
            }
            Section(L10n.string("ui.launch_settings.graphics")) {
                if let engine = model.engineOffer {
                    Picker(L10n.string("ui.launch_settings.backend"), selection: Binding(
                        get: { settings.graphics },
                        set: { graphics in model.choose { $0.graphics = graphics } }
                    )) {
                        ForEach(engine.graphics, id: \.graphics) { offer in
                            Text(marked(Self.name(of: offer.graphics),
                                        isDefault: offer.graphics == engine.defaultGraphics))
                                .tag(offer.graphics)
                        }
                    }
                    .pickerStyle(.menu)
                    .accessibilityIdentifier("launch-settings-graphics")
                    caption(Self.detail(of: settings.graphics))
                }
                if model.graphicsOffer?.options.contains(.rayTracing) == true {
                    Toggle(isOn: Binding(get: { settings.rayTracing },
                                         set: { rayTracing in model.choose { $0.rayTracing = rayTracing } })) {
                        Text(L10n.string("ui.launch_settings.ray_tracing"))
                        Text(L10n.string("ui.launch_settings.ray_tracing_detail"))
                    }
                    .toggleStyle(.switch)
                    .accessibilityIdentifier("launch-settings-ray-tracing")
                }
            }
            Section {
                HStack {
                    Text(L10n.string("ui.launch_settings.next_launch"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button(L10n.string("ui.launch_settings.reset"), action: model.reset)
                        .disabled(!model.isCustomized)
                        .accessibilityIdentifier("launch-settings-reset")
                }
                if let error = model.error {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
        }
        .formStyle(.grouped)
    }

    private func marked(_ name: String, isDefault: Bool) -> String {
        isDefault ? L10n.string("ui.launch_settings.default", name) : name
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    static func name(of engine: LaunchEngine) -> String {
        switch engine {
        case .rosetta: L10n.string("ui.launch_settings.engine.rosetta")
        case .linux: L10n.string("ui.launch_settings.engine.linux")
        }
    }

    static func detail(of engine: LaunchEngine) -> String {
        switch engine {
        case .rosetta: L10n.string("ui.launch_settings.engine.rosetta_detail")
        case .linux: L10n.string("ui.launch_settings.engine.linux_detail")
        }
    }

    static func name(of translator: LaunchTranslator) -> String {
        switch translator {
        case .rosetta: L10n.string("ui.launch_settings.translator.rosetta")
        case .fex: L10n.string("ui.launch_settings.translator.fex")
        }
    }

    static func name(of graphics: LaunchGraphics) -> String {
        switch graphics {
        case .d3dMetal: L10n.string("ui.launch_settings.graphics.d3dmetal")
        case .dxmt: L10n.string("ui.launch_settings.graphics.dxmt")
        }
    }

    static func detail(of graphics: LaunchGraphics) -> String {
        switch graphics {
        case .d3dMetal: L10n.string("ui.launch_settings.graphics.d3dmetal_detail")
        case .dxmt: L10n.string("ui.launch_settings.graphics.dxmt_detail")
        }
    }
}
