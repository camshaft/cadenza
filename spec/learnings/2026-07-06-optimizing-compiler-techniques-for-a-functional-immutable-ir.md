# Optimizing-compiler techniques for a functional/immutable IR — a grounded catalog

*2026-07-06*

**What happened.** Having decided that the compiler must lower the AST through a resolved, analyzed
intermediate representation so that emission is a pure serializer
([[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]]), we deliberately *deferred* the
harder design question — how many IR layers, what each carries, and which optimizations are Core→Core
rewrites versus needing a lower, control-flow-shaped IR — until we had surveyed the prior art rather than
reinventing it. This entry is that survey: a fact-checked catalog of techniques from optimizing-compiler
research (weighted toward functional and immutable-heap languages), each verified against primary sources
and grounded in Cadenza's specifics — a WebAssembly-component target, an acyclic immutable
reference-counted heap, algebraic effects with statically-determined handlers, and a byte-deterministic
differential gate under the "decline, don't miscompile" discipline. It is **forward-looking design input,
not a post-mortem**, and it drives **no new requirement**; it is the evidence base for the layer design
when we build it. Being a learning, it is also the one place the specification may name the concrete
implementations and papers it draws on.

Two honesty notes carried from the research and preserved here rather than smoothed over. First, **source
concentration**: the IR-shape, progressive-lowering, and effect-lowering findings each rest on
essentially one canonical primary source (respectively *Compiling without Continuations*; the MLIR
rationale; *Generalized Evidence Passing*), each peer-reviewed and shipping in production, but not
multi-source corroborated. Second, **coverage gaps**: the correctness-of-passes thread and a number of
classic functional optimizations (inlining heuristics, deforestation/stream fusion, worker/wrapper,
let-floating, strictness/demand analysis, destination-passing, uniqueness/linearity types, and the
CHAMP/RRB representation question) produced *no* surviving verified claims, so this catalog does not
cover them and they remain open (see Gaps). Benchmark figures are order-of-magnitude, not portable
guarantees.

---

## The techniques

Each entry: **(a)** what it is · **(b)** how it fits Cadenza · **(c)** IR impact · **(d)** risk/caveat.

### IR shape & layering

**1 — Direct-style ANF + explicit join points, not CPS.** *(a)* Adding *join points* — second-class
continuations, i.e. saturated tail calls that are never captured in a closure or thunk and compile to
zero-allocation stack-adjusting jumps — to a direct-style A-normal-form IR recovers all the optimization
power classically attributed to CPS; the authors "know of no optimizing transformation" available to a
CPS compiler but not their direct-style one, while avoiding CPS's committed evaluation order, which
inhibits code motion (let-floating), CSE, and rewrite-rule matching (Maurer/Downen/Ariola/Peyton Jones,
PLDI 2017 — GHC Core ships this). *(b)* This is the shape the resolved Core rung should take: it keeps
folding and rewriting easy (the whole point of a serialize-only backend) and its close correspondence to
SSA eases the eventual control-flow-shaped lowering. Cadenza's *abortive* handler lowering (`block`/`br`)
and its `match` compilation are exactly join points — the principled IR construct for "many branches jump
to one continuation." *(c)* Core needs `join`/`jump` as *first-class constructs distinct from
`let`/`call`*, with the type discipline that resets the join-point environment to empty in any subterm
evaluated in an unknown context (enforcing second-classness). *(d)* The load-bearing subtlety: join
points must be explicit *so that transformations preserve them*. Treating a join point as an ordinary
`let` (GHC's pre-2017 behavior) lets case-of-case float it, force closure allocation, and *increase*
allocation — a silent pessimization, the analogue of our effects under-frame trap. Recognizing them only
in the backend is too late.

**2 — Nanopass decomposition: many single-task passes, each IL a delta.** *(a)* Structure the AST→bytes
path as dozens of fine-grained passes, each doing one job over a *formally defined* intermediate
language, where each IL is written as a **delta** from its neighbor (explicit `-`/`+` clauses on
terminals and productions) rather than re-specified in full; the framework then auto-generates traversal
boilerplate and checks each pass's output is well-formed (Keep & Dybvig, ICFP 2013; Sarkar/Waddell/Dybvig
JFP educational pearl). *(b)* This is precisely our "each transformation is isolated; emission is a pure
serializer" plan, and it is what makes a self-hosted compiler tractable to grow and test — one pass, one
concern, one checkable grammar. *(c)* Argues for representing each IR rung's grammar explicitly (a
`(type …)` per IL, or a shared Core with per-pass well-formedness predicates), so a pass that produces an
ill-formed term is a compile-time error — the "decline, don't miscompile" discipline applied to the
compiler's own internals. *(d)* Overhead is real but modest: nanopass Chez stayed within ~2× (worst-case
avg 1.75×) despite replacing 5 backend passes with 50+ — and that figure is *confounded* by a
simultaneously-introduced slower register allocator, so it overstates pure nanopass cost. Watch it
against our known exponential-in-nesting blowup, which a resolved IR should retire, not compound.

**3 — MLIR-style progressive lowering; don't lower too far too early.** *(a)* Represent code at multiple
abstraction levels and lower *progressively* (target-independent → target-dependent) rather than in one
step; this is what lets passes be modular and cleanly split into reusable target-independent versus
target-specific stages (MLIR rationale, Lattner et al. CGO 2021). Critically, dropping to a near-assembly
IR *too early* destroys high-level structure (loops, and for us: effect handlers, persistent-collection
operations, RC ownership intent, join points), which a backend then cannot reconstruct. *(b)* This sets
our layer *boundaries*: keep high-level semantics in the upper Core rung(s) and materialize
stock-wasm-component shape only at the lowest rung — the target-independent/target-dependent seam is the
`select`→`serialize` boundary. *(c)* Justifies *more than one* rung below Core when a pass needs
control-flow-explicit or locals-explicit structure the high Core doesn't carry — inserted on demand, not
speculatively. *(d)* The discipline is "lower when a pass needs the lower form," not "lower because it's
tidy"; each added rung is boilerplate and a place for drift.

### Immutable-heap optimizations

**4 — Perceus reference counting + reuse analysis + FBIP, as Core→Core passes.** *(a)* On an acyclic
immutable heap with non-atomic RC — *exactly Cadenza's* — Perceus emits precise reference-count
instructions (dup/drop insertion, drop specialization, dup/drop fusion) as a **static transformation of
the resolved core**; cycle-free programs are provably *garbage-free* (only live references retained).
Layered on top, *reuse analysis* turns a constructor whose argument's refcount is 1 into an in-place
update — the FBIP ("functional but in-place") paradigm, a compile-time property programmers can rely on —
and *borrowed-vs-owned* distinction (with heuristic borrow inference) removes needless count traffic
(Reinking/Xie/de Moura/Leijen, PLDI 2021; Ullrich & de Moura "Counting Immutable Beans," IFL 2019 / Lean
4). *(b)* Memory records Perceus RC as essentially complete for Cadenza; this research *names the
precondition that completes it* — acyclicity is what makes the counting precise — and confirms RC/reuse
belong in the IR, not in byte emission. *(c)* Core needs **explicit control flow, per-binding
ownership/borrow annotations, and visible constructor/match structure** so reset/reuse tokens can be
threaded; this is a concrete argument for what the resolved Core must carry. *(d)* In-place fires on a
*runtime* `is-unique(x)==1` check with a copy-on-share fallback, so it is **observationally transparent**
— which is exactly why it does not perturb emitted bytes and stays compatible with byte-determinism. But
the verified results concern scalar/constructor values; their interaction with our CHAMP/RRB
structural-sharing collections is *not* established (see Gaps).

### Effect-handler lowering

**5 — Evidence passing + a tail-resumptive fast path → stock wasm.** *(a)* Algebraic effects compile to
stock WebAssembly with *no* special runtime or delimited-continuation support: *evidence-passing
semantics* pushes each handler's evidence down to every operation site as an evidence vector, so a
`perform` becomes a local transition with constant-offset lookup whose cost is *independent of intervening
handlers* (no runtime search of the context); and *tail-resumptive* operations (clauses `op = λx.λk. k e`
with `k` not free in `e`) evaluate **in place** on the current stack, skipping continuation capture
entirely. Once all transitions are localized, handlers get a direct monadic (multi-prompt) translation
into plain typed lambda calculus that targets stock platforms including WASM (Xie & Leijen, *Generalized
Evidence Passing*, ICFP 2021; disabling the tail-resumptive path made Koka's state benchmark ~10× slower;
evidence passing made counter1 ~1.5× faster than yielding multicore OCaml). *(b)* This *is* Cadenza's
effect model, independently validated: our handler resolution is dynamic-in-extent but statically
determined, and our "tail-resumptive = plain inlined code" classification is the fast path the paper
proves sound. Our current seed collapses the evidence vector *further* — monomorphization/inlining
specializes each performing function per handler context, so the evidence is a compile-time constant
rather than a runtime vector; evidence-passing is the **runtime-evidence fallback** for exactly the case
monomorphization can't pick a single copy (a recursive or runtime-chosen handler — our Tier-3 wall).
*(c)* Core needs per-operation **evidence routing** (a canonically-ordered, constant-offset vector) *and*
the static **tail-resumptive / abortive / general** classification carried as an annotation — matching
the three-class classifier the effects design already defines. *(d)* General/multi-shot reified
continuations still heap-allocate closures — precisely the Tier-3 wall at recursive/runtime-chosen
handlers, which should be pushed to the lower/CFG-shaped stage or declined. **Time-sensitivity:** this
2021 SOTA predates WasmFX/stack-switching and targets stock *core* wasm; if the component model gains
native stack switching, the constraint motivating monadic lowering could relax.

### Correctness of passes

**6 — Per-technique semantics-preservation, not a generic verifier (weaker evidence).** *(a)* The
correctness evidence that survived verification is *per-technique preservation proofs*: the
tail-resumptive optimization is proved *contextually equivalent* to the unoptimized program, and Perceus
is proved *precise/garbage-free* (never drops a live reference). Because reuse fires on a runtime
uniqueness check with copy-on-share, the RC optimization is observationally transparent and does not
change emitted bytes — the property class needed to keep output byte-deterministic under optimization.
*(b)* This dovetails with "decline, don't miscompile" and the byte-identical differential gate: each
Core→Core optimization should ship with (or be gated by) a preservation argument, and any construct
lacking a *sound, deterministic* lowering is a compile error rather than wrong bytes. *(c)* No specific IR
feature required beyond the observational-transparency property above. *(d)* **This finding is a synthesis
over correctness-adjacent proofs, not dedicated evidence.** The thread's named apparatus — translation
validation (Necula), Alive/Alive2, CompCert, verified pass frameworks, property-based/differential pass
testing — did *not* surface as verified claims. The differential gate proves byte-equality of *outputs*
but not that each individual pass preserves *source semantics*; closing that is an open design question.

---

## What it implies for the IR ladder

Converging the findings into the deferred layer question, the research points at a concrete shape (a
recommendation to weigh when we design it, not a mandate):

- **One rich resolved Core rung, in direct-style ANF with explicit join points**, carrying: resolved
  binders; type/kind annotations; per-binding ownership/borrow annotations (for RC/reuse); and
  per-operation effect evidence + tail/abortive/general classification. This single rung hosts the
  Core→Core rewrites: constant folding, RC/reuse/FBIP insertion, tail-resumptive and abortive effect
  lowering, inlining/monomorphization.
- **Progressive lowering below Core to a stock-wasm-component rung**, materializing target shape only at
  the bottom (`select`→`serialize`). Insert an intermediate control-flow/locals-explicit rung **on
  demand** — the first concrete demand is Tier-3 reified continuations (recursive/runtime-chosen handlers)
  and any allocation-sensitive lowering that wants an SSA-like form; ANF+join-points already corresponds
  closely to SSA, easing that step.
- **The Core→Core vs lower-IR split falls out cleanly:** RC/reuse, tail-resumptive/abortive effects, and
  the classic rewrites are Core→Core; only *general/one-shot reified continuations* clearly need the
  lower, CFG-shaped stage — the same Tier-3 boundary the effects work already found from the other
  direction.
- **Pass discipline: nanopass-style single-task passes with checkable per-rung grammars**, each ideally
  carrying a preservation argument, with the differential gate as the byte-level backstop.

## Gaps this research did not close

Recorded so they are not mistaken for covered ground:

- **No principled layer *count* for Cadenza.** The methodology (many single-task passes; progressive
  lowering; ANF≈SSA) is validated; the exact rung count and the mapping of our concrete stages onto rungs
  is still ours to decide.
- **Classic Core→Core functional optimizations were not assessed.** Inlining heuristics,
  deforestation/short-cut & stream fusion, worker/wrapper, let-floating/full laziness, strictness/demand
  analysis, arity raising/eta-expansion, unboxing, destination-passing — none surfaced as verified
  claims, so their fit to our *eager, RC'd, byte-deterministic* setting and their IR requirements are
  unassessed. (Note several are laziness-specific and may not transfer to an eager language.)
- **CHAMP/RRB × Perceus reuse is unaddressed.** The RC results concern scalar/constructor values; how
  structural sharing in our persistent maps/sets/vectors coexists with FBIP in-place update needs its own
  investigation.
- **Cheap general/one-shot continuations on stock wasm (no WasmFX)** — whether join-point sharing /
  bind-inlining make one-shot acceptable, or such handlers should be declined/deferred to a
  component-boundary mechanism — is open.
- **The verification apparatus for byte-determinism under optimization** (per-pass translation validation
  vs a verified pass framework vs property/differential testing against the reference compiler) did not
  surface and remains a design gap.

## The requirement it drove

**None — deliberately.** This learning confirms and refines the direction already fixed by
[[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]] (emission is a serializer;
analysis is IR transformation) and complements
[[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]] (the backend instruction
sum). Mandating a specific IR shape (ANF+join points, an evidence vector, a layer count) in the
specification would over-constrain an implementation choice; the spec's `compiler-pipeline.md`
§Representation requirements state the *obligations* (resolve before selecting; emit from a lowered form;
serialize a typed instruction sum) and leave the ladder to be rediscovered — which this catalog exists to
inform when that work begins. Related effect-lowering design:
[[2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code]],
[[2026-07-06-implementing-effects-in-the-seed-inlining-resolves-cross-function-until-recursion]];
immutable-heap substrate: [[2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery]].

## Sources

All primary/peer-reviewed unless noted; each supporting claim was verified 3-0 against these.

- Maurer, Downen, Ariola, Peyton Jones. *Compiling without Continuations.* PLDI 2017.
  <https://simon.peytonjones.org/assets/pdfs/compiling-without-continuations.pdf>
- Keep & Dybvig. *A Nanopass Framework for Commercial Compiler Development.* ICFP 2013.
  <https://www.cs.tufts.edu/comp/150FP/archive/icfp13.pdf> · Sarkar/Waddell/Dybvig, JFP educational
  pearl. <https://www.cambridge.org/core/journals/journal-of-functional-programming/article/educational-pearl-a-nanopass-framework-for-compiler-education/1E378B9B451270AF6A155FA0C21C04A3>
- MLIR Rationale (Lattner et al., CGO 2021). <https://mlir.llvm.org/docs/Rationale/Rationale/>
- Reinking, Xie, de Moura, Leijen. *Perceus: Garbage Free Reference Counting with Reuse.* PLDI 2021.
  <https://xnning.github.io/papers/perceus.pdf> · TR v4
  <http://www.microsoft.com/en-us/research/wp-content/uploads/2020/11/perceus-tr-v4.pdf>
- Ullrich & de Moura. *Counting Immutable Beans: RC Optimized for Purely Functional Programming.* IFL
  2019. <https://www.microsoft.com/en-us/research/publication/counting-immutable-beans-reference-counting-optimized-for-purely-functional-programming/>
- Xie & Leijen. *Generalized Evidence Passing for Effect Handlers.* ICFP 2021.
  <https://www.microsoft.com/en-us/research/wp-content/uploads/2021/08/genev-icfp21.pdf>
