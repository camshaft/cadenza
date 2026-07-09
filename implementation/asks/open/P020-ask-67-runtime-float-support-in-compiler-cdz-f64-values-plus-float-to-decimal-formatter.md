## 67. 🟡 (compiler.cdz frontier, NOT a seed gap) Runtime FLOAT support: f64 as a runtime value kind + a byte-identical float→decimal display formatter

**What.** compiler.cdz currently has NO runtime-float support: `NFloat` resolves to a check-only `KError`
(const float-EQ folds — `(= 3.5 3.5)`→true — but any float that must exist at RUNTIME declines/traps). So a
bare float result and every float-through-a-function case declines:
- `(module m (def (main) 3.5))` → mine traps; native = 96-byte component → `Value("3.5")`.
- `(module m (def (f x) x) (def (main) (f 3.5)))` → mine traps; native = 107-byte → `Value("3.5")`.

**Decoded native shape (2026-07-08).** A runtime float is a SCALAR-tier value (NOT heap): the component uses
**f64 valtypes (0x7C)** for params/locals/return, exports plain `run`, imports NO heap ops (no box-float/make/
display). A float threads through the calling convention as a raw f64 like an i64 threads as i64. So the
plumbing is a scalar-tier extension: add an `f64` Kind, `f64.const`/`f64.*` lowering, f64 in `functype-of`/
locals/`run` framing.

**⚠ THE HARD PART — the display.** Native does NOT bake an ASCII string; it embeds the raw f64 const and a
RUNTIME float→decimal formatter (verified: `3.5` vs `2.5` components differ in exactly 1 byte = the f64 const;
"3.5"/"2.5" ASCII absent from the bytes). To be BYTE-IDENTICAL, compiler.cdz must emit that formatter, matching
native's exact output:
- `3.5`→`"3.5"`, `0.1`→`"0.1"`, `100.0`→`"100.0"`, `-0.0`→`"-0.0"`, `1e19`→`"10000000000000000000.0"` (full
  decimal expansion, no exponent).
This is a shortest-round-trip float formatter (Ryū/Grisu-class) — a substantial, CORRECTNESS-CRITICAL algorithm.
Getting it wrong is a `hard` miscompile, so it is NOT a safe incremental drop-in.

**Scope / staging (for a dedicated cycle, like the tier-2 heap build):**
1. De-risk in Python FIRST: decode the exact fixed float-`run` envelope (the 96-byte recipe minus the f64 const)
   and the baked formatter routine's wasm; prove byte-identical reassembly for several floats.
2. Add the `f64` Kind + `f64.const`/framing so a bare `(def (main) <floatlit>)` emits the const half.
3. Transcribe/generate the formatter to match native byte-for-byte (the big, risky piece — verify against
   `3.5`/`0.1`/`1e19`/`-0.0` before shipping).
4. Then f64 params/args/locals (float threaded through calls), then float arithmetic/comparison at runtime.

**Corpus payoff.** Unlocks the float-result and float-param declines (`(f 3.5)`, `(String.scalar-len (if b …))`
is unrelated; the direct float cases) — a modest count, but floats are a language primitive the self-hosted
compiler will eventually need. Lower priority than tier-2 heap (more corpus cases there) but well-scoped.

**Status.** 🟡 compiler.cdz-frontier feature, decoded + scoped, NOT started. NOT a seed gap — nothing for the
compiler agent; this is the loop's own next big build (a dedicated cycle). Deferred from the cycle that
identified it because a rushed float formatter risks a miscompile. Related: ask-60 (heap tier-2, the other big
scalar/heap investment), [[float-render-saturates-and-gate-blindspot]] (float rendering is a known blindspot).
