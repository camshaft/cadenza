; General WIT-ABI boundary — typed component exports crossing the canonical-ABI boundary.
;
; This file is the corpus home for the "general WIT-ABI" (shape-2) coverage that used to live as
; in-crate `wasmtime`-running #[test]s in rcdzc (`Component::from_binary` + `run_returns`). Per the
; operator (2026-08-25): migrate every such behavioral test OUT of the compiler and test it once,
; fully end-to-end, in the corpus — reusing this established `.sexp` format rather than a new one.
; The mission retires rcdzc's `wasmtime` dev-dependency once all its behavioral coverage lands here.
;
; A shape-2 case declares a guest that EXPORTS a typed function; `(call <export> <arg>...)` invokes it
; across the component boundary and `(output ...)` asserts the returned value. The interesting content
; is the ABI marshalling of the param/result TYPES (records, options, lists, variants/enums) as they
; cross the canonical component boundary — a broken lift (wrong discriminant, payload offset, or element
; shape) yields a different observable value. The WIT world is USUALLY SYNTHESIZED from the guest's own
; type annotations (no external artifact). A case may instead IMPOSE an explicit world with a
; `(wit-world <world-sexpr>)` + `(component-name "iface")` clause — the export then crosses under that
; named interface (`(call <export>)` is invoked as `<iface>#<export>`); an imposed-world case runs only
; on the wasm backend (Rust/ML decline → Todo, no external-world ingest there). World type-tag heads MUST
; be STRING LITERALS: `("record" …)`, `("option" …)`, `("list" …)`; field names / scalars stay bare.
;
; This file grows one shape at a time as v-rust-backend feeds each anchored shape; each corpus case
; that goes green retires its in-crate `run_returns` equivalent (no coverage gap).
;   SHAPE 1 — option<s64> RESULT (both arms).             SHAPE 5 — bare list<s64> RESULT.
;   SHAPE 2 — named VARIANT-with-payload RESULT.          SHAPE 7 — list-of-records RESULT.
;   SHAPE 3 — scalar identity export.                     SHAPE 8 — none-only option<s64> via an IMPOSED
;   SHAPE 4 — multi-field record RESULT.                    WIT world (element type from the world decl).
; Later shapes (all via an imposed WIT world): SHAPE 9 — list-of-records with a Bytes leaf + a Bytes export
; param. SHAPE 10 — a scalar host-import RESULT (clock.now) threaded into the step. SHAPE 11 — a RECORD
; host-import RESULT (probe.info) with a field-order-follows-WIT reorder. SHAPE 12 — a list<s64> host-op ARG
; (sink.push) lowered + invoked e2e (the align-8 scalar-element list-arg marshal). SHAPE 13 — an all-scalar
; record{a,b} host-op ARG (deliver.push) flattened to two i64 core slots + invoked e2e. SHAPE 14 — a Bytes
; host-op ARG with a scalar u64 RESULT (hasher.hash) threaded into the step's deadline-nanos.

(case "an option<s64> field in a record result crosses the export boundary on both arms"
  (doc    "The general option<T> RESULT lift across the WIT export boundary. The guest takes a record
           `{ x: Int64 }` and returns a record `{ d: Option Int64 }`, mapping x=0 to None and any
           other x to Some(x). Asserting BOTH arms exercises the Option discriminant (Some vs None)
           and, on the Some arm, the s64 payload — a broken lift (wrong disc, wrong payload offset,
           or a dropped payload) produces a different d. Migrated from the in-crate wasmtime test
           `an_option_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 1/18).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             (record (= d (if (= (. m x) 0) Option.None (Option.Some (. m x))))))
           (export f)))
  (call f (: (record (= x 42)) (Record (: x Int64))))
  (output (: (record (= d (Some 42))) (record (d (Option Int64)))))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= d (None unit))) (record (d (Option Int64))))))

(case "a named variant-with-payload field in a record result crosses the export boundary on both arms"
  (doc    "SHAPE 2 — the general named-VARIANT RESULT lift (a discriminated union, distinct from a bare
           enum: one case carries a payload). The guest returns a record `{ o: Outcome }` where
           `Outcome = Continue | Close(Int64)`, mapping x=0 to Continue (nullary) and any other x to
           Close(x) (s64 payload). Asserting BOTH arms exercises the variant discriminant AND the
           payload case's s64 lane — a broken lift (wrong disc, missing/misplaced payload) yields a
           different o. Migrated from the in-crate wasmtime test
           `a_named_variant_result_guest_compiles_and_runs` (v-rust-backend shape-2 feed 2/18).")
  (input (do
           (type Outcome (Continue) (Close Int64))
           (def (f (: m (Record (: x Int64))))
             (record (= o (if (= (. m x) 0) Outcome.Continue (Outcome.Close (. m x))))))
           (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= o (Continue unit))) (record (o Outcome))))
  (call f (: (record (= x 7)) (Record (: x Int64))))
  (output (: (record (= o (Close 7))) (record (o Outcome)))))

(case "a scalar export crosses the WIT boundary and returns its argument"
  (doc    "SHAPE 3 — the simplest WIT export boundary: s64 in, s64 out (identity). Exercises the plain
           scalar param/result canonical-ABI lowering with no compound structure. Migrated from the
           in-crate wasmtime test `a_scalar_interface_export_guest_compiles_and_runs` (v-rb feed 4).")
  (input (do (def (f (: x Int64)) x) (export f)))
  (call f (: 7 Int64))
  (output (: 7 Int64)))

(case "a multi-field record result crosses the export boundary"
  (doc    "SHAPE 4 — the multi-field RECORD RESULT spill: the guest returns { a, b } from a record
           input, exercising the record result lift (two s64 fields, in order). Migrated from the
           in-crate wasmtime test `a_record_result_guest_compiles_and_runs_via_result_spill` (v-rb feed 5).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             (record (= a (. m x)) (= b (+ (. m x) (. m x)))))
           (export f)))
  (call f (: (record (= x 21)) (Record (: x Int64))))
  (output (: (record (= a 21) (= b 42)) (Record (: a Int64) (: b Int64)))))

(case "a bare list of scalars in a record result crosses the export boundary"
  (doc    "SHAPE 5 — the bare-LIST RESULT: a record field that is a list<s64>, exercising the list
           result lift (element count + s64 element stride). Migrated from the in-crate wasmtime test
           `a_list_result_guest_compiles_and_runs` (v-rb feed 6).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             (record (= xs (list (. m x) (+ (. m x) (. m x))))))
           (export f)))
  (call f (: (record (= x 5)) (Record (: x Int64))))
  (output (: (record (= xs (list 5 10))) (record (xs (List Int64))))))

(case "a list of records in a record result crosses the export boundary"
  (doc    "SHAPE 7 — the list-of-records RESULT (the structural workhorse of every reducer Step's
           `requests: list<request>`): a record field that is a LIST of RECORDS. The guest returns
           `{ items: [ { a, b } ] }` with one element built from the input (a=x, b=x+x). Exercises the
           LIST result lift (count + stride) AND the element RECORD lift (two s64 fields in order).
           Migrated from the in-crate wasmtime test `a_list_of_records_result_guest_compiles_and_runs`
           (v-rust-backend shape-2 feed 3/18).")
  (input (do
           (def (f (: m (Record (: x Int64))))
             (record (= items (list (record (= a (. m x)) (= b (+ (. m x) (. m x))))))))
           (export f)))
  (call f (: (record (= x 7)) (Record (: x Int64))))
  (output (: (record (= items (list (record (= a 7) (= b 14))))) (record (items (List (record (a Int64) (b Int64))))))))

(case "a none-only option<s64> record field resolves its element type from an imposed WIT world"
  (doc    "SHAPE 8 — the general WIT-ABI shape where the export boundary is DECLARED by an explicit WIT
           world, not synthesized from the guest. The guest's field d is STATICALLY Option.None (no Some
           arm), so its option element type cannot be inferred from a payload — it is fixed by the world's
           `d: option<s64>` declaration (the `(wit-world …)` clause). The guest exports f UNDER the
           interface `cadenza:demo/iface` (`(component-name …)`), so the run invokes it through that
           interface instance. A broken WIT-element-type resolution fails to emit or mis-types d. Migrated
           from the in-crate wasmtime test `a_none_only_option_result_resolves_via_wit`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (d ("option" (s64))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) (record (= d Option.None))) (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= d (None unit))) (record (d (Option Int64))))))

(case "a list of records with a bytes leaf crosses the export boundary via an imposed WIT world"
  (doc    "SHAPE 9 — the reducer-echo Step shape (list of records each carrying a Bytes/list<u8> leaf,
           AND a Bytes/list<u8> export PARAM), via an imposed world. Guest f: {tok: Bytes} ->
           {items: [{echo: tok}]}; exercises the list result lift, the element record lift, a bytes leaf
           that echoes the input, AND the list<u8> export-param decode. A guest Bytes field crosses as the
           world's list<u8>. Migrated from the in-crate wasmtime test
           `a_list_of_records_with_bytes_leaf_result_compiles_and_runs` (v-rb shape-2 feed 7).")
  (wit-world (world w (export iface (member f (func (param m ("record" (tok ("list" (u8))))) (result ("record" (items ("list" ("record" (echo ("list" (u8)))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: tok Bytes)))) (record (= items (list (record (= echo (. m tok))))))) (export f)))
  (call f (: (record (= tok (list 10 20 30))) (Record (: tok Bytes))))
  (output (: (record (= items ((record (= echo (10 20 30)))))) (record (items (List (record (echo (List UInt8)))))))))

(case "a reducer performing a scalar host import threads the u64 result into the step (via an imposed WIT world)"
  (doc    "SHAPE 10 — a scalar host-import RESULT (clock.now : () -> u64) driven through an imposed WIT world.
           The reducer on-message performs clock.now (nullary scalar host op) and threads the u64 into the
           step's request deadline-nanos = Some(now). Stubbing clock.now -> 42 + asserting deadline-nanos ==
           Some(42) makes the scalar host result LOAD-BEARING. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_scalar_host_import_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/clock (member now (func (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect clock (op now (-> Unit UInt64))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (clock) (record (= requests (list (record (= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos (Option.Some (clock.now unit)))))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond clock.now (: 42 UInt64)))
  (host-calls (call cadenza:platform/clock.now))
  (output (: (record (= requests ((record (= contract (1)) (= payload (2)) (= token (3)) (= deadline-nanos (Some 42))))) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))

(case "a reducer performing a RECORD host import reads a field of the result (via an imposed WIT world)"
  (doc    "SHAPE 11 — a RECORD host-import RESULT (probe.info : (Bytes) -> record{zebra, alpha}) driven through
           an imposed WIT world; the host RECORD's declared order (zebra, alpha) differs from the guest name-lex
           order, exercising the field-order-follows-WIT reorder on the lift. The reducer reads the result's
           `alpha` field into the request payload. Stubbing probe.info -> {zebra:(9), alpha:(7)} and asserting
           payload == (7) makes the record host result + its field-reorder load-bearing. Migrated from the
           in-crate wasmtime test `a_reducer_performing_a_record_result_host_op_emits_and_loads` (v-rb synthetic-op host-result).")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/probe (member info (func (param key ("list" (u8))) (result ("record" (zebra ("list" (u8))) (alpha ("list" (u8))))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect probe (op info (-> Bytes (Record (: zebra Bytes) (: alpha Bytes))))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (probe) (record (= requests (list (record (= contract (. m contract)) (= payload (. (probe.info (. m token)) alpha)) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond probe.info (: (record (= zebra (list 9)) (= alpha (list 7))) (Record (: zebra Bytes) (: alpha Bytes)))))
  (host-calls (call cadenza:platform/probe.info))
  (output (: (record (= requests ((record (= contract (1)) (= payload (7)) (= token (3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
(case "a typed reducer performing a list-of-scalars host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 12 — a `list<s64>` host-op ARG (sink.push : (list<s64>) -> unit) driven through an imposed
           WIT world. The reducer on-message performs sink.push (list 1 2 3) (a unit-result, observe-only host
           op) then returns a Continue step with an empty requests list. Running the guest LOADS the emitted
           component and INVOKES sink.push, exercising the list<s64> arg lower (value-heap List<Int64> -> the
           (ptr,count) the component list<s64> param lowers to, elements unboxed at i64 stride) e2e — a strictly
           stronger check than the anchor's emit+validate+load, since a broken flatten-arity or element stride
           fails to instantiate or run. The observed host-call sequence pins that sink.push actually fired.
           Migrated from the in-crate wasmtime test `a_reducer_performing_a_list_scalar_arg_emits_and_loads`
           (v-rust-backend shape-2 synthetic arg feed, align-8 stress).")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/sink (member push (func (param vals ("list" (s64))) (result ("unit")))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List Int64) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push (list 1 2 3)) (record (= requests (list)) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output (: (record (= requests ()) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
(case "a typed reducer performing an all-scalar record host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 13 — an all-scalar `record { a: s64, b: s64 }` host-op ARG (deliver.push : (record{a,b}) -> unit)
           driven through an imposed WIT world. The reducer on-message performs deliver.push (record a=1 b=2)
           (a unit-result, observe-only host op) then returns a Continue step with an empty requests list.
           Running the guest LOADS the emitted component and INVOKES deliver.push, exercising the record arg
           lower (value-heap record -> the two i64 core slots the component record param flattens to, each field
           unboxed in the WIT record's declared order) e2e — a strictly stronger check than the anchor's
           emit+validate+load, since a broken field flatten (wrong arity/order/offset) fails to instantiate or
           run. The observed host-call sequence pins that deliver.push actually fired. Migrated from the in-crate
           wasmtime test `a_typed_reducer_with_a_record_arg_host_import_emits_and_loads` (v-rb shape-2 arg feed).")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/deliver (member push (func (param r ("record" (a (s64)) (b (s64)))) (result ("unit")))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect deliver (op push (-> (Record (: a Int64) (: b Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (deliver) (do (deliver.push (record (= a 1) (= b 2))) (record (= requests (list)) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/deliver.push))
  (output (: (record (= requests ()) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
(case "a typed reducer performing a bytes host arg with a scalar result threads the u64 into the step (via an imposed WIT world)"
  (doc    "SHAPE 14 — a Bytes host-op ARG with a scalar RESULT (hasher.hash : (Bytes) -> u64) driven through an
           imposed WIT world. The reducer on-message performs hasher.hash(m.payload) (a list<u8> ARG, u64 result)
           and threads the u64 into the step's request deadline-nanos = Some(hash). Stubbing hasher.hash -> 42
           and asserting deadline-nanos == Some(42) makes the call load-bearing: the u64 result only reaches the
           output if the Bytes arg lowered and the call succeeded. Migrated from the in-crate wasmtime test
           `a_typed_reducer_with_a_bytes_param_host_import_emits_and_loads` (v-rb shape-2 arg feed 2/6).")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/hasher (member hash (func (param bytes ("list" (u8))) (result (u64)))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect hasher (op hash (-> Bytes UInt64))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (hasher) (record (= requests (list (record (= contract (. m contract)) (= payload (. m payload)) (= token (. m token)) (= deadline-nanos (Option.Some (hasher.hash (. m payload))))))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond hasher.hash (: 42 UInt64)))
  (host-calls (call cadenza:platform/hasher.hash))
  (output (: (record (= requests ((record (= contract (1)) (= payload (2)) (= token (3)) (= deadline-nanos (Some 42))))) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
