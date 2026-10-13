//! Epic's part of an account's installations: where each build and its Cloud
//! saves live, the credentials every account call renews and saves first, and
//! what Epic's network does for them. Only the network is a seam; tests stand
//! in for it.
use super::{Configuration, Epic, Error, IdleGuard, Product, Store};
use crate::{
    Locations,
    cloud::Control,
    cloud_status,
    installations::{self, BoxFuture, Credentials, Receipt, Transfer, blocking},
    sessions::{self, Admission, BeforeBegin, Failure, Storefront as _, Supervised, failed},
};
use epic_kit::client::Client as EpicClient;
use obelisk_core::{installation::TransferProgress, transfer::CancelToken};
use std::{path::Path, sync::Arc};

/// What an account's installations ask of Epic's network. Every call may wait
/// on it.
pub trait Network: Send + Sync + 'static {
    /// Renews the account's credentials before a long account call.
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>>;
    /// The account the session is signed in to now.
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// A short-lived code the game signs in with.
    fn exchange_code(&self) -> BoxFuture<'_, Result<String, Failure>>;
    /// Runs `transfer` into the folder it names.
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Epic>,
    ) -> BoxFuture<'a, Result<epic_kit::client::Installation, Failure>>;
    fn progress(&self) -> Option<TransferProgress>;
    fn cancel(&self);
    /// Whether Epic's live build differs from the one `receipt` installed.
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Epic>,
    ) -> BoxFuture<'a, Result<bool, Failure>>;
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
        transfer: &'a Transfer<Epic>,
    ) -> BoxFuture<'a, Result<epic_kit::client::Installation, Failure>> {
        Box::pin(async move {
            EpicClient::download(
                self,
                &transfer.product.namespace,
                &transfer.product.catalog_item_id,
                &transfer.product.app_name,
                &transfer.platform,
                &transfer.directory,
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
        receipt: &'a Receipt<Epic>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            EpicClient::update_available(
                self,
                &receipt.product.namespace,
                &receipt.product.catalog_item_id,
                &receipt.product.app_name,
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
            locations,
        }
    }

    /// As every Epic account call: renewed and saved credentials first.
    async fn prepare(&self) -> Result<(), Failure> {
        self.credentials.prepare(self.network.prepare()).await
    }

    /// The build's Cloud configuration: a Windows build keeps its saves in
    /// its Wine prefix.
    fn configuration(&self, receipt: &Receipt<Epic>) -> Configuration {
        Configuration {
            root: self.locations.cloud_archive.clone(),
            home: self.locations.home.clone(),
            install: receipt.directory.clone().into(),
            session_directory: self.locations.epic_sessions.clone(),
            helper: self.locations.helper.clone(),
            namespace: receipt.product.namespace.clone(),
            catalog_item_id: receipt.product.catalog_item_id.clone(),
            app_name: receipt.product.app_name.clone(),
            platform: receipt.platform.clone(),
            language: self.language.clone(),
            title: receipt.name.clone(),
            prefix: receipt
                .windows(&self.locations)
                .ok()
                .flatten()
                .map(|windows| windows.prefix().to_owned()),
        }
    }

    /// What maintenance of `game` syncs: its installation, or, uninstalled,
    /// the configuration its journal saved, with this Obelisk's paths.
    async fn maintenance(
        &self,
        game: &Store,
        installed: Option<&Receipt<Epic>>,
    ) -> Result<Configuration, Failure> {
        if let Some(receipt) = installed {
            return Ok(self.configuration(receipt));
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
    fn begin<'a>(&'a self, transfer: &'a Transfer<Epic>) -> BoxFuture<'a, Result<String, Failure>> {
        Box::pin(async move {
            self.prepare().await?;
            Ok(transfer.platform.clone())
        })
    }
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Epic>,
    ) -> BoxFuture<'a, Result<Receipt<Epic>, Failure>> {
        Box::pin(async move {
            let installed = self.network.download(transfer).await;
            self.credentials.save_later();
            let installed = installed?;
            Ok(Receipt {
                product: Product {
                    namespace: transfer.product.namespace.clone(),
                    catalog_item_id: transfer.product.catalog_item_id.clone(),
                    app_name: installed.app_name,
                },
                name: transfer.name.clone(),
                directory: installed.directory,
                platform: installed.platform,
                total_bytes: installed.total_bytes,
                needs_verification: false,
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        self.network.progress()
    }
    fn cancel(&self) {
        self.credentials.cancel();
        self.network.cancel();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Epic>,
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
        receipt: &'a Receipt<Epic>,
        admission: Admission<Epic>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let outcome = admission
                .hand_off(Handover {
                    network: self.network.clone(),
                    configuration: self.configuration(receipt),
                    windows: receipt.windows(&self.locations).ok().flatten(),
                    locations: self.locations.clone(),
                    saved: self.credentials.saved.clone(),
                })
                .await;
            self.credentials.save_later();
            outcome
        })
    }
    fn archive<'a>(&'a self, receipt: &'a Receipt<Epic>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.prepare().await?;
            let archived = self.network.archive(self.configuration(receipt)).await;
            self.credentials.save_later();
            archived.map_err(failed)
        })
    }
    fn removed<'a>(&'a self, receipt: &'a Receipt<Epic>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let (config, account) = (self.configuration(receipt), self.account.clone());
            blocking(Epic::SESSION_FAILED, move || {
                super::removed(&config, &account)
            })
            .await?
            .map_err(failed)
        })
    }
    fn directory_name(&self, receipt: &Receipt<Epic>) -> String {
        epic_kit::client::installation_directory_name(
            &receipt.product.namespace,
            &receipt.product.catalog_item_id,
            &receipt.product.app_name,
            &receipt.platform,
        )
    }
    fn remove_prefix(&self, receipt: &Receipt<Epic>) -> Option<Result<(), String>> {
        receipt.remove_prefix(&self.locations)
    }
    fn maintain<'a>(
        &'a self,
        game: &'a Store,
        installed: Option<&'a Receipt<Epic>>,
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
    /// Set for a Windows build, whose prefix it readies.
    windows: Option<crate::wine::Installation>,
    locations: Locations,
    saved: Arc<dyn Fn() + Send + Sync>,
}

impl<N: Network> sessions::Handover<Epic> for Handover<N> {
    /// The account, its exchange code, and how a Windows build runs.
    type Prepared = (String, String, Option<crate::wine::Config>);
    fn platform(&self) -> &str {
        &self.configuration.platform
    }
    fn prepare(&self) -> BoxFuture<'_, Result<Self::Prepared, Failure>> {
        Box::pin(async move {
            // A Windows build runs in Wine, readied before any exchange code
            // exists: a prerequisite may take minutes.
            let wine = match &self.windows {
                None => None,
                Some(windows) => {
                    let (windows, configuration, locations) = (
                        windows.clone(),
                        self.configuration.clone(),
                        self.locations.clone(),
                    );
                    Some(
                        blocking(Error::Session.to_string(), move || {
                            super::windows::ready(&windows, &configuration, &locations)
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
