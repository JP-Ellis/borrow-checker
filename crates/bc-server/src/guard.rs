//! Request guards against DNS rebinding and cross-site RPC.
//!
//! The server has no authentication, so a page on another site must not
//! reach it through the visitor's browser. [`host`] refuses a `Host` that a
//! rebinding attack would carry. [`rpc`] refuses a request a browser would
//! send cross-site without a CORS preflight.
//!
//! Each refusal is a `BcError::Validation` JSON body under its own status
//! (403 or 415), so the WASM client decodes it like any other error.

use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::Arc;

use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::header;
use axum::http::uri::Authority;
use axum::middleware::Next;
use axum::response::IntoResponse as _;
use axum::response::Response;
use bc_ipc::BcError;

use crate::Shared;

// MARK: Middleware

/// Refuses a request unless its `Host` is an IP literal, `localhost`, or one
/// of the configured `allowed-hosts`, with any port.
///
/// A DNS-rebinding page reaches the server under the attacker's hostname,
/// so its `Host` names that hostname.
pub(crate) async fn host(State(shared): State<Arc<Shared>>, req: Request, next: Next) -> Response {
    let allowed = request_host(&req)
        .and_then(HostPort::parse)
        .is_some_and(|a| {
            a.is_ip() || a.host == "localhost" || shared.allowed_hosts.contains(&a.host)
        });
    if allowed {
        next.run(req).await
    } else {
        refuse(
            StatusCode::FORBIDDEN,
            "the Host header names a host this server does not answer; add it to \
             [server] allowed-hosts",
        )
    }
}

/// Refuses an RPC request that is cross-site or not JSON.
///
/// A cross-site HTML form or `fetch` without a preflight can send only
/// `text/plain`, `multipart/form-data` or `application/x-www-form-urlencoded`.
/// Requiring `application/json` forces a preflight, which the server never
/// answers. The `Sec-Fetch-Site` and `Origin` checks refuse a cross-site
/// request outright. An `Origin` must match `Host` or `X-Forwarded-Host`.
pub(crate) async fn rpc(req: Request, next: Next) -> Response {
    // Other methods reach no handler; the router answers them 405.
    if req.method() != Method::POST {
        return next.run(req).await;
    }
    let headers = req.headers();
    if header_str(headers, "sec-fetch-site").is_some_and(|s| s.eq_ignore_ascii_case("cross-site")) {
        return refuse(StatusCode::FORBIDDEN, "cross-site RPC requests are refused");
    }
    if headers.contains_key(header::ORIGIN) {
        let origin = header_str(headers, header::ORIGIN.as_str());
        // A reverse proxy such as `trunk serve` rewrites `Host` and names the
        // browser's host in `X-Forwarded-Host`. A cross-site page cannot set
        // that header without a preflight, which the server never answers.
        let forwarded = header_str(headers, "x-forwarded-host")
            .and_then(|h| h.split(',').next())
            .map(str::trim);
        let same = origin.is_some_and(|o| {
            request_host(&req)
                .into_iter()
                .chain(forwarded)
                .any(|h| same_origin(o, h))
        });
        if !same {
            return refuse(
                StatusCode::FORBIDDEN,
                "the Origin header does not match the Host; cross-origin RPC requests are refused",
            );
        }
    }
    if !is_json(headers) {
        return refuse(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "RPC requests must have Content-Type: application/json",
        );
    }
    next.run(req).await
}

// MARK: Helpers

/// A `BcError::Validation` JSON body under `status`.
fn refuse(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(BcError::Validation(message.to_owned()))).into_response()
}

/// The header's value, when present and visible ASCII.
fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// The `Host` header, or the URI authority an HTTP/2 request carries instead.
fn request_host(req: &Request) -> Option<&str> {
    header_str(req.headers(), header::HOST.as_str())
        .or_else(|| req.uri().authority().map(Authority::as_str))
}

/// Whether `Content-Type` is `application/json`, parameters allowed.
fn is_json(headers: &HeaderMap) -> bool {
    header_str(headers, header::CONTENT_TYPE.as_str()).is_some_and(|ct| {
        ct.split(';')
            .next()
            .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
    })
}

/// Whether `origin` (`scheme://host[:port]`) names the same host and port as
/// `host`, taking the scheme's default port where either omits it.
fn same_origin(origin: &str, host: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    let default = if scheme.eq_ignore_ascii_case("http") {
        80
    } else if scheme.eq_ignore_ascii_case("https") {
        443
    } else {
        return false;
    };
    match (HostPort::parse(rest), HostPort::parse(host)) {
        (Some(o), Some(h)) => {
            o.host == h.host && o.port.unwrap_or(default) == h.port.unwrap_or(default)
        }
        _ => false,
    }
}

/// A `host[:port]` pair from `Host`, `Origin` or `X-Forwarded-Host`, with the
/// host lowercased and any trailing root dot removed. An IPv6 host keeps its
/// brackets.
#[derive(Debug, PartialEq, Eq)]
struct HostPort {
    /// The hostname or IP literal.
    host: String,
    /// The port, when given.
    port: Option<u16>,
}

impl HostPort {
    /// Parses `host`, `host:port`, `[v6]` or `[v6]:port`; `None` when
    /// malformed.
    fn parse(s: &str) -> Option<Self> {
        // The port starts after the IPv6 literal's `]`, or at the last `:`.
        let split = if s.starts_with('[') {
            s.find(']')?.checked_add(1)?
        } else {
            s.rfind(':').unwrap_or(s.len())
        };
        let (raw_host, raw_port) = s.split_at_checked(split)?;
        let port = match raw_port {
            "" => None,
            p => Some(p.strip_prefix(':')?.parse().ok()?),
        };
        let host = raw_host.trim_end_matches('.').to_ascii_lowercase();
        (!host.is_empty()).then_some(Self { host, port })
    }

    /// Whether the host is an IPv4 literal or a bracketed IPv6 literal.
    fn is_ip(&self) -> bool {
        match self
            .host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
        {
            Some(v6) => v6.parse::<Ipv6Addr>().is_ok(),
            None => self.host.parse::<Ipv4Addr>().is_ok(),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::name("Ledger.Example.", Some(("ledger.example", None)))]
    #[case::name_port("nas:7171", Some(("nas", Some(7171))))]
    #[case::v4("127.0.0.1:80", Some(("127.0.0.1", Some(80))))]
    #[case::v6("[::1]:7171", Some(("[::1]", Some(7171))))]
    #[case::v6_no_port("[::1]", Some(("[::1]", None)))]
    #[case::bad_port("nas:http", None)]
    #[case::empty("", None)]
    #[case::unclosed("[::1", None)]
    fn authorities_parse(#[case] s: &str, #[case] parts: Option<(&str, Option<u16>)>) {
        let expected = parts.map(|(host, port)| HostPort {
            host: host.to_owned(),
            port,
        });
        assert_eq!(HostPort::parse(s), expected);
    }

    #[rstest]
    #[case::v4("192.168.1.5:7171", true)]
    #[case::v6("[fd7a:115c:a1e0::1]", true)]
    #[case::unbracketed_v6("::1", false)]
    #[case::name("evil.example", false)]
    #[case::numeric_name("127.0.0.1.evil.example", false)]
    fn ip_literals_are_recognised(#[case] s: &str, #[case] expected: bool) {
        assert_eq!(HostPort::parse(s).is_some_and(|a| a.is_ip()), expected);
    }

    #[rstest]
    #[case::same("http://127.0.0.1:7171", "127.0.0.1:7171", true)]
    #[case::default_port("http://nas", "nas:80", true)]
    #[case::case("http://NAS:7171", "nas:7171", true)]
    #[case::other_port("http://127.0.0.1:1420", "127.0.0.1:7171", false)]
    #[case::other_host("http://evil.example:7171", "127.0.0.1:7171", false)]
    #[case::https_default("https://nas", "nas:80", false)]
    #[case::null("null", "127.0.0.1:7171", false)]
    #[case::scheme("file://x", "x", false)]
    fn origins_compare_by_host_and_port(
        #[case] origin: &str,
        #[case] host: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(same_origin(origin, host), expected);
    }
}
