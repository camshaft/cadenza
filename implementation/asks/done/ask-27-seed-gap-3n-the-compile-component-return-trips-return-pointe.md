## 27. 🟢 Seed gap 3n: the `compile`-component RETURN trips "return pointer not aligned" — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by the loop.** The `(p+3)&!3` retarea-alignment fix landed (a
seed rebuild). Re-probing every input that failed last cycle — `(main) 5`/`0`/`1`/`true` (input len 31), `1000`
(33), `(mmm)…42` (34), `if->42` — **all now return `Ok`**, across all mod-4 residues. The self-hosting
`compile-run` loop works for ARBITRARY programs; `compiler.cdz` is byte-identical to native on `(main) 42`/
`(< 3 5)`/depth-2 Bool chain, `soft` on `(+ 20 22)`/`(dbl 21)`. **Follow-on to reach the byte gate is item #28.**
Learning: `spec/learnings/2026-07-07-gap-3n-fixed-the-self-hosting-loop-is-operational-and-the-byte-gate-is-one-step-away.md`.
(Original finding below.)

**🔎 ROOT CAUSE (2026-07-07) — INPUT-LENGTH mod 4, converged with the compiler agent.** The failure is a
deterministic function of the input AST byte length mod 4: `input_len % 4 == 0` → OK, otherwise *"not aligned"*.
(The loop first read this as parity from an under-sampled table — all its even cases were also ≡ 0 mod 4 — then a
len ≡ 2 probe (`(module mmm (def (main) 42))`, AST len 34, FAILS) and a cross-check against the agent's own
`SEED-GAPS` note settled it as mod 4. The agent independently reached the same diagnosis and fix.) Progression of
proxies: "fails at every size" → "value threshold at 24" → "parity" → **mod 4**; 24 was a proxy because it is the
CBOR 1→2-byte int boundary that flips input AST length 31→32.

| input | input-AST bytes | mod 4 | result |
|---|---|---|---|
| `(main) 5` / `23` / `true` | 31 | 3 | FAIL |
| `(main) 42` | 32 | 0 | OK |
| `(main) 1000` | 33 | 1 | FAIL |
| `(mmm) (main) 42` | 34 | 2 | FAIL |
| `(main) (+ 2 3)` / `(< 3 5)` | 36 | 0 | OK |
| `(id 5)(def (id x) x)` | 48 | 0 | OK |

**The bug (agent's diagnosis, loop-confirmed):** the `compile` core wrapper copies the input `list<u8>` into
linear memory at the bump pointer, then allocates the RETURN area (the canonical-ABI `retptr`, which must be
4-aligned) at `bump_ptr` WITHOUT re-aligning — so `retptr = base + input_len`, 4-aligned only when
`input_len % 4 == 0`. **The fix:** align the bump pointer up to 4 (`(p + 3) & !3`) before allocating the return
area, or place the retarea at a fixed aligned offset independent of input length. Minimal repro: `(module m
(def (main) 5))` (31 B, FAIL) vs `(module m (def (main) 42))` (32 B, OK), `cadenza-seed compile-run
<compiler.cdz> <it>`.

---

**Status.** The self-hosting loop is functionally CLOSED: `compiler.cdz`'s entry is now `(def (compile b)
(compile-bytes b))` (gap 3l build path), and `cadenza-seed compile-run <compiler.cdz> <input.cdz>` compiles
`(module m (def (main) 42))` → the correct **89-byte component** through the full pipeline. The ONLY blocker to
adopting `component-check` as a byte-level gate is the seed's `compile`-RETURN marshalling: it trips *"running
the compiled compiler: return pointer not aligned"* for many inputs.

**Status.** The self-hosting loop is functionally CLOSED: `compiler.cdz`'s entry is now `(def (compile b)
(compile-bytes b))` (gap 3l build path), and `cadenza-seed compile-run <compiler.cdz> <input.cdz>` compiles
`(module m (def (main) 42))` → the correct **89-byte component** through the full pipeline. The ONLY blocker to
adopting `component-check` as a byte-level gate is the seed's `compile`-RETURN marshalling: it trips *"running
the compiled compiler: return pointer not aligned"* for many inputs.

**Corrected characterization (probed against the CURRENT seed 2026-07-07 — the SEED-GAPS 3n note is stale).**
1. **The fixed-output repro the doc cites is now FIXED.** `(def (compile b) (Bytes.of (list 0 0 0 0)))` (and
   sizes 0–4, and the identity `(def (compile b) b)`) all return cleanly today — a partial fix landed. The doc's
   "fails at EVERY size 4..128" no longer reproduces.
2. **The real compiler's failure is a SHARP DETERMINISTIC VALUE THRESHOLD, not "allocation-dependent."**
   Compiling `(module m (def (main) N))` for a bare integer `N`: **N ≤ 23 → "not aligned", N ≥ 24 → OK**
   (bisected exactly). Both sides emit an **identical 89-byte** component — same size, differing only in the one
   `i64.const` operand byte. `0`/`1`/`true`/`256`/unfolded-`if` fail; `42`/`(< 3 5)`/`(dbl 21)`/depth-2/3 Bool
   chains succeed. So the SIMPLEST inputs (`0`, `1`) are the minimal reproducer — opposite the doc's implication
   that `42`-class inputs being safe means trivial ones are.

**Minimal reproducer:** `(module m (def (main) 23))` fails, `(module m (def (main) 24))` succeeds — both 89-byte
output, deterministic across runs. `cadenza-seed compile-run <compiler.cdz> <that-input>`.

**Root (to confirm).** Not the compiler (`compiler.cdz` `emit`s all these byte-identically to native) and not
the wrapper's static marshalling (fixed-output is aligned now). It is the seed's marshalling of a **computed**
`list<u8>` whose bytes live in a runtime `Bytes` ROPE: the return pointer's alignment depends on where the
flattened rope buffer lands in linear memory relative to the retarea, and that offset is a function of the bump
allocator's state during `compile-bytes` — which for tiny programs correlates with the operand value, crossing
an alignment boundary at 24. The value threshold is a proxy for an internal allocation count shifting the bump
pointer. **Agent action:** make the `compile` export's computed `list<u8>` return robustly 4/8-aligned
regardless of the returned rope's heap position (align the retarea/return pointer independent of allocator
state), then `component-check` can grade the corpus.

**Consequence.** `component-check` cannot be adopted yet — it fails even the `42` case where native cdz-rustc
passes, confirming the bug is the seed's return wrapper, not the ABI. The compiler's test loop stays the interim
value-first `emit`-based harness (runs the emitted component via `run()`, sidestepping the compile-return path).
Related: #22 (gap 3l, resolved). Learning:
`spec/learnings/2026-07-07-the-self-hosting-loop-runs-end-to-end-but-the-compile-return-trips-on-a-value-threshold.md`.

---
