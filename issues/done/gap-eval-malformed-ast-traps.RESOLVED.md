# Gap: `eval` on a malformed AST must trap (standing gate TODO)

**File:** `spec/semantics/12-metaprogramming.sexp` — case "eval on malformed AST traps" grades TODO.
**Confirmed:** on current trunk this case is a standing todo (the compiler can't yet handle it).

`eval` applied to an `Ast` value that is not a well-formed program must TRAP (a defined trap), not
produce an unspecified value or silently succeed. Implement the eval path's malformed-input check so
the case passes; confirm against the spec text for metaprogramming/eval semantics. Add the graded
case as its own regression (it already exists as a todo — make it pass).

Area: rcdzc metaprogramming (`eval` builtin). Coordinate with v-metaprogramming (owns quote/eval/Ast).
