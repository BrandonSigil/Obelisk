use super::*;

const ROCKET_LEAGUE: Game<'static> = Game::Epic {
    namespace: "9773aa1aa54f4f7b80e44bef04986cea",
    app_name: "Sugar",
};

#[test]
fn rocket_league_and_elden_ring_run_on_the_linux_engine_and_nothing_else_does() {
    assert_eq!(of(ROCKET_LEAGUE).engine, Engine::Linux);
    assert_eq!(of(Game::Steam { app_id: 1245620 }).engine, Engine::Linux);
    for other in [
        Game::Epic {
            namespace: "9773aa1aa54f4f7b80e44bef04986cea",
            app_name: "Other",
        },
        Game::Epic {
            namespace: "5a6d34ba28bc45d081d9311c3885feef",
            app_name: "Sugar",
        },
        Game::Steam { app_id: 252950 },
        Game::Gog {
            product_id: "1245620",
        },
    ] {
        assert_eq!(of(other), Profile::default(), "{other:?}");
        assert_eq!(of(other).engine, Engine::Rosetta);
    }
}

#[test]
fn elden_ring_renders_through_d3dmetal_and_rocket_league_through_dxmt() {
    assert_eq!(
        of(Game::Steam { app_id: 1245620 }).graphics,
        Graphics::D3DMetal
    );
    assert_eq!(of(ROCKET_LEAGUE).graphics, Graphics::Dxmt);
    assert_eq!(Profile::default().graphics, Graphics::Dxmt);
}

#[test]
fn rocket_league_starts_its_anti_cheat_bootstrapper_instead_of_its_dotnet_launcher() {
    let install = Path::new("/games/rl");
    let publisher = LaunchTarget {
        executable: "/games/rl/Binaries/Win64/Launcher.exe".into(),
        arguments: vec!["-AUTH_TYPE=exchangecode".into()],
        working_directory: "/games/rl/Binaries/Win64".into(),
        is_application_bundle: false,
    };

    let started = of(ROCKET_LEAGUE).target(install, &publisher);

    assert_eq!(
        started.executable,
        "/games/rl/Binaries/Win64/RocketLeague_EAC.exe"
    );
    assert_eq!(started.working_directory, "/games/rl/Binaries/Win64");
    assert_eq!(started.arguments, publisher.arguments);
}

#[test]
fn a_game_without_a_substitute_starts_its_publisher_s_program() {
    let publisher = LaunchTarget {
        executable: "/games/er/Game/start_protected_game.exe".into(),
        arguments: Vec::new(),
        working_directory: "/games/er/Game".into(),
        is_application_bundle: false,
    };
    let install = Path::new("/games/er");
    assert_eq!(
        of(Game::Steam { app_id: 1245620 }).target(install, &publisher),
        publisher
    );
    assert_eq!(Profile::default().target(install, &publisher), publisher);
}
