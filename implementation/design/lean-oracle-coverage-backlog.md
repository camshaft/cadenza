# Lean semantics-oracle — language-support coverage backlog

**Owner:** `v-lean-oracle`. **Requested by:** operator (2026-09-01, via concierge): *"For the lean oracle it
should do a pass at improving language support. Look at the programs that are not handled by the oracle and
get a backlog of language features that it can add."*

**Method.** Surveyed every coverage gap the oracle emits — `symEval`'s `cannotProve "symeval: …"` reasons
(`Oracle/Symbolic.lean`), `denote`'s `.unsupported` arms (`Oracle/SymbolicSound.lean`), and the vacuous
cases of the T2 capstone (`denote_normalize_sound`). A gap is where the oracle returns `cannotProve` /
`Unsupported` (sound: it declines, never a false verdict) instead of a `proven` / value. Widening any of
these turns fuzzer/corpus programs from `skip` into real differential signal. Priority = (programs unlocked)
× (soundness reach) ÷ effort.

## Two kinds of gap
- **INHERENT / degrade-to-sampled** — symbolic (non-constant) operands to collection/string ops, unbounded
  data. The T2 design deliberately routes these to v-cdz-smith's SAMPLED differential; NOT a backlog target
  (listed under §C for completeness, marked *sampled*).
- **MODELABLE / addable** — a construct the oracle *could* fold/denote but doesn't yet. These are the backlog.

---

## §A — HIGH VALUE (unlock whole program classes)

### A1. Bounded recursion / library-function calls  ⭐ highest impact
- **Gap:** `symeval: … recursion …` / call-fuel exhaustion → `cannotProve` (7 sites). Any program calling a
  recursive helper (or a deep non-recursive chain past the fuel bound) declines.
- **Feature:** prove bounded recursion instead of fuel-punting. Borrow talos's **WP-over-fueled-interp**
  (`∃N,∀fuel≥N` absorbs fuel induction) — the L1 lesson already distilled in
  [[talos-wp-lessons-for-cadenza-oracle]]. Model structural recursion on finite data (fold over a concrete
  list/bounded range) as a terminating symbolic unfold.
- **Value:** unlocks the single largest declined class (anything using `List.fold`-style helpers, recursive
  sum walks). **Effort:** LARGE. Sequence: concrete-bounded first (unroll over a const collection), then
  symbolic-bounded.

### A2. `try` / `?` error short-circuit (the deferred try-cluster)
- **Gap:** `symeval: try on a failing ctor (errReturn short-circuit not modeled)`, `try operand not a
  concrete Ok/Some …`. The `.errReturn` short-circuit isn't threaded through symEval's ~54 match sites.
- **Feature:** add an `.errReturn` symbolic outcome and thread it (a `try (Err e)` / `?` on a failing value
  short-circuits the enclosing fn to that error). v-cdz-smith has flagged ~3–12 real cases waiting on this.
- **Value:** MEDIUM (error-handling programs). **Effort:** MEDIUM-risky (touches every symEval match arm) —
  a dedicated careful arc.

### A3. Compound OBSERVATION in `denote` (proj / case / ctor)  — also extends the T2 capstone
- **Gap:** `denote` MODELS tuple/record CONSTRUCTION but not OBSERVATION: `.proj` (read a tuple/record
  field), `.case` (match a value), and user `.ctor` all → `.unsupported` (vacuous in the capstone). symEval
  models these for concrete values, but the SEMANTICS model (`denote`) does not, so they can't be capstone-
  covered or value-form-checked.
- **Feature:** extend `denote` to model `.ctor` (a sum value), `.proj` (positional/field read, trap-on-poison
  when observed — the "trap-when-observed" spec rule), `.case` (first-match). Then extend the capstone's
  `WellDenoted`/case lemmas to cover them (they're currently vacuous).
- **Value:** HIGH (compound-valued programs get a semantics verdict + capstone coverage). **Effort:** MEDIUM.

---

## §B — MEDIUM VALUE (concrete, bounded, low effort — the v-cdz-smith drain cadence)

### B1. `Set.union` / `Set.remove`  (v-cdz-smith #7371 boundary, ~8 cases)
- **Gap:** `Set` transform ops over concrete sets aren't folded (Set.of/len/insert/contains ARE). Set.union →
  dedup-merge; Set.remove → minus. Fuzzer-confirmed value-correct, oracle just doesn't fold.
- **Value:** direct fuzzer-boundary drain. **Effort:** SMALL (mirror the modeled Set.insert). **Next up.**

### B2. `Option.expect` on `None`, and richer trap-message modeling
- **Gap:** `Option.expect on None (trap-message not modeled)` → cannotProve. The `None` case traps with a
  custom message the oracle doesn't model.
- **Feature:** model the trap (kind + message) for `Option.expect None` and similar custom-trap builtins.
- **Value:** SMALL-MEDIUM. **Effort:** SMALL.

### B3. Nullary / builtin ctor completeness sweep
- **Gap:** assorted `newtype ctor missing payload`, `user-ctor argument is unmodelable`, member-op-head-not-
  modeled tail. Mostly drained; a periodic sweep as v-cdz-smith widens surfaces the next family (the proven
  two-wave present→absent drain: Qty, Map.lookup, Set-transform, …).
- **Value:** incremental. **Effort:** SMALL each. **Cadence:** ride v-cdz-smith A/B loops.

---

## §C — INHERENT (degrade-to-sampled; NOT backlog, listed for completeness)
- Collection/string ops over **symbolic** (non-const) operands: `… on a non-<T> value` / `… needs
  all-concrete …` for List/Set/Map/String/Bytes — the operand is a param/unknown, so no concrete fold. The
  T2 design routes these to v-cdz-smith's sampled differential.
- Unbounded / symbolically-indexed collections and strings.
- Genuinely malformed AST (`malformed …`, `node index out of range`) — correct declines, not features.

---

## Recommended sequencing (interleaved with the type-oracle T0→T2 build)
1. **B1 Set.union/remove** — small, ready, drains a live fuzzer boundary. (quick win)
2. **A3 compound observation in `denote`** — extends both coverage AND the semantics capstone.
3. **A2 try/errReturn** — the deferred cluster (dedicated arc).
4. **A1 bounded recursion** — the big one (WP/fuel-abstraction); largest unlock, largest effort.

§B rides the existing v-cdz-smith A/B cadence; §A items are standalone arcs. This backlog is relayed to the
operator; I own the interleave between it and the type-oracle increments.
