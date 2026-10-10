use super::*;
use crate::wine::tests::{installation, pe, target as launch_target};

fn linux_config(root: &Path) -> wine::Config {
    wine::Config {
        runtime: root.join("runtime"),
        prefix: root.join("Prefixes/game-linux/prefix"),
        log: root.join("logs/wine-game.log"),
        engine: wine::Engine::Linux,
        graphics: wine::Graphics::Dxmt,
    }
}

/// A storefront that adds nothing.
struct Plain;
impl wine::Storefront for Plain {
    fn prerequisites(&self) -> Result<Vec<wine::Prerequisite>, Error> {
        Ok(Vec::new())
    }
}

/// The values of every `option` pair in the launcher's arguments.
fn options<'a>(target: &'a Target, option: &str) -> Vec<&'a str> {
    let separator = target
        .arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap();
    target.arguments[7..separator]
        .chunks(2)
        .filter(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
        .collect()
}

/// What the launcher starts in the guest: the program and its arguments.
fn guest_command(target: &Target) -> &[String] {
    let separator = target
        .arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap();
    &target.arguments[separator + 1..]
}

#[test]
fn the_engine_keeps_its_prefix_in_its_own_folder_beside_rosetta_s() {
    let rosetta = Path::new("/library/Prefixes/epic-game-Windows");
    let prefix = prefix(rosetta).unwrap();
    assert_eq!(
        prefix,
        Path::new("/library/Prefixes/epic-game-Windows-linux/prefix")
    );
    assert_eq!(
        home(&prefix).unwrap(),
        Path::new("/library/Prefixes/epic-game-Windows-linux")
    );
}

#[test]
fn a_game_starts_through_the_native_launcher_with_guest_paths() {
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(install.join("Binaries/Win64")).unwrap();
    let executable = install.join("Binaries/Win64/Game.exe");
    fs::write(&executable, pe()).unwrap();
    let config = linux_config(&root);
    let mut environment = HashMap::new();
    environment.insert("HOME".to_owned(), "/Users/player".to_owned());

    let target = target(
        &install,
        &launch_target(&executable, &["-AUTH_TYPE=exchangecode"]),
        environment,
        &config,
    )
    .unwrap();

    assert_eq!(target.executable, Path::new("/bin/bash"));
    assert_eq!(target.arguments[0], "-c");
    assert_eq!(target.arguments[1], LAUNCHER);
    assert_eq!(
        target.arguments[3..7],
        [
            root.join("runtime").to_string_lossy().into_owned(),
            root.join("Prefixes/game-linux")
                .to_string_lossy()
                .into_owned(),
            install.to_string_lossy().into_owned(),
            "/opt/game/Binaries/Win64".to_owned(),
        ]
    );
    assert_eq!(
        guest_command(&target),
        [
            "/opt/game/Binaries/Win64/Game.exe",
            "-AUTH_TYPE=exchangecode"
        ]
    );
    assert!(options(&target, "--env").contains(&"WINEPREFIX=/home/user/prefix"));
    assert_eq!(options(&target, "--graphics"), ["dxmt"]);
    // Only a storefront's guest sees more of the runtime.
    assert!(options(&target, "--bind").is_empty());
    assert!(options(&target, "--pass").is_empty());
    assert_eq!(target.environment["HOME"], "/Users/player");
    // Without a title, the Dock names the game after its program.
    assert_eq!(target.environment[APP_NAME], "Game");
    assert_eq!(target.log, Some(config.log.clone()));
}

#[test]
fn a_game_takes_its_store_name_in_the_dock_without_trademark_signs() {
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("RocketLeague_EAC.exe");
    fs::write(&executable, pe()).unwrap();
    let environment = HashMap::from([(APP_NAME.to_owned(), "Rocket League® ".to_owned())]);

    let target = target(
        &install,
        &launch_target(&executable, &[]),
        environment,
        &linux_config(&root),
    )
    .unwrap();

    assert_eq!(target.environment[APP_NAME], "Rocket League");
}

#[test]
fn a_program_outside_the_installation_never_starts() {
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    let outside = root.join("Other.exe");
    fs::write(&outside, pe()).unwrap();
    let failure = target(
        &install,
        &launch_target(&outside, &[]),
        HashMap::new(),
        &linux_config(&root),
    );
    assert!(matches!(failure, Err(Error::NotExecutable)));
}

#[test]
fn a_private_file_lives_in_the_prefix_s_c_drive_as_long_as_its_folder() {
    let (_directory, root) = installation();
    let config = linux_config(&root);
    fs::create_dir_all(config.prefix.join("drive_c")).unwrap();

    let (folder, path) = private_file(&config, "ownership.bin", b"token").unwrap();

    let name = path
        .strip_prefix("C:\\")
        .and_then(|rest| rest.strip_suffix("\\ownership.bin"))
        .unwrap();
    let file = config
        .prefix
        .join("drive_c")
        .join(name)
        .join("ownership.bin");
    assert_eq!(fs::read(&file).unwrap(), b"token");
    drop(folder);
    assert!(!file.exists());
}

#[test]
fn uninstalling_removes_the_engine_s_folder_and_any_rosetta_prefix() {
    let (_directory, root) = installation();
    let locations = crate::Locations::new(&root.join("library"), Path::new("/Obelisk.app"), &root);
    let install = root.join("library/Games/Epic/epic-game-Windows");
    let game = crate::profile::Game::Steam { app_id: 42 };
    let rosetta = crate::wine::Installation::of(&locations, game, "Windows", &install)
        .unwrap()
        .unwrap();
    fs::create_dir_all(rosetta.prefix().join("drive_c")).unwrap();
    let linux = rosetta
        .clone()
        .on_linux_engine(wine::Graphics::Dxmt)
        .unwrap();
    fs::create_dir_all(linux.prefix().join("drive_c")).unwrap();

    linux.remove(&locations).unwrap();

    assert!(!rosetta.prefix().exists());
    assert!(!home(linux.prefix()).unwrap().exists());
}

#[test]
fn a_d3dmetal_game_asks_the_launcher_for_the_d3dmetal_render_server() {
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    fs::write(install.join("Game.exe"), pe()).unwrap();
    let config = wine::Config {
        graphics: wine::Graphics::D3DMetal,
        ..linux_config(&root)
    };

    let target = target(
        &install,
        &launch_target(&install.join("Game.exe"), &[]),
        HashMap::new(),
        &config,
    )
    .unwrap();

    assert_eq!(options(&target, "--graphics"), ["d3dmetal"]);
}

/// A Steam game whose program is `Game/start_protected_game.exe`, in a
/// runtime that carries steam-hostpid.so when `host_pid`.
fn steam_target(root: &Path, host_pid: bool) -> (wine::Config, Target) {
    let install = root.join("install");
    fs::create_dir_all(install.join("Game")).unwrap();
    let program = install.join("Game/start_protected_game.exe");
    fs::write(&program, pe()).unwrap();
    let config = linux_config(root);
    if host_pid {
        fs::create_dir_all(config.runtime.join("guest")).unwrap();
        fs::write(config.runtime.join("guest").join(HOST_PID), b"elf").unwrap();
    }
    let guest = Guest::steam(
        &config,
        r"C:\windows\system32\steam.exe",
        &["SteamAppId", "Steam3Master"],
    );
    let target = target_in(
        &install,
        &launch_target(&program, &["-eac"]),
        HashMap::new(),
        &config,
        &guest,
    )
    .unwrap();
    (config, target)
}

#[test]
fn a_steam_game_starts_behind_steam_exe_with_valve_s_linux_client_and_its_session_s_values() {
    let (_directory, root) = installation();
    let (config, target) = steam_target(&root, true);

    assert_eq!(
        guest_command(&target),
        [
            r"C:\windows\system32\steam.exe",
            "/opt/game/Game/start_protected_game.exe",
            "-eac"
        ]
    );
    assert_eq!(target.arguments[6], "/opt/game/Game");
    let runtime = config.runtime.display();
    assert_eq!(
        options(&target, "--bind"),
        [
            format!("/opt/steam={runtime}/steam:ro"),
            format!("/opt/obelisk={runtime}/guest:ro"),
        ]
    );
    assert!(options(&target, "--env").contains(&"LD_PRELOAD=/opt/obelisk/steam-hostpid.so"));
    // The host's ports exist only once the session starts the game.
    assert_eq!(options(&target, "--pass"), ["SteamAppId", "Steam3Master"]);
}

#[test]
fn a_runtime_without_the_host_pid_library_preloads_nothing() {
    let (_directory, root) = installation();
    let (config, target) = steam_target(&root, false);

    assert_eq!(
        options(&target, "--bind"),
        [format!("/opt/steam={}/steam:ro", config.runtime.display())]
    );
    assert!(
        !options(&target, "--env")
            .iter()
            .any(|variable| variable.starts_with("LD_PRELOAD="))
    );
}

#[test]
fn the_guest_home_leads_lsteamclient_to_the_runtime_s_linux_steamclient() {
    let (_directory, root) = installation();
    let config = linux_config(&root);
    let link = home(&config.prefix).unwrap().join(".steam/sdk64");

    link_steam_client(&config).unwrap();
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("/opt/steam/linux64")
    );
    link_steam_client(&config).unwrap();
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("/opt/steam/linux64")
    );

    // A link elsewhere is replaced; anything else is never removed.
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("/elsewhere", &link).unwrap();
    link_steam_client(&config).unwrap();
    assert_eq!(
        fs::read_link(&link).unwrap(),
        Path::new("/opt/steam/linux64")
    );
    fs::remove_file(&link).unwrap();
    fs::create_dir(&link).unwrap();
    assert!(matches!(link_steam_client(&config), Err(Error::Storage)));
    assert!(link.is_dir());
}

/// A runtime whose window server and render servers only listen, and whose
/// engine records what it was asked to run into `engine.txt`.
fn recording_runtime(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let runtime = root.join("runtime");
    let script = |relative: &str, body: &str| {
        let path = runtime.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    };
    script("engine/wsd", r#"exec /usr/bin/nc -lU "$2/wsd.sock""#);
    let server = r#"echo "$NPT_BACKEND $NPT_D3D12_LIBRARY_PATH" > "$OBELISK_TEST_ROOT/server.txt"
exec /usr/bin/nc -lU "$4""#;
    script("graphics/bin/virgl_test_server", server);
    script("graphics/x86_64/bin/virgl_test_server", server);
    script(
        "engine/fexdarwin-run",
        r#"{ printf '%s\n' "$FEXDARWIN_BIND" "files $(ulimit -S -n)"; printf '%s\n' "$@"; } > "$OBELISK_TEST_ROOT/engine.txt""#,
    );
    runtime
}

/// Runs the launcher with `options` and the program `Game.exe`, from an
/// environment with `variables` and the Finder's 256 open files.
fn run_launcher(root: &Path, options: &[&str], variables: &[(&str, &str)]) -> (String, String) {
    let runtime = recording_runtime(root);
    let mut command = Command::new("/bin/bash");
    command
        .arg("-c")
        .arg(r#"ulimit -S -n 256; exec /bin/bash -c "$0" "$@""#)
        .arg(LAUNCHER)
        .arg("obelisk-linux-engine")
        .arg(&runtime)
        .arg(root.join("home"))
        .arg(root.join("install"))
        .arg("/opt/game")
        .args(options)
        .args(["--", "/opt/game/Game.exe", "-flag"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TMPDIR", root)
        .env("OBELISK_TEST_ROOT", root)
        .envs(variables.iter().copied());
    let status = command.stdin(Stdio::null()).status().unwrap();
    assert!(status.success(), "{status:?}");
    (
        fs::read_to_string(root.join("engine.txt")).unwrap(),
        fs::read_to_string(root.join("server.txt")).unwrap(),
    )
}

#[test]
fn the_launcher_hands_the_guest_its_binds_its_variables_and_the_session_s_values() {
    let (_directory, root) = installation();
    let runtime = root.join("runtime");

    let (engine, server) = run_launcher(
        &root,
        &[
            "--graphics",
            "dxmt",
            "--bind",
            "/opt/steam=/runtime/steam:ro",
            "--env",
            "WINEPREFIX=/home/user/prefix",
            "--pass",
            "Steam3Master",
            "--pass",
            "SteamClientService",
        ],
        &[("Steam3Master", "127.0.0.1:5000")],
    );

    let lines: Vec<&str> = engine.lines().collect();
    assert!(
        lines[0].starts_with(&format!(
            "/opt/wine={}/wine:ro;/home/user={};/opt/game={};/run/obelisk=",
            runtime.display(),
            root.join("home").display(),
            root.join("install").display()
        )),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].ends_with(";/opt/steam=/runtime/steam:ro"),
        "{}",
        lines[0]
    );
    // The Finder's 256 files are not enough for Wine.
    let files: u64 = lines[1].strip_prefix("files ").unwrap().parse().unwrap();
    assert!(files >= 10240, "{files}");
    let variables: Vec<&str> = lines
        .windows(2)
        .filter(|pair| pair[0] == "--env")
        .map(|pair| pair[1])
        .collect();
    assert_eq!(
        variables,
        [
            "WINEPREFIX=/home/user/prefix",
            "Steam3Master=127.0.0.1:5000"
        ]
    );
    assert!(lines.ends_with(&["sh", "/opt/game/Game.exe", "-flag"]));
    let dxmt = runtime.join("graphics/dxmt-native.framework/Versions/A/dxmt-native");
    assert_eq!(server.trim(), format!("dxmt {}", dxmt.display()));
}

#[test]
fn the_launcher_runs_d3dmetal_in_the_intel_render_server() {
    let (_directory, root) = installation();

    let (_, server) = run_launcher(&root, &["--graphics", "d3dmetal"], &[]);

    let library = root.join("runtime/graphics/x86_64/lib/libd3dmetal-native.dylib");
    assert_eq!(server.trim(), format!("d3dmetal {}", library.display()));
}

#[test]
fn the_launcher_refuses_an_option_it_does_not_know_or_a_variable_name_that_is_not_one() {
    let (_directory, root) = installation();
    let runtime = recording_runtime(&root);
    for options in [&["--other", "x"][..], &["--pass", "A;B"], &["--env"]] {
        let status = Command::new("/bin/bash")
            .arg("-c")
            .arg(LAUNCHER)
            .arg("obelisk-linux-engine")
            .arg(&runtime)
            .arg(root.join("home"))
            .arg(root.join("install"))
            .arg("/opt/game")
            .args(options)
            .args(["--", "/opt/game/Game.exe"])
            .env_clear()
            .env("TMPDIR", &root)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(2), "{options:?}");
    }
    assert!(!root.join("engine.txt").exists());
}

/// End to end on an installed engine runtime, through `prepare` and the
/// launcher as a launch runs them: `OBELISK_LINUX_ENGINE_RUNTIME` names the
/// runtime, `OBELISK_LINUX_ENGINE_PROGRAM` a Windows program that exits by
/// itself, `OBELISK_LINUX_ENGINE_ARGUMENTS` its arguments, and
/// `OBELISK_LINUX_ENGINE_GRAPHICS=d3dmetal` its renderer. It opens a window.
#[test]
#[ignore]
fn smoke() {
    let runtime = PathBuf::from(std::env::var("OBELISK_LINUX_ENGINE_RUNTIME").unwrap());
    let program = PathBuf::from(std::env::var("OBELISK_LINUX_ENGINE_PROGRAM").unwrap());
    let arguments = std::env::var("OBELISK_LINUX_ENGINE_ARGUMENTS").unwrap_or_default();
    let (_directory, root) = installation();
    let install = root.join("install");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join(program.file_name().unwrap());
    fs::copy(&program, &executable).unwrap();
    let config = wine::Config {
        runtime,
        prefix: root.join("Prefixes/game-linux/prefix"),
        log: root.join("logs/game.log"),
        engine: wine::Engine::Linux,
        graphics: match std::env::var("OBELISK_LINUX_ENGINE_GRAPHICS").as_deref() {
            Ok("d3dmetal") => wine::Graphics::D3DMetal,
            _ => wine::Graphics::Dxmt,
        },
    };
    let arguments: Vec<&str> = arguments.split_whitespace().collect();
    let launch = launch_target(&executable, &arguments);

    prepare(&config, &install, &launch, &Plain).unwrap();
    let target = target(&install, &launch, HashMap::new(), &config).unwrap();
    let output = Command::new(&target.executable)
        .args(&target.arguments)
        .current_dir(&target.working_directory)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let errors = String::from_utf8_lossy(&output.stderr);
    println!(
        "{}",
        errors
            .lines()
            .rev()
            .take(15)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(output.status.success(), "{:?}", output.status);
}
