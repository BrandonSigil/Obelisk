//! GOG game ownership and Game sessions. GOG's builds are DRM free: a launch
//! needs no account credential, only its installation. No Cloud sync runs
//! yet: GOG Cloud saves need the game's own Galaxy credentials.
mod client;
pub mod content;
mod launch;
mod windows;

use crate::supervisor::{self, Record};
pub use crate::supervisor::{FailureReason, Phase};
pub use client::start;
pub use launch::serve;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub use windows::prepare_windows;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("backend.gog.session")]
    Session,
    #[error("backend.gog.busy")]
    Busy,
    #[error("backend.gog.cancelled")]
    Cancelled,
    #[error("backend.gog.storage")]
    Storage,
    #[error("backend.gog.unsupported")]
    Unsupported,
    #[error("backend.runtime.rosettaUnavailable")]
    RosettaUnavailable,
    #[error("backend.runtime.wineUnavailable")]
    WineUnavailable,
    #[error("{0}")]
    Gog(#[from] gog_kit::GogError),
    #[error("backend.gog.storage")]
    Io(#[from] std::io::Error),
}
impl From<supervisor::Error> for Error {
    fn from(error: supervisor::Error) -> Self {
        match error {
            supervisor::Error::Storage => Self::Storage,
            supervisor::Error::NewerSession | supervisor::Error::Busy => Self::Busy,
            supervisor::Error::Cancelled => Self::Cancelled,
            supervisor::Error::RosettaUnavailable => Self::RosettaUnavailable,
            supervisor::Error::InvalidSession
            | supervisor::Error::Unavailable
            | supervisor::Error::StartFailed => Self::Session,
        }
    }
}
impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        match error {
            crate::Error::RosettaUnavailable => Self::RosettaUnavailable,
            crate::Error::WineUnavailable => Self::WineUnavailable,
            crate::Error::IncompatibleArchitecture
            | crate::Error::InvalidApplication
            | crate::Error::NotExecutable => Self::Unsupported,
            _ => Self::Session,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// What a GOG Game session starts: one installed build.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub home: PathBuf,
    pub install: PathBuf,
    pub session_directory: PathBuf,
    pub helper: PathBuf,
    pub product_id: String,
    pub platform: String,
    /// Set for a Windows build, which runs in Wine.
    #[serde(default)]
    pub prefix: Option<PathBuf>,
}
impl Configuration {
    /// Whether `wine` runs this build in its own prefix, and only a Windows
    /// build.
    fn runs_in(&self, wine: Option<&crate::wine::Config>) -> bool {
        self.prefix.as_ref() == wine.map(|wine| &wine.prefix)
    }
    fn validate(&self, account: &str) -> Result<()> {
        // A Mac build runs natively; a Windows build in its prefix.
        let prefix = match (self.platform.as_str(), &self.prefix) {
            (gog_kit::client::MACOS, None) => None,
            (gog_kit::client::WINDOWS, Some(prefix)) => Some(prefix),
            _ => return Err(Error::Unsupported),
        };
        if [
            &self.home,
            &self.install,
            &self.session_directory,
            &self.helper,
        ]
        .into_iter()
        .chain(prefix)
        .any(|p| {
            !p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
        }) || !gog_kit::client::valid_product_id(&self.product_id)
            || !gog_kit::auth::valid_user_id(account)
        {
            return Err(Error::Unsupported);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gog;

/// The game a GOG session runs: its product.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub product_id: String,
}

pub type Snapshot = Record<Gog>;
pub type Client = supervisor::Client<Gog>;

impl supervisor::Provider for Gog {
    type Account = String;
    type Store = Store;
    type Error = Error;
    const SUPERVISE: &'static str = "gog-supervise";
    const CANCELLED: &'static [&'static str] = &["backend.gog.cancelled"];

    fn valid(record: &Record<Self>) -> bool {
        gog_kit::auth::valid_user_id(&record.account_id)
            && gog_kit::client::valid_product_id(&record.store.product_id)
            && record.socket.is_absolute()
    }

    fn cancelled(error: &Error) -> bool {
        matches!(
            error,
            Error::Cancelled | Error::Gog(gog_kit::GogError::Cancelled)
        )
    }

    fn same_game(a: &Store, b: &Store) -> bool {
        a == b
    }

    /// A supervisor that is gone left a game that may still run.
    fn orphaned(record: &mut Record<Self>, owned: bool) {
        if owned {
            record.reconnecting = true;
        }
        record.cloud.allowed_choices.clear();
    }
}

impl crate::sessions::Storefront for Gog {
    type Game = String;
    const BEGINS: bool = false;
    const INVALID_SESSION: &'static str = "backend.gog.invalidSession";
    const SESSION_FAILED: &'static str = "backend.gog.session";
    fn game(store: &Store) -> String {
        store.product_id.clone()
    }
}

/// What the handoff pipe carries to a GOG supervisor. It holds no credential.
#[derive(Serialize, Deserialize)]
struct Request {
    config: Configuration,
    id: String,
    account_id: String,
    /// Built in Obelisk's process: a supervisor starts without any environment.
    #[serde(default)]
    environment: std::collections::HashMap<String, String>,
    /// How a Windows build runs.
    #[serde(default)]
    wine: Option<crate::wine::Config>,
}

/// A GOG Installation receipt, as the account's receipt file records it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub product_id: String,
    pub name: String,
    pub directory: String,
    pub platform: String,
    pub total_bytes: u64,
    /// A Transfer started over this installation and has not succeeded since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_verification: Option<bool>,
}

/// A GOG Transfer of one game's build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub product_id: String,
    pub title: String,
    pub platform: String,
    /// The parent folder the installation directory goes in.
    pub directory: PathBuf,
}

/// A GOG game Obelisk can install: its build for `platform`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub product_id: String,
    pub title: String,
    pub platform: String,
}

impl crate::installations::Storefront for Gog {
    type Receipt = Receipt;
    type Request = Download;
    type Offer = Offer;
    const RECEIPTS_UNAVAILABLE: &'static str = "model.gog.receiptsUnavailable";
    const RECEIPTS_NOT_SAVED: &'static str = "model.gog.receiptsNotSaved";
    const GAMES_DIRECTORY: &'static str = "Games/GOG";
    fn receipt_game(receipt: &Receipt) -> String {
        receipt.product_id.clone()
    }
    fn receipt_id(receipt: &Receipt) -> String {
        format!("{}:{}", receipt.product_id, receipt.platform)
    }
    fn receipt_platform(receipt: &Receipt) -> &str {
        &receipt.platform
    }
    fn receipt_directory(receipt: &Receipt) -> &str {
        &receipt.directory
    }
    fn needs_verification(receipt: &Receipt) -> bool {
        receipt.needs_verification == Some(true)
    }
    fn mark_needs_verification(receipt: &mut Receipt) {
        receipt.needs_verification = Some(true);
    }
    fn request_game(request: &Download) -> String {
        request.product_id.clone()
    }
    fn request_platform(request: &Download) -> Option<&str> {
        Some(&request.platform)
    }
    fn request_directory(request: &Download) -> &std::path::Path {
        &request.directory
    }
    fn transfer_name(request: &Download) -> String {
        request.product_id.clone()
    }
    fn offer_game(offer: &Offer) -> String {
        offer.product_id.clone()
    }
    fn offer_platform(offer: &Offer) -> Option<&str> {
        Some(&offer.platform)
    }
    fn install_request(offer: &Offer, directory: &std::path::Path) -> Download {
        Download {
            product_id: offer.product_id.clone(),
            title: offer.title.clone(),
            platform: offer.platform.clone(),
            directory: directory.to_owned(),
        }
    }
    fn update_request(receipt: &Receipt) -> Download {
        Download {
            product_id: receipt.product_id.clone(),
            title: receipt.name.clone(),
            platform: receipt.platform.clone(),
            directory: crate::steam::parent(&receipt.directory),
        }
    }
}

#[cfg(test)]
mod tests;
