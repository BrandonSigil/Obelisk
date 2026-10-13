//! Steam's part of an account's installations: where each installation and
//! its Cloud saves live, and what Steam's network does for them. Only the
//! network is a seam; tests stand in for it.
use super::{Product, Steam, cloud as steam_cloud};
use crate::{
    Locations,
    cloud::Control,
    cloud_status,
    installations::{self, BoxFuture, Receipt, Runtime, Runtimes, Transfer, blocking},
    protocol::Credentials,
    sessions::{self, Admission, Failure, Storefront as _, Supervised, failed},
};
use obelisk_core::{installation::TransferProgress, launch::LaunchTarget, transfer::CancelToken};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use steam_kit::cloud::{CloudListing, CloudMetadata};

/// What an account's installations ask of Steam's network. Every call may
/// wait on it.
pub trait Network: Send + Sync + 'static {
    /// The build a Transfer without a platform installs: the one the
    /// account's licenses grant, the native one first.
    fn platform<'a>(
        &'a self,
        app_id: u32,
        language: &'a str,
    ) -> BoxFuture<'a, Result<String, Failure>>;
    /// Runs `transfer` into the folder it names.
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<steam_kit::client::Installation, Failure>>;
    fn progress(&self) -> Option<TransferProgress>;
    fn cancel(&self);
    /// Whether Steam offers a newer build than the one `receipt` installed.
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
    ) -> BoxFuture<'a, Result<bool, Failure>>;
    /// What starts the game installed in `install`.
    fn launch_target<'a>(
        &'a self,
        app_id: u32,
        platform: &'a str,
        install: &'a Path,
    ) -> BoxFuture<'a, Result<LaunchTarget, Failure>>;
    /// The Cloud rules the saves of `app_id` follow, `None` without Steam Cloud.
    fn archive_rules(
        &self,
        app_id: u32,
    ) -> BoxFuture<'_, Result<Option<(CloudMetadata, CloudListing)>, steam_cloud::Failure>>;
    /// Syncs Cloud saves on the player's request, while `idle` keeps every
    /// Steam launch out. It blocks.
    fn maintain(
        &self,
        target: steam_cloud::MaintenanceTarget,
        credentials: &Credentials,
        control: Arc<Control>,
        idle: &super::IdleGuard,
    ) -> Result<(), steam_cloud::Failure>;
}

impl Network for steam_kit::client::Client {
    fn platform<'a>(
        &'a self,
        app_id: u32,
        language: &'a str,
    ) -> BoxFuture<'a, Result<String, Failure>> {
        Box::pin(async move {
            steam_kit::client::Client::platform(self, app_id, language)
                .await
                .map(str::to_owned)
                .map_err(failed)
        })
    }
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<steam_kit::client::Installation, Failure>> {
        Box::pin(async move {
            steam_kit::client::Client::download(
                self,
                transfer.product.app_id,
                transfer.platform.as_deref(),
                &transfer.product.language,
                &transfer.directory,
            )
            .await
            .map_err(failed)
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        steam_kit::client::Client::progress(self)
    }
    fn cancel(&self) {
        self.cancel_download();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            steam_kit::client::Client::update_available(
                self,
                receipt.product.app_id,
                &receipt.platform,
                &receipt.product.language,
                Path::new(&receipt.directory),
            )
            .await
            .map_err(failed)
        })
    }
    fn launch_target<'a>(
        &'a self,
        app_id: u32,
        platform: &'a str,
        install: &'a Path,
    ) -> BoxFuture<'a, Result<LaunchTarget, Failure>> {
        Box::pin(async move {
            self.launch_plan(app_id, platform, install)
                .await
                .map(|plan| plan.target)
                .map_err(failed)
        })
    }
    fn archive_rules(
        &self,
        app_id: u32,
    ) -> BoxFuture<'_, Result<Option<(CloudMetadata, CloudListing)>, steam_cloud::Failure>> {
        Box::pin(async move {
            self.cloud_archive_metadata(app_id)
                .await
                .map_err(Into::into)
        })
    }
    fn maintain(
        &self,
        target: steam_cloud::MaintenanceTarget,
        credentials: &Credentials,
        control: Arc<Control>,
        idle: &super::IdleGuard,
    ) -> Result<(), steam_cloud::Failure> {
        steam_cloud::maintain(target, credentials, control, idle)
    }
}

/// One signed-in Steam account's installations, as they reach Steam.
pub struct Content<N: Network> {
    network: Arc<N>,
    credentials: Credentials,
    locations: Locations,
    runtimes: Runtimes,
}

impl<N: Network> Content<N> {
    pub fn new(network: Arc<N>, credentials: Credentials, locations: Locations) -> Self {
        Self {
            network,
            credentials,
            runtimes: Runtimes::new(&locations),
            locations,
        }
    }

    fn account(&self) -> u64 {
        self.credentials.steam_id
    }

    /// Where the installation's Cloud saves live: a Windows game's in its
    /// Wine prefix.
    fn cloud(&self, receipt: &Receipt<Steam>) -> steam_cloud::Config {
        steam_cloud::Config {
            root: self.locations.cloud_archive.clone(),
            home: self.locations.home.clone(),
            install: receipt.directory.clone().into(),
            app_id: receipt.product.app_id,
            prefix: self
                .windows(receipt)
                .map(|windows| windows.prefix().to_owned()),
        }
    }

    /// The installation's Wine prefix, when it is a Windows one.
    fn windows(&self, receipt: &Receipt<Steam>) -> Option<crate::wine::Installation> {
        let install = Path::new(&receipt.directory);
        let game = crate::profile::Game::Steam {
            app_id: receipt.product.app_id,
        };
        crate::wine::Installation::of(&self.locations, game, &receipt.platform, install)
            .ok()
            .flatten()
    }
}

/// A Cloud failure, as the player sees it.
fn key(failure: steam_cloud::Failure) -> Failure {
    Failure::Failed(failure.key().into())
}

/// How a Cloud sync the player asked for ended.
fn cloud_failure(failure: steam_cloud::Failure) -> Failure {
    match failure {
        steam_cloud::Failure::Cancelled => Failure::Cancelled,
        failure => Failure::Failed(failure.key().into()),
    }
}

impl<N: Network> installations::Content<Steam> for Content<N> {
    /// Picks the build first, then gets Valve's Steam components, and Voile
    /// for a Windows build, unless they are in place, then the game's files.
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<Receipt<Steam>, Failure>> {
        Box::pin(async move {
            self.runtimes.begin();
            let platform = match &transfer.platform {
                Some(platform) => platform.clone(),
                None => {
                    self.network
                        .platform(transfer.product.app_id, &transfer.product.language)
                        .await?
                }
            };
            let mut needed = vec![Runtime::SteamComponents];
            if installations::needs_voile(&platform) {
                needed.push(Runtime::Voile);
            }
            let game = transfer.progress_name();
            self.runtimes.get(&game, &needed).await?;
            let transfer = Transfer {
                platform: Some(platform),
                ..transfer.clone()
            };
            let installed = self.network.download(&transfer).await?;
            Ok(Receipt {
                product: Product {
                    app_id: installed.app_id,
                    language: installed.language,
                },
                name: installed.name,
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
        self.runtimes.cancel();
        self.network.cancel();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        self.network.update_available(receipt)
    }
    fn launch<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
        admission: Admission<Steam>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let install = PathBuf::from(&receipt.directory);
            let app_id = receipt.product.app_id;
            let target = self
                .network
                .launch_target(app_id, &receipt.platform, &install)
                .await?;
            admission
                .hand_off(Handover {
                    locations: self.locations.clone(),
                    credentials: self.credentials.clone(),
                    app_id,
                    platform: receipt.platform.clone(),
                    install,
                    target,
                })
                .await
        })
    }
    fn archive<'a>(&'a self, receipt: &'a Receipt<Steam>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let rules = self
                .network
                .archive_rules(receipt.product.app_id)
                .await
                .map_err(key)?;
            let Some((metadata, listing)) = rules else {
                return Ok(());
            };
            let config = self.cloud(receipt);
            blocking(crate::Error::Storage.to_string(), move || {
                steam_cloud::archive_with_rules(&config, metadata, listing)
            })
            .await?
            .map_err(key)
        })
    }
    fn removed<'a>(&'a self, receipt: &'a Receipt<Steam>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let (config, account) = (self.cloud(receipt), self.account());
            blocking(Steam::SESSION_FAILED, move || {
                steam_cloud::removed(&config, account)
            })
            .await?
            .map_err(key)
        })
    }
    fn directory_name(&self, receipt: &Receipt<Steam>) -> String {
        format!("steam-{}-{}", receipt.product.app_id, receipt.platform)
    }
    fn remove_prefix(&self, receipt: &Receipt<Steam>) -> Option<Result<(), String>> {
        let windows = self.windows(receipt)?;
        Some(
            windows
                .remove(&self.locations)
                .map_err(|error| error.to_string()),
        )
    }
    fn maintain<'a>(
        &'a self,
        game: &'a u32,
        installed: Option<&'a Receipt<Steam>>,
        control: Arc<Control>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            // An uninstalled game syncs what its saved context names.
            let target = match installed {
                Some(receipt) => steam_cloud::MaintenanceTarget::Installed(self.cloud(receipt)),
                None => steam_cloud::MaintenanceTarget::Uninstalled {
                    root: self.locations.cloud_archive.clone(),
                    app_id: *game,
                },
            };
            let (network, credentials) = (self.network.clone(), self.credentials.clone());
            let locations = self.locations.clone();
            blocking(Steam::SESSION_FAILED, move || {
                // Its idle guard keeps every launch of Steam out meanwhile.
                let idle = super::Client::idle_guard(&locations.steam_sessions, &locations.helper)
                    .map_err(failed)?;
                network
                    .maintain(target, &credentials, control, &idle)
                    .map_err(cloud_failure)
            })
            .await?
        })
    }
    fn indicators(&self, games: &[u32]) -> Result<Vec<(u32, cloud_status::Indicator)>, Failure> {
        steam_cloud::indicators(&self.locations.cloud_archive, self.account(), games).map_err(key)
    }
}

/// Starting an installed Steam game: its launch and Wine prefix readied,
/// then its supervisor, which signs in with the account's credentials.
struct Handover {
    locations: Locations,
    credentials: Credentials,
    app_id: u32,
    platform: String,
    install: PathBuf,
    target: LaunchTarget,
}

impl sessions::Handover<Steam> for Handover {
    type Prepared = super::Request;
    fn voile(&self) -> Option<PathBuf> {
        let game = crate::profile::Game::Steam {
            app_id: self.app_id,
        };
        let windows =
            crate::wine::Installation::of(&self.locations, game, &self.platform, &self.install);
        matches!(windows, Ok(Some(_))).then(|| self.locations.voile.clone())
    }
    /// Every Steam game runs beside Obelisk's Steam host, whatever its
    /// platform or engine.
    fn steam_components(&self) -> Option<PathBuf> {
        Some(self.locations.steam_runtime_package.clone())
    }
    fn prepare(&self) -> BoxFuture<'_, Result<super::Request, Failure>> {
        let (locations, app_id) = (self.locations.clone(), self.app_id);
        let (platform, install, target) = (
            self.platform.clone(),
            self.install.clone(),
            self.target.clone(),
        );
        Box::pin(async move {
            blocking(crate::Error::StartFailed.to_string(), move || {
                super::prepare(&locations, app_id, &platform, &install, &target)
            })
            .await?
            .map_err(|error| match error {
                crate::Error::Cancelled => Failure::Cancelled,
                error => failed(error),
            })
        })
    }
    fn start(
        self,
        request: super::Request,
        cancel: CancelToken,
    ) -> Result<Arc<dyn Supervised<Steam>>, String> {
        let helper = request.config.helpers.join(crate::package::HOST);
        super::start(&helper, &self.credentials, request, &cancel)
            .map(|client| Arc::new(client) as Arc<dyn Supervised<Steam>>)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
