//! The note a game's page shows, such as what works of it and what does not
//! yet, and the table it comes from.
use crate::{ObeliskLocations, SessionStorefront};
use game_runtime::{notes, profile::Game};
use std::path::Path;

/// The note on the game `id` names, in the first of `languages` it is
/// written in, else in English: none for a game the table leaves out. `id`
/// is the game's id as the app spells it: a Steam app id, an Epic
/// `namespace:app_name`, a GOG product id.
#[uniffi::export]
pub fn game_note(
    storefront: SessionStorefront,
    id: String,
    languages: Vec<String>,
) -> Option<String> {
    let game = match storefront {
        SessionStorefront::Steam => Game::Steam {
            app_id: id.parse().ok()?,
        },
        SessionStorefront::Epic => {
            let (namespace, app_name) = id.split_once(':')?;
            Game::Epic {
                namespace,
                app_name,
            }
        }
        SessionStorefront::Gog => Game::Gog { product_id: &id },
    };
    notes::of(game, &languages)
}

/// Reads the notes again from Obelisk's `main`, keeping them in the
/// library. Runs once at startup; pages read the last ones kept until then.
#[uniffi::export(async_runtime = "tokio")]
pub async fn refresh_game_notes(locations: ObeliskLocations) {
    notes::refresh(Path::new(&locations.library)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> Vec<String> {
        vec!["en".into()]
    }

    #[test]
    fn a_game_s_note_is_found_by_the_id_the_app_gives_it() {
        let note = game_note(SessionStorefront::Steam, "440".into(), english());
        assert!(note.is_some_and(|note| note.contains("LAN")));
        let french = game_note(SessionStorefront::Steam, "440".into(), vec!["fr".into()]);
        assert!(french.is_some_and(|note| note.contains("hors ligne")));
        assert_eq!(
            game_note(SessionStorefront::Steam, "730".into(), english()),
            None
        );
    }

    #[test]
    fn an_id_that_names_no_game_of_its_storefront_has_no_note() {
        assert_eq!(
            game_note(SessionStorefront::Steam, "tf2".into(), english()),
            None
        );
        assert_eq!(
            game_note(SessionStorefront::Epic, "Sugar".into(), english()),
            None
        );
        assert_eq!(
            game_note(SessionStorefront::Gog, "440".into(), english()),
            None
        );
    }
}
