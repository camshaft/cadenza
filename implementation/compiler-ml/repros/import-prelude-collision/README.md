# BUG: a wildcard-exported variant whose name shadows a prelude type is unreachable to importers

`lib.cdz` declares `type T = | Foo(Int64) | List(List(T))` and exports it CONCRETELY
(`export { T.*, sz }` — handle + all constructors). `suite.cdz` imports `(T, sz)` and
constructs `T.List(...)` / `T.Foo(...)`.

`cdz compile . --entry suite` → **CDZ0214**: "`T`'s constructor `List` is not exported to
this file … `List` is withheld". But `T` IS wildcard-exported, and the SAME construction
in the DECLARING file compiles clean. The importer resolves `T.List` to the PRELUDE `List`
(a prelude-name collision) instead of the imported constructor, so it reads the constructor
as withheld.

- Trigger: a variant name that shadows a prelude type/name (`List`, `Bool`, `Str`?, …),
  wildcard-exported, then constructed/matched in an IMPORTING file.
- Control: non-colliding variant names (`NInt`/`NName`/`NList`) — imports + constructs fine.
- Control: the same `T.List(...)` in the declaring file — fine.

Impact on the compiler port: `Ast` naturally has a `List` variant (and `Bool`/`Str`), so a
test suite (or any consumer) in a SEPARATE file cannot construct `Ast` values. Workaround:
keep construction inside the declaring module, or expose smart-constructor functions.

Reproduce:
    cdz compile implementation/compiler-ml/repros/import-prelude-collision --entry suite -t wasm -o /tmp/out
