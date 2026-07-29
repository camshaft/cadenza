# PR#894 review comments — ord-key __CdzOpt wrapper: compound-key to-list doesn't unwrap + user-Option shadow mis-wraps (⚠ v-rust-backend)

Mirrored from GitHub PR#894 review comments (Copilot), ids `3674022652` (expr.rs:1863, +2005) +
`3674022734` (types.rs:266). Both `rcdzc/backend/rust`, both from `5980d7b14` "rcdzc(rust): SOUNDNESS #42
witness 2 — __CdzOpt Ord-wrapper for Option-keyed Set/Map" → v-rust-backend. Both correctness on the
Option-key ord-wrapper machinery.

## Comment 1 (verbatim) — expr.rs:1863 (+2005): compound key to-list leaks wrappers

- (id 3674022652, backend/rust/expr.rs:1863) "`Map.to-list` only unwraps keys for the *bare* `Option`
  key case. However `ord_key_type`/`wrap_ord_key` will also wrap `Option` nested inside tuple/record keys
  (because tuple/record key wrapping recurses element-wise). In those cases `BTreeMap` stores
  wrapper-containing tuple keys, but this `to-list` path will clone and return the wrappers instead of
  rebuilding the original key type, which will either fail to type-check or leak backend-only wrapper
  types into the language-level result. Consider adding a recursive 'unwrap-ord-key' mirror of
  `wrap_ord_key` and using it here whenever the key type needs ord-wrapping (including tuples/records
  containing wrapped leaves). This issue also appears on line 2005 of the same file."

### Liaison verification (confirmed on trunk ccc2048dc)

`wrap_ord_key` (expr.rs:285) RECURSES into `Ty::Tuple`/`Ty::Record` and wraps `Option` leaves at any depth
(the `Ty::Sum … is_flip_order_option_key_shallow` arm at :335 fires per-element), and `ord_key_type`
(types.rs:199) threads `__CdzOpt<…>` through nested tuples/records to match. But the `Map.to-list`
unwrap (expr.rs:1853-1863) handles ONLY the BARE Option key: `key_is_opt` → `__k.clone().get()`. A
tuple/record key CONTAINING an Option leaf is stored in the `BTreeMap` as a `__CdzOpt`-wrapping tuple, and
this path clones it verbatim (the plain `__k.clone()` else-arm) — returning backend-only `__CdzOpt`
wrappers into the language-level `to-list` result → rustc type mismatch or a wrapper leak. Same at :2005
(the sibling to-list/iteration path). This is the exact ASYMMETRY the float case already handles with the
recursive `key_ty_has_wrappable_float` rebuild — the Option unwrap needs the mirror. Fix (Copilot's, sound):
add a recursive `unwrap_ord_key` mirroring `wrap_ord_key`'s tuple/record/Option recursion (`.get()` per
wrapped leaf, rebuild the tuple/record) and use it wherever `key_ty_needs_ord_wrap(k)` holds, not just the
bare-Option branch. Witness: `Map<(Tuple (Option Int64) Int64), V>` → `Map.to-list`.

## Comment 3 (verbatim) — types.rs:266: user-Option shadow mis-wrapped (doc is self-contradicting)

- (id 3674022734, backend/rust/types.rs:266) "`is_flip_order_option_key_shallow` matches any sum named
  `Option` with one type arg, including a user-defined `(type Option …)` that shadows the prelude. In
  that case `ord_key_type` will emit `__CdzOpt<…>` even though the value is represented by the emitted
  user enum (not std `Option`), leading to Rust type mismatches when building `BTree{Map,Set}`
  keys/elements. To avoid mis-wrapping, the 'flip-order Option' test needs to be Db-aware (e.g., consult
  `enums::is_builtin_std_sum` on the sum decl occurrence) or `ord_key_type` needs a Db-aware variant for
  key positions."

### Liaison verification (confirmed on trunk ccc2048dc — and the doc is WRONG)

`is_flip_order_option_key_shallow` (types.rs:264): `matches!(ty.strip_nominal(), Ty::Sum { name, args, .. }
if name == "Option" && args.len() == 1)` — matches by NAME only, so a user `(type Option …)` shadow
matches. BOTH the type-side `ord_key_type` (:245, spells `__CdzOpt<…>`) AND the value-side `wrap_ord_key`
(expr.rs:335, wraps to `__CdzOpt::new`) gate on this SHALLOW test. The Db-aware `is_builtin_std_sum` guard
exists ONLY in the SEPARATE `ty_uses_flip_order_option_seen` traversal (expr.rs:3360), NOT in the key-wrap
path. CRUCIALLY the doc at types.rs:259-263 CLAIMS "the value-side wrap (`wrap_ord_key`, which HAS a `Db`)
confirms the built-in via `is_builtin_std_sum` before wrapping, so a user `(type Option …)` shadow … does
NOT get double-wrapped" — but `wrap_ord_key` (expr.rs:285) takes `(expr: String, key_ty: &Ty)`, has NO
`Db`, and uses the shallow test. So the claimed safeguard DOESN'T EXIST: a user-Option-shadow key/element
gets `__CdzOpt`-wrapped in both the type and the value while its actual rep is the user enum → rustc type
mismatch building BTreeMap/BTreeSet keys. Fix (Copilot's, sound): make the key-position flip test Db-aware
(consult `is_builtin_std_sum` on the decl occ, as `ty_uses_flip_order_option_seen` already does) — and
correct the types.rs:259-263 doc, which currently asserts a Db guard that isn't there. Witness: a module
with `(type Option (Some Int64) (None))` shadowing the prelude, used as a Map/Set key.

Owner: **v-rust-backend** (`rcdzc/backend/rust` Option-key ord-wrapper, `5980d7b14` SOUNDNESS #42). Both
correctness (wrapper leak on compound keys; user-Option-shadow mis-wrap). The types.rs doc also needs
correcting (it claims a Db guard the code lacks). Reachability of the user-Option-shadow is v-rust-backend's
call, but the compound-Option-key leak (comment 1) looks straightforwardly reachable.
