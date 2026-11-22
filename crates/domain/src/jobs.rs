//! Rules of the background job queue.

use std::time::Duration;

/// Exponential backoff with jitter, for retrying failed jobs.
///
/// The delay doubles with each failed attempt up to `max`. Half of it is fixed and half is
/// spread by the jitter, so jobs that failed together, say during a mail server outage,
/// do not all retry at the same moment, and no retry comes sooner than half the delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
}

impl Backoff {
    /// The delay before the next try after `attempt` failed runs, counting from 1.
    /// `jitter` is a random number in `[0, 1)`; values outside are clamped.
    pub fn delay(&self, attempt: u32, jitter: f64) -> Duration {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
        let full = self.base.saturating_mul(factor).min(self.max);
        let jitter = if jitter.is_nan() {
            0.0
        } else {
            jitter.clamp(0.0, 1.0)
        };
        let half = full / 2;

        half + half.mul_f64(jitter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKOFF: Backoff = Backoff {
        base: Duration::from_secs(10),
        max: Duration::from_secs(300),
    };

    #[test]
    fn delay_doubles_per_attempt() {
        let delays: Vec<u64> = (1..=5)
            .map(|attempt| BACKOFF.delay(attempt, 1.0).as_secs())
            .collect();

        assert_eq!(delays, [10, 20, 40, 80, 160]);
    }

    #[test]
    fn delay_stops_at_the_cap() {
        assert_eq!(BACKOFF.delay(6, 1.0), Duration::from_secs(300));
        assert_eq!(BACKOFF.delay(u32::MAX, 1.0), Duration::from_secs(300));
    }

    #[test]
    fn jitter_spreads_the_upper_half() {
        assert_eq!(BACKOFF.delay(2, 0.0), Duration::from_secs(10));
        assert_eq!(BACKOFF.delay(2, 0.5), Duration::from_secs(15));
        assert_eq!(BACKOFF.delay(2, 7.0), Duration::from_secs(20));
        assert_eq!(BACKOFF.delay(2, f64::NAN), Duration::from_secs(10));
    }
}
