use super::*;
use sha1::{Digest, Sha1};
use std::{
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering},
};

fn files(count: usize) -> Vec<CloudFile> {
    (0..count)
        .map(|i| CloudFile {
            name: i.to_string(),
            sha1: crate::content::hex(&Sha1::digest(b"save")),
            size: 4,
            timestamp: 1,
            persist_state: 0,
            platforms: 7,
            reupload_requested: false,
        })
        .collect()
}

#[derive(Default)]
struct Plans<'a> {
    prepared: Option<&'a Cell<usize>>,
    revoke: Option<&'a Cell<bool>>,
}
impl DownloadPlans for Plans<'_> {
    type Plan = ();
    fn prepare(&self, _: CloudFile) -> impl Future<Output = Result<(), CloudError>> + Send {
        if let Some(count) = self.prepared {
            count.set(count.get() + 1);
        }
        if let Some(authorized) = self.revoke {
            authorized.set(false);
        }
        std::future::ready(Ok(()))
    }
}

struct Active<'a>(&'a Cell<usize>);
impl<'a> Active<'a> {
    fn new(active: &'a Cell<usize>, peak: &Cell<usize>) -> Self {
        active.set(active.get() + 1);
        peak.set(peak.get().max(active.get()));
        Self(active)
    }
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

#[tokio::test]
async fn overlaps_http_reads_with_bounded_concurrency_and_delivers_each_file_once() {
    let active = Cell::new(0);
    let peak = Cell::new(0);
    let prepared = Cell::new(0);
    let mut received = std::collections::BTreeSet::new();
    download_files(
        &files(MAX_DOWNLOADS * 2 + 3),
        &Plans {
            prepared: Some(&prepared),
            ..Default::default()
        },
        |_, ()| async {
            let _active = Active::new(&active, &peak);
            tokio::task::yield_now().await;
            Ok(b"save".to_vec())
        },
        &|| true,
        |file, bytes| {
            assert_eq!(bytes, b"save");
            assert!(received.insert(file.name.clone()));
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(prepared.get(), MAX_DOWNLOADS * 2 + 3);
    assert_eq!(received.len(), MAX_DOWNLOADS * 2 + 3);
    assert_eq!(peak.get(), MAX_DOWNLOADS);
    assert_eq!(active.get(), 0);
}

#[tokio::test]
async fn large_files_are_not_started_together_and_dropping_cancels_every_reader() {
    let active = Cell::new(0);
    let peak = Cell::new(0);
    let prepared = Cell::new(0);
    for large in [false, true] {
        let mut files = files(MAX_DOWNLOADS + 4);
        if large {
            for file in &mut files {
                file.size = BATCH_BYTES as u32 + 1;
            }
        }
        peak.set(0);
        prepared.set(0);
        let plans = Plans {
            prepared: Some(&prepared),
            ..Default::default()
        };
        let operation = download_files(
            &files,
            &plans,
            |_, ()| async {
                let _active = Active::new(&active, &peak);
                std::future::pending::<()>().await;
                Ok(Vec::new())
            },
            &|| true,
            |_, _| panic!("cancelled read must not deliver bytes"),
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), operation)
                .await
                .is_err()
        );
        let expected = if large { 1 } else { MAX_DOWNLOADS };
        assert_eq!(peak.get(), expected);
        assert_eq!(prepared.get(), expected);
        assert_eq!(active.get(), 0);
    }
}

#[tokio::test]
async fn revocation_corruption_and_receiver_failure_stop_the_batch() {
    let authorized = Cell::new(true);
    let requests = Cell::new(0);
    let result = download_files(
        &files(8),
        &Plans {
            revoke: Some(&authorized),
            ..Default::default()
        },
        |_, ()| async {
            requests.set(requests.get() + 1);
            Ok(b"save".to_vec())
        },
        &|| authorized.get(),
        |_, _| panic!("revoked transfer"),
    )
    .await;
    assert!(matches!(result, Err(CloudError::Revoked)));
    assert_eq!(requests.get(), 0);

    let result = download_files(
        &files(8),
        &Plans::default(),
        |_, ()| async { Ok(b"bad!".to_vec()) },
        &|| true,
        |_, _| panic!("unverified bytes"),
    )
    .await;
    assert!(matches!(result, Err(CloudError::Integrity)));

    requests.set(0);
    let result = download_files(
        &files(MAX_DOWNLOADS * 2),
        &Plans {
            prepared: Some(&requests),
            ..Default::default()
        },
        |_, ()| async { Ok(b"save".to_vec()) },
        &|| true,
        |_, _| Err(CloudError::Storage),
    )
    .await;
    assert!(matches!(result, Err(CloudError::Storage)));
    // Nothing past the first window is requested once the receiver failed.
    assert!(requests.get() <= MAX_DOWNLOADS);
}

struct SlowPlans<'a> {
    active: &'a AtomicUsize,
    peak: &'a AtomicUsize,
}
impl DownloadPlans for SlowPlans<'_> {
    type Plan = ();
    async fn prepare(&self, _: CloudFile) -> Result<(), CloudError> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn cm_requests_overlap_instead_of_waiting_for_each_other() {
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let mut received = 0;
    download_files(
        &files(MAX_DOWNLOADS * 2),
        &SlowPlans {
            active: &active,
            peak: &peak,
        },
        |_, ()| async { Ok(b"save".to_vec()) },
        &|| true,
        |_, _| {
            received += 1;
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(received, MAX_DOWNLOADS * 2);
    assert_eq!(peak.load(Ordering::SeqCst), MAX_DOWNLOADS);
}

#[tokio::test]
async fn a_slow_file_does_not_hold_back_the_files_after_it() {
    let started = Cell::new(0);
    let files = files(MAX_DOWNLOADS * 2);
    let plans = Plans::default();
    let operation = download_files(
        &files,
        &plans,
        |file, ()| {
            started.set(started.get() + 1);
            let started = &started;
            async move {
                // The first file ends only once files past the first window
                // have started.
                while file.name == "0" && started.get() <= MAX_DOWNLOADS {
                    tokio::task::yield_now().await;
                }
                Ok(b"save".to_vec())
            }
        },
        &|| true,
        |_, _| Ok(()),
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), operation)
        .await
        .expect("the window refills while a file is slow")
        .unwrap();
    assert_eq!(started.get(), MAX_DOWNLOADS * 2);
}
