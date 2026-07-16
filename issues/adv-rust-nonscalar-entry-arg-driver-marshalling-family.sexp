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

; ── BREAKER FOLLOW-UP (2026-07-16): the family is BROADER than the scalar-ish types above — it spans COMPOUND
; entry args (List, Option/sum, and any type whose Cadenza literal is not valid Rust), with TUPLE an accidental
; EXCEPTION (Cadenza `(tuple 3 4)` renders as the valid Rust tuple literal `(3, 4)`, so it marshals correctly
; by coincidence). List/Option/etc. keep the raw Cadenza expression (`(list 1 2 3)`, `(Some 5)`) → invalid Rust.
; So the fix scope is "ALL non-scalar entry args EXCEPT Tuple", not just the 4 scalar-ish types. wasm declines all.

(case "adv rust-nonscalar-entry List: a List entry argument fails to marshal on rust"
  (doc "`(def (main (: xs (List Int64))) (List.len xs))` + `(call main (list 1 2 3))` → 3. The rust driver
        writes the raw `(list 1 2 3)` Cadenza expression into Rust source → `expected … found 1`. wasm
        declines. A List built INSIDE the program runs on both backends — the arg boundary is the fault.")
  (input (do (def (main (: xs (List Int64))) (List.len xs)) (export main)))
  (call main (: (list 1 2 3) (List Int64)))
  (output (: 3 Int64)))

(case "adv rust-nonscalar-entry Option: an Option (sum) entry argument fails to marshal on rust"
  (doc "`(def (main (: o (Option Int64))) (match o …))` + `(call main (Some 5))` → 5. The rust driver writes
        the raw `(Some 5)` Cadenza expression into Rust source → `expected … found 5`. wasm declines. An
        Option built INSIDE runs on both — same arg-boundary fault, now for a SUM type.")
  (input (do (def (main (: o (Option Int64))) (match o ((Some n) n) ((None _) -1))) (export main)))
  (call main (: (Some 5) (Option Int64)))
  (output (: 5 Int64)))

(case "adv rust-nonscalar-entry CONTROL: a Tuple entry argument marshals on rust by SYNTACTIC COINCIDENCE"
  (doc "The instructive exception: `(def (main (: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1)))` + `(call main
        (tuple 3 4))` → 7 works on BOTH backends, because Cadenza `(tuple 3 4)` happens to render as the valid
        Rust tuple literal `(3, 4)` — the driver's raw-text emit is accidentally valid here. Pins that Tuple's
        passing is a syntactic coincidence, not correct marshalling — the fix should route ALL non-scalar args
        (Tuple included, for robustness) through the library construction form, not rely on the coincidence.")
  (input (do (def (main (: p (Tuple Int64 Int64))) (+ (. p 0) (. p 1))) (export main)))
  (call main (: (tuple 3 4) (Tuple Int64 Int64)))
  (output (: 7 Int64)))

; STATUS 2026-07-16 (breaker + I verified, trunk 776893af6): PARTIALLY fixed. String entry-arg LANDED (5ad7c66a1, owned-String in gate driver) → RUNS 3 on rust. Family gate 3 pass (Int64+Tuple+String) / 5 FAIL. STILL BROKEN (same driver-marshalling root, need per-type construction form): BigInt (Big::from_sign_magnitude_bytes), Rational (cdz_num::Rational::new), Bytes (Bytes builder), List/Option/sum (value construction not raw Cadenza expr). String fix = the template; v-rust-backend actively working the rest (one rust_call_arg dispatch per my nudge). KEEP OPEN until all 5 pass on rust; then promote family reproducer. Non-scalar RETURNS all work — gap is strictly the arg-input boundary.
