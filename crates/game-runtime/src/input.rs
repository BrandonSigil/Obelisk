//! Obelisk's input service, voile-lkd's obelisk-input: it reads the Mac's
//! gamepads with SDL3 and serves them to Wine's Obelisk bus, which offers
//! them to Windows games through XInput. The Linux engine's launcher starts
//! its own; a game in Wine for macOS gets one from its supervisor for as long
//! as it runs.
use crate::wine::{Config, Engine};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Tells Wine's Obelisk bus where the service listens.
pub(crate) const SOCKET: &str = "WINE_OBELISK_INPUT_SOCKET";
/// The service starts in a few milliseconds; a game never waits longer.
const START_LIMIT: Duration = Duration::from_secs(2);
/// It ends within milliseconds of SIGTERM; after this it is killed.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// A running service, stopped when dropped.
pub(crate) struct Service {
    child: Child,
}

impl Service {
    /// Starts the service of `config`'s runtime for a game in Wine for macOS,
    /// with the socket Wine must be given. Without a service in the runtime,
    /// or when it does not start, the game runs without gamepads.
    pub(crate) fn start(config: &Config) -> Option<(Self, PathBuf)> {
        if config.engine != Engine::Rosetta {
            return None;
        }
        // It ships with the Linux engine's host programs, in every runtime.
        let program = config.runtime.join("linux/engine/obelisk-input");
        if !program.is_file() {
            return None;
        }
        let directory = directory(&config.prefix);
        let socket = directory.join("input.sock");
        let log = File::create(config.log.with_extension("input.log")).ok()?;
        let child = Command::new(program)
            .arg("--dir")
            .arg(&directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .ok()?;
        let mut service = Self { child };
        let deadline = Instant::now() + START_LIMIT;
        while !ready(&socket) {
            if Instant::now() >= deadline || !matches!(service.child.try_wait(), Ok(None)) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Some((service, socket))
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // SIGTERM lets SDL give each gamepad back its own behaviour, such as
        // the Steam Controller's mouse mode.
        let _ = Command::new("/bin/kill")
            .args(["-TERM", &self.child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + STOP_GRACE;
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The service's folder for `prefix`: the same for every launch of the
/// prefix, so a Wine server kept from the previous game finds the new
/// service, and short enough for a socket path.
fn directory(prefix: &Path) -> PathBuf {
    let digest = Sha256::digest(prefix.as_os_str().as_encoded_bytes());
    let name: String = digest[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("obelisk-input-{name}"))
}

fn ready(socket: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(socket).is_ok_and(|metadata| metadata.file_type().is_socket())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_keeps_its_folder_and_two_prefixes_do_not_share_one() {
        let first = directory(Path::new("/library/Prefixes/epic-game-Windows"));
        assert_eq!(first, directory(Path::new("/library/Prefixes/epic-game-Windows")));
        assert_ne!(first, directory(Path::new("/library/Prefixes/steam-1-windows")));
        // A Mac's TMPDIR is about 50 bytes; a socket path takes at most 103.
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.len() + "/input.sock".len() <= 103 - 50, "{name}");
    }
}
