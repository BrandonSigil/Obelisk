//! GOG's part of an account's installations: where each build lives, the
//! credentials every account call renews and saves first, and what GOG's
//! network does for them. Only the network is a seam; tests stand in for it.
use super::{Configuration, Download, Error, Gog, Receipt};
use crate::{
    Locations, cloud_status,
    installations::{self, BoxFuture, Runtime, Runtimes, blocking},
    sessions::{self, Admission, Failure, Storefront as _, Supervised},
};
use gog_kit::client::Client as GogClient;
use obelisk_core::{installation::TransferProgress, transfer::CancelToken};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// What an account's installations ask of GOG's network. Every call may wait
/// on it.
pub trait Network: Send + Sync + 'static {
    /// Renews the account's credentials before a long account call.
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>>;
    /// The account the session is signed in to now.
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// Runs one Transfer into the folder `request` names, with the depots of
    /// `language`.
    fn download<'a>(
        &'a self,
        request: &'a Download,
        language: &'a str,
    ) -> BoxFuture<'a, Result<gog_kit::client::Installation, Failure>>;
    fn progress(&self) -> Option<TransferProgress>;
    fn cancel(&self);
    /// Whether GOG's current build differs from the one `receipt` installed.
    fn update_available<'a>(&'a self, receipt: &'a Receipt)
    -> BoxFuture<'a, Result<bool, Failure>>;
}

fn failed(error: impl std::fmt::Display) -> Failure {
    Failure::Failed(error.to_string())
}

impl Network for GogClient {
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move { GogClient::prepare(self).await.map_err(failed) })
    }
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            self.credentials()
                .await
                .map(|credentials| credentials.user_id)
                .map_err(failed)
        })
    }
    fn download<'a>(
        &'a self,
        request: &'a Download,
        language: &'a str,
    ) -> BoxFuture<'a, Result<gog_kit::client::Installation, Failure>> {
        Box::pin(async move {
            GogClient::download(
                self,
                &request.product_id,
                &request.platform,
                language,
                &request.directory,
            )
            .await
            .map_err(failed)
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        GogClient::progress(self)
    }
    fn cancel(&self) {
        self.cancel_download();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            GogClient::update_available(
                self,
                &receipt.product_id,
                &receipt.platform,
                Path::new(&receipt.directory),
            )
            .await
            .map_err(failed)
        })
    }
}

/// One signed-in GOG account's installations, as they reach GOG.
pub struct Content<N: Network> {
    network: Arc<N>,
    /// Waits until the credentials an account call rotated are saved. It
    /// blocks.
    saved: Arc<dyn Fn() + Send + Sync>,
    /// The language whose depots Transfers install, such as `fr-FR`.
    language: String,
    locations: Locations,
    /// A Transfer was cancelled while its credentials were being renewed.
    cancelled: AtomicBool,
    runtimes: Runtimes,
}

impl<N: Network> Content<N> {
    pub fn new(
        network: Arc<N>,
        saved: Arc<dyn Fn() + Send + Sync>,
        language: String,
        locations: Locations,
    ) -> Self {
        Self {
            network,
            saved,
            language,
            runtimes: Runtimes::new(&locations),
            locations,
            cancelled: AtomicBool::new(false),
        }
    }

    /// As every GOG account call: renewed and saved credentials first. A
    /// cancellation meanwhile stops the call before it starts.
    async fn prepare(&self) -> Result<(), Failure> {
        self.cancelled.store(false, Ordering::Release);
        self.network.prepare().await?;
        let saved = self.saved.clone();
        blocking(Gog::SESSION_FAILED, move || saved()).await?;
        match self.cancelled.load(Ordering::Acquire) {
            true => Err(Failure::Cancelled),
            false => Ok(()),
        }
    }

    /// Saves them once the caller moved on: a completed Transfer's receipt
    /// never waits for Keychain.
    fn save_later(&self) {
        let saved = self.saved.clone();
        tokio::task::spawn_blocking(move || saved());
    }

    fn configuration(&self, receipt: &Receipt) -> Configuration {
        Configuration {
            home: self.locations.home.clone(),
            install: receipt.directory.clone().into(),
            session_directory: self.locations.gog_sessions.clone(),
            helper: self.locations.helper.clone(),
            product_id: receipt.product_id.clone(),
            platform: receipt.platform.clone(),
            prefix: self
                .windows(receipt)
                .map(|windows| windows.prefix().to_owned()),
        }
    }

    /// The build's Wine prefix, when it is a Windows one.
    fn windows(&self, receipt: &Receipt) -> Option<crate::wine::Installation> {
        let game = crate::profile::Game::Gog {
            product_id: &receipt.product_id,
        };
        crate::wine::Installation::of(
            &self.locations,
            game,
            &receipt.platform,
            Path::new(&receipt.directory),
        )
        .ok()
        .flatten()
    }
}

impl<N: Network> installations::Content<Gog> for Content<N> {
    /// Gets Voile for a Windows build unless it is in place, then the
    /// game's files.
    fn download<'a>(&'a self, request: &'a Download) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async move {
            self.runtimes.begin();
            self.prepare().await?;
            let needed = installations::needs_voile(&request.platform).then_some(Runtime::Voile);
            let game = <Gog as installations::Storefront>::transfer_name(request);
            self.runtimes.get(&game, needed.as_slice()).await?;
            let installed = self.network.download(request, &self.language).await;
            self.save_later();
            let installed = installed?;
            Ok(Receipt {
                product_id: installed.product_id,
                name: request.title.clone(),
                directory: installed.directory,
                platform: installed.platform,
                total_bytes: installed.total_bytes,
                needs_verification: Some(false),
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        self.runtimes.progress().or_else(|| self.network.progress())
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.runtimes.cancel();
        self.network.cancel();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        self.network.update_available(receipt)
    }
    fn launch<'a>(
        &'a self,
        receipt: &'a Receipt,
        _: &'a Download,
        admission: Admission<Gog>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            admission
                .hand_off(Handover {
                    network: self.network.clone(),
                    configuration: self.configuration(receipt),
                    locations: self.locations.clone(),
                })
                .await
        })
    }
    /// GOG keeps no Cloud saves for Obelisk yet.
    fn archive<'a>(
        &'a self,
        _: &'a Receipt,
        _: &'a Download,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async { Ok(()) })
    }
    fn removed<'a>(
        &'a self,
        _: &'a Receipt,
        _: &'a Download,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async { Ok(()) })
    }
    fn directory_name(&self, receipt: &Receipt, _: &Download) -> String {
        gog_kit::client::installation_directory_name(&receipt.product_id, &receipt.platform)
    }
    fn remove_prefix(&self, receipt: &Receipt) -> Option<Result<(), String>> {
        let windows = self.windows(receipt)?;
        Some(
            windows
                .remove(&self.locations)
                .map_err(|error| error.to_string()),
        )
    }
    fn maintain<'a>(
        &'a self,
        _: &'a String,
        _: Option<(&'a Receipt, &'a Download)>,
        _: Arc<crate::cloud::Control>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async { Err(failed(Error::Unsupported)) })
    }
    fn indicators(&self, _: &[String]) -> Result<Vec<(String, cloud_status::Indicator)>, Failure> {
        Ok(Vec::new())
    }
}

/// Starting an installed GOG build: its Wine prefix readied, then its
/// supervisor.
struct Handover<N: Network> {
    network: Arc<N>,
    configuration: Configuration,
    locations: Locations,
}

impl<N: Network> sessions::Handover<Gog> for Handover<N> {
    /// The account, and how a Windows build runs.
    type Prepared = (String, Option<crate::wine::Config>);
    fn voile(&self) -> Option<PathBuf> {
        self.configuration
            .prefix
            .is_some()
            .then(|| self.locations.voile.clone())
    }
    fn prepare(&self) -> BoxFuture<'_, Result<Self::Prepared, Failure>> {
        Box::pin(async move {
            let wine = match self.configuration.prefix.is_some() {
                false => None,
                true => {
                    let (configuration, locations) =
                        (self.configuration.clone(), self.locations.clone());
                    Some(
                        blocking(Error::Session.to_string(), move || {
                            super::prepare_windows(&configuration, &locations)
                        })
                        .await?
                        .map_err(failed)?,
                    )
                }
            };
            Ok((self.network.account_id().await?, wine))
        })
    }
    fn start(
        self,
        (account, wine): Self::Prepared,
        cancel: CancelToken,
    ) -> Result<Arc<dyn Supervised<Gog>>, String> {
        super::start(self.configuration, account, wine, cancel)
            .map(|client| Arc::new(client) as Arc<dyn Supervised<Gog>>)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
