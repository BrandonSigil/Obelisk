//! Bounded reads and the status mapping every GOG request shares.
use std::io::Read;

use reqwest::{StatusCode, Url};

use crate::GogError;

/// GOG's services answer a desktop client; some refuse an empty agent.
pub(crate) const USER_AGENT: &str = "GOGGalaxyClient/2.0.80.21 (Windows_NT 10.0.19045) Obelisk";

pub(crate) fn client() -> Result<reqwest::Client, GogError> {
    // Redirects never carry the account's bearer token elsewhere.
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|_| GogError::Network)
}

pub(crate) async fn bounded_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, GogError> {
    serde_json::from_slice(&bounded_bytes(response, limit).await?).map_err(|_| GogError::Protocol)
}

pub(crate) async fn bounded_bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, GogError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(GogError::Protocol);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| GogError::Network)? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(GogError::Protocol);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A content-system document: zlib-compressed JSON, or plain JSON for the
/// few GOG serves uncompressed.
pub(crate) fn inflate_json<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    limit: u64,
) -> Result<T, GogError> {
    let mut inflated = Vec::new();
    let decoded = flate2::read::ZlibDecoder::new(bytes)
        .take(limit + 1)
        .read_to_end(&mut inflated);
    let json = match decoded {
        Ok(_) if inflated.len() as u64 > limit => return Err(GogError::Protocol),
        Ok(_) => &inflated[..],
        Err(_) => bytes,
    };
    serde_json::from_slice(json).map_err(|_| GogError::Protocol)
}

pub(crate) fn check_status(status: StatusCode) -> Result<(), GogError> {
    if status.is_success() {
        Ok(())
    } else if status == StatusCode::UNAUTHORIZED {
        Err(GogError::Unauthorized)
    } else {
        // Response bodies never reach the player: they may echo a token.
        Err(GogError::Service {
            message: match status {
                StatusCode::TOO_MANY_REQUESTS => "backend.gog.rate_limited".into(),
                StatusCode::FORBIDDEN => "backend.gog.forbidden".into(),
                _ => serde_json::json!({
                    "key": "backend.gog.http",
                    "args": [status.as_u16().to_string()]
                })
                .to_string(),
            },
        })
    }
}

/// Only HTTPS locations without embedded credentials reach the host.
pub(crate) fn safe_https_url(value: &str) -> Option<String> {
    // GOG spells many image locations without their scheme.
    let value = match value.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => value.to_owned(),
    };
    Url::parse(&value)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|_| value)
}

/// The moment a GOG date names, such as `2014-10-13T00:00:00+0000`, in
/// seconds since the Unix epoch; a bare date is its midnight in UTC.
pub(crate) fn unix_seconds(text: &str) -> Option<u64> {
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if text.get(4..5) != Some("-") || text.get(7..8) != Some("-") {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (mut seconds, rest) = match text.get(10..11) {
        None => (0, ""),
        Some("T" | " ") if text.get(13..14) == Some(":") && text.get(16..17) == Some(":") => (
            number(11..13)? * 3600 + number(14..16)? * 60 + number(17..19)?,
            &text[19..],
        ),
        _ => return None,
    };
    // A fraction of a second, then the offset from UTC.
    let rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    match rest {
        "" | "Z" => {}
        offset => {
            let sign = match offset.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let digits = offset[1..].replace(':', "");
            if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let minutes: i64 =
                digits[..2].parse::<i64>().ok()? * 60 + digits[2..].parse::<i64>().ok()?;
            seconds -= sign * minutes * 60;
        }
    }
    // Days since the epoch of the proleptic Gregorian date, after Howard
    // Hinnant's `days_from_civil`.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    u64::try_from(days * 86_400 + seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn content_documents_inflate_or_read_as_plain_json() {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(br#"{"a":1}"#).unwrap();
        let compressed = encoder.finish().unwrap();
        let value: serde_json::Value = inflate_json(&compressed, 1024).unwrap();
        assert_eq!(value["a"], 1);
        let value: serde_json::Value = inflate_json(br#"{"a":2}"#, 1024).unwrap();
        assert_eq!(value["a"], 2);
        assert!(matches!(
            inflate_json::<serde_json::Value>(&compressed, 3),
            Err(GogError::Protocol)
        ));
    }

    #[test]
    fn gog_dates_read_as_unix_seconds() {
        assert_eq!(
            unix_seconds("2014-10-13T00:00:00+0000"),
            Some(1_413_158_400)
        );
        assert_eq!(
            unix_seconds("2011-05-09T23:00:00+0200"),
            Some(1_304_974_800)
        );
        assert_eq!(
            unix_seconds("2024-11-05T10:31:15+00:00"),
            Some(1_730_802_675)
        );
        assert_eq!(
            unix_seconds("2020-02-29T12:00:00.250Z"),
            Some(1_582_977_600)
        );
        assert_eq!(unix_seconds("1970-01-01"), Some(0));
        for text in [
            "",
            "2014-13-01",
            "2014/10/13",
            "2014-10-13T00:00",
            "2014-10-13T00:00:00+00",
            "x",
        ] {
            assert_eq!(unix_seconds(text), None, "{text}");
        }
    }

    #[test]
    fn images_gain_their_scheme_and_other_locations_are_refused() {
        assert_eq!(
            safe_https_url("//images.gog-statics.com/a.jpg").as_deref(),
            Some("https://images.gog-statics.com/a.jpg")
        );
        assert_eq!(safe_https_url("http://images.gog.com/a.jpg"), None);
        assert_eq!(safe_https_url("https://user:pw@images.gog.com/a.jpg"), None);
        assert_eq!(safe_https_url("not a url"), None);
    }
}
