;; HELD PIN (corpus-bugfix, 2026-07-27) — RULED. Concierge/operator ruling: option (b) — KEEP Bytes
;; UNORDERED (landed pin 03-equality-and-observation.sexp:602 stands: the spec blesses NO Bytes
;; total order). rust=231 (silent BTreeSet<Vec<u8>> derived Ord) is a REJECT-GAP; concierge routed
;; v-rust-backend to make Set/Map<Bytes>.to-list DECLINE, matching wasm + the pin. No Bytes order
;; is blessed. wasm's decline was SPEC-CORRECT all along.
;;
;; ⇒ EXPECTED: this case DECLINES on ALL THREE backends (once v-rust-backend's decline lands).
;;   Today: wasm already declines (correct); rust + rust-async compute 231 (the gap). So this pin is
;;   HELD until v-rust-backend's decline lands, then it flips green as a uniform (declines).
;; ON LAND (v-rust-backend Set/Map<Bytes>.to-list decline): rebuild cdz, gate x3 → all DECLINE,
;;   pin into 19-sets.sexp beside the tuple/record/float orderable-decline siblings (and near
;;   03:602's Bytes-order-declines companion), baseline x3 (all pass as declines), roundtrip +
;;   silent-omission + --check, MR; notify v-rust-backend + breaker.

(case "Set.to-list over Bytes elements declines uniformly — the spec blesses no Bytes order (19-sets companion of 03:602)"
  (doc    "A set of runtime `Bytes` elements asks Set.to-list for an ORDER on its elements. Per the
           landed 03-equality-and-observation:602 ruling, Bytes has NO blessed total order (it is
           byte-canonical for EQUALITY only) — so ordering it DECLINES uniformly across backends
           (reject-don't-miscompile), exactly like the tuple/list/sum/float orderable-decline cases.
           This is the COLLECTION-ordering companion of 03:602's bare `< bytes bytes` decline: rust
           must NOT silently order Bytes via its BTreeSet<Vec<u8>> derived Ord (that was a reject-gap
           computing 231); it declines like wasm. No Bytes order is blessed.")
  (input  (do
        (def (main (: n UInt8))
          (do
            (def r (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list n))))
            (def s (Set.of (list (Bytes.of (list 5)) r (Bytes.of (list 1 2)))))
            (def xs (Set.to-list s))
            (def (lat (: i Int64)) (Bytes.len (Option.expect (List.at xs i) "in")))
            (+ (* 100 (lat 0)) (+ (* 10 (lat 1)) (lat 2)))))
        (export main)))
  (declines))
