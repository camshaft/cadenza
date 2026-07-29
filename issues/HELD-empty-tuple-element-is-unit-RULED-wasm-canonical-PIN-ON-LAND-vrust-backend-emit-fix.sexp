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

;; RE-CONFIRMED trunk f85b2c320 (2026-07-28): a fuzzer smith variant
;;   (do (def (main) (let ((v0 (tuple 21.04))) (tuple v0 (tuple (tuple v0 (tuple)))))) (export main))
;; still splits: wasm inner (tuple)→unit, rust→(tuple). Same trigger family (let-bound Float64 tuple
;; ref'd >=2x + sibling empty (tuple)). Concierge ask 000000016957 STILL UNANSWERED — remains parked,
;; NOT re-escalated (one ruling covers both). Smith dup filed alongside.

;; ============================================================================
;; RULED 2026-07-28 (concierge, ask 000000016957): (tuple) in element position IS UNIT everywhere.
;; WASM CANONICAL, RUST is the bug. Spec is monotone: core-semantics:187 MUST + 01-literals:322 landed
;; ((= unit ()) true) + 15-rows Tuple.split-at-0 renders the empty prefix as `unit` typed Unit, LANDED +
;; baselined pass BOTH backends (the decisive element-position precedent). My 05-compound:9234 counter-
;; cite was a MISREAD (it's map-in-tuple binders — no typed-(tuple)-distinct-from-unit pin exists).
;;
;; REFINED MEASUREMENT (corpus-bugfix, trunk f85b2c320) — the divergence is NOT uniform:
;;   (a) COMPUTED empty tuple (Tuple.split-at (tuple 1 2) 0) -> (tuple unit ...): BOTH backends render
;;       `unit`. PASS both. (landed 15-rows precedent.)
;;   (b) DUAL-REF smith path (let ((v0 (tuple 21.04))) (tuple v0 (tuple (tuple v0 (tuple))))):
;;       wasm -> ...unit) CORRECT ; rust -> ...(tuple)) BUG. <- the ruled divergence.
;;   (c) LITERAL (tuple) element (tuple 1 (tuple)): BOTH backends render `(tuple)` (both diverge from
;;       the ruling — wasm is NOT self-consistent: collapses in (b) but not (c)).
;; => v-rust-backend owns the rust-emit fix (collapse a zero-field Tuple element to unit). Case (c)
;;    additionally needs the WASM literal-empty-tuple-element path to collapse too (flagged to v-rust +
;;    v-wasm-opt). PIN is HELD because baselines carry NO fail rows — a unit-expecting pin reds rust NOW.
;; ON LAND (v-rust-backend rust-emit fix, and wasm literal-collapse if pinning case (c)):
;;   gate x3 -> unit; pin into 05-compound (element position) or 15-rows; baseline x3; MR.

(case "an empty tuple in a tuple element position is the unit value (RULED wasm-canonical, ask-16957)"
  (input  (do (def (main) (let ((v0 (tuple 21.04))) (tuple v0 (tuple (tuple v0 (tuple)))))) (export main)))
  (call main) (output (: (tuple (tuple 21.04) (tuple (tuple (tuple 21.04) unit))) (Tuple (Tuple Float64) (Tuple (Tuple (Tuple Float64) Tuple))))))

;; ============================================================================
;; CONVERGED ROOT-CAUSE (both backends, 2026-07-28) — it is NOT a rust emit bug:
;;   • rust VALUE is already () = unit for the empty element (v-rust-backend emitted b+c).
;;   • wasm RUNTIME-encode (shape_of, lower.rs:13760) already collapses empty Ty::Tuple -> Unit.
;;   • The divergence is the COMPILE-TIME CONST / harness-RENDER path: wasm const_value_ast/type_ast
;;     (lower.rs:14479/14845) build a headed (tuple)/(Tuple) unconditionally; the shared gate harness
;;     cdz_render_expr (xtask/main.rs, test empty_tuple_type_renders_as_the_literal_tuple @ main.rs:6278)
;;     renders a zero-field (Tuple) as literal (tuple). Fix = collapse empty Core::Tuple->unit,
;;     empty Ty::Tuple->Unit (wasm 2-line, v-wasm-opt ready + green on 89 tuple/54 unit unit-tests).
;; BLOCKER / SCOPE (v-wasm-opt): the collapse is NOT element-scoped — it also flips a LANDED GREEN pin.
;;   BLAST RADIUS (corpus-bugfix grep of ALL landed outputs): EXACTLY ONE case flips —
;;   15-rows-and-open-sums.sexp:814 "concatenating two empty tuples" (Tuple.concat (tuple)(tuple)):
;;   currently (: (tuple) (Tuple)) pass on ALL 3 baselines -> (A) (: unit Unit). No other bare-root or
;;   mid-output empty (tuple) exists in the corpus.
;; OPTIONS: (A) collapse EVERYWHERE incl bare-root — re-pin 814 lockstep. (B) element-only — flag-thread,
;;   814 stays (tuple), codifies root/element asymmetry.
;; RECOMMENDATION (corpus-bugfix) = (A): spec monotone (core-semantics:187 MUST, 01-literals:322
;;   (= unit ()) true, AND 814's OWN doc says '(tuple) — which IS the unit value') => 814 is the stale
;;   display pin. Escalated A-vs-B to concierge (ask sent 2026-07-28). ON (A) RULING: ONE lockstep batch
;;   = wasm const-path 2-line (v-wasm-opt) + harness-render fix + rust mirror if any (v-rust-backend) +
;;   MY 814 re-pin (-> (: unit Unit) x3 baselines) + MY new empty-tuple-element pin. I own both corpus
;;   edits and sequence the batch so the gate never reds. Do NOT land any half.
