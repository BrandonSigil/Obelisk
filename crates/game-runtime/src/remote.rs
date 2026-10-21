//! What a Windows game's launch reads anew from GitHub, Voile's feed and the
//! game profiles: fetched within a few seconds, and kept for the launches
//! that find no network.
use std::{fs, io, path::Path, sync::OnceLock, time::Duration};

/// How long a launch waits for what it reads anew before going on with the
/// last one kept.
const WAIT: Duration = Duration::from_secs(3);

/// The body at `url`, over HTTPS, unless it takes longer than `WAIT`. One
/// client serves every read, so a launch reuses the connections of the last.
pub(crate) async fn fetched(url: &str) -> Option<Vec<u8>> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = match CLIENT.get() {
        Some(client) => client,
        None => {
            let client = reqwest::Client::builder().https_only(true).build().ok()?;
            CLIENT.get_or_init(|| client)
        }
    };
    let response = client
        .get(url)
        .timeout(WAIT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    Some(response.bytes().await.ok()?.to_vec())
}

/// Writes `bytes` at `path` in one step, unless they are there already.
pub(crate) fn keep(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|kept| kept == bytes) {
        return Ok(());
    }
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    obelisk_core::write_atomic(path, bytes)
}
