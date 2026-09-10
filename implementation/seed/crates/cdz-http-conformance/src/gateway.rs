//! The driver's gateway HTTP client (`DESIGN-http-outpost-conformance-harness.md` §3.2). A run-spec's
//! `http` steps are REAL HTTP requests at the stock gateway; [`GatewayClient`] makes them and captures the
//! observable response (status + body) the step's [`crate::spec::Expect`] asserts against. Like
//! [`crate::AdminClient`], it holds ONE pooled [`reqwest::Client`] (connections kept alive + reused across a
//! scenario's requests, not one per request).

use crate::spec::HttpRequest;
use bytes::Bytes;
use reqwest::Method;
use std::net::SocketAddr;

/// The observable response to a gateway request: the HTTP status and the full body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub body: Bytes,
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
            http: reqwest::Client::new(),
        }
    }

    /// Make one HTTP request at the gateway and capture its response. Errs (a `String`) on a bad method, a
    /// transport failure, or a body read failure — the driver surfaces it as a scenario error (distinct from
    /// an assertion failure, which is a [`crate::spec::Expect`] mismatch on a response that DID arrive).
    ///
    /// # Errors
    /// The request method is not a valid HTTP method, the request cannot be sent, or the body cannot be read.
    pub async fn send(&self, req: &HttpRequest) -> Result<GatewayResponse, String> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| format!("invalid method {:?}: {e}", req.method))?;
        let url = format!("{}{}", self.base, req.path);
        let resp = self
            .http
            .request(method, &url)
            .send()
            .await
            .map_err(|e| format!("gateway request {} {}: {e}", req.method, req.path))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| format!("reading gateway response body: {e}"))?;
        Ok(GatewayResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            })
            .await
            .expect("request succeeds");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body.as_ref(), b"hello from a wasm handler");
    }

    #[tokio::test]
    async fn a_non_200_status_is_captured_not_an_error() {
        let addr = http_stub("404 Not Found", b"").await;
        let client = GatewayClient::new(addr);
        let resp = client
            .send(&HttpRequest {
                method: "GET".into(),
                path: "/nope".into(),
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
            })
            .await
            .unwrap();
        // The end-to-end shape the run-loop uses: capture → assert.
        let ok = Expect {
            status: Some(200),
            body: Some(b"hello from a wasm handler".to_vec()),
            body_contains: Some("wasm".into()),
        };
        assert!(ok.check(resp.status, &resp.body).is_ok());
        let wrong = Expect {
            status: Some(500),
            ..Expect::default()
        };
        assert!(wrong.check(resp.status, &resp.body).is_err());
    }

    #[tokio::test]
    async fn a_bad_method_is_a_clear_error() {
        let client = GatewayClient::new("127.0.0.1:1".parse().unwrap());
        let err = client
            .send(&HttpRequest {
                method: "not a method".into(),
                path: "/".into(),
            })
            .await
            .expect_err("an invalid method errs before any request");
        assert!(err.contains("invalid method"), "got: {err}");
    }
}
