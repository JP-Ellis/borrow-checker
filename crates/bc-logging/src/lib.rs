//! Logging and tracing initialisation for BorrowChecker.
//!
//! Call [`init`] once at binary startup. Library crates use `tracing`
//! macros directly; they do not depend on this crate.

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

/// Output format for log records.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, coloured output for terminals.
    #[default]
    Pretty,
    /// Machine-readable JSON (one object per line).
    Json,
    /// Compact single-line output.
    Compact,
}

/// Configuration for the tracing subscriber.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Maximum verbosity level.
    pub max_level: tracing::Level,
    /// Output format.
    pub format: LogFormat,
    /// OTLP endpoint URL (requires `opentelemetry` feature).
    #[cfg(feature = "opentelemetry")]
    pub otlp_endpoint: Option<String>,
}

impl Default for LogConfig {
    #[inline]
    fn default() -> Self {
        Self {
            max_level: tracing::Level::INFO,
            format: LogFormat::default(),
            #[cfg(feature = "opentelemetry")]
            otlp_endpoint: None,
        }
    }
}

/// Errors from [`init`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The global subscriber has already been set.
    #[error("tracing subscriber already initialised")]
    AlreadyInitialised,
}

/// A guard that shuts down telemetry exporters when dropped.
///
/// Keep this alive for the duration of the program (e.g. bind to `_guard` in `main`).
#[must_use = "dropping LogGuard shuts down telemetry; bind to a variable"]
pub struct LogGuard {
    #[cfg(feature = "opentelemetry")]
    /// Whether OpenTelemetry was successfully initialised and needs shutdown.
    otel_enabled: bool,
    /// Private field to prevent direct construction.
    _private: (),
}

impl core::fmt::Debug for LogGuard {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LogGuard").finish()
    }
}

#[cfg(feature = "opentelemetry")]
impl Drop for LogGuard {
    #[inline]
    fn drop(&mut self) {
        if self.otel_enabled {
            opentelemetry::global::shutdown_tracer_provider();
        }
    }
}

/// Attempt to build an OTLP exporter and register a global tracer provider.
///
/// Returns `Some(tracer)` if OpenTelemetry was successfully initialised, `None` otherwise.
/// A `Some` return means `opentelemetry::global::shutdown_tracer_provider()` must be called
/// on program exit.
#[cfg(feature = "opentelemetry")]
fn try_init_otel(endpoint: &str) -> Option<opentelemetry_sdk::trace::Tracer> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;
    use opentelemetry_sdk::trace::TracerProvider;

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to build OTLP exporter; OpenTelemetry disabled");
            return None;
        }
    };

    let provider = TracerProvider::builder()
        .with_simple_exporter(exporter)
        .build();

    let tracer = provider.tracer("bc-logging");
    opentelemetry::global::set_tracer_provider(provider);
    Some(tracer)
}

/// Initialises the global tracing subscriber.
///
/// Call this once at the start of `main`. Returns a [`LogGuard`] that must
/// be kept alive until program exit.
///
/// # Errors
///
/// Returns [`InitError::AlreadyInitialised`] if called more than once.
#[expect(
    clippy::needless_pass_by_value,
    reason = "init() takes ownership of LogConfig to prevent reuse after initialisation"
)]
#[inline]
pub fn init(config: LogConfig) -> Result<LogGuard, InitError> {
    let env_filter = EnvFilter::builder()
        .with_default_directive(config.max_level.into())
        .from_env_lossy();

    #[cfg(feature = "opentelemetry")]
    let otel_layer: Option<
        tracing_opentelemetry::OpenTelemetryLayer<_, opentelemetry_sdk::trace::Tracer>,
    > = config.otlp_endpoint.as_deref().and_then(|endpoint| {
        try_init_otel(endpoint).map(|tracer| tracing_opentelemetry::layer().with_tracer(tracer))
    });

    #[cfg(feature = "opentelemetry")]
    let otel_enabled = otel_layer.is_some();

    let result = match config.format {
        LogFormat::Pretty => tracing_subscriber::registry()
            .with(
                #[cfg(feature = "opentelemetry")]
                otel_layer,
                #[cfg(not(feature = "opentelemetry"))]
                None::<fmt::Layer<_>>,
            )
            .with(env_filter)
            .with(fmt::layer().pretty())
            .try_init(),
        LogFormat::Json => tracing_subscriber::registry()
            .with(
                #[cfg(feature = "opentelemetry")]
                otel_layer,
                #[cfg(not(feature = "opentelemetry"))]
                None::<fmt::Layer<_>>,
            )
            .with(env_filter)
            .with(fmt::layer().json())
            .try_init(),
        LogFormat::Compact => tracing_subscriber::registry()
            .with(
                #[cfg(feature = "opentelemetry")]
                otel_layer,
                #[cfg(not(feature = "opentelemetry"))]
                None::<fmt::Layer<_>>,
            )
            .with(env_filter)
            .with(fmt::layer().compact())
            .try_init(),
    };

    result.map_err(|_e| InitError::AlreadyInitialised)?;
    Ok(LogGuard {
        #[cfg(feature = "opentelemetry")]
        otel_enabled,
        _private: (),
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::{assert_eq, assert_matches};

    use super::*;

    #[test]
    fn log_config_default_is_info_pretty() {
        let c = LogConfig::default();
        assert_eq!(c.max_level, tracing::Level::INFO);
        assert_matches!(c.format, LogFormat::Pretty);
    }

    #[test]
    fn init_with_pretty_format_does_not_panic() {
        let config = LogConfig {
            max_level: tracing::Level::ERROR,
            format: LogFormat::Pretty,
            ..LogConfig::default()
        };
        drop(init(config)); // may fail if subscriber already set — that's OK
    }

    #[cfg(feature = "opentelemetry")]
    #[test]
    fn init_with_otel_and_no_endpoint_does_not_panic() {
        let config = LogConfig {
            max_level: tracing::Level::ERROR,
            format: LogFormat::Compact,
            otlp_endpoint: None,
        };
        drop(init(config));
    }
}
