//! Galaxy chunk delivery and atomic installation.
//!
//! The bounded network pump and blocking disk worker follow Epic's installer;
//! path confinement, locking, hashing and progress reporting come from
//! `obelisk_core`. Each file is assembled from its chunks in order: a chunk
//! already on disk, in the file's partial copy or at the same place in its
//! installed copy, is reused once its MD5 matches, so a cancelled Transfer
//! resumes and an update downloads only the chunks that changed.
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use md5::{Digest, Md5};
use obelisk_core::{
    install::{
        self, FileDigest, Md5Part, PARTIAL_DIRECTORY, apply_permissions, checked_file_path,
        ensure_directory, hex, safe_relative_path, unhex,
    },
    transfer::{self, CancelToken, DownloadPhase, DownloadProgress, DownloadSummary, Reporter},
    verification::{VerificationError, VerificationFile},
};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc};

use crate::{
    GogError,
    manifest::{Chunk, Installed, InstalledFile, galaxy_path, relative_target},
};

const CHUNK_CONCURRENCY: usize = 12;
/// Files whose chunks are requested ahead of the one being written.
const FILE_PREFETCH: usize = 8;

/// One CDN a product's chunks come from, as its secure link describes it.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Endpoint {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub url_format: String,
    #[serde(default)]
    pub parameters: serde_json::Map<String, serde_json::Value>,
    /// Higher first.
    #[serde(default)]
    pub priority: i64,
    /// Tried only once every other endpoint failed.
    #[serde(default)]
    pub fallback_only: bool,
}

impl Endpoint {
    /// The URL of the chunk at `galaxy_path`: the endpoint's `path` parameter
    /// extended with it, or, for an endpoint without parameters such as the
    /// redistributables', its URL.
    pub fn chunk_url(&self, galaxy_path: &str) -> Result<Url, GogError> {
        if !self.parameters.contains_key("path") {
            return checked_url(&format!("{}/{galaxy_path}", self.url.trim_end_matches('/')));
        }
        let mut url = self.url_format.clone();
        for (name, value) in &self.parameters {
            let value = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Number(value) => value.to_string(),
                serde_json::Value::Bool(value) => value.to_string(),
                _ => continue,
            };
            let value = match name.as_str() {
                "path" => format!("{}/{galaxy_path}", value.trim_end_matches('/')),
                _ => value,
            };
            url = url.replace(&format!("{{{name}}}"), &value);
        }
        if url.contains(['{', '}']) {
            return Err(GogError::Protocol);
        }
        checked_url(&url)
    }

    /// Whether the endpoint's signature lapsed, or lapses within a minute.
    fn expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|now| now.as_secs())
            .unwrap_or(0);
        self.parameters
            .get("expires_at")
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .is_some_and(|expires| expires <= now + 60)
    }
}

/// Only HTTPS locations without embedded credentials.
pub(crate) fn checked_url(value: &str) -> Result<Url, GogError> {
    let url = Url::parse(value).map_err(|_| GogError::Protocol)?;
    let secure = url.scheme() == "https";
    // Tests serve their CDN on the loopback interface.
    #[cfg(test)]
    let secure = secure || url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
    if !secure
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(GogError::Protocol);
    }
    Ok(url)
}

/// Renews a product's secure link with the account's credentials.
pub(crate) type Renew = Arc<
    dyn Fn(String) -> futures_util::future::BoxFuture<'static, Result<Vec<Endpoint>, GogError>>
        + Send
        + Sync,
>;

/// Each product's CDN endpoints, renewed once they lapse.
pub(crate) struct SecureLinks {
    links: Mutex<HashMap<String, Arc<[Endpoint]>>>,
    renew: Renew,
}

impl SecureLinks {
    pub fn new(renew: Renew) -> Self {
        Self {
            links: Mutex::new(HashMap::new()),
            renew,
        }
    }

    /// The product's endpoints, fetched again when they lapsed or `stale`.
    async fn endpoints(&self, product_id: &str, stale: bool) -> Result<Arc<[Endpoint]>, GogError> {
        let mut links = self.links.lock().await;
        match links.get(product_id) {
            Some(endpoints) if !stale && !endpoints.iter().any(Endpoint::expired) => {
                Ok(Arc::clone(endpoints))
            }
            _ => {
                let mut endpoints = (self.renew)(product_id.to_owned()).await?;
                if endpoints.is_empty() || endpoints.len() > 16 {
                    return Err(GogError::Protocol);
                }
                // Fallbacks last, then the highest priority first.
                endpoints.sort_by_key(|endpoint| (endpoint.fallback_only, -endpoint.priority));
                let endpoints: Arc<[Endpoint]> = endpoints.into();
                links.insert(product_id.to_owned(), Arc::clone(&endpoints));
                Ok(endpoints)
            }
        }
    }
}

/// What the disk worker asks the network for: one chunk, and the secure
/// link that serves it.
#[derive(Clone)]
struct Request {
    source: String,
    chunk: Chunk,
}

type ChunkResult = Result<Vec<u8>, GogError>;

/// Installs `installed`, already validated, into `destination` and returns
/// what the Transfer did.
pub(crate) async fn download(
    http: reqwest::Client,
    links: Arc<SecureLinks>,
    installed: Arc<Installed>,
    key: String,
    destination: &Path,
    cancel: &CancelToken,
    progress: impl Fn(DownloadProgress) + Send + Sync + 'static,
) -> Result<DownloadSummary, GogError> {
    cancel.check()?;
    let reporter = Reporter::new(
        DownloadProgress {
            phase: DownloadPhase::Verifying,
            total_bytes: installed.total_bytes(),
            total_files: u32::try_from(installed.files.len()).map_err(|_| GogError::Protocol)?,
            ..Default::default()
        },
        progress,
    );
    let (requests, receiver) = mpsc::unbounded_channel();
    let (deliveries, results) = mpsc::channel(CHUNK_CONCURRENCY);
    let disk = DiskInstall {
        destination: destination.to_owned(),
        installed,
        key,
        cancel: cancel.clone(),
        reporter: Arc::clone(&reporter),
        requests,
        results,
        planned: Default::default(),
        next_file: 0,
    };
    let cancellation_guard = cancel.cancel_on_drop();
    let (reused_bytes, ()) = tokio::join!(
        cancel.spawn_blocking(move || disk.run()),
        transfer::pump_chunks(receiver, deliveries, CHUNK_CONCURRENCY, cancel, |request| {
            fetch_chunk(&http, &links, request, cancel, &reporter)
        }),
    );
    cancellation_guard.disarm();
    let reused_bytes = reused_bytes.map_err(|_| GogError::Protocol)??;
    Ok(reporter.summary(reused_bytes))
}

async fn fetch_chunk(
    http: &reqwest::Client,
    links: &SecureLinks,
    request: Request,
    cancel: &CancelToken,
    reporter: &Reporter,
) -> ChunkResult {
    let path = galaxy_path(&request.chunk.compressed_md5)?;
    let mut failure = GogError::Network;
    let mut stale = false;
    for attempt in 0..4 {
        cancel.check()?;
        let endpoints = links.endpoints(&request.source, stale).await?;
        stale = false;
        for endpoint in endpoints.iter() {
            let url = endpoint.chunk_url(&path)?;
            reporter.update(|status| status.phase = DownloadPhase::Downloading);
            match fetch(http, url, request.chunk.compressed_size, cancel, reporter).await {
                Ok(bytes) => {
                    let chunk = request.chunk.clone();
                    match cancel
                        .spawn_blocking(move || decode_chunk(&bytes, &chunk))
                        .await
                        .map_err(|_| GogError::Protocol)?
                    {
                        Ok(bytes) => return Ok(bytes),
                        Err(error) => failure = error,
                    }
                }
                Err(GogError::Cancelled) => return Err(GogError::Cancelled),
                // A lapsed signature: the next attempt renews the link.
                Err(error @ GogError::Http(401 | 403 | 410)) => {
                    stale = true;
                    failure = error;
                    break;
                }
                Err(error) => failure = error,
            }
        }
        if attempt < 3 {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))) => {},
                () = cancel.cancelled() => return Err(GogError::Cancelled),
            }
        }
    }
    Err(failure)
}

async fn fetch(
    http: &reqwest::Client,
    url: Url,
    expected: u64,
    cancel: &CancelToken,
    reporter: &Reporter,
) -> ChunkResult {
    let operation = async {
        let mut response = http
            .get(url)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|_| GogError::Network)?;
        if !response.status().is_success() {
            return Err(GogError::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|size| size != expected)
        {
            return Err(GogError::Integrity);
        }
        let mut bytes = Vec::with_capacity(expected as usize);
        while let Some(part) = response.chunk().await.map_err(|_| GogError::Network)? {
            reporter.update(|status| {
                status.network_bytes += part.len() as u64;
                status.content_bytes += part.len() as u64;
            });
            cancel.check()?;
            if bytes.len() as u64 + part.len() as u64 > expected {
                return Err(GogError::Integrity);
            }
            bytes.extend_from_slice(&part);
        }
        Ok(bytes)
    };
    tokio::select! {
        result = operation => result,
        () = cancel.cancelled() => Err(GogError::Cancelled),
    }
}

/// A chunk's zlib stream, checked before and after it inflates.
pub(crate) fn decode_chunk(bytes: &[u8], chunk: &Chunk) -> ChunkResult {
    if bytes.len() as u64 != chunk.compressed_size || !md5_is(bytes, &chunk.compressed_md5) {
        return Err(GogError::Integrity);
    }
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut output = Vec::with_capacity(chunk.size as usize);
    decoder
        .by_ref()
        .take(chunk.size + 1)
        .read_to_end(&mut output)
        .map_err(|_| GogError::Integrity)?;
    if output.len() as u64 != chunk.size || !md5_is(&output, &chunk.md5) {
        return Err(GogError::Integrity);
    }
    Ok(output)
}

/// An installation's file as the shared verification scheduler reads it.
struct Verified<'a> {
    installed: &'a InstalledFile,
    parts: Vec<Md5Part>,
}

impl VerificationFile for Verified<'_> {
    fn name(&self) -> &str {
        &self.installed.path
    }
    fn size(&self) -> u64 {
        self.installed.file.size()
    }
    fn digest(&self) -> FileDigest<'_> {
        FileDigest::Md5Parts(&self.parts)
    }
}

impl From<VerificationError> for GogError {
    fn from(error: VerificationError) -> Self {
        match error {
            VerificationError::Cancelled => Self::Cancelled,
            VerificationError::Install(error) => error.into(),
            VerificationError::SchedulerFailed => Self::Protocol,
        }
    }
}

fn md5_is(bytes: &[u8], expected: &str) -> bool {
    unhex::<16>(expected).is_some_and(|expected| Md5::digest(bytes)[..] == expected)
}

fn parts(chunks: &[Chunk]) -> Result<Vec<Md5Part>, GogError> {
    chunks
        .iter()
        .map(|chunk| {
            Ok(Md5Part {
                size: chunk.size,
                md5: unhex(&chunk.md5).ok_or(GogError::Protocol)?,
            })
        })
        .collect()
}

/// Where one chunk of a file comes from.
#[derive(Clone)]
enum Source {
    /// The same place in a copy already on disk, whose MD5 matched.
    Reused(PathBuf),
    Download,
}

struct PlannedFile {
    index: usize,
    sources: Vec<Source>,
}

struct DiskInstall {
    destination: PathBuf,
    installed: Arc<Installed>,
    /// Names this build's partial files.
    key: String,
    cancel: CancelToken,
    reporter: Arc<Reporter>,
    requests: mpsc::UnboundedSender<Request>,
    results: mpsc::Receiver<ChunkResult>,
    planned: std::collections::VecDeque<PlannedFile>,
    next_file: usize,
}

impl DiskInstall {
    /// Returns the number of verified bytes reused from disk.
    fn run(mut self) -> Result<u64, GogError> {
        self.cancel.check()?;
        fs::create_dir_all(&self.destination)?;
        let root = fs::canonicalize(&self.destination)?;
        let _lock = install::lock_engine(&root)?.ok_or(GogError::AlreadyDownloading)?;
        let installed = Arc::clone(&self.installed);
        let partial_relative = PathBuf::from(PARTIAL_DIRECTORY).join("gog").join(&self.key);
        let partial_root = ensure_directory(&root, &partial_relative)?;
        let checked = installed
            .files
            .iter()
            .map(|installed| {
                Ok(Verified {
                    installed,
                    parts: parts(&installed.file.chunks)?,
                })
            })
            .collect::<Result<Vec<_>, GogError>>()?;
        let verified = obelisk_core::verification::verify_files(
            &root,
            &checked,
            &self.cancel,
            &self.reporter,
        )?;
        let mut reused_bytes = verified.reused_bytes;
        for directory in &installed.directories {
            ensure_directory(&root, &safe_relative_path(directory)?)?;
        }
        self.plan_ahead(&verified.valid, &checked, &partial_root, &root)?;
        let mut ensured_parents: HashSet<PathBuf> = HashSet::new();
        for (index, (file, valid)) in checked.iter().zip(&verified.valid).enumerate() {
            self.cancel.check()?;
            let relative = safe_relative_path(&file.installed.path)?;
            let parent = relative.parent().unwrap_or(Path::new(""));
            if ensured_parents.insert(parent.to_path_buf()) {
                ensure_directory(&root, parent)?;
            }
            let target = checked_file_path(&root, &relative)?;
            if *valid {
                apply_permissions(&target, is_executable(&file.installed.file, &target)?)?;
                continue;
            }
            self.reporter.update(|status| {
                status.current_file.clone_from(&file.installed.path);
            });
            let plan = self.planned.pop_front().ok_or(GogError::Protocol)?;
            if plan.index != index {
                return Err(GogError::Protocol);
            }
            reused_bytes += self.assemble(file, &plan, &partial_root, &target)?;
            self.reporter.update(|status| status.completed_files += 1);
            self.plan_ahead(&verified.valid, &checked, &partial_root, &root)?;
        }
        self.cancel.check()?;
        for link in &installed.links {
            place_link(&root, &link.path, &link.target)?;
        }
        // Partial copies stay only after a failure or a cancellation.
        fs::remove_dir_all(partial_root)?;
        Ok(reused_bytes)
    }

    /// Plans the next few files to repair: which chunks a copy on disk
    /// already holds, and requests the rest, in the order they are written.
    fn plan_ahead(
        &mut self,
        valid: &[bool],
        files: &[Verified<'_>],
        partial_root: &Path,
        root: &Path,
    ) -> Result<(), GogError> {
        while self.planned.len() < FILE_PREFETCH && self.next_file < files.len() {
            self.cancel.check()?;
            let index = self.next_file;
            self.next_file += 1;
            if valid[index] {
                continue;
            }
            let file = &files[index].installed;
            let partial = checked_file_path(partial_root, Path::new(&partial_name(&file.path)))?;
            let target = checked_file_path(root, &safe_relative_path(&file.path)?)?;
            let copies = [partial, target];
            let mut sources = Vec::with_capacity(file.file.chunks.len());
            let mut offset = 0;
            for chunk in &file.file.chunks {
                self.cancel.check()?;
                let reused = copies
                    .iter()
                    .find(|copy| copy_chunk(copy, offset, chunk, &mut io::sink()).unwrap_or(false))
                    .cloned();
                sources.push(match reused {
                    Some(copy) => Source::Reused(copy),
                    None => {
                        self.requests
                            .send(Request {
                                source: file.source.clone(),
                                chunk: chunk.clone(),
                            })
                            .map_err(|_| GogError::Protocol)?;
                        Source::Download
                    }
                });
                offset += chunk.size;
            }
            self.planned.push_back(PlannedFile { index, sources });
        }
        Ok(())
    }

    /// Writes the file's chunks in order into a fresh partial copy, checks
    /// each one, then publishes the file atomically.
    fn assemble(
        &mut self,
        file: &Verified<'_>,
        plan: &PlannedFile,
        partial_root: &Path,
        target: &Path,
    ) -> Result<u64, GogError> {
        let name = partial_name(&file.installed.path);
        let partial = checked_file_path(partial_root, Path::new(&name))?;
        let next = checked_file_path(partial_root, Path::new(&format!("{name}.next")))?;
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&next)?;
        let mut reused_bytes = 0;
        let mut offset = 0;
        for (chunk, source) in file.installed.file.chunks.iter().zip(&plan.sources) {
            self.cancel.check()?;
            match source {
                Source::Download => {
                    self.reporter
                        .update(|status| status.phase = DownloadPhase::Downloading);
                    output.write_all(&self.results.blocking_recv().ok_or(GogError::Protocol)??)?;
                }
                Source::Reused(copy) => {
                    reused_bytes += chunk.size;
                    if !copy_chunk(copy, offset, chunk, &mut output)? {
                        return Err(GogError::Integrity);
                    }
                }
            }
            offset += chunk.size;
            // A partial copy that lasts across a cancellation keeps what was
            // written: the next attempt reuses it.
            self.reporter
                .update(|status| status.completed_bytes += chunk.size);
        }
        self.cancel.check()?;
        output.sync_all()?;
        drop(output);
        fs::rename(&next, &partial)?;
        apply_permissions(&partial, is_executable(&file.installed.file, &partial)?)?;
        self.cancel.check()?;
        fs::rename(&partial, target)?;
        File::open(target.parent().ok_or(GogError::Protocol)?)?.sync_all()?;
        Ok(reused_bytes)
    }
}

/// The partial copy of the file at `path`.
fn partial_name(path: &str) -> String {
    format!("{}.part", hex(&Md5::digest(path.to_lowercase().as_bytes())))
}

/// Streams the chunk at `offset` in `copy` into `output` in 64 KiB reads;
/// `false` when `copy` is too short to hold it or its MD5 differs.
fn copy_chunk(
    copy: &Path,
    offset: u64,
    chunk: &Chunk,
    output: &mut impl Write,
) -> Result<bool, GogError> {
    let expected: [u8; 16] = unhex(&chunk.md5).ok_or(GogError::Protocol)?;
    let mut file = match File::open(copy) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() < offset + chunk.size {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut hasher = Md5::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut remaining = chunk.size;
    while remaining > 0 {
        let count = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        file.read_exact(&mut buffer[..count])?;
        hasher.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    Ok(hasher.finalize()[..] == expected)
}

/// Places the link at `path` to `target`, both from the installation's root,
/// replacing whatever an earlier build left there. The link is relative, so
/// it keeps its target wherever the installation goes.
fn place_link(root: &Path, path: &str, target: &str) -> Result<(), GogError> {
    let relative = safe_relative_path(path)?;
    safe_relative_path(target)?;
    let target = relative_target(path, target);
    let parent = ensure_directory(root, relative.parent().unwrap_or(Path::new("")))?;
    let location = parent.join(relative.file_name().ok_or(GogError::Protocol)?);
    match fs::symlink_metadata(&location) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if fs::read_link(&location)? == Path::new(&target) {
                return Ok(());
            }
            fs::remove_file(&location)?;
        }
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&location)?,
        Ok(_) => fs::remove_file(&location)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    std::os::unix::fs::symlink(&target, &location)?;
    Ok(())
}

/// Galaxy flags executables, but some builds miss the flag on helpers their
/// game starts: programs are also recognised by their header.
fn is_executable(file: &crate::manifest::File, path: &Path) -> std::io::Result<bool> {
    Ok(file.executable() || install::starts_as_program(path)?)
}

#[cfg(test)]
mod tests;
