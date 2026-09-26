//! The HTTP seam: [`Transport`] posts one request and returns the answer. Providers only ever talk
//! to the network through it, so tests count calls without a server, and the real client is
//! built once.
//!
//! [`UreqTransport`] is the one implementation: [`ureq`] 2 over the OS TLS stack (native-tls),
//! blocking, on the calling worker thread. Its settings are decisions:
//! - **No redirects.** A redirect is returned as its status. Following one could take a request
//!   that passed the local-only guard to another machine.
//! - **`localhost` resolves to `127.0.0.1` and `::1` only**, never through DNS or a hosts file,
//!   so the guard's idea of `localhost` is where the connection goes.
//! - **The host is checked again** with the client's own URL parser when the request is
//!   loopback-only ([`HttpRequest::loopback_only`]), so a disagreement between two parsers
//!   fails closed.
//! - **No proxy** (ureq reads no proxy settings from the environment unless asked).
//! - **An error status's body is never read.** Providers echo the request, which is the user's
//!   text, back in their error bodies.
//!
//! # Logging
//!
//! This crate logs nothing, but ureq logs at `debug` level through the `log` facade, including
//! each request's header block with only `Authorization` and `Cookie` masked. Anthropic's key goes
//! in `x-api-key`, so whoever installs a logger must keep the targets in [`QUIET_LOG_TARGETS`] at
//! `info` or quieter.

use std::io::{self, Read};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::guard::is_loopback_host;

/// Log targets that must never be enabled at `debug` or `trace`: their debug output includes
/// request headers, and so API keys (see the module docs).
pub const QUIET_LOG_TARGETS: &[&str] = &["ureq"];

/// One POST. It holds an API key and the user's text, so its `Debug` shows neither.
pub struct HttpRequest {
    /// The full URL, rebuilt from a parsed [`EndpointUrl`](crate::EndpointUrl).
    pub url: String,
    /// Header names and values. Values may hold an API key.
    pub headers: Vec<(&'static str, String)>,
    /// The JSON body.
    pub body: Vec<u8>,
    /// Refuse to connect unless the URL's host is this machine. Set in local-only mode; the
    /// provider has already checked, and the transport checks again with its own parser.
    pub loopback_only: bool,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| *name).collect();
        f.debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_bytes", &self.body.len())
            .field("loopback_only", &self.loopback_only)
            .finish()
    }
}

/// One answer. For an error status the body is empty: it is never read (see the module docs).
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The body of a 2xx answer.
    pub body: Vec<u8>,
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Why a request got no answer. The variants carry no text from the request or the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// The request was loopback-only and the client's parser found another host. Nothing was
    /// sent.
    NotLoopback,
    /// The client could not parse the URL.
    InvalidUrl,
    /// The host name did not resolve.
    Dns,
    /// The connection or the TLS handshake failed.
    Connect,
    /// The server did not answer in time.
    Timeout,
    /// The connection failed mid-exchange.
    Io,
    /// The answer was larger than the configured limit.
    TooLarge,
}

impl TransportError {
    /// A fixed description, safe to show and log.
    pub fn describe(self) -> &'static str {
        match self {
            Self::NotLoopback => "the client refused a host that is not this machine",
            Self::InvalidUrl => "the client could not parse the endpoint URL",
            Self::Dns => "could not resolve the host",
            Self::Connect => "could not connect",
            Self::Timeout => "timed out",
            Self::Io => "the connection failed",
            Self::TooLarge => "the answer was too large",
        }
    }
}

/// Posts requests.
pub trait Transport: Send + Sync {
    /// **Worker.** Sends `request` and returns the answer whatever its status. Blocks until the
    /// answer arrives or a timeout fires.
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// [`UreqTransport`]'s limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportConfig {
    /// Time allowed to connect, TLS included.
    pub connect_timeout: Duration,
    /// Time allowed between bytes of the answer. Generous: a local server summarising a long
    /// meeting can think for minutes before its first byte.
    pub read_timeout: Duration,
    /// Time allowed to send the request.
    pub write_timeout: Duration,
    /// The largest answer body accepted.
    pub max_body_bytes: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(300),
            write_timeout: Duration::from_secs(60),
            max_body_bytes: 8 * 1024 * 1024,
        }
    }
}

/// The real client. Clones share one connection pool.
#[derive(Clone)]
pub struct UreqTransport {
    agent: ureq::Agent,
    max_body_bytes: usize,
}

impl UreqTransport {
    /// Builds a client. Call it once and share it; [`shared`](Self::shared) does that for the
    /// process.
    pub fn new(config: TransportConfig) -> Self {
        let mut builder = ureq::AgentBuilder::new()
            .timeout_connect(config.connect_timeout)
            .timeout_read(config.read_timeout)
            .timeout_write(config.write_timeout)
            .redirects(0)
            .try_proxy_from_env(false)
            .user_agent("inkwell")
            .resolver(resolve);
        // Without a connector, https requests fail with a connection error rather than silently
        // falling back to anything else. The OS TLS stack failing to initialise is not expected.
        if let Ok(tls) = ureq::native_tls::TlsConnector::new() {
            builder = builder.tls_connector(Arc::new(tls));
        }
        Self {
            agent: builder.build(),
            max_body_bytes: config.max_body_bytes,
        }
    }

    /// The process's one client, built on first use with the default limits.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<UreqTransport>> = OnceLock::new();
        SHARED
            .get_or_init(|| Arc::new(Self::new(TransportConfig::default())))
            .clone()
    }
}

impl Transport for UreqTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let mut call = self.agent.post(&request.url);
        let parsed = call.request_url().map_err(|_| TransportError::InvalidUrl)?;
        if request.loopback_only && !parsed.as_url().host_str().is_some_and(is_loopback_host) {
            return Err(TransportError::NotLoopback);
        }
        for (name, value) in &request.headers {
            call = call.set(name, value);
        }
        match call.send_bytes(&request.body) {
            Ok(response) => {
                let status = response.status();
                if !(200..300).contains(&status) {
                    // A 1xx or 3xx: reported by status, body unread like any other non-success.
                    return Ok(HttpResponse {
                        status,
                        body: Vec::new(),
                    });
                }
                let limit = u64::try_from(self.max_body_bytes)
                    .unwrap_or(u64::MAX)
                    .saturating_add(1);
                let mut body = Vec::new();
                response
                    .into_reader()
                    .take(limit)
                    .read_to_end(&mut body)
                    .map_err(|e| io_error(&e))?;
                if body.len() > self.max_body_bytes {
                    return Err(TransportError::TooLarge);
                }
                Ok(HttpResponse { status, body })
            }
            // The body is dropped unread: providers echo the request in it.
            Err(ureq::Error::Status(status, _)) => Ok(HttpResponse {
                status,
                body: Vec::new(),
            }),
            Err(ureq::Error::Transport(transport)) => Err(transport_error(&transport)),
        }
    }
}

/// `localhost` is this machine, whatever DNS or a hosts file says; everything else resolves
/// normally. `netloc` is `host:port` as the client passes it.
///
/// `localhost.` (trailing dot) is pinned here too, although the guard calls it remote. Both are
/// the cautious reading: the guard refuses it in local-only mode and never sends it a key, and
/// the client never lets it reach DNS.
pub(crate) fn resolve(netloc: &str) -> io::Result<Vec<SocketAddr>> {
    if let Some((host, port)) = netloc.rsplit_once(':') {
        let host = host.trim_end_matches('.');
        if host.eq_ignore_ascii_case("localhost") {
            let port: u16 = port
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid port"))?;
            return Ok(vec![
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            ]);
        }
    }
    netloc.to_socket_addrs().map(Iterator::collect)
}

fn io_error(error: &io::Error) -> TransportError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TransportError::Timeout,
        _ => TransportError::Io,
    }
}

fn transport_error(error: &ureq::Transport) -> TransportError {
    use std::error::Error as _;
    if let Some(io) = error.source().and_then(|s| s.downcast_ref::<io::Error>())
        && matches!(
            io.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        )
    {
        return TransportError::Timeout;
    }
    match error.kind() {
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => TransportError::InvalidUrl,
        ureq::ErrorKind::Dns => TransportError::Dns,
        ureq::ErrorKind::ConnectionFailed => TransportError::Connect,
        _ => TransportError::Io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_resolves_to_loopback_only() {
        for netloc in ["localhost:11434", "LOCALHOST:80", "localhost.:8080"] {
            let addrs = resolve(netloc).unwrap();
            assert!(!addrs.is_empty());
            assert!(addrs.iter().all(|a| a.ip().is_loopback()), "{netloc}");
        }
        let addrs = resolve("127.0.0.1:9").unwrap();
        assert_eq!(addrs, vec![SocketAddr::from((Ipv4Addr::LOCALHOST, 9))]);
    }

    #[test]
    fn the_shared_client_is_built_once() {
        assert!(Arc::ptr_eq(
            &UreqTransport::shared(),
            &UreqTransport::shared()
        ));
    }

    #[test]
    fn debug_output_hides_keys_and_text() {
        let request = HttpRequest {
            url: "https://api.example.com/v1/chat/completions".into(),
            headers: vec![("Authorization", "Bearer sk-synthetic-canary".into())],
            body: b"{\"messages\":\"synthetic canary text\"}".to_vec(),
            loopback_only: false,
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("canary"), "{shown}");
        assert!(shown.contains("Authorization"));

        let response = HttpResponse {
            status: 200,
            body: b"synthetic canary answer".to_vec(),
        };
        assert!(!format!("{response:?}").contains("canary"));
    }

    #[test]
    fn a_loopback_only_request_to_another_host_is_refused_before_connecting() {
        // `.invalid` never resolves (RFC 2606), and the check runs before any lookup anyway.
        let transport = UreqTransport::new(TransportConfig::default());
        let request = HttpRequest {
            url: "http://model.invalid/v1/chat/completions".into(),
            headers: Vec::new(),
            body: Vec::new(),
            loopback_only: true,
        };
        assert_eq!(
            transport.post(&request).unwrap_err(),
            TransportError::NotLoopback
        );
    }
}
