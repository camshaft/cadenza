## 66. ✅ FIXED + VALIDATED 2026-07-08 (05:27 stable) — a `let`-bound variable is reported "unbound name" when referenced inside an `and`/`or`/`not` connective

**✅ RESOLVED — the seed agent fixed the connective-desugar scope; validated by the conformance loop 2026-07-08
(05:27 republish).** The minimal reproducer `(module m (def (f k) (let ((x k)) (and (> x 0) (< x 9)))) (def
(main) (if (f 3) 1 0)))` now runs to `Value("1")`; the full sharpened probe set (let-var in `and`/`or`/`NOT`,
nested-let inner var) all emit VALID; **compiler.cdz self-compiles VALID again (259633 bytes)**; and the
value-harness returned from 0 agree / 145 error → **73 agree / 0 hard / 0 error**. The `desugar_connective`
path now threads the enclosing `let` scope. MOVE TO done/. Original report follows.

---

## 66. 🔴🔴 CONFIRMED SEED REGRESSION (SELF-HOST BLOCKER) — a `let`-bound variable is reported "unbound name" when referenced inside an `and`/`or`/`not` connective

**⏳ STILL UNFIXED (cycle 3 re-probe — stable unchanged since 05:11; reproducer still `declined: unbound name:
x`; compiler.cdz still `unbound name: roff`). ✅ NOW GATE-ENFORCED: the behavior gate carries a pinned corpus
case "a let-bound variable is in scope inside a boolean connective operand" (a FAIL until the fix lands) —
so the seed agent sees this directly. Cycle-3 also CONFIRMED THE REGRESSION IS NARROW: let-vars work fine in
arithmetic / fn-calls / `if` / nested-let / `match` / tuple — ONLY the `and`/`or`/`not` connective-desugar path
drops the `let` scope. (The 2nd behavior-gate FAIL, "two definitions of the same name is rejected", is a
SEPARATE, already-tracked native gap — SEED-GAPS ~L1093, duplicate-def first-wins — NOT part of ask-66.)

**SHARPENED root cause (2026-07-08, cycle 2):** the scope loss
affects the WHOLE connective-desugar path — `and`, `or`, AND `not` (unary) — and fires whether the connective
IS the let body or is NESTED inside it (e.g. in an `if`-condition). Crucially, a connective whose operands use
ONLY params/consts works fine even inside a `let` — it is specifically a `let`-BOUND variable that becomes
unbound inside a connective. So the desugar keeps the PARAM env but DROPS the `let` extension. New probes:
| construct | result |
|---|---|
| `(let ((x k)) (not (> x 0)))` — let-var in `not` (unary) | ❌ unbound: x |
| `(let ((x k)) (if (and (> x 0) true) x 0))` — connective NESTED in if-cond | ❌ unbound: x |
| `(let ((x k)) (not (> k 0)))` — `not` over a PARAM, let present but unused in connective | ✅ VALID |
| `(let ((x k)) (and (> k 0) (< k 9)))` — `and` over PARAMs only, let unused | ✅ VALID |
| `(let ((y (+ k 1))) (let ((x y)) (and (> x 0) (< x 9))))` — nested lets, inner let-var | ❌ unbound: x |
Fix must thread the FULL lexical env (base/params + ALL enclosing `let` slots) into connective desugaring for
`and`/`or`/`not` — not just the base+param env.

**Severity: 🔴🔴 blocking. Introduced by the 2026-07-08 05:03 stable republish.** Every historical
compiler.cdz backup — valid on its contemporaneous stable — now FAILS to self-compile on the 05:03 seed with
`declined: unbound name: <var>`. This is a genuine regression in the seed's name resolver / `and`/`or`
desugaring, NOT a compiler.cdz defect (compiler.cdz is byte-unchanged from the 74-agree state).

**Minimal reproducer (saved `/tmp/let-and-scope-repro.cdz`) — DECLINES with "unbound name: x":**
```
(module m
  (def (f k) (let ((x k)) (and (> x 0) (< x 9))))
  (def (main) (if (f 3) 1 0)))
```
`emit` → `declined: unbound name: x`. Should compile and `(f 3)` → true → `main` → 1.

**Precisely characterized (each row a one-line probe):**
| construct | result |
|---|---|
| `(let ((x k)) (and (> x 0) (< x 9)))` — let-var in BOTH `and` operands | ❌ unbound: x |
| `(let ((x k)) (and (> x 0) true))` — let-var in FIRST operand only | ❌ unbound: x |
| `(let ((x k)) (and true (< x 9)))` — let-var in SECOND operand only | ❌ unbound: x |
| `(let ((x k)) (or (> x 0) (< x 9)))` — `or` instead of `and` | ❌ unbound: x |
| `(let ((x k)) (> x 0))` — let-var in a plain comparison (NO and/or) | ✅ VALID |
| `(let ((idx (+ k 1))) (if (< idx 9) idx 0))` — let-var in a plain `if` | ✅ VALID |
| `(def (f x) (and (> x 0) (< x 9)))` — a PARAMETER (not let) in `and` | ✅ VALID |

**Root cause (for the seed agent):** the `and`/`or` desugaring resolves its operands in an environment that
is MISSING the enclosing `let`'s bound locals. A PARAMETER survives (params are in the base env), but a
`let`-introduced binding is dropped when the body is (or contains) an `and`/`or`. So `and`/`or` desugars
BEFORE (or without threading) the `let` scope extension. Likely the connective-desugar path (`desugar_connective`,
which rewrites `(and a b)` → `(if a b false)`) runs on a pre-`let`-scope AST, or re-resolves operands without
the let env. Cross-check: the fix must keep the `let`-bound slots in scope through the connective rewrite —
desugar `and`/`or` as an ordinary `if` node UNDER the same environment the `let` body sees, not in the base env.

**Why it's a self-host blocker.** compiler.cdz's `dup-scan-outer` (and other helpers) use exactly this idiom:
`(let ((idx (entry-name-index b i k))) (if (and (>= idx 0) (dup-scan-inner b i (+ k 1) end idx)) true …))`.
So compiler.cdz — and EVERY historical backup — no longer self-compiles: `declined: unbound name: idx`.
value-harness went 74 agree → **0 agree / 145 error** purely from this seed change (the harness's injected
`main` can't be built because the whole compiler.cdz rejects). The compiler.cdz source is CORRECT and unchanged.

**Acceptance signal.** The reproducer compiles and runs to `1`. Then compiler.cdz self-compiles VALID again and
the value-harness returns to ≥74 agree / 0 hard / 0 error on the fixed seed.

**⚠ NOT WORKING AROUND IT.** Per the standing discipline ("block on a seed miscompile/regression, don't
contort the implementation"): I am NOT rewriting compiler.cdz's `(let … (and …))` idioms into nested `if`s to
appease the broken seed. compiler.cdz stays at the 74-agree state (`/tmp/compiler-let-passthru.cdz`). No new
compiler.cdz functionality this cycle — blocked on this fix, will resume once it lands.

**Status.** 🔴🔴 CONFIRMED, BLOCKING, seed-side. Reproducer minimized + saved. Bisected to compiler.cdz def
`dup-scan-outer` (#69) as the first trigger, then reduced to the 3-line repro above. Related: the boolean-
connectives work [[boolean-connectives-gap-2026-07-06]] (`desugar_connective`), which this regresses.
