## 49. ✅ FIXED (seed 16:31, re-probed 2026-07-07) — a recursive-effectful `handle` returning a compound now lowers on the run/emit path

**✅ RE-PROBED FIXED.** The minimal repro (recursive `Diag` handle whose result is `(Bytes.of …)`, on the `emit`
entry) now RUNS: `ran → Value("b\"…")` (was `declined: recursive effectful function returning a compound / under
host delegation not yet emitted`). So the run/emit-entry lowering of a compound-returning recursive-effectful
handle landed (the twin of ask-46's compile-entry fix). This clears the gate-safety blocker for activating the
`Diag` handler — BUT wiring the diagnostics `compile-output` record inside the handle then hit a SEPARATE, narrower
gap (ask-51: the artifact-ABI detection doesn't look through a `handle`). So the handler LOWERS now; the remaining
blocker is detection, not lowering. Moved open → done.

---
_Original finding (now resolved) below._

## 49. 🔴 A recursive-effectful `handle` whose result is a COMPOUND declines on the `emit`/`run()` path — the last hop for effect-based diagnostics

**Finding.** ask-46 landed the recursive-effectful `handle` under the `compile` entry (verified: the diagnostics
handle self-hosts via `compile-run`). But the differential GATE drives compiler.cdz via `emit` → `run()` (the
harness runs the compiler as a `run()` component), and on THAT path a recursive-effectful `handle` whose
**result value is a runtime compound** (a `list`/`Bytes`/record) declines:
```
declined: recursive effectful function returning a compound / under host delegation
          not yet emitted (scalar + runtime-scalar paths covered)
```
A recursive-effectful `handle` that returns a **scalar** works on the same path (ask-45). So the missing piece
is: the value the `handle` *evaluates to* being a runtime compound (not the state — the RESULT).

**Boundary, isolated (2026-07-07, on the STABLE seed `implementation/stable/cadenza-seed` 16:05):**

| shape | result |
|---|---|
| run/`emit`: scalar state, **scalar** result | ✅ `ran → Value` |
| run/`emit`: **LIST state**, scalar result | ✅ `ran → Value("3")` (state-compound is fine — that's ask-45) |
| run/`emit`: scalar state, **compound** result (inline `(Bytes.of …)`) | 🔴 **declines** (above) |
| run/`emit`: scalar state, **compound** result via a HELPER (`(mk 7)`) | 🔴 **declines** (same — not inline-specific) |
| run/`emit`: **compound** result but **NON-recursive** effectful body | ✅ `ran → Value("b\"…")` |
| `compile` entry: the SAME compound-returning recursive handle (`compile-run`) | ✅ `Ok (1 byte)` (ask-46) |

**Sharpened:** the trigger is precisely **recursion + a compound RESULT value** (the value the handle-containing
computation evaluates to) — independent of the state kind (list state alone is fine) and independent of
inline-vs-helper. Non-recursive is fine; scalar-result is fine; only recursive-effectful WITH a runtime-compound
result declines. Confirmed on the STABLE seed (not a `seed/`-build artifact): the compile-entry path emits a
compound-returning recursive-effectful handle (the sibling's Run-95 verified the full `{artifacts, diagnostics}`
shape there), but the plain `run()`/`emit` entry does NOT. The differential GATE drives compiler.cdz via
`emit`→`run()` (runs it as a `run()` component), so it hits the run-entry path — hence activating the `Diag`
handler broke 169 gate cases even though `compile-run` self-hosted fine.

Confirmed on the STABLE seed (not a `seed/`-build artifact): the compile-entry path emits a compound-returning
recursive-effectful handle (the sibling's Run-95 verified the full `{artifacts, diagnostics}` shape there), but
the plain `run()`/`emit` entry does NOT. The differential GATE drives compiler.cdz via `emit`→`run()` (runs it
as a `run()` component), so it hits the run-entry path — hence activating the `Diag` handler broke 169 gate
cases even though `compile-run` self-hosted fine.

So it is the **`emit`/`run()` lowering** of a recursive-effectful handle whose result is a compound — the
compile-entry path got it (ask-46) but the run-entry path did not. As with ask-46, it fires from the mere
PRESENCE of such a `handle` in the module (169 gate cases errored when compiler.cdz merely CONTAINED the
compound-returning diagnostics handle, even before `compile` used it).

**Minimal repro (declines):**
```
(module m
  (effect D (op emit (-> Int64 Unit)) (op get (-> Unit (list Int64))))
  (def (w n) (if (< n 1) 0 (do (D.emit n) (w (- n 1)))))
  (def (main) (handle (list) ((D.emit (v) s (resume unit (List.push s v))) (D.get (u) s (resume s s)))
                 (do (w 3) (Bytes.of (list (List.len (D.get unit))))))))    ; result = Bytes (compound)
```
→ `declined: recursive effectful function returning a compound / under host delegation not yet emitted`.
Change the body's result to a scalar (`(List.len (D.get unit))`) and it runs (`→ 3`).

**Why it matters.** This is now THE last hop for effect-based diagnostics (the operator's direction). The `Diag`
effect + recursive `check-*` pass are built in compiler.cdz and proven correct (compile-run + isolation); the
handler was wired at `compile` and self-hosts via `compile-run` — but `compile` returns `Bytes` (a compound)
from the `Diag` handle, so the GATE path (`emit`/`run()`) declines, breaking 169 cases. The handler was
therefore reverted (compiler.cdz keeps the `Diag` decl + `check-*` pass, which compile fine; `compile` stays
bare-`Bytes`; the one-line handler swap is documented in `compile`'s docstring). Emitting a recursive-effectful
handle whose result is a runtime compound on the run/emit path — the same lowering ask-46 gave the compile
entry — unblocks it.

**Acceptance signal.** The minimal repro above runs (`emit` → a Bytes value), and compiler.cdz's `compile` body
can be the `Diag` handler (returning `(compile-program …)` bytes) with the gate GREEN (no `emit`/`run()`
errors). Then the diagnostics collection runs on the gate path too, and — once a diagnostics-carrying RETURN
channel exists (ask-41 artifact record / ask-42 result<>) — the collected `(Diag.collect unit)` is surfaced and
the ~30 ask-30 rejections reach `agree`.

**Status.** 🔴 Seed — the `emit`/`run()`-entry lowering of a recursive-effectful `handle` returning a runtime
compound (the compile-entry path has it after ask-46). Related: ask-45 (scalar recursive-effectful),
ask-46 (compile-entry recursive-effectful — the twin, on the other entry), ask-41/ask-42 (the diagnostics RETURN
channel, the hop after this), ask-30 (the rejections this reports). Current state: compiler.cdz reverted to
bare-`Bytes` (self-hosts, 27 agree / 0 hard / 0 error); the `Diag` decl + `check-*` pass stay (compile fine).

**✅ LOOP-VERIFIED 2026-07-07 (Run 95) — independently reproduced on the refreshed STABLE seed (16:05, SHA256SUMS
OK), zero disagreement with the finding above.** Three-way discriminator via `compile-run`: run+scalar-result
handle ✅ (`→ 3`); run+compound-result handle 🔴 (`recursive effectful function returning a compound / under host
delegation not yet emitted`); compile+compound handle ✅ (the ask-46 record shape →
`Diagnostics: [(CDZ0201,bad),(CDZ0201,bad)]`). Confirmed the byte gate held at 65/124/386 with `compile`
bare-Bytes and WRONG=0, gate 570/0 — i.e. the sibling's revert restored the gate cleanly. This is squarely a
run-vs-compile entry-ABI lowering fork (same lowering, other entry), the exact twin structure of ask-46. Learning:
`spec/learnings/2026-07-07-a-fix-verified-on-one-entry-does-not-move-a-gate-driven-through-another.md`.
