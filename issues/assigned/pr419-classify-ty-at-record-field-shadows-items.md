# PR review comment — mirrored from GitHub PR #419 (Copilot inline)

- **PR:** #419 "fleet: forty-third batch (15 MRs …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/proptest_gen.rs:290` (`classify_ty_at` Record arm)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591806900
- **Link:** https://github.com/camshaft/cadenza/pull/419#discussion_r3591806900

## Comment (verbatim)
> `classify_ty_at`'s `(Record ...)` field loop accidentally shadows the top-level `items` slice with the field's own `(NAME TYPE)` list (`Struct::List(items)`). That means `classify_ty_at(..., items, ...)` is passed the *field items* instead of the program's top-level items, so bare-name user sums inside record fields can't be resolved and generator synthesis may incorrectly decline.

## Liaison triage — CONFIRMED against trunk
Confirmed in proptest_gen.rs `classify_ty_at`:
```
let field_items = match ast.get(field) {
    crate::ast::Struct::List(items) if items.len() == 2 => items,   // <-- binds a LOCAL `items`, shadowing the fn param
    _ => return None,
};
let fname = ast.as_name(field_items[0])?.to_string();
let fty = classify_ty_at(ast, field_items[1], items, depth + 1)?;   // <-- passes the SHADOWING `items` (= field list), not top-level items
```
The `match` arm binds `items` (the field's 2-element `(NAME TYPE)` list), shadowing the function's
`items` parameter (the program's top-level items). The recursive call then passes that shadowed `items`,
so a bare-name user sum used as a record field TYPE is resolved against the wrong item list → generator
synthesis wrongly declines. Contrast the Tuple arm just above, which correctly passes the fn's `items`.
Real bug in the property-testing generator. FIX: rename the arm binding (`Struct::List(fi)`) or pass the
fn's `items` explicitly. Property-testing territory (v-property-testing owns proptest_gen). Fix on
`trunk`. Quote + link in queue file.
