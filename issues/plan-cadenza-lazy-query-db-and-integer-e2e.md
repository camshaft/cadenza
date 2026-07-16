# Plan: lazy query-DB foundation + integer programs end-to-end (v-compiler-ml re-charter)

**Mandate (operator, live):** build the lazy DATABASE/QUERY-ENGINE foundation in Cadenza (mirror rcdzc's
memoized demand-driven `Db` spine), THEN own basic INTEGER programs running end-to-end (thin but complete
parse→run). Mirror the Rust bootstrap.

## What rcdzc's spine actually is (studied `implementation/seed/crates/rcdzc/src/db.rs`)

ONE `Db` = **columns over one node identity (`StructId`)**, pure data. Each *kind of derived fact* is a
column (resolved form, solved type, core form) keyed by the node's id. Contract:
- Each column is filled by exactly ONE query module: `resolve` fills `resolved`, `infer` fills `types`,
  `lower` fills `core`.
- A module reads another's fact by calling its PRODUCER (`Infer.type_of(db, id)` → calls
  `Resolve.resolved_of`), never the raw column — which is what makes it lazy (the producer fills on demand).
- **Backward-demand memoization:** a producer reads its column; on a MISS it computes (reading upstream via
  their producers) and fills its column. Asking one node's fact touches only the nodes that answer reaches.
- **No separate cache to invalidate** — the columns ARE the memo; incrementality = re-run, not invalidate.
- **Absence ≠ value:** a slot is filled or not; a reader that needs a value and finds absence DECLINES.
  Every negative decision (decline/reject/poison) is itself a FILLED value (`Resolved::Poison`), distinct
  from "not yet determined".

## The Cadenza mirror — query-DB structure

The one real design question is state: rcdzc uses `&mut Db` with in-place column fills; Cadenza is
functional. Two viable shapes (I'll pick empirically at increment 0, defaulting to A unless it fights):

- **(A) State-effect memo (preferred — mirrors the mutable Db closest).** A `Db` effect exposing
  `get-col(col, id) -> Option Fact` and `put-col(col, id, fact) -> Unit` over a threaded store
  (`Map (col, NodeId) Fact`), handled once at the top (the `Fresh`-effect pattern generalized to a
  key→value table). A producer = `def type-of(id) = match Db.get-col(TYPES, id) with Some(t) => t |
  None => let t = compute… in (Db.put-col(TYPES, id, t); t)`. Backward-demand + memo, exactly rcdzc's
  shape, with the store threaded by the handler instead of `&mut`.
  RISK: the effectful-producer-with-two-sibling-recursive-calls shape (a node with 2 children) — v-effects
  FIXED that (multi-value-return threading; label.cdz proves it), so this is now viable.
- **(B) Threaded-Db record (pure fallback).** `Db` is a record of `Map NodeId Fact` columns, threaded
  `(Db, Fact)` through every producer. No effects; more plumbing, but zero effect-system risk. Fallback if
  (A) hits a wall.

NODE IDENTITY: `NodeId = Int64` (mirrors `StructId`). The parser assigns each AST node a stable id (an
interner/counter pass — the `max-id+1` / interning spikes I already de-risked). Columns keyed by `NodeId`.

FACTS (integer subset first): `Resolved` (name→binding), `Ty` (just `TInt` at first), `Core` (lowered
integer IR). Poison variants for negative decisions.

## Increment ladder to first-integer-program-end-to-end (narrow, then widen — the rcdzc bootstrap)

- **Inc 0 — the Db substrate.** `db.cdz`: the memo store + `get-col`/`put-col` + the "producer reads its
  column, miss→compute→fill" combinator, with a trivial column (e.g. `node-count` memoized) to prove
  backward-demand + memo + a cache HIT is O(1). Decide (A) vs (B) here. Pin: asking a fact twice computes
  once.
- **Inc 1 — parse column.** Source `String` → AST with stable `NodeId`s, as the first real column
  (`ast_of(file)`). Reuse the existing lex/parse/strlex modules; add node-id assignment.
- **Inc 2 — resolve column.** `resolved_of(id)` fills name→binding for a let/var integer program; a use of
  an unbound name → `Poison` (a filled negative decision, not absence). Producer-reads-producer: resolve
  reads parse.
- **Inc 3 — infer column (integer-only).** `type_of(id)` = `TInt` for the integer language (literal, +,
  -, *, let, var); reads resolve. (The HM `infer`/`infer-let` modules already exist — adapt onto the DB.)
- **Inc 4 — lower column.** `core_of(id)` → a small integer Core IR; reads infer. (constprop/anf/etc.
  already exist as transforms to draw from.)
- **Inc 5 — emit + run (end-to-end).** Core → the existing stack-machine `codegen`/`exec`, OR direct eval.
  MILESTONE: `let x = 2 in x * 3 + 1` goes source → DB-driven passes → **runs → 7**, via the lazy query DB.
  Then widen (if/fn, more types) incrementally.

Each increment is one gated MR (its own `@test`s; the DB makes each column independently testable).
COORDINATION: mirror rcdzc (v-inference for the infer parallel, v-runtime for eval), but the Cadenza-native
architecture is mine to drive.

## Open question for the operator/concierge

Confirm the state shape isn't over-constrained: is the **State-effect memo (A)** acceptable as the DB
substrate (it's the closest mirror of `&mut Db`), or is a **pure threaded-Db (B)** preferred for
determinism/portability? I'll default to (A) and fall back to (B) at Inc 0 if the effect shape fights, but
flagging since it's the load-bearing choice.
