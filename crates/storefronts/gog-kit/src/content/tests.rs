use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    sync::Mutex as StdMutex,
    thread,
};

use futures_util::FutureExt;
use serde_json::json;

use super::*;
use crate::manifest::{DepotManifest, Installed};

/// A CDN on the loopback interface that serves fixed chunks and records
/// every request.
struct Cdn {
    base: String,
    requests: Arc<StdMutex<Vec<String>>>,
}

impl Cdn {
    fn new(responses: HashMap<String, Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/store", listener.local_addr().unwrap());
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let recorded = requests.clone();
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                serve(socket, &responses, &recorded);
            }
        });
        Self { base, requests }
    }

    fn requests(&self) -> Vec<String> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

fn serve(
    socket: TcpStream,
    responses: &HashMap<String, Vec<u8>>,
    requests: &StdMutex<Vec<String>>,
) {
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap() <= 2 {
            break;
        }
    }
    requests.lock().unwrap().push(path.clone());
    let mut socket = socket;
    match responses.get(&path) {
        Some(body) => {
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(body).unwrap();
        }
        None => write!(
            socket,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap(),
    }
}

fn md5_hex(bytes: &[u8]) -> String {
    hex(&Md5::digest(bytes))
}

fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// A depot whose files are made of `chunks`, served by the CDN it returns.
struct Fixture {
    cdn: Cdn,
    installed: Arc<Installed>,
    chunks: HashMap<&'static str, (Vec<u8>, String)>,
}

fn fixture() -> Fixture {
    let parts: [(&str, &[u8]); 3] = [
        ("a", b"first part "),
        ("b", b"second"),
        ("tool", b"\xcf\xfa\xed\xfe tool"),
    ];
    let mut responses = HashMap::new();
    let mut chunks = HashMap::new();
    let mut described = HashMap::new();
    for (name, bytes) in parts {
        let packed = compressed(bytes);
        let path = format!("/store/{}", galaxy_path(&md5_hex(&packed)).unwrap());
        responses.insert(path.clone(), packed.clone());
        chunks.insert(name, (bytes.to_vec(), path));
        described.insert(
            name,
            json!({"md5": md5_hex(bytes), "size": bytes.len(), "compressedMd5": md5_hex(&packed),
                   "compressedSize": packed.len()}),
        );
    }
    let manifest: DepotManifest = serde_json::from_value(json!({"depot": {"items": [
        {"type": "DepotFile", "path": "Game\\game.exe", "chunks": [described["a"], described["b"]]},
        {"type": "DepotFile", "path": "empty.txt", "chunks": []},
        {"type": "DepotFile", "path": "bin/tool", "chunks": [described["tool"]]},
        {"type": "DepotDirectory", "path": "saves"},
        {"type": "DepotLink", "path": "bin/current", "target": "Game/game.exe"},
    ]}}))
    .unwrap();
    let mut installed = Installed::default();
    installed
        .add("", "1207658924", manifest.depot.items)
        .unwrap();
    Fixture {
        cdn: Cdn::new(responses),
        installed: Arc::new(installed),
        chunks,
    }
}

impl Fixture {
    /// Endpoints for the product, renewed on demand.
    fn links(&self) -> Arc<SecureLinks> {
        let base = self.cdn.base.clone();
        Arc::new(SecureLinks::new(Arc::new(move |source| {
            assert_eq!(source, "1207658924");
            let endpoint = Endpoint {
                url: base.clone(),
                url_format: String::new(),
                parameters: Default::default(),
                priority: 10,
                fallback_only: false,
            };
            async move { Ok(vec![endpoint]) }.boxed()
        })))
    }

    async fn install(&self, root: &Path) -> Result<DownloadSummary, GogError> {
        download(
            crate::http::client().unwrap(),
            self.links(),
            self.installed.clone(),
            "build".into(),
            root,
            &CancelToken::new(),
            |_| {},
        )
        .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transfer_installs_every_file_then_reuses_what_is_on_disk() {
    let fixture = fixture();
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("gog-1207658924-windows");

    let summary = fixture.install(&root).await.unwrap();
    assert_eq!(fixture.cdn.requests().len(), 3);
    assert_eq!(summary.reused_bytes, 0);
    assert_eq!(
        fs::read(root.join("Game/game.exe")).unwrap(),
        b"first part second"
    );
    assert_eq!(fs::read(root.join("empty.txt")).unwrap(), b"");
    assert!(root.join("saves").is_dir());
    let tool = fs::metadata(root.join("bin/tool"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(tool & 0o111, 0o111, "a Mach-O header makes a program");
    let game = fs::metadata(root.join("Game/game.exe"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(game & 0o111, 0);
    assert_eq!(
        fs::read_link(root.join("bin/current")).unwrap(),
        Path::new("../Game/game.exe")
    );
    assert!(
        !root
            .join(PARTIAL_DIRECTORY)
            .join("gog")
            .join("build")
            .exists()
    );

    let summary = fixture.install(&root).await.unwrap();
    assert!(
        fixture.cdn.requests().is_empty(),
        "a verified installation downloads nothing"
    );
    assert_eq!(summary.reused_bytes, summary.total_bytes);

    // The file's second chunk changed: only it is downloaded again.
    fs::write(root.join("Game/game.exe"), b"first part SECOND").unwrap();
    fixture.install(&root).await.unwrap();
    assert_eq!(fixture.cdn.requests(), [fixture.chunks["b"].1.clone()]);
    assert_eq!(
        fs::read(root.join("Game/game.exe")).unwrap(),
        b"first part second"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_chunk_fails_the_transfer() {
    let fixture = fixture();
    let (bytes, _) = &fixture.chunks["a"];
    let mut chunk: crate::manifest::Chunk = serde_json::from_value(json!({
        "md5": md5_hex(bytes), "size": bytes.len(), "compressedMd5": md5_hex(b"other"), "compressedSize": 5,
    }))
    .unwrap();
    assert!(matches!(
        decode_chunk(b"other", &chunk),
        Err(GogError::Integrity)
    ));
    chunk.compressed_md5 = md5_hex(b"other");
    chunk.compressed_size = 5;
    assert!(
        matches!(decode_chunk(b"other", &chunk), Err(GogError::Integrity)),
        "not a zlib stream"
    );
    let packed = compressed(bytes);
    chunk.compressed_md5 = md5_hex(&packed);
    chunk.compressed_size = packed.len() as u64;
    assert_eq!(decode_chunk(&packed, &chunk).unwrap(), *bytes);
}

#[test]
fn chunk_urls_extend_the_endpoints_path() {
    let endpoint: Endpoint = serde_json::from_value(json!({
        "endpoint_name": "fastly",
        "url_format": "{base_url}/token=nva={expires_at}~dirs={dirs}~token={token}{path}",
        "parameters": {
            "base_url": "https://gog-cdn-fastly.gog.com",
            "path": "/content-system/v2/store/1207658924",
            "expires_at": 1_700_000_000,
            "dirs": 4,
            "token": "signed"
        },
        "priority": 10,
        "fallback_only": false
    }))
    .unwrap();
    assert_eq!(
        endpoint.chunk_url("ab/cd/abcd").unwrap().as_str(),
        "https://gog-cdn-fastly.gog.com/token=nva=1700000000~dirs=4~token=signed/content-system/v2/store/1207658924/ab/cd/abcd"
    );
    assert!(endpoint.expired());
    let open: Endpoint = serde_json::from_value(json!({
        "url": "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store",
        "url_format": "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store",
        "parameters": {}
    }))
    .unwrap();
    assert_eq!(
        open.chunk_url("ab/cd/abcd").unwrap().as_str(),
        "https://gog-cdn-fastly.gog.com/content-system/v2/dependencies/store/ab/cd/abcd"
    );
    let unknown: Endpoint = serde_json::from_value(json!({
        "url_format": "{base_url}{missing}{path}",
        "parameters": {"base_url": "https://cdn", "path": "/p"}
    }))
    .unwrap();
    assert!(unknown.chunk_url("ab/cd/abcd").is_err());
}

#[tokio::test]
async fn endpoints_put_fallbacks_last_and_renew_once_stale() {
    let renewals = Arc::new(StdMutex::new(0));
    let counted = renewals.clone();
    let links = SecureLinks::new(Arc::new(move |_| {
        *counted.lock().unwrap() += 1;
        let endpoints: Vec<Endpoint> = serde_json::from_value(json!([
            {"url": "https://fallback", "priority": 20, "fallback_only": true},
            {"url": "https://low", "priority": 1},
            {"url": "https://high", "priority": 10},
        ]))
        .unwrap();
        async move { Ok(endpoints) }.boxed()
    }));
    let order: Vec<_> = links
        .endpoints("1", false)
        .await
        .unwrap()
        .into_iter()
        .map(|endpoint| endpoint.url)
        .collect();
    assert_eq!(order, ["https://high", "https://low", "https://fallback"]);
    links.endpoints("1", false).await.unwrap();
    assert_eq!(*renewals.lock().unwrap(), 1);
    links.endpoints("1", true).await.unwrap();
    assert_eq!(*renewals.lock().unwrap(), 2);
}
