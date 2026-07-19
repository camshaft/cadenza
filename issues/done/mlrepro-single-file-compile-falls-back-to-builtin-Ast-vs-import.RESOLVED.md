# FINDING: `cdz compile <single-file>` uses the BUILT-IN `Ast`; `cdz test`/import-following uses the CANONICAL `Ast`

v-compiler-ml stress observation (2026-07-15). A module that does `import { Ast } from "ast"` and matches
on `Ast` gets a DIFFERENT `Ast` type depending on how it is built:

- **`cdz test .` / `cdz check`** (follow the import closure): `Ast` = the canonical `ast.cdz` sum
  (`Int/Str/Bool/Name/List` — 5 variants, no `Float`). A 5-arm match is exhaustive. ✅
- **`cdz compile <file> -t wasm`** (SINGLE file): "`import` is a module form this compiler does not yet
  model (cross-module imports are not supported here)" — so the `import` is DROPPED and the bare name
  `Ast` falls back to the **built-in metaprog `Ast`** (the `quote` reifier's type), which HAS extra
  variants (`Float`/`Bool`/`Str`), so the SAME 5-arm match now reports CDZ0210 "pattern `Float` not
  covered". ❌

So the same source file's exhaustiveness (and which `Ast` it even denotes) flips between the two tool
paths. This is confusing for a port author: a module that `cdz test`s clean fails `cdz compile` with a
non-exhaustive error about a `Float` variant the imported `Ast` does not have.

IMPACT: low (the port uses `cdz test .` which follows imports, so modules are correct there), but it is a
genuine toolchain inconsistency:
1. `cdz compile` single-file should EITHER follow imports (like `cdz check`/`cdz test` already do) so a
   bare `Ast` resolves to the imported canonical type; OR
2. when it can't model the import, it should ERROR on the unresolved import as the primary failure and NOT
   silently bind the name to the built-in metaprog `Ast` (which produces the misleading downstream
   "pattern Float not covered" cascade).

REPRO: any compiler-ml module importing `Ast` and matching all 5 canonical variants — e.g. build one via
`cdz test src/scopecheck.cdz` (passes) vs `cdz compile src/scopecheck.cdz -t wasm -o /dev/null` (import
not modeled + a spurious `Float`-not-covered if it matches `Ast`). Discovered while writing a Church-
numeral integration module.
