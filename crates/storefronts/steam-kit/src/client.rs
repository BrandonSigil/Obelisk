//! Account-scoped orchestration. Authentication, CM messages and content formats
//! remain independently usable; no UI or platform keychain code lives here.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use futures_util::{StreamExt, TryStreamExt, future::BoxFuture};
use obelisk_core::{
    installation::{self, Destination, InstallFailure, InstalledFiles, TransferError},
    mac_build,
    transfer::{
        self, BodyError, BodySize, Cancelled, DownloadProgress, Picking, TransferBusy,
        TransferContext, Transfers,
    },
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    chat::{self, ChatEventSource},
    cm::{
        AppInfo, CONCURRENT_REQUESTS, ChatMode, ContentLicenses, ContentServer, DepotInfo,
        SteamConnection, supports_platform,
    },
    content::{
        CancelToken, CdnAuthorization, ContentClient, ContentError, DepotDownload,
        DepotInstallation, DepotSource,
    },
    invites::{InviteSource, ReceivedInvites},
    own_game::{GameInvite, OwnGame, OwnGameSource},
};
mod cloud;

pub use obelisk_core::{installation::TransferProgress, transfer::TransferState};

/// Opening a game and then downloading it should not pay two PICS token and
/// product round trips; publisher metadata rarely changes within a minute.
const APP_INFO_TTL: Duration = Duration::from_secs(60);
/// Valve's web chat tells a friend about typing at most this often.
const TYPING_INTERVAL: Duration = Duration::from_secs(10);

pub use crate::cm::OwnedGameInfo as OwnedGame;
pub use crate::cm::achievements::Achievement;
pub use crate::cm::{GameInviteKind, History, HistoryMessage, MessageSession, SentMessage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamePlatforms {
    pub app_id: u32,
    pub platforms: Vec<String>,
    /// The game's whole PICS app info, when PICS returned the game.
    pub pics: Option<serde_json::Value>,
}

/// A message key for the host. A stopped download keeps its own variant so
/// its Transfer ends cancelled rather than failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Failure(String),
    #[error("{0}")]
    Cancelled(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientConnectError {
    #[error("{0}")]
    AuthenticationRequired(ClientError),
    #[error("{0}")]
    Other(ClientError),
}

impl From<crate::cm::CmError> for ClientConnectError {
    fn from(error: crate::cm::CmError) -> Self {
        if error.authentication_required() {
            Self::AuthenticationRequired(error.into())
        } else {
            Self::Other(error.into())
        }
    }
}

pub fn http_client() -> Result<reqwest::Client, ClientError> {
    reqwest::Client::builder()
        .user_agent(concat!("Obelisk/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        // A depot download opens connections to many CDN servers; those it
        // stops using close soon rather than hold descriptors.
        .pool_idle_timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| ClientError::Failure("backend.steam.client.connectionInitFailed".into()))
}

/// Reads a response body while refusing anything above `limit` bytes, whether
/// advertised up front or discovered while streaming. Nothing cancels it, so
/// it fails only with `Network` or `Size`.
pub(crate) async fn read_bounded(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    transfer::read_body(
        response.content_length(),
        response.bytes_stream(),
        BodySize::AtMost(limit as u64),
        &CancelToken::new(),
        |_| {},
    )
    .await
}

#[derive(Clone, Debug)]
pub struct GameDetails {
    pub app_id: u32,
    pub name: String,
    pub windows: bool,
    /// Whether Obelisk installs the Mac build: the account can install it,
    /// and its game runs natively on Apple silicon or the game has no Windows
    /// build. It stays the Mac build when that cannot be told.
    pub macos: bool,
    pub windows_bytes: u64,
    pub macos_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    pub app_id: u32,
    pub name: String,
    pub directory: String,
    pub platform: String,
    pub language: String,
    pub total_bytes: u64,
    pub manifests: Vec<InstalledDepot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledDepot {
    pub depot_id: u32,
    pub manifest_id: u64,
}

// Intentionally no Debug: depot keys are credentials.
/// Steamworks Common Redistributables, free to every account. Windows games
/// name its depots, such as Visual C++ or DirectX, for Steam to install and run
/// before their first launch.
const COMMON_REDISTRIBUTABLES: u32 = 228_980;
/// Its depots a download leaves out: .NET Framework and XNA, which needs it,
/// 229000 to 229012. Their installers do not work in Wine, and .NET games run
/// on Wine Mono instead. The others, such as Visual C++, install in the prefix
/// as Steam installs them under Proton.
const SKIPPED_REDISTRIBUTABLES: std::ops::RangeInclusive<u32> = 229_000..=229_012;

/// One Mac build of an app: the app, and the manifest of each of its depots.
type MacBuild = (u32, Vec<(u32, u64)>);

struct PreparedDepot {
    depot_id: u32,
    manifest_id: u64,
    authorizing_app: u32,
    key: [u8; 32],
    estimated_bytes: u64,
}

/// Everything resolved over the CM before the installation lock is taken.
struct PreparedInstall {
    name: String,
    /// Validated HTTPS CDN/SteamCache hosts allowed to serve this app, in
    /// Steam's order of preference.
    servers: Vec<ContentServer>,
    depots: Vec<PreparedDepot>,
}

/// Counters accumulated by depots already installed in this invocation, so
/// per-depot engine progress can be presented as one transfer.
#[derive(Clone, Copy)]
struct ProgressBase {
    completed: u64,
    total_estimate: u64,
    network: u64,
    content_bytes: u64,
}

pub struct Client {
    http: reqwest::Client,
    cm: SteamConnection,
    friends: crate::friends::FriendsEventSource,
    chat: ChatEventSource,
    own_game: OwnGameSource,
    invites: InviteSource,
    /// When each friend last heard that the account is typing.
    typing: Mutex<HashMap<u64, Instant>>,
    connection_abort: tokio::task::AbortHandle,
    cloud_authorized: std::sync::atomic::AtomicBool,
    /// App and depot IDs expanded through PICS licenses, filled on first content query.
    catalog: AsyncMutex<Option<Arc<ContentLicenses>>>,
    app_infos: Mutex<HashMap<u32, (Instant, Arc<AppInfo>)>>,
    /// Whether each Mac build's game runs natively on Apple silicon.
    native_builds: Mutex<HashMap<MacBuild, bool>>,
    downloads: Transfers<TransferProgress>,
    picking: Picking,
    /// The session's own token, to ask Steam for web access tokens.
    refresh_token: String,
}

impl Client {
    /// Reads the connected account's achievements without initializing a game SDK.
    /// Other requests may run between the two reads; a schema change in that
    /// window is already rejected by the join.
    pub async fn achievements(
        &self,
        app_id: u32,
        language: &str,
    ) -> Result<Vec<Achievement>, ClientError> {
        // A game without achievements needs no request of the player's.
        let Some(schema) = self
            .cm
            .game_achievements(app_id, language)
            .await?
            .filter(|schema| !schema.is_empty())
        else {
            return Ok(Vec::new());
        };
        let user = self.cm.user_achievements(app_id).await?;
        crate::cm::achievements::combine(app_id, schema, user).map_err(Into::into)
    }

    /// When the account got each app its licenses grant, in seconds since
    /// the Unix epoch. It expands the licenses through PICS, as a download
    /// does, once until the library loads again.
    pub async fn acquisitions(&self) -> Result<BTreeMap<u32, u32>, ClientError> {
        Ok(self.licensed_content().await?.acquired.clone())
    }
    #[cfg(test)]
    pub(crate) fn with_test_connection(cm: SteamConnection) -> Self {
        Self::from_connection(http_client().unwrap(), cm, String::new())
    }

    fn from_connection(http: reqwest::Client, cm: SteamConnection, refresh_token: String) -> Self {
        Self {
            http,
            friends: cm.friends_source(),
            chat: cm.chat_source(),
            own_game: cm.own_game_source(),
            invites: cm.invite_source(),
            typing: Mutex::new(HashMap::new()),
            connection_abort: cm.abort_handle(),
            cloud_authorized: std::sync::atomic::AtomicBool::new(true),
            cm,
            catalog: AsyncMutex::new(None),
            app_infos: Mutex::new(HashMap::new()),
            native_builds: Mutex::new(HashMap::new()),
            downloads: Transfers::default(),
            picking: Picking::default(),
            refresh_token,
        }
    }

    pub async fn connect(
        account_name: &str,
        steam_id: u64,
        refresh_token: &str,
    ) -> Result<Self, ClientConnectError> {
        let http = http_client().map_err(ClientConnectError::Other)?;
        let mut cm = SteamConnection::connect(http.clone())
            .await
            .map_err(ClientConnectError::from)?;
        // Only this UI session joins the friends chat and shows the account
        // Online; Cloud sessions log on without either.
        cm.log_on(account_name, steam_id, refresh_token, ChatMode::Enabled)
            .await
            .map_err(ClientConnectError::from)?;
        cm.set_online().await.map_err(ClientConnectError::from)?;
        Ok(Self::from_connection(http, cm, refresh_token.into()))
    }

    /// Cookies that sign Steam's sites in as the account, through a web
    /// access token this session asks for.
    pub async fn web_cookies(&self) -> Result<Vec<crate::auth::WebCookie>, ClientError> {
        let (steam_id, token) = self.cm.web_access_token(&self.refresh_token).await?;
        if crate::auth::steam_id_from_refresh_token(&token).ok() != Some(steam_id) {
            return Err(
                crate::cm::CmError::Protocol("backend.steam.cm.invalidSession".into()).into(),
            );
        }
        Ok(crate::auth::web_cookies(steam_id, &token))
    }

    pub async fn next_friends(&self, after_revision: u64) -> crate::friends::FriendsSnapshot {
        // Publish presence immediately. Optional title lookup must never hold up
        // a new status, game exit or disconnect, even while content owns CM.
        tokio::select! {
            biased;
            snapshot = self.friends.next(after_revision) => snapshot,
            () = self.resolve_friend_game_names() => self.friends.next(after_revision).await,
        }
    }

    /// One pass over the currently unknown titles: apps that appear while the
    /// lookup is in flight are picked up by the next call. An unresolved app
    /// caches as `None`, so a failed lookup is retried only on a later event.
    async fn resolve_friend_game_names(&self) {
        let ids = self.friends.missing_game_names();
        if ids.is_empty() {
            return;
        }
        let account_id = self.friends.account_id();
        // Keep presence usable; a lookup failure must not stall the snapshot.
        let Ok(apps) = self.cached_app_infos(&ids).await else {
            return;
        };
        let names: Vec<_> = ids
            .iter()
            .map(|id| (*id, apps.get(id).map(|app| app.name.clone())))
            .collect();
        self.friends.resolve_game_names(account_id, &names);
    }

    /// Shows the account Online again, for instance after a game's own Steam
    /// session changed its status.
    pub async fn set_online(&self) -> Result<(), ClientError> {
        self.cm.set_online().await.map_err(Into::into)
    }

    /// Chat events after `after_sequence`; waits until there are some or the
    /// session ends.
    pub async fn next_chat(&self, after_sequence: u64) -> chat::ChatEvents {
        self.chat.next(after_sequence).await
    }

    pub async fn send_message(&self, friend: u64, text: &str) -> Result<SentMessage, ClientError> {
        let sent = self.cm.send_friend_message(friend, text).await?;
        // The message ends the typing; the next keystroke may say so again.
        self.typing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&friend);
        Ok(sent)
    }

    /// Tells `friend` the account is typing, at most once per interval;
    /// calls in between do nothing.
    pub async fn send_typing(&self, friend: u64) -> Result<(), ClientError> {
        {
            let now = Instant::now();
            let mut typing = self.typing.lock().unwrap_or_else(PoisonError::into_inner);
            typing.retain(|_, sent| now.duration_since(*sent) < TYPING_INTERVAL);
            if typing.contains_key(&friend) {
                return Ok(());
            }
            typing.insert(friend, now);
        }
        let sent = self.cm.send_typing(friend).await;
        if sent.is_err() {
            self.typing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&friend);
        }
        sent.map_err(Into::into)
    }

    /// Up to `count` (at most 50) messages before `before`, a (timestamp,
    /// ordinal) position, or the latest ones; oldest first.
    pub async fn recent_messages(
        &self,
        friend: u64,
        before: Option<(u32, u32)>,
        count: u32,
    ) -> Result<History, ClientError> {
        self.cm
            .recent_messages(friend, before, count)
            .await
            .map_err(Into::into)
    }

    /// Conversations with messages, with their unread counts.
    pub async fn active_sessions(&self) -> Result<Vec<MessageSession>, ClientError> {
        self.cm.active_sessions().await.map_err(Into::into)
    }

    pub async fn ack(&self, friend: u64, timestamp: u32) -> Result<(), ClientError> {
        self.cm.ack(friend, timestamp).await.map_err(Into::into)
    }

    /// The invitations friends sent, once they differ from `after_revision`
    /// or the session ends.
    pub async fn next_invites(&self, after_revision: u64) -> ReceivedInvites {
        self.invites.next(after_revision).await
    }

    pub async fn next_own_game(&self, after_revision: u64) -> OwnGame {
        self.own_game.next(after_revision).await
    }

    /// Invites `friend` into the account's current game, `app_id`, the way
    /// Steam's friend menu does, and only when that menu would offer it.
    pub async fn invite_to_game(
        &self,
        friend: u64,
        app_id: u32,
    ) -> Result<GameInviteKind, ClientError> {
        let game = self.own_game.snapshot();
        if !game.connected {
            return Err(crate::cm::CmError::Disconnected.into());
        }
        if app_id == 0 || game.app_id != app_id {
            return Err(crate::cm::SocialError::InviteUnavailable.into());
        }
        self.cm
            .invite(friend, app_id, &game.invite())
            .await
            .map_err(Into::into)
    }

    /// Invites `friend` into `invite`, which the account's running game of
    /// `app_id` named itself, such as the lobby its own Invite button asked
    /// Steam's overlay for. Steam may never show this session a private
    /// lobby, so only a persona already showing another game refuses it.
    pub async fn invite_into_game(
        &self,
        friend: u64,
        app_id: u32,
        invite: &GameInvite,
    ) -> Result<GameInviteKind, ClientError> {
        let game = self.own_game.snapshot();
        if !game.connected {
            return Err(crate::cm::CmError::Disconnected.into());
        }
        if app_id == 0 || !invite.sendable() || (game.app_id != 0 && game.app_id != app_id) {
            return Err(crate::cm::SocialError::InviteUnavailable.into());
        }
        self.cm
            .invite(friend, app_id, invite)
            .await
            .map_err(Into::into)
    }

    /// Works even while a download owns the CM query mutex. Observers wake
    /// before the worker is aborted, and the old socket cannot keep reconnecting.
    pub fn shutdown(&self) {
        self.cloud_authorized
            .store(false, std::sync::atomic::Ordering::Release);
        self.friends.disconnect(false);
        self.chat.disconnect();
        self.own_game.disconnect();
        self.invites.disconnect();
        self.connection_abort.abort();
        self.downloads.cancel();
    }

    /// Whether its CM connection ended. A request waiting on the connection
    /// reads the end only once this is true.
    pub fn ended(&self) -> bool {
        !self.cm.logged_on()
    }

    /// The account's name and avatar, as Steam pushes them once it is online.
    pub async fn profile(&self) -> Result<crate::cm::AccountProfile, ClientError> {
        match tokio::time::timeout(crate::cm::PROFILE_TIMEOUT, self.cm.profile()).await {
            Ok(profile) => profile.map_err(Into::into),
            Err(_) => Err(crate::cm::CmError::Timeout.into()),
        }
    }

    /// Ordered by the CM layer: most recently played first, then name.
    pub async fn games(&self) -> Result<Vec<OwnedGame>, ClientError> {
        let games = self.cm.owned_games().await?;
        // Rendering the library does not require expanding every license through
        // PICS. Refresh license-derived download rights only when next needed.
        *self.catalog.lock().await = None;
        self.app_infos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(games)
    }

    /// Serves PICS metadata from a short-lived cache.
    async fn app_info(&self, app_id: u32) -> Result<Arc<AppInfo>, ClientError> {
        if let Some(app) = self.cached_app(app_id) {
            return Ok(app);
        }
        let app = Arc::new(self.cm.app_info(app_id).await?);
        self.remember_app(&app);
        Ok(app)
    }

    /// PICS metadata for many apps, served from the short-lived cache and
    /// fetched in bounded batches. Apps PICS does not return are absent.
    async fn cached_app_infos(
        &self,
        app_ids: &[u32],
    ) -> Result<HashMap<u32, Arc<AppInfo>>, ClientError> {
        let mut apps = HashMap::with_capacity(app_ids.len());
        let mut missing = HashSet::new();
        for id in app_ids {
            if let Some(app) = self.cached_app(*id) {
                apps.insert(*id, app);
            } else {
                missing.insert(*id);
            }
        }
        if missing.is_empty() {
            return Ok(apps);
        }
        let requested: Vec<_> = app_ids
            .iter()
            .copied()
            .filter(|id| missing.contains(id))
            .collect();
        for app in self.cm.app_infos(&requested).await? {
            if missing.contains(&app.app_id) {
                let app = Arc::new(app);
                self.remember_app(&app);
                apps.insert(app.app_id, app);
            }
        }
        Ok(apps)
    }

    fn cached_app(&self, app_id: u32) -> Option<Arc<AppInfo>> {
        self.app_infos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&app_id)
            .filter(|(fetched, _)| fetched.elapsed() < APP_INFO_TTL)
            .map(|(_, app)| Arc::clone(app))
    }

    fn remember_app(&self, app: &Arc<AppInfo>) {
        self.app_infos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(app.app_id, (Instant::now(), Arc::clone(app)));
    }

    /// One caller expands the licenses; the others wait for its catalog.
    /// Player.GetOwnedGames is presentation data, not proof of depot/DLC rights.
    async fn licensed_content(&self) -> Result<Arc<ContentLicenses>, ClientError> {
        let mut catalog = self.catalog.lock().await;
        if let Some(owned) = catalog.as_ref() {
            return Ok(Arc::clone(owned));
        }
        let owned = Arc::new(self.cm.content_licenses().await?);
        *catalog = Some(Arc::clone(&owned));
        Ok(owned)
    }

    /// The account's licenses and the PICS metadata of `app_id`, which the
    /// account must own.
    async fn owned_app(
        &self,
        app_id: u32,
    ) -> Result<(Arc<ContentLicenses>, Arc<AppInfo>), ClientError> {
        let owned = self.licensed_content().await?;
        if !owned.apps.contains(&app_id) {
            return Err(ClientError::Failure(
                "backend.steam.client.gameNotOwned".into(),
            ));
        }
        let app = self.app_info(app_id).await?;
        Ok((owned, app))
    }

    pub fn progress(&self) -> Option<TransferProgress> {
        self.downloads.snapshot()
    }

    /// Declared platforms in bounded PICS batches, without expanding licenses.
    /// Returns each nonzero ID once, in request order. Empty means unknown;
    /// transport, access and malformed metadata errors remain request failures.
    pub async fn platforms(&self, app_ids: &[u32]) -> Result<Vec<GamePlatforms>, ClientError> {
        let mut seen = HashSet::new();
        let app_ids: Vec<_> = app_ids
            .iter()
            .copied()
            .filter(|id| *id != 0 && seen.insert(*id))
            .collect();
        let apps = self.cached_app_infos(&app_ids).await?;
        Ok(app_ids
            .into_iter()
            .map(|app_id| GamePlatforms {
                app_id,
                platforms: apps
                    .get(&app_id)
                    .map_or_else(Vec::new, |app| app.os_list.clone()),
                pics: apps.get(&app_id).map(|app| app.metadata()),
            })
            .collect())
    }

    pub async fn details(&self, app_id: u32, language: &str) -> Result<GameDetails, ClientError> {
        let owned = self.licensed_content().await?;
        let app = self.app_info(app_id).await?;
        let windows = select_depots(&app, &owned, "windows", language);
        let macos = select_depots(&app, &owned, "macos", language);
        // Only a game with both builds needs its Mac build's files read: a
        // Mac-only one's download says why it cannot be installed.
        let native = macos.is_empty()
            || windows.is_empty()
            || self
                .runs_natively(&app, &owned, language, &CancelToken::new())
                .await
                .unwrap_or(true);
        Ok(GameDetails {
            app_id,
            name: app.name.clone(),
            windows: !windows.is_empty(),
            macos: !macos.is_empty() && native,
            windows_bytes: windows.iter().filter_map(|d| d.size_bytes).sum(),
            macos_bytes: macos.iter().filter_map(|d| d.size_bytes).sum(),
        })
    }

    pub fn cancel_download(&self) {
        self.picking.cancel();
        self.downloads.cancel();
    }

    /// Resolve only a completed installation belonging to this account. PICS is
    /// fetched again so executable paths and launch arguments follow the publisher.
    pub async fn launch_plan(
        &self,
        app_id: u32,
        platform: &str,
        directory: &Path,
    ) -> Result<crate::launch::LaunchPlan, ClientError> {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(ClientError::Failure(
                "backend.steam.launch.directoryMissing".into(),
            ));
        }
        // No Transfer of this installation can start until the plan returns.
        let installed = installation::open::<Installation>(directory)?;
        let receipt = installed
            .receipt
            .as_ref()
            .filter(|receipt| receipt.app_id == app_id && receipt.platform == platform)
            .ok_or_else(|| {
                ClientError::Failure("backend.steam.client.incompleteInstallation".into())
            })?;
        let owned = self.licensed_content().await?;
        if !owned.apps.contains(&receipt.app_id) {
            return Err(ClientError::Failure(
                "backend.steam.client.gameNotOwned".into(),
            ));
        }
        let app = Arc::new(self.cm.app_info(app_id).await?);
        self.remember_app(&app);
        crate::launch::plan(&app, platform, directory).map_err(Into::into)
    }

    /// The build `download` installs when it is given no platform: of those
    /// the account's licenses grant, the Mac build when its game runs
    /// natively on Apple silicon, else the Windows build. It asks Steam for
    /// the licenses, then reads a few of the Mac build's files;
    /// `cancel_download` stops it.
    pub async fn platform(&self, app_id: u32, language: &str) -> Result<&'static str, ClientError> {
        if !crate::language::is_supported(language) {
            return Err(ClientError::Failure(
                "backend.steam.client.unsupportedPlatformOrLanguage".into(),
            ));
        }
        self.picking
            .run(async |picking| {
                picking
                    .race(self.pick_platform(app_id, language, picking))
                    .await
            })
            .await?
    }

    /// Installs the game's build for `platform` under `parent`. Without a
    /// platform, it installs the build `platform` picks.
    pub async fn download(
        &self,
        app_id: u32,
        platform: Option<&str>,
        language: &str,
        parent: &Path,
    ) -> Result<Installation, ClientError> {
        self.picking
            .run(async |picking| {
                self.install_build(app_id, platform, language, parent, picking)
                    .await
            })
            .await
    }

    /// `download`, which `picking` cancels until its Transfer starts.
    async fn install_build(
        &self,
        app_id: u32,
        platform: Option<&str>,
        language: &str,
        parent: &Path,
        picking: &CancelToken,
    ) -> Result<Installation, ClientError> {
        let unsupported =
            || ClientError::Failure("backend.steam.client.unsupportedPlatformOrLanguage".into());
        if !crate::language::is_supported(language) {
            return Err(unsupported());
        }
        let platform = match platform {
            Some(platform) => platform,
            None => {
                picking
                    .race(self.pick_platform(app_id, language, picking))
                    .await??
            }
        };
        if !matches!(platform, "windows" | "macos") {
            return Err(unsupported());
        }
        // The chosen folder is the only caller-controlled path. App IDs and depot
        // manifest validation determine every path beneath it.
        let directory = format!("steam-{app_id}-{platform}");
        installation::install(
            &self.downloads,
            app_id.to_string(),
            Destination {
                parent,
                directory: &directory,
            },
            |_, cancel| async move {
                // A cancellation that arrived while the build was picked.
                picking.check()?;
                self.prepare_depots(app_id, platform, language, &cancel)
                    .await
            },
            |prepared, destination, transfer| async move {
                self.install_depots(
                    app_id,
                    platform,
                    language,
                    prepared,
                    &destination,
                    &transfer,
                )
                .await
            },
        )
        .await
    }

    /// The build `platform` names.
    async fn pick_platform(
        &self,
        app_id: u32,
        language: &str,
        cancel: &CancelToken,
    ) -> Result<&'static str, ClientError> {
        let (owned, app) = self.owned_app(app_id).await?;
        let (macos, windows) = installable_builds(&app, &owned, language);
        if macos && self.runs_natively(&app, &owned, language, cancel).await? {
            return Ok("macos");
        }
        match (macos, windows) {
            (_, true) => Ok("windows"),
            (true, false) => Err(ClientError::Failure(
                "backend.install.macBuildNotNative".into(),
            )),
            (false, false) => Err(ClientError::Failure(
                "backend.steam.client.noCompatibleDepot".into(),
            )),
        }
    }

    /// Whether the game of the Mac build the account can install runs
    /// natively on Apple silicon, as a launch would start it: its program and
    /// its Steam API library hold arm64 slices. It reads their first bytes
    /// from the build's depots, once per build.
    async fn runs_natively(
        &self,
        app: &AppInfo,
        owned: &ContentLicenses,
        language: &str,
        cancel: &CancelToken,
    ) -> Result<bool, ClientError> {
        let Some(executable) =
            crate::launch::configuration(app, "macos").map(|config| config.executable.clone())
        else {
            return Ok(false);
        };
        let selected = select_depots(app, owned, "macos", language);
        let sources = self
            .cached_app_infos(&shared_sources(app, &selected))
            .await?;
        let mut manifests = Vec::new();
        for depot in &selected {
            if let Some(manifest_id) = public_manifest(app, depot, &sources)? {
                manifests.push((depot.depot_id, manifest_id));
            }
        }
        let build = (app.app_id, manifests);
        if let Some(native) = self.lock_native_builds().get(&build) {
            return Ok(*native);
        }
        let servers = self.content_servers(app.app_id).await?;
        let mut depots = Vec::new();
        for depot in &selected {
            if let Some(prepared) = self.prepare_depot(app, depot, &sources, cancel).await? {
                let request_code = self
                    .cm
                    .manifest_request_code(
                        prepared.depot_id,
                        prepared.authorizing_app,
                        prepared.manifest_id,
                    )
                    .await?;
                depots.push((prepared, request_code));
            }
        }
        let authorizations: Vec<_> = depots
            .iter()
            .map(|(depot, _)| DepotAuthorization {
                cm: &self.cm,
                app_id: depot.authorizing_app,
                depot_id: depot.depot_id,
            })
            .collect();
        let content = ContentClient::new(self.http.clone());
        let mut files = Vec::new();
        let mut listing = mac_build::Listing::default();
        for ((depot, request_code), authorization) in depots.iter().zip(&authorizations) {
            let source = DepotSource {
                depot_id: depot.depot_id,
                manifest_id: depot.manifest_id,
                request_code: *request_code,
                depot_key: depot.key,
                servers: &servers,
                authorization: Some(authorization),
            };
            let depot_files = content.depot_files(source, cancel).await?;
            depot_files.list(&mut listing, files.len());
            files.push(depot_files);
        }
        let files = &files;
        let read =
            move |(depot, file): (usize, usize), bytes: u64| files[depot].read(file, bytes, cancel);
        let native =
            mac_build::runs_natively(&listing, &executable, &["libsteam_api.dylib"], read).await?;
        self.lock_native_builds().insert(build, native);
        Ok(native)
    }

    fn lock_native_builds(&self) -> std::sync::MutexGuard<'_, HashMap<MacBuild, bool>> {
        self.native_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The validated HTTPS CDN and SteamCache hosts allowed to serve the app.
    /// Every one is kept: Steam's first servers are often its busiest, and a
    /// depot spreads over the fastest ones it measures.
    async fn content_servers(&self, app_id: u32) -> Result<Vec<ContentServer>, ClientError> {
        let servers: Vec<ContentServer> = self
            .cm
            .content_servers()
            .await?
            .into_iter()
            .filter(|server| {
                server.allowed_app_ids.is_empty() || server.allowed_app_ids.contains(&app_id)
            })
            .collect();
        if servers.is_empty() {
            return Err(ClientError::Failure(
                "backend.steam.client.noContentServer".into(),
            ));
        }
        Ok(servers)
    }

    /// Whether Steam offers a newer build than the one installed in
    /// `directory`: the public manifests of the depots a download would pick
    /// differ from those it installed. An installation whose receipt is gone
    /// needs its Transfer too. It reads PICS only, never the game's files.
    pub async fn update_available(
        &self,
        app_id: u32,
        platform: &str,
        language: &str,
        directory: &Path,
    ) -> Result<bool, ClientError> {
        let Some(installed) = installation::read_receipt::<Installation>(directory)?
            .filter(|receipt| receipt.app_id == app_id && receipt.platform == platform)
        else {
            return Ok(true);
        };
        let owned = self.licensed_content().await?;
        let app = self.app_info(app_id).await?;
        let selected = select_depots(&app, &owned, platform, language);
        let sources = self
            .cached_app_infos(&shared_sources(&app, &selected))
            .await?;
        let mut latest = HashSet::new();
        for depot in selected {
            if let Some(manifest_id) = public_manifest(&app, depot, &sources)? {
                latest.insert((depot.depot_id, manifest_id));
            }
        }
        // A build Steam would not install is no update.
        if latest.is_empty() {
            return Ok(false);
        }
        let installed: HashSet<_> = installed
            .manifests
            .iter()
            .map(|depot| (depot.depot_id, depot.manifest_id))
            .collect();
        Ok(latest != installed)
    }

    /// Installs every prepared depot under the installation lock, including
    /// waits for refreshed request codes.
    async fn install_depots(
        &self,
        app_id: u32,
        platform: &str,
        language: &str,
        prepared: PreparedInstall,
        destination: &Path,
        transfer: &TransferContext<TransferProgress>,
    ) -> Result<InstalledFiles<Installation>, ClientError> {
        let cancel = transfer.cancel_token();
        let mut base = ProgressBase {
            completed: 0,
            total_estimate: prepared
                .depots
                .iter()
                .map(|depot| depot.estimated_bytes)
                .sum(),
            network: 0,
            content_bytes: 0,
        };
        let mut manifests = Vec::new();
        let mut scripts = Vec::new();
        let content = ContentClient::new(self.http.clone());
        for depot in &prepared.depots {
            cancel.check()?;
            let DepotInstallation {
                summary,
                install_scripts,
            } = self
                .install_depot(
                    &content,
                    depot,
                    &prepared.servers,
                    destination,
                    transfer,
                    base,
                )
                .await?;
            base.total_estimate =
                base.total_estimate.saturating_sub(depot.estimated_bytes) + summary.total_bytes;
            base.completed += summary.total_bytes;
            base.network += summary.downloaded_bytes;
            base.content_bytes += summary.content_bytes;
            manifests.push(InstalledDepot {
                depot_id: depot.depot_id,
                manifest_id: depot.manifest_id,
            });
            scripts.extend(install_scripts);
        }
        scripts.sort();
        obelisk_core::install::write_cached_file(
            destination,
            Path::new(crate::install_script::LIST),
            scripts.join("\n").as_bytes(),
        )
        .map_err(ContentError::from)?;
        Ok(InstalledFiles {
            receipt: Installation {
                app_id,
                name: prepared.name,
                directory: destination.to_string_lossy().into_owned(),
                platform: platform.into(),
                language: language.into(),
                total_bytes: base.completed,
                manifests,
            },
            total_bytes: base.completed,
            network_bytes: base.network,
            content_bytes: base.content_bytes,
        })
    }

    /// Resolves ownership, depot selection, manifest IDs, depot keys and CDN
    /// candidates over the CM, before any installation file exists.
    async fn prepare_depots(
        &self,
        app_id: u32,
        platform: &str,
        language: &str,
        cancel: &CancelToken,
    ) -> Result<PreparedInstall, ClientError> {
        let (owned, app) = self.owned_app(app_id).await?;
        let selected = select_depots(&app, &owned, platform, language);
        if selected.is_empty() {
            return Err(ClientError::Failure(
                "backend.steam.client.noCompatibleDepot".into(),
            ));
        }
        let servers = self.content_servers(app_id);
        let depots = async {
            cancel.check()?;
            let sources = self
                .cached_app_infos(&shared_sources(&app, &selected))
                .await?;
            let lookups: Vec<_> = selected
                .iter()
                .map(|depot| self.prepare_depot(&app, depot, &sources, cancel))
                .collect();
            let depots: Vec<Option<PreparedDepot>> = futures_util::stream::iter(lookups)
                .buffered(CONCURRENT_REQUESTS)
                .try_collect()
                .await?;
            let depots: Vec<_> = depots.into_iter().flatten().collect();
            if depots.is_empty() {
                return Err(ClientError::Failure(
                    "backend.steam.client.noPublicManifest".into(),
                ));
            }
            Ok(depots)
        };
        let (servers, depots) = tokio::try_join!(servers, depots)?;
        Ok(PreparedInstall {
            name: app.name.clone(),
            servers,
            depots,
        })
    }

    /// One selected depot's manifest ID and key, or `None` when it has no
    /// public manifest. `sources` holds the apps shared depots come from.
    async fn prepare_depot(
        &self,
        app: &AppInfo,
        depot: &DepotInfo,
        sources: &HashMap<u32, Arc<AppInfo>>,
        cancel: &CancelToken,
    ) -> Result<Option<PreparedDepot>, ClientError> {
        cancel.check()?;
        let Some(manifest_id) = public_manifest(app, depot, sources)? else {
            return Ok(None);
        };
        let key_app = if depot.shared_from_app == Some(COMMON_REDISTRIBUTABLES) {
            COMMON_REDISTRIBUTABLES
        } else {
            app.app_id
        };
        let key: [u8; 32] = self
            .cm
            .depot_key(depot.depot_id, key_app)
            .await?
            .try_into()
            .map_err(|_| ClientError::Failure("backend.steam.client.invalidDepotKey".into()))?;
        Ok(Some(PreparedDepot {
            depot_id: depot.depot_id,
            manifest_id,
            authorizing_app: content_authorizing_app(app, depot),
            key,
            estimated_bytes: depot.size_bytes.unwrap_or(0),
        }))
    }

    /// Installs one depot from every server allowed to serve it. A server that
    /// refuses the depot is authorized over the CM.
    async fn install_depot(
        &self,
        content: &ContentClient,
        depot: &PreparedDepot,
        servers: &[ContentServer],
        destination: &Path,
        transfer: &TransferContext<TransferProgress>,
        base: ProgressBase,
    ) -> Result<DepotInstallation, ClientError> {
        let cancel = transfer.cancel_token();
        // The current manifest ID and depot key came from authorized Steam
        // metadata above. Keep one validated snapshot across all CDN requests.
        let cached_manifest = content
            .cached_manifest(destination, depot.depot_id, depot.manifest_id, &depot.key)
            .await?;
        cancel.check()?;
        // No request code is needed for an already validated cached archive. A
        // miss always gets a fresh code just before the CDN.
        let request_code = if cached_manifest.is_some() {
            0
        } else {
            self.cm
                .manifest_request_code(depot.depot_id, depot.authorizing_app, depot.manifest_id)
                .await?
        };
        let authorization = DepotAuthorization {
            cm: &self.cm,
            app_id: depot.authorizing_app,
            depot_id: depot.depot_id,
        };
        let plan = DepotDownload {
            depot_id: depot.depot_id,
            manifest_id: depot.manifest_id,
            request_code,
            depot_key: depot.key,
            servers: servers.to_vec(),
            authorization: Some(&authorization),
            destination: destination.to_owned(),
            cached_manifest,
        };
        let depot_label =
            crate::message::format("backend.steam.client.depot", &[depot.depot_id.to_string()]);
        // Progress arrives from the content module's network and disk
        // threads, so the callback owns everything it touches.
        let transfer = transfer.clone();
        let size = depot.estimated_bytes;
        content
            .download_depot(&plan, cancel, move |progress: DownloadProgress| {
                transfer.update(|status| {
                    status.show(progress);
                    status.completed_bytes += base.completed;
                    status.total_bytes += base.total_estimate.saturating_sub(size);
                    status.network_bytes += base.network;
                    status.content_bytes += base.content_bytes;
                    status.message.clone_from(&depot_label);
                });
            })
            .await
            .map_err(Into::into)
    }
}

/// Asks Steam for the CDN authorization a server requires for one depot.
pub(crate) struct DepotAuthorization<'a> {
    pub(crate) cm: &'a SteamConnection,
    pub(crate) app_id: u32,
    pub(crate) depot_id: u32,
}

impl CdnAuthorization for DepotAuthorization<'_> {
    fn token<'a>(&'a self, host: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            self.cm
                .cdn_auth(self.app_id, self.depot_id, host)
                .await
                .ok()
        })
    }
}

/// The apps whose PICS metadata names the manifests of `selected`'s shared
/// depots that have none of their own; one PICS query covers them all.
fn shared_sources(app: &AppInfo, selected: &[&DepotInfo]) -> Vec<u32> {
    selected
        .iter()
        .filter(|depot| depot.public_manifest.is_none())
        .filter_map(|depot| depot.shared_from_app)
        .filter(|source| *source != app.app_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// The depot's public manifest ID, taken from its source app for a shared
/// depot without its own. `None` when it has no public manifest.
fn public_manifest(
    app: &AppInfo,
    depot: &DepotInfo,
    sources: &HashMap<u32, Arc<AppInfo>>,
) -> Result<Option<u64>, ClientError> {
    let manifest_app = depot.shared_from_app.unwrap_or(app.app_id);
    match depot.public_manifest {
        Some(id) => Ok(Some(id)),
        None if manifest_app != app.app_id => sources
            .get(&manifest_app)
            .ok_or(crate::cm::CmError::MissingApp(manifest_app))?
            .depots
            .iter()
            .find(|other| other.depot_id == depot.depot_id)
            .and_then(|other| other.public_manifest)
            .map(Some)
            .ok_or_else(|| {
                ClientError::Failure(crate::message::format(
                    "backend.steam.client.sharedManifestMissing",
                    &[depot.depot_id.to_string()],
                ))
            }),
        None => Ok(None),
    }
}

/// Whether the account's licenses grant the Mac build and the Windows build.
fn installable_builds(app: &AppInfo, owned: &ContentLicenses, language: &str) -> (bool, bool) {
    let installable = |platform| !select_depots(app, owned, platform, language).is_empty();
    (installable("macos"), installable("windows"))
}

fn select_depots<'a>(
    app: &'a AppInfo,
    owned: &ContentLicenses,
    platform: &str,
    language: &str,
) -> Vec<&'a DepotInfo> {
    if !supports_platform(&app.os_list, platform) {
        return Vec::new();
    }
    let eligible: Vec<_> = app
        .depots
        .iter()
        .filter(|depot| {
            let redistributable = depot.shared_from_app == Some(COMMON_REDISTRIBUTABLES);
            supports_platform(&depot.os_list, platform)
                && !(redistributable && SKIPPED_REDISTRIBUTABLES.contains(&depot.depot_id))
                && (app.free_to_download
                    || owned.depots.contains(&depot.depot_id)
                    || redistributable)
                && depot.dlc_app_id.is_none_or(|dlc| owned.apps.contains(&dlc))
                && (depot.public_manifest.is_some() || depot.shared_from_app.is_some())
        })
        .collect();
    // Shared asset depots often have no OS restriction. They do not establish
    // that the application ships an executable for an otherwise unknown OS.
    if app.os_list.is_empty()
        && !eligible
            .iter()
            .any(|depot| depot.os_list.iter().any(|os| os == platform))
    {
        return Vec::new();
    }
    let has_language = eligible
        .iter()
        .any(|depot| depot.language.as_deref() == Some(language));
    let language = if has_language { language } else { "english" };
    let eligible: Vec<_> = eligible
        .into_iter()
        .filter(|depot| {
            depot
                .language
                .as_deref()
                .is_none_or(|value| value.is_empty() || value == language)
        })
        .collect();
    let arch = if eligible
        .iter()
        .any(|depot| depot.os_arch.as_deref() == Some("64"))
    {
        "64"
    } else {
        "32"
    };
    eligible
        .into_iter()
        .filter(|depot| {
            depot
                .os_arch
                .as_deref()
                .is_none_or(|value| value == arch || value.is_empty())
        })
        .collect()
}

fn content_authorizing_app(app: &AppInfo, depot: &DepotInfo) -> u32 {
    // FreeToDownload titles authorize shared depot requests through their own app,
    // even when the manifest itself must be looked up on the source app.
    if app.free_to_download {
        app.app_id
    } else {
        depot.shared_from_app.unwrap_or(app.app_id)
    }
}

/// Lower layers keep typed errors; the client passes message keys to the host.
impl From<Cancelled> for ClientError {
    fn from(_: Cancelled) -> Self {
        ClientError::Cancelled("backend.steam.client.downloadStopped".into())
    }
}

impl From<ContentError> for ClientError {
    fn from(error: ContentError) -> Self {
        if matches!(error, ContentError::Cancelled) {
            ClientError::Cancelled(error.to_string())
        } else {
            ClientError::Failure(error.to_string())
        }
    }
}

impl From<crate::cm::CmError> for ClientError {
    fn from(error: crate::cm::CmError) -> Self {
        ClientError::Failure(error.to_string())
    }
}

impl From<crate::cm::SocialError> for ClientError {
    fn from(error: crate::cm::SocialError) -> Self {
        ClientError::Failure(error.to_string())
    }
}

impl From<crate::launch::LaunchError> for ClientError {
    fn from(error: crate::launch::LaunchError) -> Self {
        ClientError::Failure(error.to_string())
    }
}

impl From<TransferBusy> for ClientError {
    fn from(_: TransferBusy) -> Self {
        ClientError::Failure("backend.steam.client.downloadInProgress".into())
    }
}

impl From<InstallFailure> for ClientError {
    fn from(failure: InstallFailure) -> Self {
        ClientError::Failure(
            match failure {
                InstallFailure::ParentMissing => "backend.steam.client.chooseExistingFolder",
                InstallFailure::UnsafeDirectory(_) => {
                    "backend.steam.client.unsupportedPlatformOrLanguage"
                }
                InstallFailure::DestinationIsLink(_) => {
                    "backend.steam.client.installationDirectorySymlink"
                }
                InstallFailure::DestinationIsFile(_) => {
                    "backend.steam.client.installationDirectoryIsFile"
                }
                InstallFailure::UnsafeLock(_) => "backend.steam.client.invalidInstallationLock",
                InstallFailure::Locked => "backend.steam.client.installationLocked",
                InstallFailure::UnsafeReceipt(_) => "backend.steam.client.installationFileSymlink",
                InstallFailure::Io(_) => "backend.steam.client.storageFailed",
            }
            .into(),
        )
    }
}

impl TransferError for ClientError {
    fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn depot(id: u32, os: &str, language: Option<&str>, dlc: Option<u32>) -> DepotInfo {
        DepotInfo {
            depot_id: id,
            shared_from_app: None,
            dlc_app_id: dlc,
            os_list: if os.is_empty() {
                vec![]
            } else {
                vec![os.into()]
            },
            os_arch: None,
            language: language.map(Into::into),
            public_manifest: Some(1),
            size_bytes: None,
        }
    }
    fn licenses(
        apps: impl IntoIterator<Item = u32>,
        depots: impl IntoIterator<Item = u32>,
    ) -> ContentLicenses {
        ContentLicenses {
            apps: apps.into_iter().collect(),
            depots: depots.into_iter().collect(),
            ..ContentLicenses::default()
        }
    }

    #[test]
    fn unlicensed_depot_without_dlc_tag_is_not_selected() {
        // Dead Cells publishes this additional Mac depot without dlcappid.
        // Its base package grants 588652, but not 1046443.
        let mut app = app(vec![
            depot(588652, "macos", None, None),
            depot(1046443, "macos", None, None),
        ]);
        app.app_id = 588650;
        let rights = licenses([588650], [588651, 588652, 588653, 588654]);
        let selected: Vec<_> = select_depots(&app, &rights, "macos", "english")
            .iter()
            .map(|d| d.depot_id)
            .collect();
        assert_eq!(selected, [588652]);
        let rights = licenses([588650], [588652, 1046443]);
        assert_eq!(select_depots(&app, &rights, "macos", "english").len(), 2);
    }

    #[test]
    fn the_installable_builds_are_those_the_account_owns() {
        let app = AppInfo {
            os_list: vec!["windows".into(), "macos".into()],
            ..app(vec![
                depot(2, "windows", None, None),
                depot(3, "macos", None, None),
            ])
        };
        assert_eq!(
            installable_builds(&app, &licenses([1], [2, 3]), "english"),
            (true, true)
        );
        assert_eq!(
            installable_builds(&app, &licenses([1], [2]), "english"),
            (false, true)
        );
        assert_eq!(
            installable_builds(&app, &licenses([1], []), "english"),
            (false, false)
        );
    }

    #[test]
    fn depot_rights_do_not_grant_dlc_app_ownership() {
        let app = app(vec![depot(2, "macos", None, Some(99))]);
        assert!(select_depots(&app, &licenses([1], [2, 99]), "macos", "english").is_empty());
        assert_eq!(
            select_depots(&app, &licenses([1, 99], [2]), "macos", "english").len(),
            1
        );
    }

    #[test]
    fn free_to_download_depots_do_not_require_package_membership() {
        let mut app = app(vec![depot(2, "macos", None, None)]);
        assert!(select_depots(&app, &licenses([1], []), "macos", "english").is_empty());
        app.free_to_download = true;
        assert_eq!(
            select_depots(&app, &licenses([1], []), "macos", "english").len(),
            1
        );
    }

    #[test]
    fn unlicensed_depots_cannot_choose_language_or_architecture() {
        let mut english = depot(2, "macos", Some("english"), None);
        english.os_arch = Some("32".into());
        let mut unlicensed = depot(3, "macos", Some("french"), None);
        unlicensed.os_arch = Some("64".into());
        let app = app(vec![english, unlicensed]);
        let rights = licenses([1], [2]);
        let selected = select_depots(&app, &rights, "macos", "french");
        assert_eq!(selected.iter().map(|d| d.depot_id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn selection_respects_platform_language_and_dlc_ownership() {
        let app = AppInfo {
            app_id: 1,
            name: "Game".into(),
            free_to_download: false,
            os_list: vec!["windows".into(), "macos".into()],
            launch_configs: vec![],
            cloud: None,
            tree: crate::kv::Kv::Object(Vec::new()),
            depots: vec![
                depot(1, "", None, None),
                depot(2, "windows", Some("english"), None),
                depot(3, "windows", Some("french"), None),
                depot(4, "macos", None, None),
                depot(5, "windows", None, Some(99)),
            ],
        };
        let owned = licenses([1], [1, 2, 3, 4, 5]);
        assert_eq!(
            select_depots(&app, &owned, "windows", "french")
                .iter()
                .map(|d| d.depot_id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(
            select_depots(&app, &owned, "macos", "french")
                .iter()
                .map(|d| d.depot_id)
                .collect::<Vec<_>>(),
            vec![1, 4]
        );
    }
    fn app(depots: Vec<DepotInfo>) -> AppInfo {
        AppInfo {
            app_id: 1,
            name: "Game".into(),
            free_to_download: false,
            os_list: vec!["windows".into(), "macos".into()],
            launch_configs: vec![],
            cloud: None,
            tree: crate::kv::Kv::Object(Vec::new()),
            depots,
        }
    }
    #[test]
    fn unavailable_depots_cannot_change_language_or_architecture_selection() {
        let mut english = depot(2, "windows", Some("english"), None);
        english.os_arch = Some("32".into());
        let mut unavailable_dlc = depot(3, "windows", Some("french"), Some(99));
        unavailable_dlc.os_arch = Some("64".into());
        let mut unpublished = depot(4, "windows", Some("french"), None);
        unpublished.public_manifest = None;
        unpublished.os_arch = Some("64".into());
        let mut other_platform = depot(5, "macos", Some("french"), None);
        other_platform.os_arch = Some("64".into());
        let app = app(vec![
            depot(1, "", None, None),
            english,
            unavailable_dlc,
            unpublished,
            other_platform,
        ]);
        let selected: Vec<_> =
            select_depots(&app, &licenses([1], [1, 2, 3, 4, 5]), "windows", "french")
                .into_iter()
                .map(|depot| depot.depot_id)
                .collect();
        assert_eq!(selected, [1, 2]);
    }
    #[test]
    fn neutral_depots_cannot_create_unsupported_or_unknown_platform_support() {
        let owned = licenses([1], [1, 2, 3, 4, 5]);
        let mut app = app(vec![
            depot(1, "", None, None),
            depot(2, "windows", None, None),
        ]);
        app.os_list = vec!["windows".into()];
        assert!(select_depots(&app, &owned, "macos", "english").is_empty());
        assert_eq!(select_depots(&app, &owned, "windows", "english").len(), 2);
        app.os_list.clear();
        assert!(select_depots(&app, &owned, "macos", "english").is_empty());
        assert_eq!(select_depots(&app, &owned, "windows", "english").len(), 2);
        app.depots.retain(|depot| depot.os_list.is_empty());
        assert!(select_depots(&app, &owned, "windows", "english").is_empty());
    }
    #[test]
    fn unowned_platform_specific_dlc_cannot_establish_unknown_app_platform() {
        let mut app = app(vec![
            depot(1, "", None, None),
            depot(2, "macos", None, Some(99)),
        ]);
        app.os_list.clear();
        assert!(
            select_depots(&app, &licenses([1], [1, 2, 3, 4, 5]), "macos", "english").is_empty()
        );
        assert_eq!(
            select_depots(
                &app,
                &licenses([1, 99], [1, 2, 3, 4, 5]),
                "macos",
                "english"
            )
            .len(),
            2
        );
    }
    #[test]
    fn architecture_is_chosen_among_depots_for_the_selected_language() {
        let mut english = depot(2, "windows", Some("english"), None);
        english.os_arch = Some("64".into());
        let mut french = depot(3, "windows", Some("french"), None);
        french.os_arch = Some("32".into());
        let app = app(vec![english, french]);
        assert_eq!(
            select_depots(&app, &licenses([1], [1, 2, 3, 4, 5]), "windows", "french")[0].depot_id,
            3
        );
    }
    #[test]
    fn a_windows_game_gets_the_common_redistributables_it_names_without_owning_them() {
        let mut directx = depot(228_990, "windows", None, None);
        directx.shared_from_app = Some(COMMON_REDISTRIBUTABLES);
        directx.public_manifest = None;
        let mut visual_cpp = depot(228_989, "windows", None, None);
        visual_cpp.shared_from_app = Some(COMMON_REDISTRIBUTABLES);
        let mut dotnet = depot(229_005, "windows", None, None);
        dotnet.shared_from_app = Some(COMMON_REDISTRIBUTABLES);
        let mut other = depot(3, "windows", None, None);
        other.shared_from_app = Some(99);
        other.public_manifest = None;
        let app = app(vec![
            depot(2, "windows", None, None),
            directx,
            visual_cpp,
            dotnet,
            other,
        ]);
        let selected: Vec<_> = select_depots(
            &app,
            &licenses([1], [2, 228_989, 229_005]),
            "windows",
            "english",
        )
        .iter()
        .map(|depot| depot.depot_id)
        .collect();
        assert_eq!(selected, [2, 228_990, 228_989]);
        assert!(select_depots(&app, &licenses([1], [2]), "macos", "english").is_empty());
    }
    #[test]
    fn free_to_download_shared_depots_use_the_requesting_app_for_authorization() {
        let mut shared = depot(2, "windows", None, None);
        shared.shared_from_app = Some(99);
        let mut app = app(vec![]);
        assert_eq!(content_authorizing_app(&app, &shared), 99);
        app.free_to_download = true;
        assert_eq!(content_authorizing_app(&app, &shared), 1);
    }
}
