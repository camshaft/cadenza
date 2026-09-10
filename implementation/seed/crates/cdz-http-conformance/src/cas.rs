//! The driver's CAS seeding client (`DESIGN-http-outpost-conformance-harness.md` §3.2). Before a scenario
//! runs, the driver publishes each program's compiled component into the running `cdz-cas-http` store so the
//! gateway can fetch it by hash; [`CasClient`] is the deploy-side write path (`PUT /{hash}`) plus a `GET` to
//! read one back (used to verify seeding / in assertions). Loopback HTTP only — a pooled `reqwest::Client`
//! with `default-features = false` (no TLS provider; the CAS crate's own `HttpBlobStore` carries the aws-lc-rs
//! TLS stack, which is dead weight — and a `cmake` build — for a loopback harness, so the driver uses its own
//! thin client rather than depending on that crate).
//!
//! The CAS keys blobs by the base62 [`Hash`] text in the path (`/{hash}`) and validates on `PUT` that the
//! body's digest matches the key; it keys on the DIGEST tag-agnostically, so a component published under its
//! `Program` hash text is fetched equally by that hash. The driver passes the hash as its base62 text (what
//! the nix harness rig computes for each compiled program).

use bytes::Bytes;
use std::net::SocketAddr;

/// A client for one CAS store's HTTP surface: publish blobs by hash (`PUT`) + read them back (`GET`). Holds
/// ONE pooled [`reqwest::Client`] (connections reused across a scenario's seeds). Cheap to `Clone`.
#[derive(Debug, Clone)]
pub struct CasClient {
    base: String,
    http: reqwest::Client,
    write_credential: Option<String>,
}

impl CasClient {
    /// A client targeting the CAS serving at `addr`, with no write credential (reads only; a `put` will be
    /// rejected `401` by a CAS that requires one — add it with [`with_write_credential`]).
    ///
    /// [`with_write_credential`]: CasClient::with_write_credential
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            base: format!("http://{addr}"),
            http: reqwest::Client::new(),
            write_credential: None,
        }
    }

    /// Present `Authorization: Bearer {credential}` on writes (`PUT`), matching the CAS's
    /// `CDZ_CAS_WRITE_CREDENTIAL` — required for the write path (else the CAS answers `401`).
    #[must_use]
    pub fn with_write_credential(mut self, credential: impl Into<String>) -> Self {
        self.write_credential = Some(credential.into());
        self
    }

    /// Publish `bytes` under `hash` (its base62 text) — `PUT /{hash}`. The CAS validates the body's digest
    /// against the key, so a mismatched (hash, bytes) is a `400` surfaced here as an error.
    ///
    /// # Errors
    /// The request cannot be sent, or the CAS answers a non-2xx status (a `400` hash mismatch, a `401`
    /// missing/wrong write credential, a `500` store failure).
    pub async fn put(&self, hash: &str, bytes: Bytes) -> Result<(), String> {
        let url = format!("{}/{hash}", self.base);
        let mut req = self.http.put(&url).body(bytes);
        if let Some(cred) = &self.write_credential {
            req = req.bearer_auth(cred);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("CAS PUT {hash}: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(format!("CAS PUT {hash}: status {status}"))
        }
    }

    /// Fetch the blob at `hash` — `GET /{hash}`. `Ok(Some(bytes))` on `200`, `Ok(None)` on `404` (a genuine
    /// miss), `Err` on any other status or a transport failure.
    ///
    /// # Errors
    /// The request cannot be sent, the body cannot be read, or the CAS answers a status other than
    /// `200`/`404` (e.g. `401` when a read credential is required).
    pub async fn get(&self, hash: &str) -> Result<Option<Bytes>, String> {
        let url = format!("{}/{hash}", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("CAS GET {hash}: {e}"))?;
        match resp.status().as_u16() {
            200 => {
                Ok(Some(resp.bytes().await.map_err(|e| {
                    format!("CAS GET {hash}: reading body: {e}")
                })?))
            }
            404 => Ok(None),
            other => Err(format!("CAS GET {hash}: status {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// Read a whole HTTP request off `sock` without parsing it: accumulate bytes until the peer goes idle
    /// (a short read-timeout with no data ⇒ the request is fully delivered). Enough to capture + assert a
    /// request's raw shape, or to just drain it before replying.
    async fn drain_request(sock: &mut TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            match tokio::time::timeout(Duration::from_millis(50), sock.read(&mut tmp)).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => buf.extend_from_slice(&tmp[..n]),
                Ok(Err(_)) => break,
            }
        }
        buf
    }

    /// A one-shot stub that captures the raw request bytes, replies `status_line` + `body`, and hands the
    /// captured request back over a channel so the test can assert what the client sent.
    async fn capture_stub(
        status_line: &'static str,
        body: &'static [u8],
    ) -> (SocketAddr, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let req = drain_request(&mut sock).await;
            let resp = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.write_all(body).await;
            let _ = sock.flush().await;
            let _ = tx.send(req);
        });
        (addr, rx)
    }

    #[tokio::test]
    async fn put_sends_put_hash_with_bearer_and_body() {
        let (addr, rx) = capture_stub("201 Created", b"").await;
        let client = CasClient::new(addr).with_write_credential("seed-cred");
        client
            .put("Abc123hashtext", Bytes::from_static(b"wasm-bytes-here"))
            .await
            .expect("put succeeds on 201");
        let req = String::from_utf8_lossy(&rx.await.unwrap()).to_string();
        assert!(
            req.starts_with("PUT /Abc123hashtext "),
            "request line: {req:?}"
        );
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer seed-cred"),
            "missing bearer: {req:?}"
        );
        assert!(req.ends_with("wasm-bytes-here"), "body not sent: {req:?}");
    }

    #[tokio::test]
    async fn put_without_credential_sends_no_authorization_header() {
        let (addr, rx) = capture_stub("201 Created", b"").await;
        CasClient::new(addr)
            .put("H", Bytes::from_static(b"x"))
            .await
            .expect("put ok");
        let req = String::from_utf8_lossy(&rx.await.unwrap()).to_ascii_lowercase();
        assert!(
            !req.contains("authorization:"),
            "unexpected auth header: {req:?}"
        );
    }

    #[tokio::test]
    async fn a_non_2xx_put_is_an_error() {
        let (addr, _rx) = capture_stub("400 Bad Request", b"hash mismatch").await;
        let err = CasClient::new(addr)
            .with_write_credential("c")
            .put("H", Bytes::from_static(b"x"))
            .await
            .expect_err("400 is an error");
        assert!(err.contains("400"), "got: {err}");
    }

    #[tokio::test]
    async fn get_returns_bytes_on_200_none_on_404() {
        let (hit, _r1) = capture_stub("200 OK", b"the blob").await;
        assert_eq!(
            CasClient::new(hit).get("H").await.unwrap().as_deref(),
            Some(&b"the blob"[..])
        );
        let (miss, _r2) = capture_stub("404 Not Found", b"").await;
        assert_eq!(CasClient::new(miss).get("H").await.unwrap(), None);
    }
}
