# WIT component-boundary shape coverage

A **living, honest checklist** of which value SHAPES cross the WIT component boundary, on which
path, and which are verified by a running corpus case vs. merely admitted by the gate vs. a tracked
gap. This exists because "full general WIT support" has repeatedly turned out to mean "a specific
accepted subset" (the latest: `result_is_liftable` admitted `list<u8>` but not `string` — the
host-string-RESULT gap, closed by #4894). Ground truth is the **gate predicates named below**, not
this prose — when you edit a predicate, update the matching row here so the checklist stays true.

This doc is keyed on **function/predicate names** (stable across edits), not line numbers (which rot).
Grep the name to find the arm.

## The four paths

A host operation (`(effect …)` op) crosses on one of three paths, selected inside
`first_unrepresentable_host_op` (the master decline gate, `backend/wasm/host.rs`) by two booleans:

- **bare** — a plain `(effect …)`: `!allow_option_bytes && !peer_bound`. Scalar/unit results only;
  scalar/unit/string/bytes arguments only. The compound envelope is intentionally NOT on this path.
- **world** — the world-driven / reducer / typed-interface path: `allow_option_bytes && !peer_bound`
  (needs a component + a `wit_world` with a bytes-crossing or typed-record export; set in
  `backend/wasm/mod.rs` where `allow_option_bytes` is computed). This is the compound envelope.
- **peer** — a peer-bound effect (`db.effect_bindings`): any compound crosses as an opaque `u32`
  handle via `extern_abi_val_type` (no structural marshal — by design).

Separately, the **export** boundary (a guest export member) has three sub-paths in `mod.rs`:
`try_bare_entry_param_component` (a single bare export), `record_interface_export` (a typed
interface instance, incl. `needs_result_wrapper` spilled-compound results), and
`emit_bytes_provider_member` (value-form `list<u8>`↔`list<u8>`).

The WIT **signature** side (declaring these shapes) is v-inference's `wit_world.rs`:
`wit_type_to_ty` (INBOUND: WIT→Ty), `ty_natural_wit` (OUTBOUND base: Ty→WIT), and
`wit_type_to_type_expr` (injectable source form). Sum-aware OUTBOUND (option/result/variant/enum from
a `Ty::Sum`) is synthesized on the EMIT side here (`spilled_result_wit_type`), NOT in `wit_world.rs`.

## WIRED + CORPUS-VERIFIED (a running `28-wit-abi-boundary` SHAPE proves it)

| shape | positions | path | admitting predicate | corpus SHAPE |
|---|---|---|---|---|
| scalars (Bool, Char, F32/F64, Int s8/u8…s64/u64, Qty-over-scalar) | arg + result | all | `abi_val_type` | 3, 10, 14 |
| Unit | arg + result | all | literal allow-set / gate | — |
| String | ARG all; RESULT world; export-param (bare-entry) | arg/result/export-param | bare(arg)/world(result) | 57 (result), log.emit (arg) |
| Bytes (`list<u8>`) | arg all; result world; leaf/field/element; export-param | all/world | `result_is_liftable` (Bytes arm) | 9, 14, 22, 37 |
| List&lt;scalar\|bytes\|list\|tuple\|record\|option\|variant&gt; | arg + result | world | `list_elem_marshalable` / `result_is_liftable` (List) | 12, 24, 30, 33, 34, 38, 39 |
| Tuple (all leaf-liftable) | arg + result | world | `result_is_liftable` (Tuple) | 33, 34 |
| Record (all fields boundary/leaf, incl. nested + WIT-order reorder) | arg + result + export | world/export | `is_boundary_record` / `result_is_liftable` (Record) / `record_interface_export` | 4, 11, 13, 19, 20, 21, 25, 29, 31, 35, 36 |
| option&lt;scalar\|bytes\|leaf-liftable&gt; | field + result | world | `option_payload_ty` | 1, 8, 16, 35, 36, 38 |
| result&lt;list&lt;u8&gt;, enum&gt; | arg + result | world | `result_bytes_enum` | 15, 17 |
| variant (scalar / mixed-width join / compound payload) + payloadless enum | arg + result | world | `variant_scalar_payload_cases` / `variant_liftable_payload_cases` / `enum_cases` | 2, 18, 32, + vres/cvp/mwv/wen families |
| scalar-param → spilled compound (record) result | export | export | `needs_result_wrapper` (SpillRecord retptr) | 56, sp1–sp7 |

**Value ROUND-TRIP only (NOT a typed-WIT-export verification):** SHAPE 58/59 pin that a payloadless
enum value round-trips through the guest + the generic `cadenza:run/run` encode envelope. They do
NOT verify a typed WIT `enum` export — the compiler cannot emit one today and FALLS BACK to run/encode
(confirmed by WIT-dump). Typed enum export is a DECLINED gap (below).

## WIRED but UNTESTED (predicate admits; no dedicated running SHAPE — verify opportunistically)

- `option<compound-leaf>` result (only `option<scalar>` / `option<bytes>` have SHAPEs).
- `Qty`-over-scalar result.
- a record result carrying a `variant` field.

## GAPS — DECLINED, tracked (owner in brackets)

**Synth side — v-inference (`wit_world.rs`):**
- **[synth, Direction B]** enum/variant/flags NOMINAL SYNTHESIS from an IMPOSED/external world that
  declares the shape with NO guest mirror sum: `synthesize_world_import_effect_decls` skips such an op
  today. Closing it = synthesize a nominal (`type <Name> …`) + inject + add to the sums map. Squarely
  `wit_world.rs` (v-inference). OPEN DESIGN Q (escalated): what NAME the anonymous WIT enum's synthesized
  nominal gets, and how the guest constructs/matches variants it didn't declare — needs a reference model.
  MANDATORY per the operator "full WIT algebra" ruling.
- **[synth]** option/result OUTBOUND: `wit_world.rs` maps `Ty::Sum`→None; outbound sum WIT is emitted
  here (`spilled_result_wit_type`). Imposed-world works; a synthesized-world option/result *result*
  cannot self-declare (rolls into the nominal-decl increment).

**Emit side — this crate (v-rust-backend):**
- **[emit]** multi-payload variant case (≥2 payloads); mixed int↔float / f32↔f64 single-payload
  variant join — see `variant_scalar_payload_cases` / `variant_liftable_payload_cases`.
- **[emit]** compound variant payload at the ARG (register-flatten) position; compound-payload
  variant list-element.
- **[emit]** `option<compound>` record field / list element (only `option<scalar>`/`option<bytes>`).
- **[emit]** `list<record|tuple>` element with a nested record/list/tuple field, or a tuple/variant
  element.
- **[emit]** `result<list<u8>, VARIANT>` err arm — `spilled_result_wit_type` always emits `enum`; a
  WIT `variant` err needs the world result type threaded (#3228 result-side).
- **[emit, export, Direction A — MANDATORY, my next unit]** a typed enum/Sum EXPORT (guest declares
  `type Status = …`, annotates an export/import, no `wit-world` clause) does NOT emit a typed WIT
  `enum{…}`/`variant{…}` — the export lift's result/param match declines a Sum (`ty_natural_wit(Sum)
  →None`, the "later slice"), so the WHOLE program FALLS BACK to the generic `cadenza:run/run` encode
  envelope (verified by WIT-dump on SHAPE 58: `make/run/encode`, no typed `f`). Scalars DO get a typed
  export (SHAPE 3: `export f: func(s64)->s64`). Fix: route the export sum-result/param lift through a
  db-aware `Sum→WitType::Enum`/`Variant` built from `enum_cases`/the variant detectors, instead of
  falling back. Emit-side (mine); no new `wit_world.rs` fn needed (enum_cases gives the case names).
- **[emit, export]** a NON-SCALAR entry PARAM (enum/Sum, and a record whose fields aren't all
  boundary-scalar) — `try_bare_entry_param_component` + `is_boundary_record`/`field_boundary_abi`
  decline it: *"parameter … has no scalar boundary representation — a non-scalar entry parameter is not
  yet emitted on this export path"* (verified: an enum export param, and a record with an enum field,
  both decline `todo`). The param twin of Direction A above.
- **[emit, export]** top-level Tuple/Sum/List/String/Bytes typed-interface PARAM
  (`record_interface_export`); a `result<>` top-level export result writer (`canon_write_of`); a flat
  1-value record result; an enum-disc result needing case REORDER; a nested/compound list-param
  element on the bare-entry path; a `result<>` bare-entry param.

**Design-level (no WIT boundary form on either side; needs a design decision — TRACK, don't rush):**
- **[design]** BigInt, Rational, exact-`Qty`, Map, Set, Symbol.

## By design — NOT gaps

- **bare-effect** path is scalar/unit-only for results (the world-driven path is the compound
  envelope). State it; don't "close" it.
- **peer-bound** crosses any compound as an opaque `u32` handle (`extern_abi_val_type`) — no
  structural marshal is intended.

## Harness caveat (a run-form limit, NOT an emit limit)

A String-result host op is emit-verified only via SHAPE 57's REDUCER-EXPORT form. The corpus gate
HOST-RESPONDER cannot yet ANSWER a String-result host op on the bound/simple-export form (traps on
`bind`+`host-responses`) or the bare-effect / `wit-world`+scalar-export forms. Those run-forms are
gate-blocked by the harness, not by emit (v-wasmtime-migration confirmed #4894 compiles run_agent's
bound `converse (-> String String)` and the rcdzc U9 test passes). The same caveat likely applies to
other host-RESULT shapes whose only running SHAPE is the reducer-export form.

## Recently closed

- host-string-RESULT (world path) — `result_is_liftable` gained the `string` leaf arm (#4894); SHAPE 57.
- unit OUTBOUND synth — `ty_natural_wit` `Ty::Unit → WitType::Unit` (#4903), the exact inverse of
  `wit_type_to_ty`'s inbound arm; a synthesized-world unit result now self-declares.

## Keeping this honest

- "WIRED + CORPUS-VERIFIED" requires a *running* SHAPE, not just a predicate arm. Adding an arm
  without a SHAPE puts the row under "WIRED but UNTESTED" until a case runs it.
- A gate PASS on a synthesized-world case (no `wit-world`/`component-name`) proves the value ROUND-TRIPS
  — it does NOT prove a typed WIT export was emitted. When it can't emit a typed export the compiler
  FALLS BACK to the generic `cadenza:run/run` encode envelope and the case still passes. To claim a
  *typed* WIT shape crosses, DUMP THE WIT (`wasm-tools component wit <out>.wasm`) and check for the
  actual `enum`/`variant`/`record` type — not just the gate verdict. (This is how the SHAPE 58/59
  enum-export over-claim was caught.)
- When you close a gap, move its row up and cite the SHAPE that verifies it.
- When you add a `Core`/`Ty`/`Prim` variant, decide its boundary form here (or add a gap row).
