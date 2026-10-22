//! The note a game's page shows, such as what works of it and what does not
//! yet, and the table it comes from.
use crate::{ObeliskLocations, SessionStorefront, installations::Named};
use game_runtime::{epic, gog, notes, profile::Game, steam};
use std::path::Path;

/// The note on the game `id` names, as the app names it, in `language`, else
/// in English: none for a game the table leaves out.
#[uniffi::export]
pub fn game_note(storefront: SessionStorefront, id: String, language: String) -> Option<String> {
    match storefront {
        SessionStorefront::Steam => notes::of(
            Game::Steam {
                app_id: <steam::Steam as Named>::named(&id)?,
            },
            &language,
        ),
        SessionStorefront::Epic => {
            let store = <epic::Epic as Named>::named(&id)?;
            notes::of(
                Game::Epic {
                    namespace: &store.namespace,
                    app_name: &store.app_name,
                },
                &language,
            )
        }
        SessionStorefront::Gog => notes::of(
            Game::Gog {
                product_id: &<gog::Gog as Named>::named(&id)?,
            },
            &language,
        ),
    }
}

/// Reads the notes again from Obelisk's `main`, keeping them in the
/// library. Runs once at startup: pages read those Obelisk was built with
/// until then, and the last ones kept when `main` does not answer.
#[uniffi::export(async_runtime = "tokio")]
pub async fn refresh_game_notes(locations: ObeliskLocations) {
    notes::refresh(Path::new(&locations.library)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_s_note_is_found_by_the_id_the_app_gives_it() {
        let note = |id: &str| game_note(SessionStorefront::Steam, id.into(), "en".into());
        assert!(note("440").is_some());
        assert_eq!(note("tf2"), None);
    }

    #[test]
    fn an_id_that_names_no_game_of_its_storefront_has_no_note() {
        assert_eq!(
            game_note(SessionStorefront::Epic, "Sugar".into(), "en".into()),
            None
        );
        assert_eq!(
            game_note(SessionStorefront::Gog, "440".into(), "en".into()),
            None
        );
    }
}
