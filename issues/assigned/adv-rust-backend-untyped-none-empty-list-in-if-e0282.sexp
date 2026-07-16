; BREAKER FINDING — rust-backend differential (wasm PASS, rust FAIL: artifact does not build, E0282).
;
; This case is ALREADY on trunk as `breaker specsubst: GEN of refl then SPEC substitutes the bound var
; throughout the body` in spec/semantics/25-verification.sexp (commit 66a63eabf). It PASSES on wasm but
; the RUST backend emits Rust that rustc REJECTS with `error[E0282]: type annotations needed`. The stale
; rust `.gate-baseline-rust` (predates the whole verification-kernel wave) was HIDING this as a non-entry;
; a fresh `gate --save --target rust` surfaces it as a `fail`, not a `todo`.
;
; ROOT CAUSE (from the emitted Rust, /tmp specsubst emit line 38):
;   match if free_in_hyps((1u64 as i64), vec![]) { Option::None }
;         else { Option::Some((vec![], Term::Forall(Box::new((1i64, Term::Eq(...)))))) } { ... }
;   `gen`'s `if` desugars to `if c { Option::None } else { Option::Some((vec![], ...)) }`. The backend
;   emits BARE `Option::None` and BARE `vec![]` with NO turbofish. rustc cannot infer the element type of
;   the empty `vec![]` (it comes from `refl`'s `(list)`, inlined here) nor `T` in `Option::None` from the
;   sibling branch inside an `if`-expression -> E0282 "cannot infer type of the type parameter T declared
;   on the enum Option". Wasm has no such local type inference, so the identical program runs correctly.
;
; The differential surfaces only when an empty list literal flows into an Option.Some payload (a tuple/
; sum) whose element type is fixed only by later unification, combined with a sibling Option.None in the
; same if/match — none of the isolated shapes reproduce (tested: bare empty (list) in a payload, None in a
; two-arm if, None from a match arm — all build on rust). It is the multi-def inlining of refl->gen that
; produces the un-annotated `(vec![], ...)`-inside-`Some`-beside-`None` shape rustc can't infer.
;
; SUGGESTED FIX (v-rust-backend): emit a turbofish on constructors whose type parameters are not fixed by
; their own arguments — `Option::None::<Ty>` and `Vec::<ElemTy>::new()` / `vec![] as Vec<ElemTy>` — using
; the type the checker already inferred (the wasm backend already knows the monomorphic type here). The
; type is available at emit time; only the Rust surface syntax drops it.
;
; The reproducer below is the exact committed case, EXTRACTED for a self-contained rust-backend repro.
; EXPECTED (both backends, once fixed): output (: 1 Int64). CURRENT: wasm 1 (pass), rust E0282 (fail).

(case "adv rust-backend: an empty list inlined into Option.Some beside Option.None in an if emits untyped None/vec (E0282)"
  (doc "The GEN-of-refl-then-SPEC kernel derivation. refl builds (Thm.Seq (list) (Eq t t)); gen inlines it
        into `if free-in-hyps ... then Option.None else Option.Some (Thm.Seq g (Forall x p))`. The rust
        backend emits bare `Option::None` and bare `vec![]` inside the Some arm, so rustc cannot infer
        Option<(Vec<Term>, Term)> -> E0282. Passes on wasm. Fix: turbofish un-argument-fixed constructors.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
           (type Thm (Seq (List Term) Term))
           (def (term-eq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
               ((Term.Abs v x) (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
               ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
           (def (free-in (: v Int64) (: t Term))
             (match t
               ((Term.Var n) (= n v))
               ((Term.Comb f x) (or (free-in v f) (free-in v x)))
               ((Term.Eq a b) (or (free-in v a) (free-in v b)))
               ((Term.Abs w body) (if (= w v) false (free-in v body)))
               ((Term.Forall w body) (if (= w v) false (free-in v body)))))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n) (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))
               ((Term.Forall w body) (if (= w v) (Term.Forall w body) (Term.Forall w (subst v s body))))))
           (def (free-in-hyps (: v Int64) (: hs (List Term)))
             (match hs ((list) false) ((list h .. rest) (or (free-in v h) (free-in-hyps v rest)))))
           (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (gen (: x Int64) (: th Thm))
             (match th ((Thm.Seq g p)
               (if (free-in-hyps x g) (Option.None unit) (Option.Some (Thm.Seq g (Term.Forall x p)))))))
           (def (spec (: t Term) (: th Thm))
             (match (concl th)
               ((Term.Forall x body) (Option.Some (Thm.Seq (match th ((Thm.Seq h _) h)) (subst x t body))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (gen 1 (refl (Term.Var 1)))
               ((Option.Some g) (match (spec (Term.Var 7) g)
                                  ((Option.Some sp) (if (term-eq (concl sp) (Term.Eq (Term.Var 7) (Term.Var 7))) 1 0))
                                  ((Option.None _) -1)))
               ((Option.None _) -2)))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))
