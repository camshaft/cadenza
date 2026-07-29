; FINDING #43 (breaker): on the WASM target, ordering an ALL-NULLARY sum (every variant
; payload-free) is broken: `compare` yields Equal for DISTINCT variants and `<` is false in
; both directions — while `=` on the same pair correctly says false. Two spec violations:
;   core-semantics.md #Compound Ordering Is Lexicographic — a sum orders by discriminant;
;   core-semantics.md §331 — the boolean ordering operators MUST agree with the three-way
;   compare, and here compare(x,y)=Equal contradicts (= x y)=false.
; rust/rust-async are CORRECT on every witness below; the divergence is wasm-only.
; The tell: a sum with ANY payload variant orders correctly EVEN BETWEEN its nullary variants
; (control below), so the wasm value-cmp walk likely dispatches on the all-nullary REPRESENTATION
; (bare i32 tag / unit payload) and compares only the (absent) payload, skipping the discriminant.
; Blast radius: Ordering values themselves (all-nullary builtin) cannot be ordered; Set/Map keyed
; by an all-nullary sum ENUMERATE in a wrong order (witness 4) though len stays right (= is fine).
;
; Witness 1 — user all-nullary sum: lt=0 eq=0 compare=Equal → ran 2; expected 101 (Less, unequal).
(case "an all-nullary user sum orders by discriminant — Lo below Hi"
  (input  (do
            (type Tri (Lo) (Mid) (Hi))
            (def (mk (: k Int64)) (if (< k 0) (Tri.Lo unit) (if (= k 0) (Tri.Mid unit) (Tri.Hi unit))))
            (def (main (: a Int64) (: b Int64))
              (+ (* 100 (if (< (mk a) (mk b)) 1 0))
                 (+ (* 10 (if (= (mk a) (mk b)) 1 0))
                    (match (compare (mk a) (mk b))
                      ((Ordering.Less _u) 1)
                      ((Ordering.Equal _u) 2)
                      ((Ordering.Greater _u) 3)))))
            (export main)))
  (call   main (: -7 Int64) (: 9 Int64))
  (output (: 101 Int64)))

; Witness 2 — builtin Sign (Neg,Zero,Pos): same shape, same wasm failure (ran 2).
(case "the Sign sum orders Neg below Pos per its declaration"
  (input  (do
            (def (mk (: k Int64)) (if (< k 0) (Sign.Neg unit) (if (= k 0) (Sign.Zero unit) (Sign.Pos unit))))
            (def (main (: a Int64) (: b Int64))
              (+ (* 100 (if (< (mk a) (mk b)) 1 0))
                 (+ (* 10 (if (= (mk a) (mk b)) 1 0))
                    (match (compare (mk a) (mk b))
                      ((Ordering.Less _u) 1)
                      ((Ordering.Equal _u) 2)
                      ((Ordering.Greater _u) 3)))))
            (export main)))
  (call   main (: -7 Int64) (: 9 Int64))
  (output (: 101 Int64)))

; Witness 3 — Ordering values themselves are an all-nullary sum: Less < Greater must hold.
(case "Ordering values order Less below Equal below Greater"
  (input  (do
            (def (mk (: k Int64)) (compare k 0))
            (def (main (: a Int64) (: b Int64))
              (+ (* 10 (if (< (mk a) (mk b)) 1 0))
                 (if (< (mk b) (mk a)) 1 0)))
            (export main)))
  (call   main (: -7 Int64) (: 9 Int64))
  (output (: 10 Int64)))

; Witness 4 — canonical enumeration order: Set.to-list head over {Hi, Mid, Lo} must be Lo (1).
; wasm ran 32 (head=Hi, len right); rust 31 correct.
(case "Set.to-list over an all-nullary sum enumerates in discriminant order"
  (input  (do
            (type Tri (Lo) (Mid) (Hi))
            (def (mk (: k Int64)) (if (< k 0) (Tri.Lo unit) (if (= k 0) (Tri.Mid unit) (Tri.Hi unit))))
            (def (main (: k Int64))
              (do
                (def s (Set.of (list (Tri.Hi unit) (mk k) (Tri.Lo unit))))
                (+ (* 10 (Set.len s))
                   (match (List.at (Set.to-list s) 0)
                     ((Option.Some v) (match v ((Tri.Lo _u) 1) ((Tri.Mid _u) 2) ((Tri.Hi _u) 3)))
                     ((Option.None _u) -1)))))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 31 Int64)))

; Control A (green on all targets today): a sum with a PAYLOAD variant orders correctly even
; between its two nullary variants — the fix's perimeter (must not disturb the boxed-rep walk).
(case "nullary variants of a payload-carrying sum order by discriminant"
  (input  (do
            (type Mix (P Int64) (N1) (N2))
            (def (mk (: k Int64)) (if (< k 0) (Mix.N1 unit) (if (= k 0) (Mix.N2 unit) (Mix.P k))))
            (def (main (: a Int64) (: b Int64))
              (+ (* 10 (match (compare (mk a) (mk b))
                         ((Ordering.Less _u) 1)
                         ((Ordering.Equal _u) 2)
                         ((Ordering.Greater _u) 3)))
                 (if (= (mk a) (mk b)) 1 0)))
            (export main)))
  (call   main (: -1 Int64) (: 0 Int64))
  (output (: 10 Int64)))

; Control B (green everywhere): runtime Bool ordering — false<true with compare/= agreeing.
(case "runtime Bools order false below true with compare and equality agreeing"
  (input  (do
            (def (main (: a Int64) (: b Int64))
              (do
                (def x (= a 1))
                (def y (= b 1))
                (+ (* 100 (if (< x y) 1 0))
                   (+ (* 10 (match (compare x y) ((Ordering.Less _u) 1) ((Ordering.Equal _u) 2) ((Ordering.Greater _u) 3)))
                      (if (= x y) 1 0)))))
            (export main)))
  (call   main (: 0 Int64) (: 1 Int64))
  (output (: 110 Int64)))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk d2ae042a7): SOUNDNESS — the WASM MIRROR of #42 (rust correct).
;;   Witness 1 (all-nullary Tri, compare(Lo,Hi)): wasm -> 2 (Equal, THE BUG; expected 101 = Less+unequal);
;;     rust -> 101 (correct). wasm 2 pass / 4 FAIL (the 4 all-nullary witnesses); rust+rust-async 6/6 green.
;;   DISCRIMINATOR (control, PASSES wasm): a PAYLOAD-carrying sum orders its NULLARY variants correctly —
;;     only an ALL-NULLARY sum breaks. So the wasm value-cmp walk special-cases the all-nullary REP
;;     (bare i32 tag / unit payload) and compares only the ABSENT payload, SKIPPING the discriminant.
;;   Two spec violations: core-semantics #Compound Ordering Is Lexicographic (sum orders by discriminant)
;;     + §331 (boolean ordering ops MUST agree with three-way compare — here compare=Equal contradicts
;;     (= x y)=false). Blast radius: Ordering itself (all-nullary builtin) can't be ordered; Set/Map keyed
;;     by an all-nullary sum enumerate wrong (Set.len stays right since = is fine).
;; OWNER: v-wasm-opt (value-cmp walk; possibly v-runtime). Fix: the all-nullary-sum compare path must
;;   compare the DISCRIMINANT (tag), not dispatch to an absent-payload compare. Controls (payload-sum
;;   nullary pair + runtime Bool) must stay green.
;; PIN IS HELD: baselines carry NO fail rows — a discriminant-correct pin reds the wasm gate NOW (wasm
;;   returns Equal). Lands GREEN once wasm compares the tag. The 6 witnesses expect the discriminant-order
;;   values (rust-matching); 2 controls stay green. ON FIX: gate x3 -> the correct oracles; pin into
;;   03-equality/05-compound (ordering) beside the compound-ordering pins; baseline x3.

;; FIX cf0c05ae8 (v-wasm-opt, 2026-07-29) VERIFIED wasm e2e by corpus-bugfix (throwaway worktree +
;; corpus-bugfix's populated store, runtime c134412661): 5 pass / 1 FAIL. ROOT confirmed = enum-disc
;; (bare i32) routed to ValueCmp heap walk which read disc 0 both sides; fix routes </compare to scalar
;; Core::Compare i32-tag. The 5 scalar witnesses (1,2,3 compare/</three-way + 2 controls) FLIP GREEN.
;; GAP: witness 4 (Set.to-list over all-nullary keys) STILL FAILS on the fix — wasm enumerates Mid-first
;; (32), rust correctly Lo-first (31). The CHAMP/Set key-ENUMERATION ordering path is a SEPARATE locus the
;; </compare guards didn't cover (Set.of/to-list sorts keys via a different comparison entry). Reported to
;; v-wasm-opt: EXTEND cf0c05ae8 to cover Set/Map key-ordering (I lean; one 6-green pin) OR land scalar half
;; now + HOLD witness 4. ON their decision: if extended, gate all 6 -> green + pin; if scalar-only, pin 5
;; green + keep witness 4 HELD for the Set-key follow-up.

;; SPLIT DECISION (v-wasm-opt, 2026-07-29): witness 4 root-caused to a RUNTIME locus, NOT compiler.
;; An all-nullary sum ELEMENT in a Set boxes via box-int; its disc (0/1/2) ALWAYS fixnum_fits so op_box_int
;; returns an IMMEDIATE int, not a heap node. At sort time value_cmp_shaped's Shape::Sum arm (cdz-runtime
;; lib.rs:5991) calls op_sum_disc which on an IMMEDIATE returns 0 unconditionally (lib.rs:1409) → every
;; all-nullary key compares disc 0 → stable sort keeps insertion order → Mid-first → 32. FIX (v-runtime):
;; the Shape::Sum arm must read the disc from an IMMEDIATE operand (imm_as_int / compare immediate ints
;; directly) instead of op_sum_disc→0; payload-carrying sums stay heap-nodes (op_sum_disc unchanged).
;; => cf0c05ae8 (v-wasm-opt compiler fix) is no-regression, fixes 5/6 (</compare/three-way + controls);
;;    v-wasm-opt QUEUES it. Witness 4 routed to v-runtime. PLAN: pin the 5 green witnesses on cf0c05ae8
;;    landing; HOLD witness 4 for the v-runtime value_cmp_shaped imm-disc fix, then complete the pin (all 6).
;; cf0c05ae8 QUEUED to pr-sync (v-wasm-opt, 2026-07-29) — pr-sync re-gates authoritatively + lands; on land pin 5 green (1/2/3 + 2 controls), witness 4 HELD for v-runtime. v-wasm-opt pings on land.
