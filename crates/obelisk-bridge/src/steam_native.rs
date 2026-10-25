//! What Steam's Game sessions and Cloud share across UniFFI.
use crate::SteamError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SteamHostState {
    Online,
    Disconnected,
    Lost,
    Stopped,
}

impl From<game_runtime::steam::host::HostState> for SteamHostState {
    fn from(value: game_runtime::steam::host::HostState) -> Self {
        match value {
            game_runtime::steam::host::HostState::Online => Self::Online,
            game_runtime::steam::host::HostState::Disconnected => Self::Disconnected,
            game_runtime::steam::host::HostState::Lost => Self::Lost,
            game_runtime::steam::host::HostState::Stopped => Self::Stopped,
        }
    }
}

impl From<game_runtime::steam::cloud::Failure> for SteamError {
    fn from(failure: game_runtime::steam::cloud::Failure) -> Self {
        Self::Failure {
            reason: failure.key().into(),
        }
    }
}
