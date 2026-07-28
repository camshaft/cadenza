;; PIN-ON-LAND: add to spec/semantics/05-compound-types.sexp (or 15-rows) once v-inference's
;; Record.with-over-runtime-record fix (MR 49d6eec14) lands. On trunk ca92177c5 the l6 case still
;; DECLINES 'a record row operation over a runtime record is not yet built' (lower.rs:22368) — HELD.
;;
;; v-inference 49d6eec14: runtime_record_fields builds a fresh record from synth (. record #field)
;; projections when const_record_fields misses but type_of is a concrete Ty::Record. Scoped to
;; Record.WITH only (the reported l6 gap). project/without/merge/pop over a runtime record STILL
;; decline — HOLD those (separate follow-up MR, see rowop-on-runtime-record-projection...PIN-ON-FIX).
;;
;; ON LAND (49d6eec14 on trunk): rebuild cdz, gate l6 PASS on wasm+rust+rust-async, insert near the
;; existing Record.with cases, baseline (1 pass) x3, verify titles-agree/0-dup/0-omission + gate --check
;; all 3 + roundtrip + corpus_roundtrip, commit + MR, notify v-inference.

(case "a Record.with through a runtime record (a projection target) builds a fresh record, not declines"
  (doc    "The runtime-record row-op face (v-inference 49d6eec14). `Record.with` whose TARGET is a RUNTIME
           record — here `(. outer pos)`, a projection of a nested-record param, not a compile-time literal —
           used to DECLINE 'a record row operation over a runtime record is not yet built' (lower.rs:22368):
           lower's row-ops only folded over a compile-time-visible Core::Record. The fix builds a FRESH record
           from per-field projections when const_record_fields misses but type_of is a concrete Ty::Record
           (the field set is static, the heap is immutable-shared, so no new runtime primitive). Here `bump`
           updates the inner `pos.y` through the projected sub-record `(. outer pos)`; the updated field takes
           the new value and the sibling `x` is preserved. `(bump p0 5)` → pos.y = 2+5 = 7. The inline-literal
           form (l4) and flat-param form always worked; this is the first to exercise a row-op on a derived
           runtime record. Both backends. project/without/merge/pop over a runtime record are a separate
           follow-up (still decline until their field-subset/union logic is wired through the same helper).")
  (input  (do
            (def (bump (: outer (Record (: pos (Record (: x Int64) (: y Int64))) (: vel (Record (: x Int64) (: y Int64))))) (: d Int64))
              (Record.with outer pos (Record.with (. outer pos) y (+ (. (. outer pos) y) d))))
            (def (main (: d Int64))
              (do
                (def p0 (record (pos (record (x 1) (y 2))) (vel (record (x 30) (y 40)))))
                (. (. (bump p0 d) pos) y)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))
