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

; ── b4c(proven): a full @requires/@ensures obligation — denote both, discharge P ⇒ Q[ret:=body] ─────────
; b4b denotes ONE predicate Ast → Term. b4c(proven) composes the elaboration (§2.1): for
;   @requires(<= x 100) @ensures(<= ret MAXINT) (def (f x) (+ x 1))
; the obligation is `denote(P) ⊢ denote(Q)[ret := denote(body)]` — i.e. from the precondition hypothesis
; `le x 100` derive `le (add x 1) MAXINT` (the postcondition with `it` the body's value `x+1`). This is
; exactly the b1 discharge chain, now framed as the DENOTED annotations: `it` in Q is replaced by the
; denotation of the body `(+ x 1)` → `add (Var 0) (Num 1)`, and the precondition enters via `assume`. Pins
; that the §2.1 elaboration target — the whole @requires⇒@ensures obligation — discharges through the SAME
; kernel the hand-authored b1 cases use, so the b4c compiler wiring (compile-time-eval) has a proven target.

(case "b4c(proven): @requires(<= x 100)/@ensures(<= ret MAXINT) on (f x)=x+1 discharges — P denoted as hyp, Q[ret:=body] as goal"
  (doc    "The PROVEN-tier obligation for a full @requires/@ensures pair (design §2.1). The elaboration
           denotes @requires(<= x 100) → the hypothesis `le (Var 0) (Num 100)` (via assume) and
           @ensures(<= ret MAXINT) with `ret` := the body's denotation `add (Var 0) (Num 1)` → the goal
           `le (add (Var 0) 1) MAXINT`. Discharging is the b1 chain: mono-add-r on the assumed precondition
           + a CHECKED numeral fact (101 <= MAXINT) + trans-le. The entry builds the denoted obligation and
           discharges it through the kernel, checking the conclusion is the denoted postcondition. Runs to
           `true`. Pins that the b4 elaboration's whole-obligation target (P ⇒ Q[ret:=body]) discharges via
           the SAME kernel machinery b1 exercises — so b4c's compile-time-eval wiring has a proven shape to
           produce, and the discharged Thm is exactly what b3's oracle consumes for the implicit overflow
           obligation (here `<= ret MAXINT` IS the no-overflow condition on `x+1`).")
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
                ; denote(body) = (+ x 1) → add (Var 0) (Num 1); ret := this in the @ensures goal
                (let ((body-den (add x one)))
                  ; @ensures(<= ret MAXINT) with ret:=body → goal = le (add x 1) MAXINT
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
; The dual of b4c(proven). For `@ensures(<= ret MAXINT) (def (f x) (+ x 1))` with NO (or too-weak)
; @requires, the postcondition obligation `le (add x 1) MAXINT` is NOT provable — x is unbounded, so the
; discharge chain has no bounding hypothesis to feed mono-add-r, and le-ax cannot mint a non-ground fact.
; At b4c this un-discharged obligation is the PROVEN-tier MISS: the author gets CDZ-VERIFY (or, if @test is
; stacked, the TESTED tier runs it — v-property-testing's lane). This pins that a genuinely-unprovable
; postcondition does NOT spuriously discharge — the proof tier is SOUND (it never claims a false proof).

(case "b4c(unprovable): @ensures(<= ret MAXINT) on unbounded (f x)=x+1 is NOT dischargeable — the proof tier correctly MISSES (CDZ-VERIFY)"
  (doc    "The PROVEN-tier soundness dual. With no bounding @requires, the @ensures postcondition
           `<= ret MAXINT` (ret := body `x+1`) denotes to the obligation `le (add x 1) MAXINT`, which is NOT
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
            (@ test (@ (ensures (>= ret 0)) (def (dbl (: x Int64)) (+ x x))))
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
            (@ test (@ (ensures (< ret 0)) (def (dbl (: x Int64)) (+ x x))))
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

(case "a PLAIN @requires relating TWO parameters (< a b) is enforced — BOTH params stay in scope in the predicate"
  (doc    "Every runtime @requires case so far constrains a SINGLE parameter (`>= x 0`, `<= x 100`). This pins a
           precondition relating TWO distinct parameters — the ordering contract `(< a b)` on a two-arg def —
           so the injected `(if (< a b) BODY (trap …))` must keep BOTH `a` AND `b` in scope at body-entry (the
           predicate reads both, exactly as a hand-written guard would). `(f 3 5)` satisfies `(< 3 5)`, so the
           `if` takes the pass arm and the def returns its own value `(- b a)` = `2`. Pins that a multi-parameter
           precondition resolves + enforces (the entry-side twin of the result-vs-parameter @ensures case, which
           reads `ret` alongside a param).")
  (input  (do
            (@ (requires (< a b)) (def (f (: a Int64) (: b Int64)) (- b a)))
            (def (main) (f 3 5))
            (export main)))
  (output (: 2 Int64)))

(case "a PLAIN @requires relating TWO parameters (< a b) TRAPS when violated — the two-param precondition is checked"
  (doc    "The trap half of the two-parameter precondition above. `@requires(< a b)` on `(f a b) = (- b a)`
           with `(f 5 3)` violates the ordering (`5 < 3` is FALSE), so the injected `(if (< a b) (- b a)
           (trap …))` takes the trap arm — `unreachable` — even though the body `(- b a)` = `-2` would itself
           compute fine. Pins that a precondition over two parameters enforces in both directions, not only the
           satisfied one, and that the check fires on the RELATIONSHIP between the args, not a single arg's
           range.")
  (input  (do
            (@ (requires (< a b)) (def (f (: a Int64) (: b Int64)) (- b a)))
            (def (main) (f 5 3))
            (export main)))
  (trap   "unreachable"))

(case "STACKED @requires: EVERY precondition is enforced — a violated OUTER @requires traps (not only the innermost)"
  (doc    "Soundness pin for stacked preconditions. A def may carry several `@requires`, which desugar to
           NESTED annotation wrappers: `(@ (requires (>= x 0)) (@ (requires (<= x 100)) (def (f x) (+ x 1))))`.
           `verify_enforce::enforce` must descend through the intervening `(@ …)` layer to reach the def and
           re-wrap its CURRENT body at EACH `@requires`, so the checks nest — `(if (>= x 0) (if (<= x 100)
           (+ x 1) trap) trap)` — and ALL preconditions verify. Before the fix, the scan only rewrote a
           `@requires` whose INNER was directly a `(def …)`, so the OUTER `(requires (>= x 0))` (whose inner
           is another `(@ …)` wrapper) was SILENTLY SKIPPED — only the innermost `(<= x 100)` enforced. Then
           `(f -5)` — which VIOLATES the outer `(>= x 0)` but SATISFIES the inner `(<= x 100)` — wrongly
           returned `-4` instead of trapping. This case calls `(f -5)`: the outer precondition is now checked,
           its `if` takes the trap arm, halting with the canonical `unreachable` kind. Pins that stacking does
           not drop the outer preconditions — the (D) guarantee (a violated precondition crashes) holds for
           EVERY stated precondition, not just the last.")
  (input  (do
            (@ (requires (>= x 0))
            (@ (requires (<= x 100))
               (def (f (: x Int64)) (+ x 1))))
            (def (main) (f -5))
            (export main)))
  (trap   "unreachable"))

(case "STACKED @requires: value-transparent when ALL preconditions are satisfied"
  (doc    "The value-transparency half of the stacked-@requires pin above. With both `(>= x 0)` and
           `(<= x 100)` stacked on `(f x) = x + 1`, `(f 50)` satisfies BOTH, so the nested checks
           `(if (>= x 0) (if (<= x 100) (+ x 1) trap) trap)` both take the pass arm and the def returns its
           own value `51` — no trap, no swallowed value. Together with the trap case above this pins that the
           multi-precondition enforcement composes correctly: every precondition is checked, and a run that
           satisfies all of them yields the computed result unchanged.")
  (input  (do
            (@ (requires (>= x 0))
            (@ (requires (<= x 100))
               (def (f (: x Int64)) (+ x 1))))
            (def (main) (f 50))
            (export main)))
  (output (: 51 Int64)))

(case "@requires stacked OVER @ensures: the precondition is still enforced when an @ensures wrapper sits between it and the def"
  (doc    "The reviewer's post-merge vector on the (D) @requires enforcement (a natural spelling of the
           canonical precondition+postcondition contract). `(@ (requires (>= x 0)) (@ (ensures (>= ret 0))
           (def (f x) (+ x 1))))` — the `@requires` does NOT directly wrap the def; the `@ensures` layer is
           between them. Before the descent fix, `verify_enforce::enforce` only rewrote a `@requires` whose
           INNER was directly a `(def …)`, so this `@requires` (inner = the `(@ (ensures …) …)` node) was
           SILENTLY SKIPPED — `(f -5)` returned `-4` instead of trapping, a precondition that looked enforced
           but was not. The fix DESCENDS through any intervening `(@ NAME INNER)` layer (here the `@ensures`)
           to the def and injects `(if (>= x 0) (+ x 1) (trap …))`, so the precondition enforces regardless of
           which verification/annotation layers wrap between it and the def. `(f -5)` violates `(>= x 0)`, so
           the check takes the trap arm — `unreachable`. Pins that @requires enforcement is ORDER-INSENSITIVE
           with respect to a stacked @ensures (or any other annotation), closing the reviewer-verified leak.
           (@ensures itself is not yet run-time-enforced here — that is the immediately-following increment;
           this case is purely about the @requires precondition firing through the @ensures wrapper.)")
  (input  (do
            (@ (requires (>= x 0))
            (@ (ensures (>= ret 0))
               (def (f (: x Int64)) (+ x 1))))
            (def (main) (f -5))
            (export main)))
  (trap   "unreachable"))

(case "a PLAIN @ensures postcondition is ENFORCED at body-exit: a VIOLATED postcondition traps when the def is called"
  (doc    "The (D) test-tier enforcement of a BARE `@ensures` (NOT stacked under `@test` — that is
           v-property-testing's TESTED tier, which they own). `verify_enforce::enforce` rewrites
           `(@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))` so the body becomes
           `(let ((it (- x 100))) (if (>= it 0) it (trap …)))` — the postcondition is checked at body-EXIT
           (the Hoare `{P} body {Q}` reading, `it` bound to the def's RESULT), and is VALUE-TRANSPARENT: the
           pass arm returns `it`, the def's own value, NOT `unit`. `(f 5)` computes `-95`, which violates
           `(>= ret 0)`, so the `if` takes the trap arm — `unreachable`. Before this increment a bare @ensures
           was RECORDED (db.ensures) but NOT enforced — `(f 5)` returned `-95`. Pins that a plain @ensures now
           actually verifies at run time. (A `@test @ensures` stack is v-property-testing's; this pass skips
           that shape to avoid double-injection — bare @ensures is v-verification's.)")
  (input  (do
            (@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))
            (def (main) (f 5))
            (export main)))
  (trap   "unreachable"))

(case "a PLAIN @ensures postcondition is value-transparent when SATISFIED: the def returns its computed value"
  (doc    "The value-transparency half of the plain-@ensures enforcement pin above. `(f 200)` computes `100`,
           which SATISFIES `(>= it 0)`, so the injected `(let ((it (- x 100))) (if (>= it 0) it (trap …)))`
           binds `ret = 100`, the `if` takes the pass arm, and the def returns `ret` = `100` — its OWN value,
           not `unit`, and no trap. Together with the trap case above this pins that the @ensures enforcement
           rewrite is value-transparent AND checking: a satisfied postcondition yields the computed result, a
           violated one traps.")
  (input  (do
            (@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))
            (def (main) (f 200))
            (export main)))
  (output (: 100 Int64)))

(case "a PLAIN @ensures relating ret to a PARAMETER (> ret x) is enforced — the param stays in scope alongside ret in the predicate"
  (doc    "The most common real-world postcondition shape: the result related to an INPUT, not just a
           constant. Every other runtime @ensures case pins `ret` against a literal (`>= ret 0`,
           `<= ret MAXINT`); this pins `@ensures(> ret x)` — \"the result exceeds the input\". The
           injected `(let ((ret (+ x 1))) (if (> ret x) ret (trap …)))` must keep the def's PARAM `x`
           in scope INSIDE the predicate ALONGSIDE the synthesized `ret` binder — the predicate reads
           BOTH. `(f 5)` computes `6`, and `6 > 5` holds, so the `if` takes the pass arm and the def
           returns `ret` = `6`. Pins that a multi-name postcondition (result-vs-parameter) resolves and
           enforces exactly like a hand-written `(if (> (+ x 1) x) …)`.")
  (input  (do
            (@ (ensures (> ret x)) (def (f (: x Int64)) (+ x 1)))
            (def (main) (f 5))
            (export main)))
  (output (: 6 Int64)))

(case "a PLAIN @ensures relating ret to a PARAMETER (> ret x) TRAPS when violated — the result-vs-input postcondition is checked"
  (doc    "The trap half of the result-vs-parameter postcondition above. `@ensures(> ret x)` on
           `(g x) = x - 1` — the result must exceed the input, but `x - 1 < x` always, so the
           postcondition is violated for every argument. The injected
           `(let ((ret (- x 1))) (if (> ret x) ret (trap …)))` binds `ret = 4` for `(g 5)`, and
           `4 > 5` is FALSE, so the `if` takes the trap arm — `unreachable`. Together with the case
           above this pins that a postcondition reading BOTH `ret` and a param enforces in both
           directions, not only the satisfied one.")
  (input  (do
            (@ (ensures (> ret x)) (def (g (: x Int64)) (- x 1)))
            (def (main) (g 5))
            (export main)))
  (trap   "unreachable"))

(case "a PLAIN @ensures over a HEAP result (List) is enforced — ret binds a heap value, value-transparent when satisfied"
  (doc    "The runtime @ensures cases so far all return a SCALAR (Int64); this pins @ensures over a def
           that returns a HEAP value. The injected `(let ((ret BODY)) (if Q ret (trap …)))` binds `ret`
           to a LIST, the predicate reads it via `(List.len ret)`, and the pass arm returns that same
           heap value — value-transparency must hold for a heap return, not only a scalar. `(f 7)` builds
           `(List.push (list) 7)` (a 1-element list), `(> (List.len ret) 0)` holds, so the def returns the
           list and `main` reads its length `1`. Pins that the @ensures rewrite binds + returns a heap
           `ret` correctly (no ownership/drop hazard from the extra let-binding of a heap value).")
  (input  (do
            (@ (ensures (> (List.len ret) 0)) (def (f (: x Int64)) (List.push (list) x)))
            (def (main) (List.len (f 7)))
            (export main)))
  (output (: 1 Int64)))

(case "a PLAIN @ensures over a HEAP result (List) TRAPS when violated — the postcondition checks the heap value"
  (doc    "The trap half of the heap-result postcondition above. `@ensures(> (List.len ret) 0)` on
           `(g x) = (list)` — the result must be non-empty, but the body returns the EMPTY list, so the
           postcondition is violated. The injected `(let ((ret (list))) (if (> (List.len ret) 0) ret
           (trap …)))` binds `ret` to the empty list, `(List.len ret) = 0`, `(> 0 0)` is FALSE, so the
           `if` takes the trap arm — `unreachable`. Together with the case above this pins that an
           @ensures over a heap return enforces in both directions.")
  (input  (do
            (@ (ensures (> (List.len ret) 0)) (def (g (: x Int64)) (list)))
            (def (main) (List.len (g 7)))
            (export main)))
  (trap   "unreachable"))

(case "a PLAIN @ensures whose predicate reads ONLY a parameter (not ret) is enforced — the dual of the nullary case"
  (doc    "Every runtime @ensures case reads the result binder `ret`; this pins the DUAL — a postcondition that
           references ONLY a PARAMETER and ignores `ret`. `@ensures(> x 0)` on `(f x) = (- x 1)`: the injected
           `(let ((ret (- x 1))) (if (> x 0) ret (trap …)))` binds `ret` (unused by the predicate) and checks
           `(> x 0)` over the param `x` — a postcondition constraining the INPUT at exit, a legitimate (if
           unusual) contract. `(f 5)`: `x = 5 > 0` holds, so the check takes the pass arm and returns `ret` =
           `4` — its own value. Pins that the enforcement wrap injects + returns `ret` correctly even when the
           predicate never mentions it (the binder is still introduced, the body value still flows through, the
           predicate resolves against the param in scope). Complements the nullary case (predicate reads only
           `ret`, no param): together they pin both extremes of what an @ensures predicate may reference.")
  (input  (do
            (@ (ensures (> x 0)) (def (f (: x Int64)) (- x 1)))
            (def (main) (f 5))
            (export main)))
  (output (: 4 Int64)))

(case "a PLAIN @ensures with a constant-FALSE predicate always traps — the postcondition fires unconditionally"
  (doc    "The degenerate soundness pin: an `@ensures false` (a predicate that is the literal `false`,
           independent of `ret` or any param) must ALWAYS trap when the def runs — the postcondition can never
           be satisfied. The injected `(let ((ret x)) (if false ret (trap …)))` binds `ret` then takes the trap
           arm unconditionally — `unreachable`. `(f 5)` traps despite the body `x` = `5` computing fine. Pins
           that the enforcement wrap does NOT const-fold away a statically-false postcondition into a silent
           pass (a `(if false …)` that dropped the trap arm would let a provably-false contract compile to a
           returning function) — the check is faithful even when the predicate is a compile-time constant.")
  (input  (do
            (@ (ensures false) (def (f (: x Int64)) x))
            (def (main) (f 5))
            (export main)))
  (trap   "unreachable"))

(case "TWO stacked @ensures COMPOSE: BOTH postconditions are enforced — value-transparent when both hold"
  (doc    "The `@ensures`-composition pin (analogue of the stacked-`@requires` cases above, for the exit side).
           A def may carry more than one `@ensures` — `(@ (ensures Q1) (@ (ensures Q2) (def …)))` — spelling two
           independent postconditions. `verify_enforce::enforce` processes each annotation at its OWN index and
           re-wraps the def's CURRENT body, so the inner `@ensures(Q2)` wraps first — body becomes
           `(let ((ret BODY)) (if Q2 ret (trap …)))` — and the outer `@ensures(Q1)` then wraps THAT — the def
           body becomes `(let ((ret (let ((ret BODY)) (if Q2 ret (trap))))) (if Q1 ret (trap)))`. The two `ret`
           binders NEST (each scopes its own predicate over the value flowing out of the layer below); both
           checks fire. `(f 5)` computes `6`; the inner `@ensures(< ret 1000)` holds (`6 < 1000`) so `ret = 6`
           flows out, then the outer `@ensures(>= ret 0)` holds (`6 >= 0`), so the def returns `6` — its own
           value, no trap. Pins that stacked postconditions COMPOSE and stay value-transparent when both hold
           (the exit-side twin of the stacked-@requires value-transparent case).")
  (input  (do
            (@ (ensures (>= ret 0))
            (@ (ensures (< ret 1000))
               (def (f (: x Int64)) (+ x 1))))
            (def (main) (f 5))
            (export main)))
  (output (: 6 Int64)))

(case "TWO stacked @ensures: a violated INNER postcondition traps even when the OUTER holds"
  (doc    "The trap half of the stacked-@ensures composition above — and the discriminating case: it fails only
           the INNER postcondition, so a naive implementation that enforced only the outermost `@ensures` (or
           only the innermost) would let it slip. `(@ (ensures (>= ret 0)) (@ (ensures (< ret 1000)) (def (f x)
           (+ x 2000))))` on `(f 5)` computes `2005`. The INNER `@ensures(< ret 1000)` is checked first on the
           raw body value (`2005 < 1000` is FALSE) → its `if` takes the trap arm — `unreachable` — BEFORE the
           outer `@ensures(>= ret 0)` (which WOULD hold, `2005 >= 0`) ever runs. Pins that EVERY stacked
           postcondition is enforced, not just one: an inner violation traps regardless of the outer verdict
           (the exit-side twin of the stacked-@requires \"violated OUTER traps\" case).")
  (input  (do
            (@ (ensures (>= ret 0))
            (@ (ensures (< ret 1000))
               (def (f (: x Int64)) (+ x 2000))))
            (def (main) (f 5))
            (export main)))
  (trap   "unreachable"))

(case "@ensures on a def with a parameter named ret is REJECTED (would silently not enforce — rename the param)"
  (doc    "The result-binder-capture guard, as a REJECT (breaker 2026-07-17). `@ensures(Q)` enforcement binds
           the def's RESULT to `ret` (`(let ((ret BODY)) (if Q ret (trap)))`). If a PARAMETER is
           literally named `ret`, that binder would SHADOW the param, so `verify_enforce` cannot enforce the
           postcondition for such a def. Rather than SILENTLY skip it (a footgun — the author wrote a contract
           that is quietly unenforced; a violating result would return with no trap or diagnostic),
           `collect_faults` REJECTS it: a stated contract is enforced OR the author is told precisely why not
           (the (D) philosophy). Here `(def (f (: ret Int64)) (- ret 100))` carries `@ensures(>=
           ret 0)` and has a param named `ret` → CDZ0201 at the annotation, naming the fix (rename the
           param). Pins that the guard is a diagnostic, not a silent drop. (An `@requires` on the same def would
           be fine — only `@ensures` binds `ret`. The result binder was renamed `it`→`ret` per the
           operator's collision-safety directive; a user naming a param `ret` is now vanishingly unlikely,
           but the guard stays for soundness.)")
  (input  (do
            (@ (ensures (>= ret 0)) (def (f (: ret Int64)) (- ret 100)))
            (def (main) (f 5))
            (export main)))
  (error  CDZ0201))

(case "a @requires predicate that references `ret` is REJECTED CDZ0101 — only @ensures binds the result"
  (doc    "The scope-boundary pin between the two annotations: `ret` is the @ENSURES result binder, and NOTHING
           else introduces it. A `@requires` runs at body-ENTRY, before any result exists, so it binds only the
           def's PARAMETERS (and prelude/global names) — `ret` is NOT in scope. A `@requires(>= ret 0)` therefore
           references an UNBOUND name and is rejected CDZ0101 at the annotation. This guards the exact boundary:
           a regression that leaked the @ensures `ret` binder into `@requires` scope would silently ACCEPT a
           nonsensical precondition (a precondition over a not-yet-computed result), so pinning the reject keeps
           the two contracts' scopes distinct. (`collect_faults` skips the def's params + — for @ensures ONLY —
           the `ret` subject when checking predicate names; `@requires` passes no subject, so `ret` resolves to
           Poison(CDZ0101) exactly as any stray name would.)")
  (input  (do
            (@ (requires (>= ret 0)) (def (f (: x Int64)) x))
            (def (main) (f 5))
            (export main)))
  (error  CDZ0101))

(case "a PLAIN @ensures on a NULLARY def (no parameters) enforces — ret binds the body, predicate reads only ret"
  (doc    "Every runtime @ensures case so far has at least one parameter; this pins @ensures on a def with NO
           parameters, where the postcondition predicate reads ONLY the result binder `ret` (no param is in
           scope to reference). The injected `(let ((ret BODY)) (if Q ret (trap …)))` binds `ret` to the
           nullary body and checks the predicate over it alone. `(def (f) (- 5 10))` computes `ret = -5`, which
           violates `@ensures(>= ret 0)`, so the `if` takes the trap arm — `unreachable`. Pins that the
           enforcement rewrite needs no parameter to inject its check (the empty param list is not a special
           case that skips enforcement) and that a nullary def's postcondition is checked over the result alone.")
  (input  (do
            (@ (ensures (>= ret 0)) (def (f) (- 5 10)))
            (def (main) (f))
            (export main)))
  (trap   "unreachable"))

; ── @requires enforcement EDGES (breaker) — beyond the const-arg violated/satisfied pair above ──────
; The two (D) pins above call `f` with a CONSTANT argument, so a fold could in principle have discharged
; the check at compile time. These pin the enforcement's REACH: a genuinely-runtime argument (the check
; must be emitted, not folded), a RECURSIVE def (the body-entry check re-fires at every entry, including
; self-calls), a predicate that itself PERFORMS an effect (the pre runs under the caller's handler and
; ADVANCES its state before the body runs), and a predicate that itself TRAPS (its own trap kind wins —
; the requires rewrite adds no guard around the predicate's evaluation).

(case "a @requires precondition is enforced for a genuinely-runtime argument"
  (doc    "The runtime companion of the const-arg violation pin above: the argument arrives at the CALL
           BOUNDARY, so nothing folds and the injected body-entry `(if (>= x 0) … (trap …))` must actually
           run. `(f -5)` violates → the canonical unreachable trap; `(f 5)` satisfies → 6, value-transparent.
           A pass that only proved const violations (or an emit that dropped the check on the runtime path)
           would return -4 here — the exact pre-(D) behavior — so this is the regression pin for the
           EMITTED check.")
  (input  (do
            (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))
            (def (main (: n Int64)) (f n))
            (export main)))
  (call   main (: -5 Int64))
  (trap   "unreachable")
  (call   main (: 5 Int64))
  (output (: 6 Int64)))

(case "a @requires on a recursive def is re-checked at every entry including self-calls"
  (doc    "The body-entry reading ({P} body {Q}, checked when the function RUNS) puts the injected check
           inside the def, so a RECURSIVE def re-fires it on every self-call, not only the outermost entry.
           `fact` with `@requires (>= n 0)`: n=4 → 24 (every recursive entry 4,3,2,1,0 satisfies), n=-1 →
           the entry check traps immediately. Pins that the rewrite composes with recursion (specialization
           /accumulator transforms must keep the per-entry check) — a call-site-only reading would also
           pass n=4 but differs on shapes where an internal entry first violates.")
  (input  (do
            (@ (requires (>= n 0))
              (def (fact (: n Int64)) (if (= n 0) 1 (* n (fact (- n 1))))))
            (def (main (: k Int64)) (fact k))
            (export main)))
  (call   main (: 4 Int64))
  (output (: 24 Int64))
  (call   main (: -1 Int64))
  (trap   "unreachable"))

(case "an @ensures on a recursive def is re-checked at every EXIT including self-call returns (not only the outermost)"
  (doc    "The @ensures twin of the recursive-@requires case above — the exit-side per-entry pin, with a
           DISCRIMINATING shape. `@ensures` wraps the body as `(let ((ret BODY)) (if Q ret (trap …)))` INSIDE
           the def, so a recursive def re-checks the postcondition on EVERY exit, including each self-call
           return — not only the outermost. `f` with `@ensures (>= ret 0)`: `f 0 = 5` (ok, the control); `f 1
           = (- (f 0) 10) = -5` (VIOLATES); `f 2 = (+ (f 1) 10) = 5` — the OUTERMOST result 5 satisfies, but
           reaching it recurses through `f 1` whose exit value `-5` fails `(>= ret 0)`, so the per-exit check
           traps at that inner return BEFORE `f 2` ever returns. A postcondition read only at the outermost
           call would wrongly return 5; the per-exit check traps `unreachable`. Pins that the rewrite composes
           with recursion on the exit side (a tail/accumulator transform must keep the per-exit check).")
  (input  (do
            (@ (ensures (>= ret 0))
              (def (f (: n Int64))
                (if (<= n 0) 5
                  (if (= n 1) (- (f 0) 10) (+ (f (- n 1)) 10)))))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 5 Int64))
  (call   main (: 2 Int64))
  (trap   "unreachable"))

(case "an EFFECTFUL @requires predicate performs under the caller's handler and advances its state before the body"
  (doc    "The predicate `(> (Counter.bump) 0)` PERFORMS an operation, so the injected body-entry check is
           itself effectful: it must route to the dynamically-enclosing handler and its state advance must
           be SEEN by the body's own later perform — the check is sequenced BEFORE the body, in the same
           handler extent, not hoisted out of it or double-performed. Seeded 0: the pre's bump resumes 1
           (>0, satisfied — and threads state 1), the body's bump resumes 2, so `(f 10)` = 10 + 2 = 12. A
           rewrite that evaluated the predicate OUTSIDE the handler would fail to compile or trap; one that
           re-evaluated it would yield 13.")
  (input  (do
            (effect Counter (op bump (-> Unit Int64)))
            (@ (requires (> (Counter.bump) 0))
              (def (f (: n Int64)) (+ n (Counter.bump))))
            (def (main (: n Int64))
              (handle Counter 0
                ((bump (u) s (resume (+ s 1) (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 12 Int64)))

(case "an EFFECTFUL @ensures predicate performs under the caller's handler at body-EXIT, after the body's own perform"
  (doc    "The @ensures twin of the effectful-@requires case above — the exit-side handler-extent pin. The
           postcondition `(> (Counter.bump) 100)` PERFORMS, so the injected exit check `(let ((ret BODY)) (if
           (> (Counter.bump) 100) ret (trap …)))` is itself effectful: it must route to the dynamically
           enclosing handler and be sequenced AFTER the body (the body already performed to compute `ret`), in
           the same handler extent — not hoisted, not double-performed, not evaluated before the body. Handler
           seeded 0, each `bump` resumes `s+1` and threads `s+1`: the BODY's bump is the FIRST perform (resumes
           1, state→1), so `ret = 10 + 1 = 11`; the postcondition's bump is the SECOND (resumes 2, state→2).
           `(> 2 100)` is FALSE, so the @ensures check takes the trap arm — `unreachable`. Pins that an
           effectful postcondition performs in-handler at body-exit AND its verdict is enforced (a rewrite that
           evaluated it before the body, or outside the handler, would resume 1 / fail to compile).")
  (input  (do
            (effect Counter (op bump (-> Unit Int64)))
            (@ (ensures (> (Counter.bump) 100))
              (def (f (: n Int64)) (+ n (Counter.bump))))
            (def (main (: n Int64))
              (handle Counter 0
                ((bump (u) s (resume (+ s 1) (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 10 Int64))
  (trap   "unreachable"))

(case "an EFFECTFUL @ensures predicate is value-transparent when SATISFIED — the body's own perform runs first"
  (doc    "The satisfied control for the effectful-@ensures trap above, pinning the perform ORDER precisely.
           Same shape but with a threshold `(> (Counter.bump) 0)` the second bump satisfies. Handler seeded 0,
           resumes `s+1` threading `s+1`: the BODY's bump is FIRST (resumes 1) so `ret = 10 + 1 = 11`; the
           postcondition's bump is SECOND (resumes 2), and `(> 2 0)` HOLDS, so the check takes the pass arm and
           the def returns `ret` = `11` — its own value, no trap. The result being `11` (not `12`) is the load-
           bearing detail: it proves the body performed BEFORE the postcondition (body drew state 1), and that
           the postcondition's own perform advanced state WITHOUT being folded into the returned value. A
           rewrite that evaluated the postcondition first would yield `12`; one that double-performed the body
           would drift further.")
  (input  (do
            (effect Counter (op bump (-> Unit Int64)))
            (@ (ensures (> (Counter.bump) 0))
              (def (f (: n Int64)) (+ n (Counter.bump))))
            (def (main (: n Int64))
              (handle Counter 0
                ((bump (u) s (resume (+ s 1) (+ s 1))))
                (f n)))
            (export main)))
  (call   main (: 10 Int64))
  (output (: 11 Int64)))

(case "a @requires predicate that itself traps keeps its own trap kind"
  (doc    "The predicate `(> (/ 10 n) 0)` divides by its parameter, so at n=0 evaluating the PREDICATE
           traps `integer divide by zero` — a DIFFERENT kind from the requires-violation `unreachable`.
           The enforcement rewrite wraps the BODY in the predicate-guarded if; it adds no guard around the
           predicate's own evaluation, so the predicate's trap fires first and keeps its kind (trap-kind
           observability: reordering or re-classifying it would be a miscompile). n=2 satisfies (10/2=5>0)
           → 2, the control.")
  (input  (do
            (@ (requires (> (/ 10 n) 0)) (def (f (: n Int64)) n))
            (def (main (: n Int64)) (f n))
            (export main)))
  (call   main (: 2 Int64))
  (output (: 2 Int64))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "an @ensures predicate that itself traps keeps its own trap kind (not the @ensures-failed unreachable)"
  (doc    "The @ensures twin of the @requires-predicate-traps case above — the exit-side trap-kind-observability
           pin. The postcondition `(> (/ 100 ret) 0)` divides by the RESULT binder, so when `ret = 0` evaluating
           the PREDICATE traps `integer divide by zero` — a DIFFERENT kind from the postcondition-violation
           `unreachable`. The enforcement rewrite is `(let ((ret BODY)) (if (> (/ 100 ret) 0) ret (trap …)))`:
           it binds `ret` then evaluates the predicate in the `if` test, adding NO guard around the predicate's
           own evaluation — so the predicate's trap fires first and keeps its kind. `(f 5)` computes `ret = 5`,
           `(/ 100 5) = 20 > 0` holds → returns `5` (the control). `(f 0)` computes `ret = 0`, and the
           predicate's `(/ 100 0)` traps `divide by zero` BEFORE the postcondition verdict is reached —
           reordering or re-classifying it to `unreachable` would be a miscompile (the postcondition-failure
           trap only fires when the predicate EVALUATES to false, not when it traps).")
  (input  (do
            (@ (ensures (> (/ 100 ret) 0)) (def (f (: n Int64)) n))
            (def (main (: n Int64)) (f n))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64))
  (call   main (: 0 Int64))
  (trap   "divide by zero"))

(case "an @ensures over a MATCH-bodied def wraps the whole match — the postcondition checks the match's result"
  (doc    "A cross-seam composition pin (v-patterns seam): the def BODY is a `match`, and @ensures must wrap the
           WHOLE match expression, not one arm. The injected `(let ((ret (match x …))) (if (>= ret 0) ret
           (trap …)))` binds `ret` to whichever arm the scrutinee selects, then checks the postcondition over
           that result. `(f x) = (match x (0 (- 0 1)) (_ x))`: `(f 5)` takes the wildcard arm → `ret = 5`, `(>=
           5 0)` holds → returns `5`; `(f 0)` takes the `0` arm → `ret = -1`, `(>= -1 0)` is FALSE → the check
           traps `unreachable`. Pins that the enforcement rewrite composes with a match-bodied def (the `let`
           binds the match's value, the check sees the selected arm's result) — a future pattern-matching change
           that mis-scoped the injected `ret` binder around a match would flip this. Runtime scrutinee via
           `main`'s param so neither arm folds away.")
  (input  (do
            (@ (ensures (>= ret 0))
              (def (f (: x Int64)) (match x (0 (- 0 1)) (_ x))))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64))
  (call   main (: 0 Int64))
  (trap   "unreachable"))

(case "a @requires predicate that CALLS a user-defined function resolves and enforces — not only prelude ops"
  (doc    "A cross-seam composition pin (name-resolution seam): the precondition predicate is not a bare prelude
           comparison but a CALL to a user-defined function, `(ok x)` where `(def (ok n) (>= n 0))`. The
           enforcement rewrite `(if (ok x) BODY (trap …))` must RESOLVE `ok` (a top-level def, in scope at body
           entry alongside the params) and call it — predicate resolution is not restricted to prelude
           intrinsics. `(f 7)`: `(ok 7)` = true → returns `8`; `(f -3)`: `(ok -3)` = false → the precondition
           check traps `unreachable`. Runtime arg via `main`'s param so the call isn't const-folded. Pins that
           an @requires predicate may be an ordinary boolean-returning user function (the predicate is elaborated
           in the def's scope like any expression) — a resolution change that only bound prelude names in a
           predicate would break this.")
  (input  (do
            (def (ok (: n Int64)) (>= n 0))
            (@ (requires (ok x)) (def (f (: x Int64)) (+ x 1)))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 8 Int64))
  (call   main (: -3 Int64))
  (trap   "unreachable"))

(case "a @requires predicate that MATCHES on a sum-typed parameter dispatches and enforces (v-patterns seam)"
  (doc    "A cross-seam composition pin (pattern-matching seam): the precondition predicate is not a scalar
           comparison but a `match` that DISPATCHES on a sum-typed parameter, so the injected `(if (match o …)
           BODY (trap …))` must resolve + lower a full match in the predicate position, binding the payload and
           choosing the boolean arm. `(f o)` with `@requires(match o ((Opt.Some n) (>= n 0)) ((Opt.None)
           false))`: the precondition is TRUE iff `o` is `Some n` with `n >= 0`. `(f (Opt.Some 7))`: matches the
           Some arm, `(>= 7 0)` holds → body runs → `7`; `(f (Opt.Some -3))`: Some arm, `(>= -3 0)` FALSE → the
           precondition check traps `unreachable`. Runtime payload via `main`'s param so no arm folds. Pins that
           an @requires predicate may itself be a match over a sum parameter (the predicate is elaborated +
           lowered in the def's scope exactly like a body expression) — a pattern-matching change that failed to
           lower a match in the injected precondition guard would break this.")
  (input  (do
            (type Opt (None) (Some Int64))
            (@ (requires (match o ((Opt.Some n) (>= n 0)) ((Opt.None) false)))
              (def (f (: o Opt)) (match o ((Opt.Some n) n) ((Opt.None) 0))))
            (def (main (: k Int64)) (f (Opt.Some k)))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 7 Int64))
  (call   main (: -3 Int64))
  (trap   "unreachable"))

(case "an @ensures predicate reading a top-level GLOBAL alongside ret resolves and enforces (resolution seam)"
  (doc    "A cross-seam pin (name-resolution seam): the postcondition references a top-level GLOBAL definition,
           not only the result binder `ret` and the def's params. `@ensures(< ret (limit))` on `(f x) = (+ x
           1)` with `(def (limit) 100)`: the injected `(let ((ret (+ x 1))) (if (< ret (limit)) ret (trap …)))`
           must RESOLVE `(limit)` (a top-level nullary def, in scope in the predicate exactly as in any body
           expression) alongside the synthesized `ret`. `(f 5)`: `ret = 6`, `(< 6 100)` holds → returns `6`;
           `(f 200)`: `ret = 201`, `(< 201 100)` FALSE → the postcondition traps `unreachable`. Runtime arg via
           `main`'s param (no fold). Pins that predicate name-resolution reaches the global scope, not just
           params + `ret` — a resolution change that scoped the predicate too narrowly would break this.")
  (input  (do
            (def (limit) 100)
            (@ (ensures (< ret (limit))) (def (f (: x Int64)) (+ x 1)))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64))
  (call   main (: 200 Int64))
  (trap   "unreachable"))

(case "a @requires on a UNIT-returning def is enforced — the precondition traps before the unit body"
  (doc    "The degenerate-result pin: a def whose BODY is `unit` (the empty tuple) still gets its `@requires`
           enforced. The injected `(if (>= x 0) unit (trap …))` checks the precondition at body-entry regardless
           of the body's type — a unit body is not a special case that skips enforcement. `(f 5)`: `(>= 5 0)`
           holds → returns `unit` (the body value, value-transparent even for unit); `(f -1)`: `(>= -1 0)` FALSE
           → the precondition traps `unreachable` before the unit body. Pins that enforcement is orthogonal to
           the body's result type — it wraps a unit-returning def as faithfully as a scalar one (a rewrite that
           keyed on a non-unit result would drop the check here).")
  (input  (do
            (@ (requires (>= x 0)) (def (f (: x Int64)) unit))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output unit)
  (call   main (: -1 Int64))
  (trap   "unreachable"))

; ── @requires × @test: constrained GENERATION (breaker pin, keyed on the 71efd45a6 slice) ──────────
; A `@requires` precondition on a `@test`-stacked def is a FILTER on the generated input domain, not a
; property the test may fail on. The ruling (v-verification + v-property-testing, 2026-07-17): the
; @requires trap stays a HARD production contract, so the ONLY sound test-runner behavior is to DRAW
; IN-DOMAIN — a generated input violating the pre must never surface as a spurious counterexample
; (`f(-1)` under `(requires (>= x 0))` was exactly that before the constrained-gen slice). The corpus
; can't drive `cdz test` directly, so this pins the DEF-SIDE composition the runner relies on: the
; stacked def, called in-domain, enforces the pre, the body, and the post exactly as unstacked.

(case "a @test-stacked @requires+@ensures def keeps full contract enforcement for a direct call"
  (doc    "The def-side composition the constrained-gen ruling relies on: `@test` stacked over
           `(@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def f …)))` leaves the def's OWN contract
           intact for ordinary calls — in-domain `(f 5)` runs pre → body → post and returns 6;
           out-of-domain `(f -5)` still HARD-TRAPS on the pre (the production contract the test
           runner must respect by drawing in-domain, never a soft reject). Pins that the @test wrapper
           is transparent to direct-call enforcement — the test tier changes how INPUTS are drawn,
           not what the contract means.")
  (input  (do
            (@ test (@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))))
            (def (main (: n Int64)) (f n))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 6 Int64))
  (call   main (: -5 Int64))
  (trap   "unreachable"))

; ── @invariant ESTABLISH obligation (design §10.2, paper — reuses the b4c conjunction machinery) ────────
; A data-type invariant `I` on type `T` is, per §10.2, an implicit `@ensures(I)` on every CONSTRUCTOR of
; `T` (the ESTABLISH half: each constructor must prove its result satisfies `I`). So the ESTABLISH
; obligation for `@invariant(and (>= it 0) (<= it 100)) (type Percent Int64)` with a constructor
; `mk(v)` carrying `@requires(and (>= v 0) (<= v 100))` is: from the constructor's precondition hyps,
; discharge `I[self := v]` = the conjunction `(ge v 0) AND (le v 100)`. This pins that @invariant adds NO new
; kernel machinery — the establish obligation denotes + discharges through the SAME `bounds` kernel the
; @requires/@ensures cases use (here the conjunction is established directly from the matching precondition,
; the trivial-but-load-bearing case: a constructor whose @requires IS the invariant establishes it). A
; `conj` term-former mirrors `and`; the proof carries both precondition hyps and its conclusion IS the
; invariant conjunction, so `licenses` accepts it under the constructor's 2-element precondition.

(case "@invariant ESTABLISH: a constructor's @requires discharges the type invariant as an implicit @ensures (design §10.2)"
  (doc    "The DATA-level verification-family member (design §10). An `@invariant(and (>= it 0) (<= it 100))`
           on `type Percent Int64` is an implicit `@ensures(invariant)` on each constructor — the ESTABLISH
           obligation. For a constructor `mk(v)` whose `@requires(and (>= v 0) (<= v 100))` matches the
           invariant with `self := v`, the establish obligation `(conj (ge v 0) (le v 100))` is discharged
           DIRECTLY from the two precondition hypotheses (assume-both) — the constructor's precondition IS the
           invariant, the base establish case. The proof carries {ge v 0, le v 100} and concludes the invariant
           conjunction, so `licenses` accepts it under the 2-element constructor precondition. Runs to `true`.
           Pins that @invariant reuses the b4c/b2 machinery WHOLESALE — establish is `@ensures`-on-a-constructor,
           no new kernel — so the data-level family member is expressible with the existing `bounds` kernel
           (design §10.2: 'establish/preserve reuse b4c's denotation + b3's discharge, unchanged').")
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
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (ge  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 2) a) b))
      ; `conj` mirrors the surface `and` — the invariant `(and P Q)` denotes to `(conj P Q)`.
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 3) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; establish: from the two precondition facts, mint the invariant CONJUNCTION carrying both as hyps.
      (def (establish (: p Term) (: q Term)) (Thm.Seq (list p q) (conj p q)))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps ((list) false) ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs ((list) true) ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq le ge conj concl hyps establish licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq le ge conj concl hyps establish licenses))
            (def (main)
              (let ((v    (Term.Var 0))
                    (zero (Term.Num 0))
                    (c100 (Term.Num 100)))
                ; the invariant obligation I[self := v] = (conj (ge v 0) (le v 100))
                (let ((obligation   (conj (ge v zero) (le v c100)))
                      ; the constructor precondition {ge v 0, le v 100} (its @requires = the invariant)
                      (precondition (list (ge v zero) (le v c100))))
                  ; ESTABLISH: mint the invariant conjunction from the two precondition facts
                  (let ((proof (establish (ge v zero) (le v c100))))
                    (licenses proof obligation precondition)))))
            (export main)))
  (output (: true Bool)))

; ── @ensures-over-@requires × EFFECTFUL body: order-insensitive enforcement (cross-vertical, v-effects fix) ─
; Annotation stacking order is presentation, not semantics — `@ensures(Q) @requires(P)` (reversed) must
; behave exactly like the forward `@requires(P) @ensures(Q)` twin. verify_enforce wraps each annotation
; around the def's CURRENT body at its own index, so the reversed order emits, in the precondition-FAIL
; branch, `(let ((it (trap "@requires…"))) (if (> it 0) …))` — a let binding a TRAP. That let-bound trap
; USED to mis-lower: the trap types as bottom (no machine rep), so `is_scalar(it)=false` routed the scalar
; `(> it 0)` to a bogus "comparison of a compound value needs a heap walk" decline (breaker/corpus-bugfix
; repro; cdz check accepted → check/compile divergence). v-effects FIXED the lowering (a let whose init
; unconditionally traps folds straight to the trap, body unreachable — `408d12a86`). This pins the cross-
; vertical result: my reversed-stack contract composition now COMPILES + enforces over an effectful body,
; identically to the forward twin. (Flips todo→pass under the v-effects fix; a regression in either the
; composition or the let-trap lowering re-declines it.)

(case "@ensures-over-@requires stacked on an EFFECTFUL body is order-insensitive: compiles + enforces like the forward order"
  (doc    "The cross-vertical composition pin (v-verification contract enforcement × v-effects let-trap
           lowering). `(@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.tick)))))`
           under a counter handler: the reversed stack's precondition-fail branch binds `it` to the requires
           trap — `(let ((it (trap …))) (if (> it 0) it (trap …)))` — which formerly mis-declined on the
           scalar `(> it 0)` as a compound comparison (the let-bound trap typed as bottom, is_scalar=false),
           while forward order worked. The v-effects fix (a let with an unconditionally-trapping init folds to
           the trap) makes it lower correctly, so the reversed order now behaves EXACTLY like the forward
           twin: pre `(>= 100 0)` ok, body `(+ 100 (St.tick))` resumes 1 → 101, post `(> 101 0)` ok → 101.
           Pins that contract stacking order is presentation, not semantics, over an effect-performing body —
           and guards the let-bound-trap lowering my composition relies on.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (@ (ensures (> ret 0))
              (@ (requires (>= x 0))
                (def (f (: x Int64)) (+ x (St.tick)))))
            (def (main (: k Int64))
              (handle St k
                ((tick (u) s (resume (+ s 1) (+ s 1))))
                (f 100)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 101 Int64)))

; ── @invariant PRESERVE + consumer-gift (design §10.2, paper — reuses the establish/discharge machinery) ──
; The dual of ESTABLISH (every constructor proves I on its result). PRESERVE: every OPERATION returning T
; must maintain I on its result — I is an implicit @ensures(I) on the result — AND (the dual gift) a
; consumer may ASSUME I on any T INPUT for free (an implicit @requires(I) granted, since every T value
; provably holds I). So an operation `f : Percent -> Percent` discharges `I(result)` USING `I(input)` as a
; free hypothesis. Here `dec` lowers a Percent by 1: from the input gift `(ge in 0) AND (le in 100)`, the
; result `in-1` still satisfies `(ge (in-1) MININT-ish) …` — modelled minimally as: the result upper bound
; `le (result) 100` follows from the input `le in 100` (dec never raises it), discharged via the input-gift
; hypothesis. This pins that PRESERVE reuses the establish/discharge machinery with the input invariant as
; a granted precondition — no new kernel; the consumer-gift is exactly @requires-you-get-free.

(case "@invariant PRESERVE: an operation returning T discharges the result invariant USING the input invariant as a free gift (design §10.2)"
  (doc    "The PRESERVE half + consumer-gift (design §10.2), dual to the ESTABLISH case. An operation
           `f : Percent -> Percent` must maintain the invariant on its RESULT (implicit @ensures(I)), and may
           ASSUME the invariant on its Percent INPUT for free (the dual gift — every Percent provably holds
           I). So `f` discharges `I(result)` USING `I(input)` as a granted hypothesis. Modelled minimally for
           a `dec` (lower by 1): the result's upper bound `le (dec-result) (Num 100)` follows from the input
           gift `le in (Num 100)` (dec never raises the value) via mono/trans — the input invariant hypothesis
           is what makes the result invariant provable. `licenses` accepts the proof under a precondition that
           INCLUDES the input-invariant gift. Runs to `true`. Pins that PRESERVE reuses the establish/discharge
           machinery with the input invariant as a granted @requires — the consumer-gift is exactly
           `@requires`-you-get-free-on-a-T-input, no new kernel (design §10.2: 'simultaneously a proof
           obligation on producers and a proof gift to consumers').")
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
      (def (sub (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 4) a) b))
      (def (le  (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Const 1) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; PRESERVE step: from `|- (le in c)` derive `|- (le (sub in 1) c)` — decreasing the lhs keeps `<= c`
      ; (dec never raises the value, so the upper bound is preserved). Hyps carried unchanged (the input gift).
      (def (dec-le (: th Thm))
        (match (concl th)
          ((Term.Comb (Term.Comb (Term.Const 1) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (sub x (Term.Num 1)) c))))
          (_ (Option.None))))
      (def (mem (: q Term) (: ps (List Term)))
        (match ps ((list) false) ((list h .. t) (if (term-eq q h) true (mem q t)))))
      (def (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs ((list) true) ((list h .. t) (if (mem h pre) (hyps-subset t pre) false))))
      (def (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export (. Term *))
      (export Thm)
      (export term-eq sub le concl hyps assume dec-le licenses)))
  (input  (do
            (import "bounds" (Term Thm term-eq sub le concl hyps assume dec-le licenses))
            (def (main)
              (let ((in   (Term.Var 0))
                    (c100 (Term.Num 100)))
                ; the RESULT-invariant obligation: le (dec in) 100  (the Percent upper bound on the result)
                (let ((obligation   (le (sub in (Term.Num 1)) c100))
                      ; the granted consumer-gift: the INPUT invariant `le in 100` (every Percent holds it)
                      (precondition (list (le in c100))))
                  ; PRESERVE: assume the input gift, decrement, and the result upper bound follows
                  (let ((gift (assume (le in c100))))
                    (match (dec-le gift)
                      ((Option.Some proof) (licenses proof obligation precondition))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ── @invariant NAME-RESOLUTION: a predicate name outside {it, prelude} is unbound (b4c pattern, data-level) ─
; An `@invariant(pred)` predicate references only the value binder `self` (the value of the type) and prelude/
; global names — a type declaration has no parameters. A name that is NEITHER is UNBOUND, reported CDZ0101 at
; the annotation (the same b4c name-resolution the @requires/@ensures predicates get, reused for the data-
; level member via `Db::invariant_preds`). Pins that a stray name in a data invariant is caught locally, not
; silently accepted (the soundness discipline: a contract predicate resolves like ordinary code).

(case "@invariant with an unbound predicate name is REJECTED (CDZ0101 — only `self` + prelude are in scope)"
  (doc    "The data-level name-resolution pin. `@invariant(and (>= it 0) (< it bogus))` on `type Percent`:
           `self` is the value binder (in scope) and `>=`/`<`/`and` are prelude ops (resolve), but `bogus` is
           neither a prelude name nor the value binder — so it is UNBOUND, CDZ0101 at the annotation. A type
           has no parameters, so the invariant predicate's scope is exactly {`self`, prelude/global} — anything
           else is a stray name. Pins that `collect_faults` name-resolves the invariant predicate (via
           `Db::invariant_preds`) with the same b4c discipline the @requires/@ensures predicates get, so a
           typo'd data invariant fails locally with a clear message rather than being silently recorded.")
  (input  (do
            (@ (invariant (and (>= self 0) (< self bogus))) (type Percent (Pct Int64)))
            (def (main) 0)
            (export main)))
  (error  CDZ0101))

; ── @invariant ESTABLISH Part 1: a BARE scalar invariant on a newtype AUTO-UNWRAPS + type-checks ──────────
; The establish checker `invariant_establish::synthesize` emits `(def (__invariant_check_T (: it T)) …)` per
; @invariant type so the predicate is TYPE-CHECKED. For a single-payload newtype it AUTO-UNWRAPS: a bare
; `(>= it 0)` — which alone would hit the nominal boundary (Percent not comparable to Int64, CDZ0202) — is
; rewritten to run over the unwrapped payload, so it type-checks. Pins that the natural bare form COMPILES
; (the author need not destructure) and the type remains usable end-to-end. (The run-time establish TRAP at
; each construction is Part 2; this pins Part 1 — the typed checker — is behavior-neutral for a value that
; SATISFIES the invariant, i.e. construction + use still works.)

(case "@invariant ESTABLISH Part 1: a bare-scalar invariant on a newtype auto-unwraps + type-checks; a satisfying value constructs and is usable"
  (doc    "The establish checker synthesized by `invariant_establish` type-checks the @invariant predicate.
           For the single-payload newtype `(type Percent (Pct Int64))` with the BARE `@invariant(and (>= it 0)
           (<= it 100))`, the checker AUTO-UNWRAPS — `(match it (((. Percent Pct) __u) (and (>= __u 0)
           (<= __u 100))))` — so the bare scalar predicate type-checks (it would otherwise fail CDZ0202 on the
           nominal boundary). Pins that the natural bare form compiles and the type is usable: `(mk 42)` builds
           a `Percent` and `unwrap` reads its payload back → 42. (The run-time establish check that TRAPS on a
           VIOLATING construction is Part 2; Part 1 is the typed checker, behavior-neutral for a satisfying
           value — construction + use unchanged.)")
  (input  (do
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (mk (: v Int64)) (Percent.Pct v))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main) (unwrap (mk 42)))
            (export main)))
  (output (: 42 Int64)))

; ── @invariant ESTABLISH Part 2: the CHECKED CONSTRUCTOR enforces the invariant at RUN TIME (a violation TRAPS) ─
; The (D) run-time establish enforcement (design §10.2 — ESTABLISH: every construction of `T` must satisfy `I`,
; else trap). Part 1 synthesizes the typed `__invariant_check_T`; Part 2 synthesizes the CHECKED CONSTRUCTOR
; `(def (__invariant_construct_T (: __inv_p U)) (let ((__inv_v (T.V __inv_p))) (if (__invariant_check_T __inv_v)
; __inv_v (trap))))` — it builds the value once (`__inv_v : T`, properly typed), checks it, and yields it or
; traps. This slice synthesizes it UNWIRED (the `lower_sum_new` divert that routes every `(T.V x)` through it is
; the follow-up); a source that calls it BY NAME exercises the establish behavior directly. Pins: a SATISFYING
; value constructs + flows through (mk 50 = 50), and a VIOLATING value TRAPS at construction (mk 150 > 100).
; This is the run-time complement of Part 1's type-check-only positive case above.

(case "@invariant ESTABLISH Part 2: the synthesized checked constructor enforces the invariant at run time — a satisfying value constructs, a violating value traps (design §10.2, (D))"
  (doc    "The (D) run-time establish enforcement. `invariant_establish` synthesizes, per single-payload-newtype
           @invariant type, a CHECKED CONSTRUCTOR `__invariant_construct_Percent` = `(let ((__inv_v (Percent.Pct
           __inv_p))) (if (__invariant_check_Percent __inv_v) __inv_v (trap)))`. Called with a value SATISFYING
           `0 <= it <= 100` it constructs the Percent and yields it (here unwrapped to its Int64 payload — mk(50)
           = 50); called with a VIOLATING value it TRAPS at construction (mk(150) violates `<= 100`), so no
           invalid Percent ever escapes. Pins the establish obligation is enforced at run time (the trap), the
           dynamic complement of the compile-time discharge the establish/preserve corpus above pins. The def is
           synthesized UNWIRED here (called by name); wiring `lower_sum_new` to route every `(Percent.Pct x)`
           through it is the follow-up sub-slice.")
  (input  (do
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (mk (: v Int64)) (match (__invariant_construct_Percent v) (((. Percent Pct) n) n)))
            (export mk)))
  (call mk (: 50 Int64))  (output (: 50 Int64))
  (call mk (: 0 Int64))   (output (: 0 Int64))
  (call mk (: 100 Int64)) (output (: 100 Int64))
  (call mk (: 150 Int64)) (trap "unreachable")
  (call mk (: -1 Int64))  (trap "unreachable"))

; ── @invariant ESTABLISH Part 2 (the DIVERT): a PLAIN construction AUTO-ESTABLISHES at the call site ─────────
; The wiring. `lower_sum_new` routes a single-payload construction `(Percent.Pct v)` of an @invariant newtype
; through the synthesized checked constructor (`Core::Call { __invariant_construct_Percent, [v] }`) instead of
; erasing straight to the payload — so EVERY construction establishes the invariant at run time, with NO
; `__invariant_construct` named call in the source. The author writes the natural `(Percent.Pct v)` and a
; violating value TRAPS at the construction site. The checked constructor's OWN inner `((. Percent Pct) __inv_p)`
; is EXEMPT (recorded at load), so the divert does not recurse. This is the run-time establish enforcement made
; TRANSPARENT — the previous case pins the checked constructor's behavior when called BY NAME; this pins that an
; ordinary construction is diverted through it automatically.

(case "@invariant ESTABLISH Part 2 (divert): a plain `(Percent.Pct v)` construction auto-establishes — a satisfying value constructs, a violating value traps at the call site (design §10.2, (D))"
  (doc    "The establish DIVERT wiring. `mk` builds a Percent with the PLAIN constructor `(Percent.Pct v)` — no
           `__invariant_construct` by name. `lower_sum_new` diverts that single-payload construction of the
           @invariant newtype through the synthesized checked constructor, so a satisfying value constructs and
           flows through (mk(50) = 50, value-transparent) while a VIOLATING value traps at the construction site
           (mk(150) violates `<= 100`). No invalid Percent is ever built, and the author wrote no call-site
           annotation. The checked constructor's own inner construction is exempt from the divert (no recursion).
           Pins that the run-time establish enforcement is TRANSPARENT — every ordinary construction is checked.")
  (input  (do
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (mk (: v Int64)) (match (Percent.Pct v) (((. Percent Pct) n) n)))
            (export mk)))
  (call mk (: 50 Int64))  (output (: 50 Int64))
  (call mk (: 0 Int64))   (output (: 0 Int64))
  (call mk (: 100 Int64)) (output (: 100 Int64))
  (call mk (: 150 Int64)) (trap "unreachable")
  (call mk (: -1 Int64))  (trap "unreachable"))

; ── @invariant ESTABLISH (divert) over a HEAP payload: NonEmptyList — the design's second canonical example ──
; The establish divert is payload-KIND-general: it works for a newtype over a HEAP value (a `(List …)`), not
; only a scalar. `NEList = Mk (List Int64)` with `@invariant(< 0 (List.len it))` — the design's `NonEmptyList`
; case (§10.1). `mkfrom` builds the list in-body (a `list` of one for n>0, the empty `list` otherwise) and
; constructs `(NEList.Mk …)`; the divert routes it through the checked constructor, whose auto-unwrap accessor
; `(< 0 (List.len it))` reads the underlying list length. A non-empty list satisfies (mkfrom(5) → len 1); the
; EMPTY list violates and TRAPS at construction (mkfrom(0)). Pins that the single-payload-newtype establish
; path landed for the scalar case generalizes to a heap payload with an accessor-shaped invariant — no invalid
; NonEmptyList is ever built. (The value is used in-body, not exported: a `(List …)` has no boundary rep.)

(case "@invariant ESTABLISH (divert) over a heap payload: a NonEmptyList newtype traps on the empty list, constructs a non-empty one (design §10.1/§10.2, (D))"
  (doc    "The establish divert is general over the payload KIND — here a HEAP `(List Int64)`, the design's
           `NonEmptyList`. `NEList = Mk (List Int64)` carries `@invariant(< 0 (List.len it))`. `mkfrom` builds
           the payload list in-body — `(list n)` for n>0 (length 1, satisfies) else the empty `(list)` (length
           0, violates) — and constructs `(NEList.Mk …)`, which the divert routes through the checked
           constructor; its accessor invariant `(< 0 (List.len it))` reads the underlying list length. So
           mkfrom(5) yields 1 (a non-empty list constructs and its length reads back) and mkfrom(0) TRAPS at
           construction (the empty list is not a legal NonEmptyList). Pins the establish path generalizes from
           the scalar newtype (Percent) to a heap payload with an accessor-shaped invariant.")
  (input  (do
            (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
            (def (mkfrom (: n Int64))
              (match (NEList.Mk (if (> n 0) (list n) (list)))
                (((. NEList Mk) ys) (List.len ys))))
            (export mkfrom)))
  (call mkfrom (: 5 Int64)) (output (: 1 Int64))
  (call mkfrom (: 0 Int64)) (trap "unreachable"))

; ── @invariant ESTABLISH over a MULTI-VARIANT sum: each variant's construction auto-establishes ──────────────
; The multi-variant generalization (design §10.2 — a per-CONSTRUCTOR obligation). A ≥2-variant sum is BOXED
; (`Core::SumNew{disc, payloads}`), not erased, so it never hits the newtype path. `invariant_establish`
; synthesizes ONE checked constructor per variant (`__invariant_construct_Shape__d<disc>`, keyed by the
; discriminant the boxed-construction divert has in hand), each calling the whole-value `__invariant_check_Shape`
; (Part 1, `it : Shape`, the author's own match reads the variant). So a construction of EITHER variant is
; routed through its per-variant checked constructor: a satisfying value constructs, a violating one TRAPS. This
; pins both the 1-payload `Circle` arm (disc 0) and the 2-payload `Square` arm (disc 1) — a multi-payload
; construct-def. `circ`/`sq` build a shape then re-match to a scalar so the export crosses the boundary.

(case "@invariant ESTABLISH over a multi-variant sum: each variant's construction auto-establishes — a satisfying value constructs, a violating one traps (design §10.2, (D))"
  (doc    "The multi-variant establish. `Shape = Circle Int64 | Square Int64 Int64` with a per-variant invariant
           (a Circle's radius > 0; a Square's sides both > 0). Each variant construction is routed through its
           synthesized per-variant checked constructor (`__invariant_construct_Shape__d0` for Circle,
           `__d1` for Square), which calls the whole-value `__invariant_check_Shape`. `circ(r)` builds a Circle
           and returns its radius (via re-match); `sq(w,h)` builds a Square and returns w+h. A satisfying value
           of either variant constructs (circ(5)=5, sq(3,4)=7); a violating value of either traps at
           construction (circ(0), circ(-3), sq(3,0), sq(0,4)). Pins the per-constructor establish obligation
           over a boxed multi-variant sum, including the 2-payload Square arm.")
  (input  (do
            (@ (invariant (match self (((. Shape Circle) r) (> r 0))
                                    (((. Shape Square) w h) (and (> w 0) (> h 0)))))
               (type Shape (Circle Int64) (Square Int64 Int64)))
            (def (circ (: r Int64))
              (match (Shape.Circle r) (((. Shape Circle) x) x) (((. Shape Square) w h) (+ w h))))
            (def (sq (: w Int64) (: h Int64))
              (match (Shape.Square w h) (((. Shape Circle) x) x) (((. Shape Square) a b) (+ a b))))
            (export circ)
            (export sq)))
  (call circ (: 5 Int64))           (output (: 5 Int64))
  (call circ (: 0 Int64))           (trap "unreachable")
  (call circ (: -3 Int64))          (trap "unreachable")
  (call sq (: 3 Int64) (: 4 Int64)) (output (: 7 Int64))
  (call sq (: 3 Int64) (: 0 Int64)) (trap "unreachable")
  (call sq (: 0 Int64) (: 4 Int64)) (trap "unreachable"))

; ── @invariant ESTABLISH over a SINGLE-VARIANT MULTI-PAYLOAD newtype: the tuple-erase construct path ─────────
; The third establish shape. `(type Range (Mk Int64 Int64))` is a single-variant, MULTI-payload newtype — it
; erases to a `Ty::Tuple`, NOT a single-payload value, so it takes neither the single-PAYLOAD newtype divert
; (`args.len()==1`) nor the boxed multi-VARIANT one. Without a divert here it would construct with NO establish
; check (a real (D) soundness gap — an invalid Range could be built). `invariant_establish` synthesizes its
; sole variant's checked constructor `__invariant_construct_Range__d0` (the per-variant path now fires for any
; non-sole-payload-newtype), and the tuple-erase arm of `lower_sum_new` diverts the 2-payload construction
; through it. A relational invariant `(<= lo hi)` over the two payloads: an ordered pair constructs, a
; misordered one TRAPS. `mk` builds a Range then re-matches to `(- hi lo)` so the export crosses the boundary.

(case "@invariant ESTABLISH over a single-variant multi-payload newtype: an ordered Range constructs, a misordered one traps (design §10.2, (D))"
  (doc    "The third establish shape — a single-variant MULTI-payload newtype `(type Range (Mk Int64 Int64))`,
           which erases to a tuple. Its relational `@invariant(<= lo hi)` is checked at construction via the
           synthesized `__invariant_construct_Range__d0` (the tuple-erase divert's callee). `mk(lo,hi)` builds a
           Range and returns `hi - lo`. An ordered pair satisfies and constructs (mk(3,7)=4, mk(5,5)=0); a
           misordered pair violates `<= lo hi` and TRAPS at construction (mk(7,3)). Pins the establish path
           over the multi-payload-newtype shape (a relational invariant across the two payloads), closing the
           gap where a 2-payload newtype used to construct with no check.")
  (input  (do
            (@ (invariant (match self (((. Range Mk) lo hi) (<= lo hi)))) (type Range (Mk Int64 Int64)))
            (def (mk (: lo Int64) (: hi Int64))
              (match (Range.Mk lo hi) (((. Range Mk) a b) (- b a))))
            (export mk)))
  (call mk (: 3 Int64) (: 7 Int64)) (output (: 4 Int64))
  (call mk (: 5 Int64) (: 5 Int64)) (output (: 0 Int64))
  (call mk (: 7 Int64) (: 3 Int64)) (trap "unreachable"))

; ── @invariant ESTABLISH over a NULLARY variant: the unit-construction path (the last establish shape) ───────
; A nullary variant carries no payload, but it is still a VALUE of the type, so its construction must satisfy
; the invariant. An `@invariant` that REJECTS the nullary variant — `(match self (((. T A)) false) …)`, making
; `A` uninhabitable — must TRAP when `A` is constructed. `invariant_establish` synthesizes a NO-ARG checked
; constructor `__invariant_construct_T__d0` (body `(let ((__inv_v (T.A unit))) (if (check __inv_v) __inv_v
; (trap)))`), and the nullary-unit path of `lower_sum_new` diverts `(T.A unit)` through it (its own inner
; construction exempt, no recursion). So constructing the rejected `A` traps; the accepted `B x` (x>0) still
; constructs. This closes the LAST establish shape — every variant kind (single/multi-payload newtype,
; multi-variant sum, nullary) now establishes at construction, and PRESERVE follows for free (an op returning T
; builds its result through the SAME checked constructor, so a result violating the invariant traps there too).
; (Re-added after v-syntax's ML-printer fix `6496308e7` made a nullary variant under `@invariant` round-trip.)

(case "@invariant ESTABLISH over a nullary variant: a rejected nullary variant traps at construction, an accepted payload variant constructs (design §10.2, (D))"
  (doc    "The last establish shape — a NULLARY variant. `T = A | B Int64` with an invariant that rejects `A`
           outright (`false`) and accepts `B x` when x>0. `mka` constructs `A`; because the invariant makes `A`
           uninhabitable, the synthesized no-arg checked constructor `__invariant_construct_T__d0` traps.
           `mkb(x)` constructs `B x`: x>0 satisfies (mkb(5)=5), x<=0 traps (mkb(0)). Pins that a nullary variant
           establishes at its unit-construction path — the invariant holds for EVERY value including the
           payloadless ones, so an uninhabitable nullary variant is caught at construction. (The invariant
           value binder is `self`, per the operator's ret/self ruling.)")
  (input  (do
            (@ (invariant (match self (((. T A)) false) (((. T B) x) (> x 0)))) (type T (A) (B Int64)))
            (def (mka) (match (T.A unit) (((. T A)) 0) (((. T B) x) x)))
            (def (mkb (: x Int64)) (match (T.B x) (((. T A)) 0) (((. T B) y) y)))
            (export mka)
            (export mkb)))
  (call mka)                (trap "unreachable")
  (call mkb (: 5 Int64))    (output (: 5 Int64))
  (call mkb (: 0 Int64))    (trap "unreachable"))

; ── @ensures on a def RETURNING an @invariant type: BOTH checks fire independently (composition edge) ────────
; The two (D) run-time members COMPOSE on one def: a def with an `@ensures(Q on ret)` whose RESULT type carries
; its own `@invariant(I on self)`. The result binder `ret` IS an `@invariant`-typed value, so TWO independent
; checks apply — (a) the ESTABLISH trap fires at the `(Pct.P v)` construction INSIDE the body (the invariant on
; the constructed value), and (b) the `@ensures` postcondition trap fires at body-exit on `ret`. They are
; distinct obligations at distinct sites: an in-range value that fails the postcondition traps at EXIT, while
; an out-of-range value traps EARLIER at construction (establish), before the postcondition is even reached.
; Pins that neither check subsumes or masks the other — a future change that folded them would drop one guard.

(case "@ensures on a def returning an @invariant type: establish (on construction) AND the postcondition (on ret) both fire (design §10, (D))"
  (doc    "The composition of two (D) members on one def. `Pct = P Int64` has `@invariant(0 <= self <= 100)`;
           `mk` has `@ensures(ret's payload >= 50)` and returns a `Pct`. `run(v)` calls `mk` then unwraps.
           run(70): the Pct establish (0..100) holds AND the ensures (>=50) holds → 70 flows. run(30): the Pct
           establish holds (30 in 0..100) but the ensures postcondition (30 >= 50) FAILS → trap at body-EXIT.
           run(150): the Pct ESTABLISH (<=100) fails at the `(Pct.P 150)` construction INSIDE mk's body → trap
           there, before the postcondition is reached. Pins that the establish trap and the @ensures trap are
           INDEPENDENT obligations at distinct sites — neither subsumes the other. (`ret`/`self` are the
           operator's binder names; here `ret` is itself an @invariant-typed value.)")
  (input  (do
            (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
            (@ (ensures (match ret (((. Pct P) n) (>= n 50)))) (def (mk (: v Int64)) (Pct.P v)))
            (def (run (: v Int64)) (match (mk v) (((. Pct P) n) n)))
            (export run)))
  (call run (: 70 Int64))  (output (: 70 Int64))
  (call run (: 30 Int64))  (trap "unreachable")
  (call run (: 150 Int64)) (trap "unreachable"))

; ── @invariant ESTABLISH divert: NO escape through indirect construction sites ──────────────────────────────
; The divert is a construction-SITE rewrite (`lower_sum_new` routes `(Percent.Pct v)` through the checked
; constructor), so its soundness rests on the rewrite reaching EVERY site the lowering walks — a site the
; visitor missed would build an unchecked, possibly-invalid value silently. The cases above all construct in a
; def's immediate body; this pins three indirect sites a site-based rewrite historically misses: (1) a
; construction inside a LAMBDA body, applied at runtime — the divert must fire inside the lifted closure code,
; not only in def-level bodies; (2) a DECONSTRUCT-then-RECONSTRUCT (an update helper unwraps, adjusts the
; payload, re-wraps) — the RE-construction is a fresh establish obligation, so an update that pushes the
; payload out of range traps even though both inputs were valid Percents; (3) a construction as a LIST element
; — the value is built inside a heap-collection initializer, not a scalar binding position. Each face flows a
; satisfying value and traps a violating one.

(case "@invariant ESTABLISH divert reaches indirect construction sites: a lambda body, a reconstruct-after-update, and a list element all establish"
  (doc    "Escape-face pins for the establish divert. `via-lambda` constructs `(Percent.Pct x)` inside a
           LAMBDA applied to a runtime argument — the divert fires inside the closure body (via-lambda(50)=50,
           via-lambda(150) traps). `via-bump` deconstructs a VALID Percent, adds a delta, and RE-constructs —
           the re-wrap is its own establish obligation, so 50+10=60 flows but 90+20=110 traps at the re-wrap
           (no invalid Percent escapes an update helper). `via-list` constructs as a LIST element and reads it
           back (via-list(50)=50, via-list(150) traps inside the collection initializer). Together these pin
           that the divert is reachability-complete over lambda bodies, update re-wraps, and collection
           element positions — the sites a construction-site rewrite would silently miss.")
  (input  (do
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (unp (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (via-lambda (: v Int64))
              (unp ((fn ((: x Int64)) (Percent.Pct x)) v)))
            (def (bump (: p Percent) (: d Int64))
              (match p (((. Percent Pct) n) (Percent.Pct (+ n d)))))
            (def (via-bump (: v Int64) (: d Int64)) (unp (bump (Percent.Pct v) d)))
            (def (via-list (: v Int64))
              (match (list (Percent.Pct v) (Percent.Pct 5))
                ((list h .. _) (unp h)) (_ 0)))
            (export via-lambda)
            (export via-bump)
            (export via-list)))
  (call via-lambda (: 50 Int64))              (output (: 50 Int64))
  (call via-lambda (: 150 Int64))             (trap "unreachable")
  (call via-bump (: 50 Int64) (: 10 Int64))   (output (: 60 Int64))
  (call via-bump (: 90 Int64) (: 20 Int64))   (trap "unreachable")
  (call via-list (: 50 Int64))                (output (: 50 Int64))
  (call via-list (: 150 Int64))               (trap "unreachable"))
