use super::{CloudError, CloudFile};
use futures_util::{StreamExt, stream::FuturesUnordered};

/// Each file waits about 200 ms for its CM request before its HTTP body, so
/// only overlapping files makes a save set of hundreds of them fast.
const MAX_DOWNLOADS: usize = 16;
const BATCH_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) trait DownloadPlans {
    type Plan;
    fn prepare(
        &self,
        file: CloudFile,
    ) -> impl Future<Output = Result<Self::Plan, CloudError>> + Send;
}

/// CM requests and HTTP bodies overlap in a sliding window and are consumed
/// as each file completes. Dropping this future cancels every in-flight read;
/// no detached worker can outlive the transfer owner.
pub(crate) async fn download_files<
    S: DownloadPlans,
    D: Future<Output = Result<Vec<u8>, CloudError>>,
>(
    files: &[CloudFile],
    plans: &S,
    download: impl Fn(CloudFile, S::Plan) -> D,
    authorized: &impl Fn() -> bool,
    mut receive: impl FnMut(&CloudFile, Vec<u8>) -> Result<(), CloudError>,
) -> Result<(), CloudError> {
    let download = &download;
    let mut queued = files.iter().peekable();
    let mut pending = FuturesUnordered::new();
    let mut bytes = 0;
    loop {
        // A large file runs alone. Smaller ones overlap without holding the
        // complete save set in memory. Each decoder also bounds transfer bytes.
        while let Some(file) = queued.next_if(|file| {
            pending.is_empty()
                || pending.len() < MAX_DOWNLOADS && bytes + u64::from(file.size) <= BATCH_BYTES
        }) {
            check(authorized)?;
            bytes += u64::from(file.size);
            pending.push(async move {
                let plan = plans.prepare(file.clone()).await?;
                check(authorized)?;
                let read = download(file.clone(), plan).await?;
                check(authorized)?;
                file.verify(&read)?;
                Ok::<_, CloudError>((file, read))
            });
        }
        let Some(result) = pending.next().await else {
            break;
        };
        let (file, read) = result?;
        bytes -= u64::from(file.size);
        check(authorized)?;
        receive(file, read)?;
    }
    check(authorized)
}

fn check(authorized: &impl Fn() -> bool) -> Result<(), CloudError> {
    if authorized() {
        Ok(())
    } else {
        Err(CloudError::Revoked)
    }
}

#[cfg(test)]
mod tests;
