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
| Record (all fields boundary/leaf, incl. nested + WIT-order reorder) | arg + result + export | world/export | `is_boundary_record` / `result_is_liftable` (Record) / `record_interface_export` | 11, 13, 19, 20, 21, 25, 29, 31, 35, 36 |
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
  payload case at the ARG (register-flatten) position; and a mixed int↔float / f32↔f64 single-payload variant
  join — see `variant_scalar_payload_cases` / `variant_liftable_payload_cases`.
- **[emit]** compound variant payload at the ARG (register-flatten) position; compound-payload
  variant list-element.
- **[emit, ARG-side]** `option<compound>` host-op record-ARG FIELD — ✅ scalar/bytes (pre-existing) + **tuple-of-scalars (SHAPE 123)** + **record-of-scalars (SHAPE 124)** + **record-with-a-Bytes-field (SHAPE 126)**. `field_boundary_abi` recurses the payload; `emit_record_arg_marshal` SCRATCH-FLATTENS it (`(disc, flatten(payload))` — disc + one core slot per scalar payload field / TWO `(ptr,len)` slots per Bytes field, marshalled into N scratch slots since LIR blocks are single-value, pushed after the `if`; the Some arm recurses `emit_record_arg_marshal` on the payload record and captures its N pushed slots in reverse, the None arm zero-fills; a record payload reads each WIT field from its name-lex cell index, `reorder_record_fields_to_wit` recursing the `Option(Record)` to WIT order; a Bytes payload leaf copies its rope into shared mem at the reserved scratch cursor, `record_has_option_bytes_field` reserving the cursor in the emit.rs pre-scan). NB: the slot count checks `Ty::Bytes` BEFORE `valtype_of` (which is `Some(I32)` for a Bytes handle) so a byte leaf counts as 2 slots, not 1. REMAINING: a nested-compound (option/tuple/record-of-compound) payload field inside the option; the `option<compound>` LIST ELEMENT (`list<option<compound>>`, `list_elem_marshalable`). The RESULT side is DONE — SHAPE 66.
- **[emit, ARG-side]** `option<compound>` host-op TOP-LEVEL arg (the bare param position, not nested in a record) — ✅ scalar/bytes (pre-existing) + **tuple-of-scalars, Some + None (SHAPE 127/128)** + **tuple-with-a-Bytes-element, Some (SHAPE 129)** + **record-of-scalars, Some + None (SHAPE 130/131)** + **record-with-a-Bytes-field, Some + None (SHAPE 132/133)** + **record-with-a-list<scalar>-field, Some + None (SHAPE 148/149)** + **record-with-a-nested-record field, Some (SHAPE 150)** + **record-with-an-option<scalar> field, Some (SHAPE 151)**. The option<record> arm now admits ANY payload record `is_boundary_record` accepts (the SAME `field_boundary_abi` admit set the direct record ARG uses), so a compound field (a `list<s64>`) crosses at the option-arg position exactly where it crosses at the bare-record-arg position: `option_arg_crosses` (the single shared gate for the classifier, `first_unrepresentable_host_op`, the emit dispatch, and `used_ops` — all widened in lockstep) tests `is_boundary_record` on the payload; the classifier builds the payload record abi via `field_boundary_abi` per field; `emit_option_reg_flatten`'s record branch derives its capture `slot_vts` from each field's flattened boundary ABI (`field_boundary_abi` → `flatten_record_field_abi`, mapped back to `ValType` via `ValType::from_byte`), so a `list<s64>` field counts as its 2 `(ptr,count)` slots — a `valtype_of`-based count treated the list handle as one i32 and left a value on the operand stack (CDZ0910); the emit.rs cursor pre-scan reserves the scratch cursor for an `option<record-with-a-list/tuple/bytes-field>`, and the cursor-slot reservation now bumps the declared-locals top to `slot + 1` (a cursor-only reservation formerly excluded it, panicking `coalesce_func`'s remap). The record branch of `emit_option_reg_flatten` takes a `payload_wit` (threaded from the caller's `wit_params[arg_i]`) and recurses `emit_record_arg_marshal` (WIT-order field push); the classifier REORDERS the payload record abi to WIT order (`reorder_record_fields_to_wit`) so the `(option (record …))` component type + core flatten agree with the marshal — SHAPE 130 uses distinct field widths (s64 lo, bool hi) so a missed reorder fails instantiation (proven: the pre-reorder build hit an `expected (i32 i32 i64) / found (i32 i64 i32)` codegen defect). `emit_option_reg_flatten`'s tuple branch: the component type + serialize flatten are already general over the payload abi (built from the declared WIT type / `flatten_record_field_abi`), so only the guest marshal + the two lockstep classifiers (`collect_host_imports_at`'s `HostParam::Option` arm + `first_unrepresentable_host_op`'s `arg_is_boundary_option`) + the emit.rs cursor pre-scan + `used_ops` were scoped. On Some it flattens the payload tuple POSITIONALLY via `emit_tuple_reg_flatten` (one core slot per SCALAR element, `(ptr,len)` = TWO slots per `Bytes` element with the rope copied into shared mem at the reserved cursor), captured into N scratch slots and pushed as `(disc, elem…)` after the single-value `if`; None zero-fills. Two traps: (a) `valtype_of`-is-`Some(I32)`-for-a-tuple → the tuple branch precedes the scalar branch, and the element guard is `abi_val_type OR Bytes` (not `valtype_of`, which would wrongly admit a nested-compound element); (b) a Bytes element expands to 2 scratch slots (the byte-leaf slot-count pin, same as the record byte-leaf field), and the cursor pre-scan reserves for an `option<tuple-with-bytes>` arg. REMAINING: a `variant` field inside the option payload record (option<scalar>/nested-record fields now TESTED at SHAPE 150/151; a variant field rides the same `field_boundary_abi` recursion but is untested). The `option<compound>` LIST element (`list<option<record>>` / `list<option<tuple>>`) is now DONE — SHAPE 152/153 (see the list-element entry below).
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
  33), `list<option<scalar>>` (SHAPE 38), `list<option<record>>` / `list<option<tuple>>` (SHAPE 152/153), `list<option<bytes>>` (SHAPE 154), `list<option<list>>` (SHAPE 162), and
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
  element dispatch + `collect_list_elem_ops` widened in lockstep. REMAINING (ARG-side only): an `option<option>`
  list element (the option-to-mem writer has no nested-option arm), and a mixed int↔float variant element (rolls
  into the compound-variant-payload / mixed-join gaps above).
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
- **[emit, register-path] a top-level `result<scalar, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE 186/187/188).**
  The scalar-Ok sibling of the Bytes-Ok result arg: a new `HostParam::ResultScalar(ok-abi, err-cases)` (detector
  `result_scalar_enum`, admitting an INTEGER-width Ok scalar + a payloadless-enum Err; a FLOAT Ok / `variant` err
  is a later increment — the reinterpret join lattice). It flattens to just 2 slots `(disc:i32, join)` with NO
  `mem` (no rope): `emit_result_scalar_arg_reg_flatten` reads the result disc, unboxes the Ok scalar into the join
  slot on Ok, reads the err enum's disc into it on Err. The join is `i64` iff the Ok scalar is 64-bit (the `i32`
  err disc widens via `i64.extend_i32_u`), else `i32` — SHAPE 186/187 (s64 Ok, `(param i32 i64)`) vs 188 (bool Ok,
  `(param i32 i32)`) pin both widths + both arms. Widened in LOCKSTEP: the classifier arm, `first_unrepresentable_
  host_op`'s `arg_is_boundary_result_scalar`, the emit dispatch (+ Owned/dup-site reclaim drop), `collect_used_ops`
  (`sum-disc`/`sum-payload` + the Ok unbox `get-*` + `drop`), `serialize` (`(i32, join)` core flatten), and
  `host_imports.rs` (the structural-CRef param reference + `host_param_abi` decline). NOT the cursor pre-scan and
  NOT `set_needs_memory` (no rope → no `mem`, unlike the Bytes result). REMAINING: `result<record/list/tuple,enum>`
  (a compound Ok needs the in-mem arg marshal) and `result<_, variant>` / a float Ok.
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
  (structural-CRef param reference + `host_param_abi` decline). NOT the cursor pre-scan / `set_needs_memory` (all-
  scalar → registers, no rope). REMAINING (result family): a record with a COMPOUND field (Bytes/list/nested →
  needs `mem`); `result<list,enum>`; `result<_, variant>` / a float Ok.
- **[emit, register-path] a top-level `result<tuple-of-scalars, enum>` host-op ARG — ✅ DONE / TESTED (SHAPE
  192/193/194).** The tuple-Ok sibling of the record-Ok result: a new `HostParam::ResultTuple(elem-abis, err-
  cases)` (detector `result_tuple_enum`, admitting a tuple every element of which is a non-float SCALAR + a
  payloadless-enum Err). It flattens to `(disc:i32, elem0, elem1, …)` — the discriminant then the Ok tuple's
  elements POSITIONALLY (element order, NO reorder — a tuple is positional, unlike the record's WIT reorder), the
  `i32` err disc riding the FIRST element's slot on Err. Every element is a non-float scalar → one register slot
  each, joining the first with the `i32` err disc never widens past its own width — the slot widths ARE the
  element widths, no `mem`. `emit_result_tuple_arg_reg_flatten` recurses `emit_tuple_reg_flatten` on Ok (N pushes
  captured in reverse) and puts the err enum's disc in slot 0 on Err. SHAPE 192/193 (`tuple<s64,s64>`, core
  `(param i32 i64 i64)`, both arms) + 194 (`tuple<bool,s64>`, mixed `(param i32 i32 i64)`) pin the join + widths.
  Widened in LOCKSTEP: classifier, `first_unrepresentable_host_op`, emit dispatch (+ reclaim), `collect_used_ops`,
  `serialize`, `host_imports.rs` (all four `Result*` arms — the structural-CRef `matches!` MUST list every
  `Result*` variant or a `ResultRecord`/`ResultTuple` param silently falls back to the wrong CRef → CDZ0910
  component-validation failure; this bit once when a codemod dropped `ResultRecord` from that `matches!`). NOT the
  cursor pre-scan / `set_needs_memory`. REMAINING (result family): a compound/float tuple element; a record with a
  compound field; `result<list,enum>`; `result<_, variant>`.
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
  **WIDENED (SHAPE 197/198):** the element may also be an all-scalar PRODUCT — a `list<record-of-scalars>` /
  `list<tuple-of-scalars>` — which `emit_list_arg_marshal` writes inline (`emit_record_to_mem`/`emit_tuple_to_mem`)
  and which never reaches `list<u8>` (so `has_list_param` stays false, no shared-list-type change). This was a
  single-line `result_list_enum` element-gate relaxation (scalar OR all-scalar record/tuple); the emit + used_ops
  already handle product elements via the shared list marshal. REMAINING (result family): a list element that
  reaches `list<u8>` (Bytes/nested-list/option element — needs the shared list type + `has_list_param`); a
  compound/float tuple element; a record with a compound field; `result<_, variant>`.
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
  arm). REMAINING variant gaps: a multi-payload variant case at the ARG register-flatten position, and a mixed
  int↔float / f32↔f64 single-payload variant (the canonical reinterpret join).
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
  cross for free (verified). REMAINING: a non-scalar inner payload (`option<option<bytes/record/…>>`) — the inner
  option's mem-writing payload needs threading through the recursion; and `list<option<option>>` (the
  `emit_option_to_mem` in-mem writer has no nested-option arm).
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
