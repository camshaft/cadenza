# The component's result valtype is type-directed — through an exhaustively-matched Kind sum, the same discipline as the instruction sum

*2026-07-06*

**What happened.** The compiler-in-Cadenza spike grew a second observable result type: alongside the
Int64 arithmetic it already emitted (`+ - * / %`), it added the comparisons `<` and `=`, whose result
type is **Bool**, not Int64. That forced a decision the arithmetic-only pipeline never had to make:
the component's `run` export must present its result at the correct wasm boundary valtype — an Int64
result crosses as the component-model `s64` (core `i64` = `0x7E`, component `0x78`), a Bool result as
`bool` (core `i32` = `0x7F`, component `0x7F`) — so the framing has to *know* a program's result
type. The spike solved it the same way it structures every other stage: a small type-directed pass,
`kind-of : Core → Kind`, where `Kind` is a **sum type** (`Ki64 | KBool`) matched exhaustively by both
the pass and the two valtype maps (`core-valtype`, `comp-valtype`). The result kind is read once from
the folded Core and threaded to both framing sites (the embedded core module's function type and the
component's exported type). A comparison that constant-folds still frames as Bool, because the fold
produces a `KBoolC` leaf that carries the kind and `kind-of` reads it back. Two related design moves
landed in the same rework: the resolved-IR head dispatch moved from an **integer opcode** to a `Prim`
**sum variant** matched exhaustively (completing "no string/integer tag dispatch" at the *surface*
boundary, not only the backend), and the Core sum gained `KLt`/`KEq`/`KBoolC` so a comparison is a
distinct constructor from the start.

**Why.** The lesson is that **the boundary valtype is a function of the program's result type, and
selecting it deserves the same reject-don't-miscompile discipline as instruction selection**
([[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]]). Had `kind-of`
returned an integer tag or a boolean "is it a bool?", adding a third result kind (a Float64 result, a
compound result) would silently fall through to a wrong-but-valid valtype — a miscompiled boundary
that validates and runs to a wrong-typed value. Because `Kind` is a sum and every consumer matches it
exhaustively, adding a kind is a *compile error* in the compiler until every valtype map handles it,
exactly as an unhandled `Instr` variant is a compile error in the serializer. This is the same
type-directed principle the runtime already uses for rendering — the tag-free heap is rendered by
walking a static `Shape`, not a runtime tag
([[2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape]]) — now applied to the *emit*
boundary: the compiler decides the valtype from the resolved type, and the decision is total over a
closed set of kinds. It is also explicitly the **seam where full type inference will live**: today the
operand kinds of a primitive are fixed (a comparison's operands are Int64, its result Bool), so
`kind-of` is a direct structural read with no unification; when inference arrives it replaces the body
of `kind-of` without moving the seam. The architecture reserved the right place for it.

**The requirement it drove.** Two conformance cases in `03-equality-and-observation.sexp` pin the
observable: *"an entrypoint returning a comparison presents a Bool result at the boundary"*
(`(module m (def (lt a b) (< a b)) (def (main) (lt 20 22)))` → `true`) and its Int64 companion *"an
entrypoint returning arithmetic presents an Int64 result at the boundary"*
(`(def (main) (add 20 22))` → `42`). The pair is deliberately the *same* nullary-`main`-calls-a-helper
shape emitting a *different* boundary type from its result type alone, so together they pin that the
entrypoint's boundary result type is type-directed — Bool for a comparison, Int64 for arithmetic —
rather than a fixed valtype. Both PASS today (the seed already frames each correctly), turning the
type-directed-framing property into a permanent gate obligation. This complements the existing
bare-expression Bool cases (which exercise the value) by exercising the *boundary framing* at a
module entrypoint, which is the shape a compiler actually emits. The component-abi.md requirement that
"the entry's result type MUST have a boundary representation fixed by this contract" is the *what*;
this learning records the compiler-side *how* — a total, type-directed selection over a closed Kind
sum — and why it is structured to reject an unhandled kind rather than miscompile the boundary.
