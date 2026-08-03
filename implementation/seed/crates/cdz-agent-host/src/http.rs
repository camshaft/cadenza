//! The `Http` executor — make an HTTP request and fold the response back.
//!
//! An agent that fetches a URL emits an `Http` effect; the kernel authorizes it (the host is gated by a
//! SEC-F1 `HostIn` capability — the SSRF/exfil guard), durably dispatches it, this executor performs the
//! request, and the response folds back as the result. The effect's `target` is the URL; its `payload` is
//! a `(http-request (method <name>) (headers ((k v)…)) (body <opt>))` binary-sexpr (the kernel's
//! [`cdz_kernel::event_ast::encode_http_request`] wire convention, §9b): the METHOD is CALLER-SPECIFIED
//! (never inferred from body presence — a bodyless POST and a DELETE-with-body are both expressible), the
//! request HEADERS are caller-supplied, and the body is independent of the method. The result is a
//! `(http-response (status)(headers)(body))` binary-sexpr — STATUS + response HEADERS + BODY — so a
//! reducer can read the status code and response headers, not just the body.
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
use cdz_kernel::event_ast::{decode_http_request, encode_http_response};
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
    /// Map a decoded method name to the enum. `decode_http_request` returns the method name AS ENCODED
    /// (case preserved — the kernel does not normalize case), so this matches CASE-INSENSITIVELY on its
    /// own `to_ascii_lowercase` (a caller that encoded "GET"/"Get"/"get" all resolve to
    /// [`HttpMethod::Get`]). An unrecognized name is preserved verbatim (original case) as
    /// [`HttpMethod::Other`] (open sum — never rejected here).
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
/// INDEPENDENT (any method may carry or omit a body). Returns the full [`HttpResponse`] — STATUS +
/// response HEADERS + BODY, not just the body — so a reducer can branch on the status code (200 vs 404 vs
/// 500) and read response headers (content-type, …). The `idempotency_key` lets a side-effecting transport
/// dedup a crash-re-driven request (§16c-S1/D) — relevant for a non-idempotent method.
///
/// **Error classification (supervision, [`crate::retry`]):** an `Err(reason)` MUST lead with a
/// retryability token — a transient failure (5xx, timeout, connection reset) as
/// [`crate::retry::retryable`], a permanent one (4xx client error, DNS failure, malformed URL) as
/// [`crate::retry::permanent`]. Unprefixed reasons are treated PERMANENT (fail-closed). Note: an `Err` is
/// a TRANSPORT failure (couldn't complete the request); a completed request with a 4xx/5xx STATUS is an
/// `Ok(HttpResponse)` carrying that status — the reducer decides what a 404/500 means, not the transport.
/// `#[async_trait(?Send)]` — a real HTTP request awaits the socket; not `Send` (single-threaded host).
#[async_trait::async_trait(?Send)]
pub trait HttpTransport {
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        idempotency_key: Hash,
    ) -> Result<HttpResponse, String>;
}

/// A completed HTTP response — the STATUS code, response HEADERS (ordered name/value pairs), and BODY. The
/// executor encodes this into the effect result as a `(http-response (status)(headers)(body))` binary-sexpr
/// (the kernel's [`cdz_kernel::event_ast::encode_http_response`] wire convention) so a reducer decodes it
/// with `decode_http_response` and can read status + headers, not just the body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
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
        // The payload is a `(http-request (method <name>) (headers ((k v)…)) (body <opt>))` binary-sexpr —
        // decode the caller-specified method + headers + body (NO body-presence heuristic). A blob-ref
        // payload can't be decoded (no blob-store handle) and a missing/garbage payload is malformed; both
        // are structural → PERMANENT.
        let (method, headers, body): (HttpMethod, Vec<(String, String)>, Option<Vec<u8>>) =
            match &req.payload {
                Some(Payload::Inline(bytes)) => match decode_http_request(bytes) {
                    Ok((method_name, headers, body)) => {
                        (HttpMethod::from_name(&method_name), headers, body)
                    }
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
            .request(
                method,
                &req.target,
                &headers,
                body.as_deref(),
                idempotency_key,
            )
            .await
        {
            // A completed response — encode its status + headers + body into the result as an
            // `(http-response …)` binary-sexpr so the reducer reads status/headers, not just the body. A
            // transport Err (couldn't complete) folds through with its retryability token unchanged.
            Ok(resp) => {
                let payload = encode_http_response(resp.status, &resp.headers, &resp.body);
                EffectOutcome::Ok(Some(Payload::Inline(payload.into())))
            }
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
    use cdz_kernel::event_ast::{
        decode_http_response, encode_http_request, encode_http_request_with_headers,
    };

    /// A stub transport: asserts the method/headers/body it was asked to perform, returns a canned
    /// [`HttpResponse`] (status + headers + body). Records what the executor decoded + passed through.
    struct StubHttp {
        expect_method: HttpMethod,
        expect_headers: Vec<(String, String)>,
        expect_body: Option<Vec<u8>>,
        response: HttpResponse,
    }
    #[async_trait::async_trait(?Send)]
    impl HttpTransport for StubHttp {
        async fn request(
            &self,
            method: HttpMethod,
            url: &str,
            headers: &[(String, String)],
            body: Option<&[u8]>,
            _key: Hash,
        ) -> Result<HttpResponse, String> {
            assert_eq!(url, "https://ok.host/x");
            assert_eq!(
                method, self.expect_method,
                "method threaded through verbatim"
            );
            assert_eq!(
                headers,
                &self.expect_headers[..],
                "request headers threaded through"
            );
            assert_eq!(body.map(|b| b.to_vec()), self.expect_body);
            Ok(self.response.clone())
        }
    }

    fn resp(status: u16, headers: &[(&str, &str)], body: &[u8]) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Bytes::copy_from_slice(body),
        }
    }

    /// Build an Http effect (headerless http-request payload — the 2-arg encoder).
    fn http_req(method: &str, body: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            Some(Payload::Inline(encode_http_request(method, body).into())),
            Timeliness::Interactive,
        )
    }

    /// Build an Http effect with request headers (the 3-arg encoder).
    fn http_req_h(method: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> EffectRequest {
        let hs: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x".to_string(),
            Some(Payload::Inline(
                encode_http_request_with_headers(method, &hs, body).into(),
            )),
            Timeliness::Interactive,
        )
    }

    /// Decode the executor's `(http-response …)` result payload for assertions.
    fn decode_result(out: EffectOutcome) -> (u16, Vec<(String, String)>, Vec<u8>) {
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                decode_http_response(&bytes).expect("result is a valid http-response")
            }
            other => panic!("expected Ok(Inline(http-response)), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_get_returns_status_headers_and_body() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Get,
            expect_headers: vec![],
            expect_body: None,
            response: resp(200, &[("content-type", "text/plain")], b"response body"),
        });
        let (status, headers, body) = decode_result(
            exec.perform_async(&http_req("get", None), Hash::of(b"k"))
                .await,
        );
        assert_eq!(status, 200);
        assert_eq!(
            headers,
            vec![("content-type".to_string(), "text/plain".to_string())]
        );
        assert_eq!(&body[..], b"response body");
    }

    #[tokio::test]
    async fn request_headers_thread_through_to_the_transport() {
        // Caller-supplied request headers reach the transport (asserted in the stub) + a non-200 status
        // + response headers come back readable.
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Post,
            expect_headers: vec![
                ("authorization".to_string(), "Bearer x".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ],
            expect_body: Some(b"{}".to_vec()),
            response: resp(201, &[("location", "/new/1")], b"created"),
        });
        let (status, headers, body) = decode_result(
            exec.perform_async(
                &http_req_h(
                    "post",
                    &[
                        ("authorization", "Bearer x"),
                        ("content-type", "application/json"),
                    ],
                    Some(b"{}"),
                ),
                Hash::of(b"k"),
            )
            .await,
        );
        assert_eq!(status, 201);
        assert_eq!(
            headers,
            vec![("location".to_string(), "/new/1".to_string())]
        );
        assert_eq!(&body[..], b"created");
    }

    #[tokio::test]
    async fn a_reducer_can_read_a_404_status() {
        // A completed request with a 4xx status is Ok(http-response), NOT Err — the reducer decides.
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Get,
            expect_headers: vec![],
            expect_body: None,
            response: resp(404, &[], b"not found"),
        });
        let (status, _headers, body) = decode_result(
            exec.perform_async(&http_req("get", None), Hash::of(b"k"))
                .await,
        );
        assert_eq!(
            status, 404,
            "a 404 is a completed response, not a transport error"
        );
        assert_eq!(&body[..], b"not found");
    }

    #[tokio::test]
    async fn a_bodyless_post_and_a_delete_with_body_are_both_expressible() {
        // The point of the explicit method: the two cases the body-presence heuristic couldn't do.
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Post,
            expect_headers: vec![],
            expect_body: None,
            response: resp(200, &[], b"a"),
        });
        assert!(matches!(
            exec.perform_async(&http_req("post", None), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(_)
        ));
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Delete,
            expect_headers: vec![],
            expect_body: Some(b"why".to_vec()),
            response: resp(200, &[], b"b"),
        });
        assert!(matches!(
            exec.perform_async(&http_req("delete", Some(b"why")), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(_)
        ));
    }

    #[tokio::test]
    async fn an_unknown_method_survives_verbatim_as_other() {
        let mut exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Other("propfind".to_string()),
            expect_headers: vec![],
            expect_body: None,
            response: resp(207, &[], b"x"),
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
                _h: &[(String, String)],
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<HttpResponse, String> {
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
                _h: &[(String, String)],
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<HttpResponse, String> {
                panic!("transport must not be called for a malformed request");
            }
        }
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
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
            }
            other => panic!("expected Err for a missing payload, got {other:?}"),
        }
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
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
            }
            other => panic!("expected Err for a garbage payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_transient_transport_failure_stays_retryable_through_the_executor() {
        struct FlakyHttp;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for FlakyHttp {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _h: &[(String, String)],
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<HttpResponse, String> {
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
                _h: &[(String, String)],
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<HttpResponse, String> {
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
        let exec = HttpExecutor::new(StubHttp {
            expect_method: HttpMethod::Get,
            expect_headers: vec![],
            expect_body: None,
            response: resp(200, &[], b""),
        });
        assert!(exec.handles_family(effect_ct::HTTP));
        assert!(!exec.handles_family(effect_ct::NOW));
        assert!(!exec.handles_family(effect_ct::MODEL));
        assert!(!exec.handles_family("embedding"));
    }
}
