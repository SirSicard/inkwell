//! Test doubles shared by the integration tests: a transport that counts and records, a key
//! store that can deny, a scripted model, and a loopback-only HTTP server.
//!
//! Every text here is synthetic.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use ink_llm::{
    ApiKey, HttpRequest, HttpResponse, KeyStore, KeyStoreError, Transport, TransportError,
};

/// What a [`CountingTransport`] saw of one request.
#[derive(Clone, Debug)]
pub struct Seen {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
    pub loopback_only: bool,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Answers every request with one canned response and records what it was sent.
pub struct CountingTransport {
    pub calls: AtomicUsize,
    pub seen: Mutex<Vec<Seen>>,
    answer: Result<(u16, Vec<u8>), TransportError>,
}

impl CountingTransport {
    pub fn ok(body: &str) -> Self {
        Self::status(200, body)
    }

    pub fn status(status: u16, body: &str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            answer: Ok((status, body.as_bytes().to_vec())),
        }
    }

    pub fn failing(error: TransportError) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            answer: Err(error),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn last(&self) -> Seen {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a request")
    }
}

impl Transport for CountingTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(Seen {
            url: request.url.clone(),
            headers: request
                .headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), v.clone()))
                .collect(),
            body: serde_json::from_slice(&request.body).expect("a JSON body"),
            loopback_only: request.loopback_only,
        });
        self.answer
            .clone()
            .map(|(status, body)| HttpResponse { status, body })
    }
}

/// An in-memory key store that can be told to deny, and counts reads.
#[derive(Default)]
pub struct MemKeys {
    keys: Mutex<HashMap<String, String>>,
    deny: bool,
    pub reads: AtomicUsize,
    pub existence_checks: AtomicUsize,
}

impl MemKeys {
    pub fn with(provider: &str, key: &str) -> Self {
        let keys = Self::default();
        keys.keys
            .lock()
            .unwrap()
            .insert(provider.to_owned(), key.to_owned());
        keys
    }

    /// A keychain that refuses every read, as when the user clicks Deny.
    pub fn denying() -> Self {
        Self {
            deny: true,
            ..Self::default()
        }
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl KeyStore for MemKeys {
    fn has_key(&self, provider: &str) -> Result<bool, KeyStoreError> {
        self.existence_checks.fetch_add(1, Ordering::SeqCst);
        Ok(self.keys.lock().unwrap().contains_key(provider))
    }

    fn read_key(&self, provider: &str) -> Result<ApiKey, KeyStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.deny {
            return Err(KeyStoreError::Denied);
        }
        self.keys
            .lock()
            .unwrap()
            .get(provider)
            .map(|k| ApiKey::new(k.clone()))
            .ok_or(KeyStoreError::NotFound)
    }

    fn save_key(&self, provider: &str, key: &str) -> Result<(), KeyStoreError> {
        self.keys
            .lock()
            .unwrap()
            .insert(provider.to_owned(), key.to_owned());
        Ok(())
    }

    fn delete_key(&self, provider: &str) -> Result<(), KeyStoreError> {
        self.keys.lock().unwrap().remove(provider);
        Ok(())
    }
}

/// How a [`ScriptedLlm`] answers.
type Script = Box<dyn Fn(&LlmRequest) -> Result<String, LlmError> + Send + Sync>;

/// A model whose answer is computed from the request, with its calls counted and recorded.
pub struct ScriptedLlm {
    answer: Script,
    pub calls: AtomicUsize,
    pub requests: Mutex<Vec<LlmRequest>>,
}

impl ScriptedLlm {
    pub fn new(
        answer: impl Fn(&LlmRequest) -> Result<String, LlmError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            answer: Box::new(answer),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Llm for ScriptedLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "scripted".into(),
            model: "test".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        (self.answer)(request).map(|text| LlmResponse { text })
    }
}

/// A request as the loopback server received it.
#[derive(Debug)]
pub struct Received {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A one-shot HTTP server bound to 127.0.0.1 only. It serves `connections` connections, one
/// request each, answering every one with `response` (a full HTTP/1.1 response), and returns
/// the port and a channel of what it received.
pub fn loopback_server(response: String, connections: usize) -> (u16, mpsc::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for _ in 0..connections {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((n, v)) = line.split_once(':') {
                    headers.push((n.trim().to_owned(), v.trim().to_owned()));
                }
            }
            let length = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            let _ = tx.send(Received {
                request_line: request_line.trim_end().to_owned(),
                headers,
                body,
            });
        }
    });
    (port, rx)
}

/// A full HTTP/1.1 response with a JSON body.
pub fn http_response(status: u16, reason: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
