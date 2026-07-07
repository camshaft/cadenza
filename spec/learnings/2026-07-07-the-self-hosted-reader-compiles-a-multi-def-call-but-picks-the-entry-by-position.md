# The self-hosted reader compiles a multi-def user-function call — but it picks the entry by position, and the name-based reorder is blocked on a seed blowup

*2026-07-07*

**What happened.** With the reader's atom-decode miscompiles closed (float/unbound-name/other-major now
decline — [[the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining]]), the interim
corpus harness surfaced a new bucket: **16 `error` cases** — programs where `compiler.cdz` emits an *invalid*
component (fails wasm validation) rather than either computing a value or cleanly trapping. The most concrete
was *"an underscore-prefixed function parameter binds its argument"* (`(def (f _1) (+ _1 1)) (def (main) (f
41))` → 42). Probing it directly (NOT trusting the harness's per-case label — the same discipline a prior
cycle's proxy-reasoning error taught) reduced it two ways:

1. **The underscore is a red herring.** The plain-name twin `(def (f x1) (+ x1 1)) (def (main) (f 41))` fails
   identically. The trigger is the *shape* — a two-def module whose entry calls a user function — not the name.
2. **It is the entry-selection order, not the call.** Building the invalid component and disassembling it showed
   `f` emitted as core func 0 (`() → i64`), `main` as func 1 doing `i64.const 41; call 0`, and the component
   exporting func 0 as `run`. So the argument `41` was pushed for a function declared nullary — *"values
   remaining on stack at end of block"*. The reader takes the **FIRST def as the nullary `run` entry**
   positionally; native selects the def **named `main`** and reorders it to index 0.

Mid-probe the spike **edited `compiler.cdz` live** (89,162 → 91,528 B) and the same input flipped from the
invalid 100-byte emission to a clean 88-byte **decline** (a single `run → unreachable`): an `entry-guard` now
forces a parameter-taking func-0 to a nullary `KError` trap, so a helper-first module traps cleanly instead of
emitting stack-imbalanced bytes. A controlled experiment on the *current* compiler confirmed the whole picture:

| module | core func 0 | result |
|--------|-------------|--------|
| `(def (main) (f 41)) (def (f x) (+ x 1))` — main FIRST | main (nullary) | **valid, runs = 42** ✅ |
| `(def (f x) (+ x 1)) (def (main) (f 41))` — helper FIRST | f (param'd) | valid, **traps** (clean decline) |
| `(def (main) (g)) (def (g) 42)` — main first, nullary callee | main | valid, runs = 42 ✅ |

So the reader's **multi-def user-function call works end-to-end** — a forward call resolves through the module
function environment, the argument lowers, the callee runs, `f(41) = 42` — **whenever the entry def happens to
be first**. The only gap is entry *selection*: positional-first vs. named-`main`.

**Why.** The name-based reorder is not hard to write — the spike implemented `find-main`/`visit-def`/
`skip-main-nth` to walk `main` to index 0 — but adding those recursive functions to the LIVE compile path
**tips the seed's compile-time evaluator into an exponential blowup** (>1.6 GB OOM at this compiler's scale),
the same recursive-inline / compiler-exponential-in-nesting family as [[compiler-exponential-in-nesting-depth]]
and the fixpoint blowup of [[a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop]]. So the
reorder is *reverted* and the compiler keeps positional entry, with `entry-guard` making the mismatch a clean
decline (a trapping component) rather than invalid bytes. This is the correct triage under the constraint —
**when you cannot yet emit the right thing, decline cleanly, never emit invalid bytes** — and it localizes the
blocker precisely: the entry reorder is gated on the seed's compile-time-evaluator blowup (SEED-GAPS gap 3m),
not on any missing reader or backend capability. The call machinery is *done*; only the entry-slot remap waits.

**The requirement it drove.** The finding is a *language* requirement the compiler.cdz gap made visible, so it
pins as a corpus case: `09-functions.sexp` *"the module entrypoint is the def named main regardless of its
position"* — `(def (main) (f 41)) (def (f x) (+ x 1))` → 42 (AGREE). Every other multi-def case in that file
places `main` LAST, so nothing pinned that entry selection is by NAME, not position; this case does, and it
doubles as a forward-reference pin (a call to a later-defined helper). The still-open compiler-side work — the
`error` bucket becoming clean `decline`s, and ultimately the `main`-named-entry reorder — is recorded in
SPEC-BACKLOG (the entry-selection item and the emit-frontier checklist), gated on seed gap 3m. General lesson,
a recurrence of this loop's standing rule: **a harness's per-case bucket is an aggregate to probe, not a
diagnosis** — the "underscore parameter" label named the wrong cause (the underscore), and disassembling the
actual bytes turned "invalid emission on a param name" into "positional-vs-named entry selection," which is a
different fix in a different place. And the in-flight-edit corollary: probe the artifact as it is NOW, because
the spike may fix the very thing under the probe (here the invalid emission became a clean decline mid-cycle) —
report the current truth, not the snapshot you started with.
