;; MISCOMPILE — INVALID WASM (Copilot PR #402, confirmed by corpus-bugfix 2026-07-15, trunk@aef19a3a9). A
;; Ty::Unit element in a HEAP-STORED compound (a multi-payload SumNew variant, or a Tuple/Record) produces
;; INVALID wasm — the component fails to compile.
;;
;; ROOT (backend/wasm/select.rs:1357 + :1418): box_op_ty returns Ok(None) for Ty::Unit, so a Unit payload
;; in a Core::Tuple/Record/multi-payload SumNew STORES without pushing a value (Core::Unit emits nothing)
;; → stack underflow at arr-set/sum-new. (Twin: get_op_ty returns Ok(None) for Unit, leaving an IMM_UNIT
;; handle on the stack when a Unit is PROJECTED, but valtype_of(Unit)=None means Unit must leave NO stack
;; value → stack-type mismatch.) Neighbors the Unit-closure family (pr388 closure_type_index / Unit-param
;; boxed closure) — the shared root is valtype_of(Unit)=None not being consistently handled as "zero
;; slots" across the heap store/project + closure-dispatch paths.
;;
;; FIX DIRECTION: a Unit payload maps to ZERO wasm slots — box_op_ty/get_op_ty must handle Unit as a
;; no-op that neither pushes nor pops a stack value, and sum-new/arr-set/projection layouts must skip a
;; Unit field's slot entirely (store nothing, read nothing). Confirm against the layout the runtime expects.
(case "a Unit element in a multi-payload sum variant compiles to valid wasm"
  (doc    "A sum variant (A Int64 Unit) with a Unit as its 2nd payload; constructing ((. T A) 5 unit) and
           matching out the Int64 must compile + run to 5. Today box_op_ty returns Ok(None) for the Unit
           payload → it isn't pushed → stack underflow at sum-new → INVALID wasm.")
  (input (do
    (type T (A Int64 Unit) (B))
    (def (get (: t T)) (match t (((. T A) n _) n) (((. T B)) 0)))
    (def (main) (get ((. T A) 5 unit)))
    (export main)))
  (output (: 5 Int64)))

;; ── v-wasm-opt confirmation (2026-07-15, trunk@c978178b8) — ALREADY FIXED ──────────────────────────
;; VERIFIED fixed: the case COMPILES to VALID wasm + runs to 5 (was invalid: stack underflow at sum-new).
;; Root fix landed in select.rs: `emit_unit_slot` (:1511) pushes the IMM_UNIT sentinel where a Unit
;; occupies a heap slot, and a Unit projection `Drop`s the slot value (:1531) — so box_op_ty/get_op_ty's
;; `Ok(None)` for Ty::Unit is now consistently handled as "the slot holds the unit sentinel, no boxed
;; payload". Corpus pins on trunk (05-compound-types.sexp): "a Unit element in a multi-payload sum
;; variant compiles to valid wasm" (5780) + "a Unit element between two Int64s in a tuple" (5792). This
;; item is a STALE duplicate of the landed fix. Renaming .RESOLVED.
