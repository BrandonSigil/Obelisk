import SwiftUI

struct EpicLoginView: View {
    @Bindable var model: EpicModel

    var body: some View {
        StorefrontSignInPage(
            section: .epic,
            title: L10n.string("ui.epic.sign_in_title"),
            subtitle: L10n.string("ui.epic.sign_in_subtitle"),
            warning: $model.storageWarning
        ) {
            if model.isRestoring || model.isConnecting || model.isSigningOut {
                progress
            } else {
                signIn
            }
        }
    }

    private var progress: some View {
        let title = model.isSigningOut ? L10n.string("ui.account.signing_out")
            : model.isRestoring ? L10n.string("ui.account.restoring_session")
            : L10n.string("ui.epic.connecting")
        return SignInProgress(title: title, cancel: model.isSigningOut ? nil : { model.cancelSignIn() })
    }

    private var signIn: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(L10n.string("ui.epic.account_sign_in"))
                .font(.system(size: 16, weight: .semibold))
            Text(L10n.string("ui.epic.sign_in_instructions"))
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            SignInRememberToggle(isOn: $model.rememberSession)
            VStack(spacing: 10) {
                Button { model.signIn() } label: {
                    Label(L10n.string("ui.epic.sign_in"), systemImage: "arrow.up.forward.app")
                        .font(.system(size: 13, weight: .semibold))
                        .frame(maxWidth: .infinity)
                }
                .obeliskButtonStyle(.glassProminent)
                .controlSize(.extraLarge)
                if model.canRetryRestore {
                    Button(action: model.retryRestore) {
                        Text(L10n.string("ui.account.retry_saved_session"))
                            .font(.system(size: 13, weight: .medium))
                            .frame(maxWidth: .infinity)
                    }
                    .obeliskButtonStyle(.glass)
                    .controlSize(.extraLarge)
                }
            }
            .padding(.top, 4)
        }
    }
}
