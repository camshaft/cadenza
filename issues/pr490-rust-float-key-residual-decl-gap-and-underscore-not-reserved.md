# PR #490 (merged, batch 119) — float-key wrapper: decl-injection gap + `__`-prefix is NOT actually reserved

Mirrored from Copilot inline on merged PR #490 (2 comments). Confirmed on trunk. Owner: **v-rust-backend**.
These are RESIDUAL gaps in the width-specific `__CdzF{32,64}` float-key fix (my earlier PR#487 findings).

## 1. Decl injected only on `::new(`, but wrapper name can appear without a constructor (comment 3597319849, rust/mod.rs:181)
> The float-key wrapper injection is gated only on the constructor marker `__CdzF{32,64}::new(`, but
> the wrapper type name can appear without any constructor call (e.g. a context-typed empty
> `Map.empty` / `Set.of (list)` emits an annotated `BTreeMap<__CdzF64,…>` / `BTreeSet<__CdzF64>` and
> still needs the struct decl). In that case the current scan won't insert `CDZ_F{32,64}_DECL`,
> producing Rust that references `__CdzF*` without a definition.

Trunk `rust/mod.rs:176-181`: `if out.contains("__CdzF32::new(")` / `"__CdzF64::new("` gate the decl.
An empty typed collection emits the type name in a turbofish/annotation with no `::new(` → decl missing
→ rustc "cannot find type `__CdzF64`". FIX: also gate on the TYPE-name occurrence (e.g.
`out.contains("__CdzF64")`) or track wrapper use during emit rather than post-hoc string scan.

## 2. `__`-prefix does NOT reserve the name — user `sum __CdzF64` collides (comment 3597319922, rust/mod.rs:216)
> The docs claim `__`-prefixed wrapper names can't collide because "a user ident never begins with
> `__`", but the lexer explicitly allows `_` as an identifier start (lexer.rs:639-646) and the Rust
> backend's `sanitize_ident` passes leading underscores through unchanged (rust/mod.rs:451-483). A user
> sum literally named `__CdzF64`/`__CdzF32` would sanitize to the same Rust ident → rustc E0428.

Trunk verified: `is_ident_start` (lexer.rs:640) = `c == '_' || c.is_alphabetic() || …` — a leading `_`
IS a legal Cadenza ident start. `sanitize_ident` (rust/mod.rs:454) does `if c == '_' … { s.push(c) }` —
leading `__` passes through unchanged. So the rename from `CdzF64`→`__CdzF64` did NOT fix the original
PR#487-#2 collision; it only made it less likely. FIX: reserve/reject `__*` names at the language level,
OR make `sanitize_ident` guarantee a user name can never produce a `__Cdz*` ident (e.g. escape a leading
`__` in user idents). Note this affects OTHER backend-reserved `__*` ids too (`__CdzE`, `__cdz_env`, `__pay`, `__p`).

PR: https://github.com/camshaft/cadenza/pull/490
