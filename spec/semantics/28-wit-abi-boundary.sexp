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
