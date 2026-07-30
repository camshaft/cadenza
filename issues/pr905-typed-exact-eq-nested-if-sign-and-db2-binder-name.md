# PR#905 review comment — typed-exact-eq: nested-if sign compare + confusable `db2` binder (v-compiler-ml, readability)

Mirrored from GitHub PR#905 review comment (Copilot), id `3678994090`.
File: `implementation/compiler-ml/src/db-demand.cdz:308` — compiler-ml PORT test helper → v-compiler-ml.
Blame `5a709ebd8` "compiler-ml(db-demand): item-4 differential — precise typed-exact-eq (PR#902)" — a
readability follow-on to the very `typed-exact-eq` fix I routed for PR#902 (`3677998528`).

## Comment (verbatim)

- (id 3678994090, db-demand.cdz:308) "In `typed-exact-eq`, the `TIntW` sign comparison is implemented via
  a nested `if` expression (`if sa then sb else (if sb then false else true)`), which is equivalent to
  `sa == sb` but is much harder to read/maintain. Also, the `TSum` binding name `db2` is easy to confuse
  with the surrounding `Db` variables in this module; a more semantic name reduces ambiguity in this test
  helper."

## Liaison verification (both confirmed on trunk 4c20f6bdd)

1. Line 304 (TIntW arm): `((if sa then sb else (if sb then false else true)) and (wa == wb))`. The nested
   `if` computes Bool-equality of `sa`/`sb` (both true → true; both false → true; else false) — correct
   but verbose. Copilot's `sa == sb` is equivalent IF `==` is available on `Bool` in the compiler-ml ML
   subset — WORTH CONFIRMING: this module uses `==` on ints/tags (e.g. `wa == wb` right beside it, and
   `apply-ty.cdz` uses `== 1`/`== 0-1` on tags), but I did NOT find a bare `Bool == Bool` usage in the
   port `.cdz` sources, so the nested `if` MAY be a deliberate workaround if Bool-eq isn't in the subset.
   If Bool `==` IS available → use `sa == sb`; if not → a named `bool-eq sa sb` helper (or a clearer
   `match (sa, sb)`) still beats the nested `if`. Owner's call (they know the subset).
2. Line 307 (TSum arm): `Typed.TSum(db2) => (da == db2)`. The binder `db2` visually collides with this
   module's pervasive `Db`-typed vars (`db`, `_d1`, `_d2`) — it's actually a SUM DECL id, not a Db. Rename
   to something semantic (`declB` / `sumDeclB` / `db_id_b`) so a reader doesn't misread it as a `Db`.

Both readability-only in a test helper, behavior-neutral. Low-priority polish.

Owner: **v-compiler-ml** (compiler-ml port test helper, their `5a709ebd8` PR#902 fix). Sign-compare
clarity (confirm Bool-`==` availability first) + rename the `db2` binder.
