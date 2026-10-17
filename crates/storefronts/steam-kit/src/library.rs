//! What a game's PICS app info tells that the library orders by: when the
//! game came out, and how critics and players rate it.
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryFacts {
    /// The release its store page gives, else its first one elsewhere, in
    /// seconds since the Unix epoch.
    pub release_date: Option<u64>,
    /// Out of 100.
    pub metacritic_score: Option<u32>,
    /// Steam's verdict on its players' reviews, from 1, overwhelmingly
    /// negative, to 9, overwhelmingly positive.
    pub review_score: Option<u32>,
    /// The share of its players' reviews that are positive, out of 100.
    pub review_percentage: Option<u32>,
}

/// What `pics`, a game's whole PICS app info, tells of it. A field it leaves
/// out, or names 0, is unknown.
pub fn library_facts(pics: &Value) -> LibraryFacts {
    let common = &pics["common"];
    let number = |key: &str| positive(&common[key]);
    let score = |key: &str, max: u32| {
        number(key)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value <= max)
    };
    let review_score = score("review_score", 9);
    LibraryFacts {
        release_date: number("steam_release_date").or_else(|| number("original_release_date")),
        metacritic_score: score("metacritic_score", 100),
        review_score,
        // A game without enough reviews for a verdict has no share either.
        review_percentage: review_score.and(score("review_percentage", 100)),
    }
}

/// A number of a game's PICS app info, which PICS mostly writes as text;
/// none when absent or 0.
pub(crate) fn positive(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => text.parse().ok(),
        value => value.as_u64(),
    }
    .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_release_and_the_ratings_steam_shows() {
        let pics = json!({ "common": {
            "steam_release_date": "1645744020",
            "original_release_date": "1600000000",
            "metacritic_score": "94",
            "review_score": "8",
            "review_percentage": "93"
        }});
        assert_eq!(
            library_facts(&pics),
            LibraryFacts {
                release_date: Some(1_645_744_020),
                metacritic_score: Some(94),
                review_score: Some(8),
                review_percentage: Some(93),
            }
        );
    }

    #[test]
    fn a_game_first_released_elsewhere_has_that_date() {
        let pics = json!({ "common": { "original_release_date": "1383897600" } });
        assert_eq!(library_facts(&pics).release_date, Some(1_383_897_600));
    }

    #[test]
    fn tells_nothing_a_game_does_not_publish_or_publishes_out_of_range() {
        assert_eq!(library_facts(&json!({})), LibraryFacts::default());
        let pics = json!({ "common": {
            "steam_release_date": "0",
            "metacritic_score": "140",
            "review_score": "0",
            "review_percentage": "75"
        }});
        assert_eq!(library_facts(&pics), LibraryFacts::default());
    }
}
