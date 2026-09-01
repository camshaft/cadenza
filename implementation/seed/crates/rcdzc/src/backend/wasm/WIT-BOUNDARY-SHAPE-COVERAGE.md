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
| Record (all fields boundary/leaf, incl. nested + WIT-order reorder) | arg + result + export | world/export | `is_boundary_record` / `result_is_liftable` (Record) / `record_interface_export` | 11, 13, 19, 20, 21, 25, 29, 31, 35, 36 |
| option&lt;scalar\|bytes\|leaf-liftable&gt; | field + result | world | `option_payload_ty` | 8, 16, 35, 36, 38 |
| result&lt;list&lt;u8&gt;, enum&gt; | arg + result | world | `result_bytes_enum` | 15, 17 |
| variant (scalar / mixed-width join / compound payload) + payloadless enum | arg + result | world | `variant_scalar_payload_cases` / `variant_liftable_payload_cases` / `enum_cases` | 18, 32, + vres/cvp/mwv/wen families |
| scalar-param → spilled compound (record) result | export | export | `needs_result_wrapper` (SpillRecord retptr) | 56, sp1–sp7 |
| payloadless enum RESULT as a typed WIT `enum` under a DECLARED world | export | export | `record_result_lower` enum arm (Passthrough i32) + `note` re-export | 60 (WIT-dump: `enum t0`) |
| variant-with-payload RESULT as a typed WIT `variant` under a DECLARED world | export | export | `record_result_lower` SpillRecord + `canon_write_of` variant arm | 61 (WIT-dump: `variant t0 { continue, close(s64) }`) |
| TUPLE RESULT (bare, + as a variant/record payload) as a typed WIT `tuple` under a DECLARED world | export | export | `canon_write_of` Ty::Tuple arm (positional, reuses `CanonWrite::Record`) | 62, 63 (WIT-dump: `tuple<s64,s64>` / `two(tuple<s64,s64>)`) |
| RECORD result with a compound field (a `variant` field) as a typed WIT `record` under a DECLARED world | export | export | `canon_write_of` Record arm recursing its Variant arm | 65 (WIT-dump: `record t1 {o: variant, n}`) |
| option<COMPOUND> RESULT (an `option<record>` field) as a typed WIT `option` under a DECLARED world | export | export | `canon_write_of` option arm recursing its payload | 66 (WIT-dump: `record t1 {d: option<t0>, n}`) |

**Value ROUND-TRIP only (NOT a typed-WIT-export verification):** SHAPEs 1, 2, 4, 5, 7, 58, 59 (all
NO-`wit-world`-clause) compile to the generic `cadenza:run/run` encode envelope — verified by WIT-dump
(breaker audit 2026-08-28) — NOT a typed record/sum/enum export. They pin the option/variant/record/
list/enum VALUE ROUND-TRIP through the guest + encode, which is real coverage, but do NOT verify a typed
WIT export. So the WIRED rows above cite only the `wit-world`-declared (ww=Y) SHAPEs as their typed
export/world proof; 1/2/4/5/7 are NOT cited there. These are the natural FLIP-WITNESSES for the typed
record/Sum EXPORT emit unit (they become running typed-export proof once a no-clause guest annotation
synthesizes the world member — see the no-world synthesized sum-export gap below). Verify typed shapes
by WIT-dump, never a gate PASS (the encode envelope masks a typed-export decline).

## WIRED but UNTESTED (predicate admits; no dedicated running SHAPE — verify opportunistically)

- `Qty`-over-scalar result.

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
- **[emit, ARG-side]** `option<compound>` host-op record-ARG field / list element (only `option<scalar>`/`option<bytes>` marshalable). The RESULT side is DONE — SHAPE 66; this is the marshal side (`field_boundary_abi`/`list_elem_marshalable`).
- **[emit] typed `list<COMPOUND>` EXPORT result element — ✅ DONE (SHAPE 69/70/71/72/73).** A typed
  `list<tuple<s64,s64>>` (69), `list<record{lo,hi}>` (70), NESTED-element `list<tuple<s64, list<s64>>>`
  (71), `list<variant{lo,hi(s64)}>` (72), and `list<tuple<s64, variant>>` (73) EXPORT result all cross by
  RECURSIVE `canon_write_of` composition with NO new emit (`CanonWrite::List` whose element is the
  Tuple/Record/Variant write, recursing into an inner `List`/`Variant` for a nested field). So EVERY
  compound list-element (record/tuple/list/variant, incl. a nested compound field) crosses on the RESULT
  side — the doc's old `list<...> element with a nested record/list/tuple/variant field` gap is CLOSED.
  The typed-export twin of SHAPE 7 (untyped run/encode). All are `(live-objects known-leak)` — the spilled
  list result + boxed elements are not reclaimed (the SpillRecord-result reclaim class, SHAPE 60/62/63;
  value-correct, routed to v-memory-safety). REMAINING (still open): the ARG-side (marshal) nested compound
  list element (`list_elem_marshalable`, host→guest), a distinct direction from this RESULT-side write.
- **[emit]** `result<list<u8>, VARIANT>` err arm — `spilled_result_wit_type` always emits `enum`; a
  WIT `variant` err needs the world result type threaded (#3228 result-side).
- **[emit, export] typed enum RESULT under a DECLARED world — ✅ DONE (SHAPE 60).** A payloadless-enum
  export result under an imposed/in-source `(world … (result ("enum" …)))` now crosses as a typed WIT
  `enum{…}` (WIT-dump: `enum t0 {red,green,blue}` + `f: func(s64)->t0`), not the old bare `u32`. Fix:
  `record_result_lower` gained a payloadless-enum arm → `Passthrough` i32 (the def already returns the
  raw disc = `flatten(Enum)`), and `needs_result_wrapper` is set for it so the typed path takes over from
  the provider path; the enum defined type is emitted + re-exported by the existing `note` pass. Guard:
  guest decl-order case names must equal the WIT case order (a reorder would need a runtime disc remap).
  ✅ **FIXED (breaker FINDING 1, SHAPE 64):** on that order-mismatch the guard `return None`s, which used
  to fall through to the PROVIDER path and silently export `f -> u32` (a DIFFERENT type than the imposed
  world declares). Now an **imposed-world contract guard** in the export dispatch declines loudly: when
  `wit_world.is_some()` and an export result reaches the generic `u32`-handle provider path as a COMPOUND
  (`abi_val_type` None + `extern_abi_val_type` Some), it declines instead of mislabeling. This closes the
  WHOLE class (any declared-typed compound export member the typed paths can't emit, not just enum-reorder).
  A component-name-ONLY peer provider has `wit_world = None`, so its X5c compound-as-handle crossing is
  unaffected (verified: 29-* peer list/tuple/map/set cases still PASS).
- **[emit, export] typed enum RESULT on a fully-SYNTHESIZED world (NO clause at all) — remaining slice.**
  With no world, there is no declared `WitType::Enum`, so `record_interface_export` isn't reached and the
  program falls back to run/encode (SHAPE 58/59). Closing it needs the SYNTHESIZED-world builder to derive
  an `enum` member result from the guest (db-aware `enum_cases`), then the SHAPE-60 lower applies.
- **[emit, export]** a NON-SCALAR entry PARAM (enum/Sum, and a record whose fields aren't all
  boundary-scalar) — `try_bare_entry_param_component` + `is_boundary_record`/`field_boundary_abi`
  decline it: *"parameter … has no scalar boundary representation — a non-scalar entry parameter is not
  yet emitted on this export path"* (verified: an enum export param, and a record with an enum field,
  both decline `todo`). The param twin of Direction A above.
- **[emit, export] typed `result<ok,err>` EXPORT result — ✅ DONE (SHAPE 74/75).** A `result<s64,s64>`
  (74) and a compound-payload `result<record{lo,hi}, s64>` (75) EXPORT result now cross: `canon_write_of`
  gained a Result arm (a 2-variant both-payload sum → `CanonWrite::Variant`, mapping guest `Ok`→boundary
  disc 0 / `Err`→1 BY NAME, payload written recursively at the canonical result layout), reusing the
  existing `CanonWrite::Variant` emit (SHAPE 61) with no new writer. Both `(live-objects known-leak)`
  (SpillRecord-result reclaim class). REMAINING: a `result<_, E>`/`result<T, _>` with a NULLARY arm.
- **[emit, export]** top-level Tuple/Sum/List/String/Bytes typed-interface PARAM
  (`record_interface_export`); a flat
  1-value record result; a nested/compound list-param
  element on the bare-entry path; a `result<>` bare-entry param.

**Design-level (no WIT boundary form on either side; needs a design decision — TRACK, don't rush):**
- **[design]** BigInt, Rational, exact-`Qty`, Map, Set, Symbol.

## By design — NOT gaps

- **bare-effect** path is scalar/unit-only for results (the world-driven path is the compound
  envelope). State it; don't "close" it.
- **peer-bound** crosses any compound as an opaque `u32` handle (`extern_abi_val_type`) — no
  structural marshal is intended.

⚠️ **Behavior wart (NO-`wit-world`-clause only now):** a NO-clause guest whose export result is a
compound (record/sum/list) falls back SILENTLY to the run/encode envelope, whereas a non-scalar export
PARAM declines LOUDLY (`todo`). Asymmetry: on the no-clause path the result-side silently degrades while
the param-side surfaces the limitation. (The IMPOSED-world result-side silent degrade to `u32` is now
FIXED — SHAPE 64 — it declines loudly; only the fully-synthesized no-clause path still run/encode-degrades.
The no-world synthesized typed export, below, closing it would remove the last silent fallback.)

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
- typed enum RESULT export under a declared world (Direction A) — `record_result_lower` payloadless-enum
  arm → `Passthrough` i32 + `needs_result_wrapper`; crosses as WIT `enum{…}` not `u32` (SHAPE 60,
  WIT-dump verified).
- variant-with-payload RESULT export under a declared world — already WIRED (SpillRecord +
  `canon_write_of` variant arm); now VERIFIED (SHAPE 61, WIT-dump `variant t0 { continue, close(s64) }`).
  No emit change — a previously-untested cell now pinned.
- TUPLE RESULT export (bare, and as a variant/record payload) under a declared world — `canon_write_of`
  gained a `Ty::Tuple` arm (positional twin of the Record arm, reuses `CanonWrite::Record`); crosses as
  WIT `tuple<…>` not `u32` (SHAPE 62 bare tuple result, SHAPE 63 variant-with-tuple-payload). This also
  unblocks a variant/record whose payload/field is a tuple (the variant/record arm recurses here).
- Remaining enum/variant export slice: the NO-WORLD SYNTHESIZED enum/variant export (guest annotates a
  sum result with no world clause) — the enum result diverts to the resource-escape / provider path
  before `try_bare_entry`, so it falls to run/encode; a proper multi-path trace is needed (deferred).

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
