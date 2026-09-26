//! The real transport: ureq 2 over the OS's TLS (Security.framework on macOS, SChannel on
//! Windows), HTTPS only.

use std::sync::Arc;
use std::time::Duration;

use crate::download::{Fetch, FetchError, Fetched};

/// Bounds a connection attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A transfer that stalls this long fails with a resumable error instead of holding a worker
/// forever. It also bounds how long a cancel can wait on a blocked read.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Fetches over HTTPS with ureq and native TLS, following redirects (which must stay on HTTPS).
///
/// Compression is off (ureq's `gzip` feature is not enabled), so the bytes received are the file's
/// bytes and the registry's size and hash apply to them directly.
pub struct HttpFetch {
    agent: ureq::Agent,
}

impl HttpFetch {
    /// A fetcher with the OS's TLS and certificate store.
    pub fn new() -> Result<Self, FetchError> {
        Self::build(true)
    }

    fn build(https_only: bool) -> Result<Self, FetchError> {
        let tls = ureq::native_tls::TlsConnector::new()
            .map_err(|e| FetchError::Network(format!("TLS setup failed: {e}")))?;
        let agent = ureq::AgentBuilder::new()
            .tls_connector(Arc::new(tls))
            .https_only(https_only)
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .user_agent(concat!("inkwell/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Self { agent })
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str, offset: u64) -> Result<Fetched, FetchError> {
        let mut request = self.agent.get(url);
        if offset > 0 {
            request = request.set("Range", &format!("bytes={offset}-"));
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) => return Err(FetchError::Http { status }),
            Err(ureq::Error::Transport(t)) => return Err(FetchError::Network(t.to_string())),
        };
        let (start, total) = match response.status() {
            206 => {
                let range = response.header("Content-Range").ok_or_else(|| {
                    FetchError::Network("HTTP 206 without a Content-Range header".into())
                })?;
                parse_content_range(range)?
            }
            200 => (
                0,
                response
                    .header("Content-Length")
                    .and_then(|v| v.trim().parse().ok()),
            ),
            status => return Err(FetchError::Http { status }),
        };
        Ok(Fetched {
            start,
            total,
            body: Box::new(response.into_reader()),
        })
    }
}

/// Parses `bytes <first>-<last>/<total or *>` into the first byte and the total, if given.
fn parse_content_range(value: &str) -> Result<(u64, Option<u64>), FetchError> {
    let bad = || FetchError::Network(format!("malformed Content-Range: {value:?}"));
    let rest = value.trim().strip_prefix("bytes ").ok_or_else(bad)?;
    let (range, total) = rest.split_once('/').ok_or_else(bad)?;
    let (first, last) = range.split_once('-').ok_or_else(bad)?;
    let first: u64 = first.trim().parse().map_err(|_| bad())?;
    let last: u64 = last.trim().parse().map_err(|_| bad())?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().map_err(|_| bad())?),
    };
    if last < first || total.is_some_and(|t| last >= t) {
        return Err(bad());
    }
    Ok((first, total))
}

#[cfg(test)]
mod tests {
    //! The fetcher against a one-shot HTTP server on the loopback interface: no network, no TLS.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// Serves `response` to one connection and reports the request's head.
    fn serve_once(response: &'static str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/synthetic/weights.bin",
            listener.local_addr().unwrap()
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let _ = tx.send(head);
            stream.write_all(response.as_bytes()).unwrap();
        });
        (url, rx)
    }

    /// Fetches with HTTPS enforcement off (the loopback server is plain HTTP) and reads the body.
    fn fetch(url: &str, offset: u64) -> Result<(u64, Option<u64>, Vec<u8>), FetchError> {
        HttpFetch::build(false)
            .unwrap()
            .get(url, offset)
            .map(|mut f| {
                let mut body = Vec::new();
                f.body.read_to_end(&mut body).unwrap();
                (f.start, f.total, body)
            })
    }

    fn head(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(20))
            .unwrap()
            .to_ascii_lowercase()
    }

    #[test]
    fn a_whole_file_request_sends_no_range() {
        let (url, rx) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        let result = fetch(&url, 0);
        assert_eq!(result.unwrap(), (0, Some(5), b"hello".to_vec()));
        assert!(!head(&rx).contains("range:"));
    }

    #[test]
    fn a_resume_sends_range_and_reads_content_range() {
        let (url, rx) = serve_once(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-4/5\r\nContent-Length: 2\r\nConnection: close\r\n\r\nlo",
        );
        let result = fetch(&url, 3);
        assert_eq!(result.unwrap(), (3, Some(5), b"lo".to_vec()));
        assert!(head(&rx).contains("range: bytes=3-\r\n"));
    }

    #[test]
    fn a_server_ignoring_range_is_reported_as_starting_at_zero() {
        let (url, _rx) =
            serve_once("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        let result = fetch(&url, 3);
        assert_eq!(result.unwrap(), (0, Some(5), b"hello".to_vec()));
    }

    #[test]
    fn an_error_status_is_an_http_error() {
        let (url, _rx) =
            serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let result = fetch(&url, 0);
        assert_eq!(result.unwrap_err(), FetchError::Http { status: 404 });
    }

    #[test]
    fn plain_http_is_refused_before_connecting() {
        // Port 9 on loopback: nothing listens, and nothing is dialled anyway.
        let fetcher = HttpFetch::new().unwrap();
        let err = fetcher
            .get("http://127.0.0.1:9/synthetic/weights.bin", 0)
            .err()
            .unwrap();
        assert!(
            matches!(&err, FetchError::Network(msg) if msg.contains("https")),
            "{err}"
        );
    }

    #[test]
    fn content_range_parses_and_rejects_nonsense() {
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Ok((100, Some(200)))
        );
        assert_eq!(parse_content_range("bytes 0-0/*"), Ok((0, None)));
        for bad in [
            "",
            "bytes",
            "bytes 5-3/10",
            "bytes 0-10/10",
            "items 0-1/2",
            "bytes a-b/c",
        ] {
            assert!(parse_content_range(bad).is_err(), "{bad:?} parsed");
        }
    }
}
