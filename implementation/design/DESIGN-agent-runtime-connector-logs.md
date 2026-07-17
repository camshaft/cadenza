# Connector model — per-connector kernel-written logs + Cedar principals (operator rulings, 2026-07-17)

**Owner:** v-agent-harness. **Amends:** `DESIGN-agent-runtime-client-and-adapters.md` (resolves the 2 connector
sub-forks it routed). Records the operator's rulings on the connector↔daemon notify mechanism and the
on-behalf-of identity model. Folds into the minimal-kernel design.

## Fork 1 — RULED: per-connector kernel-written logs, NOT connector-tails-the-main-log

My earlier lean was "the connector subscribes to / tails the main log for events addressed to it." The operator
**corrected this** — verbatim:

> "My worry with connectors tailing the log is then they're duplicating the whole thing. I think what would be
> better is if each connector somehow had its own log that the main kernel wrote to. And then the connector
> could just subscribe to a list of things it needed to do, rather than folding the log on the main host."

**The model:**
- **Each connector has its OWN log** — a small, scoped, per-connector feed.
- **The main kernel is the WRITER** into each connector's log: it decides what that connector needs to do and
  writes those work-items into that connector's log.
- **The connector subscribes only to its own log** — a scoped work-list. It does NOT tail/fold the global
  shared log (which would make every connector duplicate + re-fold the entire event history — wasteful, wrong
  scoping).

**The symmetry (two directions, two logs):**
- **INBOUND** (user → system): the connector posts the user's event INTO the **main log**, on-behalf-of the user.
- **OUTBOUND** (system → user): the connector reads its work-items FROM its **own** kernel-written log.

So: `main log (kernel's) → kernel projects per-connector work-items into PER-CONNECTOR LOGS → each connector
consumes its own small feed`. The per-connector log is a **scoped projection the kernel maintains**, not the
connector folding the global log.

## Fork 2 — RULED: Cedar PRINCIPALS for on-behalf-of + capabilities

> "yeah, we should be using cedar principals for this kind of thing, I think."

The identity/authority model is keyed on **Cedar principals** throughout:
- When a connector posts on behalf of user U, **U is a Cedar principal carried on the event**.
- The interpret program evaluates the Cedar policy for that principal (the attenuation ruling — Cedar docs in
  the log de-escalate to the minimal capability set for that principal).
- Reuse the Cedar principal model everywhere: the connector's on-behalf-of, the capability de-escalation, the
  publish→anyone-can-call flow — all keyed on Cedar principals (leans on the L0 Cedar on-behalf-of work).

## How this composes with the whole design

- The **per-connector log is still just a `Log`** (the L1 abstraction — an ordered `{seq, kind, payload}`
  stream). No new kernel concept: it's another log the kernel `log_append`s into and the connector `log_read`s.
  So the broad-primitive surface (`log_append`/`log_read`) already covers it — the kernel writes a connector's
  work-items via `log_append` into that connector's log; the connector reads via `log_read`. No new primitive.
- **The kernel decides what to project** into a connector's log — that decision is the Cadenza `interpret`
  program's job (event-agnostic kernel: it just `log_append`s where interpret says). So "what goes in the Slack
  connector's log" is Cadenza policy, not kernel code.
- **On-behalf-of = a Cedar principal attribute on the inbound event.** interpret reads it; Cedar attenuates.

## Sub-fork routed to the operator

**How does the kernel's write into a connector's log relate to the main log — a SEPARATE stream, or a FILTERED
VIEW of the main log?** Two readings:
- (a) **Separate stream:** the connector's log is a distinct `Log` instance (own seq space); the kernel
  duplicates the relevant work-items into it. Simple, fully decoupled, but the item exists twice (once in main,
  once in the connector log).
- (b) **Filtered view / index:** the connector's log is a cursor/index INTO the main log (the kernel maintains a
  per-connector filter; the connector reads main-log events matching its filter, but via its own scoped cursor
  so it never folds the whole thing). No duplication, but the connector still reads from the main log's storage.
- Leaning (a) for clean decoupling (a connector is a separate deployable — its own log is operationally simpler
  + independently scalable), but (b) avoids duplication. Operator to rule.

## Consequence for the build plan

No change to K0 (done) / K1 (kernel skeleton, pending the rcdzc-dep ruling). Refines the later **KA (Slack
adapter)** rung: KA = a connector with (inbound) post-to-main-log-on-behalf-of-Cedar-principal + (outbound)
read-its-own-kernel-written-log. The per-connector-log projection is a Cadenza-interpret concern (what to write
where), so it needs no new kernel primitive — proceed on K1 first; KA builds on the running kernel.
