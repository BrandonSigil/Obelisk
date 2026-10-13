use super::*;
use crate::supervisor::Provider as _;

fn configuration(platform: &str, prefix: Option<&str>) -> Configuration {
    Configuration {
        home: "/Users/player".into(),
        install: "/library/Games/GOG/gog-1207658924-windows".into(),
        session_directory: "/library/GOGSessions".into(),
        helper: "/Obelisk.app/Contents/Helpers/SteamNative/obelisk-steam-host".into(),
        product_id: "1207658924".into(),
        platform: platform.into(),
        title: "The Witcher".into(),
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
fn a_configuration_from_before_titles_still_loads() {
    let saved = serde_json::json!({
        "home": "/Users/player",
        "install": "/library/Games/GOG/gog-1207658924-windows",
        "session_directory": "/library/GOGSessions",
        "helper": "/Obelisk.app/Contents/Helpers/SteamNative/obelisk-steam-host",
        "product_id": "1207658924",
        "platform": "windows",
        "prefix": "/library/Prefixes/gog-1207658924-windows",
    });
    let loaded: Configuration = serde_json::from_value(saved).unwrap();
    assert_eq!(
        loaded,
        Configuration {
            title: String::new(),
            ..configuration("windows", Some("/library/Prefixes/gog-1207658924-windows"))
        }
    );
}

#[test]
fn a_cancelled_gog_launch_ends_silently() {
    assert!(Gog::cancelled(&Error::Cancelled));
    assert!(Gog::cancelled(&Error::Gog(gog_kit::GogError::Cancelled)));
    assert!(!Gog::cancelled(&Error::Session));
}
