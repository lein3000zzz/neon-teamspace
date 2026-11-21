//! The OpenTelemetry tracer provider and the layer that turns `tracing` spans into OTel spans.
//!
//! The provider always exists, with or without an exporter: trace and span ids are generated
//! either way, so logs stay correlated and `traceparent` keeps flowing to job rows and
//! webhooks while export is off.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::registry::LookupSpan;

use crate::ServiceInfo;
use crate::config::TracingConfig;

const TRACES_PATH: &str = "/v1/traces";

/// Builds the provider with the sampler, resource and, when enabled, the OTLP batch exporter.
pub(crate) fn provider(
    config: &TracingConfig,
    service: &ServiceInfo,
) -> Result<SdkTracerProvider, opentelemetry_otlp::ExporterBuildError> {
    let builder = SdkTracerProvider::builder()
        .with_sampler(sampler(config.sample_ratio))
        .with_resource(resource(service));
    if !config.enabled {
        return Ok(builder.build());
    }

    let mut exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary);
    if let Some(endpoint) = &config.endpoint {
        exporter = exporter.with_endpoint(traces_endpoint(endpoint));
    }
    Ok(builder.with_batch_exporter(exporter.build()?).build())
}

/// New traces are sampled by ratio. Traces started upstream keep the caller's decision, so
/// one request is never half exported.
fn sampler(ratio: f64) -> Sampler {
    Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(ratio)))
}

fn resource(service: &ServiceInfo) -> Resource {
    Resource::builder()
        .with_service_name(service.name.clone())
        .with_attribute(opentelemetry::KeyValue::new(
            "service.version",
            service.version.clone(),
        ))
        .build()
}

/// A configured endpoint is used as is by the exporter, unlike the `OTEL_*` variables, so the
/// signal path is appended here to accept the same base URL as the rest of the ecosystem.
fn traces_endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with(TRACES_PATH) {
        base.to_owned()
    } else {
        format!("{base}{TRACES_PATH}")
    }
}

/// The `tracing` to OpenTelemetry bridge.
///
/// It sees spans and events at `info` and above whatever the log level is, and never the
/// exporter's own diagnostics, which would otherwise feed back into the spans they report on.
/// Events with an `error` field below `ERROR` level do not fail the span: logging an expected
/// failure at `warn` must leave the span status unset.
pub(crate) fn layer<S>(
    provider: &SdkTracerProvider,
    service: &ServiceInfo,
) -> impl Layer<S> + Send + Sync + 'static + use<S>
where
    S: Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
{
    let filter = Targets::new()
        .with_default(LevelFilter::INFO)
        .with_target("opentelemetry", LevelFilter::OFF);
    tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(service.name.clone()))
        .with_error_events_to_status(false)
        .with_filter(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_the_traces_path_once() {
        assert_eq!(
            traces_endpoint("http://otel-lgtm:4318"),
            "http://otel-lgtm:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://otel-lgtm:4318/"),
            "http://otel-lgtm:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://otel-lgtm:4318/v1/traces"),
            "http://otel-lgtm:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://gateway/otlp"),
            "http://gateway/otlp/v1/traces"
        );
    }

    #[test]
    fn builds_a_provider_without_export() {
        let service = ServiceInfo::new("neon", "0.0.0");
        let provider = provider(&TracingConfig::default(), &service).unwrap();
        provider.shutdown().unwrap();
    }

    #[test]
    fn resource_names_the_service() {
        let resource = resource(&ServiceInfo::new("neon", "1.2.3"));
        assert_eq!(
            resource.get(&"service.name".into()),
            Some(opentelemetry::Value::from("neon"))
        );
        assert_eq!(
            resource.get(&"service.version".into()),
            Some(opentelemetry::Value::from("1.2.3"))
        );
    }
}
