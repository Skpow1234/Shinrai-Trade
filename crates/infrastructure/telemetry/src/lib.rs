//! Process-wide tracing init with optional OTLP export.
//!
//! Always installs a stderr fmt layer. When `SHINRAI_OTEL_ENDPOINT` or
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is set, also exports spans over OTLP/HTTP.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Holds the tracer provider so Drop can flush/shutdown OTLP.
#[derive(Debug)]
pub struct TelemetryGuard {
    provider: Option<SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(err) = provider.shutdown() {
                eprintln!("shinrai-telemetry: OTLP shutdown error: {err}");
            }
        }
    }
}

/// Initializes tracing for `service_name`. Safe to call once per process.
///
/// # Errors
///
/// Returns an error if the OTLP exporter cannot be built when an endpoint is set.
pub fn init(
    service_name: &str,
) -> Result<TelemetryGuard, Box<dyn std::error::Error + Send + Sync>> {
    static INIT: OnceLock<()> = OnceLock::new();
    if INIT.get().is_some() {
        return Ok(TelemetryGuard { provider: None });
    }

    let filter = EnvFilter::try_from_env("SHINRAI_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_ansi(true)
        .with_filter(filter);

    let endpoint = std::env::var("SHINRAI_OTEL_ENDPOINT")
        .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let guard = if let Some(endpoint) = endpoint {
        let exporter = SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint.clone())
            .build()?;

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(
                Resource::builder_empty()
                    .with_attributes([KeyValue::new("service.name", service_name.to_owned())])
                    .build(),
            )
            .build();

        let tracer = provider.tracer(service_name.to_owned());
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()
            .map_err(|e| format!("tracing init: {e}"))?;

        eprintln!("shinrai-telemetry: OTLP export enabled → {endpoint}");
        TelemetryGuard {
            provider: Some(provider),
        }
    } else {
        tracing_subscriber::registry()
            .with(fmt_layer)
            .try_init()
            .map_err(|e| format!("tracing init: {e}"))?;
        TelemetryGuard { provider: None }
    };

    let _ = INIT.set(());
    Ok(guard)
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_without_otlp_is_idempotent() {
        let g1 = super::init("shinrai-telemetry-test").expect("first init");
        let g2 = super::init("shinrai-telemetry-test").expect("second init");
        drop(g2);
        drop(g1);
    }
}
