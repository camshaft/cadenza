# `cdz fix` mis-anchors a quick-fix on a MULTI-FORM s-expr program (rewrites a neighbour node)

**Severity:** miscompile of a repair — `cdz fix` DESTROYS valid code, silently.
**Owner:** `main.rs` / `cdz fix` path = **v-cdz-tooling**. Found + verified by **v-lsp**.
**Sibling:** same root cause as the LSP `parse_surface` s-expr canon bug (landed `281291b36`) — the
s-expr node-id/canonicalization mismatch, here in the `cdz fix` loop's span rebuild.

## Repro
`/tmp/mf2.sexp` (a MULTI-form s-expr program — two top-level `def`s):
```
(def (a (: x Int64)) x)
(def (b (: y Int64)) 5)
(export a b)
```
`cdz check` correctly reports one warning: `2:12 CDZ0306 unused parameter: y … replace with _y`.
`cdz fix --diff` produces a WRONG edit — it rewrites the TYPE, not the param:
```
-(def (b (: y Int64)) 5)
+(def (b (: y _y)) 5)     # WRONG — should be (: _y Int64)
```
(The earlier `(def (b y) 5)` shape is even worse: the whole `(b y)` param-list is replaced, giving
`(def _y 5)` — the function is destroyed.)

**Only MULTI-form s-expr is affected.** SINGLE-form s-expr (`(def (b (: y Int64)) 5)` alone) fixes
correctly (`(: _y Int64)`), and the ML multi-def equivalent fixes correctly (`b(_y: Int64)`).

## Root cause (confirmed)
`run_fix` (`main.rs`) loads via `load_program_spanned` (which CANONICALIZES), so the `Diagnostics`
query answers with CANONICAL node ids. But the per-iteration span rebuild `reparse_spans` (main.rs
~L4118) canonicalizes ONLY the ML branch; its s-expr branch returns the RAW spans from
`read_spanned`/`read_all_spanned`. A multi-form program's `read_all_spanned` fallback wraps the roots in
a synthetic `(do …)` whose head is built LAST, so canonicalization REORDERS the ids — the canonical
`fix_node` from the diagnostic then indexes a NEIGHBOUR's span in the un-remapped table → the edit lands
on the wrong node. (Exactly the class documented in `parse_program_spanned_counted`'s own comment, and
the one v-lsp just fixed in `lsp.rs::parse_surface`.)

## Verified fix (one locus)
Mirror the ML branch (and `parse_program_spanned_counted`) in `reparse_spans`'s s-expr arm:
```rust
_ => {
    let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(text) {
        Ok(pair) => pair,
        Err(_) => cadenza_syntax::sexpr::read_all_spanned(text).ok()?,
    };
    let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&raw_arenas);
    Some(raw_spans.remap(&id_map, arenas.structure.len()))
}
```
With this patch, `cdz fix --diff /tmp/mf2.sexp` correctly yields `(: _y Int64)`. A lone form is an
identity map (no-op), so single-form behavior is unchanged.

## Gate suggestion
Add a fix corpus / `cdz fix` test over a multi-form `.sexp` unused-param case asserting the edit renames
the param (not the type), so this can't regress.
