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

- **bare** — a plain `(effect …)` with NO imposed world: `!allow_option_bytes && !peer_bound`.
  Scalar/unit results only; scalar/unit/string/bytes arguments only. The compound envelope is NOT on
  this path when no world is imposed.
- **world** — the world-driven path: `allow_option_bytes && !peer_bound`. `allow_option_bytes` (set in
  `backend/wasm/mod.rs`) engages when EITHER (a) a component + a `wit_world` with a bytes-crossing or
  typed-record EXPORT (the reducer / typed-interface path), OR (b) the imposed `wit_world` declares an
  IMPORT interface (`world_has_import_interface`) — so a CUSTOM import-only world engages the compound
  envelope too, not just the built-in reducer/platform interfaces (v-wit-boundary B0). This is the
  compound envelope. The COMPOUND host-import RESULT now crosses on BOTH the reducer/typed-interface
  emit AND the plain host-delegating envelope (`assemble_host_runtime{,_mem}` / `assemble_host{,_mem}`,
  which declare the result's WIT defined-type via `build_host_result_types` + a shared-mem realloc;
  v-wit-boundary B1, SHAPE 83).
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
| Tuple (all leaf-liftable; TOP-LEVEL arg = scalar OR `Bytes` OR nested-tuple-of-scalars OR record-of-scalars elements) | arg + result + field + TOP-LEVEL arg (scalar+Bytes+nested-tuple+record elems) | world | `result_is_liftable` (Tuple); arg: `HostParam::Tuple` → built-in `tuple<T…>` via per-param CRef + `emit_tuple_reg_flatten` (positional; a Bytes element copies its rope to `mem` at the shared cursor; a NESTED tuple element recurses inline; a RECORD element recurses `emit_record_arg_marshal` with the element's WIT threaded via `tuple_wit` + the abi reordered to WIT order, `arg_is_boundary_tuple`) | 33, 34, 98 (top-level all-scalar arg), 100 (record field), 105 (tuple&lt;bytes,s64&gt; arg), 108 (two Bytes elems, disjoint mem), 109 (Bytes-scalar-Bytes interleave), 110 (empty Bytes element), 134 (nested tuple&lt;tuple&lt;s64,bool&gt;,s64&gt; arg), 135 (record&lt;lo:s64,hi:bool&gt; element, load-bearing reorder) |
| Record (all fields boundary/leaf, incl. nested + WIT-order reorder + a `result<bytes,enum>` FIELD in the guest→host ARG direction) | arg + result + export | world/export | `is_boundary_record` / `result_is_liftable` (Record) / `record_interface_export` / `emit_record_arg_marshal`'s `RecordFieldAbi::Result` field arm | 11, 13, 19, 20, 21, 25, 29, 31, 35, 36, 215 (result&lt;bytes,enum&gt; field, guest→host ARG) |
| compound host-IMPORT RESULT (record/string/bytes/list/tuple/option/result/variant) on the PLAIN host-delegating envelope — a CUSTOM import-only `wit_world`, no typed export, plain top-level guest export | result | world (plain-envelope) | `world_has_import_interface` + `result_is_liftable` (`build_host_result_types` declares the WIT type; `needs_realloc` mem shape) | 83 |
| MULTI-PARAM host op (≥2 top-level WIT params, e.g. `f(list<u8>, s64) -> s64`) | arg | world (plain-envelope) | `collect_host_imports_at` classifies EACH arg into `params` (no arity limit); per-call scratch cursor threaded across args (incl. TWO compound args, each copying a rope to disjoint mem) | 111 (bytes+scalar), 112 (tuple&lt;bytes&gt;+option&lt;bytes&gt;) |
| option&lt;scalar\|bytes\|leaf-liftable&gt; + option&lt;tuple\|record of scalars&gt; (record-ARG FIELD) | field + result + TOP-LEVEL arg (scalar OR bytes payload) | world | `option_payload_ty` (arg: `HostParam::Option` → built-in `option<T>` via per-param CRef + `emit_option_reg_flatten`; a Bytes payload copies its rope to `mem` on Some; a tuple/record-of-scalars FIELD payload scratch-flattens via `emit_record_arg_marshal`) | 8, 16, 35, 36, 38, 97 (top-level scalar arg), 99 (record field), 106 (option&lt;bytes&gt; Some arg), 107 (option&lt;bytes&gt; None arg), 123 (option&lt;tuple&gt; field), 124 (option&lt;record&gt; field) |
| result&lt;list&lt;u8&gt;, enum&gt; | arg + result | world | `result_bytes_enum` | 15, 17 |
| variant (scalar / mixed-width join / compound payload) + payloadless enum | arg + result | world | `variant_scalar_payload_cases` / `variant_liftable_payload_cases` / `enum_cases` | 18, 32, + vres/cvp/mwv/wen families; PLAIN-envelope host RESULT: 90 (enum), 91 (variant) |
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

- (none — `Qty`-over-scalar arg + result are now pinned: SHAPE 237 Qty<Int64> arg, 238 Qty<Int64> result,
  239 Qty<Float64> arg; `abi_val_type` peels `Qty{inner}` to the inner scalar, no code.)

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

**Emit side — v-wit-boundary (custom import-only wit-world, plain host-delegating envelope):**
- **[emit, ARG] a NOMINAL/compound host-op ARGUMENT (record/enum/bare-variant param) on the PLAIN
  host-delegating envelope — ✅ DONE (B3, SHAPE 92).** The world-imposed plain path now routes through
  `build_host_group` (the SAME per-interface computation the reducer/bytes-provider path uses), which
  declares the nominal-arg WIT type in the host import instance-type (`record_defs`, threaded through the
  four plain assemblers into `host_effect_instance_type`) and bakes the nominal-arg type index into the op's
  component functype; the guest flattens the value-heap record/enum/variant into the op's core slots via the
  existing `emit_record_arg_marshal` / `emit_variant_reg_flatten` marshals. The former decline-don't-
  miscompile arg-guard is GONE. A shape `build_host_group` cannot marshal (single-nominal-kind mix,
  string+record, multi-record) still declines INSIDE it — decline-don't-miscompile preserved. The no-world
  bare-effect branch keeps the flat scalar/string/`list<u8>`-arg path (the top guard declined compound args
  there since `allow_option_bytes` is false without a world). The RESULT side was already DONE (B1).
  Verified end-to-end on the plain path (import `cadenza:platform/probe` + bare `run: func()` export, host
  stub matched): RECORD arg (SHAPE 92), scalar-payload bare-VARIANT arg (SHAPE 93), `list<s64>` arg
  (SHAPE 94). **A payloadless (all-nullary) ENUM host-op ARG now REACHES the plain path (re-verified tick
  20 on HEAD) — the tick-18 resource-escape reification is NO LONGER REPRODUCIBLE.** `(effect probe (op emit
  (-> Col Int64)))` + `(host (probe) (probe.emit (Col.Red)))` against an imposed world declaring
  `emit: func(param c (enum red green blue)) -> s64` lowers to `Core::HostCall` (trace: `apply: host-delegated
  perform → Core::HostCall (sync import / plain host-delegation)` at compute.rs:1293 — so `is_world_import_op`
  returns TRUE, NO reify), and the emitted component imports `cadenza:platform/probe` with the enum arg
  crossing as a properly-cased WIT `enum host-record-p0 { red, green, blue }` (NOT `cadenza:run/run` +
  `resource t`). So the tick-18 diagnosis (an all-nullary enum arg reifies to the resource-escape form) was
  either fixed by an intervening B3 landing or predicated on a subtly different repro; on HEAD the enum arg
  host-delegates like the record/variant/list/scalar args. RESIDUE (cosmetic): the reflected arg type is named
  by the generic `host-record-p<n>` scheme, so an ENUM arg gets an ENUM type MISNAMED `host-record-p0` — it is
  a real WIT `enum` with the right cases (structural WIT match at link is by case-set, so a conforming host
  still satisfies it), only the type NAME is a misnomer. FOLLOW-ON (low-pri, non-blocking): un-park the
  enum-arg corpus case and gate the VALUE round-trip (untested here — the plain-path host-response should now
  match, unlike the tick-18 reflection-envelope form that never matched); optionally rename the enum arm's
  reflected type off the `host-record-p` prefix. v-hivemind uses records/options, not bare enums as args.
- **[emit, RESULT] a payloadless ENUM host-op RESULT crossing BY VALUE on the plain host-delegating
  envelope — ✅ DONE (SHAPE 90).** Rides the SAME `result_crefs[i]` path as a spilled compound:
  `build_host_result_types` already maps `enum_result` to the enum's nominal `enum` DEFINED+EXPORTED type
  (`host_import_functype` keeps the core result a bare i32), so removing the plain-path enum-result decline
  guard sufficed — no extra emit. WIT-dump verified `enum host-result-t0 {…}`. The plain-path twin of `wen1`
  (which crossed the same shape only via a typed record-interface export).
- **[emit, RESOURCE-ESCAPE]** the resource-escape entrypoint form — a host result escaping DIRECTLY as
  the guest export result (`run()->String = host sim in (sim.echo "hi")`, v-hivemind's literal repro) —
  still declines on `assemble_host_runtime_resource*` (scalar/unit host ops only; a String-param or
  compound result declines). Needs the same instance-type + `needs_realloc` mem threading B1 applied to
  the plain envelope, across the 5 resource-escape assembler variants (B2). **A COMPOUND host RESULT now
  declines CLEANLY (CDZ0900) at both resource-escape sites (`mod.rs` ~2528 and ~5315, next to the existing
  String-PARAM guard) — decline-don't-miscompile.** Before this guard a scalar-param host op with a
  compound (e.g. String) result laid the result-lift op without declaring it, so the lift resolved to an
  out-of-range func index and the component failed wasm validation ("unknown function"). SHAPE 95 pins the
  idealistic escaping-String behavior as a corpus TODO (grades Todo via CDZ0900, auto-locks to Pass when
  B2 lands).
- **[naming, B1b] the PLAIN host-delegating envelope now names the host import by the world's FQ import
  interface — ✅ DONE.** `world_import_iface_for_effect(db, effect)` reverse-maps the guest effect (named
  after the interface's SHORT kebab segment by `synthesize_world_import_effect_decls`) to the world IMPORT
  interface's FULL name (`cadenza:platform/probe`), used as the component import extern name — matching what
  the world declares + what a host provides (as the reducer bytes-provider path already does). Falls back to
  the effect name with no imposed world (byte-identical). WIT-dump verified (`import cadenza:probe/probe`).
  REMAINING: the 5 resource-escape assembler sites (`assemble_host_runtime_resource*`) still name by effect —
  fold this same helper in when B2 wires compound results there.
- **[emit]** a multi-payload variant case (≥2 payloads) RESULT is ✅ **DONE — SHAPE 122**: a guest ctor with
  ≥2 payloads (`(type V (Pair Int64 Int64) …)`) crosses as a WIT variant case with a `tuple<…>` payload
  (`canon_write_of`'s Variant arm resolves the case payload via the ctor + `payload_ty_at_instantiation`, which
  yields the tuple of the ctor's payload types, then writes it through the Tuple arm). REMAINING: the multi-
  payload case at the ARG (register-flatten) position (the mixed int↔float / f32↔f64 single-payload variant is
  now ✅ DONE — SHAPE 233/234/235, `HostParam::VariantScalarsMixed`).
- **[emit]** compound variant payload at the ARG (register-flatten) position; compound-payload
  variant list-element.
- **[emit, ARG-side]** `option<compound>` host-op record-ARG FIELD — ✅ scalar/bytes (pre-existing) + **tuple-of-scalars (SHAPE 123)** + **record-of-scalars (SHAPE 124)** + **record-with-a-Bytes-field (SHAPE 126)**. `field_boundary_abi` recurses the payload; `emit_record_arg_marshal` SCRATCH-FLATTENS it (`(disc, flatten(payload))` — disc + one core slot per scalar payload field / TWO `(ptr,len)` slots per Bytes field, marshalled into N scratch slots since LIR blocks are single-value, pushed after the `if`; the Some arm recurses `emit_record_arg_marshal` on the payload record and captures its N pushed slots in reverse, the None arm zero-fills; a record payload reads each WIT field from its name-lex cell index, `reorder_record_fields_to_wit` recursing the `Option(Record)` to WIT order; a Bytes payload leaf copies its rope into shared mem at the reserved scratch cursor, `record_has_option_bytes_field` reserving the cursor in the emit.rs pre-scan). NB: the slot count checks `Ty::Bytes` BEFORE `valtype_of` (which is `Some(I32)` for a Bytes handle) so a byte leaf counts as 2 slots, not 1. The `option<compound>` LIST ELEMENT (`list<option<compound>>`) is now DONE — SHAPE 152/153/154 (see the list-element entry below). REMAINING: a nested-compound (option/tuple/record-of-compound) payload field inside the option that is not yet exercised. The RESULT side is DONE — SHAPE 66.
- **[emit, ARG-side]** `option<compound>` host-op TOP-LEVEL arg (the bare param position, not nested in a record) — ✅ scalar/bytes (pre-existing) + **tuple-of-scalars, Some + None (SHAPE 127/128)** + **tuple-with-a-Bytes-element, Some (SHAPE 129)** + **record-of-scalars, Some + None (SHAPE 130/131)** + **record-with-a-Bytes-field, Some + None (SHAPE 132/133)** + **record-with-a-list<scalar>-field, Some + None (SHAPE 148/149)** + **record-with-a-nested-record field, Some (SHAPE 150)** + **record-with-an-option<scalar> field, Some (SHAPE 151)**. The option<record> arm now admits ANY payload record `is_boundary_record` accepts (the SAME `field_boundary_abi` admit set the direct record ARG uses), so a compound field (a `list<s64>`) crosses at the option-arg position exactly where it crosses at the bare-record-arg position: `option_arg_crosses` (the single shared gate for the classifier, `first_unrepresentable_host_op`, the emit dispatch, and `used_ops` — all widened in lockstep) tests `is_boundary_record` on the payload; the classifier builds the payload record abi via `field_boundary_abi` per field; `emit_option_reg_flatten`'s record branch derives its capture `slot_vts` from each field's flattened boundary ABI (`field_boundary_abi` → `flatten_record_field_abi`, mapped back to `ValType` via `ValType::from_byte`), so a `list<s64>` field counts as its 2 `(ptr,count)` slots — a `valtype_of`-based count treated the list handle as one i32 and left a value on the operand stack (CDZ0910); the emit.rs cursor pre-scan reserves the scratch cursor for an `option<record-with-a-list/tuple/bytes-field>`, and the cursor-slot reservation now bumps the declared-locals top to `slot + 1` (a cursor-only reservation formerly excluded it, panicking `coalesce_func`'s remap). The record branch of `emit_option_reg_flatten` takes a `payload_wit` (threaded from the caller's `wit_params[arg_i]`) and recurses `emit_record_arg_marshal` (WIT-order field push); the classifier REORDERS the payload record abi to WIT order (`reorder_record_fields_to_wit`) so the `(option (record …))` component type + core flatten agree with the marshal — SHAPE 130 uses distinct field widths (s64 lo, bool hi) so a missed reorder fails instantiation (proven: the pre-reorder build hit an `expected (i32 i32 i64) / found (i32 i64 i32)` codegen defect). `emit_option_reg_flatten`'s tuple branch: the component type + serialize flatten are already general over the payload abi (built from the declared WIT type / `flatten_record_field_abi`), so only the guest marshal + the two lockstep classifiers (`collect_host_imports_at`'s `HostParam::Option` arm + `first_unrepresentable_host_op`'s `arg_is_boundary_option`) + the emit.rs cursor pre-scan + `used_ops` were scoped. On Some it flattens the payload tuple POSITIONALLY via `emit_tuple_reg_flatten` (one core slot per SCALAR element, `(ptr,len)` = TWO slots per `Bytes` element with the rope copied into shared mem at the reserved cursor), captured into N scratch slots and pushed as `(disc, elem…)` after the single-value `if`; None zero-fills. Two traps: (a) `valtype_of`-is-`Some(I32)`-for-a-tuple → the tuple branch precedes the scalar branch, and the element guard is `abi_val_type OR Bytes` (not `valtype_of`, which would wrongly admit a nested-compound element); (b) a Bytes element expands to 2 scratch slots (the byte-leaf slot-count pin, same as the record byte-leaf field), and the cursor pre-scan reserves for an `option<tuple-with-bytes>` arg. A scalar-payload `variant` field inside the option payload record is now DONE — TESTED at SHAPE 173 (`option<record{ v: variant{go, stop(s64)}, n: s64 }>`, the Some arm: `field_boundary_abi`'s Variant arm admits it, `emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`'s variant-field arm, flattening to `(opt-disc, var-disc, join, n)`). A `result<bytes,enum>` field inside the option payload record is now DONE — TESTED at SHAPE 216 (`option<record{a: result<list<u8>,enum>, k: s64}>`, the Some arm): `option_arg_crosses` admits it via `is_boundary_record`, and `emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`'s Result-field arm; the cursor reservation rides `record_has_result_field` on the option payload record (added in #9923 alongside the direct-record twin SHAPE 215 — a missing reservation panicked the marshal's `cursor.expect(...)`). REMAINING: a deeper-nested payload field (a variant field with a NON-scalar payload, an `option<compound>` field). The `option<compound>` LIST element (`list<option<record>>` / `list<option<tuple>>`) is now DONE — SHAPE 152/153 (see the list-element entry below).
- **[emit, ARG-side] a RECORD host-op ARG with a scalar-payload `variant` FIELD — ✅ DONE (SHAPE 166).** The
  variant FIELD rides the SAME `field_boundary_abi` Variant arm the bare-variant ARG (SHAPE 93) / a
  list-element variant uses; `emit_record_arg_marshal`'s variant-field arm flattens it to `(disc, payload-join)`
  via the shared `emit_variant_reg_flatten`, joining the record's core run; `collect_record_field_ops` declares
  the variant arm's ops in lockstep. Already reachable (the three sites were widened for the variant algebra);
  SHAPE 166 locks in the value round-trip that was previously untested.
- **[emit, ARG-side] a top-level `tuple<…>` host-op arg — ✅ DONE for the whole element algebra (SHAPE 139-147).**
  A bare `tuple<T…>` arg crosses as the built-in WIT `tuple<T…>`; the guest flattens the value-heap tuple
  POSITIONALLY via `emit_tuple_reg_flatten`. Element coverage (each with its marshal arm): a SCALAR (inline
  slot), a `Bytes` leaf (`(ptr,len)` rope→mem at the cursor), a nested `tuple` (recurse; SHAPE 134, +Bytes leaf
  139), a `record` (recurse `emit_record_arg_marshal`, whose fields cross via the shared `field_boundary_abi` —
  so a record element carries ANY field that builder accepts: scalar/`Bytes`/nested-record/list/tuple/option/
  result; SHAPE 135/140/141/142/143), a `list<T>` (whose element crosses at the boundary, marshalled by
  `emit_list_arg_marshal`; SHAPE 144), an `option<T>` (payload scalar/`Bytes`/tuple/record via
  `emit_option_reg_flatten`; SHAPE 145/146), and a scalar-payload `variant` (via `emit_variant_reg_flatten`;
  SHAPE 147). The gate is the single `tuple_arg_crosses(db, ty)` helper (keyed to the marshal's element
  capability), used by BOTH `first_unrepresentable_host_op` and the `collect_host_imports_at` classifier arm, so
  they stay in lockstep by construction; `tuple_arg_needs_cursor` reserves the scratch cursor for ANY
  rope-copying leaf (Bytes/list/result/option<bytes>) anywhere in the tuple tree. REMAINING: an option/variant
  element whose payload is itself a nested compound beyond the shared marshals' reach, and the compound-payload
  variant element (both roll into the general compound-variant-payload-at-ARG gap above).
- **[emit] typed `list<COMPOUND>` EXPORT result element — ✅ DONE (SHAPE 69/70/71/72/73).** A typed
  `list<tuple<s64,s64>>` (69), `list<record{lo,hi}>` (70), NESTED-element `list<tuple<s64, list<s64>>>`
  (71), `list<variant{lo,hi(s64)}>` (72), and `list<tuple<s64, variant>>` (73) EXPORT result all cross by
  RECURSIVE `canon_write_of` composition with NO new emit (`CanonWrite::List` whose element is the
  Tuple/Record/Variant write, recursing into an inner `List`/`Variant` for a nested field). So EVERY
  compound list-element (record/tuple/list/variant, incl. a nested compound field) crosses on the RESULT
  side — the doc's old `list<...> element with a nested record/list/tuple/variant field` gap is CLOSED.
  The typed-export twin of SHAPE 7 (untyped run/encode). All are `(live-objects known-leak)` — the spilled
  list result + boxed elements are not reclaimed (the SpillRecord-result reclaim class, SHAPE 60/62/63;
  value-correct, routed to v-memory-safety). The ARG-side (host→guest) `list<COMPOUND>` element is ALSO ✅ DONE:
  `list_elem_marshalable` + `emit_list_arg_marshal` write a `list<record>` (SHAPE 30/39), `list<tuple>` (SHAPE
  33), `list<option<scalar>>` (SHAPE 38), `list<option<record>>` / `list<option<tuple>>` (SHAPE 152/153), `list<option<bytes>>` (SHAPE 154), `list<option<list>>` (SHAPE 162), `list<option<option<scalar>>>` (SHAPE 224), `list<option<option<bytes>>>` (SHAPE 225), `list<option<option<record>>>` (SHAPE 226), and
  `list<variant<scalar>>` (SHAPE 43/52) element IN PLACE at its canonical layout, recursing for a nested
  `list<list<…>>`. The `list<option<COMPOUND>>` element writes the payload at the option's payload offset via
  `emit_option_to_mem`'s branches: a RECORD/TUPLE payload (SHAPE 152/153) via `emit_record_to_mem`/`emit_tuple_
  to_mem` (the same product writers `list<record>` uses), threading the running spill cursor + the payload
  record's WIT (from the element's `option<…>` WIT); a `Bytes` payload (SHAPE 154) copies its rope at the cursor
  and writes `(ptr,len)` at the payload offset; a `list` payload (SHAPE 162) marshals via `emit_list_arg_marshal`
  and writes `(ptr,count)` at the payload offset (the list analogue of the Bytes branch). On None the payload area
  is left unwritten (a none option's payload is never read on lift). `list_elem_marshalable`'s option arm was
  widened from scalar-only to also admit a `Bytes`/`list` payload + a record/tuple payload whose fields are
  `product_field_marshalable`, in lockstep with `emit_list_arg_marshal`'s `option_elem` detector +
  `collect_list_elem_ops` (recurses the payload's field ops).
  A `list<result<list<u8>, enum>>` element is ALSO ✅ DONE — SHAPE 163: a new in-place writer `emit_result_to_mem`
  writes each element (disc byte + payload at `align_up(1,4)`; Ok copies the Bytes rope at the cursor + writes
  `(ptr,len)`, Err writes the err enum's disc), `list_elem_marshalable` gained a `result_bytes_enum` arm, and the
  element dispatch + `collect_list_elem_ops` widened in lockstep. An `option<option<scalar>>` list element
  (`list<option<option<scalar>>>`) is now ✅ DONE too — SHAPE 224: `emit_option_to_mem` gained a nested-option arm
  that on the outer Some fetches the inner option (`sum-payload`) and RECURSES itself at `dest + payload_off`
  (the inner recursion writing the inner disc + its scalar payload inline), the `option_elem` detector +
  `list_elem_marshalable`'s option arm + `collect_list_elem_ops` all admitting a nested-option payload in lockstep
  (the payload is itself option-shaped → recurse). A BYTES inner (`list<option<option<bytes>>>`) is now ✅ DONE too
  — SHAPE 225: NO new code — the SHAPE 224 nested arm recurses into the inner option's Bytes arm (rope copied at
  the spill cursor, which the list-arg pre-scan reserves UNCONDITIONALLY for any `Ty::List` arg), and
  `collect_list_elem_ops`' nested-option recursion reaches the inner `bytes-len`/`bytes-get`. A RECORD inner
  (`list<option<option<record>>>`) is now ✅ DONE too — SHAPE 226: NO new code — the nested arm recurses into the
  inner option's Record arm (`emit_record_to_mem` writes the product in place, WIT-ordered), and
  `collect_list_elem_ops` reaches the inner `arr-get` + per-field ops. REMAINING (ARG-side only): a mixed int↔float
  variant element (rolls into the compound-variant-payload / mixed-join gaps above); a list/tuple INNER
  (`list<option<option<list/tuple>>>`) — the same nested arm covers it, un-pinned.
- **[emit, register-path] a top-level `result<list<u8>, enum>` host-op ARG — ✅ DONE (SHAPE 164/165).** The
  register twin of the `result` record FIELD (SHAPE 17) and the list ELEMENT (SHAPE 163), now at the bare param
  position. A new `HostParam::Result(err-cases)` variant + a `result_bytes_enum` classifier arm; the guest flattens
  the value-heap Result to `(disc:i32, i32, i32)` via a new `emit_result_arg_reg_flatten` (the record-FIELD result
  flatten minus the `arr-get`): Ok copies the `list<u8>` payload rope into `mem` at the running cursor → `(0, ptr,
  len)`; Err reads the err enum payload's `sum-disc` → `(disc, err-enum-disc, 0)`. The `(result (list u8) (enum …))`
  component type builds from the world's declared WIT via `add_wit_type_deduped`; the core functype adds 3 i32 slots
  (`host_import_functype`). Widened in LOCKSTEP: `first_unrepresentable_host_op`'s `arg_is_boundary_result`, the emit
  dispatch (+ the Owned/dup-site reclaim drop), `collect_used_ops`'s result-arg arm (`sum-disc`/`sum-payload` +
  `bytes-len`/`bytes-get` + `drop`), the emit.rs cursor pre-scan, `host_imports.rs` (the structural-CRef param
  reference + `has_list_param` + `host_param_abi` decline), and — the load-bearing fix — `set_needs_memory` (a
  result arg copies a rope into `mem`, so the host set routes to the `_mem` assembler; without it the host op lower
  was emitted memoryless → CDZ0910 "canonical option `memory` is required"). REMAINING: a `result<record,enum>` /
  `result<_, variant>` (`result_bytes_enum` requires a `list<u8>` Ok + a payloadless-enum Err).
- **[emit, register-path] a top-level `result<scalar, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE 186/187/188
  int; 209/210 float).** The scalar-Ok sibling of the Bytes-Ok result arg: a `HostParam::ResultScalar(ok-abi,
  err-cases)` (detector `result_scalar_enum`, admitting any scalar Ok + a payloadless-enum Err). It flattens to
  just 2 slots `(disc:i32, join)` with NO `mem` (no rope): `emit_result_scalar_arg_reg_flatten` reads the result
  disc, unboxes the Ok scalar into the join slot on Ok, reads the err enum's disc into it on Err. The join slot is
  `i64` iff the Ok scalar is 8-byte (an `i64` OR an `f64`), else `i32`; the `i32` err disc widens via
  `i64.extend_i32_u` for an i64 join. SHAPE 186/187 (s64 Ok, `(param i32 i64)`) vs 188 (bool Ok, `(param i32 i32)`)
  pin the int widths + both arms. **FLOAT Ok (SHAPE 209/210):** the canonical join for a float is the reinterpret
  lattice — `join(f64,i32)=i64` / `join(f32,i32)=i32` — so `result_scalar_enum` now admits a float Ok, and the Ok
  arm bit-REINTERPRETS the float payload into the (integer) join slot (`I64ReinterpretF64` for f64 → the `(param
  i32 i64)` sig, `I32ReinterpretF32` for f32 → `(param i32 i32)`); the host lift reads the join int back as the
  float. `serialize` keys the join width off the Ok's core width (i64 for f64, not just `== i64`). SHAPE 209
  (`result<f64>`) + 210 (`result<f32>`, the Ok payload annotated `(: 1.5 Float32)` since a bare literal is f64).
  Widened in LOCKSTEP: the classifier arm, `first_unrepresentable_host_op`'s `arg_is_boundary_result_scalar`, the
  emit dispatch (+ Owned/dup-site reclaim drop), `collect_used_ops` (`sum-disc`/`sum-payload` + the Ok unbox
  `get-*` — `get_op_ty` returns `get-float`/`get-float32` for a float — + `drop`), `serialize`, and `host_imports.rs`.
  NOT the cursor pre-scan and NOT `set_needs_memory` (no rope → no `mem`). The float reinterpret join is now DONE
  across all three carriers — `result<scalar>` (209/210), `result<tuple>` slot-0 (211/212), `result<record>`
  slot-0 incl. WIT-reorder (213/214). REMAINING: `result<_, variant>` (variant err arm — every result detector
  gates the err to a PAYLOADLESS enum; a variant err needs a new `HostParam` variant carrying the err variant's
  payload ABIs + the multi-slot err-flatten join, where the Err payload contributes ≥2 slots (variant disc +
  payload) so the result join is `[result-disc, join(ok-slot, variant-disc), variant-payload…]`). ⚠ TRAP
  (verified tick, nix value gate): a `result<scalar, variant-err>` host-op ARG does NOT decline cleanly and does
  NOT cross — it REIFIES via the resource-escape value-form envelope (the call returns a `#record((= kind
  "effect/probe") (= payload b"cdzast…") (= schema_descriptor …))` instead of performing the host op). `xtask gate
  --opt-sweep` is FOOLED: the reify record is consistent across O0..O3 so the case "checks", but the nix VALUE gate
  catches it (expected 42, got the reify record). So a variant-err result reads as "checked" under opt-sweep yet is
  NOT a real WIT crossing — always confirm a new result/variant boundary shape with the nix value gate, never
  opt-sweep alone. See `[[wit-boundary-opt-sweep-fooled-by-reify-value-form-fallback]]`.
- **[emit, register-path] a top-level `result<record-of-scalars, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE
  189/190/191).** The record-Ok sibling of the scalar-Ok result: a new `HostParam::ResultRecord(ok-fields, err-
  cases)` (detector `result_record_enum`, admitting a record every field of which is a SCALAR + a payloadless-enum
  Err; a compound Ok field is a later increment — the in-mem marshal). It flattens to `(disc:i32, field0, field1,
  …)` — the discriminant then the Ok record's fields POSITIONALLY in WIT declaration order, the `i32` err disc
  riding the FIRST field's slot on Err. Because every field is a scalar, each is ONE slot and joining the first
  with the `i32` err disc never widens past that field's own width, so the slot widths ARE the record's field
  widths — NO `mem`. `emit_result_record_arg_reg_flatten` recurses `emit_record_arg_marshal` on Ok (its N pushes
  captured into the slots in reverse, WIT-reordered) and puts the err enum's disc in slot 0 on Err. SHAPE 189/190
  (`record{x:s64,y:s64}`, core `(param i32 i64 i64)`, both arms) + 191 (`record{a:bool,b:s64}`, mixed
  `(param i32 i32 i64)`) pin the multi-slot join + per-field widths. Widened in LOCKSTEP: the classifier arm (which
  reorders the Ok fields to the result WIT's Ok record order), `first_unrepresentable_host_op`'s
  `arg_is_boundary_result_record`, the emit dispatch (+ reclaim), `collect_used_ops` (`sum-disc`/`sum-payload`/
  `arr-get` + each field's unbox + `drop`), `serialize` (disc + each field's flattened slots), and `host_imports.rs`
  (structural-CRef param reference + `host_param_abi` decline). An all-scalar Ok record is register-only (no
  `mem`). **WIDENED (SHAPE 202/203):** the Ok record may now have ANY boundary field (`result_record_enum` admits
  `is_boundary_record` — the SAME field set the direct record arg uses). A Bytes/list field marshals into `mem`:
  `emit_result_record_arg_reg_flatten` now threads a `cursor` to `emit_record_arg_marshal` (whose Bytes/list arms
  copy into `mem`), so `set_needs_memory` (per-field, like the direct record arg) + the emit.rs cursor pre-scan
  admit it, and `collect_used_ops` declares the field ops via `collect_record_field_ops` (the scalar-only
  `get_op_ty` missed `bytes-len`/`bytes-get` → CDZ0910 u32::MAX — the bite this increment fixed). The `(list u8)`
  type is built structurally from WIT (no `has_list_param` change; verified — modules validate + run,
  live-objects=0). **FLOAT FIELD — ANY position (SHAPE 207 non-slot-0; 213/214 slot-0):** a float field crosses in
  every WIT position. A float in a NON-slot-0 field rides its own `f64` slot (zero-filled on Err), core e.g.
  `(param i32 i64 f64)` (SHAPE 207). A float in the WIT-FIRST (slot-0) field bit-REINTERPRETS into the integer
  slot-0 join — `join(f64,i32)=i64` / `join(f32,i32)=i32`: `emit_result_record_arg_reg_flatten` overrides
  `slot_vts[0]` (WIT order) to the join int and emits `I64ReinterpretF64`/`I32ReinterpretF32` at the k==0
  reverse-capture, `serialize` emits the join int for slot 0. The reinterpret follows the WIT-REORDERED slot 0, not
  name-lex position — SHAPE 213 (`{a:f64,b:s64}`, no reorder) + 214 (guest `{a:s64,b:f64}` but WIT `(b:f64, a:s64)`
  → the name-lex-second `b:f64` reorders into slot 0 and is reinterpreted there). REMAINING (result family):
  `result<_, variant>` (err arm a variant).
- **[emit, register-path] a top-level `result<tuple, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE
  192/193/194 all-scalar; 204/205 compound element; 206 float non-slot-0 element).** The tuple-Ok sibling of the record-Ok result: a
  `HostParam::ResultTuple(elem-abis: Vec<RecordFieldAbi>, err-cases)` (detector `result_tuple_enum` + a
  payloadless-enum Err). It flattens to `(disc:i32, flatten(elem0), flatten(elem1), …)` — the discriminant then
  the Ok tuple's elements POSITIONALLY (element order, NO reorder — a tuple is positional, unlike the record's WIT
  reorder), the `i32` err disc riding the FIRST element's slot on Err. `emit_result_tuple_arg_reg_flatten` recurses
  `emit_tuple_reg_flatten` on Ok (N pushes captured in reverse) and puts the err enum's disc in slot 0 on Err (an
  integer/ptr first slot joins `i32` without widening). SHAPE 192/193 (`tuple<s64,s64>`, core `(param i32 i64 i64)`,
  both arms) + 194 (`tuple<bool,s64>`, mixed `(param i32 i32 i64)`) pin the all-scalar join + widths.
  **WIDENED (SHAPE 204/205):** the Ok tuple may now have ANY boundary element (`result_tuple_enum` admits every
  element `field_boundary_abi` does — the SAME element set the direct tuple arg uses, symmetric with the
  record-FIELD widening of SHAPE 202/203). `HostParam::ResultTuple` carries `Vec<RecordFieldAbi>` (was
  `Vec<AbiValType>`); the marshal derives its capture `slot_vts` from each element's `flatten_record_field_abi`
  (so a bytes/list element counts its `(ptr,len)` slots — a `valtype_of` count would treat the handle as one i32
  and leave a value on the stack, CDZ0910) and threads a `cursor` to `emit_tuple_reg_flatten` (a bytes/list/compound
  element copies its backing into `mem` at the cursor). `set_needs_memory` gets a per-element `ResultTuple` arm and
  the emit.rs cursor pre-scan reserves for a `result<tuple-with-a-compound-element>` (via `tuple_arg_needs_cursor`
  on the Ok tuple), and `collect_used_ops` declares the element ops via `collect_record_field_ops` per element (the
  scalar-only `get_op_ty` missed `bytes-len`/`bytes-get`/`vec-*` → CDZ0910 u32::MAX). SHAPE 204 (`tuple<s64,
  list<u8>>` bytes element) + 205 (`tuple<s64, list<s64>>` list element), both run + live-objects=0. Widened in
  LOCKSTEP: classifier, `first_unrepresentable_host_op`, emit dispatch (+ reclaim + cursor), `collect_used_ops`,
  `serialize`, `set_needs_memory`, `host_imports.rs` (all `Result*` arms — the structural-CRef `matches!` MUST list
  every `Result*` variant or a `ResultRecord`/`ResultTuple` param silently falls back to the wrong CRef → CDZ0910
  component-validation failure; this bit once when a codemod dropped `ResultRecord` from that `matches!`).
  **FLOAT ELEMENT — ANY position (SHAPE 206 non-slot-0; 211/212 slot-0):** a float element crosses in every
  position. Only slot 0 joins the `i32` err disc (the payloadless-enum Err flattens to a single `i32`; slots 1+
  have no Err counterpart and keep their own core type): a float in a LATER slot rides its own `f64`/`f32` slot
  (zero-filled `F64ConstBits(0)`/`F32ConstBits(0)` on Err), core e.g. `(param i32 i64 f64)` (SHAPE 206). A float
  FIRST element bit-REINTERPRETS into the integer slot-0 join — `join(f64,i32)=i64` / `join(f32,i32)=i32`:
  `emit_result_tuple_arg_reg_flatten` overrides `slot_vts[0]` to the join int and emits `I64ReinterpretF64` /
  `I32ReinterpretF32` at the k==0 reverse-capture (Ok arm), the Err arm stores the err disc into slot 0 (widened
  to i64 for f64), `serialize` emits the join int for slot 0, and `result_tuple_enum` now admits a float first
  element (the SHAPE 206 slot-0 decline is lifted). SHAPE 211 (`tuple<f64,s64>`, `(param i32 i64 i64)`) + 212
  (`tuple<f32,s64>`, `(param i32 i32 i64)`). REMAINING (result family): a FLOAT slot-0 field in a `result<record>`
  (adds the WIT-reorder — the tuple slot-0 done here is the positional counterpart; the record twin is next);
  `result<_, variant>` (err arm a variant).
- **[emit, MEM-path] a top-level `result<list<scalar>, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE 195/196).**
  The list-Ok sibling of the Bytes-Ok result: a new `HostParam::ResultList(err-cases)` (detector
  `result_list_enum`, admitting a `list<T>` whose ELEMENT is a scalar + a payloadless-enum Err; a `list<u8>` Ok is
  `Bytes` = `result_bytes_enum`'s job, and a `list<u8>` arg type is `Ty::Bytes` not `Ty::List` so this never sees
  it). It flattens to the SAME 3 slots as the Bytes result — `(disc:i32, ptr/errdisc:i32, count/0:i32)` — but Ok
  MARSHALS the value-heap list into `mem` (`emit_result_list_arg_reg_flatten` → `emit_list_arg_marshal`) instead
  of a rope copy, giving `(outer-ptr, count)`; Err gives `(err-enum-disc, 0)`. UNLIKE the register-only
  scalar/record/tuple results, this DOES need `mem` — `set_needs_memory` (grouped with the Bytes `Result`) + the
  emit.rs cursor pre-scan both admit it. SHAPE 195 (Ok `#list(3 4 5)`, core `(param i32 i32 i32)`) + 196 (Err, `(err
  disc, 0)`, live-objects=0 confirms no list leak on Err). Widened in LOCKSTEP: classifier, `first_unrepresentable_
  host_op`'s `arg_is_boundary_result_list`, emit dispatch (+ cursor + reclaim), the emit.rs cursor pre-scan,
  `collect_used_ops` (`sum-disc`/`sum-payload` + `vec-len`/`vec-get` + `collect_list_elem_ops` + `drop`), `serialize`
  (3 i32, same as Bytes), `set_needs_memory`, and `host_imports` (all five `Result*` arms).
  **WIDENED (SHAPE 197/198 → 199/200/201):** the element may be ANY element `list_elem_marshalable` accepts — the
  SAME element capability a bare `list<T>` arg uses: a scalar, an all-scalar product (`list<record>`/`list<tuple>`,
  197/198), a `Bytes` element that REACHES `list<u8>` (`list<list<u8>>`, 199), a nested `list` (`list<list<s64>>`,
  200), or an `option<…>` element (`list<option<s64>>`, 201). `emit_list_arg_marshal` writes each element
  identically whether the list is a bare arg or a result Ok arm, and the `result<list<T>, enum>` component type is
  built STRUCTURALLY from the declared WIT — so a `list<u8>`-reaching element needs NO `has_list_param` shared-
  `(list u8)`-type change (`ResultList` rides the structural-CRef path, verified: the modules validate + run,
  live-objects=0). The whole widening was a `result_list_enum` element-gate relaxation to `list_elem_marshalable`;
  the emit + used_ops already handled every element via the shared list marshal + `collect_list_elem_ops`.
  REMAINING (result family): a compound/float tuple element; a record with a compound field; `result<_, variant>`.
- **[emit, register-path] a top-level `option<variant>` host-op ARG — ✅ DONE (SHAPE 167/168).** The option
  payload is a scalar-payload `variant`: `option_arg_crosses` now admits it, and `emit_option_reg_flatten`'s
  variant branch flattens the value-heap option to `(opt-disc, var-disc, payload-join)` = the option disc + the
  payload variant's own `(disc, join)` flatten via the shared `emit_variant_reg_flatten` (the SAME helper the
  bare-variant ARG (SHAPE 93) / a record variant FIELD (SHAPE 166) uses); None zero-fills. The classifier builds
  `RecordFieldAbi::Option(Variant(cases))` via the shared `field_boundary_abi` Variant arm;
  `flatten_record_field_abi` already flattens `Option(Variant)` to the 3 core slots, and the `(option (variant …))`
  component type builds from the world's WIT. Widened in LOCKSTEP: `option_arg_crosses` (shared with the
  tuple-element-option gate), the classifier option arm, `emit_option_reg_flatten`, and `collect_used_ops`'s
  option-payload dispatch. No cursor needed (a scalar-payload variant does not touch `mem`).
- **[emit, register-path] a RECORD host-op ARG with an `option<variant>` FIELD — ✅ DONE (SHAPE 169/170).** The
  record-FIELD composition of the top-level `option<variant>` arg (SHAPE 167): `field_boundary_abi`'s option arm
  now admits a scalar-payload variant payload (→ `Option(Variant)`), so `is_boundary_record` accepts the record,
  and `emit_record_arg_marshal` gains an option<variant> field arm flattening the field to `(opt-disc, var-disc,
  payload-join)` via the shared `emit_variant_reg_flatten` (None zero-fills), placed BEFORE the option<scalar>
  arm (a variant handle's `valtype_of` is `Some(I32)`, so that arm's guard would else miscompile it).
  `flatten_record_field_abi` (`Option(Variant)` → 3 slots) and `collect_record_field_ops`'s option arm (recurses
  into the variant payload) were already general. No cursor needed (scalar-payload variant does not touch `mem`).
- **[emit] scalar-payload `variant` in the remaining COMPOUND positions — ✅ DONE / TESTED (SHAPE 171/172/173).**
  Lock-ins of reachable-but-untested variant shapes (the variant algebra was widened across the element/field
  sites in prior work; these pin the value round-trip): a `list<variant>` ELEMENT (171, `emit_variant_to_mem`),
  a `tuple<variant, …>` ELEMENT (172, `emit_variant_reg_flatten` positional), and an `option<record-with-a-
  variant-field>` ARG (173, `emit_option_reg_flatten`'s record branch → `emit_record_arg_marshal`'s variant-field
  arm). REMAINING variant gaps: a RECORD compound payload case in a MIXED variant (the TUPLE compound payload
  case in a mixed variant is now ✅ DONE — SHAPE 243, `VariantPayloadKind::Tuple`; the ≥3-payload-case mem join is
  ✅ DONE — SHAPE 242). (The mixed
  int↔float / f32↔f64 single-payload variant reinterpret join is now ✅ DONE — SHAPE 233/234/235; and a MULTI-payload
  case `b(s64,s64)` — one case with ≥2 payloads — is ✅ DONE, SHAPE 236: `variant_tuple_payload_case` admits n>=2
  since `variant_payload_ty_at` synthesizes the payload tuple, reusing `HostParam::VariantTuple` unchanged.)
- **[emit, ARG-side] a WIT `flags{…}` host-op ARG — ✅ DONE / TESTED (SHAPE 240).** The IMPORT twin of the
  export-side flags PARAM (SHAPE 113). The guest models flags as a PRODUCT record-of-bools (operator ruling), so
  the arg is a `Record{label: bool, …}` whose imposed WIT param is `flags`. Before, it declined ("a record
  host-arg has no matching WIT record type" — the classifier made `HostParam::Record`, found no WIT record).
  New additive `HostParam::Flags { field_bits, labels }`: the classifier consults `wit_params[arg_i]` (`Flags`) +
  `host::flags_field_bits` (each bool field's kebab name → its WIT-label bit, the PACK inverse of `param_field`'s
  flags-UNPACK arm; ≤32 labels, all-bool, count-match), the guest PACKS via `select::emit_flags_arg_pack` (per
  field `arr-get`+`get-bool`, shift into its bit, OR into the word), `serialize` flattens to `ceil(n/32)` i32
  words, and `host_imports` lays a nominal `flags` DEFINED type via the NEW `flags_params` branch (the enum-like
  single-leaf path — `is_nominal_cdef`/`export_remap` already handled `CDef::Flags`). Register-only (no `mem`).
  `used_ops` is already covered (the record-arg arm declares `arr-get`+`get-bool`+`drop`). NOT-A-GAP: >32 labels
  is a Component Model SPEC LIMIT — a `flags` type is capped at 32 labels (one i32; the validator rejects a
  >32-label flags: "cannot have more than 32 flags"), so a >32-label flags has no component boundary form and
  `flags_field_bits` correctly DECLINES it (decline-don't-miscompile), NOT a later increment. Flags at a
  record-FIELD ARG position is now ✅ DONE / TESTED (SHAPE 246): a new `RecordFieldAbi::Flags { field_bits, labels }`
  (the field twin of `HostParam::Flags`) is constructed in `reorder_record_fields_to_wit` (the one site with both
  the field abi + its WIT — converts a bool-record field whose WIT field is `flags` via `flags_field_bits_from_abi`,
  requiring `Scalar(AbiValType::Bool)` sub-fields so an int field can't masquerade); `flatten_record_field_abi` →
  `ceil(labels/32)` i32, `record_field_cref` → nominal `CDef::Flags`, `needs_memory`/`reaches_bytes` → false;
  `emit_record_arg_marshal`'s flags-field arm arr-gets the nested bool-record handle and `emit_flags_arg_pack`s it
  (used_ops already covers `arr-get`+`get-bool` via the generic record-field fallback). REMAINING: flags at a
  list-ELEMENT ARG position (`list<flags>`; the element path lacks a flags arm). The RESULT/param-lift side
  already unpacks flags — `param_field.rs`, `list_elem.rs`.
- **[emit, ARG-side] a COMPOUND (Bytes) single-payload variant host-op ARG — ✅ DONE / TESTED (SHAPE 227, 228).**
  (SHAPE 228 hardens the MULTI-Bytes-case path: `variant{a, b(bytes), c(bytes)}`, `bytes_discs = [1,2]`, exercises
  the marshal's multi-disc OR fold — SHAPE 227's single Bytes case left it untested.)
  A `variant{nullary…, bytes-case(s)}` bare top-level ARG now crosses via the NEW additive `HostParam::VariantBytes`
  (the scalar `HostParam::Variant` declines a Bytes payload — `AbiValType` can't express `list<u8>`). It mirrors
  the `result<list<u8>, enum>` family: the component `variant` DEFINED type is laid STRUCTURALLY from the declared
  WIT (`add_wit_type_deduped` → `CDef::Variant` with a `(list u8)` payload, export-remapped like a record — NOT the
  scalar variant's nominal-`AbiValType` builder), and `emit_variant_bytes_arg_reg_flatten` flattens to `(disc:i32,
  ptr:i32, len:i32)` — the SAME 3-slot Bytes shape as `HostParam::Result`, but branching on `disc ∈ bytes-discs`
  (arbitrary case discs, one or more Bytes cases) instead of Ok=0/Err≠0. On a Bytes case it copies the payload rope
  into `mem` at the reserved cursor → `(disc, ptr, len)`; a nullary case → `(disc, 0, 0)`. Additive across ~11
  lockstep sites (detector `variant_bytes_payload_cases`, `HostParam::VariantBytes`, the classifier arm,
  `first_unrepresentable_host_op`'s `arg_is_boundary_variant_bytes`, the marshal, the emit dispatch + reclaim,
  serialize's 3-slot flatten, host_imports' structural-WIT `matches!` gate + per-param `CRef`, `used_ops`,
  `set_needs_memory`, the emit.rs cursor pre-scan).
- **[emit, ARG-side] a LIST<scalar> single-payload variant host-op ARG — ✅ DONE / TESTED (SHAPE 229).** The `list`
  sibling of VariantBytes: a `variant{nullary…, list<scalar>-case(s)}` (all list cases sharing the scalar element)
  crosses via the additive `HostParam::VariantList`. Same structural-WIT `variant` component type (`(list <elem>)`
  payload case) and same `(disc, ptr, count)` 3-slot flatten, but `emit_variant_list_arg_reg_flatten` MARSHALS the
  payload list into `mem` via `emit_list_arg_marshal` (`vec-len`/`vec-get` + the scalar element) on a list case
  instead of a Bytes rope-copy. Detector `variant_list_payload_cases` returns the list-case discs + the shared
  element `Ty` (re-derived at emit/used_ops time). Additive across the same ~11 sites as VariantBytes.
- **[emit, ARG-side] a PRODUCT (tuple-of-scalars) single-payload variant host-op ARG — ✅ DONE / TESTED (SHAPE 230).**
  A `variant{nullary…, one tuple-of-scalars case}` crosses via the additive `HostParam::VariantTuple(disc, elem-abis)`.
  Unlike VariantBytes/List (fixed 3-slot `(disc, ptr, len/count)`), the flatten is VARIABLE + POSITIONAL —
  `(disc:i32, e0, e1, …)`, the tuple's elements inline — the register twin of the `result<tuple,enum>` Ok flatten
  MINUS the err-disc/float-join (`emit_variant_tuple_arg_reg_flatten`: tuple case recurses `emit_tuple_reg_flatten`;
  a nullary case zero-fills ALL payload slots). All-scalar → NO `mem`/cursor (so NOT wired into the cursor pre-scan
  or `set_needs_memory`). Detector `variant_tuple_payload_case` scopes to a SINGLE tuple case + all-scalar elements.
  Additive across detector/HostParam/classifier/first_unrepresentable/marshal/emit-dispatch/serialize (positional
  flatten)/host_imports structural-CRef/used_ops. Detector `variant_tuple_payload_case` scopes to a SINGLE tuple
  case + all-scalar elements.
- **[emit, ARG-side] a RECORD (of-scalars) single-payload variant host-op ARG — ✅ DONE / TESTED (SHAPE 231).** The
  RECORD near-twin of VariantTuple, via `HostParam::VariantRecord(disc, wit-ordered-field-abis)`. Same variable
  positional flatten `(disc, f0, f1, …)`, but the fields are WIT-REORDERED (the classifier builds the field ABIs
  then `reorder_record_fields_to_wit` — the SAME helper the `result<record,enum>` arg uses — extracting the record
  case's WIT from the arg's `WitType::Variant` at the record disc). `emit_variant_record_arg_reg_flatten` recurses
  `emit_record_arg_marshal` (WIT-order field push) on the record case; a nullary case zero-fills. All-scalar → NO
  `mem`/cursor. SHAPE 231 uses DISTINCT field widths (s64 then bool) to pin the positional slot widths. This
  COMPLETES the UNIFORM single-compound-payload variant ARG family: scalar (SHAPE 53) / bytes (227/228) /
  list (229) / tuple (230) / record (231).
- **[emit, ARG-side] a MIXED (heterogeneous) scalar+Bytes variant host-op ARG — ✅ DONE / TESTED (SHAPE 232).** The
  canonical heterogeneous tagged-union: a `variant{nullary…, scalar-case(s), bytes-case(s)}` mixing ≥1 scalar
  payload case with ≥1 Bytes payload case, via `HostParam::VariantMixed(Vec<(disc, VariantPayloadKind)>)`. This is
  the hardest variant flatten — the canonical variant JOIN `[disc] ++ position-wise-join(payload flattens)`
  (`host::variant_mixed_join_slots`, replicating `wit_ctype::flatten_variant`: a scalar → one slot, a Bytes →
  `(i32 ptr, i32 len)`, joined slot-wise, mixed int widths → `i64`). `emit_variant_mixed_arg_reg_flatten`
  DISPATCHES per case in a nested `if disc==d … else …` chain: a scalar case unboxes into slot 0 coerced to the
  joined width (wrap `i64→i32` iff slot 0 joined narrow), a Bytes case rope-copies at the cursor → `(ptr extended
  to the joined width, len)`, the innermost else (nullary) zeroes all slots; every arm zeroes the slots it does
  not own. Additive across the same ~11 sites. Verified `variant{a, b(s64), c(bytes)}` → `(i32, i64, i32)`, all
  three arms. A mixed set that also includes a `list<scalar>` payload case is ✅ DONE / TESTED (SHAPE 241): the
  detector `variant_mixed_payload_cases` admits `VariantPayloadKind::List(elem)` (a `list<scalar>` element,
  offset-agnostic) alongside the scalar+Bytes kinds (MIXED = ≥1 scalar AND ≥1 mem case, Bytes OR List — both
  flatten to the same `(ptr, len|count)` two-i32 slots via `variant_mixed_join_slots`); the emit's List arm
  marshals the list into `mem` via `emit_list_arg_marshal` (which advances the cursor) → `(outer-ptr extended to
  the joined width, count)`, mirroring the Bytes arm. Verified `variant{a, b(s64), c(list<s64>)}` → `(i32, i64,
  i32)`, all three arms. A multi-payload-CASE mix (≥3 payload-bearing cases where two mem cases share a slot) is
  ✅ DONE / TESTED (SHAPE 242): `variant{a, b(s64), c(list<u8>), d(list<s64>)}` — a scalar + a Bytes + a List case,
  so slot 1 is contributed by BOTH mem cases; `variant_mixed_join_slots` joins position-wise over ALL cases
  (`join(i64,i32,i32)=i64`, `join(i32,i32)=i32` → `(i32, i64, i32)`), all four arms exercised. A THREE-KIND mix
  (scalar + Bytes + Tuple simultaneously) is now ✅ DONE / TESTED (SHAPE 247), which exposed + fixed a slot-WIDTH
  bug: when a TUPLE case's i64 element widens slot 1 to i64, the Bytes/List arm's len/count (an i32) must be
  `i64.extend_i32_u`'d into slot 1 (it was stored raw → CDZ0910 "expected i64, found i32"). SHAPE 241/242 never
  hit it (slot 1 stayed i32 with no tuple case). The genuine-LIST sibling — scalar + `list<s64>` (NOT `list<u8>`/
  Bytes) + tuple — is ✅ DONE / TESTED (SHAPE 248), which exposed + fixed a SECOND, distinct bug: the List arm and
  the Tuple arm of `emit_variant_mixed_arg_reg_flatten` both allocated their sub-marshal scratch from a FIXED base
  (`pay + 4`), so the List arm's i32 loop counter and a tuple case's i64 `s64`-element temp landed on the SAME
  emit-local INDEX; since `scratch_ty` is ONE type map for the whole function, that index got a SINGLE declared
  type (i64) and the List arm's i32 loop-counter store became an i32-into-i64 write → CDZ0910. SHAPE 247 dodged it
  because its Bytes arm uses fixed LOW scratch locals and never allocates in that overlapping range. The fix bumps
  each dynamic arm's scratch off the RUNNING high-water (`*high`) so the arms' locals are DISJOINT and no index
  carries two ValTypes (the coalesce pass compacts them afterward). A TUPLE
  (multi-payload) compound payload case in a mixed variant is now ✅ DONE / TESTED (SHAPE 243): `variant{a, b(s64),
  c(tuple<s32,s64>)}` — the scalar+tuple mix routes to `VariantMixed` via `VariantPayloadKind::Tuple(elem-abis)`;
  the tuple case flattens POSITIONALLY inline (one slot per element) joined slot-wise with the scalar case
  (`(disc:i32, i64, i64)`), the emit's Tuple arm marshalling via the shared `emit_tuple_reg_flatten` and coercing
  each element into its joined slot width (element 0 i32→i64 widened via `i64.extend_i32_u`). Gate =
  `(any_mem || any_tuple)` (RELAXED from `any_scalar && (any_mem || any_tuple)` in SHAPE 249): a variant with a
  multi-slot case but NO scalar case (two tuples, a tuple beside a list, a Bytes beside a list) is now ✅ DONE /
  TESTED (SHAPE 249) — it used to fall through every narrower detector (`variant_bytes_payload_cases` wants ALL
  Bytes, `variant_list_payload_cases` ALL `list<scalar>` one element, `variant_tuple_payload_case` exactly ONE
  tuple + rest nullary) to CDZ0903. `VariantMixed` is dispatched LAST, so dropping the `any_scalar` requirement
  claims only the RESIDUE those decline — no case is stolen, and a scalar-less set flattens identically (the emit
  simply never takes a Scalar arm). A FLOAT scalar case in a mixed variant is now ✅ DONE / TESTED (SHAPE
  244): `variant{a, b(f64), c(bytes)}` — the int-only restriction is lifted; the float scalar case joins with the
  mem case's integer slots via the canonical reinterpret lattice (`variant_mixed_join_slots` already reinterprets;
  the emit's Scalar arm coerces via `emit_scalar_coerce_into_slot` — `i64.reinterpret_f64` here). A FLOAT tuple
  ELEMENT in a mixed variant is now ✅ DONE / TESTED (SHAPE 245): `variant{a, b(s64), c(tuple<f64,s64>)}` — the
  Tuple arm coerces each element via `emit_scalar_coerce_into_slot` (element 0 f64→i64 reinterpret when folded
  against b's i64 slot), so a mixed variant's inline (scalar/tuple) cases fully support int AND float. A RECORD
  compound payload case in a mixed variant (whose fields are all scalar) is now ✅ DONE / TESTED (SHAPE 252):
  `variant{a, b(s64), c(record{y:s32, x:s64})}` — a `VariantPayloadKind::Record(field-abis, ty)` flattens
  POSITIONALLY inline like a tuple (one slot per field) in the WIT record's field DECLARATION order. The bare
  `variant_mixed_payload_cases` collects the field ABIs name-lex; `variant_mixed_payload_cases_wit` REORDERS them
  to WIT order at the two sites that consume the slot order (the classifier → `serialize`, and the emit), and the
  emit's Record arm marshals field VALUES in WIT order via `emit_record_arg_marshal` — so `serialize` (the param
  core type), the emit (pushed values), and `host_imports` (the WIT `variant` type) all agree (a wrong reorder
  fails canonical-ABI validation → CDZ0910). A record case with a NON-scalar field (Bytes/list/nested-compound)
  still declines cleanly (the detector's `abi_val_type` gate). A mixed LIST case of a NON-scalar ELEMENT
  (`list<record>` / `list<tuple>` / `list<list>` / `list<option>` / `list<result<list<u8>,enum>>`) is now
  ✅ DONE / TESTED (SHAPE 253): the List arm admits any element `emit_list_arg_marshal` handles
  (`list_elem_marshalable`), and the emit's List arm threads the ELEMENT WIT (from the variant's WIT at the case's
  `WitType::List`) so a record/nested element orders + offsets correctly. The outer flatten is UNCHANGED — a list
  case is always `(ptr, count)` two i32 slots regardless of element — so the join / serialize / host_imports are
  untouched; only the in-`mem` element array layout differs (`emit_list_arg_marshal`'s record/tuple/… arms).
  REMAINING variant-payload gaps: the compound-payload variants at the FIELD / list-element positions
  (`RecordFieldAbi::Variant` is scalar-only). (A MULTI-payload single case `b(s64,s64)` is ✅ DONE — SHAPE 236.)
- **[emit, ARG-side] a scalar-payload variant MIXING int with float — the reinterpret join — ✅ DONE / TESTED
  (SHAPE 233/234/235).** A `variant{nullary…, scalar-case(s)}` whose payloads mix an integer with a float (or
  `f32` with `f64`) — the case the uniform `HostParam::Variant` declines (its join has no clean slot). Handled by
  the NEW additive `HostParam::VariantScalarsMixed(Vec<(name, Option<AbiValType>)>)` (the SAME case shape as
  `Variant`, so it rides Variant's nominal host_imports path — `comp_byte` expresses f32/f64). The core flatten is
  `(disc:i32, join)` with the canonical `wit_ctype::flatten_variant` reinterpret join
  (`marshal::reinterpret_join_vt`): a same-width int/float → that int width (`join(i64,f64)=i64`,
  `join(i32,f32)=i32`), anything else → `i64` — always an INTEGER slot when a float is mixed in.
  `emit_variant_mixed_scalar_arg_reg_flatten` dispatches per case (a nested `if disc==d … else …` chain), each
  unboxing with ITS OWN read op and coercing the runtime value into the join slot via `emit_scalar_coerce_into_slot`
  (`i64.reinterpret_f64` / `i32.reinterpret_f32`, a narrow int `i32.wrap_i64`, a bool/`f32`→i64 `i64.extend_i32_u`);
  a nullary case pushes the join-width zero. Register-only (no `mem`). Detector `variant_mixed_scalar_payload_cases`
  is DISJOINT from `variant_scalar_payload_cases` (extracted the shared `variant_all_scalar_cases` collector). Wired
  ARG-side ONLY — a mixed variant as a list-ELEMENT / record-FIELD still cleanly DECLINES (the shared detector +
  `emit_variant_to_mem` are unchanged; decline-don't-miscompile). Verified `{b(s64),c(f64)}`→i64 slot,
  `{b(s32),c(f32)}`→i32 slot, `{b(s64),c(f32)}`→i64 slot (the f32→i64 two-step). REMAINING: the mixed int↔float at
  the record-FIELD / list-element positions (structural enrichment, like the compound-payload variants above).
- **[emit, ARG-side] the BARE (top-level) named-variant host-op ARG — ✅ DONE / TESTED (SHAPE 184/185).**
  `emit_variant_reg_flatten` has always been documented as "the bare-variant ARG marshal", but the corpus never
  pinned it at the top-level param position directly — every prior `variant` case sat inside a record field /
  element / result. SHAPE 184/185 lock a 3-case `variant{a, b, c(s64)}` bare arg on both a scalar-payload arm
  (`(C 9)` → `(disc=2, join=9)`) and a nullary arm (`(B)` → `(disc=1, join=0)`, payload slot zero-filled). A
  3-case variant is NOT reducible to an option, so this genuinely exercises the N-case register flatten (not the
  2-case some/none path). The host stub returns a fixed value; a valid running component that crosses the
  boundary is the pin (the marshal shape is pinned by the module validating with the right core signature).
  REMAINING: a NON-scalar variant payload as a bare arg — a `variant{…, c(tuple<s64,s64>)}` / `variant{…, c(bytes)}`
  declines (CDZ0903) at the ARG position (`emit_variant_reg_flatten`'s payload flatten is scalar-only; the
  multi-slot / rope-copying join is the next variant increment, same family as the multi-payload gap above).
- **[emit, ARG-side] a RECORD host-op ARG with a payload-less `enum` FIELD — ✅ DONE (SHAPE 174).** A
  payload-less `enum` crossed only at the TOP-LEVEL arg (`HostParam::Enum`); nested in a record it declined
  because `field_boundary_abi` had no enum arm. A new `RecordFieldAbi::Enum(cases)` (via `enum_cases`) makes
  `is_boundary_record` accept the record; `flatten_record_field_abi` flattens it to ONE i32 disc, and
  `record_field_cref` lays a nominal `enum` DEFINED+EXPORTED type in the record's instance-type (the nested
  analogue of the top-level enum arg's type). The guest reads the value-heap sum's disc inline (the field rides
  the scalar-unbox marshal path — a payloadless enum's in-guest rep is a bare disc). `record_field_abi_reaches_
  bytes`/`_needs_memory` return false for it.
- **[emit, ARG-side] a `tuple<enum, …>` host-op arg — ✅ DONE (SHAPE 175).** Extends the nested-enum support to
  the tuple-ELEMENT position: `tuple_arg_crosses` now admits an enum element and the tuple-element classifier
  builds its `RecordFieldAbi::Enum` via the shared `field_boundary_abi`; `emit_tuple_reg_flatten` flattens it
  positionally as one i32 disc via the scalar-unbox path (a payloadless enum's in-guest rep is a bare disc). The
  `(tuple (enum …) …)` component type carries the enum from the world WIT.
- **[emit, ARG-side] a top-level `option<enum>` host-op arg — ✅ DONE (SHAPE 176/177).** Extends the nested-enum
  support to the option-PAYLOAD position. An enum's disc reads inline as one i32 (the scalar-unbox path), so
  `option<enum>` flattens to `(opt-disc, enum-disc)` EXACTLY like `option<scalar>`: `option_arg_crosses` now
  admits an enum payload, the classifier builds `RecordFieldAbi::Option(Enum)` (so the component type is
  `(option (enum …))`, matching the world), and `emit_option_reg_flatten`'s scalar branch marshals it with NO
  dedicated arm (None zero-fills).
- **[emit, ARG-side] a top-level `list<enum>` host-op arg — ✅ DONE (SHAPE 178).** The last enum-in-compound
  position. `list_elem_marshalable` now admits an enum element (via `enum_cases`); the element rides
  `emit_list_arg_marshal`'s scalar-store path — each element's disc is written in place at the enum's canonical
  width (`disc_size(n_cases)`), read via the guest sum's disc-unbox, with NO dedicated writer. The `(list (enum
  …))` component type builds from the world WIT (element type via `field_boundary_abi`'s enum arm). **The
  payload-less `enum` now crosses at EVERY nested position** — record FIELD (174), tuple ELEMENT (175), option
  PAYLOAD (176/177), and list ELEMENT (178), plus the pre-existing top-level arg (`HostParam::Enum`).
- **[emit, ARG-side] a `list<record>` arg with an `enum` FIELD in the record element — ✅ DONE (SHAPE 179).** The
  in-mem-writer analogue of the top-level record enum FIELD (174): `product_field_marshalable` now admits an
  enum field (via `enum_cases`), so `list_elem_marshalable` accepts the record element; `emit_record_to_mem`'s
  scalar-field path writes the enum field's disc at the field's canonical offset+width (`disc_size(n_cases)`) —
  no dedicated writer arm (the enum rides the scalar store). (A nested record with an enum field already crossed
  via the register path — `emit_record_arg_marshal` recurses; locked in as SHAPE 180.)
- **[emit, ARG-side] a NESTED record with an `enum` field — ✅ TESTED (SHAPE 180).** `record{ inner: record{ e:
  enum, n: s64 }, k: s64 }` — an enum at record depth. Composes the nested-record arg support (`emit_record_arg_
  marshal` recurses a record field) with the record enum FIELD (174); the inner record flattens inline, the enum
  disc as one i32 via the scalar-unbox path. Already reachable; SHAPE 180 locks in the value round-trip.
- **[emit, ARG-side] record host-op ARG with a `list<record>` FIELD — ✅ DONE (SHAPE 155).** The
  record-element twin of SHAPE 31 (a `list<scalar>` field): `emit_record_arg_marshal`'s list-field arm runs
  `emit_list_arg_marshal` whose element writer (`emit_record_to_mem`) writes each record element in place; the
  `field_boundary_abi` element recursion builds the `(list (record …))` field type and the marshal +
  `collect_record_field_ops` recurse it in lockstep. (A record ARG with a `list<scalar>`/nested-record/tuple
  field, and the reverse `list<record{…}>` whole-arg, were already covered.)
- **[emit, register-path] `option<list<T>>` — ✅ DONE at EVERY register position (SHAPE 156-161).** The ARG +
  tuple-ELEMENT (SHAPE 156/157/158) route through `emit_option_reg_flatten`'s list-payload branch — the register
  analogue of the option<bytes> `(disc, ptr, len)` branch: on Some it marshals the payload list into `mem` at the
  running cursor via `emit_list_arg_marshal` (leaves `(outer-ptr, count)`), captures them, pushes
  `(disc=1, ptr, count)`; on None `(0, 0, 0)`. The RECORD-FIELD position (SHAPE 159/160) uses a new option<list>
  arm in `emit_record_arg_marshal` (the list analogue of its option<bytes> field arm), and `field_boundary_abi`'s
  option arm was widened to `Option(List(<elem>))` so `is_boundary_record` admits an option<list>-field record in
  lockstep with that marshal arm. The COMPOSED `option<record{…option<list>…}>` (SHAPE 161) follows for free
  (`emit_option_reg_flatten`'s record branch recurses `emit_record_arg_marshal`). `option_arg_crosses` +
  the emit dispatch + the cursor pre-scan (`record_has_option_field_needing_mem`, renamed from
  `record_has_option_bytes_field`) + `used_ops` all widened in lockstep. The `list<option<list>>` element is now
  DONE too — SHAPE 162 (see the list-element entry above).
- **[emit, register-path] a nested `option<option<scalar>>` — ✅ DONE (SHAPE 181/182/183).** The payload is
  itself an `option<scalar>`, flattening to `(outer-disc, inner-disc, scalar)` via `emit_option_reg_flatten`'s
  new nested-option branch, which on outer Some reads the inner option handle and RECURSES
  `emit_option_reg_flatten` on it (pushing the inner `(disc, scalar)`), capturing in reverse; outer/inner None
  zero-fill. Widened in LOCKSTEP (scalar inner): `field_boundary_abi`'s option arm + `option_arg_crosses` (the
  shared arg + tuple-element gate) + the classifier option arm + `emit_option_reg_flatten` (arg + tuple element)
  + `emit_record_arg_marshal`'s nested-option field arm (delegates to `emit_option_reg_flatten`) + the
  `collect_used_ops` option-payload dispatch. `flatten_record_field_abi` (`Option(Option(Scalar))` → 3 slots) +
  `record_field_cref` (recurses) were already general. The record-FIELD (RF) and tuple-ELEMENT (TE) positions
  cross for free (verified). A `Bytes` inner (`option<option<bytes>>`, TOP-LEVEL arg) is now DONE too — SHAPE 218:
  the nested-option branch computes the inner option's flatten width DYNAMICALLY (2 slots scalar / 3 slots
  `(disc, ptr, len)` bytes, checking `Bytes` first since a bytes handle's `valtype_of` is `Some(I32)`), the inner
  recursion copies the rope into `mem` at the threaded cursor; widened in lockstep across `option_arg_crosses` +
  the classifier nested-option arm + `emit_option_reg_flatten` + `collect_used_ops` (the bytes inner needs
  `bytes-len`/`bytes-get` declared — its condition already admitted bytes via the I32 handle but the body only did
  `get_op_ty`, so the rope-copy `CallImport` was u32::MAX / CDZ0910) + the emit.rs cursor pre-scan. A `list<T>`
  inner (`option<option<list<T>>>`, TOP-LEVEL arg) is now DONE too — SHAPE 219. The ENTIRE top-level nested-option
  family — a `tuple` (SHAPE 220) and `record` (SHAPE 221) inner too — is now DONE via a GENERIC flatten (#9933): the
  nested-option branch derives `slot_vts = flatten_record_field_abi(field_boundary_abi(inner-option))` (reproducing
  scalar 2-slot / bytes+list 3-slot and handling tuple/record variable-width uniformly), `option_arg_crosses` + the
  classifier + `collect_used_ops` key off the single `field_boundary_abi(inner-option).is_some()` gate, and the
  emit.rs cursor pre-scan reserves iff the inner abi `record_field_abi_needs_memory` (so an all-scalar tuple/record
  inner needs none). The record-FIELD position is now DONE too — SHAPE 222 (`record{a: option<option<bytes>>, k}`):
  `field_boundary_abi`'s nested-option arm admits any crossing inner, and `emit_record_arg_marshal`'s nested-option
  field arm DELEGATES to `emit_option_reg_flatten` (so it inherits the generic inner handling); the cursor rides a
  new nested-option clause in `record_has_option_field_needing_mem`. The tuple-ELEMENT position is now DONE too —
  SHAPE 223 (`tuple<option<option<bytes>>, s64>`): it needed NO code change (the tuple-element arm already delegates
  to `emit_option_reg_flatten`, `tuple_arg_crosses` admits via `option_arg_crosses`, `tuple_arg_needs_cursor`
  recurses nested options, and the tuple-element `used_ops` recurses `collect_record_field_ops`). So the nested
  `option<option<X>>` now crosses at EVERY arg position (top-level, record-FIELD, tuple-ELEMENT). The LIST-ELEMENT
  position is now DONE too — SHAPE 224 (`list<option<option<scalar>>>`): `emit_option_to_mem` (the in-mem
  list-element option writer) gained a nested-option arm that RECURSES itself on the inner option at
  `dest + payload_off`, with the `option_elem` detector + `list_elem_marshalable`'s option arm +
  `collect_list_elem_ops` all admitting a nested-option payload in lockstep. So the nested `option<option<X>>` now
  crosses at EVERY arg position INCLUDING the list element. A BYTES inner at the list-element position
  (`list<option<option<bytes>>>`) is now DONE too — SHAPE 225 (no new code: the nested arm recurses into the inner
  option's Bytes arm, the rope copied at the list-arg's unconditionally-reserved spill cursor). A RECORD inner
  (`list<option<option<record>>>`) is now DONE too — SHAPE 226 (no new code: the nested arm recurses into the inner
  option's Record arm, `emit_record_to_mem` writing the product in place WIT-ordered). REMAINING (nested option): a
  list/tuple INNER at the list-element position — the same nested arm covers it, un-pinned.
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
- **[emit, export] bare entry PARAM — the `try_bare_entry_param_component` cluster (driver:
  wasm-boundary-marshal). SOURCE OF TRUTH = the `rpp*`/`eoc*`/`eot*`/`eor*`/`erp*`/`erc*` corpus cases in
  `spec/semantics/09-functions.sexp`** (this doc does not re-enumerate them — a per-case list re-stales every
  landing). WHAT CROSSES now: an aliased-width scalar; `String`/`Bytes`; a flat `list<scalar>` (incl. nested
  `list<list<…>>`) and a byte-leaf `list<string>`/`list<bytes>`; a scalar/`String`/`list`-fielded `tuple<…>`
  and `record<…>` (a record crosses structurally as a `tuple<…>` via `structuralize_wit`); a `record`/`tuple`
  nested at ANY depth (rpp8-13); a value-form `BigInt`/`Rational`/`Symbol` (list<u8> value-decode); and an
  `option<…>`/`result<…>` whose payload is any of those (scalar, byte-leaf, `list`, `tuple`, or `record` —
  eoc/eop/eos/eob/eot/eor/erp/erc). STILL DECLINES (`CDZ0904`, a clean no-bare-boundary-form decline, verified):
  a whole enum/`Sum` (non-Option/Result) entry param; a `record`/`tuple` with a `Sum` (`Option`/`Result`) FIELD
  (`ty_natural_wit` has no structural WIT for a nested sum — a shared-lowering slice); a compound LIST element
  (`list<tuple>`/`list<option>`/`list<record>` — `list_scalar_elem` declines a compound leaf); and a nested
  byte-leaf list (`list<list<string>>`).
- **[emit, export] typed `result<ok,err>` EXPORT result — ✅ DONE (SHAPE 74/75).** A `result<s64,s64>`
  (74) and a compound-payload `result<record{lo,hi}, s64>` (75) EXPORT result now cross: `canon_write_of`
  gained a Result arm (a 2-variant both-payload sum → `CanonWrite::Variant`, mapping guest `Ok`→boundary
  disc 0 / `Err`→1 BY NAME, payload written recursively at the canonical result layout), reusing the
  existing `CanonWrite::Variant` emit (SHAPE 61) with no new writer. Both `(live-objects known-leak)`
  (SpillRecord-result reclaim class). The Result arm now resolves each payload via the variant's ctor occ +
  `payload_ty_at_instantiation` (unified with the Variant arm), so a CUSTOM (non-prelude) sum with CONCRETE
  payloads (`(type Res (Ok Int64) (Err Int64))`) crosses too — ✅ **SHAPE 120** — not only a generic prelude
  `Result a b`. A NULLARY arm (`result<T>` = err unit, `result<_, E>` = ok unit) is ✅ **DONE — SHAPE 121**:
  payload-presence must agree (guest payload arm iff WIT arm carries a payload), a nullary arm writes the disc
  alone (`VariantArm { payload: None }`), layout via `variant_disc_layout` over the two (possibly-absent) arms.
- **[emit, export] flat single-scalar-field record result — ✅ DONE (SHAPE 76).** A `record{v: s64}` result
  flattens to ONE core value (returned directly, not by pointer), so the SpillRecord path (retptr) declined
  it. A `ResultLower::FlatScalarField` lower reads the one field off the def's record handle (`arr-get` +
  unbox, narrowing a ≤32-bit value) and returns the scalar; no memory. `(live-objects known-leak)` (the
  record handle is not reclaimed). Restricted to a record with exactly one scalar field (a nested-compound
  single field, or a multi-field-but-1-flat record, is a later slice).
- **[emit, export]** the TYPED-INTERFACE PARAM (`record_interface_export`) for a Tuple/Sum/List/String/Bytes
  member (distinct from the BARE-entry cluster above, which now covers those shapes — a top-level Tuple and a
  `result<>` bare-entry param both cross, rpp4/erp/erc). Remaining bare-entry gap: a compound LIST element
  (see the entry-PARAM bullet's still-declines list).

**Design-level (no WIT boundary form on either side; needs a design decision — TRACK, don't rush):**
- **[design]** BigInt, Rational, exact-`Qty`, Map, Set, Symbol.

## By design — NOT gaps

- **bare-effect** path (NO imposed `wit_world`) is scalar/unit-only for results (the world-driven path
  is the compound envelope). State it; don't "close" it. NOTE: this holds only WITHOUT an imposed world
  — a plain host-delegating guest WITH an imposed import-declaring `wit_world` now crosses a compound
  RESULT (v-wit-boundary B1, SHAPE 83); it is the presence of the world, not the export shape, that opens
  the compound envelope on this path.
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

- compound host-IMPORT RESULT on the PLAIN host-delegating envelope for a CUSTOM import-only wit-world
  (v-wit-boundary B1, PR #9573) — `allow_option_bytes` broadened to `world_has_import_interface` (B0);
  `build_host_result_types` threaded through `assemble_host_runtime{,_mem}` / `assemble_host{,_mem}` to
  declare the result's WIT defined-type + `needs_realloc` shared-mem shape; SHAPE 83 (record result,
  WIT-dump verified `record host-result-t0`). The emit lift was already structural — only the decline gate
  + the instance-type declaration were missing. Bare-effect (no world) unchanged.
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
