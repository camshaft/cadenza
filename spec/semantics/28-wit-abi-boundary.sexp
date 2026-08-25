; General WIT-ABI boundary — typed component exports crossing the canonical-ABI boundary.
;
; This file is the corpus home for the "general WIT-ABI" (shape-2) coverage that used to live as
; in-crate `wasmtime`-running #[test]s in rcdzc (`Component::from_binary` + `run_returns`). Per the
; operator (2026-08-25): migrate every such behavioral test OUT of the compiler and test it once,
; fully end-to-end, in the corpus — reusing this established `.sexp` format rather than a new one.
; The mission retires rcdzc's `wasmtime` dev-dependency once all its behavioral coverage lands here.
;
; A shape-2 case declares a guest that EXPORTS a typed function; the harness compiles it (the WIT
; world is SYNTHESIZED from the guest's own type annotations — no external world artifact needed),
; then `(call <export> <arg>...)` invokes it across the component boundary and `(output ...)` asserts
; the returned value. The interesting content is the ABI marshalling of the param/result TYPES
; (records, options, lists, variants/enums) as they cross the canonical component boundary — a broken
; lift (wrong discriminant, payload offset, or element shape) yields a different observable value.
;
; This file grows one shape at a time as v-rust-backend feeds each anchored shape; each corpus case
; that goes green retires its in-crate `run_returns` equivalent (no coverage gap).
;   SHAPE 1  — option<s64> RESULT: a record-in guest returns a record whose field is an
;     `option<s64>`, exercising the Option discriminant AND the payload lane on both arms.
;   SHAPE 2  — named VARIANT-with-payload RESULT: a discriminated union `Continue | Close(s64)` in a
;     record field, exercising the variant discriminant AND the payload case's s64 lane on both arms.

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
