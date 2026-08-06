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
//! Scope: `lifecycle/terminate` (durable `Terminated` marker + registry removal via
//! [`AgentHost::terminate`]) + `lifecycle/suspend` / `lifecycle/resume` (flip the host-scheduler bit via
//! [`AgentHost::suspend`]/[`AgentHost::resume`] — the loop then holds/replays the target's inbound). All
//! three RECORD a [`LifecycleOp`] the loop applies after deliver. `lifecycle/spawn` is driven separately
//! (via [`AgentHost::spawn_child`], which records the parent→child edge) — a follow-on arm.

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
    /// Suspend `target`: the loop drives [`AgentHost::suspend`](crate::AgentHost::suspend) (flip the
    /// host-scheduler bit — the loop then holds the target's inbound, no log mutation). `by` is the controller.
    Suspend { target: SessionId, by: SessionId },
    /// Resume `target`: the loop drives [`AgentHost::resume`](crate::AgentHost::resume) (clear the bit — held
    /// inbound replays). `by` is the controller.
    Resume { target: SessionId, by: SessionId },
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
        let family = req.content_type.family.as_ref();
        // Dispatch by family: terminate / suspend / resume (register-by-string). A non-lifecycle family is
        // structural → PERMANENT (§17). Suspend/resume take no payload; terminate carries an optional reason.
        let is_terminate = req
            .content_type
            .matches_family(effect_ct::LIFECYCLE_TERMINATE);
        let is_suspend = req
            .content_type
            .matches_family(effect_ct::LIFECYCLE_SUSPEND);
        let is_resume = req.content_type.matches_family(effect_ct::LIFECYCLE_RESUME);
        if !(is_terminate || is_suspend || is_resume) {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "LifecycleExecutor handles only lifecycle/terminate|suspend|resume, got {family}"
            )));
        }
        // `target` is the peer session id (raw SessionId string). Empty = structural PERMANENT.
        if req.target.is_empty() {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "LifecycleExecutor: {family} requires a non-empty target (the peer session id)"
            )));
        }
        let target = SessionId::new(req.target.clone());
        // A session controlling ITSELF via lifecycle/* is rejected — self-lifecycle is the `close`/own-loop
        // path, not the controller-controls-a-peer path this family is for (and it avoids the loop mutating
        // the very session it's mid-deliver on).
        if target == self.owner {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "LifecycleExecutor: a session cannot {family} itself; target == controller"
            )));
        }
        let by = self.owner.clone();
        // RECORD the op for the loop to apply after deliver (defer-to-loop). Ok(None) = accepted + enqueued
        // (NOT applied yet — the loop applies it after this deliver returns). A closed channel = host
        // shutting down → RETRYABLE.
        let op = if is_terminate {
            // The reason rides the payload as opaque bytes IF present (host-authored diagnostic; guest may
            // pass a UTF-8 reason). Inline → lossy-decode (non-UTF-8 → replacement chars, NOT empty); None →
            // empty; Blob → PERMANENT (mirrors Http/Emit — no blob-store handle; silent-drop hides a bug).
            let reason = match &req.payload {
                Some(cdz_kernel::effect::Payload::Inline(bytes)) => {
                    String::from_utf8_lossy(bytes).into_owned()
                }
                None => String::new(),
                Some(cdz_kernel::effect::Payload::Blob(_)) => {
                    return EffectOutcome::Err(crate::retry::permanent(
                        "LifecycleExecutor: a blob-ref reason is unsupported — this executor has no blob-store access; inline the reason (or omit it)",
                    ));
                }
            };
            LifecycleOp::Terminate { target, by, reason }
        } else if is_suspend {
            LifecycleOp::Suspend { target, by }
        } else {
            LifecycleOp::Resume { target, by }
        };
        match self.channel.send(op) {
            Ok(()) => EffectOutcome::Ok(None),
            Err(_) => EffectOutcome::Err(crate::retry::retryable(
                "LifecycleExecutor: the host loop lifecycle channel is closed — cannot record the op (host shutting down?)",
            )),
        }
    }

    fn handles_family(&self, family: &str) -> bool {
        // terminate + suspend + resume. (lifecycle/spawn is driven differently — via AgentHost::spawn_child
        // in a follow-on arm.)
        matches!(
            family,
            effect_ct::LIFECYCLE_TERMINATE
                | effect_ct::LIFECYCLE_SUSPEND
                | effect_ct::LIFECYCLE_RESUME
        )
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
        let LifecycleOp::Terminate { target, by, reason } =
            rx.try_recv().expect("a LifecycleOp was recorded")
        else {
            panic!("expected a Terminate op");
        };
        assert_eq!(target.as_str(), "victim");
        assert_eq!(by.as_str(), "controller", "by = the controller (owner)");
        assert_eq!(reason, "operator kill", "reason decoded from the payload");
    }

    #[tokio::test]
    async fn a_payloadless_terminate_records_an_empty_reason() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let out = exec
            .perform(&terminate_req("b", None), Hash::of(b"k"))
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let LifecycleOp::Terminate { reason, .. } = rx.try_recv().expect("recorded") else {
            panic!("expected a Terminate op");
        };
        assert_eq!(reason, "");
    }

    #[tokio::test]
    async fn suspend_and_resume_record_their_ops_and_handles_family_covers_all_three() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        // handles_family covers terminate + suspend + resume, not an unrelated family.
        assert!(exec.handles_family(effect_ct::LIFECYCLE_TERMINATE));
        assert!(exec.handles_family(effect_ct::LIFECYCLE_SUSPEND));
        assert!(exec.handles_family(effect_ct::LIFECYCLE_RESUME));
        assert!(!exec.handles_family(effect_ct::HTTP));

        let suspend = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_SUSPEND,
            "victim".to_string(),
            None,
            Timeliness::Interactive,
        );
        assert!(matches!(
            exec.perform(&suspend, Hash::of(b"k")).await,
            EffectOutcome::Ok(None)
        ));
        let LifecycleOp::Suspend { target, by } = rx.try_recv().expect("suspend recorded") else {
            panic!("expected a Suspend op");
        };
        assert_eq!(target.as_str(), "victim");
        assert_eq!(by.as_str(), "ctl");

        let resume = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_RESUME,
            "victim".to_string(),
            None,
            Timeliness::Interactive,
        );
        assert!(matches!(
            exec.perform(&resume, Hash::of(b"k")).await,
            EffectOutcome::Ok(None)
        ));
        let LifecycleOp::Resume { target, .. } = rx.try_recv().expect("resume recorded") else {
            panic!("expected a Resume op");
        };
        assert_eq!(target.as_str(), "victim");
    }

    #[tokio::test]
    async fn suspending_self_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("me"));
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_SUSPEND,
            "me".to_string(),
            None,
            Timeliness::Interactive,
        );
        assert!(
            matches!(&exec.perform(&req, Hash::of(b"k")).await,
                EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("cannot lifecycle/suspend itself")),
            "self-suspend is PERMANENT"
        );
    }

    #[tokio::test]
    async fn a_blob_ref_reason_is_a_permanent_error() {
        // A blob-ref reason can't be resolved here (no blob-store handle) — reject PERMANENT, mirroring
        // Http/Emit, rather than silently dropping a provided reason (#2425 review consistency).
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"));
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_TERMINATE,
            "victim".to_string(),
            Some(Payload::Blob(Hash::of(b"some-blob"))),
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("blob-ref reason is unsupported")),
            "a blob-ref reason is rejected PERMANENT, got {out:?}"
        );
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
