//! Tracing/logging configuration for the BorrowChecker CLI.

use core::fmt;
use core::time::Duration;
use std::sync::LazyLock;
use std::time::Instant;

use owo_colors::OwoColorize as _;
use tracing::Event;
use tracing::Subscriber;
use tracing::debug;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Instant captured at process start, used to compute elapsed time for log lines.
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// A [`Duration`] wrapper that formats itself as `X.XXµs`, `X.XXms`, `X.XXs`, or `X.XXm`,
/// automatically choosing the largest unit that keeps the integer part non-zero.
struct Elapsed(Duration);

impl fmt::Display for Elapsed {
    #[inline]
    #[expect(
        clippy::cast_precision_loss,
        clippy::as_conversions,
        clippy::float_arithmetic,
        reason = "We accept losing precision when converting u128 micros to f64 \
        for human-friendly formatting, as we will be rounding to 2 decimal places anyway."
    )]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let micros = self.0.as_micros() as f64;
        if micros < 1_000.0 {
            write!(f, "{micros:.2}µs")
        } else if micros < 1_000_000.0 {
            write!(f, "{:.2}ms", micros / 1_000.0)
        } else if micros < 60_000_000.0 {
            write!(f, "{:.2}s", micros / 1_000_000.0)
        } else {
            write!(f, "{:.2}m", micros / 60_000_000.0)
        }
    }
}

/// Custom log event formatter with optional elapsed timestamp and span breadcrumbs.
struct LogFormat {
    /// Whether to prefix each line with the elapsed time since process start.
    display_timestamp: bool,
    /// Whether to prefix each line with the active span breadcrumb trail.
    show_spans: bool,
}

impl<S, N> FormatEvent<S, N> for LogFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    #[inline]
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let ansi = writer.has_ansi_escapes();

        if self.display_timestamp {
            let elapsed = Elapsed(START.elapsed());
            if ansi {
                write!(writer, "{} ", elapsed.dimmed())?;
            } else {
                write!(writer, "{elapsed} ")?;
            }
        }

        let level = *meta.level();
        if ansi {
            match level {
                tracing::Level::ERROR => write!(writer, "{} ", level.red())?,
                tracing::Level::WARN => write!(writer, "{} ", level.yellow())?,
                tracing::Level::INFO => write!(writer, "{} ", level.green())?,
                tracing::Level::DEBUG => write!(writer, "{} ", level.blue())?,
                tracing::Level::TRACE => write!(writer, "{} ", level.magenta())?,
            }
        } else {
            write!(writer, "{level} ")?;
        }

        if self.show_spans {
            let span = event
                .parent()
                .and_then(|id| ctx.span(id))
                .or_else(|| ctx.lookup_current());
            let scope = span.into_iter().flat_map(|s| s.scope().from_root());
            let mut seen = false;
            for s in scope {
                seen = true;
                if ansi {
                    write!(writer, "{}:", s.metadata().name().bold())?;
                } else {
                    write!(writer, "{}:", s.metadata().name())?;
                }
            }
            if seen {
                writer.write_char(' ')?;
            }
        }

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Initialise the tracing subscriber for the CLI.
///
/// The effective log level is determined by `verbose` and `quiet` counts relative
/// to a `warn` baseline:
///
/// | Flags | Level |
/// |-------|-------|
/// | `-q` (or more) | `error` |
/// | _(default)_ | `warn` |
/// | `-v` | `info` |
/// | `-vv` | `debug` (+ elapsed timestamps and span breadcrumbs) |
/// | `-vvv` | `trace` (+ elapsed timestamps and span breadcrumbs) |
///
/// Setting `RUST_LOG` overrides the flag-based level entirely and suppresses
/// span breadcrumbs (use spans directly via `RUST_LOG` directives instead).
///
/// If `OTEL_EXPORTER_OTLP_ENDPOINT` is set, OpenTelemetry tracing is initialised
/// automatically via [`bc_otel::init`], regardless of the log level.
///
/// # Returns
///
/// An [`bc_otel::OtelGuard`] that **must** be kept alive for the duration of the
/// process to ensure telemetry is flushed on exit, or [`None`] if
/// `OTEL_EXPORTER_OTLP_ENDPOINT` was not set.
#[must_use]
pub(crate) fn setup_tracing(verbose: u8, quiet: u8) -> Option<bc_otel::OtelGuard> {
    let (filter, show_spans) = if let Ok(rust_log) = std::env::var("RUST_LOG") {
        (EnvFilter::new(rust_log), false)
    } else {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "verbose and quiet are u8 values (max 255 each), so the \
            computed range [-254, 256] cannot overflow i32"
        )]
        let net = (1_i32 + i32::from(verbose) - i32::from(quiet)).clamp(0_i32, 4_i32);
        let (level_str, show_spans) = match net {
            0_i32 => ("error", false),
            1_i32 => ("warn", false),
            2_i32 => ("info", false),
            3_i32 => ("debug", true),
            _ => ("trace", true),
        };
        (EnvFilter::new(level_str), show_spans)
    };

    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr());

    #[expect(
        clippy::expect_used,
        reason = "OpenTelemetry initialisation failure is unrecoverable in a CLI binary; \
        the user must fix the OTLP endpoint configuration before the process can continue"
    )]
    let otel_guard = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .map(|_| bc_otel::init().expect("failed to initialise OpenTelemetry"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .event_format(LogFormat {
                    display_timestamp: show_spans,
                    show_spans,
                })
                .with_ansi(ansi)
                .with_writer(std::io::stderr),
        )
        .with(otel_guard.as_ref().map(|_| bc_otel::tracing_layer()))
        .init();

    debug!("Tracing initialized");
    otel_guard
}
