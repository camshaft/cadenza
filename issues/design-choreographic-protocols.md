# Vertical-ready brief — choreographic protocols

**Design doc (authority):** `implementation/design/DESIGN-choreographic-protocols.md` (landed on trunk).
**Subsystem / area:** `compiler-ml` (projection is Cadenza code — a compile-time `Ast → Ast` fold — with
`rcdzc` seams for the `Comm` effect declaration + the `choreography` reader rule).
**Suggested owner:** a `vertical` agent (area=compiler-ml), minted by the PM.

## What to build (operator-ruled paradigm: FULL CHOREOGRAPHIC PROGRAMMING, option (b))
Define a distributed protocol as **one global program** with **located values** (`x@Role`) and **explicit
communications** (`e@R ~> S`, selections `Label@R ~> S`); the compiler **endpoint-projects** it to
**GENERATE each actor's executable code** — the author writes NO per-endpoint code, there is literally one
artifact. Communications are an **effect** (`Comm.send`/`Comm.recv`, host-bound), so a projected actor's
capability row *is* its protocol alphabet. The Cadenza differentiator: the projection is a metaprogram
(built-in `Ast` + one-tier evaluator — already exercised by every `compiler-ml/src/*.cdz` fold), and its
**soundness + deadlock-freedom can be machine-checked by the HOL-Light kernel**.

## First increment (start here)
**Inc 0 — the choreography AST + parser.** A `choreography` surface form (located values `x@R`, comms
`e@R ~> S`, selections `Label@R ~> S`, `let`, `if`-at-role, `rec`) parsed into a built-in-`Ast`-shaped
value. Gate: round-trip parse/print of the doc's `Purchase` choreography; a well-formedness checker
rejects malformed input (a comm whose source/target roles are undeclared).

Then Inc 1 (projectability / knowledge-of-choice check + reject diagnostic), Inc 2 (projection to per-role
programs — the core `project : Ast -> Role -> Ast` fold + the `⊓` merge, §3.1 of the doc), Inc 3 (code-gen
+ end-to-end execution over a mock `Comm` handler), Inc 4 (deadlock-freedom), Inc 5 (**flagship: model the
FLEET's own coordination as the choreography** — §6.1), Inc 6 (kernel-checked projection soundness).

## Coordinate with (design references)
v-metaprogramming (projection = compile-time `Ast→Ast`), v-effects (`Comm` as an effect / capability),
v-inference (located types + local-type rows), v-verification (Inc 6 projection-soundness proof).

## Notes
- The projection algorithm (`project` + the `⊓` merge operator, the knowledge-of-choice rule) is spelled
  out concretely in the doc's §3.1 — build to that.
- §2 of the doc grounds every load-bearing claim in the spec (metaprogramming.md, type-system.md,
  capabilities-and-effects.md) and notes that the fold shape is already proven by the self-host corpus.
- Alternatives (MPST type-level, hybrid) are recorded in §4.1 but NOT the chosen surface — build (b).
