## 25. 🟢 The self-hosted compiler selects the module entry by POSITION (first def), not by the name `main` — LANDED (gap 3m fixed) — awaiting loop re-probe

> **⏳ PENDING VALIDATION 2026-07-07.** Gap 3m (the seed compile-time-evaluator blowup) was fixed by the
> compiler agent, so the entry-reorder was RE-ADDED to `compiler.cdz` and **it now builds** (peak ~5.4 MB,
> no OOM — was >1.6 GB). `find-main`/`skip-main-nth`/`visit-def` (index-map, integer-`k` recursion — NOT the
> IList-threaded form that also blew up) walk the def named `main` to output position 0; `mod-fenv` and
> `read-defs` both walk the visiting order. **Verified via `compile-run`:** a helper-first module
> `(def (add a b) (+ a b)) (def (main) (add 20 22))` now COMPUTES (108-byte component, `run()` = 42) instead
> of the 88-byte trap-stub; `main`-in-the-middle works; `main`-first unregressed. The value-harness moved
> ~4 helper-first cases from `decline` → value-correct (agree/soft). A pre-existing latent bug the reorder
> EXPOSED (a Bool argument to an i64 param emitted invalid wasm — reorder-independent, main-first hit it too)
> was fixed alongside via a Bool→i64 arg coercion (`i64.extend_i32_u`). **To confirm → done:** the loop
> re-probes and pins a helper-first corpus case as `agree`/`soft` (not `decline`). Original report below.

## 25. 🔴 The self-hosted compiler selects the module entry by POSITION (first def), not by the name `main` — blocked on the seed compile-time-evaluator blowup (gap 3m)

**Finding.** `compiler.cdz`'s `read-module` takes the **FIRST def** as the nullary `run` entry (positional);
the native seed selects the def **named `main`** and reorders it to index 0. When they coincide (a main-first
module) the compiler is CORRECT end-to-end — `(def (main) (f 41)) (def (f x) (+ x 1))` emits a valid component
that runs = 42, a forward call to a later-defined helper included. When they don't (a helper-first module, the
common shape), positional func 0 is the parameter-taking helper, which `entry-guard` forces to a nullary
`KError` trap — a clean **decline**, not invalid bytes. So the multi-def user-function CALL machinery is
complete; the only gap is entry SELECTION.

**Verified (current compiler, probed 2026-07-07).**

| module | core func 0 | result |
|--------|-------------|--------|
| `(def (main) (f 41)) (def (f x) (+ x 1))` — main FIRST | main (nullary) | valid, runs = 42 ✅ |
| `(def (f x) (+ x 1)) (def (main) (f 41))` — helper FIRST | f (param'd) | valid, traps (clean decline) |
| `(def (main) (g)) (def (g) 42)` — main first, nullary callee | main | valid, runs = 42 ✅ |

Before the fix landed (caught mid-probe as the spike edited compiler.cdz live) the helper-first case emitted an
**invalid** component — `f` lifted as nullary func 0, `main` doing `i64.const 41; call 0`, `run` exporting func
0 — so the argument stranded: *"values remaining on stack at end of block."* `entry-guard` now makes that a
clean decline (single `run → unreachable`).

**Why it's blocked (not just unimplemented).** The name-based reorder IS written — `find-main` / `visit-def` /
`skip-main-nth` walk the def named `main` to index 0 — but adding those recursive functions to the LIVE compile
path tips the **seed's compile-time evaluator into an exponential blowup** (>1.6 GB OOM at this compiler's
scale): the recursive-inline / compiler-exponential-in-nesting family (SEED-GAPS **gap 3m**, and see
`compiler-exponential-in-nesting-depth`). So the reorder is reverted; positional entry + `entry-guard`'s clean
decline is the correct interim. **The entry reorder is gated on fixing the seed's compile-time-evaluator
blowup, not on any reader or backend capability.**

**Acceptance signal.** With gap 3m fixed, `find-main`/`skip-main-nth` can rejoin the live path; a helper-first
module then compiles (`f` reordered off the entry slot) instead of declining — the harness's helper-first
`decline`s become `agree`/`soft`, and the `error` bucket (invalid emissions) empties as every remaining
unsupported construct traps cleanly rather than emitting invalid bytes.

**Pinned (the working side — a main-first module).** `09-functions.sexp` *"the module entrypoint is the def
named main regardless of its position"* (`(def (main) (f 41)) (def (f x) (+ x 1))` → 42, AGREE) — pins that
entry selection is by NAME (a language requirement every other multi-def case, all main-last, left unpinned)
and doubles as a forward-reference pin.
Learning: `spec/learnings/2026-07-07-the-self-hosted-reader-compiles-a-multi-def-call-but-picks-the-entry-by-position.md`.

---

**🟢 LOOP-CONFIRMED 2026-07-07 (Run 63).** Re-probed via `compile-run`: a HELPER-FIRST module `(def (f x) (+ x
1)) (def (main) (f 41))` — which was a clean decline before the gap-3m fix (main wasn't func 0) — now compiles
and runs to **42** (`main` reordered to the entry by name). Byte gate: the mutual-recursion even/odd cases moved
decline → soft-disagree (they now compile, value-correct byte-different), and total declines/disagrees shifted
(153 → 141 disagree). Entry selection is now by NAME. Moved pending-validation → done.
