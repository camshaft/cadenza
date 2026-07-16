; BREAKER FINDING — WASM miscompile (decline-don't-miscompile violation, INCOMPLETE FIX of 91c4296d8): a
; single-variant NEWTYPE over a NARROW-width scalar (UInt8/Int8/Int16/Int32), matched with a literal-payload
; arm + a binding arm, emits an INVALID wasm component. The 91c4296d8 fix ("a single-variant newtype literal-
; payload arm reads the erased payload directly") closed the Int64 (and Bool) payload case by reading the raw
; erased scalar instead of the boxed accessor — but it emits the literal-equality compare at i64 width, so a
; NARROW payload (raw rep i32) gets an `i64.eqz` applied to an `i32` value → validation fails.
;
; SYMPTOM: `cdz-run: invalid component: failed to compile: wasm[0]::function[2]`.
;   wasm-tools validate: func 2 failed: type mismatch: expected i64, found i32 (at offset 0x11b)
;   (the INVERSE of the original Int64 finding's "expected i32, found i64" — same op, opposite width).
; WAT body (wasm-tools print) for `(match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))`, W = (Wrap UInt8):
;   (func (param i32) (result i32)
;     local.get 0        ;; the raw UInt8 payload — an i32 (narrow rep), read raw (91c4296d8 did this right)
;     i64.eqz            ;; <- the literal-0 check emitted at i64 width; i64.eqz over the i32 value -> INVALID
;     if (result i32) i32.const 100 else local.get 0)
; The raw-read is correct; the literal COMPARE is mis-widthed (i64 op on the i32 narrow scalar).
;
; ISOLATED (recompute-before-crying-bug, one axis at a time — all on wasm):
;   iso1 UInt8 newtype, binding-only  ((W.Wrap x) x)                 -> WORKS   (raw read, no compare)
;   iso2 bare UInt8 literal-match (no newtype)  (match n (0 100)(x x)) -> WORKS   (non-newtype narrow compare ok)
;   iso3 UInt8 newtype + literal + binding                            -> INVALID (this bug)
;   iso4 Int32 newtype + literal + binding                           -> INVALID (same, all narrow widths)
;   iso5 Int16 newtype + literal + binding                           -> INVALID
;   Int64 newtype + literal + binding (the 91c4296d8 target)          -> WORKS   (i64 compare matches i64 raw)
; So the bug is SPECIFICALLY a NARROW-width (i32-backed) erased newtype payload + a literal-refine arm; the
; fix reconciled handle-vs-raw but not the raw scalar's WIDTH against the literal-compare op width.
; (The rust backend DECLINES the single-variant path — a known rust gap — so this is a wasm-emit bug, no twin.)
;
; SUGGESTED FIX (v-patterns / backend/wasm/select.rs SumCont::LitTest, the 91c4296d8 site): when the erased
; newtype payload is a NARROW scalar (raw rep i32), emit the literal-equality at the payload's ACTUAL width
; (`i32.eqz` / `i32.eq`), not the hard-coded i64. The `holds_handle`=false path already reads the raw scalar;
; carry the payload's wasm width (i32 for Int8/16/32/UInt8, i64 for Int64) into the compare, exactly as the
; bare non-newtype narrow literal-match (iso2) already does. VERIFY emit locus via WAT.
;
; The cases below assert the CORRECT results (n=5 misses the literal → 5 via binding; n=0 hits → 100). They
; FAIL on wasm today (invalid component); the Int64 control PASSES. Flip to pass when the compare is widthed.

(case "adv narrow-newtype UInt8: a literal-payload arm + binding arm misses the literal (n=5 -> 5)"
  (doc "`(match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))` where W = (Wrap UInt8), n=5: the raw payload 5
        misses the `(W.Wrap 0)` literal arm and binds via `(W.Wrap x)` -> 5. On wasm this emits an INVALID
        component — the literal-0 check is `i64.eqz` over the i32 narrow payload (expected i64, found i32).
        Should return 5. The narrow-width analogue of the Int64 case 91c4296d8 fixed.")
  (input (do (type W (Wrap UInt8)) (def (main (: n UInt8)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 5 UInt8))
  (output (: 5 Int64)))

(case "adv narrow-newtype UInt8: the literal arm is selected when the payload matches (n=0 -> 100)"
  (doc "The literal-hit companion: n=0 matches `(W.Wrap 0)` -> 100. Same invalid-component emit today;
        should return 100. Together with the miss case, pins BOTH arms of the narrow-newtype literal
        refinement must compile — the whole match is invalid, not one arm.")
  (input (do (type W (Wrap UInt8)) (def (main (: n UInt8)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 0 UInt8))
  (output (: 100 Int64)))

(case "adv narrow-newtype Int32: the same literal refinement on an Int32 newtype (n=0 -> 100)"
  (doc "The Int32 width: `(match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))` W = (Wrap Int32), n=0 -> 100.
        Also invalid today (i64.eqz over the i32 payload). Pins the bug spans every narrow width (i32-backed),
        not only UInt8 — a fix must carry the payload width for signed narrow newtypes too.")
  (input (do (type W (Wrap Int32)) (def (main (: n Int32)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 0 Int32))
  (output (: 100 Int64)))

(case "adv narrow-newtype CONTROL: an Int64 newtype literal refinement still works (91c4296d8 target)"
  (doc "The control that PASSES: the SAME shape on an Int64 newtype — `(match (W.Wrap n) ((W.Wrap 0) 100)
        ((W.Wrap x) x))` W = (Wrap Int64), n=0 -> 100 — compiles and runs, because the literal compare's
        hard-coded i64 width matches the i64 raw payload. Pins the bug is the NARROW width specifically; the
        Int64 case 91c4296d8 fixed is unaffected.")
  (input (do (type W (Wrap Int64)) (def (main (: n Int64)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64)))

(case "adv narrow-newtype CONTROL: a bare UInt8 literal-match (no newtype) works"
  (doc "The non-newtype control: `(match n (0 100) (x x))` over a bare UInt8 n=0 -> 100 compiles and runs —
        the plain narrow literal-match already emits the compare at i32 width. Pins the bug is the erased-
        NEWTYPE payload path specifically; the ordinary narrow literal-match is correct and is the template
        the newtype path should follow.")
  (input (do (def (main (: n UInt8)) (match n (0 100) (x x))) (export main)))
  (call main (: 0 UInt8))
  (output (: 100 Int64)))
