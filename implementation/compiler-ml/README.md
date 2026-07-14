# The Cadenza compiler, written in Cadenza (ML surface)

A from-scratch port of the compiler into Cadenza itself, written in the **ML surface**, in *ideal
form* — the compiler you would write if the language were finished. The Rust reference compiler
(`implementation/seed/crates/rcdzc`) is the structural **guide**; this is not a transliteration but a
re-derivation in idiomatic Cadenza.

This is a deliberate **stress test of the language**. Where Cadenza cannot express something cleanly,
the rule is to **report the issue so it gets fixed** — either a fix landed in the seed `rcdzc`, or a
crisp repro filed — rather than contorting the code around a limitation. Friction found is a
deliverable.

## Toolchain

- Author `.cdz` files (ML surface). When unsure of syntax, generate the canonical form with
  `cdz convert <file>.sexp --from sexpr --to ml` — do not hand-transcribe nested `match`/patterns.
- **`cdz check file.cdz`** is the primary loop: every well-formedness fault as
  `file:line:col: severity [CODE]: message`, exit ≠ 0 on error. `--json` for structured output.
- To exercise the backend: `cdz convert file.cdz --to binary > file.bin && cdz compile file.bin -t wasm
  -o out.wasm` (compile is the full type-check + lowering).

## Structure (mirrors the rcdzc stages)

- `ast.cdz` — the AST datatype + pure traversals (the `ast.rs` analogue). One recursive sum; a node
  contains its children (no arena — the language has real recursive values).

Planned, following the rcdzc pipeline: decode (binary AST → `Ast`) · resolve · infer (Hindley-Milner)
· lower (→ core) · encode/emit. The compiler is fundamentally bytes → bytes.

## Language issues found (stress-test log)

- **FIXED** (seed `rcdzc` db.rs `scan_type_decl`): a `///` doc comment on a `type` declaration was
  mis-parsed — the ML reader attaches the doc as a `(doc …)` form after the type name, and the sum scan
  read it as a bogus `doc` variant (CDZ0201 "declared more than once"). Now the scan skips a leading
  `(doc …)`, mirroring how a `def`'s leading doc is stripped.
- **Note (not a bug):** author nested `match` via `sexpr → ml`; the reader resolves nesting by greedy
  last-arm absorption, so a hand-written inner `match` easily mis-attaches its catch-all to the outer
  match (CDZ0210 non-exhaustive + CDZ0213 unreachable). The printer's own output round-trips correctly.
