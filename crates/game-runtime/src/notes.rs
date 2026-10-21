//! What Obelisk tells players about particular games on their page, such as
//! what works and what does not yet: a game's note, whatever its storefront
//! or build. The table is `notes.json`, apart from the profiles so that an
//! Obelisk that reads only profiles never meets its keys. Obelisk is built
//! with it, and a release build reads it again from Obelisk's `main` when it
//! starts, keeping the last one beside its library; a development build
//! keeps the one it was built with. Each entry is checked as it is read, and
//! one that is not a note Obelisk can show is left out.
use crate::profile::Game;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{LazyLock, Mutex},
};

const TABLE: &str = include_str!("../notes.json");
/// Where the table is on Obelisk's `main`.
const URL: &str =
    "https://raw.githubusercontent.com/Zagrios/Obelisk/main/crates/game-runtime/notes.json";
/// The table's format this Obelisk reads. Another one is a table it ignores.
const FORMAT: u32 = 1;
/// The last table read, in Obelisk's library.
const KEPT: &str = "notes.json";
/// The language every note has, which stands in for one it lacks.
const FALLBACK: &str = "en";
/// The longest note shown, in characters: a few sentences beside the
/// game's details, not an article.
const LONGEST: usize = 400;

/// The table, as `notes.json` holds it: each game as written there, which
/// `checked` reads.
#[derive(Debug, Deserialize)]
struct Table {
    format: u32,
    games: Vec<serde_json::Value>,
}

/// One game of the table: its name, for the table's readers, the
/// storefronts' names for it and its note by language. Nothing else.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Known {
    name: String,
    steam: Option<u32>,
    epic: Option<EpicGame>,
    gog: Option<String>,
    /// The note in each language it is written in, by language code such
    /// as `en` or `fr`; English always.
    text: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpicGame {
    namespace: String,
    app_name: String,
}

/// The table pages read now.
struct Current {
    games: &'static [Known],
    /// The table read last, as it came: none until one is read, which
    /// leaves the one Obelisk was built with. A read that brings the same
    /// leaves `games` in place.
    read: Option<Vec<u8>>,
}

static CURRENT: LazyLock<Mutex<Current>> = LazyLock::new(|| {
    Mutex::new(Current {
        games: Vec::leak(parsed(TABLE.as_bytes()).expect("checked-in notes")),
        read: None,
    })
});

fn table() -> &'static [Known] {
    CURRENT.lock().unwrap().games
}

/// The entries of `table` that `checked` keeps, when it is a table of the
/// format Obelisk reads.
fn parsed(table: &[u8]) -> Option<Vec<Known>> {
    let table: Table = serde_json::from_slice(table).ok()?;
    (table.format == FORMAT).then(|| table.games.into_iter().filter_map(checked).collect())
}

/// `entry` as Obelisk reads it, when it holds only what Obelisk reads: a
/// name, a storefront's id and a note in English, each language's short
/// enough to show beside a game.
fn checked(entry: serde_json::Value) -> Option<Known> {
    // An entry is an object: serde would also read an array, field by field.
    if !entry.is_object() {
        return None;
    }
    let known: Known = serde_json::from_value(entry).ok()?;
    known.allowed().then_some(known)
}

/// Whether `code` is a language code: letters, with a region after a dash,
/// such as `pt-BR`.
fn language(code: &str) -> bool {
    let primary = code.split('-').next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && code.len() <= 8
        && code
            .split('-')
            .all(|part| !part.is_empty() && part.chars().all(|letter| letter.is_ascii_alphabetic()))
}

impl Known {
    fn allowed(&self) -> bool {
        !self.name.is_empty()
            && (self.steam.is_some() || self.epic.is_some() || self.gog.is_some())
            && self.text.contains_key(FALLBACK)
            && self.text.iter().all(|(code, note)| {
                language(code) && !note.trim().is_empty() && note.chars().count() <= LONGEST
            })
    }

    /// Whether this entry names `game` by its storefront's id.
    fn names(&self, game: Game<'_>) -> bool {
        match game {
            Game::Steam { app_id } => self.steam == Some(app_id),
            Game::Epic {
                namespace,
                app_name,
            } => self
                .epic
                .as_ref()
                .is_some_and(|epic| epic.namespace == namespace && epic.app_name == app_name),
            Game::Gog { product_id } => self.gog.as_deref() == Some(product_id),
        }
    }

    /// The note in the first of `languages` it is written in, matched
    /// exactly then by language alone, else in English.
    fn text(&self, languages: &[String]) -> &str {
        let primary = |code: &str| code.split('-').next().unwrap_or(code).to_ascii_lowercase();
        languages
            .iter()
            .find_map(|wanted| {
                self.text.get(wanted.as_str()).or_else(|| {
                    self.text
                        .iter()
                        .find(|(code, _)| primary(code) == primary(wanted))
                        .map(|(_, note)| note)
                })
            })
            .unwrap_or(&self.text[FALLBACK])
    }
}

/// The note on `game`, in the first of `languages`, the player's in order
/// of preference, that it is written in, else in English. None when the
/// table has none for it.
pub fn of(game: Game<'_>, languages: &[String]) -> Option<String> {
    table()
        .iter()
        .find(|known| known.names(game))
        .map(|known| known.text(languages).to_owned())
}

/// Reads the table again from Obelisk's `main` and keeps it in `library`.
/// Without the network, pages read the last one kept, else the one Obelisk
/// was built with. A development build keeps the one it was built with, as
/// edited.
pub async fn refresh(library: &Path) {
    if cfg!(debug_assertions) {
        return;
    }
    let kept = library.join(KEPT);
    match crate::remote::fetched(URL).await {
        Some(table) if read(&table) => {
            let _ = crate::remote::keep(&kept, &table);
        }
        _ => {
            let unread = CURRENT.lock().unwrap().read.is_none();
            if unread && let Ok(table) = fs::read(&kept) {
                read(&table);
            }
        }
    }
}

/// Makes `table` the one pages read, when it is a table Obelisk reads.
/// Whether it is. Unlike the profiles, a table without notes is one: the
/// last note may go.
fn read(table: &[u8]) -> bool {
    let mut current = CURRENT.lock().unwrap();
    if current.read.as_deref().unwrap_or(TABLE.as_bytes()) != table {
        let Some(games) = parsed(table) else {
            return false;
        };
        current.games = Vec::leak(games);
    }
    current.read = Some(table.to_vec());
    true
}

#[cfg(test)]
mod tests;
