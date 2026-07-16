; BREAKER FINDING — RUST-BACKEND DIFFERENTIAL MISCOMPILE (emitted Rust fails to compile, E0308): a match on a
; sum whose payload is a NARROW width (UInt8), with a literal-payload arm whose result is WIDER (Int64), emits
; invalid Rust — the function/match result type is left narrow (`u8`) and the arms are emitted at inconsistent
; widths. wasm compiles and runs correctly (the authoritative result); rust `artifact did not build`.
;
; OBSERVED (trunk 6e2aebd13, fresh build):
;   (type Box (A UInt8) (B UInt8))
;   (def (f (: b Box)) (match b ((A 0) 100) ((A x) x) ((B y) y)))   ; literal arm 100:Int64, binding arm x:UInt8
;   (def (main (: n UInt8)) (f (A n)))
;   main(0) -> wasm 100 (Int64) ; rust E0308        main(5) -> wasm 5 (Int64) ; rust E0308
; Emitted Rust (cdz compile --target rust):
;   pub fn main(n: u8) -> u8 { if (n) == 0u8 { (100u64 as i64) } else { n } }
;   ^ return type is `u8` but the match result is Int64: the literal arm is `(100u64 as i64)` (i64) while the
;     else arm is bare `n` (u8) — mismatched `if` arms AND neither matches `-> u8` → E0308 mismatched types.
;
; The match arms `100` (Int64) and `x`/`y` (UInt8) must UNIFY to Int64 (the wasm backend does this — result
; (: 100 Int64), the narrow payload widened). The rust backend leaves the fn/match result type narrow (u8) and
; emits the widened literal arm at i64, so the arms and the return type disagree → the emitted Rust won't build.
;
; ISOLATED (recompute-before-crying-bug, one axis at a time — the trigger is NARROW-payload-from-a-SUM + a
; WIDER literal-arm result, on RUST):
;   Int64 sum payload + literal arm      (type Box (Wrap Int64)…)  -> BOTH backends OK (no widening needed)
;   UInt8 sum payload, BINDING-only      (match b ((A x) x)…)      -> BOTH OK (no literal arm, no width unify)
;   BARE UInt8 literal-match (no sum)     (match n (0 100)(x x))    -> BOTH OK (rust returns 100) — so it is
;                                                                       NOT narrow-literal-match in general
;   UInt8 SUM payload + literal arm       (match b ((A 0) 100)…)    -> wasm OK, RUST E0308 (this bug)
;   (single-variant open, multi-variant closed, and open multi-variant narrow all fail on rust the same way —
;    the common trigger is a narrow payload EXTRACTED FROM A SUM VARIANT then used in a wider-result literal match)
;
; SUGGESTED FIX (v-rust-backend / the match-emit type derivation): when a sum-variant payload of a narrow width
; feeds a literal-refinement match whose arms unify to a WIDER type (Int64), the emitted match/fn result type
; and EVERY arm expression must be at the unified (widened) width — widen the narrow binding arm (`n as i64`)
; and type the fn `-> i64`, matching what the wasm backend already computes ((: 100 Int64)). The bare (non-sum)
; narrow literal-match already widens correctly; the sum-payload-extraction path drops the widening. WAT/emit
; source verified (E0308 is on the `if` arms + return type).
;
; The cases below assert the CORRECT results (wasm-authoritative: hit->100, miss->5, both Int64). They FAIL on
; rust today (E0308, artifact did not build). Flip to pass when the rust match-emit widens the arms + result.

(case "adv rust-narrow-sum-lit: a UInt8 sum-payload literal-match arm hits and widens to Int64"
  (doc "`(match (A n) ((A 0) 100) ((A x) x) ((B y) y))` over `(type Box (A UInt8) (B UInt8))` with n=0: the
        payload 0 hits the `(A 0)` literal arm → 100. The arms 100 (Int64) and x/y (UInt8) unify to Int64, so
        the result is (: 100 Int64) — wasm computes exactly this. On RUST the emit leaves the fn result type
        `u8` and emits the literal arm as `(100u64 as i64)` against a `u8` else arm → E0308 (artifact did not
        build). Should return 100.")
  (input (do (type Box (A UInt8) (B UInt8)) (def (f (: b Box)) (match b ((A 0) 100) ((A x) x) ((B y) y))) (def (main (: n UInt8)) (f (A n))) (export main)))
  (call main (: 0 UInt8))
  (output (: 100 Int64)))

(case "adv rust-narrow-sum-lit: the miss binds the narrow payload, widened to the Int64 result"
  (doc "The miss companion: n=5 misses `(A 0)` and binds `(A x)` → the UInt8 payload 5 widened to the match's
        Int64 result type → 5. Same E0308 on rust today. Together with the hit case, pins that BOTH arms must
        be emitted at the unified Int64 width — the narrow binding arm widened, not the literal arm narrowed.")
  (input (do (type Box (A UInt8) (B UInt8)) (def (f (: b Box)) (match b ((A 0) 100) ((A x) x) ((B y) y))) (def (main (: n UInt8)) (f (A n))) (export main)))
  (call main (: 5 UInt8))
  (output (: 5 Int64)))

(case "adv rust-narrow-sum-lit CONTROL: an Int64 sum payload literal-match works on both backends"
  (doc "The width control that PASSES on both: the SAME shape with an Int64 payload `(type Box (Wrap Int64))`
        needs no widening (arms already Int64) → 100 on hit. Pins the bug is the NARROW payload widening on
        rust specifically, not sum-payload literal-match in general.")
  (input (do (type Box (Wrap Int64) (Other Int64)) (def (f (: b Box)) (match b ((Wrap 0) 100) ((Wrap x) x) ((Other y) y))) (def (main (: n Int64)) (f (Wrap n))) (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64)))

(case "adv rust-narrow-sum-lit CONTROL: a BARE UInt8 literal-match (no sum) widens correctly on both backends"
  (doc "The non-sum control that PASSES on both: `(match n (0 100) (x x))` over a bare UInt8 n=0 → 100. The
        bare narrow literal-match already widens the binding arm to the Int64 result on rust; pins the gap is
        the sum-variant PAYLOAD EXTRACTION path dropping that widening, not narrow literal-match in general.")
  (input (do (def (main (: n UInt8)) (match n (0 100) (x x))) (export main)))
  (call main (: 0 UInt8))
  (output (: 100 Int64)))
