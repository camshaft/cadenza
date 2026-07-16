# Design sketch: shared `Ex` + end-to-end driver for the compiler-ml port (prep for option B)

**Author:** v-compiler-ml · **Status:** design prep (concierge leans (B); awaiting operator A/B/C ruling)
**Why now:** the port is ~71 modules each re-declaring its OWN expression type; they never connect, so
it's a pipeline on paper, not a running self-hosted compiler. A shared `Ex` is the single biggest blocker
to a `parse → infer → optimize → codegen` driver. Sketching it de-risks (B) and is useful under any option.

## Current state (surveyed tick 118/120)

The transform/analysis modules cluster around one core shape with small divergences:

| module | expr type | variants |
|---|---|---|
| constprop, deadlet, inline, optimize, interp | `Ex`/`Expr` | `Num(Int64) · Var(String) · Add · Mul · Let(String,_,_)` |
| cfold | `Ex` | `Num · Add · Mul` (no Var/Let) |
| strength | `Ex` | `Num · Var(Int64) · Add · Sub · Mul · Shl(_,Int64)` |
| anf | `Ex` | `Num · Var(Int64) · Add · Mul · Let(Int64,_,_)` |
| infer/infer-let/closure | `Ex` | `Lit · Var(Int64) · Lam · App · Let(Int64,_,_)` (lambda calc) |
| parse/interp/label | `Expr` | `Lit · Bin(op,_,_)` (the parser's real tree) |

Two clusters: an **arithmetic-let** language (constprop/deadlet/inline/optimize/cfold/strength/anf) and a
**lambda calculus** (infer/closure). The `Var` key type differs (`String` vs `Int64`) — the main friction.

## Proposed shared core

`implementation/compiler-ml/src/core-ex.cdz` — ONE canonical expression, imported like `ast.cdz` is today
(confirmed: `import { Ast } from "ast"` + using `Ast.Int(..)` cross-file already works, so `import { Ex }
from "core-ex"` + `Ex.Add(..)` is viable):

```
type Ex =
  | Num(Int64)
  | Var(Int64)            // ids, not strings — cheaper, and infer/anf/strength already use Int64
  | Bin(Int64, Ex, Ex)    // op code (43=+, 42=*, 45=-) subsumes Add/Mul/Sub
  | Let(Int64, Ex, Ex)
  | Lam(Int64, Ex)        // for the lambda-calc passes
  | App(Ex, Ex)
```

Migration is INCREMENTAL and low-risk:
1. Land `core-ex.cdz` (type + a few shared helpers: `op-of`, builders, a pretty-printer) — additive, no
   churn to existing modules.
2. Port ONE arithmetic pass (constprop is the cleanest) to `import { Ex } from "core-ex"` — proves the
   import + constructor-use path end-to-end, keeps its @tests green.
3. Port the rest of the arithmetic cluster one module per tick (each its own gated MR).
4. Reconcile `Var(String)` users: either a name→id pass up front, or keep `Var(Int64)` and thread a symbol
   table (type-env already exists).
5. THEN the driver: `def compile(src) = codegen(optimize(infer-annotate(parse(src))))` over the shared `Ex`.

## Integration bugs to expect (the real stress-test payoff, per concierge)

Connecting passes that only ran in isolation will surface: (a) op-code agreement across passes (the parser
uses 43/42; strength/codegen must agree); (b) `Var` id freshness collisions between a rename pass and a
later fresh-var pass (anf already seeds at `max-id+1` — that discipline must be global); (c) a pass
assuming a normal form an earlier pass didn't establish (e.g. codegen assuming ANF). Each is a genuine
finding to REPORT, not work around.

## Blocked-on / next

- **Operator A/B/C ruling** (concierge routed, strong lean B). If B: start at step 1 above.
- Meanwhile mode (C): no new modules; Copilot-review/repro/regression-pinning only. Trunk was mid a
  fleet-red revert (5a8e262ae recursive-sum regression → compiler-ml 8/864) at tick 118 — verify green
  before any driver MR.

## Step-1 de-risking spike (tick 130, v-compiler-ml — throwaway, verified then removed)

Built a local `core-ex.cdz`-shaped module + a consumer importing it; ran via `cdz test`, then deleted
(no MR — mode C). Confirmed the migration mechanics AND surfaced one concrete gotcha:

- **`export { Ex }` (type handle only) is NOT enough** — a consumer that constructs/matches `Ex.Bin(..)`
  cross-file gets **CDZ0214** ("constructor `Bin` is not exported ... `Ex`'s handle is visible but `Bin`
  is withheld"). The shared module MUST `export { Ex.* }` to make every constructor public. (This is why
  `ast.cdz` — the working model — exports the type such that `Ast.Int(..)` etc. work cross-file.)
- With `export { Ex.* }`, a consumer imports `{ Ex, size, op-of }` and freely builds/matches
  `Ex.Bin(43, Ex.Num(1), ...)` — verified 2/2 @tests. **Step 1 (land `core-ex.cdz`) is proven.**

So when the operator picks (B): `core-ex.cdz` MUST `export { Ex.* }` (plus helpers). No other blocker found
for step 1. The `Var(String)` vs `Var(Int64)` reconciliation (step 4) remains the real design work.
