//! Steam's part of the installations through their Content interface, with a
//! network the test stands in for and real files in a temporary library.
use super::*;
use crate::installations::{Content as _, tests::FakeRuntimes};
use std::{path::PathBuf, sync::Mutex, time::Duration};

/// The Cloud rules an archive finds.
type Rules = Result<Option<(CloudMetadata, CloudListing)>, steam_cloud::Failure>;

/// What the test's Steam network was asked, and what it answers.
#[derive(Default)]
struct Fake {
    /// The platform pick, the runtimes and the game's files, in the order
    /// the Transfer asked for them.
    calls: Arc<Mutex<Vec<String>>>,
    /// The build Steam picks; the Mac one without it.
    picked: Mutex<Option<String>>,
    downloads: Mutex<Vec<Transfer<Steam>>>,
    /// Each maintenance's target, and whether a Steam launch could start
    /// meanwhile.
    maintained: Mutex<Vec<(steam_cloud::MaintenanceTarget, bool)>>,
    archive: Mutex<Option<Rules>>,
    locations: Mutex<Option<Locations>>,
}

impl Fake {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Network for Fake {
    fn platform<'a>(&'a self, _: u32, _: &'a str) -> BoxFuture<'a, Result<String, Failure>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("platform".into());
            let picked = self.picked.lock().unwrap().clone();
            Ok(picked.unwrap_or_else(|| "macos".into()))
        })
    }
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<steam_kit::client::Installation, Failure>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("game".into());
            self.downloads.lock().unwrap().push(transfer.clone());
            Ok(steam_kit::client::Installation {
                app_id: transfer.product.app_id,
                name: transfer.name.clone(),
                directory: transfer
                    .directory
                    .join(format!(
                        "steam-{}-{}",
                        transfer.product.app_id,
                        transfer.platform.as_deref().unwrap_or("macos")
                    ))
                    .to_string_lossy()
                    .into_owned(),
                platform: transfer.platform.clone().unwrap_or_else(|| "macos".into()),
                language: transfer.product.language.clone(),
                total_bytes: 100,
                manifests: Vec::new(),
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        None
    }
    fn cancel(&self) {}
    fn update_available<'a>(
        &'a self,
        _: &'a Receipt<Steam>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async { Ok(false) })
    }
    fn launch_target<'a>(
        &'a self,
        _: u32,
        _: &'a str,
        _: &'a Path,
    ) -> BoxFuture<'a, Result<LaunchTarget, Failure>> {
        Box::pin(async {
            Err(Failure::Failed(
                "backend.steam.launch.directoryMissing".into(),
            ))
        })
    }
    fn archive_rules(
        &self,
        _: u32,
    ) -> BoxFuture<'_, Result<Option<(CloudMetadata, CloudListing)>, steam_cloud::Failure>> {
        let rules = self.archive.lock().unwrap().take().unwrap_or(Ok(None));
        Box::pin(async move { rules })
    }
    fn maintain(
        &self,
        target: steam_cloud::MaintenanceTarget,
        _: &Credentials,
        _: Arc<Control>,
        _: &crate::steam::IdleGuard,
    ) -> Result<(), steam_cloud::Failure> {
        let locations = self.locations.lock().unwrap().clone().unwrap();
        let launch_could_start =
            crate::steam::Client::idle_guard(&locations.steam_sessions, &locations.helper).is_ok();
        self.maintained
            .lock()
            .unwrap()
            .push((target, launch_could_start));
        Ok(())
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    locations: Locations,
    fake: Arc<Fake>,
    runtimes: Arc<FakeRuntimes>,
    content: Content<Fake>,
}

impl Fixture {
    /// Every runtime is in place.
    fn new() -> Self {
        Self::with_runtimes(&[Runtime::SteamComponents, Runtime::Voile])
    }

    fn with_runtimes(present: &[Runtime]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().canonicalize().unwrap();
        let locations = Locations::new(
            &library,
            Path::new("/Applications/Obelisk.app"),
            &library.join("home"),
        );
        let fake = Arc::new(Fake::default());
        *fake.locations.lock().unwrap() = Some(locations.clone());
        let credentials = Credentials {
            steam_id: 42,
            account_name: "fixture".into(),
            refresh_token: "fixture-only-never-sent".into(),
        };
        let calls = fake.calls.clone();
        let runtimes = FakeRuntimes::new(present, move |runtime| {
            calls.lock().unwrap().push(runtime);
        });
        let mut content = Content::new(fake.clone(), credentials, locations.clone());
        content.runtimes = Runtimes::from_source(runtimes.clone());
        Self {
            content,
            _root: root,
            locations,
            fake,
            runtimes,
        }
    }
    /// Portal 2, installed for `platform`.
    fn installed(&self, platform: &str) -> Receipt<Steam> {
        let directory = self
            .locations
            .library
            .join(format!("Games/Steam/steam-620-{platform}"));
        std::fs::create_dir_all(&directory).unwrap();
        Receipt {
            product: portal(),
            name: "Portal 2".into(),
            directory: directory.to_string_lossy().into_owned(),
            platform: platform.into(),
            total_bytes: 100,
            needs_verification: false,
        }
    }
    /// Portal 2's Transfer for `platform`.
    fn transfer(&self, platform: &str) -> Transfer<Steam> {
        Transfer {
            product: portal(),
            name: "Portal 2".into(),
            platform: Some(platform.into()),
            directory: self.locations.library.join("Games/Steam"),
        }
    }
    fn maintained(&self) -> Vec<(steam_cloud::MaintenanceTarget, bool)> {
        self.fake.maintained.lock().unwrap().clone()
    }
}

/// Portal 2, with its French depots.
fn portal() -> Product {
    Product {
        app_id: 620,
        language: "french".into(),
    }
}

fn control() -> Arc<Control> {
    Arc::new(Control::new(|_| {}))
}

fn installed_config(target: &steam_cloud::MaintenanceTarget) -> steam_cloud::Config {
    match target {
        steam_cloud::MaintenanceTarget::Installed(config) => config.clone(),
        target => panic!("an installed game syncs its installation, not {target:?}"),
    }
}

#[tokio::test]
async fn a_completed_transfer_is_an_installation_that_needs_no_verification() {
    let fixture = Fixture::new();
    let transfer = fixture.transfer("macos");

    let receipt = fixture.content.download(&transfer).await.unwrap();

    assert_eq!(
        *fixture.fake.downloads.lock().unwrap(),
        vec![transfer.clone()]
    );
    assert_eq!(receipt.product, portal());
    assert_eq!(receipt.platform, "macos");
    assert!(!receipt.needs_verification);
    assert_eq!(
        PathBuf::from(&receipt.directory),
        transfer.directory.join("steam-620-macos")
    );
}

#[tokio::test]
async fn a_windows_game_keeps_its_cloud_saves_in_its_wine_prefix_and_a_mac_game_has_none() {
    let fixture = Fixture::new();
    let (windows, mac) = (fixture.installed("windows"), fixture.installed("macos"));

    for receipt in [&windows, &mac] {
        fixture
            .content
            .maintain(&620, Some(receipt), control())
            .await
            .unwrap();
    }

    let maintained = fixture.maintained();
    let (windows, mac) = (
        installed_config(&maintained[0].0),
        installed_config(&maintained[1].0),
    );
    assert_eq!(
        windows.prefix,
        Some(fixture.locations.wine_prefixes.join("steam-620-windows"))
    );
    assert_eq!(windows.root, fixture.locations.cloud_archive);
    assert_eq!(windows.home, fixture.locations.home);
    assert_eq!(windows.app_id, 620);
    assert_eq!(mac.prefix, None);
}

#[tokio::test]
async fn cloud_maintenance_keeps_every_steam_launch_out_while_it_runs() {
    let fixture = Fixture::new();
    let receipt = fixture.installed("macos");

    fixture
        .content
        .maintain(&620, Some(&receipt), control())
        .await
        .unwrap();

    let (_, launch_could_start) = fixture.maintained()[0].clone();
    assert!(!launch_could_start, "maintenance holds Steam's idle guard");
    assert!(
        crate::steam::Client::idle_guard(
            &fixture.locations.steam_sessions,
            &fixture.locations.helper
        )
        .is_ok(),
        "the guard goes with the maintenance"
    );
}

#[tokio::test]
async fn an_uninstalled_game_syncs_what_its_saved_context_names() {
    let fixture = Fixture::new();

    fixture
        .content
        .maintain(&620, None, control())
        .await
        .unwrap();

    match &fixture.maintained()[0].0 {
        steam_cloud::MaintenanceTarget::Uninstalled { root, app_id } => {
            assert_eq!(root, &fixture.locations.cloud_archive);
            assert_eq!(*app_id, 620);
        }
        target => panic!("an uninstalled game has no installation to sync: {target:?}"),
    }
}

#[tokio::test]
async fn a_game_without_steam_cloud_archives_nothing_and_a_failure_is_its_cloud_key() {
    let fixture = Fixture::new();
    let receipt = fixture.installed("macos");

    fixture.content.archive(&receipt).await.unwrap();

    *fixture.fake.archive.lock().unwrap() = Some(Err(steam_cloud::Failure::Network));
    assert_eq!(
        fixture.content.archive(&receipt).await,
        Err(Failure::Failed(steam_cloud::Failure::Network.key().into()))
    );
}

#[tokio::test]
async fn an_uninstalled_windows_game_loses_its_prefix_and_a_mac_game_has_none() {
    let fixture = Fixture::new();
    let (windows, mac) = (fixture.installed("windows"), fixture.installed("macos"));
    let prefix = fixture.locations.wine_prefixes.join("steam-620-windows");
    std::fs::create_dir_all(prefix.join("drive_c")).unwrap();

    assert_eq!(
        fixture.content.directory_name(&windows),
        "steam-620-windows"
    );
    assert_eq!(fixture.content.remove_prefix(&windows), Some(Ok(())));
    assert!(!prefix.exists());
    assert_eq!(fixture.content.remove_prefix(&mac), None);
    assert_eq!(fixture.content.directory_name(&mac), "steam-620-macos");
}

#[tokio::test]
async fn a_transfer_gets_valve_components_then_voile_for_a_windows_build_before_the_game() {
    for (platform, calls) in [
        ("windows", vec!["steam components", "voile", "game"]),
        ("macos", vec!["steam components", "game"]),
    ] {
        let fixture = Fixture::with_runtimes(&[]);
        let transfer = fixture.transfer(platform);

        fixture.content.download(&transfer).await.unwrap();

        assert_eq!(fixture.fake.calls(), calls, "{platform}");
    }
}

#[tokio::test]
async fn runtimes_in_place_are_not_downloaded_again() {
    let fixture = Fixture::with_runtimes(&[Runtime::SteamComponents, Runtime::Voile]);
    let transfer = fixture.transfer("windows");

    fixture.content.download(&transfer).await.unwrap();

    assert_eq!(fixture.fake.calls(), ["game"]);
}

#[tokio::test]
async fn a_transfer_without_a_platform_picks_the_build_before_its_runtimes() {
    let fixture = Fixture::with_runtimes(&[Runtime::SteamComponents]);
    *fixture.fake.picked.lock().unwrap() = Some("windows".into());
    let mut transfer = fixture.transfer("windows");
    transfer.platform = None;

    let receipt = fixture.content.download(&transfer).await.unwrap();

    assert_eq!(fixture.fake.calls(), ["platform", "voile", "game"]);
    let downloaded = fixture.fake.downloads.lock().unwrap()[0].clone();
    assert_eq!(downloaded.platform.as_deref(), Some("windows"));
    assert_eq!(receipt.platform, "windows");
}

#[tokio::test]
async fn a_runtime_that_fails_to_download_fails_the_transfer_before_the_game() {
    let fixture = Fixture::with_runtimes(&[]);
    let failure = Failure::Failed("backend.steam.components.unavailable".into());
    fixture.runtimes.fail(failure.clone());
    let transfer = fixture.transfer("windows");

    assert_eq!(fixture.content.download(&transfer).await, Err(failure));
    assert_eq!(fixture.fake.calls(), ["steam components"]);
    let stopped = fixture.content.progress().unwrap();
    assert_eq!(stopped.message, "backend.steam.components.unavailable");
}

#[tokio::test]
async fn cancelling_the_transfer_cancels_its_runtime_download_and_the_game_never_starts() {
    let fixture = Fixture::with_runtimes(&[Runtime::SteamComponents]);
    fixture.runtimes.hold();
    let transfer = fixture.transfer("windows");

    let (downloaded, ()) = tokio::join!(fixture.content.download(&transfer), async {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while fixture.content.progress().is_none() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let shown = fixture.content.progress().unwrap();
        assert_eq!(shown.game, "620");
        assert_eq!(shown.message, "model.game.installingVoile");
        fixture.content.cancel();
    });

    assert_eq!(downloaded, Err(Failure::Cancelled));
    assert_eq!(fixture.fake.calls(), ["voile"]);
    assert!(!fixture.runtimes.present().contains(&Runtime::Voile));
}
