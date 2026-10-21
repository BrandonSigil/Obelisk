use super::*;
use crate::wine::{
    prepare,
    tests::{Plain, calls, game, installation, locations, recording_runtime},
    windows_path,
};
use std::sync::Mutex;

/// Readies the created prefix of `config` for a game whose storefront names
/// `prerequisites`, and returns the steps it showed.
fn ready(config: &Config, root: &Path, prerequisites: &[Prerequisite]) -> Vec<LaunchStep> {
    fs::write(config.prefix.join("system.reg"), "").unwrap();
    let storefront = Plain {
        prerequisites: prerequisites.to_vec(),
    };
    steps(config, root, &storefront)
}

/// Readies the prefix of `config` for `storefront`'s game, and returns the
/// steps it showed.
fn steps(config: &Config, root: &Path, storefront: &Plain) -> Vec<LaunchStep> {
    let (install, target) = game(root);
    let steps = Mutex::new(Vec::new());
    prepare(
        config,
        &locations(root),
        &install,
        &target,
        storefront,
        &|step| steps.lock().unwrap().push(step),
    )
    .unwrap();
    steps.into_inner().unwrap()
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
            name: None,
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
            name: None,
            programs: vec![(redist.join("fail.exe"), String::new())],
        },
    ];

    ready(&config, &root, &prerequisites);
    ready(&config, &root, &prerequisites);

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
fn a_visual_cpp_installer_runs_with_the_versions_of_wine_s_own_dlls_hidden() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    fs::write(
        config.runtime.join("bin/wine"),
        "#!/bin/sh\nprintf '%s %s\\n' \"${1##*/}\" \"${CX_HIDE_BUILTIN_VERSION_RESOURCES:-shown}\" \
         >> \"$WINEPREFIX/../calls\"\n",
    )
    .unwrap();
    let redist = root.join("game/redist");
    fs::create_dir_all(&redist).unwrap();
    let names = ["VC_redist.x64.exe", "vcredist_x86.exe", "DXSETUP.exe"];
    for name in names {
        fs::write(redist.join(name), "").unwrap();
    }
    let prerequisite = Prerequisite {
        id: "redist".into(),
        name: None,
        programs: names
            .iter()
            .map(|name| (redist.join(name), String::new()))
            .collect(),
    };

    ready(&config, &root, &[prerequisite]);

    assert_eq!(
        calls(&root),
        [
            "VC_redist.x64.exe 1",
            "vcredist_x86.exe 1",
            "DXSETUP.exe shown"
        ]
    );
}

#[test]
fn a_missing_program_leaves_its_prerequisite_pending() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let prerequisite = Prerequisite {
        id: "missing".into(),
        name: None,
        programs: vec![(root.join("game/absent.exe"), String::new())],
    };
    ready(&config, &root, &[prerequisite]);
    assert!(calls(&root).is_empty());
    assert!(!config.prefix.join(PREREQUISITES).exists());
}

#[test]
fn each_pending_prerequisite_shows_by_its_name_as_it_begins() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let redist = root.join("game/redist");
    fs::create_dir_all(&redist).unwrap();
    for name in ["DXSETUP.exe", "setup.exe", "fail.exe"] {
        fs::write(redist.join(name), "").unwrap();
    }
    let prerequisites = [
        Prerequisite {
            id: "directx".into(),
            name: Some("DirectX".into()),
            programs: vec![(redist.join("DXSETUP.exe"), "/silent".into())],
        },
        Prerequisite {
            id: "setup".into(),
            name: None,
            programs: vec![(redist.join("setup.exe"), String::new())],
        },
        Prerequisite {
            id: "broken".into(),
            name: Some("Broken".into()),
            programs: vec![(redist.join("fail.exe"), String::new())],
        },
    ];
    let prerequisite = |name: Option<&str>| LaunchStep::Prerequisite {
        name: name.map(str::to_owned),
    };

    assert_eq!(
        ready(&config, &root, &prerequisites),
        [
            prerequisite(Some("DirectX")),
            prerequisite(None),
            prerequisite(Some("Broken"))
        ]
    );
    // Only the one that failed runs, and shows, again.
    assert_eq!(
        ready(&config, &root, &prerequisites),
        [prerequisite(Some("Broken"))]
    );
}

#[test]
fn creating_or_updating_the_prefix_shows_as_a_step_and_a_ready_one_shows_none() {
    let (_directory, root) = installation();
    let config = recording_runtime(&root);
    let plain = Plain::default();

    assert_eq!(steps(&config, &root, &plain), [LaunchStep::WinePrefix]);
    fs::write(config.prefix.join("system.reg"), "").unwrap();
    assert_eq!(steps(&config, &root, &plain), []);
    // A runtime that gained Mono updates the prefix once.
    fs::create_dir_all(config.runtime.join("share/wine/mono/wine-mono-10.4.1")).unwrap();
    assert_eq!(steps(&config, &root, &plain), [LaunchStep::WinePrefix]);
    assert_eq!(steps(&config, &root, &plain), []);
    assert_eq!(calls(&root), ["wineboot --init", "wineboot --update"]);
}
