;; HELD-FOR-RULING (corpus-bugfix, 2026-07-25): fuzzer differential — a zero-field (tuple) in tuple
;; ELEMENT position renders `unit` on wasm but `(tuple)` on rust (both valid modules, disagree on VALUE).
;; Repro (trunk 8b6a415c1): (do (def (main) (let ((v (tuple 21.04))) (tuple v v (tuple)))) (export main))
;;   wasm → (tuple (tuple 21.04) (tuple 21.04) unit)
;;   rust → (tuple (tuple 21.04) (tuple 21.04) (tuple))
;; Trigger: a let-bound Float64-containing tuple referenced ≥2× + a sibling empty (tuple); single-ref or
;; Int payload does NOT flip; a bare top-level (tuple) — wasm ALSO gives (tuple) (type (Tuple)), not unit.
;;
;; SPEC TENSION (why this needs an operator ruling, NOT a unilateral pin):
;;   • core-semantics.md:187 — "The empty tuple MUST be the unit value, so that unit and () are the same
;;     value." + 05-compound-types 12205 pins (= unit ()) → true.  [suggests (tuple) IS unit → wasm right]
;;   • BUT 05-compound-types 9234 (LANDED, baselined both backends) — a typed (A (Tuple)) payload carries
;;     a "(tuple) VALUE (type (Tuple), distinct from Unit)", renders "(tuple), NOT unit", and "unit and
;;     (tuple) are distinct types (comparing them is CDZ0203)".  [suggests a typed (tuple) stays (tuple) →
;;     RUST right, wasm collapses it wrongly]
;;   • My bare-(tuple) probe: wasm gives (: (tuple) (Tuple)) too (NOT unit) — so wasm is INCONSISTENT with
;;     itself (bare (tuple)→(tuple), but (tuple) as a tuple element→unit).
;; LEANING: the LANDED 9234 pin (typed empty-tuple renders (tuple), distinct from unit) makes RUST look
;;   canonical here and WASM the bug (it collapses a typed (tuple) ELEMENT to unit under the compound-
;;   Float64 dual-ref path — same family as earlier compound-Float64 render bugs). But 187/12205 are in
;;   tension, so ROUTED to concierge for the ruling; on the ruling I pin the winning side + the loser
;;   backend's owner fixes. cc v-wasm-opt + v-rust-backend.
;; (no (case ...) yet — the expected value depends on the ruling)
