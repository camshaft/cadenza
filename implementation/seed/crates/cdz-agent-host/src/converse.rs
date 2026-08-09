//! GAP-1 tool-calling transport — the HERMETIC half: decode the kernel's M1 `model-request` codec into a
//! transport-agnostic [`ConverseRequest`] the Bedrock transport maps to a `Converse` call. Splitting the
//! decode + role/block INTERPRETATION out here (no aws-sdk types) keeps it unit-testable in the default gate;
//! the `live-net` Bedrock transport's job then shrinks to a thin type-for-type map onto the aws-sdk builders.
//!
//! **Where this sits (operator standing-order: host = thin mechanism).** The kernel carries opaque bytes
//! (O4); the AGENT-LOOP POLICY (which tools, how they map to effects, when to stop) lives in the reducer
//! (wasm on the log), NOT here. This module is pure MECHANISM: translate the reducer's already-built
//! `model-request` into the shape the Bedrock API wants + own the JSON⟷Bedrock byte boundary (O4). It makes
//! no agent decisions.
//!
//! **Both halves (M1+M2 on trunk).** REQUEST: [`from_model_request`] decodes M1 → [`ConverseRequest`].
//! RESPONSE: [`to_model_response`] maps a [`ConverseResponse`] → the kernel's M2 [`ModelResponse`]. Together
//! they let a reducer express the agentic loop as a fold over the `model` effect (emit request → fold
//! response → dispatch tool-calls → fold results → re-emit). The `live-net` Bedrock transport fills the two
//! aws-sdk-free intermediates from the aws-sdk `Converse` in/out (the remaining thin wiring slice).

use cdz_kernel::event_ast::{ChatMessage, ContentBlock, ModelRequest};

/// A transport-agnostic, DECODED model request — the intermediate between the kernel's M1 codec and the
/// Bedrock `Converse` API. The `live-net` transport maps this onto aws-sdk builder types (Message /
/// ContentBlock / ToolConfig / InferenceConfig) with no further interpretation. Byte payloads (`input` /
/// `schema`) stay opaque here; the transport parses them as JSON at the Bedrock boundary (O4).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConverseRequest {
    /// The Bedrock model id (Converse `modelId`). From `ModelRequest.model`.
    pub model_id: String,
    /// The SYSTEM prompt blocks, hoisted out of the messages: Bedrock's `Converse` takes `system` as a
    /// SEPARATE top-level field, not a message role — so `system`-role turns' text is collected here, in
    /// order, rather than left in `messages`. Empty when the reducer sent no system turn.
    pub system: Vec<String>,
    /// The conversation turns (Bedrock `messages`), EXCLUDING system turns (hoisted above). Roles are
    /// normalized to Bedrock's vocab (user/assistant); a `tool` role turn becomes a `user` message carrying
    /// tool-RESULT blocks (Bedrock models a tool result as a user-turn `toolResult` content block).
    pub messages: Vec<ConverseMessage>,
    /// The tool definitions offered this turn (Bedrock `toolConfig.tools`): name + opaque JSON-schema bytes.
    pub tools: Vec<ConverseTool>,
    /// The max output tokens (Bedrock `inferenceConfig.maxTokens`), if the reducer set one.
    pub max_tokens: Option<u64>,
}

/// A normalized conversation turn for Converse. `role` is Bedrock's vocab (`user` | `assistant`); the M1
/// `system` role is hoisted to [`ConverseRequest::system`] and `tool` is folded into a `user` turn carrying
/// tool-result blocks (Bedrock's model of a tool result).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConverseMessage {
    pub role: ConverseRole,
    pub content: Vec<ContentBlock>,
}

/// Bedrock's message-role vocab for Converse (the API accepts only these two message roles; system is a
/// separate field, tool-results ride a user turn).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConverseRole {
    User,
    Assistant,
}

/// A tool offered to the model — name + opaque JSON-schema bytes (the transport parses the bytes as the
/// tool's `inputSchema.json` at the Bedrock boundary). The schema rides as ref-counted [`bytes::Bytes`]
/// (operator cheaply-clonable directive: a byte buffer is `Bytes`, not `Vec<u8>`), so a `ConverseTool` /
/// `ConverseRequest` clone is an O(1) ref-count bump, not a deep schema copy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConverseTool {
    pub name: String,
    pub schema: bytes::Bytes,
}

/// Errors mapping an M1 `ModelRequest` into a [`ConverseRequest`] — a request that decoded fine but can't be
/// expressed to Bedrock (an unknown role). Total: never panics; the transport folds this as a PERMANENT
/// effect error (a malformed request the reducer built — retrying won't fix it).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ConverseMapError {
    /// A message carried a role that isn't one of system/user/assistant/tool.
    UnknownRole(String),
}

impl std::fmt::Display for ConverseMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConverseMapError::UnknownRole(r) => {
                write!(
                    f,
                    "unknown message role {r:?} (expected system/user/assistant/tool)"
                )
            }
        }
    }
}

/// Map a decoded M1 [`ModelRequest`] into a transport-agnostic [`ConverseRequest`]: hoist system turns into
/// the separate `system` field, normalize roles to Bedrock's vocab (tool→user), and pass tools + max_tokens
/// through. Pure + total (an unknown role is a clean `Err`, never a panic). Content blocks are carried
/// verbatim (opaque bytes stay opaque — the transport does the JSON⟷Bedrock mapping, O4).
pub fn from_model_request(req: &ModelRequest) -> Result<ConverseRequest, ConverseMapError> {
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for ChatMessage { role, content } in &req.messages {
        match role.as_str() {
            "system" => {
                // Bedrock's `system` is a separate top-level field: collect this turn's text blocks there.
                // (A system turn carrying non-text blocks is unusual; only its text is hoisted, in order.)
                for block in content {
                    if let ContentBlock::Text(t) = block {
                        system.push(t.clone());
                    }
                }
            }
            // A `tool` turn's results ride a USER message in Bedrock's model (a user-turn `toolResult` block).
            "user" | "tool" => messages.push(ConverseMessage {
                role: ConverseRole::User,
                content: content.clone(),
            }),
            "assistant" => messages.push(ConverseMessage {
                role: ConverseRole::Assistant,
                content: content.clone(),
            }),
            other => return Err(ConverseMapError::UnknownRole(other.to_string())),
        }
    }
    Ok(ConverseRequest {
        model_id: req.model.clone(),
        system,
        messages,
        tools: req
            .tools
            .iter()
            .map(|t| ConverseTool {
                name: t.name.clone(),
                // `ToolDef.schema` is the kernel's owned `Vec<u8>`; freeze it into `Bytes` once here (the
                // last deep copy — every downstream clone of the request is then O(1)).
                schema: bytes::Bytes::from(t.schema.clone()),
            })
            .collect(),
        max_tokens: req.max_tokens,
    })
}

// ── Response half (M2): Bedrock Converse output → the kernel's `model-response` codec ─────────────────

use cdz_kernel::event_ast::ModelResponse;

/// A transport-agnostic, decoded Bedrock `Converse` OUTPUT — the intermediate the `live-net` transport fills
/// from the aws-sdk `ConverseOutput` (raw `stopReason` string + the assistant's output content blocks), which
/// [`to_model_response`] then maps into the kernel's M2 [`ModelResponse`] for the reducer to fold. Keeping
/// this aws-sdk-free keeps the mapping hermetically testable; the transport's job shrinks to a type-for-type
/// fill. A response only ever carries `Text` / `ToolCall` blocks (never `ToolResult` — that's the reducer's
/// next request turn).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConverseResponse {
    /// Bedrock's raw `stopReason` string (e.g. `end_turn` | `tool_use` | `max_tokens` | …). Carried VERBATIM
    /// — the kernel/reducer own the vocab (constraint #1: a novel Bedrock value decodes fine, reducer folds
    /// not-`tool_use` = done). Never narrowed to an enum here.
    pub stop_reason: String,
    /// The assistant's output blocks, in order: `Text` prose and/or `ToolCall`s (id + name + opaque input
    /// bytes — the transport serialized Bedrock's `toolUse.input` JSON document to bytes at the boundary, O4).
    pub content: Vec<ContentBlock>,
}

/// Map a transport-agnostic [`ConverseResponse`] into the kernel's M2 [`ModelResponse`] — the payload the
/// reducer folds off the `model` effect's `EffectResult`. A pure, total pass-through: the block grammar +
/// raw stop-reason are already normalized by the transport, so this just moves the fields. (The
/// [`encode_model_response`](cdz_kernel::event_ast::encode_model_response) call that turns this into wire
/// bytes is the transport's final step; kept out of this pure mapping so the mapping stays trivially
/// testable.)
pub fn to_model_response(resp: &ConverseResponse) -> ModelResponse {
    ModelResponse {
        stop_reason: resp.stop_reason.clone(),
        content: resp.content.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::event_ast::{ChatMessage, ContentBlock, ModelRequest, ToolDef};

    fn msg(role: &str, content: Vec<ContentBlock>) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content,
        }
    }

    #[test]
    fn hoists_system_turns_and_normalizes_roles() {
        let req = ModelRequest {
            model: "anthropic.claude-x".into(),
            messages: vec![
                msg("system", vec![ContentBlock::Text("be terse".into())]),
                msg("user", vec![ContentBlock::Text("hi".into())]),
                msg(
                    "assistant",
                    vec![ContentBlock::ToolCall {
                        id: "call-1".into(),
                        name: "shell".into(),
                        input: b"{}".to_vec(),
                    }],
                ),
                msg(
                    "tool",
                    vec![ContentBlock::ToolResult {
                        id: "call-1".into(),
                        result: b"ok".to_vec(),
                    }],
                ),
            ],
            tools: vec![ToolDef {
                name: "shell".into(),
                schema: b"{\"type\":\"object\"}".to_vec(),
            }],
            max_tokens: Some(1024),
        };
        let cr = from_model_request(&req).expect("maps");
        assert_eq!(cr.model_id, "anthropic.claude-x");
        assert_eq!(
            cr.system,
            vec!["be terse".to_string()],
            "system hoisted out of messages"
        );
        // 3 non-system messages: user, assistant(tool-call), tool→user(tool-result).
        assert_eq!(cr.messages.len(), 3);
        assert_eq!(cr.messages[0].role, ConverseRole::User);
        assert_eq!(cr.messages[1].role, ConverseRole::Assistant);
        assert_eq!(
            cr.messages[2].role,
            ConverseRole::User,
            "a tool-result turn is a USER message in Bedrock's model"
        );
        // The tool-use id round-trips through the conversation (assistant call ↔ tool result).
        assert!(
            matches!(&cr.messages[1].content[0], ContentBlock::ToolCall { id, .. } if id == "call-1")
        );
        assert!(
            matches!(&cr.messages[2].content[0], ContentBlock::ToolResult { id, .. } if id == "call-1")
        );
        assert_eq!(cr.tools.len(), 1);
        assert_eq!(cr.tools[0].name, "shell");
        assert_eq!(cr.max_tokens, Some(1024));
    }

    #[test]
    fn an_unknown_role_is_a_clean_error_not_a_panic() {
        let req = ModelRequest {
            model: "m".into(),
            messages: vec![msg("wizard", vec![ContentBlock::Text("cast".into())])],
            tools: vec![],
            max_tokens: None,
        };
        assert_eq!(
            from_model_request(&req),
            Err(ConverseMapError::UnknownRole("wizard".into())),
            "an unknown role maps to a clean Err, never a panic"
        );
    }

    #[test]
    fn a_no_tools_no_system_request_maps_to_a_plain_conversation() {
        // The degenerate single-shot case (O1: opaque/no-tools is the additive base): just user turns.
        let req = ModelRequest {
            model: "m".into(),
            messages: vec![msg("user", vec![ContentBlock::Text("2+2?".into())])],
            tools: vec![],
            max_tokens: None,
        };
        let cr = from_model_request(&req).expect("maps");
        assert!(cr.system.is_empty() && cr.tools.is_empty() && cr.max_tokens.is_none());
        assert_eq!(cr.messages.len(), 1);
        assert_eq!(cr.messages[0].role, ConverseRole::User);
    }

    #[test]
    fn response_maps_a_tool_use_output_and_survives_the_m2_codec_round_trip() {
        // A tool_use response → M2 ModelResponse → wire bytes → decode = identity (the reducer folds exactly
        // what the transport produced). Exercises the response half end to end through the kernel codec.
        use cdz_kernel::event_ast::{decode_model_response, encode_model_response};
        let resp = ConverseResponse {
            stop_reason: "tool_use".into(),
            content: vec![
                ContentBlock::Text("running it".into()),
                ContentBlock::ToolCall {
                    id: "call-9".into(),
                    name: "shell".into(),
                    input: b"{\"cmd\":\"cargo test\"}".to_vec(),
                },
            ],
        };
        let mr = to_model_response(&resp);
        assert_eq!(mr.stop_reason, "tool_use");
        assert_eq!(mr.content.len(), 2);
        // Round-trip through the kernel M2 codec: encode → decode == the mapped response.
        let bytes = encode_model_response(&mr);
        let decoded = decode_model_response(&bytes).expect("M2 decodes");
        assert_eq!(
            decoded, mr,
            "the mapped ModelResponse round-trips through the M2 codec"
        );
    }

    #[test]
    fn response_carries_a_novel_stop_reason_verbatim() {
        // Constraint #1: stop_reason is a raw string — a Bedrock value the kernel never saw still passes
        // through (the reducer folds an unknown reason as not-tool_use = done). No enum narrowing.
        let resp = ConverseResponse {
            stop_reason: "some_future_bedrock_reason".into(),
            content: vec![ContentBlock::Text("done".into())],
        };
        let mr = to_model_response(&resp);
        assert_eq!(
            mr.stop_reason, "some_future_bedrock_reason",
            "a novel stop-reason is carried verbatim, not rejected/normalized"
        );
    }
}
