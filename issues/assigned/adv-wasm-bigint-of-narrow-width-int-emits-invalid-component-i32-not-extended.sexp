; BREAKER FINDING — WASM-backend: BigInt.of on a NARROW-WIDTH integer (8/16/32-bit, signed OR unsigned)
; emits an INVALID wasm component (decline-don't-miscompile violation, worse than a clean decline). This is
; the direct neighbor the BigInt.of unsigned fix (001f3820d, which handled the u64 path) did NOT cover.
;
; SYMPTOM: `cdz-run: invalid component: failed to compile: wasm[0]::function[6]`. The compiler EMITS a
; module but it fails wasm validation — strictly worse than a clean decline or a defined trap.
;
; ROOT CAUSE (verified via WAT — wasm-tools validate + print):
;   error: func 6 failed to validate — type mismatch: expected i64, found i32
; The runtime op `bigint-of-i64` takes an i64 param (type 2 = (func (param i64) (result i32))), but a
; NARROW-width integer (Int8/16/32, UInt8/16/32) lives in an i32 MACHINE SLOT. The BigInt.of lowering feeds
; that i32 straight into the `call bigint-of-i64` WITHOUT an `i64.extend_i32_s/_u` first — a type-invalid
; call. The full-width Int64/UInt64 path works because the value is already in an i64 slot.
;
; SCOPE (all verified on current trunk):
;   BigInt.of(UInt64 …)  -> PASS (the 001f3820d fix)     BigInt.of(Int64 …)  -> PASS (original path)
;   BigInt.of(UInt32 …)  -> INVALID COMPONENT            BigInt.of(Int32 …)  -> INVALID COMPONENT
;   BigInt.of(UInt16 …)  -> INVALID COMPONENT            BigInt.of(UInt8 …)  -> INVALID COMPONENT
;   ALL narrow widths PASS on the RUST backend — so this is a WASM-ONLY differential.
; So it is NOT unsigned-specific (the unsigned fix's framing); it is ANY narrow width, either sign — the
; missing i32->i64 extension at the bigint-of-i64 call site.
;
; SUGGESTED FIX (v-rust-backend / wasm-bigint-emit owner): at the Core::BigIntOfI64 wasm lowering, when the
; operand's machine slot is i32 (a narrow width < 64), emit `i64.extend_i32_s` for a SIGNED narrow type and
; `i64.extend_i32_u` for an UNSIGNED narrow type before the `call bigint-of-i64` — the sign of the extension
; follows the source Int width's signedness (the same signedness the unsigned fix already inspects for the
; u64 byte-path). Signed narrow → sign-extend; unsigned narrow → zero-extend; then the value fits i64
; losslessly and the call type-checks. (u64 already routes through the bytes path per 001f3820d; this is the
; ≤32-bit slots that still go through bigint-of-i64.)
;
; The cases below assert the CORRECT result; they produce an INVALID component on wasm today (breaker's
; gate reads that as a FAIL) and PASS on rust. They flip to all-pass on wasm once the extension is emitted.

(case "adv bigint-narrow: BigInt.of a UInt32 at its high bit is a positive BigInt (wasm emits invalid component today)"
  (doc "`BigInt.of` on a UInt32 param at 2^31 must build the positive 2147483648. On wasm the narrow i32 slot
        is fed to bigint-of-i64 (i64 param) with no i64.extend, so the module fails validation (invalid
        component). Passes on rust. Correct = true.")
  (input (do (def (main (: x UInt32)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 2147483648 UInt32))
  (output (: true Bool)))

(case "adv bigint-narrow: BigInt.of a signed Int32 is a positive BigInt (narrow path breaks regardless of sign)"
  (doc "The signed-narrow companion proving the bug is NOT unsigned-specific: `BigInt.of` on a signed Int32 =
        5 must be > 0. On wasm the i32-slot value hits bigint-of-i64 without sign-extension → invalid
        component. Passes on rust. Correct = true. Pins that the missing i32->i64 extend affects the signed
        narrow path too, so the fix must sign-extend signed narrows and zero-extend unsigned narrows.")
  (input (do (def (main (: x Int32)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 5 Int32))
  (output (: true Bool)))

(case "adv bigint-narrow: BigInt.of a UInt8 at its high bit widens to the exact value"
  (doc "`BigInt.of` on a UInt8 = 200 must equal the BigInt 200 exactly (zero-extended, not sign-reinterpreted
        as -56). On wasm: invalid component (no zero-extend before bigint-of-i64). Passes on rust. Correct =
        true. The smallest-width case — the zero-extension must not read the high bit of a UInt8 as a sign.")
  (input (do (def (main (: x UInt8)) (= (BigInt.of x) (BigInt.of 200))) (export main)))
  (call main (: 200 UInt8))
  (output (: true Bool)))

(case "adv bigint-narrow: BigInt.of a UInt64 at its high bit still works (the fixed control)"
  (doc "The control that PASSES on both backends today (the 001f3820d fix): `BigInt.of` on a UInt64 at 2^63
        is > 0. Pins that the bug is specifically the NARROW (≤32-bit) i32-slot path, not BigInt.of on
        unsigned in general — the full-width u64 path is already correct.")
  (input (do (def (main (: x UInt64)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 9223372036854775808 UInt64))
  (output (: true Bool)))
