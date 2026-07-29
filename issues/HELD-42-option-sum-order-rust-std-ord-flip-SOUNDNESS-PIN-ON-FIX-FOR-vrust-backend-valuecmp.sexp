; FINDING #42 (breaker): rust/rust-async ORDER Option sums by STD's None<Some, not the
; DECLARED Some<None — a cross-target total-order divergence (soundness of the blessed order).
;
; Spec: core-semantics.md #Compound Ordering Is Lexicographic — "A sum MUST be ordered first by
; the discriminant as encoded in its canonical byte form". The prelude declares
; (type (Option a) (Some a) (None)) — sums.rs:80 — so Some=disc 0, None=disc 1, i.e. (Some x) < None.
; The wasm backend's value-cmp walk honors this. The rust backend emits Cadenza Option AS std
; Option<T>, whose DERIVED Ord has None=0 < Some=1 — the FLIPPED order. Result does not diverge
; only because Cadenza's (Ok, Err) happens to match std's declaration order.
;
; Blast radius beyond `<`/`compare`: the canonical Set/Map enumeration order of Option-keyed
; collections diverges cross-target (witness 2 below reads Set.to-list head: wasm gives (Some 1),
; rust gives None → inner match -99). Any program observing sorted order of Options differs.
;
; Probable fix direction (rust backend, expr.rs ~:2836 ValueCmp): the derived-Ord shortcut is only
; valid when the emitted enum's variant order matches the Cadenza declaration; std Option's does NOT.
; Either emit Option as a user enum in decl order, or special-case the comparison (reverse for the
; builtin Option mapping), or route Option compare through the descriptor walk.
;
; Witness 1 — minimal compare divergence (wasm PASS, rust/rust-async FAIL ran→3):
(case "three-way compare orders (Some 3) below None per the declared discriminant order"
  (input  (do
            (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k)))
            (def (main (: a Int64) (: b Int64))
              (match (compare (mk a) (mk b))
                ((Ordering.Less _u) 1)
                ((Ordering.Equal _u) 2)
                ((Ordering.Greater _u) 3)))
            (export main)))
  (call   main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

; Witness 2 — canonical collection order observably diverges (wasm PASS ran 1, rust FAIL ran -99):
(case "Set.to-list over Option elements enumerates Some-first per the declared order"
  (input  (do
            (def (main (: k Int64))
              (do
                (def s (Set.of (list (Some k) (: (None unit) (Option Int64)) (Some 1))))
                (match (List.at (Set.to-list s) 0)
                  ((Option.Some v) (match v ((Option.Some inner) inner) ((Option.None _u) -99)))
                  ((Option.None _u) -1))))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 1 Int64)))

; Witness 3 — boolean < agrees with compare (both flipped together on rust, so agreement HOLDS
; per-target; pinned here as the wasm-truth): (< (Some 3) (None)) = true.
(case "the boolean ordering operator places (Some 3) below None like the three-way compare"
  (input  (do
            (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k)))
            (def (main (: a Int64) (: b Int64))
              (if (< (mk a) (mk b)) 1 0))
            (export main)))
  (call   main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

; Control (no divergence): Result's decl order (Ok, Err) matches std — (< (Ok 3) (Err "e")) = true
; on ALL targets. Pins that the fix must not disturb Result.
(case "Result ordering agrees across targets — Ok below Err on the shared declaration order"
  (input  (do
            (def (mk (: k Int64)) (if (= k 0) (: (Result.Err "e") (Result Int64 String)) (Result.Ok k)))
            (def (main (: a Int64) (: b Int64))
              (if (< (mk a) (mk b)) 1 0))
            (export main)))
  (call   main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

; ADDENDUM (tick 810): the flip propagates into NESTED positions — the fix must cover Option as a
; compound LEAF (tuple field, list element), not only a top-level ValueCmp operand. Witnesses
; (wasm PASS, rust/rust-async FAIL ran→3 and ran→0):
(case "a tuple containing an Option leaf orders by the declared Some-below-None"
  (input  (do
            (def (mk (: k Int64)) (tuple 7 (if (= k 0) (: (None unit) (Option Int64)) (Some k))))
            (def (main (: a Int64) (: b Int64))
              (match (compare (mk a) (mk b))
                ((Ordering.Less _u) 1)
                ((Ordering.Equal _u) 2)
                ((Ordering.Greater _u) 3)))
            (export main)))
  (call   main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

(case "a list of Options orders its elements by the declared Some-below-None"
  (input  (do
            (def (mk (: k Int64)) (list (if (= k 0) (: (None unit) (Option Int64)) (Some k))))
            (def (main (: a Int64) (: b Int64))
              (if (< (mk a) (mk b)) 1 0))
            (export main)))
  (call   main (: 3 Int64) (: 0 Int64))
  (output (: 1 Int64)))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk d2ae042a7): SOUNDNESS cross-backend total-order divergence.
;;   Witness 1 (compare (Some 3) vs None): wasm -> 1 (Less, declared Some<None, CORRECT per
;;     core-semantics #Compound Ordering Is Lexicographic + prelude sums.rs:80 Some=disc0 < None=disc1);
;;     rust -> 3 (Greater, std Option's None<Some, THE BUG). rust+rust-async fail; wasm passes.
;;   Control (Result Ok<Err): PASS both (decl (Ok,Err) matches std Result by luck) — fix must NOT disturb.
;;   Addendum: the flip propagates into NESTED Option leaves (tuple field, list element) — the fix can't
;;     special-case only the top-level ValueCmp operand; the derived-Ord walk sees the flip at every leaf.
;; ROOT (breaker, confirmed plausible): rust backend emits Cadenza Option AS std Option<T> and ValueCmp
;;   (backend/rust/expr.rs ~:2836) trusts std's DERIVED Ord (None<Some) — flipped from the declared order.
;; SEVERITY: soundness of the blessed total order — any program observing sorted order of Options (compare,
;;   <, Set/Map enumeration over Option keys/elements) diverges cross-target. wasm is spec-correct.
;; OWNER: v-rust-backend (ValueCmp derived-Ord shortcut — only valid when emitted variant order == decl
;;   order; std Option's does NOT). Fix: emit Option as a decl-order user enum, OR reverse-compare the
;;   builtin Option mapping, OR route Option compare through the descriptor walk (covering nested leaves).
;; PIN IS HELD: baselines carry NO fail rows — a wasm-correct pin reds the rust gate NOW (rust returns the
;;   flipped value). Lands GREEN once rust matches wasm. The 6 witnesses expect the wasm (declared-order)
;;   values; the Result control must stay green. ON FIX: gate x3 -> the wasm oracles; pin into 03-equality
;;   or 05-compound (ordering) beside the compound-ordering pins; baseline x3.

;; SPLIT DECISION (v-rust-backend, 2026-07-29): compare-side FIXED (21ebe76e2, HELD behind queued Option-C
;; driver daf3ad83f — single-MR cadence). ValueCmp now routes an Option-containing operand through a
;; type-directed compare walk ordering every Option position Some-before-None (declared), delegating
;; Option-free subtrees to native .cmp(). Verified: compare (Some 3) None → Less; nested Option-in-tuple
;; leaf; Some<None; two-Somes-by-payload; Result control unchanged. Gates rust/rust-async/wasm 0-regress.
;; WITNESS 2 (Set.to-list/Map key-enumeration over Option keys) NOT covered: rust Set/Map are BTreeSet/
;; BTreeMap ordering keys by the KEY's DERIVED Ord → an Option key still uses std's flipped None<Some,
;; independent of ValueCmp. Fix = an Ord-WRAPPER around an Option key (like the __CdzF64 float-key wrapper,
;; types.rs ord_key_type) whose Ord uses declared Some<None, OR emit Option as a decl-order enum. Bounded
;; follow-up in v-rust-backend's lane, built after the compare fix + driver land. PLAN: pin witness 1 (+ the
;; relational/compare witnesses) on 21ebe76e2 landing; HOLD witness 2 for the Set/Map-key Ord-wrapper.

;; COMPARE-SIDE SENT (v-rust-backend, 2026-07-29): MR 7392dc3b8 (queued, on clean trunk c2ebd33da after
;; the Option-C driver landed). Gates rust 5083/104/0 + rust-async 5080/107/0 + wasm 5164/23/0 --check
;; 0-regress. ON LAND: pin witness 1 (three-way compare (Some 3)<None) + the </Ordering compare witnesses
;; + the Result control into 03-equality/05-compound (rust now matches wasm). Witness 2 (Set/Map Option-key
;; enumeration) STAYS HELD for v-rust-backend's next MR (BTreeSet/Map Ord-wrapper) — they ping on ship.
