## 3. 🟢 Typed instruction sum for the backend (not string-tagged quasiquote)

**Finding (spike FINDING #1).** The backend IR should be a **typed sum** (`Instr`/`Lir`) matched
exhaustively, not `(Ast.List (list (Ast.Name "i64-const") …))` — a string tag in a `Name` payload
forfeits exhaustiveness (extends "reject, don't miscompile" to the backend: a missing opcode arm is a
compile error). Quasiquote stays for the genuinely-`Ast`-valued frontend/macro layer.

**Status.** 🟢 **DONE.** Landed in `compiler-pipeline.md` §Representation ("MUST represent instructions
as values of a typed sum type… serialize… exhaustively over its variants… an instruction variant the
serializer does not handle is a compile-time error") and §"The Compiler Constructs AST Values Via
Quasiquote" (quasiquote reserved for AST-valued frontend/macro; instruction sum built by ordinary
constructors). Learning:
`spec/learnings/2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic.md`.

---
