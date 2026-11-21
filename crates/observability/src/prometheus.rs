//! The Prometheus recorder behind the `metrics` facade.

use std::time::Duration;

use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder, PrometheusRecorder};

use crate::red::{self, Buckets};

/// How often histogram samples are drained when nothing scrapes `/metrics`, so memory stays
/// bounded between scrapes.
pub(crate) const UPKEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Buckets of histograms that no subsystem claims. Without a default the exporter renders
/// them as summaries, which cannot be aggregated across instances.
const FALLBACK_BUCKETS: Buckets = Buckets::Medium;

/// A recorder whose duration histograms use each subsystem's bucket preset.
pub(crate) fn recorder(subsystems: &[(&str, Buckets)]) -> Result<PrometheusRecorder, BuildError> {
    let mut builder = PrometheusBuilder::new().set_buckets(FALLBACK_BUCKETS.values())?;
    for (subsystem, buckets) in subsystems {
        builder = builder.set_buckets_for_metric(
            Matcher::Full(red::duration_metric(subsystem)),
            buckets.values(),
        )?;
    }
    Ok(builder.build_recorder())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::red::Red;

    fn render(subsystems: &[(&str, Buckets)], record: impl FnOnce()) -> String {
        let recorder = recorder(subsystems).unwrap();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, record);
        handle.render()
    }

    #[test]
    fn subsystems_get_their_bucket_preset() {
        let output = render(&[("jobs", Buckets::XLong)], || {
            Red::new("jobs").start("send_email").ok();
        });

        assert!(
            output.contains(
                r#"neon_jobs_operation_duration_seconds_bucket{operation="send_email",status="ok",le="300"} 1"#
            ),
            "{output}"
        );
        assert!(
            output.contains(r#"neon_jobs_operations_total{operation="send_email",status="ok"} 1"#)
        );
        assert!(output.contains(r#"neon_jobs_operations_in_flight{operation="send_email"} 0"#));
        assert!(output.contains("# HELP neon_jobs_operations_total"));
        assert!(output.contains("# TYPE neon_jobs_operation_duration_seconds histogram"));
    }

    #[test]
    fn unlisted_histograms_are_histograms_not_summaries() {
        let output = render(&[], || Red::new("sync").start("apply").ok());

        assert!(
            output.contains("# TYPE neon_sync_operation_duration_seconds histogram"),
            "{output}"
        );
        assert!(output.contains(r#"le="5""#));
    }
}
