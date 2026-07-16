//! The native Cadenza agent-harness EMBEDDER.
//!
//! The agent LOOP is authored in Cadenza (`implementation/agent-harness/`); this crate RUNS it and
//! answers its one external interaction — the model call `cadenza:model/api`.converse
//! (`String -> String`) — with a real backend, via [`cdz_run::run_agent`]. `run_agent` binds `converse`
//! to a host closure over the shared value-heap runtime (it reads the prompt rope with `str-get`, calls
//! our closure, mints the completion rope with `str-new`), so the loop stays PURE Cadenza and the only
//! non-Cadenza surface is the `converse` closure this crate supplies.
//!
//! Two backends:
//! - [`mock_converse`] — always available; a deterministic stand-in (uppercase) for tests + local runs
//!   with no network / no creds.
//! - [`bedrock_converse`] (feature `bedrock`) — a real Amazon Bedrock `InvokeModel` call (the SDK does
//!   SigV4 + HTTPS), the actual model wiring that replaces the headless `claude` CLI. It is behind the
//!   `bedrock` feature so the default build/test carries neither the aws-sdk nor tokio.
//!
//! This is the concierge-approved bring-up (option c): it ships Bedrock wiring NOW without a Cadenza
//! peer (Cadenza has no TLS/SigV4 yet) or the host-String-result ABI (unbuilt). It is an explicit
//! STOPGAP — when the host-String-result ABI lands, this collapses to a cleaner host-boundary binding.

use anyhow::Result;
use cdz_run::{Outcome, RunOpts};

/// Cedar authorization (Inc-3) — the decision every tool dispatch + resource access passes through.
pub mod cedar;

/// The interface + op a Cadenza agent loop binds its model call to (`(bind Model "cadenza:model/api")`,
/// `Model.converse : String -> String`). Fixed here so the embedder and the loop agree by construction.
pub const MODEL_IFACE: &str = "cadenza:model/api";
pub const MODEL_OP: &str = "converse";

/// Run a compiled Cadenza agent-loop `consumer` component, answering its `Model.converse` calls with
/// `converse`. A thin wrapper over [`cdz_run::run_agent`] fixing the model interface/op names. The
/// caller supplies `opts` (the value-heap runtime bytes + the export to invoke).
pub fn run_agent_loop<F>(consumer_bytes: &[u8], opts: &RunOpts, converse: F) -> Result<Outcome>
where
    F: Fn(String) -> String + Send + Sync + 'static,
{
    cdz_run::run_agent(consumer_bytes, MODEL_IFACE, MODEL_OP, opts, converse)
}

/// A deterministic MOCK model: uppercase the prompt. Stands in for a real model in tests + offline runs
/// (same `String -> String` seam the real backend fills), so the loop can be exercised with no network.
pub fn mock_converse(prompt: String) -> String {
    prompt.to_uppercase()
}

/// A real Amazon Bedrock model call: `InvokeModel` with the Anthropic Messages request shape, returning
/// the completion text. The AWS SDK handles SigV4 + HTTPS against the caller's account (the same path
/// hivemind's enrichment Lambda uses). Synchronous wrapper: the agent loop's `converse` closure is
/// synchronous (it runs inside a wasmtime host call), so we drive the async SDK on a small tokio runtime.
///
/// `model_id` should be a region-prefixed inference-profile id for newer Claude models (e.g.
/// `us.anthropic.claude-opus-4-8` / `us.anthropic.claude-haiku-4-5-20251001-v1:0`), else Bedrock returns
/// a ValidationException. `max_tokens` bounds the completion.
#[cfg(feature = "bedrock")]
pub fn bedrock_converse(model_id: String, max_tokens: u32) -> impl Fn(String) -> String {
    move |prompt: String| {
        // The loop's converse is sync (a wasmtime host-call closure); run the async SDK on a per-call
        // current-thread runtime. A model call is seconds-scale + one-per-turn, so the runtime setup cost
        // is negligible relative to the network round-trip.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build a tokio runtime for the Bedrock call");
        rt.block_on(bedrock_invoke(&model_id, max_tokens, &prompt))
            .unwrap_or_else(|e| format!("[bedrock error: {e}]"))
    }
}

/// The async Bedrock `InvokeModel` round-trip: build the Anthropic Messages body, send it, parse the
/// first text block of the response. Isolated so the sync wrapper above stays small.
#[cfg(feature = "bedrock")]
async fn bedrock_invoke(model_id: &str, max_tokens: u32, prompt: &str) -> Result<String> {
    use anyhow::anyhow;
    let cfg = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = aws_sdk_bedrockruntime::Client::new(&cfg);
    // The Anthropic Messages API shape on Bedrock (same as hivemind's lambda-enrich): a JSON body with a
    // single user message. `serde_json` is transitively available via the SDK, but to avoid taking it as
    // a direct dep we assemble the small body by hand (the prompt is escaped for JSON).
    let body = format!(
        r#"{{"anthropic_version":"bedrock-2023-05-31","max_tokens":{max_tokens},"messages":[{{"role":"user","content":"{}"}}]}}"#,
        json_escape(prompt)
    );
    let out = client
        .invoke_model()
        .model_id(model_id)
        .content_type("application/json")
        .accept("application/json")
        .body(aws_sdk_bedrockruntime::primitives::Blob::new(
            body.into_bytes(),
        ))
        .send()
        .await
        .map_err(|e| anyhow!("bedrock invoke_model: {e}"))?;
    let bytes = out.body().as_ref();
    let text = std::str::from_utf8(bytes).map_err(|e| anyhow!("bedrock body not utf-8: {e}"))?;
    // The response is `{"content":[{"type":"text","text":"..."},...],...}`. Extract the first text block
    // without pulling serde: find `"text":"` and read the JSON string that follows.
    first_text_block(text).ok_or_else(|| anyhow!("bedrock response missing content[].text: {text}"))
}

/// Escape a string for embedding as a JSON string value (the prompt into the request body). Handles the
/// characters JSON requires: `"`, `\`, and the control chars a prompt might carry.
#[cfg(feature = "bedrock")]
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Extract the first `"text":"..."` JSON string value from a Bedrock response body, decoding the JSON
/// string escapes. A minimal reader (no serde dep): find the key, then read the quoted value honoring
/// backslash escapes. Returns None if no text block is present.
#[cfg(feature = "bedrock")]
fn first_text_block(body: &str) -> Option<String> {
    let key = "\"text\":\"";
    let start = body.find(key)? + key.len();
    // Iterate the value by CHAR, not byte: a Bedrock completion is UTF-8 and routinely non-ASCII
    // (accents / CJK / emoji), so a byte-by-byte `b as char` would map each UTF-8 byte to a Latin-1
    // codepoint — corrupting every multi-byte char into mojibake. All JSON escapes (`\" \\ \/ \n \r \t
    // \uXXXX`) are ASCII, so escape handling reads cleanly char-by-char; the literal path pushes the
    // whole char intact. `\uXXXX` peeks four hex chars (all ASCII) via `by_ref().take(4)`.
    let mut chars = body[start..].chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                // A `\uXXXX` escape: the four hex digits' scalar (BMP only — sufficient for model text; a
                // surrogate pair would need more, but Bedrock rarely emits them here). A malformed/short
                // escape is skipped (best-effort decode, like the rest of this minimal reader).
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                // An unknown escape (or a trailing backslash): push the escaped char verbatim, or stop.
                Some(other) => out.push(other),
                None => return Some(out),
            },
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

#[cfg(all(test, feature = "bedrock"))]
mod bedrock_tests {
    use super::*;

    #[test]
    fn first_text_block_preserves_multibyte_utf8() {
        // A Bedrock completion is UTF-8 and routinely non-ASCII. The decode must round-trip a multi-byte
        // char (accents / CJK / emoji) INTACT — the pr465 bug pushed each UTF-8 byte as a Latin-1 char,
        // producing mojibake and desyncing the scan.
        let body = r#"{"content":[{"type":"text","text":"café 日本語 🎉"}]}"#;
        assert_eq!(
            first_text_block(body).as_deref(),
            Some("café 日本語 🎉"),
            "a multi-byte completion must round-trip intact, not as mojibake"
        );
    }

    #[test]
    fn first_text_block_decodes_escapes_and_stops_at_the_closing_quote() {
        // JSON escapes (all ASCII) decode; the value ends at the first unescaped quote (trailing JSON
        // after it is ignored). A `\uXXXX` BMP escape becomes its char.
        let body = r#"{"text":"a\tb\n\"q\" é end","more":"ignored"}"#;
        assert_eq!(
            first_text_block(body).as_deref(),
            Some("a\tb\n\"q\" é end"),
            "escapes decode and the scan stops at the closing quote"
        );
    }

    #[test]
    fn first_text_block_none_when_absent() {
        assert_eq!(first_text_block(r#"{"content":[]}"#), None);
    }
}
