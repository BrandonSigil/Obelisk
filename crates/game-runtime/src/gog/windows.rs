//! A Windows GOG build: Wine starts it in its installation's prefix, once the
//! prefix ran the prerequisites its build manifest names.
use super::{Configuration, Result};
use crate::{supervisor::LaunchStep, wine};

/// Readies a Windows build for its Game session in `windows`, its
/// installation's prefix, with the prerequisites its build manifest names,
/// showing through `step` what it waits on.
pub(super) fn ready(
    windows: &wine::Installation,
    config: &Configuration,
    locations: &crate::Locations,
    step: &dyn Fn(LaunchStep),
) -> Result<wine::Config> {
    let installed =
        gog_kit::client::installed_launch(&config.product_id, config.platform, &config.install)?;
    let prefix = Prefix {
        prerequisites: installed.prerequisites(wine::windows_path),
    };
    Ok(windows.ready(locations, &installed.target, &prefix, step)?)
}

/// What GOG adds to the prefix of an installation.
pub(super) struct Prefix {
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
                name: prerequisite.name.clone(),
                programs: vec![(prerequisite.program.clone(), prerequisite.arguments.clone())],
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
