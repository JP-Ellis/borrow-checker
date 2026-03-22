//! OpenTelemetry initialisation for BorrowChecker.
//!
//! Call [`init`] once at binary startup. Returns an [`OtelGuard`] that must be
//! kept alive until program exit to ensure telemetry is flushed on shutdown.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Errors returned by [`init`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OtelError {
    /// The OTLP exporter failed to build.
    #[error("failed to build OTLP exporter: {0}")]
    ExporterBuild(String),
}

/// A guard that shuts down OpenTelemetry exporters when dropped.
///
/// Keep this alive for the duration of the program (e.g. bind to `_guard` in
/// `main`). When dropped, the underlying [`SdkTracerProvider`] is shut down,
/// flushing any pending spans to the configured exporter.
#[must_use = "dropping OtelGuard shuts down telemetry; bind to a variable"]
#[non_exhaustive]
pub struct OtelGuard {
    /// Tracer provider kept alive for shutdown on drop.
    otel_provider: Option<SdkTracerProvider>,
}

impl core::fmt::Debug for OtelGuard {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OtelGuard").finish()
    }
}

impl Drop for OtelGuard {
    #[inline]
    fn drop(&mut self) {
        if let Some(provider) = self.otel_provider.take() {
            drop(provider.shutdown());
        }
    }
}

/// Initialises the global OpenTelemetry tracer provider.
///
/// Reads the OTLP exporter endpoint from the standard
/// `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable. Returns an
/// [`OtelGuard`] that must be kept alive until program exit.
///
/// # Errors
///
/// Returns [`OtelError::ExporterBuild`] if the OTLP exporter cannot be built.
#[inline]
pub fn init() -> Result<OtelGuard, OtelError> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .map_err(|e| OtelError::ExporterBuild(e.to_string()))?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    drop(provider.tracer("bc-otel"));
    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(OtelGuard {
        otel_provider: Some(provider),
    })
}
