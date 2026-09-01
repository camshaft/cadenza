## 75. 🧭 (cdzc design, NOT a seed gap) Inference plan: real Hindley-Milner as a SEPARATE `Hir → typed-Hir` pass — learning from the seed's coarse-kind mistakes

**What.** The design for how the rewritten compiler `cdzc` will do type inference, written now (while the
front end is blocked on ask-73) because inference is the rewrite's single biggest upcoming subsystem and the
seed has given us a complete catalog of what NOT to do. This is a design/planning ask (like ask-67), not a
seed gap — nothing for the compiler agent to fix; it records the target so the `Hir → Mir` work is built to
it. HM is already the ratified direction (2026-07-04, operator "same strategy as OCaml"); `type-system.md:30-42`
mandates its PROPERTIES normatively.

### The mistake to learn from — what the seed's "inference" actually is

The seed's inference is a **coarse wasm-valtype classifier wearing an HM costume**, not type inference:
- Lattice = one closed enum `Int64 | Bool | Float64 | Unit | Never | Heap` (codegen.rs:96). Every compound
  (String/List/Record/Sum/Tuple) collapses to one opaque `Heap` (an i32 pointer). It answers exactly "which
  valtype does this result occupy + is it on the heap."
- `unify` (codegen.rs:159) merges `Never` with anything, else demands EQUALITY — no type variables, no
  substitution, no occurs-check; just `Option<Kind>` slots filled **first-write-wins**. The code calls itself
  "Algorithm W" but its own comment admits "Kind is the current monomorphic ground lattice; full HM adds
  first-class type variables … LATER."
- A SEPARATE finer `Shape` classifier (codegen.rs:10689) carries recursive structure for the renderer, and
  **many bugs live in the gap between "I know it's Heap" and "I know its Shape."**
- It is **re-derived ad-hoc during emission** ("kinds have ONE source of truth — emit", codegen.rs:603), so
  a fixpoint pass for signatures and emit-time re-derivation for bodies must AGREE — and when they don't you
  get "branches differ in kind" / invalid components.

### Why it keeps failing — every inference ask is ONE bug

Order-dependent unification of a PLACEHOLDER kind against a CONCRETE kind, where the placeholder is a
not-yet-solved recursive self-call or threaded accumulator. Same bug on different lattice points:
- ask-14 (Bool return, branch-ORDER — self-call in `then` locks non-Bool), Tier-00 (Heap accumulator inferred
  scalar), ask-18 (List accumulator loses list kind), ask-24 (fixpoint never reaches a fixed KIND → re-expands
  → OOM), ask-34 (`(id true)` → the integer `1`, a miscompile — unconstrained result defaults i64, no
  arg-shaped return specialization), ask-65 (`Func` has `ret_kind` but no `ret_shape` → shape lost across a
  return), ask-73 (tail-recursive TUPLE return = "unknown tuple shape"; narrowed: the RECORD path already
  works, only tuples don't).
- The seed's OWN admission (ask-14): *"kind-inference order-independence is a property EVERY result kind
  needs; the fix belongs at GENERAL RESULT-UNIFICATION, not a per-kind patch."* They never did that — they
  patched per-kind (a tie-break table Heap>scalar>Int64-default>Never; a Heap-upgrade in `constrain`; a
  reverse arg→param sweep; `expect_name_only` to dodge a 4^depth re-walk). The machinery is riddled with
  exponential-cost landmines from the emit-time re-derivation.

### The plan for cdzc — do the general fix the seed admitted it needed

1. **Real HM, not a kind lattice.** A `Type` sum with actual **type variables** (`TVar`), `unify` with
   **substitution + occurs-check**, principal types. Infer STRUCTURE (int/bool/float/tuple/record/sum/fn/row),
   not "which valtype." The wasm valtype is a TRIVIAL READ-OFF of the solved type at lower time — never the
   thing inferred. This dissolves the Kind-vs-Shape gap (ask-65/73) by construction: one solved type, and its
   shape IS its structure.
2. **Inference is a SEPARATE `Hir → typed-Hir` pass, BEFORE lowering** (spec compiler-pipeline.md:40-42:
   emission MUST NOT decide a type). Annotate every Hir node with its solved `Type`; lowering reads it.
   Kills the "two mechanisms must agree" failure and the exponential emit-time re-derivation.
3. **Order-independence for free.** Unification with type variables is inherently order-independent: a
   recursive self-call gets a fresh `TVar` that unifies with its concrete sibling regardless of branch/arm
   order — no tie-break tables, no first-write-wins. Self-recursion is standard HM (give the fn a `TVar`
   signature, infer the body against it, unify). This retires the whole ask-14/18/34/73 class at once.
4. **Monomorphization = compile-time reduction, the SAME tier as const-fold** (spec type-system.md:248-250).
   Polymorphism via let-generalization; a poly fn used at Int64 and Bool specializes by the same reduction
   that inlines `(f Int64)`. Kills the i64/i32 poly boundary (ask-34/35/59) — no per-parameter kind hack.
5. **Bidirectional boundary at type-valued-params + annotations** (spec type-system.md:54-58): HM ranges over
   a NON-computational term core; first-class computable types are CHECKED (synthesized by monomorphization /
   checked against annotation) at those sites, not unified. The bidirectional boundary = the
   monomorphization boundary — one boundary, two names.
6. **Rows reuse the unifier** (open records + effect rows) — type-system.md:84-92, 146-148. One unifier for
   scalars, records, effects.
7. **Diagnostics:** on unify failure report the MINIMAL conflict at BOTH sites (type-system.md:62-64) — not
   the first constraint that failed.

### Ladder placement

The `Hir → Mir` step IS "infer + monomorphize + lower": inference is the first Hir→Hir (annotate each node
with its solved `Type`); monomorphization is the compile-time evaluator (one tier — same mechanism as
folding); then `Mir` lowers with all types solved and valtypes read off. `Lir`/`serialize` unchanged.

### Honest scope

This is the biggest single piece of the rewrite (a `Type` sum, a unifier with a substitution map,
generalize/instantiate, the bidirectional split, row unification). But it REPLACES the seed's entire per-kind
patch pile + the emit-time re-derivation, and it is what makes generic Option / records / effects fall out
uniformly instead of each needing a bespoke shape hack. It is NOT started (front end is at Phase 0, blocked
on ask-73); this ask fixes the target so the work is built right, not rediscovered per-kind.

Related: [[inference-hindley-milner]], the bidirectional-boundary learning (2026-07-04), the seed failure
catalog (ask-14/18/24/34/65/73), [[inference-plan-learn-from-seed-coarse-kind-mistakes]].
