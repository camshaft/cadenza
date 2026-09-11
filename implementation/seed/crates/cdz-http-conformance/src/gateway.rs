//! The driver's gateway HTTP client (`DESIGN-http-outpost-conformance-harness.md` §3.2). A run-spec's
//! `http` steps are REAL HTTP requests at the stock gateway; [`GatewayClient`] makes them and captures the
//! observable response (status + body) the step's [`crate::spec::Expect`] asserts against. Like
//! [`crate::AdminClient`], it holds ONE pooled [`reqwest::Client`] (connections kept alive + reused across a
//! scenario's requests, not one per request).

use crate::spec::HttpRequest;
use bytes::Bytes;
use reqwest::Method;
use std::net::SocketAddr;

/// How long the stalled/slowloris probe waits for the gateway's body-read idle timeout to floor it — set well
/// above any plausible baked idle timeout so a real 408 is observed; if nothing arrives, that's a genuine "no
/// idle timeout" gap surfaced as a clear error, not an indefinite hang.
const STALLED_READ_BUDGET: std::time::Duration = std::time::Duration::from_secs(25);

/// The observable response to a gateway request: the HTTP status, the response headers (lower-cased
/// `(name, value)` pairs, as the wire delivers them), and the full body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

/// The result of a boot readiness probe (see [`GatewayClient::settle_probe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleProbe {
    /// The gateway answered the unconfigured floor (`503 waiting for control`) — control has not configured it.
    Floor,
    /// The gateway is configured: it answered a non-floor response, OR it accepted the connection but did not
    /// respond within the probe budget (it is DRIVING the root router — e.g. a router that blocks awaiting a
    /// control.send / dispatch answer — which is NOT the instant floor, so the config IS applied).
    Configured,
    /// The gateway is not accepting connections yet (still binding) — keep waiting.
    Unreachable,
}

/// A client for one gateway's HTTP surface, holding a pooled [`reqwest::Client`]. Cheap to `Clone`.
#[derive(Debug, Clone)]
pub struct GatewayClient {
    base: String,
    http: reqwest::Client,
}

impl GatewayClient {
    /// A client targeting the gateway serving at `addr` (its real bound `listen` address).
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            base: format!("http://{addr}"),
            // A per-request timeout so a hung SUT (e.g. a gateway whose drive blocks awaiting an answer that
            // never arrives) surfaces as a clear step error rather than hanging the whole harness forever.
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Make one HTTP request at the gateway and capture its response. Errs (a `String`) on a bad method, a
    /// transport failure, or a body read failure — the driver surfaces it as a scenario error (distinct from
    /// an assertion failure, which is a [`crate::spec::Expect`] mismatch on a response that DID arrive).
    ///
    /// # Errors
    /// The request method is not a valid HTTP method, the request cannot be sent, or the body cannot be read.
    pub async fn send(&self, req: &HttpRequest) -> Result<GatewayResponse, String> {
        // A stalled/slowloris client: declare a body but send no bytes + hold open, so the gateway's body-read
        // IDLE timeout floors it (408). reqwest always sends the body it's given, so this path is raw.
        if let Some(declared) = req.stalled_content_length {
            return self.send_stalled_body(req, declared).await;
        }
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| format!("invalid method {:?}: {e}", req.method))?;
        let url = format!("{}{}", self.base, req.path);
        let mut builder = self.http.request(method, &url);
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &req.body {
            builder = builder.body(body.clone());
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| format!("gateway request {} {}: {e}", req.method, req.path))?;
        let status = resp.status().as_u16();
        // Capture the response headers (name lower-cased for case-insensitive assertion; a header whose value
        // is not valid UTF-8 is skipped — the assertions are text). Taken before the body consumes `resp`.
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
            })
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| format!("reading gateway response body: {e}"))?;
        Ok(GatewayResponse {
            status,
            headers,
            body,
        })
    }

    /// Send a stalled/slowloris request: declare `Content-Length: <declared>` (kept UNDER the body ceiling, so
    /// this is the idle path, not the 413 ceiling), send NO body bytes, and hold the connection open — the
    /// gateway's body-read IDLE timeout should floor it (408) rather than block forever. Reads the response with
    /// a generous budget (well above any plausible baked idle timeout) so a real 408 is observed; a hang (no
    /// idle timeout) surfaces as a clear read-timeout error.
    ///
    /// # Errors
    /// The connection / write fails, or no response arrives within [`STALLED_READ_BUDGET`] (the idle timeout did
    /// not fire — a real gap, surfaced as a read-timeout error rather than hanging the harness).
    async fn send_stalled_body(
        &self,
        req: &HttpRequest,
        declared: u64,
    ) -> Result<GatewayResponse, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = self.base.strip_prefix("http://").unwrap_or(&self.base);
        let mut stream = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .map_err(|_| format!("stalled connect to {addr} timed out"))?
        .map_err(|e| format!("stalled connect to {addr}: {e}"))?;
        // Write ONLY the head declaring a body, then send NO body bytes + hold open — a stalled client.
        let head = format!(
            "{} {} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n",
            req.method, req.path
        );
        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|e| format!("stalled write: {e}"))?;
        let _ = stream.flush().await;
        let mut buf = Vec::new();
        tokio::time::timeout(STALLED_READ_BUDGET, stream.read_to_end(&mut buf))
            .await
            .map_err(|_| {
                format!(
                    "stalled read timed out after {STALLED_READ_BUDGET:?}: the gateway did not floor the \
                     no-send client — a body-read idle timeout (→ 408) should have fired"
                )
            })?
            .map_err(|e| format!("stalled read: {e}"))?;
        parse_raw_response(&buf)
    }

    /// Probe the gateway's boot readiness with a short-budget `GET /`, classifying the outcome (see
    /// [`SettleProbe`]). The unconfigured floor answers INSTANTLY with `503 waiting for control`; anything
    /// slower-than-`budget` means the gateway accepted the connection and is DRIVING (configured) — critically,
    /// a root router that blocks awaiting a control.send / dispatch answer would hang the probe, and that is
    /// "configured", not "not ready". A connection error means it is not up yet.
    pub async fn settle_probe(&self, budget: std::time::Duration) -> SettleProbe {
        let url = format!("{}/", self.base);
        match tokio::time::timeout(budget, self.http.get(&url).send()).await {
            Ok(Ok(resp)) => {
                let status = resp.status().as_u16();
                let body = resp.bytes().await.unwrap_or_default();
                let is_floor =
                    status == 503 && String::from_utf8_lossy(&body).contains("waiting for control");
                if is_floor {
                    SettleProbe::Floor
                } else {
                    SettleProbe::Configured
                }
            }
            // A connection error (refused / not yet bound) ⇒ keep waiting; any other transport error post-
            // connect (or our own budget elapsing) ⇒ the gateway is up + driving, i.e. configured.
            Ok(Err(e)) if e.is_connect() => SettleProbe::Unreachable,
            Ok(Err(_)) | Err(_) => SettleProbe::Configured,
        }
    }
}

/// Parse a raw HTTP/1.1 response (status line + headers + body) into a [`GatewayResponse`] — used by the raw
/// stalled-client path, which speaks HTTP directly rather than through `reqwest`.
///
/// # Errors
/// The response has no header/body separator, or its status line is malformed / missing a numeric code.
fn parse_raw_response(raw: &[u8]) -> Result<GatewayResponse, String> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| {
            format!(
                "raw response has no header/body separator ({} bytes: {:?})",
                raw.len(),
                String::from_utf8_lossy(&raw[..raw.len().min(80)])
            )
        })?;
    let head = String::from_utf8_lossy(&raw[..sep]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    // Status line: `HTTP/1.1 <code> <reason>`; the code is the 2nd whitespace token.
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| format!("raw response has a malformed status line: {status_line:?}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let body = Bytes::copy_from_slice(&raw[sep + 4..]);
    Ok(GatewayResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_response_reads_status_headers_and_body() {
        let raw = b"HTTP/1.1 408 Request Timeout\r\nContent-Type: text/plain\r\nContent-Length: 3\r\n\r\nbye";
        let r = parse_raw_response(raw).expect("parses");
        assert_eq!(r.status, 408);
        assert_eq!(r.body.as_ref(), b"bye");
        assert!(
            r.headers
                .iter()
                .any(|(n, v)| n == "content-type" && v == "text/plain")
        );
        // A malformed response (no separator) is a clear error, not a panic.
        assert!(parse_raw_response(b"HTTP/1.1 200 OK").is_err());
    }
    use crate::spec::Expect;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A minimal one-shot HTTP/1.1 stub: bind an ephemeral port, accept ONE connection, read the request
    /// (drained to the header terminator), and write a canned `status`/`body` response, then close. Enough
    /// to exercise the reqwest client without pulling a server framework into this crate. Returns the addr.
    async fn http_stub(status_line: &'static str, body: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Drain the request headers (read until the CRLFCRLF terminator or the peer stops).
            let mut buf = [0u8; 1024];
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 || buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
            let _ = sock.flush().await;
        });
        addr
    }

    #[tokio::test]
    async fn send_captures_status_and_body() {
        let addr = http_stub("200 OK", b"hello from a wasm handler").await;
        let client = GatewayClient::new(addr);
        let resp = client
            .send(&HttpRequest {
                method: "GET".into(),
                path: "/".into(),
                ..Default::default()
            })
            .await
            .expect("request succeeds");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body.as_ref(), b"hello from a wasm handler");
        // Response headers are captured (lower-cased); the stub always sends Content-Length.
        assert!(
            resp.headers.iter().any(|(n, _)| n == "content-length"),
            "expected content-length in captured headers: {:?}",
            resp.headers
        );
    }

    #[tokio::test]
    async fn a_non_200_status_is_captured_not_an_error() {
        let addr = http_stub("404 Not Found", b"").await;
        let client = GatewayClient::new(addr);
        let resp = client
            .send(&HttpRequest {
                method: "GET".into(),
                path: "/nope".into(),
                ..Default::default()
            })
            .await
            .expect("request succeeds (a 404 is a captured response, not a transport error)");
        assert_eq!(resp.status, 404);
        assert!(resp.body.is_empty());
    }

    #[tokio::test]
    async fn captured_response_checks_against_expect() {
        let addr = http_stub("200 OK", b"hello from a wasm handler").await;
        let client = GatewayClient::new(addr);
        let resp = client
            .send(&HttpRequest {
                method: "GET".into(),
                path: "/".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        // The end-to-end shape the run-loop uses: capture → assert.
        let ok = Expect {
            status: Some(200),
            body: Some(b"hello from a wasm handler".to_vec()),
            body_contains: Some("wasm".into()),
            ..Default::default()
        };
        assert!(ok.check(resp.status, &resp.headers, &resp.body).is_ok());
        let wrong = Expect {
            status: Some(500),
            ..Expect::default()
        };
        assert!(wrong.check(resp.status, &resp.headers, &resp.body).is_err());
    }

    #[tokio::test]
    async fn send_applies_request_headers_and_body() {
        // A capture stub: record the raw request bytes, reply 200, hand the request back.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    sock.read(&mut tmp),
                )
                .await
                {
                    Ok(Ok(0)) | Err(_) => break,
                    Ok(Ok(n)) => buf.extend_from_slice(&tmp[..n]),
                    Ok(Err(_)) => break,
                }
            }
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            let _ = tx.send(buf);
        });
        GatewayClient::new(addr)
            .send(&HttpRequest {
                method: "POST".into(),
                path: "/echo".into(),
                headers: vec![("x-test".into(), "v".into())],
                body: Some(b"payload".to_vec()),
                ..Default::default()
            })
            .await
            .expect("request succeeds");
        let req = String::from_utf8_lossy(&rx.await.unwrap()).to_ascii_lowercase();
        assert!(req.starts_with("post /echo "), "request line: {req:?}");
        assert!(req.contains("x-test: v"), "missing header: {req:?}");
        assert!(req.ends_with("payload"), "body not sent: {req:?}");
    }

    #[tokio::test]
    async fn settle_probe_classifies_floor_configured_and_unreachable() {
        use std::time::Duration;
        // The unconfigured floor (503 + the waiting-for-control body) → Floor.
        let floor = http_stub(
            "503 Service Unavailable",
            b"cdz-http-gateway: waiting for control (no program configured yet)\n",
        )
        .await;
        assert_eq!(
            GatewayClient::new(floor)
                .settle_probe(Duration::from_secs(2))
                .await,
            SettleProbe::Floor
        );
        // A normal 200 → Configured.
        let ok = http_stub("200 OK", b"hi").await;
        assert_eq!(
            GatewayClient::new(ok)
                .settle_probe(Duration::from_secs(2))
                .await,
            SettleProbe::Configured
        );
        // A configured router's OWN 503 (different body) → Configured (not the floor).
        let router_503 = http_stub("503 Service Unavailable", b"upstream busy").await;
        assert_eq!(
            GatewayClient::new(router_503)
                .settle_probe(Duration::from_secs(2))
                .await,
            SettleProbe::Configured
        );
        // Nothing listening → Unreachable (keep waiting).
        assert_eq!(
            GatewayClient::new("127.0.0.1:1".parse().unwrap())
                .settle_probe(Duration::from_millis(300))
                .await,
            SettleProbe::Unreachable
        );
    }

    #[tokio::test]
    async fn settle_probe_treats_a_hanging_gateway_as_configured() {
        use std::time::Duration;
        // A stub that accepts the connection but NEVER responds (like a root router blocking on a control.send
        // whose reply never comes) → the probe budget elapses → Configured (it is driving, not the floor).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await; // hold the connection open, never reply
        });
        assert_eq!(
            GatewayClient::new(addr)
                .settle_probe(Duration::from_millis(300))
                .await,
            SettleProbe::Configured
        );
    }

    #[tokio::test]
    async fn a_bad_method_is_a_clear_error() {
        let client = GatewayClient::new("127.0.0.1:1".parse().unwrap());
        let err = client
            .send(&HttpRequest {
                method: "not a method".into(),
                path: "/".into(),
                ..Default::default()
            })
            .await
            .expect_err("an invalid method errs before any request");
        assert!(err.contains("invalid method"), "got: {err}");
    }
}
