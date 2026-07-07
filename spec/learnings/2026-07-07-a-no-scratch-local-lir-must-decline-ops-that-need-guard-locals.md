# A no-scratch-local Lir must decline the ops that need guard locals — shifts are the honest decline, not a miscompile

*2026-07-07*

**What happened.** The compiler.cdz frontier reached the shift operators `<< >>`, and the spike made a
deliberate, well-reasoned choice to **decline them rather than emit them naively**. The reasoning,
recorded in `compiler.cdz`'s header: wasm's `i64.shl`/`i64.shr_s` **mask the shift count mod 64** and
never trap, so an unguarded lowering would *miscompile* — a shift by 64 silently becomes a shift by 0
(the seed correctly traps `(<< 1 64)`; a naive emit would answer `1`). Faithful shift lowering therefore
needs a **count-range trap guard** (`count >=u 64 → unreachable`) plus a left-shift **overflow guard**,
both of which require **scratch locals** to hold intermediates. But `compiler.cdz`'s Lir is a **pure
Core→Code fold with no scratch-local allocation** — it maps each Core node to a flat instruction
sequence, never allocating a fresh wasm local. So faithful shifts are an *architectural* step (a
local-allocating lower pass), not a one-line binop. Until that pass exists, `<< >>` route through the
reader's unknown-head path to `KError → unreachable` — a **valid trapping component**: decline, don't
miscompile. (By contrast `& |` are *total single opcodes* with no guard, so they landed fully last
cycle.)

**Why.** This is the *correct* face of the reject-don't-miscompile discipline — the exact opposite of
the reader's atom-decode leak from the previous cycle
([[2026-07-07-the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining]]). There,
the reader silently emitted a wrong-but-valid component for a float (read as `false`); here, the backend
recognizes it *cannot faithfully* emit a shift and declines cleanly. The distinction that makes the
choice principled: **an operator whose correct wasm lowering requires a guard (a runtime check that
traps or corrects a hardware misbehavior like count-masking) needs scratch locals, and a fold-only
backend with no local allocation cannot express the guard — so it must decline, because the *only*
alternative is the unguarded (miscompiling) emit.** Emitting `i64.shl` bare is not "partial support," it
is a miscompile (mask-mod-64 turns an out-of-range shift into a wrong in-range one), so decline is the
only honest option a fold-only backend has. This names a concrete architectural requirement for the
compiler's own growth: **a local-allocating lower pass** — the Lir must be able to introduce scratch
wasm locals (and the code section must declare them) — before it can faithfully emit *any* guarded
operation, of which shifts are the first but not the last (checked arithmetic that needs to hold an
operand for an overflow test, a `bin`-segment fit check, etc., are the same shape). The general lesson:
**a backend's IR shape (fold-only vs. local-allocating) bounds which operators it can faithfully emit;
the ones needing guard locals are declined until the IR grows locals, and declining them is correct, not
a gap in coverage** — the coverage gap is the *local-allocating pass*, and the declined operators are
its acceptance list.

**The requirement it drove.** No new corpus case — the *seed's* shift behavior (the mask-mod-64 trap
guard: `(<< 1 64)` traps, both const and runtime) is already fully pinned in `06-numeric-model.sexp`
(the left/right shift-by-width and negative-count cases), and the finding here is about the
*Cadenza-authored compiler's emit path* choosing to decline what its fold-only Lir can't guard, which is
a `compiler.cdz`-internal architecture fact, not a seed behavior the corpus drives. The durable output is
this learning plus a note on **SPEC-BACKLOG item 20** (the emit-coverage checklist): faithful shift
emission — and any guarded operation — is gated on a **local-allocating lower pass** for `compiler.cdz`,
a named architectural step distinct from the per-operator emit work; until it lands, guarded ops
correctly `KError`-decline (the honest frontier), and the acceptance signal is the same as #23's — the
harness's `mine-declines` counting these as clean declines, not the miscompiles the atom-decode path
still produces. The two `KError` uses now in the compiler — an unknown operator head, and a guarded op
the fold-only Lir can't emit — are both the correct decline-don't-miscompile behavior; the reader's
atom-decode fall-through (item 23) is the one place that still leaks, and fixing it is routing those to
the same `KError` these already use.
