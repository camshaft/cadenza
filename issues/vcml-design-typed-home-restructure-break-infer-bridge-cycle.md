# Design/scoping: extract `Typed` to a leaf module to break the infer-db↔ty-bridge cycle

**Scoped:** 2026-08-01, v-compiler-ml. A ready-to-execute plan for cleanup option (B) (the Typed-home
restructure) — held pending concierge/operator "scope (B) or hold" answer. Executing it unlocks the
`typed-to-ty`/`typed-list-to-ty` dedup and cuts the ty/infer coupling. NOT behavior-neutral-trivial (a
type-move shifts import resolution), so it wants the go before landing.

## The problem (verified on trunk a42c3f91a)
`type Typed` is defined in `infer-db.cdz:30` (variants: `TIntW(Bool,Int64) | TBool | TErr | TFn(List(Typed),
Typed) | TSum(Int64)` — fully self-contained, references only Bool/Int64/List/Typed). Six modules import
`Typed` from infer-db: db-state, db-demand, ty-bridge, db-infer, lower-db, db. Because `ty-bridge` imports
`Typed` FROM infer-db, infer-db CANNOT import ty-bridge back (cycle) — so infer-db carries a DELIBERATE local
copy of `typed-to-ty` / `typed-list-to-ty` (its comment: "infer-db can't import ty-bridge, cyclic"). Same
shape forces the `ty-eq` situation.

## The fix (clean, moderate-risk)
1. **Create `src/typed.cdz`** — a LEAF module (zero imports) holding just `type Typed = …` + the trivial
   `t-int64()`/`t-int-deferred()` constructors + `is-deferred-int` IF they're pure `Typed` helpers (audit:
   move only what has no other dependency; keep width-logic in infer-db). Export `Typed.*` + the movers.
2. **Repoint the 6 importers** (db-state/db-demand/ty-bridge/db-infer/lower-db/db) + infer-db itself to
   `import { Typed } from "typed"`.
3. **infer-db now imports ty-bridge** (`typed-to-ty`/`typed-list-to-ty`) instead of its local copies; DELETE
   the infer-db duplicates. (Verify no OTHER cycle: ty-bridge imports ty + Typed — both leaf now.)
4. Same for `ty-eq` if the apply-ty/ty.cdz dupe is unblocked by the break (audit at execute time).

## Risk + verification
Moderate: a type-move touches ~8 files + shifts import resolution. NOT behavior-neutral by inspection (import
graph changes), so verify: all modules `cdz check` clean, the affected suites (infer-db 67/0, lower-db 19/0,
db-* , ty-bridge) green on the SELF-HOST (per the rcdzc-emit-verify-on-self-host discipline), ML round-trip
clean. Net LOC: small (moves the type + deletes ~2 dupe fns) — the value is DECOUPLING + one-source-of-truth
for the Typed↔Ty bridge, not LOC. A legibility win (the cycle-forced-copy confusion goes away).

## Sequencing vs conf-db
Independent of the conf-db retire. Either can go first. This one is behavior-affecting-but-not-gate-behavior
(no gate-enforcement change), so it likely needs less operator ceremony than conf-db (which flips gate
enforcement) — could be a good "next non-conf-db cleanup" if concierge greenlights (B).
