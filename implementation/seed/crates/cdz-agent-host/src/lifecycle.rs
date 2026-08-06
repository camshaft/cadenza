//! [`LifecycleExecutor`] — the `lifecycle/*` effect executor: a session controls ANOTHER session's
//! lifecycle (§lifecycle I3/I5). A reducer performs a `lifecycle/terminate` (or spawn/suspend/resume)
//! effect; the kernel authorizes it (SEC-F1, the Cedar `DescendantOf` authority — I6) and dispatches it
//! here, and the host mutates the session registry accordingly.
//!
//! **Why DEFER-TO-LOOP (not inline in `perform`):** a lifecycle op mutates the [`AgentHost`] registry
//! (terminate → mark+remove a session; spawn → insert a child), but [`Executor::perform`] runs DURING the
//! controller session's own `deliver`, while the [`AgentHost`] registry is mutably borrowed for exactly
//! that session (`sessions.get_mut(id) → s.deliver(.., &mut s.executor)`). So `perform` CANNOT touch the
//! registry — it's a borrow conflict, not a runtime deadlock. Instead this executor RECORDS the requested
//! op onto a channel (like [`EmitExecutor`](crate::EmitExecutor) records an [`Inbound`](crate::Inbound) onto
//! the shared inbox) and returns a provisional [`EffectOutcome`]; the [`AsyncAgentHost`](crate::AsyncAgentHost)
//! loop drains the channel and APPLIES the registry mutation AFTER `deliver` returns (where `&mut host` is
//! free again — the same shape the admin `pending_admin` slot uses). Agreed with v-agent-harness as the
//! host mechanism; the kernel `lifecycle/` family routes via the normal authorize→executor path (no kernel
//! drive-loop arm).
//!
//! v1 scope: `lifecycle/terminate` (target = the peer session id to terminate; the durable `Terminated`
//! marker + registry removal happen in the loop via [`AgentHost::terminate`]). `lifecycle/spawn` and
//! suspend/resume are follow-on sub-slices on the same channel + apply mechanism.

use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectRequest};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use tokio::sync::mpsc;

/// A lifecycle op RECORDED by a [`LifecycleExecutor`] for the loop to apply after `deliver` returns
/// (defer-to-loop, see the module doc). Carries the CONTROLLER's identity (`by` = the emitting session's
/// genesis-hash = its SessionId) so the durable marker records who did it + a future Cedar `DescendantOf`
/// (I6) can be re-checked against it.
#[derive(Debug, Clone)]
pub enum LifecycleOp {
    /// Terminate `target`: the loop drives [`AgentHost::terminate`](crate::AgentHost::terminate) (append the
    /// durable `Terminated{by, reason}` marker + remove from the registry). `by` is the controller session.
    Terminate {
        target: SessionId,
        by: SessionId,
        reason: String,
    },
}

/// The channel a [`LifecycleExecutor`] records ops onto; the [`AsyncAgentHost`](crate::AsyncAgentHost) loop
/// drains it after each `deliver`. Cloneable (mpsc sender), so every session's executor feeds the one loop.
pub type LifecycleChannel = mpsc::UnboundedSender<LifecycleOp>;

/// Executes `lifecycle/*` effects by RECORDING the requested registry mutation for the host loop to apply
/// after `deliver` (defer-to-loop — see the module doc). Holds the loop's [`LifecycleChannel`] + the OWNER
/// session id (the controller performing the effect, stamped as `by` on the op — known at wiring time, like
/// [`EmitExecutor`](crate::EmitExecutor)'s `owner`).
pub struct LifecycleExecutor {
    channel: LifecycleChannel,
    owner: SessionId,
}

impl LifecycleExecutor {
    /// Build over the loop's [`LifecycleChannel`] for the session `owner` (the controller whose
    /// `CompositeExecutor` this registers in under the `lifecycle/*` families). `owner` is stamped as `by`
    /// on every recorded op (who terminated/spawned).
    pub fn new(channel: LifecycleChannel, owner: SessionId) -> Self {
        LifecycleExecutor { channel, owner }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for LifecycleExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        // v1: lifecycle/terminate only. A non-lifecycle family is structural → PERMANENT (§17).
        if !req
            .content_type
            .matches_family(effect_ct::LIFECYCLE_TERMINATE)
        {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "LifecycleExecutor v1 handles only {}, got {}",
                effect_ct::LIFECYCLE_TERMINATE,
                req.content_type.family
            )));
        }
        // `target` is the peer session id to terminate (raw SessionId string). Empty = structural PERMANENT.
        if req.target.is_empty() {
            return EffectOutcome::Err(crate::retry::permanent(
                "LifecycleExecutor: lifecycle/terminate requires a non-empty target (the session id to terminate)",
            ));
        }
        let target = SessionId::new(req.target.clone());
        // A session terminating ITSELF via lifecycle/terminate is rejected here — self-termination is the
        // `close` path (§6a), not the controller-terminates-a-peer path this family is for. (Also avoids the
        // loop mutating the very session it's mid-deliver on.)
        if target == self.owner {
            return EffectOutcome::Err(crate::retry::permanent(
                "LifecycleExecutor: a session cannot lifecycle/terminate itself (use close); target == controller",
            ));
        }
        // The reason rides the effect payload as opaque bytes IF present (host-authored diagnostic; the
        // guest may pass a UTF-8 reason). Lossy-decode to a String for the durable marker; a non-UTF-8 or
        // absent payload yields an empty reason (the marker's `reason` is diagnostic, not load-bearing).
        let reason = match &req.payload {
            Some(cdz_kernel::effect::Payload::Inline(bytes)) => {
                String::from_utf8_lossy(bytes).into_owned()
            }
            _ => String::new(),
        };
        // RECORD the op for the loop to apply after deliver (defer-to-loop). Returns provisionally: Ok(None)
        // = the terminate request was accepted + enqueued (NOT that the target is gone yet; the loop applies
        // it after this deliver returns). A closed channel = host shutting down → RETRYABLE.
        let op = LifecycleOp::Terminate {
            target,
            by: self.owner.clone(),
            reason,
        };
        match self.channel.send(op) {
            Ok(()) => EffectOutcome::Ok(None),
            Err(_) => EffectOutcome::Err(crate::retry::retryable(
                "LifecycleExecutor: the host loop lifecycle channel is closed — cannot record the terminate (host shutting down?)",
            )),
        }
    }

    fn handles_family(&self, family: &str) -> bool {
        // v1: terminate. (spawn/suspend/resume join here as follow-on sub-slices.)
        family == effect_ct::LIFECYCLE_TERMINATE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{Payload, Timeliness};

    fn terminate_req(target: &str, reason: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_TERMINATE,
            target.to_string(),
            reason.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn terminate_records_a_lifecycle_op_for_the_loop() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("controller"));
        let out = exec
            .perform(
                &terminate_req("victim", Some(b"operator kill")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "a recorded terminate acks Ok(None) provisionally, got {out:?}"
        );
        match rx.try_recv().expect("a LifecycleOp was recorded") {
            LifecycleOp::Terminate { target, by, reason } => {
                assert_eq!(target.as_str(), "victim");
                assert_eq!(by.as_str(), "controller", "by = the controller (owner)");
                assert_eq!(reason, "operator kill", "reason decoded from the payload");
            }
        }
    }

    #[tokio::test]
    async fn a_payloadless_terminate_records_an_empty_reason() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let out = exec
            .perform(&terminate_req("b", None), Hash::of(b"k"))
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        match rx.try_recv().expect("recorded") {
            LifecycleOp::Terminate { reason, .. } => assert_eq!(reason, ""),
        }
    }

    #[tokio::test]
    async fn terminating_self_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("me"));
        let out = exec
            .perform(&terminate_req("me", None), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("cannot lifecycle/terminate itself")),
            "self-terminate is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_target_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let out = exec.perform(&terminate_req("", None), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("non-empty target")),
            "empty target is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_closed_channel_is_a_retryable_error() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let out = exec
            .perform(&terminate_req("b", None), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("RETRYABLE:") && r.contains("channel is closed")),
            "a closed lifecycle channel is RETRYABLE, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_non_lifecycle_family_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "b".to_string(),
            None,
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        assert!(matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:")));
        assert!(
            exec.handles_family(effect_ct::LIFECYCLE_TERMINATE)
                && !exec.handles_family(effect_ct::HTTP)
        );
    }
}
