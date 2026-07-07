# The self-hosting gate shifted from "seed capability" to "the compiler's source is within its own accepted subset"

*2026-07-07*

**What happened.** With the reader complete (`module bytes → component` for a whole multi-`def`
module, [[2026-07-07-the-whole-module-reader-is-wired-module-bytes-to-component.md]]) and every seed
blocker cleared, the spike's own status note reframed what remains: *"What's left for TRUE
self-hosting: grow the operator/type/effect surface until the compiler's own source is within the
subset it accepts (it currently handles arithmetic/comparison/bool/if/let/call/multi-def over
Int64/Bool)."* This is a **category shift in the blocker**. For twenty-odd cycles the gate was always a
*seed capability* — a shape the seed couldn't compile (nested binders, the `Bytes.at` Option, the
recursive-Bool kind race, the runtime-`tuple.N`, the `Never`-on-heap box). Those are all fixed. The
gate is no longer "can the seed compile the compiler's constructs?" but **"does the compiler accept the
language its own source is written in?"** — a bootstrapping-completeness question, not a seed-gap
question.

Concretely: `compile-bytes` (the whole-module entry, factored out this cycle) reads and compiles a
module written in the subset {arithmetic, comparison, boolean connectives, `if`, `let`, calls, multi-def,
Int64/Bool}. The compiler's *own* source uses more than that — sum types, `match`, `String`, recursion
over heap values, the whole reader/resolver machinery. So `compiler compiles compiler` is gated not on
the seed but on the Cadenza compiler's front end and backend growing to accept those constructs (which
the seed already compiles — the compiler just doesn't yet *emit* code for a program that uses them). A
small verified reader capability landed alongside the reframing: `cbor-skip` now handles CBOR **tags**
(major 6 — the `39` bare-name marker `d8 27 <idx>` a module's names use), completing its item-kind
coverage (array / string / tag / scalar).

**Why.** This reframing is the healthy end-state of the two-compilers flywheel and worth naming
explicitly, because it changes what future loop iterations should look for. While the gate was
seed-capability-shaped, the loop's job was *probe the seed, find the shape it miscompiles or declines,
pin a corpus case, let the seed be fixed* — a defect-finding loop. Now that every such defect is
cleared, the loop's job shifts to *coverage*: which language constructs does the Cadenza compiler's
front end resolve and its backend emit, and which does its own source need that it does not yet handle?
That is not a hunt for bugs but a measurement of a **subset frontier** — the set of programs the
self-hosted compiler can compile, which must grow to contain the compiler's own text before
`compiler compiles compiler` closes. The distinction matters because the two produce different
artifacts: a seed defect earns a corpus case (a specific miscompile pinned); a subset-frontier gap
earns a *capability inventory* (what's in, what's out, what the source needs) — the latter is a
backlog/roadmap artifact, not a per-shape corpus case, because "the compiler doesn't yet emit `match`
on a user sum in its own output" is a scope statement, not a defect. The spike is at that inflection:
the machinery is proven end-to-end on a small subset; growing the subset to self-inclusion is the
remaining work, and it is bounded by the compiler's own source, not by the seed.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR skip steps over a
tagged item to the value it wraps"* — pins the reader's last item-kind: `cbor-skip` over a CBOR tag
(`d8 27 01`, tag 39 wrapping uint 1) → offset 3, the `39` bare-name marker a canonical-AST module uses.
It completes the navigation primitive's coverage (array / string / tag / scalar) that a reader needs to
traverse the whole AST, and **PASSES**. Beyond that, no new corpus case and no new backlog item from the
reframing itself — it is a *status* observation, not a defect: the executable semantics now witnesses
the whole `bytes → AST → typed-IR → component` path (both reader halves, the resolver join, the backend
spine), and what remains for self-hosting is subset growth (the compiler accepting sum types / `match` /
`String` / recursion in the programs it *emits*, which its own source needs) plus scale (TCO for deep
sources, [[deep-recursion-traps-at-host-stack-limit]]). The operator-facing framing: **self-hosting is
no longer seed-blocked; it is bounded by the compiler's accepted subset reaching self-inclusion.** The
non-blocking backlog items 12 (`from-bytes` across a boundary) and 13 (list patterns) are part of that
subset frontier — ergonomic surface the compiler's own source would use — now recategorized from
"reader gate" to "subset-growth" work.
