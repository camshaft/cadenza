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
//! **The kernel-async seam:** today `AgentHost::deliver`/`fire_due_timers` are synchronous, so a single
//! session's fold blocks this loop for its duration (still fine: sessions interleave between deliveries,
//! and a fold is short). When v-agent-harness lands the kernel-async conversion (async
//! `deliver`/`apply` + `Store::fuel_async_yield_interval`), the in-place `host.deliver(..)` here becomes
//! `host.deliver(..).await` and a long fold cooperatively YIELDS — SAME loop, no reshape, no `Send`
//! (still one task). That's the clean swap seam this shape is built for.

use crate::host::{AgentHost, SessionId};
use cdz_kernel::event::EventBody;
use cdz_kernel::hash::Hash;
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

    /// Read-only access to the registry (for a status query over a running host — the session-status
    /// surface reads this).
    pub fn host(&self) -> &AgentHost {
        &self.host
    }

    /// Run the single-threaded multiplexing loop until `shutdown` fires OR all inbox senders are dropped
    /// (the channel closes → no more producers → nothing left to serve). Each iteration:
    /// - if an inbound event is ready, deliver it to its session in-place (drives that session's loop);
    /// - else if the earliest armed timer across all sessions is due, fire due timers (waking those
    ///   reducers);
    /// - else sleep until the next timer deadline (or park on the inbox if no timer is armed).
    ///
    /// `now_ms` is supplied by a monotonic clock closure so the loop stays testable (a test drives a fake
    /// clock); a real host passes wall-clock ms. Returns the [`AgentHost`] registry when shut down, so the
    /// caller can inspect final session state (or re-run).
    pub async fn run(
        mut self,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
        mut now_ms: impl FnMut() -> u64,
    ) -> AgentHost {
        // Drop OUR retained sender so the inbox channel closes once every EXTERNAL producer drops its
        // clone. Otherwise `self.tx` would keep the channel open forever and `rx.recv()` would never
        // return `None` — the loop could only ever exit via `shutdown`. (Producers get their senders from
        // `inbox()` BEFORE `run`; the loop itself never needs to send to itself.)
        drop(self.tx);
        loop {
            // The next armed-timer deadline across ALL sessions (the timer wheel). None = no timers armed
            // anywhere → the loop only wakes on inbound/shutdown.
            let next_deadline = self.host.next_timer_deadline_across_sessions();
            let sleep = async {
                match next_deadline {
                    Some(deadline) => {
                        let now = now_ms();
                        let dur_ms = deadline.saturating_sub(now);
                        tokio::time::sleep(tokio::time::Duration::from_millis(dur_ms)).await;
                    }
                    // No timer armed → never wake from this arm (only inbound/shutdown drive the loop).
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                // Shutdown wins — end the loop promptly.
                _ = &mut shutdown => return self.host,
                // An inbound event: route it to its session and drive that session's loop in-place.
                maybe = self.rx.recv() => {
                    match maybe {
                        Some(msg) => {
                            // Unknown-session delivery is a no-op (the producer addressed a session that
                            // isn't registered) — a robust host doesn't crash on a stray id.
                            let _ = self.host.deliver(&msg.session, msg.body, msg.cause);
                        }
                        // All senders dropped → no more producers → nothing more to serve.
                        None => return self.host,
                    }
                }
                // The earliest timer came due → fire every due timer across sessions.
                _ = sleep => {
                    self.host.fire_due_timers(now_ms());
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
        Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, EffectOutcome, Event};
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// On "go", record which session ran by stamping "ran" in KV (via a Now round-trip so it exercises
    /// the real executor path through the loop).
    struct MarkAgent;
    impl Reducer for MarkAgent {
        fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest {
                    kind: EffectKind::Now,
                    target: String::new(),
                    payload: None,
                    timeliness: Timeliness::Interactive,
                }]),
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
            .with(EffectKind::Now, Box::new(ClockExecutor::new()));
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
        let host = async_host.run(sd_rx, || 0).await;

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
        let _host = async_host.run(sd_rx, || 0).await;
    }
}
