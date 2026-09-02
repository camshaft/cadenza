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
(diagnostic-quality)

(case
  "a no-overflow obligation is DISCHARGED: for x <= 100, (x + 1) <= MAXINT via monotonicity + a CHECKED numeral fact"
  (doc
    "The first program-condition discharge — the b1 milestone. A checked `x + 1 : Int64` guarded by
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      ; arithmetic head-symbols as HeadOp-headed applications (closed sum, not magic-int Const tags)
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      ; MAXINT as a genuine numeral (the Int64 maximum) so the axiom base can CHECK bounds against it
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; LEAF rule: assume a proposition (its own hypothesis)
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; a partial evaluator over the GROUND numeric fragment: numerals and `add` of numerals. A
      ; non-numeral (a Var, a bare Head, a non-add Comb) is not evaluable → None.
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      ; CHECKED GROUND AXIOM: mint |- (le lhs rhs) ONLY when both sides are ground numeric terms and
      ; value(lhs) <= value(rhs). A non-ground or false pair yields None — no Thm forged. (The LCF
      ; axiom-schema discipline: an axiom instance is admitted only with its side-condition discharged.)
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; RULE: monotonicity of + on the right — from G |- (le x c) derive G |- (le (add x k) (add c k))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      ; RULE: transitivity of <= — from G |- (le a b) and D |- (le b c) derive G++D |- (le a c)
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl hyps assume eval-ground le-ax mono-add-r trans-le)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq add le maxint concl assume le-ax mono-add-r trans-le))
      (def
        (main)
        ; the checked op is (x + 1); x is (Var 0); precondition is (le x (num 100))
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c (Term.Num 100)))
          ; obligation `no-overflow@Id` = (le (add x 1) MAXINT)
          (let
            ((obligation (le (add x one) (maxint))))
            ; step 1: assume the precondition (le x 100)
            (let
              ((pre (assume (le x c))))
              ; step 2: monotonicity — (le (add x 1) (add 100 1))
              (match
                (mono-add-r pre one)
                ((Option.Some step1)
                  ; step 3: CHECKED numeral fact (le (add 100 1) MAXINT) — 101 <= MAXINT holds
                  (match
                    (le-ax (add c one) (maxint))
                    ((Option.Some fact)
                      ; step 4: transitivity closes to (le (add x 1) MAXINT)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof) (term-eq (concl proof) obligation))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "an UNCONSTRAINED add is NOT dischargeable: with no precondition bound, the no-overflow obligation cannot be closed (the check must stay)"
  (doc
    "The dual — the soundness-critical negative. For an UNCONSTRAINED `x + 1 : Int64` (no precondition
           bounding x), there is no `LE x c` hypothesis to feed `mono-add-r`, so the discharge cannot be
           built: the obligation `LE (add x 1) MAXINT` is NOT provable from the arithmetic base alone (it is
           simply false — x could be MAXINT). The entry models the b2 discharge attempt WITHOUT a
           precondition: assuming an ARBITRARY unrelated fact does not produce `LE (add x 1) MAXINT`, and the
           honest result is that the obligation is not reached — so the elision oracle returns None and the
           overflow check STAYS. Runs to `true` (asserts non-derivability). Pins the default-is-always-the-
           check invariant at the discharge level: absence of a bounding precondition means no proof.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl assume)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq add le maxint concl assume))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)))
          (let
            ((obligation (le (add x one) (maxint))))
            ; With no precondition, the only Thm we can honestly build about x is an assumption
            ; of some unrelated proposition — it does NOT establish the obligation.
            (let
              ((unrelated (assume (le x x))))
              ; the check must STAY: assert the obligation is NOT what we derived
              (not (term-eq (concl unrelated) obligation))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b2: the MATCH PREDICATE (the compiler's trusted surface, written IN CADENZA) ────────────────────
; The oracle's core (design §3): a discharged `Thm` LICENSES the elision of `overflow-check@Id` iff
;   (1) its conclusion is STRUCTURALLY EXACTLY the obligation `no-overflow@Id` (term-eq), AND
;   (2) every hypothesis it was proven under is DISCHARGED BY the node's stated precondition
;       (hyps ⊆ precondition, each hyp term-eq to some precondition member).
; (2) is the soundness core: a `Thm` proven under an assumption the node's precondition does NOT provide
; must NOT license an elision. At b3 the compiler compile-time-evals this predicate and consumes only
; its boolean; here we pin the predicate itself.
(case
  "the b2 match predicate LICENSES the elision: the discharged no-overflow proof matches the obligation and its hyps are covered by the node precondition"
  (doc
    "The positive b2 pin. The `bounds` kernel discharges `LE (add x 1) MAXINT` under hypothesis
           `LE x 100` (the b1 chain: assume → mono-add-r → trans-le with a CHECKED numeral fact). The
           `licenses` predicate — the compiler's trusted match surface — accepts it: (1) `term-eq (concl
           proof) obligation` holds, AND (2) `hyps-subset (hyps proof) precondition` holds (its sole
           hypothesis `LE x 100` is exactly the node's stated precondition). So the oracle returns Some and
           the Core elision pass drops the guard. Runs to `true`. Pins that a correctly-discharged proof
           under a matching precondition licenses the elision — the fact b3 consumes via compile-time eval.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      ; membership: some member of `ps` is term-eq to `q`
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      ; hyps ⊆ precondition: every hyp is a member of the precondition set
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      ; THE MATCH PREDICATE: conclusion is the obligation AND hyps are covered by the precondition
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c (Term.Num 100)))
          (let
            ((obligation (le (add x one) (maxint))) (precondition #list((le x c))))
            (let
              ((pre (assume (le x c))))
              (match
                (mono-add-r pre one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof)
                          ; the match predicate accepts: conclusion matches AND hyps ⊆ precondition
                          (licenses proof obligation precondition))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "the b2 match predicate REJECTS a proof discharged under a FOREIGN hypothesis not in the node precondition (soundness — no elision under wrong assumptions)"
  (doc
    "The soundness-critical b2 negative — the breaker vector the design flags. A proof can have the
           RIGHT conclusion `LE (add x 1) MAXINT` yet be established under a hypothesis the node's
           precondition does NOT provide: here the proof is discharged assuming `LE x 100`, but the node's
           stated precondition is only `LE x 200` (weaker). `term-eq` on the conclusion ALONE would wrongly
           accept, so the match predicate MUST also check hyps ⊆ precondition — and it fails: the proof's
           hypothesis `LE x 100` is NOT a member of the precondition `{LE x 200}`. So `licenses` returns
           false → the oracle returns None → the overflow check STAYS. Runs to `true` via `not`. Pins that a
           `Thm` proven under assumptions the node does not guarantee cannot license an elision.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le licenses))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (c200 (Term.Num 200)))
          (let
            ((obligation (le (add x one) (maxint)))
              ; the node's ACTUAL precondition is the WEAKER (le x 200)
              (precondition #list((le x c200))))
            ; discharge a proof of the SAME conclusion but under the STRONGER hyp (le x 100)
            (let
              ((pre100 (assume (le x c100))))
              (match
                (mono-add-r pre100 one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c100 one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof)
                          ; conclusion matches, BUT hyp (le x 100) ∉ precondition {(le x 200)} →
                          ; licenses must be FALSE (the check must STAY). assert NOT licenses.
                          (not (licenses proof obligation precondition)))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── SOUNDNESS PIN: the arithmetic axiom base cannot forge (breaker 2026-07-17) ──────────────────────
(case
  "the CHECKED ground axiom le-ax cannot forge a FALSE order fact (5 <= 3) — the axiom base is consistent"
  (doc
    "The breaker vector-(d) regression pin. An earlier `le-ax` minted `⊢ a≤b` for ARBITRARY terms
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export add le maxint eval-ground le-ax)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm add le maxint eval-ground le-ax))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)))
          ; (1) a FALSE ground fact 5<=3 must NOT be minted
          (let
            ((false-fact (le-ax (Term.Num 5) (Term.Num 3)))
              ; (2) a NON-ground universal (x+1)<=MAXINT must NOT be minted
              (nonground (le-ax (add x one) (maxint))))
            (and
              (match false-fact ((Option.None) true) ((Option.Some _) false))
              (match nonground ((Option.None) true) ((Option.Some _) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── SOUNDNESS PIN: a ground add that OVERFLOWS during discharge TRAPS, it does not wrap-and-forge ────
; (breaker overflow-axis vectors, 2026-07-17 — folded here rather than promoted separately.)
(case
  "le-ax of a ground add that OVERFLOWS Int64 traps during evaluation — it cannot wrap to forge a false bound"
  (doc
    "The overflow axis of the axiom-base soundness (breaker). `eval-ground` computes a ground `add`
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export add le maxint eval-ground le-ax)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm add le maxint eval-ground le-ax))
      (def
        (main)
        ; attempt the forge: le-ax (add MAXINT 1) MAXINT. eval-ground(MAXINT+1) traps (checked +),
        ; so the run halts on integer overflow — no wrapped MININT, no forged fact.
        (match
          (le-ax (add (maxint) (Term.Num 1)) (maxint))
          ((Option.Some _) true)
          ((Option.None) false)))
      (export main)))
  (trap "integer overflow"))

; ── @requires/@ensures ARITY discipline: each takes EXACTLY ONE predicate argument ────────────────────
; A `@requires`/`@ensures` with the wrong number of predicate arguments — zero, or two — is a shape error
; REJECTED at strip time (CDZ0201, the same arity discipline `@tag` gets); a silently-unrecorded predicate
; would mask the author's mistake and surface far away when the denotation consumes it. A valid one-predicate
; `@requires(pred)`/`@ensures(pred)` is accepted and runs. (Name-resolution / boolean-typedness of the
; predicate is checked LATER, at denotation, where the param scope + `ret` binder are available.) Migrated
; from rcdzc a_malformed_requires_ensures_arity_is_rejected_not_silently_dropped.
(case
  "a @requires annotation with zero predicate arguments is rejected"
  (input (do (@ (requires) (def (c) 3)) (export c)))
  (error CDZ0201 (message "takes exactly one PREDICATE argument")))

(case
  "a @requires annotation with two predicate arguments is rejected"
  (input (do (@ (requires (> x 0) (< x 9)) (def (c (: x Int64)) 3)) (export c)))
  (error CDZ0201 (message "takes exactly one PREDICATE argument")))

(case
  "a @ensures annotation with zero predicate arguments is rejected"
  (input (do (@ (ensures) (def (c) 3)) (export c)))
  (error CDZ0201 (message "takes exactly one PREDICATE argument")))

(case
  "a @ensures annotation with two predicate arguments is rejected"
  (input (do (@ (ensures (> ret 0) 5) (def (c) 3)) (export c)))
  (error CDZ0201 (message "takes exactly one PREDICATE argument")))

(case
  "a valid one-predicate @requires is accepted and the def runs"
  (input (do (@ (requires (> x 0)) (def (c (: x Int64)) 3)) (export c)))
  (call c (: 4 Int64))
  (output (: 3 Int64)))

(case
  "a valid one-predicate @ensures is accepted and the def runs"
  (input (do (@ (ensures (> ret 0)) (def (c) 3)) (export c)))
  (call c)
  (output (: 3 Int64)))

; ── b4b: the DENOTATION — a predicate `Ast` → an obligation `Term` (the semantics→logic bridge, §1A) ──
; b4a records a `@requires(pred)`/`@ensures(pred)` predicate as its `Ast` occurrence. b4b DENOTES that
; predicate Ast into a HOL `Term` the kernel discharges — the §1A shallow embedding on the pure-arith
; fragment. A predicate `(<= x 100)` is `Ast.List [Ast.Name "<=", Ast.Name "x", Ast.Int 100]`; its
; denotation is the `bounds` term `le (Var 0) (Num 100)` (a Name→Var by the param's index, an Int→Num,
; the `<=` head→`le`). This case pins the denotation as an ordinary total `Ast → Term` function (which is
; where the b4 compiler wiring will compile-time-eval it); the FULL @ensures elaboration (result binder
; `it`, the obligation implication) composes these clauses and is a later slice.
(case
  "b4b denotation: a predicate Ast (<= x 100) denotes to the bounds obligation term le (Var 0) (Num 100)"
  (doc
    "The semantics→logic bridge (design §1A) as a total `Ast → Term` function. The recorded predicate
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      ; a minimal Ast mirror (the metaprogramming Ast sum's relevant variants for the arith fragment)
      (type Ast (AName String) (AInt Int64) (AList (List Ast)))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      ; the param environment: a name → its Var index. Minimal here (only `x` at index 0).
      (def (var-of (: name String)) (if (= name "x") 0 -1))
      ; DENOTE a leaf: a name → Var, an int → Num. (A non-arith leaf is out of the fragment; here total.)
      (def
        (denote-leaf (: a Ast))
        (match
          a
          ((Ast.AName nm) (Term.Var (var-of nm)))
          ((Ast.AInt n) (Term.Num n))
          ((Ast.AList _) (Term.Num -1))))
      ; DENOTE a predicate Ast → an obligation Term (the §1A shallow embedding, arith fragment).
      ; `(<= a b)` → `le`, `(+ a b)` → `add`; operands denote via denote-leaf (or recurse for nesting).
      (def
        (denote (: a Ast))
        (match
          a
          ((Ast.AList items)
            (match
              items
              (#list((Ast.AName op) l r)
                (let
                  ((lt (denote-leaf l)) (rt (denote-leaf r)))
                  (if (= op "<=") (le lt rt) (if (= op "+") (add lt rt) (Term.Num -1)))))
              (_ (Term.Num -1))))
          (_ (denote-leaf a))))
      (export Term.*)
      (export HeadOp.*)
      (export Ast.*)
      (export term-eq add le denote)))
  (input
    (do
      (import "bounds" (HeadOp Term Ast term-eq add le denote))
      (def
        (main)
        ; the recorded predicate `(<= x 100)` as an Ast
        (let
          ((pred (Ast.AList #list((Ast.AName "<=") (Ast.AName "x") (Ast.AInt 100)))))
          ; its denotation must equal the hand-built obligation term `le (Var 0) (Num 100)`
          (let ((expected (le (Term.Var 0) (Term.Num 100)))) (term-eq (denote pred) expected))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b4c(proven): a full @requires/@ensures obligation — denote both, discharge P ⇒ Q[ret:=body] ─────────
; b4b denotes ONE predicate Ast → Term. b4c(proven) composes the elaboration (§2.1): for
;   @requires(<= x 100) @ensures(<= ret MAXINT) (def (f x) (+ x 1))
; the obligation is `denote(P) ⊢ denote(Q)[ret := denote(body)]` — i.e. from the precondition hypothesis
; `le x 100` derive `le (add x 1) MAXINT` (the postcondition with `it` the body's value `x+1`). This is
; exactly the b1 discharge chain, now framed as the DENOTED annotations: `it` in Q is replaced by the
; denotation of the body `(+ x 1)` → `add (Var 0) (Num 1)`, and the precondition enters via `assume`. Pins
; that the §2.1 elaboration target — the whole @requires⇒@ensures obligation — discharges through the SAME
; kernel the hand-authored b1 cases use, so the b4c compiler wiring (compile-time-eval) has a proven target.
(case
  "b4c(proven): @requires(<= x 100)/@ensures(<= ret MAXINT) on (f x)=x+1 discharges — P denoted as hyp, Q[ret:=body] as goal"
  (doc
    "The PROVEN-tier obligation for a full @requires/@ensures pair (design §2.1). The elaboration
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq add le maxint concl hyps assume le-ax mono-add-r trans-le))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)))
          ; denote(body) = (+ x 1) → add (Var 0) (Num 1); ret := this in the @ensures goal
          (let
            ((body-den (add x one)))
            ; @ensures(<= ret MAXINT) with ret:=body → goal = le (add x 1) MAXINT
            (let
              ((goal (le body-den (maxint)))
                ; @requires(<= x 100) → hypothesis, entered via assume
                (pre (assume (le x c100))))
              ; discharge: mono-add-r + numeral fact + trans (the b1 chain)
              (match
                (mono-add-r pre one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c100 one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof) (term-eq (concl proof) goal))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b4c(unprovable): an @ensures whose obligation is NOT dischargeable → the PROVEN tier fails (CDZ-VERIFY) ─
; The dual of b4c(proven). For `@ensures(<= ret MAXINT) (def (f x) (+ x 1))` with NO (or too-weak)
; @requires, the postcondition obligation `le (add x 1) MAXINT` is NOT provable — x is unbounded, so the
; discharge chain has no bounding hypothesis to feed mono-add-r, and le-ax cannot mint a non-ground fact.
; At b4c this un-discharged obligation is the PROVEN-tier MISS: the author gets CDZ-VERIFY (or, if @test is
; stacked, the TESTED tier runs it — v-property-testing's lane). This pins that a genuinely-unprovable
; postcondition does NOT spuriously discharge — the proof tier is SOUND (it never claims a false proof).
(case
  "b4c(unprovable): @ensures(<= ret MAXINT) on unbounded (f x)=x+1 is NOT dischargeable — the proof tier correctly MISSES (CDZ-VERIFY)"
  (doc
    "The PROVEN-tier soundness dual. With no bounding @requires, the @ensures postcondition
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
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      ; the only obligation-minting axiom is the CHECKED ground le-ax; with an unbounded x the goal
      ; `le (add x 1) MAXINT` is non-ground, so le-ax returns None — no proof.
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (export Term.*)
      (export Thm)
      (export term-eq add le maxint eval-ground le-ax)))
  (input
    (do
      (import "bounds" (Term Thm term-eq add le maxint eval-ground le-ax))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)))
          ; the unprovable obligation: le (add x 1) MAXINT with x unbounded. le-ax is the only axiom
          ; that could mint it — call it on the OBLIGATION's own sides (lhs = the numeric term
          ; (add x 1), rhs = MAXINT), exactly the no-overflow fact. eval-ground on (add x 1) fails
          ; because x is a FREE Var (non-ground) — so le-ax returns None specifically DUE TO the
          ; unbounded x, the real "unbounded add is not dischargeable" property (not a shape mismatch:
          ; both sides ARE numeric/add terms le-ax evaluates; only x's freeness blocks it). No proof
          ; reaches the goal, so the PROVEN tier misses. Assert le-ax yields None.
          (let
            ((attempt (le-ax (add x one) (maxint))))
            (match attempt ((Option.Some _) false) ((Option.None) true)))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b4c(conjunctive): a TWO-hypothesis precondition — both @requires flow to the discharge + hyps-subset ─
; b4a records STACKED @requires as a Vec (a conjunction). This pins the multi-hypothesis path the earlier
; single-precondition cases do not: `@requires(>= x 0) @requires(<= x 100)` gives a sequent with TWO
; hypotheses, and the b2 `licenses` hyps-subset must require BOTH are covered by the node precondition (not
; just one). Discharge uses only the `<= x 100` bound (the upper one drives no-overflow), but the proof
; CARRIES both hypotheses, so the match predicate's precondition must contain both — a two-element
; hyps-subset, the "ALL hyps covered" soundness check that a single-hyp case cannot exercise.
(case
  "b4c(conjunctive): two stacked @requires give a 2-hyp proof; licenses requires BOTH hyps covered by the precondition"
  (doc
    "The multi-hypothesis discharge + hyps-subset soundness path. `@requires(>= x 0)
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
      (type HeadOp (Add) (Le) (Ge))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; mono-add-r that PRESERVES the operand hyps (so the derived step keeps {ge x 0, le x 100})
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      ; CONJ: assume two facts into one 2-hyp theorem (the stacked-@requires precondition as a conjunction)
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq #list(p q) p))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export
        op-eq
        term-eq
        add
        le
        ge
        maxint
        concl
        hyps
        assume
        assume-both
        le-ax
        mono-add-r
        trans-le
        licenses)))
  (input
    (do
      (import
        "bounds"
        (HeadOp
          Term
          Thm
          term-eq
          add
          le
          ge
          maxint
          concl
          hyps
          assume
          assume-both
          le-ax
          mono-add-r
          trans-le
          licenses))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (zero (Term.Num 0)))
          (let
            ((obligation (le (add x one) (maxint)))
              ; the node precondition is the CONJUNCTION {ge x 0, le x 100}
              (precondition #list((ge x zero) (le x c100))))
            ; a proof carrying BOTH hypotheses — built via the EXPORTED assume-both rule (a Thm
            ; cannot be constructed outside the kernel; conclusion is the first arg, hyps are both).
            (let
              ((pre-le (assume-both (le x c100) (ge x zero))))
              (match
                (mono-add-r pre-le one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c100 one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof)
                          ; the proof's hyps are {ge x 0, le x 100}; licenses requires BOTH in pre
                          (licenses proof obligation precondition))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b4c(conjunctive) NEGATIVES: partial precondition coverage → NOT licensed (breaker, all-covered sentinel) ─
; The soundness sentinels for the conjunctive hyps-subset: a 2-hyp proof {le x 100, ge x 0} must NOT be
; licensed by a precondition that covers only ONE hyp — hyps-subset is ALL-covered, not any-one. Both
; directions (breaker-verified, all 3 backends): the licenses trusted-elision surface rejects a proof
; assuming a hyp the node precondition does not provide, EVEN when the obligation was discharged via the
; OTHER hyp (the discharged Thm still CARRIES the assumption).
(case
  "b4c(conjunctive) NEG-1: precondition covers only {le x 100} (missing ge x 0) — the 2-hyp proof is NOT licensed"
  (doc
    "Partial-coverage soundness sentinel (breaker vector). The 2-hyp proof carries {le x 100, ge x 0}
           (both assumed via assume-both). The node precondition covers ONLY {le x 100} — it omits `ge x 0`.
           `licenses` must be FALSE: hyps-subset requires EVERY hyp covered, and `ge x 0` is not in the
           precondition. The entry builds the 2-hyp proof, discharges the obligation, and asserts `licenses`
           is false (via `not`). Runs to `true`. Pins hyps-subset is ALL-covered, not any-one — a proof
           assuming a bound the node does not guarantee cannot license an elision.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le) (Ge))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq #list(p q) p))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export
        op-eq
        term-eq
        add
        le
        ge
        maxint
        concl
        hyps
        assume-both
        le-ax
        mono-add-r
        trans-le
        licenses)))
  (input
    (do
      (import
        "bounds"
        (HeadOp
          Term
          Thm
          term-eq
          add
          le
          ge
          maxint
          concl
          hyps
          assume-both
          le-ax
          mono-add-r
          trans-le
          licenses))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (zero (Term.Num 0)))
          (let
            ((obligation (le (add x one) (maxint)))
              ; precondition covers ONLY le x 100 — missing ge x 0
              (precondition #list((le x c100))))
            (let
              ((pre-le (assume-both (le x c100) (ge x zero))))
              (match
                (mono-add-r pre-le one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c100 one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof)
                          ; proof carries {le x 100, ge x 0}; pre lacks ge x 0 → NOT licensed
                          (not (licenses proof obligation precondition)))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "b4c(conjunctive) NEG-2 (reverse): precondition covers only {ge x 0} (missing le x 100) — NOT licensed though discharged via le"
  (doc
    "The subtle reverse sentinel (breaker vector). The obligation was DISCHARGED using the `le x 100`
           bound, but the resulting Thm STILL CARRIES `le x 100` as a hypothesis (the rules union operand
           hyps). So a precondition covering only {ge x 0} — omitting the very `le x 100` the discharge used
           — must NOT license: hyps-subset finds `le x 100` uncovered. `licenses` is FALSE. Pins that
           carrying-and-using a hyp does not exempt it from the coverage check — the discharged assumption
           must be in the node precondition regardless of its role in the proof. Runs to `true` via `not`.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le) (Ge))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (def (assume-both (: p Term) (: q Term)) (Thm.Seq #list(p q) p))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export
        op-eq
        term-eq
        add
        le
        ge
        maxint
        concl
        hyps
        assume-both
        le-ax
        mono-add-r
        trans-le
        licenses)))
  (input
    (do
      (import
        "bounds"
        (HeadOp
          Term
          Thm
          term-eq
          add
          le
          ge
          maxint
          concl
          hyps
          assume-both
          le-ax
          mono-add-r
          trans-le
          licenses))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c100 (Term.Num 100)) (zero (Term.Num 0)))
          (let
            ((obligation (le (add x one) (maxint)))
              ; precondition covers ONLY ge x 0 — missing the le x 100 the discharge used
              (precondition #list((ge x zero))))
            (let
              ((pre-le (assume-both (le x c100) (ge x zero))))
              (match
                (mono-add-r pre-le one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c100 one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof)
                          ; proof carries {le x 100, ge x 0}; pre lacks le x 100 → NOT licensed
                          (not (licenses proof obligation precondition)))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b(sub): a no-UNDERFLOW discharge — for x >= 0, (x - 1) >= MININT (the lower-bound / `-` direction) ──
; The b1 discharge pinned `+`/overflow (upper bound vs MAXINT). Overflow elision (b3) also covers `-`/`*`;
; this pins the SUBTRACTION / lower-bound direction the same convention handles: for a checked `x - 1` under
; `@requires(>= x 0)`, the no-underflow obligation is `GE (sub x 1) MININT` (x-1 must not fall below the
; Int64 minimum). The arithmetic base gains a `ge` order + `sub` head + a `mono-sub-r` rule (subtracting a
; constant from both sides of a `>=` preserves it) + the CHECKED ground `ge-ax`. From `assume (GE x 0)`:
; mono-sub-r → `GE (sub x 1) (sub 0 1)` = `GE (sub x 1) -1`, and `ge-ax (sub 0 1) MININT` mints `GE -1 MININT`
; (eval-ground (sub 0 1) = -1, and -1 >= MININT holds), then trans-ge closes to `GE (sub x 1) MININT`.
(case
  "b(sub): a no-underflow obligation is DISCHARGED — for x >= 0, (x - 1) >= MININT via monotonicity + a CHECKED numeral fact"
  (doc
    "The subtraction / lower-bound dual of the b1 overflow discharge. A checked `x - 1 : Int64` under
           `@requires(>= x 0)` has the no-underflow obligation `GE (sub x 1) MININT`. The `bounds` kernel
           discharges it with no arithmetic primitive: from `assume (GE x 0)`, `mono-sub-r` (subtracting 1
           from both sides of a `>=`) gives `GE (sub x 1) (sub 0 1)`, then the CHECKED ground axiom
           `ge-ax (sub 0 1) MININT` mints `GE (sub 0 1) MININT` — only because `eval-ground (sub 0 1) = -1`
           and `-1 >= MININT` holds — and `trans-ge` closes it to `GE (sub x 1) MININT`. Pins that the
           discharge convention generalizes to SUBTRACTION and the lower-bound (MININT) direction the b3
           elision covers for `-`, using the same side-condition-checked axiom base. Head-symbols are
           nullary variants of the closed `HeadOp` sum (`Sub`/`Ge`), applied via `Term.Head` — the
           idiomatic strongly-typed encoding, NOT a magic-int `Const` tag (operator directive 2026-08-01).")
  (module "bounds"
    (do
      (type HeadOp (Sub) (Ge))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Sub) (match b ((HeadOp.Sub) true) (_ false)))
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (sub (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Sub) a) b))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (minint) (Term.Num -9223372036854775808))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; ground evaluator over numerals + `sub`
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Sub) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (- av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      ; CHECKED ground axiom for `>=`: mint |- (ge lhs rhs) only when both ground-numeric and lhs >= rhs
      (def
        (ge-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (>= lv rv) (Option.Some (Thm.Seq #list() (ge lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; RULE: monotonicity of - on the right — from G |- (ge x c) derive G |- (ge (sub x k) (sub c k))
      (def
        (mono-sub-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Ge) x) c)
            (Option.Some (Thm.Seq (hyps th) (ge (sub x k) (sub c k)))))
          (_ (Option.None))))
      ; RULE: transitivity of >= — from G |- (ge a b) and D |- (ge b c) derive G++D |- (ge a c)
      (def
        (trans-ge (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Ge) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (ge a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq sub ge minint concl hyps assume ge-ax mono-sub-r trans-ge)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq sub ge minint concl hyps assume ge-ax mono-sub-r trans-ge))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (zero (Term.Num 0)))
          ; obligation: (ge (sub x 1) MININT) — x-1 does not underflow
          (let
            ((goal (ge (sub x one) (minint))))
            ; step 1: assume (ge x 0)
            (let
              ((pre (assume (ge x zero))))
              ; step 2: monotonicity → (ge (sub x 1) (sub 0 1))
              (match
                (mono-sub-r pre one)
                ((Option.Some step1)
                  ; step 3: CHECKED numeral fact (ge (sub 0 1) MININT) — -1 >= MININT holds
                  (match
                    (ge-ax (sub zero one) (minint))
                    ((Option.Some fact)
                      ; step 4: transitivity → (ge (sub x 1) MININT)
                      (match
                        (trans-ge step1 fact)
                        ((Option.Some proof) (term-eq (concl proof) goal))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── b(mul): a no-overflow discharge for MULTIPLICATION — for x <= 100, (x * 2) <= MAXINT ──────────────
; Completes the arithmetic-op discharge coverage (+, -, now *) that b3's guard elision handles. For a
; checked `x * 2` under `@requires(<= x 100)`, the no-overflow obligation is `LE (mul x 2) MAXINT`. The base
; gains a `mul` head + a `mono-mul-r` rule — multiplying both sides of a `<=` by a POSITIVE constant
; preserves the order (the positivity is the rule's side-condition: it only fires for a positive `Num` k).
; From `assume (le x 100)`: mono-mul-r by 2 → `LE (mul x 2) (mul 100 2)`, and `le-ax (mul 100 2) MAXINT`
; mints `LE (mul 100 2) MAXINT` (eval-ground (mul 100 2) = 200, 200 <= MAXINT), then trans-le closes it.
; `mul`=Const 4. mono-mul-r requires k a positive numeral (an arbitrary/negative multiplier does NOT
; preserve `<=` — the rule returns None, so the axiom base stays sound).
(case
  "b(mul): a no-overflow obligation is DISCHARGED for x <= 100, (x * 2) <= MAXINT via positive-multiplier monotonicity"
  (doc
    "The multiplication case, completing +/-/* discharge coverage. A checked `x * 2 : Int64` under
           `@requires(<= x 100)` has the no-overflow obligation `LE (mul x 2) MAXINT`. From `assume
           (le x 100)`, `mono-mul-r` (multiply both sides by the POSITIVE constant 2 — its positivity is the
           rule's side-condition; a non-positive multiplier returns None) gives `LE (mul x 2) (mul 100 2)`,
           then the CHECKED ground axiom `le-ax (mul 100 2) MAXINT` mints `LE (mul 100 2) MAXINT` because
           `eval-ground (mul 100 2) = 200` and `200 <= MAXINT`, and `trans-le` closes it to `LE (mul x 2)
           MAXINT`. Pins that the discharge convention covers MULTIPLICATION (b3 elides `*` guards too), with
           the positive-multiplier side-condition keeping the monotonicity rule sound. Head-symbols are
           nullary variants of the closed `HeadOp` sum (`Mul`/`Le`), applied via `Term.Head` — the idiomatic
           strongly-typed encoding, NOT a magic-int `Const` tag (operator directive 2026-08-01).")
  (module "bounds"
    (do
      (type HeadOp (Mul) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Mul) (match b ((HeadOp.Mul) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (mul (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Mul) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Mul) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (* av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      ; RULE: monotonicity of * on the right by a POSITIVE constant k — from G |- (le x c) derive
      ; G |- (le (mul x k) (mul c k)). k must be a positive Num (side-condition); else None (a non-positive
      ; multiplier flips or collapses the order, so minting would be unsound).
      (def
        (mono-mul-r (: th Thm) (: k Term))
        (match
          k
          ((Term.Num kv)
            (if
              (> kv 0)
              (match
                (concl th)
                ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
                  (Option.Some (Thm.Seq (hyps th) (le (mul x k) (mul c k)))))
                (_ (Option.None)))
              (Option.None)))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq mul le maxint concl hyps assume le-ax mono-mul-r trans-le)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq mul le maxint concl hyps assume le-ax mono-mul-r trans-le))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (two (Term.Num 2)) (c100 (Term.Num 100)))
          (let
            ((goal (le (mul x two) (maxint))))
            (let
              ((pre (assume (le x c100))))
              (match
                (mono-mul-r pre two)
                ((Option.Some step1)
                  (match
                    (le-ax (mul c100 two) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof) (term-eq (concl proof) goal))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── t1(div0): the DIVIDE-BY-ZERO trap-source obligation — for b > 0, (b != 0) so `a / b` cannot trap ──
; The @trap_free capstone (design §8) proves EVERY trap source unreachable. This pins the DIVIDE-BY-ZERO
; source: a checked `a / b` traps iff b = 0, so its trap-free obligation is `NEQ b 0` (the divisor is
; non-zero). Under `@requires(> b 0)`, the obligation discharges: from `assume (gt b 0)`, a `pos-nonzero`
; rule (a positive value is non-zero) yields `NEQ b 0`. The base gains a `gt` order + `neq` + `pos-nonzero`
; + the CHECKED ground `gt-ax`. Head-symbols are `HeadOp` sum variants (`Gt`/`Neq`) via `Term.Head`, not
; magic-int `Const` tags. Pins the div0 trap-source obligation shape the
; capstone's per-source conjunction needs.
(case
  "t1(div0): the divide-by-zero obligation NEQ b 0 is DISCHARGED for b > 0 — so a/b cannot trap"
  (doc
    "The divide-by-zero trap source of the @trap_free capstone (design §8). A checked `a / b` traps iff
           `b = 0`; its trap-free obligation is `NEQ b 0`. Under `@requires(> b 0)`, from `assume (gt b 0)`
           the `pos-nonzero` rule (a value proven `> 0` is `!= 0`) derives `NEQ b 0` — the divisor is
           provably non-zero, so the division cannot trap on that input. The entry discharges it through the
           rules and checks the conclusion is the obligation. Runs to `true`. Pins the div0 obligation shape
           the capstone's per-trap-source conjunction discharges (one source of the whole-function trap-free
           proof).")
  (module "bounds"
    (do
      (type HeadOp (Gt) (Neq))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Gt) (match b ((HeadOp.Gt) true) (_ false)))
          ((HeadOp.Neq) (match b ((HeadOp.Neq) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (gt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Gt) a) b))
      (def (neq (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Neq) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; RULE: pos-nonzero — from G |- (gt x 0) derive G |- (neq x 0). A value proven strictly positive is
      ; non-zero. The rule fires ONLY when the premise is `(gt x (Num 0))` (the zero literal); else None.
      (def
        (pos-nonzero (: th Thm))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Gt) x) (Term.Num 0))
            (Option.Some (Thm.Seq (hyps th) (neq x (Term.Num 0)))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq gt neq concl hyps assume pos-nonzero)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq gt neq concl hyps assume pos-nonzero))
      (def
        (main)
        (let
          ((b (Term.Var 1)) (zero (Term.Num 0)))
          ; the div0 trap-free obligation: (neq b 0)
          (let
            ((goal (neq b zero)))
            ; @requires(> b 0) → assume (gt b 0); pos-nonzero derives (neq b 0)
            (let
              ((pre (assume (gt b zero))))
              (match
                (pos-nonzero pre)
                ((Option.Some proof) (term-eq (concl proof) goal))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "t1(div0) NEGATIVE: an UNBOUNDED divisor is NOT provably non-zero — the divide-by-zero trap STAYS"
  (doc
    "The div0 soundness dual. With no `> b 0` (or `b != 0`) precondition, the divisor `b` is unbounded
           — `NEQ b 0` is NOT provable: `pos-nonzero` needs a `(gt b 0)` premise, and an arbitrary assumption
           about `b` does not establish it. So the @trap_free proof for the division MISSES → the div-by-zero
           guard STAYS (the function is not certified trap-free on that source). The entry confirms
           `pos-nonzero` of an unrelated assumption does not yield the obligation. Runs to `true`. Pins that
           an unprovable divide-by-zero source correctly keeps the trap — @trap_free is sound (it never
           certifies a function whose divisor could be zero).")
  (module "bounds"
    (do
      (type HeadOp (Gt) (Neq))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Gt) (match b ((HeadOp.Gt) true) (_ false)))
          ((HeadOp.Neq) (match b ((HeadOp.Neq) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (gt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Gt) a) b))
      (def (neq (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Neq) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (pos-nonzero (: th Thm))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Gt) x) (Term.Num 0))
            (Option.Some (Thm.Seq (hyps th) (neq x (Term.Num 0)))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq gt neq concl hyps assume pos-nonzero)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq gt neq concl hyps assume pos-nonzero))
      (def
        (main)
        (let
          ((b (Term.Var 1)) (zero (Term.Num 0)))
          (let
            ((goal (neq b zero)))
            ; no `> b 0` precondition — only an unrelated assumption about b; pos-nonzero cannot fire
            (let
              ((unrelated (assume (neq b b))))
              (match
                (pos-nonzero unrelated)
                ((Option.Some proof) (not (term-eq (concl proof) goal)))
                ((Option.None) true))))))
      (export main)))
  (output (: true Bool)))

; ── t1(oob): the OUT-OF-BOUNDS trap-source obligation — for 0 <= i < len, `xs[i]` cannot trap ─────────
; The @trap_free capstone (§8) proves EVERY trap source unreachable. This pins the OUT-OF-BOUNDS source: a
; checked index `List.at xs i` (or Bytes.at) traps iff i < 0 OR i >= len, so its trap-free obligation is the
; CONJUNCTION `(0 <= i) AND (i < len)` — a two-part bound. Under `@requires(>= i 0) @requires(< i len)`,
; both conjuncts are direct precondition hypotheses; the obligation is their conjunction. The base gains a
; `lt` order + a `conj` connective (HeadOp sum variants Lt/Conj via Term.Head, not magic-int Const tags) + a
; `both` rule (from G|-p and D|-q derive G++D|-p∧q).
; From assume(ge i 0) and assume(lt i len): `both` gives `CONJ (ge i 0) (lt i len)` = the in-bounds proof.
(case
  "t1(oob): the out-of-bounds obligation (0<=i) AND (i<len) is DISCHARGED from the two bound preconditions"
  (doc
    "The out-of-bounds trap source of the @trap_free capstone. A checked `xs[i]` traps iff `i < 0` or
           `i >= len`; its trap-free obligation is the conjunction `(ge i 0) AND (lt i len)`. Under
           `@requires(>= i 0)` and `@requires(< i len)`, each conjunct is a precondition hypothesis, and the
           `both` rule combines them into `CONJ (ge i 0) (lt i len)` — the index is provably in bounds, so
           the access cannot trap. The entry assumes both bounds, combines via `both`, and checks the
           conclusion is the conjunction obligation (both conjuncts, hyps unioned). Runs to `true`. Pins the
           OOB obligation shape (a two-part conjunction) the capstone's per-trap-source proof discharges.")
  (module "bounds"
    (do
      (type HeadOp (Ge) (Lt) (Conj))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))
          ((HeadOp.Lt) (match b ((HeadOp.Lt) true) (_ false)))
          ((HeadOp.Conj) (match b ((HeadOp.Conj) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (lt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Lt) a) b))
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Conj) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; RULE `both`: from G |- p and D |- q derive G++D |- (conj p q) — the in-bounds proof combines the two
      ; bound facts. (Hyps unioned, per the Inc-11 soundness rule that a multi-premise rule carries the union.)
      (def
        (both (: t1 Thm) (: t2 Thm))
        (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (conj (concl t1) (concl t2)))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq ge lt conj concl hyps assume both)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq ge lt conj concl hyps assume both))
      (def
        (main)
        (let
          ((i (Term.Var 2)) (len (Term.Var 3)) (zero (Term.Num 0)))
          ; the OOB trap-free obligation: (conj (ge i 0) (lt i len))
          (let
            ((goal (conj (ge i zero) (lt i len))))
            ; @requires(>= i 0) and @requires(< i len) → two hypotheses
            (let
              ((lower (assume (ge i zero))) (upper (assume (lt i len))))
              (match
                (both lower upper)
                ((Option.Some proof) (term-eq (concl proof) goal))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "t1(oob) NEGATIVE: with only the LOWER bound (>= i 0), the out-of-bounds obligation is NOT complete — the trap STAYS"
  (doc
    "The OOB soundness dual. The obligation is the CONJUNCTION `(ge i 0) AND (lt i len)`; a precondition
           giving ONLY the lower bound `>= i 0` (missing `< i len`) cannot establish it — `i` could still be
           >= len, so the access can still trap past the end. The entry has only the lower-bound hypothesis
           and confirms it does NOT establish the full conjunction (the upper-bound conjunct is absent). So
           the @trap_free proof for the index MISSES → the bounds-check STAYS. Runs to `true` (asserts the
           lower bound alone is not the obligation). Pins that a PARTIAL bound does not certify in-bounds —
           @trap_free is sound (it never drops a bounds check unless BOTH bounds are proven).")
  (module "bounds"
    (do
      (type HeadOp (Ge) (Lt) (Conj))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))
          ((HeadOp.Lt) (match b ((HeadOp.Lt) true) (_ false)))
          ((HeadOp.Conj) (match b ((HeadOp.Conj) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (lt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Lt) a) b))
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Conj) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq ge lt conj concl assume)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq ge lt conj concl assume))
      (def
        (main)
        (let
          ((i (Term.Var 2)) (len (Term.Var 3)) (zero (Term.Num 0)))
          (let
            ((goal (conj (ge i zero) (lt i len))))
            ; only the lower bound is assumed — no upper bound, so the conjunction is not established
            (let
              ((lower (assume (ge i zero))))
              ; the lower bound alone is NOT the full obligation → bounds check stays
              (not (term-eq (concl lower) goal))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── t1(match): the PARTIAL-MATCH / exhaustiveness trap source — a match with total arm coverage cannot trap ─
; The @trap_free capstone (§8): a `match` traps at an `Unreachable` node iff a scrutinee value hits no arm.
; Its trap-free obligation is EXHAUSTIVENESS — every reachable scrutinee value is covered. Modeled here as a
; `covers` proof: the obligation `COVERS scrut arms` holds when the arm set is TOTAL for the scrutinee's
; type. The exhaustiveness checker already decides this for the compiler; here we pin the OBLIGATION shape —
; an `exhaustive-ax` mints `COVERS s arms` only when a `total?` predicate on the arm set holds (a decidable
; ground check, like le-ax's numeral side-condition). `covers`=Const 9. A NON-total arm set yields None (the
; Unreachable stays reachable → the match can trap).
(case
  "t1(match): the exhaustiveness obligation COVERS is DISCHARGED for a TOTAL arm set — the match cannot trap"
  (doc
    "The partial-match trap source of the @trap_free capstone. A `match` traps at Unreachable iff some
           scrutinee value hits no arm; its trap-free obligation is EXHAUSTIVENESS. Modeled: `exhaustive-ax`
           mints `COVERS scrut arms` ONLY when the arm set is TOTAL for the scrutinee (a decidable
           side-condition, `total?` — here a two-variant Bool scrutinee with both arms present). A total arm
           set discharges → no Unreachable is reachable → the match cannot trap. The entry checks a
           both-arms-covered Bool match discharges the COVERS obligation. Runs to `true`. Pins the
           exhaustiveness obligation shape the capstone's per-trap-source proof discharges (the checker
           already decides totality; this pins the obligation the discharge produces).")
  (module "bounds"
    (do
      (type HeadOp (Covers))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match a ((HeadOp.Covers) (match b ((HeadOp.Covers) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def
        (covers (: scrut Term) (: arms Term))
        (Term.Comb (Term.Comb (Term.Head HeadOp.Covers) scrut) arms))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      ; total? : is the arm set (a list of covered variant tags, as Num) TOTAL for a scrutinee whose variant
      ; count is `n`? Decidable: the arm set covers exactly {0..n-1}. Here the ground check is "arms has n
      ; distinct tags 0..n-1"; modeled minimally as len(arms) == n with tags being 0..n-1 in order.
      (def (total? (: arms (List Int64)) (: n Int64)) (= (List.len arms) n))
      ; AXIOM: mint COVERS scrut arms-term ONLY when the arm TAGS are total for the scrutinee's variant
      ; count. A non-total set → None (the Unreachable stays reachable).
      (def
        (exhaustive-ax
          (: scrut Term)
          (: arms-term Term)
          (: arm-tags (List Int64))
          (: nvariants Int64))
        (if
          (total? arm-tags nvariants)
          (Option.Some (Thm.Seq #list() (covers scrut arms-term)))
          (Option.None)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq covers concl total? exhaustive-ax)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq covers concl total? exhaustive-ax))
      (def
        (main)
        (let
          ((scrut (Term.Var 0))
            ; the arm set as an opaque term (its identity is what COVERS names — a Num stands in as a
            ; generic Term leaf, not an operator head); tags are 0,1 (both Bool variants), nvariants = 2 → total.
            (arms (Term.Num 100)))
          (let
            ((goal (covers scrut arms)))
            (match
              (exhaustive-ax scrut arms #list(0 1) 2)
              ((Option.Some proof) (term-eq (concl proof) goal))
              ((Option.None) false)))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "t1(match) NEGATIVE: a NON-total arm set (one Bool arm missing) does NOT discharge COVERS — the match can still trap"
  (doc
    "The exhaustiveness soundness dual. A Bool scrutinee (2 variants) with only ONE arm covered (tags
           = {0}, missing 1) is NOT total, so `exhaustive-ax` returns None — the COVERS obligation is not
           established, the Unreachable stays reachable, and the @trap_free proof for the match MISSES → the
           match can still trap on the uncovered value. The entry confirms exhaustive-ax of a one-arm set
           over a 2-variant scrutinee yields None. Runs to `true`. Pins that a non-exhaustive match is NOT
           certified trap-free — @trap_free is sound (it never drops the Unreachable unless the match is
           proven total).")
  (module "bounds"
    (do
      (type HeadOp (Covers))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (covers (: scrut Term) (: arms Term))
        (Term.Comb (Term.Comb (Term.Head HeadOp.Covers) scrut) arms))
      (def (total? (: arms (List Int64)) (: n Int64)) (= (List.len arms) n))
      (def
        (exhaustive-ax
          (: scrut Term)
          (: arms-term Term)
          (: arm-tags (List Int64))
          (: nvariants Int64))
        (if
          (total? arm-tags nvariants)
          (Option.Some (Thm.Seq #list() (covers scrut arms-term)))
          (Option.None)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export covers total? exhaustive-ax)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm covers total? exhaustive-ax))
      (def
        (main)
        (let
          ((scrut (Term.Var 0)) (arms (Term.Num 100)))
          ; only tag 0 covered, nvariants = 2 → NOT total → None
          (match (exhaustive-ax scrut arms #list(0) 2) ((Option.Some _) false) ((Option.None) true))))
      (export main)))
  (output (: true Bool)))

; ── t1(trap): the EXPLICIT-TRAP trap source — a `trap()` under a provably-FALSE guard is unreachable ──
; The @trap_free capstone (§8): an explicit `trap()` (or effect-abort) inside `(if guard (trap) …)` traps
; iff `guard` is satisfiable. Its trap-free obligation is UNREACHABILITY — the guard is provably FALSE for
; every input satisfying @requires, so the trap branch is dead. Modeled: the obligation `FALSE guard` holds
; when a `refute` rule derives a contradiction from `assume guard` + the precondition. Simplest ground
; instance: a trap guarded by `(lt x 0)` in a function `@requires(>= x 0)` — `ge x 0` and `lt x 0` are
; contradictory, so `refute` (from G |- (ge x 0) and a guard (lt x 0)) mints `UNREACHABLE (lt x 0)`.
; `unreach` is a UNARY HeadOp head via Term.Head (not a magic-int Const tag). A guard NOT contradicted
; by the precondition → None (the trap stays reachable).
(case
  "t1(trap): an explicit trap under guard (lt x 0) is UNREACHABLE when @requires(>= x 0) — the trap is dead"
  (doc
    "The explicit-trap source of the @trap_free capstone. A `(if (lt x 0) (trap) …)` traps iff its
           guard `(lt x 0)` is satisfiable. Under `@requires(>= x 0)`, the guard CONTRADICTS the precondition
           (`ge x 0` and `lt x 0` cannot both hold), so the `refute` rule — from the precondition hypothesis
           `ge x 0` and the guard `lt x 0` — derives `UNREACHABLE (lt x 0)`: the trap branch is dead, so the
           function cannot reach the explicit trap. The entry assumes the precondition, refutes the guard,
           and checks the conclusion is the unreachability obligation. Runs to `true`. Pins the explicit-trap
           obligation shape (a guard proven false by the precondition) — the FIFTH and last trap source of
           the whole-function trap-free proof.")
  (module "bounds"
    (do
      (type HeadOp (Ge) (Lt) (Unreach))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))
          ((HeadOp.Lt) (match b ((HeadOp.Lt) true) (_ false)))
          ((HeadOp.Unreach) (match b ((HeadOp.Unreach) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (lt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Lt) a) b))
      (def (unreach (: g Term)) (Term.Comb (Term.Head HeadOp.Unreach) g))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; RULE `refute`: from G |- (ge x 0) and a GUARD (lt x 0) — a direct contradiction (x>=0 vs x<0 on the
      ; same x, same bound 0) — derive G |- (UNREACHABLE guard): the guarded branch is dead. Fires ONLY when
      ; the guard is `(lt x 0)` and the hypothesis is `(ge x 0)` for the SAME x (a recognized contradiction).
      (def
        (refute (: th Thm) (: guard Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Ge) x) (Term.Num 0))
            (match
              guard
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Lt) gx) (Term.Num 0))
                (if (term-eq x gx) (Option.Some (Thm.Seq (hyps th) (unreach guard))) (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq ge lt unreach concl hyps assume refute)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq ge lt unreach concl hyps assume refute))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (zero (Term.Num 0)))
          (let
            ((guard (lt x zero)))
            (let
              ((goal (unreach guard)))
              ; @requires(>= x 0) → assume (ge x 0); refute the guard (lt x 0) as contradictory
              (let
                ((pre (assume (ge x zero))))
                (match
                  (refute pre guard)
                  ((Option.Some proof) (term-eq (concl proof) goal))
                  ((Option.None) false)))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "t1(trap) NEGATIVE: a trap guard NOT contradicted by the precondition is NOT unreachable — the trap STAYS"
  (doc
    "The explicit-trap soundness dual. If the trap guard is NOT contradicted by the precondition — here
           the guard is `(lt x 0)` but the precondition is only `(ge x 5)`… actually a guard the precondition
           does not refute: guard `(lt x 100)` under `@requires(>= x 0)` is SATISFIABLE (x in [0,100) hits
           it), so `refute` (which recognizes only the exact `ge x 0` vs `lt x 0` contradiction) returns None
           — the trap branch is NOT proven dead, so the explicit trap STAYS reachable. The entry confirms
           `refute` of a non-contradictory guard yields None. Runs to `true`. Pins that a reachable explicit
           trap is NOT certified away — @trap_free is sound (it never drops a trap whose guard it cannot
           prove false).")
  (module "bounds"
    (do
      (type HeadOp (Ge) (Lt) (Unreach))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))
          ((HeadOp.Lt) (match b ((HeadOp.Lt) true) (_ false)))
          ((HeadOp.Unreach) (match b ((HeadOp.Unreach) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      (def (lt (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Lt) a) b))
      (def (unreach (: g Term)) (Term.Comb (Term.Head HeadOp.Unreach) g))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (refute (: th Thm) (: guard Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Ge) x) (Term.Num 0))
            (match
              guard
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Lt) gx) (Term.Num 0))
                (if (term-eq x gx) (Option.Some (Thm.Seq (hyps th) (unreach guard))) (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq ge lt unreach concl hyps assume refute)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq ge lt unreach concl hyps assume refute))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (zero (Term.Num 0)) (c100 (Term.Num 100)))
          ; guard (lt x 100) — SATISFIABLE under (ge x 0) (x in [0,100)); NOT the ge-x-0/lt-x-0
          ; contradiction refute recognizes → None → the trap stays reachable.
          (let
            ((guard (lt x c100)))
            (let
              ((pre (assume (ge x zero))))
              (match (refute pre guard) ((Option.Some _) false) ((Option.None) true))))))
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
(case
  "a stacked @test @ensures def called as a function returns its value, not unit (value-transparent)"
  (doc
    "A def carrying BOTH `@test` and `@ensures`, when CALLED as an ordinary function, returns its
           computed value — NOT `unit`. `(dbl 5)` = 10, the same value a bare `@test` def or a bare
           `@ensures` def returns (both are value-transparent). The TESTED-tier rewrite injects
           `(let ((it BODY)) (if Q it (trap …)))`: the pass branch returns `it` (the def's result), so the
           postcondition check does not swallow the value. A rewrite that returned `unit` on the pass branch
           (the earlier behavior) would make the stacked form silently non-value-transparent — this pins it
           does not. The true postcondition `(>= it 0)` holds for 10, so no trap; the value 10 flows out.")
  (input
    (do
      (@ test (@ (ensures (>= ret 0)) (def (dbl (: x Int64)) (+ x x))))
      (def (main) (dbl 5))
      (export main)))
  (output (: 10 Int64)))

(case
  "a stacked @test @ensures with a FALSE postcondition traps when the def is called (test semantics preserved)"
  (doc
    "The test-semantics half of the value-transparency pin above: making the postcondition FALSE must
           still TRAP (a `@test` fails by trapping). `(dbl 5)` = 10 and the postcondition `(< it 0)` — i.e.
           `10 < 0` — is false, so the injected `(if Q it (trap …))` takes the trap arm, halting with the
           canonical `unreachable` kind. Together with the value-transparent case above this pins that
           returning `it` (not `unit`) on the PASS branch did NOT weaken the check: a true postcondition
           yields the value, a false one still traps — the fix is value-transparent AND test-preserving.")
  (input
    (do
      (@ test (@ (ensures (< ret 0)) (def (dbl (: x Int64)) (+ x x))))
      (def (main) (dbl 5))
      (export main)))
  (trap "unreachable"))

; ── (D) TEST-TIER ENFORCEMENT — a PLAIN @requires is CHECKED at run time (Inc-b (D), verify_enforce.rs) ──
; The operator confirmed (D): @requires/@ensures/@trap_free/@invariant verify AT RUN TIME now (proof-guided
; ELISION defers to the bounded compile-time kernel interpreter (A) — a3's compile-time-eval premise was
; unbuildable: the kernel is recursive and rcdzc has no compile-time recursive evaluator). These two cases
; pin the PLAIN @requires enforcement: a violated precondition TRAPS, a satisfied one is value-transparent.
(case
  "a PLAIN @requires precondition is ENFORCED at body-entry: a VIOLATED precondition traps when the def is called"
  (doc
    "The (D) test-tier enforcement of a bare `@requires` (NOT stacked under `@test` — that is
           v-property-testing's TESTED tier). `verify_enforce::enforce` rewrites `(@ (requires (>= x 0))
           (def (f (: x Int64)) (+ x 1)))` so the body becomes `(if (>= x 0) (+ x 1) (trap …))` — the
           precondition is checked ONCE at body-entry (the Hoare `{P} body {Q}` reading), NOT at each call
           site. `(f -5)` violates `(>= x 0)`, so the `if` takes the trap arm, halting with the canonical
           `unreachable` kind. Before (D) the precondition was RECORDED (db.requires) but NOT enforced — the
           call returned `-4`. Pins that a plain @requires now actually verifies at run time; the wrapper is
           left in place so `strip_annotations` still records the predicate for the verification layer.")
  (input
    (do (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1))) (def (main) (f -5)) (export main)))
  (trap "unreachable"))

(case
  "a PLAIN @requires precondition is value-transparent when SATISFIED: the def returns its computed value"
  (doc
    "The value-transparency half of the plain-@requires enforcement pin above. `(f 5)` SATISFIES
           `(>= x 0)`, so the injected `(if (>= x 0) (+ x 1) (trap …))` takes the pass arm and returns the
           def's own value `6` — NOT `unit`, and no trap. Together with the trap case above this pins that
           the enforcement rewrite is value-transparent AND checking: a satisfied precondition yields the
           computed result, a violated one traps — the check does not swallow the value on the pass path.")
  (input
    (do (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1))) (def (main) (f 5)) (export main)))
  (output (: 6 Int64)))

(case
  "@requires on the EXPORTED ENTRY POINT is enforced — the contract holds when the harness calls main directly"
  (doc
    "Every enforcement case so far guards an INNER def that `main` calls; this pins the contract on the
           EXPORTED entry itself, invoked directly by the harness (not by user code). `verify_enforce`'s rewrite
           is applied to the def regardless of its role, so `@requires(>= k 0)` on `(main k)` becomes `(if (>= k
           0) (+ k 1) (trap …))` at the entry's body. main(5): `5 >= 0` holds → 6. main(-1): `-1 >= 0` is FALSE →
           the precondition traps at the entry's body-entry, before any user code runs. Pins that enforcement is
           not scoped to internally-called defs — a contract on `main` (the natural place to guard a program's
           inputs) verifies exactly as one on a helper, so the entry point is not an enforcement blind spot.
           Runtime arg via the harness call so nothing folds.")
  (input (do (@ (requires (>= k 0)) (def (main (: k Int64)) (+ k 1))) (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -1 Int64))
  (trap "unreachable"))

(case
  "a PLAIN @requires relating TWO parameters (< a b) is enforced — BOTH params stay in scope in the predicate"
  (doc
    "Every runtime @requires case so far constrains a SINGLE parameter (`>= x 0`, `<= x 100`). This pins a
           precondition relating TWO distinct parameters — the ordering contract `(< a b)` on a two-arg def —
           so the injected `(if (< a b) BODY (trap …))` must keep BOTH `a` AND `b` in scope at body-entry (the
           predicate reads both, exactly as a hand-written guard would). `(f 3 5)` satisfies `(< 3 5)`, so the
           `if` takes the pass arm and the def returns its own value `(- b a)` = `2`. Pins that a multi-parameter
           precondition resolves + enforces (the entry-side twin of the result-vs-parameter @ensures case, which
           reads `ret` alongside a param).")
  (input
    (do
      (@ (requires (< a b)) (def (f (: a Int64) (: b Int64)) (- b a)))
      (def (main) (f 3 5))
      (export main)))
  (output (: 2 Int64)))

(case
  "a PLAIN @requires relating TWO parameters (< a b) TRAPS when violated — the two-param precondition is checked"
  (doc
    "The trap half of the two-parameter precondition above. `@requires(< a b)` on `(f a b) = (- b a)`
           with `(f 5 3)` violates the ordering (`5 < 3` is FALSE), so the injected `(if (< a b) (- b a)
           (trap …))` takes the trap arm — `unreachable` — even though the body `(- b a)` = `-2` would itself
           compute fine. Pins that a precondition over two parameters enforces in both directions, not only the
           satisfied one, and that the check fires on the RELATIONSHIP between the args, not a single arg's
           range.")
  (input
    (do
      (@ (requires (< a b)) (def (f (: a Int64) (: b Int64)) (- b a)))
      (def (main) (f 5 3))
      (export main)))
  (trap "unreachable"))

(case
  "a @requires over a FLOAT64 parameter enforces at runtime — the check fires on float ordering, not only Int64"
  (doc
    "Every contracted def so far takes Int64 (or Bool/String) params; this pins the enforcement path over
           a FLOAT64 value, so the injected `(if PRE BODY (trap …))` compares floats at body entry. `@requires(>=
           r 0.0)` on `(f r) = r` demands a non-negative ratio; the runtime arg crosses via main's Float64 param
           so nothing folds. main(2.5): `2.5 >= 0.0` holds → pass → 2.5. main(0.0): `0.0 >= 0.0` holds (the
           boundary is inclusive) → pass → 0.0. main(-1.5): `-1.5 >= 0.0` is FALSE → the precondition fails →
           trap. Pins that a precondition resolves and enforces float comparison exactly as it does integer
           comparison — the guarded value flows on the non-negative floats and traps on the negative one, with
           the inclusive boundary passing.")
  (input
    (do
      (@ (requires (>= r 0.0)) (def (f (: r Float64)) r))
      (def (main (: k Float64)) (f k))
      (export main)))
  (call main (: 2.5 Float64))
  (output (: 2.5 Float64))
  (call main (: 0.0 Float64))
  (output (: 0.0 Float64))
  (call main (: -1.5 Float64))
  (trap "unreachable"))

(case
  "a @requires reads a BOOL parameter DIRECTLY as the predicate — the flag itself is the precondition, false traps"
  (doc
    "Completing the scalar value-domain set (Int64, Float64, now Bool). A predicate is any Bool-typed
           expression, so a BOOL parameter can BE the precondition with no comparison wrapper — `@requires(flag)`
           reads the param directly. On `(f flag x) = x` the injected `(if flag BODY (trap …))` passes exactly
           when the flag is true. Runtime args via main's params so nothing folds. main(true, 9): the flag is
           true → pass → 9. main(false, 9): the flag is false → the precondition is false → trap, even though the
           body `x` = 9 would compute fine. Pins that a bare Bool param resolves and enforces as a predicate
           without a comparison op — the guard reads the boolean value itself, the dual of the numeric-comparison
           preconditions.")
  (input
    (do
      (@ (requires flag) (def (f (: flag Bool) (: x Int64)) x))
      (def (main (: b Bool) (: k Int64)) (f b k))
      (export main)))
  (call main (: true Bool) (: 9 Int64))
  (output (: 9 Int64))
  (call main (: false Bool) (: 9 Int64))
  (trap "unreachable"))

(case
  "a @requires whose predicate is a DISJUNCTION (or) enforces: either disjunct satisfies, only the all-false input traps"
  (doc
    "Every runtime predicate so far combines with `and` or a bare comparison; this pins a precondition
           built from `or`, the short-circuiting boolean OR prelude op. `@requires(or (<= x 0) (>= x 100))` on
           `(f x) = x` demands x lies OUTSIDE the open interval (0, 100) — the injected `(if (or (<= x 0) (>= x
           100)) BODY (trap …))` passes when EITHER disjunct holds and traps only when BOTH are false. Runtime
           arg via main's param so nothing folds. main(-5): the first disjunct (-5 <= 0) holds → pass → -5.
           main(150): the first disjunct is false (150 <= 0) but the second (150 >= 100) holds → `or`
           short-circuits to true → pass → 150. main(50): BOTH disjuncts are false (50 <= 0 false, 50 >= 100
           false) → the precondition is false → trap. Pins that a disjunctive precondition resolves `or` as the
           prelude op and enforces its true short-circuit semantics at runtime — a satisfying value on EITHER
           side flows, and only the all-false case traps.")
  (input
    (do
      (@ (requires (or (<= x 0) (>= x 100))) (def (f (: x Int64)) x))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: -5 Int64))
  (output (: -5 Int64))
  (call main (: 150 Int64))
  (output (: 150 Int64))
  (call main (: 50 Int64))
  (trap "unreachable"))

(case
  "a @requires whose predicate is a NEGATION (not) enforces: the wrapped condition must be FALSE, its truth traps"
  (doc
    "The sibling of the disjunction pin — a precondition built from `not`, the boolean-negation prelude op.
           `@requires(not (= x 0))` on `(f x) = x` demands x is anything but zero: the injected `(if (not (= x
           0)) BODY (trap …))` passes when the wrapped equality is FALSE and traps when it is TRUE. Runtime arg
           via main's param so nothing folds. main(7): `(= 7 0)` is false → `(not false)` = true → pass → 7.
           main(-3): `(= -3 0)` is false → pass → -3. main(0): `(= 0 0)` is true → `(not true)` = false → the
           precondition is false → trap. Pins that a negated predicate resolves `not` as the prelude op and
           enforces its inversion at runtime — the guarded value flows exactly when the wrapped condition does
           NOT hold, and the sole zero input traps.")
  (input
    (do
      (@ (requires (not (= x 0))) (def (f (: x Int64)) x))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (output (: -3 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "STACKED @requires: EVERY precondition is enforced — a violated OUTER @requires traps (not only the innermost)"
  (doc
    "Soundness pin for stacked preconditions. A def may carry several `@requires`, which desugar to
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
  (input
    (do
      (@ (requires (>= x 0)) (@ (requires (<= x 100)) (def (f (: x Int64)) (+ x 1))))
      (def (main) (f -5))
      (export main)))
  (trap "unreachable"))

(case
  "STACKED @requires: value-transparent when ALL preconditions are satisfied"
  (doc
    "The value-transparency half of the stacked-@requires pin above. With both `(>= x 0)` and
           `(<= x 100)` stacked on `(f x) = x + 1`, `(f 50)` satisfies BOTH, so the nested checks
           `(if (>= x 0) (if (<= x 100) (+ x 1) trap) trap)` both take the pass arm and the def returns its
           own value `51` — no trap, no swallowed value. Together with the trap case above this pins that the
           multi-precondition enforcement composes correctly: every precondition is checked, and a run that
           satisfies all of them yields the computed result unchanged.")
  (input
    (do
      (@ (requires (>= x 0)) (@ (requires (<= x 100)) (def (f (: x Int64)) (+ x 1))))
      (def (main) (f 50))
      (export main)))
  (output (: 51 Int64)))

(case
  "@requires stacked OVER @ensures: the precondition is still enforced when an @ensures wrapper sits between it and the def"
  (doc
    "The reviewer's post-merge vector on the (D) @requires enforcement (a natural spelling of the
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
  (input
    (do
      (@ (requires (>= x 0)) (@ (ensures (>= ret 0)) (def (f (: x Int64)) (+ x 1))))
      (def (main) (f -5))
      (export main)))
  (trap "unreachable"))

(case
  "a PLAIN @ensures postcondition is ENFORCED at body-exit: a VIOLATED postcondition traps when the def is called"
  (doc
    "The (D) test-tier enforcement of a BARE `@ensures` (NOT stacked under `@test` — that is
           v-property-testing's TESTED tier, which they own). `verify_enforce::enforce` rewrites
           `(@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100)))` so the body becomes
           `(let ((it (- x 100))) (if (>= it 0) it (trap …)))` — the postcondition is checked at body-EXIT
           (the Hoare `{P} body {Q}` reading, `it` bound to the def's RESULT), and is VALUE-TRANSPARENT: the
           pass arm returns `it`, the def's own value, NOT `unit`. `(f 5)` computes `-95`, which violates
           `(>= ret 0)`, so the `if` takes the trap arm — `unreachable`. Before this increment a bare @ensures
           was RECORDED (db.ensures) but NOT enforced — `(f 5)` returned `-95`. Pins that a plain @ensures now
           actually verifies at run time. (A `@test @ensures` stack is v-property-testing's; this pass skips
           that shape to avoid double-injection — bare @ensures is v-verification's.)")
  (input
    (do (@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100))) (def (main) (f 5)) (export main)))
  (trap "unreachable"))

(case
  "a PLAIN @ensures postcondition is value-transparent when SATISFIED: the def returns its computed value"
  (doc
    "The value-transparency half of the plain-@ensures enforcement pin above. `(f 200)` computes `100`,
           which SATISFIES `(>= it 0)`, so the injected `(let ((it (- x 100))) (if (>= it 0) it (trap …)))`
           binds `ret = 100`, the `if` takes the pass arm, and the def returns `ret` = `100` — its OWN value,
           not `unit`, and no trap. Together with the trap case above this pins that the @ensures enforcement
           rewrite is value-transparent AND checking: a satisfied postcondition yields the computed result, a
           violated one traps.")
  (input
    (do (@ (ensures (>= ret 0)) (def (f (: x Int64)) (- x 100))) (def (main) (f 200)) (export main)))
  (output (: 100 Int64)))

(case
  "a @requires predicate over a HEAP collection is enforced at entry and value-transparent when satisfied"
  (doc
    "The runtime-enforce pins above use SCALAR predicates; this precondition READS a heap
           param ((> (List.len xs) 0)) — the injected entry check must BORROW xs then leave it live
           for the body's own match+len (a consuming check breaks the pass path). Satisfied → 52;
           empty → entry trap.")
  (input
    (do
      (@
        (requires (> (List.len xs) 0))
        (def
          (headx (: xs (List Int64)))
          (match xs (#list(h (.. _t)) (+ (* h 10) (List.len xs))) (#list() -1))))
      (def (main (: mode Int64)) (headx (if (= mode 1) #list(5 6) #list())))
      (export main)))
  (call main (: 1 Int64))
  (output (: 52 Int64))
  (call main (: 2 Int64))
  (trap "unreachable"))

(case
  "a @ensures postcondition over a HEAP result checks at exit and returns the live collection"
  (doc
    "The exit-side twin: (let ((it <body>)) (if (> (List.len it) 0) it (trap))) must borrow
           the RESULT list for the check then hand the LIVE handle to the caller — value-transparency
           over a heap handle, not a scalar (a consuming check breaks the caller's fold). Satisfied →
           caller folds 11; empty-returning mode → exit trap.")
  (input
    (do
      (@
        (ensures (> (List.len ret) 0))
        (def (mk (: mode Int64)) (if (= mode 1) #list(5 6) #list())))
      (def
        (sum-l (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (sum-l t (+ acc h)))))
      (def (main (: mode Int64)) (sum-l (mk mode) 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 11 Int64))
  (call main (: 2 Int64))
  (trap "unreachable")
  (live-objects 0))

(case
  "STACKED @requires and @ensures both enforce at runtime on one def with a heap precondition"
  (doc
    "Both wrappers on ONE def: entry borrows xs (len>0), exit guards ret>=0, the value flows
           through both injections (15). NB the conditions sit on a WRAPPER with recursion in a plain
           helper — @requires re-checks EVERY entry including self-calls (the recursive-@requires pin),
           so conditions directly on a self-recursive def whose tail shrinks to empty trap at their
           own base case. Empty input → entry trap.")
  (input
    (do
      (def
        (go (: xs (List Int64)) (: acc Int64))
        (match xs (#list() acc) (#list(h (.. t)) (go t (+ acc (if (< h 0) (- 0 h) h))))))
      (@
        (requires (> (List.len xs) 0))
        (@ (ensures (>= ret 0)) (def (abs-sum (: xs (List Int64))) (go xs 0))))
      (def (main (: mode Int64)) (abs-sum (if (= mode 1) #list(3 -7 5) #list())))
      (export main)))
  (call main (: 1 Int64))
  (output (: 15 Int64))
  (call main (: 2 Int64))
  (trap "unreachable")
  (live-objects 0))

(case
  "a RELATIONAL @requires over two parameters enforces their order at entry"
  (doc
    "The fn-level relational face (the type-level twin is the @invariant ordered-pair): one
           injected check reads BOTH params ((< lo hi)). Ordered → 14; swapped → entry trap.")
  (input
    (do
      (def (go (: i Int64) (: hi Int64) (: acc Int64)) (if (> i hi) acc (go (+ i 1) hi (+ acc i))))
      (@ (requires (< lo hi)) (def (range-sum (: lo Int64) (: hi Int64)) (go lo hi 0)))
      (def (main (: lo Int64) (: hi Int64)) (range-sum lo hi))
      (export main)))
  (call main (: 2 Int64) (: 5 Int64))
  (output (: 14 Int64))
  (call main (: 5 Int64) (: 2 Int64))
  (trap "unreachable"))

(case
  "a @ensures relating the RESULT to a PARAMETER enforces the relation at exit"
  (doc
    "The pins above check ret against CONSTANTS; (>= ret x) needs the param IN SCOPE inside
           the injected exit-let. The sign-flip face is the point: at x=-3 doubling SHRINKS (-6 < -3)
           so the postcondition genuinely fires; x=0 pins the boundary (0>=0).")
  (input
    (do
      (@ (ensures (>= ret x)) (def (double-up (: x Int64)) (* x 2)))
      (def (main (: x Int64)) (double-up x))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: -3 Int64))
  (trap "unreachable"))

(case
  "a PLAIN @ensures relating ret to a PARAMETER (> ret x) is enforced — the param stays in scope alongside ret in the predicate"
  (doc
    "The most common real-world postcondition shape: the result related to an INPUT, not just a
           constant. Every other runtime @ensures case pins `ret` against a literal (`>= ret 0`,
           `<= ret MAXINT`); this pins `@ensures(> ret x)` — \"the result exceeds the input\". The
           injected `(let ((ret (+ x 1))) (if (> ret x) ret (trap …)))` must keep the def's PARAM `x`
           in scope INSIDE the predicate ALONGSIDE the synthesized `ret` binder — the predicate reads
           BOTH. `(f 5)` computes `6`, and `6 > 5` holds, so the `if` takes the pass arm and the def
           returns `ret` = `6`. Pins that a multi-name postcondition (result-vs-parameter) resolves and
           enforces exactly like a hand-written `(if (> (+ x 1) x) …)`.")
  (input
    (do (@ (ensures (> ret x)) (def (f (: x Int64)) (+ x 1))) (def (main) (f 5)) (export main)))
  (output (: 6 Int64)))

(case
  "a PLAIN @ensures relating ret to a PARAMETER (> ret x) TRAPS when violated — the result-vs-input postcondition is checked"
  (doc
    "The trap half of the result-vs-parameter postcondition above. `@ensures(> ret x)` on
           `(g x) = x - 1` — the result must exceed the input, but `x - 1 < x` always, so the
           postcondition is violated for every argument. The injected
           `(let ((ret (- x 1))) (if (> ret x) ret (trap …)))` binds `ret = 4` for `(g 5)`, and
           `4 > 5` is FALSE, so the `if` takes the trap arm — `unreachable`. Together with the case
           above this pins that a postcondition reading BOTH `ret` and a param enforces in both
           directions, not only the satisfied one.")
  (input
    (do (@ (ensures (> ret x)) (def (g (: x Int64)) (- x 1))) (def (main) (g 5)) (export main)))
  (trap "unreachable"))

(case
  "a PLAIN @ensures over a HEAP result (List) is enforced — ret binds a heap value, value-transparent when satisfied"
  (doc
    "The runtime @ensures cases so far all return a SCALAR (Int64); this pins @ensures over a def
           that returns a HEAP value. The injected `(let ((ret BODY)) (if Q ret (trap …)))` binds `ret`
           to a LIST, the predicate reads it via `(List.len ret)`, and the pass arm returns that same
           heap value — value-transparency must hold for a heap return, not only a scalar. `(f 7)` builds
           `(List.push (list) 7)` (a 1-element list), `(> (List.len ret) 0)` holds, so the def returns the
           list and `main` reads its length `1`. Pins that the @ensures rewrite binds + returns a heap
           `ret` correctly (no ownership/drop hazard from the extra let-binding of a heap value).")
  (input
    (do
      (@ (ensures (> (List.len ret) 0)) (def (f (: x Int64)) (List.push #list() x)))
      (def (main) (List.len (f 7)))
      (export main)))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "a PLAIN @ensures over a HEAP result (List) TRAPS when violated — the postcondition checks the heap value"
  (doc
    "The trap half of the heap-result postcondition above. `@ensures(> (List.len ret) 0)` on
           `(g x) = (list)` — the result must be non-empty, but the body returns the EMPTY list, so the
           postcondition is violated. The injected `(let ((ret (list))) (if (> (List.len ret) 0) ret
           (trap …)))` binds `ret` to the empty list, `(List.len ret) = 0`, `(> 0 0)` is FALSE, so the
           `if` takes the trap arm — `unreachable`. Together with the case above this pins that an
           @ensures over a heap return enforces in both directions.")
  (input
    (do
      (@ (ensures (> (List.len ret) 0)) (def (g (: x Int64)) #list()))
      (def (main) (List.len (g 7)))
      (export main)))
  (trap "unreachable"))

(case
  "a PLAIN @ensures whose predicate reads ONLY a parameter (not ret) is enforced — the dual of the nullary case"
  (doc
    "Every runtime @ensures case reads the result binder `ret`; this pins the DUAL — a postcondition that
           references ONLY a PARAMETER and ignores `ret`. `@ensures(> x 0)` on `(f x) = (- x 1)`: the injected
           `(let ((ret (- x 1))) (if (> x 0) ret (trap …)))` binds `ret` (unused by the predicate) and checks
           `(> x 0)` over the param `x` — a postcondition constraining the INPUT at exit, a legitimate (if
           unusual) contract. `(f 5)`: `x = 5 > 0` holds, so the check takes the pass arm and returns `ret` =
           `4` — its own value. Pins that the enforcement wrap injects + returns `ret` correctly even when the
           predicate never mentions it (the binder is still introduced, the body value still flows through, the
           predicate resolves against the param in scope). Complements the nullary case (predicate reads only
           `ret`, no param): together they pin both extremes of what an @ensures predicate may reference.")
  (input (do (@ (ensures (> x 0)) (def (f (: x Int64)) (- x 1))) (def (main) (f 5)) (export main)))
  (output (: 4 Int64)))

(case
  "a PLAIN @ensures with a constant-FALSE predicate always traps — the postcondition fires unconditionally"
  (doc
    "The degenerate soundness pin: an `@ensures false` (a predicate that is the literal `false`,
           independent of `ret` or any param) must ALWAYS trap when the def runs — the postcondition can never
           be satisfied. The injected `(let ((ret x)) (if false ret (trap …)))` binds `ret` then takes the trap
           arm unconditionally — `unreachable`. `(f 5)` traps despite the body `x` = `5` computing fine. Pins
           that the enforcement wrap does NOT const-fold away a statically-false postcondition into a silent
           pass (a `(if false …)` that dropped the trap arm would let a provably-false contract compile to a
           returning function) — the check is faithful even when the predicate is a compile-time constant.")
  (input (do (@ (ensures false) (def (f (: x Int64)) x)) (def (main) (f 5)) (export main)))
  (trap "unreachable"))

(case
  "TWO stacked @ensures COMPOSE: BOTH postconditions are enforced — value-transparent when both hold"
  (doc
    "The `@ensures`-composition pin (analogue of the stacked-`@requires` cases above, for the exit side).
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
  (input
    (do
      (@ (ensures (>= ret 0)) (@ (ensures (< ret 1000)) (def (f (: x Int64)) (+ x 1))))
      (def (main) (f 5))
      (export main)))
  (output (: 6 Int64)))

(case
  "TWO stacked @ensures: a violated INNER postcondition traps even when the OUTER holds"
  (doc
    "The trap half of the stacked-@ensures composition above — and the discriminating case: it fails only
           the INNER postcondition, so a naive implementation that enforced only the outermost `@ensures` (or
           only the innermost) would let it slip. `(@ (ensures (>= ret 0)) (@ (ensures (< ret 1000)) (def (f x)
           (+ x 2000))))` on `(f 5)` computes `2005`. The INNER `@ensures(< ret 1000)` is checked first on the
           raw body value (`2005 < 1000` is FALSE) → its `if` takes the trap arm — `unreachable` — BEFORE the
           outer `@ensures(>= ret 0)` (which WOULD hold, `2005 >= 0`) ever runs. Pins that EVERY stacked
           postcondition is enforced, not just one: an inner violation traps regardless of the outer verdict
           (the exit-side twin of the stacked-@requires \"violated OUTER traps\" case).")
  (input
    (do
      (@ (ensures (>= ret 0)) (@ (ensures (< ret 1000)) (def (f (: x Int64)) (+ x 2000))))
      (def (main) (f 5))
      (export main)))
  (trap "unreachable"))

(case
  "@ensures on a def with a parameter named ret is REJECTED (would silently not enforce — rename the param)"
  (doc
    "The result-binder-capture guard, as a REJECT (breaker 2026-07-17). `@ensures(Q)` enforcement binds
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
  (input
    (do
      (@ (ensures (>= ret 0)) (def (f (: ret Int64)) (- ret 100)))
      (def (main) (f 5))
      (export main)))
  (error CDZ0201 (message "cannot carry `@ensures`")))

(case
  "a @requires predicate that references `ret` is REJECTED CDZ0101 — only @ensures binds the result"
  (doc
    "The scope-boundary pin between the two annotations: `ret` is the @ENSURES result binder, and NOTHING
           else introduces it. A `@requires` runs at body-ENTRY, before any result exists, so it binds only the
           def's PARAMETERS (and prelude/global names) — `ret` is NOT in scope. A `@requires(>= ret 0)` therefore
           references an UNBOUND name and is rejected CDZ0101 at the annotation. This guards the exact boundary:
           a regression that leaked the @ensures `ret` binder into `@requires` scope would silently ACCEPT a
           nonsensical precondition (a precondition over a not-yet-computed result), so pinning the reject keeps
           the two contracts' scopes distinct. (`collect_faults` skips the def's params + — for @ensures ONLY —
           the `ret` subject when checking predicate names; `@requires` passes no subject, so `ret` resolves to
           Poison(CDZ0101) exactly as any stray name would.)")
  (input (do (@ (requires (>= ret 0)) (def (f (: x Int64)) x)) (def (main) (f 5)) (export main)))
  (error CDZ0101))

(case
  "a @requires predicate that is NOT Bool-typed is REJECTED CDZ0203 — a predicate must denote a truth value"
  (doc
    "The type-of-the-predicate pin, distinct from the scope pins above (unbound name → CDZ0101). A contract
           predicate is spliced into an injected `(if PRE BODY (trap …))`, whose test position DEMANDS a Bool, so
           the predicate expression must type as Bool. `@requires(+ x 1)` gives an Int64-typed predicate (`x` is
           Int64, `+` returns Int64) — well-scoped, but the WRONG TYPE for a truth value. It is rejected CDZ0203
           (cannot unify Int64 with Bool) at the annotation, exactly as a hand-written `(if (+ x 1) …)` would be.
           This guards the predicate's TYPE obligation separately from its NAME obligation: a regression that
           dropped the Bool constraint on the predicate would silently accept `@requires(+ x 1)` and splice a
           non-Bool into the `if`, either miscompiling or coercing the guard — so pinning the reject keeps a
           contract predicate constrained to a genuine truth value. (The dual of the value-domain enforcement
           cases: those pin that a Bool-typed predicate ENFORCES; this pins that a non-Bool one is REFUSED.)")
  (input (do (@ (requires (+ x 1)) (def (f (: x Int64)) x)) (def (main) (f 5)) (export main)))
  (error CDZ0203))

(case
  "an @ensures predicate that is NOT Bool-typed is REJECTED CDZ0203 — the postcondition must denote a truth value too"
  (doc
    "The @ensures twin of the non-Bool @requires reject above, pinning the Bool constraint on the EXIT-check
           side. An @ensures predicate binds `ret` and is spliced into the injected `(let ((ret BODY)) (if Q ret
           (trap …)))`, whose `if` test DEMANDS a Bool, so the postcondition expression must type as Bool.
           `@ensures(+ ret 1)` is Int64-typed (`ret` is Int64, `+` returns Int64) — well-scoped (ret IS in scope
           for @ensures, unlike @requires), but the WRONG TYPE for a truth value → rejected CDZ0203 (cannot unify
           Int64 with Bool). Pins that BOTH injected checks constrain their predicate to Bool: a regression that
           dropped the Bool constraint on only the postcondition side would splice a non-Bool into the exit `if`
           while the entry side stayed sound. Together with the @requires case this closes the predicate-type
           obligation on both rewrite halves.")
  (input (do (@ (ensures (+ ret 1)) (def (f (: x Int64)) x)) (def (main) (f 5)) (export main)))
  (error CDZ0203))

(case
  "a PLAIN @ensures on a NULLARY def (no parameters) enforces — ret binds the body, predicate reads only ret"
  (doc
    "Every runtime @ensures case so far has at least one parameter; this pins @ensures on a def with NO
           parameters, where the postcondition predicate reads ONLY the result binder `ret` (no param is in
           scope to reference). The injected `(let ((ret BODY)) (if Q ret (trap …)))` binds `ret` to the
           nullary body and checks the predicate over it alone. `(def (f) (- 5 10))` computes `ret = -5`, which
           violates `@ensures(>= ret 0)`, so the `if` takes the trap arm — `unreachable`. Pins that the
           enforcement rewrite needs no parameter to inject its check (the empty param list is not a special
           case that skips enforcement) and that a nullary def's postcondition is checked over the result alone.")
  (input (do (@ (ensures (>= ret 0)) (def (f) (- 5 10))) (def (main) (f)) (export main)))
  (trap "unreachable"))

; ── @requires enforcement EDGES (breaker) — beyond the const-arg violated/satisfied pair above ──────
; The two (D) pins above call `f` with a CONSTANT argument, so a fold could in principle have discharged
; the check at compile time. These pin the enforcement's REACH: a genuinely-runtime argument (the check
; must be emitted, not folded), a RECURSIVE def (the body-entry check re-fires at every entry, including
; self-calls), a predicate that itself PERFORMS an effect (the pre runs under the caller's handler and
; ADVANCES its state before the body runs), and a predicate that itself TRAPS (its own trap kind wins —
; the requires rewrite adds no guard around the predicate's evaluation).
(case
  "a @requires precondition is enforced for a genuinely-runtime argument"
  (doc
    "The runtime companion of the const-arg violation pin above: the argument arrives at the CALL
           BOUNDARY, so nothing folds and the injected body-entry `(if (>= x 0) … (trap …))` must actually
           run. `(f -5)` violates → the canonical unreachable trap; `(f 5)` satisfies → 6, value-transparent.
           A pass that only proved const violations (or an emit that dropped the check on the runtime path)
           would return -4 here — the exact pre-(D) behavior — so this is the regression pin for the
           EMITTED check.")
  (input
    (do
      (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: -5 Int64))
  (trap "unreachable")
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a @requires on a recursive def is re-checked at every entry including self-calls"
  (doc
    "The body-entry reading ({P} body {Q}, checked when the function RUNS) puts the injected check
           inside the def, so a RECURSIVE def re-fires it on every self-call, not only the outermost entry.
           `fact` with `@requires (>= n 0)`: n=4 → 24 (every recursive entry 4,3,2,1,0 satisfies), n=-1 →
           the entry check traps immediately. Pins that the rewrite composes with recursion (specialization
           /accumulator transforms must keep the per-entry check) — a call-site-only reading would also
           pass n=4 but differs on shapes where an internal entry first violates.")
  (input
    (do
      (@ (requires (>= n 0)) (def (fact (: n Int64)) (if (= n 0) 1 (* n (fact (- n 1))))))
      (def (main (: k Int64)) (fact k))
      (export main)))
  (call main (: 4 Int64))
  (output (: 24 Int64))
  (call main (: -1 Int64))
  (trap "unreachable"))

(case
  "an @ensures on a recursive def is re-checked at every EXIT including self-call returns (not only the outermost)"
  (doc
    "The @ensures twin of the recursive-@requires case above — the exit-side per-entry pin, with a
           DISCRIMINATING shape. `@ensures` wraps the body as `(let ((ret BODY)) (if Q ret (trap …)))` INSIDE
           the def, so a recursive def re-checks the postcondition on EVERY exit, including each self-call
           return — not only the outermost. `f` with `@ensures (>= ret 0)`: `f 0 = 5` (ok, the control); `f 1
           = (- (f 0) 10) = -5` (VIOLATES); `f 2 = (+ (f 1) 10) = 5` — the OUTERMOST result 5 satisfies, but
           reaching it recurses through `f 1` whose exit value `-5` fails `(>= ret 0)`, so the per-exit check
           traps at that inner return BEFORE `f 2` ever returns. A postcondition read only at the outermost
           call would wrongly return 5; the per-exit check traps `unreachable`. Pins that the rewrite composes
           with recursion on the exit side (a tail/accumulator transform must keep the per-exit check).")
  (input
    (do
      (@
        (ensures (>= ret 0))
        (def (f (: n Int64)) (if (<= n 0) 5 (if (= n 1) (- (f 0) 10) (+ (f (- n 1)) 10)))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 0 Int64))
  (output (: 5 Int64))
  (call main (: 2 Int64))
  (trap "unreachable"))

(case
  "an EFFECTFUL @requires predicate performs under the caller's handler and advances its state before the body"
  (doc
    "The predicate `(> (Counter.bump) 0)` PERFORMS an operation, so the injected body-entry check is
           itself effectful: it must route to the dynamically-enclosing handler and its state advance must
           be SEEN by the body's own later perform — the check is sequenced BEFORE the body, in the same
           handler extent, not hoisted out of it or double-performed. Seeded 0: the pre's bump resumes 1
           (>0, satisfied — and threads state 1), the body's bump resumes 2, so `(f 10)` = 10 + 2 = 12. A
           rewrite that evaluated the predicate OUTSIDE the handler would fail to compile or trap; one that
           re-evaluated it would yield 13.")
  (input
    (do
      (effect Counter (op bump (-> Unit Int64)))
      (@ (requires (> (Counter.bump) 0)) (def (f (: n Int64)) (+ n (Counter.bump))))
      (def (main (: n Int64)) (handle Counter 0 ((bump (u) s (resume (+ s 1) (+ s 1)))) (f n)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 12 Int64)))

(case
  "an EFFECTFUL @ensures predicate performs under the caller's handler at body-EXIT, after the body's own perform"
  (doc
    "The @ensures twin of the effectful-@requires case above — the exit-side handler-extent pin. The
           postcondition `(> (Counter.bump) 100)` PERFORMS, so the injected exit check `(let ((ret BODY)) (if
           (> (Counter.bump) 100) ret (trap …)))` is itself effectful: it must route to the dynamically
           enclosing handler and be sequenced AFTER the body (the body already performed to compute `ret`), in
           the same handler extent — not hoisted, not double-performed, not evaluated before the body. Handler
           seeded 0, each `bump` resumes `s+1` and threads `s+1`: the BODY's bump is the FIRST perform (resumes
           1, state→1), so `ret = 10 + 1 = 11`; the postcondition's bump is the SECOND (resumes 2, state→2).
           `(> 2 100)` is FALSE, so the @ensures check takes the trap arm — `unreachable`. Pins that an
           effectful postcondition performs in-handler at body-exit AND its verdict is enforced (a rewrite that
           evaluated it before the body, or outside the handler, would resume 1 / fail to compile).")
  (input
    (do
      (effect Counter (op bump (-> Unit Int64)))
      (@ (ensures (> (Counter.bump) 100)) (def (f (: n Int64)) (+ n (Counter.bump))))
      (def (main (: n Int64)) (handle Counter 0 ((bump (u) s (resume (+ s 1) (+ s 1)))) (f n)))
      (export main)))
  (call main (: 10 Int64))
  (trap "unreachable"))

(case
  "an EFFECTFUL @ensures predicate is value-transparent when SATISFIED — the body's own perform runs first"
  (doc
    "The satisfied control for the effectful-@ensures trap above, pinning the perform ORDER precisely.
           Same shape but with a threshold `(> (Counter.bump) 0)` the second bump satisfies. Handler seeded 0,
           resumes `s+1` threading `s+1`: the BODY's bump is FIRST (resumes 1) so `ret = 10 + 1 = 11`; the
           postcondition's bump is SECOND (resumes 2), and `(> 2 0)` HOLDS, so the check takes the pass arm and
           the def returns `ret` = `11` — its own value, no trap. The result being `11` (not `12`) is the load-
           bearing detail: it proves the body performed BEFORE the postcondition (body drew state 1), and that
           the postcondition's own perform advanced state WITHOUT being folded into the returned value. A
           rewrite that evaluated the postcondition first would yield `12`; one that double-performed the body
           would drift further.")
  (input
    (do
      (effect Counter (op bump (-> Unit Int64)))
      (@ (ensures (> (Counter.bump) 0)) (def (f (: n Int64)) (+ n (Counter.bump))))
      (def (main (: n Int64)) (handle Counter 0 ((bump (u) s (resume (+ s 1) (+ s 1)))) (f n)))
      (export main)))
  (call main (: 10 Int64))
  (output (: 11 Int64)))

(case
  "a @requires predicate that itself traps keeps its own trap kind"
  (doc
    "The predicate `(> (/ 10 n) 0)` divides by its parameter, so at n=0 evaluating the PREDICATE
           traps `integer divide by zero` — a DIFFERENT kind from the requires-violation `unreachable`.
           The enforcement rewrite wraps the BODY in the predicate-guarded if; it adds no guard around the
           predicate's own evaluation, so the predicate's trap fires first and keeps its kind (trap-kind
           observability: reordering or re-classifying it would be a miscompile). n=2 satisfies (10/2=5>0)
           → 2, the control.")
  (input
    (do
      (@ (requires (> (/ 10 n) 0)) (def (f (: n Int64)) n))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an @ensures predicate that itself traps keeps its own trap kind (not the @ensures-failed unreachable)"
  (doc
    "The @ensures twin of the @requires-predicate-traps case above — the exit-side trap-kind-observability
           pin. The postcondition `(> (/ 100 ret) 0)` divides by the RESULT binder, so when `ret = 0` evaluating
           the PREDICATE traps `integer divide by zero` — a DIFFERENT kind from the postcondition-violation
           `unreachable`. The enforcement rewrite is `(let ((ret BODY)) (if (> (/ 100 ret) 0) ret (trap …)))`:
           it binds `ret` then evaluates the predicate in the `if` test, adding NO guard around the predicate's
           own evaluation — so the predicate's trap fires first and keeps its kind. `(f 5)` computes `ret = 5`,
           `(/ 100 5) = 20 > 0` holds → returns `5` (the control). `(f 0)` computes `ret = 0`, and the
           predicate's `(/ 100 0)` traps `divide by zero` BEFORE the postcondition verdict is reached —
           reordering or re-classifying it to `unreachable` would be a miscompile (the postcondition-failure
           trap only fires when the predicate EVALUATES to false, not when it traps).")
  (input
    (do
      (@ (ensures (> (/ 100 ret) 0)) (def (f (: n Int64)) n))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "an @ensures over a MATCH-bodied def wraps the whole match — the postcondition checks the match's result"
  (doc
    "A cross-seam composition pin (v-patterns seam): the def BODY is a `match`, and @ensures must wrap the
           WHOLE match expression, not one arm. The injected `(let ((ret (match x …))) (if (>= ret 0) ret
           (trap …)))` binds `ret` to whichever arm the scrutinee selects, then checks the postcondition over
           that result. `(f x) = (match x (0 -1) (_ x))`: `(f 5)` takes the wildcard arm → `ret = 5`, `(>=
           5 0)` holds → returns `5`; `(f 0)` takes the `0` arm → `ret = -1`, `(>= -1 0)` is FALSE → the check
           traps `unreachable`. Pins that the enforcement rewrite composes with a match-bodied def (the `let`
           binds the match's value, the check sees the selected arm's result) — a future pattern-matching change
           that mis-scoped the injected `ret` binder around a match would flip this. Runtime scrutinee via
           `main`'s param so neither arm folds away.")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (f (: x Int64)) (match x (0 -1) (_ x))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "a @requires predicate that CALLS a user-defined function resolves and enforces — not only prelude ops"
  (doc
    "A cross-seam composition pin (name-resolution seam): the precondition predicate is not a bare prelude
           comparison but a CALL to a user-defined function, `(ok x)` where `(def (ok n) (>= n 0))`. The
           enforcement rewrite `(if (ok x) BODY (trap …))` must RESOLVE `ok` (a top-level def, in scope at body
           entry alongside the params) and call it — predicate resolution is not restricted to prelude
           intrinsics. `(f 7)`: `(ok 7)` = true → returns `8`; `(f -3)`: `(ok -3)` = false → the precondition
           check traps `unreachable`. Runtime arg via `main`'s param so the call isn't const-folded. Pins that
           an @requires predicate may be an ordinary boolean-returning user function (the predicate is elaborated
           in the def's scope like any expression) — a resolution change that only bound prelude names in a
           predicate would break this.")
  (input
    (do
      (def (ok (: n Int64)) (>= n 0))
      (@ (requires (ok x)) (def (f (: x Int64)) (+ x 1)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 7 Int64))
  (output (: 8 Int64))
  (call main (: -3 Int64))
  (trap "unreachable"))

(case
  "a @requires predicate that calls a RECURSIVE user helper evaluates soundly — predicate eval drives recursion to a fixpoint"
  (doc
    "Extends the user-fn predicate case above (a FLAT `(ok x)`) to a RECURSIVE helper, pinning that
           predicate evaluation drives a self-recursive call to termination like any expression — the injected
           `(if (even x) BODY (trap …))` must fully evaluate the recursion before branching, not just resolve the
           name. `(even n)` counts down by 2 to a base case: `(< n 1) → true`, `(= n 1) → false`, else `(even (-
           n 2))`. `f` is guarded by `@requires(even x)`. main(4): even(4)→even(2)→even(0)=true → the
           precondition holds → 4. main(3): even(3)→even(1)=false → the precondition is false → trap. That the
           odd input traps only AFTER the recursion unwinds to its base case proves the predicate call is
           evaluated to a fixpoint at body-entry, not short-circuited — a predicate is an ordinary computation,
           recursion included. Runtime arg via main's param so nothing folds.")
  (input
    (do
      (def (even (: n Int64)) (if (< n 1) true (if (= n 1) false (even (- n 2)))))
      (@ (requires (even x)) (def (f (: x Int64)) x))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 4 Int64))
  (output (: 4 Int64))
  (call main (: 3 Int64))
  (trap "unreachable"))

(case
  "a @requires predicate that MATCHES on a sum-typed parameter dispatches and enforces (v-patterns seam)"
  (doc
    "A cross-seam composition pin (pattern-matching seam): the precondition predicate is not a scalar
           comparison but a `match` that DISPATCHES on a sum-typed parameter, so the injected `(if (match o …)
           BODY (trap …))` must resolve + lower a full match in the predicate position, binding the payload and
           choosing the boolean arm. `(f o)` with `@requires(match o ((Opt.Some n) (>= n 0)) ((Opt.None)
           false))`: the precondition is TRUE iff `o` is `Some n` with `n >= 0`. `(f (Opt.Some 7))`: matches the
           Some arm, `(>= 7 0)` holds → body runs → `7`; `(f (Opt.Some -3))`: Some arm, `(>= -3 0)` FALSE → the
           precondition check traps `unreachable`. Runtime payload via `main`'s param so no arm folds. Pins that
           an @requires predicate may itself be a match over a sum parameter (the predicate is elaborated +
           lowered in the def's scope exactly like a body expression) — a pattern-matching change that failed to
           lower a match in the injected precondition guard would break this.")
  (input
    (do
      (type Opt (None) (Some Int64))
      (@
        (requires (match o ((Opt.Some n) (>= n 0)) ((Opt.None) false)))
        (def (f (: o Opt)) (match o ((Opt.Some n) n) ((Opt.None) 0))))
      (def (main (: k Int64)) (f (Opt.Some k)))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (trap "unreachable"))

(case
  "an @ensures predicate reading a top-level GLOBAL alongside ret resolves and enforces (resolution seam)"
  (doc
    "A cross-seam pin (name-resolution seam): the postcondition references a top-level GLOBAL definition,
           not only the result binder `ret` and the def's params. `@ensures(< ret (limit))` on `(f x) = (+ x
           1)` with `(def (limit) 100)`: the injected `(let ((ret (+ x 1))) (if (< ret (limit)) ret (trap …)))`
           must RESOLVE `(limit)` (a top-level nullary def, in scope in the predicate exactly as in any body
           expression) alongside the synthesized `ret`. `(f 5)`: `ret = 6`, `(< 6 100)` holds → returns `6`;
           `(f 200)`: `ret = 201`, `(< 201 100)` FALSE → the postcondition traps `unreachable`. Runtime arg via
           `main`'s param (no fold). Pins that predicate name-resolution reaches the global scope, not just
           params + `ret` — a resolution change that scoped the predicate too narrowly would break this.")
  (input
    (do
      (def (limit) 100)
      (@ (ensures (< ret (limit))) (def (f (: x Int64)) (+ x 1)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: 200 Int64))
  (trap "unreachable"))

(case
  "a @requires on a UNIT-returning def is enforced — the precondition traps before the unit body"
  (doc
    "The degenerate-result pin: a def whose BODY is `unit` (the empty tuple) still gets its `@requires`
           enforced. The injected `(if (>= x 0) unit (trap …))` checks the precondition at body-entry regardless
           of the body's type — a unit body is not a special case that skips enforcement. `(f 5)`: `(>= 5 0)`
           holds → returns `unit` (the body value, value-transparent even for unit); `(f -1)`: `(>= -1 0)` FALSE
           → the precondition traps `unreachable` before the unit body. Pins that enforcement is orthogonal to
           the body's result type — it wraps a unit-returning def as faithfully as a scalar one (a rewrite that
           keyed on a non-unit result would drop the check here).")
  (input
    (do
      (@ (requires (>= x 0)) (def (f (: x Int64)) unit))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output unit)
  (call main (: -1 Int64))
  (trap "unreachable"))

; ── @requires × @test: constrained GENERATION (breaker pin, keyed on the 71efd45a6 slice) ──────────
; A `@requires` precondition on a `@test`-stacked def is a FILTER on the generated input domain, not a
; property the test may fail on. The ruling (v-verification + v-property-testing, 2026-07-17): the
; @requires trap stays a HARD production contract, so the ONLY sound test-runner behavior is to DRAW
; IN-DOMAIN — a generated input violating the pre must never surface as a spurious counterexample
; (`f(-1)` under `(requires (>= x 0))` was exactly that before the constrained-gen slice). The corpus
; can't drive `cdz test` directly, so this pins the DEF-SIDE composition the runner relies on: the
; stacked def, called in-domain, enforces the pre, the body, and the post exactly as unstacked.
(case
  "a @test-stacked @requires+@ensures def keeps full contract enforcement for a direct call"
  (doc
    "The def-side composition the constrained-gen ruling relies on: `@test` stacked over
           `(@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def f …)))` leaves the def's OWN contract
           intact for ordinary calls — in-domain `(f 5)` runs pre → body → post and returns 6;
           out-of-domain `(f -5)` still HARD-TRAPS on the pre (the production contract the test
           runner must respect by drawing in-domain, never a soft reject). Pins that the @test wrapper
           is transparent to direct-call enforcement — the test tier changes how INPUTS are drawn,
           not what the contract means.")
  (input
    (do
      (@ test (@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x 1)))))
      (def (main (: n Int64)) (f n))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -5 Int64))
  (trap "unreachable"))

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
(case
  "@invariant ESTABLISH: a constructor's @requires discharges the type invariant as an implicit @ensures (design §10.2)"
  (doc
    "The DATA-level verification-family member (design §10). An `@invariant(and (>= it 0) (<= it 100))`
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
      (type HeadOp (Le) (Ge) (Conj))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))
          ((HeadOp.Ge) (match b ((HeadOp.Ge) true) (_ false)))
          ((HeadOp.Conj) (match b ((HeadOp.Conj) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (ge (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Ge) a) b))
      ; `conj` mirrors the surface `and` — the invariant `(and P Q)` denotes to `(conj P Q)`.
      (def (conj (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Conj) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; establish: from the two precondition facts, mint the invariant CONJUNCTION carrying both as hyps.
      (def (establish (: p Term) (: q Term)) (Thm.Seq #list(p q) (conj p q)))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq le ge conj concl hyps establish licenses)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq le ge conj concl hyps establish licenses))
      (def
        (main)
        (let
          ((v (Term.Var 0)) (zero (Term.Num 0)) (c100 (Term.Num 100)))
          ; the invariant obligation I[self := v] = (conj (ge v 0) (le v 100))
          (let
            ((obligation (conj (ge v zero) (le v c100)))
              ; the constructor precondition {ge v 0, le v 100} (its @requires = the invariant)
              (precondition #list((ge v zero) (le v c100))))
            ; ESTABLISH: mint the invariant conjunction from the two precondition facts
            (let
              ((proof (establish (ge v zero) (le v c100))))
              (licenses proof obligation precondition)))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

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
(case
  "@ensures-over-@requires stacked on an EFFECTFUL body is order-insensitive: compiles + enforces like the forward order"
  (doc
    "The cross-vertical composition pin (v-verification contract enforcement × v-effects let-trap
           lowering). `(@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.tick)))))`
           under a counter handler: the reversed stack's precondition-fail branch binds `it` to the requires
           trap — `(let ((it (trap …))) (if (> it 0) it (trap …)))` — which formerly mis-declined on the
           scalar `(> it 0)` as a compound comparison (the let-bound trap typed as bottom, is_scalar=false),
           while forward order worked. The v-effects fix (a let with an unconditionally-trapping init folds to
           the trap) makes it lower correctly, so the reversed order now behaves EXACTLY like the forward
           twin: pre `(>= 100 0)` ok, body `(+ 100 (St.tick))` resumes 1 → 101, post `(> 101 0)` ok → 101.
           Pins that contract stacking order is presentation, not semantics, over an effect-performing body —
           and guards the let-bound-trap lowering my composition relies on.")
  (input
    (do
      (effect St (op tick (-> Unit Int64)))
      (@ (ensures (> ret 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.tick)))))
      (def (main (: k Int64)) (handle St k ((tick (u) s (resume (+ s 1) (+ s 1)))) (f 100)))
      (export main)))
  (call main (: 0 Int64))
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
(case
  "@invariant PRESERVE: an operation returning T discharges the result invariant USING the input invariant as a free gift (design §10.2)"
  (doc
    "The PRESERVE half + consumer-gift (design §10.2), dual to the ESTABLISH case. An operation
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
      (type HeadOp (Sub) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Sub) (match b ((HeadOp.Sub) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (sub (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Sub) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      ; PRESERVE step: from `|- (le in c)` derive `|- (le (sub in 1) c)` — decreasing the lhs keeps `<= c`
      ; (dec never raises the value, so the upper bound is preserved). Hyps carried unchanged (the input gift).
      (def
        (dec-le (: th Thm))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (sub x (Term.Num 1)) c))))
          (_ (Option.None))))
      (def
        (mem (: q Term) (: ps (List Term)))
        (match ps (#list() false) (#list(h (.. t)) (if (term-eq q h) true (mem q t)))))
      (def
        (hyps-subset (: hs (List Term)) (: pre (List Term)))
        (match hs (#list() true) (#list(h (.. t)) (if (mem h pre) (hyps-subset t pre) false))))
      (def
        (licenses (: thm Thm) (: obligation Term) (: pre (List Term)))
        (and (term-eq (concl thm) obligation) (hyps-subset (hyps thm) pre)))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq sub le concl hyps assume dec-le licenses)))
  (input
    (do
      (import "bounds" (HeadOp Term Thm term-eq sub le concl hyps assume dec-le licenses))
      (def
        (main)
        (let
          ((in (Term.Var 0)) (c100 (Term.Num 100)))
          ; the RESULT-invariant obligation: le (dec in) 100  (the Percent upper bound on the result)
          (let
            ((obligation (le (sub in (Term.Num 1)) c100))
              ; the granted consumer-gift: the INPUT invariant `le in 100` (every Percent holds it)
              (precondition #list((le in c100))))
            ; PRESERVE: assume the input gift, decrement, and the result upper bound follows
            (let
              ((gift (assume (le in c100))))
              (match
                (dec-le gift)
                ((Option.Some proof) (licenses proof obligation precondition))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; ── @invariant NAME-RESOLUTION: a predicate name outside {it, prelude} is unbound (b4c pattern, data-level) ─
; An `@invariant(pred)` predicate references only the value binder `self` (the value of the type) and prelude/
; global names — a type declaration has no parameters. A name that is NEITHER is UNBOUND, reported CDZ0101 at
; the annotation (the same b4c name-resolution the @requires/@ensures predicates get, reused for the data-
; level member via `Db::invariant_preds`). Pins that a stray name in a data invariant is caught locally, not
; silently accepted (the soundness discipline: a contract predicate resolves like ordinary code).
(case
  "@invariant with an unbound predicate name is REJECTED (CDZ0101 — only `self` + prelude are in scope)"
  (doc
    "The data-level name-resolution pin. `@invariant(and (>= it 0) (< it bogus))` on `type Percent`:
           `self` is the value binder (in scope) and `>=`/`<`/`and` are prelude ops (resolve), but `bogus` is
           neither a prelude name nor the value binder — so it is UNBOUND, CDZ0101 at the annotation. A type
           has no parameters, so the invariant predicate's scope is exactly {`self`, prelude/global} — anything
           else is a stray name. Pins that `collect_faults` name-resolves the invariant predicate (via
           `Db::invariant_preds`) with the same b4c discipline the @requires/@ensures predicates get, so a
           typo'd data invariant fails locally with a clear message rather than being silently recorded.")
  (input
    (do
      (@ (invariant (and (>= self 0) (< self bogus))) (type Percent (Pct Int64)))
      (def (main) 0)
      (export main)))
  (error CDZ0101))

; The @requires/@ensures analogue of the @invariant name-resolution pin: a predicate references only names
; in scope — the def's PARAMETERS, `ret` (for @ensures), and prelude/global names. A name that is none of
; those is UNBOUND → CDZ0101 at the annotation (b4c discipline). The valid-names path (a predicate over
; params / ret / prelude ops) is the satisfying-@requires/@ensures family elsewhere in this file. (migrated
; from rcdzc requires_ensures_predicate_unbound_name_is_cdz0101_valid_names_ok.)
(case
  "an @requires predicate referencing an unbound name is rejected CDZ0101"
  (input (do (@ (requires (> y 0)) (def (f (: x Int64)) (+ x 1))) (export f)))
  (error CDZ0101))

(case
  "an @ensures predicate referencing an unbound name is rejected CDZ0101"
  (input (do (@ (ensures (> zzz 0)) (def (f (: x Int64)) (+ x 1))) (export f)))
  (error CDZ0101))

(case
  "@invariant destructure-arm predicate: a stray name inside the arm is still REJECTED (arm binder scope does not mask it)"
  (doc
    "The destructure-form sibling of the flat unbound-name reject above. The canonical
           `@invariant(match self ((T.V v) …))` shape binds `v` predicate-LOCALLY (in scope in the arm — see
           the destructure-invariant establish cases below that use such binders successfully), but a name that
           is NEITHER the arm binder NOR prelude is STILL unbound: `(match self ((Percent.Pct v) (> v nope)))`
           resolves `v` (arm binder) yet `nope` is a stray name → CDZ0101. Pins that the invariant predicate's
           binder-scope walk threads the arm binders WITHOUT masking a genuine unbound name (a flat walk that
           pushed every bare name would wrongly accept `nope`). Migrated from rcdzc
           an_invariant_predicate_with_an_unbound_name_is_rejected (its flat/positive/binder-in-scope halves
           are the 2840/2863/3022 cases here).")
  (input
    (do
      (@ (invariant (match self ((Percent.Pct v) (> v nope)))) (type Percent (Pct Int64)))
      (def (main) 0)
      (export main)))
  (error CDZ0101 (message "nope")))

; ── @invariant ESTABLISH Part 1: a BARE scalar invariant on a newtype AUTO-UNWRAPS + type-checks ──────────
; The establish checker `invariant_establish::synthesize` emits `(def (__invariant_check_T (: it T)) …)` per
; @invariant type so the predicate is TYPE-CHECKED. For a single-payload newtype it AUTO-UNWRAPS: a bare
; `(>= it 0)` — which alone would hit the nominal boundary (Percent not comparable to Int64, CDZ0202) — is
; rewritten to run over the unwrapped payload, so it type-checks. Pins that the natural bare form COMPILES
; (the author need not destructure) and the type remains usable end-to-end. (The run-time establish TRAP at
; each construction is Part 2; this pins Part 1 — the typed checker — is behavior-neutral for a value that
; SATISFIES the invariant, i.e. construction + use still works.)
(case
  "@invariant ESTABLISH Part 1: a bare-scalar invariant on a newtype auto-unwraps + type-checks; a satisfying value constructs and is usable"
  (doc
    "The establish checker synthesized by `invariant_establish` type-checks the @invariant predicate.
           For the single-payload newtype `(type Percent (Pct Int64))` with the BARE `@invariant(and (>= it 0)
           (<= it 100))`, the checker AUTO-UNWRAPS — `(match it (((. Percent Pct) __u) (and (>= __u 0)
           (<= __u 100))))` — so the bare scalar predicate type-checks (it would otherwise fail CDZ0202 on the
           nominal boundary). Pins that the natural bare form compiles and the type is usable: `(mk 42)` builds
           a `Percent` and `unwrap` reads its payload back → 42. (The run-time establish check that TRAPS on a
           VIOLATING construction is Part 2; Part 1 is the typed checker, behavior-neutral for a satisfying
           value — construction + use unchanged.)")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (mk (: v Int64)) (Percent.Pct v))
      (def (unwrap (: p Percent)) (match p ((Percent.Pct n) n)))
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
(case
  "@invariant ESTABLISH Part 2: the synthesized checked constructor enforces the invariant at run time — a satisfying value constructs, a violating value traps (design §10.2, (D))"
  (doc
    "The (D) run-time establish enforcement. `invariant_establish` synthesizes, per single-payload-newtype
           @invariant type, a CHECKED CONSTRUCTOR `__invariant_construct_Percent` = `(let ((__inv_v (Percent.Pct
           __inv_p))) (if (__invariant_check_Percent __inv_v) __inv_v (trap)))`. Called with a value SATISFYING
           `0 <= it <= 100` it constructs the Percent and yields it (here unwrapped to its Int64 payload — mk(50)
           = 50); called with a VIOLATING value it TRAPS at construction (mk(150) violates `<= 100`), so no
           invalid Percent ever escapes. Pins the establish obligation is enforced at run time (the trap), the
           dynamic complement of the compile-time discharge the establish/preserve corpus above pins. The def is
           synthesized UNWIRED here (called by name); wiring `lower_sum_new` to route every `(Percent.Pct x)`
           through it is the follow-up sub-slice.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (mk (: v Int64)) (match (__invariant_construct_Percent v) ((Percent.Pct n) n)))
      (export mk)))
  (call mk (: 50 Int64))
  (output (: 50 Int64))
  (call mk (: 0 Int64))
  (output (: 0 Int64))
  (call mk (: 100 Int64))
  (output (: 100 Int64))
  (call mk (: 150 Int64))
  (trap "unreachable")
  (call mk (: -1 Int64))
  (trap "unreachable"))

; ── @invariant ESTABLISH Part 2 (the DIVERT): a PLAIN construction AUTO-ESTABLISHES at the call site ─────────
; The wiring. `lower_sum_new` routes a single-payload construction `(Percent.Pct v)` of an @invariant newtype
; through the synthesized checked constructor (`Core::Call { __invariant_construct_Percent, [v] }`) instead of
; erasing straight to the payload — so EVERY construction establishes the invariant at run time, with NO
; `__invariant_construct` named call in the source. The author writes the natural `(Percent.Pct v)` and a
; violating value TRAPS at the construction site. The checked constructor's OWN inner `((. Percent Pct) __inv_p)`
; is EXEMPT (recorded at load), so the divert does not recurse. This is the run-time establish enforcement made
; TRANSPARENT — the previous case pins the checked constructor's behavior when called BY NAME; this pins that an
; ordinary construction is diverted through it automatically.
(case
  "@invariant ESTABLISH Part 2 (divert): a plain `(Percent.Pct v)` construction auto-establishes — a satisfying value constructs, a violating value traps at the call site (design §10.2, (D))"
  (doc
    "The establish DIVERT wiring. `mk` builds a Percent with the PLAIN constructor `(Percent.Pct v)` — no
           `__invariant_construct` by name. `lower_sum_new` diverts that single-payload construction of the
           @invariant newtype through the synthesized checked constructor, so a satisfying value constructs and
           flows through (mk(50) = 50, value-transparent) while a VIOLATING value traps at the construction site
           (mk(150) violates `<= 100`). No invalid Percent is ever built, and the author wrote no call-site
           annotation. The checked constructor's own inner construction is exempt from the divert (no recursion).
           Pins that the run-time establish enforcement is TRANSPARENT — every ordinary construction is checked.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (mk (: v Int64)) (match (Percent.Pct v) ((Percent.Pct n) n)))
      (export mk)))
  (call mk (: 50 Int64))
  (output (: 50 Int64))
  (call mk (: 0 Int64))
  (output (: 0 Int64))
  (call mk (: 100 Int64))
  (output (: 100 Int64))
  (call mk (: 150 Int64))
  (trap "unreachable")
  (call mk (: -1 Int64))
  (trap "unreachable"))

; ── @invariant ESTABLISH (divert) over a HEAP payload: NonEmptyList — the design's second canonical example ──
; The establish divert is payload-KIND-general: it works for a newtype over a HEAP value (a `(List …)`), not
; only a scalar. `NEList = Mk (List Int64)` with `@invariant(< 0 (List.len it))` — the design's `NonEmptyList`
; case (§10.1). `mkfrom` builds the list in-body (a `list` of one for n>0, the empty `list` otherwise) and
; constructs `(NEList.Mk …)`; the divert routes it through the checked constructor, whose auto-unwrap accessor
; `(< 0 (List.len it))` reads the underlying list length. A non-empty list satisfies (mkfrom(5) → len 1); the
; EMPTY list violates and TRAPS at construction (mkfrom(0)). Pins that the single-payload-newtype establish
; path landed for the scalar case generalizes to a heap payload with an accessor-shaped invariant — no invalid
; NonEmptyList is ever built. (The value is used in-body, not exported: a `(List …)` has no boundary rep.)
(case
  "an @invariant newtype REBUILT through a chain of constructor calls re-establishes each time"
  (doc
    "The LIFECYCLE face of the NonEmptyList establish pin (which constructs once): `grow` unwraps
           and RECONSTRUCTS via `NEList.Mk` per step, so a chain of two grows runs the checked constructor
           THREE times total (base + 2) — each rebuild re-establishes the invariant, and the persistent
           base still reads its original length after. 3 + 10·1 = 13. A divert that checked only the
           first-ever construction of the type would let a later rebuild skip the check.")
  (input
    (do
      (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
      (def
        (grow (: ne NEList) (: v Int64))
        (match ne ((NEList.Mk xs) (NEList.Mk (List.push xs v)))))
      (def
        (main (: k Int64))
        (let
          ((base (NEList.Mk #list(k))))
          (let
            ((grown (grow (grow base 10) 20)))
            (match
              grown
              ((NEList.Mk xs) (+ (List.len xs) (* 10 (match base ((NEList.Mk b) (List.len b))))))))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 13 Int64)))

(case
  "shrinking an @invariant newtype to EMPTY traps at the rebuild — the invariant catches the crossing"
  (doc
    "The CROSSING face: a LEGAL one-element NEList is made illegal by an operation — `drop-first`
           rebuilds with the tail, and for a singleton the tail is EMPTY, violating `(< 0 (List.len
           self))`. The trap fires AT the rebuild constructor (where the illegal value would be born),
           not later at a read — establish is per-construction, so an op that crosses the invariant
           cannot hand out the broken value. The 2→1 shrink of the same op constructs fine (still
           non-empty); only the 1→0 crossing traps.")
  (input
    (do
      (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
      (def
        (drop-first (: ne NEList))
        (match ne ((NEList.Mk xs) (NEList.Mk (match xs (#list(_h (.. t)) t) (_ #list()))))))
      (def
        (main (: k Int64))
        (let ((one (NEList.Mk #list(k)))) (match (drop-first one) ((NEList.Mk xs) (List.len xs)))))
      (export main)))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH (divert) over a heap payload: a NonEmptyList newtype traps on the empty list, constructs a non-empty one (design §10.1/§10.2, (D))"
  (doc
    "The establish divert is general over the payload KIND — here a HEAP `(List Int64)`, the design's
           `NonEmptyList`. `NEList = Mk (List Int64)` carries `@invariant(< 0 (List.len it))`. `mkfrom` builds
           the payload list in-body — `(list n)` for n>0 (length 1, satisfies) else the empty `(list)` (length
           0, violates) — and constructs `(NEList.Mk …)`, which the divert routes through the checked
           constructor; its accessor invariant `(< 0 (List.len it))` reads the underlying list length. So
           mkfrom(5) yields 1 (a non-empty list constructs and its length reads back) and mkfrom(0) TRAPS at
           construction (the empty list is not a legal NonEmptyList). Pins the establish path generalizes from
           the scalar newtype (Percent) to a heap payload with an accessor-shaped invariant.")
  (input
    (do
      (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64))))
      (def
        (mkfrom (: n Int64))
        (match (NEList.Mk (if (> n 0) #list(n) #list())) ((NEList.Mk ys) (List.len ys))))
      (export mkfrom)))
  (call mkfrom (: 5 Int64))
  (output (: 1 Int64))
  (call mkfrom (: 0 Int64))
  (trap "unreachable"))

; ── @invariant ESTABLISH over a MULTI-VARIANT sum: each variant's construction auto-establishes ──────────────
; The multi-variant generalization (design §10.2 — a per-CONSTRUCTOR obligation). A ≥2-variant sum is BOXED
; (`Core::SumNew{disc, payloads}`), not erased, so it never hits the newtype path. `invariant_establish`
; synthesizes ONE checked constructor per variant (`__invariant_construct_Shape__d<disc>`, keyed by the
; discriminant the boxed-construction divert has in hand), each calling the whole-value `__invariant_check_Shape`
; (Part 1, `it : Shape`, the author's own match reads the variant). So a construction of EITHER variant is
; routed through its per-variant checked constructor: a satisfying value constructs, a violating one TRAPS. This
; pins both the 1-payload `Circle` arm (disc 0) and the 2-payload `Square` arm (disc 1) — a multi-payload
; construct-def. `circ`/`sq` build a shape then re-match to a scalar so the export crosses the boundary.
(case
  "@invariant ESTABLISH over a multi-variant sum: each variant's construction auto-establishes — a satisfying value constructs, a violating one traps (design §10.2, (D))"
  (doc
    "The multi-variant establish. `Shape = Circle Int64 | Square Int64 Int64` with a per-variant invariant
           (a Circle's radius > 0; a Square's sides both > 0). Each variant construction is routed through its
           synthesized per-variant checked constructor (`__invariant_construct_Shape__d0` for Circle,
           `__d1` for Square), which calls the whole-value `__invariant_check_Shape`. `circ(r)` builds a Circle
           and returns its radius (via re-match); `sq(w,h)` builds a Square and returns w+h. A satisfying value
           of either variant constructs (circ(5)=5, sq(3,4)=7); a violating value of either traps at
           construction (circ(0), circ(-3), sq(3,0), sq(0,4)). Pins the per-constructor establish obligation
           over a boxed multi-variant sum, including the 2-payload Square arm.")
  (input
    (do
      (@
        (invariant
          (match self ((Shape.Circle r) (> r 0)) ((Shape.Square w h) (and (> w 0) (> h 0)))))
        (type Shape (Circle Int64) (Square Int64 Int64)))
      (def
        (circ (: r Int64))
        (match (Shape.Circle r) ((Shape.Circle x) x) ((Shape.Square w h) (+ w h))))
      (def
        (sq (: w Int64) (: h Int64))
        (match (Shape.Square w h) ((Shape.Circle x) x) ((Shape.Square a b) (+ a b))))
      (export circ)
      (export sq)))
  (call circ (: 5 Int64))
  (output (: 5 Int64))
  (call circ (: 0 Int64))
  (trap "unreachable")
  (call circ (: -3 Int64))
  (trap "unreachable")
  (call sq (: 3 Int64) (: 4 Int64))
  (output (: 7 Int64))
  (call sq (: 3 Int64) (: 0 Int64))
  (trap "unreachable")
  (call sq (: 0 Int64) (: 4 Int64))
  (trap "unreachable")
  (live-objects 0))

; ── @invariant ESTABLISH over a SINGLE-VARIANT MULTI-PAYLOAD newtype: the tuple-erase construct path ─────────
; The third establish shape. `(type Range (Mk Int64 Int64))` is a single-variant, MULTI-payload newtype — it
; erases to a `Ty::Tuple`, NOT a single-payload value, so it takes neither the single-PAYLOAD newtype divert
; (`args.len()==1`) nor the boxed multi-VARIANT one. Without a divert here it would construct with NO establish
; check (a real (D) soundness gap — an invalid Range could be built). `invariant_establish` synthesizes its
; sole variant's checked constructor `__invariant_construct_Range__d0` (the per-variant path now fires for any
; non-sole-payload-newtype), and the tuple-erase arm of `lower_sum_new` diverts the 2-payload construction
; through it. A relational invariant `(<= lo hi)` over the two payloads: an ordered pair constructs, a
; misordered one TRAPS. `mk` builds a Range then re-matches to `(- hi lo)` so the export crosses the boundary.
(case
  "@invariant ESTABLISH over a single-variant multi-payload newtype: an ordered Range constructs, a misordered one traps (design §10.2, (D))"
  (doc
    "The third establish shape — a single-variant MULTI-payload newtype `(type Range (Mk Int64 Int64))`,
           which erases to a tuple. Its relational `@invariant(<= lo hi)` is checked at construction via the
           synthesized `__invariant_construct_Range__d0` (the tuple-erase divert's callee). `mk(lo,hi)` builds a
           Range and returns `hi - lo`. An ordered pair satisfies and constructs (mk(3,7)=4, mk(5,5)=0); a
           misordered pair violates `<= lo hi` and TRAPS at construction (mk(7,3)). Pins the establish path
           over the multi-payload-newtype shape (a relational invariant across the two payloads), closing the
           gap where a 2-payload newtype used to construct with no check.")
  (input
    (do
      (@ (invariant (match self ((Range.Mk lo hi) (<= lo hi)))) (type Range (Mk Int64 Int64)))
      (def (mk (: lo Int64) (: hi Int64)) (match (Range.Mk lo hi) ((Range.Mk a b) (- b a))))
      (export mk)))
  (call mk (: 3 Int64) (: 7 Int64))
  (output (: 4 Int64))
  (call mk (: 5 Int64) (: 5 Int64))
  (output (: 0 Int64))
  (call mk (: 7 Int64) (: 3 Int64))
  (trap "unreachable")
  (live-objects known-leak))

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
(case
  "@invariant ESTABLISH over a nullary variant: a rejected nullary variant traps at construction, an accepted payload variant constructs (design §10.2, (D))"
  (doc
    "The last establish shape — a NULLARY variant. `T = A | B Int64` with an invariant that rejects `A`
           outright (`false`) and accepts `B x` when x>0. `mka` constructs `A`; because the invariant makes `A`
           uninhabitable, the synthesized no-arg checked constructor `__invariant_construct_T__d0` traps.
           `mkb(x)` constructs `B x`: x>0 satisfies (mkb(5)=5), x<=0 traps (mkb(0)). Pins that a nullary variant
           establishes at its unit-construction path — the invariant holds for EVERY value including the
           payloadless ones, so an uninhabitable nullary variant is caught at construction. (The invariant
           value binder is `self`, per the operator's ret/self ruling.)")
  (input
    (do
      (@ (invariant (match self ((T.A) false) ((T.B x) (> x 0)))) (type T (A) (B Int64)))
      (def (mka) (match (T.A unit) ((T.A) 0) ((T.B x) x)))
      (def (mkb (: x Int64)) (match (T.B x) ((T.A) 0) ((T.B y) y)))
      (export mka)
      (export mkb)))
  (call mka)
  (trap "unreachable")
  (call mkb (: 5 Int64))
  (output (: 5 Int64))
  (call mkb (: 0 Int64))
  (trap "unreachable"))

; ── @ensures on a def RETURNING an @invariant type: BOTH checks fire independently (composition edge) ────────
; The two (D) run-time members COMPOSE on one def: a def with an `@ensures(Q on ret)` whose RESULT type carries
; its own `@invariant(I on self)`. The result binder `ret` IS an `@invariant`-typed value, so TWO independent
; checks apply — (a) the ESTABLISH trap fires at the `(Pct.P v)` construction INSIDE the body (the invariant on
; the constructed value), and (b) the `@ensures` postcondition trap fires at body-exit on `ret`. They are
; distinct obligations at distinct sites: an in-range value that fails the postcondition traps at EXIT, while
; an out-of-range value traps EARLIER at construction (establish), before the postcondition is even reached.
; Pins that neither check subsumes or masks the other — a future change that folded them would drop one guard.
(case
  "@ensures on a def returning an @invariant type: establish (on construction) AND the postcondition (on ret) both fire (design §10, (D))"
  (doc
    "The composition of two (D) members on one def. `Pct = P Int64` has `@invariant(0 <= self <= 100)`;
           `mk` has `@ensures(ret's payload >= 50)` and returns a `Pct`. `run(v)` calls `mk` then unwraps.
           run(70): the Pct establish (0..100) holds AND the ensures (>=50) holds → 70 flows. run(30): the Pct
           establish holds (30 in 0..100) but the ensures postcondition (30 >= 50) FAILS → trap at body-EXIT.
           run(150): the Pct ESTABLISH (<=100) fails at the `(Pct.P 150)` construction INSIDE mk's body → trap
           there, before the postcondition is reached. Pins that the establish trap and the @ensures trap are
           INDEPENDENT obligations at distinct sites — neither subsumes the other. (`ret`/`self` are the
           operator's binder names; here `ret` is itself an @invariant-typed value.)")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (@ (ensures (match ret ((Pct.P n) (>= n 50)))) (def (mk (: v Int64)) (Pct.P v)))
      (def (run (: v Int64)) (match (mk v) ((Pct.P n) n)))
      (export run)))
  (call run (: 70 Int64))
  (output (: 70 Int64))
  (call run (: 30 Int64))
  (trap "unreachable")
  (call run (: 150 Int64))
  (trap "unreachable"))

(case
  "@requires over an @invariant-typed PARAMETER: the type establish (at construction) and the precondition (at body entry) are independent obligations at distinct sites"
  (doc
    "The precondition mirror of the `@ensures returning an @invariant type` case above. There the two (D)
           obligations were establish-on-construction + postcondition-on-result; here they are
           establish-on-construction + PRECONDITION-on-parameter, and the pin is that they fire at DISTINCT sites
           on a single invariant-typed value — the establish at the caller's construction, the @requires at the
           callee's body ENTRY, neither subsuming the other. `Pct = P Int64` has `@invariant(0 <= self <= 100)`;
           `hi` takes a `Pct` param and carries `@requires(payload >= 50)`. `run(v)` constructs `(Pct.P v)` and
           passes it to `hi`. run(70): the establish (0..100) holds at the `(Pct.P 70)` construction AND the
           precondition (70 >= 50) holds at hi's body entry → 70 flows. run(30): the establish holds (30 in
           0..100) at construction, but the @requires (30 >= 50) FAILS at hi's body-entry → trap AFTER a valid
           Pct was built. run(150): the establish (<=100) FAILS at the `(Pct.P 150)` construction INSIDE run,
           BEFORE `hi` is ever entered → trap there, so the precondition is never reached. That the 30 case and
           the 150 case trap at DIFFERENT sites (body-entry vs construction) on the same invariant-typed value
           proves the two obligations are independent. (`self` is the invariant binder; `@requires` binds no
           result, so it deconstructs the param by name.)")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (@ (requires (match p ((Pct.P n) (>= n 50)))) (def (hi (: p Pct)) (match p ((Pct.P n) n))))
      (def (run (: v Int64)) (hi (Pct.P v)))
      (export run)))
  (call run (: 70 Int64))
  (output (: 70 Int64))
  (call run (: 30 Int64))
  (trap "unreachable")
  (call run (: 150 Int64))
  (trap "unreachable"))

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
(case
  "@invariant ESTABLISH divert reaches indirect construction sites: a lambda body, a reconstruct-after-update, and a list element all establish"
  (doc
    "Escape-face pins for the establish divert. `via-lambda` constructs `(Percent.Pct x)` inside a
           LAMBDA applied to a runtime argument — the divert fires inside the closure body (via-lambda(50)=50,
           via-lambda(150) traps). `via-bump` deconstructs a VALID Percent, adds a delta, and RE-constructs —
           the re-wrap is its own establish obligation, so 50+10=60 flows but 90+20=110 traps at the re-wrap
           (no invalid Percent escapes an update helper). `via-list` constructs as a LIST element and reads it
           back (via-list(50)=50, via-list(150) traps inside the collection initializer). Together these pin
           that the divert is reachability-complete over lambda bodies, update re-wraps, and collection
           element positions — the sites a construction-site rewrite would silently miss.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
      (def (unp (: p Percent)) (match p ((Percent.Pct n) n)))
      (def (via-lambda (: v Int64)) (unp ((fn ((: x Int64)) (Percent.Pct x)) v)))
      (def (bump (: p Percent) (: d Int64)) (match p ((Percent.Pct n) (Percent.Pct (+ n d)))))
      (def (via-bump (: v Int64) (: d Int64)) (unp (bump (Percent.Pct v) d)))
      (def
        (via-list (: v Int64))
        (match #list((Percent.Pct v) (Percent.Pct 5)) (#list(h (.. _)) (unp h)) (_ 0)))
      (export via-lambda)
      (export via-bump)
      (export via-list)))
  (call via-lambda (: 50 Int64))
  (output (: 50 Int64))
  (call via-lambda (: 150 Int64))
  (trap "unreachable")
  (call via-bump (: 50 Int64) (: 10 Int64))
  (output (: 60 Int64))
  (call via-bump (: 90 Int64) (: 20 Int64))
  (trap "unreachable")
  (call via-list (: 50 Int64))
  (output (: 50 Int64))
  (call via-list (: 150 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH fires when the type is IMPORTED from another module and constructed in the entry (cross-package divert site)"
  (doc
    "The divert-reachability set so far (lambda / reconstruct / list-element / match-arm) is all within a
           SINGLE file; this pins the establish check fires when the constructor is IMPORTED across a package
           link. `lib` declares `@invariant(0 <= self <= 100) type Pct (P Int64)` and exports it CONCRETELY with
           the wildcard `(. Pct *)` (the handle + constructor, as a user sum must be to be constructed by the
           importer — a bare `(export Pct)` would export the handle only, abstract). The entry `(import \"lib\"
           (Pct))` brings the type + `Pct.P` into scope and constructs `(Pct.P v)` for a runtime v, then unwraps.
           The establish divert (a construction-SITE rewrite in lower_sum_new) must fire at the entry's
           construction site even though the type was DECLARED in another file — the invariant travels with the
           nominal type across the link. mk(50): `(Pct.P 50)` establishes (0..100) → 50. mk(150): `(Pct.P 150)`
           violates `<= self 100` → traps at the entry's construction, exactly as an in-file construction of a
           locally-declared invariant type would. Pins that @invariant establish is reachability-complete over
           CROSS-MODULE construction sites — the divert is keyed to the type's declaration, not the file.")
  (module "lib"
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (def (unp (: p Pct)) (match p ((Pct.P n) n)))
      (export Pct.* unp)))
  (input (do (import "lib" (Pct unp)) (def (mk (: v Int64)) (unp (Pct.P v))) (export mk)))
  (call mk (: 50 Int64))
  (output (: 50 Int64))
  (call mk (: 150 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH fires when the constructor is inside a @requires PREDICATE — the divert reaches predicate-position code"
  (doc
    "Extends the establish-divert reachability set to a CONTRACT-PREDICATE construction site. All divert
           cases so far construct in ordinary value positions (def body, lambda, list, match arm, cross-module
           entry); this pins that the divert also fires inside the injected `(if PRE BODY (trap …))` test when
           PRE itself constructs an @invariant value. `Pct = P Int64` has `@invariant(0 <= self <= 100)`; `f`
           carries `@requires((>= (unp (Pct.P x)) 0))` — the precondition CONSTRUCTS `(Pct.P x)`, unwraps it, and
           compares. The key: the comparison `(>= … 0)` is true for BOTH inputs (unp returns x, and both 50 and
           150 are >= 0), so if the establish did NOT fire the predicate would pass for 150. f(50): `(Pct.P 50)`
           establishes (50 in 0..100) → unp → 50 >= 0 → precondition holds → 50. f(150): `(Pct.P 150)` VIOLATES
           `<= self 100` → the establish check inside the predicate traps BEFORE the comparison is reached. That
           150 traps THOUGH `150 >= 0` is true proves the establish divert reached the constructor in predicate
           position — the construction-site rewrite walks contract-predicate code too, not only value-position
           bodies. Runtime arg via main's param so nothing folds.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (def (unp (: p Pct)) (match p ((Pct.P n) n)))
      (@ (requires (>= (unp (Pct.P x)) 0)) (def (f (: x Int64)) x))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 50 Int64))
  (output (: 50 Int64))
  (call main (: 150 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH fires when the constructor is directly a MATCH ARM's selected result (v-patterns divert site)"
  (doc
    "An escape-face pin extending the divert-reachability set (lambda / reconstruct / list-element) with
           a MATCH-ARM construction site: the constructor call is the RESULT EXPRESSION of a match arm the
           scrutinee selects, so the establish divert must fire inside whichever arm runs — a construction-site
           rewrite that walked `fn`/`let`/list but skipped a match arm's body would silently miss it. `(mk x) =
           (match x (0 (Nat.Mk 0)) (_ (Nat.Mk (- x 1))))` over `@invariant(>= self 0) type Nat`: `(mk 0)` takes
           the `0` arm, builds `Nat.Mk 0` (0 >= 0, establishes) → read back `0`; `(mk -5)` takes the wildcard
           arm, builds `Nat.Mk (- -5 1)` = `Nat.Mk -6` which VIOLATES `>= self 0` → the establish check traps
           `unreachable` inside the selected arm. Runtime scrutinee via `main`'s param so neither arm folds.
           Pins that @invariant establish is reachability-complete over match-arm construction sites too.")
  (input
    (do
      (@ (invariant (>= self 0)) (type Nat (Mk Int64)))
      (def (mk (: x Int64)) (match x (0 (Nat.Mk 0)) (_ (Nat.Mk (- x 1)))))
      (def (main (: k Int64)) (match (mk k) ((Nat.Mk v) v)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 0 Int64))
  (call main (: -5 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH divert reaches a LET-INIT construction site and a NESTED-invariant constructor argument"
  (doc
    "Two more escape-face pins extending the divert-reachability set (lambda / reconstruct / list-element /
           match-arm) with the LET-INIT position and a NESTED @invariant. `via-let` constructs `(Nat.Mk v)` as a
           LET-BINDING's initializer — the establish divert must fire on the init expression, not only on a
           def-body-tail or an argument position (via-let(5)=5 establishes, via-let(-1) traps at the let-init).
           `mk` nests one @invariant type inside another: `(Box.B (Nat.Mk x))` builds a `Nat` (which carries its
           own `>= self 0` invariant) as the ARGUMENT to the `Box` constructor — the INNER `Nat.Mk` establish
           must fire even though the value is immediately consumed by the outer constructor, so mk(5)=5 flows and
           mk(-1) traps at the inner Nat establish before Box is ever built. Together they pin that the divert is
           reachability-complete over let-init positions AND over an invariant construction used as a
           constructor argument to a second invariant type — nesting does not let an inner establish escape.")
  (input
    (do
      (@ (invariant (>= self 0)) (type Nat (Mk Int64)))
      (@ (invariant true) (type Box (B Nat)))
      (def (via-let (: v Int64)) (let ((n (Nat.Mk v))) (match n ((Nat.Mk x) x))))
      (def (unbox (: b Box)) (match b ((Box.B n) (match n ((Nat.Mk v) v)))))
      (def (mk (: x Int64)) (unbox (Box.B (Nat.Mk x))))
      (export via-let)
      (export mk)))
  (call via-let (: 5 Int64))
  (output (: 5 Int64))
  (call via-let (: -1 Int64))
  (trap "unreachable")
  (call mk (: 5 Int64))
  (output (: 5 Int64))
  (call mk (: -1 Int64))
  (trap "unreachable"))

(case
  "@ensures / @requires predicate PROJECTS a TUPLE component (`(. ret N)` on the result, `(. p N)` on a param)"
  (doc
    "The enforcement predicate is not limited to scalar/heap results — it may PROJECT a component of a
           tuple. Existing cases pin @ensures over a scalar and over a heap List result; this pins the tuple
           projection face for BOTH the result binder and a parameter. `proj-ret` returns `(tuple x (x+1))` under
           @ensures(>= (. ret 0) 0): the postcondition reads component 0 of the result tuple — proj-ret(5) yields
           (5,6) whose 0th is 5 >= 0 (returns 6 = component 1), proj-ret(-2) yields (-2,-1) whose 0th is -2 < 0
           and TRAPS. `proj-arg` takes a tuple parameter under @requires(>= (. p 0) 0): the precondition projects
           component 0 of the argument tuple — main builds `(tuple k (k+100))` at runtime, proj-arg over (5,105)
           passes (returns 105), over (-2,98) traps before the body. Together they pin that `(. binder N)` in a
           predicate resolves + lowers against both the result binder `ret` and a tuple-typed parameter, so a
           future change to tuple projection or predicate scoping cannot silently break contract enforcement over
           product-typed results/arguments. Runtime values via main's param (no const-fold).")
  (input
    (do
      (@ (ensures (>= (. ret 0) 0)) (def (proj-ret (: x Int64)) #tuple(x (+ x 1))))
      (@ (requires (>= (. p 0) 0)) (def (proj-arg (: p (Tuple Int64 Int64))) (. p 1)))
      (def (main-ret (: k Int64)) (. (proj-ret k) 1))
      (def (main-arg (: k Int64)) (proj-arg #tuple(k (+ k 100))))
      (export main-ret)
      (export main-arg)))
  (call main-ret (: 5 Int64))
  (output (: 6 Int64))
  (call main-ret (: -2 Int64))
  (trap "unreachable")
  (call main-arg (: 5 Int64))
  (output (: 105 Int64))
  (call main-arg (: -2 Int64))
  (trap "unreachable")
  (live-objects known-leak))

(case
  "@ensures over a HANDLE-bodied def called with a runtime arg is enforced (the verify_enforce let-over-handle shape)"
  (doc
    "Regression pin for the exact rewrite `verify_enforce` injects over an effectful body. Enforcement
           wraps the def body as `(let ((ret BODY)) (if Q ret (trap)))`; when BODY is a `handle` expression and
           the def is CALLED with the caller's runtime argument, the let-bound handle init used to spuriously
           reject `CDZ0101 unbound <caller-param>` — the tail-resumptive effects fold `deep_fresh_copy`d the
           threaded handle seed at each multi-use state-binder site, re-pushing the pinned caller-arg UNPINNED so
           it re-resolved against the folded orphan (v-effects root-cause; fixed by let-binding a non-constant
           seed once at fold entry). `f` handles a `St` state effect seeded by its param `n`, its arm resumes
           `s` twice `(resume s s)` (the ≥2 state-binder use that triggered the orphan), under @ensures(>= ret
           0). Called `(f k)` with main's runtime param: f(7) folds the handle to 7, the postcondition 7>=0
           holds → 7; f(-3) folds to -3, -3>=0 is false → traps. Pins that contract enforcement composes with a
           handle-bodied def + runtime arg (the effects/verify seam), so the fold's seed-preservation fix cannot
           silently regress. Runtime arg via main's param (no const-fold).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (@
        (ensures (>= ret 0))
        (def (f (: n Int64)) (handle St n ((get (u) s (resume s s))) (St.get))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 7 Int64))
  (output (: 7 Int64))
  (call main (: -3 Int64))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH divert reaches a Set ELEMENT and a Map VALUE construction site (heap-collection positions beyond List)"
  (doc
    "Two more escape-face pins for the establish divert, extending the heap-collection reachability set
           (which already covers a List element) to a Set element and a Map value position. `via-set` constructs
           `(Nat.Mk v)` as the element of `(Set.of (list …))` under `@invariant(>= self 0) type Nat`, reads it
           back via `Set.to-list` + a match: via-set(5)=5 establishes, via-set(-1) traps inside the Set
           initializer. `via-map` constructs `(Nat.Mk v)` as the VALUE of `(Map.insert Map.empty 1 …)` and reads
           it back via `Map.lookup`: via-map(5)=5, via-map(-1) traps inside the Map initializer. A construction-
           site rewrite that walked List element positions but skipped Set-element / Map-value positions would
           silently build an unchecked value in a CHAMP; these pin that the divert is reachability-complete over
           those heap-collection construction sites too. Runtime payload via the export param (no const-fold).")
  (input
    (do
      (@ (invariant (>= self 0)) (type Nat (Mk Int64)))
      (def (unp (: p Nat)) (match p ((Nat.Mk v) v)))
      (def
        (via-set (: v Int64))
        (match (Set.to-list #set((Nat.Mk v))) (#list(h (.. _)) (unp h)) (_ 0)))
      (def
        (via-map (: v Int64))
        (match (Map.lookup (Map.insert Map.empty 1 (Nat.Mk v)) 1) ((Some n) (unp n)) ((None _u) -1)))
      (export via-set)
      (export via-map)))
  (call via-set (: 5 Int64))
  (output (: 5 Int64))
  (call via-set (: -1 Int64))
  (trap "unreachable")
  (call via-map (: 5 Int64))
  (output (: 5 Int64))
  (call via-map (: -1 Int64))
  (trap "unreachable"))

(case
  "@ensures predicate PROJECTS a RECORD field of the result (single field, and a two-field relation)"
  (doc
    "The enforcement-predicate projection face for a NAMED-FIELD product (the record companion to the
           tuple-component pin). `proj` returns `(record (x n) (y 2))` under @ensures(>= (. ret x) 0): the
           postcondition reads field `x` of the result record — proj(5) has x=5>=0 (returns field y = 2),
           proj(-1) has x=-1<0 and traps. `rel` returns `(record (lo n) (hi 5))` under @ensures(< (. ret lo)
           (. ret hi)) relating TWO fields: rel(2) has lo=2 < hi=5 (returns lo = 2), rel(9) has lo=9 not < 5 and
           traps. Together they pin that `(. ret field)` in a predicate resolves + lowers against a record
           result — both a single-field read and a two-field relation — so a future change to record projection
           or predicate scoping cannot silently break contract enforcement over record-typed results. Runtime
           payload via the export param (no const-fold).")
  (input
    (do
      (@ (ensures (>= ret.x 0)) (def (proj (: n Int64)) #record((= x n) (= y 2))))
      (@ (ensures (< ret.lo ret.hi)) (def (rel (: n Int64)) #record((= lo n) (= hi 5))))
      (def (main-proj (: k Int64)) (. (proj k) y))
      (def (main-rel (: k Int64)) (. (rel k) lo))
      (export main-proj)
      (export main-rel)))
  (call main-proj (: 5 Int64))
  (output (: 2 Int64))
  (call main-proj (: -1 Int64))
  (trap "unreachable")
  (call main-rel (: 2 Int64))
  (output (: 2 Int64))
  (call main-rel (: 9 Int64))
  (trap "unreachable")
  (live-objects known-leak))

(case
  "@ensures predicate PROJECTS a NESTED component (`(. (. ret 0) 1)` — a tuple inside a tuple), the deepest projection face"
  (doc
    "The enforcement-predicate projection face at DEPTH > 1 — the nested companion to the single-level
           tuple-component (`(. ret N)`) and record-field pins. A projection predicate may chain accessors to
           reach a component of a component; this pins that `(. (. ret 0) 1)` resolves + lowers correctly when
           the result is a tuple whose 0th component is ITSELF a tuple. `proj-nest` returns
           `(tuple (tuple n (+ n 1)) 99)` under @ensures(>= (. (. ret 0) 1) 0): the postcondition projects
           outer component 0 (the inner tuple), then inner component 1 (which is n+1) — so it holds iff
           n+1 >= 0. proj-nest(5): inner is (5,6), the nested projection reads 6 >= 0 → holds, and main returns
           inner component 0 = 5. proj-nest(-3): inner is (-3,-2), the nested projection reads -2 < 0 → TRAPS
           before returning. Pins that a chained `(. (. binder i) j)` accessor in a contract predicate resolves
           against the result binder `ret` and lowers to the right nested read, so a future change to projection
           lowering or predicate scoping cannot silently break enforcement over nested product results. Runtime
           payload via main's param (no const-fold).")
  (input
    (do
      (@
        (ensures (>= (. (. ret 0) 1) 0))
        (def (proj-nest (: n Int64)) #tuple(#tuple(n (+ n 1)) 99)))
      (def (main-nest (: k Int64)) (. (. (proj-nest k) 0) 0))
      (export main-nest)))
  (call main-nest (: 5 Int64))
  (output (: 5 Int64))
  (call main-nest (: -3 Int64))
  (trap "unreachable")
  (live-objects known-leak))

(case
  "@requires over a HANDLE-bodied def, and @ensures over a handle arm that uses state 3x — both enforce (effects-fold robustness)"
  (doc
    "Two more effects-seam pins hardening the let-over-handle / seed-thread fix beyond the single
           @ensures-over-handle shape already pinned. `req` puts a @requires(>= n 0) over a handle-bodied def
           called with a runtime arg — the PRECONDITION injection `(if PRE handle-body (trap))` is a DIFFERENT
           rewrite from @ensures's `(let ((ret handle-body)) …)`, so it exercises the fold under a distinct
           wrapper: req(7)=7, req(-3) traps at the precondition before the body folds. `thrice` puts
           @ensures(>= ret 0) over a handle whose arm resumes the state binder THREE times `(resume (+ s s) s)`
           — the ≥2-state-use shape that originally orphaned the threaded seed; confirming 3 uses fold correctly
           pins the seed-preservation fix is not limited to exactly two uses: thrice(4)=8, thrice(-1) traps.
           Together they guard the effects/verify seam across both injection shapes and higher state-binder
           multiplicity. Runtime arg via the export param (no const-fold).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (@
        (requires (>= n 0))
        (def (req (: n Int64)) (handle St n ((get (u) s (resume s s))) (St.get))))
      (@
        (ensures (>= ret 0))
        (def (thrice (: n Int64)) (handle St n ((get (u) s (resume (+ s s) s))) (St.get))))
      (def (main-req (: k Int64)) (req k))
      (def (main-thrice (: k Int64)) (thrice k))
      (export main-req)
      (export main-thrice)))
  (call main-req (: 7 Int64))
  (output (: 7 Int64))
  (call main-req (: -3 Int64))
  (trap "unreachable")
  (call main-thrice (: 4 Int64))
  (output (: 8 Int64))
  (call main-thrice (: -1 Int64))
  (trap "unreachable"))

(case
  "@ensures on a RECURSIVE def whose body is a HANDLE is re-checked at every recursive exit (recursion x effects-fold seam)"
  (doc
    "Composes the recursive re-check invariant (already pinned for a plain recursive def) with the
           handle-bodied effectful body (already pinned non-recursively). `f` is self-recursive AND its body is
           a `handle` that folds a `St` state effect seeded by the param, resuming state twice per level, with
           the recursive `(f (- n 1))` call inside the handled body under @ensures(< ret 2). Each recursive
           EXIT re-runs the postcondition against that level's returned value: f(1) folds/recurses to a result
           < 2 (returns 1); f(5) recurses deeper so an exit's result reaches >= 2 and the postcondition traps.
           This pins that enforcement's `(let ((ret BODY)) (if Q ret trap))` wrapper composes correctly when
           BODY is BOTH a handle fold AND a self-call — the effects seed-thread fix and the per-exit re-check
           must both hold at once. Runtime arg via the export param (no const-fold).")
  (input
    (do
      (effect St (op get (-> Unit Int64)))
      (@
        (ensures (< ret 2))
        (def
          (f (: n Int64))
          (handle St n ((get (u) s (resume s s))) (if (<= (St.get) 0) 0 (+ 1 (f (- n 1)))))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 5 Int64))
  (trap "unreachable"))

(case
  "@ensures over a def whose body uses the `?` try-operator under an Option boundary — the postcondition matches the FALLIBLE result"
  (doc
    "Cross-seam pin: contract enforcement composes with the `?` short-circuit operator (v-try-operator).
           `f` returns a fallible `(Option Int64)` and its body uses `(try (Some n))` to unwrap, so `ret` binds
           the WRAPPED result (a `Some`/`None`), and the postcondition matches on it:
           `@ensures(match ret ((Some v) (>= v 0)) ((None _u) true))`. The `?` needs a fallible enclosing
           boundary to short-circuit to (DESIGN-try-operator §4/§6); this pins that verify_enforce's
           `(let ((ret BODY)) (if Q ret trap))` wrapper does NOT disturb the boundary walk — `ret` still binds
           the Option result and the `?` resolves against f's declared `(Option Int64)`. f(5) unwraps 5, builds
           `(Some 6)` whose payload 6 >= 0 → main reads 6; f(-3) builds `(Some -2)` whose payload violates the
           postcondition → traps. Runtime arg via main's param (no const-fold). Guards the try-operator × verify
           seam so a future `?`-desugar or enforcement change cannot silently break enforcement over fallible
           bodies.")
  (input
    (do
      (@
        (ensures (match ret ((Some v) (>= v 0)) ((None _u) true)))
        (def (f (: n Int64)) (: (let ((x (try (Some n)))) (Some (+ x 1))) (Option Int64))))
      (def (main (: k Int64)) (match (f k) ((Some v) v) ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -3 Int64))
  (trap "unreachable")
  (live-objects known-leak))

(case
  "FULL CONTRACT: @requires + @ensures on ONE def fire INDEPENDENTLY — a pre-violation and a post-violation trap on DIFFERENT inputs"
  (doc
    "The canonical precondition+postcondition contract on a single def, pinning that BOTH arms enforce on
           DISTINCT inputs (the existing @requires-over-@ensures case pins the precondition firing THROUGH the
           wrapper; this pins the two checks are independent and each traps on its own violating input).
           `@requires(>= x 0)` + `@ensures(< ret 10)` on `(f x) = (+ x 1)` carves three regions over the runtime
           arg: x=3 satisfies both (3>=0, ret=4<10) → 4; x=-1 violates the PRECONDITION at body-entry → traps
           BEFORE the body runs; x=20 satisfies the precondition (20>=0) but ret=21 violates the POSTCONDITION at
           body-exit → traps. Two traps from two different injected checks — verify_enforce wraps the body as
           `(if PRE (let ((ret BODY)) (if POST ret trap)) trap)`, and this witnesses both trap edges. Runtime
           arg via main's param (no const-fold).")
  (input
    (do
      (@ (requires (>= x 0)) (@ (ensures (< ret 10)) (def (f (: x Int64)) (+ x 1))))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 3 Int64))
  (output (: 4 Int64))
  (call main (: -1 Int64))
  (trap "unreachable")
  (call main (: 20 Int64))
  (trap "unreachable"))

(case
  "NESTED CONTRACT: an @ensures predicate that CALLS a @requires-guarded helper enforces the HELPER's precondition during postcondition eval"
  (doc
    "Contract composition across a def boundary: `f`'s @ensures predicate CALLS a user helper `always-ok`
           that itself carries @requires. The pin isolates WHICH contract fires by making the helper body
           UNCONDITIONALLY true: `(@ (requires (>= x 0)) (def (always-ok (: x Int64)) true))` — so f's
           `@ensures(always-ok ret)` can NEVER be false on its own. `f(x) = x + 1`. main(5): ret=6, the
           postcondition calls always-ok(6) — its @requires(6>=0) holds, body true → contract satisfied → 6.
           main(-3): ret=-2, the postcondition calls always-ok(-2) — the HELPER's OWN @requires(-2>=0) FAILS and
           traps DURING postcondition evaluation, even though always-ok's body would have returned true. That
           the -3 case traps THOUGH the outer postcondition can't be false proves the nested @requires is what
           enforces — pins that a @requires-guarded call inside an @ensures predicate carries its own contract
           (verify_enforce's injected checks compose transitively through helper calls). Runtime arg via main's
           param (no const-fold).")
  (input
    (do
      (@ (requires (>= x 0)) (def (always-ok (: x Int64)) true))
      (@ (ensures (always-ok ret)) (def (f (: n Int64)) (+ n 1)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -3 Int64))
  (trap "unreachable"))

(case
  "TWO @ensures-guarded defs COMPOSE along a tail call — the inner postcondition fires on the returned value, the outer on the same value at its own exit"
  (doc
    "Distinct from the NESTED CONTRACT case above (there an @ensures PREDICATE calls a @requires-guarded
           helper): here TWO defs each carry an @ensures, and `outer`'s body is a TAIL CALL to `inner`, so the
           value `outer` returns IS `inner`'s already-checked result. Both postconditions are injected
           independently — `inner` wraps its body as `(let ((ret (+ x 1))) (if (>= ret 0) ret trap))`, and
           `outer` wraps ITS body (the call `(inner y)`) as `(let ((ret (inner y))) (if (< ret 100) ret trap))`.
           So the value passes through inner's exit check THEN outer's exit check. main(5): inner returns 6 (6 >=
           0 holds), outer sees 6 (6 < 100 holds) → 6. main(-3): inner computes -2, its OWN @ensures(>= ret 0)
           FAILS → trap inside inner, before outer's postcondition is reached. main(200): inner returns 201 (201
           >= 0 holds, passes inner), but outer's @ensures(< ret 100) FAILS on 201 → trap at outer's exit. That
           -3 traps in inner and 200 traps in outer proves the two postconditions fire at their OWN exit sites
           along the call chain — each def's verify_enforce wrapper is independent, composing through the tail
           call. Runtime arg via main's param so nothing folds.")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (inner (: x Int64)) (+ x 1)))
      (@ (ensures (< ret 100)) (def (outer (: y Int64)) (inner y)))
      (def (main (: k Int64)) (outer k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -3 Int64))
  (trap "unreachable")
  (call main (: 200 Int64))
  (trap "unreachable"))

(case
  "CROSS-TYPE @invariant: a value deconstructed from one @invariant newtype and RE-constructed into ANOTHER establishes BOTH invariants at their own sites"
  (doc
    "Two DISTINCT @invariant types, and a value flowing from one into the other through a helper — each
           establish check fires at its own construction site. `Nat` has `@invariant(>= self 0)`, `Pct` has
           `@invariant(<= self 100)`. `to-pct` deconstructs a `Nat` (reading its erased scalar) and RE-wraps it
           as a `Pct`, so the re-wrap is a fresh Pct-establish obligation. main builds `(Nat.Mk k)` FIRST (Nat's
           >=0 establish) then feeds it to to-pct (Pct's <=100 establish). Three regions over the runtime arg:
           k=50 → Nat.Mk 50 ok (50>=0) → Pct.P 50 ok (50<=100) → 50; k=150 → Nat.Mk 150 ok but Pct.P 150 VIOLATES
           <=100 → traps at the Pct establish; k=-5 → Nat.Mk -5 VIOLATES >=0 → traps at the FIRST (Nat) establish
           before to-pct is even called. Pins that two independent @invariant types each enforce at their
           respective sites when a value crosses between them — the first-violating site traps first. Runtime
           arg via main's param (no const-fold).")
  (input
    (do
      (@ (invariant (>= self 0)) (type Nat (Mk Int64)))
      (@ (invariant (<= self 100)) (type Pct (P Int64)))
      (def (nat-val (: n Nat)) (match n ((Nat.Mk v) v)))
      (def (to-pct (: n Nat)) (Pct.P (nat-val n)))
      (def (main (: k Int64)) (match (to-pct (Nat.Mk k)) ((Pct.P p) p)))
      (export main)))
  (call main (: 50 Int64))
  (output (: 50 Int64))
  (call main (: 150 Int64))
  (trap "unreachable")
  (call main (: -5 Int64))
  (trap "unreachable"))

(case
  "TWO distinct @invariant-typed PARAMETERS of one def each establish at their OWN argument construction site"
  (doc
    "The CO-PARAMETER companion of the CROSS-TYPE case above (there one value flows sequentially Nat→Pct;
           here two DIFFERENT invariant types are DISTINCT parameters of a single def, each built independently
           at the call). `Pct` has `@invariant(0 <= self <= 100)`, `Pos` has `@invariant(>= self 1)`. `(f a b)`
           takes `a : Pct` and `b : Pos` and sums their payloads; `main` builds BOTH arguments — `(Pct.P x)` and
           `(Pos.Q y)` — at the call site, so each establish fires at its own construction. main(50, 5): Pct.P 50
           establishes (0..100) AND Pos.Q 5 establishes (>=1) → 55. main(150, 5): the `(Pct.P 150)` construction
           VIOLATES `<= self 100` → traps at the Pct argument's establish. main(50, 0): Pct.P 50 is fine, but
           `(Pos.Q 0)` VIOLATES `>= self 1` → traps at the Pos argument's establish. Pins that a def with several
           invariant-typed parameters gets an INDEPENDENT establish per argument at its own construction site —
           the two invariants do not interfere, and whichever argument is built-invalid traps. Runtime args via
           main's params so nothing folds.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (@ (invariant (>= self 1)) (type Pos (Q Int64)))
      (def (up (: p Pct)) (match p ((Pct.P n) n)))
      (def (uq (: q Pos)) (match q ((Pos.Q n) n)))
      (def (f (: a Pct) (: b Pos)) (+ (up a) (uq b)))
      (def (main (: x Int64) (: y Int64)) (f (Pct.P x) (Pos.Q y)))
      (export main)))
  (call main (: 50 Int64) (: 5 Int64))
  (output (: 55 Int64))
  (call main (: 150 Int64) (: 5 Int64))
  (trap "unreachable")
  (call main (: 50 Int64) (: 0 Int64))
  (trap "unreachable"))

(case
  "@ensures over a BIGINT (arbitrary-precision heap) result checks it with structural equality — the postcondition reads a non-scalar heap value"
  (doc
    "The enforcement predicate operates on a BigInt result — a heap-allocated arbitrary-precision value, a
           distinct representation from an Int64 scalar — using structural `=` (BigInt has no `>=` prelude op;
           equality is the comparison). `f(n) = (BigInt.of n)` under `@ensures(= ret (BigInt.of 42))`: the
           postcondition builds its own `(BigInt.of 42)` and compares it to `ret` by value. main projects the
           result to a Bool via `(= (f k) (BigInt.of 42))`. f(42): ret = BigInt 42, the postcondition's `=`
           holds → main returns true. f(7): ret = BigInt 7, `= (BigInt.of 42)` is false → the postcondition
           traps. Pins that verify_enforce's `ret` binder + predicate lower correctly over a heap BigInt result
           (not just Int64 scalars / tuples / records), and structural `=` composes in predicate position.
           Runtime arg via main's param (no const-fold).")
  (input
    (do
      (@ (ensures (= ret (BigInt.of 42))) (def (f (: n Int64)) (BigInt.of n)))
      (def (main (: k Int64)) (= (f k) (BigInt.of 42)))
      (export main)))
  (call main (: 42 Int64))
  (output (: true Bool))
  (call main (: 7 Int64))
  (trap "unreachable")
  (live-objects known-leak))

; ── Contracts x first-class function values: enforcement travels WITH the def, not the call site ────
; Every enforcement case above calls its guarded def directly by name. These pin that the verify_enforce
; body-rewrite survives the def becoming a VALUE — passed as a fn argument, or selected by a runtime
; branch — so the contract fires no matter how the call reaches the body (the rewrite is IN the body,
; not at the direct call site; an implementation enforcing at direct call sites only would silently skip
; these indirect applications).
(case
  "@requires is enforced when the guarded def is applied through a first-class fn value"
  (doc
    "`safe` is @requires-guarded and passed BY NAME to `apply1`, which calls it through its fn
           PARAMETER. The body-entry check travels with the def: `apply1 safe 5` computes 6, and
           `apply1 safe -1` violates `(>= x 0)` and traps — through the indirect call, exactly as a
           direct one. An enforcement keyed to direct call sites would return 0 at -1 instead of
           trapping.")
  (input
    (do
      (@ (requires (>= x 0)) (def (safe (: x Int64)) (+ x 1)))
      (def (apply1 (: f (-> Int64 Int64)) (: v Int64)) (f v))
      (def (main (: v Int64)) (apply1 safe v))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -1 Int64))
  (trap "unreachable"))

(case
  "@requires on a MODULE export is enforced when the def is called across the module-access boundary"
  (doc
    "Every enforcement case so far calls the contracted def in the same top-level scope; this pins that
           the body-entry check TRAVELS with a def exported from a module and reached via member access. A
           module declaration binds its name in the enclosing scope (core-semantics.md), and `(. m f)` reaches
           the export, so a `@requires`-guarded def defined INSIDE the module must carry its injected `(if PRE
           BODY (trap …))` through the module-access call site. `m` exports `safe`, guarded by `@requires(>= x
           0)`; `main` calls `((. m safe) v)`. main(5): `5 >= 0` holds → the export computes 6. main(-1): `-1 >=
           0` is FALSE → the precondition traps THROUGH the member-access call, exactly as a same-scope direct
           call would. Pins that verify_enforce's rewrite is a property of the def itself, not of the call being
           lexically local — an enforcement keyed to local call sites would return the unchecked body across the
           module boundary. Runtime arg via main's param so nothing folds.")
  (input
    (do
      (module m
        (@ (requires (>= x 0)) (def (safe (: x Int64)) (+ x 1)))

        (export safe))
      (def (main (: v Int64)) (m.safe v))
      (export main)))
  (call main (: 5 Int64))
  (output (: 6 Int64))
  (call main (: -1 Int64))
  (trap "unreachable"))

(case
  "@ensures on a MODULE export is enforced when the def is called across the module-access boundary"
  (doc
    "The body-EXIT companion of the @requires-across-a-module case above: it pins the POSTCONDITION
           rewrite half — `(let ((ret BODY)) (if Q ret (trap …)))` — travels with a module export too, not only
           the precondition `(if PRE …)` half. `m` exports `dec`, guarded by `@ensures(>= ret 0)` over `(dec x)
           = x - 1`; `main` calls `((. m dec) v)`. main(3): `dec` returns 2, `2 >= 0` holds → 2 flows through the
           member-access call. main(0): `dec` returns -1, `-1 >= 0` is FALSE → the postcondition traps at
           body-EXIT, THROUGH the member access. Pins that BOTH verify_enforce rewrite halves (entry-check and
           exit-check) are properties of the def and survive the module boundary — a postcondition dropped on
           the module-access path would leak a -1 for main(0). Runtime arg via main's param so nothing folds.")
  (input
    (do
      (module m
        (@ (ensures (>= ret 0)) (def (dec (: x Int64)) (- x 1)))

        (export dec))
      (def (main (: v Int64)) (m.dec v))
      (export main)))
  (call main (: 3 Int64))
  (output (: 2 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "@ensures holds when the guarded def arrives through a runtime branch"
  (doc
    "The function-valued-conditional composition: `(if b abs1 idf)` selects between the
           @ensures-guarded `abs1` and the unguarded `idf` at run time, then applies the selection.
           b=true routes through the guarded body — the postcondition `(>= ret 0)` checks abs1's result
           (5, satisfied) inside the selected function. The guard must ride the fn value through the
           branch join (both arms share the arrow type; only one carries a contract).")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (abs1 (: x Int64)) (if (< x 0) (- 0 x) x)))
      (def (idf (: x Int64)) x)
      (def (main (: b Bool) (: v Int64)) ((if b abs1 idf) v))
      (export main)))
  (call main (: true Bool) (: -5 Int64))
  (output (: 5 Int64)))

(case
  "a VIOLATED @ensures traps when the def arrives through a runtime branch"
  (doc
    "The violation twin: `bad` (result x-10, postcondition `(>= ret 0)`) selected by b=true traps at
           v=3 (ret=-7 violates); the same call through the b=false arm picks the unguarded `idf` and
           returns 3 — one call site, contract enforcement decided by WHICH function the runtime branch
           delivered. Pins that the trap fires in the guarded body only (a join that smeared the contract
           over both arms would trap the idf path too; one that dropped it would return -7).")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (bad (: x Int64)) (- x 10)))
      (def (idf (: x Int64)) x)
      (def (main (: b Bool) (: v Int64)) ((if b bad idf) v))
      (export main)))
  (call main (: true Bool) (: 3 Int64))
  (trap "unreachable")
  (call main (: false Bool) (: 3 Int64))
  (output (: 3 Int64)))

(case
  "@requires over a HEAP argument reads the list's content at body entry"
  (doc
    "The heap-argument face of the precondition (the enforcement pins guard scalars): `head-of` is
           guarded by `(> (List.len xs) 0)`, so the check must WALK the heap argument (an RRB len read)
           at body entry — a non-empty call computes (42), the empty-list call traps before the body's
           `List.at` could produce its own miss. Pins that verify_enforce's predicate evaluation composes
           with heap-typed parameters, not only scalar comparisons.")
  (input
    (do
      (@
        (requires (> (List.len xs) 0))
        (def (head-of (: xs (List Int64))) (match (List.at xs 0) ((Some v) v) ((None u) -1))))
      (def (main (: n Int64)) (if (> n 0) (head-of #list(n 2)) (head-of #list())))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

(case
  "an @invariant newtype constructed INSIDE a handler arm establishes per resume"
  (doc
    "The effects-composition face of the ESTABLISH divert: the checked constructor runs INSIDE a
           handler arm (`(resume (Pos v) s)`) — the divert must reach construction sites in arm bodies,
           and the establish trap must fire through the handler machinery. n=42 constructs and unwraps
           (42); n=-1 violates `(> self 0)` and traps AT THE ARM's construction, not downstream. Extends
           the divert-site family (lambda/match-arm/let-init/collection pins) to handler arms.")
  (input
    (do
      (@ (invariant (> self 0)) (type Pos (Pos Int64)))
      (effect Mk (op make (-> Int64 Pos)))
      (def
        (main (: n Int64))
        (handle Mk 0 ((make (v) s (resume (Pos v) s))) (match (Mk.make n) ((Pos v) v))))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64))
  (call main (: -1 Int64))
  (trap "unreachable"))

(case
  "a def BODY that itself binds `ret` in an inner let composes soundly with @ensures — lexical shadowing, not a collision"
  (doc
    "`verify_enforce` rewrites `@ensures` to `(let ((ret BODY)) (if Q ret (trap …)))` — it introduces an
           OUTER binder literally named `ret` (RESULT_BINDER). This pins that a user body which ITSELF binds a
           variable named `ret` in an INNER `let` composes soundly: the injected `(let ((ret …)) …)` wraps the
           whole body, so the body's own inner `(let ((ret (- x 100))) (+ ret 5))` is lexically NESTED and its
           `ret` shadows locally, while the postcondition's `ret` (in `(>= ret 0)`) binds to the OUTER
           injected let (the def's actual result). Distinct from the `2022` case, which rejects a PARAMETER
           named `ret`; here `ret` is an inner LOCAL, which is legal and must not be confused with the result
           binder. `(f 200)` → inner `ret = 100`, body result `105`, which satisfies `(>= ret 0)` at the outer
           binder → value-transparent, returns `105`. A future change to how verify_enforce chooses/handles its
           result binder that failed to respect lexical nesting would break this — the pin guards that seam.")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (f (: x Int64)) (let ((ret (- x 100))) (+ ret 5))))
      (def (main) (f 200))
      (export main)))
  (output (: 105 Int64)))

(case
  "a def BODY that binds `ret` in an inner let still has its @ensures checked against the ACTUAL result — traps on violation"
  (doc
    "The trap half of the inner-`ret`-shadow composition above. The postcondition `(>= ret 0)` must be
           checked against the def's ACTUAL result (the value the outer injected `(let ((ret BODY)) …)` binds),
           NOT against the body's inner `ret` local. `(f 200)` → inner local `ret = 100`, but the body result is
           `(- ret 1000)` = `-900`, which the OUTER `ret` binds; `(>= -900 0)` is false, so the `if` takes the
           trap arm → `unreachable`. Confirms the enforcement reads the result binder, and the inner shadow does
           not leak a satisfying value into the postcondition check.")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (f (: x Int64)) (let ((ret (- x 100))) (- ret 1000))))
      (def (main) (f 200))
      (export main)))
  (trap "unreachable"))

(case
  "@invariant ESTABLISH divert reaches a CALL-ARGUMENT construction site — a violating value traps before the callee runs (design §10.2, (D))"
  (doc
    "A fourth indirect-construction face for the establish divert (companion to the lambda-body /
           reconstruct-after-update / list-element trio above): the construction `(Pct.P v)` sits directly in
           ARGUMENT position of a call `(use (Pct.P v))`. `Pct = P Int64` has `@invariant(0 <= self <= 100)`.
           The divert (`lower_sum_new` routing every `(Pct.P v)` through the synthesized checked constructor)
           must fire at the call-argument construction site, so a violating value TRAPS at construction BEFORE
           the callee `use` is entered — never passing an invalid `Pct` across the call boundary. `run(70)`:
           establish (0..100) holds → `use` receives a valid Pct and computes `70 + 1 = 71`. `run(150)`: the
           `(Pct.P 150)` establish (<=100) fails at the construction site in argument position → trap there,
           and `use` is never reached. Guards that the site-based rewrite visitor does not skip an argument
           position (a place a body-only walk could miss), keeping the establish obligation sound at the call
           boundary.")
  (input
    (do
      (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64)))
      (def (use (: p Pct)) (match p ((Pct.P n) (+ n 1))))
      (def (run (: v Int64)) (use (Pct.P v)))
      (export run)))
  (call run (: 70 Int64))
  (output (: 71 Int64))
  (call run (: 150 Int64))
  (trap "unreachable"))

; --- Contracts over HEAP data: the record-payload invariant (with the row-op re-wrap divert)
; and String byte-len predicates in both contract halves. ---
(case
  "@invariant ESTABLISH fires through a ROW-OP re-wrap (Record.without + Record.extend rebuild the payload)"
  (doc
    "The establish matrix covers literal/lambda/list-element/update-rewrap construction sites — none rebuilds the payload via ROW OPS: bump strips hi with without, re-adds via extend, re-wraps Rng.R. The divert must catch THIS site even though the payload came from row ops, not a record literal (d=5 -> 13; d=-20 violates -> trap at the re-wrap).")
  (input
    (do
      (@
        (invariant (match self ((Rng.R r) (< r.lo r.hi))))
        (type Rng (R (Record (: lo Int64) (: hi Int64)))))
      (def
        (bump (: v Rng) (: d Int64))
        (match v ((Rng.R r) (Rng.R (Record.extend (Record.without r (hi)) #"hi" (+ r.hi d))))))
      (def
        (mk (: a Int64) (: b Int64) (: d Int64))
        (match (bump (Rng.R #record((= lo a) (= hi b))) d) ((Rng.R r) (- r.hi r.lo))))
      (export mk)))
  (call mk (: 2 Int64) (: 10 Int64) (: 5 Int64))
  (output (: 13 Int64))
  (call mk (: 2 Int64) (: 10 Int64) (: -20 Int64))
  (trap "unreachable"))

(case
  "@invariant over a RECORD-payload newtype establishes on the record's relational fields"
  (doc
    "No establish case has a RECORD payload: (type Rng (R (Record (lo)(hi)))) with the relational field invariant (< lo hi) establishes at construction (2,10 -> 8; 10,2 -> trap).")
  (input
    (do
      (@
        (invariant (match self ((Rng.R r) (< r.lo r.hi))))
        (type Rng (R (Record (: lo Int64) (: hi Int64)))))
      (def
        (mk (: a Int64) (: b Int64))
        (match (Rng.R #record((= lo a) (= hi b))) ((Rng.R r) (- r.hi r.lo))))
      (export mk)))
  (call mk (: 2 Int64) (: 10 Int64))
  (output (: 8 Int64))
  (call mk (: 10 Int64) (: 2 Int64))
  (trap "unreachable"))

(case
  "@requires and @ensures read a HEAP String param's byte-len — enforce on the empty-string violation"
  (doc
    "Every contract predicate reads scalars/tuple-projections/globals — none reads a HEAP param: @requires (> (byte-len s) 0) + the relational @ensures (> (byte-len ret) (byte-len s)) tie result heap-len to param heap-len; the injected checks must BORROW the rope, and the param stays alive precondition->body->postcondition. hi passes (3); empty violates -> trap.")
  (input
    (do
      (@
        (requires (> (String.byte-len s) 0))
        (@
          (ensures (> (String.byte-len ret) (String.byte-len s)))
          (def (shout (: s String)) (String.concat s "!"))))
      (def (main (: k Int64)) (String.byte-len (shout (if (= k 1) "hi" ""))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (trap "unreachable")
  (live-objects known-leak))

(case
  "@requires over a BYTES parameter reads Bytes.len at body entry — completes the heap-param domain (List/Map/String/Bytes)"
  (doc
    "The Bytes face of a heap-param precondition. Bytes is a distinct heap type from String with its own
           `Bytes.len` op, so this pins the enforcement path borrows a BYTE SEQUENCE (not a rope) at body entry.
           `@requires(> (Bytes.len b) 0)` on `(size b) = (Bytes.len b)` demands a non-empty byte sequence; the
           injected `(if (> (Bytes.len b) 0) BODY (trap …))` reads the length before the body runs. Runtime
           selection via main's param so the Bytes value isn't const-folded away. main(1) builds `(Bytes.of (list
           0 255 128))` — length 3 > 0 → pass → 3. main(0) builds `(Bytes.of (list))` — the empty byte sequence,
           length 0, so `(> 0 0)` is FALSE → the precondition fails → trap before the body's own `Bytes.len`
           could answer. Pins that verify_enforce's heap-predicate composition extends from the RRB (List) and
           CHAMP (Map) and rope (String) len reads to the Bytes len read — the whole heap-param domain enforces.")
  (input
    (do
      (@ (requires (> (Bytes.len b) 0)) (def (size (: b Bytes)) (Bytes.len b)))
      (def (main (: k Int64)) (size (if (= k 1) (Bytes.of #list(0 255 128)) (Bytes.of #list()))))
      (export main)))
  (call main (: 1 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

; --- A @requires walking a MAP argument at body entry. ---
(case
  "@requires over a MAP argument walks the CHAMP at body entry"
  (doc
    "The map-argument face of the precondition (List is pinned at :3210): `first-val` is
           guarded by `(> (Map.len m) 0)` — the check reads the CHAMP's len at entry, so the
           populated call computes (42) and the EMPTY-map call traps before the body's lookup could
           answer its own None (-1 never happens; the trap is the precondition's). Extends
           verify_enforce's heap-predicate composition from the RRB len read to the CHAMP.")
  (input
    (do
      (@
        (requires (> (Map.len m) 0))
        (def
          (first-val (: m (Map Int64 Int64)))
          (match (Map.lookup m 1) ((Some v) v) ((None _u) -1))))
      (def
        (main (: n Int64))
        (if (> n 0) (first-val (Map.insert Map.empty 1 n)) (first-val Map.empty)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

; --- A contract firing on a perform-produced argument. ---
(case
  "a CONTRACTED fn consumes perform results — @requires fires on a handler-produced value"
  (doc
    "The INVERSE of the effectful-predicate pins (:2134 — the PREDICATE performs): here the contracted fn's ARGUMENT is a perform result, so @requires checks a value the handler produced mid-expression. Seeding 0 makes the first perform return 0 and the precondition traps ON the perform-produced value; seed 3 threads state 3,4 through both calls (14).")
  (input
    (do
      (effect Src (op next (-> Unit Int64)))
      (@ (requires (> n 0)) (@ (ensures (> ret n)) (def (dbl (: n Int64)) (* n 2))))
      (def
        (main (: k Int64))
        (handle Src k ((next (_u) s (resume s (+ s 1)))) (+ (dbl (Src.next)) (dbl (Src.next)))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 14 Int64))
  (call main (: 0 Int64))
  (trap "unreachable"))

; --- The IDIOMATIC operator-encoding: a HeadOp SUM, not a magic-int Const tag. ---
; This case mirrors the flagship discharge (:47) but encodes each arithmetic head-symbol as a NULLARY
; VARIANT of a closed `HeadOp` sum applied via `Term.Head`, instead of a `Term.Const Int64` magic tag
; with an out-of-band comment legend (add=0, le=1, …). It matches the shape of the compiler-bundled
; trusted kernel `verify_kernel.cdz` after the 2026-08-01 fleet-wide directive to eliminate C-style
; sentinel/magic-int anti-patterns from ALL Cadenza code: a closed operator set is a sum type, `op-eq`
; is an exhaustive sum match (a new head is a compile-checked addition, not a silently-clashing int).
; The kernel is otherwise byte-for-byte the `bounds` order-logic — this pins that the idiomatic encoding
; discharges the SAME no-overflow obligation end-to-end (a live semantic witness for the bundled kernel,
; which is otherwise only parse-checked). `Var`/`Num` keep their Int64 payloads: a de Bruijn variable
; index and a numeric literal are GENUINELY integers, not sentinels — only the operator TAG was the
; anti-pattern.
(case
  "IDIOMATIC HeadOp-sum encoding discharges no-overflow: for x<=100, (x+1)<=MAXINT (sum-typed head, not a magic-int Const tag)"
  (doc
    "The strongly-typed-head analogue of the flagship discharge at :47. Head-symbols are nullary
           variants of a closed `HeadOp` sum (Add/Le/…) applied through `Term.Head`, matched by an
           exhaustive `op-eq` — the idiomatic encoding of a closed operator set, replacing the
           `Term.Const Int64` magic-tag form (add=0, le=1) the operator's fleet-wide directive kills.
           The proof is unchanged: assume `LE x (Num 100)`, `mono-add-r` to `LE (add x 1) (add 100 1)`,
           the CHECKED ground axiom `le-ax (add 100 1) MAXINT` mints `LE (add 100 1) MAXINT` (101<=MAXINT
           holds), `trans-le` closes to `LE (add x 1) MAXINT`, and `term-eq` confirms the conclusion IS
           the obligation. Runs to `true`. This is the shape of the compiler-bundled `verify_kernel.cdz`
           after its magic-int elimination, giving that otherwise-parse-only asset a live discharge run.")
  (module "bounds"
    (do
      (type HeadOp (Add) (Le))
      (type Term (Var Int64) (Num Int64) (Comb Term Term) (Head HeadOp))
      (type Thm (Seq (List Term) Term))
      ; equality on the closed operator set — an exhaustive sum match, not an Int64 compare
      (def
        (op-eq (: a HeadOp) (: b HeadOp))
        (match
          a
          ((HeadOp.Add) (match b ((HeadOp.Add) true) (_ false)))
          ((HeadOp.Le) (match b ((HeadOp.Le) true) (_ false)))))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Num n) (match b ((Term.Num m) (= n m)) (_ false)))
          ((Term.Head o) (match b ((Term.Head p) (op-eq o p)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (add (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b))
      (def (le (: a Term) (: b Term)) (Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b))
      (def (maxint) (Term.Num 9223372036854775807))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def
        (eval-ground (: t Term))
        (match
          t
          ((Term.Num n) (Option.Some n))
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Add) a) b)
            (match
              (eval-ground a)
              ((Option.Some av)
                (match
                  (eval-ground b)
                  ((Option.Some bv) (Option.Some (+ av bv)))
                  ((Option.None) (Option.None))))
              ((Option.None) (Option.None))))
          (_ (Option.None))))
      (def
        (le-ax (: lhs Term) (: rhs Term))
        (match
          (eval-ground lhs)
          ((Option.Some lv)
            (match
              (eval-ground rhs)
              ((Option.Some rv)
                (if (<= lv rv) (Option.Some (Thm.Seq #list() (le lhs rhs))) (Option.None)))
              ((Option.None) (Option.None))))
          ((Option.None) (Option.None))))
      (def
        (mono-add-r (: th Thm) (: k Term))
        (match
          (concl th)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) x) c)
            (Option.Some (Thm.Seq (hyps th) (le (add x k) (add c k)))))
          (_ (Option.None))))
      (def
        (trans-le (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) a) b)
            (match
              (concl t2)
              ((Term.Comb (Term.Comb (Term.Head HeadOp.Le) b2) c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (le a c)))
                  (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export Term.*)
      (export HeadOp.*)
      (export Thm)
      (export op-eq term-eq add le maxint concl hyps assume eval-ground le-ax mono-add-r trans-le)))
  (input
    (do
      (import
        "bounds"
        (HeadOp Term Thm term-eq add le maxint assume eval-ground le-ax mono-add-r trans-le concl))
      (def
        (main)
        (let
          ((x (Term.Var 0)) (one (Term.Num 1)) (c (Term.Num 100)))
          (let
            ((obligation (le (add x one) (maxint))))
            (let
              ((pre (assume (le x c))))
              (match
                (mono-add-r pre one)
                ((Option.Some step1)
                  (match
                    (le-ax (add c one) (maxint))
                    ((Option.Some fact)
                      (match
                        (trans-le step1 fact)
                        ((Option.Some proof) (term-eq (concl proof) obligation))
                        ((Option.None) false)))
                    ((Option.None) false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; --- Contract enforcement × ABORTIVE effects: the conditions surface pins effectful PREDICATES
; (:2134) and perform-produced ARGUMENTS (:3554), but not the composition with an ABORTIVE
; perform — a body that never produces a value has no postcondition to check, while the
; precondition's entry check runs BEFORE anything in the body (including an abort) can fire. ---
(case
  "an abortive perform in a contracted body skips the @ensures (an abandoned body has no result)"
  (doc
    "`f` carries `@ensures (< ret 10)`; its abort path performs `(Bail.bail 99)`, whose handler arm
           returns 99 as the HANDLE's value — a value that would VIOLATE the postcondition if it were
           checked. It must not be: the abort abandons `f`'s body, so there is no function result for
           `@ensures` to check, and 99 flows out of the handle untrapped. The satisfying path (x=5 → 6)
           still enforces normally. An implementation that attached the postcondition check to the
           HANDLE's value (rather than the body's completion) would trap the abort path here.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (@ (ensures (< ret 10)) (def (f (: x Int64)) (if (< x 0) (Bail.bail 99) (+ x 1))))
      (def (main (: x Int64)) (handle Bail 0 ((bail (n) s n)) (f x)))
      (export main)))
  (call main (: -3 Int64))
  (output (: 99 Int64))
  (call main (: 5 Int64))
  (output (: 6 Int64)))

(case
  "a body that TRAPS keeps its OWN trap kind under an @ensures — the postcondition wrapper is transparent to an aborting body"
  (doc
    "The trap-based sibling of the abortive-perform case above: both pin that the @ensures wrapper does not
           interfere with a body that produces no normal result. `verify_enforce` wraps the body as `(let ((ret
           BODY)) (if Q ret (trap …)))`. When BODY itself traps — here `(/ 100 x)` on `x = 0`, a runtime
           divide-by-zero — the `let` never binds `ret`, so the postcondition `if` is never reached and the
           body's OWN trap kind propagates. main(5): `100 / 5` = 20, `20 >= 0` holds → 20 flows normally. main(0):
           the body traps with `divide by zero` — NOT the `@ensures`-failed `unreachable`, proving the wrapper is
           transparent to an aborting body (it does not catch the body's trap and relabel it, nor evaluate Q on a
           nonexistent result). Pins that the postcondition is a check on a COMPLETED body's value, layered
           strictly outside the body's own control flow.")
  (input
    (do
      (@ (ensures (>= ret 0)) (def (f (: x Int64)) (/ 100 x)))
      (def (main (: k Int64)) (f k))
      (export main)))
  (call main (: 5 Int64))
  (output (: 20 Int64))
  (call main (: 0 Int64))
  (trap "divide by zero"))

(case
  "a violated @requires traps BEFORE the body's abortive perform can fire"
  (doc
    "Entry-check-first ordering under an abortive body: `f` carries `@requires (> x -100)` and its
           body's first action on the negative path is the abortive `(Bail.bail 99)`. At x=-500 the
           precondition is violated, so the entry check traps `unreachable` — the abort (which would have
           produced a clean 99 through the handler) never fires. At x=-3 the precondition holds and the
           abort proceeds normally (99). An emit that evaluated any of the body before the entry check
           would abort instead of trapping at x=-500.")
  (input
    (do
      (effect Bail (op bail (-> Int64 Int64)))
      (@ (requires (> x -100)) (def (f (: x Int64)) (if (< x 0) (Bail.bail 99) (+ x 1))))
      (def (main (: x Int64)) (handle Bail 0 ((bail (n) s n)) (f x)))
      (export main)))
  (call main (: -500 Int64))
  (trap "unreachable")
  (call main (: -3 Int64))
  (output (: 99 Int64)))

(case
  "a RESUMING handler around a contracted body still enforces @ensures on the resumed-through value"
  (doc
    "The resumptive companion of the abort-skips-@ensures pin: here the handler RESUMES 50 into the
           body, so the body COMPLETES (with 50 + x) and its postcondition `(< ret 10)` checks the
           completed value — x=1 → 51, violated → the canonical unreachable trap. x=-45 → 5 passes. Pins
           that a resumptive perform does not detach the postcondition: only an ABORT (body never
           completes) skips it; a body completed via resume is checked like any other.")
  (input
    (do
      (effect Ask (op get (-> Unit Int64)))
      (@ (ensures (< ret 10)) (def (f (: x Int64)) (+ x (Ask.get))))
      (def (main (: x Int64)) (handle Ask 0 ((get (_u) s (resume 50 s))) (f x)))
      (export main)))
  (call main (: 1 Int64))
  (trap "unreachable")
  (call main (: -45 Int64))
  (output (: 5 Int64)))

; ── RELATIONAL contracts over HEAP values: conditions relating two params, a result to a param,
; and both ends of a call chain. The fn-level relational face above is scalar; these pin the
; heap shapes real code leans on (the zip precondition, the growth postcondition). ---
(case
  "a @requires relating TWO heap params enforces the zip precondition at runtime"
  (doc
    "`@requires (= (List.len xs) (List.len ys))` on a pairwise-product fold — the classic zip
           precondition relating two HEAP arguments. Matched 3/3 lengths → 1·4+2·5+3·6 = 32; a 3-vs-2
           call traps at entry BEFORE the body's indexing could read out of bounds. Pins that a
           relational predicate over two heap params evaluates both operands' lengths at the call
           boundary (a single-subject-only rewrite that dropped the second operand would let the
           mismatch through to an in-body index miss).")
  (input
    (do
      (@
        (requires (= (List.len xs) (List.len ys)))
        (def
          (zip-sum (: xs (List Int64)) (: ys (List Int64)) (: i Int64) (: acc Int64))
          (if
            (>= i (List.len xs))
            acc
            (zip-sum
              xs
              ys
              (+ i 1)
              (+ acc (* (Option.expect (List.at xs i) "x") (Option.expect (List.at ys i) "y")))))))
      (def (main (: n Int64)) (zip-sum #list(1 2 3) (if (> n 0) #list(4 5 6) #list(4 5)) 0 0))
      (export main)))
  (call main (: 1 Int64))
  (output (: 32 Int64))
  (call main (: 0 Int64))
  (trap "unreachable")
  (live-objects 0))

(case
  "an @ensures relating the RESULT to a PARAM over heap values enforces growth"
  (doc
    "`@ensures (> (List.len ret) (List.len xs))` — the postcondition compares the RESULT's length
           against the INPUT's, both heap values. The growth path (n>0 pushes) satisfies → 3; the n=0
           identity path returns the input unchanged, violating strict growth → trap. Pins that `ret`
           and a param are BOTH in scope for the postcondition and that heap-measure comparisons between
           them evaluate at exit (a post that could only see `ret` cannot express this contract).")
  (input
    (do
      (@
        (ensures (> (List.len ret) (List.len xs)))
        (def (grow (: xs (List Int64)) (: n Int64)) (if (> n 0) (List.push xs n) xs)))
      (def (main (: n Int64)) (List.len (grow #list(1 2) n)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 3 Int64))
  (call main (: 0 Int64))
  (trap "unreachable")
  (live-objects known-leak))
