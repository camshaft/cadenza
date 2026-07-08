# A coarse kind-classifier re-derived at emission is not inference — and it fails the same way at every lattice point

*2026-07-08*

**What happened.** Over ~8 self-hosting cycles, a cluster of independent-looking compiler bugs kept
appearing, each filed as its own gap: a recursive `Bool`-returning function declined depending on which
`if` branch held the self-call (ask-14); a `List.push` accumulator "lost its list return kind" (ask-18); a
monotone fixpoint OOMed when a re-seeded list parameter was consumed as a list (ask-24); a polymorphic
identity applied to a `Bool` *returned the integer 1* — a wrong-value miscompile (ask-34); a payload's shape
was lost across a bare function return (ask-65); a tail-recursive **tuple**-returning function declined
"runtime sum match without a constructor arm" while the *identical shape as a record compiled* (ask-73). A
deliberate mapping of the seed's (`cdz-rustc`'s) inference machinery, cross-read against every one of these
asks, showed they are not separate bugs — **they are one bug observed at different points of a coarse
lattice.** The seed even admitted the general shape of the fix in ask-14 ("kind-inference order-independence
is a property EVERY result kind needs; the fix belongs at general result-unification, not a per-kind patch")
and then did not build it — it patched each lattice point separately (a branch-kind tie-break table, a
Heap-beats-scalar upgrade in `constrain`, a reverse argument→parameter sweep, a `bare-name-only` re-read to
dodge an exponential re-walk). This confirmed, the expensive empirical way, the prediction the
Hindley-Milner decision already made four days earlier
([[2026-07-04-inference-is-hindley-milner]]): "the coarse seed-Int64/refine-return-kind-to-a-fixpoint scheme
is a stopgap … it must be replaced by a real inference pass."

**Why.** Three compounding design choices, none of which is inference:

1. **The lattice is a wasm-valtype classifier, not a type.** The seed's `Kind` is a closed enum —
   `Int64 | Bool | Float64 | Unit | Never | Heap` — and every compound (String, List, Record, Sum, Tuple)
   collapses to one opaque `Heap` (an i32 pointer). It answers only "which valtype does this result occupy?"
   A separate `Shape` classifier carries recursive structure for the renderer, so *the compiler holds two
   disagreeing notions of a value's type*, and a whole family of bugs (ask-65, ask-73) lives precisely in the
   gap between "I know it's `Heap`" and "I know its `Shape`." A value's structure is never inferred; only its
   register class is.

2. **It is re-derived ad-hoc during emission.** "Kinds have one source of truth — `emit`": a function's
   return kind is *whatever emitting its body happens to yield*. So there are two mechanisms — a fixpoint
   pass that computes signatures, and emit-time re-derivation that computes bodies — and correctness requires
   them to agree. When they disagree the result is "branches differ in kind" or an invalid component. Because
   the derivation is fused into emission and inlines callees per-call to recover shape, it also carries
   exponential-cost landmines (2ⁿ env copies, 4ⁿ branch re-walks) that then need their own targeted defusing.

3. **"Unification" is order-dependent slot-filling.** The seed calls itself "Algorithm W," but its `unify`
   merges `Never` with anything and otherwise demands equality: no type variables, no substitution, no
   occurs-check — just `Option<Kind>` slots filled first-write-wins. A recursive self-call therefore reports
   the callee's *still-defaulting placeholder* kind, and whether the result comes out right depends on the
   order in which branches, arms, and constraints are discovered. **This is exactly the "infer a parameter's
   kind from its first use site" ad-hoc guessing that Hindley-Milner exists to replace, and that
   [[2026-07-04-inference-is-hindley-milner]] explicitly rejected — reintroduced through the back door as
   the *result*-kind solver.** Every one of the asks above is this single race: a placeholder kind unified
   against a concrete kind in an order that locks the wrong answer, plus its OOM tail (when the race never
   converges, the mismatched Heap argument forces unbounded per-call inlining of the recursive callee).

The through-line: a coarse, register-class lattice, re-derived at emission, resolved by order-dependent
slot-filling, cannot be made order-independent by patching — because order-independence is a property of
*having type variables and a real unifier*, which the design does not have. Each per-kind patch buys one
lattice point and leaves the next recursive/compound/polymorphic value to fail the same way.

**What the replacement must be (the design this drove for the from-scratch `cdzc` compiler).** Real
Hindley-Milner, structured to make each of the three failures impossible by construction:

- **Infer structure, not register class.** A `Type` sum with actual type variables (`TVar`), a `unify` with
  substitution and occurs-check, principal types. The wasm valtype is a trivial read-off of the *solved*
  type at lowering time — never the thing inferred. One solved type per node; its shape *is* its structure,
  so the `Kind`-vs-`Shape` gap (ask-65/73) cannot exist.
- **Inference is a separate `Hir → typed-Hir` pass, before lowering** — mandated by compiler-pipeline.md
  §"Emission Serializes A Lowered Representation" (emission MUST NOT decide a type). Lowering reads the
  already-solved type; there is no emit-time re-derivation and thus no "two mechanisms must agree" and no
  re-derivation cost blowup.
- **Order-independence for free.** Unification over type variables is inherently order-independent: a
  recursive self-call gets a fresh `TVar` that unifies with its concrete sibling regardless of branch/arm
  order. No tie-break tables, no first-write-wins. This is the "general result-unification, not per-kind
  patch" the seed admitted it needed — it retires the whole ask-14/18/34/65/73 class at once.
- **Monomorphization is the compile-time-evaluation tier**, the same reduction that folds and inlines
  ([[2026-07-04-generics-are-type-valued-parameters]]) — so the i64/i32 polymorphism boundary (ask-34) is
  not a special calling-convention hack but an instance of let-generalization + specialization.
- **First-class types meet HM at a bidirectional boundary** — HM ranges over a non-computational term core;
  type-valued-parameter and annotation sites switch to bidirectional checking
  ([[2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary]]); the bidirectional boundary
  and the monomorphization boundary are the same boundary.
- **Rows reuse the one unifier** (open records and effect rows), and a unification failure reports the
  minimal conflict at both sites.

**The requirement it drove.** No new normative requirement — this learning is the *empirical vindication*
of requirements already fixed on 2026-07-04: type-system.md §"Inference Is Principal-Type Inference By
Unification" (unify over type variables; principal types; propagate to every occurrence; contradictory-use
rejection), §"A Let-Bound Definition Is Generalized", and §"Inference And First-Class Types Meet At A
Bidirectional Boundary"; and compiler-pipeline.md §"Emission Serializes A Lowered Representation" (type
checking is an IR transformation, not an effect of emitting bytes). Its contribution is the *why now*: the
coarse-kind stopgap was tried at scale and failed one way at every lattice point, so the general
result-unification the spec already mandates is not optional polish — it is the thing that makes the whole
class of order-dependent return-kind bugs structurally impossible. Composes with
[[2026-07-04-inference-is-hindley-milner]] (the decision this proves out), the seed failure catalog
(asks 14/18/24/34/65/73), and the internal-IR-is-a-typed-sum direction
([[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]]). The from-scratch `cdzc`
compiler is where the replacement is built (its `Hir → Mir` step is "infer + monomorphize + lower"); the
coarse kind-fixpoint remains the interim only until then.
