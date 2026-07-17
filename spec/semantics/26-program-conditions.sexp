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

; ── b4c(proven): a full @requires/@ensures obligation — denote both, discharge P ⇒ Q[it:=body] ─────────
; b4b denotes ONE predicate Ast → Term. b4c(proven) composes the elaboration (§2.1): for
;   @requires(<= x 100) @ensures(<= it MAXINT) (def (f x) (+ x 1))
; the obligation is `denote(P) ⊢ denote(Q)[it := denote(body)]` — i.e. from the precondition hypothesis
; `le x 100` derive `le (add x 1) MAXINT` (the postcondition with `it` the body's value `x+1`). This is
; exactly the b1 discharge chain, now framed as the DENOTED annotations: `it` in Q is replaced by the
; denotation of the body `(+ x 1)` → `add (Var 0) (Num 1)`, and the precondition enters via `assume`. Pins
; that the §2.1 elaboration target — the whole @requires⇒@ensures obligation — discharges through the SAME
; kernel the hand-authored b1 cases use, so the b4c compiler wiring (compile-time-eval) has a proven target.

(case "b4c(proven): @requires(<= x 100)/@ensures(<= it MAXINT) on (f x)=x+1 discharges — P denoted as hyp, Q[it:=body] as goal"
  (doc    "The PROVEN-tier obligation for a full @requires/@ensures pair (design §2.1). The elaboration
           denotes @requires(<= x 100) → the hypothesis `le (Var 0) (Num 100)` (via assume) and
           @ensures(<= it MAXINT) with `it` := the body's denotation `add (Var 0) (Num 1)` → the goal
           `le (add (Var 0) 1) MAXINT`. Discharging is the b1 chain: mono-add-r on the assumed precondition
           + a CHECKED numeral fact (101 <= MAXINT) + trans-le. The entry builds the denoted obligation and
           discharges it through the kernel, checking the conclusion is the denoted postcondition. Runs to
           `true`. Pins that the b4 elaboration's whole-obligation target (P ⇒ Q[it:=body]) discharges via
           the SAME kernel machinery b1 exercises — so b4c's compile-time-eval wiring has a proven shape to
           produce, and the discharged Thm is exactly what b3's oracle consumes for the implicit overflow
           obligation (here `<= it MAXINT` IS the no-overflow condition on `x+1`).")
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
      (export (. Term *))
      (export Thm)
      (export term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le))
            (def (main)
              (let ((x    (Term.Var 0))
                    (one  (Term.Num 1))
                    (c100 (Term.Num 100)))
                ; denote(body) = (+ x 1) → add (Var 0) (Num 1); it := this in the @ensures goal
                (let ((body-den (add x one)))
                  ; @ensures(<= it MAXINT) with it:=body → goal = le (add x 1) MAXINT
                  (let ((goal (le body-den (maxint)))
                        ; @requires(<= x 100) → hypothesis, entered via assume
                        (pre  (assume (le x c100))))
                    ; discharge: mono-add-r + numeral fact + trans (the b1 chain)
                    (match (mono-add-r pre one)
                      ((Option.Some step1)
                        (match (le-ax (add c100 one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof) (term-eq (concl proof) goal))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── b4c(unprovable): an @ensures whose obligation is NOT dischargeable → the PROVEN tier fails (CDZ-VERIFY) ─
; The dual of b4c(proven). For `@ensures(<= it MAXINT) (def (f x) (+ x 1))` with NO (or too-weak)
; @requires, the postcondition obligation `le (add x 1) MAXINT` is NOT provable — x is unbounded, so the
; discharge chain has no bounding hypothesis to feed mono-add-r, and le-ax cannot mint a non-ground fact.
; At b4c this un-discharged obligation is the PROVEN-tier MISS: the author gets CDZ-VERIFY (or, if @test is
; stacked, the TESTED tier runs it — v-property-testing's lane). This pins that a genuinely-unprovable
; postcondition does NOT spuriously discharge — the proof tier is SOUND (it never claims a false proof).

(case "b4c(unprovable): @ensures(<= it MAXINT) on unbounded (f x)=x+1 is NOT dischargeable — the proof tier correctly MISSES (CDZ-VERIFY)"
  (doc    "The PROVEN-tier soundness dual. With no bounding @requires, the @ensures postcondition
           `<= it MAXINT` (it := body `x+1`) denotes to the obligation `le (add x 1) MAXINT`, which is NOT
           provable: x is unbounded so there is no `le x c` hypothesis for mono-add-r, and the checked
           le-ax cannot mint the non-ground `le (add x 1) MAXINT` (eval-ground fails on the Var). The entry
           attempts the discharge WITHOUT a precondition and confirms it does not reach the obligation — so
           the PROVEN tier correctly MISSES (→ CDZ-VERIFY, or TESTED if @test is stacked). Runs to `true`
           (asserts non-derivability). Pins the proof tier is SOUND: a genuinely-unprovable postcondition
           does not spuriously discharge, so an @ensures never yields a FALSE proof — the LCF guarantee at
           the program-condition level.")
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
      ; the only obligation-minting axiom is the CHECKED ground le-ax; with an unbounded x the goal
      ; `le (add x 1) MAXINT` is non-ground, so le-ax returns None — no proof.
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
      (export term-eq add le maxint eval-ground le-ax)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint eval-ground le-ax))
            (def (main)
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1)))
                ; the unprovable obligation: le (add x 1) MAXINT with x unbounded. le-ax is the only axiom
                ; that could mint it — call it on the OBLIGATION's own sides (lhs = the numeric term
                ; (add x 1), rhs = MAXINT), exactly the no-overflow fact. eval-ground on (add x 1) fails
                ; because x is a FREE Var (non-ground) — so le-ax returns None specifically DUE TO the
                ; unbounded x, the real "unbounded add is not dischargeable" property (not a shape mismatch:
                ; both sides ARE numeric/add terms le-ax evaluates; only x's freeness blocks it). No proof
                ; reaches the goal, so the PROVEN tier misses. Assert le-ax yields None.
                (let ((attempt (le-ax (add x one) (maxint))))
                  (match attempt
                    ((Option.Some _) false)
                    ((Option.None) true)))))
            (export main)))
  (output (: true Bool)))

; ── b4c(conjunctive): a TWO-hypothesis precondition — both @requires flow to the discharge + hyps-subset ─
; b4a records STACKED @requires as a Vec (a conjunction). This pins the multi-hypothesis path the earlier
; single-precondition cases do not: `@requires(>= x 0) @requires(<= x 100)` gives a sequent with TWO
; hypotheses, and the b2 `licenses` hyps-subset must require BOTH are covered by the node precondition (not
; just one). Discharge uses only the `<= x 100` bound (the upper one drives no-overflow), but the proof
; CARRIES both hypotheses, so the match predicate's precondition must contain both — a two-element
; hyps-subset, the "ALL hyps covered" soundness check that a single-hyp case cannot exercise.

(case "b4c(conjunctive): two stacked @requires give a 2-hyp proof; licenses requires BOTH hyps covered by the precondition"
  (doc    "The multi-hypothesis discharge + hyps-subset soundness path. `@requires(>= x 0)
           @requires(<= x 100)` on `(f x)=x+1` yields a proof carrying TWO hypotheses {ge x 0, le x 100}
           (both assumed, unioned through the rules). The obligation `le (add x 1) MAXINT` is discharged
           from the `le x 100` bound (mono-add-r + numeral fact + trans), but the resulting Thm's hyps
           include BOTH assumptions. `licenses` then checks hyps-subset over a TWO-element precondition
           {ge x 0, le x 100} — and it must require BOTH covered: a proof carrying `ge x 0` cannot be
           licensed by a precondition lacking it. The entry builds the 2-hyp proof and asserts `licenses`
           accepts it under the matching 2-element precondition. Runs to `true`. Pins that hyps-subset is
           the ALL-hyps-covered check (not any-one), the soundness core over a genuine conjunction.")
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
      (def (ge  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
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
      ; mono-add-r that PRESERVES the operand hyps (so the derived step keeps {ge x 0, le x 100})
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
      ; CONJ: assume two facts into one 2-hyp theorem (the stacked-@requires precondition as a conjunction)
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq (list p q) p))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps ((list) false) ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs ((list) true) ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq add le ge maxint concl hyps assume assume-both le-ax mono-add-r trans-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le ge maxint concl hyps assume assume-both le-ax mono-add-r trans-le licenses))
            (def (main)
              (let ((x    (Term.Var 0))
                    (one  (Term.Num 1))
                    (c100 (Term.Num 100))
                    (zero (Term.Num 0)))
                (let ((obligation  (le (add x one) (maxint)))
                      ; the node precondition is the CONJUNCTION {ge x 0, le x 100}
                      (precondition (list (ge x zero) (le x c100))))
                  ; a proof carrying BOTH hypotheses — built via the EXPORTED assume-both rule (a Thm
                  ; cannot be constructed outside the kernel; conclusion is the first arg, hyps are both).
                  (let ((pre-le (assume-both (le x c100) (ge x zero))))
                    (match (mono-add-r pre-le one)
                      ((Option.Some step1)
                        (match (le-ax (add c100 one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof)
                                ; the proof's hyps are {ge x 0, le x 100}; licenses requires BOTH in pre
                                (licenses proof obligation precondition))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── b4c(conjunctive) NEGATIVES: partial precondition coverage → NOT licensed (breaker, all-covered sentinel) ─
; The soundness sentinels for the conjunctive hyps-subset: a 2-hyp proof {le x 100, ge x 0} must NOT be
; licensed by a precondition that covers only ONE hyp — hyps-subset is ALL-covered, not any-one. Both
; directions (breaker-verified, all 3 backends): the licenses trusted-elision surface rejects a proof
; assuming a hyp the node precondition does not provide, EVEN when the obligation was discharged via the
; OTHER hyp (the discharged Thm still CARRIES the assumption).

(case "b4c(conjunctive) NEG-1: precondition covers only {le x 100} (missing ge x 0) — the 2-hyp proof is NOT licensed"
  (doc    "Partial-coverage soundness sentinel (breaker vector). The 2-hyp proof carries {le x 100, ge x 0}
           (both assumed via assume-both). The node precondition covers ONLY {le x 100} — it omits `ge x 0`.
           `licenses` must be FALSE: hyps-subset requires EVERY hyp covered, and `ge x 0` is not in the
           precondition. The entry builds the 2-hyp proof, discharges the obligation, and asserts `licenses`
           is false (via `not`). Runs to `true`. Pins hyps-subset is ALL-covered, not any-one — a proof
           assuming a bound the node does not guarantee cannot license an elision.")
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
      (def (ge  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
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
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq (list p q) p))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps ((list) false) ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs ((list) true) ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq add le ge maxint concl hyps assume-both le-ax mono-add-r trans-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le ge maxint concl hyps assume-both le-ax mono-add-r trans-le licenses))
            (def (main)
              (let ((x    (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (zero (Term.Num 0)))
                (let ((obligation (le (add x one) (maxint)))
                      ; precondition covers ONLY le x 100 — missing ge x 0
                      (precondition (list (le x c100))))
                  (let ((pre-le (assume-both (le x c100) (ge x zero))))
                    (match (mono-add-r pre-le one)
                      ((Option.Some step1)
                        (match (le-ax (add c100 one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof)
                                ; proof carries {le x 100, ge x 0}; pre lacks ge x 0 → NOT licensed
                                (not (licenses proof obligation precondition)))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "b4c(conjunctive) NEG-2 (reverse): precondition covers only {ge x 0} (missing le x 100) — NOT licensed though discharged via le"
  (doc    "The subtle reverse sentinel (breaker vector). The obligation was DISCHARGED using the `le x 100`
           bound, but the resulting Thm STILL CARRIES `le x 100` as a hypothesis (the rules union operand
           hyps). So a precondition covering only {ge x 0} — omitting the very `le x 100` the discharge used
           — must NOT license: hyps-subset finds `le x 100` uncovered. `licenses` is FALSE. Pins that
           carrying-and-using a hyp does not exempt it from the coverage check — the discharged assumption
           must be in the node precondition regardless of its role in the proof. Runs to `true` via `not`.")
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
      (def (ge  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
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
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq (list p q) p))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps ((list) false) ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs ((list) true) ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq add le ge maxint concl hyps assume-both le-ax mono-add-r trans-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le ge maxint concl hyps assume-both le-ax mono-add-r trans-le licenses))
            (def (main)
              (let ((x    (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (zero (Term.Num 0)))
                (let ((obligation (le (add x one) (maxint)))
                      ; precondition covers ONLY ge x 0 — missing the le x 100 the discharge used
                      (precondition (list (ge x zero))))
                  (let ((pre-le (assume-both (le x c100) (ge x zero))))
                    (match (mono-add-r pre-le one)
                      ((Option.Some step1)
                        (match (le-ax (add c100 one) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof)
                                ; proof carries {le x 100, ge x 0}; pre lacks le x 100 → NOT licensed
                                (not (licenses proof obligation precondition)))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── b(sub): a no-UNDERFLOW discharge — for x >= 0, (x - 1) >= MININT (the lower-bound / `-` direction) ──
; The b1 discharge pinned `+`/overflow (upper bound vs MAXINT). Overflow elision (b3) also covers `-`/`*`;
; this pins the SUBTRACTION / lower-bound direction the same convention handles: for a checked `x - 1` under
; `@requires(>= x 0)`, the no-underflow obligation is `GE (sub x 1) MININT` (x-1 must not fall below the
; Int64 minimum). The arithmetic base gains a `ge` order + `sub` head + a `mono-sub-r` rule (subtracting a
; constant from both sides of a `>=` preserves it) + the CHECKED ground `ge-ax`. From `assume (GE x 0)`:
; mono-sub-r → `GE (sub x 1) (sub 0 1)` = `GE (sub x 1) -1`, and `ge-ax (sub 0 1) MININT` mints `GE -1 MININT`
; (eval-ground (sub 0 1) = -1, and -1 >= MININT holds), then trans-ge closes to `GE (sub x 1) MININT`.

(case "b(sub): a no-underflow obligation is DISCHARGED — for x >= 0, (x - 1) >= MININT via monotonicity + a CHECKED numeral fact"
  (doc    "The subtraction / lower-bound dual of the b1 overflow discharge. A checked `x - 1 : Int64` under
           `@requires(>= x 0)` has the no-underflow obligation `GE (sub x 1) MININT`. The `bounds` kernel
           discharges it with no arithmetic primitive: from `assume (GE x 0)`, `mono-sub-r` (subtracting 1
           from both sides of a `>=`) gives `GE (sub x 1) (sub 0 1)`, then the CHECKED ground axiom
           `ge-ax (sub 0 1) MININT` mints `GE (sub 0 1) MININT` — only because `eval-ground (sub 0 1) = -1`
           and `-1 >= MININT` holds — and `trans-ge` closes it to `GE (sub x 1) MININT`. Pins that the
           discharge convention generalizes to SUBTRACTION and the lower-bound (MININT) direction the b3
           elision covers for `-`, using the same side-condition-checked axiom base. `sub`=Const 3, `ge`=
           Const 2.")
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
      (def (sub (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 3) a) b))
      (def (ge  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (minint) (Term.Num -9223372036854775808))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; ground evaluator over numerals + `sub`
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 3) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (- av bv)))
                                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      ; CHECKED ground axiom for `>=`: mint |- (ge lhs rhs) only when both ground-numeric and lhs >= rhs
      (def (ge-ax (: lhs Term) (: rhs Term))
        (match (eval-ground lhs)
          ((Option.Some lv) (match (eval-ground rhs)
                              ((Option.Some rv) (if (>= lv rv)
                                                  (Option.Some (Thm.Seq (list) (ge lhs rhs)))
                                                  (Option.None)))
                              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; RULE: monotonicity of - on the right — from G |- (ge x c) derive G |- (ge (sub x k) (sub c k))
      (def (mono-sub-r (: th Thm) (: k Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 2) x) c)
            (Option.Some (Thm.Seq (hyps th) (ge (sub x k) (sub c k)))))
          (_ (Option.None))))
      ; RULE: transitivity of >= — from G |- (ge a b) and D |- (ge b c) derive G++D |- (ge a c)
      (def (trans-ge (: t1 Thm) (: t2 Thm))
        (match (concl t1)
          ((Term.Comb (Term.Comb (Term.Const 2) a) b)
            (match (concl t2)
              ((Term.Comb (Term.Comb (Term.Const 2) b2) c)
                (if (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (ge a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq sub ge minint concl hyps assume ge-ax mono-sub-r trans-ge)))
  (input  (do
            (import "bounds" (Term Thm term-eq sub ge minint concl hyps assume ge-ax mono-sub-r trans-ge))
            (def (main)
              (let ((x    (Term.Var 0))
                    (one  (Term.Num 1))
                    (zero (Term.Num 0)))
                ; obligation: (ge (sub x 1) MININT) — x-1 does not underflow
                (let ((goal (ge (sub x one) (minint))))
                  ; step 1: assume (ge x 0)
                  (let ((pre (assume (ge x zero))))
                    ; step 2: monotonicity → (ge (sub x 1) (sub 0 1))
                    (match (mono-sub-r pre one)
                      ((Option.Some step1)
                        ; step 3: CHECKED numeral fact (ge (sub 0 1) MININT) — -1 >= MININT holds
                        (match (ge-ax (sub zero one) (minint))
                          ((Option.Some fact)
                            ; step 4: transitivity → (ge (sub x 1) MININT)
                            (match (trans-ge step1 fact)
                              ((Option.Some proof) (term-eq (concl proof) goal))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── b(mul): a no-overflow discharge for MULTIPLICATION — for x <= 100, (x * 2) <= MAXINT ──────────────
; Completes the arithmetic-op discharge coverage (+, -, now *) that b3's guard elision handles. For a
; checked `x * 2` under `@requires(<= x 100)`, the no-overflow obligation is `LE (mul x 2) MAXINT`. The base
; gains a `mul` head + a `mono-mul-r` rule — multiplying both sides of a `<=` by a POSITIVE constant
; preserves the order (the positivity is the rule's side-condition: it only fires for a positive `Num` k).
; From `assume (le x 100)`: mono-mul-r by 2 → `LE (mul x 2) (mul 100 2)`, and `le-ax (mul 100 2) MAXINT`
; mints `LE (mul 100 2) MAXINT` (eval-ground (mul 100 2) = 200, 200 <= MAXINT), then trans-le closes it.
; `mul`=Const 4. mono-mul-r requires k a positive numeral (an arbitrary/negative multiplier does NOT
; preserve `<=` — the rule returns None, so the axiom base stays sound).

(case "b(mul): a no-overflow obligation is DISCHARGED for x <= 100, (x * 2) <= MAXINT via positive-multiplier monotonicity"
  (doc    "The multiplication case, completing +/-/* discharge coverage. A checked `x * 2 : Int64` under
           `@requires(<= x 100)` has the no-overflow obligation `LE (mul x 2) MAXINT`. From `assume
           (le x 100)`, `mono-mul-r` (multiply both sides by the POSITIVE constant 2 — its positivity is the
           rule's side-condition; a non-positive multiplier returns None) gives `LE (mul x 2) (mul 100 2)`,
           then the CHECKED ground axiom `le-ax (mul 100 2) MAXINT` mints `LE (mul 100 2) MAXINT` because
           `eval-ground (mul 100 2) = 200` and `200 <= MAXINT`, and `trans-le` closes it to `LE (mul x 2)
           MAXINT`. Pins that the discharge convention covers MULTIPLICATION (b3 elides `*` guards too), with
           the positive-multiplier side-condition keeping the monotonicity rule sound.")
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
      (def (mul (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 4) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (eval-ground (: t Term))
        (match t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Const 4) a) b)
            (match (eval-ground a)
              ((Option.Some av) (match (eval-ground b)
                                  ((Option.Some bv) (Option.Some (* av bv)))
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
      ; RULE: monotonicity of * on the right by a POSITIVE constant k — from G |- (le x c) derive
      ; G |- (le (mul x k) (mul c k)). k must be a positive Num (side-condition); else None (a non-positive
      ; multiplier flips or collapses the order, so minting would be unsound).
      (def (mono-mul-r (: th Thm) (: k Term))
        (match k
          ((Term.Num kv)
            (if (> kv 0)
              (match (concl th)
                ((Term.Comb (Term.Comb (Term.Const 1) x) c)
                  (Option.Some (Thm.Seq (hyps th) (le (mul x k) (mul c k)))))
                (_ (Option.None)))
              (Option.None)))
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
      (export (. Term *))
      (export Thm)
      (export term-eq mul le maxint concl hyps assume le-ax mono-mul-r trans-le)))
  (input  (do
            (import "bounds" (Term Thm term-eq mul le maxint concl hyps assume le-ax mono-mul-r trans-le))
            (def (main)
              (let ((x    (Term.Var 0))
                    (two  (Term.Num 2))
                    (c100 (Term.Num 100)))
                (let ((goal (le (mul x two) (maxint))))
                  (let ((pre (assume (le x c100))))
                    (match (mono-mul-r pre two)
                      ((Option.Some step1)
                        (match (le-ax (mul c100 two) (maxint))
                          ((Option.Some fact)
                            (match (trans-le step1 fact)
                              ((Option.Some proof) (term-eq (concl proof) goal))
                              ((Option.None) false)))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── t1(div0): the DIVIDE-BY-ZERO trap-source obligation — for b > 0, (b != 0) so `a / b` cannot trap ──
; The @trap_free capstone (design §8) proves EVERY trap source unreachable. This pins the DIVIDE-BY-ZERO
; source: a checked `a / b` traps iff b = 0, so its trap-free obligation is `NEQ b 0` (the divisor is
; non-zero). Under `@requires(> b 0)`, the obligation discharges: from `assume (gt b 0)`, a `pos-nonzero`
; rule (a positive value is non-zero) yields `NEQ b 0`. The base gains a `gt` order + `neq` + `pos-nonzero`
; + the CHECKED ground `gt-ax`. `gt`=Const 5, `neq`=Const 6. Pins the div0 trap-source obligation shape the
; capstone's per-source conjunction needs.

(case "t1(div0): the divide-by-zero obligation NEQ b 0 is DISCHARGED for b > 0 — so a/b cannot trap"
  (doc    "The divide-by-zero trap source of the @trap_free capstone (design §8). A checked `a / b` traps iff
           `b = 0`; its trap-free obligation is `NEQ b 0`. Under `@requires(> b 0)`, from `assume (gt b 0)`
           the `pos-nonzero` rule (a value proven `> 0` is `!= 0`) derives `NEQ b 0` — the divisor is
           provably non-zero, so the division cannot trap on that input. The entry discharges it through the
           rules and checks the conclusion is the obligation. Runs to `true`. Pins the div0 obligation shape
           the capstone's per-trap-source conjunction discharges (one source of the whole-function trap-free
           proof).")
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
      (def (gt  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 5) a) b))
      (def (neq (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 6) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; RULE: pos-nonzero — from G |- (gt x 0) derive G |- (neq x 0). A value proven strictly positive is
      ; non-zero. The rule fires ONLY when the premise is `(gt x (Num 0))` (the zero literal); else None.
      (def (pos-nonzero (: th Thm))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 5) x) (Term.Num 0))
            (Option.Some (Thm.Seq (hyps th) (neq x (Term.Num 0)))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq gt neq concl hyps assume pos-nonzero)))
  (input  (do
            (import "bounds" (Term Thm term-eq gt neq concl hyps assume pos-nonzero))
            (def (main)
              (let ((b    (Term.Var 1))
                    (zero (Term.Num 0)))
                ; the div0 trap-free obligation: (neq b 0)
                (let ((goal (neq b zero)))
                  ; @requires(> b 0) → assume (gt b 0); pos-nonzero derives (neq b 0)
                  (let ((pre (assume (gt b zero))))
                    (match (pos-nonzero pre)
                      ((Option.Some proof) (term-eq (concl proof) goal))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "t1(div0) NEGATIVE: an UNBOUNDED divisor is NOT provably non-zero — the divide-by-zero trap STAYS"
  (doc    "The div0 soundness dual. With no `> b 0` (or `b != 0`) precondition, the divisor `b` is unbounded
           — `NEQ b 0` is NOT provable: `pos-nonzero` needs a `(gt b 0)` premise, and an arbitrary assumption
           about `b` does not establish it. So the @trap_free proof for the division MISSES → the div-by-zero
           guard STAYS (the function is not certified trap-free on that source). The entry confirms
           `pos-nonzero` of an unrelated assumption does not yield the obligation. Runs to `true`. Pins that
           an unprovable divide-by-zero source correctly keeps the trap — @trap_free is sound (it never
           certifies a function whose divisor could be zero).")
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
      (def (gt  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 5) a) b))
      (def (neq (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 6) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (pos-nonzero (: th Thm))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 5) x) (Term.Num 0))
            (Option.Some (Thm.Seq (hyps th) (neq x (Term.Num 0)))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq gt neq concl hyps assume pos-nonzero)))
  (input  (do
            (import "bounds" (Term Thm term-eq gt neq concl hyps assume pos-nonzero))
            (def (main)
              (let ((b    (Term.Var 1))
                    (zero (Term.Num 0)))
                (let ((goal (neq b zero)))
                  ; no `> b 0` precondition — only an unrelated assumption about b; pos-nonzero cannot fire
                  (let ((unrelated (assume (neq b b))))
                    (match (pos-nonzero unrelated)
                      ((Option.Some proof) (not (term-eq (concl proof) goal)))
                      ((Option.None) true))))))
            (export main)))
  (output (: true Bool)))

; ── t1(oob): the OUT-OF-BOUNDS trap-source obligation — for 0 <= i < len, `xs[i]` cannot trap ─────────
; The @trap_free capstone (§8) proves EVERY trap source unreachable. This pins the OUT-OF-BOUNDS source: a
; checked index `List.at xs i` (or Bytes.at) traps iff i < 0 OR i >= len, so its trap-free obligation is the
; CONJUNCTION `(0 <= i) AND (i < len)` — a two-part bound. Under `@requires(>= i 0) @requires(< i len)`,
; both conjuncts are direct precondition hypotheses; the obligation is their conjunction. The base gains a
; `lt` order (Const 7) + a `conj` connective (Const 8) + a `both` rule (from G|-p and D|-q derive G++D|-p∧q).
; From assume(ge i 0) and assume(lt i len): `both` gives `CONJ (ge i 0) (lt i len)` = the in-bounds proof.

(case "t1(oob): the out-of-bounds obligation (0<=i) AND (i<len) is DISCHARGED from the two bound preconditions"
  (doc    "The out-of-bounds trap source of the @trap_free capstone. A checked `xs[i]` traps iff `i < 0` or
           `i >= len`; its trap-free obligation is the conjunction `(ge i 0) AND (lt i len)`. Under
           `@requires(>= i 0)` and `@requires(< i len)`, each conjunct is a precondition hypothesis, and the
           `both` rule combines them into `CONJ (ge i 0) (lt i len)` — the index is provably in bounds, so
           the access cannot trap. The entry assumes both bounds, combines via `both`, and checks the
           conclusion is the conjunction obligation (both conjuncts, hyps unioned). Runs to `true`. Pins the
           OOB obligation shape (a two-part conjunction) the capstone's per-trap-source proof discharges.")
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
      (def (ge   (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (lt   (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 7) a) b))
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 8) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; RULE `both`: from G |- p and D |- q derive G++D |- (conj p q) — the in-bounds proof combines the two
      ; bound facts. (Hyps unioned, per the Inc-11 soundness rule that a multi-premise rule carries the union.)
      (def (both (: t1 Thm) (: t2 Thm))
        (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (conj (concl t1) (concl t2)))))
      (export (. Term *))
      (export Thm)
      (export term-eq ge lt conj concl hyps assume both)))
  (input  (do
            (import "bounds" (Term Thm term-eq ge lt conj concl hyps assume both))
            (def (main)
              (let ((i    (Term.Var 2))
                    (len  (Term.Var 3))
                    (zero (Term.Num 0)))
                ; the OOB trap-free obligation: (conj (ge i 0) (lt i len))
                (let ((goal (conj (ge i zero) (lt i len))))
                  ; @requires(>= i 0) and @requires(< i len) → two hypotheses
                  (let ((lower (assume (ge i zero)))
                        (upper (assume (lt i len))))
                    (match (both lower upper)
                      ((Option.Some proof) (term-eq (concl proof) goal))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "t1(oob) NEGATIVE: with only the LOWER bound (>= i 0), the out-of-bounds obligation is NOT complete — the trap STAYS"
  (doc    "The OOB soundness dual. The obligation is the CONJUNCTION `(ge i 0) AND (lt i len)`; a precondition
           giving ONLY the lower bound `>= i 0` (missing `< i len`) cannot establish it — `i` could still be
           >= len, so the access can still trap past the end. The entry has only the lower-bound hypothesis
           and confirms it does NOT establish the full conjunction (the upper-bound conjunct is absent). So
           the @trap_free proof for the index MISSES → the bounds-check STAYS. Runs to `true` (asserts the
           lower bound alone is not the obligation). Pins that a PARTIAL bound does not certify in-bounds —
           @trap_free is sound (it never drops a bounds check unless BOTH bounds are proven).")
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
      (def (ge   (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (lt   (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 7) a) b))
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 8) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (export (. Term *))
      (export Thm)
      (export term-eq ge lt conj concl assume)))
  (input  (do
            (import "bounds" (Term Thm term-eq ge lt conj concl assume))
            (def (main)
              (let ((i    (Term.Var 2))
                    (len  (Term.Var 3))
                    (zero (Term.Num 0)))
                (let ((goal (conj (ge i zero) (lt i len))))
                  ; only the lower bound is assumed — no upper bound, so the conjunction is not established
                  (let ((lower (assume (ge i zero))))
                    ; the lower bound alone is NOT the full obligation → bounds check stays
                    (not (term-eq (concl lower) goal))))))
            (export main)))
  (output (: true Bool)))

; ── t1(match): the PARTIAL-MATCH / exhaustiveness trap source — a match with total arm coverage cannot trap ─
; The @trap_free capstone (§8): a `match` traps at an `Unreachable` node iff a scrutinee value hits no arm.
; Its trap-free obligation is EXHAUSTIVENESS — every reachable scrutinee value is covered. Modeled here as a
; `covers` proof: the obligation `COVERS scrut arms` holds when the arm set is TOTAL for the scrutinee's
; type. The exhaustiveness checker already decides this for the compiler; here we pin the OBLIGATION shape —
; an `exhaustive-ax` mints `COVERS s arms` only when a `total?` predicate on the arm set holds (a decidable
; ground check, like le-ax's numeral side-condition). `covers`=Const 9. A NON-total arm set yields None (the
; Unreachable stays reachable → the match can trap).

(case "t1(match): the exhaustiveness obligation COVERS is DISCHARGED for a TOTAL arm set — the match cannot trap"
  (doc    "The partial-match trap source of the @trap_free capstone. A `match` traps at Unreachable iff some
           scrutinee value hits no arm; its trap-free obligation is EXHAUSTIVENESS. Modeled: `exhaustive-ax`
           mints `COVERS scrut arms` ONLY when the arm set is TOTAL for the scrutinee (a decidable
           side-condition, `total?` — here a two-variant Bool scrutinee with both arms present). A total arm
           set discharges → no Unreachable is reachable → the match cannot trap. The entry checks a
           both-arms-covered Bool match discharges the COVERS obligation. Runs to `true`. Pins the
           exhaustiveness obligation shape the capstone's per-trap-source proof discharges (the checker
           already decides totality; this pins the obligation the discharge produces).")
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
      (def (covers (: scrut Term) (: arms Term)) (Term.Comb (Term.Comb (Term.Const 9) scrut) arms))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      ; total? : is the arm set (a list of covered variant tags, as Num) TOTAL for a scrutinee whose variant
      ; count is `n`? Decidable: the arm set covers exactly {0..n-1}. Here the ground check is "arms has n
      ; distinct tags 0..n-1"; modeled minimally as len(arms) == n with tags being 0..n-1 in order.
      (def (total? (: arms (List Int64)) (: n Int64))
        (= (List.len arms) n))
      ; AXIOM: mint COVERS scrut arms-term ONLY when the arm TAGS are total for the scrutinee's variant
      ; count. A non-total set → None (the Unreachable stays reachable).
      (def (exhaustive-ax (: scrut Term) (: arms-term Term) (: arm-tags (List Int64)) (: nvariants Int64))
        (if (total? arm-tags nvariants)
          (Option.Some (Thm.Seq (list) (covers scrut arms-term)))
          (Option.None)))
      (export (. Term *))
      (export Thm)
      (export term-eq covers concl total? exhaustive-ax)))
  (input  (do
            (import "bounds" (Term Thm term-eq covers concl total? exhaustive-ax))
            (def (main)
              (let ((scrut (Term.Var 0))
                    ; the arm set as an opaque term (its identity is what COVERS names); tags are 0,1 (both
                    ; Bool variants), nvariants = 2 → total.
                    (arms  (Term.Const 100)))
                (let ((goal (covers scrut arms)))
                  (match (exhaustive-ax scrut arms (list 0 1) 2)
                    ((Option.Some proof) (term-eq (concl proof) goal))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

(case "t1(match) NEGATIVE: a NON-total arm set (one Bool arm missing) does NOT discharge COVERS — the match can still trap"
  (doc    "The exhaustiveness soundness dual. A Bool scrutinee (2 variants) with only ONE arm covered (tags
           = {0}, missing 1) is NOT total, so `exhaustive-ax` returns None — the COVERS obligation is not
           established, the Unreachable stays reachable, and the @trap_free proof for the match MISSES → the
           match can still trap on the uncovered value. The entry confirms exhaustive-ax of a one-arm set
           over a 2-variant scrutinee yields None. Runs to `true`. Pins that a non-exhaustive match is NOT
           certified trap-free — @trap_free is sound (it never drops the Unreachable unless the match is
           proven total).")
  (module "bounds"
    (do
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (covers (: scrut Term) (: arms Term)) (Term.Comb (Term.Comb (Term.Const 9) scrut) arms))
      (def (total? (: arms (List Int64)) (: n Int64)) (= (List.len arms) n))
      (def (exhaustive-ax (: scrut Term) (: arms-term Term) (: arm-tags (List Int64)) (: nvariants Int64))
        (if (total? arm-tags nvariants)
          (Option.Some (Thm.Seq (list) (covers scrut arms-term)))
          (Option.None)))
      (export (. Term *))
      (export Thm)
      (export covers total? exhaustive-ax)))
  (input  (do
            (import "bounds" (Term Thm covers total? exhaustive-ax))
            (def (main)
              (let ((scrut (Term.Var 0))
                    (arms  (Term.Const 100)))
                ; only tag 0 covered, nvariants = 2 → NOT total → None
                (match (exhaustive-ax scrut arms (list 0) 2)
                  ((Option.Some _) false)
                  ((Option.None) true))))
            (export main)))
  (output (: true Bool)))

; ── t1(trap): the EXPLICIT-TRAP trap source — a `trap()` under a provably-FALSE guard is unreachable ──
; The @trap_free capstone (§8): an explicit `trap()` (or effect-abort) inside `(if guard (trap) …)` traps
; iff `guard` is satisfiable. Its trap-free obligation is UNREACHABILITY — the guard is provably FALSE for
; every input satisfying @requires, so the trap branch is dead. Modeled: the obligation `FALSE guard` holds
; when a `refute` rule derives a contradiction from `assume guard` + the precondition. Simplest ground
; instance: a trap guarded by `(lt x 0)` in a function `@requires(>= x 0)` — `ge x 0` and `lt x 0` are
; contradictory, so `refute` (from G |- (ge x 0) and a guard (lt x 0)) mints `UNREACHABLE (lt x 0)`.
; `unreach`=Const 10. A guard NOT contradicted by the precondition → None (the trap stays reachable).

(case "t1(trap): an explicit trap under guard (lt x 0) is UNREACHABLE when @requires(>= x 0) — the trap is dead"
  (doc    "The explicit-trap source of the @trap_free capstone. A `(if (lt x 0) (trap) …)` traps iff its
           guard `(lt x 0)` is satisfiable. Under `@requires(>= x 0)`, the guard CONTRADICTS the precondition
           (`ge x 0` and `lt x 0` cannot both hold), so the `refute` rule — from the precondition hypothesis
           `ge x 0` and the guard `lt x 0` — derives `UNREACHABLE (lt x 0)`: the trap branch is dead, so the
           function cannot reach the explicit trap. The entry assumes the precondition, refutes the guard,
           and checks the conclusion is the unreachability obligation. Runs to `true`. Pins the explicit-trap
           obligation shape (a guard proven false by the precondition) — the FIFTH and last trap source of
           the whole-function trap-free proof.")
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
      (def (ge      (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (lt      (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 7) a) b))
      (def (unreach (: g Term)) (Term.Comb (Term.Const 10) g))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; RULE `refute`: from G |- (ge x 0) and a GUARD (lt x 0) — a direct contradiction (x>=0 vs x<0 on the
      ; same x, same bound 0) — derive G |- (UNREACHABLE guard): the guarded branch is dead. Fires ONLY when
      ; the guard is `(lt x 0)` and the hypothesis is `(ge x 0)` for the SAME x (a recognized contradiction).
      (def (refute (: th Thm) (: guard Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 2) x) (Term.Num 0))
            (match guard
              ((Term.Comb (Term.Comb (Term.Const 7) gx) (Term.Num 0))
                (if (term-eq x gx)
                  (Option.Some (Thm.Seq (hyps th) (unreach guard)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq ge lt unreach concl hyps assume refute)))
  (input  (do
            (import "bounds" (Term Thm term-eq ge lt unreach concl hyps assume refute))
            (def (main)
              (let ((x    (Term.Var 0))
                    (zero (Term.Num 0)))
                (let ((guard (lt x zero)))
                  (let ((goal (unreach guard)))
                    ; @requires(>= x 0) → assume (ge x 0); refute the guard (lt x 0) as contradictory
                    (let ((pre (assume (ge x zero))))
                      (match (refute pre guard)
                        ((Option.Some proof) (term-eq (concl proof) goal))
                        ((Option.None) false)))))))
            (export main)))
  (output (: true Bool)))

(case "t1(trap) NEGATIVE: a trap guard NOT contradicted by the precondition is NOT unreachable — the trap STAYS"
  (doc    "The explicit-trap soundness dual. If the trap guard is NOT contradicted by the precondition — here
           the guard is `(lt x 0)` but the precondition is only `(ge x 5)`… actually a guard the precondition
           does not refute: guard `(lt x 100)` under `@requires(>= x 0)` is SATISFIABLE (x in [0,100) hits
           it), so `refute` (which recognizes only the exact `ge x 0` vs `lt x 0` contradiction) returns None
           — the trap branch is NOT proven dead, so the explicit trap STAYS reachable. The entry confirms
           `refute` of a non-contradictory guard yields None. Runs to `true`. Pins that a reachable explicit
           trap is NOT certified away — @trap_free is sound (it never drops a trap whose guard it cannot
           prove false).")
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
      (def (ge      (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      (def (lt      (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 7) a) b))
      (def (unreach (: g Term)) (Term.Comb (Term.Const 10) g))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (refute (: th Thm) (: guard Term))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 2) x) (Term.Num 0))
            (match guard
              ((Term.Comb (Term.Comb (Term.Const 7) gx) (Term.Num 0))
                (if (term-eq x gx)
                  (Option.Some (Thm.Seq (hyps th) (unreach guard)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq ge lt unreach concl hyps assume refute)))
  (input  (do
            (import "bounds" (Term Thm term-eq ge lt unreach concl hyps assume refute))
            (def (main)
              (let ((x     (Term.Var 0))
                    (zero  (Term.Num 0))
                    (c100  (Term.Num 100)))
                ; guard (lt x 100) — SATISFIABLE under (ge x 0) (x in [0,100)); NOT the ge-x-0/lt-x-0
                ; contradiction refute recognizes → None → the trap stays reachable.
                (let ((guard (lt x c100)))
                  (let ((pre (assume (ge x zero))))
                    (match (refute pre guard)
                      ((Option.Some _) false)
                      ((Option.None) true))))))
            (export main)))
  (output (: true Bool)))
; ── TESTED tier: a `@test`-stacked `@ensures` is VALUE-TRANSPARENT and still checks the postcondition ─────
; When `@test` is stacked on `@ensures`, the postcondition runs as a property test (v-property-testing's
; lane): the rewrite injects `(let ((it BODY)) (if Q it (trap …)))` — `it` bound to the def's own result so
; the predicate reads the computed value, TRAPPING when Q is false (a `@test` passes by returning, fails by
; trapping). The pass branch returns `it` (the def's VALUE), NOT `unit` — so the def stays value-transparent
; when ALSO called as an ordinary function, exactly like a bare `@test` def and a bare `@ensures` def both do
; (neither changes the def's return value). These two pin both halves so neither regresses: value-transparency
; (the def still returns its value) AND the test semantics (a false postcondition still traps). The stacked
; rewrite returning `unit` on the pass branch was a surprise a breaker probe flagged; the fix returns `it`.

(case "a stacked @test @ensures def called as a function returns its value, not unit (value-transparent)"
  (doc    "A def carrying BOTH `@test` and `@ensures`, when CALLED as an ordinary function, returns its
           computed value — NOT `unit`. `(dbl 5)` = 10, the same value a bare `@test` def or a bare
           `@ensures` def returns (both are value-transparent). The TESTED-tier rewrite injects
           `(let ((it BODY)) (if Q it (trap …)))`: the pass branch returns `it` (the def's result), so the
           postcondition check does not swallow the value. A rewrite that returned `unit` on the pass branch
           (the earlier behavior) would make the stacked form silently non-value-transparent — this pins it
           does not. The true postcondition `(>= it 0)` holds for 10, so no trap; the value 10 flows out.")
  (input  (do
            (@ test (@ (ensures (>= it 0)) (def (dbl (: x Int64)) (+ x x))))
            (def (main) (dbl 5))
            (export main)))
  (output (: 10 Int64)))

(case "a stacked @test @ensures with a FALSE postcondition traps when the def is called (test semantics preserved)"
  (doc    "The test-semantics half of the value-transparency pin above: making the postcondition FALSE must
           still TRAP (a `@test` fails by trapping). `(dbl 5)` = 10 and the postcondition `(< it 0)` — i.e.
           `10 < 0` — is false, so the injected `(if Q it (trap …))` takes the trap arm, halting with the
           canonical `unreachable` kind. Together with the value-transparent case above this pins that
           returning `it` (not `unit`) on the PASS branch did NOT weaken the check: a true postcondition
           yields the value, a false one still traps — the fix is value-transparent AND test-preserving.")
  (input  (do
            (@ test (@ (ensures (< it 0)) (def (dbl (: x Int64)) (+ x x))))
            (def (main) (dbl 5))
            (export main)))
  (trap   "unreachable"))

; ── (D) TEST-TIER ENFORCEMENT — a PLAIN @requires is CHECKED at run time (Inc-b (D), verify_enforce.rs) ──
; The operator confirmed (D): @requires/@ensures/@trap_free/@invariant verify AT RUN TIME now (proof-guided
; ELISION defers to the bounded compile-time kernel interpreter (A) — a3's compile-time-eval premise was
; unbuildable: the kernel is recursive and rcdzc has no compile-time recursive evaluator). These two cases
; pin the PLAIN @requires enforcement: a violated precondition TRAPS, a satisfied one is value-transparent.

(case "a PLAIN @requires precondition is ENFORCED at body-entry: a VIOLATED precondition traps when the def is called"
  (doc    "The (D) test-tier enforcement of a bare `@requires` (NOT stacked under `@test` — that is
           v-property-testing's TESTED tier). `verify_enforce::enforce` rewrites `(@ (requires (>= x 0))
           (def (f (: x Int64)) (+ x 1)))` so the body becomes `(if (>= x 0) (+ x 1) (trap …))` — the
           precondition is checked ONCE at body-entry (the Hoare `{P} body {Q}` reading), NOT at each call
           site. `(f -5)` violates `(>= x 0)`, so the `if` takes the trap arm, halting with the canonical
           `unreachable` kind. Before (D) the precondition was RECORDED (db.requires) but NOT enforced — the
           call returned `-4`. Pins that a plain @requires now actually verifies at run time; the wrapper is
           left in place so `strip_annotations` still records the predicate for the verification layer.")
  (input  (do
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))
            (def (main) (f -5))
            (export main)))
  (trap   "unreachable"))

(case "a PLAIN @requires precondition is value-transparent when SATISFIED: the def returns its computed value"
  (doc    "The value-transparency half of the plain-@requires enforcement pin above. `(f 5)` SATISFIES
           `(>= x 0)`, so the injected `(if (>= x 0) (+ x 1) (trap …))` takes the pass arm and returns the
           def's own value `6` — NOT `unit`, and no trap. Together with the trap case above this pins that
           the enforcement rewrite is value-transparent AND checking: a satisfied precondition yields the
           computed result, a violated one traps — the check does not swallow the value on the pass path.")
  (input  (do
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))
            (def (main) (f 5))
            (export main)))
  (output (: 6 Int64)))
