//! End-to-end (hermetic): an agent runs a real TOOL-CALLING loop — model → shell tool-call → model →
//! end_turn — through the WHOLE cdz-agent-host stack (GAP-1, the self-hosting harness's first real
//! agent-runs-a-shell-tool-call). This is the HOST-side counterpart to v-agent-harness's kernel M3 fold-proof
//! (`agent_loop_reducer_folds_…`): M3 proves the loop composes at the kernel level with a scripted executor;
//! THIS drives the identical fold through the real machinery — a `ModelExecutor` (over a STUB Converse
//! transport, so it's hermetic) + the real [`ShellExecutor`] (which spawns an actual process) composed in one
//! `CompositeExecutor`, exactly as a deployed agent runs. Only the model transport is stubbed; swapping in the
//! `live-net` Bedrock Converse transport changes nothing else.
//!
//! The loop (a fold over existing effects, NO new kernel mechanism — operator's minimize-kernel/host-logic):
//! inbound task → emit `model` effect (M1 request offering a `shell` tool) → stub returns an M2 `tool_use`
//! response → reducer dispatches the `shell` effect (real command) → shell result folds back → reducer
//! re-emits `model` (M1 carrying the tool-RESULT with the call id) → stub returns `end_turn` → reducer records
//! the answer. Driven to quiescence by one `deliver()`.
//!
//! Requires `live-exec` (the real ShellExecutor) — the executor set + the agentic loop are what's proven.

#![cfg(feature = "live-exec")]

use cdz_agent_host::{ModelExecutor, ModelTransport, ShellExecutor};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::event_ast::{
    encode_model_request, encode_model_response, ChatMessage, ContentBlock, ModelRequest,
    ModelResponse, ToolDef,
};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::Session;
use cdz_kernel::kv::Kv;
use cdz_kernel::reducer::{FoldOutput, Reducer};
use std::cell::Cell;

const MODEL_ID: &str = "claude-test";

/// A STUB Converse model transport scripting the two turns of the loop: the FIRST model call returns an M2
/// `tool_use` (call the `shell` tool), the SECOND returns `end_turn` (the answer). Hermetic — the real
/// Bedrock Converse transport implements this same trait behind `live-net`, changing only the I/O. The
/// transport receives M1 request bytes; it doesn't need to decode them (the script is turn-ordered), it just
/// returns the M2 bytes the reducer folds.
struct ScriptedConverse {
    calls: Cell<u32>,
}
#[async_trait::async_trait(?Send)]
impl ModelTransport for ScriptedConverse {
    async fn invoke(
        &self,
        _model_id: &str,
        _body: &[u8],
        _key: Hash,
    ) -> Result<bytes::Bytes, EffectOutcome> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        let resp = if n == 0 {
            // Turn 1: ask to run the shell tool. The input is the JSON the shell tool-call carries; the
            // reducer maps this tool-call to a `shell` effect whose target is the command.
            ModelResponse {
                stop_reason: "tool_use".to_string(),
                content: vec![ContentBlock::ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    input: br#"{"cmd":"echo built-green"}"#.to_vec(),
                }],
            }
        } else {
            // Turn 2: the model saw the tool result → done, with the final answer.
            ModelResponse {
                stop_reason: "end_turn".to_string(),
                content: vec![ContentBlock::Text("done: built-green".to_string())],
            }
        };
        Ok(encode_model_response(&resp).into())
    }
}

/// The reference AGENT-LOOP reducer (M3 shape, host-side). Distinguishes a MODEL response (decodes as M2)
/// from a TOOL result (raw shell stdout, doesn't) and routes: `tool_use` → dispatch the shell effect;
/// `end_turn` → record the answer; a tool result → re-emit the next model turn carrying the ToolResult (with
/// the call id, so the loop closes). The tool→effect map (`shell` tool → `shell` family) is REDUCER-defined
/// (O2 / operator standing-order: policy lives in the reducer, not the host).
struct ToolCallingAgent;
impl ToolCallingAgent {
    fn model_effect(req: &ModelRequest) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::MODEL,
            MODEL_ID,
            Some(Payload::Inline(encode_model_request(req).into())),
            Timeliness::Interactive,
        )
    }
}
#[async_trait::async_trait(?Send)]
impl Reducer for ToolCallingAgent {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                // Kick the loop: a model request offering the shell tool.
                let req = ModelRequest {
                    model: MODEL_ID.to_string(),
                    messages: vec![ChatMessage {
                        role: "user".to_string(),
                        content: vec![ContentBlock::Text("build the project".to_string())],
                    }],
                    tools: vec![ToolDef {
                        name: "shell".to_string(),
                        schema: br#"{"type":"object"}"#.to_vec(),
                    }],
                    max_tokens: Some(1024),
                };
                FoldOutput::with(vec![Self::model_effect(&req)])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                ..
            } => {
                if let Ok(resp) = cdz_kernel::event_ast::decode_model_response(bytes) {
                    match resp.stop_reason.as_str() {
                        "tool_use" => {
                            let mut effects = Vec::new();
                            for blk in &resp.content {
                                if let ContentBlock::ToolCall { name, .. } = blk {
                                    if name == "shell" {
                                        // Reducer-defined tool→effect map: the `shell` tool → a `shell`
                                        // effect. The target is the command (what the real ShellExecutor
                                        // runs); a production reducer would derive it from the tool input.
                                        effects.push(EffectRequest::new_with_family(
                                            effect_ct::SHELL,
                                            "echo built-green",
                                            None,
                                            Timeliness::Interactive,
                                        ));
                                    }
                                }
                            }
                            FoldOutput::with(effects)
                        }
                        _ => {
                            // end_turn → record the answer, loop done.
                            let answer: String = resp
                                .content
                                .iter()
                                .filter_map(|b| match b {
                                    ContentBlock::Text(t) => Some(t.as_str()),
                                    _ => None,
                                })
                                .collect();
                            kv.put(b"answer".to_vec(), answer.into_bytes());
                            FoldOutput::none()
                        }
                    }
                } else {
                    // A TOOL (shell) result → re-emit the next model turn carrying the tool-result (call id
                    // round-trips so the model correlates it). Records the raw tool output for the assertion.
                    kv.put(b"shell-out".to_vec(), bytes.to_vec());
                    let req = ModelRequest {
                        model: MODEL_ID.to_string(),
                        messages: vec![ChatMessage {
                            role: "tool".to_string(),
                            content: vec![ContentBlock::ToolResult {
                                id: "call-1".to_string(),
                                result: bytes.to_vec(),
                            }],
                        }],
                        tools: vec![],
                        max_tokens: Some(1024),
                    };
                    FoldOutput::with(vec![Self::model_effect(&req)])
                }
            }
            _ => FoldOutput::none(),
        }
    }
}

fn inbound_task() -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"go".to_vec().into()),
    }
}

/// Grant `model` (to the test model id) + `shell` (to the exact command) — deny-by-default, SEC-F1 scopes
/// each target. This is where the COMMAND is authorized (Cedar's job in production; the flat Authorizer here):
/// the host ShellExecutor runs only what this policy permits.
fn agent_caps() -> Authorizer {
    Authorizer::new(vec![
        Capability {
            kind: EffectKind::Model,
            predicate: ResourcePredicate::Exact(MODEL_ID.into()),
        },
        Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::Exact("echo built-green".into()),
        },
    ])
}

#[tokio::test]
async fn agent_runs_a_shell_tool_call_end_to_end_model_tool_model_end_turn() {
    let mut reducer = ToolCallingAgent;
    let mut exec = CompositeExecutor::new()
        .with_effect(
            effect_ct::MODEL,
            Box::new(ModelExecutor::new(ScriptedConverse {
                calls: Cell::new(0),
            })),
        )
        .with_effect(effect_ct::SHELL, Box::new(ShellExecutor::new()));
    let mut session = Session::genesis(Hash::of(b"tool-agent-v1"), Hash::of(b"tool-agent-nonce"));

    session
        .deliver(inbound_task(), None, &mut reducer, &agent_caps(), &mut exec)
        .await
        .unwrap();

    // The loop ran model→shell→model→end_turn to quiescence and recorded the final answer.
    assert_eq!(
        session.kv().get(b"answer"),
        Some(&b"done: built-green"[..]),
        "the agent folded through model → shell tool-call → model → end_turn and recorded the answer"
    );
    // The REAL ShellExecutor ran `echo built-green` — its stdout is what the reducer folded as the tool result.
    let shell_out = session.kv().get(b"shell-out").expect("shell ran");
    assert_eq!(
        String::from_utf8_lossy(shell_out).trim(),
        "built-green",
        "the real ShellExecutor executed the tool-call command and its stdout folded back"
    );
    assert_eq!(
        session.open_effects(),
        0,
        "every effect in the loop settled"
    );

    // Replay-equivalence: the model + shell outcomes are in the log, so replay reconstructs the identical KV
    // without re-invoking the transport or re-running the command (a side-effecting tool runs once).
    let replayed = Session::replay(session.log().to_vec(), &mut reducer)
        .await
        .unwrap();
    assert_eq!(
        replayed.kv().get(b"answer"),
        Some(&b"done: built-green"[..])
    );
    assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
}
