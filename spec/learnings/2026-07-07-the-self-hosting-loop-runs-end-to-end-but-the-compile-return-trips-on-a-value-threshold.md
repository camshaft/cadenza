# The self-hosting loop runs end-to-end — and the compile-return alignment bug has a sharp, value-dependent threshold the handoff doc missed

*2026-07-07*

**What happened.** `compiler.cdz`'s entry was rewired from the nullary `main` (with a hardcoded target program's
bytes in its body) to `(def (compile b) (compile-bytes b))` — the real self-hosting seam, built by the seed as a
`cadenza:compiler/compile : func(list<u8>) -> list<u8>` component (gap 3l's build path). This is the pending
step 1 from the earlier `bytes → bytes` learning
([[a-bytes-to-bytes-compile-entry-unblocks-the-real-differential-harness]]), now landed. Probing the loop
end-to-end — `cadenza-seed compile-run <compiler.cdz> <input.cdz>` — the self-hosted compiler **compiles a real
program**: `(module m (def (main) 42))` → the correct **89-byte component** (`\0asm` header, `i64.const 42`),
built by the full `read-module → resolve-module → fold → lower → serialize → frame` pipeline. The compiler is
now a genuine byte-transform, not a hand-fed stub.

But the return path is unreliable — the seed's `compile`-return wrapper trips *"return pointer not aligned"* for
many inputs (SEED-GAPS gap 3n). The handoff doc characterized this as "INPUT/ALLOCATION-DEPENDENT, not a clean
size threshold," with a fixed-output repro `(def (compile b) (Bytes.of (list 0 0 0 0)))` that "fails at EVERY
size 4..128." Probing the seed **as it is now** (rebuilt since the doc was written) corrected that on two
counts:

1. **The fixed-output path is now FIXED.** `(def (compile b) (Bytes.of (list 0 0 0 0)))`, `(list 1 2 3 4)`,
   sizes 0–4, and the identity `(def (compile b) b)` all return cleanly today — the doc's own repro no longer
   reproduces. So a partial fix landed; the doc's 3n section is stale.
2. **The real compiler's failure is a SHARP, DETERMINISTIC VALUE THRESHOLD, not "allocation-dependent."**
   Compiling `(module m (def (main) N))` for a bare integer `N`: **`N ≤ 23` → "not aligned", `N ≥ 24` → OK**,
   bisected exactly. Every output is 89 bytes — identical size on both sides of the boundary; the values differ
   only in the one `i64.const` operand byte. `0`, `1`, `true`, `256`, and an unfolded `(if (< 3 5) 42 99)` all
   fail; `42`, `(< 3 5)`, `(dbl 21)`, and the depth-2/3 Bool chains all succeed. So the doc's implication that
   `42`-class inputs are safe is right, but the corollary that the *simplest* inputs are safe is exactly wrong —
   `0` and `1` fail, and they are the minimal reproducer.

The minimal pair is `(main) 24` (OK) vs `(main) 23` (fails) — same 89-byte output, single-byte operand
difference, fully deterministic across repeated runs.

**Why.** The bug is not in the compiler (`compiler.cdz` compiles all these `emit`s byte-identically to native —
verified) and not in the wrapper's static marshalling (the fixed-output path is aligned now). It is in how the
seed marshals a **computed** `list<u8>` return whose bytes live in a runtime `Bytes` rope: the return pointer's
alignment depends on where that rope's flattened buffer lands in linear memory relative to the retarea, and that
landing position is a function of the bump allocator's state during `compile-bytes` — which, for these tiny
programs, correlates with the operand value in a way that crosses an alignment boundary right at 24. The value
threshold is almost certainly a proxy: some internal allocation count (nodes folded, rope leaves, a scratch
buffer sized by the value) shifts the bump pointer by a few bytes as `N` grows, and 24 is where it happens to
become 4/8-aligned. The lesson worth keeping is the same one this loop keeps relearning from a different angle:
**the handoff doc's characterization of an open bug is an aggregate to re-probe, not a fact to inherit** —
"allocation-dependent, fails at every size" became, on a direct bisect against the current seed, "fixed-output
now works; the real compiler fails deterministically for N ≤ 23," which is a completely different and far more
actionable shape. A stale "fails at every size" would send a fix at the wrapper; the value-threshold points at
the rope-flatten-to-retarea offset instead.

**The requirement it drove.** No corpus case — this is a seed component-ABI defect (the `compile`-return
marshalling), not a language behavior with a value oracle, and it lives entirely in the seed's hand-emitted
return wrapper. The durable outputs are this learning and a dedicated SPEC-BACKLOG item (gap 3n, sharpened): the
`compile` export's computed `list<u8>` return must be robustly aligned regardless of the returned rope's heap
position, with the minimal reproducer `(module m (def (main) 23))` fails / `(main) 24` succeeds (89-byte output
both, deterministic) replacing the doc's stale fixed-output repro. Until it lands, `component-check` cannot grade
the corpus (it fails even the `42` case where native passes — the bug is the seed's return wrapper, not the
compiler) and `compile-run` is reliable only for `N ≥ 24`-class inputs, so the compiler's test loop stays the
interim value-first `emit`-based harness (which runs the emitted component via `run()`, sidestepping the
compile-return path entirely). The strategic status is worth stating plainly: **the self-hosting loop is
functionally closed — a Cadenza-authored compiler compiles a program to a correct component through the real
seam — and the last thing between here and a byte-level self-hosting gate is one seed-side alignment bug in the
return marshalling, not any missing compiler capability.**
