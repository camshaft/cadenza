# Phase-2 2b+2c staged: Dispatched-frame schema-hash collapse (STAGING — apply in the atomic window only)

STAGING artifact for the schema-hash effect-identity phase-2 flag-day. NOT a live edit: applying it now
would not compile (it depends on the `EventBody::Dispatched.schema_hash` `Option<Hash>` -> `Hash` struct
flip, which is part of 2b) and would red the platform target before the other lanes flip. Apply this to the
live `event_ast.rs` + `event.rs` struct in the phase-2 atomic squash the assembler (v-effects) opens.

Owner split: **2b (v-effects)** = the `EventBody::Dispatched.schema_hash` struct flip `Option<Hash>` -> `Hash`
+ the ~6 kernel.rs Dispatched-construction sites (become mandatory `req.schema_hash`) + the router
(`by_family` -> `by_schema_hash`) + authz (bind-to-schema_hash) flip. **2c (v-compiler-ml, handed over
2026-08-13)** = the `event_ast.rs` encode/decode below. They are INSEPARABLE (2c doesn't compile without 2b's
struct flip), so 2c folds INTO the 2b commit; v-effects lands both atomically.

Post-collapse Dispatched wire = fixed 6-leaf, DROP `kind`+`family`, `schema_hash` MANDATORY (slice-D=A
abandon-logs, no back-compat arity):
`(dispatched <id:Int> <target:Bytes KEEP> <idem:hash-raw32> <deadline:opt-ms> <token:opt-bytes> <schema_hash:hash-raw32>)`

## 2c ENCODE (v-cml) — replace the `EventBody::Dispatched` arm in the encode match (~event_ast.rs:1801)
```rust
EventBody::Dispatched { id, target, idempotency_key, deadline_ms, token, schema_hash } => {
    let head = b.name("dispatched");
    let idv = u64_leaf(b, id.0);
    let t = bytes_leaf(b, target);
    let idem = hash_form(b, idempotency_key);
    let dl = opt_ms_form(b, *deadline_ms);
    let tok = opt_bytes_form(b, token.as_deref());
    let shc = hash_form(b, schema_hash);   // schema_hash: &Hash after the 2b Option->Hash struct flip
    b.list(vec![head, idv, t, idem, dl, tok, shc])
}
// destructure DROPS kind+family. If 2b's struct KEEPS kind/family (populated, unrouted) add `..` to the
// pattern; if it DROPS them, the pattern above is exact. v-effects' call as struct owner.
```

## 2c DECODE (v-cml) — replace the entire `"dispatched"` arm (~event_ast.rs:2220/2232/2243, the 8/7/6 arities)
```rust
"dispatched" => {
    // Post-collapse: fixed 6-leaf. kind+family dropped (routing/authz key on schema_hash); schema_hash
    // MANDATORY, no back-compat arity (slice-D=A abandon-logs).
    let [idv, t, idem, dl, tok, sh] = form(a, id, "dispatched")? else {
        return Err(shape("dispatched arity"));
    };
    EventBody::Dispatched {
        id: EffectId(read_u64(a, *idv)?),
        target: std::sync::Arc::from(read_target_bytes(a, *t)?.as_slice()),
        idempotency_key: read_hash(a, *idem)?,
        deadline_ms: read_opt_ms(a, *dl)?,
        token: read_opt_bytes(a, *tok)?,
        schema_hash: read_hash(a, *sh)?,   // mandatory Hash (2b struct flipped Option->Hash)
    }
}
```
All helpers are existing (u64_leaf/bytes_leaf/hash_form/opt_ms_form/opt_bytes_form/read_u64/
read_target_bytes/read_hash/read_opt_ms/read_opt_bytes/form/shape) — no new helpers.

## 2b test updates (v-effects, in the same commit)
The event_ast round-trip @tests that build a `Dispatched` need the 6-leaf form: fixtures at ~event_ast.rs:2444,
2458, 2475, 2627, plus the schema_hash round-trip test (~2641 destructures `schema_hash`). Each currently
constructs the Option/8-leaf shape; flip to mandatory `schema_hash: Hash` + drop kind/family.

## Verified (2026-08-13) v-cml's diff targets are accurate against current origin (f8ac4d9a3)
- encode arm at event_ast.rs:1801; decode arms at :2220 (8-leaf, current w/ family), :2232 (7-leaf), :2243
  (6-leaf pre-family) — all three collapse to the ONE fixed 6-leaf above.
- helpers all present. Fixtures at :2444/2458/2475/2627 need the 6-leaf update.

## Co-gate (v-cml + v-effects) at window-open
encode(x) then decode == x (round-trip) + `cargo xtask gate --target platform` (the corpus consumer). v-cml
co-gates at window-open. My 2b kernel-wire decode (parse side) reads the SAME 6-leaf order this codec writes.
