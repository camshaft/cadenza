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
; host-op ARG with a scalar u64 RESULT (hasher.hash) threaded into the step's deadline-nanos. SHAPE 15 — a
; COMPOUND result<Bytes, enum> host-import RESULT (run.run) matched Ok(v)->payload / Err->fallback. SHAPE 16 —
; the option<s64> PARAM read (the read side: a record{d: option<s64>} param matched Some(x)->x / None->-1).
; SHAPE 17 — the result<Bytes, enum> PARAM read (Ok(bs)->Bytes.len / Err(e)->10+disc; both arms, enum-err arg).
; SHAPE 18 — a named-VARIANT RESULT writer NAME-matches (not positionally): a reversed guest decl still maps
; each case to the WIT case by name (Continue->continue disc 0, Close->close disc 1). SHAPE 19 — a record PARAM
; whose WIT field order is NOT name-lex is read by NAME (guest reads .contract across a payload,contract WIT order).
; SHAPE 20 — a record RESULT whose WIT field order is NOT name-lex is written by NAME (guest builds {first,second}
; against a second,first WIT result order). SHAPE 21 — a NESTED record param with a bytes leaf (record{a, sub:{b}})
; reads both the outer and the nested list<u8> leaf.
; SHAPE 22 — a record param carrying a list<u8> LEAF beside a scalar (record{data,tag}) reads the bytes leaf
; lifted through guest memory (Bytes.len).
; SHAPE 23 — a record PARAM interface export built via the boundary wrapper (f(record{a})=m.a, f({a:7})=7).
; SHAPE 24 — a nested list<list<s64>> host-op ARG (sink.push) recursively marshalled + invoked e2e. SHAPE 25 — a MULTI-EXPORT
; record interface (two members f,g), one boundary wrapper per member, both run under the interface.
; SHAPE 26 — a no-effects reducer step (empty requests list; dead element-writer derived from the WIT type).
; SHAPE 27 — the CAPSTONE full reducer-step: list<record> requests + 3 byte leaves + option + named variant, every field asserted.
; SHAPE 28 — a list<u8> LEAF param AND a spilled record result in ONE member (both memory paths + all scratch locals).

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
(case "a typed reducer performing a run.run result host op threads the Ok bytes into the step (via an imposed WIT world)"
  (doc    "SHAPE 15 — a COMPOUND result<Bytes, enum> host-import RESULT (run.run : (Bytes,Bytes,Bytes) -> result<list<u8>, variant{timeout,faulted}>)
           driven through an imposed WIT world. The reducer on-message performs run.run(contract,contract,payload) and
           matches the result: Ok(v) -> the request payload is v; Err(_) -> the payload falls back to m.payload. Stubbing
           run.run -> Ok(b\"RAN\") and asserting payload == (82 65 78) makes the compound result<T,E> lift load-bearing (the
           spilled result disc + Ok list<u8> payload). Migrated from the in-crate wasmtime test
           `a_reducer_performing_run_with_a_result_host_result_emits_and_loads` (v-rb shape-2 run.run feed; #3301 landed the
           RunSink host half so this needs NO Entry::Run — host-responses stubs the result + output-assertion suffices).
           The sole-use world builder is RETAINED (a separate WIT-type unit test still reads it), so only the anchor retires.")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/run (member run (func (param program ("list" (u8))) (param contract ("list" (u8))) (param input ("list" (u8))) (result ("result" ("list" (u8)) ("variant" (timeout) (faulted)))))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (type Error (Timeout) (Faulted)) (effect run (op run (-> Bytes Bytes Bytes (Result Bytes Error)))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (run) (record (= requests (list (record (= contract (. m contract)) (= payload (match (run.run (. m contract) (. m contract) (. m payload)) ((Ok v) v) ((Err _e) (. m payload)))) (= token (. m token)) (= deadline-nanos Option.None)))) (= outcome Outcome.Continue)))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-responses (respond run.run (: (Ok (list 82 65 78)) (Result Bytes Error))))
  (host-calls (call cadenza:platform/run.run))
  (output (: (record (= requests ((record (= contract (1)) (= payload (82 65 78)) (= token (3)) (= deadline-nanos (None unit))))) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
(case "an option<s64> param field is read and rebuilt by the wrapper on both arms (via an imposed WIT world)"
  (doc    "SHAPE 16 — the option<T> PARAM lift (the read side, complement to SHAPE 1's option RESULT): the guest
           f takes a record { d: option<s64> } and matches it (Some(x) -> x, None -> -1). Feeding d=Some(42) -> 42
           and d=None -> -1 exercises BOTH arms of the boundary option read (disc None=0/Some=1 + the payload at
           the variant payload offset), which the wrapper rebuilds into the guest option cell. The export crosses
           under the interface cadenza:demo/iface. Migrated from the in-crate wasmtime test
           `an_option_param_field_is_read_by_the_wrapper` (the param-side variant reader, deadline-nanos read shape).")
  (wit-world (world w (export iface (member f (func (param m ("record" (d ("option" (s64))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: d (Option Int64))))) (match (. m d) ((Option.Some x) x) (Option.None (- 0 1)))) (export f)))
  (call f (: (record (= d (Some 42))) (Record (: d (Option Int64)))))
  (output (: 42 Int64))
  (call f (: (record (= d None)) (Record (: d (Option Int64)))))
  (output (: -1 Int64)))
(case "a result<Bytes, enum> param field is read and rebuilt by the wrapper on Ok and Err arms (via an imposed WIT world)"
  (doc    "SHAPE 17 — the result<Ok, Err> PARAM lift (the read side, complement to SHAPE 15's result RESULT): the
           guest f takes a record { a: result<Bytes, Error> } where Error is a 4-case enum, and matches m.a:
           Ok(bs) -> Bytes.len(bs); Err(e) -> 10 + e's decl disc. Feeding Ok([1,2,3]) -> 3 exercises the Bytes Ok
           arm (ptr/len copy-in inside the sum); Err(timeout) -> 10 and Err(faulted) -> 13 exercise the Enum Err
           arm (the flattened disc leaf rebuilt into the guest error cell, disc-preserving) - a misread of the
           Bytes arm's 2 leaves would misalign the enum disc and flip the Err results. The enum-err ARG is passed
           as the render form `(<case> unit)` (cdz-run's coerce_one gained a Type::Enum arm for this). Migrated
           from the in-crate wasmtime test `a_result_bytes_enum_param_field_is_read_by_the_wrapper`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (a ("result" ("list" (u8)) ("enum" timeout missing schema faulted))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Error (Timeout) (Missing) (Schema) (Faulted)) (def (f (: m (Record (: a (Result Bytes Error))))) (match (. m a) ((Result.Ok bs) (Bytes.len bs)) ((Result.Err e) (match e (Error.Timeout 10) (Error.Missing 11) (Error.Schema 12) (Error.Faulted 13))))) (export f)))
  (call f (: (record (= a (Ok (list 1 2 3)))) (Record (: a (Result Bytes Error)))))
  (output (: 3 Int64))
  (call f (: (record (= a (Err (timeout unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 10 Int64))
  (call f (: (record (= a (Err (faulted unit)))) (Record (: a (Result Bytes Error)))))
  (output (: 13 Int64)))
(case "a named-variant result writer name-matches (not positionally) with a reversed guest decl (via an imposed WIT world)"
  (doc    "SHAPE 18 — the named-VARIANT writer keys on the case NAME, not decl position. The guest declares its sum
           in the OPPOSITE order to the WIT variant cases: (type Rev (Close Int64) (Continue)) (Close is guest
           decl-disc 0) against the world's variant { continue, close(s64) } (continue is boundary case 0). A
           name-match maps Close->close (boundary disc 1) and Continue->continue (boundary disc 0); a POSITIONAL
           match would put Close(payload) onto the nullary continue case and the payload-shape guard would reject
           at compile. A green run of BOTH arms (x=0 -> continue, x=5 -> close 5) proves the writer is keyed on the
           case NAME, immune to guest decl reordering. Migrated from the in-crate wasmtime test
           `the_named_variant_writer_name_matches_not_positionally`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (o ("variant" (continue) (close (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Rev (Close Int64) (Continue)) (def (f (: m (Record (: x Int64)))) (record (= o (if (= (. m x) 0) Rev.Continue (Rev.Close (. m x)))))) (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= o (continue unit))) (record (o Rev))))
  (call f (: (record (= x 5)) (Record (: x Int64))))
  (output (: (record (= o (close 5))) (record (o Rev)))))
(case "a record param field is read by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc    "SHAPE 19 — a record PARAM whose WIT field order differs from the guest name-lex order is read by NAME, not
           position. The world declares f's param as record { payload: list<u8>, contract: list<u8> } (WIT order
           payload,contract), while the guest reads (. m contract) and returns Bytes.len(m.contract). Calling with
           payload=[9,9] (len 2) and contract=[1,2,3] (len 3) must return 3 (the contract length): a positional
           misroute would read payload and return 2. Proves the param permute keys on the field NAME across the
           WIT/guest order mismatch. Migrated from the in-crate wasmtime test
           `a_non_name_lex_record_param_permutes_by_name`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (payload ("list" (u8))) (contract ("list" (u8))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) (Bytes.len (. m contract))) (export f)))
  (call f (: (record (= payload (list 9 9)) (= contract (list 1 2 3))) (Record (: contract Bytes) (: payload Bytes))))
  (output (: 3 Int64)))
(case "a record result field is written by NAME when the WIT field order is not name-lexicographic (via an imposed WIT world)"
  (doc    "SHAPE 20 — the record RESULT writer places fields by NAME, not by the guest name-lex slot order. The
           world declares f's result as record { second: s64, first: s64 } (WIT order second,first; name-lex is
           first < second), the shape of a real step/request (declaration-ordered, not alphabetical). The guest
           builds { first: m.x, second: 2*m.x }; the writer must place first at WIT-position 1 and second at
           WIT-position 0, reading each from its guest name-lex slot. f({x:10}) renders (in WIT order) as
           { second: 20, first: 10 } - a positional write would swap them. Migrated from the in-crate wasmtime
           test `a_non_name_lex_record_result_permutes_by_name`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (second (s64)) (first (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) (record (= first (. m x)) (= second (+ (. m x) (. m x))))) (export f)))
  (call f (: (record (= x 10)) (Record (: x Int64))))
  (output (: (record (= second 20) (= first 10)) (record (second Int64) (first Int64)))))
(case "a nested record param with a bytes leaf compiles and runs (via an imposed WIT world)"
  (doc    "SHAPE 21 — a record PARAM with a NESTED record field carrying a list<u8> leaf, the shape of a reducer
           message's sender (a record-within-record with byte leaves). The guest reads both the outer bytes leaf
           and the nested one: f(m: record{a: Bytes, sub: record{b: Bytes}}) = Bytes.len(m.a) + Bytes.len(m.sub.b).
           The wrapper builds the outer value-heap cell with a nested sub-cell for sub, copying each list<u8> leaf
           out of shared memory. f({a:[1,2], sub:{b:[1,2,3]}}) == 5 (len(a)=2 + len(sub.b)=3). Migrated from the
           in-crate wasmtime test `a_nested_record_bytes_param_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (a ("list" (u8))) (sub ("record" (b ("list" (u8))))))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Bytes) (: sub (Record (: b Bytes)))))) (+ (Bytes.len (. m a)) (Bytes.len (. (. m sub) b)))) (export f)))
  (call f (: (record (= a (list 1 2)) (= sub (record (= b (list 1 2 3))))) (Record (: a Bytes) (: sub (Record (: b Bytes))))))
  (output (: 5 Int64)))
(case "a record param carrying a bytes leaf beside a scalar compiles and runs (via an imposed WIT world)"
  (doc    "SHAPE 22 — a record PARAM carrying a list<u8> LEAF beside a scalar (the memory boundary every real
           reducer needs: Message/Step carry list<u8>). The canon lift lowers the incoming `data` list into the
           guest's linear memory, the wrapper copies those bytes into a value-heap Bytes, builds the {data, tag}
           record, and the def returns Bytes.len(data). f({data:[1,2,3,4,5], tag:99}) == 5 proves the copied bytes
           have the right length (bytes-alloc + the copy loop + the memory lift all agree end to end). Migrated
           from the in-crate wasmtime test `a_record_with_a_bytes_leaf_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (data ("list" (u8))) (tag (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: data Bytes) (: tag Int64)))) (Bytes.len (. m data))) (export f)))
  (call f (: (record (= data (list 1 2 3 4 5)) (= tag 99)) (Record (: data Bytes) (: tag Int64))))
  (output (: 5 Int64)))
(case "a record param interface export builds the record via the boundary wrapper and runs (via an imposed WIT world)"
  (doc    "SHAPE 23 — a RECORD-param interface export handled by the boundary WRAPPER (the on-message(message)->step
           shape, record in). The canon lift hands the def the flattened field; the wrapper builds the value-heap
           record handle then calls the def. Guest f(m: record{a: s64}) = m.a; f({a:7}) == 7 proves the wrapper's
           record build (arr-alloc/box-int) + the field read agree end to end. Migrated from the in-crate wasmtime
           test `a_record_param_guest_compiles_and_runs_via_a_wrapper`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (a (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Int64)))) (. m a)) (export f)))
  (call f (: (record (= a 7)) (Record (: a Int64))))
  (output (: 7 Int64)))
(case "a multi-export record interface guest emits a wrapper per member and runs both (via an imposed WIT world)"
  (doc    "SHAPE 25 — a MULTI-EXPORT record-interface guest: the world's interface iface has TWO record-param
           members f(record{a: s64})->s64 and g(record{b: s64})->s64 (the shape a real reducer needs:
           on-message/on-response/on-notification are separate members). The compiler emits one boundary wrapper
           per member appended to the core module. Guest defines both f(m)=m.a and g(m)=m.b; running BOTH under
           the interface (f({a:7})==7, g({b:9})==9) proves each member's wrapper builds its own record + reads its
           own field independently. Migrated from the in-crate wasmtime test
           `a_multi_export_record_interface_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (a (s64)))) (result (s64)))) (member g (func (param m ("record" (b (s64)))) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: a Int64)))) (. m a)) (def (g (: m (Record (: b Int64)))) (. m b)) (export f) (export g)))
  (call f (: (record (= a 7)) (Record (: a Int64))))
  (output (: 7 Int64))
  (call g (: (record (= b 9)) (Record (: b Int64))))
  (output (: 9 Int64)))

(case "a reducer emitting no effects builds an empty-requests step and runs (via an imposed WIT world)"
  (doc    "SHAPE 26 - a reducer that emits NO effects: an empty requests list with a Continue outcome, a very common
           output (a fold that only reads or updates state). The empty list has an unresolved element type, so the
           result writer must derive the dead element writer of the request list from the WIT type alone
           (canon_write_from_wit - the same principle as the None-only option), or emit falls through to a
           wrong-signature component. Migrated from `a_reducer_emitting_no_effects_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (s64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) (record (= requests (list)) (= outcome Outcome.Continue))) (export f)))
  (call f (: (record (= contract (list 1)) (= payload (list 2))) (Record (: contract Bytes) (: payload Bytes))))
  (output (: (record (= requests ()) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option Int64))))) (outcome outcome)))))

(case "a full reducer-step-shaped guest writes every field of the step and runs (via an imposed WIT world)"
  (doc    "SHAPE 27 - the CAPSTONE full reducer-step-shaped guest, the whole result writer end to end. The guest
           returns one request whose contract and token both copy m.contract, payload copies m.payload, and
           deadline-nanos is Some(5), with a Continue outcome. Exercises record permute (step and request are
           declaration-ordered) plus list-of-records plus three byte leaves plus option plus a named variant - the
           reducer-echo step shape. Asserting every field pins the whole step lift. Migrated from
           `a_full_step_shaped_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (s64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8))))))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (def (f (: m (Record (: contract Bytes) (: payload Bytes)))) (record (= requests (list (record (= contract (. m contract)) (= payload (. m payload)) (= token (. m contract)) (= deadline-nanos (Option.Some 5))))) (= outcome Outcome.Continue))) (export f)))
  (call f (: (record (= contract (list 170 187)) (= payload (list 1 2 3))) (Record (: contract Bytes) (: payload Bytes))))
  (output (: (record (= requests ((record (= contract (170 187)) (= payload (1 2 3)) (= token (170 187)) (= deadline-nanos (Some 5))))) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option Int64))))) (outcome outcome)))))

(case "a typed reducer performing a nested list<list<s64>> host arg emits, loads, and runs (via an imposed WIT world)"
  (doc    "SHAPE 24 — a NESTED list<list<s64>> host-op ARG (sink.push : (list<list<s64>>) -> unit) driven through an
           imposed WIT world. The reducer on-message performs sink.push (list (list 1 2) (list 3)) (a unit-result,
           observe-only host op) then returns a Continue step with an empty requests list. Running the guest LOADS
           the emitted component and INVOKES sink.push, exercising the RECURSIVE nested-list arg marshal (the outer
           list of (ptr,count) inner lists, each an i64-strided array) e2e - runtime byte-movement coverage a
           validate-only check cannot provide: a broken nested marshal (wrong inner stride, dropped inner list,
           or list<s64>-instead-of-list<list<s64>>) fails to instantiate or run. The observed host-call sequence
           pins that sink.push actually fired. Runtime coverage for v-rust-backend INCREMENT 1 (#3321, recursive
           nested-list host-arg lowering); complements the in-crate validate_all test which cannot check byte-movement.")
  (wit-world (world w (export guest (member on-message (func (param m ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))))) (result ("record" (requests ("list" ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8))) (deadline-nanos ("option" (u64)))))) (outcome ("variant" (continue) (close ("record" (schema ("list" (u8))) (reason ("list" (u8)))))))))))) (import cadenza:platform/sink (member push (func (param vals ("list" ("list" (s64)))) (result ("unit")))))))
  (component-name "cadenza:platform/guest")
  (input (do (type Outcome (Continue) (Close (Record (: schema Bytes) (: reason Bytes)))) (effect sink (op push (-> (List (List Int64)) Unit))) (def (onMessage (: m (Record (: contract Bytes) (: payload Bytes) (: token Bytes)))) (host (sink) (do (sink.push (list (list 1 2) (list 3))) (record (= requests (list)) (= outcome Outcome.Continue))))) (export onMessage)))
  (call on-message (: (record (= contract (list 1)) (= payload (list 2)) (= token (list 3))) (Record (: contract Bytes) (: payload Bytes) (: token Bytes))))
  (host-calls (call cadenza:platform/sink.push))
  (output (: (record (= requests ()) (= outcome (continue unit))) (record (requests (List (record (contract (List UInt8)) (payload (List UInt8)) (token (List UInt8)) (deadline-nanos (Option UInt64))))) (outcome outcome)))))
(case "a bytes-param leaf and a spilled record result in one member run through both memory paths (via an imposed WIT world)"
  (doc    "SHAPE 28 — BOTH memory boundaries in ONE member: a list<u8> LEAF param AND a spilled record result,
           the exact combined shape of a reducer's on-message(message) -> step. The wrapper uses both memory
           paths (bytes copy-in for the param leaf + result spill) and all four scratch locals without collision.
           f(m: record{data: Bytes}) = let k = Bytes.len(m.data) in { n: k, twice: 2*k }; f({data:[1..7]}) ==
           { n: 7, twice: 14 } proves the copied-in bytes length + the spilled two-field record result agree.
           Migrated from the in-crate wasmtime test `a_bytes_param_and_record_result_guest_compiles_and_runs`.")
  (wit-world (world w (export iface (member f (func (param m ("record" (data ("list" (u8))))) (result ("record" (n (s64)) (twice (s64)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: data Bytes)))) (let ((k (Bytes.len (. m data)))) (record (= n k) (= twice (+ k k))))) (export f)))
  (call f (: (record (= data (list 1 2 3 4 5 6 7))) (Record (: data Bytes))))
  (output (: (record (= n 7) (= twice 14)) (record (n Int64) (twice Int64)))))
