//! Epic's part of the installations through their Content interface, with a
//! network the test stands in for and real files in a temporary library.
use super::*;
use crate::{
    installations::{Content as _, tests::eventually},
    runtimes::{
        Runtimes,
        tests::{FakeRuntimes, ready, recorded},
    },
    sessions::Sessions,
};
use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::Notify;

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const STRANGER: &str = "fedcba9876543210fedcba9876543210";

/// What the test's Epic network was asked, in order, and what it answers.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
    /// The account each `account_id` call finds, then `account`.
    accounts: Mutex<VecDeque<String>>,
    /// Renewing credentials waits until the test releases it.
    held: Mutex<bool>,
    released: Notify,
    /// Each maintenance's configuration, and whether an Epic launch could
    /// start meanwhile.
    maintained: Mutex<Vec<(Configuration, bool)>>,
    maintenance: Mutex<Option<Error>>,
}

impl Fake {
    fn call(&self, call: &str) {
        self.calls.lock().unwrap().push(call.into());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Network for Fake {
    fn prepare(&self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move {
            self.call("prepare");
            if *self.held.lock().unwrap() {
                self.released.notified().await;
            }
            Ok(())
        })
    }
    fn account_id(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            self.call("account");
            Ok(self
                .accounts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| ACCOUNT.into()))
        })
    }
    fn exchange_code(&self) -> BoxFuture<'_, Result<String, Failure>> {
        Box::pin(async move {
            self.call("exchange");
            Ok("fixture-code".into())
        })
    }
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Epic>,
    ) -> BoxFuture<'a, Result<epic_kit::client::Installation, Failure>> {
        Box::pin(async move {
            self.call("download");
            Ok(epic_kit::client::Installation {
                app_name: transfer.product.app_name.clone(),
                namespace: transfer.product.namespace.clone(),
                catalog_item_id: transfer.product.catalog_item_id.clone(),
                account_id: ACCOUNT.into(),
                directory: transfer
                    .directory
                    .join("epic-hash-Mac")
                    .to_string_lossy()
                    .into_owned(),
                platform: transfer.platform.clone(),
                total_bytes: 100,
                build_version: "1".into(),
                launch_executable: "Game.app".into(),
                launch_command: String::new(),
                deployment_id: None,
                prerequisite: None,
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        None
    }
    fn cancel(&self) {
        self.call("cancel");
    }
    fn update_available<'a>(
        &'a self,
        _: &'a Receipt<Epic>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            self.call("update");
            Ok(true)
        })
    }
    fn archive(&self, _: Configuration) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.call("archive");
            Ok(())
        })
    }
    fn maintain<'a>(
        self: Arc<Self>,
        config: Configuration,
        _: Arc<Control>,
        _: &'a IdleGuard,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let launch_could_start =
                crate::epic::Client::idle_guard(&config.session_directory, &config.helper).is_ok();
            self.maintained
                .lock()
                .unwrap()
                .push((config, launch_could_start));
            match self.maintenance.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    locations: Locations,
    fake: Arc<Fake>,
    content: Content<Fake>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().canonicalize().unwrap();
        let locations = Locations::new(
            &library,
            Path::new("/Applications/Obelisk.app"),
            &library.join("home"),
        );
        std::fs::create_dir_all(&locations.cloud_archive).unwrap();
        let fake = Arc::new(Fake::default());
        let saves = fake.clone();
        let content = Content::new(
            fake.clone(),
            Arc::new(move || saves.call("saved")),
            ACCOUNT.into(),
            "fr".into(),
            locations.clone(),
        );
        Self {
            _root: root,
            locations,
            fake,
            content,
        }
    }
    /// The build of `platform`, installed.
    fn installed(&self, platform: &str) -> Receipt<Epic> {
        let directory = self
            .locations
            .library
            .join(format!("Games/Epic/epic-hash-{platform}"));
        std::fs::create_dir_all(&directory).unwrap();
        Receipt {
            product: product(),
            name: "Fixture".into(),
            directory: directory.to_string_lossy().into_owned(),
            platform: platform.into(),
            total_bytes: 100,
            needs_verification: false,
        }
    }
    fn transfer(&self, platform: &str) -> Transfer<Epic> {
        Transfer {
            product: product(),
            name: "Fixture".into(),
            platform: platform.into(),
            directory: self.locations.library.join("Games/Epic"),
        }
    }
}

/// The game's catalog item.
fn product() -> Product {
    Product {
        namespace: "sandbox".into(),
        catalog_item_id: "catalog".into(),
        app_name: "game".into(),
    }
}

fn game() -> Store {
    Store {
        namespace: "sandbox".into(),
        app_name: "game".into(),
    }
}

fn control() -> Arc<Control> {
    Arc::new(Control::new(|_| {}))
}

#[tokio::test]
async fn every_account_call_renews_and_saves_the_credentials_first() {
    let fixture = Fixture::new();
    let transfer = fixture.transfer("Mac");

    assert_eq!(fixture.content.begin(&transfer).await.unwrap(), "Mac");
    let receipt = fixture.content.download(&transfer).await.unwrap();

    assert_eq!(receipt.product, product());
    assert_eq!(receipt.name, "Fixture");
    assert!(!receipt.needs_verification);
    // The Transfer's own rotation is saved after it, without holding it.
    eventually(|| fixture.fake.calls().len() == 4).await;
    assert_eq!(
        fixture.fake.calls(),
        ["prepare", "saved", "download", "saved"]
    );

    let receipt = fixture.installed("Mac");
    fixture.content.archive(&receipt).await.unwrap();
    eventually(|| fixture.fake.calls().len() == 8).await;
    assert_eq!(
        fixture.fake.calls()[4..],
        ["prepare", "saved", "archive", "saved"]
    );

    // A short call renews the token itself, then its rotation is saved.
    assert!(fixture.content.update_available(&receipt).await.unwrap());
    eventually(|| fixture.fake.calls().len() == 10).await;
    assert_eq!(fixture.fake.calls()[8..], ["update", "saved"]);
}

#[tokio::test]
async fn a_transfer_cancelled_while_its_credentials_renew_never_begins() {
    let fixture = Fixture::new();
    let transfer = fixture.transfer("Mac");
    *fixture.fake.held.lock().unwrap() = true;

    let (begun, ()) = tokio::join!(fixture.content.begin(&transfer), async {
        eventually(|| fixture.fake.calls().contains(&"prepare".to_string())).await;
        fixture.content.cancel();
        fixture.fake.released.notify_one();
    });

    assert_eq!(begun, Err(Failure::Cancelled));
}

#[tokio::test]
async fn a_windows_build_keeps_its_saves_in_its_wine_prefix_and_a_mac_build_has_none() {
    let fixture = Fixture::new();
    for platform in ["Windows", "Mac"] {
        let receipt = fixture.installed(platform);
        fixture
            .content
            .maintain(&game(), Some(&receipt), control())
            .await
            .unwrap();
    }

    let maintained = fixture.fake.maintained.lock().unwrap().clone();
    let (windows, mac) = (&maintained[0].0, &maintained[1].0);
    assert_eq!(
        windows.prefix,
        Some(fixture.locations.wine_prefixes.join("epic-hash-Windows"))
    );
    assert_eq!(windows.catalog_item_id, "catalog");
    assert_eq!(windows.language, "fr");
    assert_eq!(windows.root, fixture.locations.cloud_archive);
    assert_eq!(windows.session_directory, fixture.locations.epic_sessions);
    assert_eq!(mac.prefix, None);
    assert!(
        maintained
            .iter()
            .all(|(_, launch_could_start)| !launch_could_start),
        "maintenance holds Epic's idle guard"
    );
}

#[tokio::test]
async fn an_uninstalled_game_without_a_saved_context_cannot_sync_and_a_cancelled_sync_ends_silently()
 {
    let fixture = Fixture::new();

    assert_eq!(
        fixture.content.maintain(&game(), None, control()).await,
        Err(Failure::Failed("backend.epic.cloud.failed".into()))
    );
    assert!(fixture.fake.maintained.lock().unwrap().is_empty());

    let receipt = fixture.installed("Mac");
    *fixture.fake.maintenance.lock().unwrap() = Some(Error::Cancelled);
    assert_eq!(
        fixture
            .content
            .maintain(&game(), Some(&receipt), control())
            .await,
        Err(Failure::Cancelled)
    );
}

#[tokio::test]
async fn a_launch_code_that_another_account_signed_is_refused_before_the_game_starts() {
    let fixture = Fixture::new();
    let receipt = fixture.installed("Mac");
    fixture
        .fake
        .accounts
        .lock()
        .unwrap()
        .extend([ACCOUNT.to_string(), STRANGER.to_string()]);
    let sessions = Sessions::<Epic>::new(Box::new(|| Ok(None)), ready());
    eventually(|| !sessions.state().recovering).await;
    let admission = sessions.admit(game(), ACCOUNT.into()).unwrap();

    let launched = fixture.content.launch(&receipt, admission).await;

    assert_eq!(
        launched,
        Err(Failure::Failed("backend.epic.unauthorized".into()))
    );
    assert_eq!(
        fixture.fake.calls()[..3],
        ["account", "exchange", "account"]
    );
    assert!(
        sessions.state().active.is_none(),
        "a refused launch gives its Reservation back"
    );
}

#[tokio::test]
async fn a_windows_launch_gets_voile_before_it_is_prepared_and_a_mac_launch_gets_none() {
    let fixture = Fixture::new();
    let (events, record) = recorded();
    let runtimes = FakeRuntimes::new(&[], record);
    runtimes.hold();
    let sessions = Sessions::<Epic>::new(
        Box::new(|| Ok(None)),
        Runtimes::from_source(runtimes.clone()),
    );
    eventually(|| !sessions.state().recovering).await;
    let admission = sessions.admit(game(), ACCOUNT.into()).unwrap();
    let canceller = admission.canceller();
    let windows = fixture.installed("Windows");

    let (launched, ()) = tokio::join!(fixture.content.launch(&windows, admission), async {
        eventually(|| !events.lock().unwrap().is_empty()).await;
        canceller.cancel();
    });

    assert_eq!(launched, Err(Failure::Cancelled), "nothing is prepared");
    assert_eq!(*events.lock().unwrap(), ["voile"]);

    // Downloads no longer wait: one the Mac build got would show.
    runtimes.release();
    // A launch code of another account stops the Mac launch once prepared.
    fixture
        .fake
        .accounts
        .lock()
        .unwrap()
        .extend([ACCOUNT.to_string(), STRANGER.to_string()]);
    let admission = sessions.admit(game(), ACCOUNT.into()).unwrap();
    let launched = fixture
        .content
        .launch(&fixture.installed("Mac"), admission)
        .await;
    assert_eq!(
        launched,
        Err(Failure::Failed("backend.epic.unauthorized".into()))
    );
    assert_eq!(*events.lock().unwrap(), ["voile"], "a Mac build needs none");
}

#[test]
fn an_uninstalled_windows_build_loses_its_prefix_and_a_mac_build_has_none() {
    let fixture = Fixture::new();
    let (windows, mac) = (fixture.installed("Windows"), fixture.installed("Mac"));
    let prefix = fixture.locations.wine_prefixes.join("epic-hash-Windows");
    std::fs::create_dir_all(prefix.join("drive_c")).unwrap();

    assert_eq!(fixture.content.remove_prefix(&windows), Some(Ok(())));
    assert!(!prefix.exists());
    assert_eq!(fixture.content.remove_prefix(&mac), None);
    assert_eq!(
        fixture.content.directory_name(&windows),
        epic_kit::client::installation_directory_name("sandbox", "catalog", "game", "Windows")
    );
}

#[tokio::test]
async fn an_older_receipt_learns_its_catalog_item_from_the_installation_itself() {
    let fixture = Fixture::new();
    let mut receipt = fixture.installed("Mac");
    let installed = epic_kit::client::Installation {
        app_name: "game".into(),
        namespace: "sandbox".into(),
        catalog_item_id: "catalog".into(),
        account_id: ACCOUNT.into(),
        directory: receipt.directory.clone(),
        platform: "Mac".into(),
        total_bytes: 100,
        build_version: "1".into(),
        launch_executable: "Game.app".into(),
        launch_command: String::new(),
        deployment_id: None,
        prerequisite: None,
    };
    std::fs::write(
        Path::new(&receipt.directory).join(".obelisk-install.json"),
        serde_json::to_vec(&installed).unwrap(),
    )
    .unwrap();
    receipt.product.catalog_item_id.clear();
    let file = fixture.locations.library.join("epic-installations.json");
    // An older receipt file has no catalog item at all.
    let mut older = serde_json::to_value([&receipt]).unwrap();
    older[0].as_object_mut().unwrap().remove("catalogItemId");
    std::fs::write(&file, serde_json::to_vec(&older).unwrap()).unwrap();
    let sessions = Arc::new(Sessions::<Epic>::new(Box::new(|| Ok(None)), ready()));

    let installations = crate::installations::Installations::new(
        Arc::new(Content::new(
            fixture.fake.clone(),
            Arc::new(|| {}),
            ACCOUNT.into(),
            "fr".into(),
            fixture.locations.clone(),
        )),
        sessions,
        ACCOUNT.to_string(),
        file.clone(),
        fixture.locations.library.clone(),
        fixture.locations.home.clone(),
        crate::installations::Queueing {
            queue: crate::queue::DownloadQueue::new(),
            account: ACCOUNT.into(),
            name: |game: &Store| game.app_name.clone(),
        },
    );

    assert_eq!(
        installations.state().receipts[0].product.catalog_item_id,
        "catalog"
    );
    let saved: Vec<Receipt<Epic>> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        saved[0].product.catalog_item_id, "catalog",
        "the file keeps what it learnt"
    );
}
