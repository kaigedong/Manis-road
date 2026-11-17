//! Tolerant timestamp parsing for wire models.
//!
//! The Motu backend is not consistent about time fields: the same key can come
//! back as epoch milliseconds, a numeric string, or a `yyyy-MM-dd` date (for
//! example the VIP expiry `memEndTime`). Java reads these into `Long` and the
//! server usually sends numbers, but live accounts do return formatted dates.
//! These helpers accept all observed shapes and normalise them to epoch
//! milliseconds at `+08:00`, which is the timezone the API formats dates in.

use serde::{Deserialize, Deserializer, de};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTime {
    Number(i64),
    Float(f64),
    Text(String),
}

/// Deserialize an optional timestamp into epoch milliseconds.
///
/// Accepts `null`, numbers, numeric strings, and `yyyy-MM-dd`,
/// `yyyy/MM/dd`, optionally followed by `HH:mm[:ss]`.
#[allow(clippy::cast_possible_truncation)] // fractional epoch values are not expected
pub(crate) fn option_millis<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<RawTime>::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw {
        RawTime::Number(value) => Ok(Some(value)),
        RawTime::Float(value) => Ok(Some(value as i64)),
        RawTime::Text(text) => {
            let text = text.trim();
            if text.is_empty() {
                return Ok(None);
            }
            if let Ok(value) = text.parse::<i64>() {
                return Ok(Some(value));
            }
            parse_date_millis(text).map(Some).ok_or_else(|| {
                de::Error::custom(format!("unsupported timestamp format: {text}"))
            })
        }
    }
}

/// Parse `yyyy-MM-dd[ HH:mm[:ss]]` (also `/` separators and `T`) at `+08:00`.
#[must_use]
pub fn parse_date_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = match text.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (text, None),
    };
    let mut parts = date.split(['-', '/']);
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (hour, minute, second) = match time {
        None => (0, 0, 0),
        Some(time) => {
            let mut clock = time.split(':');
            let hour = clock.next()?.parse::<u32>().ok()?;
            let minute = clock.next()?.parse::<u32>().ok()?;
            let second = clock
                .next()
                .map_or(Some(0), |value| value.split('.').next().unwrap_or(value).parse().ok())?;
            if hour > 23 || minute > 59 || second > 59 {
                return None;
            }
            (hour, minute, second)
        }
    };

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60
        + i64::from(second)
        - 8 * 3_600;
    Some(seconds * 1_000)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_date_only_at_utc_plus_eight() {
        // 2026-05-09T00:00:00+08:00 == 2026-05-08T16:00:00Z.
        assert_eq!(
            parse_date_millis("2026-05-09"),
            Some(1_778_256_000_000)
        );
        assert_eq!(parse_date_millis("1970-01-01"), Some(-28_800_000));
    }

    #[test]
    fn parses_date_and_time_in_several_separators() {
        let expected = parse_date_millis("2026-05-09 08:30:15").expect("timestamp");
        assert_eq!(parse_date_millis("2026-05-09T08:30:15"), Some(expected));
        assert_eq!(parse_date_millis("2026/05/09 08:30:15"), Some(expected));
        assert_eq!(parse_date_millis("2026-05-09 08:30"), Some(expected - 15_000));
    }

    #[test]
    fn rejects_invalid_dates_and_times() {
        assert!(parse_date_millis("2026-13-01").is_none());
        assert!(parse_date_millis("2026-05-09 24:00").is_none());
        assert!(parse_date_millis("not-a-date").is_none());
        assert!(parse_date_millis("2026-05").is_none());
    }
}
