;; FINDING (corpus-bugfix, 2026-07-24, trunk 096c1652a): a RUNTIME BigInt sum-payload LITERAL PROBE
;; build-fails on the rust + rust-async backends with `error[E0605]: non-primitive cast: Big as i64`,
;; even NON-recursive. wasm computes it correctly (40/-1) after v-wasm-opt's 5505b5010 (queued->landed).
;; The rust literal-probe compare renders the BigInt literal test as `Big as i64` (a raw numeric cast)
;; instead of a BigInt comparison. HARD build-fail, NOT an honest `todo` decline. Owner: v-rust-backend.
;; Minimal (non-recursive) repro:
(case "MINIMAL rust runtime BigInt literal probe build-fails E0605"
  (input (do
    (type W (Mk BigInt))
    (def (main (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ (- 0 1))))
    (export main)))
  (call main (: 1 Int64))
  (output (: 40 Int64)))
;; wasm: PASS (40).  rust/rust-async: FAIL "artifact did not build: error[E0605]: non-primitive cast: `Big` as `i64`".
;; The recursive form (walk n down, probe at base) build-fails identically on rust; wasm passes (40 at k=1, -1 at k=2).
