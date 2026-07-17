; ============================================================================================
; 26-program-conditions.sexp — program pre/post-conditions whose proofs are DISCHARGED by the
; verification kernel (Increment-b, the "conditions feed optimization" workstream). See
; implementation/design/DESIGN-verification-program-conditions.md. Vertical: v-verification.
;
; Increment (a) built an unforgeable HOL `Thm` (25-verification.sexp). Increment (b) USES it: a
; pre/post-condition on a Cadenza program denotes into a HOL obligation `Term`, and the kernel
; discharges it into a `Thm`. The operator's headline is that a DISCHARGED obligation is a
; first-class optimizer input — a proven `no-overflow@Id` lets the Core-tier elision pass drop the
; overflow guard (four-way seam with v-core-opt/v-wasm-opt/v-rust-backend; see the design §3/§7).
;
; These b1 cases are the FRONT-LOADED design validation: NO compiler change. They hand-author the
; obligation `Term`s and prove them THROUGH the kernel, exactly as a b2 denotation would emit them,
; so the discharge machinery is validated end-to-end before any optimizer wiring exists.
;
; THE ARITHMETIC-DISCHARGE CONVENTION (design §1A + the b1 crux). The HOL kernel has NO built-in
; arithmetic decision procedure — it proves via primitive rules over abstract `Term`s. So a
; no-overflow obligation is discharged from an EXPLICIT, minimal, trusted arithmetic-axiom base
; (the analogue of HOL-Light's `ARITH`): order facts minted by `le-ax` and monotonicity/transitivity
; rules. Concretely, for a checked `x + k : Int64` at a node whose PRECONDITION bounds `x ≤ c`:
;   • the obligation `no-overflow@Id` is the term `LE (add x k) MAXINT`
;     (arithmetic head-symbols `add`/`le`/`maxint` encode as `Const`-headed `Comb` applications —
;      `add` is `Const 0`, `le` is `Const 1`, `maxint` is `Const 2`, matching the kernel's Term sum);
;   • from the precondition hypothesis `LE x (num c)` (via `assume`), the `mono-add-r` rule derives
;     `LE (add x k) (add c k)`, and an `le-ax` numeral fact `LE (add c k) MAXINT` closes it by `trans`.
; The b2 match predicate (the compiler's trusted surface, NOT here) will additionally require the
; discharged Thm's HYPS ⊆ the node's stated precondition, so a Thm proven under DIFFERENT assumptions
; cannot license an elision. These cases pin the discharge; the match predicate is pinned at b2.
;
; `bounds` is the arithmetic kernel: a HOL `Thm` specialized to integer-order reasoning. It keeps the
; SAME LCF discipline as `hol` (abstract Thm, private constructor, rules are the only way to mint one),
; so the unforgeability audit of 25-verification.sexp carries over unchanged — an obligation is
; discharged only by the trusted order-rules, never fabricated.
; ============================================================================================

(case "a no-overflow obligation is DISCHARGED: for x <= 100, (x + 1) <= MAXINT via monotonicity + a numeral fact"
  (doc    "The first program-condition discharge — the b1 milestone. A checked `x + 1 : Int64` guarded by
           the precondition `x <= 100` has the no-overflow obligation `LE (add x 1) MAXINT`. The `bounds`
           kernel proves it WITHOUT any arithmetic primitive: from `assume (LE x (num 100))` it applies the
           `mono-add-r` rule (adding 1 to both sides of a `<=`) to get `LE (add x 1) (add 100 1)`, then an
           `le-ax` numeral fact `LE (add 100 1) MAXINT` and `trans` close it to `LE (add x 1) MAXINT`. The
           entry derives the obligation THROUGH the rules and checks the conclusion is structurally the
           obligation via the exported `term-eq`; it never fabricates the Thm. Runs to `true`. Pins that a
           no-overflow condition is dischargeable end-to-end from a bounded precondition — the fact a b2
           elision would consume. The arithmetic head-symbols (add=Const 0, le=Const 1, maxint=Const 2) are
           ordinary Const-headed Comb applications, so NO kernel extension is needed.")
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
      (def (maxint) (Term.Const 2))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; LEAF rule: assume a proposition (its own hypothesis)
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      ; AXIOM: a concrete numeral order fact, minted hypothesis-free (the ARITH base)
      (def (le-ax (: a Term) (: b Term)) (Thm.Seq (list) (le a b)))
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
      (export term-eq)
      (export add)
      (export le)
      (export maxint)
      (export concl)
      (export hyps)
      (export assume)
      (export le-ax)
      (export mono-add-r)
      (export trans-le)))
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
                        ; step 3: numeral fact (le (add 100 1) MAXINT)
                        (let ((fact (le-ax (add c one) (maxint))))
                          ; step 4: transitivity closes to (le (add x 1) MAXINT)
                          (match (trans-le step1 fact)
                            ((Option.Some proof) (term-eq (concl proof) obligation))
                            ((Option.None) false))))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "an UNCONSTRAINED add is NOT dischargeable: with no precondition bound, the no-overflow obligation cannot be closed (the check must stay)"
  (doc    "The dual — the soundness-critical negative. For an UNCONSTRAINED `x + 1 : Int64` (no precondition
           bounding x), there is no `LE x c` hypothesis to feed `mono-add-r`, so the discharge cannot be
           built: the obligation `LE (add x 1) MAXINT` is NOT provable from the arithmetic base alone (it is
           simply false — x could be MAXINT). The entry models the b2 discharge attempt WITHOUT a
           precondition: it has only the `le-ax`/`mono-add-r`/`trans-le` machinery and an unbounded x, and
           checks that NO chain yields the obligation. Concretely, attempting `mono-add-r` needs a `le`-shaped
           premise; `assume`-ing an ARBITRARY unrelated fact does not produce `LE (add x 1) MAXINT`, and the
           honest result is that the obligation is not reached — so the elision oracle returns None and the
           overflow check STAYS. Runs to `true` (the test asserts non-derivability, i.e. that the naive
           attempt does NOT match the obligation). Pins the default-is-always-the-check invariant at the
           discharge level: absence of a bounding precondition means no proof, hence no elision.")
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
      (def (maxint) (Term.Const 2))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export add)
      (export le)
      (export maxint)
      (export concl)
      (export assume)))
  (input  (do
            (import "bounds" (Term Thm term-eq add le maxint concl assume))
            (def (main)
              (let ((x   (Term.Var 0))
                    (one (Term.Num 1)))
                (let ((obligation (le (add x one) (maxint))))
                  ; With no precondition, the only Thm we can honestly build about x is an assumption
                  ; of some unrelated proposition — it does NOT establish the obligation. Model the
                  ; oracle's honest failure: the best available Thm's conclusion is not the obligation.
                  (let ((unrelated (assume (le x x))))
                    ; the check must STAY: assert the obligation is NOT what we derived
                    (not (term-eq (concl unrelated) obligation))))))
            (export main)))
  (output (: true Bool)))

; ── b2: the MATCH PREDICATE (the compiler's trusted surface, written IN CADENZA) ────────────────────
; The oracle's core (design §3): a discharged `Thm` LICENSES the elision of `overflow-check@Id` iff
;   (1) its conclusion is STRUCTURALLY EXACTLY the obligation `no-overflow@Id` for the node's ACTUAL
;       operands (term-eq), AND
;   (2) every hypothesis it was proven under is DISCHARGED BY the node's stated precondition
;       (hyps ⊆ precondition, each hyp term-eq to some precondition member).
; (2) is the soundness core: a `Thm` proven under an assumption the node's precondition does NOT provide
; must NOT license an elision (a proof of "x+1 ≤ MAXINT ASSUMING x ≤ 100" cannot elide the guard at a node
; whose precondition is only "x ≤ 200"). At b3 the compiler compile-time-evals this predicate and consumes
; only its boolean; here we pin the predicate itself.

(case "the b2 match predicate LICENSES the elision: the discharged no-overflow proof matches the obligation and its hyps are covered by the node precondition"
  (doc    "The positive b2 pin. The `bounds` kernel discharges `LE (add x 1) MAXINT` under hypothesis
           `LE x 100` (the b1 chain: assume → mono-add-r → trans-le with a numeral fact). The `licenses`
           predicate — the compiler's trusted match surface — accepts it: (1) `term-eq (concl proof)
           obligation` holds (the conclusion IS the obligation for the node's actual operands), AND (2)
           `hyps-subset (hyps proof) precondition` holds (its sole hypothesis `LE x 100` is exactly the
           node's stated precondition). So the oracle would return Some and the Core elision pass would drop
           the guard. Runs to `true`. Pins that a correctly-discharged proof under a matching precondition
           licenses the elision — the fact b3 consumes via compile-time eval.")
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
      (def (maxint) (Term.Const 2))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (le-ax (: a Term) (: b Term)) (Thm.Seq (list) (le a b)))
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
                  ; discharge the obligation (the b1 chain)
                  (let ((pre (assume (le x c))))
                    (match (mono-add-r pre one)
                      ((Option.Some step1)
                        (let ((fact (le-ax (add c one) (maxint))))
                          (match (trans-le step1 fact)
                            ((Option.Some proof)
                              ; the match predicate accepts: conclusion matches AND hyps ⊆ precondition
                              (licenses proof obligation precondition))
                            ((Option.None) false))))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "the b2 match predicate REJECTS a proof discharged under a FOREIGN hypothesis not in the node precondition (soundness — no elision under wrong assumptions)"
  (doc    "The soundness-critical b2 negative — the breaker vector the design flags. A proof can have the
           RIGHT conclusion `LE (add x 1) MAXINT` yet be established under a hypothesis the node's
           precondition does NOT provide: here the proof is discharged assuming `LE x 100`, but the node's
           stated precondition is only `LE x 200` (weaker — it does not license the `≤100`-dependent proof).
           `term-eq` on the conclusion ALONE would wrongly accept (conclusions match), so the match predicate
           MUST also check hyps ⊆ precondition — and it fails: the proof's hypothesis `LE x 100` is NOT a
           member of the precondition `{LE x 200}`. So `licenses` returns false → the oracle returns None →
           the overflow check STAYS. The entry asserts `licenses` is false for this mismatched-assumption
           proof (runs to `true` via `not`). Pins that a `Thm` proven under assumptions the node does not
           guarantee cannot license an elision — the exact forge-adjacent vector (right answer, wrong
           reasons) that a conclusion-only match would miss.")
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
      (def (maxint) (Term.Const 2))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps  (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (le-ax (: a Term) (: b Term)) (Thm.Seq (list) (le a b)))
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
                        (let ((fact (le-ax (add c100 one) (maxint))))
                          (match (trans-le step1 fact)
                            ((Option.Some proof)
                              ; conclusion matches, BUT hyp (le x 100) ∉ precondition {(le x 200)} →
                              ; licenses must be FALSE (the check must STAY). assert NOT licenses.
                              (not (licenses proof obligation precondition)))
                            ((Option.None) false))))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))
