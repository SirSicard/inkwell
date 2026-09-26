//! The local-only guard (architecture rule 6): in local-only mode, nothing is sent to an endpoint
//! that is not on this machine. The check is in code, before any network call and before the
//! keychain is asked for a key.
//!
//! # What counts as this machine
//!
//! Exactly three kinds of host, compared after the URL is parsed by the `http` crate:
//! - `localhost`, in any letter case;
//! - an IPv4 address in `127.0.0.0/8`, written as four plain decimal numbers;
//! - the IPv6 loopback `::1`, in brackets (`[::1]`, or a longer spelling of the same address).
//!
//! Everything else is remote, including hosts that merely look local. The decisions, each with a
//! named test:
//! - `localhost.example.com`, `127.0.0.1.nip.io`, `foo.localhost`: names DNS resolves, remote.
//! - `localhost.` (a trailing dot) is remote: whether the fully qualified form goes to the hosts
//!   file or to DNS differs between resolvers, so the guard cannot promise where it lands.
//! - `0.0.0.0` and `[::]` are remote: "any address" is not a loopback address, and connecting to
//!   it behaves differently per OS.
//! - `[::ffff:127.0.0.1]` (an IPv4-mapped loopback) is remote. It is not in the rule's list, and
//!   refusing it costs nothing: `127.0.0.1` says the same thing plainly.
//! - Other spellings of IPv4 (`127.1`, `2130706433`, `0127.0.0.1`) are remote. Some URL parsers
//!   read them as addresses and some as names; refusing them removes the disagreement.
//!
//! A URL with user information (`http://127.0.0.1@example.com`), a fragment
//! (`http://example.com#@127.0.0.1`) or a query does not parse at all ([`EndpointError`]): each is
//! a classic way to make two parsers see different hosts, and a model endpoint needs none of them.
//! Neither does a scheme other than `http` or `https`, or a URL with no host.
//!
//! What is sent is rebuilt from the parsed parts ([`EndpointUrl::join`]), so the HTTP client never
//! sees the original string. The client also resolves `localhost` itself, to the loopback
//! addresses only ([`crate::transport`]), and checks the host again with its own parser.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};

/// Why a URL is not a usable model endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointError {
    /// Not a URL the `http` crate can parse.
    Invalid,
    /// The scheme is missing, or is not `http` or `https`.
    NotHttp,
    /// There is no host.
    NoHost,
    /// The URL carries user information (`user@host`). Refused: see the module docs.
    UserInfo,
    /// The URL has a query or a fragment. Refused: see the module docs.
    QueryOrFragment,
    /// The port is not a number from 0 to 65535.
    BadPort,
    /// Plain `http` to another machine, for a provider that needs an API key. A key only
    /// travels over `https`, or to this machine.
    PlainHttp,
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "not a valid URL",
            Self::NotHttp => "the URL must start with http:// or https://",
            Self::NoHost => "the URL has no host",
            Self::UserInfo => "the URL must not contain a user name or password",
            Self::QueryOrFragment => "the URL must not contain a query (?) or a fragment (#)",
            Self::BadPort => "the URL's port is not valid",
            Self::PlainHttp => "an API key is only sent over https, or to this machine",
        })
    }
}

impl std::error::Error for EndpointError {}

/// A model endpoint's base URL, parsed once when the provider is built.
///
/// Its [`Display`](std::fmt::Display) form is `scheme://host[:port][/path]`: never user
/// information, a query or a fragment, since those are refused. It is what errors and
/// [`LlmInfo`] name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointUrl {
    scheme: &'static str,
    host: String,
    port: Option<u16>,
    /// The path without a trailing slash; empty for the root.
    path: String,
    loopback: bool,
}

impl EndpointUrl {
    /// Parses and checks a base URL (for example `http://localhost:11434/v1`).
    pub fn parse(raw: &str) -> Result<Self, EndpointError> {
        // The `http` crate drops a fragment without a word, so it is refused on the raw text.
        if raw.contains('#') {
            return Err(EndpointError::QueryOrFragment);
        }
        let uri: http::Uri = raw.parse().map_err(|_| EndpointError::Invalid)?;
        let authority = uri.authority().ok_or(EndpointError::NoHost)?;
        let scheme = match uri.scheme_str() {
            Some("http") => "http",
            Some("https") => "https",
            _ => return Err(EndpointError::NotHttp),
        };
        if authority.as_str().contains('@') {
            return Err(EndpointError::UserInfo);
        }
        if uri.query().is_some() {
            return Err(EndpointError::QueryOrFragment);
        }
        let host = authority.host();
        if host.is_empty() {
            return Err(EndpointError::NoHost);
        }
        // What follows the host is nothing, a bare `:`, or `:` and a valid port. The `http` crate
        // accepts an out-of-range port and reports it as absent, which would silently change
        // where the request goes.
        let port = match authority.as_str().strip_prefix(host) {
            Some("" | ":") => None,
            Some(_) => Some(authority.port_u16().ok_or(EndpointError::BadPort)?),
            None => return Err(EndpointError::Invalid),
        };
        let path = uri.path().trim_end_matches('/').to_owned();
        Ok(Self {
            scheme,
            host: host.to_owned(),
            port,
            path,
            loopback: is_loopback_host(host),
        })
    }

    /// Whether the host is on this machine (see the module docs for the exact rule).
    pub fn is_loopback(&self) -> bool {
        self.loopback
    }

    /// Whether the scheme is `https`.
    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    /// Where a model at this URL runs, for [`LlmInfo`].
    pub fn endpoint(&self) -> Endpoint {
        if self.loopback {
            Endpoint::Loopback(self.to_string())
        } else {
            Endpoint::Remote(self.to_string())
        }
    }

    /// The URL of `suffix` under this base: `join("/chat/completions")`. Rebuilt from the parsed
    /// parts, never from the text the user typed.
    pub fn join(&self, suffix: &str) -> String {
        format!("{self}{suffix}")
    }
}

impl std::fmt::Display for EndpointUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.path)
    }
}

/// Whether `host`, as a URL writes it, is this machine: `localhost` in any case, `127.0.0.0/8`
/// in plain dotted decimal, or `[::1]`. IPv6 hosts are bracketed, as in a URL.
pub fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        // `is_loopback` is `::1` only; an IPv4-mapped `::ffff:127.0.0.1` is deliberately not.
        return inner.parse::<Ipv6Addr>().is_ok_and(|a| a.is_loopback());
    }
    // The standard parser takes only four plain decimal numbers with no leading zeros, so the
    // shorthand and octal spellings some URL parsers accept are not loopback here.
    host.parse::<Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

/// The local-only switch. Clones share one flag, so the setting can change at runtime and every
/// provider sees it on its next call.
#[derive(Clone, Debug, Default)]
pub struct LocalOnly(Arc<AtomicBool>);

impl LocalOnly {
    /// A switch in the given position.
    pub fn new(on: bool) -> Self {
        Self(Arc::new(AtomicBool::new(on)))
    }

    /// Turns local-only mode on or off for every clone.
    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Release);
    }

    /// Whether local-only mode is on.
    pub fn is_on(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Refuses `endpoint` when local-only mode is on and it is not on this machine.
    pub fn check(&self, endpoint: &Endpoint) -> Result<(), LlmError> {
        match endpoint {
            Endpoint::Remote(url) if self.is_on() => Err(LlmError::LocalOnly {
                endpoint: url.clone(),
            }),
            _ => Ok(()),
        }
    }
}

/// Any [`Llm`] behind the local-only switch: in local-only mode, a model whose
/// [`info`](Llm::info) says it is remote is never called.
///
/// [`ByokLlm`](crate::ByokLlm) applies the same check itself; this wrapper is for models it does
/// not build (an engine the Mac shell registers, a local adapter), so the pipeline can hold every
/// model the same way.
pub struct GuardedLlm {
    inner: Arc<dyn Llm>,
    local_only: LocalOnly,
}

impl GuardedLlm {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn Llm>, local_only: LocalOnly) -> Self {
        Self { inner, local_only }
    }
}

impl Llm for GuardedLlm {
    fn info(&self) -> LlmInfo {
        self.inner.info()
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        self.local_only.check(&self.inner.info().endpoint)?;
        self.inner.complete(request, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::mock::MockLlm;

    fn loopback(raw: &str) -> bool {
        EndpointUrl::parse(raw)
            .unwrap_or_else(|e| panic!("{raw} should parse: {e}"))
            .is_loopback()
    }

    #[test]
    fn loopback_hosts_are_local() {
        for raw in [
            "http://localhost:11434/v1",
            "http://LOCALHOST/v1",
            "http://LocalHost:8080",
            "HTTP://127.0.0.1/v1",
            "http://127.1.2.3:9000/v1",
            "https://127.255.255.254/",
            "http://[::1]:8080/v1",
            "http://[0:0:0:0:0:0:0:1]/",
        ] {
            assert!(loopback(raw), "{raw} should be loopback");
        }
    }

    #[test]
    fn lookalike_hosts_are_remote() {
        for raw in [
            "http://localhost.example.com/v1",
            "http://127.0.0.1.nip.io/v1",
            "http://foo.localhost/v1",
            "http://localhost./v1",
            "http://127.0.0.1./v1",
            "http://0.0.0.0:8080/v1",
            "http://[::]/v1",
            "http://[::ffff:127.0.0.1]/v1",
            "http://[::ffff:7f00:1]/v1",
            "http://127.1/v1",
            "http://2130706433/v1",
            "http://0127.0.0.1/v1",
            "http://127.000.0.1/v1",
            "http://0x7f.0.0.1/v1",
            "http://[fe80::1%25en0]/v1",
            "https://api.openai.com/v1",
        ] {
            assert!(!loopback(raw), "{raw} should be remote");
        }
    }

    #[test]
    fn user_information_is_refused() {
        for raw in [
            "http://127.0.0.1@example.com/v1",
            "http://user:secret@127.0.0.1/v1",
            "http://a@b@127.0.0.1/",
            "http://localhost@localhost/",
        ] {
            assert_eq!(
                EndpointUrl::parse(raw),
                Err(EndpointError::UserInfo),
                "{raw}"
            );
        }
    }

    #[test]
    fn fragments_and_queries_are_refused() {
        for raw in [
            "http://example.com#@127.0.0.1",
            "http://127.0.0.1/v1#frag",
            "http://127.0.0.1/v1?key=abc",
            "http://127.0.0.1/v1?",
        ] {
            assert_eq!(
                EndpointUrl::parse(raw),
                Err(EndpointError::QueryOrFragment),
                "{raw}"
            );
        }
    }

    #[test]
    fn non_http_schemes_are_refused() {
        for raw in [
            "ftp://127.0.0.1/",
            "ws://localhost/v1",
            "localhost:8080",
            "file:///etc/hosts",
        ] {
            let got = EndpointUrl::parse(raw);
            assert!(
                matches!(got, Err(EndpointError::NotHttp | EndpointError::Invalid)),
                "{raw}: {got:?}"
            );
        }
    }

    #[test]
    fn a_url_with_no_host_is_refused() {
        for raw in ["/v1", "http:///v1", "http://", "", "   "] {
            let got = EndpointUrl::parse(raw);
            assert!(
                matches!(got, Err(EndpointError::NoHost | EndpointError::Invalid)),
                "{raw}: {got:?}"
            );
        }
    }

    #[test]
    fn an_out_of_range_port_is_refused() {
        assert_eq!(
            EndpointUrl::parse("http://localhost:99999/v1"),
            Err(EndpointError::BadPort)
        );
        // An empty port is the default port.
        assert!(loopback("http://127.0.0.1:/v1"));
    }

    #[test]
    fn the_rebuilt_url_is_scheme_host_port_and_path_only() {
        let url = EndpointUrl::parse("HTTP://LOCALHOST:11434/v1/").unwrap();
        assert_eq!(url.to_string(), "http://LOCALHOST:11434/v1");
        assert_eq!(
            url.join("/chat/completions"),
            "http://LOCALHOST:11434/v1/chat/completions"
        );
        let root = EndpointUrl::parse("https://api.example.com").unwrap();
        assert_eq!(
            root.join("/v1/messages"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            EndpointUrl::parse("http://[::1]:8080/v1")
                .unwrap()
                .to_string(),
            "http://[::1]:8080/v1"
        );
    }

    #[test]
    fn the_endpoint_classification_matches_the_guard() {
        let local = EndpointUrl::parse("http://127.0.0.1:8080/v1").unwrap();
        assert_eq!(
            local.endpoint(),
            Endpoint::Loopback("http://127.0.0.1:8080/v1".into())
        );
        let remote = EndpointUrl::parse("http://localhost.example.com/v1").unwrap();
        assert_eq!(
            remote.endpoint(),
            Endpoint::Remote("http://localhost.example.com/v1".into())
        );
    }

    #[test]
    fn the_switch_refuses_remote_endpoints_only_when_on() {
        let switch = LocalOnly::new(false);
        let remote = Endpoint::Remote("https://api.example.com/v1".into());
        assert_eq!(switch.check(&remote), Ok(()));

        let clone = switch.clone();
        clone.set(true);
        assert!(switch.is_on(), "clones share the flag");
        assert_eq!(
            switch.check(&remote),
            Err(LlmError::LocalOnly {
                endpoint: "https://api.example.com/v1".into()
            })
        );
        assert_eq!(switch.check(&Endpoint::InProcess), Ok(()));
        assert_eq!(
            switch.check(&Endpoint::Loopback("http://localhost/v1".into())),
            Ok(())
        );
    }

    #[test]
    fn a_guarded_remote_model_is_never_called_in_local_only_mode() {
        let request = LlmRequest {
            system: String::new(),
            user: "synthetic".into(),
            max_tokens: 16,
            temperature: 0.0,
            json_schema: None,
        };
        let remote = Arc::new(MockLlm::new(
            Endpoint::Remote("https://api.example.com/v1".into()),
            "answer",
        ));
        let switch = LocalOnly::new(true);
        let guarded = GuardedLlm::new(remote.clone(), switch.clone());
        assert!(matches!(
            guarded.complete(&request, &CancelToken::new()),
            Err(LlmError::LocalOnly { .. })
        ));
        assert_eq!(remote.calls(), 0);

        switch.set(false);
        assert_eq!(
            guarded
                .complete(&request, &CancelToken::new())
                .unwrap()
                .text,
            "answer"
        );
        assert_eq!(remote.calls(), 1);

        let local = Arc::new(MockLlm::new(Endpoint::InProcess, "local"));
        let guarded = GuardedLlm::new(local.clone(), LocalOnly::new(true));
        assert_eq!(
            guarded
                .complete(&request, &CancelToken::new())
                .unwrap()
                .text,
            "local"
        );
        assert_eq!(local.calls(), 1);
    }
}
