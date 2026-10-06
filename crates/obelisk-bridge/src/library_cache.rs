//! The library cache across UniFFI. Each storefront session keeps the library
//! it loads, with everything its storefront returned; Swift reads the last one
//! of an account while its session connects, records launches and forgets an
//! account that signs out.
use crate::{
    EpicGame, GameAccount, GameIdentity, SteamGame, game_session::Bridged, installations::Named,
};
use game_runtime::{epic, steam};
use obelisk_core::library_cache::{self, AccountLibrary, Entry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const STEAM: &str = "steam";
const EPIC: &str = "epic";

/// The last library of every storefront account, in the library directory.
#[derive(uniffi::Object)]
pub struct LibraryCache {
    inner: library_cache::LibraryCache,
}

/// A Steam game of the cache, with its platforms once Steam told them.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CachedSteamGame {
    pub game: SteamGame,
    pub platforms: Option<Vec<String>>,
}

/// An Epic game of the cache, with its playtime once Epic told it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CachedEpicGame {
    pub game: EpicGame,
    pub playtime_seconds: Option<u64>,
}

/// When an account launched a game from Obelisk.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct GameLaunch {
    pub game: GameIdentity,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

#[uniffi::export]
impl LibraryCache {
    /// The cache of the library in `directory`. Every cache of one directory
    /// shares one document.
    #[uniffi::constructor]
    pub fn new(directory: String) -> Arc<Self> {
        Arc::new(Self {
            inner: library_cache::LibraryCache::open(directory),
        })
    }

    /// The account's last Steam library, empty when it has none.
    pub fn steam_library(&self, steam_id: u64) -> Vec<CachedSteamGame> {
        // Each field on its own: reading the entry flattened would copy its PICS.
        self.inner.library(STEAM, &steam_id.to_string(), |entry| {
            Some(CachedSteamGame {
                game: SteamGame::deserialize(entry).ok()?,
                platforms: entry
                    .get("platforms")
                    .and_then(|platforms| Vec::deserialize(platforms).ok()),
            })
        })
    }

    /// The account's last Epic library, empty when it has none.
    pub fn epic_library(&self, account_id: String) -> Vec<CachedEpicGame> {
        // Each field on its own: reading the entry flattened would copy its
        // metadata.
        self.inner.library(EPIC, &account_id, |entry| {
            Some(CachedEpicGame {
                game: EpicGame::deserialize(entry).ok()?,
                playtime_seconds: entry.get("playtimeSeconds").and_then(Value::as_u64),
            })
        })
    }

    /// Every launch the cache keeps, for every storefront and account.
    pub fn launches(&self) -> Vec<GameLaunch> {
        self.inner
            .launches()
            .into_iter()
            .filter_map(|launch| {
                let game = match launch.storefront.as_str() {
                    STEAM => <steam::Steam as Bridged>::identity(
                        &<steam::Steam as Named>::named(&launch.game)?,
                        &launch.account.parse().ok()?,
                    ),
                    EPIC => <epic::Epic as Bridged>::identity(
                        &<epic::Epic as Named>::named(&launch.game)?,
                        &launch.account,
                    ),
                    _ => return None,
                };
                Some(GameLaunch {
                    game,
                    at: launch.at,
                })
            })
            .collect()
    }

    /// Records a launch in its game's entry. A game whose library was never
    /// kept has no entry to keep it.
    pub fn record_launch(&self, launch: GameLaunch) {
        let (storefront, account, game) = match launch.game {
            GameIdentity::Steam { app_id, account_id } => (
                STEAM,
                account_id.to_string(),
                <steam::Steam as Named>::name(&app_id),
            ),
            GameIdentity::Epic {
                account_id,
                namespace,
                app_name,
            } => (
                EPIC,
                account_id,
                <epic::Epic as Named>::name(&epic::Store {
                    namespace,
                    app_name,
                }),
            ),
        };
        self.inner
            .record_launch(storefront, &account, &game, launch.at);
    }

    /// Removes the account's library and its launches, as signing it out
    /// does. Its session can no longer write it.
    pub fn forget(&self, account: GameAccount) {
        match account {
            GameAccount::Steam { steam_id } => self.inner.forget(STEAM, &steam_id.to_string()),
            GameAccount::Epic { account_id } => self.inner.forget(EPIC, &account_id),
        }
    }

    /// Writes the last change now, as Obelisk quits.
    pub fn flush(&self) {
        self.inner.flush();
    }
}

impl LibraryCache {
    pub(crate) fn steam(&self, steam_id: u64) -> SteamLibrary {
        SteamLibrary(self.inner.account(STEAM, &steam_id.to_string()))
    }

    pub(crate) fn epic(&self, account_id: &str) -> EpicLibrary {
        EpicLibrary {
            account: self.inner.account(EPIC, account_id),
            playtimes: Default::default(),
        }
    }
}

/// The Steam library a session keeps for its account.
pub(crate) struct SteamLibrary(AccountLibrary);

impl SteamLibrary {
    /// Keeps the account's games with every field Steam returned. Each keeps
    /// the platforms and PICS app info an earlier request loaded.
    pub(crate) fn games(&self, games: Vec<steam_kit::client::OwnedGame>) -> Vec<SteamGame> {
        let (entries, games) = games
            .into_iter()
            .map(|mut owned| {
                let record = std::mem::take(&mut owned.record);
                let game = SteamGame::from(owned);
                let mut entry = entry_of(&SteamEntry {
                    id: <steam::Steam as Named>::name(&game.app_id),
                    game: game.clone(),
                    platforms: None,
                });
                entry.insert("record".into(), record);
                (entry, game)
            })
            .unzip();
        self.0.replace(entries, &["platforms", "pics"]);
        games
    }

    /// Keeps each game's platforms and whole PICS app info.
    pub(crate) fn platforms(&self, games: Vec<steam_kit::client::GamePlatforms>) {
        let mut games: BTreeMap<String, _> = games
            .into_iter()
            .map(|game| (<steam::Steam as Named>::name(&game.app_id), game))
            .collect();
        self.0.update(|entry| {
            let Some(game) = entry
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| games.remove(id))
            else {
                return false;
            };
            let platforms = set(entry, "platforms", game.platforms.into());
            game.pics.is_some_and(|pics| set(entry, "pics", pics)) | platforms
        });
    }
}

/// The Epic library a session keeps for its account.
pub(crate) struct EpicLibrary {
    account: AccountLibrary,
    /// The playtimes the session last loaded, by app name: they may answer
    /// before the games.
    playtimes: Mutex<Option<BTreeMap<String, u64>>>,
}

impl EpicLibrary {
    /// Keeps the account's games with everything Epic returned. Until the
    /// session loads the playtimes, each game keeps its earlier one.
    pub(crate) fn games(&self, games: Vec<epic_kit::client::Game>) -> Vec<EpicGame> {
        let playtimes = self.playtimes.lock().unwrap();
        let (entries, games) = games
            .into_iter()
            .map(|mut loaded| {
                let metadata = std::mem::take(&mut loaded.metadata);
                let game = EpicGame::from(loaded);
                let mut entry = entry_of(&EpicEntry {
                    id: <epic::Epic as Named>::name(&epic::Store {
                        namespace: game.namespace.clone(),
                        app_name: game.app_name.clone(),
                    }),
                    playtime_seconds: playtimes
                        .as_ref()
                        .and_then(|playtimes| playtimes.get(&game.app_name).copied()),
                    game: game.clone(),
                });
                entry.insert("metadata".into(), metadata);
                (entry, game)
            })
            .unzip();
        let kept: &[&str] = if playtimes.is_some() {
            &[]
        } else {
            &["playtimeSeconds"]
        };
        self.account.replace(entries, kept);
        games
    }

    /// Keeps every game's playtime. A game missing from `playtimes` was
    /// never played.
    pub(crate) fn playtimes(&self, playtimes: &BTreeMap<String, u64>) {
        *self.playtimes.lock().unwrap() = Some(playtimes.clone());
        self.account.update(|entry| {
            let seconds = entry
                .get("appName")
                .and_then(Value::as_str)
                .and_then(|app_name| playtimes.get(app_name).copied());
            match seconds {
                Some(seconds) => set(entry, "playtimeSeconds", seconds.into()),
                None => entry.remove("playtimeSeconds").is_some(),
            }
        });
    }
}

fn entry_of(entry: &impl Serialize) -> Entry {
    match serde_json::to_value(entry) {
        Ok(Value::Object(entry)) => entry,
        _ => Entry::new(),
    }
}

/// Sets the entry's `key` to `value`; `true` when it changed.
fn set(entry: &mut Entry, key: &str, value: Value) -> bool {
    if entry.get(key) == Some(&value) {
        return false;
    }
    entry.insert(key.into(), value);
    true
}

/// A Steam game as the cache keeps it: as the library shows it, with its
/// platforms. Its entry also keeps its `Player.GetOwnedGames` `record` and
/// its whole PICS app info, `pics`.
#[derive(Serialize)]
struct SteamEntry {
    id: String,
    #[serde(flatten)]
    game: SteamGame,
    /// Absent until Steam told the game's platforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    platforms: Option<Vec<String>>,
}

/// An Epic game as the cache keeps it: as the library shows it, with its
/// playtime. Its entry also keeps its catalog items, library records and
/// assets, `metadata`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EpicEntry {
    id: String,
    #[serde(flatten)]
    game: EpicGame,
    /// Absent until Epic told it, or when the game was never played.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    playtime_seconds: Option<u64>,
}

#[cfg(test)]
mod tests;
