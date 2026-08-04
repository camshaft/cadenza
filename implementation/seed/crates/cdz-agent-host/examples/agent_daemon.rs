//! A runnable AGENT DAEMON — the capstone that makes "run an agent as a real process" concrete.
//!
//! It assembles the pieces this crate provides into a live host process: the real executor set
//! (`live_executor_set` — Clock + HTTP-over-reqwest + Model-over-Bedrock, env credentials), a hosted
//! session driven by a reducer, and the single-threaded multi-session loop
//! (`AsyncAgentHost::run_with_wall_clock`) that interleaves sessions on one task until shutdown. A producer
//! feeds inbound events through `AsyncAgentHost::inbox`; here `main` seeds one event then drops the inbox
//! so the loop returns once it's processed (a long-lived daemon would instead hold the inbox open and fire
//! the shutdown channel from a signal handler).
//!
//! The real wiring is behind `live-net` (it constructs the real Bedrock/HTTP transports, which need both
//! network egress and AWS env credentials), so it is NOT built by the default hermetic gate — it's
//! lint-checked under `--features live-net` in CI and RUN by a human with credentials:
//!
//! ```text
//! AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… AWS_REGION=… \
//!   cargo run --example agent_daemon --features live-net
//! ```
//!
//! This is a DEMONSTRATION of the wiring, not a production entrypoint (a real deployment supplies its own
//! reducer(s), authorizer/policy, session ids, and inbound source — e.g. a socket or a queue consumer).

// An example target must always have a `main`, and CI lints examples WITHOUT the feature (`clippy
// --all-targets`). So the live wiring lives in a `#[cfg(feature = "live-net")]` module with its own async
// main, and a no-op main covers the default build (a file-level `#![cfg]` would compile the file out → no
// main → E0601).
#[cfg(not(feature = "live-net"))]
fn main() {
    eprintln!("agent_daemon: build with `--features live-net` to run the live agent host demo.");
}

#[cfg(feature = "live-net")]
use live_daemon::main;

#[cfg(feature = "live-net")]
mod live_daemon {
    use cdz_agent_host::{
        live_executor_set, AgentHost, AsyncAgentHost, HostedSession, Inbound, SessionId,
    };
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::hash::Hash;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A minimal demo agent: on an inbound message it fetches a URL via the real HTTP executor and records
    /// the response phase. A real deployment ships its own reducer (or a wasm `AsyncComponentReducer`);
    /// this just exercises the live loop end to end.
    struct DemoAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for DemoAgent {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    kv.put(b"phase".to_vec(), b"fetching".to_vec());
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::HTTP,
                        "https://example.com/",
                        Some(Payload::Inline(
                            cdz_kernel::event_ast::encode_http_request("get", None).into(),
                        )),
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult { .. } => {
                    kv.put(b"phase".to_vec(), b"done".to_vec());
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

    /// Grant the demo agent HTTP to the one host it fetches (SEC-F1: deny everything else).
    fn demo_authz() -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["example.com".into()]),
        }])
    }

    #[tokio::main(flavor = "current_thread")]
    pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // Assemble the REAL executor set (Clock + HTTP + Bedrock; env credentials, no broker).
        let executors = live_executor_set().await?;

        // One hosted session driven by the demo reducer, gated by its authorizer.
        let mut registry = AgentHost::new();
        registry.spawn(
            SessionId::new("demo"),
            HostedSession::genesis(
                Hash::of(b"demo-agent-v1"),
                Box::new(DemoAgent),
                Box::new(demo_authz()),
                executors,
            ),
        );

        // Wrap the registry in the async multi-session loop; hand producers an inbox sender.
        let host = AsyncAgentHost::new(registry);
        let inbox = host.inbox();

        // The shutdown channel: a long-lived daemon would fire this from a signal handler (enable tokio's
        // `signal` feature + a ctrl-c task); this demo runs until the inbox drains — seed one event then
        // drop the only sender, so `run_with_wall_clock` returns once that event is processed.
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Seed one inbound event (a real deployment's producer would feed these from a socket/queue).
        inbox.send(Inbound {
            session: SessionId::new("demo"),
            body: inbound_go(),
            cause: None,
        })?;
        drop(inbox); // channel closes after the seeded event → the loop returns when it's drained

        // Run the loop on the wall clock. A KernelError here is corruption / a programming fault (a genuine
        // reducer fault is a FoldFailed log EVENT, not a KernelError), so surface it rather than swallow.
        let final_registry = host
            .run_with_wall_clock(shutdown_rx)
            .await
            .map_err(|e| format!("agent daemon loop hit a kernel error: {e:?}"))?;
        if let Some(session) = final_registry.get(&SessionId::new("demo")) {
            let phase = session
                .session()
                .kv()
                .get(b"phase")
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_else(|| "<never ran>".to_string());
            eprintln!("agent_daemon: demo session final phase = {phase}");
        }
        Ok(())
    }
}
