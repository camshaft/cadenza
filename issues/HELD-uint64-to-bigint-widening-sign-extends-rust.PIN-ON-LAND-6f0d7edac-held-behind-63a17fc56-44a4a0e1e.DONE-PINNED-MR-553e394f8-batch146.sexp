;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-rust-backend fixes UInt64→BigInt
;; widening on the rust backend. Origin: breaker FINDING (issue 000000017240). CONFIRMED trunk
;; 7f19bc1ca: [main 128] expected 817, rust ran → -799 (wasm PASSES 817). (BigInt.of n) for a runtime
;; u64 binding = 2^63+9: wasm widens UNSIGNED (817); rust + rust-async SIGN-EXTEND the i64 carrier →
;; BigInt = -9223372036854775799, %1000 = -799. wasm-right + rust-wrong ⇒ RUST EMIT bug (shared
;; lowering fine): the rust UInt64→BigInt conversion goes via a sign-extending i64 cast instead of u64
;; before BigInt::from. FIX: cast n to u64 first (cf. 44a4a0e1e BinIntRead cast). 4th u64-binding-family
;; member (runtime 7ff56255f done; const-fold → v-core-opt; rust-E0308 → 44a4a0e1e; this = rust
;; BigInt.of widening). wasm=817 is the oracle. ON FIX: rebuild cdz; gate x3 → 817/9; pin into
;; 06-numeric-model.sexp beside the u64-bin pins; baseline x3; roundtrip + silent-omission + --check; MR.

(case "a top-bit u64 binding widens to BigInt unsigned"
  (input  (do
        (def (main (: x UInt8))
          (match (Bytes.of (list x 0 0 0 0 0 0 9))
            ((bin (u64 n)) (Int64.of (% (BigInt.of n) (BigInt.of 1000))))
            (_ -2)))
        (export main)))
  (call   main (: 128 UInt8)) (output (: 817 Int64))
  (call   main (: 0 UInt8)) (output (: 9 Int64)))
