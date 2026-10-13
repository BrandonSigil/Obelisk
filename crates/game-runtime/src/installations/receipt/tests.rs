//! The records every storefront's installations share, and what each
//! storefront adds to them.
use super::*;
use crate::{
    epic::{self, Epic},
    gog::{self, Gog},
    steam::{self, Steam},
};

/// Where Obelisk installed each game before: an update stays there.
const PREVIOUS: &str = "/library/Previous location";

/// Portal 2's installation of `platform`, with its English depots.
fn portal(platform: &str) -> Receipt<Steam> {
    Receipt {
        product: steam::Product {
            app_id: 620,
            language: "english".into(),
        },
        name: "Portal 2".into(),
        directory: format!("{PREVIOUS}/steam-620-{platform}"),
        platform: platform.into(),
        total_bytes: 7,
        needs_verification: false,
    }
}

/// An Epic game's installation of `platform`, with its catalog item.
fn sandbox_game(platform: &str) -> Receipt<Epic> {
    Receipt {
        product: epic::Product {
            namespace: "sandbox".into(),
            catalog_item_id: "catalog".into(),
            app_name: "game".into(),
        },
        name: "Fixture".into(),
        directory: format!("{PREVIOUS}/epic-hash-{platform}"),
        platform: platform.into(),
        total_bytes: 7,
        needs_verification: false,
    }
}

/// The Witcher's installation of `platform`.
fn witcher(platform: &str) -> Receipt<Gog> {
    Receipt {
        product: gog::Product {
            product_id: "1207658924".into(),
        },
        name: "The Witcher".into(),
        directory: format!("{PREVIOUS}/gog-1207658924-{platform}"),
        platform: platform.into(),
        total_bytes: 7,
        needs_verification: false,
    }
}

/// The receipts `file` holds, which save back to the same keys.
fn kept<S: Storefront>(file: serde_json::Value) -> Vec<Receipt<S>> {
    let receipts: Vec<Receipt<S>> = serde_json::from_value(file.clone()).unwrap();
    assert_eq!(serde_json::to_value(&receipts).unwrap(), file);
    receipts
}

#[test]
fn each_storefronts_receipt_file_keeps_its_own_keys() {
    let steam: Vec<Receipt<Steam>> = kept(serde_json::json!([
        {
            "appId": 620,
            "name": "Portal 2",
            "directory": "/library/Games/Steam/steam-620-macos",
            "platform": "macos",
            "language": "french",
            "totalBytes": 100,
            "needsVerification": true
        },
        {
            "appId": 620,
            "name": "Portal 2",
            "directory": "/library/Games/Steam/steam-620-windows",
            "platform": "windows",
            "language": "french",
            "totalBytes": 100
        }
    ]));
    assert!(steam[0].needs_verification && !steam[1].needs_verification);
    let epic: Vec<Receipt<Epic>> = kept(serde_json::json!([{
        "namespace": "sandbox",
        "catalogItemId": "catalog",
        "appName": "game",
        "name": "Fixture",
        "directory": "/library/Games/Epic/epic-hash-Windows",
        "platform": "Windows",
        "totalBytes": 7
    }]));
    assert_eq!(epic[0].product.catalog_item_id, "catalog");
    let gog: Vec<Receipt<Gog>> = kept(serde_json::json!([{
        "productId": "1207658924",
        "name": "The Witcher",
        "directory": "/library/Games/GOG/gog-1207658924-windows",
        "platform": "windows",
        "totalBytes": 7
    }]));
    assert_eq!(gog[0].product.product_id, "1207658924");

    // An older Epic file has no catalog item at all.
    let older: Vec<Receipt<Epic>> = serde_json::from_value(serde_json::json!([{
        "namespace": "sandbox",
        "appName": "game",
        "name": "Fixture",
        "directory": "/library/Games/Epic/epic-hash-Mac",
        "platform": "Mac",
        "totalBytes": 7
    }]))
    .unwrap();
    assert_eq!(older[0].product.catalog_item_id, "");
}

#[test]
fn receipts_name_one_installation_per_game_and_platform() {
    assert_eq!(portal("macos").id(), "620-macos");
    assert_eq!(sandbox_game("Windows").id(), "sandbox:game:Windows");
    assert_eq!(witcher("windows").id(), "1207658924:windows");
    assert_eq!(portal("macos").game(), 620);
    assert_eq!(
        sandbox_game("Windows").game(),
        epic::Store {
            namespace: "sandbox".into(),
            app_name: "game".into(),
        }
    );
    assert_eq!(witcher("windows").game(), "1207658924");
}

/// `receipt`'s update is `expected`, which goes over it.
fn updates<S: Storefront>(receipt: &Receipt<S>, expected: Transfer<S>) {
    assert_eq!(receipt.transfer(), expected);
    assert!(expected.lands_on(receipt));
}

#[test]
fn an_update_goes_over_its_installation_in_the_folder_it_sits_in() {
    updates(
        &portal("macos"),
        Transfer {
            product: steam::Product {
                app_id: 620,
                language: "english".into(),
            },
            name: "Portal 2".into(),
            platform: Some("macos".into()),
            directory: PREVIOUS.into(),
        },
    );
    updates(
        &sandbox_game("Windows"),
        Transfer {
            product: epic::Product {
                namespace: "sandbox".into(),
                catalog_item_id: "catalog".into(),
                app_name: "game".into(),
            },
            name: "Fixture".into(),
            platform: "Windows".into(),
            directory: PREVIOUS.into(),
        },
    );
    updates(
        &witcher("windows"),
        Transfer {
            product: gog::Product {
                product_id: "1207658924".into(),
            },
            name: "The Witcher".into(),
            platform: "windows".into(),
            directory: PREVIOUS.into(),
        },
    );
}

#[test]
fn a_transfer_lands_on_its_games_installation_of_its_platform_or_any_while_the_storefront_picks() {
    let (mac, windows) = (portal("macos"), portal("windows"));
    let picked = Transfer {
        platform: None,
        ..mac.transfer()
    };
    assert!(picked.lands_on(&mac) && picked.lands_on(&windows));
    let chosen = windows.transfer();
    assert!(chosen.lands_on(&windows) && !chosen.lands_on(&mac));
    let mut other = picked.clone();
    other.product.app_id = 730;
    assert!(!other.lands_on(&mac) && !other.lands_on(&windows));

    let (mac, windows) = (witcher("macos"), witcher("windows"));
    let chosen = windows.transfer();
    assert!(chosen.lands_on(&windows) && !chosen.lands_on(&mac));
    let mut other = chosen.clone();
    other.product.product_id = "1495134320".into();
    assert!(!other.lands_on(&windows) && !other.lands_on(&mac));

    // A new installation's Transfer starts from an offer.
    let chosen = Offer::<Gog> {
        product: mac.product.clone(),
        name: "The Witcher".into(),
        platform: "macos".into(),
    }
    .transfer("/library/Games/GOG".into());
    assert!(chosen.lands_on(&mac) && !chosen.lands_on(&windows));
    let picked = Offer::<Steam> {
        product: portal("macos").product,
        name: "Portal 2".into(),
        platform: None,
    }
    .transfer("/library/Games/Steam".into());
    assert!(picked.lands_on(&portal("macos")) && picked.lands_on(&portal("windows")));
}

/// Uninstalls `windows` and `mac`, two builds of one game, whose folders are
/// gone: the Windows build's prefix goes with it.
fn removes_its_prefix<S: Storefront>(windows: Receipt<S>, mac: Receipt<S>) {
    let (_directory, root) = crate::wine::tests::installation();
    let locations = crate::wine::tests::locations(&root);
    let folder = Path::new(&windows.directory).file_name().unwrap();
    let prefix = locations.wine_prefixes.join(folder);
    std::fs::create_dir_all(prefix.join("drive_c")).unwrap();

    assert_eq!(windows.remove_prefix(&locations), Some(Ok(())));
    assert!(!prefix.exists());
    assert_eq!(mac.remove_prefix(&locations), None);
}

#[test]
fn an_uninstalled_windows_build_loses_its_prefix_and_a_mac_build_has_none() {
    removes_its_prefix(portal("windows"), portal("macos"));
    removes_its_prefix(sandbox_game("Windows"), sandbox_game("Mac"));
    removes_its_prefix(witcher("windows"), witcher("macos"));
}
