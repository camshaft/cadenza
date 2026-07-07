# The diagnostics pivot from Result-return to effects hit a parallel compile-entry lowering gap

*2026-07-07*

**What happened.** The diagnostics channel (the last hop for ~30 type-rejections `decline → agree`) has two
possible shapes, and the compiler has now tried both — hitting a compile-entry lowering gap on each:

1. **Result-return** (`compile : … → result<bytes, list<diagnostic>>`): declines because a deep `Core`
   sum-match in `compile`'s call graph mis-lowers under Result-shaping (ask-42 — a scrutinee-kind divergence:
   the heap-sum scrutinee is inferred non-heap, so the constructor arm falls to the scalar `gen_match_arms` and
   declines). Verified still open on the live seed.
2. **Effects** (a `Diag` effect, `(Diag.emit code)` at each rejection, collected by a handler — the operator's
   "diagnostics via effects" direction): the recursive-effectful *collection* now works (ask-45 fixed), but
   installing the `handle` at the `compile` entry declines "recursive effectful function on the compile-entry
   path not yet emitted" (NEW ask-46).

Verifying ask-46 on the live seed with the exact discriminators from its report:

| program | result |
|---|---|
| `handle` over a recursive effectful walk, `compile` entry | **declines** |
| the SAME handle under a `main`/`run` entry | **VALID** |
| the recursive effectful fn with NO handle, `compile` entry | **VALID** |

The `main`-vs-`compile` swap is decisive: the *entry kind* determines whether the recursive-effectful `handle`
lowering fires. The run entry got that lowering (ask-45); the compile entry did not. And it's presence-triggered,
not reachability-triggered — a `handle`-over-recursive-effectful *anywhere* in a compile-entry module declines,
even if `compile` never calls it.

**Why.** The honest cross-cutting observation — stated carefully, because I probed for a single unifying root and
did *not* cleanly find one: **both diagnostics routes hit a lowering gap specific to the `compile` entry, but of
different mechanism.** ask-46 is *cleanly* entry-kind-gated (same handle: run compiles, compile declines).
ask-42 is a scrutinee-kind divergence under Result-shaping (a distinct mechanism — my attempt to reproduce it as
a pure entry-kind difference declined for an unrelated reason, so I can't claim ask-42 and ask-46 share one
root). What they *do* share is the shape of the problem: **the `compile` entry is a less-complete lowering path
than the `run` entry** — features that lower under `run` (recursive-effectful handles; and, historically, the
whole self-hosting seam) reach the `compile` entry later, because `compile` is the newer ABI (it carries the
`list<u8> → list<u8>` / artifact-record marshalling that `run` doesn't). So a capability the compiler needs *at
its own entry* can be present-and-working under `run` yet unemitted under `compile`, and the diagnostics pass —
which by definition runs at the compile entry — is the first thing to need those capabilities *there*. The
lesson worth keeping: **when a self-hosting compiler grows a new entry ABI, its lowering coverage forks — the new
entry lags the old one, and the features that lag are exactly the ones the compiler needs to run *as* that
entry** (the diagnostics handler, an internal state effect, a deep analysis sum-match). Every internal-state
effect the compiler wants (diagnostics, symbol table, return-kind table, fresh-slot counter) threads a recursive
effectful handle, so ask-46 gates the operator's whole "lean on effects in the compiler" direction, not just
diagnostics.

**The requirement it drove.** No corpus case — ask-46 is a `compile`-entry ABI lowering gap (a
`compiler.cdz`/seed self-hosting concern), not a value-behavior the `(output (: v T))` oracle expresses; and the
effect/handler value behavior it needs is separately pinned by the growing effects corpus (which runs under the
`run` entry). The verification is recorded on ask-46 (reproduces on the live seed, with the entry-kind
discriminator confirmed), and this learning captures the cross-cutting shape: the diagnostics pivot (Result →
effects) traded ask-42 for ask-46, both compile-entry lowering gaps, and the durable path is extending ask-45's
recursive-effectful `handle` lowering to the compile-entry ABI (then compiler.cdz wires its already-built `Diag`
handler and diagnostics self-host). General lesson: **a new entry ABI on a self-hosting compiler forks its
lowering coverage; probe the run-entry-vs-compile-entry discriminator to tell "the feature is unimplemented"
from "the feature is unimplemented AT THIS ENTRY" — the latter is a much smaller, ABI-path-local fix.**
