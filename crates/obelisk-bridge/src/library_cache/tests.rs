use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn cache(directory: &TempDir) -> Arc<LibraryCache> {
    LibraryCache::new(directory.path().to_str().unwrap().to_owned())
}

/// The file as the next Obelisk reads it.
fn file(cache: &LibraryCache, directory: &TempDir) -> Value {
    cache.flush();
    let bytes = std::fs::read(directory.path().join(library_cache::FILE_NAME)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn owned(app_id: u32, name: &str) -> steam_kit::client::OwnedGame {
    steam_kit::client::OwnedGame {
        app_id,
        name: name.into(),
        icon_hash: "hash".into(),
        last_played: 1_790_000_000,
        playtime_minutes: 90,
        record: json!({"appid": app_id, "has_dlc": true}),
    }
}

fn steam_game(app_id: u32, name: &str) -> SteamGame {
    SteamGame {
        app_id,
        name: name.into(),
        icon_hash: "hash".into(),
        last_played: 1_790_000_000,
        playtime_minutes: 90,
    }
}

fn epic(app_name: &str) -> epic_kit::client::Game {
    epic_kit::client::Game {
        app_name: app_name.into(),
        namespace: "space".into(),
        catalog_item_id: "item".into(),
        title: app_name.to_uppercase(),
        description: String::new(),
        image_url: Some("https://cdn.invalid/cover.png".into()),
        header_image_url: None,
        developer: "Psyonix".into(),
        windows: true,
        macos: false,
        metadata: json!({"catalogItems": {"item": {"id": "item"}}}),
    }
}

fn playtimes(cache: &LibraryCache) -> Vec<(String, Option<u64>)> {
    cache
        .epic_library("epic-a".into())
        .into_iter()
        .map(|cached| (cached.game.app_name, cached.playtime_seconds))
        .collect()
}

#[test]
fn a_steam_session_keeps_every_field_steam_returned_and_each_games_platforms() {
    let directory = TempDir::new().unwrap();
    let cache = cache(&directory);
    let library = cache.steam(7);
    let games = library.games(vec![owned(620, "Portal 2"), owned(570, "Dota 2")]);
    assert_eq!(
        games,
        [steam_game(620, "Portal 2"), steam_game(570, "Dota 2")]
    );
    library.platforms(vec![steam_kit::client::GamePlatforms {
        app_id: 620,
        platforms: vec!["macos".into(), "windows".into()],
        pics: Some(json!({"common": {"review_score": "9"}})),
    }]);

    library.games(vec![owned(620, "Portal 2")]);
    assert_eq!(
        cache.steam_library(7),
        [CachedSteamGame {
            game: steam_game(620, "Portal 2"),
            platforms: Some(vec!["macos".into(), "windows".into()]),
        }],
        "a reloaded library keeps the platforms loaded before"
    );
    let saved = &file(&cache, &directory)["steam"]["7"][0];
    assert_eq!(
        saved["record"]["has_dlc"], true,
        "stored as JSON, not as text"
    );
    assert_eq!(saved["pics"]["common"]["review_score"], "9");
    assert!(cache.steam_library(8).is_empty());
}

#[test]
fn an_epic_session_keeps_each_games_playtime_whichever_answers_first() {
    let directory = TempDir::new().unwrap();
    let cache = cache(&directory);
    let first = cache.epic("epic-a");
    first.games(vec![epic("rocket"), epic("fall")]);
    first.playtimes(&BTreeMap::from([("rocket".into(), 3600)]));
    assert_eq!(
        playtimes(&cache),
        [("rocket".into(), Some(3600)), ("fall".into(), None)]
    );

    let next = cache.epic("epic-a");
    next.games(vec![epic("rocket"), epic("fall")]);
    assert_eq!(
        playtimes(&cache)[0],
        ("rocket".into(), Some(3600)),
        "until its playtimes answer, a game keeps the earlier one"
    );
    let next = cache.epic("epic-a");
    next.playtimes(&BTreeMap::from([("fall".into(), 60)]));
    next.games(vec![epic("rocket"), epic("fall")]);
    assert_eq!(
        playtimes(&cache),
        [("rocket".into(), None), ("fall".into(), Some(60))]
    );
    let saved = &file(&cache, &directory)["epic"]["epic-a"][0];
    assert_eq!(saved["id"], "space:rocket");
    assert_eq!(saved["metadata"]["catalogItems"]["item"]["id"], "item");
    assert!(saved.get("headerImageUrl").is_none());
}

#[test]
fn launches_name_their_game_and_forgetting_an_account_stops_its_session_writing() {
    let directory = TempDir::new().unwrap();
    let cache = cache(&directory);
    let steam = cache.steam(7);
    steam.games(vec![owned(620, "Portal 2")]);
    cache.epic("epic-a").games(vec![epic("rocket")]);
    let portal = GameIdentity::Steam {
        app_id: 620,
        account_id: 7,
    };
    let rocket = GameIdentity::Epic {
        account_id: "epic-a".into(),
        namespace: "space".into(),
        app_name: "rocket".into(),
    };
    cache.record_launch(GameLaunch {
        game: portal.clone(),
        at: 1_790_000_000,
    });
    cache.record_launch(GameLaunch {
        game: rocket.clone(),
        at: 1_789_430_000,
    });
    let mut launches = cache.launches();
    launches.sort_by_key(|launch| launch.at);
    assert_eq!(
        launches,
        [
            GameLaunch {
                game: rocket.clone(),
                at: 1_789_430_000
            },
            GameLaunch {
                game: portal,
                at: 1_790_000_000
            },
        ]
    );

    cache.forget(GameAccount::Steam { steam_id: 7 });
    steam.games(vec![owned(620, "Portal 2")]);
    assert!(
        cache.steam_library(7).is_empty(),
        "a signed-out session's late answer"
    );
    assert_eq!(
        cache.launches(),
        [GameLaunch {
            game: rocket,
            at: 1_789_430_000
        }]
    );
}

#[test]
fn a_cache_written_by_obelisk_before_still_shows() {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join(library_cache::FILE_NAME),
        json!({
            "version": 1,
            "steam": {"7": [{
                "id": "620", "appId": 620, "name": "Portal 2", "iconHash": "hash",
                "lastPlayed": 1_790_000_000, "playtimeMinutes": 90, "platforms": ["macos"],
                "record": {"appid": 620}, "pics": {"common": {}},
                "lastLaunchedAt": "2026-09-21T14:13:20Z",
            }]},
            "epic": {"epic-a": [{
                "id": "space:rocket", "appName": "rocket", "namespace": "space", "catalogItemId": "item",
                "title": "ROCKET", "description": "", "imageUrl": "https://cdn.invalid/cover.png",
                "developer": "Psyonix", "windows": true, "macos": false, "playtimeSeconds": 3600,
                "metadata": {},
            }]},
        })
        .to_string(),
    )
    .unwrap();
    let cache = cache(&directory);
    assert_eq!(
        cache.steam_library(7),
        [CachedSteamGame {
            game: steam_game(620, "Portal 2"),
            platforms: Some(vec!["macos".into()]),
        }]
    );
    let cached = cache.epic_library("epic-a".into());
    assert_eq!(cached[0].game, EpicGame::from(epic("rocket")));
    assert_eq!(cached[0].playtime_seconds, Some(3600));
    assert_eq!(cache.launches().len(), 1);
}

fn gog(product_id: &str) -> gog_kit::client::Game {
    gog_kit::client::Game {
        product_id: product_id.into(),
        title: format!("Game {product_id}"),
        image_url: None,
        header_image_url: None,
        windows: true,
        macos: false,
        info: gog_kit::client::GameInfo {
            summary: "Survival horror.".into(),
            developers: vec!["Tango Gameworks".into()],
            release_date: Some(1_413_158_400),
            rating: Some(75),
            ..Default::default()
        },
        metadata: json!({"catalog": {"type": "game"}}),
    }
}

#[test]
fn a_gog_session_keeps_each_games_page_and_playtime() {
    let directory = TempDir::new().unwrap();
    let cache = cache(&directory);
    let library = cache.gog("48628349957132247");
    let shown = library.games(vec![gog("1"), gog("2")]);
    assert_eq!(shown[0].developers, ["Tango Gameworks"]);
    library.playtimes(&BTreeMap::from([("2".to_owned(), 90)]));
    // A later load keeps the playtimes the session last loaded.
    library.games(vec![gog("1"), gog("2")]);
    let cached = cache.gog_library("48628349957132247".into());
    let read: Vec<_> = cached
        .iter()
        .map(|cached| (cached.game.product_id.as_str(), cached.playtime_minutes))
        .collect();
    assert_eq!(read, [("1", None), ("2", Some(90))]);
    assert_eq!(cached[0].game.release_date, Some(1_413_158_400));
    assert_eq!(cached[0].game.rating, Some(75));
    let file = file(&cache, &directory);
    assert_eq!(
        file["gog"]["48628349957132247"][1]["metadata"]["catalog"]["type"],
        "game"
    );

    let launch = GameLaunch {
        game: GameIdentity::Gog {
            account_id: "48628349957132247".into(),
            product_id: "2".into(),
        },
        at: 1_790_000_000,
    };
    cache.record_launch(launch.clone());
    assert_eq!(cache.launches(), [launch]);
    cache.forget(GameAccount::Gog {
        user_id: "48628349957132247".into(),
    });
    assert!(cache.gog_library("48628349957132247".into()).is_empty());
}

#[test]
fn a_gog_game_cached_before_its_page_still_shows() {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join(library_cache::FILE_NAME),
        json!({"version": 1, "gog": {"7": [{"id": "1", "productId": "1", "title": "Old",
                                            "windows": true, "macos": false}]}})
        .to_string(),
    )
    .unwrap();
    let cached = cache(&directory).gog_library("7".into());
    assert_eq!(cached[0].game.title, "Old");
    assert!(cached[0].game.developers.is_empty());
}
