//! The event (system) reducer interface (`design/cadenza-platform.md` §4).
//!
//! Every effect a reducer emits is shepherded by a **system reducer**, instantiated once per event. It is a
//! separate, privileged interface from the ordinary [`Reducer`](crate::Reducer): where an ordinary reducer
//! reacts to responses, messages, and notifications and emits requests, an event reducer is driven by
//! dispatch-lifecycle **signals** and emits direct **commands** for the kernel to carry out. It is still a
//! fold — a signal and its own state produce commands and a new state — but its vocabulary differs, so it is
//! its own trait.
//!
//! The kernel's role is small: on an emitted effect it looks up the event reducer for the effect's contract
//! (the override registry, §3), instantiates it, and drives it with signals — a dispatch starting, a reducer
//! it ran returning, a monitored reducer exiting, an armed timer firing — carrying out the [`Command`]s the
//! reducer returns. Everything else is the event reducer's own state: the handler chains it routes through,
//! the request context (the leaf's token, accumulated grants, lineage), and the single-use capabilities it
//! grants to handlers and enforces itself. The kernel mints nothing and tracks none of it; its whole
//! contribution to that enforcement is the unforgeable [`Origin`] it stamps on every message (§3), which the
//! event reducer checks against its own record of who holds which capability.
//!
//! The [`Command`] set is therefore just what a reducer cannot do to its own state: run a reducer with an
//! event, arm a timer, monitor a reducer, and retire the context once the dispatch resolves. Answering and
//! forwarding are not their own commands — they are running the next reducer with a [`Delivered::Response`]
//! or [`Delivered::Message`] — and attaching a grant or minting a capability is the event reducer's own
//! bookkeeping, not a command.

use crate::{Bytes, Hash, Message, Notification, Origin, Outcome, Request, Response};
use async_trait::async_trait;
use std::time::Duration;

/// The effect to shepherd, handed to [`on_dispatch_start`](EventReducer::on_dispatch_start): what a leaf
/// reducer emitted. `leaf` and `continuation_token` are where the eventual answer folds back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    /// The contract-id being performed.
    pub contract: Hash,
    /// The input value, in its canonical encoding.
    pub payload: Bytes,
    /// The reducer (and host) that emitted the effect — where the answer folds back to.
    pub leaf: Origin,
    /// The leaf's correlation token, returned on the answer it eventually receives.
    pub continuation_token: Bytes,
    /// The leaf's optional deadline; the event reducer arms a timer for it and enforces the timeout.
    pub deadline: Option<Duration>,
}

/// The result a reducer produced when the event reducer ran it, delivered to
/// [`on_handler_returned`](EventReducer::on_handler_returned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerResult {
    /// The reducer that ran.
    pub reducer: Hash,
    /// The requests it emitted (its forwards, answers, and its own effects).
    pub requests: Vec<Request>,
    /// Whether it kept running or terminated.
    pub outcome: Outcome,
}

/// An event the event reducer directs the kernel to deliver to a reducer, via [`Command::RunReducer`]. The
/// three variants select the ordinary reducer's three entry points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivered {
    /// Deliver to `on_message` — an effect performed on the reducer.
    Message(Message),
    /// Deliver to `on_response` — a reply to a request the reducer performed.
    Response(Response),
    /// Deliver to `on_notification` — a control-plane event.
    Notification(Notification),
}

/// A direct command an event reducer emits for the kernel to carry out (§4). This is the whole set — exactly
/// what a reducer cannot do to its own state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Run `reducer`, delivering `event` to the entry point its variant selects; the reducer's result comes
    /// back on [`on_handler_returned`](EventReducer::on_handler_returned). Answering a caller and forwarding
    /// up a chain are both this — running the next reducer with a [`Delivered::Response`] or
    /// [`Delivered::Message`].
    RunReducer {
        /// The reducer to run.
        reducer: Hash,
        /// The event to deliver to it.
        event: Delivered,
    },
    /// Arm a timer; when it fires, [`on_timer_fired`](EventReducer::on_timer_fired) is called with `token`.
    /// This is how a deadline is enforced: the kernel owns only the raw timer, and what a timeout means is
    /// the event reducer's own policy.
    ArmTimer {
        /// How long until the timer fires.
        after: Duration,
        /// The token returned on [`on_timer_fired`](EventReducer::on_timer_fired), so the reducer knows
        /// which timer fired.
        token: Bytes,
    },
    /// Watch `reducer`; if it exits or crashes, [`on_monitor_fired`](EventReducer::on_monitor_fired) is
    /// called with its id, so the event reducer can turn a dead handler into a failure that bubbles down.
    Monitor {
        /// The reducer to watch.
        reducer: Hash,
    },
    /// The dispatch has resolved; the context — this instance's state — may be collected.
    RetireContext,
}

/// The per-event system reducer interface (§4): driven by dispatch-lifecycle signals, emitting [`Command`]s.
/// A separate, privileged interface from the ordinary [`Reducer`](crate::Reducer). `Send + Sync` so the
/// runtime can schedule it; `&mut self` because each signal folds into the context it holds. Async so a
/// signal may await the reducer's direct accesses (its key-value store, the content store) without blocking.
#[async_trait]
pub trait EventReducer: Send + Sync {
    /// A leaf emitted an effect; begin a dispatch for it.
    async fn on_dispatch_start(&mut self, effect: Effect) -> Vec<Command>;

    /// A reducer this event reducer ran has returned its requests and outcome.
    async fn on_handler_returned(&mut self, result: HandlerResult) -> Vec<Command>;

    /// A monitored reducer exited or crashed.
    async fn on_monitor_fired(&mut self, reducer: Hash) -> Vec<Command>;

    /// An armed timer fired, carrying the `token` chosen when it was armed.
    async fn on_timer_fired(&mut self, token: Bytes) -> Vec<Command>;
}

/// A minimal reference [`EventReducer`]: one handler per contract, no chain and no grants yet. It routes a
/// fresh effect to its handler (or answers `MissingHandler` when there is none), arms and enforces the
/// deadline, and retires the context. It is enough to drive the interface end to end; the handler chain,
/// forwarding, and the single-use capabilities are built on top of it in later slices.
///
/// It is instantiated per dispatch, so it holds one dispatch's state: where the answer folds back (`leaf`,
/// `token`) and the `contract`, set when the dispatch starts. Its `handlers` map is its configuration —
/// which reducer answers which contract — the stand-in for the handler chains a fuller event reducer keeps.
pub struct DirectDispatch {
    handlers: std::collections::HashMap<Hash, Hash>,
    leaf: Option<Origin>,
    token: Option<Bytes>,
    contract: Option<Hash>,
}

impl DirectDispatch {
    /// A dispatcher configured with `handlers`, a map from a contract-id to the single reducer that answers
    /// it. A contract absent from the map has no handler, and an effect against it answers `MissingHandler`.
    #[must_use]
    pub fn new(handlers: std::collections::HashMap<Hash, Hash>) -> Self {
        Self {
            handlers,
            leaf: None,
            token: None,
            contract: None,
        }
    }

    /// The token the reference arms its deadline timer with, so [`on_timer_fired`](Self::on_timer_fired)
    /// recognizes the deadline. A distinct value from any real correlation token.
    const DEADLINE_TOKEN: &'static [u8] = b"deadline";

    /// The response that folds back to the leaf carrying `payload`, or `None` before a dispatch has started.
    fn answer_to_leaf(&self, payload: Result<Bytes, crate::Error>) -> Option<Command> {
        Some(Command::RunReducer {
            reducer: self.leaf?.reducer,
            event: Delivered::Response(Response {
                id: self.contract?,
                continuation_token: self.token.clone()?,
                payload,
            }),
        })
    }
}

#[async_trait]
impl EventReducer for DirectDispatch {
    async fn on_dispatch_start(&mut self, effect: Effect) -> Vec<Command> {
        self.leaf = Some(effect.leaf);
        self.token = Some(effect.continuation_token.clone());
        self.contract = Some(effect.contract);
        match self.handlers.get(&effect.contract).copied() {
            // A handler answers this contract: deliver the effect to it as a message (with the leaf as the
            // source), watch it, and arm the deadline if the leaf set one. Its answer arrives later, once
            // forwarding and the response capability are built (a later slice).
            Some(handler) => {
                let mut commands = vec![
                    Command::RunReducer {
                        reducer: handler,
                        event: Delivered::Message(Message {
                            id: effect.contract,
                            payload: effect.payload,
                            from: effect.leaf,
                            continuation_token: effect.continuation_token,
                        }),
                    },
                    Command::Monitor { reducer: handler },
                ];
                if let Some(after) = effect.deadline {
                    commands.push(Command::ArmTimer {
                        after,
                        token: Bytes::from_static(Self::DEADLINE_TOKEN),
                    });
                }
                commands
            }
            // No handler answers the contract: fold `MissingHandler` straight back to the leaf and retire.
            None => {
                let mut commands = Vec::new();
                commands.extend(self.answer_to_leaf(Err(crate::Error::MissingHandler)));
                commands.push(Command::RetireContext);
                commands
            }
        }
    }

    async fn on_handler_returned(&mut self, _result: HandlerResult) -> Vec<Command> {
        // The handler answers by forwarding or responding against the capability it was granted; routing
        // those, and completing the dispatch on a response, is the next slice. Until then the reference has
        // nothing to do when a handler returns.
        Vec::new()
    }

    async fn on_monitor_fired(&mut self, _reducer: Hash) -> Vec<Command> {
        // A watched handler died with its obligation open. The reference retires the context; turning the
        // death into a failure the leaf receives needs an `Error` variant for a failed handler (the set is
        // `Timeout | MissingHandler` today), a decision left open — so this is deliberately minimal.
        vec![Command::RetireContext]
    }

    async fn on_timer_fired(&mut self, token: Bytes) -> Vec<Command> {
        // The deadline elapsed: fold `Timeout` back to the leaf and retire. Any other token is unknown to
        // the reference and does nothing.
        if token.as_ref() == Self::DEADLINE_TOKEN {
            let mut commands = Vec::new();
            commands.extend(self.answer_to_leaf(Err(crate::Error::Timeout)));
            commands.push(Command::RetireContext);
            commands
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Delivered, DirectDispatch, Effect, EventReducer};
    use crate::{Bytes, Error, Hash, Origin};
    use std::collections::HashMap;
    use std::time::Duration;

    fn leaf() -> Origin {
        Origin {
            reducer: Hash::of(b"leaf"),
            host: Hash::of(b"host-a"),
        }
    }

    fn effect(contract: &[u8], deadline: Option<Duration>) -> Effect {
        Effect {
            contract: Hash::of(contract),
            payload: Bytes::from_static(b"input"),
            leaf: leaf(),
            continuation_token: Bytes::from_static(b"leaf-token"),
            deadline,
        }
    }

    #[tokio::test]
    async fn routes_a_fresh_effect_to_its_handler_and_watches_it() {
        let handler = Hash::of(b"http-edge");
        let mut ev = DirectDispatch::new(HashMap::from([(Hash::of(b"http.get"), handler)]));
        let commands = ev.on_dispatch_start(effect(b"http.get", None)).await;
        // It delivers the effect to the handler as a message from the leaf, and watches the handler.
        match &commands[0] {
            Command::RunReducer {
                reducer,
                event: Delivered::Message(m),
            } => {
                assert_eq!(*reducer, handler);
                assert_eq!(m.id, Hash::of(b"http.get"));
                assert_eq!(m.from, leaf());
                assert_eq!(m.payload, Bytes::from_static(b"input"));
            }
            other => panic!("expected RunReducer(Message) to the handler, got {other:?}"),
        }
        assert!(commands.contains(&Command::Monitor { reducer: handler }));
        // No deadline was set, so no timer is armed, and the dispatch stays open (not retired).
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, Command::ArmTimer { .. }))
        );
        assert!(!commands.contains(&Command::RetireContext));
    }

    #[tokio::test]
    async fn answers_missing_handler_when_no_handler_is_registered() {
        let mut ev = DirectDispatch::new(HashMap::new());
        let commands = ev.on_dispatch_start(effect(b"nobody.answers", None)).await;
        // The leaf gets a MissingHandler response folded straight back, and the dispatch retires.
        match &commands[0] {
            Command::RunReducer {
                reducer,
                event: Delivered::Response(r),
            } => {
                assert_eq!(*reducer, leaf().reducer);
                assert_eq!(r.continuation_token, Bytes::from_static(b"leaf-token"));
                assert_eq!(r.payload, Err(Error::MissingHandler));
            }
            other => panic!("expected a MissingHandler response to the leaf, got {other:?}"),
        }
        assert_eq!(commands.last(), Some(&Command::RetireContext));
    }

    #[tokio::test]
    async fn arms_a_deadline_and_enforces_it_as_a_timeout() {
        let handler = Hash::of(b"slow.edge");
        let mut ev = DirectDispatch::new(HashMap::from([(Hash::of(b"slow.op"), handler)]));
        let start = ev
            .on_dispatch_start(effect(b"slow.op", Some(Duration::from_secs(30))))
            .await;
        // A deadline was set, so a timer is armed alongside routing to the handler.
        assert!(start.iter().any(
            |c| matches!(c, Command::ArmTimer { after, .. } if *after == Duration::from_secs(30))
        ));
        // When that timer fires, the leaf receives Timeout and the dispatch retires.
        let fired = ev.on_timer_fired(Bytes::from_static(b"deadline")).await;
        match &fired[0] {
            Command::RunReducer {
                reducer,
                event: Delivered::Response(r),
            } => {
                assert_eq!(*reducer, leaf().reducer);
                assert_eq!(r.payload, Err(Error::Timeout));
            }
            other => panic!("expected a Timeout response to the leaf, got {other:?}"),
        }
        assert_eq!(fired.last(), Some(&Command::RetireContext));
        // An unrelated timer token does nothing.
        assert!(
            ev.on_timer_fired(Bytes::from_static(b"other"))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_dead_handler_retires_the_context() {
        let handler = Hash::of(b"crashy");
        let mut ev = DirectDispatch::new(HashMap::from([(Hash::of(b"op"), handler)]));
        ev.on_dispatch_start(effect(b"op", None)).await;
        // The monitor fires for the handler that died mid-obligation; the reference retires the context.
        let commands = ev.on_monitor_fired(handler).await;
        assert_eq!(commands, vec![Command::RetireContext]);
    }
}
