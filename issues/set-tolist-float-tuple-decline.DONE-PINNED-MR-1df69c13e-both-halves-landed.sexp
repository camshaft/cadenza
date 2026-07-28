;; WIDENED (breaker #34 sweep): the empty-enumeration hits ALL FIVE compound×float-leaf faces on wasm
;; (each 10*Set.len + List.len(to-list), wasm gives N0 where rust gives NN): (1) Map.to-list float-tuple
;; KEYS; (2) Set.to-list float-field RECORDS; (3) Set.to-list float-leaf LISTS; (4) NESTED tuple
;; (tuple (tuple 1.5 0) k); (5) the base float-leaf tuple. Shared compound-sort float-leaf path — every
;; compound kind. One fix site (the orderable-descriptor float-leaf arm) closes all five; the ruling
;; (decline-vs-enumerate) applies uniformly to all. Pin the base tuple face + optionally 1-2 others.

;; HELD PIN (corpus-bugfix, 2026-07-28) — EXPECTED VALUE likely DECLINE, ruling-flagged. Origin:
;; breaker FINDING (issue 000000017431). On trunk 31a5f4f32: Set.to-list over FLOAT-LEAF TUPLES →
;; wasm returns EMPTY (Set.len 3, to-list [] → List.len 0, silent data loss), rust enumerates 3.
;;
;; ⚠ SPEC PRECEDENT (corpus-bugfix lane check): spec/semantics/03-equality-and-observation.sexp:626
;; "a runtime compound containing a float leaf declines ordering — floats offer no total order (§319)"
;; (LANDED, PASS x3) — a float-containing compound has NO blessed total order → ORDERING it must
;; DECLINE. Set.to-list ORDERS its elements (needs a total order on the element shape), so by 03:626
;; the correct answer is a UNIFORM DECLINE on both backends. ⇒ BOTH backends are wrong today: wasm's
;; silent empty-list (worse — returns a VALUE, folds process nothing) AND rust's enumeration (orders a
;; shape the spec says has no order — a reject-gap, same shape as the Set.to-list-Bytes ruling).
;; NEITHER 30 (wasm) NOR 33 (rust) is the oracle — the case should DECLINE.
;;
;; RULING FLAGGED to v-wasm-opt + concierge (does the Set.to-list ORDER-descriptor path inherit
;; 03:626's bare-< decline? I believe yes — same total-order requirement). ON RULING:
;;   (a) DECLINE (likely, per 03:626): wasm stops emitting the empty-list (declines like the Bytes
;;       arm), rust's orderable-descriptor drops the float-compound (declines) → pin (declines) x3.
;;   (b) if a float-compound order IS blessed for collections (contradicts 03:626 — needs operator):
;;       then wasm's empty is the bug, rust=33 the oracle → pin 33.
;; Until ruled: NO (case) with a value — do NOT pin 30 or 33. wasm's silent-empty is a real bug either
;; way (must decline or enumerate, never silently []). Staged repro for whichever owner the ruling picks.

;; RULING CONFIRMED = DECLINE (v-wasm-opt verified against 03:626 §319). Base face as a DECLINE pin —
;; HELD until BOTH backends decline (v-wasm-opt 42b2a02b0 wasm ✓queued; v-rust-backend rust decline
;; routed). On both landing: gate x3 (all declines) + pin here + baseline x3.
(case "Set.to-list over float-leaf tuple elements declines — a float-containing compound offers no total order (§319, 03:626 companion)"
  (input  (do
        (def (main)
          (List.len (Set.to-list (Set.of (list (tuple 1.5 1) (tuple 2.5 2) (tuple -1.0 3))))))
        (export main)))
  (declines))
