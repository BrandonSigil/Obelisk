//! GOG's part of the installations through their Content interface, with a
//! network the test stands in for and real folders in a temporary library.
use super::*;
use crate::installations::{Content as _, tests::FakeRuntimes};
use std::sync::Mutex;
use tokio::sync::Notify;

const ACCOUNT: &str = "48628349957132247";

/// What the test's GOG network was asked, in order.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
    /// Renewing credentials waits until the test releases it.
    held: Mutex<bool>,
    released: Notify,
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
            Ok(ACCOUNT.into())
        })
    }
    fn download<'a>(
        &'a self,
        request: &'a Download,
        language: &'a str,
    ) -> BoxFuture<'a, Result<gog_kit::client::Installation, Failure>> {
        Box::pin(async move {
            self.call(&format!("download {language}"));
            Ok(gog_kit::client::Installation {
                product_id: request.product_id.clone(),
                user_id: ACCOUNT.into(),
                directory: request
                    .directory
                    .join(format!("gog-{}-{}", request.product_id, request.platform))
                    .to_string_lossy()
                    .into_owned(),
                platform: request.platform.clone(),
                total_bytes: 100,
                build_id: "1".into(),
                version_name: "1.0".into(),
                language: language.into(),
                content_root: String::new(),
                script_interpreter: false,
                products: Vec::new(),
                dependencies: Vec::new(),
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        None
    }
    fn cancel(&self) {
        self.call("cancel");
    }
    fn update_available<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            self.call("update");
            Ok(false)
        })
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
    /// Voile is in place.
    fn new() -> Self {
        Self::with_runtimes(&[Runtime::Voile])
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
        let saves = fake.clone();
        let mut content = Content::new(
            fake.clone(),
            Arc::new(move || saves.call("saved")),
            "fr-FR".into(),
            locations.clone(),
        );
        let calls = fake.clone();
        let runtimes = FakeRuntimes::new(present, move |runtime| calls.call(&runtime));
        content.runtimes = Runtimes::from_source(runtimes.clone());
        Self {
            _root: root,
            locations,
            fake,
            runtimes,
            content,
        }
    }

    fn request(&self, platform: &str) -> Download {
        Download {
            product_id: "1207658924".into(),
            title: "The Witcher".into(),
            platform: platform.into(),
            directory: self.locations.library.join("Games/GOG"),
        }
    }

    fn receipt(&self, platform: &str) -> Receipt {
        let directory = self
            .locations
            .library
            .join(format!("Games/GOG/gog-1207658924-{platform}"));
        std::fs::create_dir_all(&directory).unwrap();
        Receipt {
            product_id: "1207658924".into(),
            name: "The Witcher".into(),
            directory: directory.to_string_lossy().into_owned(),
            platform: platform.into(),
            total_bytes: 100,
            needs_verification: false,
        }
    }
}

#[tokio::test]
async fn a_transfer_saves_renewed_credentials_first_and_installs_the_language() {
    let fixture = Fixture::new();
    let request = fixture.request("windows");
    let receipt = fixture.content.download(&request).await.unwrap();
    assert_eq!(receipt.product_id, "1207658924");
    assert_eq!(receipt.name, "The Witcher");
    assert_eq!(receipt.platform, "windows");
    assert!(!receipt.needs_verification);
    assert_eq!(
        Path::new(&receipt.directory),
        fixture
            .locations
            .library
            .join("Games/GOG/gog-1207658924-windows")
    );
    let calls = fixture.fake.calls();
    assert_eq!(&calls[..3], ["prepare", "saved", "download fr-FR"]);
}

#[tokio::test]
async fn a_transfer_cancelled_while_credentials_renew_never_starts() {
    let fixture = Arc::new(Fixture::new());
    *fixture.fake.held.lock().unwrap() = true;
    let running = {
        let fixture = fixture.clone();
        tokio::spawn(async move {
            let request = fixture.request("macos");
            fixture.content.download(&request).await
        })
    };
    while fixture.fake.calls().is_empty() {
        tokio::task::yield_now().await;
    }
    fixture.content.cancel();
    fixture.fake.released.notify_one();
    assert_eq!(running.await.unwrap(), Err(Failure::Cancelled));
    assert!(
        !fixture
            .fake
            .calls()
            .iter()
            .any(|call| call.starts_with("download"))
    );
}

#[tokio::test]
async fn gog_keeps_no_cloud_saves() {
    let fixture = Fixture::new();
    let (receipt, request) = (fixture.receipt("macos"), fixture.request("macos"));
    assert_eq!(fixture.content.archive(&receipt, &request).await, Ok(()));
    assert_eq!(fixture.content.removed(&receipt, &request).await, Ok(()));
    assert!(
        fixture
            .content
            .indicators(&["1207658924".into()])
            .unwrap()
            .is_empty()
    );
    let control = Arc::new(crate::cloud::Control::new(|_| {}));
    assert_eq!(
        fixture
            .content
            .maintain(&"1207658924".into(), Some((&receipt, &request)), control)
            .await,
        Err(Failure::Failed("backend.gog.unsupported".into()))
    );
}

#[test]
fn only_a_windows_build_runs_in_its_own_prefix() {
    let fixture = Fixture::new();
    let windows = fixture.receipt("windows");
    let configuration = fixture.content.configuration(&windows);
    assert_eq!(
        configuration.prefix.as_deref(),
        Some(
            fixture
                .locations
                .wine_prefixes
                .join("gog-1207658924-windows")
                .as_path()
        )
    );
    assert_eq!(
        configuration.session_directory,
        fixture.locations.gog_sessions
    );
    configuration.validate(ACCOUNT).unwrap();
    assert!(fixture.content.remove_prefix(&windows).is_some());

    let mac = fixture.receipt("macos");
    let configuration = fixture.content.configuration(&mac);
    assert_eq!(configuration.prefix, None);
    configuration.validate(ACCOUNT).unwrap();
    assert!(fixture.content.remove_prefix(&mac).is_none());
    assert_eq!(
        fixture
            .content
            .directory_name(&mac, &fixture.request("macos")),
        "gog-1207658924-macos"
    );
}

#[tokio::test]
async fn a_windows_transfer_gets_voile_before_the_game_and_a_mac_one_does_not() {
    for (platform, downloads) in [
        ("windows", vec!["voile", "download fr-FR"]),
        ("macos", vec!["download fr-FR"]),
    ] {
        let fixture = Fixture::with_runtimes(&[]);

        fixture
            .content
            .download(&fixture.request(platform))
            .await
            .unwrap();

        let calls: Vec<_> = fixture
            .fake
            .calls()
            .into_iter()
            .filter(|call| call == "voile" || call.starts_with("download"))
            .collect();
        assert_eq!(calls, downloads, "{platform}");
    }
}

#[tokio::test]
async fn voile_that_fails_to_download_fails_the_transfer_before_the_game() {
    let fixture = Fixture::with_runtimes(&[]);
    let failure = Failure::Failed("backend.voile.downloadFailed".into());
    fixture.runtimes.fail(failure.clone());

    let downloaded = fixture.content.download(&fixture.request("windows")).await;

    assert_eq!(downloaded, Err(failure));
    assert!(
        !fixture
            .fake
            .calls()
            .iter()
            .any(|call| call.starts_with("download"))
    );
}
