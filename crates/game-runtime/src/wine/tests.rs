use super::*;

/// The smallest file `portable_executable` accepts: a DOS header whose last
/// field points just past it, at a PE signature.
pub(crate) fn pe() -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
    bytes.extend_from_slice(b"PE\0\0");
    bytes
}

pub(crate) fn installation() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

pub(crate) fn target(executable: &Path, arguments: &[&str]) -> LaunchTarget {
    LaunchTarget {
        executable: executable.to_string_lossy().into_owned(),
        arguments: arguments
            .iter()
            .map(|argument| (*argument).into())
            .collect(),
        working_directory: executable.parent().unwrap().to_string_lossy().into_owned(),
        is_application_bundle: false,
    }
}

/// A game no profile names.
const OTHER: crate::profile::Game<'static> = crate::profile::Game::Steam { app_id: 42 };
/// A game whose profile names the Linux engine and D3DMetal.
const ELDEN_RING: crate::profile::Game<'static> = crate::profile::Game::Steam { app_id: 1245620 };

/// Obelisk's folders under `root`: its library, its bundle and its home.
pub(crate) fn locations(root: &Path) -> crate::Locations {
    crate::Locations::new(
        &root.join("library"),
        &root.join("Obelisk.app"),
        &root.join("home"),
    )
}

/// Installs the Linux engine's runtime as Voile's development runtime: its
/// engine, its Wine and Neptune's Direct3D, each a placeholder.
pub(crate) fn engine_runtime(locations: &crate::Locations) -> PathBuf {
    let runtime = locations.voile.join("current/linux");
    for file in ["engine/fexdarwin-run", "wine/bin/wine"] {
        fs::create_dir_all(runtime.join(file).parent().unwrap()).unwrap();
        fs::write(runtime.join(file), "").unwrap();
    }
    fs::create_dir_all(runtime.join("windows")).unwrap();
    for library in ["d3d11.dll", "dxgi.dll", "d3d12.dll"] {
        fs::write(runtime.join("windows").join(library), library).unwrap();
    }
    runtime
}

pub(crate) fn config(root: &Path) -> Config {
    Config {
        runtime: root.join("runtime"),
        prefix: root.join("prefix"),
        log: root.join("logs/wine-game.log"),
        engine: Engine::Rosetta,
        graphics: Graphics::D3DMetal,
        options: GraphicsOptions::default(),
        arguments: Vec::new(),
        variables: Default::default(),
        dll_overrides: Default::default(),
    }
}

/// A storefront that adds nothing, or the prerequisites it is given.
#[derive(Default)]
struct Plain {
    prerequisites: Vec<Prerequisite>,
}
impl Storefront for Plain {
    fn prerequisites(&self) -> Result<Vec<Prerequisite>, Error> {
        Ok(self.prerequisites.clone())
    }
    fn furnish(&self, config: &Config) -> Result<(), Error> {
        fs::write(config.prefix.join("furnished"), "")?;
        Ok(())
    }
}

/// A storefront whose games start behind its helper and read a variable
/// their session sets.
struct Behind;
impl Storefront for Behind {
    const HELPER: Option<&'static str> = Some(r"C:\helper.exe");
    const PASSED: &'static [&'static str] = &["SESSION_PORT"];
    fn prerequisites(&self) -> Result<Vec<Prerequisite>, Error> {
        Ok(Vec::new())
    }
}

/// The game no profile names, installed in `install`, without a store title.
fn plain(install: &Path) -> Game<'_> {
    Game {
        id: OTHER,
        install,
        title: "",
    }
}

/// The engine's configuration of a prefix beside `root`.
fn linux_config(root: &Path) -> Config {
    Config {
        prefix: root.join("Prefixes/game-linux/prefix"),
        engine: Engine::Linux,
        ..config(root)
    }
}

/// A Windows game inside `root/game`, and its target.
fn game(root: &Path) -> (PathBuf, LaunchTarget) {
    let install = root.join("game");
    fs::create_dir_all(&install).unwrap();
    fs::write(install.join("Game.exe"), pe()).unwrap();
    let target = target(&install.join("Game.exe"), &[]);
    (install, target)
}

/// The environment Wine commands of `config` run with.
fn environment(config: &Config) -> HashMap<String, String> {
    let (install, target) = game(config.prefix.parent().unwrap());
    super::target::<Plain>(config, &plain(&install), &target, HashMap::new())
        .unwrap()
        .environment
}

#[test]
fn a_windows_game_starts_in_wine_itself_when_no_storefront_helper_stands_in() {
    let (_directory, root) = installation();
    let install = root.join("game");
    fs::create_dir_all(install.join("Binaries")).unwrap();
    fs::write(install.join("Binaries/Game.exe"), pe()).unwrap();
    let config = config(&root);

    let target = super::target::<Plain>(
        &config,
        &plain(&install),
        &target(&install.join("Binaries/Game.exe"), &["-EpicPortal"]),
        HashMap::from([("HOME".to_owned(), "/home".to_owned())]),
    )
    .unwrap();

    assert_eq!(target.executable, root.join("runtime/bin/wine"));
    assert_eq!(
        target.arguments,
        [
            install.join("Binaries/Game.exe").to_string_lossy().as_ref(),
            "-EpicPortal"
        ]
    );
    assert_eq!(target.working_directory, install.join("Binaries"));
    assert_eq!(target.environment["HOME"], "/home");
    assert_eq!(target.environment["WINEMSYNC"], "1");
    assert_eq!(
        target.environment["WINEPREFIX"],
        root.join("prefix").to_string_lossy()
    );
    assert_eq!(target.log, Some(root.join("logs/wine-game.log")));
}

#[test]
fn only_a_windows_program_inside_the_installation_starts_and_readies_a_prefix() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let install = root.join("game");
    fs::create_dir_all(&install).unwrap();
    fs::write(install.join("script.exe"), b"#!/bin/sh\n").unwrap();
    fs::write(root.join("outside.exe"), pe()).unwrap();

    for executable in [install.join("script.exe"), root.join("outside.exe")] {
        let target = target(&executable, &[]);
        assert!(matches!(
            super::target::<Plain>(&config, &plain(&install), &target, HashMap::new()),
            Err(Error::NotExecutable)
        ));
        assert!(matches!(
            prepare(&config, &root, &install, &target, &Plain::default()),
            Err(Error::NotExecutable)
        ));
    }
    assert!(calls(&root).is_empty());
    assert!(!config.prefix.join("furnished").exists());
}

#[test]
fn an_engine_game_takes_its_store_title_in_the_dock_without_trademark_signs_or_else_its_program_s_name()
 {
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("RocketLeague_EAC.exe");
    fs::write(&executable, pe()).unwrap();
    let config = linux_config(&root);
    let named = |title: &str| {
        let game = Game {
            title,
            ..plain(&install)
        };
        super::target::<Plain>(&config, &game, &target(&executable, &[]), HashMap::new())
            .unwrap()
            .environment[crate::linux_engine::APP_NAME]
            .clone()
    };

    assert_eq!(named("Rocket League® "), "Rocket League");
    assert_eq!(named(""), "RocketLeague_EAC");
}

#[test]
fn a_storefront_s_helper_and_session_variables_reach_its_game_on_either_engine() {
    use crate::linux_engine::tests::{guest_command, options};
    let (_directory, root) = installation();
    let (install, launched) = game(&root);
    let program = install.join("Game.exe").to_string_lossy().into_owned();
    let (rosetta, linux) = (config(&root), linux_config(&root));

    let behind =
        super::target::<Behind>(&rosetta, &plain(&install), &launched, HashMap::new()).unwrap();
    assert_eq!(behind.arguments, [r"C:\helper.exe", program.as_str()]);
    let behind =
        super::target::<Behind>(&linux, &plain(&install), &launched, HashMap::new()).unwrap();
    assert_eq!(
        guest_command(&behind),
        [r"C:\helper.exe", "/opt/game/Game.exe"]
    );
    assert_eq!(options(&behind, "--pass"), ["SESSION_PORT"]);

    let alone =
        super::target::<Plain>(&rosetta, &plain(&install), &launched, HashMap::new()).unwrap();
    assert_eq!(alone.arguments, [program.as_str()]);
    let alone =
        super::target::<Plain>(&linux, &plain(&install), &launched, HashMap::new()).unwrap();
    assert_eq!(guest_command(&alone), ["/opt/game/Game.exe"]);
    assert!(options(&alone, "--pass").is_empty());
}

#[test]
fn a_private_file_is_read_through_the_z_drive_on_rosetta_and_from_the_c_drive_on_the_engine() {
    let (_directory, root) = installation();
    for config in [config(&root), linux_config(&root)] {
        fs::create_dir_all(config.prefix.join("drive_c")).unwrap();

        let (folder, path) = private_file(&config, "ownership.bin", b"token").unwrap();

        let file = match config.engine {
            Engine::Rosetta => PathBuf::from(path.strip_prefix("Z:").unwrap().replace('\\', "/")),
            Engine::Linux => config
                .prefix
                .join("drive_c")
                .join(path.strip_prefix("C:\\").unwrap().replace('\\', "/")),
        };
        assert!(path.ends_with("\\ownership.bin"), "{path}");
        assert_eq!(fs::read(&file).unwrap(), b"token");
        drop(folder);
        assert!(!file.exists());
    }
}

/// Where Wine logs a storefront's games.
type Diagnostics = fn(&crate::Locations) -> &PathBuf;

#[test]
fn each_storefront_s_game_is_readied_in_its_prefix_and_logs_into_its_store_s_diagnostics_folder() {
    use crate::profile::Game::{Epic, Gog, Steam};
    let rows: [(crate::profile::Game, &str, Diagnostics, &str); 3] = [
        (
            Steam { app_id: 42 },
            "Steam/steam-42-windows",
            |locations| &locations.steam_diagnostics,
            "wine-42.log",
        ),
        (
            Epic {
                namespace: "space",
                app_name: "game",
            },
            "Epic/epic-hash-Windows",
            |locations| &locations.epic_diagnostics,
            "wine-game.log",
        ),
        (
            Gog {
                product_id: "1207658924",
            },
            "GOG/gog-1207658924-windows",
            |locations| &locations.gog_diagnostics,
            "wine-1207658924.log",
        ),
    ];
    for (game, folder, diagnostics, log) in rows {
        let (_directory, root) = installation();
        let locations = locations(&root);
        record_calls(&locations.voile.join("current"));
        let install = locations.library.join("Games").join(folder);
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("Game.exe"), pe()).unwrap();
        fs::write(install.join("setup.exe"), "").unwrap();
        let storefront = Plain {
            prerequisites: vec![Prerequisite {
                id: "setup".into(),
                programs: vec![(install.join("setup.exe"), "/quiet".into())],
            }],
        };
        let windows = Installation::of(&locations, game, "windows", &install)
            .unwrap()
            .unwrap();

        let config = windows
            .ready(
                &locations,
                &target(&install.join("Game.exe"), &[]),
                &storefront,
            )
            .unwrap();

        let prefix = locations.wine_prefixes.join(install.file_name().unwrap());
        assert_eq!(windows.prefix(), prefix);
        assert_eq!(config.prefix, prefix);
        assert_eq!(config.log, diagnostics(&locations).join(log));
        assert!(diagnostics(&locations).is_dir(), "{game:?}");
        assert_eq!(
            calls(&locations.wine_prefixes),
            [
                "wineboot --init".to_owned(),
                format!("{} /quiet", install.join("setup.exe").display()),
            ]
        );
        assert!(prefix.join("furnished").is_file());
    }
}

#[test]
fn the_program_a_profile_names_instead_is_the_one_readied_and_the_one_started() {
    use crate::linux_engine::tests::guest_command;
    let rocket_league = crate::profile::Game::Epic {
        namespace: "9773aa1aa54f4f7b80e44bef04986cea",
        app_name: "Sugar",
    };
    let (_directory, root) = installation();
    let locations = locations(&root);
    engine_runtime(&locations);
    record_calls(&locations.voile.join("current"));
    let install = locations.library.join("Games/Epic/epic-rl-Windows");
    fs::create_dir_all(install.join("Binaries/Win64")).unwrap();
    // The publisher's program is no Windows program Wine could start.
    fs::write(install.join("Launcher.exe"), b"#!/bin/sh\n").unwrap();
    fs::write(install.join("Binaries/Win64/RocketLeague_EAC.exe"), pe()).unwrap();
    let launcher = target(&install.join("Launcher.exe"), &["-AUTH_TYPE=exchangecode"]);
    let windows = Installation::of(&locations, rocket_league, "Windows", &install)
        .unwrap()
        .unwrap();
    // A prefix the engine already created.
    fs::create_dir_all(windows.prefix().join("drive_c/windows/system32")).unwrap();
    fs::write(windows.prefix().join("system.reg"), "registry").unwrap();

    let config = windows
        .ready(&locations, &launcher, &Plain::default())
        .unwrap();
    let game = Game {
        id: rocket_league,
        ..plain(&install)
    };
    let started = super::target::<Plain>(&config, &game, &launcher, HashMap::new()).unwrap();

    assert_eq!(
        guest_command(&started),
        [
            "/opt/game/Binaries/Win64/RocketLeague_EAC.exe",
            "-AUTH_TYPE=exchangecode"
        ]
    );
    // The engine's prefix renders through Neptune's Direct3D.
    assert_eq!(
        fs::read(windows.prefix().join("drive_c/windows/system32/d3d11.dll")).unwrap(),
        b"d3d11.dll"
    );
    // Without the profile, the publisher's program is neither readied nor
    // started.
    let other = Installation::of(&locations, OTHER, "Windows", &install)
        .unwrap()
        .unwrap();
    assert!(matches!(
        other.ready(&locations, &launcher, &Plain::default()),
        Err(Error::NotExecutable)
    ));
    assert!(matches!(
        super::target::<Plain>(&config, &plain(&install), &launcher, HashMap::new()),
        Err(Error::NotExecutable)
    ));
    assert!(calls(&locations.wine_prefixes).is_empty());
    assert!(!other.prefix().exists());
}

#[test]
fn a_d3dmetal_game_is_translated_once_it_is_readied() {
    use std::os::unix::fs::PermissionsExt;
    let (_directory, root) = installation();
    let locations = locations(&root);
    engine_runtime(&locations);
    // Obelisk's Intel helper only records what it was asked.
    let helper = locations.helpers.join(crate::package::SDK_INTEL);
    fs::create_dir_all(&locations.helpers).unwrap();
    let record = root.join("translated");
    fs::write(
        &helper,
        format!("#!/bin/sh\necho \"$1\" >> \"{}\"\n", record.display()),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let (install, target) = game(&root);
    let windows = Installation::of(&locations, ELDEN_RING, "windows", &install)
        .unwrap()
        .unwrap();
    fs::create_dir_all(windows.prefix().join("drive_c/windows/system32")).unwrap();
    fs::write(windows.prefix().join("system.reg"), "registry").unwrap();

    windows
        .ready(&locations, &target, &Plain::default())
        .unwrap();

    assert_eq!(fs::read_to_string(&record).unwrap(), "warm-d3dmetal\n");
}

#[test]
fn d3dmetal_is_selected_only_when_the_runtime_carries_it() {
    let (_directory, root) = installation();
    let config = config(&root);
    assert!(!environment(&config).contains_key("CX_ACTIVE_GRAPHICS_BACKEND"));

    let library = root.join("runtime/lib/external/libd3dshared.dylib");
    fs::create_dir_all(library.parent().unwrap()).unwrap();
    fs::write(&library, b"").unwrap();

    let environment = environment(&config);
    assert_eq!(environment["CX_ACTIVE_GRAPHICS_BACKEND"], "d3dmetal");
    assert_eq!(
        environment["CX_APPLEGPTK_LIBD3DSHARED_PATH"],
        library.to_string_lossy()
    );
}

/// Lays in `config`'s runtime D3DMetal, its DLLs beside Wine's, and DXMT's.
fn backends_runtime(config: &Config) {
    for folder in [
        "lib/d3dmetal/x86_64-windows",
        "lib/dxmt/i386-windows",
        "lib/external/D3DMetal.framework",
    ] {
        fs::create_dir_all(config.runtime.join(folder)).unwrap();
    }
    for file in [
        "lib/external/libd3dshared.dylib",
        "lib/external/D3DMetal.framework/D3DMetal",
    ] {
        fs::write(config.runtime.join(file), b"").unwrap();
    }
}

#[test]
fn a_game_s_backend_comes_before_wine_s_own_dlls_with_dxmt_behind_d3dmetal() {
    let (_directory, root) = installation();
    let d3dmetal = config(&root);
    backends_runtime(&d3dmetal);
    let runtime = d3dmetal.runtime.to_string_lossy().into_owned();

    let environment = environment(&d3dmetal);
    assert_eq!(
        environment["WINEDLLPATH_PREPEND"],
        format!("{runtime}/lib/d3dmetal:{runtime}/lib/dxmt")
    );
    assert_eq!(environment["CX_ACTIVE_GRAPHICS_BACKEND"], "d3dmetal");
    assert_eq!(
        environment["D3DMETAL_FRAMEWORK_PATH"],
        format!("{runtime}/lib/external/D3DMetal.framework/D3DMetal")
    );

    let dxmt = Config {
        graphics: Graphics::Dxmt,
        ..d3dmetal
    };
    let environment = self::environment(&dxmt);
    assert_eq!(
        environment["WINEDLLPATH_PREPEND"],
        format!("{runtime}/lib/dxmt")
    );
    assert_eq!(environment["CX_ACTIVE_GRAPHICS_BACKEND"], "dxmt");
    assert_eq!(
        environment["CX_APPLEGPTK_LIBD3DSHARED_PATH"],
        format!("{runtime}/lib/external/libd3dshared.dylib")
    );
}

#[test]
fn a_runtime_with_d3dmetal_over_wine_s_dlls_searches_wine_s_folder_alone() {
    let (_directory, root) = installation();
    let config = config(&root);
    fs::create_dir_all(config.runtime.join("lib/external")).unwrap();
    fs::write(config.runtime.join("lib/external/libd3dshared.dylib"), b"").unwrap();

    let environment = environment(&config);

    assert!(!environment.contains_key("WINEDLLPATH_PREPEND"));
    assert_eq!(environment["CX_ACTIVE_GRAPHICS_BACKEND"], "d3dmetal");
}

#[test]
fn turning_ray_tracing_off_reaches_d3dmetal_alone() {
    let (_directory, root) = installation();
    let config = config(&root);
    let defaults = environment(&config);
    assert!(!defaults.keys().any(|name| name.starts_with("D3DM_")));

    let options = Config {
        options: GraphicsOptions { ray_tracing: false },
        ..config
    };
    let environment = self::environment(&options);
    assert_eq!(environment["D3DM_SUPPORT_DXR"], "0");

    let dxmt = Config {
        graphics: Graphics::Dxmt,
        ..options
    };
    assert!(
        !self::environment(&dxmt)
            .keys()
            .any(|name| name.starts_with("D3DM_"))
    );
}

/// A runtime whose wine records each call with the folders it searches
/// first, in `calls` beside the prefix.
fn backend_recording_runtime(root: &Path) -> Config {
    let config = recording_runtime(root);
    fs::write(
        config.runtime.join("bin/wine"),
        "#!/bin/sh\nprintf '%s %s\\n' \"$*\" \"$WINEDLLPATH_PREPEND\" >> \"$WINEPREFIX/../calls\"\n",
    )
    .unwrap();
    config
}

#[test]
fn a_prefix_gets_every_backend_s_placeholders_when_created_or_when_its_runtime_gains_one() {
    let (_directory, root) = installation();
    let config = Config {
        graphics: Graphics::Dxmt,
        ..backend_recording_runtime(&root)
    };
    let runtime = config.runtime.to_string_lossy().into_owned();
    prepare_plain(&config, &root);
    fs::write(config.prefix.join("system.reg"), "").unwrap();
    prepare_plain(&config, &root);
    assert_eq!(calls(&root), ["wineboot --init "]);

    backends_runtime(&config);
    prepare_plain(&config, &root);
    prepare_plain(&config, &root);

    assert_eq!(
        calls(&root),
        [
            "wineboot --init ".to_owned(),
            format!("wineboot --update {runtime}/lib/d3dmetal:{runtime}/lib/dxmt"),
        ]
    );
    assert_eq!(
        fs::read_to_string(config.prefix.join(BACKENDS)).unwrap(),
        "d3dmetal dxmt"
    );
}

#[test]
fn launch_settings_move_a_game_to_the_engine_and_backend_its_player_chose() {
    let (_directory, root) = installation();
    let library = root.join("library");
    let locations = locations(&root);
    engine_runtime(&locations);
    let install = library.join("Games/Steam/steam-42-windows");
    let choices = crate::launch_settings::Choices {
        engine: Some(Engine::Linux),
        graphics: Some(Graphics::D3DMetal),
        ..Default::default()
    };
    crate::launch_settings::save(&locations.library, &install, &choices).unwrap();

    let windows = Installation::of(&locations, OTHER, "windows", &install)
        .unwrap()
        .unwrap();

    assert_eq!(
        windows.prefix(),
        library.join("Prefixes/steam-42-windows-linux/prefix")
    );
    let config = windows.runtime(&locations).unwrap();
    assert_eq!(config.engine, Engine::Linux);
    assert_eq!(config.graphics, Graphics::D3DMetal);
}

#[test]
fn the_player_s_launch_settings_arguments_follow_the_storefront_s() {
    let (_directory, root) = installation();
    let locations = locations(&root);
    let install = root.join("library/Games/Steam/steam-42-windows");
    let choices = crate::launch_settings::Choices {
        arguments: Some(r#"-dx11 "+exec auto exec.cfg""#.into()),
        ..Default::default()
    };
    crate::launch_settings::save(&locations.library, &install, &choices).unwrap();
    fs::create_dir_all(locations.voile.join("current/bin")).unwrap();
    fs::write(locations.voile.join("current/bin/wine"), "").unwrap();
    let windows = Installation::of(&locations, OTHER, "windows", &install)
        .unwrap()
        .unwrap();
    let config = windows.runtime(&locations).unwrap();
    assert_eq!(config.arguments, ["-dx11", "+exec auto exec.cfg"]);

    let (install, game_target) = game(&root);
    let game_target = LaunchTarget {
        arguments: vec!["-AUTH_TYPE=exchangecode".into()],
        ..game_target
    };
    let config = Config {
        arguments: vec!["-dx11".into()],
        ..self::config(&root)
    };
    let started =
        super::target::<Plain>(&config, &plain(&install), &game_target, HashMap::new()).unwrap();
    assert_eq!(
        started.arguments[started.arguments.len() - 2..],
        ["-AUTH_TYPE=exchangecode", "-dx11"]
    );
}

#[test]
fn the_game_s_log_starts_with_how_obelisk_runs_it_and_only_the_player_s_arguments() {
    let (_directory, root) = installation();
    let config = Config {
        options: GraphicsOptions { ray_tracing: false },
        arguments: vec!["-dx11".into(), "+exec auto exec.cfg".into(), String::new()],
        ..config(&root)
    };
    assert_eq!(
        config.log_header(),
        "obelisk: engine=wine-macos graphics=d3dmetal ray_tracing=off voile=development \
         arguments=-dx11 \"+exec auto exec.cfg\" \"\"\n"
    );
    assert_eq!(
        command_line(
            config
                .log_header()
                .split_once("arguments=")
                .unwrap()
                .1
                .trim_end()
        ),
        config.arguments
    );

    // The Linux engine's render servers take no options; a release names
    // itself in its manifest, which the engine's runtime sits inside.
    fs::create_dir_all(root.join("v26.3.0-15/linux")).unwrap();
    fs::write(
        root.join("v26.3.0-15/runtime.json"),
        r#"{"version": "v26.3.0-15"}"#,
    )
    .unwrap();
    let linux = Config {
        runtime: root.join("v26.3.0-15/linux"),
        engine: Engine::Linux,
        graphics: Graphics::Dxmt,
        ..self::config(&root)
    };
    assert_eq!(
        linux.log_header(),
        "obelisk: engine=wine-linux graphics=dxmt voile=v26.3.0-15\n"
    );
}

#[test]
fn a_missing_runtime_is_reported_before_any_prefix_is_created() {
    let (_directory, root) = installation();
    let locations = locations(&root);
    let (install, target) = game(&root);
    let windows = Installation::of(&locations, OTHER, "windows", &install)
        .unwrap()
        .unwrap();

    assert!(matches!(
        windows.ready(&locations, &target, &Plain::default()),
        Err(Error::WineUnavailable)
    ));
    assert!(!windows.prefix().exists());
}

#[test]
fn wine_reports_its_errors_unless_the_runtime_names_other_channels() {
    let (_directory, root) = installation();
    let config = config(&root);
    assert_eq!(environment(&config)["WINEDEBUG"], "err+all,fixme-all");

    fs::create_dir_all(&config.runtime).unwrap();
    fs::write(config.runtime.join("winedebug"), "+steamclient,err+all\n").unwrap();

    assert_eq!(environment(&config)["WINEDEBUG"], "+steamclient,err+all");
}

#[test]
fn dotnet_games_run_only_on_the_mono_a_runtime_carries() {
    let (_directory, root) = installation();
    let config = config(&root);
    assert_eq!(
        environment(&config)["WINEDLLOVERRIDES"],
        "mscoree,mshtml=;winemenubuilder.exe=d"
    );

    fs::create_dir_all(config.runtime.join("share/wine/mono/wine-mono-10.4.1")).unwrap();

    assert_eq!(
        environment(&config)["WINEDLLOVERRIDES"],
        "mshtml=;winemenubuilder.exe=d"
    );
}

#[test]
fn every_wine_command_syncs_through_mach_and_gives_32_bit_games_4_gb() {
    let (_directory, root) = installation();
    let environment = environment(&config(&root));

    assert_eq!(environment["WINEMSYNC"], "1");
    assert_eq!(environment["WINE_LARGE_ADDRESS_AWARE"], "1");
}

#[test]
fn a_profile_s_variables_and_dll_overrides_come_after_obelisk_s_own_and_open_the_log() {
    let (_directory, root) = installation();
    let config = Config {
        variables: BTreeMap::from([
            ("ROSETTA_ADVERTISE_AVX".to_owned(), String::new()),
            ("WINENCPU".to_owned(), "10".to_owned()),
        ]),
        dll_overrides: BTreeMap::from([
            ("xaudio2_7".to_owned(), "n,b".to_owned()),
            ("SpeechSynthWrapper".to_owned(), String::new()),
        ]),
        ..config(&root)
    };

    let environment = environment(&config);

    assert_eq!(environment["WINENCPU"], "10");
    assert_eq!(environment["ROSETTA_ADVERTISE_AVX"], "");
    assert_eq!(
        environment["WINEDLLOVERRIDES"],
        "mscoree,mshtml=;winemenubuilder.exe=d;SpeechSynthWrapper=;xaudio2_7=n,b"
    );
    assert_eq!(
        config.log_header(),
        "obelisk: engine=wine-macos graphics=d3dmetal ray_tracing=on voile=development \
         variables=ROSETTA_ADVERTISE_AVX= WINENCPU=10 \
         dll_overrides=SpeechSynthWrapper=;xaudio2_7=n,b\n"
    );
}

#[test]
fn every_wine_command_sees_the_avx_rosetta_translates() {
    let (_directory, root) = installation();

    assert_eq!(environment(&config(&root))["ROSETTA_ADVERTISE_AVX"], "1");
}

#[test]
fn dotnet_programs_map_their_jit_code_without_w_xor_x() {
    let (_directory, root) = installation();

    assert_eq!(
        environment(&config(&root))["DOTNET_EnableWriteXorExecute"],
        "0"
    );
}

#[test]
fn gstreamer_loads_only_the_plugins_the_runtime_carries() {
    let (_directory, root) = installation();
    let config = config(&root);
    assert!(!environment(&config).contains_key("GST_PLUGIN_SYSTEM_PATH"));
    assert!(!environment(&config).contains_key("GST_REGISTRY"));

    let plugins = config.runtime.join("lib/gstreamer-1.0");
    fs::create_dir_all(&plugins).unwrap();

    let environment = environment(&config);
    assert_eq!(
        environment["GST_PLUGIN_SYSTEM_PATH"],
        plugins.to_string_lossy()
    );
    assert_eq!(
        environment["GST_REGISTRY"],
        config
            .prefix
            .join("obelisk-gstreamer-registry.bin")
            .to_string_lossy()
    );
}

#[test]
fn a_prefix_profile_stops_linking_into_the_mac_home() {
    let (_directory, root) = installation();
    let home = root.join("home");
    fs::create_dir_all(home.join("Documents")).unwrap();
    fs::write(home.join("Documents/keep.txt"), b"mine").unwrap();
    let profile = profile(&root.join("prefix"));
    fs::create_dir_all(profile.join("Desktop")).unwrap();
    fs::create_dir_all(profile.join("AppData/LocalLow")).unwrap();
    std::os::unix::fs::symlink(home.join("Documents"), profile.join("Documents")).unwrap();
    std::os::unix::fs::symlink(&home, profile.join("Desktop/My Mac Desktop")).unwrap();
    let config = recording_runtime(&root);
    fs::write(config.prefix.join("system.reg"), "").unwrap();
    let (install, target) = game(&root);

    prepare(&config, &home, &install, &target, &Plain::default()).unwrap();

    assert!(
        fs::symlink_metadata(profile.join("Documents"))
            .unwrap()
            .is_dir()
    );
    assert_eq!(fs::read_dir(profile.join("Documents")).unwrap().count(), 0);
    assert!(!profile.join("Desktop/My Mac Desktop").exists());
    assert!(profile.join("AppData/LocalLow").is_dir());
    assert_eq!(fs::read(home.join("Documents/keep.txt")).unwrap(), b"mine");
}

#[test]
fn a_prefix_registers_the_runtime_mono_once_even_when_an_older_runtime_created_it() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    // A runtime without Mono only creates the prefix.
    prepare_plain(&config, &root);
    assert_eq!(calls(&root), ["wineboot --init"]);
    assert!(!config.prefix.join(MONO).exists());

    fs::write(config.prefix.join("system.reg"), "").unwrap();
    fs::create_dir_all(config.runtime.join("share/wine/mono/wine-mono-10.4.1")).unwrap();
    prepare_plain(&config, &root);
    prepare_plain(&config, &root);

    assert_eq!(calls(&root), ["wineboot --init", "wineboot --update"]);
    assert_eq!(
        fs::read_to_string(config.prefix.join(MONO)).unwrap(),
        "wine-mono-10.4.1"
    );
}

#[test]
fn a_new_prefix_gets_mono_from_its_creation_alone() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    fs::create_dir_all(config.runtime.join("share/wine/mono/wine-mono-10.4.1")).unwrap();

    prepare_plain(&config, &root);

    assert_eq!(calls(&root), ["wineboot --init"]);
    assert_eq!(
        fs::read_to_string(config.prefix.join(MONO)).unwrap(),
        "wine-mono-10.4.1"
    );
}

fn prepare_plain(config: &Config, root: &Path) {
    let (install, target) = game(root);
    prepare(config, root, &install, &target, &Plain::default()).unwrap();
}

/// A runtime whose wine records each call in `calls` beside the prefix, and
/// fails for any program named `fail`.
pub(crate) fn recording_runtime(root: &Path) -> Config {
    let config = config(root);
    fs::create_dir_all(&config.prefix).unwrap();
    record_calls(&config.runtime);
    config
}

/// Installs in `runtime` a wine that records each call in `calls` beside the
/// prefix it runs, and fails for any program named `fail`.
pub(crate) fn record_calls(runtime: &Path) {
    let bin = runtime.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (name, body) in [
        (
            "wine",
            "printf '%s\\n' \"$*\" >> \"$WINEPREFIX/../calls\"\ncase \"$*\" in *fail*) exit 1;; esac\n",
        ),
        ("wineserver", ""),
    ] {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

pub(crate) fn calls(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn each_prerequisite_runs_once_and_a_failed_one_again_on_the_next_launch() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let redist = root.join("game/redist");
    fs::create_dir_all(&redist).unwrap();
    for name in ["vc_redist.x64.exe", "Visual C++ x64.cmd", "fail.exe"] {
        fs::write(redist.join(name), "").unwrap();
    }
    let prerequisites = [
        Prerequisite {
            id: "vcredist".into(),
            programs: vec![
                (redist.join("Visual C++ x64.cmd"), String::new()),
                (
                    redist.join("vc_redist.x64.exe"),
                    r#"/q "/log=C:\a b.txt""#.into(),
                ),
            ],
        },
        Prerequisite {
            id: "broken".into(),
            programs: vec![(redist.join("fail.exe"), String::new())],
        },
    ];

    install_prerequisites(&config, &root, &prerequisites).unwrap();
    install_prerequisites(&config, &root, &prerequisites).unwrap();

    let script = windows_path(&redist.join("Visual C++ x64.cmd"));
    let program = redist.join("vc_redist.x64.exe");
    let failing = redist.join("fail.exe");
    assert_eq!(
        calls(&root),
        [
            format!("cmd /c {script}"),
            format!("{} /q /log=C:\\a b.txt", program.display()),
            failing.display().to_string(),
            failing.display().to_string(),
        ]
    );
    assert_eq!(
        fs::read_to_string(config.prefix.join(PREREQUISITES)).unwrap(),
        "vcredist"
    );
}

#[test]
fn a_missing_program_leaves_its_prerequisite_pending() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let prerequisite = Prerequisite {
        id: "missing".into(),
        programs: vec![(root.join("game/absent.exe"), String::new())],
    };
    install_prerequisites(&config, &root, &[prerequisite]).unwrap();
    assert!(calls(&root).is_empty());
    assert!(!config.prefix.join(PREREQUISITES).exists());
}

#[test]
fn a_windows_installation_runs_in_the_prefix_named_after_its_folder_whatever_the_storefront() {
    let locations = crate::Locations::new(
        Path::new("/library"),
        Path::new("/Obelisk.app"),
        Path::new("/home"),
    );
    for (platform, install) in [
        ("windows", "/library/Games/Steam/steam-42-windows"),
        ("Windows", "/library/Games/Epic/epic-hash-Windows"),
    ] {
        let windows = Installation::of(&locations, OTHER, platform, Path::new(install))
            .unwrap()
            .expect("a Windows build");
        let folder = Path::new(install).file_name().unwrap();
        assert_eq!(
            windows.prefix(),
            Path::new("/library/Prefixes").join(folder)
        );
    }
    for platform in ["macos", "Mac"] {
        let native = Installation::of(
            &locations,
            OTHER,
            platform,
            Path::new("/library/Games/native"),
        );
        assert_eq!(native.unwrap(), None, "a Mac build runs outside Wine");
    }
    assert!(
        Installation::of(&locations, OTHER, "windows", Path::new("/")).is_err(),
        "no folder names no prefix"
    );
}

#[test]
fn a_saved_configuration_from_before_engines_and_graphics_runs_its_engine_s_default_backend() {
    let saved = r#"{"runtime":"/Voile/v1","prefix":"/Prefixes/game","log":"/logs/game.log"}"#;
    let config: Config = serde_json::from_str(saved).unwrap();
    assert_eq!(config.engine, Engine::Rosetta);
    assert_eq!(config.graphics, Graphics::D3DMetal);
    assert_eq!(config.options, GraphicsOptions::default());

    let saved = r#"{"runtime":"/Voile/v1/linux","prefix":"/Prefixes/game-linux/prefix","log":"/logs/game.log","engine":"Linux"}"#;
    let config: Config = serde_json::from_str(saved).unwrap();
    assert_eq!(config.engine, Engine::Linux);
    assert_eq!(config.graphics, Graphics::Dxmt);
    let config = Config {
        graphics: Graphics::D3DMetal,
        ..config
    };
    let again: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(again, config);
}

#[test]
fn a_game_whose_profile_names_the_linux_engine_runs_there_in_its_own_prefix_with_its_graphics() {
    let (_directory, root) = installation();
    let library = root.join("library");
    let locations = locations(&root);
    let runtime = engine_runtime(&locations);
    let install = library.join("Games/Steam/steam-1245620-windows");

    let windows = Installation::of(&locations, ELDEN_RING, "windows", &install)
        .unwrap()
        .unwrap();

    assert_eq!(
        windows.prefix(),
        library.join("Prefixes/steam-1245620-windows-linux/prefix")
    );
    let config = windows.runtime(&locations).unwrap();
    assert_eq!(config.engine, Engine::Linux);
    assert_eq!(config.graphics, Graphics::D3DMetal);
    assert_eq!(config.runtime, runtime);
    let other = Installation::of(&locations, OTHER, "windows", &install)
        .unwrap()
        .unwrap();
    assert_eq!(
        other.prefix(),
        library.join("Prefixes/steam-1245620-windows")
    );
}

#[test]
fn an_uninstalled_windows_game_loses_its_prefix_but_never_what_a_link_points_to() {
    use obelisk_core::installation::RemovalFailure;
    let (_directory, root) = installation();
    let library = root.join("library");
    let locations = crate::Locations::new(&library, Path::new("/Obelisk.app"), Path::new("/home"));
    let windows = |name: &str| {
        Installation::of(
            &locations,
            OTHER,
            "windows",
            &library.join("Games/Steam").join(name),
        )
        .unwrap()
        .unwrap()
    };
    let prefix = library.join("Prefixes/steam-42-windows");
    fs::create_dir_all(prefix.join("drive_c/users")).unwrap();
    fs::write(prefix.join("system.reg"), "registry").unwrap();
    windows("steam-42-windows").remove(&locations).unwrap();
    assert!(!prefix.exists());
    windows("steam-42-windows").remove(&locations).unwrap();

    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, library.join("Prefixes/steam-43-windows")).unwrap();
    assert!(matches!(
        windows("steam-43-windows").remove(&locations),
        Err(RemovalFailure::Unsafe(_))
    ));
    assert!(elsewhere.is_dir());
}

#[test]
fn uninstalling_removes_both_engines_prefixes_and_the_launch_settings() {
    let (_directory, root) = installation();
    let locations = locations(&root);
    let install = root.join("library/Games/Steam/steam-1245620-windows");
    let rosetta = Installation::of(&locations, OTHER, "windows", &install)
        .unwrap()
        .unwrap();
    fs::create_dir_all(rosetta.prefix().join("drive_c")).unwrap();
    let linux = Installation::of(&locations, ELDEN_RING, "windows", &install)
        .unwrap()
        .unwrap();
    fs::create_dir_all(linux.prefix().join("drive_c")).unwrap();
    let choices = crate::launch_settings::Choices {
        engine: Some(Engine::Rosetta),
        ..Default::default()
    };
    crate::launch_settings::save(&locations.library, &install, &choices).unwrap();
    let settings = locations
        .library
        .join("LaunchSettings/steam-1245620-windows.json");
    assert!(settings.is_file());

    // The player moved the game back to Rosetta: the engine's folder still goes.
    Installation::of(&locations, ELDEN_RING, "windows", &install)
        .unwrap()
        .unwrap()
        .remove(&locations)
        .unwrap();

    assert!(!rosetta.prefix().exists());
    assert!(!linux.prefix().parent().unwrap().exists());
    assert!(!settings.exists());
}

#[test]
fn only_processes_running_the_runtime_are_left_behind_wine_processes() {
    let listing = "p10\nftxt\nn/usr/lib/dyld\np20\nftxt\nn/usr/lib/dyld\nftxt\nn/lib/Voile/current/bin/wine-preloader\nftxt\nn/lib/Voile/current/lib/wine/ntdll.so\np30\nftxt\nn/lib/Voile-other/bin/wine\np40\nftxt\nn/lib/Voile/v1/bin/wineserver\n";
    assert_eq!(running_from(listing, Path::new("/lib/Voile"), 1), [20, 40]);
    assert_eq!(
        running_from(listing, Path::new("/lib/Voile"), 20),
        [40],
        "never Obelisk itself"
    );
}

#[test]
fn quitting_a_windows_game_asks_its_windows_to_close_and_kills_nothing() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let environment = environment(&config);
    let program = root.join("game/Game.exe");
    let command = quit_command(&config, &environment, Some(&program), 7).unwrap();
    assert_eq!(command.get_program(), config.runtime.join("bin/wine"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["taskkill", "/T", "/IM", "Game.exe"],
        "without /F, taskkill only posts WM_CLOSE"
    );
    // The game's own environment, which its Wine server agrees with.
    let envs: HashMap<_, _> = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.unwrap().to_owned()))
        .collect();
    assert_eq!(envs.len(), environment.len());
    assert!(
        environment
            .iter()
            .all(|(name, value)| envs[std::ffi::OsStr::new(name)] == **value)
    );

    quit(&config, &environment, Some(&program), 7);
    assert_eq!(calls(&root), ["taskkill /T /IM Game.exe"]);
    assert!(quit_command(&config, &environment, None, 7).is_none());
}

#[test]
fn quitting_a_linux_engine_game_uses_its_window_server_quit() {
    let (_directory, root) = installation();
    let config = Config {
        engine: Engine::Linux,
        ..config(&root)
    };
    let environment = HashMap::from([("TMPDIR".to_owned(), "/var/folders/T/".to_owned())]);
    let command = quit_command(&config, &environment, None, 77).unwrap();
    assert_eq!(command.get_program(), config.runtime.join("engine/wsdctl"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [
            "/var/folders/T/obelisk-engine-77/wsd.sock",
            "window",
            "quit"
        ]
    );
    let command = quit_command(&config, &HashMap::new(), None, 77).unwrap();
    assert_eq!(
        command.get_args().next().unwrap(),
        "/tmp/obelisk-engine-77/wsd.sock"
    );
}

#[test]
fn only_a_prefix_and_its_wine_server_belong_to_one_windows_game() {
    use std::os::unix::fs::MetadataExt;
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let prefix = fs::metadata(&config.prefix).unwrap();
    assert_eq!(
        private_folders(&config),
        [
            config.prefix.clone(),
            PathBuf::from(format!(
                "/tmp/.wine-{}/server-{:x}-{:x}",
                prefix.uid(),
                prefix.dev(),
                prefix.ino()
            )),
        ]
    );
    let linux = Config {
        prefix: root.join("Game-linux/prefix"),
        engine: Engine::Linux,
        ..config
    };
    assert_eq!(private_folders(&linux), [root.join("Game-linux")]);
}
