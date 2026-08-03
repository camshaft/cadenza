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

use crate::retry;
use bytes::Bytes;
use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
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
/// **Error classification (supervision, [`crate::retry`]):** an `Err(reason)` MUST lead with a
/// retryability token so the kernel supervisor can decide backoff-retry vs give-up — a transient failure
/// (Bedrock 429/5xx/timeout, throttle, connection reset) as [`crate::retry::retryable`], a permanent one
/// (400/auth/malformed) as [`crate::retry::permanent`]. An unprefixed reason is treated PERMANENT
/// (fail-closed), so forgetting the prefix means "not retried," never "retried forever."
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
    ) -> Result<Bytes, String>;
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
    async fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        // These are structural request errors — retrying can't fix them, so they're PERMANENT (§17
        // totality: an observable Err, never a panic).
        // Key the guard on the effect FAMILY STRING (seq-39 / effect-schema slice 2), not the EffectKind
        // enum — the same decision the router and authz make. Decouples this executor from the enum ahead
        // of its retirement; matches_family is the one-source-of-truth family compare.
        if !req.content_type.matches_family(effect_ct::MODEL) {
            return EffectOutcome::Err(retry::permanent(format!(
                "ModelExecutor only handles the {} family, got {}",
                effect_ct::MODEL,
                req.content_type.family
            )));
        }
        // A model call needs a request body. An inline payload IS the request; a blob-ref payload is
        // rejected because this executor has no blob-store handle to resolve it, and a payload-free Model
        // effect has no request at all. Both are structural (PERMANENT), never panics.
        let body: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(retry::permanent(
                    "ModelExecutor: blob-ref payload unsupported — this executor has no blob-store access; inline the request body",
                ));
            }
            None => {
                return EffectOutcome::Err(retry::permanent(
                    "ModelExecutor: a Model effect requires a request payload",
                ));
            }
        };
        match self
            .transport
            .invoke(&req.target, body, idempotency_key)
            .await
        {
            // The transport's `Bytes` completion moves straight into `Payload::Inline` (ref-counted
            // `Bytes`) — no copy. The transport's Err reason already carries its own retryability token
            // (RETRYABLE:/PERMANENT:, per the trait contract), so pass it through unchanged.
            Ok(response) => EffectOutcome::Ok(Some(Payload::Inline(response))),
            Err(reason) => EffectOutcome::Err(reason),
        }
    }

    /// This single-kind executor serves exactly the `Model` family — the capability-manifest mechanism
    /// dimension when it's used bare as a `dyn Executor` (in a `CompositeExecutor` the composite's own
    /// `by_family` override answers instead). Overrides the trait's fail-safe `false` default.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::MODEL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;

    /// A transport that echoes a canned completion, recording what it was asked to invoke so a test can
    /// assert the executor extracted the model id + body correctly.
    struct StubTransport {
        response: Bytes,
    }
    #[async_trait::async_trait(?Send)]
    impl ModelTransport for StubTransport {
        async fn invoke(&self, model_id: &str, body: &[u8], _key: Hash) -> Result<Bytes, String> {
            // Prove the executor passed the model id (target) + the request body (payload) through.
            assert_eq!(model_id, "test-model");
            assert_eq!(body, b"prompt");
            Ok(self.response.clone()) // Bytes clone = O(1) refcount bump, not a deep copy
        }
    }

    fn model_req(payload: Option<Payload>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::MODEL,
            "test-model".to_string(),
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
        match exec.perform(&model_req(None), Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("requires a request payload"), "{msg}");
                // A structural request error is PERMANENT — a supervisor must not retry it.
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
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
                &model_req(Some(Payload::Blob(Hash::of(b"big-body")))),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err(msg) => {
                // Rejected because this executor has no blob-store access (an invariant, not a "yet").
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
    async fn a_transient_transport_failure_stays_retryable_through_the_executor() {
        // A transport classifies its own error; the executor passes the reason through unchanged, so a
        // RETRYABLE transport failure (e.g. a Bedrock throttle) reaches the supervisor still RETRYABLE.
        struct ThrottledTransport;
        #[async_trait::async_trait(?Send)]
        impl ModelTransport for ThrottledTransport {
            async fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, String> {
                Err(retry::retryable("bedrock throttled (429)"))
            }
        }
        let mut exec = ModelExecutor::new(ThrottledTransport);
        match exec
            .perform(
                &model_req(Some(Payload::Inline(b"prompt".to_vec().into()))),
                Hash::of(b"k"),
            )
            .await
        {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("throttled"), "{msg}");
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
    async fn a_non_model_family_is_a_permanent_err() {
        struct NeverCalled;
        #[async_trait::async_trait(?Send)]
        impl ModelTransport for NeverCalled {
            async fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, String> {
                panic!("transport must not be called for a non-model-family effect");
            }
        }
        let mut exec = ModelExecutor::new(NeverCalled);
        // A request in the http family (not model): the guard keys on the family string now, so this is
        // rejected as PERMANENT and the transport is never touched.
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://x/".to_string(),
            Some(Payload::Inline(b"x".to_vec().into())),
            Timeliness::Interactive,
        );
        match exec.perform(&req, Hash::of(b"k")).await {
            EffectOutcome::Err(msg) => {
                assert!(
                    msg.contains(effect_ct::MODEL) && msg.contains(effect_ct::HTTP),
                    "err names the handled family (model) + the rejected one (http): {msg}"
                );
                assert_eq!(
                    retry::classify(&msg),
                    retry::Retryability::Permanent,
                    "{msg}"
                );
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
}
