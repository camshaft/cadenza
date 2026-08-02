//! The `Http` executor — make an HTTP request and fold the response back.
//!
//! An agent that fetches a URL emits an `Http` effect; the kernel authorizes it (the host is gated by a
//! SEC-F1 `HostIn` capability — the SSRF/exfil guard), durably dispatches it, this executor performs the
//! request, and the response body folds back as the result. The effect's `target` is the URL; its
//! `payload` is the optional request body (a POST/PUT body — `None` is a GET); the response body becomes
//! the result payload.
//!
//! **Transport seam** (identical shape to [`crate::model`]): the real request touches the network, so
//! this executor is GENERIC over an [`HttpTransport`]. The executor owns the pure, hermetically-testable
//! effect mapping (kind check, method/body derivation, outcome mapping); the transport owns the I/O (a
//! real client — behind the crate's `live-net` feature). A stub transport drives the hermetic tests +
//! the end-to-end agent-loop test.
//!
//! **Bytes-first** (operator perf directive): both the request body and the response body are
//! [`Bytes`] — a fetched body is a hot-path buffer folded into the log/KV, so a ref-counted clone beats
//! a deep copy. **Trust boundary:** the kernel already gated the resolved URL's host (SEC-F1) before
//! dispatch; this executor does not re-authorize.

use bytes::Bytes;
use cdz_kernel::effect::{EffectKind, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The I/O half of an HTTP request, factored out so the executor's logic is hermetically testable. An
/// impl performs the real request (behind `live-net`); a stub returns canned bytes for tests. Total: it
/// returns `Err(reason)` rather than panicking, so a transport failure folds as an observable
/// `EffectOutcome::Err` (§9d/§17).
///
/// `body` is `None` for a bodyless request (a GET) and `Some(bytes)` for a request with a body (a POST/
/// PUT). The response body is returned as [`Bytes`] (the hot-path buffer). The `idempotency_key` lets a
/// side-effecting transport dedup a crash-re-driven request (§16c-S1/D) — relevant for a non-idempotent
/// method; a GET is naturally idempotent and can ignore it.
pub trait HttpTransport {
    fn request(
        &self,
        url: &str,
        body: Option<&[u8]>,
        idempotency_key: Hash,
    ) -> Result<Bytes, String>;
}

/// Performs `Http` effects by delegating the request to an [`HttpTransport`]. Single-KIND: a non-`Http`
/// kind is an observable `Err` (§9d) — register it under `Http` in a
/// [`cdz_kernel::executor::CompositeExecutor`] alongside the other real executors.
pub struct HttpExecutor<T: HttpTransport> {
    transport: T,
}

impl<T: HttpTransport> HttpExecutor<T> {
    pub fn new(transport: T) -> Self {
        HttpExecutor { transport }
    }
}

impl<T: HttpTransport> Executor for HttpExecutor<T> {
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        if req.kind != EffectKind::Http {
            return EffectOutcome::Err(format!(
                "HttpExecutor only handles Http effects, got {:?}",
                req.kind
            ));
        }
        // The request body: an inline payload IS the body (a POST/PUT); no payload is a bodyless request
        // (a GET). A blob-ref body isn't supported yet (the transport would fetch it from the blob store
        // — a later slice); surface it as an observable Err, never a panic (§17).
        let body: Option<&[u8]> = match &req.payload {
            // `&bytes[..]` borrows the payload as a slice — works whether `Inline` holds `Vec<u8>` or
            // (post-flip) `bytes::Bytes` (both `Deref<Target = [u8]>`; `Bytes` has no `as_slice`).
            Some(Payload::Inline(bytes)) => Some(&bytes[..]),
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(
                    "HttpExecutor: blob-ref request body not supported yet (inline it)".to_string(),
                );
            }
            None => None,
        };
        match self.transport.request(&req.target, body, idempotency_key) {
            // The transport's `Bytes` response body moves straight into `Payload::Inline` (ref-counted
            // `Bytes` after the kernel's perf-directive flip) — no copy, no conversion.
            Ok(response) => EffectOutcome::Ok(Some(Payload::Inline(response))),
            Err(reason) => EffectOutcome::Err(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub transport that asserts what it was asked to fetch and returns a canned response body.
    struct StubHttp {
        expect_body: Option<Vec<u8>>,
        response: Bytes,
    }
    impl HttpTransport for StubHttp {
        fn request(&self, url: &str, body: Option<&[u8]>, _key: Hash) -> Result<Bytes, String> {
            assert_eq!(url, "https://ok.host/x");
            assert_eq!(body.map(|b| b.to_vec()), self.expect_body);
            Ok(self.response.clone()) // Bytes clone = O(1) refcount bump, not a deep copy
        }
    }

    fn http_req(payload: Option<Payload>) -> EffectRequest {
        EffectRequest {
            kind: EffectKind::Http,
            target: "https://ok.host/x".to_string(),
            payload,
        }
    }

    #[test]
    fn a_get_has_no_body_and_returns_the_response() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_body: None,
            response: Bytes::from_static(b"response body"),
        });
        match exec.perform(&http_req(None), Hash::of(b"k")) {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(&bytes[..], b"response body")
            }
            other => panic!("expected Ok(Inline(response)), got {other:?}"),
        }
    }

    #[test]
    fn an_inline_payload_is_the_request_body() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_body: Some(b"post this".to_vec()),
            response: Bytes::from_static(b"ok"),
        });
        // The stub asserts it received exactly the inline payload as the request body.
        match exec.perform(
            &http_req(Some(Payload::Inline(b"post this".to_vec().into()))),
            Hash::of(b"k"),
        ) {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => assert_eq!(&bytes[..], b"ok"),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn a_blob_body_is_an_observable_err_for_now() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_body: None,
            response: Bytes::new(),
        });
        match exec.perform(
            &http_req(Some(Payload::Blob(Hash::of(b"big")))),
            Hash::of(b"k"),
        ) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("blob-ref request body not supported"), "{msg}")
            }
            other => panic!("expected Err for a blob-ref body, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_failure_folds_as_an_err_not_a_panic() {
        struct FailingHttp;
        impl HttpTransport for FailingHttp {
            fn request(&self, _u: &str, _b: Option<&[u8]>, _k: Hash) -> Result<Bytes, String> {
                Err("connection refused".to_string())
            }
        }
        let mut exec = HttpExecutor::new(FailingHttp);
        match exec.perform(&http_req(None), Hash::of(b"k")) {
            EffectOutcome::Err(msg) => assert!(msg.contains("connection refused"), "{msg}"),
            other => panic!("expected the transport error to fold as Err, got {other:?}"),
        }
    }

    #[test]
    fn non_http_kind_is_an_observable_err() {
        struct NeverCalled;
        impl HttpTransport for NeverCalled {
            fn request(&self, _u: &str, _b: Option<&[u8]>, _k: Hash) -> Result<Bytes, String> {
                panic!("transport must not be called for a non-Http kind");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let req = EffectRequest {
            kind: EffectKind::Model,
            target: "m".to_string(),
            payload: None,
        };
        match exec.perform(&req, Hash::of(b"k")) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("Http"), "err names the handled kind: {msg}")
            }
            other => panic!("expected Err for a non-Http kind, got {other:?}"),
        }
    }
}
