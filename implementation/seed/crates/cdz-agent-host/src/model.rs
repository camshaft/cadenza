//! The `Model` executor — invoke a model and fold the completion back (the headline milestone).
//!
//! When a reducer's `Model` effect actually reaches a model and the completion folds back as an
//! `EffectResult`, an agent LOOPS end-to-end. That is what this executor does. The effect's `target` is
//! the model id (e.g. a Bedrock model id); its `payload` is the opaque request body (a userspace
//! agreement — the kernel doesn't decode it); the completion becomes the result payload.
//!
//! **Transport seam.** The actual model call touches the network and needs credentials, so it can't run
//! in a hermetic gate. This executor is therefore GENERIC over a [`ModelTransport`]: the executor owns
//! the effect-mapping logic (kind check, payload extraction, outcome mapping — all pure and
//! hermetically testable), and the transport owns the I/O (SigV4 signing, the cred-broker, the HTTP
//! call). The real Bedrock transport lands behind the crate's `live-net` feature; a stub transport
//! drives the hermetic tests + the end-to-end agent-loop test. This mirrors the kernel's own seams
//! (`LogSink`, `BlobStore` are traits so the I/O is swappable + fault-injectable).
//!
//! **Idempotency (§16c-S1/D).** A model invoke is a paid, side-effecting call. `perform` receives the
//! kernel's `idempotency_key` and passes it to the transport, so a real Bedrock transport can dedup a
//! re-driven dispatch after a crash (or at least be aware the same key means "the same logical call").

use bytes::Bytes;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The I/O half of model invocation, factored out so the executor's logic is hermetically testable.
/// An impl performs the real call (Bedrock over HTTP with SigV4 + the cred-broker — behind `live-net`);
/// a stub returns canned bytes for tests. Total: it returns `Err(reason)` rather than panicking, so the
/// executor can fold a transport failure as an observable `EffectOutcome::Err` (§9d/§17).
///
/// Returns the response body as [`Bytes`]: a model completion is a hot-path body that gets folded into
/// the log/KV and cloned on the way, so a ref-counted buffer makes a clone an O(1) refcount bump, not a
/// deep copy (operator perf directive). A transport holding a `Vec<u8>` freezes it with `.into()` (a
/// move, no copy).
///
/// **Error classification (supervision):** the `Err` half is a classified
/// [`EffectOutcome`](cdz_kernel::event::EffectOutcome) carrying a typed
/// [`Retryability`](cdz_kernel::event::Retryability), so the kernel supervisor decides backoff-retry vs
/// give-up structurally — a transient failure (Bedrock 429/5xx/timeout, throttle, connection reset) via
/// [`EffectOutcome::err_retryable`](cdz_kernel::event::EffectOutcome::err_retryable), a permanent one
/// (400/auth/malformed) via [`EffectOutcome::err`](cdz_kernel::event::EffectOutcome::err) (`Permanent` is the
/// fail-closed default). The executor folds the returned outcome through unchanged.
/// `#[async_trait(?Send)]` — the invoke is async (a real Bedrock call awaits the socket) and not `Send`
/// (single-threaded host, no cross-thread futures; §15b), matching the kernel's `Executor`/`Reducer`.
#[async_trait::async_trait(?Send)]
pub trait ModelTransport {
    /// Invoke `model_id` with the opaque request `body`, returning the raw response bytes. The
    /// `idempotency_key` lets a side-effecting transport dedup a crash-re-driven dispatch (§16c-S1/D) —
    /// so a supervisor's retry of a RETRYABLE failure re-drives with the same key and doesn't double-charge.
    async fn invoke(
        &self,
        model_id: &str,
        body: &[u8],
        idempotency_key: Hash,
    ) -> Result<Bytes, EffectOutcome>;
}

/// Performs `Model` effects by delegating the network call to a [`ModelTransport`]. Single-KIND: a
/// non-`Model` kind is an observable `Err` (§9d) — register it under `Model` in a
/// [`cdz_kernel::executor::CompositeExecutor`] alongside the other real executors.
pub struct ModelExecutor<T: ModelTransport> {
    transport: T,
}

impl<T: ModelTransport> ModelExecutor<T> {
    pub fn new(transport: T) -> Self {
        ModelExecutor { transport }
    }
}

#[async_trait::async_trait(?Send)]
impl<T: ModelTransport> Executor for ModelExecutor<T> {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
        // These are structural request errors — retrying can't fix them, so they're PERMANENT (§17
        // totality: an observable Err, never a panic).
        // Key the guard on the effect FAMILY STRING (seq-39 / effect-schema slice 2), not the EffectKind
        // enum — the same decision the router and authz make. Decouples this executor from the enum ahead
        // of its retirement; matches_family is the one-source-of-truth family compare.
        if !req.content_type.matches_family(effect_ct::MODEL) {
            return EffectOutcome::err(format!(
                "ModelExecutor only handles the {} family, got {}",
                effect_ct::MODEL,
                req.content_type.family
            ));
        }
        // A model call needs a request body. An inline payload IS the request; a blob-ref payload is
        // rejected because this executor has no blob-store handle to resolve it, and a payload-free Model
        // effect has no request at all. Both are structural (PERMANENT), never panics.
        let body: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            Some(Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "ModelExecutor: blob-ref payload unsupported — this executor has no blob-store access; inline the request body",
                );
            }
            None => {
                return EffectOutcome::err(
                    "ModelExecutor: a Model effect requires a request payload",
                );
            }
        };
        // target = the model id. The target is now opaque Arc<[u8]>; a model id is UTF-8, so a non-UTF-8
        // target is malformed → PERMANENT (fail-closed).
        let model_id = match req.target_str() {
            Ok(m) => m,
            Err(_) => {
                return EffectOutcome::err(
                    "ModelExecutor: the Model target (model id) is not valid UTF-8",
                );
            }
        };
        match self.transport.invoke(model_id, body, idempotency_key).await {
            // The transport's `Bytes` completion moves straight into `Payload::Inline` (ref-counted
            // `Bytes`) — no copy. The transport's Err is already a classified `EffectOutcome::Err` carrying
            // its typed retryability (per the trait contract), so pass it through unchanged.
            Ok(response) => EffectOutcome::Ok(Some(Payload::Inline(response))),
            Err(outcome) => outcome,
        }
    }

    /// This single-kind executor serves exactly the `Model` family — the capability-manifest mechanism
    /// dimension when it's used bare as a `dyn Executor` (in a `CompositeExecutor` the composite's own
    /// `by_family` override answers instead). Overrides the trait's fail-safe `false` default.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::MODEL
    }
}

/// The REAL Bedrock model transport (behind `live-net`) — fills the [`ModelTransport`] seam via
/// `aws-sdk-bedrockruntime`'s `InvokeModel`. This is the headline: with it wired into a
/// [`cdz_kernel::executor::CompositeExecutor`], a reducer's `Model` effect reaches Bedrock and the
/// completion folds back — an agent loops against a real model.
///
/// **Credentials come from the ENVIRONMENT** (operator directive): via the SDK's default credential
/// provider chain — environment variables (`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
/// `AWS_SESSION_TOKEN` + region), the shared config/credentials profile, and IMDS, all part of the DEFAULT
/// chain (not feature-gated). Only SSO and `credentials-process` are `aws-config` feature-gated (`sso` /
/// `credentials-process`) and we don't enable them. No broker, no credential wiring, no Membrain in this
/// repo.
///
/// **Request/response shape.** `invoke`'s `body` is the OPAQUE `InvokeModel` request body (a userspace
/// agreement — the model's native JSON, e.g. the Anthropic Messages schema); `model_id` is the effect
/// target. The returned [`Bytes`] is the raw `InvokeModel` response body, folded back verbatim so the
/// reducer decodes it. The kernel authorized the model-id target before dispatch; this does not
/// re-authorize. Unlike the HTTP client there is no cross-host redirect class to guard — the SDK talks to
/// the standard regional Bedrock endpoint for the resolved model id.
///
/// **Error classification (§17):** a throttle / server / timeout / dispatch failure is
/// `err_retryable`; a request-construction or other service error is `err` (Permanent, the fail-closed
/// default — a misclassification never retries forever).
// `Clone` is cheap: an `aws_sdk_bedrockruntime::Client` is an `Arc`-backed handle over a shared
// `SdkConfig` (connection pool + credential cache), so cloning is a refcount bump, NOT a re-resolution of
// AWS config / IMDS. This lets the daemon build ONE Bedrock client at startup and clone it into a fresh
// per-session executor — the per-install AWS/IMDS load (which stalled the single-threaded loop) moves to
// boot (#1987 review).
#[cfg(feature = "live-net")]
#[derive(Clone)]
pub struct BedrockModelTransport {
    client: aws_sdk_bedrockruntime::Client,
}

#[cfg(feature = "live-net")]
impl BedrockModelTransport {
    /// Build the transport, loading AWS config from the ambient environment (default provider chain +
    /// region from env). Async because the default chain may probe the environment (e.g. IMDS). A missing
    /// region or unresolvable credentials surface later, per-request, as a classified transport `Err` —
    /// construction itself just wires the client to the environment.
    pub async fn new() -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        BedrockModelTransport {
            client: aws_sdk_bedrockruntime::Client::new(&config),
        }
    }

    /// Build from an explicit SDK config (e.g. a caller that already loaded one, or a test/integration
    /// harness pointing at a specific region) instead of the ambient default chain.
    pub fn from_conf(config: &aws_config::SdkConfig) -> Self {
        BedrockModelTransport {
            client: aws_sdk_bedrockruntime::Client::new(config),
        }
    }
}

#[cfg(feature = "live-net")]
#[async_trait::async_trait(?Send)]
impl ModelTransport for BedrockModelTransport {
    async fn invoke(
        &self,
        model_id: &str,
        body: &[u8],
        _idempotency_key: Hash,
    ) -> Result<Bytes, EffectOutcome> {
        use aws_sdk_bedrockruntime::primitives::Blob;

        // GAP-1 tool-calling (ADDITIVE): if `body` decodes as an M1 `model-request` (the kernel's
        // tool-calling codec), route to the Converse API (tools + multi-turn); otherwise it's a legacy
        // opaque `InvokeModel` body — fold it back verbatim as before. A single-shot no-tools invoke can
        // use either path; the decode disambiguates without a mode flag.
        if let Ok(req) = cdz_kernel::event_ast::decode_model_request(body) {
            return self.converse_tool_calling(&req).await;
        }

        let resp = self
            .client
            .invoke_model()
            .model_id(model_id)
            .content_type("application/json")
            .body(Blob::new(body.to_vec()))
            .send()
            .await
            .map_err(classify_bedrock_error)?; // -> EffectOutcome

        // The response body is the model's native completion JSON — folded back verbatim as Bytes.
        Ok(Bytes::from(resp.body.into_inner()))
    }
}

#[cfg(feature = "live-net")]
impl BedrockModelTransport {
    /// GAP-1 tool-calling path: an M1 [`ModelRequest`](cdz_kernel::event_ast::ModelRequest) → a Bedrock
    /// `Converse` call (messages + toolConfig) → an M2
    /// [`ModelResponse`](cdz_kernel::event_ast::ModelResponse), returned as its encoded wire bytes (what the
    /// reducer folds off the effect result). The request/response INTERPRETATION is [`crate::converse`]'s
    /// (hermetically tested); this is the thin aws-sdk translation + the network call. Per the operator
    /// standing-order this is pure MECHANISM (translate + the O4 JSON⟷Document boundary via
    /// [`crate::converse_json`]); the agent-loop policy lives in the reducer.
    async fn converse_tool_calling(
        &self,
        req: &cdz_kernel::event_ast::ModelRequest,
    ) -> Result<Bytes, EffectOutcome> {
        use crate::converse::{
            from_model_request, to_model_response, ConverseResponse, ConverseRole,
        };
        use crate::converse_json::json_bytes_to_document;
        use aws_sdk_bedrockruntime::types::{
            ContentBlock as BdContentBlock, ConversationRole, ConverseOutput,
            InferenceConfiguration, Message, SystemContentBlock, Tool, ToolConfiguration,
            ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolSpecification,
            ToolUseBlock,
        };
        use cdz_kernel::event_ast::{encode_model_response, ContentBlock as KContentBlock};

        // Decode + interpret M1 → transport-agnostic ConverseRequest (system-hoist, role-normalize). A
        // malformed request (unknown role) is a PERMANENT structural error the reducer built.
        let cr = from_model_request(req).map_err(|e| EffectOutcome::err(e.to_string()))?;

        // ── Build the Converse call ──────────────────────────────────────────────────────────────────
        let mut call = self.client.converse().model_id(&cr.model_id);

        // system prompts (Bedrock's separate top-level field).
        for s in &cr.system {
            call = call.system(SystemContentBlock::Text(s.clone()));
        }

        // messages: each ConverseMessage → a Bedrock Message, each kernel ContentBlock → a Bedrock one.
        for m in &cr.messages {
            let role = match m.role {
                ConverseRole::User => ConversationRole::User,
                ConverseRole::Assistant => ConversationRole::Assistant,
            };
            let mut msg = Message::builder().role(role);
            for block in &m.content {
                let bd = match block {
                    KContentBlock::Text(t) => BdContentBlock::Text(t.clone()),
                    KContentBlock::ToolCall { id, name, input } => {
                        let input_doc =
                            json_bytes_to_document(input).map_err(EffectOutcome::err)?;
                        BdContentBlock::ToolUse(
                            ToolUseBlock::builder()
                                .tool_use_id(id)
                                .name(name)
                                .input(input_doc)
                                .build()
                                .map_err(|e| EffectOutcome::err(format!("tool-use block: {e}")))?,
                        )
                    }
                    KContentBlock::ToolResult { id, result } => {
                        let result_doc =
                            json_bytes_to_document(result).map_err(EffectOutcome::err)?;
                        BdContentBlock::ToolResult(
                            ToolResultBlock::builder()
                                .tool_use_id(id)
                                .content(ToolResultContentBlock::Json(result_doc))
                                .build()
                                .map_err(|e| {
                                    EffectOutcome::err(format!("tool-result block: {e}"))
                                })?,
                        )
                    }
                };
                msg = msg.content(bd);
            }
            let msg = msg
                .build()
                .map_err(|e| EffectOutcome::err(format!("message: {e}")))?;
            call = call.messages(msg);
        }

        // tools → toolConfig (each schema's JSON bytes → the tool's inputSchema Document).
        if !cr.tools.is_empty() {
            let mut tc = ToolConfiguration::builder();
            for t in &cr.tools {
                let schema_doc = json_bytes_to_document(&t.schema).map_err(EffectOutcome::err)?;
                let spec = ToolSpecification::builder()
                    .name(&t.name)
                    .input_schema(ToolInputSchema::Json(schema_doc))
                    .build()
                    .map_err(|e| EffectOutcome::err(format!("tool spec: {e}")))?;
                tc = tc.tools(Tool::ToolSpec(spec));
            }
            let tc = tc
                .build()
                .map_err(|e| EffectOutcome::err(format!("tool config: {e}")))?;
            call = call.tool_config(tc);
        }

        // max_tokens → inferenceConfig (Bedrock wants i32; clamp a huge value rather than overflow).
        if let Some(mt) = cr.max_tokens {
            let mt = i32::try_from(mt).unwrap_or(i32::MAX);
            call = call.inference_config(InferenceConfiguration::builder().max_tokens(mt).build());
        }

        // ── Call + map the output back to M2 ─────────────────────────────────────────────────────────
        let out = call.send().await.map_err(classify_converse_error)?;
        let stop_reason = out.stop_reason().as_str().to_string();
        // The assistant's output message → kernel content blocks (Text + ToolCall; a response never carries
        // a ToolResult). A missing/non-message output is an empty content list (stop_reason still drives the
        // reducer — e.g. end_turn with no text is a valid, if terse, done).
        let content = match out.output() {
            Some(ConverseOutput::Message(m)) => m
                .content()
                .iter()
                .filter_map(bedrock_block_to_kernel)
                .collect(),
            _ => Vec::new(),
        };
        let resp = ConverseResponse {
            stop_reason,
            content,
        };
        let mr = to_model_response(&resp);
        Ok(Bytes::from(encode_model_response(&mr)))
    }
}

/// Map a Bedrock output [`ContentBlock`](aws_sdk_bedrockruntime::types::ContentBlock) to a kernel
/// [`ContentBlock`](cdz_kernel::event_ast::ContentBlock) for the M2 response: `Text` → `Text`, `ToolUse` →
/// `ToolCall` (its input `Document` → JSON bytes, O4). Any other Bedrock block kind (image/document/… — a
/// model won't emit these unprompted for a text+tools request) is dropped (`None`), so the response carries
/// only what the reducer's fold understands.
#[cfg(feature = "live-net")]
fn bedrock_block_to_kernel(
    block: &aws_sdk_bedrockruntime::types::ContentBlock,
) -> Option<cdz_kernel::event_ast::ContentBlock> {
    use aws_sdk_bedrockruntime::types::ContentBlock as BdContentBlock;
    use cdz_kernel::event_ast::ContentBlock as KContentBlock;
    match block {
        BdContentBlock::Text(t) => Some(KContentBlock::Text(t.clone())),
        BdContentBlock::ToolUse(tu) => Some(KContentBlock::ToolCall {
            id: tu.tool_use_id().to_string(),
            name: tu.name().to_string(),
            input: crate::converse_json::document_to_json_bytes(tu.input()),
        }),
        _ => None,
    }
}

/// Classify a Bedrock `InvokeModel` error into the supervision retryability token (§17). Throttling and
/// server-side (5xx) errors are transient → retryable; a construction/dispatch failure (timeout, connect)
/// is also retryable; everything else (validation, access-denied, model-not-found) is permanent
/// (fail-closed — retrying can't fix a 4xx).
#[cfg(feature = "live-net")]
fn classify_bedrock_error(
    e: aws_sdk_bedrockruntime::error::SdkError<
        aws_sdk_bedrockruntime::operation::invoke_model::InvokeModelError,
    >,
) -> EffectOutcome {
    use aws_sdk_bedrockruntime::error::SdkError;
    // Transport-level failures that never reached, or didn't cleanly complete at, the service → transient:
    // a timeout, a dispatch (connect/IO) failure, or a ResponseError (a reply arrived but was unparseable /
    // truncated — typically a mid-stream drop, worth a retry). All are retryable; only a completed SERVICE
    // error carries a status we classify below.
    let transient_transport = matches!(
        e,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_)
    );
    if transient_transport {
        return EffectOutcome::err_retryable(format!("Bedrock transport failure: {e}"));
    }
    // A completed service error: throttling / 5xx are transient; other 4xx are permanent.
    if let SdkError::ServiceError(ref svc) = e {
        let err = svc.err();
        if err.is_throttling_exception() || err.is_model_timeout_exception() {
            return EffectOutcome::err_retryable(format!("Bedrock throttled/timed out: {e}"));
        }
        if err.is_internal_server_exception() || err.is_service_unavailable_exception() {
            return EffectOutcome::err_retryable(format!("Bedrock server error: {e}"));
        }
    }
    EffectOutcome::err(format!("Bedrock invoke failed: {e}"))
}

/// Classify a Bedrock `Converse` error (§17), same policy as [`classify_bedrock_error`] but over the Converse
/// operation's error type: transport failures (timeout/dispatch/response) + throttling/model-timeout/5xx are
/// RETRYABLE; every other service error (validation, access-denied, model-not-found) is PERMANENT
/// (fail-closed). Kept separate because the SDK's error enum is per-operation.
#[cfg(feature = "live-net")]
fn classify_converse_error(
    e: aws_sdk_bedrockruntime::error::SdkError<
        aws_sdk_bedrockruntime::operation::converse::ConverseError,
    >,
) -> EffectOutcome {
    use aws_sdk_bedrockruntime::error::SdkError;
    let transient_transport = matches!(
        e,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_)
    );
    if transient_transport {
        return EffectOutcome::err_retryable(format!("Bedrock Converse transport failure: {e}"));
    }
    if let SdkError::ServiceError(ref svc) = e {
        let err = svc.err();
        if err.is_throttling_exception() || err.is_model_timeout_exception() {
            return EffectOutcome::err_retryable(format!(
                "Bedrock Converse throttled/timed out: {e}"
            ));
        }
        if err.is_internal_server_exception() || err.is_service_unavailable_exception() {
            return EffectOutcome::err_retryable(format!("Bedrock Converse server error: {e}"));
        }
    }
    EffectOutcome::err(format!("Bedrock Converse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event::Retryability;

    /// A transport that echoes a canned completion, recording what it was asked to invoke so a test can
    /// assert the executor extracted the model id + body correctly.
    struct StubTransport {
        response: Bytes,
    }
    #[async_trait::async_trait(?Send)]
    impl ModelTransport for StubTransport {
        async fn invoke(
            &self,
            model_id: &str,
            body: &[u8],
            _key: Hash,
        ) -> Result<Bytes, EffectOutcome> {
            // Prove the executor passed the model id (target) + the request body (payload) through.
            assert_eq!(model_id, "test-model");
            assert_eq!(body, b"prompt");
            Ok(self.response.clone()) // Bytes clone = O(1) refcount bump, not a deep copy
        }
    }

    fn model_req(payload: Option<Payload>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::MODEL,
            "test-model",
            payload,
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn invokes_the_model_and_returns_the_completion() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::from_static(b"a completion"),
        });
        match exec
            .perform(
                EffectId(0),
                &model_req(Some(Payload::Inline(b"prompt".to_vec().into()))),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(&bytes[..], b"a completion");
            }
            other => panic!("expected Ok(Inline(completion)), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_payload_is_an_observable_err() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::new(),
        });
        match exec
            .perform(EffectId(0), &model_req(None), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("requires a request payload"), "{message}");
                // A structural request error is PERMANENT — a supervisor must not retry it.
                assert_eq!(retryability, Retryability::Permanent, "{message}");
            }
            other => panic!("expected Err for a payload-free Model effect, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_blob_payload_is_a_permanent_err_no_blob_store_access() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::new(),
        });
        match exec
            .perform(
                EffectId(0),
                &model_req(Some(Payload::Blob(Hash::of(b"big-body")))),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                // Rejected because this executor has no blob-store access (an invariant, not a "yet").
                assert!(message.contains("no blob-store access"), "{message}");
                assert_eq!(retryability, Retryability::Permanent, "{message}");
            }
            other => panic!("expected Err for a blob-ref payload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_transient_transport_failure_stays_retryable_through_the_executor() {
        // A transport classifies its own error; the executor passes the reason through unchanged, so a
        // RETRYABLE transport failure (e.g. a Bedrock throttle) reaches the supervisor still RETRYABLE.
        struct ThrottledTransport;
        #[async_trait::async_trait(?Send)]
        impl ModelTransport for ThrottledTransport {
            async fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, EffectOutcome> {
                Err(EffectOutcome::err_retryable("bedrock throttled (429)"))
            }
        }
        let mut exec = ModelExecutor::new(ThrottledTransport);
        match exec
            .perform(
                EffectId(0),
                &model_req(Some(Payload::Inline(b"prompt".to_vec().into()))),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(message.contains("throttled"), "{message}");
                assert_eq!(retryability, Retryability::Retryable, "{message}");
            }
            other => panic!("expected the transport error to fold as Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_model_family_is_a_permanent_err() {
        struct NeverCalled;
        #[async_trait::async_trait(?Send)]
        impl ModelTransport for NeverCalled {
            async fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, EffectOutcome> {
                panic!("transport must not be called for a non-model-family effect");
            }
        }
        let mut exec = ModelExecutor::new(NeverCalled);
        // A request in the http family (not model): the guard keys on the family string now, so this is
        // rejected as PERMANENT and the transport is never touched.
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://x/",
            Some(Payload::Inline(b"x".to_vec().into())),
            Timeliness::Interactive,
        );
        match exec.perform(EffectId(0), &req, Hash::of(b"k")).await {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert!(
                    message.contains(effect_ct::MODEL) && message.contains(effect_ct::HTTP),
                    "err names the handled family (model) + the rejected one (http): {message}"
                );
                assert_eq!(retryability, Retryability::Permanent, "{message}");
            }
            other => panic!("expected Err for a non-model-family effect, got {other:?}"),
        }
    }

    #[test]
    fn handles_only_the_model_family() {
        // Bare-leaf mechanism dimension: serves Model, nothing else (the trait default false otherwise).
        let exec = ModelExecutor::new(StubTransport {
            response: Bytes::from_static(b""),
        });
        assert!(exec.handles_family(effect_ct::MODEL));
        assert!(!exec.handles_family(effect_ct::NOW));
        assert!(!exec.handles_family(effect_ct::HTTP));
        assert!(!exec.handles_family("embedding"));
    }

    // ---- an agent RUNS a model-invocation loop end-to-end through the ModelExecutor (converted from the
    // deleted model_agent_e2e integration test, operator no-integration-tests mandate — hermetic: a Session
    // + Rust reducers + the real ModelExecutor/ClockExecutor over STUB transports, no network). Beyond the
    // isolated effect-mapping units above: the completion drives the agent loop + replays, a CompositeExecutor
    // routes Now+Model for one agent, and a SEC-F1 model-id grant denies an off-id call before the transport. ----
    use crate::clock::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, EffectKind, ResourcePredicate};
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kernel::Session;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A canned-completion transport for the agent loop (distinct from the effect-mapping `StubTransport`
    /// above): echoes `<model_id> says: <prompt>` so the agent can prove the executor threaded id + body.
    struct CannedModel;
    #[async_trait::async_trait(?Send)]
    impl ModelTransport for CannedModel {
        async fn invoke(
            &self,
            model_id: &str,
            body: &[u8],
            _key: Hash,
        ) -> Result<Bytes, EffectOutcome> {
            Ok(
                format!("{model_id} says: {}", String::from_utf8_lossy(body))
                    .into_bytes()
                    .into(),
            )
        }
    }

    /// A minimal agent that calls a model: on "go" it emits a `Model` effect (a prompt); when the completion
    /// comes back it records it and marks itself `answered`.
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

    fn model_go() -> EventBody {
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
            .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(CannedModel)));
        let mut session = Session::genesis(
            Hash::of(b"model-agent-v1"),
            Hash::of(b"model-agent-v1-nonce"),
        );
        let captured = crate::testutil::log_capture::attach_recording_sink(&mut session);

        session
            .deliver(model_go(), None, &mut ModelAgent, &model_cap(), &mut exec)
            .await
            .unwrap();

        // The loop closed: the agent prompted the model, the executor invoked the transport, and the
        // completion folded back and advanced the agent to `answered`.
        assert_eq!(
            session.kv().get(b"phase").as_deref(),
            Some(&b"answered"[..])
        );
        let completion = session
            .kv()
            .get(b"completion")
            .expect("completion recorded");
        assert_eq!(
            String::from_utf8_lossy(&completion),
            "claude-test says: hello",
            "the executor threaded model id + prompt through the transport and folded the completion back"
        );
        assert_eq!(session.open_effects(), 0);

        // Replay-equivalence: the completion is recorded, so replay reconstructs the identical KV without
        // ever calling the transport again (a paid model call happens once; replay is free). Replay from the
        // durable-log SOURCE (recording sink), not a resident Vec (I5).
        let replayed = Session::replay(
            crate::testutil::log_capture::replay_input(&captured),
            &mut reducer,
        )
        .await
        .unwrap();
        assert_eq!(
            replayed.kv().get(b"phase").as_deref(),
            Some(&b"answered"[..])
        );
        assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
    }

    #[tokio::test]
    async fn one_composite_routes_both_now_and_model_for_one_agent() {
        // A real agent both reads the clock AND calls a model — the CompositeExecutor routes each kind to its
        // own real executor.
        struct ClockThenModel;
        #[async_trait::async_trait(?Send)]
        impl Reducer for ClockThenModel {
            async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
                match &event.body {
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
                    } => match kv.get(b"phase").as_deref() {
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
            .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(CannedModel)));
        let mut session = Session::genesis(
            Hash::of(b"clock-then-model-v1"),
            Hash::of(b"clock-then-model-v1-nonce"),
        );

        session
            .deliver(model_go(), None, &mut ClockThenModel, &authz, &mut exec)
            .await
            .unwrap();

        // Both kinds routed to their own real executor across the multi-step loop.
        assert_eq!(session.kv().get(b"phase").as_deref(), Some(&b"done"[..]));
        assert!(session.kv().get(b"at").is_some(), "the clock leg ran");
        assert_eq!(
            String::from_utf8_lossy(&session.kv().get(b"completion").unwrap()),
            "claude-test says: hi"
        );
        assert_eq!(session.open_effects(), 0);
    }

    #[tokio::test]
    async fn a_model_call_to_an_unpermitted_id_is_denied_before_the_transport() {
        // Deny-by-default (SEC-F1): the grant is for `claude-test`; an agent prompting a DIFFERENT model id is
        // denied at the gate — the transport (a paid call) is never reached. A panic-if-called transport
        // proves the executor was never consulted.
        struct RefusedModel;
        #[async_trait::async_trait(?Send)]
        impl ModelTransport for RefusedModel {
            async fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, EffectOutcome> {
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
            .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(RefusedModel)));
        let mut session = Session::genesis(
            Hash::of(b"wrong-model-v1"),
            Hash::of(b"wrong-model-v1-nonce"),
        );
        let captured = crate::testutil::log_capture::attach_recording_sink(&mut session);
        session
            .deliver(
                model_go(),
                None,
                &mut WrongModelAgent,
                &model_cap(),
                &mut exec,
            )
            .await
            .unwrap();

        // Denied at the gate → a denial is on the log (read from the recording sink, I5) and nothing open.
        assert!(captured
            .borrow()
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));
        assert_eq!(session.open_effects(), 0);
    }
}
