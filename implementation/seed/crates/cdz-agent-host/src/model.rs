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
use cdz_kernel::effect::{EffectKind, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The I/O half of model invocation, factored out so the executor's logic is hermetically testable.
/// An impl performs the real call (Bedrock over HTTP with SigV4 + the cred-broker — behind `live-net`);
/// a stub returns canned bytes for tests. Total: it returns `Err(reason)` rather than panicking, so the
/// executor can fold a transport failure as an observable `EffectOutcome::Err` (§9d/§17).
///
/// Returns [`Bytes`] (not `Vec<u8>`): a model completion is a hot-path body that gets folded into the
/// log/KV and cloned on the way, so a ref-counted buffer avoids the deep copy (operator perf directive,
/// 2026-08-02). A transport that already has a `Vec<u8>` freezes it with `.into()` (a move, no copy).
pub trait ModelTransport {
    /// Invoke `model_id` with the opaque request `body`, returning the raw response bytes. The
    /// `idempotency_key` lets a side-effecting transport dedup a crash-re-driven dispatch (§16c-S1/D).
    fn invoke(&self, model_id: &str, body: &[u8], idempotency_key: Hash) -> Result<Bytes, String>;
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

impl<T: ModelTransport> Executor for ModelExecutor<T> {
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        if req.kind != EffectKind::Model {
            return EffectOutcome::Err(format!(
                "ModelExecutor only handles Model effects, got {:?}",
                req.kind
            ));
        }
        // A model call needs a request body. Inline bytes are the request; a Blob-ref payload isn't
        // supported yet (the transport would have to fetch it from the blob store — a later slice).
        // Both "wrong shape" cases are observable Errs, never panics (§17).
        let body: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(
                    "ModelExecutor: blob-ref payload not supported yet (inline the request body)"
                        .to_string(),
                );
            }
            None => {
                return EffectOutcome::Err(
                    "ModelExecutor: a Model effect requires a request payload".to_string(),
                );
            }
        };
        match self.transport.invoke(&req.target, body, idempotency_key) {
            // The transport's `Bytes` completion moves straight into `Payload::Inline` (now ref-counted
            // `Bytes` after the kernel's perf-directive flip) — no copy, no conversion.
            Ok(response) => EffectOutcome::Ok(Some(Payload::Inline(response))),
            Err(reason) => EffectOutcome::Err(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport that echoes a canned completion, recording what it was asked to invoke so a test can
    /// assert the executor extracted the model id + body correctly.
    struct StubTransport {
        response: Bytes,
    }
    impl ModelTransport for StubTransport {
        fn invoke(&self, model_id: &str, body: &[u8], _key: Hash) -> Result<Bytes, String> {
            // Prove the executor passed the model id (target) + the request body (payload) through.
            assert_eq!(model_id, "test-model");
            assert_eq!(body, b"prompt");
            Ok(self.response.clone()) // Bytes clone = O(1) refcount bump, not a deep copy
        }
    }

    fn model_req(payload: Option<Payload>) -> EffectRequest {
        EffectRequest {
            kind: EffectKind::Model,
            target: "test-model".to_string(),
            payload,
        }
    }

    #[test]
    fn invokes_the_model_and_returns_the_completion() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::from_static(b"a completion"),
        });
        match exec.perform(
            &model_req(Some(Payload::Inline(b"prompt".to_vec().into()))),
            Hash::of(b"k"),
        ) {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(&bytes[..], b"a completion");
            }
            other => panic!("expected Ok(Inline(completion)), got {other:?}"),
        }
    }

    #[test]
    fn a_missing_payload_is_an_observable_err() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::new(),
        });
        match exec.perform(&model_req(None), Hash::of(b"k")) {
            EffectOutcome::Err(msg) => assert!(msg.contains("requires a request payload"), "{msg}"),
            other => panic!("expected Err for a payload-free Model effect, got {other:?}"),
        }
    }

    #[test]
    fn a_blob_payload_is_an_observable_err_for_now() {
        let mut exec = ModelExecutor::new(StubTransport {
            response: Bytes::new(),
        });
        match exec.perform(
            &model_req(Some(Payload::Blob(Hash::of(b"big-body")))),
            Hash::of(b"k"),
        ) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("blob-ref payload not supported"), "{msg}")
            }
            other => panic!("expected Err for a blob-ref payload, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_failure_folds_as_an_err_not_a_panic() {
        struct FailingTransport;
        impl ModelTransport for FailingTransport {
            fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, String> {
                Err("bedrock throttled (429)".to_string())
            }
        }
        let mut exec = ModelExecutor::new(FailingTransport);
        match exec.perform(
            &model_req(Some(Payload::Inline(b"prompt".to_vec().into()))),
            Hash::of(b"k"),
        ) {
            EffectOutcome::Err(msg) => assert!(msg.contains("throttled"), "{msg}"),
            other => panic!("expected the transport error to fold as Err, got {other:?}"),
        }
    }

    #[test]
    fn non_model_kind_is_an_observable_err() {
        struct NeverCalled;
        impl ModelTransport for NeverCalled {
            fn invoke(&self, _m: &str, _b: &[u8], _k: Hash) -> Result<Bytes, String> {
                panic!("transport must not be called for a non-Model kind");
            }
        }
        let mut exec = ModelExecutor::new(NeverCalled);
        let req = EffectRequest {
            kind: EffectKind::Http,
            target: "https://x/".to_string(),
            payload: Some(Payload::Inline(b"x".to_vec().into())),
        };
        match exec.perform(&req, Hash::of(b"k")) {
            EffectOutcome::Err(msg) => {
                assert!(msg.contains("Model"), "err names the handled kind: {msg}")
            }
            other => panic!("expected Err for a non-Model kind, got {other:?}"),
        }
    }
}
