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
