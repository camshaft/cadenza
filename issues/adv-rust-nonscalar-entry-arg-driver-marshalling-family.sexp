; BREAKER FINDING (FAMILY — consolidates the String + BigInt siblings) — RUST-TARGET DIFFERENTIAL: the rust
; test-DRIVER cannot marshal ANY non-scalar entry argument across the (call …) boundary. For every non-scalar
; entry-parameter type, calling the export with a literal argument fails to build on rust while wasm cleanly
; DECLINES; a SCALAR (Int64/Bool/Float) entry arg works on both. Four distinct rust errors, ONE root cause:
; the driver writes the raw Cadenza literal/expression into Rust source instead of the proper runtime
; construction the library BODY already uses.
;
; OBSERVED (trunk 2558474d6; each library emit is VALID standalone — the fault is the DRIVER's arg marshalling):
;   String  (def (main (: s String)) …) (call main "abc")                 wasm: DECLINES  rust: E0308 mismatched types
;   BigInt  (def (main (: a BigInt)) …) (call main 100N)                  wasm: DECLINES  rust: error: invalid suffix `N`
;   Rational(def (main (: r Rational)) …) (call main 1R)                  wasm: DECLINES  rust: error: invalid suffix `R`
;   Bytes   (def (main (: b Bytes)) …) (call main (Bytes.of (list 1 2 3))) wasm: DECLINES  rust: error: expected expression, found `.`
;   Int64   (def (main (: n Int64)) …) (call main 41)                     wasm: RUNS 42   rust: RUNS 42   (scalar control)
;
; The library is correct: e.g. `pub fn main(r: cdz_num::Rational) -> bool` with an in-body `1R` emitted as
; `cdz_num::Rational::new(Big::from_i64(1), Big::from_i64(1))`; `100N` → `cdz_num::Big::from_sign_magnitude_bytes(…)`.
; But the DRIVER marshals the ARGUMENT by emitting the raw Cadenza text — `100000000000000000000N` (invalid N
; suffix), `1R` (invalid R suffix), `(Bytes.of (list 1 2 3))` (a `.` member-access expression), a bare String
; at a mismatched type — none of which is valid Rust.
;
; ISOLATED (recompute-before-crying-bug): for EACH type, the value built INSIDE the program (main takes Int64,
; constructs it) OR passed to a HELPER builds + runs on BOTH backends; only the value at the EXPORTED-ENTRY /
; (call …) boundary breaks. NO corpus case anywhere passes a non-scalar arg across (call …) — every non-scalar
; input is built inside the program — so this whole surface was untested (the convention that masked it).
; VERIFIED CURRENT: no rust-backend / xtask (gate-driver) commit between my recent base and trunk.
;
; SUGGESTED FIX (v-rust-backend / the gate rust-driver argument marshalling, ONE fix for the family): marshal a
; non-scalar entry argument as the SAME runtime construction the library body uses for that literal —
; `cdz_num::Big::from_sign_magnitude_bytes(…)` for BigInt, `cdz_num::Rational::new(…)` for Rational, the Bytes
; builder for Bytes, an owned `String` (`"…".to_string()`) for String — NOT the raw Cadenza literal/expression
; text. (Or, if non-scalar entry params are intentionally unsupported on rust, DECLINE like wasm — the two
; backends must agree; today wasm declines and rust emits invalid Rust.) Fixing the driver's arg-emit to route
; through the same literal-lowering the body uses closes all four (and any future non-scalar entry type) at once.
;
; The cases below assert the CORRECT results. Each DECLINES on wasm and FAILS TO BUILD on rust today (except
; the Int64 scalar control, which passes both); flip to a uniform outcome (both run, or both decline) when the
; driver marshals non-scalar args correctly.

(case "adv rust-nonscalar-entry String: a String entry argument"
  (doc "`(def (main (: s String)) (String.byte-len s))` + `(call main \"abc\")` → 3. Library `fn main(s:
        String)` is valid; the rust driver marshals the \"abc\" arg at a mismatched type → E0308. wasm declines.")
  (input (do (def (main (: s String)) (String.byte-len s)) (export main)))
  (call main (: "abc" String))
  (output (: 3 Int64)))

(case "adv rust-nonscalar-entry BigInt: a BigInt entry argument"
  (doc "`(def (main (: a BigInt)) (= a 100N))` + `(call main 100N)` → true. Library emits the in-body 100N as
        `cdz_num::Big::from_sign_magnitude_bytes(…)`; the driver writes the raw `100N` arg → invalid suffix N.")
  (input (do (def (main (: a BigInt)) (= a 100N)) (export main)))
  (call main (: 100N BigInt))
  (output (: true Bool)))

(case "adv rust-nonscalar-entry Rational: a Rational entry argument"
  (doc "`(def (main (: r Rational)) (= r 1R))` + `(call main 1R)` → true. Library emits the in-body 1R as
        `cdz_num::Rational::new(…)`; the driver writes the raw `1R` arg → invalid suffix R.")
  (input (do (def (main (: r Rational)) (= r 1R)) (export main)))
  (call main (: 1R Rational))
  (output (: true Bool)))

(case "adv rust-nonscalar-entry Bytes: a Bytes entry argument"
  (doc "`(def (main (: b Bytes)) (Bytes.len b))` + `(call main (Bytes.of (list 1 2 3)))` → 3. The driver
        writes the raw `(Bytes.of (list 1 2 3))` member-access expression → `expected expression, found .`.")
  (input (do (def (main (: b Bytes)) (Bytes.len b)) (export main)))
  (call main (: (Bytes.of (list 1 2 3)) Bytes))
  (output (: 3 Int64)))

(case "adv rust-nonscalar-entry CONTROL: a scalar Int64 entry argument works on both backends"
  (doc "The scalar baseline that PASSES on both: `(def (main (: n Int64)) (+ n 1))` + `(call main 41)` → 42.
        Pins the driver marshals a SCALAR entry arg correctly — the gap is specifically NON-scalar types
        (String/BigInt/Rational/Bytes), whose literals the driver emits as raw Cadenza text.")
  (input (do (def (main (: n Int64)) (+ n 1)) (export main)))
  (call main (: 41 Int64))
  (output (: 42 Int64)))
