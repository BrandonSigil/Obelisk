//! Epic's part of an account's installations: where each build and its Cloud
//! saves live, the credentials every account call renews and saves first, and
//! what Epic's network does for them. Only the network is a seam; tests stand
//! in for it.
use super::{Configuration, Download, Epic, Error, IdleGuard, Receipt, Store};
use crate::{
    Locations,
    cloud::Control,
    cloud_status,
    installations::{self, BoxFuture, Credentials, Runtime, Runtimes, blocking},
    sessions::{self, Admission, BeforeBegin, Failure, Storefront as _, Supervised, failed},
};
use epic_kit::client::Client as EpicClient;
use obelisk_core::{installation::TransferProgress, transfer::CancelToken};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// What an account's installations ask of Epic's network. Every call may wait
/// on it.
pub trait Network: Send + Sync + 'static {
    /// Renews the account's credentials before a long account call.
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>>;
    /// The account the session is signed in to now.
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// A short-lived code the game signs in with.
    fn exchange_code(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// Runs one Transfer into the folder `request` names.
    fn download<'a>(
        &'a self,
        request: &'a Download,
    ) -> BoxFuture<'a, Result<epic_kit::client::Installation, Failure>>;
    fn progress(&self) -> Option<TransferProgress>;
    fn cancel(&self);
    /// Whether Epic's live build differs from the one `receipt` installed.
    fn update_available<'a>(&'a self, receipt: &'a Receipt)
    -> BoxFuture<'a, Result<bool, Failure>>;
    /// Saves the build's Cloud archive before its files go.
    fn archive(&self, config: Configuration) -> BoxFuture<'_, Result<(), Error>>;
    /// Syncs Cloud saves on the player's request, while `idle` keeps every
    /// Epic launch out.
    fn maintain<'a>(
        self: Arc<Self>,
        config: Configuration,
        control: Arc<Control>,
        idle: &'a IdleGuard,
    ) -> BoxFuture<'a, Result<(), Error>>;
}

impl Network for EpicClient {
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move { EpicClient::prepare(self).await.map(|_| ()).map_err(failed) })
    }
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            self.credentials()
                .await
                .map(|credentials| credentials.account_id)
                .map_err(failed)
        })
    }
    fn exchange_code(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move { EpicClient::exchange_code(self).await.map_err(failed) })
    }
    fn download<'a>(
        &'a self,
        request: &'a Download,
    ) -> BoxFuture<'a, Result<epic_kit::client::Installation, Failure>> {
        Box::pin(async move {
            EpicClient::download(
                self,
                &request.namespace,
                &request.catalog_item_id,
                &request.app_name,
                &request.platform,
                &request.directory,
            )
            .await
            .map_err(failed)
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        EpicClient::progress(self)
    }
    fn cancel(&self) {
        self.cancel_download();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            EpicClient::update_available(
                self,
                &receipt.namespace,
                &receipt.catalog_item_id,
                &receipt.app_name,
                &receipt.platform,
                Path::new(&receipt.directory),
            )
            .await
            .map_err(failed)
        })
    }
    fn archive(&self, config: Configuration) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(super::archive(config, self))
    }
    fn maintain<'a>(
        self: Arc<Self>,
        config: Configuration,
        control: Arc<Control>,
        idle: &'a IdleGuard,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(super::maintain(config, self, control, idle))
    }
}

/// One signed-in Epic account's installations, as they reach Epic.
pub struct Content<N: Network> {
    network: Arc<N>,
    credentials: Credentials,
    account: String,
    /// The language games launch in.
    language: String,
    locations: Locations,
    runtimes: Runtimes,
}

impl<N: Network> Content<N> {
    pub fn new(
        network: Arc<N>,
        saved: Arc<dyn Fn() + Send + Sync>,
        account: String,
        language: String,
        locations: Locations,
    ) -> Self {
        Self {
            network,
            credentials: Credentials::new(saved, Epic::SESSION_FAILED),
            account,
            language,
            runtimes: Runtimes::new(&locations),
            locations,
        }
    }

    /// As every Epic account call: renewed and saved credentials first.
    async fn prepare(&self) -> Result<(), Failure> {
        self.credentials.prepare(self.network.prepare()).await
    }

    /// The build's Cloud configuration: a Windows build keeps its saves in
    /// its Wine prefix.
    fn configuration(&self, receipt: &Receipt, request: &Download) -> Configuration {
        Configuration {
            root: self.locations.cloud_archive.clone(),
            home: self.locations.home.clone(),
            install: receipt.directory.clone().into(),
            session_directory: self.locations.epic_sessions.clone(),
            helper: self.locations.helper.clone(),
            namespace: receipt.namespace.clone(),
            catalog_item_id: request.catalog_item_id.clone(),
            app_name: receipt.app_name.clone(),
            platform: receipt.platform.clone(),
            language: self.language.clone(),
            title: receipt.name.clone(),
            prefix: self
                .windows(receipt)
                .map(|windows| windows.prefix().to_owned()),
        }
    }

    /// The build's Wine prefix, when it is a Windows one.
    fn windows(&self, receipt: &Receipt) -> Option<crate::wine::Installation> {
        let install = Path::new(&receipt.directory);
        super::windows::installation(
            &self.locations,
            &receipt.namespace,
            &receipt.app_name,
            &receipt.platform,
            install,
        )
        .ok()
        .flatten()
    }

    /// What maintenance of `game` syncs: its installation, or, uninstalled,
    /// the configuration its journal saved, with this Obelisk's paths.
    async fn maintenance(
        &self,
        game: &Store,
        installed: Option<(&Receipt, &Download)>,
    ) -> Result<Configuration, Failure> {
        if let Some((receipt, request)) = installed {
            return Ok(self.configuration(receipt, request));
        }
        let (root, account, wanted) = (
            self.locations.cloud_archive.clone(),
            self.account.clone(),
            (game.namespace.clone(), game.app_name.clone()),
        );
        let saved = blocking(Epic::SESSION_FAILED, move || {
            super::indicators(&root, &account, &[wanted])
        })
        .await?
        .map_err(failed)?
        .into_iter()
        .find(|shown| shown.namespace == game.namespace && shown.app_name == game.app_name)
        .and_then(|shown| shown.configuration);
        let mut configuration = saved.ok_or_else(|| failed(Error::Failed))?;
        configuration.root = self.locations.cloud_archive.clone();
        configuration.session_directory = self.locations.epic_sessions.clone();
        configuration.helper = self.locations.helper.clone();
        Ok(configuration)
    }
}

impl<N: Network> installations::Content<Epic> for Content<N> {
    /// Gets Voile for a Windows build unless it is in place, then the
    /// game's files.
    fn download<'a>(&'a self, request: &'a Download) -> BoxFuture<'a, Result<Receipt, Failure>> {
        Box::pin(async move {
            self.runtimes.begin();
            self.prepare().await?;
            let needed = installations::needs_voile(&request.platform).then_some(Runtime::Voile);
            let game = <Epic as installations::Storefront>::transfer_name(request);
            self.runtimes.get(&game, needed.as_slice()).await?;
            let installed = self.network.download(request).await;
            self.credentials.save_later();
            let installed = installed?;
            Ok(Receipt {
                namespace: request.namespace.clone(),
                catalog_item_id: request.catalog_item_id.clone(),
                app_name: installed.app_name,
                name: request.title.clone(),
                directory: installed.directory,
                platform: installed.platform,
                total_bytes: installed.total_bytes,
                needs_verification: false,
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        self.runtimes.progress().or_else(|| self.network.progress())
    }
    fn cancel(&self) {
        self.credentials.cancel();
        self.runtimes.cancel();
        self.network.cancel();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            let available = self.network.update_available(receipt).await;
            // The call may have renewed the account's credentials.
            self.credentials.save_later();
            available
        })
    }
    fn launch<'a>(
        &'a self,
        receipt: &'a Receipt,
        request: &'a Download,
        admission: Admission<Epic>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let outcome = admission
                .hand_off(Handover {
                    network: self.network.clone(),
                    configuration: self.configuration(receipt, request),
                    locations: self.locations.clone(),
                    saved: self.credentials.saved.clone(),
                })
                .await;
            self.credentials.save_later();
            outcome
        })
    }
    fn archive<'a>(
        &'a self,
        receipt: &'a Receipt,
        request: &'a Download,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.prepare().await?;
            let archived = self
                .network
                .archive(self.configuration(receipt, request))
                .await;
            self.credentials.save_later();
            archived.map_err(failed)
        })
    }
    fn removed<'a>(
        &'a self,
        receipt: &'a Receipt,
        request: &'a Download,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let (config, account) = (self.configuration(receipt, request), self.account.clone());
            blocking(Epic::SESSION_FAILED, move || {
                super::removed(&config, &account)
            })
            .await?
            .map_err(failed)
        })
    }
    fn directory_name(&self, receipt: &Receipt, request: &Download) -> String {
        epic_kit::client::installation_directory_name(
            &receipt.namespace,
            &request.catalog_item_id,
            &receipt.app_name,
            &receipt.platform,
        )
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
        game: &'a Store,
        installed: Option<(&'a Receipt, &'a Download)>,
        control: Arc<Control>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let config = self.maintenance(game, installed).await?;
            let (sessions, helper) = (config.session_directory.clone(), config.helper.clone());
            // Its idle guard keeps every launch of Epic out meanwhile.
            let idle = blocking(Epic::SESSION_FAILED, move || {
                super::Client::idle_guard(&sessions, &helper)
            })
            .await?
            .map_err(failed)?;
            match self.network.clone().maintain(config, control, &idle).await {
                Err(Error::Cancelled) => Err(Failure::Cancelled),
                result => result.map_err(failed),
            }
        })
    }
    fn indicators(
        &self,
        games: &[Store],
    ) -> Result<Vec<(Store, cloud_status::Indicator)>, Failure> {
        let games: Vec<_> = games
            .iter()
            .map(|game| (game.namespace.clone(), game.app_name.clone()))
            .collect();
        Ok(
            super::indicators(&self.locations.cloud_archive, &self.account, &games)
                .map_err(failed)?
                .into_iter()
                .map(|shown| {
                    let game = Store {
                        namespace: shown.namespace,
                        app_name: shown.app_name,
                    };
                    (game, shown.indicator)
                })
                .collect(),
        )
    }
}

/// Starting an installed Epic build: its Wine prefix readied, then an
/// exchange code for its account, then its supervisor.
struct Handover<N: Network> {
    network: Arc<N>,
    configuration: Configuration,
    locations: Locations,
    saved: Arc<dyn Fn() + Send + Sync>,
}

impl<N: Network> sessions::Handover<Epic> for Handover<N> {
    /// The account, its exchange code, and how a Windows build runs.
    type Prepared = (String, String, Option<crate::wine::Config>);
    fn voile(&self) -> Option<PathBuf> {
        self.configuration
            .prefix
            .is_some()
            .then(|| self.locations.voile.clone())
    }
    fn prepare(&self) -> BoxFuture<'_, Result<Self::Prepared, Failure>> {
        Box::pin(async move {
            // A Windows build runs in Wine, readied before any exchange code
            // exists: a prerequisite may take minutes.
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
            let account = self.network.account_id().await?;
            let exchange = self.network.exchange_code().await?;
            // The code must belong to the account the game is launched for.
            if self.network.account_id().await? != account {
                return Err(failed(epic_kit::EpicError::Unauthorized));
            }
            Ok((account, exchange, wine))
        })
    }
    /// The exchange rotated the saved credentials, before the game starts.
    fn before_begin(&self) -> BeforeBegin {
        let saved = self.saved.clone();
        Box::new(move || saved())
    }
    fn start(
        self,
        (account, exchange, wine): Self::Prepared,
        cancel: CancelToken,
    ) -> Result<Arc<dyn Supervised<Epic>>, String> {
        super::start(self.configuration, account, exchange, wine, cancel)
            .map(|client| Arc::new(client) as Arc<dyn Supervised<Epic>>)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
