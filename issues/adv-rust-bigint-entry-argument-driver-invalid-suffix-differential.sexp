; BREAKER FINDING — RUST-TARGET DIFFERENTIAL (sibling of the String-entry-arg finding
; adv-rust-string-entry-argument-driver-E0308-differential.sexp): calling an exported entry whose parameter
; is a BIGINT with a BigInt-literal argument fails to build on the rust target, while wasm cleanly DECLINES.
;   (def (main (: a BigInt)) …) invoked via (call main (: 100000000000000000000N BigInt)):
;     wasm: DECLINES ("compiler can't compile it yet") — clean reject-don't-miscompile
;     rust: artifact did NOT build — error: invalid suffix `N` for number literal
;
; The emitted LIBRARY is VALID (compiles standalone): `pub fn main(a: cdz_num::Big) -> bool { … }`, and an
; IN-BODY BigInt literal is correctly emitted as `cdz_num::Big::from_sign_magnitude_bytes(&[…])` (proper
; Rust). So the fault is the rust-target test DRIVER's BIGINT-ARGUMENT marshalling — the harness that calls
; `main` with the corpus arg `100000000000000000000N` writes the CADENZA literal (with its `N` suffix, and a
; value that overflows i128) RAW into Rust source instead of the `cdz_num::Big::from_sign_magnitude_bytes(…)`
; form the library body uses → `invalid suffix N for number literal` (and would overflow even without the N).
;
; ISOLATED (recompute-before-crying-bug, one axis at a time):
;   - BigInt built INSIDE the program (main takes Int64, `(BigInt.of n)`) → BUILDS + runs on BOTH backends.
;   - BigInt param on a HELPER (main takes Int64, builds the BigInt, passes it) → BUILDS + runs on BOTH.
;   - The library `fn main(a: cdz_num::Big)` + in-body BigInt literal compile standalone (rustc-clean).
;   - Only a BigInt at the EXPORTED-ENTRY / (call …) boundary breaks — the driver marshals the arg wrong.
;   - Int64/Bool/Float entry args → fine on both (every existing corpus (call …) uses these).
;   - NO corpus case anywhere passes a BigInt (or String) ARG across (call …) — the string/BigInt inputs are
;     always built INSIDE the program, so this surface is wholly untested. Same masking as the String sibling.
;   - VERIFIED CURRENT: no rust-backend / xtask commit between my base and trunk, so it reproduces on trunk.
;
; SUGGESTED FIX (v-rust-backend / the gate rust-driver argument marshalling): a BigInt entry argument must be
; emitted as the same `cdz_num::Big::from_sign_magnitude_bytes(&[…])` construction the library body uses for a
; BigInt literal — NOT the raw Cadenza `…N` literal text. Fix ALONGSIDE the String-entry-arg sibling
; (adv-rust-string-entry-argument-driver-E0308-differential.sexp) — both are the rust driver mis-marshalling a
; NON-SCALAR entry argument (String → wrong type/E0308; BigInt → raw N-suffixed literal/invalid suffix). If a
; BigInt entry parameter is intentionally unsupported on rust, DECLINE like wasm — the two backends must agree.
;
; The case below asserts the CORRECT result (a + a = 2·10^20 for a = 10^20 → true). It DECLINES on wasm and
; FAILS TO BUILD on rust today; the point is the two backends diverge — flip to a uniform outcome (both run →
; true, or both decline) when the driver marshals a BigInt arg correctly.

(case "adv rust-bigint-entry: an exported entry with a BigInt parameter is called with a BigInt argument"
  (doc "`(def (main (: a BigInt)) (= (+ a a) 200000000000000000000N))` exported and called `(call main
        100000000000000000000N)` should yield true (10^20 + 10^20 = 2·10^20, exact BigInt arithmetic). The
        emitted rust LIBRARY `fn main(a: cdz_num::Big) -> bool` is valid and its in-body BigInt literal is a
        proper `cdz_num::Big::from_sign_magnitude_bytes(…)`, but the rust-target DRIVER marshals the
        `100000000000000000000N` argument by writing the raw Cadenza literal into Rust source → `invalid
        suffix N for number literal`, failing the build. wasm cleanly DECLINES. A BigInt built inside the
        program, or a BigInt param on a HELPER, builds fine on both backends — so the fault is the
        exported-entry BigInt-argument boundary. Both backends must agree (both run → true, or both decline);
        today they diverge. Sibling of the String-entry-arg driver finding.")
  (input (do (def (main (: a BigInt)) (= (+ a a) 200000000000000000000N)) (export main)))
  (call main (: 100000000000000000000N BigInt))
  (output (: true Bool)))
