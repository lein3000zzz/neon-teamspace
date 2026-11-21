//! RED metrics (rate, errors, duration) for the operations of one subsystem.
//!
//! Each subsystem gets three metrics, labelled by `operation` and, once the outcome is known,
//! `status` (`ok` or `error`):
//!
//! - `neon_<subsystem>_operations_total`
//! - `neon_<subsystem>_operation_duration_seconds`
//! - `neon_<subsystem>_operations_in_flight` (no `status`)
//!
//! [`Red::start`] returns a guard, so the start and the outcome cannot drift apart: a guard
//! dropped without [`RedGuard::finish`], by an early return, `?` or a cancelled future,
//! counts as an error.
//!
//! Operation names become label values. Keep them to a fixed set such as route templates or
//! use case names, never ids, or every value becomes a new time series.

use std::sync::Arc;
use std::time::Instant;

use metrics::{SharedString, Unit};

/// Prefix of every metric name.
pub const NAMESPACE: &str = "neon";

/// `status` label value of a successful operation.
pub const STATUS_OK: &str = "ok";
/// `status` label value of a failed or abandoned operation.
pub const STATUS_ERROR: &str = "error";

const LABEL_OPERATION: &str = "operation";
const LABEL_STATUS: &str = "status";

/// Histogram bucket presets, in seconds. Pick the one whose range covers the slowest normal
/// operation of the subsystem, so the tail still lands in a bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buckets {
    /// 0.1 ms to 250 ms: in-memory work and cache hits.
    Micro,
    /// 1 ms to 1 s: single queries.
    Short,
    /// 5 ms to 5 s: HTTP requests and use cases.
    Medium,
    /// 5 ms to 10 s: requests that call other services.
    MediumLong,
    /// 100 ms to 2 min: background jobs.
    Long,
    /// 100 ms to 5 min: exports and batch work.
    XLong,
}

impl Buckets {
    /// The upper bounds of the buckets, in seconds.
    #[must_use]
    pub const fn values(self) -> &'static [f64] {
        match self {
            Self::Micro => &[0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25],
            Self::Short => &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
            Self::Medium => &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
            Self::MediumLong => &[
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ],
            Self::Long => &[0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0],
            Self::XLong => &[0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0],
        }
    }
}

/// Name of the operation counter of `subsystem`.
#[must_use]
pub fn total_metric(subsystem: &str) -> String {
    format!("{NAMESPACE}_{subsystem}_operations_total")
}

/// Name of the duration histogram of `subsystem`.
#[must_use]
pub fn duration_metric(subsystem: &str) -> String {
    format!("{NAMESPACE}_{subsystem}_operation_duration_seconds")
}

/// Name of the in-flight gauge of `subsystem`.
#[must_use]
pub fn in_flight_metric(subsystem: &str) -> String {
    format!("{NAMESPACE}_{subsystem}_operations_in_flight")
}

/// RED metrics for one subsystem. Cheap to clone.
///
/// Creating it sends the help texts to the recorder installed at that moment, so create it
/// after [`init`](crate::init), which also describes every subsystem it is given.
#[derive(Debug, Clone)]
pub struct Red {
    names: Arc<Names>,
}

#[derive(Debug)]
struct Names {
    total: SharedString,
    duration: SharedString,
    in_flight: SharedString,
}

impl Red {
    /// RED metrics for `subsystem`, such as `http`, `jobs` or `mcp`. The name must be a valid
    /// Prometheus name fragment: a lowercase letter, then lowercase letters, digits and
    /// underscores. Debug builds panic on anything else.
    #[must_use]
    pub fn new(subsystem: &str) -> Self {
        debug_assert!(
            is_valid_subsystem(subsystem),
            "invalid RED subsystem {subsystem:?}"
        );
        describe(subsystem);
        Self {
            names: Arc::new(Names {
                total: shared(total_metric(subsystem)),
                duration: shared(duration_metric(subsystem)),
                in_flight: shared(in_flight_metric(subsystem)),
            }),
        }
    }

    /// Starts timing `operation` and counts it as in flight until the guard finishes or drops.
    pub fn start(&self, operation: impl Into<SharedString>) -> RedGuard {
        let operation = operation.into();
        metrics::gauge!(
            self.names.in_flight.clone(),
            LABEL_OPERATION => operation.clone()
        )
        .increment(1.0);
        RedGuard {
            names: Arc::clone(&self.names),
            operation,
            started: Instant::now(),
            finished: false,
        }
    }
}

/// Whether `subsystem` makes valid metric names: a lowercase letter, then lowercase letters,
/// digits and underscores.
pub(crate) fn is_valid_subsystem(subsystem: &str) -> bool {
    let mut chars = subsystem.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Sends the help texts of a subsystem's metrics to the current recorder.
pub(crate) fn describe(subsystem: &str) {
    metrics::describe_counter!(
        total_metric(subsystem),
        Unit::Count,
        format!("Total {subsystem} operations by operation and status.")
    );
    metrics::describe_histogram!(
        duration_metric(subsystem),
        Unit::Seconds,
        format!("Duration of {subsystem} operations in seconds, by operation and status.")
    );
    metrics::describe_gauge!(
        in_flight_metric(subsystem),
        Unit::Count,
        format!("{subsystem} operations currently running, by operation.")
    );
}

fn shared(name: String) -> SharedString {
    SharedString::from(Arc::<str>::from(name))
}

/// One running operation. Records its outcome exactly once.
#[must_use = "dropping the guard right away records the operation as an error"]
#[derive(Debug)]
pub struct RedGuard {
    names: Arc<Names>,
    operation: SharedString,
    started: Instant,
    finished: bool,
}

impl RedGuard {
    /// Records the operation as `ok` or `error`.
    pub fn finish(mut self, ok: bool) {
        self.record(ok);
    }

    /// Records the operation as `ok`.
    pub fn ok(self) {
        self.finish(true);
    }

    /// Records the operation as `error`.
    pub fn err(self) {
        self.finish(false);
    }

    fn record(&mut self, ok: bool) {
        if self.finished {
            return;
        }
        self.finished = true;

        let status = if ok { STATUS_OK } else { STATUS_ERROR };
        let elapsed = self.started.elapsed().as_secs_f64();
        metrics::gauge!(
            self.names.in_flight.clone(),
            LABEL_OPERATION => self.operation.clone()
        )
        .decrement(1.0);
        metrics::histogram!(
            self.names.duration.clone(),
            LABEL_OPERATION => self.operation.clone(),
            LABEL_STATUS => status
        )
        .record(elapsed);
        metrics::counter!(
            self.names.total.clone(),
            LABEL_OPERATION => self.operation.clone(),
            LABEL_STATUS => status
        )
        .increment(1);
    }
}

impl Drop for RedGuard {
    fn drop(&mut self) {
        self.record(false);
    }
}

#[cfg(test)]
mod tests {
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    use super::*;

    struct Recorded {
        kind: MetricKind,
        name: String,
        labels: Vec<(String, String)>,
        value: DebugValue,
    }

    struct Metrics(Vec<Recorded>);

    impl Metrics {
        fn take(snapshotter: &Snapshotter) -> Self {
            Self(
                snapshotter
                    .snapshot()
                    .into_vec()
                    .into_iter()
                    .map(|(key, _, _, value)| {
                        let (kind, key) = key.into_parts();
                        let labels = key
                            .labels()
                            .map(|label| (label.key().to_owned(), label.value().to_owned()))
                            .collect();
                        Recorded {
                            kind,
                            name: key.name().to_owned(),
                            labels,
                            value,
                        }
                    })
                    .collect(),
            )
        }

        fn get(
            &self,
            kind: MetricKind,
            name: &str,
            labels: &[(&str, &str)],
        ) -> Option<&DebugValue> {
            self.0.iter().find_map(|recorded| {
                let same_labels = recorded.labels.len() == labels.len()
                    && labels.iter().all(|(key, value)| {
                        recorded.labels.iter().any(|(k, v)| k == key && v == value)
                    });
                (recorded.kind == kind && recorded.name == name && same_labels)
                    .then_some(&recorded.value)
            })
        }

        fn count(&self, operation: &str, status: &str) -> Option<u64> {
            match self.get(
                MetricKind::Counter,
                "neon_test_operations_total",
                &[("operation", operation), ("status", status)],
            )? {
                DebugValue::Counter(count) => Some(*count),
                _ => None,
            }
        }

        fn durations(&self, operation: &str, status: &str) -> usize {
            match self.get(
                MetricKind::Histogram,
                "neon_test_operation_duration_seconds",
                &[("operation", operation), ("status", status)],
            ) {
                Some(DebugValue::Histogram(values)) => values.len(),
                _ => 0,
            }
        }

        fn in_flight(&self, operation: &str) -> Option<f64> {
            match self.get(
                MetricKind::Gauge,
                "neon_test_operations_in_flight",
                &[("operation", operation)],
            )? {
                DebugValue::Gauge(value) => Some(value.into_inner()),
                _ => None,
            }
        }
    }

    fn record(f: impl FnOnce(&Red)) -> Metrics {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || f(&Red::new("test")));
        Metrics::take(&snapshotter)
    }

    #[test]
    fn finish_ok_records_total_duration_and_releases_in_flight() {
        let metrics = record(|red| red.start("load").finish(true));

        assert_eq!(metrics.count("load", "ok"), Some(1));
        assert_eq!(metrics.count("load", "error"), None);
        assert_eq!(metrics.durations("load", "ok"), 1);
        assert_eq!(metrics.in_flight("load"), Some(0.0));
    }

    #[test]
    fn finish_err_records_status_error() {
        let metrics = record(|red| {
            red.start("load").err();
            red.start("load").ok();
            red.start("load").finish(false);
        });

        assert_eq!(metrics.count("load", "error"), Some(2));
        assert_eq!(metrics.count("load", "ok"), Some(1));
        assert_eq!(metrics.durations("load", "error"), 2);
        assert_eq!(metrics.in_flight("load"), Some(0.0));
    }

    #[test]
    fn dropping_an_unfinished_guard_records_an_error() {
        let metrics = record(|red| {
            let guard = red.start("save");
            drop(guard);
        });

        assert_eq!(metrics.count("save", "error"), Some(1));
        assert_eq!(metrics.count("save", "ok"), None);
        assert_eq!(metrics.in_flight("save"), Some(0.0));
    }

    #[test]
    fn in_flight_counts_running_operations() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let red = Red::new("test");
            let _first = red.start("sync");
            let _second = red.start("sync");
            assert_eq!(Metrics::take(&snapshotter).in_flight("sync"), Some(2.0));
        });
    }

    #[test]
    fn describes_metrics_with_units() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || Red::new("test").start("load").ok());

        let described: Vec<_> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, unit, help, _)| (key.key().name().to_owned(), unit, help.is_some()))
            .collect();
        assert!(described.contains(&(
            "neon_test_operation_duration_seconds".to_owned(),
            Some(Unit::Seconds),
            true
        )));
        assert!(described.contains(&(
            "neon_test_operations_total".to_owned(),
            Some(Unit::Count),
            true
        )));
    }

    #[test]
    fn subsystem_names_must_make_valid_metric_names() {
        for valid in ["http", "jobs", "mcp_tools", "sync2"] {
            assert!(is_valid_subsystem(valid), "{valid}");
        }
        for invalid in ["", "HTTP", "2fa", "web-socket", "_jobs", "jobs.queue"] {
            assert!(!is_valid_subsystem(invalid), "{invalid}");
        }
    }

    #[test]
    fn bucket_presets_match_redkit() {
        assert_eq!(Buckets::Micro.values().len(), 9);
        assert_eq!(Buckets::Medium.values()[0], 0.005);
        assert_eq!(Buckets::MediumLong.values().last(), Some(&10.0));
        assert_eq!(Buckets::Long.values().last(), Some(&120.0));
        assert_eq!(Buckets::XLong.values().last(), Some(&300.0));
        for preset in [
            Buckets::Micro,
            Buckets::Short,
            Buckets::Medium,
            Buckets::MediumLong,
            Buckets::Long,
            Buckets::XLong,
        ] {
            assert!(preset.values().is_sorted(), "{preset:?} is not sorted");
        }
    }
}
