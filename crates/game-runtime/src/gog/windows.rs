//! A Windows GOG build: Wine starts it in its installation's prefix, once the
//! prefix ran the redistributables its build manifest names.
use super::{Configuration, Error, Result};
use crate::wine;

/// Readies a Windows build for its Game session: its prefix, the
/// redistributables its installation names, and the folder of Wine's log.
pub fn prepare_windows(
    config: &Configuration,
    locations: &crate::Locations,
) -> Result<wine::Config> {
    // The configuration may come from a saved journal: its prefix must be the
    // one of the installation it names.
    let game = crate::profile::Game::Gog {
        product_id: &config.product_id,
    };
    let windows = wine::Installation::of(locations, game, &config.platform, &config.install)?
        .filter(|windows| config.prefix.as_deref() == Some(windows.prefix()))
        .ok_or(Error::Unsupported)?;
    let wine = windows.runtime(
        locations,
        locations
            .gog_diagnostics
            .join(format!("wine-{}.log", config.product_id)),
    )?;
    let installed =
        gog_kit::client::installed_launch(&config.product_id, &config.platform, &config.install)?;
    let prefix = Prefix {
        prerequisites: installed.prerequisites(wine::windows_path),
    };
    wine::prepare(
        &wine,
        &config.home,
        &config.install,
        &installed.target,
        &prefix,
    )?;
    crate::linux_engine::translate_d3dmetal(&wine, &locations.helpers);
    Ok(wine)
}

/// What GOG adds to the prefix of an installation.
struct Prefix {
    prerequisites: Vec<gog_kit::client::Prerequisite>,
}

impl wine::Storefront for Prefix {
    /// The build's installer scripts and redistributables, as Galaxy runs
    /// them.
    fn prerequisites(&self) -> std::result::Result<Vec<wine::Prerequisite>, crate::Error> {
        Ok(self
            .prerequisites
            .iter()
            .map(|prerequisite| wine::Prerequisite {
                id: format!("gog:{}", prerequisite.id),
                programs: vec![(prerequisite.program.clone(), prerequisite.arguments.clone())],
            })
            .collect())
    }
}
