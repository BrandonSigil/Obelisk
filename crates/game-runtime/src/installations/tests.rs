//! Installations through their interface, against a storefront session the
//! test drives. Steam stands in for every storefront: the records and their
//! rules are the same for each, and `receipt/tests.rs` covers what differs
//! per storefront.
use super::*;
use crate::{
    runtimes::{Runtimes, tests::FakeRuntimes},
    sessions::{Observation, Supervised},
    steam::{Product, Steam, Store},
    supervisor::{FailureReason, Phase},
};
use std::{
    collections::VecDeque,
    sync::{
        Condvar,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// How a scripted launch ends: behind a supervisor, at once, or failed.
type Launched = Result<Option<Arc<dyn Supervised<Steam>>>, Failure>;

/// What the fake storefront does with each call, in order.
#[derive(Default)]
struct Script {
    /// Each download's outcome; a missing one waits until `release`.
    downloads: VecDeque<Result<(), Failure>>,
    launches: VecDeque<Launched>,
    archive: Option<Failure>,
    prefix: Option<Result<(), String>>,
    held: bool,
    calls: Vec<String>,
    /// Each Transfer, launches' verifications included.
    transfers: Vec<Transfer<Steam>>,
    progress: Option<TransferProgress>,
    /// The games the storefront has a newer build of, until a Transfer
    /// installs it.
    newer: Vec<u32>,
    /// Update checks cannot reach the storefront.
    unreachable: bool,
    /// Update checks wait while the test holds them, with the answer they
    /// had when asked.
    checks_held: bool,
    /// The game each update check asked about.
    checks: Vec<u32>,
    /// Holding the storefront holds only its downloads: launches go on.
    launches_free: bool,
    /// The build Steam picks for a Transfer without a platform; the Mac one
    /// without it.
    picked: Option<String>,
    /// The next Transfer fails as it begins.
    begin_failure: Option<Failure>,
}

struct Fake {
    script: Mutex<Script>,
    released: Condvar,
    cancelled: AtomicBool,
    /// The number of the storefront session's connection of the moment.
    connection: watch::Sender<u64>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(Script::default()),
            released: Condvar::new(),
            cancelled: AtomicBool::new(false),
            connection: watch::channel(0).0,
        })
    }
    fn with<T>(&self, change: impl FnOnce(&mut Script) -> T) -> T {
        change(&mut self.script.lock().unwrap())
    }
    fn calls(&self) -> Vec<String> {
        self.with(|script| script.calls.clone())
    }
    fn hold(&self) {
        self.with(|script| script.held = true);
    }
    fn release(&self) {
        self.with(|script| script.held = false);
        self.released.notify_all();
    }
    fn release_checks(&self) {
        self.with(|script| script.checks_held = false);
        self.released.notify_all();
    }
    /// The storefront session connects again, through a new connection.
    fn reconnects(&self) {
        self.connection.send_modify(|connection| *connection += 1);
    }
    /// Waits while the test holds the storefront, off the async runtime.
    async fn wait(self: &Arc<Self>) {
        self.wait_while(|script| script.held).await;
    }
    /// Waits while `held` and nothing cancelled, off the async runtime.
    async fn wait_while(self: &Arc<Self>, held: fn(&Script) -> bool) {
        let fake = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut script = fake.script.lock().unwrap();
            while held(&script) && !fake.cancelled.load(Ordering::Acquire) {
                script = fake.released.wait(script).unwrap();
            }
        })
        .await
        .unwrap();
    }
}

struct Content(Arc<Fake>);

impl super::Content<Steam> for Content {
    fn begin<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<String, Failure>> {
        Box::pin(async move {
            self.0.with(|script| {
                if let Some(failure) = script.begin_failure.take() {
                    return Err(failure);
                }
                let picked = script.picked.clone().unwrap_or_else(|| "macos".into());
                Ok(transfer.platform.clone().unwrap_or(picked))
            })
        })
    }
    fn download<'a>(
        &'a self,
        transfer: &'a Transfer<Steam>,
    ) -> BoxFuture<'a, Result<Receipt<Steam>, Failure>> {
        Box::pin(async move {
            // Each Transfer starts uncancelled, as a storefront's does.
            self.0.cancelled.store(false, Ordering::Release);
            self.0.with(|script| {
                script.calls.push(format!("download:{}", transfer.game()));
                script.transfers.push(transfer.clone());
                script.progress = Some(TransferProgress {
                    state: TransferState::Downloading,
                    completed_bytes: 50,
                    total_bytes: 100,
                    ..TransferProgress::new(transfer.progress_name())
                });
            });
            self.0.wait().await;
            if self.0.cancelled.load(Ordering::Acquire) {
                self.0.with(|script| {
                    if let Some(progress) = &mut script.progress {
                        progress.state = TransferState::Cancelled;
                    }
                });
                return Err(Failure::Cancelled);
            }
            let outcome = self
                .0
                .with(|script| script.downloads.pop_front())
                .unwrap_or(Ok(()));
            outcome?;
            let platform = transfer.platform.clone().expect("a settled platform");
            let directory = transfer
                .directory
                .join(format!("steam-{}-{platform}", transfer.game()));
            std::fs::create_dir_all(&directory).unwrap();
            self.0.with(|script| {
                if let Some(progress) = &mut script.progress {
                    progress.state = TransferState::Complete;
                    progress.completed_bytes = 100;
                }
                script.newer.retain(|app| *app != transfer.game());
            });
            Ok(Receipt {
                product: transfer.product.clone(),
                name: transfer.name.clone(),
                directory: directory.to_string_lossy().into_owned(),
                platform,
                total_bytes: 100,
                needs_verification: false,
            })
        })
    }
    fn progress(&self) -> Option<TransferProgress> {
        self.0.with(|script| script.progress.clone())
    }
    fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.released.notify_all();
    }
    fn update_available<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            let (answer, held) = self.0.with(|script| {
                script.checks.push(receipt.game());
                let answer = if script.unreachable {
                    Err(Failure::Failed("backend.steam.content.network".into()))
                } else {
                    Ok(script.newer.contains(&receipt.game()))
                };
                (answer, script.checks_held)
            });
            if held {
                self.0.wait_while(|script| script.checks_held).await;
            }
            answer
        })
    }
    fn reconnected(&self, connection: u64) -> BoxFuture<'_, u64> {
        let connections = self.0.connection.subscribe();
        Box::pin(async move {
            match obelisk_core::watch::next_matching(connections, |now| *now != connection).await {
                Ok(now) | Err(now) => now,
            }
        })
    }
    fn launch<'a>(
        &'a self,
        receipt: &'a Receipt<Steam>,
        admission: Admission<Steam>,
        join: Option<&'a crate::steam::Join>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let free = self.0.with(|script| {
                script.calls.push(match join {
                    Some(join) => format!("launch:{}:{join:?}", receipt.game()),
                    None => format!("launch:{}", receipt.game()),
                });
                script.launches_free
            });
            if !free {
                self.0.wait().await;
            }
            let outcome = self
                .0
                .with(|script| script.launches.pop_front())
                .unwrap_or(Err(Failure::Failed(
                    "backend.steam.runtime.startFailed".into(),
                )));
            match outcome {
                Ok(Some(supervisor)) => admission.follow(supervisor).await,
                Ok(None) => Ok(()),
                Err(failure) => Err(failure),
            }
        })
    }
    fn archive<'a>(&'a self, receipt: &'a Receipt<Steam>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.0.with(|script| {
                script.calls.push(format!("archive:{}", receipt.game()));
                script.archive.take().map_or(Ok(()), Err)
            })
        })
    }
    fn removed<'a>(&'a self, receipt: &'a Receipt<Steam>) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.0
                .with(|script| script.calls.push(format!("removed:{}", receipt.game())));
            Ok(())
        })
    }
    fn directory_name(&self, receipt: &Receipt<Steam>) -> String {
        format!("steam-{}-{}", receipt.game(), receipt.platform)
    }
    fn remove_prefix(&self, receipt: &Receipt<Steam>) -> Option<Result<(), String>> {
        (receipt.platform == "windows").then(|| {
            self.0.with(|script| {
                script.calls.push("prefix".into());
                script.prefix.take().unwrap_or(Ok(()))
            })
        })
    }
    /// Asks about a conflict, and ends with the player's answer.
    fn maintain<'a>(
        &'a self,
        game: &'a u32,
        installed: Option<&'a Receipt<Steam>>,
        control: Arc<cloud::Control>,
    ) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            let target = if installed.is_some() {
                "installed"
            } else {
                "uninstalled"
            };
            self.0
                .with(|script| script.calls.push(format!("maintain:{game}:{target}")));
            control.show(cloud::Phase::Syncing, None);
            match control.ask(cloud::Question::Conflict, None).await {
                Ok(choice) => {
                    self.0
                        .with(|script| script.calls.push(format!("answer:{choice:?}")));
                    Ok(())
                }
                Err(_) => Err(Failure::Cancelled),
            }
        })
    }
    fn indicators(&self, games: &[u32]) -> Result<Vec<(u32, cloud_status::Indicator)>, Failure> {
        Ok(games
            .iter()
            .map(|game| {
                (
                    *game,
                    cloud_status::Indicator {
                        state: cloud_status::SavedState::NeverSynced,
                        game_session: false,
                        synced_at: None,
                        pending_upload: false,
                        installation_removed: false,
                    },
                )
            })
            .collect())
    }
}

/// A supervisor whose game runs until the test ends it.
struct Game {
    state: Mutex<Observation<Steam>>,
    changed: Condvar,
    choices: Mutex<Vec<(u64, crate::cloud::Choice)>>,
}
impl Game {
    fn running() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Observation {
                id: "session".into(),
                account: 42,
                store: Store::new(620),
                phase: Phase::Running,
                finished: false,
                failure: None,
                error: None,
                revision: 1,
                reconnecting: false,
                rosetta: None,
                cloud: Default::default(),
                played: None,
            }),
            changed: Condvar::new(),
            choices: Mutex::new(Vec::new()),
        })
    }
    /// The session reaches `phase`, its sync showing `cloud`.
    fn reach(&self, phase: Phase, cloud: cloud::Status) {
        let mut state = self.state.lock().unwrap();
        state.phase = phase;
        state.cloud = cloud;
        state.revision += 1;
        self.changed.notify_all();
    }
    /// The session's sync shows `cloud`.
    fn show(&self, cloud: cloud::Status) {
        let mut state = self.state.lock().unwrap();
        state.cloud = cloud;
        state.revision += 1;
        self.changed.notify_all();
    }
    fn end(&self, failure: Option<&str>) {
        let mut state = self.state.lock().unwrap();
        state.phase = if failure.is_some() {
            Phase::Failed
        } else {
            Phase::Finished
        };
        state.failure = failure.map(|_| FailureReason::Failed);
        state.error = failure.map(Into::into);
        state.finished = true;
        state.revision += 1;
        self.changed.notify_all();
    }
}
impl Supervised<Steam> for Game {
    fn snapshot(&self, after: Option<u64>) -> Result<Observation<Steam>, String> {
        let mut state = self.state.lock().unwrap();
        while after == Some(state.revision) && !state.finished {
            state = self.changed.wait(state).unwrap();
        }
        Ok(state.clone())
    }
    fn cancel(&self) {}
    fn stop(&self) -> Result<(), String> {
        self.end(None);
        Ok(())
    }
    fn quit(&self) -> Result<(), String> {
        Ok(())
    }
    fn force_quit(&self) -> Result<(), String> {
        self.end(None);
        Ok(())
    }
    fn begin(&self) -> Result<(), String> {
        Ok(())
    }
    fn control(&self, _: &u64, _: crate::steam::Control) -> Result<(), String> {
        Ok(())
    }
    fn choose(
        &self,
        _: &u64,
        decision_id: u64,
        choice: crate::cloud::Choice,
    ) -> Result<(), String> {
        self.choices.lock().unwrap().push((decision_id, choice));
        Ok(())
    }
    fn resume_after_rosetta(&self, _: &u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn revoke(&self, _: &u64) -> Result<(), String> {
        Ok(())
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    fake: Arc<Fake>,
    runtimes: Arc<FakeRuntimes>,
    sessions: Arc<Sessions<Steam>>,
    queue: Arc<DownloadQueue>,
    installations: Installations<Steam>,
}

/// Waits until no Transfer of the account runs or waits in the queue.
async fn idle(installations: &Installations<Steam>) {
    eventually(|| {
        let state = installations.state();
        !state.downloading && state.queued.is_empty()
    })
    .await;
}

/// The account's place in `queue`, which names a game by its app id.
fn queueing(queue: &Arc<DownloadQueue>) -> Queueing<Steam> {
    Queueing {
        queue: queue.clone(),
        account: "42".into(),
        name: |game: &u32| game.to_string(),
    }
}

impl Fixture {
    /// An account with Portal 2 installed for macOS when `installed`, and
    /// every runtime in place.
    async fn new(installed: bool) -> Self {
        Self::with_runtimes(installed, &[Runtime::SteamComponents, Runtime::Voile]).await
    }

    /// As `new`, with only the runtimes `present` in place: a download of
    /// another is recorded with the storefront's calls.
    async fn with_runtimes(installed: bool, present: &[Runtime]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let games = root.join("Games/Steam");
        std::fs::create_dir_all(&games).unwrap();
        let file = root.join("steam-installations-42.json");
        if installed {
            let installation = games.join("steam-620-macos");
            std::fs::create_dir_all(&installation).unwrap();
            save(&file, &[receipt(&installation, "macos")]).unwrap();
        }
        let fake = Fake::new();
        let calls = fake.clone();
        let runtimes = FakeRuntimes::new(present, move |runtime| {
            calls.with(|script| script.calls.push(runtime))
        });
        let sessions = Arc::new(Sessions::new(
            Box::new(|| Ok(None)),
            Runtimes::from_source(runtimes.clone()),
        ));
        eventually(|| !sessions.state().recovering).await;
        let queue = DownloadQueue::new();
        let installations = Installations::new(
            Arc::new(Content(fake.clone())),
            sessions.clone(),
            42,
            file,
            root.clone(),
            root.join("home"),
            queueing(&queue),
        );
        Self {
            _root: directory,
            root,
            fake,
            runtimes,
            sessions,
            queue,
            installations,
        }
    }
    fn transfer(&self, platform: &str) -> Transfer<Steam> {
        Transfer {
            product: portal(),
            name: "Portal 2".into(),
            platform: Some(platform.into()),
            directory: self.root.join("Games/Steam"),
        }
    }
    fn offer(&self, platform: &str) -> Offer<Steam> {
        Offer {
            product: portal(),
            name: "Portal 2".into(),
            platform: Some(platform.into()),
        }
    }
    fn receipt(&self) -> Receipt<Steam> {
        self.installations.state().receipts[0].clone()
    }
    fn saved(&self) -> Vec<Receipt<Steam>> {
        load(&self.root.join("steam-installations-42.json")).unwrap()
    }
}

/// Portal 2, with its French depots.
fn portal() -> Product {
    Product {
        app_id: 620,
        language: "french".into(),
    }
}

fn receipt(directory: &Path, platform: &str) -> Receipt<Steam> {
    Receipt {
        product: portal(),
        name: "Portal 2".into(),
        directory: directory.to_string_lossy().into_owned(),
        platform: platform.into(),
        total_bytes: 100,
        needs_verification: false,
    }
}

/// Whether a Transfer or uninstall of `game` may start, as the state shows it.
fn modifiable(installations: &Installations<Steam>, game: u32) -> bool {
    installations.state().gate.modify.allows(&game)
}

/// Whether Cloud maintenance of `game` may start, as the state shows it.
fn maintainable(installations: &Installations<Steam>, game: u32) -> bool {
    installations.state().gate.maintain.allows(&game)
}

pub(crate) async fn eventually(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn receipts_load_from_the_account_file_without_games_removed_outside_obelisk() {
    let fixture = Fixture::new(true).await;
    assert_eq!(fixture.installations.state().receipts.len(), 1);

    std::fs::remove_dir_all(fixture.root.join("Games/Steam/steam-620-macos")).unwrap();
    let file = fixture.root.join("steam-installations-42.json");
    let reloaded = Installations::new(
        Arc::new(Content(Fake::new())),
        fixture.sessions.clone(),
        42,
        file.clone(),
        fixture.root.clone(),
        fixture.root.join("home"),
        queueing(&fixture.queue),
    );
    assert!(reloaded.state().receipts.is_empty());

    std::fs::write(&file, b"not json").unwrap();
    let unreadable = Installations::new(
        Arc::new(Content(Fake::new())),
        fixture.sessions.clone(),
        42,
        file,
        fixture.root.clone(),
        fixture.root.join("home"),
        queueing(&fixture.queue),
    );
    assert_eq!(
        unreadable.state().warning.map(|(_, key)| key).as_deref(),
        Some("model.steam.receiptsUnavailable")
    );
}

#[tokio::test]
async fn a_transfer_reports_its_progress_and_publishes_its_receipt_once_complete() {
    let fixture = Fixture::new(false).await;
    fixture.fake.hold();
    fixture.installations.install(fixture.offer("windows"));
    let state = fixture.installations.state();
    assert!(state.downloading);
    assert_eq!(state.transfer_game, Some(620));
    assert_eq!(state.transfer.unwrap().state, TransferState::Preparing);
    assert!(!modifiable(&fixture.installations, 620));

    eventually(|| {
        fixture
            .installations
            .state()
            .transfer
            .is_some_and(|transfer| transfer.completed_bytes == 50)
    })
    .await;
    fixture.fake.release();
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    assert_eq!(state.transfer.unwrap().state, TransferState::Complete);
    assert_eq!(state.receipts.len(), 1);
    assert_eq!(state.last_download, None);
    assert_eq!(state.report, None);
    assert_eq!(fixture.saved(), state.receipts);
}

#[tokio::test]
async fn an_offer_without_a_platform_shows_its_transfer_at_once_and_keeps_the_build_steam_picked() {
    let fixture = Fixture::new(false).await;
    fixture.fake.hold();
    fixture.installations.install(Offer {
        platform: None,
        ..fixture.offer("macos")
    });
    let state = fixture.installations.state();
    assert!(
        state.queued == [620] || state.transfer_game == Some(620),
        "the game waits on its Transfer at once"
    );
    eventually(|| fixture.installations.state().transfer_game == Some(620)).await;
    let state = fixture.installations.state();
    assert_eq!(state.transfer.unwrap().state, TransferState::Preparing);

    fixture.fake.release();
    idle(&fixture.installations).await;
    let receipt = fixture.receipt();
    assert_eq!(receipt.platform, "macos");
    assert!(receipt.directory.ends_with("steam-620-macos"));

    fixture.fake.hold();
    fixture.installations.install(Offer {
        platform: None,
        ..fixture.offer("macos")
    });
    assert!(
        fixture.receipt().needs_verification,
        "the installed build updates in place"
    );
    fixture.fake.release();
    idle(&fixture.installations).await;
    assert_eq!(fixture.installations.state().receipts.len(), 1);
}

#[tokio::test]
async fn a_failed_transfer_is_reported_and_stays_resumable_and_its_installation_unverified() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| {
        script
            .downloads
            .push_back(Err(Failure::Failed("backend.steam.content.network".into())))
    });
    fixture.installations.install(fixture.offer("macos"));
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    assert_eq!(
        state.report,
        Some(Report::Storefront("backend.steam.content.network".into()))
    );
    assert_eq!(state.transfer.unwrap().state, TransferState::Failed);
    assert_eq!(state.last_download, Some(fixture.transfer("macos")));
    assert!(state.receipts[0].needs_verification);
    assert!(fixture.saved()[0].needs_verification);

    fixture.installations.install(fixture.offer("macos"));
    idle(&fixture.installations).await;
    let state = fixture.installations.state();
    assert_eq!(state.report, None);
    assert!(!state.receipts[0].needs_verification);
    assert_eq!(fixture.fake.calls(), ["download:620", "download:620"]);
}

/// The ids of the receipts a Transfer started over.
fn unverified(receipts: &[Receipt<Steam>]) -> Vec<String> {
    receipts
        .iter()
        .filter(|receipt| receipt.needs_verification)
        .map(Receipt::id)
        .collect()
}

#[tokio::test]
async fn a_transfer_marks_only_the_installation_it_goes_over_for_verification() {
    let fixture = Fixture::new(true).await;
    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;

    fixture.fake.hold();
    fixture.installations.update("620-windows");
    eventually(|| fixture.fake.calls().len() == 2).await;
    assert_eq!(
        unverified(&fixture.installations.state().receipts),
        ["620-windows"]
    );
    assert_eq!(unverified(&fixture.saved()), ["620-windows"]);

    fixture.fake.release();
    idle(&fixture.installations).await;
    assert!(unverified(&fixture.installations.state().receipts).is_empty());
}

#[tokio::test]
async fn a_cancelled_transfer_ends_silently() {
    let fixture = Fixture::new(false).await;
    fixture.fake.hold();
    fixture.installations.install(fixture.offer("macos"));
    eventually(|| fixture.fake.calls().len() == 1).await;
    fixture.installations.cancel(&620);
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    assert_eq!(state.report, None);
    assert_eq!(state.transfer.unwrap().state, TransferState::Cancelled);
    assert!(state.receipts.is_empty());
    assert!(state.last_download.is_some());
}

#[tokio::test]
async fn a_folder_that_cannot_be_created_is_reported_before_any_transfer() {
    let fixture = Fixture::new(false).await;
    let games = fixture.root.join("Games/Steam");
    std::fs::remove_dir_all(&games).unwrap();
    std::fs::write(&games, b"").unwrap();
    fixture.installations.install(fixture.offer("macos"));
    let state = fixture.installations.state();
    assert!(matches!(state.report, Some(Report::DownloadDirectory(_))));
    assert!(!state.downloading);
    assert!(fixture.fake.calls().is_empty());
}

#[tokio::test]
async fn a_launch_goes_straight_to_its_game_then_ends_its_preparation_once_the_game_runs() {
    let fixture = Fixture::new(true).await;
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)))
    });
    fixture.installations.launch("620-macos");
    let state = fixture.installations.state();
    assert!(state.launching);
    assert!(!state.verifying_before_launch);
    assert_eq!(state.launch_game, Some(620));
    assert!(
        fixture.sessions.state().busy(),
        "the Reservation holds from admission"
    );

    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(fixture.fake.calls(), ["launch:620"]);
    assert!(fixture.sessions.state().busy());
    assert!(!modifiable(&fixture.installations, 620));

    game.end(None);
    eventually(|| !fixture.sessions.state().busy()).await;
    eventually(|| fixture.installations.state().launch_game.is_none()).await;
    assert_eq!(fixture.installations.state().report, None);
}

#[tokio::test]
async fn an_unverified_installation_is_verified_before_it_launches_and_a_failure_is_reported() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| {
        script
            .downloads
            .push_back(Err(Failure::Failed("backend.steam.content.network".into())))
    });
    fixture.installations.install(fixture.offer("macos"));
    idle(&fixture.installations).await;

    fixture.installations.launch("620-macos");
    assert!(fixture.installations.state().verifying_before_launch);
    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(
        fixture.fake.calls(),
        ["download:620", "download:620", "launch:620"]
    );
    assert_eq!(
        fixture.installations.state().report,
        Some(Report::Storefront(
            "backend.steam.runtime.startFailed".into()
        ))
    );
    assert!(!fixture.sessions.state().busy());
}

#[tokio::test]
async fn a_join_launches_its_game_joining_what_it_names_after_the_verification() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| {
        script
            .downloads
            .push_back(Err(Failure::Failed("backend.steam.content.network".into())))
    });
    fixture.installations.install(fixture.offer("macos"));
    idle(&fixture.installations).await;

    let lobby = crate::steam::Join::Lobby {
        lobby_id: 109_775_244_421_126_053,
    };
    fixture.installations.join("620-macos", lobby.clone());
    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(
        fixture.fake.calls(),
        [
            "download:620".to_owned(),
            "download:620".to_owned(),
            format!("launch:620:{lobby:?}")
        ]
    );
}

#[tokio::test]
async fn a_launch_installs_the_newer_build_first_unless_the_storefront_cannot_tell() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| script.newer.push(620));
    fixture.installations.launch("620-macos");
    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(fixture.fake.calls(), ["download:620", "launch:620"]);
    assert_eq!(
        fixture.fake.with(|script| script.transfers.clone()),
        [fixture.transfer("macos")],
        "the update stays where the game is"
    );
    assert!(fixture.installations.state().updates.is_empty());

    // Offline, the installed build starts.
    fixture.fake.with(|script| {
        script.newer.push(620);
        script.unreachable = true;
    });
    fixture.installations.launch("620-macos");
    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(
        fixture.fake.calls(),
        ["download:620", "launch:620", "launch:620"]
    );
    assert_eq!(
        fixture.fake.with(|script| script.checks.clone()),
        [620, 620]
    );
}

#[tokio::test]
async fn an_update_check_names_each_installation_with_a_newer_build_until_its_transfer() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| script.newer.push(620));
    fixture.installations.check_updates();
    eventually(|| fixture.installations.state().updates == ["620-macos"]).await;

    fixture.installations.check_updates();
    assert_eq!(
        fixture.fake.with(|script| script.checks.clone()),
        [620],
        "the storefront is asked again only after a while"
    );
    assert_eq!(fixture.fake.calls(), Vec::<String>::new());

    fixture.installations.update("620-macos");
    idle(&fixture.installations).await;
    assert!(fixture.installations.state().updates.is_empty());
}

#[tokio::test]
async fn a_session_that_connects_again_is_asked_for_newer_builds_at_once_after_any_check_under_way()
{
    let fixture = Fixture::new(true).await;
    let checks = || fixture.fake.with(|script| script.checks.clone());
    fixture.fake.with(|script| script.checks_held = true);
    fixture.installations.check_updates();
    eventually(|| checks() == [620]).await;

    // The check under way asked through the old connection.
    fixture.fake.reconnects();
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.fake.with(|script| script.newer.push(620));
    fixture.fake.release_checks();
    eventually(|| fixture.installations.state().updates == ["620-macos"]).await;
    assert_eq!(checks(), [620, 620]);

    fixture.installations.check_updates();
    assert_eq!(checks(), [620, 620], "a check just ran");
    fixture.fake.reconnects();
    eventually(|| checks() == [620, 620, 620]).await;
}

#[tokio::test]
async fn another_games_transfer_leaves_this_launch_open_and_cancelling_it_stops_only_it() {
    let fixture = Fixture::new(true).await;
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)));
        script.launches_free = true;
    });
    fixture.fake.hold();
    fixture.installations.install(Offer {
        product: Product {
            app_id: 730,
            ..portal()
        },
        ..fixture.offer("macos")
    });
    eventually(|| fixture.fake.calls() == ["download:730"]).await;
    assert_eq!(
        fixture.installations.state().gate.launch,
        Opening::Open { except: vec![730] }
    );

    fixture.installations.launch("620-macos");
    eventually(|| !fixture.installations.state().launching).await;
    assert_eq!(fixture.fake.calls(), ["download:730", "launch:620"]);
    assert_eq!(
        fixture.fake.with(|script| script.checks.clone()),
        [620],
        "no newer build: it starts at once"
    );

    fixture.installations.cancel(&620);
    assert!(fixture.installations.state().downloading);
    fixture.installations.cancel(&730);
    idle(&fixture.installations).await;
    assert!(fixture.sessions.state().busy(), "the game keeps running");
    assert_eq!(fixture.installations.state().report, None);
    game.end(None);
    eventually(|| !fixture.sessions.state().busy()).await;
}

#[tokio::test]
async fn a_launch_that_needs_a_transfer_goes_first_and_pauses_the_one_under_way() {
    let fixture = Fixture::new(true).await;
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)));
        script.launches_free = true;
        script.newer.push(620);
    });
    fixture.fake.hold();
    fixture.installations.install(Offer {
        product: Product {
            app_id: 730,
            ..portal()
        },
        ..fixture.offer("macos")
    });
    eventually(|| fixture.fake.calls() == ["download:730"]).await;

    fixture.installations.launch("620-macos");
    eventually(|| fixture.fake.calls() == ["download:730", "download:620"]).await;
    let view = fixture.queue.view();
    assert!(
        view.running
            .is_some_and(|item| item.game == "620" && item.launching)
    );
    assert_eq!(
        view.waiting
            .iter()
            .map(|item| item.game.as_str())
            .collect::<Vec<_>>(),
        ["730"],
        "the download under way waits behind the launch"
    );

    fixture.fake.release();
    eventually(|| fixture.fake.calls().len() == 4).await;
    let calls = fixture.fake.calls();
    assert!(calls[2..].contains(&"launch:620".to_string()));
    assert!(
        calls[2..].contains(&"download:730".to_string()),
        "it resumes"
    );
    idle(&fixture.installations).await;
    game.end(None);
    eventually(|| !fixture.sessions.state().busy()).await;
}

#[tokio::test]
async fn downloads_queue_behind_the_transfer_under_way_and_cancelling_one_takes_it_out() {
    let fixture = Fixture::new(false).await;
    fixture.fake.hold();
    fixture.installations.install(fixture.offer("macos"));
    eventually(|| fixture.fake.calls() == ["download:620"]).await;
    for app_id in [730, 440] {
        fixture.installations.install(Offer {
            product: Product { app_id, ..portal() },
            ..fixture.offer("macos")
        });
    }
    assert_eq!(fixture.installations.state().queued, [730, 440]);
    assert_eq!(fixture.queue.view().waiting.len(), 2);

    fixture.installations.cancel(&730);
    eventually(|| fixture.installations.state().queued == [440]).await;
    fixture.fake.release();
    idle(&fixture.installations).await;
    assert_eq!(fixture.fake.calls(), ["download:620", "download:440"]);
    assert_eq!(fixture.installations.state().receipts.len(), 2);
}

/// Another storefront's installations, whose game runs.
struct OtherStorefront;

impl queue::Owner for OtherStorefront {
    fn ready(&self, _: u64) -> bool {
        false
    }
    fn start(self: Arc<Self>, _: u64) {}
    fn stop(&self, _: u64) {}
    fn removed(&self, _: u64) {}
}

#[tokio::test]
async fn a_running_game_holds_the_queued_automatic_updates() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| script.newer.push(620));
    let other: Arc<dyn queue::Owner> = Arc::new(OtherStorefront);
    fixture.queue.hold(&other, true);
    fixture.installations.set_automatic_updates(true);
    fixture.installations.check_updates();
    eventually(|| fixture.installations.state().queued == [620]).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fixture.fake.calls(), Vec::<String>::new());
    assert!(fixture.queue.view().waiting[0].automatic);

    fixture.queue.hold(&other, false);
    eventually(|| fixture.fake.calls() == ["download:620"]).await;
    idle(&fixture.installations).await;
    assert!(fixture.installations.state().updates.is_empty());
}

#[tokio::test]
async fn automatic_updates_install_each_newer_build_once_a_game_no_longer_runs() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| script.newer.push(620));
    let admission = fixture.sessions.admit(620, 42).unwrap();
    fixture.installations.set_automatic_updates(true);
    fixture.installations.check_updates();
    eventually(|| fixture.installations.state().updates == ["620-macos"]).await;
    assert_eq!(fixture.fake.calls(), Vec::<String>::new(), "the game runs");

    drop(admission);
    eventually(|| fixture.fake.calls() == ["download:620"]).await;
    eventually(|| fixture.installations.state().updates.is_empty()).await;
    assert_eq!(
        fixture.fake.with(|script| script.transfers.clone()),
        [fixture.transfer("macos")]
    );
}

#[tokio::test]
async fn an_automatic_update_pauses_while_they_are_off_then_resumes() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| script.newer.push(620));
    fixture.fake.hold();
    fixture.installations.set_automatic_updates(true);
    fixture.installations.check_updates();
    eventually(|| fixture.fake.calls() == ["download:620"]).await;

    // A game of any storefront starts.
    fixture.installations.set_automatic_updates(false);
    idle(&fixture.installations).await;
    let state = fixture.installations.state();
    assert_eq!(state.transfer.unwrap().state, TransferState::Cancelled);
    assert_eq!(state.report, None);
    assert_eq!(state.updates, ["620-macos"]);

    fixture.fake.release();
    fixture.fake.cancelled.store(false, Ordering::Release);
    fixture.installations.set_automatic_updates(true);
    eventually(|| fixture.fake.calls() == ["download:620", "download:620"]).await;
    eventually(|| fixture.installations.state().updates.is_empty()).await;
}

#[tokio::test]
async fn turning_automatic_updates_off_leaves_the_players_own_transfer_alone() {
    let fixture = Fixture::new(true).await;
    fixture.fake.hold();
    fixture.installations.update("620-macos");
    eventually(|| fixture.fake.calls() == ["download:620"]).await;
    fixture.installations.set_automatic_updates(false);
    assert!(fixture.installations.state().downloading);
    fixture.fake.release();
    idle(&fixture.installations).await;
    assert_eq!(
        fixture.installations.state().transfer.unwrap().state,
        TransferState::Complete
    );
}

#[tokio::test]
async fn a_failed_automatic_update_shows_on_its_game_only_and_waits_for_the_next_check() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| {
        script.newer.push(620);
        script
            .downloads
            .push_back(Err(Failure::Failed("backend.steam.content.network".into())));
    });
    fixture.installations.set_automatic_updates(true);
    fixture.installations.check_updates();
    eventually(|| fixture.fake.calls() == ["download:620"]).await;
    idle(&fixture.installations).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let state = fixture.installations.state();
    assert_eq!(
        fixture.fake.calls(),
        ["download:620"],
        "not retried at once"
    );
    assert_eq!(state.report, None);
    assert_eq!(state.transfer.unwrap().state, TransferState::Failed);
    assert_eq!(state.updates, ["620-macos"]);
}

#[tokio::test]
async fn cancelling_a_launch_still_preparing_never_hands_the_game_off() {
    let fixture = Fixture::new(true).await;
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)))
    });
    fixture.fake.hold();
    fixture.installations.launch("620-macos");
    eventually(|| fixture.fake.calls() == ["launch:620"]).await;

    fixture.installations.cancel(&620);
    fixture.fake.release();
    eventually(|| !fixture.installations.state().launching).await;
    eventually(|| !fixture.sessions.state().busy()).await;
    assert_eq!(fixture.installations.state().report, None);
    assert!(modifiable(&fixture.installations, 620));
}

#[tokio::test]
async fn nothing_touches_an_installation_its_game_session_or_another_action_holds() {
    let fixture = Fixture::new(true).await;
    let admission = fixture.sessions.admit(620, 42).unwrap();
    fixture.installations.install(fixture.offer("macos"));
    fixture.installations.uninstall("620-macos");
    fixture.installations.launch("620-macos");
    let state = fixture.installations.state();
    assert!(!state.downloading && state.uninstall_game.is_none() && !state.launching);
    drop(admission);

    fixture.installations.set_diagnostic(true);
    assert!(!modifiable(&fixture.installations, 620));
    fixture.installations.set_diagnostic(false);
    assert!(modifiable(&fixture.installations, 620));
    assert!(fixture.fake.calls().is_empty());
}

#[tokio::test]
async fn an_uninstall_archives_removes_records_and_deletes_the_prefix_in_order() {
    let fixture = Fixture::new(false).await;
    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;
    let receipt = fixture.receipt();

    fixture.installations.uninstall("620-windows");
    assert!(fixture.installations.state().uninstall_game.is_some());
    eventually(|| fixture.installations.state().uninstall_game.is_none()).await;

    let state = fixture.installations.state();
    assert!(state.receipts.is_empty());
    assert_eq!(state.transfer, None);
    assert_eq!(state.report, None);
    assert!(!Path::new(&receipt.directory).exists());
    assert!(fixture.saved().is_empty());
    assert_eq!(
        fixture.fake.calls(),
        ["download:620", "archive:620", "removed:620", "prefix"]
    );
}

#[tokio::test]
async fn a_failed_archive_keeps_the_game_and_a_lost_prefix_is_only_a_warning() {
    let fixture = Fixture::new(true).await;
    fixture.fake.with(|script| {
        script.archive = Some(Failure::Failed("backend.steam.cloud.network".into()))
    });
    let receipt = fixture.receipt();
    fixture.installations.uninstall("620-macos");
    eventually(|| fixture.installations.state().uninstall_game.is_none()).await;
    let state = fixture.installations.state();
    assert_eq!(
        state.report,
        Some(Report::Uninstall("backend.steam.cloud.network".into()))
    );
    assert_eq!(state.receipts, [receipt]);
    assert!(Path::new(&state.receipts[0].directory).is_dir());

    let fixture = Fixture::new(false).await;
    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;
    fixture
        .fake
        .with(|script| script.prefix = Some(Err("busy".into())));
    fixture.installations.uninstall("620-windows");
    eventually(|| fixture.installations.state().uninstall_game.is_none()).await;
    let state = fixture.installations.state();
    assert!(state.receipts.is_empty());
    assert_eq!(
        state.warning.map(|(_, key)| key).as_deref(),
        Some("model.installations.prefixNotRemoved")
    );
}

#[tokio::test]
async fn a_removal_is_recorded_even_when_the_receipts_cannot_be_saved_after_it() {
    let fixture = Fixture::new(true).await;
    let receipt = fixture.receipt();
    let file = fixture.root.join("steam-installations-42.json");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(&file).unwrap();

    fixture.installations.uninstall("620-macos");
    eventually(|| fixture.installations.state().uninstall_game.is_none()).await;

    let state = fixture.installations.state();
    assert!(!Path::new(&receipt.directory).exists());
    assert!(
        state.receipts.is_empty(),
        "the vanished folder leaves the receipts"
    );
    assert!(matches!(state.report, Some(Report::Uninstall(_))));
    assert_eq!(fixture.fake.calls(), ["archive:620", "removed:620"]);
}

#[tokio::test]
async fn an_uninstall_forgets_only_the_stopped_transfer_of_its_own_platform() {
    let fixture = Fixture::new(true).await;
    let installations = &fixture.installations;
    let fail = || {
        fixture.fake.with(|script| {
            script
                .downloads
                .push_back(Err(Failure::Failed("backend.steam.content.network".into())))
        })
    };
    let uninstalled = || installations.state().uninstall_game.is_none();
    fail();
    installations.install(fixture.offer("windows"));
    idle(installations).await;
    assert_eq!(
        installations.state().last_download,
        Some(fixture.transfer("windows"))
    );

    installations.uninstall("620-macos");
    eventually(uninstalled).await;
    assert!(installations.state().receipts.is_empty());
    assert_eq!(
        installations.state().last_download,
        Some(fixture.transfer("windows")),
        "the other platform's Transfer still resumes"
    );

    installations.install(fixture.offer("windows"));
    idle(installations).await;
    fail();
    installations.update("620-windows");
    idle(installations).await;
    assert_eq!(
        installations.state().last_download,
        Some(fixture.transfer("windows"))
    );
    installations.uninstall("620-windows");
    eventually(uninstalled).await;
    assert_eq!(installations.state().last_download, None);
}

#[tokio::test]
async fn signing_out_stops_the_transfer_and_later_actions() {
    let fixture = Fixture::new(false).await;
    fixture.fake.hold();
    fixture.installations.install(fixture.offer("macos"));
    eventually(|| fixture.fake.calls().len() == 1).await;
    fixture.installations.signed_out();
    idle(&fixture.installations).await;
    assert_eq!(fixture.installations.state().report, None);
    fixture.installations.install(fixture.offer("macos"));
    assert!(!fixture.installations.state().downloading);
}

/// The conflict a sync asks about, with its choices.
fn conflict(decision_id: u64) -> cloud::Status {
    cloud::Status {
        phase: cloud::Phase::Conflict,
        decision_id,
        error: None,
        allowed_choices: vec![
            cloud::Choice::Local,
            cloud::Choice::Cloud,
            cloud::Choice::Cancel,
        ],
    }
}

#[tokio::test]
async fn cloud_maintenance_holds_every_reservation_and_asks_its_question_until_it_ends() {
    let fixture = Fixture::new(true).await;
    let installations = &fixture.installations;
    assert!(maintainable(installations, 620));
    installations.maintain_cloud(620, Some("620-macos"));

    let state = installations.state();
    assert_eq!(
        state
            .maintenance
            .as_ref()
            .map(|maintenance| maintenance.game),
        Some(620)
    );
    assert!(!modifiable(installations, 620));
    assert!(
        !modifiable(installations, 730),
        "its idle guard keeps every launch out"
    );
    assert!(
        !maintainable(installations, 730),
        "one Cloud operation at a time"
    );
    installations.install(fixture.offer("macos"));
    installations.launch("620-macos");
    assert!(!installations.state().downloading && !installations.state().launching);

    eventually(|| {
        installations
            .state()
            .maintenance
            .is_some_and(|maintenance| maintenance.cloud.phase == cloud::Phase::Conflict)
    })
    .await;
    let question = installations.state().maintenance.unwrap().cloud;
    assert_eq!(
        question.allowed_choices,
        conflict(question.decision_id).allowed_choices
    );
    assert!(
        installations
            .answer_cloud(&620, question.decision_id + 1, cloud::Choice::Local)
            .is_err(),
        "an answer binds to the question shown"
    );
    assert!(
        installations
            .answer_cloud(&730, question.decision_id, cloud::Choice::Local)
            .is_err()
    );
    installations
        .answer_cloud(&620, question.decision_id, cloud::Choice::Local)
        .unwrap();

    eventually(|| installations.state().maintenance.is_none()).await;
    assert_eq!(
        fixture.fake.calls(),
        ["maintain:620:installed", "answer:Local"]
    );
    assert!(modifiable(installations, 620));
    assert!(
        maintainable(installations, 620),
        "Sync now starts the next attempt"
    );
}

#[tokio::test]
async fn an_uninstalled_game_syncs_from_its_saved_context_and_a_stranger_installation_is_refused() {
    let fixture = Fixture::new(true).await;
    fixture
        .installations
        .maintain_cloud(620, Some("620-windows"));
    fixture.installations.maintain_cloud(730, Some("620-macos"));
    assert!(
        fixture.installations.state().maintenance.is_none(),
        "an installation the account lacks, or another game's, is refused"
    );

    fixture.installations.maintain_cloud(730, None);
    eventually(|| fixture.fake.calls() == ["maintain:730:uninstalled"]).await;
    fixture.installations.signed_out();
    eventually(|| fixture.installations.state().maintenance.is_none()).await;
    assert_eq!(
        fixture.fake.calls(),
        ["maintain:730:uninstalled"],
        "signing out cancels it"
    );
}

#[tokio::test]
async fn a_game_sessions_cloud_question_is_answered_through_the_same_call() {
    let fixture = Fixture::new(true).await;
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)))
    });
    fixture.installations.launch("620-macos");
    eventually(|| !fixture.installations.state().launching).await;
    assert!(
        !maintainable(&fixture.installations, 730),
        "a Game session keeps Cloud maintenance closed"
    );

    game.show(conflict(7));
    eventually(|| {
        fixture
            .sessions
            .state()
            .active
            .is_some_and(|active| active.cloud.decision_id == 7)
    })
    .await;
    assert!(
        fixture
            .installations
            .answer_cloud(&620, 6, cloud::Choice::Cloud)
            .is_err()
    );
    fixture
        .installations
        .answer_cloud(&620, 7, cloud::Choice::Cloud)
        .unwrap();
    assert_eq!(*game.choices.lock().unwrap(), [(7, cloud::Choice::Cloud)]);
    game.end(None);
    eventually(|| !fixture.sessions.state().busy()).await;
}

#[tokio::test]
async fn cloud_indicators_come_from_the_storefronts_journals() {
    let fixture = Fixture::new(false).await;
    let indicators = fixture.installations.cloud_indicators(&[620]).unwrap();
    assert_eq!(indicators.len(), 1);
    assert_eq!(indicators[0].1.state, cloud_status::SavedState::NeverSynced);
}

#[tokio::test]
async fn the_gate_shows_what_may_start_and_follows_every_game_session() {
    let fixture = Fixture::new(true).await;
    let open = Gate {
        modify: Opening::Open { except: vec![] },
        uninstall: Opening::Open { except: vec![] },
        launch: Opening::Open { except: vec![] },
        maintain: Opening::Open { except: vec![] },
    };
    assert_eq!(fixture.installations.state().gate, open);

    // A Game session holds its own game's files and every Cloud operation.
    let admission = fixture.sessions.admit(620, 42).unwrap();
    let held = Gate {
        modify: Opening::Open { except: vec![620] },
        uninstall: Opening::Open { except: vec![620] },
        launch: Opening::Closed,
        maintain: Opening::Closed,
    };
    eventually(|| fixture.installations.state().gate == held).await;
    drop(admission);
    eventually(|| fixture.installations.state().gate == open).await;

    // A Transfer holds its own game and every uninstall: other games still
    // queue and launch.
    fixture.fake.hold();
    fixture.installations.install(fixture.offer("windows"));
    assert_eq!(
        fixture.installations.state().gate,
        Gate {
            modify: Opening::Open { except: vec![620] },
            uninstall: Opening::Closed,
            launch: Opening::Open { except: vec![620] },
            maintain: Opening::Open { except: vec![620] },
        }
    );
    fixture.fake.release();
    eventually(|| fixture.installations.state().gate == open).await;

    fixture.installations.set_diagnostic(true);
    assert_eq!(
        fixture.installations.state().gate,
        Gate {
            modify: Opening::Closed,
            uninstall: Opening::Closed,
            launch: Opening::Closed,
            maintain: Opening::Closed,
        }
    );
}

#[tokio::test]
async fn a_new_installation_goes_to_the_library_and_an_update_or_launch_stays_where_it_is() {
    let fixture = Fixture::new(false).await;
    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;
    let transfers = fixture.fake.with(|script| script.transfers.clone());
    assert_eq!(transfers, [fixture.transfer("windows")]);

    // An installation Obelisk made in another folder keeps it.
    let previous = fixture.root.join("Previous location");
    let installed = previous.join("steam-620-macos");
    std::fs::create_dir_all(&installed).unwrap();
    let file = fixture.root.join("steam-installations-43.json");
    let mut receipt = receipt(&installed, "macos");
    receipt.product.language = "english".into();
    receipt.needs_verification = true;
    save(&file, &[receipt]).unwrap();
    let fake = Fake::new();
    let moved = Installations::new(
        Arc::new(Content(fake.clone())),
        fixture.sessions.clone(),
        43,
        file,
        fixture.root.clone(),
        fixture.root.join("home"),
        queueing(&DownloadQueue::new()),
    );
    moved.launch("620-macos");
    eventually(|| !moved.state().launching).await;
    moved.update("620-macos");
    idle(&moved).await;
    moved.install(Offer {
        platform: Some("macos".into()),
        ..fixture.offer("macos")
    });
    eventually(|| fake.calls().len() == 4).await;
    let update = Transfer {
        product: Product {
            app_id: 620,
            language: "english".into(),
        },
        directory: previous,
        ..fixture.transfer("macos")
    };
    assert_eq!(
        fake.with(|script| script.transfers.clone()),
        [update.clone(), update.clone(), update],
        "a launch's verification, an update and an offer of the installed build"
    );
    moved.update("620-windows");
    assert_eq!(
        fake.calls().len(),
        4,
        "the account has no such installation"
    );

    idle(&moved).await;
    moved.install(fixture.offer("windows"));
    eventually(|| fake.calls().len() == 5).await;
    assert_eq!(
        fake.with(|script| script.transfers.last().cloned()),
        Some(fixture.transfer("windows")),
        "another platform installs in the library, beside the build in Previous location"
    );
}

#[tokio::test]
async fn the_account_asks_one_cloud_question_whichever_sync_asks_it_until_it_is_answered() {
    let fixture = Fixture::new(true).await;
    let installations = &fixture.installations;

    // Maintenance asks about its conflict.
    installations.maintain_cloud(620, Some("620-macos"));
    let sync = || installations.state().cloud_sync;
    eventually(|| sync().is_some_and(|sync| sync.question.is_some())).await;
    let maintenance = sync().unwrap();
    assert_eq!(maintenance.kind, SyncKind::Maintenance);
    assert_eq!(maintenance.game, 620);
    assert!(!maintenance.working && !maintenance.cancellable);
    let question = maintenance.question.unwrap();
    assert_eq!(question.kind, QuestionKind::Conflict);
    installations
        .answer_cloud(&620, question.decision_id, cloud::Choice::Local)
        .unwrap();
    eventually(|| sync().is_none()).await;

    // A launch's check can be cancelled while it works.
    let game = Game::running();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(game.clone() as Arc<dyn Supervised<Steam>>)))
    });
    installations.launch("620-macos");
    let checking = cloud::Status {
        phase: cloud::Phase::Checking,
        decision_id: 6,
        ..Default::default()
    };
    game.reach(Phase::Preparing, checking);
    eventually(|| sync().is_some_and(|sync| sync.working)).await;
    let launch = sync().unwrap();
    assert_eq!(launch.kind, SyncKind::Launch);
    assert!(launch.cancellable);
    assert_eq!(launch.question, None, "a check asks nothing");

    // The exit's conflict closes with its answer, until the sync asks again.
    game.reach(Phase::Finalizing, conflict(7));
    eventually(|| sync().is_some_and(|sync| sync.question.is_some())).await;
    let exit = sync().unwrap();
    assert_eq!(exit.kind, SyncKind::Exit);
    assert!(!exit.cancellable);
    installations
        .answer_cloud(&620, 7, cloud::Choice::Cloud)
        .unwrap();
    assert_eq!(
        sync().unwrap().question,
        None,
        "the supervisor still shows it"
    );
    assert_eq!(*game.choices.lock().unwrap(), [(7, cloud::Choice::Cloud)]);
    game.show(conflict(8));
    eventually(|| sync().is_some_and(|sync| sync.question.is_some())).await;

    game.end(None);
    eventually(|| sync().is_none()).await;

    // A new session's decisions start again: its question shows.
    let next = Game::running();
    next.state.lock().unwrap().id = "next session".into();
    fixture.fake.with(|script| {
        script
            .launches
            .push_back(Ok(Some(next.clone() as Arc<dyn Supervised<Steam>>)))
    });
    installations.launch("620-macos");
    next.reach(Phase::Finalizing, conflict(7));
    eventually(|| sync().is_some_and(|sync| sync.question.is_some())).await;

    // A revoked account asks nothing.
    fixture.sessions.revoke(42);
    eventually(|| sync().is_some_and(|sync| sync.question.is_none())).await;
    next.end(None);
    eventually(|| sync().is_none()).await;
}

#[tokio::test]
async fn a_windows_transfer_gets_its_runtimes_before_its_files() {
    for (platform, calls) in [
        ("windows", vec!["steam components", "voile", "download:620"]),
        ("macos", vec!["steam components", "download:620"]),
    ] {
        let fixture = Fixture::with_runtimes(false, &[]).await;

        fixture.installations.install(fixture.offer(platform));
        idle(&fixture.installations).await;

        assert_eq!(fixture.fake.calls(), calls, "{platform}");
        assert_eq!(fixture.installations.state().receipts.len(), 1);
    }
}

#[tokio::test]
async fn a_transfer_without_a_platform_gets_the_runtimes_of_the_build_steam_picked() {
    let fixture = Fixture::with_runtimes(false, &[Runtime::SteamComponents]).await;
    fixture
        .fake
        .with(|script| script.picked = Some("windows".into()));

    fixture.installations.install(Offer {
        platform: None,
        ..fixture.offer("windows")
    });
    idle(&fixture.installations).await;

    assert_eq!(fixture.fake.calls(), ["voile", "download:620"]);
    assert_eq!(
        fixture.fake.with(|script| script.transfers.clone()),
        [fixture.transfer("windows")],
        "the game's files are the build Steam picked"
    );
    assert_eq!(fixture.receipt().platform, "windows");
}

#[tokio::test]
async fn a_runtime_download_shows_as_the_transfer_and_cancelling_the_transfer_stops_it_before_its_files()
 {
    let fixture = Fixture::with_runtimes(false, &[Runtime::SteamComponents]).await;
    fixture.runtimes.hold();
    fixture.installations.install(fixture.offer("windows"));
    eventually(|| {
        fixture
            .installations
            .state()
            .transfer
            .is_some_and(|transfer| transfer.message == "model.game.installingVoile")
    })
    .await;
    let shown = fixture.installations.state().transfer.unwrap();
    assert_eq!(shown.game, "620");
    assert_eq!(shown.state, TransferState::Downloading);
    assert_eq!((shown.completed_bytes, shown.total_bytes), (5, 10));

    fixture.installations.cancel(&620);
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    let stopped = state.transfer.unwrap();
    assert_eq!(stopped.state, TransferState::Cancelled);
    // Voile's bytes stay out of the stopped Transfer, where they would read
    // as the game's.
    assert_eq!((stopped.completed_bytes, stopped.total_bytes), (0, 0));
    assert_eq!(
        fixture.fake.calls(),
        ["voile"],
        "no game file was asked for"
    );
    assert!(!fixture.runtimes.in_place().contains(&Runtime::Voile));
    assert_eq!(state.report, None);
}

#[tokio::test]
async fn a_runtime_that_fails_to_download_fails_the_transfer_with_its_key_before_its_files() {
    let fixture = Fixture::with_runtimes(false, &[Runtime::SteamComponents]).await;
    fixture
        .runtimes
        .fail(Failure::Failed("backend.voile.downloadFailed".into()));

    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    assert_eq!(
        state.report,
        Some(Report::Storefront("backend.voile.downloadFailed".into()))
    );
    let stopped = state.transfer.unwrap();
    assert_eq!(stopped.state, TransferState::Failed);
    assert_eq!(stopped.message, "backend.voile.downloadFailed");
    assert_eq!((stopped.completed_bytes, stopped.total_bytes), (0, 0));
    assert_eq!(fixture.fake.calls(), ["voile"]);
    assert_eq!(state.last_download, Some(fixture.transfer("windows")));
}

#[tokio::test]
async fn a_transfer_whose_begin_fails_gets_no_runtime_and_no_files() {
    let fixture = Fixture::with_runtimes(false, &[]).await;
    fixture.fake.with(|script| {
        script.begin_failure = Some(Failure::Failed("backend.steam.content.network".into()))
    });

    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;

    let state = fixture.installations.state();
    assert_eq!(
        state.report,
        Some(Report::Storefront("backend.steam.content.network".into()))
    );
    assert_eq!(state.transfer.unwrap().state, TransferState::Failed);
    assert_eq!(fixture.fake.calls(), Vec::<String>::new());
    assert!(fixture.runtimes.in_place().is_empty());
}

#[tokio::test]
async fn the_transfer_after_one_cancelled_during_its_runtimes_gets_them_again() {
    let fixture = Fixture::with_runtimes(false, &[Runtime::SteamComponents]).await;
    fixture.runtimes.hold();
    fixture.installations.install(fixture.offer("windows"));
    eventually(|| fixture.fake.calls() == ["voile"]).await;
    fixture.installations.cancel(&620);
    idle(&fixture.installations).await;

    fixture.runtimes.release();
    fixture.installations.install(fixture.offer("windows"));
    idle(&fixture.installations).await;

    assert_eq!(fixture.fake.calls(), ["voile", "voile", "download:620"]);
    let state = fixture.installations.state();
    assert_eq!(state.transfer.unwrap().state, TransferState::Complete);
    assert_eq!(state.receipts.len(), 1);
}
