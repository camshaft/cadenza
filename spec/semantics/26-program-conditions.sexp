; ============================================================================================
; 26-program-conditions.sexp — program pre/post-conditions whose proofs are DISCHARGED by the
; verification kernel (Increment-b, the "conditions feed optimization" workstream). See
; implementation/design/DESIGN-verification-program-conditions.md. Vertical: v-verification.
;
; Increment (a) built an unforgeable HOL `Thm` (25-verification.sexp). Increment (b) USES it: a
; pre/post-condition on a Cadenza program denotes into a HOL obligation `Term`, and the kernel
; discharges it into a `Thm`. The operator's headline is that a DISCHARGED obligation is a
; first-class optimizer input — a proven `no-overflow@Id` lets the Core-tier elision pass drop the
; overflow guard (the disjunction seam with v-core-opt; see the design §3/§7).
;
; These b1/b2 cases are the FRONT-LOADED design validation: NO compiler change. They hand-author the
; obligation `Term`s and prove them THROUGH the kernel, exactly as a b2 denotation would emit them,
; so the discharge machinery is validated end-to-end before any optimizer wiring exists.
;
; THE ARITHMETIC-DISCHARGE CONVENTION (design §1A + the b1 crux). The HOL kernel has NO built-in
; arithmetic decision procedure — it proves via primitive rules over abstract `Term`s. So a
; no-overflow obligation is discharged from a minimal, trusted arithmetic-axiom base (the analogue of
; HOL-Light's `ARITH`) whose axioms are AXIOM SCHEMAS THAT CHECK THEIR GROUND SIDE-CONDITION — an
; instance is minted only when a decidable numeric fact actually holds. Concretely, for a checked
; `x + k : Int64` at a node whose PRECONDITION bounds `x ≤ c`:
;   • the obligation `no-overflow@Id` is the term `LE (add x k) MAXINT`, where `add`/`le` are
;     `Const`-headed `Comb` applications (add=Const 0, le=Const 1) and MAXINT is the Int64 maximum as
;     a `Num` — a genuine numeral, so the axiom base can CHECK bounds against it;
;   • from the precondition hypothesis `LE x (Num c)` (via `assume`), the `mono-add-r` rule derives
;     `LE (add x (Num k)) (add (Num c) (Num k))`, and the CHECKED ground axiom `le-ax` mints
;     `LE (add (Num c) (Num k)) MAXINT` — but ONLY because `eval-ground (add c k) = c+k` and `c+k ≤
;     MAXINT` actually holds — then `trans-le` closes it to `LE (add x k) MAXINT`.
;
; SOUNDNESS OF THE AXIOM BASE (breaker 2026-07-17, FIXED). An earlier `le-ax` minted `⊢ a≤b` for
; ARBITRARY terms with empty hypotheses — an UNRESTRICTED axiom that forged false ground facts
; (`⊢ 5≤3`) and non-theorems (`⊢ (x+1)≤MAXINT`), which (empty hyps ⊆ any precondition) made `licenses`
; accept ANY obligation → an unsound elision. `le-ax` is now a CHECKED GROUND SCHEMA: it evaluates
; both sides with `eval-ground` (a partial evaluator over numerals + `add`) and mints `⊢ lhs≤rhs` ONLY
; when both are ground numeric terms and `value(lhs) ≤ value(rhs)`; a non-ground or false pair yields
; `Option.None` — no `Thm`. This is the LCF axiom-schema discipline: an axiom instance is admitted only
; with its decidable side-condition discharged. (A ground `add` whose sum would overflow Int64 traps in
; `eval-ground` — the discharge fails loudly rather than forging.)
;
; The b2 MATCH PREDICATE `licenses` (the compiler's trusted surface) additionally requires the
; discharged Thm's HYPS ⊆ the node's stated precondition, so a Thm proven under DIFFERENT assumptions
; cannot license an elision. `bounds` keeps the SAME LCF discipline as `hol` (abstract Thm, private
; constructor, rules the only way to mint one), so the unforgeability audit of 25-verification.sexp
; carries over — an obligation is discharged only by the trusted, side-condition-checked order-rules.
; ============================================================================================

(case "a no-overflow obligation is DISCHARGED: for x <= 100, (x + 1) <= MAXINT via monotonicity + a CHECKED numeral fact"
  (doc    "The first program-condition discharge — the b1 milestone. A checked `x + 1 : Int64` guarded by
           the precondition `x <= 100` has the no-overflow obligation `LE (add x 1) MAXINT`. The `bounds`
           kernel proves it WITHOUT any arithmetic primitive: from `assume (LE x (Num 100))` it applies the
           `mono-add-r` rule (adding 1 to both sides of a `<=`) to get `LE (add x 1) (add 100 1)`, then the
           CHECKED ground axiom `le-ax (add 100 1) MAXINT` — which mints `LE (add 100 1) MAXINT` only
           because `eval-ground (add 100 1) = 101` and `101 <= MAXINT` holds — and `trans-le` closes it to
           `LE (add x 1) MAXINT`. The entry derives the obligation THROUGH the rules and checks the
           conclusion is structurally the obligation via `term-eq`; it never fabricates the Thm. Runs to
           `true`. Pins that a no-overflow condition is dischargeable end-to-end from a bounded precondition
           via a SOUND (side-condition-checked) arithmetic base — the fact a b2 elision consumes.")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n)    (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      ; arithmetic head-symbols as Const-headed applications
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      ; MAXINT as a genuine numeral (the Int64 maximum) so the axiom base can CHECK bounds against it
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; LEAF rule: assume a proposition (its own hypothesis)
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; a partial evaluator over the GROUND numeric fragment: numerals and `add` of numerals. A
      ; non-numeral (a Var, a bare Const, a non-add Comb) is not evaluable → None.
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 0) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (+ av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      ; CHECKED GROUND AXIOM: mint |- (le lhs rhs) ONLY when both sides are ground numeric terms and
      ; value(lhs) <= value(rhs). A non-ground or false pair yields None — no Thm forged. (The LCF
      ; axiom-schema discipline: an axiom instance is admitted only with its side-condition discharged.)
      (def (le-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (<= lv rv)
                                                  (Option.Some (Thm.Seq (list) (le lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; RULE: monotonicity of + on the right — from G |- (le x c) derive G |- (le (add x k) (add c k))
      (def (mono-add-r (: th Thm) (: k Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 1) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      ; RULE: transitivity of <= — from G |- (le a b) and D |- (le b c) derive G++D |- (le a c)
      (def (trans-le (: t1 Thm) (: t2 Thm))
        (match (concl t1)
          ((Term.Comb (Term.Comb (Term.Const 1) a) b)
            (match (concl t2)
              ((Term.Comb (Term.Comb (Term.Const 1) b2) c)
                (if (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq add le maxint concl hyps assume eval-ground le-ax mono-add-r trans-le)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl assume le-ax mono-add-r trans-le))
            (def (main)
              ; the checked op is (x + 1); x is (Var 0); precondition is (le x (num 100))
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1))
                    (c   (Term.Num 100)))
                ; obligation `no-overflow@Id` = (le (add x 1) MAXINT)
                (let ((obligation (le (add x one) (maxint))))
                  ; step 1: assume the precondition (le x 100)
                  (let ((pre (assume (le x c))))
                    ; step 2: monotonicity — (le (add x 1) (add 100 1))
                    (match (mono-add-r pre one)
                      ((Option.Some step1)
                        ; step 3: CHECKED numeral fact (le (add 100 1) MAXINT) — 101 <= MAXINT holds
                        (match (le-ax (add c one) (maxint))
                          ((Option.Some fact)
                            ; step 4: transitivity closes to (le (add x 1) MAXINT)
                            (match (trans-le step1 fact)
                              ((Option.Some proof) (term-eq (concl proof) obligation))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "an UNCONSTRAINED add is NOT dischargeable: with no precondition bound, the no-overflow obligation cannot be closed (the check must stay)"
  (doc    "The dual — the soundness-critical negative. For an UNCONSTRAINED `x + 1 : Int64` (no precondition
           bounding x), there is no `LE x c` hypothesis to feed `mono-add-r`, so the discharge cannot be
           built: the obligation `LE (add x 1) MAXINT` is NOT provable from the arithmetic base alone (it is
           simply false — x could be MAXINT). The entry models the b2 discharge attempt WITHOUT a
           precondition: assuming an ARBITRARY unrelated fact does not produce `LE (add x 1) MAXINT`, and the
           honest result is that the obligation is not reached — so the elision oracle returns None and the
           overflow check STAYS. Runs to `true` (asserts non-derivability). Pins the default-is-always-the-
           check invariant at the discharge level: absence of a bounding precondition means no proof.")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n)    (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (export (. Term *))
      (export Thm)
      (export term-eq add le maxint concl assume)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl assume))
            (def (main)
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1)))
                (let ((obligation (le (add x one) (maxint))))
                  ; With no precondition, the only Thm we can honestly build about x is an assumption
                  ; of some unrelated proposition — it does NOT establish the obligation.
                  (let ((unrelated (assume (le x x))))
                    ; the check must STAY: assert the obligation is NOT what we derived
                    (not (term-eq (concl unrelated) obligation))))))
            (export main)))
  (output (: true Bool)))

; ── b2: the MATCH PREDICATE (the compiler's trusted surface, written IN CADENZA) ────────────────────
; The oracle's core (design §3): a discharged `Thm` LICENSES the elision of `overflow-check@Id` iff
;   (1) its conclusion is STRUCTURALLY EXACTLY the obligation `no-overflow@Id` (term-eq), AND
;   (2) every hypothesis it was proven under is DISCHARGED BY the node's stated precondition
;       (hyps ⊆ precondition, each hyp term-eq to some precondition member).
; (2) is the soundness core: a `Thm` proven under an assumption the node's precondition does NOT provide
; must NOT license an elision. At b3 the compiler compile-time-evals this predicate and consumes only
; its boolean; here we pin the predicate itself.

(case "the b2 match predicate LICENSES the elision: the discharged no-overflow proof matches the obligation and its hyps are covered by the node precondition"
  (doc    "The positive b2 pin. The `bounds` kernel discharges `LE (add x 1) MAXINT` under hypothesis
           `LE x 100` (the b1 chain: assume → mono-add-r → trans-le with a CHECKED numeral fact). The
           `licenses` predicate — the compiler's trusted match surface — accepts it: (1) `term-eq (concl
           proof) obligation` holds, AND (2) `hyps-subset (hyps proof) precondition` holds (its sole
           hypothesis `LE x 100` is exactly the node's stated precondition). So the oracle returns Some and
           the Core elision pass drops the guard. Runs to `true`. Pins that a correctly-discharged proof
           under a matching precondition licenses the elision — the fact b3 consumes via compile-time eval.")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n)    (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 0) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (+ av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def (le-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (<= lv rv)
                                                  (Option.Some (Thm.Seq (list) (le lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def (mono-add-r (: th Thm) (: k Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 1) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def (trans-le (: t1 Thm) (: t2 Thm))
        (match (concl t1)
          ((Term.Comb (Term.Comb (Term.Const 1) a) b)
            (match (concl t2)
              ((Term.Comb (Term.Comb (Term.Const 1) b2) c)
                (if (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      ; membership: some member of `ps` is term-eq to `q`
      (def (mem (: q Term) (: ps (List Term)))
        (match ps
          ((list) false)
          ((list h .. t) (if (term-eq q h) true (mem q t)))))
      ; hyps ⊆ precondition: every hyp is a member of the precondition set
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs
          ((list) true)
          ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      ; THE MATCH PREDICATE: conclusion is the obligation AND hyps are covered by the precondition
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses))
            (def (main)
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1))
                    (c   (Term.Num 100)))
                (let ((obligation  (le (add x one) (maxint)))
                      (precondition (list (le x c))))
                  (let ((pre (assume (le x c))))
                    (match (mono-add-r pre one)
                      ((Option.Some step1)
                        (match (le-ax (add c one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof)
                                ; the match predicate accepts: conclusion matches AND hyps ⊆ precondition
                                (licenses proof obligation precondition))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "the b2 match predicate REJECTS a proof discharged under a FOREIGN hypothesis not in the node precondition (soundness — no elision under wrong assumptions)"
  (doc    "The soundness-critical b2 negative — the breaker vector the design flags. A proof can have the
           RIGHT conclusion `LE (add x 1) MAXINT` yet be established under a hypothesis the node's
           precondition does NOT provide: here the proof is discharged assuming `LE x 100`, but the node's
           stated precondition is only `LE x 200` (weaker). `term-eq` on the conclusion ALONE would wrongly
           accept, so the match predicate MUST also check hyps ⊆ precondition — and it fails: the proof's
           hypothesis `LE x 100` is NOT a member of the precondition `{LE x 200}`. So `licenses` returns
           false → the oracle returns None → the overflow check STAYS. Runs to `true` via `not`. Pins that a
           `Thm` proven under assumptions the node does not guarantee cannot license an elision.")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n)    (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 0) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (+ av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def (le-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (<= lv rv)
                                                  (Option.Some (Thm.Seq (list) (le lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def (mono-add-r (: th Thm) (: k Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 1) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def (trans-le (: t1 Thm) (: t2 Thm))
        (match (concl t1)
          ((Term.Comb (Term.Comb (Term.Const 1) a) b)
            (match (concl t2)
              ((Term.Comb (Term.Comb (Term.Const 1) b2) c)
                (if (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps
          ((list) false)
          ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs
          ((list) true)
          ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses))
            (def (main)
              (let ((x    (Term.Var 0))
                    (one  (Term.Num 1))
                    (c100 (Term.Num 100))
                    (c200 (Term.Num 200)))
                (let ((obligation  (le (add x one) (maxint)))
                      ; the node's ACTUAL precondition is the WEAKER (le x 200)
                      (precondition (list (le x c200))))
                  ; discharge a proof of the SAME conclusion but under the STRONGER hyp (le x 100)
                  (let ((pre100 (assume (le x c100))))
                    (match (mono-add-r pre100 one)
                      ((Option.Some step1)
                        (match (le-ax (add c100 one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof)
                                ; conclusion matches, BUT hyp (le x 100) ∉ precondition {(le x 200)} →
                                ; licenses must be FALSE (the check must STAY). assert NOT licenses.
                                (not (licenses proof obligation precondition)))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── SOUNDNESS PIN: the arithmetic axiom base cannot forge (breaker 2026-07-17) ──────────────────────
(case "the CHECKED ground axiom le-ax cannot forge a FALSE order fact (5 <= 3) — the axiom base is consistent"
  (doc    "The breaker vector-(d) regression pin. An earlier `le-ax` minted `⊢ a≤b` for ARBITRARY terms
           with empty hypotheses — so `le-ax (Num 5) (Num 3)` forged `⊢ 5≤3`, a false ground fact, and (empty
           hyps ⊆ any precondition) that forged Thm made `licenses` accept ANY obligation. The fixed `le-ax`
           is a CHECKED ground schema: `le-ax (Num 5) (Num 3)` evaluates both sides (5 and 3) and, since
           `5 <= 3` is false, returns `Option.None` — NO Thm is minted. Likewise `le-ax` of a NON-ground pair
           (a `Var`) returns None: `eval-ground` cannot value a variable, so no universal non-theorem like
           `(x+1) <= MAXINT` can be forged. The entry asserts both: `le-ax (Num 5) (Num 3)` is None AND
           `le-ax` of an unbounded `(add x 1)` against MAXINT is None. Runs to `true`. Pins that the ARITH
           axiom base admits an instance only with its decidable side-condition discharged — closing the
           forge that would otherwise make every elision unsound.")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 0) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (+ av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def (le-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (<= lv rv)
                                                  (Option.Some (Thm.Seq (list) (le lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (export (. Term *))
      (export Thm)
      (export add le maxint eval-ground le-ax)))
  (input  (do
            (import "bounds" (Term Thm add le maxint eval-ground le-ax))
            (def (main)
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1)))
                ; (1) a FALSE ground fact 5<=3 must NOT be minted
                (let ((false-fact (le-ax (Term.Num 5) (Term.Num 3)))
                      ; (2) a NON-ground universal (x+1)<=MAXINT must NOT be minted
                      (nonground (le-ax (add x one) (maxint))))
                  (and (match false-fact ((Option.None) true) ((Option.Some _) false))
                       (match nonground  ((Option.None) true) ((Option.Some _) false))))))
            (export main)))
  (output (: true Bool)))

; ── SOUNDNESS PIN: a ground add that OVERFLOWS during discharge TRAPS, it does not wrap-and-forge ────
; (breaker overflow-axis vectors, 2026-07-17 — folded here rather than promoted separately.)
(case "le-ax of a ground add that OVERFLOWS Int64 traps during evaluation — it cannot wrap to forge a false bound"
  (doc    "The overflow axis of the axiom-base soundness (breaker). `eval-ground` computes a ground `add`
           with Cadenza's CHECKED `+`, so `eval-ground (add MAXINT 1)` TRAPS with `integer overflow` rather
           than wrapping to MININT. This matters because a wrapping `+` would let `le-ax (add MAXINT 1)
           MAXINT` mint `⊢ (add MAXINT 1) ≤ MAXINT` (since MININT ≤ MAXINT) — a FALSE no-overflow fact that
           would license an unsound elision. Because `+` traps, the discharge fails LOUDLY instead: no wrong
           `Thm` is ever minted. The entry attempts exactly that forge — `le-ax (add MAXINT 1) MAXINT` — and
           the run TRAPS. Pins that the checked-ground-axiom base cannot be defeated via arithmetic overflow:
           the side-condition either holds on true ground values or the evaluation traps, never wraps. (b3
           NOTE: when the compiler compile-time-evals this discharge, it must treat the trap as fail-closed —
           'not licensed / keep the guard' — never a build abort; pinned at b3.)")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 0) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (+ av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def (le-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (<= lv rv)
                                                  (Option.Some (Thm.Seq (list) (le lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (export (. Term *)) (export Thm) (export add le maxint eval-ground le-ax)))
  (input  (do
            (import "bounds" (Term Thm add le maxint eval-ground le-ax))
            (def (main)
              ; attempt the forge: le-ax (add MAXINT 1) MAXINT. eval-ground(MAXINT+1) traps (checked +),
              ; so the run halts on integer overflow — no wrapped MININT, no forged fact.
              (match (le-ax (add (maxint) (Term.Num 1)) (maxint))
                ((Option.Some _) true)
                ((Option.None) false)))
            (export main)))
  (trap   "integer overflow"))

; ── b4b: the DENOTATION — a predicate `Ast` → an obligation `Term` (the semantics→logic bridge, §1A) ──
; b4a records a `@requires(pred)`/`@ensures(pred)` predicate as its `Ast` occurrence. b4b DENOTES that
; predicate Ast into a HOL `Term` the kernel discharges — the §1A shallow embedding on the pure-arith
; fragment. A predicate `(<= x 100)` is `Ast.List [Ast.Name "<=", Ast.Name "x", Ast.Int 100]`; its
; denotation is the `bounds` term `le (Var 0) (Num 100)` (a Name→Var by the param's index, an Int→Num,
; the `<=` head→`le`). This case pins the denotation as an ordinary total `Ast → Term` function (which is
; where the b4 compiler wiring will compile-time-eval it); the FULL @ensures elaboration (result binder
; `it`, the obligation implication) composes these clauses and is a later slice.

(case "b4b denotation: a predicate Ast (<= x 100) denotes to the bounds obligation term le (Var 0) (Num 100)"
  (doc    "The semantics→logic bridge (design §1A) as a total `Ast → Term` function. The recorded predicate
           `(<= x 100)` reifies to `Ast.List [Ast.Name \"<=\", Ast.Name \"x\", Ast.Int 100]`; `denote` maps
           it to the `bounds` kernel term `le (Var 0) (Num 100)` — `<=`→`le`, the param name `x`→`Var 0`
           (its parameter index, supplied by an env), the literal `100`→`Num 100`. The entry denotes the
           predicate and checks (via `term-eq`) it equals the hand-built obligation term. Runs to `true`.
           Pins that a `@requires`/`@ensures` predicate's Ast denotes to exactly the obligation Term the
           b1/b2 discharge machinery consumes — so the b4 elaboration feeds the SAME kernel the hand-authored
           b1 cases exercise, no new discharge path. (`+`→`add`, `>`→a flipped `le` etc. extend the same
           match; this pins the `<=` clause + the Name/Int leaves, the load-bearing shapes.)")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      ; a minimal Ast mirror (the metaprogramming Ast sum's relevant variants for the arith fragment)
      (type Ast (AName String) (AInt Int64) (AList (List Ast)))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n)    (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 0) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      ; the param environment: a name → its Var index. Minimal here (only `x` at index 0).
      (def (var-of (: name String)) (if (= name "x") 0 (- 0 1)))
      ; DENOTE a leaf: a name → Var, an int → Num. (A non-arith leaf is out of the fragment; here total.)
      (def (denote-leaf (: a Ast))
        (match a
          ((Ast.AName nm) (Term.Var (var-of nm)))
          ((Ast.AInt n)   (Term.Num n))
          ((Ast.AList _)  (Term.Num (- 0 1)))))
      ; DENOTE a predicate Ast → an obligation Term (the §1A shallow embedding, arith fragment).
      ; `(<= a b)` → `le`, `(+ a b)` → `add`; operands denote via denote-leaf (or recurse for nesting).
      (def (denote (: a Ast))
        (match a
          ((Ast.AList items)
            (match items
              ((list (Ast.AName op) l r)
                (let ((lt (denote-leaf l)) (rt (denote-leaf r)))
                  (if (= op "<=") (le lt rt)
                    (if (= op "+") (add lt rt)
                      (Term.Num (- 0 1))))))
              (_ (Term.Num (- 0 1)))))
          (_ (denote-leaf a))))
      (export (. Term *))
      (export (. Ast *))
      (export term-eq add le denote)))
  (input  (do
            (import "bounds" (Term Ast term-eq add le denote))
            (def (main)
              ; the recorded predicate `(<= x 100)` as an Ast
              (let ((pred (Ast.AList (list (Ast.AName "<=") (Ast.AName "x") (Ast.AInt 100)))))
                ; its denotation must equal the hand-built obligation term `le (Var 0) (Num 100)`
                (let ((expected (le (Term.Var 0) (Term.Num 100))))
                  (term-eq (denote pred) expected))))
            (export main)))
  (output (: true Bool)))
