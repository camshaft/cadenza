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

;; ============================================================================
;; RULING REVERSED -> B (concierge, 2026-07-28, answer 000000017874). The earlier "wasm-canonical /
;; collapse-to-unit" framing is RETRACTED. Correct rule = TYPE-DIRECTED RENDER:
;;   • VALUE identity stands: unit == () (05-compound:8121). ONE shared empty value.
;;   • But Unit and (Tuple) are DISTINCT TYPES (05-compound:6938 LANDED both backends: "(V.A (tuple))
;;     carries a (tuple) value of TYPE (Tuple), distinct from Unit; comparing them is CDZ0203; renders
;;     (A (tuple)) rather than collapsing an empty tuple to unit"). Render is TYPE-DIRECTED.
;;   • => a value statically typed (Tuple) MUST render (tuple) on ALL paths; a Unit-typed value renders
;;     unit on all paths. NO Ty::Tuple([])->Unit collapse in infer (spec-illegal, breaks 5 pins). NO
;;     15-rows:814 flip (stays (: (tuple) (Tuple))). Option A REJECTED.
;;
;; INVERTED FINDING (corpus-bugfix re-measured trunk 51c0a2983 — the empty element's static type is
;; (Tuple), NOT Unit, in both b and c):
;;   (b) DUAL-REF (let ((v0 (tuple 21.04))) (tuple v0 (tuple (tuple v0 (tuple))))):
;;         wasm -> ...unit)  = BUG (heap/shape_of lower.rs:13760 collapses the (Tuple)-typed element to
;;                             a Unit shape, mis-rendering unit for a (Tuple)-typed value)
;;         rust -> ...(tuple)) = CORRECT (type-directed)   <-- rust is RIGHT, wasm is the bug. (INVERTED
;;                             from my original pin which had it backwards.)
;;   (c) LITERAL (tuple 1 (tuple)): BOTH render (tuple) = CORRECT under B. No bug. (No wasm literal gap
;;       after all — the earlier "wasm self-inconsistent" read was measuring against the wrong canonical.)
;; => SOLE real bug = wasm heap/runtime-encode path collapsing a (Tuple)-typed value to unit. OWNER =
;;    v-wasm-opt (shape_of / value-encode, lower.rs:13760 — must NOT collapse empty Ty::Tuple to Unit
;;    SHAPE for a value whose static type is (Tuple)). rust needs NO change. 814 UNTOUCHED.
;; ON LAND (v-wasm-opt heap-render fix): gate x3 -> (tuple) (type-directed); pin case (b) into 05-compound
;;   or 15-rows beside 6938; baseline x3. The pin's EXPECTED is the (Tuple)-typed (tuple) render, NOT unit.

(case "an empty tuple in element position keeps its (Tuple)-typed (tuple) render on all paths (RULED-B type-directed, ask-17874; 6938 distinct-type)"
  (input  (do (def (main) (let ((v0 (tuple 21.04))) (tuple v0 (tuple (tuple v0 (tuple)))))) (export main)))
  (call main) (output (: (tuple (tuple 21.04) (tuple (tuple (tuple 21.04) (tuple)))) (Tuple (Tuple Float64) (Tuple (Tuple (Tuple Float64) (Tuple)))))))

;; ============================================================================
;; RENDER-PATH AUDIT (v-wasm-opt, 2026-07-28, confirms the B re-scope) — the bug is the WASM HEAP path
;; ONLY, and it is TWO-LANE lockstep (neither half alone fixes it, proven empirically):
;;   • RUST gate driver (cdz_render_expr, cdz-rust-render/src/lib.rs:606-611) -> (tuple). ALREADY correct.
;;   • WASM CONST path (const_value_ast/type_ast, lower.rs:14479/14845) -> (tuple)/(Tuple). ALREADY correct.
;;   • WASM HEAP path -> unit. THE ONLY non-compliant path. Fix = BOTH:
;;       (1) COMPILER (v-wasm-opt): shape_of (lower.rs:13761) emits ShapeNode::Unit for empty Ty::Tuple;
;;           must emit ShapeNode::Tuple([]) so the wire descriptor carries Tuple[0].
;;       (2) RUNTIME (v-runtime): value-encode Shape::Tuple(elems) arm (cdz-runtime/src/lib.rs:2492-2495)
;;           renders unit when elems.is_empty(); must render (tuple) (empty headed list).
;; => NO 814 flip, NO everywhere-element pin, NO rust change. B preserves 814 as (: (tuple) (Tuple)).
;;    This pin (dual-ref heap-path (Tuple)-typed element -> (tuple)) is a REGRESSION WITNESS that lands
;;    GREEN once BOTH halves (v-wasm-opt shape_of + v-runtime value-encode) land LOCKSTEP.
;;    v-wasm-opt routes (2) to v-runtime + gates (1); I gate this witness x3 -> (tuple) on their batch.
