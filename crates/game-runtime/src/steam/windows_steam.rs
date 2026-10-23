//! A Windows Steam game that Valve's Windows Steam starts, as its Launch
//! settings may ask: Steam installs itself in a prefix of its own, the
//! installation joins its library as installed, and Steam starts the game
//! there, hidden. It is the one client whose answer to a Windows game's
//! module signature check Valve's servers accept, which a VAC-secured server
//! asks for: the Mac steamclient checks only Mac programs. Steam signs in
//! with what it keeps in the prefix, and keeps the game's Cloud saves itself.
use crate::{Error, supervisor::Target, wine};
use obelisk_core::launch::LaunchTarget;
use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

/// Where Steam installs itself in a prefix, and keeps its default library.
const STEAM: &str = "drive_c/Program Files (x86)/Steam";
/// Its program there, as Wine reads it.
const STEAM_EXE: &str = r"C:\Program Files (x86)\Steam\steam.exe";
/// Valve's installer of Windows Steam, which installs it silently with `/S`.
const INSTALLER_URL: &str = "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe";
/// Where the prefix keeps the installer until Steam is installed.
const INSTALLER: &str = "obelisk-steam-setup.exe";
/// The largest installer read: it is about 2.5 MB.
const MAX_INSTALLER_BYTES: u64 = 32 * 1024 * 1024;
/// The prerequisite that runs the installer, once per prefix.
const INSTALL_STEAM: &str = "obelisk-windows-steam";
/// Steamworks Common Redistributables, whose depots, such as DirectX's, a
/// game's installation shares with every other game.
const COMMON_REDISTRIBUTABLES: u32 = 228_980;
/// Its depots, which follow its app ID.
const COMMON_REDISTRIBUTABLE_DEPOTS: std::ops::RangeInclusive<u32> = 228_981..=229_100;
/// What starts Steam and lasts as long as the game.
const LAUNCHER: &str = include_str!("windows_steam.sh");

/// What Windows Steam adds to the prefix of the installation of `app_id` in
/// `install`, whose prefix is `prefix`.
pub(super) struct Prefix<'a> {
    pub install: &'a Path,
    pub app_id: u32,
    pub prefix: &'a Path,
}

impl wine::Storefront for Prefix<'_> {
    /// Steam's installer, until Steam is installed, then the steps of the
    /// installation's install scripts, which Steam would run on its first
    /// launch.
    fn prerequisites(&self) -> Result<Vec<wine::Prerequisite>, Error> {
        let mut prerequisites = Vec::new();
        if !installed(self.prefix) {
            prerequisites.push(wine::Prerequisite {
                id: INSTALL_STEAM.to_owned(),
                programs: vec![(self.prefix.join(INSTALLER), "/S".to_owned())],
            });
        }
        prerequisites.extend(super::windows::install_scripts(self.install)?);
        Ok(prerequisites)
    }

    /// Downloads Steam's installer while Steam is missing, then makes the
    /// installation one of Steam's.
    fn furnish(&self, config: &wine::Config) -> Result<(), Error> {
        if !installed(&config.prefix) {
            let installer = config.prefix.join(INSTALLER);
            if !installer.is_file() {
                download_installer(&installer)?;
            }
        } else {
            let _ = fs::remove_file(config.prefix.join(INSTALLER));
        }
        link(&config.prefix, self.install, self.app_id)
    }
}

/// Whether Steam is installed in `prefix`.
fn installed(prefix: &Path) -> bool {
    prefix.join(STEAM).join("steam.exe").is_file()
}

/// Downloads Steam's installer to `path`, from Valve's CDN as Obelisk's Steam
/// components are. Valve replaces it with each release, so no checksum pins
/// it: it must be a Windows program, of a sane size.
fn download_installer(path: &Path) -> Result<(), Error> {
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| Error::StartFailed)?;
    let bytes = runtime.block_on(async {
        let response = reqwest::Client::new()
            .get(INSTALLER_URL)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| Error::StartFailed)?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_INSTALLER_BYTES)
        {
            return Err(Error::StartFailed);
        }
        response.bytes().await.map_err(|_| Error::StartFailed)
    })?;
    if bytes.len() as u64 > MAX_INSTALLER_BYTES || !bytes.starts_with(b"MZ") {
        return Err(Error::StartFailed);
    }
    let directory = path.parent().ok_or(Error::Storage)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.persist(path).map_err(|_| Error::Storage)?;
    Ok(())
}

/// Makes the installation of `app_id` in `install` one of Steam's in
/// `prefix`: a link to it in Steam's default library, named after its
/// folder, and an app manifest naming the depots Obelisk installed. An app
/// manifest Steam already keeps stays, with what Steam learned since.
fn link(prefix: &Path, install: &Path, app_id: u32) -> Result<(), Error> {
    let apps = prefix.join(STEAM).join("steamapps");
    let common = apps.join("common");
    fs::create_dir_all(&common)?;
    let folder = install.file_name().ok_or(Error::NotExecutable)?;
    let link = common.join(folder);
    match fs::read_link(&link) {
        Ok(target) if target == install => {}
        // The games folder moved: the link follows it.
        Ok(_) => {
            fs::remove_file(&link)?;
            std::os::unix::fs::symlink(install, &link)?;
        }
        Err(_) if fs::symlink_metadata(&link).is_ok() => return Err(Error::Storage),
        Err(_) => std::os::unix::fs::symlink(install, &link)?,
    }
    let manifest = apps.join(format!("appmanifest_{app_id}.acf"));
    if manifest.is_file() {
        return Ok(());
    }
    let installed =
        obelisk_core::installation::read_receipt::<steam_kit::client::Installation>(install)
            .map_err(|_| Error::Storage)?
            .filter(|receipt| receipt.app_id == app_id)
            .ok_or(Error::Storage)?;
    let contents = app_manifest(&installed, &folder.to_string_lossy());
    let mut file = tempfile::NamedTempFile::new_in(&apps)?;
    file.write_all(contents.as_bytes())?;
    file.persist(&manifest).map_err(|_| Error::Storage)?;
    Ok(())
}

/// Steam's app manifest of `installed`, whose folder in its library is
/// `folder`: fully installed, with the depots Obelisk installed. It names no
/// build: Steam finds the depots up to date and records the build itself, or
/// downloads what Obelisk left out.
fn app_manifest(installed: &steam_kit::client::Installation, folder: &str) -> String {
    let (shared, own): (Vec<_>, Vec<_>) = installed
        .manifests
        .iter()
        .partition(|depot| COMMON_REDISTRIBUTABLE_DEPOTS.contains(&depot.depot_id));
    let mut manifest = String::new();
    let _ = write!(
        manifest,
        "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app}\"\n\t\"universe\"\t\t\"1\"\n\t\"name\"\t\t\"{name}\"\n\
         \t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"{folder}\"\n\t\"SizeOnDisk\"\t\t\"{size}\"\n\
         \t\"buildid\"\t\t\"0\"\n\t\"InstalledDepots\"\n\t{{\n",
        app = installed.app_id,
        name = vdf_escape(&installed.name),
        folder = vdf_escape(folder),
        size = installed.total_bytes,
    );
    for depot in own {
        let _ = write!(
            manifest,
            "\t\t\"{}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{}\"\n\t\t}}\n",
            depot.depot_id, depot.manifest_id
        );
    }
    manifest.push_str("\t}\n");
    if !shared.is_empty() {
        manifest.push_str("\t\"SharedDepots\"\n\t{\n");
        for depot in shared {
            let _ = writeln!(
                manifest,
                "\t\t\"{}\"\t\t\"{COMMON_REDISTRIBUTABLES}\"",
                depot.depot_id
            );
        }
        manifest.push_str("\t}\n");
    }
    let _ = write!(
        manifest,
        "\t\"UserConfig\"\n\t{{\n\t\t\"language\"\t\t\"{language}\"\n\t}}\n\
         \t\"MountedConfig\"\n\t{{\n\t\t\"language\"\t\t\"{language}\"\n\t}}\n}}\n",
        language = vdf_escape(&installed.language),
    );
    manifest
}

/// `text` as a quoted value of Valve's KeyValues text.
fn vdf_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The Windows game `target` names, which Steam starts, hidden, with the
/// arguments its profile and its player add: Steam adds its own launch
/// options, so the storefront's are left out.
pub(super) fn target(
    config: &wine::Config,
    game: &wine::Game<'_>,
    target: &LaunchTarget,
    app_id: u32,
    home: &Path,
) -> Result<Target, Error> {
    let settings = &config.settings;
    let after: Vec<String> = settings
        .profile_arguments
        .iter()
        .chain(&settings.arguments)
        .cloned()
        .collect();
    wine::launcher_target(
        config,
        game,
        target,
        LAUNCHER,
        "obelisk-windows-steam",
        vec![STEAM_EXE.to_owned(), app_id.to_string()],
        after,
        crate::native::environment(home),
    )
}

/// Where Steam keeps the accounts it signs in to, in its folder.
const LOGIN_USERS: &str = "config/loginusers.vdf";
/// Where Steam keeps their login tokens, encrypted for the Windows user
/// CrossOver's Wine runs as.
const LOGIN_TOKENS: &str = "drive_c/users/crossover/AppData/Local/Steam/local.vdf";
/// Steam's DevTools, on the Mac's loopback while `-cef-enable-debugging`
/// asks for them.
const DEVTOOLS: &str = "http://127.0.0.1:8080";
/// Where those DevTools take a page's script.
const DEVTOOLS_SOCKETS: &str = "ws://127.0.0.1:8080/";
/// Where Steam's interface pages load from.
const STEAM_INTERFACE: &str = "https://steamloopback.host/";
/// Steam's first start in a prefix updates it, which can take minutes under
/// Rosetta.
const START_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long Steam gets, once it has the token, to sign in and keep the login:
/// its interface finishes its own sign-in before it answers, over a minute
/// later under Rosetta.
const KEEP_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// `EResult::OK`, as `SetLoginToken` answers it.
const SIGNED_IN: i64 = 1;

/// The account Valve's Windows Steam in `prefix` signs in to by itself: one
/// whose login it remembers, with a login token kept. The account Steam last
/// signed in to comes first.
pub(crate) fn account(prefix: &Path) -> Option<String> {
    if !prefix.join(LOGIN_TOKENS).is_file() {
        return None;
    }
    let users = fs::read_to_string(prefix.join(STEAM).join(LOGIN_USERS)).ok()?;
    remembered_account(&users)
}

/// The account `users`, Steam's loginusers.vdf, remembers the login of,
/// the most recent first.
fn remembered_account(users: &str) -> Option<String> {
    let mut accounts = Vec::new();
    let (mut name, mut remembered, mut recent) = (None, false, false);
    for line in users.lines() {
        let fields: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        match fields.as_slice() {
            ["AccountName", value] => name = Some((*value).to_owned()),
            ["RememberPassword", value] => remembered = *value == "1",
            ["MostRecent", value] => recent = *value == "1",
            _ if line.trim() == "}" => {
                if let (Some(account), true) = (name.take(), remembered) {
                    accounts.push((recent, account));
                }
                (remembered, recent) = (false, false);
            }
            _ => {}
        }
    }
    accounts.sort_by_key(|(recent, _)| !recent);
    accounts.into_iter().next().map(|(_, account)| account)
}

/// Signs Valve's Windows Steam in config's readied prefix in to
/// `account_name`'s account with `refresh_token`, a SteamClient login token
/// Obelisk signed in for it, as Steam's own sign-in window would: Steam
/// starts hidden with its DevTools on the loopback, its interface's script
/// takes the token, and Steam keeps the login once signed in, encrypted its
/// own way. Steam's window cannot draw in Voile's Wine, so it never shows.
/// The token reaches no page but Steam's own interface, and no log.
pub(super) async fn sign_in(
    config: &wine::Config,
    home: &Path,
    account_name: &str,
    refresh_token: &str,
) -> Result<(), Error> {
    // A Steam that kept a login signs in with it as it starts, then takes
    // no other: it starts signed out, and gets the kept one back unless the
    // new one took its place.
    let kept = KeptLogin::set_aside(&config.prefix)?;
    let signed_in = run_signed_out(config, home, account_name, refresh_token).await;
    if signed_in.is_ok() {
        kept.discard();
    }
    signed_in
}

/// `sign_in`, once Steam's kept login is set aside: Steam starts, takes the
/// login, and ends before this returns.
async fn run_signed_out(
    config: &wine::Config,
    home: &Path,
    account_name: &str,
    refresh_token: &str,
) -> Result<(), Error> {
    let steam = wine::command(config, home)?
        .args([
            STEAM_EXE,
            "-silent",
            "-nofriendsui",
            "-nochatui",
            "-cef-disable-gpu",
            "-cef-disable-gpu-compositing",
            "-cef-enable-debugging",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut steam = Running {
        steam,
        config: config.clone(),
    };
    let signed_in = hand_over(&config.prefix, account_name, refresh_token).await;
    shut_down(&mut steam, home).await;
    signed_in
}

/// The Steam a sign-in started, with its DevTools on the loopback: it ends
/// with the sign-in, whatever ends that, even a cancellation, together with
/// everything else of the prefix.
struct Running {
    steam: std::process::Child,
    config: wine::Config,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.steam.kill();
        let _ = self.steam.wait();
        wine::stop(&self.config);
    }
}

/// Hands Steam `refresh_token` once, on the first page of its interface on
/// the loopback that offers `SteamClient` within `START_TIMEOUT`, then waits
/// for Steam to keep the login in `prefix`, as `account_name`'s, its name in
/// lowercase. Its kept login was set aside, so a login it keeps is the new
/// one, whether or not it answers the call first. A Steam just installed may
/// never load its interface's own script, but its pages still carry
/// `SteamClient`, which is all the login needs.
async fn hand_over(prefix: &Path, account_name: &str, refresh_token: &str) -> Result<(), Error> {
    let socket = offering_page(START_TIMEOUT).await?;
    let call = set_login_token(&socket, account_name, refresh_token);
    tokio::pin!(call);
    let mut answered = false;
    let deadline = Instant::now() + KEEP_TIMEOUT;
    loop {
        if account(prefix).is_some_and(|kept| kept.eq_ignore_ascii_case(account_name)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::WindowsSteamSignIn);
        }
        tokio::select! {
            answer = &mut call, if !answered => {
                answered = true;
                // Steam refused the login, as its sign-in page would show.
                if answer.is_some_and(|result| result != SIGNED_IN) {
                    return Err(Error::WindowsSteamSignIn);
                }
            }
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

/// The DevTools address of the first page of Steam's interface whose script
/// offers `SteamClient.Auth.SetLoginToken`, within `timeout`.
async fn offering_page(timeout: Duration) -> Result<String, Error> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + timeout;
    loop {
        for socket in interface_pages(&client).await {
            let offers = evaluate(
                &socket,
                "typeof SteamClient === 'object' && !!SteamClient.Auth \
                 && typeof SteamClient.Auth.SetLoginToken === 'function'",
                Duration::from_secs(10),
            )
            .await;
            if offers.as_ref().and_then(serde_json::Value::as_bool) == Some(true) {
                return Ok(socket);
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::WindowsSteamSignIn);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// The DevTools addresses of the pages of Steam's own interface: only those
/// ever receive the token.
async fn interface_pages(client: &reqwest::Client) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Page {
        url: String,
        #[serde(rename = "webSocketDebuggerUrl")]
        socket: Option<String>,
    }
    let pages = async {
        client
            .get(format!("{DEVTOOLS}/json"))
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .json::<Vec<Page>>()
            .await
    }
    .await
    .unwrap_or_default();
    pages
        .into_iter()
        .filter(|page| page.url.starts_with(STEAM_INTERFACE))
        .filter_map(|page| page.socket)
        .filter(|socket| socket.starts_with(DEVTOOLS_SOCKETS))
        .collect()
}

/// Hands the page at `socket` `refresh_token`, as Steam's sign-in page does
/// once its own sign-in succeeds, and returns Steam's `EResult`; `None` when
/// Steam does not answer within `KEEP_TIMEOUT`.
async fn set_login_token(socket: &str, account_name: &str, refresh_token: &str) -> Option<i64> {
    let expression = format!(
        "SteamClient.Auth.SetLoginToken({}, {}).then(answer => answer.result)",
        serde_json::to_string(refresh_token).ok()?,
        serde_json::to_string(account_name).ok()?,
    );
    evaluate(socket, &expression, KEEP_TIMEOUT).await?.as_i64()
}

/// What `expression` evaluates to on the page at `socket`, once its promise
/// settles, within `timeout`.
async fn evaluate(socket: &str, expression: &str, timeout: Duration) -> Option<serde_json::Value> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let request = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": expression, "awaitPromise": true, "returnByValue": true },
    });
    let (mut stream, _) = tokio_tungstenite::connect_async(socket).await.ok()?;
    stream
        .send(Message::Text(request.to_string().into()))
        .await
        .ok()?;
    let value = tokio::time::timeout(timeout, async {
        while let Some(Ok(message)) = stream.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(reply) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if reply["id"] == 1 {
                return Some(reply["result"]["result"]["value"].clone());
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    let _ = stream.close(None).await;
    value
}

/// Steam's kept login in a prefix, its accounts and their tokens, set aside
/// while a sign-in replaces it. It comes back once dropped, over what Steam
/// wrote, unless discarded: a sign-in that fails, or that ends unfinished,
/// leaves the login it found.
struct KeptLogin {
    files: Vec<PathBuf>,
}

/// What a set-aside file of the kept login is called, beside it.
const SET_ASIDE: &str = "obelisk-previous";

impl KeptLogin {
    /// Moves the kept login of `prefix` aside. A login a sign-in Obelisk did
    /// not see end left aside comes back first.
    fn set_aside(prefix: &Path) -> Result<Self, Error> {
        let files = vec![
            prefix.join(STEAM).join(LOGIN_USERS),
            prefix.join(LOGIN_TOKENS),
        ];
        for file in &files {
            let aside = Self::aside(file);
            if aside.is_file() && !file.exists() {
                fs::rename(&aside, file)?;
            }
            if file.is_file() {
                fs::rename(file, aside)?;
            }
        }
        Ok(Self { files })
    }

    fn aside(file: &Path) -> PathBuf {
        let mut name = file.as_os_str().to_owned();
        name.push(".");
        name.push(SET_ASIDE);
        PathBuf::from(name)
    }

    /// The new login took its place.
    fn discard(mut self) {
        for file in self.files.drain(..) {
            let _ = fs::remove_file(Self::aside(&file));
        }
    }
}

impl Drop for KeptLogin {
    fn drop(&mut self) {
        for file in &self.files {
            let aside = Self::aside(file);
            if aside.is_file() {
                let _ = fs::rename(&aside, file);
            }
        }
    }
}

/// Steam writes its configuration as it shuts down: it gets that long
/// before it ends with the rest of the prefix.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(20);

/// Asks Steam to shut down, as its menu does, and gives it time to.
async fn shut_down(running: &mut Running, home: &Path) {
    let shutdown = wine::command(&running.config, home).and_then(|mut command| {
        Ok(command
            .args([STEAM_EXE, "-shutdown"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?)
    });
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline && matches!(running.steam.try_wait(), Ok(None)) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if let Ok(mut shutdown) = shutdown {
        let _ = shutdown.kill();
        let _ = shutdown.wait();
    }
}

#[cfg(test)]
mod tests;
