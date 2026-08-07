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
//! [`AgentHost::suspend`]/[`AgentHost::resume`] — the loop then holds/replays the target's inbound) +
//! `lifecycle/spawn` (create a child running a payload `reducer_hash`, via
//! [`AgentHost::spawn_child_with_nonce`], recording the parent→child edge). All RECORD a [`LifecycleOp`] the
//! loop applies after deliver. Spawn is distinct: it takes no peer `target`, RETURNS the child's SessionId
//! synchronously (pre-computed via [`Session::derive_genesis_hash`](cdz_kernel::kernel::Session::derive_genesis_hash)),
//! and the loop registers the child with the SAME nonce so the returned id matches. The loop-apply
//! MATERIALIZES the child's reducer from `reducer_hash` via the session factory
//! ([`SessionFactory::build_spawned`](crate::SessionFactory)) then registers it — the full spawn path
//! (executor pre-compute → loop factory-resolve + register) is wired end to end.

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
    /// Spawn a CHILD under `parent` (§lifecycle I3): the loop drives
    /// [`AgentHost::spawn_child_with_nonce`](crate::AgentHost::spawn_child_with_nonce) — materializes a child
    /// from `reducer_hash` (via the loop's session factory), registers it under its genesis-hash id, and
    /// records the parent→child edge. `parent` is the controller (= the emitting session, whose SessionId is
    /// its genesis-hash-hex). `spawn_nonce` is minted ONCE by the executor + carried here so the loop builds
    /// the child with the SAME nonce the executor pre-computed the returned `child_id` from (via
    /// [`Session::derive_genesis_hash`](cdz_kernel::kernel::Session::derive_genesis_hash)) — the id the loop
    /// registers then matches the id the reducer already folded byte-for-byte. `child_id` is carried so the
    /// loop registers under the exact pre-computed id (belt-and-suspenders; it also re-derives).
    Spawn {
        parent: SessionId,
        reducer_hash: Hash,
        spawn_nonce: Hash,
        child_id: SessionId,
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
    /// The owner session's GENESIS HASH — its provenance identity (`Session::genesis_hash()`), threaded at
    /// wiring time. Used as the PARENT genesis when this owner spawns a child (`perform_spawn`), instead of
    /// re-parsing the owner SessionId as hex. The id is a host registry LABEL (may be a vanity string like
    /// "concierge" for a root/named supervisor); the genesis hash is the stable identity `derive_genesis_hash`
    /// needs. Threading the Hash (not `Hash::from_hex(owner)`) keeps a vanity-id supervisor spawn-capable
    /// (#2484 c1 / tick-#784 class: SessionId is opaque, resolve by genesis-hash content, never assume hex==id).
    owner_genesis: Hash,
}

impl LifecycleExecutor {
    /// Build over the loop's [`LifecycleChannel`] for the session `owner` (the controller whose
    /// `CompositeExecutor` this registers in under the `lifecycle/*` families). `owner` is stamped as `by`
    /// on every recorded op (who terminated/spawned); `owner_genesis` is that session's genesis hash
    /// (`Session::genesis_hash()`), the parent-provenance for a spawn — threaded so it's never re-parsed from
    /// the id string (which may be a vanity label, not genesis-hex).
    pub fn new(channel: LifecycleChannel, owner: SessionId, owner_genesis: Hash) -> Self {
        LifecycleExecutor {
            channel,
            owner,
            owner_genesis,
        }
    }

    /// Handle `lifecycle/spawn` (§lifecycle I3): the OWNER (parent) spawns a CHILD running `reducer_hash`.
    /// Unlike terminate/suspend/resume this CREATES a session + RETURNS the child's `SessionId` synchronously
    /// as the effect result, so the parent's reducer folds the id immediately — while the loop registers the
    /// child AFTER `deliver` (defer-to-loop). To make the returned id REAL (match what the loop registers),
    /// we PRE-COMPUTE it: mint the spawn nonce ONCE here, derive the child genesis-hash from
    /// `(reducer_hash, nonce, Some(parent_genesis))` via [`Session::derive_genesis_hash`], and hand BOTH the
    /// nonce and the pre-computed id to the loop in [`LifecycleOp::Spawn`] so it builds the child with the
    /// SAME nonce (see [`AgentHost::spawn_child_with_nonce`](crate::AgentHost::spawn_child_with_nonce)).
    ///
    /// `reducer_hash` rides the payload as 32 raw bytes (Inline) — the child reducer's content hash the loop's
    /// factory materializes. `parent_genesis` is `self.owner`'s SessionId parsed back to a `Hash` (the id IS
    /// the parent's genesis-hash-hex). Returns `Ok(Some(child_id_hex_bytes))` = accepted + the id (NOT yet
    /// registered — the loop does that). A missing/malformed reducer_hash payload, or a non-hex owner, is a
    /// structural PERMANENT; a closed channel is RETRYABLE.
    fn perform_spawn(&self, req: &EffectRequest) -> EffectOutcome {
        // reducer_hash rides the payload as 32 raw bytes. None / wrong-length / Blob = structural PERMANENT
        // (a spawn with no reducer identity is meaningless; a blob-ref can't be resolved here).
        let reducer_hash = match &req.payload {
            Some(cdz_kernel::effect::Payload::Inline(bytes)) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(bytes);
                Hash::from_bytes(arr)
            }
            _ => {
                return EffectOutcome::Err(crate::retry::permanent(
                    "LifecycleExecutor: lifecycle/spawn requires the child reducer_hash as a 32-byte inline payload",
                ));
            }
        };
        // The parent genesis hash = the owner's GENESIS HASH, threaded at wiring time (NOT re-parsed from the
        // owner id string). This is what keeps a VANITY-id supervisor ("concierge") spawn-capable: the id is a
        // registry label, but derive_genesis_hash needs the stable provenance identity (§tick-#784 / #2484 c1
        // — SessionId is opaque, resolve provenance by genesis-hash, never assume hex==id).
        let parent = self.owner.clone();
        let parent_genesis = self.owner_genesis;
        // Mint the nonce ONCE + pre-compute the child id from the SAME triple the loop will build with, so
        // the id returned here == the id the loop registers (byte-for-byte). This is the load-bearing bit.
        let spawn_nonce = crate::host::mint_spawn_nonce();
        let child_hash = cdz_kernel::kernel::Session::derive_genesis_hash(
            reducer_hash,
            spawn_nonce,
            Some(parent_genesis),
        );
        let child_id = SessionId::new(child_hash.to_hex());
        let op = LifecycleOp::Spawn {
            parent,
            reducer_hash,
            spawn_nonce,
            child_id: child_id.clone(),
        };
        match self.channel.send(op) {
            // Return the pre-computed child id as the effect result — the parent's reducer gets the id NOW,
            // the loop registers the child (with the same nonce) after this deliver returns.
            Ok(()) => EffectOutcome::Ok(Some(cdz_kernel::effect::Payload::Inline(
                child_id.as_str().as_bytes().to_vec().into(),
            ))),
            Err(_) => EffectOutcome::Err(crate::retry::retryable(
                "LifecycleExecutor: the host loop lifecycle channel is closed — cannot record the spawn op (host shutting down?)",
            )),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for LifecycleExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        let family = req.content_type.family.as_ref();
        // SPAWN is structurally distinct from terminate/suspend/resume (no peer `target` — it CREATES a
        // child; the reducer identity rides the payload; it RETURNS the child's SessionId synchronously). So
        // dispatch it first, on its own path.
        if req.content_type.matches_family(effect_ct::LIFECYCLE_SPAWN) {
            return self.perform_spawn(req);
        }
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
                "LifecycleExecutor handles only lifecycle/spawn|terminate|suspend|resume, got {family}"
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
        } else {
            // suspend / resume take NO payload (they flip a scheduler bit — there's no reason string to
            // carry). A caller that attaches one is a structural mistake: reject PERMANENT rather than
            // silently drop it, mirroring terminate's Blob guard (#2452 Copilot c3 — suspend/resume were
            // building their op WITHOUT reading req.payload, so a payload, esp. a Blob ref, vanished silently
            // — the exact inconsistency terminate's exhaustive match guards against).
            if req.payload.is_some() {
                return EffectOutcome::Err(crate::retry::permanent(format!(
                    "LifecycleExecutor: {family} takes no payload (it flips a scheduler bit); drop the payload"
                )));
            }
            if is_suspend {
                LifecycleOp::Suspend { target, by }
            } else {
                LifecycleOp::Resume { target, by }
            }
        };
        match self.channel.send(op) {
            Ok(()) => EffectOutcome::Ok(None),
            Err(_) => EffectOutcome::Err(crate::retry::retryable(
                "LifecycleExecutor: the host loop lifecycle channel is closed — cannot record the op (host shutting down?)",
            )),
        }
    }

    fn handles_family(&self, family: &str) -> bool {
        // spawn + terminate + suspend + resume — the full lifecycle/* family this executor serves.
        matches!(
            family,
            effect_ct::LIFECYCLE_SPAWN
                | effect_ct::LIFECYCLE_TERMINATE
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
    async fn spawn_records_an_op_and_returns_the_pre_computed_child_id() {
        // §lifecycle I3 spawn executor: lifecycle/spawn returns the child's SessionId SYNCHRONOUSLY (the
        // pre-computed derive_genesis_hash id) + records a LifecycleOp::Spawn for the loop to register with
        // the SAME nonce. The owner (parent) SessionId must be a genesis-hash-hex so its provenance parses.
        let parent_hash = Hash::of(b"parent-reducer");
        let parent_id = SessionId::new(parent_hash.to_hex());
        let reducer_hash = Hash::of(b"child-reducer");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, parent_id.clone(), parent_hash);
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_SPAWN,
            String::new(), // spawn has no peer target
            Some(Payload::Inline(reducer_hash.as_bytes().to_vec().into())),
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        // The returned id is the effect result (Ok(Some(payload=child_id_hex))).
        let EffectOutcome::Ok(Some(Payload::Inline(returned))) = &out else {
            panic!("spawn returns Ok(Some(child_id)), got {out:?}");
        };
        let returned_id = String::from_utf8(returned.to_vec()).unwrap();
        // The recorded op carries the same child_id + reducer_hash + parent, and a nonce.
        let LifecycleOp::Spawn {
            parent,
            reducer_hash: op_reducer,
            spawn_nonce,
            child_id,
        } = rx.try_recv().expect("a Spawn op was recorded")
        else {
            panic!("expected a Spawn op");
        };
        assert_eq!(parent, parent_id, "op carries the parent (owner)");
        assert_eq!(
            op_reducer, reducer_hash,
            "op carries the child reducer_hash"
        );
        assert_eq!(
            child_id.as_str(),
            returned_id,
            "the returned id == the recorded op's child_id"
        );
        // The returned id is EXACTLY derive_genesis_hash(reducer, nonce, Some(parent)) — the load-bearing
        // match the loop relies on to register the same id.
        let expected = cdz_kernel::kernel::Session::derive_genesis_hash(
            reducer_hash,
            spawn_nonce,
            Some(parent_hash),
        );
        assert_eq!(
            returned_id,
            expected.to_hex(),
            "the returned child id == derive_genesis_hash of the (reducer, nonce, parent) triple"
        );
    }

    #[tokio::test]
    async fn spawn_without_a_reducer_hash_payload_is_permanent() {
        let parent_id = SessionId::new(Hash::of(b"p").to_hex());
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, parent_id, Hash::of(b"p"));
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_SPAWN,
            String::new(),
            None, // no reducer_hash
            Timeliness::Interactive,
        );
        assert!(
            matches!(&exec.perform(&req, Hash::of(b"k")).await,
                EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("reducer_hash")),
            "spawn with no reducer_hash payload is PERMANENT"
        );
    }

    #[tokio::test]
    async fn spawn_with_a_vanity_id_owner_succeeds_using_the_threaded_genesis_hash() {
        // §tick-#784 fix (regression pin): a parent whose SessionId is a VANITY string ("concierge") — NOT
        // genesis-hash-hex — CAN spawn. The provenance comes from the owner's GENESIS HASH threaded at wiring
        // time (not Hash::from_hex(id)), so a vanity-id supervisor is spawn-capable (the self-hosting use
        // case). The child id is derived from that genesis, independent of the vanity id string.
        let owner_genesis = Hash::of(b"concierge-genesis");
        let reducer_hash = Hash::of(b"worker-reducer");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("concierge"), owner_genesis);
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_SPAWN,
            String::new(),
            Some(Payload::Inline(reducer_hash.as_bytes().to_vec().into())),
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        // Spawn ACCEPTED (Ok) despite the vanity id — and the returned child id == derive_genesis_hash from
        // the THREADED genesis (not from the id string).
        let EffectOutcome::Ok(Some(Payload::Inline(bytes))) = &out else {
            panic!("expected Ok(child_id) for a vanity-id owner spawn, got {out:?}");
        };
        let expected_child = cdz_kernel::kernel::Session::derive_genesis_hash(
            reducer_hash,
            // the nonce the op recorded (read it off the channel below) — assert the id matches that op.
            match rx.try_recv().expect("a Spawn op was recorded") {
                LifecycleOp::Spawn {
                    spawn_nonce,
                    child_id,
                    parent,
                    ..
                } => {
                    assert_eq!(
                        parent.as_str(),
                        "concierge",
                        "op records the vanity owner as parent"
                    );
                    // child_id in the op == the returned id == derive from (reducer, nonce, owner_genesis).
                    assert_eq!(
                        child_id.as_str().as_bytes(),
                        &bytes[..],
                        "returned child id == the op's child_id"
                    );
                    spawn_nonce
                }
                other => panic!("expected Spawn op, got {other:?}"),
            },
            Some(owner_genesis),
        );
        assert_eq!(
            String::from_utf8_lossy(bytes),
            expected_child.to_hex(),
            "child id derives from the THREADED owner genesis, not the vanity id string"
        );
    }

    #[tokio::test]
    async fn terminate_records_a_lifecycle_op_for_the_loop() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec =
            LifecycleExecutor::new(tx, SessionId::new("controller"), Hash::of(b"controller"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
    async fn suspend_or_resume_with_a_payload_is_a_permanent_error() {
        // #2452 Copilot c3: suspend/resume take NO payload (they flip a scheduler bit). A caller-attached
        // payload (esp. a Blob ref) must be rejected PERMANENT, not silently dropped — mirroring terminate's
        // payload guard, the inconsistency the finding flagged.
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
        for family in [effect_ct::LIFECYCLE_SUSPEND, effect_ct::LIFECYCLE_RESUME] {
            // Inline payload → PERMANENT.
            let inline = EffectRequest::new_with_family(
                family,
                "victim".to_string(),
                Some(Payload::Inline(b"unexpected".to_vec().into())),
                Timeliness::Interactive,
            );
            assert!(
                matches!(&exec.perform(&inline, Hash::of(b"k")).await,
                    EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("takes no payload")),
                "{family} with an inline payload is PERMANENT"
            );
            // Blob-ref payload → PERMANENT (the specific silent-drop the finding called out).
            let blob = EffectRequest::new_with_family(
                family,
                "victim".to_string(),
                Some(Payload::Blob(Hash::of(b"some-blob"))),
                Timeliness::Interactive,
            );
            assert!(
                matches!(&exec.perform(&blob, Hash::of(b"k")).await,
                    EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("takes no payload")),
                "{family} with a blob-ref payload is PERMANENT (no silent drop)"
            );
        }
    }

    #[tokio::test]
    async fn suspending_self_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("me"), Hash::of(b"me"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("me"), Hash::of(b"me"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
        let mut exec = LifecycleExecutor::new(tx, SessionId::new("ctl"), Hash::of(b"ctl"));
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
