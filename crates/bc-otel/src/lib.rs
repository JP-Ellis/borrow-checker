//! OpenTelemetry initialisation for BorrowChecker.
//!
//! Call [`init`] once at binary startup. Returns an [`OtelGuard`] that must be
//! kept alive until program exit to ensure telemetry is flushed on shutdown.
//!
//! After calling [`init`], obtain a `tracing` layer via [`tracing_layer`] and
//! register it with your [`tracing_subscriber`] stack so that `tracing` spans
//! are forwarded to the OTLP exporter.
//!
//! The underlying [`tracing_opentelemetry`] crate is re-exported for
//! callers that need direct access to its types.

use opentelemetry_sdk::trace::SdkTracerProvider;
/// Re-export of the [`tracing_opentelemetry`] crate for callers that need
/// direct access to its layer types or builder utilities.
#[expect(
    clippy::pub_use,
    reason = "intentional crate-level re-export so callers do not need a direct tracing-opentelemetry dependency"
)]
#[expect(
    clippy::useless_attribute,
    reason = "pub_use expect on an external-crate re-export is flagged as useless by useless_attribute, but both lints apply here"
)]
pub use tracing_opentelemetry;

/// Errors returned by [`init`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OtelError {
    /// The OTLP exporter failed to build.
    ///
    /// Stores the error as a [`String`] because the underlying SDK error type
    /// does not implement `Send + Sync`, which prevents it from being used
    /// directly in an `enum` that must cross thread boundaries.
    #[error("failed to build OTLP exporter: {0}")]
    ExporterBuild(String),
}

/// A guard that shuts down OpenTelemetry exporters when dropped.
///
/// Keep this alive for the duration of the program (e.g. bind to `_guard` in
/// `main`). When dropped, the underlying [`SdkTracerProvider`] is shut down,
/// flushing any pending spans to the configured exporter.
#[must_use = "dropping OtelGuard shuts down telemetry; bind to a variable"]
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
            if let Err(e) = provider.shutdown() {
                #[expect(
                    clippy::print_stderr,
                    reason = "last-resort shutdown error: no logger available at drop time"
                )]
                {
                    eprintln!("bc-otel: failed to flush telemetry on shutdown: {e}");
                }
            }
        }
    }
}

/// Initialises the global OpenTelemetry tracer provider.
///
/// Reads the OTLP exporter endpoint from the standard
/// `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable. Returns an
/// [`OtelGuard`] that must be kept alive until program exit.
///
/// After calling this function, obtain a `tracing` bridge layer by calling
/// [`tracing_layer`] and register it with your [`tracing_subscriber`]
/// stack.
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

    opentelemetry::global::set_tracer_provider(provider.clone());

    Ok(OtelGuard {
        otel_provider: Some(provider),
    })
}

/// Returns the `tracing`-to-OpenTelemetry bridge layer.
///
/// Register the returned layer with your [`tracing_subscriber`] stack so that
/// `tracing` spans are forwarded to the OTLP exporter configured by [`init`].
///
/// Call [`init`] **before** this function so that the global tracer provider
/// is set; otherwise the layer will use a no-op tracer.
///
/// # Example
///
/// ```no_run
/// use tracing_subscriber::prelude::*;
///
/// let _guard = bc_otel::init().expect("failed to initialise telemetry");
/// tracing_subscriber::registry()
///     .with(bc_otel::tracing_layer())
///     .init();
/// ```
#[inline]
#[must_use]
pub fn tracing_layer<S>()
-> tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry::global::BoxedTracer>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_opentelemetry::layer().with_tracer(opentelemetry::global::tracer("bc-otel"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: `OtelError` implements [`core::error::Error`].
    #[test]
    fn otel_error_implements_std_error() {
        fn assert_error<E: core::error::Error>() {}
        assert_error::<OtelError>();
    }

    /// `init()` with an environment-set endpoint that is unreachable should
    /// not panic — it must return `Ok` (the exporter build succeeds; network
    /// errors only surface at export time) or `Err(OtelError::ExporterBuild)`.
    /// Either way, no panic is acceptable.
    #[tokio::test]
    async fn init_with_bad_endpoint_does_not_panic() {
        // SAFETY: Tests run in isolated processes under nextest; no concurrent
        // threads are reading environment variables.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:0");
        }
        let result = init();
        // SAFETY: Same as above.
        unsafe { std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT") }
        // Shutdown cleanly regardless of the outcome.
        drop(result);
    }
}
