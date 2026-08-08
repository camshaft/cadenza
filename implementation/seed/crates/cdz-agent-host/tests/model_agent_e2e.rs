//! End-to-end: an agent RUNS a real model-invocation loop through the [`ModelExecutor`].
//!
//! This is the headline milestone in its hermetic form: a reducer emits a `Model` effect (target = the
//! model id, payload = the request body); the kernel authorizes + durably dispatches it; the
//! `ModelExecutor` calls its transport; the completion folds back and drives the agent's next step. The
//! transport here is a STUB (canned completion) so the loop is hermetically gateable — the REAL Bedrock
//! transport (SigV4 + cred-broker) drops into the exact same wiring behind `live-net`, changing only the
//! transport, not the loop.
//!
//! Also proves the multi-executor composition the whole design turns on: a `CompositeExecutor` routes
//! `Now` to the `ClockExecutor` and `Model` to the `ModelExecutor` in ONE agent, exactly as a real agent
//! (which both checks the time and calls a model) needs.

use cdz_agent_host::{ClockExecutor, ModelExecutor, ModelTransport};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};

/// A stub model transport: returns a canned completion, so the agent loop is hermetic. The real Bedrock
/// transport implements this same trait behind `live-net`.
struct StubModel;
#[async_trait::async_trait(?Send)]
impl ModelTransport for StubModel {
    async fn invoke(
        &self,
        model_id: &str,
        body: &[u8],
        _key: Hash,
    ) -> Result<bytes::Bytes, EffectOutcome> {
        // Echo enough that the test can prove the executor threaded the model id + prompt through.
        Ok(
            format!("{model_id} says: {}", String::from_utf8_lossy(body))
                .into_bytes()
                .into(),
        )
    }
}

/// A minimal agent that calls a model: on "go" it emits a `Model` effect (a prompt); when the
/// completion comes back it records it and marks itself `answered`. The loop closes through a real
/// executor (here a stub transport) — the shape a Bedrock-backed agent runs verbatim.
struct ModelAgent;
#[async_trait::async_trait(?Send)]
impl Reducer for ModelAgent {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                kv.put(b"phase".to_vec(), b"prompting".to_vec());
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::MODEL,
                    "claude-test",
                    Some(Payload::Inline(b"hello".to_vec().into())),
                    Timeliness::Interactive,
                )])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(completion))),
                ..
            } => {
                kv.put(b"completion".to_vec(), completion.to_vec());
                kv.put(b"phase".to_vec(), b"answered".to_vec());
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

/// Grant exactly `Model` to the specific model id (deny-by-default; SEC-F1 scopes the target).
fn model_cap() -> Authorizer {
    Authorizer::new(vec![Capability {
        kind: EffectKind::Model,
        predicate: ResourcePredicate::Exact("claude-test".into()),
    }])
}

#[tokio::test]
async fn agent_loop_runs_end_to_end_through_the_model_executor() {
    let mut reducer = ModelAgent;
    let mut exec = CompositeExecutor::new()
        .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(StubModel)));
    let mut session = Session::genesis(
        Hash::of(b"model-agent-v1"),
        Hash::of(b"model-agent-v1-nonce"),
    );

    session
        .deliver(inbound_go(), None, &mut ModelAgent, &model_cap(), &mut exec)
        .await
        .unwrap();

    // The loop closed: the agent prompted the model, the executor invoked the transport, and the
    // completion folded back and advanced the agent to `answered`.
    assert_eq!(session.kv().get(b"phase"), Some(&b"answered"[..]));
    let completion = session
        .kv()
        .get(b"completion")
        .expect("completion recorded");
    assert_eq!(
        String::from_utf8_lossy(completion),
        "claude-test says: hello",
        "the executor threaded model id + prompt through the transport and folded the completion back"
    );
    assert_eq!(session.open_effects(), 0);

    // Replay-equivalence: the completion is recorded, so replay reconstructs the identical KV without
    // ever calling the transport again (a paid model call happens once; replay is free).
    let replayed = Session::replay(session.log().to_vec(), &mut reducer)
        .await
        .unwrap();
    assert_eq!(replayed.kv().get(b"phase"), Some(&b"answered"[..]));
    assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
}

#[tokio::test]
async fn one_composite_routes_both_now_and_model_for_one_agent() {
    // A real agent both reads the clock AND calls a model — the CompositeExecutor routes each kind to
    // its own real executor. This proves the compose path v-agent-harness confirmed (with(kind, exec)).
    struct ClockThenModel;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ClockThenModel {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                // Step 1: ask for the time.
                EventBody::Inbound { .. } => {
                    kv.put(b"phase".to_vec(), b"timing".to_vec());
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::NOW,
                        String::new(),
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => match kv.get(b"phase") {
                    // Step 2: got the time → now prompt the model.
                    Some(b"timing") => {
                        kv.put(b"at".to_vec(), bytes.to_vec());
                        kv.put(b"phase".to_vec(), b"prompting".to_vec());
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::MODEL,
                            "claude-test",
                            Some(Payload::Inline(b"hi".to_vec().into())),
                            Timeliness::Interactive,
                        )])
                    }
                    // Step 3: got the completion → done.
                    Some(b"prompting") => {
                        kv.put(b"completion".to_vec(), bytes.to_vec());
                        kv.put(b"phase".to_vec(), b"done".to_vec());
                        FoldOutput::none()
                    }
                    _ => FoldOutput::none(),
                },
                _ => FoldOutput::none(),
            }
        }
    }

    let authz = Authorizer::new(vec![
        Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        },
        Capability {
            kind: EffectKind::Model,
            predicate: ResourcePredicate::Exact("claude-test".into()),
        },
    ]);
    let mut exec = CompositeExecutor::new()
        .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()))
        .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(StubModel)));
    let mut session = Session::genesis(
        Hash::of(b"clock-then-model-v1"),
        Hash::of(b"clock-then-model-v1-nonce"),
    );

    session
        .deliver(inbound_go(), None, &mut ClockThenModel, &authz, &mut exec)
        .await
        .unwrap();

    // Both kinds routed to their own real executor across the multi-step loop: a real recorded time,
    // then a real (stub) completion, driving the agent to `done`.
    assert_eq!(session.kv().get(b"phase"), Some(&b"done"[..]));
    assert!(session.kv().get(b"at").is_some(), "the clock leg ran");
    assert_eq!(
        String::from_utf8_lossy(session.kv().get(b"completion").unwrap()),
        "claude-test says: hi"
    );
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test]
async fn a_model_call_to_an_unpermitted_id_is_denied_before_the_transport() {
    // Deny-by-default (SEC-F1): the grant is for `claude-test`; an agent prompting a DIFFERENT model id
    // is denied at the gate — the transport (a paid call) is never reached. A transport that panics if
    // called proves the executor was never consulted.
    struct MustNotCall;
    #[async_trait::async_trait(?Send)]
    impl ModelTransport for MustNotCall {
        async fn invoke(
            &self,
            _m: &str,
            _b: &[u8],
            _k: Hash,
        ) -> Result<bytes::Bytes, EffectOutcome> {
            panic!("a denied Model effect must never reach the transport");
        }
    }
    struct WrongModelAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for WrongModelAgent {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::MODEL,
                    "expensive-other-model",
                    Some(Payload::Inline(b"hi".to_vec().into())),
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }
    let mut exec = CompositeExecutor::new()
        .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(MustNotCall)));
    let mut session = Session::genesis(
        Hash::of(b"wrong-model-v1"),
        Hash::of(b"wrong-model-v1-nonce"),
    );
    session
        .deliver(
            inbound_go(),
            None,
            &mut WrongModelAgent,
            &model_cap(),
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
