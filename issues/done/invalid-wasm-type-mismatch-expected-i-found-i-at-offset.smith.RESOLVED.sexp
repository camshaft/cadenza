(do (def (main) (match ((fn (v0) (* (tuple) 0)) 0) (_ 0))) (export main))

---
ROUTED to v-inference (corpus-bugfix 2026-07-18, VERIFIED + ISOLATED): check-layer GAP producing invalid wasm.
(match ((fn (v0) (* (tuple) 0)) 0) (_ 0)) -> check rc=0 (unused-param warning only) -> INVALID WASM function[2]
(i32 where i64). SHARP ISOLATION: bare (* (tuple) 0) REJECTS CDZ0201 ("a (Tuple) and an Int64 are different
types"), but the SAME expr INSIDE an immediately-applied closure ((fn (v0) (* (tuple) 0)) 0) PASSES check ->
emits invalid wasm. So the closure BODY arith-type-check is bypassed on the apply path. FIX: type-check a
closure body like any expr — (* (Tuple) Int64) must reject CDZ0201 inside a lambda too; that closes the
invalid-wasm before emit. v-inference (arith operand check through closure-apply). Root = check over-accept,
not a backend bug (backend correctly can't emit arith over a unit tuple).

; ---
; RESOLVED-PENDING-MERGE (v-inference, 2026-07-18, MR e070bc737): the isolation was exact — the
; immediately-applied INLINE lambda body bypassed the arith type-check. ROOT: the apply-path fault-collection
; baseline-subtraction (de-dupes a NAMED def's body faults) wrongly ALSO deleted an INLINE lambda's body
; faults (whose body is never separately collected), so (* (tuple) 0) + unused param slipped past. FIX gates
; the baseline on the callee being a named def -> inline lambdas surface their body faults -> check REJECTS
; CDZ0201 before emit, closing the invalid-wasm downstream. 2101/2101 pass. Retire on land.

; LANDED + VERIFIED (corpus-bugfix 2026-07-18, trunk 28fb7a9a9, confirmed by v-patterns + me): (match
; ((fn (v0) (* (tuple) 0)) 0) (_ 0)) now REJECTS CDZ0201 "a (Tuple) and an Int64 are different types"
; (check rc=1), no invalid wasm. The inline-lambda check-gap fix (e070bc737) landed — the immediately-applied
; lambda body now surfaces its arith type-fault. NOTE: my tick-122 "rc=0" was a tail-1 MISREAD (showed only
; the trailing WARNING line; the CDZ0201 error was on an earlier line, rc was actually 1). Fully resolved.
