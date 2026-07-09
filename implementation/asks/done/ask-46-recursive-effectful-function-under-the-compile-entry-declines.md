## 46. ✅ FIXED (seed 15:59, re-probed 2026-07-07) — a recursive effectful `handle` under the `compile` ENTRY now lowers

**✅ RE-PROBED FIXED.** The minimal repro now compiles (`(def (compile b) b)` with a `handle`-over-recursive-effectful
helper → `Ok`, was `declined: recursive effectful function on the compile-entry path not yet emitted`), AND the
full diagnostics shape works: a `compile` body that installs a `Diag` handler, recurses a Core tree performing
`Diag.emit` at each `KBad`, `collect`s the list, and branches on its length → `Ok (1 byte)` (the KBad triggered
the stub path). So the compile-entry ABI now composes with the recursive-effectful lowering (ask-45's fix
extended to the compile entry). This UNBLOCKS installing the diagnostics handler in compiler.cdz — the `Diag`
decl + `check-*` pass were already built and validated, so activation is the one-line `handle` swap in `compile`.
Moved open → done.

**✅ LOOP-VERIFIED 2026-07-07 (Run 95) — independently re-probed on the refreshed STABLE seed (16:05, SHA256SUMS
OK).** Ran the handoff's exact target shape via `compile-run` on `implementation/stable/cadenza-seed`:
```
(module m
  (effect D (op emit (-> Int64 Unit)) (op collect (-> Unit (list Int64))))
  (def (w n) (if (< n 1) (D.collect unit) (do (D.emit n) (w (- n 1)))))
  (def (compile inputs)
    (record (artifacts (list))
      (diagnostics (handle (list)
        ((D.emit (v) s (resume unit (List.push s (record (code "CDZ0201") (message "bad") (severity 0)))))
         (D.collect (u) s (resume s s))) (w 2))))))
```
→ `VALID compile component (4182 bytes)` / `compile → Diagnostics: [("CDZ0201","bad"),("CDZ0201","bad")]`. The
handler installs at `compile`, the recursive walk emits twice, `collect` surfaces the accumulated
`list<diagnostic>`, and the ask-41 record returns it. Behavior gate 570/0 green on the same toolchain. The
decline is gone. This is the seed-side completion of diagnostics-via-effects; the remaining hop is compiler.cdz
activating its dormant `Diag` handler in `compile`'s body (compiler-agent work, tracked in the SEED-GAPS banner,
NOT a seed gap). Confirmed done.

---
_Original finding (now resolved) below._

## 46. 🔴 A recursive effectful function under the `compile` ENTRY declines — blocks installing the diagnostics handler (effects-in-the-compiler, next hop after ask-45)

**Finding.** ask-45 landed the recursive-effectful runtime-compound path — a `handle` over a recursive
effectful function works under a normal `main`/`run` entry. But the SAME construct under the **`compile`
entry** (the `list<u8> → list<u8>` component seam) declines:
```
declined: recursive effectful function on the compile-entry path not yet emitted
```
So the compile-entry's ABI wrapping does not yet compose with the recursive-effectful lowering that the run
entry just gained. This is the immediate next hop for the operator's "diagnostics via effects" direction: the
compiler's diagnostics handler must be installed at `compile`, and the check pass it handles (`check-node`
walking the `Core` recursively, performing `Diag.emit` at each rejection) is exactly a recursive effectful
function.

**Boundary, isolated (2026-07-07, `compile-run`):**

| shape | result |
|---|---|
| `compile` body = `handle` over a **non-recursive** effectful body | ✅ VALID |
| `compile` body = `handle` over a **recursive** effectful walk (`emit` in a loop) | 🔴 **declines** (above) |
| the **same recursive handle** under a `main`/`run` entry (not `compile`) | ✅ VALID (ask-45) |
| a recursive effectful def with **NO handle anywhere**, `compile` bare | ✅ VALID |
| a recursive effectful def performed under a `handle` **in an unused helper def**, `compile` bare | 🔴 **declines** (same) |
| `handle` over a recursive effectful fn with **SCALAR** state (counter), `compile` entry | 🔴 **declines** (same) |
| `compile` body directly USES `(handle 0 … (recursive-walk))`, scalar state | 🔴 **declines** (same) |

**Not compound-specific.** The decline fires for a recursive effectful `handle` with SCALAR state too (a
`(D.next)→i64` counter), not just `list`/record state — so it is the recursive-effectful `handle` LOWERING
under the compile entry that is unimplemented, the full breadth of ask-45's fix (which covered both scalar and
compound under the RUN entry). Every meaningful internal-state effect in the compiler (diagnostics, symbol
table, return-kind table, fresh-slot counter) threads through the compiler's recursive passes, so ALL of them
hit this — it gates the operator's whole "lean on effects in the compiler" direction, not just diagnostics.

The last row is the sharp one: it is not about `compile` *calling* the handler — **the mere PRESENCE of a
`handle` over a recursive effectful function anywhere in a module whose entry is `compile`** triggers the
decline. (A recursive effectful function with no handler compiles fine; adding the handler is what breaks it.)
So the seed's compile-entry compilation path rejects the recursive-effectful `handle` lowering during
whole-module compilation, independent of reachability from `compile`.

**Minimal repro (declines):**
```
(module m
  (effect D (op emit (-> Int64 Unit)))
  (def (w n) (if (< n 1) 0 (do (D.emit n) (w (- n 1)))))
  (def (helper) (handle (list) ((D.emit (v) s (resume unit (List.push s v)))) (w 3)))
  (def (compile b) b))                 ; compile doesn't even call helper
```
→ `declined: recursive effectful function on the compile-entry path not yet emitted`. Change `(def (compile
b) b)` to `(def (main) …)` (a run entry) and it compiles. Change `helper` to not use a `handle` (bare `w`) and
it compiles.

**Why it matters.** This is the last gap between the operator's effects direction and a working effect-based
diagnostics pass in `compiler.cdz`. The `Diag` effect declaration and the recursive `check-node`/`check-funcs`
rejection pass (performing `Diag.emit 201` at each type error / malformed form) are ALREADY in compiler.cdz and
compile fine (a recursive effectful function with no handler is OK). The one thing that cannot yet be written is
the `handle` that installs the diagnostics collector at `compile` — its presence declines the whole module. So
the diagnostics-via-effects wiring is one seed fix away: emit the recursive-effectful `handle` lowering under
the compile entry exactly as it is emitted under a run entry (ask-45).

**Acceptance signal.** The minimal repro above compiles VALID (and runs `compile`), and compiler.cdz's `compile`
body can be `(handle (list) ((Diag.emit (v) s (resume unit (List.push s v))) (Diag.collect (u) s (resume s s)))
(do (check-funcs …) (compile-program …)))` — self-hosting with the diagnostics collector installed. Then, once
a diagnostics-carrying RETURN channel exists (ask-41 artifact record / ask-42 result<>), `compile` surfaces the
collected `(Diag.collect unit)` alongside the bytes and the ~30 ask-30 rejection cases reach `agree`.

**Compiler-side is PROVEN-READY (2026-07-07).** The `check-node`/`check-arith`/`check-cmp`/`check-funcs` pass in
compiler.cdz is not just written — its diagnostic-counting logic is verified correct in isolation: a standalone
mirror (recurse a Core-like sum tree, `Diag.emit 201` per type error, collect + count) returns exactly the right
count across nested trees (well-typed→0, one bad operand→1, nested-bad→2, total 3). So the ONLY missing piece is
the seed emitting the recursive-effectful `handle` under the compile entry; the compiler-side collection is done
and trustworthy. Activation is the one-line `handle` swap in `compile`, no further compiler work.

**Status.** 🔴 Seed — the compile-entry compilation path's recursive-effectful `handle` lowering (the
run-entry path has it after ask-45). Related: ask-45 (the run-entry recursive-effectful fix this extends to the
compile entry), ask-41/ask-42 (the diagnostics RETURN channel, the hop after this one), ask-30 (the rejections
this reports). Current state: compiler.cdz keeps the `Diag` decl + `check-*` pass (compile fine), `compile`
stays bare-`Bytes` (self-hosts, 27 agree / 0 hard / 0 error); the `handle` is documented in `compile`'s docstring
for one-line activation.

**🔴 LOOP-VERIFIED 2026-07-07 (Run 88) — reproduces on live seed 14:58, entry-kind discriminator confirmed.**
Ran the minimal repro + discriminators: `handle`-over-recursive-effectful under `compile` entry → `declined:
recursive effectful function on the compile-entry path not yet emitted`; the SAME handle under `(def (main) 5)`
→ VALID; the recursive effectful `w` with NO handle under `compile` → VALID. So it is squarely the
recursive-effectful `handle` LOWERING that ask-45 landed for the RUN entry, not yet extended to the COMPILE
entry ABI path — the entry KIND decides. This gates the operator's whole "effects in the compiler" direction
(diagnostics, symbol table, return-kind table — every internal-state effect threads a recursive effectful
handle). Diagnostics collection (ask-45) works; only installing the handler at `compile` is blocked.
