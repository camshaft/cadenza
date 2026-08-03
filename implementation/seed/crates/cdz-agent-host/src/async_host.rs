//! The async multi-session host loop — cooperatively interleaves many agent sessions on ONE task.
//!
//! [`AgentHost`] (sync) holds the registry + drives one session per call. This wraps it in a
//! **single-threaded async event loop**: sessions feed a shared inbound channel, and one loop `select!`s
//! over that channel + a timer (the earliest armed-timer deadline across all sessions) + a shutdown
//! signal, driving the addressed session IN-PLACE when its input is ready. Many agents run concurrently
//! (interleaved), each still on the single-threaded kernel loop that replay-determinism needs (§15b).
//!
//! **Why single-threaded (not a task per session):** the kernel's `Reducer`/`Authorize`/`Executor` are
//! not `: Send`, and a `ComponentReducer` holds wasmtime types — so a session can't move into its own
//! tokio task. That's by design, confirmed with v-agent-harness: a session's folds MUST be sequential
//! (replay-determinism), and agents are I/O-bound on effects, so cooperative interleaving on one thread
//! saturates the useful concurrency — parallelism across sessions buys nothing. This loop needs no
//! `Send`; sessions never cross a thread.
//!
//! **The kernel-async seam:** `AgentHost::deliver`/`fire_due_timers` are ASYNC (they drive the kernel's
//! `Session::deliver_async`/`fire_due_timers_async`, folding through an [`cdz_kernel::reducer::Reducer`]
//! with `Store::fuel_async_yield_interval`), so a long fold cooperatively YIELDS and this loop interleaves
//! other sessions while it awaits — SAME loop, no reshape, no `Send` (still one task). The in-place
//! `host.deliver(..).await` here is exactly that seam.

use crate::host::{AgentHost, SessionId};
use cdz_kernel::event::EventBody;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::KernelError;
use tokio::sync::mpsc;

/// One inbound delivery to route to a session: its id + the event body + optional cause. The `Inbox`
/// sender clones cheaply, so many producers (network listeners, peer-emit bridges, a test) can feed the
/// one loop.
pub struct Inbound {
    pub session: SessionId,
    pub body: EventBody,
    pub cause: Option<Hash>,
}

/// The sending half a producer holds to deliver events into the host loop. Cloneable (mpsc sender).
pub type Inbox = mpsc::UnboundedSender<Inbound>;

/// The async host: owns the [`AgentHost`] registry and runs the single-threaded multiplexing loop.
/// Construct with [`AsyncAgentHost::new`], hand out [`AsyncAgentHost::inbox`] senders to producers, then
/// [`AsyncAgentHost::run`] the loop (typically the process's main future). A `shutdown` receiver ends it.
pub struct AsyncAgentHost {
    host: AgentHost,
    rx: mpsc::UnboundedReceiver<Inbound>,
    tx: Inbox,
}

impl AsyncAgentHost {
    /// Build over an existing (already-populated) [`AgentHost`] registry.
    pub fn new(host: AgentHost) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        AsyncAgentHost { host, rx, tx }
    }

    /// A cloneable sender to feed inbound events into the loop. Every producer that wants to deliver to a
    /// session holds one of these.
    pub fn inbox(&self) -> Inbox {
        self.tx.clone()
    }

    /// Mutable access to the registry (to `spawn` sessions before/around running the loop). During the
    /// run, spawning is done by delivering to the loop; before the run, use this.
    pub fn host_mut(&mut self) -> &mut AgentHost {
        &mut self.host
    }

    /// Read-only access to the registry — usable BEFORE `run` (which consumes `self`) or on the
    /// `AgentHost` that `run` RETURNS after shutdown. There is no concurrent `&self` while the loop runs
    /// (it owns `self`); a LIVE session-status query is routed THROUGH the event loop (a future inbound
    /// message kind), not a concurrent borrow. So this accessor serves pre-run setup + post-run inspection.
    pub fn host(&self) -> &AgentHost {
        &self.host
    }

    /// Run the single-threaded multiplexing loop until `shutdown` fires OR all inbox senders are dropped
    /// (the channel closes → no more producers → nothing left to serve). Each iteration:
    /// - FIRST fire every due timer (deadline ≤ now) across sessions — done BEFORE the `select!` so a
    ///   continuously-ready inbox can't STARVE a due deadline (a `select!` gives no fairness guarantee, so
    ///   the sleep arm might never be polled under sustained load — PR#1303; firing here bounds a timer's
    ///   lateness to one loop iteration);
    /// - then `select!` over shutdown | the next inbound event | a sleep until the earliest future deadline.
    ///
    /// `now_ms` is a monotonic clock closure so the loop stays testable (a test drives a fake clock); a
    /// real host passes wall-clock ms. Returns `Ok(AgentHost)` on clean shutdown (so the caller can inspect
    /// final state / re-run), or `Err(KernelError)` if a session's fold hit a KERNEL error — that's
    /// corruption / a programming error (a genuine reducer FAULT is instead captured as a `FoldFailed` log
    /// EVENT, not a KernelError), so the loop FAILS FAST + surfaces it rather than swallowing it (PR#1303).
    pub async fn run(
        mut self,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
        mut now_ms: impl FnMut() -> u64,
    ) -> Result<AgentHost, KernelError> {
        // Drop OUR retained sender so the inbox channel closes once every EXTERNAL producer drops its
        // clone. Otherwise `self.tx` would keep the channel open forever and `rx.recv()` would never
        // return `None` — the loop could only ever exit via `shutdown`. (Producers get their senders from
        // `inbox()` BEFORE `run`; the loop itself never needs to send to itself.)
        drop(self.tx);
        loop {
            // Fire any ALREADY-DUE timers up front (deadline ≤ now), before we might block on a ready
            // inbox — this is what stops a busy inbox from starving deadlines (a `select!` has no fairness
            // guarantee). Bounds a timer's lateness to a single iteration.
            if let Some(deadline) = self.host.next_timer_deadline_across_sessions() {
                if deadline <= now_ms() {
                    self.host.fire_due_timers(now_ms()).await;
                    // Loop back: firing may have armed new timers / the inbox may now be ready; re-evaluate.
                    continue;
                }
            }

            // The next FUTURE armed-timer deadline (all due ones fired above). None = no timer armed → the
            // sleep arm never wakes; only inbound/shutdown drive the loop.
            let next_deadline = self.host.next_timer_deadline_across_sessions();
            let sleep = async {
                match next_deadline {
                    Some(deadline) => {
                        let dur_ms = deadline.saturating_sub(now_ms());
                        tokio::time::sleep(tokio::time::Duration::from_millis(dur_ms)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                // Shutdown wins — end the loop promptly.
                _ = &mut shutdown => return Ok(self.host),
                // An inbound event: route it to its session and drive that session's loop in-place.
                maybe = self.rx.recv() => {
                    match maybe {
                        Some(msg) => {
                            match self.host.deliver(&msg.session, msg.body, msg.cause).await {
                                // Delivered + the session ran a turn.
                                Some(Ok(())) => {}
                                // Unknown session id: a no-op (the producer addressed a session that isn't
                                // registered) — a robust host doesn't crash on a stray id.
                                None => {}
                                // A KERNEL error (corruption / programming error — NOT a reducer fault,
                                // which is a FoldFailed event). Not recoverable in-loop: fail fast so an
                                // operator/supervisor sees it, rather than swallowing it (PR#1303).
                                Some(Err(e)) => return Err(e),
                            }
                        }
                        // All senders dropped → no more producers → nothing more to serve.
                        None => return Ok(self.host),
                    }
                }
                // The earliest FUTURE timer came due → fire due timers across sessions (next iteration's
                // up-front check also catches any that became due meanwhile).
                _ = sleep => {
                    self.host.fire_due_timers(now_ms()).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostedSession;
    use crate::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, EffectOutcome, Event};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// On "go", record which session ran by stamping "ran" in KV (via a Now round-trip so it exercises
    /// the real executor path through the loop).
    struct MarkAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MarkAgent {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Now,
                    String::new(),
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(_),
                    ..
                } => {
                    kv.put(b"ran".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn mark_host() -> HostedSession {
        let executor = cdz_kernel::executor::CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        HostedSession::genesis(
            Hash::of(b"mark-v1"),
            Box::new(MarkAgent),
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Now,
                predicate: ResourcePredicate::Any,
            }])),
            executor,
        )
    }

    fn go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    #[tokio::test]
    async fn two_sessions_interleave_on_one_loop() {
        // Two sessions registered; both fed an inbound via the shared inbox; the single loop drives each
        // in turn. After both are delivered + the senders dropped, the loop ends and BOTH ran.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("a"), mark_host());
        host.spawn(SessionId::new("b"), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();

        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Feed both sessions, then DROP the inbox → the loop drains the two messages and returns (all
        // senders gone). A fixed clock (no timers armed here) keeps it deterministic.
        inbox
            .send(Inbound {
                session: SessionId::new("a"),
                body: go(),
                cause: None,
            })
            .unwrap();
        inbox
            .send(Inbound {
                session: SessionId::new("b"),
                body: go(),
                cause: None,
            })
            .unwrap();
        drop(inbox);

        // Run to completion (channel closes after the two messages) → returns the registry to inspect.
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // Both sessions ran their loop through the real ClockExecutor.
        for id in ["a", "b"] {
            assert_eq!(
                host.get(&SessionId::new(id))
                    .unwrap()
                    .session()
                    .kv()
                    .get(b"ran"),
                Some(&b"1"[..]),
                "session {id} ran"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_ends_the_loop_even_with_live_senders() {
        // With a live inbox sender (loop would otherwise wait for input), firing shutdown ends run()
        // promptly — graceful shutdown.
        let host = AgentHost::new();
        let async_host = AsyncAgentHost::new(host);
        let _inbox = async_host.inbox(); // kept alive → channel stays open
        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        sd_tx.send(()).unwrap();
        // Should return via the shutdown arm, not hang.
        let _host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("shutdown returns Ok");
    }

    /// A timer agent: arms a timer at `deadline_ms` on "go"; records "woke" when it fires (PR#1303 fix).
    struct TimerAgent {
        deadline_ms: u64,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerAgent {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Timer,
                    self.deadline_ms.to_string(),
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::TimerFired { .. } => {
                    kv.put(b"woke".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test]
    async fn an_already_due_timer_fires_before_the_loop_blocks_on_the_inbox() {
        // PR#1303 starvation fix: the loop fires due timers UP FRONT (before select!), so a timer whose
        // deadline has already passed at loop entry fires even though the inbox has a queued message the
        // loop would otherwise process first. Arm a timer for t=1000, deliver "go" (arms it) via the
        // registry before running, then run with the clock already at 5000 (past the deadline) → the
        // up-front fire wakes it. Shutdown after so the loop returns.
        let mut host = AgentHost::new();
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        host.spawn(
            SessionId::new("t"),
            HostedSession::genesis(
                Hash::of(b"timer-v1"),
                Box::new(TimerAgent { deadline_ms: 1000 }),
                Box::new(authz),
                CompositeExecutor::new(),
            ),
        );
        // Arm the timer directly (pre-run) so it's already armed when the loop starts.
        host.deliver(&SessionId::new("t"), go(), None).await;
        assert_eq!(
            host.get(&SessionId::new("t"))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            None,
            "not fired yet"
        );

        let async_host = AsyncAgentHost::new(host);
        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Shut down immediately; the clock is already past the deadline, so the up-front fire runs before
        // the shutdown arm is taken.
        sd_tx.send(()).unwrap();
        let host = async_host
            .run(sd_rx, || 5000)
            .await
            .expect("clean shutdown");
        assert_eq!(
            host.get(&SessionId::new("t"))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            Some(&b"1"[..]),
            "an already-due timer fires up front, not starved by shutdown/inbox"
        );
    }
}
