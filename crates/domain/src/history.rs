//! Page history: which versions of a page the hourly cleanup keeps.
//!
//! A page keeps every version for a while, then one a day, then one a week, and never more
//! than a number of them:
//!
//! - every version younger than [`Retention::keep_all`];
//! - of those older, but younger than [`Retention::keep_daily`], the latest of each day;
//! - of those older still, the latest of each week, Monday to Sunday;
//! - of those kept, the latest [`Retention::max_versions`].
//!
//! Days and weeks are UTC's. The latest version is always kept, and counts as its day's or
//! week's.

use std::collections::HashSet;
use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

/// How long versions are kept, from the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub keep_all: Duration,
    pub keep_daily: Duration,
    pub max_versions: usize,
}

/// A version as the cleanup sees it: its id and when it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dated {
    pub id: Uuid,
    pub at: OffsetDateTime,
}

/// Which period of the retention a version falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Bucket {
    /// Kept, whatever else there is.
    All(Uuid),
    /// One a day: the day, counted from the epoch.
    Day(i64),
    /// One a week: the Monday it starts on, counted from the epoch.
    Week(i64),
}

/// The ids of the versions of one page to delete, of `versions` in any order, as of `now`.
pub fn to_delete(versions: &[Dated], now: OffsetDateTime, retention: Retention) -> Vec<Uuid> {
    let mut newest_first = versions.to_vec();
    newest_first.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    let mut seen: HashSet<Bucket> = HashSet::new();
    let mut kept = 0usize;
    let mut doomed = Vec::new();
    for (index, version) in newest_first.iter().enumerate() {
        let age = (now - version.at).unsigned_abs();
        let day = version.at.unix_timestamp().div_euclid(86_400);
        let bucket = if age < retention.keep_all {
            Bucket::All(version.id)
        } else if age < retention.keep_daily {
            Bucket::Day(day)
        } else {
            // The epoch was a Thursday: day 4 is the first Monday.
            Bucket::Week((day - 4).div_euclid(7))
        };
        let first_of_its_bucket = seen.insert(bucket);
        if index == 0 || (first_of_its_bucket && kept < retention.max_versions) {
            kept += 1;
        } else {
            doomed.push(version.id);
        }
    }
    doomed
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);
    const DAY: Duration = Duration::from_secs(86_400);

    const RETENTION: Retention = Retention {
        keep_all: Duration::from_secs(7 * 86_400),
        keep_daily: Duration::from_secs(30 * 86_400),
        max_versions: 100,
    };

    /// Thursday 2026-11-26, noon.
    const NOW: OffsetDateTime = datetime!(2026-11-26 12:00 UTC);

    fn version(n: u128, at: OffsetDateTime) -> Dated {
        Dated {
            id: Uuid::from_u128(n),
            at,
        }
    }

    fn deleted(versions: &[Dated], retention: Retention) -> Vec<u128> {
        let mut ids: Vec<u128> = to_delete(versions, NOW, retention)
            .into_iter()
            .map(|id| id.as_u128())
            .collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn recent_versions_are_all_kept() {
        let versions: Vec<Dated> = (0..50)
            .map(|n| version(n, NOW - HOUR * u32::try_from(n).expect("small")))
            .collect();
        assert!(deleted(&versions, RETENTION).is_empty());
    }

    #[test]
    fn older_ones_thin_to_the_latest_of_each_day() {
        // Three versions on each of two days ten and eleven days ago, at 9, 10 and 11.
        let mut versions = Vec::new();
        for (day, base) in [(10u32, 10u128), (11, 20)] {
            for hour in 0..3u32 {
                let at = (NOW - DAY * day).replace_hour(9 + u8::try_from(hour).expect("small"));
                versions.push(version(base + u128::from(hour), at.expect("an hour")));
            }
        }
        assert_eq!(deleted(&versions, RETENTION), [10, 11, 20, 21]);
    }

    #[test]
    fn older_still_thin_to_the_latest_of_each_week() {
        // Monday 2026-09-28 to Sunday 2026-10-04 is one week; Monday 2026-10-05 starts the
        // next.
        let versions = [
            version(1, datetime!(2026-09-28 08:00 UTC)),
            version(2, datetime!(2026-10-01 08:00 UTC)),
            version(3, datetime!(2026-10-04 23:00 UTC)),
            version(4, datetime!(2026-10-05 01:00 UTC)),
            version(5, datetime!(2026-10-06 01:00 UTC)),
        ];
        assert_eq!(deleted(&versions, RETENTION), [1, 2, 4]);
    }

    #[test]
    fn the_cap_keeps_the_latest() {
        let versions: Vec<Dated> = (0..10)
            .map(|n| version(n, NOW - HOUR * u32::try_from(n).expect("small")))
            .collect();
        let capped = Retention {
            max_versions: 4,
            ..RETENTION
        };
        assert_eq!(deleted(&versions, capped), [4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn the_latest_version_stays_however_old() {
        let old = [version(1, NOW - DAY * 400), version(2, NOW - DAY * 401)];
        let tight = Retention {
            keep_all: HOUR,
            keep_daily: HOUR,
            max_versions: 0,
        };
        assert_eq!(deleted(&old, tight), [2]);
        assert!(deleted(&old[..1], tight).is_empty());
    }

    #[test]
    fn the_order_given_does_not_matter() {
        let mut versions: Vec<Dated> = (0..40)
            .map(|n| version(n, NOW - DAY * u32::try_from(n).expect("small")))
            .collect();
        let forward = deleted(&versions, RETENTION);
        versions.reverse();
        assert_eq!(deleted(&versions, RETENTION), forward);
        // Seven days kept whole, then one a day to thirty, then one a week.
        assert!(!forward.contains(&6) && !forward.contains(&29));
        assert!(forward.len() > 3);
    }
}
