# The final self-host blocker is a scale limit, not a shape gap — and a scale limit resists a minimal corpus case

*2026-07-07*

**What happened.** The reader reached the point where it can be *joined* to the pipeline, and that join
is the last blocker before self-hosting. The reader's `read-node : Bytes → Node` is verified end-to-end
as a Node builder — `read (quote (+ 1 2))` builds the right `Node` (a scalar → `NInt`; an array → an
`NPrim` whose head is the prelude symbol `name-eq` matched). Every reader primitive works. But feeding
its output to the compiler's real `resolve : Node → Core` declines: **"runtime compound element of a
kind the runtime cannot box yet"**. So `read → resolve → fold → lower → serialize → frame` cannot be
connected, even though every stage runs on its own and `compiler.cdz` still compiles (the join is
deliberately left uncommitted rather than parked as dead code, because dead `read-node` would serve no
purpose until `resolve` accepts its output).

The decisive observation is the *shape* of this blocker: **it does not reduce to a minimal case.**
Growing a recursive `Node → Core` resolver arm by arm, every structural feature compiles and runs at
runtime in isolation — `KConst`, `KAdd` (2-tuple), `KIf` (3-tuple), `KLet` (Int64 + heap), `KCall`,
a `KIf` whose branch is a `KBoolC` (the and/or desugar), a runtime `(Tuple String Node Node)` built and
matched, `head-prim` on a runtime String. I confirmed this directly: a 3-variant resolver runs, and a
6-variant heterogeneous resolver (`KConst`/`KBoolC`/`KAdd`/`KLt`/`KNot`/`KIf`) runs too (→ 4). Only the
**full 18-variant `Core` returned by the full `resolve`** declines. It is not about which `Node` is
passed either — `resolve` on a runtime `(Node.NInt 42)` (a scalar `KConst` arm that boxes no compound)
*also* declines, because the seed compiles the whole function and *some* arm's Core construction poisons
every call. So it is a **full-function, scale/union property**: a specific element-kind combination in
the 18-variant union that the runtime heap-boxer rejects on this path, though every sub-combination
boxes fine when built more directly.

**Why.** This is a different *kind* of blocker than the ones that preceded it, and the difference is the
lesson. Every earlier reader gap — the nested payload binder, the `Bytes.at` Option, the bare nullary
constructor, the recursive-Bool kind race, the runtime `tuple.N` — was a **shape gap**: a specific
syntactic/semantic form the seed didn't handle, reproducible in three or four lines, pinnable as a
corpus case that flips green when fixed. Tier 2f is a **scale limit**: the shapes all work; their
*union at size 18 on the runtime heap-box path* is what fails. A scale limit resists the corpus's
central technique — a minimal witness — because by construction it has no minimal witness (every
reduction of it passes). This matters for how the loop documents it: pinning a giant 18-variant resolver
would be brittle (fragile to the exact threshold, implementation-specific to the current boxer, and
likely to pass or fail for the wrong reasons as the seed changes), and pinning any tractable resolver
would pass today and guard nothing. So the honest artifact is **not a corpus case** — it is this
learning plus a precisely-bisected backlog entry that hands the seed engineer the localization (trace
which `gen_runtime_*` / heap-box path `resolve`-of-a-runtime-`NPrim` hits) rather than a reduced
repro that does not exist. The rule for future loops: **a shape gap earns a minimal corpus case; a
scale limit earns a bisection and a backlog entry, and its regression guard is the real artifact
(here, the whole `compiler.cdz` `resolve`) compiling once the fix lands, not a synthetic minimal case.**

**The requirement it drove.** No corpus case — deliberately, per the reasoning above (every tractable
resolver passes; the failing one is the full 18-variant `resolve`, too large and threshold-specific to
pin durably). The finding is recorded here and as **SPEC-BACKLOG item 16**, the final self-host blocker,
with the agent's bisection carried over: the reader (`read-node`) is built and verified; the real
`resolve` on a runtime-built `Node` declines "cannot box"; every structural sub-shape works, so the
seed fix is to find which element-kind combination in the 18-variant `Core` union the runtime boxer
rejects on the `resolve` path. Its regression guard, when fixed, is `compiler.cdz` connecting
`read → resolve → … → frame` end-to-end and compiling — the two-compilers gate on the whole compiler,
which is the point of the exercise. This also updates the self-hosting picture: with items 14 and 15
fixed and the reader built, **Tier 2f is the single remaining hard blocker on `bytes → bytes`
self-hosting** (items 12 symbol-table `from-bytes` and 13 list patterns remain, but the reader routes
around 12 for structure and 13 is ergonomic, so 2f is the true gate).
