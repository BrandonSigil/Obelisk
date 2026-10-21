use super::*;
use std::collections::HashSet;

const TEAM_FORTRESS_2: Game<'static> = Game::Steam { app_id: 440 };

fn languages(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| (*code).to_owned()).collect()
}

fn entries() -> Vec<serde_json::Map<String, serde_json::Value>> {
    let table: serde_json::Value = serde_json::from_str(TABLE).unwrap();
    table["games"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_object().unwrap().clone())
        .collect()
}

#[test]
fn every_note_of_the_table_passes_the_checks_a_table_read_from_main_passes() {
    assert_eq!(table().len(), entries().len());
}

#[test]
fn no_two_notes_of_the_table_name_the_same_game() {
    let mut names = HashSet::new();
    for known in table() {
        let ids = [
            known.steam.map(|app_id| format!("steam {app_id}")),
            known
                .epic
                .as_ref()
                .map(|epic| format!("epic {}:{}", epic.namespace, epic.app_name)),
            known.gog.as_ref().map(|product| format!("gog {product}")),
        ];
        for id in ids.into_iter().flatten() {
            assert!(names.insert(id.clone()), "{id}");
        }
    }
}

#[test]
fn team_fortress_2_says_it_plays_on_lan_but_not_on_secure_servers() {
    let english = of(TEAM_FORTRESS_2, &languages(&["en-US"])).unwrap();
    assert!(english.contains("LAN"), "{english}");
    assert!(english.contains("VAC"), "{english}");
    assert!(of(Game::Steam { app_id: 730 }, &languages(&["en"])).is_none());
}

/// A table entry Obelisk keeps, with `key` set to `value`, or removed
/// without one.
fn entry_with(key: &str, value: Option<serde_json::Value>) -> Option<Known> {
    let mut entry = serde_json::json!({
        "name": "A game",
        "steam": 42,
        "text": {"en": "Plays offline.", "fr": "Se joue hors ligne.", "pt-BR": "Joga offline."}
    });
    let fields = entry.as_object_mut().unwrap();
    match value {
        Some(value) => fields.insert(key.into(), value),
        None => fields.remove(key),
    };
    checked(entry)
}

#[test]
fn a_note_follows_the_player_s_languages_in_order_then_english() {
    let known = entry_with("name", Some("A game".into())).unwrap();
    assert_eq!(
        known.text(&languages(&["fr-FR", "en"])),
        "Se joue hors ligne."
    );
    assert_eq!(known.text(&languages(&["fr-CA"])), "Se joue hors ligne.");
    assert_eq!(
        known.text(&languages(&["de-DE", "fr"])),
        "Se joue hors ligne."
    );
    assert_eq!(known.text(&languages(&["pt-BR"])), "Joga offline.");
    assert_eq!(known.text(&languages(&["pt-PT"])), "Joga offline.");
    assert_eq!(known.text(&languages(&["de"])), "Plays offline.");
    assert_eq!(known.text(&[]), "Plays offline.");
}

#[test]
fn an_entry_that_is_not_a_note_obelisk_can_show_is_left_out() {
    use serde_json::json;
    assert!(entry_with("name", Some("A game".into())).is_some());
    // A storefront's id, and a name.
    assert!(entry_with("steam", None).is_none());
    assert!(entry_with("gog", Some("1207658924".into())).is_some());
    assert!(entry_with("name", Some("".into())).is_none());
    // English always, every language short and not empty.
    assert!(entry_with("text", Some(json!({"fr": "Se joue hors ligne."}))).is_none());
    assert!(entry_with("text", Some(json!({"en": "  "}))).is_none());
    assert!(entry_with("text", Some(json!({"en": "a".repeat(LONGEST)}))).is_some());
    assert!(entry_with("text", Some(json!({"en": "a".repeat(LONGEST + 1)}))).is_none());
    assert!(entry_with("text", Some(json!({"en": "Plays.", "english": "Plays."}))).is_none());
    assert!(entry_with("text", Some(json!({"en": "Plays.", "f1": "Plays."}))).is_none());
    // Nothing else: an Obelisk that reads only notes never runs a game by them.
    assert!(entry_with("arguments", Some(json!(["-dxlevel", "95"]))).is_none());
    assert!(checked(json!([["A game"]])).is_none());
}

#[test]
fn a_table_of_another_format_is_not_read() {
    let mut table: serde_json::Value = serde_json::from_str(TABLE).unwrap();
    table["format"] = 2.into();
    assert!(parsed(table.to_string().as_bytes()).is_none());
    assert!(parsed(b"404: Not Found").is_none());
    assert_eq!(
        parsed(br#"{"format": 1, "games": []}"#).map(|games| games.len()),
        Some(0)
    );
}
