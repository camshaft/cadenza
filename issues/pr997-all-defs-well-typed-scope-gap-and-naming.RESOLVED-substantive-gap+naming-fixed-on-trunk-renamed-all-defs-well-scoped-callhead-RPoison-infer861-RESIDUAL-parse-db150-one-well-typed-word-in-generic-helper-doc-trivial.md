# PR#997 review comments — all-defs-well-typed misses unbound callee-head + "well-typed" should be "well-scoped" (v-compiler-ml)

Mirrored from GitHub PR#997 review comments (Copilot), ids `3695464390` (infer-db.cdz:878, scope gap),
`3695464394` (sread-eval.cdz:30, +39, naming), `3695464401` (parse-db.cdz:154, naming). Compiler-ml port
source → v-compiler-ml. Blame `629386983` "compiler-ml: whole-program scope check — reject an unbound name
in an uncalled sibling def (reject-path gap 1)".

## Comment 1 (verbatim) — infer-db.cdz:878, SCOPE GAP

- (id 3695464390) "`all-defs-well-typed` is documented/used as a whole-program *scope* check (CDZ0101),
  but the implementation only flags `Resolved.RPoison`. Calls to an unknown callee `(foo 1)` don't
  produce `RPoison` in `resolve-node`'s `NApp` arm (it leaves the callee fact absent), so an unbound name
  that appears only in call-head position inside an uncalled sibling def would still pass this check and
  allow the program to run."

### Liaison verification (confirmed on trunk be950f1aa)

`all-defs-well-typed` (infer-db.cdz:861) → `all-defs-well-typed-go` → scans the resolved column for
`RPoison` (an unbound-name decision). Doc (856-860): "scans the resolved column for an RPoison". The check
`629386983` was added specifically to "reject an unbound name in an uncalled sibling def". Copilot's gap:
`resolve-node`'s NApp arm doesn't emit `RPoison` for an unknown CALLEE HEAD `(foo 1)` — it leaves the
callee fact ABSENT (not a filled RPoison). So an unbound name that appears ONLY in call-head position
inside an uncalled sibling def has no RPoison in the column → `all-defs-well-typed` returns true → the
program runs despite the unbound callee. So the scope-check the PR added still has a hole exactly in
call-head position. Owner (v-compiler-ml) confirms: does the NApp arm leave the callee-head unresolved
(absent) vs RPoison? If so, the scope scan must ALSO detect an absent/unresolved callee head (or
resolve-node must poison an unknown callee). Reject-path completeness.

## Comments 2-3 (verbatim) — naming: "well-typed" → "well-scoped"

- (id 3695464394, sread-eval.cdz:30, +39) "The comment says this is a 'well-typed' pre-check, but
  `all-defs-well-typed` is actually a whole-program *scope* check for unbound names (CDZ0101) and does
  not type-check every def. Wording this as 'well-scoped / no unbound names' will better match the actual
  behavior and avoid suggesting that all typing errors in uncalled defs are rejected here."
- (id 3695464401, parse-db.cdz:154) "This helper is for a whole-program *scope* pass (detect unbound
  names in any top-level def). The comment currently says 'well-typed', which is easy to misread as a
  full typecheck; aligning it to 'well-scoped / no unbound names' matches the actual intended use
  (CDZ0101)."

### Liaison verification (confirmed on trunk be950f1aa)

The helper is named `all-defs-well-typed` but only checks SCOPE (RPoison / unbound names), not full
typing — the call-sites' "well-typed pre-check" comments (sread-eval.cdz:30/39, parse-db.cdz:154) overstate
(they imply every def is typechecked, when uncalled defs are only scope-scanned). Reword the comments (and
ideally the helper name, owner's call) to "well-scoped / no unbound names". Doc/naming; behavior-neutral.
NOTE: comment 1's GAP means the "scope" check itself is incomplete (misses call-head) — fixing that first
makes the "well-scoped" naming actually accurate.

Owner: **v-compiler-ml** (compiler-ml port; `629386983`). Close the call-head scope gap (comment 1) + reword
"well-typed"→"well-scoped" at the call sites (comments 2-3).
