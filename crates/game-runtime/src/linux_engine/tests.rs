use super::*;
use crate::wine::tests::{installation, pe, target as launch_target};

fn linux_config(root: &Path) -> wine::Config {
    wine::Config {
        runtime: root.join("runtime"),
        prefix: root.join("Prefixes/game-linux/prefix"),
        log: root.join("logs/wine-game.log"),
        engine: wine::Engine::Linux,
    }
}

#[test]
fn only_rocket_league_runs_through_the_engine() {
    assert!(runs_epic("9773aa1aa54f4f7b80e44bef04986cea", "Sugar"));
    assert!(!runs_epic("9773aa1aa54f4f7b80e44bef04986cea", "Other"));
    assert!(!runs_epic("5a6d34ba28bc45d081d9311c3885feef", "Sugar"));
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
            "/games/game/Binaries/Win64".to_owned(),
        ]
    );
    let separator = target
        .arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap();
    assert_eq!(
        target.arguments[separator + 1..],
        [
            "/games/game/Binaries/Win64/Game.exe",
            "-AUTH_TYPE=exchangecode"
        ]
    );
    assert!(
        target.arguments[7..separator]
            .windows(2)
            .any(|pair| pair == ["--env", "WINEPREFIX=/home/user/prefix"])
    );
    assert_eq!(target.environment["HOME"], "/Users/player");
    assert_eq!(target.log, Some(config.log.clone()));
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
    let locations = crate::Locations::new(&root.join("library"), Path::new("/SailOver.app"), &root);
    let install = root.join("library/Games/Epic/epic-game-Windows");
    let rosetta = crate::wine::Installation::of(&locations, "Windows", &install)
        .unwrap()
        .unwrap();
    fs::create_dir_all(rosetta.prefix().join("drive_c")).unwrap();
    let linux = rosetta.clone().on_linux_engine().unwrap();
    fs::create_dir_all(linux.prefix().join("drive_c")).unwrap();

    linux.remove(&locations).unwrap();

    assert!(!rosetta.prefix().exists());
    assert!(!home(linux.prefix()).unwrap().exists());
}

/// End to end on an installed engine runtime, through `prepare` and the
/// launcher as a launch runs them: `SAILOVER_LINUX_ENGINE_RUNTIME` names the
/// runtime, `SAILOVER_LINUX_ENGINE_PROGRAM` a Windows program that exits by
/// itself, `SAILOVER_LINUX_ENGINE_ARGUMENTS` its arguments. It opens a window.
#[test]
#[ignore]
fn smoke() {
    let runtime = PathBuf::from(std::env::var("SAILOVER_LINUX_ENGINE_RUNTIME").unwrap());
    let program = PathBuf::from(std::env::var("SAILOVER_LINUX_ENGINE_PROGRAM").unwrap());
    let arguments = std::env::var("SAILOVER_LINUX_ENGINE_ARGUMENTS").unwrap_or_default();
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
    };
    let arguments: Vec<&str> = arguments.split_whitespace().collect();
    let launch = launch_target(&executable, &arguments);

    prepare(&config, &install, &launch).unwrap();
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
