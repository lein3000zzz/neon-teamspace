//! The time use cases read, which tests set by hand.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use time::OffsetDateTime;

/// Where use cases read the time from: the system's, or one tests move.
#[derive(Debug, Clone, Default)]
pub struct Clock {
    manual: Option<Arc<Mutex<OffsetDateTime>>>,
}

impl Clock {
    pub fn system() -> Self {
        Self::default()
    }

    /// Now, in UTC, to the microsecond the database keeps, so a time a use case returns
    /// equals the one read back later.
    pub fn now(&self) -> OffsetDateTime {
        let now = match &self.manual {
            Some(time) => *time.lock().unwrap_or_else(PoisonError::into_inner),
            None => OffsetDateTime::now_utc(),
        };
        truncate(now)
    }
}

/// A time that only moves when told to, for tests.
#[derive(Debug, Clone)]
pub struct ManualTime {
    time: Arc<Mutex<OffsetDateTime>>,
}

impl ManualTime {
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            time: Arc::new(Mutex::new(start)),
        }
    }

    /// A clock that reads this time.
    pub fn clock(&self) -> Clock {
        Clock {
            manual: Some(Arc::clone(&self.time)),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.time.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }
}

fn truncate(time: OffsetDateTime) -> OffsetDateTime {
    let micros = time.nanosecond() / 1000 * 1000;
    time.replace_nanosecond(micros).unwrap_or(time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_time_moves_only_when_told() {
        let start = OffsetDateTime::UNIX_EPOCH + Duration::from_nanos(1_500);
        let time = ManualTime::new(start);
        let clock = time.clock();

        assert_eq!(
            clock.now(),
            OffsetDateTime::UNIX_EPOCH + Duration::from_micros(1)
        );
        time.advance(Duration::from_secs(60));
        assert_eq!(
            clock.now(),
            OffsetDateTime::UNIX_EPOCH + Duration::from_secs(60) + Duration::from_micros(1)
        );
    }

    #[test]
    fn the_system_clock_has_microsecond_precision() {
        assert_eq!(Clock::system().now().nanosecond() % 1000, 0);
    }
}
