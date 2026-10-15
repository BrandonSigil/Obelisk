use super::*;
use crate::profile::{self, Game};

/// A game no profile names.
const OTHER: Game<'static> = Game::Steam { app_id: 42 };
/// A game whose profile names the Linux engine and D3DMetal.
const ELDEN_RING: Game<'static> = Game::Steam { app_id: 1245620 };

#[test]
fn a_game_without_choices_runs_as_its_profile_says() {
    let settings = Choices::default().apply(&profile::of(OTHER));
    assert_eq!(settings.engine, Engine::Rosetta);
    assert_eq!(settings.graphics, Graphics::D3DMetal);
    assert!(settings.options.ray_tracing);

    let settings = Choices::default().apply(&profile::of(ELDEN_RING));
    assert_eq!(settings.engine, Engine::Linux);
    assert_eq!(settings.graphics, Graphics::D3DMetal);
}

#[test]
fn another_engine_renders_with_its_own_default_backend_unless_the_player_picked_one() {
    let other = profile::of(OTHER);
    let linux = Choices {
        engine: Some(Engine::Linux),
        ..Choices::default()
    };
    assert_eq!(linux.apply(&other).graphics, Graphics::Dxmt);

    let picked = Choices {
        graphics: Some(Graphics::D3DMetal),
        ..linux
    };
    assert_eq!(picked.apply(&other).graphics, Graphics::D3DMetal);

    let rosetta = Choices {
        engine: Some(Engine::Rosetta),
        ..Choices::default()
    };
    assert_eq!(
        rosetta.apply(&profile::of(ELDEN_RING)).graphics,
        Graphics::D3DMetal
    );
}

#[test]
fn only_what_differs_from_the_profile_is_kept() {
    let other = profile::of(OTHER);
    let same = Choices {
        arguments: Some("   ".into()),
        engine: Some(Engine::Rosetta),
        graphics: Some(Graphics::D3DMetal),
        ray_tracing: Some(true),
    };
    assert!(same.normalized(&other).is_empty());

    let dxmt = Choices {
        arguments: Some("-dx11 \"+exec autoexec.cfg\"".into()),
        graphics: Some(Graphics::Dxmt),
        ray_tracing: Some(false),
        ..Choices::default()
    };
    assert_eq!(dxmt.normalized(&other), dxmt);
    let padded = Choices {
        arguments: Some("  -dx11  ".into()),
        ..Choices::default()
    };
    assert_eq!(
        padded.normalized(&other).arguments.as_deref(),
        Some("-dx11")
    );

    // On the Linux engine, DXMT is the default: only the engine is kept.
    let linux = Choices {
        engine: Some(Engine::Linux),
        graphics: Some(Graphics::Dxmt),
        ..Choices::default()
    };
    assert_eq!(
        linux.normalized(&other),
        Choices {
            engine: Some(Engine::Linux),
            ..Choices::default()
        }
    );
}

#[test]
fn each_engine_has_its_translator_and_only_rosetta_s_d3dmetal_takes_options() {
    assert_eq!(offer(Engine::Rosetta).translator, Translator::Rosetta);
    assert_eq!(offer(Engine::Linux).translator, Translator::Fex);
    for engine in [Engine::Rosetta, Engine::Linux] {
        for graphics in [Graphics::D3DMetal, Graphics::Dxmt] {
            let options = offer(engine).graphics(graphics).unwrap().options;
            let expected: &[GraphicsOption] = match (engine, graphics) {
                (Engine::Rosetta, Graphics::D3DMetal) => &[GraphicsOption::RayTracing],
                _ => &[],
            };
            assert_eq!(options, expected, "{engine:?} {graphics:?}");
        }
    }
}

#[test]
fn choices_are_kept_per_installation_folder_and_none_leaves_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let library = directory.path();
    let file = library.join("LaunchSettings/steam-42-windows.json");
    let install = Path::new("/library/Games/Steam/steam-42-windows");
    let choices = Choices {
        arguments: Some("-windowed".into()),
        engine: Some(Engine::Linux),
        ray_tracing: Some(false),
        ..Choices::default()
    };

    save(library, install, &choices).unwrap();

    assert!(file.is_file());
    assert_eq!(load(library, install), choices);
    assert_eq!(
        load(library, Path::new("/elsewhere/steam-43-windows")),
        Choices::default()
    );
    save(library, install, &Choices::default()).unwrap();
    assert!(!file.exists());
    remove(library, install).unwrap();
}

#[test]
fn an_unreadable_file_means_no_choices() {
    let directory = tempfile::tempdir().unwrap();
    let library = directory.path();
    fs::create_dir_all(library.join("LaunchSettings")).unwrap();
    fs::write(library.join("LaunchSettings/game.json"), "not json").unwrap();

    assert_eq!(load(library, Path::new("/games/game")), Choices::default());
}

#[test]
fn the_view_shows_each_engine_s_defaults_and_the_player_s_choices() {
    let choices = Choices {
        engine: Some(Engine::Rosetta),
        ..Choices::default()
    };
    let view = View::new(&profile::of(ELDEN_RING), choices.clone());

    assert_eq!(view.settings.engine, Engine::Rosetta);
    assert_eq!(view.choices, choices);
    assert_eq!(view.default_engine, Engine::Linux);
    assert_eq!(
        view.default_graphics,
        [Graphics::D3DMetal, Graphics::D3DMetal]
    );
}
