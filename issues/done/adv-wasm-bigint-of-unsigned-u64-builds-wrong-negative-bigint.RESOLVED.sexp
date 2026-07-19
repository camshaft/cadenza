; BREAKER FINDING — WASM-backend BigInt.of UNSIGNED miscompile (SILENT WRONG VALUE). The wasm-side twin
; of the rust-backend bug just fixed in fe99dd1ec — that fix was rust-backend-only (backend/rust/expr.rs);
; the WASM path still miscompiles.
;
; `BigInt.of : ∀a.(Int a) -> BigInt`. On a UInt64 value >= 2^63, the wasm backend lowers Core::BigIntOfI64
; to the runtime op `bigint-of-i64`, whose param is AbiValType::S64 (SIGNED — runtime_abi.rs:166-176). So a
; u64 >= 2^63 is reinterpreted as a NEGATIVE i64 and BigInt.of builds the wrong NEGATIVE BigInt. Rust fixed
; this by widening through i128 via the sign-magnitude byte path; wasm has no such widening.
;
; VERIFIED (current trunk, rigorously — recompute-before-crying-bug):
;   - the BigInt LITERAL 2^63 renders correctly (RHS trustworthy).
;   - BigInt.of(UInt64 2^63) == 2^63 (built as 2^62+2^62 in BigInt) -> FALSE on wasm (should be true).
;   - BigInt.of(UInt64 2^63) > 0 -> FALSE on wasm (it's NEGATIVE — the smoking gun).
;   - BigInt.of(UInt64 2^63 - 1) [fits i64] > 0 -> TRUE (the working control; boundary is exactly 2^63).
;   - BigInt.of(UInt64 u64::MAX) > 0 -> FALSE on wasm.
;   - ALL of the above PASS on the RUST backend (fe99dd1ec fixed it there).
;
; SUGGESTED FIX (v-rust-backend / whoever owns the wasm bigint emit): mirror the rust i128-widening on the
; wasm path — for an UNSIGNED source width, route BigInt.of through `bigint-of-bytes` with the value's
; sign-magnitude bytes (the op already exists, runtime_abi.rs:166), instead of the signed `bigint-of-i64`.
; A signedness check on the source Int width at Core::BigIntOfI64 emit distinguishes the two paths.
;
; These cases are graded as the CURRENT (wrong) wasm behavior would FAIL them — they assert the CORRECT
; result, so they FAIL on wasm today (the miscompile) and PASS on rust. They flip to all-pass when wasm is
; fixed. (Filed to .claude/fleet/queue for corpus-bugfix to route + adopt once fixed.)

(case "adv bigint: BigInt.of a UInt64 at 2^63 is a positive BigInt, not a wrong negative"
  (doc "BigInt.of on a UInt64 value at 2^63 (one past Int64.max) must build the POSITIVE big integer 2^63.
        The wasm backend lowers through the SIGNED bigint-of-i64 runtime op, reinterpreting the high bit as
        sign and building a negative BigInt — a silent wrong value. Compared here to 2^62+2^62 (the same
        2^63 assembled by BigInt addition, which cannot coincidentally equal a negative). Correct = true;
        wasm currently returns false. Rust (fe99dd1ec) returns true.")
  (input (do (def (main (: x UInt64))
               (= (BigInt.of x)
                  (+ (BigInt.of (: 4611686018427387904 UInt64)) (BigInt.of (: 4611686018427387904 UInt64)))))
             (export main)))
  (call main (: 9223372036854775808 UInt64))
  (output (: true Bool)))

(case "adv bigint: BigInt.of a UInt64 at 2^63 is greater than zero"
  (doc "The sign smoking-gun: BigInt.of on a UInt64 at 2^63 must be > 0. The wasm sign-reinterpretation
        makes it NEGATIVE, so `> 0` is false — proving the built BigInt has the wrong sign, not merely wrong
        digits. Correct = true; wasm returns false; rust returns true.")
  (input (do (def (main (: x UInt64)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 9223372036854775808 UInt64))
  (output (: true Bool)))

(case "adv bigint: BigInt.of a UInt64 at u64::MAX is greater than zero"
  (doc "The extreme: BigInt.of on the maximum UInt64 (2^64 - 1) must be the positive 18446744073709551615.
        The signed lowering reinterprets it as -1's neighborhood (negative), so `> 0` is false. Correct =
        true; wasm returns false; rust returns true.")
  (input (do (def (main (: x UInt64)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 18446744073709551615 UInt64))
  (output (: true Bool)))

(case "adv bigint: BigInt.of a UInt64 at 2^63 - 1 is positive (the working boundary control)"
  (doc "The control at the boundary: 2^63 - 1 = Int64.max FITS a signed i64, so even the signed lowering
        builds the correct positive BigInt. This passes on BOTH backends today — it pins that the bug is
        exactly the >= 2^63 unsigned range, not BigInt.of on a UInt64 in general.")
  (input (do (def (main (: x UInt64)) (> (BigInt.of x) (BigInt.of 0))) (export main)))
  (call main (: 9223372036854775807 UInt64))
  (output (: true Bool)))

## UPDATE 2026-07-16: v-rust-backend HANDED OFF to v-wasm-opt (needs wasm-LIR u64 byte-materialization; emit site select.rs:6965; fix = signedness branch -> bigint-of-bytes reusing the const path byte-build @9457-9466; rust fix is the value-semantics reference). Owner is now v-wasm-opt.
