## 39. 🟡 Runtime AST construction / `Ast.encode` / `Ast.decode` decline — `Ast` is a compile-time-only value, not a registered RUNTIME sum type

**Finding (2026-07-07, split off from ask-38).** The COMPILE-TIME AST path is complete: `(Ast.Int 7)`
as a constant is a value, matches, encodes/decodes (`Ast.decode` now total `Bytes → Result<Ast, e>` —
ask-38), and compares equal to `(quote 7)`. But the **RUNTIME** AST path declines, because `Ast` is a
compile-time `CVal::Ast(node)` special case, NOT a registered runtime heap sum type with a variant order:

| program (runtime operand forces the non-const path) | current |
|---|---|
| `(def (mk n) (Ast.Int n)) (main (mk 9))` — render a runtime-built AST | **declines** "unknown sum variant: Ast.Int" |
| `(def (mk n) (Ast.encode (Ast.Int n))) (main (mk 9))` — encode a runtime AST | **declines** "unsupported dotted-application" |
| `(def (d b) (Ast.decode b)) (main (d (Ast.encode (quote 7))))` — decode runtime `Bytes` | **declines** "unsupported dotted-application" |
| `(def (mk n) (Ast.Int n)) (main (match (mk 9) ((Ast.Int v) v) …))` — runtime construct + match | **works** ✓ (match already lowers runtime sums) |

So runtime MATCH on a runtime-built AST works (the general runtime-sum-match path handles it), but
runtime CONSTRUCTION-as-a-value (render), and `Ast.encode`/`Ast.decode` on runtime operands, do not —
those three are const-fold-only.

**Why it's NOT currently blocking self-hosting.** `compiler.cdz`'s entry `(def (compile-bytes b) …)`
decodes its runtime input `b` with its OWN hand-written `read-module` CBOR reader (over runtime `Bytes`),
not the built-in `Ast.decode`; and it builds its output via its own typed-IR + serializer, not by
constructing runtime `Ast.*` values and calling `Ast.encode`. So the built-in runtime AST codec is not on
the critical path today. Operator decision (2026-07-07): **file for later, move the loop to the next real
blocker.**

**Proposed resolution (seed, M2-scale — when prioritized).** Register `Ast` as a runtime heap sum type
with a fixed variant order (`Int | Float | Str | Bool | Name | List`, matching `ast_sum_to_node` /
`node_to_cbor`), so a runtime `(Ast.Int n)` lowers via the ordinary `gen_runtime_ctor` path, renders via
`Shape::Sum`, and `Ast.encode`/`Ast.decode` get runtime lowerings over the value heap (encode walks the
runtime sum → Bytes; decode is the inverse fallible reader, the runtime twin of the const fold). Spans the
usual ~5 places (construct + match/render + encode + decode + infer/shape). Until then it is decline-clean
(reject-don't-miscompile), not a miscompile.

**Acceptance signal.** `(def (mk n) (Ast.Int n)) (main (mk 9))` renders `(Ast.Int 9)`; a runtime
`Ast.encode`/`Ast.decode` round-trips over a runtime `Bytes`; existing const cases unchanged. Corpus:
runtime companions of the 12-metaprogramming.sexp const cases.

**Status.** 🟡 **Seed (M2-scale), deprioritized by operator — not blocking self-hosting** (compiler.cdz
uses its own reader/serializer). Const AST path complete. Related: ask-38 (the total `Ast.decode` this
splits from), [[ast-decode-total-result-and-cval-ast-roundtrip]], [[quote-vs-ast-constructor-equality-miscompiles]]
(the const construct/consume bridge), the M2 runtime-heap sum machinery (`gen_runtime_ctor` /
`gen_match_runtime_sum` / `Shape::Sum`).
