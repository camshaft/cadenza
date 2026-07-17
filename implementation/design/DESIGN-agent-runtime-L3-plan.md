# L3 implementation plan — subscriptions: the one reactive primitive (agent-runtime ladder rung 3)

**Owner:** v-agent-harness (implementation). **Charter:** `DESIGN-agent-runtime-vision.md` §8. **Builds on:**
L1 (the `Log`/`Event` + fold owner) and L2 (messaging: `Message`/`Ack`, `inbox_for`). This plans **L3**,
vision §15 rung 3: *the `SUBSCRIBE` event + fold-time dispatch — "an addressed message is a scheduling
event"; re-cast the agent loop + one reporter as subscriptions, unifying scheduling.*

> Status: PLAN (this doc). No L3 code yet. Written after L1 + L2 shipped; hand-off-safe.

## The thesis L3 proves (vision §8)

A **subscription** = `{predicate over the event stream, handler program, capability}`, and it is itself a
durable log event (`SUBSCRIBE{predicate, program-ref, capability}`). **The owner's fold dispatches them**:
each event that lands is matched against the active subscriptions' predicates, and matches schedule their
handler programs — no separate daemon/poller, it rides the fold that already exists. This is *the*
unification: the agent loop ("wake me on messages addressed to me"), reporters, auto-compaction, and the
compute router are all subscriptions. L3 builds the mechanism; the cascade (handlers invoking programs +
appending events) grows on top.

## What L3 REUSES

L1's `Log`/`Event` (a `SUBSCRIBE` is an `Event` of kind `"subscribe"`); L2's messaging + `inbox_for`
(the canonical subscription is "messages addressed to me" — L2's `to == agent` filter is a predicate).
The pure-codec + fold-projection discipline from L2a/L2b is exactly the shape L3 follows.

## L3 decomposed into gated sub-rungs (one MR each, sequential)

- **L3a — the Subscription type + a predicate + codec.** A `Subscription{id, predicate, program_ref,
  capability}` where `predicate` is, for the first cut, a concrete matchable value (NOT a general
  expression language — that's deferred, vision "open leaf-level"): e.g. `Predicate::MessageTo(agent)` /
  `Predicate::EventKind(kind)`. Pure encode/decode to the `subscribe` event payload + a `matches(pred,
  event) -> bool` (pure). Round-trip + match/no-match tests (no store/network), mirroring L2a.
- **L3b — active-subscriptions projection (a fold).** `active_subscriptions(events) -> Vec<(Seq,
  Subscription)>` = fold `subscribe` events, minus any revoked-by-supersession (an `UNSUBSCRIBE`/superseding
  event by id). Pure over `&[Event]`, hand-built-log tests — mirrors L2b's `inbox_for`.
- **L3c — fold-time dispatch.** `dispatch(events, new_event) -> Vec<(Seq, Subscription)>` = the active
  subscriptions whose predicate `matches(new_event)`. This is the core "event lands → which predicates
  match → schedule those" step, pure + tested. (Actually RUNNING a matched handler program ties to the
  fold owner + capabilities — L4/L5; L3c returns the matches, the schedulable set.)
- **L3d — recast the agent loop as a subscription (the unification dogfood).** Show that L2's inbox is a
  subscription: a `Predicate::MessageTo(me)` subscription's `dispatch` matches exactly the messages
  `inbox_for(me)` surfaces — proving the agent loop IS "wake me on messages addressed to me" (§8). An
  integration test tying L2 + L3 together, the "subscriptions unify scheduling" proof.

## Crate shape

A new `sub` module in `cdz-kernel` (Subscription/Predicate + codec + the projection + dispatch), + an
integration test for L3d. Pure over the L1 `Log` + L2 messaging; no new crate/heavy deps. Same excluded-
crate CI job (`cargo test` + clippy + fmt + `cargo build --features aws`).

## Deliberately deferred (vision "open leaf-level"; don't block L3)

A general predicate EXPRESSION language (semantic match, boolean combinators) — L3 uses concrete
`Predicate` variants; the expression language is a later rung. Actually EXECUTING handler programs under
their capability (L4 capability=effect-type + the fold owner driving them). Subscription revocation
semantics beyond simple supersession-by-id.

## First action next tick

Start **L3a**: the `Subscription`/`Predicate` types + `matches` + pure codec in a `sub` module + round-trip
and match tests. Gate + MR.
