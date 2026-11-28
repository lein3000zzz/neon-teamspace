//! Durations written the way people type them: `30s`, `1h 30m`, `7 days`.
//!
//! Settings and messages need only this small grammar, so it lives here rather than in a
//! dependency. Precision is one millisecond.

use std::time::Duration;

/// Each unit with its length in milliseconds and the names it is written as. The first
/// name is the one [`format`](fn@format) writes.
const UNITS: [(u64, &[&str]); 6] = [
    (7 * DAY, &["w", "week", "weeks"]),
    (DAY, &["d", "day", "days"]),
    (HOUR, &["h", "hr", "hrs", "hour", "hours"]),
    (MINUTE, &["m", "min", "mins", "minute", "minutes"]),
    (SECOND, &["s", "sec", "secs", "second", "seconds"]),
    (1, &["ms", "msec", "millisecond", "milliseconds"]),
];

const SECOND: u64 = 1000;
const MINUTE: u64 = 60 * SECOND;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// Parses whole numbers, each followed by a unit, optionally separated by spaces:
/// `90s`, `1h 30m`, `2 days`. `None` when the text is not such a duration or is too long
/// to represent.
pub fn parse(text: &str) -> Option<Duration> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return None;
    }

    let mut millis: u64 = 0;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let amount: u64 = rest[..digits].parse().ok()?;
        rest = rest[digits..].trim_start();

        let letters = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let unit = rest[..letters].to_ascii_lowercase();
        let (length, _) = UNITS
            .iter()
            .find(|(_, names)| names.contains(&unit.as_str()))?;
        rest = rest[letters..].trim_start();

        millis = millis.checked_add(amount.checked_mul(*length)?)?;
    }

    Some(Duration::from_millis(millis))
}

/// Writes a duration with its largest days-or-smaller units first, skipping zero parts:
/// `1h 30m`, `7d`, `250ms`, `0s`. [`parse`] reads it back. Below a millisecond is dropped.
pub fn format(duration: Duration) -> String {
    let mut millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    if millis == 0 {
        return "0s".to_owned();
    }

    let mut parts = Vec::new();
    // Weeks are skipped: 30 days reads better as `30d` than as `4w 2d`.
    for (length, names) in &UNITS[1..] {
        let amount = millis / length;
        if amount > 0 {
            parts.push(format!("{amount}{}", names[0]));
            millis %= length;
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn parses_short_and_long_unit_names() {
        let cases = [
            ("30s", secs(30)),
            ("1h 30m", secs(5400)),
            ("1h30m", secs(5400)),
            ("2 days", secs(2 * 86_400)),
            ("1w", secs(7 * 86_400)),
            ("  5 Minutes ", secs(300)),
            ("250ms", Duration::from_millis(250)),
            ("1d 2h 3m 4s 5ms", Duration::from_millis(93_784_005)),
            ("0s", Duration::ZERO),
        ];

        for (text, want) in cases {
            assert_eq!(parse(text), Some(want), "{text:?}");
        }
    }

    #[test]
    fn refuses_anything_else() {
        for text in [
            "",
            "  ",
            "30",
            "s",
            "1.5h",
            "-1s",
            "1h,30m",
            "1 fortnight",
            "soon",
            "1h 30",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
        assert_eq!(parse(&format!("{}w", u64::MAX / 1000)), None);
    }

    #[test]
    fn formats_largest_units_first() {
        assert_eq!(format(secs(5400)), "1h 30m");
        assert_eq!(format(secs(7 * 86_400)), "7d");
        assert_eq!(format(secs(30 * 86_400)), "30d");
        assert_eq!(format(Duration::from_millis(100)), "100ms");
        assert_eq!(format(Duration::from_millis(93_784_005)), "1d 2h 3m 4s 5ms");
        assert_eq!(format(Duration::from_micros(10)), "0s");
        assert_eq!(format(Duration::ZERO), "0s");
    }

    #[test]
    fn formatted_durations_parse_back() {
        for duration in [
            secs(1),
            secs(5400),
            secs(86_400 + 1),
            Duration::from_millis(1001),
        ] {
            assert_eq!(parse(&format(duration)), Some(duration));
        }
    }
}
