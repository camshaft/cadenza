//! The `Http` executor — make an HTTP request and fold the response back.
//!
//! An agent that fetches a URL emits an `Http` effect; the kernel authorizes it (the host is gated by a
//! SEC-F1 `HostIn` capability — the SSRF/exfil guard), durably dispatches it, this executor performs the
//! request, and the response body folds back as the result. The effect's `target` is the URL; its
//! `payload` is a `(http-request (method <name>) (body <opt>))` binary-sexpr (the kernel's
//! [`cdz_kernel::event_ast::encode_http_request`] wire convention, §9b): the METHOD is CALLER-SPECIFIED
//! (never inferred from body presence — a bodyless POST and a DELETE-with-body are both expressible), and
//! the body is independent of the method. The response body becomes the result payload.
//!
//! **Transport seam** (identical shape to [`crate::model`]): the real request touches the network, so
//! this executor is GENERIC over an [`HttpTransport`]. The executor owns the pure, hermetically-testable
//! effect mapping (family check, payload DECODE, method mapping, outcome mapping); the transport owns the
//! I/O (a real client — behind the crate's `live-net` feature). A stub transport drives the hermetic
//! tests + the end-to-end agent-loop test.
//!
//! **Bytes-first** (operator perf directive): both the request body and the response body are
//! [`Bytes`] — a fetched body is a hot-path buffer folded into the log/KV, so a ref-counted clone beats
//! a deep copy. **Trust boundary:** the kernel already gated the resolved URL's host (SEC-F1) before
//! dispatch; this executor does not re-authorize.

use crate::retry;
use bytes::Bytes;
use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::event_ast::decode_http_request;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The HTTP method an `Http` effect carries — CALLER-SPECIFIED, never inferred from body presence (so
/// POST vs PUT vs DELETE are distinguishable, a bodyless POST and a DELETE-with-body are both
/// expressible). An OPEN sum: an unknown/extension method (`PROPFIND`, …) survives verbatim as
/// [`HttpMethod::Other`] rather than being rejected — the kernel codec returns the method name as a
/// string and does not reject unknowns (tolerant-reader posture).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    /// Any method the codec surfaced that isn't a well-known one above — carried through verbatim.
    Other(String),
}

impl HttpMethod {
    /// Map a decoded method name (the kernel returns it lowercase) to the enum. Case-insensitive; an
    /// unrecognized name is preserved as [`HttpMethod::Other`] (open sum — never rejected here).
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "get" => HttpMethod::Get,
            "post" => HttpMethod::Post,
            "put" => HttpMethod::Put,
            "delete" => HttpMethod::Delete,
            "patch" => HttpMethod::Patch,
            "head" => HttpMethod::Head,
            "options" => HttpMethod::Options,
            _ => HttpMethod::Other(name.to_string()),
        }
    }

    /// The uppercase wire name a transport puts on the request line.
    pub fn as_str(&self) -> &str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Other(s) => s,
        }
    }
}

/// The I/O half of an HTTP request, factored out so the executor's logic is hermetically testable. An
/// impl performs the real request (behind `live-net`); a stub returns canned bytes for tests. Total: it
/// returns `Err(reason)` rather than panicking, so a transport failure folds as an observable
/// `EffectOutcome::Err` (§9d/§17).
///
/// `method` is the caller-specified [`HttpMethod`] (the transport puts it on the request line — no
/// inference). `body` is `None` for a bodyless request and `Some(bytes)` for one with a body; the two are
/// INDEPENDENT (any method may carry or omit a body). The response body is returned as [`Bytes`] (the
/// hot-path buffer). The `idempotency_key` lets a side-effecting transport dedup a crash-re-driven request
/// (§16c-S1/D) — relevant for a non-idempotent method.
///
/// **Error classification (supervision, [`crate::retry`]):** an `Err(reason)` MUST lead with a
/// retryability token — a transient failure (5xx, timeout, connection reset) as
/// [`crate::retry::retryable`], a permanent one (4xx client error, DNS failure, malformed URL) as
/// [`crate::retry::permanent`]. Unprefixed reasons are treated PERMANENT (fail-closed).
/// `#[async_trait(?Send)]` — a real HTTP request awaits the socket; not `Send` (single-threaded host).
#[async_trait::async_trait(?Send)]
pub trait HttpTransport {
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        body: Option<&[u8]>,
        idempotency_key: Hash,
    ) -> Result<Bytes, String>;
}

/// Performs `Http` effects by delegating the request to an [`HttpTransport`]. Single-family: a non-`Http`
/// family is an observable `Err` (§9d) — register it under `Http` in a
/// [`cdz_kernel::executor::CompositeExecutor`] alongside the other real executors.
pub struct HttpExecutor<T: HttpTransport> {
    transport: T,
}

impl<T: HttpTransport> HttpExecutor<T> {
    pub fn new(transport: T) -> Self {
        HttpExecutor { transport }
    }
}

#[async_trait::async_trait(?Send)]
impl<T: HttpTransport> Executor for HttpExecutor<T> {
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        // Family-keyed (seq-39), not the EffectKind enum — the same decision the router + authz make.
        if !req.content_type.matches_family(effect_ct::HTTP) {
            // Structural — PERMANENT, a supervisor must not retry it (§17: observable Err, never a panic).
            return EffectOutcome::Err(retry::permanent(format!(
                "HttpExecutor only handles the {} family, got {}",
                effect_ct::HTTP,
                req.content_type.family
            )));
        }
        // The payload is a `(http-request (method <name>) (body <opt>))` binary-sexpr — decode the
        // caller-specified method + body (NO body-presence heuristic). A blob-ref payload can't be decoded
        // (no blob-store handle) and a missing/garbage payload is malformed; both are structural → PERMANENT.
        let (method, body): (HttpMethod, Option<Vec<u8>>) = match &req.payload {
            Some(Payload::Inline(bytes)) => match decode_http_request(bytes) {
                Ok((method_name, body)) => (HttpMethod::from_name(&method_name), body),
                Err(e) => {
                    return EffectOutcome::Err(retry::permanent(format!(
                        "HttpExecutor: malformed http-request payload: {e:?}"
                    )));
                }
            },
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(retry::permanent(
                    "HttpExecutor: blob-ref request payload unsupported — this executor has no blob-store access; inline the http-request",
                ));
            }
            None => {
                return EffectOutcome::Err(retry::permanent(
                    "HttpExecutor: an Http effect requires an http-request payload (method + optional body)",
                ));
            }
        };
        match self
            .transport
            .request(method, &req.target, body.as_deref(), idempotency_key)
            .await
        {
            // The transport's `Bytes` response body moves straight into `Payload::Inline` (ref-counted
            // `Bytes`) — no copy. The transport's Err reason already carries its retryability token, so
            // pass it through unchanged.
            Ok(response) => EffectOutcome::Ok(Some(Payload::Inline(response))),
            Err(reason) => EffectOutcome::Err(reason),
        }
    }

    /// This single-family executor serves exactly the `Http` family — the capability-manifest mechanism
    /// dimension when it's used bare as a `dyn Executor` (in a `CompositeExecutor` the composite's own
    /// `by_family` override answers instead). Overrides the trait's fail-safe `false` default.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::HTTP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event_ast::encode_http_request;

    /// A stub transport that asserts the method + body it was asked to perform and returns a canned
    /// response. It records the exact method the executor decoded + passed through.
    struct StubHttp {
        expect_method: HttpMethod,
        expect_body: Option<Vec<u8>>,
        response: Bytes,
    }
    #[async_trait::async_trait(?Send)]
    impl HttpTransport for StubHttp {
        async fn request(
            &self,
            method: HttpMethod,
            url: &str,
            body: Option<&[u8]>,
            _key: Hash,
        ) -> Result<Bytes, String> {
            assert_eq!(url, "https://ok.host/x");
            assert_eq!(method, self.expect_method, "method threaded through verbatim");
            assert_eq!(body.map(|b| b.to_vec()), self.expect_body);
            Ok(self.response.clone()) // Bytes clone = O(1) refcount bump, not a deep copy
        }
    }

    /// Build an Http effect whose payload is the `(http-request …)` binary-sexpr the kernel codec writes.
    fn http_req(method: &str, body: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            Some(Payload::Inline(encode_http_request(method, body).into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn a_get_carries_no_body_and_returns_the_response() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Get,
            expect_body: None,
            response: Bytes::from_static(b"response body"),
        });
        match exec
            .perform_async(&http_req("get", None), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(&bytes[..], b"response body")
            }
            other => panic!("expected Ok(Inline(response)), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_explicit_method_and_body_thread_through() {
        // POST with a body: the caller-specified method reaches the transport, body independent of it.
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Post,
            expect_body: Some(b"post this".to_vec()),
            response: Bytes::from_static(b"ok"),
        });
        match exec
            .perform_async(&http_req("post", Some(b"post this")), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => assert_eq!(&bytes[..], b"ok"),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_bodyless_post_and_a_delete_with_body_are_both_expressible() {
        // The whole point of the explicit method: the two cases the body-presence heuristic couldn't do.
        // Bodyless POST:
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Post,
            expect_body: None,
            response: Bytes::from_static(b"a"),
        });
        assert!(matches!(
            exec.perform_async(&http_req("post", None), Hash::of(b"k")).await,
            EffectOutcome::Ok(_)
        ));
        // DELETE carrying a body:
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Delete,
            expect_body: Some(b"why".to_vec()),
            response: Bytes::from_static(b"b"),
        });
        assert!(matches!(
            exec.perform_async(&http_req("delete", Some(b"why")), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(_)
        ));
    }

    #[tokio::test]
    async fn an_unknown_method_survives_verbatim_as_other() {
        // Open sum: an extension method (PROPFIND) is carried through, not rejected.
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Other("propfind".to_string()),
            expect_body: None,
            response: Bytes::from_static(b"x"),
        });
        assert!(matches!(
            exec.perform_async(&http_req("propfind", None), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(_)
        ));
    }

    #[tokio::test]
    async fn a_blob_payload_is_a_permanent_err_no_blob_store_access() {
        struct NeverCalled;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for NeverCalled {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<Bytes, String> {
                panic!("transport must not be called for a blob-ref payload");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            Some(Payload::Blob(Hash::of(b"big"))),
            Timeliness::Interactive,
        );
        match exec.perform_async(&req, Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("no blob-store access"), "{msg}");
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
            }
            other => panic!("expected Err for a blob-ref payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_or_malformed_payload_is_a_permanent_err() {
        struct NeverCalled;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for NeverCalled {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<Bytes, String> {
                panic!("transport must not be called for a malformed request");
            }
        }
        // No payload → PERMANENT (an Http effect requires an http-request payload).
        let mut exec = HttpExecutor::new(NeverCalled);
        let no_payload = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            None,
            Timeliness::Interactive,
        );
        match exec.perform_async(&no_payload, Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("http-request payload"), "{msg}");
                assert_eq!(retry::classify(&msg), retry::Retryability::Permanent, "{msg}");
            }
            other => panic!("expected Err for a missing payload, got {other:?}"),
        }
        // Garbage (non-http-request) inline payload → PERMANENT (malformed), never a panic (decode is total).
        let mut exec = HttpExecutor::new(NeverCalled);
        let garbage = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            Some(Payload::Inline(b"not a sexpr".to_vec().into())),
            Timeliness::Interactive,
        );
        match exec.perform_async(&garbage, Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("malformed"), "{msg}");
                assert_eq!(retry::classify(&msg), retry::Retryability::Permanent, "{msg}");
            }
            other => panic!("expected Err for a garbage payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_transient_transport_failure_stays_retryable_through_the_executor() {
        // A transient transport failure (connection reset) carries RETRYABLE; the executor passes it
        // through, so it reaches the supervisor still classified retryable.
        struct FlakyHttp;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for FlakyHttp {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<Bytes, String> {
                Err(retry::retryable("connection refused"))
            }
        }
        let mut exec = HttpExecutor::new(FlakyHttp);
        match exec
            .perform_async(&http_req("get", None), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("connection refused"), "{msg}");
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Retryable,
                    "{msg}"
                );
            }
            other => panic!("expected the transport error to fold as Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_http_family_is_a_permanent_err() {
        struct NeverCalled;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for NeverCalled {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<Bytes, String> {
                panic!("transport must not be called for a non-http-family effect");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let req = EffectRequest::new_with_family(
            effect_ct::MODEL,
            "m".to_string(),
            None,
            Timeliness::Interactive,
        );
        match exec.perform_async(&req, Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(
                    msg.contains(effect_ct::HTTP) && msg.contains(effect_ct::MODEL),
                    "err names the handled (http) + rejected (model) families: {msg}"
                );
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
            }
            other => panic!("expected Err for a non-http-family effect, got {other:?}"),
        }
    }

    #[test]
    fn handles_only_the_http_family() {
        // Bare-leaf mechanism dimension: serves Http, nothing else (the trait default false otherwise).
        let exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Get,
            expect_body: None,
            response: Bytes::from_static(b""),
        });
        assert!(exec.handles_family(effect_ct::HTTP));
        assert!(!exec.handles_family(effect_ct::NOW));
        assert!(!exec.handles_family(effect_ct::MODEL));
        assert!(!exec.handles_family("embedding"));
    }
}
