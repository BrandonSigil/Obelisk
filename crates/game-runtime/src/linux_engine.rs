//! Voile's Linux engine. A Windows game whose anti-cheat supports Linux, such
//! as Rocket League's Easy Anti-Cheat, runs in voile-wine built for x86-64
//! Linux, which voile-lkd's engine executes in native macOS processes: the
//! anti-cheat then loads its Linux module. Only the games whose profile names
//! the engine run there; every other Windows game keeps Rosetta's Wine.
use crate::{Error, supervisor::Target, wine};
use obelisk_core::launch::LaunchTarget;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Starts the window server and Neptune's render server, then the game, and
/// ends them with it (see the script).
const LAUNCHER: &str = include_str!("linux_engine/launch.sh");
/// The private folder of the window server and the render server that the
/// launcher with `pid` started, in the temporary folder of its `environment`.
pub(crate) fn sockets(environment: &HashMap<String, String>, pid: u32) -> PathBuf {
    let root = environment
        .get("TMPDIR")
        .filter(|root| !root.is_empty())
        .map_or("/tmp", String::as_str);
    Path::new(root).join(format!("obelisk-engine-{pid}"))
}

/// The window server socket of the launcher with `pid`.
pub(crate) fn window_server(environment: &HashMap<String, String>, pid: u32) -> PathBuf {
    sockets(environment, pid).join("wsd.sock")
}

/// The launcher's variable that names the game in the menu bar, the Dock and
/// Cmd-Tab: its name in the store, or its program's file name without one.
pub(crate) const APP_NAME: &str = "OBELISK_APP_NAME";
/// A store title as an application name: without the trademark signs stores
/// add ("Rocket League®"), which Mac apps leave out.
fn app_name(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '®' | '™' | '©'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The guest sees the installation here, and the prefix in its home. Every
/// folder above a mount must exist in the engine's root: Wine's CreateProcess
/// lists each folder of the program's path, and a folder the engine makes up
/// for a mount, such as /games for /games/game, cannot be handed to the Wine
/// server, which then ends the game.
const GAME: &str = "/opt/game";
const HOME: &str = "/home/user";
/// The runtime's Steam folder, where linux64 holds Valve's Linux steamclient.
const STEAM: &str = "/opt/steam";
/// The runtime's guest libraries.
const LIBRARIES: &str = "/opt/obelisk";
/// The library that gives Valve's steamclient the macOS pid of its process.
const HOST_PID: &str = "steam-hostpid.so";
/// The prefix's folder in the guest's home.
const PREFIX: &str = "prefix";
/// Neptune's Direct3D, which replaces Wine's in the prefix.
const DIRECT3D: [&str; 3] = ["d3d11.dll", "dxgi.dll", "d3d12.dll"];
/// Creating a prefix through the engine takes about a minute.
const BOOT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Records, in a runtime, the macOS build Rosetta translated its D3DMetal for.
const TRANSLATED: &str = ".obelisk-d3dmetal-translated";
/// Translating D3DMetal takes about 8 seconds; one still running after this
/// long is abandoned, and the game pays for the rest.
const TRANSLATION_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// The engine's prefix of the installation whose Rosetta prefix is
/// `rosetta`: its own folder beside it, the prefix inside, since the guest
/// sees that folder as its home.
pub(crate) fn prefix(rosetta: &Path) -> Result<PathBuf, Error> {
    let name = rosetta.file_name().ok_or(Error::Storage)?;
    let mut home = name.to_owned();
    home.push("-linux");
    Ok(rosetta.with_file_name(home).join(PREFIX))
}

/// The guest's home of an engine prefix.
pub(crate) fn home(prefix: &Path) -> Result<&Path, Error> {
    prefix.parent().ok_or(Error::Storage)
}

/// What every Wine command of the engine runs with in the guest.
fn guest_environment(config: &wine::Config) -> Vec<(&'static str, String)> {
    vec![
        ("HOME", HOME.into()),
        ("USER", "user".into()),
        ("LOGNAME", "user".into()),
        ("PATH", "/opt/wine/bin:/usr/bin:/bin".into()),
        ("LANG", "en_US.UTF-8".into()),
        ("WINEPREFIX", format!("{HOME}/{PREFIX}")),
        ("WINEARCH", "win64".into()),
        ("WINEDEBUG", config.debug_channels()),
        (
            "WINEDLLOVERRIDES",
            "mscoree,mshtml=;winemenubuilder.exe=d;d3d11,dxgi,d3d12=n".into(),
        ),
        ("VTEST_SOCKET_NAME", "/run/obelisk/vtest.sock".into()),
        ("NPT_DEBUG", "force_vtest".into()),
    ]
}

/// The guest path of the Windows program `target` names inside `install`.
fn guest_program(install: &Path, target: &LaunchTarget) -> Result<String, Error> {
    let executable = wine::program(install, target)?;
    guest_path(install, &executable).ok_or(Error::NotExecutable)
}

/// Where the guest sees `path`, a file or folder inside `install`.
fn guest_path(install: &Path, path: &Path) -> Option<String> {
    let root = install.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    let relative = path.strip_prefix(&root).ok()?;
    Some(match relative.as_os_str().is_empty() {
        true => GAME.to_owned(),
        false => format!("{GAME}/{}", relative.to_string_lossy()),
    })
}

/// Readies the engine's prefix to start `target`: the game must be a Windows
/// program inside the installation, the prefix is created through the engine
/// on first use, lsteamclient is led to Valve's Linux steamclient for a
/// storefront whose games reach it, the storefront furnishes the prefix, and
/// Neptune's Direct3D goes into it before each launch. Storefront
/// prerequisites do not run there.
pub(crate) fn prepare<S: wine::Storefront>(
    config: &wine::Config,
    install: &Path,
    target: &LaunchTarget,
    storefront: &S,
) -> Result<(), Error> {
    guest_program(install, target)?;
    if !config.installed() {
        return Err(Error::WineUnavailable);
    }
    crate::files::private_directory(home(&config.prefix)?)?;
    if !config.prefix.join("system.reg").is_file() {
        boot(config)?;
    }
    if S::STEAM_CLIENT {
        link_steam_client(config)?;
    }
    storefront.furnish(config)?;
    let system32 = config.prefix.join("drive_c/windows/system32");
    for library in DIRECT3D {
        fs::copy(
            config.runtime.join("windows").join(library),
            system32.join(library),
        )?;
    }
    match config.log.parent() {
        Some(logs) => crate::files::private_directory(logs),
        None => Err(Error::Storage),
    }
}

/// Creates the prefix through the engine, under its timeout wrapper, which
/// ends every process of the engine it started.
fn boot(config: &wine::Config) -> Result<(), Error> {
    let engine = config.runtime.join("engine");
    let mut command = Command::new(engine.join("run-with-timeout.sh"));
    command
        .arg(BOOT_TIMEOUT.as_secs().to_string())
        .arg(engine.join("fexdarwin-run"))
        .args(["--cwd", HOME]);
    for (name, value) in guest_environment(config) {
        command.arg("--env").arg(format!("{name}={value}"));
    }
    let status = command
        .args([
            "/bin/sh",
            "-c",
            "/opt/wine/bin/wine wineboot --init; /opt/wine/bin/wineserver -w",
        ])
        .env_clear()
        .env("FEXDARWIN_LKD", "1")
        .env("FEXDARWIN_ROOT", config.runtime.join("root"))
        .env("FEXDARWIN_ROOT_RO", "1")
        .env("FEXDARWIN_BIND", binds(config, None)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() || !config.prefix.join("system.reg").is_file() {
        return Err(Error::StartFailed);
    }
    Ok(())
}

/// The engine's folders as the guest sees them; a launch adds the game.
fn binds(config: &wine::Config, install: Option<&Path>) -> Result<String, Error> {
    let mut binds = vec![
        format!("/opt/wine={}:ro", config.runtime.join("wine").display()),
        format!("{HOME}={}", home(&config.prefix)?.display()),
    ];
    if let Some(install) = install {
        binds.push(format!("{GAME}={}", install.display()));
    }
    Ok(binds.join(";"))
}

/// Has Rosetta translate D3DMetal and the Intel render server before the
/// first game of `config` that renders with D3DMetal. The render server
/// loads D3DMetal when its game first asks for Direct3D, and translating it
/// then holds the game about 7 seconds, long enough for Easy Anti-Cheat to
/// give up. Rosetta keeps a translation per file until macOS updates, so it
/// runs once per runtime and macOS build, through Obelisk's Intel helper in
/// `helpers`. A failure only leaves the cost to the game.
pub(crate) fn translate_d3dmetal(config: &wine::Config, helpers: &Path) {
    if config.engine != wine::Engine::Linux || config.graphics != wine::Graphics::D3DMetal {
        return;
    }
    let (Some(system), Some(release)) = (macos_build(), config.runtime.parent()) else {
        return;
    };
    let record = config.runtime.join(TRANSLATED);
    if fs::read_to_string(&record).is_ok_and(|translated| translated == system) {
        return;
    }
    let server = config.runtime.join("graphics/x86_64");
    // Each server is translated as it starts, and stops at an unknown option.
    for program in ["bin/virgl_test_server", "libexec/virgl_render_server"] {
        let mut command = Command::new(server.join(program));
        command.arg("--obelisk-translate");
        quietly(command);
    }
    let mut command = Command::new(helpers.join(crate::package::SDK_INTEL));
    command
        .arg("warm-d3dmetal")
        .arg(server.join("lib/libd3dmetal-native.dylib"))
        .arg(release.join("lib/external/D3DMetal.framework"));
    if quietly(command) == Some(0) {
        let _ = obelisk_core::write_atomic(&record, system.as_bytes());
    }
}

/// The exit code of `command`, run without input or output, or `None` when
/// it could not start or outlasted `TRANSLATION_TIMEOUT`.
fn quietly(mut command: Command) -> Option<i32> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    wine::wait(child, TRANSLATION_TIMEOUT).ok().flatten()
}

/// The running macOS build, such as `25A354`.
fn macos_build() -> Option<String> {
    let output = Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.osversion"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let build = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !build.is_empty()).then_some(build)
}

/// Leads lsteamclient to Valve's Linux steamclient: it loads the library the
/// guest home's .steam/sdk64 leads to, here the runtime's, which the guest
/// sees in /opt/steam.
fn link_steam_client(config: &wine::Config) -> Result<(), Error> {
    let steam = home(&config.prefix)?.join(".steam");
    crate::files::private_directory(&steam)?;
    let link = steam.join("sdk64");
    let client = Path::new(STEAM).join("linux64");
    match fs::read_link(&link) {
        Ok(current) if current == client => return Ok(()),
        Ok(_) => fs::remove_file(&link)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        // Something that is not a link stands there.
        Err(_) => return Err(Error::Storage),
    }
    std::os::unix::fs::symlink(client, link)?;
    Ok(())
}

/// What a storefront's games need in the guest beyond Wine's own.
#[derive(Debug, Default)]
struct Guest {
    /// Runtime folders the guest sees, read-only, by their guest path.
    binds: Vec<(&'static str, PathBuf)>,
    environment: Vec<(&'static str, String)>,
}

impl Guest {
    /// The guest of S's games. When they reach Valve's steamclient,
    /// lsteamclient finds Valve's Linux steamclient in the runtime. Steam's
    /// engine drops a client whose pid is not a live macOS process: a runtime
    /// that carries steam-hostpid.so has it preloaded in every guest process,
    /// where it gives that pid to Valve's steamclient alone.
    fn of<S: wine::Storefront>(config: &wine::Config) -> Self {
        let mut guest = Self::default();
        if !S::STEAM_CLIENT {
            return guest;
        }
        guest.binds.push((STEAM, config.runtime.join("steam")));
        let libraries = config.runtime.join("guest");
        if libraries.join(HOST_PID).is_file() {
            guest.binds.push((LIBRARIES, libraries));
            guest
                .environment
                .push(("LD_PRELOAD", format!("{LIBRARIES}/{HOST_PID}")));
        }
        guest
    }
}

/// The game `target` names inside `install`, started through the engine's
/// launcher with `environment`, in a guest S completes, logging to the
/// installation's log. The menu bar, the Dock and Cmd-Tab show `title`, or
/// the program's file name without one. The launcher is a native program:
/// no Rosetta.
pub(crate) fn target<S: wine::Storefront>(
    install: &Path,
    target: &LaunchTarget,
    title: &str,
    environment: HashMap<String, String>,
    config: &wine::Config,
) -> Result<Target, Error> {
    let guest = Guest::of::<S>(config);
    let program = wine::program(install, target)?;
    let executable = guest_path(install, &program).ok_or(Error::NotExecutable)?;
    let name = match (app_name(title), Path::new(&target.executable).file_stem()) {
        (name, Some(stem)) if name.is_empty() => stem.to_string_lossy().into_owned(),
        (name, _) => name,
    };
    let mut environment = environment;
    environment.insert(APP_NAME.to_owned(), name);
    let install = install.canonicalize()?;
    let working_directory = guest_path(&install, Path::new(&target.working_directory))
        .unwrap_or_else(|| GAME.to_owned());
    let mut arguments = vec![
        "-c".to_owned(),
        LAUNCHER.to_owned(),
        "obelisk-linux-engine".to_owned(),
        config.runtime.to_string_lossy().into_owned(),
        home(&config.prefix)?.to_string_lossy().into_owned(),
        install.to_string_lossy().into_owned(),
        working_directory,
        "--graphics".to_owned(),
        config.graphics.name().to_owned(),
    ];
    for (path, folder) in &guest.binds {
        arguments.push("--bind".to_owned());
        arguments.push(format!("{path}={}:ro", folder.display()));
    }
    let variables = guest_environment(config)
        .into_iter()
        .chain(guest.environment.iter().cloned());
    for (name, value) in variables {
        arguments.push("--env".to_owned());
        arguments.push(format!("{name}={value}"));
    }
    // The launcher hands the guest the variables S's games get from their
    // session, which exist only once the game starts.
    for name in S::PASSED {
        arguments.push("--pass".to_owned());
        arguments.push((*name).to_owned());
    }
    arguments.push("--".to_owned());
    arguments.extend(S::HELPER.map(str::to_owned));
    arguments.push(executable);
    arguments.extend(target.arguments.iter().cloned());
    Ok(Target {
        executable: "/bin/bash".into(),
        arguments,
        working_directory: install.clone(),
        environment,
        log: Some(config.log.clone()),
        installation: Some(install),
        program: Some(program),
    })
}

/// A file the game reads in the engine's prefix: the guest cannot see the
/// Mac's disk, so it goes into the prefix's C: drive, in a private folder that
/// lives as long as the game. Returns the folder and the file's Windows path.
pub(crate) fn private_file(
    config: &wine::Config,
    name: &str,
    bytes: &[u8],
) -> Result<(tempfile::TempDir, String), Error> {
    let drive = config.prefix.join("drive_c");
    let (directory, _) = crate::files::private_temp_file(&drive, ".obelisk-", name, bytes)?;
    let folder = directory
        .path()
        .file_name()
        .ok_or(Error::Storage)?
        .to_string_lossy()
        .into_owned();
    Ok((directory, format!("C:\\{folder}\\{name}")))
}

#[cfg(test)]
pub(crate) mod tests;
