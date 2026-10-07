//! Account-scoped GOG requests. Token refresh is serialized, while reads
//! release the session lock so disconnect can invalidate them promptly.
use std::{future::Future, sync::Arc, time::Instant};

use tokio::sync::{Mutex, watch};

use crate::{
    GogError,
    auth::{self, Api, Credentials, Session},
};

mod download;
mod gameplay;
mod launch;
pub use crate::library::{Game, GameDetails, GameInfo};
pub use download::{Installation, TransferProgress, TransferState, installation_directory_name};
pub use gameplay::Achievement;
pub use launch::{InstalledLaunch, Prerequisite, installed_launch};

/// How Obelisk names GOG's two desktop builds.
pub const MACOS: &str = "macos";
pub const WINDOWS: &str = "windows";

/// GOG product ids are decimal numbers; they name URLs and folders.
pub fn valid_product_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// The content system's name for an Obelisk platform.
pub(crate) fn content_platform(platform: &str) -> Result<&'static str, GogError> {
    match platform {
        MACOS => Ok("osx"),
        WINDOWS => Ok("windows"),
        _ => Err(GogError::Protocol),
    }
}

pub struct Client {
    downloads: download::Downloads,
    api: Api,
    session: Arc<Mutex<Option<Session>>>,
    credential_events: watch::Sender<CredentialRevision>,
    avatar_url: Option<String>,
}

impl Client {
    /// Signs in with the code the web sign-in returned.
    pub async fn sign_in(input: &str) -> Result<Self, GogError> {
        let code = auth::authorization_code(input)?;
        let api = Api::new()?;
        let session = api.authorize(&code).await?;
        Self::from_session(api, session).await
    }

    pub async fn restore(credentials: Credentials) -> Result<Self, GogError> {
        let api = Api::new()?;
        let session = api.refresh(&credentials).await?;
        Self::from_session(api, session).await
    }

    /// The session's account, named from its profile.
    async fn from_session(api: Api, mut session: Session) -> Result<Self, GogError> {
        let profile = api.profile(&session).await?;
        if !profile.username.is_empty() {
            session.credentials.username = profile.username;
        }
        Ok(Self {
            downloads: download::Downloads::default(),
            api,
            session: Arc::new(Mutex::new(Some(session))),
            credential_events: watch::channel(CredentialRevision {
                revision: 1,
                stopped: false,
            })
            .0,
            avatar_url: profile.avatar_url,
        })
    }

    /// The account's avatar, as its profile shows it.
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar_url.clone()
    }

    pub async fn credentials(&self) -> Result<Credentials, GogError> {
        self.session
            .lock()
            .await
            .as_ref()
            .map(|session| session.credentials.clone())
            .ok_or(GogError::Unauthorized)
    }

    /// Refresh before a long read so the caller can persist a rotated token
    /// before the read starts.
    pub async fn prepare(&self) -> Result<Credentials, GogError> {
        self.access_token(None).await?;
        self.credentials().await
    }

    /// Every game the account owns, with its artwork and platforms.
    pub async fn games(&self) -> Result<Vec<Game>, GogError> {
        let user_id = self.credentials().await?.user_id;
        let user_id = user_id.as_str();
        self.with_access_token(|token| async move {
            crate::library::games(&self.api, &token, user_id).await
        })
        .await
    }

    /// What a game's page adds to its library entry, for its `platform` build
    /// with the depots of `language`, such as `fr-FR`.
    pub async fn game_details(
        &self,
        product_id: &str,
        platform: &str,
        language: &str,
    ) -> Result<GameDetails, GogError> {
        if !valid_product_id(product_id) {
            return Err(GogError::Protocol);
        }
        let platform = content_platform(platform)?;
        let size = self
            .with_access_token(|token| async move {
                let size = self
                    .installation_size(&token, product_id, platform, language)
                    .await;
                // Re-raise so the funnel refreshes the token once.
                match size {
                    Err(GogError::Unauthorized) => Err(GogError::Unauthorized),
                    size => Ok(size.ok()),
                }
            })
            .await?;
        Ok(GameDetails {
            installation_bytes: size,
        })
    }

    /// The revision of the credentials now, which grows with each rotation.
    pub fn credentials_revision(&self) -> u64 {
        self.credential_events.borrow().revision
    }

    pub async fn next_credentials_revision(&self, after_revision: u64) -> Option<u64> {
        obelisk_core::watch::next_matching(self.credential_events.subscribe(), |state| {
            state.stopped || state.revision != after_revision
        })
        .await
        .ok()
        .filter(|state| !state.stopped)
        .map(|state| state.revision)
    }

    /// Synchronous cancellation crosses UniFFI even when a Swift task is
    /// suspended. This affects no running content transfer.
    pub fn shutdown_observers(&self) {
        self.credential_events
            .send_modify(|state| state.stopped = true);
    }

    /// Forgets the session. GOG offers no revocation of a Galaxy token: it
    /// expires within the hour, and the refresh token is never used again.
    pub async fn disconnect(&self) -> Result<(), GogError> {
        self.shutdown_observers();
        self.cancel_download();
        self.session.lock().await.take();
        Ok(())
    }

    async fn access_token(&self, rejected: Option<&str>) -> Result<String, GogError> {
        self.tokens().access_token(rejected).await
    }

    fn tokens(&self) -> TokenSource {
        TokenSource {
            api: self.api.clone(),
            session: self.session.clone(),
            credential_events: self.credential_events.clone(),
        }
    }

    /// Runs `operation` with a current access token. When GOG rejects that
    /// token, it is refreshed once and the operation runs again. A result
    /// computed for a session the account discarded meanwhile is never
    /// returned.
    pub(crate) async fn with_access_token<T, Fut>(
        &self,
        operation: impl Fn(String) -> Fut,
    ) -> Result<T, GogError>
    where
        Fut: Future<Output = Result<T, GogError>>,
    {
        let token = self.access_token(None).await?;
        let result = match operation(token.clone()).await {
            Err(GogError::Unauthorized) => {
                let token = self.access_token(Some(&token)).await?;
                operation(token).await
            }
            result => result,
        };
        if self.session.lock().await.is_none() {
            return Err(GogError::Unauthorized);
        }
        result
    }
}

#[derive(Clone, Copy)]
struct CredentialRevision {
    revision: u64,
    stopped: bool,
}

/// The account's access token, renewed as it lapses, for work that outlives
/// one call, such as a Transfer's secure links.
#[derive(Clone)]
pub(crate) struct TokenSource {
    api: Api,
    session: Arc<Mutex<Option<Session>>>,
    credential_events: watch::Sender<CredentialRevision>,
}

impl TokenSource {
    /// A current access token, renewed when it lapsed or GOG `rejected` it.
    pub(crate) async fn access_token(&self, rejected: Option<&str>) -> Result<String, GogError> {
        // The renewing task owns the refresh, not the caller awaiting it: a
        // dropped caller never loses a token GOG already rotated.
        let tokens = self.clone();
        let rejected = rejected.map(str::to_owned);
        tokio::spawn(async move {
            let mut guard = tokens.session.lock().await;
            let session = guard.as_mut().ok_or(GogError::Unauthorized)?;
            if session.expires_at <= Instant::now()
                || rejected.as_deref() == Some(&session.access_token)
            {
                *session = tokens.api.refresh(&session.credentials).await?;
                tokens
                    .credential_events
                    .send_modify(|state| state.revision = state.revision.wrapping_add(1));
            }
            Ok(session.access_token.clone())
        })
        .await
        .map_err(|_| GogError::Cancelled)?
    }
}
