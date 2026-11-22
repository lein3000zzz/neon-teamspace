use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Mutex;

use async_trait::async_trait;
use governor::clock::{Clock, DefaultClock, Reference as _};
use governor::middleware::NoOpMiddleware;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota as GovernorQuota, RateLimiter as Governor};

use super::{Bucket, Limited, Quota, RateLimiter};

type Keyed<C> =
    Governor<String, DashMapStateStore<String>, C, NoOpMiddleware<<C as Clock>::Instant>>;

/// A token bucket per key in this process's memory, with [`governor`].
///
/// Buckets that are full again are forgotten now and then, so memory holds the keys seen
/// lately rather than every key ever seen.
pub struct MemoryLimiter<C: Clock = DefaultClock> {
    buckets: HashMap<Bucket, Keyed<C>>,
    clock: C,
    /// When each bucket last forgot its idle keys.
    swept: Mutex<HashMap<Bucket, C::Instant>>,
}

impl MemoryLimiter {
    pub fn new() -> Self {
        Self::with_clock(DefaultClock::default())
    }
}

impl Default for MemoryLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Clock + Clone> MemoryLimiter<C> {
    /// A limiter on `clock`, which tests move by hand.
    pub fn with_clock(clock: C) -> Self {
        let buckets = Bucket::ALL
            .into_iter()
            .map(|bucket| {
                let limiter =
                    Governor::dashmap_with_clock(governor_quota(bucket.quota()), clock.clone());
                (bucket, limiter)
            })
            .collect();
        Self {
            buckets,
            swept: Mutex::new(HashMap::new()),
            clock,
        }
    }

    /// The keys held across every bucket.
    pub fn len(&self) -> usize {
        self.buckets.values().map(Keyed::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn take(&self, bucket: Bucket, key: &str) -> Result<(), Limited> {
        let Some(limiter) = self.buckets.get(&bucket) else {
            return Ok(());
        };
        self.sweep(bucket, limiter);
        limiter
            .check_key(&format!("{}{key}", bucket.prefix()))
            .map_err(|not_until| Limited {
                retry_after: not_until.wait_time_from(self.clock.now()),
            })
    }

    /// Forgets the bucket's full keys, at most once per time a bucket takes to fill.
    fn sweep(&self, bucket: Bucket, limiter: &Keyed<C>) {
        let quota = bucket.quota();
        let refill = quota.every.saturating_mul(quota.burst);
        let now = self.clock.now();
        let mut swept = self
            .swept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let last = *swept.entry(bucket).or_insert(now);
        if now.duration_since(last) >= refill.into() {
            limiter.retain_recent();
            limiter.shrink_to_fit();
            swept.insert(bucket, now);
        }
    }
}

fn governor_quota(quota: Quota) -> GovernorQuota {
    let burst = NonZeroU32::new(quota.burst).unwrap_or(NonZeroU32::MIN);
    GovernorQuota::with_period(quota.every)
        .unwrap_or_else(|| GovernorQuota::per_second(burst))
        .allow_burst(burst)
}

#[async_trait]
impl<C> RateLimiter for MemoryLimiter<C>
where
    C: Clock + Clone + Send + Sync + 'static,
    C::Instant: Send + Sync,
{
    async fn check(&self, bucket: Bucket, key: &str) -> Result<(), Limited> {
        self.take(bucket, key)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use governor::clock::FakeRelativeClock;

    use super::*;

    fn limiter() -> (MemoryLimiter<FakeRelativeClock>, FakeRelativeClock) {
        let clock = FakeRelativeClock::default();
        (MemoryLimiter::with_clock(clock.clone()), clock)
    }

    #[tokio::test]
    async fn a_key_gets_its_burst_then_one_per_period() {
        let (limiter, clock) = limiter();

        for _ in 0..10 {
            assert_eq!(limiter.check(Bucket::LoginIp, "1.2.3.4").await, Ok(()));
        }
        assert_eq!(
            limiter.check(Bucket::LoginIp, "1.2.3.4").await,
            Err(Limited {
                retry_after: Duration::from_secs(30)
            })
        );

        clock.advance(Duration::from_secs(20));
        assert_eq!(
            limiter.check(Bucket::LoginIp, "1.2.3.4").await,
            Err(Limited {
                retry_after: Duration::from_secs(10)
            })
        );
        clock.advance(Duration::from_secs(10));
        assert_eq!(limiter.check(Bucket::LoginIp, "1.2.3.4").await, Ok(()));
        assert!(limiter.check(Bucket::LoginIp, "1.2.3.4").await.is_err());
    }

    #[tokio::test]
    async fn keys_and_buckets_are_counted_apart() {
        let (limiter, _) = limiter();
        for _ in 0..10 {
            limiter
                .check(Bucket::Register, "1.2.3.4")
                .await
                .expect("allowed");
        }

        assert!(limiter.check(Bucket::Register, "1.2.3.4").await.is_err());
        assert_eq!(limiter.check(Bucket::Register, "5.6.7.8").await, Ok(()));
        assert_eq!(limiter.check(Bucket::Reset, "1.2.3.4").await, Ok(()));
    }

    #[tokio::test]
    async fn the_cross_ip_bucket_allows_fifty() {
        let (limiter, _) = limiter();

        for _ in 0..50 {
            limiter
                .check(Bucket::LoginEmail, "ada@example.com")
                .await
                .expect("allowed");
        }
        let limited = limiter
            .check(Bucket::LoginEmail, "ada@example.com")
            .await
            .expect_err("limited");
        assert_eq!(limited.retry_after, Duration::from_secs(6));
    }

    #[tokio::test]
    async fn full_buckets_are_forgotten() {
        let (limiter, clock) = limiter();
        for ip in ["1.1.1.1", "2.2.2.2", "3.3.3.3"] {
            limiter.check(Bucket::Join, ip).await.expect("allowed");
        }
        assert_eq!(limiter.len(), 3);

        clock.advance(Duration::from_secs(10 * 30));
        limiter
            .check(Bucket::Join, "4.4.4.4")
            .await
            .expect("allowed");

        assert_eq!(limiter.len(), 1);
    }
}
