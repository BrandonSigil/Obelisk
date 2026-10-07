use super::*;
use crate::{installations::Storefront as _, supervisor::Provider as _};

fn configuration(platform: &str, prefix: Option<&str>) -> Configuration {
    Configuration {
        home: "/Users/player".into(),
        install: "/library/Games/GOG/gog-1207658924-windows".into(),
        session_directory: "/library/GOGSessions".into(),
        helper: "/Obelisk.app/Contents/Helpers/SteamNative/obelisk-steam-host".into(),
        product_id: "1207658924".into(),
        platform: platform.into(),
        prefix: prefix.map(Into::into),
    }
}

#[test]
fn a_configuration_runs_a_mac_build_natively_and_a_windows_build_in_wine() {
    let account = "48628349957132247";
    configuration("macos", None).validate(account).unwrap();
    configuration("windows", Some("/library/Prefixes/gog-1207658924-windows"))
        .validate(account)
        .unwrap();
    for refused in [
        configuration("macos", Some("/library/Prefixes/x")),
        configuration("windows", None),
        configuration("linux", None),
        configuration("windows", Some("relative")),
        Configuration {
            product_id: "../1207658924".into(),
            ..configuration("macos", None)
        },
    ] {
        assert!(refused.validate(account).is_err(), "{refused:?}");
    }
    assert!(configuration("macos", None).validate("not-a-user").is_err());
}

#[test]
fn receipts_name_one_installation_per_game_and_platform() {
    let receipt = Receipt {
        product_id: "1207658924".into(),
        name: "The Witcher".into(),
        directory: "/library/Games/GOG/gog-1207658924-windows".into(),
        platform: "windows".into(),
        total_bytes: 7,
        needs_verification: None,
    };
    assert_eq!(Gog::receipt_id(&receipt), "1207658924:windows");
    let update = Gog::update_request(&receipt);
    assert_eq!(update.directory, std::path::Path::new("/library/Games/GOG"));
    assert_eq!(update.title, "The Witcher");
    let mut marked = receipt.clone();
    Gog::mark_needs_verification(&mut marked);
    assert!(Gog::needs_verification(&marked));
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["productId"],
        "1207658924"
    );
}

#[test]
fn a_cancelled_gog_launch_ends_silently() {
    assert!(Gog::cancelled(&Error::Cancelled));
    assert!(Gog::cancelled(&Error::Gog(gog_kit::GogError::Cancelled)));
    assert!(!Gog::cancelled(&Error::Session));
}
