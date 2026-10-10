//! What Obelisk knows about particular Windows games, whatever their
//! storefront: the engine that runs one, the graphics its Direct3D goes
//! through there, and the program started instead of its publisher's. Every
//! game this table does not name runs with the defaults, in Rosetta's Wine.
use crate::wine::{Engine, Graphics};
use obelisk_core::launch::LaunchTarget;
use std::path::Path;

/// A game as its storefront names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game<'a> {
    Steam {
        app_id: u32,
    },
    Epic {
        namespace: &'a str,
        app_name: &'a str,
    },
    Gog {
        product_id: &'a str,
    },
}

/// How Obelisk runs one Windows game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub engine: Engine,
    /// What renders its Direct3D on the Linux engine.
    pub graphics: Graphics,
    /// The program the game starts with instead of its publisher's, relative
    /// to the installation, when that one cannot run there.
    pub executable: Option<&'static str>,
}

/// The profile of `game`: the one this table gives it, else the defaults.
pub fn of(game: Game<'_>) -> Profile {
    match game {
        // Rocket League: its Easy Anti-Cheat refuses Rosetta's Wine (code
        // 206) and loads its Linux module on the Linux engine. Its publisher
        // executable, Launcher.exe, is a .NET program and the engine's Wine
        // has no Mono: the game starts with the Easy Anti-Cheat bootstrapper
        // Launcher.exe would start, with the same arguments.
        Game::Epic {
            namespace: "9773aa1aa54f4f7b80e44bef04986cea",
            app_name: "Sugar",
        } => Profile {
            engine: Engine::Linux,
            graphics: Graphics::Dxmt,
            executable: Some("Binaries/Win64/RocketLeague_EAC.exe"),
        },
        // ELDEN RING: Easy Anti-Cheat, as Rocket League's. It only has
        // Direct3D 12, which D3DMetal renders and DXMT does not. Steam's
        // launch already starts its bootstrapper, start_protected_game.exe.
        Game::Steam { app_id: 1245620 } => Profile {
            engine: Engine::Linux,
            graphics: Graphics::D3DMetal,
            executable: None,
        },
        _ => Profile::default(),
    }
}

impl Profile {
    /// What starts for `target` inside `install`: `target`, or the program
    /// the profile names instead, with the same arguments, in that program's
    /// folder.
    pub(crate) fn target(&self, install: &Path, target: &LaunchTarget) -> LaunchTarget {
        let Some(executable) = self.executable else {
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
}

#[cfg(test)]
mod tests;
