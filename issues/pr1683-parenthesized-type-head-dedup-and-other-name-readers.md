# PR #1683 review comments — rcdzc/src/db.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1683 (extract type name + head params from a parenthesized
`(type (Name a …) …)` head). Both points VERIFIED — a real completeness pair on the parser change.

## 1. head_params not de-duped → `(type (Box a a) …)` inflates arity (Copilot, db.rs:5471) — correctness [VERIFIED]
> `head_params` can contain duplicates when the head repeats a param name (e.g. `(Box a a)`). Unlike
> `collect_type_params`, this path doesn't de-dup, so `decl.params.len()` exceeds the distinct-param
> count → the sum constructor appears higher-arity and can mis-type/check or mis-render generic sums.

VERIFIED (db.rs ~5460): the head-param collect is `.filter_map(as_name).filter(lowercase && != "unit")
.map(...).collect()` — NO de-dup. `(type (Box a a) …)` yields `["a","a"]`, inflating param count vs
`collect_type_params` (which de-dups). De-dup in first-appearance order to match the payload-param path.
MED.

## 2. Other name-readers still use `tail.first().as_name()` → won't recognize the parenthesized head (Copilot, db.rs:5451) — correctness/completeness [VERIFIED]
> This teaches `scan_type_decl` to accept `(type (Name a …) …)`, but other paths still read the name as
> `tail.first().as_name()` and will fail to recognize such decls (`link::top_item_defined_name`,
> `invariant_establish`'s `(type …)` scan, `proptest_gen`'s `name_resolves_to_user_type`/`classify_sum`)
> → incorrect absent/unexported import diagnostics, missed invariant synthesis, generators not finding
> user types.

VERIFIED: `top_item_defined_name` (link.rs:779) reads a `(type …)` name via `tail.first()
.and_then(as_name)` — which returns `None` for a parenthesized head (`tail.first()` is a `List`, not a
name-atom). It's used in link.rs:413 for export/import name resolution, so a `(type (Box a) …)` decl is
invisible → treated as un-exported/absent. Same shape for the invariant/proptest readers Copilot names.
MED completeness gap. Fix: extract a shared "type-decl head → (name, params)" helper and use it
everywhere raw `(type …)` tails are name-read, so the new syntax is recognized consistently. Recommend
v-inference confirm the caller set before/after land.
