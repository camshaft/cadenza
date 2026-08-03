# Audit: compiler-ml `-1`/sentinel sites — DOMAIN-VALUE (keep) vs SENTINEL (→ Option/Result)

Operator steer (2026-08-01): *"most of these places [the `(0-1)` fixes] need to be using options and not
sentinel values… I really hope it's not getting distracted."* SHARP + RIGHT: `(0-1)→-1` (Problem-A) makes a
sentinel cleaner-WRITTEN but it's STILL a sentinel. Goal = ELIMINATE sentinels (Problem-B → Option/Result/trap
per the immutable-Rust-port rule). This audit classifies every `-1`/sentinel site so Problem-B leads.

## Verdict: the operator's hypothesis is CORRECT — MOST `-1` sites are sentinels needing Option, NOT domain values.

## SENTINELS → must become Option/Result/sum/trap (Problem-B, the real work) — the MAJORITY
| Family | ~sites | Meaning of `-1`/magic | Fix (per immutable-Rust rule) | Slice |
|---|---|---|---|---|
| `argId = -1` in `NApp(callee, argId)` | ~50 (all pipeline stages) | "nullary call, NO argument" | `NApp(callee, Option(NodeId))` — absence is a legit typed None | **B1** (highest ripple: Node arena field threaded parse→resolve→infer→lower→eval + emit-boundary) |
| `op-code-of`/`op-code` → -1 | ~22 | "not an operator" (lookup miss) | `Option(Int64)` (None = not-an-op) | **B5** (easiest, turnkey — designed) |
| `bodyId = -1` (read-def-body) | ~7 | "unsupported signature → decline" | `Option` (out-of-subset is an expected checked outcome) | **B3** |
| `paramId/param2/3/4 = -1` | ~16 (sread) | "no param at this arity / not present" | `Option(NodeId)` per slot | **B2** |
| `binderId = -1` (ctor-pattern) | few | "wildcard `_` / no binder" | `Option(NodeId)` or a `PBind|PWild` variant | **B2** |
| argType `enc = -1` | ~3 | "unsupported payload type" | `PayloadTy = TyInt|TyBool|TyUnsupported|TyNone` sum | **B4** (after gap-B-C — DONE, so ready) |
| emit-db `index-of`/`func-index`/`byte-of` → -1 | ~5 | "not found / index out of range" | `index-of`/`func-index` → `Option` (legit miss); `byte-of` OOR → `Option` or trap (a compiler-emitted index SHOULD be in range → arguably trap = should-never-happen) | **B7** |
| `ty.int-parts` `SignDef/WidthDef => -1` | ~3 | "sign/width not fixed" (test helper) | `Option`, or leave (test-only helper, low value) | B-minor |

## GENUINE DOMAIN VALUES → keep `-1` (the MINORITY) — these are NOT sentinels
| Site | Why it's a real value |
|---|---|
| eval-db `b == -1` (div/mod guard, :74? / :217) | `-1` is the actual DIVISOR — `Int64.min % -1` / `x / -1` special-case (rcdzc parity). The value -1 IS the domain input, not a stand-in for absence. KEEP. |
| int-width test data (`-1`/`-3`/`-50`/`-56`/`-128`/`-129`) | real negative operands to fits/wrap/overflow/checked-sub — domain values under test. KEEP (S-A4 de-obfuscation was correct here). |
| eval-db `CNum(-1, …)` (:803) | a real -1 Core literal being emitted (a match-else default value). KEEP. |
| sread `NBin(-1, …)` poison-op (:831) | ⚠ BORDERLINE — `-1` as an op-code here IS the "not-a-real-op" sentinel feeding TErr. Arguably B5-adjacent (once op-code is Option, this poison path changes). Flag with B5. |

## Re-prioritization (per the steer — LEAD WITH PROBLEM-B)
1. Problem-A (`0-1`→`-1`) is DONE for the domain-value sites (S-A1..S-A4 landed) — those were legit
   de-obfuscation. STOP doing Problem-A on sentinel sites (spelling a sentinel clearer ≠ progress).
2. **S-A5 (sread) is now RE-SCOPED:** most of its ~34 `-1` sites are SENTINELS (bodyId/paramId/argId/enc) =
   B2/B3/B5 territory, NOT cosmetic. Do NOT land S-A5 as a cosmetic `0-1`→`-1` pass — fold those sites into the
   Problem-B slices (Option-ize them) instead. Only the genuine-domain `-1` in sread (the NBin poison, borderline)
   is cosmetic, and it's B5-adjacent. ⟹ **DROP S-A5 as a standalone cosmetic slice.**
3. LEAD with Problem-B, easiest→hardest: **B5 op-code→Option** (turnkey) → B7 emit-db lookups→Option → B3
   bodyId→Option → B2 param/binder→Option → B4 payload-ty→sum → **B1 argId→Option** (biggest, last) → B-MARKER
   read-source→ReadResult. Each gated + emit-co-verify where it reaches lower/emit.

## Report to operator (via concierge)
Split: of the `-1`/sentinel sites, the VAST MAJORITY are sentinels needing Option (argId ~50, op-code ~22,
sig-readers ~23, argType ~3, emit lookups ~5 = ~100+ sentinel sites) vs a SMALL set of genuine domain values
(the div/mod -1 guard, int-width test data, CNum literal — ~a dozen). The operator's hypothesis holds. Problem-A
only correctly applied to the domain-value minority; the sentinel majority is Problem-B and now leads.
