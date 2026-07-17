# L2 implementation plan — messaging + inbox as a fold (agent-runtime ladder rung 2)

**Owner:** v-agent-harness (implementation). **Charter:** `DESIGN-agent-runtime-vision.md`. **Builds on:**
L1 (the `cdz-kernel` fold owner over a `Log` — file or DynamoDB — proven replay-deterministic). This plans
**L2**, vision §15 rung 2: *re-express one fleet interaction (merge-request/reject) as MESSAGE/ACK events
with the inbox as a projection.* The first **fleet-convergence** dogfood.

> Status: PLAN (this doc). No L2 code yet. Written after L1a–d shipped; hand-off-safe if the operator mints
> a separate agent-runtime vertical.

## The thesis L2 proves (vision §9)

- A **message is a typed, durable, addressed event**: `MESSAGE{from, to, kind, subject, refs[], body}`.
  `kind` is the fleet's earned vocabulary (merge-request / reject / assign / ask / answer / note / …).
- The **inbox is a PROJECTION, not a queue**: "my unread" is a fold over `MESSAGE` events addressed to me
  minus those a later `ACK` event marks processed. This is exactly the shape the current file-inbox fakes
  (JSON in a hub dir + `processed/` moves) — L2 shows it as a fold over the L1 log.
- **Reply-then-ack is crash-safe for free**: the reply `MESSAGE` is appended *before* the `ACK`; a crash
  between leaves the reply landed + the source un-acked (re-driven). The fleet's hard-won rule, native.

## What L2 REUSES from L1

The L1 `Log` trait (`append(kind, payload) -> seq`, `tail(from) -> events`) + `Event{seq, kind, payload}`
are exactly the substrate: a `MESSAGE`/`ACK` is an `Event` whose `kind` is `"message"`/`"ack"` and whose
`payload` is the encoded message. No new storage — L2 is a projection layer over the L1 log.

## L2 decomposed into gated sub-rungs (one MR each, sequential)

- **L2a — the message type + its encoding.** A `Message{from, to, kind, subject, refs, body}` struct + a
  pure encode/decode to/from the `Event` payload bytes (like dynamo_log's marshalling: pure, unit-tested,
  no network). `kind` stays the fleet vocabulary strings. An `Ack{message_seq}` likewise. Deliverable: a
  `msg` module with round-trip tests (incl. the fleet's multi-field/`refs[]` shapes + binary-safe body).
- **L2b — the inbox projection (the fold).** `inbox_for(log, agent) -> Vec<Message>`: fold the log's
  `message` events addressed to `agent`, minus those an `ack` event (by source seq) marks processed. Pure
  over a `&[Event]` (testable with a hand-built log). This is the "inbox = fold" proof: append messages +
  acks, fold, assert the unacked set. Also `is_acked(log, seq)`.
- **L2c — reply-then-ack, crash-safe.** A helper that, given a source message, appends the REPLY message
  then the ACK (in that order) to the `Log`. Test the ordering invariant: after reply-but-before-ack the
  source is still unacked (re-drivable); after both, acked + the reply present. Proves the fleet's durable
  rule natively over the log.
- **L2d — the merge-request/reject round-trip (the fleet-convergence dogfood).** Wire one concrete fleet
  interaction end-to-end over the log: append a `merge-request` MESSAGE → fold an inbox showing it →
  append a `reject`/`merged` REPLY + ACK → fold showing it processed. The smallest slice that re-expresses
  a REAL fleet exchange as log events + projections, demonstrating §9's "deletes the file-inbox machinery."

## Crate shape

All in `cdz-kernel` (the microkernel) — a new `msg` module (L2a) + inbox-projection functions (L2b/c) +
an integration test (L2d). No new crate, no new heavy deps (pure over the existing `Log`). The DynamoDB
backend (L1d) already makes this work against a real log behind the `aws` feature; CI stays on the file log.

## Gate (per rung)

`cargo test` in cdz-kernel (pure projections + the file log — no network/creds, CI-safe), clippy -D + fmt,
`cargo build --features aws`. Each rung's fold/projection is the invariant it pins.

## Open (don't block L2)

The full typed `kind` enum vs open strings (start with strings, matching the fleet + L1's Event.kind); the
subscription-driven wake (L3, "an addressed message is a scheduling event"); multi-agent addressing beyond
one `to` (fan-out) — all later.

## First action next tick

Start **L2a**: the `Message`/`Ack` types + pure encode/decode in a `msg` module + round-trip tests. Gate + MR.
