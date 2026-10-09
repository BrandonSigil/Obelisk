//! Voile's Linux engine. A Windows game whose anti-cheat supports Linux, such
//! as Rocket League's Easy Anti-Cheat, runs in voile-wine built for x86-64
//! Linux, which voile-lkd's engine executes in native macOS processes: the
//! anti-cheat then loads its Linux module. Only the games this list names run
//! there; every other Windows game keeps Rosetta's Wine.
use crate::{Error, supervisor::Target, wine};
use obelisk_core::launch::LaunchTarget;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// An Epic build that runs through the engine.
struct EpicGame {
    namespace: &'static str,
    app_name: &'static str,
    /// The program the engine starts instead of the publisher's executable,
    /// relative to the installation, when that executable cannot run there.
    executable: Option<&'static str>,
}

const EPIC: &[EpicGame] = &[
    // Rocket League: its Easy Anti-Cheat refuses Rosetta's Wine (code 206).
    // Its publisher executable, Launcher.exe, is a .NET program and the
    // engine's Wine has no Mono: the engine starts the Easy Anti-Cheat
    // bootstrapper Launcher.exe would start, with the same arguments.
    EpicGame {
        namespace: "9773aa1aa54f4f7b80e44bef04986cea",
        app_name: "Sugar",
        executable: Some("Binaries/Win64/RocketLeague_EAC.exe"),
    },
];

fn epic_game(namespace: &str, app_name: &str) -> Option<&'static EpicGame> {
    EPIC.iter()
        .find(|game| game.namespace == namespace && game.app_name == app_name)
}

/// Whether the Epic build `app_name` of `namespace` runs through the engine.
pub fn runs_epic(namespace: &str, app_name: &str) -> bool {
    epic_game(namespace, app_name).is_some()
}

/// What the engine starts for the Epic build `app_name` of `namespace`
/// installed in `install`: `target`, or the program the list names instead,
/// with the same arguments, in that program's folder.
pub(crate) fn epic_target(
    namespace: &str,
    app_name: &str,
    install: &Path,
    target: &LaunchTarget,
) -> LaunchTarget {
    let Some(executable) = epic_game(namespace, app_name).and_then(|game| game.executable) else {
        return target.clone();
    };
    let executable = install.join(executable);
    LaunchTarget {
        working_directory: executable
            .parent()
            .unwrap_or(install)
            .to_string_lossy()
            .into_owned(),
        executable: executable.to_string_lossy().into_owned(),
        arguments: target.arguments.clone(),
        is_application_bundle: false,
    }
}

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

/// The guest sees the installation here, and the prefix in its home.
const GAME: &str = "/games/game";
const HOME: &str = "/home/user";
/// The prefix's folder in the guest's home.
const PREFIX: &str = "prefix";
/// Neptune's Direct3D, which replaces Wine's in the prefix.
const DIRECT3D: [&str; 3] = ["d3d11.dll", "dxgi.dll", "d3d12.dll"];
/// Creating a prefix through the engine takes about a minute.
const BOOT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
/// on first use, and Neptune's Direct3D goes into it before each launch.
/// Storefront prerequisites do not run there.
pub(crate) fn prepare(
    config: &wine::Config,
    install: &Path,
    target: &LaunchTarget,
) -> Result<(), Error> {
    guest_program(install, target)?;
    if !config.installed() {
        return Err(Error::WineUnavailable);
    }
    crate::files::private_directory(home(&config.prefix)?)?;
    if !config.prefix.join("system.reg").is_file() {
        boot(config)?;
    }
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

/// The game `target` names inside `install`, started through the engine's
/// launcher with `environment`, logging to the installation's log. The
/// launcher is a native program: no Rosetta.
pub(crate) fn target(
    install: &Path,
    target: &LaunchTarget,
    environment: HashMap<String, String>,
    config: &wine::Config,
) -> Result<Target, Error> {
    let program = wine::program(install, target)?;
    let executable = guest_path(install, &program).ok_or(Error::NotExecutable)?;
    let mut environment = environment;
    let name = match environment.get(APP_NAME) {
        Some(title) => app_name(title),
        None => String::new(),
    };
    let name = match (name.is_empty(), Path::new(&target.executable).file_stem()) {
        (true, Some(stem)) => stem.to_string_lossy().into_owned(),
        _ => name,
    };
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
    ];
    for (name, value) in guest_environment(config) {
        arguments.push("--env".to_owned());
        arguments.push(format!("{name}={value}"));
    }
    arguments.push("--".to_owned());
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
    use std::{io::Write, os::unix::fs::PermissionsExt};
    let drive = config.prefix.join("drive_c");
    let directory = tempfile::Builder::new()
        .prefix(".obelisk-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&drive)?;
    let path = directory.path().join(name);
    crate::files::atomic_write(&path, crate::files::Durability::FileSync, |file| {
        file.write_all(bytes)?;
        Ok(())
    })?;
    let folder = directory
        .path()
        .file_name()
        .ok_or(Error::Storage)?
        .to_string_lossy()
        .into_owned();
    Ok((directory, format!("C:\\{folder}\\{name}")))
}

#[cfg(test)]
mod tests;
