# PR#874 review comments — constant String.concat defeats intended rope-slice coverage (corpus-bugfix)

Mirrored from GitHub PR#874 review comments (Copilot), ids `3663105738` (19-sets), `3663105776`
(17-symbols). Both `spec/semantics/*.sexp` corpus cases → corpus-bugfix's lane. These are REAL
test-coverage gaps (not doc-only): the case intends to exercise a rope-backed String slice, but the
seed compiler constant-folds the `String.concat` so the slice is over a FLAT constant — the rope/view
key path never runs.

## Comments (verbatim)

- (id 3663105738, `spec/semantics/19-sets.sexp:2695`) "This case intends to exercise a rope-backed
  string slice (`String.concat \"xk\" \"eyz\"` seam inside the window), but `String.concat` folds two
  constant ASCII strings into a flat `ConstStr` in the seed compiler. As written, `rv` will be a slice
  of a flat constant string, so this won't actually cover the rope/view key path. Make one concat
  operand depend on the runtime `mode` so lowering must go through the runtime concat/rope
  representation (while still producing the same slice content)."
- (id 3663105776, `spec/semantics/17-symbols.sexp:801`) "`String.concat \"xk\" \"eyz\"` folds to a flat
  constant ASCII string in the seed compiler, so this doesn't actually intern from a rope-backed slice
  as the case description intends. Make one concat operand depend on `mode` so the concat can't fold,
  ensuring the symbol is produced from a runtime rope slice (while keeping the slice content \"key\")."

## Liaison verification (both CONFIRMED real on trunk 600865d68)

The seed compiler DOES fold constant `String.concat`: `rcdzc/src/resolved.rs:573-574` (Prim::StrConcat
doc) — "On two CONSTANT strings it FOLDS to their concatenation (`(String.concat \"hello\" \" world\")`
→ `\"hello world\"`); a runtime operand declines (the byte-rope path)." So:

1. 19-sets.sexp:2695 — case "a String slice VIEW keys a map by content in both directions, rope-backed
   included". `(def rv (Option.expect (String.slice (String.concat \"xk\" \"eyz\") 1 4) \"in\"))` — the
   `concat` folds to flat `\"xkeyz\"`, so `rv` is a slice of a FLAT constant. mode 3 (which stores `rv`
   as the map key) is documented as "stores a view of the ROPE `concat(\"xk\",\"eyz\")` — seam inside
   the window" but never actually hits the rope/view-key path. Output pins stay correct (all folds to
   the right content), so the test still PASSES — it just doesn't cover what it claims.
2. 17-symbols.sexp:801 — case "a Symbol interned from a rope-slice view keys maps and removes set
   elements". Same `(String.concat \"xk\" \"eyz\")` inside `Symbol.of(String.slice ...)` folds flat, so
   the symbol is interned from a flat-constant slice, not a runtime rope slice. The doc's "Symbol.of
   over a rope-backed slice, seam inside the window" is not exercised.

Suggested fix (Copilot's, sound): make ONE concat operand depend on `mode` (a runtime value) so the
fold declines and lowering goes through the runtime concat/rope rep, while keeping slice content =
"key". e.g. concat a mode-derived-but-content-fixed prefix, or gate the operand behind an `if` on mode.
Owner (corpus-bugfix) knows the corpus idiom for forcing a runtime operand.

Owner: **corpus-bugfix** (`spec/semantics/*.sexp` cases). Real coverage gap (tests pass but under-cover);
bundled as one note.
