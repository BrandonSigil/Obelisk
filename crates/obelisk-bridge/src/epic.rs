//! Epic's account seam. Only refresh credentials cross into the macOS Keychain;
//! the account access token stays inside the Rust session.
use std::sync::Arc;

use crate::{LibraryCache, library_cache::EpicLibrary};
use epic_kit::{auth, client};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EpicError {
    #[error("backend.epic.network")]
    Network,
    #[error("backend.epic.unauthorized")]
    Unauthorized,
    #[error("backend.epic.invalid_code")]
    InvalidCode,
    #[error("backend.epic.invalid_response")]
    InvalidResponse,
    #[error("{message}")]
    Service { message: String },
}

impl From<epic_kit::EpicError> for EpicError {
    fn from(error: epic_kit::EpicError) -> Self {
        match error {
            epic_kit::EpicError::Network => Self::Network,
            epic_kit::EpicError::Unauthorized => Self::Unauthorized,
            epic_kit::EpicError::InvalidCode => Self::InvalidCode,
            epic_kit::EpicError::Protocol => Self::InvalidResponse,
            epic_kit::EpicError::Service { message } => Self::Service { message },
            error => Self::Service {
                message: error.to_string(),
            },
        }
    }
}

#[uniffi::export]
pub fn epic_login_url() -> String {
    auth::login_url()
}

#[uniffi::export]
pub fn epic_authorization_code(input: String) -> Result<String, EpicError> {
    auth::authorization_code(&input).map_err(Into::into)
}

/// The directory name the Epic engine installs into, so the host removes only
/// directories it created without re-deriving the hashing scheme.
#[uniffi::export]
pub fn epic_installation_directory_name(
    namespace: String,
    catalog_item_id: String,
    app_name: String,
    platform: String,
) -> String {
    client::installation_directory_name(&namespace, &catalog_item_id, &app_name, &platform)
}

#[derive(Clone, uniffi::Record)]
pub struct EpicCredentials {
    pub account_id: String,
    pub display_name: String,
    pub refresh_token: String,
}

impl From<auth::Credentials> for EpicCredentials {
    fn from(credentials: auth::Credentials) -> Self {
        Self {
            account_id: credentials.account_id,
            display_name: credentials.display_name,
            refresh_token: credentials.refresh_token,
        }
    }
}

impl From<EpicCredentials> for auth::Credentials {
    fn from(credentials: EpicCredentials) -> Self {
        Self {
            account_id: credentials.account_id,
            display_name: credentials.display_name,
            refresh_token: credentials.refresh_token,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EpicGame {
    pub app_name: String,
    pub namespace: String,
    pub catalog_item_id: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_image_url: Option<String>,
    pub developer: String,
    pub windows: bool,
    pub macos: bool,
}

impl From<client::Game> for EpicGame {
    fn from(game: client::Game) -> Self {
        Self {
            app_name: game.app_name,
            namespace: game.namespace,
            catalog_item_id: game.catalog_item_id,
            title: game.title,
            description: game.description,
            image_url: game.image_url,
            header_image_url: game.header_image_url,
            developer: game.developer,
            windows: game.windows,
            macos: game.macos,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct EpicAchievement {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon_url: String,
    pub hidden: bool,
    pub unlocked: bool,
    pub unlock_date: Option<String>,
}
impl From<client::Achievement> for EpicAchievement {
    fn from(value: client::Achievement) -> Self {
        Self {
            id: value.id,
            name: value.name,
            description: value.description,
            icon_url: value.icon_url,
            hidden: value.hidden,
            unlocked: value.unlocked,
            unlock_date: value.unlock_date,
        }
    }
}

/// What a game's page adds to its library entry.
#[derive(Clone, Debug, uniffi::Record)]
pub struct EpicGameDetails {
    /// Unknown when Epic serves no manifest of the build.
    pub installation_bytes: Option<u64>,
}

impl From<client::GameDetails> for EpicGameDetails {
    fn from(details: client::GameDetails) -> Self {
        Self {
            installation_bytes: details.installation_bytes,
        }
    }
}

/// How long the account played one game, as Epic records it.
#[derive(Clone, Debug, uniffi::Record)]
pub struct EpicPlaytime {
    pub app_name: String,
    pub seconds: u64,
}

#[derive(uniffi::Object)]
pub struct EpicSession {
    /// Shared with the Cloud maintenance this session starts.
    pub(crate) client: Arc<client::Client>,
    pub(crate) persistence: Arc<crate::credentials::Persistence>,
    /// Where the account's library is kept as it loads.
    library: Option<EpicLibrary>,
}
impl EpicSession {
    async fn with(
        client: client::Client,
        library: Option<Arc<LibraryCache>>,
    ) -> Result<Arc<Self>, EpicError> {
        let library = match library {
            Some(library) => Some(library.epic(&client.credentials().await?.account_id)),
            None => None,
        };
        Ok(Arc::new(Self {
            client: Arc::new(client),
            persistence: Default::default(),
            library,
        }))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl EpicSession {
    pub async fn achievements(
        &self,
        namespace: String,
        locale: String,
    ) -> Result<Vec<EpicAchievement>, EpicError> {
        Ok(self
            .client
            .achievements(&namespace, &locale)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Keeps every library it loads in `library`.
    #[uniffi::constructor]
    pub async fn sign_in(
        authorization_code: String,
        library: Option<Arc<LibraryCache>>,
    ) -> Result<Arc<Self>, EpicError> {
        Self::with(client::Client::sign_in(&authorization_code).await?, library).await
    }

    /// Keeps every library it loads in `library`.
    #[uniffi::constructor]
    pub async fn restore(
        credentials: EpicCredentials,
        library: Option<Arc<LibraryCache>>,
    ) -> Result<Arc<Self>, EpicError> {
        Self::with(client::Client::restore(credentials.into()).await?, library).await
    }

    pub async fn credentials(&self) -> Result<EpicCredentials, EpicError> {
        Ok(self.client.credentials().await?.into())
    }

    pub async fn start_friends(&self) {
        self.client.start_friends();
    }
    pub async fn reconnect_friends(&self) {
        self.client.reconnect_friends();
    }
    pub fn stop_friends(&self) {
        self.client.stop_friends();
    }
    pub fn shutdown_observers(&self) {
        self.client.shutdown_observers();
    }
    pub async fn next_friends(&self, after_revision: u64) -> crate::EpicFriendsSnapshot {
        self.client.next_friends(after_revision).await.into()
    }
    pub async fn next_credentials_revision(&self, after_revision: u64) -> Option<u64> {
        self.client.next_credentials_revision(after_revision).await
    }

    /// Saves the account's credentials through `credentials` now and after
    /// every rotation, one save at a time and in order, until the session's
    /// observers stop. Every account call relies on it.
    pub async fn persist(&self, credentials: Arc<dyn crate::CredentialSave>) {
        crate::credentials::persist(self.client.clone(), self.persistence.clone(), credentials);
    }

    pub async fn games(&self) -> Result<Vec<EpicGame>, EpicError> {
        let games = self.client.games().await?;
        Ok(match &self.library {
            Some(library) => library.games(games),
            None => games.into_iter().map(Into::into).collect(),
        })
    }

    /// Every game the account played, in one request.
    pub async fn playtimes(&self) -> Result<Vec<EpicPlaytime>, EpicError> {
        let playtimes = self.client.playtimes().await?;
        if let Some(library) = &self.library {
            library.playtimes(&playtimes);
        }
        Ok(playtimes
            .into_iter()
            .map(|(app_name, seconds)| EpicPlaytime { app_name, seconds })
            .collect())
    }

    pub async fn disconnect(&self) -> Result<(), EpicError> {
        self.client.disconnect().await.map_err(Into::into)
    }

    /// Opens Epic's site signed in as the account, then `redirect_url`, an
    /// Epic page. The address holds a single-use code: never keep or log it.
    pub async fn web_sign_in_url(&self, redirect_url: String) -> Result<String, EpicError> {
        Ok(self.client.web_sign_in_url(&redirect_url).await?)
    }

    pub async fn game_details(
        &self,
        namespace: String,
        catalog_item_id: String,
        app_name: String,
        platform: String,
    ) -> Result<EpicGameDetails, EpicError> {
        Ok(self
            .client
            .game_details(&namespace, &catalog_item_id, &app_name, &platform)
            .await?
            .into())
    }
}
