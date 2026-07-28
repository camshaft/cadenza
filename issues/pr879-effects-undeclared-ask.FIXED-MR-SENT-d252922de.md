# PR#879 review comments — undeclared Ask effect in corpus case + weak decline assertion in v-effects test

Mirrored from GitHub PR#879 review comments (Copilot), ids `3667458726` (corpus → corpus-bugfix),
`3667458772` + `3667458797` (rcdzc effects test → v-effects). Two owners, split below.

## Comment A (verbatim) — corpus-bugfix

- (id 3667458726, `spec/semantics/14-effects-and-handlers.sexp:5747`) "This case uses `handle Ask ...` /
  `(Ask.ask ...)` but never declares the `Ask` effect in the module, so the `(declines)` can be
  satisfied by an unrelated 'unknown effect' rejection/decline rather than exercising the intended
  tail-resumptive-fold behavior."

### Liaison verification (confirmed on trunk 64ee9058c)

Case "an effect performed via a collection-extracted closure declines honestly (not-yet-reducible, not
a false no-handler claim)" (:5745). Body: `(handle Ask 5 ((ask (n) s ...) (match (List.at (list (fn (x)
(Ask.ask x))) 0) ...)))` with `(export main)` then `(declines)`. There is NO `(effect Ask (op ask ...))`
declaration in the module — unlike the SIBLING pin just above (:5730) which declares `(effect E (op get
...))` / the `1033` case which declares `A`/`B`. So the intended decline ("not yet reducible by the
tail-resumptive fold", v-effects `1747c764a`) could be masked by a plain "unknown effect Ask" decline —
`(declines)` doesn't distinguish. Add the `(effect Ask (op ask (-> Int64 Int64)))` declaration so the
decline exercises the real fold path. Corpus lane.

Owner A: **corpus-bugfix** (`spec/semantics/*.sexp`).

## Comments B (verbatim) — v-effects

- (id 3667458772, `implementation/seed/crates/rcdzc/src/tests.rs:58831`) "The comment says
  `compile_scratch` returns Ok/Err, but this test calls `compile_component`. This mismatch makes the
  intent harder to follow when reading the test."
- (id 3667458797, `implementation/seed/crates/rcdzc/src/tests.rs:58849`) "The `Err(_)` arm silently
  accepts any compilation error, including coded rejections (e.g. CDZ0401) that would indicate a real
  regression in handler routing/typechecking. To avoid the test becoming a false green, assert that the
  error is an uncoded 'decline' (i.e. `code.is_none()`)."

### Liaison verification (confirmed on trunk 64ee9058c)

Test in the nested-handler binder-collision safety pin (blame `eba1a7930` "rcdzc(effects): pin
nested-handler binder-collision safety"). Line 58830 comment: "`compile_scratch` returns Ok(bytes) if it
folds, Err otherwise" — but the code (58824, 58838) calls `compile_component`, not `compile_scratch`.
Stale name. Line 58848: `Err(_) => { /* declines cleanly (todo) — acceptable ... */ }` accepts ANY Err,
so a coded rejection (e.g. CDZ0401 handler-routing/typecheck regression) would pass as a clean decline —
false green. Tighten to assert the error is an UNCODED decline (`code.is_none()`), matching the comment's
"todo/not-yet-reducible" intent. Both test-quality, behavior-neutral (the pin's real assertion — value ==
1033 on the Ok path — is unaffected).

Owner B: **v-effects** (their `eba1a7930` nested-handler pin in rcdzc effects tests).
