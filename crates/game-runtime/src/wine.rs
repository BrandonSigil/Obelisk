//! A Windows game runs in Obelisk's Wine runtime, inside the prefix of its
//! installation. What a storefront adds, such as Steam's steam.exe helper or
//! the prerequisites it names, comes from that storefront's side, and is
//! translated here for the engine the game's profile names.
use crate::{
    Error,
    supervisor::{Target, architecture::Architecture},
};
use obelisk_core::launch::LaunchTarget;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Wine is an Intel program: Rosetta runs it and the Windows game inside it.
pub(crate) const ARCHITECTURE: Architecture = Architecture::Intel;
/// The user every prefix of CrossOver's Wine names its profile after.
const USER: &str = "crossover";
/// The prerequisites a prefix ran, one id per line.
const PREREQUISITES: &str = "obelisk-prerequisites";
/// The Wine Mono whose .NET registration the prefix holds.
const MONO: &str = "obelisk-mono";
/// The runtime's graphics backends whose DLLs the prefix holds placeholders
/// for, by folder name.
const BACKENDS: &str = "obelisk-graphics";
/// D3DMetal's DLLs in a runtime, beside Wine's own.
const D3DMETAL_DLLS: &str = "lib/d3dmetal";
/// DXMT's DLLs in a runtime, beside Wine's own.
const DXMT_DLLS: &str = "lib/dxmt";
/// D3DMetal's library, which its unix libraries load.
const D3DMETAL_FRAMEWORK: &str = "lib/external/D3DMetal.framework/D3DMetal";
/// Their output, rewritten by each launch that runs one.
const PREREQUISITES_LOG: &str = "obelisk-prerequisites.log";
/// The registry GStreamer keeps of the runtime's plugins.
const GSTREAMER_REGISTRY: &str = "obelisk-gstreamer-registry.bin";
/// An installer still running after this long is stopped.
const PREREQUISITE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Success, "a newer version is already installed" and "restart required".
const INSTALLED: [i32; 3] = [0, 1638, 3010];
/// A command that ends a game is stopped after this long: the Wine server it
/// talks to may be frozen with the game.
const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

/// The Windows user profile inside `prefix`, where Auto-Cloud roots such as
/// AppData and Documents live.
pub fn profile(prefix: &Path) -> PathBuf {
    prefix.join("drive_c/users").join(USER)
}

/// How one Windows installation runs: the Wine runtime, the installation's
/// prefix, the log Wine writes while its game runs, the engine that runs
/// that Wine, the graphics backend that renders its Direct3D, with that
/// backend's options, the arguments its player adds to its command line, and
/// the environment variables and DLL overrides its game's profile adds to
/// Wine's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SavedConfig")]
pub struct Config {
    pub runtime: PathBuf,
    pub prefix: PathBuf,
    pub log: PathBuf,
    pub engine: Engine,
    pub graphics: Graphics,
    pub options: GraphicsOptions,
    pub arguments: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub dll_overrides: BTreeMap<String, String>,
}

/// A configuration as an earlier Obelisk may have saved it: one without a
/// backend renders with its engine's default.
#[derive(Deserialize)]
struct SavedConfig {
    runtime: PathBuf,
    prefix: PathBuf,
    log: PathBuf,
    #[serde(default)]
    engine: Engine,
    graphics: Option<Graphics>,
    #[serde(default)]
    options: GraphicsOptions,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    variables: BTreeMap<String, String>,
    #[serde(default)]
    dll_overrides: BTreeMap<String, String>,
}

impl From<SavedConfig> for Config {
    fn from(saved: SavedConfig) -> Self {
        Self {
            runtime: saved.runtime,
            prefix: saved.prefix,
            log: saved.log,
            engine: saved.engine,
            graphics: saved
                .graphics
                .unwrap_or_else(|| Graphics::default_on(saved.engine)),
            options: saved.options,
            arguments: saved.arguments,
            variables: saved.variables,
            dll_overrides: saved.dll_overrides,
        }
    }
}

/// What runs a Windows game's Wine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    /// Voile's Wine for macOS, under Rosetta.
    #[default]
    Rosetta,
    /// Voile's Wine for Linux, which voile-lkd's engine runs natively; for the
    /// games whose `profile` names it.
    Linux,
}

/// The graphics backend that renders a Windows game's Direct3D 10 to 12
/// through Metal. dxgi is one per process, so a game has one: on DXMT, it has
/// no Direct3D 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Graphics {
    /// DXMT: Direct3D 10 and 11. On the Linux engine, in Neptune's arm64
    /// render server.
    Dxmt,
    /// D3DMetal: Direct3D 11 and 12, for 64-bit programs only, so 32-bit
    /// games on Rosetta's Wine render through DXMT. On the Linux engine, in
    /// Neptune's Intel render server under Rosetta.
    D3DMetal,
}

impl Graphics {
    /// What renders on `engine` when neither the game's profile nor its
    /// player picks another backend.
    pub fn default_on(engine: Engine) -> Self {
        match engine {
            Engine::Rosetta => Self::D3DMetal,
            Engine::Linux => Self::Dxmt,
        }
    }

    /// The name Wine, Neptune and Obelisk's reports know it by.
    pub fn name(self) -> &'static str {
        match self {
            Self::D3DMetal => "d3dmetal",
            Self::Dxmt => "dxmt",
        }
    }
}

/// What a game asks of D3DMetal on Rosetta's Wine. The Linux engine's render
/// servers take none of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsOptions {
    /// Direct3D 12 offers DirectX Raytracing, as D3DMetal does by default.
    pub ray_tracing: bool,
}

impl Default for GraphicsOptions {
    fn default() -> Self {
        Self { ray_tracing: true }
    }
}

/// The Wine prefix of the Windows installation in `install`, named after the
/// installation's folder.
pub fn prefix(prefixes: &Path, install: &Path) -> Result<PathBuf, Error> {
    let name = install.file_name().ok_or(Error::NotExecutable)?;
    Ok(prefixes.join(name))
}

impl Config {
    fn wine(&self) -> PathBuf {
        self.runtime.join("bin/wine")
    }

    /// Whether the runtime is in place.
    pub fn installed(&self) -> bool {
        match self.engine {
            Engine::Rosetta => self.wine().is_file(),
            Engine::Linux => self.runtime.join("engine/fexdarwin-run").is_file(),
        }
    }

    /// How the game starts: Wine for macOS is an Intel program, the Linux
    /// engine's launcher a native one.
    pub(crate) fn architecture(&self) -> Architecture {
        match self.engine {
            Engine::Rosetta => ARCHITECTURE,
            Engine::Linux => Architecture::Arm64,
        }
    }

    /// Released runtimes carry D3DMetal. When one does, CrossOver's Wine
    /// preloads libd3dshared for D3DMetal's DLLs.
    fn d3dmetal(&self) -> Option<PathBuf> {
        let library = self.runtime.join("lib/external/libd3dshared.dylib");
        library.is_file().then_some(library)
    }

    /// The runtime's graphics backends, D3DMetal's DLLs then DXMT's, each in
    /// its own folder beside Wine's. A runtime from before them has
    /// D3DMetal's DLLs over Wine's own, and none.
    fn backends(&self) -> Vec<PathBuf> {
        self.dll_folders(&[D3DMETAL_DLLS, DXMT_DLLS])
    }

    /// The folders Wine searches before its own for the game's DLLs: its
    /// backend's, then, behind D3DMetal, DXMT's for the 32-bit programs
    /// D3DMetal does not run.
    fn graphics_folders(&self) -> Vec<PathBuf> {
        match self.graphics {
            Graphics::D3DMetal => self.backends(),
            Graphics::Dxmt => self.dll_folders(&[DXMT_DLLS]),
        }
    }

    /// Those of the DLL `folders` the runtime carries.
    fn dll_folders(&self, folders: &[&str]) -> Vec<PathBuf> {
        folders
            .iter()
            .map(|folder| self.runtime.join(folder))
            .filter(|folder| folder.is_dir())
            .collect()
    }

    /// The backend CrossOver's Wine is told renders, when the runtime
    /// carries it: Wine reports the hardware scheduling D3D12 expects from
    /// D3DMetal alone.
    fn active_graphics(&self) -> Option<&'static str> {
        let carried = match self.graphics {
            Graphics::D3DMetal => self.d3dmetal().is_some(),
            Graphics::Dxmt => self.runtime.join(DXMT_DLLS).is_dir(),
        };
        carried.then_some(self.graphics.name())
    }

    /// The GStreamer plugins the runtime carries, which decode games' videos
    /// and sounds through Media Foundation, DirectShow and WMReader.
    fn gstreamer_plugins(&self) -> Option<PathBuf> {
        let plugins = self.runtime.join("lib/gstreamer-1.0");
        plugins.is_dir().then_some(plugins)
    }

    /// The Wine Mono the runtime carries, such as `wine-mono-10.4.1`.
    fn mono(&self) -> Option<String> {
        fs::read_dir(self.runtime.join("share/wine/mono"))
            .ok()?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.starts_with("wine-mono-"))
            .max()
    }

    /// No Gecko installer prompt and no menu entries. .NET games run on the
    /// Wine Mono a runtime carries; without it, mscoree stays off so that no
    /// Mono installer prompt appears either. The profile's overrides follow.
    fn overrides(&self) -> String {
        let own = if self.mono().is_some() {
            "mshtml=;winemenubuilder.exe=d"
        } else {
            "mscoree,mshtml=;winemenubuilder.exe=d"
        };
        self.with_profile_overrides(own)
    }

    /// Wine's `own` DLL overrides, then the profile's.
    pub(crate) fn with_profile_overrides(&self, own: &str) -> String {
        std::iter::once(own.to_owned())
            .chain(self.profile_overrides())
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The profile's DLL overrides, each as `WINEDLLOVERRIDES` spells it.
    fn profile_overrides(&self) -> impl Iterator<Item = String> + '_ {
        self.dll_overrides
            .iter()
            .map(|(dll, mode)| format!("{dll}={mode}"))
    }

    /// Wine's errors, unless a development runtime's `winedebug` file names
    /// other channels, such as `+steamclient`.
    pub(crate) fn debug_channels(&self) -> String {
        fs::read_to_string(self.runtime.join("winedebug"))
            .ok()
            .and_then(|channels| channels.lines().next().map(str::to_owned))
            .filter(|channels| !channels.is_empty())
            .unwrap_or_else(|| "err+all,fixme-all".into())
    }

    fn environment(&self) -> HashMap<String, String> {
        let mut environment = HashMap::from([
            (
                "WINEPREFIX".to_owned(),
                self.prefix.to_string_lossy().into_owned(),
            ),
            ("WINEDLLOVERRIDES".to_owned(), self.overrides()),
            ("WINEDEBUG".to_owned(), self.debug_channels()),
            // Events, mutexes and semaphores become Mach semaphores instead of
            // wineserver requests. The server and every process of the prefix
            // must agree on it, so every Wine command sets it.
            ("WINEMSYNC".to_owned(), "1".to_owned()),
            // 32-bit games get 4 GB of address space, as on 64-bit Windows,
            // even when their executable does not ask for it.
            ("WINE_LARGE_ADDRESS_AWARE".to_owned(), "1".to_owned()),
            // Rosetta translates AVX and AVX2 but hides them from CPUID unless
            // asked, and some games refuse to start without them.
            ("ROSETTA_ADVERTISE_AVX".to_owned(), "1".to_owned()),
            // CrossOver turns off the W^X mode in which .NET 7 and 8 map
            // their JIT code: it breaks those programs under Rosetta.
            ("DOTNET_EnableWriteXorExecute".to_owned(), "0".to_owned()),
        ]);
        // Only the runtime's plugins load, never a GStreamer installed on the
        // Mac or in the home, and the prefix keeps their registry.
        if let Some(plugins) = self.gstreamer_plugins() {
            environment.insert(
                "GST_PLUGIN_SYSTEM_PATH".into(),
                plugins.to_string_lossy().into_owned(),
            );
            environment.insert(
                "GST_REGISTRY".into(),
                self.prefix
                    .join(GSTREAMER_REGISTRY)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        // CrossOver's Wine preloads libd3dshared whatever renders.
        if let Some(library) = self.d3dmetal() {
            environment.insert(
                "CX_APPLEGPTK_LIBD3DSHARED_PATH".into(),
                library.to_string_lossy().into_owned(),
            );
        }
        // Outside Apple's own layout, D3DMetal's unix libraries do not find
        // their framework by themselves.
        let framework = self.runtime.join(D3DMETAL_FRAMEWORK);
        if framework.is_file() {
            environment.insert(
                "D3DMETAL_FRAMEWORK_PATH".into(),
                framework.to_string_lossy().into_owned(),
            );
        }
        if let Some(folders) = search_path(&self.graphics_folders()) {
            environment.insert("WINEDLLPATH_PREPEND".into(), folders);
        }
        if let Some(active) = self.active_graphics() {
            environment.insert("CX_ACTIVE_GRAPHICS_BACKEND".into(), active.into());
        }
        // Direct3D 12 offers DirectX Raytracing unless D3DMetal is told not to.
        if self.graphics == Graphics::D3DMetal && !self.options.ray_tracing {
            environment.insert("D3DM_SUPPORT_DXR".into(), "0".into());
        }
        // The profile's variables come last, so that they may replace
        // Obelisk's own for their game.
        environment.extend(self.variables.clone());
        environment
    }

    /// The line the game's log starts with: how Obelisk runs it, for whoever
    /// reads the log, such as a bug report's reader. Of the arguments, only
    /// those its player adds: a storefront's may carry credentials.
    pub(crate) fn log_header(&self) -> String {
        let engine = match self.engine {
            Engine::Rosetta => "wine-macos",
            Engine::Linux => "wine-linux",
        };
        let mut header = format!("obelisk: engine={engine} graphics={}", self.graphics.name());
        let ray_tracing = crate::launch_settings::offer(self.engine)
            .graphics(self.graphics)
            .is_some_and(|offer| {
                offer
                    .options
                    .contains(&crate::launch_settings::GraphicsOption::RayTracing)
            });
        if ray_tracing {
            let state = if self.options.ray_tracing {
                "on"
            } else {
                "off"
            };
            header.push_str(&format!(" ray_tracing={state}"));
        }
        // Only a development runtime has no manifest.
        let voile = crate::voile::release_of(&self.runtime);
        header.push_str(&format!(
            " voile={}",
            voile.as_deref().unwrap_or("development")
        ));
        if !self.variables.is_empty() {
            let variables: Vec<_> = self
                .variables
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            header.push_str(&format!(" variables={}", typed(&variables)));
        }
        if !self.dll_overrides.is_empty() {
            let overrides: Vec<_> = self.profile_overrides().collect();
            header.push_str(&format!(" dll_overrides={}", overrides.join(";")));
        }
        if !self.arguments.is_empty() {
            header.push_str(&format!(" arguments={}", typed(&self.arguments)));
        }
        header.push('\n');
        header
    }
}

/// `folders` as one search path, or `None` when there are none.
fn search_path(folders: &[PathBuf]) -> Option<String> {
    let folders: Vec<_> = folders
        .iter()
        .map(|folder| folder.to_string_lossy())
        .collect();
    (!folders.is_empty()).then(|| folders.join(":"))
}

/// What one storefront adds to the prefix of its Windows installations, and
/// to the games that start there.
pub trait Storefront {
    /// The Windows program its games start behind, given the game's program
    /// and arguments, such as Steam's steam.exe.
    const HELPER: Option<&'static str> = None;
    /// Variables its games read that exist only once their Game session
    /// starts them, such as the Steam host's ports.
    const PASSED: &'static [&'static str] = &[];
    /// Whether its games reach Valve's steamclient through lsteamclient.
    const STEAM_CLIENT: bool = false;
    /// The programs it names to run once in a prefix before a game first
    /// starts, in order.
    fn prerequisites(&self) -> Result<Vec<Prerequisite>, Error>;
    /// Places its own files in the prefix, before any prerequisite runs.
    fn furnish(&self, _config: &Config) -> Result<(), Error> {
        Ok(())
    }
}

/// A storefront's Windows game as Wine starts it: the game its profile is
/// looked up by, its installation's folder, and its name in the store.
pub(crate) struct Game<'a> {
    pub id: crate::profile::Game<'a>,
    pub install: &'a Path,
    pub title: &'a str,
}

/// Readies the prefix of the Windows installation in `install` to start
/// `target`: the game must be a Windows program inside the installation, then
/// the prefix is created or updated for the runtime, the storefront furnishes
/// it and its prerequisites run. Nothing touches the prefix for a game that
/// cannot start.
fn prepare<S: Storefront>(
    config: &Config,
    home: &Path,
    install: &Path,
    target: &LaunchTarget,
    storefront: &S,
) -> Result<(), Error> {
    if config.engine == Engine::Linux {
        return crate::linux_engine::prepare(config, install, target, storefront);
    }
    program(install, target)?;
    boot(config, home)?;
    storefront.furnish(config)?;
    install_prerequisites(config, home, &storefront.prerequisites()?)?;
    match config.log.parent() {
        Some(logs) => crate::files::private_directory(logs),
        None => Err(Error::Storage),
    }
}

/// The game `target` names, or the program its profile names instead, that
/// Wine starts in the prefix with `environment` and Wine's own settings, as S
/// completes it, logging to the installation's log.
pub(crate) fn target<S: Storefront>(
    config: &Config,
    game: &Game<'_>,
    target: &LaunchTarget,
    environment: HashMap<String, String>,
) -> Result<Target, Error> {
    let mut launched = crate::profile::of(game.id, game.install).target(game.install, target);
    launched.arguments.extend(config.arguments.iter().cloned());
    if config.engine == Engine::Linux {
        return crate::linux_engine::target::<S>(
            game.install,
            &launched,
            game.title,
            environment,
            config,
        );
    }
    let executable = program(game.install, &launched)?;
    let mut arguments: Vec<String> = S::HELPER.map(str::to_owned).into_iter().collect();
    arguments.push(executable.to_string_lossy().into_owned());
    arguments.extend(launched.arguments.iter().cloned());
    let mut environment = environment;
    environment.extend(config.environment());
    Ok(Target {
        executable: config.wine(),
        arguments,
        working_directory: launched.working_directory.into(),
        environment,
        log: Some(config.log.clone()),
        installation: Some(game.install.to_owned()),
        program: Some(executable),
    })
}

/// A file the game reads, in a private folder that lives as long as the
/// returned TempDir, and its Windows path. Rosetta's Wine reads it on the
/// Mac's disk, through its Z: drive; the engine's guest, which cannot see
/// that disk, in the prefix's C: drive.
pub(crate) fn private_file(
    config: &Config,
    name: &str,
    bytes: &[u8],
) -> Result<(tempfile::TempDir, String), Error> {
    if config.engine == Engine::Linux {
        return crate::linux_engine::private_file(config, name, bytes);
    }
    let (directory, path) =
        crate::files::private_temp_file(&std::env::temp_dir(), "obelisk-", name, bytes)?;
    Ok((directory, windows_path(&path)))
}

/// The Windows program `target` names, when it is one inside `install`.
pub(crate) fn program(install: &Path, target: &LaunchTarget) -> Result<PathBuf, Error> {
    let root = install.canonicalize()?;
    let executable = Path::new(&target.executable)
        .canonicalize()
        .map_err(|_| Error::NotExecutable)?;
    if !executable.starts_with(&root) || !portable_executable(&executable)? {
        return Err(Error::NotExecutable);
    }
    Ok(executable)
}

/// Creates the prefix on first use, registers the runtime's Wine Mono in it,
/// and keeps its user folders apart from the Mac home.
fn boot(config: &Config, home: &Path) -> Result<(), Error> {
    if !config.installed() {
        return Err(Error::WineUnavailable);
    }
    crate::files::private_directory(&config.prefix)?;
    let created = !config.prefix.join("system.reg").is_file();
    if created {
        wineboot(config, home, "--init")?;
    }
    // wineboot installs Mono's .NET registration and a placeholder for every
    // DLL of the graphics backends when it creates a prefix: Wine loads a
    // DLL only through its placeholder, such as DXMT's winemetal. A prefix
    // created by a runtime without Mono or without one of the backends lacks
    // them: updating the prefix once adds them.
    let backends = config
        .backends()
        .iter()
        .filter_map(|folder| folder.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let records = [(MONO, config.mono()), (BACKENDS, Some(backends))];
    let stale: Vec<_> = records
        .into_iter()
        .filter_map(|(record, value)| Some((config.prefix.join(record), value?)))
        .filter(|(record, value)| fs::read_to_string(record).unwrap_or_default() != *value)
        .collect();
    if !stale.is_empty() {
        if !created {
            wineboot(config, home, "--update")?;
        }
        for (record, value) in stale {
            fs::write(record, value)?;
        }
    }
    unlink_home_folders(&profile(&config.prefix))
}

/// Runs wineboot in the prefix and waits until its server stops writing the
/// registry, which the game must not start before. Every backend's folder
/// comes first, so that the prefix gets a placeholder for each of their DLLs.
fn wineboot(config: &Config, home: &Path, argument: &str) -> Result<(), Error> {
    let mut environment = config.environment();
    if let Some(folders) = search_path(&config.backends()) {
        environment.insert("WINEDLLPATH_PREPEND".into(), folders);
    }
    let status = Command::new(config.wine())
        .args(["wineboot", argument])
        .env_clear()
        .envs(crate::native::environment(home))
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    let server = Command::new(config.runtime.join("bin/wineserver"))
        .arg("-w")
        .env_clear()
        .env("WINEPREFIX", &config.prefix)
        .status()?;
    if !status.success() || !server.success() {
        return Err(Error::StartFailed);
    }
    Ok(())
}

/// Games save into the profile's Documents and similar folders, which Cloud
/// syncs and never follows through links. A prefix created before the runtime
/// stopped linking them to the Mac home still holds those links: each becomes
/// an empty folder, and the Mac folder it pointed at stays untouched.
fn unlink_home_folders(profile: &Path) -> Result<(), Error> {
    for (folder, recreate) in [(profile.to_owned(), true), (profile.join("Desktop"), false)] {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                fs::remove_file(entry.path())?;
                if recreate {
                    fs::create_dir(entry.path())?;
                }
            }
        }
    }
    Ok(())
}

/// A program run once in a prefix before its game starts, such as a DirectX
/// installer. Storefronts name them: Steam in install scripts, Epic in its
/// manifests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prerequisite {
    /// A prefix runs each id once.
    pub id: String,
    /// Run in order, each with its Windows command line.
    pub programs: Vec<(PathBuf, String)>,
}

/// Where `path` is inside every prefix: Wine's Z: drive is the Mac's root.
pub(crate) fn windows_path(path: &Path) -> String {
    format!("Z:{}", path.to_string_lossy().replace('/', "\\"))
}

/// One Windows installation: its game runs in Wine, in the installation's own
/// prefix, where its saves live too. The prefix is readied before each launch
/// and goes when the installation is uninstalled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installation {
    install: PathBuf,
    prefix: PathBuf,
    /// Where Wine logs while its game runs.
    log: PathBuf,
    profile: crate::profile::Profile,
    /// What its player adds to its game's command line.
    arguments: Vec<String>,
}

impl Installation {
    /// The installation of `game` in `install`, when `platform` names a
    /// Windows build, run as the game's profile and its player's launch
    /// settings say.
    pub fn of(
        locations: &crate::Locations,
        game: crate::profile::Game<'_>,
        platform: &str,
        install: &Path,
    ) -> Result<Option<Self>, Error> {
        if !windows_build(platform) {
            return Ok(None);
        }
        let choices = crate::launch_settings::load(&locations.library, install);
        let arguments = command_line(choices.arguments.as_deref().unwrap_or_default());
        let profile = choices.apply(&crate::profile::of(game, install));
        let prefix = prefix(&locations.wine_prefixes, install)?;
        let prefix = match profile.engine {
            Engine::Rosetta => prefix,
            Engine::Linux => crate::linux_engine::prefix(&prefix)?,
        };
        Ok(Some(Self {
            install: install.to_owned(),
            prefix,
            log: log(locations, game),
            profile,
            arguments,
        }))
    }

    /// The Windows machine the installation runs in.
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// Readies the prefix, with `storefront`'s additions, to start `target`
    /// or the program the game's profile names instead, and returns how its
    /// game runs. D3DMetal is translated before a game that renders with it
    /// starts.
    pub fn ready<S: Storefront>(
        &self,
        locations: &crate::Locations,
        target: &LaunchTarget,
        storefront: &S,
    ) -> Result<Config, Error> {
        let config = self.runtime(locations)?;
        let target = self.profile.target(&self.install, target);
        prepare(&config, &locations.home, &self.install, &target, storefront)?;
        crate::linux_engine::translate_d3dmetal(&config, &locations.helpers);
        Ok(config)
    }

    /// How its game runs, once Voile is installed.
    fn runtime(&self, locations: &crate::Locations) -> Result<Config, Error> {
        let runtime = match self.profile.engine {
            Engine::Rosetta => crate::voile::runtime(&locations.voile),
            Engine::Linux => crate::voile::linux_runtime(&locations.voile),
        };
        Ok(Config {
            runtime: runtime.ok_or(Error::WineUnavailable)?,
            prefix: self.prefix.clone(),
            log: self.log.clone(),
            engine: self.profile.engine,
            graphics: self.profile.graphics,
            options: self.profile.options,
            arguments: self.arguments.clone(),
            variables: self.profile.variables.clone(),
            dll_overrides: self.profile.dll_overrides.clone(),
        })
    }

    /// Deletes the prefixes of the uninstalled installation, as Steam deletes
    /// a Proton prefix, and its launch settings. Both engines' go, whichever
    /// runs the game now: an earlier launch may have made the other's before
    /// the game moved. A missing prefix is already gone; a link in its place
    /// is never followed.
    pub fn remove(
        &self,
        locations: &crate::Locations,
    ) -> Result<(), obelisk_core::installation::RemovalFailure> {
        use obelisk_core::installation::RemovalFailure;
        let unsafe_prefix = |_| RemovalFailure::Unsafe(self.prefix.clone());
        let rosetta = prefix(&locations.wine_prefixes, &self.install).map_err(unsafe_prefix)?;
        let linux = crate::linux_engine::prefix(&rosetta).map_err(unsafe_prefix)?;
        remove_folder(crate::linux_engine::home(&linux).map_err(unsafe_prefix)?)?;
        remove_prefix(locations, &rosetta)?;
        crate::launch_settings::remove(&locations.library, &self.install)?;
        Ok(())
    }
}

/// Whether `platform` names a Windows build, as a storefront spells it:
/// Steam's and GOG's `windows`, Epic's `Windows`.
pub(crate) fn windows_build(platform: &str) -> bool {
    platform.eq_ignore_ascii_case("windows")
}

/// Where Wine logs `game`: in its storefront's diagnostics folder.
fn log(locations: &crate::Locations, game: crate::profile::Game<'_>) -> PathBuf {
    crate::locations::game_log(&locations.home, game, "wine")
}

/// Deletes a Rosetta prefix, after its Wine server.
fn remove_prefix(
    locations: &crate::Locations,
    prefix: &Path,
) -> Result<(), obelisk_core::installation::RemovalFailure> {
    // A Wine server lingers a few seconds after its game and would write the
    // registry back into the prefix.
    if prefix.is_dir()
        && let Some(runtime) = crate::voile::runtime(&locations.voile)
    {
        let _ = wineserver(&runtime, prefix, "-k");
    }
    remove_folder(prefix)
}

/// Deletes `folder`. A missing one is already gone; a link in its place is
/// never followed.
fn remove_folder(folder: &Path) -> Result<(), obelisk_core::installation::RemovalFailure> {
    use obelisk_core::installation::RemovalFailure;
    match fs::symlink_metadata(folder) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if !metadata.is_dir() => {
            return Err(RemovalFailure::Unsafe(folder.to_owned()));
        }
        Ok(_) => {}
    }
    Ok(fs::remove_dir_all(folder)?)
}

/// Runs, in order, the prerequisites this prefix has not run yet. One that
/// fails or outlasts its timeout runs again on the next launch; the game still
/// starts, and may tell what it misses.
fn install_prerequisites(
    config: &Config,
    home: &Path,
    prerequisites: &[Prerequisite],
) -> Result<(), Error> {
    let record = config.prefix.join(PREREQUISITES);
    let mut ran: BTreeSet<String> = match fs::read_to_string(&record) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => return Err(error.into()),
    };
    let pending: Vec<_> = prerequisites
        .iter()
        .filter(|prerequisite| !prerequisite.id.contains('\n') && !ran.contains(&prerequisite.id))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let log = fs::File::create(config.prefix.join(PREREQUISITES_LOG))?;
    for prerequisite in pending {
        if run_prerequisite(config, home, prerequisite, &log)? {
            ran.insert(prerequisite.id.clone());
            let text: Vec<_> = ran.iter().map(String::as_str).collect();
            obelisk_core::write_atomic(&record, text.join("\n").as_bytes())?;
        }
    }
    // The game must not start while an installer's processes still run.
    wineserver(&config.runtime, &config.prefix, "-w")?;
    Ok(())
}

/// Whether every program of `prerequisite` ran and reported success.
fn run_prerequisite(
    config: &Config,
    home: &Path,
    prerequisite: &Prerequisite,
    log: &fs::File,
) -> Result<bool, Error> {
    for (program, arguments) in &prerequisite.programs {
        let Ok(program) = program.canonicalize() else {
            return Ok(false);
        };
        let mut command = Command::new(config.wine());
        // Wine starts scripts only through cmd, which reads Windows paths.
        let script = program
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat")
            });
        if script {
            command.args(["cmd", "/c"]).arg(windows_path(&program));
        } else {
            command.arg(&program);
        }
        let mut environment = config.environment();
        // Wine's msvcp140, msvcp140_2 and vcruntime140_1 claim the version of
        // the newest Visual C++ runtime, so its installer keeps them over
        // Microsoft's. With their versions hidden, it installs Microsoft's,
        // which a game whose profile loads them native first then gets.
        if visual_cpp_installer(&program) {
            environment.insert("CX_HIDE_BUILTIN_VERSION_RESOURCES".into(), "1".into());
        }
        command
            .args(command_line(arguments))
            .current_dir(program.parent().unwrap_or(&config.prefix))
            .env_clear()
            .envs(crate::native::environment(home))
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?);
        let code = wait(command.spawn()?, PREREQUISITE_TIMEOUT)?;
        if code.is_none() {
            wineserver(&config.runtime, &config.prefix, "-k")?;
        }
        if !code.is_some_and(|code| INSTALLED.contains(&code)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `program` installs a Visual C++ runtime, as Microsoft names its
/// installers: vc_redist or, before 2015, vcredist.
fn visual_cpp_installer(program: &Path) -> bool {
    program
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|name| name.starts_with("vc_redist") || name.starts_with("vcredist"))
}

/// The exit code of `child`, or `None` once it ran for `timeout` and was killed.
pub(crate) fn wait(mut child: Child, timeout: Duration) -> Result<Option<i32>, Error> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status.code().unwrap_or(-1)));
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Ends every process of the prefix: Wine asks them to quit, then kills
/// those that remain, including the ones it started apart from the game.
pub(crate) fn stop(config: &Config) {
    // The engine's processes follow its launcher, which the session stops.
    if config.engine == Engine::Rosetta {
        let _ = wineserver(&config.runtime, &config.prefix, "-k");
    }
}

/// Asks the Windows game to quit as its player closing its windows would, and
/// kills nothing. `environment` is the game's own, `pid` its launch helper's.
pub(crate) fn quit(
    config: &Config,
    environment: &HashMap<String, String>,
    program: Option<&Path>,
    pid: u32,
) {
    if let Some(command) = quit_command(config, environment, program, pid) {
        let _ = run(command, QUIT_TIMEOUT);
    }
}

/// In Wine for macOS, taskkill without /F posts WM_CLOSE to the windows of
/// the game's program and of the processes it started, in the game's prefix
/// and with its environment, which its Wine server must agree with. The Linux
/// engine's window server asks every window of the game to close, as its own
/// Quit menu does; a game that shows no window yet is asked nothing.
fn quit_command(
    config: &Config,
    environment: &HashMap<String, String>,
    program: Option<&Path>,
    pid: u32,
) -> Option<Command> {
    let command = match config.engine {
        Engine::Rosetta => {
            let mut command = Command::new(config.wine());
            command
                .args(["taskkill", "/T", "/IM"])
                .arg(program?.file_name()?)
                .env_clear()
                .envs(environment);
            command
        }
        Engine::Linux => {
            let mut command = Command::new(config.runtime.join("engine/wsdctl"));
            command
                .arg(crate::linux_engine::window_server(environment, pid))
                .args(["window", "quit"])
                .env_clear();
            command
        }
    };
    Some(command)
}

/// Kills every process of the prefix at once. Unlike `stop`, it never waits
/// on a Wine server frozen with its game for longer than `QUIT_TIMEOUT`.
pub(crate) fn kill(config: &Config) {
    // The engine's processes follow its launcher, which the session kills.
    if config.engine == Engine::Rosetta {
        let mut command = Command::new(config.runtime.join("bin/wineserver"));
        command
            .arg("-k")
            .env_clear()
            .env("WINEPREFIX", &config.prefix);
        let _ = run(command, QUIT_TIMEOUT);
    }
}

/// The folders whose files only this prefix's processes use: the prefix, and
/// for Wine for macOS the folder of its Wine server, which Wine names after
/// the prefix's device and inode.
pub(crate) fn private_folders(config: &Config) -> Vec<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    match config.engine {
        Engine::Rosetta => {
            let mut folders = vec![config.prefix.clone()];
            if let Ok(prefix) = fs::metadata(&config.prefix) {
                folders.push(PathBuf::from(format!(
                    "/tmp/.wine-{}/server-{:x}-{:x}",
                    prefix.uid(),
                    prefix.dev(),
                    prefix.ino()
                )));
            }
            folders
        }
        Engine::Linux => crate::linux_engine::home(&config.prefix)
            .map(|home| vec![home.to_owned()])
            .unwrap_or_default(),
    }
}

/// Runs `command` without input or output for at most `timeout`.
fn run(mut command: Command, timeout: Duration) -> Result<Option<i32>, Error> {
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    wait(child, timeout)
}

/// Ends the Wine processes an Obelisk that died left behind, and returns how
/// many. macOS ends every process of an Obelisk that dies, its sessions'
/// supervisors and Steam hosts included, but not the Windows processes Wine
/// starts apart: those froze without their session, and nothing else runs
/// from `voile`.
pub fn end_orphans(voile: &Path) -> usize {
    let mut command = Command::new("/usr/sbin/lsof");
    command.args(["-n", "-P", "-d", "txt", "-Fpn"]);
    let Some(listing) = crate::supervisor::quit::listing(command) else {
        return 0;
    };
    let voile = voile.canonicalize().unwrap_or_else(|_| voile.to_owned());
    let orphans = running_from(&listing, &voile, std::process::id());
    crate::supervisor::quit::kill(&orphans);
    orphans.len()
}

/// The processes of an `lsof -Fpn` listing that run files from `root`.
fn running_from(listing: &str, root: &Path, own: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut pid = None;
    for line in listing.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok().filter(|pid| *pid != own);
        } else if let Some(path) = line.strip_prefix('n')
            && let Some(current) = pid
            && Path::new(path).starts_with(root)
            && !found.contains(&current)
        {
            found.push(current);
        }
    }
    found
}

/// Waits for, or stops, the Wine server of `prefix`. Neither starts a server,
/// so no Wine setting is needed.
fn wineserver(runtime: &Path, prefix: &Path, argument: &str) -> Result<(), Error> {
    Command::new(runtime.join("bin/wineserver"))
        .arg(argument)
        .env_clear()
        .env("WINEPREFIX", prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

/// Splits a Windows command line on the spaces outside double quotes.
fn command_line(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in line.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(character);
                started = true;
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

/// `arguments` as a command line `command_line` splits back into them.
fn typed(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if argument.is_empty() || argument.contains([' ', '\t']) {
                format!("\"{argument}\"")
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `path` starts like a Windows program: a DOS header naming a PE header.
fn portable_executable(path: &Path) -> Result<bool, Error> {
    let mut file = fs::File::open(path)?;
    let mut dos = [0u8; 64];
    if file.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" {
        return Ok(false);
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().expect("four bytes")) as u64;
    let mut signature = [0u8; 4];
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    Ok(file.read_exact(&mut signature).is_ok() && &signature == b"PE\0\0")
}

#[cfg(test)]
pub(crate) mod tests;
